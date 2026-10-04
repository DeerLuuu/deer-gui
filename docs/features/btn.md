# Btn —— 按钮（语义对齐）

> M6 控件族 5a 的第 1 项。**本轮不改任何行为**：现有 `Kind::Button` 已有完整的
> 点击/禁用/hover/pressed/焦点语义（A 线落地），这里做的是把「它到底承诺什么」
> **写清楚并钉死**——每个语义条目都给出钉住它的测试。

## 1. 这是什么 / 什么时候用它

`Kind::Button` 是可点击、可聚焦的叶子控件：画一个强调色圆角底 + 居中标签，
点击时通过 `UiEvent::Clicked(id)` 报告（**树是纯数据，事件用 id 关联**）。

什么时候**不**该用它：

- 需要「二选一/多选」的按钮组 → 那是 M6 5c 的 `Segmented`/`ChipGroup`（还没做，不要以为能跑）；
- 需要开关语义（保持开/关状态）→ M6 5d 的 `Switch`（还没做）。按钮本身**没有状态**，
  「它被按过没有」是 App 的事；
- 一排动作按钮想省样板 → [`RowActions`](row-actions.md)（组合层便捷构造）。

## 2. 最小示例

```rust
use deer_gui::prelude::*;

let mut app = Builder::new(Kind::Column, "app").padding(12.0).gap(8.0);
let save = app.button("保存");                     // 返回自动 id（事件关联用）
let del = app.button_opts("删除", |n| {
    n.props.disabled = true;                       // 禁用：整棵子树不响应输入
});
let tree = app.build();
// 点击后 App 在 UiEvent::Clicked(save) 里拿到「是哪个按钮」
```

能跑的完整版（含点击/禁用/Enter/拖出结算/RowActions 组合的自检断言）：

```sh
cargo run -p deer-gui --features testing --example m6_basics
```

## 3. 完整 API

| 入口 | 说明 |
|---|---|
| `Builder::button(label) -> String` | 追加一个按钮，返回自动 id（`button_N`） |
| `Builder::button_opts(label, f) -> String` | 同上，闭包里改节点（禁用、显式 id 等在 `f` 里做；**先设参数、id 最后定**） |
| `Node::disabled()` / `props.disabled` | 禁用（对整棵子树生效，不止按钮自己） |
| `UiEvent::Clicked(id)` | 点击事件（下面「语义」表格给出全部结算规则） |

### 语义（本仓库按钮承诺的全部行为，逐条有测试）

| 交互 | 行为 | 钉住它的判据 |
|---|---|---|
| 左键按下 | 命中的按钮**即被捕获**（D7：按下即默认捕获），同时**聚焦它**、记 `pressed`（视觉加深） | `r3b_pointer_down_focuses_the_clicked_focusable_widget` |
| 按住拖出（节点/整棵树） | hover **钉在捕获者上**不换人、不发 `HoverChanged` | `r3_press_captures_and_drag_out_still_routes_click_settled_by_capturer` |
| 左键抬起 | **按捕获者结算 `Clicked`** —— 抬起在哪都算它的点击（拖出即丢是改前的旧行为，D7 裁定废弃） | 同上 + `r3c_drag_out_and_back_settles_the_same_as_in_place` |
| `Enter`（焦点在启用的按钮上） | = 一次点击（`Clicked`） | `r12_enter_on_focused_button_is_a_click` |
| 禁用（自身或祖先） | **整棵子树静默**：无 hover、无 Clicked、无 FocusChanged、`pressed` 不落、**不进焦点序** | `r4_disabled_node_and_subtree_ignore_input` |
| 视觉 | hover 提亮 / pressed 加深 / 禁用 = `border` 底 + `text_dim` 字（不透明色 ⇒ 像素判据不回退） | `deer-gpu/src/interact.rs` 的状态色测试 + 像素判据 |
| `Tab` / `Shift+Tab` / 方向键 | 按钮在焦点序列里按树序/几何邻近循环 | `r6_*`、`arrow_*` |
| `Escape` | 清焦点（按钮不拦截） | `r7_escape_clears_focus` |
| 右键 | 按下即发 `PointerRight`（纯透传，不 pressed/不聚焦） | `right_press_fires_pointer_right_without_pressing` |
| 窗口失焦 | 清 `hover`/`pressed`（捕获跟着丢：抬起可能永远不来） | `r13_window_blur_clears_hover_and_press` |

### 开放问题（deer-ui 特有行为不可考，登记不臆造）

- deer-ui 的 Btn 是否支持 `Space` 激活？本仓**只有 `Enter`**（`Key::Char` 不被消费）；
- 无障碍属性（aria-disabled 等）没有任何承载位（`NodeProps` 没有 aria 字段）；
- 快捷键提示/tooltip → 属 `HoverTip`（M6 5b，还没做）。

## 4. 自检（怎么确认你真的用对了）

```rust
// testkit 注入（tests/m6_basics.rs 的判据形状）：
let step = h.tap(ok.as_str())?;
assert!(step.events.contains(&UiEvent::Clicked(ok.clone())));
h.assert_focus(Some(ok.as_str()))?;              // 点击可聚焦控件 ⇒ 聚焦它

// 禁用按钮必须静默（允许 HoverChanged —— 那是 hover 离开别的节点）：
let step = h.tap(off.as_str())?;
assert!(step.events.iter().all(|e| !matches!(e, UiEvent::Clicked(_) | UiEvent::FocusChanged(_))));
```

App 侧对应关系：`Clicked(id)` 里的 `id` 就是 `button()` 当初返回的那个 String ——
**拿着它查你自己的数据，不要去树上找回调**（树里没有回调）。

## 5. 常见坑

| 现象 | 原因 | 怎么改 |
|---|---|---|
| 按钮点了没反应 | `Clicked(id)` 发了但 App 没按 id 关联（或按钮在禁用子树里） | 核对 id 与 `disabled`；用 testkit 的 `tap` + `events.contains` 定位是哪半 |
| 点了别的按钮也算上一个的点击 | 按住不放拖过去再抬起 ⇒ **捕获者**结算（D7 语义，不是 bug） | 需要严格的「抬起处才算」就得自己在 App 层比对坐标（本仓刻意不做第二套路由） |
| 「 disabled 按钮怎么还能 Tab 到」的错觉 | 没有这回事——禁用按钮**不进** `focusables` | 用 `assert_focus_order_excludes` 钉住 |
| `Key::Char(' ')` 想当点击 | 字符键不消费（输入统一走 `TextInput`） | 用 `Enter`；`Space` 语义见开放问题 |

## 6. 相关

- 相关功能：[`input`](input.md)（事件模型与 dirty 约定）、[`row-actions`](row-actions.md)（一排按钮的便捷构造）、[`hit-testing`](hit-testing.md)（命中=输入路由的唯一依据）
- 内部原理：`crates/deer-gui/src/interaction.rs` 的状态机文档（规则 r2–r13 的注释）
- 边界（**做不到什么**）：无 aria/无障碍属性；无快捷键提示与 tooltip（`HoverTip` 未做）；
  无长按/双击/右键菜单（右键只有 `PointerRight` 透传，菜单属 M6 5f）；无 toggle/选中态
  （选择类是 5c）；无 `Space` 激活；按钮没有自己的「值」——值在 App 手里。

## 7. 检查清单（发布前过一遍）

- [x] 示例能跑：`cargo run -p deer-gui --features testing --example m6_basics` → `exit=0`
- [x] 示例有自检断言（点击/禁用/Enter/拖出结算/重建保留，全部 assert）
- [x] `FEATURES.md` 已登记（状态 / 指南链接 / 示例命令都对）
- [x] `docs/TUTORIAL.md` 已有对应章节（§3 的禁用按钮即本控件；Btn 属既有主线，未新增章）
- [x] 明确写了「做不到什么」
