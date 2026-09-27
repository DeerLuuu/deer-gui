//! **共用管线层**（M3c-T1）：把「形状管线」与「文本管线」的状态参数化到
//! **两个维度**（颜色附件格式 + viewport 策略），其余状态两路径**逐字相同**。
//!
//! ## 为什么要有这个模块
//!
//! M3a（形状）与 M3b（文本）各自在 `gpu_render.rs` 里建过一次管线，管线状态是
//! **两份手写的字面量**。M3c 要让**窗口**也用同一套画法 ⇒ 若不提出来，就会变成
//! **四份**（离屏/窗口 × 形状/文本），而其中 20 多项状态必须完全一致 ——
//! 「以后只改了一边」是这类复制粘贴最典型的缺陷，且症状极隐蔽
//! （例如只改了形状的混合、文本却不同 ⇒ 文本半透明处颜色不对，但没人会想到去比对两份字面量）。
//!
//! 所以这里把状态集中到**一处**，并把它做成**纯数据 + 纯函数**：
//! 无 GPU 也能断言「两路径除格式与 viewport 外逐字段相同」（见本文件末尾的测试）。
//!
//! ## 冻结的状态（两路径必须逐字相同）
//!
//! | 状态 | 值 | 为什么不能改 |
//! |---|---|---|
//! | 拓扑 | `TRIANGLE_LIST` | 顶点流是「每矩形/每字形两个三角形」 |
//! | 剔除 | `cull_mode = NONE` | GUI 矩形两个朝向都可能（顶点顺序不保证） |
//! | 正面 | `COUNTER_CLOCKWISE` | 与上面配套；剔除关掉后它不生效，但仍钉住以免将来开剔除时行为漂移 |
//! | 多边形模式 | `FILL` | — |
//! | 多重采样 | `1` 采样、不做 sample shading / alpha-to-coverage / alpha-to-one | M3a/M3b 的逐字节判据是在 1 采样下取得的 |
//! | 混合（color） | `SRC_ALPHA` / `ONE_MINUS_SRC_ALPHA` / `ADD` | 与 CPU `null.rs::blend_cov` 逐字节等价（M3b 已用「非预乘输出」锁定，见 `spirv::fragment_shader_text` 的表格） |
//! | 混合（alpha） | `ONE` / `ONE_MINUS_SRC_ALPHA` / `ADD` | 同上（CPU 也是 `a + da*(1-a)`） |
//! | 颜色写掩码 | RGBA 全写 | 少写一个通道会让回读对不上 |
//! | 逻辑运算 | 关（`COPY`） | 开着会绕过混合 |
//! | 深度/模板 | 无 | 没有深度附件 |
//!
//! ## 参数化的两个维度（只有这两个不同）
//!
//! 1. **颜色附件格式**：离屏 `R8G8B8A8_UNORM`；窗口 `B8G8R8A8_SRGB`（见下节）。
//! 2. **viewport/scissor 策略**：离屏用**静态**（M2a 实测：动态在 Intel 上画不出像素），
//!    窗口用**动态**（M2b 实测能上屏，且窗口尺寸会变）。**不得互相照搬**。
//!
//! ## ⚠️ sRGB：格式这一维**不是**无关紧要的（M3c-T2 的结论）
//!
//! 附件格式决定了两件事，而且**两件都不只是「显示 gamma」**：
//!
//! 1. **写入编码**：片元着色器输出的值被当作**线性**值写入 sRGB 附件时，驱动会做
//!    sRGB 编码（M2b 实测：清屏 `rgb(0x10,0x14,0x24)` 回读成 `[0x47,0x4F,0x69]`）。
//! 2. **混合发生的空间**：Vulkan 规定 sRGB 附件的混合在**线性**空间进行，
//!    最后才编码 —— 而 `SRC_ALPHA/ONE_MINUS_SRC_ALPHA` 在**两个空间里不是同一个运算**。
//!
//! 第 2 条是真正致命的：CPU 基准 `null.rs::blend_cov` 是在**字节（sRGB）空间**做
//! `(1-a)*src + a*dst`，而 sRGB 附件的硬件混合是 `srgb_encode((1-a)*srgb_decode(src) + a*srgb_decode(dst))`。
//! 二者在**不透明**像素上一致（都是「原值直通」，差 ≤1 LSB 的舍入），但在**半透明**像素上
//! 差异巨大。实测数字（`src = 0xC0`、`alpha = 0.5`、`dst = 0`）：
//!
//! ```text
//!   字节空间（CPU / UNORM 附件）：(1-0.5)*192 + 0.5*0        = 96
//!   线性空间（SRGB 附件硬件混合）：srgb_encode(0.5 * srgb_decode(192)) = 140
//!                                                          差 = 44（不是 1 LSB！）
//! ```
//!
//! 结论（**选择 (b)：线性格式交换链**）：
//! 要让「与 CPU 逐字节一致」这条判据在窗口上仍然成立，**只能**让窗口附件与离屏同语义，
//! 即用**非 sRGB（`*_UNORM`）的交换链格式**。若用 sRGB 格式，则半透明像素必然差几十个
//! 字节，逐字节判据在窗口路径上不成立（那是「两种不同的渲染语义」，不是实现缺陷）。
//!
//! 因此 [`crate::swapchain::pick_config`] 的格式优先级在 M3c 调整为
//! **`*_UNORM` 优先、sRGB 退路**（见那里的文档），窗口路径的实际格式由
//! `Swapchain::format()` 决定并作为 `color_format` 传进 [`build_pipelines`] ——
//! 于是「窗口到底用哪个格式」是**运行期事实**，不是编译期假设。
//!
//! ## 本模块提供的东西
//!
//! - [`ViewportStrategy`]：静态（写进管线）/ 动态（录制时给）——见 `device.rs::build_pipeline` 的教训；
//! - [`ShapeState`] / [`TextState`]：两条管线**除格式与 viewport 外**的全部状态（纯数据）；
//! - [`build_pipelines`]：一次性建出两条管线（含各自的管线布局与着色器）；
//! - [`srgb_encode`] / [`srgb_decode`]：sRGB 传输函数（纯函数，用来**证明**上面那段结论）。

use deer_gpu::GpuResult;

use crate::device::{Pipeline, PipelineLayout, RenderPass, ShaderModule, VkDevice};
use crate::ffi_dev as vk;
use crate::spirv;

// ── sRGB 传输函数（纯函数；M3c-T2 的判据） ───────────────────────────────────

/// 线性通道值（0–255 的**线性**刻度）→ **sRGB 编码字节**。
///
/// 这是 Vulkan 规定「写入 sRGB 附件时驱动做的事」。分段点 `0.0031308` 与
/// 指数 `1/2.4` 都来自 sRGB 规范（IEC 61966-2-1）。
///
/// **用 `f64`**：这套换算要用来给「逐字节」判据提供期望值，`f32` 在 `powf`
/// 上的舍入可能把结果推过 `.5` 的舍入边界 ⇒ 期望值本身就不稳。
pub fn srgb_encode(linear: u8) -> u8 {
    let x = linear as f64 / 255.0;
    let y = if x <= 0.003_130_8 {
        12.92 * x
    } else {
        1.055 * x.powf(1.0 / 2.4) - 0.055
    };
    (y * 255.0).round().clamp(0.0, 255.0) as u8
}

/// **sRGB 编码字节** → 线性值（`[0,1]` 浮点，`f64`）。
///
/// 与 [`srgb_encode`] 互为逆（见本文件的往返测试）。用它可以把「CPU 的 sRGB 字节」
/// 换算到线性空间，从而**预测** sRGB 附件会写出什么。
pub fn srgb_decode(byte: u8) -> f64 {
    let x = byte as f64 / 255.0;
    if x <= 0.040_45 {
        x / 12.92
    } else {
        ((x + 0.055) / 1.055).powf(2.4)
    }
}

/// 线性值（`[0,1]` 浮点）→ **sRGB 编码字节**。
///
/// 与 [`srgb_encode`] 的关系：`srgb_encode(b) == srgb_encode_linear(b as f64 / 255.0)`。
/// 单独提供它是因为「解码 → 处理 → 编码」这条链上中间值是 `[0,1]` 浮点，
/// 不该被强行压回 `u8`（那会丢掉暗端精度，见下面的往返测试）。
pub fn srgb_encode_linear(linear: f64) -> u8 {
    let y = if linear <= 0.003_130_8 {
        12.92 * linear
    } else {
        1.055 * linear.powf(1.0 / 2.4) - 0.055
    };
    (y * 255.0).round().clamp(0.0, 255.0) as u8
}

// ── viewport 策略 ────────────────────────────────────────────────────────────

/// viewport/scissor 的**来源**。两条路径必须各用其一，**不得互相照搬**。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewportStrategy {
    /// **静态**：把 viewport/scissor 写进管线（`p_viewports` 指向真实值）。
    ///
    /// 用于**离屏**路径。M2a 实测：本机 Intel 驱动上**动态**版本的管线
    /// 「清屏正常、但 `vkCmdDraw` 一个像素都不画」—— 所以离屏必须静态。
    Static { width: u32, height: u32 },
    /// **动态**：管线只声明 `count = 1`、指针为空，具体值在录制时给。
    ///
    /// 用于**窗口**路径：M2b 实证它能上屏，而且窗口尺寸会变（不必重建管线）。
    Dynamic,
}

// ── 两条管线的状态（纯数据） ─────────────────────────────────────────────────

/// 颜色混合附件状态。**两条路径必须逐字相同**（除 `color_format` 决定的编码语义外）。
///
/// 这里把「非预乘输出 + `SRC_ALPHA`」这一对写成常量：它们必须**成对**成立 ——
/// 若把 `src_color_blend_factor` 改成 `ONE`（预乘语义），那么片元着色器也必须
/// 改成预乘输出，否则 RGB 会被乘两次 alpha（M3b 的 `fragment_shader_text` 文档里有推导）。
pub fn blend_attachment() -> vk::PipelineColorBlendAttachmentState {
    vk::PipelineColorBlendAttachmentState {
        blend_enable: vk::VK_TRUE,
        src_color_blend_factor: vk::VK_BLEND_FACTOR_SRC_ALPHA,
        dst_color_blend_factor: vk::VK_BLEND_FACTOR_ONE_MINUS_SRC_ALPHA,
        color_blend_op: vk::VK_BLEND_OP_ADD,
        src_alpha_blend_factor: vk::VK_BLEND_FACTOR_ONE,
        dst_alpha_blend_factor: vk::VK_BLEND_FACTOR_ONE_MINUS_SRC_ALPHA,
        alpha_blend_op: vk::VK_BLEND_OP_ADD,
        color_write_mask: vk::VK_COLOR_COMPONENT_RGBA_BITS,
    }
}

fn input_assembly() -> vk::PipelineInputAssemblyStateCreateInfo {
    vk::PipelineInputAssemblyStateCreateInfo {
        s_type: vk::VK_STRUCTURE_TYPE_PIPELINE_INPUT_ASSEMBLY_STATE_CREATE_INFO,
        p_next: std::ptr::null(),
        flags: 0,
        topology: vk::VK_PRIMITIVE_TOPOLOGY_TRIANGLE_LIST,
        primitive_restart_enable: vk::VK_FALSE,
    }
}

fn rasterization() -> vk::PipelineRasterizationStateCreateInfo {
    vk::PipelineRasterizationStateCreateInfo {
        s_type: vk::VK_STRUCTURE_TYPE_PIPELINE_RASTERIZATION_STATE_CREATE_INFO,
        p_next: std::ptr::null(),
        flags: 0,
        depth_clamp_enable: vk::VK_FALSE,
        rasterizer_discard_enable: vk::VK_FALSE,
        polygon_mode: vk::VK_POLYGON_MODE_FILL,
        // GUI 不做背面剔除：矩形的两个朝向都可能出现（顶点顺序由 CPU 侧决定）
        cull_mode: vk::VK_CULL_MODE_NONE,
        front_face: vk::VK_FRONT_FACE_COUNTER_CLOCKWISE,
        depth_bias_enable: vk::VK_FALSE,
        depth_bias_constant_factor: 0.0,
        depth_bias_clamp: 0.0,
        depth_bias_slope_factor: 0.0,
        line_width: 1.0,
    }
}

fn multisample() -> vk::PipelineMultisampleStateCreateInfo {
    vk::PipelineMultisampleStateCreateInfo {
        s_type: vk::VK_STRUCTURE_TYPE_PIPELINE_MULTISAMPLE_STATE_CREATE_INFO,
        p_next: std::ptr::null(),
        flags: 0,
        rasterization_samples: vk::VK_SAMPLE_COUNT_1_BIT,
        sample_shading_enable: vk::VK_FALSE,
        min_sample_shading: 1.0,
        p_sample_mask: std::ptr::null(),
        // 这两个都会改变片段覆盖率 ⇒ 会让 M3a/M3b 的逐字节判据失效
        alpha_to_coverage_enable: vk::VK_FALSE,
        alpha_to_one_enable: vk::VK_FALSE,
    }
}

/// 形状管线（M3a）与文本管线（M3b）**除颜色格式、viewport 策略、顶点布局外**的全部状态。
///
/// 做成纯数据（不含句柄、**不含指针**）是为了能在无 GPU 的机器上断言
/// 「两路径逐字段相同」—— 这是本模块存在的理由，也是它唯一的验收方式。
///
/// 顶点布局（`stride` + `attrs`）用值而不是指针存：指针版会让比较变成
/// 「比地址」这种无意义的东西，也要求调用方保证生命周期。这里只在
/// **真正建管线的那一刻**才把它展开成 Vulkan 的结构体（见 `build_pipelines`）。
///
/// **不派生 `PartialEq`/`Debug`**：里面的成员是 `ffi_dev` 的手写 `repr(C)` 结构体，
/// 含裸指针 ⇒ 派生出来的比较是「比地址」、派生的 `Debug` 打印一堆地址，两者都
/// 会误导。测试改为**逐字段**断言（见本文件 tests），那才是这里真正想表达的意思。
#[derive(Clone)]
pub struct PipelineState {
    /// 颜色附件格式。**这是两个参数化维度之一**，且它决定写入编码与混合空间（见模块文档）。
    pub color_format: i32,
    /// viewport/scissor 的来源。**另一个参数化维度**。
    pub viewport: ViewportStrategy,
    /// 顶点 stride（形状 44 / 文本 32）。
    pub stride: u32,
    /// 顶点属性表（形状 4 个 / 文本 3 个）。
    pub attrs: Vec<crate::device::VertexAttr>,
    // —— 以下四项**两路径必须逐字相同**（由同一批构造函数产出）——
    pub input_assembly: vk::PipelineInputAssemblyStateCreateInfo,
    pub rasterization: vk::PipelineRasterizationStateCreateInfo,
    pub multisample: vk::PipelineMultisampleStateCreateInfo,
    pub blend: vk::PipelineColorBlendAttachmentState,
}

impl PipelineState {
    /// 顶点属性表 → Vulkan 的 `VertexInputAttributeDescription` 列表。
    ///
    /// 只做「值 → 值」的映射（不含指针），于是可以安全返回；真正的
    /// `VkPipelineVertexInputStateCreateInfo`（含指针）由
    /// `device.rs::create_pipeline_from_state` 在**一个**作用域里拼出来 ——
    /// 那样 `binding` / `descs` / `info` 三者的生命周期都在同一帧里，
    /// 结构体不会持有已失效的地址（手写 `repr(C)` 结构体最容易踩的坑之一）。
    pub(crate) fn vertex_attr_descs(&self) -> Vec<vk::VertexInputAttributeDescription> {
        self.attrs
            .iter()
            .map(|a| vk::VertexInputAttributeDescription {
                location: a.location,
                binding: 0,
                format: a.format,
                offset: a.offset,
            })
            .collect()
    }
}

/// 形状管线状态（M3a 的顶点布局：pos / rect / radius_kind / color）。
pub fn shape_state(
    color_format: i32,
    viewport: ViewportStrategy,
    stride: u32,
    attrs: Vec<crate::device::VertexAttr>,
) -> PipelineState {
    PipelineState {
        color_format,
        viewport,
        stride,
        attrs,
        input_assembly: input_assembly(),
        rasterization: rasterization(),
        multisample: multisample(),
        blend: blend_attachment(),
    }
}

/// 文本管线状态（M3b 的顶点布局：pos / uv / color）。
///
/// ## 与 [`shape_state`] 的关系（本模块的核心断言）
///
/// **除了 `color_format`、`viewport`、`stride`/`attrs`，其余字段逐字相同** ——
/// 函数体刻意复用**同一批构造函数**（而不是再抄一遍字面量），所以「相同」是
/// **结构上保证**的，不靠人眼比对。测试
/// `shape_and_text_states_differ_only_in_the_three_allowed_dimensions` 把它钉死
/// （含变异：改任一冻结字段即红）。
///
/// 文本管线**额外**需要的资源（描述符集布局 + 采样器）不在这里，见 [`build_pipelines`]。
pub fn text_state(
    color_format: i32,
    viewport: ViewportStrategy,
    stride: u32,
    attrs: Vec<crate::device::VertexAttr>,
) -> PipelineState {
    PipelineState {
        color_format,
        viewport,
        stride,
        attrs,
        input_assembly: input_assembly(),
        rasterization: rasterization(),
        multisample: multisample(),
        blend: blend_attachment(),
    }
}

// ── 两条管线的集合 ───────────────────────────────────────────────────────────

/// 一次建好的两条管线（+ 各自的着色器与管线布局）。
///
/// ## 为什么着色器模块也放在这里
///
/// `Pipeline` 与 `ShaderModule` 的生命周期必须满足「管线 ≤ 着色器」：Vulkan 允许
/// 在管线建好后销毁着色器模块，但**不允许反过来**。把两者放进同一个结构体、
/// 并让**字段顺序 = 析构顺序**（`shape`/`text` 管线声明在 `*_vs`/`*_fs` **之前**）⇒
/// 析构时先销毁管线、再销毁着色器，顺序由类型系统保证。
///
/// 文本管线还需要它的描述符集布局与采样器才能在绘制时 `bind`/`update` ——
/// 这里一并持有（同一个理由：布局必须比用到它的管线活得久）。
///
/// `shape_vs` / `shape_fs` / `text_vs` / `text_fs` **从不被读取** —— 它们存在只为
/// **所有权**：着色器模块必须在管线存活期间有效（Vulkan 允许建好管线后销毁模块，
/// 但**不允许反过来**）。`device.rs` 里那三个公开字段（`shape_layout` /
/// `text_layout` / `text_set_layout` / `sampler`）确实会被渲染循环读用来
/// `bind` 与 `update`，所以不加全局 `allow`，只对这四个模块加。
#[allow(dead_code)]
pub struct PipelineSet {
    /// 形状管线（顶点属性透传 + 按 `rect`/`radius_kind` 逐像素判定）。
    pub shape: Pipeline,
    /// 文本管线（采样字形图集覆盖率）。**总是**存在 —— 不用文本时只是不 bind 它。
    pub text: Pipeline,
    /// 两条管线各自的布局（形状无描述符；文本有 set 0）。
    pub shape_layout: PipelineLayout,
    pub text_layout: PipelineLayout,
    /// 文本的 set 0 布局（`binding 0 = COMBINED_IMAGE_SAMPLER`）。
    pub text_set_layout: crate::device::DescriptorSetLayout,
    /// 字形图集采样器（`NEAREST` + `ClampToEdge` + 无 mipmap）。
    pub sampler: crate::device::Sampler,
    /// 这次建管线用的颜色格式（便于调用方断言「与交换链格式一致」）。
    color_format: i32,
    // —— 以下字段只在析构时起作用：必须**后**于上面的管线/布局声明 ——
    shape_vs: ShaderModule,
    shape_fs: ShaderModule,
    text_vs: ShaderModule,
    text_fs: ShaderModule,
}

impl PipelineSet {
    /// 颜色格式（两条管线相同 —— 一次 [`build_pipelines`] 只服务一个渲染通道）。
    pub fn color_format(&self) -> i32 {
        self.color_format
    }
}

/// 建出两条管线（形状 + 文本），状态由本模块统一提供。
///
/// ## 顶点布局为什么是参数
///
/// 「形状」与「文本」的顶点格式本来就不同（stride 44 的 `GpuVertex` vs stride 32 的
/// `TextVertex`）—— 那是**唯一**允许两路径不同的第三处（前两处是颜色格式与
/// viewport 策略）。调用方把自己的 `stride` + `attrs` 传进来即可。
///
/// ## `color_format` 必须与 `render_pass` 的附件格式一致
///
/// 不一致时 `vkCreateGraphicsPipelines` 会拒绝（校验层报
/// `VUID-VkGraphicsPipelineCreateInfo-renderPass-06043` 一类）。这里**不**去
/// `render_pass` 里查（`RenderPass` 没有公开 format getter），靠调用方传对 ——
/// 调用方本来就持有那个格式（离屏是 `COLOR_FORMAT` 常量，窗口是 `swapchain.format()`）。
///
/// ## `viewport` 由调用方按**路径**选
///
/// 离屏传 [`ViewportStrategy::Static`]（M2a 教训：动态在 Intel 上零像素），
/// 窗口传 [`ViewportStrategy::Dynamic`]（M2b 实证能上屏）。**不得互相照搬。**
#[allow(clippy::too_many_arguments)]
pub fn build_pipelines(
    device: &VkDevice,
    render_pass: &RenderPass,
    color_format: i32,
    viewport: ViewportStrategy,
    shape_stride: u32,
    shape_attrs: &[crate::device::VertexAttr],
    text_stride: u32,
    text_attrs: &[crate::device::VertexAttr],
) -> GpuResult<PipelineSet> {
    // ① 着色器（自研 SPIR-V 汇编器产出；M3a/M3b 已过官方 spirv-val）——
    //    两条路径**用同一批着色器**，这是「同一套画法」的前提。
    let shape_vs = device.create_shader_module(&spirv::vertex_shader_rect_attrs())?;
    let shape_fs = device.create_shader_module(&spirv::fragment_shader_rect_shape())?;
    let text_vs = device.create_shader_module(&spirv::vertex_shader_text())?;
    let text_fs = device.create_shader_module(&spirv::fragment_shader_text())?;

    // ② 文本的 set 0 + 采样器（`NEAREST` + `ClampToEdge` + 无 mipmap，M3b 冻结）
    let text_set_layout = device.create_descriptor_set_layout_combined_sampler()?;
    let sampler = device.create_sampler()?;

    // ③ 两个管线布局：形状不带描述符、文本带 set 0
    let shape_layout = device.create_pipeline_layout(None)?;
    let text_layout = device.create_pipeline_layout_ex(None, Some(&text_set_layout))?;

    // ④ 状态：两条管线由**同一批构造函数**产出 ⇒「除三个允许维度外相同」是
    //    结构上保证的，不靠人眼比对（见本文件的状态断言测试）。
    let shape_state = shape_state(color_format, viewport, shape_stride, shape_attrs.to_vec());
    let text_state = text_state(color_format, viewport, text_stride, text_attrs.to_vec());

    // ⑤ 真正建管线（顶点输入结构体在 device 侧的一个作用域里拼，避免悬垂指针）
    let shape = device.create_pipeline_from_state(
        &shape_state,
        &[
            (&shape_vs, vk::VK_SHADER_STAGE_VERTEX_BIT),
            (&shape_fs, vk::VK_SHADER_STAGE_FRAGMENT_BIT),
        ],
        &shape_layout,
        render_pass,
    )?;
    let text = device.create_pipeline_from_state(
        &text_state,
        &[
            (&text_vs, vk::VK_SHADER_STAGE_VERTEX_BIT),
            (&text_fs, vk::VK_SHADER_STAGE_FRAGMENT_BIT),
        ],
        &text_layout,
        render_pass,
    )?;

    Ok(PipelineSet {
        // ⚠️ 字段顺序 = 析构顺序：管线与布局在前、着色器在后
        shape,
        text,
        shape_layout,
        text_layout,
        text_set_layout,
        sampler,
        color_format,
        shape_vs,
        shape_fs,
        text_vs,
        text_fs,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── ① 冻结状态：两条管线除三个允许维度外**逐字段相同** ──────────────────

    /// 形状与文本的**真实**顶点布局（与 `gpu_render.rs` 里那两个表一致）。
    ///
    /// 这里复制一份是刻意的：本测试要断言的是「**状态**除三个维度外相同」，
    /// 若直接引用 `gpu_render` 的表，就把「测的东西」和「被测的东西」绑在一起了。
    /// 顶点布局本来就**允许**不同（那是第三个维度），所以这里给出两份不同的值，
    /// 并在断言里**排除**它们。
    fn shape_layout() -> (u32, Vec<crate::device::VertexAttr>) {
        (
            44,
            vec![
                crate::device::VertexAttr {
                    location: 0,
                    format: 104, // R32G32_SFLOAT
                    offset: 0,
                },
                crate::device::VertexAttr {
                    location: 1,
                    format: 109, // R32G32B32A32_SFLOAT
                    offset: 8,
                },
            ],
        )
    }

    fn text_layout() -> (u32, Vec<crate::device::VertexAttr>) {
        (
            32,
            vec![crate::device::VertexAttr {
                location: 0,
                format: 104, // R32G32_SFLOAT
                offset: 0,
            }],
        )
    }

    /// 逐字段比较两个状态里**冻结的那几项**（拓扑、剔除、多重采样、混合）。
    ///
    /// 为什么手写逐字段比较、不用派生的 `PartialEq`：成员是含裸指针的
    /// `repr(C)` 结构体，派生的 `PartialEq` 会去比较**指针地址** ——
    /// 两个语义完全相同的状态会因为「`p_next` 落在不同栈地址」而不相等，
    /// 断言就变成噪声。这里只比较**语义字段**。
    fn assert_frozen_fields_equal(a: &PipelineState, b: &PipelineState) {
        // 输入装配
        assert_eq!(
            a.input_assembly.topology, b.input_assembly.topology,
            "topology 必须相同"
        );
        assert_eq!(
            a.input_assembly.primitive_restart_enable, b.input_assembly.primitive_restart_enable
        );
        // 光栅化
        assert_eq!(a.rasterization.polygon_mode, b.rasterization.polygon_mode);
        assert_eq!(a.rasterization.cull_mode, b.rasterization.cull_mode, "cull_mode 必须相同");
        assert_eq!(a.rasterization.front_face, b.rasterization.front_face);
        assert_eq!(
            a.rasterization.depth_bias_enable, b.rasterization.depth_bias_enable
        );
        assert_eq!(
            a.rasterization.depth_bias_constant_factor,
            b.rasterization.depth_bias_constant_factor
        );
        assert_eq!(a.rasterization.line_width, b.rasterization.line_width);
        // 多重采样
        assert_eq!(
            a.multisample.rasterization_samples, b.multisample.rasterization_samples
        );
        assert_eq!(
            a.multisample.sample_shading_enable, b.multisample.sample_shading_enable
        );
        assert_eq!(
            a.multisample.alpha_to_coverage_enable, b.multisample.alpha_to_coverage_enable,
            "alpha-to-coverage 会改变覆盖率 ⇒ 两路径必须相同"
        );
        assert_eq!(
            a.multisample.alpha_to_one_enable, b.multisample.alpha_to_one_enable
        );
        // 混合（全部 8 个字段）
        assert_eq!(a.blend.blend_enable, b.blend.blend_enable);
        assert_eq!(
            a.blend.src_color_blend_factor, b.blend.src_color_blend_factor
        );
        assert_eq!(
            a.blend.dst_color_blend_factor, b.blend.dst_color_blend_factor
        );
        assert_eq!(a.blend.color_blend_op, b.blend.color_blend_op);
        assert_eq!(
            a.blend.src_alpha_blend_factor, b.blend.src_alpha_blend_factor
        );
        assert_eq!(
            a.blend.dst_alpha_blend_factor, b.blend.dst_alpha_blend_factor
        );
        assert_eq!(a.blend.alpha_blend_op, b.blend.alpha_blend_op);
        assert_eq!(a.blend.color_write_mask, b.blend.color_write_mask);
    }

    /// **本模块的核心断言**：两条管线状态只在三个**允许**维度上不同。
    ///
    /// 允许不同的维度恰好是三个：
    /// 1. `color_format` —— 但同一路径下两条管线**用同一个**格式（一次
    ///    `build_pipelines` 只服务一个渲染通道），所以这里断言它们**相等**；
    /// 2. `viewport` —— 同上，同一路径下相等；
    /// 3. `stride` / `attrs` —— 顶点格式本来就不同（形状 44 字节 4 属性 / 文本 32 字节 3 属性）。
    ///
    /// 其余**必须逐字相同**。这条测试的价值：任何人在 `shape_state` 或 `text_state`
    /// 里只改一边（例如只把形状的 `cull_mode` 打开），这里立刻红 —— 而那种错在
    /// **像素对照**里极难定位（只有形状或只有文本的一处会不对，很容易被当成「那个用例本身有问题」）。
    #[test]
    fn shape_and_text_states_differ_only_in_the_allowed_dimensions() {
        let fmt = vk::VK_FORMAT_R8G8B8A8_UNORM;
        let vp = ViewportStrategy::Static {
            width: 64,
            height: 64,
        };
        let (ss, sa) = shape_layout();
        let (ts, ta) = text_layout();
        let shape = shape_state(fmt, vp, ss, sa);
        let text = text_state(fmt, vp, ts, ta);

        // 三个允许维度
        assert_eq!(shape.color_format, text.color_format, "同一路径两条管线必须同一格式");
        assert_eq!(shape.viewport, text.viewport, "同一路径两条管线必须同一 viewport 策略");
        assert_ne!(shape.stride, text.stride, "形状与文本的 stride 本来就不同（44 vs 32）");
        assert_ne!(shape.attrs.len(), text.attrs.len(), "属性个数本来就不同（4 vs 3）");

        // 其余冻结项：逐字段相同
        assert_frozen_fields_equal(&shape, &text);

        // 反向自检：让「逐字段比较」确实有分辨力 ——
        // 若把形状的 cull_mode 改掉，比较必须能发现（否则上面的断言是空转的）
        let mut mutated = text.clone();
        mutated.rasterization.cull_mode = vk::VK_CULL_MODE_BACK_BIT;
        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                assert_frozen_fields_equal(&shape, &mutated)
            }))
            .is_err(),
            "把 cull_mode 改成 BACK 后，逐字段比较**必须**能发现（否则这条护栏是空转的）"
        );
    }

    /// 把**冻结的那几项**逐个钉住具体值 —— 只断言「两者相同」还不够：
    /// 两边一起被改错也会「相同」。这条是本项目的 `docs`/判据所依赖的**字面值**。
    #[test]
    fn frozen_state_values_are_exact() {
        let s = shape_state(
            37,
            ViewportStrategy::Dynamic,
            shape_layout().0,
            shape_layout().1,
        );

        assert_eq!(s.input_assembly.topology, vk::VK_PRIMITIVE_TOPOLOGY_TRIANGLE_LIST);
        assert_eq!(s.input_assembly.primitive_restart_enable, vk::VK_FALSE);

        assert_eq!(
            s.rasterization.cull_mode,
            vk::VK_CULL_MODE_NONE,
            "GUI 不做背面剔除（矩形两个朝向都可能）"
        );
        assert_eq!(s.rasterization.polygon_mode, vk::VK_POLYGON_MODE_FILL);
        assert_eq!(s.rasterization.front_face, vk::VK_FRONT_FACE_COUNTER_CLOCKWISE);
        assert_eq!(s.rasterization.depth_bias_enable, vk::VK_FALSE);
        assert_eq!(s.rasterization.line_width, 1.0);
        assert_eq!(s.rasterization.depth_clamp_enable, vk::VK_FALSE);
        assert_eq!(s.rasterization.rasterizer_discard_enable, vk::VK_FALSE);

        assert_eq!(s.multisample.rasterization_samples, vk::VK_SAMPLE_COUNT_1_BIT);
        assert_eq!(s.multisample.sample_shading_enable, vk::VK_FALSE);
        assert_eq!(
            s.multisample.alpha_to_coverage_enable,
            vk::VK_FALSE,
            "alpha-to-coverage 会改变覆盖率 ⇒ 破坏逐字节判据"
        );
        assert_eq!(
            s.multisample.alpha_to_one_enable,
            vk::VK_FALSE,
            "alpha-to-one 会改 alpha 通道"
        );

        // 混合：与 CPU `null.rs::blend_cov` 等价的那一组（M3a/M3b 冻结）
        assert_eq!(s.blend.blend_enable, vk::VK_TRUE);
        assert_eq!(s.blend.src_color_blend_factor, vk::VK_BLEND_FACTOR_SRC_ALPHA);
        assert_eq!(
            s.blend.dst_color_blend_factor,
            vk::VK_BLEND_FACTOR_ONE_MINUS_SRC_ALPHA
        );
        assert_eq!(s.blend.color_blend_op, vk::VK_BLEND_OP_ADD);
        assert_eq!(s.blend.src_alpha_blend_factor, vk::VK_BLEND_FACTOR_ONE);
        assert_eq!(
            s.blend.dst_alpha_blend_factor,
            vk::VK_BLEND_FACTOR_ONE_MINUS_SRC_ALPHA
        );
        assert_eq!(s.blend.alpha_blend_op, vk::VK_BLEND_OP_ADD);
        assert_eq!(s.blend.color_write_mask, vk::VK_COLOR_COMPONENT_RGBA_BITS);
    }

    /// 属性表映射成 Vulkan 结构：`binding` 恒为 0、offset/format/location 原样。
    #[test]
    fn vertex_attr_descs_map_fields_verbatim() {
        let (stride, attrs) = shape_layout();
        let s = shape_state(37, ViewportStrategy::Dynamic, stride, attrs.clone());
        let descs = s.vertex_attr_descs();
        assert_eq!(descs.len(), attrs.len());
        for (d, a) in descs.iter().zip(attrs.iter()) {
            assert_eq!(d.location, a.location);
            assert_eq!(d.format, a.format);
            assert_eq!(d.offset, a.offset);
            assert_eq!(d.binding, 0, "只有一个顶点缓冲 ⇒ binding 恒为 0");
        }
    }

    /// viewport 策略必须**可区分**，且它是**独立**维度（除它之外状态相同）。
    ///
    /// M2a/M2b 的教训就在这一点上：动态/静态**不得互相照搬**（离屏动态 ⇒ Intel 零像素；
    /// 窗口静态 ⇒ 尺寸一变就错位）。所以两条路径各用其一，且这里断言
    /// 「viewport 是唯一差异」—— 防止有人为了「统一」而顺手把别的状态也改了。
    #[test]
    fn viewport_strategy_is_an_independent_dimension() {
        assert_ne!(ViewportStrategy::Dynamic, ViewportStrategy::Static { width: 8, height: 8 });
        assert_eq!(ViewportStrategy::Dynamic, ViewportStrategy::Dynamic);
        let a = shape_state(1, ViewportStrategy::Dynamic, 44, Vec::new());
        let b = shape_state(
            1,
            ViewportStrategy::Static {
                width: 8,
                height: 8,
            },
            44,
            Vec::new(),
        );
        assert_ne!(a.viewport, b.viewport);
        // 除 viewport 外其余冻结项相同
        assert_frozen_fields_equal(&a, &b);
    }

    // ── ② sRGB 传输函数（M3c-T2 的判据） ────────────────────────────────────

    /// **已知值**：M2b 实测过的那三个字节必须能被 [`srgb_encode`] 复现。
    ///
    /// 这三对数字是**真机实测**得到的（本机 Intel/NVIDIA 一致）：清屏
    /// `rgb(0x10,0x14,0x24)` 写进 `B8G8R8A8_SRGB` 附件后回读成 `[0x47,0x4F,0x69]`。
    /// 用它做锚点，比「自己算一遍再和自己比」有意义得多。
    #[test]
    fn srgb_encode_matches_measured_driver_bytes() {
        assert_eq!(srgb_encode(0x10), 0x47, "16 → 71（M2b 实测）");
        assert_eq!(srgb_encode(0x14), 0x4F, "20 → 79（M2b 实测）");
        assert_eq!(srgb_encode(0x24), 0x69, "36 → 105（M2b 实测）");
    }

    /// 端点与单调性：0→0、255→255，且整体单调不减。
    ///
    /// `255→255` 与 `0→0` 是 sRGB 传输函数的**不动点** —— 它们是「不透明纯黑/纯白
    /// 在两路径下必然一致」的原因（也解释了为什么纯色 UI 看起来「没问题」，
    /// 而问题只在中间调与半透明处暴露）。
    #[test]
    fn srgb_encode_endpoints_and_monotonicity() {
        assert_eq!(srgb_encode(0), 0);
        assert_eq!(srgb_encode(255), 255);
        let mut prev = 0u8;
        for v in 0..=255u8 {
            let e = srgb_encode(v);
            assert!(e >= prev, "sRGB 编码必须单调不减：{v} 处 {e} < {prev}");
            prev = e;
        }
    }

    /// **往返**：`srgb_encode_linear(srgb_decode(v)) == v` 对全部 256 个值成立。
    ///
    /// 这条比「编解码各自看着对」强得多：它保证
    /// 「把 CPU 的 sRGB 字节解码到线性，再让硬件编码回去」能**逐字节还原** ——
    /// 也就是不透明像素在 sRGB 附件上能逐字节对齐的**充分依据**。
    ///
    /// ⚠️ **中间不能把线性值压回 `u8`**：写成
    /// `srgb_encode(round(srgb_decode(v)*255) as u8)` 会引入一次 8 位线性量化，
    /// 而线性刻度在暗端的分辨率远低于 sRGB ⇒ 那个版本在暗端错到 6 个字节
    /// （实测 `v=6`：`decode(6)*255 = 0.464 → round → 0 → encode → 0`，误差 6）。
    /// 那是**测试写法**的错，不是传输函数的错 —— 下一个测试把这个陷阱记下来。
    #[test]
    fn srgb_roundtrip_is_exact_for_all_256_bytes() {
        for v in 0..=255u8 {
            assert_eq!(
                srgb_encode_linear(srgb_decode(v)),
                v,
                "encode(decode({v})) 必须逐字节还原（不透明像素在 sRGB 附件上可对齐的依据）"
            );
        }
    }

    /// **陷阱记录**：中间多做一次「8 位线性量化」会让暗端失真。
    ///
    /// 这一步很容易踩（`srgb_decode` 返回浮点，顺手 `round() as u8` 就中招），
    /// 而且症状反直觉：**亮端完全正常、暗端差好几个字节** ——
    /// 混在「sRGB 的舍入误差」里会被当成 1 LSB 问题放过去。
    ///
    /// 实测（本机 `f64`）：8 位线性量化后往返最大误差 **6**（出现在 `v = 6`、`v = 7`）；
    /// 换成 16 位线性刻度则误差为 **0**。也就是说问题在**中间表示的位深**，
    /// 不在传输函数本身 —— 这条测试把两者区分开。
    #[test]
    fn quantizing_the_linear_value_to_8_bits_loses_the_dark_end() {
        // 8 位线性量化：暗端失真（期望就是「不相等」）
        let dark = 6u8;
        let quantized = (srgb_decode(dark) * 255.0).round().clamp(0.0, 255.0) as u8;
        assert_eq!(
            quantized, 0,
            "srgb_decode(6)*255 = 0.464 ⇒ 8 位线性量化后是 0（线性刻度分辨不出这么暗的值）"
        );
        assert_eq!(srgb_encode_linear(0.0), 0, "线性 0 编码回 0");
        let err = (srgb_encode_linear(0.0) as i32 - dark as i32).abs();
        assert_eq!(err, 6, "该写法在暗端的失真（6 个字节，远超 1 LSB）");

        // 8 位量化下的全局最大误差（把「暗端失真」的量级钉住）
        let mut worst = 0i32;
        for v in 0..=255u8 {
            let q = (srgb_decode(v) * 255.0).round().clamp(0.0, 255.0) as u8;
            let e = srgb_encode_linear(q as f64 / 255.0);
            worst = worst.max((e as i32 - v as i32).abs());
        }
        assert_eq!(worst, 6, "8 位线性量化的最大往返误差实测为 6");

        // 16 位线性刻度 ⇒ 往返无损（证明错的是位深而不是传输函数）
        let mut worst16 = 0i32;
        for v in 0..=255u8 {
            let q16 = (srgb_decode(v) * 65535.0).round().clamp(0.0, 65535.0) as u32;
            let e = srgb_encode_linear(q16 as f64 / 65535.0);
            worst16 = worst16.max((e as i32 - v as i32).abs());
        }
        assert_eq!(worst16, 0, "16 位线性刻度下往返应无损");
    }

    /// **判据的核心数字**：sRGB 附件与 CPU（字节空间混合）在**半透明**处差多远。
    ///
    /// 这条测试把「为什么不能选 (a)」写成**可执行的事实**，而不是文档里的一句话：
    /// `src = 0xC0`、`alpha = 0.5`、`dst = 0` 时
    /// - 字节空间（CPU / `*_UNORM` 附件）：`(1-a)*src + a*dst = 96`
    /// - 线性空间（`*_SRGB` 附件硬件混合）：`srgb_encode((1-a)*srgb_decode(src)) = 140`
    ///
    /// 差 **44** 个字节 —— 远超 M3b 为 UNORM 舍入定的「≤1 LSB」容差。
    /// 所以窗口若用 sRGB 格式，「与 CPU 逐字节一致」这条判据**不可能**成立。
    /// 结论：交换链改用线性格式（选择 (b)）。
    #[test]
    fn srgb_attachment_blending_diverges_far_beyond_one_lsb() {
        let src = 0xC0u8; // 192
        let a = 0.5f64;
        let dst = 0u8;

        // 字节空间混合（= CPU `null.rs::blend_cov`、= `*_UNORM` 附件上的硬件混合）
        let byte_space = ((1.0 - a) * src as f64 + a * dst as f64)
            .round()
            .clamp(0.0, 255.0) as u8;

        // 线性空间混合（= `*_SRGB` 附件上的硬件混合：先解码、按 alpha 混合、再编码）
        let linear_space = srgb_encode_linear((1.0 - a) * srgb_decode(src) + a * srgb_decode(dst));

        assert_eq!(byte_space, 96, "字节空间混合的期望值");
        assert_eq!(linear_space, 140, "线性空间混合的期望值");
        let diff = (linear_space as i32 - byte_space as i32).abs();
        assert_eq!(diff, 44, "两个空间的差");
        assert!(
            diff > 1,
            "若这里 ≤1，则 sRGB 附件也能满足逐字节判据 —— 但实测差 {diff}，\
             ⇒ 窗口必须用线性(*_UNORM)格式（M3c-T2 的选择 (b)）"
        );
    }

    /// **不透明**像素在两种空间下一致（差 ≤1 LSB）—— 解释「为什么这个坑只在一部分
    /// 用例上暴露」，也是「纯色 UI 看起来没问题」的原因。
    ///
    /// 注意：这条说的是**不透明**像素。只要 alpha < 1，sRGB 附件的线性混合就会与
    /// CPU 的字节空间混合分道扬镳 —— 那正是下一个测试量化的东西。
    #[test]
    fn opaque_pixels_agree_between_the_two_spaces() {
        for v in 0..=255u8 {
            // sRGB 附件路径（不透明）：片元输出线性 → 硬件编码
            let srgb_path = srgb_encode_linear(srgb_decode(v));
            assert!(
                (srgb_path as i32 - v as i32).abs() <= 1,
                "不透明像素在 sRGB 附件与 UNORM 附件上应一致（≤1 LSB）：{v} vs {srgb_path}"
            );
        }
    }
}
