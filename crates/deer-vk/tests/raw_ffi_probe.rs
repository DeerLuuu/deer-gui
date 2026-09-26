//! **手写裸 FFI 渲染路径**（回归测试）。
//!
//! 这个测试**完全绕开 `deer-vk` 的封装**：只用 loader 拿到的函数指针，手写
//! 渲染通道 / 帧缓冲 / 管线 / 命令缓冲 / 内存 / 栅栏 / 拷贝。
//!
//! ## 它存在的理由（一次真实排查的价值）
//!
//! M2a 期间 `vkCmdDraw` **不产生任何像素**。当时用这个测试证明了
//! **bug 不在封装里**，从而把注意力转向 Vulkan 用法与 SPIR-V 产物 ——
//! 最终定位到根因：自研 SPIR-V 汇编器的**段序错误**
//! （`OpEntryPoint` 排在类型之后、`OpFunction` 掉进类型段）。
//!
//! 所以它现在是一条**端到端回归测试**：画绿色三角形到红色背景，
//! 断言两类像素的数量与比例。
//!
//! 不做任何销毁：进程退出时驱动会清理。这是探针，不是库代码。

use deer_vk::device::VkDevice;
use deer_vk::ffi_dev as vk;
use deer_vk::spirv;

const W: u32 = 64;
const H: u32 = 64;
const FMT: i32 = vk::VK_FORMAT_R8G8B8A8_UNORM;

#[test]
fn raw_render_path() {
    let Ok(dev) = VkDevice::open(0) else {
        println!("跳过：没有可用的 Vulkan");
        return;
    };
    let fns = *dev.fns();
    let device = dev.handle();
    let queue = dev.queue();
    let family = dev.queue_family_index();
    let mem_props = *dev.memory_properties();

    // ── 图像 ──
    let img_info = vk::ImageCreateInfo {
        s_type: vk::VK_STRUCTURE_TYPE_IMAGE_CREATE_INFO,
        p_next: std::ptr::null(),
        flags: 0,
        image_type: vk::VK_IMAGE_TYPE_2D,
        format: FMT,
        extent: vk::Extent3D { width: W, height: H, depth: 1 },
        mip_levels: 1,
        array_layers: 1,
        samples: vk::VK_SAMPLE_COUNT_1_BIT,
        tiling: vk::VK_IMAGE_TILING_OPTIMAL,
        usage: vk::VK_IMAGE_USAGE_COLOR_ATTACHMENT_BIT | vk::VK_IMAGE_USAGE_TRANSFER_SRC_BIT,
        sharing_mode: vk::VK_SHARING_MODE_EXCLUSIVE,
        queue_family_index_count: 0,
        p_queue_family_indices: std::ptr::null(),
        initial_layout: 0,
    };
    let mut image: vk::ImageHandle = std::ptr::null_mut();
    // SAFETY: 诊断调用；结构体在栈上存活。
    let rc = unsafe { (fns.create_image)(device, &img_info, std::ptr::null(), &mut image) };
    assert_eq!(rc, 0, "vkCreateImage {rc}");
    let mut req = std::mem::MaybeUninit::<vk::MemoryRequirements>::uninit();
    unsafe { (fns.get_image_memory_requirements)(device, image, req.as_mut_ptr()) };
    let req = unsafe { req.assume_init() };
    let dev_idx = pick(&mem_props, req.memory_type_bits, 0x1).expect("DEVICE_LOCAL");
    let mut img_mem: vk::DeviceMemoryHandle = std::ptr::null_mut();
    let ai = vk::MemoryAllocateInfo {
        s_type: vk::VK_STRUCTURE_TYPE_MEMORY_ALLOCATE_INFO,
        p_next: std::ptr::null(),
        allocation_size: req.size,
        memory_type_index: dev_idx,
    };
    let rc = unsafe { (fns.allocate_memory)(device, &ai, std::ptr::null(), &mut img_mem) };
    assert_eq!(rc, 0, "vkAllocateMemory(image) {rc}");
    let rc = unsafe { (fns.bind_image_memory)(device, image, img_mem, 0) };
    assert_eq!(rc, 0, "vkBindImageMemory {rc}");

    // ── 视图 ──
    let vi = vk::ImageViewCreateInfo {
        s_type: 15,
        p_next: std::ptr::null(),
        flags: 0,
        image,
        view_type: vk::VK_IMAGE_VIEW_TYPE_2D,
        format: FMT,
        components_r: 0,
        components_g: 0,
        components_b: 0,
        components_a: 0,
        subresource_range: vk::ImageSubresourceRange {
            aspect_mask: 1,
            base_mip_level: 0,
            level_count: 1,
            base_array_layer: 0,
            layer_count: 1,
        },
    };
    let mut view: vk::ImageViewHandle = std::ptr::null_mut();
    let rc = unsafe { (fns.create_image_view)(device, &vi, std::ptr::null(), &mut view) };
    assert_eq!(rc, 0, "vkCreateImageView {rc}");

    // ── 渲染通道（finalLayout = 6 = TRANSFER_SRC_OPTIMAL） ──
    let attach = vk::AttachmentDescription {
        flags: 0,
        format: FMT,
        samples: 1,
        load_op: vk::VK_ATTACHMENT_LOAD_OP_CLEAR,
        store_op: 1,
        stencil_load_op: 2,
        stencil_store_op: 2,
        initial_layout: 0,
        final_layout: 6,
    };
    let color_ref = vk::AttachmentReference { attachment: 0, layout: 2 };
    let subpass = vk::SubpassDescription {
        flags: 0,
        pipeline_bind_point: 0,
        input_attachment_count: 0,
        p_input_attachments: std::ptr::null(),
        color_attachment_count: 1,
        p_color_attachments: &color_ref,
        p_resolve_attachments: std::ptr::null(),
        p_depth_stencil_attachment: std::ptr::null(),
        preserve_attachment_count: 0,
        p_preserve_attachments: std::ptr::null(),
    };
    let deps = [vk::SubpassDependency {
        src_subpass: u32::MAX,
        dst_subpass: 0,
        src_stage_mask: 1 << 10,
        dst_stage_mask: 1 << 10,
        src_access_mask: 0,
        dst_access_mask: 1 << 8,
        dependency_flags: 0,
    }];
    let rpi = vk::RenderPassCreateInfo {
        s_type: 38,
        p_next: std::ptr::null(),
        flags: 0,
        attachment_count: 1,
        p_attachments: &attach,
        subpass_count: 1,
        p_subpasses: &subpass,
        dependency_count: 1,
        p_dependencies: deps.as_ptr(),
    };
    let mut render_pass: vk::RenderPassHandle = std::ptr::null_mut();
    let rc = unsafe { (fns.create_render_pass)(device, &rpi, std::ptr::null(), &mut render_pass) };
    assert_eq!(rc, 0, "vkCreateRenderPass {rc}");

    // ── 帧缓冲 ──
    let fbi = vk::FramebufferCreateInfo {
        s_type: 37,
        p_next: std::ptr::null(),
        flags: 0,
        render_pass,
        attachment_count: 1,
        p_attachments: &view,
        width: W,
        height: H,
        layers: 1,
    };
    let mut fb: vk::FramebufferHandle = std::ptr::null_mut();
    let rc = unsafe { (fns.create_framebuffer)(device, &fbi, std::ptr::null(), &mut fb) };
    assert_eq!(rc, 0, "vkCreateFramebuffer {rc}");
    println!("渲染通道 + 帧缓冲 ✅");

    // ── 管线布局 ──
    let pli = vk::PipelineLayoutCreateInfo {
        s_type: 30,
        p_next: std::ptr::null(),
        flags: 0,
        set_layout_count: 0,
        p_set_layouts: std::ptr::null(),
        push_constant_range_count: 0,
        p_push_constant_ranges: std::ptr::null(),
    };
    let mut pl: vk::PipelineLayoutHandle = std::ptr::null_mut();
    let rc = unsafe { (fns.create_pipeline_layout)(device, &pli, std::ptr::null(), &mut pl) };
    assert_eq!(rc, 0, "vkCreatePipelineLayout {rc}");

    // ── 着色器 ──
    let vs_bytes = spirv::vertex_shader_triangle([[-0.8, -0.8], [0.8, -0.8], [-0.8, 0.8]]);
    let fs_bytes = spirv::fragment_shader_solid([0.0, 1.0, 0.0, 1.0]);
    let mk = |bytes: &[u8]| -> vk::ShaderModuleHandle {
        let info = vk::ShaderModuleCreateInfo {
            s_type: 16,
            p_next: std::ptr::null(),
            flags: 0,
            code_size: bytes.len(),
            p_code: bytes.as_ptr() as *const u32,
        };
        let mut m: vk::ShaderModuleHandle = std::ptr::null_mut();
        let rc = unsafe { (fns.create_shader_module)(device, &info, std::ptr::null(), &mut m) };
        assert_eq!(rc, 0, "vkCreateShaderModule {rc}");
        m
    };
    let vs = mk(&vs_bytes);
    let fs = mk(&fs_bytes);

    let entry = c"main";
    let stages = [
        vk::PipelineShaderStageCreateInfo {
            s_type: 18,
            p_next: std::ptr::null(),
            flags: 0,
            stage: 1,
            module: vs,
            p_name: entry.as_ptr(),
            p_specialization_info: std::ptr::null(),
        },
        vk::PipelineShaderStageCreateInfo {
            s_type: 18,
            p_next: std::ptr::null(),
            flags: 0,
            stage: 16,
            module: fs,
            p_name: entry.as_ptr(),
            p_specialization_info: std::ptr::null(),
        },
    ];
    let vertex_input = vk::PipelineVertexInputStateCreateInfo {
        s_type: 19,
        p_next: std::ptr::null(),
        flags: 0,
        vertex_binding_description_count: 0,
        p_vertex_binding_descriptions: std::ptr::null(),
        vertex_attribute_description_count: 0,
        p_vertex_attribute_descriptions: std::ptr::null(),
    };
    let ia = vk::PipelineInputAssemblyStateCreateInfo {
        s_type: 20,
        p_next: std::ptr::null(),
        flags: 0,
        topology: 3,
        primitive_restart_enable: 0,
    };
    let viewport = vk::Viewport {
        x: 0.0,
        y: 0.0,
        width: W as f32,
        height: H as f32,
        min_depth: 0.0,
        max_depth: 1.0,
    };
    let scissor = vk::Rect2D {
        offset: vk::Offset2D { x: 0, y: 0 },
        extent: vk::Extent2D { width: W, height: H },
    };
    let vps = vk::PipelineViewportStateCreateInfo {
        s_type: 22,
        p_next: std::ptr::null(),
        flags: 0,
        viewport_count: 1,
        p_viewports: &viewport,
        scissor_count: 1,
        p_scissors: &scissor,
    };
    let raster = vk::PipelineRasterizationStateCreateInfo {
        s_type: 23,
        p_next: std::ptr::null(),
        flags: 0,
        depth_clamp_enable: 0,
        rasterizer_discard_enable: 0,
        polygon_mode: 0,
        cull_mode: 0,
        front_face: 0,
        depth_bias_enable: 0,
        depth_bias_constant_factor: 0.0,
        depth_bias_clamp: 0.0,
        depth_bias_slope_factor: 0.0,
        line_width: 1.0,
    };
    let ms = vk::PipelineMultisampleStateCreateInfo {
        s_type: 24,
        p_next: std::ptr::null(),
        flags: 0,
        rasterization_samples: 1,
        sample_shading_enable: 0,
        min_sample_shading: 1.0,
        p_sample_mask: std::ptr::null(),
        alpha_to_coverage_enable: 0,
        alpha_to_one_enable: 0,
    };
    let blend_att = vk::PipelineColorBlendAttachmentState {
        blend_enable: 0,
        src_color_blend_factor: 1,
        dst_color_blend_factor: 0,
        color_blend_op: 0,
        src_alpha_blend_factor: 1,
        dst_alpha_blend_factor: 0,
        alpha_blend_op: 0,
        color_write_mask: 0xf,
    };
    let blend = vk::PipelineColorBlendStateCreateInfo {
        s_type: 26,
        p_next: std::ptr::null(),
        flags: 0,
        logic_op_enable: 0,
        logic_op: 3,
        attachment_count: 1,
        p_attachments: &blend_att,
        blend_constants: [0.0; 4],
    };
    let gpi = vk::GraphicsPipelineCreateInfo {
        s_type: 28,
        p_next: std::ptr::null(),
        flags: 0,
        stage_count: 2,
        p_stages: stages.as_ptr(),
        p_vertex_input_state: &vertex_input,
        p_input_assembly_state: &ia,
        p_tessellation_state: std::ptr::null(),
        p_viewport_state: &vps,
        p_rasterization_state: &raster,
        p_multisample_state: &ms,
        p_depth_stencil_state: std::ptr::null(),
        p_color_blend_state: &blend,
        p_dynamic_state: std::ptr::null(),
        layout: pl,
        render_pass,
        subpass: 0,
        base_pipeline_handle: std::ptr::null_mut(),
        base_pipeline_index: -1,
    };
    let mut pipeline: vk::PipelineHandle = std::ptr::null_mut();
    let rc = unsafe {
        (fns.create_graphics_pipelines)(
            device,
            std::ptr::null_mut(),
            1,
            &gpi,
            std::ptr::null(),
            &mut pipeline,
        )
    };
    assert_eq!(rc, 0, "vkCreateGraphicsPipelines {rc}");
    assert!(!pipeline.is_null(), "管线句柄为空");
    println!("图形管线 ✅（手写，静态 viewport，混合关）");

    // ── 命令池 + 命令缓冲 ──
    let cpi = vk::CommandPoolCreateInfo {
        s_type: 39,
        p_next: std::ptr::null(),
        flags: 0,
        queue_family_index: family,
    };
    let mut pool: vk::CommandPoolHandle = std::ptr::null_mut();
    let rc = unsafe { (fns.create_command_pool)(device, &cpi, std::ptr::null(), &mut pool) };
    assert_eq!(rc, 0, "vkCreateCommandPool {rc}");
    let cbai = vk::CommandBufferAllocateInfo {
        s_type: 40,
        p_next: std::ptr::null(),
        command_pool: pool,
        level: 0,
        command_buffer_count: 1,
    };
    let mut cmd: vk::CommandBufferHandle = std::ptr::null_mut();
    let rc = unsafe { (fns.allocate_command_buffers)(device, &cbai, &mut cmd) };
    assert_eq!(rc, 0, "vkAllocateCommandBuffers {rc}");

    let begin = vk::CommandBufferBeginInfo {
        s_type: 42,
        p_next: std::ptr::null(),
        flags: 1,
        p_inheritance_info: std::ptr::null(),
    };
    let rc = unsafe { (fns.begin_command_buffer)(cmd, &begin) };
    assert_eq!(rc, 0, "vkBeginCommandBuffer {rc}");

    let clear = vk::ClearValue {
        color: vk::ClearColorValue { float32: [1.0, 0.0, 0.0, 1.0] },
    };
    let rpb = vk::RenderPassBeginInfo {
        s_type: 43,
        p_next: std::ptr::null(),
        render_pass,
        framebuffer: fb,
        render_area: scissor,
        clear_value_count: 1,
        p_clear_values: &clear,
    };
    unsafe {
        (fns.cmd_begin_render_pass)(cmd, &rpb, 0);
        (fns.cmd_bind_pipeline)(cmd, 0, pipeline);
        (fns.cmd_set_viewport)(cmd, 0, 1, &viewport);
        (fns.cmd_set_scissor)(cmd, 0, 1, &scissor);
        (fns.cmd_draw)(cmd, 3, 1, 0, 0);
        (fns.cmd_end_render_pass)(cmd);
    }
    println!("渲染通道录制完成（清屏 + 绘制）");

    // ── 暂存缓冲 ──
    let bytes = (W as u64) * (H as u64) * 4;
    let bi = vk::BufferCreateInfo {
        s_type: 12,
        p_next: std::ptr::null(),
        flags: 0,
        size: bytes,
        usage: 2,
        sharing_mode: 0,
        queue_family_index_count: 0,
        p_queue_family_indices: std::ptr::null(),
    };
    let mut staging: vk::BufferHandle = std::ptr::null_mut();
    let rc = unsafe { (fns.create_buffer)(device, &bi, std::ptr::null(), &mut staging) };
    assert_eq!(rc, 0, "vkCreateBuffer {rc}");
    let mut breq = std::mem::MaybeUninit::<vk::MemoryRequirements>::uninit();
    unsafe { (fns.get_buffer_memory_requirements)(device, staging, breq.as_mut_ptr()) };
    let breq = unsafe { breq.assume_init() };
    let host_idx = pick(&mem_props, breq.memory_type_bits, 0x2 | 0x4).expect("HOST_VISIBLE|COHERENT");
    let mut stage_mem: vk::DeviceMemoryHandle = std::ptr::null_mut();
    let ai2 = vk::MemoryAllocateInfo {
        s_type: vk::VK_STRUCTURE_TYPE_MEMORY_ALLOCATE_INFO,
        p_next: std::ptr::null(),
        allocation_size: breq.size,
        memory_type_index: host_idx,
    };
    let rc = unsafe { (fns.allocate_memory)(device, &ai2, std::ptr::null(), &mut stage_mem) };
    assert_eq!(rc, 0, "vkAllocateMemory(staging) {rc}");
    let rc = unsafe { (fns.bind_buffer_memory)(device, staging, stage_mem, 0) };
    assert_eq!(rc, 0, "vkBindBufferMemory {rc}");

    // ── 同一次录制里接上屏障 + 拷贝（**与库里的做法一致**）──
    // 一开始我把它们录进了第二个命令缓冲、只提交了第二个 ⇒ 图像从未被渲染、
    // 回读到全 0。修正后这条路径才是有效的对照实验。
    let barrier = vk::ImageMemoryBarrier {
        s_type: 45,
        p_next: std::ptr::null(),
        src_access_mask: 1 << 8,
        dst_access_mask: 1 << 7,
        old_layout: 6, // 渲染通道的 finalLayout 已经是 TRANSFER_SRC_OPTIMAL
        new_layout: 6,
        src_queue_family_index: u32::MAX,
        dst_queue_family_index: u32::MAX,
        image,
        subresource_range: vk::ImageSubresourceRange {
            aspect_mask: 1,
            base_mip_level: 0,
            level_count: 1,
            base_array_layer: 0,
            layer_count: 1,
        },
    };
    let copy = vk::BufferImageCopy {
        buffer_offset: 0,
        buffer_row_length: 0,
        buffer_image_height: 0,
        image_subresource: vk::ImageSubresourceLayers {
            aspect_mask: 1,
            mip_level: 0,
            base_array_layer: 0,
            layer_count: 1,
        },
        image_offset: vk::Offset3D { x: 0, y: 0, z: 0 },
        image_extent: vk::Extent3D {
            width: W,
            height: H,
            depth: 1,
        },
    };
    unsafe {
        (fns.cmd_pipeline_barrier)(
            cmd,
            1 << 10,
            1 << 12,
            0,
            0,
            std::ptr::null(),
            0,
            std::ptr::null(),
            1,
            &barrier,
        );
        (fns.cmd_copy_image_to_buffer)(cmd, image, 6, staging, 1, &copy);
    }
    let rc = unsafe { (fns.end_command_buffer)(cmd) };
    assert_eq!(rc, 0, "vkEndCommandBuffer {rc}");
    println!("命令缓冲录制 ✅（清屏 + 绘制 + 屏障 + 拷贝，一次提交）");

    // ── 提交 + 栅栏 ──
    let fi = vk::FenceCreateInfo {
        s_type: 8,
        p_next: std::ptr::null(),
        flags: 0,
    };
    let mut fence: vk::FenceHandle = std::ptr::null_mut();
    let rc = unsafe { (fns.create_fence)(device, &fi, std::ptr::null(), &mut fence) };
    assert_eq!(rc, 0, "vkCreateFence {rc}");
    let si = vk::SubmitInfo {
        s_type: 4,
        p_next: std::ptr::null(),
        wait_semaphore_count: 0,
        p_wait_semaphores: std::ptr::null(),
        p_wait_dst_stage_mask: std::ptr::null(),
        command_buffer_count: 1,
        p_command_buffers: &cmd,
        signal_semaphore_count: 0,
        p_signal_semaphores: std::ptr::null(),
    };
    let rc = unsafe { (fns.queue_submit)(queue, 1, &si, fence) };
    assert_eq!(rc, 0, "vkQueueSubmit {rc}");
    let rc = unsafe { (fns.wait_for_fences)(device, 1, &fence, 1, 1_000_000_000) };
    assert_eq!(rc, 0, "vkWaitForFences {rc}");
    println!("提交 + 栅栏 ✅");

    // ── 读回 ──
    let mut mapped: *mut std::ffi::c_void = std::ptr::null_mut();
    let rc = unsafe { (fns.map_memory)(device, stage_mem, 0, u64::MAX, 0, &mut mapped) };
    assert_eq!(rc, 0, "vkMapMemory {rc}");
    let pixels = unsafe { std::slice::from_raw_parts(mapped as *const u8, bytes as usize).to_vec() };
    unsafe { (fns.unmap_memory)(device, stage_mem) };

    let mut counts = std::collections::BTreeMap::new();
    for p in pixels.chunks_exact(4) {
        *counts.entry([p[0], p[1], p[2], p[3]]).or_insert(0usize) += 1;
    }
    println!("\n颜色统计：");
    for (c, n) in &counts {
        println!("    {c:?} × {n}");
    }
    let green = counts.get(&[0u8, 255, 0, 255]).copied().unwrap_or(0);
    let red = counts.get(&[255u8, 0, 0, 255]).copied().unwrap_or(0);
    println!("\n红（清屏）{red} 个，绿（绘制）{green} 个");

    // ── 判据 1：清屏必须铺满整屏，且三角形必须真的被画出来 ──
    assert_eq!(
        red + green,
        (W * H) as usize,
        "画面应当只由「清屏色 + 三角形色」组成（说明没有奇怪的第三色）"
    );
    assert!(
        green > 0,
        "**`vkCmdDraw` 没有产生任何像素** —— 这是 M2a 期间那个缺陷的回归！\
         根因是 SPIR-V 段序（见 crates/deer-vk/src/spirv.rs 的 Section 文档）"
    );

    // ── 判据 2：三角形面积必须合理。
    // 顶点是 (-0.8,-0.8) (0.8,-0.8) (-0.8,0.8) ⇒ 两条直角边各 0.8 屏宽，
    // 直角三角形面积 = 0.8*0.8/2 = 0.32 ⇒ 约 32% 的像素应当是绿色。
    let ratio = green as f64 / (W * H) as f64;
    println!("绿色占比 {:.1}%（理论约 32%）", ratio * 100.0);
    assert!(
        (0.25..0.40).contains(&ratio),
        "绿色占比 {:.1}% 偏离理论值 32% 太多 —— 说明视口/裁剪/顶点位置有问题",
        ratio * 100.0
    );

    // ── 判据 3：三角形在**左上**。
    //
    // 本项目的顶点着色器把 NDC 坐标**原样**写入 `gl_Position`。
    // 而 Vulkan 的 NDC 是 y 向下（与 OpenGL 相反），所以：
    //   顶点 y = -0.8 ⇒ 屏幕上方；y = +0.8 ⇒ 屏幕下方
    // ⇒ 三角形 (-0.8,-0.8) (0.8,-0.8) (-0.8,0.8) 覆盖**左上**象限。
    //
    // （第一版我把这里写成了「左下」，那是套用 OpenGL 的直觉 —— 断言的
    //   期望值必须按**实际 API 约定**写，不能按记忆写。）
    let at = |x: u32, y: u32| -> [u8; 4] {
        let i = ((y as usize) * (W as usize) + (x as usize)) * 4;
        [pixels[i], pixels[i + 1], pixels[i + 2], pixels[i + 3]]
    };
    assert_eq!(
        at(W / 4, H / 8),
        [0, 255, 0, 255],
        "左上象限应当是三角形颜色（NDC 的 y 向下）"
    );
    assert_eq!(
        at(W / 2, (H * 7) / 8),
        [255, 0, 0, 255],
        "下半屏应当是背景色（三角形在左上）"
    );
    assert_eq!(
        at((W * 7) / 8, H / 8),
        [255, 0, 0, 255],
        "右上角应当是背景色（直角三角形不覆盖它）"
    );
    println!("像素位置也正确（三角形在左上象限，符合 Vulkan 的 NDC y 向下）✅");
}

/// 挑一个同时满足所有 `want` 位的可用内存类型。
fn pick(props: &vk::PhysicalDeviceMemoryProperties, bits: u32, want: u32) -> Option<u32> {
    for i in 0..props.memory_type_count.min(32) {
        let mt = &props.memory_types[i as usize];
        if bits & (1 << i) != 0 && mt.property_flags & want == want {
            return Some(i);
        }
    }
    None
}
