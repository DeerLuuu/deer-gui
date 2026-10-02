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
//! - **DPI**：只透传 `Resized` 给的物理像素，不做任何缩放换算。
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

/// 交给渲染层的窗口信息（原生句柄 + **物理**尺寸）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowInfo {
    /// `platform = Windows`，`handle = HWND`，`display = HINSTANCE`。
    pub raw: deer_gpu::RawWindowHandle,
    /// 当前**物理**像素尺寸（`Resized` 事件的值）。
    pub extent: Extent,
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
    Ok(WindowInfo { raw, extent: Extent { width: size.width, height: size.height } })
}
