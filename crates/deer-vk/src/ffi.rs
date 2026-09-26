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

use deer_gpu::{GpuError, GpuResult};

pub type InstanceHandle = *mut c_void;
pub type PhysicalDeviceHandle = *mut c_void;
pub type VkResult = i32;

pub const VK_SUCCESS: VkResult = 0;
pub const VK_INCOMPLETE: VkResult = 5;

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
type PfnEnumeratePhysicalDevices =
    unsafe extern "system" fn(InstanceHandle, *mut u32, *mut PhysicalDeviceHandle) -> VkResult;
type PfnGetPhysicalDeviceProperties =
    unsafe extern "system" fn(PhysicalDeviceHandle, *mut PhysicalDeviceProperties);
type PfnEnumerateInstanceLayerProperties = unsafe extern "system" fn(*mut u32, *mut LayerProperties) -> VkResult;

/// 从 `vulkan-1.dll` 取到的函数表。
struct Fns {
    create_instance: PfnCreateInstance,
    destroy_instance: PfnDestroyInstance,
    enumerate_physical_devices: PfnEnumeratePhysicalDevices,
    get_physical_device_properties: PfnGetPhysicalDeviceProperties,
    enumerate_instance_layer_properties: PfnEnumerateInstanceLayerProperties,
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

        Ok(Loader {
            module,
            fns: Fns {
                create_instance,
                destroy_instance,
                enumerate_physical_devices,
                get_physical_device_properties,
                enumerate_instance_layer_properties,
            },
        })
    }

    /// 取一个符号并转成函数指针（**实例级**扩展符号用，如 `vkCreateDebugUtilsMessengerEXT`）。
    ///
    /// # Safety
    /// `T` 必须是该符号**真实签名**对应的函数指针类型。
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

/// `VkDebugUtilsMessengerCreateInfoEXT`
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

/// 校验层回调：把消息打到 stderr（`[VALIDATION]`/`[VK ERROR]` 前缀便于筛选）。
///
/// 用 `eprintln!` 而非日志框架：零依赖约束下最简单，且校验消息本来就不该混进正常输出。
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
    let kind = match (severity, msg_type) {
        (0x0000_0100, _) => "VK ERROR",
        (0x0000_1000, _) => "VALIDATION",
        (0x0000_2000, _) => "VK WARN",
        _ => "VK INFO",
    };
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

        // 层名（C 字符串）+ 调试扩展名，都要活到 vkCreateInstance 调用结束
        let layer_name = c"VK_LAYER_KHRONOS_validation";
        let ext_debug_utils = c"VK_EXT_debug_utils";
        let layer_ptrs: [*const c_char; 1] = [layer_name.as_ptr()];
        let ext_ptrs: [*const c_char; 1] = [ext_debug_utils.as_ptr()];

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
            enabled_extension_count: if use_validation { 1 } else { 0 },
            pp_enabled_extension_names: if use_validation {
                ext_ptrs.as_ptr()
            } else {
                ptr::null()
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
                // SAFETY: 符号名与规范一致；签名由本文件声明。
                unsafe { loader.sym("vkCreateDebugUtilsMessengerEXT")? };
            destroy_debug =
                Some(unsafe { loader.sym::<PfnDestroyDebugUtilsMessenger>("vkDestroyDebugUtilsMessengerEXT")? });
            let info = DebugUtilsMessengerCreateInfo {
                s_type: 1_000_128_001, // VK_STRUCTURE_TYPE_DEBUG_UTILS_MESSENGER_CREATE_INFO_EXT
                p_next: ptr::null(),
                flags: 0,
                // ERROR | WARNING（info 量太大，先只看有问题的）
                message_severity: 0x0000_0100 | 0x0000_1000,
                // GENERAL | VALIDATION | PERFORMANCE
                message_type: 0x0000_0001 | 0x0000_0010 | 0x0000_0100,
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

        Ok(Instance {
            handle,
            loader,
            debug_messenger,
            destroy_debug,
            validation_enabled: use_validation,
        })
    }

    /// 本次实例是否真的启用了校验层。
    pub fn validation_enabled(&self) -> bool {
        self.validation_enabled
    }

    pub fn handle(&self) -> InstanceHandle {
        self.handle
    }

    /// 枚举物理设备（两阶段：先问数量，再取列表）。
    pub fn enumerate_physical_devices(&self) -> GpuResult<Vec<PhysicalDeviceHandle>> {
        let mut count: u32 = 0;
        // SAFETY: 传 null 作为设备数组 = Vulkan 规定的「只查数量」用法。
        let rc = unsafe {
            (self.loader.fns.enumerate_physical_devices)(self.handle, &mut count, ptr::null_mut())
        };
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
        let rc = unsafe {
            (self.loader.fns.enumerate_physical_devices)(self.handle, &mut count, devices.as_mut_ptr())
        };
        if rc != VK_SUCCESS && rc != VK_INCOMPLETE {
            return Err(GpuError::Driver {
                code: rc,
                message: format!("vkEnumeratePhysicalDevices 失败：{}", result_name(rc)),
            });
        }
        devices.truncate(count as usize);
        Ok(devices)
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
        let mut raw = std::mem::MaybeUninit::<PhysicalDeviceProperties>::uninit();
        // SAFETY: Vulkan 保证该函数完整写入结构体，故 `assume_init` 安全；
        // `pd` 来自本实例的枚举结果。
        unsafe { (self.loader.fns.get_physical_device_properties)(pd, raw.as_mut_ptr()) };
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

fn result_name(rc: VkResult) -> &'static str {
    match rc {
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
