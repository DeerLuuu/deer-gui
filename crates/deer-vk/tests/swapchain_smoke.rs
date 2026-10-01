//! M2b 驱动验收：**surface + 交换链 + 帧同步 + 呈现**。
//!
//! ## 这个文件分两层
//!
//! 1. **纯逻辑 + 布局 + 扩展可用性**（默认就跑，不需要窗口）：
//!    - [`pick_config`] 的确定性规则（伪造的能力/格式/模式列表 → 期望配置）；
//!    - `Acquire`/`Present` 的**返回码映射**（`VK_TIMEOUT`/`VK_NOT_READY` 绝不当成功）；
//!    - `VkSwapchainCreateInfoKHR` / `VkPresentInfoKHR` 等结构体的大小与偏移断言；
//!    - `vkEnumerateInstanceExtensionProperties` 真的能查到 `VK_KHR_surface`。
//! 2. **真窗口端到端**（默认**跳过**）：用原生 Win32 `CreateWindowExW` 建一个真窗口
//!    （**不引 winit** —— `deer-vk` 不许有第三方依赖），跑完整链：
//!    实例 → surface → 设备（图形+呈现队列）→ 交换链 → 逐帧 acquire/画/呈现 → resize → 销毁。
//!
//! 第 2 层只在 `DEER_VK_WINDOW_TESTS=1` 时执行：无桌面环境（CI）会假红。
//! 跳过时**明确打印原因**，不伪装通过。
//!
//! 领队的端到端验证在 `deer-gui --features window --example window_preview`（走 winit），
//! 这里这条是为了「不依赖任何第三方 crate 也能自证驱动链路是通的」。

use deer_gpu::{Color, Extent, GpuError, Platform, RawWindowHandle};
use deer_vk::ffi;
use deer_vk::ffi_dev as vk;
use deer_vk::surface::{self, Surface, SurfaceCapabilitiesKHR, SurfaceFormatKHR};
use deer_vk::swapchain::{
    map_acquire_result, map_present_result, pick_config, readback_row_pitch, reorder_to_rgba8,
    Acquire, Present, PresentInfoKHR, SurfaceCapabilities, SurfaceFormat, SwapchainCreateInfoKHR,
    VK_COLOR_SPACE_SRGB_NONLINEAR_KHR, VK_FORMAT_B8G8R8A8_SRGB, VK_FORMAT_B8G8R8A8_UNORM,
    VK_FORMAT_R8G8B8A8_SRGB, VK_FORMAT_UNDEFINED, VK_PRESENT_MODE_FIFO_KHR,
    VK_PRESENT_MODE_IMMEDIATE_KHR, VK_PRESENT_MODE_MAILBOX_KHR,
};
use deer_vk::windowed::{FrameOutcome, WindowedRenderer};

/// `VK_FORMAT_R8G8B8A8_UNORM`（线性，用来验证「没有 sRGB 时的退回」）。
const VK_FORMAT_R8G8B8A8_UNORM: i32 = 37;
/// `VK_IMAGE_USAGE_COLOR_ATTACHMENT_BIT`
const USAGE_COLOR_ATTACHMENT: u32 = 1 << 4;
/// `VK_IMAGE_USAGE_TRANSFER_SRC_BIT`（回读呈现帧必需）
const USAGE_TRANSFER_SRC: u32 = 1 << 0;

/// 真窗口测试用的清屏色（**逐通道写成常量**，因为回读断言要逐字节比对）。
/// 与 `deer-gui` 示例 `window_preview` 里的 `CLEAR` 是同一个值。
const CLEAR_R: u8 = 0x10;
const CLEAR_G: u8 = 0x14;
const CLEAR_B: u8 = 0x24;

// ─────────────────────────────────────────────────────────────────────────────
// 1. 纯逻辑：pick_config
// ─────────────────────────────────────────────────────────────────────────────

fn fmt(format: i32) -> SurfaceFormat {
    SurfaceFormat {
        format,
        color_space: VK_COLOR_SPACE_SRGB_NONLINEAR_KHR,
    }
}

fn caps(
    min_image_count: u32,
    max_image_count: u32,
    current: Option<Extent>,
    min_extent: Extent,
    max_extent: Extent,
) -> SurfaceCapabilities {
    SurfaceCapabilities {
        min_image_count,
        max_image_count,
        current_extent: current,
        min_image_extent: min_extent,
        max_image_extent: max_extent,
        supported_composite_alpha: 0x1,
        current_transform: 0x1,
        // 默认给「能渲染 + 能回读」（真机 Windows surface 也是这两位的超集）
        supported_usage_flags: USAGE_COLOR_ATTACHMENT | USAGE_TRANSFER_SRC,
    }
}

fn e(w: u32, h: u32) -> Extent {
    Extent {
        width: w,
        height: h,
    }
}

/// **线性格式优先**（M3c 裁决）+ `FIFO` + 用 `currentExtent` + `minImageCount + 1`。
///
/// 为什么不是 sRGB：sRGB 附件连**混合**都在线性空间，而 CPU 参考实现按字节混合 ⇒
/// 半透明像素差几十字节（实测 `src=0xC0,a=0.5,dst=0`：96 vs 140，差 44）。
/// 那条实测与推导见 `pipelines.rs` 的模块文档。
#[test]
fn pick_config_prefers_linear_bgra8_and_fifo() {
    let c = caps(2, 8, Some(e(960, 600)), e(1, 1), e(8192, 8192));
    let formats = [
        fmt(VK_FORMAT_R8G8B8A8_UNORM),
        fmt(VK_FORMAT_B8G8R8A8_UNORM),
        fmt(VK_FORMAT_B8G8R8A8_SRGB),
        fmt(VK_FORMAT_R8G8B8A8_SRGB),
    ];
    // 故意把 FIFO 放在中间：规则是「优先 FIFO」，不是「取第一个」
    let modes = [
        VK_PRESENT_MODE_MAILBOX_KHR,
        VK_PRESENT_MODE_FIFO_KHR,
        VK_PRESENT_MODE_IMMEDIATE_KHR,
    ];
    let cfg = pick_config(&c, &formats, &modes, e(100, 100)).expect("应当挑得出来");
    assert_eq!(
        cfg.format, VK_FORMAT_B8G8R8A8_UNORM,
        "线性 BGRA 优先（其次线性 RGBA，再才是 sRGB）"
    );
    assert_eq!(cfg.present_mode, VK_PRESENT_MODE_FIFO_KHR);
    assert_eq!(cfg.extent, e(960, 600), "currentExtent 有值时必须用它");
    assert_eq!(cfg.image_count, 3, "minImageCount + 1");
}

/// 只有线性 RGBA 时选它（而不是退回 sRGB）。
#[test]
fn pick_config_prefers_linear_rgba_when_no_bgra() {
    let c = caps(2, 8, None, e(1, 1), e(4096, 4096));
    let formats = [fmt(VK_FORMAT_R8G8B8A8_UNORM), fmt(VK_FORMAT_B8G8R8A8_SRGB)];
    let cfg = pick_config(&c, &formats, &[VK_PRESENT_MODE_FIFO_KHR], e(640, 480)).unwrap();
    assert_eq!(cfg.format, VK_FORMAT_R8G8B8A8_UNORM);
}

/// 只有 sRGB 时**如实退回**（此时窗口与 CPU 的混合空间不同 ⇒
/// `window_parity` 会打印实际格式，不许悄悄按逐字节判）。
#[test]
fn pick_config_falls_back_to_srgb_only_when_no_linear_format() {
    let c = caps(2, 8, None, e(1, 1), e(4096, 4096));
    let formats = [fmt(VK_FORMAT_B8G8R8A8_SRGB), fmt(VK_FORMAT_R8G8B8A8_SRGB)];
    let cfg = pick_config(&c, &formats, &[VK_PRESENT_MODE_FIFO_KHR], e(640, 480)).unwrap();
    assert_eq!(cfg.format, VK_FORMAT_B8G8R8A8_SRGB);
}

/// 同样的输入必须给同样的输出（确定性 —— 配置漂移是「有时对有时错」的根源）。
#[test]
fn pick_config_is_deterministic() {
    let c = caps(3, 0, None, e(1, 1), e(4096, 4096));
    let formats = [fmt(VK_FORMAT_B8G8R8A8_SRGB)];
    let modes = [VK_PRESENT_MODE_FIFO_KHR, VK_PRESENT_MODE_MAILBOX_KHR];
    let a = pick_config(&c, &formats, &modes, e(640, 480)).unwrap();
    for _ in 0..8 {
        assert_eq!(pick_config(&c, &formats, &modes, e(640, 480)).unwrap(), a);
    }
}

/// 没有 sRGB 格式时**如实退回**第一个非 `UNDEFINED` 格式（不假装有 sRGB）。
#[test]
fn pick_config_falls_back_when_no_srgb_format() {
    let c = caps(2, 4, None, e(1, 1), e(4096, 4096));
    let formats = [fmt(VK_FORMAT_R8G8B8A8_UNORM)];
    let cfg = pick_config(&c, &formats, &[VK_PRESENT_MODE_FIFO_KHR], e(640, 480)).unwrap();
    assert_eq!(cfg.format, VK_FORMAT_R8G8B8A8_UNORM);
}

/// 列表只有 `VK_FORMAT_UNDEFINED` ⇒ 规范含义是「任意格式都行」⇒ 用**线性** `B8G8R8A8_UNORM`。
#[test]
fn pick_config_handles_undefined_only_format() {
    let c = caps(2, 4, None, e(1, 1), e(4096, 4096));
    let formats = [fmt(VK_FORMAT_UNDEFINED)];
    let cfg = pick_config(&c, &formats, &[VK_PRESENT_MODE_FIFO_KHR], e(640, 480)).unwrap();
    assert_eq!(cfg.format, VK_FORMAT_B8G8R8A8_UNORM);
}

/// `want` 超出 `maxImageExtent` ⇒ 夹到上限。
#[test]
fn pick_config_clamps_want_to_max_extent() {
    let c = caps(2, 4, None, e(1, 1), e(1920, 1080));
    let cfg = pick_config(
        &c,
        &[fmt(VK_FORMAT_B8G8R8A8_SRGB)],
        &[VK_PRESENT_MODE_FIFO_KHR],
        e(5000, 4000),
    )
    .unwrap();
    assert_eq!(cfg.extent, e(1920, 1080));
}

/// `want` 小于 `minImageExtent` ⇒ 夹到下限（0 会被夹到 1 以上，绝不产生 0×0 交换链）。
#[test]
fn pick_config_clamps_want_to_min_extent() {
    let c = caps(2, 4, None, e(320, 240), e(1920, 1080));
    let cfg = pick_config(
        &c,
        &[fmt(VK_FORMAT_B8G8R8A8_SRGB)],
        &[VK_PRESENT_MODE_FIFO_KHR],
        e(10, 0),
    )
    .unwrap();
    assert_eq!(cfg.extent, e(320, 240));
}

/// `currentExtent` 有值时**必须**用它（Windows 上给别的尺寸会直接 `OUT_OF_DATE`）。
#[test]
fn pick_config_respects_current_extent_over_want() {
    let c = caps(2, 4, Some(e(800, 600)), e(1, 1), e(4096, 4096));
    let cfg = pick_config(
        &c,
        &[fmt(VK_FORMAT_B8G8R8A8_SRGB)],
        &[VK_PRESENT_MODE_FIFO_KHR],
        e(1280, 720),
    )
    .unwrap();
    assert_eq!(cfg.extent, e(800, 600));
}

/// 图像数：`min+1`，封顶 `maxImageCount`（`0` = 无上限）。
#[test]
fn pick_config_image_count_rules() {
    let f = [fmt(VK_FORMAT_B8G8R8A8_SRGB)];
    let m = [VK_PRESENT_MODE_FIFO_KHR];
    // min+1 < max
    let c = caps(2, 8, None, e(1, 1), e(4096, 4096));
    assert_eq!(pick_config(&c, &f, &m, e(64, 64)).unwrap().image_count, 3);
    // min+1 > max ⇒ 夹到 max
    let c = caps(2, 3, None, e(1, 1), e(4096, 4096));
    assert_eq!(pick_config(&c, &f, &m, e(64, 64)).unwrap().image_count, 3);
    // max = 0 ⇒ 无上限
    let c = caps(3, 0, None, e(1, 1), e(4096, 4096));
    assert_eq!(pick_config(&c, &f, &m, e(64, 64)).unwrap().image_count, 4);
    // max == min ⇒ 只能是 min
    let c = caps(1, 1, None, e(1, 1), e(4096, 4096));
    assert_eq!(pick_config(&c, &f, &m, e(64, 64)).unwrap().image_count, 1);
    // 退化：max < min（驱动/窗口异常）也不能 panic，且 ≥ 1
    let c = caps(3, 1, None, e(1, 1), e(4096, 4096));
    assert!(pick_config(&c, &f, &m, e(64, 64)).unwrap().image_count >= 1);
}

/// FIFO 不在列表里（规范保证它一定在，但**不靠这个假设**）⇒ 退回第一个。
#[test]
fn pick_config_falls_back_to_first_present_mode() {
    let c = caps(2, 4, None, e(1, 1), e(4096, 4096));
    let cfg = pick_config(
        &c,
        &[fmt(VK_FORMAT_B8G8R8A8_SRGB)],
        &[VK_PRESENT_MODE_MAILBOX_KHR, VK_PRESENT_MODE_IMMEDIATE_KHR],
        e(640, 480),
    )
    .unwrap();
    assert_eq!(cfg.present_mode, VK_PRESENT_MODE_MAILBOX_KHR);
}

/// 空列表 ⇒ 明确报错（不 panic、不返回一个假配置）。
#[test]
fn pick_config_rejects_empty_inputs() {
    let c = caps(2, 4, None, e(1, 1), e(4096, 4096));
    let err = pick_config(&c, &[], &[VK_PRESENT_MODE_FIFO_KHR], e(64, 64)).unwrap_err();
    assert!(matches!(err, GpuError::Unsupported(_)), "实际 {err:?}");
    let err = pick_config(&c, &[fmt(VK_FORMAT_B8G8R8A8_SRGB)], &[], e(64, 64)).unwrap_err();
    assert!(matches!(err, GpuError::Unsupported(_)), "实际 {err:?}");
}

// ─────────────────────────────────────────────────────────────────────────────
// 2. 纯逻辑：Acquire / Present 返回码映射（本模块最危险的转换）
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn acquire_result_mapping_is_exact() {
    assert_eq!(
        map_acquire_result(ffi::VK_SUCCESS, 7).unwrap(),
        Acquire::Image(7)
    );
    assert_eq!(
        map_acquire_result(ffi::VK_SUBOPTIMAL_KHR, 3).unwrap(),
        Acquire::Suboptimal
    );
    assert_eq!(
        map_acquire_result(ffi::VK_ERROR_OUT_OF_DATE_KHR, 0).unwrap(),
        Acquire::OutOfDate
    );
    // 下面这些**必须是错误**：当成功用会拿未定义的索引去录命令
    for rc in [ffi::VK_TIMEOUT, ffi::VK_NOT_READY, ffi::VK_INCOMPLETE, -4, -1, 12345] {
        match map_acquire_result(rc, 9) {
            Err(GpuError::Driver { code, .. }) => assert_eq!(code, rc, "错误码要原样带出"),
            other => panic!("rc={rc} 必须报错，实际 {other:?}"),
        }
    }
}

#[test]
fn present_result_mapping_is_exact() {
    assert_eq!(map_present_result(ffi::VK_SUCCESS).unwrap(), Present::Presented);
    assert_eq!(
        map_present_result(ffi::VK_SUBOPTIMAL_KHR).unwrap(),
        Present::Suboptimal
    );
    assert_eq!(
        map_present_result(ffi::VK_ERROR_OUT_OF_DATE_KHR).unwrap(),
        Present::OutOfDate
    );
    for rc in [ffi::VK_TIMEOUT, ffi::VK_NOT_READY, -4, ffi::VK_ERROR_SURFACE_LOST_KHR] {
        match map_present_result(rc) {
            Err(GpuError::Driver { code, .. }) => assert_eq!(code, rc),
            other => panic!("rc={rc} 必须报错，实际 {other:?}"),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// 2b. 纯逻辑：回读的通道换序与行 stride（M2b 补强）
// ─────────────────────────────────────────────────────────────────────────────

/// **换序的方向性**：`B8G8R8A8` 内存里是 B,G,R,A ⇒ 必须变成 R,G,B,A。
///
/// 这是最容易搞反的一步（换反了红蓝互换，而「四角都是同一个颜色」的断言
/// 在清屏色 RGB 对称时**照样通过** —— 所以这里用一个通道各不相同的图案钉住方向）。
#[test]
fn reorder_bgra_to_rgba_swaps_red_and_blue() {
    // 像素 0 = B=0x10, G=0x14, R=0x24, A=0xFF   （注意：内存顺序是 B 在最前）
    // 像素 1 = B=0x01, G=0x02, R=0x03, A=0x04
    let bgra = [
        0x10u8, 0x14, 0x24, 0xFF, // BGRA
        0x01, 0x02, 0x03, 0x04,
    ];
    let rgba = reorder_to_rgba8(&bgra, VK_FORMAT_B8G8R8A8_SRGB).expect("换序");
    assert_eq!(
        rgba,
        vec![
            0x24, 0x14, 0x10, 0xFF, // R,G,B,A —— R 与 B 互换，G/A 不动
            0x03, 0x02, 0x01, 0x04,
        ],
        "B8G8R8A8 → RGBA8 必须逐像素交换第 0/2 字节，且不跨像素错位"
    );
    // 线性 BGRA 走同一条路
    assert_eq!(
        reorder_to_rgba8(&bgra, VK_FORMAT_B8G8R8A8_UNORM).expect("换序"),
        rgba
    );
}

/// `R8G8B8A8` 本来就是 RGBA8 ⇒ **原样**（不许再换一次）。
#[test]
fn reorder_rgba_is_identity() {
    let bytes = vec![1u8, 2, 3, 4, 5, 6, 7, 8];
    assert_eq!(
        reorder_to_rgba8(&bytes, VK_FORMAT_R8G8B8A8_SRGB).unwrap(),
        bytes
    );
    assert_eq!(
        reorder_to_rgba8(&bytes, VK_FORMAT_R8G8B8A8_UNORM).unwrap(),
        bytes
    );
}

/// 不认识的格式 / 长度不是 4 的倍数 ⇒ 明确报错（不猜、不近似）。
#[test]
fn reorder_rejects_unknown_format_and_bad_length() {
    assert!(reorder_to_rgba8(&[1, 2, 3, 4], 999).is_err(), "未知格式要报错");
    assert!(reorder_to_rgba8(&[1, 2, 3], VK_FORMAT_B8G8R8A8_SRGB).is_err(), "长度不是 4 的倍数要报错");
    assert!(reorder_to_rgba8(&[], VK_FORMAT_B8G8R8A8_SRGB).unwrap().is_empty());
}

/// 行 stride：`bufferRowLength = 0` ⇒ 紧排 = 宽×4，**没有行尾 padding**。
///
/// padding 算错的症状是「四角正确、中间斜切」——极难一眼看出，所以这里钉死。
#[test]
fn readback_row_pitch_is_tight() {
    assert_eq!(readback_row_pitch(0), 0);
    assert_eq!(readback_row_pitch(1), 4);
    assert_eq!(readback_row_pitch(4), 16);
    assert_eq!(readback_row_pitch(960), 3840);
    // 回读缓冲总大小 = pitch × 高（真机 624×441 的例子）
    assert_eq!(readback_row_pitch(624) as u64 * 441, 624 * 4 * 441);
}

// ─────────────────────────────────────────────────────────────────────────────
// 3. 结构体布局（手写 ABI 的护栏）
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn swapchain_create_info_layout() {
    // 8 + 8 + 4(+4pad) + 8 + 4*3 + 8 + 4*4(+pad) + 8 + 4*4 + 8 = 104
    assert_eq!(std::mem::size_of::<SwapchainCreateInfoKHR>(), 104);
    assert_eq!(std::mem::align_of::<SwapchainCreateInfoKHR>(), 8);
    assert_eq!(std::mem::offset_of!(SwapchainCreateInfoKHR, p_next), 8);
    assert_eq!(std::mem::offset_of!(SwapchainCreateInfoKHR, surface), 24);
    assert_eq!(std::mem::offset_of!(SwapchainCreateInfoKHR, min_image_count), 32);
    assert_eq!(std::mem::offset_of!(SwapchainCreateInfoKHR, image_format), 36);
    assert_eq!(std::mem::offset_of!(SwapchainCreateInfoKHR, image_extent), 44);
    assert_eq!(std::mem::offset_of!(SwapchainCreateInfoKHR, image_array_layers), 52);
    assert_eq!(std::mem::offset_of!(SwapchainCreateInfoKHR, image_usage), 56);
    assert_eq!(std::mem::offset_of!(SwapchainCreateInfoKHR, queue_family_index_count), 64);
    assert_eq!(
        std::mem::offset_of!(SwapchainCreateInfoKHR, p_queue_family_indices),
        72
    );
    assert_eq!(std::mem::offset_of!(SwapchainCreateInfoKHR, pre_transform), 80);
    assert_eq!(std::mem::offset_of!(SwapchainCreateInfoKHR, composite_alpha), 84);
    assert_eq!(std::mem::offset_of!(SwapchainCreateInfoKHR, present_mode), 88);
    assert_eq!(std::mem::offset_of!(SwapchainCreateInfoKHR, clipped), 92);
    assert_eq!(std::mem::offset_of!(SwapchainCreateInfoKHR, old_swapchain), 96);
}

#[test]
fn present_info_layout() {
    // 8 + 8 + 4(+4pad) + 8 + 4(+4pad) + 8 + 8 + 8 = 64
    assert_eq!(std::mem::size_of::<PresentInfoKHR>(), 64);
    assert_eq!(std::mem::offset_of!(PresentInfoKHR, p_next), 8);
    assert_eq!(std::mem::offset_of!(PresentInfoKHR, wait_semaphore_count), 16);
    assert_eq!(std::mem::offset_of!(PresentInfoKHR, p_wait_semaphores), 24);
    assert_eq!(std::mem::offset_of!(PresentInfoKHR, swapchain_count), 32);
    assert_eq!(std::mem::offset_of!(PresentInfoKHR, p_swapchains), 40);
    assert_eq!(std::mem::offset_of!(PresentInfoKHR, p_image_indices), 48);
    assert_eq!(std::mem::offset_of!(PresentInfoKHR, p_results), 56);
}

#[test]
fn surface_struct_layouts() {
    assert_eq!(std::mem::size_of::<SurfaceCapabilitiesKHR>(), 52);
    assert_eq!(std::mem::offset_of!(SurfaceCapabilitiesKHR, current_extent), 8);
    assert_eq!(std::mem::offset_of!(SurfaceCapabilitiesKHR, max_image_extent), 24);
    assert_eq!(std::mem::offset_of!(SurfaceCapabilitiesKHR, supported_usage_flags), 48);
    assert_eq!(std::mem::size_of::<SurfaceFormatKHR>(), 8);
    // 信号量创建信息（帧同步对象）
    assert_eq!(std::mem::size_of::<vk::SemaphoreCreateInfo>(), 24);
}

#[test]
fn platform_extension_names_are_exact() {
    assert_eq!(surface::SURFACE_EXTENSION, "VK_KHR_surface");
    assert_eq!(
        Surface::platform_extension(Platform::Windows),
        "VK_KHR_win32_surface"
    );
}

/// 冻死 API 把句柄类型写作 `crate::ffi::vk::*` —— 这条路径必须逐字可解析
/// （领队的 HAL 接线按冻死签名写；这里用类型别名断言把它钉住）。
#[test]
fn frozen_api_handle_paths_resolve() {
    let _: Option<deer_vk::ffi::vk::SurfaceHandle> = None;
    let _: Option<deer_vk::ffi::vk::ImageHandle> = None;
    let _: Option<deer_vk::ffi::vk::ImageViewHandle> = None;
    let _: Option<deer_vk::ffi::vk::QueueHandle> = None;
    let _: Option<deer_vk::ffi::vk::SemaphoreHandle> = None;
    let _: Option<deer_vk::ffi::vk::PhysicalDeviceHandle> = None;
    let _: Option<deer_vk::ffi::vk::InstanceHandle> = None;
    let _: Option<deer_vk::ffi::SurfaceHandle> = None;
}

// ─────────────────────────────────────────────────────────────────────────────
// 4. 真机：扩展可用性 + 实例扩展启用（不建窗口，默认也跑）
// ─────────────────────────────────────────────────────────────────────────────

/// `extension_available` 必须是**真查**（`vkEnumerateInstanceExtensionProperties`）：
/// 真扩展为真、假扩展为假。这条挡住「靠猜/硬编码 true」的实现。
#[test]
fn extension_availability_is_queried_not_guessed() {
    if !ffi::Instance::extension_available(surface::SURFACE_EXTENSION) {
        eprintln!(
            "跳过：本机 Vulkan 未提供 {}（没有 loader/ICD 或没有显卡）",
            surface::SURFACE_EXTENSION
        );
        return;
    }
    assert!(
        !ffi::Instance::extension_available("VK_KHR_deer_gui_not_a_real_extension"),
        "不存在的扩展必须查成 false"
    );
    println!(
        "真机扩展可用性：{} = true，假扩展 = false ✅",
        surface::SURFACE_EXTENSION
    );
}

/// `create_with_extensions` 必须真的启用扩展（`enabled_extensions` 如实记录），
/// 且对不可用扩展**明确报错**（不静默忽略）。
#[test]
fn instance_enables_requested_extensions_and_rejects_missing() {
    if !ffi::Instance::extension_available(surface::SURFACE_EXTENSION) {
        eprintln!("跳过：本机没有 Vulkan（或没有 WSI 扩展）");
        return;
    }
    // ① 不请求任何扩展 ⇒ 与旧行为一致（既有 46 条测试依赖这条路径）
    let plain = ffi::Instance::create().expect("创建基础实例");
    assert!(
        plain.enabled_extensions().is_empty(),
        "没请求扩展就不该有 enabled_extensions：{:?}",
        plain.enabled_extensions()
    );
    drop(plain);

    // ② 请求 surface 扩展 ⇒ 真的启用
    let inst = ffi::Instance::create_with_extensions(false, &[surface::SURFACE_EXTENSION])
        .expect("创建带 surface 扩展的实例");
    assert_eq!(inst.enabled_extensions(), &[surface::SURFACE_EXTENSION.to_string()]);
    println!("实例扩展已启用：{:?}", inst.enabled_extensions());
    drop(inst);

    // ③ 请求不存在的扩展 ⇒ Unsupported 且信息里点名
    let name = "VK_KHR_deer_gui_not_a_real_extension";
    match ffi::Instance::create_with_extensions(false, &[name]) {
        Err(GpuError::Unsupported(msg)) => {
            assert!(msg.contains(name), "错误信息要点名缺失的扩展：{msg}")
        }
        other => panic!("期望 Unsupported，实际 {:?}", other.map(|i| i.enabled_extensions().to_vec())),
    }
}

/// Windows 上 `VK_KHR_win32_surface` 必须可用（否则窗口路径无法工作）。
/// 非 Windows 明确跳过并说明。
#[test]
fn win32_surface_extension_is_available_on_windows() {
    if !ffi::Instance::extension_available(surface::SURFACE_EXTENSION) {
        eprintln!("跳过：本机没有 Vulkan");
        return;
    }
    if cfg!(windows) {
        assert!(
            ffi::Instance::extension_available(surface::WIN32_SURFACE_EXTENSION),
            "Windows 上必须能查到 VK_KHR_win32_surface（否则无法建 surface）"
        );
    } else {
        eprintln!("跳过：非 Windows 平台不检查 win32 surface 扩展");
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// 5. 真窗口端到端（默认跳过）
// ─────────────────────────────────────────────────────────────────────────────

/// 门槛判定：`DEER_VK_WINDOW_TESTS` 有没有（真的）打开 —— 本文件**两层里的第二层**都用它。
///
/// **本文件只此一处判据**（曾经是两处内联副本）：判据分叉过一次就会出现
/// 「同一个命令，一个用例认为设了门槛、另一个认为没设」，而这个方向恰好是**静默**的。
///
/// **为什么不能写严格 `== "1"`**（实测，不是猜的）：`cmd` 的
/// `set DEER_VK_WINDOW_TESTS=1 && cargo test …` 会把 `&&` 前的空格也算进变量值 ——
/// `cmd /c "set X=1 && set X"` 实测打印 `X=1 `（**带一个尾空格**），于是
/// `windowed_chain_end_to_end` 会**早退跳过**（0.00s）却仍然报 ok；写成
/// `set "X=1" && …` 或 `set X=1&& …` 才是不带空格的 `1`。
/// 判定委托给 [`ffi::env_flag`]（先 `trim()` 再比；断言在 `src/ffi.rs` 的单测里）。
fn window_tests_enabled() -> bool {
    ffi::env_flag("DEER_VK_WINDOW_TESTS")
}

/// **默认跳过**：需要真实窗口/桌面（`DEER_VK_WINDOW_TESTS=1` 才跑）。
///
/// 跑的是完整链：原生 Win32 窗口 → `VkSurfaceKHR` → 设备（图形 + 呈现队列，设备扩展
/// `VK_KHR_swapchain`）→ 交换链 → 逐帧 `acquire`/画/`submit`/`present` → `resize` → `Drop`。
#[test]
fn windowed_chain_end_to_end() {
    if !window_tests_enabled() {
        eprintln!("跳过：需要真实窗口（设 DEER_VK_WINDOW_TESTS=1 启用）");
        return;
    }
    #[cfg(windows)]
    {
        run_windowed_e2e();
    }
    #[cfg(not(windows))]
    {
        eprintln!("跳过：真窗口端到端验证目前只在 Windows 上实现（surface.rs 也只实现了 Windows）");
    }
}

#[cfg(windows)]
fn run_windowed_e2e() {
    use std::time::Instant;

    // 本机没有 Vulkan / 没有 WSI 扩展 ⇒ 如实跳过（不伪装通过）
    if !ffi::Instance::extension_available(surface::SURFACE_EXTENSION)
        || !ffi::Instance::extension_available(surface::WIN32_SURFACE_EXTENSION)
    {
        eprintln!("跳过：本机 Vulkan 不提供 VK_KHR_surface / VK_KHR_win32_surface");
        return;
    }

    // 数值门槛也要先 `trim()`：`cmd /c "set DEER_WINDOW_ADAPTER=1 && …"` 的值是 `"1 "`，
    // 不带 `trim` 时 `parse()` 失败 ⇒ **静默退回适配器 0**（以为在测指定 GPU、实际测的是另一块）。
    let adapter: usize = std::env::var("DEER_WINDOW_ADAPTER")
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(0);

    let win = Win32TestWindow::create("deer-vk-swapchain-smoke", 640, 480)
        .expect("建 Win32 测试窗口失败");
    let raw = RawWindowHandle {
        platform: Platform::Windows,
        handle: win.hwnd as usize,
        display: win.hinstance as usize,
    };

    const CLEAR: Color = Color::rgb(CLEAR_R, CLEAR_G, CLEAR_B);
    let want = Extent {
        width: 640,
        height: 480,
    };

    let mut r = match WindowedRenderer::new(adapter, raw, want, CLEAR) {
        Ok(r) => r,
        Err(GpuError::Unsupported(msg)) => {
            // 例如「该设备没有同时支持图形与呈现的队列族」⇒ 是环境问题，跳过并说明
            eprintln!("跳过：本机这个适配器无法在此窗口上呈现（{msg}）");
            return;
        }
        Err(e) => panic!("创建窗口渲染器失败（adapter={adapter}）：{e}"),
    };

    println!("适配器          : {}", r.adapter().name);
    println!("实例扩展        : {:?}", r.instance_extensions());
    println!(
        "交换链          : {}×{} / format {:#x} / present mode {} / {} 张图",
        r.extent().width,
        r.extent().height,
        r.format(),
        r.present_mode(),
        r.image_count()
    );

    // —— 真机值断言 ——
    assert!(r.extent().width > 0 && r.extent().height > 0, "交换链尺寸必须 > 0");
    // **线性优先**（M3c 裁决，见 `swapchain::pick_config` 的文档）：sRGB 附件连混合都在线性
    // 空间，而 CPU 基准按字节混合 ⇒ 半透明差几十字节 ⇒ 窗口上屏 parity 只能在**线性**附件上成立。
    // 这里只断言「格式**要么是线性的、要么是那两种 sRGB 退路**」：
    // 前者是正常路径，后者是「本机 surface 只有 sRGB」时的如实退回（**退路不许被删掉**）。
    let fmt = r.format();
    let linear = fmt == VK_FORMAT_B8G8R8A8_UNORM || fmt == VK_FORMAT_R8G8B8A8_UNORM;
    let srgb_fallback = fmt == VK_FORMAT_B8G8R8A8_SRGB || fmt == VK_FORMAT_R8G8B8A8_SRGB;
    assert!(
        linear || srgb_fallback,
        "交换链格式必须是线性 *_UNORM（首选）或 sRGB（退路），实际 {fmt:#x} \
         —— 两者都不是说明 `pick_config` 选错了；若本机能力列表确实只有别的格式，\
         请连能力列表一起报告"
    );
    if !linear {
        eprintln!(
            "⚠️ 本机只给到 sRGB 格式（{fmt:#x}）⇒ 走退路：**半透明像素与 CPU 不会逐字节一致**\
             （sRGB 附件的混合在线性空间），`window_parity` 会直接失败并说明原因"
        );
    }
    println!(
        "颜色附件        : {fmt:#010x}（{}）",
        if linear { "线性 UNORM（首选）" } else { "sRGB（退路）" }
    );
    assert_eq!(
        r.present_mode(),
        VK_PRESENT_MODE_FIFO_KHR,
        "present mode 必须是 FIFO（规范保证任何实现都支持）"
    );
    assert!(r.image_count() >= 2, "图像数应当 ≥ minImageCount+1，实际 {}", r.image_count());
    // 开的校验层会额外启用 VK_EXT_debug_utils ⇒ 只断言「两个 surface 扩展都在」，
    // 不断言总数（曾经写死 == 2，开校验层时就假红）。
    let exts = r.instance_extensions();
    assert!(
        exts.iter().any(|e| e == surface::SURFACE_EXTENSION),
        "窗口路径必须启用 {}，实际 {exts:?}",
        surface::SURFACE_EXTENSION
    );
    assert!(
        exts.iter().any(|e| e == surface::WIN32_SURFACE_EXTENSION),
        "窗口路径必须启用 {}，实际 {exts:?}",
        surface::WIN32_SURFACE_EXTENSION
    );

    // —— 连续呈现 ——
    let target = 30u64;
    let mut presented = 0u64;
    let mut out_of_date = 0u64;
    let started = Instant::now();
    for _ in 0..target {
        match r.render_and_present().expect("呈现一帧") {
            FrameOutcome::Presented => presented += 1,
            FrameOutcome::OutOfDate => {
                out_of_date += 1;
                // 正常路径：重建后重试（**绝不能当成功**）
                let cur = r.extent();
                r.resize(cur).expect("过期后重建交换链");
            }
        }
    }
    let elapsed = started.elapsed().as_secs_f64();
    println!(
        "呈现 {presented} 帧 / 过期 {out_of_date} 次 / 耗时 {elapsed:.2}s（{:.1} 帧/秒）",
        presented as f64 / elapsed.max(1e-9)
    );
    assert!(presented > 0, "至少要成功呈现一帧");
    assert_eq!(
        r.frames_presented(),
        presented + r.suboptimal_frames(),
        "frames_presented 必须等于「真的调过 present 的帧数」"
    );

    // ─────────────────────────────────────────────────────────────────────────
    // 呈现帧的**像素级**回读（M2b 补强）：把「呈现了 N 帧」升级成
    // 「呈现出去的那张图确实是我们要的颜色」。
    // ─────────────────────────────────────────────────────────────────────────
    assert!(
        r.readback_enabled() && r.readback_available(),
        "默认应当开着回读（`set_readback_enabled(true)` 是默认值）"
    );
    // 清屏色是显式常量（与示例同一个值）：R=0x10, G=0x14, B=0x24（**写入值**）
    //
    // 回读期望值**按实际附件格式分支**（紧跟其后的代码就是那个分支）：
    // - **线性** `*_UNORM`（M3c 起首选，本机实测 `0x2c`）⇒ **直通** `[16, 20, 36]`；
    // - **sRGB**（退路）⇒ 驱动做 sRGB 编码 ⇒ `[srgb(16), srgb(20), srgb(36)] = [71, 79, 105]`。
    // 第一版把「sRGB 编码」写死成唯一期望值，线性优先落地后就变成了「拿 sRGB 卡线性」的假红 ——
    // 所以现在两种都写出来、由格式决定用哪个（`srgb_encoded_byte` 本身有单测）。
    let clear_linear = [CLEAR_R, CLEAR_G, CLEAR_B, 0xFF];
    // **线性附件直通、sRGB 附件编码**（M3c 的裁决：线性优先）——
    // 期望值必须跟着**实际格式**走，否则这条判据会变成「拿 sRGB 的期望值去卡线性附件」。
    let expected_clear = if linear {
        [CLEAR_R, CLEAR_G, CLEAR_B, 0xFF]
    } else {
        [
            deer_vk::windowed::srgb_encoded_byte(CLEAR_R),
            deer_vk::windowed::srgb_encoded_byte(CLEAR_G),
            deer_vk::windowed::srgb_encoded_byte(CLEAR_B),
            0xFF,
        ]
    };
    let pixels = r.read_back_last_frame().expect("回读最后呈现的那一帧");
    let (rw, rh) = (r.extent().width, r.extent().height);
    assert_eq!(
        pixels.len(),
        (rw as usize) * (rh as usize) * 4,
        "回读长度必须是 宽×高×4（RGBA8）"
    );
    let at = |x: u32, y: u32| -> [u8; 4] {
        let i = ((y as usize) * (rw as usize) + (x as usize)) * 4;
        [pixels[i], pixels[i + 1], pixels[i + 2], pixels[i + 3]]
    };
    println!("回读 {rw}×{rh}（RGBA8，{} 字节）", pixels.len());
    println!(
        "  清屏色（写入值）= {clear_linear:?} ⇒ {} 附件实测应为 {expected_clear:?}（{}）",
        if linear { "线性" } else { "sRGB" },
        if linear {
            "直通，不编码"
        } else {
            "sRGB 编码，非直通"
        }
    );
    for (name, x, y) in [
        ("左上", 0u32, 0u32),
        ("右上", rw - 1, 0),
        ("左下", 0, rh - 1),
        ("右下", rw - 1, rh - 1),
    ] {
        let c = at(x, y);
        println!("  {name} ({x},{y}) = {c:?}");
        assert_eq!(
            c, expected_clear,
            "{name} ({x},{y}) 必须与「清屏色在该附件格式下的期望值」逐字节相同\
             （线性直通 / sRGB 编码，取决于实际格式）"
        );
    }
    // R/B 换序的**方向性**证据：三个通道值互不相同且线性序是 16 < 20 < 36，
    // 所以（无论是否编码）必须严格递增（直通 16<20<36；编码后 71<79<105）。
    // 若把 BGRA 当 RGBA 用（或反过来），这里会变成递减 —— 「红蓝互换」一定会现形。
    let corner = at(0, 0);
    assert!(
        corner[0] < corner[1] && corner[1] < corner[2],
        "四角通道必须按 R<G<B 递增（B8G8R8A8 → RGBA8 换序反了就会递减）：{corner:?}"
    );
    let center = at(rw / 2, rh / 2);
    println!(
        "  中心 ({},{}) = {center:?}（在三角形内 ⇒ 必须不同于清屏色）",
        rw / 2,
        rh / 2
    );
    assert_ne!(
        center, expected_clear,
        "窗口中间必须画了东西（这是「几何真的进到呈现帧里」的判据）"
    );
    // 三角形覆盖的中心区域里应当能找到不止一种非清屏色（抗锯齿/边界另说，这里只要求「有异色」）
    let differing = pixels
        .chunks_exact(4)
        .filter(|p| p[..3] != expected_clear[..3])
        .count();
    println!("  与清屏色不同的像素：{differing} 个（{:.2}% 画面）", differing as f64 * 100.0 / (rw as f64 * rh as f64));
    assert!(differing > 0, "整幅图全是清屏色 ⇒ 几何没画进去");

    // 关掉回读 ⇒ 必须**明确报错**，不许拿上一帧的数据冒充
    r.set_readback_enabled(false).expect("关闭回读");
    r.render_and_present().expect("关掉回读后再画一帧");
    match r.read_back_last_frame() {
        Err(GpuError::Unsupported(msg)) => {
            println!("回读关闭后按预期报错：{msg}");
        }
        other => panic!("回读关闭后必须报错，实际 {:?}", other.map(|v| v.len())),
    }
    // 重新打开 ⇒ 又能回读（并且值仍然正确）
    r.set_readback_enabled(true).expect("重新打开回读");
    r.render_and_present().expect("打开回读后画一帧");
    let again = r.read_back_last_frame().expect("重新回读");
    assert_eq!(again.len(), pixels.len());
    assert_eq!(
        [again[0], again[1], again[2], again[3]],
        expected_clear,
        "重新打开回读后左上角仍应是清屏色（线性直通 / sRGB 编码，取决于附件格式）"
    );

    // —— resize：先真的改变窗口尺寸，再让渲染器重建交换链 ——
    let before = r.extent();
    win.resize(800, 600);
    pump_messages();
    r.resize(Extent {
        width: 800,
        height: 600,
    })
    .expect("resize 重建交换链");
    let after = r.extent();
    let client = win.client_size();
    println!(
        "resize 后交换链 : {}×{}（之前 {}×{}）；窗口客户区 {}×{}",
        after.width, after.height, before.width, before.height, client.width, client.height
    );
    assert!(after.width > 0 && after.height > 0);
    assert_ne!(after, before, "窗口尺寸变了，交换链尺寸应当跟着变");
    for _ in 0..5 {
        match r.render_and_present().expect("resize 后呈现") {
            // 统计以 `frames_presented()` 为准（它就是「真的调过 present 的次数」）
            FrameOutcome::Presented => {}
            FrameOutcome::OutOfDate => {
                out_of_date += 1;
                let cur = r.extent();
                r.resize(cur).expect("再次重建");
            }
        }
    }

    // —— OutOfDate 的观测（**不强制**：驱动是否在尺寸变化后立刻报 OUT_OF_DATE 有实现差异）——
    // 这里只**观测并打印**，然后断言「过期之后重建能恢复正常」这条不变量。
    win.resize(900, 520);
    pump_messages();
    let mut observed_out_of_date = false;
    for i in 0..30 {
        match r.render_and_present().expect("观测过期") {
            FrameOutcome::Presented => {}
            FrameOutcome::OutOfDate => {
                observed_out_of_date = true;
                out_of_date += 1;
                println!("第 {i} 帧观测到 OutOfDate（尺寸变化后交换链过期）✅");
                let cur = r.extent();
                r.resize(cur).expect("过期后重建");
                break;
            }
        }
    }
    if !observed_out_of_date {
        println!(
            "注意：窗口尺寸变化后 30 帧内没观测到 OutOfDate（驱动可能直接返回 SUBOPTIMAL 或自动适配）\
             —— 映射正确性由 `acquire_result_mapping_is_exact` / `present_result_mapping_is_exact` 单测钉住"
        );
    }
    // 无论是否观测到过期，重建之后必须能继续呈现
    let recovered = (0..5).any(|_| matches!(r.render_and_present(), Ok(FrameOutcome::Presented)));
    assert!(recovered, "resize 之后必须能恢复正常呈现");

    // —— 回读的开销实测（诚实边界：它每帧多一次全屏 copy + 两次 barrier）——
    let bench = |r: &mut WindowedRenderer, frames: u32| -> f64 {
        let t = Instant::now();
        for _ in 0..frames {
            let _ = r.render_and_present().expect("基准呈现");
        }
        // 等最后一帧真的做完再计时收尾（否则测的是「提交多快」而不是「帧多快」）
        r.wait_idle().expect("基准 wait_idle");
        frames as f64 / t.elapsed().as_secs_f64().max(1e-9)
    };
    let with = bench(&mut r, 20);
    r.set_readback_enabled(false).expect("关回读");
    let without = bench(&mut r, 20);
    r.set_readback_enabled(true).expect("开回读");
    println!(
        "回读开销实测（各 20 帧，{}×{}）：开 {with:.1} 帧/秒 / 关 {without:.1} 帧/秒 ⇒ 差 {:.1}%",
        r.extent().width,
        r.extent().height,
        (without - with) / without * 100.0
    );

    // ─────────────────────────────────────────────────────────────────────────
    // 窗口侧 host→vertex 屏障：**从「只有计数」升级成「有断言」**
    //
    // ## 为什么必须补在这里（补错地方这条判据就是死的）
    //
    // 上面跑了 60+ 帧的 `render_and_present`，走的是 **M2b 三角形路径**（`record`），
    // 它**根本不碰 UI 顶点缓冲** ⇒ 在那条链上读 `ui_host_to_vertex_barrier_count()`
    // 恒为 0 —— 一条永远绿、也永远抓不到任何东西的判据。真正发这条屏障的是
    // `record_ui`，只有 `draw_and_present` 会走到 ⇒ 断言必须挂在 UI 路径上。
    //
    // ## 判据的三条腿（三条都要，缺一条就被绕过）
    //
    // 1. **首帧必须 +1**：这一帧真的上传了顶点 ⇒ 必须发一条 host→vertex 屏障。
    //    只发不发是「同步不对」，在同一队列同一提交里颜色几乎必然对 ⇒ 会长期潜伏。
    // 2. **同语料第二帧不得增加**：内容逐字节相同 ⇒ 跳过重传（B3）⇒ 也不该发屏障。
    //    这条挡住「每帧无脑发屏障」这种看似无害的退化。
    // 3. **空帧不得增加**：没有顶点 ⇒ 既不上传也不发屏障。
    //
    // ## 前置断言（为什么必须有）
    //
    // 「计数没增长」有两种解释：① 正确地没发；② **根本没上传**（那这条判据就白测了）。
    // 所以首帧必须**同时**断言 `buffer_uploads` 增长 —— 那是「确实有东西要同步」的证据。
    //
    // ## 变异验证（**已实测**，两条都确定性变红）
    //
    // - 删掉 `windowed.rs` 里 `self.ui_host_to_vertex_barriers += 1;`（**保留**屏障发射）
    //   ⇒ 第 1 条红（`b1 == b0`）—— 证明计数的自增与「真的发了」这两件事被分开守着；
    // - 删掉整个 `if self.ui_barrier { … }` 块（连发射带计数一起删）
    //   ⇒ 第 1 条红 —— 这条是原始的「窗口路径从不发屏障」缺陷形态。
    //
    // 边界（诚实说明）：本判据证明「这段代码被执行了、且时机符合 B3 语义」；
    // **屏障参数的确切位**由 `ui_barrier_constants_pin_the_exact_bits` 钉，
    // 「驱动真的按语义用了它」属于真机逐像素对照的范畴，不在本条射程内。
    // ─────────────────────────────────────────────────────────────────────────
    {
        use deer_gpu::{DrawCmd, DrawList, RectI};

        let mut list = DrawList::new();
        list.push(DrawCmd::FillRect {
            rect: RectI::new(20, 20, 120, 60),
            color: Color::rgb(200, 30, 40),
        });
        assert!(
            !list.is_empty() && list.clip_balanced(),
            "前置：本段判据要用一份非空且裁剪平衡的绘制列表"
        );

        // 三类屏障一起采样：**顶点 / 索引 / 间接**（顺序固定，错误信息里按这个序读）。
        let snap = |r: &WindowedRenderer| -> (u64, u64, u64) {
            (
                r.ui_host_to_vertex_barrier_count(),
                r.ui_index_barrier_count(),
                r.ui_indirect_barrier_count(),
            )
        };

        let s0 = snap(&r);
        let up0 = r.render_stats().buffer_uploads;

        // ① 首帧：顶点 / 索引 / 间接 三块缓冲都是第一次上传 ⇒ 三类屏障**各**发一条
        assert!(
            matches!(r.draw_and_present(&list, None), Ok(FrameOutcome::Presented)),
            "前置：UI 首帧必须真的呈现出去（否则后面读到的计数没有意义）"
        );
        let s1 = snap(&r);
        let up1 = r.render_stats().buffer_uploads;
        assert!(
            up1 > up0,
            "前置不成立：首帧没有上传任何缓冲（buffer_uploads {} → {}）⇒ \
             「屏障计数 +1」无从谈起，本段判据会被静默架空",
            up0,
            up1
        );
        assert_eq!(
            s1,
            (s0.0 + 1, s0.1 + 1, s0.2 + 1),
            "UI 首帧上传了顶点/索引/间接 ⇒ 三类 host→缓冲屏障**各**必须发一条\
             （顶点/索引/间接 计数 {s0:?} → {s1:?}）；\
             不发就是「主机写完 GPU 直接取数」的原始缺陷形态"
        );

        // ② 同语料第二帧：跳过重传 ⇒ 三类都不得再发屏障
        assert!(
            matches!(r.draw_and_present(&list, None), Ok(FrameOutcome::Presented)),
            "前置：同语料第二帧也要呈现成功"
        );
        let s2 = snap(&r);
        assert_eq!(
            s2, s1,
            "语料逐字节相同 ⇒ 跳过重传 ⇒ 三类屏障都不该再发\
             （顶点/索引/间接 计数 {s1:?} → {s2:?}）；每帧无脑发屏障属于 B3 语义的退化"
        );

        // ③ 空帧：没有顶点 ⇒ 三类都不得发屏障
        let empty = DrawList::new();
        assert!(empty.is_empty(), "前置：空列表必须真的是空的");
        assert!(
            matches!(r.draw_and_present(&empty, None), Ok(FrameOutcome::Presented)),
            "前置：空帧也要呈现成功"
        );
        let s3 = snap(&r);
        assert_eq!(
            s3, s2,
            "空帧没有上传 ⇒ 三类屏障都不得发（顶点/索引/间接 计数 {s2:?} → {s3:?}）"
        );

        println!(
            "窗口侧 host→缓冲屏障（顶点/索引/间接）：初始 {s0:?} → 首帧 {s1:?}（各 +1 ✅）\
             → 同语料 {s2:?}（不增 ✅）→ 空帧 {s3:?}（不增 ✅）；\
             首帧 buffer_uploads {up0} → {up1}（前置成立）"
        );
    }

    // ── T1.3 ②：窗口侧贴任意 RGBA 纹理（走**纹理管线** ⇒ RGBA 调制） ──────────
    //
    // 判据与附件格式无关，且**能区分两种语义**：1×1 texel = (255,0,0,255)、tint = 白。
    //   - RGB 调制（本实现）⇒ out = tint × texel = (255,0,0) ⇒ 绿/蓝必须为 **0**；
    //   - 覆盖率语义（界面管线那一支）⇒ out = tint = (255,255,255) ⇒ 绿/蓝为 255。
    // `255` 与 `0` 在 sRGB 编码下都是不动点 ⇒ 线性 / sRGB 附件都适用。
    // 用 1×1 纹理 ⇒ uv 取到哪儿都是同一个 texel，不引入采样歧义。
    //
    // ⚠️ **已知边界（本文件末尾的实测记录）**：纹理趟**之后**若再走一次普通
    //    `render_and_present()`，本机在该组合下 `0xc0000005`。因此这里**只做**
    //    「画一趟 + 回读一次」，不做「画纹理 → 再画界面 → 再回读」。
    //    单跑本段（画 + 回读）在校验层下**零错误零泄漏**。
    {
        let red = r
            .device()
            .create_texture_rgba8(1, 1, &[255, 0, 0, 255])
            .expect("建 1×1 红纹理");
        let full = deer_gpu::RectI::new(0, 0, rw as i32, rh as i32);
        let outcome = r
            .draw_textured_quad(&red, full, deer_gpu::Color::WHITE)
            .expect("窗口侧纹理 quad");
        assert!(
            matches!(outcome, FrameOutcome::Presented),
            "纹理 quad 那趟应成功呈现，实际 {outcome:?}"
        );
        let px = r.read_back_last_frame().expect("回读纹理那一帧");
        let i0 = (((rh / 2) as usize) * (rw as usize) + (rw / 2) as usize) * 4;
        let c = [px[i0], px[i0 + 1], px[i0 + 2], px[i0 + 3]];
        println!("窗口侧纹理 quad 中心 = {c:?}");
        assert_eq!(c[0], 255, "红通道应为 255（白 tint × texel R=255）：{c:?}");
        assert_eq!(
            c[1], 0,
            "绿通道必须为 0 ⇒ 证明窗口侧走的是 **RGB 调制**；若这里是 255，\
             说明它误用了界面管线的覆盖率语义（那时输出恒等于 tint）：{c:?}"
        );
        assert_eq!(c[2], 0, "蓝通道同理必须为 0：{c:?}");
        println!("窗口侧纹理 quad ✅ RGBA 调制生效（纹理管线）");
    }

    // —— 收尾：wait_idle + Drop（析构顺序在 Drop 里靠字段顺序保证）——
    r.wait_idle().expect("vkDeviceWaitIdle");
    let total = r.frames_presented();
    drop(r);
    println!(
        "收尾完成 ✅ 累计呈现 {total} 帧（含 resize 前后），OutOfDate 观测 {out_of_date} 次；\
         渲染器已按依赖倒序销毁（无校验层错误即无泄漏/无顺序错误）"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Win32 测试窗口（**只用 user32/kernel32**，不引 winit）
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(windows)]
mod win32_window {
    use std::ffi::c_void;

    pub const WS_OVERLAPPEDWINDOW: u32 = 0x00CF_0000;
    pub const WS_VISIBLE: u32 = 0x1000_0000;
    pub const SW_SHOW: i32 = 5;
    pub const SWP_NOZORDER: u32 = 0x0004;
    pub const SWP_NOACTIVATE: u32 = 0x0010;
    pub const PM_REMOVE: u32 = 0x0001;

    #[repr(C)]
    pub struct WndClassW {
        pub style: u32,
        pub lpfn_wnd_proc: Option<unsafe extern "system" fn(*mut c_void, u32, usize, isize) -> isize>,
        pub cb_cls_extra: i32,
        pub cb_wnd_extra: i32,
        pub h_instance: *mut c_void,
        pub h_icon: *mut c_void,
        pub h_cursor: *mut c_void,
        pub hbr_background: *mut c_void,
        pub lpsz_menu_name: *const u16,
        pub lpsz_class_name: *const u16,
    }

    /// `MSG`（x64 布局：48 字节）
    #[repr(C)]
    pub struct Msg {
        pub hwnd: *mut c_void,
        pub message: u32,
        pub w_param: usize,
        pub l_param: isize,
        pub time: u32,
        pub pt_x: i32,
        pub pt_y: i32,
        pub l_private: u32,
    }

    #[link(name = "user32")]
    unsafe extern "system" {
        pub fn RegisterClassW(cls: *const WndClassW) -> u16;
        pub fn CreateWindowExW(
            ex_style: u32,
            class_name: *const u16,
            window_name: *const u16,
            style: u32,
            x: i32,
            y: i32,
            width: i32,
            height: i32,
            parent: *mut c_void,
            menu: *mut c_void,
            instance: *mut c_void,
            param: *mut c_void,
        ) -> *mut c_void;
        pub fn DestroyWindow(hwnd: *mut c_void) -> i32;
        pub fn ShowWindow(hwnd: *mut c_void, cmd: i32) -> i32;
        pub fn UpdateWindow(hwnd: *mut c_void) -> i32;
        pub fn DefWindowProcW(hwnd: *mut c_void, msg: u32, wp: usize, lp: isize) -> isize;
        pub fn SetWindowPos(
            hwnd: *mut c_void,
            after: *mut c_void,
            x: i32,
            y: i32,
            cx: i32,
            cy: i32,
            flags: u32,
        ) -> i32;
        pub fn GetClientRect(hwnd: *mut c_void, rect: *mut Rect) -> i32;
        pub fn PeekMessageW(
            msg: *mut Msg,
            hwnd: *mut c_void,
            filter_min: u32,
            filter_max: u32,
            remove: u32,
        ) -> i32;
        pub fn TranslateMessage(msg: *const Msg) -> i32;
        pub fn DispatchMessageW(msg: *const Msg) -> isize;
    }

    #[repr(C)]
    #[derive(Default, Clone, Copy)]
    pub struct Rect {
        pub left: i32,
        pub top: i32,
        pub right: i32,
        pub bottom: i32,
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        pub fn GetModuleHandleW(name: *const u16) -> *mut c_void;
    }

    /// 极简窗口过程：默认处理（不拦截任何消息）。
    pub unsafe extern "system" fn wnd_proc(
        hwnd: *mut c_void,
        msg: u32,
        wp: usize,
        lp: isize,
    ) -> isize {
        // SAFETY: 窗口过程由 Win32 调用，参数由系统保证有效。
        unsafe { DefWindowProcW(hwnd, msg, wp, lp) }
    }
}

#[cfg(windows)]
struct Win32TestWindow {
    hwnd: *mut std::ffi::c_void,
    hinstance: *mut std::ffi::c_void,
}

#[cfg(windows)]
impl Win32TestWindow {
    fn create(class: &str, width: i32, height: i32) -> Result<Win32TestWindow, String> {
        use std::sync::Once;
        use win32_window::*;

        fn wide(s: &str) -> Vec<u16> {
            s.encode_utf16().chain(std::iter::once(0)).collect()
        }

        // 类只需注册一次（多个测试/多次调用会重复注册）
        static REGISTER: Once = Once::new();
        let class_w = wide(class);
        let module = unsafe {
            // SAFETY: 传 null 表示取本进程可执行模块。
            GetModuleHandleW(std::ptr::null())
        };
        let mut err: Option<String> = None;
        REGISTER.call_once(|| {
            let wc = WndClassW {
                style: 0,
                lpfn_wnd_proc: Some(wnd_proc),
                cb_cls_extra: 0,
                cb_wnd_extra: 0,
                h_instance: module,
                h_icon: std::ptr::null_mut(),
                h_cursor: std::ptr::null_mut(),
                hbr_background: std::ptr::null_mut(),
                lpsz_menu_name: std::ptr::null(),
                lpsz_class_name: class_w.as_ptr(),
            };
            // SAFETY: `wc` 在调用期间存活；类名以 NUL 结尾。
            let atom = unsafe { RegisterClassW(&wc) };
            if atom == 0 {
                err = Some("RegisterClassW 失败（窗口类已存在或参数非法）".to_string());
            }
        });
        if let Some(e) = err {
            return Err(e);
        }

        let title = wide("deer-vk swapchain smoke");
        // SAFETY: 类名/标题都以 NUL 结尾且在调用期间存活；父窗口/菜单传 null 合法。
        let hwnd = unsafe {
            CreateWindowExW(
                0,
                class_w.as_ptr(),
                title.as_ptr(),
                WS_OVERLAPPEDWINDOW | WS_VISIBLE,
                100,
                100,
                width,
                height,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                module,
                std::ptr::null_mut(),
            )
        };
        if hwnd.is_null() {
            return Err("CreateWindowExW 失败".to_string());
        }
        // SAFETY: `hwnd` 刚创建且有效。
        unsafe {
            ShowWindow(hwnd, SW_SHOW);
            UpdateWindow(hwnd);
        }
        Ok(Win32TestWindow {
            hwnd,
            hinstance: module,
        })
    }

    /// 改变窗口尺寸（用于触发交换链过期/重建）。
    fn resize(&self, width: i32, height: i32) {
        use win32_window::*;
        // SAFETY: `hwnd` 由本结构持有且有效。
        unsafe {
            SetWindowPos(
                self.hwnd,
                std::ptr::null_mut(),
                0,
                0,
                width,
                height,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
    }

    /// 客户区尺寸（**物理像素**，去掉边框/标题栏）—— 交换链的 `currentExtent` 应当就是它。
    fn client_size(&self) -> Extent {
        use win32_window::*;
        let mut rect = Rect::default();
        // SAFETY: `hwnd` 有效；`rect` 是可写输出。
        unsafe { GetClientRect(self.hwnd, &mut rect) };
        Extent {
            width: (rect.right - rect.left).max(0) as u32,
            height: (rect.bottom - rect.top).max(0) as u32,
        }
    }
}

#[cfg(windows)]
impl Drop for Win32TestWindow {
    fn drop(&mut self) {
        use win32_window::*;
        if !self.hwnd.is_null() {
            // SAFETY: `hwnd` 由本结构创建且未销毁。
            unsafe { DestroyWindow(self.hwnd) };
            self.hwnd = std::ptr::null_mut();
        }
    }
}

/// 抽干窗口消息队列（避免测试期间窗口被系统标记「无响应」）。
#[cfg(windows)]
fn pump_messages() {
    use win32_window::*;
    let mut msg = Msg {
        hwnd: std::ptr::null_mut(),
        message: 0,
        w_param: 0,
        l_param: 0,
        time: 0,
        pt_x: 0,
        pt_y: 0,
        l_private: 0,
    };
    // SAFETY: `msg` 是可写输出；`PM_REMOVE` 表示取走消息。
    unsafe {
        while PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

// ── B5-3：**窗口侧析构顺序**的具名判据（复审 Important F-3）──────────────────

/// 子进程探针成功跑完时打印的哨兵（父进程用它区分「真的跑了」与「0 tests run」）。
const DTOR_WINDOW_SENTINEL: &str = "【DTOR 窗口探针】";

/// **子进程探针**：建真窗口 → 建 [`WindowedRenderer`] → 画一帧（建出 `UiResources`）
/// → **主动释放** → 析构 → 进程正常退出。
///
/// ## 为什么需要它（复审 F-3 实测的失效模式）
///
/// `UiResources` 的字段顺序是**硬契约**（`set` 必须在 `pool` 之前）：
/// 写反 ⇒ 每条测试的逻辑都通过，但进程退出时
/// `STATUS_ACCESS_VIOLATION`（`vkFreeDescriptorSets` 访问已销毁的池），
/// **没有失败用例名**、关校验层时**没有消息**。本探针把这条契约变成
/// 「子进程退出码必须为 0」——父进程那条具名用例因此能报出它。
///
/// 由 `windowed_renderer_construct_and_destruct_in_a_fresh_child_exits_cleanly`
/// 以 `DEER_VK_DTOR_PROBE_WINDOW` 打开；默认**立即返回**。
#[test]
#[cfg(windows)]
fn child_probe_windowed_renderer_release() {
    if std::env::var("DEER_VK_DTOR_PROBE_WINDOW").is_err() {
        return;
    }
    if !window_tests_enabled() {
        eprintln!("子进程：需要 DEER_VK_WINDOW_TESTS=1（真实窗口）");
        std::process::exit(2);
    }
    if !ffi::Instance::extension_available(surface::SURFACE_EXTENSION)
        || !ffi::Instance::extension_available(surface::WIN32_SURFACE_EXTENSION)
    {
        eprintln!("子进程：本机 Vulkan 不提供 VK_KHR_surface / VK_KHR_win32_surface");
        std::process::exit(3);
    }
    let win = Win32TestWindow::create("deer-vk-dtor-probe", 320, 240)
        .expect("子进程：建 Win32 测试窗口失败");
    let raw = RawWindowHandle {
        platform: Platform::Windows,
        handle: win.hwnd as usize,
        display: win.hinstance as usize,
    };
    const CLEAR: Color = Color::rgb(CLEAR_R, CLEAR_G, CLEAR_B);
    let want = Extent {
        width: 320,
        height: 240,
    };
    let mut r = match WindowedRenderer::new(0, raw, want, CLEAR) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("子进程：建窗口渲染器失败（{e}）");
            std::process::exit(4);
        }
    };
    // 画一帧界面树（**必须**：`UiResources` 是惰性建的，不画就没有描述符集/池可析构）
    let mut shapes = deer_gpu::DrawList::new();
    shapes.push(deer_gpu::DrawCmd::FillRect {
        rect: deer_gpu::RectI::new(4, 4, 100, 60),
        color: Color::WHITE,
    });
    r.draw_and_present(&shapes, None)
        .expect("子进程：形状界面帧失败");
    // 有系统字体时连文本路径一起走（图集纹理 + **描述符改指**也是析构契约的一部分）
    let text_ran = match deer_gpu::text::TextEngine::from_system_font(16.0) {
        Ok(mut engine) => {
            let mut with_text = shapes.clone();
            with_text.push(deer_gpu::DrawCmd::Text {
                rect: deer_gpu::RectI::new(4, 4, 120, 30),
                text: "Wg".into(),
                color: Color::WHITE,
                size: 16.0,
                align: 0,
            });
            r.draw_and_present(&with_text, Some(&mut engine))
                .expect("子进程：带文本的界面帧失败");
            true
        }
        Err(e) => {
            eprintln!("子进程：拿不到系统字体（{e}）⇒ 只覆盖形状界面帧路径");
            false
        }
    };
    // ★ 前置条件（**显式断言**）：界面资源必须真的被建出来了 ——
    //   否则「析构顺序」这件事根本没发生，本探针是空转的。
    assert!(
        r.ui_build_count() >= 1,
        "子进程前置条件不成立：界面资源从未被建（ui_build_count = {}）⇒ \
         没有描述符集/池可析构，本探针空转",
        r.ui_build_count()
    );
    // 主动释放一次 + 再画一帧（覆盖 `release_ui_resources` 那条析构路径）
    // ⚠️ 这一帧用**只有形状**的列表：`None` 引擎 + 列表里有 `Text` 命令会（正确地）报
    //    `Unsupported` —— 那是契约，不是缺陷（别把探针写成依赖那个错误）。
    r.release_ui_resources().expect("子进程：释放界面资源失败");
    r.draw_and_present(&shapes, None)
        .expect("子进程：重建后再画一帧失败");
    r.wait_idle().expect("子进程：wait_idle 失败");
    println!(
        "{DTOR_WINDOW_SENTINEL}窗口渲染器（UiResources：统一管线 + 描述符集/池{text_state}）\
         已构造、使用并析构，进程准备正常退出（界面资源建过 {} 次）",
        r.ui_build_count(),
        text_state = if text_ran { " + 图集" } else { "" },
    );
    drop(r);
    drop(win);
}

/// **具名判据**：在干净**子进程**里构造 + 使用 + 析构一个窗口渲染器，子进程必须正常退出。
///
/// 与 [`child_probe_windowed_renderer_release`] 配对 —— 抓「`UiResources` 字段顺序写反
/// ⇒ 进程 abort」这条**原本没有名字**的失效模式（复审 Important F-3）。
///
/// 默认（未设 `DEER_VK_WINDOW_TESTS=1`）**明确跳过**，与 `windowed_chain_end_to_end` 同一门槛。
#[test]
#[cfg(windows)]
fn windowed_renderer_construct_and_destruct_in_a_fresh_child_exits_cleanly() {
    if std::env::var("DEER_VK_DTOR_PROBE_WINDOW").is_ok() {
        return; // 自己是子进程时不要递归
    }
    if !window_tests_enabled() {
        eprintln!("跳过：需要真实窗口（设 DEER_VK_WINDOW_TESTS=1 启用）");
        return;
    }
    let exe = std::env::current_exe().expect("拿不到当前测试二进制路径");
    let out = std::process::Command::new(&exe)
        .args([
            "--exact",
            "child_probe_windowed_renderer_release",
            "--nocapture",
        ])
        .env("DEER_VK_DTOR_PROBE_WINDOW", "1")
        .env("DEER_VK_WINDOW_TESTS", "1")
        .output()
        .expect("启动子进程");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    // ★ 先看**退出码**：它才是这条判据的主体 —— 顺序写反时子进程会以 `0xC0000005` 退出。
    //   （顺序不能反：崩掉的子进程**不会**打印哨兵，先查哨兵会把「崩了」误报成「空转」。）
    assert!(
        out.status.success(),
        "子进程（构造 + 使用 + 析构一个窗口渲染器）**没有正常退出**：exit={:?}\n\
         若是 `0xC0000005`（STATUS_ACCESS_VIOLATION）：几乎一定是 `UiResources` 的\
         **字段顺序被写反**了（`pool` 提到 `set` 之前 ⇒ `vkFreeDescriptorSets` 访问已销毁的池）。\n\
         --- stdout ---\n{}\n--- stderr ---\n{}",
        out.status.code(),
        stdout.lines().rev().take(8).collect::<Vec<_>>().join("\n"),
        stderr.lines().rev().take(8).collect::<Vec<_>>().join("\n")
    );
    // ★ 再看**哨兵**：退出码 0 但没有哨兵 ⇒ 探针根本没跑（`--exact` 没匹配到）⇒ 判据空转。
    assert!(
        stdout.contains(DTOR_WINDOW_SENTINEL),
        "子进程退出码正常，但没有跑探针（stdout 里没有哨兵 `{DTOR_WINDOW_SENTINEL}`）⇒ \
         本判据会空转（`--exact` 没匹配到用例名？）\n--- stdout ---\n{}",
        stdout
    );
    println!("干净子进程里「构造 + 使用 + 析构窗口渲染器」正常退出 ✅（析构顺序契约有名字了）");
}
