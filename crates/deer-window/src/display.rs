//! # `deer-window` 的 L1 —— DisplayServer（winit / 窗口句柄 / 输入翻译 / DPI）
//!
//! 分层归属见 `docs/ARCHITECTURE.md` §2.3：**凡有「设备 / 句柄 / 平台 / 全局状态」即在此层**。
//! 本模块是 `deer-window` 的**平台面**（唯一直接碰 winit 的那一半）：
//!
//! - **窗口与原生句柄**：[`WindowConfig`] / [`WindowInfo`] / [`raw_handle_from_win32`] /
//!   [`raw_handle_from_rwh06`] / `window_info` —— 句柄以 HAL 的不透明形式
//!   （`deer_gpu::RawWindowHandle`）交出 ⇒ 渲染后端（`deer-vk`）**不依赖 winit**；
//! - **平台臂**：只有 Win32 实现了句柄填法，其余平台明确 `Err(`[`UNSUPPORTED_PLATFORM_MSG`]`)`，
//!   **不静默填 0**；
//! - **输入模型与 winit 映射**：[`InputEvent`] / [`Key`] / [`Mods`] / [`PointerButton`] +
//!   [`map_key`] / [`map_mouse_button`] / [`map_mods`] / [`map_wheel`] / [`printable_text`] ——
//!   winit 类型**不进回调签名**（物理键与文本分开，见 [`Key`] 的说明）；
//! - **DPI**（AF-3）：`scale_factor` **只透传**（`WindowInfo` 初值 +
//!   `InputEvent::ScaleFactorChanged` 事件）；坐标与尺寸仍是**物理像素**，
//!   不做任何缩放换算（红线：布局是像素级纯函数）；
//! - **剪贴板**（AF-2）：[`Clipboard`] 公共构造器式句柄，自写 Win32 `CF_UNICODETEXT`
//!   （不引第三方，Q3 裁断）；非 Windows 明确 `Err(`[`CLIPBOARD_UNSUPPORTED_MSG`]`)`。
//!
//! **不在这里的**：事件循环与 `App` 运行时（`App` / `Waker` / 帧调度）在 [`crate::host`]（L3）——
//! 它**驱动**本模块提供的窗口与事件。
//!
//! 依赖方向是**单向**的：host → display（display 不反向依赖 host）。所以本模块里被 host 用到的
//! 内部项要标 `pub(crate)`（目前只有 `window_info` 一处）。
//!
//! 拆分前这些内容与 L3 同住在 `lib.rs`（1571 行，见 `docs/ARCHITECTURE.md` §2.3 的「已知错位」）；
//! **公开路径一个都没变** —— `lib.rs` 用 `pub use` 把本模块的公开项重新导出。

use deer_gpu::Extent;
use winit::event::{MouseButton, MouseScrollDelta};
use winit::keyboard::{Key as KeyboardKey, ModifiersState, NamedKey};
use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle as RwhRawWindowHandle};
use winit::window::Window;

/// 非 Windows 平台（或 winit 给回了非 Win32 句柄）时 `run()` 会返回这句话。
///
/// **不静默填 0**：宁可明确报「未实现」，也不让 Vulkan 拿着 0 号 HWND 去建 surface。
pub const UNSUPPORTED_PLATFORM_MSG: &str =
    "deer-window 目前只实现了 Windows 窗口（winit 后端已就绪，但 RawWindowHandle 的填法未实现）";

/// 窗口配置：标题 + **逻辑**尺寸（建窗时用 [`LogicalSize`](winit::dpi::LogicalSize)，
/// 由 winit/系统换算成物理像素）。
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

/// 交给渲染层的窗口信息（原生句柄 + **物理**尺寸 + DPI 缩放系数）。
///
/// `Eq` 因 `scale_factor`（`f64`）而放弃 —— 比较 `WindowInfo` 用 `PartialEq` 照常可用
/// （既有三处测试就是 `assert_eq!`，不受影响）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WindowInfo {
    /// `platform = Windows`，`handle = HWND`，`display = HINSTANCE`。
    pub raw: deer_gpu::RawWindowHandle,
    /// 当前**物理**像素尺寸（只由 `Resized` 更新；DPI 变化**不改**它 —— 见
    /// `ScaleFactorChanged`：本层刻意保持现有物理像素，不做「按 OS 建议放大窗口」的换算）。
    pub extent: Extent,
    /// **DPI 缩放系数**（AF-3，Q4=**只透传**）：OS 报多少就是多少（`f64` 原样），
    /// 本层**不换算任何东西** —— 事件坐标仍是物理像素、`extent` 仍是物理尺寸，
    /// 布局仍是像素级纯函数。初值 = 建窗时 `window.scale_factor()`；之后随
    /// `ScaleFactorChanged` 事件同步更新（同一条 winit 报告，两处**同一值**）。
    pub scale_factor: f64,
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
    /// 滚动键（T3.2 剩余）：`PageUp` / `PageDown` 翻页，`Home` / `End` 到顶/到底。
    PageUp,
    PageDown,
    Home,
    End,
    /// 可打印字符键：**逻辑**字符（受当前布局与 Shift 影响，`Shift+a` ⇒ `Char('A')`）。
    Char(char),
    /// 其它一切：修饰键、功能键、编辑键、死键、多字符组合、无法识别的键。
    Other,
}

/// 本层自己的输入事件模型：**唯一**进入 [`App`](crate::App) 的输入表示。
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
        /// **系统按键重复**（长按不松时 OS 补发的 KeyDown）：`true` = 这是一条重复，
        /// `false` = 用户真的按下了一次（T3.6，winit 的 `event.repeat` 首次建模）。
        ///
        /// 交互层默认**不区分**（`handle` 照常消费）—— 要「忽略重复」的调用方自己过滤；
        /// 建模的意义是「能区分」而不是「替你决定」。
        repeat: bool,
    },
    KeyUp {
        key: Key,
        mods: Mods,
    },
    /// 文本输入：IME 的 `Commit` 结果，或**可打印**按键产生的文本（见 [`printable_text`]）。
    TextInput {
        text: String,
    },
    /// **IME 预编辑**（还没上屏的那一段）。
    ///
    /// ⚠️ 本枚举与 `deer_gui::interaction::InputEvent` 是**两份定义、必须逐字同步**
    /// （mirror 纪律）：变体名、字段名、语义都要对得上。
    ImePreedit {
        text: String,
    },
    FocusChanged {
        focused: bool,
    },
    /// **DPI 缩放系数变了**（AF-3，Q4=**只透传**）：OS 报多少就带多少（`f64` 原样，
    /// 不取整、不经 `f32` 折腾），本层**不换算任何坐标** —— 事件坐标与
    /// `WindowInfo::extent` 仍是同一套物理像素口径，布局仍是像素级纯函数（红线）。
    /// 要按 DPI 缩放是上层自己的事。
    ///
    /// ⚠️ **窗口物理尺寸刻意保持不变**（不碰 winit 给的 `InnerSizeWriter`）：winit 的
    /// 语义是「写了它 = 改窗口尺寸；不写 = 窗口保持现有物理像素」（Windows 后端
    /// `runner.rs` 的 `dispatch_event`：只有写了**不同**值才 `set_size`）。因此本事件
    /// **不会**跟着一条 `Resized`（物理尺寸没变），`App::resized` 不触发 —— 这条变化
    /// 只有 `scale_factor` 本身。窗口层在派发本事件**之前**已把
    /// `WindowInfo::scale_factor` 记账到新值（`App::input` 拿到的 `info` 就是新口径）。
    ///
    /// ⚠️ 本枚举与 `deer_gui::interaction::InputEvent` 是**两份定义、必须逐字同步**
    /// （mirror 纪律）：变体名、字段名、语义都要对得上。
    ScaleFactorChanged {
        scale_factor: f64,
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
        // 滚动键（T3.2 剩余）：PageUp/PageDown/Home/End 首次映射（此前落进 `Other`）。
        KeyboardKey::Named(NamedKey::PageUp) => Key::PageUp,
        KeyboardKey::Named(NamedKey::PageDown) => Key::PageDown,
        KeyboardKey::Named(NamedKey::Home) => Key::Home,
        KeyboardKey::Named(NamedKey::End) => Key::End,
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

/// 把 `(HWND, HINSTANCE)` 打包成 HAL 的不透明句柄。
///
/// 纯函数、不做平台判定 —— 平台判定在 [`raw_handle_from_rwh06`] / [`run()`](crate::run) 里。
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
///
/// `pub(crate)`：L3（`host`）建窗后要拿它去喂 `App::init`／`App::input`。
pub(crate) fn window_info(window: &Window) -> Result<WindowInfo, String> {
    let handle = window
        .window_handle()
        .map_err(|e| format!("取原生窗口句柄失败：{e}"))?;
    let raw = raw_handle_from_rwh06(handle.as_raw())?;
    let size = window.inner_size();
    // AF-3：scale_factor 与 extent **同一来源**（winit 的窗口对象），建窗时取一次。
    let scale_factor = window.scale_factor();
    Ok(WindowInfo { raw, extent: Extent { width: size.width, height: size.height }, scale_factor })
}

// ———————————————————————————————————————————————————————————————
// 剪贴板（AF-2）：自写 Win32 `CF_UNICODETEXT`，不引第三方（Q3 裁断）
// ———————————————————————————————————————————————————————————————

/// 非 Windows 平台上 [`Clipboard`] 的构造与两个方法都返回这句话。
///
/// **不静默**：要么真的复制/读取成功，要么明确说「本平台没实现」—— 绝不假装成功、
/// 也不静默丢数据。与 [`UNSUPPORTED_PLATFORM_MSG`] 同一条纪律的另一处落实。
pub const CLIPBOARD_UNSUPPORTED_MSG: &str =
    "deer-window 的剪贴板目前只实现了 Windows（CF_UNICODETEXT，纯文本）；当前平台明确不支持";

/// **剪贴板句柄**（AF-2）：自写 Win32 `OpenClipboard` / `EmptyClipboard` /
/// `SetClipboardData` / `GetClipboardData`，只有 `CF_UNICODETEXT` 一种格式；
/// **不引第三方**（引 `arboard` 要走依赖例外登记，Q3 已裁断自写 —— 就这几十行 FFI）。
///
/// # 为什么是「公共构造器」，不是「`init` 交出句柄（与 [`Waker`](crate::Waker) 同型）」
///
/// （App 地基任务书 D4 给了两案、本层择一并在此写死理由，免得后人再争一遍：）
///
/// 1. [`Waker`] 之所以要经 `App::wake_handle` **交接**，是因为 `EventLoopProxy` 只有
///    事件循环内部造得出来 —— 它是 host 私产的边角。剪贴板相反：它是 **OS 全局资源**，
///    跟窗口、事件循环都没有生命周期耦合（`OpenClipboard(NULL)` 不需要任何窗口）⇒
///    为它扩 `App` trait（**永久面**）买不到任何东西。
/// 2. **非 Windows 的「明确 Unsupported」必须真的可达**：交接式在非 Windows 根本走不到
///    （`run()` 在 `window_info` 一步就已 `Err`，任何 `App` 回调都不会被调），
///    「Unsupported」会变成一句没人能触发的死字。公共构造器让**任何平台**的用户都能调
///    [`Clipboard::new`] 并拿到明确的 [`CLIPBOARD_UNSUPPORTED_MSG`]。
/// 3. **调用点最短**：粘贴发生在输入处理处，而 `App::input` 的签名里就带着
///    `info: &WindowInfo`（`info.raw.handle` 即 HWND）—— 现场
///    `Clipboard::new(info.raw.handle)` 即用即走，App 不必为存句柄加状态。
///
/// # 语义
///
/// - **每次 `set_text` / `get_text` 内部原子地完成「开（重试）→ 干活 → 关」**：不跨调用
///   持有剪贴板 —— 跨调用持有会卡住全系统其它程序的复制粘贴；
/// - `hwnd` 是打开剪贴板时登记的**属主**窗口（`EmptyClipboard` 会把剪贴板交给它）：
///   写入**必须**用真实窗口（NULL 属主 ⇒ `SetClipboardData` 失败，Win32 明文 +
///   本机实测，见 [`Clipboard::set_text`]）；读取（`get_text`）不需要属主，`0` 可用。
///   我们做**立即渲染**（`SetClipboardData` 直接给数据），永远用不到延迟渲染的
///   `WM_RENDERFORMAT`；
/// - **可在任意线程调用**（开与关发生在同一次方法调用里，满足 Win32 的线程约束）。
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Clipboard {
    /// 打开剪贴板时关联的窗口（HWND；`0` = 与当前任务关联）。仅传给 Win32，调用方不用读它
    /// （非 Windows 目标上没有任何人读它 ⇒ 显式 `allow`，别让交叉编译多一条警告）。
    #[cfg_attr(not(windows), allow(dead_code))]
    hwnd: usize,
}

// 与 `Waker` 同一条先例：`Debug` 只说「有个句柄」，不把内部值打出来。
impl std::fmt::Debug for Clipboard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.pad("Clipboard { .. }")
    }
}

#[cfg(windows)]
impl Clipboard {
    /// 构造剪贴板句柄（Windows）。
    ///
    /// 构造本身**不碰**剪贴板（真正的 `OpenClipboard` 在每次 `set_text` / `get_text`
    /// 里原子完成，带重试）；所以 Windows 上它不会失败 —— `Result` 是为了给非 Windows
    /// 留出「明确 `Unsupported`」的通道（见 [`CLIPBOARD_UNSUPPORTED_MSG`]），两个平台
    /// 的调用方写法逐字相同。
    ///
    /// `hwnd` 传 `WindowInfo::raw.handle`（建窗后就有）。**传 `0` 只够用来读**
    /// （`get_text` 不需要属主）；**写必须用真实窗口** —— Win32 明文（`EmptyClipboard`
    /// 的 Remarks）：用 NULL 窗口句柄打开剪贴板时，`EmptyClipboard` 会把属主设成
    /// NULL，这会让 `SetClipboardData` 失败（本机实测同样如此，且表现为**时灵时不灵**）。
    /// `set_text` 对 `hwnd = 0` 在动手前就明确拒绝，不让你撞上那面墙。
    pub fn new(hwnd: usize) -> Result<Clipboard, String> {
        Ok(Clipboard { hwnd })
    }

    /// 把 `text` 写进系统剪贴板（`CF_UNICODETEXT`，UTF-16 + 结尾 NUL）。
    ///
    /// 成功 ⇒ **所有权交给系统**（缓冲由系统在下次 `EmptyClipboard`/退出时释放）；
    /// 失败 ⇒ 明确 `Err`，已分配的缓冲当场 `GlobalFree`（不漏）。注意 `EmptyClipboard`
    /// 成功而 `SetClipboardData` 失败的窗口期里，**原有的剪贴板内容已经没了**（那是
    /// `EmptyClipboard` 的语义）—— 这个错误信息里会写明。
    pub fn set_text(&self, text: &str) -> Result<(), String> {
        if self.hwnd == 0 {
            // 显式拒绝，不让人撞 MSDN 那面墙（NULL 属主 ⇒ SetClipboardData 失败，
            // 且实测是「时灵时不灵」的静默坏法 —— 比直接失败更难查）。
            return Err(
                "set_text 需要**真实窗口句柄**（hwnd = 0 是 NULL 属主：Win32 规定此时 \
                 SetClipboardData 会失败）；建窗后用 WindowInfo::raw.handle"
                    .to_string(),
            );
        }
        let units = text_to_utf16_nul(text)?;
        win32::set_text(self.hwnd, &units)
    }

    /// 读出系统剪贴板里的文本（`CF_UNICODETEXT`）。
    ///
    /// 剪贴板为空、或放的是图片/文件等**非文本**格式 ⇒ 明确 `Err`（**不静默给空串**）；
    /// 内容不是合法 UTF-16（残缺代理对）⇒ 同样明确 `Err`，**不做 lossy 替换** ——
    /// 替换出来的「看起来差不多」的字符串正是「静默改数据」。
    pub fn get_text(&self) -> Result<String, String> {
        let units = win32::get_text(self.hwnd)?;
        utf16_nul_to_text(&units)
    }
}

#[cfg(not(windows))]
impl Clipboard {
    /// 非 Windows：明确 `Err`（[`CLIPBOARD_UNSUPPORTED_MSG`]），**不静默**。
    pub fn new(_hwnd: usize) -> Result<Clipboard, String> {
        Err(CLIPBOARD_UNSUPPORTED_MSG.to_string())
    }

    /// 非 Windows：明确 `Err`（[`CLIPBOARD_UNSUPPORTED_MSG`]），**不静默**。
    pub fn set_text(&self, _text: &str) -> Result<(), String> {
        Err(CLIPBOARD_UNSUPPORTED_MSG.to_string())
    }

    /// 非 Windows：明确 `Err`（[`CLIPBOARD_UNSUPPORTED_MSG`]），**不静默**。
    pub fn get_text(&self) -> Result<String, String> {
        Err(CLIPBOARD_UNSUPPORTED_MSG.to_string())
    }
}

/// Rust 文本 → **以 NUL 结尾**的 UTF-16（`CF_UNICODETEXT` 的内存形态）。
///
/// **含 NUL 的文本明确拒绝**：`CF_UNICODETEXT` 以第一个 NUL 结尾，带着 NUL 写进去
/// 会被**所有**读取方截断（记事本只见前半段）—— 那是静默丢数据，不如当场报错。
/// （非 Windows 目标上只有单测用它 ⇒ 显式 `allow`，别让交叉编译多两条警告。）
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn text_to_utf16_nul(text: &str) -> Result<Vec<u16>, String> {
    if text.contains('\0') {
        return Err(
            "文本包含 NUL 字符（\\0）：CF_UNICODETEXT 以 NUL 结尾，无法无损表示，已明确拒绝"
                .to_string(),
        );
    }
    let mut units: Vec<u16> = text.encode_utf16().collect();
    units.push(0); // 结尾 NUL：CF_UNICODETEXT 的终止符
    Ok(units)
}

/// **以 NUL 结尾**的 UTF-16 → Rust 文本（严格：非法 UTF-16 明确报错，**不做 lossy 替换**）。
/// （非 Windows 目标上只有单测用它 ⇒ 同上，显式 `allow`。）
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn utf16_nul_to_text(units: &[u16]) -> Result<String, String> {
    let len = units.iter().position(|&u| u == 0).ok_or_else(|| {
        "CF_UNICODETEXT 缓冲里没有结尾的 NUL —— 内容不是合法的文本格式".to_string()
    })?;
    String::from_utf16(&units[..len])
        .map_err(|e| format!("CF_UNICODETEXT 内容不是合法 UTF-16（代理对残缺？）：{e}"))
}

/// Win32 剪贴板 FFI（**私有**；只在 `cfg(windows)` 下编译）。
///
/// 只声明用到的九个函数，类型按 Win32 的宽度写死（指针 = `*mut c_void`、
/// `BOOL` = `i32`、`UINT` = `u32`）；**不引 `windows-rs`**（Q3：为几十行 FFI 开依赖
/// 例外不划算，也与「除 winit 外零第三方依赖」的口径冲突）。
#[cfg(windows)]
mod win32 {
    #[link(name = "user32")]
    unsafe extern "system" {
        fn OpenClipboard(hwndnewowner: *mut core::ffi::c_void) -> i32;
        fn CloseClipboard() -> i32;
        fn EmptyClipboard() -> i32;
        fn SetClipboardData(uformat: u32, hmem: *mut core::ffi::c_void) -> *mut core::ffi::c_void;
        fn GetClipboardData(uformat: u32) -> *mut core::ffi::c_void;
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GlobalAlloc(uflags: u32, dwbytes: usize) -> *mut core::ffi::c_void;
        fn GlobalLock(hmem: *mut core::ffi::c_void) -> *mut core::ffi::c_void;
        fn GlobalUnlock(hmem: *mut core::ffi::c_void) -> i32;
        fn GlobalSize(hmem: *mut core::ffi::c_void) -> usize;
        fn GlobalFree(hmem: *mut core::ffi::c_void) -> *mut core::ffi::c_void;
        fn GetLastError() -> u32;
        fn Sleep(dwmilliseconds: u32);
    }

    /// `CF_UNICODETEXT`：UTF-16 字符串、以 NUL 结尾（Win32 剪贴板的标准文本格式编号）。
    const CF_UNICODETEXT: u32 = 13;
    /// `GMEM_MOVEABLE`：剪贴板要求可移动内存（`SetClipboardData` 的约定）。
    const GMEM_MOVEABLE: u32 = 0x0002;
    /// `OpenClipboard` 的重试上界：剪贴板是**全系统互斥**资源（任何别的进程开着它时这里
    /// 就失败），撞一下就报错会让合法调用假红 —— 每 2ms 试一次、共约 100ms，仍失败才报。
    const OPEN_ATTEMPTS: u32 = 50;
    const OPEN_RETRY_SLEEP_MS: u32 = 2;

    /// `OpenClipboard`（带重试）。失败重试的理由见 [`OPEN_ATTEMPTS`]。
    fn open_with_retry(hwnd: usize) -> Result<(), String> {
        let mut last_err = 0u32;
        for attempt in 0..OPEN_ATTEMPTS {
            if unsafe { OpenClipboard(hwnd as *mut core::ffi::c_void) } != 0 {
                return Ok(());
            }
            last_err = unsafe { GetLastError() };
            if attempt + 1 < OPEN_ATTEMPTS {
                unsafe { Sleep(OPEN_RETRY_SLEEP_MS) };
            }
        }
        Err(format!(
            "OpenClipboard 连续 {OPEN_ATTEMPTS} 次失败（GetLastError={last_err}）\
             —— 剪贴板被其它进程长期占用"
        ))
    }

    /// 写入：缓冲先在自己进程备好（**不占着**全系统互斥的剪贴板做内存拷贝），再
    /// 开 → 清空 → 交出所有权 → 关。每个失败分支都把已分配的缓冲 `GlobalFree` 掉。
    pub(super) fn set_text(hwnd: usize, units: &[u16]) -> Result<(), String> {
        let bytes = units.len() * 2; // 含结尾 NUL 的完整字节数
        let mem = unsafe { GlobalAlloc(GMEM_MOVEABLE, bytes) };
        if mem.is_null() {
            return Err(format!("GlobalAlloc 失败（要分配 {bytes} 字节）"));
        }
        // ① 锁住并逐字节拷入（含结尾 NUL），然后立刻解锁。
        let locked = unsafe { GlobalLock(mem) };
        if locked.is_null() {
            unsafe { GlobalFree(mem) };
            return Err("GlobalLock 失败（写入剪贴板缓冲）".to_string());
        }
        unsafe {
            core::ptr::copy_nonoverlapping(units.as_ptr(), locked as *mut u16, units.len());
            GlobalUnlock(mem);
        }
        // ② 开（重试）→ 清空 → 交所有权 → 关。
        if let Err(e) = open_with_retry(hwnd) {
            unsafe { GlobalFree(mem) };
            return Err(e);
        }
        if unsafe { EmptyClipboard() } == 0 {
            let err = unsafe { GetLastError() };
            unsafe { CloseClipboard() };
            unsafe { GlobalFree(mem) };
            return Err(format!("EmptyClipboard 失败（GetLastError={err}）"));
        }
        if unsafe { SetClipboardData(CF_UNICODETEXT, mem) }.is_null() {
            // 失败 ⇒ 所有权还在我们手里，必须释放（否则漏一块 GMEM_MOVEABLE）。
            // ⚠️ 此时原剪贴板内容已被 EmptyClipboard 清掉 —— 错误里写明这一点。
            let err = unsafe { GetLastError() };
            unsafe { CloseClipboard() };
            unsafe { GlobalFree(mem) };
            return Err(format!(
                "SetClipboardData(CF_UNICODETEXT) 失败（GetLastError={err}；\
                 原有剪贴板内容已被清空）"
            ));
        }
        // 成功 ⇒ `mem` 的所有权归系统，**不**再由我们释放；只剩关剪贴板。
        // CloseClipboard 失败极罕见且数据已进剪贴板（下 anyone 开前系统自己会处理），
        // 这里忽略返回值、不回滚 —— 数据没有丢，不谎报失败。
        unsafe { CloseClipboard() };
        Ok(())
    }

    /// 读出：开（重试）→ 取句柄 → 锁 → 拷出 → 解锁 → 关。
    /// 没有 `CF_UNICODETEXT`（空/非文本）⇒ 明确 `Err`，**不静默给空串**。
    pub(super) fn get_text(hwnd: usize) -> Result<Vec<u16>, String> {
        open_with_retry(hwnd)?;
        let result = (|| {
            let mem = unsafe { GetClipboardData(CF_UNICODETEXT) };
            if mem.is_null() {
                return Err(
                    "剪贴板当前没有 CF_UNICODETEXT 文本（为空，或放的是图片/文件等非文本格式）"
                        .to_string(),
                );
            }
            let size = unsafe { GlobalSize(mem) }; // 字节数
            if size < 2 {
                return Err(format!(
                    "CF_UNICODETEXT 缓冲只有 {size} 字节，装不下一个 UTF-16 单元"
                ));
            }
            let locked = unsafe { GlobalLock(mem) };
            if locked.is_null() {
                return Err("GlobalLock 失败（读取剪贴板缓冲）".to_string());
            }
            let units = unsafe {
                let count = size / 2;
                core::slice::from_raw_parts(locked as *const u16, count).to_vec()
            };
            unsafe { GlobalUnlock(mem) };
            Ok(units)
        })();
        // 无论成败都先关剪贴板（不能跨调用占着全系统互斥的资源），再交出结果。
        unsafe { CloseClipboard() };
        result
    }
}

/// 剪贴板单测（AF-2）。**碰真 OS 剪贴板的只有一条**（全系统互斥的资源，别让多条测试
/// 并行互踩）；其余都是纯编解码判据，任何平台都能跑。
#[cfg(test)]
mod clipboard_tests {
    use super::{CLIPBOARD_UNSUPPORTED_MSG, text_to_utf16_nul, utf16_nul_to_text};
    // `Clipboard` 的构造判据只有 Windows 那条用到（非 Windows 上构造返回 Err，
    // 没有可断言的句柄本身）⇒ 按目标引入，别让交叉编译多一条 unused 警告。
    #[cfg(windows)]
    use super::Clipboard;

    /// **编解码往返保真**（含中文 / emoji 代理对 / ZWJ 序列 / 空串 / ASCII）——
    /// 「多字节不丢字」的第一道判据，纯函数，任何平台都跑。
    #[test]
    fn utf16_codec_round_trips_multibyte_text() {
        let cases = [
            "",
            "plain ascii 123",
            "中文往返：你好，世界",
            "é è ñ（2 字节）",
            "emoji 😀🎉（代理对）",
            "ZWJ 序列 👨‍👩‍👧 与组合 ✔︎",
            "mixed aA1 中 😀 tail",
        ];
        for text in cases {
            let units = text_to_utf16_nul(text).expect("这些用例都不含 NUL，编不出错");
            assert_eq!(
                units.last(),
                Some(&0),
                "结尾必须有 NUL（CF_UNICODETEXT 的终止符）：{text:?}"
            );
            let got = utf16_nul_to_text(&units).expect("自己编的合法 UTF-16 必须能解回来");
            assert_eq!(got, text, "编解码往返必须逐字符保真");
        }
    }

    /// **含 NUL 的输入明确拒绝**（不静默截断）：这是「往返保真」的边界判据 ——
    /// 放行它就是放行「写进去了但读回来变短」这种静默丢数据。
    #[test]
    fn codec_rejects_nul_in_input_instead_of_silent_truncation() {
        let err = text_to_utf16_nul("a\0b").expect_err("含 NUL 的文本必须被拒绝");
        assert!(err.contains("NUL"), "错误信息要点明原因：{err}");
        // 解码侧按 CF_UNICODETEXT 语义**只见到第一个 NUL**（这正是「静默截断」本身）：
        // 带内嵌 NUL 的缓冲解回来只剩 "a" —— 所以拒绝必须发生在**写入之前**。
        let truncated = utf16_nul_to_text(&[0x61, 0, 0x62, 0]).expect("按定义能解到第一个 NUL");
        assert_eq!(truncated, "a", "第一个 NUL 之后的内容不属于 CF_UNICODETEXT 文本");
    }

    /// **非法 UTF-16 明确报错，不做 lossy 替换**：高代理对后面跟的不是低代理对 ⇒ `Err`。
    /// （把解码改成 `from_utf16_lossy` 的变异会让这条变红 —— 静默替换 U+FFFD 不许有。）
    #[test]
    fn codec_rejects_broken_utf16_instead_of_lossy_replacement() {
        // 0xD83D 是高代理，后面跟普通字符（没有低代理）⇒ 非法序列。
        let err = utf16_nul_to_text(&[0xD83D, 0x0041, 0]).expect_err("残缺代理对必须报错");
        assert!(err.contains("UTF-16"), "错误信息要点明原因：{err}");
    }

    /// **Unsupported 文案是真的明话**（不是空串、不是含糊其辞）：
    /// 非 Windows 分支返回的就是它 —— 文案本身也是契约的一部分。
    #[test]
    fn unsupported_message_is_explicit() {
        assert!(!CLIPBOARD_UNSUPPORTED_MSG.is_empty());
        assert!(
            CLIPBOARD_UNSUPPORTED_MSG.contains("Windows") && CLIPBOARD_UNSUPPORTED_MSG.contains("不支持"),
            "文案必须写清「只实现到哪 + 当前不支持」：{CLIPBOARD_UNSUPPORTED_MSG}"
        );
    }

    /// **`hwnd = 0`（NULL 属主）的写入在动手前就被明确拒绝** —— 这是把 Win32 的坑
    /// 变成确定性契约的判据：`EmptyClipboard` 的 Remarks（MSDN）写明 NULL 属主会让
    /// `SetClipboardData` 失败，本机实测还是「时灵时不灵」的坏法。这条测试**不碰**
    /// 剪贴板（拒绝发生在打开之前），任何环境下都确定。
    ///
    /// 变异提示：删掉 `set_text` 里的 `hwnd == 0` 守卫，这条就会真的走 OS 路径并返回
    /// 别的错误（或「成功」）⇒ 红。
    #[cfg(windows)]
    #[test]
    fn set_text_rejects_null_owner_upfront() {
        let cb = Clipboard::new(0).expect("构造本身不碰剪贴板，hwnd=0 也能构造（够 get_text 用）");
        let err = cb.set_text("x").expect_err("NULL 属主的写入必须在打开剪贴板之前被拒绝");
        assert!(
            err.contains("真实窗口") && err.contains("hwnd"),
            "错误信息要点明「要真实窗口句柄」：{err}"
        );
    }
}
