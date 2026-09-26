//! **字形流水线的共享数据类型**（M4）：光栅化产物、图集键、图集槽位。
//!
//! 为什么单独一个模块：光栅化（`raster`）、图集（`atlas`）、度量（`measure`）、
//! 后端（`null` / `deer-vk`）都要碰这几个类型。把它们放在这里，
//! **生产者与消费者只依赖一份定义**，不存在「两处各写一个 bitmap 结构最后对不上」。
//!
//! 坐标约定（**贯穿全项目，改这里等于改语义**）：
//!
//! ```text
//!                         ┌──────────────┐ ← 位图顶边 = 基线 - top（top 向上为正）
//!   基线 ─────────────────┼──────────────┼─────→ 笔位置向右推进 advance
//!                         └──────────────┘
//!                         ↑
//!              位图左边缘 = 笔位置 + left（通常 ≥ 0；斜体/悬垂才为负）
//! ```
//!
//! - `coverage` 是 **8 位覆盖率**（0 = 全空，255 = 全实），不是颜色；
//!   颜色由使用方给（这样同一张字形位图能给任意颜色用，也能进 GPU 图集）。
//! - 位图是**紧致**的（宽高正好包住墨迹），所以「位图尺寸」不等于「advance」。

/// 一个已光栅化的字形位图（覆盖率图）。
///
/// 不变式：`coverage.len() == width * height`（越界访问是编程错误，用 [`GlyphImage::coverage_at`] 安全取）。
#[derive(Debug, Clone, PartialEq)]
pub struct GlyphImage {
    /// 位图宽（像素）。
    pub width: u32,
    /// 位图高（像素）。
    pub height: u32,
    /// 位图左边缘相对**笔位置**的水平偏移（像素）。**通常 ≥ 0**：多数字体的左边距（`lsb`）为正，
    /// 所以 `left = floor(lsb * scale)` 落在笔位置右侧。只有斜体 / 悬垂字形（`j`、`f` 的斜体等）
    /// 才会是负的（向左探出）。实测（consola.ttf @32px）：`'.'`=6、`'i'`=2、`'W'`=0，没有一个为负。
    pub left: i32,
    /// 位图上边缘相对**基线**的垂直偏移（像素，**向上为正**）。
    ///
    /// 绘制时的像素行是 `y = baseline_y - top`，即「基线往上抬 `top` 行」。
    /// 下伸部（如 `g` 的尾巴）会让 `top` 变小甚至为负。
    pub top: i32,
    /// 前进宽度（像素，已按字号缩放）。**与位图宽度无关**：空格没有位图但有 advance。
    pub advance: f32,
    /// 覆盖率，行优先，`0..=255`。
    pub coverage: Vec<u8>,
}

impl GlyphImage {
    /// 一个没有墨迹的字形（空格、无轮廓字形）：位图为空，但**保留 advance**。
    pub fn blank(advance: f32) -> GlyphImage {
        GlyphImage {
            width: 0,
            height: 0,
            left: 0,
            top: 0,
            advance,
            coverage: Vec::new(),
        }
    }

    /// 直接构造（光栅化器用；会 `debug_assert` 长度不变式）。
    pub fn new(
        width: u32,
        height: u32,
        left: i32,
        top: i32,
        advance: f32,
        coverage: Vec<u8>,
    ) -> GlyphImage {
        debug_assert_eq!(
            coverage.len(),
            (width as usize) * (height as usize),
            "覆盖率缓冲长度必须等于 width*height"
        );
        GlyphImage {
            width,
            height,
            left,
            top,
            advance,
            coverage,
        }
    }

    /// 位图是否为「无墨迹」（宽或高为 0，或全部采样点为 0）。
    pub fn is_blank(&self) -> bool {
        self.width == 0 || self.height == 0 || self.coverage.iter().all(|&c| c == 0)
    }

    /// 取覆盖率（越界返回 `0`，不 panic —— 后端贴图时代码路径不该因一个越界就崩）。
    pub fn coverage_at(&self, x: u32, y: u32) -> u8 {
        if x >= self.width || y >= self.height {
            return 0;
        }
        self.coverage[(y as usize) * (self.width as usize) + (x as usize)]
    }

    /// 覆盖率总和（诊断 / 断言用：与「墨迹面积」成正比）。
    pub fn coverage_sum(&self) -> u64 {
        self.coverage.iter().map(|&c| c as u64).sum()
    }

    /// 最大覆盖率（全实心字形应当是 255）。
    pub fn max_coverage(&self) -> u8 {
        self.coverage.iter().copied().max().unwrap_or(0)
    }

    /// 覆盖率 ≥ `threshold` 的像素数（「墨迹像素」）。
    pub fn ink_pixels(&self, threshold: u8) -> usize {
        self.coverage.iter().filter(|&&c| c >= threshold).count()
    }

    /// 单位面积的平均覆盖率（**与 `coverage` 同一量纲：`0.0..=255.0`**）。
    ///
    /// 注意不是 `0.0..=1.0`：满覆盖位图给的是 `255.0`。
    /// 想要比例请自己除以 255（如 `mean_coverage() / 255.0`）。
    pub fn mean_coverage(&self) -> f32 {
        if self.width == 0 || self.height == 0 {
            return 0.0;
        }
        self.coverage_sum() as f32 / ((self.width as u64) * (self.height as u64)) as f32
    }
}

/// 图集里的字形身份：**同一个字形在不同字号下是两条记录**。
///
/// `px_size` 是**取整后的字号**（像素）：字号连续变化时不该无限膨胀图集，
/// 取整等价于「按字号分桶缓存」，这也是所有位图字体缓存的通行做法。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct GlyphKey {
    /// 字形索引（**不是**字符：多个字符可映射到同一字形）。
    pub glyph_index: u16,
    /// 取整后的字号（像素）。`0` 视为 `1`。
    pub px_size: u16,
}

impl GlyphKey {
    pub fn new(glyph_index: u16, px_size: u16) -> GlyphKey {
        GlyphKey {
            glyph_index,
            px_size: px_size.max(1),
        }
    }
}

/// 图集里分配给某个字形的位置（图集左上角为原点，单位像素）。
///
/// 不变式：`x + w <= 图集宽`、`y + h <= 图集高`、任意两个槽位**不重叠**。
/// `get()` 回来的数据长度恒为 `w * h`。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AtlasSlot {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

impl AtlasSlot {
    pub fn right(&self) -> u32 {
        self.x + self.w
    }
    pub fn bottom(&self) -> u32 {
        self.y + self.h
    }
    /// 两个槽位是否重叠（图集不变式的检查入口；空槽位永不重叠）。
    pub fn overlaps(&self, other: &AtlasSlot) -> bool {
        if self.w == 0 || self.h == 0 || other.w == 0 || other.h == 0 {
            return false;
        }
        self.x < other.right() && other.x < self.right() && self.y < other.bottom() && other.y < self.bottom()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coverage_access_is_bounds_safe() {
        let img = GlyphImage::new(2, 2, 0, 0, 5.0, vec![0, 1, 2, 255]);
        assert_eq!(img.coverage_at(1, 1), 255);
        assert_eq!(img.coverage_at(2, 0), 0, "越界必须回 0 而不是 panic");
        assert_eq!(img.coverage_sum(), 258);
        assert_eq!(img.max_coverage(), 255);
    }

    #[test]
    fn blank_glyph_keeps_advance() {
        let img = GlyphImage::blank(4.0);
        assert!(img.is_blank());
        assert_eq!(img.advance, 4.0);
        assert_eq!(img.mean_coverage(), 0.0);
    }

    #[test]
    fn slot_overlap_is_symmetric_and_edge_exact() {
        let a = AtlasSlot { x: 0, y: 0, w: 8, h: 8 };
        let touching = AtlasSlot { x: 8, y: 0, w: 4, h: 4 };
        let inside = AtlasSlot { x: 4, y: 4, w: 4, h: 4 };
        assert!(!a.overlaps(&touching), "边界相接不算重叠");
        assert!(!touching.overlaps(&a));
        assert!(a.overlaps(&inside));
        assert!(inside.overlaps(&a), "重叠判定必须对称");
    }

    #[test]
    fn glyph_key_never_stores_zero_size() {
        assert_eq!(GlyphKey::new(3, 0).px_size, 1);
    }
}
