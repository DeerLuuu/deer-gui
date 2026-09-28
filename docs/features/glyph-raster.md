# 功能指南：字形光栅化（glyph-raster）

> 状态 ✅ · 示例 `cargo run -p deer-gui --example glyph_atlas` ·
> 清单条目见 [`FEATURES.md`](../../FEATURES.md)

## 1. 这是什么 / 什么时候用它

把**字体的轮廓**（`deer_gpu::font::Glyph` 里的一堆直线/二次贝塞尔）
变成一张**像素覆盖率图**（`deer_gpu::glyph::GlyphImage`）：
`coverage[i] == 255` 表示这个像素被字形完全盖住，`0` 表示完全没盖住，
中间值就是抗锯齿的边缘。

**什么时候用它**：你要自己控制「字形 → 位图」这一步时——例如把字形打包进纹理、
做位图字体缓存、离线把一批字符导出成图片，或者只想要某个字符的墨迹像素数。

**什么时候不该用它**：你只是想把一段文字画到界面上 —— 直接用
[`text-rendering.md`](text-rendering.md) 的 `TextEngine` / `CpuRenderer::with_text`，
它已经把「度量 → 光栅化 → 图集 → 贴图」串好了。手写这一层意味着你要自己管
缩放、图集与缓存（见第 5 节的坑）。

## 2. 最小示例

```rust
use deer_gpu::font::Font;
use deer_gpu::raster::Rasterizer;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 字体解析：deer_gpu::measure::find_system_font() 按 consola → arial → segoeui 找
    let path = deer_gpu::measure::find_system_font().ok_or("找不到系统等宽字体")?;
    let font = Font::parse(std::fs::read(path)?)?;

    // ppem = 每 em 多少像素 = 字号
    let r = Rasterizer::new(16.0);          // 默认 4×4 = 16 个采样点
    let img = r.rasterize_char(&font, 'o')? // None = cmap 里没有这个字符
        .expect("系统字体里应当有 'o'");

    println!(
        "{}×{} left={} top={} advance={:.2} 墨迹像素={} 最大覆盖率={}",
        img.width, img.height, img.left, img.top, img.advance,
        img.ink_pixels(128), img.max_coverage()
    );
    Ok(())
}
```

跑完整版（会把整个 ASCII 可见字符集光栅化并打进图集、写出 PNG）：

```sh
cargo run -p deer-gui --example glyph_atlas
```

产物 `render_out/glyph_atlas.png`：覆盖率 0 显示黑、255 显示白。

## 3. 完整 API

### `deer_gpu::raster::Rasterizer`

| 方法 | 说明 |
|---|---|
| `Rasterizer::new(ppem: f32)` | `ppem` = 每 em 的像素数（就是字号）。超采样默认 **4**（4×4 = 16 采样/像素）。 |
| `Rasterizer::with_supersample(ppem: f32, supersample: u32)` | 指定超采样倍数；内部取 `supersample.max(1)`。`1` = 不抗锯齿（覆盖率只能是 0 或 255）。 |
| `rasterize(&self, g: &Glyph, units_per_em: u16) -> GlyphImage` | 光栅化一个**已经取出的**字形。`units_per_em` 来自 `Font::units_per_em`。 |
| `rasterize_char(&self, font: &Font, ch: char) -> GpuResult<Option<GlyphImage>>` | `char` → cmap → 字形 → 光栅化。**`Ok(None)` 表示 cmap 里没有这个字符**（不是错误）；字体数据坏 / 缺少必需表才是 `Err`。 |
| 字段 `ppem: f32` / `supersample: u32` | 公开的，读完可以直接看当前设置。 |

缩放规则：`scale = ppem / units_per_em`（font units → 像素）。

### `deer_gpu::glyph::GlyphImage` —— 坐标系（**这段最重要**）

```text
                 位图左上角 (left, top)
                        ┌──────────────┐
   基线（baseline）──────┼──────────────┼──────→ 笔位置 x，画完前进 advance
                        └──────────────┘
         ↑ left：位图左边缘相对「笔位置」的水平偏移（像素，i32）
         ↑ top ：位图**上**边缘相对「基线」的垂直偏移（像素，**向上为正**）
```

- `left` 相对**笔位置**：画这个字形时，位图的左边缘画在 `pen_x + left`。
  **通常 `left >= 0`**：多数字体的左边距（`lsb`）为正，`left = floor(lsb * scale)` 落在笔位置**右侧**。
  实测 consola.ttf @32px：`'.'`=6、`'i'`=2、`'l'`=2、`'H'`=1、`'W'`=0 —— **没有一个为负**。
  只有斜体 / 悬垂字形（斜体 `f`、某些 `j`）才会为负，那才是「向左探出」。
- `top` 相对**基线**、**向上为正**：位图第 0 行画在 `baseline_y - top`。
  下伸部（`g`、`y` 的尾巴）会让 `top` 变小甚至为负。
  **注意这是「向上为正」，和屏幕坐标（向下为正）相反** —— 这是本项目最容易写反的一个符号。
- `advance` 是**前进宽度**（像素），画完这个字形笔位置前进这么多。
  它和 `width` **没有关系**：空格 `width == 0` 但 `advance > 0`。
- `coverage` 是 `width * height` 字节的**覆盖率**（`0..=255`），**不是颜色**。
  颜色由使用方给 —— 同一张位图可以用任意颜色画，也能原样塞进图集。
- 位图是**紧致**的（宽高正好包住墨迹），所以「位图尺寸」≠「advance」≠「字号」。

`GlyphImage` 上能用的方法：

| 方法 | 作用 |
|---|---|
| `GlyphImage::blank(advance)` | 无墨迹字形（空格等）：`width == height == 0`，但**保留 `advance`** |
| `coverage_at(x, y) -> u8` | 越界返回 `0`，**不 panic**（后端贴图时代的路径不该因越界崩） |
| `is_blank()` | 宽或高为 0，或所有覆盖率都是 0 |
| `coverage_sum() -> u64` | 覆盖率总和，与墨迹面积成正比 |
| `max_coverage() -> u8` | 最大覆盖率（实心字形应当是 255） |
| `ink_pixels(threshold) -> usize` | 覆盖率 `>= threshold` 的像素数 |
| `mean_coverage() -> f32` | 单位面积平均覆盖率，**与 `coverage` 同一量纲：`0.0..=255.0`**（满覆盖位图 = `255.0`；要比例请自己除以 255）。注意**不是** `0.0..=1.0` |

字段 `width` / `height` / `left` / `top` / `advance` / `coverage` 都是公开的。

### 算法与不变量

| 项 | 行为 |
|---|---|
| 展平 | 直线直接用；`Segment::Quad` / `Segment::Cubic` 用 de Casteljau **自适应递归展平**（平坦度阈值 `0.25px`，深度上限 16 层兜底） |
| 填充规则 | **nonzero winding**（TrueType 规范）：逐采样点对全部边求环绕数，`!= 0` 视为实心。外轮廓与内轮廓方向相反时出「洞」 |
| 位图范围 | `left = floor(x_min * scale)`、`top = ceil(y_max * scale)`、`width = ceil(x_max * scale) - left`、`height = top - floor(y_min * scale)`；bbox 取**全部轮廓点（含贝塞尔控制点）**的包围盒 ⇒ 是真实曲线 bbox 的超集，不会裁掉墨迹（代价：可能宽不到 1px） |
| 抗锯齿 | 像素内取 `ss × ss` 网格，覆盖率 = `命中采样数 * 255 / ss²`（**四舍五入**到 `u8`） |
| `ss == 1` | 结果只能是 `0` 或 `255`（没有中间值） |
| `ss` 上限 | 公开字段 `supersample` 内部再夹到 `1..=64`（`ss²` 不至于爆掉） |
| 确定性 | 同一输入两次调用**逐字段相同**：无随机、无时间、不依赖 HashMap 迭代序 |
| `ppem <= 0`（含 NaN）或 `units_per_em == 0` | 返回 `blank(0.0)`（不 panic、不除零） |
| 无轮廓 / 退化轮廓 | `blank(advance)`：`advance` 仍按 `scale` 缩放保留 |
| 位图单边 > 4096 | `blank(advance)`（不尝试巨额分配） |

## 4. 自检（怎么确认你真的用对了）

覆盖率图最容易「看起来是字，其实反了/糊了」，所以要**断言**，不要靠眼睛：

```rust
use deer_gpu::font::{Contour, Glyph, Segment};
use deer_gpu::raster::Rasterizer;

// ① 单位正方形：font units 0..10，upem=10，ppem=10 ⇒ 正好铺满 10×10 像素
let square = Glyph {
    contours: vec![Contour {
        start: (0.0, 0.0),
        segments: vec![
            Segment::Line { to: (10.0, 0.0) },
            Segment::Line { to: (10.0, 10.0) },
            Segment::Line { to: (0.0, 10.0) },
            Segment::Line { to: (0.0, 0.0) },
        ],
    }],
    bbox: (0, 0, 10, 10),
    advance_width: 10,
    left_side_bearing: 0,
};
let img = Rasterizer::new(10.0).rasterize(&square, 10);
assert_eq!((img.width, img.height), (10, 10), "位图必须是 10×10");
assert_eq!(img.max_coverage(), 255, "实心方形应当有全实像素");
assert_eq!(img.ink_pixels(255), 100, "10×10 应当全部是 255");
assert_eq!(img.coverage_sum(), 255 * 100);

// ② ss=1 时不允许出现中间值（没开抗锯齿）
let hard = Rasterizer::with_supersample(10.0, 1).rasterize(&square, 10);
assert!(
    hard.coverage.iter().all(|&c| c == 0 || c == 255),
    "supersample=1 时覆盖率只能是 0 或 255"
);

// ③ 确定性：两次结果必须逐字节相同
let a = Rasterizer::new(16.0).rasterize(&square, 10);
let b = Rasterizer::new(16.0).rasterize(&square, 10);
assert_eq!(a, b, "光栅化必须是确定性的");
```

**洞**（nonzero 的本质）也可以直接断言：两个同心正方形、**方向相反** ⇒ 中心 `coverage == 0`；
把内圈方向改成相同 ⇒ 中心**不再是 0**。如果两条都通过，说明填充规则真的是 nonzero。

## 5. 常见坑

| 现象 | 原因 | 怎么改 |
|---|---|---|
| 字是**上下颠倒**的 | `top` 是「相对基线、**向上为正**」，直接拿它当屏幕 y 用就反了 | 画第 0 行用 `baseline_y - top`，不是 `baseline_y + top` |
| 文字挤在一起 / 字符重叠 | 用 `width` 当步进 —— 位图是紧致的，步进必须用 `advance` | 笔位置累加 `advance`，`width` 只用来定位位图 |
| 取空格得到一张 0×0 的图，以为出错了 | 空格**没有轮廓**：`blank(advance)`，位图为空但 advance 保留 | 用 `is_blank()` 判断；按 `advance` 前进，别跳过它 |
| `rasterize_char` 返回 `Ok(None)` | cmap 里确实没有这个字符（不是错误） | 自行决定怎么回退；`TextEngine` 会自己回退到 glyph 0（`.notdef`）画出豆腐块并计入 `missing_glyphs()`，见 [`text-rendering.md`](text-rendering.md) |
| 边缘有阶梯、想要更平滑 | 默认超采样是 4×4；`ss` 越低越硬 | 用 `with_supersample(ppem, 8)` 之类调高；`ss=1` 是**故意**不抗锯齿的快档 |
| 同一个字号下两个 `Rasterizer` 结果不一致 | 不该发生 —— 光栅化是确定性的 | 若真出现，检查是否传了不同的 `units_per_em`（必须来自同一个 `Font`） |

## 6. 相关

- 把位图打包进纹理：[`glyph-atlas.md`](glyph-atlas.md)
- 把「度量 + 光栅化 + 图集」串起来画字：[`text-rendering.md`](text-rendering.md)
- 字体解析（这些 `Glyph` 从哪来）：`crates/deer-gpu/src/font.rs`
- **做不到**（本模块的边界）：
  - **不做 TrueType hinting（这次有实测依据）**：不读 `glyf` 的 instructions，也不做像素网格拟合；
    小字号清晰度靠超采样抗锯齿 + 亚像素定位。**最省的 hinting-lite（垂直两极对齐整数像素行）已被实现并量过**：
    8 字号 × 6 字形，需整体垂直缩放、**最大形变 12.5%**、部分覆盖质量区间 **[-10.3%, +12.8%]**、
    均值仅 **+0.4%** ⇒ **没有净收益**；真 hinting 需要**指令虚拟机 + stem 识别 + CVT**。
    （可复现：`crates/deer-gpu/tests/text_raster.rs` 的 `simplified_vertical_extent_gridfit_is_not_shipped_measured`。）
  - **亚像素水平定位已落地为 opt-in 路径**：`Rasterizer::rasterize_at` / `rasterize_char_at` + `split_subpixel_x`（**1/4 像素档**）。
    **默认的 `Rasterizer::rasterize` 仍是整数落位、逐字节不变**（黄金指纹测试钉住）。
    实测收益：落位误差 RMSE **0.2890 → 0.0733 px（3.94×）**、最坏 **0.5 → 0.125 px**、
    相邻间距 RMSE **0.4924 → 0.1214 px**。
  - **⚠️ 这是「间距精度换边缘锐度」，不是「清晰度提升」**（代价必须一并写）：
    「固定相位 → 4 相位均值」的**部分覆盖质量占比上升** —— `l` **0.255→0.420（+64.3%）**、
    `H` **+45.0%**、`o` **+5.7%**、`e` **+9.9%**；另外位图可能**宽 1px**。**没有** LCD 子像素（RGB 三通道）渲染。
  - **「尚未生效」的边界**：这条 opt-in 路径**未接进** `TextEngine` / `CpuRenderer`（超出本次 scope）⇒
    **默认文本像素一点没变**，「整段文本更整齐」**还没有**。接法（下一格）：`GlyphKey` 加**相位档**、
    `GlyphPlacement` 暴露相位、`draw_text_real` 用 `whole + left`；影响面：**图集记录 ×4**、
    相位相关判据要改写。详见 `crates/deer-gpu/src/text.rs` 的「诚实边界」与 `crates/deer-gpu/src/raster.rs`。
  - **不支持 CFF / OpenType-CFF**（`OTTO`）：解析层直接报错，不静默给空轮廓。
  - **不做字距与连字**：不读 `GSUB`/`GPOS`/`kern`，`advance` 就是 `hmtx` 的原始值。
  - **不做竖排、变体、着色字体**。

## 7. 检查清单（发布前过一遍）

- [x] 示例能跑：`cargo run -p deer-gui --example glyph_atlas` → `exit=0`
- [x] 示例有自检断言（覆盖率/墨迹/不重叠/确定性）
- [x] `FEATURES.md` 已登记（状态 / 指南链接 / 示例命令都对）
- [x] 如果属于新手主线，`docs/TUTORIAL.md` 已更新
- [x] 明确写了「做不到什么」
