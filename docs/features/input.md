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
- 你想要**方向键上下导航、右/中键语义、按键重复、按键滚动** —— 这些**都已经做了**（见第 6 节「已落地」）。
  **已经可以做到的**（别再当成没做）：**输入框光标**（T3.5：在光标处插入 / `Backspace` 删**光标前一个 Unicode 字符** /
  **左右方向键**移动光标，单位是**字符位**；T3.8 起画面上**真的画出了那根竖线**）、**IME 预编辑**
  （画在光标处 + 下划线、光标推到它之后，见 [`ime.md`](ime.md)）、**可视滚动条 + 拖滑块改偏移**
  （见 [`scrollbar.md`](scrollbar.md)）、滚轮驱动的**垂直滚动**
  （见 [`scroll-and-multiline.md`](scroll-and-multiline.md)）。
  惯性的窗口侧用法（两行）：`redraw()` 开头 `advance_inertia(&mut state)`、
  `next_deadline()` 返回 `inertia_deadline(&state)`（见 [`scrollbar.md`](scrollbar.md)）。
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
| `Wheel { dx, dy }` | 滚轮；**驱动滚动偏移**（见 3.4）：目标 = `hover` 命中节点最近的可滚动祖先（含自身），一格 = `WHEEL_STEP_PX`(40 px)，`dy < 0` ⇒ 偏移增大。`dx`（水平）本期忽略 |
| `KeyDown/KeyUp { key, mods }` | `key: Key`（`Tab` / `Escape` / `Enter` / `Backspace` / `Left` / `Right` / `Up` / `Down` / `Char(char)` / `Other`），`mods: Mods`（`shift` / `ctrl` / `alt` / `sup`） |
| `TextInput { text }` | **字符输入**：一段文本（**追加**，不是覆盖） |
| `FocusChanged { focused }` | **窗口焦点**（不是控件焦点）；`focused: false` 清悬停/按下态 |
| `ScaleFactorChanged { scale_factor }` | **DPI 缩放系数变了**（AF-3，只透传）：OS 报多少给多少；**坐标/尺寸不换算**（仍是物理像素），交互层不消费、`UiState` 不动 |

> **字符输入与物理键是分开的两个变体**：`TextInput` 只承载「打出来的字」（含 IME 提交与中文），
> `KeyDown { key: Char(c) }` 承载「物理键」这条通路 —— 两者**语义不同**，状态机对前者的处理是「写进文本缓冲」，
> 对后者（`Char`/`Other`）**不消费**。别把键盘输入塞进 `KeyDown` 就以为能打字。

### 3.2 命中测试：`hit()`

```rust
pub fn hit<'a>(root: &'a Node, geo: &Geometry, clip: ClipSnapshot, x: f32, y: f32) -> Option<&'a Node>
```

- **唯一路由依据是 `deer_core::hit_test`**（**最深命中者胜出**）—— 本层**不另写遍历**，
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

**为什么不能只从 `DrawList` 派生**：`DrawCmd::NodeHint { rect, node_id_len, node_id_fp }` **不含 id 本身**
（只有它的长度与指纹），全仓库也没有 id 侧表。所以 `from_draw_list` **与树 + 几何共走**：把第 k 个 `NodeHint`
绑到第 k 个「有几何的节点」上，并用 **`node_id_len` 和 `node_id_fp` 双重校验**（不一致就断言失败，而不是悄悄错位）。
两个校验和由 `DrawCmd::node_hint(rect, id)` 这**唯一**构造点产出（产出侧不要手写字面量）。

**为什么长度不够（曾经的盲区，已修）**：只比长度时，**等长 id 互换**（`button_1` / `button_2` 都是 8 字节）
会让长度序列逐项相同 ⇒ 不 panic、静默给出错快照。`node_id_fp` 是 id 的 **FNV-1a 64 确定性指纹**
（无随机种子、无分配；标准测试向量 `""`/`"foobar"` 被单测钉住），它让 id **真的参与**校验。
`NodeHint.rect` 仍然**不**参与校验（它只是诊断用的矩形）。

**谁提供 `NodeHint`**：`InteractiveRenderer` 给每个有几何的节点发；`DefaultRenderer` **不发** ⇒
那份快照会是**空的**（= 全不裁剪），而且**看不出来**。所以窗口侧 `init` 里要**先断言快照非空**。

### 3.4 状态机：`handle()` / `UiState` / `UiEvent`

```rust
pub fn handle(state: &mut UiState, root: &Node, geo: &Geometry, clip: ClipSnapshot, ev: &InputEvent) -> Vec<UiEvent>
pub fn focusables(root: &Node) -> Vec<String>   // Button / Field / NumberField / ColorField / Switch，且跳过禁用子树
```

| 输入 | 效果 |
|---|---|
| `PointerMoved` | 更新 `hover`（走 `hit`）；变化时发 `HoverChanged`。**捕获中**（`pressed` 在手，T3.7）⇒ **路由给捕获者**：hover 钉在捕获节点上，拖出节点/出树不换人。**拖动调值锚点在手且捕获者仍是它** ⇒ 反解成值，**变了才发** `NumberChanged`（M6 5d） |
| `PointerDown { Left }` | 同步 `hover`、记 `pressed`（**按下即捕获**，D7 默认捕获）；**命中可聚焦控件（`Button`/`Field`/`NumberField`/`ColorField`/`Switch`）⇒ 聚焦它**；**落在 `ScrubNum` 上 ⇒ 建立拖动锚点**（label 解析失败 = 不进入） |
| `PointerUp { Left }` | **按捕获者结算** `Clicked(id)`（抬起在哪都算 —— 拖出去再抬起也是捕获者的点击）；释放捕获、清拖动锚点，hover 回到抬起处的物理节点；**落点是选择类组（`Segmented`/`ChipGroup`/`TabBar`）的直接子节点 ⇒ 追加选择结算**（`SelectionChanged`/`ChipToggled`/`TabChanged`，见 §3.5）；**落点本身是 `Switch` ⇒ 追加开关结算**（`Toggled`，翻转语义每次都发） |
| `KeyDown { Tab }` / `Shift+Tab` | 在 `focusables()` 里**按树序**循环焦点 ⇒ `FocusChanged`；焦点从值输入框离开 ⇒ **失焦提交**（见下） |
| `KeyDown { Escape }` | 清焦点 ⇒ `FocusChanged(None)`；焦点从值输入框离开 ⇒ 失焦提交 |
| `KeyDown { Enter }` | 焦点在启用的**按钮**上 ⇒ `Clicked` + 选择结算；焦点在启用的**开关**上 ⇒ `Clicked` + 翻转；焦点在**值输入框**（`NumberField`/`ColorField`）上 ⇒ **提交**（解析 → 值事件 → 规范化回写）；普通 `Field` 不受 `Enter` 影响 |
| `KeyDown { Char(' ') }`（= winit 的 Space） | 焦点在启用的**开关**上 ⇒ 与 `Enter` 同一条翻转路径；焦点在**值输入框**上不消费（空格是草稿正文）；其余不消费 |
| `KeyDown { Backspace }` | 焦点是启用的**值输入框**（`Field`/`NumberField`/`ColorField`）⇒ 删**一个 Unicode 字符**（不是字节）⇒ `TextChanged` |
| `TextInput { text }` | 焦点是启用的**值输入框** ⇒ 插入**光标处** ⇒ `TextChanged` |
| 失焦提交（所有事件共用，`handle` 末尾） | 处理**前**焦点在值输入框、处理**后**焦点换了人（Tab/Escape/点别处）⇒ 替它提交一次（解析 → 发值事件 → 规范化回写）。窗口失焦**不动 UI 焦点** ⇒ 不触发 |
| `Wheel { dy }` | **滚动**：`hover` 命中节点**最近的可滚动祖先（含自身）**偏移 `−dy × WHEEL_STEP_PX`，夹进 `[0, max_scroll]`；**变了才发** `Scrolled { id, offset }`。禁用子树不响应；没有可滚动祖先 / 上限为 0 / `dy = 0` ⇒ 空转 |
| 其余（`KeyUp`、右/中键、非空格 `Key::Char`、`Key::Other`、`focused: true`） | **不消费**（有测试钉住：不消费的事件**不得改变任何状态**） |

**`UiState` 是唯一真相**：`hover` / `focus` / `pressed`（= **左键捕获者**，T3.7 起，见上表）
/ `texts: BTreeMap<String, String>`（不在表里的输入框视为空串；`Field`/`NumberField`/`ColorField` 共用）
/ `scroll: ScrollState`（**滚动偏移 + 每个容器的 `max_scroll`**）
/ `segments`（M6 5c：分段组 id → 选中段 id）/ `chips`（芯片 id → 开/关）/ `tabs`（页签组 id → 活动页 id）
/ `num_opts`（M6 5d：控件 id → 值域/步长 `NumOpts`，表里没有 = 无值域、步长 1.0）
/ `switches`（M6 5d：开关自身 id → 开/关，表里没有 = 关）/ `scrub`（M6 5d：拖动调值的**瞬态**锚点，抬起/失焦即清）——
选择类与数值类的**值**是应用数据（texts 同一条纪律：按 id 键控、住树外、重建不丢）。
**`UiEvent`** 是「发生了什么」：`HoverChanged` / `FocusChanged` / `Clicked` / `TextChanged` / `Scrolled { id, offset }`
/ `SelectionChanged { id, selected }` / `ChipToggled { id, chip, on }` / `TabChanged { id, index }`（M6 5c，见 §3.5）
/ `NumberChanged { id, value }` / `Toggled { id, on }` / `ColorChanged { id, rgb }`（M6 5d，见 §3.5）。

> **滚动偏移为什么住在 `UiState` 里**：滚轮必须在**唯一入口**（`handle`）被消费。若另开一个
> 「带滚动的 `handle`」，那条路径上的滚轮会静默无效而没人看得出来。偏移是**布局的输入**
> （每帧喂给 `layout_with_scroll`），`max_scroll` 是**布局的输出**（每帧 `state.scroll.set_metrics(...)` 灌回来）
> —— **灌漏了滚轮就完全无效**（上限表为空 ⇒ 一律按 0 夹取，fail-closed）。
> 完整用法与常见坑见 [`scroll-and-multiline.md`](scroll-and-multiline.md)。

**dirty 判据必须比对状态，不能看「有没有事件」**：

```rust
pub fn same_visual(&self, other: &UiState) -> bool   // 只比 hover/focus/pressed（texts 不参与）
```

`PointerDown` 就属于「**会改状态但可能不发 `UiEvent`**」的那一类（点空白处时 `pressed` 由 `Some` 变 `None`、
事件列表却是空的）⇒ 只看事件会漏掉它，而且漏得很安静（界面看起来只是「不响应按下」）。

### 3.5 值类控件的值与事件（M6 5c 选择类 / 5d 数值类）

`Segmented`（互斥单选）/ `ChipGroup`（多选开关）/ `TabBar`（页签）三种组与
`NumberField` / `ScrubNum` / `Switch` / `ColorField` 四种数值类控件的**值是应用数据**，
住在 `UiState` 的公开字段里（texts 同一条纪律），控件只负责「交互 → 更新值 → 报告」。
结算规则（与 `interaction.rs` 的 `resolve_selection` / `resolve_switch` /
`commit_value_field` 逐字同步）：

**选择类（5c，r14）**：

| 组 | 点击直接子节点 | 事件 | 值的落点 |
|---|---|---|---|
| `Segmented` | 点**新**段 ⇒ 选中它；点已选段 = 无变化 ⇒ 只发 `Clicked` | `SelectionChanged { id: 组id, selected: 段id }` | `segments[组id] = 段id` |
| `ChipGroup` | 每次点击**必翻转必发**（翻转本身就是动作） | `ChipToggled { id: 组id, chip: 芯片id, on: 翻转后的新值 }` | `chips[芯片id] = on`（表里没有 = 关） |
| `TabBar` | 点**新**页 ⇒ 换活动页；点当前页只发 `Clicked` | `TabChanged { id: 组id, index: 页下标 }`（**禁用页也计数**；内容切换是 App 的事） | `tabs[组id] = 页id` |

- `Enter` 激活（焦点在启用的段/芯片/页签上）与指针点击走**同一条**结算路径；
- 禁用/被裁剪的子节点点不到（`hit` 拒绝）⇒ 天然静默，也不在焦点序里；
- 只认**直接子节点**：命中落在组内嵌套容器的更深层 ⇒ 不属于任何组，不上溯。

**数值类（5d，r16 = 开关结算 + 提交）**：

| 控件 | 交互 | 事件 | 值的落点 |
|---|---|---|---|
| `NumberField` | 编辑进 `texts`（与 `Field` 同一套机械）；**提交**（失焦 / `Enter`）才解析 | `NumberChanged { id, value: f64 }`（**夹取后**；失败不发） | 草稿在 `texts[id]`（成功后规范化回写）；值域/步长在 `num_opts[id]` |
| `ScrubNum` | 按下从 label 解析基准 + 建锚点；拖动 = 基准 + (x−按下x)·step，**变了才发** | `NumberChanged { id, value: f64 }` | 值的真相在 **App**（label 是显示串）；锚点是 `scrub`（瞬态，抬起/失焦即清） |
| `Switch` | 点击 / `Enter` / `Space`（= winit `Named(Space) ⇒ Char(' ')`）三路**同一条**翻转结算（每次激活必发） | `Toggled { id, on: 翻转后的新值 }` | `switches[开关自身id]`（表里没有 = 关） |
| `ColorField` | 编辑进 `texts`；提交按 `#RRGGBB` 解析（`#` 可省、大小写都行） | `ColorChanged { id, rgb: [u8; 3] }`（失败不发） | 草稿在 `texts[id]`（成功后回写 `#rrggbb`） |

- 「提交」的全部来源（`Enter`、Tab、Escape、点到别处）都经过焦点变化 ⇒ `handle`
  末尾的一个失焦钩子覆盖全部路径；**窗口失焦不动 UI 焦点 ⇒ 不触发**；
- 提交成功后的**规范化回写不发 `TextChanged`**（值已由值事件报告，回写不是用户新输入）；
- 解析/格式化的唯一实现在 `deer_core::values`（交互层发不发事件、绘制层画不画标记
  用**同一份**）；指南见 [`number-field`](number-field.md) / [`scrub-num`](scrub-num.md) /
  [`switch`](switch.md) / [`color-field`](color-field.md)。

### 3.6 脚本化重放：`DEER_INPUT_SCRIPT`

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
| `wheel:0,3` | 滚轮（**会被消费**：滚 `hover` 所在的可滚动容器；语料里没有可滚动容器时它就是空转 —— 可用来验证「无事可做的事件不改状态」） |

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

### 3.7 怎么做确定性回归

1. **纯逻辑那半（首选）**：`cargo test -p deer-gui --test interaction` 与 `--test interactive_form`
   —— 脚本解析、命中规则、状态机、dirty 账本都能在没有窗口、没有 GPU 的环境里跑。
2. **窗口那半**：用**两条**脚本跑同一个示例 —— ① **有动作**的脚本（内置脚本或自定义）；
   ② **空转**脚本（例如只剩 `wheel:0,3` 这类不消费的事件）。判据看**重绘账本**（M5b 起由窗口层打印）：
   `[deer-window] 重绘账本：requests=… skipped=… frames=…` —— 其中 **`skipped` = 「派发了输入但没请求重绘」的次数**
   （即 `App::wants_redraw()` 为假、省下的那一帧）。空转脚本下 `skipped` 应当随输入条数增长而 `frames` 不涨；
   有动作的脚本应当在动作那几步让 `frames` 增长。`move @id` 之类可让脚本不写死坐标、跟着布局走。
   > ⚠️ **`skipped` 与重绘策略正交**：它衡量的是「**输入有没有改变状态**」（输入门禁），
   > 所以在 `DEER_WINDOW_REDRAW=continuous` 档**它照样增长** —— **不要**写成「连续模式下恒 0」。
   > 另外，`interactive_form` **默认**声明 `RedrawPolicy::Continuous`（脚本重放自己推进）；但**M5c 起 `OnDemand` 下也能重放**：
   > 加 `DEER_FORM_ONDEMAND=1` 那一档就是用**唤醒面**自驱的（`next_deadline` / `wake_after`）——
   > 「`OnDemand` 下接口上做不到」这条**已过时**，见 [`window.md`](window.md) 第 6 节的唤醒面。
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
| 以为「不脏就不画」= 事件驱动重绘 | **M5b 起窗口层已默认按需重绘**（`ControlFlow::Wait` + `App::wants_redraw()`，见 [`window.md`](window.md) 第 6 节） | 若你确实看到连续重绘，先 grep 启动自证标记 `[deer-window] 重绘策略：请求=… 实际=…`（可能被 `DEER_WINDOW_REDRAW=continuous` 强制成了 `Continuous`） |

## 6. 做不到 / 已知问题

### 6.1 已知问题（**覆盖缺口，不是功能缺失**）

| 问题 | 现象 | 为什么现有判据抓不住 |
|---|---|---|
| **~~按钮的 focus 视觉几乎不可见~~**（**已修**） | 改前：焦点态与空闲态**实测只差 56 px / 792 px**（其中 32 px 是「圆角补方角」，其余是描边压过字形） | 根因：填充是 `tint(accent, Idle) == accent`、焦点描边也是 `theme.accent`，且测试**只要求差异 > 0**。现在环改用 `theme.on_accent`（与填充的通道平均差 97.7）并**内缩** `FOCUS_RING_INSET` ⇒ 实测 **228 px**、方角补块 **0**；判据是「环带内 ≥ 180 px + 平均通道差 ≥ 64 + 环带外差异 = 0 + 圆角剪影外 = 0」并有反向自检 |
| **~~`node_id_len` 校验和盲区~~**（**已修**，见 3.3） | ~~只比长度、`NodeHint.rect` 不参与校验 ⇒ **等长 id 互换不会 panic**，还给出**错快照**~~ ⇒ 现在 `NodeHint` 带 `node_id_fp`（id 的 FNV-1a 64 指纹），等长互换**确定性 panic** | `NodeHint.rect` **仍**不参与校验（只有 id 参与）；回归见 `r19_equal_length_id_swap_must_be_caught_by_the_id_fingerprint`（改前实测：`aaaa` 的裁剪从 `Some(0,0,5,5)` 错位成 `None`） |
| **~~`Field` 的焦点环贴边画~~**（**已修**） | 与按钮那条**同源**：改前（本 crate 语料）与 idle 差 **684 px**，其中 **464 px 落在期望环带之外**、**32 px 是「圆角补方角」补块** | 修法与按钮同一套：环内缩 `FOCUS_RING_INSET`、并**保留**那条 1px 边框 ⇒ 差异**恰好**是环带（实测 570 px，环带 636）。判据升级成「环带外差异 = 0 + 圆角剪影外 = 0 + 环带内 ≥ 477 px + 平均通道差 ≥ 64」，并反向自检「环色同色 / 近似色 / **几何退回贴边** / 内缩不足 1 px」四处 |
| **焦点环色与标签同色** | 环用 `theme.accent`，按钮/输入框标签也是同色系 ⇒ **环与字形重叠处没有边界** | 要满足「在**任何内容**上都轮廓清楚」需要另立设计（例如描边加深/双色环），不是调一个常量能解决 |
| **脚本语句序号含空段 / 注释段** | `# 注释行\nbad:3` 报「**第 2 条**」 | 序号按 `split(';' / '\n')` 的**段**计数，空段与注释段也占号 ⇒ 报错行号与人的直觉差一位（错误本身仍然可靠） |

### 6.2 仍未做 / 边界（**不要以为能跑**）

**已经落地的**（别再当成没做）：
- **指针捕获**（T3.7，D7 裁定=**按下即默认捕获**）：按下记捕获（`pressed`）；拖拽期间
  移动**路由给捕获者**（hover 钉在捕获节点上，拖出节点/拖出整棵树都不换人）；抬起
  **按捕获者结算** `Clicked` 并释放捕获。**未捕获路径**（没按下 / 按在空白、禁用、
  被裁剪处）行为不变 —— 没按下时 hover 照旧跟随指针；**滚动条拖动**是自己的捕获
  （`scroll.drag`，从不置 `pressed`）⇒ 行为不变。改前「拖出即丢」的实测差异登记见
  `ROADMAP.md` 的 D7 行；
- **方向键上下导航**（T3.1，D2 几何邻近）：严格方向 + 垂直最近 + 水平平手 + 树序兜底；
  无焦点不定义、到边停、聚焦输入框时不消费；
- **按键滚动**（T3.2 收尾）：`PageUp`/`PageDown` 翻一页（视口高）、`Home`/`End` 到顶/到底；
  目标 = 焦点容器优先、退 `hover`（与滚轮同规则）；输入框聚焦时不消费；
- **右键**（T3.3，Q1 纯透传）：按下即发 `PointerRight { id }`（不 pressed/不焦点；禁用子树不发）；
  **中键**只同步 hover；
- **按键重复**（T3.6）：`KeyDown` 带 `repeat: bool`（winit 的 `event.repeat` 首次建模；
  交互层不区分、调用方可过滤；脚本 `key:Tab*` 生成 `repeat: true`）；
- **按时间的动画**：**M5c 起可以在 `OnDemand` 下自驱** —— 用唤醒面（`App::wake_handle` + `Waker::wake_after` /
  `App::next_deadline`，见 [`window.md`](window.md) 第 6 节），**不再必须**声明 `Continuous`（声明了仍然可以）。
  注意 `next_deadline()` 必须返回**固定时刻并自己往前走**（返回 `now()+50ms` 会永远不到点 ⇒ 空转），
  且**本层不做防护**（那等于凭空造超时）；
- **滚动那一支**：滚轮 → `ScrollState` 偏移 → 几何/绘制/命中（见
  [`scroll-and-multiline.md`](scroll-and-multiline.md)）；**可视滚动条**（拖滑块改偏移、
  **点轨道空白跳到指针处**并可续拖）与**惯性驱动的收口函数**（`advance_inertia` +
  `inertia_deadline`，参考实现 `scroll_inertia_window`），见 [`scrollbar.md`](scrollbar.md)；
- **IME 预编辑**（见 [`ime.md`](ime.md)）：只进 `UiState::preedit`、提交才进 `texts` 并清缓冲 ⇒ **无双写**，
  画在光标处 + 下划线、光标推到它之后；
- **输入框光标**（T3.5 建模 + T3.8 渲染）：画面上真的有一根竖线，失焦时不画。

**仍未做**：
- **停靠面板 / 多窗口**；

- **`Focused` 只能人肉验证**（`DEER_INPUT_HOLD=1` 留窗观察），没有自动判据
  （`Ime` 那半已有自动判据，见 [`ime.md`](ime.md)；**真机输入法**仍需人拼一段中文肉眼确认）。

## 7. 相关

- 命中测试的几何依据：[`hit-testing.md`](hit-testing.md)、[`layout.md`](layout.md)
- 绘制列表（`NodeHint` / 裁剪栈的来源）：[`draw-list.md`](draw-list.md)
- CPU 基准后端（像素对照）：[`rendering.md`](rendering.md)、[`pixels.md`](pixels.md)
- 窗口与上屏：[`window.md`](window.md)、[`vulkan-swapchain.md`](vulkan-swapchain.md)
- 节点与属性（`disabled` / `id`）：[`node-tree.md`](node-tree.md)、[`imperative-api.md`](imperative-api.md)
- 选择类控件（r14 结算的语义侧）：[`segmented.md`](segmented.md)、[`chip-group.md`](chip-group.md)、[`tab-bar.md`](tab-bar.md)

## 8. 检查清单（发布前过一遍）

- [x] 示例能跑：`cargo run -p deer-gui --features window --example interactive_form` → `exit=0`（内置脚本有期望终态）
- [x] 纯逻辑回归能跑：`cargo test -p deer-gui --test interaction` / `--test interactive_form`
- [x] 脚本语法与实现一致（`move` / `down` / `up` / `key` / `keyup` / `text` / `focus` / `wheel`；`@id` 标明是示例层展开）
- [x] `FEATURES.md` 已登记（状态 / 指南链接 / 示例命令）
- [x] 明确写了「做不到什么」，并把**已知问题**单列（含「为什么现有判据抓不住」）
