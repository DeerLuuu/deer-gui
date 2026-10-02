//! # deer-text —— L1 **TextServer**
//!
//! 文本栈的**独立 crate**：字体解析 → 字形光栅化 → 字形图集 → 文本引擎 → 真实字体度量。
//! 这一层是「服务级」的（有缓存、要读字体文件），所以不与 L0 的纯数据契约同处一个 crate。
//!
//! ## 分层位置（`docs/ARCHITECTURE.md` §2.3；LY2 物理拆分）
//!
//! ```text
//!   deer-core（L0：节点树 / 布局 / 绘制命令 / 错误类型 / HAL 契约）
//!        ↑
//!        ├── deer-text（本 crate，L1 TextServer：字体 / 光栅化 / 图集 / 排版度量）
//!        └── deer-gpu（L1 RenderServer：null 后端 / render / interact / HAL traits）
//! ```
//!
//! **依赖方向是单向的**：`deer-gpu → deer-text → deer-core`（见 `ROADMAP.md` 的 LY2 登记）。
//! 所以本 crate **不依赖 `deer-gpu`**，也不依赖任何第三方 crate。
//!
//! ## 本 crate 装了什么
//!
//! | 模块 | 内容 |
//! |---|---|
//! | [`font`] | 零依赖 TrueType 解析（`head`/`hhea`/`hmtx`/`maxp`/`cmap`/`loca`/`glyf`） |
//! | [`glyph`] | 流水线共享数据类型：[`GlyphImage`] / [`GlyphKey`] / [`AtlasSlot`] |
//! | [`raster`] | 轮廓 → 覆盖率位图（扫描线 + 超采样抗锯齿、nonzero winding） |
//! | [`atlas`] | 货架打包的字形图集（按需增高、不移动已有槽位） |
//! | [`measure`] | [`FontMeasure`]：真实 `hmtx`/`hhea` 度量 + 贪心换行（实现 `deer_core::Measure`） |
//! | [`text`] | [`TextEngine`]：字体 + 字号 + 字符串 → 图集槽位 + 排版偏移 |
//! | [`png`] | 零依赖 PNG 编码器（`atlas_png()` 与示例出图用） |
//!
//! ## 诚实边界（**没有**做的事）
//!
//! - **不做 hinting / 字距连字（`kern`/`GSUB`/`GPOS`）/ 竖排 RTL / 多字体回退**；
//!   宽度就是 advance 之和，缺字回退到 glyph 0（`.notdef`）。
//! - **不支持 CFF / OpenType-CFF（`OTTO`）轮廓**：解析层直接报错，不静默给空轮廓。
//! - **亚像素水平定位**：光栅化层有 opt-in 路径（1/4 相位档），但**文本引擎默认不接线**，
//!   产品像素与整数落位一致（理由与实测见 [`text`] 与 [`raster`] 的模块文档）。
//! - **图集不淘汰**（无 LRU）：只增不减，长会话里字号种类多时会持续增长。
//! - [`png`] 是本 crate 里**唯一与文本无关**的模块：它原本在 `deer-gpu`，而
//!   [`TextEngine::atlas_png`] 需要它；由于依赖方向是 `deer-gpu → deer-text`（不能反向），
//!   物理上随迁进来，并由 `deer-gpu` 以 `pub use deer_text::png;` **保留原路径**
//!   （`deer_gpu::png::encode_rgba` 逐字不变）。更合适的长期归属是 L0 公共工具，
//!   留给 LY4 对账（见 `ROADMAP.md`）。

#![forbid(unsafe_code)]
#![deny(clippy::all)]

pub mod atlas;
pub mod font;
pub mod glyph;
pub mod measure;
pub mod png;
pub mod raster;
pub mod text;

pub use atlas::GlyphAtlas;
pub use font::{Contour, Font, Glyph, Point, Segment};
pub use glyph::{AtlasSlot, GlyphImage, GlyphKey};
pub use measure::{FontMeasure, find_system_font};
pub use raster::Rasterizer;
pub use text::{GlyphPlacement, TextEngine};
