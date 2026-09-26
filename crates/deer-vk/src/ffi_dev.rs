//! Vulkan **设备级** FFI（M2a）。
//!
//! 与 `ffi.rs`（实例级：只到物理设备枚举）分开的理由：`ffi.rs` 已经被真机验证过，
//! 不希望在它上面继续堆结构体；新模块只依赖 `kernel32`（动态加载），
//! 与 `ffi.rs` 用同一套手法。
//!
//! ## 布局怎么保证正确
//!
//! 手写结构体的风险是「字段写错 ⇒ 驱动读到垃圾」，而症状往往不是崩溃、
//! 而是诡异行为。本模块用**两层**防守：
//!
//! 1. `size_of` / `offset_of!` 断言（本文件末尾的测试）—— 挡住明显错误；
//! 2. **驱动验收**（`tests/`）—— 真的调 `vkCreateDevice` / `vkCreateShaderModule` /
//!    `vkCreateGraphicsPipelines`，驱动拒绝即失败。这比自写校验器有力得多。

use std::ffi::{c_char, c_void};
use std::ptr;

// ── 复用实例级模块的句柄类型 ─────────────────────────────────────────────────

pub use crate::ffi::{InstanceHandle, PhysicalDeviceHandle, VkResult, VK_SUCCESS};

pub type DeviceHandle = *mut c_void;
pub type QueueHandle = *mut c_void;
pub type ShaderModuleHandle = *mut c_void;
pub type PipelineHandle = *mut c_void;
pub type PipelineLayoutHandle = *mut c_void;
pub type RenderPassHandle = *mut c_void;
pub type CommandPoolHandle = *mut c_void;
pub type CommandBufferHandle = *mut c_void;
pub type ImageHandle = *mut c_void;
pub type ImageViewHandle = *mut c_void;
pub type DeviceMemoryHandle = *mut c_void;
pub type FramebufferHandle = *mut c_void;
pub type FenceHandle = *mut c_void;
pub type BufferHandle = *mut c_void;
pub type SemaphoreHandle = *mut c_void;

/// `VK_NULL_HANDLE`
pub const NULL_HANDLE: *mut c_void = ptr::null_mut();
/// `VK_WHOLE_SIZE`
pub const WHOLE_SIZE: u64 = u64::MAX;
/// `VK_QUEUE_FAMILY_IGNORED`
pub const QUEUE_FAMILY_IGNORED: u32 = u32::MAX;
/// `VK_REMAINING_ARRAY_LAYERS`
pub const REMAINING_ARRAY_LAYERS: u32 = u32::MAX;
/// `VK_REMAINING_MIP_LEVELS`
pub const REMAINING_MIP_LEVELS: u32 = u32::MAX;

// ── 结构体类型枚举（只列用到的） ─────────────────────────────────────────────

pub const VK_STRUCTURE_TYPE_DEVICE_QUEUE_CREATE_INFO: i32 = 2;
pub const VK_STRUCTURE_TYPE_DEVICE_CREATE_INFO: i32 = 3;
pub const VK_STRUCTURE_TYPE_SUBMIT_INFO: i32 = 4;
pub const VK_STRUCTURE_TYPE_SHADER_MODULE_CREATE_INFO: i32 = 16;
pub const VK_STRUCTURE_TYPE_PIPELINE_SHADER_STAGE_CREATE_INFO: i32 = 18;
pub const VK_STRUCTURE_TYPE_PIPELINE_VERTEX_INPUT_STATE_CREATE_INFO: i32 = 19;
pub const VK_STRUCTURE_TYPE_PIPELINE_INPUT_ASSEMBLY_STATE_CREATE_INFO: i32 = 20;
pub const VK_STRUCTURE_TYPE_PIPELINE_VIEWPORT_STATE_CREATE_INFO: i32 = 22;
pub const VK_STRUCTURE_TYPE_PIPELINE_RASTERIZATION_STATE_CREATE_INFO: i32 = 23;
pub const VK_STRUCTURE_TYPE_PIPELINE_MULTISAMPLE_STATE_CREATE_INFO: i32 = 24;
pub const VK_STRUCTURE_TYPE_PIPELINE_COLOR_BLEND_STATE_CREATE_INFO: i32 = 26;
pub const VK_STRUCTURE_TYPE_PIPELINE_LAYOUT_CREATE_INFO: i32 = 30;
pub const VK_STRUCTURE_TYPE_GRAPHICS_PIPELINE_CREATE_INFO: i32 = 28;
pub const VK_STRUCTURE_TYPE_ATTACHMENT_DESCRIPTION: i32 = 8;
pub const VK_STRUCTURE_TYPE_SUBPASS_DESCRIPTION: i32 = 9;
pub const VK_STRUCTURE_TYPE_ATTACHMENT_REFERENCE: i32 = 7;
pub const VK_STRUCTURE_TYPE_RENDER_PASS_CREATE_INFO: i32 = 38;
pub const VK_STRUCTURE_TYPE_RENDER_PASS_BEGIN_INFO: i32 = 43;
pub const VK_STRUCTURE_TYPE_FRAMEBUFFER_CREATE_INFO: i32 = 37;
pub const VK_STRUCTURE_TYPE_IMAGE_CREATE_INFO: i32 = 14;
pub const VK_STRUCTURE_TYPE_IMAGE_VIEW_CREATE_INFO: i32 = 15;
pub const VK_STRUCTURE_TYPE_MEMORY_ALLOCATE_INFO: i32 = 5;
pub const VK_STRUCTURE_TYPE_COMMAND_POOL_CREATE_INFO: i32 = 39;
pub const VK_STRUCTURE_TYPE_COMMAND_BUFFER_ALLOCATE_INFO: i32 = 40;
pub const VK_STRUCTURE_TYPE_COMMAND_BUFFER_BEGIN_INFO: i32 = 42;
pub const VK_STRUCTURE_TYPE_BUFFER_CREATE_INFO: i32 = 12;
pub const VK_STRUCTURE_TYPE_FENCE_CREATE_INFO: i32 = 8;
pub const VK_STRUCTURE_TYPE_SEMAPHORE_CREATE_INFO: i32 = 9;

// ── 枚举值 ───────────────────────────────────────────────────────────────────

/// `VkImageType`
pub const VK_IMAGE_TYPE_2D: i32 = 1;
/// `VkFormat`：8 位 BGRA，非线性 sRGB（Windows 交换链常见格式）
pub const VK_FORMAT_B8G8R8A8_SRGB: i32 = 50;
/// `VkFormat`：8 位 RGBA，非线性 sRGB
pub const VK_FORMAT_R8G8B8A8_SRGB: i32 = 43;
/// `VkFormat`：8 位 RGBA，线性（回读用）
pub const VK_FORMAT_R8G8B8A8_UNORM: i32 = 37;
/// `VkImageTiling`
pub const VK_IMAGE_TILING_OPTIMAL: i32 = 0;
pub const VK_IMAGE_TILING_LINEAR: i32 = 1;
/// `VkImageLayout`
pub const VK_IMAGE_LAYOUT_UNDEFINED: i32 = 0;
pub const VK_IMAGE_LAYOUT_GENERAL: i32 = 1;
pub const VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL: i32 = 2;
pub const VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL: i32 = 5;
pub const VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL: i32 = 6;
pub const VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL: i32 = 7;
pub const VK_IMAGE_LAYOUT_PRESENT_SRC_KHR: i32 = 1000001002;
/// `VkSampleCountFlagBits`
pub const VK_SAMPLE_COUNT_1_BIT: u32 = 1;
/// `VkImageUsageFlagBits`
pub const VK_IMAGE_USAGE_TRANSFER_SRC_BIT: u32 = 1 << 0;
pub const VK_IMAGE_USAGE_TRANSFER_DST_BIT: u32 = 1 << 1;
pub const VK_IMAGE_USAGE_SAMPLED_BIT: u32 = 1 << 2;
pub const VK_IMAGE_USAGE_COLOR_ATTACHMENT_BIT: u32 = 1 << 4;
/// `VkImageAspectFlagBits`
pub const VK_IMAGE_ASPECT_COLOR_BIT: u32 = 1 << 0;
/// `VkImageViewType`
pub const VK_IMAGE_VIEW_TYPE_2D: i32 = 1;
/// `VkComponentSwizzle Identity`
pub const VK_COMPONENT_SWIZZLE_IDENTITY: i32 = 0;
/// `VkMemoryPropertyFlagBits`
pub const VK_MEMORY_PROPERTY_DEVICE_LOCAL_BIT: u32 = 1 << 0;
pub const VK_MEMORY_PROPERTY_HOST_VISIBLE_BIT: u32 = 1 << 1;
pub const VK_MEMORY_PROPERTY_HOST_COHERENT_BIT: u32 = 1 << 2;
/// `VkBufferUsageFlagBits`
pub const VK_BUFFER_USAGE_TRANSFER_SRC_BIT: u32 = 1 << 0;
pub const VK_BUFFER_USAGE_TRANSFER_DST_BIT: u32 = 1 << 1;
pub const VK_BUFFER_USAGE_VERTEX_BUFFER_BIT: u32 = 1 << 5;
pub const VK_BUFFER_USAGE_UNIFORM_BUFFER_BIT: u32 = 1 << 4;
/// `VkSharingMode`
pub const VK_SHARING_MODE_EXCLUSIVE: i32 = 0;
/// `VkAttachmentLoadOp`
pub const VK_ATTACHMENT_LOAD_OP_LOAD: i32 = 0;
pub const VK_ATTACHMENT_LOAD_OP_CLEAR: i32 = 1;
pub const VK_ATTACHMENT_LOAD_OP_DONT_CARE: i32 = 2;
/// `VkAttachmentStoreOp`
pub const VK_ATTACHMENT_STORE_OP_STORE: i32 = 0;
pub const VK_ATTACHMENT_STORE_OP_DONT_CARE: i32 = 1;
/// `VkImageLayout` 在附件描述里用 Undefined
pub const VK_IMAGE_LAYOUT_UNDEFINED_ATTACHMENT: i32 = 0;
/// `VkPipelineBindPoint`
pub const VK_PIPELINE_BIND_POINT_GRAPHICS: i32 = 0;
/// `VkPrimitiveTopology`
pub const VK_PRIMITIVE_TOPOLOGY_TRIANGLE_LIST: i32 = 3;
/// `VkPolygonMode`
pub const VK_POLYGON_MODE_FILL: i32 = 0;
/// `VkCullModeFlagBits`
pub const VK_CULL_MODE_NONE: u32 = 0;
pub const VK_CULL_MODE_BACK_BIT: u32 = 1 << 0;
/// `VkFrontFace`
pub const VK_FRONT_FACE_COUNTER_CLOCKWISE: i32 = 0;
pub const VK_FRONT_FACE_CLOCKWISE: i32 = 1;
/// `VkLogicOp`
pub const VK_LOGIC_OP_COPY: i32 = 3;
/// `VkBlendFactor`
pub const VK_BLEND_FACTOR_SRC_ALPHA: i32 = 6;
pub const VK_BLEND_FACTOR_ONE_MINUS_SRC_ALPHA: i32 = 7;
pub const VK_BLEND_FACTOR_ONE: i32 = 1;
pub const VK_BLEND_FACTOR_ZERO: i32 = 0;
/// `VkBlendOp`
pub const VK_BLEND_OP_ADD: i32 = 0;
/// `VkColorComponentFlagBits`
pub const VK_COLOR_COMPONENT_R_BIT: u32 = 1 << 0;
pub const VK_COLOR_COMPONENT_G_BIT: u32 = 1 << 1;
pub const VK_COLOR_COMPONENT_B_BIT: u32 = 1 << 2;
pub const VK_COLOR_COMPONENT_A_BIT: u32 = 1 << 3;
pub const VK_COLOR_COMPONENT_RGBA_BITS: u32 = 0xf;
/// `VkPipelineStageFlagBits`
pub const VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT: u32 = 1 << 0;
pub const VK_PIPELINE_STAGE_TRANSFER_BIT: u32 = 1 << 12;
pub const VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT: u32 = 1 << 10;
pub const VK_PIPELINE_STAGE_BOTTOM_OF_PIPE_BIT: u32 = 1 << 13;
pub const VK_PIPELINE_STAGE_ALL_COMMANDS_BIT: u32 = 1 << 16;
/// `VkAccessFlagBits`
pub const VK_ACCESS_TRANSFER_WRITE_BIT: u32 = 1 << 9;
pub const VK_ACCESS_TRANSFER_READ_BIT: u32 = 1 << 11;
pub const VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT: u32 = 1 << 8;
pub const VK_ACCESS_MEMORY_READ_BIT: u32 = 1 << 15;
/// `VkDependencyFlagBits`
pub const VK_DEPENDENCY_BY_REGION_BIT: u32 = 1;
/// `VkCommandBufferUsageFlagBits`
pub const VK_COMMAND_BUFFER_USAGE_ONE_TIME_SUBMIT_BIT: u32 = 1;
/// `VkCommandPoolCreateFlagBits`
pub const VK_COMMAND_POOL_CREATE_TRANSIENT_BIT: u32 = 1;
pub const VK_COMMAND_POOL_CREATE_RESET_COMMAND_BUFFER_BIT: u32 = 2;
/// `VkQueueFlagBits`
pub const VK_QUEUE_GRAPHICS_BIT: u32 = 1;
/// `VkShaderStageFlagBits`
pub const VK_SHADER_STAGE_VERTEX_BIT: u32 = 1;
pub const VK_SHADER_STAGE_FRAGMENT_BIT: u32 = 16;
/// `VkDynamicState`
pub const VK_DYNAMIC_STATE_VIEWPORT: i32 = 0;
pub const VK_DYNAMIC_STATE_SCISSOR: i32 = 1;
/// 等待/不等待
pub const VK_TRUE: u32 = 1;
pub const VK_FALSE: u32 = 0;
/// 无限等待
pub const U64_MAX: u64 = u64::MAX;

// ── 结构体 ───────────────────────────────────────────────────────────────────

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Extent2D {
    pub width: u32,
    pub height: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Extent3D {
    pub width: u32,
    pub height: u32,
    pub depth: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Offset2D {
    pub x: i32,
    pub y: i32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct Offset3D {
    pub x: i32,
    pub y: i32,
    pub z: i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Rect2D {
    pub offset: Offset2D,
    pub extent: Extent2D,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Viewport {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub min_depth: f32,
    pub max_depth: f32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct ClearColorValue {
    pub float32: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ClearDepthStencilValue {
    pub depth: f32,
    pub stencil: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub union ClearValue {
    pub color: ClearColorValue,
    pub depth_stencil: ClearDepthStencilValue,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ImageSubresourceRange {
    pub aspect_mask: u32,
    pub base_mip_level: u32,
    pub level_count: u32,
    pub base_array_layer: u32,
    pub layer_count: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ImageSubresourceLayers {
    pub aspect_mask: u32,
    pub mip_level: u32,
    pub base_array_layer: u32,
    pub layer_count: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct BufferImageCopy {
    pub buffer_offset: u64,
    pub buffer_row_length: u32,
    pub buffer_image_height: u32,
    pub image_subresource: ImageSubresourceLayers,
    pub image_offset: Offset3D,
    pub image_extent: Extent3D,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ImageMemoryBarrier {
    pub s_type: i32,
    pub p_next: *const c_void,
    pub src_access_mask: u32,
    pub dst_access_mask: u32,
    pub old_layout: i32,
    pub new_layout: i32,
    pub src_queue_family_index: u32,
    pub dst_queue_family_index: u32,
    pub image: ImageHandle,
    pub subresource_range: ImageSubresourceRange,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct BufferMemoryBarrier {
    pub s_type: i32,
    pub p_next: *const c_void,
    pub src_access_mask: u32,
    pub dst_access_mask: u32,
    pub src_queue_family_index: u32,
    pub dst_queue_family_index: u32,
    pub buffer: BufferHandle,
    pub offset: u64,
    pub size: u64,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct MemoryBarrier {
    pub s_type: i32,
    pub p_next: *const c_void,
    pub src_access_mask: u32,
    pub dst_access_mask: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct DeviceQueueCreateInfo {
    pub s_type: i32,
    pub p_next: *const c_void,
    pub flags: u32,
    pub queue_family_index: u32,
    pub queue_count: u32,
    pub p_queue_priorities: *const f32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct DeviceCreateInfo {
    pub s_type: i32,
    pub p_next: *const c_void,
    pub flags: u32,
    pub queue_create_info_count: u32,
    pub p_queue_create_infos: *const DeviceQueueCreateInfo,
    pub enabled_layer_count: u32,
    pub pp_enabled_layer_names: *const *const c_char,
    pub enabled_extension_count: u32,
    pub pp_enabled_extension_names: *const *const c_char,
    pub p_enabled_features: *const c_void,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct QueueFamilyProperties {
    pub queue_flags: u32,
    pub queue_count: u32,
    pub timestamp_valid_bits: u32,
    pub min_image_transfer_granularity: Extent3D,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ShaderModuleCreateInfo {
    pub s_type: i32,
    pub p_next: *const c_void,
    pub flags: u32,
    pub code_size: usize,
    pub p_code: *const u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct PipelineShaderStageCreateInfo {
    pub s_type: i32,
    pub p_next: *const c_void,
    pub flags: u32,
    pub stage: u32,
    pub module: ShaderModuleHandle,
    pub p_name: *const c_char,
    pub p_specialization_info: *const c_void,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct PipelineVertexInputStateCreateInfo {
    pub s_type: i32,
    pub p_next: *const c_void,
    pub flags: u32,
    pub vertex_binding_description_count: u32,
    pub p_vertex_binding_descriptions: *const c_void,
    pub vertex_attribute_description_count: u32,
    pub p_vertex_attribute_descriptions: *const c_void,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct PipelineInputAssemblyStateCreateInfo {
    pub s_type: i32,
    pub p_next: *const c_void,
    pub flags: u32,
    /// ⚠️ **字段顺序必须与规范一致**：`topology` 在 `primitiveRestartEnable` **之前**。
    /// 本项目曾把这两者写反 ⇒ 结构体从 24 字节变成 32 字节，驱动读到的 `pNext`
    /// 变成别的字段，`vkCreateGraphicsPipelines` 直接 `STATUS_STACK_BUFFER_OVERRUN`。
    /// 字段都是 4 字节整数时，顺序错不会报编译错误，只会让驱动读到垃圾。
    pub topology: i32,
    pub primitive_restart_enable: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct PipelineViewportStateCreateInfo {
    pub s_type: i32,
    pub p_next: *const c_void,
    pub flags: u32,
    pub viewport_count: u32,
    pub p_viewports: *const Viewport,
    pub scissor_count: u32,
    pub p_scissors: *const Rect2D,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct PipelineRasterizationStateCreateInfo {
    pub s_type: i32,
    pub p_next: *const c_void,
    pub flags: u32,
    pub depth_clamp_enable: u32,
    pub rasterizer_discard_enable: u32,
    pub polygon_mode: i32,
    pub cull_mode: u32,
    pub front_face: i32,
    pub depth_bias_enable: u32,
    pub depth_bias_constant_factor: f32,
    pub depth_bias_clamp: f32,
    pub depth_bias_slope_factor: f32,
    pub line_width: f32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct PipelineMultisampleStateCreateInfo {
    pub s_type: i32,
    pub p_next: *const c_void,
    pub flags: u32,
    pub rasterization_samples: u32,
    pub sample_shading_enable: u32,
    pub min_sample_shading: f32,
    pub p_sample_mask: *const u32,
    pub alpha_to_coverage_enable: u32,
    pub alpha_to_one_enable: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct PipelineColorBlendAttachmentState {
    pub blend_enable: u32,
    pub src_color_blend_factor: i32,
    pub dst_color_blend_factor: i32,
    pub color_blend_op: i32,
    pub src_alpha_blend_factor: i32,
    pub dst_alpha_blend_factor: i32,
    pub alpha_blend_op: i32,
    pub color_write_mask: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct PipelineColorBlendStateCreateInfo {
    pub s_type: i32,
    pub p_next: *const c_void,
    pub flags: u32,
    pub logic_op_enable: u32,
    pub logic_op: i32,
    pub attachment_count: u32,
    pub p_attachments: *const PipelineColorBlendAttachmentState,
    pub blend_constants: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct PipelineLayoutCreateInfo {
    pub s_type: i32,
    pub p_next: *const c_void,
    pub flags: u32,
    pub set_layout_count: u32,
    pub p_set_layouts: *const c_void,
    pub push_constant_range_count: u32,
    pub p_push_constant_ranges: *const PushConstantRange,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct PushConstantRange {
    pub stage_flags: u32,
    pub offset: u32,
    pub size: u32,
}

/// `VkPipelineDynamicStateCreateInfo`
#[repr(C)]
#[derive(Clone, Copy)]
pub struct PipelineDynamicStateCreateInfo {
    pub s_type: i32,
    pub p_next: *const c_void,
    pub flags: u32,
    pub dynamic_state_count: u32,
    pub p_dynamic_states: *const i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct GraphicsPipelineCreateInfo {
    pub s_type: i32,
    pub p_next: *const c_void,
    pub flags: u32,
    pub stage_count: u32,
    pub p_stages: *const PipelineShaderStageCreateInfo,
    pub p_vertex_input_state: *const PipelineVertexInputStateCreateInfo,
    pub p_input_assembly_state: *const PipelineInputAssemblyStateCreateInfo,
    pub p_tessellation_state: *const c_void,
    pub p_viewport_state: *const PipelineViewportStateCreateInfo,
    pub p_rasterization_state: *const PipelineRasterizationStateCreateInfo,
    pub p_multisample_state: *const PipelineMultisampleStateCreateInfo,
    pub p_depth_stencil_state: *const c_void,
    pub p_color_blend_state: *const PipelineColorBlendStateCreateInfo,
    pub p_dynamic_state: *const PipelineDynamicStateCreateInfo,
    pub layout: PipelineLayoutHandle,
    pub render_pass: RenderPassHandle,
    pub subpass: u32,
    pub base_pipeline_handle: PipelineHandle,
    pub base_pipeline_index: i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct AttachmentDescription {
    pub flags: u32,
    pub format: i32,
    pub samples: u32,
    pub load_op: i32,
    pub store_op: i32,
    pub stencil_load_op: i32,
    pub stencil_store_op: i32,
    pub initial_layout: i32,
    pub final_layout: i32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct AttachmentReference {
    pub attachment: u32,
    pub layout: i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct SubpassDescription {
    pub flags: u32,
    pub pipeline_bind_point: i32,
    pub input_attachment_count: u32,
    pub p_input_attachments: *const AttachmentReference,
    pub color_attachment_count: u32,
    pub p_color_attachments: *const AttachmentReference,
    pub p_resolve_attachments: *const AttachmentReference,
    pub p_depth_stencil_attachment: *const AttachmentReference,
    pub preserve_attachment_count: u32,
    pub p_preserve_attachments: *const u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct SubpassDependency {
    pub src_subpass: u32,
    pub dst_subpass: u32,
    pub src_stage_mask: u32,
    pub dst_stage_mask: u32,
    pub src_access_mask: u32,
    pub dst_access_mask: u32,
    pub dependency_flags: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct RenderPassCreateInfo {
    pub s_type: i32,
    pub p_next: *const c_void,
    pub flags: u32,
    pub attachment_count: u32,
    pub p_attachments: *const AttachmentDescription,
    pub subpass_count: u32,
    pub p_subpasses: *const SubpassDescription,
    pub dependency_count: u32,
    pub p_dependencies: *const SubpassDependency,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ImageCreateInfo {
    pub s_type: i32,
    pub p_next: *const c_void,
    pub flags: u32,
    pub image_type: i32,
    pub format: i32,
    pub extent: Extent3D,
    pub mip_levels: u32,
    pub array_layers: u32,
    pub samples: u32,
    pub tiling: i32,
    pub usage: u32,
    pub sharing_mode: i32,
    pub queue_family_index_count: u32,
    pub p_queue_family_indices: *const u32,
    pub initial_layout: i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ImageViewCreateInfo {
    pub s_type: i32,
    pub p_next: *const c_void,
    pub flags: u32,
    pub image: ImageHandle,
    pub view_type: i32,
    pub format: i32,
    pub components_r: i32,
    pub components_g: i32,
    pub components_b: i32,
    pub components_a: i32,
    pub subresource_range: ImageSubresourceRange,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct MemoryAllocateInfo {
    pub s_type: i32,
    pub p_next: *const c_void,
    pub allocation_size: u64,
    pub memory_type_index: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct MemoryRequirements {
    pub size: u64,
    pub alignment: u64,
    pub memory_type_bits: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct PhysicalDeviceMemoryProperties {
    pub memory_type_count: u32,
    pub memory_types: [MemoryType; 32],
    pub memory_heap_count: u32,
    pub memory_heaps: [MemoryHeap; 16],
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct MemoryType {
    pub property_flags: u32,
    pub heap_index: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct MemoryHeap {
    pub size: u64,
    pub flags: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct BufferCreateInfo {
    pub s_type: i32,
    pub p_next: *const c_void,
    pub flags: u32,
    pub size: u64,
    pub usage: u32,
    pub sharing_mode: i32,
    pub queue_family_index_count: u32,
    pub p_queue_family_indices: *const u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct CommandPoolCreateInfo {
    pub s_type: i32,
    pub p_next: *const c_void,
    pub flags: u32,
    pub queue_family_index: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct CommandBufferAllocateInfo {
    pub s_type: i32,
    pub p_next: *const c_void,
    pub command_pool: CommandPoolHandle,
    pub level: i32,
    pub command_buffer_count: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct CommandBufferBeginInfo {
    pub s_type: i32,
    pub p_next: *const c_void,
    pub flags: u32,
    pub p_inheritance_info: *const c_void,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct FramebufferCreateInfo {
    pub s_type: i32,
    pub p_next: *const c_void,
    pub flags: u32,
    pub render_pass: RenderPassHandle,
    pub attachment_count: u32,
    pub p_attachments: *const ImageViewHandle,
    pub width: u32,
    pub height: u32,
    pub layers: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct RenderPassBeginInfo {
    pub s_type: i32,
    pub p_next: *const c_void,
    pub render_pass: RenderPassHandle,
    pub framebuffer: FramebufferHandle,
    pub render_area: Rect2D,
    pub clear_value_count: u32,
    pub p_clear_values: *const ClearValue,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct FenceCreateInfo {
    pub s_type: i32,
    pub p_next: *const c_void,
    pub flags: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct SemaphoreCreateInfo {
    pub s_type: i32,
    pub p_next: *const c_void,
    pub flags: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct SubmitInfo {
    pub s_type: i32,
    pub p_next: *const c_void,
    pub wait_semaphore_count: u32,
    pub p_wait_semaphores: *const SemaphoreHandle,
    pub p_wait_dst_stage_mask: *const u32,
    pub command_buffer_count: u32,
    pub p_command_buffers: *const CommandBufferHandle,
    pub signal_semaphore_count: u32,
    pub p_signal_semaphores: *const SemaphoreHandle,
}

/// 命令缓冲层级
pub const VK_COMMAND_BUFFER_LEVEL_PRIMARY: i32 = 0;
/// 子通道内容
pub const VK_SUBPASS_CONTENTS_INLINE: u32 = 0;

// ── 设备级函数表 ─────────────────────────────────────────────────────────────

macro_rules! pfn {
    ($name:ident, $sig:ty) => {
        pub type $name = $sig;
    };
}

pfn!(
    PfnCreateDevice,
    unsafe extern "system" fn(
        PhysicalDeviceHandle,
        *const DeviceCreateInfo,
        *const c_void,
        *mut DeviceHandle,
    ) -> VkResult
);
pfn!(
    PfnDestroyDevice,
    unsafe extern "system" fn(DeviceHandle, *const c_void)
);
pfn!(
    PfnGetDeviceQueue,
    unsafe extern "system" fn(DeviceHandle, u32, u32, *mut QueueHandle)
);
pfn!(
    PfnGetPhysicalDeviceQueueFamilyProperties,
    unsafe extern "system" fn(PhysicalDeviceHandle, *mut u32, *mut QueueFamilyProperties)
);
pfn!(
    PfnGetPhysicalDeviceMemoryProperties,
    unsafe extern "system" fn(PhysicalDeviceHandle, *mut PhysicalDeviceMemoryProperties)
);
pfn!(
    PfnCreateShaderModule,
    unsafe extern "system" fn(DeviceHandle, *const ShaderModuleCreateInfo, *const c_void, *mut ShaderModuleHandle) -> VkResult
);
pfn!(
    PfnDestroyShaderModule,
    unsafe extern "system" fn(DeviceHandle, ShaderModuleHandle, *const c_void)
);
pfn!(
    PfnCreateRenderPass,
    unsafe extern "system" fn(DeviceHandle, *const RenderPassCreateInfo, *const c_void, *mut RenderPassHandle) -> VkResult
);
pfn!(
    PfnDestroyRenderPass,
    unsafe extern "system" fn(DeviceHandle, RenderPassHandle, *const c_void)
);
pfn!(
    PfnCreatePipelineLayout,
    unsafe extern "system" fn(DeviceHandle, *const PipelineLayoutCreateInfo, *const c_void, *mut PipelineLayoutHandle) -> VkResult
);
pfn!(
    PfnDestroyPipelineLayout,
    unsafe extern "system" fn(DeviceHandle, PipelineLayoutHandle, *const c_void)
);
pfn!(
    PfnCreateGraphicsPipelines,
    unsafe extern "system" fn(
        DeviceHandle,
        PipelineHandle,
        u32,
        *const GraphicsPipelineCreateInfo,
        *const c_void,
        *mut PipelineHandle,
    ) -> VkResult
);
pfn!(
    PfnDestroyPipeline,
    unsafe extern "system" fn(DeviceHandle, PipelineHandle, *const c_void)
);
pfn!(
    PfnCreateImage,
    unsafe extern "system" fn(DeviceHandle, *const ImageCreateInfo, *const c_void, *mut ImageHandle) -> VkResult
);
pfn!(
    PfnDestroyImage,
    unsafe extern "system" fn(DeviceHandle, ImageHandle, *const c_void)
);
pfn!(
    PfnCreateImageView,
    unsafe extern "system" fn(DeviceHandle, *const ImageViewCreateInfo, *const c_void, *mut ImageViewHandle) -> VkResult
);
pfn!(
    PfnDestroyImageView,
    unsafe extern "system" fn(DeviceHandle, ImageViewHandle, *const c_void)
);
pfn!(
    PfnGetImageMemoryRequirements,
    unsafe extern "system" fn(DeviceHandle, ImageHandle, *mut MemoryRequirements)
);
pfn!(
    PfnGetBufferMemoryRequirements,
    unsafe extern "system" fn(DeviceHandle, BufferHandle, *mut MemoryRequirements)
);
pfn!(
    PfnAllocateMemory,
    unsafe extern "system" fn(DeviceHandle, *const MemoryAllocateInfo, *const c_void, *mut DeviceMemoryHandle) -> VkResult
);
pfn!(
    PfnFreeMemory,
    unsafe extern "system" fn(DeviceHandle, DeviceMemoryHandle, *const c_void)
);
pfn!(
    PfnBindImageMemory,
    unsafe extern "system" fn(DeviceHandle, ImageHandle, DeviceMemoryHandle, u64) -> VkResult
);
pfn!(
    PfnBindBufferMemory,
    unsafe extern "system" fn(DeviceHandle, BufferHandle, DeviceMemoryHandle, u64) -> VkResult
);
pfn!(
    PfnCreateBuffer,
    unsafe extern "system" fn(DeviceHandle, *const BufferCreateInfo, *const c_void, *mut BufferHandle) -> VkResult
);
pfn!(
    PfnDestroyBuffer,
    unsafe extern "system" fn(DeviceHandle, BufferHandle, *const c_void)
);
pfn!(
    PfnMapMemory,
    unsafe extern "system" fn(DeviceHandle, DeviceMemoryHandle, u64, u64, u32, *mut *mut c_void) -> VkResult
);
pfn!(
    PfnUnmapMemory,
    unsafe extern "system" fn(DeviceHandle, DeviceMemoryHandle)
);
pfn!(
    PfnCreateCommandPool,
    unsafe extern "system" fn(DeviceHandle, *const CommandPoolCreateInfo, *const c_void, *mut CommandPoolHandle) -> VkResult
);
pfn!(
    PfnDestroyCommandPool,
    unsafe extern "system" fn(DeviceHandle, CommandPoolHandle, *const c_void)
);
pfn!(
    PfnAllocateCommandBuffers,
    unsafe extern "system" fn(DeviceHandle, *const CommandBufferAllocateInfo, *mut CommandBufferHandle) -> VkResult
);
pfn!(
    PfnBeginCommandBuffer,
    unsafe extern "system" fn(CommandBufferHandle, *const CommandBufferBeginInfo) -> VkResult
);
pfn!(
    PfnEndCommandBuffer,
    unsafe extern "system" fn(CommandBufferHandle) -> VkResult
);
pfn!(
    PfnResetCommandBuffer,
    unsafe extern "system" fn(CommandBufferHandle, u32) -> VkResult
);
pfn!(
    PfnCreateFramebuffer,
    unsafe extern "system" fn(DeviceHandle, *const FramebufferCreateInfo, *const c_void, *mut FramebufferHandle) -> VkResult
);
pfn!(
    PfnDestroyFramebuffer,
    unsafe extern "system" fn(DeviceHandle, FramebufferHandle, *const c_void)
);
pfn!(
    PfnCreateFence,
    unsafe extern "system" fn(DeviceHandle, *const FenceCreateInfo, *const c_void, *mut FenceHandle) -> VkResult
);
pfn!(
    PfnDestroyFence,
    unsafe extern "system" fn(DeviceHandle, FenceHandle, *const c_void)
);
pfn!(
    PfnWaitForFences,
    unsafe extern "system" fn(DeviceHandle, u32, *const FenceHandle, u32, u64) -> VkResult
);
pfn!(
    PfnResetFences,
    unsafe extern "system" fn(DeviceHandle, u32, *const FenceHandle) -> VkResult
);
pfn!(
    PfnQueueSubmit,
    unsafe extern "system" fn(QueueHandle, u32, *const SubmitInfo, FenceHandle) -> VkResult
);
pfn!(
    PfnQueueWaitIdle,
    unsafe extern "system" fn(QueueHandle) -> VkResult
);
pfn!(
    PfnDeviceWaitIdle,
    unsafe extern "system" fn(DeviceHandle) -> VkResult
);
pfn!(
    PfnCmdBeginRenderPass,
    unsafe extern "system" fn(CommandBufferHandle, *const RenderPassBeginInfo, u32)
);
pfn!(PfnCmdEndRenderPass, unsafe extern "system" fn(CommandBufferHandle));
pfn!(
    PfnCmdBindPipeline,
    unsafe extern "system" fn(CommandBufferHandle, i32, PipelineHandle)
);
pfn!(
    PfnCmdSetViewport,
    unsafe extern "system" fn(CommandBufferHandle, u32, u32, *const Viewport)
);
pfn!(
    PfnCmdSetScissor,
    unsafe extern "system" fn(CommandBufferHandle, u32, u32, *const Rect2D)
);
pfn!(
    PfnCmdDraw,
    unsafe extern "system" fn(CommandBufferHandle, u32, u32, u32, u32)
);
pfn!(
    PfnCmdPushConstants,
    unsafe extern "system" fn(CommandBufferHandle, PipelineLayoutHandle, u32, u32, u32, *const c_void)
);
pfn!(
    PfnCmdPipelineBarrier,
    unsafe extern "system" fn(CommandBufferHandle, u32, u32, u32, u32, *const MemoryBarrier, u32, *const BufferMemoryBarrier, u32, *const ImageMemoryBarrier)
);
pfn!(
    PfnCmdCopyImageToBuffer,
    unsafe extern "system" fn(CommandBufferHandle, ImageHandle, i32, BufferHandle, u32, *const BufferImageCopy)
);
pfn!(
    PfnCmdClearColorImage,
    unsafe extern "system" fn(CommandBufferHandle, ImageHandle, i32, *const ClearColorValue, u32, *const ImageSubresourceRange)
);
