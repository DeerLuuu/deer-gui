//! **真实字体度量与换行**（M4）：用 `hmtx` / `hhea` 的真实数据替换布局里的「每字符 0.6em」近似。
//!
//! ## 这是什么
//!
//! [`FontMeasure`] 把 [`crate::font::Font`] 的真实度量（`unitsPerEm`、`hmtx` 的
//! advance width、`hhea` 的升部/降部/行距）换算成**像素**，并实现
//! [`deer_core::Measure`] —— 布局引擎于是可以注入它，替换
//! [`deer_core::ApproxMeasure`]（后者**保持不变**：它是确定性测试用的近似实现，
//! 故意与字体环境解耦）。
//!
//! 换算只有一个比例因子：`scale = font_size / units_per_em`。
//! 所有像素值都是「font units × scale」，不再有第二套算法。
//!
//! ## `advance(ch)` 的三种情形（确定性回退链，每一步都有测试）
//!
//! | 情形 | 结果 | 为什么这样定 |
//! |---|---|---|
//! | ① `cmap` 命中且字形可读 | `glyph.advance_width as f32 * scale` | 唯一的「真实值」路径（数据来自 `hmtx`） |
//! | ② `cmap` 未命中（或该字形索引读不出来） | **glyph 0（`.notdef`）的 advance** | 与 TrueType 渲染器语义一致：缺字符画 `.notdef` 的方框，宽度就用它的 advance；**不**用 `0.6em` 猜 |
//! | ③ 连 glyph 0 都取不到（`hmtx` 截断 / 索引越界 / 字体损坏） | `0.5 * font_size` | 兜底，**绝不 panic** —— 坏字体不该让整个布局崩掉 |
//!
//! 情形 ③ 在合法字体上**不可达**（`num_glyphs >= 1` 且 `hmtx` 有数据），
//! 它是防御性分支；测试用「把 `hmtx` 的目录项指到文件最后一个字节」把它构造出来。
//!
//! ## `wrap` 的规则（贪心，全部确定性）
//!
//! - 按**空格 / 制表**切词（其它空白字符不切，避免悄悄改变文本）；
//! - 贪心塞进当前行，**行首不留空白**（切词时空白本身就被丢弃）；
//! - 单词自身宽度 > `max_width` 时**按字符硬切**（一个字符就超宽时该行允许超宽 ——
//!   这是唯一例外，否则会死循环）；
//! - `max_width <= 0` → 不换行，返回 `vec![text.to_string()]`；
//! - 空字符串 → `vec![String::new()]`（**算 1 行**，与 [`deer_core::Measure::height`]
//!   的约定一致）。
//!
//! `wrap` 保留全部非空白字符：`wrap(text).concat()` 去掉空白后与 `text` 去掉空白后逐字符相同
//! （测试断言了这条不丢字性质）。

use std::path::PathBuf;

use crate::font::Font;
use deer_core::layout::{Measure, TextStyle};

/// 把字体度量换算成像素，并提供换行。
///
/// `font_size` 是像素尺寸（`px`）；`units_per_em` 来自 `head` 表。
// 注意：不 derive `Debug` —— `Font` 没有实现 `Debug`（它持有原始字体字节）。
#[derive(Clone, Copy)]
pub struct FontMeasure<'a> {
    /// 被度量的字体。
    pub font: &'a Font,
    /// 字号（像素）。
    pub font_size: f32,
}

impl<'a> FontMeasure<'a> {
    /// 绑定字体与字号。
    pub fn new(font: &'a Font, font_size: f32) -> FontMeasure<'a> {
        FontMeasure { font, font_size }
    }

    /// 像素/字体单位 的比例：`font_size / units_per_em`。
    ///
    /// `units_per_em == 0` 在解析期就被拒绝，这里仍然防御性回 `0.0`
    /// （回 0 会让所有度量变 0，比 `NaN`/`inf` 更容易发现）。
    pub fn scale(&self) -> f32 {
        let upem = self.font.units_per_em;
        if upem == 0 {
            return 0.0;
        }
        self.font_size / upem as f32
    }

    /// 升部（像素，**正**）：基线到行顶的距离。
    pub fn ascent(&self) -> f32 {
        self.font.ascender as f32 * self.scale()
    }

    /// 降部（像素，**正**）：基线到行底的距离。
    ///
    /// `hhea.descender` 通常是负数，这里取反得到正的「向下深度」。
    /// 若字体把 descender 写成正数（坏字体），结果是负数 —— 不做绝对值，
    /// 保持「读到的就是事实」。
    pub fn descent(&self) -> f32 {
        -(self.font.descender as f32) * self.scale()
    }

    /// 行高（像素）= `(ascender - descender + line_gap) * scale`。
    pub fn line_height(&self) -> f32 {
        self.font.line_height_units() * self.scale()
    }

    /// 单个字符的 advance（像素）。
    ///
    /// 三种情形的回退链见[模块文档](self)。
    pub fn advance(&self, ch: char) -> f32 {
        // ① cmap 命中 → 真实 hmtx advance
        if let Ok(Some(idx)) = self.font.glyph_index(ch) {
            if let Ok(g) = self.font.glyph(idx) {
                return g.advance_width as f32 * self.scale();
            }
        }
        // ② 未命中（或字形读不出）→ .notdef（glyph 0）的 advance
        if let Ok(g) = self.font.glyph(0) {
            return g.advance_width as f32 * self.scale();
        }
        // ③ 连 .notdef 都取不到 → 半个 em 兜底（不 panic）
        0.5 * self.font_size
    }

    /// 文本宽度（像素）：逐字符 advance 求和后 **`ceil()`**。
    ///
    /// 向上取整是刻意的：与 [`deer_core::ApproxMeasure`] 的「整数友好」约定一致，
    /// 便于布局与像素级断言；也让 `text_width("")` 精确等于 `0.0`。
    pub fn text_width(&self, text: &str) -> f32 {
        self.sum_advances(text).ceil()
    }

    /// 逐字符 advance 的原始和（**未取整**，诊断/交叉验证用）。
    fn sum_advances(&self, text: &str) -> f32 {
        text.chars().map(|c| self.advance(c)).sum()
    }

    /// 贪心换行。规则见[模块文档](self)。
    ///
    /// 实现**委托**给 `deer_core::layout::wrap_greedy`（唯一的词切分/硬切算法），
    /// 只注入「宽度怎么算」= [`FontMeasure::text_width`]。这样 `ApproxMeasure::wrap`
    /// （近似度量）与本函数的行为**逐字相同** —— 否则「换行点随度量的类别而分叉」
    /// 这类问题要在两个实现里各查一遍。
    pub fn wrap(&self, text: &str, max_width: f32) -> Vec<String> {
        deer_core::layout::wrap_greedy(text, max_width, |s| self.text_width(s))
    }
}

impl Measure for FontMeasure<'_> {
    /// 与 [`FontMeasure::text_width`] **同一份算法**（不是另起一套）：
    /// 忽略 `style.font_size`，字号由构造时给定的 `font_size` 决定。
    fn width(&self, text: &str, _style: TextStyle) -> f32 {
        self.text_width(text)
    }

    /// 换行点 = [`FontMeasure::wrap`]（多行绘制与布局高度**共用**这一处行数定义）。
    fn wrap(&self, text: &str, _style: TextStyle, max_width: f32) -> Vec<String> {
        FontMeasure::wrap(self, text, max_width)
    }

    /// 行数 × `style.line_height`。行数来自 [`FontMeasure::wrap`]（空串算 1 行）。
    ///
    /// 为什么用 `style.line_height` 而不是 `self.line_height()`：
    /// 行距属于**样式**（主题可能给出行距大于字体自带行高），度量层只负责行数。
    fn height(&self, text: &str, style: TextStyle, max_width: f32) -> f32 {
        self.wrap(text, max_width).len() as f32 * style.line_height
    }
}

/// 找一个可用于真值对照的系统字体。
///
/// 顺序：`consola.ttf`（**等宽**，ASCII 度量可交叉验证）→ `arial.ttf` → `segoeui.ttf`，
/// 目录是 `%WINDIR%\Fonts`（`WINDIR` 缺失时退回 `C:\Windows\Fonts`）。
///
/// 返回 `None` = 这台机器没有这三个字体之一 —— 调用方应当**明确跳过**，
/// 而不是伪装成功。
pub fn find_system_font() -> Option<PathBuf> {
    let dir = std::env::var_os("WINDIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Windows"))
        .join("Fonts");
    for name in ["consola.ttf", "arial.ttf", "segoeui.ttf"] {
        let p = dir.join(name);
        if p.is_file() {
            return Some(p);
        }
    }
    None
}
