//! **文字引擎**（M4）：把「字体 + 字号 + 字符串」变成「图集里的字形位图 + 排版偏移」。
//!
//! 这一层是 M4 三条链的**汇合点**：
//!
//! ```text
//!   font（解析）──┐
//!                 ├─→ TextEngine ─→ 后端（CPU 采样贴图 / GPU 上传图集）
//!   raster（光栅化）┤
//!   atlas（打包）──┘
//! ```
//!
//! ## 三条设计决定（都是为了让「布局」与「渲染」不可能对不上）
//!
//! 1. **单一真相是图集**：字形位图只存一份（在 [`GlyphAtlas`] 里），
//!    [`GlyphPlacement`] 只记「在图集的哪个槽位 + 排版偏移 + advance」。
//!    这样 GPU 侧要上传的就是同一张图集，CPU 侧采样也是同一张 —— 不存在两处各存一份导致不一致。
//! 2. **按字号分桶**：[`GlyphKey::px_size`] 是取整后的字号。字号连续变化不会把图集撑爆，
//!    代价是「12.4px 与 12.6px 共用同一张位图」（位图字体的通行做法）。
//! 3. **度量与渲染共用同一个字体对象**：`measure()` 给出的 `FontMeasure` 与 `glyph()`
//!    用的是同一份 `Font`，所以「布局算出的宽度」与「画出来的宽度」不会漂。
//!
//! ## 诚实边界
//!
//! - **不做亚像素水平定位**：advance 是浮点的，但字形位图按整数像素落位
//!   （`left`/`top` 由光栅化器取整）。小字号下这会让间距有 ±1px 的抖动。
//! - **不做 hinting**：抗锯齿靠超采样，不是字体自带的指令。
//! - **不做字距 / 连字 / 替换**（GSUB/GPOS 未实现）：宽度就是 advance 之和。
//! - **不做多字体回退**：一个引擎只有一份字体；缺字（cmap 未命中）**回退到 glyph 0（`.notdef`）**
//!   画出「豆腐块」并计入 `missing_glyphs()`，**不会**去别的字体里找。这样「布局算的宽度」
//!   与「画出来的宽度」在缺字时仍然一致（两边都按 `.notdef` 的 advance 算）。
//! - **图集不淘汰**：只增不减（`GlyphAtlas` 没有 LRU）。长会话里字号种类很多时会持续增长。

use std::collections::HashMap;

use crate::atlas::GlyphAtlas;
use crate::error::{GpuError, GpuResult};
use crate::font::Font;
use crate::glyph::{AtlasSlot, GlyphKey};
use crate::measure::{FontMeasure, find_system_font};
use crate::raster::Rasterizer;

/// 字形在排版时的落位信息：图集槽位 + 相对笔位置的偏移 + 前进宽度。
///
/// `Copy` 是刻意的：排版循环里会大量按值传递它，避免借用整个引擎。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GlyphPlacement {
    /// 位图在图集里的位置（`slot.w`/`slot.h` 为 0 表示这个字形没有墨迹，如空格）。
    pub slot: AtlasSlot,
    /// 位图左边缘相对**笔位置**的水平偏移（像素）。
    pub left: i32,
    /// 位图顶边相对**基线**的垂直偏移（像素，向上为正）：绘制行 = `baseline - top`。
    pub top: i32,
    /// 前进宽度（像素）：画完这个字形，笔位置往右移这么多。
    pub advance: f32,
}

/// 文字引擎：一份字体 + 一张图集 + 每个已光栅化字形的排版信息。
pub struct TextEngine {
    font: Font,
    /// 默认字号（`measure()` 用它；每个字形可以单独指定 `px_size`）。
    font_size: f32,
    atlas: GlyphAtlas,
    placements: HashMap<GlyphKey, GlyphPlacement>,
    missing: usize,
}

/// 图集初始宽度（高度按需增长，见 [`GlyphAtlas::new`]）。
///
/// 512 是权衡：太小 ⇒ 频繁增高；太大 ⇒ 空图集也要传 512×512 的纹理（GPU 侧）。
const ATLAS_WIDTH: u32 = 512;

impl TextEngine {
    /// 从已解析字体建引擎。
    pub fn from_font(font: Font, font_size: f32) -> TextEngine {
        TextEngine {
            font,
            font_size: font_size.max(1.0),
            atlas: GlyphAtlas::new(ATLAS_WIDTH),
            placements: HashMap::new(),
            missing: 0,
        }
    }

    /// 从字体文件字节建引擎。
    pub fn from_font_bytes(data: Vec<u8>, font_size: f32) -> GpuResult<TextEngine> {
        Ok(TextEngine::from_font(Font::parse(data)?, font_size))
    }

    /// 从字体文件路径建引擎。
    pub fn from_font_file(path: &std::path::Path, font_size: f32) -> GpuResult<TextEngine> {
        let data = std::fs::read(path).map_err(|e| {
            GpuError::Unsupported(format!("读不到字体文件 {}：{e}", path.display()))
        })?;
        TextEngine::from_font_bytes(data, font_size)
    }

    /// 用系统字体建引擎（`consola.ttf` → `arial.ttf` → `segoeui.ttf`）。
    ///
    /// **找不到就返回错误，绝不静默降级成占位字形** —— 静默降级会让人以为
    /// 「字形已经好了」，而实际上屏幕上还是方块。
    pub fn from_system_font(font_size: f32) -> GpuResult<TextEngine> {
        let path = find_system_font().ok_or_else(|| {
            GpuError::Unsupported(
                "系统字体目录里找不到 consola.ttf / arial.ttf / segoeui.ttf；\
                 请改用 TextEngine::from_font_file 指定字体文件"
                    .to_string(),
            )
        })?;
        TextEngine::from_font_file(&path, font_size)
    }

    pub fn font(&self) -> &Font {
        &self.font
    }

    pub fn font_size(&self) -> f32 {
        self.font_size
    }

    /// 改默认字号。**已有图集缓存不失效**（不同字号是不同的 `GlyphKey`）。
    pub fn set_font_size(&mut self, font_size: f32) {
        self.font_size = font_size.max(1.0);
    }

    /// 度量接口（与布局阶段用的是同一份字体、同一个字号）。
    pub fn measure(&self) -> FontMeasure<'_> {
        FontMeasure::new(&self.font, self.font_size)
    }

    /// 取一个字符在当前字号下的排版信息；必要时光栅化并入图集（带缓存）。
    ///
    /// **缺字回退到 glyph 0（`.notdef`）**，与布局阶段 [`FontMeasure::advance`] 的口径一致 ——
    /// 这样「布局算的宽度」与「画出来的宽度」不会因为一个缺字就对不上；屏幕上会看到
    /// 一个「豆腐块」（这也是字体惯例，比静默不画诚实）。缺字另计入 `missing_glyphs()`。
    ///
    /// 返回 `None` 的只剩一种情形：图集放不下（字形比图集还宽 / 高度超过上限），
    /// 或连 glyph 0 都取不出来。这种情况也计入 `missing_glyphs()`。
    pub fn glyph(&mut self, ch: char, px_size: f32) -> Option<GlyphPlacement> {
        let px = px_size.max(1.0);
        let px_key = (px.round() as u16).max(1);

        let index = match self.font.glyph_index(ch) {
            Ok(Some(i)) => i,
            // 未映射字符：计入诊断，**回退到 .notdef**（不是静默不画）
            Ok(None) | Err(_) => {
                self.missing += 1;
                0
            }
        };

        let key = GlyphKey::new(index, px_key);
        if let Some(p) = self.placements.get(&key) {
            return Some(*p);
        }

        let glyph = match self.font.glyph(index) {
            Ok(g) => g,
            Err(_) => {
                self.missing += 1;
                return None;
            }
        };
        let rasterizer = Rasterizer::new(px_key as f32);
        let image = rasterizer.rasterize(&glyph, self.font.units_per_em);
        let slot = match self.atlas.insert(key, &image) {
            Some(s) => s,
            None => {
                self.missing += 1;
                return None;
            }
        };
        let placement = GlyphPlacement {
            slot,
            left: image.left,
            top: image.top,
            advance: image.advance,
        };
        self.placements.insert(key, placement);
        Some(placement)
    }

    /// 一段文本的**落笔宽度**（像素）：逐字形真实 advance 之和，与 `glyph()` 同源。
    ///
    /// 与布局用的 `FontMeasure::text_width` 只差**末尾取整**（后者 `ceil()`，差 < 1px）：
    /// 这里是「笔实际走过多远」，布局那边是「盒子里要留多宽」。
    pub fn text_width(&mut self, text: &str, px_size: f32) -> f32 {
        let mut w = 0.0;
        for ch in text.chars() {
            if let Some(p) = self.glyph(ch, px_size) {
                w += p.advance;
            }
        }
        w
    }

    /// 已经光栅化并入图集的字形数（诊断 / 断言用）。
    pub fn rasterized_glyphs(&self) -> usize {
        self.placements.len()
    }

    /// 缺字**计数**：**按调用次数累计**（不是「缺字种类数」）。三种情况各 +1：
    /// ① cmap 未命中（已回退 `.notdef`，仍然计数）；② 字形数据读不出来；③ 图集放不下。
    pub fn missing_glyphs(&self) -> usize {
        self.missing
    }

    pub fn atlas(&self) -> &GlyphAtlas {
        &self.atlas
    }

    /// 把图集写成 PNG（灰色：0 → 黑，255 → 白）。调试与示例用。
    pub fn atlas_png(&self) -> Result<Vec<u8>, String> {
        let (w, h) = self.atlas.size();
        let cov = self.atlas.coverage();
        let expected = (w as usize) * (h as usize);
        if cov.len() != expected {
            return Err(format!(
                "图集缓冲长度 {} 与尺寸 {w}×{h} 不一致",
                cov.len()
            ));
        }
        let mut rgba = Vec::with_capacity(expected * 4);
        for &c in cov {
            rgba.extend_from_slice(&[c, c, c, 255]);
        }
        crate::png::encode_rgba(w, h, &rgba)
    }
}

impl std::fmt::Debug for TextEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let (aw, ah) = self.atlas.size();
        f.debug_struct("TextEngine")
            .field("font_size", &self.font_size)
            .field("units_per_em", &self.font.units_per_em)
            .field("glyphs", &self.placements.len())
            .field("atlas", &format_args!("{aw}×{ah}"))
            .field("missing", &self.missing)
            .finish()
    }
}
