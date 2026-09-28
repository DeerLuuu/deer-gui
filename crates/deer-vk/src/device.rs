//! Vulkan **设备**（M2a）：逻辑设备 + 队列 + 着色器模块。
//!
//! ## 线程模型（为什么这么绕）
//!
//! Vulkan 的句柄是裸指针（不是 `Send`），而且**销毁必须在创建它的线程上**。
//! 直接把它放进结构体会让类型不再是 `Send`，于是 `GpuResult<Box<dyn Device>>`
//! 这类要在两线程间传递的签名就编译不过。
//!
//! 本模块的解法：**把实例与设备的所有权留在一个后台线程里**，主线程只持有
//! 句柄数字与函数指针（都是 `Send`）。线程 `park()` 等主线程发停止信号，
//! 收到后再按正确顺序销毁。对调用方完全透明。
//!
//! 注意：这**不是**「渲染线程」——真正的渲染线程设计属于 Q-4（未决）。
//! 这里只是为了让 Vulkan 的生命周期约束与 Rust 的类型系统共存。

use std::sync::mpsc::{Receiver, Sender};
use std::thread::JoinHandle;

use deer_gpu::{GpuError, GpuResult};

use crate::ffi;
use crate::ffi_dev as vk;
use crate::loader::Lib;

/// SPIR-V 魔数（`MagicNumber`，固定值）。
pub const SPIRV_MAGIC: u32 = 0x0723_0203;

// 上传次数计数器（**按线程**统计）。
//
// ⚠️ 这里用 `//` 而不是 `///`：`thread_local!` 是**宏调用**，rustdoc 不为宏调用
// 生成文档（会报 `unused doc comment`）。面向使用者的说明都写在下面
// [`texture_r8_upload_count`] 的文档里。
//
// ——— 为什么需要这个计数器（M3b review M-4）———
//
// 「图集纹理**只在指纹变化时**重传」是本项目一条**性能前提**（`create_texture_r8`
// 内部会 `vkQueueWaitIdle` —— 每帧重传会把 CPU 卡在每一帧的同步上）。但它此前
// **只有实现、没有护栏**：reviewer 实测把 `refresh_atlas_texture()` 的指纹判断
// 删掉、改成每帧重传之后，`consecutive_text_frames_stay_in_sync` **依然全绿**
// —— 因为那个测试只看最终像素，而「重传几次」对像素没有影响。
//
// 「删掉实现、测试照样绿」正是本项目最在意的那类缺口，所以这里给出一个可断言的
// 可观测量：测试用两次读取的差值断言「同一图集连续多帧只上传一次」。
//
// ——— 为什么自增点在**被调用方**（本模块）而不是让调用方上报 ———
//
// 调用点在 `gpu_render.rs::refresh_atlas_texture`，而计数在 `create_texture_r8`
// 内部自增 —— 于是无论调用方怎么改（删指纹、改判断条件、换调用点），只要真的走了
// 上传路径，计数就会动。若做成「渲染器字段 + 调用方上报」，护栏就退化成
// 「实现者在自证」，挡不住「把上报一起删掉」这种变异。
//
// ——— ⚠️ 为什么是 `thread_local!` 而不是进程级 `static`（一次真实的缺陷）———
//
// 第一版写成了进程级 `static AtomicUsize`。它**在默认的并行 `cargo test` 下是
// flaky 的**：别的测试线程并行上传纹理 ⇒ 计数器被「外来上传」污染 ⇒ 本测试断言的
// **帧间差值**变成非零。reviewer 实测：`cargo test -p deer-vk` **1/8 FAILED**
// （重试偶发）、`--test gpu_vs_cpu` 并行 **2/20**、加 `--test-threads=1` 则 **0/15**；
// 而失败信息恒为「触发了 1 次纹理上传」—— **把排查者指向
// `refresh_atlas_texture`，可那里根本没有问题**。
//
// 更糟的是它的必然结局：假红 →「又是那个 flaky」→ **护栏被删掉**。也就是说
// 一个 flaky 的护栏比没有护栏更危险 —— 它会亲手把刚补上的护栏送走。
//
// **结论（本项目的护栏纪律）**：护栏本身必须是**并行安全**的。任何
// 「进程级可变状态 + 窗口差值」的断言在默认多线程测试下都会 flaky。
// 这里改成 `thread_local!`，于是别的测试线程的上传**根本不会**进入本线程的计数。
//
// 对照：`ffi::validation_message_count()` 是进程级 `static` 且**没问题** ——
// 因为它断言的是全局 `== 0`（「整个进程一条消息都没有」），本来就不受并发影响；
// 而本计数器断言的是**窗口差值**，两者不是一回事（我先前把这个类比用错了）。
// 仓库里 `GpuGeometryRenderer::host_to_vertex_barriers` 用的是「每实例字段」，
// 那是另一种正确的免疫写法。
//
// ——— 使用约束（诚实说明）———
//
// 计数**只属于调用它的那个线程**，所以读取方必须在**同一个线程**上渲染后再读
// —— 本项目的渲染是同步内联的（`GpuGeometryRenderer::render` 在调用者线程上执行），
// 测试也是单线程内完成渲染与断言，因此满足这条约束。
// 若将来换成专用渲染线程，这个计数器要改成「每设备」形态（reader 需要拿到
// 设备句柄；`gpu_render.rs` 目前没有暴露 `device()`，所以本轮先用 TLS）。
thread_local! {
    static TEXTURE_R8_UPLOAD_COUNT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// **本线程**累计的 `create_texture_r8` 上传路径次数。
///
/// 测试用法：在两次渲染之间取**差值** ——
/// ```ignore
/// let before = deer_vk::device::texture_r8_upload_count();
/// gpu.render(&list)?;                       // 同一图集（未变化）
/// assert_eq!(deer_vk::device::texture_r8_upload_count() - before, 0,
///            "同一图集不应重传");
/// ```
///
/// ## 语义边界（review N-2）
///
/// 计的是「**进入了上传路径**」：自增点紧跟在 [`validate_texture_r8_args`]
/// **之后**、**第一次驱动调用之前**。于是
/// - 参数被拒的调用**不计**（它没碰驱动 —— 否则负例测试会让计数虚增，
///   护栏就不可断言了）；
/// - 「参数合法、但驱动随后失败」**计**（那确实是一次上传尝试，且失败会让调用方
///   立刻拿到 `Err`，不影响本护栏的差值语义）。
///
/// 之所以**不**把自增移到「整个上传成功之后」：那样就不能放在函数开头，
/// 得在函数末尾再加一处 —— 而 `create_texture_r8` 中间有多个 `?` 提前返回点，
/// 「上传成功之后」这个点会被这些早退绕过，反而更容易漏计。
/// 所以选择明确、单一、且**每一条路径都会经过**的位置。
///
/// ## 并行安全
///
/// 计数是 `thread_local` 的（见上面 `TEXTURE_R8_UPLOAD_COUNT` 的说明）：
/// 别的测试线程并行上传**不会**污染本线程的窗口差值。
/// 因此**必须在做渲染的那个线程上读**（本项目的渲染是同步内联的）。
pub fn texture_r8_upload_count() -> usize {
    TEXTURE_R8_UPLOAD_COUNT.with(|c| c.get())
}

// ——— 通用纹理（`RGBA8_UNORM`）的上传计数 ———
//
// 与 `TEXTURE_R8_UPLOAD_COUNT` **完全同一套**理由与约束（`thread_local` + 在被调用方
// 自增 + 只算「跨过参数校验」的调用，见上面那段长说明）。
//
// 为什么**分成两个**计数器而不是合并成一个「纹理上传数」：
// `texture_r8_upload_count()` 的既有判据断言的是**字形图集的重传语义**
// （同一图集连续多帧只传一次）。如果把通用纹理的上传也计进去，那条差值就会与
// 「这一帧有没有贴新纹理」纠缠在一起 —— 两个语义不同的量共用一个计数器，
// 是「护栏互相污染」的经典来源。
thread_local! {
    static TEXTURE_RGBA8_UPLOAD_COUNT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// **本线程**累计的 `create_texture_rgba8` 上传路径次数。
///
/// 语义边界与并行安全约束同 [`texture_r8_upload_count`]：自增点紧跟在参数校验之后、
/// 第一次驱动调用之前；读方必须与渲染**同线程**（本项目渲染是同步内联的）。
pub fn texture_rgba8_upload_count() -> usize {
    TEXTURE_RGBA8_UPLOAD_COUNT.with(|c| c.get())
}

// ── 间接绘制（M3+ 第 4 项下半）的 ABI 与常量 ──────────────────────────────────
//
// 为什么这些**声明在使用者旁边**（而不是 `ffi_dev.rs`）：`ffi_dev.rs` 不在本任务的
// 允许改动清单里。既有先例是 M3a-T3 的 `HOST`/`VERTEX_INPUT` 同步位、以及 M2b 的
// 交换链结构体 —— 都写在使用者模块里，并注明「值取自 SDK 头文件 + 有测试钉住」。
//
// 值取自本机 Vulkan SDK `1.4.357.0/include/vulkan/vulkan_core.h`。

/// `VK_BUFFER_USAGE_INDEX_BUFFER_BIT`（= `0x0000_0040`）。
pub const VK_BUFFER_USAGE_INDEX_BUFFER_BIT: u32 = 0x0000_0040;
/// `VK_BUFFER_USAGE_INDIRECT_BUFFER_BIT`（= `0x0000_0100`）。
pub const VK_BUFFER_USAGE_INDIRECT_BUFFER_BIT: u32 = 0x0000_0100;
/// `VkIndexType::VK_INDEX_TYPE_UINT32`（= **1**；`UINT16` 才是 0）。
///
/// ⚠️ 写错成 0 不会编译报错：驱动会按 `UINT16` 解释索引缓冲的**一半字节**，
/// 症状是「画出来的顶点错位/乱线」，而不是任何错误码。
pub const VK_INDEX_TYPE_UINT32: i32 = 1;

/// `VkDrawIndexedIndirectCommand`（**手写 ABI**，5 个 `u32` = 20 字节、无 padding）。
///
/// ```c
///   uint32_t indexCount; uint32_t instanceCount; uint32_t firstIndex;
///   int32_t vertexOffset; uint32_t firstInstance;
/// ```
///
/// 它被**逐字节**灌进间接命令缓冲（`VK_BUFFER_USAGE_INDIRECT_BUFFER_BIT`），
/// 驱动按这个布局读 ⇒ 字段顺序/大小错了就是「读成垃圾值」（可能什么都不画，也可能越界）。
/// `size_of`/`offset_of` 断言见 `tests/texture_indirect.rs`。
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DrawIndexedIndirectCommand {
    pub index_count: u32,
    pub instance_count: u32,
    pub first_index: u32,
    pub vertex_offset: i32,
    pub first_instance: u32,
}

impl DrawIndexedIndirectCommand {
    /// 「把 `vertex_count` 个顶点按顺序索引、画一遍」这条命令。
    ///
    /// 本项目形状是**顺序索引**（`0..N`），所以只有 `index_count` 随帧变化 ——
    /// 这也是「命令内容不变就不重传」那条复用判据的依据。
    pub fn for_vertex_count(vertex_count: u32) -> Self {
        DrawIndexedIndirectCommand {
            index_count: vertex_count,
            instance_count: 1,
            first_index: 0,
            vertex_offset: 0,
            first_instance: 0,
        }
    }

    /// 小端字节序列（= 驱动从缓冲里读到的内容）。
    pub fn to_bytes(self) -> [u8; 20] {
        let mut out = [0u8; 20];
        out[0..4].copy_from_slice(&self.index_count.to_le_bytes());
        out[4..8].copy_from_slice(&self.instance_count.to_le_bytes());
        out[8..12].copy_from_slice(&self.first_index.to_le_bytes());
        out[12..16].copy_from_slice(&self.vertex_offset.to_le_bytes());
        out[16..20].copy_from_slice(&self.first_instance.to_le_bytes());
        out
    }

    /// 顺序索引 `0..n` 的 `u32` 字节序列（索引缓冲的内容）。
    pub fn sequential_indices(vertex_count: u32) -> Vec<u8> {
        let mut out = Vec::with_capacity(vertex_count as usize * 4);
        for i in 0..vertex_count {
            out.extend_from_slice(&i.to_le_bytes());
        }
        out
    }
}

/// `vkCmdBindIndexBuffer(cmd, buffer, offset, indexType)`。
pub type PfnCmdBindIndexBuffer =
    unsafe extern "system" fn(vk::CommandBufferHandle, vk::BufferHandle, u64, i32);
/// `vkCmdDrawIndexedIndirect(cmd, buffer, offset, drawCount, stride)`。
pub type PfnCmdDrawIndexedIndirect =
    unsafe extern "system" fn(vk::CommandBufferHandle, vk::BufferHandle, u64, u32, u32);

/// 设备级函数表（在后台线程里解析；全是函数指针，故 `Send`）。
#[derive(Clone, Copy)]
pub struct DeviceFns {
    pub create_device: vk::PfnCreateDevice,
    pub destroy_device: vk::PfnDestroyDevice,
    pub get_device_queue: vk::PfnGetDeviceQueue,
    pub get_queue_family_properties: vk::PfnGetPhysicalDeviceQueueFamilyProperties,
    pub get_memory_properties: vk::PfnGetPhysicalDeviceMemoryProperties,
    pub create_shader_module: vk::PfnCreateShaderModule,
    pub destroy_shader_module: vk::PfnDestroyShaderModule,
    pub create_render_pass: vk::PfnCreateRenderPass,
    pub destroy_render_pass: vk::PfnDestroyRenderPass,
    pub create_pipeline_layout: vk::PfnCreatePipelineLayout,
    pub destroy_pipeline_layout: vk::PfnDestroyPipelineLayout,
    pub create_graphics_pipelines: vk::PfnCreateGraphicsPipelines,
    pub destroy_pipeline: vk::PfnDestroyPipeline,
    pub create_image: vk::PfnCreateImage,
    pub destroy_image: vk::PfnDestroyImage,
    pub create_image_view: vk::PfnCreateImageView,
    pub destroy_image_view: vk::PfnDestroyImageView,
    pub get_image_memory_requirements: vk::PfnGetImageMemoryRequirements,
    pub get_buffer_memory_requirements: vk::PfnGetBufferMemoryRequirements,
    pub allocate_memory: vk::PfnAllocateMemory,
    pub free_memory: vk::PfnFreeMemory,
    pub bind_image_memory: vk::PfnBindImageMemory,
    pub bind_buffer_memory: vk::PfnBindBufferMemory,
    pub create_buffer: vk::PfnCreateBuffer,
    pub destroy_buffer: vk::PfnDestroyBuffer,
    pub map_memory: vk::PfnMapMemory,
    pub unmap_memory: vk::PfnUnmapMemory,
    pub create_command_pool: vk::PfnCreateCommandPool,
    pub destroy_command_pool: vk::PfnDestroyCommandPool,
    pub allocate_command_buffers: vk::PfnAllocateCommandBuffers,
    pub begin_command_buffer: vk::PfnBeginCommandBuffer,
    pub end_command_buffer: vk::PfnEndCommandBuffer,
    pub reset_command_buffer: vk::PfnResetCommandBuffer,
    pub create_framebuffer: vk::PfnCreateFramebuffer,
    pub destroy_framebuffer: vk::PfnDestroyFramebuffer,
    pub create_fence: vk::PfnCreateFence,
    pub destroy_fence: vk::PfnDestroyFence,
    pub wait_for_fences: vk::PfnWaitForFences,
    pub reset_fences: vk::PfnResetFences,
    pub queue_submit: vk::PfnQueueSubmit,
    pub queue_wait_idle: vk::PfnQueueWaitIdle,
    pub device_wait_idle: vk::PfnDeviceWaitIdle,
    pub cmd_begin_render_pass: vk::PfnCmdBeginRenderPass,
    pub cmd_end_render_pass: vk::PfnCmdEndRenderPass,
    pub cmd_bind_pipeline: vk::PfnCmdBindPipeline,
    /// `vkCmdBindVertexBuffers`（顶点缓冲路径用）
    pub cmd_bind_vertex_buffers: vk::PfnCmdBindVertexBuffers,
    pub cmd_set_viewport: vk::PfnCmdSetViewport,
    pub cmd_set_scissor: vk::PfnCmdSetScissor,
    pub cmd_draw: vk::PfnCmdDraw,
    /// `vkCmdBindIndexBuffer`（索引绘制路径用；M3+ 第 4 项下半）
    pub cmd_bind_index_buffer: PfnCmdBindIndexBuffer,
    /// `vkCmdDrawIndexedIndirect`（**间接绘制**；M3+ 第 4 项下半）
    pub cmd_draw_indexed_indirect: PfnCmdDrawIndexedIndirect,
    pub cmd_push_constants: vk::PfnCmdPushConstants,
    pub cmd_pipeline_barrier: vk::PfnCmdPipelineBarrier,
    pub cmd_copy_image_to_buffer: vk::PfnCmdCopyImageToBuffer,
    pub cmd_clear_color_image: vk::PfnCmdClearColorImage,
    // ── M3b：纹理采样（采样器 + 描述符集 + 缓冲→图像拷贝） ──────────────────
    pub create_sampler: vk::PfnCreateSampler,
    pub destroy_sampler: vk::PfnDestroySampler,
    pub create_descriptor_set_layout: vk::PfnCreateDescriptorSetLayout,
    pub destroy_descriptor_set_layout: vk::PfnDestroyDescriptorSetLayout,
    pub create_descriptor_pool: vk::PfnCreateDescriptorPool,
    pub destroy_descriptor_pool: vk::PfnDestroyDescriptorPool,
    pub allocate_descriptor_sets: vk::PfnAllocateDescriptorSets,
    pub free_descriptor_sets: vk::PfnFreeDescriptorSets,
    pub update_descriptor_sets: vk::PfnUpdateDescriptorSets,
    pub cmd_bind_descriptor_sets: vk::PfnCmdBindDescriptorSets,
    pub cmd_copy_buffer_to_image: vk::PfnCmdCopyBufferToImage,
}

/// 已打开的逻辑设备。
///
/// `handle` 与 `fns` 都是 `Send` 的普通数据；真正的 Vulkan 对象在那个后台线程里。
pub struct VkDevice {
    handle: vk::DeviceHandle,
    queue: vk::QueueHandle,
    queue_family_index: u32,
    /// 呈现队列（M2b）。单队列族实现里它与 `queue` 是同一个句柄 ——
    /// 但仍然单独存一份：将来图形/呈现分离时，只有这里会变。
    present_queue: vk::QueueHandle,
    present_queue_family_index: u32,
    /// 物理设备句柄（交换链要查 surface 能力，那些查询都在物理设备上）。
    /// 生命周期：与实例同寿（Own 时实例在后台线程里，Borrowed 时归调用方）。
    physical_device: ffi::PhysicalDeviceHandle,
    fns: DeviceFns,
    adapter: deer_gpu::AdapterInfo,
    memory_type_count: u32,
    /// **校验层是否真的启用**（不是「是否请求」）。
    ///
    /// 存在的理由（T3 review F7）：校验层是 GPU 正确性的主要证据来源，但
    /// 「跑起来没看到消息」**不等于**「校验层在跑」—— 层没装好、或有人把
    /// `create_with_extensions` 的 `Err` 改成静默降级，都会让「零消息」变成空话。
    /// 让测试能**断言**这个事实，比让评审人肉眼看输出可靠。
    validation_enabled: bool,
    /// 物理设备的内存属性（挑内存类型时必需）
    mem_props: vk::PhysicalDeviceMemoryProperties,
    /// 停止信号：`Drop` 时 `send` 让后台线程销毁实例与设备
    stop: Option<Sender<()>>,
    thread: Option<JoinHandle<()>>,
}

// SAFETY: `VkDevice` 自身不含 Vulkan 对象（真正的对象在后台线程里，且那个线程
// 直到收到停止信号前不会碰它们）。句柄与函数指针都是可安全跨线程传递的普通值。
// 并发使用由调用方的 `&mut self` 与 Vulkan 自身的线程规则约束。
unsafe impl Send for VkDevice {}
unsafe impl Sync for VkDevice {}

impl VkDevice {
    /// 打开一个逻辑设备（选第一个含图形队列的队列族）。
    pub fn open(adapter_index: usize) -> GpuResult<VkDevice> {
        VkDevice::open_inner(adapter_index, InstancePlan::Own)
    }

    /// 打开设备，并保证拿到一个**同时支持图形与呈现**的队列族（拿不到就明确报错）。
    ///
    /// ## 实例从哪来（这条最容易搞错）
    ///
    /// `VkSurfaceKHR` **属于创建它的那个实例**：`VkSurfaceKHR` 与 `VkPhysicalDevice`
    /// 必须来自同一个 `VkInstance`（`vkGetPhysicalDeviceSurfaceSupportKHR` 的 VU）。
    /// 所以本函数**借用** `surface` 的实例，而不是另建一个：
    ///
    /// ```text
    ///   WindowedRenderer::new:
    ///     instance = Instance::create_with_extensions(..surface 扩展..)   // 归它所有
    ///     surface  = Surface::create(&instance, window)                    // 记下实例句柄
    ///     device   = VkDevice::open_with_present(adapter, &surface)        // 借用上面那个实例
    /// ```
    ///
    /// ## 生命周期契约（调用方必须保证）
    ///
    /// `surface` 背后的实例必须比返回的 `VkDevice` **活得久**。
    /// [`crate::windowed::WindowedRenderer`] 用字段顺序保证（device 比 surface/instance
    /// 先析构）。实例只在**创建设备这一次**被访问（后台线程随后只是 park）。
    pub fn open_with_present(adapter_index: usize, surface: &crate::surface::Surface) -> GpuResult<VkDevice> {
        let target = PresentTarget {
            // 句柄存成 usize：函数指针与整数都是 Send，于是「借用」不需要把
            // 非 Send 的 `Instance` 搬进后台线程。
            instance: surface.instance_handle() as usize,
            surface: surface.handle() as usize,
            core: surface.core_fns(),
            support: surface.support_fn(),
        };
        VkDevice::open_inner(adapter_index, InstancePlan::Borrowed(target))
    }

    /// `open` / `open_with_present` 的公共部分：起一个「生命周期线程」持有 Vulkan 对象。
    fn open_inner(adapter_index: usize, plan: InstancePlan) -> GpuResult<VkDevice> {
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<ReadyInfo, GpuError>>();
        let (stop_tx, stop_rx) = std::sync::mpsc::channel::<()>();

        let thread = std::thread::Builder::new()
            .name("deer-vk-lifetime".to_string())
            .spawn(move || {
                lifetime_thread(adapter_index, plan, ready_tx, stop_rx);
            })
            .map_err(|e| GpuError::Driver {
                code: -1,
                message: format!("无法创建 Vulkan 生命周期线程：{e}"),
            })?;

        match ready_rx.recv() {
            Ok(Ok(info)) => Ok(VkDevice {
                handle: info.handle,
                queue: info.queue,
                queue_family_index: info.queue_family_index,
                present_queue: info.present_queue,
                present_queue_family_index: info.present_queue_family_index,
                physical_device: info.physical_device,
                fns: info.fns,
                adapter: info.adapter,
                memory_type_count: info.memory_type_count,
                validation_enabled: info.validation_enabled,
                mem_props: info.mem_props,
                stop: Some(stop_tx),
                thread: Some(thread),
            }),
            Ok(Err(e)) => {
                // 线程已发回错误；它自己会退出，这里只需把 stop 丢掉让它走完
                drop(stop_tx);
                let _ = thread.join();
                Err(e)
            }
            Err(_) => {
                let _ = thread.join();
                Err(GpuError::Driver {
                    code: -1,
                    message: "Vulkan 生命周期线程在就绪前退出".to_string(),
                })
            }
        }
    }

    pub fn handle(&self) -> vk::DeviceHandle {
        self.handle
    }

    pub fn queue(&self) -> vk::QueueHandle {
        self.queue
    }

    pub fn queue_family_index(&self) -> u32 {
        self.queue_family_index
    }

    /// 呈现队列（`open_with_present` 下由「图形 + 呈现」共用的那个队列族提供）。
    pub fn present_queue(&self) -> vk::QueueHandle {
        self.present_queue
    }

    /// 呈现队列的队列族索引。
    pub fn present_queue_family_index(&self) -> u32 {
        self.present_queue_family_index
    }

    /// 物理设备句柄（`pub(crate)`：交换链要查 surface 能力）。
    pub(crate) fn physical_device(&self) -> ffi::PhysicalDeviceHandle {
        self.physical_device
    }

    pub fn fns(&self) -> &DeviceFns {
        &self.fns
    }

    pub fn adapter(&self) -> &deer_gpu::AdapterInfo {
        &self.adapter
    }

    /// 创建一个着色器模块。**这是 SPIR-V 可被驱动解析的验收** ——
    /// 但注意：`vkCreateShaderModule` **很宽容**（实测连 `bound = 0` 都接受），
    /// 所以本函数先做**结构护栏**，真正的「管线可创建性」在管线测试里验收。
    pub fn create_shader_module(&self, code: &[u8]) -> GpuResult<ShaderModule> {
        if code.is_empty() {
            return Err(GpuError::Unsupported("着色器字节为空".to_string()));
        }
        if code.len() % 4 != 0 {
            return Err(GpuError::Unsupported(format!(
                "SPIR-V 必须 4 字节对齐，实际 {} 字节",
                code.len()
            )));
        }
        if code.len() < 20 {
            return Err(GpuError::Unsupported(format!(
                "SPIR-V 至少要有 20 字节头部，实际 {} 字节",
                code.len()
            )));
        }
        // 头部第 0 个字：魔数；第 3 个字：bound（必须 > 0）
        let magic = u32::from_le_bytes([code[0], code[1], code[2], code[3]]);
        if magic != SPIRV_MAGIC {
            return Err(GpuError::Unsupported(format!(
                "SPIR-V 魔数错误：期望 {SPIRV_MAGIC:#010x}，实际 {magic:#010x}"
            )));
        }
        let bound = u32::from_le_bytes([code[12], code[13], code[14], code[15]]);
        if bound == 0 {
            return Err(GpuError::Unsupported(
                "SPIR-V 的 bound 不能为 0（必须大于所有用到的 Id）".to_string(),
            ));
        }
        let info = vk::ShaderModuleCreateInfo {
            s_type: vk::VK_STRUCTURE_TYPE_SHADER_MODULE_CREATE_INFO,
            p_next: std::ptr::null(),
            flags: 0,
            code_size: code.len(),
            p_code: code.as_ptr() as *const u32,
        };
        let mut handle: vk::ShaderModuleHandle = std::ptr::null_mut();
        // SAFETY: `info` 指向本栈帧存活的结构体，`code` 生命周期覆盖调用；
        // `handle` 是可写输出。函数指针来自成功解析的 loader。
        let rc = unsafe {
            (self.fns.create_shader_module)(self.handle, &info, std::ptr::null(), &mut handle)
        };
        if rc != ffi::VK_SUCCESS {
            return Err(GpuError::Driver {
                code: rc,
                message: format!(
                    "vkCreateShaderModule 失败（{}）⇒ 生成的 SPIR-V 被驱动拒绝",
                    vk_result_name(rc)
                ),
            });
        }
        Ok(ShaderModule {
            handle,
            device: self.handle,
            destroy: self.fns.destroy_shader_module,
        })
    }

    /// 等待设备空闲。
    pub fn wait_idle(&self) -> GpuResult<()> {
        // SAFETY: `handle` 是本结构持有的有效设备句柄。
        let rc = unsafe { (self.fns.device_wait_idle)(self.handle) };
        if rc != ffi::VK_SUCCESS {
            return Err(GpuError::Driver {
                code: rc,
                message: format!("vkDeviceWaitIdle 失败：{}", vk_result_name(rc)),
            });
        }
        Ok(())
    }

    /// 探测可用内存类型数（诊断/测试用）。
    pub fn memory_type_count(&self) -> u32 {
        self.memory_type_count
    }

    /// **校验层是否真的启用**（不是「是否请求」）。
    ///
    /// ## 为什么需要它（T3 review F7）
    ///
    /// 「`DEER_VK_VALIDATION=1` 跑起来没看到校验消息」是一份很弱的证据：如果层没装好、
    /// 或者将来有人把 [`ffi::Instance::create_with_extensions`] 里「层缺失就报错」的逻辑
    /// 改成静默降级，**测试会照样全绿**，而校验层其实什么都没查。把这个事实暴露出来，
    /// 测试就能硬断言「请求了就必须真的启用」。
    ///
    /// ## `Own` 与 `Borrowed` 的差别（诚实说明）
    ///
    /// - `VkDevice::open`（`Own`）：实例由本设备创建并持有 ⇒ 返回**实例的事实**
    ///   （[`ffi::Instance::validation_enabled`]）；
    /// - `VkDevice::open_with_present`（`Borrowed`）：实例归调用方（[`crate::surface::Surface`]
    ///   只存句柄，查不到层的状态）⇒ 只能返回**请求值** `validation_from_env()`。
    ///   窗口路径（`windowed.rs`）正是用这个判据建实例的，所以两者一致。
    pub fn validation_enabled(&self) -> bool {
        self.validation_enabled
    }

    /// **物理设备**的内存属性。挑内存类型时必需（逻辑设备上拿不到）。
    pub fn memory_properties(&self) -> &vk::PhysicalDeviceMemoryProperties {
        &self.mem_props
    }

    /// 创建一个**渲染通道**：一个颜色附件、一个子通道。
    ///
    /// 附件的 `initialLayout` 取 `UNDEFINED`、`finalLayout` 取 `TRANSFER_SRC_OPTIMAL` ——
    /// 这样一帧画完就能直接回读（离屏渲染的常规做法）。
    /// `load_op` 可配：想每帧清屏就传 `CLEAR`，想保留上一帧内容就传 `LOAD`。
    pub fn create_render_pass(
        &self,
        format: i32,
        load_op: i32,
        final_layout: i32,
    ) -> GpuResult<RenderPass> {
        let attachment = vk::AttachmentDescription {
            flags: 0,
            format,
            samples: vk::VK_SAMPLE_COUNT_1_BIT,
            load_op,
            store_op: vk::VK_ATTACHMENT_STORE_OP_STORE,
            stencil_load_op: vk::VK_ATTACHMENT_LOAD_OP_DONT_CARE,
            stencil_store_op: vk::VK_ATTACHMENT_STORE_OP_DONT_CARE,
            initial_layout: vk::VK_IMAGE_LAYOUT_UNDEFINED_ATTACHMENT,
            final_layout,
        };
        let color_ref = vk::AttachmentReference {
            attachment: 0,
            layout: vk::VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
        };
        let subpass = vk::SubpassDescription {
            flags: 0,
            pipeline_bind_point: vk::VK_PIPELINE_BIND_POINT_GRAPHICS,
            input_attachment_count: 0,
            p_input_attachments: std::ptr::null(),
            color_attachment_count: 1,
            p_color_attachments: &color_ref,
            p_resolve_attachments: std::ptr::null(),
            p_depth_stencil_attachment: std::ptr::null(),
            preserve_attachment_count: 0,
            p_preserve_attachments: std::ptr::null(),
        };
        // 依赖：外部 → 子通道（等着色器写入完成），子通道 → 外部（保证回读前写完）
        let deps = [
            vk::SubpassDependency {
                src_subpass: u32::MAX, // VK_SUBPASS_EXTERNAL
                dst_subpass: 0,
                src_stage_mask: vk::VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT,
                dst_stage_mask: vk::VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT,
                src_access_mask: 0,
                dst_access_mask: vk::VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT,
                dependency_flags: 0,
            },
            vk::SubpassDependency {
                src_subpass: 0,
                dst_subpass: u32::MAX,
                src_stage_mask: vk::VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT,
                dst_stage_mask: vk::VK_PIPELINE_STAGE_TRANSFER_BIT,
                src_access_mask: vk::VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT,
                dst_access_mask: vk::VK_ACCESS_TRANSFER_READ_BIT,
                dependency_flags: 0,
            },
        ];
        let info = vk::RenderPassCreateInfo {
            s_type: vk::VK_STRUCTURE_TYPE_RENDER_PASS_CREATE_INFO,
            p_next: std::ptr::null(),
            flags: 0,
            attachment_count: 1,
            p_attachments: &attachment,
            subpass_count: 1,
            p_subpasses: &subpass,
            dependency_count: deps.len() as u32,
            p_dependencies: deps.as_ptr(),
        };
        let mut handle: vk::RenderPassHandle = std::ptr::null_mut();
        // SAFETY: 上述结构体都在本栈帧存活；句柄是可写输出。
        let rc = unsafe {
            (self.fns.create_render_pass)(self.handle, &info, std::ptr::null(), &mut handle)
        };
        if rc != ffi::VK_SUCCESS {
            return Err(GpuError::Driver {
                code: rc,
                message: format!("vkCreateRenderPass 失败：{}", vk_result_name(rc)),
            });
        }
        Ok(RenderPass {
            handle,
            device: self.handle,
            destroy: self.fns.destroy_render_pass,
            final_layout,
        })
    }

    /// 创建管线布局。
    ///
    /// 推送常量的 `size` 必须是 4 的倍数。**注意范围大小与实际 push 的大小要一致** ——
    /// 不匹配是那种「能建成功但运行时行为诡异」的错误。
    ///
    /// 这是**薄包装**：等价于 [`VkDevice::create_pipeline_layout_ex`] 传
    /// `descriptor_set_layout = None`。保留它是因为 M3a 的一批调用点（`windowed.rs` /
    /// 示例 / 既有测试）都不需要描述符集，不该被文本管线的需求波及。
    pub fn create_pipeline_layout(
        &self,
        push_constant: Option<(u32, u32, u32)>,
    ) -> GpuResult<PipelineLayout> {
        self.create_pipeline_layout_ex(push_constant, None)
    }

    /// 创建管线布局（**可选带一个 `set 0` 的描述符集布局**）。
    ///
    /// ## 为什么需要它（M3b-T4）
    ///
    /// 文本的片元着色器里有一个 `UniformConstant` 的 `OpTypeSampledImage`，装饰为
    /// `descriptor set 0 / binding 0`（见 `spirv::fragment_shader_text`）。SPIR-V 一旦声明了
    /// 描述符，管线布局里就**必须**给出对应的 `VkDescriptorSetLayout` —— 否则
    /// `vkCreateGraphicsPipelines` 会拒绝（「着色器用了 set 0 但布局里没有」）。
    ///
    /// ## 生命周期契约
    ///
    /// `descriptor_set_layout` 必须在管线布局的整个使用期内保持有效 —— 换句话说
    /// **管线布局不得比描述符集布局活得久**。调用方按字段顺序保证（描述符集布局声明在前）。
    pub fn create_pipeline_layout_ex(
        &self,
        push_constant: Option<(u32, u32, u32)>,
        descriptor_set_layout: Option<&DescriptorSetLayout>,
    ) -> GpuResult<PipelineLayout> {
        let range = push_constant.map(|(stage_flags, offset, size)| vk::PushConstantRange {
            stage_flags,
            offset,
            size,
        });
        if let Some(r) = &range {
            if r.size % 4 != 0 {
                return Err(GpuError::Unsupported(format!(
                    "推送常量大小必须是 4 的倍数，实际 {}",
                    r.size
                )));
            }
        }
        let set_layouts: Vec<vk::DescriptorSetLayoutHandle> = descriptor_set_layout
            .map(|l| vec![l.handle()])
            .unwrap_or_default();
        let info = vk::PipelineLayoutCreateInfo {
            s_type: vk::VK_STRUCTURE_TYPE_PIPELINE_LAYOUT_CREATE_INFO,
            p_next: std::ptr::null(),
            flags: 0,
            set_layout_count: set_layouts.len() as u32,
            p_set_layouts: if set_layouts.is_empty() {
                std::ptr::null()
            } else {
                // `p_set_layouts` 在本项目的 ffi_dev 里声明为 `*const c_void`
                // （Vulkan 头里是 `const VkDescriptorSetLayout*`）⇒ 显式转换。
                set_layouts.as_ptr() as *const std::ffi::c_void
            },
            push_constant_range_count: if range.is_some() { 1 } else { 0 },
            p_push_constant_ranges: range.as_ref().map_or(std::ptr::null(), |r| r),
        };
        let mut handle: vk::PipelineLayoutHandle = std::ptr::null_mut();
        // SAFETY: 结构体在栈上存活；句柄是可写输出。
        let rc = unsafe {
            (self.fns.create_pipeline_layout)(self.handle, &info, std::ptr::null(), &mut handle)
        };
        if rc != ffi::VK_SUCCESS {
            return Err(GpuError::Driver {
                code: rc,
                message: format!("vkCreatePipelineLayout 失败：{}", vk_result_name(rc)),
            });
        }
        Ok(PipelineLayout {
            handle,
            device: self.handle,
            destroy: self.fns.destroy_pipeline_layout,
        })
    }

    /// 创建一个**用真实顶点输入**的图形管线（M3a 的顶点缓冲路径）。
    ///
    /// 与 [`VkDevice::create_graphics_pipeline`] 的差别只有两点：
    /// 1. **声明顶点缓冲与属性**（那个版本的 `vertex_binding_description_count = 0`）；
    /// 2. **静态 viewport/scissor**（尺寸 = `extent`）—— 实测动态版在本机 Intel 驱动上
    ///    画不出任何像素（见 [`VkDevice::create_graphics_pipeline_static_viewport`]）。
    ///
    /// ## 调用方的两条硬约束
    ///
    /// - **不要再调 `vkCmdSetViewport` / `vkCmdSetScissor`**：本管线没有声明这两个动态状态，
    ///   对静态状态发动态设置命令会触发校验层报错（`vbo_probe.rs` 里踩过）。
    /// - `attrs[].offset` 必须与 `stride` 描述的那个顶点结构**逐字节**一致。
    ///   M3a 的顶点是 [`crate::gpu_geom::GpuVertex`]（`#[repr(C)]`，stride 44）——
    ///   `gpu_render.rs` 用 `offset_of!` 取偏移，所以这里不靠手抄数字。
    ///
    /// `VertexAttr` 比 `vk::VertexInputAttributeDescription` 少一个字段：`binding` 恒为 0
    /// （只有一个顶点缓冲）。少一个「忘了写 binding 于是读到别的缓冲」的机会。
    ///
    /// ## 参数校验（5 条错误路径）
    ///
    /// 校验逻辑抽在纯函数 [`validate_vertex_pipeline_args`] 里（**不碰 Vulkan**），
    /// 所以 5 条错误路径有无 GPU 都能回归（`device.rs` 末尾的单元测试 +
    /// `tests/pipeline_smoke.rs` 走真实 `VkDevice` 的那条各覆盖一遍）。
    pub fn create_vertex_pipeline(
        &self,
        stages: &[vk::PipelineShaderStageCreateInfo],
        layout: &PipelineLayout,
        render_pass: &RenderPass,
        extent: vk::Extent2D,
        stride: u32,
        attrs: &[VertexAttr],
    ) -> GpuResult<Pipeline> {
        validate_vertex_pipeline_args(extent, stride, attrs)?;
        // `binding` 恒为 0；`input_rate` = 每顶点。
        let binding = vk::VertexInputBindingDescription {
            binding: 0,
            stride,
            input_rate: vk::VK_VERTEX_INPUT_RATE_VERTEX,
        };
        let attr_descs: Vec<vk::VertexInputAttributeDescription> = attrs
            .iter()
            .map(|a| vk::VertexInputAttributeDescription {
                location: a.location,
                binding: 0,
                format: a.format,
                offset: a.offset,
            })
            .collect();
        let vertex_input = vk::PipelineVertexInputStateCreateInfo {
            s_type: vk::VK_STRUCTURE_TYPE_PIPELINE_VERTEX_INPUT_STATE_CREATE_INFO,
            p_next: std::ptr::null(),
            flags: 0,
            vertex_binding_description_count: 1,
            p_vertex_binding_descriptions: &binding,
            vertex_attribute_description_count: attr_descs.len() as u32,
            p_vertex_attribute_descriptions: attr_descs.as_ptr(),
        };
        let viewport = vk::Viewport {
            x: 0.0,
            y: 0.0,
            width: extent.width as f32,
            height: extent.height as f32,
            min_depth: 0.0,
            max_depth: 1.0,
        };
        let scissor = vk::Rect2D {
            offset: vk::Offset2D { x: 0, y: 0 },
            extent,
        };
        self.build_pipeline(
            stages,
            layout,
            render_pass,
            Some((&viewport, &scissor)),
            &vertex_input,
            true,
        )
    }

    /// 创建一个 `R8_UNORM` 覆盖率纹理（上传 + 转为 `SHADER_READ_ONLY_OPTIMAL`）。
    ///
    /// ## 参数校验
    ///
    /// `w == 0 || h == 0`、`data.len() != w * h` 都由纯函数
    /// [`validate_texture_r8_args`] 在**调用驱动前**拦下（理由见那里的文档）。
    ///
    /// ## 上传路径（为什么绕了一圈 staging buffer）
    ///
    /// ```text
    ///   主机数据 → (map/memcpy) HOST_VISIBLE staging buffer
    ///            → vkCmdCopyBufferToImage → DEVICE_LOCAL 图像
    ///            → 屏障转 SHADER_READ_ONLY_OPTIMAL
    /// ```
    ///
    /// 为什么不直接把图像做成 `HOST_VISIBLE + LINEAR` 映射写入（那样少一次拷贝）：
    /// - LINEAR 图像的**行距由驱动决定**（要另查 `vkGetImageSubresourceLayout`），
    ///   对 `R8_UNORM` 也可能带 padding ⇒ 写入得逐行处理，多一处易错点；
    /// - 多数桌面驱动对 `LINEAR` + `SAMPLED` 组合支持有限。
    ///
    /// 标准路径的代价只是一次性的一次拷贝 —— 字形图集只在**内容变化时**重传
    /// （M3b T4 的契约），不是每帧，所以可忽略。
    pub fn create_texture_r8(&self, w: u32, h: u32, data: &[u8]) -> GpuResult<Texture> {
        self.create_texture(w, h, TextureFormat::R8Unorm, data)
    }

    /// 创建一个 **`R8G8B8A8_UNORM` 通用彩色纹理**（M3+ 第 4 项下半）。
    ///
    /// 与 [`VkDevice::create_texture_r8`] 走**同一条**上传路径，只是格式与每像素字节数
    /// 不同（`data` 必须恰好 `w*h*4` 字节，顺序 R,G,B,A）。
    ///
    /// ## 采样侧的诚实边界
    ///
    /// 当前统一片元着色器只读纹理的 **R 通道**（覆盖率语义）⇒ 这张纹理的 RGB 不会
    /// 直接显示出来（颜色由顶点颜色给、由 R 调制）。要真正按 RGB 调制需要新的片元
    /// 着色器（`spirv.rs`，不在本任务 scope）。**但上传本身四通道全保真** ——
    /// 由 [`VkDevice::read_texture_bytes`] 逐字节证明。
    pub fn create_texture_rgba8(&self, w: u32, h: u32, data: &[u8]) -> GpuResult<Texture> {
        self.create_texture(w, h, TextureFormat::Rgba8Unorm, data)
    }

    /// 通用纹理创建入口（`format` 决定 `VkFormat` 与主机数据的每像素字节数）。
    ///
    /// 上传路径：`主机 → HOST_VISIBLE staging → vkCmdCopyBufferToImage → SHADER_READ_ONLY`，
    /// 一次性提交并等队列空闲 ⇒ 返回时纹理**已经可以采样**。
    ///
    /// 计数在**被调用方**（本函数）按格式分别自增：跨过参数校验之后、第一次驱动调用之前
    /// —— 于是「参数被拒」不计（没碰驱动），而「改了调用方却仍走上传路径」照样会动。
    pub fn create_texture(
        &self,
        w: u32,
        h: u32,
        format: TextureFormat,
        data: &[u8],
    ) -> GpuResult<Texture> {
        validate_texture_args(w, h, format, data)?;
        // 计数点紧跟在**校验之后**、第一次驱动调用**之前**：
        // - 参数被拒的调用**不计**（它没碰驱动，否则负例测试会让计数虚增）；
        // - 「参数合法但驱动随后失败」**计**（那确实是一次上传尝试）。
        // 用 `thread_local` ⇒ 别的测试线程的并行上传污染不到本线程的差值断言。
        match format {
            TextureFormat::R8Unorm => TEXTURE_R8_UPLOAD_COUNT.with(|c| c.set(c.get() + 1)),
            TextureFormat::Rgba8Unorm => {
                TEXTURE_RGBA8_UPLOAD_COUNT.with(|c| c.set(c.get() + 1))
            }
        }
        let fmt = format.vulkan_format();
        let fns = self.fns;
        let device = self.handle;

        // ① 图像：DEVICE_LOCAL + OPTIMAL，用法 = 采样 + 传输目标 + **传输源**
        //
        // `TRANSFER_SRC` 是 [`VkDevice::read_texture_bytes`] 需要的（上传保真判据要读回来）。
        // 多一个 usage 位对驱动没有额外代价，也不影响采样语义。
        let img_info = vk::ImageCreateInfo {
            s_type: vk::VK_STRUCTURE_TYPE_IMAGE_CREATE_INFO,
            p_next: std::ptr::null(),
            flags: 0,
            image_type: vk::VK_IMAGE_TYPE_2D,
            format: fmt,
            extent: vk::Extent3D {
                width: w,
                height: h,
                depth: 1,
            },
            mip_levels: 1,
            array_layers: 1,
            samples: vk::VK_SAMPLE_COUNT_1_BIT,
            tiling: vk::VK_IMAGE_TILING_OPTIMAL,
            usage: vk::VK_IMAGE_USAGE_SAMPLED_BIT
                | vk::VK_IMAGE_USAGE_TRANSFER_DST_BIT
                | vk::VK_IMAGE_USAGE_TRANSFER_SRC_BIT,
            sharing_mode: vk::VK_SHARING_MODE_EXCLUSIVE,
            queue_family_index_count: 0,
            p_queue_family_indices: std::ptr::null(),
            initial_layout: vk::VK_IMAGE_LAYOUT_UNDEFINED,
        };
        let mut image_handle: vk::ImageHandle = std::ptr::null_mut();
        // SAFETY: 结构体在栈上存活到调用结束；输出句柄可写。
        let rc = unsafe { (fns.create_image)(device, &img_info, std::ptr::null(), &mut image_handle) };
        check_vk("vkCreateImage", rc)?;
        let image = OwnedHandle::destroy(image_handle, device, fns.destroy_image);

        // ② 绑定 DEVICE_LOCAL 内存
        let mut req = std::mem::MaybeUninit::<vk::MemoryRequirements>::uninit();
        // SAFETY: 该函数完整写入结构体。
        unsafe { (fns.get_image_memory_requirements)(device, image.handle(), req.as_mut_ptr()) };
        let req = unsafe { req.assume_init() };
        let local = vk::VK_MEMORY_PROPERTY_DEVICE_LOCAL_BIT;
        let mem_index = pick_memory_type(self.mem_props, req.memory_type_bits, local)?;
        let image_memory = alloc_memory(device, &fns, req.size, mem_index, local)?;
        check_vk(
            "vkBindImageMemory",
            // SAFETY: 图像与内存都是本设备的新对象，尺寸匹配；offset 0 合法。
            unsafe { (fns.bind_image_memory)(device, image.handle(), image_memory.handle(), 0) },
        )?;

        // ③ 视图（与图像同格式、2D、单层单 mip）
        let view_info = vk::ImageViewCreateInfo {
            s_type: vk::VK_STRUCTURE_TYPE_IMAGE_VIEW_CREATE_INFO,
            p_next: std::ptr::null(),
            flags: 0,
            image: image.handle(),
            view_type: vk::VK_IMAGE_VIEW_TYPE_2D,
            format: fmt,
            components_r: vk::VK_COMPONENT_SWIZZLE_IDENTITY,
            components_g: vk::VK_COMPONENT_SWIZZLE_IDENTITY,
            components_b: vk::VK_COMPONENT_SWIZZLE_IDENTITY,
            components_a: vk::VK_COMPONENT_SWIZZLE_IDENTITY,
            subresource_range: full_subresource_range(),
        };
        let mut view_handle: vk::ImageViewHandle = std::ptr::null_mut();
        // SAFETY: 同上。
        let rc = unsafe { (fns.create_image_view)(device, &view_info, std::ptr::null(), &mut view_handle) };
        check_vk("vkCreateImageView", rc)?;
        let view = OwnedHandle::destroy(view_handle, device, fns.destroy_image_view);

        // ④ 上传：staging buffer → 图像（一次性提交，返回时图像已可采样）
        self.upload_r8_into_image(image.handle(), w, h, data)?;

        Ok(Texture {
            view,
            image,
            memory: image_memory,
            width: w,
            height: h,
            format,
        })
    }

    /// 把 staging buffer 的内容拷进图像，并把图像转到 `SHADER_READ_ONLY_OPTIMAL`
    /// （**一次性提交并等待完成** ⇒ 返回后图像即可被采样）。
    fn upload_r8_into_image(&self, image: vk::ImageHandle, w: u32, h: u32, data: &[u8]) -> GpuResult<()> {
        let fns = self.fns;
        let device = self.handle;

        // ── staging buffer：HOST_VISIBLE | HOST_COHERENT（不需要 flush）
        let buf_info = vk::BufferCreateInfo {
            s_type: vk::VK_STRUCTURE_TYPE_BUFFER_CREATE_INFO,
            p_next: std::ptr::null(),
            flags: 0,
            size: data.len() as u64,
            usage: vk::VK_BUFFER_USAGE_TRANSFER_SRC_BIT,
            sharing_mode: vk::VK_SHARING_MODE_EXCLUSIVE,
            queue_family_index_count: 0,
            p_queue_family_indices: std::ptr::null(),
        };
        let mut buf_handle: vk::BufferHandle = std::ptr::null_mut();
        // SAFETY: 结构体在栈上；输出句柄可写。
        let rc = unsafe { (fns.create_buffer)(device, &buf_info, std::ptr::null(), &mut buf_handle) };
        check_vk("vkCreateBuffer", rc)?;
        let buffer = OwnedHandle::destroy(buf_handle, device, fns.destroy_buffer);

        let mut breq = std::mem::MaybeUninit::<vk::MemoryRequirements>::uninit();
        // SAFETY: 完整写入结构体。
        unsafe { (fns.get_buffer_memory_requirements)(device, buffer.handle(), breq.as_mut_ptr()) };
        let breq = unsafe { breq.assume_init() };
        let host_bits =
            vk::VK_MEMORY_PROPERTY_HOST_VISIBLE_BIT | vk::VK_MEMORY_PROPERTY_HOST_COHERENT_BIT;
        let bidx = pick_memory_type(self.mem_props, breq.memory_type_bits, host_bits)?;
        let buf_mem = alloc_memory(device, &fns, breq.size, bidx, host_bits)?;
        check_vk(
            "vkBindBufferMemory",
            // SAFETY: 缓冲与内存都是本设备的新对象，尺寸匹配；offset 0 合法。
            unsafe { (fns.bind_buffer_memory)(device, buffer.handle(), buf_mem.handle(), 0) },
        )?;

        // ── 写数据（HOST_COHERENT ⇒ 不需要 vkFlushMappedMemoryRanges）
        {
            let mut ptr: *mut std::ffi::c_void = std::ptr::null_mut();
            // SAFETY: 内存是 HOST_VISIBLE 且尚未映射；offset/size 在范围内。
            let rc = unsafe { (fns.map_memory)(device, buf_mem.handle(), 0, vk::WHOLE_SIZE, 0, &mut ptr) };
            check_vk("vkMapMemory", rc)?;
            if ptr.is_null() {
                return Err(GpuError::Driver {
                    code: -1,
                    message: "vkMapMemory 返回成功但指针为空".to_string(),
                });
            }
            // SAFETY: 映射覆盖整个缓冲（`data.len()` 字节，等于缓冲大小）；
            // 源与目标不重叠；HOST_COHERENT ⇒ 解映射后数据对设备可见。
            unsafe {
                std::ptr::copy_nonoverlapping(data.as_ptr(), ptr as *mut u8, data.len());
                (fns.unmap_memory)(device, buf_mem.handle());
            }
        }

        // ── 一次性命令：布局转换 → 拷贝 → 布局转换
        let pool = self.create_transient_command_pool()?;
        let cmd = self.alloc_one_command_buffer(pool.handle())?;
        let begin = vk::CommandBufferBeginInfo {
            s_type: vk::VK_STRUCTURE_TYPE_COMMAND_BUFFER_BEGIN_INFO,
            p_next: std::ptr::null(),
            flags: vk::VK_COMMAND_BUFFER_USAGE_ONE_TIME_SUBMIT_BIT,
            p_inheritance_info: std::ptr::null(),
        };
        check_vk(
            "vkBeginCommandBuffer",
            // SAFETY: 命令缓冲由本设备分配且未在录制中。
            unsafe { (fns.begin_command_buffer)(cmd, &begin) },
        )?;

        let range = full_subresource_range();
        // UNDEFINED → TRANSFER_DST_OPTIMAL（只写入，不关心旧内容）
        let to_dst = vk::ImageMemoryBarrier {
            s_type: vk::VK_STRUCTURE_TYPE_IMAGE_MEMORY_BARRIER,
            p_next: std::ptr::null(),
            src_access_mask: 0,
            dst_access_mask: vk::VK_ACCESS_TRANSFER_WRITE_BIT,
            old_layout: vk::VK_IMAGE_LAYOUT_UNDEFINED,
            new_layout: vk::VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL,
            src_queue_family_index: vk::QUEUE_FAMILY_IGNORED,
            dst_queue_family_index: vk::QUEUE_FAMILY_IGNORED,
            image,
            subresource_range: range,
        };
        // SAFETY: 命令缓冲正在录制；屏障在栈上存活到调用结束。
        unsafe {
            (fns.cmd_pipeline_barrier)(
                cmd,
                vk::VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT,
                vk::VK_PIPELINE_STAGE_TRANSFER_BIT,
                0,
                0,
                std::ptr::null(),
                0,
                std::ptr::null(),
                1,
                &to_dst,
            )
        };

        let copy = vk::BufferImageCopy {
            buffer_offset: 0,
            // 0 = 「紧密打包」：每行恰好 w 字节、层高恰好 h 行 —— 与主机数据布局
            // 一致，不需要驱动做任何行对齐推断。
            buffer_row_length: 0,
            buffer_image_height: 0,
            image_subresource: vk::ImageSubresourceLayers {
                aspect_mask: vk::VK_IMAGE_ASPECT_COLOR_BIT,
                mip_level: 0,
                base_array_layer: 0,
                layer_count: 1,
            },
            image_offset: vk::Offset3D { x: 0, y: 0, z: 0 },
            image_extent: vk::Extent3D {
                width: w,
                height: h,
                depth: 1,
            },
        };
        // SAFETY: 同上；缓冲与图像都是本设备对象且尺寸匹配。
        unsafe {
            (fns.cmd_copy_buffer_to_image)(
                cmd,
                buffer.handle(),
                image,
                vk::VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL,
                1,
                &copy,
            )
        };

        // TRANSFER_DST_OPTIMAL → SHADER_READ_ONLY_OPTIMAL
        let to_shader = vk::ImageMemoryBarrier {
            s_type: vk::VK_STRUCTURE_TYPE_IMAGE_MEMORY_BARRIER,
            p_next: std::ptr::null(),
            src_access_mask: vk::VK_ACCESS_TRANSFER_WRITE_BIT,
            dst_access_mask: vk::VK_ACCESS_SHADER_READ_BIT,
            old_layout: vk::VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL,
            new_layout: vk::VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
            src_queue_family_index: vk::QUEUE_FAMILY_IGNORED,
            dst_queue_family_index: vk::QUEUE_FAMILY_IGNORED,
            image,
            subresource_range: range,
        };
        // SAFETY: 同上。
        unsafe {
            (fns.cmd_pipeline_barrier)(
                cmd,
                vk::VK_PIPELINE_STAGE_TRANSFER_BIT,
                vk::VK_PIPELINE_STAGE_FRAGMENT_SHADER_BIT,
                0,
                0,
                std::ptr::null(),
                0,
                std::ptr::null(),
                1,
                &to_shader,
            )
        };

        check_vk(
            "vkEndCommandBuffer",
            // SAFETY: 命令缓冲正在录制。
            unsafe { (fns.end_command_buffer)(cmd) },
        )?;

        // ── 提交并等待完成（一次性；此处无并发，等待比栅栏更简单）
        let submit = vk::SubmitInfo {
            s_type: vk::VK_STRUCTURE_TYPE_SUBMIT_INFO,
            p_next: std::ptr::null(),
            wait_semaphore_count: 0,
            p_wait_semaphores: std::ptr::null(),
            p_wait_dst_stage_mask: std::ptr::null(),
            command_buffer_count: 1,
            p_command_buffers: &cmd,
            signal_semaphore_count: 0,
            p_signal_semaphores: std::ptr::null(),
        };
        // SAFETY: 队列是本设备的图形队列；提交后立刻等待它空闲 ⇒ 不存在
        // 「命令缓冲被重用而 GPU 仍在读」的窗口。
        let rc = unsafe { (fns.queue_submit)(self.queue, 1, &submit, vk::NULL_HANDLE) };
        check_vk("vkQueueSubmit", rc)?;
        // SAFETY: 队列属于本设备。
        check_vk("vkQueueWaitIdle", unsafe { (fns.queue_wait_idle)(self.queue) })?;
        Ok(())
        // buffer / buf_mem / pool 在此按声明逆序 `Drop`，队列此刻已空闲。
    }

    /// 把纹理从设备内存**回读**到主机（`vkCmdCopyImageToBuffer` 的一次性路径）。
    ///
    /// ## 为什么需要它（这是「通用纹理」那条验收的关键证据）
    ///
    /// 统一片元着色器只消费纹理的 **R 通道**（`texture(tex,uv).r`，覆盖率语义）
    /// ⇒ 「RGBA8 的 G/B/A 有没有传对」**只看渲染出来的像素是看不出来的**。
    /// 回读是唯一能直接证明「四通道逐字节保真」的判据（见
    /// `tests/texture_indirect.rs::rgba8_texture_upload_preserves_all_four_channels`）。
    ///
    /// ## 语义
    ///
    /// - 返回字节序与上传时**一致**（R8：1 字节/像素；RGBA8：R,G,B,A 4 字节/像素），
    ///   `bufferRowLength = 0` ⇒ 行距紧排（`宽 × bytes_per_pixel`），与主机数据布局同构；
    /// - 结束后图像**回到 `SHADER_READ_ONLY_OPTIMAL`**（与上传完成后一致）
    ///   ⇒ 回读不改变纹理的可采样状态；
    /// - 一次性提交 + `vkQueueWaitIdle`（与上传同款）：本项目没有并发提交，等待比栅栏简单；
    /// - 尺寸/格式从 `texture` 读（不靠调用方传参）⇒ 不存在「参数与纹理不符」的错配。
    pub fn read_texture_bytes(&self, texture: &Texture) -> GpuResult<Vec<u8>> {
        let fns = self.fns;
        let device = self.handle;
        let (w, h) = (texture.width, texture.height);
        let size = texture.byte_len();
        if size == 0 {
            return Err(GpuError::Unsupported(
                "0 字节纹理无法回读（宽或高为 0）".to_string(),
            ));
        }
        let image = texture.image.handle();

        // ── staging buffer：TRANSFER_DST + HOST_VISIBLE | HOST_COHERENT
        let buf_info = vk::BufferCreateInfo {
            s_type: vk::VK_STRUCTURE_TYPE_BUFFER_CREATE_INFO,
            p_next: std::ptr::null(),
            flags: 0,
            size,
            usage: vk::VK_BUFFER_USAGE_TRANSFER_DST_BIT,
            sharing_mode: vk::VK_SHARING_MODE_EXCLUSIVE,
            queue_family_index_count: 0,
            p_queue_family_indices: std::ptr::null(),
        };
        let mut buf_handle: vk::BufferHandle = std::ptr::null_mut();
        // SAFETY: 结构体在栈上；输出句柄可写。
        let rc = unsafe { (fns.create_buffer)(device, &buf_info, std::ptr::null(), &mut buf_handle) };
        check_vk("vkCreateBuffer(readback)", rc)?;
        let buffer = OwnedHandle::destroy(buf_handle, device, fns.destroy_buffer);

        let mut breq = std::mem::MaybeUninit::<vk::MemoryRequirements>::uninit();
        // SAFETY: 完整写入结构体。
        unsafe { (fns.get_buffer_memory_requirements)(device, buffer.handle(), breq.as_mut_ptr()) };
        let breq = unsafe { breq.assume_init() };
        let host_bits =
            vk::VK_MEMORY_PROPERTY_HOST_VISIBLE_BIT | vk::VK_MEMORY_PROPERTY_HOST_COHERENT_BIT;
        let bidx = pick_memory_type(self.mem_props, breq.memory_type_bits, host_bits)?;
        let buf_mem = alloc_memory(device, &fns, breq.size, bidx, host_bits)?;
        check_vk(
            "vkBindBufferMemory(readback)",
            // SAFETY: 缓冲与内存都是本设备的新对象，尺寸匹配；offset 0 合法。
            unsafe { (fns.bind_buffer_memory)(device, buffer.handle(), buf_mem.handle(), 0) },
        )?;

        // ── 一次性命令：SHADER_READ_ONLY → TRANSFER_SRC → 拷贝 → 转回 SHADER_READ_ONLY
        let pool = self.create_transient_command_pool()?;
        let cmd = self.alloc_one_command_buffer(pool.handle())?;
        let begin = vk::CommandBufferBeginInfo {
            s_type: vk::VK_STRUCTURE_TYPE_COMMAND_BUFFER_BEGIN_INFO,
            p_next: std::ptr::null(),
            flags: vk::VK_COMMAND_BUFFER_USAGE_ONE_TIME_SUBMIT_BIT,
            p_inheritance_info: std::ptr::null(),
        };
        check_vk(
            "vkBeginCommandBuffer",
            // SAFETY: 命令缓冲由本设备分配且未在录制中。
            unsafe { (fns.begin_command_buffer)(cmd, &begin) },
        )?;

        let range = full_subresource_range();
        let to_src = vk::ImageMemoryBarrier {
            s_type: vk::VK_STRUCTURE_TYPE_IMAGE_MEMORY_BARRIER,
            p_next: std::ptr::null(),
            src_access_mask: vk::VK_ACCESS_SHADER_READ_BIT,
            dst_access_mask: vk::VK_ACCESS_TRANSFER_READ_BIT,
            old_layout: vk::VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
            new_layout: vk::VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL,
            src_queue_family_index: vk::QUEUE_FAMILY_IGNORED,
            dst_queue_family_index: vk::QUEUE_FAMILY_IGNORED,
            image,
            subresource_range: range,
        };
        // SAFETY: 命令缓冲正在录制；屏障在栈上存活到调用结束。
        unsafe {
            (fns.cmd_pipeline_barrier)(
                cmd,
                vk::VK_PIPELINE_STAGE_FRAGMENT_SHADER_BIT,
                vk::VK_PIPELINE_STAGE_TRANSFER_BIT,
                0,
                0,
                std::ptr::null(),
                0,
                std::ptr::null(),
                1,
                &to_src,
            )
        };

        let copy = vk::BufferImageCopy {
            buffer_offset: 0,
            // 0 = 紧密打包：每行 `宽 × bytes_per_pixel` 字节 —— 与主机数据布局一致
            buffer_row_length: 0,
            buffer_image_height: 0,
            image_subresource: vk::ImageSubresourceLayers {
                aspect_mask: vk::VK_IMAGE_ASPECT_COLOR_BIT,
                mip_level: 0,
                base_array_layer: 0,
                layer_count: 1,
            },
            image_offset: vk::Offset3D { x: 0, y: 0, z: 0 },
            image_extent: vk::Extent3D {
                width: w,
                height: h,
                depth: 1,
            },
        };
        // SAFETY: 缓冲与图像都是本设备对象且尺寸匹配（`size` 由格式算出）。
        unsafe {
            (fns.cmd_copy_image_to_buffer)(
                cmd,
                image,
                vk::VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL,
                buffer.handle(),
                1,
                &copy,
            )
        };

        let back_to_shader = vk::ImageMemoryBarrier {
            s_type: vk::VK_STRUCTURE_TYPE_IMAGE_MEMORY_BARRIER,
            p_next: std::ptr::null(),
            src_access_mask: vk::VK_ACCESS_TRANSFER_READ_BIT,
            dst_access_mask: vk::VK_ACCESS_SHADER_READ_BIT,
            old_layout: vk::VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL,
            new_layout: vk::VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
            src_queue_family_index: vk::QUEUE_FAMILY_IGNORED,
            dst_queue_family_index: vk::QUEUE_FAMILY_IGNORED,
            image,
            subresource_range: range,
        };
        // SAFETY: 同上。
        unsafe {
            (fns.cmd_pipeline_barrier)(
                cmd,
                vk::VK_PIPELINE_STAGE_TRANSFER_BIT,
                vk::VK_PIPELINE_STAGE_FRAGMENT_SHADER_BIT,
                0,
                0,
                std::ptr::null(),
                0,
                std::ptr::null(),
                1,
                &back_to_shader,
            )
        };

        check_vk(
            "vkEndCommandBuffer",
            // SAFETY: 命令缓冲正在录制。
            unsafe { (fns.end_command_buffer)(cmd) },
        )?;

        let submit = vk::SubmitInfo {
            s_type: vk::VK_STRUCTURE_TYPE_SUBMIT_INFO,
            p_next: std::ptr::null(),
            wait_semaphore_count: 0,
            p_wait_semaphores: std::ptr::null(),
            p_wait_dst_stage_mask: std::ptr::null(),
            command_buffer_count: 1,
            p_command_buffers: &cmd,
            signal_semaphore_count: 0,
            p_signal_semaphores: std::ptr::null(),
        };
        // SAFETY: 队列是本设备的图形队列；提交后立刻等空闲 ⇒ 不存在「缓冲被销毁而 GPU 仍在写」的窗口。
        check_vk("vkQueueSubmit", unsafe {
            (fns.queue_submit)(self.queue, 1, &submit, vk::NULL_HANDLE)
        })?;
        // SAFETY: 队列属于本设备。
        check_vk("vkQueueWaitIdle", unsafe { (fns.queue_wait_idle)(self.queue) })?;

        // ── 读回（HOST_COHERENT ⇒ 不需要 vkInvalidateMappedMemoryRanges）
        let mut ptr: *mut std::ffi::c_void = std::ptr::null_mut();
        // SAFETY: 内存是 HOST_VISIBLE 且尚未映射；offset/size 在范围内。
        let rc = unsafe { (fns.map_memory)(device, buf_mem.handle(), 0, vk::WHOLE_SIZE, 0, &mut ptr) };
        check_vk("vkMapMemory(readback)", rc)?;
        if ptr.is_null() {
            return Err(GpuError::Driver {
                code: -1,
                message: "vkMapMemory 返回成功但指针为空".to_string(),
            });
        }
        // SAFETY: 映射覆盖整个缓冲（`size` 字节）；拷贝出来即与映射解耦。
        let out = unsafe { std::slice::from_raw_parts(ptr as *const u8, size as usize).to_vec() };
        // SAFETY: 与上面的 map 配对；缓冲/内存在本函数末尾按声明逆序析构。
        unsafe { (fns.unmap_memory)(device, buf_mem.handle()) };
        Ok(out)
    }

    /// 创建一个 `TRANSIENT` 命令池（用于一次性上传/拷贝）。
    ///
    /// `TRANSIENT` 提示驱动「这个池里的命令缓冲寿命很短」，驱动可据此选更省的分配策略。
    pub fn create_transient_command_pool(&self) -> GpuResult<CommandPool> {
        let info = vk::CommandPoolCreateInfo {
            s_type: vk::VK_STRUCTURE_TYPE_COMMAND_POOL_CREATE_INFO,
            p_next: std::ptr::null(),
            flags: vk::VK_COMMAND_POOL_CREATE_TRANSIENT_BIT,
            queue_family_index: self.queue_family_index,
        };
        let mut handle: vk::CommandPoolHandle = std::ptr::null_mut();
        // SAFETY: 结构体在栈上；输出句柄可写。
        let rc = unsafe {
            (self.fns.create_command_pool)(self.handle, &info, std::ptr::null(), &mut handle)
        };
        check_vk("vkCreateCommandPool", rc)?;
        Ok(CommandPool {
            handle: OwnedHandle::destroy(handle, self.handle, self.fns.destroy_command_pool),
        })
    }

    /// 从池里分配**一个**主命令缓冲。
    fn alloc_one_command_buffer(&self, pool: vk::CommandPoolHandle) -> GpuResult<vk::CommandBufferHandle> {
        let info = vk::CommandBufferAllocateInfo {
            s_type: vk::VK_STRUCTURE_TYPE_COMMAND_BUFFER_ALLOCATE_INFO,
            p_next: std::ptr::null(),
            command_pool: pool,
            level: vk::VK_COMMAND_BUFFER_LEVEL_PRIMARY,
            command_buffer_count: 1,
        };
        let mut handles = [std::ptr::null_mut(); 1];
        // SAFETY: 输出数组长度与 `command_buffer_count` 一致。
        let rc = unsafe { (self.fns.allocate_command_buffers)(self.handle, &info, handles.as_mut_ptr()) };
        check_vk("vkAllocateCommandBuffers", rc)?;
        Ok(handles[0])
    }

    /// 创建最近邻 / ClampToEdge / 无 mipmap 的采样器。
    ///
    /// 参数由纯函数 [`sampler_create_info`] 产出，所以「必须最近邻」这类契约
    /// 在**没有 GPU 的机器上**也能被单测钉住。
    pub fn create_sampler(&self) -> GpuResult<Sampler> {
        let info = sampler_create_info();
        let mut handle: vk::SamplerHandle = std::ptr::null_mut();
        // SAFETY: 结构体在栈上；输出句柄可写。
        let rc = unsafe { (self.fns.create_sampler)(self.handle, &info, std::ptr::null(), &mut handle) };
        check_vk("vkCreateSampler", rc)?;
        Ok(Sampler {
            handle: OwnedHandle::destroy(handle, self.handle, self.fns.destroy_sampler),
        })
    }

    /// 创建一个**只有一个 binding** 的描述符集布局：`binding 0` =
    /// `COMBINED_IMAGE_SAMPLER`（片元阶段可见）。
    ///
    /// ## 为什么是 `COMBINED_IMAGE_SAMPLER` 而不是分开的 sampler + sampled-image
    ///
    /// 分开写需要两个 binding、两份 `VkDescriptorImageInfo`，而着色器里
    /// `OpTypeSampledImage` + `OpImageSampleImplicitLod` 对两者要求完全相同。
    /// 合成一个 binding 少一半描述符管理代码 —— 而描述符管理正是
    /// 「写错不报错、只是画不出来」的重灾区。
    pub fn create_descriptor_set_layout_combined_sampler(&self) -> GpuResult<DescriptorSetLayout> {
        let binding = vk::DescriptorSetLayoutBinding {
            binding: 0,
            descriptor_type: vk::VK_DESCRIPTOR_TYPE_COMBINED_IMAGE_SAMPLER,
            descriptor_count: 1,
            stage_flags: vk::VK_SHADER_STAGE_FRAGMENT_BIT,
            p_immutable_samplers: std::ptr::null(),
        };
        let info = vk::DescriptorSetLayoutCreateInfo {
            s_type: vk::VK_STRUCTURE_TYPE_DESCRIPTOR_SET_LAYOUT_CREATE_INFO,
            p_next: std::ptr::null(),
            flags: 0,
            binding_count: 1,
            p_bindings: &binding,
        };
        let mut handle: vk::DescriptorSetLayoutHandle = std::ptr::null_mut();
        // SAFETY: 结构体与 binding 都在栈上存活到调用结束。
        let rc = unsafe {
            (self.fns.create_descriptor_set_layout)(self.handle, &info, std::ptr::null(), &mut handle)
        };
        check_vk("vkCreateDescriptorSetLayout", rc)?;
        Ok(DescriptorSetLayout {
            handle: OwnedHandle::destroy(handle, self.handle, self.fns.destroy_descriptor_set_layout),
        })
    }

    /// 创建一个描述符池（容量 `max_sets` 个 `COMBINED_IMAGE_SAMPLER`）。
    ///
    /// ⚠️ 池容量必须与用途匹配：可分配的集数受 `max_sets` 与各类型 `descriptorCount`
    /// **双重**限制。不足时 `vkAllocateDescriptorSets` 返回
    /// `VK_ERROR_OUT_OF_POOL_MEMORY`（错误信息里带上 `max_sets`，便于定位）。
    ///
    /// 池**总是**带 `FREE_DESCRIPTOR_SET_BIT`：我们的用法是「单个描述符集随对象
    /// 归还给池」（见 [`DescriptorSet`]），而 `vkFreeDescriptorSets` 要求池建时
    /// 带这个标志 —— 不带时校验层报 `VUID-vkFreeDescriptorSets-descriptorPool-00312`
    /// （实测：这条错误只在**析构**时才出现，很容易被漏掉）。
    pub fn create_descriptor_pool(&self, max_sets: u32) -> GpuResult<DescriptorPool> {
        if max_sets == 0 {
            return Err(GpuError::Unsupported(
                "描述符池的 max_sets 必须 > 0（否则分配必定失败）".to_string(),
            ));
        }
        let size = vk::DescriptorPoolSize {
            descriptor_type: vk::VK_DESCRIPTOR_TYPE_COMBINED_IMAGE_SAMPLER,
            descriptor_count: max_sets,
        };
        let info = vk::DescriptorPoolCreateInfo {
            s_type: vk::VK_STRUCTURE_TYPE_DESCRIPTOR_POOL_CREATE_INFO,
            p_next: std::ptr::null(),
            flags: vk::VK_DESCRIPTOR_POOL_CREATE_FREE_DESCRIPTOR_SET_BIT,
            max_sets,
            pool_size_count: 1,
            p_pool_sizes: &size,
        };
        let mut handle: vk::DescriptorPoolHandle = std::ptr::null_mut();
        // SAFETY: 结构体在栈上存活到调用结束。
        let rc = unsafe { (self.fns.create_descriptor_pool)(self.handle, &info, std::ptr::null(), &mut handle) };
        check_vk("vkCreateDescriptorPool", rc)?;
        Ok(DescriptorPool {
            handle: OwnedHandle::destroy(handle, self.handle, self.fns.destroy_descriptor_pool),
            max_sets,
        })
    }

    /// 从池里分配**一个**描述符集（用给定布局）。
    ///
    /// ## 生命周期契约（调用方必须保证）
    ///
    /// 返回的 [`DescriptorSet`] 析构时会调 `vkFreeDescriptorSets(device, pool, ..)`，
    /// 所以 **`pool` 必须比返回值活得久**。典型用法是把池与集放进同一结构体，
    /// **池的字段声明在集之后**（Rust 按声明顺序析构 ⇒ 后声明的先析构 ⇒ 集先于池）。
    pub fn allocate_descriptor_set(
        &self,
        pool: &DescriptorPool,
        layout: &DescriptorSetLayout,
    ) -> GpuResult<DescriptorSet> {
        let info = vk::DescriptorSetAllocateInfo {
            s_type: vk::VK_STRUCTURE_TYPE_DESCRIPTOR_SET_ALLOCATE_INFO,
            p_next: std::ptr::null(),
            descriptor_pool: pool.handle(),
            descriptor_set_count: 1,
            p_set_layouts: &layout.handle(),
        };
        let mut handle: vk::DescriptorSetHandle = std::ptr::null_mut();
        // SAFETY: 输出句柄可写；池与布局都是本设备对象。
        let rc = unsafe { (self.fns.allocate_descriptor_sets)(self.handle, &info, &mut handle) };
        check_vk("vkAllocateDescriptorSets", rc)?;
        Ok(DescriptorSet {
            handle,
            pool: pool.handle(),
            device: self.handle,
            fns: self.fns,
        })
    }

    /// 把「纹理 + 采样器」写进描述符集的 `binding 0`。
    ///
    /// `imageLayout` 用 [`Texture::layout`]（上传完成后的**实际**布局）——
    /// 写错布局不会编译失败，而是采样读到未定义内容或校验层报警。
    pub fn update_descriptor_texture(
        &self,
        set: &DescriptorSet,
        tex: &Texture,
        s: &Sampler,
    ) -> GpuResult<()> {
        let image_info = vk::DescriptorImageInfo {
            sampler: s.handle(),
            image_view: tex.image_view(),
            image_layout: tex.layout(),
        };
        let write = vk::WriteDescriptorSet {
            s_type: vk::VK_STRUCTURE_TYPE_WRITE_DESCRIPTOR_SET,
            p_next: std::ptr::null(),
            dst_set: set.handle(),
            dst_binding: 0,
            dst_array_element: 0,
            descriptor_count: 1,
            descriptor_type: vk::VK_DESCRIPTOR_TYPE_COMBINED_IMAGE_SAMPLER,
            p_image_info: &image_info,
            p_buffer_info: std::ptr::null(),
            p_texel_buffer_view: std::ptr::null(),
        };
        // SAFETY: `write` 与 `image_info` 在调用期间存活；`dst_set` 是本设备对象；
        // 此处没有在飞的提交（描述符集未被绑定到执行中的命令缓冲）。
        unsafe { (self.fns.update_descriptor_sets)(self.handle, 1, &write, 0, std::ptr::null()) };
        Ok(())
    }

    /// 创建一个**图形管线**（M3c：状态由 [`crate::pipelines::PipelineState`] 统一提供）。
    ///
    /// ## 与 `build_pipeline`（私有）、`create_vertex_pipeline`（M3a 诊断用）的关系
    ///
    /// `build_pipeline` 里的管线状态是**字面量**，M3a 与 M3b 各写了一份；
    /// M3c 把「形状与文本命共同的那 20 多项状态」提到 `pipelines.rs`，由本方法消费。
    /// 于是**离屏与窗口、形状与文本**四条路径共用同一份状态来源 ——
    /// 这正是「提取共用层」要解决的事（复制四份字面量 = 四份将来只改一份的风险）。
    ///
    /// `create_vertex_pipeline` 保留不动：它是 M3a 的路径，且 `pipeline_smoke.rs`
    /// 的 `create_vertex_pipeline_rejects_bad_arguments` 直接依赖它做**参数校验**
    /// 的驱动侧复验。两条路径产出**完全相同**的状态（本方法与它逐字段同值），
    /// 所以离屏改用本方法后行为不变 —— 由 M3a/M3b 的 207 条判据守住。
    ///
    /// ## 顶点输入结构体的生命周期
    ///
    /// `binding` / `descs` / `vertex_input` 三者都在这**一个**作用域里，
    /// `vertex_input` 持有的两处指针在本函数返回前一直有效 ——
    /// 手写 `repr(C)` 结构体最容易踩的坑就是「结构体活着、它指向的东西已经没了」。
    pub fn create_pipeline_from_state(
        &self,
        state: &crate::pipelines::PipelineState,
        stages: &[(&ShaderModule, u32)],
        layout: &PipelineLayout,
        render_pass: &RenderPass,
    ) -> GpuResult<Pipeline> {
        use crate::pipelines::ViewportStrategy;

        // ① viewport/scissor：静态版写进管线、动态版留空（值在录制时给）
        let (vp, sc) = match state.viewport {
            ViewportStrategy::Static { width, height } => (
                vk::Viewport {
                    x: 0.0,
                    y: 0.0,
                    width: width as f32,
                    height: height as f32,
                    min_depth: 0.0,
                    max_depth: 1.0,
                },
                vk::Rect2D {
                    offset: vk::Offset2D { x: 0, y: 0 },
                    extent: vk::Extent2D { width, height },
                },
            ),
            ViewportStrategy::Dynamic => (
                vk::Viewport {
                    x: 0.0,
                    y: 0.0,
                    width: 0.0,
                    height: 0.0,
                    min_depth: 0.0,
                    max_depth: 1.0,
                },
                vk::Rect2D {
                    offset: vk::Offset2D { x: 0, y: 0 },
                    extent: vk::Extent2D {
                        width: 0,
                        height: 0,
                    },
                },
            ),
        };
        let dynamic = matches!(state.viewport, ViewportStrategy::Dynamic);
        let viewport_state = vk::PipelineViewportStateCreateInfo {
            s_type: vk::VK_STRUCTURE_TYPE_PIPELINE_VIEWPORT_STATE_CREATE_INFO,
            p_next: std::ptr::null(),
            flags: 0,
            viewport_count: 1,
            p_viewports: if dynamic { std::ptr::null() } else { &vp },
            scissor_count: 1,
            p_scissors: if dynamic { std::ptr::null() } else { &sc },
        };
        // ⚠️ 动态状态列表必须**只在动态策略下**声明：静态管线声明了动态状态却不设值，
        // 渲染时用的是「未定义」的 viewport（校验层会报，驱动可能画 0 像素）。
        let dynamic_states = [vk::VK_DYNAMIC_STATE_VIEWPORT, vk::VK_DYNAMIC_STATE_SCISSOR];
        let dynamic_state = vk::PipelineDynamicStateCreateInfo {
            s_type: vk::VK_STRUCTURE_TYPE_PIPELINE_DYNAMIC_STATE_CREATE_INFO,
            p_next: std::ptr::null(),
            flags: 0,
            dynamic_state_count: if dynamic { dynamic_states.len() as u32 } else { 0 },
            p_dynamic_states: if dynamic {
                dynamic_states.as_ptr()
            } else {
                std::ptr::null()
            },
        };

        // ② 顶点输入（值 → 结构体，全部在同一作用域）
        let binding = vk::VertexInputBindingDescription {
            binding: 0,
            stride: state.stride,
            input_rate: vk::VK_VERTEX_INPUT_RATE_VERTEX,
        };
        let descs = state.vertex_attr_descs();
        let vertex_input = vk::PipelineVertexInputStateCreateInfo {
            s_type: vk::VK_STRUCTURE_TYPE_PIPELINE_VERTEX_INPUT_STATE_CREATE_INFO,
            p_next: std::ptr::null(),
            flags: 0,
            vertex_binding_description_count: 1,
            p_vertex_binding_descriptions: &binding,
            vertex_attribute_description_count: descs.len() as u32,
            p_vertex_attribute_descriptions: descs.as_ptr(),
        };

        // ③ 颜色混合（`color_blend` 持有所述 attachment 的指针 ⇒ 两者同一作用域）
        let blend_attachment = state.blend;
        let color_blend = vk::PipelineColorBlendStateCreateInfo {
            s_type: vk::VK_STRUCTURE_TYPE_PIPELINE_COLOR_BLEND_STATE_CREATE_INFO,
            p_next: std::ptr::null(),
            flags: 0,
            logic_op_enable: vk::VK_FALSE,
            logic_op: vk::VK_LOGIC_OP_COPY,
            attachment_count: 1,
            p_attachments: &blend_attachment,
            blend_constants: [0.0; 4],
        };

        // ④ 阶段列表
        let entry = c"main";
        let stage_infos: Vec<vk::PipelineShaderStageCreateInfo> = stages
            .iter()
            .map(|(m, stage)| vk::PipelineShaderStageCreateInfo {
                s_type: vk::VK_STRUCTURE_TYPE_PIPELINE_SHADER_STAGE_CREATE_INFO,
                p_next: std::ptr::null(),
                flags: 0,
                stage: *stage,
                module: m.handle(),
                p_name: entry.as_ptr(),
                p_specialization_info: std::ptr::null(),
            })
            .collect();
        if stage_infos.is_empty() {
            return Err(GpuError::Unsupported(
                "图形管线至少要有一个着色器阶段".to_string(),
            ));
        }

        // ⑤ 组装并创建
        let info = vk::GraphicsPipelineCreateInfo {
            s_type: vk::VK_STRUCTURE_TYPE_GRAPHICS_PIPELINE_CREATE_INFO,
            p_next: std::ptr::null(),
            flags: 0,
            stage_count: stage_infos.len() as u32,
            p_stages: stage_infos.as_ptr(),
            p_vertex_input_state: &vertex_input,
            p_input_assembly_state: &state.input_assembly,
            p_tessellation_state: std::ptr::null(),
            p_viewport_state: &viewport_state,
            p_rasterization_state: &state.rasterization,
            p_multisample_state: &state.multisample,
            p_depth_stencil_state: std::ptr::null(),
            p_color_blend_state: &color_blend,
            p_dynamic_state: &dynamic_state,
            layout: layout.handle(),
            render_pass: render_pass.handle(),
            subpass: 0,
            base_pipeline_handle: vk::NULL_HANDLE,
            base_pipeline_index: -1,
        };
        let mut pipeline_handle: vk::PipelineHandle = vk::NULL_HANDLE;
        // SAFETY: 上面所有被引用的结构体都在本函数作用域内存活到这次调用结束；
        // 输出句柄可写；管线缓存为空（合法）。
        let rc = unsafe {
            (self.fns.create_graphics_pipelines)(
                self.handle,
                vk::NULL_HANDLE,
                1,
                &info,
                std::ptr::null(),
                &mut pipeline_handle,
            )
        };
        if rc != ffi::VK_SUCCESS || pipeline_handle.is_null() {
            // 句柄为空但返回成功，是本项目实测过的「驱动不报错也不写句柄」失败方式
            return Err(GpuError::Driver {
                code: rc,
                message: format!(
                    "vkCreateGraphicsPipelines 失败（rc={} handle_is_null={}）：{}",
                    vk_result_name(rc),
                    pipeline_handle.is_null(),
                    "管线创建被拒"
                ),
            });
        }
        Ok(Pipeline {
            handle: pipeline_handle,
            device: self.handle,
            destroy: self.fns.destroy_pipeline,
        })
    }

    /// 创建一个**图形管线**（单颜色附件、无顶点输入、动态 viewport/scissor、alpha 混合开）。
    ///
    /// **这是 SPIR-V 的真正验收关**：`vkCreateShaderModule` 很宽容（实测连 `bound = 0`
    /// 都接受），而 `vkCreateGraphicsPipelines` 会把两个阶段**链接并与管线状态校验**，
    /// 因此它拒绝就意味着着色器或状态真的有问题。
    pub fn create_graphics_pipeline(
        &self,
        vs: &ShaderModule,
        fs: &ShaderModule,
        layout: &PipelineLayout,
        render_pass: &RenderPass,
    ) -> GpuResult<Pipeline> {
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
        self.create_graphics_pipeline_raw(&stages, layout, render_pass)
    }

    /// 建一个**用静态 viewport/scissor** 的图形管线（尺寸写死在管线里）。
    ///
    /// 与动态版本的差别：动态版靠 `vkCmdSetViewport`/`vkCmdSetScissor` 在录制时给值，
    /// 静态版把值放进管线创建信息。
    ///
    /// **为什么两个都要有**：实测发现动态版在本机 Intel 驱动上**画不出任何像素**
    /// （清屏正常、绘制为零）。保留两个版本既能定位问题，也给调用方一个可用选择。
    pub fn create_graphics_pipeline_static_viewport(
        &self,
        vs: &ShaderModule,
        fs: &ShaderModule,
        layout: &PipelineLayout,
        render_pass: &RenderPass,
        width: u32,
        height: u32,
    ) -> GpuResult<Pipeline> {
        self.create_graphics_pipeline_static_viewport_ex(
            vs,
            fs,
            layout,
            render_pass,
            width,
            height,
            true,
        )
    }

    /// 同上，但可以**关掉 alpha 混合**（排查用）。
    ///
    /// 存在的理由：混合状态是最后一个没被排除的管线状态项。若关掉混合就画得出来，
    /// 说明问题在混合配置；否则可以继续排除它。
    #[allow(clippy::too_many_arguments)]
    pub fn create_graphics_pipeline_static_viewport_ex(
        &self,
        vs: &ShaderModule,
        fs: &ShaderModule,
        layout: &PipelineLayout,
        render_pass: &RenderPass,
        width: u32,
        height: u32,
        blend_enable: bool,
    ) -> GpuResult<Pipeline> {
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
        let viewport = vk::Viewport {
            x: 0.0,
            y: 0.0,
            width: width as f32,
            height: height as f32,
            min_depth: 0.0,
            max_depth: 1.0,
        };
        let scissor = vk::Rect2D {
            offset: vk::Offset2D { x: 0, y: 0 },
            extent: vk::Extent2D { width, height },
        };
        self.build_pipeline(
            &stages,
            layout,
            render_pass,
            Some((&viewport, &scissor)),
            &empty_vertex_input(),
            blend_enable,
        )
    }

    /// 用**显式给定**的阶段列表建管线。
    ///
    /// 存在的理由：① 支持非「顶点+片段」的组合（未来加几何/细分阶段）；
    /// ② 让测试能构造**非法**阶段组合，从而验证驱动确实在校验
    /// （没有这条，「建成功」的结论就无法排除「驱动什么都没检查」）。
    pub fn create_graphics_pipeline_raw(
        &self,
        stages: &[vk::PipelineShaderStageCreateInfo],
        layout: &PipelineLayout,
        render_pass: &RenderPass,
    ) -> GpuResult<Pipeline> {
        self.build_pipeline(stages, layout, render_pass, None, &empty_vertex_input(), true)
    }

    /// 建管线的核心：`static_viewport = None` ⇒ 动态 viewport/scissor；`Some` ⇒ 写死。
    ///
    /// `vertex_input` 由调用方给：M2a 的路径传 [`empty_vertex_input`]（位置来自着色器里的
    /// 常量表），M3a 的 [`VkDevice::create_vertex_pipeline`] 传**真实的** binding + attribute
    /// （顶点缓冲路径）。之所以做成参数而不是两个函数：其余 20 多项管线状态**完全相同**，
    /// 复制一份就等于复制一份「以后只改了一边」的风险。
    fn build_pipeline(
        &self,
        stages: &[vk::PipelineShaderStageCreateInfo],
        layout: &PipelineLayout,
        render_pass: &RenderPass,
        static_viewport: Option<(&vk::Viewport, &vk::Rect2D)>,
        vertex_input: &vk::PipelineVertexInputStateCreateInfo,
        blend_enable: bool,
    ) -> GpuResult<Pipeline> {
        if stages.is_empty() {
            return Err(GpuError::Unsupported(
                "图形管线至少要有一个着色器阶段".to_string(),
            ));
        }
        let input_assembly = vk::PipelineInputAssemblyStateCreateInfo {
            s_type: vk::VK_STRUCTURE_TYPE_PIPELINE_INPUT_ASSEMBLY_STATE_CREATE_INFO,
            p_next: std::ptr::null(),
            flags: 0,
            topology: vk::VK_PRIMITIVE_TOPOLOGY_TRIANGLE_LIST,
            primitive_restart_enable: vk::VK_FALSE,
        };
        // viewport/scissor：动态版把值留给录制时给（`count = 1` + 空指针是合法的）；
        // 静态版把值放进管线。**两个版本都要有** —— 实测动态版在本机 Intel 驱动上
        // 画不出任何像素（清屏正常、绘制为零），静态版可用。
        let viewport_state = match static_viewport {
            None => vk::PipelineViewportStateCreateInfo {
                s_type: vk::VK_STRUCTURE_TYPE_PIPELINE_VIEWPORT_STATE_CREATE_INFO,
                p_next: std::ptr::null(),
                flags: 0,
                viewport_count: 1,
                p_viewports: std::ptr::null(),
                scissor_count: 1,
                p_scissors: std::ptr::null(),
            },
            Some((vp, sc)) => vk::PipelineViewportStateCreateInfo {
                s_type: vk::VK_STRUCTURE_TYPE_PIPELINE_VIEWPORT_STATE_CREATE_INFO,
                p_next: std::ptr::null(),
                flags: 0,
                viewport_count: 1,
                p_viewports: vp,
                scissor_count: 1,
                p_scissors: sc,
            },
        };
        let dynamic_states = [vk::VK_DYNAMIC_STATE_VIEWPORT, vk::VK_DYNAMIC_STATE_SCISSOR];
        let dynamic_state = vk::PipelineDynamicStateCreateInfo {
            s_type: vk::VK_STRUCTURE_TYPE_PIPELINE_DYNAMIC_STATE_CREATE_INFO,
            p_next: std::ptr::null(),
            flags: 0,
            dynamic_state_count: if static_viewport.is_none() {
                dynamic_states.len() as u32
            } else {
                0
            },
            p_dynamic_states: if static_viewport.is_none() {
                dynamic_states.as_ptr()
            } else {
                std::ptr::null()
            },
        };
        let raster = vk::PipelineRasterizationStateCreateInfo {
            s_type: vk::VK_STRUCTURE_TYPE_PIPELINE_RASTERIZATION_STATE_CREATE_INFO,
            p_next: std::ptr::null(),
            flags: 0,
            depth_clamp_enable: vk::VK_FALSE,
            rasterizer_discard_enable: vk::VK_FALSE,
            polygon_mode: vk::VK_POLYGON_MODE_FILL,
            cull_mode: vk::VK_CULL_MODE_NONE, // GUI 不做背面剔除（矩形两个朝向都可能）
            front_face: vk::VK_FRONT_FACE_COUNTER_CLOCKWISE,
            depth_bias_enable: vk::VK_FALSE,
            depth_bias_constant_factor: 0.0,
            depth_bias_clamp: 0.0,
            depth_bias_slope_factor: 0.0,
            line_width: 1.0,
        };
        let multisample = vk::PipelineMultisampleStateCreateInfo {
            s_type: vk::VK_STRUCTURE_TYPE_PIPELINE_MULTISAMPLE_STATE_CREATE_INFO,
            p_next: std::ptr::null(),
            flags: 0,
            rasterization_samples: vk::VK_SAMPLE_COUNT_1_BIT,
            sample_shading_enable: vk::VK_FALSE,
            min_sample_shading: 1.0,
            p_sample_mask: std::ptr::null(),
            alpha_to_coverage_enable: vk::VK_FALSE,
            alpha_to_one_enable: vk::VK_FALSE,
        };
        // 标准 alpha 混合：GUI 有半透明面板，必须开
        let blend_attachment = vk::PipelineColorBlendAttachmentState {
            blend_enable: if blend_enable { vk::VK_TRUE } else { vk::VK_FALSE },
            src_color_blend_factor: vk::VK_BLEND_FACTOR_SRC_ALPHA,
            dst_color_blend_factor: vk::VK_BLEND_FACTOR_ONE_MINUS_SRC_ALPHA,
            color_blend_op: vk::VK_BLEND_OP_ADD,
            src_alpha_blend_factor: vk::VK_BLEND_FACTOR_ONE,
            dst_alpha_blend_factor: vk::VK_BLEND_FACTOR_ONE_MINUS_SRC_ALPHA,
            alpha_blend_op: vk::VK_BLEND_OP_ADD,
            color_write_mask: vk::VK_COLOR_COMPONENT_RGBA_BITS,
        };
        let color_blend = vk::PipelineColorBlendStateCreateInfo {
            s_type: vk::VK_STRUCTURE_TYPE_PIPELINE_COLOR_BLEND_STATE_CREATE_INFO,
            p_next: std::ptr::null(),
            flags: 0,
            logic_op_enable: vk::VK_FALSE,
            logic_op: vk::VK_LOGIC_OP_COPY,
            attachment_count: 1,
            p_attachments: &blend_attachment,
            blend_constants: [0.0; 4],
        };
        let info = vk::GraphicsPipelineCreateInfo {
            s_type: vk::VK_STRUCTURE_TYPE_GRAPHICS_PIPELINE_CREATE_INFO,
            p_next: std::ptr::null(),
            flags: 0,
            stage_count: stages.len() as u32,
            p_stages: stages.as_ptr(),
            p_vertex_input_state: vertex_input,
            p_input_assembly_state: &input_assembly,
            p_tessellation_state: std::ptr::null(),
            p_viewport_state: &viewport_state,
            p_rasterization_state: &raster,
            p_multisample_state: &multisample,
            p_depth_stencil_state: std::ptr::null(),
            p_color_blend_state: &color_blend,
            p_dynamic_state: &dynamic_state,
            layout: layout.handle(),
            render_pass: render_pass.handle(),
            subpass: 0,
            base_pipeline_handle: std::ptr::null_mut(),
            base_pipeline_index: -1,
        };
        let mut handle: vk::PipelineHandle = std::ptr::null_mut();
        // SAFETY: 上述结构体与数组都在本栈帧存活；`handle` 是可写输出。
        // pipelineCache 传空（合法）；一次只建一个。
        let rc = unsafe {
            (self.fns.create_graphics_pipelines)(
                self.handle,
                std::ptr::null_mut(),
                1,
                &info,
                std::ptr::null(),
                &mut handle,
            )
        };
        if rc != ffi::VK_SUCCESS {
            return Err(GpuError::Driver {
                code: rc,
                message: format!(
                    "vkCreateGraphicsPipelines 失败（{}）⇒ 着色器或管线状态有问题",
                    vk_result_name(rc)
                ),
            });
        }
        if handle.is_null() {
            // 驱动返回成功却没写输出参数 —— 这类情况几乎总是「我们给的结构体与驱动理解的不一致」。
            return Err(GpuError::Driver {
                code: rc,
                message: "vkCreateGraphicsPipelines 返回成功但管线句柄为空（结构体或参数不符）".to_string(),
            });
        }
        Ok(Pipeline {
            handle,
            device: self.handle,
            destroy: self.fns.destroy_pipeline,
        })
    }
}

/// 一个顶点属性（`vk::VertexInputAttributeDescription` 的项目内形态）。
///
/// 比原生结构少一个 `binding` 字段：顶点缓冲只有 0 号一个，写死比「每次都写对」可靠。
/// `offset` 是**相对顶点起点**的字节偏移，必须落在 `stride` 之内（[`VkDevice::create_vertex_pipeline`] 会检查）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VertexAttr {
    /// 着色器里的 `layout(location = N)`。
    pub location: u32,
    /// `VkFormat`（例如 `VK_FORMAT_R32G32B32A32_SFLOAT`）。
    pub format: i32,
    /// 相对顶点起点的字节偏移。
    pub offset: u32,
}

/// 「没有顶点输入」的顶点输入状态。
///
/// M2a 的着色器把顶点位置写在 SPIR-V 的常量表里（不碰顶点缓冲），
/// 所以那条路径声明 `count = 0`；M3a 的顶点缓冲路径用
/// [`VkDevice::create_vertex_pipeline`] 传真实的 binding + attribute。
fn empty_vertex_input() -> vk::PipelineVertexInputStateCreateInfo {
    vk::PipelineVertexInputStateCreateInfo {
        s_type: vk::VK_STRUCTURE_TYPE_PIPELINE_VERTEX_INPUT_STATE_CREATE_INFO,
        p_next: std::ptr::null(),
        flags: 0,
        vertex_binding_description_count: 0,
        p_vertex_binding_descriptions: std::ptr::null(),
        vertex_attribute_description_count: 0,
        p_vertex_attribute_descriptions: std::ptr::null(),
    }
}

/// 校验 [`VkDevice::create_vertex_pipeline`] 的参数（**纯函数**：不碰 Vulkan、不碰设备）。
///
/// 抽出来的理由（T3 review **F6**，上一轮被静默丢掉的那条）：这 5 条错误路径原本埋在
/// 一个需要真实 `VkDevice` 的方法里，于是**一条测试都没有** —— 而「负例没有测试」等于没有
/// 负例。抽成纯函数后，单元测试在任何机器上都能跑（含无 GPU 的 CI）。
fn validate_vertex_pipeline_args(
    extent: vk::Extent2D,
    stride: u32,
    attrs: &[VertexAttr],
) -> GpuResult<()> {
    if stride == 0 {
        return Err(GpuError::Unsupported("顶点 stride 不能为 0".to_string()));
    }
    if attrs.is_empty() {
        return Err(GpuError::Unsupported(
            "顶点管线至少要有一个属性（否则顶点缓冲毫无意义）".to_string(),
        ));
    }
    if extent.width == 0 || extent.height == 0 {
        return Err(GpuError::Unsupported(format!(
            "静态 viewport 的宽高必须 > 0，实际 {}×{}",
            extent.width, extent.height
        )));
    }
    for (i, a) in attrs.iter().enumerate() {
        if a.offset >= stride {
            return Err(GpuError::Unsupported(format!(
                "属性 {}（location {}）的 offset {} 超出 stride {}",
                i, a.location, a.offset, stride
            )));
        }
        if attrs[..i].iter().any(|b| b.location == a.location) {
            return Err(GpuError::Unsupported(format!(
                "属性 location {} 重复声明",
                a.location
            )));
        }
    }
    Ok(())
}

/// 一个 Vulkan 对象的**销毁**方式。
///
/// 为什么需要它：销毁函数的签名不统一 —— `vkDestroyImage` / `vkDestroyBuffer` /
/// `vkDestroySampler` / `vkDestroyDescriptorSetLayout` / `vkDestroyDescriptorPool`
/// 都是 `(device, handle, pAllocator)`，而**内存**用的是 `vkFreeMemory`。
/// 把两者写成两种**命名的**销毁方式，比「一个万能 `unsafe fn` + 调用方自己记参数
/// 含义」可靠得多：`VkDeviceMemory` 不是「句柄对象」而是内存，调错销毁函数不会
/// 编译失败（都是裸指针），而是运行期把驱动的内存管理器搞坏。
#[derive(Clone, Copy)]
enum DestroyOp {
    /// `(device, handle, pAllocator)` 形态的 `vkDestroy*`。
    Destroy(
        vk::DeviceHandle,
        unsafe extern "system" fn(vk::DeviceHandle, *mut std::ffi::c_void, *const std::ffi::c_void),
    ),
    /// `vkFreeMemory(device, memory, pAllocator)`。
    FreeMemory(vk::DeviceHandle, vk::PfnFreeMemory),
}

/// 一个**自有**的 Vulkan 句柄（创建即拥有，`Drop` 即销毁）。
///
/// 与 `gpu_render.rs` 里那个私有的 `VkObject` 是同一模式。放在 `device.rs` 是因为
/// M3b 的纹理 / 采样器 / 描述符都从这里创建，让「创建 + 销毁」成对出现在**同一个
/// 模块**里，比每个模块各写一遍 `Drop` 更不容易漏（漏掉就是句柄泄漏，而 Vulkan
/// 不会替我们报错）。
struct OwnedHandle {
    handle: *mut std::ffi::c_void,
    op: DestroyOp,
}

impl OwnedHandle {
    fn handle(&self) -> *mut std::ffi::c_void {
        self.handle
    }

    /// 用 `(device, handle, pAllocator)` 形态的 `vkDestroy*` 包装一个句柄。
    fn destroy(
        handle: *mut std::ffi::c_void,
        device: vk::DeviceHandle,
        f: unsafe extern "system" fn(vk::DeviceHandle, *mut std::ffi::c_void, *const std::ffi::c_void),
    ) -> OwnedHandle {
        OwnedHandle {
            handle,
            op: DestroyOp::Destroy(device, f),
        }
    }

    /// 用 `vkFreeMemory` 包装一块设备内存。
    fn memory(handle: vk::DeviceMemoryHandle, device: vk::DeviceHandle, f: vk::PfnFreeMemory) -> OwnedHandle {
        OwnedHandle {
            handle,
            op: DestroyOp::FreeMemory(device, f),
        }
    }
}

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        if self.handle.is_null() {
            return;
        }
        // SAFETY: `handle` 是本设备创建且尚未销毁的对象；`op` 里存的是与之匹配的
        // 销毁函数与设备句柄；设备比本对象活得久（调用方持有 `VkDevice`）。
        unsafe {
            match self.op {
                DestroyOp::Destroy(device, f) => f(device, self.handle, std::ptr::null()),
                DestroyOp::FreeMemory(device, f) => f(device, self.handle, std::ptr::null()),
            }
        }
        self.handle = std::ptr::null_mut();
    }
}

/// 一个 `R8_UNORM` 覆盖率纹理（图像 + 视图 + 设备内存，`Drop` 时全部销毁）。
///
/// **字段顺序 = 析构顺序**：`view` 在前 ⇒ 先 `vkDestroyImageView`、再 `vkDestroyImage`、
/// 最后 `vkFreeMemory`（视图引用图像、图像占用内存，顺序反过来就是使用已释放对象）。
///
/// ## 为什么特意做成 `R8_UNORM`
///
/// 字形图集是一张**单通道**覆盖率位图（每像素 0..255）。用 `R8_UNORM` 时着色器里
/// `texture(tex, uv).r` 直接就是 `cov/255` 的归一化覆盖率 —— 与 CPU 参考
/// `null.rs::draw_text_real` 的 `cov as f32 / 255.0` 逐字对应；换成 `R8G8B8A8`
/// 只是把同一份覆盖率复制四份，浪费 4 倍带宽与显存。
pub struct Texture {
    view: OwnedHandle,
    image: OwnedHandle,
    memory: OwnedHandle,
    width: u32,
    height: u32,
    /// 纹理格式（决定主机数据的每像素字节数与 `VkFormat`）。
    ///
    /// **必须记下来**：回读（[`VkDevice::read_texture_bytes`]）要按它算行距与总字节数；
    /// 不记就只能假定 R8 ⇒ RGBA8 纹理回读会拿到 1/4 的数据（或越界）。
    format: TextureFormat,
}

/// 只打印尺寸（句柄对调用方无意义，而印出裸指针只会让日志变噪）。
impl std::fmt::Debug for Texture {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Texture")
            .field("width", &self.width)
            .field("height", &self.height)
            .finish_non_exhaustive()
    }
}

impl Texture {
    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    /// 纹理格式（R8 覆盖率 / RGBA8 通用彩色）。
    pub fn format(&self) -> TextureFormat {
        self.format
    }

    /// 主机侧数据的总字节数（`w * h * bytes_per_pixel`）。
    pub fn byte_len(&self) -> u64 {
        self.width as u64 * self.height as u64 * self.format.bytes_per_pixel() as u64
    }

    pub fn image_view(&self) -> vk::ImageViewHandle {
        self.view.handle()
    }

    /// 原始图像句柄。
    ///
    /// 存在的理由不只是「以后可能要用」：`image` / `memory` 两个字段**唯一的作用**
    /// 就是「被 Drop 时按序销毁」（视图引用图像、图像占用内存）。只写不读会让
    /// Rust 报 `dead_code`，而用 `#[allow]` 压掉等于把「这两个字段有意义」这条信息
    /// 也一起压掉了。给一个真实可用的读取口，比压 lint 好。
    pub fn image(&self) -> vk::ImageHandle {
        self.image.handle()
    }

    /// 图像绑定的设备内存（重传纹理时需要，见 M3b T4「只在图集变化时重传」）。
    pub fn image_memory_handle(&self) -> vk::DeviceMemoryHandle {
        self.memory.handle()
    }

    /// 纹理上传完成后图像所处的布局（**常量事实**，不是猜测）。
    ///
    /// 必须是这个布局：`vkUpdateDescriptorSets` 里 `imageLayout` 与实际布局不符时，
    /// 校验层会报（驱动可能只是采到垃圾）。上传流程的最后一步就是转到
    /// `SHADER_READ_ONLY_OPTIMAL`，所以这里直接返回它。
    pub fn layout(&self) -> i32 {
        vk::VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL
    }
}

/// 最近邻 / ClampToEdge / 无 mipmap 的采样器。`Drop` 时销毁。
pub struct Sampler {
    handle: OwnedHandle,
}

impl std::fmt::Debug for Sampler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Sampler(nearest, clamp-to-edge, no-mip)").finish()
    }
}

impl Sampler {
    pub fn handle(&self) -> vk::SamplerHandle {
        self.handle.handle()
    }
}

/// 只有一个 binding（`binding 0 = CombinedImageSampler`）的描述符集布局。`Drop` 时销毁。
pub struct DescriptorSetLayout {
    handle: OwnedHandle,
}

impl std::fmt::Debug for DescriptorSetLayout {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DescriptorSetLayout(binding 0 = CombinedImageSampler)")
            .finish()
    }
}

impl DescriptorSetLayout {
    pub fn handle(&self) -> vk::DescriptorSetLayoutHandle {
        self.handle.handle()
    }
}

/// 一个描述符池。`Drop` 时销毁（**会连带释放池内所有描述符集**）。
pub struct DescriptorPool {
    handle: OwnedHandle,
    max_sets: u32,
}

impl std::fmt::Debug for DescriptorPool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DescriptorPool")
            .field("max_sets", &self.max_sets)
            .finish_non_exhaustive()
    }
}

impl DescriptorPool {
    pub fn handle(&self) -> vk::DescriptorPoolHandle {
        self.handle.handle()
    }

    pub fn max_sets(&self) -> u32 {
        self.max_sets
    }
}

/// 一个命令池。`Drop` 时销毁（会连带释放池内所有命令缓冲）。
pub struct CommandPool {
    handle: OwnedHandle,
}

impl CommandPool {
    pub fn handle(&self) -> vk::CommandPoolHandle {
        self.handle.handle()
    }
}

/// 一个描述符集。`Drop` 时**归还给池**（`vkFreeDescriptorSets`），而不是销毁池。
///
/// ## 为什么不借用 `DescriptorPool`（而自己拿着池句柄 + 函数指针）
///
/// Vulkan 的规则是「池活多久，集子最多活多久」。如果这里借用
/// `&'a DescriptorPool`，在 `deer-gpu` 的 `Box<dyn Device>` 抽象下就没法把两者
/// 分开持有 —— 而 Vulkan 的正确用法恰恰是「集先还、池后销」。所以本类型自己持有
/// 归还所需的全部信息，`Drop` 时直接 `vkFreeDescriptorSets`；调用方只需保证
/// 「池比集活得久」（见 [`VkDevice::allocate_descriptor_set`] 的契约）。
///
/// 这里**没有**用 [`OwnedHandle`]：`vkFreeDescriptorSets` 的签名是
/// `(device, pool, count, pSets) -> VkResult`，与 `OwnedHandle` 期望的
/// `(device, handle, pAllocator)` 形态不同。硬塞需要一次函数指针 `transmute`
/// （把「多一个参数」的签名当「少一个参数」来调）—— 那是**未定义行为**，
/// 只是恰好能跑。所以这里老实存原始参数。
pub struct DescriptorSet {
    handle: vk::DescriptorSetHandle,
    pool: vk::DescriptorPoolHandle,
    device: vk::DeviceHandle,
    fns: DeviceFns,
}

impl std::fmt::Debug for DescriptorSet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DescriptorSet").finish_non_exhaustive()
    }
}

impl DescriptorSet {
    pub fn handle(&self) -> vk::DescriptorSetHandle {
        self.handle
    }
}

impl Drop for DescriptorSet {
    fn drop(&mut self) {
        if self.handle.is_null() {
            return;
        }
        let sets = [self.handle];
        // SAFETY: `handle` 由本设备从 `pool` 分配且尚未归还；`pool` 比本对象活得久
        // （调用方契约）；`sets` 的长度 = 1 = 传入的 count。
        unsafe {
            (self.fns.free_descriptor_sets)(self.device, self.pool, 1, sets.as_ptr());
        }
        self.handle = std::ptr::null_mut();
    }
}

/// 校验 [`VkDevice::create_texture_r8`] 的参数（**纯函数**：不碰 Vulkan、不碰设备）。
///
/// 保留这个只认 `R8` 的窄入口（历史调用点与判据都按它的语义写的）；
/// 实现委托给通用版 [`validate_texture_args`]。
///
/// `#[cfg_attr(not(test), allow(dead_code))]` 是**精确**表达：生产路径统一走
/// [`validate_texture_args`]，而这里保留的窄入口只剩本文件末尾的 R8 负例单测在调
/// （保留它 = 保住那批断言对「R8 口径」的覆盖，不是死代码）。
#[cfg_attr(not(test), allow(dead_code))]
fn validate_texture_r8_args(w: u32, h: u32, data: &[u8]) -> GpuResult<()> {
    validate_texture_args(w, h, TextureFormat::R8Unorm, data)
}

/// 本项目支持的**通用纹理格式**（M3+ 第 4 项下半）。
///
/// ## 为什么是这两种
///
/// - [`TextureFormat::R8Unorm`]：字形图集（**单通道覆盖率**）—— 已有能力，保持默认；
/// - [`TextureFormat::Rgba8Unorm`]：**通用彩色纹理**（图标 / 图片 / 任意内容），
///   这是 `gpu-hal.md` 里登记的「纹理上传未做」缺口。
///
/// 两者共用同一条上传路径（staging buffer → `vkCmdCopyBufferToImage` → 转
/// `SHADER_READ_ONLY_OPTIMAL`），差异只在 `VkFormat` 与**每像素字节数**。
///
/// ## 诚实边界（写在这里，别让人以为它比实际更通用）
///
/// - **采样**：当前统一片元着色器只消费纹理的 **R 通道**（`texture(tex,uv).r`，
///   覆盖率语义）⇒ RGBA8 纹理贴出来的颜色由**顶点颜色**决定、由 R 当覆盖率调制。
///   要按 RGB 调制需要一支新的片元着色器，而着色器在 `spirv.rs` ——
///   **不在本任务 scope** ⇒ 已作为未做项登记（见任务报告）。
///   四通道的**上传**保真由 [`VkDevice::read_texture_bytes`] 直接证明（不依赖采样）。
/// - 不支持 mipmap / 数组 / 立方体 / 压缩格式（本项目用不到）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextureFormat {
    /// 单通道 8 位（字形覆盖率）。
    R8Unorm,
    /// 四通道 8 位（通用彩色纹理；字节序 R,G,B,A）。
    Rgba8Unorm,
}

impl TextureFormat {
    /// 对应的 `VkFormat` 值（与 SDK `vulkan_core.h` 一致：37 / 43 是 RGBA8 系列、`R8_UNORM` 是 9）。
    pub fn vulkan_format(self) -> i32 {
        match self {
            TextureFormat::R8Unorm => vk::VK_FORMAT_R8_UNORM,
            TextureFormat::Rgba8Unorm => vk::VK_FORMAT_R8G8B8A8_UNORM,
        }
    }

    /// 每像素字节数（**主机数据的行距口径**：`data.len()` 必须是 `w * h * bpp`）。
    pub fn bytes_per_pixel(self) -> u32 {
        match self {
            TextureFormat::R8Unorm => 1,
            TextureFormat::Rgba8Unorm => 4,
        }
    }

    /// 可读名字（错误信息 / 诊断用）。
    pub fn name(self) -> &'static str {
        match self {
            TextureFormat::R8Unorm => "R8_UNORM",
            TextureFormat::Rgba8Unorm => "R8G8B8A8_UNORM",
        }
    }
}

/// 校验通用纹理的参数（**纯函数**：不碰 Vulkan、不碰设备）。
///
/// 抽成纯函数的理由与 [`validate_vertex_pipeline_args`] 相同：错误路径若埋在
/// 「必须先有真机才能跑」的方法里，就等于**没有负例测试**。
///
/// ## 两条必须拦下的
///
/// - **`w == 0 || h == 0`**：0 边图像在 Vulkan 里非法；某些驱动直接拒绝，另一些会让
///   后续采样读到未定义内存（表现为「画面偶尔花」这种极难查的缺陷）。
/// - **`data.len() != w * h * bpp`**：**最危险**的一条，而且**必须按格式算** ——
///   把 RGBA8 的 `w*h*4` 字节按 R8 校验（或反过来）会：字数不够 ⇒ 上传时越界读；
///   字数多了 ⇒ 静默忽略多余数据、掩盖调用方算错尺寸。
///
/// `w * h * bpp` 用 `u64` 相乘：`u32` 下 65536×65536 会溢出，溢出后与
/// `data.len()` 的比较会得出**错误结论**（甚至反而「通过」）。
pub fn validate_texture_args(
    w: u32,
    h: u32,
    format: TextureFormat,
    data: &[u8],
) -> GpuResult<()> {
    if w == 0 || h == 0 {
        return Err(GpuError::Unsupported(format!(
            "{} 纹理的宽高必须 > 0，实际 {w}×{h}",
            format.name()
        )));
    }
    let expected = w as u64 * h as u64 * format.bytes_per_pixel() as u64;
    if data.len() as u64 != expected {
        return Err(GpuError::Unsupported(format!(
            "{} 纹理数据长度必须等于 宽×高×{bpp} = {w}×{h}×{bpp} = {expected} 字节，实际 {} 字节",
            format.name(),
            data.len(),
            bpp = format.bytes_per_pixel()
        )));
    }
    Ok(())
}

/// 最近邻 / ClampToEdge / 无 mipmap 的采样器参数（**纯函数**，便于单测钉死）。
///
/// ## 为什么是这三个「非默认」选择 —— 它们是与 CPU 逐像素对齐的前提
///
/// CPU 参考 `null.rs::draw_text_real` 的采样方式是
/// `coverage[(slot.y + gy) * atlas_w + (slot.x + gx)]` —— 取**一个**纹素的整数值，
/// 没有任何插值、没有任何环绕。所以：
///
/// - **`Nearest`**：`Linear` 会在字形边缘做双线性插值 ⇒ 与 CPU 的硬边系统性不同，
///   「逐像素一致」这条验收直接失效；
/// - **`ClampToEdge`**：字形图集是多个字形共用的一张**大图**。uv 落在字形边界之外
///   哪怕一点点，`Repeat` 会采到图集**对侧**的纹素（完全无关的字形），表现为
///   「字形边缘出现别的字符的碎片」；
/// - **无 mipmap（`minLod = maxLod = 0`、`mip_levels = 1`）**：有 mip 时硬件按纹理
///   坐标导数选层，小字号会采到模糊层，与 CPU 的清晰点采样不一致。
fn sampler_create_info() -> vk::SamplerCreateInfo {
    vk::SamplerCreateInfo {
        s_type: vk::VK_STRUCTURE_TYPE_SAMPLER_CREATE_INFO,
        p_next: std::ptr::null(),
        flags: 0,
        mag_filter: vk::VK_FILTER_NEAREST,
        min_filter: vk::VK_FILTER_NEAREST,
        mipmap_mode: vk::VK_SAMPLER_MIPMAP_MODE_NEAREST,
        address_mode_u: vk::VK_SAMPLER_ADDRESS_MODE_CLAMP_TO_EDGE,
        address_mode_v: vk::VK_SAMPLER_ADDRESS_MODE_CLAMP_TO_EDGE,
        address_mode_w: vk::VK_SAMPLER_ADDRESS_MODE_CLAMP_TO_EDGE,
        mip_lod_bias: 0.0,
        // 各向异性会带来额外的、依赖实现的多重采样 ⇒ 与 CPU 点采样不可比
        anisotropy_enable: vk::VK_FALSE,
        max_anisotropy: 1.0,
        // 不是阴影比较采样器（`compare_enable` 会改变采样语义）
        compare_enable: vk::VK_FALSE,
        compare_op: vk::VK_COMPARE_OP_ALWAYS,
        // 无 mipmap：把 LOD 区间钉死在 0，硬件就只会采第 0 层
        min_lod: 0.0,
        max_lod: 0.0,
        // border color 只在 CLAMP_TO_BORDER 下生效；给个合法值以免结构体未初始化
        border_color: vk::VK_BORDER_COLOR_FLOAT_TRANSPARENT_BLACK,
        unnormalized_coordinates: vk::VK_FALSE,
    }
}

/// 「整张图、单层单 mip、彩色通道」的子资源范围（M3b 的纹理与屏障共用）。
fn full_subresource_range() -> vk::ImageSubresourceRange {
    vk::ImageSubresourceRange {
        aspect_mask: vk::VK_IMAGE_ASPECT_COLOR_BIT,
        base_mip_level: 0,
        level_count: 1,
        base_array_layer: 0,
        layer_count: 1,
    }
}

/// 挑一个满足 `required` 位的内存类型（找不到就报错，**不静默退回**第一个）。
///
/// 「静默退回」是危险的：拿 `DEVICE_LOCAL` 的索引去要求 `HOST_VISIBLE` 的内存会
/// 在映射时失败（或更糟：映射成功但主机写入对设备不可见）。
fn pick_memory_type(
    props: vk::PhysicalDeviceMemoryProperties,
    type_bits: u32,
    required: u32,
) -> GpuResult<u32> {
    let count = props.memory_type_count.min(32);
    for i in 0..count {
        let ty = &props.memory_types[i as usize];
        if type_bits & (1 << i) != 0 && ty.property_flags & required == required {
            return Ok(i);
        }
    }
    Err(GpuError::Unsupported(format!(
        "找不到满足属性 {required:#x} 的内存类型（memoryTypeBits = {type_bits:#x}）"
    )))
}

/// 分配设备内存。
fn alloc_memory(
    device: vk::DeviceHandle,
    fns: &DeviceFns,
    size: u64,
    memory_type_index: u32,
    _required: u32,
) -> GpuResult<OwnedHandle> {
    let info = vk::MemoryAllocateInfo {
        s_type: vk::VK_STRUCTURE_TYPE_MEMORY_ALLOCATE_INFO,
        p_next: std::ptr::null(),
        allocation_size: size,
        memory_type_index,
    };
    let mut handle: vk::DeviceMemoryHandle = std::ptr::null_mut();
    // SAFETY: 结构体在栈上；输出句柄可写。
    let rc = unsafe { (fns.allocate_memory)(device, &info, std::ptr::null(), &mut handle) };
    check_vk("vkAllocateMemory", rc)?;
    Ok(OwnedHandle::memory(handle, device, fns.free_memory))
}

/// 把 `VkResult` 变成 `GpuResult`（成功返回 `Ok(())`）。
fn check_vk(what: &str, rc: i32) -> GpuResult<()> {
    if rc == ffi::VK_SUCCESS {
        Ok(())
    } else {
        Err(GpuError::Driver {
            code: rc,
            message: format!("{what} 失败：{}", vk_result_name(rc)),
        })
    }
}

/// 一个渲染通道。`Drop` 时销毁。
pub struct RenderPass {
    handle: vk::RenderPassHandle,
    device: vk::DeviceHandle,
    destroy: vk::PfnDestroyRenderPass,
    /// 建通道时给的 `finalLayout`（颜色附件离开渲染通道后所处的布局）。
    ///
    /// **为什么必须记下来**：渲染完成后要对图像做别的操作（例如 `vkCmdCopyImageToBuffer`
    /// 回读）时，`VkImageMemoryBarrier.oldLayout` 必须是**图像当前的实际布局**。
    /// 曾经在 `offscreen.rs` 里写死 `COLOR_ATTACHMENT_OPTIMAL`，而这里的 `finalLayout`
    /// 是 `TRANSFER_SRC_OPTIMAL` ⇒ 校验层报 `cannot transition the layout ...`。
    /// 现在由调用方问 [`RenderPass::final_layout`] 拿事实，不再靠记忆。
    final_layout: i32,
}

impl RenderPass {
    pub fn handle(&self) -> vk::RenderPassHandle {
        self.handle
    }

    /// 颜色附件离开渲染通道后的布局（即建通道时传入的 `finalLayout`）。
    pub fn final_layout(&self) -> i32 {
        self.final_layout
    }
}

impl Drop for RenderPass {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            // SAFETY: 句柄由本设备创建且未销毁；设备此时仍存活。
            unsafe { (self.destroy)(self.device, self.handle, std::ptr::null()) };
            self.handle = std::ptr::null_mut();
        }
    }
}

/// 一个管线布局。`Drop` 时销毁。
pub struct PipelineLayout {
    handle: vk::PipelineLayoutHandle,
    device: vk::DeviceHandle,
    destroy: vk::PfnDestroyPipelineLayout,
}

impl PipelineLayout {
    pub fn handle(&self) -> vk::PipelineLayoutHandle {
        self.handle
    }
}

impl Drop for PipelineLayout {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            // SAFETY: 同上。
            unsafe { (self.destroy)(self.device, self.handle, std::ptr::null()) };
            self.handle = std::ptr::null_mut();
        }
    }
}

/// 一个图形管线。`Drop` 时销毁。
pub struct Pipeline {
    handle: vk::PipelineHandle,
    device: vk::DeviceHandle,
    destroy: vk::PfnDestroyPipeline,
}

impl Pipeline {
    pub fn handle(&self) -> vk::PipelineHandle {
        self.handle
    }
}

impl Drop for Pipeline {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            // SAFETY: 同上。
            unsafe { (self.destroy)(self.device, self.handle, std::ptr::null()) };
            self.handle = std::ptr::null_mut();
        }
    }
}

impl Drop for VkDevice {
    fn drop(&mut self) {
        // 先让后台线程销毁 Vulkan 对象，再回收线程
        if let Some(tx) = self.stop.take() {
            let _ = tx.send(());
        }
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// 一个着色器模块。`Drop` 时销毁。
pub struct ShaderModule {
    handle: vk::ShaderModuleHandle,
    device: vk::DeviceHandle,
    destroy: vk::PfnDestroyShaderModule,
}

impl ShaderModule {
    pub fn handle(&self) -> vk::ShaderModuleHandle {
        self.handle
    }
}

impl Drop for ShaderModule {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            // SAFETY: 句柄由本设备的 `vkCreateShaderModule` 创建且未销毁；
            // 设备此时仍存活（`VkDevice` 的 Drop 会先等线程，而模块应在它之前析构）。
            unsafe { (self.destroy)(self.device, self.handle, std::ptr::null()) };
            self.handle = std::ptr::null_mut();
        }
    }
}

struct ReadyInfo {
    handle: vk::DeviceHandle,
    mem_props: vk::PhysicalDeviceMemoryProperties,
    queue: vk::QueueHandle,
    queue_family_index: u32,
    present_queue: vk::QueueHandle,
    present_queue_family_index: u32,
    physical_device: ffi::PhysicalDeviceHandle,
    fns: DeviceFns,
    adapter: deer_gpu::AdapterInfo,
    memory_type_count: u32,
    /// 见 [`VkDevice::validation_enabled`]（`Own` 是实例的**事实**，`Borrowed` 是**请求值**）。
    validation_enabled: bool,
}

// SAFETY: Vulkan 句柄是**进程级的不透明值**（`VK_NULL_HANDLE` 之外的任何句柄都可跨线程
// 传递；线程安全性由「不在两个线程同时使用同一个队列」这类规则约束，而不是由句柄类型决定）。
// 这里只是在「后台线程 → 主线程」单向传递一次句柄，主线程随后独占使用。
// `DeviceFns` 全是函数指针，天然 `Send`。
unsafe impl Send for ReadyInfo {}

/// 设备创建时「实例从哪来」。
enum InstancePlan {
    /// 自己创建实例（并拥有、销毁它）—— `VkDevice::open` 用。
    Own,
    /// 借用调用方（surface）的实例，**不拥有、不销毁** —— `VkDevice::open_with_present` 用。
    Borrowed(PresentTarget),
}

/// 借用路径需要的最小信息（全是可跨线程的值，不含 `Instance` 本身）。
struct PresentTarget {
    /// `InstanceHandle` 的裸值（非 0）。
    instance: usize,
    /// `SurfaceHandle` 的裸值。
    surface: usize,
    /// 该实例的「枚举物理设备 + 取属性」函数。
    core: ffi::CoreFns,
    /// `vkGetPhysicalDeviceSurfaceSupportKHR`（实例级函数，但取到后是纯指针）。
    support: crate::surface::PfnGetPhysicalDeviceSurfaceSupportKHR,
}

/// 后台线程：创建实例与设备，报告句柄，然后 `park` 等停止信号。
fn lifetime_thread(
    adapter_index: usize,
    plan: InstancePlan,
    ready: Sender<Result<ReadyInfo, GpuError>>,
    stop: Receiver<()>,
) {
    let (info, owned_instance) = match create_device(adapter_index, &plan) {
        Ok(v) => v,
        Err(e) => {
            let _ = ready.send(Err(e));
            return;
        }
    };
    let destroy_device = info.fns.destroy_device;
    let device_wait_idle = info.fns.device_wait_idle;
    let device = info.handle;

    if ready.send(Ok(info)).is_err() {
        // 主线程已经不等了 ⇒ 立刻清理
        // SAFETY: 设备刚创建、尚未销毁。
        unsafe { (destroy_device)(device, std::ptr::null()) };
        return;
    }

    // 等停止信号（`recv` 在发送端析构时也会返回，避免永久阻塞）
    let _ = stop.recv();

    // ⚠️ **必须先 `vkDeviceWaitIdle` 再销毁设备**（T3 fix round 2 / R3）。
    //
    // 存在的理由：`VkDevice` 的 `Drop` 只是「发停止信号 + join 本线程」，它**不知道**
    // 队列上还有没有在飞的提交。正常路径下调用方已经等到栅栏，但**异常路径**（栅栏超时、
    // 设备丢失）下可能有提交仍在执行，此时销毁仍在被 GPU 使用的对象是**未定义行为** ——
    // 也就是说「超时后不再复用」这条修复只是把 UB 从「复用」搬到了「销毁」。
    //
    // 失败不报错：设备可能已经丢失（那时 `VK_ERROR_DEVICE_LOST`），而我们此时除了销毁别无他法。
    // SAFETY: 设备由本线程创建、尚未销毁。
    let idle_rc = unsafe { (device_wait_idle)(device) };
    if idle_rc != ffi::VK_SUCCESS {
        eprintln!(
            "[deer-vk] 销毁设备前 vkDeviceWaitIdle 失败（{}）—— 按可能的设备丢失处理，继续销毁",
            vk_result_name(idle_rc)
        );
    }

    // 销毁顺序：设备 → 实例（Own 时 `owned_instance` 的 Drop 负责后者；
    // Borrowed 时它是 None —— 例 **不属于我们**，绝不能在这里销毁）。
    // SAFETY: 设备由本线程创建、尚未销毁，且此刻没有其他线程在使用它
    // （`VkDevice` 的 Drop 会先 join 本线程）。
    unsafe { (destroy_device)(device, std::ptr::null()) };
    drop(owned_instance);
}

/// 创建设备（在后台线程里跑）：实例 → 物理设备 → 队列族 → 设备扩展 → `vkCreateDevice`。
///
/// 返回 `(就绪信息, 需要本线程保活的实例)`：`Own` 时是 `Some`（最后销毁），
/// `Borrowed` 时是 `None`（借来的实例由调用方管）。
fn create_device(
    adapter_index: usize,
    plan: &InstancePlan,
) -> GpuResult<(ReadyInfo, Option<ffi::Instance>)> {
    // ① 实例（自己建 or 借用）
    //
    // `DEER_VK_VALIDATION=1` ⇒ 这条路径也开校验层（与 `VkBackend::new` / `WindowedRenderer` 一致）。
    //
    // 曾经这里**故意不读**这个环境变量：那时 offscreen 路径有 3 个真缺陷
    // （barrier sType 写成 47、oldLayout 与渲染通道 finalLayout 不符、图像内存 `mem::forget` 泄漏），
    // 而且损坏的推送常量着色器会让进程 0xc0000005 崩溃 —— 接上校验层就会吐一堆消息/崩溃。
    // 这些已在 task-18 全部修掉（着色器那条在测试里加了显式「地雷门」），所以现在接回来，
    // 让「`DEER_VK_VALIDATION=1` 跑全量 deer-vk」真正覆盖设备/离屏/窗口三条路径。
    let owned_instance = match plan {
        InstancePlan::Own => Some(ffi::Instance::create_with_validation(
            ffi::Instance::validation_from_env(),
        )?),
        InstancePlan::Borrowed(_) => None,
    };
    // 校验层「真的启用了没有」：`Own` 时问实例（**事实**）；`Borrowed` 时只能给出**请求值**
    // （借来的实例归调用方，句柄里查不到层状态 —— 见 `VkDevice::validation_enabled`）。
    let validation_enabled_fact = match &owned_instance {
        Some(inst) => inst.validation_enabled(),
        None => ffi::Instance::validation_from_env(),
    };
    let (instance_handle, core) = match plan {
        InstancePlan::Own => {
            let inst = owned_instance
                .as_ref()
                .expect("Own 分支刚刚创建了实例");
            (inst.handle(), inst.core_fns())
        }
        InstancePlan::Borrowed(t) => (t.instance as ffi::InstanceHandle, t.core),
    };

    // SAFETY: `instance_handle` 在本函数期间一直存活 —— Own 时由 `owned_instance` 持有，
    // Borrowed 时由调用方按 `open_with_present` 的生命周期契约保证。
    let physical_devices = unsafe { core.enumerate_physical_devices(instance_handle)? };
    let Some(pd) = physical_devices.get(adapter_index).copied() else {
        return Err(GpuError::NoAdapter);
    };

    let fns = resolve_device_fns()?;

    // ② 队列族：图形 +（呈现路径）能向该 surface 呈现
    let mut count: u32 = 0;
    // SAFETY: 传 null 是 Vulkan 规定的「只查数量」用法。
    unsafe { (fns.get_queue_family_properties)(pd, &mut count, std::ptr::null_mut()) };
    if count == 0 {
        return Err(GpuError::NoAdapter);
    }
    let mut families = vec![vk::QueueFamilyProperties {
        queue_flags: 0,
        queue_count: 0,
        timestamp_valid_bits: 0,
        min_image_transfer_granularity: vk::Extent3D {
            width: 0,
            height: 0,
            depth: 0,
        },
    }; count as usize];
    // SAFETY: 数组容量与 `count` 一致。
    unsafe { (fns.get_queue_family_properties)(pd, &mut count, families.as_mut_ptr()) };
    families.truncate(count as usize);

    let mut chosen_family: Option<u32> = None;
    for (i, f) in families.iter().enumerate() {
        if f.queue_flags & vk::VK_QUEUE_GRAPHICS_BIT == 0 {
            continue;
        }
        match plan {
            InstancePlan::Own => {
                chosen_family = Some(i as u32);
                break;
            }
            InstancePlan::Borrowed(t) => {
                let mut supported: u32 = 0;
                // SAFETY: `pd` 来自 `instance_handle`；surface 按契约来自同一实例；
                // `supported` 是可写输出。
                let rc = unsafe {
                    (t.support)(
                        pd,
                        i as u32,
                        t.surface as ffi::SurfaceHandle,
                        &mut supported,
                    )
                };
                if rc != ffi::VK_SUCCESS {
                    return Err(GpuError::Driver {
                        code: rc,
                        message: format!(
                            "vkGetPhysicalDeviceSurfaceSupportKHR 失败（队列族 {i}）：{}",
                            vk_result_name(rc)
                        ),
                    });
                }
                if supported == vk::VK_TRUE {
                    chosen_family = Some(i as u32);
                    break;
                }
            }
        }
    }
    let Some(qfi) = chosen_family else {
        return Err(match plan {
            InstancePlan::Own => GpuError::Unsupported("没有任何队列族支持图形操作".to_string()),
            InstancePlan::Borrowed(_) => GpuError::Unsupported(
                "本机这个设备**没有**任何队列族同时支持「图形」与「在该窗口上呈现」\
                 ⇒ 无法为这个窗口建交换链（换一张显卡试试，例如 DEER_WINDOW_ADAPTER=1）"
                    .to_string(),
            ),
        });
    };

    // ③ 设备扩展：呈现路径必须启用 `VK_KHR_swapchain`（交换链是**设备**扩展）
    let mut enabled_extensions: [*const std::ffi::c_char; 1] = [std::ptr::null()];
    let enable_swapchain = matches!(plan, InstancePlan::Borrowed(_));
    if enable_swapchain {
        let ext = crate::swapchain::SWAPCHAIN_EXTENSION;
        if !ffi::device_extension_available(pd, ext)? {
            return Err(GpuError::Unsupported(format!(
                "本机这个物理设备不支持设备扩展 {ext}（交换链必需）\
                 ⇒ 无法呈现到窗口（可以换一张显卡试试）"
            )));
        }
        enabled_extensions[0] = c"VK_KHR_swapchain".as_ptr();
    }

    let priority: f32 = 1.0;
    let queue_info = vk::DeviceQueueCreateInfo {
        s_type: vk::VK_STRUCTURE_TYPE_DEVICE_QUEUE_CREATE_INFO,
        p_next: std::ptr::null(),
        flags: 0,
        queue_family_index: qfi,
        queue_count: 1,
        p_queue_priorities: &priority,
    };
    let device_info = vk::DeviceCreateInfo {
        s_type: vk::VK_STRUCTURE_TYPE_DEVICE_CREATE_INFO,
        p_next: std::ptr::null(),
        flags: 0,
        queue_create_info_count: 1,
        p_queue_create_infos: &queue_info,
        enabled_layer_count: 0,
        pp_enabled_layer_names: std::ptr::null(),
        enabled_extension_count: if enable_swapchain { 1 } else { 0 },
        pp_enabled_extension_names: if enable_swapchain {
            enabled_extensions.as_ptr()
        } else {
            std::ptr::null()
        },
        p_enabled_features: std::ptr::null(),
    };
    let mut device: vk::DeviceHandle = std::ptr::null_mut();
    // SAFETY: 上述结构体都在本栈帧存活；扩展名是 `'static` C 字符串字面量；
    // 句柄是可写输出。
    let rc = unsafe { (fns.create_device)(pd, &device_info, std::ptr::null(), &mut device) };
    if rc != ffi::VK_SUCCESS {
        return Err(GpuError::Driver {
            code: rc,
            message: format!(
                "vkCreateDevice 失败：{}{}",
                vk_result_name(rc),
                if enable_swapchain {
                    "（本次启用了设备扩展 VK_KHR_swapchain）"
                } else {
                    ""
                }
            ),
        });
    }

    let mut queue: vk::QueueHandle = std::ptr::null_mut();
    // SAFETY: 设备刚创建、队列族存在、索引 0 合法。
    unsafe { (fns.get_device_queue)(device, qfi, 0, &mut queue) };

    // 内存类型数（诊断用）
    let mut mem_props = std::mem::MaybeUninit::<vk::PhysicalDeviceMemoryProperties>::uninit();
    // SAFETY: 该函数完整写入结构体。
    unsafe { (fns.get_memory_properties)(pd, mem_props.as_mut_ptr()) };
    let mem_props_value = unsafe { mem_props.assume_init() };
    let memory_type_count = mem_props_value.memory_type_count;

    // SAFETY: `pd` 来自 `instance_handle`，实例在 `owned_instance` 或调用方手里存活。
    let adapter = match unsafe { core.properties(pd) } {
        Ok(p) => deer_gpu::AdapterInfo {
            name: p.device_name,
            kind: match p.device_type {
                ffi::PhysicalDeviceType::DiscreteGpu => deer_gpu::AdapterKind::DiscreteGpu,
                ffi::PhysicalDeviceType::IntegratedGpu => deer_gpu::AdapterKind::IntegratedGpu,
                ffi::PhysicalDeviceType::VirtualGpu => deer_gpu::AdapterKind::VirtualGpu,
                ffi::PhysicalDeviceType::Cpu => deer_gpu::AdapterKind::Cpu,
                ffi::PhysicalDeviceType::Other => deer_gpu::AdapterKind::Other,
            },
            driver: format!("Vulkan apiVersion {:#010x}", p.api_version),
        },
        Err(e) => {
            // SAFETY: 设备刚创建、尚未销毁。
            unsafe { (fns.destroy_device)(device, std::ptr::null()) };
            return Err(e);
        }
    };

    Ok((
        ReadyInfo {
            handle: device,
            queue,
            queue_family_index: qfi,
            // 单队列族实现：图形与呈现共用同一个队列
            present_queue: queue,
            present_queue_family_index: qfi,
            physical_device: pd,
            fns,
            adapter,
            memory_type_count,
            // `Own` 时这是**实例的事实**；`Borrowed` 时是**请求值**（见 `validation_enabled`）。
            validation_enabled: validation_enabled_fact,
            mem_props: mem_props_value,
        },
        owned_instance,
    ))
}

/// 解析设备级函数表（全是函数指针，故 `Send`，可拷进后台线程与 `VkDevice`）。
fn resolve_device_fns() -> GpuResult<DeviceFns> {
    let lib = Lib::open()?;
    // SAFETY: 每个符号名都与 `vk::Pfn*` 声明的签名一致（见 ffi_dev.rs 的类型定义）。
    // 这些函数在 Vulkan 1.0 就是全局导出的，所以不需要 vkGetInstanceProcAddr。
    unsafe {
        Ok(DeviceFns {
            create_device: lib.sym("vkCreateDevice")?,
            destroy_device: lib.sym("vkDestroyDevice")?,
            get_device_queue: lib.sym("vkGetDeviceQueue")?,
            get_queue_family_properties: lib.sym("vkGetPhysicalDeviceQueueFamilyProperties")?,
            get_memory_properties: lib.sym("vkGetPhysicalDeviceMemoryProperties")?,
            create_shader_module: lib.sym("vkCreateShaderModule")?,
            destroy_shader_module: lib.sym("vkDestroyShaderModule")?,
            create_render_pass: lib.sym("vkCreateRenderPass")?,
            destroy_render_pass: lib.sym("vkDestroyRenderPass")?,
            create_pipeline_layout: lib.sym("vkCreatePipelineLayout")?,
            destroy_pipeline_layout: lib.sym("vkDestroyPipelineLayout")?,
            create_graphics_pipelines: lib.sym("vkCreateGraphicsPipelines")?,
            destroy_pipeline: lib.sym("vkDestroyPipeline")?,
            create_image: lib.sym("vkCreateImage")?,
            destroy_image: lib.sym("vkDestroyImage")?,
            create_image_view: lib.sym("vkCreateImageView")?,
            destroy_image_view: lib.sym("vkDestroyImageView")?,
            get_image_memory_requirements: lib.sym("vkGetImageMemoryRequirements")?,
            get_buffer_memory_requirements: lib.sym("vkGetBufferMemoryRequirements")?,
            allocate_memory: lib.sym("vkAllocateMemory")?,
            free_memory: lib.sym("vkFreeMemory")?,
            bind_image_memory: lib.sym("vkBindImageMemory")?,
            bind_buffer_memory: lib.sym("vkBindBufferMemory")?,
            create_buffer: lib.sym("vkCreateBuffer")?,
            destroy_buffer: lib.sym("vkDestroyBuffer")?,
            map_memory: lib.sym("vkMapMemory")?,
            unmap_memory: lib.sym("vkUnmapMemory")?,
            create_command_pool: lib.sym("vkCreateCommandPool")?,
            destroy_command_pool: lib.sym("vkDestroyCommandPool")?,
            allocate_command_buffers: lib.sym("vkAllocateCommandBuffers")?,
            begin_command_buffer: lib.sym("vkBeginCommandBuffer")?,
            end_command_buffer: lib.sym("vkEndCommandBuffer")?,
            reset_command_buffer: lib.sym("vkResetCommandBuffer")?,
            create_framebuffer: lib.sym("vkCreateFramebuffer")?,
            destroy_framebuffer: lib.sym("vkDestroyFramebuffer")?,
            create_fence: lib.sym("vkCreateFence")?,
            destroy_fence: lib.sym("vkDestroyFence")?,
            wait_for_fences: lib.sym("vkWaitForFences")?,
            reset_fences: lib.sym("vkResetFences")?,
            queue_submit: lib.sym("vkQueueSubmit")?,
            queue_wait_idle: lib.sym("vkQueueWaitIdle")?,
            device_wait_idle: lib.sym("vkDeviceWaitIdle")?,
            cmd_begin_render_pass: lib.sym("vkCmdBeginRenderPass")?,
            cmd_end_render_pass: lib.sym("vkCmdEndRenderPass")?,
            cmd_bind_pipeline: lib.sym("vkCmdBindPipeline")?,
            cmd_bind_vertex_buffers: lib.sym("vkCmdBindVertexBuffers")?,
            cmd_set_viewport: lib.sym("vkCmdSetViewport")?,
            cmd_set_scissor: lib.sym("vkCmdSetScissor")?,
            cmd_draw: lib.sym("vkCmdDraw")?,
            cmd_bind_index_buffer: lib.sym("vkCmdBindIndexBuffer")?,
            cmd_draw_indexed_indirect: lib.sym("vkCmdDrawIndexedIndirect")?,
            cmd_push_constants: lib.sym("vkCmdPushConstants")?,
            cmd_pipeline_barrier: lib.sym("vkCmdPipelineBarrier")?,
            cmd_copy_image_to_buffer: lib.sym("vkCmdCopyImageToBuffer")?,
            cmd_clear_color_image: lib.sym("vkCmdClearColorImage")?,
            create_sampler: lib.sym("vkCreateSampler")?,
            destroy_sampler: lib.sym("vkDestroySampler")?,
            create_descriptor_set_layout: lib.sym("vkCreateDescriptorSetLayout")?,
            destroy_descriptor_set_layout: lib.sym("vkDestroyDescriptorSetLayout")?,
            create_descriptor_pool: lib.sym("vkCreateDescriptorPool")?,
            destroy_descriptor_pool: lib.sym("vkDestroyDescriptorPool")?,
            allocate_descriptor_sets: lib.sym("vkAllocateDescriptorSets")?,
            free_descriptor_sets: lib.sym("vkFreeDescriptorSets")?,
            update_descriptor_sets: lib.sym("vkUpdateDescriptorSets")?,
            cmd_bind_descriptor_sets: lib.sym("vkCmdBindDescriptorSets")?,
            cmd_copy_buffer_to_image: lib.sym("vkCmdCopyBufferToImage")?,
        })
    }
}

/// `VkResult` 的可读名（便于定位）。
pub fn vk_result_name(rc: i32) -> &'static str {
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

#[cfg(test)]
mod tests {
    use super::*;

    /// 好参数：1 个 binding、属性 offset 落在 stride 内、extent 非零。
    fn good_attrs() -> [VertexAttr; 2] {
        [
            VertexAttr {
                location: 0,
                format: vk::VK_FORMAT_R32G32_SFLOAT,
                offset: 0,
            },
            VertexAttr {
                location: 1,
                format: vk::VK_FORMAT_R32G32B32A32_SFLOAT,
                offset: 8,
            },
        ]
    }

    /// **F6 的 5 条错误路径**（T3 review 点名、上一轮被静默丢掉的那条）。
    ///
    /// 一条一个断言，且都断言**错误信息**（不只是 `is_err()`）—— 否则「拒了但拒错原因」也算通过。
    #[test]
    fn vertex_pipeline_args_reject_all_five_bad_inputs() {
        let ok_extent = vk::Extent2D {
            width: 16,
            height: 16,
        };
        let attrs = good_attrs();

        // ① stride == 0
        let e = validate_vertex_pipeline_args(ok_extent, 0, &attrs).expect_err("stride 0 必须被拒");
        assert!(format!("{e}").contains("stride"), "{e}");

        // ② attrs 为空
        let e = validate_vertex_pipeline_args(ok_extent, 44, &[]).expect_err("空属性表必须被拒");
        assert!(format!("{e}").contains("属性"), "{e}");

        // ③ extent 有 0 边（宽为 0）
        let e = validate_vertex_pipeline_args(
            vk::Extent2D {
                width: 0,
                height: 8,
            },
            44,
            &attrs,
        )
        .expect_err("viewport 宽为 0 必须被拒");
        assert!(format!("{e}").contains("viewport"), "{e}");

        // ③b extent 有 0 边（高为 0）—— 两个方向都试，避免只查了一个字段
        let e = validate_vertex_pipeline_args(
            vk::Extent2D {
                width: 8,
                height: 0,
            },
            44,
            &attrs,
        )
        .expect_err("viewport 高为 0 必须被拒");
        assert!(format!("{e}").contains("viewport"), "{e}");

        // ④ 某个属性的 offset ≥ stride
        let bad_offset = [
            VertexAttr {
                location: 0,
                format: vk::VK_FORMAT_R32G32_SFLOAT,
                offset: 0,
            },
            VertexAttr {
                location: 1,
                format: vk::VK_FORMAT_R32G32B32A32_SFLOAT,
                offset: 44, // == stride ⇒ 已越界（记录从 stride 起就走出了本顶点）
            },
        ];
        let e = validate_vertex_pipeline_args(ok_extent, 44, &bad_offset)
            .expect_err("offset ≥ stride 必须被拒");
        assert!(format!("{e}").contains("offset"), "{e}");

        // ⑤ location 重复
        let dup = [
            VertexAttr {
                location: 1,
                format: vk::VK_FORMAT_R32G32_SFLOAT,
                offset: 0,
            },
            VertexAttr {
                location: 1,
                format: vk::VK_FORMAT_R32G32B32A32_SFLOAT,
                offset: 8,
            },
        ];
        let e = validate_vertex_pipeline_args(ok_extent, 44, &dup).expect_err("location 重复必须被拒");
        assert!(format!("{e}").contains("重复"), "{e}");
    }

    /// 好参数必须**通过**（否则上面 5 条可能是「永远报错」的假绿）。
    #[test]
    fn vertex_pipeline_args_accept_good_input() {
        let extent = vk::Extent2D {
            width: 16,
            height: 16,
        };
        validate_vertex_pipeline_args(extent, 44, &good_attrs())
            .expect("合法参数（stride 44 / offset 0,8 / extent 16×16）必须通过");
    }

    /// **M3b：`R8_UNORM` 纹理的尺寸校验**（错误路径 + 正例）。
    ///
    /// 照 [`validate_vertex_pipeline_args`] 的同一模式：校验是**纯函数**，
    /// 所以无 GPU 的机器也能覆盖负例 —— 而「负例没有测试」等于没有负例。
    #[test]
    fn texture_r8_args_reject_bad_sizes_and_lengths() {
        // 正例：必须通过（否则下面几条可能是「永远报错」的假绿）
        validate_texture_r8_args(4, 2, &[0u8; 8]).expect("4×2 且 8 字节覆盖率数据必须通过");

        // ① 宽为 0
        let e = validate_texture_r8_args(0, 4, &[]).expect_err("宽为 0 必须被拒");
        assert!(format!("{e}").contains("宽高"), "{e}");

        // ② 高为 0（两个方向都试，避免只查了一个字段）
        let e = validate_texture_r8_args(4, 0, &[]).expect_err("高为 0 必须被拒");
        assert!(format!("{e}").contains("宽高"), "{e}");

        // ③ data 太短
        let e = validate_texture_r8_args(4, 2, &[0u8; 7]).expect_err("data 少 1 字节必须被拒");
        let msg = format!("{e}");
        assert!(msg.contains("7") && msg.contains("8"), "错误信息要点出实得与期望：{msg}");

        // ④ data 太长（「静默忽略多余数据」是最容易被放过的写法）
        let e = validate_texture_r8_args(4, 2, &[0u8; 9]).expect_err("data 多 1 字节必须被拒");
        let msg = format!("{e}");
        assert!(msg.contains("9") && msg.contains("8"), "错误信息要点出实得与期望：{msg}");

        // ⑤ 1×1 的合法最小纹理（边界：w*h == 1 不该被上面的比较写错成 0）
        validate_texture_r8_args(1, 1, &[255]).expect("1×1 覆盖率纹理必须通过");
    }

    /// **M3b：采样器参数必须是「最近邻 + ClampToEdge + 无 mipmap」**。
    ///
    /// 抽成纯函数并在无 GPU 下单测的理由：这三条不是「随手选的默认值」，而是
    /// **与 CPU 参考逐像素对齐的前提**（`null.rs::draw_text_real` 是点采样，
    /// 且只按图集内坐标取纹素）—— 详见 [`sampler_create_info`] 的文档。
    #[test]
    fn sampler_params_are_nearest_clamp_no_mip() {
        let info = sampler_create_info();
        assert_eq!(info.mag_filter, vk::VK_FILTER_NEAREST, "放大必须最近邻");
        assert_eq!(info.min_filter, vk::VK_FILTER_NEAREST, "缩小必须最近邻");
        assert_eq!(info.mipmap_mode, vk::VK_SAMPLER_MIPMAP_MODE_NEAREST);
        assert_eq!(info.address_mode_u, vk::VK_SAMPLER_ADDRESS_MODE_CLAMP_TO_EDGE);
        assert_eq!(info.address_mode_v, vk::VK_SAMPLER_ADDRESS_MODE_CLAMP_TO_EDGE);
        assert_eq!(info.address_mode_w, vk::VK_SAMPLER_ADDRESS_MODE_CLAMP_TO_EDGE);
        assert_eq!(info.anisotropy_enable, vk::VK_FALSE, "各向异性会做额外采样");
        assert_eq!(info.compare_enable, vk::VK_FALSE, "不是阴影采样器");
        assert_eq!(info.unnormalized_coordinates, vk::VK_FALSE, "uv 是归一化坐标");
        // 无 mipmap ⇒ LOD 区间必须钉在 0（min==max==0 才会只用第 0 层）
        assert_eq!(info.min_lod, 0.0, "无 mipmap ⇒ minLod 必须是 0");
        assert_eq!(info.max_lod, 0.0, "无 mipmap ⇒ maxLod 必须是 0");
        // sType 写错是「驱动读垃圾」的经典来源，必须对上
        assert_eq!(info.s_type, vk::VK_STRUCTURE_TYPE_SAMPLER_CREATE_INFO);
    }
}
