//! **窗口出图链**（M2b）：把「实例 + surface + 设备 + 交换链 + 渲染通道 + 管线 + 同步 + 呈现」
//! 装成一条能跑的链。
//!
//! ## 一帧的完整流程
//!
//! ```text
//!  ① 等这一槽的 in-flight 栅栏（上一帧真的做完了才复用它的命令缓冲/信号量）
//!  ② vkAcquireNextImageKHR(image_available[slot])
//!      ├ VK_SUCCESS            → 拿到图像索引
//!      ├ VK_SUBOPTIMAL_KHR     → 索引同样有效：照常画+呈现，然后上报 OutOfDate（要重建）
//!      ├ VK_ERROR_OUT_OF_DATE_KHR → 直接返回 FrameOutcome::OutOfDate（**不 reset 栅栏**）
//!      └ 其它（TIMEOUT/NOT_READY/…）→ 返回 Err（**绝不当成功**）
//!  ③ 只有确定要提交时才 reset 栅栏（否则②的失败路径会留下永不信号的栅栏 ⇒ 下一帧死等）
//!  ④ 录制：beginRenderPass(CLEAR=调用方给的颜色) → bindPipeline → setViewport/Scissor → draw → end
//!     （若开着回读：再用两次屏障把图像复制到主机可见缓冲，**复制必须在 present 之前**）
//!  ⑤ vkQueueSubmit（等 image_available，信号 render_finished，挂 in-flight 栅栏）
//!  ⑥ vkQueuePresentKHR（等 render_finished）
//! ```
//!
//! ## 同步对象为什么这么配
//!
//! - `image_available`：**每帧槽一个**。复用它之前一定等过该槽的栅栏 ⇒ 上一次的等待已经结束。
//! - `render_finished`：**每张交换链图像一个**。呈现引擎等它，只有重新 acquire 到同一张图像
//!   才说明呈现引擎放开了它 —— 这是「不能用一个信号量连续呈现两次」的标准解法。
//! - in-flight 栅栏：**每帧槽一个**，创建时就是 signaled（第一帧的 wait 立即返回）。
//!
//! ## 回读（M2b 补强）
//!
//! [`WindowedRenderer::read_back_last_frame`] 能把**呈现出去的那一帧**读成 RGBA8 像素，
//! 于是「窗口里显示的确实是要求的颜色」有了**像素级**证据（而不只是「呈现了 N 帧」）。
//! 实现在 present **之前**把图像 `vkCmdCopyImageToBuffer` 进一块 `HOST_VISIBLE|HOST_COHERENT`
//! 缓冲（present 之后图像所有权归呈现引擎，不能再碰），布局
//! `PRESENT_SRC_KHR ⇄ TRANSFER_SRC_OPTIMAL` 往返后交还 `PRESENT_SRC_KHR`。
//!
//! ## 本轮的诚实边界（M3 才做）
//!
//! 窗口里显示的是**清屏色 + 一个三角形**（M2a 已验证的几何），**不是**界面：
//! 把 `DrawList`（含真实字形）送上 GPU 属于 M3。同理未做：mailbox 呈现模式、
//! 独占全屏、HDR/色彩空间选择、帧率限制、深度/多重采样。

use std::ffi::c_void;
use std::ptr;

use deer_gpu::{AdapterInfo, Color, Extent, GpuError, GpuResult, RawWindowHandle};

use crate::device::{vk_result_name, Pipeline, PipelineLayout, RenderPass, ShaderModule, VkDevice};
use crate::ffi;
use crate::ffi_dev as vk;
use crate::spirv;
use crate::surface::{self, Surface};
use crate::swapchain::{
    readback_row_pitch, reorder_to_rgba8, Acquire, Present, Semaphore, Swapchain,
};

/// `VkFenceCreateFlagBits::VK_FENCE_CREATE_SIGNALED_BIT`
/// （`ffi_dev.rs` 里没有它 —— M2a 的栅栏都是「不预设 signaled ⇒ 必须真的等 GPU」）。
/// 这里需要「初始就 signaled」，否则第一帧的 `wait` 会白等一个超时。
const VK_FENCE_CREATE_SIGNALED_BIT: u32 = 0x0000_0001;

/// 同时在飞的帧数（也是每帧槽命令缓冲/信号量/栅栏的数量）。
///
/// 取 2 的理由：1 会让 CPU 每帧都等 GPU（吞吐掉一半），3 以上对 GUI 没有额外收益
/// 却要多一份命令缓冲与延迟。
pub const FRAMES_IN_FLIGHT: usize = 2;

/// 等 in-flight 栅栏的上限。有限超时的理由同 [`crate::swapchain::DEFAULT_ACQUIRE_TIMEOUT_NS`]：
/// 驱动出问题时**宁可失败也不要永久挂住**（一个挂住的 GUI 进程比一个报错的更难查）。
const FENCE_TIMEOUT_NS: u64 = 5_000_000_000;

/// 三角形的 NDC 顶点（M2a 已验证可画出像素的那组）。
const TRIANGLE_NDC: [[f32; 2]; 3] = [[-0.6, -0.55], [0.6, -0.55], [0.0, 0.6]];
/// 三角形颜色：与默认清屏色明显不同，肉眼一眼能看出「GPU 真的画了」。
const TRIANGLE_COLOR: [f32; 4] = [0.35, 0.62, 1.0, 1.0];

/// 一帧的结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameOutcome {
    /// 这一帧已提交呈现。
    Presented,
    /// 交换链已过期/不再最优 ⇒ 调用方 `resize` 后重试。
    OutOfDate,
}

/// HAL 颜色（0–255 + alpha 0.0–1.0）→ Vulkan 的 `VkClearColorValue.float32`。
///
/// **纯函数**且**不做任何"美化"**（不 clamp、不做 sRGB 变换、不替换通道顺序）——
/// 「传进去的颜色就是清屏色」这条约定必须能被单测逐位钉住，否则示例里那句
/// 「窗口里就是这个颜色」的断言就站不住。
pub fn clear_color_value(color: Color) -> [f32; 4] {
    [
        color.r as f32 / 255.0,
        color.g as f32 / 255.0,
        color.b as f32 / 255.0,
        color.a,
    ]
}

/// 线性通道值（0–255）→ **sRGB 附件里实际写入的字节**。
///
/// ## 为什么需要它（实测纠正了一个想当然的假设）
///
/// 交换链格式是 `B8G8R8A8_SRGB`（本机就是），而 Vulkan 规定：写入 sRGB 附件的
/// **颜色值在线性空间**，驱动负责做 sRGB 编码。所以：
///
/// ```text
///   clear = rgb(0x10, 0x14, 0x24)   ← 线性
///   ⇒ 回读到的字节 = sRGB 编码后的 [0x47, 0x4F, 0x69] = [71, 79, 105]
///   （不是「直通」的 [16, 20, 36] —— 第一版断言按直通写，实测假红）
/// ```
///
/// 这条纯函数就是那个编码公式（sRGB 传输函数），用来给回读断言提供**正确参考值**。
/// 它同时是一份证据：本机 Intel 与 NVIDIA 驱动的实测字节都与它逐位一致。
///
/// 想要「逐字节直通」的调用方只能用**线性格式**（`*_UNORM`）的交换链 —— 那是能力选择问题，
/// 不是回读问题。
pub fn srgb_encoded_byte(linear: u8) -> u8 {
    let x = linear as f64 / 255.0;
    // sRGB EOTF 的逆（线性 → 编码），阈值 0.0031308 是规范给的分段点
    let y = if x <= 0.003_130_8 {
        12.92 * x
    } else {
        1.055 * x.powf(1.0 / 2.4) - 0.055
    };
    (y * 255.0).round().clamp(0.0, 255.0) as u8
}

/// 帧缓冲（私有 RAII：`offscreen.rs` 的 `Framebuffer` 字段是私有的，无法在外面构造）。
struct OwnedFramebuffer {
    handle: vk::FramebufferHandle,
    device: vk::DeviceHandle,
    destroy: vk::PfnDestroyFramebuffer,
}

impl OwnedFramebuffer {
    fn handle(&self) -> vk::FramebufferHandle {
        self.handle
    }
}

impl Drop for OwnedFramebuffer {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            // SAFETY: 句柄由本结构创建且未销毁；设备由 `WindowedRenderer` 保证比它活得久
            // （字段声明顺序：framebuffers 在 device 之前）。
            unsafe { (self.destroy)(self.device, self.handle, ptr::null()) };
            self.handle = ptr::null_mut();
        }
    }
}

/// 命令池（私有 RAII）。命令缓冲由池统一回收，不单独 `vkFreeCommandBuffers`。
struct OwnedCommandPool {
    handle: vk::CommandPoolHandle,
    device: vk::DeviceHandle,
    destroy: vk::PfnDestroyCommandPool,
}

impl OwnedCommandPool {
    fn handle(&self) -> vk::CommandPoolHandle {
        self.handle
    }
}

impl Drop for OwnedCommandPool {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            // SAFETY: 同上（command_pool 声明在 device 之前）。
            unsafe { (self.destroy)(self.device, self.handle, ptr::null()) };
            self.handle = ptr::null_mut();
        }
    }
}

/// 栅栏（私有 RAII；`offscreen::Fence` 同样无法从外部构造）。
struct OwnedFence {
    handle: vk::FenceHandle,
    device: vk::DeviceHandle,
    wait: vk::PfnWaitForFences,
    reset: vk::PfnResetFences,
    destroy: vk::PfnDestroyFence,
}

impl OwnedFence {
    /// 创建一个**初始就是 signaled** 的栅栏（第一帧的 `wait` 才能立即返回）。
    fn create(device: &VkDevice) -> GpuResult<OwnedFence> {
        let fns = device.fns();
        let info = vk::FenceCreateInfo {
            s_type: vk::VK_STRUCTURE_TYPE_FENCE_CREATE_INFO,
            p_next: ptr::null(),
            flags: VK_FENCE_CREATE_SIGNALED_BIT,
        };
        let mut handle: vk::FenceHandle = ptr::null_mut();
        // SAFETY: `info` 在栈上存活；`handle` 是可写输出；设备来自 `&VkDevice`（存活）。
        let rc = unsafe { (fns.create_fence)(device.handle(), &info, ptr::null(), &mut handle) };
        if rc != ffi::VK_SUCCESS {
            return Err(GpuError::Driver {
                code: rc,
                message: format!("vkCreateFence 失败：{}", vk_result_name(rc)),
            });
        }
        Ok(OwnedFence {
            handle,
            device: device.handle(),
            wait: fns.wait_for_fences,
            reset: fns.reset_fences,
            destroy: fns.destroy_fence,
        })
    }

    fn handle(&self) -> vk::FenceHandle {
        self.handle
    }

    /// 等栅栏被信号（**有限超时**：超时算失败，不算成功）。
    fn wait(&self, timeout_ns: u64) -> GpuResult<()> {
        // SAFETY: `&self.handle` 指向本结构的句柄且调用期间存活；设备仍有效。
        let rc = unsafe { (self.wait)(self.device, 1, &self.handle, vk::VK_TRUE, timeout_ns) };
        if rc != ffi::VK_SUCCESS {
            return Err(GpuError::Driver {
                code: rc,
                message: format!(
                    "vkWaitForFences 失败：{}（超时 = GPU 没在预期时间内做完，不是「没关系」）",
                    vk_result_name(rc)
                ),
            });
        }
        Ok(())
    }

    fn reset(&self) -> GpuResult<()> {
        // SAFETY: 句柄有效且此刻没有等待者（本实现每帧槽串行使用）。
        let rc = unsafe { (self.reset)(self.device, 1, &self.handle) };
        if rc != ffi::VK_SUCCESS {
            return Err(GpuError::Driver {
                code: rc,
                message: format!("vkResetFences 失败：{}", vk_result_name(rc)),
            });
        }
        Ok(())
    }
}

impl Drop for OwnedFence {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            // SAFETY: 句柄由本结构创建且未销毁；设备比它活得久（声明顺序）。
            unsafe { (self.destroy)(self.device, self.handle, ptr::null()) };
            self.handle = ptr::null_mut();
        }
    }
}

/// 一块缓冲（回读用的暂存目标）。私有 RAII，理由同 `OwnedFramebuffer`。
struct OwnedBuffer {
    handle: vk::BufferHandle,
    device: vk::DeviceHandle,
    destroy: vk::PfnDestroyBuffer,
}

impl OwnedBuffer {
    fn handle(&self) -> vk::BufferHandle {
        self.handle
    }
}

impl Drop for OwnedBuffer {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            // SAFETY: 句柄由本结构创建且未销毁；设备比它活得久（声明顺序）。
            unsafe { (self.destroy)(self.device, self.handle, ptr::null()) };
            self.handle = ptr::null_mut();
        }
    }
}

/// 一块设备内存（回读缓冲绑定的那块，HOST_VISIBLE | HOST_COHERENT）。
struct OwnedMemory {
    handle: vk::DeviceMemoryHandle,
    device: vk::DeviceHandle,
    free: vk::PfnFreeMemory,
}

impl OwnedMemory {
    fn handle(&self) -> vk::DeviceMemoryHandle {
        self.handle
    }
}

impl Drop for OwnedMemory {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            // SAFETY: 句柄由本结构分配且未释放；设备比它活得久（声明顺序）。
            // 缓冲先析构（字段声明顺序：buffer 在 memory 之前）；本渲染器在销毁前已
            // `vkDeviceWaitIdle`，不会有仍在引用这块内存的提交。
            unsafe { (self.free)(self.device, self.handle, ptr::null()) };
            self.handle = ptr::null_mut();
        }
    }
}

/// 呈现帧回读的暂存目标：一块 `HOST_VISIBLE | HOST_COHERENT` 缓冲，尺寸 = 宽×高×4。
///
/// 每帧在 **present 之前**把交换链图像 `vkCmdCopyImageToBuffer` 进来；`read_back_last_frame`
/// 等那一帧的栅栏后映射读回。为什么不做在 present 之后：present 一旦提交，
/// 图像所有权就归呈现引擎（再次碰它是未定义行为），所以复制必须在 present 之前。
struct ReadbackTarget {
    buffer: OwnedBuffer,
    memory: OwnedMemory,
    /// 缓冲字节数 = `readback_row_pitch(w) * h`（用来校验尺寸没算错）
    size: u64,
}

/// 一条完整的窗口出图链。
///
/// ## 字段顺序 = 析构顺序（**不要重排**）
///
/// Rust 按**声明顺序**析构字段，而 Vulkan 要求按依赖倒序销毁。所以：
/// `帧缓冲 → 命令池/管线/渲染通道/着色器 → 信号量/栅栏/回读缓冲 → 交换链 → 设备 → surface → 实例`。
/// 三个关键点：
/// 1. 帧缓冲引用交换链的图像视图 ⇒ 必须比交换链先销毁；
/// 2. `VkDevice` 必须先于 `Surface`/`Instance`（它借用过那个实例）；
/// 3. `Surface` 必须先于 `Instance`（`vkDestroySurfaceKHR` 需要实例还活着）。
pub struct WindowedRenderer {
    framebuffers: Vec<OwnedFramebuffer>,
    /// 命令缓冲从它分配 ⇒ 只为「比命令缓冲/设备活得久」而持有（`Drop` 会回收命令缓冲）。
    #[allow(dead_code)]
    command_pool: OwnedCommandPool,
    command_buffers: Vec<vk::CommandBufferHandle>,
    pipeline: Pipeline,
    pipeline_layout: PipelineLayout,
    render_pass: RenderPass,
    vs: ShaderModule,
    fs: ShaderModule,
    image_available: Vec<Semaphore>,
    render_finished: Vec<Semaphore>,
    in_flight: Vec<OwnedFence>,
    /// 回读暂存目标（`None` = 关闭了回读）。
    readback: Option<ReadbackTarget>,
    swapchain: Swapchain,
    device: VkDevice,
    surface: Surface,
    instance: ffi::Instance,
    clear: Color,
    extent: Extent,
    frame: usize,
    frames_presented: u64,
    suboptimal_frames: u64,
    /// 是否在每帧 present 前记录「图像 → 回读缓冲」的复制。默认 **开**：
    /// 冻死的契约是「先 `render_and_present()` 再 `read_back_last_frame()`」，
    /// 默认关会让那个 API 必然报错。不想要这份开销的调用方用 `set_readback_enabled(false)`。
    readback_enabled: bool,
    /// 最近一次 `render_and_present()` 是否真的记录了复制（防止读回**上一帧的陈旧数据**）。
    last_frame_readback: bool,
    /// 最近一次呈现用的帧槽（回读要等它的栅栏）。
    last_presented_slot: usize,
}

impl WindowedRenderer {
    /// 一条完整链：实例（含 surface 扩展）→ device（图形 + 呈现队列）→ surface → 交换链
    /// → 渲染通道（格式 = 交换链格式）→ 三角形管线（复用 M2a 的 SPIR-V）→ 每帧命令缓冲 + 同步。
    pub fn new(
        adapter_index: usize,
        window: RawWindowHandle,
        want: Extent,
        clear: Color,
    ) -> GpuResult<WindowedRenderer> {
        // ⓪ 回读开关：默认开（见字段文档）。开启时每帧多一次全屏 copy（宽×高×4 字节），
        //    换来「呈现出去的到底是哪张图」可以被逐字节验证。
        let readback_enabled = true;

        // ① 实例：**先用 extension_available 查、缺了就明确报错**（不静默退化成「没有 surface」）
        let validation = ffi::Instance::validation_from_env();
        let platform_ext = Surface::platform_extension(window.platform);
        let instance = ffi::Instance::create_with_extensions(
            validation,
            &[surface::SURFACE_EXTENSION, platform_ext],
        )?;
        if instance.validation_enabled() {
            eprintln!("[deer-vk] 窗口路径已启用 VK_LAYER_KHRONOS_validation（消息打到 stderr）");
        }

        // ② surface（HWND → VkSurfaceKHR）
        let surface = Surface::create(&instance, window)?;

        // ③ 设备：**借用 surface 的实例**拿「图形 + 呈现」队列族（跨实例用 surface 是校验层会抓的错误）
        let device = VkDevice::open_with_present(adapter_index, &surface)?;
        let pd = device.physical_device();

        // ④ 交换链（确定性配置）
        let cfg = Swapchain::choose_config(&surface, pd, want)?;
        let swapchain = Swapchain::create(&device, &surface, cfg, None)?;
        let extent = swapchain.extent();

        // ⑤ 渲染通道：附件最终布局 = PRESENT_SRC_KHR（交换链图像必须处于这个布局才能呈现）；
        //    每帧 CLEAR ⇒ 清屏色就是调用方给的颜色。
        let render_pass = device.create_render_pass(
            swapchain.format(),
            vk::VK_ATTACHMENT_LOAD_OP_CLEAR,
            vk::VK_IMAGE_LAYOUT_PRESENT_SRC_KHR,
        )?;
        let pipeline_layout = device.create_pipeline_layout(None)?;
        // 复用 M2a 真机验证过的两个着色器（自研 SPIR-V 汇编器产出）
        let vs = device.create_shader_module(&spirv::vertex_shader_triangle(TRIANGLE_NDC))?;
        let fs = device.create_shader_module(&spirv::fragment_shader_solid(TRIANGLE_COLOR))?;
        // 动态 viewport/scissor 版管线（`create_graphics_pipeline`）：尺寸变化时不用重建管线
        let pipeline = device.create_graphics_pipeline(&vs, &fs, &pipeline_layout, &render_pass)?;

        // ⑥ 每张交换链图像一个帧缓冲
        let framebuffers =
            create_framebuffers(&device, &render_pass, swapchain.image_views(), extent)?;

        // ⑦ 命令池 + 每帧槽一个命令缓冲
        let command_pool = create_command_pool(&device)?;
        let command_buffers =
            allocate_command_buffers(&device, command_pool.handle(), FRAMES_IN_FLIGHT)?;

        // ⑧ 同步对象
        let in_flight = (0..FRAMES_IN_FLIGHT)
            .map(|_| OwnedFence::create(&device))
            .collect::<GpuResult<Vec<_>>>()?;
        let image_available = (0..FRAMES_IN_FLIGHT)
            .map(|_| Semaphore::create(&device))
            .collect::<GpuResult<Vec<_>>>()?;
        let render_finished = (0..swapchain.image_count())
            .map(|_| Semaphore::create(&device))
            .collect::<GpuResult<Vec<_>>>()?;

        // ⑨ 回读暂存目标（默认开启：契约要求 `render_and_present` 之后能直接回读）
        let readback = if readback_enabled {
            Some(create_readback_target(&device, extent)?)
        } else {
            None
        };

        Ok(WindowedRenderer {
            framebuffers,
            command_pool,
            command_buffers,
            pipeline,
            pipeline_layout,
            render_pass,
            vs,
            fs,
            image_available,
            render_finished,
            in_flight,
            readback,
            swapchain,
            device,
            surface,
            instance,
            clear,
            extent,
            frame: 0,
            frames_presented: 0,
            suboptimal_frames: 0,
            readback_enabled,
            last_frame_readback: false,
            last_presented_slot: 0,
        })
    }

    pub fn adapter(&self) -> &AdapterInfo {
        self.device.adapter()
    }

    pub fn extent(&self) -> Extent {
        self.extent
    }

    pub fn format(&self) -> i32 {
        self.swapchain.format()
    }

    pub fn present_mode(&self) -> i32 {
        self.swapchain.present_mode()
    }

    pub fn image_count(&self) -> u32 {
        self.swapchain.image_count()
    }

    /// 已成功提交呈现的帧数（**只统计真的调了 `vkQueuePresentKHR` 且返回成功/不最优的帧**）。
    pub fn frames_presented(&self) -> u64 {
        self.frames_presented
    }

    /// 交换链不再最优（`VK_SUBOPTIMAL_KHR`）的帧数 —— 诊断用。
    ///
    /// 这类帧**已经呈现了**，但调用方应当重建交换链，所以它们被上报为
    /// [`FrameOutcome::OutOfDate`]。计数放这里，是为了不把「不最优」伪装成「正常」。
    pub fn suboptimal_frames(&self) -> u64 {
        self.suboptimal_frames
    }

    /// 本渲染器实例实际启用的实例扩展（诊断/验证用：证明 surface 扩展真的启用了）。
    pub fn instance_extensions(&self) -> &[String] {
        self.instance.enabled_extensions()
    }

    /// 开启/关闭**呈现帧回读**（每帧在 present 前多录一次「图像 → 暂存缓冲」的复制）。
    ///
    /// 默认**开**（冻死契约是「`render_and_present()` 之后就能 `read_back_last_frame()`」）。
    /// 关掉可以省掉每帧一次全屏 copy（约 `宽×高×4` 字节的 GPU 带宽 + 一次 barrier 往返），
    /// 代价是 `read_back_last_frame()` 会明确报错（**不会**返回陈旧数据冒充）。
    pub fn set_readback_enabled(&mut self, enabled: bool) -> GpuResult<()> {
        if enabled && self.readback.is_none() {
            self.readback = Some(create_readback_target(&self.device, self.extent)?);
        }
        self.readback_enabled = enabled;
        if !enabled {
            // 关掉之后「最近一帧有没有复制过」必须立即失效，防止读到旧内容
            self.last_frame_readback = false;
        }
        Ok(())
    }

    /// 回读开关当前状态。
    pub fn readback_enabled(&self) -> bool {
        self.readback_enabled
    }

    /// 本渲染器是否**具备**回读能力（暂存缓冲已建好）。
    pub fn readback_available(&self) -> bool {
        self.readback.is_some()
    }

    /// 把**刚刚呈现出去的那一帧**回读为 **RGBA8**（长度 = `宽 × 高 × 4`）。
    ///
    /// ## 语义与契约
    ///
    /// - 返回的是「最近一次 `render_and_present()` 所渲染的那张交换链图像」的像素；
    /// - **通道顺序是 R,G,B,A**：交换链若是 `B8G8R8A8_*`（本机就是），这里会换 R/B
    ///   （见 [`reorder_to_rgba8`]）—— 这正是「清屏色 = 已知值」能当判据的原因；
    /// - 返回值是**图像里的字节**（sRGB 格式下即 sRGB 编码值），不做线性化，
    ///   这样它才和窗口上看到的/截图工具抓到的逐字节一致；
    /// - **会强制一次 GPU→CPU 同步**：等最近那一帧的 in-flight 栅栏（有限超时 5 s），
    ///   所以它适合做验收/截图，**不适合每帧都调**（那会毁掉流水线并行）；
    /// - 必须在 `render_and_present()` **之后**、且中间没有再 `resize` 过；
    ///   回读开关关着（或 resize 过后）会明确报错，**绝不用陈旧数据冒充**。
    pub fn read_back_last_frame(&mut self) -> GpuResult<Vec<u8>> {
        let Some(rb) = self.readback.as_ref() else {
            return Err(GpuError::Unsupported(
                "本渲染器没有回读暂存缓冲（回读被关闭过）⇒ 先 `set_readback_enabled(true)`"
                    .to_string(),
            ));
        };
        if !self.last_frame_readback {
            return Err(GpuError::Unsupported(
                "最近一次 `render_and_present()` 没有记录回读复制（回读开关是关的，或之后 resize 过）\
                 ⇒ 请在 `render_and_present()` 之前调用 `set_readback_enabled(true)`"
                    .to_string(),
            ));
        }
        // 尺寸自检：算错行 stride 会让整幅图斜切，而「四角对、中间错」很难一眼看出
        let want = readback_row_pitch(self.extent.width) as u64 * self.extent.height as u64;
        if rb.size != want {
            return Err(GpuError::Driver {
                code: -1,
                message: format!(
                    "回读缓冲 {} 字节与当前交换链 {}×{} 不符（应为 {}）⇒ 请 resize 重建",
                    rb.size, self.extent.width, self.extent.height, want
                ),
            });
        }

        // 等「最后那一帧」的栅栏：队列内提交是有序的 ⇒ 它 signaled 就意味着
        // 那一帧的 image→buffer 复制已经完成，且它之后的写不可能存在。
        // 用有限超时而不是无限等 / `vkDeviceWaitIdle`（后者没有超时参数，可能永久挂住）。
        self.in_flight[self.last_presented_slot].wait(FENCE_TIMEOUT_NS)?;

        let fns = *self.device.fns();
        let mut mapped: *mut c_void = ptr::null_mut();
        // SAFETY: 内存是 HOST_VISIBLE | HOST_COHERENT（建缓冲时就是这么挑的类型）；
        // `WHOLE_SIZE` 表示映射整块；`mapped` 是可写输出。
        let rc = unsafe {
            (fns.map_memory)(
                self.device.handle(),
                rb.memory.handle(),
                0,
                vk::WHOLE_SIZE,
                0,
                &mut mapped,
            )
        };
        if rc != ffi::VK_SUCCESS {
            return Err(GpuError::Driver {
                code: rc,
                message: format!("vkMapMemory（回读缓冲）失败：{}", vk_result_name(rc)),
            });
        }
        if mapped.is_null() {
            return Err(GpuError::Driver {
                code: rc,
                message: "vkMapMemory 返回空指针".to_string(),
            });
        }
        // SAFETY: 映射了 `rb.size` 字节（WHOLE_SIZE）；拷贝出来即与映射解耦。
        let raw =
            unsafe { std::slice::from_raw_parts(mapped as *const u8, rb.size as usize).to_vec() };
        // SAFETY: 与上面的 map 配对（本函数内唯一一次 map/unmap）。
        unsafe { (fns.unmap_memory)(self.device.handle(), rb.memory.handle()) };

        reorder_to_rgba8(&raw, self.swapchain.format())
    }

    /// 尺寸变化：重建交换链（以及依赖它的帧缓冲/同步对象；格式变了还会重建渲染通道与管线）。
    pub fn resize(&mut self, extent: Extent) -> GpuResult<()> {
        let want = Extent {
            width: extent.width.max(1),
            height: extent.height.max(1),
        };
        // 换交换链前必须等 GPU 空闲：旧图像可能还在被读/被呈现
        self.device.wait_idle()?;

        let old_format = self.swapchain.format();
        let pd = self.device.physical_device();
        let cfg = Swapchain::choose_config(&self.surface, pd, want)?;

        // 先把「新交换链需要的同步对象」备好：尽量让失败发生在旧状态被破坏之前
        let mut new_render_finished = (0..cfg.image_count)
            .map(|_| Semaphore::create(&self.device))
            .collect::<GpuResult<Vec<_>>>()?;
        // 回读缓冲的尺寸跟着交换链走 ⇒ 也提前备好（同样是为了「失败不破坏旧状态」）
        let new_readback = if self.readback_enabled {
            Some(create_readback_target(&self.device, cfg.extent)?)
        } else {
            None
        };
        // `oldSwapchain` 复用：成功之后旧交换链即被 retire，由 `self.swapchain = new` 析构
        let new_swapchain =
            Swapchain::create(&self.device, &self.surface, cfg, Some(&self.swapchain))?;
        // 驱动可以给比 `minImageCount` 更多的图像 ⇒ 补齐信号量
        while (new_render_finished.len() as u32) < new_swapchain.image_count() {
            new_render_finished.push(Semaphore::create(&self.device)?);
        }
        // 交换链实际尺寸可能与请求的不同（`currentExtent` 优先级）；缓冲按实际尺寸再核一次
        if let Some(rb) = &new_readback {
            let want = readback_row_pitch(new_swapchain.extent().width) as u64
                * new_swapchain.extent().height as u64;
            if rb.size != want {
                return Err(GpuError::Driver {
                    code: -1,
                    message: format!(
                        "回读缓冲尺寸 {} 与新交换链 {}×{} 不符（驱动改了尺寸？）",
                        rb.size,
                        new_swapchain.extent().width,
                        new_swapchain.extent().height
                    ),
                });
            }
        }

        // 帧缓冲引用旧交换链的图像视图 ⇒ 必须在旧交换链销毁**之前**销毁
        self.framebuffers.clear();
        self.swapchain = new_swapchain;
        self.render_finished = new_render_finished;
        self.readback = new_readback;
        // 尺寸变了，上次回读的数据不再对应当前交换链 ⇒ 明确失效（不许拿陈旧数据冒充）
        self.last_frame_readback = false;

        // 格式变了就必须重建渲染通道与管线（管线里写着附件格式与渲染通道句柄）。
        // 注意顺序：先换管线（旧管线引用旧渲染通道），再换渲染通道。
        if self.swapchain.format() != old_format {
            let new_pass = self.device.create_render_pass(
                self.swapchain.format(),
                vk::VK_ATTACHMENT_LOAD_OP_CLEAR,
                vk::VK_IMAGE_LAYOUT_PRESENT_SRC_KHR,
            )?;
            let new_pipeline = self.device.create_graphics_pipeline(
                &self.vs,
                &self.fs,
                &self.pipeline_layout,
                &new_pass,
            )?;
            self.pipeline = new_pipeline;
            self.render_pass = new_pass;
        }
        // 动态 viewport/scissor ⇒ 尺寸变化不需要重建管线

        let framebuffers = create_framebuffers(
            &self.device,
            &self.render_pass,
            self.swapchain.image_views(),
            self.swapchain.extent(),
        )?;
        self.framebuffers = framebuffers;
        self.extent = self.swapchain.extent();
        // 尺寸变了，帧槽计数归零（不是必须，但让「第一帧」总是从槽 0 开始，便于复现）
        self.frame = 0;
        Ok(())
    }

    /// 画一帧并呈现。
    ///
    /// 交换链过期/不再最优时返回 [`FrameOutcome::OutOfDate`]（**不 panic、不假装成功**），
    /// 调用方 `resize` 后重试。
    pub fn render_and_present(&mut self) -> GpuResult<FrameOutcome> {
        if self.framebuffers.len() != self.swapchain.image_count() as usize {
            return Err(GpuError::Unsupported(
                "帧缓冲与交换链图像数不一致（上一次 resize 失败过？）⇒ 请再调一次 resize"
                    .to_string(),
            ));
        }
        let slot = self.frame % FRAMES_IN_FLIGHT;

        // ① 等这一槽上一帧做完
        self.in_flight[slot].wait(FENCE_TIMEOUT_NS)?;

        // ② 取图像
        let (image_index, acquire_suboptimal) =
            match self.swapchain.acquire(self.image_available[slot].handle())? {
                Acquire::Image(i) => (i, false),
                Acquire::Suboptimal => {
                    // SUBOPTIMAL 时驱动仍然写回了有效索引（规范），拿它去 present 才能释放这张图像
                    let i = self.swapchain.last_acquired_image().ok_or_else(|| {
                        GpuError::Driver {
                            code: ffi::VK_SUBOPTIMAL_KHR,
                            message:
                                "vkAcquireNextImageKHR 返回 SUBOPTIMAL 但没记下图像索引（内部不一致）"
                                    .to_string(),
                        }
                    })?;
                    (i, true)
                }
                Acquire::OutOfDate => {
                    // **不 reset 栅栏**：这一帧不提交，栅栏保持 signaled，下一帧的 wait 立即返回
                    self.frame = (slot + 1) % FRAMES_IN_FLIGHT;
                    return Ok(FrameOutcome::OutOfDate);
                }
            };
        if image_index >= self.swapchain.image_count()
            || image_index as usize >= self.framebuffers.len()
        {
            return Err(GpuError::Driver {
                code: -1,
                message: format!(
                    "驱动给出的图像索引 {image_index} 超出范围（交换链 {} 张 / 帧缓冲 {} 个）",
                    self.swapchain.image_count(),
                    self.framebuffers.len()
                ),
            });
        }

        // ③ 确定要提交了，才 reset 栅栏
        self.in_flight[slot].reset()?;

        // ④ 录制
        self.record(slot, image_index)?;

        // ⑤ 提交
        let fns = *self.device.fns();
        let image_available = self.image_available[slot].handle();
        let render_finished = self.render_finished[image_index as usize].handle();
        let cmd = self.command_buffers[slot];
        let fence = self.in_flight[slot].handle();
        let wait_semaphores = [image_available];
        let wait_stages = [vk::VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT];
        let command_buffers = [cmd];
        let signal_semaphores = [render_finished];
        let submit = vk::SubmitInfo {
            s_type: vk::VK_STRUCTURE_TYPE_SUBMIT_INFO,
            p_next: ptr::null(),
            wait_semaphore_count: 1,
            p_wait_semaphores: wait_semaphores.as_ptr(),
            p_wait_dst_stage_mask: wait_stages.as_ptr(),
            command_buffer_count: 1,
            p_command_buffers: command_buffers.as_ptr(),
            signal_semaphore_count: 1,
            p_signal_semaphores: signal_semaphores.as_ptr(),
        };
        // SAFETY: 队列是设备自己的；三个数组与 `submit` 都在本栈帧存活；
        // 命令缓冲已结束录制；栅栏已 reset 且本帧独占。
        let rc = unsafe { (fns.queue_submit)(self.device.queue(), 1, &submit, fence) };
        if rc != ffi::VK_SUCCESS {
            return Err(GpuError::Driver {
                code: rc,
                message: format!("vkQueueSubmit 失败：{}", vk_result_name(rc)),
            });
        }

        // ⑥ 呈现
        let presented =
            self.swapchain
                .present(self.device.present_queue(), render_finished, image_index)?;
        self.frames_presented += 1;
        self.frame = (slot + 1) % FRAMES_IN_FLIGHT;
        // 回读用：记住这一帧在哪个槽（要等它的栅栏），以及「这一帧确实复制过」。
        self.last_presented_slot = slot;
        self.last_frame_readback = self.readback_enabled && self.readback.is_some();

        match presented {
            Present::Presented if !acquire_suboptimal => Ok(FrameOutcome::Presented),
            // SUBOPTIMAL（acquire 或 present 侧）：**这一帧已呈现**，但交换链已不最优 ⇒
            // 如实上报 OutOfDate 让调用方重建，而不是把它当成功吞掉。
            Present::Presented | Present::Suboptimal => {
                self.suboptimal_frames += 1;
                Ok(FrameOutcome::OutOfDate)
            }
            // 过期：图像已 acquire 但没呈现成功 ⇒ 也必须重建（调用方 resize）
            Present::OutOfDate => Ok(FrameOutcome::OutOfDate),
        }
    }

    /// 等 GPU 空闲（截图、关窗、销毁资源前用）。
    pub fn wait_idle(&mut self) -> GpuResult<()> {
        self.device.wait_idle()
    }

    /// 录制第 `slot` 个命令缓冲：清屏到调用方给的颜色 + 画三角形（+ 可选的回读复制）。
    fn record(&self, slot: usize, image_index: u32) -> GpuResult<()> {
        let fns = *self.device.fns();
        let cmd = self.command_buffers[slot];

        // SAFETY: 命令缓冲由本结构持有；本帧独占该槽，且上一次提交已被栅栏等到完成。
        let rc = unsafe { (fns.reset_command_buffer)(cmd, 0) };
        if rc != ffi::VK_SUCCESS {
            return Err(GpuError::Driver {
                code: rc,
                message: format!("vkResetCommandBuffer 失败：{}", vk_result_name(rc)),
            });
        }
        let begin = vk::CommandBufferBeginInfo {
            s_type: vk::VK_STRUCTURE_TYPE_COMMAND_BUFFER_BEGIN_INFO,
            p_next: ptr::null(),
            flags: 0,
            p_inheritance_info: ptr::null(),
        };
        // SAFETY: `begin` 在栈上存活；句柄有效。
        let rc = unsafe { (fns.begin_command_buffer)(cmd, &begin) };
        if rc != ffi::VK_SUCCESS {
            return Err(GpuError::Driver {
                code: rc,
                message: format!("vkBeginCommandBuffer 失败：{}", vk_result_name(rc)),
            });
        }

        // 清屏色 = 调用方给的颜色（示例会用它做可断言的颜色）。
        // 转换走纯函数 [`clear_color_value`]，这样「传进去什么」可以被单测钉住。
        let clear_value = vk::ClearValue {
            color: vk::ClearColorValue {
                float32: clear_color_value(self.clear),
            },
        };
        let area = vk::Rect2D {
            offset: vk::Offset2D { x: 0, y: 0 },
            extent: vk::Extent2D {
                width: self.extent.width,
                height: self.extent.height,
            },
        };
        let pass_begin = vk::RenderPassBeginInfo {
            s_type: vk::VK_STRUCTURE_TYPE_RENDER_PASS_BEGIN_INFO,
            p_next: ptr::null(),
            render_pass: self.render_pass.handle(),
            framebuffer: self.framebuffers[image_index as usize].handle(),
            render_area: area,
            clear_value_count: 1,
            p_clear_values: &clear_value,
        };
        // SAFETY: 上述结构体都在本栈帧存活；命令缓冲处于录制状态；
        // 管线/渲染通道/帧缓冲都是本结构持有且都还没有销毁。
        unsafe {
            (fns.cmd_begin_render_pass)(cmd, &pass_begin, vk::VK_SUBPASS_CONTENTS_INLINE);
            (fns.cmd_bind_pipeline)(
                cmd,
                vk::VK_PIPELINE_BIND_POINT_GRAPHICS,
                self.pipeline.handle(),
            );
            // 动态 viewport/scissor：管线里没写死尺寸，这里每帧给
            let viewport = vk::Viewport {
                x: 0.0,
                y: 0.0,
                width: self.extent.width as f32,
                height: self.extent.height as f32,
                min_depth: 0.0,
                max_depth: 1.0,
            };
            (fns.cmd_set_viewport)(cmd, 0, 1, &viewport);
            let scissor = vk::Rect2D {
                offset: vk::Offset2D { x: 0, y: 0 },
                extent: vk::Extent2D {
                    width: self.extent.width,
                    height: self.extent.height,
                },
            };
            (fns.cmd_set_scissor)(cmd, 0, 1, &scissor);
            (fns.cmd_draw)(cmd, 3, 1, 0, 0);
            (fns.cmd_end_render_pass)(cmd);
        }

        // —— 回读复制：**必须在 present 之前**，且布局要交还 `PRESENT_SRC_KHR` ——
        //
        // 为什么不在 present 之后做：`vkQueuePresentKHR` 一提交，图像所有权就归呈现引擎，
        // 应用再碰它是未定义行为（校验层也不一定能抓到，但那是真错）。
        //
        // 渲染通道的 `finalLayout` 已经是 `PRESENT_SRC_KHR` ⇒ 这里的 oldLayout 就是它。
        // 复制完再把布局转回 `PRESENT_SRC_KHR`（present 要求图像处于该布局）。
        if self.readback_enabled {
            if let Some(rb) = &self.readback {
                let image = self.swapchain.images()[image_index as usize];
                let to_transfer = vk::ImageMemoryBarrier {
                    s_type: vk::VK_STRUCTURE_TYPE_IMAGE_MEMORY_BARRIER,
                    p_next: ptr::null(),
                    // 渲染通道刚刚写完（finalLayout 转换也是这次写的一部分）
                    src_access_mask: vk::VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT,
                    dst_access_mask: vk::VK_ACCESS_TRANSFER_READ_BIT,
                    old_layout: vk::VK_IMAGE_LAYOUT_PRESENT_SRC_KHR,
                    new_layout: vk::VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL,
                    src_queue_family_index: vk::QUEUE_FAMILY_IGNORED,
                    dst_queue_family_index: vk::QUEUE_FAMILY_IGNORED,
                    image,
                    subresource_range: color_subresource_range(),
                };
                let copy = vk::BufferImageCopy {
                    buffer_offset: 0,
                    // 0 = 与图像宽度一致 ⇒ 紧排、无行尾 padding（与 [`readback_row_pitch`] 一致）
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
                        width: self.extent.width,
                        height: self.extent.height,
                        depth: 1,
                    },
                };
                let back_to_present = vk::ImageMemoryBarrier {
                    s_type: vk::VK_STRUCTURE_TYPE_IMAGE_MEMORY_BARRIER,
                    p_next: ptr::null(),
                    src_access_mask: vk::VK_ACCESS_TRANSFER_READ_BIT,
                    // 呈现引擎的读由 `vkQueuePresentKHR` 等的信号量同步 ⇒ 这里到 BOTTOM 即可
                    dst_access_mask: 0,
                    old_layout: vk::VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL,
                    new_layout: vk::VK_IMAGE_LAYOUT_PRESENT_SRC_KHR,
                    src_queue_family_index: vk::QUEUE_FAMILY_IGNORED,
                    dst_queue_family_index: vk::QUEUE_FAMILY_IGNORED,
                    image,
                    subresource_range: color_subresource_range(),
                };
                // SAFETY: 上述结构体都在本栈帧存活；命令缓冲处于录制状态；
                // 图像/缓冲都由本结构持有且都在同一队列族上（EXCLUSIVE 共享模式）。
                unsafe {
                    (fns.cmd_pipeline_barrier)(
                        cmd,
                        vk::VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT,
                        vk::VK_PIPELINE_STAGE_TRANSFER_BIT,
                        0,
                        0,
                        ptr::null(),
                        0,
                        ptr::null(),
                        1,
                        &to_transfer,
                    );
                    (fns.cmd_copy_image_to_buffer)(
                        cmd,
                        image,
                        vk::VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL,
                        rb.buffer.handle(),
                        1,
                        &copy,
                    );
                    (fns.cmd_pipeline_barrier)(
                        cmd,
                        vk::VK_PIPELINE_STAGE_TRANSFER_BIT,
                        vk::VK_PIPELINE_STAGE_BOTTOM_OF_PIPE_BIT,
                        0,
                        0,
                        ptr::null(),
                        0,
                        ptr::null(),
                        1,
                        &back_to_present,
                    );
                }
            }
        }

        // SAFETY: 命令缓冲处于录制状态。
        let rc = unsafe { (fns.end_command_buffer)(cmd) };
        if rc != ffi::VK_SUCCESS {
            return Err(GpuError::Driver {
                code: rc,
                message: format!("vkEndCommandBuffer 失败：{}", vk_result_name(rc)),
            });
        }
        Ok(())
    }
}

impl Drop for WindowedRenderer {
    fn drop(&mut self) {
        // 先等 GPU 空闲，然后**字段按声明顺序**（依赖倒序）析构。
        // 这里不手动销毁各对象：`Owned*`/`Swapchain`/`Surface`/`Instance` 各自的 Drop
        // 已经负责，而字段顺序保证了先后关系 —— 手动再写一遍反而容易漏。
        if let Err(e) = self.device.wait_idle() {
            eprintln!("[deer-vk] 关闭窗口渲染器时 vkDeviceWaitIdle 失败：{e}（仍按顺序销毁）");
        }
    }
}

/// 交换链图像的完整子资源范围（1 mip / 1 layer / COLOR）。
fn color_subresource_range() -> vk::ImageSubresourceRange {
    vk::ImageSubresourceRange {
        aspect_mask: vk::VK_IMAGE_ASPECT_COLOR_BIT,
        base_mip_level: 0,
        level_count: 1,
        base_array_layer: 0,
        layer_count: 1,
    }
}

/// 建回读暂存缓冲：`HOST_VISIBLE | HOST_COHERENT`（⇒ 不需要显式 flush/invalidate）。
///
/// 尺寸 = [`readback_row_pitch`] × 高（紧排，无行尾 padding）。
fn create_readback_target(device: &VkDevice, extent: Extent) -> GpuResult<ReadbackTarget> {
    let height = extent.height.max(1);
    let size = readback_row_pitch(extent.width) as u64 * height as u64;
    if size == 0 || extent.width == 0 {
        return Err(GpuError::Unsupported(
            "回读缓冲尺寸为 0（交换链宽或高为 0）".to_string(),
        ));
    }
    let fns = device.fns();

    let info = vk::BufferCreateInfo {
        s_type: vk::VK_STRUCTURE_TYPE_BUFFER_CREATE_INFO,
        p_next: ptr::null(),
        flags: 0,
        size,
        usage: vk::VK_BUFFER_USAGE_TRANSFER_DST_BIT,
        sharing_mode: vk::VK_SHARING_MODE_EXCLUSIVE,
        queue_family_index_count: 0,
        p_queue_family_indices: ptr::null(),
    };
    let mut buffer: vk::BufferHandle = ptr::null_mut();
    // SAFETY: `info` 在栈上存活；`buffer` 是可写输出。
    let rc = unsafe { (fns.create_buffer)(device.handle(), &info, ptr::null(), &mut buffer) };
    if rc != ffi::VK_SUCCESS {
        return Err(GpuError::Driver {
            code: rc,
            message: format!(
                "vkCreateBuffer（回读缓冲 {size} 字节）失败：{}",
                vk_result_name(rc)
            ),
        });
    }
    let buffer = OwnedBuffer {
        handle: buffer,
        device: device.handle(),
        destroy: fns.destroy_buffer,
    };

    let mut req = std::mem::MaybeUninit::<vk::MemoryRequirements>::uninit();
    // SAFETY: 该函数完整写入结构体；缓冲刚创建且有效。
    unsafe {
        (fns.get_buffer_memory_requirements)(device.handle(), buffer.handle(), req.as_mut_ptr())
    };
    let req = unsafe { req.assume_init() };
    let memory_type_index = pick_memory_type(
        device.memory_properties(),
        req.memory_type_bits,
        vk::VK_MEMORY_PROPERTY_HOST_VISIBLE_BIT | vk::VK_MEMORY_PROPERTY_HOST_COHERENT_BIT,
    )?;

    let alloc = vk::MemoryAllocateInfo {
        s_type: vk::VK_STRUCTURE_TYPE_MEMORY_ALLOCATE_INFO,
        p_next: ptr::null(),
        allocation_size: req.size,
        memory_type_index,
    };
    let mut memory: vk::DeviceMemoryHandle = ptr::null_mut();
    // SAFETY: `alloc` 在栈上存活；`memory` 是可写输出。
    let rc = unsafe { (fns.allocate_memory)(device.handle(), &alloc, ptr::null(), &mut memory) };
    if rc != ffi::VK_SUCCESS {
        return Err(GpuError::Driver {
            code: rc,
            message: format!(
                "vkAllocateMemory（回读缓冲，类型 {memory_type_index}）失败：{}",
                vk_result_name(rc)
            ),
        });
    }
    let memory = OwnedMemory {
        handle: memory,
        device: device.handle(),
        free: fns.free_memory,
    };
    // SAFETY: 缓冲与内存都是本函数刚创建的对象，兼容性由 memory_type_bits 保证；offset 0 合法。
    let rc =
        unsafe { (fns.bind_buffer_memory)(device.handle(), buffer.handle(), memory.handle(), 0) };
    if rc != ffi::VK_SUCCESS {
        return Err(GpuError::Driver {
            code: rc,
            message: format!("vkBindBufferMemory（回读缓冲）失败：{}", vk_result_name(rc)),
        });
    }

    Ok(ReadbackTarget {
        buffer,
        memory,
        size,
    })
}

/// 从设备的内存类型里挑一个同时满足 `required` 所有位的。
///
/// 与 `offscreen.rs` 的同名逻辑一致（那边是私有的）；属性来自**物理设备**
/// （逻辑设备上拿不到），由 `VkDevice::memory_properties()` 提供。
fn pick_memory_type(
    props: &vk::PhysicalDeviceMemoryProperties,
    type_bits: u32,
    required: u32,
) -> GpuResult<u32> {
    let count = props.memory_type_count.min(32);
    for i in 0..count {
        let m = &props.memory_types[i as usize];
        if type_bits & (1 << i) == 0 {
            continue;
        }
        if m.property_flags & required == required {
            return Ok(i);
        }
    }
    Err(GpuError::Unsupported(format!(
        "找不到满足属性 {required:#x} 的内存类型（缓冲区允许的 type_bits = {type_bits:#x}）"
    )))
}

/// 每张交换链图像建一个帧缓冲。
fn create_framebuffers(
    device: &VkDevice,
    render_pass: &RenderPass,
    views: &[vk::ImageViewHandle],
    extent: Extent,
) -> GpuResult<Vec<OwnedFramebuffer>> {
    let fns = device.fns();
    let mut out = Vec::with_capacity(views.len());
    for (i, view) in views.iter().enumerate() {
        let info = vk::FramebufferCreateInfo {
            s_type: vk::VK_STRUCTURE_TYPE_FRAMEBUFFER_CREATE_INFO,
            p_next: ptr::null(),
            flags: 0,
            render_pass: render_pass.handle(),
            attachment_count: 1,
            p_attachments: view,
            width: extent.width,
            height: extent.height,
            layers: 1,
        };
        let mut handle: vk::FramebufferHandle = ptr::null_mut();
        // SAFETY: `info` 与 `view` 在调用期间存活；`handle` 是可写输出。
        let rc =
            unsafe { (fns.create_framebuffer)(device.handle(), &info, ptr::null(), &mut handle) };
        if rc != ffi::VK_SUCCESS {
            return Err(GpuError::Driver {
                code: rc,
                message: format!(
                    "vkCreateFramebuffer 失败（第 {i} 张图像，{}×{}）：{}",
                    extent.width,
                    extent.height,
                    vk_result_name(rc)
                ),
            });
        }
        out.push(OwnedFramebuffer {
            handle,
            device: device.handle(),
            destroy: fns.destroy_framebuffer,
        });
    }
    Ok(out)
}

/// 建命令池（允许逐缓冲 `vkResetCommandBuffer`）。
fn create_command_pool(device: &VkDevice) -> GpuResult<OwnedCommandPool> {
    let fns = device.fns();
    let info = vk::CommandPoolCreateInfo {
        s_type: vk::VK_STRUCTURE_TYPE_COMMAND_POOL_CREATE_INFO,
        p_next: ptr::null(),
        flags: vk::VK_COMMAND_POOL_CREATE_RESET_COMMAND_BUFFER_BIT,
        queue_family_index: device.queue_family_index(),
    };
    let mut handle: vk::CommandPoolHandle = ptr::null_mut();
    // SAFETY: `info` 在栈上存活；`handle` 是可写输出。
    let rc = unsafe { (fns.create_command_pool)(device.handle(), &info, ptr::null(), &mut handle) };
    if rc != ffi::VK_SUCCESS {
        return Err(GpuError::Driver {
            code: rc,
            message: format!("vkCreateCommandPool 失败：{}", vk_result_name(rc)),
        });
    }
    Ok(OwnedCommandPool {
        handle,
        device: device.handle(),
        destroy: fns.destroy_command_pool,
    })
}

/// 从池里分配 `count` 个主命令缓冲。
fn allocate_command_buffers(
    device: &VkDevice,
    pool: vk::CommandPoolHandle,
    count: usize,
) -> GpuResult<Vec<vk::CommandBufferHandle>> {
    let fns = device.fns();
    let info = vk::CommandBufferAllocateInfo {
        s_type: vk::VK_STRUCTURE_TYPE_COMMAND_BUFFER_ALLOCATE_INFO,
        p_next: ptr::null(),
        command_pool: pool,
        level: vk::VK_COMMAND_BUFFER_LEVEL_PRIMARY,
        command_buffer_count: count as u32,
    };
    let mut buffers = vec![ptr::null_mut(); count];
    // SAFETY: `info` 在栈上存活；`buffers` 容量与 `command_buffer_count` 一致。
    let rc =
        unsafe { (fns.allocate_command_buffers)(device.handle(), &info, buffers.as_mut_ptr()) };
    if rc != ffi::VK_SUCCESS {
        return Err(GpuError::Driver {
            code: rc,
            message: format!(
                "vkAllocateCommandBuffers 失败（{count} 个）：{}",
                vk_result_name(rc)
            ),
        });
    }
    Ok(buffers)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_outcome_is_compareable() {
        assert_ne!(FrameOutcome::Presented, FrameOutcome::OutOfDate);
        assert_eq!(FrameOutcome::Presented, FrameOutcome::Presented);
    }

    #[test]
    fn triangle_is_not_degenerate() {
        // 退化三角形（面积为 0 / 三点共线）会被光栅化器直接丢掉 —— 那样「窗口一片清屏色」
        // 就会被误判成「绘制有问题」。这里把顶点质量钉住。
        let [a, b, c] = TRIANGLE_NDC;
        let area = (b[0] - a[0]) * (c[1] - a[1]) - (c[0] - a[0]) * (b[1] - a[1]);
        assert!(area.abs() > 0.05, "三角形面积太小：{area}");
        for p in [a, b, c] {
            assert!(p[0].abs() <= 1.0 && p[1].abs() <= 1.0, "顶点必须在 NDC 内：{p:?}");
        }
        // 颜色要与典型清屏色区分得开（否则「画没画」肉眼看不出来）
        const { assert!(TRIANGLE_COLOR[2] > 0.8, "三角形应当明显偏蓝") };
    }

    #[test]
    fn frames_in_flight_is_at_least_one() {
        const { assert!(FRAMES_IN_FLIGHT >= 1, "至少在飞的帧数要为 1") };
    }

    /// 「清屏色 = 调用方给的颜色」这条约定的**逐位**判据（示例的 CLEAR 就是 RGB 0x101424）。
    #[test]
    fn clear_color_value_is_exact() {
        let c = Color::rgb(0x10, 0x14, 0x24);
        let v = clear_color_value(c);
        assert_eq!(
            v,
            [16.0 / 255.0, 20.0 / 255.0, 36.0 / 255.0, 1.0],
            "通道不能被交换/缩放（B8G8R8A8 与 R8G8B8A8 的差别由驱动按格式处理）"
        );
        // 边界：0 与 255
        assert_eq!(clear_color_value(Color::rgb(0, 0, 0)), [0.0, 0.0, 0.0, 1.0]);
        assert_eq!(clear_color_value(Color::WHITE), [1.0, 1.0, 1.0, 1.0]);
        // alpha 原样传（半透明清屏是合法用法）
        let t = clear_color_value(Color::rgba(1, 2, 3, 0.25));
        assert_eq!(t[3], 0.25);
        assert_eq!(t[0..3], [1.0 / 255.0, 2.0 / 255.0, 3.0 / 255.0]);
    }

    /// sRGB 编码公式：**实测值**（本机 Intel/NVIDIA 驱动写进交换链的字节）与它逐位一致。
    ///
    /// 这三个数就是真窗口回读测试里四角读到的值 —— 把「驱动实测」与「公式」对齐之后，
    /// 像素断言才有正确参考值。
    #[test]
    fn srgb_encoding_matches_measured_driver_bytes() {
        assert_eq!(srgb_encoded_byte(0), 0);
        assert_eq!(srgb_encoded_byte(255), 255);
        // 清屏色 rgb(0x10,0x14,0x24)（线性）在 sRGB 附件里实测写成 [0x47,0x4F,0x69]
        assert_eq!(srgb_encoded_byte(0x10), 0x47, "16/255 线性 → 71");
        assert_eq!(srgb_encoded_byte(0x14), 0x4F, "20/255 线性 → 79");
        assert_eq!(srgb_encoded_byte(0x24), 0x69, "36/255 线性 → 105");
        // 单调不减（任何分段点写错都会在这里现形）
        let mut prev = 0u8;
        for v in 0..=255u8 {
            let e = srgb_encoded_byte(v);
            assert!(e >= prev, "sRGB 编码必须单调不减：{v} → {e} < {prev}");
            prev = e;
        }
        // 编码后比线性亮（sRGB 的已知性质：暗部抬升）
        assert!(srgb_encoded_byte(16) > 16);
    }
}
