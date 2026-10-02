//! **M5-4 的闭环**：输入 → 命中 → 状态 → 重绘，而且**只在状态真的变了时才重绘**。
//!
//! ```sh
//! # 真实窗口（自己点、按 Tab/Esc；Esc 退出）
//! cargo run -q -p deer-gui --features window --example interactive_form
//!
//! # 脚本化重放（确定性，读完脚本自己退出，退出码 0 = 断言全过）
//! DEER_INPUT_SCRIPT="move @button_1;down:left;up:left;key:Tab;text:hi" \
//!   cargo run -q -p deer-gui --features window --example interactive_form
//! ```
//!
//! # 这个 example 在证明什么
//!
//! 1. **闭环**：`App::input` → `interaction::handle` → `UiState` 真的变了 → `App::redraw`
//!    画出**不一样**的一帧；每一步都往 stdout 打一行，所以「有没有闭环」是**看到的**。
//! 2. **只在 dirty 时重绘**：`redraw()` 第一件事是查脏位，不脏就打印 `redraw: no` 直接返回
//!    （**不碰 GPU**）。置脏的判据是「**状态真的变了**」而不是「收到事件了」——
//!    `PointerDown` 就属于「会改状态但可能**不发** `UiEvent`」的那一类（点空白处时
//!    `pressed` 由 `Some` 变 `None`、事件列表却是空的），只能靠比对状态逮住。
//! 3. **脚本化重放**：`DEER_INPUT_SCRIPT` 里的每一句都被解析成 [`InputEvent`]
//!    （语法见 `deer_gui::input_script`）；脚本里可以写 `move @button_1`（按**节点 id**
//!    定位，坐标由布局算出来 —— 不写死坐标，布局一变脚本跟着变，而不是静默点空）。
//!    重放完打印最终 `hover/focus/pressed/texts`，并与**内置脚本的期望终态**逐字段断言。
//! 4. **裁剪感知命中**：命中用的 `ClipSnapshot` 从**本帧真实绘制列表**派生
//!    （`InteractiveRenderer` 给每个有几何的节点发 `NodeHint`；`DefaultRenderer` **不发**
//!    ⇒ 那份快照会是空的，「全不裁剪」而**看不出来**）。所以 `init` 里先断言快照非空。
//!
//! # 关于重绘策略（M5c 后重写：`OnDemand` 下也能重放了）
//!
//! M5b 起 `deer-window` 的事件循环是 `ControlFlow::Wait`（没有事件就睡死），重绘时机由
//! [`App::redraw_policy`] 的声明决定：
//!
//! | 声明 | 语义 |
//! |---|---|
//! | `OnDemand`（库默认，省电） | 只有 [`App::wants_redraw`] 为真（**这条输入改了状态**）、系统事件、**或 App 自己排的唤醒**才来一帧 |
//! | `Continuous` | 每画完一帧续下一帧（= M5b 之前那条无条件 `ControlFlow::Poll` 的语义） |
//!
//! 本示例**默认声明 `Continuous`**（脚本重放必须**自己推进**），另有一条 **M5c 的省电重放档**：
//!
//! ```sh
//! cmd /c "set DEER_FORM_ONDEMAND=1 && set DEER_VK_WINDOW_TESTS=1 && cargo run -q -p deer-gui --features window --example interactive_form"
//! ```
//!
//! > **历史**（M5b 时写在这里的话）：「`deer-window` 没有给 App 定时器 / 用户事件这类『自己唤醒
//! > 事件循环』的手段 ⇒ 脚本重放期间窗口一个真实输入都没有，`OnDemand` 下它只会拿到建窗引导帧
//! > 那 1 帧：脚本卡在第 2 步、进程不退出（M5b-A2 实测：目标 5 条语句，35 s 超时被杀）。
//! > **这是接口边界，不是本示例在偷懒**；要在 `OnDemand` 下重放，得先给窗口层加「用户事件」面。」
//! > —— M5c 把那个面加上了（`App::wake_handle` + [`Waker`]），这条边界**不再存在**：现在
//! > `OnDemand` 档用 `Waker::wake_after`（**deadline**，不睡线程）逐帧推进脚本，跑完还能
//! > 证明「空闲期一帧都不画」。
//!
//! `OnDemand` 档的推进方式（**与真实输入共用同一个入口**）：建窗引导帧喂第 1 条脚本事件，
//! 之后**每画完一帧排一次 `wake_after(STEP)`** ⇒ 到点被唤醒 ⇒ 一帧 ⇒ 再喂一条。
//! 「状态没变就不重绘」这条判据**没有降级**，它现在是两层、且共用同一份判据（同一个 `dirty`）：
//!
//! 1. **App 侧**：`redraw()` 先查 `dirty`，不脏就**不碰 GPU**、只记一笔 `false` ——
//!    `redraw_log` / `painted` / `skipped` 就是这层的账本（`finish_script` 里断言）；
//! 2. **窗口层**：`wants_redraw()` 返回**同一份** `dirty` ⇒ `OnDemand` 下窗口层也不会为
//!    「没改状态」的输入请求帧（本示例声明连续，所以这一层在这里只影响账本的 `skipped` 计数；
//!    窗口层「空闲零重绘」的完整演示是 `cargo run -p deer-window --example idle_probe`，
//!    唤醒面的演示是 `cargo run -p deer-window --example wake_probe`）。
//!
//! **脚本事件与真实窗口事件走同一个输入入口**（[`Form::handle_input`]）：状态机、命中、置脏、
//! 记账只有一份实现 ⇒「脚本重放」与「人手点」不可能漂；脚本推进是「输入进来了」这件事本身，
//! **不是**「`redraw` 被调用」的副作用。

use std::collections::VecDeque;
use std::path::Path;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use deer_gui::gpu::interact::{FieldText, InteractiveRenderer};
use deer_gui::gpu::measure::find_system_font;
use deer_gui::gpu::null::CpuRenderer;
use deer_gui::gpu::{Color, DrawCmd, DrawList, Extent, TextEngine, Theme};
use deer_gui::input_script::{self, ENV_VAR};
use deer_gui::interaction::{self, ClipSnapshot, InputEvent, Key, UiState};
use deer_gui::layout::builder::{Builder, L};
use deer_gui::layout::layout::{self, Geometry, Measure, TextStyle};
use deer_gui::layout::node::{Kind, Node, Rect};
use deer_gui::prelude::*;
use deer_gui::vk::windowed::{FrameOutcome, WindowedRenderer};
use deer_gui::window::{App, Flow, RedrawPolicy, Waker, WindowConfig, WindowInfo, run};

/// 清屏色（离屏对照与上屏用同一个值）。
const CLEAR: Color = Color::rgb(0x08, 0x09, 0x0c);

/// 窗口尺寸（脚本里的坐标与它同一套物理像素口径）。
const WIDTH: u32 = 420;
const HEIGHT: u32 = 220;

/// M5c 省电重放档的开关（`DEER_FORM_ONDEMAND=1`；**先 `trim()` 再比**，理由见 [`window_tests_enabled`]）。
const ONDEMAND_ENV: &str = "DEER_FORM_ONDEMAND";

/// 省电重放档每步之间的间隔（`Waker::wake_after` 的 deadline；**不睡线程**）。
const STEP: Duration = Duration::from_millis(20);

/// 脚本跑完之后的**安静窗口**：这期间不排任何唤醒，用来证明「OnDemand 下空闲零重绘」。
const QUIET: Duration = Duration::from_millis(400);

/// 安静窗口的**下限比例**：墙钟到点了才算数（winit 的定时器可能早一点点醒）。
const QUIET_MIN_RATIO: f64 = 0.8;

/// 安静窗口的**兜底宽限**（M5c fix）：早于下限来的帧不判红，而是「报一笔 + 补排等到点」；
/// 但拖过 `deadline + QUIET_GRACE` 还没等到「到点」那一帧 ⇒ 判红（那是唤醒链坏了，
/// **不是**「被外来帧打断」）。
///
/// 取 2s：正常路径永远到不了这里（到点帧只会迟到几毫秒），它只用于把「唤醒链坏掉」与
/// 「窗口被系统事件反复打断」从「假红」里分出来。
/// **已知边界**：如果**连一帧都不再来**，本示例没有看门狗 ⇒ 会一直开着窗口（与改前一致；
/// 唤醒链整体坏掉这种情形由 `wake_probe` 的档先把关）。
const QUIET_GRACE: Duration = Duration::from_millis(2000);

/// 内置脚本（没设 `DEER_INPUT_SCRIPT` 时用它）—— 它**有期望终态**，所以是断言的一部分。
///
/// `move @button_1` 读作「指针移到 button_1 的中心」（坐标由布局算，不写死）。
const BUILTIN_SCRIPT: &str = "move @button_1;down:left;up:left;key:Tab;text:hi";

/// 门槛判定（`DEER_VK_WINDOW_TESTS` 有没有设）—— 判据住在 `deer_gui::env_gate`。
///
/// **为什么必须容忍两边空白**（实测数据，不是猜的）：`cmd` 的
/// `set DEER_VK_WINDOW_TESTS=1 && cargo run …` 会把 `&&` 前的空格也算进变量值 ——
/// `cmd /c "set DEER_VK_WINDOW_TESTS=1 && set DEER_VK_WINDOW_TESTS"` 实测打印
/// `DEER_VK_WINDOW_TESTS=1 `（**带一个尾空格**）；写成 `set …=1&& …`（不留空格）才是不带空格的 `1`。
/// 严格 `== "1"` 会把「**已经设了**门槛」判成「没设」⇒ 明明建了窗口、跑完了脚本、exit=0，
/// 却打印「这不是通过，是被跳过」，**与事实相反**。所以判定统一委托给
/// [`deer_gui::env_gate::flag`]（先 `trim()` 再比，断言在那个模块的单测里）。
fn window_tests_enabled() -> bool {
    deer_gui::env_gate::flag("DEER_VK_WINDOW_TESTS")
}

/// 界面树（**确定**：id 由 `IdGen` 按树序生成 ⇒ 脚本里可以写 `button_1`/`field_1`）。
fn app_tree() -> Node {
    let mut app = Builder::new(Kind::Column, "app").padding(12.0).gap(10.0);
    app.text("M5-4 interactive form");
    app.container_opts(Kind::Row, "row", L::new().gap(8.0).to_props(), |r| {
        r.button("OK");
        r.field("name");
    });
    app.button_opts("disabled", |n| n.props.disabled = true);
    app.build()
}

/// 一帧：树 + 几何 + 状态 → `DrawList`（**带 `NodeHint`** ⇒ 裁剪快照非空）。
struct Frame {
    tree: Node,
    geo: Geometry,
    list: DrawList,
    clip: ClipSnapshot,
}

impl Frame {
    /// 几何**每帧重算**（很便宜，而且保证与列表同源）；列表由状态派生。
    fn build(
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
        // ⚠️ 不要手抄这份转换 —— 用 `UiState::to_interact_state()`（唯一收口）。
        // 这里保留字面写法只是历史示例，但字段必须齐（T3.2/T3.4 加的 scroll/preedit/carets），
        // 否则 `--features window` 的 examples 编不过（master 上一度真红过：PR 里只跑了默认 feature）。
        let interact = state.to_interact_state();
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

    /// 节点 id 的中心点（脚本的 `move @id` 用它算坐标）。
    fn center(&self, id: &str) -> Result<(f32, f32), String> {
        let r = self
            .geo
            .get(id)
            .copied()
            .ok_or_else(|| format!("布局没有给 `{id}` 几何"))?;
        Ok((r.x + r.w / 2.0, r.y + r.h / 2.0))
    }

    fn rect_of(&self, id: &str) -> Option<RectI> {
        self.geo.get(id).map(|r| {
            RectI::new(r.x as i32, r.y as i32, r.w as i32, r.h as i32)
        })
    }
}

/// 把脚本模板里的 `move @id` 换成 `move:x,y`。
///
/// **为什么要有这一步**：写死坐标的脚本在布局改变后会**静默点到空处**，
/// 于是「重放通过」变成一句空话。按 id 定位让脚本与布局解耦，而且仍然可读。
fn resolve_script(tpl: &str, frame: &Frame) -> Result<String, String> {
    let mut out = String::new();
    for stmt in tpl.split(';') {
        let stmt = stmt.trim();
        if stmt.is_empty() {
            continue;
        }
        if let Some(id) = stmt.strip_prefix("move @") {
            let (x, y) = frame.center(id.trim())?;
            out.push_str(&format!("move:{x},{y};"));
        } else {
            out.push_str(stmt);
            out.push(';');
        }
    }
    Ok(out)
}

/// 与 `idle` 比较，返回（矩形内差异字节数、矩形外差异字节数）。
fn diff_split(a: &[u8], b: &[u8], extent: Extent, rect: RectI) -> (usize, usize) {
    let w = (extent.width.max(1)) as usize;
    let mut inside = 0usize;
    let mut outside = 0usize;
    for i in 0..a.len().min(b.len()) {
        if a[i] == b[i] {
            continue;
        }
        let px = ((i / 4) % w) as i32;
        let py = ((i / 4) / w) as i32;
        if rect.contains(px, py) {
            inside += 1;
        } else {
            outside += 1;
        }
    }
    (inside, outside)
}

fn fmt_state(s: &UiState) -> String {
    format!(
        "hover={:?} focus={:?} pressed={:?} texts={:?}",
        s.hover, s.focus, s.pressed, s.texts
    )
}

/// 一条输入事件的**来源**：只影响打印与「脚本模式下忽略窗口事件」的判定，**不影响状态机**。
///
/// 做成显式参数（而不是两个入口函数）是刻意的：让「脚本重放」与「人手输入」在
/// [`Form::handle_input`] 里**合流** ⇒ Escape 语义、置脏、记账都只有一条路径。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Source {
    /// 脚本重放喂进来的事件（`DEER_INPUT_SCRIPT` 或内置脚本）。
    Script,
    /// 真实窗口事件（[`App::input`]）。
    Window,
}

struct Form {
    theme: Theme,
    extent: Extent,
    state: UiState,
    /// 脏位：`redraw()` 只在它置上时才真的画。
    dirty: bool,
    /// 每一帧的账：`true` = 真的重绘了；`false` = 状态没变、跳过了。
    redraw_log: Vec<bool>,
    /// 脚本还没喂完的事件（空 ⇒ 交互模式）。
    queue: VecDeque<InputEvent>,
    /// **被画掉的**脚本事件里，每条是否真的改了状态（`step_script` 里记；最后那帧
    /// 「队列已空 ⇒ 收尾」与「脚本里 Escape」不记账 —— 它们没有画任何输入）。
    fed_changed: Vec<bool>,
    /// 已喂掉的事件条数（= `fed_changed.len()`；单独留着是为了少写 `.len()` 与便于打印）。
    fed_events: usize,
    /// 脚本模式：喂完之后要断言的期望终态（内置脚本才有；自定义脚本为 `None`）。
    expected: Option<UiState>,
    /// 已经真的画过几帧。
    rendered: u64,
    /// 上一次读到的 `ui_text_skipped`（累计值 ⇒ 差值才是「这一帧掉了几个字形」）。
    last_skipped: Option<usize>,
    engine: Option<TextEngine>,
    renderer: Option<WindowedRenderer>,
    /// 环境变量给的自定义脚本模板（`None` ⇒ 用内置脚本，那个有期望终态）。
    script_src: Option<String>,
    done: bool,
    /// **M5c 省电重放档**：声明 `OnDemand` 并用 [`Waker`]（deadline）推进脚本。
    ondemand: bool,
    /// 建窗后从 [`App::wake_handle`] 拿到的唤醒句柄（`OnDemand` 档的推进器）。
    waker: Option<Waker>,
    /// `wake_after` 排了几次（可数与核对：**每画完一帧排一次**）。
    wake_after_calls: u64,
    /// 安静窗口（`OnDemand` 档：脚本跑完之后的「什么都不做」那段）。
    quiet: Option<Quiet>,
    /// `wants_redraw` 被窗口层问了几次（`OnDemand` 档应当**一次都没有**：没输入、没 `wake()`）。
    ///
    /// 用 `Cell`：问它的地方是 `wants_redraw(&self)` —— 那里改不了自己的字段（接口事实）。
    wants_asked: std::cell::Cell<u64>,
    /// 安静窗口期间收到的**外来输入**条数（本档不注入输入 ⇒ 每条都是环境噪声）。
    ///
    /// 它不是省电失效的证据（`wants_redraw` 答假 ⇒ 一帧都不画），但会让 `wants_redraw` 的
    /// **增量口径**被污染 ⇒ `finish_quiet` 据此把那一**条**判据显式跳过并写明「这不是通过」。
    quiet_inputs: u64,
    /// 安静窗口期间**早于 `deadline`** 来的帧数（系统事件 / 外来输入 / 上一个没兑现的 `STEP`）。
    ///
    /// M5c 复审 I2：这类帧**不是**「QUIET 到点那一帧」，不能拿它判「窗口结束」。
    quiet_interruptions: u64,
}

/// 安静窗口的现场：起点 + **到点时刻** + 起点时的帧数/计数。
///
/// `deadline` 是 M5c fix 的关键：**早于它来的帧一律不是 QUIET 到点那一帧**（别的来源），
/// 于是「墙钟」这条代理判据不再被一次外来帧误判成「事件循环在打转」（复审 I2 的假红）。
struct Quiet {
    started: Instant,
    /// `started + QUIET`（绝对时刻；补排时用它算剩余时间，所以窗口**不会**被反复打断而无限延长）。
    deadline: Instant,
    frames_at_start: u64,
    /// 进安静窗口时 `wants_asked` 的快照（M5c 复审 M4：进程级计数不能用来对「安静窗口里」下结论）。
    wants_asked_at_start: u64,
}

impl Form {
    /// 当前这一帧（有字体引擎就用它的真实度量，否则用近似度量）。
    ///
    /// 写成两支而不是 `Box<dyn Measure>`：`FontMeasure` 借用了引擎，装进 trait object 会
    /// 逼出生命周期体操（或 `Box::leak` 泄漏），而这里只需要「两条具体路径」二选一。
    fn frame(&self) -> Frame {
        match &self.engine {
            Some(engine) => Frame::build(
                &app_tree(),
                &self.theme,
                self.extent,
                &self.state,
                &engine.measure(),
            ),
            None => Frame::build(
                &app_tree(),
                &self.theme,
                self.extent,
                &self.state,
                &ApproxMeasure,
            ),
        }
    }

    /// 喂一条输入事件：`handle` + 「状态真的变了才置脏」。返回是否要退出（`key:Escape`）。
    ///
    /// **输入路径的核心**，只被 [`Form::handle_input`] 调用 —— 脚本事件与真实窗口事件
    /// 都经过它，所以「命中 → 状态 → 是否置脏」只有一份实现。
    fn feed(&mut self, ev: &InputEvent) -> bool {
        let f = self.frame();
        let before = self.state.clone();
        let events = interaction::handle(&mut self.state, &f.tree, &f.geo, f.clip.clone(), ev);
        let changed = self.state != before;
        self.dirty |= changed;
        println!(
            "input: {ev:?} => state_changed={changed} dirty={} events={events:?} {}",
            self.dirty,
            fmt_state(&self.state)
        );
        if !changed {
            println!("        （状态没变 ⇒ 不置脏 ⇒ 这一帧不会重绘）");
        }
        matches!(ev, InputEvent::KeyDown { key: Key::Escape, .. })
    }

    /// **唯一的输入入口**：脚本事件与真实窗口事件都在这里落地（M5b-A2 的要点）。
    ///
    /// 做三件事：① 状态机（[`Form::feed`]：命中 → 更新 `UiState` → **状态变了才置脏** → 打印/记账）；
    /// ② `key:Escape` ⇒ 与真实窗口按 Esc **同一条语义**（先收尾：打账本 + 断言，再退出）；
    /// ③ 返回 [`Flow`]（`Exit` = 请求结束事件循环）。
    ///
    /// 为什么脚本推进落在这里（而不是散在 `redraw` 里）：这样「一条输入进来了」是**驱动**，
    /// `redraw` 只负责「按 `dirty` 决定画不画」；[`App::wants_redraw`] 报的也是同一份判据。
    fn handle_input(&mut self, ev: &InputEvent, source: Source) -> Result<Flow, String> {
        if self.done {
            return Ok(Flow::Exit);
        }
        if self.feed(ev) {
            // 脚本里的 `key:Escape` 与真实窗口按 Esc 同一条语义（退出），但**收尾仍然要做**
            // （先打印终态、再断言），否则「脚本结束」没有可比对的数字。
            // 注意：这一帧**没有画过**任何输入 ⇒ 不记账（`step_script` 拿到 `Exit` 就直接返回），
            // 所以它不会去和「重绘次数」对账（见 `finish_script` 的口径）。
            println!(
                "{} ⇒ 收尾",
                if source == Source::Script {
                    "脚本里的 Escape"
                } else {
                    "窗口按 Esc"
                }
            );
            // M5c：收尾走 `exit_or_quiet` —— `Continuous` 档直接退，`OnDemand` 档先进安静窗口。
            return self.exit_or_quiet();
        }
        Ok(Flow::Continue)
    }

    /// 喂**一条**脚本事件（`redraw` 每帧喂一条 ⇒ 事件 → 状态 → 重绘 的关系是可数的）。
    ///
    /// 为什么不用「一次把脚本喂完」：那样整段重放会被**一帧**画完（`redraw_log` 只有 1 个
    /// `true`），于是「输入 ⇒ 置脏 ⇒ 重绘」这条链在日志里根本看不出来 —— 日志看着像通过，
    /// 实际上什么都没证明。每帧一条事件，`redraw: yes/no` 的序列就是这条链的证据。
    ///
    /// **M5b-A2**：事件不再直接调 `feed`，而是走 [`Form::handle_input`] —— 与真实窗口事件
    /// **同一个入口**。脚本推进因此是「输入路径」的一环，而不是「`redraw` 顺手改了状态」。
    fn step_script(&mut self) -> Result<Option<Flow>, String> {
        let Some(ev) = self.queue.pop_front() else {
            return Ok(None);
        };
        let before = self.state.clone();
        let flow = self.handle_input(&ev, Source::Script)?;
        if flow == Flow::Exit {
            // 已经在 `handle_input` 里收尾并退出：这一帧没画过输入 ⇒ 不记账。
            return Ok(Some(Flow::Exit));
        }
        // 记账：这一帧确实喂了一条输入事件；它是否真的改了状态（= 是否该重绘那一帧）。
        self.fed_events += 1;
        self.fed_changed.push(self.state != before);
        Ok(Some(Flow::Continue))
    }

    /// **一帧画完之后**（M5c 的 `OnDemand` 档）：排下一次唤醒，好让脚本继续推进。
    ///
    /// 这就是「`OnDemand` 下自己推进」的全部秘密：**不睡线程**，只装一个 deadline ——
    /// `deer-window` 平时仍然睡在 `ControlFlow::Wait` 上，到点由系统叫醒（`wake_probe` 量过：
    /// 预约迟到 < 2ms，两次唤醒之间零 CPU）。
    ///
    /// 排的时机是「**每画完一帧**排一次」⇒ 帧的序列与 `Continuous` 档**逐帧同构**
    /// （所以 `finish_script` 那套帧账判据一个字都不用改）；区别只在两帧之间**真的睡着了**。
    fn schedule_step(&mut self) {
        if !self.ondemand {
            return;
        }
        // ⚠️ 收尾之后**不许**再排「下一步」的唤醒：`exit_or_quiet` 已经排了安静窗口那一次，
        // 再插一个更早的（`STEP` < `QUIET`）会让安静窗口的**上界**变成那个 STEP（窗口层取
        // `earliest`）⇒ 安静窗口被一次「**非 deadline** 的帧」打断（M5c 复审 I2 实测：15–20ms
        // 就被打断，当时那被误判成「事件循环在打转」）。`finish_quiet` 现在分得清这种打断
        // （如实报出 + 按绝对值补排到点），但这条护栏仍然保留：**别**在收尾之后再排唤醒。
        if self.done || self.quiet.is_some() {
            return;
        }
        let Some(waker) = self.waker.clone() else {
            println!("[interactive_form] ⚠️ 没有唤醒句柄 ⇒ 脚本推不动（不该发生：OnDemand 档必须有）");
            return;
        };
        self.wake_after_calls += 1;
        waker.wake_after(STEP);
        println!("[interactive_form] 排下一次唤醒：wake_after({}ms)（deadline，不睡线程）", STEP.as_millis());
    }

    /// 脚本**喂完**之后的收尾：先断言（[`Form::finish_script`]），再按档决定怎么退。
    ///
    /// - `Continuous` 档（默认）：直接 `Flow::Exit`（与 M5b 行为逐字一致）；
    /// - `OnDemand` 档：进**安静窗口** —— 排**唯一**一次 `wake_after(QUIET)`（唯一指的是
    ///   「为推进脚本」；被一次**非 deadline** 的帧打断时会为「等到到点」按绝对值**补排**一次，
    ///   见 [`Form::finish_quiet`]），期间不排任何别的唤醒、也没有输入。**窗口内一帧都不许画**，
    ///   窗口到点那一帧进来时在 `redraw` 顶部的 `done` 分支里处理（只断言、不画、不碰 GPU）。
    fn exit_or_quiet(&mut self) -> Result<Flow, String> {
        self.finish_script()?;
        if !self.ondemand {
            return Ok(Flow::Exit);
        }
        let Some(waker) = self.waker.clone() else {
            return Err(format!(
                "{ONDEMAND_ENV} 档必须先拿到 Waker（App::wake_handle 没被调用？）"
            ));
        };
        let started = Instant::now();
        self.quiet = Some(Quiet {
            started,
            // **到点的绝对时刻**：判「这一帧是不是 QUIET 到点那一帧」全靠它（复审 I2）。
            deadline: started + QUIET,
            frames_at_start: self.rendered,
            // 增量口径的快照（复审 M4）。
            wants_asked_at_start: self.wants_asked.get(),
        });
        self.wake_after_calls += 1;
        waker.wake_after(QUIET);
        println!();
        println!(
            "[interactive_form] 安静窗口：{}ms —— 期间**不为推进脚本**排任何唤醒、没有输入、不碰 GPU；\
             到点那一帧只做断言（wake_after 累计 {} 次）",
            QUIET.as_millis(),
            self.wake_after_calls
        );
        Ok(Flow::Continue)
    }

    /// 安静窗口**收尾**那一帧（`redraw` 顶部 `done` 分支调用）：只断言，不画。
    ///
    /// **M5c fix（复审 I2）**：先按 `quiet.deadline` **区分这一帧的来源**，再判老判据 ——
    ///
    /// - 帧来得**早**（`elapsed < QUIET_MIN_RATIO × QUIET`）⇒ 它**不是**「QUIET 到点」那一帧
    ///   （来源：系统事件 / 外来输入 / 上一个还没兑现的 `STEP`，「deadline 到点 ⇒ 一定画一帧」
    ///   这条本层语义保证了「到点帧」不会这么早）。**照实报一笔**、把到点那一刻按**绝对值**
    ///   补排（残留 STEP 被兑现后窗口层手里已经没有 deadline 了，不补排就没人再叫醒事件循环），
    ///   窗口**继续**等 —— 不再像改前那样把任何一帧都当成「窗口结束」，也不再误诊成
    ///   「事件循环在打转」（复审 I2 的假红现场就是 15ms 被一次外来帧判红）。
    /// - 帧来得**不早** ⇒ 这才是「睡到点」（允许 winit 定时器早一点点醒 ⇒ 用同一个下限比例）。
    ///
    /// 判据（两条，都可数）：
    ///
    /// 1. **窗口内重绘 = 0 帧**（`rendered` 在这段时间里一次都没涨）；
    /// 2. **`wants_redraw` 增量 = 0**（进窗口时的快照 ⇒ 只在安静窗口内问才算数）。
    ///    若窗口期间有**外来输入**（环境噪声，不是产品缺陷）⇒ 这一条**显式跳过**并打印
    ///    「这不是通过，是被跳过」，其余判据照判。
    fn finish_quiet(&mut self) -> Result<Flow, String> {
        // 显式前置断言：只有 `quiet` 是 Some 时才该走到这里（调用方查过，这里再钉一次 ——
        // 本项目纪律：前置不成立不许静默）。
        let Some(quiet) = self.quiet.as_ref() else {
            return Err("内部错误：finish_quiet 被调用时安静窗口已经结束".to_string());
        };
        let started = quiet.started;
        let deadline = quiet.deadline;
        let frames_at_start = quiet.frames_at_start;
        let asked_at_start = quiet.wants_asked_at_start;

        let now = Instant::now();
        let elapsed = now.saturating_duration_since(started);
        let floor = QUIET.mul_f64(QUIET_MIN_RATIO);

        // ★ 帧来源判定：早于下限 ⇒ 它**不是**「QUIET 到点」那一帧。
        //
        // 但**不能**把它当「窗口结束」（改前的假红就是这么来的）。处理：
        // 如实报一笔 + 把到点那一刻按**绝对值**补排；只有拖过 `deadline + QUIET_GRACE` 还这样，
        // 才落到下面的红（那时说明「到点帧」根本没来 —— 唤醒链坏了，而不是「睡到点」）。
        let early = elapsed < floor;
        let over_grace = now.saturating_duration_since(deadline) >= QUIET_GRACE;
        if early && !over_grace {
            self.quiet_interruptions += 1;
            println!();
            println!(
                "⚠️  安静窗口被一次**非 deadline** 的帧打断（第 {} 次，{}ms < 下限 {}ms）——\
                 这不是省电失效（`rendered` 没涨、GPU 没碰）；来源是系统事件 / 外来输入 / \
                 上一个还没兑现的 STEP（窗口层的 `earliest` 会保留更早的那个）。窗口继续等到点。",
                self.quiet_interruptions,
                elapsed.as_millis(),
                floor.as_millis()
            );
            let remain = deadline.saturating_duration_since(now);
            let Some(waker) = self.waker.clone() else {
                return Err("安静窗口被打断后要补排 wake_after，但唤醒句柄不见了".to_string());
            };
            self.wake_after_calls += 1;
            waker.wake_after(remain);
            println!(
                "[interactive_form] 补排一次 wake_after({}ms) 到安静窗口到点（wake_after 累计 {} 次）",
                remain.as_millis(),
                self.wake_after_calls
            );
            return Ok(Flow::Continue);
        }

        // 到这里有两种可能：① 真的到点了（`elapsed >= floor`，正常路径）；② 早到 **且** 已经拖过
        // `deadline + QUIET_GRACE`（唤醒链没把「到点帧」叫来 ⇒ 下面判红）。
        // 两种都先**消费**掉安静窗口、把数字打出来，再判。
        self.quiet = None;
        let painted = self.rendered - frames_at_start;
        let asked_delta = self.wants_asked.get().saturating_sub(asked_at_start);
        println!();
        println!("—— 安静窗口结束（OnDemand：由一次 wake_after 到点叫醒）——");
        println!(
            "窗口时长    : {}ms（要求 ≥ {}ms = {}×{}ms；超过 deadline + {}ms 视为唤醒链坏了）",
            elapsed.as_millis(),
            floor.as_millis(),
            QUIET_MIN_RATIO,
            QUIET.as_millis(),
            QUIET_GRACE.as_millis()
        );
        println!("窗口内重绘  : {painted} 帧（要求 **0**）");
        println!(
            "wants_redraw 被问 : {asked_delta} 次（**进窗口后的增量**，要求 **0** —— 没输入、没 wake()）"
        );
        println!(
            "期间未到点的帧 / 外来输入 : {} 次 / {} 条（如实报出）",
            self.quiet_interruptions, self.quiet_inputs
        );
        println!("wake_after 累计   : {} 次", self.wake_after_calls);
        if painted != 0 {
            return Err(format!(
                "安静窗口内重绘了 {painted} 帧 —— OnDemand 下空闲**必须零重绘**（有人在凭空唤它）"
            ));
        }
        if early {
            // 这条判据就是改前那句「窗口时长 ≥ 0.8×QUIET」，只是**只在真的没等到点时才红**
            // （改前把任何一帧都当窗口结束 ⇒ 一次外来帧就假红，复审 I2 的现场是 15ms）。
            return Err(format!(
                "安静窗口在 {}ms（< 下限 {}ms）就被判为结束，而且已经超过 deadline + {}ms 还没等到\
                 「到点」那一帧（被打断 {} 次、外来输入 {} 条）⇒ 不是「睡到点」，是唤醒链没把到点\
                 那一帧叫来",
                elapsed.as_millis(),
                floor.as_millis(),
                QUIET_GRACE.as_millis(),
                self.quiet_interruptions,
                self.quiet_inputs
            ));
        }
        let mut asked_skipped = false;
        if asked_delta != 0 {
            if self.quiet_inputs > 0 {
                // 污染 ≠ 失败：这条判据的**前提**（「安静窗口里没有输入」）不成立 ⇒ 显式跳过。
                asked_skipped = true;
                println!(
                    "⏭️  **这不是通过，是被跳过**：「wants_redraw 增量 0」这条没验到 —— 安静窗口\
                     期间进来了 {} 条**外来输入**（环境噪声，不是产品缺陷）。请重跑（重跑时别碰\
                     鼠标/键盘）。",
                    self.quiet_inputs
                );
            } else {
                return Err(format!(
                    "安静窗口内 wants_redraw 被问了 {asked_delta} 次（**增量**，要求 0）—— \
                     本档没有输入、也没叫 wake()，是有人凭空问了它"
                ));
            }
        }
        println!(
            "空闲期零重绘断言 ✅：{painted} 帧 / {}ms 空闲，事件循环真的睡着（OnDemand 没被削弱）{}",
            elapsed.as_millis(),
            if asked_skipped {
                "（「wants_redraw 增量 0」那条**被跳过**：外来输入污染 —— 这不是通过，是被跳过）"
            } else {
                ""
            }
        );
        Ok(Flow::Exit)
    }

    /// 脚本结束：**先打印真实数据，再打期望值，最后断言**。
    ///
    /// 幂等：`redraw` 的两条分支都可能走到这里（「队列空了」与「脚本里 Escape 了」），
    /// 收尾只能做一次 —— 否则第二次收尾时的帧账已经不对了（会报一个假的帧数错误）。
    fn finish_script(&mut self) -> Result<(), String> {
        if self.done {
            return Ok(());
        }
        self.done = true;
        println!();
        println!("—— 脚本重放结束 ——");
        println!("真实终态    : {}", fmt_state(&self.state));
        println!(
            "dirty 账本  : 重绘 {} 次 / 共 {} 帧（不脏的帧只打印，不画）",
            self.redraw_log.iter().filter(|d| **d).count(),
            self.redraw_log.len()
        );
        println!(
            "              序列 = {:?}（true = 真的重绘了这一帧；false = 状态没变、跳过）",
            self.redraw_log
        );
        // 判据：**重绘的次数 == 首帧 + 每条被喂掉的输入事件**。
        //
        // 为什么数「重绘」而不是「帧」：脚本收尾会在**最后一帧的中间**发生（队列空了 / 写了
        // Escape），那一帧已在账本里记过；之后再来的 `redraw` 直接走 `done` 早退、不记账。
        // 所以「账本长度 == 已喂事件数 + 1」并不成立，而「**重绘**次数 == 已喂事件数 + 1」成立，
        // 而且它才是我们真正要证明的那件事：每次输入都换来一次真重绘。
        // 判据（dirty 账本的可断言形态）——**三条计数必须互相印证**：
        //
        // 1. `redraw` 的每一次调用都记一笔（`true` = 真的画了、`false` = 状态没变、跳过）；
        // 2. `painted == 1 + 改了状态的输入事件数`：建窗首帧一次（`init` 里置了 `dirty`），
        //    加上每一条**真的改了状态**的输入各一次；
        // 3. `skipped == 已喂事件数 - 改了状态的输入事件数`：**没改状态的输入一帧都不画**。
        //
        // 🚫 不要写成「帧数 == 脚本条数 + 1」：脚本里出现 `key:Escape` 时，那一帧既没有被画、
        //    也不该被计数（调用方拿到退出请求就直接返回了），这个式子会**假红**。
        // 🚫 也不要写「skipped 必须为 0」：那只是**内置脚本**的性质（它每一步都改状态），
        //    对「点空白处」这种**故意不改状态**的脚本，跳过 3 帧才是正确行为。
        let painted = self.redraw_log.iter().filter(|d| **d).count();
        let skipped = self.redraw_log.iter().filter(|d| !**d).count();
        let changed = self.fed_changed.iter().filter(|c| **c).count();
        println!(
            "              （记账 {} 条 = 重绘 {painted} + 跳过 {skipped}｜已喂事件 {}（其中改状态 {changed}））",
            self.redraw_log.len(),
            self.fed_events
        );
        if painted != 1 + changed {
            return Err(format!(
                "重绘 {painted} 次，但「建窗首帧 1 + 改状态的输入 {changed}」要求 {} 次\
                 —— 记账有漏；序列 = {:?}",
                1 + changed,
                self.redraw_log
            ));
        }
        if skipped != self.fed_events - changed {
            return Err(format!(
                "跳过 {skipped} 帧，但「已喂 {} - 改状态 {changed}」要求 {} 帧\
                 —— 没改状态的输入一帧都不许画；序列 = {:?}",
                self.fed_events,
                self.fed_events - changed,
                self.redraw_log
            ));
        }
        if self.expected.is_some() && changed != self.fed_events {
            return Err(format!(
                "内置脚本的每一步都该改状态，实测 {} / {} 条改了 —— 脚本或状态机变了？",
                changed, self.fed_events
            ));
        }
        println!(
            "dirty 账本断言 ✅：{painted} 次重绘 = 建窗首帧 + {changed} 条改状态的输入；\
             跳过 {skipped} 帧（= 没改状态的输入数）"
        );
        if let Some(expected) = self.expected.clone() {
            println!("期望终态    : {}", fmt_state(&expected));
            if self.state != expected {
                return Err(format!(
                    "脚本重放的终态与期望不一致：实际 {} / 期望 {}",
                    fmt_state(&self.state),
                    fmt_state(&expected)
                ));
            }
            println!("脚本重放终态断言 ✅");
        } else {
            println!("（自定义脚本：只断言「解析 + 重放 + 退出」，不断言终态）");
        }
        Ok(())
    }
}

impl App for Form {
    fn init(&mut self, info: &WindowInfo) -> Result<(), String> {
        self.extent = Extent {
            width: info.extent.width.max(1),
            height: info.extent.height.max(1),
        };
        println!(
            "[interactive_form] 窗口 {}×{}（物理像素；脚本坐标与它同一套口径）",
            self.extent.width, self.extent.height
        );
        if let Some(font) = find_system_font() {
            match TextEngine::from_font_file(Path::new(&font), self.theme.font_size) {
                Ok(e) => {
                    println!("[interactive_form] 字体：{}", font.display());
                    self.engine = Some(e);
                }
                Err(e) => println!("[interactive_form] 字体解析失败，退回占位文本：{e}"),
            }
        } else {
            println!("[interactive_form] 没找到系统字体 ⇒ 退化路径：占位文本（交互链不受影响）");
        }

        let r = WindowedRenderer::new(0, info.raw, self.extent, CLEAR)
            .map_err(|e| format!("创建窗口渲染器失败：{e}"))?;
        println!("[interactive_form] 适配器：{}", r.adapter().name);
        self.renderer = Some(r);

        // ★ 前置断言：这一帧的列表必须发得出 `NodeHint`，否则命中会退化成「全不裁剪」
        //   而**不报错**（本项目纪律：前置不成立不会自己喊疼）。
        let f = self.frame();
        let hints = f.list.counts().node_hint;
        if hints == 0 || f.clip.is_empty() || !f.clip.is_known("button_1") {
            return Err(format!(
                "前置条件不成立：这一帧的列表没有节点提示（node_hint={hints}，快照 len={}）—— \
                 命中会退化成「全不裁剪」而不报错；检查是不是用了不发 NodeHint 的渲染器",
                f.clip.len()
            ));
        }
        println!(
            "[interactive_form] 绘制列表：{} 条命令（NodeHint {hints} 条）；\
             裁剪快照 {} 个节点，button_1 的裁剪 = {:?}",
            f.list.len(),
            f.clip.len(),
            f.clip.clip_of("button_1")
        );
        println!(
            "[interactive_form] 焦点树序：{:?}",
            interaction::focusables(&f.tree)
        );

        // 脚本：内置脚本有期望终态（是判据）；自定义脚本只保证「解析 + 重放 + 退出」。
        let custom = self.script_src.is_some();
        let tpl = self.script_src.clone().unwrap_or_else(|| BUILTIN_SCRIPT.to_string());
        let src = resolve_script(&tpl, &f)?;
        println!("[interactive_form] 脚本（{}）：{src}", if custom { "自定义" } else { "内置" });
        let evs = input_script::parse_script(&src)?;
        println!("[interactive_form] 解析出 {} 条输入事件", evs.len());
        self.queue = evs.into_iter().collect();
        if !custom {
            self.expected = Some(expected_state());
        }

        // 首帧必然要画（窗口刚建好）。
        self.dirty = true;
        if self.ondemand {
            println!(
                "[interactive_form] M5c 省电重放档：{ONDEMAND_ENV}=1 ⇒ 声明 OnDemand，\
                 脚本由 Waker::wake_after({}ms) 的 deadline 推进（不睡线程）",
                STEP.as_millis()
            );
        }
        Ok(())
    }

    fn resized(&mut self, width: u32, height: u32) -> Result<(), String> {
        let e = Extent {
            width: width.max(1),
            height: height.max(1),
        };
        if let Some(r) = self.renderer.as_mut() {
            r.resize(e).map_err(|err| format!("重建交换链失败：{err}"))?;
        }
        self.extent = e;
        self.dirty = true;
        Ok(())
    }

    /// **真实窗口事件的入口**（脚本事件由 `redraw` 每帧喂一条，见 [`Form::step_script`]）。
    ///
    /// 两条来源最终都进 [`Form::handle_input`] —— 状态机/置脏/记账只有那一份实现。
    ///
    /// `deer-window` 会在本方法返回后**立刻**读一次 [`App::wants_redraw`]（「这条输入改了状态吗」）：
    /// 脚本模式下窗口事件被忽略 ⇒ 状态不会被真实输入改脏 ⇒ 返回 `Continue` 时 `dirty` 不变。
    fn input(&mut self, _info: &WindowInfo, ev: &InputEvent) -> Result<Flow, String> {
        if self.done {
            // **M5c fix（复审 I2 的衍生缺口）**：安静窗口里的外来输入**不能**像改前那样直接
            // `Flow::Exit` —— 那会让安静窗口的两条判据**一条都不跑**（`finish_quiet` 永远不被
            // 调用、`run()` 照样 Ok）⇒ **静默通过**，比假红更坏。这里数一笔、如实报出来，
            // 把窗口留给 deadline（`wants_redraw` 那条判据会按「被污染 ⇒ 跳过」处理）。
            if self.quiet.is_some() {
                self.quiet_inputs += 1;
                println!(
                    "[interactive_form] ⚠️ 安静窗口期间收到**外来输入**（第 {} 条）：{ev:?} \
                     —— 如实报出；它不是省电失效（答假就不画），但会污染 wants_redraw 的增量口径",
                    self.quiet_inputs
                );
                return Ok(Flow::Continue);
            }
            return Ok(Flow::Exit);
        }
        if !self.queue.is_empty() {
            // 脚本模式下**忽略**窗口事件：重放必须确定性（跑脚本时动鼠标不该改结果）。
            println!("input: 脚本模式，忽略窗口事件 {ev:?}");
            return Ok(Flow::Continue);
        }
        // 交互模式（队列已空）：窗口事件与脚本事件走**同一个入口**。
        self.handle_input(ev, Source::Window)
    }

    /// **M5b：这一条输入改了状态吗**（`deer-window` 在每条输入派发之后**立刻**读一次）。
    ///
    /// 报的就是 `redraw()` 用的**同一份** `dirty`（由 `init`/`resized` 的首帧与 `feed` 里
    /// 「状态真的变了才置脏」置上，由 `redraw` 画完清掉）⇒「窗口层要不要请求一帧」与
    /// 「App 要不要碰 GPU」共用一份判据，两处不可能漂。
    ///
    /// `OnDemand` 下这就是省电的开关（本示例默认声明连续，所以这里只影响窗口层账本的
    /// `skipped` 计数；窗口层的省电演示见 `cargo run -p deer-window --example idle_probe`）。
    /// M5c：顺手数一下它**被问了几次** —— `OnDemand` 重放档里必须是 **0**（没输入、也没叫
    /// `wake()` ⇒ 窗口层没有问它的理由），见 [`Form::finish_quiet`]。
    fn wants_redraw(&self) -> bool {
        // `&self` 改不了自己的字段 ⇒ 计数走 `Cell`（只加一个计数，不改 `dirty` 本身）。
        self.wants_asked.set(self.wants_asked.get() + 1);
        self.dirty
    }

    /// **M5c 的 `OnDemand` 重放档**：建窗后拿到唤醒句柄，存下来。
    ///
    /// 注意这里**不**立刻排唤醒 —— 推进链是从**建窗引导帧**起步的（那一帧喂第 1 条脚本事件、
    /// 然后排第一次 `wake_after`）。这样帧的序列与 `Continuous` 档**逐帧同构**，
    /// 没有「引导帧与第一次唤醒抢时序」这种竞态。
    fn wake_handle(&mut self, waker: Waker) {
        println!(
            "[interactive_form] wake_handle：拿到唤醒句柄（{}）",
            if self.ondemand {
                format!("{ONDEMAND_ENV}=1 ⇒ 省电重放档：OnDemand + Waker（deadline）推进")
            } else {
                format!("未设 {ONDEMAND_ENV} ⇒ 常规档声明 Continuous，本档用不到唤醒面（照旧存下来）")
            }
        );
        self.waker = Some(waker);
    }

    /// **M5b/M5c：重绘策略**。
    ///
    /// - 默认（`Continuous`）：脚本重放必须自己推进，M5b 只能靠「每画完一帧续一帧」；
    /// - `DEER_FORM_ONDEMAND=1`（省电重放档）：声明 `OnDemand`，改由 [`Waker::wake_after`]
    ///   的 **deadline** 推进（见模块文档「关于重绘策略」）。
    ///
    /// 代价说清楚：`Continuous` 每画完一帧都请求下一帧（空闲也烧 CPU）。`OnDemand` 档没有这个
    /// 代价 —— 两帧之间事件循环**真的睡着**（`wake_probe` 量过：预约迟到 < 2ms、迭代次数个位数）。
    fn redraw_policy(&self) -> RedrawPolicy {
        if self.ondemand {
            RedrawPolicy::OnDemand
        } else {
            RedrawPolicy::Continuous
        }
    }

    /// 画一帧（**只在 `dirty` 时真的碰 GPU**）。
    ///
    /// **M5b-A2 的因果方向**：`redraw` 自身**不直接**改状态 —— 它只**驱动**脚本：每帧把下一条
    /// 脚本事件交给 [`Form::handle_input`]（输入路径，与真实窗口事件同一个入口），
    /// 然后按**输入路径置下的** `dirty` 决定画不画。于是「一条输入 ⇒ 状态 ⇒ 这一帧画出来的
    /// 东西」是同一条因果链，而「脚本推进」不再依赖「`redraw` 被调用」。
    ///
    /// ⚠️ 措辞要精确（M5b 终审 Minor 4）：**不要**把这条写成「`redraw` **不再改状态**」—— 逐字不准确：
    /// 它经 `step_script → handle_input → feed` **确实仍会**改 `state`（`:591`/`:611`，这正是它推进
    /// 脚本的方式）。准确的说法是：**状态变更只有一条路径**（唯一入口 [`Form::handle_input`]），
    /// `redraw` 自己**不再直接**改；它另外动的只有账本与帧计数（`redraw_log` / `rendered` / `dirty`，
    /// 以及 `OutOfDate` 时重新置脏）。
    fn redraw(&mut self) -> Result<Flow, String> {
        if self.done {
            // M5c 的 `OnDemand` 档：脚本已经喂完，这一帧是**安静窗口到点**那一帧 —— 只断言。
            if self.quiet.is_some() {
                return self.finish_quiet();
            }
            return Ok(Flow::Exit);
        }
        if !self.dirty {
            // 不脏 ⇒ **不碰 GPU**。脚本模式下每帧喂一条事件，所以这条分支在重放里
            // 只会出现在「事件不改变状态」的那些帧上（那正是 dirty 判据的意义）。
            self.redraw_log.push(false);
            let stepped = self.step_script()?;
            println!(
                "redraw: no（状态没变，跳过这一帧；已画 {} 帧）",
                self.rendered
            );
            return match stepped {
                Some(Flow::Continue) => {
                    self.schedule_step();
                    Ok(Flow::Continue)
                }
                Some(Flow::Exit) => Ok(Flow::Exit),
                None => self.exit_or_quiet(),
            };
        }
        self.redraw_log.push(true);
        self.rendered += 1;
        self.dirty = false;

        // 脚本模式：**先喂一条事件、再画** —— 于是「输入 ⇒ 状态 ⇒ 这一帧画出来的东西」
        // 是同一条因果链（与真实窗口路径的区别只是「事件从哪来」：这里走 `Source::Script`，
        // 但落点是同一个 [`Form::handle_input`]）。
        let stepped = self.step_script()?;
        let f = self.frame();
        println!(
            "redraw: yes（第 {} 帧）{} 命令数={}",
            self.rendered,
            fmt_state(&self.state),
            f.list.len()
        );
        let mut engine = self.engine.take();
        let (outcome, skipped) = {
            let r = self.renderer.as_mut().ok_or("还没有窗口渲染器")?;
            let outcome = r.draw_and_present(&f.list, engine.as_mut());
            // **静默少画**的护栏：`ui_text_skipped` 是「因缺字/图集满而被跳过的字形数」。
            // 拿不到槽位的字形**不会报错**（M4 的既有语义）⇒ 不 print 出来就没人知道。
            (outcome, r.ui_text_skipped())
        };
        self.engine = engine;
        match outcome {
            Ok(FrameOutcome::Presented) => {}
            Ok(FrameOutcome::OutOfDate) => {
                let e = self.extent;
                if let Some(r) = self.renderer.as_mut() {
                    r.resize(e)
                        .map_err(|err| format!("交换链过期后重建失败：{err}"))?;
                }
                self.dirty = true;
            }
            Err(e) => return Err(format!("呈现失败：{e}")),
        }
        if let Some(last) = self.last_skipped {
            // 每帧**增量**（`ui_text_skipped` 是累计值）——「这一帧掉了几个字形」才是有用的数字。
            println!("           本帧被跳过的字形：{} 个（累计 {skipped}）", skipped - last);
        }
        self.last_skipped = Some(skipped);
        match stepped {
            Some(Flow::Continue) => {
                self.schedule_step();
                Ok(Flow::Continue)
            }
            Some(Flow::Exit) => Ok(Flow::Exit),
            None => self.exit_or_quiet(),
        }
    }

    fn close_requested(&mut self) -> Flow {
        Flow::Exit
    }
}

/// 内置脚本的期望终态（**写死** —— 判据不能拿实现当标准）。
///
/// 推导（逐步对应脚本 `move @button_1;down:left;up:left;key:Tab;text:hi`）：
///
/// | 语句 | 效果 |
/// |---|---|
/// | `move @button_1` | `hover = button_1` |
/// | `down:left` | `pressed = button_1`；命中的**可聚焦控件** ⇒ `focus = button_1` |
/// | `up:left` | 抬起处仍是 `button_1` ⇒ `Clicked`，且 `pressed` 清空 |
/// | `key:Tab` | 焦点按**树序**移到下一个可聚焦控件 `field_1`（禁用按钮不在序列里） |
/// | `text:hi` | 焦点是启用的输入框 ⇒ `texts["field_1"] = "hi"` |
///
/// ⇒ 终态：`hover=button_1`、`focus=field_1`、`pressed=None`、`texts={field_1: "hi"}`。
fn expected_state() -> UiState {
    UiState {
        hover: Some("button_1".into()),
        focus: Some("field_1".into()),
        pressed: None,
        texts: std::collections::BTreeMap::from([("field_1".to_string(), "hi".to_string())]),
        carets: Default::default(),
        scroll: Default::default(),
        preedit: Default::default(),
    }
}

/// 离屏的「四档状态」像素数字（不需要 GPU：这是 CPU 后端上的**结构性**判据）。
///
/// 为什么放在这里而不是放在测试里：本示例已经握有「真实布局 + 真实列表 + 状态」这套
/// 组装方式，把它跑成数字比在测试里重写一遍更可信。**GPU 侧**的逐像素对照在
/// `tests/interactive_form.rs`（那才是「GPU == CPU」的判据）。
fn pixel_numbers(theme: &Theme, extent: Extent) -> Result<(), String> {
    let tree = app_tree();
    let idle_state = UiState::default();
    let idle_frame = Frame::build(&tree, theme, extent, &idle_state, &ApproxMeasure);
    let idle = CpuRenderer::new()
        .render(extent, &idle_frame.list, CLEAR)
        .map_err(|e| format!("idle: CPU 渲染失败：{e}"))?
        .pixels;
    let btn = idle_frame
        .rect_of("button_1")
        .ok_or("布局没有给 button_1 几何")?;
    println!();
    println!("—— 四档状态的像素数字（CPU 后端，{}×{}，清屏 {CLEAR:?}）——", extent.width, extent.height);
    println!("button_1 矩形 = {btn:?}");

    let cases: Vec<(&str, UiState)> = vec![
        ("idle", UiState::default()),
        (
            "hover",
            UiState {
                hover: Some("button_1".into()),
                ..Default::default()
            },
        ),
        (
            "pressed",
            UiState {
                hover: Some("button_1".into()),
                pressed: Some("button_1".into()),
                ..Default::default()
            },
        ),
        (
            "focused",
            UiState {
                focus: Some("button_1".into()),
                ..Default::default()
            },
        ),
    ];
    let mut seen: Vec<(&str, Vec<u8>)> = Vec::new();
    for (name, st) in &cases {
        let f = Frame::build(&tree, theme, extent, st, &ApproxMeasure);
        let px = CpuRenderer::new()
            .render(extent, &f.list, CLEAR)
            .map_err(|e| format!("{name}: CPU 渲染失败：{e}"))?
            .pixels;
        let (inside, outside) = diff_split(&idle, &px, extent, btn);
        println!(
            "  {name:<8} 命令 {:<3} 文本命令 {}｜与 idle 差异：矩形内 {inside} 字节、矩形外 **{outside}** 字节",
            f.list.len(),
            f.list.cmds.iter().filter(|c| matches!(c, DrawCmd::Text { .. })).count()
        );
        if outside != 0 {
            return Err(format!(
                "{name}: 有 {outside} 字节差异落在 button_1 的 {btn:?} **之外** ⇒ 状态视觉溢出了"
            ));
        }
        if name != &"idle" && inside == 0 {
            return Err(format!("{name}: 按钮矩形内没有差异 ⇒ 这个状态根本没画出来"));
        }
        // 反向自检：与已经看过的**别的**状态必须不同（否则状态之间不可区分）。
        for (prev, ppx) in &seen {
            if prev != name && *ppx == px {
                return Err(format!("{name} 与 {prev} 的像素完全相同 ⇒ 两档状态不可区分"));
            }
        }
        seen.push((name, px));
    }
    println!("四档状态：差异全部落在 button_1 矩形内（越界字节 = 0），且四档彼此不同 ✅");
    Ok(())
}

fn main() -> ExitCode {
    let theme = Theme::default();
    let script_src = std::env::var(ENV_VAR).ok().filter(|s| !s.trim().is_empty());
    // M5c 省电重放档：**先 `trim()` 再比**（`cmd` 的 `set X=1 && …` 会把空格算进值里）。
    let ondemand = deer_gui::env_gate::flag(ONDEMAND_ENV);
    match &script_src {
        Some(_) => println!("[{ENV_VAR}] 已给出 ⇒ 脚本化重放（重放期间忽略窗口事件）"),
        None => {
            println!("没有设 {ENV_VAR} ⇒ 交互模式：自己点、按 Tab 移焦点、打字、Esc 退出");
            println!("             内置脚本（会自动重放）：{BUILTIN_SCRIPT}");
        }
    }
    if ondemand {
        println!(
            "[interactive_form] 省电重放档：{ONDEMAND_ENV} 已设 ⇒ OnDemand + Waker（deadline）推进\
             （M5b 时这条在接口上做不到：那时窗口层没有「自己唤醒事件循环」的手段）"
        );
    }
    if !window_tests_enabled() {
        // 与既有窗口类 example 同一约定：**退出码 0 但明确写着「这不是通过」**。
        println!("⚠️ **这不是通过，是被跳过**：没有设 DEER_VK_WINDOW_TESTS=1（本示例要真窗口）");
    }

    let app = Form {
        theme: theme.clone(),
        extent: Extent {
            width: WIDTH,
            height: HEIGHT,
        },
        state: UiState::default(),
        dirty: false,
        redraw_log: Vec::new(),
        queue: VecDeque::new(),
        fed_changed: Vec::new(),
        fed_events: 0,
        expected: None,
        rendered: 0,
        last_skipped: None,
        engine: None,
        renderer: None,
        script_src,
        done: false,
        ondemand,
        waker: None,
        wake_after_calls: 0,
        quiet: None,
        wants_asked: std::cell::Cell::new(0),
        quiet_inputs: 0,
        quiet_interruptions: 0,
    };
    let result = run(
        WindowConfig::new("M5-4 交互闭环（输入 → 命中 → 状态 → 重绘）", WIDTH, HEIGHT),
        app,
    );
    // 离屏像素数字：**在窗口生命周期之外**跑（不占窗口，也不需要 GPU）。
    let pixels = pixel_numbers(&theme, Extent { width: WIDTH, height: HEIGHT });
    match (result, pixels) {
        (Ok(()), Ok(())) => {
            println!("interactive_form 通过 ✅（脚本重放断言 + 四档像素数字）");
            ExitCode::SUCCESS
        }
        (Err(e), _) => {
            eprintln!("interactive_form 失败（窗口路径）：{e}");
            ExitCode::FAILURE
        }
        (_, Err(e)) => {
            eprintln!("interactive_form 失败（像素数字）：{e}");
            ExitCode::FAILURE
        }
    }
}
