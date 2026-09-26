//! **`VkSurfaceKHR`**（M2b）：把 HAL 的不透明窗口句柄变成可呈现的表面。
//!
//! ## 为什么 surface 必须由「实例级函数」创建
//!
//! `vkCreateWin32SurfaceKHR` / `vkGetPhysicalDeviceSurfaceSupportKHR` 这类 WSI 函数
//! **不在 `vulkan-1.dll` 的导出表里**（它们是 ICD 提供、由 loader 分发的）。
//! 所以它们只能通过 `vkGetInstanceProcAddr(instance, name)` 取 —— 见
//! [`crate::ffi::Instance::proc`]。用模块级 `GetProcAddress` 会拿到空指针。
//!
//! ## 三个「必须记住」的 Vulkan 约束
//!
//! 1. **`VkSurfaceKHR` 属于创建它的那个实例**：`vkDestroySurfaceKHR` 必须传同一个实例；
//!    而且 `vkGetPhysicalDeviceSurfaceSupportKHR` 的 VU 要求 surface 与 physicalDevice
//!    **来自同一个实例**。⇒ 所以 [`crate::device::VkDevice::open_with_present`]
//!    **借用** surface 的实例，而不是自己另建一个（跨实例用 surface 是校验层会抓的错误）。
//! 2. surface 必须在实例**之前**销毁（本 crate 的 `WindowedRenderer` 用字段顺序保证）。
//! 3. `hwnd` 必须非 0，否则 `vkCreateWin32SurfaceKHR` 会失败或产生悬空引用
//!    ⇒ 这里直接返回 [`GpuError::BadWindowHandle`]，不把垃圾交给驱动。

use std::ffi::c_void;
use std::ptr;

use deer_gpu::{GpuError, GpuResult, Platform, RawWindowHandle};

use crate::ffi;
use crate::ffi_dev as vk;

/// 跨平台的基础 surface 扩展（所有 WSI 都要求它）。
pub const SURFACE_EXTENSION: &str = "VK_KHR_surface";
/// Windows：`vkCreateWin32SurfaceKHR`。
pub const WIN32_SURFACE_EXTENSION: &str = "VK_KHR_win32_surface";
/// X11（Xlib 路径）。
pub const XLIB_SURFACE_EXTENSION: &str = "VK_KHR_xlib_surface";
/// X11（XCB 路径）。
pub const XCB_SURFACE_EXTENSION: &str = "VK_KHR_xcb_surface";
/// Wayland。
pub const WAYLAND_SURFACE_EXTENSION: &str = "VK_KHR_wayland_surface";
/// macOS（MoltenVK 的 Metal 表面）。
pub const METAL_SURFACE_EXTENSION: &str = "VK_EXT_metal_surface";

/// `VK_STRUCTURE_TYPE_WIN32_SURFACE_CREATE_INFO_KHR`（1000xxxxxx 段，与核心 0..45 不是一套）。
pub const VK_STRUCTURE_TYPE_WIN32_SURFACE_CREATE_INFO_KHR: i32 = 1_000_009_000;

/// `VkWin32SurfaceCreateInfoKHR`
///
/// ```c
///   VkStructureType sType; const void* pNext; VkWin32SurfaceCreateFlagsKHR flags;
///   HINSTANCE hinstance; HWND hwnd;
/// ```
/// 布局：sType(0) + pad + pNext(8) + flags(16) + pad + hinstance(24) + hwnd(32) ⇒ **40 字节**。
#[repr(C)]
#[derive(Clone, Copy)]
struct Win32SurfaceCreateInfoKHR {
    s_type: i32,
    p_next: *const c_void,
    flags: u32,
    hinstance: *mut c_void,
    hwnd: *mut c_void,
}

/// `VkSurfaceCapabilitiesKHR`
///
/// ```c
///   uint32_t minImageCount; uint32_t maxImageCount;
///   VkExtent2D currentExtent; VkExtent2D minImageExtent; VkExtent2D maxImageExtent;
///   uint32_t maxImageArrayLayers; VkSurfaceTransformFlagsKHR supportedTransforms;
///   VkSurfaceTransformFlagBitsKHR currentTransform; VkCompositeAlphaFlagsKHR supportedCompositeAlpha;
///   VkImageUsageFlags supportedUsageFlags;
/// ```
/// 5 个 `u32`（20）+ 3 个 `Extent2D`（24）+ 2 个 `u32`（8）= **52 字节**，`align_of == 4`。
///
/// ⚠️ `currentExtent` 为 `(0xFFFFFFFF, 0xFFFFFFFF)` 时表示「由应用决定」
/// （`VK_EXT_surface_maintenance1` 之前就是这个哨兵值），不是「宽高都是 42 亿像素」。
#[repr(C)]
#[derive(Clone, Copy)]
pub struct SurfaceCapabilitiesKHR {
    pub min_image_count: u32,
    pub max_image_count: u32,
    pub current_extent: vk::Extent2D,
    pub min_image_extent: vk::Extent2D,
    pub max_image_extent: vk::Extent2D,
    pub max_image_array_layers: u32,
    pub supported_transforms: u32,
    pub current_transform: i32,
    pub supported_composite_alpha: u32,
    pub supported_usage_flags: u32,
}

/// `VkSurfaceFormatKHR`
///
/// ```c
///   VkFormat format; VkColorSpaceKHR colorSpace;
/// ```
/// 布局：8 字节。
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SurfaceFormatKHR {
    pub format: i32,
    pub color_space: i32,
}

/// `vkGetPhysicalDeviceSurfaceSupportKHR(pd, queueFamilyIndex, surface, pSupported)`。
pub(crate) type PfnGetPhysicalDeviceSurfaceSupportKHR = unsafe extern "system" fn(
    ffi::PhysicalDeviceHandle,
    u32,
    ffi::SurfaceHandle,
    *mut u32,
) -> ffi::VkResult;
/// `vkGetPhysicalDeviceSurfaceCapabilitiesKHR(pd, surface, pCapabilities)`。
pub(crate) type PfnGetPhysicalDeviceSurfaceCapabilitiesKHR = unsafe extern "system" fn(
    ffi::PhysicalDeviceHandle,
    ffi::SurfaceHandle,
    *mut SurfaceCapabilitiesKHR,
) -> ffi::VkResult;
/// `vkGetPhysicalDeviceSurfaceFormatsKHR(pd, surface, pCount, pFormats)`。
pub(crate) type PfnGetPhysicalDeviceSurfaceFormatsKHR = unsafe extern "system" fn(
    ffi::PhysicalDeviceHandle,
    ffi::SurfaceHandle,
    *mut u32,
    *mut SurfaceFormatKHR,
) -> ffi::VkResult;
/// `vkGetPhysicalDeviceSurfacePresentModesKHR(pd, surface, pCount, pModes)`。
pub(crate) type PfnGetPhysicalDeviceSurfacePresentModesKHR = unsafe extern "system" fn(
    ffi::PhysicalDeviceHandle,
    ffi::SurfaceHandle,
    *mut u32,
    *mut i32,
) -> ffi::VkResult;
/// `vkCreateWin32SurfaceKHR(instance, pCreateInfo, pAllocator, pSurface)`。
type PfnCreateWin32SurfaceKHR = unsafe extern "system" fn(
    ffi::InstanceHandle,
    *const Win32SurfaceCreateInfoKHR,
    *const c_void,
    *mut ffi::SurfaceHandle,
) -> ffi::VkResult;
/// `vkDestroySurfaceKHR(instance, surface, pAllocator)`。
type PfnDestroySurfaceKHR =
    unsafe extern "system" fn(ffi::InstanceHandle, ffi::SurfaceHandle, *const c_void);

// Windows 上取模块句柄用作 `hinstance`（窗口自带 HINSTANCE 缺失时的兜底）。
// 只依赖 kernel32（与 `ffi.rs`/`loader.rs` 同一套做法，**不引第三方依赖**）。
#[cfg(windows)]
#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetModuleHandleW(name: *const u16) -> *mut c_void;
}

/// 一个已创建的 `VkSurfaceKHR`。`Drop` 保证 `vkDestroySurfaceKHR` 被调。
///
/// **生命周期契约**：`Surface` 比创建它的 [`ffi::Instance`] 先析构，
/// 否则 `Drop` 会在已销毁的实例上调用 `vkDestroySurfaceKHR`。
/// [`crate::windowed::WindowedRenderer`] 用字段声明顺序（device → surface → instance）保证。
pub struct Surface {
    handle: ffi::SurfaceHandle,
    /// 创建它的实例句柄 —— `Drop` 与「借用实例建设备」都要用。
    instance: ffi::InstanceHandle,
    platform: Platform,
    destroy: PfnDestroySurfaceKHR,
    get_capabilities: PfnGetPhysicalDeviceSurfaceCapabilitiesKHR,
    get_formats: PfnGetPhysicalDeviceSurfaceFormatsKHR,
    get_present_modes: PfnGetPhysicalDeviceSurfacePresentModesKHR,
    get_support: PfnGetPhysicalDeviceSurfaceSupportKHR,
    /// 实例的核心函数副本（`VkDevice::open_with_present` 要用它枚举物理设备）。
    core: ffi::CoreFns,
}

impl Surface {
    /// HAL 的不透明窗口句柄 → `VkSurfaceKHR`（Windows: `vkCreateWin32SurfaceKHR`）。
    ///
    /// 非 Windows 平台明确返回 [`GpuError::Unsupported`]（**不静默**：静默返回一个
    /// 假 surface 会让上层在第一次 `acquire` 时拿到莫名其妙的驱动错误）。
    pub fn create(instance: &ffi::Instance, window: RawWindowHandle) -> GpuResult<Surface> {
        if window.platform != Platform::Windows {
            return Err(GpuError::Unsupported(format!(
                "deer-vk 的 surface 目前只实现了 Windows（收到 {:?}）；\
                 对应扩展应为 {} —— 平台分支在 `surface.rs::Surface::create` 里补",
                window.platform,
                Surface::platform_extension(window.platform)
            )));
        }
        if window.handle == 0 {
            return Err(GpuError::BadWindowHandle);
        }

        // SAFETY: 符号名与规范一致，签名由本文件声明；`instance` 由调用方保证存活。
        let create_win32: PfnCreateWin32SurfaceKHR =
            unsafe { instance.proc("vkCreateWin32SurfaceKHR")? };
        // SAFETY: 同上（`vkDestroySurfaceKHR` 是实例级函数）。
        let destroy: PfnDestroySurfaceKHR = unsafe { instance.proc("vkDestroySurfaceKHR")? };
        // SAFETY: 同上（这四个查询都是实例级函数，签名逐字对应规范）。
        let get_capabilities: PfnGetPhysicalDeviceSurfaceCapabilitiesKHR =
            unsafe { instance.proc("vkGetPhysicalDeviceSurfaceCapabilitiesKHR")? };
        // SAFETY: 同上。
        let get_formats: PfnGetPhysicalDeviceSurfaceFormatsKHR =
            unsafe { instance.proc("vkGetPhysicalDeviceSurfaceFormatsKHR")? };
        // SAFETY: 同上。
        let get_present_modes: PfnGetPhysicalDeviceSurfacePresentModesKHR =
            unsafe { instance.proc("vkGetPhysicalDeviceSurfacePresentModesKHR")? };
        // SAFETY: 同上。
        let get_support: PfnGetPhysicalDeviceSurfaceSupportKHR =
            unsafe { instance.proc("vkGetPhysicalDeviceSurfaceSupportKHR")? };

        // `hinstance`：优先用窗口层给的（`RawWindowHandle::display` = GWLP_HINSTANCE），
        // 没有就退回本进程模块句柄 —— 两者都能让驱动定位窗口所属进程。
        #[cfg(windows)]
        let hinstance = if window.display != 0 {
            window.display as *mut c_void
        } else {
            // SAFETY: 传 null 表示「取本进程可执行模块」；返回值为 0 也可接受
            // （Win32 surface 的 hinstance 字段在多数驱动上不参与判定）。
            unsafe { GetModuleHandleW(ptr::null()) }
        };
        #[cfg(not(windows))]
        let hinstance = window.display as *mut c_void;

        let info = Win32SurfaceCreateInfoKHR {
            s_type: VK_STRUCTURE_TYPE_WIN32_SURFACE_CREATE_INFO_KHR,
            p_next: ptr::null(),
            flags: 0,
            hinstance,
            hwnd: window.handle as *mut c_void,
        };
        let mut handle: ffi::SurfaceHandle = ptr::null_mut();
        // SAFETY: 实例由调用方保证存活；`info` 在本栈帧存活且字段已初始化；
        // `handle` 是可写输出参数；hwnd 非 0（上面已判）。
        let rc = unsafe { create_win32(instance.handle(), &info, ptr::null(), &mut handle) };
        if rc != ffi::VK_SUCCESS {
            return Err(GpuError::Driver {
                code: rc,
                message: format!(
                    "vkCreateWin32SurfaceKHR 失败（{}）⇒ 窗口句柄可能已失效（HWND={:#x}）",
                    ffi::result_name(rc),
                    window.handle
                ),
            });
        }
        if handle.is_null() {
            return Err(GpuError::Driver {
                code: rc,
                message: "vkCreateWin32SurfaceKHR 返回成功但句柄为空".to_string(),
            });
        }

        Ok(Surface {
            handle,
            instance: instance.handle(),
            platform: window.platform,
            destroy,
            get_capabilities,
            get_formats,
            get_present_modes,
            get_support,
            core: instance.core_fns(),
        })
    }

    pub fn handle(&self) -> ffi::SurfaceHandle {
        self.handle
    }

    /// 平台 surface 扩展名（Windows → `"VK_KHR_win32_surface"`）。
    ///
    /// 这些字符串必须**逐字**正确：`vkCreateInstance` 拼错一个字符就是
    /// `VK_ERROR_EXTENSION_NOT_PRESENT`，而且报错不会告诉你是谁拼错了。
    pub fn platform_extension(platform: Platform) -> &'static str {
        match platform {
            Platform::Windows => WIN32_SURFACE_EXTENSION,
            Platform::X11 => XLIB_SURFACE_EXTENSION,
            Platform::Wayland => WAYLAND_SURFACE_EXTENSION,
            Platform::MacOs => METAL_SURFACE_EXTENSION,
        }
    }

    /// 本 surface 所在的平台（诊断用）。
    pub fn platform(&self) -> Platform {
        self.platform
    }

    /// 创建它的实例句柄（`pub(crate)`：`VkDevice::open_with_present` 要借用它）。
    pub(crate) fn instance_handle(&self) -> ffi::InstanceHandle {
        self.instance
    }

    /// 实例核心函数副本（同上）。
    pub(crate) fn core_fns(&self) -> ffi::CoreFns {
        self.core
    }

    /// 呈现支持查询函数（同上；放进后台线程用）。
    pub(crate) fn support_fn(&self) -> PfnGetPhysicalDeviceSurfaceSupportKHR {
        self.get_support
    }

    /// 表面能力（交换链配置的唯一依据）。
    pub(crate) fn capabilities(
        &self,
        pd: ffi::PhysicalDeviceHandle,
    ) -> GpuResult<SurfaceCapabilitiesKHR> {
        let mut caps = std::mem::MaybeUninit::<SurfaceCapabilitiesKHR>::uninit();
        // SAFETY: `pd` 与 surface 由调用方保证来自同一存活实例（本结构记录了 instance）；
        // 该函数完整写入结构体。
        let rc = unsafe { (self.get_capabilities)(pd, self.handle, caps.as_mut_ptr()) };
        if rc != ffi::VK_SUCCESS {
            return Err(GpuError::Driver {
                code: rc,
                message: format!(
                    "vkGetPhysicalDeviceSurfaceCapabilitiesKHR 失败：{}",
                    ffi::result_name(rc)
                ),
            });
        }
        // SAFETY: 上面的调用返回成功 ⇒ 结构体已被完整写入。
        Ok(unsafe { caps.assume_init() })
    }

    /// 表面支持的格式/色彩空间列表（两阶段查询）。
    pub(crate) fn formats(&self, pd: ffi::PhysicalDeviceHandle) -> GpuResult<Vec<SurfaceFormatKHR>> {
        let mut count = 0u32;
        // SAFETY: 传 null 数组 = 规定的「只查数量」用法。
        let rc = unsafe { (self.get_formats)(pd, self.handle, &mut count, ptr::null_mut()) };
        if rc != ffi::VK_SUCCESS && rc != ffi::VK_INCOMPLETE {
            return Err(GpuError::Driver {
                code: rc,
                message: format!(
                    "vkGetPhysicalDeviceSurfaceFormatsKHR（查数量）失败：{}",
                    ffi::result_name(rc)
                ),
            });
        }
        if count == 0 {
            return Err(GpuError::Unsupported(
                "该 surface 报告 0 个支持格式（按规范不可能；驱动/窗口有问题）".to_string(),
            ));
        }
        let mut formats = vec![
            SurfaceFormatKHR {
                format: 0,
                color_space: 0
            };
            count as usize
        ];
        // SAFETY: `formats` 容量与 `count` 一致，驱动最多写入 `count` 项。
        let rc = unsafe { (self.get_formats)(pd, self.handle, &mut count, formats.as_mut_ptr()) };
        if rc != ffi::VK_SUCCESS && rc != ffi::VK_INCOMPLETE {
            return Err(GpuError::Driver {
                code: rc,
                message: format!(
                    "vkGetPhysicalDeviceSurfaceFormatsKHR 失败：{}",
                    ffi::result_name(rc)
                ),
            });
        }
        formats.truncate(count as usize);
        Ok(formats)
    }

    /// 表面支持的呈现模式列表（两阶段查询）。
    pub(crate) fn present_modes(&self, pd: ffi::PhysicalDeviceHandle) -> GpuResult<Vec<i32>> {
        let mut count = 0u32;
        // SAFETY: 传 null 数组 = 规定的「只查数量」用法。
        let rc = unsafe { (self.get_present_modes)(pd, self.handle, &mut count, ptr::null_mut()) };
        if rc != ffi::VK_SUCCESS && rc != ffi::VK_INCOMPLETE {
            return Err(GpuError::Driver {
                code: rc,
                message: format!(
                    "vkGetPhysicalDeviceSurfacePresentModesKHR（查数量）失败：{}",
                    ffi::result_name(rc)
                ),
            });
        }
        if count == 0 {
            return Err(GpuError::Unsupported(
                "该 surface 报告 0 个呈现模式（FIFO 按规范必须支持；驱动/窗口有问题）".to_string(),
            ));
        }
        let mut modes = vec![0i32; count as usize];
        // SAFETY: `modes` 容量与 `count` 一致。
        let rc = unsafe { (self.get_present_modes)(pd, self.handle, &mut count, modes.as_mut_ptr()) };
        if rc != ffi::VK_SUCCESS && rc != ffi::VK_INCOMPLETE {
            return Err(GpuError::Driver {
                code: rc,
                message: format!(
                    "vkGetPhysicalDeviceSurfacePresentModesKHR 失败：{}",
                    ffi::result_name(rc)
                ),
            });
        }
        modes.truncate(count as usize);
        Ok(modes)
    }

    /// 某队列族能否向本 surface 呈现（这是选队列族的硬条件）。
    ///
    /// `pd` 必须是**同一个实例**枚举出的物理设备（见模块文档的约束 1）。
    /// 公开的理由：诊断/测试要能直接问「这张卡的这个队列族能不能往这个窗口呈现」。
    ///
    /// 关于 `clippy::not_unsafe_ptr_arg_deref`：签名是**冻死的公开 API**
    /// （`fn supports_present(&self, pd: PhysicalDeviceHandle, family: u32) -> GpuResult<bool>`），
    /// 不能改成 `unsafe fn`。`pd` 是 Vulkan 的不透明句柄，有效性是调用方的契约 ——
    /// 本文件的文档已把它写清楚；这与本 crate 其他「收裸句柄的公开函数」是同一套约定。
    #[allow(clippy::not_unsafe_ptr_arg_deref)]
    pub fn supports_present(
        &self,
        pd: ffi::PhysicalDeviceHandle,
        queue_family_index: u32,
    ) -> GpuResult<bool> {
        let mut supported: u32 = vk::VK_FALSE;
        // SAFETY: `pd` 与本 surface 由调用方保证来自同一存活实例；`supported` 是可写输出。
        let rc = unsafe { (self.get_support)(pd, queue_family_index, self.handle, &mut supported) };
        if rc != ffi::VK_SUCCESS {
            return Err(GpuError::Driver {
                code: rc,
                message: format!(
                    "vkGetPhysicalDeviceSurfaceSupportKHR 失败：{}",
                    ffi::result_name(rc)
                ),
            });
        }
        Ok(supported == vk::VK_TRUE)
    }
}

impl Drop for Surface {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            // SAFETY: 句柄由本结构创建、未销毁；`self.instance` 由调用方按文档保证
            // 比本结构活得久（`WindowedRenderer` 的字段顺序把 instance 放在最后）。
            unsafe { (self.destroy)(self.instance, self.handle, ptr::null()) };
            self.handle = ptr::null_mut();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn surface_capabilities_layout() {
        // 5 个 u32 + 3 个 Extent2D + 2 个 u32 = 20 + 24 + 8 = 52
        assert_eq!(std::mem::size_of::<SurfaceCapabilitiesKHR>(), 52);
        assert_eq!(std::mem::align_of::<SurfaceCapabilitiesKHR>(), 4);
        assert_eq!(
            std::mem::offset_of!(SurfaceCapabilitiesKHR, current_extent),
            8,
            "currentExtent 在 minImageCount/maxImageCount（4+4）之后"
        );
        assert_eq!(std::mem::offset_of!(SurfaceCapabilitiesKHR, max_image_array_layers), 32);
        assert_eq!(std::mem::offset_of!(SurfaceCapabilitiesKHR, supported_usage_flags), 48);
    }

    #[test]
    fn win32_surface_create_info_layout() {
        // sType(0)+pad(4) pNext(8) flags(16)+pad(4) hinstance(24) hwnd(32) ⇒ 40
        assert_eq!(std::mem::size_of::<Win32SurfaceCreateInfoKHR>(), 40);
        assert_eq!(std::mem::offset_of!(Win32SurfaceCreateInfoKHR, p_next), 8);
        assert_eq!(std::mem::offset_of!(Win32SurfaceCreateInfoKHR, hinstance), 24);
        assert_eq!(std::mem::offset_of!(Win32SurfaceCreateInfoKHR, hwnd), 32);
    }

    #[test]
    fn surface_format_layout() {
        assert_eq!(std::mem::size_of::<SurfaceFormatKHR>(), 8);
    }

    #[test]
    fn extension_names_are_exact() {
        // 拼错一个字符 ⇒ VK_ERROR_EXTENSION_NOT_PRESENT，且报错不会指出是谁拼错了
        assert_eq!(SURFACE_EXTENSION, "VK_KHR_surface");
        assert_eq!(
            Surface::platform_extension(Platform::Windows),
            "VK_KHR_win32_surface"
        );
        assert_eq!(
            Surface::platform_extension(Platform::X11),
            "VK_KHR_xlib_surface"
        );
        assert_eq!(
            Surface::platform_extension(Platform::Wayland),
            "VK_KHR_wayland_surface"
        );
        assert_eq!(
            Surface::platform_extension(Platform::MacOs),
            "VK_EXT_metal_surface"
        );
    }

    #[test]
    fn non_windows_platform_reports_unsupported_not_silent() {
        // 本机（Windows）上直接喂一个「非 Windows」的句柄 ⇒ 必须明确报 Unsupported，
        // 而不是返回一个假 surface。
        if let Ok(instance) = ffi::Instance::create() {
            let bad = RawWindowHandle {
                platform: Platform::Wayland,
                handle: 1,
                display: 0,
            };
            match Surface::create(&instance, bad) {
                Err(GpuError::Unsupported(msg)) => {
                    assert!(msg.contains("Wayland"), "错误信息要指出平台：{msg}");
                }
                Err(other) => panic!("期望 Unsupported，实际 {other:?}"),
                Ok(_) => panic!("期望 Unsupported，实际拿到 Ok(Surface)"),
            }
        } else {
            println!("跳过：本机没有 Vulkan loader");
        }
    }

    #[test]
    fn zero_hwnd_is_rejected_before_touching_driver() {
        if let Ok(instance) = ffi::Instance::create() {
            let zero = RawWindowHandle {
                platform: Platform::Windows,
                handle: 0,
                display: 0,
            };
            assert_eq!(
                Surface::create(&instance, zero).err(),
                Some(GpuError::BadWindowHandle),
                "HWND=0 必须在调驱动之前被拒"
            );
        } else {
            println!("跳过：本机没有 Vulkan loader");
        }
    }
}
