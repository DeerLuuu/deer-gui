# ScrubNum —— 拖动调值

> M6 控件族 5d 的第 2 项。`Kind` 扩展（`Kind::ScrubNum`）：显示一个数值（`label` =
> **App 格式化的当前值**），在它上面**按住左右拖**改值 —— 依赖 T3.7 指针捕获
> （拖出不丢事件）。deer-ui 源码不在本仓库，这里是「常见 GUI 拖动调值语义的最小
> 正确版」，deer-ui 特有且不可考的行为一律登记为开放问题（第 3 节末）。

## 1. 这是什么 / 什么时候用它

`Kind::ScrubNum` 是一个**拖出来的数**：按下时从当前 label 解析出**基准值**，之后每次
移动都按「基准 + (x − 按下 x) × 步长」反解出新值，**变了才发**
`UiEvent::NumberChanged { id, value }`（已按值域夹取）。值的真相在 **App 手里**：
拿着事件改自己的数据 → 重建树（新 label）—— 锚点不动，所以「App 慢一帧」不影响后续值。

什么时候**不**该用它：

- 要**打字**输精确值 → [`NumberField`](number-field.md)；
- 要「开/关」而不是连续量 → [`Switch`](switch.md)；
- 要可见滑块/轨道的滑杆（slider）→ 本期不做（见「做不到什么」）。

## 2. 最小示例

```rust
use deer_gui::prelude::*;

let mut app = Builder::new(Kind::Column, "app").padding(8.0).gap(6.0);
let vol = app.scrub_num_opts("vol", "40", |_| {});
// vol = "vol"；label = "40" 就是当前值的**显示** —— App 必须把当前值格式化进 label，
// 「事件 → 改数据 → 重建树 → 新 label」就是整个回路。
// 步长/值域是 App 的数据（不塞 = 步长 1.0/px、无值域）：
// state.num_opts.insert("vol".into(), NumOpts { min: Some(0.0), max: Some(100.0), step: 2.0 });
let tree = app.build();
// 拖动中 App 在 UiEvent::NumberChanged { id: "vol", value } 一路拿到新值。
```

能跑的完整版（含提交/失败/失焦夹取/拖动/开关/颜色的自检断言）：

```sh
cargo run -p deer-gui --features testing --example m6_values
```

## 3. 完整 API

| 入口 | 说明 |
|---|---|
| `Builder::scrub_num(label) -> String` | 造一个（id 自动 `scrub_num_N`），返回节点 id |
| `Builder::scrub_num_opts(id, label, f)` | 指定 id 与就地改节点（`id` 传空串 = 自动） |
| `props.label` | **当前值的显示串**（App 写；按下时的基准值从这里解析） |
| `UiState::num_opts: BTreeMap<id, NumOpts>` | **步长/值域**住哪。表里没有 = 步长 1.0/px、无值域；`step` = 每**像素**对应的值变化 |
| `UiState::scrub: Option<ScrubAnchor>` | **瞬态锚点**（按下 x + 基准值 + 上次发出的值）。抬起 / 窗口失焦即清；只读它做诊断，**别手改** |
| `UiEvent::NumberChanged { id, value }` | 拖动中**变了才发**；`value` = 夹取后的 f64 |

### 语义（本仓拖动调值承诺的全部行为，逐条有测试）

| 交互 | 行为 | 钉住它的判据 |
|---|---|---|
| 按下 | 捕获（T3.7，D7）+ 解析 label 建**锚点**；解析失败 ⇒ 不进入拖动调值（这次按下只是普通点击） | `scrub_num_drags_from_the_parsed_anchor_and_clears_on_release` / `scrub_num_with_unparseable_label_never_enters_scrubbing` |
| 拖动中 | 值 = `基准 + (x − 按下x) × step`，夹值域；**变了才发** `NumberChanged`（静止的拖动零事件） | 拖动判据（+5px ⇒ 45；原地再动 ⇒ 无事件） |
| 基准 | **固定在按下那刻**（不是上一步的值）—— App 慢一帧重建树也不影响后续值 | 拖动判据（从 x0+5 拖回 x0−3 ⇒ 40−3=37，与 45 无关） |
| 拖出控件 | 捕获保证事件不丢（T3.7）：拖出节点、拖出整棵树照常调值；抬起按捕获者结算 `Clicked` | 既有 r3 判据 + 拖动判据 |
| 抬起 / 窗口失焦 | 锚点即清（瞬态；`UiState::scrub = None`） | 拖动判据的收尾断言 |
| 步长/值域 | `num_opts[它自己的 id]`：`step`（值/像素）、`min`/`max` 夹取（倒置时上界赢） | `scrub_num_uses_step_and_range_from_num_opts` |
| 视觉 | 与普通按钮同款（`accent` 底 + label 居中）；拖动中 `pressed` 加深就是拖动反馈；`DefaultRenderer`（无状态）也是普通按钮底 | 既有按钮判据 + 5c 的 opt-in 红线（杂散 id 零影响） |
| 禁用 | 整棵子树静默（按下都拿不到它 —— `hit` 拒绝） | 既有 r4/r12 判据 |

### 开放问题（deer-ui 特有行为不可考，登记不臆造）

- 灵敏度修饰键（按住 Shift 细调 / Ctrl 粗调）？本仓 `mods` 透传给了事件，
  但锚点语义没有按修饰键分档 —— 本期不做；
- 垂直拖动调值？本仓只认水平位移（`x − start_x`）；
- label 不可解析时的**视觉**标记（数值框有下划线，这里没有）—— 按下不进入即静默，
  登记为已知差异。

## 4. 自检（怎么确认你真的用对了）

```rust
// testkit 注入（tests/m6_values.rs 的判据形状）：
let (x0, y0) = h.center_of("vol")?;
h.send(&InputEvent::PointerMoved { x: x0, y: y0 })?;
h.send(&InputEvent::PointerDown { button: PointerButton::Left, x: x0, y: y0 })?;
let step = h.send(&InputEvent::PointerMoved { x: x0 + 5.0, y: y0 })?;
assert!(step.events.contains(&UiEvent::NumberChanged { id: "vol".into(), value: 45.0 }));
h.send(&InputEvent::PointerUp { button: PointerButton::Left, x: x0 + 5.0, y: y0 })?;
assert!(h.state().scrub.is_none());   // 抬起锚点即清
```

App 侧对应关系：拿着 `NumberChanged.value` 改自己的数据，**重建树时把新值格式化回
label** —— 否则下一次按下的基准还是旧值。

## 5. 常见坑

| 现象 | 原因 | 怎么改 |
|---|---|---|
| 按下拖动没有任何事件 | label 不可解析（基准解析失败 ⇒ 刻意不进入） | label 必须是 `format_num` 风格的可解析串 |
| 值「跳回」老值 | App 收到事件后**没把新值格式化回 label** 就重建了树（或没重建）—— 下一次按下的基准是旧 label | 事件 → 改数据 → 重建树（新 label），三步闭环 |
| 拖 1px 变化太大/太小 | 步长缺省 1.0/px | `num_opts` 里塞合适的 `step` |
| 期望「相对上一步」的累计 | 本控件从**锚点**反解（基准 = 按下那刻的值）—— 刻意语义（App 慢一帧不影响后续值） | 想要增量语义就由 App 自己累加 |
| 焦点序里找不到它 | 刻意的：拖动控件没有键盘语义，不进 `focusables`（`Tab` 不为它停站） | 用指针/触摸交互 |

## 6. 相关

- 相关功能：[`input`](input.md) §3.5（值类控件事件表）、[`number-field`](number-field.md)（同一张 `num_opts` 表的另一个读者）、[`btn`](btn.md)（视觉与点击结算同源）
- 内部原理：`crates/deer-gui/src/interaction.rs` 的 `PointerDown`/`PointerMoved` 锚点块与 `ScrubAnchor`；T3.7 指针捕获（D7）
- 边界（**做不到什么**）：无可见滑块/轨道（那是 slider，不做）；无垂直拖动；无修饰键灵敏度分档；无滚轮步进（滚轮归滚动容器）；无键盘交互（不进焦点序）；
  label 不可解析时静默不进入（无错误标记）；**它不替你存值** —— 值的真相永远在 App 的数据里。

## 7. 检查清单（发布前过一遍）

- [x] 示例能跑：`cargo run -p deer-gui --features testing --example m6_values` → `exit=0`
- [x] 示例有自检断言（拖动值/锚点清理/失败静默/三路翻转/颜色提交，全部 assert）
- [x] `FEATURES.md` 已登记（状态 / 指南链接 / 示例命令都对）
- [x] `docs/TUTORIAL.md` 已有对应小节（§3 数值类控件）
- [x] 明确写了「做不到什么」
