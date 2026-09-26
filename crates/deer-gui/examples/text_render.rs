//! 功能示例：**真实文字渲染**（M4）—— 把节点树里的文字用真实字体字形画进像素。
//!
//! ```sh
//! cargo run -p deer-gui --example text_render
//! ```
//!
//! 产物：`render_out/text_render.png`（外加同目录 `text_render_atlas.png`，字形图集本身）。
//!
//! 这一条链是：`TextEngine`（字体 + 光栅化 + 图集）→ 布局用 `FontMeasure` 度量 →
//! `CpuRenderer::with_text` 从图集采样覆盖率 → 像素。
//!
//! 文件末尾是**自检断言**：墨迹像素、抗锯齿灰阶、逐帧确定性，以及
//! 「带字库与不带字库必须画出不同画面」——避免「跑成功但画的是方块」。

use std::path::Path;

use deer_gui::gpu::null::CpuRenderer;
use deer_gui::gpu::text::TextEngine;
use deer_gui::gpu::{self, Extent, Theme};
use deer_gui::layout::builder::{Builder, L};
use deer_gui::layout::layout;
use deer_gui::layout::layout::TextStyle;
use deer_gui::layout::node::{Kind, Rect};

const W: u32 = 460;
const H: u32 = 190;

fn main() -> Result<(), String> {
    // ① 拿字体。**找不到就明确失败** —— 示例是给人看的，不许伪装成功。
    let font_path = deer_gui::gpu::measure::find_system_font()
        .ok_or_else(|| "找不到系统字体（consola.ttf / arial.ttf / segoeui.ttf），本示例需要真实字体".to_string())?;
    println!("字体：{}", font_path.display());

    // ② 建一棵普通的界面树：标题 + 一行按钮 + 一段正文
    let mut app = Builder::new(Kind::Column, "app").padding(16.0).gap(10.0);
    app.text("deer-gui: real glyphs");
    app.container_opts(Kind::Row, "bar", L::new().gap(8.0).to_props(), |r| {
        r.button("Apply");
        r.button("Cancel");
        r.field("field");
    });
    app.text("The quick brown fox jumps over the lazy dog.");
    app.text("0123456789 +-*/=()[]{} #@!?");
    let tree = app.build();

    // ③ 字号一处定义：引擎、主题、布局三处都用它（这是本轮最重要的一致性要求）
    let font_size = 16.0f32;
    let engine = TextEngine::from_font_file(Path::new(&font_path), font_size)
        .map_err(|e| format!("解析字体失败：{e}"))?;
    let theme = Theme {
        font_size,
        ..Theme::default()
    };
    let style = TextStyle {
        font_size,
        line_height: theme.line_height,
    };

    // ④ 布局与绘制列表都用**引擎的度量**（不是 ApproxMeasure）
    let geo = layout::layout(
        &tree,
        Rect::new(0.0, 0.0, W as f32, H as f32),
        style,
        &engine.measure(),
    );
    let list = gpu::build_draw_list(&tree, &geo, theme.clone(), &engine.measure());
    println!("绘制命令 {} 条（文字 {} 条）", list.len(), list.counts().text);

    // ⑤ 渲染：带字库 ⇒ 真实字形
    let mut renderer = CpuRenderer::with_text(engine);
    let fb = renderer
        .render(Extent { width: W, height: H }, &list, theme.surface)
        .map_err(|e| format!("渲染失败：{e}"))?;

    // ⑥ 没有字库时画出来的是什么（下面用来证明「真实字形真的生效」）
    let mut plain_renderer = CpuRenderer::new();
    let fb_plain = plain_renderer
        .render(Extent { width: W, height: H }, &list, theme.surface)
        .map_err(|e| format!("占位渲染失败：{e}"))?;

    // ⑦ 第二次渲染必须逐字节相同
    let fb2 = renderer
        .render(Extent { width: W, height: H }, &list, theme.surface)
        .map_err(|e| format!("第二次渲染失败：{e}"))?;

    // ⑧ 写产物
    std::fs::create_dir_all("render_out").map_err(|e| format!("建不了 render_out：{e}"))?;
    let png = gpu::png::encode_rgba(W, H, fb.to_rgba())?;
    std::fs::write("render_out/text_render.png", &png).map_err(|e| format!("写 PNG 失败：{e}"))?;
    if let Some(engine) = renderer.text() {
        let atlas_png = engine.atlas_png()?;
        std::fs::write("render_out/text_render_atlas.png", &atlas_png)
            .map_err(|e| format!("写图集 PNG 失败：{e}"))?;
    }

    // ⑨ 统计与自检
    let ink = count_ink(&fb, theme.surface);
    let levels = distinct_levels(&fb);
    let (glyphs, missing, atlas_size) = match renderer.text() {
        Some(e) => (e.rasterized_glyphs(), e.missing_glyphs(), e.atlas().size()),
        None => unreachable!("我们刚用 with_text 建的渲染器"),
    };
    println!(
        "光栅化字形 {glyphs} 个 / 缺字 {missing} / 图集 {}×{} / 墨迹像素 {ink} / 灰阶 {levels}",
        atlas_size.0, atlas_size.1
    );
    println!("产物：render_out/text_render.png（{} 字节）、render_out/text_render_atlas.png", png.len());

    assert!(ink > 500, "文字必须画出足够多的墨迹像素，实际 {ink}");
    assert!(levels > 8, "真实字形必须有抗锯齿灰阶（>8 种像素值），实际 {levels}");
    assert!(
        !fb.bytes_eq(&fb_plain),
        "带字库与不带字库必须画出**不同**的画面，否则说明真实字形路径没生效"
    );
    assert!(fb.bytes_eq(&fb2), "同一份绘制列表重复渲染必须逐字节相同");
    assert_eq!(missing, 0, "这段文本全是 ASCII，不该有缺字");
    assert!(glyphs > 20, "这段文本至少涉及 20 多个不同字形，实际 {glyphs}");

    println!("\n自检全部通过 ✅（真实字形、抗锯齿、确定性、与占位路径可区分）");
    Ok(())
}

/// 背景色之外的像素数（「被画过」的像素）。
fn count_ink(fb: &gpu::null::Framebuffer, bg: gpu::Color) -> usize {
    fb.pixels
        .chunks_exact(4)
        .filter(|p| !(p[0] == bg.r && p[1] == bg.g && p[2] == bg.b))
        .count()
}

/// 画面里出现过的不同像素值个数（抗锯齿灰阶数）。
fn distinct_levels(fb: &gpu::null::Framebuffer) -> usize {
    let mut set = std::collections::BTreeSet::new();
    for p in fb.pixels.chunks_exact(4) {
        set.insert((p[0], p[1], p[2], p[3]));
    }
    set.len()
}
