//! 离屏 **GPU 几何渲染器**（M3a-T3）：`DrawList` → 顶点流 → 顶点缓冲 → GPU 光栅化 → 回读像素。
//!
//! 这是 M3a 的终点：把「非文本绘制命令」真的交给 Vulkan 画出来，并让结果**与 CPU 后端
//! 逐像素可比**（对照测试见 `tests/gpu_vs_cpu.rs`）。三个模块的分工：
//!
//! | 层 | 模块 | 职责 |
//! |---|---|---|
//! | 翻译（纯逻辑） | [`crate::gpu_geom`] | `DrawList` → [`GpuVertex`] 流 + CPU 侧裁剪 |
//! | 着色器 | [`crate::spirv`] | 透传属性 + 按顶点属性上的 `rect`/`radius_kind` 逐像素判定 |
//! | **本模块** | `gpu_render` | 顶点缓冲、静态管线、离屏图像、命令录制、回读 |
//!
//! ## 一帧的流程
//!
//! ```text
//!  ① 入口检查：裁剪栈平衡（与 CPU 同一判据）；`unsupported` 非空 ⇒ Unsupported
//!  ② gpu_geom::build_stream ⇒ 顶点流（CPU 侧已裁剪）
//!  ③ 顶点写进 HOST_VISIBLE 顶点缓冲（每帧重传；容量不足时重建）
//!  ④ 录制：beginRenderPass(CLEAR) → bindPipeline → bindVertexBuffers → draw(vertex_count)
//!          → endRenderPass → 屏障(TRANSFER_SRC) → copyImageToBuffer
//!  ⑤ vkQueueSubmit + 等栅栏（有限超时）
//!  ⑥ map 暂存缓冲读回 RGBA8
//! ```
//!
//! ## 为什么不复用 `offscreen.rs`
//!
//! 它做的是同一件事，但有两点不对：
//! 1. 它在录制时调 `vkCmdSetViewport`/`vkCmdSetScissor`（它的管线是**动态** viewport）；
//!    本模块用**静态** viewport/scissor —— 实测动态版在本机 Intel 驱动上画不出任何像素，
//!    而对着静态状态的管线发动态设置命令会触发校验层报错（`tests/vbo_probe.rs` 有实测记录）；
//! 2. 它没有 `vkCmdBindVertexBuffers`（M2a 的顶点来自着色器常量表）。
//!
//! 所以本模块自己持有资源。**资源全部包成 [`VkObject`]**（析构顺序由字段顺序保证：
//! 对象在前、`VkDevice` 在最后 ⇒ 设备最后销毁）。
//!
//! ## 与 CPU 逐像素可比的三条前提
//!
//! - **颜色格式是 `R8G8B8A8_UNORM`（不是 `_SRGB`）**：CPU 参考实现不做 gamma 转换，
//!   用 SRGB 格式会让 GPU 多一次编码 ⇒ 两边永远对不上；
//! - **alpha 混合按 `src-alpha / one-minus-src-alpha`**（[`crate::device`] 里既有的管线状态），
//!   与 `null.rs::blend_cov`（覆盖率 = 1）同式；
//! - **不预乘**：顶点颜色就是 `Color` 的原值（见 [`crate::gpu_geom`] 的颜色约定）。

use std::ffi::c_void;

use deer_gpu::{Color, DrawList, Extent, GpuError, GpuResult};

use crate::device::{
    vk_result_name, DeviceFns, Pipeline, PipelineLayout, RenderPass, ShaderModule, VertexAttr, VkDevice,
};
use crate::ffi;
use crate::ffi_dev as vk;
use crate::gpu_geom::{self, GpuVertex};
use crate::spirv;

/// `VK_FORMAT_R32_SFLOAT`（单个 `float`）——`radius_kind` 用它。
///
/// **为什么在这里定义**：`ffi_dev` 目前只声明了 `R32G32_SFLOAT` / `R32G32B32_SFLOAT` /
/// `R32G32B32A32_SFLOAT`（`vbo_probe.rs` 只需要 vec2）。规范里 `VK_FORMAT_R32_SFLOAT = 100`。
/// 放在本模块而不是随手写 100：名字带来源，且只在这一处出现。
const VK_FORMAT_R32_SFLOAT: i32 = 100;

/// 颜色附件的格式：**线性 UNORM**（见模块文档「三条前提」）。
const COLOR_FORMAT: i32 = vk::VK_FORMAT_R8G8B8A8_UNORM;

/// 等栅栏的超时（1 秒）：驱动出问题时**宁可失败，也不要永久挂住**（比失败更难排查）。
const TIMEOUT_NS: u64 = 1_000_000_000;

/// 顶点缓冲的最小分配（即使一帧只有 6 个顶点也按 4 KiB 分配，避免每帧重建）。
const MIN_VERTEX_BYTES: u64 = 4096;

/// 顶点属性表：`location` 与**顶点偏移**都由 [`GpuVertex`] 的锁布局算出（`offset_of!`），
/// 所以它不可能与 `gpu_geom` 的 `#[repr(C)]` 布局漂移。
///
/// `tests/gpu_vs_cpu.rs::vertex_layout_matches_the_hand_written_attribute_offsets`
/// 另外钉住**字面数字**（stride 44 / 0 / 8 / 24 / 28）—— 布局若被改动，两个地方都会红。
fn vertex_attrs() -> [VertexAttr; 4] {
    [
        VertexAttr {
            location: 0,
            format: vk::VK_FORMAT_R32G32_SFLOAT,
            offset: std::mem::offset_of!(GpuVertex, pos) as u32,
        },
        VertexAttr {
            location: 1,
            format: vk::VK_FORMAT_R32G32B32A32_SFLOAT,
            offset: std::mem::offset_of!(GpuVertex, rect) as u32,
        },
        VertexAttr {
            location: 2,
            format: VK_FORMAT_R32_SFLOAT,
            offset: std::mem::offset_of!(GpuVertex, radius_kind) as u32,
        },
        VertexAttr {
            location: 3,
            format: vk::VK_FORMAT_R32G32B32A32_SFLOAT,
            offset: std::mem::offset_of!(GpuVertex, color) as u32,
        },
    ]
}

/// 对象销毁/内存释放的函数形态。
///
/// 本项目里所有 `vk::*Handle` 都是 `*mut c_void` 的别名、所有 destroy/free 都是
/// `(DeviceHandle, 句柄, 分配器)` 且不返回错误码 —— 所以**一个**包装类型就够
/// （`offscreen.rs` 里为 6 种对象各写一遍 `Drop`，这里等价但少 100 行样板；
/// 代价是类型上区分不了句柄种类，用[`wrap_create`] 的 `what` 参数在报错里补回来）。
type DestroyFn = unsafe extern "system" fn(vk::DeviceHandle, *mut c_void, *const c_void);

/// 一个「有句柄 + 有销毁函数」的 Vulkan 对象：`Drop` 时销毁。
///
/// 不带 `what` 字段：对象名只在**创建失败**时要紧（那时对象还不存在），所以它是
/// [`wrap_create`] 的参数；留成字段没人读只会招来 `dead_code`。
struct VkObject {
    handle: *mut c_void,
    device: vk::DeviceHandle,
    destroy: DestroyFn,
}

impl VkObject {
    fn handle(&self) -> *mut c_void {
        self.handle
    }
}

impl Drop for VkObject {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            // SAFETY: 句柄由本模块在本设备上创建且尚未销毁；`destroy` 是对应的销毁函数；
            // 设备比本对象活得久（`GpuGeometryRenderer` 里 `device` 声明在最后 ⇒ 最后析构）。
            unsafe { (self.destroy)(self.device, self.handle, std::ptr::null()) };
            self.handle = std::ptr::null_mut();
        }
    }
}

/// 顶点缓冲 + 它绑定的内存。
///
/// **字段顺序 = 析构顺序**：`buffer` 在前 ⇒ 先 `vkDestroyBuffer`、再 `vkFreeMemory`
/// （内存不能先于绑定它的缓冲消失）。
struct VertexBuffer {
    buffer: VkObject,
    memory: VkObject,
    capacity: u64,
}

/// 把 `rc` 变成 `GpuResult<()>`（错误信息里带上调用名与驱动返回码的名字）。
fn check(what: &str, rc: i32) -> GpuResult<()> {
    if rc == ffi::VK_SUCCESS {
        Ok(())
    } else {
        Err(GpuError::Driver {
            code: rc,
            message: format!("{what} 失败：{}", vk_result_name(rc)),
        })
    }
}

/// 把一个「创建对象」调用的结果包成 [`VkObject`]。
fn wrap_create(
    what: &'static str,
    rc: i32,
    handle: *mut c_void,
    device: vk::DeviceHandle,
    destroy: DestroyFn,
) -> GpuResult<VkObject> {
    check(what, rc)?;
    if handle.is_null() {
        // 驱动返回成功却没写输出参数 —— 几乎总是「我们给的结构体与驱动理解的不一致」。
        return Err(GpuError::Driver {
            code: rc,
            message: format!("{what} 返回成功但句柄为空（结构体或参数不符）"),
        });
    }
    Ok(VkObject {
        handle,
        device,
        destroy,
    })
}

/// 挑一个同时满足 `required` 所有位的内存类型。
///
/// 与 `offscreen.rs::pick_memory_type` 同逻辑（那个函数不 `pub`，而本模块不允许改它）
/// —— 属性来自**物理设备**，逻辑设备上拿不到，所以由调用方传进来。
fn pick_memory_type(
    props: &vk::PhysicalDeviceMemoryProperties,
    type_bits: u32,
    required: u32,
) -> GpuResult<u32> {
    for i in 0..props.memory_type_count.min(32) {
        let m = &props.memory_types[i as usize];
        if type_bits & (1 << i) == 0 {
            continue;
        }
        if m.property_flags & required == required {
            return Ok(i);
        }
    }
    Err(GpuError::Unsupported(format!(
        "找不到满足属性 {required:#x} 的内存类型（资源的 type_bits = {type_bits:#x}）"
    )))
}

/// 分配一块设备内存。
fn alloc_memory(
    what: &'static str,
    device: vk::DeviceHandle,
    fns: &DeviceFns,
    size: u64,
    memory_type_index: u32,
) -> GpuResult<VkObject> {
    let info = vk::MemoryAllocateInfo {
        s_type: vk::VK_STRUCTURE_TYPE_MEMORY_ALLOCATE_INFO,
        p_next: std::ptr::null(),
        allocation_size: size,
        memory_type_index,
    };
    let mut handle: vk::DeviceMemoryHandle = std::ptr::null_mut();
    // SAFETY: `info` 在栈上存活；`handle` 是可写输出；函数指针来自成功解析的 loader。
    let rc = unsafe { (fns.allocate_memory)(device, &info, std::ptr::null(), &mut handle) };
    if rc != ffi::VK_SUCCESS {
        return Err(GpuError::Driver {
            code: rc,
            message: format!("{what} 失败（size={size}）：{}", vk_result_name(rc)),
        });
    }
    wrap_create(what, rc, handle, device, fns.free_memory)
}

/// 建一块缓冲 + 主机可见内存（顶点缓冲与回读暂存都用它）。
fn create_host_buffer(
    what: &'static str,
    device: vk::DeviceHandle,
    fns: &DeviceFns,
    mem_props: &vk::PhysicalDeviceMemoryProperties,
    size: u64,
    usage: u32,
) -> GpuResult<(VkObject, VkObject)> {
    let info = vk::BufferCreateInfo {
        s_type: vk::VK_STRUCTURE_TYPE_BUFFER_CREATE_INFO,
        p_next: std::ptr::null(),
        flags: 0,
        size,
        usage,
        sharing_mode: vk::VK_SHARING_MODE_EXCLUSIVE,
        queue_family_index_count: 0,
        p_queue_family_indices: std::ptr::null(),
    };
    let mut handle: vk::BufferHandle = std::ptr::null_mut();
    // SAFETY: 同上。
    let rc = unsafe { (fns.create_buffer)(device, &info, std::ptr::null(), &mut handle) };
    let buffer = wrap_create(what, rc, handle, device, fns.destroy_buffer)?;

    let mut req = std::mem::MaybeUninit::<vk::MemoryRequirements>::uninit();
    // SAFETY: 该函数完整写入结构体。
    unsafe { (fns.get_buffer_memory_requirements)(device, buffer.handle(), req.as_mut_ptr()) };
    let req = unsafe { req.assume_init() };
    let index = pick_memory_type(
        mem_props,
        req.memory_type_bits,
        vk::VK_MEMORY_PROPERTY_HOST_VISIBLE_BIT | vk::VK_MEMORY_PROPERTY_HOST_COHERENT_BIT,
    )?;
    let memory = alloc_memory(what, device, fns, req.size, index)?;
    // SAFETY: 缓冲与内存都是本设备的新对象，尺寸匹配；offset 0 合法。
    check("vkBindBufferMemory", unsafe {
        (fns.bind_buffer_memory)(device, buffer.handle(), memory.handle(), 0)
    })?;
    Ok((buffer, memory))
}

/// 离屏 GPU 几何渲染器。
///
/// ## 字段顺序（**不要重排**）
///
/// 资源在前、`device` 最后 ⇒ Rust 按声明顺序析构 ⇒ **`VkDevice` 最后销毁**，
/// 所有资源都在设备仍存活时被销毁（否则校验层会报 `leaked objects` 或直接崩）。
///
/// ## 为什么允许 `dead_code`
///
/// 其中几个字段（`layout` / `vs` / `fs` / `image_memory` / `view` / `pool` / `device`）
/// **从不被读取** —— 它们存在只为**所有权**：`Drop` 要在正确的顺序上把对象销毁。
/// `allow(dead_code)` 是这里的正确表达；**删掉任何一个都会漏资源或让销毁顺序变错**。
#[allow(dead_code)]
pub struct GpuGeometryRenderer {
    pass: RenderPass,
    pipeline: Pipeline,
    layout: PipelineLayout,
    vs: ShaderModule,
    fs: ShaderModule,
    image: VkObject,
    image_memory: VkObject,
    view: VkObject,
    framebuffer: VkObject,
    pool: VkObject,
    cmd: vk::CommandBufferHandle,
    /// 顶点缓冲（**惰性创建**：一帧都没画过非空几何时不分配）。
    vertex: Option<VertexBuffer>,
    staging: VkObject,
    staging_memory: VkObject,
    /// 回读暂存的字节数（= 宽 × 高 × 4）。
    staging_size: u64,
    fence: VkObject,
    fns: DeviceFns,
    device_handle: vk::DeviceHandle,
    queue: vk::QueueHandle,
    mem_props: vk::PhysicalDeviceMemoryProperties,
    extent: Extent,
    clear: [f32; 4],
    unsupported: Vec<String>,
    /// **必须最后**（最后析构）。
    device: VkDevice,
}

impl GpuGeometryRenderer {
    /// 建一整套离屏 GPU 渲染设施（并打开 Vulkan 设备）。
    ///
    /// `extent` 为 0 时按 **1** 处理（与 CPU `Framebuffer::new(..max(1))` 同一约定）——
    /// Vulkan 图像不能是 0 宽高，而 CPU 能「画」1×1，所以这里也按 1×1 渲染，两边仍可比。
    /// [`GpuGeometryRenderer::extent`] 返回的是**实际**渲染尺寸。
    pub fn new(adapter_index: usize, extent: Extent, clear: Color) -> GpuResult<GpuGeometryRenderer> {
        let extent = Extent {
            width: extent.width.max(1),
            height: extent.height.max(1),
        };
        let device = VkDevice::open(adapter_index)?;
        let fns = *device.fns();
        let device_handle = device.handle();
        let mem_props = *device.memory_properties();

        // ① 着色器：顶点属性透传 + 按 `rect`/`radius_kind` 逐像素判定（M3a-T1）
        let vs = device.create_shader_module(&spirv::vertex_shader_rect_attrs())?;
        let fs = device.create_shader_module(&spirv::fragment_shader_rect_shape())?;

        // ② 渲染通道：清屏 + 离开通道即 `TRANSFER_SRC_OPTIMAL`（好直接回读）
        let pass = device.create_render_pass(
            COLOR_FORMAT,
            vk::VK_ATTACHMENT_LOAD_OP_CLEAR,
            vk::VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL,
        )?;

        // 管线布局：**不含推送常量** —— 顶点属性方案，刻意绕开 M2a 的推送常量地雷
        // （`spirv.rs::vertex_shader_rect_pushconstant` 的说明）。
        let layout = device.create_pipeline_layout(None)?;

        // ③ 管线：静态 viewport/scissor = 整幅 extent；真实顶点输入（binding stride 44）
        let entry = c"main";
        let stages = [
            vk::PipelineShaderStageCreateInfo {
                s_type: vk::VK_STRUCTURE_TYPE_PIPELINE_SHADER_STAGE_CREATE_INFO,
                p_next: std::ptr::null(),
                flags: 0,
                stage: vk::VK_SHADER_STAGE_VERTEX_BIT,
                module: vs.handle(),
                p_name: entry.as_ptr(),
                p_specialization_info: std::ptr::null(),
            },
            vk::PipelineShaderStageCreateInfo {
                s_type: vk::VK_STRUCTURE_TYPE_PIPELINE_SHADER_STAGE_CREATE_INFO,
                p_next: std::ptr::null(),
                flags: 0,
                stage: vk::VK_SHADER_STAGE_FRAGMENT_BIT,
                module: fs.handle(),
                p_name: entry.as_ptr(),
                p_specialization_info: std::ptr::null(),
            },
        ];
        let pipeline = device.create_vertex_pipeline(
            &stages,
            &layout,
            &pass,
            vk::Extent2D {
                width: extent.width,
                height: extent.height,
            },
            std::mem::size_of::<GpuVertex>() as u32,
            &vertex_attrs(),
        )?;

        // ④ 离屏图像（DEVICE_LOCAL：颜色附件 | 传输源）
        let img_info = vk::ImageCreateInfo {
            s_type: vk::VK_STRUCTURE_TYPE_IMAGE_CREATE_INFO,
            p_next: std::ptr::null(),
            flags: 0,
            image_type: vk::VK_IMAGE_TYPE_2D,
            format: COLOR_FORMAT,
            extent: vk::Extent3D {
                width: extent.width,
                height: extent.height,
                depth: 1,
            },
            mip_levels: 1,
            array_layers: 1,
            samples: vk::VK_SAMPLE_COUNT_1_BIT,
            tiling: vk::VK_IMAGE_TILING_OPTIMAL,
            usage: vk::VK_IMAGE_USAGE_COLOR_ATTACHMENT_BIT | vk::VK_IMAGE_USAGE_TRANSFER_SRC_BIT,
            sharing_mode: vk::VK_SHARING_MODE_EXCLUSIVE,
            queue_family_index_count: 0,
            p_queue_family_indices: std::ptr::null(),
            initial_layout: vk::VK_IMAGE_LAYOUT_UNDEFINED,
        };
        let mut image_handle: vk::ImageHandle = std::ptr::null_mut();
        // SAFETY: 结构体在栈上存活；句柄是可写输出。
        let rc = unsafe { (fns.create_image)(device_handle, &img_info, std::ptr::null(), &mut image_handle) };
        let image = wrap_create("vkCreateImage", rc, image_handle, device_handle, fns.destroy_image)?;

        let mut image_req = std::mem::MaybeUninit::<vk::MemoryRequirements>::uninit();
        // SAFETY: 该函数完整写入结构体。
        unsafe { (fns.get_image_memory_requirements)(device_handle, image.handle(), image_req.as_mut_ptr()) };
        let image_req = unsafe { image_req.assume_init() };
        let image_mem_index = pick_memory_type(
            &mem_props,
            image_req.memory_type_bits,
            vk::VK_MEMORY_PROPERTY_DEVICE_LOCAL_BIT,
        )?;
        let image_memory = alloc_memory(
            "vkAllocateMemory(image)",
            device_handle,
            &fns,
            image_req.size,
            image_mem_index,
        )?;
        // SAFETY: 图像与内存都是新对象，尺寸匹配；offset 0 合法。
        check("vkBindImageMemory", unsafe {
            (fns.bind_image_memory)(device_handle, image.handle(), image_memory.handle(), 0)
        })?;

        // ⑤ 视图 + 帧缓冲
        let view_info = vk::ImageViewCreateInfo {
            s_type: vk::VK_STRUCTURE_TYPE_IMAGE_VIEW_CREATE_INFO,
            p_next: std::ptr::null(),
            flags: 0,
            image: image.handle(),
            view_type: vk::VK_IMAGE_VIEW_TYPE_2D,
            format: COLOR_FORMAT,
            components_r: vk::VK_COMPONENT_SWIZZLE_IDENTITY,
            components_g: vk::VK_COMPONENT_SWIZZLE_IDENTITY,
            components_b: vk::VK_COMPONENT_SWIZZLE_IDENTITY,
            components_a: vk::VK_COMPONENT_SWIZZLE_IDENTITY,
            subresource_range: color_range(),
        };
        let mut view_handle: vk::ImageViewHandle = std::ptr::null_mut();
        // SAFETY: 同上。
        let rc = unsafe { (fns.create_image_view)(device_handle, &view_info, std::ptr::null(), &mut view_handle) };
        let view = wrap_create("vkCreateImageView", rc, view_handle, device_handle, fns.destroy_image_view)?;

        let fb_info = vk::FramebufferCreateInfo {
            s_type: vk::VK_STRUCTURE_TYPE_FRAMEBUFFER_CREATE_INFO,
            p_next: std::ptr::null(),
            flags: 0,
            render_pass: pass.handle(),
            attachment_count: 1,
            p_attachments: &view.handle(),
            width: extent.width,
            height: extent.height,
            layers: 1,
        };
        let mut fb_handle: vk::FramebufferHandle = std::ptr::null_mut();
        // SAFETY: 同上。
        let rc = unsafe { (fns.create_framebuffer)(device_handle, &fb_info, std::ptr::null(), &mut fb_handle) };
        let framebuffer = wrap_create("vkCreateFramebuffer", rc, fb_handle, device_handle, fns.destroy_framebuffer)?;

        // ⑥ 命令池 + 一个主命令缓冲
        let pool_info = vk::CommandPoolCreateInfo {
            s_type: vk::VK_STRUCTURE_TYPE_COMMAND_POOL_CREATE_INFO,
            p_next: std::ptr::null(),
            flags: vk::VK_COMMAND_POOL_CREATE_RESET_COMMAND_BUFFER_BIT,
            queue_family_index: device.queue_family_index(),
        };
        let mut pool_handle: vk::CommandPoolHandle = std::ptr::null_mut();
        // SAFETY: 同上。
        let rc = unsafe { (fns.create_command_pool)(device_handle, &pool_info, std::ptr::null(), &mut pool_handle) };
        let pool = wrap_create("vkCreateCommandPool", rc, pool_handle, device_handle, fns.destroy_command_pool)?;

        let alloc_info = vk::CommandBufferAllocateInfo {
            s_type: vk::VK_STRUCTURE_TYPE_COMMAND_BUFFER_ALLOCATE_INFO,
            p_next: std::ptr::null(),
            command_pool: pool.handle(),
            level: vk::VK_COMMAND_BUFFER_LEVEL_PRIMARY,
            command_buffer_count: 1,
        };
        let mut cmd: vk::CommandBufferHandle = std::ptr::null_mut();
        // SAFETY: 池有效；`cmd` 是可写输出。
        check("vkAllocateCommandBuffers", unsafe {
            (fns.allocate_command_buffers)(device_handle, &alloc_info, &mut cmd)
        })?;
        if cmd.is_null() {
            return Err(GpuError::Driver {
                code: 0,
                message: "vkAllocateCommandBuffers 返回成功但命令缓冲为空".to_string(),
            });
        }

        // ⑦ 回读暂存（HOST_VISIBLE | HOST_COHERENT ⇒ 不需要显式 flush）
        let staging_size = (extent.width as u64) * (extent.height as u64) * 4;
        let (staging, staging_memory) = create_host_buffer(
            "vkCreateBuffer(staging)",
            device_handle,
            &fns,
            &mem_props,
            staging_size,
            vk::VK_BUFFER_USAGE_TRANSFER_DST_BIT,
        )?;

        // ⑧ 栅栏（每帧复用：提交前 reset、提交后等它）
        let fence_info = vk::FenceCreateInfo {
            s_type: vk::VK_STRUCTURE_TYPE_FENCE_CREATE_INFO,
            p_next: std::ptr::null(),
            flags: 0, // 不预设 signaled ⇒ 第一次等就是真的等 GPU
        };
        let mut fence_handle: vk::FenceHandle = std::ptr::null_mut();
        // SAFETY: 同上。
        let rc = unsafe { (fns.create_fence)(device_handle, &fence_info, std::ptr::null(), &mut fence_handle) };
        let fence = wrap_create("vkCreateFence", rc, fence_handle, device_handle, fns.destroy_fence)?;

        Ok(GpuGeometryRenderer {
            pass,
            pipeline,
            layout,
            vs,
            fs,
            image,
            image_memory,
            view,
            framebuffer,
            pool,
            cmd,
            vertex: None,
            staging,
            staging_memory,
            staging_size,
            fence,
            fns,
            device_handle,
            queue: device.queue(),
            mem_props,
            extent,
            clear: [
                clear.r as f32 / 255.0,
                clear.g as f32 / 255.0,
                clear.b as f32 / 255.0,
                clear.a,
            ],
            unsupported: Vec::new(),
            device,
        })
    }

    /// **实际**渲染尺寸（请求 0 尺寸时是 1×1，见 [`GpuGeometryRenderer::new`]）。
    pub fn extent(&self) -> Extent {
        self.extent
    }

    /// 上一帧里**未能翻译**的命令说明（目前只有 `DrawCmd::Text`）。
    ///
    /// 与 [`GpuGeometryRenderer::render`] 的错误配套：`render` 返回 `Unsupported` 时，
    /// 这里能看到具体是哪几条、什么内容。
    pub fn unsupported(&self) -> &[String] {
        &self.unsupported
    }

    /// 画一帧并回读 RGBA8（长度 = 宽 × 高 × 4，行优先、无 padding）。
    ///
    /// ## 报错策略（与 CPU 后端对齐）
    ///
    /// - **裁剪栈不平衡**（`!list.clip_balanced()`，帧末净计数）⇒ `GpuError::Driver`。
    ///   判据与 `null.rs:227-236`（`CpuFrame::record` / `CpuRenderer::render`）**完全相同**。
    ///   刻意**不用** [`gpu_geom::GpuStream::clip_unbalanced`] 当报错条件 —— 它更严
    ///   （额外拒绝「多出的 `PopClip`」这种 CPU 画得出来的列表），拿它报错会让 GPU 拒收
    ///   CPU 能画的输入，两边行为不再可比；那个字段是**诊断**用的；
    /// - **`unsupported` 非空**（文本）⇒ `GpuError::Unsupported` + [`Self::unsupported`] 可查；
    /// - 其余是 `GpuError::Driver`（Vulkan 调用失败），错误信息带调用名与返回码名。
    pub fn render(&mut self, list: &DrawList) -> GpuResult<Vec<u8>> {
        self.unsupported.clear();

        // ① 入口检查（与 CPU 同一判据）
        if !list.clip_balanced() {
            return Err(GpuError::Driver {
                code: -1,
                message: "绘制列表的裁剪栈不平衡（PushClip/PopClip 未配对）".to_string(),
            });
        }

        // ② 翻译成顶点流（CPU 侧已裁剪）
        let stream = gpu_geom::build_stream(list, self.extent);
        if !stream.unsupported.is_empty() {
            self.unsupported = stream.unsupported.clone();
            return Err(GpuError::Unsupported(self.unsupported.join("；")));
        }

        // ③ 上传顶点（每帧重传；容量不足时重建缓冲）
        let count = stream.vertices.len();
        let count_u32 = u32::try_from(count).map_err(|_| {
            GpuError::Unsupported(format!("顶点数 {count} 超出 u32（一帧画不了这么多）"))
        })?;
        if count > 0 {
            let bytes = (count * std::mem::size_of::<GpuVertex>()) as u64;
            self.ensure_vertex_capacity(bytes)?;
            self.upload_vertices(&stream.vertices)?;
        }

        // ④⑤ 录制、提交、等栅栏
        self.record_and_submit(count_u32)?;

        // ⑥ 回读
        self.read_back()
    }

    /// 确保顶点缓冲至少有 `bytes` 字节（不够就按 2 的幂重建）。
    fn ensure_vertex_capacity(&mut self, bytes: u64) -> GpuResult<()> {
        if self.vertex.as_ref().is_some_and(|v| v.capacity >= bytes) {
            return Ok(());
        }
        // 先丢掉旧的：`VertexBuffer` 的字段顺序保证「先缓冲、后内存」；
        // 到这里上一帧的提交已经等过栅栏 ⇒ 缓冲不在使用中，可以安全销毁。
        self.vertex = None;
        let capacity = bytes.next_power_of_two().max(MIN_VERTEX_BYTES);
        let (buffer, memory) = create_host_buffer(
            "vkCreateBuffer(vertex)",
            self.device_handle,
            &self.fns,
            &self.mem_props,
            capacity,
            vk::VK_BUFFER_USAGE_VERTEX_BUFFER_BIT,
        )?;
        self.vertex = Some(VertexBuffer {
            buffer,
            memory,
            capacity,
        });
        Ok(())
    }

    /// 把顶点写进顶点缓冲（map → memcpy → unmap；HOST_COHERENT 所以不用 flush）。
    fn upload_vertices(&self, verts: &[GpuVertex]) -> GpuResult<()> {
        let vb = self
            .vertex
            .as_ref()
            .expect("调用方保证了「有顶点」⇒ 缓冲已建");
        let bytes = std::mem::size_of_val(verts);
        // SAFETY: `GpuVertex` 是 `#[repr(C)]` 的纯 `f32` 结构（无指针、无 Drop），
        // 按字节视图读它是定义良好的。
        let src = unsafe { std::slice::from_raw_parts(verts.as_ptr() as *const u8, bytes) };
        let mut mapped: *mut c_void = std::ptr::null_mut();
        // SAFETY: 内存是 HOST_VISIBLE；`mapped` 是可写输出；映射整块。
        check("vkMapMemory(vertex)", unsafe {
            (self.fns.map_memory)(
                self.device_handle,
                vb.memory.handle(),
                0,
                vk::WHOLE_SIZE,
                0,
                &mut mapped,
            )
        })?;
        if mapped.is_null() {
            return Err(GpuError::Driver {
                code: 0,
                message: "vkMapMemory(vertex) 返回空指针".to_string(),
            });
        }
        // SAFETY: 映射了整块缓冲（≥ bytes，由 `ensure_vertex_capacity` 保证）；
        // 源与目标不重叠。
        unsafe {
            std::ptr::copy_nonoverlapping(src.as_ptr(), mapped as *mut u8, bytes);
            (self.fns.unmap_memory)(self.device_handle, vb.memory.handle());
        }
        Ok(())
    }

    /// 录制一帧（清屏 + 绑定管线/顶点缓冲 + 绘制 + 屏障 + 拷贝），提交并等栅栏。
    fn record_and_submit(&self, vertex_count: u32) -> GpuResult<()> {
        // 上一帧已经等过栅栏 ⇒ 命令缓冲不在执行中，可以重置。
        check("vkResetCommandBuffer", unsafe {
            (self.fns.reset_command_buffer)(self.cmd, 0)
        })?;
        let begin = vk::CommandBufferBeginInfo {
            s_type: vk::VK_STRUCTURE_TYPE_COMMAND_BUFFER_BEGIN_INFO,
            p_next: std::ptr::null(),
            flags: vk::VK_COMMAND_BUFFER_USAGE_ONE_TIME_SUBMIT_BIT,
            p_inheritance_info: std::ptr::null(),
        };
        // SAFETY: 句柄有效；`begin` 在栈上存活。
        check("vkBeginCommandBuffer", unsafe {
            (self.fns.begin_command_buffer)(self.cmd, &begin)
        })?;

        let clear_value = vk::ClearValue {
            color: vk::ClearColorValue {
                float32: self.clear,
            },
        };
        let begin_pass = vk::RenderPassBeginInfo {
            s_type: vk::VK_STRUCTURE_TYPE_RENDER_PASS_BEGIN_INFO,
            p_next: std::ptr::null(),
            render_pass: self.pass.handle(),
            framebuffer: self.framebuffer.handle(),
            render_area: vk::Rect2D {
                offset: vk::Offset2D { x: 0, y: 0 },
                extent: vk::Extent2D {
                    width: self.extent.width,
                    height: self.extent.height,
                },
            },
            clear_value_count: 1,
            p_clear_values: &clear_value,
        };
        // SAFETY: 上述结构体都在本栈帧存活；句柄都是本结构持有的有效句柄。
        unsafe {
            (self.fns.cmd_begin_render_pass)(self.cmd, &begin_pass, vk::VK_SUBPASS_CONTENTS_INLINE);
            (self.fns.cmd_bind_pipeline)(
                self.cmd,
                vk::VK_PIPELINE_BIND_POINT_GRAPHICS,
                self.pipeline.handle(),
            );
            // ⚠️ **不调** `vkCmdSetViewport`/`vkCmdSetScissor`：本管线的 viewport/scissor 是
            // **静态**的（写死在管线里，见 `device.rs::create_vertex_pipeline`）。对静态状态
            // 发动态设置命令会触发校验层报错 —— `tests/vbo_probe.rs` 记着这条实测。
            if vertex_count > 0 {
                let vb = self
                    .vertex
                    .as_ref()
                    .expect("非空顶点数 ⇒ 缓冲已建（`render` 里先 ensure 再录）");
                let offset: vk::DeviceSize = 0;
                (self.fns.cmd_bind_vertex_buffers)(self.cmd, 0, 1, &vb.buffer.handle(), &offset);
                (self.fns.cmd_draw)(self.cmd, vertex_count, 1, 0, 0);
            }
            (self.fns.cmd_end_render_pass)(self.cmd);
        }

        // 屏障：`oldLayout` 必须是图像**当前实际**布局 ⇒ 向渲染通道要 `final_layout()`。
        // （曾经在别处写死 `COLOR_ATTACHMENT_OPTIMAL` 而渲染通道给的是
        //   `TRANSFER_SRC_OPTIMAL`，校验层直接报 `cannot transition the layout`。）
        let barrier = vk::ImageMemoryBarrier {
            s_type: vk::VK_STRUCTURE_TYPE_IMAGE_MEMORY_BARRIER,
            p_next: std::ptr::null(),
            src_access_mask: vk::VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT,
            dst_access_mask: vk::VK_ACCESS_TRANSFER_READ_BIT,
            old_layout: self.pass.final_layout(),
            new_layout: vk::VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL,
            src_queue_family_index: vk::QUEUE_FAMILY_IGNORED,
            dst_queue_family_index: vk::QUEUE_FAMILY_IGNORED,
            image: self.image.handle(),
            subresource_range: color_range(),
        };
        let copy = vk::BufferImageCopy {
            buffer_offset: 0,
            buffer_row_length: 0, // 0 = 与图像宽度一致（无 padding）
            buffer_image_height: 0,
            image_subresource: vk::ImageSubresourceLayers {
                aspect_mask: vk::VK_IMAGE_ASPECT_COLOR_BIT,
                mip_level: 0,
                base_array_layer: 0,
                layer_count: 1,
            },
            image_offset: vk::Offset3D { x: 0, y: 0, z: 0 },
            image_extent: vk::Extent3D {
                width: self.extent.width,
                height: self.extent.height,
                depth: 1,
            },
        };
        // SAFETY: 结构体在栈上存活。
        unsafe {
            (self.fns.cmd_pipeline_barrier)(
                self.cmd,
                vk::VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT,
                vk::VK_PIPELINE_STAGE_TRANSFER_BIT,
                0,
                0,
                std::ptr::null(),
                0,
                std::ptr::null(),
                1,
                &barrier,
            );
            (self.fns.cmd_copy_image_to_buffer)(
                self.cmd,
                self.image.handle(),
                vk::VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL,
                self.staging.handle(),
                1,
                &copy,
            );
        }
        // SAFETY: 句柄有效且处于录制状态。
        check("vkEndCommandBuffer", unsafe {
            (self.fns.end_command_buffer)(self.cmd)
        })?;

        // 提交前把栅栏放回未信号态（它上一帧已经信号过了）。
        // SAFETY: 栅栏有效且当前没有等待者（上一帧已等到）。
        check("vkResetFences", unsafe {
            (self.fns.reset_fences)(self.device_handle, 1, &self.fence.handle())
        })?;
        let submit = vk::SubmitInfo {
            s_type: vk::VK_STRUCTURE_TYPE_SUBMIT_INFO,
            p_next: std::ptr::null(),
            wait_semaphore_count: 0,
            p_wait_semaphores: std::ptr::null(),
            p_wait_dst_stage_mask: std::ptr::null(),
            command_buffer_count: 1,
            p_command_buffers: &self.cmd,
            signal_semaphore_count: 0,
            p_signal_semaphores: std::ptr::null(),
        };
        // SAFETY: 队列与命令缓冲都有效；`submit` 在栈上存活；栅栏用于同步。
        check("vkQueueSubmit", unsafe {
            (self.fns.queue_submit)(self.queue, 1, &submit, self.fence.handle())
        })?;
        // 有限超时：驱动出问题时宁可失败，不要永久挂住。
        // SAFETY: 栅栏有效。
        let rc = unsafe {
            (self.fns.wait_for_fences)(
                self.device_handle,
                1,
                &self.fence.handle(),
                vk::VK_TRUE,
                TIMEOUT_NS,
            )
        };
        if rc != ffi::VK_SUCCESS {
            return Err(GpuError::Driver {
                code: rc,
                message: format!(
                    "vkWaitForFences 失败（{} ⇒ 可能是超时，即 GPU 没在预期时间内做完）",
                    vk_result_name(rc)
                ),
            });
        }
        Ok(())
    }

    /// map 暂存缓冲 → 拷出 → unmap。
    fn read_back(&self) -> GpuResult<Vec<u8>> {
        let mut mapped: *mut c_void = std::ptr::null_mut();
        // SAFETY: 内存是 HOST_VISIBLE 且 COHERENT；映射整块。
        check("vkMapMemory(staging)", unsafe {
            (self.fns.map_memory)(
                self.device_handle,
                self.staging_memory.handle(),
                0,
                vk::WHOLE_SIZE,
                0,
                &mut mapped,
            )
        })?;
        if mapped.is_null() {
            return Err(GpuError::Driver {
                code: 0,
                message: "vkMapMemory(staging) 返回空指针".to_string(),
            });
        }
        // SAFETY: 映射了整块缓冲；长度取自缓冲大小。
        let out =
            unsafe { std::slice::from_raw_parts(mapped as *const u8, self.staging_size as usize).to_vec() };
        // SAFETY: 与上面的 map 配对。
        unsafe { (self.fns.unmap_memory)(self.device_handle, self.staging_memory.handle()) };
        Ok(out)
    }
}

/// 颜色附件的子资源范围（只有一个 mip / 一层）。
fn color_range() -> vk::ImageSubresourceRange {
    vk::ImageSubresourceRange {
        aspect_mask: vk::VK_IMAGE_ASPECT_COLOR_BIT,
        base_mip_level: 0,
        level_count: 1,
        base_array_layer: 0,
        layer_count: 1,
    }
}
