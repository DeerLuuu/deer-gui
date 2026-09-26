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
//! use deer_window::{App, Flow, WindowConfig, WindowInfo, run};
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
//! - **输入事件**（键盘/鼠标/IME/滚轮）不在本期范围内：事件循环只把
//!   `Resized` / `RedrawRequested` / `CloseRequested` 转发给 [`App`]，其余事件丢弃，
//!   留给 M5 的输入层。
//! - **DPI**：只透传 `Resized` 给的物理像素，不做任何缩放换算；`LogicalSize` 只在建窗时用。
//! - 事件循环用 [`ControlFlow::Poll`] + `about_to_wait` 里 `request_redraw()` 形成连续重绘；
//!   真正上屏时节奏由呈现（vsync）决定，本层只管「一直要下一帧」。

use std::sync::Arc;

use deer_gpu::Extent;
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
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

    /// 点了关闭按钮/系统关闭：默认允许关闭。
    fn close_requested(&mut self) -> Flow {
        Flow::Exit
    }
}

/// 帧计数 + 退出标记 —— [`run()`] 内部账本的**纯逻辑核心**（不碰窗口，可直接单测）。
///
/// `run()` 用它统计「`App::redraw` 被**成功**调用的次数」（回调返回 `Err` 的那次不算），
/// 并在 `Flow::Exit` 时置上退出标记。
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct FrameCounter {
    frames: u64,
    exit_requested: bool,
}

impl FrameCounter {
    pub fn new() -> FrameCounter {
        FrameCounter::default()
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
/// 无论哪种收尾，都会往 stdout 打一行摘要（帧数 + 最终物理尺寸 + 结果），
/// 便于脚本/验证断言：
/// `[deer-window] 事件循环结束：frames=<n> extent=<w>x<h> result=ok|error`。
pub fn run<A: App + 'static>(config: WindowConfig, app: A) -> Result<(), String> {
    let event_loop =
        EventLoop::new().map_err(|e| format!("创建 winit EventLoop 失败（run() 必须在主线程调用）：{e}"))?;
    let mut handler = RunHandler {
        config,
        app,
        window: None,
        extent: Extent { width: 0, height: 0 },
        counter: FrameCounter::new(),
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

/// [`run()`] 的 `ApplicationHandler` 实现：事件循环 → `App` 回调的接线。
struct RunHandler<A: App> {
    config: WindowConfig,
    app: A,
    /// `resumed` 里建好后由事件循环持有；窗口活到 `run()` 结束。
    window: Option<Arc<Window>>,
    /// 最近一次已知的物理尺寸（初始取自 `inner_size()`，之后由 `Resized` 更新）。
    extent: Extent,
    counter: FrameCounter,
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
        match err {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }
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

        // 连续重绘：Poll 让事件循环不睡死，about_to_wait 里 request_redraw 产生下一帧。
        event_loop.set_control_flow(ControlFlow::Poll);
        self.window = Some(window);
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
                if let Err(e) = self.app.resized(size.width, size.height) {
                    self.fail(
                        event_loop,
                        format!("App::resized({}x{}) 失败：{e}", size.width, size.height),
                    )
                }
            }
            WindowEvent::RedrawRequested => {
                let result = self.app.redraw();
                match self.counter.on_redraw(result) {
                    Ok(()) => {
                        if self.counter.exit_requested() {
                            self.exiting = true;
                            event_loop.exit();
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
            // 其余事件（键盘/鼠标/IME/滚轮…）本期不转发：输入层留给 M5。
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if self.error.is_some() || self.exiting {
            return;
        }
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }
}
