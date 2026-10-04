# NumberField —— 数值输入框（提交才解析）

> M6 控件族 5d 的第 1 项。`Kind` 扩展（`Kind::NumberField`，[`Kind::Field`] 的数值变体
> —— 不是组合层）：编辑期草稿与 `Field` 同住 `UiState::texts`（复用全套光标/退格/IME
> 机械），**提交时**（失焦 / `Enter`）才解析。deer-ui 源码不在本仓库，这里是「常见 GUI
> 数值输入语义的最小正确版」，deer-ui 特有且不可考的行为一律登记为开放问题（第 3 节末）。

## 1. 这是什么 / 什么时候用它

`Kind::NumberField` 是一个**值输入框**：用户在里面敲的是**数**。编辑中的任何串都只是
**草稿**（住 `UiState::texts`，怎么打字/退格/移动光标/IME 与 `Field` 完全同一套）；
只有**提交**——焦点离开（Tab / Escape / 点到别处）或按 `Enter`——才解析：
解析成功 ⇒ 发 `UiEvent::NumberChanged { id, value: f64 }`（已按值域夹取）并把草稿
**规范化回写**（`" 42 " → "42"`、`"-1.50" → "-1.5"`）；解析失败 ⇒ **什么都不发**，
草稿保留，绘制侧画一条「不可解析」下划线。

什么时候**不**该用它：

- 要**拖着改**的数值（滑块手感）→ [`ScrubNum`](scrub-num.md)；
- 要普通文本（不解析）→ `Builder::field`（普通 `Kind::Field`，输入即值）；
- 要带上下步进按钮的数字框（spinbox）→ 本期不做（见「做不到什么」）。

## 2. 最小示例

```rust
use deer_gui::prelude::*;

let mut app = Builder::new(Kind::Column, "app").padding(8.0).gap(6.0);
let age = app.number_field_opts("age", "0", |_| {});
// age = "age" —— 事件与 UiState 用的就是它；label 是占位串。
// 值域是 App 的数据：想夹取就塞表（不塞 = 无值域、草稿原样提交）：
// state.num_opts.insert("age".into(), NumOpts { min: Some(0.0), max: Some(150.0), step: 1.0 });
let tree = app.build();
// 提交成功后 App 在 UiEvent::NumberChanged { id: "age", value } 拿到 f64；
// 草稿已被规范化回写（state.texts["age"] == "150"）。
```

能跑的完整版（含提交/失败/失焦夹取/拖动/开关/颜色的自检断言）：

```sh
cargo run -p deer-gui --features testing --example m6_values
```

## 3. 完整 API

| 入口 | 说明 |
|---|---|
| `Builder::number_field(label) -> String` | 造一个（id 自动 `number_field_N`），返回节点 id |
| `Builder::number_field_opts(id, label, f)` | 指定 id 与就地改节点（宽度/禁用等；`id` 传空串 = 自动） |
| `UiState::texts`（草稿） | 编辑期草稿在哪。表里没有 = 空串；与 `Field` 同一张表、同一套光标机械 |
| `UiState::num_opts: BTreeMap<id, NumOpts>` | **值域/步长**住哪。表里没有 = 无值域、步长 1.0；`NumOpts { min, max, step }`，min/max 是 `Option<f64>`（`None` = 该边不夹），`step` 是 [`ScrubNum`](scrub-num.md) 用的（本控件不用步长） |
| `UiEvent::NumberChanged { id, value }` | **提交成功**（失焦 / `Enter`）时发；`value` = **夹取后**的 f64。失败不发；成功后的规范化回写**不发** `TextChanged` |

### 为什么扩 `Kind` 而不是复用 `Field`（裁定 + 理由）

1. **交互层必须认出「这是个数」**才发得出 `NumberChanged`（提交时解析、失败静默都是
   `Kind` 分支的行为；组合层没有识别通道）；
2. **「不可解析」是新的绘制档**（下划线标记）—— 绘制侧也要按 `Kind` 区分。
   解析/格式化的**唯一实现在 `deer_core::values`**（`parse_num`/`format_num`）：
   交互层判「发不发事件」与绘制层判「画不画标记」用**同一份**，「界面说非法、
   事件却发出去了」这种分叉在结构上不可能。

### 语义（本仓数值输入承诺的全部行为，逐条有测试）

| 交互 | 行为 | 钉住它的判据 |
|---|---|---|
| 打字 / 退格 / 光标 / IME | 与 `Field` **同一套**（草稿进 `texts`，逐字符发 `TextChanged`） | `number_field_edits_a_draft_and_commits_on_enter` |
| `Enter`（焦点在它上） | 提交：解析 → 夹值域 → 发 `NumberChanged` → 规范化回写；失败 ⇒ 静默 + 草稿保留 | 同上 + `number_field_rejects_unparseable_and_keeps_the_draft` |
| 失焦（Tab / Escape / 点到别处） | **同一次提交**（`handle` 末尾的失焦钩子，blur 的全部来源都经过焦点变化） | `number_field_commits_on_blur_and_clamps_into_the_range` |
| 解析规则 | `trim` 后交 `f64::from_str`（`3` / `-1.5` / `1e3` / `+7` / `inf` 都行）；**`NaN` 判失败**；空串 = 「还没输入」⇒ 提交静默、**不**画标记 | `deer-core` 的 `values.rs` 单测 |
| 值域 | 提交时夹取（`min`/`max` 各自独立；倒置时上界赢）；**只在提交/拖动时夹**，打字过程不拦 | 失焦夹取判据 |
| 视觉 | 与 `Field` 同壳（填充/1px 边框/焦点环/光标/IME 预编辑）；草稿**非空且不可解析** ⇒ 文本区底部 1px 下划线（`text_dim`）；`DefaultRenderer`（无状态）= 带占位标签的输入框，不画标记 | `invalid_draft_underline_and_color_swatch_follow_the_displayed_text` |
| 禁用 | 整棵子树静默（点不到、不进焦点序 —— 既有规则原样生效） | 既有 r4/r12 判据 |
| 值的寿命 | 草稿按 id 键控住树外 ⇒ 整树重建不丢且仍可用（Keep） | `keep_rebuild_preserves_value_tables` |

### 开放问题（deer-ui 特有行为不可考，登记不臆造）

- 是否有步进按钮（spinbox）/上下方向键步进？本仓方向键上下是焦点导航（T3.1），
  且焦点在输入框上时被让给文本语义 —— 本期**不做**步进；
- 千分位 / 固定小数位等**格式化配置**没有承载位（`format_num` 用 Rust 默认浮点格式）；
- `inf` 算合法值（能被原样读回）；要不要在值域层默认禁 `inf` —— 登记为开放问题。

## 4. 自检（怎么确认你真的用对了）

```rust
// testkit 注入（tests/m6_values.rs 的判据形状）：
h.tap("age")?;                                  // 点击聚焦
h.send(&InputEvent::TextInput { text: "999".into() })?;   // 草稿
let step = h.tap("button_1")?;                  // 点别处 = 失焦 ⇒ 提交
assert!(step.events.contains(&UiEvent::NumberChanged { id: "age".into(), value: 150.0 }));
h.assert_text("age", "150")?;                   // 夹取 + 规范化回写
// 失败档：打 "3px" → Enter ⇒ 无事件、草稿保留。
```

## 5. 常见坑

| 现象 | 原因 | 怎么改 |
|---|---|---|
| 打完字没有 `NumberChanged` | 还没提交 —— 本控件**只在失焦 / `Enter`** 时解析（输入即值的直觉来自普通 `Field`，这里不是） | 失焦（Tab/点别处）或按 `Enter` |
| `Enter` 后草稿变了（`" 42 "` 变 `"42"`） | 提交成功后的**规范化回写**（刻意行为，不发 `TextChanged`） | 以 `NumberChanged.value` 为准；草稿只是显示 |
| 提交了但事件里没有值 | 草稿不可解析（`"3px"` / 空 / `NaN`）⇒ 静默 + 下划线标记 | 看画面上的下划线；拿草稿对 `parse_num` 的规则自检 |
| 999 没报错而是变成 150 | 值域夹取（`num_opts` 里有 `max: Some(150)`）—— 刻意行为 | 不想夹取就不塞 `num_opts` |
| 界面说非法但 App 想自己拦 | 「非法」的判据只有一份（`deer_core::values::parse_num`）| 直接调它，别另写一套 |

## 6. 相关

- 相关功能：[`input`](input.md) §3.5（值类控件事件表；普通 `Field` 的草稿机械也在它的 §3.4）、[`scrub-num`](scrub-num.md)（同一张 `num_opts` 表的另一个读者）、[`color-field`](color-field.md)（同构的提交式输入）
- 内部原理：`crates/deer-core/src/values.rs`（唯一解析实现）；`crates/deer-gui/src/interaction.rs` 的 `commit_value_field` 与失焦钩子
- 边界（**做不到什么**）：无步进按钮/方向键步进；无千分位/固定小数位格式化；无单位解析（`"3px"` 就是非法）；无多行；`NaN`/空串按「不可解析」处理（不发事件）；
  普通文本输入请用 `Field`（它的 `Enter` 没有任何语义）；焦点在它上面时方向键上下**不**做焦点导航（留给文本语义，与 `Field` 同边界）。

## 7. 检查清单（发布前过一遍）

- [x] 示例能跑：`cargo run -p deer-gui --features testing --example m6_values` → `exit=0`
- [x] 示例有自检断言（提交/夹取/失败静默/拖动/三路翻转/颜色提交，全部 assert）
- [x] `FEATURES.md` 已登记（状态 / 指南链接 / 示例命令都对）
- [x] `docs/TUTORIAL.md` 已有对应小节（§3 数值类控件）
- [x] 明确写了「做不到什么」
