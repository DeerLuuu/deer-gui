//! **字形光栅化**（M4）：把 `font::Glyph` 的轮廓变成像素覆盖率图。
//!
//! 由 teammate `raster-dev` 实现（见共享任务 t1）。契约见 [`crate::glyph::GlyphImage`]。
//!
//! ## 流水线
//!
//! 1. **缩放**：`scale = ppem / units_per_em`（font units → 像素）。
//! 2. **Y 轴翻转**：字体 units 的 y 向上、位图行向下，故
//!    `px = x * scale - left`、`py = top - y * scale`。
//! 3. **展平**：`Quad` / `Cubic` 用 de Casteljau 递归细分成折线，平坦度阈值
//!    `FLATNESS_TOLERANCE`（0.25 像素）、深度上限 `MAX_FLATTEN_DEPTH`；`Line` 直接取端点。
//! 4. **填充**：**nonzero winding**（TrueType 规范的填充规则）。逐采样点对全部边求环绕数，
//!    `!= 0` 视为实心 —— 所以「外轮廓与内轮廓方向相反」时自然出现洞（见 `tests/text_raster.rs`）。
//! 5. **超采样**：像素内 ss×ss 网格采样，覆盖率 = `命中数 * 255 / ss²`（四舍五入到 `u8`）。
//!    `ss == 1` 时覆盖率只可能是 0 或 255。
//!
//! ## 落位与紧致
//!
//! `left = floor(x_min * scale)`、`top = ceil(y_max * scale)`、
//! `width = ceil(x_max * scale) - left`、`height = top - floor(y_min * scale)`。
//! 这里的 bbox 取**全部轮廓点（含控制点）**的包围盒（[`crate::font::Glyph::outline_bbox`]）：
//! 贝塞尔曲线必落在控制点凸包内，所以它是真实曲线 bbox 的**超集** ⇒ 位图不会裁掉墨迹，
//! 代价是可能比真实墨迹宽不到 1 像素（不做精确曲线极值求解）。
//!
//! ## 诚实的边界（**没有**做的事）
//!
//! - **不做 hinting**：不读 `glyf` 的 instructions，也不做像素网格拟合。小字号清晰度靠
//!   超采样抗锯齿，不靠字形指令。
//! - **不做亚像素水平定位**：字形按**整数像素**落位（`left` 是整数），一行内相邻字形的
//!   间距只有整数像素精度。次像素定位要把 `left` 拆成整数部分 + 小数位移，属于合成阶段。
//! - **不支持 CFF / OpenType-CFF 轮廓**：`font.rs` 遇到 `OTTO` 直接报错，本模块只处理
//!   `glyf` 的直线 / 二次 / 三次段。
//! - **不做精确曲线极值求解**：bbox 用控制点包围盒（见上）。
//! - **采样点正好落在轮廓边上**时按环绕数算法的朝向约定判定（不算穿越）；结果确定，
//!   但不保证与解析面积逐位一致。
//! - `ppem <= 0`、`units_per_em == 0`、非有限缩放、位图单边超过 `MAX_BITMAP_DIM`
//!   ⇒ 返回 `GlyphImage::blank(0.0)`（**不 panic**，也不尝试巨额分配）。

use crate::error::GpuResult;
use crate::font::{Font, Glyph, Segment};
use crate::glyph::GlyphImage;

/// 像素空间里的一个点。
type Point = (f32, f32);
/// 多边形的一条边（环绕数填充用）。
type Edge = (Point, Point);

/// 贝塞尔展平的平坦度阈值（像素）。
///
/// 0.25 像素约等于四分之一灰阶，比超采样的量化误差（1/16 覆盖率 ≈ 0.06 灰阶）同量级，
/// 再细就只是白烧 CPU。
const FLATNESS_TOLERANCE: f32 = 0.25;

/// 展平递归的最大深度。
///
/// 防护：病态控制点（例如三点几乎共线但永不满足平坦度）不能让递归无限下去。
/// 16 层对应单段最多 2^16 个子段，实际曲线 3~5 层就收敛。
const MAX_FLATTEN_DEPTH: u32 = 16;

/// 超采样倍数的内部上限。
///
/// `supersample` 是公开字段，误设成 `u32::MAX` 会让采样循环与 `ss²` 一起爆掉；
/// 超过上限时按 64 处理（64² = 4096 采样/像素，已远超肉眼分辨力）。
const MAX_SUPERSAMPLE: u32 = 64;

/// 位图单边像素上限。
///
/// `ppem` 设成天文数字（或 units_per_em 极小）时不该去分配 TB 级缓冲：超过上限直接给
/// 空位图。4096 远大于任何真实 UI 字号。
const MAX_BITMAP_DIM: u32 = 4096;

/// 视为「同一个点」的距离平方阈值（像素）：用来丢掉零长边。
const DEGENERATE_LEN2: f32 = 1e-12;

/// 字形光栅化器：把字体 units 的轮廓变成像素覆盖率图。
///
/// 无状态、可复用（`rasterize` 取 `&self`），同一输入的结果**逐字段确定**。
#[derive(Debug, Clone)]
pub struct Rasterizer {
    /// 目标字号（像素 / em）。`<= 0`（含 NaN）时输出空位图。
    pub ppem: f32,
    /// 每个像素内的采样倍率（ss×ss）。默认 4 ⇒ 16 采样/像素。
    pub supersample: u32,
}

impl Rasterizer {
    /// 默认超采样：4×4 = 16 采样/像素。
    pub fn new(ppem: f32) -> Rasterizer {
        Rasterizer {
            ppem,
            supersample: 4,
        }
    }

    /// 指定超采样倍率；`supersample` 至少为 1。
    pub fn with_supersample(ppem: f32, supersample: u32) -> Rasterizer {
        Rasterizer {
            ppem,
            supersample: supersample.max(1),
        }
    }

    /// 光栅化一个字形（`units_per_em` 来自 `head.unitsPerEm`）。
    ///
    /// 退化输入的定义（都有测试）：
    /// - `ppem <= 0` / `ppem` 是 NaN / `units_per_em == 0` ⇒ `blank(0.0)`；
    /// - 无轮廓字形 ⇒ `blank(advance)`（advance 按 `ppem / units_per_em` 缩放后保留）；
    /// - 轮廓退化到零宽或零高（单点、水平/垂直直线）⇒ `blank(advance)`。
    pub fn rasterize(&self, g: &Glyph, units_per_em: u16) -> GlyphImage {
        // NaN 也要挡住（`NaN <= 0.0` 是 false，所以要显式判 NaN）。
        // 注意别写成 `!(ppem > 0.0)`：`clippy::neg_cmp_op_on_partial_ord` 会红。
        if units_per_em == 0 || self.ppem.is_nan() || self.ppem <= 0.0 {
            return GlyphImage::blank(0.0);
        }
        let scale = self.ppem / units_per_em as f32;
        if !scale.is_finite() || scale <= 0.0 {
            return GlyphImage::blank(0.0);
        }
        let advance = g.advance_width as f32 * scale;
        if !advance.is_finite() {
            return GlyphImage::blank(0.0);
        }
        if g.contours.is_empty() {
            return GlyphImage::blank(advance);
        }
        let Some((x0, y0, x1, y1)) = g.outline_bbox() else {
            return GlyphImage::blank(advance);
        };
        if !(x0.is_finite() && y0.is_finite() && x1.is_finite() && y1.is_finite()) {
            return GlyphImage::blank(advance);
        }

        // ── 紧致位图范围（见模块文档「落位与紧致」）──
        let left = (x0 * scale).floor();
        let top = (y1 * scale).ceil();
        let width = (x1 * scale).ceil() - left;
        let height = top - (y0 * scale).floor();
        if width < 1.0 || height < 1.0 {
            // 零宽/零高：没有可着墨的像素。
            return GlyphImage::blank(advance);
        }
        if width > MAX_BITMAP_DIM as f32 || height > MAX_BITMAP_DIM as f32 {
            return GlyphImage::blank(advance);
        }
        let w = width as u32;
        let h = height as u32;
        let (left_i, top_i) = (left as i32, top as i32);

        // ── 轮廓 → 像素空间折线 → 边表 ──
        let mut edges: Vec<Edge> = Vec::new();
        for c in &g.contours {
            let start = to_bitmap(c.start, scale, left, top);
            let mut pts: Vec<Point> = vec![start];
            let mut cur = start;
            for seg in &c.segments {
                match *seg {
                    Segment::Line { to } => {
                        let p = to_bitmap(to, scale, left, top);
                        push_distinct(&mut pts, p);
                        cur = p;
                    }
                    Segment::Quad { ctrl, to } => {
                        let ctrl_px = to_bitmap(ctrl, scale, left, top);
                        let p = to_bitmap(to, scale, left, top);
                        flatten_quad(cur, ctrl_px, p, 0, &mut pts);
                        cur = p;
                    }
                    Segment::Cubic { c1, c2, to } => {
                        let c1_px = to_bitmap(c1, scale, left, top);
                        let c2_px = to_bitmap(c2, scale, left, top);
                        let p = to_bitmap(to, scale, left, top);
                        flatten_cubic(cur, c1_px, c2_px, p, 0, &mut pts);
                        cur = p;
                    }
                }
            }
            // 按环取边：最后一点到首点的闭合边也在这里补上（不依赖轮廓自己闭合）。
            if pts.len() >= 3 {
                for i in 0..pts.len() {
                    let a = pts[i];
                    let b = pts[(i + 1) % pts.len()];
                    if dist2(a, b) > DEGENERATE_LEN2 {
                        edges.push((a, b));
                    }
                }
            }
        }
        if edges.is_empty() {
            return GlyphImage::blank(advance);
        }

        // ── 超采样：ss×ss 网格，覆盖率 = round(命中数 * 255 / ss²) ──
        let ss = self.supersample.clamp(1, MAX_SUPERSAMPLE);
        let inv = 1.0 / ss as f32;
        let total = ss * ss;
        let half = total / 2;
        let w_usize = w as usize;
        let mut coverage = vec![0u8; w_usize * (h as usize)];
        for (row, row_slice) in coverage.chunks_exact_mut(w_usize).enumerate() {
            for (col, slot) in row_slice.iter_mut().enumerate() {
                let mut hits = 0u32;
                for sy in 0..ss {
                    let py = row as f32 + (sy as f32 + 0.5) * inv;
                    for sx in 0..ss {
                        let px = col as f32 + (sx as f32 + 0.5) * inv;
                        if winding_number(&edges, px, py) != 0 {
                            hits += 1;
                        }
                    }
                }
                // hits == total ⇒ (total*255 + total/2) / total == 255，不会溢出 u8。
                *slot = ((hits * 255 + half) / total) as u8;
            }
        }

        GlyphImage::new(w, h, left_i, top_i, advance, coverage)
    }

    /// 字符 → 字形 → 覆盖率图。
    ///
    /// `cmap` 里没有这个字符时返回 `Ok(None)`（**不是** `.notdef` 的空位图，
    /// 那样调用方无法区分「缺字符」与「真的没墨」）。字形解析出错才返回 `Err`。
    pub fn rasterize_char(&self, font: &Font, ch: char) -> GpuResult<Option<GlyphImage>> {
        let Some(glyph_index) = font.glyph_index(ch)? else {
            return Ok(None);
        };
        let g = font.glyph(glyph_index)?;
        Ok(Some(self.rasterize(&g, font.units_per_em)))
    }
}

/// font units → 位图像素坐标（含 **y 轴翻转**）。
///
/// `left` / `top` 是位图左上角在「像素坐标系」里的位置：`left` 相对笔位置、
/// `top` 相对基线向上。翻转后位图第 0 行是字形最高处。
fn to_bitmap(p: Point, scale: f32, left: f32, top: f32) -> Point {
    (p.0 * scale - left, top - p.1 * scale)
}

/// 两点距离的平方。
fn dist2(a: Point, b: Point) -> f32 {
    let dx = a.0 - b.0;
    let dy = a.1 - b.1;
    dx * dx + dy * dy
}

/// 线段中点。
fn mid(a: Point, b: Point) -> Point {
    ((a.0 + b.0) * 0.5, (a.1 + b.1) * 0.5)
}

/// 追加一个点，但丢掉与上一个点重合的（零长边没有几何意义，还会浪费环绕数计算）。
fn push_distinct(pts: &mut Vec<Point>, p: Point) {
    if let Some(&last) = pts.last() {
        if dist2(last, p) <= DEGENERATE_LEN2 {
            return;
        }
    }
    pts.push(p);
}

/// 点 `p` 到直线 `a-b` 的垂直距离（退化线段则退化为点距）。
///
/// 这就是贝塞尔的「平坦度」度量：控制点离弦越远，曲线越弯。
fn point_line_dist(p: Point, a: Point, b: Point) -> f32 {
    let dx = b.0 - a.0;
    let dy = b.1 - a.1;
    let len2 = dx * dx + dy * dy;
    if len2 <= DEGENERATE_LEN2 {
        return dist2(a, p).sqrt();
    }
    (dx * (p.1 - a.1) - dy * (p.0 - a.0)).abs() / len2.sqrt()
}

/// 二次贝塞尔递归展平：够平就把终点交出去，否则在 `t = 0.5` 处切分。
fn flatten_quad(p0: Point, ctrl: Point, p1: Point, depth: u32, out: &mut Vec<Point>) {
    if depth >= MAX_FLATTEN_DEPTH || point_line_dist(ctrl, p0, p1) <= FLATNESS_TOLERANCE {
        push_distinct(out, p1);
        return;
    }
    let p01 = mid(p0, ctrl);
    let p12 = mid(ctrl, p1);
    let m = mid(p01, p12);
    flatten_quad(p0, p01, m, depth + 1, out);
    flatten_quad(m, p12, p1, depth + 1, out);
}

/// 三次贝塞尔递归展平（de Casteljau 在 `t = 0.5` 处切分）。
///
/// TrueType 里没有三次段，这里是为 CFF / 其它轮廓源预留的路径（目前只有合成测试走到）。
fn flatten_cubic(p0: Point, c1: Point, c2: Point, p1: Point, depth: u32, out: &mut Vec<Point>) {
    let flat = depth >= MAX_FLATTEN_DEPTH
        || (point_line_dist(c1, p0, p1) <= FLATNESS_TOLERANCE
            && point_line_dist(c2, p0, p1) <= FLATNESS_TOLERANCE);
    if flat {
        push_distinct(out, p1);
        return;
    }
    let p01 = mid(p0, c1);
    let p12 = mid(c1, c2);
    let p23 = mid(c2, p1);
    let p012 = mid(p01, p12);
    let p123 = mid(p12, p23);
    let m = mid(p012, p123);
    flatten_cubic(p0, p01, p012, m, depth + 1, out);
    flatten_cubic(m, p123, p23, p1, depth + 1, out);
}

/// 环绕数（Dan Sunday 的 winding number）：向右射线与各边的**带符号**穿越计数。
///
/// 返回 `0` 表示在轮廓外；非 0 表示在轮廓内（nonzero 规则）。
/// 边 `a → b` 在点 `p` 左边时贡献 +1（向上穿越）或 -1（向下穿越）。
/// 水平边因 `y` 判定不满足而自然被忽略；采样点正好落在边上时按本函数的朝向约定判定，
/// 结果是确定的（这也是「同一输入两次结果相同」的一部分）。
fn winding_number(edges: &[Edge], px: f32, py: f32) -> i32 {
    let mut wn = 0i32;
    for &(a, b) in edges {
        if a.1 <= py {
            if b.1 > py && is_left(a, b, px, py) > 0.0 {
                wn += 1;
            }
        } else if b.1 <= py && is_left(a, b, px, py) < 0.0 {
            wn -= 1;
        }
    }
    wn
}

/// `p` 是否在有向边 `a → b` 的左侧（> 0 在左，< 0 在右，== 0 共线）。
fn is_left(a: Point, b: Point, px: f32, py: f32) -> f32 {
    (b.0 - a.0) * (py - a.1) - (px - a.0) * (b.1 - a.1)
}
