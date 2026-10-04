# Switch —— 开关（独立翻转）

> M6 控件族 5d 的第 3 项。`Kind` 扩展（`Kind::Switch`）：点击 / `Enter` / `Space`
> 翻转开/关并发 `UiEvent::Toggled { id, on: 翻转后的新值 }`。开/关值住
> `UiState::switches`（**开关自身 id → bool**，表里没有 = 关）。deer-ui 源码不在本仓库，
> 这里是「常见 GUI 开关语义的最小正确版」，deer-ui 特有且不可考的行为一律登记为
> 开放问题（第 3 节末）。

## 1. 这是什么 / 什么时候用它

`Kind::Switch` 是一个**独立的开/关**：一次激活 = 一次翻转 = 一次 `Toggled`（翻转本身
就是动作，**每次都发** —— 与单选的「变了才发」刻意不同）。它是独立控件（不是组的
孩子），键是它自己的 id —— 与 [`ChipGroup`](chip-group.md) 的芯片（组 + 芯片两个 id）
同形但**不同表**。

什么时候**不**该用它：

- 一组**多选**标签（要组容器、组 id 报事件）→ [`ChipGroup`](chip-group.md)；
- **互斥**选择 → [`Segmented`](segmented.md)；
- 「按一下执行一次」的动作 → `Builder::button`（那不是状态，是命令）。

## 2. 最小示例

```rust
use deer_gui::prelude::*;

let mut app = Builder::new(Kind::Column, "app").padding(8.0).gap(6.0);
let wifi = app.switch_opts("wifi", "Wi-Fi", |_| {});
// wifi = "wifi" —— 事件与 UiState 用的就是它；label = 开关上的文字。
// 初值是 App 的数据：想默认开就塞表（不塞 = 关，也是合法状态）：
// state.switches.insert("wifi".into(), true);
let tree = app.build();
// 点击/Enter/Space 后 App 在 UiEvent::Toggled { id: "wifi", on } 拿到翻转后的值。
```

能跑的完整版（含三路激活/初值/禁用/拖动/提交式输入的自检断言）：

```sh
cargo run -p deer-gui --features testing --example m6_values
```

## 3. 完整 API

| 入口 | 说明 |
|---|---|
| `Builder::switch(label) -> String` | 造一个（id 自动 `switch_N`），返回节点 id |
| `Builder::switch_opts(id, label, f)` | 指定 id 与就地改节点（`id` 传空串 = 自动） |
| `UiState::switches: BTreeMap<id, bool>` | **值**住在哪。表里没有 = 关；App 直接读写（texts 同一条纪律：按 id 键控、住树外、重建不丢） |
| `UiEvent::Toggled { id, on }` | 每次激活必发；`id` = 开关自身节点 id；`on` = **翻转之后**的新值 |

### 为什么扩 `Kind` 而不是用按钮 + 约定（裁定 + 理由）

1. **交互层必须认出「这是个开关」**才发得出 `Toggled` 并维护 `switches` 表
   （组合层没有识别通道 —— 与 5c 扩 Kind 的第 1 条判据同一句）；
2. **「开/关」是持久视觉**：开 = `accent` 底 + `on_accent` 字，关 = `border` 底 +
   正文（复用 5c 的选中/未选中双色档）。绘制侧要按 `Kind` 读 `InteractState::switches`。

### 语义（本仓开关承诺的全部行为，逐条有测试）

| 交互 | 行为 | 钉住它的判据 |
|---|---|---|
| 点击 | `Clicked` + `Toggled{on: 新值}`，`switches[id]` 翻转 | `switch_toggles_on_click_enter_and_space` |
| `Enter`（焦点在它上） | **同一条** `resolve_switch` 结算（键盘与指针不分叉，与 r14 同路） | 同上 ② |
| `Space`（焦点在它上） | 与 `Enter` 完全同路。`Space` 在 winit 侧就是 `Named(Space) ⇒ Key::Char(' ')`（display.rs 的冻结映射）⇒ 交互层认的是 `KeyDown{Char(' ')}`；真实窗口流里随后的 `TextInput{" "}` 被**忽略**（Switch 不是值输入框）⇒ 不会双翻转 | 同上 ③ |
| 初值 | 先塞 `switches`（值是 App 的数据）；点开着的 ⇒ 翻成关 | 同上 ④ |
| 焦点 | 点击即聚焦；在 `Tab`/方向键焦点序里（树序 / T3.1 几何邻近）；**只挪焦点不改开关** | 焦点序判据 |
| 禁用（自身或祖先） | 整棵子树静默：无 Clicked/无 Toggled/不进焦点序 | `disabled_switch_is_silent_and_scrub_num_stays_out_of_the_focus_order` |
| 按住拖出再抬起 | 按捕获者结算（D7）—— 拖出去也算它的翻转 | 既有 r3 判据 + r16 同路 |
| 视觉 | 开 = `accent`/`on_accent`（与普通按钮同款）；关 = `border`/正文（muted 档，5c 同款）；hover 提亮 / pressed 加深照常；禁用赢过一切；**差异只落在开关矩形内**；`DefaultRenderer`（无状态）画普通按钮底、不读开关表 | `switch_visual_differs_only_inside_the_switch_rect`（含像素级框外零差异 + 杂散 id 免疫） |
| 值的寿命 | 按自身 id 键控住树外 ⇒ 整树重建不丢且仍可用（Keep） | `keep_rebuild_preserves_value_tables` |

### 开放问题（deer-ui 特有行为不可考，登记不臆造）

- 是否有滑块式开关（thumb 滑动动画）？本仓是双色按钮（无动画概念）；
- 是否有 `indeterminate`（第三态）？`bool` 承载不了，本期不做；
- 是否支持「点已开的开关 = 不变」的单击语义？本仓翻转语义固定（与芯片 r14b 同型）；
- 无 aria/switch 角色属性（`NodeProps` 没有承载位）。

## 4. 自检（怎么确认你真的用对了）

```rust
// testkit 注入（tests/m6_values.rs 的判据形状）：
let step = h.tap("wifi")?;
assert!(step.events.contains(&UiEvent::Toggled { id: "wifi".into(), on: true }));
// 再点同一个：Toggled { on: false }（翻转语义，每次都发）。
// 初值：直接塞 UiState（值是 App 的数据）：
// st.switches.insert("wifi".into(), true);
```

App 侧对应关系：拿着 `Toggled.id`（= `switch()` 返回的那个 id）查你自己的数据；
`on` 是翻转**之后**的值，直接抄即可。

## 5. 常见坑

| 现象 | 原因 | 怎么改 |
|---|---|---|
| 画面上开关永远是「关」色 | `switches` 表是空的（没塞初值，也没点过） | 塞 `state.switches`（表里没有 = 关，合法状态不是 bug） |
| `Enter`/`Space` 没反应 | 焦点不在它上（`Space`/`Enter` 需要**焦点在开关上**；点击会聚焦它） | 先点它一下，或 `Tab` 把焦点挪过去 |
| `Space` 翻转了两次 | 手工把 `Key::Char(' ')` **和** `TextInput{" "}` 都喂给了 `handle` 且自己又写了处理 | 真实窗口流不会双翻转（`TextInput` 对 Switch 是忽略的）；脚本里二选一 |
| 想按标签文本判断开/关 | 事件与值表用的是**节点 id**，不是标签 | 用 `switch()` 返回的 id 对号；标签可以重名，id 不会 |
| 开关不显示开态 | 用了 `DefaultRenderer` / `build_draw_list`（无状态渲染器，刻意不画开/关） | 走 `InteractiveRenderer` + `UiState::to_interact_state()`（Harness 已接好） |

## 6. 相关

- 相关功能：[`input`](input.md) §3.5（值类控件事件表）、[`chip-group`](chip-group.md)（同形的翻转语义，但组 + 芯片两个 id）、[`btn`](btn.md)（视觉与点击结算同源）
- 内部原理：`crates/deer-gui/src/interaction.rs` 的 `resolve_switch`（r16）；`crates/deer-gpu/src/interact.rs` 的 muted 档（开关的关 = 让位）
- 边界（**做不到什么**）：无滑块/动画；无第三态；无「点了不变」的单击取消语义；无 aria 角色；
  它不是组 —— 塞进 `ChipGroup` 里当芯片用会**同时**触发两套翻转（芯片表 + 开关表），不要这么摆；
  无键盘左右切换（`Key::Char` 只认空格，其余字符不消费）。

## 7. 检查清单（发布前过一遍）

- [x] 示例能跑：`cargo run -p deer-gui --features testing --example m6_values` → `exit=0`
- [x] 示例有自检断言（三路激活/初值/禁用/组外免疫/DefaultRenderer 无状态，全部 assert）
- [x] `FEATURES.md` 已登记（状态 / 指南链接 / 示例命令都对）
- [x] `docs/TUTORIAL.md` 已有对应小节（§3 数值类控件）
- [x] 明确写了「做不到什么」
