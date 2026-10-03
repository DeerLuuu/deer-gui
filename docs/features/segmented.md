# Segmented —— 分段选择（互斥单选）

> M6 控件族 5c 的第 1 项。`Kind` 扩展（`Kind::Segmented`，**不是**组合层）——
> 为什么扩而不组合，见第 3 节「语义」前的裁定段；deer-ui 源码不在本仓库，
> 这里是「常见 GUI 分段选择语义的最小正确版」，deer-ui 特有且不可考的行为
> 一律登记为开放问题（第 3 节末），不臆造。

## 1. 这是什么 / 什么时候用它

`Kind::Segmented` 是一个**互斥单选**组容器：直接子节点 = 段（通常用 `Builder::segmented`
造按钮），点一个段 = 选中它；**同一时刻最多一个选中段**。选中是**值**，由 App 持有
（`UiState::segments`，texts 同一条纪律）；控件只通过 `UiEvent::SelectionChanged` 报告。

什么时候**不**该用它：

- 要多选（每项独立开/关）→ [`ChipGroup`](chip-group.md)；
- 要「页签 + 切换内容」→ [`TabBar`](tab-bar.md)（Segmented 只报选中，不管内容）；
- 一排**互不影响**的动作按钮 → [`RowActions`](row-actions.md)（那不是选择，是命令）。

## 2. 最小示例

```rust
use deer_gui::prelude::*;

let mut app = Builder::new(Kind::Column, "app").padding(8.0).gap(6.0);
let seg = app.segmented_opts("mode", L::new().gap(2.0).to_props(), &["日", "周", "月"]);
// seg = ["button_1", "button_2", "button_3"] —— 事件与 UiState 用的就是这些 id。
// 初值是 App 的数据：想默认选中「周」就塞表（不塞 = 没有选中段，也是合法状态）：
// state.segments.insert("mode".into(), "button_2".into());
let tree = app.build();
// 点击「月」后 App 在 UiEvent::SelectionChanged { id: "mode", selected: .. } 拿到新选中段。
```

能跑的完整版（含点击/换选/禁用/`Enter`/方向键/视觉红线的自检断言）：

```sh
cargo run -p deer-gui --features testing --example m6_select
```

## 3. 完整 API

| 入口 | 说明 |
|---|---|
| `Builder::segmented(labels) -> Vec<String>` | 造一组（组 id 自动 `segmented_N`），返回**段 id 列表**（顺序与 labels 一致） |
| `Builder::segmented_opts(id, layout, labels)` | 指定组 id 与布局（`gap` 防粘连；`id` 传空串 = 自动） |
| `UiState::segments: BTreeMap<组id, 段id>` | **值**住在哪。表里没有 = 没有选中段；App 直接读写 |
| `UiEvent::SelectionChanged { id, selected }` | 换选中时发（`id`=组、`selected`=**新段**的节点 id，不是标签文本） |

### 为什么扩 `Kind` 而不是组合层（裁定 + 理由）

RowActions（5a）选择了组合层，Segmented **扩了** `Kind`，差别在于两条判据：

1. **交互层必须在树里认出「这是一组选择」**，才发得出 `SelectionChanged`
   （组合层 = Row + button，`handle` 看到的就是普通按钮点击，没有任何识别通道；
   给 `NodeProps` 加「角色」字段要同步 registry/scene/.dui 三处，代价更大还得多一个字段）；
2. **「选中」是本系统第一种持久视觉**（hover/pressed/focus 全是瞬态）——绘制侧需要新的一档
   状态色，满足「需要新视觉形态才扩 `Kind`」的判据（DEV-PLAN Phase 5 验收标准）。

同步面（compiler 强制，穷尽 match 一个都逃不掉）：`Kind` 枚举/as_str/parse/is_container/
is_horizontal、registry 的容器集合、layout 的 measure/place、`DefaultRenderer` 与
`InteractiveRenderer` 的绘制分支、`IdGen` 的 kind 计数表。

### 语义（本仓分段选择承诺的全部行为，逐条有测试）

| 交互 | 行为 | 钉住它的判据 |
|---|---|---|
| 点**新**段 | `Clicked(段)` + `SelectionChanged { 组, 新段 }`，`segments[组] = 新段` | `segmented_selection_semantics`（tests/m6_select.rs） |
| 点**已选中**段 | 只有 `Clicked` —— 没有变化就不发值事件（与 `Scrolled`「变了才发」同纪律） | 同上 |
| `Enter`（焦点在启用的段上） | = 一次点击：`Clicked` + 换选中。键盘与指针走**同一条**结算路径（r14） | 同上 ④ |
| `Tab` / `Shift+Tab` / 方向键上下 | 段是 `Button` ⇒ 自动在焦点序里（树序 / T3.1 几何邻近）；**Tab/方向键只挪焦点，不改选中** | 同上 ④ + testkit 的 T3.1 判据 |
| 禁用段（自身或祖先 `disabled`） | 整棵子树静默：无 Clicked/无 SelectionChanged/不进焦点序（既有规则原样生效，不设第二道闸门） | testkit 判据 ② |
| 按住拖出再抬起 | 按捕获者结算（D7）—— 拖出去也算它的点击与选择 | 既有 r3 判据 + r14 同路 |
| 视觉 | 选中段 = `accent` 底 + `on_accent` 字（与普通按钮同款）；**未选中段 = `border` 底 + 正文字**（让位）；hover 提亮 / pressed 加深照常；禁用 = `border` 底 + `text_dim` 字（禁用赢过一切）；`DefaultRenderer`（无状态）不画选中 | `selection_visual_only_tints_group_members` + 像素判据（差异只落在新旧两个选中段矩形内） |
| 值的寿命 | 按组 id 键控、住树外 ⇒ 整树重建不丢且仍可用（Keep） | `keep_rebuild_preserves_selection_values` |
| 组外免疫 | 选择映射里塞了组外节点的 id ⇒ 那些节点一个字节都不变（opt-in 红线） | `plain_buttons_and_default_renderer_are_blind_to_selection_maps` |

### 开放问题（deer-ui 特有行为不可考，登记不臆造）

- deer-ui 的 Segmented 是否支持**左右方向键在段间移动**？本仓方向键只有上下（T3.1），
  左右被 `Field` 光标占用 ⇒ 段间横向移动本期只能 `Tab`；
- 段宽是否**均分**（等宽铺满组）？本仓按内容 + `min_w` 自然宽度排布（Row 数学）；
- 是否有 radio 语义的无障碍属性（aria-role）？`NodeProps` 没有任何承载位；
- 是否支持竖排（垂直分段）？本仓组固定横排（`is_horizontal`）；竖排未做。

## 4. 自检（怎么确认你真的用对了）

```rust
// testkit 注入（tests/m6_select.rs 的判据形状）：
let step = h.tap(week.as_str())?;
assert!(step.events.contains(&UiEvent::Clicked(week.clone())));
assert!(step.events.contains(&UiEvent::SelectionChanged {
    id: "mode".into(),
    selected: week.clone(),
}));
// 再点同一个段：只有 Clicked（变了才发）。
// 初值：直接塞 UiState（值是 App 的数据）：
// st.segments.insert("mode".into(), week.clone());
```

App 侧对应关系：拿着 `SelectionChanged.selected`（= 构造时返回的那个段 id）查你自己的数据；
`TabChanged`/`ChipToggled` 见各自指南。

## 5. 常见坑

| 现象 | 原因 | 怎么改 |
|---|---|---|
| 界面上没有任何段「亮着」 | `segments` 表是空的（没塞初值，也没点过） | 塞 `state.segments`（表里没有 = 没有选中段，这是合法状态不是 bug） |
| 点了段但事件里没有 `SelectionChanged` | 点的是**已选中**段（变了才发），或该段在禁用子树里 | 先看 `Clicked` 在不在：在 ⇒ 是「无变化」；不在 ⇒ 查禁用/裁剪 |
| 想按标签文本判断选中 | 事件与值表用的都是**节点 id**，不是标签 | 用 `segmented()` 返回的 id 列表对号；标签可以重名，id 不会 |
| 段之间粘成一条 | 组 `gap` 默认 0 | `segmented_opts` 传 `L::new().gap(2.0).to_props()` |
| 选中态不显示 | 用了 `DefaultRenderer` / `build_draw_list`（无状态渲染器，刻意不画选中） | 走 `InteractiveRenderer` + `UiState::to_interact_state()`（testkit 的 Harness 已接好） |

## 6. 相关

- 相关功能：[`input`](input.md) §3.5（r14 结算与三张值表）、[`chip-group`](chip-group.md) / [`tab-bar`](tab-bar.md)（同批的另两种选择组）、[`btn`](btn.md)（段就是按钮 —— 点击/焦点语义全部来自它）
- 内部原理：`crates/deer-gui/src/interaction.rs` 的 `resolve_selection`（r14a–r14e）；`crates/deer-gpu/src/interact.rs` 的 muted 档
- 边界（**做不到什么**）：无多选（那是 ChipGroup）；无竖排；无段宽均分；无左右方向键段间导航；
  无 aria/radio 角色；**只认直接子节点** —— 段内再嵌容器，命中落在更深层时不算选中段
  （不上溯，r14d；复杂段内容的语义登记为开放问题）；无「点选中段再点一次取消选中」
  （单选不允许空选 —— 想可取消就改用 ChipGroup）；`scroll` 对组无效（滚动容器仅 `Column`）。

## 7. 检查清单（发布前过一遍）

- [x] 示例能跑：`cargo run -p deer-gui --features testing --example m6_select` → `exit=0`
- [x] 示例有自检断言（点击/换选/禁用/Enter/组外免疫/DefaultRenderer 无状态，全部 assert）
- [x] `FEATURES.md` 已登记（状态 / 指南链接 / 示例命令都对）
- [x] `docs/TUTORIAL.md` 已有对应章节（§3.5 选择类控件）
- [x] 明确写了「做不到什么」
