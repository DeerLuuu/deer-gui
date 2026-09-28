//! **hello_window —— 从「只有一棵界面树」到「有窗口显示」的最小样板**。
//!
//! 配套文档：[`docs/GETTING-STARTED-UI-STEPS.md`](../../../docs/GETTING-STARTED-UI-STEPS.md)
//! —— 它是**分步迁移指南**（加什么 / 应该看到什么 / 怎么验证），本文件里的中文注释
//! 逐段标了 **【步骤 N】**，两边一一对应。
//!
//! 界面只有三样（这是「有一棵树」最典型的形状，也是使用者最常见的起点）：
//!
//! ```text
//! Column "app"
//! ├─ Text  "title"  ← 标题文本
//! ├─ Field "input"  ← 输入框
//! └─ Row   "bar"
//!    ├─ Button "ok"   （可用）
//!    └─ Button "off"  （props.disabled = true）
//! ```
//!
//! # 这个示例专门示范的一件错事
//!
//! **`build()` 只能调一次，而且在加完子节点之后。** 两个常见写法都是错的：
//!
//! ```ignore
//! let tree = app.build();     // ❌ 早了一步：此后加的节点都不在 tree 里
//! app.text("标题");
//! let tree = app.build();     // ❌ 又调了一次：拿到的还是那一刻的快照
//! ```
//!
//! `Builder::build(&self) -> Node` 是**克隆当前树**（`crates/deer-layout/src/builder.rs:182`），
//! 不是「定稿」。所以早调只会拿到一棵**缺子节点的空树** —— 而布局照样会算出几何、
//! 渲染器照样会画出一张图，于是「没有窗口/没有内容」这个现象**不报错**，只是画面上什么都没有。
//! 本示例的 [`assert_preconditions`] 因此把 `tree.children.len()` 也**显式断言**了。
//!
//! # 四档跑法
//!
//! ```sh
//! # ① 真窗口、纯交互（默认）：点确定/输入框打字/Tab 换焦点/Esc 退出
//! cargo run -q -p deer-gui --features window --example hello_window
//!
//! # ② 真窗口、**门槛档**：画满 30 帧自己退（证明建窗 + 交换链 + 呈现 + 退出这条路能跑）
//! cargo run -q -p deer-gui --features window --example hello_window -- --frames 30
//!
//! # ③ 离屏自检（不建窗）：树 → 几何 → 绘制列表 → PNG + 像素断言，exit 0 = 全过
//! cargo run -q -p deer-gui --features window --example hello_window -- --headless
//!
//! # ④ 关掉离屏档的像素断言（门槛变量的写法见文档第 8 节，「实测坑」第 3 条）
//! cmd /c "set "DEER_HELLO_PIXELS=0" && cargo run -q -p deer-gui --features window --example hello_window -- --headless"
//! ```
//!
//! ⚠️ ① 是给人用的窗口，**故意不自己退**；要自动化就用 ②/③。**不许留一个会挂住的示例**
//! 也是本项目示例的纪律之一。

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use deer_gui::env_gate;
use deer_gui::gpu::interact::{FieldText, InteractState, InteractiveRenderer};
use deer_gui::gpu::measure::find_system_font;
use deer_gui::gpu::null::CpuRenderer;
use deer_gui::interaction::{self, ClipSnapshot, InputEvent, Key, PointerButton, UiEvent, UiState};
use deer_gui::layout::layout::{self, Geometry};
use deer_gui::prelude::*;
use deer_gui::vk::windowed::{FrameOutcome, WindowedRenderer};
use deer_gui::window::{App, Flow, RedrawPolicy, WindowConfig, WindowInfo, Waker, run};

// ---------------------------------------------------------------------------
// 【步骤 1】起点：常量与「我已有的一棵树」
// ---------------------------------------------------------------------------

/// 清屏色：界面没盖住的地方就是这个颜色（离屏与上屏用**同一个**值）。
const CLEAR: Color = Color::rgb(0x08, 0x09, 0x0c);

/// 窗口尺寸（**物理**像素）。离屏档用同一个尺寸，这样两边出的图可以直接比。
const WIDTH: u32 = 360;
const HEIGHT: u32 = 220;

/// 找不到系统字体时退回的字号 —— 让「布局度量」与「画出来的字号」用**同一个**数字。
const FALLBACK_FONT_SIZE: f32 = 16.0;

/// 标题文本（它同时是断言里用的那个字符串，只写一遍）。
const TITLE: &str = "deer-gui hello window";

/// 输入框的占位文本（`FieldText::Content` 下，没打过字时才显示它）。
const PLACEHOLDER: &str = "type here";

/// 离屏档像素断言的门槛变量（默认开，`=0`/`false` 关掉）。
const PIXELS_ENV: &str = "DEER_HELLO_PIXELS";

// ---------------------------------------------------------------------------
// 【步骤 2】`build()` 只调一次，而且在加完子节点之后
// ---------------------------------------------------------------------------

/// 建树拿到的四样东西：树 + 几个**要在断言里按 id 定位**的节点 id。
///
/// 为什么要留下 id：`Builder::text()` / `Builder::field()` 的 id 是 `IdGen` 自动生成的
/// （`text_1` / `field_1`），**不是**你写的那串字。想让后面的自检与脚本按名字找节点，
/// 就得把返回的 id 存下来（容器的 id 由 `container_opts` 的第二个参数显式给定）。
struct Widgets {
    tree: Node,
    title: String,
    field: String,
    ok: String,
    off: String,
}

/// **本程序里唯一一次 `build()`**（就在这个函数最后一行）。
///
/// 【步骤 2】的要点全在这里：`Builder` 一路把子节点加完 ⇒ **最后**才 `build()`。
fn build_widgets() -> Widgets {
    let mut app = Builder::new(Kind::Column, "app").padding(12.0).gap(10.0);
    let title = app.text(TITLE); // 标题文本
    let field = app.field(PLACEHOLDER); // 输入框（占位文本）
    let mut ok = String::new();
    let mut off = String::new();
    // 两个按钮放在一行里；id 用闭包显式改名（`Builder::button_opts` 会把 n.id 当最终 id）。
    //
    // ⚠️ 标签刻意用 **ASCII**：`find_system_font()` 只找 consola / arial / segoeui 三款，
    // 它们**没有中文字形** ⇒ 中文标签会被画成占位方块（实测：出图里两个按钮都是两个方框）。
    // 想要中文标签就自己喂一个含 CJK 字形的字体：`TextEngine::from_font_file(那个字体, size)`。
    app.container_opts(Kind::Row, "bar", L::new().w(300.0).gap(8.0).to_props(), |r| {
        ok = r.button_opts("OK", |n| n.id = "ok".into());
        off = r.button_opts("Off", |n| {
            n.id = "off".into();
            n.props.disabled = true; // ← 第 2 个按钮：禁用
        });
    });
    let tree = app.build(); // ✅ 唯一一次，且在加完子节点之后
    Widgets {
        tree,
        title,
        field,
        ok,
        off,
    }
}

// ---------------------------------------------------------------------------
// 【步骤 3、4】一帧 = 树 → 几何 → 绘制列表 → 裁剪快照
// ---------------------------------------------------------------------------

/// 一帧的四样纯数据（都能留到断言里用）。
struct Frame {
    tree: Node,
    geo: Geometry,
    list: DrawList,
    clip: ClipSnapshot,
}

/// 【步骤 3】几何 + 【步骤 4】绘制列表与裁剪快照。
///
/// - **几何每帧重算**：它很便宜，而且保证「画的东西」与「命中的东西」同源；
/// - **度量必须两处一致**：布局与绘制用**同一份** `measure` —— 否则「布局算出来的文字宽度」
///   与「画出来的文字宽度」会漂；
/// - **[`InteractiveRenderer`] 才发 `NodeHint`**（`DefaultRenderer` 不发）⇒ 裁剪快照才有内容。
fn build_frame(
    tree: &Node,
    theme: &Theme,
    extent: Extent,
    state: &UiState,
    measure: &impl Measure,
) -> Frame {
    let geo = layout::layout(
        tree,
        Rect::new(0.0, 0.0, extent.width as f32, extent.height as f32),
        TextStyle {
            font_size: theme.font_size,
            line_height: theme.line_height,
        },
        measure,
    );
    // `UiState` 是交互层的真相；渲染层只认它的**只读子集** `InteractState`。
    let interact = InteractState {
        hover: state.hover.clone(),
        focus: state.focus.clone(),
        pressed: state.pressed.clone(),
    };
    // `FieldText::Content` ⇒ 输入框画的是 `state.texts["input"]`（没打过字时退回占位 label）。
    let list = InteractiveRenderer::with_texts(
        theme.clone(),
        measure,
        &interact,
        FieldText::Content,
        &state.texts,
    )
    .build(tree, &geo);
    let clip = ClipSnapshot::from_draw_list(&list, tree, &geo);
    Frame {
        tree: tree.clone(),
        geo,
        list,
        clip,
    }
}

/// 【步骤 2、3、4 的自检】—— 三个前置条件不成立时**明确报错**，不往下走。
///
/// 为什么必须显式断言（而不是「反正画出来能看」）：
///
/// 1. `tree.children.len()`：`build()` 早调/多调只会给你一棵**空树**，而空树也能一路画到
///    「一张除了清屏色什么都没有的图」—— 静默；
/// 2. `geo` 里有没有 `bar`：布局没接上（或树是空的）时同样不报错，只是没有几何；
/// 3. **`node_hint` 与裁剪快照非空**：裁剪语义是 `allows()` 对**未知 id 放行**（fail-open，
///    `crates/deer-gui/src/interaction.rs:175`）⇒ 快照为空时命中会**静默退化成「全不裁剪」**：
///    界面照样能用，但「被裁掉的控件点不中」这条护栏已经不存在了。用不发 `NodeHint` 的
///    渲染器（例如 `DefaultRenderer`）就会走到这个坑里，**而且一行报错都没有**。
fn assert_preconditions(f: &Frame, field_id: &str) -> Result<(), String> {
    let n = f.tree.children.len();
    if n != 3 {
        return Err(format!(
            "界面树根 `{}` 应当有 3 个子节点（标题 / 输入框 / 按钮行），实际 {n} —— \
             十有八九是 `build()` 调早了或调了两次（`Builder::build` 是克隆当前树，不是定稿）",
            f.tree.id
        ));
    }
    if !f.geo.contains_key("bar") {
        return Err("布局没有给 `bar` 几何 ⇒ 几何那一步没接上（见步骤 3）".to_string());
    }
    let hints = f.list.counts().node_hint;
    if hints == 0 || f.clip.is_empty() {
        return Err(format!(
            "前置条件不成立：这一帧的绘制列表没有节点提示（node_hint={hints}，裁剪快照 len={}）—— \
             命中会退化成「全不裁剪」而**不报错**。检查是不是用了不发 NodeHint 的渲染器\
             （`DefaultRenderer` 不发；`InteractiveRenderer` 才发）",
            f.clip.len()
        ));
    }
    for id in ["ok", field_id] {
        if !f.clip.is_known(id) {
            return Err(format!(
                "前置条件不成立：裁剪快照里没有 `{id}`（`allows()` 对未知 id 放行）—— \
                 那会让「被裁掉就不命中」这条护栏静默失效"
            ));
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// 【步骤 7】真实字体度量（找不到就明确降级）
// ---------------------------------------------------------------------------

/// 加载结果：引擎（没有就是 `None`）、字号、字体文件路径（离屏出图还要用它）。
struct LoadedFont {
    engine: Option<TextEngine>,
    size: f32,
    path: Option<PathBuf>,
}

/// 【步骤 7】：`find_system_font` + `TextEngine::from_font_file`。
///
/// **字号一处定义**：`theme.font_size` / `TextStyle.font_size` / 引擎字号必须是同一个数字。
/// `TextEngine` 会把字号取整（实测 `16.0` → `font_size()` 报 `16`），所以顺序是
/// **先建引擎、再用 `engine.font_size()` 去建 `Theme`**。
///
/// 找不到字体不是一个错误：打印**降级说明**（文字会画成「每个字符同一个等宽方块」的占位格）
/// 然后继续 —— 交互链完全不受影响。**不静默**是这里的重点。
fn load_font() -> LoadedFont {
    let Some(path) = find_system_font() else {
        println!(
            "[hello_window] 没找到系统字体（找过 %WINDIR%\\Fonts 下的 \
             consola.ttf / arial.ttf / segoeui.ttf）⇒ **降级**：文字画成等宽占位块，交互不受影响"
        );
        return LoadedFont {
            engine: None,
            size: FALLBACK_FONT_SIZE,
            path: None,
        };
    };
    match TextEngine::from_font_file(Path::new(&path), FALLBACK_FONT_SIZE) {
        Ok(e) => {
            let size = e.font_size();
            println!("[hello_window] 字体：{}（{} px）", path.display(), size);
            LoadedFont {
                engine: Some(e),
                size,
                path: Some(path),
            }
        }
        Err(e) => {
            println!(
                "[hello_window] 字体解析失败 ⇒ **降级**：等宽占位块（交互不受影响）：{e}"
            );
            LoadedFont {
                engine: None,
                size: FALLBACK_FONT_SIZE,
                path: None,
            }
        }
    }
}

// ---------------------------------------------------------------------------
// 【步骤 5、6】窗口壳 + 输入
// ---------------------------------------------------------------------------

/// 应用状态。窗口档与离屏档共用同一套「树 + 几何 + 状态 → 绘制列表」。
struct Hello {
    tree: Node,
    /// 输入框的 id。它由 `IdGen` 自动生成（实测 `field_1`）**不是**你写的那串字，
    /// 所以要**存下来**才能按 id 定位（容器/按钮那边我们是用闭包显式改成 `bar`/`ok`/`off` 的）。
    field_id: String,
    theme: Theme,
    extent: Extent,
    state: UiState,
    /// 脏位：【步骤 6】`redraw()` 只在它置上时才真的碰 GPU。
    dirty: bool,
    /// 已真的画过几帧。
    drawn: u64,
    /// `--frames N` 档：还要画几帧（`None` = 纯交互，一直开着）。
    frames_left: Option<u32>,
    engine: Option<TextEngine>,
    renderer: Option<WindowedRenderer>,
    waker: Option<Waker>,
}

impl Hello {
    /// 当前这一帧。有字体引擎就用**真实度量**（`FontMeasure`），否则用确定性近似
    /// （`ApproxMeasure`，每字符 `0.6em`）—— 两条路径都真，只是精度不同。
    ///
    /// 写成两支而不是 `Box<dyn Measure>`：`FontMeasure` 借用了引擎，装进 trait object
    /// 会逼出生命周期体操，而这里只需要「两条具体路径」二选一。
    fn frame(&self) -> Frame {
        match &self.engine {
            Some(e) => build_frame(
                &self.tree,
                &self.theme,
                self.extent,
                &self.state,
                &e.measure(),
            ),
            None => build_frame(
                &self.tree,
                &self.theme,
                self.extent,
                &self.state,
                &ApproxMeasure,
            ),
        }
    }
}

impl App for Hello {
    /// 【步骤 5】建窗后**调一次**：字体 → 渲染器 → **前置断言**。
    fn init(&mut self, info: &WindowInfo) -> Result<(), String> {
        self.extent = Extent {
            width: info.extent.width.max(1),
            height: info.extent.height.max(1),
        };
        println!(
            "[hello_window] 窗口 {}×{}（物理像素）",
            self.extent.width, self.extent.height
        );

        // ① 字体（【步骤 7】）。
        let font = load_font();
        self.theme = Theme {
            font_size: font.size,
            ..Theme::default()
        };
        self.engine = font.engine;

        // ② 渲染器：窗口 + 交换链 + 呈现（`0` = 第 0 个 Vulkan 适配器）。
        let r = WindowedRenderer::new(0, info.raw, self.extent, CLEAR)
            .map_err(|e| format!("创建窗口渲染器失败：{e}"))?;
        println!("[hello_window] 适配器：{}", r.adapter().name);
        self.renderer = Some(r);

        // ③ 前置断言：这一帧的列表必须发得出 `NodeHint`，且裁剪快照非空
        //    （为 0 会让命中**静默**退化成「全不裁剪」，见 [`assert_preconditions`]）。
        let f = self.frame();
        assert_preconditions(&f, &self.field_id)?;
        println!(
            "[hello_window] 绘制列表：{} 条命令（NodeHint {} 条）；裁剪快照 {} 个节点",
            f.list.len(),
            f.list.counts().node_hint,
            f.clip.len()
        );
        // 【步骤 6】焦点序：禁用子树整棵跳过 ⇒ `off` 不在里面（这就是「禁用」的后果之一）。
        println!(
            "[hello_window] 焦点树序：{:?}（禁用的 `off` 不在里面）",
            interaction::focusables(&f.tree)
        );
        println!(
            "[hello_window] 树上的 id/标签：{:?}",
            ids_of(&f.tree)
        );

        // 首帧必然要画（窗口刚建好）。
        self.dirty = true;
        Ok(())
    }

    /// 尺寸变化（含 DPI 变化）：重建交换链 + 置脏。
    fn resized(&mut self, width: u32, height: u32) -> Result<(), String> {
        let e = Extent {
            width: width.max(1),
            height: height.max(1),
        };
        if let Some(r) = self.renderer.as_mut() {
            r.resize(e)
                .map_err(|err| format!("重建交换链失败：{err}"))?;
        }
        self.extent = e;
        self.dirty = true;
        Ok(())
    }

    /// 【步骤 6】输入：`interaction::handle` 改状态 ⇒ 应用逻辑 ⇒ **状态真的变了才置脏**。
    fn input(&mut self, _info: &WindowInfo, ev: &InputEvent) -> Result<Flow, String> {
        // ① 命中 + 状态机。裁剪快照来自**本帧真实绘制列表**（不是第二套规则）。
        let before = self.state.clone();
        let (tree, geo, clip) = {
            let f = self.frame();
            (f.tree, f.geo, f.clip)
        };
        let events = interaction::handle(&mut self.state, &tree, &geo, clip, ev);

        // ② 应用逻辑：交互层只说「`ok` 被点了」，点了要干什么是你的事。
        for e in &events {
            if let UiEvent::Clicked(id) = e {
                println!("[hello_window] 收到 Clicked({id}) ⇒ 你的应用逻辑在这里（本示例只打印）");
            }
        }

        // ③ 脏判据比的是**状态**，不是「有没有收到事件」：
        //    `PointerDown` 会改 `pressed` 却可能一个 `UiEvent` 都不发（点空白处），
        //    只看事件会漏掉它，而且漏得很安静。
        if self.state != before {
            self.dirty = true;
        }

        // ④ `Esc` 的语义在 App 这边：`handle` 只做「清 UI 焦点」。
        if matches!(ev, InputEvent::KeyDown { key: Key::Escape, .. }) {
            println!("[hello_window] 窗口按 Esc ⇒ 退出");
            return Ok(Flow::Exit);
        }
        Ok(Flow::Continue)
    }

    /// 窗口层在**每条输入派发之后立刻**读一次它：真 ⇒ 请求一帧；假 ⇒ 省下那一帧。
    fn wants_redraw(&self) -> bool {
        self.dirty
    }

    /// 【步骤 6】省电档：空闲时零重绘（事件循环睡在 `ControlFlow::Wait`）。
    ///
    /// `--frames` 档也用它：推进靠 `Waker::wake_after` 排的 deadline（`deer-window` 的语义是
    /// 「App 自己下的单 ⇒ 到点一律画一帧」），不需要连续重绘。
    fn redraw_policy(&self) -> RedrawPolicy {
        RedrawPolicy::OnDemand
    }

    fn wake_handle(&mut self, waker: Waker) {
        self.waker = Some(waker);
    }

    /// 画一帧（**只在 `dirty` 时真的碰 GPU**）。
    fn redraw(&mut self) -> Result<Flow, String> {
        // `--frames` 档的每一帧都是我们自己排的 deadline（不是输入改的状态），
        // 所以它**不看** `dirty`；交互档才走「状态没变就不碰 GPU」这条省电路径。
        if !self.dirty && self.frames_left.is_none() {
            println!(
                "[hello_window] redraw: no（状态没变，跳过这一帧；已画 {} 帧）",
                self.drawn
            );
            return Ok(Flow::Continue);
        }
        self.dirty = false;
        self.drawn += 1;

        let f = self.frame();
        println!(
            "[hello_window] redraw: yes（第 {} 帧）命令 {} 条 hover={:?} focus={:?} texts={:?}",
            self.drawn,
            f.list.len(),
            self.state.hover,
            self.state.focus,
            self.state.texts
        );

        // 引擎要 `&mut`（字符串里出现新字形时要就地光栅化并入图集）⇒ 从字段里临时取出。
        let mut engine = self.engine.take();
        let r = self.renderer.as_mut().ok_or("还没有窗口渲染器")?;
        let outcome = r.draw_and_present(&f.list, engine.as_mut());
        let skipped = r.ui_text_skipped();
        self.engine = engine;
        match outcome {
            Ok(FrameOutcome::Presented) => {}
            Ok(FrameOutcome::OutOfDate) => {
                // 交换链过期是**正常路径**：重建后重试，绝不能当成功。
                let e = self.extent;
                if let Some(r) = self.renderer.as_mut() {
                    r.resize(e)
                        .map_err(|err| format!("交换链过期后重建失败：{err}"))?;
                }
                self.dirty = true;
            }
            Err(e) => return Err(format!("呈现失败：{e}")),
        }
        println!("[hello_window] 本帧被跳过的字形：{skipped} 个（累计）");

        // 门槛档：再排一帧，画满就退（`--frames N`）。
        let mut done = false;
        if let Some(left) = self.frames_left.as_mut() {
            *left -= 1;
            done = *left == 0;
        }
        if done {
            println!("[hello_window] --frames 已画满 {} 帧 ⇒ 退出", self.drawn);
            return Ok(Flow::Exit);
        }
        if self.frames_left.is_some() {
            let w = self
                .waker
                .clone()
                .ok_or("`--frames` 档需要唤醒句柄（`App::wake_handle` 没被调用？）")?;
            w.wake_after(Duration::from_millis(16));
        }
        Ok(Flow::Continue)
    }
}

/// 按 id 找节点（前序，确定性）。**这不是第二套命中测试** —— 只用来读树内容。
fn node_by_id<'a>(n: &'a Node, id: &str) -> Option<&'a Node> {
    if n.id == id {
        return Some(n);
    }
    n.children.iter().find_map(|c| node_by_id(c, id))
}

/// 打印用：把每个叶子节点的 `id=标签` 列出来（确认 `Builder` 到底给了什么 id）。
fn ids_of(tree: &Node) -> Vec<String> {
    let mut out = Vec::new();
    fn walk(n: &Node, out: &mut Vec<String>) {
        match n.props.label.as_deref() {
            Some(l) => out.push(format!("{}={l:?}", n.id)),
            None => out.push(n.id.clone()),
        }
        for c in &n.children {
            walk(c, out);
        }
    }
    walk(tree, &mut out);
    out
}

// ---------------------------------------------------------------------------
// 【步骤 8】离屏自检：不需要窗口的自动化验证
// ---------------------------------------------------------------------------

/// 【步骤 8】`--headless`：树 → 几何 → 绘制列表 → PNG，外加一条**像素级**判据。
///
/// 判据（与 `counter.rs` 同一套模板）：模拟「点输入框 + 打字」得到的第二帧，
/// 它的像素差异**必须全部落在输入框矩形内，框外为 0**。
///
/// 为什么 `--headless` 也值得有像素断言：文本类断言只证明「绘制列表里的字符串变了」，
/// 不证明「画出来真的不一样」。而后者有一个**很尖的前置**（见下面的降级分支）：
/// 无字库的 `CpuRenderer::new()` 给**每个字符画同一个等宽方块** ⇒ 「占位文本」与「打的字」
/// 在像素上可能**逐个相同**，于是断言**假红**。所以这里必须先有字体引擎才做像素断言。
fn headless() -> Result<(), String> {
    println!("[hello_window] --headless：离屏自检（不建窗，树 → 几何 → 绘制列表 → PNG）");
    let font = load_font();
    let theme = Theme {
        font_size: font.size,
        ..Theme::default()
    };
    let extent = Extent {
        width: WIDTH,
        height: HEIGHT,
    };

    // 【步骤 2】自检：树到底有几个子节点（`build()` 早调就会在这里红）。
    let w = build_widgets();
    println!(
        "[hello_window] 树：root={} 子节点 {} 个 ⇒ {:?}",
        w.tree.id,
        w.tree.children.len(),
        ids_of(&w.tree)
    );
    for id in [&w.title, &w.field, &w.ok, &w.off] {
        if node_by_id(&w.tree, id).is_none() {
            return Err(format!("树里没有节点 `{id}`（`build()` 调早了？）"));
        }
    }

    // 【步骤 3、4】几何 + 绘制列表 + 裁剪快照，并断言三个前置。
    let state0 = UiState::default();
    let f0 = match &font.engine {
        Some(e) => build_frame(&w.tree, &theme, extent, &state0, &e.measure()),
        None => build_frame(&w.tree, &theme, extent, &state0, &ApproxMeasure),
    };
    assert_preconditions(&f0, &w.field)?;
    println!(
        "[hello_window] 几何 `bar` = {:?}",
        f0.geo
            .get("bar")
            .map(|r| format!("({:.1},{:.1} {:.1}×{:.1})", r.x, r.y, r.w, r.h))
    );
    println!(
        "[hello_window] 绘制列表：{} 条命令（NodeHint {} 条）；裁剪快照 {} 个节点",
        f0.list.len(),
        f0.list.counts().node_hint,
        f0.clip.len()
    );
    println!(
        "[hello_window] 焦点树序：{:?}（禁用的 `off` 不在里面）",
        interaction::focusables(&f0.tree)
    );

    // 【步骤 8】出图：有字体就**真字形**，没有就明确降级到占位字形（并打印说明）。
    let out = Path::new("target").join("hello_window.png");
    let png = match (&font.path, font.engine.is_some()) {
        (Some(p), true) => deer_gui::render_tree_to_png_with_font(
            &w.tree,
            WIDTH,
            HEIGHT,
            theme.clone(),
            p,
            font.size,
        )?,
        _ => {
            println!(
                "[hello_window] 降级说明：没有可用的字体引擎 ⇒ 改用 `render_tree_to_png`\
                 （近似度量 + 占位方块字形；像素断言会跳过）"
            );
            deer_gui::render_tree_to_png(&w.tree, WIDTH, HEIGHT, theme.clone())?
        }
    };
    std::fs::write(&out, &png).map_err(|e| format!("写 {} 失败：{e}", out.display()))?;
    println!("[hello_window] 出图：{}（{} 字节）", out.display(), png.len());

    // ---- 像素断言的前置（**显式**，不静默跳过） ----
    if font.engine.is_none() {
        println!(
            "⚠️ 无字库 ⇒ **本档不做像素断言**（占位方块会让文本变化在像素上不可见 ⇒ 假红）；\
             文本级断言已完成"
        );
        return Ok(());
    }
    if !env_gate::flag_default_true(PIXELS_ENV) {
        println!("[hello_window] {PIXELS_ENV} 被设为「关」⇒ 按要求跳过像素断言");
        return Ok(());
    }

    // ---- 第二帧：模拟「点输入框 + 打字」（走库里的 `interaction::handle`，与窗口路径同一入口） ----
    let rect = f0
        .geo
        .get(&w.field)
        .map(|r| RectI::new(r.x as i32, r.y as i32, r.w as i32, r.h as i32))
        .ok_or_else(|| format!("布局没有给输入框 `{}` 几何", w.field))?;
    let (cx, cy) = (
        rect.x as f32 + rect.w as f32 / 2.0,
        rect.y as f32 + rect.h as f32 / 2.0,
    );
    let mut state = state0.clone();
    for ev in [
        InputEvent::PointerMoved { x: cx, y: cy },
        InputEvent::PointerDown {
            button: PointerButton::Left,
            x: cx,
            y: cy,
        },
        InputEvent::PointerUp {
            button: PointerButton::Left,
            x: cx,
            y: cy,
        },
        InputEvent::TextInput {
            text: "hi".to_string(),
        },
    ] {
        let events = interaction::handle(&mut state, &f0.tree, &f0.geo, f0.clip.clone(), &ev);
        println!("[hello_window] 喂 {ev:?} ⇒ {events:?}");
    }
    let f1 = build_frame(
        &w.tree,
        &theme,
        extent,
        &state,
        &font.engine.as_ref().expect("上面已确认有引擎").measure(),
    );

    // 文本级证据：打进去的字**真的到了绘制列表**。
    let typed: Vec<&str> = f1
        .list
        .cmds
        .iter()
        .filter_map(|c| match c {
            DrawCmd::Text { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    println!("[hello_window] 第二帧画出来的文本：{typed:?}");
    if !typed.contains(&"hi") {
        return Err(format!(
            "绘制列表里没有 `hi` ⇒ 打字没有走到渲染（第二帧的文本命令：{typed:?}）"
        ));
    }

    // 像素级证据：差异必须都在输入框矩形内，**框外为 0**。
    let mut pixels = CpuRenderer::with_text(font.engine.expect("上面已确认有引擎"));
    let (inside, outside) = pixel_diff_split(&f0.list, &f1.list, extent, rect, &mut pixels)?;
    println!(
        "像素断言        : 输入框矩形 {rect:?} 内 {inside} 像素；**框外 {outside}** 像素\
         （这一帧同时变的只有输入框自己：hover/focus 视觉 + 文本内容，都在这个矩形里）"
    );
    if inside == 0 {
        return Err(
            "输入框矩形内一个像素都没变 ⇒ 打的字**没有**画出来（字形没换？度量与绘制用了不同的字号？）"
                .to_string(),
        );
    }
    if outside != 0 {
        return Err(format!(
            "有 {outside} 个像素差异落在输入框矩形 {rect:?} **之外** ⇒ 变化视觉溢出了\
             （先打印真实数据：把这一帧同时变了的**所有**东西都列成矩形再断言，见文档第 8 节）"
        ));
    }
    println!("离屏自检 ✅：树 → 几何 → 列表 → PNG 打通；打字走完了「输入 → 状态 → 画出来」这条闭环");
    Ok(())
}

/// 两帧像素的差异落在给定矩形**内**多少、**外**多少（返回的是**像素数**，不是字节数）。
///
/// 返回 `(inside, outside)`；判据是 `inside > 0 && outside == 0`。
fn pixel_diff_split(
    a: &DrawList,
    b: &DrawList,
    extent: Extent,
    rect: RectI,
    renderer: &mut CpuRenderer,
) -> Result<(usize, usize), String> {
    let pa = renderer
        .render(extent, a, CLEAR)
        .map_err(|e| format!("CPU 渲染失败：{e}"))?
        .pixels;
    let pb = renderer
        .render(extent, b, CLEAR)
        .map_err(|e| format!("CPU 渲染失败：{e}"))?
        .pixels;
    if pa.len() != pb.len() {
        return Err(format!("两帧像素长度不一致：{} vs {}", pa.len(), pb.len()));
    }
    let w = extent.width.max(1) as usize;
    let mut inside = 0usize;
    let mut outside = 0usize;
    for (idx, (x, y)) in pa.chunks_exact(4).zip(pb.chunks_exact(4)).enumerate() {
        if x == y {
            continue;
        }
        let px = (idx % w) as i32;
        let py = (idx / w) as i32;
        if rect.contains(px, py) {
            inside += 1;
        } else {
            outside += 1;
        }
    }
    Ok((inside, outside))
}

// ---------------------------------------------------------------------------
// 【步骤 5】入口
// ---------------------------------------------------------------------------

/// 取 `--key value` 或 `--key=value` 的值。
fn arg_value(args: &[String], key: &str) -> Option<String> {
    for (i, a) in args.iter().enumerate() {
        if a == key {
            return args.get(i + 1).cloned();
        }
        if let Some(v) = a.strip_prefix(&format!("{key}=")) {
            return Some(v.to_string());
        }
    }
    None
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();

    // ① 离屏档：不建窗（无窗口环境 / CI 用）。
    if args.iter().any(|a| a == "--headless") {
        return match headless() {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("[hello_window] 离屏自检失败：{e}");
                ExitCode::FAILURE
            }
        };
    }

    // ② 真窗口档：**本进程必须在主线程建事件循环**（winit 要求），
    //    所以这只能是 example 而不是 `#[test]`（`cargo test` 在子线程跑每个测试）。
    let frames: Option<u32> = arg_value(&args, "--frames").and_then(|v| v.parse().ok());
    match frames {
        Some(n) => println!("[hello_window] 门槛档：画满 {n} 帧自动退出（用于自动化验证）"),
        None => println!(
            "[hello_window] **交互模式**：点 `确定`（会打印一行）/ 点输入框打字 / Tab 换焦点 / \
             **Esc 退出**（`off` 是禁用的，点它没反应）"
        ),
    }

    let w = build_widgets();
    let app = Hello {
        tree: w.tree,
        field_id: w.field,
        theme: Theme {
            font_size: FALLBACK_FONT_SIZE,
            ..Theme::default()
        },
        extent: Extent {
            width: WIDTH,
            height: HEIGHT,
        },
        state: UiState::default(),
        dirty: false,
        drawn: 0,
        frames_left: frames,
        engine: None,
        renderer: None,
        waker: None,
    };
    match run(WindowConfig::new("deer-gui hello window", WIDTH, HEIGHT), app) {
        Ok(()) => {
            println!("[hello_window] 通过 ✅（窗口正常关闭 / 画满指定帧数）");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("[hello_window] 失败：{e}");
            ExitCode::FAILURE
        }
    }
}
