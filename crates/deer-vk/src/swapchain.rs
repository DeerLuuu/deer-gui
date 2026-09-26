//! **交换链 + 呈现 + 信号量**（M2b）。
//!
//! ## 这个模块最危险的地方（也是任务书点名的第一风险）
//!
//! `vkAcquireNextImageKHR` 与 `vkQueuePresentKHR` **不是只有「成功/失败」两种结果**：
//!
//! | 返回码 | 含义 | 正确处理 |
//! |---|---|---|
//! | `VK_SUCCESS` | 正常 | 用拿到的图像索引 |
//! | `VK_SUBOPTIMAL_KHR` | 还能用但已不最优 | **不是**成功，也不能当失败丢弃：照常画/呈现，然后重建 |
//! | `VK_ERROR_OUT_OF_DATE_KHR` | 与 surface 不匹配 | 重建交换链后重试（**正常路径**） |
//! | `VK_TIMEOUT` / `VK_NOT_READY` | 没拿到图像 | **必须报错**（当成功用会画到未分配的图像索引上 ⇒ 崩溃或花屏） |
//!
//! 所以映射被抽成两个**纯函数**（[`map_acquire_result`] / [`map_present_result`]）：
//! 它们不碰驱动，可以被穷举单测 —— 这类错误在真机上很难稳定复现，只能靠单测钉住。
//!
//! ## 确定性配置
//!
//! [`Swapchain::choose_config`] 把「挑格式/呈现模式/尺寸/图像数」的规则收成
//! [`pick_config`]（同样不碰驱动的纯逻辑）：同样的能力列表 ⇒ **同一组配置**。
//! 不用随机的理由是：配置一变，之前验证过的渲染通道/管线状态就可能失效，
//! 而「有时对有时错」是最贵的 bug。

use std::cell::Cell;
use std::ffi::c_void;
use std::ptr;

use deer_gpu::{Extent, GpuError, GpuResult};

use crate::device::{vk_result_name, DeviceFns, VkDevice};
use crate::ffi;
use crate::ffi_dev as vk;
use crate::loader::Lib;
use crate::surface::{Surface, SurfaceCapabilitiesKHR, SurfaceFormatKHR};

/// `VkSwapchainKHR`（设备级对象）。
pub type SwapchainHandle = *mut c_void;

/// **设备扩展**：`vkCreateSwapchainKHR` 等符号只有在设备启用了它之后才可用。
pub const SWAPCHAIN_EXTENSION: &str = "VK_KHR_swapchain";

/// `VK_STRUCTURE_TYPE_SWAPCHAIN_CREATE_INFO_KHR`
pub const VK_STRUCTURE_TYPE_SWAPCHAIN_CREATE_INFO_KHR: i32 = 1_000_001_000;
/// `VK_STRUCTURE_TYPE_PRESENT_INFO_KHR`
pub const VK_STRUCTURE_TYPE_PRESENT_INFO_KHR: i32 = 1_000_001_001;

/// `VkPresentModeKHR`
pub const VK_PRESENT_MODE_IMMEDIATE_KHR: i32 = 0;
/// 三重缓冲式的「不排队、覆盖旧帧」（撕裂风险低，但要求驱动支持）。
pub const VK_PRESENT_MODE_MAILBOX_KHR: i32 = 1;
/// **规范保证一定支持**：垂直同步队列（本实现的首选）。
pub const VK_PRESENT_MODE_FIFO_KHR: i32 = 2;
/// FIFO 的「来不及就撕裂」变体。
pub const VK_PRESENT_MODE_FIFO_RELAXED_KHR: i32 = 3;

/// `VkColorSpaceKHR::VK_COLOR_SPACE_SRGB_NONLINEAR_KHR`（Windows 上的唯一常见值）。
pub const VK_COLOR_SPACE_SRGB_NONLINEAR_KHR: i32 = 0;

/// `VkCompositeAlphaFlagBitsKHR`
pub const VK_COMPOSITE_ALPHA_OPAQUE_BIT_KHR: u32 = 0x1;
pub const VK_COMPOSITE_ALPHA_PRE_MULTIPLIED_BIT_KHR: u32 = 0x2;
pub const VK_COMPOSITE_ALPHA_POST_MULTIPLIED_BIT_KHR: u32 = 0x4;
pub const VK_COMPOSITE_ALPHA_INHERIT_BIT_KHR: u32 = 0x8;

/// `VkSurfaceTransformFlagBitsKHR::IDENTITY`
pub const VK_SURFACE_TRANSFORM_IDENTITY_BIT_KHR: i32 = 0x1;

/// `VkFormat` 的 `VK_FORMAT_UNDEFINED`。
pub const VK_FORMAT_UNDEFINED: i32 = 0;
/// `VkFormat::VK_FORMAT_B8G8R8A8_SRGB`（Windows 交换链首选；**字节序是 B,G,R,A**）。
pub const VK_FORMAT_B8G8R8A8_SRGB: i32 = 50;
/// `VkFormat::VK_FORMAT_B8G8R8A8_UNORM`（同上，线性）。
pub const VK_FORMAT_B8G8R8A8_UNORM: i32 = 44;
/// `VkFormat::VK_FORMAT_R8G8B8A8_SRGB`（**字节序就是 R,G,B,A**）。
pub const VK_FORMAT_R8G8B8A8_SRGB: i32 = 43;

/// `VkSwapchainCreateInfoKHR`
///
/// ```c
///   VkStructureType sType; const void* pNext; VkSwapchainCreateFlagsKHR flags;
///   VkSurfaceKHR surface; uint32_t minImageCount; VkFormat imageFormat;
///   VkColorSpaceKHR imageColorSpace; VkExtent2D imageExtent; uint32_t imageArrayLayers;
///   VkImageUsageFlags imageUsage; VkSharingMode imageSharingMode;
///   uint32_t queueFamilyIndexCount; const uint32_t* pQueueFamilyIndices;
///   VkSurfaceTransformFlagBitsKHR preTransform; VkCompositeAlphaFlagBitsKHR compositeAlpha;
///   VkPresentModeKHR presentMode; VkBool32 clipped; VkSwapchainKHR oldSwapchain;
/// ```
/// 布局：8 + 8 + 4 + 8 + 4 + 4 + 4 + 8 + 4 + 4 + 4 + 4 + 8 + 4 + 4 + 4 + 4 + 8 = **104 字节**。
#[repr(C)]
#[derive(Clone, Copy)]
pub struct SwapchainCreateInfoKHR {
    pub s_type: i32,
    pub p_next: *const c_void,
    pub flags: u32,
    pub surface: ffi::SurfaceHandle,
    pub min_image_count: u32,
    pub image_format: i32,
    pub image_color_space: i32,
    pub image_extent: vk::Extent2D,
    pub image_array_layers: u32,
    pub image_usage: u32,
    pub image_sharing_mode: i32,
    pub queue_family_index_count: u32,
    pub p_queue_family_indices: *const u32,
    pub pre_transform: i32,
    pub composite_alpha: u32,
    pub present_mode: i32,
    pub clipped: u32,
    pub old_swapchain: SwapchainHandle,
}

/// `VkPresentInfoKHR`
///
/// ```c
///   VkStructureType sType; const void* pNext; uint32_t waitSemaphoreCount;
///   const VkSemaphore* pWaitSemaphores; uint32_t swapchainCount;
///   const VkSwapchainKHR* pSwapchains; const uint32_t* pImageIndices; VkResult* pResults;
/// ```
/// 布局：8 + 8 + 4 + 8 + 4 + 8 + 8 + 8 = **64 字节**（`pNext` 在第 8 字节，不是第 4）。
#[repr(C)]
#[derive(Clone, Copy)]
pub struct PresentInfoKHR {
    pub s_type: i32,
    pub p_next: *const c_void,
    pub wait_semaphore_count: u32,
    pub p_wait_semaphores: *const vk::SemaphoreHandle,
    pub swapchain_count: u32,
    pub p_swapchains: *const SwapchainHandle,
    pub p_image_indices: *const u32,
    pub p_results: *mut ffi::VkResult,
}

/// 表面能力（[`SurfaceCapabilitiesKHR`] 的**可测副本**）。
///
/// 存在的理由：原始结构体把「由应用决定尺寸」编码成 `currentExtent = (0xFFFFFFFF, 0xFFFFFFFF)`
/// 这个哨兵值 —— 直接拿它做判断是 bug 温床。这里把它翻成 `Option<Extent>`，
/// 于是 [`pick_config`] 的规则可以被普通单测穷举（不需要窗口、不需要驱动）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SurfaceCapabilities {
    pub min_image_count: u32,
    /// `0` 表示「无上限」（规范定义，不是「不能建」）。
    pub max_image_count: u32,
    /// `None` = 应用决定（哨兵值或窗口最小化导致的 0×0）。
    pub current_extent: Option<Extent>,
    pub min_image_extent: Extent,
    pub max_image_extent: Extent,
    pub supported_composite_alpha: u32,
    pub current_transform: i32,
    /// `VkImageUsageFlags`：交换链图像允许的用途。
    ///
    /// **回读呈现帧**要求 `VK_IMAGE_USAGE_TRANSFER_SRC_BIT`；这张位图就是唯一依据
    /// （不能假设它一定在 —— 不在就必须明确报错，而不是建一个回读不了却声称能回读的交换链）。
    pub supported_usage_flags: u32,
}

impl SurfaceCapabilities {
    /// 从驱动的原始结构体翻译过来（哨兵值 `u32::MAX` / 0×0 ⇒ `None`）。
    pub fn from_raw(raw: &SurfaceCapabilitiesKHR) -> SurfaceCapabilities {
        let cw = raw.current_extent.width;
        let ch = raw.current_extent.height;
        let current_extent = if cw == u32::MAX || ch == u32::MAX || cw == 0 || ch == 0 {
            None
        } else {
            Some(Extent {
                width: cw,
                height: ch,
            })
        };
        SurfaceCapabilities {
            min_image_count: raw.min_image_count,
            max_image_count: raw.max_image_count,
            current_extent,
            min_image_extent: Extent {
                width: raw.min_image_extent.width,
                height: raw.min_image_extent.height,
            },
            max_image_extent: Extent {
                width: raw.max_image_extent.width,
                height: raw.max_image_extent.height,
            },
            supported_composite_alpha: raw.supported_composite_alpha,
            current_transform: raw.current_transform,
            supported_usage_flags: raw.supported_usage_flags,
        }
    }
}

/// 一个「格式 + 色彩空间」组合（[`SurfaceFormatKHR`] 的可测副本）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SurfaceFormat {
    pub format: i32,
    pub color_space: i32,
}

impl SurfaceFormat {
    pub fn from_raw(raw: &SurfaceFormatKHR) -> SurfaceFormat {
        SurfaceFormat {
            format: raw.format,
            color_space: raw.color_space,
        }
    }
}

/// 交换链的最终配置（**冻死 API**）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SwapchainConfig {
    pub extent: Extent,
    pub format: i32,
    pub present_mode: i32,
    pub image_count: u32,
}

/// `vkAcquireNextImageKHR` 的结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Acquire {
    /// 拿到了索引为 `i` 的可绘制图像。
    Image(u32),
    /// 交换链与 surface 不匹配 ⇒ 调用方应重建后重试。
    OutOfDate,
    /// 仍然可用但不最优 ⇒ 这一帧照常画/呈现，然后重建。
    Suboptimal,
}

/// `vkQueuePresentKHR` 的结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Present {
    /// 已提交给呈现引擎。
    Presented,
    /// 交换链过期（**仍算一次呈现调用**，但必须重建）。
    OutOfDate,
    /// 呈现了，但已不最优。
    Suboptimal,
}

/// **纯逻辑**：挑一组确定性的合法交换链配置。
///
/// 规则（顺序即优先级，全部可单测）：
/// 1. **格式**：优先 `B8G8R8A8_SRGB`，其次 `R8G8B8A8_SRGB`；
///    若列表只有 `VK_FORMAT_UNDEFINED`（规范：表示「任意格式都行」）⇒ 用 `B8G8R8A8_SRGB`；
///    都没有 ⇒ 用第一个**非 UNDEFINED** 的（如实退回，不假装有 sRGB）。
/// 2. **呈现模式**：优先 `FIFO`（规范保证任何实现都支持），其次 `FIFO_RELAXED`，
///    再退回列表第一个。
/// 3. **尺寸**：`currentExtent` 有值就**必须**用它（否则 `vkCreateSwapchainKHR`
///    在 Windows 上直接 `OUT_OF_DATE`）；没有就夹取 `want` 到 `[min, max]`。
/// 4. **图像数**：`minImageCount + 1`（多一张能少一次等待），上限 `maxImageCount`
///    （`0` = 无上限），下限 1。
pub fn pick_config(
    caps: &SurfaceCapabilities,
    formats: &[SurfaceFormat],
    present_modes: &[i32],
    want: Extent,
) -> GpuResult<SwapchainConfig> {
    if formats.is_empty() {
        return Err(GpuError::Unsupported(
            "surface 未报告任何支持格式 ⇒ 无法挑交换链配置".to_string(),
        ));
    }
    if present_modes.is_empty() {
        return Err(GpuError::Unsupported(
            "surface 未报告任何呈现模式（FIFO 按规范必须支持）⇒ 无法挑交换链配置".to_string(),
        ));
    }

    // ① 格式
    let preferred = [VK_FORMAT_B8G8R8A8_SRGB, VK_FORMAT_R8G8B8A8_SRGB];
    let chosen_format = preferred
        .iter()
        .find_map(|p| formats.iter().find(|f| f.format == *p).copied())
        .or_else(|| {
            formats
                .iter()
                .find(|f| f.format != VK_FORMAT_UNDEFINED)
                .copied()
        })
        .unwrap_or(SurfaceFormat {
            // 全是 UNDEFINED ⇒ 规范允许任意格式，选 Windows 最常见的 sRGB BGRA
            format: VK_FORMAT_B8G8R8A8_SRGB,
            color_space: formats[0].color_space,
        });

    // ② 呈现模式
    let present_mode = if present_modes.contains(&VK_PRESENT_MODE_FIFO_KHR) {
        VK_PRESENT_MODE_FIFO_KHR
    } else if present_modes.contains(&VK_PRESENT_MODE_FIFO_RELAXED_KHR) {
        VK_PRESENT_MODE_FIFO_RELAXED_KHR
    } else {
        present_modes[0]
    };

    // ③ 尺寸
    let extent = pick_extent(caps, want);

    // ④ 图像数
    let upper = if caps.max_image_count == 0 {
        u32::MAX
    } else {
        caps.max_image_count
    };
    let image_count = caps
        .min_image_count
        .saturating_add(1)
        .min(upper)
        .max(1);

    Ok(SwapchainConfig {
        extent,
        format: chosen_format.format,
        present_mode,
        image_count,
    })
}

/// 把一个维度夹到 `[lo, hi]`，并保证结果 ≥ 1。
///
/// 手写而不用 `u32::clamp`：驱动理论上可能报出 `min > max` 的退化能力
/// （窗口最小化/驱动 bug），而 `clamp` 在 `min > max` 时会 **panic** ——
/// 初始化路径 panic 是本项目明确禁止的。
fn clamp_dim(v: u32, lo: u32, hi: u32) -> u32 {
    let lo = lo.max(1);
    let hi = if hi < lo { lo } else { hi };
    v.clamp(lo, hi)
}

/// 尺寸规则（`pick_config` 的 ③）。
fn pick_extent(caps: &SurfaceCapabilities, want: Extent) -> Extent {
    let lo = caps.min_image_extent;
    let hi = caps.max_image_extent;
    let source = caps.current_extent.unwrap_or(want);
    Extent {
        width: clamp_dim(source.width, lo.width, hi.width),
        height: clamp_dim(source.height, lo.height, hi.height),
    }
}

/// **纯逻辑**：`vkAcquireNextImageKHR` 返回码 → [`Acquire`]。
///
/// `image_index` 是驱动写回的值（`VK_SUCCESS` / `VK_SUBOPTIMAL_KHR` 时有效）。
/// 其它任何返回码（含 `VK_TIMEOUT` / `VK_NOT_READY`）都**返回 `Err`** ——
/// 这是本模块最重要的一条：把它们当成功用会拿着未初始化的索引去录命令。
pub fn map_acquire_result(rc: ffi::VkResult, image_index: u32) -> GpuResult<Acquire> {
    match rc {
        ffi::VK_SUCCESS => Ok(Acquire::Image(image_index)),
        ffi::VK_SUBOPTIMAL_KHR => Ok(Acquire::Suboptimal),
        ffi::VK_ERROR_OUT_OF_DATE_KHR => Ok(Acquire::OutOfDate),
        _ => Err(GpuError::Driver {
            code: rc,
            message: format!(
                "vkAcquireNextImageKHR 失败：{}（TIMEOUT/NOT_READY 都算失败，绝不当成功）",
                vk_result_name(rc)
            ),
        }),
    }
}

/// **纯逻辑**：`vkQueuePresentKHR` 返回码 → [`Present`]。
pub fn map_present_result(rc: ffi::VkResult) -> GpuResult<Present> {
    match rc {
        ffi::VK_SUCCESS => Ok(Present::Presented),
        ffi::VK_SUBOPTIMAL_KHR => Ok(Present::Suboptimal),
        ffi::VK_ERROR_OUT_OF_DATE_KHR => Ok(Present::OutOfDate),
        _ => Err(GpuError::Driver {
            code: rc,
            message: format!("vkQueuePresentKHR 失败：{}", vk_result_name(rc)),
        }),
    }
}

/// `vkAcquireNextImageKHR` 的默认等待上限。
///
/// 规范允许传 `U64_MAX`（无限等），但那意味着「驱动/窗口出问题时永久挂住」——
/// 本项目的原则是**宁可失败也不要挂住**。1 秒对 FIFO 的垂直同步节奏足够宽裕
/// （一帧 16.7 ms）。
pub const DEFAULT_ACQUIRE_TIMEOUT_NS: u64 = 1_000_000_000;

// ── 呈现帧回读（M2b 补强）的**纯逻辑**部分 ────────────────────────────────────

/// 回读缓冲的**每行字节数**。
///
/// `vkCmdCopyImageToBuffer` 里 `bufferRowLength = 0` 的含义是「与图像宽度一致」⇒
/// 紧排、**没有任何行尾 padding**：一行就是 `宽 × 4` 字节（RGBA8/BGRA8 都是 4 字节/像素）。
/// 这条被写死成纯函数的原因：padding 算错会让整幅图像斜切，
/// 而「四角像素正确、中间错位」这种症状极难一眼看出。
pub fn readback_row_pitch(width: u32) -> u32 {
    width.saturating_mul(4)
}

/// 把交换链图像里的字节序**换成 RGBA8**（呈现帧回读的对外约定）。
///
/// - `R8G8B8A8_*`：本来就是 R,G,B,A ⇒ 原样返回；
/// - `B8G8R8A8_*`：内存里是 B,G,R,A ⇒ **交换第 0 与第 2 个字节**（每个像素一次）；
/// - 其它格式：明确报错（不猜、不近似）。
///
/// ⚠️ 不做 sRGB→线性解码：返回的就是**图像里的字节**（sRGB 格式下是 sRGB 编码值），
/// 这样它才和「窗口上看到的」「截图工具抓到的」逐字节一致，才能用来做像素断言。
pub fn reorder_to_rgba8(bytes: &[u8], format: i32) -> GpuResult<Vec<u8>> {
    if bytes.len() % 4 != 0 {
        return Err(GpuError::Unsupported(format!(
            "回读数据的字节数 {} 不是 4 的倍数 ⇒ 不可能是 RGBA8/BGRA8 像素阵列",
            bytes.len()
        )));
    }
    match format {
        VK_FORMAT_R8G8B8A8_SRGB | vk::VK_FORMAT_R8G8B8A8_UNORM => Ok(bytes.to_vec()),
        VK_FORMAT_B8G8R8A8_SRGB | VK_FORMAT_B8G8R8A8_UNORM => {
            let mut out = bytes.to_vec();
            for px in out.chunks_exact_mut(4) {
                px.swap(0, 2);
            }
            Ok(out)
        }
        other => Err(GpuError::Unsupported(format!(
            "回读不支持交换链格式 {other:#x}（目前只支持 R8G8B8A8_SRGB/UNORM 与 B8G8R8A8_SRGB/UNORM）"
        ))),
    }
}

/// 设备级交换链函数表（只解析一次，`Swapchain` 持有副本）。
#[derive(Clone, Copy)]
struct SwapchainFns {
    create: PfnCreateSwapchainKHR,
    destroy: PfnDestroySwapchainKHR,
    get_images: PfnGetSwapchainImagesKHR,
    acquire_next_image: PfnAcquireNextImageKHR,
    queue_present: PfnQueuePresentKHR,
    create_semaphore: PfnCreateSemaphore,
    destroy_semaphore: PfnDestroySemaphore,
}

// ── 交换链/信号量的函数指针类型（签名逐字对应规范） ──────────────────────────
//
// 放在 `swapchain.rs` 而不是 `ffi_dev.rs`：`ffi_dev.rs` 已被 M2a 的真机验证钉住，
// 不动它 —— 新功能的 ABI 声明与它的使用者放在一起，改动面更小。

/// `vkCreateSwapchainKHR(device, pCreateInfo, pAllocator, pSwapchain)`。
type PfnCreateSwapchainKHR = unsafe extern "system" fn(
    vk::DeviceHandle,
    *const SwapchainCreateInfoKHR,
    *const c_void,
    *mut SwapchainHandle,
) -> ffi::VkResult;
/// `vkDestroySwapchainKHR(device, swapchain, pAllocator)`。
type PfnDestroySwapchainKHR =
    unsafe extern "system" fn(vk::DeviceHandle, SwapchainHandle, *const c_void);
/// `vkGetSwapchainImagesKHR(device, swapchain, pSwapchainImageCount, pSwapchainImages)`。
type PfnGetSwapchainImagesKHR = unsafe extern "system" fn(
    vk::DeviceHandle,
    SwapchainHandle,
    *mut u32,
    *mut vk::ImageHandle,
) -> ffi::VkResult;
/// `vkAcquireNextImageKHR(device, swapchain, timeout, semaphore, fence, pImageIndex)`。
type PfnAcquireNextImageKHR = unsafe extern "system" fn(
    vk::DeviceHandle,
    SwapchainHandle,
    u64,
    vk::SemaphoreHandle,
    vk::FenceHandle,
    *mut u32,
) -> ffi::VkResult;
/// `vkQueuePresentKHR(queue, pPresentInfo)`。
type PfnQueuePresentKHR =
    unsafe extern "system" fn(vk::QueueHandle, *const PresentInfoKHR) -> ffi::VkResult;
/// `vkCreateSemaphore(device, pCreateInfo, pAllocator, pSemaphore)`。
type PfnCreateSemaphore = unsafe extern "system" fn(
    vk::DeviceHandle,
    *const vk::SemaphoreCreateInfo,
    *const c_void,
    *mut vk::SemaphoreHandle,
) -> ffi::VkResult;
/// `vkDestroySemaphore(device, semaphore, pAllocator)`。
type PfnDestroySemaphore =
    unsafe extern "system" fn(vk::DeviceHandle, vk::SemaphoreHandle, *const c_void);

/// 解析交换链/信号量相关符号。
///
/// 这些都是**设备级**函数，`vulkan-1.dll` 直接导出它们的 loader 跳板
/// （`vkGetDeviceProcAddr` 那条路更严格，但需要实例→设备的句柄链；
/// 本项目 `device.rs` 对 core 1.0 符号一直用同一手法，这里保持一致）。
/// 万一某驱动没导出，`Lib::sym` 会返回 `Unsupported` 而不是空指针崩溃。
fn load_swapchain_fns() -> GpuResult<SwapchainFns> {
    let lib = Lib::open()?;
    // SAFETY: 每个符号名都与 `vk::Pfn*` 声明的签名逐字对应（见 `ffi_dev.rs`）。
    unsafe {
        Ok(SwapchainFns {
            create: lib.sym("vkCreateSwapchainKHR")?,
            destroy: lib.sym("vkDestroySwapchainKHR")?,
            get_images: lib.sym("vkGetSwapchainImagesKHR")?,
            acquire_next_image: lib.sym("vkAcquireNextImageKHR")?,
            queue_present: lib.sym("vkQueuePresentKHR")?,
            create_semaphore: lib.sym("vkCreateSemaphore")?,
            destroy_semaphore: lib.sym("vkDestroySemaphore")?,
        })
    }
}

/// 一个 `VkSemaphore`。创建/销毁都由本模块负责，`Drop` 保证不泄漏。
///
/// 用途：帧同步（`image_available` / `render_finished`）。
pub struct Semaphore {
    handle: vk::SemaphoreHandle,
    device: vk::DeviceHandle,
    destroy: PfnDestroySemaphore,
}

impl Semaphore {
    /// 在给定设备上创建一个**未信号**的信号量（与 Vulkan 的默认初值一致）。
    pub fn create(device: &VkDevice) -> GpuResult<Semaphore> {
        let fns = load_swapchain_fns()?;
        let info = vk::SemaphoreCreateInfo {
            s_type: vk::VK_STRUCTURE_TYPE_SEMAPHORE_CREATE_INFO,
            p_next: ptr::null(),
            flags: 0,
        };
        let mut handle: vk::SemaphoreHandle = ptr::null_mut();
        // SAFETY: `info` 在栈上存活；`handle` 是可写输出；设备句柄来自 `&VkDevice`（存活）。
        let rc = unsafe { (fns.create_semaphore)(device.handle(), &info, ptr::null(), &mut handle) };
        if rc != ffi::VK_SUCCESS {
            return Err(GpuError::Driver {
                code: rc,
                message: format!("vkCreateSemaphore 失败：{}", vk_result_name(rc)),
            });
        }
        if handle.is_null() {
            return Err(GpuError::Driver {
                code: rc,
                message: "vkCreateSemaphore 返回成功但句柄为空".to_string(),
            });
        }
        Ok(Semaphore {
            handle,
            device: device.handle(),
            destroy: fns.destroy_semaphore,
        })
    }

    pub fn handle(&self) -> vk::SemaphoreHandle {
        self.handle
    }
}

impl Drop for Semaphore {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            // SAFETY: 句柄由 `create` 创建且未销毁；设备由调用方保证比它活得久
            // （`WindowedRenderer` 把设备声明在信号量之后 ⇒ 信号量先析构）。
            // 调用方还须保证没有仍在等待它的提交（本 crate 里靠 `vkDeviceWaitIdle`）。
            unsafe { (self.destroy)(self.device, self.handle, ptr::null()) };
            self.handle = ptr::null_mut();
        }
    }
}

/// 一个已创建的交换链（含它的图像与图像视图）。
///
/// 不实现 `Send`/`Sync`：句柄是裸指针，跨线程使用需要显式同步（与 `ffi::Instance` 同样的取舍）。
pub struct Swapchain {
    handle: SwapchainHandle,
    device: vk::DeviceHandle,
    /// 建/销毁图像视图用的设备函数表。
    fns: DeviceFns,
    sc: SwapchainFns,
    images: Vec<vk::ImageHandle>,
    views: Vec<vk::ImageViewHandle>,
    extent: Extent,
    format: i32,
    present_mode: i32,
    /// `SUBOPTIMAL` 时驱动仍会写回有效索引，但冻死的 [`Acquire`] 枚举放不下它
    /// ⇒ 存这里，供 `windowed.rs` 用 `present` 释放这张图像（`&self` ⇒ 只能 `Cell`）。
    last_acquired: Cell<Option<u32>>,
}

impl Swapchain {
    /// 从 surface 能力挑一组**确定性**的合法配置。
    ///
    /// `pd` 必须是 `surface` 所在实例枚举出的物理设备（见 `surface.rs` 的约束 1）。
    pub fn choose_config(
        surface: &Surface,
        pd: ffi::PhysicalDeviceHandle,
        want: Extent,
    ) -> GpuResult<SwapchainConfig> {
        let raw_caps = surface.capabilities(pd)?;
        let caps = SurfaceCapabilities::from_raw(&raw_caps);
        let formats: Vec<SurfaceFormat> = surface
            .formats(pd)?
            .iter()
            .map(SurfaceFormat::from_raw)
            .collect();
        let modes = surface.present_modes(pd)?;
        pick_config(&caps, &formats, &modes, want)
    }

    /// 创建交换链（每个图像配一个 `VkImageView`）。
    ///
    /// `old` 非空时走 `oldSwapchain` 复用：新交换链建好后，**调用方**再销毁旧的
    /// （这里不销毁 —— 借用中的对象不能被 drop；`windowed.rs::resize` 负责顺序）。
    pub fn create(
        device: &VkDevice,
        surface: &Surface,
        cfg: SwapchainConfig,
        old: Option<&Swapchain>,
    ) -> GpuResult<Swapchain> {
        if cfg.extent.width == 0 || cfg.extent.height == 0 {
            return Err(GpuError::Unsupported(
                "交换链尺寸不能为 0（窗口最小化时应先不重建，或夹到 ≥1）".to_string(),
            ));
        }
        if cfg.image_count == 0 {
            return Err(GpuError::Unsupported(
                "交换链图像数必须 ≥ 1（vkCreateSwapchainKHR 的 minImageCount 不能为 0）".to_string(),
            ));
        }

        let pd = device.physical_device();
        let raw_caps = surface.capabilities(pd)?;
        let caps = SurfaceCapabilities::from_raw(&raw_caps);
        let formats = surface.formats(pd)?;
        let modes = surface.present_modes(pd)?;

        // 用调用方给的 cfg 但也**校验**它 —— 自己算错了要在这里就报清楚，
        // 而不是让驱动抛一个 `VK_ERROR_FORMAT_NOT_SUPPORTED`/`OUT_OF_DATE`。
        if cfg.image_count < caps.min_image_count {
            return Err(GpuError::Unsupported(format!(
                "交换链图像数 {} 小于 surface 要求的最小值 {}",
                cfg.image_count, caps.min_image_count
            )));
        }
        if caps.max_image_count != 0 && cfg.image_count > caps.max_image_count {
            return Err(GpuError::Unsupported(format!(
                "交换链图像数 {} 超过 surface 允许的最大值 {}",
                cfg.image_count, caps.max_image_count
            )));
        }
        let format_supported = formats
            .iter()
            .any(|f| f.format == cfg.format || f.format == VK_FORMAT_UNDEFINED);
        if !format_supported {
            return Err(GpuError::Unsupported(format!(
                "格式 {:#x} 不在 surface 支持的格式列表里（{:?}）",
                cfg.format,
                formats.iter().map(|f| f.format).collect::<Vec<_>>()
            )));
        }
        if !modes.contains(&cfg.present_mode) {
            return Err(GpuError::Unsupported(format!(
                "呈现模式 {} 不在 surface 支持的模式列表里（{modes:?}）",
                cfg.present_mode
            )));
        }

        // ── 图像用途：先查 supportedUsageFlags，缺谁就明确报谁（**不静默去掉**）──
        //
        // ① COLOR_ATTACHMENT：渲染通道的附件，WSI 规范要求可呈现图像必须支持；
        // ② TRANSFER_SRC：**回读呈现帧**（`WindowedRenderer::read_back_last_frame`）必需。
        //    「回读不了却声称能回读」比「直接报错」难查得多，所以这里硬失败。
        if caps.supported_usage_flags & vk::VK_IMAGE_USAGE_COLOR_ATTACHMENT_BIT == 0 {
            return Err(GpuError::Unsupported(format!(
                "该 surface 的交换链图像不支持 VK_IMAGE_USAGE_COLOR_ATTACHMENT_BIT（supportedUsageFlags={:#x}）\
                 ⇒ 无法作为渲染目标（按 WSI 规范这本不该发生，说明驱动/窗口异常）",
                caps.supported_usage_flags
            )));
        }
        if caps.supported_usage_flags & vk::VK_IMAGE_USAGE_TRANSFER_SRC_BIT == 0 {
            return Err(GpuError::Unsupported(format!(
                "该 surface 的交换链图像不支持 VK_IMAGE_USAGE_TRANSFER_SRC_BIT（supportedUsageFlags={:#x}）\
                 ⇒ 无法回读呈现出去的帧（`read_back_last_frame`）。\
                 本实现不静默降级成「没有回读能力」：那样调用方会拿到假数据",
                caps.supported_usage_flags
            )));
        }
        let image_usage =
            vk::VK_IMAGE_USAGE_COLOR_ATTACHMENT_BIT | vk::VK_IMAGE_USAGE_TRANSFER_SRC_BIT;

        // 色彩空间要与格式配对（列表里同一个格式可能有多个色彩空间）
        let color_space = formats
            .iter()
            .find(|f| f.format == cfg.format)
            .map(|f| f.color_space)
            .unwrap_or(formats[0].color_space);

        // 合成方式：优先 OPAQUE（绝大多数窗口场景），否则取支持位里最低的那一个
        let composite_alpha = if caps.supported_composite_alpha & VK_COMPOSITE_ALPHA_OPAQUE_BIT_KHR
            != 0
        {
            VK_COMPOSITE_ALPHA_OPAQUE_BIT_KHR
        } else {
            let first = [
                VK_COMPOSITE_ALPHA_PRE_MULTIPLIED_BIT_KHR,
                VK_COMPOSITE_ALPHA_POST_MULTIPLIED_BIT_KHR,
                VK_COMPOSITE_ALPHA_INHERIT_BIT_KHR,
            ]
            .into_iter()
            .find(|b| caps.supported_composite_alpha & b != 0);
            match first {
                Some(b) => b,
                None => {
                    return Err(GpuError::Unsupported(
                        "surface 没有报告任何合成方式（supportedCompositeAlpha = 0）".to_string(),
                    ))
                }
            }
        };

        // 变换：优先 IDENTITY（本实现不做旋转）；驱动只支持别的就如实采用 current
        let pre_transform = if caps.current_transform != 0 {
            caps.current_transform
        } else {
            VK_SURFACE_TRANSFORM_IDENTITY_BIT_KHR
        };

        let info = SwapchainCreateInfoKHR {
            s_type: VK_STRUCTURE_TYPE_SWAPCHAIN_CREATE_INFO_KHR,
            p_next: ptr::null(),
            flags: 0,
            surface: surface.handle(),
            min_image_count: cfg.image_count,
            image_format: cfg.format,
            image_color_space: color_space,
            image_extent: vk::Extent2D {
                width: cfg.extent.width,
                height: cfg.extent.height,
            },
            image_array_layers: 1,
            image_usage,
            // 单队列族（本实现的图形队列 == 呈现队列）⇒ EXCLUSIVE，无需队列族索引列表
            image_sharing_mode: vk::VK_SHARING_MODE_EXCLUSIVE,
            queue_family_index_count: 0,
            p_queue_family_indices: ptr::null(),
            pre_transform,
            composite_alpha,
            present_mode: cfg.present_mode,
            clipped: vk::VK_TRUE,
            old_swapchain: old.map_or(ptr::null_mut(), |o| o.handle),
        };

        let sc = load_swapchain_fns()?;
        let mut handle: SwapchainHandle = ptr::null_mut();
        // SAFETY: 设备由 `&VkDevice` 保证存活且已启用 `VK_KHR_swapchain`；
        // `info` 在栈上存活且字段全部初始化；surface 来自同一实例（文档约束）；
        // `handle` 是可写输出。`old_swapchain` 若非空由调用方保证存活。
        let rc = unsafe { (sc.create)(device.handle(), &info, ptr::null(), &mut handle) };
        if rc != ffi::VK_SUCCESS {
            return Err(GpuError::Driver {
                code: rc,
                message: format!(
                    "vkCreateSwapchainKHR 失败：{}（尺寸 {}×{}，格式 {:#x}，模式 {}，图像数 {}）",
                    vk_result_name(rc),
                    cfg.extent.width,
                    cfg.extent.height,
                    cfg.format,
                    cfg.present_mode,
                    cfg.image_count
                ),
            });
        }
        if handle.is_null() {
            return Err(GpuError::Driver {
                code: rc,
                message: "vkCreateSwapchainKHR 返回成功但句柄为空".to_string(),
            });
        }

        // 图像列表（两阶段；VK_INCOMPLETE 表示数量又变了，按拿到的算）
        let mut count = 0u32;
        // SAFETY: 传 null 数组 = 规定的「只查数量」用法。
        let rc = unsafe { (sc.get_images)(device.handle(), handle, &mut count, ptr::null_mut()) };
        if rc != ffi::VK_SUCCESS && rc != ffi::VK_INCOMPLETE {
            // SAFETY: 刚创建、尚未交给任何结构 ⇒ 这里必须自己清理，否则泄漏
            unsafe { (sc.destroy)(device.handle(), handle, ptr::null()) };
            return Err(GpuError::Driver {
                code: rc,
                message: format!("vkGetSwapchainImagesKHR（查数量）失败：{}", vk_result_name(rc)),
            });
        }
        if count == 0 {
            // SAFETY: 同上。
            unsafe { (sc.destroy)(device.handle(), handle, ptr::null()) };
            return Err(GpuError::Driver {
                code: rc,
                message: "交换链报告 0 张图像".to_string(),
            });
        }
        let mut images = vec![ptr::null_mut(); count as usize];
        // SAFETY: `images` 容量与 `count` 一致。
        let rc = unsafe {
            (sc.get_images)(device.handle(), handle, &mut count, images.as_mut_ptr())
        };
        if rc != ffi::VK_SUCCESS && rc != ffi::VK_INCOMPLETE {
            // SAFETY: 同上（`images` 里的句柄属于交换链，随交换链一起销毁）。
            unsafe { (sc.destroy)(device.handle(), handle, ptr::null()) };
            return Err(GpuError::Driver {
                code: rc,
                message: format!("vkGetSwapchainImagesKHR 失败：{}", vk_result_name(rc)),
            });
        }
        images.truncate(count as usize);

        // 每张图像一个视图（2D / 交换链格式 / COLOR / 1 mip / 1 layer / IDENTITY 分量）
        let fns = *device.fns();
        let mut views = Vec::with_capacity(images.len());
        for (i, image) in images.iter().enumerate() {
            let view_info = vk::ImageViewCreateInfo {
                s_type: vk::VK_STRUCTURE_TYPE_IMAGE_VIEW_CREATE_INFO,
                p_next: ptr::null(),
                flags: 0,
                image: *image,
                view_type: vk::VK_IMAGE_VIEW_TYPE_2D,
                format: cfg.format,
                components_r: vk::VK_COMPONENT_SWIZZLE_IDENTITY,
                components_g: vk::VK_COMPONENT_SWIZZLE_IDENTITY,
                components_b: vk::VK_COMPONENT_SWIZZLE_IDENTITY,
                components_a: vk::VK_COMPONENT_SWIZZLE_IDENTITY,
                subresource_range: vk::ImageSubresourceRange {
                    aspect_mask: vk::VK_IMAGE_ASPECT_COLOR_BIT,
                    base_mip_level: 0,
                    level_count: 1,
                    base_array_layer: 0,
                    layer_count: 1,
                },
            };
            let mut view: vk::ImageViewHandle = ptr::null_mut();
            // SAFETY: 图像由本交换链持有；`view_info` 在栈上存活；`view` 是可写输出。
            let rc = unsafe {
                (fns.create_image_view)(device.handle(), &view_info, ptr::null(), &mut view)
            };
            if rc != ffi::VK_SUCCESS {
                // 已经建好的视图 + 交换链都要清掉（图像本身归交换链）
                for v in &views {
                    // SAFETY: 这些视图由本循环创建且未销毁；设备仍存活。
                    unsafe { (fns.destroy_image_view)(device.handle(), *v, ptr::null()) };
                }
                // SAFETY: 交换链刚创建、尚未交给任何结构。
                unsafe { (sc.destroy)(device.handle(), handle, ptr::null()) };
                return Err(GpuError::Driver {
                    code: rc,
                    message: format!(
                        "vkCreateImageView 失败（第 {i} 张交换链图像）：{}",
                        vk_result_name(rc)
                    ),
                });
            }
            views.push(view);
        }

        Ok(Swapchain {
            handle,
            device: device.handle(),
            fns,
            sc,
            images,
            views,
            extent: cfg.extent,
            format: cfg.format,
            present_mode: cfg.present_mode,
            last_acquired: Cell::new(None),
        })
    }

    pub fn extent(&self) -> Extent {
        self.extent
    }

    pub fn format(&self) -> i32 {
        self.format
    }

    pub fn present_mode(&self) -> i32 {
        self.present_mode
    }

    pub fn images(&self) -> &[vk::ImageHandle] {
        &self.images
    }

    pub fn image_views(&self) -> &[vk::ImageViewHandle] {
        &self.views
    }

    pub fn image_count(&self) -> u32 {
        self.images.len() as u32
    }

    /// 取下一张可绘制图像（默认 1 秒超时）。
    pub fn acquire(&self, image_available: vk::SemaphoreHandle) -> GpuResult<Acquire> {
        self.acquire_with_timeout(image_available, DEFAULT_ACQUIRE_TIMEOUT_NS)
    }

    /// 同 [`Swapchain::acquire`]，但显式给等待上限（`u64::MAX` = 无限等，不推荐）。
    ///
    /// `clippy::not_unsafe_ptr_arg_deref` 的 `allow` 理由：签名是**冻死的公开 API**
    /// （`image_available` 是 `SemaphoreHandle`，不能包成新类型），而 Vulkan 句柄的
    /// 有效性/状态（必须未被信号、且本次 acquire 独占）是调用方的契约，已写在文档里。
    #[allow(clippy::not_unsafe_ptr_arg_deref)]
    pub fn acquire_with_timeout(
        &self,
        image_available: vk::SemaphoreHandle,
        timeout_ns: u64,
    ) -> GpuResult<Acquire> {
        let mut index: u32 = 0;
        // SAFETY: 交换链与信号量都由调用方按契约提供（信号量须处于未信号状态）；
        // fence 传 null（本实现只等信号量）；`index` 是可写输出。
        let rc = unsafe {
            (self.sc.acquire_next_image)(
                self.device,
                self.handle,
                timeout_ns,
                image_available,
                ptr::null_mut(),
                &mut index,
            )
        };
        let outcome = map_acquire_result(rc, index)?;
        // SUBOPTIMAL 时索引同样有效（规范）—— 记下来，让调用方能用 present 释放这张图像。
        if matches!(outcome, Acquire::Suboptimal) {
            self.last_acquired.set(Some(index));
        }
        Ok(outcome)
    }

    /// 最近一次 `acquire` 拿到的图像索引（`Suboptimal` 时同样有效）。
    pub(crate) fn last_acquired_image(&self) -> Option<u32> {
        self.last_acquired.get()
    }

    /// 呈现索引为 `image_index` 的图像。
    ///
    /// `clippy::not_unsafe_ptr_arg_deref` 的 `allow` 理由同 [`Swapchain::acquire_with_timeout`]：
    /// 签名冻死（`queue`/`render_finished` 都是裸句柄），有效性是调用方契约。
    #[allow(clippy::not_unsafe_ptr_arg_deref)]
    pub fn present(
        &self,
        queue: vk::QueueHandle,
        render_finished: vk::SemaphoreHandle,
        image_index: u32,
    ) -> GpuResult<Present> {
        if image_index >= self.image_count() {
            return Err(GpuError::Unsupported(format!(
                "要呈现的图像索引 {image_index} 超出范围（本交换链只有 {} 张）",
                self.image_count()
            )));
        }
        let waits = [render_finished];
        let chains = [self.handle];
        let indices = [image_index];
        let info = PresentInfoKHR {
            s_type: VK_STRUCTURE_TYPE_PRESENT_INFO_KHR,
            p_next: ptr::null(),
            wait_semaphore_count: 1,
            p_wait_semaphores: waits.as_ptr(),
            swapchain_count: 1,
            p_swapchains: chains.as_ptr(),
            p_image_indices: indices.as_ptr(),
            // pResults 传 null：单个交换链时结果就是返回值本身（规范允许）
            p_results: ptr::null_mut(),
        };
        // SAFETY: 队列来自本交换链所在设备；三个数组都在本栈帧存活；
        // 信号量由调用方保证已作为本次提交的 signal 信号量、且尚未被等待。
        let rc = unsafe { (self.sc.queue_present)(queue, &info) };
        map_present_result(rc)
    }
}

impl Drop for Swapchain {
    fn drop(&mut self) {
        // 顺序：图像视图 → 交换链（视图引用交换链的图像，必须先销毁视图）
        for v in &self.views {
            if !v.is_null() {
                // SAFETY: 视图由本结构创建且未销毁；设备由调用方保证比它活得久。
                unsafe { (self.fns.destroy_image_view)(self.device, *v, ptr::null()) };
            }
        }
        self.views.clear();
        if !self.handle.is_null() {
            // SAFETY: 交换链由本结构创建且未销毁。调用方须保证没有仍在进行的呈现
            // （`windowed.rs` 在重建前会 `vkDeviceWaitIdle`）。
            unsafe { (self.sc.destroy)(self.device, self.handle, ptr::null()) };
            self.handle = ptr::null_mut();
        }
    }
}
