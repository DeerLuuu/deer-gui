//! M5b 的**重绘策略**单测：不建窗口、不跑事件循环（真开窗的验证在 `examples/idle_probe.rs`
//! 与 `examples/window_smoke.rs` 里 —— CI 无桌面时真开窗会假红）。
//!
//! 验收要的东西全在这里用**数**证明（不用 fps，也不靠「看起来不卡」）：
//!
//! 1. **真值表**：`DEER_WINDOW_REDRAW` 的判定，含 `"continuous"` / `"CONTINUOUS"` /
//!    `" continuous "`（`cmd` 的 `set X=… && …` 会带尾空格）/ `"on-demand"` / `""`；
//! 2. **空闲探针**：连续喂 K 条**不改变状态**的事件 ⇒ `wants_redraw()` 假 ⇒ **请求重绘 0 次**
//!    （可观测计数：`redraw_requests` 不变、`skipped_frames == K`）；
//! 3. **变化探针**：`wants_redraw()` 真 ⇒ 请求重绘**恰好一次**；
//! 4. **策略**：`OnDemand` 画完就停；`Continuous` 每画完一帧续下一帧（请求数 = 帧数 + 1）。
//!
//! **共享判据**：`Harness` 复刻 `run()` 的接线，但它数的是 **`FrameCounter`**（`on_input` /
//! `note_request` / `on_redraw`）与 **`RedrawPolicy::wants_next_frame`** —— 就是 `run()` 用的那一份，
//! 不是另写一套「测试专用逻辑」（那种测试只会自证自己）。

use deer_gpu::Extent;
use deer_window::{
    App, Flow, FrameCounter, InputEvent, Key, Mods, PointerButton, REDRAW_ENV, RedrawPolicy,
    WindowInfo, parse_redraw_policy, raw_handle_from_win32, resolve_redraw_policy,
};

/// 喂多少条空闲事件（K）。取 50：一条两条看不出「连续喂也不请求」。
const IDLE_EVENTS: u64 = 50;

/// 测试用 App：**只有 `KeyDown` 会改状态**。
///
/// 这样「空闲事件」与「变化事件」是同一个 App 上的两种输入 —— 空闲探针与变化探针的差别
/// 只来自事件本身，不来自两个不同的实现（免得两个探针各测各的）。
struct ProbeApp {
    /// 脏标记：由 `input` 置，由 [`App::wants_redraw`] 读，由 `redraw` 清。
    dirty: bool,
    /// 真画过的帧数（App 自己数的，用来跟账本交叉核对）。
    redraws: u64,
    /// 声明的策略（`Continuous` 用来验证「每帧续一帧」）。
    declared: RedrawPolicy,
}

impl ProbeApp {
    fn new(declared: RedrawPolicy) -> ProbeApp {
        ProbeApp { dirty: false, redraws: 0, declared }
    }
}

impl App for ProbeApp {
    fn init(&mut self, _info: &WindowInfo) -> Result<(), String> {
        Ok(())
    }

    fn redraw(&mut self) -> Result<Flow, String> {
        self.redraws += 1;
        // 画完这一帧，界面就跟状态一致了 ⇒ 清脏标记。
        // 不清的话「改了状态」会变成永久为真 ⇒ 每条输入都请求重绘（省电白搭）。
        self.dirty = false;
        Ok(Flow::Continue)
    }

    fn input(&mut self, _info: &WindowInfo, ev: &InputEvent) -> Result<Flow, String> {
        if matches!(ev, InputEvent::KeyDown { .. }) {
            self.dirty = true;
        }
        Ok(Flow::Continue)
    }

    fn wants_redraw(&self) -> bool {
        self.dirty
    }

    fn redraw_policy(&self) -> RedrawPolicy {
        self.declared
    }
}

/// `run()` 接线的**无窗口**复刻：
///
/// - `feed` = `RunHandler::dispatch(.., Gate::AppDecides)`：派发 → 问 `wants_redraw` →
///   真则 `request_redraw()`（`Harness::request` 就是 `RunHandler::request_redraw` 的两句）；
/// - `system_event` = `Resized` / `Focused` / 窗口暴露 / 建窗引导帧：**一律**请求；
/// - `deliver_redraw` = `RedrawRequested` 分支：画一帧 → `OnDemand` 停 / `Continuous` 续。
struct Harness {
    app: ProbeApp,
    counter: FrameCounter,
    info: WindowInfo,
    /// **实际生效**的策略（App 声明 + 环境变量覆盖），与 `run()` 里算的是同一个函数。
    policy: RedrawPolicy,
    /// 有没有**尚未兑现**的重绘请求 —— 复刻 winit 的语义：`RedrawRequested` 只会因为
    /// 「有人 `request_redraw()`」（或系统要重画）才来；没人请求就**不会有** `App::redraw`。
    /// 少了这一条，测试就会自己去调 `redraw`（`OnDemand` 下凭空空转），测出来的东西是假的。
    pending: bool,
}

impl Harness {
    fn new(declared: RedrawPolicy, env_raw: Option<&str>) -> Harness {
        Harness {
            app: ProbeApp::new(declared),
            counter: FrameCounter::new(),
            info: WindowInfo {
                raw: raw_handle_from_win32(0x1A2B, 0x7FF6_0000),
                extent: Extent { width: 320, height: 200 },
                scale_factor: 1.0,
            },
            policy: resolve_redraw_policy(declared, env_raw),
            pending: false,
        }
    }

    /// `RunHandler::request_redraw()`：记账 + 请求（这里没有窗口可请求，账本照记）。
    fn request(&mut self) {
        self.counter.note_request();
        self.pending = true;
    }

    /// 派发一条**普通输入**（`Gate::AppDecides`）。
    fn feed(&mut self, ev: &InputEvent) {
        self.app.input(&self.info, ev).expect("测试 App 不报错");
        if self.counter.on_input(self.app.wants_redraw()) {
            self.request();
        }
    }

    /// **系统事件**（`Resized`/`Focused`/窗口暴露/建窗引导帧）：不看 `wants_redraw`，一律请求。
    fn system_event(&mut self) {
        self.request();
    }

    /// `RedrawRequested` 到达：画一帧，再按**生效策略**决定要不要续下一帧。
    ///
    /// 返回这一趟**真画了帧没有**：没有未兑现的请求 ⇒ winit 不会送这个事件 ⇒ `false`。
    fn deliver_redraw(&mut self) -> bool {
        if !self.pending {
            return false;
        }
        self.pending = false;
        self.counter.on_redraw(self.app.redraw()).expect("测试 App 不报错");
        if self.policy.wants_next_frame() {
            self.request();
        }
        true
    }
}

/// 一条「不改变状态」的空闲事件（hover）。
fn idle_event(i: u64) -> InputEvent {
    InputEvent::PointerMoved { x: i as f32, y: i as f32 }
}

/// 一条「改变状态」的事件。
fn changing_event() -> InputEvent {
    InputEvent::KeyDown { key: Key::Char('a'), mods: Mods::default(), repeat: false }
}

// ——————————————— 1. 真值表 ———————————————

/// `parse_redraw_policy` 的真值表：**先 `trim()`、大小写不敏感**，其余一律「没覆盖」。
///
/// 含事故档：`cmd` 的 `set DEER_WINDOW_REDRAW=continuous && …` 实测值是 `"continuous "`
/// （带尾空格）—— 严格比较会把「设了」判成「没设」⇒ 省电模式关不掉却照样 exit=0。
#[test]
fn parse_redraw_policy_truth_table() {
    let continuous = [
        "continuous",
        "CONTINUOUS",
        "Continuous",
        " continuous ",
        "\tcontinuous\t",
        "continuous ", // ← `cmd /c "set X=continuous && …"` 的实际值
        " continuous",
    ];
    for raw in continuous {
        assert_eq!(
            parse_redraw_policy(raw),
            Some(RedrawPolicy::Continuous),
            "{raw:?} 必须解析为 Continuous"
        );
    }

    let on_demand = ["on-demand", "ON-DEMAND", "On-Demand", " on-demand ", "demand", " Demand "];
    for raw in on_demand {
        assert_eq!(
            parse_redraw_policy(raw),
            Some(RedrawPolicy::OnDemand),
            "{raw:?} 必须解析为 OnDemand（不覆盖 App 的声明）"
        );
    }

    // 未设 / 空 / 纯空白 / 认不出来的值 ⇒ **没有覆盖**（`None`，不是「默认 OnDemand」）。
    for raw in ["", " ", "\t", "\n", "cont", "continuou", "1", "0", "yes", "true", "always", "连续"] {
        assert_eq!(parse_redraw_policy(raw), None, "{raw:?} 不该被认成任何策略");
    }
}

/// `resolve_redraw_policy` 的真值表：只有 `continuous` 是**无条件覆盖**，其余走 App 自己的声明。
#[test]
fn resolve_redraw_policy_truth_table() {
    use RedrawPolicy::{Continuous, OnDemand};

    let cases: &[(RedrawPolicy, Option<&str>, RedrawPolicy, &str)] = &[
        // App 声明 OnDemand（省电）——环境变量能把它打开
        (OnDemand, Some("continuous"), Continuous, "continuous 必须无条件强制 Continuous"),
        (OnDemand, Some("CONTINUOUS"), Continuous, "大小写不敏感"),
        (OnDemand, Some(" continuous "), Continuous, "两侧空白必须被 trim 掉"),
        (OnDemand, Some("continuous "), Continuous, "`cmd` 的尾空格档（事故现场）"),
        (OnDemand, Some("on-demand"), OnDemand, "显式 on-demand = 不覆盖"),
        (OnDemand, Some("demand"), OnDemand, "demand 是 on-demand 的别名"),
        (OnDemand, Some(""), OnDemand, "空值 = 未设"),
        (OnDemand, None, OnDemand, "未设 = 用 App 自己的策略"),
        (OnDemand, Some("nonsense"), OnDemand, "无法识别 = 用 App 自己的策略"),
        // App 声明 Continuous（要连续动画）——`on-demand` **不是**「把 App 按回按需」
        (Continuous, Some("on-demand"), Continuous, "on-demand 不覆盖 Continuous 声明"),
        (Continuous, Some("demand"), Continuous, "同上"),
        (Continuous, Some(""), Continuous, "空值不覆盖"),
        (Continuous, None, Continuous, "未设不覆盖"),
        (Continuous, Some("nonsense"), Continuous, "无法识别不覆盖"),
        (Continuous, Some("continuous"), Continuous, "声明 Continuous + 强制 Continuous"),
    ];
    for (app_policy, raw, expected, why) in cases {
        assert_eq!(
            resolve_redraw_policy(*app_policy, *raw),
            *expected,
            "App 声明={} 请求={raw:?} ⇒ 期望 {}：{why}",
            app_policy.label(),
            expected.label()
        );
    }
}

/// 环境变量名冻死成常量（自证日志、示例、单测共用同一个字符串；改名要一起改）。
#[test]
fn redraw_env_name_is_frozen() {
    assert_eq!(REDRAW_ENV, "DEER_WINDOW_REDRAW");
}

// ——————————————— 2. 空闲探针 ———————————————

/// **空闲探针**：连续喂 [`IDLE_EVENTS`] 条不改变状态的事件 ⇒ `wants_redraw()` 假 ⇒
/// **不请求重绘**（`redraw_requests` 一动不动）、**一帧都不画**，而「跳过」计数正好等于 K。
///
/// 这一个测试就是 M5b 的全部意义：旧版（`ControlFlow::Poll` + `about_to_wait` 里
/// `request_redraw`）在这些事件之间会一直空转重绘。
#[test]
fn idle_probe_k_events_request_no_redraw() {
    let mut h = Harness::new(RedrawPolicy::OnDemand, None);

    // 建窗后的**引导帧**：run() 会请求一次并画一帧（winit 对「建窗必发 RedrawRequested」
    // 没有保证 ⇒ 不主动要一帧的话窗口就一直是没画过的样子）。这是一次性的，不是空转。
    h.system_event();
    h.deliver_redraw();
    assert_eq!(h.counter.frames(), 1, "引导帧应当画成 1 帧");
    assert_eq!(h.counter.redraw_requests(), 1, "引导帧只请求一次");

    let requests_before = h.counter.redraw_requests();
    let frames_before = h.counter.frames();

    for i in 0..IDLE_EVENTS {
        h.feed(&idle_event(i));
    }

    assert!(!h.app.wants_redraw(), "空闲事件不该把 App 弄脏");
    assert_eq!(
        h.counter.redraw_requests(),
        requests_before,
        "{IDLE_EVENTS} 条空闲事件之后请求重绘次数**不许**增加"
    );
    assert_eq!(h.counter.frames(), frames_before, "没有请求 ⇒ 一帧都不该画");
    assert_eq!(
        h.counter.skipped_frames(),
        IDLE_EVENTS,
        "每一条空闲事件都该记一次「跳过帧」"
    );
}

// ——————————————— 3. 变化探针 ———————————————

/// **变化探针**：`wants_redraw()` 真 ⇒ 请求重绘**恰好一次**（不是两次、也不是每条事件一次），
/// 画完之后脏标记被清掉（否则会退化成「每条输入都重绘」）。
#[test]
fn change_probe_requests_exactly_one_redraw() {
    let mut h = Harness::new(RedrawPolicy::OnDemand, None);

    // 先来几条空闲的：它们只该记「跳过」，不请求。
    for i in 0..3 {
        h.feed(&idle_event(i));
    }
    assert_eq!(h.counter.redraw_requests(), 0);
    assert_eq!(h.counter.skipped_frames(), 3);

    // 一条改状态的输入 ⇒ 请求重绘。
    h.feed(&changing_event());
    assert!(h.app.wants_redraw(), "KeyDown 必须把 App 弄脏");
    assert_eq!(h.counter.redraw_requests(), 1, "变化事件应当请求**恰好一次**重绘");
    assert_eq!(h.counter.skipped_frames(), 3, "变化事件不算「跳过」");
    assert_eq!(h.counter.frames(), 0, "只是请求了，还没画");

    // 请求兑现成一帧。
    h.deliver_redraw();
    assert_eq!(h.counter.frames(), 1);
    assert_eq!(h.app.redraws, 1, "App 自己数的帧数要和账本一致");
    assert!(!h.app.wants_redraw(), "画完必须清脏标记");
    assert_eq!(h.counter.redraw_requests(), 1, "OnDemand：画完不再续下一帧");
}

/// **系统事件一律置位**：App 一点没脏（`wants_redraw() == false`）也照样请求重绘 ——
/// 尺寸变了/焦点变了/窗口重新暴露了，界面必须重画，这不是 App 的脏标记说了算的。
/// 同时它**不**该被算成「跳过帧」。
#[test]
fn system_events_always_request_redraw() {
    let mut h = Harness::new(RedrawPolicy::OnDemand, None);
    assert!(!h.app.wants_redraw());

    h.system_event();
    h.system_event();
    assert_eq!(h.counter.redraw_requests(), 2, "系统事件不看 wants_redraw");
    assert_eq!(h.counter.skipped_frames(), 0, "系统事件不是「跳过」");
}

// ——————————————— 4. 策略：OnDemand 停 / Continuous 续 ———————————————

/// `OnDemand`：画完就停 —— 再多的 `RedrawRequested` 也不会自己生出下一帧；
/// `Continuous`：每画完一帧续一帧 ⇒ N 帧对应 N+1 次请求（含最初那次）。
#[test]
fn on_demand_stops_but_continuous_keeps_asking() {
    // OnDemand：引导帧 + 1 次投递 ⇒ 只有 1 帧；之后再投递也**没人请求** ⇒ 一帧都不画。
    let mut idle = Harness::new(RedrawPolicy::OnDemand, None);
    idle.system_event();
    let mut drawn = 0;
    for _ in 0..5 {
        if idle.deliver_redraw() {
            drawn += 1;
        }
    }
    assert_eq!(drawn, 1, "OnDemand 下没人请求 ⇒ 只有引导帧那一帧");
    assert_eq!(idle.counter.frames(), 1);
    assert_eq!(idle.counter.redraw_requests(), 1);

    // Continuous：5 帧 + 每次都续 ⇒ 6 次请求。
    let mut cont = Harness::new(RedrawPolicy::Continuous, None);
    cont.system_event();
    let mut drawn = 0;
    for _ in 0..5 {
        if cont.deliver_redraw() {
            drawn += 1;
        }
    }
    assert_eq!(drawn, 5, "Continuous 下每帧都续上了下一帧");
    assert_eq!(cont.counter.frames(), 5);
    assert_eq!(cont.counter.redraw_requests(), 6, "每画完一帧续一帧：N 帧 ⇒ N+1 次请求");
}

/// **运行时开关**：`DEER_WINDOW_REDRAW=continuous` 把「声明省电」的 App **无条件**变成连续重绘；
/// 尾空格档（`cmd` 的实际值 `"continuous "`）必须同样生效。
#[test]
fn env_continuous_forces_redraw_for_on_demand_app() {
    for raw in ["continuous", "CONTINUOUS", " continuous ", "continuous "] {
        let mut h = Harness::new(RedrawPolicy::OnDemand, Some(raw));
        assert_eq!(h.policy, RedrawPolicy::Continuous, "{raw:?} 必须把策略强制成 Continuous");
        h.system_event();
        for _ in 0..3 {
            h.deliver_redraw();
        }
        assert_eq!(h.counter.frames(), 3, "{raw:?}：连续重绘下应当一直有下一帧");
        assert_eq!(h.counter.redraw_requests(), 4, "{raw:?}：3 帧 + 最初那次请求");
    }

    // 反例（防止「永远 Continuous」这种假通过）：不设变量时声明 OnDemand 的 App 仍然停。
    let mut h = Harness::new(RedrawPolicy::OnDemand, None);
    h.system_event();
    for _ in 0..3 {
        h.deliver_redraw();
    }
    assert_eq!(h.policy, RedrawPolicy::OnDemand);
    assert_eq!(h.counter.frames(), 1, "不设变量 ⇒ 省电模式生效（只画引导帧）");
}

// ——————————————— 5. 默认值（M5b 接口冻结：既有实现一行都不用改）———————————————

/// 只实现 `init`/`redraw` 的 App（M5b 之前的写法）拿到的默认值：
/// `wants_redraw() == false`、`redraw_policy() == OnDemand` —— 也就是**旧实现默认省电**。
#[test]
fn app_defaults_are_quiet_and_on_demand() {
    /// 刻意**不**实现 `wants_redraw` / `redraw_policy`：这条测试钉的就是默认值。
    struct Legacy;

    impl App for Legacy {
        fn init(&mut self, _info: &WindowInfo) -> Result<(), String> {
            Ok(())
        }
        fn redraw(&mut self) -> Result<Flow, String> {
            Ok(Flow::Continue)
        }
    }

    let app = Legacy;
    assert!(!app.wants_redraw(), "默认不请求重绘（省电）");
    assert_eq!(app.redraw_policy(), RedrawPolicy::OnDemand);
    assert_eq!(RedrawPolicy::default(), RedrawPolicy::OnDemand);
}

/// `RedrawPolicy` 的小接口：可读名（自证日志用）、`Copy`、`Continuous` 才续帧。
#[test]
fn redraw_policy_labels_and_next_frame() {
    assert_eq!(RedrawPolicy::OnDemand.label(), "OnDemand");
    assert_eq!(RedrawPolicy::Continuous.label(), "Continuous");
    assert!(!RedrawPolicy::OnDemand.wants_next_frame());
    assert!(RedrawPolicy::Continuous.wants_next_frame());

    let policy = RedrawPolicy::OnDemand; // Copy
    let copied = policy;
    assert_eq!(copied, RedrawPolicy::OnDemand);
    assert_ne!(RedrawPolicy::OnDemand, RedrawPolicy::Continuous);
}

// ——————————————— 6. 账本与「跳过」的语义钉死 ———————————————

/// `FrameCounter` 的三个计数语义（`run()` 的账本行就是这三个数）：
/// `on_input(false)` ⇒ 跳过 +1 且**不**返回「要请求」；`note_request` 只加请求数；
/// `on_redraw(Ok)` 只加帧数。三者互不代偿。
#[test]
fn frame_counter_counts_requests_skips_and_frames_separately() {
    let mut counter = FrameCounter::new();
    assert_eq!((counter.redraw_requests(), counter.skipped_frames(), counter.frames()), (0, 0, 0));

    assert!(!counter.on_input(false), "wants_redraw=false ⇒ 不请求");
    assert_eq!((counter.redraw_requests(), counter.skipped_frames(), counter.frames()), (0, 1, 0));

    assert!(counter.on_input(true), "wants_redraw=true ⇒ 要请求");
    assert_eq!(
        (counter.redraw_requests(), counter.skipped_frames(), counter.frames()),
        (0, 1, 0),
        "on_input 只做决策 + 记「跳过」；请求数由 note_request 记（单一真相来源）"
    );

    counter.note_request();
    counter.note_request();
    assert_eq!((counter.redraw_requests(), counter.skipped_frames(), counter.frames()), (2, 1, 0));

    counter.on_redraw(Ok(Flow::Continue)).expect("Continue 不是错误");
    assert_eq!((counter.redraw_requests(), counter.skipped_frames(), counter.frames()), (2, 1, 1));

    // 出错那一次不计数（与旧行为一致）。
    let mut failing = FrameCounter::new();
    failing.note_request();
    assert_eq!(failing.on_redraw(Err("交换链过期".to_string())).unwrap_err(), "交换链过期");
    assert_eq!((failing.redraw_requests(), failing.skipped_frames(), failing.frames()), (1, 0, 0));
}

/// 收尾核对：`input` 里**非键盘**的事件（点击、滚轮、指针抬起）都不算「改了状态」——
/// 这条是 [`ProbeApp`] 的契约，也是上面两个探针能成立的前提。
#[test]
fn probe_app_only_treats_keydown_as_state_change() {
    let mut h = Harness::new(RedrawPolicy::OnDemand, None);
    let events = [
        InputEvent::PointerMoved { x: 1.0, y: 1.0 },
        InputEvent::PointerDown { button: PointerButton::Left, x: 1.0, y: 1.0 },
        InputEvent::PointerUp { button: PointerButton::Left, x: 1.0, y: 1.0 },
        InputEvent::Wheel { dx: 0.0, dy: -1.0 },
        InputEvent::KeyUp { key: Key::Char('a'), mods: Mods::default() },
        InputEvent::TextInput { text: "hi".to_string() },
    ];
    for ev in &events {
        h.feed(ev);
        assert!(!h.app.wants_redraw(), "{ev:?} 在探针里应当算「没改状态」");
    }
    assert_eq!(h.counter.redraw_requests(), 0, "这些事件一条都不该请求重绘");
    assert_eq!(h.counter.skipped_frames(), events.len() as u64);

    h.feed(&changing_event());
    assert_eq!(h.counter.redraw_requests(), 1);
    assert_eq!(h.counter.skipped_frames(), events.len() as u64);
}
