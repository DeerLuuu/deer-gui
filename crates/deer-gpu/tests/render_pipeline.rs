//! 渲染链路测试：`树 + 几何 → DrawList → 像素`，以及默认渲染器的行为。
//!
//! 这一层的作用：把「渲染器」的语义钉死。后续 Vulkan 后端应当对同一份绘制列表
//! 给出与 CPU 后端**相同**的像素 —— 基准就在这里。

use deer_gpu::null::CpuRenderer;
use deer_gpu::render::{DefaultRenderer, NullRenderer, build_draw_list};
use deer_core::{ DrawCmd };
use deer_gpu::{ Extent, Theme };
use deer_core::builder::{Builder, L};
use deer_core::layout::{ApproxMeasure, TextStyle, layout};
use deer_core::node::{Kind, Rect};

fn theme() -> Theme {
    Theme::default()
}

fn style(t: &Theme) -> TextStyle {
    TextStyle {
        font_size: t.font_size,
        line_height: t.line_height,
    }
}

fn ui() -> deer_core::Node {
    let mut app = Builder::new(Kind::Column, "app").padding(12.0).gap(8.0);
    app.text("标题");
    app.container_opts(Kind::Row, "bar", L::new().gap(8.0).to_props(), |r| {
        r.button("确定");
        r.button_opts("禁用", |n| n.props.disabled = true);
    });
    app.build()
}

#[test]
fn every_visible_node_gets_a_draw_command() {
    let t = theme();
    let tree = ui();
    let geo = layout(&tree, Rect::new(0.0, 0.0, 320.0, 200.0), style(&t), &ApproxMeasure);
    let list = DefaultRenderer::new(t.clone(), &ApproxMeasure).build(&tree, &geo);
    assert!(list.clip_balanced(), "渲染器产出的裁剪栈必须平衡");

    let c = list.counts();
    // 2 个文字（标题 + 行内无文字）+ 2 个按钮 ⇒ 至少 2 个 Text 与 2 个按钮底色
    assert!(c.text >= 2, "标题与按钮文字都该产生 Text 命令，实际 {}", c.text);
    assert!(
        c.fill_round_rect >= 3,
        "根容器 + 2 个按钮应产生圆角填充，实际 {}",
        c.fill_round_rect
    );
}

#[test]
fn disabled_button_uses_dim_color() {
    let t = theme();
    let tree = ui();
    let geo = layout(&tree, Rect::new(0.0, 0.0, 320.0, 200.0), style(&t), &ApproxMeasure);
    let list = DefaultRenderer::new(t.clone(), &ApproxMeasure).build(&tree, &geo);

    // 禁用按钮的底色应是 border 色，而不是 accent
    let disabled_rect = geo["button_2"];
    let found = list.cmds.iter().any(|c| match c {
        DrawCmd::FillRoundRect { rect, color, .. } => {
            rect.x == disabled_rect.x as i32 && *color == t.border
        }
        _ => false,
    });
    assert!(found, "禁用按钮必须用 border 色填充（不能用 accent）");

    let text_dim = list.cmds.iter().any(|c| match c {
        DrawCmd::Text { text, color, .. } => text == "禁用" && *color == t.text_dim,
        _ => false,
    });
    assert!(text_dim, "禁用按钮的文字必须用 text_dim 色");
}

#[test]
fn container_without_padding_draws_no_box() {
    // 只有显式内边距的容器才画底 —— 否则整屏都是方块，看不出层次
    let t = theme();
    let mut app = Builder::new(Kind::Column, "app");
    app.container_auto(Kind::Row, |r| {
        r.button("A");
    });
    let tree = app.build();
    let geo = layout(&tree, Rect::new(0.0, 0.0, 100.0, 60.0), style(&t), &ApproxMeasure);
    let list = DefaultRenderer::new(t, &ApproxMeasure).build(&tree, &geo);
    let boxes = list
        .cmds
        .iter()
        .filter(|c| matches!(c, DrawCmd::FillRoundRect { .. }))
        .count();
    assert_eq!(boxes, 1, "无内边距的容器不画底 ⇒ 只剩按钮那一个圆角填充");
}

#[test]
fn button_text_is_centered_by_the_same_measure_as_layout() {
    let t = theme();
    let tree = ui();
    let geo = layout(&tree, Rect::new(0.0, 0.0, 320.0, 200.0), style(&t), &ApproxMeasure);
    let list = DefaultRenderer::new(t.clone(), &ApproxMeasure).build(&tree, &geo);

    let btn = geo["button_1"];
    let tw = deer_core::Measure::width(&ApproxMeasure, "确定", style(&t));
    let expected_x = btn.x + ((btn.w - tw) / 2.0).floor();
    let found = list.cmds.iter().any(|c| match c {
        DrawCmd::Text { rect, text, .. } if text == "确定" => rect.x as f32 == expected_x,
        _ => false,
    });
    assert!(found, "按钮文字必须按布局度量居中（期望 x={expected_x}）");
}

#[test]
fn null_renderer_emits_one_hint_per_geometry_entry() {
    let t = theme();
    let tree = ui();
    let geo = layout(&tree, Rect::new(0.0, 0.0, 320.0, 200.0), style(&t), &ApproxMeasure);
    let list = NullRenderer::build(&tree, &geo);
    assert_eq!(
        list.counts().node_hint,
        geo.len(),
        "NullRenderer 每个有几何的节点出一条 hint"
    );
}

#[test]
fn end_to_end_pixels_are_not_a_flat_fill() {
    // 端到端：树 → 几何 → 绘制列表 → 像素。画面必须有多种颜色（真的画上了东西）。
    let t = theme();
    let tree = ui();
    let geo = layout(&tree, Rect::new(0.0, 0.0, 320.0, 200.0), style(&t), &ApproxMeasure);
    let list = build_draw_list(&tree, &geo, t.clone(), &ApproxMeasure);
    let fb = CpuRenderer::new()
        .render(
            Extent {
                width: 320,
                height: 200,
            },
            &list,
            t.surface,
        )
        .expect("渲染");

    let mut set = std::collections::BTreeSet::new();
    for p in fb.pixels.chunks_exact(4) {
        set.insert([p[0], p[1], p[2], p[3]]);
    }
    assert!(set.len() > 3, "画面颜色种类太少（{}）⇒ 很可能没画上", set.len());
    // 强调色必须真的出现在像素里
    let accent_px = fb.count_color(t.accent);
    assert!(accent_px > 50, "强调色像素太少（{accent_px}）⇒ 按钮没画出来");
    assert_eq!(fb.to_rgba().len(), 320 * 200 * 4);
}

#[test]
fn render_is_deterministic_end_to_end() {
    let t = theme();
    let tree = ui();
    let a = deer_gui_render_once(&tree, &t);
    let b = deer_gui_render_once(&tree, &t);
    assert!(a.bytes_eq(&b), "两次端到端渲染必须逐字节相同");
}

fn deer_gui_render_once(tree: &deer_core::Node, t: &Theme) -> deer_gpu::null::Framebuffer {
    let geo = layout(tree, Rect::new(0.0, 0.0, 320.0, 200.0), style(t), &ApproxMeasure);
    let list = build_draw_list(tree, &geo, t.clone(), &ApproxMeasure);
    CpuRenderer::new()
        .render(
            Extent {
                width: 320,
                height: 200,
            },
            &list,
            t.surface,
        )
        .expect("渲染")
}
