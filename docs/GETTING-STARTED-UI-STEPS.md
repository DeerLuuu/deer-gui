# 分步迁移：我只有一棵界面树 → 怎么变成有窗口的界面

> **这份指南的每一步都对着本仓库的真代码与真跑过的输出核对过**（文件/函数名写在括号里，
> 输出照抄本次实测）。它要解决的是一个**很具体的处境**：
>
> > 「我照教程写了建树的代码，程序也跑起来了 —— 但**没有窗口**，屏幕上什么都没有。」
>
> 配套样板：[`crates/deer-gui/examples/hello_window.rs`](../crates/deer-gui/examples/hello_window.rs)
> —— 界面只有三样（标题文本 + 两个按钮、其中一个禁用 + 一个输入框），**代码里的中文注释逐段
> 标了【步骤 N】，与本文的 8 步一一对应**。
>
> 与 [`GETTING-STARTED-UI.md`](GETTING-STARTED-UI.md) 的分工：那一份讲「一个完整的交互界面是
> 怎么构成的」（主示例 `--example counter`）；**这一份只讲从「有树」到「有窗口」这段路**，
> 每一步都给出「加什么代码 / 应该看到什么 / 怎么验证」。

先看全貌 —— 左边的树就是你已经有的东西，右边的窗口是这段路要走到的位置：

```text
Column "app"                    +--------------------------+
├─ Text  "title"                |  deer-gui hello window   |  ← ① 树（你已有）
├─ Field "input"        ───▶    |  [ type here          ]  |  ← ② 几何（layout）
└─ Row   "bar"                  |  [OK] [Off]              |  ← ③ 绘制列表（InteractiveRenderer）
   ├─ Button "ok"               +--------------------------+  ← ④ 窗口（deer-window）
   └─ Button "off" (disabled)
```

---

## 步骤 1　检查起点：确认你手上这棵树是**完整的**

### 加什么

先把你已有的建树代码收成一个函数，并且**把 `Builder` 返回的 id 存下来** —— 后面每一步的
自检都要按 id 找节点：

```rust
use deer_gui::prelude::*;

/// 建树拿到的东西：树 + 几个要在断言里按 id 定位的节点 id。
struct Widgets {
    tree: Node,
    title: String,
    field: String,
    ok: String,
    off: String,
}

fn build_widgets() -> Widgets {
    let mut app = Builder::new(Kind::Column, "app").padding(12.0).gap(10.0);
    let title = app.text("deer-gui hello window"); // 标题文本
    let field = app.field("type here");            // 输入框
    let mut ok = String::new();
    let mut off = String::new();
    app.container_opts(Kind::Row, "bar", L::new().w(300.0).gap(8.0).to_props(), |r| {
        ok = r.button_opts("OK", |n| n.id = "ok".into());
        off = r.button_opts("Off", |n| {
            n.id = "off".into();
            n.props.disabled = true; // ← 第二个按钮：禁用
        });
    });
    let tree = app.build();
    Widgets { tree, title, field, ok, off }
}
```

> **为什么按钮的 id 能在闭包里改**：`Builder::button_opts(label, f)` 先建节点、把节点交给
> `f` 去改，**最后**才定 id —— `f` 里把 `n.id` 写成 `"ok"`，它就是最终 id
> （`crates/deer-core/src/builder.rs:96`、`:110`）。
> 但 `Builder::text()` / `Builder::field()` **没有**这个口子，它们的 id 由 `IdGen` 自动生成
> （`crates/deer-core/src/node.rs:238`）⇒ 只能**接住返回值**。

### 应该看到什么

跑起来**什么都没有**：没有窗口、没有像素、没有报错（退出码 0）。这不是坏事 —— 它说明
「树」这一步本身是好的，缺的是后面七步。

### 怎么验证

```rust
let w = build_widgets();
println!("子节点 {} 个：{:?}", w.tree.children.len(), /* 逐节点 id */);
```

本示例实测打印（**注意自动 id 长什么样**）：

```text
[hello_window] 树：root=app 子节点 3 个 ⇒ ["app", "text_1=\"deer-gui hello window\"", "field_1=\"type here\"", "bar", "ok=\"OK\"", "off=\"Off\""]
```

- `app` / `bar` / `ok` / `off` 是我们显式起的名字；
- **`text_1` / `field_1` 不是** —— 它们是 `IdGen` 按 kind 计数的产物。
  所以「按 id 定位」的代码不要写死 `"text_1"`，用**返回值**。

---

## 步骤 2　`build()` 只调一次，而且在加完子节点之后

**这一步就是那个最常见的坑。** 下面两种写法都是错的，而且**都不报错**：

```rust
let mut app = Builder::new(Kind::Column, "app").padding(12.0).gap(8.0);
let tree = app.build();        // ❌ 早了一步：之后加的节点都不在这棵树里
app.text("标题");
app.field("请输入");
let tree = app.build();        // ❌ 又调了一次：拿到的还是那一刻的快照
```

`Builder::build(&self) -> Node` 是**克隆当前树**（`crates/deer-core/src/builder.rs:182`），
不是「定稿」。早调得到的是**一棵缺子节点的树**；而缺子节点的树照样能一路走到
「一张只有清屏色的图」—— 于是「没有窗口/没有内容」这个现象在后面的步骤里也是
**静默**的。

### 加什么

正确写法只有一个：**一路加完，最后 `build()` 一次**（就是步骤 1 那段代码的最后三行）。

### 应该看到什么

`builder.rs` 里 `build()` 的调用点在整个示例里**只有一处**。用编辑器全局搜一下
`app.build()` 应该只命中一行。

### 怎么验证

**显式断言**子节点数量 —— 这是唯一能让「`build()` 调早了」立刻喊疼的办法：

```rust
let n = f.tree.children.len();
if n != 3 {
    return Err(format!(
        "界面树根 `{}` 应当有 3 个子节点（标题 / 输入框 / 按钮行），实际 {n} —— \
         十有八九是 `build()` 调早了或调了两次",
        f.tree.id
    ));
}
```

不做这一步的话，`children.len() == 0` 会一路走到「画出一张空图」而不报错。

---

## 步骤 3　加几何：`layout()`

树只描述**结构**，画之前得先算出每个节点的矩形。

### 加什么

```rust
use deer_gui::layout::layout::{self, Geometry};
use deer_gui::prelude::*; // Rect / TextStyle / Measure / ApproxMeasure / Extent / DrawList…

fn layout_of(tree: &Node, theme: &Theme, extent: Extent, measure: &impl Measure) -> Geometry {
    layout::layout(
        tree,
        Rect::new(0.0, 0.0, extent.width as f32, extent.height as f32), // 画布盒子
        TextStyle { font_size: theme.font_size, line_height: theme.line_height },
        measure,                                                        // 文本度量
    )
}
```

三个要点（都是从代码里读出来的）：

- **画布盒子**就是窗口的物理尺寸（离屏档用同一套数值 ⇒ 两边出的图可以直接比）；
- **`TextStyle` 与 `Theme` 共用同一个 `font_size`**，别各写一个数；
- **几何每帧重算**（`crates/deer-gui/examples/counter.rs:206`）：它很便宜，而且能保证
  「画的东西」与「命中的东西」同源。只在尺寸变化时缓存会造成两者不同步。

### 应该看到什么

`layout()` 返回 `Geometry`（节点 id → 矩形）。**这一步仍然没有窗口**，但你可以把几何打出来了。

### 怎么验证

```rust
println!("几何 `bar` = {:?}", geo.get("bar").map(|r| format!("({:.1},{:.1} {:.1}×{:.1})", r.x, r.y, r.w, r.h)));
```

实测（360×220 的窗口，根 Column 的 `padding = 12.0`、`gap = 10.0`）：

```text
[hello_window] 几何 `bar` = Some("(12.0,72.0 300.0×22.0)")
```

`geo.get("bar")` **必须有值** —— 它为空说明树是空的（回去看步骤 2）或者布局没接上。

---

## 步骤 4　加绘制列表与裁剪快照

几何是「控件在哪」，绘制列表是「要画哪些命令」。

### 加什么

```rust
use deer_gui::gpu::interact::{FieldText, InteractState, InteractiveRenderer};
use deer_gui::interaction::ClipSnapshot;

fn draw_list_of(
    tree: &Node,
    theme: &Theme,
    geo: &Geometry,
    state: &UiState,          // 交互状态（步骤 6 才会变；先给 UiState::default()）
    measure: &impl Measure,
) -> (DrawList, ClipSnapshot) {
    // 渲染层只认 UiState 的**只读子集** InteractState（避免 deer-gpu 反向依赖 deer-gui）
    let interact = InteractState {
        hover: state.hover.clone(),
        focus: state.focus.clone(),
        pressed: state.pressed.clone(),
    };
    let list = InteractiveRenderer::with_texts(
        theme.clone(),
        measure,
        &interact,
        FieldText::Content,   // 输入框画 state.texts 里的内容（没打过字时退回占位 label）
        &state.texts,
    )
    .build(tree, geo);
    // 裁剪快照**必须**从「这一帧真实的绘制列表」派生
    let clip = ClipSnapshot::from_draw_list(&list, tree, geo);
    (list, clip)
}
```

### 应该看到什么

- 列表里每个「有几何的节点」**先发一条 `NodeHint`**（`crates/deer-gpu/src/interact.rs:236`）；
- 裁剪快照因此非空；
- **还是没有窗口。**

### 怎么验证（**这一步的护栏最关键**）

```rust
let hints = list.counts().node_hint;
if hints == 0 || clip.is_empty() {
    return Err(format!(
        "前置条件不成立：这一帧的列表没有节点提示（node_hint={hints}，快照 len={}）—— \
         命中会退化成「全不裁剪」而**不报错**；检查是不是用了不发 NodeHint 的渲染器",
        clip.len()
    ));
}
for id in ["ok", field_id.as_str()] {   // ⚠️ 必须是步骤 1 **接住的返回值**
    if !clip.is_known(id) {
        return Err(format!("前置条件不成立：裁剪快照里没有 `{id}`"));
    }
}
```

> 这条断言**当场咬到过本示例自己**：第一版把字段节点的 id 写成 `"input"`，实测直接红 ——
> 因为 `Builder::field()` 给的是自动 id `field_1`（见步骤 1 的打印）。**能红的断言才叫护栏**。

实测：

```text
[hello_window] 绘制列表：16 条命令（NodeHint 6 条）；裁剪快照 6 个节点
```

**为什么为 0 会「静默退化成全不裁剪」**：`ClipSnapshot::allows()` 对**未知 id 放行**
（fail-open，`crates/deer-gui/src/interaction.rs:175`）。这是**刻意**的 —— `unclipped()`
必须让整个界面可用。代价是：快照为空（或漏了某个节点）时，命中测试**不会**把任何点裁掉，
界面看起来完全正常，而「被裁掉就不命中」这条护栏**已经不存在了**。

谁会踩到它：**用不发 `NodeHint` 的渲染器**。`DefaultRenderer`（`build_draw_list`）**不发**，
只有 `InteractiveRenderer` 发 —— 所以走交互路线的界面必须用后者。

---

## 步骤 5　加窗口壳：窗口出现，但**点不动**

前四步全在「值」的世界里。这一步才建真窗口。

### 加什么

```rust
use deer_gui::vk::windowed::{FrameOutcome, WindowedRenderer};
use deer_gui::window::{App, Flow, WindowConfig, WindowInfo, run};

struct Hello {
    tree: Node,
    theme: Theme,
    extent: Extent,
    state: UiState,
    dirty: bool,
    engine: Option<TextEngine>,          // 字体引擎（步骤 7 才有；先把字段留好）
    renderer: Option<WindowedRenderer>,
}

impl App for Hello {
    /// 建窗后**调一次**：字体 → 渲染器 → 前置断言。
    fn init(&mut self, info: &WindowInfo) -> Result<(), String> {
        self.extent = Extent { width: info.extent.width.max(1), height: info.extent.height.max(1) };
        // ① 渲染器：窗口 + 交换链 + 呈现（`0` = 第 0 个 Vulkan 适配器）
        let r = WindowedRenderer::new(0, info.raw, self.extent, CLEAR)
            .map_err(|e| format!("创建窗口渲染器失败：{e}"))?;
        println!("适配器：{}", r.adapter().name);
        self.renderer = Some(r);
        // ② 前置断言（步骤 2、3、4 的三条自检放这里）
        self.dirty = true;               // 首帧必然要画
        Ok(())
    }

    fn redraw(&mut self) -> Result<Flow, String> {
        if !self.dirty { return Ok(Flow::Continue); }
        self.dirty = false;
        let list = /* 步骤 4 的绘制列表 */;
        let mut engine = self.engine.take();     // 引擎要 `&mut`（新字形要就地光栅化）
        let r = self.renderer.as_mut().ok_or("还没有窗口渲染器")?;
        let outcome = r.draw_and_present(&list, engine.as_mut());
        self.engine = engine;
        match outcome {
            Ok(FrameOutcome::Presented) => {}
            Ok(FrameOutcome::OutOfDate) => {     // 交换链过期是**正常路径**，不是成功
                let e = self.extent;
                self.renderer.as_mut().expect("上面刚确认存在").resize(e)
                    .map_err(|err| format!("交换链过期后重建失败：{err}"))?;
                self.dirty = true;
            }
            Err(e) => return Err(format!("呈现失败：{e}")),
        }
        Ok(Flow::Continue)
    }

    fn resized(&mut self, width: u32, height: u32) -> Result<(), String> {
        let e = Extent { width: width.max(1), height: height.max(1) };
        if let Some(r) = self.renderer.as_mut() { r.resize(e).map_err(|e| format!("重建交换链失败：{e}"))?; }
        self.extent = e;
        Ok(())
    }
}

fn main() -> ExitCode {
    // ⚠️ `run()` 必须在**主线程**调用（winit 要求）⇒ 窗口那条链只能是 example，
    //    不能是 `#[test]`（`cargo test` 在子线程跑每个测试）。
    match run(WindowConfig::new("deer-gui hello window", 360, 220), app) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => { eprintln!("失败：{e}"); ExitCode::FAILURE }
    }
}
```

**`Cargo.toml` 必须给这个示例加 `required-features`**（照本仓库其它窗口示例的写法，
`crates/deer-gui/Cargo.toml:22`、`:39`、`:46`）：

```toml
[[example]]
name = "hello_window"
required-features = ["window"]
```

少了这段，`cargo test --workspace` 会用**默认 feature**（`default = []`，不含 `window`）
去编译它 ⇒ 那个构建**直接红**，而不是「跳过」（`deer_gui::window` 在没开 feature 时不存在）。
加上之后的行为是**明确跳过**，实测报错信息是这句：

```text
error: target `hello_window` in package `deer-gui` requires the features: `window`
```

### 应该看到什么（**这一步的现象很特别**）

```text
[deer-window] 窗口已建：title="deer-gui — deer-gui hello window" extent=360x220 platform=Windows handle(HWND)=0x1A0676 display(HINSTANCE)=0x7FF6DE040000
[hello_window] 适配器：Intel(R) RaptorLake-S Mobile Graphics Controller
[hello_window] redraw: yes（第 1 帧）命令 16 条 hover=None focus=None texts={}
```

**窗口出现了、内容也画出来了，但点不动** —— 因为 `App::input` 的默认实现**什么都不做**
（`crates/deer-window/src/lib.rs:759`）。而且它空闲时**不会**重画（默认
`RedrawPolicy::OnDemand`）。「窗口出现但没反应」在这一步是**预期现象**，不是 bug。

### 怎么验证

① 人看的档：窗口开着，只是点没反应（`Esc` 也退不出去 —— 那也要步骤 6 接）。
② 自动化的档：加一个**帧数门槛**，让窗口画满 N 帧自己退（**别留一个会挂住的示例**）：

```rust
// init 里：frames_left = Some(30)
// redraw 末尾：画满就 Flow::Exit，否则排下一帧（`OnDemand` 下这是自己给自己下单）
let w = self.waker.clone().ok_or("`--frames` 档需要唤醒句柄")?;
w.wake_after(Duration::from_millis(16));
```

```powershell
cargo run -q -p deer-gui --features window --example hello_window -- --frames 30
```

实测（exit 0）：

```text
[hello_window] --frames 已画满 30 帧 ⇒ 退出
[deer-window] 事件循环结束：frames=30 extent=360x220 result=ok
[deer-window] 重绘账本：requests=33 skipped=0 frames=30
[hello_window] 通过 ✅（窗口正常关闭 / 画满指定帧数）
```

---

## 步骤 6　接输入：`interaction::handle` + `dirty` + `wants_redraw`

### 加什么

```rust
impl App for Hello {
    fn input(&mut self, _info: &WindowInfo, ev: &InputEvent) -> Result<Flow, String> {
        // ① 命中 + 状态机。裁剪快照来自**本帧真实绘制列表**（不是第二套规则）。
        let before = self.state.clone();
        let (tree, geo, clip) = { let f = self.frame(); (f.tree, f.geo, f.clip) };
        let events = interaction::handle(&mut self.state, &tree, &geo, clip, ev);

        // ② 应用逻辑：交互层只说「`ok` 被点了」，点了要干什么是**你的事**。
        for e in &events {
            if let UiEvent::Clicked(id) = e { println!("收到 Clicked({id})"); }
        }

        // ③ 脏判据比的是**状态**，不是「有没有收到事件」。
        if self.state != before { self.dirty = true; }

        // ④ `Esc` 的语义在 App 这边（`handle` 只做「清 UI 焦点」）。
        if matches!(ev, InputEvent::KeyDown { key: Key::Escape, .. }) { return Ok(Flow::Exit); }
        Ok(Flow::Continue)
    }

    /// 窗口层在**每条输入派发之后立刻**读一次它：真 ⇒ 请求一帧；假 ⇒ 省下那一帧。
    fn wants_redraw(&self) -> bool { self.dirty }

    /// 省电档：空闲时零重绘（事件循环睡在 `ControlFlow::Wait`）。
    fn redraw_policy(&self) -> RedrawPolicy { RedrawPolicy::OnDemand }
}
```

三条容易搞错的地方：

- **脏判据必须比状态**：`PointerDown` 会改 `pressed` 却可能**一个 `UiEvent` 都不发**
  （点空白处）；只数事件会漏掉它，而且漏得很安静。
- **`wants_redraw` 报的应当与 `redraw` 用的是同一份 `dirty`** ⇒ 两处不可能漂。
- **文本输入走 `InputEvent::TextInput`**，不是 `KeyDown { key: Key::Char('a') }` ——
  状态机**不消费** `Key::Char`（`crates/deer-gui/src/interaction.rs:487`）。

### 应该看到什么

```text
[hello_window] redraw: yes（第 1 帧）…
（之后你不碰鼠标就再没有新的 redraw 行）
[hello_window] redraw: yes（第 2 帧）… hover=Some("ok")     ← 鼠标划过「OK」时出现
```

再打印一行焦点序，顺便看「禁用」的后果：

```text
[hello_window] 焦点树序：["field_1", "ok"]（禁用的 `off` 不在里面）
```

可聚焦 = `Kind::Button` / `Kind::Field` 且**不在禁用子树里**（`interaction::focusables`）。

### 怎么验证

- **不碰鼠标 ⇒ 不画帧**：交互档跑起来后数一数 `redraw:` 的行数。实测（开窗 7 秒、不动鼠标）：
  `redraw: yes` **1 行**（建窗首帧）、`redraw: no` **0 行** —— 不动鼠标时窗口层**根本不会请求帧**，
  所以连 `redraw` 都没被调过（`wants_redraw` 答假 ⇒ 省下的正是这些帧）；
- **划过按钮 ⇒ 画一帧且视觉有变化**（这一条要人动手）：把鼠标移到 `OK` 上，会多一行
  `redraw: yes（第 2 帧）… hover=Some("ok")` —— 按钮被提亮，所以画面确实变了；
- 想要**可自动判定**的版本（数出「跳过了几帧」）：看 `--example counter` 的门槛档，
  它打印 `dirty 账本 : 真的重绘 10 次 / 跳过 1 帧`，或窗口层的
  `[deer-window] 重绘账本：requests=… skipped=… frames=…`。

---

## 步骤 7　换成真实字体度量（`FontMeasure`）

到这一步界面已经能点能打字了，但文字还是**「每个字符同一个等宽方块」的占位格**
（`crates/deer-gpu/src/null.rs::draw_text`）。换真字形只要三行。

### 加什么

```rust
use deer_gui::gpu::measure::find_system_font;
use std::path::Path;

/// 找不到字体 ⇒ **打印降级说明**（文字画成等宽占位块）然后继续 —— 不静默，也不报错。
fn load_font() -> (Option<TextEngine>, f32) {
    let Some(path) = find_system_font() else {
        println!("没找到系统字体 ⇒ **降级**：等宽占位块，交互不受影响");
        return (None, FALLBACK_FONT_SIZE);
    };
    match TextEngine::from_font_file(Path::new(&path), FALLBACK_FONT_SIZE) {
        Ok(e) => {
            let size = e.font_size();       // ← **字号一处定义**：先用引擎，再建 Theme
            println!("字体：{}（{size} px）", path.display());
            (Some(e), size)
        }
        Err(e) => { println!("字体解析失败 ⇒ **降级**：等宽占位块：{e}"); (None, FALLBACK_FONT_SIZE) }
    }
}
```

然后**度量与绘制用同一份 `measure`**：

```rust
// 有引擎 ⇒ FontMeasure（真实 advance）；没有 ⇒ ApproxMeasure（每字符 0.6em 的确定性近似）
let measure_is_real = engine.is_some();
self.theme = Theme { font_size: size, ..Theme::default() };
```

`TextEngine` 会把字号取整（实测 `16.0` → `font_size()` 报 `16`），所以顺序必须是
**先建引擎、再用 `engine.font_size()` 去建 `Theme`**；否则「布局算出来的文字宽度」与
「画出来的文字宽度」会漂。找不到字体时布局与绘制**两侧都**用 `ApproxMeasure`
（同一份近似 ⇒ 仍然自洽）。

### 应该看到什么

```text
[hello_window] 字体：C:\Windows\Fonts\consola.ttf（16 px）
```

> ⚠️ 顺带一条实测：`find_system_font()` 只找 `consola.ttf` / `arial.ttf` / `segoeui.ttf`
> 三款，它们**都没有中文字形** ⇒ 中文标签会被画成占位方块（本示例第一版按钮标签写的是
> 中文，出图里两个按钮就是两个方框）。想要中文就自己喂一个含 CJK 字形的字体：
> `TextEngine::from_font_file(那个字体, size)`。

### 怎么验证（**两条分支都要看到**）

```powershell
# ① 正常档：应当打印真实字体路径 + 字号
cargo run -q -p deer-gui --features window --example hello_window -- --headless

# ② 强制降级档：把 WINDIR 指到一个空目录 ⇒ find_system_font() 返回 None
cmd /c 'set "WINDIR=Z:\no-such-dir" && cargo run -q -p deer-gui --features window --example hello_window -- --headless'
```

实测降级档输出（exit 0，**明确说明跳过什么**，不假装通过）：

```text
[hello_window] 没找到系统字体（找过 %WINDIR%\Fonts 下的 consola.ttf / arial.ttf / segoeui.ttf）⇒ **降级**：文字画成等宽占位块，交互不受影响
[hello_window] 降级说明：没有可用的字体引擎 ⇒ 改用 `render_tree_to_png`（近似度量 + 占位方块字形；像素断言会跳过）
⚠️ 无字库 ⇒ **本档不做像素断言**（占位方块会让文本变化在像素上不可见 ⇒ 假红）；文本级断言已完成
```

---

## 步骤 8　让它**可回归**：离屏自检 + 像素断言

窗口是给人看的，CI 里要的是「不用窗口也能判定」。

### 加什么

给示例加 `--headless` 档：不建窗，走完「树 → 几何 → 绘制列表 → PNG」，再做一条像素断言。

```rust
// ① 出图（门面里的两步入口）
let png = match font_path {
    Some(p) => deer_gui::render_tree_to_png_with_font(&tree, W, H, theme.clone(), p, size)?, // 真字形
    None    => deer_gui::render_tree_to_png(&tree, W, H, theme.clone())?,                    // 占位块
};
std::fs::write("target/hello_window.png", &png)?;

// ② 像素断言：喂一串输入得到「第二帧」，差异**必须全落在输入框矩形内，框外为 0**
let mut st = UiState::default();
for ev in [moved, down, up, InputEvent::TextInput { text: "hi".into() }] {
    interaction::handle(&mut st, &tree, &geo, clip.clone(), &ev);   // 与窗口路径**同一个入口**
}
let f1 = /* 用 st 重建一帧 */;
let (inside, outside) = pixel_diff_split(&f0.list, &f1.list, extent, field_rect, &mut pixels)?;
assert!(inside > 0, "框内必须有差异（文本真的换了字形）");
assert!(outside == 0, "框外不许有差异（变化视觉溢出了）");
```

**像素断言的前置必须显式写出来**：渲染要用 `CpuRenderer::with_text(engine)`（真字形）。
无字库的 `CpuRenderer::new()` 只会给出一条**假红**（理由见下面「实测坑 ①」）。

### 应该看到什么

```text
[hello_window] 出图：target\hello_window.png（317108 字节）
[hello_window] 喂 PointerMoved { x: 85.0, y: 51.0 } ⇒ [HoverChanged(Some("field_1"))]
[hello_window] 喂 PointerDown { button: Left, x: 85.0, y: 51.0 } ⇒ [FocusChanged(Some("field_1"))]
[hello_window] 喂 PointerUp { button: Left, x: 85.0, y: 51.0 } ⇒ [Clicked("field_1")]
[hello_window] 喂 TextInput { text: "hi" } ⇒ [TextChanged { id: "field_1", value: "hi" }]
[hello_window] 第二帧画出来的文本：["deer-gui hello window", "hi", "OK", "Off"]
像素断言        : 输入框矩形 RectI { x: 12, y: 40, w: 146, h: 22 } 内 1355 像素；**框外 0** 像素
离屏自检 ✅：树 → 几何 → 列表 → PNG 打通；打字走完了「输入 → 状态 → 画出来」这条闭环
```

### 三条门禁命令（**本仓库的验收线**）

```powershell
# ① 全套断言（本次实测基线：**475 passed / 0 failed**；数字会随里程碑涨，别写死）
cargo test --workspace

# ② clippy 干净（本项目 `#![deny(clippy::all)]`；本次实测 0 warning）
cargo clippy -p deer-gui --features window --all-targets

# ③ 文档里提到的示例必须真实存在（本次实测 5 passed）
cargo test -p deer-gui --test docs_consistency

# ④ 上面那套样板（步骤 1–8 手写的东西）**已经变成 API**：见 `docs/features/testing.md`
cargo run -q -p deer-gui --features testing --example testkit_demo
```

> **示例与 `cargo test` 不要并发跑**（实测：并发不会报错，而是**串行**等构建锁 ——
> 别指望省时间；理由另见 `GETTING-STARTED-UI.md` 第 8 节第 4 条）。

想要更硬的可回归判据（脚本重放 + 逐字段终态断言 + dirty 账本），照
[`crates/deer-gui/examples/counter.rs`](../crates/deer-gui/examples/counter.rs) 抄
—— 它把「点两次 `+` ⇒ 计数显示为 2」断在**绘制列表**与**像素**两个层次上。

> 步骤 1–8 手写的那套样板（几何 → 列表 → 裁剪快照 → 像素 → 前置断言）已经抽成库里的
> **testkit**：`Harness::frame()` 每次都会断言前置条件（`node_hint > 0`、裁剪快照非空、
> 用到的 id 在快照里），`move @id` 的坐标由布局算出来，像素差异的期望矩形**自动**包含
> 「这一帧真的变了的东西」，失败信息里自带**可复制的复现命令**。
> 想省掉抄样板：见 [`features/testing.md`](features/testing.md)（最小示例 + 完整 API + 常见坑）。

---

## 三个实测坑（都在这台机器上跑过、数字照抄）

### ① 无字库渲染器的**占位方块**会让文本变化**像素不可见** ⇒ 断言假红

`CpuRenderer::new()`（无字库）给**每个字符画同一个等宽方块**。于是：

```text
无字库：「AAAA」vs「BBBB」（等长、内容不同）⇒ 差异 0 像素（框外 0）
无字库：「AAAA」vs「AA」（不等长）      ⇒ 差异 132 像素（框外 0）
```

**等长**的两个字符串在像素上**逐个相同**，而绘制列表里的文本确实变了 ⇒ 像素断言
`inside > 0` **必然假红**。`counter.rs` 里也有同一条实测记录（`count = 0` / `count = 1`
矩形内差异 **0** 像素）。

⇒ **文本类像素断言必须用 `CpuRenderer::with_text(engine)`（真字形）**；拿不到字体就
**明确降级**（本示例打印「本档不做像素断言」，而不是假装通过）。
（本示例的占位文本与打进去的字长度不同，所以那一条**恰好**还能看见差异 ——
但**别指望它**，长度一变就没了。）

### ② 像素断言必须列全「**这一帧同时变的东西**」

实测（`--example counter -- --headless`，照抄）：

```text
像素断言 count 0→1   : 差异 count 矩形内 68 像素（框 RectI { x: 12, y: 72, w: 87, h: 18 }）；同时变了的按钮 ["plus"] 内 [393]；**框外 0** 像素
```

「框外 0」是**列全之后**的结果。如果只列 `count` 矩形，那 **393** 个差异就会跑到框外 ⇒
看起来像「视觉溢出了」，其实是**断言漏列了**：鼠标按在 `+` 上，**按钮自己的
hover/pressed 视觉也变了**。

⇒ 判据是「框外 = 0」（它是真牙齿），但**前提**是把参与变化的东西都写成矩形。
本示例只改了输入框一个控件（hover/focus 视觉与文本内容**都在输入框矩形内**），
所以它列一个矩形就够；换成点按钮就得多列按钮矩形。

### ③ 门槛变量的写法：`set "VAR=1" &&` —— 写错会**静默**不生效

两件事叠在一起（都在本机实测）：

**a. `cmd` 的空格会进变量值**：

```text
cmd /c 'set X=1 && set X'      →  X=1␠   （值带一个**尾空格**）
cmd /c 'set "X=1" && set X'    →  X=1
cmd /c 'set X=1&& set X'       →  X=1
```

严格 `== "1"` 的判定会把「已经设了」判成「没设」⇒ 用例**静默跳过却报 pass**。
所以本仓库统一用 `deer_gui::env_gate::flag(name)` / `flag_default_true(name)`（它们先
`trim()` 再判），自己写的判定别忘了这一条。

**b. 在 PowerShell 里，外层引号会把内层 `set "X=…"` 的引号吃掉**（这条更阴）：

```powershell
# ❌ 变量**根本没设上**，门槛默认值照旧生效 —— 而命令仍然 exit 0
cmd /c "set "DEER_HELLO_PIXELS=0" && cargo run -q -p deer-gui --features window --example hello_window -- --headless"
# 实测：像素断言**照跑**（你以为关掉了，其实没关）

# ✅ 外层用**单引号**，让 cmd 自己看到那对双引号
cmd /c 'set "DEER_HELLO_PIXELS=0" && cargo run -q -p deer-gui --features window --example hello_window -- --headless'
# 实测：[hello_window] DEER_HELLO_PIXELS 被设为「关」⇒ 按要求跳过像素断言
```

⇒ 门槛命令一律写成 `cmd /c 'set "VAR=VALUE" && …'`（单引号在外、双引号在内）。
**判定门槛时永远别只信退出码** —— 打印一行「实际生效什么」，让它自证。

---

## 卡住时按这个顺序查

| 现象 | 先看哪一步 | 最可能的原因 |
|---|---|---|
| 程序 exit 0，但**什么都没有** | 步骤 5 | 没实现 `App` / 没 `run()` —— 只有树是不会自己变成窗口的 |
| 有窗口，但界面是**空白** | 步骤 2 | `build()` 调早了/调了两次 ⇒ 树是空的（子节点数断言会直接喊疼） |
| 窗口画出来了，但**点不动** | 步骤 6 | `App::input` 没实现（默认什么都不做）；或 `wants_redraw` 恒假 |
| 点得动，但**按下去界面不亮** | 步骤 6 | 脏判据写成「有没有 `UiEvent`」而不是「状态变没变」 |
| 输入框**打不进字** | 步骤 6 | 用了 `KeyDown { key: Char(..) }`；文本一律走 `InputEvent::TextInput` |
| 输入框**永远显示占位提示** | 步骤 4 | `FieldText::Label`（或没用 `with_texts(…, &state.texts)`） |
| 中文标签画成**方块** | 步骤 7 | `find_system_font()` 只找三款无 CJK 字形的字体 |
| 文本断言**总是假红** | 步骤 8 / 实测坑 ① | 用了无字库的 `CpuRenderer::new()` |
| 「框外」总是 **393 像素** | 步骤 8 / 实测坑 ② | 断言漏列了同时变色的按钮矩形 |
| 门槛设了却像没设 | 实测坑 ③ | `set` 的尾空格，或 PowerShell 吃掉了内层引号 |

---

## 相关文档

- **样板代码**：[`crates/deer-gui/examples/hello_window.rs`](../crates/deer-gui/examples/hello_window.rs)
  （注释里逐段标了【步骤 N】）
- **完整交互界面教程**：[`GETTING-STARTED-UI.md`](GETTING-STARTED-UI.md)
  （主示例 `--example counter`：建树 / 布局 / 输入 / 脚本重放 / 像素断言 / 12 条常见坑）
- **从没写过 Rust**：[`GETTING-STARTED.md`](GETTING-STARTED.md)
- **逐功能指南**：[`features/`](features/)（输入与焦点、窗口与唤醒面、布局、绘制与像素）
- **能做到什么 / 做不到什么**：[`../FEATURES.md`](../FEATURES.md)（唯一真相）
