//! **真实字形的像素级验收**（M4）。
//!
//! 这一组测试要证明的不是「函数被调用了」，而是**屏幕上画出来的东西真的来自字形**：
//!
//! 1. 有字库与无字库（占位格）画出的画面**必须不同**，且真实字形有抗锯齿灰阶（占位格只有两种颜色）；
//! 2. 同一份绘制列表重复渲染**逐字节相同**（图集缓存填充不能改变输出）；
//! 3. 形状性质：`l` 的墨迹比 advance 窄、`o` 的墨迹包围盒里**有洞**、`g` 的墨迹比 `x` 更靠下（下伸部）、
//!    空格不画墨但推进笔位置。
//!
//! 字体来源：系统字体（`consola` → `arial` → `segoeui`）。**找不到就明确跳过并打印原因**，
//! 不伪装通过 —— 这些断言针对的是任意正常字体都成立的性质，不是某个字体的位图指纹。
//! 机器无关的确定性断言在 `tests/text_raster.rs`（手工构造轮廓）。

use deer_core::draw::{Color, DrawCmd, DrawList, RectI};
use deer_gpu::null::{CpuRenderer, Framebuffer};
use deer_gpu::text::TextEngine;
use deer_gpu::Extent;

const BG: Color = Color::rgb(20, 22, 32);
const FG: Color = Color::rgb(230, 232, 239);

/// 取系统字体；没有就返回 `None`（调用方打印原因并跳过）。
fn engine(font_size: f32) -> Option<TextEngine> {
    match TextEngine::from_system_font(font_size) {
        Ok(e) => Some(e),
        Err(e) => {
            eprintln!("跳过：这台机器上拿不到系统字体（{e}）");
            None
        }
    }
}

fn text_list(text: &str, rect: RectI, size: f32) -> DrawList {
    let mut list = DrawList::new();
    list.push(DrawCmd::Text {
        rect,
        text: text.to_string(),
        color: FG,
        size,
        align: 0,
    });
    list
}

fn render(engine: TextEngine, list: &DrawList, w: u32, h: u32) -> Framebuffer {
    let mut r = CpuRenderer::with_text(engine);
    r.render(Extent { width: w, height: h }, list, BG).expect("渲染必须成功")
}

/// 墨迹像素的包围盒 `(x0, y0, x1, y1)` 与数量（背景色之外、且与文字色不完全相同的都算「被碰过」）。
fn ink_bbox(fb: &Framebuffer) -> Option<(i32, i32, i32, i32, usize)> {
    let mut bbox: Option<(i32, i32, i32, i32)> = None;
    let mut count = 0usize;
    for y in 0..fb.height as i32 {
        for x in 0..fb.width as i32 {
            let p = fb.pixel(x, y).expect("坐标在范围内");
            let is_bg = p[0] == BG.r && p[1] == BG.g && p[2] == BG.b;
            if is_bg {
                continue;
            }
            count += 1;
            bbox = Some(match bbox {
                None => (x, y, x, y),
                Some((x0, y0, x1, y1)) => (x0.min(x), y0.min(y), x1.max(x), y1.max(y)),
            });
        }
    }
    bbox.map(|(x0, y0, x1, y1)| (x0, y0, x1, y1, count))
}

/// 画面里出现过的不同像素值个数（抗锯齿灰阶数）。
fn distinct_pixels(fb: &Framebuffer) -> usize {
    let mut set = std::collections::BTreeSet::new();
    for px in fb.pixels.chunks_exact(4) {
        set.insert((px[0], px[1], px[2], px[3]));
    }
    set.len()
}

#[test]
fn real_glyphs_differ_from_placeholder_boxes() {
    let Some(e) = engine(24.0) else { return };
    let list = text_list("Hamburgefonstiv", RectI::new(2, 2, 200, 30), 24.0);

    let real = render(e, &list, 220, 36);
    let mut plain_renderer = CpuRenderer::new();
    let plain = plain_renderer
        .render(Extent { width: 220, height: 36 }, &list, BG)
        .expect("占位渲染必须成功");

    assert!(
        !real.bytes_eq(&plain),
        "有字库与无字库画出的画面必须不同（否则说明真实字形路径根本没生效）"
    );
    let real_levels = distinct_pixels(&real);
    let plain_levels = distinct_pixels(&plain);
    assert!(
        real_levels > 4,
        "真实字形必须有抗锯齿灰阶（>4 种像素值），实际 {real_levels}"
    );
    assert!(
        plain_levels <= 3,
        "占位格路径只有背景+文字两种颜色（允许极少数边界），实际 {plain_levels}"
    );
    assert!(ink_bbox(&real).is_some(), "真实字形必须画出墨迹");
}

#[test]
fn rendering_with_text_is_deterministic() {
    let Some(e) = engine(20.0) else { return };
    let list = text_list("determinism 0123", RectI::new(1, 1, 260, 24), 20.0);

    let mut r = CpuRenderer::with_text(e);
    let a = r
        .render(Extent { width: 280, height: 28 }, &list, BG)
        .expect("第一次渲染");
    let b = r
        .render(Extent { width: 280, height: 28 }, &list, BG)
        .expect("第二次渲染");
    assert!(
        a.bytes_eq(&b),
        "带字库重复渲染必须逐字节相同（图集缓存填充不得改变输出）"
    );
}

#[test]
fn narrow_glyph_ink_is_narrower_than_its_advance() {
    let Some(mut probe) = engine(32.0) else { return };
    let advance = probe.glyph('l', 32.0).expect("'l' 应该有字形").advance;
    assert!(advance > 4.0, "advance 应当合理，实际 {advance}");
    drop(probe);

    let ink = |ch: &str| -> (f32, usize) {
        let Some(e) = engine(32.0) else { panic!("字体在第一次调用时还在") };
        let (x0, _, x1, _, count) =
            ink_bbox(&render(e, &text_list(ch, RectI::new(0, 0, 60, 44), 32.0), 64, 48))
                .unwrap_or_else(|| panic!("'{ch}' 必须画出墨迹"));
        ((x1 - x0 + 1) as f32, count)
    };

    let (i_w, i_px) = ink("i");
    let (l_w, l_px) = ink("l");
    let (m_w, m_px) = ink("m");

    // ① 墨迹必须窄于字格（advance）：等宽字体下 'l' 也可能带衬线，所以这里只要求「不超格」
    assert!(
        l_w <= advance,
        "'l' 的墨迹宽度 {l_w} 不该超过 advance {advance}"
    );
    // ② 墨迹必须**随字形变化** —— 占位格下不同字符的墨迹面积会完全相等。
    //    面积比宽度更稳：等宽字体的字符宽度接近，但 `m`（三根竖笔）明显比 `l`（一根）重。
    assert!(
        i_px < m_px,
        "窄字形 'i' 的墨迹像素（{i_px}）必须少于宽字形 'm'（{m_px}）—— 真实字形才会这样"
    );
    assert!(
        l_px < m_px,
        "窄字形 'l' 的墨迹像素（{l_px}）必须少于宽字形 'm'（{m_px}）—— 真实字形才会这样"
    );
    println!("ink: i={i_w}px/{i_px} l={l_w}px/{l_px} m={m_w}px/{m_px} advance={advance}");
}

#[test]
fn glyph_with_a_counter_keeps_its_hole() {
    let Some(e) = engine(40.0) else { return };
    let fb = render(e, &text_list("o", RectI::new(0, 0, 80, 56), 40.0), 84, 60);
    let (x0, y0, x1, y1, _) = ink_bbox(&fb).expect("'o' 必须画出墨迹");

    // 墨迹包围盒的几何中心必须**没有墨**（'o' 的内圈是空的）
    let cx = (x0 + x1) / 2;
    let cy = (y0 + y1) / 2;
    let c = fb.pixel(cx, cy).expect("中心点在范围内");
    assert_eq!(
        (c[0], c[1], c[2]),
        (BG.r, BG.g, BG.b),
        "'o' 的中心 ({cx},{cy}) 必须是背景色（洞被填上说明填充规则错了）"
    );
    assert!(
        (x1 - x0) > 8 && (y1 - y0) > 8,
        "'o' 的墨迹包围盒不该退化成一条线：{x0},{y0}..{x1},{y1}"
    );
}

#[test]
fn descender_reaches_below_a_glyph_without_one() {
    // 这条断言**不看裁剪结果**：画布够高（120px），两个字形都不会被边界截断，
    // 并且用「基线」这个绝对几何量做判据 —— 否则把 top 的符号改反、两个字形一起下移时，
    // 「谁更低」的相对顺序仍然成立，断言就抓不到了。
    let size = 40.0f32;
    let rect = RectI::new(0, 0, 80, 56);
    let Some(probe) = engine(size) else { return };
    let (ascent, descent) = {
        let m = probe.measure();
        (m.ascent(), m.descent())
    };
    // 与文档/实现一致的基线规则：把「升部 + 降部」这块在 rect 里垂直居中
    let baseline =
        (rect.y as f32 + ((rect.h as f32 - (ascent + descent)) / 2.0).round() + ascent.round()).round() as i32;
    drop(probe);

    let ink_bottom = |ch: &str| -> (i32, i32) {
        let Some(e) = engine(size) else { panic!("字体在第一次调用时还在") };
        let (_, y0, _, y1, count) =
            ink_bbox(&render(e, &text_list(ch, rect, size), 100, 120)).unwrap_or_else(|| panic!("'{ch}' 必须有墨"));
        assert!(count > 0);
        (y0, y1)
    };
    let (_, x_bottom) = ink_bottom("x");
    let (_, g_bottom) = ink_bottom("g");

    assert!(
        (x_bottom - baseline).abs() <= 2,
        "'x' 没有下伸部，墨迹底行 {x_bottom} 必须贴着基线 {baseline}（差太远说明 top 的符号或基线算法错了）"
    );
    assert!(
        g_bottom > baseline + 2,
        "'g' 有下伸部，墨迹底行 {g_bottom} 必须明显越过基线 {baseline}"
    );
}

#[test]
fn ink_x_position_is_pen_plus_left() {
    // 水平偏移的**符号**判据：墨迹左边缘必须落在 `pen + left`，不是 `pen - left`。
    // （反例：把 `draw_text_real` 里的 `+ p.left` 改成 `- p.left`，这条必须变红。）
    let size = 40.0f32;
    let pen_x = 20;
    let Some(mut e) = engine(size) else { return };
    let p = e.glyph('.', size).expect("'.' 应该有字形");
    assert!(
        p.left != 0,
        "需要挑一个 left != 0 的字形来做符号判据，实际 left={}",
        p.left
    );
    let (x0, _, _, _, _) = ink_bbox(&render(
        e,
        &text_list(".", RectI::new(pen_x, 0, 80, 56), size),
        120,
        60,
    ))
    .expect("'.' 必须画出墨迹");
    assert_eq!(
        x0,
        pen_x + p.left,
        "墨迹左边缘应是 pen({pen_x}) + left({}) = {}；若看到 {} 说明符号反了",
        p.left,
        pen_x + p.left,
        pen_x - p.left
    );
}

#[test]
fn space_advances_the_pen_without_ink() {
    let Some(mut e) = engine(28.0) else { return };
    let width_with_space = e.text_width("l l", 28.0);
    let width_without_space = e.text_width("ll", 28.0);
    assert!(
        width_with_space > width_without_space,
        "空格必须推进笔位置（{width_with_space} vs {width_without_space}）"
    );

    let space = render(e, &text_list(" ", RectI::new(0, 0, 40, 36), 28.0), 44, 40);
    assert!(
        ink_bbox(&space).is_none(),
        "空格不能画出任何墨迹（占位实现会画出方块，这正是要区分的）"
    );
}

#[test]
fn buttons_center_labels_using_real_metrics() {
    // 真实度量的直接后果：按钮文字居中时，「布局算的宽度」与「画出来的宽度」必须一致。
    let Some(mut e) = engine(16.0) else { return };
    let label = "Apply";
    let w = e.text_width(label, 16.0);
    let font_w = e.measure().text_width(label);
    assert!(
        (w - font_w).abs() <= 1.0,
        "TextEngine::text_width({w}) 与 FontMeasure::text_width({font_w}) 必须一致（同一个字体、同一套 advance）"
    );
    assert!(e.missing_glyphs() == 0, "ASCII 标签不该有缺字");
}

#[test]
fn missing_glyph_falls_back_to_notdef_and_keeps_width_consistent() {
    // consola 不含 CJK ⇒ 必然走 .notdef 回退。这条测试把「诚实边界」钉成断言：
    // 缺字**画豆腐块**（不是静默不画），且布局宽度与落笔宽度仍然一致。
    let Some(mut e) = engine(24.0) else { return };
    let p = e.glyph('中', 24.0).expect("缺字必须回退到 .notdef，而不是不画");
    assert!(p.advance > 0.0, "回退字形也要有 advance，实际 {}", p.advance);
    assert!(e.missing_glyphs() >= 1, "缺字必须计入 missing_glyphs() 诊断");

    let drawn = e.text_width("中", 24.0);
    let layout_w = e.measure().text_width("中");
    assert!(
        (drawn - layout_w).abs() <= 1.0,
        "落笔宽度 {drawn} 与布局宽度 {layout_w} 必须一致（只允许 ≤1px 的取整差）"
    );

    let fb = render(e, &text_list("中", RectI::new(0, 0, 60, 40), 24.0), 64, 44);
    assert!(
        ink_bbox(&fb).is_some(),
        "缺字必须画出豆腐块（可见的失败信号），而不是什么都不画"
    );
}
