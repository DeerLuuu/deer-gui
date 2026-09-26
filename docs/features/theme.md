# 功能指南：主题（theme）

> 状态 ✅ · 示例 `cargo run -p deer-gui --example theme` ·
> 清单条目见 [`FEATURES.md`](../../FEATURES.md)

## 1. 这是什么 / 什么时候用它

**一组配色 + 字号**，决定渲染出来的样子。默认是深色。

什么时候用它：想换配色（深/浅/品牌色）、调字号。
什么时候不用：只想看默认效果——`Theme::default()` 就行。

## 2. 最小示例

```rust
use deer_gui::prelude::*;

// 浅色主题：只改要改的字段，其余沿用默认（`..` 是「其余字段不变」）
let light = Theme {
    text: Color::rgb(0x1a, 0x1d, 0x28),
    text_dim: Color::rgb(0x6b, 0x73, 0x88),
    surface: Color::rgb(0xf5, 0xf6, 0xfa),
    border: Color::rgb(0xd8, 0xdd, 0xe8),
    accent: Color::rgb(0x2f, 0x6f, 0xe0),
    on_accent: Color::rgb(0xff, 0xff, 0xff),
    ..Theme::default()
};

let png = deer_gui::render_tree_to_png(&tree, 300, 200, light)?;
```

跑完整版：`cargo run -p deer-gui --example theme` → 产物 `render_out/theme-{dark,light,warm}.png`

## 3. 完整字段表

| 字段 | 类型 | 影响哪里 |
|---|---|---|
| `text` | `Color` | 正文/文本节点 |
| `text_dim` | `Color` | **禁用**文字、输入框提示文字 |
| `surface` | `Color` | 带 `pad` 容器的底色；**也是画布背景色** |
| `border` | `Color` | 容器边框；**禁用按钮的底色** |
| `accent` | `Color` | 按钮底色（强调色） |
| `on_accent` | `Color` | 按钮上的文字 |
| `font_size` | `f32` | 字号（**影响文本度量宽高** ⇒ 布局随之变化）。真实字形渲染时，这个字号必须与 `TextEngine` 的字号一致，否则度量与绘制会漂（见 [`text-rendering.md`](text-rendering.md) 第 5 节；门面入口以 `font_size` 形参为准并覆盖本字段） |
| `line_height` | `f32` | 行高（影响文本节点高度） |

### `Color`

| 构造 | 说明 |
|---|---|
| `Color::rgb(r, g, b)` | 不透明，各通道 0–255 |
| `Color::rgba(r, g, b, a)` | `a` 是 `f32` 0.0–1.0 |
| `Color::WHITE` / `Color::TRANSPARENT` | 常量 |
| `.packed()` | 打包成 `0xRRGGBBAA`（诊断用） |

## 4. 自检

```rust
// 不同主题必须产出**不同**的像素，否则说明主题没被用上
let (_, _, dark)  = deer_gui::render_tree_to_rgba(&tree, 300, 200, dark_theme())?;
let (_, _, light) = deer_gui::render_tree_to_rgba(&tree, 300, 200, light_theme())?;
assert_ne!(dark, light, "换主题后像素必须变化");
```

> 注意：**字数变了会改布局**。若你在断言几何，换字号后要重新算期望值。

## 5. 常见坑

| 现象 | 原因 | 怎么改 |
|---|---|---|
| 换了主题但画面没变 | 传的还是 `Theme::default()`，或者改的字段没被渲染器用到 | 用上面的 `assert_ne!` 自检；确认字段名没写错 |
| 改了 `font_size`，布局断言挂了 | 字号参与文本尺寸计算 ⇒ **几何会变** | 这是预期行为；把几何期望值按新字号重算 |
| 禁用按钮看起来「还是蓝色」 | 禁用的是 `border` 色，若你的主题里 `border` 与 `accent` 接近就看不出差别 | 让两者区分度大一些 |
| 画布背景不是想要的颜色 | 背景就是 `surface` | 改 `surface`，或用 `render_tree_to_rgba` 后自己填背景 |

## 6. 相关

- 渲染：[`rendering.md`](rendering.md)
- 布局（字号影响几何）：[`layout.md`](layout.md)
- **做不到**：一个工程内多主题同时生效（每次渲染只能给一个 `Theme`）；CSS 那样的层叠/继承；逐控件样式覆盖（现在样式在 `DefaultRenderer` 里写死）

## 7. 检查清单

- [x] 示例能跑：`cargo run -p deer-gui --example theme` → `exit=0`
- [x] 示例有自检断言（不同主题像素必须不同）
- [x] `FEATURES.md` 已登记
- [x] `docs/TUTORIAL.md` 已包含
- [x] 明确写了「做不到什么」
