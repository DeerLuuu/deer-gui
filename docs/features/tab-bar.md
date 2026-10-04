# TabBar —— 页签栏

> M6 控件族 5c 的第 3 项。`Kind` 扩展（`Kind::TabBar`）—— 扩 Kind 的裁定与理由
> 与 [`Segmented`](segmented.md) 完全同源（交互层识别 + 新持久视觉），见该指南第 3 节。
> deer-ui 源码不在本仓库，这里是「常见 GUI 页签栏语义的最小正确版」：
> **TabBar 只报告选中页，内容切换是 App 的事**；deer-ui 特有且不可考的行为登记为
> 开放问题（第 3 节末），不臆造。

## 1. 这是什么 / 什么时候用它

`Kind::TabBar` 是一个**单选页签**组容器：直接子节点 = 页签（按钮），点一个页签 =
活动页换成它，`UiEvent::TabChanged { id, index }` 把**新活动页的下标**报给 App ——
然后 App 拿着 index 渲染自己的内容（本仓的树里**没有**「页内容」这个概念，
那是 App 的数据与树，别在 TabBar 里找）。

什么时候**不**该用它：

- 选择不换「页面内容」 → [`Segmented`](segmented.md)（同是单选，语义是「选一个值」）；
- 多选 → [`ChipGroup`](chip-group.md)；
- 需要可关闭/可拖拽重排的页签 → 本期没做（开放问题）。

## 2. 最小示例

```rust
use deer_gui::prelude::*;

let mut app = Builder::new(Kind::Column, "app").padding(8.0).gap(6.0);
let tabs = app.tab_bar_opts("tabs", L::new().gap(2.0).to_props(), &["文件", "编辑", "视图"]);
// tabs = ["button_1", "button_2", "button_3"] —— 事件用的 id。
let tree = app.build();
// 点击「视图」⇒ UiEvent::TabChanged { id: "tabs", index: 2 }
// （index = 页签在组直接子节点里的树序下标，**禁用页也一起数** —— App 自己的
//   页签数组含禁用项，按同一个下标对齐。）
```

**禁用单个页签**：便捷构造 `tab_bar` 不带禁用参数（两参数塞不下「第几个禁用」这种
逐项差异），需要时手写这个组 —— 两条路径产出**结构相等**的树（有判据）：

```rust
use deer_gui::prelude::*;

let mut app = Builder::new(Kind::Column, "app");
app.container_opts(Kind::TabBar, "tabs", L::new().gap(2.0).to_props(), |g| {
    g.button("文件");
    g.button_opts("编辑", |n| n.props.disabled = true); // 这页禁用
    g.button("视图");
});
let tree = app.build();
```

能跑的完整版（含换页/禁用页静默/index 计数/`Enter` 的自检断言）：

```sh
cargo run -p deer-gui --features testing --example m6_select
```

## 3. 完整 API

| 入口 | 说明 |
|---|---|
| `Builder::tab_bar(labels) -> Vec<String>` | 造一组（组 id 自动 `tab_bar_N`），返回**页签 id 列表**（顺序与 labels 一致） |
| `Builder::tab_bar_opts(id, layout, labels)` | 指定组 id 与布局（`id` 传空串 = 自动） |
| `container_opts(Kind::TabBar, …)` + `button_opts(…, \|n\| n.props.disabled = true)` | 手写带禁用页的组（与便捷构造结构相等） |
| `UiState::tabs: BTreeMap<组id, 页id>` | **值**住在哪（存节点 id，不是下标 —— 标签重名、树增删页都不会错位）；表里没有 = 没有活动页 |
| `UiEvent::TabChanged { id, index }` | 换页时发（`id`=组、`index`=新活动页在组**直接子节点**里的树序下标，**禁用页也计数**） |

### 语义（本仓页签栏承诺的全部行为，逐条有测试）

| 交互 | 行为 | 钉住它的判据 |
|---|---|---|
| 点**非活动**页 | `Clicked(页)` + `TabChanged { 组, index }`，`tabs[组] = 页`。**内容切换是 App 的事** —— TabBar 只报告 | `tab_bar_semantics_and_disabled_tabs`（tests/m6_select.rs） |
| 点**当前活动**页 | 只有 `Clicked` —— 没有变化就不发值事件（r14c） | 同上 ④ |
| **禁用页**（该页签自身 `disabled`） | 静默：无 Clicked/无 TabChanged/不进焦点序（既有禁用子树规则原样生效） | 同上 ② |
| `index` 的口径 | **含禁用页**一起数（App 的页签数组含禁用项，按同一树序对齐）——「视图」在「编辑」被禁用时 index 仍是 2 | 同上 ③（变异 M-C 钉过反向） |
| `Enter`（焦点在启用的页签上） | = 一次点击：`Clicked` + 换页。键盘与指针走**同一条**结算路径（r14） | 同上 ⑤ |
| `Tab` / 方向键 | 页签是 `Button` ⇒ 自动在焦点序里（树序 / T3.1 几何邻近；禁用页不在序里） | 同上 ⑤ + testkit 的 T3.1 判据 |
| 视觉 | **活动页 = `accent` 底 + `on_accent` 字；非活动页 = `border` 底 + 正文字**；hover/pressed 照常；禁用 = `border` + `text_dim`；`DefaultRenderer`（无状态）不画活动页 | `selection_visual_only_tints_group_members`（muted 机制三组共用） |
| 值的寿命 | 按组 id 键控、住树外 ⇒ 整树重建不丢且仍可用 | `keep_rebuild_preserves_selection_values` |
| 组外免疫 | 选择映射里塞了组外节点的 id ⇒ 那些节点一个字节都不变 | `plain_buttons_and_default_renderer_are_blind_to_selection_maps` |

### 开放问题（deer-ui 特有行为不可考，登记不臆造）

- deer-ui 的 TabBar 是否支持**可关闭页签**（× 按钮）/ 拖拽重排 / 溢出时的滚动或折叠？
  本仓全都没有（滚动容器仅 `Column`，组不可滚）；
- 是否支持**竖排页签**？本仓组固定横排（`is_horizontal`）；
- 活动页初值语义：本仓「表里没有 = 没有活动页」（合法状态，App 塞初值）；
  deer-ui 是否强制恒有活动页不可考；
- 无障碍属性（tab role / aria-selected）没有任何承载位。

## 4. 自检（怎么确认你真的用对了）

```rust
// testkit 注入（tests/m6_select.rs 的判据形状）：
let step = h.tap(files.as_str())?;
assert!(step.events.contains(&UiEvent::Clicked(files.clone())));
assert!(step.events.contains(&UiEvent::TabChanged { id: "tabs".into(), index: 0 }));
// 禁用页：点了必须什么都发不出来（连焦点都不换）。
// 初值：直接塞 UiState —— st.tabs.insert("tabs".into(), files.clone());
```

App 侧对应关系：拿 `TabChanged.index` 换自己渲染的内容（`data.page = index`）——
这就是「内容切换由 App 做」的全部代码。

## 5. 常见坑

| 现象 | 原因 | 怎么改 |
|---|---|---|
| 「点了页签怎么内容没换」 | TabBar 只报告，**换内容是 App 的事**（设计如此，不是 bug） | 在 `TabChanged` 处理里更新 App 数据并重建树 |
| index 和自己的页签数组对不上 | 用了「只数启用页」的口径（本仓刻意相反：**禁用页也计数**） | 页签数组含禁用项时按同一树序对齐；判据钉过这条（变异 M-C） |
| 界面上没有页签「亮着」 | `tabs` 表是空的（没塞初值，也没点过） | 塞 `state.tabs`（没有活动页是合法状态） |
| 想禁用某一页，`tab_bar()` 却做不到 | 便捷构造不带禁用参数（刻意） | 用 `container_opts(Kind::TabBar, …)` + `button_opts` 手写（结构等价有判据） |
| 活动页不亮 | 用了无状态的 `DefaultRenderer` | 走 `InteractiveRenderer` + `to_interact_state()`（Harness 已接好） |
| 挪焦点时页签跟着换页 | 不会 —— `Tab`/方向键只挪焦点，换页只属于点击/`Enter` 激活 | 拿 `FocusChanged` 与 `TabChanged` 分开处理 |

## 6. 相关

- 相关功能：[`input`](input.md) §3.5（r14 结算与三张值表）、[`segmented`](segmented.md)（同为单选；扩 Kind 的裁定在那里写全）、[`chip-group`](chip-group.md)（多选开关）
- 内部原理：`crates/deer-gui/src/interaction.rs` 的 `resolve_selection`（r14c）；`crates/deer-gpu/src/interact.rs` 的 muted 档（三组共用同一机制）
- 边界（**做不到什么**）：无页内容容器（内容是 App 的树，本仓没有「页」概念）；
  无可关闭页签/拖拽重排/溢出滚动/竖排；无 aria-selected；只认**直接子节点**
  （嵌套容器内的命中不算页签，r14d）；`scroll` 对组无效（滚动容器仅 `Column`）。

## 7. 检查清单（发布前过一遍）

- [x] 示例能跑：`cargo run -p deer-gui --features testing --example m6_select` → `exit=0`
- [x] 示例有自检断言（换页/禁用页静默/index 计数/组外免疫，全部 assert）
- [x] `FEATURES.md` 已登记（状态 / 指南链接 / 示例命令都对）
- [x] `docs/TUTORIAL.md` 已有对应章节（§3.5 选择类控件）
- [x] 明确写了「做不到什么」
