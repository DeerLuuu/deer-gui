//! Vulkan **离屏渲染 + 回读像素**（M2a-4..6）。
//!
//! 这是 GPU 这条路的**终点判据**：把界面画进一张离屏图像，再把像素读回来，
//! 用它验证「GPU 真的画对了」——判据是**具体颜色值**，不是「有没有像素」。
//!
//! ## 一帧的完整流程（每一步都有存在理由）
//!
//! ```text
//!  ① 创建离屏 VkImage（DEVICE_LOCAL，用途 = 颜色附件 | 传输源）
//!  ② 建 ImageView + Framebuffer（绑定到渲染通道）
//!  ③ 建命令池 + 命令缓冲
//!  ④ 录制：beginRenderPass(CLEAR) → bindPipeline → setViewport/Scissor → draw → endRenderPass
//!  ⑤ 屏障：TRANSFER_SRC_OPTIMAL（渲染通道的 finalLayout 已保证，这里做显式屏障更稳）
//!  ⑥ copyImageToBuffer → 一块 HOST_VISIBLE 的暂存缓冲
//!  ⑦ vkQueueSubmit + 栅栏等待
//!  ⑧ map 暂存缓冲，读回 RGBA8
//! ```
//!
//! ## 像素格式
//!
//! 用 `VK_FORMAT_R8G8B8A8_UNORM`：回读的字节顺序就是 **R, G, B, A**，
//! 行优先、**无 padding**（`bufferRowLength = 0` 表示与图像宽度一致）。
//! 这一点对断言很重要 —— 用 `B8G8R8A8` 会让红蓝互换，是极容易踩的坑。

use deer_gpu::{GpuError, GpuResult};

use crate::device::{vk_result_name, DeviceFns};
use crate::ffi;
use crate::ffi_dev as vk;

/// 一块设备内存（不区分图像/缓冲用途；只管分配与释放）。
pub struct Memory {
    handle: vk::DeviceMemoryHandle,
    device: vk::DeviceHandle,
    free: vk::PfnFreeMemory,
}

impl Memory {
    pub fn handle(&self) -> vk::DeviceMemoryHandle {
        self.handle
    }
}

impl Drop for Memory {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            // SAFETY: 句柄由本设备分配且未释放；设备此时仍存活。
            unsafe { (self.free)(self.device, self.handle, std::ptr::null()) };
            self.handle = std::ptr::null_mut();
        }
    }
}

/// 一张离屏图像。
pub struct Image {
    handle: vk::ImageHandle,
    device: vk::DeviceHandle,
    destroy: vk::PfnDestroyImage,
    pub width: u32,
    pub height: u32,
    pub format: i32,
}

impl Image {
    pub fn handle(&self) -> vk::ImageHandle {
        self.handle
    }
}

impl Drop for Image {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            // SAFETY: 句柄由本设备创建且未销毁。
            unsafe { (self.destroy)(self.device, self.handle, std::ptr::null()) };
            self.handle = std::ptr::null_mut();
        }
    }
}

/// 一张图像的视图。
pub struct ImageView {
    handle: vk::ImageViewHandle,
    device: vk::DeviceHandle,
    destroy: vk::PfnDestroyImageView,
}

impl ImageView {
    pub fn handle(&self) -> vk::ImageViewHandle {
        self.handle
    }
}

impl Drop for ImageView {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            // SAFETY: 同上。
            unsafe { (self.destroy)(self.device, self.handle, std::ptr::null()) };
            self.handle = std::ptr::null_mut();
        }
    }
}

/// 帧缓冲（把图像视图绑到渲染通道）。
pub struct Framebuffer {
    handle: vk::FramebufferHandle,
    device: vk::DeviceHandle,
    destroy: vk::PfnDestroyFramebuffer,
}

impl Framebuffer {
    pub fn handle(&self) -> vk::FramebufferHandle {
        self.handle
    }
}

impl Drop for Framebuffer {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            // SAFETY: 同上。
            unsafe { (self.destroy)(self.device, self.handle, std::ptr::null()) };
            self.handle = std::ptr::null_mut();
        }
    }
}

/// 一块缓冲（暂存回读用）。
pub struct Buffer {
    handle: vk::BufferHandle,
    device: vk::DeviceHandle,
    destroy: vk::PfnDestroyBuffer,
    pub size: u64,
}

impl Buffer {
    pub fn handle(&self) -> vk::BufferHandle {
        self.handle
    }
}

impl Drop for Buffer {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            // SAFETY: 同上。
            unsafe { (self.destroy)(self.device, self.handle, std::ptr::null()) };
            self.handle = std::ptr::null_mut();
        }
    }
}

/// 命令池。命令缓冲**必须在它之前销毁**（Vulkan 规定 `vkFreeCommandBuffers`
/// 或池销毁时自动释放；本实现让命令缓冲不单独释放，由池统一回收）。
pub struct CommandPool {
    handle: vk::CommandPoolHandle,
    device: vk::DeviceHandle,
    destroy: vk::PfnDestroyCommandPool,
    queue_family_index: u32,
}

impl CommandPool {
    pub fn handle(&self) -> vk::CommandPoolHandle {
        self.handle
    }
    pub fn queue_family_index(&self) -> u32 {
        self.queue_family_index
    }
}

impl Drop for CommandPool {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            // SAFETY: 池由本设备创建且未销毁；它持有的命令缓冲会被一并释放。
            unsafe { (self.destroy)(self.device, self.handle, std::ptr::null()) };
            self.handle = std::ptr::null_mut();
        }
    }
}

/// 栅栏：用于「等这一帧真的做完」。
pub struct Fence {
    handle: vk::FenceHandle,
    device: vk::DeviceHandle,
    destroy: vk::PfnDestroyFence,
    wait: vk::PfnWaitForFences,
    reset: vk::PfnResetFences,
}

impl Fence {
    pub fn handle(&self) -> vk::FenceHandle {
        self.handle
    }

    /// 等待信号。
    ///
    /// `timeout_ns` 用 [`vk::U64_MAX`] 表示无限等待。**测试应给有限超时** ——
    /// 否则驱动出问题时测试会永久挂住（比失败更难排查）。
    pub fn wait(&self, timeout_ns: u64) -> GpuResult<()> {
        // SAFETY: 句柄有效；`&self.handle` 指向本结构持有的句柄，生命周期覆盖调用。
        let rc = unsafe { (self.wait)(self.device, 1, &self.handle, vk::VK_TRUE, timeout_ns) };
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

    pub fn reset(&self) -> GpuResult<()> {
        // SAFETY: 句柄有效且当前没有等待者。
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

impl Drop for Fence {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            // SAFETY: 同上。
            unsafe { (self.destroy)(self.device, self.handle, std::ptr::null()) };
            self.handle = std::ptr::null_mut();
        }
    }
}

/// 设备侧的「离屏渲染会话」：把「建图像 → 录制 → 提交 → 读回」这套流程收在一处。
///
/// 设计取舍：**不引入全局状态机**。调用方按顺序调方法，每步失败都返回 `GpuError`。
/// 这样它能在测试里被逐步验证，也不会像「隐式帧循环」那样难以调试。
///
/// ## 字段顺序 = 析构顺序（**不要重排**）
///
/// - `image` 在 `image_memory` **之前** ⇒ 先销毁 `VkImage`、再 `vkFreeMemory`（内存不能
///   先于绑定它的图像消失）；
/// - 两者都在 `device`（不在本结构里，由 `VkDevice` 持有）之前 ⇒ `vkFreeMemory` 时设备仍存活。
pub struct OffscreenRenderer {
    pub image: Image,
    /// 图像绑定的设备内存。**必须由本结构持有**（曾经用 `mem::forget` 泄漏掉）。
    image_memory: Memory,
    pub view: ImageView,
    pub framebuffer: Framebuffer,
    pub pool: CommandPool,
    cmd: vk::CommandBufferHandle,
    fns: DeviceFns,
    device: vk::DeviceHandle,
    queue: vk::QueueHandle,
    /// 暂存回读缓冲（HOST_VISIBLE | HOST_COHERENT）
    staging: Buffer,
    staging_memory: Memory,
    width: u32,
    height: u32,
}

impl OffscreenRenderer {
    /// 建一套离屏渲染设施（**私有**）。
    ///
    /// 为什么不公开：它接收**裸 Vulkan 句柄**。clippy 的 `not_unsafe_ptr_arg_deref`
    /// 正确地指出「公开函数收裸指针却不标 `unsafe`」是危险 API —— 调用方可能传任意值。
    /// 所以公开入口只留 [`offscreen_for`]（收 `&VkDevice`，句柄有效性由它保证），
    /// 本函数不对外暴露。这是**修 API 而不是压 lint**。
    #[allow(clippy::too_many_arguments)]
    fn new(
        device: vk::DeviceHandle,
        queue: vk::QueueHandle,
        queue_family_index: u32,
        fns: DeviceFns,
        mem_props: &vk::PhysicalDeviceMemoryProperties,
        render_pass: &crate::device::RenderPass,
        width: u32,
        height: u32,
    ) -> GpuResult<OffscreenRenderer> {
        if width == 0 || height == 0 {
            return Err(GpuError::Unsupported("离屏图像的宽高必须大于 0".to_string()));
        }
        let format = vk::VK_FORMAT_R8G8B8A8_UNORM;

        // ① 图像：DEVICE_LOCAL、颜色附件 + 传输源
        let img_info = vk::ImageCreateInfo {
            s_type: vk::VK_STRUCTURE_TYPE_IMAGE_CREATE_INFO,
            p_next: std::ptr::null(),
            flags: 0,
            image_type: vk::VK_IMAGE_TYPE_2D,
            format,
            extent: vk::Extent3D {
                width,
                height,
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
        let rc = unsafe { (fns.create_image)(device, &img_info, std::ptr::null(), &mut image_handle) };
        if rc != ffi::VK_SUCCESS {
            return Err(GpuError::Driver {
                code: rc,
                message: format!("vkCreateImage 失败：{}", vk_result_name(rc)),
            });
        }
        let image = Image {
            handle: image_handle,
            device,
            destroy: fns.destroy_image,
            width,
            height,
            format,
        };

        // ② 分配并绑定设备内存
        let mut req = std::mem::MaybeUninit::<vk::MemoryRequirements>::uninit();
        // SAFETY: 该函数完整写入结构体。
        unsafe { (fns.get_image_memory_requirements)(device, image.handle(), req.as_mut_ptr()) };
        let req = unsafe { req.assume_init() };
        let mem_index = pick_memory_type(mem_props, req.memory_type_bits, vk::VK_MEMORY_PROPERTY_DEVICE_LOCAL_BIT)?;
        let image_memory = alloc_memory(device, &fns, req.size, mem_index)?;
        // SAFETY: 图像与内存都是本设备的新对象，尺寸匹配；offset 0 合法。
        let rc = unsafe { (fns.bind_image_memory)(device, image.handle(), image_memory.handle(), 0) };
        if rc != ffi::VK_SUCCESS {
            return Err(GpuError::Driver {
                code: rc,
                message: format!("vkBindImageMemory 失败：{}", vk_result_name(rc)),
            });
        }

        // ③ 视图
        let view_info = vk::ImageViewCreateInfo {
            s_type: vk::VK_STRUCTURE_TYPE_IMAGE_VIEW_CREATE_INFO,
            p_next: std::ptr::null(),
            flags: 0,
            image: image.handle(),
            view_type: vk::VK_IMAGE_VIEW_TYPE_2D,
            format,
            components_r: vk::VK_COMPONENT_SWIZZLE_IDENTITY,
            components_g: vk::VK_COMPONENT_SWIZZLE_IDENTITY,
            components_b: vk::VK_COMPONENT_SWIZZLE_IDENTITY,
            components_a: vk::VK_COMPONENT_SWIZZLE_IDENTITY,
            subresource_range: color_range(),
        };
        let mut view_handle: vk::ImageViewHandle = std::ptr::null_mut();
        // SAFETY: 同上。
        let rc = unsafe { (fns.create_image_view)(device, &view_info, std::ptr::null(), &mut view_handle) };
        if rc != ffi::VK_SUCCESS {
            return Err(GpuError::Driver {
                code: rc,
                message: format!("vkCreateImageView 失败：{}", vk_result_name(rc)),
            });
        }
        let view = ImageView {
            handle: view_handle,
            device,
            destroy: fns.destroy_image_view,
        };

        // ④ 帧缓冲
        let fb_info = vk::FramebufferCreateInfo {
            s_type: vk::VK_STRUCTURE_TYPE_FRAMEBUFFER_CREATE_INFO,
            p_next: std::ptr::null(),
            flags: 0,
            render_pass: render_pass.handle(),
            attachment_count: 1,
            p_attachments: &view.handle(),
            width,
            height,
            layers: 1,
        };
        let mut fb_handle: vk::FramebufferHandle = std::ptr::null_mut();
        // SAFETY: 同上。
        let rc = unsafe { (fns.create_framebuffer)(device, &fb_info, std::ptr::null(), &mut fb_handle) };
        if rc != ffi::VK_SUCCESS {
            return Err(GpuError::Driver {
                code: rc,
                message: format!("vkCreateFramebuffer 失败：{}", vk_result_name(rc)),
            });
        }
        let framebuffer = Framebuffer {
            handle: fb_handle,
            device,
            destroy: fns.destroy_framebuffer,
        };

        // ⑤ 命令池 + 一个主命令缓冲
        let pool_info = vk::CommandPoolCreateInfo {
            s_type: vk::VK_STRUCTURE_TYPE_COMMAND_POOL_CREATE_INFO,
            p_next: std::ptr::null(),
            flags: vk::VK_COMMAND_POOL_CREATE_RESET_COMMAND_BUFFER_BIT,
            queue_family_index,
        };
        let mut pool_handle: vk::CommandPoolHandle = std::ptr::null_mut();
        // SAFETY: 同上。
        let rc = unsafe { (fns.create_command_pool)(device, &pool_info, std::ptr::null(), &mut pool_handle) };
        if rc != ffi::VK_SUCCESS {
            return Err(GpuError::Driver {
                code: rc,
                message: format!("vkCreateCommandPool 失败：{}", vk_result_name(rc)),
            });
        }
        let pool = CommandPool {
            handle: pool_handle,
            device,
            destroy: fns.destroy_command_pool,
            queue_family_index,
        };
        let alloc_info = vk::CommandBufferAllocateInfo {
            s_type: vk::VK_STRUCTURE_TYPE_COMMAND_BUFFER_ALLOCATE_INFO,
            p_next: std::ptr::null(),
            command_pool: pool.handle(),
            level: vk::VK_COMMAND_BUFFER_LEVEL_PRIMARY,
            command_buffer_count: 1,
        };
        let mut cmd: vk::CommandBufferHandle = std::ptr::null_mut();
        // SAFETY: 池有效；`cmd` 是可写输出。
        let rc = unsafe { (fns.allocate_command_buffers)(device, &alloc_info, &mut cmd) };
        if rc != ffi::VK_SUCCESS {
            return Err(GpuError::Driver {
                code: rc,
                message: format!("vkAllocateCommandBuffers 失败：{}", vk_result_name(rc)),
            });
        }

        // ⑥ 暂存缓冲（HOST_VISIBLE | HOST_COHERENT ⇒ 不需要显式 flush）
        let bytes = (width as u64) * (height as u64) * 4;
        let buf_info = vk::BufferCreateInfo {
            s_type: vk::VK_STRUCTURE_TYPE_BUFFER_CREATE_INFO,
            p_next: std::ptr::null(),
            flags: 0,
            size: bytes,
            usage: vk::VK_BUFFER_USAGE_TRANSFER_DST_BIT,
            sharing_mode: vk::VK_SHARING_MODE_EXCLUSIVE,
            queue_family_index_count: 0,
            p_queue_family_indices: std::ptr::null(),
        };
        let mut buf_handle: vk::BufferHandle = std::ptr::null_mut();
        // SAFETY: 同上。
        let rc = unsafe { (fns.create_buffer)(device, &buf_info, std::ptr::null(), &mut buf_handle) };
        if rc != ffi::VK_SUCCESS {
            return Err(GpuError::Driver {
                code: rc,
                message: format!("vkCreateBuffer 失败：{}", vk_result_name(rc)),
            });
        }
        let staging = Buffer {
            handle: buf_handle,
            device,
            destroy: fns.destroy_buffer,
            size: bytes,
        };
        let mut buf_req = std::mem::MaybeUninit::<vk::MemoryRequirements>::uninit();
        // SAFETY: 该函数完整写入结构体。
        unsafe { (fns.get_buffer_memory_requirements)(device, staging.handle(), buf_req.as_mut_ptr()) };
        let buf_req = unsafe { buf_req.assume_init() };
        let host_index = pick_memory_type(
            mem_props,
            buf_req.memory_type_bits,
            vk::VK_MEMORY_PROPERTY_HOST_VISIBLE_BIT | vk::VK_MEMORY_PROPERTY_HOST_COHERENT_BIT,
        )?;
        let staging_memory = alloc_memory(device, &fns, buf_req.size, host_index)?;
        // SAFETY: 缓冲与内存都是新对象，尺寸匹配。
        let rc = unsafe {
            (fns.bind_buffer_memory)(device, staging.handle(), staging_memory.handle(), 0)
        };
        if rc != ffi::VK_SUCCESS {
            return Err(GpuError::Driver {
                code: rc,
                message: format!("vkBindBufferMemory 失败：{}", vk_result_name(rc)),
            });
        }

        // **所有权明确**：图像内存归 `OffscreenRenderer`（`image_memory` 字段）。
        //
        // 这里曾经是 `std::mem::forget(image_memory)` + 一句「所有权转移给 image（它的 Drop
        // 不释放内存，所以这里显式保管）」—— 那句话自相矛盾，实际效果是**没人释放**：
        // 校验层在 `vkDestroyDevice` 时报 `has 1 leaked objects that have not been destroyed`，
        // 每个 `OffscreenRenderer` 泄漏一块设备内存。
        //
        // 现在的规则：`Memory` 的 Drop 会 `vkFreeMemory`；把它作为字段持有即可。
        // 字段**声明在 `image` 之后**（Rust 按声明顺序析构）⇒ 先销毁图像、再释放内存，
        // 不会出现「内存先没了而图像还绑着它」。**不会双释放**：`Image::drop` 只销毁
        // `VkImage`（它不持有内存），`Memory::drop` 只释放 `VkDeviceMemory`，两者各管一头。
        Ok(OffscreenRenderer {
            image,
            image_memory,
            view,
            framebuffer,
            pool,
            cmd,
            fns,
            device,
            queue,
            staging,
            staging_memory,
            width,
            height,
        })
    }

    pub fn width(&self) -> u32 {
        self.width
    }
    pub fn height(&self) -> u32 {
        self.height
    }

    /// 图像绑定的设备内存句柄（诊断/泄漏排查用；所有权仍在本结构，`Drop` 时释放）。
    pub fn image_memory(&self) -> vk::DeviceMemoryHandle {
        self.image_memory.handle()
    }

    /// 录制一帧：清屏 + 绑定管线 + 动态 viewport/scissor + 画 `vertex_count` 个顶点。
    ///
    /// 然后提交并**等栅栏**（有限超时），再回读像素。
    pub fn render_and_read_back(
        &self,
        render_pass: &crate::device::RenderPass,
        pipeline: &crate::device::Pipeline,
        vertex_count: u32,
        clear: [f32; 4],
    ) -> GpuResult<Vec<u8>> {
        if vertex_count == 0 {
            return Err(GpuError::Unsupported("vertex_count 必须大于 0".to_string()));
        }
        // 先确保上一次使用同一个命令缓冲的提交已完成（本实现一次只用一个缓冲）
        // SAFETY: 命令缓冲由本结构持有且未处于录制状态。
        let rc = unsafe { (self.fns.reset_command_buffer)(self.cmd, 0) };
        if rc != ffi::VK_SUCCESS {
            return Err(GpuError::Driver {
                code: rc,
                message: format!("vkResetCommandBuffer 失败：{}", vk_result_name(rc)),
            });
        }
        let begin = vk::CommandBufferBeginInfo {
            s_type: vk::VK_STRUCTURE_TYPE_COMMAND_BUFFER_BEGIN_INFO,
            p_next: std::ptr::null(),
            flags: vk::VK_COMMAND_BUFFER_USAGE_ONE_TIME_SUBMIT_BIT,
            p_inheritance_info: std::ptr::null(),
        };
        // SAFETY: 句柄有效，`begin` 在栈上存活。
        let rc = unsafe { (self.fns.begin_command_buffer)(self.cmd, &begin) };
        if rc != ffi::VK_SUCCESS {
            return Err(GpuError::Driver {
                code: rc,
                message: format!("vkBeginCommandBuffer 失败：{}", vk_result_name(rc)),
            });
        }

        // —— 渲染通道 ——
        let clear_value = vk::ClearValue {
            color: vk::ClearColorValue { float32: clear },
        };
        let begin_pass = vk::RenderPassBeginInfo {
            s_type: vk::VK_STRUCTURE_TYPE_RENDER_PASS_BEGIN_INFO,
            p_next: std::ptr::null(),
            render_pass: render_pass.handle(),
            framebuffer: self.framebuffer.handle(),
            render_area: vk::Rect2D {
                offset: vk::Offset2D { x: 0, y: 0 },
                extent: vk::Extent2D {
                    width: self.width,
                    height: self.height,
                },
            },
            clear_value_count: 1,
            p_clear_values: &clear_value,
        };
        // SAFETY: 上述结构体在栈上存活。
        unsafe {
            (self.fns.cmd_begin_render_pass)(self.cmd, &begin_pass, vk::VK_SUBPASS_CONTENTS_INLINE);
            (self.fns.cmd_bind_pipeline)(
                self.cmd,
                vk::VK_PIPELINE_BIND_POINT_GRAPHICS,
                pipeline.handle(),
            );
            // viewport：NDC → 像素（注意 height 取正，Vulkan 的 Y 向下）
            let vp = vk::Viewport {
                x: 0.0,
                y: 0.0,
                width: self.width as f32,
                height: self.height as f32,
                min_depth: 0.0,
                max_depth: 1.0,
            };
            (self.fns.cmd_set_viewport)(self.cmd, 0, 1, &vp);
            let sc = vk::Rect2D {
                offset: vk::Offset2D { x: 0, y: 0 },
                extent: vk::Extent2D {
                    width: self.width,
                    height: self.height,
                },
            };
            (self.fns.cmd_set_scissor)(self.cmd, 0, 1, &sc);
            (self.fns.cmd_draw)(self.cmd, vertex_count, 1, 0, 0);
            (self.fns.cmd_end_render_pass)(self.cmd);
        }

        // —— 屏障 + 拷回暂存缓冲 ——
        //
        // ⚠️ 这里曾经写死 `old_layout = COLOR_ATTACHMENT_OPTIMAL`，并配一句
        // 「渲染通道的 finalLayout 已经是 TRANSFER_SRC_OPTIMAL，但为了不依赖那一点，
        //  这里显式再做一次屏障（重复屏障是合法的）」—— **两句都错**：
        //   · `oldLayout` 必须等于图像**当前实际**布局。渲染通道的 `finalLayout` 是
        //     `TRANSFER_SRC_OPTIMAL`，所以声明成 `COLOR_ATTACHMENT_OPTIMAL` 是一次
        //     「从错误布局出发」的转换，校验层直接报 `cannot transition the layout ...`；
        //   · 「重复屏障」只有 `oldLayout == newLayout` 时才叫重复；从别的布局出发不是。
        //
        // 正确做法：向渲染通道要它的 `finalLayout()`，用它当 `oldLayout`。
        // 这样无论调用方建渲染通道时给的是什么 finalLayout（本模块文档推荐
        // `TRANSFER_SRC_OPTIMAL`），这次转换的起点都是事实。屏障本身仍然必要 ——
        // 它建立 `COLOR_ATTACHMENT_WRITE → TRANSFER_READ` 的内存可见性依赖。
        let barrier = vk::ImageMemoryBarrier {
            s_type: vk::VK_STRUCTURE_TYPE_IMAGE_MEMORY_BARRIER,
            p_next: std::ptr::null(),
            src_access_mask: vk::VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT,
            dst_access_mask: vk::VK_ACCESS_TRANSFER_READ_BIT,
            old_layout: render_pass.final_layout(),
            new_layout: vk::VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL,
            src_queue_family_index: vk::QUEUE_FAMILY_IGNORED,
            dst_queue_family_index: vk::QUEUE_FAMILY_IGNORED,
            image: self.image.handle(),
            subresource_range: color_range(),
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
                    width: self.width,
                    height: self.height,
                    depth: 1,
                },
            };
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
        let rc = unsafe { (self.fns.end_command_buffer)(self.cmd) };
        if rc != ffi::VK_SUCCESS {
            return Err(GpuError::Driver {
                code: rc,
                message: format!("vkEndCommandBuffer 失败：{}", vk_result_name(rc)),
            });
        }

        // —— 提交 + 等栅栏 ——
        let fence_info = vk::FenceCreateInfo {
            s_type: vk::VK_STRUCTURE_TYPE_FENCE_CREATE_INFO,
            p_next: std::ptr::null(),
            flags: 0, // 不预设 signaled ⇒ 必须真的等 GPU
        };
        let mut fence_handle: vk::FenceHandle = std::ptr::null_mut();
        // SAFETY: 同上。
        let rc = unsafe { (self.fns.create_fence)(self.device, &fence_info, std::ptr::null(), &mut fence_handle) };
        if rc != ffi::VK_SUCCESS {
            return Err(GpuError::Driver {
                code: rc,
                message: format!("vkCreateFence 失败：{}", vk_result_name(rc)),
            });
        }
        let fence = Fence {
            handle: fence_handle,
            device: self.device,
            destroy: self.fns.destroy_fence,
            wait: self.fns.wait_for_fences,
            reset: self.fns.reset_fences,
        };

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
        let rc = unsafe { (self.fns.queue_submit)(self.queue, 1, &submit, fence.handle()) };
        if rc != ffi::VK_SUCCESS {
            return Err(GpuError::Driver {
                code: rc,
                message: format!("vkQueueSubmit 失败：{}", vk_result_name(rc)),
            });
        }

        // 有限超时（1 秒）：驱动出问题时**宁可失败，也不要永久挂住**
        fence.wait(1_000_000_000)?;

        // —— 读回 ——
        let mut mapped: *mut std::ffi::c_void = std::ptr::null_mut();
        // SAFETY: 内存是 HOST_VISIBLE 且 COHERENT；映射整块。
        let rc = unsafe {
            (self.fns.map_memory)(
                self.device,
                self.staging_memory.handle(),
                0,
                vk::WHOLE_SIZE,
                0,
                &mut mapped,
            )
        };
        if rc != ffi::VK_SUCCESS {
            return Err(GpuError::Driver {
                code: rc,
                message: format!("vkMapMemory 失败：{}", vk_result_name(rc)),
            });
        }
        if mapped.is_null() {
            return Err(GpuError::Driver {
                code: 0,
                message: "vkMapMemory 返回空指针".to_string(),
            });
        }
        // SAFETY: 映射了 `staging.size` 字节（VK_WHOLE_SIZE）；长度取自缓冲大小。
        let out = unsafe {
            std::slice::from_raw_parts(mapped as *const u8, self.staging.size as usize).to_vec()
        };
        // SAFETY: 与上面的 map 配对。
        unsafe { (self.fns.unmap_memory)(self.device, self.staging_memory.handle()) };
        Ok(out)
    }
}

/// 从 `VkDevice` 一步建好离屏渲染设施（把内部句柄与内存属性的取值收在一处）。
pub fn offscreen_for(
    dev: &crate::device::VkDevice,
    render_pass: &crate::device::RenderPass,
    width: u32,
    height: u32,
) -> GpuResult<OffscreenRenderer> {
    OffscreenRenderer::new(
        dev.handle(),
        dev.queue(),
        dev.queue_family_index(),
        *dev.fns(),
        dev.memory_properties(),
        render_pass,
        width,
        height,
    )
}

fn color_range() -> vk::ImageSubresourceRange {
    vk::ImageSubresourceRange {
        aspect_mask: vk::VK_IMAGE_ASPECT_COLOR_BIT,
        base_mip_level: 0,
        level_count: 1,
        base_array_layer: 0,
        layer_count: 1,
    }
}

/// 从设备的内存类型里挑一个同时满足 `required` 所有位的。
///
/// 属性来自**物理设备**（`vkGetPhysicalDeviceMemoryProperties`），
/// 由调用方在打开设备时取好并传入 —— 逻辑设备上拿不到它。
fn pick_memory_type(
    props: &vk::PhysicalDeviceMemoryProperties,
    type_bits: u32,
    required: u32,
) -> GpuResult<u32> {
    let count = props.memory_type_count.min(32);
    for i in 0..count {
        let m = &props.memory_types[i as usize];
        // 该类型必须被资源允许（type_bits 的第 i 位为 1）
        if type_bits & (1 << i) == 0 {
            continue;
        }
        // 且必须包含所有必需属性
        if m.property_flags & required == required {
            return Ok(i);
        }
    }
    Err(GpuError::Unsupported(format!(
        "找不到满足属性 {required:#x} 的内存类型（资源的 type_bits = {type_bits:#x}）"
    )))
}

fn alloc_memory(
    device: vk::DeviceHandle,
    fns: &DeviceFns,
    size: u64,
    memory_type_index: u32,
) -> GpuResult<Memory> {
    let info = vk::MemoryAllocateInfo {
        s_type: vk::VK_STRUCTURE_TYPE_MEMORY_ALLOCATE_INFO,
        p_next: std::ptr::null(),
        allocation_size: size,
        memory_type_index,
    };
    let mut handle: vk::DeviceMemoryHandle = std::ptr::null_mut();
    // SAFETY: 结构体在栈上存活；句柄是可写输出。
    let rc = unsafe { (fns.allocate_memory)(device, &info, std::ptr::null(), &mut handle) };
    if rc != ffi::VK_SUCCESS {
        return Err(GpuError::Driver {
            code: rc,
            message: format!("vkAllocateMemory 失败（size={size}）：{}", vk_result_name(rc)),
        });
    }
    Ok(Memory {
        handle,
        device,
        free: fns.free_memory,
    })
}
