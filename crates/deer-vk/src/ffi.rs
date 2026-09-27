//! Vulkan FFI：**自己声明符号 + 运行时动态加载**（不引 `ash` / `vulkano` / `wgpu`）。
//!
//! ## 为什么是动态加载，而不是 `#[link(name = "vulkan-1")]`
//!
//! 实测（本机）：`#[link(name = "vulkan-1")]` 会让**链接期**失败 ——
//! `LNK1181: 无法打开输入文件 "vulkan-1.lib"`。原因是系统只带 `vulkan-1.dll`
//! （loader），而**导入库 `.lib` 属于 Vulkan SDK**，本机未安装。
//!
//! 所以正确的「不依赖 SDK」做法是：用 `LoadLibraryW` 打开 `vulkan-1.dll`，
//! 再用 `GetProcAddress` 取函数指针。这样：
//! - **零链接期依赖**（只用到 `kernel32`，它永远在）；
//! - **不需要 SDK**（结构体布局与符号名都在本模块里手写）；
//! - **可降级**：loader 缺失时返回 `GpuError`，上层可回退到 CPU 后端。
//!
//! ## 边界
//!
//! - 只声明被用到的符号；加一个功能 = 加一个函数指针类型 + 一个 `resolve`；
//! - 结构体按 Vulkan ABI 布局（`#[repr(C)]`），并用**大小/偏移断言**钉住；
//! - 所有 `unsafe` 集中在本模块；`Instance` 用 `Drop` 保证 `vkDestroyInstance` 必被调。

use std::ffi::{CStr, c_char, c_void};
use std::ptr;
use std::sync::atomic::{AtomicUsize, Ordering};

use deer_gpu::{GpuError, GpuResult};

/// 设备级 FFI 的类型集合 —— 冻死 API 里把它们写作 `crate::ffi::vk::*`。
///
/// 它把 [`crate::ffi_dev`]（设备级结构体/函数指针/句柄）与本模块新增的
/// [`SurfaceHandle`]（实例级 surface 句柄）收到同一条路径下，于是
/// `crate::ffi::vk::SurfaceHandle` / `ImageHandle` / `QueueHandle` / `SemaphoreHandle`
/// / `PhysicalDeviceHandle` 全都能逐字解析 —— 领队的 HAL 接线按冻死签名写，不该在这里踩空。
pub mod vk {
    pub use super::SurfaceHandle;
    pub use crate::ffi_dev::*;
}

pub type InstanceHandle = *mut c_void;
pub type PhysicalDeviceHandle = *mut c_void;
/// `VkSurfaceKHR`（M2b）。它是**实例级**对象：销毁时必须用创建它的那个实例。
pub type SurfaceHandle = *mut c_void;
pub type VkResult = i32;

pub const VK_SUCCESS: VkResult = 0;
pub const VK_NOT_READY: VkResult = 1;
pub const VK_TIMEOUT: VkResult = 2;
pub const VK_INCOMPLETE: VkResult = 5;

/// `VK_ERROR_OUT_OF_DATE_KHR` —— 交换链与 surface 不再匹配（**正常路径**，要重建）。
pub const VK_ERROR_OUT_OF_DATE_KHR: VkResult = -1_000_001_004;
/// `VK_SUBOPTIMAL_KHR` —— 还能用但已不最优（**不是**错误，也不能当成功吞掉）。
pub const VK_SUBOPTIMAL_KHR: VkResult = 1_000_001_003;
/// `VK_ERROR_SURFACE_LOST_KHR` —— surface 失效（窗口被销毁等）。
pub const VK_ERROR_SURFACE_LOST_KHR: VkResult = -1_000_000_000;

pub const VK_STRUCTURE_TYPE_APPLICATION_INFO: i32 = 0;
pub const VK_STRUCTURE_TYPE_INSTANCE_CREATE_INFO: i32 = 1;

/// `VkPhysicalDeviceType`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PhysicalDeviceType {
    Other,
    IntegratedGpu,
    DiscreteGpu,
    VirtualGpu,
    Cpu,
}

impl PhysicalDeviceType {
    pub fn from_raw(v: i32) -> PhysicalDeviceType {
        match v {
            1 => PhysicalDeviceType::IntegratedGpu,
            2 => PhysicalDeviceType::DiscreteGpu,
            3 => PhysicalDeviceType::VirtualGpu,
            4 => PhysicalDeviceType::Cpu,
            _ => PhysicalDeviceType::Other,
        }
    }
}

pub const MAX_PHYSICAL_DEVICE_NAME_SIZE: usize = 256;
pub const UUID_SIZE: usize = 16;
pub const MAX_EXTENSION_NAME_SIZE: usize = 256;
pub const MAX_DESCRIPTION_SIZE: usize = 256;

/// Khronos 官方校验层的名字。装 Vulkan SDK 后可用。
pub const VALIDATION_LAYER: &str = "VK_LAYER_KHRONOS_validation";

/// `VkPhysicalDeviceLimits`（字段顺序按规范；本模块只真正读前几个）。
#[repr(C)]
#[derive(Clone, Copy)]
pub struct PhysicalDeviceLimits {
    pub max_image_dimension_1d: u32,
    pub max_image_dimension_2d: u32,
    pub max_image_dimension_3d: u32,
    pub max_image_dimension_cube: u32,
    pub max_image_array_layers: u32,
    pub max_texel_buffer_elements: u32,
    pub max_uniform_buffer_range: u32,
    pub max_storage_buffer_range: u32,
    pub max_push_constants_size: u32,
    pub max_memory_allocation_count: u32,
    pub max_sampler_allocation_count: u32,
    pub buffer_image_granularity: u64,
    pub sparse_address_space_size: u64,
    pub max_bound_descriptor_sets: u32,
    pub max_per_stage_descriptor_samplers: u32,
    pub max_per_stage_descriptor_uniform_buffers: u32,
    pub max_per_stage_descriptor_storage_buffers: u32,
    pub max_per_stage_descriptor_sampled_images: u32,
    pub max_per_stage_descriptor_storage_images: u32,
    pub max_per_stage_descriptor_input_attachments: u32,
    pub max_per_stage_resources: u32,
    pub max_descriptor_set_samplers: u32,
    pub max_descriptor_set_uniform_buffers: u32,
    pub max_descriptor_set_uniform_buffers_dynamic: u32,
    pub max_descriptor_set_storage_buffers: u32,
    pub max_descriptor_set_storage_buffers_dynamic: u32,
    pub max_descriptor_set_sampled_images: u32,
    pub max_descriptor_set_storage_images: u32,
    pub max_descriptor_set_input_attachments: u32,
    pub max_vertex_input_attributes: u32,
    pub max_vertex_input_bindings: u32,
    pub max_vertex_input_attribute_offset: u32,
    pub max_vertex_input_binding_stride: u32,
    pub max_vertex_output_components: u32,
    pub max_tessellation_generation_level: u32,
    pub max_tessellation_patch_size: u32,
    pub max_tessellation_control_per_vertex_input_components: u32,
    pub max_tessellation_control_per_vertex_output_components: u32,
    pub max_tessellation_control_per_patch_output_components: u32,
    pub max_tessellation_control_total_output_components: u32,
    pub max_tessellation_evaluation_input_components: u32,
    pub max_tessellation_evaluation_output_components: u32,
    pub max_geometry_shader_invocations: u32,
    pub max_geometry_input_components: u32,
    pub max_geometry_output_components: u32,
    pub max_geometry_output_vertices: u32,
    pub max_geometry_total_output_components: u32,
    pub max_fragment_input_components: u32,
    pub max_fragment_output_attachments: u32,
    pub max_fragment_dual_source_attachments: u32,
    pub max_fragment_combined_output_resources: u32,
    pub max_compute_shared_memory_size: u32,
    pub max_compute_work_group_count: [u32; 3],
    pub max_compute_work_group_invocations: u32,
    pub max_compute_work_group_size: [u32; 3],
    pub sub_pixel_precision_bits: u32,
    pub sub_texel_precision_bits: u32,
    pub mipmap_precision_bits: u32,
    pub max_draw_indexed_index_value: u32,
    pub max_draw_indirect_count: u32,
    pub max_sampler_lod_bias: f32,
    pub max_sampler_anisotropy: f32,
    pub max_viewport_dimensions: [u32; 2],
    pub viewport_bounds_range: [f32; 2],
    pub viewport_sub_pixel_bits: u32,
    pub min_memory_map_alignment: usize,
    pub min_texel_buffer_offset_alignment: u64,
    pub min_uniform_buffer_offset_alignment: u64,
    pub min_storage_buffer_offset_alignment: u64,
    pub min_texel_offset: i32,
    pub max_texel_offset: u32,
    pub min_texel_gather_offset: i32,
    pub max_texel_gather_offset: u32,
    pub min_interpolation_offset: f32,
    pub max_interpolation_offset: f32,
    pub sub_pixel_interpolation_offset_bits: u32,
    pub max_framebuffer_width: u32,
    pub max_framebuffer_height: u32,
    pub max_framebuffer_layers: u32,
    pub framebuffer_color_sample_counts: u32,
    pub framebuffer_depth_sample_counts: u32,
    pub framebuffer_stencil_sample_counts: u32,
    pub framebuffer_no_attachments_sample_counts: u32,
    pub max_color_attachments: u32,
    pub sampled_image_color_sample_counts: u32,
    pub sampled_image_integer_sample_counts: u32,
    pub sampled_image_depth_sample_counts: u32,
    pub sampled_image_stencil_sample_counts: u32,
    pub storage_image_sample_counts: u32,
    pub max_sample_mask_words: u32,
    pub timestamp_compute_and_graphics: u32,
    pub timestamp_period: f32,
    pub max_clip_distances: u32,
    pub max_cull_distances: u32,
    pub max_combined_clip_and_cull_distances: u32,
    pub discrete_queue_priorities: u32,
    pub point_size_range: [f32; 2],
    pub line_width_range: [f32; 2],
    pub point_size_granularity: f32,
    pub line_width_granularity: f32,
    pub strict_lines: u32,
    pub standard_sample_locations: u32,
    pub optimal_buffer_copy_offset_alignment: u64,
    pub optimal_buffer_copy_row_pitch_alignment: u64,
    pub non_coherent_atom_size: u64,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct PhysicalDeviceSparseProperties {
    pub residency_standard_2d_block_shape: u32,
    pub residency_standard_2d_multisample_block_shape: u32,
    pub residency_standard_3d_block_shape: u32,
    pub residency_aligned_mip_size: u32,
    pub residency_non_resident_strict: u32,
}

/// `VkPhysicalDeviceProperties`
#[repr(C)]
#[derive(Clone, Copy)]
pub struct PhysicalDeviceProperties {
    pub api_version: u32,
    pub driver_version: u32,
    pub vendor_id: u32,
    pub device_id: u32,
    pub device_type: i32,
    pub device_name: [c_char; MAX_PHYSICAL_DEVICE_NAME_SIZE],
    pub pipeline_cache_uuid: [u8; UUID_SIZE],
    pub limits: PhysicalDeviceLimits,
    pub sparse_properties: PhysicalDeviceSparseProperties,
}

/// `VkApplicationInfo`
#[repr(C)]
struct ApplicationInfo {
    s_type: i32,
    p_next: *const c_void,
    p_application_name: *const c_char,
    application_version: u32,
    p_engine_name: *const c_char,
    engine_version: u32,
    api_version: u32,
}

/// `VkInstanceCreateInfo`
#[repr(C)]
struct InstanceCreateInfo {
    s_type: i32,
    p_next: *const c_void,
    flags: u32,
    p_application_info: *const ApplicationInfo,
    enabled_layer_count: u32,
    pp_enabled_layer_names: *const *const c_char,
    enabled_extension_count: u32,
    pp_enabled_extension_names: *const *const c_char,
}

// ── 函数指针类型（Vulkan 1.0 全局导出符号的签名） ────────────────────────────

type PfnCreateInstance =
    unsafe extern "system" fn(*const InstanceCreateInfo, *const c_void, *mut InstanceHandle) -> VkResult;
type PfnDestroyInstance = unsafe extern "system" fn(InstanceHandle, *const c_void);
/// `vkEnumeratePhysicalDevices` —— 做成 `pub(crate)`，因为「借用他人实例」的设备创建路径
/// （`VkDevice::open_with_present`）需要在不拥有实例的前提下调用它。见 [`CoreFns`]。
pub(crate) type PfnEnumeratePhysicalDevices =
    unsafe extern "system" fn(InstanceHandle, *mut u32, *mut PhysicalDeviceHandle) -> VkResult;
/// `vkGetPhysicalDeviceProperties` —— 理由同上。
pub(crate) type PfnGetPhysicalDeviceProperties =
    unsafe extern "system" fn(PhysicalDeviceHandle, *mut PhysicalDeviceProperties);
/// `vkEnumerateDeviceExtensionProperties`（查**设备**扩展是否可用）。
pub(crate) type PfnEnumerateDeviceExtensionProperties =
    unsafe extern "system" fn(PhysicalDeviceHandle, *const c_char, *mut u32, *mut ExtensionProperties) -> VkResult;
type PfnEnumerateInstanceLayerProperties = unsafe extern "system" fn(*mut u32, *mut LayerProperties) -> VkResult;
type PfnEnumerateInstanceExtensionProperties =
    unsafe extern "system" fn(*const c_char, *mut u32, *mut ExtensionProperties) -> VkResult;
type PfnGetInstanceProcAddr =
    unsafe extern "system" fn(InstanceHandle, *const c_char) -> FARPROC;

/// 从 `vulkan-1.dll` 取到的函数表。
struct Fns {
    create_instance: PfnCreateInstance,
    destroy_instance: PfnDestroyInstance,
    enumerate_physical_devices: PfnEnumeratePhysicalDevices,
    get_physical_device_properties: PfnGetPhysicalDeviceProperties,
    enumerate_instance_layer_properties: PfnEnumerateInstanceLayerProperties,
    enumerate_instance_extension_properties: PfnEnumerateInstanceExtensionProperties,
    get_instance_proc_addr: PfnGetInstanceProcAddr,
}

// ── 动态加载（只依赖 kernel32） ──────────────────────────────────────────────

type Module = *mut c_void;
pub type FARPROC = *mut c_void;

#[link(name = "kernel32")]
unsafe extern "system" {
    fn LoadLibraryW(name: *const u16) -> Module;
    fn GetProcAddress(module: Module, name: *const c_char) -> FARPROC;
    fn FreeLibrary(module: Module) -> i32;
}

/// 已加载的 loader。
struct Loader {
    module: Module,
    fns: Fns,
}

impl Loader {
    fn load() -> GpuResult<Loader> {
        // "vulkan-1.dll" 转成 UTF-16 + NUL
        let dll: Vec<u16> = "vulkan-1.dll\0".encode_utf16().collect();
        // SAFETY: 传入以 NUL 结尾的 UTF-16 字符串；失败返回空句柄（我们会检查）。
        let module = unsafe { LoadLibraryW(dll.as_ptr()) };
        if module.is_null() {
            return Err(GpuError::Unsupported(
                "找不到 vulkan-1.dll（本机没有 Vulkan loader）⇒ 请改用 CPU 后端".to_string(),
            ));
        }

        // 取符号辅助：名字是 ASCII，加 NUL 后传 GetProcAddress
        fn sym(module: Module, name: &str) -> GpuResult<FARPROC> {
            let mut cname = Vec::with_capacity(name.len() + 1);
            cname.extend_from_slice(name.as_bytes());
            cname.push(0);
            // SAFETY: `cname` 以 NUL 结尾且在调用期间存活；GetProcAddress 只读它。
            let p = unsafe { GetProcAddress(module, cname.as_ptr() as *const c_char) };
            if p.is_null() {
                return Err(GpuError::Unsupported(format!(
                    "vulkan-1.dll 缺少符号 {name}（loader 版本过旧？）"
                )));
            }
            Ok(p)
        }

        // SAFETY: 每个符号都用 `sym` 查到非空地址后，按 Vulkan 规范声明的签名
        // transmute 成函数指针。符号名与规范一致，签名由我们自己声明并由
        // 布局断言（见文件末尾测试）兜住。
        let create_instance = unsafe {
            std::mem::transmute::<FARPROC, PfnCreateInstance>(sym(module, "vkCreateInstance")?)
        };
        let destroy_instance = unsafe {
            std::mem::transmute::<FARPROC, PfnDestroyInstance>(sym(module, "vkDestroyInstance")?)
        };
        let enumerate_physical_devices = unsafe {
            std::mem::transmute::<FARPROC, PfnEnumeratePhysicalDevices>(sym(
                module,
                "vkEnumeratePhysicalDevices",
            )?)
        };
        let get_physical_device_properties = unsafe {
            std::mem::transmute::<FARPROC, PfnGetPhysicalDeviceProperties>(sym(
                module,
                "vkGetPhysicalDeviceProperties",
            )?)
        };
        let enumerate_instance_layer_properties = unsafe {
            std::mem::transmute::<FARPROC, PfnEnumerateInstanceLayerProperties>(sym(
                module,
                "vkEnumerateInstanceLayerProperties",
            )?)
        };
        let enumerate_instance_extension_properties = unsafe {
            std::mem::transmute::<FARPROC, PfnEnumerateInstanceExtensionProperties>(sym(
                module,
                "vkEnumerateInstanceExtensionProperties",
            )?)
        };
        let get_instance_proc_addr = unsafe {
            std::mem::transmute::<FARPROC, PfnGetInstanceProcAddr>(sym(
                module,
                "vkGetInstanceProcAddr",
            )?)
        };

        Ok(Loader {
            module,
            fns: Fns {
                create_instance,
                destroy_instance,
                enumerate_physical_devices,
                get_physical_device_properties,
                enumerate_instance_layer_properties,
                enumerate_instance_extension_properties,
                get_instance_proc_addr,
            },
        })
    }

    /// 取一个符号并转成函数指针（**loader 模块级**符号，如 `vkEnumerateInstanceLayerProperties`）。
    ///
    /// 实例级/层提供的扩展函数请用 [`Loader::inst_sym`] —— 那条路必须走
    /// `vkGetInstanceProcAddr`。
    ///
    /// # Safety
    /// `T` 必须是该符号**真实签名**对应的函数指针类型。
    #[allow(dead_code)]
    unsafe fn sym<T: Copy>(&self, name: &str) -> GpuResult<T> {
        let mut cname = Vec::with_capacity(name.len() + 1);
        cname.extend_from_slice(name.as_bytes());
        cname.push(0);
        // SAFETY: `cname` 以 NUL 结尾且在调用期间存活；GetProcAddress 只读它。
        let p = unsafe { GetProcAddress(self.module, cname.as_ptr() as *const c_char) };
        if p.is_null() {
            return Err(GpuError::Unsupported(format!(
                "vulkan-1.dll 缺少符号 {name}（扩展未启用或 loader 过旧？）"
            )));
        }
        // SAFETY: 调用方保证 `T` 与符号真实签名一致。
        Ok(unsafe { std::mem::transmute_copy::<FARPROC, T>(&p) })
    }

    /// 取一个**实例级**符号（含层提供的扩展，如 `vkCreateDebugUtilsMessengerEXT`）。
    ///
    /// ⚠️ 这里**必须**用 `vkGetInstanceProcAddr`，不能直接对模块 `GetProcAddress`：
    /// - 层提供的扩展函数不一定出现在 `vulkan-1.dll` 的导出表里
    ///   （即使字符串在文件里，`GetProcAddress` 也可能拿到空）；
    /// - `vkGetInstanceProcAddr` 会走 loader 的调度链，把层的实现串进来。
    ///
    /// 实测踩过：用 `GetProcAddress` 取 `vkCreateDebugUtilsMessengerEXT` 得到空指针，
    /// 于是「启用校验层」在最后一步失败。
    ///
    /// # Safety
    /// `T` 必须是该符号**真实签名**对应的函数指针类型。
    unsafe fn inst_sym<T: Copy>(
        &self,
        instance: InstanceHandle,
        name: &str,
    ) -> GpuResult<T> {
        let mut cname = Vec::with_capacity(name.len() + 1);
        cname.extend_from_slice(name.as_bytes());
        cname.push(0);
        // SAFETY: `cname` 以 NUL 结尾且在调用期间存活；instance 是本模块刚创建的有效句柄。
        let p = unsafe { (self.fns.get_instance_proc_addr)(instance, cname.as_ptr() as *const c_char) };
        if p.is_null() {
            return Err(GpuError::Unsupported(format!(
                "vkGetInstanceProcAddr 取不到 {name}（层未启用该扩展？）"
            )));
        }
        // SAFETY: 调用方保证 `T` 与符号真实签名一致。
        Ok(unsafe { std::mem::transmute_copy::<FARPROC, T>(&p) })
    }

    /// 列出本机已注册的**实例层**名字。
    ///
    /// 用途：`create_with_validation` 据此判断校验层是否可用（**不靠猜**）。
    fn available_layers(&self) -> Vec<String> {
        let mut count = 0u32;
        // SAFETY: 传 null 数组 = Vulkan 规定的「只查数量」用法。
        let rc = unsafe { (self.fns.enumerate_instance_layer_properties)(&mut count, ptr::null_mut()) };
        if rc != VK_SUCCESS || count == 0 {
            return Vec::new();
        }
        let mut props = vec![
            LayerProperties {
                layer_name: [0; MAX_EXTENSION_NAME_SIZE],
                spec_version: 0,
                implementation_version: 0,
                description: [0; MAX_DESCRIPTION_SIZE],
            };
            count as usize
        ];
        // SAFETY: `props` 容量与 `count` 一致，驱动最多写入 `count` 项。
        let rc = unsafe {
            (self.fns.enumerate_instance_layer_properties)(&mut count, props.as_mut_ptr())
        };
        if rc != VK_SUCCESS && rc != VK_INCOMPLETE {
            return Vec::new();
        }
        props.truncate(count as usize);
        props
            .into_iter()
            .map(|p| {
                // SAFETY: `layer_name` 是 NUL 结尾的 C 字符串（Vulkan 保证）。
                unsafe { CStr::from_ptr(p.layer_name.as_ptr()) }
                    .to_string_lossy()
                    .into_owned()
            })
            .collect()
    }

    /// 列出本机已注册的**实例扩展**名字（`pLayerName = NULL` ⇒ loader + ICD 提供的那些）。
    ///
    /// 用途：`create_with_extensions` 据此判断扩展是否真的可用（**不靠猜、不静默降级**）。
    /// 注意它**不枚举层提供的扩展**（例如校验层的 `VK_EXT_debug_utils` 是否可见取决于
    /// loader 自己是否导出该扩展 —— 本机实测可见）。
    fn available_extensions(&self) -> Vec<String> {
        let mut count = 0u32;
        // SAFETY: 传 null 数组 = Vulkan 规定的「只查数量」用法；pLayerName 传 null 表示
        // 查「实现（loader + ICD）提供」的扩展。
        let rc = unsafe {
            (self.fns.enumerate_instance_extension_properties)(ptr::null(), &mut count, ptr::null_mut())
        };
        if rc != VK_SUCCESS || count == 0 {
            return Vec::new();
        }
        let mut props = vec![
            ExtensionProperties {
                extension_name: [0; MAX_EXTENSION_NAME_SIZE],
                spec_version: 0,
            };
            count as usize
        ];
        // SAFETY: `props` 容量与 `count` 一致，驱动最多写入 `count` 项。
        let rc = unsafe {
            (self.fns.enumerate_instance_extension_properties)(
                ptr::null(),
                &mut count,
                props.as_mut_ptr(),
            )
        };
        if rc != VK_SUCCESS && rc != VK_INCOMPLETE {
            return Vec::new();
        }
        props.truncate(count as usize);
        props
            .iter()
            .map(|p| {
                // SAFETY: `extensionName` 是 NUL 结尾的 C 字符串（Vulkan 保证），
                // 上限为 MAX_EXTENSION_NAME_SIZE。
                unsafe { CStr::from_ptr(p.extension_name.as_ptr()) }
                    .to_string_lossy()
                    .into_owned()
            })
            .collect()
    }
}

impl Drop for Loader {
    fn drop(&mut self) {
        if !self.module.is_null() {
            // SAFETY: 模块由 `LoadLibraryW` 加载，且只在 `Drop` 里释放一次。
            unsafe { FreeLibrary(self.module) };
            self.module = ptr::null_mut();
        }
    }
}

/// `VkLayerProperties`
#[repr(C)]
#[derive(Clone, Copy)]
pub struct LayerProperties {
    pub layer_name: [c_char; MAX_EXTENSION_NAME_SIZE],
    pub spec_version: u32,
    pub implementation_version: u32,
    pub description: [c_char; MAX_DESCRIPTION_SIZE],
}

/// `VkExtensionProperties`
///
/// 布局：`char extensionName[256]` + `uint32_t specVersion` ⇒ 260 字节，
/// 但 `align_of == 4`（只有 `char` 数组与 `u32`），**不补齐到 8**。
#[repr(C)]
#[derive(Clone, Copy)]
pub struct ExtensionProperties {
    pub extension_name: [c_char; MAX_EXTENSION_NAME_SIZE],
    pub spec_version: u32,
}

/// `VK_EXT_debug_utils`（校验层的诊断消息通道）。
pub const DEBUG_UTILS_EXTENSION: &str = "VK_EXT_debug_utils";

/// 实例级核心函数的一份**可复制副本**。
///
/// 存在的理由：`VkDevice::open_with_present` 要在**不拥有实例**的前提下枚举物理设备
/// （实例归 `WindowedRenderer` 所有）。函数指针是 `Send` 的普通值，句柄用 `usize`
/// 传递，于是「借用实例」不需要把整个 [`Instance`]（含 `Loader`，非 `Send`）搬进后台线程。
///
/// 生命周期契约：用它的句柄必须仍然存活。由 `WindowedRenderer` 的字段顺序保证
/// （device → surface → instance，即 device 先析构）。
#[derive(Clone, Copy)]
pub(crate) struct CoreFns {
    pub(crate) enumerate_physical_devices: PfnEnumeratePhysicalDevices,
    pub(crate) get_physical_device_properties: PfnGetPhysicalDeviceProperties,
}

impl CoreFns {
    /// 枚举某实例的物理设备（两阶段：先问数量，再取列表）。
    ///
    /// # Safety
    /// `instance` 必须是 `vkCreateInstance` 创建、且**当前仍然存活**的实例句柄。
    pub(crate) unsafe fn enumerate_physical_devices(
        &self,
        instance: InstanceHandle,
    ) -> GpuResult<Vec<PhysicalDeviceHandle>> {
        let mut count: u32 = 0;
        // SAFETY: 传 null 作为设备数组 = Vulkan 规定的「只查数量」用法；`instance` 由调用方保证有效。
        let rc = unsafe { (self.enumerate_physical_devices)(instance, &mut count, ptr::null_mut()) };
        if rc != VK_SUCCESS && rc != VK_INCOMPLETE {
            return Err(GpuError::Driver {
                code: rc,
                message: format!("vkEnumeratePhysicalDevices（查数量）失败：{}", result_name(rc)),
            });
        }
        if count == 0 {
            return Ok(Vec::new());
        }
        let mut devices = vec![ptr::null_mut(); count as usize];
        // SAFETY: `devices` 的容量与 `count` 一致，驱动最多写入 `count` 个句柄。
        let rc = unsafe { (self.enumerate_physical_devices)(instance, &mut count, devices.as_mut_ptr()) };
        if rc != VK_SUCCESS && rc != VK_INCOMPLETE {
            return Err(GpuError::Driver {
                code: rc,
                message: format!("vkEnumeratePhysicalDevices 失败：{}", result_name(rc)),
            });
        }
        devices.truncate(count as usize);
        Ok(devices)
    }

    /// 取物理设备属性（名称 / 类型 / 版本）。
    ///
    /// # Safety
    /// `pd` 必须来自**同一个存活实例**的枚举结果；传任意指针会让驱动解引用无效内存。
    pub(crate) unsafe fn properties(&self, pd: PhysicalDeviceHandle) -> GpuResult<DeviceProperties> {
        let mut raw = std::mem::MaybeUninit::<PhysicalDeviceProperties>::uninit();
        // SAFETY: Vulkan 保证该函数完整写入结构体，故 `assume_init` 安全；
        // `pd` 由调用方保证来自存活实例。
        unsafe { (self.get_physical_device_properties)(pd, raw.as_mut_ptr()) };
        let p = unsafe { raw.assume_init() };
        // SAFETY: `device_name` 是 NUL 结尾的 C 字符串（Vulkan 保证），
        // 上限为 MAX_PHYSICAL_DEVICE_NAME_SIZE。
        let device_name = unsafe { CStr::from_ptr(p.device_name.as_ptr()) }
            .to_string_lossy()
            .into_owned();
        Ok(DeviceProperties {
            device_name,
            device_type: PhysicalDeviceType::from_raw(p.device_type),
            api_version: p.api_version,
            driver_version: p.driver_version,
            vendor_id: p.vendor_id,
            device_id: p.device_id,
            max_image_dimension_2d: p.limits.max_image_dimension_2d,
            min_uniform_buffer_offset_alignment: p.limits.min_uniform_buffer_offset_alignment,
        })
    }
}

/// 判断某**设备扩展**在该物理设备上是否可用（查 `vkEnumerateDeviceExtensionProperties`）。
///
/// 这是 `VK_KHR_swapchain` 必须的检查：交换链是**设备扩展**，
/// 不启用就会在 `vkCreateSwapchainKHR` 处失败（而且失败信息与真实原因差得远）。
pub(crate) fn device_extension_available(pd: PhysicalDeviceHandle, name: &str) -> GpuResult<bool> {
    let lib = crate::loader::Lib::open()?;
    // SAFETY: 符号名与规范一致；签名由本文件的 `PfnEnumerateDeviceExtensionProperties` 声明。
    let enumerate: PfnEnumerateDeviceExtensionProperties = unsafe { lib.sym("vkEnumerateDeviceExtensionProperties")? };
    let mut count = 0u32;
    // SAFETY: 传 null 数组 = 规定的「只查数量」用法；`pd` 由调用方保证有效。
    let rc = unsafe { enumerate(pd, ptr::null(), &mut count, ptr::null_mut()) };
    if rc != VK_SUCCESS && rc != VK_INCOMPLETE {
        return Err(GpuError::Driver {
            code: rc,
            message: format!("vkEnumerateDeviceExtensionProperties 失败：{}", result_name(rc)),
        });
    }
    if count == 0 {
        return Ok(false);
    }
    let mut props = vec![
        ExtensionProperties {
            extension_name: [0; MAX_EXTENSION_NAME_SIZE],
            spec_version: 0,
        };
        count as usize
    ];
    // SAFETY: `props` 容量与 `count` 一致，驱动最多写入 `count` 项。
    let rc = unsafe { enumerate(pd, ptr::null(), &mut count, props.as_mut_ptr()) };
    if rc != VK_SUCCESS && rc != VK_INCOMPLETE {
        return Err(GpuError::Driver {
            code: rc,
            message: format!("vkEnumerateDeviceExtensionProperties（取列表）失败：{}", result_name(rc)),
        });
    }
    props.truncate(count as usize);
    Ok(props.iter().any(|p| {
        // SAFETY: `extensionName` 是 NUL 结尾 C 字符串（Vulkan 保证）。
        unsafe { CStr::from_ptr(p.extension_name.as_ptr()) }
            .to_string_lossy()
            == name
    }))
}
/// `VK_STRUCTURE_TYPE_DEBUG_UTILS_MESSENGER_CREATE_INFO_EXT`。
///
/// ⚠️ **这个值必须写对**：它在 1000xxxxxx 段（扩展编号），与核心的 0..45 不是一套。
/// 本项目第一版凭记忆写了 `1000128001`（正确是 `1000128004`），
/// 于是校验层报 `sType must be VK_STRUCTURE_TYPE_DEBUG_UTILS_MESSENGER_CREATE_INFO_EXT`
/// —— 一个**看起来像**「字段顺序/布局错」的错误，实际只是最后一个数字差 3。
/// 值已与 SDK 的 `vulkan_core.h` 逐位核对。
pub const VK_STRUCTURE_TYPE_DEBUG_UTILS_MESSENGER_CREATE_INFO_EXT: i32 = 1_000_128_004;

// ── VkDebugUtilsMessageSeverityFlagBitsEXT / MessageTypeFlagBitsEXT ──
//
// ⚠️ 这两组位值都是**从 1 开始的连续位**，不是「按名字猜的量级」。
// 本项目第一版把 VALIDATION 写成 0x10、PERFORMANCE 写成 0x100（按名字臆测量级），
// 结果校验层报 `messageType must be a valid combination of ...`。
// 值已与 SDK 的 `vulkan_core.h` 核对。
/// `VERBOSE = 0x01`
pub const DEBUG_UTILS_MESSAGE_SEVERITY_VERBOSE: u32 = 0x0000_0001;
/// `INFO = 0x10`
pub const DEBUG_UTILS_MESSAGE_SEVERITY_INFO: u32 = 0x0000_0010;
/// `WARNING = 0x100`
pub const DEBUG_UTILS_MESSAGE_SEVERITY_WARNING: u32 = 0x0000_0100;
/// `ERROR = 0x1000`
pub const DEBUG_UTILS_MESSAGE_SEVERITY_ERROR: u32 = 0x0000_1000;
/// `GENERAL = 0x01`
pub const DEBUG_UTILS_MESSAGE_TYPE_GENERAL: u32 = 0x0000_0001;
/// `VALIDATION = 0x02`
pub const DEBUG_UTILS_MESSAGE_TYPE_VALIDATION: u32 = 0x0000_0002;
/// `PERFORMANCE = 0x04`
pub const DEBUG_UTILS_MESSAGE_TYPE_PERFORMANCE: u32 = 0x0000_0004;

/// `VkDebugUtilsMessengerCreateInfoEXT`
///
/// 字段顺序**按官方 `vulkan_core.h`**（不是按字母顺序直觉）：
/// ```c
///   VkStructureType sType; const void* pNext;
///   VkDebugUtilsMessengerCreateFlagsEXT flags;
///   VkDebugUtilsMessageSeverityFlagsEXT messageSeverity;
///   VkDebugUtilsMessageTypeFlagsEXT messageType;
///   PFN_vkDebugUtilsMessengerCallbackEXT pfnUserCallback;
///   void* pUserData;
/// ```
/// 即 `flags` 在**第三位**（紧跟 `pNext`）。
#[repr(C)]
struct DebugUtilsMessengerCreateInfo {
    s_type: i32,
    p_next: *const c_void,
    flags: u32,
    message_severity: u32,
    message_type: u32,
    pfn_user_callback: PfnDebugUtilsMessengerCallback,
    p_user_data: *mut c_void,
}

/// `VkDebugUtilsMessengerCallbackDataEXT`（只需要第一个字段 `pMessage`）
#[repr(C)]
struct DebugUtilsMessengerCallbackData {
    s_type: i32,
    p_next: *const c_void,
    flags: u32,
    p_message_id_name: *const c_char,
    message_id_number: i32,
    p_message: *const c_char,
    queue_label_count: u32,
    p_queue_labels: *const c_void,
    cmd_buf_label_count: u32,
    p_cmd_buf_labels: *const c_void,
    object_count: u32,
    p_objects: *const c_void,
}

type PfnDebugUtilsMessengerCallback = unsafe extern "system" fn(
    message_severity: u32,
    message_type: u32,
    p_callback_data: *const DebugUtilsMessengerCallbackData,
    p_user_data: *mut c_void,
) -> u32;

/// 校验层回调累计收到的消息数（**进程级**，永不重置）。
///
/// ## 为什么它还在（M3c 复查后保留，而不是被 thread_local 取代）
///
/// 它与下面的 [`VALIDATION_MESSAGE_COUNT_TLS`] 是**两个不同的问题**，不能互相顶替：
///
/// | 计数器 | 语义 | 谁需要它 |
/// |---|---|---|
/// | 本项（进程级） | 「**本进程**至今一共报过多少条」——含**其它线程**的 | 「整个进程一条消息都没有」这种**绝对断言**（测试用法见 [`validation_message_count_global`]） |
/// | [`VALIDATION_MESSAGE_COUNT_TLS`] | 「**本线程**报过多少条」 | 「**本测试**期间零消息」这种**窗口差值**断言（[`validation_message_count`]） |
///
/// ⚠️ 两者被同时自增，并由 [`validation_message_count`] 的一条自检断言把
/// 「全局 ≥ 本线程」钉住 ⇒ 因此**不可能出现「本线程计数把它漏掉了」而无人察觉**。
static VALIDATION_MESSAGE_COUNT: AtomicUsize = AtomicUsize::new(0);

thread_local! {
    /// **本线程**累计收到的校验层消息数。
    ///
    /// ## 为什么必须是 thread_local（M3c 复查：实测 + 一次真实的假红）
    ///
    /// 第一版这里是进程级 `AtomicUsize`。它的用途是「跑完一段代码后断言**没有增长**」
    /// —— 这是**窗口差值**断言，而进程级计数器会被**别的测试线程**的消息污染。
    /// 与 `device::TEXTURE_R8_UPLOAD_COUNT`（review I-1）**完全同类**。
    ///
    /// 这次不是理论风险，而是**已经发生过的假红**：M3c 期间我有一条新测试的顶点属性表
    /// 不全（只声明 1 个，着色器消费 4/3 个）⇒ 校验层报 **5 条**
    /// `VUID-...-Input-07904` ⇒ 落进进程级计数 ⇒
    /// `texture_sampler_descriptor_are_clean_under_validation` **假红**
    /// （它断言「本测试期间零消息」，却被别的测试的消息顶红了）。
    ///
    /// ## 实测：回调是**线程亲和**的（这才是能改成 thread_local 的依据）
    ///
    /// M3c 复查做过一次**故意的违规探针**测量（工作线程用
    /// `create_graphics_pipelines` 传非法参数触发 11 条消息，主线程同时读**自己的** TLS）：
    ///
    /// ```text
    ///   起始：               全局=0   主线程 TLS=0
    ///   工作线程仍存活时：   主线程 TLS 自增=0   全局自增=11
    ///   工作线程自己的 TLS 自增=11
    ///   join 后：            主线程 TLS 自增=0
    ///   ⇒ 消息全部记在**触发它的那个线程**上
    /// ```
    ///
    /// 所以「按线程计数」如实反映「这段代码自己触发了什么」。若哪天换了 loader /
    /// 校验层版本而**不再亲和**，回归锁
    /// `tests/pipeline_smoke.rs::validation_counter_is_thread_local_not_process_wide`
    /// 会红（它是确定性的，不是偶发 —— 见那条测试的构造）。
    ///
    /// ## 使用约束（诚实说明）
    ///
    /// 计数只属于调用它的线程 ⇒ **读的一方必须在同一线程上**做了那次调用。
    /// 本项目满足：渲染与断言都在测试线程内完成。
    /// 若将来有测试要断言「**别的线程**产生了消息」，那才是
    /// [`validation_message_count_global`] 的用途。
    pub static VALIDATION_MESSAGE_COUNT_TLS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// **本线程**累计收到的校验层消息数。
///
/// 测试用法（配合 `DEER_VK_VALIDATION=1`）：跑完一段代码后断言它**没有增长** ——
/// 因为只统计本线程，别的测试并行产生的消息**不会**干扰这个差值。
pub fn validation_message_count() -> usize {
    VALIDATION_MESSAGE_COUNT_TLS.with(|c| c.get())
}

/// **进程级**累计收到的校验层消息数（含其它线程）。
///
/// 用于「整个进程一条消息都没有」这种**绝对断言**。因为它含别的线程的消息，
/// **不适合**做「本测试期间零消息」的窗口差值 —— 那种场合用
/// [`validation_message_count`]（按线程）。
pub fn validation_message_count_global() -> usize {
    VALIDATION_MESSAGE_COUNT.load(Ordering::Relaxed)
}

/// 校验层回调：把消息打到 stderr（`[VALIDATION]`/`[VK ERROR]` 前缀便于筛选）。
///
/// 用 `eprintln!` 而非日志框架：零依赖约束下最简单，且校验消息本来就不该混进正常输出。
///
/// ⚠️ 标签映射曾写错（把 `WARNING = 0x100` 标成 `VK ERROR`、`ERROR = 0x1000` 标成
/// `VALIDATION`、还引用了一个不存在的 `0x2000`）。后果不是漏报而是**误分类**：
/// 真正的校验错误被打成 `[VALIDATION]`，而警告被打成 `[VK ERROR]`，
/// 让人按前缀筛选时得出相反结论。现在按位值正确定义（见本文件上面的常量注释）。
unsafe extern "system" fn validation_callback(
    severity: u32,
    msg_type: u32,
    data: *const DebugUtilsMessengerCallbackData,
    _user: *mut c_void,
) -> u32 {
    if data.is_null() {
        return 0;
    }
    // SAFETY: 驱动保证 `p_callback_data` 指向有效的回调数据结构，且 `p_message`
    // 是 NUL 结尾字符串、在回调期间存活。
    let msg = unsafe {
        let p = (*data).p_message;
        if p.is_null() {
            return 0;
        }
        CStr::from_ptr(p).to_string_lossy().into_owned()
    };
    let kind = match severity {
        DEBUG_UTILS_MESSAGE_SEVERITY_ERROR => "VK ERROR",
        DEBUG_UTILS_MESSAGE_SEVERITY_WARNING => "VK WARN",
        _ if msg_type & DEBUG_UTILS_MESSAGE_TYPE_VALIDATION != 0 => "VALIDATION",
        DEBUG_UTILS_MESSAGE_SEVERITY_VERBOSE => "VK VERBOSE",
        _ => "VK INFO",
    };
    // 计数点放在「确定有一条消息」之后（上面两个提前返回不计）—— 见 `validation_message_count`。
    VALIDATION_MESSAGE_COUNT.fetch_add(1, Ordering::Relaxed);
    VALIDATION_MESSAGE_COUNT_TLS.with(|c| c.set(c.get() + 1));
    eprintln!("[{kind}] {msg}");
    0 // VK_FALSE：不中止
}

/// 一个已创建的 VkInstance。`Drop` 保证销毁。
///
/// **故意不实现 `Send`/`Sync`** —— Vulkan 实例的线程语义需要显式同步，
/// 悄悄放宽会让上层在不经意间跨线程用同一个句柄。
pub struct Instance {
    handle: InstanceHandle,
    loader: Loader,
    /// 校验层回调句柄（启用校验层时才有值），`Drop` 时销毁
    debug_messenger: Option<DebugUtilsMessengerHandle>,
    destroy_debug: Option<PfnDestroyDebugUtilsMessenger>,
    /// 记录本次实例是否真的启用了校验层（供上层如实报告）
    validation_enabled: bool,
    /// 本次实例实际启用的扩展名（**如实记录**，不靠调用方记忆）。
    enabled_extensions: Vec<String>,
}

type DebugUtilsMessengerHandle = *mut c_void;
type PfnCreateDebugUtilsMessenger =
    unsafe extern "system" fn(InstanceHandle, *const DebugUtilsMessengerCreateInfo, *const c_void, *mut DebugUtilsMessengerHandle) -> VkResult;
type PfnDestroyDebugUtilsMessenger =
    unsafe extern "system" fn(InstanceHandle, DebugUtilsMessengerHandle, *const c_void);

impl Instance {
    /// 创建实例（`apiVersion` 用 1.0，最大兼容；**不启用**校验层）。
    pub fn create() -> GpuResult<Instance> {
        Instance::create_with_validation(false)
    }

    /// 创建实例，可选启用 **`VK_LAYER_KHRONOS_validation`** 校验层。
    ///
    /// 校验层是本项目最缺的诊断手段：驱动对本项目的错误 SPIR-V / 遗漏的管线状态
    /// **不报错也不画**（例如 `vkCmdDraw` 完全不产生片元），而校验层会直接指出问题。
    ///
    /// 行为约定：
    /// - `validate = true` 且层**存在** ⇒ 启用，并挂上 `VkDebugUtilsMessengerEXT`
    ///   把消息打到 stderr；
    /// - `validate = true` 但层**不存在** ⇒ 返回 `Err(Unsupported)` 并说明原因
    ///   （**不静默降级** —— 静默不启用会让人误以为「校验通过」）。
    pub fn create_with_validation(validate: bool) -> GpuResult<Instance> {
        Instance::create_with_extensions(validate, &[])
    }

    /// 创建实例，并启用指定的**实例扩展**（M2b：`VK_KHR_surface` + 平台 surface 扩展）。
    ///
    /// 与 [`Instance::create_with_validation`] 的关系：后者等价于
    /// `create_with_extensions(validate, &[])` —— 行为逐字不变（既有 46 条测试依赖它）。
    ///
    /// **不靠猜**：每个请求的扩展都先用 [`Instance::extension_available`] 查一遍
    /// （`vkEnumerateInstanceExtensionProperties`），缺了就返回 `Err(Unsupported)`
    /// 并列出本机可用的扩展。为什么不静默忽略 —— 静默忽略会变成「实例建好了但
    /// `vkCreateWin32SurfaceKHR` 取不到」，症状离原因很远。
    pub fn create_with_extensions(validate: bool, extensions: &[&str]) -> GpuResult<Instance> {
        const VK_API_VERSION_1_0: u32 = 1 << 22;
        let loader = Loader::load()?;

        // 需要在 loader 存活期间解析校验层相关符号
        let available = loader.available_layers();
        let use_validation = if validate {
            if available.iter().any(|n| n == VALIDATION_LAYER) {
                true
            } else {
                return Err(GpuError::Unsupported(format!(
                    "请求启用校验层 {VALIDATION_LAYER}，但本机未安装。\
                     已注册的层有 {available:?}。\
                     装 Vulkan SDK 后即可用（或把层 manifest 的目录加进 VK_LAYER_PATH）"
                )));
            }
        } else {
            false
        };

        if !extensions.is_empty() {
            let supported = loader.available_extensions();
            let missing: Vec<&str> = extensions
                .iter()
                .copied()
                .filter(|e| !supported.iter().any(|s| s == e))
                .collect();
            if !missing.is_empty() {
                return Err(GpuError::Unsupported(format!(
                    "本机 Vulkan 不支持实例扩展 {missing:?}（可用扩展共 {} 个：{:?}）。\
                     这类扩展通常是「平台相关的 WSI 扩展」，显卡驱动或 loader 太旧时会缺",
                    supported.len(),
                    supported
                )));
            }
        }

        // 层名（C 字符串）+ 扩展名（可能是运行时字符串），都要活到 vkCreateInstance 调用结束
        let layer_name = c"VK_LAYER_KHRONOS_validation";
        let layer_ptrs: [*const c_char; 1] = [layer_name.as_ptr()];
        // 扩展名统一转成 NUL 结尾字节串并持有：`ppEnabledExtensionNames` 只借它们的指针。
        let ext_bytes: Vec<Vec<u8>> = extensions
            .iter()
            .map(|e| {
                let mut v = Vec::with_capacity(e.len() + 1);
                v.extend_from_slice(e.as_bytes());
                v.push(0);
                v
            })
            .collect();
        let mut ext_ptrs: Vec<*const c_char> = Vec::with_capacity(ext_bytes.len() + 1);
        if use_validation {
            ext_ptrs.push(c"VK_EXT_debug_utils".as_ptr());
        }
        ext_ptrs.extend(ext_bytes.iter().map(|b| b.as_ptr() as *const c_char));

        let app_name = c"deer-gui";
        let engine_name = c"deer-gui";
        let app_info = ApplicationInfo {
            s_type: VK_STRUCTURE_TYPE_APPLICATION_INFO,
            p_next: ptr::null(),
            p_application_name: app_name.as_ptr(),
            application_version: 1,
            p_engine_name: engine_name.as_ptr(),
            engine_version: 1,
            api_version: VK_API_VERSION_1_0,
        };
        let create_info = InstanceCreateInfo {
            s_type: VK_STRUCTURE_TYPE_INSTANCE_CREATE_INFO,
            p_next: ptr::null(),
            flags: 0,
            p_application_info: &app_info,
            enabled_layer_count: if use_validation { 1 } else { 0 },
            pp_enabled_layer_names: if use_validation {
                layer_ptrs.as_ptr()
            } else {
                ptr::null()
            },
            enabled_extension_count: ext_ptrs.len() as u32,
            pp_enabled_extension_names: if ext_ptrs.is_empty() {
                ptr::null()
            } else {
                ext_ptrs.as_ptr()
            },
        };

        let mut handle: InstanceHandle = ptr::null_mut();
        // SAFETY: `create_info` 指向本栈帧已初始化、且在调用期间存活的结构体；
        // 层名/扩展名数组同样在本栈帧存活；`handle` 是可写输出参数。
        let rc = unsafe { (loader.fns.create_instance)(&create_info, ptr::null(), &mut handle) };
        if rc != VK_SUCCESS {
            // 启用了校验层却创建失败时，明确区分「层的问题」与一般失败
            let hint = if use_validation {
                "（注意：本次请求了校验层，失败可能是层与驱动不兼容）"
            } else {
                ""
            };
            return Err(GpuError::Driver {
                code: rc,
                message: format!("vkCreateInstance 失败：{}{hint}", result_name(rc)),
            });
        }
        if handle.is_null() {
            return Err(GpuError::Driver {
                code: rc,
                message: "vkCreateInstance 返回成功但句柄为空".to_string(),
            });
        }

        // 挂调试回调（只有启用校验层时才有意义）
        let mut debug_messenger = None;
        let mut destroy_debug = None;
        if use_validation {
            let create_dbg: PfnCreateDebugUtilsMessenger =
                // SAFETY: 符号名与规范一致；签名由本文件声明。用 inst_sym 是因为
                // 这是**实例级**（并且由层提供）的函数。
                unsafe { loader.inst_sym(handle, "vkCreateDebugUtilsMessengerEXT")? };
            destroy_debug = Some(
                // SAFETY: 同上。
                unsafe { loader.inst_sym::<PfnDestroyDebugUtilsMessenger>(handle, "vkDestroyDebugUtilsMessengerEXT")? },
            );
            let info = DebugUtilsMessengerCreateInfo {
                s_type: VK_STRUCTURE_TYPE_DEBUG_UTILS_MESSENGER_CREATE_INFO_EXT,
                p_next: ptr::null(),
                flags: 0,
                // ERROR | WARNING（info/verbose 量太大，先只看有问题的）
                message_severity: DEBUG_UTILS_MESSAGE_SEVERITY_ERROR
                    | DEBUG_UTILS_MESSAGE_SEVERITY_WARNING,
                // GENERAL | VALIDATION | PERFORMANCE
                message_type: DEBUG_UTILS_MESSAGE_TYPE_GENERAL
                    | DEBUG_UTILS_MESSAGE_TYPE_VALIDATION
                    | DEBUG_UTILS_MESSAGE_TYPE_PERFORMANCE,
                pfn_user_callback: validation_callback,
                p_user_data: ptr::null_mut(),
            };
            let mut m: DebugUtilsMessengerHandle = ptr::null_mut();
            // SAFETY: 实例刚创建且有效；`info` 在栈上存活；`m` 是可写输出。
            let rc = unsafe { create_dbg(handle, &info, ptr::null(), &mut m) };
            if rc != VK_SUCCESS {
                return Err(GpuError::Driver {
                    code: rc,
                    message: format!("vkCreateDebugUtilsMessengerEXT 失败：{}", result_name(rc)),
                });
            }
            debug_messenger = Some(m);
        }

        // 如实记录本次启用的扩展（含校验层自带的 `VK_EXT_debug_utils`）
        let mut enabled_extensions = Vec::with_capacity(ext_ptrs.len());
        if use_validation {
            enabled_extensions.push(DEBUG_UTILS_EXTENSION.to_string());
        }
        enabled_extensions.extend(extensions.iter().map(|e| (*e).to_string()));

        Ok(Instance {
            handle,
            loader,
            debug_messenger,
            destroy_debug,
            validation_enabled: use_validation,
            enabled_extensions,
        })
    }

    /// 本次实例是否真的启用了校验层。
    pub fn validation_enabled(&self) -> bool {
        self.validation_enabled
    }

    /// 本次实例**实际启用**的扩展名（按启用顺序）。
    pub fn enabled_extensions(&self) -> &[String] {
        &self.enabled_extensions
    }

    /// 某**实例扩展**在本机是否可用（查 `vkEnumerateInstanceExtensionProperties`，**不靠猜**）。
    ///
    /// loader 缺失或枚举失败时返回 `false`（「查不到」等价于「不可用」）。
    pub fn extension_available(name: &str) -> bool {
        match Loader::load() {
            Ok(loader) => loader.available_extensions().iter().any(|e| e == name),
            Err(_) => false,
        }
    }

    /// `DEER_VK_VALIDATION=1`/`true` ⇒ 请求校验层（与 [`crate::VkBackend::new`] 同一套约定）。
    ///
    /// 放在这里而不是各调用点重复一遍：窗口路径（`windowed.rs`）与离屏路径
    /// 必须用**同一个判据**，否则「同一个二进制加了环境变量却只有一半开了校验」。
    ///
    /// **公开**（而非 `pub(crate)`）是为了让示例/测试也能用同一份判据 ——
    /// 例如 `vulkan_pipeline` 要在请求校验层时跳过那支已知损坏的推送常量着色器
    /// （见 `ROADMAP.md` Q-5），它不该自己手写一遍 `std::env::var(...)`。
    pub fn validation_from_env() -> bool {
        std::env::var("DEER_VK_VALIDATION")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false)
    }

    /// 本实例的核心函数副本（供「借用实例」的设备创建路径使用）。
    pub(crate) fn core_fns(&self) -> CoreFns {
        CoreFns {
            enumerate_physical_devices: self.loader.fns.enumerate_physical_devices,
            get_physical_device_properties: self.loader.fns.get_physical_device_properties,
        }
    }

    /// 取一个**实例级**函数指针（走 `vkGetInstanceProcAddr`）。
    ///
    /// 平台 surface 扩展（`vkCreateWin32SurfaceKHR` / `vkGetPhysicalDeviceSurfaceSupportKHR` …）
    /// **不在** `vulkan-1.dll` 的导出表里 —— 它们是 ICD 提供、由 loader 分发的，
    /// 所以必须走这条路（见 [`Loader::inst_sym`] 的实测记录）。
    ///
    /// # Safety
    /// `T` 必须是该符号**真实签名**对应的函数指针类型；`&self` 必须仍存活
    /// （返回的函数指针在实例销毁后失效）。
    pub(crate) unsafe fn proc<T: Copy>(&self, name: &str) -> GpuResult<T> {
        // SAFETY: 调用方保证 `T` 与符号真实签名一致；实例句柄由本结构持有且存活。
        unsafe { self.loader.inst_sym(self.handle, name) }
    }

    pub fn handle(&self) -> InstanceHandle {
        self.handle
    }

    /// 枚举物理设备（两阶段：先问数量，再取列表）。
    pub fn enumerate_physical_devices(&self) -> GpuResult<Vec<PhysicalDeviceHandle>> {
        // SAFETY: `self.handle` 是本结构持有的有效实例句柄。
        unsafe { self.core_fns().enumerate_physical_devices(self.handle) }
    }

    /// 取设备属性（名称 / 类型 / 版本）。
    ///
    /// # Safety
    /// `pd` 必须是**本实例**枚举出的物理设备句柄，且实例尚未销毁。
    /// 传任意指针会让驱动解引用无效内存。
    pub unsafe fn physical_device_properties(
        &self,
        pd: PhysicalDeviceHandle,
    ) -> GpuResult<DeviceProperties> {
        // SAFETY: 调用方保证 `pd` 来自本实例；实例仍存活。
        unsafe { self.core_fns().properties(pd) }
    }
}

impl Drop for Instance {
    fn drop(&mut self) {
        // 先销毁调试回调，再销毁实例（Vulkan 要求的顺序）
        if let (Some(m), Some(destroy)) = (self.debug_messenger, self.destroy_debug) {
            // SAFETY: 回调由本实例创建、尚未销毁；实例此时仍存活。
            unsafe { destroy(self.handle, m, ptr::null()) };
            self.debug_messenger = None;
        }
        if !self.handle.is_null() {
            // SAFETY: 句柄由 `vkCreateInstance` 创建且未被销毁（`Drop` 只跑一次）。
            unsafe { (self.loader.fns.destroy_instance)(self.handle, ptr::null()) };
            self.handle = ptr::null_mut();
        }
    }
}

/// 从 `VkPhysicalDeviceProperties` 提炼的可读信息。
#[derive(Debug, Clone, PartialEq)]
pub struct DeviceProperties {
    pub device_name: String,
    pub device_type: PhysicalDeviceType,
    pub api_version: u32,
    pub driver_version: u32,
    pub vendor_id: u32,
    pub device_id: u32,
    pub max_image_dimension_2d: u32,
    pub min_uniform_buffer_offset_alignment: u64,
}

pub(crate) fn result_name(rc: VkResult) -> &'static str {
    match rc {
        0 => "VK_SUCCESS",
        1 => "VK_NOT_READY",
        2 => "VK_TIMEOUT",
        3 => "VK_EVENT_SET",
        4 => "VK_EVENT_RESET",
        5 => "VK_INCOMPLETE",
        -1 => "VK_ERROR_OUT_OF_HOST_MEMORY",
        -2 => "VK_ERROR_OUT_OF_DEVICE_MEMORY",
        -3 => "VK_ERROR_INITIALIZATION_FAILED",
        -4 => "VK_ERROR_DEVICE_LOST",
        -5 => "VK_ERROR_MEMORY_MAP_FAILED",
        -6 => "VK_ERROR_LAYER_NOT_PRESENT",
        -7 => "VK_ERROR_EXTENSION_NOT_PRESENT",
        -8 => "VK_ERROR_FEATURE_NOT_PRESENT",
        -9 => "VK_ERROR_INCOMPATIBLE_DRIVER",
        -10 => "VK_ERROR_TOO_MANY_OBJECTS",
        -11 => "VK_ERROR_FORMAT_NOT_SUPPORTED",
        -12 => "VK_ERROR_FRAGMENTED_POOL",
        -13 => "VK_ERROR_UNKNOWN",
        VK_ERROR_SURFACE_LOST_KHR => "VK_ERROR_SURFACE_LOST_KHR",
        VK_ERROR_OUT_OF_DATE_KHR => "VK_ERROR_OUT_OF_DATE_KHR",
        VK_SUBOPTIMAL_KHR => "VK_SUBOPTIMAL_KHR",
        _ => "未知 VkResult",
    }
}

/// 结构体布局断言 —— 这些是**绝对不能错**的约束。
///
/// 布局错了驱动会读到垃圾，症状不是崩溃而是诡异行为（最难查的一类）。
/// 断言值来自 Vulkan 规范的结构体定义 + 本机 64 位 MSVC ABI。
#[cfg(test)]
mod layout_tests {
    use super::*;

    #[test]
    fn instance_create_info_layout() {
        // s_type(4)+pad(4) p_next(8) flags(4)+pad(4) p_app(8) layerCount(4)+pad(4)
        // ppLayers(8) extCount(4)+pad(4) ppExts(8) = 64
        assert_eq!(std::mem::size_of::<InstanceCreateInfo>(), 64);
    }

    #[test]
    fn application_info_layout() {
        // s_type(4)+pad(4) p_next(8) p_name(8) appVer(4)+pad(4) p_engine(8) engVer(4) apiVer(4) = 48
        assert_eq!(std::mem::size_of::<ApplicationInfo>(), 48);
    }

    #[test]
    fn sparse_properties_layout() {
        assert_eq!(std::mem::size_of::<PhysicalDeviceSparseProperties>(), 20);
    }

    #[test]
    fn critical_offsets_are_stable() {
        assert_eq!(
            std::mem::offset_of!(PhysicalDeviceProperties, device_type),
            16,
            "device_type 必须在 api/driver/vendor/device_id（4×4 字节）之后"
        );
        assert_eq!(
            std::mem::offset_of!(PhysicalDeviceProperties, device_name),
            20,
            "device_name 必须紧跟 device_type"
        );
        assert_eq!(
            std::mem::size_of::<[c_char; MAX_PHYSICAL_DEVICE_NAME_SIZE]>(),
            256
        );
    }

    #[test]
    fn limits_layout_is_sane() {
        // max_image_dimension_2d 是第二个字段 ⇒ offset 4
        assert_eq!(std::mem::offset_of!(PhysicalDeviceLimits, max_image_dimension_2d), 4);
        // 整个 limits 结构体是 4 字节对齐的平凡布局
        assert_eq!(std::mem::align_of::<PhysicalDeviceLimits>(), 8);
    }
}
