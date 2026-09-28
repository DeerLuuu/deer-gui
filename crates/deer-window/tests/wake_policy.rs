//! M5c 的**唤醒面**单测：不建窗口、不跑事件循环（真开窗的验证在
//! `examples/wake_probe.rs` 与 `examples/idle_probe.rs` 里 —— CI 无桌面时真开窗会假红）。
//!
//! 验收要的东西在这里用**数**证明（不用 fps，也不靠「看起来不卡」）：
//!
//! 1. **收敛计划**（[`deer_window::plan_wake`]）：没有 deadline ⇒ `Wait`（**不造超时**，
//!    这就是省电模式的逻辑形态）；未来的 ⇒ `WaitUntil(那个时刻)`；到点/过期的 ⇒ `Due`；
//! 2. **唤醒账本**（[`deer_window::WakeStats`]）：`wake()` 与输入**同一把尺**（答真才请求）、
//!    `wake_after()` 只装 deadline，**deadline 到点 ⇒ `fired`/`requested` 同步 +1**；
//! 3. **空闲零唤醒**：没有任何 deadline 时，跑 10000 轮「事件循环迭代」⇒ **一次都不唤醒、
//!    一次都不请求重绘**（省电护栏的**纯逻辑**形态：它抓「凭空超时」这类退化）；
//! 4. **K 次 `wake_after` ⇒ 恰好 K 次到点 + K 帧**（可数，且与推/拉两条路都对得上）；
//! 5. **接口冻结**：M5c 之前写的 `App`（只实现 `init`/`redraw`）拿到的默认值是
//!    `next_deadline() == None`（= 不装 deadline ⇒ 省电），一行都不用改。
//!
//! **共享判据**：这里的 `WakeHarness` 复刻 `RunHandler` 的唤醒接线（`user_event` /
//! `about_to_wait` → `refresh_control_flow`），但它调用的是**产品代码里那两个纯函数**
//! （[`plan_wake`] / [`WakeStats`]）⇒ 数与 `run()` 数的是同一份实现，不是另写一套
//! 「测试专用逻辑」（那种测试只会自证自己）。
//!
//! ⚠️ **本文件管不到的东西（M5c 复审 I1 的教训 —— 别再写「`iters` 有单测」）**：
//!
//! - `WakePlan → ControlFlow` 的**翻译**在 `RunHandler::refresh_control_flow` 里，而 harness
//!   是**复刻** ⇒ 把 `Wait` 那一支改成 `Poll`（复审实测：`iters` 5 → **11 695 034**、CPU 烧满
//!   一核、退出码仍 0）**本文件一条都抓不住**。抓它的是 `src/lib.rs` 末尾的
//!   `wake_policy_control_flow_mapping` 单测（直接钉住那次翻译）；
//! - `iters` 的**运行期上界**只在真窗口里量得到 ⇒ 抓它的是 `examples/wake_probe.rs` 的
//!   **空闲档**（`DEER_WAKE_TICKS=0`：断言 `iters ∈ [1, 32]`，账本经 `App::on_wake_stats`
//!   交回 App）。本文件**没有**读 `iters` 的判据 —— 曾有的那条
//!   `assert_eq!(iters, IDLE_ITERS)` 是**同义反复**（harness 自己循环 N 次调 `note_iter`，
//!   再断言等于 N），已按复审删掉。

use std::time::{Duration, Instant};

use deer_window::{
    App, Flow, WakePlan, WakeStats, Waker, WindowInfo, earliest, plan_wake,
};

/// 测试用 App：`wants_redraw` 由一个开关控制；`next_deadline` 报一个**可设的状态**
/// （不是 `Instant::now() + …` —— 那正是接口文档里 ⚠️ 禁掉的写法）。
#[derive(Default)]
struct ProbeApp {
    want: bool,
    deadline: Option<Instant>,
}

impl ProbeApp {
    fn with_want(want: bool) -> ProbeApp {
        ProbeApp { want, deadline: None }
    }
}

impl App for ProbeApp {
    fn init(&mut self, _info: &WindowInfo) -> Result<(), String> {
        Ok(())
    }

    fn redraw(&mut self) -> Result<Flow, String> {
        Ok(Flow::Continue)
    }

    fn wants_redraw(&self) -> bool {
        self.want
    }

    fn next_deadline(&self) -> Option<Instant> {
        self.deadline
    }
}

/// `RunHandler` 唤醒接线的**无窗口**复刻：`arm` = `user_event(Wake::At(…))`、
/// `look` = `user_event(Wake::Look)`、`refresh` = `refresh_control_flow`（`about_to_wait`
/// 每轮都调它）、`deliver` = `RedrawRequested` 到达（**只有有人请求过才会来**）。
struct WakeHarness {
    app: ProbeApp,
    stats: WakeStats,
    /// 由 `wake_after` 推来的 deadline（`refresh` 兑现后清掉 —— 与产品代码同规则）。
    armed: Option<Instant>,
    /// 有没有**尚未兑现**的重绘请求：复刻 winit 的语义（没人 `request_redraw()`
    /// 就不会有 `App::redraw`）—— 少了这一条，测试会自己去画帧，测出来的是假的。
    pending: bool,
}

impl WakeHarness {
    fn new(app: ProbeApp) -> WakeHarness {
        WakeHarness { app, stats: WakeStats::new(), armed: None, pending: false }
    }

    /// `user_event(Wake::At(t))`：**只装 deadline，不画**。
    fn arm(&mut self, at: Instant) {
        self.stats.on_arm();
        self.armed = earliest(self.armed, Some(at));
    }

    /// `user_event(Wake::Look)`：**与输入同一把尺** —— 问 `wants_redraw`，答真才请求。
    fn look(&mut self) {
        if self.stats.on_look(self.app.wants_redraw()) {
            self.pending = true;
        }
    }

    /// `refresh_control_flow`：按 [`plan_wake`] 行动（到点 ⇒ 记一次 + 无条件请求一帧）。
    fn refresh(&mut self, now: Instant) -> WakePlan {
        self.stats.note_iter();
        let plan = plan_wake(now, self.armed, self.app.next_deadline());
        match plan {
            WakePlan::Wait | WakePlan::WaitUntil(_) => {}
            WakePlan::Due => {
                if self.armed.is_some_and(|t| t <= now) {
                    self.armed = None;
                }
                self.stats.on_fire();
                self.pending = true;
            }
        }
        plan
    }

    /// `RedrawRequested` 到达：没有未兑现的请求 ⇒ 这一帧**不会来**（返回 `false`）。
    fn deliver(&mut self) -> bool {
        if !self.pending {
            return false;
        }
        self.pending = false;
        true
    }
}

// ——————————————— 1. 收敛计划（真值表）———————————————

/// `plan_wake` 的真值表：**没有任何 deadline ⇒ `Wait`**（省电模式的逻辑形态）、
/// 未来的 ⇒ `WaitUntil`、到点或过期 ⇒ `Due`；两个来源取**更近**的那个。
#[test]
fn plan_wake_truth_table() {
    let now = Instant::now();
    let past = now - Duration::from_millis(1);
    let soon = now + Duration::from_millis(10);
    let later = now + Duration::from_millis(100);

    // 都没有 ⇒ **睡死**（不造超时）。
    assert_eq!(plan_wake(now, None, None), WakePlan::Wait);
    // 只有一个（推式/拉式各来一遍）。
    assert_eq!(plan_wake(now, Some(soon), None), WakePlan::WaitUntil(soon));
    assert_eq!(plan_wake(now, None, Some(soon)), WakePlan::WaitUntil(soon));
    // 到点 / 过期 ⇒ Due（「过期」也算到点，见 `App::next_deadline` 的 ⚠️）。
    assert_eq!(plan_wake(now, Some(now), None), WakePlan::Due, "正好到点 ⇒ Due");
    assert_eq!(plan_wake(now, Some(past), None), WakePlan::Due, "过期 ⇒ Due（视为现在）");
    assert_eq!(plan_wake(now, None, Some(past)), WakePlan::Due);
    // 两个来源 ⇒ 取更近的：早的先到点，晚的那个不该把唤醒推后。
    assert_eq!(plan_wake(now, Some(soon), Some(later)), WakePlan::WaitUntil(soon));
    assert_eq!(plan_wake(now, Some(later), Some(soon)), WakePlan::WaitUntil(soon));
    // 其中一个已经到点 ⇒ Due（不能被另一个未来的 deadline 掩掉）。
    assert_eq!(plan_wake(now, Some(past), Some(later)), WakePlan::Due);
    assert_eq!(plan_wake(now, Some(later), Some(past)), WakePlan::Due);
}

/// `earliest` 的语义：`None` 不参与比较；两个都 `None` ⇒ `None`。
#[test]
fn earliest_ignores_none_and_takes_the_sooner() {
    let a = Instant::now();
    let b = a + Duration::from_millis(5);
    assert_eq!(earliest(None, None), None);
    assert_eq!(earliest(Some(a), None), Some(a));
    assert_eq!(earliest(None, Some(a)), Some(a));
    assert_eq!(earliest(Some(a), Some(b)), Some(a));
    assert_eq!(earliest(Some(b), Some(a)), Some(a));
    assert_eq!(earliest(Some(a), Some(a)), Some(a), "相等时取谁都一样（但不能丢）");
}

// ——————————————— 2. 唤醒账本（计数语义）———————————————

/// `WakeStats` 的计数语义：`wake()` 与输入同一把尺（答真才请求、答假记一次「唤醒被跳过」）；
/// `wake_after()` 只装 deadline（`arms`），**不**请求；deadline 到点 ⇒ `fired` 与 `requested`
/// **同步 +1**（这条不变量不许漂）；`iters` 只由 `note_iter` 加。
#[test]
fn wake_stats_counts_each_thing_separately() {
    let mut s = WakeStats::new();
    assert_eq!((s.looks(), s.arms(), s.fired(), s.requested(), s.skipped(), s.iters()), (0, 0, 0, 0, 0, 0));

    // wake()：答假 ⇒ 不请求，记一次 skipped。
    assert!(!s.on_look(false), "答假 ⇒ 返回 false（调用方据此不请求）");
    assert_eq!((s.looks(), s.requested(), s.skipped()), (1, 0, 1));

    // wake()：答真 ⇒ 请求（requested +1、skipped 不动）。
    assert!(s.on_look(true), "答真 ⇒ 返回 true");
    assert_eq!((s.looks(), s.requested(), s.skipped()), (2, 1, 1));

    // wake_after()：只装 deadline —— **不请求**（到点才请求）。
    s.on_arm();
    s.on_arm();
    assert_eq!((s.arms(), s.requested()), (2, 1), "装 deadline 本身不请求重绘");

    // 到点：fired 与 requested 同步 +1。
    s.on_fire();
    s.on_fire();
    s.on_fire();
    assert_eq!((s.fired(), s.requested()), (3, 4), "fired ⇒ 每次都请求：3 次到点 ⇒ 请求 3+1=4 次");
    assert_eq!(s.fired(), 3, "到点次数就是唤醒次数（不受 wake() 影响）");

    // `iters` 只由 `note_iter` 加，且**不碰**其它账（下面这行把其余五个数一起钉住）。
    // ⚠️ 这不是「防空转判据」：它只证明计数器接线对（M5c 复审 I1 把那句 overclaim 删了）。
    for _ in 0..7 {
        s.note_iter();
    }
    assert_eq!(s.iters(), 7);
    assert_eq!((s.looks(), s.arms(), s.fired(), s.requested(), s.skipped()), (2, 2, 3, 4, 1));
}

/// 「`fired` 与 `requested` 同步」在**任意**序列下都成立（这里用一条只到点的序列钉死它）。
#[test]
fn fired_and_requested_never_drift() {
    let mut s = WakeStats::new();
    for i in 1..=25u64 {
        s.on_fire();
        assert_eq!(s.fired(), i);
        assert_eq!(s.requested(), i, "第 {i} 次到点之后 requested 必须 == fired（只算唤醒面的请求）");
    }
}

// ——————————————— 3. 空闲零唤醒（省电护栏的纯逻辑形态）———————————————

/// **空闲零唤醒**：没有任何 deadline（App 也没声明）时，跑 [`IDLE_ITERS`] 轮事件循环
/// ⇒ **一次都不唤醒、一次都不请求重绘**，而且每一轮收敛到的都是 [`WakePlan::Wait`]。
///
/// 这个测试就是「省电模式不许被唤醒面削弱」的**纯逻辑**护栏：把 `plan_wake` 的
/// `None ⇒ Wait` 改成「凭空来个 1ms 超时」⇒ 这个测试立刻红（`Wait` 那一支的断言先炸）。
#[test]
fn idle_plans_no_wakeup_at_all() {
    /// 空闲轮数：取 10000 —— 一二百轮看不出「一直都没醒」，而这个数足以让任何「偶尔超时」
    /// 的实现露馅（并且它跑起来是纯计算，毫秒级）。
    const IDLE_ITERS: u64 = 10_000;

    let mut h = WakeHarness::new(ProbeApp::with_want(false));
    let base = Instant::now();
    for i in 0..IDLE_ITERS {
        // 墙钟往前走（模拟时间流逝）：没有任何 deadline 时，这不该改变任何东西。
        let now = base + Duration::from_micros(i);
        assert_eq!(h.refresh(now), WakePlan::Wait, "第 {i} 轮：没有 deadline ⇒ 必须 Wait（睡死）");
    }
    // 🚫 **不要在这里断言 `iters == IDLE_ITERS`**（M5c 复审 I1）：harness 每轮自己调
    //    `note_iter`，再断言它等于轮数 ⇒ **同义反复**（只有删掉 `note_iter` 才会红，与
    //    「有没有空转」无关）。`iters` 的真判据在真窗口那边：`examples/wake_probe.rs` 的
    //    空闲档断言上界；`WakePlan ⇒ ControlFlow` 的翻译由 `src/lib.rs` 的单测钉住。
    //    这里真正有判别力的是**上面**每条 `refresh` 的返回值断言（`plan_wake` 真值表）
    //    与下面「零唤醒 / 零请求 / 一帧都不画」。
    assert_eq!(h.stats.fired(), 0, "空闲期**零**唤醒");
    assert_eq!(h.stats.requested(), 0, "空闲期**零**重绘请求");
    assert_eq!(h.stats.looks(), 0);
    assert_eq!(h.stats.arms(), 0);
    assert_eq!(h.stats.skipped(), 0, "没叫过 wake() ⇒ 也没有「唤醒被跳过」");
    assert!(!h.deliver(), "没人请求重绘 ⇒ winit 不会送 RedrawRequested ⇒ 一帧都不画");
}

/// **反向自检**（同一个测试里就能看出「凭空超时」长什么样）：若 App 每次都报一个**过去的**
/// deadline，收敛结果会**每一轮都是 `Due`** ⇒ 每轮请求一帧。
///
/// 这不是本层的 bug，是 App **自己要求**连续重绘（接口文档的 ⚠️ 2 写明了）。把它钉在这里，
/// 是为了让「`Due` 是有条件发生的」这件事**可观测**：空闲档（上一个测试）零请求，
/// 而这一档每轮都请求 —— 两者差别只来自 deadline。
#[test]
fn a_deadline_stuck_in_the_past_asks_for_a_frame_every_round() {
    let mut h = WakeHarness::new(ProbeApp::with_want(false));
    let now = Instant::now();
    h.app.deadline = Some(now - Duration::from_millis(1));
    for i in 0..5 {
        assert_eq!(h.refresh(now), WakePlan::Due, "第 {i} 轮：过期的 deadline ⇒ 立刻到点");
    }
    assert_eq!(h.stats.fired(), 5, "5 轮 ⇒ 5 次到点（App 自己要求的，账本上看得见）");
}

// ——————————————— 4. K 次 wake_after ⇒ K 次到点 + K 帧 ————————————————

/// **推式**：`wake_after` 排 K 次 ⇒ **恰好 K 次到点、K 帧**（不是 K-1，也不是 K+1）。
///
/// 时序按真实用法走：第 i 次「装 deadline」→ 时间走到那一刻 → 收敛 ⇒ `Due` ⇒ 一帧。
#[test]
fn k_wake_after_gives_exactly_k_fires_and_k_frames() {
    /// 定时唤醒次数。
    const K: u64 = 7;
    /// 每次的间隔。
    const STEP: Duration = Duration::from_millis(50);

    let mut h = WakeHarness::new(ProbeApp::with_want(false));
    let mut now = Instant::now();
    for i in 0..K {
        // 装一次 deadline（`Waker::wake_after`），它**不该**当场画帧。
        h.arm(now + STEP);
        assert!(!h.deliver(), "第 {i} 次装 deadline 之后不该已经有帧");
        // 中间「睡」过去：这一段里没有任何唤醒（这正是省电的意义）。
        now += STEP / 2;
        assert_eq!(h.refresh(now), WakePlan::WaitUntil(now + STEP / 2), "没到点 ⇒ 继续睡到那一刻");
        assert_eq!(h.stats.fired(), i, "还没到点 ⇒ 不该有到点计数");
        // 到点。
        now += STEP / 2;
        assert_eq!(h.refresh(now), WakePlan::Due, "到点 ⇒ Due");
        assert_eq!(h.stats.fired(), i + 1);
        assert!(h.deliver(), "到点必然请求过一帧 ⇒ 这一帧必须来");
    }
    assert_eq!(h.stats.arms(), K, "装了 {K} 次 deadline");
    assert_eq!(h.stats.fired(), K, "恰好 {K} 次到点");
    assert_eq!(h.stats.requested(), K, "唤醒面恰好请求 {K} 次（fired 与 requested 同步）");
    assert_eq!(h.stats.skipped(), 0, "定时档不该有「唤醒被跳过」");
    assert_eq!(h.stats.looks(), 0, "一次 wake() 都没叫");
    // 到点之后**没有**残留的 deadline ⇒ 收敛回 Wait（不空转）。
    assert_eq!(h.refresh(now + Duration::from_secs(1)), WakePlan::Wait);
}

/// **拉式**（`App::next_deadline`）：同样 K 次 ⇒ K 次到点 + K 帧，而且 `arms == 0`
/// （`wake_after` 一次都没调）—— 两条路都真的能推。
#[test]
fn k_pull_deadlines_give_exactly_k_fires_and_k_frames() {
    const K: u64 = 4;
    const STEP: Duration = Duration::from_millis(30);

    let mut h = WakeHarness::new(ProbeApp::with_want(false));
    let mut now = Instant::now();
    for i in 0..K {
        // App 声明「下一帧要在这个时刻」（拉式；**固定时刻**，每次都是新算的那个）。
        h.app.deadline = Some(now + STEP);
        assert_eq!(h.refresh(now), WakePlan::WaitUntil(now + STEP));
        now += STEP;
        assert_eq!(h.refresh(now), WakePlan::Due, "第 {i} 次到点");
        assert!(h.deliver());
        // App 在「画完这一帧」时把声明往后推（真实实现就是这个写法）。
        h.app.deadline = None;
    }
    assert_eq!(h.stats.fired(), K);
    assert_eq!(h.stats.requested(), K);
    assert_eq!(h.stats.arms(), 0, "拉式不经过 wake_after ⇒ arms 必须是 0（它是一个可数证据）");
    // App 撤回声明 ⇒ 收敛回 Wait（**这才是「拉式必须自己往前走」的可观测形态**）。
    assert_eq!(h.refresh(now + Duration::from_secs(1)), WakePlan::Wait);
}

// ——————————————— 5. wake() 与「答真才画」— 以及接口冻结 ———————————————

/// `wake()`（`Wake::Look`）：**答真才画**。50 声里只有第 50 声答真 ⇒ 只 1 帧，
/// 另外 49 声记成「唤醒被跳过」（**不**混进输入的「跳过帧」口径 —— 那是另一本账）。
#[test]
fn wake_only_paints_when_the_app_says_it_is_dirty() {
    let mut h = WakeHarness::new(ProbeApp::with_want(false));
    for _ in 0..49 {
        h.look();
    }
    assert!(!h.deliver(), "App 一直答假 ⇒ 一声都没换来帧");
    assert_eq!((h.stats.looks(), h.stats.skipped(), h.stats.requested()), (49, 49, 0));

    h.app.want = true; // 状态真的变了
    h.look();
    assert_eq!((h.stats.looks(), h.stats.skipped(), h.stats.requested()), (50, 49, 1));
    assert!(h.deliver(), "答真 ⇒ 请求了一帧，这一帧必须来");
    assert_eq!(h.stats.fired(), 0, "`wake()` 不是 deadline ⇒ 到点计数必须是 0（两本账不许混）");
}

/// **接口冻结**（M5c 的硬要求）：只实现 `init`/`redraw` 的 App（M5b 及之前的写法）
/// 拿到的默认值是 `next_deadline() == None` —— 也就是**不装 deadline、保持省电**，
/// 一行都不用改。（`wake_handle` 的默认实现同样什么都不做；它需要一个真 `Waker` 才能调，
/// 所以那一条的证据在真窗口的 `idle_probe`/`window_smoke` 里：它们没实现它，行为没变。）
#[test]
fn legacy_apps_get_a_quiet_default() {
    struct Legacy;

    impl App for Legacy {
        fn init(&mut self, _info: &WindowInfo) -> Result<(), String> {
            Ok(())
        }
        fn redraw(&mut self) -> Result<Flow, String> {
            Ok(Flow::Continue)
        }
    }

    let legacy = Legacy;
    assert_eq!(legacy.next_deadline(), None, "默认不声明 deadline（省电）");
    assert!(!legacy.wants_redraw(), "默认不请求重绘（省电）");
}

/// **编译期断言**：`Waker` 必须是 `Send`（可以搬到别的线程里叫醒事件循环）——
/// `examples/wake_probe.rs` 真的把它送进了别的线程（走 `mpsc` 通道，通道要求 `T: Send`）。
///
/// 依据是 winit 0.30.13 的 Windows 后端那句
/// `unsafe impl<T: Send + 'static> Send for EventLoopProxy<T>`（源码行号见 `lib.rs` 的注释）。
/// **`Sync` 不断言**：本层不依赖它（理由写在 [`Waker`] 的文档里）。
#[test]
fn waker_is_send() {
    fn assert_send<T: Send>() {}
    assert_send::<Waker>();
}
