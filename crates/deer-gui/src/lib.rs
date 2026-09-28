//! # deer-gui
//!
//! **门面 crate**：一条 `use` 拿到运行时全部能力，并给出「离屏渲染成图片」的便捷入口。
//!
//! ## 现在能做什么 / 还不能做什么
//!
//! | 能力 | 状态 |
//! |---|---|
//! | 构筑节点树（命令式 / `.dui` 场景文件） | ✅ |
//! | 布局计算（几何表）、命中测试 | ✅ |
//! | 树 + 几何 → 绘制列表 → **像素**（CPU 后端） | ✅ |
//! | 把像素写成 PNG 文件 | ✅ |
//! | **真实字体字形**（零依赖 TTF 解析 + 光栅化 + 图集 + 真实度量） | ✅ |
//! | **渲染到窗口 / 屏幕上**（M2b：窗口 + `VkSurfaceKHR` + 交换链 + 呈现） | ✅ 需开 `window` feature |
//! | GPU 侧**界面**（Vulkan 消费 `DrawList`，窗口里显示真实界面） | ✅ M3a/M3b/M3c（与 CPU 逐像素对照） |
//! | GPU 侧文本（把字形图集上传给 Vulkan） | ✅ M3b（`R8_UNORM` 图集 + 最近邻采样） |
//! | 输入与焦点（事件通路 + 命中/状态机 + `Tab` 焦点 + 文本输入 + 脚本重放 + 不脏不画） | ✅ M5（**仍未做**的部分见 `docs/features/input.md` 第 6 节） |
//!
//! ## 最小用法
//!
//! ```no_run
//! use deer_gui::prelude::*;
//! use deer_gui::render_tree_to_png;
//!
//! // ① 用命令式 API 建树（也可以用 .dui 场景文件：`parse_scene(...)`）
//! let mut app = Builder::new(Kind::Column, "app").padding(12.0).gap(8.0);
//! app.text("标题");
//! app.container_opts(Kind::Row, "bar", L::new().gap(8.0).to_props(), |r| {
//!     r.button("确定");
//!     r.button("取消");
//! });
//! let tree = app.build();
//!
//! // ② 一步渲染成 PNG 文件
//! let png = render_tree_to_png(&tree, 320, 200, Theme::default()).unwrap();
//! std::fs::write("ui.png", png).unwrap();
//! ```

#![forbid(unsafe_code)]
#![deny(clippy::all)]

pub use deer_gpu::{self as gpu, DrawCmd, DrawList, GpuError, GpuResult, Theme};
pub use deer_layout::{self as layout, Node};
pub use deer_vk::{self as vk, VkBackend};

/// 交互层（M5-2/M5-3，**纯逻辑**）：命中测试、裁剪快照、悬停/按下/点击/焦点/文本输入。
///
/// 不碰窗口、不碰 GPU —— 输入是值、输出是值，所以整条交互链能在无 winit / 无 Vulkan
/// 的环境里被单测覆盖（`cargo test -p deer-gui --lib interaction`）。
pub mod interaction;

/// 输入脚本（M5-4，**纯逻辑**）：`DEER_INPUT_SCRIPT` 那种字符串 → 一串
/// [`interaction::InputEvent`]，外加「脚本 → 状态」的纯逻辑重放。
///
/// 窗口侧（`examples/interactive_form.rs`）与测试侧共用这一份解析器 ⇒ 脚本语法只有一处定义。
pub mod input_script;

/// **环境变量门槛**判定（`DEER_VK_WINDOW_TESTS` 这类开关，**纯逻辑**）。
///
/// 门槛判据必须只有一处定义，且必须先 `trim()` 再比 —— `cmd` 的 `set X=1 && …` 会把
/// `&&` 前的空格算进变量值（实测 `"1 "`），严格判等会把「已启用」判成「未启用」
/// ⇒ **静默跳过却报 pass**（一整档验证证据因此失效）。完整实测说明与断言见模块文档。
pub mod env_gate;

/// 窗口层（**需要 `window` feature**）。
///
/// 这是本 workspace 唯一引入第三方依赖（`winit`）的地方，理由是窗口/事件循环的
/// 平台代码量与跨平台覆盖（见 `ROADMAP.md` 的 Q-1）。渲染层通过 HAL 的不透明
/// [`gpu::RawWindowHandle`] 拿原生句柄，**不依赖 winit** —— 所以换窗口实现不会动渲染栈。
#[cfg(feature = "window")]
pub use deer_window as window;

use std::path::Path;

use deer_layout::layout::{ApproxMeasure, TextStyle};

/// 常用类型的集中导入。
pub mod prelude {
    pub use crate::gpu::render::{DefaultRenderer, build_draw_list};
    pub use crate::gpu::null::{CpuRenderer, Framebuffer};
    pub use crate::gpu::{Color, DrawCmd, DrawList, Extent, RectI, Theme};
    pub use crate::gpu::atlas::GlyphAtlas;
    pub use crate::gpu::glyph::{GlyphImage, GlyphKey};
    pub use crate::gpu::measure::FontMeasure;
    pub use crate::gpu::raster::Rasterizer;
    pub use crate::gpu::text::{GlyphPlacement, TextEngine};
    pub use crate::layout::builder::{Builder, L};
    pub use crate::layout::layout::{ApproxMeasure, Measure, TextStyle, hit_test, layout, measure_tree};
    pub use crate::layout::node::{Align, Kind, Node, Rect, Size};
    pub use crate::layout::scene::{SceneError, encode_scene, parse_scene};
}

/// 把一棵树渲染成 RGBA8 像素（**离屏**，不涉及窗口）。
///
/// 返回 `(width, height, pixels)`；`pixels` 是行优先、无 padding 的 RGBA8。
pub fn render_tree_to_rgba(
    tree: &Node,
    width: u32,
    height: u32,
    theme: Theme,
) -> GpuResult<(u32, u32, Vec<u8>)> {
    let measure = ApproxMeasure;
    let geo = layout::layout::layout(
        tree,
        layout::node::Rect::new(0.0, 0.0, width as f32, height as f32),
        TextStyle {
            font_size: theme.font_size,
            line_height: theme.line_height,
        },
        &measure,
    );
    let list = gpu::build_draw_list(tree, &geo, theme.clone(), &measure);
    let fb = gpu::null::CpuRenderer::new().render(
        gpu::Extent { width, height },
        &list,
        theme.surface,
    )?;
    Ok((fb.width, fb.height, fb.pixels))
}

/// 把一棵树直接渲染成 PNG 字节流（便于 `std::fs::write` 落盘）。
pub fn render_tree_to_png(
    tree: &Node,
    width: u32,
    height: u32,
    theme: Theme,
) -> Result<Vec<u8>, String> {
    let (w, h, px) = render_tree_to_rgba(tree, width, height, theme.clone())
        .map_err(|e| format!("渲染失败：{e}"))?;
    gpu::png::encode_rgba(w, h, &px)
}

/// 把一棵树渲染成 RGBA8 像素，**用真实字体字形**（M4）。
///
/// 与 [`render_tree_to_rgba`] 的三点区别：
///
/// 1. 布局的文本度量用 [`gpu::FontMeasure`]（字体真实 advance），不是「每字符 0.6em」的近似；
/// 2. 文字画成**真实字形**（从字形图集采样覆盖率），不是等宽占位格；
/// 3. `font_size` **同时**决定引擎字号、布局的 `TextStyle.font_size` 与 `theme.font_size`
///    —— 三者必须一致，否则「布局算出来的宽度」与「画出来的宽度」会漂。
///
/// `font_path` 指向一个 TrueType 字体文件（含 `glyf` 表，即 `.ttf`/`.ttc`）。
/// **不支持 CFF/OTTO**（会明确报错，不静默给空白字形）。
pub fn render_tree_to_rgba_with_font(
    tree: &Node,
    width: u32,
    height: u32,
    theme: Theme,
    font_path: &Path,
    font_size: f32,
) -> GpuResult<(u32, u32, Vec<u8>)> {
    let engine = gpu::TextEngine::from_font_file(font_path, font_size)?;
    render_tree_to_rgba_with_engine(tree, width, height, theme, font_size, engine)
}

/// 同上，但由调用方给一个已经建好的 [`gpu::TextEngine`]
/// （例如想复用已解析的字体、或用 `TextEngine::from_system_font`）。
pub fn render_tree_to_rgba_with_engine(
    tree: &Node,
    width: u32,
    height: u32,
    mut theme: Theme,
    font_size: f32,
    engine: gpu::TextEngine,
) -> GpuResult<(u32, u32, Vec<u8>)> {
    // 字号一处定义：`theme.font_size` 就是绘制列表里 `DrawCmd::Text.size` 的来源，
    // 所以要把它钉到与引擎/度量同一个值上。
    theme.font_size = font_size;
    let style = TextStyle {
        font_size,
        line_height: theme.line_height,
    };

    let geo = layout::layout::layout(
        tree,
        layout::node::Rect::new(0.0, 0.0, width as f32, height as f32),
        style,
        &engine.measure(),
    );
    let list = gpu::build_draw_list(tree, &geo, theme.clone(), &engine.measure());

    let mut renderer = gpu::null::CpuRenderer::with_text(engine);
    let fb = renderer.render(gpu::Extent { width, height }, &list, theme.surface)?;
    Ok((fb.width, fb.height, fb.pixels))
}

/// 把一棵树用真实字体渲染成 PNG 字节流。
pub fn render_tree_to_png_with_font(
    tree: &Node,
    width: u32,
    height: u32,
    theme: Theme,
    font_path: &Path,
    font_size: f32,
) -> Result<Vec<u8>, String> {
    let (w, h, px) = render_tree_to_rgba_with_font(tree, width, height, theme.clone(), font_path, font_size)
        .map_err(|e| format!("渲染失败：{e}"))?;
    gpu::png::encode_rgba(w, h, &px)
}

/// 同时算出几何表（调试布局时有用）。
pub fn layout_tree(tree: &Node, width: u32, height: u32, theme: Theme) -> layout::layout::Geometry {
    layout::layout::layout(
        tree,
        layout::node::Rect::new(0.0, 0.0, width as f32, height as f32),
        TextStyle {
            font_size: theme.font_size,
            line_height: theme.line_height,
        },
        &ApproxMeasure,
    )
}
