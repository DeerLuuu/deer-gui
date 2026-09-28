# 功能指南：输入与焦点（input）

> 状态 ✅（M5-1..M5-4 已落地；**局部**：见第 6 节的「仍未做」）·
> 示例 `cargo run -p deer-gui --features window --example interactive_form` ·
> 清单条目见 [`FEATURES.md`](../../FEATURES.md)

## 1. 这是什么 / 什么时候用它

把「**输入事件 → 命中测试 → 状态变更 → 重绘**」这条闭环接起来：窗口里能**悬停 / 按下 / 点击 / Tab 聚焦 /
输入文字**，而且整条链**可确定性回归**（脚本化事件重放 + 状态终态断言 + 像素判据）。

分工（三层，各只有一份实现）：

| 层 | 在哪 | 职责 |
|---|---|---|
| 事件模型 + winit 映射（M5-1） | `deer-window`（开 `window` feature 时） | `CursorMoved` / `MouseInput` / `MouseWheel` / `KeyboardInput` / `Ime` / `Focused` → `InputEvent`，交给 `App::input` |
| 命中与状态机（M5-2/3，**纯逻辑**） | `deer_gui::interaction` | `hit()` / `handle()` / `ClipSnapshot` / `UiState` / `UiEvent` |
| 闭环与脚本重放（M5-4） | `deer-gui` 的窗口集成 + `interactive_form` 示例 | dirty 账本、脚本解析、像素证明 |

**什么时候用它**：你要让窗口里的界面**真的能点、能 Tab、能打字**；或者你要给交互行为写**可确定性回归**的判据。

**什么时候不该用它**：
- 你只想要一张图 / 一组断言 —— 用离屏出图（[`rendering.md`](rendering.md)），别开窗口；
- 你想要**滚动、方向键导航、右键/中键语义、IME 预编辑、光标位置** —— 这些**还没做**（第 6 节）；
- 你想把命中测试当**通用碰撞检测** —— 它是**输入路由**，语义见第 3.2 节（禁用子树不回退、半开区间）。

## 2. 最小示例

```rust
use deer_gui::interaction::{self, ClipSnapshot, InputEvent, Key, UiState};
use deer_gui::prelude::*;

// ① 树 + 几何（几何来自布局；命中测试只认它）
let mut app = Builder::new(Kind::Column, "app").padding(12.0).gap(8.0);
app.button("OK");
app.field("name");
let root = app.build();
let geo = deer_gui::layout_tree(&root, 320, 200, Theme::default().clone());

// ② 裁剪快照：必须**从本帧真实绘制列表**派生（见 3.3 节；空快照 = 全不裁剪）
let list = deer_gui::gpu::build_draw_list(&root, &geo, Theme::default().clone(), &ApproxMeasure);
let clip = ClipSnapshot::from_draw_list(&list, &root, &geo);
assert!(!clip.is_empty(), "空快照会让裁剪护栏静默失效，先断言它非空");

// ③ 状态机：输入是值、输出是值（不碰窗口、不碰 GPU）
let mut state = UiState::default();
let events = [
    InputEvent::PointerMoved { x: 40.0, y: 30.0 },
    InputEvent::PointerDown { button: deer_gui::interaction::PointerButton::Left, x: 40.0, y: 30.0 },
    InputEvent::PointerUp { button: deer_gui::interaction::PointerButton::Left, x: 40.0, y: 30.0 },
    InputEvent::KeyDown { key: Key::Tab, mods: Default::default() },
    InputEvent::TextInput { text: "hi".to_string() },
];
for ev in &events {
    let _events = interaction::handle(&mut state, &root, &geo, clip, ev);
}
// ④ 判据：**状态终态** + （窗口侧）**像素**，不是「跑完没报错」
println!("hover={:?} focus={:?} texts={:?}", state.hover, state.focus, state.texts);
```

跑**真窗口**示例（有内置脚本 ⇒ 退出码 0 即断言全过）：

```powershell
# 自己点、按 Tab/Esc（Esc 退出）
cargo run -q -p deer-gui --features window --example interactive_form

# 脚本化重放（确定性；读完脚本自己退出）
$env:DEER_INPUT_SCRIPT='move:60,60;down:left;up:left;key:Tab;text:hi'
cargo run -q -p deer-gui --features window --example interactive_form
```

> ⚠️ **窗口侧示例要真窗口**：`interactive_form` 的窗口路径需要 `--features window`；真窗口 e2e 在
> `cargo test` 里跑不了（winit 要求主线程），所以**纯逻辑那半在 `tests/interaction.rs` / `tests/interactive_form.rs`**，
> 窗口那半靠示例 + 脚本重放。`DEER_INPUT_HOLD=1` 可留窗做**人肉**验证（`Focused` / `Ime::Commit` 只能这样验）。

## 3. 完整 API

### 3.1 事件模型（M5-1 冻结定义）

`InputEvent`（**开 `window` 时直接用 `deer-window` 那一份；不开时是逐字相同的镜像**，见 `interaction` 模块注释）：

| 变体 | 口径 |
|---|---|
| `PointerMoved { x, y }` | **物理像素**、窗口左上角原点（与 `WindowInfo::extent` 同一套） |
| `PointerDown/Up { button, x, y }` | `button` ∈ `Left` / `Right` / `Middle`；只有**左键**参与点击与按下态 |
| `Wheel { dx, dy }` | 滚轮；**本期状态机不消费**（见 3.4） |
| `KeyDown/KeyUp { key, mods }` | `key: Key`（`Tab` / `Escape` / `Enter` / `Backspace` / `Left` / `Right` / `Up` / `Down` / `Char(char)` / `Other`），`mods: Mods`（`shift` / `ctrl` / `alt` / `sup`） |
| `TextInput { text }` | **字符输入**：一段文本（**追加**，不是覆盖） |
| `FocusChanged { focused }` | **窗口焦点**（不是控件焦点）；`focused: false` 清悬停/按下态 |

> **字符输入与物理键是分开的两个变体**：`TextInput` 只承载「打出来的字」（含 IME 提交与中文），
> `KeyDown { key: Char(c) }` 承载「物理键」这条通路 —— 两者**语义不同**，状态机对前者的处理是「写进文本缓冲」，
> 对后者（`Char`/`Other`）**不消费**。别把键盘输入塞进 `KeyDown` 就以为能打字。

### 3.2 命中测试：`hit()`

```rust
pub fn hit<'a>(root: &'a Node, geo: &Geometry, clip: ClipSnapshot, x: f32, y: f32) -> Option<&'a Node>
```

- **唯一路由依据是 `deer_layout::hit_test`**（**最深命中者胜出**）—— 本层**不另写遍历**，
  只补两件它不做的事：**查裁剪**、**查禁用**（否则「谁是输入路由的唯一依据」会有两份，必然漂）。
- **禁用**：命中节点自身**或任一祖先** `props.disabled` ⇒ **整个点不命中**（**禁用子树不响应**）。
- **有意的语义选择：不回退到祖先**。理由：回退需要「第二套路由规则」（谁是次优候选），与上一条冲突。
  **代价要写明**：点在被裁剪 / 被禁用区域上的手势会**完全消失**（既不命中子节点，也不会落到父容器），
  而不是「降级成父容器的一次点击」。
- **半开区间**：`x ∈ [x, x+w)`、`y ∈ [y, y+h)`（与 `hit_test` 逐字一致）。
- **裁剪快照里未知的 id 放行**（fail-open，见 3.3）—— 所以测试里**必须先断言 `is_known()`**。

### 3.3 裁剪快照：`ClipSnapshot`

```rust
pub fn from_draw_list(list: &DrawList, root: &Node, geo: &Geometry) -> ClipSnapshot
pub fn allows(&self, id: &str, x: f32, y: f32) -> bool      // 未知 id ⇒ 放行（fail-open）
pub fn is_known(&self, id: &str) -> bool                     // 护栏用：先断言它
pub fn clip_of(&self, id: &str) -> Option<RectI>             // None = 不裁剪**或**未知
pub fn with_node_clip(self, id: impl Into<String>, clip: Option<RectI>) -> ClipSnapshot
pub fn unclipped() -> ClipSnapshot
```

**为什么不能只从 `DrawList` 派生**：`DrawCmd::NodeHint { rect, node_id_len }` **只有 id 长度、没有 id**，
全仓库也没有 id 侧表。所以 `from_draw_list` **与树 + 几何共走**：把第 k 个 `NodeHint` 绑到第 k 个「有几何的节点」上，
并用 **`node_id_len` 当校验和**（不一致就断言失败，而不是悄悄错位）。

**盲区（已知，见 6.2）**：校验和**只比长度**，且 `NodeHint.rect` 不参与校验 ⇒
**等长的 id 互换不会 panic，还会静默给出错快照**。

**谁提供 `NodeHint`**：`InteractiveRenderer` 给每个有几何的节点发；`DefaultRenderer` **不发** ⇒
那份快照会是**空的**（= 全不裁剪），而且**看不出来**。所以窗口侧 `init` 里要**先断言快照非空**。

### 3.4 状态机：`handle()` / `UiState` / `UiEvent`

```rust
pub fn handle(state: &mut UiState, root: &Node, geo: &Geometry, clip: ClipSnapshot, ev: &InputEvent) -> Vec<UiEvent>
pub fn focusables(root: &Node) -> Vec<String>   // Kind::Button | Kind::Field，且跳过禁用子树
```

| 输入 | 效果 |
|---|---|
| `PointerMoved` | 更新 `hover`（走 `hit`）；变化时发 `HoverChanged` |
| `PointerDown { Left }` | 同步 `hover`、记 `pressed`；**命中可聚焦控件（`Button`/`Field`）⇒ 聚焦它** |
| `PointerUp { Left }` | 抬起落在同一节点 ⇒ `Clicked(id)`；按下后移出再抬起 ⇒ **不产生** `Clicked` |
| `KeyDown { Tab }` / `Shift+Tab` | 在 `focusables()` 里**按树序**循环焦点 ⇒ `FocusChanged` |
| `KeyDown { Escape }` | 清焦点 ⇒ `FocusChanged(None)` |
| `KeyDown { Backspace }` | 焦点是启用的输入框 ⇒ 删**一个 Unicode 字符**（不是字节）⇒ `TextChanged` |
| `TextInput { text }` | 焦点是启用的输入框 ⇒ **追加**到 `texts[id]` ⇒ `TextChanged` |
| 其余（`Wheel`、`KeyUp`、右/中键、方向键、`Key::Char`/`Other`、`focused: true`） | **不消费**（有测试钉住：不消费的事件**不得改变任何状态**） |

**`UiState` 是唯一真相**：`hover` / `focus` / `pressed` / `texts: BTreeMap<String, String>`（不在表里的输入框视为空串）。
**`UiEvent`** 是「发生了什么」：`HoverChanged` / `FocusChanged` / `Clicked` / `TextChanged`。

**dirty 判据必须比对状态，不能看「有没有事件」**：

```rust
pub fn same_visual(&self, other: &UiState) -> bool   // 只比 hover/focus/pressed（texts 不参与）
```

`PointerDown` 就属于「**会改状态但可能不发 `UiEvent`**」的那一类（点空白处时 `pressed` 由 `Some` 变 `None`、
事件列表却是空的）⇒ 只看事件会漏掉它，而且漏得很安静（界面看起来只是「不响应按下」）。

### 3.5 脚本化重放：`DEER_INPUT_SCRIPT`

解析在 `deer_gui::input_script::parse_script`（**纯逻辑**，与窗口示例共用**同一份**解析器，语法只有一处定义）：

| 语法 | 含义 |
|---|---|
| `move:X,Y` | 指针移到 `(X, Y)`（f32；负数/贴边都允许） |
| `down:left` / `up:left`（也支持 `right` / `middle`） | 指针按下 / 抬起；坐标 = **最近一次 `move`**（还没 move 过就是 `(0,0)`） |
| `key:Tab` | `KeyDown`。可写 `Escape` / `Enter` / `Backspace` / `Left` / `Right` / `Up` / `Down` / `Other` / `Char(a)` |
| `shift+key:Tab`（或 `key:shift+Tab`） | 带修饰键；`ctrl+` / `alt+` / `sup+` 同档、可叠加；前缀写在**动词一侧或键名一侧都行** |
| `keyup:Char(a)` | `KeyUp` |
| `text:hi 你好` | 一段文本输入（原样，含空格与中文；**追加**） |
| `focus:on` / `focus:off` | 窗口焦点变化 |
| `wheel:0,3` | 滚轮（状态机不消费；可用来验证「不消费的事件不改状态」） |

- 语句用 **`;` 或换行**分隔；`#` 到行尾是注释；**空语句被忽略**。
- **语法错一律 `Err(String)`**，带**语句序号**与原文；**刻意不「跳过看不懂的语句」**（脚本是判据的一部分）。
- **`move @节点id`（按 id 定位）是\*\*示例层\*\*的模板展开，不是库语法**：`parse_script` 只认 `move:X,Y`；
  `interactive_form` 会先把 `move @button_1` 展开成坐标（`button_1` 的中心由布局算），再交给解析器。
  好处是「布局一变脚本跟着变」，而不是静默点空。

真实错例（照抄可复现）：

```text
# 注释行
bad:3
```

⇒ `Err("第 2 条语句 `bad:3` …没有 `:`（语法是 `动词:参数`，例如 `key:Tab`）")`
—— 注意它报「**第 2 条**」（序号把**空段与注释段也算进去**了，见 6.3）。

### 3.6 怎么做确定性回归

1. **纯逻辑那半（首选）**：`cargo test -p deer-gui --test interaction` 与 `--test interactive_form`
   —— 脚本解析、命中规则、状态机、dirty 账本都能在没有窗口、没有 GPU 的环境里跑。
2. **窗口那半**：用**两条**脚本跑同一个示例 —— ① **有动作**的脚本（内置脚本或自定义）；
   ② **空转**脚本（例如只剩 `wheel:0,3` 这类不消费的事件）。判据看**重绘账本**：
   示例每帧打印一行 `true`（真的画了）/ `false`（状态没变、跳过）；空转脚本应当**几乎全是 `false`**，
   有动作的脚本应当在动作那几步出现 `true`。`move @id` 之类可让脚本不写死坐标、跟着布局走。
3. **像素判据**：状态变了 ⇒ 画出来的那一帧必须与「该状态下的 CPU 基准」逐像素一致（不透明 0 / 半透明 ≤1 LSB）；
   **按状态分别对照**，不要只对照最终一帧。

## 4. 自检（怎么确认你真的用对了）

```rust
// ① 先断言快照真的覆盖了你要测的节点（否则「被裁掉不命中」可能只是「快照里没这个 id」）
assert!(clip.is_known("field_1"), "快照漏了这个节点 —— 护栏会静默失效");
assert!(!clip.is_empty(), "空快照 = 全不裁剪，且看不出来");

// ② 断言**状态终态**，而不是「跑完没报错」
assert_eq!(state.hover.as_deref(), Some("button_1"));
assert_eq!(state.focus.as_deref(), Some("field_1"));
assert_eq!(state.texts.get("field_1").map(String::as_str), Some("hi"));

// ③ dirty 判据要咬住「状态没变就不重绘」
let before = state.clone();
let _ = interaction::handle(&mut state, &root, &geo, clip, &ev);
assert!(state.same_visual(&before), "这一步不该改变视觉状态");

// ④ 脚本解析必须**显式失败**：语法错 ⇒ Err（不许静默跳过）
assert!(parse_script("move @button_1").is_err(), "库语法只认 move:X,Y（@id 是示例层展开）");
assert!(parse_script("# 注释行\nbad:3").unwrap_err().contains("第 2 条"));
```

## 5. 常见坑

| 现象 | 原因 | 怎么改 |
|---|---|---|
| 点得中，但被裁掉的地方也点得中 | 用了 `ClipSnapshot::unclipped()`，或快照是空的（`DefaultRenderer` **不发** `NodeHint`） | 用 `InteractiveRenderer` 出列表，并**先断言 `!clip.is_empty()`** |
| 「裁剪测试」绿了但其实没测到 | `allows()` 对**未知 id 放行**（fail-open） | 先 `assert!(clip.is_known(id))`，再断言「被裁掉不命中」 |
| 点空白处界面不响应按下 | dirty 判据写成「有没有 `UiEvent`」—— `PointerDown` 可能**不发事件**却改了 `pressed` | 用 `UiState::same_visual()` 比对**状态** |
| 打字打不进去 | 用 `KeyDown { key: Char(..) }` 当输入；状态机**不消费**它 | 用 `TextInput { text }`（字符输入是**独立变体**） |
| `Backspace` 删多了/删乱了（中文） | 按**字节**删 | 实现是按 **Unicode 字符**删；自己写时也别按字节切 |
| Tab 顺序每次不一样 | 用了容器/文本 | 可聚焦集合是 **`Button` / `Field`**（`focusables()`），且跳过禁用子树；顺序 = **树序** |
| 点禁用按钮的父容器也没反应 | 禁用判定看**祖先链**：子树内的一切都不命中，且**不回退祖先** | 这是**有意语义**（第 3.2 节）；要能点就别把可点控件放进禁用容器 |
| 脚本明明写错却「跑过去了」 | 用了 `@id` 之类**示例层**语法去喂 `parse_script`，或自己吞掉了 `Err` | 别吞 `Err`：语法错必须让用例红 |
| 以为「不脏就不画」= 事件驱动重绘 | 窗口仍是 `ControlFlow::Poll` **连续重绘**，只是每帧先查脏位 | 现在只保证「不脏就不真的画」；真正的按需重绘见第 6 节「仍未做」 |

## 6. 做不到 / 已知问题

### 6.1 已知问题（**覆盖缺口，不是功能缺失**）

| 问题 | 现象 | 为什么现有判据抓不住 |
|---|---|---|
| **按钮的 focus 视觉几乎不可见** | 焦点态与空闲态**实测只差 56 px / 792 px**（其中 32 px 是「圆角补方角」，其余是描边压过字形） | 填充是 `tint(accent, Idle) == accent`、焦点描边也是 `theme.accent` ⇒ 视觉差极小；而测试**只要求差异 > 0** ⇒ 抓不住「焦点环形同虚设」。**输入框的焦点是可见的** |
| **`node_id_len` 校验和盲区** | 只比长度、`NodeHint.rect` 不参与校验 ⇒ **等长 id 互换不会 panic**，还给出**错快照** | 例：`button_1` 的 clip 本应 `Some(0,0,60,40)`，错位后变成 `None`（= 不裁剪）⇒ 裁剪护栏静默失效 |
| **脚本语句序号含空段 / 注释段** | `# 注释行\nbad:3` 报「**第 2 条**」 | 序号按 `split(';' / '\n')` 的**段**计数，空段与注释段也占号 ⇒ 报错行号与人的直觉差一位（错误本身仍然可靠） |

### 6.2 仍未做（**不要以为能跑**）

- **事件驱动重绘**：窗口仍是 `ControlFlow::Poll` **连续重绘** ⇒ 现在是「**不脏就不画**」，**不是**「按需重绘」；
- **停靠面板 / 多窗口**；**滚动**与**方向键导航**；**右键 / 中键**的语义；**IME 预编辑**；
- **按键重复未建模**（长按会产生重复 `KeyDown`，状态机不区分「重复」与「新按」）；
- **`texts` 没有光标位置**：`Backspace` **只删末尾**，不能用方向键移动插入点；
- **`Focused` / `Ime::Commit` 只能人肉验证**（`DEER_INPUT_HOLD=1` 留窗观察），没有自动判据。

## 7. 相关

- 命中测试的几何依据：[`hit-testing.md`](hit-testing.md)、[`layout.md`](layout.md)
- 绘制列表（`NodeHint` / 裁剪栈的来源）：[`draw-list.md`](draw-list.md)
- CPU 基准后端（像素对照）：[`rendering.md`](rendering.md)、[`pixels.md`](pixels.md)
- 窗口与上屏：[`window.md`](window.md)、[`vulkan-swapchain.md`](vulkan-swapchain.md)
- 节点与属性（`disabled` / `id`）：[`node-tree.md`](node-tree.md)、[`imperative-api.md`](imperative-api.md)

## 8. 检查清单（发布前过一遍）

- [x] 示例能跑：`cargo run -p deer-gui --features window --example interactive_form` → `exit=0`（内置脚本有期望终态）
- [x] 纯逻辑回归能跑：`cargo test -p deer-gui --test interaction` / `--test interactive_form`
- [x] 脚本语法与实现一致（`move` / `down` / `up` / `key` / `keyup` / `text` / `focus` / `wheel`；`@id` 标明是示例层展开）
- [x] `FEATURES.md` 已登记（状态 / 指南链接 / 示例命令）
- [x] 明确写了「做不到什么」，并把**已知问题**单列（含「为什么现有判据抓不住」）
