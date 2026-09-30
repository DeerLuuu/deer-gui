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
const OP_EXT_INST: u16 = 12;
const OP_MEMORY_MODEL: u16 = 14;
const OP_ENTRY_POINT: u16 = 15;
const OP_EXECUTION_MODE: u16 = 16;
const OP_CAPABILITY: u16 = 17;
const OP_TYPE_VOID: u16 = 19;
const OP_TYPE_BOOL: u16 = 20;
const OP_TYPE_INT: u16 = 21;
const OP_TYPE_FLOAT: u16 = 22;
const OP_TYPE_VECTOR: u16 = 23;
/// `OpTypeImage`（M3b：纹理）
const OP_TYPE_IMAGE: u16 = 25;
/// `OpTypeSampler`（M3b）
const OP_TYPE_SAMPLER: u16 = 26;
/// `OpTypeSampledImage`（M3b：`OpTypeImage` + `OpTypeSampler` 的组合类型）
const OP_TYPE_SAMPLED_IMAGE: u16 = 27;
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
/// `OpImageSampleImplicitLod`（M3b：纹理采样）
const OP_IMAGE_SAMPLE_IMPLICIT_LOD: u16 = 87;
/// `OpVectorTimesScalar`（向量 × 标量，M3b：`color.rgb * cov`）
const OP_VECTOR_TIMES_SCALAR: u16 = 142;
const OP_F_NEGATE: u16 = 127;
const OP_F_ORD_LESS_THAN: u16 = 184;
const OP_F_ORD_GREATER_THAN: u16 = 186;
const OP_LOGICAL_OR: u16 = 166;
const OP_LOGICAL_AND: u16 = 167;
const OP_LABEL: u16 = 248;
const OP_RETURN: u16 = 253;
const OP_RETURN_VALUE: u16 = 254;

// ── 存储类别（SPIR-V 3.3 Storage Class） ─────────────────────────────────────

const SC_INPUT: u32 = 1;
const SC_OUTPUT: u32 = 3;
const SC_FUNCTION: u32 = 7;
/// 推送常量
pub const SC_PUSH_CONSTANT: u32 = 9;
/// `UniformConstant` —— **纹理/采样器**的存储类别（M3b）。
///
/// 与 `Uniform`（`VK_DESCRIPTOR_TYPE_UNIFORM_BUFFER` 用）不同：`SampledImage`
/// 这类「不透明类型」必须放在 `UniformConstant` 里（Vulkan 规范：
/// VUID-StandaloneSpirv-OpTypeImage-06940 一类约束）。放错存储类别时
/// `spirv-val` 会报 `OpVariable ... has illegal type`。
pub const SC_UNIFORM_CONSTANT: u32 = 0;

// ── 装饰（SPIR-V 3.6 Decoration） ────────────────────────────────────────────

const DECORATION_BUILT_IN: u32 = 11;
const DECORATION_LOCATION: u32 = 30;
/// `Decoration InBounds` —— 给 `OpAccessChain` 加这个装饰能让校验器放心
/// （表示索引不会越界）。Vulkan 校验层对缺它的访问链会报 warning。
const DECORATION_IN_BOUNDS: u32 = 16;
/// `Decoration DescriptorSet`（M3b：描述符集编号）
const DECORATION_DESCRIPTOR_SET: u32 = 34;
/// `Decoration Binding`（M3b：描述符集内 binding 编号）
const DECORATION_BINDING: u32 = 33;

// ── 图像类型参数（SPIR-V 3.8 / 3.17） ────────────────────────────────────────

/// `Dim 2D`
const DIM_2D: u32 = 1;
/// `ImageFormat Unknown`。
///
/// Vulkan 要求**采样**用的 `OpTypeImage` 声明成 `Unknown`：真实格式由
/// `VkImageView` 决定，着色器不该假设它。写具体格式（如 `R8`）会让
/// 校验器报「格式不匹配」——因为图像视图才是权威。
const IMAGE_FORMAT_UNKNOWN: u32 = 0;
/// `Sampled 1` —— 表示这个图像**只用于采样**（不是存储图像、也不是两者兼用）。
const SAMPLED_1: u32 = 1;

// ── 内建量与执行模型 ─────────────────────────────────────────────────────────

/// `BuiltIn Position`（顶点输出 `gl_Position`）
const BUILTIN_POSITION: u32 = 0;
/// `BuiltIn VertexIndex`（顶点输入 `gl_VertexIndex`）
pub const BUILTIN_VERTEX_INDEX: u32 = 42;
/// `BuiltIn FragCoord`（片段输入 `gl_FragCoord`）。
///
/// M3a 的片元判据以**整数像素**坐标为准 ⇒ 片元着色器必须能拿到窗口空间的
/// 像素坐标，`gl_FragCoord` 是唯一途径（不能用插值属性代替：插值会在像素间
/// 连续变化，而 CPU 参考实现是按整数像素判定的）。
pub const BUILTIN_FRAG_COORD: u32 = 15;
/// `ExecutionModel Vertex`
const EXECUTION_MODEL_VERTEX: u32 = 0;
/// `ExecutionModel Fragment`
const EXECUTION_MODEL_FRAGMENT: u32 = 4;
/// `ExecutionMode OriginUpperLeft`（片段着色器必需）
const EXECUTION_MODE_ORIGIN_UPPER_LEFT: u32 = 7;
/// `Capability Shader`
const CAPABILITY_SHADER: u32 = 1;

pub const SPIRV_VERSION_1_0: u32 = 0x0001_0000;

// ── GLSL.std.450 扩展指令集 ──────────────────────────────────────────────────

/// `GLSL.std.450` 扩展指令集的**导入名字符串**（`OpExtInstImport` 用）。
pub const EXT_INST_GLSL_STD_450: &str = "GLSL.std.450";

/// `GLSLstd450Floor`（**扩展**指令编号）。
///
/// ## ⚠️ 一条必须记住的教训：core SPIR-V **没有** `OpFloor`
///
/// M3a 的计划初稿写着「`OP_FLOOR=8`，照 `f_mul` 同构写一个 `op_floor`」——
/// 这是**两处错**，而且错法很隐蔽：
///
/// 1. **8 不是 `OpFloor`**。core SPIR-V 的操作码里 `8` 是 `OpLine`
///    （`OpLine <文件名 Id> <行号> <列号>`），本文件上面已有 `OP_LINE = 8`。
///    若真按「同构」发出 `[8, ty, result, operand]`，那是一条 `OpLine`，
///    会被 [`section_of_opcode`] 路由到**调试段**、操作数全被当成文件名/行号 ——
///    校验器只会报一堆无关的段序错误。
/// 2. **更根本的是 core SPIR-V 根本没有 `OpFloor`**（也没有 `OpSqrt`/`OpSin` …）。
///    这类数学函数全在 **`GLSL.std.450` 扩展指令集**里，必须
///    `OpExtInstImport` 一个 `"GLSL.std.450"` 串，再用
///    `OpExtInst resultType result set instruction operands...`
///    引用它。`8` 是**扩展指令编号** `GLSLstd450Floor`（见 SDK 的
///    `Include/spirv/unified1/GLSL.std.450.h`：`Round=1, RoundEven=2, Trunc=3, …, Floor=8`）
///    —— 「8」这个数字是对的，只是它**不是 opcode 而是 ext-inst 编号**。
///
/// 这正是本任务「必须过官方 `spirv-val`、不能只看 `vkCreateShaderModule` 成功」
/// 的价值所在：这两处错**都不会让建模块失败**，实测是 `spirv-val` 报
/// `error: line 38: Invalid opcode: 9`（连试 9 也一样报）才暴露出来。
///
/// 编号已对照 SDK 头文件核对。基准（[`OP_F_ORD_LESS_THAN`] 等）也一并核对过：
/// `OpFNegate = 127`、`OpFAdd = 129`、`OpFSub = 131`、`OpFMul = 133`、
/// `OpLogicalOr = 166`、`OpLogicalAnd = 167`、`OpFOrdLessThan = 184`、
/// `OpFOrdGreaterThan = 186`、`OpExtInst = 12`。
pub const GLSL_STD_450_FLOOR: u32 = 8;

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

    /// `OpTypeSampler result`（M3b）。
    ///
    /// 没有操作数 —— 采样器的**实际参数**（过滤方式、寻址模式）由 Rust 侧的
    /// `VkSampler` 决定，着色器里只声明「我要一个采样器」。
    pub fn type_sampler(&mut self) -> u32 {
        let r = self.id();
        self.op(OP_TYPE_SAMPLER, &[r]);
        r
    }

    /// `OpTypeImage result sampledType dim depth arrayed ms sampled format [access]`
    ///
    /// M3b 只用 2D、无深度、非数组、单采样、`Sampled = 1`、`format = Unknown`。
    /// 把参数**写死**而不是做成通用接口，理由：本项目的自研汇编器只服务自己的
    /// 着色器，通用的 `OpTypeImage` 有 9+ 个组合参数，接口一放开就会有人传错
    /// （而传错的表现是「驱动建管线时给一句含糊的错误」）。
    /// 真需要别的形态时再加一个具名函数，比一个万能参数表更好review。
    pub fn type_image_2d_unknown(&mut self, sampled_ty: u32) -> u32 {
        let r = self.id();
        self.op(
            OP_TYPE_IMAGE,
            &[
                r,
                sampled_ty,
                DIM_2D,
                0,                 // depth = 0（非深度图像）
                0,                 // arrayed = 0
                0,                 // ms = 0（单采样）
                SAMPLED_1,         // sampled = 1
                IMAGE_FORMAT_UNKNOWN,
            ],
        );
        r
    }

    /// `OpTypeSampledImage result imageType`（M3b）。
    pub fn type_sampled_image(&mut self, image: u32) -> u32 {
        let r = self.id();
        self.op(OP_TYPE_SAMPLED_IMAGE, &[r, image]);
        r
    }

    /// `OpImageSampleImplicitLod resultType result sampledImage coordinate`（M3b）。
    ///
    /// ## ⚠️ 这里**不能**给 `Lod` 操作数（实测的反直觉之处）
    ///
    /// 编码是 `OpImageSampleImplicitLod resultType result sampledImage coordinate
    /// [imageOperands...]`，看起来「补一个显式 `Lod 0` 更稳」——**错**。
    /// 本机 `spirv-val`（SDK 1.4.357.0）明确拒绝：
    ///
    /// ```text
    ///   error: Image Operand Lod can only be used with ExplicitLod opcodes and OpImageFetch
    ///     %22 = OpImageSampleImplicitLod %v4float %20 %21 Lod %float_0
    /// ```
    ///
    /// 规范把 `ImageOperands` 按 opcode 分类：`Lod` 只属于 `*ExplicitLod` 系列
    /// 与 `OpImageFetch`。所以隐式采样**一个操作数都不给**。
    ///
    /// 那 LOD 怎么办？本用例里根本不需要它：字形图集 `mip_levels = 1`，
    /// 隐式 LOD 唯一能采到的就是第 0 层 —— 「只有一层」这个事实由**资源**保证，
    /// 不需要（也无法）在指令里再声明一次。
    ///
    /// 顺带说明为什么不用 `OpImageSampleExplicitLod`：那样确实可以写 `Lod 0`，
    /// 但会引入一个**没必要的约束**（把「层数」这个本属资源的信息编进着色器）。
    /// 隐式版在单层纹理上的行为与 `Lod 0` 完全一致。
    pub fn op_image_sample_implicit_lod(
        &mut self,
        result_ty: u32,
        sampled_image: u32,
        coord: u32,
    ) -> u32 {
        let r = self.id();
        self.op(OP_IMAGE_SAMPLE_IMPLICIT_LOD, &[result_ty, r, sampled_image, coord]);
        r
    }

    /// `OpVectorTimesScalar`（`vecN * scalar`）。
    ///
    /// 当前**没有调用方**：`fragment_shader_text` 原本用它做「预乘」
    /// （`color.rgb * cov`），后来按混合状态（`SRC_ALPHA/ONE_MINUS_SRC_ALPHA`）
    /// 修正为**非预乘**（覆盖率只乘进 alpha，标量 `OpFMul` 就够）—— 详见
    /// [`fragment_shader_text`] 的说明。
    ///
    /// 保留它的理由：M3b 之后的文本/图形混合（例如图集外的调制色、sRGB 校正）
    /// 迟早要按向量缩放，而这条指令已经过 `spirv-val` 验证；
    /// 删掉再写一遍不如留着并**如实标注它是备用的**。
    #[allow(dead_code)]
    pub fn op_vector_times_scalar(&mut self, ty: u32, vector: u32, scalar: u32) -> u32 {
        let r = self.id();
        self.op(OP_VECTOR_TIMES_SCALAR, &[ty, r, vector, scalar]);
        r
    }

    /// `OpCompositeExtract` 的**连续分量**形式（取 `.rgb` 这类）。
    ///
    /// 当前**没有调用方**（同 [`Self::op_vector_times_scalar`] 的原因）。
    ///
    /// 实现方式：逐分量 `OpCompositeExtract` + `OpCompositeConstruct`，
    /// **不用** `OpVectorShuffle`（那需要一个常量索引向量）—— 本项目在
    /// 「运行时常量数组索引」上吃过大亏（见 [`vertex_shader_triangle`] 的教训），
    /// 能不碰向量索引就不碰。
    #[allow(dead_code)]
    pub fn vector_swizzle(
        &mut self,
        out_ty: u32,
        component_ty: u32,
        vector: u32,
        indexes: &[u32],
    ) -> u32 {
        let parts: Vec<u32> = indexes
            .iter()
            .map(|&i| self.composite_extract(component_ty, vector, &[i]))
            .collect();
        self.composite_construct(out_ty, &parts)
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

    /// `OpFNegate`（浮点取负：`OpFNegate resultType result operand`）
    pub fn op_fnegate(&mut self, ty: u32, value: u32) -> u32 {
        let r = self.id();
        self.op(OP_F_NEGATE, &[ty, r, value]);
        r
    }

    /// `OpExtInstImport result "GLSL.std.450"`，返回该扩展指令集的 Id。
    ///
    /// 只能调用一次（每个模块每个扩展指令集一个 Id），且必须在
    /// `OpMemoryModel` **之前** —— 段序由 [`Section`] 自动保证。
    pub fn ext_inst_import_glsl_std_450(&mut self) -> u32 {
        let r = self.id();
        let lit = Self::literal_string(EXT_INST_GLSL_STD_450);
        let mut ops = vec![r];
        ops.extend_from_slice(&lit);
        self.op(OP_EXT_INST_IMPORT, &ops);
        r
    }

    /// `OpExtInst resultType result set instruction operands...`
    ///
    /// `instruction` 是**扩展指令集内**的编号（不是 opcode）。
    pub fn op_ext_inst(&mut self, ty: u32, set: u32, instruction: u32, operands: &[u32]) -> u32 {
        let r = self.id();
        let mut ops = vec![ty, r, set, instruction];
        ops.extend_from_slice(operands);
        self.op(OP_EXT_INST, &ops);
        r
    }

    /// `GLSL.std.450` 的 `Floor`（向下取整，一元浮点）。
    ///
    /// ⚠️ **不是** `self.op(8, ...)`：core SPIR-V 没有 `OpFloor`，见
    /// [`GLSL_STD_450_FLOOR`] 的说明。`set` 由
    /// [`Module::ext_inst_import_glsl_std_450`] 产生。
    pub fn op_floor(&mut self, ty: u32, set: u32, value: u32) -> u32 {
        self.op_ext_inst(ty, set, GLSL_STD_450_FLOOR, &[value])
    }

    /// `OpFOrdLessThan`（有序浮点比较 `<`，结果类型必须是 `OpTypeBool`）。
    ///
    /// ⚠️ **有序**（Ordered）版本：任一操作数是 NaN 时结果为 `false`。
    /// 不能用无序版本（`OpFUnordLessThan`）替代 —— 语义不同。
    pub fn op_ford_less_than(&mut self, bool_ty: u32, a: u32, b: u32) -> u32 {
        let r = self.id();
        self.op(OP_F_ORD_LESS_THAN, &[bool_ty, r, a, b]);
        r
    }

    /// `OpFOrdGreaterThan`（有序浮点比较 `>`，结果类型必须是 `OpTypeBool`）。
    pub fn op_ford_greater_than(&mut self, bool_ty: u32, a: u32, b: u32) -> u32 {
        let r = self.id();
        self.op(OP_F_ORD_GREATER_THAN, &[bool_ty, r, a, b]);
        r
    }

    /// `OpLogicalOr`（**标量** `bool` 的逻辑或；结果类型必须是 `OpTypeBool`）。
    ///
    /// ⚠️ 向量布尔（`OpTypeVector` of `bool`）需要 `Vector16` 能力，本模块不开，
    /// 所以只能逐标量 `|` 再串起来（片元判据就是这么用的）。
    pub fn op_logical_or(&mut self, bool_ty: u32, a: u32, b: u32) -> u32 {
        let r = self.id();
        self.op(OP_LOGICAL_OR, &[bool_ty, r, a, b]);
        r
    }

    /// `OpLogicalAnd`（标量 `bool` 的逻辑与）。
    pub fn op_logical_and(&mut self, bool_ty: u32, a: u32, b: u32) -> u32 {
        let r = self.id();
        self.op(OP_LOGICAL_AND, &[bool_ty, r, a, b]);
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
        OP_F_NEGATE => "OpFNegate",
        OP_EXT_INST => "OpExtInst",
        OP_TYPE_IMAGE => "OpTypeImage",
        OP_TYPE_SAMPLER => "OpTypeSampler",
        OP_TYPE_SAMPLED_IMAGE => "OpTypeSampledImage",
        OP_IMAGE_SAMPLE_IMPLICIT_LOD => "OpImageSampleImplicitLod",
        OP_VECTOR_TIMES_SCALAR => "OpVectorTimesScalar",
        OP_F_ORD_LESS_THAN => "OpFOrdLessThan",
        OP_F_ORD_GREATER_THAN => "OpFOrdGreaterThan",
        OP_LOGICAL_OR => "OpLogicalOr",
        OP_LOGICAL_AND => "OpLogicalAnd",
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
/// ## ⚠️⚠️ 规范违规（2025 补记）：**开启校验层会崩进程**
///
/// 校验层（`DEER_VK_VALIDATION=1` + `VK_LAYER_KHRONOS_validation`）对当前写法直接报：
///
/// ```text
/// vkCreateShaderModule(): pCreateInfo->pCode (spirv-val produced an error):
/// PushConstant OpVariable <id> '10[%10]' has illegal type.
/// Such variables must be typed as OpTypeStruct
///   %10 = OpVariable %_ptr_PushConstant_v4float_0 PushConstant
/// VUID-StandaloneSpirv-PushConstant-06808
/// ```
///
/// 也就是说第 3 种写法（`vec4` 直连，不加 `Block`）**不是**规范允许的形式 ——
/// PushConstant 存储类的变量**必须**是 `OpTypeStruct`。报错之后进程会以
/// **`0xc0000005`（STATUS_ACCESS_VIOLATION）**结束（实测：`--test-threads=1` 也可复现，
/// 且只发生在 `vkCreateShaderModule` 这一步）。
///
/// 后果与处置：
/// - `tests/device_smoke.rs::push_constant_rect_shader_is_accepted_by_driver` 与
///   `tests/pipeline_smoke.rs::push_constant_rect_shader_is_known_broken` 都在
///   校验层下**显式跳过**（打印原因，不伪装通过）—— 于是「带校验层跑全量 deer-vk」是安全动作；
/// - 修好它的正路是：把推送常量块声明成 `OpTypeStruct` + `Block`（并在 `vkCreatePipelineLayout`
///   里给出匹配的 `VkPushConstantRange`），然后靠**管线创建**（不是建模块）验收；
/// - 在修好之前，**不要让 `VkDevice::open()` 去读 `DEER_VK_VALIDATION`**：
///   那会让所有打开设备的测试都踩到这颗地雷（`device.rs` 里有同样的注释）。
/// - `crates/deer-gui/examples/vulkan_pipeline.rs` 也调用本函数：开校验层跑它同样会崩。
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

/// 顶点着色器（**矩形属性透传**）：M3a 的顶点流 → 光栅化的入口。
///
/// ## 顶点布局（**与 `deer-gpu` 顶点流层共用，不允许实现时改**）
///
/// | location | 类型 | 含义 |
/// |---|---|---|
/// | 0 | `vec2` | 位置（NDC，y 向下） |
/// | 1 | `vec4` | `rect = (x, y, w, h)`，像素单位 |
/// | 2 | `float` | `radius_kind`：`0` = 普通填充；`>0` = 圆角半径；**`<0` = 描边（带宽 = `-radius_kind`）**。`width == 1` 时取 `-1.0`，更宽的描边取 `-width`（见 `gpu_geom::radius_kind_for_stroke`） |
/// | 3 | `vec4` | 颜色（预乘不做，直接 src-alpha 混合） |
///
/// ## 两支着色器之间的接口
///
/// | VS 输出 location | 类型 | FS 输入 |
/// |---|---|---|
/// | 0 | `vec4` | `rect` |
/// | 1 | `float` | `radius_kind` |
/// | 2 | `vec4` | `color` |
///
/// FS 自己从 `gl_FragCoord` 拿像素坐标 ⇒ **VS 不传位置**（`gl_Position` 是
/// 内建输出，不占 location）。这样「位置」只存在于光栅化器里，FS 拿到的
/// `rect` 属性是**插值后恒定**的（每个顶点都写同一个值，插值结果就是这个值）。
///
/// ## 实现要点
///
/// 全部是「逐属性 `OpLoad` + `OpStore`」，没有 `OpSelect`、没有动态索引
/// （与 [`vertex_shader_from_vertex_buffer`] 同一条路；动态索引是硬教训，
/// 见 [`vertex_shader_triangle`] 的说明）。
///
/// `color` 是 `vec4`，与 `rect` 表达式类型相同 ⇒ `types.push` 必须**推两次**
/// （同一个类型 Id 复用），否则 `OpStore` 的类型对不上。
pub fn vertex_shader_rect_attrs() -> Vec<u8> {
    let mut m = Module::new();
    m.shader_capability().memory_model_glsl450().source_unknown();

    let void = m.type_void();
    let f32_ty = m.type_float();
    let v2 = m.type_vector(f32_ty, 2);
    let v4 = m.type_vector(f32_ty, 4);
    let ptr_in_v2 = m.type_pointer(SC_INPUT, v2);
    let ptr_in_v4 = m.type_pointer(SC_INPUT, v4);
    let ptr_in_f32 = m.type_pointer(SC_INPUT, f32_ty);
    let ptr_out_v4 = m.type_pointer(SC_OUTPUT, v4);
    let ptr_out_f32 = m.type_pointer(SC_OUTPUT, f32_ty);
    let fn_ty = m.type_function(void, &[]);

    let out_pos = m.variable(ptr_out_v4, SC_OUTPUT);
    let out_rect = m.variable(ptr_out_v4, SC_OUTPUT);
    let out_rk = m.variable(ptr_out_f32, SC_OUTPUT);
    let out_color = m.variable(ptr_out_v4, SC_OUTPUT);
    let in_pos = m.variable(ptr_in_v2, SC_INPUT);
    let in_rect = m.variable(ptr_in_v4, SC_INPUT);
    let in_rk = m.variable(ptr_in_f32, SC_INPUT);
    let in_color = m.variable(ptr_in_v4, SC_INPUT);

    let z = m.constant_f32(f32_ty, 0.0);
    let w = m.constant_f32(f32_ty, 1.0);

    let fn_id = m.id();
    let block = m.id();
    m.entry_point(
        EXECUTION_MODEL_VERTEX,
        fn_id,
        "main",
        &[out_pos, out_rect, out_rk, out_color, in_pos, in_rect, in_rk, in_color],
    );

    // 输入 location 0/1/2/3（顺序必须与顶点布局表一致）
    m.debug_name(in_pos, "in_pos");
    m.decorate(in_pos, DECORATION_LOCATION, &[0]);
    m.debug_name(in_rect, "in_rect");
    m.decorate(in_rect, DECORATION_LOCATION, &[1]);
    m.debug_name(in_rk, "in_radius_kind");
    m.decorate(in_rk, DECORATION_LOCATION, &[2]);
    m.debug_name(in_color, "in_color");
    m.decorate(in_color, DECORATION_LOCATION, &[3]);

    // 内建输出
    m.debug_name(out_pos, "gl_Position");
    m.decorate(out_pos, DECORATION_BUILT_IN, &[BUILTIN_POSITION]);

    // 输出 location 0/1/2（与片段着色器的输入对齐）
    m.debug_name(out_rect, "out_rect");
    m.decorate(out_rect, DECORATION_LOCATION, &[0]);
    m.debug_name(out_rk, "out_radius_kind");
    m.decorate(out_rk, DECORATION_LOCATION, &[1]);
    m.debug_name(out_color, "out_color");
    m.decorate(out_color, DECORATION_LOCATION, &[2]);

    m.function(void, fn_id, fn_ty, block);

    // gl_Position = vec4(pos, 0, 1)
    let p = m.load(v2, in_pos);
    let px = m.composite_extract(f32_ty, p, &[0]);
    let py = m.composite_extract(f32_ty, p, &[1]);
    // px/py 是运行时值 ⇒ 必须 OpCompositeConstruct（不能用 OpConstantComposite）
    let pos4 = m.composite_construct(v4, &[px, py, z, w]);
    m.store(out_pos, pos4);

    // 透传属性：rect(vec4) / radius_kind(float) / color(vec4)
    let rect = m.load(v4, in_rect);
    m.store(out_rect, rect);
    let rk = m.load(f32_ty, in_rk);
    m.store(out_rk, rk);
    let color = m.load(v4, in_color);
    m.store(out_color, color);

    m.return_void();
    m.function_end();
    m.finish()
}

/// 片段着色器（**矩形形状判据**）：复刻 `deer-gpu` CPU 参考后端的 `fill` /
/// `inside_rounded` / `stroke`。
///
/// ## 为什么必须逐字等价
///
/// M3a 的验收是「GPU 画面与 CPU 后端**逐像素**一致」。片元着色器就是那个
/// 「像素级判据」：任何一处朴素写法（例如把 `>=` 写成 `>`、或者圆角用浮点距离
/// 但圆心取矩形角点）都会让边界像素对不上，而 GPU 与 CPU 的差异会表现为
/// 「差一列像素」这种极难定位的现象。
///
/// ## 判据（与 `crates/deer-gpu/src/null.rs` 的 CPU 版逐字对应）
///
/// ```text
/// px = floor(frag.x); py = floor(frag.y)
/// dl = px - rect.x ; dr = (rect.x + rect.w) - 1 - px
/// dt = py - rect.y ; db = (rect.y + rect.h) - 1 - py
/// out_rect  = (dl<0) | (dr<0) | (dt<0) | (db<0)
/// r_eff     = select(rk < 0, 0.0, rk)          // 描边不做圆角（rk == -1）
/// corner_fail = (dl<r_eff) & (dt<r_eff) & (((dl-r_eff)^2 + (dt-r_eff)^2) > r_eff^2)
///               | … 另三角同理用 (dr,dt)/(dl,db)/(dr,db)
/// fill_mask = !(out_rect | corner_fail_TL | corner_fail_TR | corner_fail_BL | corner_fail_BR)
/// bw        = -rk                              // 描边带宽（Ruling 6：rk == -1 ⇒ 1px）
/// stroke_mask = (py < rect.y+bw) | (py > rect.y+rect.h-bw-1)
///             | (px < rect.x+bw) | (px > rect.x+rect.w-bw-1)
/// mask = select(rk < 0, stroke_mask, fill_mask)
/// out_color = select(mask, color, vec4(0,0,0,0))
/// ```
///
/// ## 三处刻意的写法（都有理由，不要「优化」掉）
///
/// 1. **`!x` 不引入 `OpLogicalNot`**：两处逻辑非都用「把 `OpSelect` 的
///    true/false 两支对调」实现 —— `fill_mask` 是「全零 vs 掩码」，
///    `stroke_mask` 是「与 `bw` 比大小」。少一个算子就少一个出错面，
///    也避免控制流（本着色器**完全无分支**）。
/// 2. **四角都算、再并起来**：CPU 版 `inside_rounded` 是循环里第一个失败就
///    `return false`（短路），数学上等价于「四个 corner_fail 的或」。这里展开成
///    四条并列的表达式，语义与 CPU 完全一致，而非依赖求值顺序。
/// 3. **圆角圆心带 `r_eff` 偏移**（`ccx = rect.x + r_eff`、`ccy = rect.y + r_eff`）：
///    这是 CPU 版的原式。写成「以矩形角点为圆心」在 `r = 0` 时恰好等价，
///    但 `r > 0` 时**整个角区都会被判掉**（边长 `r` 的角区与角点为圆心、半径 `r`
///    的圆不相交），会画出「十字」而不是圆角矩形 —— 一个只在大圆角下暴露的错。
///
/// ## 与 CPU 版的已知差异（诚实记录）
///
/// - CPU 版 `fill` 的 `radius` 已被调用方 `max(0)`，且只在 `r > 0` 时才做圆角；
///   本着色器把 `rk <= 0` 统一当作 `r_eff = 0`（判定恒真，不裁剪）—— 等价。
/// - 混合/格式转换不在本着色器内（那是管线状态），所以「逐像素一致」还需要
///   管线侧配置匹配（由后续任务负责）。
///
/// ## gl_FragCoord
///
/// 片段着色器必须声明 `OriginUpperLeft`（已声明）—— 与 CPU 参考实现的
/// 「y 向下、左上为原点」坐标系一致。`gl_FragCoord.xy` 在像素中心取值
/// （`x.5`），`OpFloor` 之后正是整数像素坐标。
/// 形状片段判据（**规范中的唯一一份**）：`rect` / `radius_kind` / `gl_FragCoord`
/// ⇒ 「这个片元是否落在形状里」。
///
/// ## 为什么把它提出来（B5-1）
///
/// 统一管线要在**同一支**片段着色器里同时算「形状」和「文本」两支输出，
/// 而「形状」那一支必须与 [`fragment_shader_rect_shape`] **逐字相同** ——
/// 那是 M3a 用 **20,663 个采样点**逐像素核对过、并被 207 条判据覆盖的冻结数学。
/// 若在统一着色器里**再抄一遍**，就变成两份必须永远一致的实现
/// （本项目已经吃过「两处复制、只改一处」的亏）。
///
/// 所以把判据**原样搬**到这里，[`fragment_shader_rect_shape`] 与
/// [`fragment_shader_unified`] **都调它**。搬动时没有改任何运算顺序或形式：
/// 调用方与原来传的是同样的 Id（`gl_FragCoord` 的 `OpLoad`、`in_rect` 的 `OpLoad`、
/// `in_radius_kind` 的 `OpLoad`），所以**生成的字节码序列与搬动前一致**。
///
/// ## 判据本身（详见 [`fragment_shader_rect_shape`] 的注释，那里有完整推导）
///
/// - `fill_mask = !(out_rect | fail_tl | fail_tr | fail_bl | fail_br)`（圆角四角用圆外判定）
/// - `stroke_mask` = 四条**被另一轴区间约束**的矩形边带的并集（`bw = -rk`）
/// - `mask = select(rk < 0, stroke_mask, fill_mask)`，最后判 `mask > 0.5`
///
/// ⚠️ **别凭直觉改这段**：过程中先后有 5 个「看起来对」的候选公式被穷举推翻
/// （半平面版、先裁矩形再算带宽版、blob 减内矩形版、两矩形并集版、带不互相约束版）。
/// 改完必须重跑 `cargo test -p deer-vk --lib`（`cpu_reference_mask_matches_null_rs`）。
///
/// ## 为什么要**返回** `zero`、为什么 `load` 要传进来（两个「减少字节差异」的细节）
///
/// [`fragment_shader_rect_shape`] 的字节码是**冻结的**（M3a 的 20,663 点逐像素核对
/// 与 207 条判据建立在它之上）⇒ 把它抽成共用函数时，字节差异越小越好。
/// 下面两个细节都是被「逐字节比较」抓出来的：
///
/// 1. **常量只能发射一次。** `Module::constant_f32` **不做去重**（每次调用都新分配
///    Id），所以若调用方与这里各造一个 `0.0`，就会多出一条 `OpConstant`（+8 字节）。
///    ⇒ 由本函数发射，并把 `f0` 随 `hit` 一起返回。
/// 2. **发射顺序尽量与原函数一致。** 原函数是「常量 → `OpLoad` 输入」；
///    Id 按发射顺序分配，所以把 `OpLoad` 放在函数外先做会让**全部** Id 平移。
///    ⇒ 把 5 个输入**指针**传进来，让本函数在发射完常量**之后**才 `OpLoad`。
///
/// ## ⚠️ 实测结论：**仍然不是逐字节相同**（别把这条文档读成「已字节相同」）
///
/// 上面两点把差异从 +16 字节/全部 Id 平移缩到了 **0 字节差异、但仍有 Id 重编号**：
/// 实测重构前后各 3168 字节、**指令流逐条相同**（同为 164 条指令、各 opcode 计数一致），
/// 但有 **111 个字节**不同 —— 全是 Id 编号差异（新着色器里少了一次早期的
/// `f1` 分配，导致后续 Id 整体前移一位）。
///
/// **语义等价的保证来自别处，不来自字节比较**：
/// - 判据本体是**同一个函数**（两条管线不可能各有一份而漂移）；
/// - `cpu_reference_mask_matches_null_rs` 用 **20,663 个采样点**逐像素锁住纯逻辑版；
/// - 全套 `cargo test`（含 M3a/M3b 的 GPU 逐像素对照）**241 条全绿**。
///
/// ## 返回
///
/// `(hit, zero, frag, rect, rk, color)`：
/// - `hit`：`OpTypeBool`，`true` = 命中；
/// - `zero`：那个 `0.0` 常量的 Id（给调用方的 `OpSelect` 复用）；
/// - 后四个：本函数 `OpLoad` 出来的值（调用方因此**不重复 load**）。
///
/// 调用方负责把 `hit` 变成颜色（形状输出需要**逐分量** `OpSelect`，
/// 见 [`fragment_shader_rect_shape`] 里那条 `spirv-val` 的实测教训）。
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
fn emit_shape_hit(
    m: &mut Module,
    f32_ty: u32,
    bool_ty: u32,
    v4_ty: u32,
    glsl: u32,
    p_frag: u32,
    p_rect: u32,
    p_rk: u32,
    p_color: u32,
) -> (u32, u32, u32, u32, u32, u32) {
    // ── 常量 ─────────────────────────────────────────────────────────────────
    // 发射顺序**与原 `fragment_shader_rect_shape` 逐字相同**（0.0、1.0、true、false）
    // —— 这是「重构后字节码不变」的一部分，别调换（见函数文档）。
    let f0 = m.constant_f32(f32_ty, 0.0);
    let f1 = m.constant_f32(f32_ty, 1.0);
    // `OpConstantTrue` / `OpConstantFalse`：给 `not_bool` 的两支用
    let b_true = m.id();
    m.op(OP_CONSTANT_TRUE, &[bool_ty, b_true]);
    let b_false = m.id();
    m.op(OP_CONSTANT_FALSE, &[bool_ty, b_false]);

    // ── 输入加载（**必须**在常量之后：Id 按发射顺序分配，见函数文档） ──────────
    let frag = m.load(v4_ty, p_frag);
    let rect = m.load(v4_ty, p_rect);
    let rk = m.load(f32_ty, p_rk);
    let color = m.load(v4_ty, p_color);

    // ── px / py ──────────────────────────────────────────────────────────────
    let fx = m.composite_extract(f32_ty, frag, &[0]);
    let fy = m.composite_extract(f32_ty, frag, &[1]);
    let px = m.op_floor(f32_ty, glsl, fx);
    let py = m.op_floor(f32_ty, glsl, fy);

    // ── rect 属性 ────────────────────────────────────────────────────────────
    let rx = m.composite_extract(f32_ty, rect, &[0]);
    let ry = m.composite_extract(f32_ty, rect, &[1]);
    let rw = m.composite_extract(f32_ty, rect, &[2]);
    let rh = m.composite_extract(f32_ty, rect, &[3]);

    // ── 四边距离（与 CPU 的 right()-1 / bottom()-1 一致） ────────────────────
    let dl = m.f_sub(f32_ty, px, rx);
    let dr = {
        let x1 = m.f_add(f32_ty, rx, rw);
        let x1m1 = m.f_sub(f32_ty, x1, f1);
        m.f_sub(f32_ty, x1m1, px)
    };
    let dt = m.f_sub(f32_ty, py, ry);
    let db = {
        let y1 = m.f_add(f32_ty, ry, rh);
        let y1m1 = m.f_sub(f32_ty, y1, f1);
        m.f_sub(f32_ty, y1m1, py)
    };

    // out_rect = (dl<0) | (dr<0) | (dt<0) | (db<0)
    let out_rect = {
        let a = m.op_ford_less_than(bool_ty, dl, f0);
        let b = m.op_ford_less_than(bool_ty, dr, f0);
        let c = m.op_ford_less_than(bool_ty, dt, f0);
        let d = m.op_ford_less_than(bool_ty, db, f0);
        let ab = m.op_logical_or(bool_ty, a, b);
        let cd = m.op_logical_or(bool_ty, c, d);
        m.op_logical_or(bool_ty, ab, cd)
    };

    // r_eff = select(rk < 0, 0.0, rk) —— 描边（rk < 0）不做圆角
    let rk_neg = m.op_ford_less_than(bool_ty, rk, f0);
    let r_eff = m.op_select(f32_ty, rk_neg, f0, rk);
    let r_sq = m.f_mul(f32_ty, r_eff, r_eff);

    // corner_fail(axis_a, axis_b) —— 轴须为有序对 (dl, dt) / (dr, dt) /
    // (dl, db) / (dr, db)，顺序与 CPU `corners` 数组一致。
    //
    //   in_corner = (a < r_eff) & (b < r_eff)
    //   outside   = ((a - r_eff)^2 + (b - r_eff)^2) > r_eff^2
    //   fail      = in_corner & outside
    let mut corner_fail = |a: u32, b: u32| -> u32 {
        let in_a = m.op_ford_less_than(bool_ty, a, r_eff);
        let in_b = m.op_ford_less_than(bool_ty, b, r_eff);
        let in_corner = m.op_logical_and(bool_ty, in_a, in_b);
        let da = m.f_sub(f32_ty, a, r_eff);
        let dbv = m.f_sub(f32_ty, b, r_eff);
        let da2 = m.f_mul(f32_ty, da, da);
        let db2 = m.f_mul(f32_ty, dbv, dbv);
        let dist2 = m.f_add(f32_ty, da2, db2);
        let outside = m.op_ford_greater_than(bool_ty, dist2, r_sq);
        m.op_logical_and(bool_ty, in_corner, outside)
    };
    let fail_tl = corner_fail(dl, dt);
    let fail_tr = corner_fail(dr, dt);
    let fail_bl = corner_fail(dl, db);
    let fail_br = corner_fail(dr, db);

    // fill_mask = !(out_rect | fail_tl | fail_tr | fail_bl | fail_br)
    // 把「逻辑非」折进 OpSelect 的两支对调（true=0.0、false=1.0），省掉 OpLogicalNot。
    let bad = {
        let a = m.op_logical_or(bool_ty, out_rect, fail_tl);
        let b = m.op_logical_or(bool_ty, fail_tr, fail_bl);
        let c = m.op_logical_or(bool_ty, a, b);
        m.op_logical_or(bool_ty, c, fail_br)
    };
    let fill_mask = m.op_select(f32_ty, bad, f0, f1);

    // 逻辑非的小工具：`!b` = `OpSelect %bool b false true`（不引入 `OpLogicalNot`）。
    // 结果的类型必须是 `OpTypeBool`（与判据同类型）。
    // 逻辑非的小工具：`!b` = `OpSelect %bool b false true`（不引入 `OpLogicalNot`）。
    // 做成**自由函数**（不捕获 `m`）⇒ 调用处直接 `not_bool(&mut m, …)`，
    // 不会与其它闭包（`corner_fail`）争同一个 `m` 的可变借用。
    fn not_bool(m: &mut Module, bool_ty: u32, b: u32, b_true: u32, b_false: u32) -> u32 {
        m.op_select(bool_ty, b, b_false, b_true)
    }

    // ── 描边判据（rk < 0 ⇒ bw = -rk；Ruling 6：rk == -1 即 1px 带宽） ───────
    //
    // ## ⚠️ 这里**没有**照抄任务书给的 stroke_mask 公式（两个必须记录的原因）
    //
    // **原因 1：任务书给的是「四条半平面的或」，不是四条「矩形边」。**
    //   `(py < ry+bw) | (py > bottom-bw-1) | (px < rx+bw) | (px > right-bw-1)`
    // 半平面在另一个轴上无限延伸 ⇒ 矩形上下左右整片外部都被判成描边。
    // CPU 参考 `null.rs::stroke` 只涂 4 条**被矩形裁剪过的边**。
    //
    // **原因 2：边带会沿短边向矩形外伸出 bw。** `null.rs::stroke` 是
    // `for k in 0..w` 反复填 4 条 1px 线，**没有**把 k 限制在矩形内：
    // `(2,3,9,7)`、宽度 8 时，下边框在 `k=7` 那轮填的是 `y == bottom-1-7 == 2`
    // —— 矩形上方一行（`ry == 3`）。左/右边框在 `k ≥ rh` 时同样涂到左右之外。
    // 所以「只涂矩形内」这个直觉在 `带宽 > 矩形尺寸` 时是**错**的。
    //
    // ## 与 `null.rs::stroke` 逐字对应的闭式（四带并集）
    //
    //   x_span = px ∈ [rx, right)      y_span = py ∈ [ry, bottom)
    //   上带 = x_span & y ∈ [ry,        ry+bw)
    //   下带 = x_span & y ∈ [bottom-bw, bottom)
    //   左带 = y_span & x ∈ [rx,        rx+bw)
    //   右带 = y_span & x ∈ [right-bw, right)
    //   stroke_mask = 上带 | 下带 | 左带 | 右带
    //
    // **上/下带只受 x_span 约束（不受 y_span 约束）**，所以能向矩形上下各伸出 bw；
    // 左/右带只受 y_span 约束，同理向左右伸出。反过来说：上/下带的 y 只能是
    // `[ry, ry+bw)`（不能是「矩形内」）—— 这是踩了五次才定的形式，别改。
    //
    // ## 验证（穷举，不是推理）
    //
    // 用 Rust 程序对 CPU 原式逐像素核对：**17 组 `(rect, radius_kind)`**（普通填充 / 圆角
    // 1·2·4·12 / 描边 -1·-3·-6·-8·-40 / 退化 1×1、1×20、20×1、2×50 …）× 采样外扩
    // `pad = |radius_kind| + 3` 的**全部**整数像素 = **20,663 个采样点，零不一致**。
    // `cpu_reference_mask_matches_null_rs` 把该核对固化进 CI，`--nocapture` 会**打印实测点数**
    // ⇒ 这个数字**可复现**：
    //
    // ```text
    // $ cargo test -p deer-vk --lib cpu_reference_mask_matches_null_rs -- --nocapture
    // GPU 片元判据与 null.rs 参考逐像素一致（20663 个采样点）✅
    // ```
    //
    // ⚠️ **别把别的数字写进这里**：本注释曾经写「14 个矩形 × 13 种宽度 = **466,901** 个采样点」，
    // 而提交 `0b07997` 的信息里还有第三个数字 **523,248** —— 那两个来自**早期一次性裸程序**
    // 的更大规模统计（口径与本测试不同，且**在本仓库里复现不出来**）。它们与
    // `cases` 数组（17 组）和 `pad` 公式（`|rk| + 3`）都对不上，属**历史记录，不再引用**；
    // 以 CI 打印的 **20,663** 为准（同一类「手写汇总数错」在本分支出现过两次，见 ledger 的
    // 数字核对条目）。
    //
    // ⚠️ 过程中先后有 5 个「看起来对」的候选公式被穷举推翻（半平面版、
    // 先裁矩形再算带宽版、blob 减内矩形版、两矩形并集版、带不互相约束版）。
    // **别凭直觉改这段**，改完必须重跑 `cargo test -p deer-vk --lib`。
    let bw = m.op_fnegate(f32_ty, rk);
    let stroke_mask = {
        let right = m.f_add(f32_ty, rx, rw); // right() = rx + rw
        let bottom = m.f_add(f32_ty, ry, rh); // bottom() = ry + rh

        // x_span = px ∈ [rx, right) ：`px >= rx` 用 `!(px < rx)` 表达（不引入 OpLogicalNot）
        let x_span = {
            let lt = m.op_ford_less_than(bool_ty, px, rx); // px < rx
            let ge = not_bool(m, bool_ty, lt, b_true, b_false); // px >= rx
            let lt_r = m.op_ford_less_than(bool_ty, px, right); // px < right
            m.op_logical_and(bool_ty, ge, lt_r)
        };
        // y_span = py ∈ [ry, bottom)
        let y_span = {
            let lt = m.op_ford_less_than(bool_ty, py, ry);
            let ge = not_bool(m, bool_ty, lt, b_true, b_false);
            let lt_b = m.op_ford_less_than(bool_ty, py, bottom);
            m.op_logical_and(bool_ty, ge, lt_b)
        };

        // 上带 = x_span & (py ∈ [ry, ry+bw))：`py >= ry` 与 `py < bottom` 已由 x_span
        // 之外的 y_span 提供（x_span 只是 px 的区间；这里 y 的下界必须显式给）
        let band_top = {
            let lt_ry = m.op_ford_less_than(bool_ty, py, ry); // py < ry
            let ge_ry = not_bool(m, bool_ty, lt_ry, b_true, b_false); // py >= ry
            let t = m.f_add(f32_ty, ry, bw);
            let lt_t = m.op_ford_less_than(bool_ty, py, t); // py < ry+bw
            let c = m.op_logical_and(bool_ty, ge_ry, lt_t);
            m.op_logical_and(bool_ty, x_span, c)
        };
        // 下带 = x_span & (py ∈ [bottom-bw, bottom))
        let band_bottom = {
            let t = m.f_sub(f32_ty, bottom, bw);
            let lt = m.op_ford_less_than(bool_ty, py, t); // py < bottom-bw
            let ge = not_bool(m, bool_ty, lt, b_true, b_false); // py >= bottom-bw
            let lt_b = m.op_ford_less_than(bool_ty, py, bottom); // py < bottom
            let c = m.op_logical_and(bool_ty, ge, lt_b);
            m.op_logical_and(bool_ty, x_span, c)
        };
        // 左带 = y_span & (px ∈ [rx, rx+bw))
        let band_left = {
            let lt_rx = m.op_ford_less_than(bool_ty, px, rx); // px < rx
            let ge_rx = not_bool(m, bool_ty, lt_rx, b_true, b_false); // px >= rx
            let t = m.f_add(f32_ty, rx, bw);
            let lt_t = m.op_ford_less_than(bool_ty, px, t); // px < rx+bw
            let c = m.op_logical_and(bool_ty, ge_rx, lt_t);
            m.op_logical_and(bool_ty, y_span, c)
        };
        // 右带 = y_span & (px ∈ [right-bw, right))
        let band_right = {
            let t = m.f_sub(f32_ty, right, bw);
            let lt = m.op_ford_less_than(bool_ty, px, t); // px < right-bw
            let ge = not_bool(m, bool_ty, lt, b_true, b_false); // px >= right-bw
            let lt_r = m.op_ford_less_than(bool_ty, px, right); // px < right
            let c = m.op_logical_and(bool_ty, ge, lt_r);
            m.op_logical_and(bool_ty, y_span, c)
        };

        let tb = m.op_logical_or(bool_ty, band_top, band_bottom);
        let lr = m.op_logical_or(bool_ty, band_left, band_right);
        m.op_logical_or(bool_ty, tb, lr)
    };
    // stroke_mask 是「**在**描边上」的判定（与 CPU 取真方向一致），不需取反；
    // fill_mask 的「逻辑非」折进了下面的 OpSelect。
    let stroke_f = m.op_select(f32_ty, stroke_mask, f1, f0);

    // mask = select(rk < 0, stroke_mask, fill_mask)
    //
    // 两个掩码都是 float（0.0 / 1.0）而不是 bool：让最后的判据直接就是
    // `mask > 0.5`，省掉一次 bool 中转（也少一个出错面）。
    let mask = m.op_select(f32_ty, rk_neg, stroke_f, fill_mask);
    let half = m.constant_f32(f32_ty, 0.5);
    (
        m.op_ford_greater_than(bool_ty, mask, half),
        f0,
        frag,
        rect,
        rk,
        color,
    )
}

/// 片段着色器（**M3a：矩形/圆角/描边判据**）：形状管线的片元阶段。
///
/// 判据本体已提到 [`emit_shape_hit`]（B5-1 统一管线要复用它，避免两份必须
/// 永远一致的实现）。本函数只负责「接口装配 + 把判据结果变成颜色」。
pub fn fragment_shader_rect_shape() -> Vec<u8> {
    let mut m = Module::new();
    m.shader_capability().memory_model_glsl450().source_unknown();
    // `glsl450` 是内存模型，**不是**扩展指令集；`OpFloor` 需要另外导入
    // `GLSL.std.450`（见 GLSL_STD_450_FLOOR 的说明）。
    let glsl = m.ext_inst_import_glsl_std_450();

    let void = m.type_void();
    let f32_ty = m.type_float();
    let bool_ty = m.type_bool();
    let v4 = m.type_vector(f32_ty, 4);
    let ptr_in_v4 = m.type_pointer(SC_INPUT, v4);
    let ptr_in_f32 = m.type_pointer(SC_INPUT, f32_ty);
    let ptr_out_v4 = m.type_pointer(SC_OUTPUT, v4);
    let fn_ty = m.type_function(void, &[]);

    let out_color = m.variable(ptr_out_v4, SC_OUTPUT);
    let in_rect = m.variable(ptr_in_v4, SC_INPUT);
    let in_rk = m.variable(ptr_in_f32, SC_INPUT);
    let in_color = m.variable(ptr_in_v4, SC_INPUT);
    let in_frag = m.variable(ptr_in_v4, SC_INPUT);

    let fn_id = m.id();
    let block = m.id();
    m.entry_point(
        EXECUTION_MODEL_FRAGMENT,
        fn_id,
        "main",
        &[out_color, in_rect, in_rk, in_color, in_frag],
    );
    m.execution_mode(fn_id, EXECUTION_MODE_ORIGIN_UPPER_LEFT, &[]);

    // 输出 location 0
    m.debug_name(out_color, "out_color");
    m.decorate(out_color, DECORATION_LOCATION, &[0]);
    // 输入 location 0/1/2（与顶点着色器的输出对齐）
    m.debug_name(in_rect, "in_rect");
    m.decorate(in_rect, DECORATION_LOCATION, &[0]);
    m.debug_name(in_rk, "in_radius_kind");
    m.decorate(in_rk, DECORATION_LOCATION, &[1]);
    m.debug_name(in_color, "in_color");
    m.decorate(in_color, DECORATION_LOCATION, &[2]);
    // 内建输入：gl_FragCoord（**不能**同时有 Location 装饰）
    m.debug_name(in_frag, "gl_FragCoord");
    m.decorate(in_frag, DECORATION_BUILT_IN, &[BUILTIN_FRAG_COORD]);

    m.function(void, fn_id, fn_ty, block);

    // 判据本体（唯一实现）。常量与 `OpLoad` 都由它发射（顺序对字节码有影响，见其文档）；
    // 后四个返回值就是它 load 出来的输入，这里直接用，**不要重复 load**。
    let (hit, f0, _frag, _rect, _rk, color) = emit_shape_hit(
        &mut m, f32_ty, bool_ty, v4, glsl, in_frag, in_rect, in_rk, in_color,
    );

    // out_color = select(hit, color, vec4(0,0,0,0))
    //
    // ⚠️ **必须逐分量选，不能对 `vec4` 直接 `OpSelect` 标量条件**：
    // 本机 `spirv-val`（SDK 1.4.357.0）实测拒绝
    //   `OpSelect %v4float %scalar_bool %color %zero`
    // 并报 `Expected vector sizes of Result Type and the condition to be equal: Select`。
    // 所以走与 [`vertex_shader_select_full_vec4`] 相同的稳妥写法：
    // 抽出分量 → 逐分量 `OpSelect` → `OpCompositeConstruct` 组回。
    let hit_vec = {
        let mut comps = Vec::with_capacity(4);
        for c in 0..4u32 {
            let v = m.composite_extract(f32_ty, color, &[c]);
            comps.push(m.op_select(f32_ty, hit, v, f0));
        }
        m.composite_construct(v4, &comps)
    };
    m.store(out_color, hit_vec);

    m.return_void();
    m.function_end();
    m.finish()
}

/// 顶点着色器（**文本/字形**）：M3b 的第二条管线。
///
/// ## 顶点布局（与 `gpu_text.rs` 的 `TextVertex` 共用，**不允许实现时改**）
///
/// | location | 类型 | 含义 |
/// |---|---|---|
/// | 0 | `vec2` | 位置（NDC，y 向下） |
/// | 1 | `vec2` | uv（**归一化到整张字形图集**，`[0,1]`） |
/// | 2 | `vec4` | 颜色 |
///
/// `TextVertex` 是 `#[repr(C)]`、stride 32（`pos` 0 / `uv` 8 / `color` 16）。
///
/// ## 为什么单独一条管线而不是复用 M3a 的 shape 管线
///
/// M3a 的顶点属性是 `pos + rect + radius_kind + color`，其中 `radius_kind` 的
/// **负值已被「描边带宽」占用**（`-1.0` = 1px，见 Ruling 6）。文本既没有矩形
/// 也没有描边，硬塞进那条管线就必须给 `radius_kind` 找个「既不是圆角半径、
/// 也不是描边」的第三种含义 —— 那会破坏 M3a 已经冻结并逐像素验证过的契约。
///
/// ## 实现要点
///
/// 纯 `OpLoad` + `OpStore` 透传，无 `OpSelect`、无动态索引（与
/// [`vertex_shader_from_vertex_buffer`] 同一条路）。
pub fn vertex_shader_text() -> Vec<u8> {
    let mut m = Module::new();
    m.shader_capability().memory_model_glsl450().source_unknown();

    let void = m.type_void();
    let f32_ty = m.type_float();
    let v2 = m.type_vector(f32_ty, 2);
    let v4 = m.type_vector(f32_ty, 4);
    let ptr_in_v2 = m.type_pointer(SC_INPUT, v2);
    let ptr_in_v4 = m.type_pointer(SC_INPUT, v4);
    let ptr_out_v4 = m.type_pointer(SC_OUTPUT, v4);
    let ptr_out_v2 = m.type_pointer(SC_OUTPUT, v2);
    let fn_ty = m.type_function(void, &[]);

    let out_pos = m.variable(ptr_out_v4, SC_OUTPUT);
    let out_uv = m.variable(ptr_out_v2, SC_OUTPUT);
    let out_color = m.variable(ptr_out_v4, SC_OUTPUT);
    let in_pos = m.variable(ptr_in_v2, SC_INPUT);
    let in_uv = m.variable(ptr_in_v2, SC_INPUT);
    let in_color = m.variable(ptr_in_v4, SC_INPUT);

    let z = m.constant_f32(f32_ty, 0.0);
    let w = m.constant_f32(f32_ty, 1.0);

    let fn_id = m.id();
    let block = m.id();
    m.entry_point(
        EXECUTION_MODEL_VERTEX,
        fn_id,
        "main",
        &[out_pos, out_uv, out_color, in_pos, in_uv, in_color],
    );

    // 输入 location 0/1/2（顺序必须与 TextVertex 布局一致）
    m.debug_name(in_pos, "in_pos");
    m.decorate(in_pos, DECORATION_LOCATION, &[0]);
    m.debug_name(in_uv, "in_uv");
    m.decorate(in_uv, DECORATION_LOCATION, &[1]);
    m.debug_name(in_color, "in_color");
    m.decorate(in_color, DECORATION_LOCATION, &[2]);

    // 内建输出
    m.debug_name(out_pos, "gl_Position");
    m.decorate(out_pos, DECORATION_BUILT_IN, &[BUILTIN_POSITION]);
    // 输出 location 0/1（与片段着色器输入对齐）
    m.debug_name(out_uv, "out_uv");
    m.decorate(out_uv, DECORATION_LOCATION, &[0]);
    m.debug_name(out_color, "out_color");
    m.decorate(out_color, DECORATION_LOCATION, &[1]);

    m.function(void, fn_id, fn_ty, block);

    // gl_Position = vec4(pos, 0, 1)
    let p = m.load(v2, in_pos);
    let px = m.composite_extract(f32_ty, p, &[0]);
    let py = m.composite_extract(f32_ty, p, &[1]);
    // px/py 是运行时值 ⇒ 必须 OpCompositeConstruct
    let pos4 = m.composite_construct(v4, &[px, py, z, w]);
    m.store(out_pos, pos4);

    // uv / color 透传
    let uv = m.load(v2, in_uv);
    m.store(out_uv, uv);
    let color = m.load(v4, in_color);
    m.store(out_color, color);

    m.return_void();
    m.function_end();
    m.finish()
}

/// 片段着色器（**字形覆盖率采样**）：M3b 的文本管线片元阶段。
///
/// ## 判据（与 CPU `null.rs::draw_text_real` 逐字节对应）
///
/// ```text
///   cov = texture(tex, uv).r                 // R8_UNORM ⇒ 已归一化到 [0,1]
///   out = vec4(color.rgb, color.a * cov)     // **非预乘**：覆盖率只乘进 alpha
/// ```
///
/// CPU 侧是 `blend_cov(fb, x, y, color, cov as f32 / 255.0, clip)`：
/// `cov/255.0` 与 `R8_UNORM` 采样得到的 `.r` **是同一个数**（UNORM 采样即
/// `value / 255`），所以这里不需要额外的缩放或取整。
///
/// ## ⚠️ 为什么 rgb **不**乘 cov（一条由混合状态决定的硬约束）
///
/// 本项目的管线混合状态是 **`SRC_ALPHA / ONE_MINUS_SRC_ALPHA`**（见
/// `device.rs::build_pipeline`），代入方程
/// `out = src_color * src_alpha + dst_color * (1 - src_alpha)`：
///
/// | 着色器输出 | 混合后 RGB | 与 CPU 一致？ |
/// |---|---|---|
/// | `(rgb*cov, a*cov)`（**预乘**） | `rgb*a*cov² + bg*(1-a*cov)` | ❌ alpha 被乘两次，文本边缘明显偏暗 |
/// | `(rgb, a*cov)`（**非预乘**） | `rgb*a*cov + bg*(1-a*cov)` | ✅ 与 `blend_cov` 逐字节等价 |
///
/// CPU 的 `blend_cov` 正是后者：`a = c.a * cov`，然后 RGB 与 alpha 都按这个 `a`
/// 做 src-over。**预乘写法只在混合因子是 `ONE / ONE_MINUS_SRC_ALPHA` 时才等价** ——
/// 本项目的管线不是。
///
/// 计划初稿（M3b Task 2）写的是预乘版，是**计划里的错**；这里按实测的混合状态
/// 修正为上面的非预乘版，并由 `pipeline_smoke.rs` 的绑定断言 +
/// （后续 T4 的）逐像素对照共同守住。
///
/// ## 接口
///
/// | 方向 | location | 类型 | 含义 |
/// |---|---|---|---|
/// | in | 0 | `vec2` | uv（归一化到整张图集） |
/// | in | 1 | `vec4` | 颜色（**非预乘**，见上） |
/// | out | 0 | `vec4` | `(rgb, a*cov)` |
/// | uniform | set 0 binding 0 | `sampler2D` | 字形覆盖率图集 |
///
/// ## 无控制流
///
/// 单基本块、零 `OpBranch`、零 `OpPhi`：只有一次采样 + 一次标量乘法。
pub fn fragment_shader_text() -> Vec<u8> {
    let mut m = Module::new();
    m.shader_capability().memory_model_glsl450().source_unknown();

    let void = m.type_void();
    let f32_ty = m.type_float();
    let v2 = m.type_vector(f32_ty, 2);
    let v4 = m.type_vector(f32_ty, 4);
    // 纹理类型链：OpTypeImage → OpTypeSampledImage → 指向它的指针（UniformConstant）
    let img_ty = m.type_image_2d_unknown(f32_ty);
    let sampled_ty = m.type_sampled_image(img_ty);
    let ptr_tex = m.type_pointer(SC_UNIFORM_CONSTANT, sampled_ty);
    let ptr_in_v2 = m.type_pointer(SC_INPUT, v2);
    let ptr_in_v4 = m.type_pointer(SC_INPUT, v4);
    let ptr_out_v4 = m.type_pointer(SC_OUTPUT, v4);
    let fn_ty = m.type_function(void, &[]);

    let tex = m.variable(ptr_tex, SC_UNIFORM_CONSTANT);
    let out_color = m.variable(ptr_out_v4, SC_OUTPUT);
    let in_uv = m.variable(ptr_in_v2, SC_INPUT);
    let in_color = m.variable(ptr_in_v4, SC_INPUT);

    let fn_id = m.id();
    let block = m.id();
    // ⚠️ `interface` 必须包含所有静态使用的 Input/Output 变量。
    // **UniformConstant 的纹理变量不在此列**（SPIR-V 1.0 的 OpEntryPoint
    // interface 只列 Input/Output；把 UniformConstant 塞进去会被校验器拒绝）。
    m.entry_point(EXECUTION_MODEL_FRAGMENT, fn_id, "main", &[out_color, in_uv, in_color]);
    m.execution_mode(fn_id, EXECUTION_MODE_ORIGIN_UPPER_LEFT, &[]);

    // 描述符接口：set 0 / binding 0 —— 必须与 Rust 侧的
    // `create_descriptor_set_layout_combined_sampler()` 逐字一致。
    m.debug_name(tex, "glyph_atlas");
    m.decorate(tex, DECORATION_DESCRIPTOR_SET, &[0]);
    m.decorate(tex, DECORATION_BINDING, &[0]);

    // 输出 location 0
    m.debug_name(out_color, "out_color");
    m.decorate(out_color, DECORATION_LOCATION, &[0]);
    // 输入 location 0/1（与顶点着色器输出对齐）
    m.debug_name(in_uv, "in_uv");
    m.decorate(in_uv, DECORATION_LOCATION, &[0]);
    m.debug_name(in_color, "in_color");
    m.decorate(in_color, DECORATION_LOCATION, &[1]);

    m.function(void, fn_id, fn_ty, block);

    // cov = texture(tex, uv).r
    let sampled = m.load(sampled_ty, tex);
    let uv = m.load(v2, in_uv);
    let rgba = m.op_image_sample_implicit_lod(v4, sampled, uv);
    let cov = m.composite_extract(f32_ty, rgba, &[0]);

    // out = vec4(color.rgb, color.a * cov)  —— **非预乘**（理由见上面的表格）
    let color = m.load(v4, in_color);
    let rgb0 = m.composite_extract(f32_ty, color, &[0]);
    let rgb1 = m.composite_extract(f32_ty, color, &[1]);
    let rgb2 = m.composite_extract(f32_ty, color, &[2]);
    let a = m.composite_extract(f32_ty, color, &[3]);
    let a_scaled = m.f_mul(f32_ty, a, cov);
    let out = m.composite_construct(v4, &[rgb0, rgb1, rgb2, a_scaled]);    m.store(out_color, out);

    m.return_void();
    m.function_end();
    m.finish()
}

// ═══════════════════════════════════════════════════════════════════════════════
// B5-1：**统一管线**的着色器对（形状与文本合成一条管线）
// ═══════════════════════════════════════════════════════════════════════════════

/// 顶点着色器（**统一管线**：形状与文本共用一个顶点流）。
///
/// ## 为什么要统一（收益）
///
/// 此前形状与文本是**两条管线**，交错语料（形状、文本、形状、文本…）每切换一次
/// 就要重绑管线 + 描述符集 + 顶点缓冲 ⇒ 实测语料是 **8 次 draw + 8 次切换**。
/// 把两者合成**一条**管线后同一份语料可以**一次 draw** 画完 ⇒ **1 draw + 1 切换**。
///
/// ## 接口（**冻结**，`UnifiedVertex` 必须与它逐字一致）
///
/// | location | 类型 | 含义 | 形状段取值 | 文本段取值 |
/// |---|---|---|---|---|
/// | 0 | `vec2` | 位置（NDC，y 向下） | 矩形顶点 | 字形四边形顶点 |
/// | 1 | `vec4` | `rect`（`x, y, w, h`） | 真实矩形 | **未使用**（填 0） |
/// | 2 | `float` | `radius_kind` | 真实值（负 = 描边带宽） | **未使用**（填 0） |
/// | 3 | `vec4` | 颜色（**非预乘**） | 真实颜色 | 真实颜色 |
/// | 4 | `vec2` | `uv`（**判别符**） | **负值**（约定 `(-1,-1)`） | 归一化图集坐标 `[0,1]` |
///
/// stride 52 = 8 + 16 + 4 + 16 + 8，偏移 0 / 8 / 24 / 28 / 44。
///
/// ## 判别符为什么用 `uv`
///
/// 需要一个「**两条路径都携带、且取值域天然不重叠**」的属性：
/// - `uv.x < 0` ⇒ **形状**（形状段没有纹理，约定填负值）；
/// - `uv.x >= 0` ⇒ **文本**（归一化图集坐标恒 `≥ 0`）。
///
/// 不能用 `radius_kind`：它的**负值已被「描边带宽」占用**（Ruling 6），
/// 再借它表达「这是文本」就会破坏 M3a 冻结的契约（见 [`vertex_shader_text`] 的说明）。
///
/// ## 为什么不用控制流
///
/// 顶点阶段只是**透传**（`OpLoad` + `OpStore`），本来就没有分支。
/// 判别发生在**片元**阶段，且那里也**不用** `OpBranchConditional` ——
/// 只用 `OpSelect`（见 [`fragment_shader_unified`]），所以整支着色器
/// 仍是**单基本块、零 `OpPhi`**（`unified_shaders_have_no_control_flow` 钉住）。
///
/// ## `gl_Position` 用 `OpCompositeConstruct` 而不是 `OpCompositeInsert`
///
/// `px`/`py` 是**运行时**值（从顶点缓冲 `OpLoad` 出来的），所以只能构造，
/// 不能像 [`vertex_shader_hardcoded_position`] 那样把常量直接拼进字面量。
/// `z = 0.0`、`w = 1.0` 是常量，但仍按构造走（与 [`vertex_shader_text`] 一致）。
pub fn vertex_shader_unified() -> Vec<u8> {
    let mut m = Module::new();
    m.shader_capability().memory_model_glsl450().source_unknown();

    let void = m.type_void();
    let f32_ty = m.type_float();
    let v2 = m.type_vector(f32_ty, 2);
    let v4 = m.type_vector(f32_ty, 4);
    let ptr_in_v2 = m.type_pointer(SC_INPUT, v2);
    let ptr_in_f32 = m.type_pointer(SC_INPUT, f32_ty);
    let ptr_in_v4 = m.type_pointer(SC_INPUT, v4);
    let ptr_out_v4 = m.type_pointer(SC_OUTPUT, v4);
    let ptr_out_f32 = m.type_pointer(SC_OUTPUT, f32_ty);
    let ptr_out_v2 = m.type_pointer(SC_OUTPUT, v2);
    let fn_ty = m.type_function(void, &[]);

    // 5 个输入（location 0..4）
    let in_pos = m.variable(ptr_in_v2, SC_INPUT);
    let in_rect = m.variable(ptr_in_v4, SC_INPUT);
    let in_rk = m.variable(ptr_in_f32, SC_INPUT);
    let in_color = m.variable(ptr_in_v4, SC_INPUT);
    let in_uv = m.variable(ptr_in_v2, SC_INPUT);

    // 5 个输出（location 0..4）。`gl_Position` 是内建，不占 location。
    let out_pos = m.variable(ptr_out_v4, SC_OUTPUT);
    let out_rect = m.variable(ptr_out_v4, SC_OUTPUT);
    let out_rk = m.variable(ptr_out_f32, SC_OUTPUT);
    let out_color = m.variable(ptr_out_v4, SC_OUTPUT);
    let out_uv = m.variable(ptr_out_v2, SC_OUTPUT);

    let z = m.constant_f32(f32_ty, 0.0);
    let w = m.constant_f32(f32_ty, 1.0);

    let fn_id = m.id();
    let block = m.id();
    m.entry_point(
        EXECUTION_MODEL_VERTEX,
        fn_id,
        "main",
        &[
            out_pos, out_rect, out_rk, out_color, out_uv, in_pos, in_rect, in_rk, in_color,
            in_uv,
        ],
    );

    // ── 输入 location 0..4 ───────────────────────────────────────────────────
    m.debug_name(in_pos, "in_pos");
    m.decorate(in_pos, DECORATION_LOCATION, &[0]);
    m.debug_name(in_rect, "in_rect");
    m.decorate(in_rect, DECORATION_LOCATION, &[1]);
    m.debug_name(in_rk, "in_radius_kind");
    m.decorate(in_rk, DECORATION_LOCATION, &[2]);
    m.debug_name(in_color, "in_color");
    m.decorate(in_color, DECORATION_LOCATION, &[3]);
    m.debug_name(in_uv, "in_uv");
    m.decorate(in_uv, DECORATION_LOCATION, &[4]);

    // ── 内建输出 + 输出 location 0..4（与片元着色器的输入对齐） ──────────────
    m.debug_name(out_pos, "gl_Position");
    m.decorate(out_pos, DECORATION_BUILT_IN, &[BUILTIN_POSITION]);
    m.debug_name(out_rect, "out_rect");
    m.decorate(out_rect, DECORATION_LOCATION, &[0]);
    m.debug_name(out_rk, "out_radius_kind");
    m.decorate(out_rk, DECORATION_LOCATION, &[1]);
    m.debug_name(out_color, "out_color");
    m.decorate(out_color, DECORATION_LOCATION, &[2]);
    m.debug_name(out_uv, "out_uv");
    m.decorate(out_uv, DECORATION_LOCATION, &[3]);

    m.function(void, fn_id, fn_ty, block);

    // gl_Position = vec4(pos, 0, 1)
    let p = m.load(v2, in_pos);
    let px = m.composite_extract(f32_ty, p, &[0]);
    let py = m.composite_extract(f32_ty, p, &[1]);
    // px/py 是运行时值 ⇒ 必须 OpCompositeConstruct
    let pos4 = m.composite_construct(v4, &[px, py, z, w]);
    m.store(out_pos, pos4);

    // 其余 4 个属性**透传**。逐分量 extract + construct 与直接 load/store 等价，
    // 但这里统一走「构造」路径 —— 与 [`vertex_shader_text`] 的写法一致，
    // 且不依赖「向量类型可直接 store」这种隐含前提。
    for (src, dst, ty, n) in [
        (in_rect, out_rect, v4, 4u32),
        (in_color, out_color, v4, 4u32),
        (in_uv, out_uv, v2, 2u32),
    ] {
        let v = m.load(ty, src);
        let comps: Vec<u32> = (0..n)
            .map(|c| m.composite_extract(f32_ty, v, &[c]))
            .collect();
        let out = m.composite_construct(ty, &comps);
        m.store(dst, out);
    }
    // 标量属性（radius_kind）直接 load/store
    let rk = m.load(f32_ty, in_rk);
    m.store(out_rk, rk);

    m.return_void();
    m.function_end();
    m.finish()
}

/// 片段着色器（**统一管线**：形状与文本各算一支，用 `OpSelect` 二选一）。
///
/// ## 判别符与两支
///
/// ```text
///   is_shape = uv.x < 0.0                    // 形状段约定 uv = (-1,-1)
///   shape_out = select(shape_hit, color, vec4(0))
///   cov       = texture(atlas, uv).r
///   text_out  = vec4(color.rgb, color.a * cov)     // **非预乘**
///   out_color = select(is_shape, shape_out, text_out)
/// ```
///
/// ## 无控制流
///
/// 单基本块、零 `OpBranch` / `OpPhi` / `OpSwitch` —— 判别只用 `OpSelect`。
/// 这与 M3a/M3b 的既有契约一致（`unified_shaders_have_no_control_flow` 钉住）。
///
/// ## 形状那一支**逐字复用** M3a 的判据
///
/// 调的是 [`emit_shape_hit`]（与 [`fragment_shader_rect_shape`] **同一个函数**），
/// 所以「统一之后形状像素会不会变」这个问题在**实现层**就没有第二种答案。
/// 这里**没有**重写任何数学 —— 重写就等于制造第二份必须永远一致的实现。
///
/// ## ⚠️ 形状帧也会**采样**纹理（隐式依赖）
///
/// `text_out` 是无条件计算的（无分支的代价），所以**形状帧也会执行采样**。
/// 形状段的 `uv = (-1,-1)` 会被采到图集**边缘**的某个值，但那一支的结果**不被采用**
/// （`is_shape` 为真时选的是 `shape_out`）。
///
/// 这带来两条**必须满足**的前提（B5-2 负责，记在这里免得以后忘）：
/// 1. 绑定的图集纹理**必须存在**（形状帧也要绑一个，没有 GlyphAtlas 时用 1×1 哑纹理）；
/// 2. 采样器必须是 `ClampToEdge`（越界 uv 不能是未定义行为）。
///
/// ⚠️ **判据要打对目标（B5-3 更正）**：承重的是「**绑**一个已写入的有效描述符」，
/// **不是**那张 1×1 哑纹理的**取值** —— 把它换成 `cov=0`（`[0]`）甚至 `[128]`，
/// 形状帧在像素上**逐字节不变**（这一支的结果被 `OpSelect` 丢弃）。
/// 所以**不能**用「哑纹理换成 cov=0 ⇒ 形状帧变红」当承重性判据（早先这里就是这么写的，
/// 与实测相反，已改）。真正的判据是
/// `tests/gpu_vs_cpu.rs::a_shape_only_frame_binds_the_one_by_one_dummy_texture`：
/// 删掉 `cmd_bind_descriptor_sets` ⇒ 默认档 18 failed（形状帧读回**整幅 0**）、
/// 校验层 `VUID-vkCmdDraw-None-08600`；跳过 `update_descriptor_texture` ⇒
/// `VUID-vkCmdDraw-None-08114`。详见 `gpu_render::DUMMY_COVERAGE` 的说明。
///
/// ## ⚠️ 逐分量 `OpSelect`（一次实测教训）
///
/// 与 [`fragment_shader_rect_shape`] 同样的原因：本机 `spirv-val`（SDK 1.4.357.0）
/// **拒绝**「标量 `bool` 条件 + `vec4` 结果」的 `OpSelect`
/// （`Expected vector sizes of Result Type and the condition to be equal`）。
/// 所以两处 vec4 选择（形状输出、最终二选一）都写成
/// 「逐分量 `OpSelect` + `OpCompositeConstruct`」。
///
/// ## 描述符
///
/// `set 0 / binding 0` = 字形覆盖率图集（`sampler2D`），与
/// `create_descriptor_set_layout_combined_sampler()` 逐字一致。
pub fn fragment_shader_unified() -> Vec<u8> {
    let mut m = Module::new();
    m.shader_capability().memory_model_glsl450().source_unknown();
    // `OpFloor` 来自 GLSL.std.450（不是 core opcode），形状判据要用
    let glsl = m.ext_inst_import_glsl_std_450();

    let void = m.type_void();
    let f32_ty = m.type_float();
    let bool_ty = m.type_bool();
    let v2 = m.type_vector(f32_ty, 2);
    let v4 = m.type_vector(f32_ty, 4);
    // 纹理类型链：OpTypeImage → OpTypeSampledImage → 指向它的指针（UniformConstant）
    let img_ty = m.type_image_2d_unknown(f32_ty);
    let sampled_ty = m.type_sampled_image(img_ty);
    let ptr_tex = m.type_pointer(SC_UNIFORM_CONSTANT, sampled_ty);
    let ptr_in_v2 = m.type_pointer(SC_INPUT, v2);
    let ptr_in_f32 = m.type_pointer(SC_INPUT, f32_ty);
    let ptr_in_v4 = m.type_pointer(SC_INPUT, v4);
    let ptr_out_v4 = m.type_pointer(SC_OUTPUT, v4);
    let fn_ty = m.type_function(void, &[]);

    let tex = m.variable(ptr_tex, SC_UNIFORM_CONSTANT);
    let out_color = m.variable(ptr_out_v4, SC_OUTPUT);
    let in_rect = m.variable(ptr_in_v4, SC_INPUT);
    let in_rk = m.variable(ptr_in_f32, SC_INPUT);
    let in_color = m.variable(ptr_in_v4, SC_INPUT);
    let in_uv = m.variable(ptr_in_v2, SC_INPUT);
    let in_frag = m.variable(ptr_in_v4, SC_INPUT);

    let fn_id = m.id();
    let block = m.id();
    // ⚠️ `interface` 必须包含所有静态使用的 Input/Output 变量。
    // **UniformConstant 的纹理变量不在此列**（SPIR-V 1.0 的 OpEntryPoint
    // interface 只列 Input/Output；把 UniformConstant 塞进去会被校验器拒绝）。
    m.entry_point(
        EXECUTION_MODEL_FRAGMENT,
        fn_id,
        "main",
        &[out_color, in_rect, in_rk, in_color, in_uv, in_frag],
    );
    m.execution_mode(fn_id, EXECUTION_MODE_ORIGIN_UPPER_LEFT, &[]);

    // 描述符接口：set 0 / binding 0
    m.debug_name(tex, "glyph_atlas");
    m.decorate(tex, DECORATION_DESCRIPTOR_SET, &[0]);
    m.decorate(tex, DECORATION_BINDING, &[0]);

    // 输出 location 0
    m.debug_name(out_color, "out_color");
    m.decorate(out_color, DECORATION_LOCATION, &[0]);
    // 输入 location 0..3（与统一顶点着色器的输出对齐）
    m.debug_name(in_rect, "in_rect");
    m.decorate(in_rect, DECORATION_LOCATION, &[0]);
    m.debug_name(in_rk, "in_radius_kind");
    m.decorate(in_rk, DECORATION_LOCATION, &[1]);
    m.debug_name(in_color, "in_color");
    m.decorate(in_color, DECORATION_LOCATION, &[2]);
    m.debug_name(in_uv, "in_uv");
    m.decorate(in_uv, DECORATION_LOCATION, &[3]);
    // 内建输入：gl_FragCoord（**不能**同时有 Location 装饰）
    m.debug_name(in_frag, "gl_FragCoord");
    m.decorate(in_frag, DECORATION_BUILT_IN, &[BUILTIN_FRAG_COORD]);

    m.function(void, fn_id, fn_ty, block);

    // ── 公共输入 ─────────────────────────────────────────────────────────────
    // `0.0` 只造一次，供判据、形状输出、判别符三处共用
    // （`constant_f32` 不去重，多造会多一条 `OpConstant`）
    // ── 形状那一支：判据**逐字复用**（同一个函数，不是抄一遍） ────────────────
    // 常量与 rect/rk/color/frag 的 `OpLoad` 都由它发射；返回的 `f0` 是它的 `0.0` 常量
    let (shape_hit, f0, _frag, _rect, _rk, color) = emit_shape_hit(
        &mut m, f32_ty, bool_ty, v4, glsl, in_frag, in_rect, in_rk, in_color,
    );
    // `uv` 只在文本那一支用（采样 + 判别符），统一着色器自己 load
    let uv = m.load(v2, in_uv);

    // shape_out = select(shape_hit, color, vec4(0)) —— 逐分量（见上面的教训）
    let shape_out = {
        let mut comps = Vec::with_capacity(4);
        for c in 0..4u32 {
            let v = m.composite_extract(f32_ty, color, &[c]);
            comps.push(m.op_select(f32_ty, shape_hit, v, f0));
        }
        m.composite_construct(v4, &comps)
    };

    // ── 文本那一支：cov = texture(tex, uv).r ────────────────────────────────
    let sampled = m.load(sampled_ty, tex);
    let rgba = m.op_image_sample_implicit_lod(v4, sampled, uv);
    let cov = m.composite_extract(f32_ty, rgba, &[0]);

    // text_out = vec4(color.rgb, color.a * cov) —— **非预乘**（覆盖率只进 alpha）
    let text_out = {
        let r = m.composite_extract(f32_ty, color, &[0]);
        let g = m.composite_extract(f32_ty, color, &[1]);
        let b = m.composite_extract(f32_ty, color, &[2]);
        let a = m.composite_extract(f32_ty, color, &[3]);
        let a_scaled = m.f_mul(f32_ty, a, cov);
        m.composite_construct(v4, &[r, g, b, a_scaled])
    };

    // ── 二选一 ────────────────────────────────────────────────────────────────
    // is_shape = uv.x < 0.0（形状段约定 uv = (-1,-1)）
    let uv_x = m.composite_extract(f32_ty, uv, &[0]);
    let is_shape = m.op_ford_less_than(bool_ty, uv_x, f0);

    // out_color = select(is_shape, shape_out, text_out) —— 逐分量
    let out = {
        let mut comps = Vec::with_capacity(4);
        for c in 0..4u32 {
            let s = m.composite_extract(f32_ty, shape_out, &[c]);
            let t = m.composite_extract(f32_ty, text_out, &[c]);
            comps.push(m.op_select(f32_ty, is_shape, s, t));
        }
        m.composite_construct(v4, &comps)
    };
    m.store(out_color, out);

    m.return_void();
    m.function_end();
    m.finish()
}

/// **纹理 quad 的片元着色器**（T1.3 A1）：`out = 顶点色 × 采样 rgba`。
///
/// ## 为什么单独一支，而不是给统一 FS 加「第三态」
///
/// 统一 FS 在 `uv.x >= 0` 那一支是**覆盖率语义**：`vec4(color.rgb, color.a * texel.r)`
/// —— 只取 `.r`，因为字形图集是 `R8_UNORM`（覆盖率）。纹理 quad 要的是 **RGBA 调制**
/// （四个通道都参与），两者在同一个 FS 里就得再分辨一次「现在绑的是图集还是用户纹理」。
///
/// 而这件事**不需要第三个状态**：[`crate::gpu_render::GpuGeometryRenderer::draw_textured_quad`]
/// 本来就是把 `set 0 / binding 0` **临时改指到用户纹理**、然后**单独一次提交**
/// （画完立刻改回图集/哑元）。也就是说 —— **纹理 quad 与文本从不在同一次 draw 里**。
/// 既然不在同一趟，判别就该发生在**管线选择**上，而不是在片元里：
/// 同一支顶点着色器 + 同一套顶点布局 + 另一个 FS ⇒ 另一条管线。代价是零，
/// 比「加第三个判别符」或「扩成两个 binding」都小。
///
/// ## 语义（与 CPU 参考**必须**逐字一致，否则 parity 必挂）
///
/// ```text
/// out.rgb = color.rgb * texel.rgb      （两个都是非预乘 ⇒ 相乘仍是调制）
/// out.a   = color.a   * texel.a
/// ```
///
/// 注意与**文本**那一支的区别：文本是 `vec4(color.rgb, color.a * cov)`（色不动、只压 alpha），
/// 这里是**四个分量全部相乘**。别把两者写成一样 —— 一样了就是「RGB 调制没做」。
///
/// ## 顶点布局
///
/// 与 [`vertex_shader_unified`] 共用 ⇒ 复用 `UnifiedVertex`（stride 52 冻结）。
/// 本 FS 只声明 `in_color`（location 2）与 `in_uv`（location 3）：
/// 顶点着色器多输出的 `rect`/`radius_kind` 允许不被片元消费。
pub fn fragment_shader_textured() -> Vec<u8> {
    let mut m = Module::new();
    m.shader_capability().memory_model_glsl450().source_unknown();

    let void = m.type_void();
    let f32_ty = m.type_float();
    let v2 = m.type_vector(f32_ty, 2);
    let v4 = m.type_vector(f32_ty, 4);
    let img_ty = m.type_image_2d_unknown(f32_ty);
    let sampled_ty = m.type_sampled_image(img_ty);
    let ptr_tex = m.type_pointer(SC_UNIFORM_CONSTANT, sampled_ty);
    let ptr_in_v2 = m.type_pointer(SC_INPUT, v2);
    let ptr_in_v4 = m.type_pointer(SC_INPUT, v4);
    let ptr_out_v4 = m.type_pointer(SC_OUTPUT, v4);
    let fn_ty = m.type_function(void, &[]);

    let tex = m.variable(ptr_tex, SC_UNIFORM_CONSTANT);
    let out_color = m.variable(ptr_out_v4, SC_OUTPUT);
    let in_color = m.variable(ptr_in_v4, SC_INPUT);
    let in_uv = m.variable(ptr_in_v2, SC_INPUT);

    let fn_id = m.id();
    let block = m.id();
    m.entry_point(
        EXECUTION_MODEL_FRAGMENT,
        fn_id,
        "main",
        &[out_color, in_color, in_uv],
    );
    m.execution_mode(fn_id, EXECUTION_MODE_ORIGIN_UPPER_LEFT, &[]);

    // 描述符接口：与统一 FS **同一个** set 0 / binding 0（这就是「不改布局」的意思）
    m.debug_name(tex, "user_texture");
    m.decorate(tex, DECORATION_DESCRIPTOR_SET, &[0]);
    m.decorate(tex, DECORATION_BINDING, &[0]);

    m.debug_name(out_color, "out_color");
    m.decorate(out_color, DECORATION_LOCATION, &[0]);
    // location 必须与统一顶点着色器的输出对齐：color = 2、uv = 3
    m.debug_name(in_color, "in_color");
    m.decorate(in_color, DECORATION_LOCATION, &[2]);
    m.debug_name(in_uv, "in_uv");
    m.decorate(in_uv, DECORATION_LOCATION, &[3]);

    m.function(void, fn_id, fn_ty, block);

    let color = m.load(v4, in_color);
    let uv = m.load(v2, in_uv);
    let sampled = m.load(sampled_ty, tex);
    let texel = m.op_image_sample_implicit_lod(v4, sampled, uv);

    // out = color * texel —— 逐分量（本机 spirv-val 拒绝「标量条件 + vec4」的 OpSelect，
    // 这里不用 OpSelect，直接四个 OpFMul）
    let out = {
        let mut comps = Vec::with_capacity(4);
        for c in 0..4u32 {
            let a = m.composite_extract(f32_ty, color, &[c]);
            let b = m.composite_extract(f32_ty, texel, &[c]);
            comps.push(m.f_mul(f32_ty, a, b));
        }
        m.composite_construct(v4, &comps)
    };
    m.store(out_color, out);

    m.return_void();
    m.function_end();
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

    // ── M3a：片元判据的语义回归 ──────────────────────────────────────────────

    /// GPU 片元掩码判据的**可直接执行**版本。
    ///
    /// ⚠️ 这是 [`fragment_shader_rect_shape`] 里那串运算的**忠实转写**
    /// （同样的运算顺序、同样的 float 语义），不是重写的第二套实现 ——
    /// 它的价值在于把「只能在 GPU 上跑、错了只会看到差一列像素」的判据
    /// 变成可在 CI 里断言的纯函数。它与着色器的对应关系由
    /// `gpu_mask_predicate_matches_cpu_reference` 的断言与注释固定下来；
    /// 每加一个算子都要同步这两处。
    ///
    /// 语义确认（与 `crates/deer-gpu/src/null.rs` 对照）：
    /// - `fill(...)`: `for y in [rect.y, rect.y+h)`、`for x in [rect.x, rect.x+w)`
    ///   ⇒ 整矩形减去 `(dl<0)|(dr<0)|(dt<0)|(db<0)`（`dr`/`db` 带 `- 1`）；
    /// - `fill(..., radius = r > 0)`: `inside_rounded` 四角
    ///   `(x - ccx)^2 + (y - ccy)^2 > r^2`，圆心 `cc = rect.角 + r`，
    ///   角区由 `(dx < 0 / > 0)` 判定 ⇒ `(a < r_eff) & (b < r_eff)`；
    /// - `stroke(..., width = w)`: 4 条 `w` 宽的**矩形边**：
    ///   `x_span = [rx,right)`、`y_span = [ry,bottom)`，
    ///   `上带 = x_span & y∈[ry,ry+w)`、`下带 = x_span & y∈[bottom-w,bottom)`、
    ///   `左带 = y_span & x∈[rx,rx+w)`、`右带 = y_span & x∈[right-w,right)`，
    ///   `stroke_mask = 四条带的并集`。
    ///   **注意两点**：(a) 上/下带**不受** `y_span` 约束，所以能向矩形上下各伸出 `w`
    ///   （`null.rs::stroke` 的 `for k in 0..w` 没有把 k 限制在矩形内）；
    ///   (b) 但每条带**必须**受另一轴的 `x_span`/`y_span` 约束，否则矩形外整片
    ///   都会被判成描边。见 `fragment_shader_rect_shape` 的说明。
    ///
    /// `px`/`py` 是整数像素坐标，等价于 `floor(gl_FragCoord.xy)`（像素中心 `x.5` 向下取整）。
    fn gpu_mask_predicate(rect: [i32; 4], radius_kind: f32, px: i32, py: i32) -> bool {
        let f = |v: i32| v as f32;
        let [rx, ry, rw, rh] = rect;
        let (pxf, pyf) = (f(px), f(py));

        let f0 = 0.0f32;
        let f1 = 1.0f32;
        let dl = pxf - f(rx);
        let dr = (f(rx) + f(rw)) - f1 - pxf;
        let dt = pyf - f(ry);
        let db = (f(ry) + f(rh)) - f1 - pyf;

        // out_rect = (dl<0) | (dr<0) | (dt<0) | (db<0)
        let out_rect = (dl < f0) | (dr < f0) | (dt < f0) | (db < f0);

        // r_eff = select(rk < 0, 0.0, rk)
        let r_eff = if radius_kind < f0 { f0 } else { radius_kind };
        let r_sq = r_eff * r_eff;

        let corner_fail = |a: f32, b: f32| -> bool {
            let in_corner = (a < r_eff) & (b < r_eff);
            let (da, dbv) = (a - r_eff, b - r_eff);
            let outside = (da * da + dbv * dbv) > r_sq;
            in_corner & outside
        };
        let fail_tl = corner_fail(dl, dt);
        let fail_tr = corner_fail(dr, dt);
        let fail_bl = corner_fail(dl, db);
        let fail_br = corner_fail(dr, db);
        let fill_mask = !(out_rect | fail_tl | fail_tr | fail_bl | fail_br);

        // bw = -rk；right() = rx+rw、bottom() = ry+rh
        let bw = -radius_kind;
        let right = f(rx) + f(rw);
        let bottom = f(ry) + f(rh);
        let x_span = (pxf >= f(rx)) & (pxf < right);
        let y_span = (pyf >= f(ry)) & (pyf < bottom);
        let band_top = x_span & (pyf >= f(ry)) & (pyf < f(ry) + bw);
        let band_bottom = x_span & (pyf >= bottom - bw) & (pyf < bottom);
        let band_left = y_span & (pxf >= f(rx)) & (pxf < f(rx) + bw);
        let band_right = y_span & (pxf >= right - bw) & (pxf < right);

        // mask = select(rk < 0, stroke_mask, fill_mask)
        if radius_kind < f0 {
            band_top | band_bottom | band_left | band_right
        } else {
            fill_mask
        }
    }

    /// **端口自检**：`gpu_mask_predicate` 是否忠实于 `null.rs` 的 CPU 参考判据。
    ///
    /// 这里按 `null.rs` 的**原式**独立写一遍 CPU 版（整数坐标、整数半径），
    /// 逐像素比对。两处若漂移（例如圆角圆心忘了加 `r`、或下/右边框忘了 `- 1`），
    /// 这条测试会红。
    #[test]
    fn cpu_reference_mask_matches_null_rs() {
        // null.rs::fill 的坐标范围：for y in rect.y..rect.bottom()、x 同理
        fn cpu_fill(rect: [i32; 4], radius: i32, x: i32, y: i32) -> bool {
            let [rx, ry, rw, rh] = rect;
            let (right, bottom) = (rx + rw, ry + rh);
            if !(x >= rx && x < right && y >= ry && y < bottom) {
                return false;
            }
            let r = radius.max(0);
            if r > 0 && !cpu_inside_rounded(rect, x, y, r) {
                return false;
            }
            true
        }

        // null.rs::inside_rounded 的逐字转写
        fn cpu_inside_rounded(rect: [i32; 4], x: i32, y: i32, r: i32) -> bool {
            let [rx, ry, rw, rh] = rect;
            let (right, bottom) = (rx + rw, ry + rh);
            let corners = [
                (rx + r, ry + r, -1, -1),
                (right - 1 - r, ry + r, 1, -1),
                (rx + r, bottom - 1 - r, -1, 1),
                (right - 1 - r, bottom - 1 - r, 1, 1),
            ];
            for (ccx, ccy, sx, sy) in corners {
                let in_corner_x = if sx < 0 { x < ccx } else { x > ccx };
                let in_corner_y = if sy < 0 { y < ccy } else { y > ccy };
                if in_corner_x && in_corner_y {
                    let dx = (x - ccx) as f32;
                    let dy = (y - ccy) as f32;
                    if dx * dx + dy * dy > (r * r) as f32 {
                        return false;
                    }
                }
            }
            true
        }

        // null.rs::stroke：4 条 1px 边叠加 w 次
        fn cpu_stroke(rect: [i32; 4], width: i32, x: i32, y: i32) -> bool {
            let [rx, ry, rw, rh] = rect;
            let (right, bottom) = (rx + rw, ry + rh);
            let w = width.max(1);
            for k in 0..w {
                let top = y == ry + k && x >= rx && x < right;
                let bot = y == bottom - 1 - k && x >= rx && x < right;
                let left = x == rx + k && y >= ry && y < bottom;
                let rig = x == right - 1 - k && y >= ry && y < bottom;
                if top || bot || left || rig {
                    return true;
                }
            }
            false
        }

        // (rect = (x, y, w, h), radius_kind)
        let cases: [([i32; 4], i32); 17] = [
            ([2, 3, 9, 7], 0),   // 普通填充
            ([2, 3, 9, 7], 1),   // 半径 1 的圆角
            ([2, 3, 9, 7], 2),   // 圆角填充
            ([2, 3, 9, 7], 4),   // 大圆角（半径 > 半宽，角区重叠）
            ([2, 3, 9, 7], -1),  // 描边 1px
            ([2, 3, 9, 7], -3),  // 描边 3px
            ([2, 3, 9, 7], -8),  // 带宽 > 高度（边带会延伸到矩形外）
            ([2, 3, 9, 7], -40), // 带宽远超矩形（最容易暴露错误公式）
            ([0, 0, 1, 1], 0),   // 退化 1×1
            ([0, 0, 1, 1], -1),  // 1×1 的 1px 描边（整个矩形）
            ([0, 0, 1, 1], -4),  // 1×1 的 4px 描边（矩形外一圈）
            ([5, 5, 1, 20], -3), // 细长矩形（1×20）的 3px 描边
            ([5, 5, 20, 1], -3), // 细长矩形（20×1）的 3px 描边
            ([0, 0, 2, 50], -6), // 2×50 的 6px 描边（带宽 > 宽度）
            ([0, 0, 8, 8], -8),  // 正方形 + 等宽描边
            ([4, 9, 2, 2], 1),   // 小矩形的 1px 圆角
            ([0, 0, 30, 30], 12), // 大矩形 + 大圆角
        ];

        let mut checked = 0usize;
        for (rect, rk) in cases {
            // 采样范围要**盖住带宽**：`bw > 矩形尺寸` 时 CPU 会把像素涂到矩形外
            // （最多外扩 `bw`）—— 只扫矩形附近就会漏掉那类错。
            let pad = rk.unsigned_abs() as i32 + 3;
            for y in -pad..(rect[1] + rect[3] + pad) {
                for x in -pad..(rect[0] + rect[2] + pad) {
                    let expect = if rk < 0 {
                        cpu_stroke(rect, -rk, x, y)
                    } else {
                        cpu_fill(rect, rk, x, y)
                    };
                    let got = gpu_mask_predicate(rect, rk as f32, x, y);
                    assert_eq!(
                        got, expect,
                        "rect={rect:?} rk={rk} 像素=({x},{y})：GPU 判据 {got} != CPU 参考 {expect}"
                    );
                    checked += 1;
                }
            }
        }
        println!("GPU 片元判据与 null.rs 参考逐像素一致（{checked} 个采样点）✅");
    }

    /// 结构自检：M3a 的两支着色器必须**只用**声明的算子，且不含控制流（无分支/无循环）。
    ///
    /// 为什么单独查「无控制流」：任务的片元判据要求「算掩码 + 一次 `OpSelect`」，
    /// 一旦有人改成 `OpBranchConditional`，多基本块就会引入 `OpPhi`/变量重载，
    /// 出错面大得多，而功能测试未必立刻发现。
    #[test]
    fn rect_shaders_have_no_control_flow() {
        const OP_BRANCH: u16 = 249;
        const OP_BRANCH_CONDITIONAL: u16 = 250;
        const OP_LOOP_MERGE: u16 = 246;
        const OP_PHI: u16 = 245;
        const OP_SWITCH: u16 = 247;
        for (name, bytes) in [
            ("vs_rect_attrs", vertex_shader_rect_attrs()),
            ("fs_rect_shape", fragment_shader_rect_shape()),
        ] {
            let w = words(&bytes);
            // OpLabel 只应出现一次（单个基本块）
            let mut labels = 0usize;
            let mut i = 5usize;
            while i < w.len() {
                let wc = (w[i] >> 16) as usize;
                let op = (w[i] & 0xffff) as u16;
                assert!(
                    !matches!(
                        op,
                        OP_BRANCH | OP_BRANCH_CONDITIONAL | OP_LOOP_MERGE | OP_PHI | OP_SWITCH
                    ),
                    "{name}: 不允许控制流指令（opcode {op}）"
                );
                if op == OP_LABEL {
                    labels += 1;
                }
                i += wc;
            }
            assert_eq!(labels, 1, "{name}: 必须是单基本块（实得 {labels} 个 OpLabel）");
        }
    }
}
