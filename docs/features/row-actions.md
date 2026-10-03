# RowActions —— 行尾动作按钮组（组合层）

> M6 控件族 5a 的第 2 项。**刻意不扩 `Kind`**：RowActions = `Row` + 若干 `button`
> 的便捷构造，渲染、布局、命中、事件全部复用既有路径 —— 判据是「与手写等价树
> 结构相等 + 绘制命令逐条相同」。

## 1. 这是什么 / 什么时候用它

列表/表单行尾的那组动作按钮（「编辑 / 删除」）的**样板收口**：一次调用建出
一个 `Row` + 每个标签一个 `Button`，返回按钮 id 列表（顺序与标签一致）供事件关联。

什么时候**不**该用它：

- 需要非默认布局的复杂行（比如按钮之外还要图标、文本）→ 直接写 `Row` + 控件，
  `row_actions` 只是等价书写，没有额外能力；
- 需要溢出折叠（按钮太多收成「+N」）或分隔线 → **没做**（见「做不到什么」）。

## 2. 最小示例

```rust
use deer_gui::prelude::*;

let mut app = Builder::new(Kind::Row, "row").main(Align::End); // 组贴行尾
// 返回 ["button_1", "button_2"]（顺序与标签一致；Row 自身 id 自动生成 row_N）
let actions = app.row_actions(&["编辑", "删除"]);

// 常用形态：指定 Row 的 id 与布局（gap 防粘连、main 让组贴到父行尾）
let actions = app.row_actions_opts(
    "actions",
    L::new().gap(8.0).main(Align::End).to_props(),
    &["编辑", "删除"],
);
// 点击后 UiEvent::Clicked(actions[i]) 与普通按钮完全同一条路径
```

能跑的完整版（自检断言：结构等价 + 绘制命令逐条相同 + tap 得到 `Clicked`）：

```sh
cargo run -p deer-gui --features testing --example m6_basics
```

## 3. 完整 API

| 入口 | 说明 |
|---|---|
| `Builder::row_actions(labels: &[&str]) -> Vec<String>` | 追加一个自动 id 的 Row（默认布局）+ 每标签一个按钮；返回按钮 id 列表 |
| `Builder::row_actions_opts(id, layout, labels) -> Vec<String>` | 同上；`id` 传空串 = 自动生成；`layout` 落在 Row 上（典型：`L::new().gap(8.0).main(Align::End).to_props()`） |
| 空的 `labels` | 合法：得到一个没有子节点的 Row（无 padding ⇒ 零绘制命令） |

### 语义（最小正确版，本仓定义）

- **纯组合**：产出的树与「手写 `container_opts(Row) + button` 逐字等价」——
  判据 `row_actions_builds_exactly_the_hand_written_row_of_buttons`（结构相等）
  与 `row_actions_draw_list_is_identical_to_the_hand_written_row`（绘制列表
  `PartialEq` 逐条相同）双钉；
- **没有新事件、没有新状态**：每个按钮照常发 `UiEvent::Clicked(它自己的 id)`、
  照常进焦点序、禁用照常静默 —— App 拿到的世界与手写一模一样；
- **无 padding 的 Row 自身零绘制命令**：全列表命令数 = 按钮数 × 2
  （每按钮 1 条圆角底 + 1 条文本）—— `m6_basics` 示例打印这组计数；
- **id 规则与两条构筑路径同一份**：自动 id 走 `IdGen` 按 kind 计数（Row 得
  `row_N`、按钮得 `button_N`，跨调用连续、与显式命名互不撞号）。

### 开放问题（deer-ui 特有行为不可考，登记不臆造）

- deer-ui 的 RowActions 是否带溢出折叠/更多菜单？本仓**没有**（溢出需要测量
  与第二阶段布局，等真有需求再立项）；
- 是否带分隔线/图标位？分隔线本仓没有对应绘制原语；图标属 5e `Icon`。

## 4. 自检（怎么确认你真的用对了）

```rust
// 便捷构造与手写等价（tests/m6_basics.rs 的判据形状）：
assert!(convenient.structurally_eq(&hand_written));
assert_eq!(build_draw_list(&convenient, &geo, theme, &m), build_draw_list(&hand_written, &geo, theme, &m));

// 返回的 id 必须真的是树里那排按钮（否则事件关联落空）：
assert_eq!(actions, row.children.iter().map(|c| c.id.clone()).collect::<Vec<_>>());
```

## 5. 常见坑

| 现象 | 原因 | 怎么改 |
|---|---|---|
| 按钮粘成一团 | 默认布局 `gap = 0`（与所有容器一致，没有魔法默认值） | 用 `row_actions_opts` 给 `L::new().gap(8.0)` |
| 组没有贴到行尾 | 「行尾」是**父行**的主轴对齐语义 | 组放进父 Row，用 `main(Align::End)` 表达；`row_actions_opts` 的 `layout` 只管组自己内部 |
| 想要禁用的动作按钮 | `row_actions` 不收每按钮属性（标签进按钮出，保持最小面） | 手写该行，或对整组包一层禁用容器（`props.disabled` 对子树生效） |
| 以为要注册/同步什么 | 没有新 `Kind` —— registry/scene/绘制**都不用动**，这正是组合层裁定 | —— |

## 6. 相关

- 相关功能：[`imperative-api`](imperative-api.md)（容器嵌套）、[`btn`](btn.md)（按钮语义）、[`draw-list`](draw-list.md)（命令计数怎么读）
- 内部原理：`crates/deer-core/src/builder.rs`（构造 + `mod tests`）、`crates/deer-gui/tests/m6_basics.rs`（绘制判据）
- 边界（**做不到什么**）：不收每按钮属性（禁用/显式 id 请手写该行）；无溢出折叠、
  无分隔线、无图标；只产出 `Row`（竖排动作组直接用 `Column` 手写）；`.dui` 场景文件
  **没有** `row_actions` 语法（它是 Rust 构造层的便捷，`.dui` 里照旧写 row + button ——
  两者本来就产出同一棵树）。

## 7. 检查清单（发布前过一遍）

- [x] 示例能跑：`cargo run -p deer-gui --features testing --example m6_basics` → `exit=0`
- [x] 示例有自检断言（结构等价 + 绘制列表逐条相同 + tap 得 `Clicked`）
- [x] `FEATURES.md` 已登记（状态 / 指南链接 / 示例命令都对）
- [x] `docs/TUTORIAL.md` 已更新（§3 加了 `row_actions_opts` 便捷写法）
- [x] 明确写了「做不到什么」
