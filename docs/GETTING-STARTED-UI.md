# 从零上手：做一个能点、能打字、能回归的界面

> **这份指南的每一条命令、每一个 API 名都对着本仓库的真代码核对过**（文件与行号写在括号里）。
> 主示例是一个**能真的跑起来**的小程序：`crates/deer-gui/examples/counter.rs`
> —— 标题 + 计数 + `+`/`-` 按钮 + 一个文本输入框。
>
> 前置假设：你已经能在仓库根目录跑 `cargo build`（没装 Rust 请看
> [`GETTING-STARTED.md`](GETTING-STARTED.md)，那份是「从没写过 Rust」的入门）。
> 这一份是**界面与交互**的入门：怎么建树、怎么画、怎么接输入、怎么让它**可回归**。
>
> **只有一棵树、还没有窗口？** 直接看 [`GETTING-STARTED-UI-STEPS.md`](GETTING-STARTED-UI-STEPS.md)（8 步迁移指南，样板 `--example hello_window`）。

---

## 1. 准备：仓库里谁是谁 + 三条能跑的命令

### 1.1 五个 crate 各干什么

| crate | 职责 | 关键类型 / 函数 |
|---|---|---|
| `deer-layout` | 节点树、布局代数、命中测试、`.dui` 场景文件解析 | `Node` / `Kind` / `LayoutProps` / `Builder` / `L` / `layout()` / `hit_test()` / `Geometry` |
| `deer-gpu` | **平台无关的绘制数据** + CPU 参考后端（软件光栅化） | `DrawList` / `DrawCmd` / `Color` / `RectI` / `Theme` / `Extent` / `CpuRenderer` |
| `deer-vk` | Vulkan 后端：自研绑定、自研 SPIR-V 汇编器、离屏与上屏 | `WindowedRenderer` / `FrameOutcome` / `GpuGeometryRenderer` |
| `deer-window` | 窗口 + 事件循环（**本 workspace 唯一引入第三方依赖 `winit` 的地方**） | `App` / `run()` / `WindowConfig` / `WindowInfo` / `Flow` / `InputEvent` / `Waker` / `RedrawPolicy` |
| `deer-gui` | **门面**：一条 `use` 拿到全部能力，并给「离屏出图」的便捷入口 | `prelude` / `render_tree_to_png()` / `interaction` / `input_script` / `window`（要开 feature） |

依赖方向是**单向**的：`deer-gui` → {`deer-layout`, `deer-gpu`, `deer-vk`, `deer-window`}。
`deer-layout` 不知道 `deer-gpu` 的存在（`Builder` 的注释写明了它只维护一棵保留式节点树）。

### 1.2 一切「开窗口」的命令都必须带 `--features window`

窗口层是 **feature 门控**的（`crates/deer-gui/src/lib.rs` 里 `#[cfg(feature = "window")] pub use deer_window as window;`）。
**不带 `--features window` 就跑窗口示例 ⇒ 编译不过**（找不到 `deer_gui::window`）。所以：

```powershell
cd Z:\deer-gui

# ① 本指南的主示例：真窗口 + **纯交互**（本文件第 4–6 节逐段讲它）
#    它会一直开着，直到你按 Esc 或点关闭按钮 —— 它不是「跑完自己退」的演示
cargo run -q -p deer-gui --features window --example counter

# ② 同一个示例的**确定性重放档**（跑完自己退，退出码 0 = 断言全过）
cmd /c "set DEER_COUNTER_SCRIPT=@builtin&& cargo run -q -p deer-gui --features window --example counter"

# ③ 离屏自检（**不需要窗口**）：同一套输入逻辑 + 真字形，CPU 后端出像素并断言
cargo run -q -p deer-gui --features window --example counter -- --headless

# ④ 最小「窗口里画一棵树」（第 2 节的参考实现；**呈现 120 帧后自己退**）
cargo run -q -p deer-gui --features window --example window_preview
```

`-q` 是「安静模式」（少打 cargo 自己的日志）。②③④ 都会**自己退出**并给出退出码，
所以 `$LASTEXITCODE` 可以直接当判据用（实测：三条都是 `0`）。① 是给人用的窗口，
**故意不自己退**。

> **① 到底会做什么**：它**什么都不重放** —— 窗口开着你就能点 `+`/`-`、点输入框打字、
> `Tab` 换焦点、`Esc` 退出。这是刻意的：**默认行为要一眼可预期**。
> （本示例的第一版把「没给脚本」实现成「自动重放脚本」，结果窗口在 `OnDemand` 下
> 只能靠**真实鼠标事件**唤醒事件循环去推进 —— 看上去就是「鼠标一动它才动一下」。
> 那是错的，现在脚本重放必须**显式**要：`DEER_COUNTER_SCRIPT=@builtin`。）

---

## 2. 最小渲染：一棵树 → 一帧画面

整条链路过一遍就懂了（`window_preview.rs` 的模块注释里画的就是它）：

```text
Node 树 ──layout──▶ Geometry ──InteractiveRenderer/DefaultRenderer──▶ DrawList ──▶ 后端（CPU 或 Vulkan）
```

一个**真窗口**里跑完整条链需要四样东西（`counter.rs` 的 `init` + `redraw` 就是这四步）：

```rust
// ① 建树：一棵界面树（纯数据）
let tree = counter_tree(0);

// ② 字体引擎：**可选但强烈建议**（没有它，文字画成等宽占位格）
let engine = TextEngine::from_font_file(Path::new(&font_path), 16.0)?;

// ③ 几何：把树 + 画布盒子 + 文本样式 + 度量 → 每个节点的矩形
let geo = layout::layout(
    &tree,
    Rect::new(0.0, 0.0, extent.width as f32, extent.height as f32),
    TextStyle { font_size: theme.font_size, line_height: theme.line_height },
    &engine.measure(),
);

// ④ 绘制列表 → 上屏
let list = InteractiveRenderer::with_texts(
    theme.clone(),
    &engine.measure(),
    &interact,          // InteractState：hover / focus / pressed
    FieldText::Content, // 输入框画 state.texts 里的内容
    &state.texts,
)
.build(&tree, &geo);
renderer.draw_and_present(&list, Some(&mut engine))?;
```

要点（都是从代码里读出来的，不是经验之谈）：

- **几何每帧重算**：它很便宜，而且能保证「画的东西」与「命中的东西」同源。
  只在尺寸变化时缓存会造成两者不同步。
- **度量必须两处一致**：布局用 `&engine.measure()`（`FontMeasure`），绘制也用同一个
  ⇒ 布局算出来的文字宽度与画出来的宽度不会漂。找不到字体时两侧都用 `ApproxMeasure`
  （确定性近似：每字符 `0.6em`，`crates/deer-layout/src/layout.rs:48`）。
- **字号是「一处定义」**：`theme.font_size` 同时喂给 `TextStyle.font_size`、`DrawCmd::Text.size`
  和 `TextEngine::from_font_file` 的字号。`TextEngine` 会把字号取整
  （实测 `16.0` → `font_size()` 报 `16`），所以**先建引擎、再用 `engine.font_size()` 去建
  `Theme`**（`counter.rs` 的 `init` 与 `headless_selfcheck` 都是这个顺序）。
- **调用方持有引擎**：`draw_and_present(&list, Option<&mut TextEngine>)` 收 `&mut`
  （字符串里出现新字形时要就地光栅化并入图集），所以引擎要放在 `App` 自己的字段里
  （`engine: Option<TextEngine>`），每帧 `as_mut()` 递进去。

只想要一张图（不涉及窗口）的话，用门面里的两步入口就够了：

```rust
use deer_gui::prelude::*;
use deer_gui::render_tree_to_png;

let tree = /* … */;
let png = render_tree_to_png(&tree, 320, 200, Theme::default())?;   // Vec<u8>（PNG 字节）
std::fs::write("ui.png", &png)?;
```

---

## 3. 加布局与文本：`Node` / `Builder` / `L`

### 3.1 两条建树路径，产出同一棵树

```rust
// 路径 A：`Builder`（imgui 手感，调用点即控件）
let mut app = Builder::new(Kind::Column, "app").padding(12.0).gap(8.0);
app.text("标题");
app.container_opts(Kind::Row, "bar", L::new().gap(8.0).to_props(), |r| {
    r.button("确定");
    r.field("请输入");
});
let tree = app.build();

// 路径 B：`Node` 链式构造（**id 完全由你控制**，适合脚本化/断言）
let tree = Node::new(Kind::Column, "app")
    .with_layout(LayoutProps { padding: 12.0, gap: 10.0, ..Default::default() })
    .push(Node::new(Kind::Button, "plus").with_label("+"))
    .push(Node::new(Kind::Text, "count").with_label("count = 0"));
```

`Builder` 的公开入口（`crates/deer-layout/src/builder.rs`）：

| 方法 | 作用 | 返回 |
|---|---|---|
| `Builder::new(kind, id)` / `Builder::auto(kind)` | 建根 | `Builder` |
| `.padding(v)` / `.gap(v)` / `.size(w, h)` | 内边距 / 子间距 / 尺寸（**按值接收 self，必须链式写**） | `Builder` |
| `.text(s)` / `.button(s)` / `.field(s)` | 加一个叶子控件（id 由 `IdGen` 自动生成） | 它的 id（`String`） |
| `.button_opts(s, \|n\| {…})` | 加按钮并改它的节点（例如 `n.id = …` / `n.props.disabled = true`） | 它的 id |
| `.container(kind, id, \|r\| {…})` / `.container_opts(kind, id, L…, \|r\| {…})` | 建子容器（闭包里的节点挂在它下面） | — |
| `.build()` | 定稿成只读的 `Node` | `Node` |

**`Builder` 只有 `text`/`button`/`field` 三个叶子入口，没有「挂一个现成 `Node`」的公开方法**
（`push` 是私有的）⇒ 想在容器里塞一棵**手搭的**子树（比如本示例那个带显式合成 id 的
计数节点），用**路径 B** 更直接。两条路径产出的树结构一致，`layout()` 不区分来源。

### 3.2 id 的生成规则（**脚本与断言都依赖它**）

自动 id 是 `IdGen` **按 kind** 计数的：`column_N` / `row_N` / `text_N` / `button_N` / `field_N`
（`crates/deer-layout/src/node.rs:238`）。显式命名会**占号**（`IdGen::reserve`），所以
`Builder::new(Kind::Column, "app")` 之后第一个自动 Text 是 `text_1`。

> ⚠️ **动态内容一定要显式 id**。本示例的计数显示内容每帧都变；如果它用自动 id
> （`text_1`）就**不稳定** —— 树一变、某个兄弟节点一增一减，id 就换人，脚本里的
> `move @text_1` 会**静默点到空处**。所以 `counter.rs` 用 `Node::with_id` 显式命名
> （`"count"` / `"plus"` / `"minus"` / `"input"`），并在 `assert_counter_tree` 里
> **断言这几个 id 真的存在**。

### 3.3 尺寸、对齐、生长

`LayoutProps` 的字段（`crates/deer-layout/src/node.rs:106`）：

```rust
LayoutProps {
    width: Option<Size>,      // Size::Px(300.0) 或 Size::Pct(50.0)
    height: Option<Size>,
    padding: f32,             // 内边距（**容器只有 padding > 0 时才画底色**）
    gap: f32,                 // 子节点间距
    main_axis: Option<Align>, // 主轴对齐：Align::{Start, Center, End, Stretch}
    cross_axis: Option<Align>,// 交叉轴对齐
    grow: f32,                // 剩余空间的分配权重
}
```

`L` 是它的便捷构造器：`L::new().w(300.0).h(22.0).pad(8.0).gap(8.0).main(Align::Center)
.cross(Align::Stretch).grow(1.0).to_props()`。

本示例用的一条**很实际的技巧**：

```rust
// counter.rs：两个 Row 都给了固定宽度 300
let bar = Node::new(Kind::Row, "bar").with_layout(L::new().w(300.0).gap(8.0).to_props());
```

**为什么**：根 `Column` 的宽度取子节点的最大固有宽度，而计数文本「count = 0」→「count = 12」
会让 `info` 那一行变宽 ⇒ **整棵 Column 的宽度跟着变** ⇒ 按钮的 x 坐标漂移 ⇒ 脚本里写死的
`move @plus`（坐标由布局算出来）在计数变大后**点不中**。给行固定宽度就把坐标钉死了。

> 反过来，没有固定宽度时**别**在脚本里用「展开后的绝对坐标」当长期判据 —— 用 `move @id`
> 让坐标跟着布局走（第 6 节）。

### 3.4 文本是怎么画出来的

三种控件的绘制分支在 `crates/deer-gpu/src/interact.rs`（`InteractiveRenderer::walk`）：

| 节点 | 画什么 | 文字来源 |
|---|---|---|
| `Kind::Text` | 一条 `DrawCmd::Text` | `props.label` |
| `Kind::Button` | `FillRoundRect` + `Text`（居中，且**矩形不越出按钮**）+（聚焦时）`StrokeRect` | `props.label` |
| `Kind::Field` | `FillRoundRect` + `StrokeRect` + `Text`（内缩 `TEXT_INSET` 像素） | `FieldText::Label` ⇒ `props.label`；`FieldText::Content` ⇒ `state.texts["id"]`，没有条目时回退 `props.label` |
| `Kind::Column`/`Row` | 仅当 `layout.padding > 0` 时画 `FillRoundRect` + `StrokeRect` | — |

所以本示例的「计数」就是一个 `Kind::Text` 节点的 `props.label`：
**把 `count` 写进 label，计数变化就必然体现在画面上**（这是第 6 节那条断言的地基）。

真实字形要 `TextEngine`：`CpuRenderer::with_text(engine)`（CPU 侧）/ `draw_and_present(&list,
Some(&mut engine))`（窗口侧）。**没有引擎时**文字走「等宽占位格」——每个字符同一个方块
（`crates/deer-gpu/src/null.rs::draw_text`）。这条差别有一个**很尖的后果**，写在第 7 节。

---

## 4. 加交互：`App::input` → `interaction::handle` → 状态 → 重绘

### 4.1 `App` trait 的全部方法（`crates/deer-window/src/lib.rs:730`）

| 方法 | 何时被调 | 默认实现 |
|---|---|---|
| `init(&mut self, info: &WindowInfo) -> Result<(), String>` | 建窗后**一次**（在这里建渲染器） | **必须实现** |
| `resized(&mut self, width: u32, height: u32) -> Result<(), String>` | 尺寸/DPI 变化（参数是**物理**像素） | 什么都不做 |
| `redraw(&mut self) -> Result<Flow, String>` | 要一帧（画 + 呈现） | **必须实现** |
| `input(&mut self, info: &WindowInfo, ev: &InputEvent) -> Result<Flow, String>` | 每条输入 | 什么都不做 |
| `close_requested(&mut self) -> Flow` | 点关闭按钮 | `Flow::Exit` |
| `wants_redraw(&self) -> bool` | **每条输入派发之后立刻**问一次 | `false` |
| `redraw_policy(&self) -> RedrawPolicy` | 建事件循环前问一次 | `RedrawPolicy::OnDemand` |
| `wake_handle(&mut self, waker: Waker)` | 建窗后**一次**（在 `init` 成功后、引导帧之前） | 什么都不做 |
| `next_deadline(&self) -> Option<Instant>` | 每轮事件循环收敛时（**拉**式预约） | `None` |

`run<A: App + 'static>(config: WindowConfig, app: A) -> Result<(), String>` 是入口。
**它必须在主线程调用**（winit 要求）—— 这正是所谓「M5 的窗口那条链必须是 example
而不是 `#[test]`」的原因：`cargo test` 在子线程跑每个测试。

`WindowConfig::new(title, width, height)` 收的是**逻辑**尺寸（建窗时系统按 DPI 换算）；
`WindowInfo.extent` 与 `InputEvent` 里的坐标是**物理**像素 —— 两者同一套口径。

### 4.2 输入路径只有一条

`input` 收到 `InputEvent` 之后，标准的四步（`counter.rs::Counter::feed`，出自该文件 `:529`）：

```rust
fn feed(&mut self, ev: &InputEvent) -> Result<bool, String> {
    // ① 命中 + 状态机（裁剪快照来自**本帧真实绘制列表**）
    let (tree, geo, clip) = { let f = self.frame(); (f.tree, f.geo, f.clip) };
    let before_state = self.state.clone();
    let before_count = self.count;
    let events = interaction::handle(&mut self.state, &tree, &geo, clip, ev);

    // ② 应用逻辑改界面自己的数据：交互层只说「plus 被点了」，点了要干什么是你的事
    self.apply_events(&events);

    // ③ 脏判据：**状态真的变了**（不是「收到事件了」）
    let changed = self.state != before_state || self.count != before_count;
    self.dirty |= changed;

    // ④ 是否请求退出（Esc 的语义由 App 定）
    Ok(matches!(ev, InputEvent::KeyDown { key: Key::Escape, .. }))
}
```

`interaction::handle` 的完整语义表（`crates/deer-gui/src/interaction.rs:474`）：

| 事件 | 效果 |
|---|---|
| `PointerMoved` | 同步 `hover`（变了才发 `HoverChanged`） |
| `PointerDown { Left }` | 同步 `hover`；**命中的可聚焦控件 ⇒ 聚焦它**（变了才发 `FocusChanged`）；记 `pressed` |
| `PointerUp { Left }` | 同步 `hover`；**抬起处 == 按下处** ⇒ `Clicked`；无论如何清 `pressed` |
| `KeyDown { Tab }` | 按**树序**循环焦点（`Shift` 反向）⇒ `FocusChanged` |
| `KeyDown { Escape }` | 清焦点 ⇒ `FocusChanged(None)` |
| `KeyDown { Enter }` | 焦点在**启用的按钮**上 ⇒ `Clicked` |
| `KeyDown { Backspace }` | 焦点是启用的输入框 ⇒ 删**一个 Unicode 字符** ⇒ `TextChanged` |
| `TextInput` | 焦点是启用的输入框 ⇒ 追加 ⇒ `TextChanged`（**空串不算变化**） |
| `FocusChanged { focused: false }` | 窗口失焦：清 `hover`/`pressed`（`focus`/`texts` 不动） |
| 其余（`Wheel`、`KeyUp`、右/中键、方向键、`Key::Char`/`Other`、`focused: true`） | 本里程碑**不消费** |

**指针与键盘是两条事件**：文本一律走 `InputEvent::TextInput`；`Key::Char('a')` 是**物理键**，
状态机**不消费**它（用 `KeyDown { key: Char(..) }` 当打字输入是常见错误）。

### 4.3 三条容易搞错的语义

**① 命中：`hit()` 复用 `hit_test()`，再加两道判据**

```rust
pub fn hit<'a>(root: &'a Node, geo: &Geometry, clip: ClipSnapshot, x: f32, y: f32) -> Option<&'a Node>
```

- **最深命中者胜出**（后序覆盖），**半开区间** `px ∈ [x, x+w)`；
- **禁用**：命中节点自身**或任一祖先** `props.disabled` ⇒ 整个点不命中；
- **裁剪**：该点在命中节点的有效裁剪外 ⇒ 不命中；
- **不回退到祖先**：命中的是容器就是容器（所以「离开按钮」不等于「没有 hover」）。

**② 裁剪：快照必须从「本帧真实绘制列表」派生**

```rust
let clip = ClipSnapshot::from_draw_list(&list, &tree, &geo);
```

只有**发 `NodeHint` 的渲染器**（`InteractiveRenderer`）能让它非空；`DefaultRenderer`
**不发** ⇒ 快照会是**空的**，而 `allows()` 对**未知 id 放行**（fail-open）⇒ 命中静默退化成
「全不裁剪」。所以**先断言前置**（`counter.rs` 的 `init`）：

```rust
if hints == 0 || f.clip.is_empty() { return Err("……命中会退化成「全不裁剪」而不报错".into()); }
for id in ["plus", "minus", "input"] {
    if !f.clip.is_known(id) { return Err(format!("裁剪快照里没有 `{id}`")); }
}
```

**③ 脏判据要比「状态」而不是比「有没有事件」**

`PointerDown` 就属于「**会改状态、却可能一个 `UiEvent` 都不发**」的那一类（点空白处：
`pressed` 由 `Some` 变 `None`、事件列表是空的）。只看事件会漏掉它，而且漏得很安静。
`UiState::same_visual(&other)` 是给这种比对准备的：只比 `hover`/`focus`/`pressed`（不含 `texts`）。

### 4.4 `wants_redraw` 与省电的关系

`deer-window` 在**每条输入派发之后立刻**读一次 `App::wants_redraw()`：

- 真 ⇒ 请求一帧（`redraw` 被调）；
- 假 ⇒ **不请求**（`FrameCounter` 记一次 `skipped`）。

而 `redraw_policy()` 决定「画完一帧之后要不要续下一帧」：
`RedrawPolicy::OnDemand`（默认，省电；事件循环睡在 `ControlFlow::Wait`）
vs `RedrawPolicy::Continuous`（动画/演示用，空闲也烧 CPU）。
运行时可以用环境变量覆盖：`DEER_WINDOW_REDRAW=continuous`（`deer-window` 会打一行自证日志）。

`wants_redraw` 报的应当与 `redraw` 用的**是同一份** `dirty`（`counter.rs` 就是这么写的）
⇒「窗口层要不要请求一帧」与「App 要不要碰 GPU」共用一个判据，两处不可能漂。

> `wants_redraw(&self)` 是 `&self`：想在里面**记账**就用 `Cell`
> （`interactive_form.rs` 的 `wants_asked: std::cell::Cell<u64>` 是个现成例子）。

---

## 5. 加快捷键与文本编辑

**`Tab` / `Shift+Tab`：不用你写。** 焦点序 = `focusables(root)` 的**树序**：

```rust
pub fn focusables(root: &Node) -> Vec<String>
```

可聚焦 = `Kind::Button` / `Kind::Field`，且**不在禁用子树里**（整棵子树跳过）。
顺序只看树、不看几何 —— 所以「零尺寸的按钮仍在焦点序列里，但点不到」是有意为之。

本示例的焦点序（实测打印）：

```text
[counter] 焦点树序：["plus", "minus", "input"]
```

**`Esc`：语义在**你**这边。** `handle` 只做「清 UI 焦点」；「退出程序」是 App 的决定
（`counter.rs` 的 `feed` 返回 `true` ⇒ `handle_input` 收尾后返回 `Flow::Exit`）。

**文本输入：**

```rust
// 打字（Go 的是 InputEvent::TextInput，不是 KeyDown{Char}）—— 由 deer-window 从 winit 的
// Ime::Commit / KeyboardInput 映射过来；库里也提供 printable_text() 做「控制字符一律丢」的过滤。
InputEvent::TextInput { text: "hi".into() }

// 删一个字符（Unicode 字符，不是字节）：
InputEvent::KeyDown { key: Key::Backspace, mods: Mods::default() }
```

文本缓冲的**唯一真相**是 `UiState.texts: BTreeMap<String, String>`（`id → 内容`，
不在里面的输入框视为空串）；`handle` 只对「焦点节点是 `Field` 且不在禁用子树里」的情况动手。
想把它渲染出来必须用 `FieldText::Content` + `InteractiveRenderer::with_texts(…, &state.texts)`，
否则输入框永远显示 `props.label` 那个占位提示。

`Enter` 在**聚焦的启用按钮**上等价于点击 ⇒ 键盘也能按 `+`。

---

## 6. 让它可回归：脚本重放 + dirty 账本

### 6.1 脚本语法（`deer_gui::input_script`）

语句用 `;` 或换行分隔；`#` 到行尾是注释；**空语句被忽略**；**语法错一律 `Err(String)`**
（带语句序号与原文）—— 刻意不「跳过看不懂的语句」，否则脚本作为判据就悄悄失效了。

| 语法 | 含义 |
|---|---|
| `move:X,Y` | 指针移到 `(X, Y)`（f32，负数/贴边都行） |
| `down:left` / `down:right` / `down:middle` | 按下；坐标 = **最近一次 `move`**（没 move 过就是 `(0,0)`） |
| `up:left`（同上三档） | 抬起 |
| `key:Tab` | `KeyDown`；可写 `Escape`/`Esc`/`Enter`/`Backspace`/`Left`/`Right`/`Up`/`Down`/`Other`/`Char(a)` |
| `shift+key:Tab`（或 `key:shift+Tab`） | 带修饰键；`ctrl+`/`alt+`/`sup+` 同档、可叠加，前缀写在**动词侧或键名侧都行** |
| `keyup:Char(a)` | `KeyUp` |
| `text:hi 你好` | 一段文本输入（原样，含空格与中文；**追加**不是覆盖） |
| `focus:on` / `focus:off` | 窗口失焦 / 重新获得窗口焦点 |
| `wheel:0,3` | 滚轮（状态机不消费；可用来验证「不消费的事件不改状态」） |

解析器只有一个：`input_script::parse_script(src) -> Result<Vec<InputEvent>, String>`，
窗口侧与测试侧共用它 ⇒ 脚本语法只有一处定义。

> `move @节点id` **不是库语法**，是示例层的模板展开：`counter.rs` 的 `resolve_script`
> 先把 `move @plus` 换成 `move:26.5,51`（坐标由**布局**算出来），再交给 `parse_script`。
> **为什么值得抄这一手**：写死坐标的脚本在布局一变后就静默点到空处，于是「重放通过」
> 变成一句空话。

### 6.2 本示例的可自动判定断言（**这就是它的门槛**）

「点两次 `+` ⇒ 计数显示为 2」被断在两个层次上：

```rust
/// 层次一：绘制列表画出来的文本 == 期望（`counter.rs::assert_count_shown`，出自 `:313`）
fn assert_count_shown(frame: &Frame, expected: i32) -> Result<(), String> {
    let want = format!("{COUNT_PREFIX}{expected}");          // "count = 2"
    let drawn = frame.drawn_texts();
    let got = drawn.get("count");
    // … got == Some(want) 才算过；打印时把整帧文本一起打出来，便于看错在哪
}

/// 层次二：像素级（`counter.rs::assert_pixels_show_count`，出自 `:411`）
/// - `count` 矩形内**必须有**差异（文本真的换了字形）；
/// - 所有期望矩形（count + 这一帧视觉真的变了的按钮）**之外必须为 0**（没溢出）。
```

**怎么读「绘制列表画出来的文本」**：列表里第 k 条 `NodeHint` 对应**前序里第 k 个有几何的
节点**（与 `ClipSnapshot::from_draw_list` 逐字相同的绑定口径），某条 `NodeHint` 之后、
下一条之前的 `Text` 命令就属于它。`counter.rs::Frame::drawn_texts` 就是这几行。

**跑法**（脚本建议走环境变量，理由见第 8 节）。`DEER_COUNTER_SCRIPT` 有三档取值：

| 取值 | 行为 |
|---|---|
| **没设** | **纯交互**：不重放任何事件，窗口一直开着（`Esc` / 关窗退出） |
| `@builtin` | 内置脚本（`+` 两次、点输入框、输 `ok`）→ **有期望终态**，跑完自己退 |
| 其它字符串 | 把它当脚本重放；只断言「计数显示与 `count` 一致」，不断言终态 |
| `@none` / `@interactive` | 同「没设」（显式表达「我就是要交互」） |

```powershell
# 门槛档：脚本跑完自己退出，退出码 0 = 终态断言全过（实测 exit=0）
cmd /c "set DEER_COUNTER_SCRIPT=@builtin&& cargo run -q -p deer-gui --features window --example counter"
cmd /c "set DEER_COUNTER_SCRIPT=move @plus;down:left;up:left;move @plus;down:left;up:left&& cargo run -q -p deer-gui --features window --example counter"

# 不用带窗口的门槛（CI 友好）：离屏自检，同一套逻辑 + 真字形 + 像素断言（实测 exit=0）
cargo run -q -p deer-gui --features window --example counter -- --headless
```

实测输出（关键行，照抄）：

```text
像素断言 count 0→1   : 差异 count 矩形内 68 像素（框 RectI { x: 12, y: 72, w: 87, h: 18 }）；同时变了的按钮 ["plus"] 内 [393]；**框外 0** 像素
像素断言 count 1→2   : 差异 count 矩形内 42 像素（框 RectI { x: 12, y: 72, w: 87, h: 18 }）；同时变了的按钮 ["plus"] 内 [393]；**框外 0** 像素
断言 count 显示 : 期望 `count = 2`；绘制列表实际 Some("count = 2")
终态断言    : count=2 input="ok"；整帧画出来的文本 = {"count": "count = 2", "input": "ok", "minus": "-", "plus": "+", "title": "deer-gui counter"}
离屏自检 ✅：点两次 `+` ⇒ 计数显示 `count = 2`（绘制列表证据 + 像素证据）；输入框画出 `ok`
```

### 6.3 dirty 账本怎么读

`counter.rs` 每调用一次 `redraw` 就记一笔：`true` = 真的画了，`false` = 状态没变、跳过（**不碰 GPU**）。
收尾时打印：

```text
dirty 账本  : 真的重绘 10 次 / 跳过 1 帧（共记账 11 帧）；序列 = [true, true, true, true, false, true, true, true, true, true, true]
              （painted=10 + skipped=1 = 记账 11 帧；`rendered`=10）
```

窗口层另外打自己的账（`[deer-window]` 前缀，`deer-window` 自己打的，不靠 App）：

```text
[deer-window] 重绘账本：requests=17 skipped=2 frames=11
[deer-window] 唤醒账本：wake=1 wake_after=8 fired=4 requested=5 skipped=0 iters=19
```

- `skipped` = 「派发了输入但**没有**请求重绘」的次数（省下的那一帧）；
- `iters` = 事件循环迭代次数 —— **空闲时必须是低值**，它是「没在空转」的可数证据。

> ⚠️ **`skipped` 与重绘策略正交**：它衡量的是「这条输入有没有改变状态」，
> 所以在 `DEER_WINDOW_REDRAW=continuous` 档**照样增长** —— 不要写成「连续模式下恒 0」。

**「只在变化时重绘」怎么证明**：跑两段脚本对照 —— ① 有动作的（每条输入都改状态）；
② 空转的（例如只剩 `wheel:0,3`）。有动作的那档 `frames` 随动作涨；空转那档
`skipped` 随输入条数涨而 `frames` **不涨**。窗口层级的完整演示是
`cargo run -p deer-window --example idle_probe`（它**默认要求这次真的收到过输入**，
否则 `exit=1` 并写明「前置不成立」—— 见第 8 节第 12 条）。

### 6.4 一个**必须自己处理**的边界（本示例第一版就挂在这）

脚本重放是「每画一帧喂一条事件」。当某一帧**不脏**（那条事件没改状态）时，我们
**仍然把它喂掉了** —— 如果那条事件**改了状态**，`dirty` 现在是 `true`，而窗口层早在
「派发输入之前」就度过 `wants_redraw()`（那时是 `false`）⇒ **它不会再要一帧，脚本停在这里**。

实测现场（进程卡住、不退出）：

```text
redraw: no（状态没变，跳过这一帧；已画 4 帧）
input: PointerDown { … } => 状态变了=true dirty=true events=[] …
```

修法是**自己叫一声**（`OnDemand` 下这正是唤醒面的用途）：

```rust
if self.dirty && !self.done {
    let waker = self.waker.clone().ok_or("需要唤醒句柄")?;
    waker.wake();   // 「叫一声，窗口层随后问一次 wants_redraw」→ 此刻是 true ⇒ 拿到那一帧
}
```

脚本的**常规**推进仍用 `waker.wake_after(Duration::from_millis(16))`：那是「排一步」的语义，
只装一个 deadline、**不睡线程**，事件循环平时仍睡在 `ControlFlow::Wait` 上。
（`wake()` 与 `wake_after()` 的区别见 `deer-window` 的 `Waker` 文档：前者与输入同一把尺 ——
先问 `wants_redraw`；后者是 App 自己下的单 —— 到点**一律**画一帧。）

---

## 7. 怎么验证自己没搞坏

### 7.1 四条常用命令

```powershell
# ① 全套断言（本次实测基线：42 个测试套件、**475 passed / 0 failed**；数字会随里程碑涨，别写死）
cargo test --workspace

# ② clippy 干净（本项目 `#![deny(clippy::all)]`；本次实测 0 warning）
cargo clippy -p deer-gui --features window --all-targets

# ③ 文档里提到的示例必须真实存在（本次实测 5 passed）
cargo test -p deer-gui --test docs_consistency

# ④ 本指南的主示例 + 它的离屏自检
cargo run -q -p deer-gui --features window --example counter -- --headless

# ⑤ **不想抄样板**：testkit —— 上面那套「树 → 几何 → 列表 → 裁剪快照 → 像素 → 断言」
#    已经变成 API（前置断言 / 越界 = 0 / CPU↔GPU 对照 / 可复制的复现命令都默认带上）
#    见 `docs/features/testing.md`
cargo run -q -p deer-gui --features testing --example testkit_demo
```
> **④ 与 ①–③ 不要并发跑**：`cargo test --workspace` 会**编译** `crates/deer-gui` 的全部 target，
> 而窗口示例与它共用同一份构建产物与构建锁 —— 串行跑最省心（理由见第 8 节第 4 条）。

### 7.2 离线 GPU vs CPU 逐像素对照

上屏那条链有**独立的像素判据**，跟「逻辑对了」不是一回事：

```powershell
# 上屏 parity：把窗口里**真实的像素**读回来与 CPU 后端逐像素对照
cmd /c "set DEER_VK_WINDOW_TESTS=1&& cargo run -q -p deer-gui --features window --example window_parity"
```

判据（**这两条数字是验收线，别放宽**）：

| 语料 | 判据 |
|---|---|
| **不透明**（形状 / 文本 / 界面树） | **逐字节相同**（最大通道差 **0**） |
| **半透明**（`alpha` 混合、圆角、半透明文本） | 最大通道差 **≤ 1 LSB**（**实测上限，不是证明上界**：CPU `round()` vs GPU UNORM 定点混合） |

> **1 LSB 不许断言成 0**：8 位 UNORM 的舍入与 CPU 的 `f32` 舍入在个别像素上必然差 1。
> 反过来，**不透明那档也不许放宽到 1** —— 那是这条判据真正的牙齿。
>
> 前提：交换链必须是**线性** `*_UNORM`（不是 `_SRGB`）。`window_preview.rs` 里有一条
> `assert!(linear, …)` 直接钉住这件事 —— sRGB 附件连**混合**都发生在线性空间，
> 与 CPU 的字节空间实现差得远（实测某算例差 **44** 字节）。

### 7.3 「我的改动让像素变了」怎么定位

`counter.rs` 的像素断言是**可复用**的模板：**把「这一帧同时变了的所有东西」都列成矩形，
然后要求框外差异 = 0**。

> 本示例第一版就栽在这里：只列了 `count` 矩形，结果框外冒出 **393** 个差异像素 ——
> 那不是「溢出」，是**鼠标按在 `+` 上，按钮自己的 hover/pressed/focus 视觉也变了**。
> 断言必须把参与变化的东西都写进去（`buttons_whose_visual_changed` 就是干这个的）。

### 7.4 一个**必须知道**的陷阱：占位字形让文本变化在像素上不可见

`CpuRenderer::new()`（**无字库**）给**每个字符画同一个等宽方块** ⇒ `count = 0` 与 `count = 1`
的像素**逐个相同**（实测：矩形内差异 **0** 像素，而绘制列表里的文本确实从 `0` 变成了 `1`）。

⇒ **文本类像素断言必须用 `CpuRenderer::with_text(engine)`**（真字形）；用占位档只会得到一条
**假红**。本示例的 `--headless` 因此**先建字体引擎**，找不到字体时**明确降级**
（只做文本级断言，并打印「像素断言被跳过」），不假装通过。

---

## 8. 常见坑（以下每条都在本仓库里核实过）

| # | 现象 | 原因与解法 |
|---|---|---|
| **1** | 门槛变量「设了却像没设」 | `cmd` 的 `set X=1 && …` 会把 `&&` **前的空格**算进变量值 —— 实测 `cmd /c "set X=1 && set X"` 打印 `X=1 `（**带一个尾空格**），写 `set X=1&& …` 才是 `X=1`。**判定必须先 `trim()` 再比**：本仓库的门槛统一委托给 `deer_gui::env_gate::flag`（`flag_default_true` 是反向默认）。自己写的判定别忘了这一条 |
| **2** | 脚本参数「传进去了却没生效」 | 实测：`--script "move @plus;down:left;up:left;…"` 这种带空格/分号/`@` 的长参数被 shell 截成了 `move;`，**解析器没报错**（它是一段合法脚本）⇒「重放通过」变成空话。**用环境变量**（`set VAR=…`）过脚本，没有这层转义 |
| **3** | 窗口「开着不动」，一动鼠标它才动一下 | 这是**本示例第一版的真实缺陷**，成因值得记住：在 `OnDemand` 下 App 自己排的 `Waker::wake_after` deadline 没兑现时，**真实输入事件**会顺手把事件循环叫醒 ⇒ 表现成「鼠标一动窗口才动」。**纪律**：给人看的示例，**默认档必须一眼可预期** —— 交互就是交互（一直开着），脚本重放就显式要（`DEER_COUNTER_SCRIPT=@builtin`）。若你写的 App 也有「该自己推进却推不动」的现象，先查 `wake_after` 之后**有没有真的排下一次**（`App::next_deadline` 返回 `now()+50ms` 这类写法会永远不到点） |
| **4** | `example` 与 `cargo test` 的并发 | **不要**把「开窗口的 example」与 `cargo test` 塞进同一条流水线并发跑。两条硬理由（都核实过）：① winit 要求事件循环在**主线程**，而 `cargo test` 在**子线程**跑每个测试 ⇒ 窗口链路**只能**是 example（`window_preview.rs` 的模块注释与 `hal_window_path.rs:8-12` 都写着这条）；② 两者会共用 cargo 的构建锁/构建产物（实测并发两次 `cargo` **不会报错**：后启动的那次等在锁上 —— 它是串行的，不是并行的，别指望靠它省时间）。**先 `cargo test`，再跑 example。** |
| **5** | `cargo clean -p <crate>` 的时机 | 只在「改了**构建脚本 / feature 组合**、而增量编译结果明显不对（换了 feature 却还用旧的 `.rlib`）」时用。**不要在「窗口 example 与测试交替跑」时用它**：它会把整套依赖重编一遍（分钟级），而那个问题通常根本不存在 —— 先看第 4 条的串行顺序 |
| **6** | 按钮的焦点环几乎看不见 | **已知缺陷**（`docs/features/input.md` 第 6.1 节）：焦点态与空闲态实测只差 **56 px / 792 px**，其中 32 px 还是「圆角补方角」的补块 —— 按钮的填充是 `theme.accent`、焦点描边**也是** `theme.accent`（按钮那支用的是 `theme.on_accent`，实测可见；但设计上环与字形同色系时边界仍不清楚）。**输入框的焦点是可见的**（描边加宽 + 填色回到 idle 色） |
| **7** | `node_id_len` 校验和盲区 | **已知缺陷**（同上）：`DrawCmd::NodeHint` **不带 id**，`ClipSnapshot::from_draw_list` 只能把「第 k 条提示」绑到「第 k 个有几何的节点」，并用 `node_id_len` 当校验和 —— 只比长度 ⇒ **等长 id 互换不会 panic，还给出错快照**（例：`button_1` 的 clip 本应 `Some(0,0,60,40)`，错位后变成 `None` = 「不裁剪」）⇒ 裁剪护栏**静默失效**。所以**别把节点 id 命名成一堆等长的**（`btn_a` / `btn_b`），并像本示例那样**先断言 `clip.is_known(id)`** |
| **8** | 点了没反应 | 先分三件事查：① `wants_redraw` 答假（状态其实没变）⇒ 不会请求帧；② 列表来自 `DefaultRenderer`（**不发 `NodeHint`**）⇒ 裁剪快照是空的、且**不报错**；③ 节点（或任一祖先）`props.disabled` ⇒ 整个子树不响应，**也不回退到祖先** |
| **9** | 打字打不进去 | 用 `KeyDown { key: Char('a') }` 当输入 —— 状态机**不消费** `Key::Char`。文本一律走 `InputEvent::TextInput` |
| **10** | `Tab` 顺序跟想的不一样 | 可聚焦集合是 `Button` / `Field`，按**树序**，禁用子树整棵跳过。容器与文本**不在**序列里 |
| **11** | 界面一闪一闪、空闲还烧 CPU | 检查是不是把 `redraw_policy` 写成了 `RedrawPolicy::Continuous`，或环境变量被强制了 —— `deer-window` 启动时会打一行自证：`[deer-window] 重绘策略：请求=… 实际=…（App 声明=…）`；`DEER_WINDOW_REDRAW=continuous` 会**无条件**覆盖成连续 |
| **12** | 用 `idle_probe` 想证明省电却 `exit=1` | 它**默认要求这一次真的收到过输入**（`wants_redraw` 只在每条输入之后被问一次；0 输入时「省电」与「变化」两档**无法区分**，所以它显式判失败并在 stderr 写明「前置不成立」）。要拿到绿：跑的时候**真的动一下鼠标/滚轮**，或用 `DEER_IDLE_DIRTY=1` 那档看「变化探针」。`DEER_WINDOW_REDRAW=continuous` 那一档是唯一例外（0 输入下仍 `exit=0`） |

---

## 9. 下一步

- **读源码**：`crates/deer-gui/examples/counter.rs`（本指南的主角，逐段有注释）；
  再读 `interactive_form.rs`（同骨架 + 四档像素数字 + `OnDemand` 下 `wake_after` 自驱重放）。
- **逐功能细节**：`docs/features/input.md`（输入与焦点）、`docs/features/window.md`（窗口与唤醒面）、
  `docs/features/layout.md` / `node-tree.md` / `imperative-api.md`（建树与布局）、
  `docs/features/pixels.md` / `rendering.md`（绘制与像素）。
- **更慢的 Rust 入门**：[`GETTING-STARTED.md`](GETTING-STARTED.md)。
- **全景教程**：[`TUTORIAL.md`](TUTORIAL.md)。
- **能做到什么 / 做不到什么**：[`../FEATURES.md`](../FEATURES.md)（唯一真相）与
  `docs/features/input.md` 第 6 节的「已知问题 / 仍未做」。
