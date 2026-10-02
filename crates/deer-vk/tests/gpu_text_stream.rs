//! M3b-T3：**文本顶点流**的纯逻辑验收（**不需要 GPU、不需要着色器**）。
//!
//! ## 这一层在守什么
//!
//! `gpu_text::build_text_stream` 把 `DrawCmd::Text` 翻译成「每个字形一个四边形」的顶点流，
//! 而它的正确性最终由 **Task 4 的逐像素对照**（GPU ↔ CPU）来证明。在那之前，这里用三种
//! **互相独立**的手段把几何/映射这一层钉住：
//!
//! 1. **精确布局断言**：按 CPU `null.rs::draw_text_real` 的同一套公式（`pen.round()`、
//!    基线 `rect.y + ((h-(asc+desc))/2).round() + ascent.round()`、逐字形 advance）
//!    重推期望的 `pos`/`uv`，逐顶点比 —— 单字符、多字符、`align = 0/1/2`；
//! 2. **CPU 后端当独立参照物**：同一份绘制列表交给 `CpuRenderer::with_text`，
//!    断言「CPU 写了颜色的像素」与「顶点流覆盖到的、且映射纹素覆盖率 > 0 的像素」
//!    **逐像素完全一致**（两个方向都查 ⇒ 既不漏也不多）；
//! 3. **最近邻采样的纹素对应性（性质测试）**：对每个四边形覆盖的每个像素，
//!    按 `uv` 线性插值到像素中心再 `floor(u * 图集宽)`，必须正好落在
//!    「CPU 给这个像素取的那个纹素」上 —— 这是「GPU 与 CPU 采样同一份覆盖率」的充分条件。
//!
//! ## 与 CPU 一致的边界语义（M3a defer 的假阳性修复）
//!
//! 空串 / `size <= 0` / 与 clip 求交后无可见字形 ⇒ **跳过并计入 `skipped`，不报错**
//! （M3a 里是无条件报 `Unsupported`，会拒收 CPU 能正常出图的帧）。
//!
//! 字体来源：系统字体（`consola` → `arial` → `segoeui`）；**拿不到就明确跳过并打印原因**，
//! 不伪装通过。断言针对的是「任意正常字体都成立的性质」，不是某个字体的位图指纹。

use deer_core::draw::{Color, DrawCmd, DrawList, RectI};
use deer_gpu::null::CpuRenderer;
use deer_gpu::text::{GlyphPlacement, TextEngine};
use deer_gpu::{ AtlasSlot, Extent };
use deer_vk::gpu_text::{self, TextVertex};

/// 前景 / 背景：用**不透明**色，让「CPU 有没有写这个像素」变成「是否等于背景色」。
const FG: Color = Color::rgb(255, 255, 255);
const BG: Color = Color::rgb(0, 0, 0);

fn engine(font_size: f32) -> Option<TextEngine> {
    match TextEngine::from_system_font(font_size) {
        Ok(e) => Some(e),
        Err(e) => {
            eprintln!("跳过：这台机器上拿不到系统字体（{e}）");
            None
        }
    }
}

fn text_list(text: &str, rect: RectI, size: f32, align: u8) -> DrawList {
    let mut l = DrawList::new();
    l.push(DrawCmd::Text {
        rect,
        text: text.to_string(),
        color: FG,
        size,
        align,
    });
    l
}

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-6
}

/// 与 CPU `null.rs::draw_text_real` **同一套公式**的测试侧重推：
/// 返回每个有墨迹字形的「位图左上角像素 `(gx0, gy0)` + 图集槽位」，顺序与绘制顺序一致。
///
/// 这是**独立的**一次重推（不是在实现里调一遍实现）：它照抄 CPU 参考实现的算法，
/// 所以能抓住「实现把 `pen.round()` 写成 `pen as i32`」这类偏差。
fn cpu_layout(
    e: &mut TextEngine,
    text: &str,
    rect: RectI,
    size: f32,
    align: u8,
) -> Vec<(i32, i32, AtlasSlot)> {
    let placements: Vec<Option<GlyphPlacement>> = text.chars().map(|c| e.glyph(c, size)).collect();
    let total: f32 = placements.iter().flatten().map(|p| p.advance).sum();
    let start_x = match align {
        1 => rect.x as f32 + (rect.w as f32 - total) / 2.0,
        2 => rect.right() as f32 - total,
        _ => rect.x as f32,
    };
    let (ascent, descent) = {
        let m = e.measure();
        (m.ascent(), m.descent())
    };
    let top_of_text_block = rect.y as f32 + ((rect.h as f32 - (ascent + descent)) / 2.0).round();
    let baseline = (top_of_text_block + ascent.round()).round();

    let mut pen = start_x;
    let mut out = Vec::new();
    for p in placements.into_iter().flatten() {
        if p.slot.w > 0 && p.slot.h > 0 {
            out.push((pen.round() as i32 + p.left, baseline as i32 - p.top, p.slot));
        }
        pen += p.advance;
    }
    out
}

fn ndc_x(px: i32, w: u32) -> f32 {
    2.0 * px as f32 / w as f32 - 1.0
}
fn ndc_y(py: i32, h: u32) -> f32 {
    2.0 * py as f32 / h as f32 - 1.0
}
/// `pos`（NDC）→ 像素（反推，用于性质测试）。
fn px_of(ndc: f32, n: u32) -> i32 {
    ((ndc + 1.0) * n as f32 / 2.0).round() as i32
}

/// 顶点布局是给 Task 4 的 `VkVertexInputAttributeDescription` 的硬契约：
/// `stride = 32`（`pos` 8 + `uv` 8 + `color` 16），偏移 0 / 8 / 16。
#[test]
fn text_vertex_layout_is_stable() {
    assert_eq!(std::mem::size_of::<TextVertex>(), 32, "stride = 8 + 8 + 16");
    assert_eq!(std::mem::align_of::<TextVertex>(), 4, "全是 f32 ⇒ 不需要补齐");
    assert_eq!(std::mem::offset_of!(TextVertex, pos), 0, "location 0：vec2 pos");
    assert_eq!(std::mem::offset_of!(TextVertex, uv), 8, "location 1：vec2 uv");
    assert_eq!(std::mem::offset_of!(TextVertex, color), 16, "location 2：vec4 color");
}

/// 单字符：四边形 = 6 个顶点，`pos`/`uv` 与 CPU 公式推出的期望值**逐个精确相等**。
#[test]
fn single_char_quad_matches_the_cpu_layout_formula() {
    let Some(mut e) = engine(24.0) else { return };
    let extent = Extent {
        width: 64,
        height: 32,
    };
    let rect = RectI::new(4, 4, 40, 20);
    let size = 24.0;

    // 先把期望值算出来（会顺带把字形光栅化入图集 —— 与实现拿到的缓存一致）
    let quads = cpu_layout(&mut e, "A", rect, size, 0);
    assert_eq!(quads.len(), 1, "单个有墨迹字形 ⇒ 一个四边形");
    let (gx0, gy0, slot) = quads[0];
    let (aw, ah) = e.atlas().size();

    let list = text_list("A", rect, size, 0);
    let s = gpu_text::build_text_stream(&list, extent, &mut e);

    assert_eq!(s.vertices.len(), 6, "一个字形 = 两个三角形");
    assert_eq!(s.skipped, 0, "正常绘制不该被计入 skipped");

    // 与实现同一套裁剪：初值 = 整幅画布
    let (cx0, cy0) = (gx0.max(0), gy0.max(0));
    let (cx1, cy1) = (
        (gx0 + slot.w as i32).min(extent.width as i32),
        (gy0 + slot.h as i32).min(extent.height as i32),
    );
    assert!(cx1 > cx0 && cy1 > cy0, "这个用例的字形应当有可见部分");

    let u = |px: i32| (slot.x as f32 + (px - gx0) as f32) / aw as f32;
    let v = |py: i32| (slot.y as f32 + (py - gy0) as f32) / ah as f32;
    let expect = [
        (ndc_x(cx0, extent.width), ndc_y(cy0, extent.height), u(cx0), v(cy0)),
        (ndc_x(cx1, extent.width), ndc_y(cy0, extent.height), u(cx1), v(cy0)),
        (ndc_x(cx1, extent.width), ndc_y(cy1, extent.height), u(cx1), v(cy1)),
        (ndc_x(cx0, extent.width), ndc_y(cy0, extent.height), u(cx0), v(cy0)),
        (ndc_x(cx1, extent.width), ndc_y(cy1, extent.height), u(cx1), v(cy1)),
        (ndc_x(cx0, extent.width), ndc_y(cy1, extent.height), u(cx0), v(cy1)),
    ];
    for (i, (ex, ey, eu, ev)) in expect.iter().enumerate() {
        let got = s.vertices[i];
        assert!(
            close(got.pos[0], *ex) && close(got.pos[1], *ey),
            "顶点 {i} 的 pos 应为 ({ex}, {ey})，实际 {:?}",
            got.pos
        );
        assert!(
            close(got.uv[0], *eu) && close(got.uv[1], *ev),
            "顶点 {i} 的 uv 应为 ({eu}, {ev})，实际 {:?}",
            got.uv
        );
        assert_eq!(got.color, [1.0, 1.0, 1.0, 1.0], "不透明白色");
    }
}

/// `align = 1`（居中）与 `align = 2`（右对齐）：起点按 CPU 公式平移。
///
/// **这两个分支此前全仓库没有测试**（只有 `align = 0` 被用过）—— 本条是它们的第一份覆盖。
#[test]
fn align_1_and_2_shift_the_start_x() {
    let Some(mut e) = engine(20.0) else { return };
    let extent = Extent {
        width: 128,
        height: 40,
    };
    let rect = RectI::new(10, 5, 100, 24);
    let size = 20.0;

    let lefts: Vec<i32> = (0..3)
        .map(|align| {
            let quads = cpu_layout(&mut e, "Align", rect, size, align);
            assert_eq!(quads.len(), 5, "5 个字母都有墨迹");
            quads[0].0 // 第一个字形的位图左边缘像素
        })
        .collect();

    // CPU 公式的直接推论：居中在左对齐右侧、右对齐又在居中的右侧
    assert!(lefts[0] < lefts[1], "align=1（居中）应当比 align=0（左）更靠右：{lefts:?}");
    assert!(lefts[1] < lefts[2], "align=2（右）应当比 align=1 更靠右：{lefts:?}");

    // 三种对齐的起点都必须**精确等于** CPU 公式（不是「大致靠右」）
    let total: f32 = {
        let ps: Vec<Option<GlyphPlacement>> = "Align".chars().map(|c| e.glyph(c, size)).collect();
        ps.iter().flatten().map(|p| p.advance).sum()
    };
    let p0 = e.glyph('A', size).expect("字形应当能进图集");
    assert_eq!(lefts[0], rect.x + p0.left, "align=0：起点就是 rect.x");
    let start1 = rect.x as f32 + (rect.w as f32 - total) / 2.0;
    assert_eq!(
        lefts[1],
        start1.round() as i32 + p0.left,
        "align=1：起点 = rect.x + (w - total)/2"
    );
    let start2 = rect.right() as f32 - total;
    assert_eq!(
        lefts[2],
        start2.round() as i32 + p0.left,
        "align=2：起点 = rect.right() - total"
    );

    for align in 0..3u8 {
        let quads = cpu_layout(&mut e, "Align", rect, size, align);
        let list = text_list("Align", rect, size, align);
        let s = gpu_text::build_text_stream(&list, extent, &mut e);
        assert_eq!(s.vertices.len(), 6 * quads.len(), "align={align}：顶点数应为 6×字形数");
        let first = s.vertices[0];
        let (gx0, gy0, _) = quads[0];
        assert!(
            close(first.pos[0], ndc_x(gx0.max(0), extent.width))
                && close(first.pos[1], ndc_y(gy0.max(0), extent.height)),
            "align={align}：首顶点应当落在首字形左上角 ({gx0}, {gy0})，实际 {:?}",
            first.pos
        );
    }
}

/// 多字符：`pen` 逐字形累加 advance，且每个字形都用 `pen.round()` 落位。
#[test]
fn multi_char_advances_the_pen_glyph_by_glyph() {
    let Some(mut e) = engine(18.0) else { return };
    let extent = Extent {
        width: 160,
        height: 40,
    };
    let rect = RectI::new(2, 2, 150, 30);
    let size = 18.0;

    let quads = cpu_layout(&mut e, "AB", rect, size, 0);
    assert_eq!(quads.len(), 2);
    let list = text_list("AB", rect, size, 0);
    let s = gpu_text::build_text_stream(&list, extent, &mut e);
    assert_eq!(s.vertices.len(), 12);
    assert_eq!(s.skipped, 0);

    for (i, (gx0, gy0, _)) in quads.iter().enumerate() {
        let v = s.vertices[i * 6];
        assert!(
            close(v.pos[0], ndc_x(*gx0, extent.width)) && close(v.pos[1], ndc_y(*gy0, extent.height)),
            "第 {i} 个字形的左上角应为 ({gx0}, {gy0})，实际 {:?}",
            v.pos
        );
    }
    // 第二个字形必须更靠右（advance 真的累加了）
    assert!(
        s.vertices[6].pos[0] > s.vertices[0].pos[0],
        "第二个字形必须在第一个右边"
    );
}

/// **`align = 2` + 文本宽于 `rect` ⇒ 起点为负，被画布裁掉**（「起点公式 × 裁剪」的交互）。
///
/// 这是控制者点名的边界（第 1 轮核查的 harness 里已验证过一次，这里把它变成长期回归）：
/// - 首个可见顶点必须在画布左边界（NDC `-1`），**不能**是负起点映射出来的 `<-1`；
/// - `uv` 必须跟着像素位置右移（裁掉多少列，`u` 就右移多少列）；
/// - 所有顶点都必须落在画布内（`pos ∈ [-1, 1]`）—— 这条能抓住「裁剪漏了」的整类错误。
#[test]
fn align_2_with_text_wider_than_rect_clips_the_negative_start() {
    let Some(mut e) = engine(20.0) else { return };
    let extent = Extent {
        width: 128,
        height: 48,
    };
    let size = 20.0;
    let rect = RectI::new(10, 5, 30, 30); // 窄到让右对齐的起点为负

    // 前提自查：这个 rect 必须真的窄（否则本用例失去意义）
    let total: f32 = {
        let ps: Vec<Option<GlyphPlacement>> = "Align".chars().map(|c| e.glyph(c, size)).collect();
        ps.iter().flatten().map(|p| p.advance).sum()
    };
    assert!(
        rect.right() as f32 - total < 0.0,
        "前提：文本宽 {total} 必须超过 rect 右边界 {} ⇒ 起点为负",
        rect.right()
    );

    let quads = cpu_layout(&mut e, "Align", rect, size, 2);
    assert!(quads[0].0 < 0, "首字形位图左边缘应当为负，实际 {}", quads[0].0);
    let (gx0, _gy0, slot) = quads[0];
    let (aw, _ah) = e.atlas().size();

    let list = text_list("Align", rect, size, 2);
    let s = gpu_text::build_text_stream(&list, extent, &mut e);
    assert!(!s.vertices.is_empty(), "被裁掉左边之后仍应有可见字形");
    assert_eq!(s.skipped, 0, "画出东西了 ⇒ 不算 skipped");

    let v = s.vertices[0];
    assert!(
        close(v.pos[0], -1.0),
        "首个可见顶点必须贴在画布左边界（NDC -1），实际 {:?}（起点 {gx0}）",
        v.pos
    );
    assert!(
        close(v.uv[0], (slot.x as f32 + (0 - gx0) as f32) / aw as f32),
        "uv 必须跟着像素位置右移：裁掉 {} 列 ⇒ u 应右移同样多列",
        -gx0
    );
    // 全部顶点都在画布内（裁剪没漏）
    for (i, v) in s.vertices.iter().enumerate() {
        assert!(
            (-1.0..=1.0).contains(&v.pos[0]) && (-1.0..=1.0).contains(&v.pos[1]),
            "顶点 {i} 越出画布：{:?}",
            v.pos
        );
    }
}

/// **未定义的 `align` 值 ⇒ 左对齐兜底**（契约的一部分：CPU 是 `_ => rect.x as f32`）。
///
/// 也就是说 `align = 3 / 9 / 255` 必须与 `align = 0` 产出**完全相同**的顶点流 ——
/// 将来若有人把 match 改成「1/2 之外报错或夹到最近值」，这条会红。
#[test]
fn unknown_align_values_fall_back_to_left_alignment() {
    let Some(mut e) = engine(20.0) else { return };
    let extent = Extent {
        width: 160,
        height: 48,
    };
    let rect = RectI::new(10, 5, 120, 30);
    let size = 20.0;

    let baseline = gpu_text::build_text_stream(&text_list("Align", rect, size, 0), extent, &mut e);
    assert!(!baseline.vertices.is_empty());

    for align in [3u8, 9, 255] {
        let s = gpu_text::build_text_stream(&text_list("Align", rect, size, align), extent, &mut e);
        assert_eq!(
            s.vertices, baseline.vertices,
            "align={align} 未定义 ⇒ 必须与 align=0（左对齐）产出完全相同的顶点流"
        );
        assert_eq!(s.skipped, baseline.skipped);
    }
}

/// **越界 alpha 必须与 CPU 基线一样夹到 `[0,1]`**（`gpu_text` 侧的覆盖缺口，现已补上）。
///
/// CPU `null.rs::blend_cov` 第一步就是 `c.a.clamp(0.0, 1.0)`，而 `Color::rgba` 对 alpha 没有校验。
/// 若不夹：`a = 1.5` 时 GPU 会按 `src*1.5 + dst*(-0.5)` 外推混合，而 CPU 按 `a = 1.0` —— 两边必然对不上。
/// 这条用例与 `gpu_geom_stream::out_of_range_alpha_is_clamped_like_the_cpu_baseline` 对称；
/// **它存在的意义**：将来把 `color_f32` 从 `gpu_geom` 提为 `pub(crate)` 共用时，缺口不会一起被继承。
#[test]
fn out_of_range_alpha_is_clamped_like_the_cpu_baseline() {
    let Some(mut e) = engine(20.0) else { return };
    let Some(e_cpu) = engine(20.0) else { return };
    let extent = Extent {
        width: 96,
        height: 48,
    };
    let rect = RectI::new(4, 4, 80, 36);
    let size = 20.0;
    let mk = |a: f32| {
        let mut l = DrawList::new();
        l.push(DrawCmd::Text {
            rect,
            text: "Ag".into(),
            color: Color::rgba(10, 20, 30, a),
            size,
            align: 0,
        });
        l
    };

    // ① a = 1.5 ⇒ 顶点 alpha 夹到 1.0
    let s_hi = gpu_text::build_text_stream(&mk(1.5), extent, &mut e);
    assert!(!s_hi.vertices.is_empty());
    for (i, v) in s_hi.vertices.iter().enumerate() {
        assert!(close(v.color[3], 1.0), "顶点 {i}：a=1.5 应夹到 1.0，实际 {}", v.color[3]);
        assert!(close(v.color[0], 10.0 / 255.0), "RGB 不该被改动");
    }
    // ② a = -0.25 ⇒ 顶点 alpha 夹到 0.0；CPU 那边也一个像素都不写
    let s_lo = gpu_text::build_text_stream(&mk(-0.25), extent, &mut e);
    assert!(!s_lo.vertices.is_empty(), "几何照旧产出（裁剪/覆盖与颜色无关）");
    for (i, v) in s_lo.vertices.iter().enumerate() {
        assert!(close(v.color[3], 0.0), "顶点 {i}：a=-0.25 应夹到 0.0，实际 {}", v.color[3]);
    }
    let clear = Color::rgb(0, 0, 0);
    let fb_lo = CpuRenderer::with_text(e_cpu)
        .render(extent, &mk(-0.25), clear)
        .expect("CPU 渲染");
    let wrote = (0..extent.height as i32)
        .flat_map(|y| (0..extent.width as i32).map(move |x| (x, y)))
        .filter(|&(x, y)| fb_lo.pixel(x, y).map(|p| p[0..3] != [0, 0, 0]).unwrap_or(false))
        .count();
    assert_eq!(wrote, 0, "a 夹到 0 ⇒ CPU 不写任何像素（GPU 侧顶点 alpha = 0，语义一致）");
}

/// **假阳性修复**：空串 ⇒ 跳过并计入 `skipped`，**不报错**（这个 API 根本没有 Result 可报错）。
#[test]
fn empty_string_is_skipped_not_an_error() {
    let Some(mut e) = engine(16.0) else { return };
    let extent = Extent {
        width: 64,
        height: 32,
    };
    let list = text_list("", RectI::new(2, 2, 60, 20), 16.0, 0);
    let s = gpu_text::build_text_stream(&list, extent, &mut e);
    assert!(s.vertices.is_empty(), "空串不产顶点");
    assert_eq!(s.skipped, 1, "空串的命令计入 skipped");
}

/// **假阳性修复**：`size <= 0` ⇒ 跳过。
///
/// ⚠️ 这里与 CPU 有一处**有意的**差异（实现文档里也写了）：CPU 的 `draw_text_real` 会把
/// `size` 交给 `TextEngine::glyph`，而后者会把字号夹到 `>= 1` ⇒ **CPU 会画 1px 的字形**。
/// 本计划（Task 3）显式要求「`size <= 0` ⇒ 跳过」，所以 GPU 侧不画。
#[test]
fn non_positive_size_is_skipped_not_an_error() {
    let Some(mut e) = engine(16.0) else { return };
    let extent = Extent {
        width: 64,
        height: 32,
    };
    let mut list = DrawList::new();
    for size in [0.0f32, -3.0] {
        list.push(DrawCmd::Text {
            rect: RectI::new(2, 2, 60, 20),
            text: "Hi".into(),
            color: FG,
            size,
            align: 0,
        });
    }
    let s = gpu_text::build_text_stream(&list, extent, &mut e);
    assert!(s.vertices.is_empty(), "size <= 0 不产顶点");
    assert_eq!(s.skipped, 2, "两条都被跳过");
}

/// **假阳性修复**：与 clip 求交后无可见字形 ⇒ 跳过（不报错）。
#[test]
fn clipped_away_text_is_skipped_not_an_error() {
    let Some(mut e) = engine(16.0) else { return };
    let extent = Extent {
        width: 64,
        height: 32,
    };
    let mut list = DrawList::new();
    list.push(DrawCmd::PushClip {
        rect: RectI::new(100, 100, 4, 4), // 完全在画布外
    });
    list.push(DrawCmd::Text {
        rect: RectI::new(2, 2, 60, 20),
        text: "Hi".into(),
        color: FG,
        size: 16.0,
        align: 0,
    });
    list.push(DrawCmd::PopClip);
    let s = gpu_text::build_text_stream(&list, extent, &mut e);
    assert!(s.vertices.is_empty(), "被裁空不产顶点");
    assert_eq!(s.skipped, 1, "被裁空的文本命令计入 skipped");
}

/// **局部裁剪**：`uv` 必须跟着**像素位置**走 —— 裁掉左边若干像素后，左边界处的 `u`
/// 要相应右移（如果照抄整块 uv，GPU 就会把字形整体向左拉伸，采样错位）。
#[test]
fn partially_clipped_glyph_keeps_uv_tied_to_pixel_position() {
    let Some(mut e) = engine(24.0) else { return };
    let extent = Extent {
        width: 64,
        height: 32,
    };
    let size = 24.0;
    let rect = RectI::new(2, 4, 60, 24);
    let quads = cpu_layout(&mut e, "H", rect, size, 0);
    let (gx0, gy0, slot) = quads[0];
    let (aw, ah) = e.atlas().size();
    assert!(slot.w > 4, "这个字形的位图要够宽，才谈得上「裁掉左边几列」");
    assert!(gx0 + 3 > 0, "裁剪边界要落在画布内");
    assert!(
        gy0 >= 0 && gy0 + slot.h as i32 <= extent.height as i32,
        "本用例字形在垂直方向完整可见"
    );

    // 裁掉字形左边的 3 列像素
    let cut = gx0 + 3;
    let mut list = DrawList::new();
    list.push(DrawCmd::PushClip {
        rect: RectI::new(cut, 0, 60, 32),
    });
    list.push(DrawCmd::Text {
        rect,
        text: "H".into(),
        color: FG,
        size,
        align: 0,
    });
    list.push(DrawCmd::PopClip);

    let s = gpu_text::build_text_stream(&list, extent, &mut e);
    assert_eq!(s.vertices.len(), 6);
    let v = s.vertices[0];
    assert!(
        close(v.pos[0], ndc_x(cut, extent.width)),
        "左边界应当被裁到 x = {cut}，实际 {:?}",
        v.pos
    );
    assert!(
        close(v.uv[0], (slot.x as f32 + 3.0) / aw as f32),
        "左边界处的 u 应当是「槽位第 3 列」= {}，实际 {}",
        (slot.x as f32 + 3.0) / aw as f32,
        v.uv[0]
    );
    // 上下未被裁 ⇒ v 仍是槽位顶边
    assert!(
        close(v.uv[1], slot.y as f32 / ah as f32),
        "上边界未被裁 ⇒ v 仍是槽位顶边"
    );
}

/// **缺字走 `.notdef`**：未映射字符回退到 glyph 0（豆腐块），而不是静默不画。
#[test]
fn missing_char_falls_back_to_notdef() {
    let Some(mut e) = engine(20.0) else { return };
    let extent = Extent {
        width: 64,
        height: 32,
    };
    // 私用区字符在正常字体里都没有映射
    let before = e.missing_glyphs();
    let mut l1 = DrawList::new();
    l1.push(DrawCmd::Text {
        rect: RectI::new(2, 2, 60, 24),
        text: '\u{E123}'.to_string(),
        color: FG,
        size: 20.0,
        align: 0,
    });
    let s = gpu_text::build_text_stream(&l1, extent, &mut e);
    assert!(e.missing_glyphs() > before, "缺字必须计入 missing_glyphs（诊断）");
    assert_eq!(s.vertices.len(), 6, "缺字走 .notdef：仍然画出豆腐块");
    assert_eq!(s.skipped, 0);

    // 它用的应当是 glyph 0 的槽位：与另一个同样未映射的字符产出的顶点完全一致
    let mut l2 = DrawList::new();
    l2.push(DrawCmd::Text {
        rect: RectI::new(2, 2, 60, 24),
        text: '\u{E456}'.to_string(),
        color: FG,
        size: 20.0,
        align: 0,
    });
    let s2 = gpu_text::build_text_stream(&l2, extent, &mut e);
    assert_eq!(s.vertices, s2.vertices, "两个未映射字符必须都落到同一张 .notdef 位图");
}

/// **极大 size**：字形比图集还大 ⇒ 图集明确拒绝（`glyph()` 返回 `None`）⇒ 不产顶点、不 panic，
/// 且这条命令计入 `skipped`（它一个像素都没画出来）。
///
/// **为什么是 1500 而不是 5000**：光栅化是 O(面积)，5000px 的字形要 **16.5 秒**（实测），
/// 而 1500px 已经足够让 `W` 的位图宽（~1000）远超图集宽 512 ⇒ 断言强度一样、代价 1/11。
/// 用例开头**先自查前提**：若这个字号竟然放得下（换字体/换字号规则时会），
/// 会明确提示「请调大 size」，而不是让断言悄悄失去意义。
#[test]
fn huge_size_emits_nothing_without_panicking() {
    let Some(mut e) = engine(16.0) else { return };
    let extent = Extent {
        width: 64,
        height: 32,
    };
    let huge = 1500.0f32;
    assert!(
        e.glyph('W', huge).is_none(),
        "前提：{huge}px 的 'W' 必须放不进图集（图集宽 {}）—— 若这里失败，请把 size 调大",
        e.atlas().size().0
    );

    let list = text_list("W", RectI::new(0, 0, 64, 32), huge, 0);
    let s = gpu_text::build_text_stream(&list, extent, &mut e);
    assert!(
        s.vertices.is_empty(),
        "{huge}px 的字形放不进图集 ⇒ 不产顶点，实际 {} 个",
        s.vertices.len()
    );
    assert_eq!(s.skipped, 1, "什么都没画出来的命令计入 skipped");
}

/// **零面积 `rect` 仍然绘制** —— 这是**照抄 CPU** 的行为，不是漏判：
/// CPU 的 `draw_text_real` 只用 `rect` 算「起点 + 基线」，字形大小来自 `size`
/// ⇒ `rect.w = 0` 时它照样画（只是对齐/基线按这个退化盒子算）。
///
/// 所以「零面积」**不在**跳过条件里（跳过条件是：空串 / `size <= 0` / 被裁空）。
#[test]
fn zero_area_rect_still_draws_like_the_cpu() {
    let Some(mut e) = engine(16.0) else { return };
    let extent = Extent {
        width: 64,
        height: 32,
    };
    let rect = RectI::new(4, 4, 0, 20);
    let quads = cpu_layout(&mut e, "L", rect, 16.0, 0);
    assert_eq!(quads.len(), 1);
    let list = text_list("L", rect, 16.0, 0);
    let s = gpu_text::build_text_stream(&list, extent, &mut e);
    assert_eq!(s.vertices.len(), 6, "零面积 rect 仍应画出字形（与 CPU 一致）");
    assert_eq!(s.skipped, 0, "它确实画出了东西 ⇒ 不算 skipped");
}

/// **形状命令与文本交错不影响文本流**（`NodeHint` 忽略、形状命令不属于文本流）。
#[test]
fn shape_commands_do_not_affect_the_text_stream() {
    let Some(mut e) = engine(18.0) else { return };
    let extent = Extent {
        width: 96,
        height: 40,
    };
    let rect = RectI::new(3, 3, 80, 30);

    let only_text = text_list("Ok", rect, 18.0, 0);
    let mut mixed = DrawList::new();
    mixed.push(DrawCmd::FillRect {
        rect: RectI::new(0, 0, 96, 40),
        color: BG,
    });
    mixed.push(DrawCmd::node_hint(RectI::new(0, 0, 96, 40), "mix"));
    mixed.push(DrawCmd::StrokeRect {
        rect: RectI::new(1, 1, 94, 38),
        color: FG,
        width: 1,
    });
    mixed.push(DrawCmd::Text {
        rect,
        text: "Ok".into(),
        color: FG,
        size: 18.0,
        align: 0,
    });
    mixed.push(DrawCmd::FillRoundRect {
        rect: RectI::new(2, 2, 10, 10),
        radius: 2,
        color: FG,
    });

    let a = gpu_text::build_text_stream(&only_text, extent, &mut e);
    let b = gpu_text::build_text_stream(&mixed, extent, &mut e);
    assert_eq!(
        a.vertices, b.vertices,
        "形状/提示命令不得改变文本顶点流（它们走另一条管线）"
    );
    assert_eq!(a.skipped, b.skipped);
    assert!(!b.vertices.is_empty());
}

/// **性质测试（最近邻采样的纹素对应性）**：对每个四边形覆盖的每个像素，
/// 在**像素中心**插值 `uv` 再 `floor(u * 图集宽)`，必须正好是该像素在本字形里的那一列/行纹素。
///
/// 这是「GPU 用 NEAREST 采样得到的覆盖率 == CPU 直接查表得到的覆盖率」的充分条件，
/// 也是 Task 4 里文本能逐像素对上的**几何前提**。
#[test]
fn nearest_sampling_maps_every_covered_pixel_to_its_own_texel() {
    let Some(mut e) = engine(20.0) else { return };
    let extent = Extent {
        width: 160,
        height: 64,
    };
    let rect = RectI::new(6, 8, 148, 48);
    let size = 20.0;
    let quads = cpu_layout(&mut e, "Wave", rect, size, 0);
    assert_eq!(quads.len(), 4);
    let (aw, ah) = e.atlas().size();

    let list = text_list("Wave", rect, size, 0);
    let s = gpu_text::build_text_stream(&list, extent, &mut e);
    assert_eq!(s.vertices.len(), 24);

    for (qi, (gx0, gy0, slot)) in quads.iter().enumerate() {
        let q = &s.vertices[qi * 6..qi * 6 + 6];
        // 反推这个四边形的像素范围与 uv 范围
        let (x0, y0) = (px_of(q[0].pos[0], extent.width), px_of(q[0].pos[1], extent.height));
        let (x1, y1) = (px_of(q[2].pos[0], extent.width), px_of(q[2].pos[1], extent.height));
        let (u0, v0) = (q[0].uv[0], q[0].uv[1]);
        let (u1, v1) = (q[2].uv[0], q[2].uv[1]);
        assert_eq!(
            (x0, y0),
            (*gx0, *gy0),
            "四边形左上角应当就是 CPU 的落位点（本用例字形完整落在画布内）"
        );
        assert_eq!((x1, y1), (gx0 + slot.w as i32, gy0 + slot.h as i32));

        for y in y0..y1 {
            for x in x0..x1 {
                let fx = (x as f32 + 0.5 - x0 as f32) / (x1 - x0) as f32;
                let fy = (y as f32 + 0.5 - y0 as f32) / (y1 - y0) as f32;
                let u = u0 + fx * (u1 - u0);
                let v = v0 + fy * (v1 - v0);
                let (tx, ty) = ((u * aw as f32).floor() as i32, (v * ah as f32).floor() as i32);
                let want = (slot.x as i32 + (x - gx0), slot.y as i32 + (y - gy0));
                assert_eq!(
                    (tx, ty),
                    want,
                    "像素 ({x}, {y}) 的最近邻采样应落在纹素 {want:?}，实际 ({tx}, {ty})"
                );
            }
        }
    }
}

/// **CPU 后端当独立参照物（两个方向）**：
/// 「CPU 写了颜色的像素」必须**恰好等于**「顶点流覆盖到、且映射纹素覆盖率 > 0 的像素」。
///
/// - 少了 ⇒ 几何/落位/基线错了（GPU 会漏画）；
/// - 多了 ⇒ 多边形覆盖或 uv 映射错了（GPU 会多画）。
#[test]
fn cpu_ink_pixels_equal_the_pixels_covered_with_nonzero_coverage() {
    let Some(mut e) = engine(20.0) else { return };
    let Some(e_cpu) = engine(20.0) else { return };
    let extent = Extent {
        width: 128,
        height: 48,
    };
    let size = 20.0;
    let rect = RectI::new(4, 4, 110, 40);
    let list = text_list("Ag1", rect, size, 0);

    let s = gpu_text::build_text_stream(&list, extent, &mut e);
    assert_eq!(s.vertices.len(), 18, "三个字形各 6 顶点");
    let (aw, ah) = e.atlas().size();
    let cov = e.atlas().coverage().to_vec();

    // ① 用顶点流 + 最近邻语义推出「本帧会有墨迹的像素」
    let (w, h) = (extent.width as i32, extent.height as i32);
    let mut inked = vec![false; (w * h) as usize];
    for q in s.vertices.chunks_exact(6) {
        let (x0, y0) = (px_of(q[0].pos[0], extent.width), px_of(q[0].pos[1], extent.height));
        let (x1, y1) = (px_of(q[2].pos[0], extent.width), px_of(q[2].pos[1], extent.height));
        let (u0, v0) = (q[0].uv[0], q[0].uv[1]);
        let (u1, v1) = (q[2].uv[0], q[2].uv[1]);
        for y in y0..y1 {
            for x in x0..x1 {
                if x < 0 || y < 0 || x >= w || y >= h {
                    continue;
                }
                let fx = (x as f32 + 0.5 - x0 as f32) / (x1 - x0) as f32;
                let fy = (y as f32 + 0.5 - y0 as f32) / (y1 - y0) as f32;
                let u = u0 + fx * (u1 - u0);
                let v = v0 + fy * (v1 - v0);
                let tx = ((u * aw as f32).floor() as u32).min(aw - 1);
                let ty = ((v * ah as f32).floor() as u32).min(ah - 1);
                if cov[(ty * aw + tx) as usize] > 0 {
                    inked[(y * w + x) as usize] = true;
                }
            }
        }
    }

    // ② CPU 参考实现画一遍，逐像素比
    let fb = CpuRenderer::with_text(e_cpu)
        .render(extent, &list, BG)
        .expect("CPU 渲染失败");
    let mut mismatches = 0usize;
    for y in 0..h {
        for x in 0..w {
            let cpu_wrote = fb.pixel(x, y).map(|p| p[0..3] != [0, 0, 0]).unwrap_or(false);
            let pred = inked[(y * w + x) as usize];
            if cpu_wrote != pred {
                if mismatches < 5 {
                    eprintln!("像素 ({x}, {y})：CPU 写了={cpu_wrote}，顶点流预测={pred}");
                }
                mismatches += 1;
            }
        }
    }
    assert_eq!(mismatches, 0, "CPU 墨迹像素与顶点流预测必须逐像素一致");
}
