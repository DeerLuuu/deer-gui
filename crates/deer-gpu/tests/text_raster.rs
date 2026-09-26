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

use deer_gpu::font::{Contour, Font, Glyph, Segment};
use deer_gpu::glyph::GlyphImage;
use deer_gpu::raster::Rasterizer;

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
