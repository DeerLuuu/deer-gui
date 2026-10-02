# 功能指南：滚动条（scrollbar）

> 跑 `cargo run -p deer-gui --example scroll_bar` ·
> 代码：几何 `crates/deer-layout/src/layout.rs`（`scrollbar_geom`）、绘制 `crates/deer-gpu/src/interact.rs` ·
> 出处：App 地基任务书 2026-10-02 的 **T3.2**

## 1. 这是什么 / 什么时候用它

**内容超出视口的容器会自己长出滚动条** —— 一条轨道 + 一个滑块，滑块的位置与高度反映
「当前看到的是内容的哪一段」。**滑块可以直接拖**（拖动改偏移、抬起结束）。

什么时候你会看到它：

- 树里有一个 `Kind::Column` 且 `layout.scroll = true` 的容器；
- 它**内容高于自己**（`max_scroll > 0`）；
- 而且你把 `UiState`（含滚动偏移与上限）交给了渲染器 —— 见下面第 2 节。

## 2. 最小示例

```rust
use deer_gui::layout::layout::{layout_with_scroll, ScrollOffsets};
use deer_gui::interaction::UiState;

// ① 布局：`layout_with_scroll` 会回一份「每个可滚动容器的滚动上限」
let (geo, metrics) = layout_with_scroll(&tree, box_, style, &measure, &offsets);

// ② 把上限灌进状态（**每帧都要做**）—— 灌漏了滚动条根本不会出现
let mut state = UiState::default();
state.scroll.set_metrics(&metrics);

// ③ 渲染：交互状态由 `UiState::to_interact_state()` 统一转换（它会把滚动快照一起带上）
let list = InteractiveRenderer::new(theme, &measure, &state.to_interact_state())
    .build(&tree, &geo);
```

跑 `--example scroll_bar` 的输出（实测）：

```text
① 布局：视口高 120，8 行 × 40 ⇒ max_scroll = 200
② 滑块位置随偏移移动：
  偏移   0 ⇒ 轨道 y=2 h=116｜滑块 y=2  h=44
  偏移 100 ⇒ 轨道 y=2 h=116｜滑块 y=38 h=44
  偏移 200 ⇒ 轨道 y=2 h=116｜滑块 y=75 h=44
```

## 3. 完整 API

几何只有**一份**实现，绘制与（将来的）命中都读它：

```rust
pub const SCROLLBAR_W: f32 = 8.0;          // 轨道宽
pub const SCROLLBAR_INSET: f32 = 2.0;      // 距视口边缘
pub const SCROLLBAR_MIN_THUMB: f32 = 24.0; // 滑块下限（内容极长时仍看得见）

pub struct ScrollbarGeom { pub track: Rect, pub thumb: Rect }
pub fn scrollbar_geom(viewport: Rect, offset: i32, max_scroll: i32) -> Option<ScrollbarGeom>

// 反解（拖动/命中用）：指针在这儿 ⇒ 偏移该是多少。与上一个函数是同一套映射的两个方向。
pub fn scrollbar_offset_for_pointer(viewport: Rect, max_scroll: i32, pointer_y: f32, grab_dy: f32) -> i32
```

绘制侧的接口是 `InteractState` 上的一个字段：

```rust
pub struct InteractState { hover, focus, pressed, pub scroll: ScrollView }
pub struct ScrollView { pub offsets: ScrollOffsets, pub metrics: ScrollMetrics }
```

**默认空** ⇒ 一个滚动条都不画 —— 这是刻意的 opt-in：既有语料逐字节不变。

## 4. 自检（怎么确认你真的用对了）

`--example scroll_bar` 结尾有三条自检，跑一次就全过：

1. 滑块随偏移**单调下移**（到顶贴顶、到底贴底）；
2. 内容装得下 ⇒ **不画**（满格滑块是噪音，还盖住内容最右 8 像素）；
3. **不喂滚动状态 ⇒ 不画**（这条保证既有像素判据不受影响）。

另有三条单测在 `deer-gpu`：默认状态不画 / 喂了状态**恰好**多出「轨道 + 滑块」两条、
且矩形与 `scrollbar_geom` 的结果逐字节相同 / 上限为 0 时不画。

## 5. 常见坑

- **容器没给显式高度** ⇒ `max_scroll` 恒为 0 ⇒ 滚动条永远不出现。
  滚动容器的语义是「内容超出**自己的**高度」，父盒子给多大都不管用。
- **忘了 `set_metrics`** ⇒ 上限为 0 ⇒ 同上。这一步每帧都要做（上限是**布局的输出**）。
- **只给 `InteractState` 塞了 `hover/focus/pressed`** ⇒ 滚动条不出现而且**不报错**。
  现在这个转换只有一处（`UiState::to_interact_state()`），别再手写它。
- **以为颜色能配** —— 现在是固定的（轨道 `theme.border`、滑块 `theme.text_dim`），
  没有「滚动条主题色」这个字段（加 `Theme` 字段是公开 API 变更，要走登记）。

## 6. 相关

- 滚动本身（滚轮驱动、视口裁剪、多行换行）：[`scroll-and-multiline.md`](scroll-and-multiline.md)
- 输入与状态：[`input.md`](input.md)
- 布局：[`layout.md`](layout.md)

### 做不到什么

- **点轨道跳转**（点轨道空白处应当跳到那一页）**还没做** —— 只能**拖滑块**或滚轮；
- **惯性滚动**（滚轮/拖动松手后的衰减）**还没做**；
- **横向滚动**：滚动条只有垂直方向（与「滚动容器本期只做垂直」一致）；
- **不能配颜色/宽度**：三个常量是编译期固定的；主题化需要先给 `Theme` 加字段（公开 API 变更，未登记）；
- **不会自动隐藏**（没有「静止时淡出」这类行为）。

## 7. 检查清单（发布前过一遍）

- [x] `--example scroll_bar` 真的跑过，`exit = 0`
- [x] 示例结尾有自检断言（三条），不是「跑成功就算」
- [x] 几何只有一份实现（绘制与命中都调 `scrollbar_geom`）
- [x] 默认状态不画（opt-in，既有语料逐字节不变）
- [x] 本指南含「做不到什么」一节
- [x] 登记进 `FEATURES.md` 且指南链接 + 示例命令都对
