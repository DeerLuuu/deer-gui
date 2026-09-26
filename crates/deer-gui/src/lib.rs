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
//! | **渲染到窗口 / 屏幕上** | ❌ 里程碑 M2–M3 |
//! | 真实字形排版（现在是等宽格占位） | ❌ 里程碑 M4 |
//! | 输入事件与焦点 | ❌ 里程碑 M5 |
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

use deer_layout::layout::{ApproxMeasure, TextStyle};

/// 常用类型的集中导入。
pub mod prelude {
    pub use crate::gpu::render::{DefaultRenderer, build_draw_list};
    pub use crate::gpu::null::{CpuRenderer, Framebuffer};
    pub use crate::gpu::{Color, DrawCmd, DrawList, Extent, RectI, Theme};
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
