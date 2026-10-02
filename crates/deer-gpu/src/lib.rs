//! # deer-gpu —— GPU 硬件抽象层（HAL）
//!
//! **这是 deer-gui「完全从头」的边界。** 本 crate 不依赖 `wgpu`、`ash`、`vulkano`
//! 或任何图形库：它只定义**后端必须实现的契约**，以及几何/绘制列表这类平台无关数据。
//!
//! ## 为什么要有这一层
//!
//! 「自己写 Vulkan / DX12 / Metal」如果没有抽象层，就是三份互不相干的实现，
//! 每个新后端都要重做窗口、交换链、资源生命周期与渲染器。有了 HAL：
//!
//! - **加一个后端 = 实现 [`Backend`]**（外加 `Device` / `Swapchain` / `CommandEncoder`）；
//! - **渲染器只写一次**：它面向 [`DrawList`]，与后端无关；
//! - **可在无 GPU 环境测**：`deer-gpu::null` 提供一个纯 CPU 后端，让渲染器逻辑
//!   在 CI 里被断言（不需要驱动）。
//!
//! ## 「从头」到底从哪开始
//!
//! 本层**包含**：设备/队列/交换链/资源/管线/命令缓冲的抽象，渲染器，同步语义。
//! 本层**不包含**：Vulkan/DX12/Metal 的 API 绑定 —— 那是各后端 crate 的事
//! （`deer-vk` 自己声明 extern 符号，不引 `ash`）。

#![forbid(unsafe_code)]
#![deny(clippy::all)]

pub mod atlas;
pub mod draw;
pub mod error;
pub mod font;
pub mod glyph;
pub mod interact;
pub mod measure;
pub mod null;
pub mod png;
pub mod raster;
pub mod render;
pub mod text;

pub use atlas::GlyphAtlas;
pub use draw::{Color, DrawCmd, DrawList, RectI, TextureId};
pub use error::{GpuError, GpuResult};
pub use font::{Contour, Font, Glyph, Point, Segment};
pub use glyph::{AtlasSlot, GlyphImage, GlyphKey};
pub use interact::{
    FieldText, InteractState, InteractiveRenderer, ScrollView, build_interactive_draw_list,
    build_interactive_draw_list_with_texts,
};
pub use measure::FontMeasure;
pub use raster::Rasterizer;
pub use render::{DefaultRenderer, NullRenderer, build_draw_list};
pub use text::{GlyphPlacement, TextEngine};

use deer_layout::Geometry;
use deer_layout::node::Node;

/// 一个 GPU 后端的入口。实现它即可被 deer-gui 使用。
///
/// 约定：
/// - `new` 不得在初始化失败时 panic —— 返回 [`GpuError`] 让上层决定回退策略；
/// - 所有资源句柄用不透明 id（`u32`/`u64`），**不暴露后端类型**；
/// - `Surface` 的尺寸变化通过 [`Swapchain::resize`] 表达，后端负责重建交换链。
pub trait Backend {
    /// 后端名（用于日志与“当前用哪个后端”的断言）。
    fn name(&self) -> &'static str;
    /// 枚举可用适配器（显卡）。返回空表示没有可用设备。
    fn adapters(&self) -> Vec<AdapterInfo>;
    /// 用指定适配器打开设备。
    fn open(&self, adapter: AdapterIndex) -> GpuResult<Box<dyn Device>>;
}

pub type AdapterIndex = usize;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdapterInfo {
    pub name: String,
    /// 设备类型（Vulkan 的 `VkPhysicalDeviceType` 语义）。
    pub kind: AdapterKind,
    /// 驱动版本（可读形式，仅用于诊断）。
    pub driver: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdapterKind {
    DiscreteGpu,
    IntegratedGpu,
    VirtualGpu,
    Cpu,
    Other,
}

/// 绘制目标的格式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetFormat {
    /// 8-bit RGBA，非线性 sRGB。
    Bgra8Srgb,
    Rgba8Srgb,
    /// 8-bit RGBA 线性（截图/回读时用）。
    Rgba8Unorm,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Extent {
    pub width: u32,
    pub height: u32,
}

/// 打开的设备：能建交换链、能建资源、能提交命令。
pub trait Device {
    fn info(&self) -> &AdapterInfo;

    /// 创建一个面向原生窗口的交换链。
    ///
    /// # Safety 语义
    /// `window` 必须是仍存活的窗口句柄；后端不得比它活得更久。
    /// 本 trait 用 `RawWindowHandle` 这种不透明形式表达，避免 HAL 依赖具体窗口库。
    fn create_swapchain(
        &mut self,
        window: RawWindowHandle,
        extent: Extent,
        format: TargetFormat,
    ) -> GpuResult<Box<dyn Swapchain>>;

    /// 分配一块可被 GPU 读的纹理（供字形图集 / 图片用）。
    fn create_texture(&mut self, desc: TextureDesc) -> GpuResult<TextureId>;

    /// 上传像素到纹理。
    fn upload_texture(&mut self, id: TextureId, data: &[u8], region: TextureRegion) -> GpuResult<()>;

    /// 开始记录一帧命令。
    fn begin_frame(&mut self) -> GpuResult<Box<dyn Frame>>;

    /// 等待 GPU 空闲（截图、关窗、资源销毁前用）。
    fn wait_idle(&mut self) -> GpuResult<()>;
}

/// 平台的窗口句柄。**不依赖 winit**：只带必要的原生标识。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RawWindowHandle {
    pub platform: Platform,
    /// Windows: HWND；X11: Window；Wayland: wl_surface 指针；macOS: NSView 指针。
    pub handle: usize,
    pub display: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    Windows,
    X11,
    Wayland,
    MacOs,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextureDesc {
    pub width: u32,
    pub height: u32,
    pub format: TargetFormat,
    /// 是否允许 CPU 回读（截图与测试用）。
    pub readable: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextureRegion {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// 交换链。
pub trait Swapchain {
    fn extent(&self) -> Extent;
    fn format(&self) -> TargetFormat;
    /// 重建（窗口尺寸变化 / 交换链过期）。
    fn resize(&mut self, extent: Extent) -> GpuResult<()>;
}

/// 一帧：记录命令 → 提交 → 呈现。
pub trait Frame {
    /// 记录绘制列表。
    ///
    /// ## `TextEngine` 是**按帧传入**的参数，不是设备状态（T1.1 的设计决策）
    ///
    /// 绘制列表里的 `DrawCmd::Text` 需要字形图集 ⇒ 需要一个 [`TextEngine`]。
    /// 它**不能**挂在 `Device` 上（那会让 HAL 的 `Device` 第一次变成有状态，
    /// 而 `create_swapchain` / `create_texture` / `begin_frame` 至今都不保留跨调用状态），
    /// 也**不能**由后端自己造（字体是上层的事）。
    ///
    /// 传入形态与既有的窗口路径一致：`WindowedRenderer::draw_and_present(list, text)`
    /// 早就是这个形状（`Option<&mut TextEngine>`）—— 本 trait 只是把它对齐过来。
    ///
    /// ## 语义（后端**必须**遵守，不许静默忽略）
    ///
    /// - 列表里**没有**文本命令 ⇒ 可以传 `None`；
    /// - 列表里**有**文本命令而传了 `None` ⇒ 返回 [`GpuError::Unsupported`]，
    ///   **不许**静默丢弃（「窗口里少了一段字」是查不出的 bug）；
    /// - 文本假阳性（空串 / `size <= 0` / 被裁空）不在此列：那是数据问题，
    ///   后端跳过并计数即可。
    fn record(&mut self, list: &DrawList, text: Option<&mut TextEngine>) -> GpuResult<()>;
    /// 回读这一帧的图像（`readable` 目标；截图与自动测试用）。
    ///
    /// ## 调用时序（T1.4 的语义决策：**不引入隐式呈现**）
    ///
    /// 本方法在**提交之前**被调用 —— 即 [`Frame::record`] 之后、
    /// [`Frame::submit_and_present`] 之前。这个时序对两类目标的含义完全不同：
    ///
    /// - **离屏 / 显式 `readable` 的目标**：数据在命令录制完就已落在目标上，本方法能拿到像素；
    /// - **交换链**：Vulkan 的交换链图像只有**呈现之后**才保证可读。本方法偏偏在呈现前调用，
    ///   两者**前提冲突**。此处刻意**不做「替调用方隐式呈现一次再回读」**——
    ///   那会把一个「读」变成有副作用的动作，也让「这一帧到底有没有上屏」变得说不清。
    ///
    /// ## 后端**必须**遵守的行为
    ///
    /// - 目标确实可在提交前回读 ⇒ 返回像素；
    /// - 目标在这个时序下拿不到有效数据（典型是交换链）⇒ 返回 [`GpuError::Unsupported`]，
    ///   并且**错误信息要指出真正可用的入口** —— 「报错但不告诉你怎么办」等于没报错。
    ///   `deer-vk` 的写法：`WindowedRenderer::read_back_last_frame()`（在
    ///   `render_and_present()` **之后**调，取回上一帧的像素）；离屏回读用
    ///   `OffscreenRenderer`。该契约由 `deer-vk` 侧的一条用例直接钉住（断言错误信息
    ///   里出现替代入口的名字）。
    /// - 这条不是「以后再补」的占位：`Unsupported` 是**语义选择的结果**，不是一个 TODO。
    fn read_pixels(&mut self) -> GpuResult<Vec<u8>>;
    /// 提交并呈现。返回是否成功呈现（`false` = 交换链过期，调用方应 resize 后重试）。
    fn submit_and_present(self: Box<Self>) -> GpuResult<PresentResult>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PresentResult {
    Presented,
    /// 交换链过期 / 窗口尺寸变了 ⇒ 调用方 `resize` 后重试。
    OutOfDate,
}

/// 把「节点树 + 几何 + 主题」翻译成平台无关的 [`DrawList`]。
///
/// **这是唯一需要为新控件类型改动的地方**（加一种 `DrawCmd` 分支），
/// 与后端完全解耦 —— 后端只认 `DrawList`。
pub trait Renderer {
    fn build_draw_list(&self, tree: &Node, geo: &Geometry, theme: &Theme) -> DrawList;
}

/// 主题（第一步只带必要的颜色与字号；真正的令牌体系在后续迁移）。
#[derive(Debug, Clone, PartialEq)]
pub struct Theme {
    pub text: Color,
    pub text_dim: Color,
    pub surface: Color,
    pub border: Color,
    pub accent: Color,
    pub on_accent: Color,
    pub font_size: f32,
    pub line_height: f32,
}

impl Default for Theme {
    fn default() -> Self {
        Theme {
            text: Color::rgb(0xe6, 0xe8, 0xef),
            text_dim: Color::rgb(0x8b, 0x93, 0xa7),
            surface: Color::rgba(20, 22, 32, 0.55),
            border: Color::rgb(0x2a, 0x2f, 0x3f),
            accent: Color::rgb(0x4c, 0x8d, 0xff),
            on_accent: Color::rgb(0xff, 0xff, 0xff),
            font_size: 13.0,
            line_height: 18.0,
        }
    }
}
