# Keep —— 重建树时保留瞬态交互状态（一个保证，不是控件）

> M6 控件族 5a 的第 3 项。**本轮裁定：纯文档 + 测试交付，零新代码** ——
> 理由见第 3 节的「为什么不是一个属性」。这个页面把既有保证**显式写下来**，
> 并有测试钉住（此前这条承诺没有任何判据）。

## 1. 这是什么 / 什么时候用它

Keep 回答一个问题：**App 重建了整棵界面树，用户的输入状态还在吗？**

答案：**在**。只要节点的 id 稳定（`IdGen` 确定性 ⇒ 同一规格重建出同一批 id），
`UiState` 里按 id 键控的状态全部保留且仍然可用：

| 状态 | 重建后 | 归属 |
|---|---|---|
| `texts`（输入框内容） | 保留，继续可编辑 | **应用数据**（不是 UI 瞬态） |
| `carets`（光标，字符位） | 保留 | UI 瞬态（键控 by id） |
| `focus`（焦点） | 保留；焦点节点被删 ⇒ 空转（输入不落、`Tab` 恢复到树序第一个） | UI 瞬态 |
| `scroll.offsets`（滚动偏移） | 保留；重新灌 `max_scroll` 后夹回界内 | UI 瞬态（偏移是布局输入） |
| `hover` / `pressed` | 不保留也不需要：捕获在抬起时结算/释放，hover 随下一次移动恢复 | 纯瞬态 |

什么时候**不**该指望它：控件从树里**删掉**了 —— 值还在 `texts` 里（库不偷删，
它是应用数据），但输入空转；清理孤儿值是 App 的责任。

## 2. 最小示例

App 的重建惯例就是「内容变了整树重建」—— 什么都不用做，状态自动跟过去：

```rust
use deer_gui::testing::Harness;

let mut h = Harness::new(build_tree(), 260, 220, Theme::default());
h.tap("field_1")?;                                   // 聚焦输入框
h.send(&InputEvent::TextInput { text: "你好".into() })?;

h.set_tree(build_tree());                            // ★ 整树重建（Harness 保留 state）
h.assert_focus(Some("field_1"))?;                    // 焦点还在
h.assert_text("field_1", "你好")?;                   // 内容还在
h.send(&InputEvent::TextInput { text: "!".into() })?;
h.assert_text("field_1", "你好!")?;                  // 而且还能继续输入
```

能跑的完整版（含重建 + 断言）：

```sh
cargo run -p deer-gui --features testing --example m6_basics
```

## 3. 完整 API

**没有 API。** Keep 不是一个类型、不是一个属性、不进注册表 —— 它是
「`UiState` 按 id 键控 + `IdGen` 确定性」两条既有事实的**显式声明**。

### 语义（最小正确版，本仓定义）

> deer-ui 源码不在本仓库，其 `Keep` 的原始语义不可考；以下是**本仓可证明的
> 最小正确版**，凡超出它的都是「做不到」。

1. **id 稳定的子树**：重建（新 `Builder`、新几何、重新灌滚动上限）后，
   `texts`/`carets`/`focus`/`scroll.offsets` 逐值保留，**且仍然可用**
   （继续输入落到同一个框、滚轮继续滚同一个容器）——
   判据：`tests/m6_basics.rs::keep_rebuilding_the_tree_preserves_texts_focus_and_scroll`；
2. **控件被删除**：值作为应用数据残留（`texts["field_1"]` 原样）、stale 焦点下
   输入空转（不追加、不发事件）、`Tab` 从树序第一个恢复 ——
   判据：`keep_removed_field_keeps_its_value_and_focus_recovers`；
3. **按下途中重建**：抬起仍按**捕获者 id** 结算 `Clicked`（App 收到的 id 可能
   已不在新树里 —— 容忍它，或忽略，语义归 App）；
4. **为什么不是一个 `Keep` 属性**：保证是**全局**的（没有「没标 Keep 就丢状态」
   这回事）；加一个属性就要同步 Kind/registry/scene/绘制四处，却零行为 ——
   一个 no-op 字段只会诱导「标了才安全」的错误心智模型。所以：文档 + 测试，
   不加代码。

### 开放问题（deer-ui 特有行为不可考，登记不臆造）

- 若 deer-ui 的 `Keep` 指「**隐藏但保活**」（组件不卸载、只是不可见），那本仓
  **没有对应物**：节点在树里就参与布局与绘制，没有 display:none —— 要「暂时
  不可见」得由 App 把子树从树里拿掉（值按第 2 条保留）。

## 4. 自检（怎么确认你真的用对了）

```sh
cargo test -p deer-gui --test m6_basics keep_
```

两条测试：重建保留（texts/焦点/光标/滚动偏移 + 可用性）、删除边界（值残留 +
空转 + `Tab` 恢复）。App 侧自查清单：重建用**同一套构造代码**（id 才稳定）；
显式命名的节点别改名；滚动记得每帧 `set_metrics` 灌回上限（不灌滚轮 fail-closed）。

## 5. 常见坑

| 现象 | 原因 | 怎么改 |
|---|---|---|
| 重建后「状态丢了」 | id 漂了：构造顺序/显式命名变了 ⇒ `IdGen` 生出**另一批** id，旧状态成了孤儿 | 保持同一套构造函数；显式 id 别改名；用 `structurally_eq` 比对新旧树 |
| 删了输入框但 `texts` 里还有它的值 | 设计如此：值是应用数据，库不偷删 | App 删控件时顺手清 `state.texts`（或保留以便恢复） |
| 焦点还在却打不进字 | 焦点指向已被删除的节点（stale）—— 输入空转 | `Tab` 一下即恢复；或 App 在重建时主动清/改 `focus` |
| 滚动位置没保住 | 忘了每帧 `set_metrics` 灌上限（上限表空 ⇒ 偏移被夹成 0） | 重建帧也要灌；见 `scroll-and-multiline.md` |

## 6. 相关

- 相关功能：[`input`](input.md)（`UiState` 是唯一真相）、[`scroll-and-multiline`](scroll-and-multiline.md)（`set_metrics` 接线）、[`node-tree`](node-tree.md)（id 规则与确定性）
- 内部原理：`crates/deer-gui/src/interaction.rs`（`UiState` 字段文档）、`crates/deer-gui/tests/m6_basics.rs`（判据）
- 边界（**做不到什么**）：没有「隐藏但保活」；不保留布局/像素（重建后画面由新树
  + 保留的状态共同决定）；`hover`/`pressed` 不跨重建（瞬态，事件流自然恢复）；
  不能「只保留一部分控件的状态」（没有标记机制 —— 这是裁定，不是遗漏）。

## 7. 检查清单（发布前过一遍）

- [x] 示例能跑：`cargo run -p deer-gui --features testing --example m6_basics` → `exit=0`
- [x] 示例有自检断言（重建后 focus/texts 保留 + 继续输入可用）
- [x] `FEATURES.md` 已登记（状态 / 指南链接 / 示例命令都对）
- [x] `docs/TUTORIAL.md` 未加新章（不属于新手主线；入口在本页与 FEATURES）
- [x] 明确写了「做不到什么」
