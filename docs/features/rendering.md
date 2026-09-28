# 功能指南：渲染（rendering）

> 状态 ✅（**仅离屏**）· 示例 `cargo run -p deer-gui --example render_to_png` ·
> 清单条目见 [`FEATURES.md`](../../FEATURES.md)

## 1. 这是什么 / 什么时候用它

把界面树变成**一张图片**（或一块像素）。当前是**离屏渲染**：不涉及窗口，
输出 PNG 文件或内存里的 RGBA8 缓冲。

什么时候用它：
- 生成界面设计稿 / 给文档配图；
- 验证布局是否正确（配合 [`geometry.md`](layout.md)）；
- 做布局实验，快速看结果。

**什么时候不能用它**：想让界面显示在窗口里、让用户点 —— 窗口与上屏从 M2b 起就通了，
**界面（`DrawList`）上屏是 M3c 已落地**（见 [`window.md`](window.md)），**输入与焦点是 M5 已落地**
（见 [`input.md`](input.md)）；本页讲的是**离屏**出图那条路。

## 2. 最小示例

```rust
use deer_gui::prelude::*;
use deer_gui::render_tree_to_png;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut app = Builder::new(Kind::Column, "app").padding(12.0).gap(8.0);
    app.text("你好");
    app.button("确定");
    let tree = app.build();

    // 参数：树、画布宽、画布高、主题
    let png = render_tree_to_png(&tree, 320, 200, Theme::default())?;
    std::fs::write("out.png", png)?;

    println!("写出 out.png");
    Ok(())
}
```

跑完整版：`cargo run -p deer-gui --example render_to_png` → 产物 `render_out/render_to_png.png`

## 3. 完整 API

### 三个入口

| 函数 | 输出 | 什么时候用 |
|---|---|---|
| `deer_gui::render_tree_to_png(&tree, w, h, theme)` | `Result<Vec<u8>, String>`（PNG 字节） | 要落盘成图片 |
| `deer_gui::render_tree_to_rgba(&tree, w, h, theme)` | `Result<(u32, u32, Vec<u8>), GpuError>` | 要自己处理像素 |
| `deer_gui::layout_tree(&tree, w, h, theme)` | `Geometry` | 只要几何，不渲染 |

### 渲染管线（四步，每步都能单独用）

```
树 + 画布
   │  ① layout()            → 几何表（每个控件的矩形）
   │  ② build_draw_list()   → 绘制列表（与后端无关的命令）
   │  ③ 后端                → 像素（CPU 后端已实现，Vulkan 在做）
   │  ④ encode_rgba()       → PNG 字节（零依赖编码器）
```

```rust
use deer_gui::prelude::*;

// ①
let geo = deer_gui::layout_tree(&tree, 320, 200, theme.clone());
// ②
let list = deer_gpu::build_draw_list(&tree, &geo, theme.clone(), &ApproxMeasure);
// ③
let fb = deer_gpu::null::CpuRenderer::new().render(
    deer_gpu::Extent { width: 320, height: 200 },
    &list,
    theme.surface,      // 背景色
)?;
// ④
let png = deer_gpu::png::encode_rgba(fb.width, fb.height, &fb.pixels)
    .map_err(|e| format!("编码失败：{e}"))?;
```

### CPU 后端（软件光栅化）

`CpuRenderer::render(extent, &DrawList, clear_color) -> GpuResult<Framebuffer>`

`Framebuffer` 上能做的：

| 方法 | 作用 |
|---|---|
| `.pixel(x, y)` | 取一个像素 → `Option<[u8; 4]>`（越界返回 `None`，不 panic） |
| `.count_color(color)` | 统计与给定颜色**逐字节相同**的像素数（断言用） |
| `.bytes_eq(&other)` | 两个帧缓冲逐字节比较（确定性验证用） |
| `.to_rgba()` | 拿到 `&[u8]` |
| `.width` / `.height` / `.pixels` | 字段 |

支持的命令：矩形填充、圆角填充（四角圆心近似，**无抗锯齿**）、1px+ 描边、裁剪栈，
以及文字 —— **无字库时**画等宽占位格（`CpuRenderer::new()`），
**有字库时**从字形图集采样真实字形（`CpuRenderer::with_text(engine)`，
见 [`text-rendering.md`](text-rendering.md)）。

## 4. 自检

```rust
let (w, h, px) = deer_gui::render_tree_to_rgba(&tree, 320, 200, Theme::default())?;

// ① 长度必须对
assert_eq!(px.len(), (w as usize) * (h as usize) * 4);

// ② 画面不能只有一种颜色 —— 否则什么都没画上（这种「假成功」最坑）
let mut colors = std::collections::BTreeSet::new();
for p in px.chunks_exact(4) { colors.insert([p[0], p[1], p[2], p[3]]); }
assert!(colors.len() > 1, "只有一种颜色 ⇒ 什么都没画上");

// ③ 确定性：两次渲染必须逐字节相同
let (_, _, a) = deer_gui::render_tree_to_rgba(&tree, 320, 200, Theme::default())?;
let (_, _, b) = deer_gui::render_tree_to_rgba(&tree, 320, 200, Theme::default())?;
assert_eq!(a, b);
```

## 5. 常见坑

| 现象 | 原因 | 怎么改 |
|---|---|---|
| 图片只有一种纯色 | ① 画布太小，内容被压成 0 尺寸；② 容器没给 `pad` ⇒ 按设计不画底色 | 用 `layout_tree` 打印几何确认；给容器加 `pad` |
| 图片里的「字」是方块 | 这个入口（`render_tree_to_png` / `CpuRenderer::new()`）**没有字库**，按设计画等宽占位格 —— 不是 bug | 要真实字形：用 `render_tree_to_png_with_font(...)` 或 `CpuRenderer::with_text(engine)`，见 [`text-rendering.md`](text-rendering.md) |
| 内容挤在左上角，右边一大片空白 | 根节点**不撑满画布**（宿主给的盒子是上限，不是命令） | 给根显式 `size`，或给子节点 `grow` |
| 圆角看起来是锯齿 | CPU 后端的圆角**不做抗锯齿** | 放大看会明显；抗锯齿属于后端能力，后续加 |
| PNG 文件比预期大很多 | 编码器用 zlib 的 **stored（未压缩）** 块，文件 ≈ 原始像素 + 少量开销 | 正常现象，换来的是**零依赖**。要小就接 deflate 压缩 |

## 6. 相关

- 布局：[`layout.md`](layout.md)
- 像素处理：[`pixels.md`](pixels.md)
- 绘制列表：[`draw-list.md`](draw-list.md)
- 主题：[`theme.md`](theme.md)
- **做不到**：本页这条**离屏 CPU** 路径**不**负责窗口与输入 —— 界面呈到窗口是 [M3c 已落地](window.md)、输入与焦点是 [M5 已落地](input.md)、GPU 侧文本也已落地（[`gpu-geometry.md`](gpu-geometry.md)）；仍缺的是圆角/字形之外的抗锯齿、滚动容器等

## 7. 检查清单

- [x] 示例能跑：`cargo run -p deer-gui --example render_to_png` → `exit=0`
- [x] 示例有自检断言（颜色数 > 1）
- [x] `FEATURES.md` 已登记
- [x] `docs/TUTORIAL.md` 已包含
- [x] 明确写了「做不到什么」
