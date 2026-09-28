//! # deer-window —— 窗口层（M2b）
//!
//! 负责「原生窗口 + 事件循环」，并把窗口的原生句柄以 **HAL 的不透明形式**
//! （[`deer_gpu::RawWindowHandle`]）交给上层。渲染后端（`deer-vk`）**不依赖 winit**：
//! 它只认那个不透明句柄 —— 这样「加一个平台 = 换窗口实现」，而不是「换整个渲染栈」。
//!
//! **本 crate 是本 workspace 唯一引入第三方依赖的地方**（`winit`）。理由与替代方案
//! （自写 Win32）的对比登记在 `ROADMAP.md` 的 Q-1 里。
//!
//! ## 用法
//!
//! ```no_run
//! use deer_window::{
//!     App, Flow, InputEvent, Key, RedrawPolicy, WindowConfig, WindowInfo, run,
//! };
//!
//! struct MyApp;
//!
//! impl App for MyApp {
//!     fn init(&mut self, info: &WindowInfo) -> Result<(), String> {
//!         println!("窗口 {}x{}，HWND=0x{:X}", info.extent.width, info.extent.height, info.raw.handle);
//!         Ok(()) // 这里创建 Vulkan 设备 / 交换链（deer-vk 只吃 info.raw）
//!     }
//!     fn redraw(&mut self) -> Result<Flow, String> {
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
//!     fn redraw_policy(&self) -> RedrawPolicy {
//!         RedrawPolicy::OnDemand
//!     }
//! }
//!
//! # fn main() -> Result<(), String> {
//! run(WindowConfig::new("demo", 800, 600), MyApp)
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
//!   `DeviceEvent`（原始设备事件）、物理键码（`physical_key`/scancode）、按键重复的区分
//!   （winit 的 `repeat` **没有**建模）、IME 预编辑（`Preedit` 只用来抑制重复文本，不派发）、
//!   键盘布局无关的快捷键。
//! - **DPI**：只透传 `Resized` 给的物理像素，不做任何缩放换算；`LogicalSize` 只在建窗时用。
//!
//! ## 重绘策略（M5b：从「连续重绘」改成「事件驱动重绘」）
//!
//! 事件循环用 [`ControlFlow::Wait`] —— **纯阻塞**：没有事件就睡死。**不**用 `WaitUntil` 兜底，
//! 超时唤醒就是隐藏的空转，省电模式会名存实亡。置位规则一共四条：
//!
//! | 触发 | 是否请求重绘 |
//! |---|---|
//! | 派发了一条 [`InputEvent`]（hover/press/wheel/key/text/focus 的**输入部分**） | 只有 [`App::wants_redraw`] 为真才请求 |
//! | **系统事件**（`Resized` / `Focused` / `Occluded(false)` 窗口重新暴露 / 建窗后的引导帧） | **一律**请求（不经过 `wants_redraw`） |
//! | `RedrawRequested` 到达 | 才调 [`App::redraw`]（一帧画一次；不请求就一帧都不画） |
//! | [`RedrawPolicy::Continuous`] | 每画完一帧再请求下一帧 |
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
//! **诚实的边界**：`OnDemand` 下 App **没有**任何「主动唤醒事件循环」的手段（本层不提供
//! 定时器 / 用户事件）—— 需要按时间自己推进的动画只能声明 [`RedrawPolicy::Continuous`]。
//! 给 App 一个 `EventLoopProxy`（用户事件）是后续里程碑的事，本层不假装有。

use std::sync::Arc;

use deer_gpu::Extent;
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, Ime, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{Key as KeyboardKey, ModifiersState, NamedKey};
use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle as RwhRawWindowHandle};
use winit::window::{Window, WindowId};

/// 非 Windows 平台（或 winit 给回了非 Win32 句柄）时 `run()` 会返回这句话。
///
/// **不静默填 0**：宁可明确报「未实现」，也不让 Vulkan 拿着 0 号 HWND 去建 surface。
pub const UNSUPPORTED_PLATFORM_MSG: &str =
    "deer-window 目前只实现了 Windows 窗口（winit 后端已就绪，但 RawWindowHandle 的填法未实现）";

/// 窗口配置：标题 + **逻辑**尺寸（建窗时用 [`LogicalSize`]，由 winit/系统换算成物理像素）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowConfig {
    pub title: String,
    pub width: u32,
    pub height: u32,
}

impl WindowConfig {
    /// 标题、宽、高**逐字**存入，不做任何钳制（0 也照存，由调用方负责合理值）。
    pub fn new(title: impl Into<String>, width: u32, height: u32) -> WindowConfig {
        WindowConfig { title: title.into(), width, height }
    }

    /// 标题栏实际显示的文本：加 `deer-gui` 前缀便于识别（多窗口/截图时一眼看出是谁的）。
    ///
    /// 标题为空时只显示 `deer-gui`（不留下一个孤零零的分隔符）。
    pub fn display_title(&self) -> String {
        if self.title.is_empty() {
            "deer-gui".to_string()
        } else {
            format!("deer-gui — {}", self.title)
        }
    }
}

impl Default for WindowConfig {
    /// 默认：无标题（显示为 `deer-gui`）、800×600。
    fn default() -> WindowConfig {
        WindowConfig { title: String::new(), width: 800, height: 600 }
    }
}

/// 交给渲染层的窗口信息（原生句柄 + **物理**尺寸）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowInfo {
    /// `platform = Windows`，`handle = HWND`，`display = HINSTANCE`。
    pub raw: deer_gpu::RawWindowHandle,
    /// 当前**物理**像素尺寸（`Resized` 事件的值）。
    pub extent: Extent,
}

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

/// 指针按键：winit 的按键在**本层模型**里的表示。
///
/// 只有这三个变体（M5-1 接口冻结）：winit 的 `Back`/`Forward`/`Other` **没有**对应变体，
/// [`map_mouse_button`] 对它们返回 `None`（整条事件丢弃），**不**降级成左键 ——
/// 那会凭空产生点击。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PointerButton {
    Left,
    Right,
    Middle,
}

/// 修饰键状态：**随 [`InputEvent`] 的键盘事件透传**，本层不解释它的语义。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Mods {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
    /// Super（Windows 键 / Command 键 / Meta）。
    pub sup: bool,
}

/// **物理键**：不含文本语义 —— 文本一律走 [`InputEvent::TextInput`]。
///
/// 这条分工是刻意的（以后的文本框要靠它区分「插入字符」与「快捷键/焦点移动」）：
/// 按 `a` 会**同时**产生 `KeyDown { key: Char('a') }` 与 `TextInput { text: "a" }`；
/// 而 `Enter` / `Tab` / `Backspace` / `Escape` **只有** `KeyDown`
/// （winit 给它们的 `text` 是控制字符，被 [`printable_text`] 挡掉）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Key {
    Tab,
    Escape,
    Enter,
    Backspace,
    Left,
    Right,
    Up,
    Down,
    /// 可打印字符键：**逻辑**字符（受当前布局与 Shift 影响，`Shift+a` ⇒ `Char('A')`）。
    Char(char),
    /// 其它一切：修饰键、功能键、编辑键、死键、多字符组合、无法识别的键。
    Other,
}

/// 本层自己的输入事件模型：**唯一**进入 [`App`] 的输入表示。
///
/// 坐标是**物理像素**、窗口左上角为原点（与 `Resized`/[`WindowInfo::extent`] 同一套口径，
/// 本层不做 DPI 换算；压栈换算留给上层）。
#[derive(Debug, Clone, PartialEq)]
pub enum InputEvent {
    PointerMoved {
        x: f32,
        y: f32,
    },
    PointerDown {
        button: PointerButton,
        x: f32,
        y: f32,
    },
    PointerUp {
        button: PointerButton,
        x: f32,
        y: f32,
    },
    Wheel {
        dx: f32,
        dy: f32,
    },
    KeyDown {
        key: Key,
        mods: Mods,
    },
    KeyUp {
        key: Key,
        mods: Mods,
    },
    /// 文本输入：IME 的 `Commit` 结果，或**可打印**按键产生的文本（见 [`printable_text`]）。
    TextInput {
        text: String,
    },
    FocusChanged {
        focused: bool,
    },
}

/// winit 的**逻辑键** → [`Key`]（纯函数：只吃字段、不吃 winit 事件 ⇒ 单测不用构造事件对象）。
///
/// - `Named(Tab / Escape / Enter / Backspace / Arrow{Left,Right,Up,Down})` ⇒ 同名变体；
/// - `Named(Space)` ⇒ [`Key::Char`]`(' ')`：空格是**可打印字符**，不是命令键；
/// - `Character(s)`：恰好一个字符 ⇒ [`Key::Char`]，多字符（死键组合等）⇒ [`Key::Other`]；
/// - 其它命名键（修饰键、功能键、编辑键、小键盘…）、`Dead`、`Unidentified` ⇒ [`Key::Other`]。
///
/// `mods` **不参与**判定：Shift/Alt 的影响已由 winit 写进 `logical_key`
/// （`Shift+a` ⇒ `Character("A")`），Ctrl 按 winit 的口径不改变 `logical_key`。
/// 参数保留是因为计划把签名冻结成 `map_key(key, mods)`；「修饰键不改变映射结果」
/// 这条契约由单测固定（`tests/input_map.rs`），修饰键本身由 [`InputEvent`] 的 `mods` 透传。
pub fn map_key(key: &KeyboardKey, mods: Mods) -> Key {
    let _ = mods;
    match key {
        KeyboardKey::Named(NamedKey::Tab) => Key::Tab,
        KeyboardKey::Named(NamedKey::Escape) => Key::Escape,
        KeyboardKey::Named(NamedKey::Enter) => Key::Enter,
        KeyboardKey::Named(NamedKey::Backspace) => Key::Backspace,
        KeyboardKey::Named(NamedKey::ArrowLeft) => Key::Left,
        KeyboardKey::Named(NamedKey::ArrowRight) => Key::Right,
        KeyboardKey::Named(NamedKey::ArrowUp) => Key::Up,
        KeyboardKey::Named(NamedKey::ArrowDown) => Key::Down,
        KeyboardKey::Named(NamedKey::Space) => Key::Char(' '),
        KeyboardKey::Character(s) => match single_char(s) {
            Some(c) => Key::Char(c),
            None => Key::Other,
        },
        // 其它命名键 / Dead / Unidentified：明确落到 Other，不猜。
        _ => Key::Other,
    }
}

/// `Character("…")` 里恰好一个字符时取出来；空串/多字符 ⇒ `None`（⇒ [`Key::Other`]）。
///
/// 用 `chars()` 而不是 `len()`：`"中"` 是 3 字节但 1 个字符，`"😀"` 是 4 字节 1 个字符。
fn single_char(s: &str) -> Option<char> {
    let mut chars = s.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) => Some(c),
        _ => None,
    }
}

/// winit 的鼠标按键 → [`PointerButton`]（纯函数）。
///
/// `Back` / `Forward` / `Other(_)` ⇒ `None`：[`PointerButton`] 没有对应变体（接口冻结，
/// 不新增 `Other`），**丢弃**好过把它们当左键（那会凭空产生点击）。
pub fn map_mouse_button(button: MouseButton) -> Option<PointerButton> {
    match button {
        MouseButton::Left => Some(PointerButton::Left),
        MouseButton::Right => Some(PointerButton::Right),
        MouseButton::Middle => Some(PointerButton::Middle),
        MouseButton::Back | MouseButton::Forward | MouseButton::Other(_) => None,
    }
}

/// winit 的修饰键位标志 → [`Mods`]（纯函数）。
///
/// winit 0.30 **没有**「随时查当前修饰键」的接口，所以本层自己维护：初始全 `false`，
/// 之后由 `ModifiersChanged` 更新（`mods` 只随键盘事件透传，不单独派发）。
pub fn map_mods(state: ModifiersState) -> Mods {
    Mods {
        shift: state.shift_key(),
        ctrl: state.control_key(),
        alt: state.alt_key(),
        sup: state.super_key(),
    }
}

/// 滚轮增量 → `(dx, dy)`（纯函数）。
///
/// - `LineDelta(x, y)`：**行**为单位，原样透传（换算成像素是上层的事）；
/// - `PixelDelta(p)`：像素为单位，`f64 → f32`（本层坐标口径就是 f32 物理像素）。
pub fn map_wheel(delta: &MouseScrollDelta) -> (f32, f32) {
    match delta {
        MouseScrollDelta::LineDelta(x, y) => (*x, *y),
        MouseScrollDelta::PixelDelta(p) => (p.x as f32, p.y as f32),
    }
}

/// 把 winit 给的按键文本过滤成「**可打印文本**」（纯函数）。
///
/// - `None` / 空串 ⇒ `None`；
/// - 含控制字符 ⇒ `None`：winit 会给 `Enter` 填 `"\r"`、`Tab` 填 `"\t"`、
///   `Backspace` 填 `"\x08"`、`Escape` 填 `"\x1b"`、`Ctrl+A` 填 `"\u{1}"` —— 这些是
///   **物理键**，由 [`Key`] 表达，绝不能当文本插进文本框；
/// - 其余（含空格、CJK、emoji、死键组合出的多字符串）⇒ `Some(原样)`。
pub fn printable_text(text: Option<&str>) -> Option<String> {
    let text = text?;
    if text.is_empty() || text.chars().any(char::is_control) {
        return None;
    }
    Some(text.to_string())
}

/// 用户实现这个 trait；[`run()`] 负责事件循环与窗口生命周期。
///
/// 所有回调都在**主线程**（事件循环线程）上被调用。
pub trait App {
    /// 窗口建好后**调一次**：在这里创建渲染器（Vulkan 设备/交换链）。
    ///
    /// 返回 `Err` 会让 `run()` 打印原因、退出事件循环并返回 `Err`。
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
    /// [`InputEvent::KeyDown`]/[`InputEvent::KeyUp`]（见 [`Key`] 的说明）。
    /// `info` 是**当前**窗口信息（原生句柄 + 物理尺寸，`Resized` 之后同步更新），
    /// 口径与 [`App::resized`] 一致 —— 输入事件里的坐标就跟它同一套物理像素。
    fn input(&mut self, info: &WindowInfo, ev: &InputEvent) -> Result<Flow, String> {
        // 默认实现什么都不做。参数名保持与冻死 API 一致（不改名成 `_info`/`_ev`），
        // 用 `let _` 消化掉。
        let _ = (info, ev);
        Ok(Flow::Continue)
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

/// 把 `(HWND, HINSTANCE)` 打包成 HAL 的不透明句柄。
///
/// 纯函数、不做平台判定 —— 平台判定在 [`raw_handle_from_rwh06`] / [`run()`] 里。
/// 两个值都按 `usize` 原样存入：`handle = HWND`、`display = HINSTANCE`。
pub fn raw_handle_from_win32(hwnd: usize, hinstance: usize) -> deer_gpu::RawWindowHandle {
    deer_gpu::RawWindowHandle {
        platform: deer_gpu::Platform::Windows,
        handle: hwnd,
        display: hinstance,
    }
}

/// 把 winit/rwh-0.6 的原始句柄翻译成 HAL 的不透明句柄。
///
/// - `Win32` ⇒ `platform = Windows`、`handle = HWND`、`display = GWLP_HINSTANCE`；
/// - 其它平台（Xlib/Wayland/AppKit/…）⇒ `Err(`[`UNSUPPORTED_PLATFORM_MSG`]`)`，
///   **不静默填 0**；
/// - Win32 句柄缺 `hinstance` 时也返回 `Err`（`display` 没法编）。
///
/// 它是纯函数（只读传入的句柄），所以**在 Windows 上也能单测非 Windows 分支**：
/// 直接喂一个 `RawWindowHandle::Xlib(...)` 进去即可。
pub fn raw_handle_from_rwh06(
    raw: RwhRawWindowHandle,
) -> Result<deer_gpu::RawWindowHandle, String> {
    match raw {
        RwhRawWindowHandle::Win32(h) => {
            let hwnd = h.hwnd.get() as usize;
            let hinstance = h
                .hinstance
                .map(|v| v.get() as usize)
                .ok_or_else(|| "Win32 窗口句柄缺少 hinstance（GWLP_HINSTANCE），display 无法填".to_string())?;
            Ok(raw_handle_from_win32(hwnd, hinstance))
        }
        // 非 Windows（Xlib/Wayland/AppKit/…）：明确报未实现。枚举是 non_exhaustive，
        // 所以必须用通配臂；错误文案与 UNSUPPORTED_PLATFORM_MSG 逐字一致。
        _ => Err(UNSUPPORTED_PLATFORM_MSG.to_string()),
    }
}

/// 从窗口取出 HAL 用的 [`WindowInfo`]（原生句柄 + 物理尺寸）。
fn window_info(window: &Window) -> Result<WindowInfo, String> {
    let handle = window
        .window_handle()
        .map_err(|e| format!("取原生窗口句柄失败：{e}"))?;
    let raw = raw_handle_from_rwh06(handle.as_raw())?;
    let size = window.inner_size();
    Ok(WindowInfo { raw, extent: Extent { width: size.width, height: size.height } })
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
///   `[deer-window] 事件循环结束：frames=<n> extent=<w>x<h> result=ok|error` 与
///   `[deer-window] 重绘账本：requests=<n> skipped=<n> frames=<n>`。
pub fn run<A: App + 'static>(config: WindowConfig, app: A) -> Result<(), String> {
    // 重绘策略在这里定，**在建 EventLoop 之前**：策略 = App 自己声明的 + 环境变量的覆盖，
    // 与窗口无关 ⇒ 就算建窗失败，自证那行也已经打出来了。
    let requested = std::env::var(REDRAW_ENV).ok();
    let app_policy = app.redraw_policy();
    let policy = resolve_redraw_policy(app_policy, requested.as_deref());
    print_redraw_policy(requested.as_deref(), app_policy, policy);

    let event_loop =
        EventLoop::new().map_err(|e| format!("创建 winit EventLoop 失败（run() 必须在主线程调用）：{e}"))?;
    let mut handler = RunHandler {
        config,
        app,
        policy,
        window: None,
        info: None,
        extent: Extent { width: 0, height: 0 },
        counter: FrameCounter::new(),
        mods: Mods::default(),
        cursor: (0.0, 0.0),
        ime_composing: false,
        error: None,
        exiting: false,
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

/// [`run()`] 的 `ApplicationHandler` 实现：事件循环 → `App` 回调的接线。
struct RunHandler<A: App> {
    config: WindowConfig,
    app: A,
    /// **实际生效**的重绘策略（[`App::redraw_policy`] + [`REDRAW_ENV`] 的覆盖，在 `run()` 里算好）。
    policy: RedrawPolicy,
    /// `resumed` 里建好后由事件循环持有；窗口活到 `run()` 结束。
    window: Option<Arc<Window>>,
    /// 建窗时算好的 [`WindowInfo`]（`Resized` 时同步 `extent`）：原样转发给 `App::input`。
    info: Option<WindowInfo>,
    /// 最近一次已知的物理尺寸（初始取自 `inner_size()`，之后由 `Resized` 更新）。
    extent: Extent,
    counter: FrameCounter,
    /// 当前修饰键状态：由 `ModifiersChanged` 维护（winit 0.30 没有「随时查」的接口）。
    mods: Mods,
    /// 最近一次 `CursorMoved` 的**物理**坐标：`MouseInput` 只给按键、不给坐标，
    /// 而 `PointerDown/Up` 需要坐标 ⇒ 用最近一次光标位置补上（还没收到移动事件时是 (0,0)）。
    cursor: (f32, f32),
    /// IME 正在预编辑（收到非空 `Preedit` 且还没 `Commit`/`Disabled`）。
    /// 此时 `KeyboardInput` 的文本**不**再转 `TextInput`：否则中文输入会重复上屏
    /// （一次来自 `Ime::Commit`，一次来自按键自带的 `text`）。
    ime_composing: bool,
    /// 第一个错误（后续错误不再覆盖它）。
    error: Option<String>,
    /// 已经请求 `event_loop.exit()`：同一批事件里后面的回调不再处理，
    /// 这样「帧数」就是 App 真正要求画的帧数，不会被 `exit()` 之后的残留事件多加。
    exiting: bool,
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

    /// 把一条输入事件交给 `App::input`，并按**置位规则**决定要不要请求重绘。
    ///
    /// 走到这里的事件都是**已映射的输入**（hover/press/focus/text/滚轮…）；反过来，**没有**
    /// 映射成 [`InputEvent`] 的 winit 事件（`ModifiersChanged`、`CursorEntered/Left`、被丢弃的侧键…）
    /// 根本走不到这里，也就不会请求重绘 —— 「每个 winit 事件都重绘」这条被刻意避开了。
    ///
    /// `gate` 决定置位规则：普通输入看 [`App::wants_redraw`]（**为假就不请求**，这是 M5b 的省电核心），
    /// 系统类输入（`Focused`）**一律**请求（见 [`Gate`]）。取消/出错的语义与旧版一致。
    ///
    /// `Err` 走 [`RunHandler::fail`]（打印 + 记住 + 退出）；`Flow::Exit` 与 `redraw` 同义。
    fn dispatch(&mut self, event_loop: &ActiveEventLoop, ev: &InputEvent, gate: Gate) {
        let Some(info) = self.info else {
            // 还没成功建窗（理论上到不了这里）：没有窗口就没有输入，直接丢弃。
            return;
        };
        match self.app.input(&info, ev) {
            Ok(Flow::Continue) => match gate {
                Gate::AppDecides => {
                    // **先问 App 这一条输入改了状态没有**（`input` 之后问，App 才有机会更新脏标记）。
                    let wants = self.app.wants_redraw();
                    if self.counter.on_input(wants) {
                        self.request_redraw();
                    }
                }
                Gate::Always => self.request_redraw(),
            },
            Ok(Flow::Exit) => {
                self.exiting = true;
                event_loop.exit();
            }
            Err(e) => self.fail(event_loop, format!("App::input 失败：{e}")),
        }
    }

    /// 请求重绘 + **记账**（`request_redraw()` 的**唯一**落点 ⇒ 「请求次数」只有一个真相来源）。
    ///
    /// 只有三种触发：输入置位、系统事件（含 `Resized`/`Focused`/窗口暴露/建窗引导帧）、
    /// `Continuous` 的续帧。窗口还没建时只记账（没有窗口可请求，也没别的地方会读这个数）。
    fn request_redraw(&mut self) {
        self.counter.note_request();
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    /// 收尾：打摘要、给出最终结果（回调错误优先于事件循环自身的错误）。
    fn finish(self, loop_error: Option<String>) -> Result<(), String> {
        let err = self.error.or(loop_error);
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

impl<A: App + 'static> ApplicationHandler for RunHandler<A> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.error.is_some() || self.window.is_some() {
            // 已经出过错就不再建窗；`resumed` 在部分平台会被多次调用，窗口只建一次。
            return;
        }

        let attrs = Window::default_attributes()
            .with_title(self.config.display_title())
            // 建窗用逻辑尺寸（系统按 DPI 换算）；之后的 Resized 一律物理像素直传。
            .with_inner_size(LogicalSize::new(
                f64::from(self.config.width),
                f64::from(self.config.height),
            ));
        let window = match event_loop.create_window(attrs) {
            Ok(w) => Arc::new(w),
            Err(e) => return self.fail(event_loop, format!("创建窗口失败：{e}")),
        };

        let size = window.inner_size();
        self.extent = Extent { width: size.width, height: size.height };
        let info = match window_info(&window) {
            Ok(info) => info,
            Err(e) => return self.fail(event_loop, e),
        };
        println!(
            "[deer-window] 窗口已建：title=\"{}\" extent={}x{} platform={:?} handle(HWND)=0x{:X} display(HINSTANCE)=0x{:X}",
            self.config.display_title(),
            info.extent.width,
            info.extent.height,
            info.raw.platform,
            info.raw.handle,
            info.raw.display
        );

        if let Err(e) = self.app.init(&info) {
            return self.fail(event_loop, format!("App::init 失败：{e}"));
        }

        // IME：winit 要求**显式允许**才会发 `Ime` 事件（`Ime::Commit` 是中文/日文输入的
        // 文本来源）。不开的话 CJK 输入在本层完全收不到 —— 那就与「输入通路已接通」相反。
        window.set_ime_allowed(true);

        // **纯阻塞**：没有事件就睡死（旧版是 `Poll`：一直空转问「有没有事」）。
        // 刻意**不**用 `WaitUntil` 兜底 —— 超时唤醒就是隐藏的空转，省电模式会名存实亡。
        event_loop.set_control_flow(ControlFlow::Wait);
        self.info = Some(info);
        self.window = Some(window);

        // **引导帧**：winit 对「建窗后一定发一条 `RedrawRequested`」**没有保证**
        // （见 winit 0.30 `Window::request_redraw` 的「no strong guarantees」），
        // 而 `OnDemand` 下 App 没有别的办法要到第一帧（它拿不到窗口句柄）⇒ 这里主动要一帧，
        // 否则窗口会一直留一块没画过的区域，`Continuous` 的续帧链也根本起不来。
        // 这是一次性的引导，不是空转：之后要么由输入/系统事件置位，要么由 `Continuous` 续帧。
        self.request_redraw();
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        if self.error.is_some() || self.exiting {
            return;
        }
        match event {
            WindowEvent::Resized(size) => {
                self.extent = Extent { width: size.width, height: size.height };
                // `WindowInfo` 跟着更新：`App::input` 拿到的尺寸必须与 `resized` 同口径，
                // 否则输入坐标会按旧尺寸解释（点偏）。
                if let Some(info) = &mut self.info {
                    info.extent = self.extent;
                }
                if let Err(e) = self.app.resized(size.width, size.height) {
                    self.fail(
                        event_loop,
                        format!("App::resized({}x{}) 失败：{e}", size.width, size.height),
                    )
                }
                // **系统事件一律置位**：尺寸变了必须重画（不然窗口留一片脏区），
                // 而且这不看 `App::wants_redraw`、也不看策略 —— 系统说「要重画」就是「要重画」。
                self.request_redraw();
            }
            WindowEvent::RedrawRequested => {
                let result = self.app.redraw();
                match self.counter.on_redraw(result) {
                    Ok(()) => {
                        if self.counter.exit_requested() {
                            self.exiting = true;
                            event_loop.exit();
                        } else if self.policy.wants_next_frame() {
                            // `Continuous`：每画完一帧再请求下一帧（这一条就是「连续重绘」的发动机）。
                            // `OnDemand` 什么都不做 ⇒ 没有输入/系统事件就**不再有下一帧**（省电）。
                            self.request_redraw();
                        }
                    }
                    Err(e) => self.fail(event_loop, format!("App::redraw 失败：{e}")),
                }
            }
            // 关闭请求：只有 App 说 Exit 才退（`close_requested` 的返回值语义）。
            WindowEvent::CloseRequested if self.app.close_requested() == Flow::Exit => {
                self.exiting = true;
                event_loop.exit();
            }

            // ——— 输入事件的翻译（winit 事件 → 本层 `InputEvent` → `App::input`）———
            // 普通输入一律走 `Gate::AppDecides`：**只有** `App::wants_redraw` 为真才请求重绘。
            WindowEvent::CursorMoved { position, .. } => {
                self.cursor = (position.x as f32, position.y as f32);
                let (x, y) = self.cursor;
                self.dispatch(event_loop, &InputEvent::PointerMoved { x, y }, Gate::AppDecides);
            }
            WindowEvent::MouseInput { state, button, .. } => {
                let Some(button) = map_mouse_button(button) else {
                    // 侧键/未知键：`PointerButton`（接口冻结）表示不了 ⇒ 整条丢弃。
                    // **不**派发、**不**请求重绘（也就不会产生「不明点击」）。
                    return;
                };
                let (x, y) = self.cursor;
                let ev = match state {
                    ElementState::Pressed => InputEvent::PointerDown { button, x, y },
                    ElementState::Released => InputEvent::PointerUp { button, x, y },
                };
                self.dispatch(event_loop, &ev, Gate::AppDecides);
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let (dx, dy) = map_wheel(&delta);
                self.dispatch(event_loop, &InputEvent::Wheel { dx, dy }, Gate::AppDecides);
            }
            WindowEvent::KeyboardInput { event, .. } => {
                // 合成的按键事件（失焦时系统补发的 KeyUp）**照发**：上层的「按住的键」
                // 靠它清掉，丢掉会留下按下的残留状态。
                let key = map_key(&event.logical_key, self.mods);
                let mods = self.mods;
                match event.state {
                    ElementState::Pressed => {
                        // **物理键先发**：`Char('a')`/`Enter`/`Tab`… 一律先来 KeyDown。
                        self.dispatch(event_loop, &InputEvent::KeyDown { key, mods }, Gate::AppDecides);
                        // **文本与物理键分开**：IME 预编辑中由 `Ime::Commit` 负责文本
                        // （免得中文重复上屏）；`Enter/Tab/Backspace/Esc` 的 text 是
                        // 控制字符，被 `printable_text` 挡掉 ⇒ 它们只有 KeyDown。
                        let text = if self.ime_composing {
                            None
                        } else {
                            printable_text(event.text.as_deref())
                        };
                        if let Some(text) = text {
                            self.dispatch(event_loop, &InputEvent::TextInput { text }, Gate::AppDecides);
                        }
                    }
                    ElementState::Released => {
                        self.dispatch(event_loop, &InputEvent::KeyUp { key, mods }, Gate::AppDecides);
                    }
                }
            }
            WindowEvent::Ime(ime) => match ime {
                Ime::Commit(text) => {
                    self.ime_composing = false;
                    // 空提交（预编辑被清掉）不算输入 ⇒ 不派发、不重绘。
                    if !text.is_empty() {
                        self.dispatch(event_loop, &InputEvent::TextInput { text }, Gate::AppDecides);
                    }
                }
                Ime::Preedit(text, _) => {
                    // 预编辑文本**不派发**（M5 不建模预编辑）：这里只用它抑制按键文本，
                    // 避免「预编辑中按键的 text」与「Commit」双重上屏。
                    self.ime_composing = !text.is_empty();
                }
                Ime::Enabled | Ime::Disabled => self.ime_composing = false,
            },
            WindowEvent::Focused(focused) => {
                // **系统事件一律置位**：焦点变化算系统事件（`Gate::Always`），不看 `wants_redraw` ——
                // 光标/焦点框这类视觉状态一变，界面就该重画，不该由 App 的脏标记决定。
                self.dispatch(event_loop, &InputEvent::FocusChanged { focused }, Gate::Always);
            }
            WindowEvent::Occluded(false) => {
                // 窗口**重新暴露**（从被遮挡状态回来）：必须重画（期间交换链可能已过期）⇒ 系统事件一律置位。
                // 诚实说明：winit 0.30 的文档明确写 Windows **不支持**这个事件（iOS/Android 才有），
                // 所以这一臂在 Windows 上是「接线接好了但系统不喂」—— 不假装它在 Windows 上跑过。
                self.request_redraw();
            }
            WindowEvent::ModifiersChanged(modifiers) => {
                // 只记账：修饰键在 `InputEvent` 里是键盘事件的**字段**，没有单独的事件 ⇒
                // 不派发、也不请求重绘（单独按 Shift 不会让界面有任何变化）。
                self.mods = map_mods(modifiers.state());
            }
            // 其余事件（触摸/手势/拖放/CursorEntered…）本期不转发：见模块文档的「仍未接线」。
            _ => {}
        }
    }
}
