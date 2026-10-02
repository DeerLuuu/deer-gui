# 功能指南：真实文字渲染（text-rendering）

> 状态 ✅（**仅离屏 / CPU 后端**）· 示例 `cargo run -p deer-gui --example text_render` ·
> 清单条目见 [`FEATURES.md`](../../FEATURES.md)

## 1. 这是什么 / 什么时候用它

把「字体文件」接进整条链路：**真实度量**（`FontMeasure`，来自 `hmtx`/`hhea`）
→ **布局**（`deer_core::Measure` 注入点）→ **光栅化** → **图集** → **CPU 后端贴像素**。
结果就是：离屏渲染出来的图里，字是**真字形**，不再是等宽方块占位。

**什么时候用它**：你想让渲染出来的图里出现真字；或者你想知道一段文字在某个字号下
有多宽、该怎么换行（`FontMeasure` 可以单独用，不需要渲染）。

**什么时候不该用它**：想要**窗口里**的文本 —— 那需要窗口 + GPU 侧文本，现在都**没有**
（见第 6 节）。也不要用它做富文本：没有多字体回退、没有粗体/斜体合成、没有图标字体。

## 2. 最小示例

```rust
use deer_gui::prelude::*;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // ① 建树（命令式；`.dui` 场景文件见第 3 节）
    let mut app = Builder::new(Kind::Column, "app").padding(12.0).gap(8.0);
    app.text("Hello 你好，真实字形");
    app.button("确定");
    let tree = app.build();

    // ② 找字体（consola → arial → segoeui；找不到是 None，不是静默降级）
    let font = deer_text::measure::find_system_font().ok_or("找不到系统字体")?;

    // ③ 带字体的渲染：布局与渲染用**同一个度量**
    let theme = Theme::default();
    let png = deer_gui::render_tree_to_png_with_font(&tree, 360, 160, theme, &font, 16.0)?;
    std::fs::write("out.png", png)?;
    Ok(())
}
```

跑完整版（写真 `render_out/text_render.png`，含覆盖率/墨迹/确定性/下伸部断言）：

```sh
cargo run -p deer-gui --example text_render
```

## 3. 完整 API

### 3.1 门面：带字体的渲染（最省事）

| 函数 | 说明 |
|---|---|
| `deer_gui::render_tree_to_rgba_with_font(&tree, w, h, theme, font_path: &Path, font_size: f32)` | → `GpuResult<(u32, u32, Vec<u8>)>`：布局用 `FontMeasure`，渲染用同一个 `TextEngine`（真实字形） |
| `deer_gui::render_tree_to_png_with_font(&tree, w, h, theme, font_path: &Path, font_size: f32)` | → `Result<Vec<u8>, String>`：上者的 PNG 版本（零依赖编码器） |
| `deer_gui::render_tree_to_rgba_with_engine(&tree, w, h, theme, font_size: f32, engine: TextEngine)` | 自己建好的 `TextEngine`（例如 `from_system_font`，或跨多次渲染复用同一份字体/图集），其余同上 |
| `render_tree_to_rgba` / `render_tree_to_png`（**不带**字体） | 仍然用 `ApproxMeasure` + 占位字形格。**不要**用它验证文字排版 |

**`font_size` 的语义（容易踩）**：它**同时**是 `TextEngine` 的字号和布局用的
`TextStyle.font_size`，并且会把传入 `theme.font_size` **覆盖**成同一个值 ——
因为 `DefaultRenderer` 发出的 `DrawCmd::Text.size` 取自 `theme.font_size`。
这样「布局 / 绘制列表 / 光栅化」三处才是同一个字号。**不要**指望用 `theme.font_size`
单独改字号：带字体的入口以形参 `font_size` 为准。

### 3.2 `deer_text::measure::FontMeasure<'a>` —— 真实度量与换行

```rust
FontMeasure { font: &'a Font, font_size: f32 }
FontMeasure::new(font, font_size)
```

| 方法 | 说明 |
|---|---|
| `scale() -> f32` | `font_size / units_per_em`（font units → 像素的比例） |
| `ascent() -> f32` | 升部（像素，**正**）= `ascender * scale` |
| `descent() -> f32` | 降部（像素，**正**，是 `-descender * scale`） |
| `line_height() -> f32` | 行高（像素）= `line_height_units() * scale` = `(ascender - descender + line_gap) * scale` |
| `advance(ch: char) -> f32` | 该字符的前进宽度（像素）。cmap 未命中（或字形读不出）→ **glyph 0（`.notdef`）的 advance**（确定性回退）；连 glyph 0 都取不到 → `0.5 * font_size` 兜底（不 panic） |
| `text_width(text: &str) -> f32` | `Σ advance(ch)`，结果 **`ceil()`**（整数友好，便于像素级断言）；`text_width("") == 0.0` |
| `wrap(text: &str, max_width: f32) -> Vec<String>` | 贪心按**空格 / 制表**切分；丢弃行首空白；**单词本身超过 `max_width` 时按字符硬切**（单字符就超宽时允许该行超宽，否则无法前进）；`max_width <= 0` 原样返回单行；空串 / 全空白 → 一行空串（**算 1 行**） |

它还实现了 `deer_core::layout::Measure`：

| trait 方法 | 实现 |
|---|---|
| `width(text, style)` | `== text_width(text)`（**同一套算法**，不是另写一份）。**忽略 `style.font_size`**：字号由构造 `FontMeasure` 时给定的那个决定 |
| `height(text, style, max_width)` | `wrap(text, max_width).len() * style.line_height`（空串也算 1 行）。行数来自 `wrap`，**行距来自样式**（不是字体自带行高） |
| `wrap(text, style, max_width)` | **换行点的第三处入口**（trait 上的方法，默认实现是「不换行」）；`FontMeasure` 覆盖它并**委托**给上面的固有 `wrap`。布局用它算「预留几行」，渲染器用它算「画出几行」——同一处定义。算法本体在 `deer_core::layout::wrap_greedy`（`ApproxMeasure` 与 `FontMeasure` 共用，只差「宽度怎么算」） |

> **多行文本节点**（`text` + `layout.wrap`）就是靠这个 trait 方法实现的：`deer_gpu::render::text_lines`
> 按行展开成 N 条 `DrawCmd::Text` ⇒ 后端**每个命令仍然只画一行**（`draw_text_real` 不需要懂换行）。
> 见 [`scroll-and-multiline.md`](scroll-and-multiline.md)。

`deer_text::measure::find_system_font() -> Option<PathBuf>`：在 `%WINDIR%\Fonts` 里按
`consola.ttf` → `arial.ttf` → `segoeui.ttf` 找。找不到返回 `None`（**明确失败**，不静默降级）。

### 3.3 `deer_text::text::TextEngine` —— 度量 + 光栅化 + 图集

```rust
TextEngine { /* font + 按 px_size 分桶的 Rasterizer + GlyphAtlas + placements */ }
```

| 方法 | 说明 |
|---|---|
| `from_font(font: Font, font_size: f32) -> TextEngine` | 用已解析的 `Font` 建（不会失败）。字号内部夹到 **≥ 1.0** |
| `from_font_bytes(data: Vec<u8>, font_size: f32) -> GpuResult<TextEngine>` | 从内存里的字体字节建 |
| `from_font_file(path: &Path, font_size: f32) -> GpuResult<TextEngine>` | 从字体文件路径建（内部 `std::fs::read` + 解析） |
| `from_system_font(font_size: f32) -> GpuResult<TextEngine>` | 用 `find_system_font()`；**找不到 ⇒ `Err`**，不静默降级 |
| `font() -> &Font` / `font_size() -> f32` / `set_font_size(f32)` | 读/改当前字号（同样夹到 ≥ 1.0）。改字号**不失效**已有图集缓存 |
| `measure() -> FontMeasure<'_>` | 借用**同一个字体、同一个（默认）字号**，得到与渲染同源的度量 |
| `glyph(&mut self, ch: char, px_size: f32) -> Option<GlyphPlacement>` | 命中缓存就返回；否则光栅化并入图集。**缺字（cmap 未命中）回退到 glyph 0（`.notdef`）并画出「豆腐块」**，同时计入 `missing_glyphs()` —— 与布局的 `FontMeasure::advance` 口径一致，所以缺字不会让「布局宽度」和「落笔宽度」对不上。返回 `None` 只剩两种情形：图集放不下（字形比图集宽 / 高度超上限），或连 glyph 0 都取不出；这两种也计入 `missing_glyphs()`。`px_size` 夹到 ≥ 1.0 后**取整**（`round`）当字号分桶键 |
| `text_width(&mut self, text: &str, px_size: f32) -> f32` | 一段文本的**落笔宽度**（像素）：逐字形真实 advance 之和。与布局用的 `FontMeasure::text_width` 只差**末尾取整**（后者 `ceil()`，差 < 1px）——这里是「笔实际走过多远」，那边是「盒子里要留多宽」。**注意是 `&mut self`**：量新字符时会顺手光栅化入图集 |
| `rasterized_glyphs() -> usize` | 已光栅化并入图集的字形数（诊断 / 缓存命中率） |
| `missing_glyphs() -> usize` | 缺字计数（cmap 未命中 / 图集放不下 / 回退字形取不出）。**按调用次数累计**：同一个缺字字符每被 `glyph()` 请求一次就 `+1`（即使该字符的 `.notdef` 已缓存），所以它衡量的是「碰到缺字的次数」而不是「缺字种类数」 |
| `atlas() -> &GlyphAtlas` | 内部图集（初始宽 512，高度按需增长；用法见 [`glyph-atlas.md`](glyph-atlas.md)） |
| `atlas_png() -> Result<Vec<u8>, String>` | 把图集导出成 PNG（灰度：覆盖率 0 → 黑、255 → 白），调试 / 示例用 |

### 3.4 `GlyphPlacement` —— 怎么把字形贴到像素上

```rust
GlyphPlacement { slot: AtlasSlot, left: i32, top: i32, advance: f32 }
```

- `slot`：字形位图在 `atlas()` 里的位置。
- `left` / `top`：与 [`GlyphImage`](glyph-raster.md) 同一套坐标系 ——
  `left` 相对**笔位置**，`top` 相对**基线、向上为正**。
- `advance`：画完前进多少像素。

贴图时：`pen_x += advance`；位图像素 `(px, py)`（`px < slot.w`、`py < slot.h`）
画到帧缓冲的 `x = pen_x + left + px`、`y = baseline_y - top + py`（笔位置与基线取整）。

**基线怎么定**（CPU 后端当前实现）：把「升部 + 降部」这块垂直居中放进 `DrawCmd::Text` 的 `rect`，
`baseline = rect.y + round((rect.h - (ascent + descent)) / 2) + round(ascent)` ——
与 `Theme::line_height` 无关（`rect` 已经是布局算好的盒子）。所以要改文字垂直位置，改布局盒子或字号，
不要指望调 `line_height` 挪基线。

`use deer_gui::prelude::*;` 会带出 `TextEngine` / `GlyphPlacement` / `FontMeasure` / `Rasterizer` /
`GlyphImage` / `GlyphKey` / `GlyphAtlas`，跟着教程走时不必逐个写 `deer_gpu::...` 路径。

### 3.5 CPU 后端：`CpuRenderer::with_text`

| 构造 / 方法 | 行为 |
|---|---|
| `CpuRenderer::new()` | **无字库**：`DrawCmd::Text` 仍走原来的等宽占位格（M1 旧行为，**像素级可区分**） |
| `CpuRenderer::with_text(engine: TextEngine)` | **有字库**：`Text` 命令按 `TextEngine` 的真实字形贴图（覆盖率采样 + 既有 `blend`） |
| `text() -> Option<&TextEngine>` | 取回内部字库（没有就是 `None`） |
| `render(&mut self, extent, &DrawList, clear_color)` | **注意是 `&mut self`**：绘制字符串里出现新字形时要就地把字形光栅化并入图集 |

同一份绘制列表重复渲染**仍然逐字节相同**（这是被测试钉住的不变式，不是口头承诺）。

### 3.6 用 `.dui` 场景文件（两条路径都能用真字）

字体是**渲染期**的事，和你怎么建树无关：`.dui` 建出来的树同样直接走
`render_tree_to_png_with_font`。

```rust
let text = std::fs::read_to_string("ui.dui")?;
let tree = parse_scene(&text, "ui.dui")?;         // 命令式建树也完全等价
let font = deer_text::measure::find_system_font().ok_or("找不到系统字体")?;
let png = deer_gui::render_tree_to_png_with_font(&tree, 360, 160, Theme::default(), &font, 16.0)?;
```

## 4. 自检（怎么确认你真的用对了）

**铁律（这一轮最重要的一条）：布局、绘制列表、光栅化必须用同一个字号、同一个度量。**
布局用 `FontMeasure`、渲染却用 `ApproxMeasure`（或反过来），或 `theme.font_size` 与字库字号不一致，
都会让文字和它所在的盒子错位、换行位置对不上 —— 而且**图看起来「差不多」**，很难肉眼发现。
所以自检要落在「度量一致」和「真字形不是方块」上：

```rust
// 片段：放在一个返回 Result 的函数里（`?` 需要）；`tree` 用第 2 节那棵。
use deer_text::measure::FontMeasure;
use deer_text::text::TextEngine;

// ① 度量交叉验证：advance 必须来自 hmtx（两条独立路径算出同一个数）
let mut engine = TextEngine::from_system_font(16.0)?;
let m = engine.measure();
let ch = 'o';
let idx = engine.font().glyph_index(ch)?.expect("字体里应当有 'o'");
let from_hmtx = engine.font().glyph(idx)?.advance_width as f32 * m.scale();
assert!((m.advance(ch) - from_hmtx).abs() < 0.01, "advance 必须与 hmtx 一致");

// ② 度量必须与渲染同源：`TextEngine::text_width` 是**未取整的落笔宽度**，
//    `FontMeasure::text_width` 会 **`ceil()`** 成盒宽 ⇒ 两者只允许 ≤1px 的取整差
//    （实测差 0.015625，所以容差写 0.01 会**必然失败**）
let size = engine.font_size();
let expect = FontMeasure::new(engine.font(), size).text_width("Hello");
assert!(
    (engine.text_width("Hello", size) - expect).abs() <= 1.0,
    "度量必须同源（只允许 ≤1px 取整差）"
);

// ③ 真实字形不是方块：'l' 的**竖干**必须明显窄于 advance
//    判据与 crates/deer-gpu/tests/text_raster.rs 的系统字体测试一致（@32px）。
//    注意不能量「全高墨迹」：Consolas 的 'l' 带左上旗 + 整宽底横，
//    本机实测全高有墨列数 = 14px（80% advance），直接卡 60% 会**失败**；
//    去掉顶旗/底横后的竖干（只看中间 1/3 行）= 4px（23% advance）才是「窄」的本意。
let p = engine.glyph('l', 32.0).expect("'l' 应当能光栅化");
let key = deer_text::glyph::GlyphKey::new(engine.font().glyph_index('l')?.unwrap(), 32);
let (slot, bytes) = engine.atlas().get(key).expect("刚插入就能取到");

let has_ink = |x: u32, lo: u32, hi: u32| (lo..hi).any(|y| bytes[(y * slot.w + x) as usize] > 0);
let ink_cols = |lo: u32, hi: u32| (0..slot.w).filter(|&x| has_ink(x, lo, hi)).count();
let stem = ink_cols(slot.h / 3, slot.h * 2 / 3); // 竖干（中间 1/3 行）
let full = ink_cols(0, slot.h); // 全高（含左上旗与整宽底横）

assert!(stem > 0, "'l' 的竖干必须有墨");
assert!((stem as f32) < p.advance * 0.6, "'l' 的竖干应当 < advance 的 60%");
assert!((full as f32) < p.advance, "'l' 的全高墨迹不该溢出自己的 advance");

// ④ 空格：不画墨，但必须占位（advance > 0）
let sp = engine.glyph(' ', 16.0).expect("空格应当有 placement");
assert!(sp.advance > 0.0, "空格必须前进");

// ⑤ 确定性：用第 2 节那棵树，两次渲染逐字节相同
let font = deer_text::measure::find_system_font().ok_or("找不到系统字体")?;
let theme = Theme::default();
let (_, _, a) = deer_gui::render_tree_to_rgba_with_font(&tree, 200, 80, theme.clone(), &font, 16.0)?;
let (_, _, b) = deer_gui::render_tree_to_rgba_with_font(&tree, 200, 80, theme, &font, 16.0)?;
assert_eq!(a, b, "同一输入两次渲染必须逐字节相同");

// ⑥ 缺字：cmap 没有的字符回退 `.notdef`（豆腐块）并计数，且两侧宽度仍一致。
//    consola 不含 CJK ⇒ '中' 必然走回退。这是
//    tests/text_pixels.rs::missing_glyph_falls_back_to_notdef_and_keeps_width_consistent
//    同一组判据的**独立最小复现**（那条测试在 24px 引擎上做，这里统一用引擎默认字号 ——
//    左右两边必须同一个 px_size，否则就是踩了上面第 1 条坑）
let px = engine.font_size();
let before = engine.missing_glyphs();
let miss = engine.glyph('中', px).expect("缺字必须回退到 .notdef，而不是不画");
assert!(miss.advance > 0.0, "回退字形也要有 advance");
assert!(engine.missing_glyphs() > before, "缺字必须计入 missing_glyphs()");
let drawn = engine.text_width("中", px);          // 落笔宽度（未取整）
let layout_w = engine.measure().text_width("中"); // 盒子宽度（ceil）
assert!((drawn - layout_w).abs() <= 1.0, "两侧宽度只允许 ≤1px 的取整差");
```

**整段已实跑通过（`exit=0`）**：把上面这段**逐字**搬进一个临时程序（外层只用第 2 节的树 +
`use deer_gui::prelude::*;`），用 consola.ttf 实测输出 ——
① `'o'` 的 advance 两条独立路径都是 `8.796875`（差 0）；
② `FontMeasure::text_width("Hello") = 44`、`TextEngine::text_width = 43.984375`（差 `0.015625` ⇒ ② 的容差必须按「≤1px 取整差」写，写 0.01 必失败）；
③ `'l'` @32px 位图 `14×23`、`left=2`、`advance=17.5938`，竖干 `4px`、全高 `14px`、`advance*0.6 = 10.5563`；
④ 空格 `advance=8.7969`（不画墨但占位）；
⑤ 两次渲染 `64000` 字节逐字节相同；
⑥ 缺字 `advance=8.7969`，落笔宽度 `8.796875` vs 布局宽度 `9`（差 `0.203125`）。
⑥ 是 `tests/text_pixels.rs::missing_glyph_falls_back_to_notdef_and_keeps_width_consistent` 同一组判据的**最小复现**（原测试还额外渲染一遍、断言豆腐块真的有墨迹）。

**「真的在用真字形」的判据**：把 `with_text` 换成 `CpuRenderer::new()`（占位格）渲染同一棵树，
两张图的像素必须**明显不同**。如果一样，说明真字形那条路根本没生效。

## 5. 常见坑

| 现象 | 原因 | 怎么改 |
|---|---|---|
| **文字和它所在的盒子错位 / 按钮里的字偏了 / 换行位置和渲染对不上** | 破了**铁律**：布局、绘制列表、光栅化三处没有用同一个字号、同一个度量。典型写法是布局用 `FontMeasure`，渲染却用不带字体的 `render_tree_to_png`（内部 `ApproxMeasure`），或者 `theme.font_size` 与传给字库的字号不一致 | 走 `render_tree_to_png_with_font` / `render_tree_to_rgba_with_font`（它会把 `theme.font_size` 覆盖成同一个值）；手动串链路时把 `engine.measure()` 给布局、**同一个** `engine` 给 `CpuRenderer::with_text`，并用同一个字号 |
| 改了 `theme.font_size` 字没变大 | 带字体的入口以形参 `font_size` 为准，并会**覆盖** `theme.font_size` | 改传给 `*_with_font` 的 `font_size` |
| 布局宽度与渲染视觉宽度对不上，但字号「看着一样」 | `measure()` 用的是引擎的**默认** `font_size`，而 `glyph(ch, px_size)` / `text_width(text, px_size)` 各自收一个 `px_size` —— 传了不同的值就破了铁律 | 让每次 `glyph`/`text_width` 的 `px_size` 都等于 `font_size()`；要整段改字号就用 `set_font_size()` |
| 图里的字还是**方块** | 用了不带字体的 `render_tree_to_png`，它内部是 `ApproxMeasure` + 占位字形格 | 换 `render_tree_to_png_with_font`；见 [`rendering.md`](rendering.md) 第 5 节 |
| `from_system_font` 直接 `Err` | 机器上没有 `consola.ttf`/`arial.ttf`/`segoeui.ttf`（或 `%WINDIR%` 异常） | 用 `TextEngine::from_font_bytes(std::fs::read("你的.ttf")?, size)`；**不要**期待静默降级 |
| 有些字符显示成**豆腐块** | cmap 未命中（字体里没有这个字符）⇒ `glyph()` **回退到 `.notdef`** 并画出来（字体惯例：可见的失败信号，比静默不画诚实） | 这是正常回退；用 `missing_glyphs()` 统计。本项目**不做**多字体回退，要那个字符就换一份含它的字体 |
| 缺字时**布局宽度与画面宽度对不上**（>1px） | 两个宽度只允许差末尾取整（<1px）：`TextEngine::text_width` 是**落笔宽度**（未取整），`FontMeasure::text_width` **`ceil()`** 成盒子宽度。若差得多，说明两边的**字号/字体**不同（破了第 1 条铁律），不是缺字造成的 | 让 `glyph`/`text_width` 的 `px_size` 与 `measure()` 的字号一致；缺字本身两边都按 `.notdef` 的 advance 算，**不会**造成宽度不一致 |
| 换行结果和预期不一样 | `wrap` 是**贪心按空白**切，且对超长单词**按字符硬切**（`ApproxMeasure` 与 `FontMeasure` 共用 `wrap_greedy`，只差宽度算法） | 这是刻意的确定性行为；要别的断行规则得自己写（`Measure::wrap` 是唯一入口，`height` 与多行绘制都依赖它） |
| `text_width` 比手算的多个 1px | 它 **`ceil()`** 到整数（与 `ApproxMeasure` 的整数友好约定一致） | 断言时按 `ceil` 算期望值 |
| 小字号看起来发糊/笔画粘连 | 不做 hinting，用超采样抗锯齿代替 | 见 [`glyph-raster.md`](glyph-raster.md) 第 6 节；放大字号或提高 `supersample` |
| 输出 PNG 很大 | PNG 编码器用 zlib **stored** 块（零依赖的代价） | 正常现象，见 [`pixels.md`](pixels.md) |

## 6. 相关

- 字形位图与坐标系：[`glyph-raster.md`](glyph-raster.md)
- 图集与打包：[`glyph-atlas.md`](glyph-atlas.md)
- 离屏渲染主流程：[`rendering.md`](rendering.md)
- 字体解析层（`Font` / `Glyph`）：`crates/deer-text/src/font.rs`
- **做不到**（本模块的边界）：
  - **没有 GPU 侧文本**：`CpuRenderer::with_text` 是 CPU 贴图。Vulkan 后端还不消费 `DrawCmd::Text`
    / 字形图集；把 `DrawList` 送上 GPU（含文本）属于 **M3**，图集已是它的前置依赖。
  - **没有子像素定位 / LCD 渲染**：字形按整数像素落位。**这是决策，不是遗漏** —— 光栅化层
    已有 opt-in 亚像素路径（`Rasterizer::rasterize_at` + `split_subpixel_x`），实测落位 RMSE
    改善 **3.94×**（0.2890→0.0733 px），但代价是**边缘锐度下降**（部分覆盖质量最高 **+64.3%**）
    ⇒ **无净视觉收益**，且接入会改变默认落位、破坏逐字节 parity 判据。
    完整数据与重新评估的触发条件见 `ROADMAP.md` 的「M4-6 决策登记」。
  - **没有多字体回退**：一个 `TextEngine` 一份字体；字体里没有的字符回退到 **`.notdef`（豆腐块）**，不会去别的字体里找。`missing_glyphs()` 给出计数。
  - **没有富文本 / 图标字体**：单一样式（一个字号一种颜色由 `DrawCmd::Text` 给），
    不支持粗体/斜体合成、下划线、字距调整、连字、颜色 emoji。
  - **没有竖排 / RTL / 复杂脚本整形**（不做 `GSUB`/`GPOS`）。
  - **不做 hinting**、**不支持 CFF**（`OTTO` 字体解析就报错）。
  - **仍然没有窗口**：只能离屏成 PNG / RGBA 缓冲。

## 7. 检查清单（发布前过一遍）

- [x] 示例能跑：`cargo run -p deer-gui --example text_render` → `exit=0`
- [x] 示例有自检断言（度量一致 / 墨迹 / 确定性 / 下伸部）
- [x] `FEATURES.md` 已登记（状态 / 指南链接 / 示例命令都对）
- [x] 如果属于新手主线，`docs/TUTORIAL.md` 已更新（第 11 章）
- [x] 明确写了「做不到什么」
