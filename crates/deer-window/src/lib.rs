//! # deer-window —— 窗口层（M2b）
//!
//! 负责「原生窗口 + 事件循环」，并把窗口的原生句柄以 **HAL 的不透明形式**
//! （[`deer_gpu::RawWindowHandle`]）交给上层。渲染后端（`deer-vk`）**不依赖 winit**：
//! 它只认那个不透明句柄 —— 这样「加一个平台 = 换窗口实现」，而不是「换整个渲染栈」。
//!
//! **本 crate 是本 workspace 唯一引入第三方依赖的地方**（`winit`）。理由与替代方案
//! （自写 Win32）的对比登记在 `ROADMAP.md` 的 Q-1 里。
//!
//! **分层归属（2026-10-02 v2；物理拆分 2026-10-04 落地）**：本 crate 按 `docs/ARCHITECTURE.md`
//! §2.3 分成两层 —— **L1 DisplayServer**（winit / 窗口句柄 / 输入翻译 / DPI 的平台映射）在
//! `display` 模块（`src/display.rs`），**L3 host**（`App` / `Waker` / `RedrawPolicy` 的事件循环
//! 与脏重绘）在 `host` 模块（`src/host.rs`）。本文件只做**模块声明 + `pub use` 枢纽**：
//! 公开路径与拆分前**逐字相同**（`deer_window::App` 等一个都没变），两模块之间只多出
//! `pub(crate)` 的内部项。
//!
//! ## 用法
//!
//! ```no_run
//! use std::time::{Duration, Instant};
//!
//! use deer_window::{
//!     App, Flow, InputEvent, Key, RedrawPolicy, Waker, WindowConfig, WindowInfo, run,
//! };
//!
//! struct MyApp {
//!     /// M5c：建窗后由 [`App::wake_handle`] 交过来的唤醒句柄（不实现那个方法就一直是 `None`）。
//!     waker: Option<Waker>,
//!     /// M5c：定时动画「下一帧要画的时间」（拉式的 [`App::next_deadline`] 报的就是它）。
//!     deadline: Option<Instant>,
//! }
//!
//! impl App for MyApp {
//!     fn init(&mut self, info: &WindowInfo) -> Result<(), String> {
//!         println!("窗口 {}x{}，HWND=0x{:X}", info.extent.width, info.extent.height, info.raw.handle);
//!         Ok(()) // 这里创建 Vulkan 设备 / 交换链（deer-vk 只吃 info.raw）
//!     }
//!     /// M5c：建好窗后**调一次**，把唤醒句柄交给你 —— **存下来**就能在 `OnDemand`（省电）
//!     /// 下被非窗口事件唤醒。默认实现什么都不做（老实现一行都不用改）。
//!     fn wake_handle(&mut self, waker: Waker) {
//!         println!("拿到唤醒句柄：省电模式下也能按时间被叫醒");
//!         self.waker = Some(waker);
//!     }
//!     /// M5c：**我希望被唤醒的最近时刻**（拉式；默认 `None` = 不需要，事件循环就睡死）。
//!     /// ⚠️ 必须给出**固定**的时刻并自己往前推（别返回 `Instant::now() + …`，理由见模块文档）。
//!     fn next_deadline(&self) -> Option<Instant> {
//!         self.deadline
//!     }
//!     fn redraw(&mut self) -> Result<Flow, String> {
//!         self.deadline = None; // 这一帧画完了 ⇒ 先把上一个预约撤掉
//!         // 60fps 的定时动画：这一帧画完，**排下一次唤醒**（不睡线程 —— 只装一个 deadline）。
//!         // 两种写法等价：这里用推式的 `wake_after`，也可以改成
//!         // `self.deadline = Some(Instant::now() + Duration::from_millis(16));`（拉式）。
//!         if let Some(waker) = &self.waker {
//!             waker.wake_after(Duration::from_millis(16));
//!         }
//!         Ok(Flow::Continue)
//!     }
//!     fn input(&mut self, _info: &WindowInfo, ev: &InputEvent) -> Result<Flow, String> {
//!         match ev {
//!             InputEvent::PointerDown { x, y, .. } => println!("按下 ({x}, {y})"),
//!             InputEvent::TextInput { text } => println!("文本 {text}"),
//!             // 物理键与文本是**分开**的两条事件：Esc/Tab/Enter 只走 KeyDown。
//!             InputEvent::KeyDown { key: Key::Escape, .. } => return Ok(Flow::Exit),
//!             _ => {}
//!         }
//!         Ok(Flow::Continue)
//!     }
//!     /// M5b：**这一条输入改了状态吗**？为真才请求重绘（默认 `false` = 不请求）。
//!     /// 真实实现里应当反映刚才那条 `input` 是否真的改了界面状态。
//!     fn wants_redraw(&self) -> bool {
//!         false
//!     }
//!     /// M5b：重绘策略。默认 `OnDemand`（省电，空闲时零重绘）；
//!     /// 要连续动画的实现返回 `Continuous`（每画完一帧续下一帧，空闲也烧 CPU）。
//!     /// **M5c 起**：按时间自己推进的动画**不必**再退化成 `Continuous` —— 用 [`Waker`] 即可。
//!     fn redraw_policy(&self) -> RedrawPolicy {
//!         RedrawPolicy::OnDemand
//!     }
//! }
//!
//! # fn main() -> Result<(), String> {
//! run(
//!     WindowConfig::new("demo", 800, 600),
//!     MyApp { waker: None, deadline: None },
//! )
//! # }
//! ```
//!
//! ## 边界（本里程碑不做的事，别当成已实现）
//!
//! - **只有 Windows** 的句柄填法实现了：Win32 之外的原生窗口会明确返回 [`Err`]
//!   （见 [`UNSUPPORTED_PLATFORM_MSG`]），绝不静默填 0。窗口本身照旧用 winit，
//!   所以别的平台「能开窗」，只是句柄交给 HAL 这一步还没实现。
//! - **输入事件**（M5-1 起已接通）：`CursorMoved` / `MouseInput` / `MouseWheel` /
//!   `KeyboardInput` / `Ime` / `Focused` 会被翻成 **本层自己的** [`InputEvent`]（winit 类型
//!   不进回调签名）后交给 [`App::input`]；`ModifiersChanged` 只做内部记账（修饰键随键盘事件
//!   的 `mods` 字段透传），不派发、也不触发重绘。
//! - **仍未接线的输入面**（别当成已实现）：触摸/手势（`Touch`/`PinchGesture`）、拖放、
//!   `DeviceEvent`（原始设备事件）、物理键码（`physical_key`/scancode）、键盘布局无关的快捷键。
//!   （已先后摘出本栏：**IME 预编辑** —— T3.4 起 `Ime::Preedit` ⇒ [`InputEvent::ImePreedit`]
//!   真的派发；**按键重复** —— T3.6 起 winit 的 `event.repeat` 进 `KeyDown.repeat`。）
//! - **DPI**（AF-3）：`scale_factor` **只透传** —— `WindowInfo::scale_factor` 初值 +
//!   `InputEvent::ScaleFactorChanged` 事件（OS 报多少给多少）；坐标与尺寸仍是
//!   **物理像素**，不做任何缩放换算（布局是像素级纯函数）；窗口物理尺寸也**不**
//!   因 DPI 变化而改（不碰 winit 的 `InnerSizeWriter`，见该事件的文档）；
//!   `LogicalSize` 只在建窗时用。
//!
//! ## 重绘策略（M5b：从「连续重绘」改成「事件驱动重绘」）
//!
//! 事件循环默认用 [`ControlFlow::Wait`] —— **纯阻塞**：没有事件就睡死。**不**用 `WaitUntil` 兜底，
//! 超时唤醒就是隐藏的空转，省电模式会名存实亡。
//!
//! ⚠️ **唯一的例外是 App 自己显式声明的 deadline**（M5c，见下面「唤醒面」一节）：那时用
//! `WaitUntil(那个时刻)`。这**不是**被否掉的那种「兜底」—— 兜底是**本层凭空造**一个超时
//! （没有谁要求过 ⇒ 纯空转）；deadline 是 **App 自己要求在那个时刻醒来**。**没人声明就一个
//! 纳秒的超时都不设**（[`ControlFlow::Wait`]，睡死）。这两句话看着像自相矛盾，所以专门写进
//! 文档：**先读「唤醒面」那节，再回来看这里**。
//!
//! 置位规则一共六条（表里就是 6 行；M5c fix 轮把「五条」改对）：
//!
//! | 触发 | 是否请求重绘 |
//! |---|---|
//! | 派发了一条 [`InputEvent`]（hover/press/wheel/key/text/focus 的**输入部分**） | 只有 [`App::wants_redraw`] 为真才请求 |
//! | **系统事件**（`Resized` / `Focused` / `Occluded(false)` 窗口重新暴露 / 建窗后的引导帧） | **一律**请求（不经过 `wants_redraw`） |
//! | `RedrawRequested` 到达 | 才调 [`App::redraw`]（一帧画一次；不请求就一帧都不画） |
//! | [`RedrawPolicy::Continuous`] | 每画完一帧再请求下一帧 |
//! | **唤醒**：[`Waker::wake`] | 与输入**同一把尺**：问 [`App::wants_redraw`]，答真才请求 |
//! | **唤醒**：deadline 到点（[`Waker::wake_after`] / [`App::next_deadline`]） | **一律**请求（不经过 `wants_redraw`，理由见「唤醒面」） |
//!
//! 默认是 [`RedrawPolicy::OnDemand`]（省电）：空闲时**零重绘**，CPU 不再空转。
//! **运行时开关**：环境变量 [`REDRAW_ENV`]（`DEER_WINDOW_REDRAW`）取值 `continuous` ⇒
//! **无条件**强制 [`RedrawPolicy::Continuous`]（把省电关掉）；`on-demand` / `demand` / 未设 /
//! 无法识别 ⇒ 用 [`App::redraw_policy`] 自己声明的策略。取值**先 `trim()` 再比、大小写不敏感**
//! （`cmd` 的 `set X=1 && …` 会把空格算进值里 ⇒ 严格比较会静默失效；实测与理由见
//! `crates/deer-gui/src/env_gate.rs` 的模块文档）。
//!
//! `run()` 启动时**自证**解析结果（验收要 grep 这一行，别只看退出码）：
//!
//! ```text
//! [deer-window] 重绘策略：请求=continuous 实际=Continuous（App 声明=OnDemand）
//! ```
//!
//! 结束时打一行账本，用来**数**「重绘次数 / 跳过帧数」（不用 fps 口径）：
//!
//! ```text
//! [deer-window] 重绘账本：requests=<请求重绘次数> skipped=<输入没改状态被跳过的帧数> frames=<成功画过的帧数>
//! ```
//!
//! ## 唤醒面（M5c：App 可以**自己**唤醒事件循环）
//!
//! > M5b 的诚实边界曾是：「`OnDemand` 下 App **没有**任何『主动唤醒事件循环』的手段（本层不
//! > 提供定时器 / 用户事件）—— 需要按时间自己推进的动画只能声明 [`RedrawPolicy::Continuous`]；
//! > 给 App 一个 `EventLoopProxy`（用户事件）是后续里程碑的事，本层不假装有。」
//! > **这一段现在过时了** —— 这一节就是它的替代品（M5c 交付）。
//!
//! [`App::wake_handle`] 在建窗后**调一次**，把可克隆的 [`Waker`] 交给 App（**默认实现什么都不
//! 做** ⇒ M5c 之前写的 `App` 实现一行都不用改）。三个手段：
//!
//! | 手段 | 语义 | 到的时候画不画 |
//! |---|---|---|
//! | [`Waker::wake`] | 「**看一眼**」（提示性：也许别的地方改了状态） | 与输入同一把尺：问 [`App::wants_redraw`]，**答真才画** |
//! | [`Waker::wake_after(d)`] | 「**d 之后叫醒我**」（预约：定时动画 / 脚本重放） | 到点**一律**画一帧 |
//! | [`App::next_deadline`] | 同上，但是**拉**式（App 声明「我希望被唤醒的最近时刻」） | 到点**一律**画一帧 |
//!
//! **为什么 deadline 到点一律画一帧、而 `wake()` 要先问 `wants_redraw`**：本层**没有**
//! 「唤醒回调」—— App 能对唤醒做出反应的**唯一**地方就是 [`App::redraw`]。若 deadline 到点还
//! 要先问 `wants_redraw` 为真，App 就得在**预约的那一刻**预先把自己标脏（而且 `wants_redraw`
//! 是 `&self`、根本改不了自己的状态）⇒ 定时动画会被逼成「预约时先置脏」这种绕圈子的写法。
//! 所以一刀切开：**`wake_after` / `next_deadline` 是 App 自己下的单**（「那个时刻请叫我」）
//! ⇒ 到了就画；**`wake()` 是提示**（「看看有没有变化」）⇒ 没变化就不画。
//!
//! **这「不是」先前被否掉的 `WaitUntil` 兜底**（最容易被读成自相矛盾的一条，单独说一遍）：
//! 被否掉的是**本层凭空造超时** —— 没人要求、到点也没事干，纯粹空转。现在本层**只在 App
//! 显式声明了 deadline 时**才用 `WaitUntil`，而**声明这个动作是 App 主动做的**：它不声明，
//! 本层就 [`ControlFlow::Wait`] 睡死。**开机不会自己醒来，只有 App 说「那个时刻叫我」才醒。**
//! 「唤醒账本」里的 `iters` 是**观测值，不是判据**（M5c 复审 I1 的结论，别再把它当全局证据）：
//! 空闲时它应当是个位数（本机实测 5–7，含启动期的系统事件），但**本层不设全局阈值** ——
//! `RedrawPolicy::Continuous` 档下它与帧数同阶（实测 229365）是**合法**的。真正读它下断言的是
//! **空闲档自己**：`examples/wake_probe.rs` 的 `DEER_WAKE_TICKS=0` 档断言 `iters` 上界
//! （账本经 [`App::on_wake_stats`] 交给 App）；另有本文件末尾的 `plan_to_control_flow` 单测
//! 钉死「`Wait` ⇒ `ControlFlow::Wait`（不是 `Poll`）」。**`tests/wake_policy.rs` 里没有读它的
//! 断言**（那条同义反复的 `assert_eq!(iters, IDLE_ITERS)` 已按复审删掉）。
//!
//! 两条 **⚠️ 使用须知**（都是「App 自己的要求」的直接后果，不是本层的 bug）：
//!
//! 1. **`next_deadline()` 必须给出固定的时刻、并且自己往前走**：别每次都返回
//!    `Instant::now() + 50ms` —— 那样它**永远不到点**，事件循环每 50ms 醒一次却一帧都不画
//!    （空转，正是本节开头反对的那种）。要「每 50ms 来一次」就用 [`Waker::wake_after`]：
//!    在 [`App::redraw`] 里排下一次（推式）。
//! 2. **已过期**的 deadline 视为「立刻到点」⇒ 画一帧。App 若不把它清掉/往前推，就等于自己
//!    要求连续重绘（那就是 [`RedrawPolicy::Continuous`] 的语义 —— 本层照做，账本上会看见
//!    `fired` 跟着 `iters` 一起涨）。
//!
//! 结束时打一行的**唤醒账本**（`iters` 的定性见上面「唤醒面」：**观测值**，本层不设全局阈值；
//! 读它下断言的是空闲档的 `examples/wake_probe.rs`（经 [`App::on_wake_stats`] 拿账本））：
//!
//! ```text
//! [deer-window] 唤醒账本：wake=<wake()投递到达> wake_after=<wake_after()投递到达> fired=<deadline到点> requested=<唤醒面请求的重绘次数> skipped=<wake()答假而没请求> iters=<事件循环迭代次数>
//! ```
//!
//! ## 多窗口（T4.4-R1：路由 + 排队建窗 + 多配置启动）
//!
//! 八项决策里属于**窗口层**的五条在这里落地（设计登记见 `ROADMAP.md`「设计登记：多窗口
//! （T4.4）」，渲染归属（决策 3）/树模型整合（决策 6 的上层）在 R2/R3）：
//!
//! | 决策 | 落点 |
//! |---|---|
//! | 1 事件签名 | 新增 [`App::window_input`]，**默认实现转发既有 `App::input`** ⇒ 单窗口用户零改动；多窗口用户覆盖它，拿 [`WindowId`] 区分窗。（实现签名比登记文本多一个 `info: &WindowInfo` 参数 —— 默认转发 `App::input` 需要该窗的信息，维护者裁决接受；见 `ROADMAP.md` 设计登记的收口注记） |
//! | 2 建窗模型 | 主窗仍由 [`run()`] 首建；之后经 [`WindowSpawner`]（`App::window_spawner` 交付，与 [`Waker`] **同一根通道**）`spawn_window(config)` **排队**，在事件循环的安全点（持 `ActiveEventLoop` 的 `user_event` 臂）真正建窗 |
//! | 3 多配置启动 | [`run_multi`]：每一项各建一扇窗（第一项 = 主窗），与 [`run()`] **同一条代码路径** |
//! | 4 关闭语义 | `CloseRequested` **只关该窗**（判定收在 [`WindowTable::close`]，可单测）；**全部窗口关闭** ⇒ 事件循环退出 |
//! | 5 焦点模型 | winit 本就按窗发 `Focused` ⇒ 按 [`WindowId`] 路由，App 收到的 `FocusChanged` 只关于那一扇窗 |
//!
//! **T4.4-R3 补齐的生命周期路由**（与 [`App::window_input`] 同一条「默认转发 ⇒ 单窗口
//! 零改动；覆盖 ⇒ 旧方法不再被调」的纪律，全部**带 id**）：[`App::window_init`]（每窗
//! 一次，渲染器在这里创建）、[`App::window_resized`]、[`App::window_redraw`]（该窗的
//! `RedrawRequested` ⇒ 多窗口下一次只画这一扇）、[`App::window_close_requested`]、
//! [`App::window_destroyed`]（该窗已从活窗表移除，App 在这里释放**这一扇**的渲染资源）。
//! **两层同源编号（接缝裁决）**：[`WindowId::raw()`] 直传渲染层窗口表
//! （deer-vk `WindowedRenderer::new_with_primary_id` / `add_window`），映射是恒等式 ——
//! 禁止靠「0/1 恰好错位对上」的隐式约定。
//!
//! ```no_run
//! use deer_window::{App, Flow, InputEvent, WindowConfig, WindowInfo, WindowId, WindowSpawner, run};
//!
//! struct TwoWindows {
//!     spawner: Option<WindowSpawner>,
//!     spawned: bool,
//! }
//!
//! impl App for TwoWindows {
//!     fn init(&mut self, _info: &WindowInfo) -> Result<(), String> { Ok(()) }
//!     /// 主窗建好后拿到建窗句柄（与 `wake_handle` 同批），排一扇新窗。
//!     fn window_spawner(&mut self, spawner: WindowSpawner) {
//!         self.spawner = Some(spawner);
//!     }
//!     fn redraw(&mut self) -> Result<Flow, String> {
//!         if !self.spawned {
//!             self.spawned = true;
//!             if let Some(sp) = &self.spawner {
//!                 sp.spawn_window(WindowConfig::new("第二扇", 320, 200)); // 只排队，安全点才真建
//!             }
//!         }
//!         Ok(Flow::Continue)
//!     }
//!     /// 多窗口 App 覆盖这里（覆盖后 `input` 不再被调）：`id` 就是决策 6 里
//!     /// 「每窗一棵树」的 `HashMap` 键。
//!     fn window_input(
//!         &mut self,
//!         id: WindowId,
//!         _info: &WindowInfo,
//!         _ev: &InputEvent,
//!     ) -> Result<Flow, String> {
//!         let _ = id; // 按窗分发给你自己的每窗状态
//!         Ok(Flow::Continue)
//!     }
//! }
//! # fn main() -> Result<(), String> {
//! run(WindowConfig::new("主窗", 800, 600), TwoWindows { spawner: None, spawned: false })
//! # }
//! ```
//!
//! **明确不做**（决策 8，登记在案）：跨窗口拖放 · owned/父子窗口 · 窗口间消息传递 ·
//! 每窗独立 GPU 实例 · 多线程渲染。渲染侧按窗的资源表（决策 3）已在 **R2（deer-vk）**
//! 落地；每窗一棵树的上层整合、双窗 demo 与四件套已在 **R3（deer-gui）** 落地
//! （见 `deer-gui` 的 `docs/features/multi-window.md` 与 `dual_window` 示例）。
//!
//! **仍未做（别当成已实现）**：自定义用户事件类型（对外**只有** [`Waker`] 与
//! [`WindowSpawner`] 两个面 —— `Wake` 是私有类型，App 拿不到 `EventLoopProxy::send_event`）、
//! 跨进程唤醒、**按窗定向**的多窗口唤醒（目前的唤醒是 App 级的，所有活窗一起重画）。

mod display;
mod host;

// ——— `pub use` 枢纽：公开路径与拆分前**逐字相同**（`lib.rs` 不再定义任何条目）———

pub use display::{
    Clipboard, CLIPBOARD_UNSUPPORTED_MSG, InputEvent, Key, Mods, PointerButton,
    UNSUPPORTED_PLATFORM_MSG, WindowConfig, WindowInfo, map_key, map_mouse_button, map_mods,
    map_wheel, printable_text, raw_handle_from_rwh06, raw_handle_from_win32,
};
pub use host::{
    App, Flow, FrameCounter, REDRAW_ENV, RedrawPolicy, WakePlan, WakeStats, Waker, WindowId,
    WindowSpawner, earliest, parse_redraw_policy, plan_wake, resolve_redraw_policy, run, run_multi,
};

// ——— crate 内部胶水（**不从根导出任何新名字**；全部只在 `cfg(test)` 下存在）———
//
// 下面这个单测模块与拆分前**逐字相同**，靠 `use super::*` 取名字。它用到三样东西：
// `WakePlan`（上面已 `pub use`）、`plan_to_control_flow`（host 的私有纯函数 ⇒ 这里转一手）、
// `Instant` / `Duration` / `ControlFlow`（标准库 / winit 类型 ⇒ 只给测试用）。
#[cfg(test)]
pub(crate) use host::plan_to_control_flow;
#[cfg(test)]
use std::time::{Duration, Instant};
#[cfg(test)]
use winit::event_loop::ControlFlow;

#[cfg(test)]
mod tests {
    use super::*;

    /// **省电承诺的落点**：`WakePlan::Wait` 必须翻成 `ControlFlow::Wait`（**睡死**）——
    /// **不是** `Poll`。
    ///
    /// M5c 复审 I1 的变异就是这一格：只把这一处改成 `Poll` ⇒ `iters` 5 → 11 695 034、
    /// CPU 烧满一核，而当时**全仓库没有任何断言抓得住**（`plan_wake` 的真值表只管到 `WakePlan`；
    /// `tests/wake_policy.rs` 的 harness 是复刻，也管不到这次翻译）⇒ exit=0、每条判据仍 ✅。
    /// 这条单测把「翻译」本身钉住（真窗口那一侧的运行期上界断言在 `examples/wake_probe.rs` 的
    /// 空闲档里；两者一起才覆盖「Wait ⇒ Poll」的两种写法：改翻译、改调用点）。
    #[test]
    fn wake_policy_control_flow_mapping() {
        let at = Instant::now() + Duration::from_millis(5);
        assert_eq!(
            plan_to_control_flow(WakePlan::Wait),
            ControlFlow::Wait,
            "空闲（没有 deadline）⇒ 必须睡死；Poll 就是 M5c 复审实测的那种空转"
        );
        assert_eq!(
            plan_to_control_flow(WakePlan::WaitUntil(at)),
            ControlFlow::WaitUntil(at),
            "睡到那个时刻（不是轮询、也不是立刻醒）"
        );
        assert_eq!(
            plan_to_control_flow(WakePlan::Due),
            ControlFlow::Wait,
            "到点那一帧已经 request_redraw() 过了 ⇒ 本轮先睡，别自转"
        );
    }
}
