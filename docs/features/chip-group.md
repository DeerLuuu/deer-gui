# ChipGroup —— 标签组（多选开关）

> M6 控件族 5c 的第 2 项。`Kind` 扩展（`Kind::ChipGroup`）—— 扩 Kind 的裁定与理由
> 与 [`Segmented`](segmented.md) 完全同源（交互层识别 + 新持久视觉），见该指南第 3 节。
> deer-ui 源码不在本仓库，这里是「常见 GUI 标签/芯片组语义的最小正确版」，
> deer-ui 特有且不可考的行为登记为开放问题（第 3 节末），不臆造。

## 1. 这是什么 / 什么时候用它

`Kind::ChipGroup` 是一个**多选**组容器：直接子节点 = 芯片（通常用 `Builder::chip_group`
造按钮），**每个芯片独立开/关**，互不影响。开关状态是**值**，由 App 持有
（`UiState::chips`，芯片 id → bool，**表里没有 = 关**）；控件通过
`UiEvent::ChipToggled { id, chip, on }` 报告每次翻转。

什么时候**不**该用它：

- 只允许选一个 → [`Segmented`](segmented.md)（互斥单选）；
- 页签 + 切换内容 → [`TabBar`](tab-bar.md)；
- 芯片带「点 × 删除」语义（邮件标签那种）→ 本期没做（开放问题），不要以为能跑。

## 2. 最小示例

```rust
use deer_gui::prelude::*;

let mut app = Builder::new(Kind::Column, "app").padding(8.0).gap(6.0);
let chips = app.chip_group_opts("tags", L::new().gap(4.0).to_props(), &["红", "蓝", "绿"]);
// chips = ["button_1", "button_2", "button_3"] —— 事件与 UiState 用的就是这些 id。
// 初值是 App 的数据：让「红」默认开着就塞表（不塞 = 关）：
// state.chips.insert("button_1".into(), true);
let tree = app.build();
// 点击芯片后 App 在 UiEvent::ChipToggled { id: "tags", chip: .., on: 翻转后的新值 } 拿到结果。
```

能跑的完整版（含翻转/独立性/初值/`Enter` 的自检断言）：

```sh
cargo run -p deer-gui --features testing --example m6_select
```

## 3. 完整 API

| 入口 | 说明 |
|---|---|
| `Builder::chip_group(labels) -> Vec<String>` | 造一组（组 id 自动 `chip_group_N`），返回**芯片 id 列表**（顺序与 labels 一致） |
| `Builder::chip_group_opts(id, layout, labels)` | 指定组 id 与布局（`gap` 防粘连；`id` 传空串 = 自动） |
| `UiState::chips: BTreeMap<芯片id, bool>` | **值**住在哪。键是**芯片自己**的 id（不是组 id —— 每个芯片独立）；表里没有 = 关 |
| `UiEvent::ChipToggled { id, chip, on }` | 每次点击都发（`id`=组、`chip`=芯片节点 id、`on`=**翻转之后**的状态） |

### 语义（本仓标签组承诺的全部行为，逐条有测试）

| 交互 | 行为 | 钉住它的判据 |
|---|---|---|
| 点**关着**的芯片 | `Clicked` + `ChipToggled { on: true }`，`chips[芯片] = true` | `chip_group_toggle_semantics`（tests/m6_select.rs） |
| 点**开着**的芯片 | `Clicked` + `ChipToggled { on: false }` —— **每次点击必翻转发一次**（翻转本身就是动作；与单选的「变了才发」刻意不同，r14b） | 同上 ② |
| 两个芯片 | 互相独立：点蓝不动红 | 同上 ③ |
| 初值 | App 直接塞 `chips`（值是 App 的数据）；初值为开的芯片点击后翻成 off（`on: false`） | 同上 ④ |
| `Enter`（焦点在启用的芯片上） | = 一次点击：翻转 + 发事件。键盘与指针走**同一条**结算路径（r14） | 同上 ⑤ |
| `Tab` / 方向键 | 芯片是 `Button` ⇒ 自动在焦点序里（树序 / T3.1 几何邻近） | testkit 的 T3.1 判据 |
| 禁用芯片 | 整棵子树静默：无 Clicked/无 ChipToggled/不进焦点序 | 既有 r4 判据原样生效 |
| 视觉 | **开 = `accent` 底 + `on_accent` 字；关 = `border` 底 + 正文字**（让位）；hover/pressed 照常；禁用 = `border` + `text_dim`（禁用赢过一切）；`DefaultRenderer`（无状态）不画开关 | `selection_visual_only_tints_group_members`（muted 机制三组共用） |
| 值的寿命 | 按芯片 id 键控、住树外 ⇒ 整树重建不丢且仍可用 | `keep_rebuild_preserves_selection_values` |
| 组外免疫 | 选择映射里塞了组外节点的 id ⇒ 那些节点一个字节都不变 | `plain_buttons_and_default_renderer_are_blind_to_selection_maps` |

### 开放问题（deer-ui 特有行为不可考，登记不臆造）

- deer-ui 的 Chip 是否有「点 × 删除」语义（email 标签式）？本仓芯片只有开/关；
- 是否有「最多选 N 个」的组约束？本仓无（约束是 App 的事 —— 收到 `on: true` 后
  自己改树把超编的芯片禁用即可）；
- 是否有 chip 的图标/着色变体？本仓只有文字 + 两档状态色（`Icon` 属 5e，未做）；
- 无障碍属性（aria-pressed 等）没有任何承载位。

## 4. 自检（怎么确认你真的用对了）

```rust
// testkit 注入（tests/m6_select.rs 的判据形状）：
let step = h.tap(red.as_str())?;
assert!(step.events.contains(&UiEvent::ChipToggled {
    id: "tags".into(),
    chip: red.clone(),
    on: true,   // 首点 = 开（on 是翻转后的值）
}));
let step = h.tap(red.as_str())?;
assert!(step.events.contains(&UiEvent::ChipToggled {
    id: "tags".into(),
    chip: red.clone(),
    on: false,  // 再点 = 关（每次点击都发）
}));
// 初值：直接塞 UiState —— st.chips.insert(red.clone(), true);
```

## 5. 常见坑

| 现象 | 原因 | 怎么改 |
|---|---|---|
| 界面上芯片全是「关」的样子 | `chips` 表是空的（表里没有 = 关） | 塞 `state.chips` 给需要的芯片 `true` |
| 「怎么我点一下发两次事件」 | 没有两发 —— 一次点击一条 `ChipToggled`；看到两条多半是把 `tap`（move+down+up）的中间步也算上了 | 用 `Step.events`（已是三步并集）数变体条数 |
| 想要「至少/最多选 N 个」 | 本仓没有组约束（开放问题） | App 侧实现：收到事件后改自己的数据并重建树（禁用超编芯片） |
| `on` 的含义拿反了 | `on` 是**翻转之后**的新值，不是点击前的旧值 | 以事件为准更新自己的数据，不要自己再取反 |
| 芯片之间粘住 | 组 `gap` 默认 0 | `chip_group_opts` 传 gap |
| 选中态不显示 | 用了无状态的 `DefaultRenderer` | 走 `InteractiveRenderer` + `to_interact_state()`（Harness 已接好） |

## 6. 相关

- 相关功能：[`input`](input.md) §3.5（r14 结算与三张值表）、[`segmented`](segmented.md)（互斥单选；扩 Kind 的裁定在那里写全）、[`tab-bar`](tab-bar.md)（单选页签）
- 内部原理：`crates/deer-gui/src/interaction.rs` 的 `resolve_selection`（r14b）；`crates/deer-gpu/src/interact.rs` 的 muted 档（三组共用同一机制）
- 边界（**做不到什么**）：无「点 × 删除」语义；无最多/至少 N 个的组约束；无图标与着色变体
  （`Icon` 未做）；无 aria-pressed；只认**直接子节点**（嵌套容器内的命中不算芯片，r14d）；
  `scroll` 对组无效（滚动容器仅 `Column`）；无键盘 `Space` 激活（字符键不消费，与 Btn 同边界）。

## 7. 检查清单（发布前过一遍）

- [x] 示例能跑：`cargo run -p deer-gui --features testing --example m6_select` → `exit=0`
- [x] 示例有自检断言（翻转/独立性/初值/组外免疫，全部 assert）
- [x] `FEATURES.md` 已登记（状态 / 指南链接 / 示例命令都对）
- [x] `docs/TUTORIAL.md` 已有对应章节（§3.5 选择类控件）
- [x] 明确写了「做不到什么」
