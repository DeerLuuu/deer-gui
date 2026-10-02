//! M4 光栅化验收：`Rasterizer` 把 `font::Glyph` 的轮廓变成覆盖率图。
//!
//! 分两层：
//!
//! - **手工构造的 `Glyph`**（units / advance 自己定）⇒ 在任何机器上结果都一样，
//!   断言是硬的（不依赖系统装了什么字体）；
//! - **系统字体**（`%WINDIR%\Fonts\consola.ttf`）⇒ 端到端检查真实字形（洞、窄字形、空格）。
//!   找不到文件时 `eprintln!` **明确打印「跳过」并说明哪些断言没有执行**，不伪装通过。
//!
//! 覆盖率断言有一部分利用了「轴对齐矩形轮廓的覆盖率可分离」这一性质：
//! 像素 `(i, j)` 的覆盖率 = `cx(i) * cy(j)`，所以期望值可以手算出来（见各测试注释）。
//! 这些手算值就是独立参照 —— 不是把实现跑出来的数字抄回测试。

use deer_text::font::{Contour, Font, Glyph, Segment};
use deer_text::glyph::GlyphImage;
use deer_text::raster::{split_subpixel_x, Rasterizer, SUBPIXEL_LEVELS};

// ───────────────────────── 手工构造字形的工具 ─────────────────────────

/// 手工构造一个字形：只给轮廓与 advance（font units），bbox 留空（光栅化器只用轮廓点算 bbox）。
fn glyph(contours: Vec<Contour>, advance_units: u16) -> Glyph {
    Glyph {
        contours,
        bbox: (0, 0, 0, 0),
        advance_width: advance_units,
        left_side_bearing: 0,
    }
}

/// 折线闭合轮廓：起点 + 各顶点，最后显式闭合回起点（跟真字体的轮廓一样首尾相接）。
fn polygon(points: &[(f32, f32)]) -> Contour {
    let mut segments: Vec<Segment> = points[1..].iter().map(|&to| Segment::Line { to }).collect();
    segments.push(Segment::Line { to: points[0] });
    Contour {
        start: points[0],
        segments,
    }
}

/// 轴对齐正方形轮廓（左下 + 边长），顶点顺序给出**逆时针**（font units 的 y 向上）。
fn square_ccw(x: f32, y: f32, side: f32) -> Contour {
    polygon(&[
        (x, y),
        (x + side, y),
        (x + side, y + side),
        (x, y + side),
    ])
}

/// 正方形轮廓的**顺时针**版本（同样的四个角，顺序反过来）。
fn square_cw(x: f32, y: f32, side: f32) -> Contour {
    polygon(&[
        (x, y),
        (x, y + side),
        (x + side, y + side),
        (x + side, y),
    ])
}

/// 4 段二次贝塞尔近似的圆（on-curve 在 `(±r,0)/(0,±r)`，控制点在四个角）。
///
/// 用二次贝塞尔而不是真圆，正好也把 `Segment::Quad` 的展平路径测到。
fn quad_circle(r: f32) -> Contour {
    Contour {
        start: (r, 0.0),
        segments: vec![
            Segment::Quad {
                ctrl: (r, r),
                to: (0.0, r),
            },
            Segment::Quad {
                ctrl: (-r, r),
                to: (-r, 0.0),
            },
            Segment::Quad {
                ctrl: (-r, -r),
                to: (0.0, -r),
            },
            Segment::Quad {
                ctrl: (r, -r),
                to: (r, 0.0),
            },
        ],
    }
}

/// 4 段**三次**贝塞尔近似的圆（标准 kappa = 0.5523）。
///
/// `glyf` 里没有三次段，但 API 要求 `Segment::Cubic` 也走自适应展平（留给 CFF / 其它轮廓源），
/// 所以这条路径必须有测试。4 段三次贝塞尔近似的圆非常接近真圆。
fn cubic_circle(r: f32) -> Contour {
    // 标准 kappa = 4(√2 − 1)/3 ≈ 0.5522847：写成公式而不是magic number，
    // 既不用手抄精度（f32 抄多了会被 clippy::excessive_precision 拦），也能自解释。
    let k = r * 4.0 * (std::f32::consts::SQRT_2 - 1.0) / 3.0;
    Contour {
        start: (r, 0.0),
        segments: vec![
            Segment::Cubic {
                c1: (r, k),
                c2: (k, r),
                to: (0.0, r),
            },
            Segment::Cubic {
                c1: (-k, r),
                c2: (-r, k),
                to: (-r, 0.0),
            },
            Segment::Cubic {
                c1: (-r, -k),
                c2: (-k, -r),
                to: (0.0, -r),
            },
            Segment::Cubic {
                c1: (k, -r),
                c2: (r, -k),
                to: (r, 0.0),
            },
        ],
    }
}

/// 找一个系统 TrueType 字体（与 `font_parse.rs` 同策略；consola 优先，等宽好核对）。
fn system_font() -> Option<(String, Vec<u8>)> {
    let windir = std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".to_string());
    for name in ["consola.ttf", "arial.ttf", "segoeui.ttf", "verdana.ttf"] {
        let p = format!("{windir}\\Fonts\\{name}");
        if let Ok(d) = std::fs::read(&p) {
            return Some((name.to_string(), d));
        }
    }
    None
}

// ───────────────────────── 1. 实心正方形 ─────────────────────────

/// units 0..10、upem=10、ppem=10 ⇒ `scale = 1.0`，正方形正好是一整块 10×10 的实心位图。
#[test]
fn unit_square_is_fully_covered() {
    let g = glyph(vec![square_ccw(0.0, 0.0, 10.0)], 10);
    let img: GlyphImage = Rasterizer::new(10.0).rasterize(&g, 10);

    assert_eq!((img.width, img.height), (10, 10), "位图应当正好 10×10");
    assert_eq!(img.left, 0, "left = floor(0 * 1.0) = 0");
    assert_eq!(img.top, 10, "top = ceil(10 * 1.0) = 10");
    assert_eq!(img.advance, 10.0, "advance = 10 units × scale 1.0");
    assert_eq!(img.coverage.len(), 100, "coverage 长度不变式");
    assert!(
        img.coverage.iter().all(|&c| c == 255),
        "实心正方形的每个像素都该是 255，实际 min={:?} max={}",
        img.coverage.iter().copied().min(),
        img.max_coverage()
    );
    assert_eq!(img.ink_pixels(255), 100, "100 个满覆盖像素");
    assert_eq!(img.coverage_sum(), 25500, "100 × 255");
    // `GlyphImage::mean_coverage()` 与 `coverage` **同一量纲**：覆盖率总和 / 像素数，
    // 满覆盖给 255.0 而不是 1.0（`glyph.rs` 的文档已明确写清这一点；要比例请自己除以 255）。
    assert_eq!(img.mean_coverage(), 255.0, "满覆盖位图的平均覆盖率（量纲 0..=255，与 coverage 相同）");
}

// ───────────────────────── 2. 半像素平移 ⇒ 抗锯齿 ─────────────────────────

/// units 0.5..10.5 ⇒ 正方形跨 11×11 像素，最外一圈被切一半。
///
/// 手算（ss=4，每像素 16 采样）：墨迹区域是 `[0.5,10.5]²`，与像素网格可分离。
///
/// - 4 个角像素：x、y 各命中 2/4 ⇒ 4/16 ⇒ `round(4*255/16) = 64`
/// - 36 个边像素：一个方向 4/4、另一个 2/4 ⇒ 8/16 ⇒ `128`
/// - 81 个内部像素：16/16 ⇒ `255`
///
/// 于是 `coverage_sum = 4*64 + 36*128 + 81*255 = 25519`，墨迹面积 = 25519/255 ≈ 100.07 px²
/// （正方形本来就是 10×10 = 100 px²，误差只来自覆盖率量化）。
#[test]
fn half_pixel_shift_gives_antialiased_edges_and_conserves_area() {
    let g = glyph(vec![square_ccw(0.5, 0.5, 10.0)], 10);
    let img = Rasterizer::new(10.0).rasterize(&g, 10);

    assert_eq!((img.width, img.height), (11, 11), "平移半像素后跨 11 像素");
    assert_eq!(img.left, 0, "floor(0.5) = 0");
    assert_eq!(img.top, 11, "ceil(10.5) = 11");

    let partial = img.coverage.iter().filter(|&&c| c > 0 && c < 255).count();
    assert_eq!(partial, 40, "最外一圈 11×11 减去 9×9 内部 = 40 个半覆盖像素");
    assert_eq!(img.coverage_at(0, 0), 64, "角像素被切 3/4 ⇒ 4/16 采样 ⇒ 64");
    assert_eq!(img.coverage_at(0, 5), 128, "边像素被切一半 ⇒ 8/16 采样 ⇒ 128");
    assert_eq!(img.coverage_at(5, 5), 255, "内部像素满覆盖");
    assert_eq!(img.coverage_sum(), 25519, "见测试文档里的手算");

    let area = img.coverage_sum() as f32 / 255.0;
    assert!(
        (area - 100.0).abs() <= 3.0,
        "墨迹面积应 ≈ 100 px²（±3），实际 {area:.3}"
    );

    // ss=1：采样点只有像素中心 ⇒ 覆盖率只可能是 0 或 255（没有抗锯齿）。
    let hard_r = Rasterizer::with_supersample(10.0, 1);
    assert_eq!(hard_r.supersample, 1, "with_supersample 应把倍率限制为至少 1");
    let hard = hard_r.rasterize(&g, 10);
    assert!(
        hard.coverage.iter().all(|&c| c == 0 || c == 255),
        "ss=1 时覆盖率只能是 0/255，实际出现了 {:?}",
        hard.coverage.iter().find(|&&c| c != 0 && c != 255)
    );
    // 硬边版本仍然是「差不多半个正方形有墨」：面积应当仍在同一量级
    let hard_area = hard.coverage_sum() as f32 / 255.0;
    assert!(
        (hard_area - 100.0).abs() <= 15.0,
        "ss=1 的面积仍应 ≈ 100（±15，量化更粗），实际 {hard_area:.3}"
    );
}

// ───────────────────────── 3. nonzero winding 出洞 ─────────────────────────

/// 外轮廓逆时针、内轮廓顺时针 ⇒ 内轮廓把环绕数抵消掉 ⇒ 中心出现洞。
#[test]
fn nonzero_winding_punches_hole_when_orientations_differ() {
    let g = glyph(
        vec![square_ccw(0.0, 0.0, 10.0), square_cw(3.0, 3.0, 4.0)],
        10,
    );
    let img = Rasterizer::new(10.0).rasterize(&g, 10);

    assert_eq!((img.width, img.height), (10, 10));
    assert_eq!(img.coverage_at(5, 5), 0, "中心（内轮廓内）必须是洞");
    assert_eq!(img.coverage_at(4, 4), 0, "内轮廓范围内都该是洞");
    assert_eq!(img.coverage_at(1, 5), 255, "左侧外环有墨");
    assert_eq!(img.coverage_at(8, 5), 255, "右侧外环有墨");
    assert_eq!(img.coverage_at(5, 1), 255, "上侧外环有墨");
    assert_eq!(img.coverage_at(5, 8), 255, "下侧外环有墨");

    // 洞的面积：外 100 - 内 16 = 84 px²；满覆盖像素数应当明显小于 100
    let ink = img.ink_pixels(255);
    assert!(
        ink > 60 && ink < 100,
        "外环满覆盖像素应在 60..100 之间（洞挖掉了约 16 px²），实际 {ink}"
    );
}

/// 与上一条成对：两个轮廓**同向**时，nonzero 规则下中心仍是实心（环绕数 = 2）。
///
/// 这条是「第 3 条真的有区分度」的证明：even-odd 规则在这里会误判出洞。
#[test]
fn same_orientation_contours_do_not_punch_hole() {
    let g = glyph(
        vec![square_ccw(0.0, 0.0, 10.0), square_ccw(3.0, 3.0, 4.0)],
        10,
    );
    let img = Rasterizer::new(10.0).rasterize(&g, 10);

    assert_eq!(
        img.coverage_at(5, 5),
        255,
        "同向两个轮廓在 nonzero 下中心仍是实心（even-odd 会在这里误判成洞）"
    );
    assert_eq!(img.coverage_at(4, 4), 255);
    assert_eq!(
        img.coverage_sum(),
        25500,
        "同向同心正方形合起来就是整块 10×10 实心"
    );
    assert_eq!(img.ink_pixels(255), 100);
}

// ───────────────────────── 4. y 轴翻转 ─────────────────────────

/// 一个**上下不对称**的三角形 `(0,0)-(10,0)-(5,10)`：字体 units 下是「底边在下、顶点在上」。
///
/// 位图行向下（`py = top - y`）⇒ 顶点在位图**顶行**、底边在**底行**：底行的有墨列数
/// 必须多于顶行。忘了 y 翻转会把顶点翻到底部（顶行变成整条底边），这条测试就是为它写的
///（对称的正方形/圆抓不到这个 bug）。
#[test]
fn y_axis_is_flipped_between_font_units_and_bitmap_rows() {
    let g = glyph(vec![polygon(&[(0.0, 0.0), (10.0, 0.0), (5.0, 10.0)])], 10);
    let img = Rasterizer::new(10.0).rasterize(&g, 10);

    assert_eq!((img.width, img.height), (10, 10));
    // 顶点在顶部 ⇒ 顶行只有顶点附近有墨、两个上角必须是空的
    assert_eq!(img.coverage_at(0, 0), 0, "左上角该空；忘了翻转会让它变实心");
    assert_eq!(img.coverage_at(9, 0), 0, "右上角该空；忘了翻转会让它变实心");
    assert!(
        (4..=5).any(|x| img.coverage_at(x, 0) > 0),
        "顶点附近（顶行中间）该有墨"
    );
    // 底边在底部 ⇒ 左下角必须有墨
    assert!(img.coverage_at(0, 9) > 0, "左下角该有墨（底边）");

    // 逐行看：底行的墨迹必须比顶行多（三角形从顶点往下变宽）
    let row_ink = |y: u32| (0..img.width).filter(|&x| img.coverage_at(x, y) > 0).count();
    assert!(
        row_ink(9) > row_ink(0),
        "底行有墨列数 {} 应多于顶行 {}",
        row_ink(9),
        row_ink(0)
    );
}

// ───────────────────────── 5. 无轮廓/退化输入 ─────────────────────────

/// 无轮廓字形 ⇒ `is_blank()`，但 advance 按比例保留（空格就是这条路径）。
#[test]
fn glyph_without_contours_is_blank_but_keeps_advance() {
    let g = glyph(Vec::new(), 500);
    assert!(g.is_blank(), "没有轮廓就是空字形");

    let img = Rasterizer::new(20.0).rasterize(&g, 1000); // scale = 0.02
    assert!(img.is_blank());
    assert_eq!((img.width, img.height), (0, 0));
    assert_eq!((img.left, img.top), (0, 0));
    assert_eq!(img.coverage_sum(), 0);
    assert!(
        (img.advance - 10.0).abs() < 1e-4,
        "advance = 500 units × 0.02 = 10px，实际 {}",
        img.advance
    );
}

/// 退化输入的定义：`ppem <= 0`（含 NaN）/ `units_per_em == 0` ⇒ `blank(0.0)`，不 panic。
#[test]
fn degenerate_scale_returns_blank_without_panic() {
    let g = glyph(vec![square_ccw(0.0, 0.0, 10.0)], 10);
    let cases = [
        (Rasterizer::new(0.0), 10u16, "ppem == 0"),
        (Rasterizer::new(-5.0), 10, "ppem < 0"),
        (Rasterizer::new(f32::NAN), 10, "ppem == NaN"),
        (Rasterizer::new(f32::INFINITY), 10, "ppem == +∞"),
        (Rasterizer::new(10.0), 0, "units_per_em == 0"),
    ];
    for (r, upem, why) in cases {
        let img = r.rasterize(&g, upem);
        assert!(img.is_blank(), "{why} 时必须给空位图");
        assert_eq!((img.width, img.height), (0, 0), "{why}");
        assert_eq!(img.advance, 0.0, "{why} 时 advance 定义为 0");
        assert_eq!(img.coverage.len(), 0, "{why}");
    }
}

// ───────────────────────── 6. 二次贝塞尔展平（圆） ─────────────────────────

/// 4 段 quad 近似的「圆」：墨迹面积相对**外接正方形**（位图 = 控制点 bbox）的比值
/// 应落在 60%~90%。
///
/// 注意这不是真圆：控制点在四个角上 ⇒ 曲线在 45° 方向**鼓出**（中点半径 1.0607r）。
/// 解析积分（Green 公式，单段：`x=1-t², y=2t-t²` ⇒ 面积贡献 `5/6 r²`）给出精确比值
/// **5/6 ≈ 0.8333**；真圆的 π/4 ≈ 0.7854 是另一个形状，不能拿来当这条的期望值。
///
/// 这里同时验证三件事：① `Quad` 被真的展平（不是当直线）；② 位图尺寸 = 控制点 bbox；
/// ③ 四个角空（形状内切于该正方形）。
#[test]
fn quad_circle_flattens_and_ink_ratio_is_plausible() {
    let r = 40.0f32;
    let g = glyph(vec![quad_circle(r)], 80);
    let img = Rasterizer::new(40.0).rasterize(&g, 100); // scale = 0.4 ⇒ 半径 16px

    assert_eq!(
        (img.width, img.height),
        (32, 32),
        "控制点 bbox 是 [-r,r]² ⇒ 32×32 的外接正方形"
    );
    assert_eq!(img.max_coverage(), 255, "圆心附近应当全实");
    assert_eq!(img.coverage_at(16, 16), 255, "圆心必须实心");

    let ink = img.coverage_sum() as f32 / 255.0;
    let bbox_area = (img.width * img.height) as f32;
    let ratio = ink / bbox_area;
    eprintln!(
        "[圆] 位图 {}×{}，墨迹面积 {ink:.1}px²，外接正方形 {bbox_area:.0}px²，比值 {ratio:.4}\
         （4 段 quad 形状的解析值 5/6 ≈ 0.8333；差的那 {:.2}% 来自 0.25px 展平容差与覆盖率量化）",
        img.width,
        img.height,
        (5.0 / 6.0 - ratio) * 100.0
    );
    assert!(
        (0.60..=0.90).contains(&ratio),
        "墨迹面积 / 外接正方形应在 60%~90%（解析值 5/6 ≈ 0.8333），实际 {ratio:.4}"
    );
    // 比 60%~90% 更强的一条：与解析值 5/6 的偏差只允许来自展平容差与覆盖率量化
    assert!(
        (ratio - 5.0 / 6.0).abs() < 0.02,
        "墨迹面积比 {ratio:.4} 应与解析值 5/6 ≈ {:.4} 相差 < 2%（差得多说明展平/填充有 bug）",
        5.0 / 6.0
    );

    // 四个角必须在圆外
    assert_eq!(img.coverage_at(0, 0), 0);
    assert_eq!(img.coverage_at(31, 0), 0);
    assert_eq!(img.coverage_at(0, 31), 0);
    assert_eq!(img.coverage_at(31, 31), 0);
    // 四条边的中点应当在圆上（有墨）
    assert!(img.coverage_at(16, 0) > 0, "上边中点应落在墨上");
    assert!(img.coverage_at(16, 31) > 0, "下边中点应落在墨上");
    assert!(img.coverage_at(0, 16) > 0, "左边中点应落在墨上");
    assert!(img.coverage_at(31, 16) > 0, "右边中点应落在墨上");
}

/// 三次贝塞尔（`Segment::Cubic`）也要走自适应展平。
///
/// 4 段 kappa 三次贝塞尔近似的圆非常接近真圆，所以这条的期望值就是 **π/4 ≈ 0.7854**
/// （与上一条的 quad 版 5/6 不同 —— 两条互为对照，说明展平真的在跟着控制点走）。
#[test]
fn cubic_circle_flattens_to_a_real_circle() {
    let r = 40.0f32;
    let g = glyph(vec![cubic_circle(r)], 80);
    let img = Rasterizer::new(40.0).rasterize(&g, 100); // scale = 0.4 ⇒ 半径 16px

    assert_eq!((img.width, img.height), (32, 32));
    assert_eq!(img.coverage_at(16, 16), 255, "圆心必须实心");

    let ink = img.coverage_sum() as f32 / 255.0;
    let bbox_area = (img.width * img.height) as f32;
    let ratio = ink / bbox_area;
    let pi_over_4 = std::f32::consts::PI / 4.0;
    eprintln!(
        "[cubic 圆] 墨迹面积 {ink:.1}px²，外接正方形 {bbox_area:.0}px²，比值 {ratio:.4}\
         （真圆 π/4 ≈ {pi_over_4:.4}；相对偏差 {:.2}%）",
        (ratio - pi_over_4) / pi_over_4 * 100.0
    );
    assert!(
        (ratio - pi_over_4).abs() < 0.02,
        "三次贝塞尔圆的墨迹面积比 {ratio:.4} 应接近 π/4 ≈ {pi_over_4:.4}（±0.02）"
    );
    assert_eq!(img.coverage_at(0, 0), 0, "左上角该空（圆内切于外接正方形）");
    assert_eq!(img.coverage_at(31, 31), 0, "右下角该空");
}

// ───────────────────────── 7. 确定性 ─────────────────────────

/// 同一输入连续两次光栅化 ⇒ `GlyphImage` 逐字段相等（含 coverage 逐字节）。
///
/// 这条是在挡「随机抖动 / 时间戳 / HashMap 迭代序」这类不确定性渗进来。
#[test]
fn rasterization_is_deterministic() {
    let g = glyph(
        vec![square_ccw(0.0, 0.0, 10.0), square_cw(3.0, 3.0, 4.0)],
        10,
    );
    // 故意用非整数 ppem：浮点路径也必须是确定的
    let r = Rasterizer::new(23.5);
    let a = r.rasterize(&g, 10);
    let b = r.rasterize(&g, 10);
    assert_eq!(a, b, "同一输入两次光栅化必须逐字段相同");
    assert_eq!(a.coverage, b.coverage, "覆盖率必须逐字节相同");

    let c = Rasterizer::new(23.5).rasterize(&g, 10);
    assert_eq!(a, c, "换一个同参数的 Rasterizer 也要给出相同结果");

    // 曲线字形也验一次
    let circ = glyph(vec![quad_circle(40.0)], 80);
    let ca = r.rasterize(&circ, 100);
    let cb = r.rasterize(&circ, 100);
    assert_eq!(ca, cb);
}

// ───────────────────────── 8. 系统字体端到端 ─────────────────────────

/// 真实字形（Consolas @32px）：'o' 的洞、'l' 的窄、空格的 advance。
///
/// 没有系统字体时**明确跳过**（打印原因与「哪些断言没执行」），不伪装通过。
#[test]
fn system_font_glyphs_are_plausible() {
    let Some((name, data)) = system_font() else {
        eprintln!(
            "[跳过] 本机 %WINDIR%\\Fonts 下没有 consola/arial/segoeui/verdana —— \
             'o' 的洞、'l' 的窄、空格 advance 这 3 组端到端断言**没有执行**（不是通过）"
        );
        return;
    };
    let font = Font::parse(data)
        .unwrap_or_else(|e| panic!("{name} 存在但解析失败：{e}（这是被测代码的问题，不是环境跳过）"));
    let r = Rasterizer::new(32.0);

    // ── 'o'：外圈有墨、中心是洞 ──
    let o = r
        .rasterize_char(&font, 'o')
        .expect("查 'o' 不该报错")
        .expect("'o' 必须有字形");
    assert!(o.width > 4 && o.height > 4, "'o' 的位图不该退化成 {}×{}", o.width, o.height);
    assert_eq!(o.max_coverage(), 255, "'o' 的笔画应当有全实像素");
    let (cx, cy) = (o.width / 2, o.height / 2);
    assert_eq!(
        o.coverage_at(cx, cy),
        0,
        "'o' 的计数器（位图中心）必须是空的 —— nonzero 填充把内轮廓挖成洞"
    );
    assert!(
        (0..o.width).any(|x| o.coverage_at(x, 0) > 0),
        "'o' 最上面一行应当有墨"
    );
    assert!(
        (0..o.width).any(|x| o.coverage_at(x, o.height - 1) > 0),
        "'o' 最下面一行应当有墨"
    );
    assert!(
        (0..cx).any(|x| o.coverage_at(x, cy) > 0),
        "'o' 中心行左侧应当有墨（左笔画）"
    );
    assert!(
        (cx..o.width).any(|x| o.coverage_at(x, cy) > 0),
        "'o' 中心行右侧应当有墨（右笔画）"
    );
    // 中心行上「没有墨」的列数 ≈ 计数器的宽度
    let counter_cols = (0..o.width)
        .filter(|&x| o.coverage_at(x, cy) == 0)
        .count();
    eprintln!(
        "[{name} 'o' @32px] 位图 {}×{} left={} top={} advance={:.2} 墨迹像素={} 中心行空洞宽约 {counter_cols}px",
        o.width,
        o.height,
        o.left,
        o.top,
        o.advance,
        o.ink_pixels(1)
    );

    // ── 'l'：主体（竖干）必须远窄于 advance ──
    //
    // 领队给的原始判据是「'l' 的墨迹宽度 < advance 的 60%」。这条对 consola.ttf **不成立**，
    // 而且不是光栅化器的问题 —— 实测证据（详见给领队的报告）：
    //   ① 字体自己声明的 glyf bbox 是 x=172..977（805 units = 0.393 em），
    //      而 advance = 1126 units ⇒ 这个字形**本身**就有 71.5% advance 宽；
    //   ② 独立对照：用 Windows 自带 GDI+ 加载同一个 consola.ttf 渲染 'l'，ink 宽 13px，
    //      形状同样是「左上横旗 + 右侧竖干 + 整宽底横」（族名确认为 Consolas）；
    //   ③ Consolas 故意把 'l' 做成带左上旗与整宽底横，用来和 '1' / 'I' 区分；
    //      不是所有字体的 'l' 都像 Arial 那样是一根光杆。
    // 所以这里按「窄」的本意断言：**去掉顶旗与底横之后的竖干**必须 < advance 的 60%，
    // 外加一条「全高墨迹不溢出自己的 advance」兜底（避免把判据放宽到什么都测不出）。
    let l = r
        .rasterize_char(&font, 'l')
        .expect("查 'l' 不该报错")
        .expect("'l' 必须有字形");
    assert!(l.advance > 0.0, "'l' 的 advance 应为正");
    assert!(
        l.height >= 8,
        "'l' 太矮（{}px），取中间三分之一量竖干没有意义",
        l.height
    );
    // 有墨的列数（只看 [lo, hi) 这些行）
    let ink_cols = |lo: u32, hi: u32| {
        (0..l.width)
            .filter(|&x| (lo..hi).any(|y| l.coverage_at(x, y) > 0))
            .count()
    };
    let full_ink_cols = ink_cols(0, l.height);
    let stem_cols = ink_cols(l.height / 3, l.height * 2 / 3);
    eprintln!(
        "[{name} 'l' @32px] 位图 {}×{} left={} top={} advance={:.2}px；\
         竖干墨迹 {stem_cols}px（advance 的 {:.0}%）；全高墨迹 {full_ink_cols}px（{:.0}%，含左上旗与整宽底横）",
        l.width,
        l.height,
        l.left,
        l.top,
        l.advance,
        stem_cols as f32 / l.advance * 100.0,
        full_ink_cols as f32 / l.advance * 100.0
    );
    assert!(
        (stem_cols as f32) < l.advance * 0.6,
        "'l' 的竖干宽度 {stem_cols}px 应 < advance {:.2}px 的 60%（{:.2}px）",
        l.advance,
        l.advance * 0.6
    );
    assert!(
        (full_ink_cols as f32) < l.advance,
        "'l' 的全高墨迹 {full_ink_cols}px 不该溢出自己的 advance {:.2}px",
        l.advance
    );

    // ── 空格：没有墨，但 advance 必须保留 ──
    let sp = r
        .rasterize_char(&font, ' ')
        .expect("查空格不该报错")
        .expect("空格必须有字形");
    assert!(sp.is_blank(), "空格不该有墨迹");
    assert!(
        sp.advance > 0.0,
        "空格必须保留 advance（否则排版会塌），实际 {}",
        sp.advance
    );

    // ── cmap 里没有的字符 ⇒ None（不是空位图）──
    assert!(
        r.rasterize_char(&font, '\u{E000}')
            .expect("查询不该报错")
            .is_none(),
        "私用区字符不该有字形"
    );
}

// ═══════════ 亚像素水平定位 / hinting 决策 —— 判据与实测记录 ═══════════

/// FNV-1a 64 位：把 `GlyphImage` 的全部字段（含 advance 的位模式、coverage 逐字节）折成一个数。
fn digest(img: &GlyphImage) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut eat = |b: u8| {
        h ^= b as u64;
        h = h.wrapping_mul(0x100_0000_01b3);
    };
    for v in [
        img.width,
        img.height,
        img.left as u32,
        img.top as u32,
        img.advance.to_bits(),
        img.coverage.len() as u32,
    ] {
        for b in v.to_le_bytes() {
            eat(b);
        }
    }
    for &c in &img.coverage {
        eat(c);
    }
    h
}

/// 合成语料（与机器无关）：**默认路径**的黄金指纹。
///
/// 指纹是在**改代码之前**、在干净的工作树上 dump 出来的（数值与命令见
/// `.superpowers/sdd/hinting-report.md`）。它钉的是本任务最容易翻车的一点：
/// 加了亚像素定位之后，**旧默认路径必须逐字节不变**。
/// 任何改动默认路径采样数学的动作（相位、四舍五入、y 翻转、展平容差、超采样网格）
/// 都会让指纹变红 —— 这是「旧默认不变」的证据，而不是靠人眼比对。
#[test]
fn legacy_default_path_output_is_byte_frozen() {
    // (名字, 字形, upem, ppem, 期望指纹（干净树上 dump）)
    let cases: Vec<(&str, Glyph, u16, f32, u64)> = vec![
        (
            "square_0_10",
            glyph(vec![square_ccw(0.0, 0.0, 10.0)], 10),
            10,
            10.0,
            0x299a_36f0_db60_10a4,
        ),
        (
            "square_half",
            glyph(vec![square_ccw(0.5, 0.5, 10.0)], 10),
            10,
            10.0,
            0xcf5b_572a_35f0_5d25,
        ),
        (
            "hole_opposite",
            glyph(vec![square_ccw(0.0, 0.0, 10.0), square_cw(3.0, 3.0, 4.0)], 10),
            10,
            10.0,
            0xda42_b85e_960f_c1b4,
        ),
        (
            "hole_same",
            glyph(vec![square_ccw(0.0, 0.0, 10.0), square_ccw(3.0, 3.0, 4.0)], 10),
            10,
            10.0,
            0x299a_36f0_db60_10a4,
        ),
        (
            "triangle",
            glyph(vec![polygon(&[(0.0, 0.0), (10.0, 0.0), (5.0, 10.0)])], 10),
            10,
            10.0,
            0xedc4_ebe1_fb62_c99e,
        ),
        (
            "quad_circle",
            glyph(vec![quad_circle(40.0)], 80),
            100,
            40.0,
            0xfc65_05a7_bc36_a53c,
        ),
        (
            "cubic_circle",
            glyph(vec![cubic_circle(40.0)], 80),
            100,
            40.0,
            0xa19c_405f_4219_6cc0,
        ),
        (
            "blank",
            glyph(Vec::new(), 500),
            1000,
            20.0,
            0x0e0f_a0ee_188e_8a42,
        ),
        (
            "odd_ppem",
            glyph(vec![quad_circle(40.0)], 80),
            100,
            23.5,
            0xabf2_8382_2918_0ecc,
        ),
    ];
    let mut checked = 0;
    for (name, g, upem, ppem, want) in cases {
        let img = Rasterizer::new(ppem).rasterize(&g, upem);
        assert_eq!(
            digest(&img),
            want,
            "{name}: 默认路径输出变了（w={} h={} left={} top={} adv={} sum={}）—— \
             新功能必须是新路径，不许动旧数学",
            img.width,
            img.height,
            img.left,
            img.top,
            img.advance,
            img.coverage_sum()
        );
        checked += 1;
    }
    assert_eq!(checked, 9, "前置条件：9 个合成语料都要被检查");
}

/// 模拟「垂直两极网格对齐」：把 y ∈ [y_min, y_max] 线性映射到像素行 [a, b]（a、b 为整数）。
fn grid_fit_glyph_y(g: &Glyph, scale: f32) -> Option<(Glyph, f32)> {
    let (_, y0, _, y1) = g.outline_bbox()?;
    // NaN 也要挡住（`y1 <= y0` 对 NaN 为 false），否则下面的比例会算出 NaN。
    if y1.is_nan() || y0.is_nan() || y1 <= y0 {
        return None;
    }
    let a = (y0 * scale).round();
    let mut b = (y1 * scale).round();
    if b <= a {
        b = a + 1.0;
    }
    let k = (b - a) / (y1 - y0); // 新的「像素/字体单位」比例
    let ratio = k / scale; // 相对原比例的形变
    let m = |p: (f32, f32)| (p.0, a / scale + (p.1 - y0) * k / scale);
    Some((
        Glyph {
            contours: g
                .contours
                .iter()
                .map(|c| Contour {
                    start: m(c.start),
                    segments: c
                        .segments
                        .iter()
                        .map(|s| match *s {
                            Segment::Line { to } => Segment::Line { to: m(to) },
                            Segment::Quad { ctrl, to } => Segment::Quad {
                                ctrl: m(ctrl),
                                to: m(to),
                            },
                            Segment::Cubic { c1, c2, to } => Segment::Cubic {
                                c1: m(c1),
                                c2: m(c2),
                                to: m(to),
                            },
                        })
                        .collect(),
                })
                .collect(),
            bbox: g.bbox,
            advance_width: g.advance_width,
            left_side_bearing: g.left_side_bearing,
        },
        ratio,
    ))
}

/// 墨迹质量（Σ coverage / 255，单位 px²）。
fn ink_mass(img: &GlyphImage) -> f32 {
    img.coverage_sum() as f32 / 255.0
}

/// 部分覆盖（0 < c < 255）的墨迹质量。
fn partial_mass(img: &GlyphImage) -> f32 {
    img.coverage
        .iter()
        .filter(|&&c| c > 0 && c < 255)
        .map(|&c| c as f32 / 255.0)
        .sum()
}

/// coverage 的 x 质心（像素，相对位图左边缘）。
fn centroid_x(img: &GlyphImage) -> f32 {
    let mut num = 0.0f64;
    let mut den = 0.0f64;
    for y in 0..img.height {
        for x in 0..img.width {
            let c = img.coverage_at(x, y) as f64 / 255.0;
            num += c * (x as f64 + 0.5);
            den += c;
        }
    }
    if den == 0.0 { 0.0 } else { (num / den) as f32 }
}

/// 小数笔位置 → (整数落位, 1/4 像素偏移)：量化、**进位**、退化输入。
///
/// 会让这条变红的实现改动：丢掉 `q == 1.0` 的进位（12.9 会被落成 12 + 1.0 → 误差 0.9px）。
#[test]
fn split_subpixel_x_quantizes_and_carries() {
    assert_eq!(
        SUBPIXEL_LEVELS, 4,
        "默认档位是 1/4 像素（与 FreeType / Skia 同量级）"
    );
    // 前置条件：1/4 的倍数是 f32 精确值 ⇒ 可以用逐位相等断言
    assert_eq!(0.25f32 + 0.25 + 0.25, 0.75);

    // 相位就近量化
    assert_eq!(split_subpixel_x(12.0, SUBPIXEL_LEVELS), (12, 0.0));
    assert_eq!(split_subpixel_x(12.1, SUBPIXEL_LEVELS), (12, 0.0));
    assert_eq!(split_subpixel_x(12.25, SUBPIXEL_LEVELS), (12, 0.25));
    assert_eq!(split_subpixel_x(12.5, SUBPIXEL_LEVELS), (12, 0.5));
    assert_eq!(split_subpixel_x(12.6, SUBPIXEL_LEVELS), (12, 0.5));
    assert_eq!(split_subpixel_x(12.75, SUBPIXEL_LEVELS), (12, 0.75));
    assert_eq!(
        split_subpixel_x(12.9, SUBPIXEL_LEVELS),
        (13, 0.0),
        "四舍五入到整数像素时必须进位（否则落位误差接近 1px）"
    );
    // 负数：floor + 相位，仍然满足「整数部分 + 相位 ≈ 原值」
    assert_eq!(split_subpixel_x(-0.25, SUBPIXEL_LEVELS), (-1, 0.75));
    // levels = 1 ⇒ 只能取整；levels = 0 视为 1
    assert_eq!(split_subpixel_x(12.6, 1), (13, 0.0));
    assert_eq!(split_subpixel_x(12.6, 0), (13, 0.0));
    // 非有限输入：确定地退回 (0, 0.0)，不 panic
    for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        assert_eq!(
            split_subpixel_x(bad, SUBPIXEL_LEVELS),
            (0, 0.0),
            "非有限输入 {bad}"
        );
    }

    // 不变式（1000 个采样）：落位误差 ≤ 半个档位 = 1/8 px
    let mut worst = 0.0f32;
    let mut n = 0;
    for i in 0..1000 {
        let pen = -50.0 + i as f32 * 0.1234;
        let (whole, sub) = split_subpixel_x(pen, SUBPIXEL_LEVELS);
        worst = worst.max((whole as f32 + sub - pen).abs());
        n += 1;
    }
    assert_eq!(n, 1000, "前置条件：跑满 1000 个采样");
    assert!(worst <= 0.125 + 1e-6, "落位误差上界是 1/8 px，实测最坏 {worst}");
}

/// `subpixel_x = 0.0` 必须与默认路径**逐字段相同**（新开关不许悄悄改旧数学）。
#[test]
fn rasterize_at_zero_offset_equals_default_path() {
    let cases: Vec<(&str, Glyph, u16, f32)> = vec![
        ("square", glyph(vec![square_ccw(0.0, 0.0, 10.0)], 10), 10, 10.0),
        (
            "hole",
            glyph(vec![square_ccw(0.0, 0.0, 10.0), square_cw(3.0, 3.0, 4.0)], 10),
            10,
            10.0,
        ),
        ("quad_circle", glyph(vec![quad_circle(40.0)], 80), 100, 23.5),
        ("blank", glyph(Vec::new(), 500), 1000, 20.0),
    ];
    let mut checked = 0;
    for (name, g, upem, ppem) in &cases {
        let r = Rasterizer::new(*ppem);
        assert_eq!(
            r.rasterize(g, *upem),
            r.rasterize_at(g, *upem, 0.0),
            "{name}: subpixel_x=0 必须与默认路径逐字段相同（含 coverage 逐字节）"
        );
        checked += 1;
    }
    // -0.0 也必须落到同一条路径
    assert_eq!(
        Rasterizer::new(10.0).rasterize_at(&cases[0].1, 10, -0.0),
        Rasterizer::new(10.0).rasterize_at(&cases[0].1, 10, 0.0)
    );
    checked += 1;

    // 真实字体也验一遍（真实字形的 left/top/advance 路径）
    if let Some((name, data)) = system_font() {
        let font = Font::parse(data).expect("解析");
        let r = Rasterizer::new(16.0);
        for ch in ['l', 'o', 'H'] {
            let a = r.rasterize_char(&font, ch).expect("查").expect("字形");
            let b = r.rasterize_char_at(&font, ch, 0.0).expect("查").expect("字形");
            assert_eq!(
                a, b,
                "{name} '{ch}': 字符路径上 subpixel_x=0 也必须逐字段相同"
            );
            checked += 1;
        }
    } else {
        eprintln!("[部分跳过] 没有系统字体：真实字形那一半断言没有执行（不是通过）");
    }
    assert!(checked >= 5, "前置条件：至少 5 个组合，实际 {checked}");
}

/// 亚像素偏移把墨迹**整体平移**那么多：画布上的质心位移 = 偏移量；墨迹质量守恒。
#[test]
fn subpixel_offset_translates_ink_by_the_fractional_amount() {
    let g = glyph(vec![square_ccw(0.0, 0.0, 10.0)], 10);
    let r = Rasterizer::new(10.0);
    let base = r.rasterize(&g, 10);
    let base_mass = ink_mass(&base);
    let base_canvas = base.left as f32 + centroid_x(&base);
    assert_eq!(base_canvas, 5.0, "前置条件：单位正方形质心在 5.0");

    let mut n = 0;
    for &phi in &[0.25f32, 0.5, 0.75] {
        let img = r.rasterize_at(&g, 10, phi);
        let canvas = img.left as f32 + centroid_x(&img);
        assert!(
            (canvas - (5.0 + phi)).abs() <= 0.01,
            "phi={phi}: 画布上的墨迹质心 {canvas} 应≈{}（平移必须真的发生）",
            5.0 + phi
        );
        assert!(
            (ink_mass(&img) - base_mass).abs() <= 0.1,
            "phi={phi}: 墨迹质量 {} 应≈{base_mass}（解析覆盖率对平移不变，只差超采样量化）",
            ink_mass(&img)
        );
        assert_ne!(img.coverage, base.coverage, "phi={phi}: 平移必须真的改变覆盖率");
        n += 1;
    }
    assert_eq!(n, 3, "前置条件：3 个相位都测了");
    // 0.25 与 0.75 是镜像相位，输出不能相同（否则说明相位被吞掉了）
    assert_ne!(r.rasterize_at(&g, 10, 0.25), r.rasterize_at(&g, 10, 0.75));
    // 相位把左边界推进下一列时，位图允许比默认宽 1px（文档化的代价）
    assert_eq!(r.rasterize_at(&g, 10, 0.25).width, 11);

    // 真实字形：相对质心位移同样 ≈ 相位。
    // 注意：ss×ss 超采样把覆盖率量化到 1/ss²，所以**质心**里有一点量化噪声 ——
    // 默认 ss=4 上实测最坏 ~0.03px。为了证明这确实是量化噪声而不是平移 bug，
    // 同一组断言在 ss=16 上再跑一遍：偏差必须显著变小。
    if let Some((name, data)) = system_font() {
        let font = Font::parse(data).expect("解析");
        let mut worst = [0.0f32, 0.0f32];
        for (idx, (ss, label, tol)) in [(4u32, "默认 ss=4", 0.10f32), (16, "ss=16", 0.06)]
            .into_iter()
            .enumerate()
        {
            let r = Rasterizer::with_supersample(16.0, ss);
            let mut checked = 0;
            let mut local_worst = 0.0f32;
            for ch in ['l', 'o', 'H'] {
                let Some(gi) = font.glyph_index(ch).expect("查") else {
                    continue;
                };
                let g = font.glyph(gi).expect("取");
                let b = r.rasterize(&g, font.units_per_em);
                let bc = b.left as f32 + centroid_x(&b);
                assert!(b.height > 3, "前置条件：'{ch}' 必须有真实高度");
                for &phi in &[0.25f32, 0.5, 0.75] {
                    let img = r.rasterize_at(&g, font.units_per_em, phi);
                    let c = img.left as f32 + centroid_x(&img);
                    let dev = (c - bc - phi).abs();
                    assert!(
                        dev <= tol,
                        "{name} '{ch}' phi={phi} [{label}]: 质心位移偏差 {dev:.4} 应 ≤ {tol}"
                    );
                    local_worst = local_worst.max(dev);
                    checked += 1;
                }
            }
            assert!(checked >= 9, "前置条件：真实字形至少 9 个组合，实际 {checked}");
            worst[idx] = local_worst;
        }
        eprintln!(
            "[质心位移偏差 @16px] {name}: ss=4 最坏 {:.4}px；ss=16 最坏 {:.4}px（细化后必须更小）",
            worst[0], worst[1]
        );
        assert!(
            worst[1] < worst[0],
            "细化超采样必须把质心偏差压小（证明偏差来自覆盖率量化，不是平移算错）：ss=4 {:.4} vs ss=16 {:.4}",
            worst[0],
            worst[1]
        );
    } else {
        eprintln!("[部分跳过] 没有系统字体：真实字形的质心位移断言没有执行（不是通过）");
    }
}

/// 字符路径与字形路径同源；`cmap` 未命中仍然返回 `None`（亚像素不改变这条语义）。
#[test]
fn rasterize_char_at_matches_glyph_path_and_keeps_none_semantics() {
    let Some((name, data)) = system_font() else {
        eprintln!("[跳过] 没有系统字体：字符路径一致性断言没有执行（不是通过）");
        return;
    };
    let font = Font::parse(data).expect("解析");
    let r = Rasterizer::new(16.0);
    let mut n = 0;
    for ch in ['l', 'H', 'o', '.'] {
        let Some(gi) = font.glyph_index(ch).expect("查") else {
            continue;
        };
        let g = font.glyph(gi).expect("取");
        for &phi in &[0.0f32, 0.25, 0.5, 0.75] {
            let a = r
                .rasterize_char_at(&font, ch, phi)
                .expect("查询不该报错")
                .expect("应有字形");
            let b = r.rasterize_at(&g, font.units_per_em, phi);
            assert_eq!(a, b, "{name} '{ch}' phi={phi}: 字符路径必须与字形路径一致");
            n += 1;
        }
    }
    assert!(n >= 16, "前置条件：至少 16 个组合，实际 {n}");
    assert!(
        r.rasterize_char_at(&font, '\u{E000}', 0.25)
            .expect("查询不该报错")
            .is_none(),
        "cmap 未命中在亚像素路径上仍须返回 None（不许退回空位图）"
    );
}

/// 统计落位误差 / 相邻间距误差：`(落位 RMSE, 落位最坏, 间距 RMSE, 间距最坏)`。
fn placement_errors(advance: f32, n: usize, place: impl Fn(f32) -> f32) -> (f64, f64, f64, f64) {
    let (mut se, mut mx, mut gse, mut gmx) = (0.0f64, 0.0f64, 0.0f64, 0.0f64);
    let mut prev = 0.0f32;
    for k in 1..=n {
        let pen = k as f32 * advance;
        let x = place(pen);
        let e = (x - pen) as f64;
        se += e * e;
        mx = mx.max(e.abs());
        if k > 1 {
            let ge = (x - prev - advance) as f64;
            gse += ge * ge;
            gmx = gmx.max(ge.abs());
        }
        prev = x;
    }
    (
        (se / n as f64).sqrt(),
        mx,
        (gse / (n - 1) as f64).sqrt(),
        gmx,
    )
}

/// **清晰度/节奏的前后数字**：小数 advance 下，整数落位 vs 1/4 亚像素落位。
///
/// 判据用**生产代码** `split_subpixel_x`（不是测试里重写一遍量化）：
/// - 整数落位：落位 RMSE ≈ 1/√12 ≈ 0.2887，最坏 0.5px；相邻间距误差最坏可达 ~1px；
/// - 1/4 落位：落位 RMSE ≈ 0.072，最坏 0.125px；相邻间距误差最坏 ≤ 0.25px。
#[test]
fn subpixel_placement_reduces_spacing_error_measured() {
    let mut advs: Vec<(String, f32)> = vec![
        (
            "合成 8.796875（consola@16px 的 advance）".to_string(),
            8.796875,
        ),
        (
            "合成 17.59375（consola@32px 的 advance）".to_string(),
            17.59375,
        ),
    ];
    if let Some((name, data)) = system_font() {
        let font = Font::parse(data).expect("解析");
        let mut seen = 0;
        for px in [16.0f32, 32.0] {
            if let Some(gi) = font.glyph_index('H').expect("查") {
                let a = font.glyph(gi).expect("取").advance_width as f32 * px
                    / font.units_per_em as f32;
                advs.push((format!("{name} 'H'@{px}px 实测 advance {a}"), a));
                seen += 1;
            }
        }
        assert_eq!(seen, 2, "前置条件：系统字体两档 advance 取样");
    } else {
        eprintln!("[部分跳过] 没有系统字体：真实 advance 那两组没有执行（不是通过）");
    }

    let n = 64usize;
    let mut ratios = Vec::new();
    for (label, adv) in &advs {
        let frac = adv.fract();
        assert!(
            frac > 0.05 && frac < 0.95,
            "前置条件：{label} 的 advance 必须有明显小数部分（实测 {adv}）—— 否则这条判据是空话"
        );
        let (ri, mi, gi_, gmi) = placement_errors(*adv, n, |pen| pen.round());
        let (rq, mq, gq, gmq) = placement_errors(*adv, n, |pen| {
            let (whole, sub) = split_subpixel_x(pen, SUBPIXEL_LEVELS);
            // `rasterize_at` 内部会把相位 `rem_euclid(1.0)` 归一，所以「有效落位」是
            // 整数部分 + 归一后的相位 —— 这也正是 `split_subpixel_x` 必须**进位**的原因。
            whole as f32 + sub.rem_euclid(1.0)
        });
        eprintln!(
            "[落位] {label}（n={n}）：整数 RMSE={ri:.4} max={mi:.4} 间距 RMSE={gi_:.4} max={gmi:.4} \
             → 1/4 RMSE={rq:.4} max={mq:.4} 间距 RMSE={gq:.4} max={gmq:.4}（落位 RMSE 改善 {:.2}×）",
            ri / rq
        );
        assert!(ri > 0.25, "整数落位 RMSE 应≈0.2887，实测 {ri}");
        assert!(rq < 0.09, "1/4 落位 RMSE 应≈0.072，实测 {rq}");
        assert!(
            ri / rq >= 3.5,
            "落位 RMSE 至少改善 3.5×，实测 {:.2}×",
            ri / rq
        );
        assert!(mq <= 0.125 + 1e-6, "1/4 落位最坏误差应 ≤ 1/8 px，实测 {mq}");
        assert!(gmq <= 0.25 + 1e-6, "相邻间距误差最坏应 ≤ 1/4 px，实测 {gmq}");
        assert!(
            gmi > 0.5,
            "整数落位的相邻间距误差最坏应 > 0.5px（这正是要修的问题），实测 {gmi}"
        );
        ratios.push(ri / rq);
    }
    assert!(ratios.len() >= 2, "前置条件：至少 2 组 advance");
}

/// **端到端落位忠实性**：真的调光栅化器，而不是在测试里重算算术。
///
/// 正方形（units 0..10、upem=10、ppem=10）的墨迹质心解析上就在 `pen + 5.0`。
/// 用 `split_subpixel_x` + `rasterize_at` 落位后，画布质心与它的偏差必须只剩
/// 覆盖率的量化/抗锯齿噪声（≤ 1/8 px 档位 + 一点）；整数落位则必然有 ≤ 0.5px 的误差。
///
/// 会变红的实现改动：① 相位被吞（`sub_x = 0`）⇒ 偏差退化成整数落位的量级；
/// ② `split_subpixel_x` 丢掉进位 ⇒ 相位 1.0 被 `rasterize_at` 归一成 0，落位差整 1px；
/// ③ `rasterize_at` 里采样原点用 `left` 而不是 `left - sub_x` ⇒ 位图边界动了但墨迹没动。
#[test]
fn subpixel_placement_is_faithful_end_to_end() {
    let g = glyph(vec![square_ccw(0.0, 0.0, 10.0)], 10);
    let r = Rasterizer::new(10.0);
    let adv = 8.796875f32; // 小数 advance（consola @16px 的实测值）
    let n = 64usize;
    let mut worst_sub = 0.0f32;
    let mut worst_int = 0.0f32;
    for k in 0..n {
        let pen = k as f32 * adv;
        // 亚像素：split_subpixel_x + rasterize_at（生产路径）
        let (whole, sub) = split_subpixel_x(pen, SUBPIXEL_LEVELS);
        let img = r.rasterize_at(&g, 10, sub);
        let canvas = whole as f32 + img.left as f32 + centroid_x(&img);
        worst_sub = worst_sub.max((canvas - (pen + 5.0)).abs());
        // 整数：pen.round() + 整数落位位图
        let base = r.rasterize(&g, 10);
        let canvas_i = pen.round() + base.left as f32 + centroid_x(&base);
        worst_int = worst_int.max((canvas_i - (pen + 5.0)).abs());
    }
    eprintln!(
        "[端到端落位] n={n}：亚像素最坏 {worst_sub:.4}px，整数最坏 {worst_int:.4}px（理论整数上界 0.5）"
    );
    assert!(
        worst_sub <= 0.15,
        "亚像素落位偏差应只剩「相位量化到 1/4」的噪声（≤1/8px 再加一点抗锯齿），实测 {worst_sub}"
    );
    assert!(
        worst_int > 0.2,
        "整数落位必须有可见的落位误差（这正是要修的），实测 {worst_int}"
    );
    assert!(
        worst_sub < worst_int / 3.0,
        "亚像素应把端到端落位误差压到整数的 1/3 以下：{worst_sub} vs {worst_int}"
    );
}

/// **诚实记录代价**：亚像素定位把「每字形一个固定相位」变成「按笔位置分布的 4 个相位」，
/// 相位越靠近 0.5，边缘越灰（同样的墨迹摊到两列）⇒ **部分覆盖质量上升**。
///
/// 这条**不**声称清晰度提升：它把「间距正确性」的代价钉成可数的数字，免得报告里
/// 只讲 4× 间距改善、不提边缘变灰。墨迹质量（Σ coverage）必须仍然守恒（< 1% 漂移）。
#[test]
fn subpixel_phase_spread_trades_edge_sharpness_measured() {
    let Some((name, data)) = system_font() else {
        eprintln!("[跳过] 没有系统字体：相位边缘灰阶代价没有测量（不是通过）");
        return;
    };
    let font = Font::parse(data).expect("解析");
    let px = 16.0f32;
    let r = Rasterizer::new(px);
    let mut samples = 0;
    let mut worst_spread = 0.0f32;
    let mut worst_drift = 0.0f32;
    let mut worst_edge_penalty = 0.0f32;
    for ch in ['l', 'H', 'o', 'e'] {
        let Some(gi) = font.glyph_index(ch).expect("查") else {
            continue;
        };
        let g = font.glyph(gi).expect("取");
        let mut ratios = Vec::new();
        let mut masses = Vec::new();
        for &phi in &[0.0f32, 0.25, 0.5, 0.75] {
            let img = r.rasterize_at(&g, font.units_per_em, phi);
            let m = ink_mass(&img);
            assert!(m > 1.0, "前置条件：'{ch}' 必须有墨（phi={phi}，mass={m}）");
            ratios.push(partial_mass(&img) / m);
            masses.push(m);
        }
        let fixed = ratios[0]; // 整数落位 = 该字形的自然相位（phi = 0）
        let mean = ratios.iter().sum::<f32>() / ratios.len() as f32;
        let hi = ratios.iter().cloned().fold(f32::MIN, f32::max);
        let lo = ratios.iter().cloned().fold(f32::MAX, f32::min);
        let drift = masses.iter().cloned().fold(f32::MIN, f32::max)
            / masses.iter().cloned().fold(f32::MAX, f32::min)
            - 1.0;
        eprintln!(
            "[{name} '{ch}' @{px}px] 部分覆盖质量：固定相位 {fixed:.3} → 4 相位均值 {mean:.3}\
             （{:+.1}%），区间 [{lo:.3}, {hi:.3}]；墨迹质量漂移 {:.2}%",
            (mean / fixed - 1.0) * 100.0,
            drift * 100.0
        );
        assert!(
            hi - lo > 0.05,
            "四个相位的边缘灰阶必须真的不同（否则这条「代价」是空话）：'{ch}' 区间 [{lo:.3}, {hi:.3}]"
        );
        assert!(
            drift.abs() < 0.01,
            "墨迹质量对相位守恒（漂移应 <1%），'{ch}' 实测 {:.2}%",
            drift * 100.0
        );
        worst_spread = worst_spread.max(hi - lo);
        worst_drift = worst_drift.max(drift.abs());
        worst_edge_penalty = worst_edge_penalty.max(mean / fixed - 1.0);
        samples += 1;
    }
    assert!(samples >= 3, "前置条件：至少 3 个字形，实际 {samples}");
    eprintln!(
        "[汇总 @{px}px] 相位造成的部分覆盖质量最大摆幅 {worst_spread:.3}；\
         固定相位→4 相位均值的最大代价 {:+.1}%；墨迹质量最大漂移 {:.2}%",
        worst_edge_penalty * 100.0,
        worst_drift * 100.0
    );
}

/// 亚像素路径的确定性：同输入两次逐字节相同；相位按 mod 1 归一；非法输入退回默认相位。
#[test]
fn subpixel_path_is_deterministic() {
    let g = glyph(vec![quad_circle(40.0)], 80);
    let r = Rasterizer::new(23.5);
    let mut n = 0;
    for phi in [
        0.0f32,
        0.25,
        0.5,
        0.75,
        1.0,
        -0.25,
        12.75,
        f32::NAN,
        f32::INFINITY,
    ] {
        let a = r.rasterize_at(&g, 100, phi);
        let b = r.rasterize_at(&g, 100, phi);
        assert_eq!(a, b, "phi={phi}: 两次必须逐字段相同");
        assert_eq!(a.coverage, b.coverage, "phi={phi}: coverage 必须逐字节相同");
        n += 1;
    }
    assert_eq!(n, 9, "前置条件：9 个相位（含非法/超范围）都测了");
    // 归一化：mod 1；非有限 → 0
    assert_eq!(r.rasterize_at(&g, 100, 1.0), r.rasterize_at(&g, 100, 0.0));
    assert_eq!(r.rasterize_at(&g, 100, 12.75), r.rasterize_at(&g, 100, 0.75));
    assert_eq!(r.rasterize_at(&g, 100, -0.25), r.rasterize_at(&g, 100, 0.75));
    assert_eq!(r.rasterize_at(&g, 100, f32::NAN), r.rasterize_at(&g, 100, 0.0));
    assert_eq!(
        r.rasterize_at(&g, 100, f32::INFINITY),
        r.rasterize_at(&g, 100, 0.0)
    );
    // 四个档位必须给出四个不同结果（否则相位没参与采样）
    let q = [0.0f32, 0.25, 0.5, 0.75].map(|p| r.rasterize_at(&g, 100, p));
    assert_ne!(q[0], q[1], "0 与 0.25 档必须不同");
    assert_ne!(q[1], q[2], "0.25 与 0.5 档必须不同");
    assert_ne!(q[2], q[3], "0.5 与 0.75 档必须不同");
}

/// **hinting 决策的实测依据**：为什么这一轮**不交**简化网格对齐（不是半成品，是不划算）。
///
/// 我把最省的 hinting-lite —— 「把字形垂直两极 `y_min`/`y_max` 对齐到整数像素行」——
/// 实现在测试里（[`grid_fit_glyph_y`]，**不进产品路径**）并在 consola 上量了 8 个字号 × 6 个字形：
///
/// - **声称的收益成立**：极值行确实贴到网格上（下面显式断言 `top`/`height` 都是整数）。
/// - **代价**：为了对齐两极必须把整条轮廓垂直缩放 `ratio`（9px 上实测形变 12.5%），
///   这一缩放会把字形**内部**的水平边缘（x-height、横杠）推离网格 ⇒
///   「部分覆盖质量占比」逐例在 −10%..+13% 之间大幅摆动，**均值没有净收益**。
/// - 结论：极值对齐不是「文本清晰度」的可数改善；真正的 TrueType hinting 需要指令虚拟机 +
///   stem 识别 + CVT，是另一个量级的工程 ⇒ **本轮登记不做**，并把这份数据留成决策记录。
#[test]
fn simplified_vertical_extent_gridfit_is_not_shipped_measured() {
    let Some((name, data)) = system_font() else {
        eprintln!("[跳过] 没有系统字体：hinting 决策的实测依据没有采集（不是通过）");
        return;
    };
    let font = Font::parse(data).expect("解析");
    let mut deltas = Vec::new();
    let mut max_distortion = 0.0f32;
    let mut extremes_aligned = 0usize;
    for px in [9.0f32, 10.0, 11.0, 12.0, 13.0, 16.0, 20.0, 32.0] {
        let scale = px / font.units_per_em as f32;
        let r = Rasterizer::new(px);
        for ch in ['x', 'H', 'o', 'e', 'n', 'g'] {
            let Some(gi) = font.glyph_index(ch).expect("查") else {
                continue;
            };
            let g = font.glyph(gi).expect("取");
            let before = r.rasterize(&g, font.units_per_em);
            let (gf, ratio) = grid_fit_glyph_y(&g, scale).expect("前置条件：字形有 bbox");
            let after = r.rasterize(&gf, font.units_per_em);

            // 声称的收益：极值行确实贴到整数像素行。
            // 注意要走一遍 f32 往返（字体单位 → 像素），所以只能断到 1e-3 px；
            // `after.top` 偶尔会比整数多 1（恰好 ceil 到一个 1e-6 的余量上），故允许 ±1。
            let (_, ny0, _, ny1) = gf.outline_bbox().expect("前置条件：变换后仍有轮廓");
            let (ny0_px, ny1_px) = (ny0 * scale, ny1 * scale);
            assert!(
                (ny1_px - ny1_px.round()).abs() < 1e-3,
                "{name} '{ch}'@{px}px: 对齐后顶边必须落在整数像素行（实测小数部分 {:.6}）",
                ny1_px.fract()
            );
            assert!(
                (ny0_px - ny0_px.round()).abs() < 1e-3,
                "{name} '{ch}'@{px}px: 对齐后底边必须落在整数像素行（实测小数部分 {:.6}）",
                ny0_px.fract()
            );
            assert!(
                (after.top as f32 - ny1_px.round()).abs() <= 1.0,
                "{name} '{ch}'@{px}px: 对齐后 `top`({}) 应≈整数行 {}",
                after.top,
                ny1_px.round()
            );
            extremes_aligned += 1;

            let b = partial_mass(&before) / ink_mass(&before).max(1e-6);
            let a = partial_mass(&after) / ink_mass(&after).max(1e-6);
            deltas.push(a - b);
            max_distortion = max_distortion.max((ratio - 1.0).abs());
        }
    }
    assert!(
        deltas.len() >= 40,
        "前置条件：样本数应 ≥ 40（8 字号 × 6 字形），实际 {}",
        deltas.len()
    );
    assert_eq!(extremes_aligned, deltas.len(), "前置条件：每个样本都验了极值对齐");
    let mean = deltas.iter().sum::<f32>() / deltas.len() as f32;
    let lo = deltas.iter().cloned().fold(f32::MAX, f32::min);
    let hi = deltas.iter().cloned().fold(f32::MIN, f32::max);
    eprintln!(
        "[hinting 决策 @{name}] 极值网格对齐：部分覆盖质量占比逐例变化 均值 {:+.1}%、区间 [{:+.1}%, {:+.1}%]；\
         最大垂直形变 {:.1}%；样本 {}（极值对齐 {extremes_aligned} 例全部成立）",
        mean * 100.0,
        lo * 100.0,
        hi * 100.0,
        max_distortion * 100.0,
        deltas.len()
    );
    assert!(
        mean.abs() <= 0.02,
        "若这条红了说明极值网格对齐现在有净收益了，应重新评估是否实现：实测均值 {:+.1}%（区间 [{:+.1}%, {:+.1}%]）",
        mean * 100.0,
        lo * 100.0,
        hi * 100.0
    );
    assert!(
        hi - lo >= 0.10,
        "前置条件：这条「不划算」的结论建立在逐例大幅摆动上（实测摆幅 {:.1}%）",
        (hi - lo) * 100.0
    );
    assert!(
        max_distortion >= 0.05,
        "前置条件：极值对齐必须真的带来可见的形变（实测最大 {:.1}%）",
        max_distortion * 100.0
    );
    assert!(
        deltas.len() == 48,
        "前置条件：consola 上 8 字号 × 6 字形都应有字形，实际 {}",
        deltas.len()
    );
}
