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
//! # 关于「已渲染帧数」与窗口帧率（别误读这一条）
//!
//! `deer-window` 的事件循环是 `ControlFlow::Poll` + `about_to_wait` 里 `request_redraw()`
//! ⇒ `redraw()` 会被**持续**调用（见 `deer-window` 的模块文档）。所以「只在 dirty 时重绘」
//! 在真实窗口里的准确含义是：**每一帧都先查脏位，只有脏了才真的画**；不脏的帧只打印一行。
//! 真正的「事件驱动按需重绘」要改事件循环的控制流（`ControlFlow::Wait` + 有新事件才
//! `request_redraw`），那是窗口层的改动，本 example **不改**它（改了会让其它 example 的行为
//! 跟着变）。这不是缺陷，是当前窗口层的契约；dirty 账本本身是可断言的
//! （`tests/interactive_form.rs` 里有同一判据的纯逻辑版本）。

use std::collections::VecDeque;
use std::path::Path;
use std::process::ExitCode;

use deer_gui::gpu::interact::{FieldText, InteractState, InteractiveRenderer};
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
use deer_gui::window::{App, Flow, WindowConfig, WindowInfo, run};

/// 清屏色（离屏对照与上屏用同一个值）。
const CLEAR: Color = Color::rgb(0x08, 0x09, 0x0c);

/// 窗口尺寸（脚本里的坐标与它同一套物理像素口径）。
const WIDTH: u32 = 420;
const HEIGHT: u32 = 220;

/// 内置脚本（没设 `DEER_INPUT_SCRIPT` 时用它）—— 它**有期望终态**，所以是断言的一部分。
///
/// `move @button_1` 读作「指针移到 button_1 的中心」（坐标由布局算，不写死）。
const BUILTIN_SCRIPT: &str = "move @button_1;down:left;up:left;key:Tab;text:hi";

/// 门槛判定（`DEER_VK_WINDOW_TESTS` 有没有设）。
///
/// **为什么必须容忍两边空白**（实测数据，不是猜的）：`cmd` 的
/// `set DEER_VK_WINDOW_TESTS=1 && cargo run …` 会把 `&&` 前的空格也算进变量值 ——
/// `cmd /c "set DEER_VK_WINDOW_TESTS=1 && set DEER_VK_WINDOW_TESTS"` 实测打印
/// `DEER_VK_WINDOW_TESTS=1 `（**带一个尾空格**）；写成 `set …=1&& …`（不留空格）才是不带空格的 `1`。
/// 严格 `== "1"` 会把「**已经设了**门槛」判成「没设」⇒ 明明建了窗口、跑完了脚本、exit=0，
/// 却打印「这不是通过，是被跳过」，**与事实相反**。所以这里 `trim()` 后再比。
fn window_tests_enabled() -> bool {
    std::env::var("DEER_VK_WINDOW_TESTS")
        .map(|v| {
            let v = v.trim();
            v == "1" || v.eq_ignore_ascii_case("true")
        })
        .unwrap_or(false)
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
        let interact = InteractState {
            hover: state.hover.clone(),
            focus: state.focus.clone(),
            pressed: state.pressed.clone(),
        };
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

    /// 喂一条输入事件：`handle` + 「状态真的变了才置脏」。返回是否要退出。
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

    /// 喂**一条**脚本事件（`redraw` 每帧喂一条 ⇒ 事件 → 状态 → 重绘 的关系是可数的）。
    ///
    /// 为什么不用「一次把脚本喂完」：那样整段重放会被**一帧**画完（`redraw_log` 只有 1 个
    /// `true`），于是「输入 ⇒ 置脏 ⇒ 重绘」这条链在日志里根本看不出来 —— 日志看着像通过，
    /// 实际上什么都没证明。每帧一条事件，`redraw: yes/no` 的序列就是这条链的证据。
    fn step_script(&mut self) -> Result<Option<Flow>, String> {
        let Some(ev) = self.queue.pop_front() else {
            return Ok(None);
        };
        let before = self.state.clone();
        if self.feed(&ev) {
            // 脚本里写了 `key:Escape` ⇒ 与真实窗口按 Esc 同一条语义（退出），
            // 但收尾仍然要做（先打印终态、再断言），否则「脚本结束」没有可比对的数字。
            // 注意：这一帧**没有画过**任何输入（调用方拿到退出请求就直接返回）⇒ 不记账，
            // 所以它不会去和「重绘次数」对账。
            println!("脚本里的 Escape ⇒ 收尾并退出");
            self.finish_script()?;
            return Ok(Some(Flow::Exit));
        }
        // 记账：这一帧确实画了一条输入事件；它是否真的改了状态（= 是否该重绘那一帧）。
        self.fed_events += 1;
        self.fed_changed.push(self.state != before);
        Ok(Some(Flow::Continue))
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

    fn input(&mut self, _info: &WindowInfo, ev: &InputEvent) -> Result<Flow, String> {
        if self.done {
            return Ok(Flow::Exit);
        }
        if self.queue.is_empty() {
            // 交互模式（或脚本已经喂完）：窗口事件直接喂。
            // 脚本模式下队列被 `redraw` 推进，所以这里的事件只在交互模式出现。
            return Ok(if self.feed(ev) { Flow::Exit } else { Flow::Continue });
        }
        // 脚本模式下**忽略**窗口事件：重放必须确定性（跑脚本时动鼠标不该改结果）。
        println!("input: 脚本模式，忽略窗口事件 {ev:?}");
        Ok(Flow::Continue)
    }

    fn redraw(&mut self) -> Result<Flow, String> {
        if self.done {
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
                Some(flow) => Ok(flow),
                None => {
                    self.finish_script()?;
                    Ok(Flow::Exit)
                }
            };
        }
        self.redraw_log.push(true);
        self.rendered += 1;
        self.dirty = false;

        // 脚本模式：**先喂一条事件、再画** —— 于是「输入 ⇒ 状态 ⇒ 这一帧画出来的东西」
        // 是同一条因果链（与真实窗口路径的区别只是「事件从哪来」，`feed` 是同一份代码）。
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
            Some(flow) => Ok(flow),
            None => {
                self.finish_script()?;
                Ok(Flow::Exit)
            }
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
    match &script_src {
        Some(_) => println!("[{ENV_VAR}] 已给出 ⇒ 脚本化重放（重放期间忽略窗口事件）"),
        None => {
            println!("没有设 {ENV_VAR} ⇒ 交互模式：自己点、按 Tab 移焦点、打字、Esc 退出");
            println!("             内置脚本（会自动重放）：{BUILTIN_SCRIPT}");
        }
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
