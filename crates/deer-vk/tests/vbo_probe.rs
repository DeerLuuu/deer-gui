//! **构造性实验**：走**完全标准**的 Vulkan 顶点路径 —— 真实顶点缓冲 + 顶点属性。
//!
//! 顶点位置来自 `location 0` 的 `vec2` 属性，**不依赖** `gl_VertexIndex`、
//! `OpSelect` 或任何内置变量。
//!
//! 判别：
//! - 若这里**画得出**绿色 ⇒ 问题在「从内置变量算顶点位置」那条路（SPIR-V 生成）；
//! - 若这里**也画不出** ⇒ 问题在更底层（管线状态或我的 Vulkan 用法）。
//!
//! 不做任何销毁：进程退出时驱动清理。诊断代码，不是库代码。

use deer_vk::device::VkDevice;
use deer_vk::ffi_dev as vk;
use deer_vk::spirv;

const W: u32 = 64;
const H: u32 = 64;
const FMT: i32 = vk::VK_FORMAT_R8G8B8A8_UNORM;

/// 三个 NDC 顶点（x, y 各 4 字节，紧密排列 ⇒ stride = 8）
const VERTS: [[f32; 2]; 3] = [[-0.8, -0.8], [0.8, -0.8], [-0.8, 0.8]];

#[test]
fn vertex_buffer_path() {
    let Ok(dev) = VkDevice::open(0) else {
        println!("跳过：没有可用的 Vulkan");
        return;
    };
    let fns = *dev.fns();
    let device = dev.handle();
    let queue = dev.queue();
    let family = dev.queue_family_index();
    let mem_props = *dev.memory_properties();

    // ── 顶点缓冲（DEVICE_LOCAL 或 HOST_VISIBLE 都行；这里直接用 HOST_VISIBLE 免得再拷贝）──
    let vbytes: &[u8] = unsafe {
        std::slice::from_raw_parts(VERTS.as_ptr() as *const u8, std::mem::size_of_val(&VERTS))
    };
    let bi = vk::BufferCreateInfo {
        s_type: 12,
        p_next: std::ptr::null(),
        flags: 0,
        size: vbytes.len() as u64,
        usage: vk::VK_BUFFER_USAGE_VERTEX_BUFFER_BIT,
        sharing_mode: 0,
        queue_family_index_count: 0,
        p_queue_family_indices: std::ptr::null(),
    };
    let mut vbuf: vk::BufferHandle = std::ptr::null_mut();
    // SAFETY: 诊断调用。
    let rc = unsafe { (fns.create_buffer)(device, &bi, std::ptr::null(), &mut vbuf) };
    assert_eq!(rc, 0, "vkCreateBuffer(vertex) {rc}");
    let mut vreq = std::mem::MaybeUninit::<vk::MemoryRequirements>::uninit();
    unsafe { (fns.get_buffer_memory_requirements)(device, vbuf, vreq.as_mut_ptr()) };
    let vreq = unsafe { vreq.assume_init() };
    let vmem_idx = pick(&mem_props, vreq.memory_type_bits, 0x2 | 0x4).expect("HOST_VISIBLE");
    let mut vmem: vk::DeviceMemoryHandle = std::ptr::null_mut();
    let vai = vk::MemoryAllocateInfo {
        s_type: vk::VK_STRUCTURE_TYPE_MEMORY_ALLOCATE_INFO,
        p_next: std::ptr::null(),
        allocation_size: vreq.size,
        memory_type_index: vmem_idx,
    };
    let rc = unsafe { (fns.allocate_memory)(device, &vai, std::ptr::null(), &mut vmem) };
    assert_eq!(rc, 0, "vkAllocateMemory(vertex) {rc}");
    let rc = unsafe { (fns.bind_buffer_memory)(device, vbuf, vmem, 0) };
    assert_eq!(rc, 0, "vkBindBufferMemory(vertex) {rc}");
    // 上传顶点数据
    let mut vmap: *mut std::ffi::c_void = std::ptr::null_mut();
    let rc = unsafe { (fns.map_memory)(device, vmem, 0, u64::MAX, 0, &mut vmap) };
    assert_eq!(rc, 0, "vkMapMemory(vertex) {rc}");
    unsafe { std::ptr::copy_nonoverlapping(vbytes.as_ptr(), vmap as *mut u8, vbytes.len()) };
    unsafe { (fns.unmap_memory)(device, vmem) };
    println!("顶点缓冲 ✅（{} 字节，3 个 vec2）", vbytes.len());

    // ── 离屏图像 ──
    let img = vk::ImageCreateInfo {
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
        sharing_mode: 0,
        queue_family_index_count: 0,
        p_queue_family_indices: std::ptr::null(),
        initial_layout: 0,
    };
    let mut image: vk::ImageHandle = std::ptr::null_mut();
    let rc = unsafe { (fns.create_image)(device, &img, std::ptr::null(), &mut image) };
    assert_eq!(rc, 0, "vkCreateImage {rc}");
    let mut ireq = std::mem::MaybeUninit::<vk::MemoryRequirements>::uninit();
    unsafe { (fns.get_image_memory_requirements)(device, image, ireq.as_mut_ptr()) };
    let ireq = unsafe { ireq.assume_init() };
    let imem_idx = pick(&mem_props, ireq.memory_type_bits, 0x1).expect("DEVICE_LOCAL");
    let mut imem: vk::DeviceMemoryHandle = std::ptr::null_mut();
    let iai = vk::MemoryAllocateInfo {
        s_type: vk::VK_STRUCTURE_TYPE_MEMORY_ALLOCATE_INFO,
        p_next: std::ptr::null(),
        allocation_size: ireq.size,
        memory_type_index: imem_idx,
    };
    let rc = unsafe { (fns.allocate_memory)(device, &iai, std::ptr::null(), &mut imem) };
    assert_eq!(rc, 0, "vkAllocateMemory(image) {rc}");
    let rc = unsafe { (fns.bind_image_memory)(device, image, imem, 0) };
    assert_eq!(rc, 0, "vkBindImageMemory {rc}");

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

    // ── 渲染通道 ──
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
    let vs_bytes = spirv::vertex_shader_from_vertex_buffer();
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

    // ── 顶点输入：1 个 binding（stride 8）+ 1 个属性（location 0，vec2，offset 0）──
    let binding = vk::VertexInputBindingDescription {
        binding: 0,
        stride: 8,
        input_rate: vk::VK_VERTEX_INPUT_RATE_VERTEX,
    };
    let attr = vk::VertexInputAttributeDescription {
        location: 0,
        binding: 0,
        format: vk::VK_FORMAT_R32G32_SFLOAT,
        offset: 0,
    };
    let vertex_input = vk::PipelineVertexInputStateCreateInfo {
        s_type: 19,
        p_next: std::ptr::null(),
        flags: 0,
        vertex_binding_description_count: 1,
        p_vertex_binding_descriptions: &binding,
        vertex_attribute_description_count: 1,
        p_vertex_attribute_descriptions: &attr,
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
    println!("图形管线 ✅（带真实顶点输入）");

    // ── 命令缓冲 ──
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

    // ── 暂存缓冲 ──
    let bytes = (W as u64) * (H as u64) * 4;
    let sbi = vk::BufferCreateInfo {
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
    let rc = unsafe { (fns.create_buffer)(device, &sbi, std::ptr::null(), &mut staging) };
    assert_eq!(rc, 0, "vkCreateBuffer(staging) {rc}");
    let mut sreq = std::mem::MaybeUninit::<vk::MemoryRequirements>::uninit();
    unsafe { (fns.get_buffer_memory_requirements)(device, staging, sreq.as_mut_ptr()) };
    let sreq = unsafe { sreq.assume_init() };
    let smem_idx = pick(&mem_props, sreq.memory_type_bits, 0x2 | 0x4).expect("HOST_VISIBLE");
    let mut smem: vk::DeviceMemoryHandle = std::ptr::null_mut();
    let sai = vk::MemoryAllocateInfo {
        s_type: vk::VK_STRUCTURE_TYPE_MEMORY_ALLOCATE_INFO,
        p_next: std::ptr::null(),
        allocation_size: sreq.size,
        memory_type_index: smem_idx,
    };
    let rc = unsafe { (fns.allocate_memory)(device, &sai, std::ptr::null(), &mut smem) };
    assert_eq!(rc, 0, "vkAllocateMemory(staging) {rc}");
    let rc = unsafe { (fns.bind_buffer_memory)(device, staging, smem, 0) };
    assert_eq!(rc, 0, "vkBindBufferMemory(staging) {rc}");

    // ── 录制：清屏 + **绑定顶点缓冲** + 绘制 + 屏障 + 拷贝 ──
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
        // **关键**：绑定顶点缓冲（binding 0，offset 0）
        let offset: vk::DeviceSize = 0;
        (fns.cmd_bind_vertex_buffers)(cmd, 0, 1, &vbuf, &offset);
        (fns.cmd_draw)(cmd, 3, 1, 0, 0);
        (fns.cmd_end_render_pass)(cmd);
    }

    let barrier = vk::ImageMemoryBarrier {
        s_type: 45,
        p_next: std::ptr::null(),
        src_access_mask: 1 << 8,
        dst_access_mask: 1 << 7,
        old_layout: 6,
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
        image_extent: vk::Extent3D { width: W, height: H, depth: 1 },
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
    println!("录制 ✅（清屏 + 绑顶点缓冲 + 绘制 + 屏障 + 拷贝）");

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

    // ── 读回 ──
    let mut mapped: *mut std::ffi::c_void = std::ptr::null_mut();
    let rc = unsafe { (fns.map_memory)(device, smem, 0, u64::MAX, 0, &mut mapped) };
    assert_eq!(rc, 0, "vkMapMemory {rc}");
    let pixels = unsafe { std::slice::from_raw_parts(mapped as *const u8, bytes as usize).to_vec() };
    unsafe { (fns.unmap_memory)(device, smem) };

    let mut counts = std::collections::BTreeMap::new();
    for p in pixels.chunks_exact(4) {
        *counts.entry([p[0], p[1], p[2], p[3]]).or_insert(0usize) += 1;
    }
    println!("\n颜色统计：");
    for (c, n) in &counts {
        println!("    {c:?} × {n}");
    }
    let green = counts.get(&[0u8, 255, 0, 255]).copied().unwrap_or(0);
    println!("\n绿（绘制）{green} 个");
    assert_eq!(
        green, 0,
        "**绘制缺陷已修复**（标准顶点缓冲路径画出了 {green} 个绿像素）！\
         请把这条断言改成真正的像素校验，并更新文档"
    );
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
