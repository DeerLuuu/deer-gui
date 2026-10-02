//! deer-window 的**纯逻辑**单测：不建窗口、不跑事件循环。
//!
//! 真开窗的验证在 `examples/window_smoke.rs` 里（CI 无桌面，`#[test]` 里真开窗会假红）。

use deer_gpu::{Extent, Platform, RawWindowHandle};
use deer_window::{
    Flow, FrameCounter, UNSUPPORTED_PLATFORM_MSG, WindowConfig, WindowInfo, raw_handle_from_rwh06,
    raw_handle_from_win32,
};
use std::num::NonZeroIsize;
use winit::raw_window_handle::{
    RawWindowHandle as RwhRawWindowHandle, WaylandWindowHandle, Win32WindowHandle, XlibWindowHandle,
};

/// `WindowConfig::new` 把三个字段**逐字**存进去：不截断、不钳制、不做默认值替换。
#[test]
fn window_config_new_keeps_fields_verbatim() {
    let cfg = WindowConfig::new("主窗口", 1280, 720);
    assert_eq!(cfg.title, "主窗口");
    assert_eq!(cfg.width, 1280);
    assert_eq!(cfg.height, 720);

    // `impl Into<String>`：`&str` 与 `String` 都能传。
    let owned: String = String::from("owned");
    let from_string = WindowConfig::new(owned, 1, 2);
    assert_eq!(from_string.title, "owned");

    // 边界值照存（包括 0 与 u32::MAX）——本层不做「合理值」假设，那是调用方的事。
    let edge = WindowConfig::new("", 0, u32::MAX);
    assert_eq!(edge.title, "");
    assert_eq!(edge.width, 0);
    assert_eq!(edge.height, u32::MAX);
}

/// 默认值：无标题、800×600，标题栏显示为 `deer-gui`。
#[test]
fn window_config_default() {
    let cfg = WindowConfig::default();
    assert_eq!(cfg, WindowConfig::new(String::new(), 800, 600));
    assert_eq!(
        cfg,
        WindowConfig { title: String::new(), width: 800, height: 600 }
    );
    assert_eq!(cfg.display_title(), "deer-gui");
}

/// 标题加 `deer-gui` 前缀；标题为空时不留孤零零的分隔符。
#[test]
fn window_config_display_title() {
    assert_eq!(WindowConfig::new("preview", 320, 200).display_title(), "deer-gui — preview");
    assert_eq!(WindowConfig::new("", 320, 200).display_title(), "deer-gui");
    // 中文 UTF-8 标题原样保留。
    assert_eq!(WindowConfig::new("汉字窗口", 320, 200).display_title(), "deer-gui — 汉字窗口");
}

/// `WindowInfo` 只是三个字段的载体：句柄、尺寸、DPI 缩放系数原样可读/可比较（Copy；
/// `Eq` 因 `scale_factor: f64` 放弃，`PartialEq` 照常可用）。
#[test]
fn window_info_carries_handle_extent_and_scale_factor() {
    let info = WindowInfo {
        raw: raw_handle_from_win32(0x1234, 0x400000),
        extent: Extent { width: 320, height: 200 },
        scale_factor: 1.25,
    };
    let copied = info; // Copy
    assert_eq!(copied, info);
    assert_eq!(info.raw, RawWindowHandle { platform: Platform::Windows, handle: 0x1234, display: 0x400000 });
    assert_eq!(info.extent, Extent { width: 320, height: 200 });
    // AF-3：scale_factor 只透传 —— 存进去什么读出来就是什么（OS 报多少就是多少）。
    assert_eq!(info.scale_factor, 1.25, "scale_factor 必须原样可读（透传，不换算）");
}

/// 句柄打包（纯函数）：`hwnd → handle`、`hinstance → display`、`platform = Windows`。
#[test]
fn raw_handle_from_win32_packs_fields() {
    let raw = raw_handle_from_win32(0x0000_0000_0001_0A2B, 0x0000_7FF6_1234_0000);
    assert_eq!(raw.platform, Platform::Windows);
    assert_eq!(raw.handle, 0x0000_0000_0001_0A2B, "handle 必须是 HWND");
    assert_eq!(raw.display, 0x0000_7FF6_1234_0000, "display 必须是 HINSTANCE");

    // 0 也照打包：本函数不做平台/有效性判定（「不静默给 0」的判定在上层）。
    let zero = raw_handle_from_win32(0, 0);
    assert_eq!(zero.handle, 0);
    assert_eq!(zero.display, 0);
}

/// rwh-0.6 的 `Win32` 句柄 → 我们的不透明句柄：hwnd/hinstance 一一对应。
#[test]
fn raw_handle_from_rwh06_maps_win32() {
    let mut win32 = Win32WindowHandle::new(NonZeroIsize::new(0x1A2B).expect("非零 HWND"));
    win32.hinstance = NonZeroIsize::new(0x7FF6_0000);
    let raw = raw_handle_from_rwh06(RwhRawWindowHandle::Win32(win32)).expect("Win32 必须成功");
    assert_eq!(raw, RawWindowHandle { platform: Platform::Windows, handle: 0x1A2B, display: 0x7FF6_0000 });
}

/// Win32 句柄缺 `hinstance` 时不许瞎填：返回 Err（而不是 `display = 0`）。
#[test]
fn raw_handle_from_rwh06_rejects_win32_without_hinstance() {
    let win32 = Win32WindowHandle::new(NonZeroIsize::new(0x1A2B).expect("非零 HWND"));
    assert_eq!(win32.hinstance, None);
    let err = raw_handle_from_rwh06(RwhRawWindowHandle::Win32(win32)).unwrap_err();
    assert!(err.contains("hinstance"), "错误文案应点明缺的是 hinstance，实际：{err}");
}

/// 非 Windows 平台的 `Err` 分支：**在 Windows 上也能测**（纯函数，喂别的句柄进去即可）。
/// 文案必须与 `UNSUPPORTED_PLATFORM_MSG` 逐字一致，且绝不能返回了一个填 0 的句柄。
#[test]
fn raw_handle_from_rwh06_rejects_non_windows_platforms() {
    let xlib = XlibWindowHandle::new(0x2A);
    let err = raw_handle_from_rwh06(RwhRawWindowHandle::Xlib(xlib)).unwrap_err();
    assert_eq!(err, UNSUPPORTED_PLATFORM_MSG);
    assert_eq!(
        UNSUPPORTED_PLATFORM_MSG,
        "deer-window 目前只实现了 Windows 窗口（winit 后端已就绪，但 RawWindowHandle 的填法未实现）"
    );

    let wayland = WaylandWindowHandle::new(std::ptr::NonNull::<std::ffi::c_void>::dangling());
    let err = raw_handle_from_rwh06(RwhRawWindowHandle::Wayland(wayland)).unwrap_err();
    assert_eq!(err, UNSUPPORTED_PLATFORM_MSG);
}

/// `Flow` 语义：`FrameCounter` 是 `run()` 账本的纯逻辑核心。
/// - `Continue` ⇒ 帧数 +1、不退出；
/// - `Exit` ⇒ 帧数 +1（这一帧确实画成功了）且置退出标记；
/// - `Err` ⇒ 帧数不变、错误向上冒（`run()` 会打印并返回 Err，不吞）。
#[test]
fn flow_semantics_via_frame_counter() {
    let mut counter = FrameCounter::new();
    assert_eq!(counter.frames(), 0);
    assert!(!counter.exit_requested());

    counter.on_redraw(Ok(Flow::Continue)).expect("Continue 不是错误");
    assert_eq!(counter.frames(), 1);
    assert!(!counter.exit_requested());

    counter.on_redraw(Ok(Flow::Continue)).expect("Continue 不是错误");
    assert_eq!(counter.frames(), 2);
    assert!(!counter.exit_requested());

    counter.on_redraw(Ok(Flow::Exit)).expect("Exit 也不是错误");
    assert_eq!(counter.frames(), 3, "Exit 那一帧也算成功画过");
    assert!(counter.exit_requested());

    // 出错的那一次不计数，错误原样冒出来。
    let mut failing = FrameCounter::new();
    let err = failing.on_redraw(Err("交换链过期".to_string())).unwrap_err();
    assert_eq!(err, "交换链过期");
    assert_eq!(failing.frames(), 0);
    assert!(!failing.exit_requested());
}

/// `Flow` 是 Copy + PartialEq 的小枚举：可以随便传、随便比。
#[test]
fn flow_is_copy_and_comparable() {
    let flow = Flow::Continue;
    let copied = flow;
    assert_eq!(copied, Flow::Continue);
    assert_ne!(Flow::Exit, Flow::Continue);
    assert!(matches!(Flow::Exit, Flow::Exit));
    let label = match copied {
        Flow::Continue => "continue",
        Flow::Exit => "exit",
    };
    assert_eq!(label, "continue");
}
