# 功能指南：IME 预编辑（ime）

> 跑 `cargo run -p deer-gui --example ime_preedit` ·
> 状态在 `crates/deer-gui/src/interaction.rs`、绘制在 `crates/deer-gpu/src/interact.rs` ·
> 映射在 `crates/deer-window/src/lib.rs` · 出处：App 地基任务书 **T3.4**

## 1. 这是什么 / 什么时候用它

**中文/日文输入法「正在拼、还没上屏」的那一段**（例：你敲 `zhong` 时输入法候选框里那段）
怎么走完从窗口到画面的全程。

什么时候你会碰到它：

- 你的界面里有输入框，用户会**用中文输入法打字**；
- 你要区分「**已上屏**的内容」与「**还没定**的那一段」—— 两者的存储、颜色、语义都不同；
- 你要排查「中文打进去两遍」（那是双写）或「拼的时候看不到字」。

**一句话记法**：预编辑**属于状态**（`UiState::preedit`），**不属于内容**（`UiState::texts`）。

## 2. 最小示例

```rust
use deer_gui::interaction::{InputEvent, UiState};

// 输入法事件由窗口层映射而来（winit 的 `Ime::Preedit` → `InputEvent::ImePreedit`）
handle(&mut state, &tree, &geo, clip, &InputEvent::ImePreedit { text: "zhong".into() });
//   ⇒ state.preedit = Some(Preedit { id: "name", text: "zhong" })
//   ⇒ state.texts 里**没有**变化（还没上屏）

handle(&mut state, &tree, &geo, clip, &InputEvent::TextInput { text: "中".into() });
//   ⇒ state.texts["name"] 变成 "中"，且 **state.preedit 被清空**（无双写）

// 渲染：把 UiState 交给唯一的状态转换，预编辑就会画在光标处
let list = InteractiveRenderer::new(theme, &measure, &state.to_interact_state())
    .build(&tree, &geo);
```

实测输出（`--example ime_preedit`）：

```text
① 预编辑只进**缓冲**，不进 `texts`（还没上屏）
   preedit = Some(("name", "zhong"))
   texts["name"] = None（没被预编辑污染）
② 预编辑文字 x = 18，下划线宽 = 39      ← 画在光标处，下面一条下划线
   光标 x = [57]                        ← 光标被推到预编辑**之后**
③ 提交 ⇒ texts["name"] = Some("中")，preedit = None（必须清空）
   提交后那两条应当消失：已消失 ✅
```

## 3. 完整 API

```rust
// ① 事件（**两处定义必须逐字同步**：deer-gui / deer-window）
InputEvent::ImePreedit { text: String }   // 空串 = 取消
InputEvent::TextInput { text: String }    // 提交（Commit）走这条

// ② 状态
pub struct Preedit { pub id: String, pub text: String }   // 挂在哪个 Field 上
pub struct UiState { /* … */ pub preedit: Option<Preedit>, /* … */ }

// ③ 绘制侧（deer-gpu）
pub struct PreeditView { pub id: String, pub text: String }
pub struct InteractState { /* … */ pub preedit: Option<PreeditView>, pub carets: BTreeMap<String, usize> }
pub const PREEDIT_UNDERLINE_H: i32 = 1;
```

**分工是硬的**（这就是「无双写」）：

| 事件 | 去哪 | 画面 |
|---|---|---|
| `ImePreedit { text }`（非空） | 只进 `preedit` | 暗色文字 + 下划线，画在**光标处** |
| `ImePreedit { "" }`（取消） | 清 `preedit` | 那两条消失 |
| `TextInput { text }`（提交） | 进 `texts`，**并清 `preedit`** | 变成正常颜色的**内容** |

## 4. 自检（怎么确认你真的用对了）

`--example ime_preedit` 结尾有三条自检：预编辑不进 `texts` 且提交后清缓冲 /
预编辑文字 + 下划线被画出来 / **光标在预编辑之后**。

`deer-gui` 另有 3 条单测（缓冲与提交的对应关系、取消、无焦点时忽略），
`deer-gpu` 有 4 条绘制判据（不多画 / 恰好多两条且左端对齐 / 光标右移「预编辑宽度」/
**别人的预编辑不画到我身上**）。

## 5. 常见坑

- **双写**：预编辑进 `texts` 之后又提交一次 ⇒ 那段字上屏两遍。
  规则是「预编辑**永远不**进 `texts`」，提交时才进、且顺手清缓冲。别绕过 `handle` 自己塞。
- **忘了 `carets`**：预编辑画在**光标处** —— 没有光标位置就没有落点。
  两者一起带过去（`to_interact_state()` 一次带全）。
- **自己写状态转换**：`InteractState` 的字段会随版本增加（`scroll`/`carets`/`preedit` 都是这么加进来的）。
  手写漏一个字段的后果是「某个功能静默不生效」。**只走 `UiState::to_interact_state()`。**
- **以为 `range` 有用**：`winit` 的 `Ime::Preedit` 还带一个光标 range，**本期忽略**
  （用了 `_`）—— 预编辑总是画在当前光标之后。
- **在无窗口的测试里断言真输入法行为**：做不到。那需要中文输入法 + 真窗口，见第 6 节。

### 做不到什么

- **真机 IME 行为未自动化验证**：判据都停在「事件 → 状态 → 绘制命令」这一层。
  真中文输入法（拼音/五笔）的**实际**连接需要人手工跑一次真窗口（见下）；
- **不显示候选框**：输入法的候选窗由**系统/输入法自己**画，不经过本库；
- **不用 `Preedit` 的 range**：预编辑总是渲染在光标之后，不做「带选区/带内嵌下划线范围」的精细排版；
- **不做「预编辑期间的光标闪烁」**：光标是静态的（没有按时间闪烁）；
- **不处理输入法开关状态**：`Ime::Enabled`/`Disabled` 只用于复位「正在拼」标志，
  没有暴露成事件给上层。

**手工验证步骤**（需要 Windows + 中文输入法）：

```bash
cargo run -p deer-gui --features window --example interactive_form
# 点中输入框 → 切到中文输入法 → 敲 "zhong"
# 期望：能看到暗色的 "zhong" + 下划线，插入符停在它后面
#       选词上屏后 "zhong" 那两条消失，字变成正常颜色进入内容
```

## 6. 相关

- 输入与焦点的其余部分：[`input.md`](input.md)
- 光标（预编辑的落点）：本页第 3 节与 `carets`
- 文本渲染：[`text-rendering.md`](text-rendering.md)

## 7. 检查清单（发布前过一遍）

- [x] `--example ime_preedit` 真的跑过，`exit = 0`
- [x] 示例结尾有自检断言（三条），不是「跑成功就算」
- [x] 「无双写」有判据（预编辑不进 `texts`、提交清缓冲）
- [x] 绘制侧有判据（恰好多两条 / 光标在预编辑之后 / 别人的预编辑不画过来）
- [x] 本指南含「做不到什么」一节，且写明**真机 IME 未自动化验证**
- [x] 登记进 `FEATURES.md` 且指南链接 + 示例命令都对
