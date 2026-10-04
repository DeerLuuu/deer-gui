//! # `deer-window` 的 L3 —— host（`App` 运行时：事件循环 / Waker / 帧调度 / 省电重绘）
//!
//! 分层归属见 `docs/ARCHITECTURE.md` §2.3：**L3 host = App 运行时（事件循环 + Waker + 脏重绘）**。
//! 本模块负责：
//!
//! - [`App`] trait 与它的回调时序（`init` / `resized` / `redraw` / `input` / `close_requested` /
//!   `wants_redraw` / `redraw_policy` / `wake_handle` / `next_deadline` / `on_wake_stats`）；
//! - [`run()`]：建 winit `EventLoop`（带**私有**用户事件）+ `ApplicationHandler` 接线 + 建窗 +
//!   跑循环 + 收尾打账本（「重绘账本」「唤醒账本」）；
//! - **帧调度**：置位六条规则（见 `lib.rs` 的 crate 文档表）+ [`RedrawPolicy`]（默认
//!   [`RedrawPolicy::OnDemand`] 省电）+ `DEER_WINDOW_REDRAW` 运行时覆盖；
//! - **唤醒面**：[`Waker`] / [`WakePlan`] / [`plan_wake`] / [`earliest`] / [`WakeStats`]、
//!   以及纯函数 `plan_to_control_flow`；
//! - **账本**：[`FrameCounter`]（帧 / 请求 / 跳过帧 / 退出标记）与 [`WakeStats`]；
//! - **多窗口**（T4.4-R1）：[`WindowId`] 事件路由 + [`WindowSpawner`] 排队建窗 +
//!   多 [`WindowConfig`] 启动 + 关闭语义（只关该窗；全关 ⇒ 退出）+ 每窗独立焦点
//!   （决策 1/2/3/4/5，见 `ROADMAP.md` 的「设计登记：多窗口（T4.4）」）。
//!
//! **平台面**（窗口句柄 / winit 输入翻译 / DPI）在 [`crate::display`]（L1）—— 本模块**驱动**它：
//! 依赖方向是**单向**的 host → display。
//!
//! ⚠️ winit 的 `EventLoop` / `ApplicationHandler` / `ControlFlow` 在**本模块**：它们是**事件循环的
//! 驱动面**（L3），不是平台句柄面（L1）—— 平台句柄与输入映射才在 `display`。
//!
//! 拆分前这些内容与 L1 同住在 `lib.rs`；**公开路径一个都没变**（`lib.rs` 用 `pub use` 重新导出）。
//!
//! ⚠️ 本模块里 `WindowId` 是**本层自己的**窗口标识（winit 那个以 `WinitWindowId` 的别名进来，
//! 不出现在任何公开签名里 —— 与 [`InputEvent`] 同一条纪律）。

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use deer_gpu::Extent;
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, Ime, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
// `WindowId` 名字留给本层自己的公开类型（见下面 [`WindowId`]）；winit 的那个只在
// `alive` 表的键上出现，从不进任何公开签名。
use winit::window::{Window, WindowId as WinitWindowId};

use crate::display::{
    InputEvent, Mods, PointerButton, WindowConfig, WindowInfo, map_key, map_mouse_button, map_mods,
    map_wheel, printable_text, window_info,
};

/// 一帧/一次关闭请求的处置结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flow {
    /// 继续跑事件循环。
    Continue,
    /// 请求结束事件循环（`run()` 随后返回 `Ok(())`）。
    Exit,
}

/// 运行时覆盖重绘策略的环境变量名（**低功耗模式必须能被关掉**）。
///
/// 取值 `continuous`（大小写不敏感、两侧空白先 `trim()`）⇒ 无条件强制
/// [`RedrawPolicy::Continuous`]；`on-demand` / `demand` / 未设 / 无法识别 ⇒ 用
/// [`App::redraw_policy`] 自己声明的策略。判定逻辑见 [`resolve_redraw_policy`]（纯函数，有真值表单测）。
///
/// 名字抽成常量是为了让**自证那行日志**、示例与单测共用同一个字符串 —— 三处各写一遍迟早会分叉。
pub const REDRAW_ENV: &str = "DEER_WINDOW_REDRAW";

/// 重绘策略：**省电（默认）还是连续**。由 [`App::redraw_policy`] 声明，可被 [`REDRAW_ENV`] 覆盖。
///
/// 刻意**不**塞进 [`Flow`]：`Flow` 是每次回调的返回值，有 5 个既有实现按 `Flow::Continue`/`Exit` 穷举匹配，
/// 往里面加变体会一次性破坏它们（M5b 的接口冻结要求：既有实现一行都不用改）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum RedrawPolicy {
    /// **默认**：只有「输入改了状态」（[`App::wants_redraw`] 为真）或系统事件才请求重绘 ——
    /// 空闲时零重绘，事件循环在 [`ControlFlow::Wait`] 上睡死。
    #[default]
    OnDemand,
    /// 每画完一帧立刻请求下一帧（`ControlFlow::Poll` 时代的连续重绘语义）——
    /// 适合动画；空闲时也会一直烧 CPU，所以**不是**默认值。
    Continuous,
}

impl RedrawPolicy {
    /// 打印用的可读名（自证的「实际=…」就是它）：`"OnDemand"` / `"Continuous"`。
    pub fn label(self) -> &'static str {
        match self {
            RedrawPolicy::OnDemand => "OnDemand",
            RedrawPolicy::Continuous => "Continuous",
        }
    }

    /// 画完一帧之后是否**再请求下一帧**（`Continuous` ⇒ 是）。
    ///
    /// 抽成纯函数是为了让它能被单测数：`OnDemand` 下 N 帧 ⇒ N 次请求（只有引导帧与系统事件），
    /// `Continuous` 下 N 帧 ⇒ N+1 次请求（每帧续下一帧）。
    pub fn wants_next_frame(self) -> bool {
        matches!(self, RedrawPolicy::Continuous)
    }
}

/// [`REDRAW_ENV`] 的字符串值 → 强制策略（纯函数，**先 `trim()` 再比、大小写不敏感**）。
///
/// - `"continuous"`（含 `"CONTINUOUS"` / `" continuous "` / `"continuous\t"`）⇒ `Some(Continuous)`；
/// - `"on-demand"` / `"demand"` ⇒ `Some(OnDemand)`（识别为「明确要求按需」，见 [`resolve_redraw_policy`]）；
/// - 未设（`None`）、空串、纯空白、其它任何值 ⇒ `None`（= 没有可用的覆盖）。
///
/// **必须 trim**：`cmd` 的 `set X=1 && …` 会把 `&&` 前的空格算进变量值
/// （`cmd /c "set X=1 && set X"` 实测打印 `X=1 `），严格比较会让「设了」静默失效。
pub fn parse_redraw_policy(raw: &str) -> Option<RedrawPolicy> {
    let v = raw.trim();
    if v.eq_ignore_ascii_case("continuous") {
        Some(RedrawPolicy::Continuous)
    } else if v.eq_ignore_ascii_case("on-demand") || v.eq_ignore_ascii_case("demand") {
        Some(RedrawPolicy::OnDemand)
    } else {
        None
    }
}

/// [`App::redraw_policy`] 的声明 + [`REDRAW_ENV`] 的原始值 → **实际生效**的策略（纯函数）。
///
/// 规则（接口冻结，别改方向）：
///
/// - 环境变量是 `continuous` ⇒ **无条件** `Continuous`（这就是「关掉省电模式」的开关）；
/// - 其余一切（`on-demand` / `demand` / 未设 / 空值 / 无法识别）⇒ 用 `app_policy`：
///   显式写 `on-demand` 是「不覆盖」，**不是**「把声明 Continuous 的 App 按回 OnDemand」——
///   自证日志会把「请求 / 实际 / App 声明」三者都打出来，不靠猜。
pub fn resolve_redraw_policy(app_policy: RedrawPolicy, raw: Option<&str>) -> RedrawPolicy {
    match raw.and_then(parse_redraw_policy) {
        Some(RedrawPolicy::Continuous) => RedrawPolicy::Continuous,
        _ => app_policy,
    }
}

// ——————————————— M5c：唤醒面（用户事件 + deadline）———————————————

/// winit 的**用户事件**类型：**私有**（本轮不开自定义用户事件面 —— 「Not Doing」）。
///
/// App 那一侧只看得到 [`Waker`] 与 [`WindowSpawner`]（各是一根**同型**通道的薄封装）；
/// winit 的类型与这个枚举都不进回调签名（与 [`InputEvent`] 同一条纪律）。
#[derive(Debug, Clone, PartialEq, Eq)]
enum Wake {
    /// [`Waker::wake`]：立刻醒来看一眼。
    Look,
    /// [`Waker::wake_after`]：装一个 deadline（到点由事件循环自己醒）。
    ///
    /// 带上**绝对**时刻而不是 `Duration`：走一趟通道/消息队列也要时间，用相对量会让每次唤醒
    /// 都往后漂一点（要做 60fps 的定时动画时，那种漂移会累积）。
    At(Instant),
    /// [`WindowSpawner::spawn_window`]：把「想建一个窗」的请求**排队**进事件循环
    /// （T4.4-R1，决策 2）。winit 0.30 只允许在持 [`ActiveEventLoop`] 的安全点建窗 ⇒
    /// 真正的建窗发生在 [`RunHandler::user_event`] 收到这条请求的时刻 —— 排队与建窗之间
    /// 隔一次事件循环迭代是**设计**，不是延迟。
    /// （`Copy` 因此放弃 —— `WindowConfig` 带着 `String`；消息本就应当按值走一趟通道。）
    Spawn(WindowConfig),
}

/// **可克隆的唤醒句柄**（M5c）：App 在建好窗后由 [`App::wake_handle`] 拿到，存下来即可在
/// [`RedrawPolicy::OnDemand`]（省电）下被**非窗口事件**唤醒（定时动画、脚本重放、别的线程改状态）。
///
/// 它是 winit `EventLoopProxy` 的**薄封装**：App 拿不到 `send_event`，也就没法往里塞自定义事件
/// （本轮刻意不做）。语义见 `lib.rs` 的 crate 文档「唤醒面」一节；两个方法的**分工**尤其别混：
/// [`Waker::wake`] 是提示（答真才画），[`Waker::wake_after`] 是预约（到点一定画）。
///
/// **`Send`**：可以搬到别的线程里去叫醒主线程（方法都是 `&self`）；编译期有一处断言钉着它
/// （见紧随其后的 `const _: fn()`）。**不沿用「也保证 `Sync`」这个说法** —— 在 Windows 上
/// 它今天**恰好**同时满足 `Sync`（winit 只为 Windows 写了 `unsafe impl Send`，`Sync` 是字段
/// 自动推导出来的巧合），但不是 winit 的承诺，本层**不依赖**它：要跨线程共享就每个线程
/// 各 `clone()` 一份。
#[derive(Clone)]
pub struct Waker {
    proxy: EventLoopProxy<Wake>,
}

impl Waker {
    /// 立刻唤醒事件循环。
    ///
    /// 醒来之后**问一次** [`App::wants_redraw`]：**答真才请求一帧，答假就一帧都不画**。
    /// 所以「多叫一声」不会毁掉省电（也不会漏帧）—— 这是它与 [`Waker::wake_after`] 的分工。
    /// 典型用法：后台线程更新了共享状态，叫主线程起来看一眼。
    ///
    /// 事件循环**已经结束**时（`send_event` 返回 `Err(EventLoopClosed)`）**静默丢弃**：那时 App
    /// 正在退出，没有可报告的对象；本层纪律是**不 panic**。除此之外不会失败。
    pub fn wake(&self) {
        let _ = self.proxy.send_event(Wake::Look);
    }

    /// **`d` 之后**唤醒事件循环一次（定时唤醒：定时动画 / 脚本重放的推进器）。
    ///
    /// **用 deadline 实现，不睡线程**：只是把「`now + d`」交给事件循环（后台线程调用也一样），
    /// 事件循环平时仍然睡在 [`ControlFlow::Wait`] 上，到点由系统叫醒 ⇒ **两次唤醒之间零 CPU**。
    ///
    /// 与 [`Waker::wake`] 的语义**不同**：这是 App **自己下的单**（「那个时刻请叫我」）⇒ 到点
    /// **一律**画一帧（不看 [`App::wants_redraw`]，理由见 `lib.rs` 的 crate 文档「唤醒面」）。调用它本身
    /// **不画**：只装 deadline。
    ///
    /// 排了多次也不会打架：取**最近**的那个（早的先到点），到点后由 App 在 [`App::redraw`] 里
    /// 排下一次。已经过期的时刻视为「立刻到点」。
    pub fn wake_after(&self, d: Duration) {
        let _ = self.proxy.send_event(Wake::At(Instant::now() + d));
    }
}

/// `Waker` 的 `Debug`：只说「有个句柄」，不把内部代理打出来（与 winit 自己那份一致）。
impl std::fmt::Debug for Waker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.pad("Waker { .. }")
    }
}

/// **编译期断言**：`Waker` 必须是 `Send`（可以搬到别的线程里去叫醒事件循环）。
///
/// 依据：winit 0.30.13 的 Windows 后端有
/// `unsafe impl<T: Send + 'static> Send for EventLoopProxy<T>`
/// （本机源码 `winit-0.30.13/src/platform_impl/windows/event_loop.rs:820`）。
///
/// **`Sync` 故意不断言**：本层不用它（见 [`Waker`] 的说明）。若哪天 winit 去掉了那个
/// `unsafe impl Send`，这里会立刻编译失败 —— 那正是我们要的（`Send` 是承诺，不是碰巧）。
const _: fn() = || {
    fn assert_send<T: Send>() {}
    assert_send::<Waker>();
};

// ——————————————— T4.4-R1：多窗口（WindowId / WindowSpawner）———————————————

/// **窗口标识**（T4.4-R1）：多窗口路由的**钥匙** —— [`App::window_input`] 用它告诉 App
/// 「这条事件属于哪个窗」，App 用它给「每窗一棵树 / 每窗一份状态」当 `HashMap` 的键（决策 6）。
///
/// 它是**本层建窗时自发**的序号（主窗 = `1`，之后每扇窗递增），**不是** winit 的 `WindowId`：
/// ① 同一条纪律 —— winit 类型不进回调签名（与 [`InputEvent`] 一致）；
/// ② winit 的 id 没有公开构造（只有值恒相同的 `dummy()`），纯逻辑单测造不出两个不同的 id，
/// 路由表和关闭语义就没法测 —— 本层自己的序号两样都解决。
/// `Copy + Eq + Hash + Ord` ⇒ 键控、排序、日志都顺手。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct WindowId(u64);

impl WindowId {
    /// 原始序号 → 标识（**唯一**的构造点：本层建窗时分配；单测里用它造两个不同的键）。
    pub(crate) fn from_raw(n: u64) -> WindowId {
        WindowId(n)
    }
}

/// **多窗口的建窗句柄**（T4.4-R1，决策 2）：与 [`Waker`] **同一根通道**（同一个
/// `EventLoopProxy`）的薄封装 ——「与 Waker 同型的通道」就是这个意思。
///
/// [`WindowSpawner::spawn_window`] 只做一件事：把「想建一个窗」的请求**排队**进事件循环。
/// winit 0.30 的硬约束是「只能在持 [`ActiveEventLoop`] 的安全点建窗」⇒ 真正的建窗发生在
/// 事件循环收到这条请求的时刻（[`RunHandler::user_event`] 的 `Spawn` 臂）—— App 在
/// `init` / `redraw` / 任何回调里调用它都安全，因为重的建窗动作不在调用点上发生。
///
/// 与 [`Waker`] 同一条纪律：App 拿不到 `send_event` 原始面；`Send` 是承诺（同样有编译期
/// 断言）；事件循环已结束时请求**静默丢弃**（App 正在退出，没有可报告的对象，不 panic）。
#[derive(Clone)]
pub struct WindowSpawner {
    proxy: EventLoopProxy<Wake>,
}

impl WindowSpawner {
    /// 请求建一个新窗口（参数与 [`run()`] 的 `config` 同型：标题 + **逻辑**尺寸）。
    ///
    /// **不等待、没有回执**：请求只是排队；建窗成不成功经由正常的生命周期抵达
    /// （新窗的 [`App::init`] / `Err` 路径），本层不另设「spawn 的回执」这一个面。
    /// 事件循环**已经结束**时静默丢弃（与 [`Waker::wake`] 同一口径：不 panic）。
    pub fn spawn_window(&self, config: WindowConfig) {
        let _ = self.proxy.send_event(Wake::Spawn(config));
    }
}

/// `WindowSpawner` 的 `Debug`：只说「有个句柄」（与 [`Waker`] 的做法一致）。
impl std::fmt::Debug for WindowSpawner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.pad("WindowSpawner { .. }")
    }
}

/// **编译期断言**：`WindowSpawner` 必须是 `Send`（与 [`Waker`] 同一承诺、同一依据）。
const _: fn() = || {
    fn assert_send<T: Send>() {}
    assert_send::<WindowSpawner>();
};

/// 一轮「该睡多久」的**纯逻辑**答案（[`plan_wake`] 的返回值，可单测）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WakePlan {
    /// 没有任何 deadline ⇒ **睡死**（[`ControlFlow::Wait`]）。**这是省电模式的默认值**：
    /// 本层不造任何超时（「App 不声明就不给 timeout」）。
    Wait,
    /// 有一个**未来**的 deadline ⇒ 睡到那个时刻（[`ControlFlow::WaitUntil`]）。
    WaitUntil(Instant),
    /// deadline 已经**到点**（含已过期——过期的时刻视为「现在」）⇒ 立刻醒来画一帧。
    Due,
}

/// 取两个可选时刻里**更早**的那个（`None` 不参与比较；两个都 `None` ⇒ `None`）。
///
/// `Waker::wake_after` 可以排多次、[`App::next_deadline`] 又可能另给一个 ⇒ 需要一个
/// 「谁先到点听谁的」的合并规则，且只有这一处实现（免得推式/拉式两条路各写一遍、迟早分叉）。
pub fn earliest(a: Option<Instant>, b: Option<Instant>) -> Option<Instant> {
    match (a, b) {
        (Some(x), Some(y)) => Some(if x <= y { x } else { y }),
        (Some(x), None) | (None, Some(x)) => Some(x),
        (None, None) => None,
    }
}

/// **唤醒计划**（纯函数，不碰窗口）：`now` 时刻，面对「推来的 deadline」与「App 声明的 deadline」
/// 该收敛到哪个 [`ControlFlow`]。
///
/// 规则只有三条（顺序即优先级）：
///
/// 1. 两个都没有 ⇒ [`WakePlan::Wait`]（**省电**：睡死，不造超时）；
/// 2. 更早的那个在 `now` **之后** ⇒ [`WakePlan::WaitUntil`]（睡到那个时刻）；
/// 3. 更早的那个在 `now` **或之前** ⇒ [`WakePlan::Due`]（到点/已过期 ⇒ 立刻画一帧）。
pub fn plan_wake(now: Instant, armed: Option<Instant>, declared: Option<Instant>) -> WakePlan {
    match earliest(armed, declared) {
        None => WakePlan::Wait,
        Some(at) if at <= now => WakePlan::Due,
        Some(at) => WakePlan::WaitUntil(at),
    }
}

/// 把 [`WakePlan`] 翻成 winit 的 [`ControlFlow`]（**唯一的翻译点**，[`RunHandler::refresh_control_flow`]
/// 只调它）。
///
/// 抽成函数是为了**可单测**：`WakePlan::Wait ⇒ ControlFlow::Wait`（**睡死**）是整个省电承诺的
/// 落点，而它原来只是 `match` 里的一个字面量。M5c 复审的变异把它改成 `Poll` ⇒ CPU 烧满一核、
/// `iters` 5 → 11 695 034，**却没有任何断言抓得住**（`plan_wake` 的真值表只管到 `WakePlan`，
/// 管不到这次翻译；`wake_policy.rs` 的 harness 是复刻，也管不到）。现在 `lib.rs` 末尾的
/// `wake_policy_control_flow_mapping` 单测直接钉住这三条映射。
///
/// `Due ⇒ Wait`：到点那一帧已经 `request_redraw()` 过了，本轮先睡 —— 下一个 deadline 由 App
/// 在 `redraw` 里重装（推式）或由 `next_deadline` 现问（拉式）。
///
/// `pub(crate)`：`lib.rs` 末尾那条单测（`wake_policy_control_flow_mapping`）要直接调它 ——
/// 它是「`Wait` ⇒ [`ControlFlow::Wait`]」这条省电承诺唯一的翻译点，必须被钉住。
pub(crate) fn plan_to_control_flow(plan: WakePlan) -> ControlFlow {
    match plan {
        WakePlan::Wait | WakePlan::Due => ControlFlow::Wait,
        WakePlan::WaitUntil(at) => ControlFlow::WaitUntil(at),
    }
}

/// 唤醒面的**账本**（[`run()`] 收尾打的那行「唤醒账本」就是它；`tests/wake_policy.rs` 数的也是它）。
///
/// 与 [`FrameCounter`] 同一纪律：**只能通过这几个方法加计数**（「请求次数」只有一个真相来源），
/// 判定逻辑（[`WakeStats::on_look`]）也放在这里，示例与单测数的是**同一份实现**。
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct WakeStats {
    looks: u64,
    arms: u64,
    fired: u64,
    requested: u64,
    skipped: u64,
    iters: u64,
}

impl WakeStats {
    pub fn new() -> WakeStats {
        WakeStats::default()
    }

    /// [`Waker::wake`] **投递到达**（事件循环真的收到了这一声）。
    ///
    /// 决策与输入走同一把尺：`wants_redraw` 为真 ⇒ 记一次请求并返回 `true`；为假 ⇒ 记一次
    /// 「唤醒被跳过」（`skipped`）并返回 `false`。
    ///
    /// 为什么**不**复用 [`FrameCounter::on_input`] 的 `skipped`：那一格的口径是「**派发了一条
    /// 输入**但没请求重绘」，`wake()` 不是输入 ⇒ 混进去会让那条口径失去意义（文档与验收都在按
    /// 它数）。两条账各数各的，名字也不一样。
    pub fn on_look(&mut self, wants_redraw: bool) -> bool {
        self.looks += 1;
        if wants_redraw {
            self.requested += 1;
            true
        } else {
            self.skipped += 1;
            false
        }
    }

    /// [`Waker::wake_after`] **投递到达**：只装 deadline，**不**请求（到点才请求）。
    pub fn on_arm(&mut self) {
        self.arms += 1;
    }

    /// deadline **到点**：**一律**请求一帧 ⇒ `fired` 与 `requested` **同步 +1**（这条不变量
    /// 有单测钉着：到点就一定有一次请求，两者不许漂）。
    pub fn on_fire(&mut self) {
        self.fired += 1;
        self.requested += 1;
    }

    /// 事件循环**迭代一次**（= `about_to_wait` 被调用一次）：**空转观测值**。
    ///
    /// 省电空闲下它应当只有**个位数**（实测 5–7）；退化成「超时打转」时它会一秒涨上千，而
    /// `frames`/`wants_redraw` 那套**抓不住**这种空转（没人请求重绘，一帧都不会多画）。
    ///
    /// ⚠️ **它自己不是判据**（M5c 复审 I1：本层没有全局阈值，`Continuous` 档下它本来就高）。
    /// 下断言的地方在**声明了空闲语义的那一档**：`examples/wake_probe.rs` 的 `DEER_WAKE_TICKS=0`
    /// 档通过 [`App::on_wake_stats`] 拿账本，断言 `iters` 的**上界**。
    pub fn note_iter(&mut self) {
        self.iters += 1;
    }

    /// `Waker::wake` 的投递到达次数。
    pub fn looks(&self) -> u64 {
        self.looks
    }

    /// `Waker::wake_after` 的投递到达次数（= 装了几次 deadline）。
    pub fn arms(&self) -> u64 {
        self.arms
    }

    /// deadline 到点的次数。
    pub fn fired(&self) -> u64 {
        self.fired
    }

    /// 唤醒面**请求重绘**的次数（`wake()` 答真 + deadline 到点）。与 `frames` 不一定相等：
    /// 请求要等 `RedrawRequested` 到达才变成一帧（winit 会把重复请求合并）—— 口径与
    /// [`FrameCounter::redraw_requests`] 一致。
    pub fn requested(&self) -> u64 {
        self.requested
    }

    /// `wake()` 问了 [`App::wants_redraw`] 但**答假**、因此没请求重绘的次数。
    pub fn skipped(&self) -> u64 {
        self.skipped
    }

    /// 事件循环迭代次数（**观测值**，见 [`WakeStats::note_iter`]：断言在 `wake_probe` 空闲档）。
    pub fn iters(&self) -> u64 {
        self.iters
    }
}

/// 用户实现这个 trait；[`run()`] 负责事件循环与窗口生命周期。
///
/// 所有回调都在**主线程**（事件循环线程）上被调用。
pub trait App {
    /// 窗口建好后**调一次**：在这里创建渲染器（Vulkan 设备/交换链）。
    ///
    /// 返回 `Err` 会让 `run()` 打印原因、退出事件循环并返回 `Err`。
    ///
    /// T4.4-R1 起是「**每建一扇窗调一次**」（多 `WindowConfig` 启动 / 动态 spawn 的窗各一次，
    /// 按建窗顺序）：单窗口用户的感知完全不变；多窗口下按 id 区分窗靠的是
    /// [`App::window_input`]（决策 6 的每窗状态从收到该窗第一条事件起初始化即可）。
    fn init(&mut self, info: &WindowInfo) -> Result<(), String>;

    /// 尺寸变化（含 DPI 变化）：调用方应当重建交换链。默认什么都不做。
    ///
    /// `width`/`height` 是**物理**像素，直接透传，本层不乘缩放系数。
    fn resized(&mut self, width: u32, height: u32) -> Result<(), String> {
        // 默认实现什么都不做。参数名保持与冻死 API 一致（不改名成 `_width`），
        // 用 `let _` 消化掉，否则 rustc 的 unused_variables 会让 clippy 不干净。
        let _ = (width, height);
        Ok(())
    }

    /// 画一帧（含呈现）。返回 `Flow::Exit` 表示请求结束事件循环。
    fn redraw(&mut self) -> Result<Flow, String>;

    /// 收到一条输入事件（[`InputEvent`]）。
    ///
    /// 默认实现**什么都不做**并返回 `Flow::Continue` —— M5 之前写的 `App` 实现不用改。
    /// 返回 `Flow::Exit` 与 [`App::redraw`] 同义（请求结束事件循环）；返回 `Err` 会让
    /// `run()` 打印原因、退出事件循环并返回 `Err`（**绝不吞掉**）。
    ///
    /// **字符与物理键是两条事件**：文本走 [`InputEvent::TextInput`]，物理键走
    /// [`InputEvent::KeyDown`]/[`InputEvent::KeyUp`]（见 [`Key`](crate::display::Key) 的说明）。
    /// `info` 是**当前**窗口信息（原生句柄 + 物理尺寸，`Resized` 之后同步更新），
    /// 口径与 [`App::resized`] 一致 —— 输入事件里的坐标就跟它同一套物理像素。
    fn input(&mut self, info: &WindowInfo, ev: &InputEvent) -> Result<Flow, String> {
        // 默认实现什么都不做。参数名保持与冻死 API 一致（不改名成 `_info`/`_ev`），
        // 用 `let _` 消化掉。
        let _ = (info, ev);
        Ok(Flow::Continue)
    }

    /// **多窗口输入路由**（T4.4-R1，决策 1/6）：这条输入属于哪个窗，连同**该窗自己的**
    /// [`WindowInfo`] 一起交给你；它**只收这一个窗**的事件（决策 6「每窗一棵树」的前提）。
    ///
    /// 默认实现**转发既有 [`App::input`]**（忽略 id）—— 单窗口用户**零改动**：只实现过
    /// `input` 的 App 照常收到事件。要多窗口的实现**覆盖本方法**；覆盖之后 [`App::input`]
    /// **不再被调用**（转发只发生在默认实现里 —— 不然两条路都会响）。
    ///
    /// 为什么签名里有 `info`：登记文本写的是 `(id, event)` 的简写 —— 转发目标 [`App::input`]
    /// 需要 `info`（该窗的物理尺寸/句柄），缺了它「默认转发」根本写不出来，这也是唯一让
    /// 「单窗口零改动」成立的非破坏读法。`info` 就是**这个窗**的当前信息（口径与
    /// [`App::resized`] 一致：物理像素，`Resized` 之后同步更新）。
    ///
    /// 返回值与 [`App::input`] 同义：`Flow::Exit` ⇒ 结束事件循环；`Err` ⇒ 打印 + 退出 + 返回。
    fn window_input(
        &mut self,
        id: WindowId,
        info: &WindowInfo,
        ev: &InputEvent,
    ) -> Result<Flow, String> {
        let _ = id;
        self.input(info, ev)
    }

    /// 点了关闭按钮/系统关闭：默认允许关闭。
    fn close_requested(&mut self) -> Flow {
        Flow::Exit
    }

    /// **这一条输入改了状态吗**（M5b）：为真则 `run()` 立刻请求一帧重绘，为假就**不请求**。
    ///
    /// 默认 `false` —— M5b 之前写的 `App` 实现不用改一行：它们本来就是「每次输入都重绘」
    /// （旧行为），现在变成「不主动重绘」；要恢复旧观感就实现本方法（或声明
    /// [`RedrawPolicy::Continuous`]，或设 `DEER_WINDOW_REDRAW=continuous`）。
    ///
    /// 调用时机：`run()` 在**每条** [`InputEvent`] 交给 [`App::input`] **之后**读一次它 ——
    /// 所以实现里应当反映「刚才那条事件是否真的改了要画的东西」（例如「hover 到的元素变了」
    /// 才算脏，「鼠标在同一个按钮上又动了 1 像素」不算）。
    ///
    /// **只对输入事件生效**：系统事件（`Resized`/`Focused`/窗口重新暴露/建窗引导帧）**一律**请求重绘，
    /// 不看这个返回值（否则尺寸变了却没人重画，窗口会留一片脏区）。
    fn wants_redraw(&self) -> bool {
        false
    }

    /// 重绘策略：默认 [`RedrawPolicy::OnDemand`]（省电，空闲时零重绘）。
    ///
    /// 需要「一直动」的实现（动画、每帧都要重画的 demo、按墙钟自动退出的示例）返回
    /// [`RedrawPolicy::Continuous`]：`run()` 会在每画完一帧后再请求下一帧。
    ///
    /// 这个声明可被 [`REDRAW_ENV`]（`DEER_WINDOW_REDRAW=continuous`）在**运行时**覆盖。
    fn redraw_policy(&self) -> RedrawPolicy {
        RedrawPolicy::OnDemand
    }

    /// **唤醒句柄**（M5c）：主窗建好后**调一次**，把 [`Waker`] 交给你 —— **存下来**。
    ///
    /// 默认实现**什么都不做** ⇒ M5c 之前写的 `App` 实现一行都不用改（它们拿不到句柄，
    /// 想做按时间的动画就仍然只能声明 [`RedrawPolicy::Continuous`]）。
    ///
    /// 调用时机（可依赖的三条）：
    ///
    /// 1. 在**第一扇窗**的 [`App::init`] **成功之后**（渲染器已经建好了，可以立刻排一个唤醒）；
    /// 2. 在**建窗引导帧**之前；
    /// 3. 整个 `run()` **只调一次**（T4.4-R1 起多窗共享同一个 [`Waker`]：唤醒是 App 级的，
    ///    不属于某扇窗）。
    ///
    /// 拿到之后就有三个手段（语义见 `lib.rs` 的 crate 文档「唤醒面」）：[`Waker::wake`]（提示）、
    /// [`Waker::wake_after`]（预约）、[`App::next_deadline`]（拉式预约）。
    fn wake_handle(&mut self, waker: Waker) {
        // 默认实现什么都不做。参数名保持易读（不改成 `_waker`），用 `let _` 消化掉 unused 警告。
        let _ = waker;
    }

    /// **建窗句柄**（T4.4-R1，决策 2）：主窗建好后**调一次**，把 [`WindowSpawner`] 交给你 ——
    /// 与 [`App::wake_handle`] **同型**的交付面（「App 经与 Waker 同型的通道获取句柄」）。
    /// **存下来**，之后任何回调里都能 `spawner.spawn_window(config)` 排队建新窗
    /// （真正的建窗在事件循环的安全点完成，见 [`WindowSpawner`]）。
    ///
    /// 默认实现**什么都不做** ⇒ 单窗口实现一行不用改。调用时机与 [`App::wake_handle`]
    /// 完全同批：第一扇窗 `init` 成功之后、引导帧之前，整个 `run()` 只调一次。
    fn window_spawner(&mut self, spawner: WindowSpawner) {
        // 默认实现什么都不做（同 wake_handle 的纪律）。
        let _ = spawner;
    }

    /// **我希望被唤醒的最近时刻**（M5c，**拉**式）：`Some(t)` ⇒ 事件循环用
    /// `ControlFlow::WaitUntil(t)` 睡到 `t`，**到点画一帧**（不看 [`App::wants_redraw`]，
    /// 理由见 `lib.rs` 的 crate 文档「唤醒面」）；`None`（**默认**）⇒ 不装 deadline，事件循环在
    /// [`ControlFlow::Wait`] 上睡死（**省电**）。
    ///
    /// 它每轮事件循环收敛时都可能被问一次（`resumed` / 收到用户事件 / `about_to_wait`），
    /// 所以实现要**便宜、无副作用**（`&self`：它不该在这里改状态；要改状态就等
    /// [`App::redraw`]）。
    ///
    /// ⚠️ **必须给出固定的时刻、并且自己往前走**（两条都是踩过的坑，写进接口文档）：
    ///
    /// - **别**每次都返回 `Instant::now() + 50ms`：那样它**永远不到点** —— 事件循环每 50ms 醒
    ///   一次、却一帧都不画（空转，正是本层反对的东西）。「每 50ms 画一帧」请用
    ///   [`Waker::wake_after`]：在 [`App::redraw`] 里排下一次（推式）。
    /// - 已经**过期**的时刻视为「立刻到点」⇒ 立刻画一帧；App 不清掉它，就等于自己要求
    ///   连续重绘（那是 [`RedrawPolicy::Continuous`] 的语义）。账本上会看见 `fired` 跟着
    ///   `iters` 一起涨 —— 那不是本层空转，是 App 自己下的单。
    fn next_deadline(&self) -> Option<Instant> {
        None
    }

    /// **收尾时把唤醒账本交给你**（M5c fix 轮加的；**默认实现什么都不做**）。
    ///
    /// `run()` 收尾（打「唤醒账本」那行**之前**）**调一次**，参数是最终的 [`WakeStats`]。
    /// 这是 App（及示例/探针）**唯一**能读到 `iters` 的地方 —— 它不在任何其它回调的参数里。
    ///
    /// 存在的理由只有一条：**让空闲档能对 `iters` 下上界断言**。M5c 复审（I1）实测：
    /// 只把事件循环的 [`ControlFlow::Wait`] 换成 `Poll` ⇒ `iters` 从 5 涨到 **11 695 034**、
    /// CPU 烧满一核，而**退出码 0、每条判据仍 ✅** —— 因为当时仓库里**没有任何断言读 `iters`**。
    /// 现在 `examples/wake_probe.rs` 的 `DEER_WAKE_TICKS=0`（空闲）档用它断言上界。
    ///
    /// **本层刻意不做全局阈值**：`RedrawPolicy::Continuous` 档下 `iters` 与帧数同阶（实测
    /// 229365）是**合法**的 ⇒ 把阈值做进 [`run()`] 的收尾路径会假红。上界只能由**声明了空闲语义
    /// 的那一档**自己下（review 的原话：「只用于空闲档断言上界」）。
    ///
    /// ⚠️ 调用时机：事件循环**已经结束**（窗口可能已销毁）⇒ 这里只适合记账 / 断言 / 打印，
    /// **不要**再碰渲染资源。
    fn on_wake_stats(&mut self, stats: &WakeStats) {
        // 默认实现什么都不做。参数名保持易读（不改成 `_stats`），用 `let _` 消化 unused 警告。
        let _ = stats;
    }
}

/// 帧数 + 重绘请求数 + 跳过帧数 + 退出标记 —— [`run()`] 内部账本的**纯逻辑核心**（不碰窗口，可直接单测）。
///
/// `run()` 用它统计：
///
/// - `frames`：「`App::redraw` 被**成功**调用的次数」（回调返回 `Err` 的那次不算）；
/// - `redraw_requests`：「`request_redraw()` 被调用的次数」—— 系统事件/引导帧/续帧/输入置位都记在这里；
/// - `skipped`：「派发了输入但**没有**请求重绘」的次数（[`App::wants_redraw`] 为假 ⇒ 省下的那一帧）；
/// - `exit_requested`：`Flow::Exit` 时置上。
///
/// M5b 的验收就是数这三个数（不用 fps）：空闲喂 K 条不改状态的事件 ⇒ `skipped == K`、
/// `redraw_requests == 0`、`frames == 0`；改了状态 ⇒ `redraw_requests == 1`。
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct FrameCounter {
    frames: u64,
    redraw_requests: u64,
    skipped: u64,
    exit_requested: bool,
}

impl FrameCounter {
    pub fn new() -> FrameCounter {
        FrameCounter::default()
    }

    /// 派发完一条 [`InputEvent`] 之后的**重绘决策**（纯逻辑）：
    /// `wants_redraw` 为真 ⇒ 返回 `true`（调用方据此 `request_redraw()`，请求数由
    /// [`FrameCounter::note_request`] 记）；为假 ⇒ 记一次**跳过帧**并返回 `false`（**不**请求）。
    ///
    /// 与策略无关：`Continuous` 下的续帧走的是「画完再请求」，不改变「这条输入要不要重绘」。
    pub fn on_input(&mut self, wants_redraw: bool) -> bool {
        if wants_redraw {
            true
        } else {
            self.skipped += 1;
            false
        }
    }

    /// 记一次「请求重绘」：系统事件（`Resized`/`Focused`/窗口重新暴露）、建窗后的引导帧、
    /// `Continuous` 的续帧、以及输入置位请求的那一帧 —— 都走这里，**只此一处**加计数
    /// （否则「请求数」会有第二个真相来源，迟早对不上）。
    pub fn note_request(&mut self) {
        self.redraw_requests += 1;
    }

    /// 记一次 `App::redraw` 的结果：
    /// `Ok(_)` ⇒ 帧数 +1（`Flow::Exit` 这一帧也算画成功了）并可能置退出标记；
    /// `Err(e)` ⇒ 帧数**不变**，错误原样返回（由调用方决定打印/退出）。
    pub fn on_redraw(&mut self, result: Result<Flow, String>) -> Result<(), String> {
        match result {
            Ok(flow) => {
                self.frames += 1;
                if flow == Flow::Exit {
                    self.exit_requested = true;
                }
                Ok(())
            }
            Err(e) => Err(e),
        }
    }

    /// 成功画过的帧数（`init` 之后的 `redraw` 调用次数）。
    pub fn frames(&self) -> u64 {
        self.frames
    }

    /// **请求重绘的次数**（`request_redraw()` 被调用的次数，M5b 的「重绘次数」口径之一）。
    ///
    /// 与 [`FrameCounter::frames`] 不一定相等：请求要等 `RedrawRequested` 到达才会变成一帧
    /// （winit 也会把重复请求合并成一条），所以脚本核对时看 `frames` 才是「真画了几帧」。
    pub fn redraw_requests(&self) -> u64 {
        self.redraw_requests
    }

    /// **跳过帧数**：派发了输入但 [`App::wants_redraw`] 为假 ⇒ 没有请求重绘的次数。
    ///
    /// 这就是「省电模式真的省了」的可观测证据（M5b 的空闲探针就数它）。
    pub fn skipped_frames(&self) -> u64 {
        self.skipped
    }

    /// 是否有回调请求结束事件循环。
    pub fn exit_requested(&self) -> bool {
        self.exit_requested
    }
}

/// 建窗口并跑事件循环（**必须在主线程**调用；winit 的要求）。
///
/// 出错返回 `Err(说明)`，**不 panic**。具体地：
///
/// - 建 `EventLoop` / 建窗口失败 ⇒ 打印到 stderr、请求退出、返回 `Err`；
/// - `App` 的任何回调返回 `Err` ⇒ 同样打印 + 退出 + 返回 `Err`（**绝不吞掉**）；
/// - `App::redraw` 返回 `Flow::Exit`（或 `App::close_requested` 返回 `Flow::Exit`）⇒
///   退出事件循环，返回 `Ok(())`。
///
/// 无论哪种收尾，都会往 stdout 打便于脚本/验证断言的行（**别只看退出码**）：
///
/// - 启动时（自证重绘策略真的解析对了）：
///   `[deer-window] 重绘策略：请求=<环境变量原值|未设> 实际=<OnDemand|Continuous>（App 声明=<…>）`；
/// - 结束时（数得出来的账本）：
///   `[deer-window] 事件循环结束：frames=<n> extent=<w>x<h> result=ok|error`、
///   `[deer-window] 重绘账本：requests=<n> skipped=<n> frames=<n>` 与
///   `[deer-window] 唤醒账本：wake=<n> wake_after=<n> fired=<n> requested=<n> skipped=<n> iters=<n>`
///   （M5c 起；`iters` = 事件循环迭代次数，**是观测值**：本层**不设全局阈值**
///   （`Continuous` 档下它与帧数同阶是合法的）—— 读它下断言的是**空闲档**的
///   `examples/wake_probe.rs`（上界），账本经 [`App::on_wake_stats`] 交给 App）。
pub fn run<A: App + 'static>(config: WindowConfig, app: A) -> Result<(), String> {
    run_multi(&[config], app)
}

/// 建多扇窗口并跑事件循环（T4.4-R1：**多 `WindowConfig` 启动**，决策 3；**必须在主线程**调用）。
///
/// 与 [`run()`] 唯一的差别是**开多少扇窗**：`configs` 的每一项都会在 `resumed` 里按顺序
/// 建成真窗并各走一次 [`App::init`]（**第一项 = 主窗**，仍是「`run()` 首建」的那扇，决策 2；
/// 主窗建好后 [`App::wake_handle`] / [`App::window_spawner`] 各交一次句柄）。之后的一切 ——
/// 事件按 [`WindowId`] 路由（决策 1/6）、`CloseRequested` 只关该窗、全关 ⇒ 退出（决策 4）、
/// 每窗独立焦点（决策 5）—— 与 [`run()`] 走**同一条代码路径**（[`run()`] 就是 `configs`
/// 只有一项的 [`run_multi`]）。
///
/// 出错返回 `Err(说明)`，**不 panic**（错误面与 [`run()`] 完全一致，见它的文档）；
/// `configs` 为空是**调用方 bug** ⇒ 直接 `Err`（没有窗的事件循环只会永远等待）。
pub fn run_multi<A: App + 'static>(configs: &[WindowConfig], app: A) -> Result<(), String> {
    if configs.is_empty() {
        return Err(
            "run_multi 至少要一个 WindowConfig：没有窗口的事件循环只会永远等待".to_string(),
        );
    }
    // 重绘策略在这里定，**在建 EventLoop 之前**：策略 = App 自己声明的 + 环境变量的覆盖，
    // 与窗口无关 ⇒ 就算建窗失败，自证那行也已经打出来了。
    let requested = std::env::var(REDRAW_ENV).ok();
    let app_policy = app.redraw_policy();
    let policy = resolve_redraw_policy(app_policy, requested.as_deref());
    print_redraw_policy(requested.as_deref(), app_policy, policy);

    // M5c：**带用户事件**建事件循环（这才是「App 能自己唤醒本层」的底座）。
    // 事件类型是本层**私有**的 `Wake`（`EventLoopProxy` 不对 App 开放）⇒ 用
    // `with_user_event()` 而不是 `EventLoop::new()`；`EventLoop<()>` 那条路没有 proxy。
    // `build()` 收 `&mut self` ⇒ 先落到一个局部变量上（临时的也能编译，但这样读起来不靠运气）。
    let mut builder = EventLoop::<Wake>::with_user_event();
    let event_loop = builder
        .build()
        .map_err(|e| format!("创建 winit EventLoop 失败（run() 必须在主线程调用）：{e}"))?;
    // 句柄在建窗**之前**就取好：`resumed` 里才有东西交给 App（`App::wake_handle`）。
    // T4.4-R1：`WindowSpawner` 与 `Waker` **同一根通道**（同一个 proxy，决策 2）——
    // 两张「薄封装」各挡住各自的面（`wake`/`wake_after` vs `spawn_window`）。
    let proxy = event_loop.create_proxy();
    let waker = Waker { proxy: proxy.clone() };
    let spawner = WindowSpawner { proxy };

    let mut handler = RunHandler {
        configs: configs.to_vec(),
        app,
        policy,
        table: WindowTable::new(),
        alive: HashMap::new(),
        main: None,
        extent: Extent { width: 0, height: 0 },
        counter: FrameCounter::new(),
        mods: Mods::default(),
        error: None,
        exiting: false,
        waker,
        spawner,
        handles_given: false,
        next_window_id: 1,
        armed: None,
        wake_stats: WakeStats::new(),
    };

    // run_app 的返回值也要接住：只有把它和回调错误合并起来，错误才不会被吞。
    let loop_error = event_loop
        .run_app(&mut handler)
        .err()
        .map(|e| format!("winit 事件循环异常返回：{e}"));

    handler.finish(loop_error)
}

/// 打印**自证**行：环境变量的原值、App 自己声明的策略、实际生效的策略三者并列。
///
/// 为什么要自成一行、还要把「原值」原样打出来：门槛类判定最容易**静默失效**
/// （`cmd` 的 `set X=1 && …` 值里带一个尾空格，严格比较会把「设了」判成「没设」而照样 exit=0）。
/// 打印原值 ⇒ 尾空格/大小写一眼可见；打印实际生效值 ⇒ 验收可以 grep 结果，而不是相信退出码。
fn print_redraw_policy(
    requested: Option<&str>,
    app_policy: RedrawPolicy,
    effective: RedrawPolicy,
) {
    let raw = requested.unwrap_or("未设");
    let mut notes = String::new();
    if requested.is_some_and(|v| v.trim() != v) {
        notes.push_str("；原值含空白，已 trim 后判定");
    }
    if requested.is_some_and(|v| parse_redraw_policy(v).is_none()) {
        notes.push_str("；取值无法识别，按「用 App 自己的策略」处理");
    }
    println!(
        "[deer-window] 重绘策略：请求={raw} 实际={}（App 声明={}；{}=continuous 可强制关掉省电模式）{notes}",
        effective.label(),
        app_policy.label(),
        REDRAW_ENV,
    );
}

// ---------------------------------------------------------------------------
// 指针路由账本（T3.7 指针捕获的**窗口侧半**）
// ---------------------------------------------------------------------------

/// **指针事件的坐标账本 + 事件合成**（T3.7 指针捕获，D7 裁定=按下即默认捕获）。
///
/// 捕获语义的** interaction 层半**（按下记 `pressed`、移动路由给捕获者、抬起按捕获者
/// 结算）住在 `deer-gui`；它成立的前提是**拖拽期间移动/抬起事件仍源源不断地到达** ——
/// 这半归窗口层，职责收拢成两条（有单测钉住，见本文件末尾 `pointer_route_tests`）：
///
/// 1. **移动不做窗口边界过滤**（[`PointerRoute::on_cursor_moved`]）：指针拖出窗口后，
///    `CursorMoved` 带着窗外坐标**照样**记账并合成 [`InputEvent::PointerMoved`]。
///    主流平台在按键按住期间本来就会把移动继续投给按下时的窗口（Win32/X11/Wayland
///    的隐式捕获），本层**不加任何「坐标在窗口内才发」的判断** —— 加了捕获就断流。
/// 2. **按键事件用最近一次光标位置**（[`PointerRoute::on_mouse_button`]）：winit 的
///    `MouseInput` 不带坐标，按下/抬起的位置 = 账本里的最近值。拖出去在外面抬起时，
///    [`InputEvent::PointerUp`] 带的是**最后已知**位置（interaction 层按捕获者结算，
///    位置只影响 hover 落点，不影响点击归属）。还没收到过移动事件时是 `(0, 0)`
///    （既有约定，不另造第三种坐标）。
///
/// 这两条**改前就在**（`CursorMoved` 直通、`MouseInput` 取 `self.cursor`）—— 本类型
/// 把它们**收成一处可单测的契约**，捕获语义赖以成立的前提从此有护栏，而不是散在
/// 事件分支里没人看。
#[derive(Debug, Clone, Copy, Default)]
struct PointerRoute {
    /// 最近一次 `CursorMoved` 的物理坐标（`MouseInput` 合成事件时的位置来源）。
    cursor: (f32, f32),
}

impl PointerRoute {
    /// `CursorMoved` ⇒ **无条件**记账并合成 `PointerMoved`（约定 1：不做边界过滤）。
    fn on_cursor_moved(&mut self, x: f32, y: f32) -> InputEvent {
        self.cursor = (x, y);
        InputEvent::PointerMoved { x, y }
    }

    /// `MouseInput` ⇒ 用**最近一次**光标位置合成 `PointerDown`/`PointerUp`（约定 2）。
    fn on_mouse_button(&self, button: PointerButton, pressed: bool) -> InputEvent {
        let (x, y) = self.cursor;
        if pressed {
            InputEvent::PointerDown { button, x, y }
        } else {
            InputEvent::PointerUp { button, x, y }
        }
    }
}

/// **DPI 账本**（AF-3，Q4 裁断=**只透传**）—— scale_factor 的**唯一**记账点。
///
/// 与 [`PointerRoute`] 同一套做法：把「透传」这条契约收成一处**可单测的纯类型**，
/// 而不是散在 winit 事件分支里没人看。契约只有一条，但它是**红线**：
///
/// > OS 报多少就转发多少（`f64` 原样，不取整、不经 `f32` 折腾、不乘除任何数）——
/// > 事件坐标与 `WindowInfo::extent` 仍是同一套物理像素，布局仍是像素级纯函数。
/// > 自动按 DPI 缩放会破坏全仓逐字节像素判据（Q4），本层永远不做。
///
/// `WindowInfo::scale_factor` 与账本**同源**（建窗时都取自 winit 的
/// `window.scale_factor()`；之后都由 `ScaleFactorChanged` 更新）⇒ App 在任何回调里
/// 看到的 `info.scale_factor` 与最近一条 `ScaleFactorChanged` 事件的值**必然一致**。
#[derive(Debug, Clone, Copy, PartialEq)]
struct ScaleLedger {
    scale_factor: f64,
}

impl ScaleLedger {
    /// 建窗时的初值（`WindowInfo::scale_factor` 字段与它**同源同值**）。
    fn at_creation(scale_factor: f64) -> ScaleLedger {
        ScaleLedger { scale_factor }
    }

    /// `ScaleFactorChanged` ⇒ 记账并合成事件。**只透传**：值原样进事件、原样进账本。
    fn on_scale_factor_changed(&mut self, scale_factor: f64) -> InputEvent {
        self.scale_factor = scale_factor;
        InputEvent::ScaleFactorChanged { scale_factor }
    }

    /// 当前记账值（与 `WindowInfo::scale_factor` 恒等 —— 同一处记账的两面）。
    #[cfg(test)]
    fn current(&self) -> f64 {
        self.scale_factor
    }
}

// ---------------------------------------------------------------------------
// 多窗口路由表（T4.4-R1 的**纯逻辑核心**；不碰窗口、可直接单测 —— 与
// `PointerRoute` / `ScaleLedger` 同一套做法：契约收成一处可单测的纯类型）
// ---------------------------------------------------------------------------

/// **一个活窗口的纯状态**（T4.4-R1）——不含 winit 句柄 ⇒ 能脱离窗口单测。
///
/// 从 [`RunHandler`] 的单窗字段（拆分前的 `info` / `pointer` / `dpi` / `ime_composing`）
/// **原样**搬进来：这四样本就是「每窗一份」的状态（光标账本、DPI 账本、IME 状态都是
/// 窗口自己的），单窗口时代它们只是恰好只有一个实例。
struct WindowEntry {
    /// 建窗时算好、`Resized` / `ScaleFactorChanged` 时同步更新的 [`WindowInfo`]。
    info: WindowInfo,
    /// 指针路由账本（T3.7 的窗口侧半）：最近光标位置 + Down/Up 的事件合成。
    pointer: PointerRoute,
    /// DPI 账本（AF-3）：**该窗**的 scale_factor 唯一记账点。
    dpi: ScaleLedger,
    /// 该窗的 IME 是否正在预编辑（收到非空 `Preedit` 且还没 `Commit`/`Disabled`）。
    ime_composing: bool,
}

impl WindowEntry {
    /// 建窗时的初始纯状态（DPI 初值与 [`WindowInfo::scale_factor`] 同源同值）。
    fn at_creation(info: WindowInfo) -> WindowEntry {
        WindowEntry {
            info,
            pointer: PointerRoute::default(),
            dpi: ScaleLedger::at_creation(info.scale_factor),
            ime_composing: false,
        }
    }
}

/// [`WindowTable::close`] 的**纯判定**结果（决策 4 的关闭语义）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CloseOutcome {
    /// 该窗被移除。`last` = 这是不是**最后一扇**窗（true ⇒ 事件循环该退出）。
    Closed { last: bool },
    /// 表里没有这个 id（重复的 `Destroyed` / 关闭竞态）⇒ 什么都不发生（幂等）。
    Unknown,
}

/// **活窗口表**（T4.4-R1）：[`WindowId`] → [`WindowEntry`]。
///
/// 「事件到对的窗」（决策 1/6）与关闭语义（决策 4：只关该窗；全关 ⇒ 退出）的**判定**
/// 全收在这一个类型里，[`RunHandler`] 只做 IO（真正的 `request_redraw` / 窗口销毁）。
/// winit 的 `Arc<Window>` **不在**表里（没有公开构造 ⇒ 在表里就没法单测），由
/// [`RunHandler::alive`] 平行持有；两张表只经 [`RunHandler::create_window`] 与
/// close/`Destroyed` 两处同步改动，其余代码各取所需（纯状态查 `table`，句柄查 `alive`）。
struct WindowTable {
    entries: HashMap<WindowId, WindowEntry>,
}

impl WindowTable {
    fn new() -> WindowTable {
        WindowTable { entries: HashMap::new() }
    }

    /// 登记一扇新窗（建窗成功后的**唯一**入口；同 id 重复登记是本层的 bug）。
    fn insert(&mut self, id: WindowId, entry: WindowEntry) {
        let prev = self.entries.insert(id, entry);
        debug_assert!(prev.is_none(), "同一 {id:?} 登记了两次（建窗路径有 bug）");
    }

    fn get(&self, id: WindowId) -> Option<&WindowEntry> {
        self.entries.get(&id)
    }

    fn get_mut(&mut self, id: WindowId) -> Option<&mut WindowEntry> {
        self.entries.get_mut(&id)
    }

    /// 关闭语义（决策 4）的**唯一**实现：移除该窗并回答「这是不是最后一扇」。
    /// `CloseRequested`（App 允许关闭时）与 `Destroyed`（外部销毁/清理回执）都走它。
    fn close(&mut self, id: WindowId) -> CloseOutcome {
        match self.entries.remove(&id) {
            Some(_) => CloseOutcome::Closed { last: self.entries.is_empty() },
            None => CloseOutcome::Unknown,
        }
    }

    fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// 窗口数（单测断言用；生产路径只关心 `is_empty`）。
    #[cfg(test)]
    fn len(&self) -> usize {
        self.entries.len()
    }
}

/// [`run()`] 的 `ApplicationHandler` 实现：事件循环 → `App` 回调的接线（T4.4-R1 起多窗）。
struct RunHandler<A: App> {
    /// 尚未建窗的配置（`resumed` 里按顺序清空 = **主窗批**，决策 3；spawn 的请求走用户事件，
    /// 不进这里）。
    configs: Vec<WindowConfig>,
    app: A,
    /// **实际生效**的重绘策略（[`App::redraw_policy`] + [`REDRAW_ENV`] 的覆盖，在 `run()` 里算好）。
    policy: RedrawPolicy,
    /// **活窗口表**（纯状态；路由与关闭语义的判定在这里，见 [`WindowTable`]）。
    table: WindowTable,
    /// 活窗口的 winit 句柄（`request_redraw` / 关窗销毁用）。键是 winit 的 id（事件带来的），
    /// 值里带着**本层的** [`WindowId`] —— 两套 id 的**唯一**换算点。
    /// 与 `table` 只经 [`RunHandler::create_window`] / close/`Destroyed` 两处同步改动。
    alive: HashMap<WinitWindowId, LiveWindow>,
    /// **主窗**（第一扇建成的窗，决策 2「主窗仍由 run() 首建」）：收尾摘要的 extent 口径。
    main: Option<WindowId>,
    /// 主窗最近一次已知的物理尺寸（收尾摘要那行用；口径与拆分前一致）。
    extent: Extent,
    counter: FrameCounter,
    /// 当前修饰键状态：由 `ModifiersChanged` 维护（winit 0.30 没有「随时查」的接口）。
    /// 键盘是连接级状态（同一时刻至多一扇窗有焦点）⇒ 保持**单份**，与拆分前一致。
    mods: Mods,
    /// 第一个错误（后续错误不再覆盖它）。
    error: Option<String>,
    /// 已经请求 `event_loop.exit()`：同一批事件里后面的回调不再处理，
    /// 这样「帧数」就是 App 真正要求画的帧数，不会被 `exit()` 之后的残留事件多加。
    exiting: bool,
    /// 交给 App 的唤醒句柄（`resumed` 里 clone 一份给 [`App::wake_handle`]）。
    waker: Waker,
    /// 交给 App 的建窗句柄（与 [`App::window_spawner`] 同批交付；与 `waker` 同一根 proxy）。
    spawner: WindowSpawner,
    /// `wake_handle` / `window_spawner` 是否已交付（「只调一次」的保证落点）。
    handles_given: bool,
    /// 下一个待分配的 [`WindowId`]（从 1 起；1 号 = 主窗）。
    next_window_id: u64,
    /// [`Waker::wake_after`] **推**来的最近 deadline（`None` = 没推过，或被兑现后清掉了）。
    ///
    /// 与「App 声明的那个」（[`App::next_deadline`]，每轮现问）分开存：推来的这个**本层**负责
    /// 清（兑现一次就清，否则同一个时刻会被反复算成「到点」⇒ 空转）；拉式的那个是 App 的状态，
    /// 本层**不动**它（App 自己往前走，见 `next_deadline` 的 ⚠️）。
    armed: Option<Instant>,
    /// 唤醒面的账本（收尾那行「唤醒账本」）。
    wake_stats: WakeStats,
}

/// [`RunHandler::alive`] 的值：winit 句柄 + 它在本层的 [`WindowId`]（换算就发生在这一步）。
struct LiveWindow {
    id: WindowId,
    window: Arc<Window>,
}

impl<A: App> RunHandler<A> {
    /// 统一出错处理：打印原因 → 记住 → 请求退出。`run()` 最终返回这个 `Err`。
    fn fail(&mut self, event_loop: &ActiveEventLoop, msg: String) {
        eprintln!("[deer-window] 错误：{msg}");
        if self.error.is_none() {
            self.error = Some(msg);
        }
        event_loop.exit();
    }

    /// 建**一扇**窗（主窗批与 `spawn` 共用的**唯一**建窗点；持 [`ActiveEventLoop`] ⇒
    /// winit 0.30 的安全点，决策 2）。成功后依次：`窗口已建` 日志 → [`App::init`] →
    /// （仅第一扇之后）`wake_handle` + `window_spawner` → IME 允许 → 入两张表 → 引导帧。
    /// 失败走 [`RunHandler::fail`]（打印 + 记住 + 请求退出）。
    fn create_window(&mut self, event_loop: &ActiveEventLoop, config: &WindowConfig) {
        let attrs = Window::default_attributes()
            .with_title(config.display_title())
            // 建窗用逻辑尺寸（系统按 DPI 换算）；之后的 Resized 一律物理像素直传。
            .with_inner_size(LogicalSize::new(f64::from(config.width), f64::from(config.height)));
        let window = match event_loop.create_window(attrs) {
            Ok(w) => Arc::new(w),
            Err(e) => return self.fail(event_loop, format!("创建窗口失败：{e}")),
        };

        let info = match window_info(&window) {
            Ok(info) => info,
            Err(e) => return self.fail(event_loop, e),
        };
        let is_main = self.main.is_none();
        let id = WindowId::from_raw(self.next_window_id);
        self.next_window_id += 1;
        if is_main {
            // 主窗 = 第一扇（决策 2「主窗仍由 run() 首建」）；收尾摘要的 extent 口径就是它
            // （单窗口时它就是那扇窗 —— 与拆分前一致）。
            self.extent = info.extent;
            self.main = Some(id);
        }
        println!(
            "[deer-window] 窗口已建：title=\"{}\" extent={}x{} platform={:?} handle(HWND)=0x{:X} display(HINSTANCE)=0x{:X}",
            config.display_title(),
            info.extent.width,
            info.extent.height,
            info.raw.platform,
            info.raw.handle,
            info.raw.display
        );
        println!(
            "[deer-window] 窗口身份：id={id:?}{}（单窗口用户的 App 仍然只看到 init/redraw/input）",
            if is_main { "，主窗" } else { "" }
        );

        if let Err(e) = self.app.init(&info) {
            return self.fail(event_loop, format!("App::init 失败：{e}"));
        }

        // `wake_handle` / `window_spawner` 的「只调一次」保证：第一扇窗 init 成功之后、
        // 引导帧之前（与拆分前 `wake_handle` 的时机一致；`window_spawner` 同批交付）。
        if !self.handles_given {
            self.handles_given = true;
            self.app.wake_handle(self.waker.clone());
            self.app.window_spawner(self.spawner.clone());
        }

        // IME：winit 要求**显式允许**才会发 `Ime` 事件（`Ime::Commit` 是中文/日文输入的
        // 文本来源）。不开的话 CJK 输入在本层完全收不到 —— 那就与「输入通路已接通」相反。
        // 每扇窗都要开（各自有各自的 IME 上下文）。
        window.set_ime_allowed(true);

        let winit_id = window.id();
        self.table.insert(id, WindowEntry::at_creation(info));
        self.alive.insert(winit_id, LiveWindow { id, window });

        // **引导帧**（每扇窗各一次）：winit 对「建窗后一定发一条 `RedrawRequested`」**没有保证**
        // （winit 0.30 `Window::request_redraw` 的「no strong guarantees」），而 `OnDemand` 下
        // App 没有别的办法要到第一帧（它拿不到窗口句柄）⇒ 主动要一帧，否则窗口会一直留一块
        // 没画过的区域，`Continuous` 的续帧链也根本起不来。这是一次性的引导，不是空转。
        //
        // ⚠️ 诚实注记（M5c 复审 M2，单窗时代实测）：**这一行在 Windows 上删掉也全绿** —— 那一帧
        // 其实是 OS（窗口显示后的 WM_PAINT）给的。保留它是为了**非 Windows / 其它 winit 后端**，
        // 以及「没有 OS 帧时 OnDemand 也能起步」。
        self.request_redraw_for(winit_id);
    }

    /// **唯一**决定「睡多久」的地方（`resumed` / `user_event` / `about_to_wait` 都调它，实现只有这一份）。
    ///
    /// 收敛规则就在纯函数 [`plan_wake`] 里（有真值表单测），这里只做「按计划行动」：
    ///
    /// - [`WakePlan::Wait`] ⇒ [`ControlFlow::Wait`]（**睡死**：本层不造任何超时 —— 省电模式）；
    /// - [`WakePlan::WaitUntil(t)`] ⇒ `WaitUntil(t)`（睡到那一刻，期间零 CPU）；
    /// - [`WakePlan::Due`] ⇒ 记一次 `fired`（连带 `requested`）→ **无条件**请求一帧 →
    ///   把**推来的**那个 deadline 清掉（App 自己下的单：到点就该画；清掉是为了不空转）
    ///   → 本轮先 `Wait`（下一轮由 App 在 `redraw` 里重装下一个 deadline）。
    ///
    /// 为什么每个事件批次之后都要重新收敛（而不是只在 `resumed` 里设一次）：deadline 会变
    /// （App 声明改了、`wake_after` 刚推来一个），`ControlFlow` 是**当前**那一轮的状态。
    fn refresh_control_flow(&mut self, event_loop: &ActiveEventLoop) {
        let now = Instant::now();
        let armed = self.armed;
        let declared = self.app.next_deadline();
        let plan = plan_wake(now, armed, declared);
        if matches!(plan, WakePlan::Due) {
            // 兑现一次：推来的那个清掉（拉式的由 App 自己负责往前走）。
            if armed.is_some_and(|t| t <= now) {
                self.armed = None;
            }
            self.wake_stats.on_fire();
            // deadline 是 **App 级**的（不属于某扇窗）⇒ 所有活窗都该重画。
            self.request_redraw_all();
        }
        // 翻译只有这一处：`WakePlan` ⇒ `ControlFlow`（见 `plan_to_control_flow` 的单测）。
        event_loop.set_control_flow(plan_to_control_flow(plan));
    }

    /// 把一条输入事件交给**它所属的窗**（T4.4-R1 路由：[`App::window_input`]，默认实现转发
    /// [`App::input`] —— 单窗口零改动），并按**置位规则**决定要不要请求重绘。
    ///
    /// 走到这里的事件都是**已映射的输入**（hover/press/focus/text/滚轮…）；反过来，**没有**
    /// 映射成 [`InputEvent`] 的 winit 事件（`ModifiersChanged`、`CursorEntered/Left`、被丢弃的侧键…）
    /// 根本走不到这里，也就不会请求重绘 —— 「每个 winit 事件都重绘」这条被刻意避开了。
    ///
    /// `gate` 决定置位规则：普通输入看 [`App::wants_redraw`]（**为假就不请求**，这是 M5b 的省电核心），
    /// 系统类输入（`Focused`）**一律**请求（见 [`Gate`]）。置位与 T3.7 一样**定向给来源窗**
    /// （[`RunHandler::request_redraw_for`]）。`Err` 走 [`RunHandler::fail`]；`Flow::Exit` 与 `redraw` 同义。
    fn dispatch(
        &mut self,
        event_loop: &ActiveEventLoop,
        winit_id: WinitWindowId,
        ev: &InputEvent,
        gate: Gate,
    ) {
        /// `dispatch` 与「行动」分离出来的中间决定（先算完、再动 `self`，借用才不打架）。
        enum After {
            Request,
            Nothing,
            Exit,
            Fail(String),
        }
        let after = {
            // 两套 id 的换算点：winit 事件带来 winit 的 id，App 只认本层的 [`WindowId`]。
            let Some(id) = self.alive.get(&winit_id).map(|live| live.id) else {
                // 未知窗（销毁竞态里的事件）：没有属主 ⇒ 丢弃（不派发、不记账、不请求）。
                return;
            };
            // `info` 是**这个窗**的（Copy 取出，借用到此为止）。
            let Some(info) = self.table.get(id).map(|e| e.info) else {
                return;
            };
            match self.app.window_input(id, &info, ev) {
                Ok(Flow::Continue) => match gate {
                    Gate::AppDecides => {
                        // **先问 App 这一条输入改了状态没有**（回调之后问，App 才有机会更新脏标记）。
                        let wants = self.app.wants_redraw();
                        if self.counter.on_input(wants) {
                            After::Request
                        } else {
                            After::Nothing
                        }
                    }
                    Gate::Always => After::Request,
                },
                Ok(Flow::Exit) => After::Exit,
                Err(e) => After::Fail(format!("App::window_input 失败：{e}")),
            }
        };
        match after {
            After::Request => self.request_redraw_for(winit_id),
            After::Nothing => {}
            After::Exit => {
                self.exiting = true;
                event_loop.exit();
            }
            After::Fail(e) => self.fail(event_loop, e),
        }
    }

    /// 请求**某一扇**窗重绘 + **记账**（置位规则的定向版：输入/系统事件都属于**来源窗** ——
    /// 尺寸变了重画那扇、hover 变了重画那扇）。
    ///
    /// 「请求次数」的**唯一**加计数点仍是 [`FrameCounter::note_request`]（与全窗版共享同一个
    /// 真相来源）。窗已不在 `alive`（关闭竞态）时只记账 —— 与拆分前「窗口还没建时只记账」同口径。
    fn request_redraw_for(&mut self, winit_id: WinitWindowId) {
        self.counter.note_request();
        if let Some(live) = self.alive.get(&winit_id) {
            live.window.request_redraw();
        }
    }

    /// 请求**所有**活窗重绘 + 记账（App 级的「该画了」：`wake()` 答真 / deadline 到点 ——
    /// 它们不属于任何一扇窗）。**单窗口时与拆分前的 `request_redraw` 逐字同行为**
    /// （记一次账 + 那一扇窗 `request_redraw`）。
    fn request_redraw_all(&mut self) {
        self.counter.note_request();
        for live in self.alive.values() {
            live.window.request_redraw();
        }
    }

    /// 收尾：先**把账本交给 App**（[`App::on_wake_stats`]，空闲档的唯一上界落点），
    /// 再打摘要、给出最终结果（回调错误优先于事件循环自身的错误）。
    fn finish(mut self, loop_error: Option<String>) -> Result<(), String> {
        let err = self.error.or(loop_error);
        // M5c fix（复审 I1）：账本交给 App **在打印之前** —— `iters` 的上界断言只能由
        // 「声明了空闲语义的那一档」自己下，本层不做全局阈值（`Continuous` 档它本来就高）。
        self.app.on_wake_stats(&self.wake_stats);
        println!(
            "[deer-window] 事件循环结束：frames={} extent={}x{} result={}",
            self.counter.frames(),
            self.extent.width,
            self.extent.height,
            if err.is_some() { "error" } else { "ok" }
        );
        // 账本单列一行：上面那行的字段/顺序被文档与脚本按逐字匹配用着（不改它），
        // M5b 的新计数走这一行 —— 「重绘次数 / 跳过帧数」就这样数出来。
        println!(
            "[deer-window] 重绘账本：requests={} skipped={} frames={}",
            self.counter.redraw_requests(),
            self.counter.skipped_frames(),
            self.counter.frames(),
        );
        // M5c 的唤醒账本（再单列一行，同样不打乱上面两行）：`iters` 是**观测值**，**不是**本层的
        // 判据 —— 本层不设全局阈值（`Continuous` 档下它与帧数同阶是合法的）；退化成「超时打转」时
        // `frames` 确实抓不住（没人请求重绘），但抓它的门槛在**空闲档自己**那条：`wake_probe` 的
        // `DEER_WAKE_TICKS=0` 档断言 `iters` 上界（见 `WakeStats::note_iter`）。
        println!(
            "[deer-window] 唤醒账本：wake={} wake_after={} fired={} requested={} skipped={} iters={}",
            self.wake_stats.looks(),
            self.wake_stats.arms(),
            self.wake_stats.fired(),
            self.wake_stats.requested(),
            self.wake_stats.skipped(),
            self.wake_stats.iters(),
        );
        match err {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }
}

/// 派发一条输入事件后的置位规则（M5b 的四条规则里的前两条）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Gate {
    /// 普通输入：**只有** [`App::wants_redraw`] 为真才请求重绘（为假 ⇒ 记一次「跳过帧」）。
    AppDecides,
    /// 系统类输入（`Focused`）：**一律**请求重绘，不看 `wants_redraw`。
    Always,
}

impl<A: App + 'static> ApplicationHandler<Wake> for RunHandler<A> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.error.is_some() || self.configs.is_empty() {
            // 已经出过错就不再建窗；`resumed` 在部分平台会被多次调用，
            // 主窗批（`configs`）只在这里清空一次。
            return;
        }

        // **主窗批**（决策 2「主窗仍由 run() 首建」+ 决策 3「多 WindowConfig 启动」）：
        // 按顺序每项建一扇窗；第一扇就是主窗。真正的建窗动作收在 [`RunHandler::create_window`]
        // 一个点里（`spawn` 的动态建窗走的也是它）。
        for config in std::mem::take(&mut self.configs) {
            self.create_window(event_loop, &config);
            if self.error.is_some() {
                // create_window 内部已经 fail()（打印 + 记住 + 请求退出），别再往下建。
                return;
            }
        }

        // **纯阻塞**：没有事件就睡死（旧版是 `Poll`：一直空转问「有没有事」）。
        // 刻意**不**用 `WaitUntil` 兜底 —— 超时唤醒就是隐藏的空转，省电模式会名存实亡。
        // 唯一的例外是 App **自己声明**的 deadline（M5c）：那时用 `WaitUntil(那个时刻)`，
        // 由 `refresh_control_flow` 按 `App::next_deadline` / `Waker::wake_after` 收敛。
        // App 什么都没声明 ⇒ 走 `Wait` 这一支（与 M5b 逐字同行为）。
        self.refresh_control_flow(event_loop);
    }

    /// **用户事件到达**（来源只有本层的 [`Waker`] 与 [`WindowSpawner`]；`Wake` 是私有类型，
    /// App 塞不进来别的）。
    ///
    /// 三条路（语义差异见 `lib.rs` 的 crate 文档，别把三者写成一样）：
    ///
    /// - `Wake::Look`（[`Waker::wake`]）：**与输入同一把尺** —— 先问 [`App::wants_redraw`]，
    ///   答真才请求一帧（答假记一次 `skipped`，**不**混进输入的「跳过帧」口径）；
    ///   唤醒是 **App 级**的 ⇒ 请求发给**所有**活窗。
    /// - `Wake::At(t)`（[`Waker::wake_after`]）：只**装** deadline（记一次 `arms`），
    ///   **不画** —— 到点由 [`RunHandler::refresh_control_flow`] 兑现（那时才请求）。
    /// - `Wake::Spawn(config)`（[`WindowSpawner::spawn_window`]，T4.4-R1）：**排队的建窗请求
    ///   到站** —— 这里正持 [`ActiveEventLoop`]，就是决策 2 说的「安全点」，真正建窗
    ///   （[`RunHandler::create_window`]，与主窗批同一条路）。错误/退出中到达的请求被丢弃
    ///   （与 `Look`/`At` 同一守卫）。
    ///
    /// 收尾处**显式重算** `ControlFlow`：`wake_after` 的 deadline 与 spawn 的建窗都是在
    /// 这一刻落地的。不靠「`about_to_wait` 反正马上会来一次」——那种依赖 winit 事件顺序的
    /// 推理，读代码的人不该被迫做。
    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: Wake) {
        if self.error.is_some() || self.exiting {
            return;
        }
        match event {
            Wake::Look => {
                let wants = self.app.wants_redraw();
                if self.wake_stats.on_look(wants) {
                    self.request_redraw_all();
                }
            }
            Wake::At(at) => {
                self.wake_stats.on_arm();
                // 排了多次取最近的那个：早的先到点。
                self.armed = earliest(self.armed, Some(at));
            }
            Wake::Spawn(config) => self.create_window(event_loop, &config),
        }
        self.refresh_control_flow(event_loop);
    }

    /// 每轮事件循环的收口：**记一次迭代** + 收敛 `ControlFlow`（含 deadline 到点的兑现）。
    ///
    /// 为什么放在这里而不是 `new_events`：`AboutToWait` 是 winit 在**每次**要睡下去之前
    /// 必发的事件（`NewEvents` 只在从 OS 收到新事件时发）—— 「该睡多久」正该在这时候定。
    /// 迭代计数（`iters`）也在这儿数：它是「空闲时事件循环醒了几次」这个**可数观测值** ——
    /// **不是**判据（本层不设全局阈值；下断言的是空闲档自己的上界，见 [`WakeStats::note_iter`]）。
    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        self.wake_stats.note_iter();
        if self.error.is_some() || self.exiting {
            // 已经在收尾（`exit()` 请求过了）：不要再请求帧，也不要再装 deadline。
            event_loop.set_control_flow(ControlFlow::Wait);
            return;
        }
        self.refresh_control_flow(event_loop);
    }

    /// winit 把窗口事件连同**它的**窗口 id 一起送来（T4.4-R1 的路由入口）：先换算成本层的
    /// [`WindowId`]（`alive` 表是两套 id 的唯一换算点），再把事件落到**该窗自己的**纯状态上
    /// （[`WindowEntry`]：光标账本 / DPI 账本 / IME 状态 / info）。未知窗的事件（销毁竞态）
    /// 直接丢弃 —— 与「没有窗口就没有输入」同一口径。
    ///
    /// 单窗口行为与拆分前**逐字一致**：只有一扇窗时，这里的每条路由都落在那唯一一份状态上。
    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WinitWindowId,
        event: WindowEvent,
    ) {
        if self.error.is_some() || self.exiting {
            return;
        }
        // 两套 id 的换算点（后续所有分支都要用本层 id 查 `table`）。
        let Some(id) = self.alive.get(&window_id).map(|live| live.id) else {
            return;
        };
        match event {
            WindowEvent::Resized(size) => {
                let Some(entry) = self.table.get_mut(id) else { return };
                let extent = Extent { width: size.width, height: size.height };
                // `WindowInfo` 跟着更新：`window_input` 拿到的尺寸必须与 `resized` 同口径，
                // 否则输入坐标会按旧尺寸解释（点偏）。
                entry.info.extent = extent;
                // 收尾摘要的口径 = 主窗的最近尺寸（单窗时就是那扇 —— 与拆分前一致）。
                if self.main == Some(id) {
                    self.extent = extent;
                }
                if let Err(e) = self.app.resized(size.width, size.height) {
                    self.fail(
                        event_loop,
                        format!("App::resized({}x{}) 失败：{e}", size.width, size.height),
                    );
                    return;
                }
                // **系统事件一律置位**：尺寸变了必须重画（不然窗口留一片脏区），
                // 而且这不看 `App::wants_redraw`、也不看策略 —— 系统说「要重画」就是「要重画」。
                // （T4.4-R1：定向给**变了的那扇窗**。）
                self.request_redraw_for(window_id);
            }
            WindowEvent::ScaleFactorChanged { scale_factor, inner_size_writer: _ } => {
                // AF-3（Q4=**只透传**）：把 OS 报的 scale_factor 原样记账并转发给 App，
                // **本层不换算任何坐标**（事件坐标与 `WindowInfo::extent` 仍是物理像素）。
                //
                // `inner_size_writer` **刻意不碰**（`_`）：winit 的语义是「写它 = 改窗口
                // 物理尺寸；不写 = 窗口保持现有物理像素」。保持物理尺寸就是最纯粹的透传 ——
                // 「按 OS 建议把窗口放大」需要复刻 winit 的 `logical×new_scale` 计算，
                // 那是一层换算，Q4 裁断不做。⇒ **不会**有 `Resized` 跟随（物理尺寸没变）。
                //
                // T4.4-R1：DPI 账本与 `info.scale_factor` 都是该窗**自己**的那一份。
                let ev = {
                    let Some(entry) = self.table.get_mut(id) else { return };
                    let ev = entry.dpi.on_scale_factor_changed(scale_factor);
                    entry.info.scale_factor = scale_factor;
                    ev
                };
                // **系统事件一律置位**（与 `Resized`/`Focused` 同档，六条规则表里的一类）：
                // DPI 变了必须重画一帧。不看 `App::wants_redraw`。
                self.dispatch(event_loop, window_id, &ev, Gate::Always);
            }
            WindowEvent::RedrawRequested => {
                let result = self.app.redraw();
                match self.counter.on_redraw(result) {
                    Ok(()) => {
                        if self.counter.exit_requested() {
                            self.exiting = true;
                            event_loop.exit();
                        } else if self.policy.wants_next_frame() {
                            // `Continuous`：每画完一帧再请求下一帧（这一条就是「连续重绘」的发动机）；
                            // T4.4-R1：续帧续在**刚画完的那扇窗**上。`OnDemand` 什么都不做 ⇒
                            // 没有输入/系统事件就**不再有下一帧**（省电）。
                            self.request_redraw_for(window_id);
                        }
                    }
                    Err(e) => self.fail(event_loop, format!("App::redraw 失败：{e}")),
                }
            }
            // 关闭语义（决策 4）：`CloseRequested` **只关被请求的那扇窗**（`Arc` 落下 ⇒
            // winit 销毁它）；**全部窗口关闭** ⇒ 事件循环退出。App 仍有一票否决
            // （`close_requested() == Continue` ⇒ 哪扇都不关 —— 与拆分前同语义）。
            WindowEvent::CloseRequested => {
                if self.app.close_requested() == Flow::Exit {
                    match self.table.close(id) {
                        CloseOutcome::Closed { last } => {
                            self.alive.remove(&window_id);
                            if last {
                                self.exiting = true;
                                event_loop.exit();
                            }
                        }
                        CloseOutcome::Unknown => {}
                    }
                }
            }
            // 窗口被外部销毁（或上面 close 落地后的回执）：账面清理**必须幂等**
            // （[`WindowTable::close`] 对未知 id 是 `Unknown` ⇒ 无动作）。
            // 最后一扇窗没了 ⇒ 退出（决策 4 的另一半：「全部窗口关闭 ⇒ 事件循环退出」）。
            WindowEvent::Destroyed => {
                let _ = self.table.close(id);
                self.alive.remove(&window_id);
                if self.table.is_empty() {
                    self.exiting = true;
                    event_loop.exit();
                }
            }

            // ——— 输入事件的翻译（winit 事件 → 本层 `InputEvent` → 该窗 → `App::window_input`）———
            // 普通输入一律走 `Gate::AppDecides`：**只有** `App::wants_redraw` 为真才请求重绘，
            // 且**定向给来源窗**。
            WindowEvent::CursorMoved { position, .. } => {
                // **不做窗口边界过滤**（`PointerRoute` 约定 1）：拖出去的移动照样投给 App，
                // interaction 层的指针捕获（按下即捕获，D7）靠这条流。
                // T4.4-R1：账本是**该窗自己的**（A 窗的拖拽不会污染 B 窗的最近光标）。
                let ev = {
                    let Some(entry) = self.table.get_mut(id) else { return };
                    entry.pointer.on_cursor_moved(position.x as f32, position.y as f32)
                };
                self.dispatch(event_loop, window_id, &ev, Gate::AppDecides);
            }
            WindowEvent::MouseInput { state, button, .. } => {
                let Some(button) = map_mouse_button(button) else {
                    // 侧键/未知键：`PointerButton`（接口冻结）表示不了 ⇒ 整条丢弃。
                    // **不**派发、**不**请求重绘（也就不会产生「不明点击」）。
                    return;
                };
                // 坐标 = **该窗**最近一次光标位置（`PointerRoute` 约定 2）：拖出去在外面抬起，
                // `PointerUp` 带最后已知位置，结算归 interaction 层的捕获者。
                let ev = {
                    let Some(entry) = self.table.get_mut(id) else { return };
                    entry.pointer.on_mouse_button(button, state == ElementState::Pressed)
                };
                self.dispatch(event_loop, window_id, &ev, Gate::AppDecides);
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let (dx, dy) = map_wheel(&delta);
                self.dispatch(event_loop, window_id, &InputEvent::Wheel { dx, dy }, Gate::AppDecides);
            }
            WindowEvent::KeyboardInput { event, .. } => {
                // 合成的按键事件（失焦时系统补发的 KeyUp）**照发**：上层的「按住的键」
                // 靠它清掉，丢掉会留下按下的残留状态。
                let key = map_key(&event.logical_key, self.mods);
                let mods = self.mods;
                match event.state {
                    ElementState::Pressed => {
                        // **物理键先发**：`Char('a')`/`Enter`/`Tab`… 一律先来 KeyDown。
                        // T3.6：winit 的 `event.repeat` 首次建模进事件 —— OS 的按键重复
                        // （长按补发）从此**可区分**；`Gate::AppDecides` 照旧。
                        self.dispatch(
                            event_loop,
                            window_id,
                            &InputEvent::KeyDown { key, mods, repeat: event.repeat },
                            Gate::AppDecides,
                        );
                        // **文本与物理键分开**：IME 预编辑中由 `Ime::Commit` 负责文本
                        // （免得中文重复上屏）；`Enter/Tab/Backspace/Esc` 的 text 是
                        // 控制字符，被 `printable_text` 挡掉 ⇒ 它们只有 KeyDown。
                        // T4.4-R1：预编辑状态是**该窗自己**的。
                        let composing =
                            self.table.get(id).is_some_and(|entry| entry.ime_composing);
                        let text = if composing {
                            None
                        } else {
                            printable_text(event.text.as_deref())
                        };
                        if let Some(text) = text {
                            self.dispatch(
                                event_loop,
                                window_id,
                                &InputEvent::TextInput { text },
                                Gate::AppDecides,
                            );
                        }
                    }
                    ElementState::Released => {
                        self.dispatch(
                            event_loop,
                            window_id,
                            &InputEvent::KeyUp { key, mods },
                            Gate::AppDecides,
                        );
                    }
                }
            }
            WindowEvent::Ime(ime) => match ime {
                Ime::Commit(text) => {
                    // 空提交（预编辑被清掉）不算输入 ⇒ 不派发、不重绘。
                    if let Some(entry) = self.table.get_mut(id) {
                        entry.ime_composing = false;
                    }
                    if !text.is_empty() {
                        self.dispatch(
                            event_loop,
                            window_id,
                            &InputEvent::TextInput { text },
                            Gate::AppDecides,
                        );
                    }
                }
                Ime::Preedit(text, _) => {
                    // ① 仍用它抑制按键文本（避免「预编辑中按键的 text」与「Commit」双写）；
                    // ② **并且真的派发**（T3.4 起预编辑被建模了 —— UI 层要拿它画下划线）。
                    // `Gate::Always`：预编辑是**视觉**状态（与焦点同类），一变就该重画。
                    // T4.4-R1：状态记在**该窗自己**的账上。
                    if let Some(entry) = self.table.get_mut(id) {
                        entry.ime_composing = !text.is_empty();
                    }
                    self.dispatch(
                        event_loop,
                        window_id,
                        &InputEvent::ImePreedit { text },
                        Gate::Always,
                    );
                }
                Ime::Enabled | Ime::Disabled => {
                    if let Some(entry) = self.table.get_mut(id) {
                        entry.ime_composing = false;
                    }
                }
            },
            WindowEvent::Focused(focused) => {
                // **每窗独立焦点**（决策 5）：winit 本就按窗发 `Focused`，这里按 id 路由 ——
                // App 收到的 `FocusChanged` 只关于**这一扇窗**（每窗一份 UiState 焦点的前提）。
                // **系统事件一律置位**：焦点变化算系统事件（`Gate::Always`），不看
                // `wants_redraw` —— 光标/焦点框这类视觉状态一变，界面就该重画。
                self.dispatch(
                    event_loop,
                    window_id,
                    &InputEvent::FocusChanged { focused },
                    Gate::Always,
                );
            }
            WindowEvent::Occluded(false) => {
                // 窗口**重新暴露**（从被遮挡状态回来）：必须重画（期间交换链可能已过期）⇒ 系统事件一律置位。
                // 诚实说明：winit 0.30 的文档明确写 Windows **不支持**这个事件（iOS/Android 才有），
                // 所以这一臂在 Windows 上是「接线接好了但系统不喂」—— 不假装它在 Windows 上跑过。
                self.request_redraw_for(window_id);
            }
            WindowEvent::ModifiersChanged(modifiers) => {
                // 只记账：修饰键在 `InputEvent` 里是键盘事件的**字段**，没有单独的事件 ⇒
                // 不派发、也不请求重绘（单独按 Shift 不会让界面有任何变化）。
                // 键盘是连接级状态 ⇒ 记在 handler 的一份上（不是每窗一份）。
                self.mods = map_mods(modifiers.state());
            }
            // 其余事件（触摸/手势/拖放/CursorEntered…）本期不转发：见模块文档的「仍未接线」。
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// 指针路由账本的单测（T3.7 窗口侧半；不建窗口、不跑事件循环 —— 与
// `tests/input_map.rs` 同一纪律：测的是纯函数，喂的是 winit 事件的**字段**）
// ---------------------------------------------------------------------------

#[cfg(test)]
mod pointer_route_tests {
    use super::PointerRoute;
    use crate::{InputEvent, PointerButton};

    /// **约定 1：移动不做窗口边界过滤** —— 拖出窗口的坐标照样合成 `PointerMoved`，
    /// 且账本吃下它（后续抬起带这个窗外位置）。捕获语义（interaction 层）靠这条流。
    #[test]
    fn moves_outside_the_window_are_delivered_verbatim_and_ledgered() {
        let mut r = PointerRoute::default();
        // 前置：账本初值是既有约定的 (0,0) —— 否则下面的「窗外位置生效」是空话。
        let probe = r.on_mouse_button(PointerButton::Left, false);
        assert_eq!(
            probe,
            InputEvent::PointerUp { button: PointerButton::Left, x: 0.0, y: 0.0 },
            "前置：初始账本是 (0,0)（既有约定）"
        );

        // 拖出窗口：负坐标 / 远超窗口尺寸的坐标都**原样**投递（不许过滤、不许夹取）。
        for (x, y) in [(-37.5f32, 12.0f32), (9_999.0, -2.0), (-1.0, -1.0)] {
            let ev = r.on_cursor_moved(x, y);
            assert_eq!(
                ev,
                InputEvent::PointerMoved { x, y },
                "窗外移动必须逐值透传，实际 {ev:?}"
            );
        }

        // 账本吃下了最后一个窗外位置：随后的抬起带**它**（不是 (0,0)）。
        let up = r.on_mouse_button(PointerButton::Left, false);
        assert_eq!(
            up,
            InputEvent::PointerUp { button: PointerButton::Left, x: -1.0, y: -1.0 },
            "窗外抬起必须带最后已知（窗外）位置 —— 拖出去松手也是完整的一次点击"
        );
    }

    /// **约定 2：按键事件用最近一次光标位置** —— 抬起带**最后**位置，不是按下的位置。
    #[test]
    fn button_events_carry_the_last_known_cursor_not_the_press_point() {
        let mut r = PointerRoute::default();
        let down = {
            let ev = r.on_cursor_moved(30.0, 20.0);
            assert_eq!(ev, InputEvent::PointerMoved { x: 30.0, y: 20.0 }, "前置：移动透传");
            r.on_mouse_button(PointerButton::Left, true)
        };
        assert_eq!(
            down,
            InputEvent::PointerDown { button: PointerButton::Left, x: 30.0, y: 20.0 },
            "按下位置 = 当时最近的移动位置"
        );

        // 拖到别处再抬起：抬起带**最后**位置（30,20 会被判为「位置错」的红）。
        let _ = r.on_cursor_moved(120.0, 400.0);
        let up = r.on_mouse_button(PointerButton::Left, false);
        assert_eq!(
            up,
            InputEvent::PointerUp { button: PointerButton::Left, x: 120.0, y: 400.0 },
            "抬起必须带最后已知位置，不是按下时的 (30,20)"
        );
    }

    /// **变异自检**：账本必须停在**最后一次移动**上 —— 记账被改坏（没更新/只更新一次）
    /// 时这条会红（停在更早的位置或初值）。它证明约定 1 的第二条（账本吃下窗外位置）
    /// 有判别力，不是摆设。
    #[test]
    fn ledger_stays_on_the_last_move() {
        let mut r = PointerRoute::default();
        // 只移动、不按键，再按键：账本必须停在**最后一次移动**上。
        r.on_cursor_moved(5.0, 5.0);
        r.on_cursor_moved(500.0, 5.0);
        let up = r.on_mouse_button(PointerButton::Middle, false);
        assert_eq!(
            up,
            InputEvent::PointerUp { button: PointerButton::Middle, x: 500.0, y: 5.0 },
            "账本必须跟到最后一次移动（若是 (5,5) ⇒ 记账被改坏；若是 (0,0) ⇒ 移动没记账）"
        );
    }
}

// ---------------------------------------------------------------------------
// DPI 账本的单测（AF-3；纯类型，钉「只透传、不换算」这条红线）
// ---------------------------------------------------------------------------

#[cfg(test)]
mod scale_ledger_tests {
    use super::ScaleLedger;
    use crate::InputEvent;

    /// **红线：值原样透传** —— 事件携带的就是 OS 报的那个 `f64`（不取整、不经 `f32`），
    /// 且账本与事件**同值**（`WindowInfo::scale_factor` 与它同源 ⇒ App 看到的口径一致）。
    ///
    /// `1.1` 是故意的：它**不能**被 `f32` 精确表示 —— 变异「把值经 `f32` 折腾一遍」
    /// （`v as f32 as f64`）会让这条立刻变红（1.1f32 回到 f64 是
    /// 1.100_000_023_841_857_9，与 1.1f64 不相等），而 1.25 / 2.0 这类恰好可表示的
    /// 值抓不住那种坏法。
    #[test]
    fn scale_factor_is_forwarded_verbatim_and_ledgered() {
        // 前置：初值就是建窗时 OS 报的那个（`window_info` 的同源值）。
        let mut dpi = ScaleLedger::at_creation(1.0);
        assert_eq!(dpi.current(), 1.0, "前置：建窗初值必须先立住，否则后面无从谈『变了』");

        for new in [1.25f64, 1.1f64, 2.0f64] {
            let ev = dpi.on_scale_factor_changed(new);
            assert_eq!(
                ev,
                InputEvent::ScaleFactorChanged { scale_factor: new },
                "事件必须携带 OS 报的原值（透传红线）：{new}"
            );
            assert_eq!(
                dpi.current(),
                new,
                "账本必须与事件同值（WindowInfo::scale_factor 与它同源 ⇒ 口径一致）：{new}"
            );
        }
    }

    /// **变异自检**：账本不更新（只发事件、不改记账）⇒ 这条红 —— 它钉住
    /// 「`info.scale_factor` 跟着最近一条事件走」这半边契约（另一半在上面那条）。
    #[test]
    fn ledger_must_track_the_latest_change() {
        let mut dpi = ScaleLedger::at_creation(1.0);
        let _ = dpi.on_scale_factor_changed(1.5);
        let _ = dpi.on_scale_factor_changed(2.0);
        assert_eq!(
            dpi.current(),
            2.0,
            "账本必须停在最近一次变化上（停在 1.5 ⇒ 记账被改坏；停在 1.0 ⇒ 没记账）"
        );
    }
}

// ---------------------------------------------------------------------------
// 多窗口路由 / 关闭语义的单测（T4.4-R1；不建窗口、不跑事件循环 —— 与
// `pointer_route_tests` 同一纪律：测的是纯逻辑，`Arc<Window>` 根本不出现。
// `WindowId` 用本层自发的序号（`from_raw`）造两个不同的键 —— winit 的 id 没有
// 公开构造，这正是「本层自己发 id」这个设计的原因之一）
// ---------------------------------------------------------------------------

#[cfg(test)]
mod window_table_tests {
    use super::{CloseOutcome, WindowEntry, WindowTable, WindowId};
    use crate::{InputEvent, PointerButton, WindowInfo, raw_handle_from_win32};
    use deer_gpu::Extent;

    /// 造一份假窗的纯状态（句柄值是编的 —— 纯逻辑测试不碰真窗口；scale 用来区分窗）。
    fn entry(scale_factor: f64) -> WindowEntry {
        WindowEntry::at_creation(WindowInfo {
            raw: raw_handle_from_win32(0x1A2B, 0x7FF6_0000),
            extent: Extent { width: 320, height: 200 },
            scale_factor,
        })
    }

    fn id(n: u64) -> WindowId {
        WindowId::from_raw(n)
    }

    /// **事件到对的窗**（决策 1/6）：喂给 B 的移动只进 **B 的**账本；A 的账本纹丝不动、
    /// A 的 info 也不被 B 的事件改掉 —— 「每窗一份状态」是路由表存在的意义。
    #[test]
    fn input_routes_to_the_window_it_belongs_to() {
        let mut t = WindowTable::new();
        t.insert(id(1), entry(1.0));
        t.insert(id(2), entry(2.0));
        assert_eq!(t.len(), 2, "前置：两扇窗都得在表上");

        // B(2) 收到一条**窗外**移动；随后 B 的抬起带它（T3.7 捕获语义依赖的账本行为）。
        let moved = {
            let e = t.get_mut(id(2)).expect("B 在表上");
            e.pointer.on_cursor_moved(-37.5, 12.0)
        };
        assert_eq!(moved, InputEvent::PointerMoved { x: -37.5, y: 12.0 });
        let up_b = {
            let e = t.get(id(2)).expect("B 在表上");
            e.pointer.on_mouse_button(PointerButton::Left, false)
        };
        assert_eq!(
            up_b,
            InputEvent::PointerUp { button: PointerButton::Left, x: -37.5, y: 12.0 },
            "B 的抬起必须带 **B 的**最近光标位置"
        );

        // A(1) 从头到尾没收到事件：它的抬起必须还带它自己的 (0,0) 初值 ——
        // 被 B 的移动污染（共享账本）或停在别处（路由错了窗）都会在这里红。
        let up_a = {
            let e = t.get(id(1)).expect("A 在表上");
            e.pointer.on_mouse_button(PointerButton::Left, false)
        };
        assert_eq!(
            up_a,
            InputEvent::PointerUp { button: PointerButton::Left, x: 0.0, y: 0.0 },
            "A 没收到过移动 ⇒ 抬起带它自己的初值，不能被 B 的移动污染"
        );

        // DPI 同理：B 的 ScaleFactorChanged 只动 B 的账本与 info。
        let ev_b = {
            let e = t.get_mut(id(2)).expect("B 在表上");
            let ev = e.dpi.on_scale_factor_changed(1.5);
            e.info.scale_factor = 1.5; // 与 RunHandler 同一步（info 与账本同源同值）
            ev
        };
        assert_eq!(ev_b, InputEvent::ScaleFactorChanged { scale_factor: 1.5 });
        assert_eq!(
            t.get(id(1)).expect("A 在表上").info.scale_factor,
            1.0,
            "A 的 DPI 不能被 B 的事件改掉"
        );
        assert_eq!(
            t.get(id(2)).expect("B 在表上").info.scale_factor,
            1.5,
            "B 的 info 必须跟着它自己的 DPI 事件走"
        );
    }

    /// **关闭语义状态机**（决策 4）：只关被请求的那扇；还剩别的窗 ⇒ `last=false`
    /// （事件循环继续）；关到最后一扇 ⇒ `last=true`（该退出）；关不存在的 id ⇒
    /// `Unknown` 且**不动表**（幂等 —— `Destroyed` 回执与关闭竞态靠它不炸）。
    #[test]
    fn close_semantics_state_machine() {
        let mut t = WindowTable::new();
        t.insert(id(1), entry(1.0));
        t.insert(id(2), entry(1.0));
        t.insert(id(3), entry(1.0));

        // 关中间那扇：还剩两扇 ⇒ 不退出。
        assert_eq!(t.close(id(2)), CloseOutcome::Closed { last: false }, "还剩别的窗 ⇒ 不退出");
        assert_eq!(t.len(), 2);

        // 再关同一扇（重复 CloseRequested / Destroyed 回执）：Unknown ⇒ 无动作。
        assert_eq!(t.close(id(2)), CloseOutcome::Unknown, "未知/已关的 id 必须是 Unknown");
        assert_eq!(t.len(), 2, "Unknown 不得动表（幂等）");

        // 关到最后一扇：last=true ⇒ handler 据此请求事件循环退出。
        assert_eq!(t.close(id(1)), CloseOutcome::Closed { last: false });
        assert_eq!(t.close(id(3)), CloseOutcome::Closed { last: true }, "最后一扇 ⇒ 该退出");
        assert!(t.is_empty());
    }
}

// ---------------------------------------------------------------------------
// `App::window_input` 默认转发的单测（T4.4-R1，决策 1；trait 级纯逻辑 —— 不碰事件循环）
// ---------------------------------------------------------------------------

#[cfg(test)]
mod window_input_forwarding_tests {
    use super::{App, Flow, WindowId};
    use crate::{InputEvent, WindowInfo, raw_handle_from_win32};
    use deer_gpu::Extent;

    fn info() -> WindowInfo {
        WindowInfo {
            raw: raw_handle_from_win32(0x1A2B, 0x7FF6_0000),
            extent: Extent { width: 320, height: 200 },
            scale_factor: 1.0,
        }
    }

    /// 只实现 `input` 的 App（**单窗口用户的既有写法**）：默认 `window_input` 必须把事件
    /// **原样转发**进 `input` —— 「单窗口用户零改动」就是这一条（决策 1）。
    struct LegacyApp {
        inputs: Vec<InputEvent>,
    }

    impl App for LegacyApp {
        fn init(&mut self, _info: &WindowInfo) -> Result<(), String> {
            Ok(())
        }
        fn redraw(&mut self) -> Result<Flow, String> {
            Ok(Flow::Continue)
        }
        fn input(&mut self, _info: &WindowInfo, ev: &InputEvent) -> Result<Flow, String> {
            self.inputs.push(ev.clone());
            Ok(Flow::Continue)
        }
    }

    /// **覆盖了** `window_input` 的多窗口 App：它必须拿到 (id, 事件)，而 `input` **不再被调**
    /// （转发只发生在默认实现里 —— 不然两条路都会响，多窗口 App 会被同一事件打两次）。
    struct MultiApp {
        calls: Vec<(WindowId, InputEvent)>,
        legacy_inputs: usize,
    }

    impl App for MultiApp {
        fn init(&mut self, _info: &WindowInfo) -> Result<(), String> {
            Ok(())
        }
        fn redraw(&mut self) -> Result<Flow, String> {
            Ok(Flow::Continue)
        }
        fn window_input(
            &mut self,
            id: WindowId,
            _info: &WindowInfo,
            ev: &InputEvent,
        ) -> Result<Flow, String> {
            self.calls.push((id, ev.clone()));
            Ok(Flow::Continue)
        }
        fn input(&mut self, _info: &WindowInfo, _ev: &InputEvent) -> Result<Flow, String> {
            self.legacy_inputs += 1;
            Ok(Flow::Continue)
        }
    }

    #[test]
    fn default_window_input_forwards_to_input() {
        let mut app = LegacyApp { inputs: Vec::new() };
        let ev = InputEvent::PointerMoved { x: 3.0, y: 4.0 };
        // 两条**不同的** id 都要转发（默认实现不挑窗 —— 单窗口时代它只有一个 id 而已）。
        for wid in [WindowId::from_raw(1), WindowId::from_raw(2)] {
            let flow = app
                .window_input(wid, &info(), &ev)
                .expect("默认转发不该报错");
            assert_eq!(flow, Flow::Continue, "默认实现不改变 Flow 语义");
        }
        assert_eq!(app.inputs.len(), 2, "每条 window_input 都应转发进 input");
        assert_eq!(app.inputs[0], ev, "转发的是**同一条**事件（不丢不改）");
    }

    #[test]
    fn overridden_window_input_receives_its_own_window_and_silences_input() {
        let mut app = MultiApp { calls: Vec::new(), legacy_inputs: 0 };
        let ev = InputEvent::TextInput { text: "hi".to_string() };
        let flow = app
            .window_input(WindowId::from_raw(2), &info(), &ev)
            .expect("覆盖者自己决定语义；本实现返回 Continue");
        assert_eq!(flow, Flow::Continue);
        assert_eq!(app.calls.len(), 1, "覆盖者恰好收到这一次调用");
        assert_eq!(
            app.calls[0].0,
            WindowId::from_raw(2),
            "覆盖者必须拿到**事件所属窗**的 id（路由的钥匙）"
        );
        assert_eq!(app.calls[0].1, ev, "事件原样抵达");
        assert_eq!(
            app.legacy_inputs, 0,
            "覆盖 window_input 之后 input **不再被调**（转发只在默认实现里）"
        );
    }
}
