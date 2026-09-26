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
    pub cmd_set_viewport: vk::PfnCmdSetViewport,
    pub cmd_set_scissor: vk::PfnCmdSetScissor,
    pub cmd_draw: vk::PfnCmdDraw,
    pub cmd_push_constants: vk::PfnCmdPushConstants,
    pub cmd_pipeline_barrier: vk::PfnCmdPipelineBarrier,
    pub cmd_copy_image_to_buffer: vk::PfnCmdCopyImageToBuffer,
    pub cmd_clear_color_image: vk::PfnCmdClearColorImage,
}

/// 已打开的逻辑设备。
///
/// `handle` 与 `fns` 都是 `Send` 的普通数据；真正的 Vulkan 对象在那个后台线程里。
pub struct VkDevice {
    handle: vk::DeviceHandle,
    queue: vk::QueueHandle,
    queue_family_index: u32,
    fns: DeviceFns,
    adapter: deer_gpu::AdapterInfo,
    memory_type_count: u32,
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
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<ReadyInfo, GpuError>>();
        let (stop_tx, stop_rx) = std::sync::mpsc::channel::<()>();

        let thread = std::thread::Builder::new()
            .name("deer-vk-lifetime".to_string())
            .spawn(move || {
                lifetime_thread(adapter_index, ready_tx, stop_rx);
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
                fns: info.fns,
                adapter: info.adapter,
                memory_type_count: info.memory_type_count,
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
        })
    }

    /// 创建管线布局。
    ///
    /// 推送常量的 `size` 必须是 4 的倍数。**注意范围大小与实际 push 的大小要一致** ——
    /// 不匹配是那种「能建成功但运行时行为诡异」的错误。
    pub fn create_pipeline_layout(
        &self,
        push_constant: Option<(u32, u32, u32)>,
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
        let info = vk::PipelineLayoutCreateInfo {
            s_type: vk::VK_STRUCTURE_TYPE_PIPELINE_LAYOUT_CREATE_INFO,
            p_next: std::ptr::null(),
            flags: 0,
            set_layout_count: 0,
            p_set_layouts: std::ptr::null(),
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
        if stages.is_empty() {
            return Err(GpuError::Unsupported(
                "图形管线至少要有一个着色器阶段".to_string(),
            ));
        }
        // 顶点完全由着色器内的常量表 + 推送常量决定 ⇒ 无需顶点缓冲/属性
        let vertex_input = vk::PipelineVertexInputStateCreateInfo {
            s_type: vk::VK_STRUCTURE_TYPE_PIPELINE_VERTEX_INPUT_STATE_CREATE_INFO,
            p_next: std::ptr::null(),
            flags: 0,
            vertex_binding_description_count: 0,
            p_vertex_binding_descriptions: std::ptr::null(),
            vertex_attribute_description_count: 0,
            p_vertex_attribute_descriptions: std::ptr::null(),
        };
        let input_assembly = vk::PipelineInputAssemblyStateCreateInfo {
            s_type: vk::VK_STRUCTURE_TYPE_PIPELINE_INPUT_ASSEMBLY_STATE_CREATE_INFO,
            p_next: std::ptr::null(),
            flags: 0,
            topology: vk::VK_PRIMITIVE_TOPOLOGY_TRIANGLE_LIST,
            primitive_restart_enable: vk::VK_FALSE,
        };
        // viewport/scissor 用**动态状态**：`count = 1` 但指针为空是合法的，因为值由
        // `vkCmdSetViewport` / `vkCmdSetScissor` 在录制时给。这对 GUI 渲染很关键
        // （每帧尺寸都可能变，不该重建管线）。
        let viewport_state = vk::PipelineViewportStateCreateInfo {
            s_type: vk::VK_STRUCTURE_TYPE_PIPELINE_VIEWPORT_STATE_CREATE_INFO,
            p_next: std::ptr::null(),
            flags: 0,
            viewport_count: 1,
            p_viewports: std::ptr::null(),
            scissor_count: 1,
            p_scissors: std::ptr::null(),
        };
        let dynamic_states = [vk::VK_DYNAMIC_STATE_VIEWPORT, vk::VK_DYNAMIC_STATE_SCISSOR];
        let dynamic_state = vk::PipelineDynamicStateCreateInfo {
            s_type: 27, // VK_STRUCTURE_TYPE_PIPELINE_DYNAMIC_STATE_CREATE_INFO
            p_next: std::ptr::null(),
            flags: 0,
            dynamic_state_count: dynamic_states.len() as u32,
            p_dynamic_states: dynamic_states.as_ptr(),
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
            blend_enable: vk::VK_TRUE,
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
            p_vertex_input_state: &vertex_input,
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

/// 一个渲染通道。`Drop` 时销毁。
pub struct RenderPass {
    handle: vk::RenderPassHandle,
    device: vk::DeviceHandle,
    destroy: vk::PfnDestroyRenderPass,
}

impl RenderPass {
    pub fn handle(&self) -> vk::RenderPassHandle {
        self.handle
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
    queue: vk::QueueHandle,
    queue_family_index: u32,
    fns: DeviceFns,
    adapter: deer_gpu::AdapterInfo,
    memory_type_count: u32,
}

// SAFETY: Vulkan 句柄是**进程级的不透明值**（`VK_NULL_HANDLE` 之外的任何句柄都可跨线程
// 传递；线程安全性由「不在两个线程同时使用同一个队列」这类规则约束，而不是由句柄类型决定）。
// 这里只是在「后台线程 → 主线程」单向传递一次句柄，主线程随后独占使用。
// `DeviceFns` 全是函数指针，天然 `Send`。
unsafe impl Send for ReadyInfo {}

/// 后台线程：创建实例与设备，报告句柄，然后 `park` 等停止信号。
fn lifetime_thread(
    adapter_index: usize,
    ready: Sender<Result<ReadyInfo, GpuError>>,
    stop: Receiver<()>,
) {
    let instance = match ffi::Instance::create() {
        Ok(i) => i,
        Err(e) => {
            let _ = ready.send(Err(e));
            return;
        }
    };
    let fns = match resolve_device_fns(&instance) {
        Ok(f) => f,
        Err(e) => {
            let _ = ready.send(Err(e));
            return;
        }
    };

    let physical_devices = match instance.enumerate_physical_devices() {
        Ok(d) => d,
        Err(e) => {
            let _ = ready.send(Err(e));
            return;
        }
    };
    let Some(pd) = physical_devices.get(adapter_index).copied() else {
        let _ = ready.send(Err(GpuError::NoAdapter));
        return;
    };

    // 找一个带图形位的队列族
    let mut count: u32 = 0;
    // SAFETY: 传 null 是 Vulkan 规定的「只查数量」用法。
    unsafe { (fns.get_queue_family_properties)(pd, &mut count, std::ptr::null_mut()) };
    if count == 0 {
        let _ = ready.send(Err(GpuError::NoAdapter));
        return;
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

    let Some(qfi) = families
        .iter()
        .position(|f| f.queue_flags & vk::VK_QUEUE_GRAPHICS_BIT != 0)
    else {
        let _ = ready.send(Err(GpuError::Unsupported(
            "没有任何队列族支持图形操作".to_string(),
        )));
        return;
    };
    let qfi = qfi as u32;

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
        enabled_extension_count: 0,
        pp_enabled_extension_names: std::ptr::null(),
        p_enabled_features: std::ptr::null(),
    };
    let mut device: vk::DeviceHandle = std::ptr::null_mut();
    // SAFETY: 上述结构体都在本栈帧存活；句柄是可写输出。
    let rc = unsafe {
        (fns.create_device)(pd, &device_info, std::ptr::null(), &mut device)
    };
    if rc != ffi::VK_SUCCESS {
        let _ = ready.send(Err(GpuError::Driver {
            code: rc,
            message: format!("vkCreateDevice 失败：{}", vk_result_name(rc)),
        }));
        return;
    }

    let mut queue: vk::QueueHandle = std::ptr::null_mut();
    // SAFETY: 设备刚创建、队列族存在、索引 0 合法。
    unsafe { (fns.get_device_queue)(device, qfi, 0, &mut queue) };

    // 内存类型数（诊断用）
    let mut mem_props = std::mem::MaybeUninit::<vk::PhysicalDeviceMemoryProperties>::uninit();
    // SAFETY: 该函数完整写入结构体。
    unsafe { (fns.get_memory_properties)(pd, mem_props.as_mut_ptr()) };
    let memory_type_count = unsafe { mem_props.assume_init() }.memory_type_count;

    let adapter = match unsafe { instance.physical_device_properties(pd) } {
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
            let _ = ready.send(Err(e));
            return;
        }
    };

    if ready
        .send(Ok(ReadyInfo {
            handle: device,
            queue,
            queue_family_index: qfi,
            fns,
            adapter,
            memory_type_count,
        }))
        .is_err()
    {
        // 主线程已经不等了 ⇒ 立刻清理
        // SAFETY: 设备刚创建、尚未销毁。
        unsafe { (fns.destroy_device)(device, std::ptr::null()) };
        return;
    }

    // 等停止信号（`recv` 在发送端析构时也会返回，避免永久阻塞）
    let _ = stop.recv();

    // 销毁顺序：设备 → 实例（`instance` 的 Drop 负责后者）
    // SAFETY: 设备由本线程创建、尚未销毁，且此刻没有其他线程在使用它
    // （`VkDevice` 的 Drop 会先 join 本线程）。
    unsafe { (fns.destroy_device)(device, std::ptr::null()) };
    drop(instance);
}

fn resolve_device_fns(instance: &ffi::Instance) -> GpuResult<DeviceFns> {
    let lib = Lib::open()?;
    // SAFETY: 每个符号名都与 `vk::Pfn*` 声明的签名一致（见 ffi_dev.rs 的类型定义）。
    // 这些函数在 Vulkan 1.0 就是全局导出的，所以不需要 vkGetInstanceProcAddr。
    unsafe {
        let _ = instance; // 保留参数以便将来切到 vkGetInstanceProcAddr
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
            cmd_set_viewport: lib.sym("vkCmdSetViewport")?,
            cmd_set_scissor: lib.sym("vkCmdSetScissor")?,
            cmd_draw: lib.sym("vkCmdDraw")?,
            cmd_push_constants: lib.sym("vkCmdPushConstants")?,
            cmd_pipeline_barrier: lib.sym("vkCmdPipelineBarrier")?,
            cmd_copy_image_to_buffer: lib.sym("vkCmdCopyImageToBuffer")?,
            cmd_clear_color_image: lib.sym("vkCmdClearColorImage")?,
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
