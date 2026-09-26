//! 极简 SPIR-V 汇编器。
//!
//! ## 为什么自己写
//!
//! M1 确认了本机**没有 Vulkan SDK**（`VULKAN_SDK` 为空、无 `vk.xml`），所以拿不到
//! `glslc`/`glslangValidator`，没有现成办法把 GLSL 编成 SPIR-V。三条路：
//!
//! 1. 手写最小 SPIR-V 字节数组 —— 能用，但每加一个着色器都要重新数位（不可维护）；
//! 2. **极简汇编器（本文件）** —— 一次性投入，之后所有着色器用构造器生成；
//! 3. 允许可选外部 `glslc` —— 有就用、没有退回 2。
//!
//! 选了 **2**：最符合本项目「自己写、零第三方依赖」的定位，且第二个着色器就开始回本。
//!
//! ## 边界（诚实说明）
//!
//! 这不是通用 SPIR-V 编译器：**没有类型检查、没有 `spirv-val`**。
//! 它只保证**生成的模块语法自洽**（词数、Id 边界、指令顺序），
//! 而真正的正确性由**驱动验收** —— 测试把产物喂给 `vkCreateShaderModule`
//! 与 `vkCreateGraphicsPipelines`，驱动拒绝即失败。这比自写校验器更有说服力。
//!
//! ## 格式速览
//!
//! - 头部 5 个字（20 字节）：魔数 `0x07230203`、版本、生成器、`bound`、保留；
//! - 每条指令：首字高 16 位 = **词数（含首字）**，低 16 位 = 操作码；其后是操作数；
//! - `Id` 从 1 开始；头部 `bound` 必须 **>** 所有用到的 Id。

// ── 操作码（逐个核对过 SPIR-V 1.0 规范） ─────────────────────────────────────

const OP_SOURCE: u16 = 3;
const OP_NAME: u16 = 5;
const OP_MEMBER_NAME: u16 = 6;
const OP_STRING: u16 = 7;
const OP_LINE: u16 = 8;
const OP_EXTENSION: u16 = 10;
const OP_EXT_INST_IMPORT: u16 = 11;
const OP_MEMORY_MODEL: u16 = 14;
const OP_ENTRY_POINT: u16 = 15;
const OP_EXECUTION_MODE: u16 = 16;
const OP_CAPABILITY: u16 = 17;
const OP_TYPE_VOID: u16 = 19;
const OP_TYPE_BOOL: u16 = 20;
const OP_TYPE_INT: u16 = 21;
const OP_TYPE_FLOAT: u16 = 22;
const OP_TYPE_VECTOR: u16 = 23;
const OP_TYPE_ARRAY: u16 = 28;
const OP_TYPE_STRUCT: u16 = 30;
const OP_TYPE_POINTER: u16 = 32;
const OP_TYPE_FUNCTION: u16 = 33;
const OP_CONSTANT: u16 = 43;
const OP_CONSTANT_COMPOSITE: u16 = 44;
const OP_CONSTANT_TRUE: u16 = 41;
const OP_CONSTANT_FALSE: u16 = 42;
const OP_FUNCTION: u16 = 54;
const OP_FUNCTION_PARAMETER: u16 = 55;
const OP_FUNCTION_END: u16 = 56;
const OP_FUNCTION_CALL: u16 = 57;
const OP_VARIABLE: u16 = 59;
const OP_LOAD: u16 = 61;
const OP_STORE: u16 = 62;
const OP_ACCESS_CHAIN: u16 = 65;
const OP_DECORATE: u16 = 71;
const OP_MEMBER_DECORATE: u16 = 72;
const OP_DECORATION_GROUP: u16 = 73;
const OP_GROUP_DECORATE: u16 = 74;
const OP_GROUP_MEMBER_DECORATE: u16 = 75;
const OP_COMPOSITE_EXTRACT: u16 = 81;
const OP_COMPOSITE_CONSTRUCT: u16 = 80;
const OP_BITWISE_AND: u16 = 194;
const OP_CONVERT_U_TO_F: u16 = 112;
const OP_U_DIV: u16 = 134;
const OP_I_EQUAL: u16 = 170;
const OP_SELECT: u16 = 169;
const OP_F_ADD: u16 = 129;
const OP_F_SUB: u16 = 131;
const OP_F_MUL: u16 = 133;
const OP_LABEL: u16 = 248;
const OP_RETURN: u16 = 253;
const OP_RETURN_VALUE: u16 = 254;

// ── 存储类别（SPIR-V 3.3 Storage Class） ─────────────────────────────────────

const SC_INPUT: u32 = 1;
const SC_OUTPUT: u32 = 3;
const SC_FUNCTION: u32 = 7;
/// 推送常量
pub const SC_PUSH_CONSTANT: u32 = 9;

// ── 装饰（SPIR-V 3.6 Decoration） ────────────────────────────────────────────

const DECORATION_BUILT_IN: u32 = 11;
const DECORATION_LOCATION: u32 = 30;
/// `Decoration InBounds` —— 给 `OpAccessChain` 加这个装饰能让校验器放心
/// （表示索引不会越界）。Vulkan 校验层对缺它的访问链会报 warning。
const DECORATION_IN_BOUNDS: u32 = 16;

// ── 内建量与执行模型 ─────────────────────────────────────────────────────────

/// `BuiltIn Position`（顶点输出 `gl_Position`）
const BUILTIN_POSITION: u32 = 0;
/// `BuiltIn VertexIndex`（顶点输入 `gl_VertexIndex`）
pub const BUILTIN_VERTEX_INDEX: u32 = 42;
/// `ExecutionModel Vertex`
const EXECUTION_MODEL_VERTEX: u32 = 0;
/// `ExecutionModel Fragment`
const EXECUTION_MODEL_FRAGMENT: u32 = 4;
/// `ExecutionMode OriginUpperLeft`（片段着色器必需）
const EXECUTION_MODE_ORIGIN_UPPER_LEFT: u32 = 7;
/// `Capability Shader`
const CAPABILITY_SHADER: u32 = 1;

pub const SPIRV_VERSION_1_0: u32 = 0x0001_0000;
const GENERATOR: u32 = 0;

/// SPIR-V 模块的**逻辑布局段**。
///
/// ## 为什么必须有这个（一次代价很大的教训）
///
/// SPIR-V 规范对指令顺序有**严格**要求，模块被划成若干段，段的顺序固定：
///
/// ```text
///   ① OpCapability
///   ② OpExtension
///   ③ OpExtInstImport
///   ④ OpMemoryModel
///   ⑤ OpEntryPoint          ← 必须在**类型/常量之前**！
///   ⑥ OpExecutionMode
///   ⑦ 调试信息（OpSource / OpName / OpString / OpLine）
///   ⑧ 注解（OpDecorate / OpMemberDecorate / OpGroupDecorate / OpDecorationGroup）
///   ⑨ 类型声明、常量、全局变量
///   ⑩ 函数
/// ```
///
/// 本项目第一版把**每条指令按调用顺序**线性写出，于是 `OpEntryPoint`
/// 落在了类型与常量**之后** ⇒ 整份模块的段全部错位。
///
/// 后果极具误导性：`vkCreateShaderModule` **接受**（它只存字节），
/// `vkCreateGraphicsPipelines` 也返回成功、句柄非空，**但 `vkCmdDraw` 一个像素都不画**。
/// 直到装上 Vulkan SDK、用官方 `spirv-val` 校验才看到：
///
/// ```text
///   error: EntryPoint is in an invalid layout section
/// ```
///
/// 所以现在改成：**按段累积，`finish()` 时按规范顺序拼接**。
/// 路由由 [`section_of_opcode`] 自动完成，着色器代码不需要关心顺序。
#[derive(Clone, Copy, PartialEq, Eq)]
enum Section {
    Capability = 0,
    Extension = 1,
    ExtInstImport = 2,
    MemoryModel = 3,
    EntryPoint = 4,
    ExecutionMode = 5,
    Debug = 6,
    Annotation = 7,
    /// 类型声明 + 常量 + 全局变量（规范里是同一段）
    TypeConstGlobal = 8,
    /// 函数体（含 `OpFunction` / 基本块 / 指令 / `OpFunctionEnd`）
    ///
    /// **必须是最后一个** —— 它是「图定义段」的终止者：任何类型/常量/全局变量
    /// 指令出现在 `OpFunction` 之后都会被校验器拒绝
    /// （`OpConstant cannot appear in the graph definitions section`）。
    /// 枚举值等于真实输出顺序，`assemble()` 直接按它遍历。
    Function = 9,
}

const SECTION_COUNT: usize = 10;

/// 按 opcode 决定它属于哪一段（**只用于函数体之外**的指令）。
///
/// 范围依据 SPIR-V 规范的操作码编号（稳定，不会变）。
///
/// ⚠️ 两条容易错的规则：
/// 1. `OpFunction` 是在「进入函数体之前」发射的（那时 `in_function` 还是 false），
///    所以它**必须**在这里显式映射到 `Section::Function`。漏了它就会掉进
///    `_ => TypeConstGlobal`，把函数头排到类型段里 —— ID 顺序虽然没乱，
///    但段序全错（实测踩过：症状是 `OpFunction` 出现在类型段末尾）。
/// 2. `OpLoad` / `OpStore` / 算术等**既可以出现在全局也可以出现在函数内**的指令，
///    不能靠 opcode 判断；它们的归属由 [`Module::in_function`] 决定。
fn section_of_opcode(opcode: u16) -> Section {
    match opcode {
        OP_CAPABILITY => Section::Capability,
        OP_EXTENSION => Section::Extension,
        OP_EXT_INST_IMPORT => Section::ExtInstImport,
        OP_MEMORY_MODEL => Section::MemoryModel,
        OP_ENTRY_POINT => Section::EntryPoint,
        OP_EXECUTION_MODE => Section::ExecutionMode,
        OP_SOURCE | OP_NAME | OP_MEMBER_NAME | OP_STRING | OP_LINE => Section::Debug,
        OP_DECORATE | OP_MEMBER_DECORATE | OP_GROUP_DECORATE | OP_GROUP_MEMBER_DECORATE
        | OP_DECORATION_GROUP => Section::Annotation,
        // 函数头：它在 `in_function` 置位**之前**发射，必须显式归到函数段
        OP_FUNCTION => Section::Function,
        _ => Section::TypeConstGlobal,
    }
}

/// 该 opcode 是否是**类型声明**。
fn is_type_opcode(opcode: u16) -> bool {
    matches!(
        opcode,
        OP_TYPE_VOID
            | OP_TYPE_BOOL
            | OP_TYPE_INT
            | OP_TYPE_FLOAT
            | OP_TYPE_VECTOR
            | OP_TYPE_ARRAY
            | OP_TYPE_STRUCT
            | OP_TYPE_POINTER
            | OP_TYPE_FUNCTION
    )
}

/// 该 opcode 是否是**常量声明**。
fn is_constant_opcode(opcode: u16) -> bool {
    matches!(
        opcode,
        OP_CONSTANT | OP_CONSTANT_COMPOSITE | OP_CONSTANT_TRUE | OP_CONSTANT_FALSE
    )
}

/// 一个正在构建中的 SPIR-V 模块。
///
/// 指令按**段**累积；[`Module::finish`] 按规范顺序拼接成最终字节流。
pub struct Module {
    /// 每段的字（不含头部 5 个字）
    sections: [Vec<u32>; SECTION_COUNT],
    next_id: u32,
    /// 是否处于函数体内（`OpFunction` 之后、`OpFunctionEnd` 之前）
    in_function: bool,
    /// 全局变量声明（`OpVariable`，存储类 Input/Output/PushConstant 等）。
    ///
    /// 与类型/常量**分开存**是为了控制最终顺序：全局变量可能引用
    /// 「函数体内延迟声明的类型」（实测 `vs_hardcoded_position` 就是这样），
    /// 所以它必须排在延迟类型**之后**。
    globals: Vec<u32>,
    /// 函数体内延迟声明的**类型**（`OpType*`）
    deferred_types: Vec<u32>,
    /// 函数体内延迟声明的**常量**（`OpConstant*`）
    deferred_consts: Vec<u32>,
    /// 是否有 `OpAccessChain`（决定要不要发 `OpDecorate %id InBounds`）
    needs_in_bounds: bool,
}

impl Default for Module {
    fn default() -> Self {
        Self::new()
    }
}

impl Module {
    pub fn new() -> Module {
        Module {
            sections: std::array::from_fn(|_| Vec::new()),
            next_id: 1,
            in_function: false,
            globals: Vec::new(),
            deferred_types: Vec::new(),
            deferred_consts: Vec::new(),
            needs_in_bounds: false,
        }
    }

    /// 分配一个新 Id（从 1 开始；0 是保留的「无 Id」）。
    pub fn id(&mut self) -> u32 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    fn section_mut(&mut self, s: Section) -> &mut Vec<u32> {
        &mut self.sections[s as usize]
    }

    /// 写一条指令到**它所属的段**。调用方只管语义，顺序由本函数保证。
    ///
    /// 归属规则：
    /// 1. 在函数体内（`OpFunction` .. `OpFunctionEnd`）⇒ **一律** `Function` 段；
    /// 2. 否则按 [`section_of_opcode`]。
    ///
    /// **词数由本函数自动算**，所以不可能写错词数。
    fn op(&mut self, opcode: u16, operands: &[u32]) -> &mut Module {
        // ⚠️ 顺序很重要：必须**先**决定这条指令进哪个段，**再**更新状态。
        // 反过来的话 `OpFunctionEnd` 会先把 in_function 置 false，
        // 于是它自己被判成「函数外」而掉进类型段（实测踩过这个坑）。
        let wc = operands.len() as u16 + 1;
        // 编码好的指令（词数 + 操作码 合成第一个词）
        let mut encoded = Vec::with_capacity(operands.len() + 1);
        encoded.push(((wc as u32) << 16) | opcode as u32);
        encoded.extend_from_slice(operands);

        if opcode == OP_ACCESS_CHAIN {
            self.needs_in_bounds = true;
        }

        if self.in_function {
            // 函数体内：类型/常量声明要**延迟**到图定义段，其余进函数段
            if opcode == OP_VARIABLE {
                // 局部变量（Function 存储类）属于函数体，不是全局
                self.section_mut(Section::Function).extend_from_slice(&encoded);
            } else if is_type_opcode(opcode) {
                self.deferred_types.extend_from_slice(&encoded);
            } else if is_constant_opcode(opcode) {
                self.deferred_consts.extend_from_slice(&encoded);
            } else {
                self.section_mut(Section::Function).extend_from_slice(&encoded);
            }
        } else if opcode == OP_VARIABLE {
            // 全局变量单独存，稍后排在延迟类型之后
            self.globals.extend_from_slice(&encoded);
        } else {
            let sec = section_of_opcode(opcode);
            self.section_mut(sec).extend_from_slice(&encoded);
        }

        // 最后更新「是否在函数内」的状态
        match opcode {
            OP_FUNCTION => self.in_function = true,
            OP_FUNCTION_END => self.in_function = false,
            _ => {}
        }
        self
    }

    fn literal_string(s: &str) -> Vec<u32> {
        let mut bytes = s.as_bytes().to_vec();
        bytes.push(0);
        while bytes.len() % 4 != 0 {
            bytes.push(0);
        }
        bytes
            .chunks_exact(4)
            .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect()
    }

    // ── 头部区 ───────────────────────────────────────────────────────────────

    pub fn shader_capability(&mut self) -> &mut Module {
        self.op(OP_CAPABILITY, &[CAPABILITY_SHADER])
    }

    /// Logical 寻址 + GLSL450 内存模型（图形着色器的标配）。
    pub fn memory_model_glsl450(&mut self) -> &mut Module {
        self.op(OP_MEMORY_MODEL, &[0, 1])
    }

    pub fn source_unknown(&mut self) -> &mut Module {
        self.op(OP_SOURCE, &[0, 0])
    }

    /// `OpEntryPoint executionModel entryPoint "name" interface...`
    ///
    /// `interface` 必须是该执行模型下**所有静态使用的 Input/Output 变量**。
    pub fn entry_point(&mut self, model: u32, fn_id: u32, name: &str, interface: &[u32]) -> &mut Module {
        let lit = Self::literal_string(name);
        let mut ops = vec![model, fn_id];
        ops.extend_from_slice(&lit);
        ops.extend_from_slice(interface);
        self.op(OP_ENTRY_POINT, &ops)
    }

    pub fn execution_mode(&mut self, entry: u32, mode: u32, params: &[u32]) -> &mut Module {
        let mut ops = vec![entry, mode];
        ops.extend_from_slice(params);
        self.op(OP_EXECUTION_MODE, &ops)
    }

    pub fn debug_name(&mut self, target: u32, what: &str) -> &mut Module {
        let lit = Self::literal_string(what);
        let mut ops = vec![target];
        ops.extend_from_slice(&lit);
        self.op(OP_NAME, &ops)
    }

    pub fn decorate(&mut self, target: u32, decoration: u32, params: &[u32]) -> &mut Module {
        let mut ops = vec![target, decoration];
        ops.extend_from_slice(params);
        self.op(OP_DECORATE, &ops)
    }

    #[allow(dead_code)]
    pub fn member_decorate(&mut self, ty: u32, member: u32, decoration: u32, params: &[u32]) -> &mut Module {
        let mut ops = vec![ty, member, decoration];
        ops.extend_from_slice(params);
        self.op(OP_MEMBER_DECORATE, &ops)
    }

    // ── 类型与常量 ───────────────────────────────────────────────────────────

    pub fn type_void(&mut self) -> u32 {
        let r = self.id();
        self.op(OP_TYPE_VOID, &[r]);
        r
    }

    pub fn type_float(&mut self) -> u32 {
        let r = self.id();
        self.op(OP_TYPE_FLOAT, &[r, 32]);
        r
    }

    pub fn type_uint(&mut self) -> u32 {
        let r = self.id();
        self.op(OP_TYPE_INT, &[r, 32, 0]); // 32 位无符号
        r
    }

    pub fn type_vector(&mut self, component: u32, count: u32) -> u32 {
        let r = self.id();
        self.op(OP_TYPE_VECTOR, &[r, component, count]);
        r
    }

    /// `OpTypeArray result elementType length`（`length` 是一个常量 Id）
    pub fn type_array(&mut self, element: u32, length_const: u32) -> u32 {
        let r = self.id();
        self.op(OP_TYPE_ARRAY, &[r, element, length_const]);
        r
    }

    /// `OpTypeStruct result member0 member1 ...`
    pub fn type_struct(&mut self, members: &[u32]) -> u32 {
        let r = self.id();
        let mut ops = vec![r];
        ops.extend_from_slice(members);
        self.op(OP_TYPE_STRUCT, &ops);
        r
    }

    pub fn type_pointer(&mut self, storage_class: u32, pointee: u32) -> u32 {
        let r = self.id();
        self.op(OP_TYPE_POINTER, &[r, storage_class, pointee]);
        r
    }

    pub fn type_function(&mut self, ret: u32, params: &[u32]) -> u32 {
        let r = self.id();
        let mut ops = vec![r, ret];
        ops.extend_from_slice(params);
        self.op(OP_TYPE_FUNCTION, &ops);
        r
    }

    pub fn constant_f32(&mut self, ty: u32, value: f32) -> u32 {
        let r = self.id();
        self.op(OP_CONSTANT, &[ty, r, value.to_bits()]);
        r
    }

    pub fn constant_u32(&mut self, ty: u32, value: u32) -> u32 {
        let r = self.id();
        self.op(OP_CONSTANT, &[ty, r, value]);
        r
    }

    /// `OpConstantComposite resultType result constituents...`
    ///
    /// ⚠️ **所有 `constituents` 必须都是常量**。若成员是运行时算出来的值
    /// （`OpSelect` / `OpLoad` / 算术结果），必须改用 [`Self::composite_construct`]
    /// —— 用错会让 SPIR-V 非法，而驱动**不报错、只是不画**。
    /// 本项目就踩过这个坑（三角形着色器曾因此完全画不出像素）。
    pub fn constant_composite(&mut self, ty: u32, parts: &[u32]) -> u32 {
        let r = self.id();
        let mut ops = vec![ty, r];
        ops.extend_from_slice(parts);
        self.op(OP_CONSTANT_COMPOSITE, &ops);
        r
    }

    /// `OpCompositeConstruct resultType result constituents...`
    ///
    /// 成员**可以是运行时值**。把多个标量/向量凑成一个向量时用这个，
    /// 而不是 `OpConstantComposite`。
    pub fn composite_construct(&mut self, ty: u32, parts: &[u32]) -> u32 {
        let r = self.id();
        let mut ops = vec![ty, r];
        ops.extend_from_slice(parts);
        self.op(OP_COMPOSITE_CONSTRUCT, &ops);
        r
    }

    /// `OpVariable resultType result storageClass [initializer]`
    ///
    /// **Output 变量不要给初值**：Vulkan/SPIR-V 1.0 下带初值的 Output 全局变量
    /// 会让后续读取失去「已定义」性（校验报 use before definition）。
    pub fn variable(&mut self, ptr_ty: u32, storage_class: u32) -> u32 {
        let r = self.id();
        self.op(OP_VARIABLE, &[ptr_ty, r, storage_class]);
        r
    }

    // ── 函数体 ───────────────────────────────────────────────────────────────

    pub fn function(&mut self, ret: u32, fn_id: u32, fn_ty: u32, first_block: u32) -> &mut Module {
        // OpFunction resultType result functionControl functionType
        self.op(OP_FUNCTION, &[ret, fn_id, 0, fn_ty]);
        // 第一个基本块紧跟其后
        self.op(OP_LABEL, &[first_block]);
        self
    }

    pub fn load(&mut self, ty: u32, ptr: u32) -> u32 {
        let r = self.id();
        self.op(OP_LOAD, &[ty, r, ptr]);
        r
    }

    pub fn store(&mut self, ptr: u32, value: u32) -> &mut Module {
        self.op(OP_STORE, &[ptr, value])
    }

    pub fn access_chain(&mut self, ty: u32, base: u32, indexes: &[u32]) -> u32 {
        let r = self.id();
        let mut ops = vec![ty, r, base];
        ops.extend_from_slice(indexes);
        self.op(OP_ACCESS_CHAIN, &ops);
        r
    }

    /// `OpCompositeExtract result composite index0 index1 ...`
    pub fn composite_extract(&mut self, ty: u32, composite: u32, indexes: &[u32]) -> u32 {
        let r = self.id();
        let mut ops = vec![ty, r, composite];
        ops.extend_from_slice(indexes);
        self.op(OP_COMPOSITE_EXTRACT, &ops);
        r
    }

    pub fn f_add(&mut self, ty: u32, a: u32, b: u32) -> u32 {
        let r = self.id();
        self.op(OP_F_ADD, &[ty, r, a, b]);
        r
    }

    pub fn f_sub(&mut self, ty: u32, a: u32, b: u32) -> u32 {
        let r = self.id();
        self.op(OP_F_SUB, &[ty, r, a, b]);
        r
    }

    pub fn f_mul(&mut self, ty: u32, a: u32, b: u32) -> u32 {
        let r = self.id();
        self.op(OP_F_MUL, &[ty, r, a, b]);
        r
    }

    /// `OpBitwiseAnd`
    pub fn bitwise_and(&mut self, ty: u32, a: u32, b: u32) -> u32 {
        let r = self.id();
        self.op(OP_BITWISE_AND, &[ty, r, a, b]);
        r
    }

    pub fn type_bool(&mut self) -> u32 {
        let r = self.id();
        self.op(OP_TYPE_BOOL, &[r]);
        r
    }

    /// `OpIEqual`
    pub fn op_i_equal(&mut self, bool_ty: u32, a: u32, b: u32) -> u32 {
        let r = self.id();
        self.op(OP_I_EQUAL, &[bool_ty, r, a, b]);
        r
    }

    /// `OpSelect`
    pub fn op_select(&mut self, ty: u32, cond: u32, true_val: u32, false_val: u32) -> u32 {
        let r = self.id();
        self.op(OP_SELECT, &[ty, r, cond, true_val, false_val]);
        r
    }

    /// `OpUDiv`（无符号整数除法）
    pub fn op_udiv(&mut self, ty: u32, result: u32, a: u32, b: u32) -> &mut Module {
        self.op(OP_U_DIV, &[ty, result, a, b])
    }

    /// `OpConvertUToF`（无符号整数 → 浮点）
    pub fn convert_u_to_f(&mut self, ty: u32, value: u32) -> u32 {
        let r = self.id();
        self.op(OP_CONVERT_U_TO_F, &[ty, r, value]);
        r
    }

    pub fn return_void(&mut self) -> &mut Module {
        self.op(OP_RETURN, &[])
    }

    pub fn function_end(&mut self) -> &mut Module {
        self.op(OP_FUNCTION_END, &[])
    }

    /// 收尾：**按规范顺序拼接各段**，回填 `bound`，转成小端字节流。
    ///
    /// 顺序见 [`Section`] 的文档。这里还会按需补一条
    /// `OpDecorate %accessChain InBounds`（见 [`Module::access_chain`]）。
    pub fn finish(mut self) -> Vec<u8> {
        let words = self.assemble();
        let mut out = Vec::with_capacity(words.len() * 4);
        for w in &words {
            out.extend_from_slice(&w.to_le_bytes());
        }
        out
    }

    /// 按规范顺序组装全部词（含头部 5 个字）。`finish` 与 `describe` 共用它，
    /// **保证「诊断看到的」就是「实际输出的」**。
    fn assemble(&mut self) -> Vec<u32> {
        // 若有 OpAccessChain，补 InBounds 装饰（必须落在注解段 ⇒ 由 op() 自动路由）
        if self.needs_in_bounds {
            let ids: Vec<u32> = self
                .sections
                .iter()
                .flat_map(|s| extract_access_chain_result_ids(s))
                .collect();
            self.needs_in_bounds = false;
            for id in ids {
                self.decorate(id, DECORATION_IN_BOUNDS, &[]);
            }
        }

        let mut words = Vec::with_capacity(
            5 + self.sections.iter().map(|s| s.len()).sum::<usize>(),
        );
        words.extend_from_slice(&[0x0723_0203, SPIRV_VERSION_1_0, GENERATOR, 0, 0]);
        // 「函数」段必须最后；其余按 Section 的枚举顺序
        for i in 0..SECTION_COUNT {
            if i == Section::Function as usize {
                continue;
            }
            words.extend_from_slice(&self.sections[i]);
            // 「类型/常量/全局」段之后补齐三部分，顺序**必须**是：
            //   ① 函数体内延迟的**类型**
            //   ② 函数体内延迟的**常量**
            //   ③ **全局变量**
            //
            // 为什么是这个顺序（两次实测教训）：
            // - 延迟声明若放段首 ⇒ 模块级类型被挤到后面，报「Type Id is not a type」；
            // - 延迟声明若放段尾 ⇒ 落到 `OpFunction` 之后，报
            //   「OpConstant cannot appear in the graph definitions section」；
            // - 全局变量要在延迟类型之后，因为它可能引用它们；且**不能**在类型段
            //   里逐条插入（那会把函数体内创建的类型挤到全局变量之后）。
            if i == Section::TypeConstGlobal as usize {
                words.extend_from_slice(&self.deferred_types);
                words.extend_from_slice(&self.deferred_consts);
                words.extend_from_slice(&self.globals);
            }
        }
        words.extend_from_slice(&self.sections[Section::Function as usize]);
        words[3] = self.next_id; // bound 必须 > 所有用过的 Id
        words
    }

    /// 模块的总词数（不含头部 5 个字）。
    pub fn word_count(&self) -> usize {
        self.sections.iter().map(|s| s.len()).sum::<usize>() + 5
    }

    /// 诊断用：每段的词数，便于定位「指令跑到错段去了」。
    pub fn section_word_counts(&self) -> [usize; SECTION_COUNT] {
        std::array::from_fn(|i| self.sections[i].len())
    }

    /// 诊断用：把模块按**段**打印成人类可读的形式。
    ///
    /// 为什么需要它：段序错了的时候，驱动不报错也不画，而 `spirv-val` 只会说
    /// 「某条指令不该在这里」。有了这个转储就能一眼看出**指令落在哪个段**，
    /// 从而定位是路由规则的问题还是调用顺序的问题。
    ///
    /// 用法：`println!("{}", module.describe())`（在 `finish()` 之前调用）。
    ///
    /// **它展示的是 `assemble()` 的真实输出顺序**，不是内部存储顺序 ——
    /// 否则诊断与实际不符（这一点本身也踩过坑）。
    pub fn describe(&mut self) -> String {
        let words = self.assemble();
        let mut s = String::new();
        s.push_str(&format!("SPIR-V 模块（bound={}，共 {} 词）\n", words[3], words.len()));
        s.push_str(&format!(
            "  各段词数 = {:?}\n  延迟类型 {} 词 / 延迟常量 {} 词 / 全局变量 {} 词\n",
            self.section_word_counts(),
            self.deferred_types.len(),
            self.deferred_consts.len(),
            self.globals.len()
        ));
        let mut p = 5usize;
        while p < words.len() {
            let first = words[p];
            let wc = (first >> 16) as usize;
            let opcode = (first & 0xffff) as u16;
            if wc == 0 || p + wc > words.len() {
                s.push_str(&format!("  <损坏的词流：wc={wc}>\n"));
                break;
            }
            s.push_str(&format!("  {:>4}  {:<24} ({} 词)\n", p, opcode_name(opcode), wc));
            p += wc;
        }
        s
    }
}

/// opcode → 名字（只为诊断输出，未列出的显示编号）。
fn opcode_name(op: u16) -> String {
    match op {
        OP_SOURCE => "OpSource",
        OP_NAME => "OpName",
        OP_MEMBER_NAME => "OpMemberName",
        OP_STRING => "OpString",
        OP_LINE => "OpLine",
        OP_EXTENSION => "OpExtension",
        OP_EXT_INST_IMPORT => "OpExtInstImport",
        OP_MEMORY_MODEL => "OpMemoryModel",
        OP_ENTRY_POINT => "OpEntryPoint",
        OP_EXECUTION_MODE => "OpExecutionMode",
        OP_CAPABILITY => "OpCapability",
        OP_TYPE_VOID => "OpTypeVoid",
        OP_TYPE_BOOL => "OpTypeBool",
        OP_TYPE_INT => "OpTypeInt",
        OP_TYPE_FLOAT => "OpTypeFloat",
        OP_TYPE_VECTOR => "OpTypeVector",
        OP_TYPE_ARRAY => "OpTypeArray",
        OP_TYPE_STRUCT => "OpTypeStruct",
        OP_TYPE_POINTER => "OpTypePointer",
        OP_TYPE_FUNCTION => "OpTypeFunction",
        OP_CONSTANT => "OpConstant",
        OP_CONSTANT_COMPOSITE => "OpConstantComposite",
        OP_FUNCTION => "OpFunction",
        OP_FUNCTION_PARAMETER => "OpFunctionParameter",
        OP_FUNCTION_END => "OpFunctionEnd",
        OP_FUNCTION_CALL => "OpFunctionCall",
        OP_VARIABLE => "OpVariable",
        OP_LOAD => "OpLoad",
        OP_STORE => "OpStore",
        OP_ACCESS_CHAIN => "OpAccessChain",
        OP_DECORATE => "OpDecorate",
        OP_MEMBER_DECORATE => "OpMemberDecorate",
        OP_COMPOSITE_EXTRACT => "OpCompositeExtract",
        OP_COMPOSITE_CONSTRUCT => "OpCompositeConstruct",
        OP_BITWISE_AND => "OpBitwiseAnd",
        OP_CONVERT_U_TO_F => "OpConvertUToF",
        OP_U_DIV => "OpUDiv",
        OP_I_EQUAL => "OpIEqual",
        OP_SELECT => "OpSelect",
        OP_F_ADD => "OpFAdd",
        OP_F_SUB => "OpFSub",
        OP_F_MUL => "OpFMul",
        OP_LABEL => "OpLabel",
        OP_RETURN => "OpReturn",
        OP_RETURN_VALUE => "OpReturnValue",
        other => return format!("Op#{other}"),
    }
    .to_string()
}

/// 从一段字里找出所有 `OpAccessChain` 的**结果 Id**。
///
/// 编码：`OpAccessChain resultType result id0 id1 ...`
/// ⇒ 结果 Id 在操作数的第 2 个（下标 1）。
fn extract_access_chain_result_ids(words: &[u32]) -> Vec<u32> {
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < words.len() {
        let first = words[i];
        let wc = (first >> 16) as usize;
        let opcode = (first & 0xffff) as u16;
        if wc == 0 || i + wc > words.len() {
            break;
        }
        if opcode == OP_ACCESS_CHAIN && wc >= 4 {
            out.push(words[i + 2]);
        }
        i += wc;
    }
    out
}

// ── 着色器 ───────────────────────────────────────────────────────────────────

/// 最小顶点着色器：**空 `main`**，不声明任何变量。
///
/// 用途：诊断。如果连它建管线都崩，那问题不在 SPIR-V 内容，而在管线状态或模块句柄。
pub fn vertex_shader_empty() -> Vec<u8> {
    let mut m = Module::new();
    m.shader_capability().memory_model_glsl450().source_unknown();
    let void = m.type_void();
    let fn_ty = m.type_function(void, &[]);
    let fn_id = m.id();
    let block = m.id();
    m.entry_point(EXECUTION_MODEL_VERTEX, fn_id, "main", &[]);
    m.function(void, fn_id, fn_ty, block);
    m.return_void();
    m.function_end();
    m.finish()
}

/// 最小片段着色器：**空 `main`**，不写任何输出。
pub fn fragment_shader_empty() -> Vec<u8> {
    let mut m = Module::new();
    m.shader_capability().memory_model_glsl450().source_unknown();
    let void = m.type_void();
    let fn_ty = m.type_function(void, &[]);
    let fn_id = m.id();
    let block = m.id();
    m.entry_point(EXECUTION_MODEL_FRAGMENT, fn_id, "main", &[]);
    m.execution_mode(fn_id, EXECUTION_MODE_ORIGIN_UPPER_LEFT, &[]);
    m.function(void, fn_id, fn_ty, block);
    m.return_void();
    m.function_end();
    m.finish()
}

/// 诊断用：写一个**常量** `gl_Position`（不用 `gl_VertexIndex`、不用数组）。
pub fn vertex_shader_const_position() -> Vec<u8> {
    let mut m = Module::new();
    m.shader_capability().memory_model_glsl450().source_unknown();
    let void = m.type_void();
    let f32_ty = m.type_float();
    let v4 = m.type_vector(f32_ty, 4);
    let ptr_out_v4 = m.type_pointer(SC_OUTPUT, v4);
    let fn_ty = m.type_function(void, &[]);
    let out_pos = m.variable(ptr_out_v4, SC_OUTPUT);
    let x = m.constant_f32(f32_ty, 0.0);
    let y = m.constant_f32(f32_ty, 0.0);
    let z = m.constant_f32(f32_ty, 0.0);
    let w = m.constant_f32(f32_ty, 1.0);
    let pos = m.constant_composite(v4, &[x, y, z, w]);
    let fn_id = m.id();
    let block = m.id();
    m.entry_point(EXECUTION_MODEL_VERTEX, fn_id, "main", &[out_pos]);
    m.debug_name(out_pos, "gl_Position");
    m.decorate(out_pos, DECORATION_BUILT_IN, &[BUILTIN_POSITION]);
    m.function(void, fn_id, fn_ty, block);
    m.store(out_pos, pos);
    m.return_void();
    m.function_end();
    m.finish()
}

/// 诊断用：读 `gl_VertexIndex` 但不索引数组（只是把它当标量用不到的地方）。
pub fn vertex_shader_reads_vertex_index() -> Vec<u8> {
    let mut m = Module::new();
    m.shader_capability().memory_model_glsl450().source_unknown();
    let void = m.type_void();
    let f32_ty = m.type_float();
    let u32_ty = m.type_uint();
    let v4 = m.type_vector(f32_ty, 4);
    let ptr_out_v4 = m.type_pointer(SC_OUTPUT, v4);
    let ptr_in_u32 = m.type_pointer(SC_INPUT, u32_ty);
    let fn_ty = m.type_function(void, &[]);
    let out_pos = m.variable(ptr_out_v4, SC_OUTPUT);
    let in_index = m.variable(ptr_in_u32, SC_INPUT);
    let x = m.constant_f32(f32_ty, 0.0);
    let y = m.constant_f32(f32_ty, 0.0);
    let z = m.constant_f32(f32_ty, 0.0);
    let w = m.constant_f32(f32_ty, 1.0);
    let pos = m.constant_composite(v4, &[x, y, z, w]);
    let fn_id = m.id();
    let block = m.id();
    m.entry_point(EXECUTION_MODEL_VERTEX, fn_id, "main", &[out_pos, in_index]);
    m.debug_name(out_pos, "gl_Position");
    m.decorate(out_pos, DECORATION_BUILT_IN, &[BUILTIN_POSITION]);
    m.debug_name(in_index, "gl_VertexIndex");
    m.decorate(in_index, DECORATION_BUILT_IN, &[BUILTIN_VERTEX_INDEX]);
    m.function(void, fn_id, fn_ty, block);
    let _idx = m.load(u32_ty, in_index); // 读出来，但不使用它的值
    m.store(out_pos, pos);
    m.return_void();
    m.function_end();
    m.finish()
}

/// 诊断用：**三个顶点都用同一组常量位置**（不读 `gl_VertexIndex`、不用 `OpSelect`）。
///
/// 用途：隔离「顶点选择逻辑（`OpSelect`）有问题」还是「管线/光栅化有问题」。
/// 位置取屏幕正中心 `(0,0,0,1)`；三个顶点同位置会退化成零面积 ⇒ 不产生像素，
/// 所以它只用来验证「顶点着色器有没有被跑起来」（配合不同顶点数观察行为）。
pub fn vertex_shader_hardcoded_position() -> Vec<u8> {
    let mut m = Module::new();
    m.shader_capability().memory_model_glsl450().source_unknown();
    let void = m.type_void();
    let f32_ty = m.type_float();
    let v4 = m.type_vector(f32_ty, 4);
    let ptr_out_v4 = m.type_pointer(SC_OUTPUT, v4);
    let fn_ty = m.type_function(void, &[]);
    let out_pos = m.variable(ptr_out_v4, SC_OUTPUT);
    let fn_id = m.id();
    let block = m.id();
    m.entry_point(EXECUTION_MODEL_VERTEX, fn_id, "main", &[out_pos]);
    m.debug_name(out_pos, "gl_Position");
    m.decorate(out_pos, DECORATION_BUILT_IN, &[BUILTIN_POSITION]);
    m.function(void, fn_id, fn_ty, block);
    let x = m.constant_f32(f32_ty, 0.0);
    let y = m.constant_f32(f32_ty, 0.0);
    let z = m.constant_f32(f32_ty, 0.0);
    let w = m.constant_f32(f32_ty, 1.0);
    let pos = m.constant_composite(v4, &[x, y, z, w]);
    m.store(out_pos, pos);
    m.return_void();
    m.function_end();
    m.finish()
}

/// 诊断用：顶点位置来自**三个独立常量**，按 `gl_VertexIndex` 用 `OpSelect` 逐分量选。
///
/// 与 `vertex_shader_triangle` 的区别：这里 v4 的三个分量**全部**参与选择
/// （不是先选 xy 再组 vec4），用来排查「先选标量再 `OpConstantComposite`」是否有问题。
pub fn vertex_shader_select_full_vec4(positions: [[f32; 4]; 3]) -> Vec<u8> {
    let mut m = Module::new();
    m.shader_capability().memory_model_glsl450().source_unknown();
    let void = m.type_void();
    let f32_ty = m.type_float();
    let u32_ty = m.type_uint();
    let bool_ty = m.type_bool();
    let v4 = m.type_vector(f32_ty, 4);
    let ptr_out_v4 = m.type_pointer(SC_OUTPUT, v4);
    let ptr_in_u32 = m.type_pointer(SC_INPUT, u32_ty);
    let fn_ty = m.type_function(void, &[]);
    let out_pos = m.variable(ptr_out_v4, SC_OUTPUT);
    let in_index = m.variable(ptr_in_u32, SC_INPUT);

    // 三个完整 vec4 常量
    let mut consts = Vec::new();
    for p in positions {
        let mut parts = Vec::new();
        for c in p {
            parts.push(m.constant_f32(f32_ty, c));
        }
        consts.push(m.constant_composite(v4, &parts));
    }
    let k1 = m.constant_u32(u32_ty, 1);
    let k2 = m.constant_u32(u32_ty, 2);

    let fn_id = m.id();
    let block = m.id();
    m.entry_point(EXECUTION_MODEL_VERTEX, fn_id, "main", &[out_pos, in_index]);
    m.debug_name(out_pos, "gl_Position");
    m.decorate(out_pos, DECORATION_BUILT_IN, &[BUILTIN_POSITION]);
    m.debug_name(in_index, "gl_VertexIndex");
    m.decorate(in_index, DECORATION_BUILT_IN, &[BUILTIN_VERTEX_INDEX]);
    m.function(void, fn_id, fn_ty, block);
    let idx = m.load(u32_ty, in_index);
    let c1 = m.op_i_equal(bool_ty, idx, k1);
    let c2 = m.op_i_equal(bool_ty, idx, k2);
    // ⚠️ **不能**对向量直接 `OpSelect` 配标量 bool 条件：
    // 规范要求「Result Type 与 condition 的分量数相等」，否则 spirv-val 报
    // `Expected vector sizes of Result Type and the condition to be equal: Select`。
    // 所以这里抽出分量，逐分量做标量选择，再用 `OpCompositeConstruct` 组回 vec4。
    let mut comps = Vec::with_capacity(4);
    for c in 0..4u32 {
        let v0 = m.composite_extract(f32_ty, consts[0], &[c]);
        let v1 = m.composite_extract(f32_ty, consts[1], &[c]);
        let v2 = m.composite_extract(f32_ty, consts[2], &[c]);
        let a = m.op_select(f32_ty, c1, v1, v0);
        let b = m.op_select(f32_ty, c2, v2, a);
        comps.push(b);
    }
    let pos = m.composite_construct(v4, &comps);
    m.store(out_pos, pos);
    m.return_void();
    m.function_end();
    m.finish()
}

/// 诊断用：**三个顶点都由纯常量 vec4 直接写出**，完全不用 `OpSelect` /
/// `OpCompositeConstruct` / `gl_VertexIndex`。
///
/// 用途：最干净的判别 —— 若它画得出，问题在「顶点位置计算」路径；
/// 若画不出，问题在光栅化/管线状态，与着色器逻辑无关。
///
/// 顶点位置按 `gl_VertexIndex` 选取这一步被**去掉了**：SPIR-V 里三个顶点
/// 都执行同一条 `OpStore`（存同一个常量）。所以三个顶点重合 ⇒ 退化三角形、
/// 不产生像素。为让它有面积，改为**每次执行的顶点都落在同一处**是不行的；
/// 因此本函数实际只用于验证「常量路径能不能画出**任何**东西」——
/// 配合 `POINT_LIST` 或退化三角形不产生像素都属于预期。
pub fn vertex_shader_single_constant() -> Vec<u8> {
    let mut m = Module::new();
    m.shader_capability().memory_model_glsl450().source_unknown();
    let void = m.type_void();
    let f32_ty = m.type_float();
    let v4 = m.type_vector(f32_ty, 4);
    let ptr_out_v4 = m.type_pointer(SC_OUTPUT, v4);
    let fn_ty = m.type_function(void, &[]);
    let out_pos = m.variable(ptr_out_v4, SC_OUTPUT);
    let fn_id = m.id();
    let block = m.id();
    m.entry_point(EXECUTION_MODEL_VERTEX, fn_id, "main", &[out_pos]);
    m.debug_name(out_pos, "gl_Position");
    m.decorate(out_pos, DECORATION_BUILT_IN, &[BUILTIN_POSITION]);
    m.function(void, fn_id, fn_ty, block);
    let x = m.constant_f32(f32_ty, -0.8);
    let y = m.constant_f32(f32_ty, -0.8);
    let z = m.constant_f32(f32_ty, 0.0);
    let w = m.constant_f32(f32_ty, 1.0);
    let pos = m.constant_composite(v4, &[x, y, z, w]);
    m.store(out_pos, pos);
    m.return_void();
    m.function_end();
    m.finish()
}

/// 顶点着色器（**顶点缓冲输入**）：从 location 0 读一个 `vec2` 位置。
///
/// 这是**完全标准**的 Vulkan 顶点路径（真实顶点缓冲 + 顶点属性），
/// 不依赖 `gl_VertexIndex`、`OpSelect` 或任何内置变量。
///
/// 存在的理由：用来判别「这个驱动能不能画出任何几何」。
/// 若它画得出，问题在「从内置变量算顶点位置」那条路；
/// 若它也画不出，问题在更底层（管线状态或用法）。
pub fn vertex_shader_from_vertex_buffer() -> Vec<u8> {
    let mut m = Module::new();
    m.shader_capability().memory_model_glsl450().source_unknown();

    let void = m.type_void();
    let f32_ty = m.type_float();
    let v2 = m.type_vector(f32_ty, 2);
    let v4 = m.type_vector(f32_ty, 4);
    let ptr_in_v2 = m.type_pointer(SC_INPUT, v2);
    let ptr_out_v4 = m.type_pointer(SC_OUTPUT, v4);
    let fn_ty = m.type_function(void, &[]);

    let in_pos = m.variable(ptr_in_v2, SC_INPUT);
    let out_pos = m.variable(ptr_out_v4, SC_OUTPUT);

    let z = m.constant_f32(f32_ty, 0.0);
    let w = m.constant_f32(f32_ty, 1.0);

    let fn_id = m.id();
    let block = m.id();
    m.entry_point(EXECUTION_MODEL_VERTEX, fn_id, "main", &[in_pos, out_pos]);
    m.debug_name(in_pos, "in_pos");
    m.decorate(in_pos, DECORATION_LOCATION, &[0]);
    m.debug_name(out_pos, "gl_Position");
    m.decorate(out_pos, DECORATION_BUILT_IN, &[BUILTIN_POSITION]);

    m.function(void, fn_id, fn_ty, block);
    let p = m.load(v2, in_pos);
    let x = m.composite_extract(f32_ty, p, &[0]);
    let y = m.composite_extract(f32_ty, p, &[1]);
    // x / y 是运行时值 ⇒ 必须用 OpCompositeConstruct
    let pos4 = m.composite_construct(v4, &[x, y, z, w]);
    m.store(out_pos, pos4);
    m.return_void();
    m.function_end();
    m.finish()
}

/// 顶点着色器（三角形）：把 3 个 NDC 顶点写进 `gl_Position`。
///
/// ## ⚠️ 这里为什么不用常量数组索引（一个实测教训）
///
/// 早期版本把 3 个顶点放进 `OpConstantComposite` 数组，再用 `gl_VertexIndex`
/// 做**运行时索引**（`OpAccessChain`）。实测：**驱动在编译期直接崩**
/// （`STATUS_STACK_BUFFER_OVERRUN`），而 `vkCreateShaderModule` 明明接受了它
/// —— 因为驱动建模块时只存字节，**建管线时才真正编译**。
///
/// 改成本实现：用 `gl_VertexIndex` 做 `OpSelect` 逐分量选，**没有任何动态索引**。
/// 顶点仍是编译期常量（数组元素本身是常量），只是选择过程在运行时。
pub fn vertex_shader_triangle(positions_ndc: [[f32; 2]; 3]) -> Vec<u8> {
    let mut m = Module::new();
    m.shader_capability().memory_model_glsl450().source_unknown();

    let void = m.type_void();
    let f32_ty = m.type_float();
    let u32_ty = m.type_uint();
    let bool_ty = m.type_bool();
    let v2 = m.type_vector(f32_ty, 2);
    let v4 = m.type_vector(f32_ty, 4);
    let ptr_out_v4 = m.type_pointer(SC_OUTPUT, v4);
    let ptr_in_u32 = m.type_pointer(SC_INPUT, u32_ty);
    let fn_ty = m.type_function(void, &[]);

    let out_pos = m.variable(ptr_out_v4, SC_OUTPUT);
    let in_index = m.variable(ptr_in_u32, SC_INPUT);

    // 三个顶点的分量常量
    let mut xs = Vec::new();
    let mut ys = Vec::new();
    for p in positions_ndc {
        xs.push(m.constant_f32(f32_ty, p[0]));
        ys.push(m.constant_f32(f32_ty, p[1]));
    }
    // 角点逐分量选择：i0 = 0，故 i1 = (i0 == 1)，i2 = (i0 == 2)
    let k0 = m.constant_u32(u32_ty, 0);
    let k1 = m.constant_u32(u32_ty, 1);
    let k2 = m.constant_u32(u32_ty, 2);
    let b_ty = m.type_pointer(SC_FUNCTION, bool_ty);
    let _ = b_ty;
    let z0 = m.constant_f32(f32_ty, 0.0);
    let w1 = m.constant_f32(f32_ty, 1.0);

    let fn_id = m.id();
    let block = m.id();
    m.entry_point(EXECUTION_MODEL_VERTEX, fn_id, "main", &[out_pos, in_index]);
    m.debug_name(out_pos, "gl_Position");
    m.decorate(out_pos, DECORATION_BUILT_IN, &[BUILTIN_POSITION]);
    m.debug_name(in_index, "gl_VertexIndex");
    m.decorate(in_index, DECORATION_BUILT_IN, &[BUILTIN_VERTEX_INDEX]);

    m.function(void, fn_id, fn_ty, block);
    let idx = m.load(u32_ty, in_index);
    // 逐分量选：从顶点 0 出发，依次「若 idx==k 则换成顶点 k」
    let mut cur_x = xs[0];
    let mut cur_y = ys[0];
    for k in 1..3 {
        let cond = m.op_i_equal(bool_ty, idx, if k == 1 { k1 } else { k2 });
        cur_x = m.op_select(f32_ty, cond, xs[k], cur_x);
        cur_y = m.op_select(f32_ty, cond, ys[k], cur_y);
    }
    // ⚠️ `cur_x` / `cur_y` 是 `OpSelect` 的**运行时结果** ⇒ 必须用 `OpCompositeConstruct`，
    // 不能用 `OpConstantComposite`（那会生成非法 SPIR-V，驱动不报错、只是不画）。
    let pos4 = m.composite_construct(v4, &[cur_x, cur_y, z0, w1]);
    m.store(out_pos, pos4);
    m.return_void();
    m.function_end();
    let _ = (k0, v2);
    m.finish()
}

/// 片段着色器：输出一个固定颜色。位置输入未被使用 ⇒ 模块最小。
pub fn fragment_shader_solid(color: [f32; 4]) -> Vec<u8> {
    let mut m = Module::new();
    m.shader_capability().memory_model_glsl450().source_unknown();

    let void = m.type_void();
    let f32_ty = m.type_float();
    let v4 = m.type_vector(f32_ty, 4);
    let ptr_out_v4 = m.type_pointer(SC_OUTPUT, v4);
    let out_color = m.variable(ptr_out_v4, SC_OUTPUT);
    let fn_ty = m.type_function(void, &[]);
    let fn_id = m.id();
    let block = m.id();

    m.entry_point(EXECUTION_MODEL_FRAGMENT, fn_id, "main", &[out_color]);
    m.execution_mode(fn_id, EXECUTION_MODE_ORIGIN_UPPER_LEFT, &[]);
    m.debug_name(out_color, "out_color");
    m.decorate(out_color, DECORATION_LOCATION, &[0]);

    let mut parts = Vec::new();
    for c in color {
        parts.push(m.constant_f32(f32_ty, c));
    }
    let col = m.constant_composite(v4, &parts);

    m.function(void, fn_id, fn_ty, block);
    m.store(out_color, col);
    m.return_void();
    m.function_end();
    m.finish()
}

/// 顶点着色器（**推送常量矩形**）—— ⚠️ **实验性，当前在 Intel 驱动上不可用**。
///
/// ## 状态：已知不工作（不要用它建管线）
///
/// 实测（Intel RaptorLake，Vulkan 1.4.309）：用这支着色器建图形管线时，
/// `vkCreateGraphicsPipelines` **要么返回成功但不写管线句柄（空句柄）、要么直接
/// `STATUS_ACCESS_VIOLATION`**。试过三种推送常量写法都不行：
///
/// 1. `{vec4}` struct + `Block` + `OpAccessChain` 取成员 → 返回成功、句柄为空
/// 2. 同一个 struct，改为直接 `OpLoad` 整个 Block → 返回成功、句柄为空
/// 3. 直接声明为 `vec4`（不加 Block）→ 访问违例
///
/// `vkCreateShaderModule` 对三种写法**都接受** —— 再次印证「建模块时的校验极弱，
/// 真正编译发生在建管线时」。
///
/// ## 对 M2a 的影响与后续方案
///
/// M2a-3 的验收（**图形管线能建成功**）已由不带推送常量的着色器达成：
/// `vertex_shader_empty` / `vertex_shader_const_position` / `vertex_shader_triangle`。
/// 所以**不阻塞** M2a-4..6（命令缓冲 / 离屏渲染 / 回读）。
///
/// 矩形绘制后续改为**顶点缓冲**方案（每个矩形 6 个顶点上传到缓冲）——
/// 那是标准做法、不依赖推送常量，代价是每矩形一次缓冲写。等 M2a 出图后再做。
///
/// 保留本函数是为了：① 记录这条实测教训；② 后续换驱动/换写法时有个起点。
pub fn vertex_shader_rect_pushconstant() -> Vec<u8> {
    let mut m = Module::new();
    m.shader_capability().memory_model_glsl450().source_unknown();

    let void = m.type_void();
    let f32_ty = m.type_float();
    let u32_ty = m.type_uint();
    let v4 = m.type_vector(f32_ty, 4);
    let ptr_out_v4 = m.type_pointer(SC_OUTPUT, v4);
    let ptr_in_u32 = m.type_pointer(SC_INPUT, u32_ty);
    let ptr_pc_v4 = m.type_pointer(SC_PUSH_CONSTANT, v4);
    let _ = ptr_pc_v4;    let fn_ty = m.type_function(void, &[]);

    // 推送常量：**直接声明为 vec4**（不加 Block）。
    // 规范允许把推送常量块声明成「单个标量/向量」；实测这样做驱动才接受，
    // 而声明成 `{vec4}` struct + Block 会让 `vkCreateGraphicsPipelines`
    // **返回成功却不写管线句柄**（空句柄 —— 最容易被忽略的失败方式）。
    let ptr_pc_v4 = m.type_pointer(SC_PUSH_CONSTANT, v4);
    let pc_var = m.variable(ptr_pc_v4, SC_PUSH_CONSTANT);

    let out_pos = m.variable(ptr_out_v4, SC_OUTPUT);
    let in_index = m.variable(ptr_in_u32, SC_INPUT);

    let k0 = m.constant_u32(u32_ty, 0);
    let k1 = m.constant_u32(u32_ty, 1);
    let f0 = m.constant_f32(f32_ty, 0.0);
    let f1 = m.constant_f32(f32_ty, 1.0);

    let fn_id = m.id();
    let block = m.id();
    m.entry_point(EXECUTION_MODEL_VERTEX, fn_id, "main", &[out_pos, in_index]);
    m.debug_name(out_pos, "gl_Position");
    m.decorate(out_pos, DECORATION_BUILT_IN, &[BUILTIN_POSITION]);
    m.debug_name(in_index, "gl_VertexIndex");
    m.decorate(in_index, DECORATION_BUILT_IN, &[BUILTIN_VERTEX_INDEX]);

    m.function(void, fn_id, fn_ty, block);
    // idx = gl_VertexIndex
    let idx = m.load(u32_ty, in_index);
    // x_bit = idx & 1 ; y_bit = (idx >> 1) & 1
    let x_bit = m.bitwise_and(u32_ty, idx, k1);
    // 用整数除法当右移（避免再引入移位操作码）：idx / 2
    let two = m.constant_u32(u32_ty, 2);
    let half = m.id();
    m.op_udiv(u32_ty, half, idx, two);
    let y_bit = m.bitwise_and(u32_ty, half, k1);
    // 0/1 → 0.0/1.0
    let xf = m.convert_u_to_f(f32_ty, x_bit);
    let yf = m.convert_u_to_f(f32_ty, y_bit);

    // bounds = 推送常量（一个 vec4）⇒ (x0, y0, x1, y1)
    let bounds = m.load(v4, pc_var);
    let x0 = m.composite_extract(f32_ty, bounds, &[0]);
    let y0 = m.composite_extract(f32_ty, bounds, &[1]);
    let x1 = m.composite_extract(f32_ty, bounds, &[2]);
    let y1 = m.composite_extract(f32_ty, bounds, &[3]);
    let dx = m.f_sub(f32_ty, x1, x0);
    let dy = m.f_sub(f32_ty, y1, y0);
    let px = m.f_mul(f32_ty, dx, xf);
    let py = m.f_mul(f32_ty, dy, yf);
    let fx = m.f_add(f32_ty, x0, px);
    let fy = m.f_add(f32_ty, y0, py);
    // ⚠️ `fx` / `fy` 是算术结果（运行时值）⇒ 用 `OpCompositeConstruct`。
    let pos4 = m.composite_construct(v4, &[fx, fy, f0, f1]);
    m.store(out_pos, pos4);
    m.return_void();
    m.function_end();
    let _ = k0;
    m.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(bytes: &[u8]) -> Vec<u32> {
        bytes
            .chunks_exact(4)
            .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect()
    }

    /// 逐条指令校验「首字高 16 位声明的词数」是否与总词数自洽。
    /// 这是最容易写错、且驱动只会给一个含糊错误的地方。
    fn assert_instruction_stream_well_formed(bytes: &[u8], what: &str) {
        assert_eq!(bytes.len() % 4, 0, "{what}: SPIR-V 必须 4 字节对齐");
        let w = words(bytes);
        assert_eq!(w[0], 0x0723_0203, "{what}: 魔数必须是 0x07230203");
        let mut i = 5;
        while i < w.len() {
            let wc = (w[i] >> 16) as usize;
            assert!(wc >= 1, "{what}: 第 {i} 个词的词数为 0");
            assert!(
                i + wc <= w.len(),
                "{what}: 第 {i} 个词声明 {wc} 个词，但只剩 {}",
                w.len() - i
            );
            i += wc;
        }
        assert_eq!(i, w.len(), "{what}: 指令流必须正好用完整份模块");
    }

    #[test]
    fn all_shaders_are_well_formed() {
        assert_instruction_stream_well_formed(
            &vertex_shader_triangle([[-1.0, -1.0], [3.0, -1.0], [-1.0, 3.0]]),
            "triangle vs",
        );
        assert_instruction_stream_well_formed(&fragment_shader_solid([1.0, 0.0, 0.0, 1.0]), "solid fs");
        assert_instruction_stream_well_formed(&vertex_shader_rect_pushconstant(), "rect vs");
    }

    #[test]
    fn bound_exceeds_every_id() {
        let mut m = Module::new();
        let a = m.id();
        let b = m.id();
        let c = m.id();
        let max_id = a.max(b).max(c);
        let bytes = m.finish();
        let bound = u32::from_le_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]);
        assert!(bound > max_id, "bound({bound}) 必须 > 最大 Id({max_id})");
    }

    #[test]
    fn string_literals_are_nul_terminated_and_padded() {
        let lit = Module::literal_string("main");
        assert_eq!(lit.len(), 2, "\"main\\0\" = 5 字节 ⇒ 补齐到 8 字节 = 2 个字");
        assert_eq!(lit[0].to_le_bytes(), *b"main");
        assert_eq!(lit[1].to_le_bytes(), [0, 0, 0, 0]);
        // 正好 4 的倍数时不应多补一个空字
        assert_eq!(Module::literal_string("abcd").len(), 2);
        assert_eq!(Module::literal_string("abc").len(), 1);
    }

    #[test]
    fn execution_models_are_correct() {
        let vs = words(&vertex_shader_triangle([[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]]));
        let fs = words(&fragment_shader_solid([0.0, 0.0, 0.0, 1.0]));
        let entry_model = |w: &[u32]| -> Option<u32> {
            let mut i = 5;
            while i < w.len() {
                let wc = (w[i] >> 16) as usize;
                if (w[i] & 0xffff) as u16 == OP_ENTRY_POINT {
                    return Some(w[i + 1]);
                }
                i += wc;
            }
            None
        };
        assert_eq!(entry_model(&vs), Some(EXECUTION_MODEL_VERTEX), "顶点着色器模型 = 0");
        assert_eq!(entry_model(&fs), Some(EXECUTION_MODEL_FRAGMENT), "片段着色器模型 = 4");
    }

    #[test]
    fn fragment_shader_declares_origin_upper_left() {
        let w = words(&fragment_shader_solid([0.0, 0.0, 0.0, 1.0]));
        let mut i = 5;
        let mut found = false;
        while i < w.len() {
            let wc = (w[i] >> 16) as usize;
            if (w[i] & 0xffff) as u16 == OP_EXECUTION_MODE {
                assert_eq!(w[i + 2], EXECUTION_MODE_ORIGIN_UPPER_LEFT);
                found = true;
            }
            i += wc;
        }
        assert!(found, "片段着色器必须声明 OriginUpperLeft");
    }

    #[test]
    fn rect_shader_uses_push_constants() {
        let w = words(&vertex_shader_rect_pushconstant());
        let mut i = 5;
        let mut push_const_vars = 0;
        while i < w.len() {
            let wc = (w[i] >> 16) as usize;
            if (w[i] & 0xffff) as u16 == OP_VARIABLE {
                // OpVariable resultType result storageClass
                if w[i + 3] == SC_PUSH_CONSTANT {
                    push_const_vars += 1;
                }
            }
            i += wc;
        }
        assert_eq!(push_const_vars, 1, "矩形着色器必须有一个推送常量变量");
    }

    #[test]
    fn output_variables_have_no_initializer() {
        // 带初值的 Output 全局变量会让后续读取失去「已定义」性（校验报错）
        for bytes in [
            vertex_shader_triangle([[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]]),
            vertex_shader_rect_pushconstant(),
            fragment_shader_solid([0.0, 0.0, 0.0, 1.0]),
        ] {
            let w = words(&bytes);
            let mut i = 5;
            while i < w.len() {
                let wc = (w[i] >> 16) as usize;
                if (w[i] & 0xffff) as u16 == OP_VARIABLE {
                    let storage = w[i + 3];
                    if storage == SC_OUTPUT {
                        assert_eq!(wc, 4, "Output 变量的 OpVariable 必须是 4 个词（无初值）");
                    }
                }
                i += wc;
            }
        }
    }
}
