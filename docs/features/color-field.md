# ColorField —— 颜色输入框（最小版）

> M6 控件族 5d 的第 4 项。`Kind` 扩展（`Kind::ColorField`，最小版）：文本输入
> `#RRGGBB` + 色块预览 —— 编辑期草稿住 `UiState::texts`（与普通 `Kind::Field` 同一套
> 机械），**提交时**（失焦 / `Enter`）按 `#RRGGBB` 解析。完整取色器**不做**
> （见「做不到什么」）。deer-ui 源码不在本仓库，deer-ui 特有且不可考的行为一律
> 登记为开放问题（第 3 节末）。

## 1. 这是什么 / 什么时候用它

`Kind::ColorField` 是一个**颜色输入框**：编辑中的串只是**草稿**（`texts`，打字/退格/
光标/IME 与 `Field` 完全同一套）；**提交**（焦点离开 / `Enter`）时按 `#RRGGBB` 解析：
成功 ⇒ 发 `UiEvent::ColorChanged { id, rgb: [u8; 3] }` 并把草稿**规范化回写**
（`"FF8800"` / `"#FF8800"` → `"#ff8800"`）；失败 ⇒ **什么都不发**，色块退回
`border` 色（色块「消失」本身就是标记）。

什么时候**不**该用它：

- 要**弹窗取色器**（色板/吸管/HSL 面板）→ 本期不做（见「做不到什么」）；
- 要普通文本 → `Builder::field`；要数值 → [`NumberField`](number-field.md)。

## 2. 最小示例

```rust
use deer_gui::prelude::*;

let mut app = Builder::new(Kind::Column, "app").padding(8.0).gap(6.0);
let tint = app.color_field_opts("tint", "#ff8800", |_| {});
// tint = "tint"；label = 初始显示串（占位草稿）。色块颜色 = 对显示串实时解析。
let tree = app.build();
// 提交成功后 App 在 UiEvent::ColorChanged { id: "tint", rgb: [u8; 3] } 拿到三通道；
// 草稿已被规范化回写（state.texts["tint"] == "#aabbcc"）。
```

能跑的完整版（含提交/失败/失焦夹取/拖动/开关/颜色的自检断言）：

```sh
cargo run -p deer-gui --features testing --example m6_values
```

## 3. 完整 API

| 入口 | 说明 |
|---|---|
| `Builder::color_field(label) -> String` | 造一个（id 自动 `color_field_N`），返回节点 id |
| `Builder::color_field_opts(id, label, f)` | 指定 id 与就地改节点（`id` 传空串 = 自动） |
| `UiState::texts`（草稿） | 编辑期草稿在哪。表里没有 = 空串（色块退 `border`） |
| `UiEvent::ColorChanged { id, rgb }` | **提交成功**（失焦 / `Enter`）时发；`rgb` = 三通道。失败不发；规范化回写**不发** `TextChanged` |
| `deer_core::values::{parse_hex_color, format_hex_color}` | 解析/规范化的**唯一**实现（`#` 可省、大小写都行；规范化 = `#rrggbb` 小写带 `#`，幂等且能原样读回） |

### 语义（本仓颜色输入承诺的全部行为，逐条有测试）

| 交互 | 行为 | 钉住它的判据 |
|---|---|---|
| 打字 / 退格 / 光标 / IME | 与 `Field` **同一套**（草稿进 `texts`，逐字符发 `TextChanged`） | `color_field_commits_canonical_hex_and_rejects_the_rest` |
| `Enter` / 失焦提交 | 解析 `#RRGGBB` → 发 `ColorChanged` → 回写 `#rrggbb`；失败 ⇒ 静默 + 草稿保留 | 同上 |
| 解析规则 | 6 位 hex、大小写都行、`#` 可省、两侧空白 trim；`#RGB` 短式 / `#RRGGBBAA` / `rgb()` 函数式**不做**（一律 None） | `deer-core` 的 `values.rs` 单测 |
| 色块 | 节点矩形右侧的正方形（边长 = 高 − 4）；颜色 = 对**显示中的串**实时再解析（合法 = 该颜色；失败 = `border` 色，与壳同色 ⇒ 「消失」即标记）；文本矩形右侧收窄，与色块不重叠 | `invalid_draft_underline_and_color_swatch_follow_the_displayed_text` |
| `DefaultRenderer`（无状态） | 色块从 **label** 解析（树数据，无状态可画）；编辑态的差异（草稿/焦点/标记）是 `InteractiveRenderer` 的活 | 同上 ③ |
| 禁用 | 整棵子树静默（既有规则原样生效） | 既有 r4/r12 判据 |
| 值的寿命 | 草稿按 id 键控住树外 ⇒ 整树重建不丢且仍可用（Keep） | `keep_rebuild_preserves_value_tables` |

### 开放问题（deer-ui 特有行为不可考，登记不臆造）

- 完整取色器（色板 / 吸管 / HSL / 透明度）？全部不做 —— `rgb` 只有 3 通道，
  **alpha 不支持**；
- 命名颜色（`"red"` / CSS 颜色名）？不做（最小版不猜意图）；
- `#RGB` 短式是否应该自动展开？登记为开放问题（现版判非法）。

## 4. 自检（怎么确认你真的用对了）

```rust
// testkit 注入（tests/m6_values.rs 的判据形状）：
h.tap("tint")?;                                            // 点击聚焦
h.send(&InputEvent::TextInput { text: "AABBCC".into() })?; // 草稿（# 可省、大小写都行）
let step = h.send(&InputEvent::KeyDown { key: Key::Enter, .. })?;
assert!(step.events.contains(&UiEvent::ColorChanged { id: "tint".into(), rgb: [0xaa, 0xbb, 0xcc] }));
h.assert_text("tint", "#aabbcc")?;                          // 规范化回写
// 失败档：打 "#fff" → Enter ⇒ 无事件、草稿保留、色块退 border 色。
```

## 5. 常见坑

| 现象 | 原因 | 怎么改 |
|---|---|---|
| 色块「不见了」 | 显示串不可解析 ⇒ 刻意退回 `border` 色（与壳同色 = 标记） | 检查草稿是不是 6 位 hex（`#` 可省） |
| 打完字没有 `ColorChanged` | 还没提交 —— **只在失焦 / `Enter`** 时解析 | 失焦（Tab/点别处）或按 `Enter` |
| 提交了但事件里没有颜色 | `#fff` / `#11223344` / `rgb(...)` 都判非法（最小版不猜意图） | 展开成 `#RRGGBB` |
| 草稿提交后变了大小写/多了 `#` | 规范化回写（`#AABBCC` → `#aabbcc`，刻意行为，不发 `TextChanged`） | 以 `ColorChanged.rgb` 为准 |
| 想在打字过程中实时拿颜色 | 本控件只有**提交**语义；色块是「显示串的实时解析」，但事件不实时发 | 监听 `TextChanged` 自己解析（`parse_hex_color` 是公开的） |

## 6. 相关

- 相关功能：[`input`](input.md) §3.5（值类控件事件表；普通 `Field` 的草稿机械也在它的 §3.4）、[`number-field`](number-field.md)（同构的提交式输入）
- 内部原理：`crates/deer-core/src/values.rs`（唯一解析实现）；`crates/deer-gui/src/interaction.rs` 的 `commit_value_field` 与失焦钩子；`crates/deer-gpu/src/interact.rs` 的色块绘制
- 边界（**做不到什么**）：无取色器面板/色板/吸管；无 alpha（3 通道）；无 `#RGB` 短式 / `#RRGGBBAA` / `rgb()` 函数式 / 命名颜色；无多行；
  提交才发事件（打字过程不发 `ColorChanged`）；焦点在它上面时方向键上下**不**做焦点导航（与 `Field` 同边界）。

## 7. 检查清单（发布前过一遍）

- [x] 示例能跑：`cargo run -p deer-gui --features testing --example m6_values` → `exit=0`
- [x] 示例有自检断言（提交/规范化/失败静默/三路翻转/拖动，全部 assert）
- [x] `FEATURES.md` 已登记（状态 / 指南链接 / 示例命令都对）
- [x] `docs/TUTORIAL.md` 已有对应小节（§3 数值类控件）
- [x] 明确写了「做不到什么」
