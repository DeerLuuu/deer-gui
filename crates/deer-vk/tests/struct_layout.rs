//! 手写 Vulkan 结构体的**大小与偏移断言**。
//!
//! ## 为什么必须有这一层
//!
//! 手写 `#[repr(C)]` 结构体最大的风险是「比规范小」—— 驱动按官方大小写入就越界，
//! 症状是 `STATUS_STACK_BUFFER_OVERRUN`（0xc0000409）或访问违例（0xc0000005）
//! **而不是**一个清晰的错误码。
//!
//! ## 这里断言的是「实测值」，不是「我算的值」
//!
//! 本项目在第一版把这些数字**手算错了**（`ClearValue` 我写 20、实际 16；
//! `Layout` 偏移我写 152、实际 104 ……），导致 4 条断言假红。
//! 所以现在每个数字都注明**它由什么决定**，并且都是跑出来核过的。
//!
//! **这些断言的作用**：锁住当前布局。以后谁改了字段类型或顺序，这里立刻红
//! —— 而不是等到驱动崩溃。注意 `align_of` 是 8 的结构体，尾部会补到 8 的倍数，
//! 所以「字段和 = 60」的结构体实际是 64。

use deer_vk::ffi_dev as vk;

#[test]
fn base_structs_match_measured_sizes() {
    assert_eq!(std::mem::size_of::<vk::Extent2D>(), 8);
    assert_eq!(std::mem::size_of::<vk::Extent3D>(), 12);
    assert_eq!(std::mem::size_of::<vk::Offset2D>(), 8);
    assert_eq!(std::mem::size_of::<vk::Offset3D>(), 12);
    assert_eq!(std::mem::size_of::<vk::Rect2D>(), 16);
    assert_eq!(std::mem::size_of::<vk::Viewport>(), 24, "6 个 f32");
    assert_eq!(std::mem::size_of::<vk::ClearColorValue>(), 16, "4 个 f32");
    assert_eq!(
        std::mem::size_of::<vk::ClearValue>(),
        16,
        "union(float32[4], depth+stencil) = 16（**不是 20** —— 第一版算错过）"
    );
    assert_eq!(std::mem::size_of::<vk::ImageSubresourceRange>(), 20, "5 个 u32");
    assert_eq!(std::mem::size_of::<vk::ImageSubresourceLayers>(), 16, "4 个 u32");
}

#[test]
fn creation_structs_match_measured_sizes() {
    assert_eq!(std::mem::size_of::<vk::ShaderModuleCreateInfo>(), 40);
    assert_eq!(std::mem::size_of::<vk::DeviceQueueCreateInfo>(), 40);
    assert_eq!(std::mem::size_of::<vk::DeviceCreateInfo>(), 72);
    assert_eq!(std::mem::size_of::<vk::AttachmentDescription>(), 36, "纯 u32/i32，无指针 ⇒ 不补齐");
    assert_eq!(std::mem::size_of::<vk::AttachmentReference>(), 8);
    assert_eq!(std::mem::size_of::<vk::SubpassDependency>(), 28, "7 个 u32，无指针");
    assert_eq!(std::mem::size_of::<vk::MemoryAllocateInfo>(), 32);
    assert_eq!(std::mem::size_of::<vk::ImageCreateInfo>(), 88);
    // 下面这几个**都比「字段和」大** —— 因为它们含指针 ⇒ align_of = 8 ⇒ 尾部补齐
    assert_eq!(
        std::mem::size_of::<vk::BufferCreateInfo>(),
        56,
        "字段和 48，align_of = 8 ⇒ 56"
    );
    assert_eq!(
        std::mem::size_of::<vk::CommandPoolCreateInfo>(),
        24,
        "4+4pad+8+4+4 = 24（不是 16 —— pNext 把它顶到 8 字节对齐）"
    );
    assert_eq!(
        std::mem::size_of::<vk::FramebufferCreateInfo>(),
        64,
        "字段和 52，align_of = 8 ⇒ 64"
    );
    assert_eq!(
        std::mem::size_of::<vk::RenderPassBeginInfo>(),
        64,
        "字段和 56，align_of = 8 ⇒ 64"
    );
    assert_eq!(
        std::mem::size_of::<vk::FenceCreateInfo>(),
        24,
        "4+4pad+8+4+4 = 24（不是 16）"
    );
    assert_eq!(std::mem::size_of::<vk::SubpassDescription>(), 72);
    assert_eq!(std::mem::size_of::<vk::RenderPassCreateInfo>(), 64);
    assert_eq!(std::mem::size_of::<vk::ImageViewCreateInfo>(), 80);
    assert_eq!(std::mem::size_of::<vk::SubmitInfo>(), 72);
    assert_eq!(
        std::mem::size_of::<vk::PhysicalDeviceMemoryProperties>(),
        520,
        "2 个 u32 + 32*8 + u32 + 16*16 = 520"
    );
}

#[test]
fn pipeline_structs_match_measured_sizes() {
    assert_eq!(std::mem::size_of::<vk::PipelineShaderStageCreateInfo>(), 48);
    assert_eq!(std::mem::size_of::<vk::PipelineVertexInputStateCreateInfo>(), 48);
    assert_eq!(
        std::mem::size_of::<vk::PipelineInputAssemblyStateCreateInfo>(),
        32,
        "字段和 28，但 align_of = 8 ⇒ 补到 32（**不是 24**）"
    );
    assert_eq!(std::mem::size_of::<vk::PipelineViewportStateCreateInfo>(), 48);
    assert_eq!(
        std::mem::size_of::<vk::PipelineRasterizationStateCreateInfo>(),
        64,
        "字段和 60，align_of = 8 ⇒ 补到 64（**不是 60**）"
    );
    assert_eq!(std::mem::size_of::<vk::PipelineMultisampleStateCreateInfo>(), 48);
    assert_eq!(std::mem::size_of::<vk::PipelineColorBlendAttachmentState>(), 32);
    assert_eq!(std::mem::size_of::<vk::PipelineColorBlendStateCreateInfo>(), 56);
    assert_eq!(std::mem::size_of::<vk::PipelineDynamicStateCreateInfo>(), 32);
    assert_eq!(std::mem::size_of::<vk::PushConstantRange>(), 12);
    assert_eq!(std::mem::size_of::<vk::PipelineLayoutCreateInfo>(), 48);
    assert_eq!(
        std::mem::size_of::<vk::GraphicsPipelineCreateInfo>(),
        144,
        "14 个指针 + 6 个 u32/i32 + 尾部 u32/i32，按 8 字节对齐"
    );
}

#[test]
fn critical_offsets_are_stable() {
    // 这些偏移决定了驱动会不会读到「别的字段的值」——错了不会编译报错，只会行为诡异
    assert_eq!(std::mem::offset_of!(vk::ShaderModuleCreateInfo, code_size), 24);
    assert_eq!(std::mem::offset_of!(vk::GraphicsPipelineCreateInfo, p_stages), 24);
    assert_eq!(
        std::mem::offset_of!(vk::GraphicsPipelineCreateInfo, layout),
        104,
        "前 13 个字段占 104 字节（**不是 152** —— 第一版算错过）"
    );
    assert_eq!(std::mem::offset_of!(vk::GraphicsPipelineCreateInfo, render_pass), 112);
    assert_eq!(std::mem::offset_of!(vk::GraphicsPipelineCreateInfo, subpass), 120);
    assert_eq!(
        std::mem::offset_of!(vk::GraphicsPipelineCreateInfo, base_pipeline_handle),
        128
    );
    assert_eq!(
        std::mem::offset_of!(vk::GraphicsPipelineCreateInfo, base_pipeline_index),
        136
    );
    assert_eq!(std::mem::offset_of!(vk::PipelineShaderStageCreateInfo, module), 24);
    assert_eq!(std::mem::offset_of!(vk::PipelineShaderStageCreateInfo, p_name), 32);
    assert_eq!(
        std::mem::offset_of!(vk::PipelineInputAssemblyStateCreateInfo, topology),
        20,
        "topology 必须在 primitive_restart_enable **之前**（顺序错不会编译报错）"
    );
}

#[test]
fn alignment_matches_expectation() {
    // 有指针的结构体在 x86_64 上按 8 字节对齐 —— 这是「字段和 ≠ 结构体大小」的原因
    assert_eq!(std::mem::align_of::<vk::GraphicsPipelineCreateInfo>(), 8);
    assert_eq!(std::mem::align_of::<vk::PipelineRasterizationStateCreateInfo>(), 8);
    assert_eq!(std::mem::align_of::<vk::PipelineInputAssemblyStateCreateInfo>(), 8);
}
