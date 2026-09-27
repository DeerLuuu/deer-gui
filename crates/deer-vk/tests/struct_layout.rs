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

/// **M3b：采样器 + 描述符结构体**（review M-6：这些此前「实现有、护栏没有」）。
///
/// 护栏缺口的性质：reviewer 已逐字段手工核过下面这些大小/偏移**全对**，
/// 但它们没进本文件 ⇒ 将来改字段类型或顺序时**没有人拦** —— 而手写 `repr(C)`
/// 结构体写错的症状是驱动按官方大小写入越界（`0xc0000409`/`0xc0000005`），
/// 不是一句清晰的错误码（见本文件开头的说明）。
///
/// 每个数字都注明**它由什么决定**。含指针的结构体 `align_of = 8`，
/// 所以「字段和」常小于 `size_of`。
#[test]
fn sampler_and_descriptor_structs_match_measured_sizes() {
    // i32(4) + pad(4) + pNext/pAllocator 形态的指针(8) + flags(4)
    // + 2×filter(8) + mipmap(4) + 3×address(12) + mipLodBias f32(4)
    // + anisotropyEnable(4) + maxAnisotropy f32(4) + compareEnable(4) + compareOp(4)
    // + minLod f32(4) + maxLod f32(4) + borderColor(4) + unnormalizedCoordinates(4) = 80
    assert_eq!(
        std::mem::size_of::<vk::SamplerCreateInfo>(),
        80,
        "sType(4)+pad(4)+2 指针(16)+flags..compareOp(44)+lod/border/unnorm(16) = 80"
    );

    // binding(4) + descriptorType(4) + descriptorCount(4) + stageFlags(4) + pImmutableSamplers(8) = 24
    assert_eq!(std::mem::size_of::<vk::DescriptorSetLayoutBinding>(), 24);
    // sType(4)+pad(4)+pNext(8)+flags(4)+bindingCount(4)+pBindings(8) = 32（**正好 8 的倍数**，
    // 没有尾部补齐 —— 我第一版把它和 DescriptorPoolCreateInfo 搞混了，写成 40，实测红）
    assert_eq!(
        std::mem::size_of::<vk::DescriptorSetLayoutCreateInfo>(),
        32,
        "sType(4)+pad(4)+pNext(8)+flags(4)+bindingCount(4)+pBindings(8) = 32（无需补齐）"
    );
    assert_eq!(std::mem::size_of::<vk::DescriptorPoolSize>(), 8, "2 个 u32");
    // sType(4)+pad(4)+pNext(8)+flags(4)+maxSets(4)+poolSizeCount(4)+pad(4)+pPoolSizes(8) = 40
    assert_eq!(
        std::mem::size_of::<vk::DescriptorPoolCreateInfo>(),
        40,
        "字段和 36，但 pPoolSizes 要求 8 字节对齐 ⇒ 中间补 4、总 40"
    );
    // sType(4)+pad(4)+pNext(8)+pool(8)+count(4)+pad(4)+pSetLayouts(8) = 40
    assert_eq!(
        std::mem::size_of::<vk::DescriptorSetAllocateInfo>(),
        40,
        "字段和 36，descriptorSetCount 后补 4 ⇒ 40"
    );
    assert_eq!(
        std::mem::size_of::<vk::DescriptorImageInfo>(),
        24,
        "sampler(8) + imageView(8) + imageLayout(4) + pad(4) = 24"
    );
    // sType(4)+pad(4)+pNext(8)+dstSet(8)+dstBinding(4)+dstArrayElement(4)+descriptorCount(4)
    // +descriptorType(4)+3 指针(24) = 64
    assert_eq!(
        std::mem::size_of::<vk::WriteDescriptorSet>(),
        64,
        "前 3 个 u32 连着排，然后 descriptorType 才是指针前的最后一个 4 字节字段"
    );
}

/// **M3b：关键偏移**（只看大小不够 —— 字段顺序错时大小可能仍对，而驱动会读到别的字段）。
#[test]
fn sampler_and_descriptor_offsets_are_stable() {
    // pNext 必须在 sType 之后（8 字节对齐）；`flags` 紧贴 pNext 之后
    assert_eq!(std::mem::offset_of!(vk::SamplerCreateInfo, p_next), 8);
    assert_eq!(std::mem::offset_of!(vk::SamplerCreateInfo, flags), 16);
    assert_eq!(
        std::mem::offset_of!(vk::SamplerCreateInfo, mag_filter),
        20,
        "flags(16..20) 之后才是 magFilter —— 我第一版漏了 flags 写成 16，实测红"
    );
    assert_eq!(std::mem::offset_of!(vk::SamplerCreateInfo, min_filter), 24);
    assert_eq!(std::mem::offset_of!(vk::SamplerCreateInfo, mipmap_mode), 28);
    assert_eq!(std::mem::offset_of!(vk::SamplerCreateInfo, address_mode_u), 32);
    assert_eq!(std::mem::offset_of!(vk::SamplerCreateInfo, address_mode_v), 36);
    assert_eq!(std::mem::offset_of!(vk::SamplerCreateInfo, address_mode_w), 40);
    assert_eq!(std::mem::offset_of!(vk::SamplerCreateInfo, mip_lod_bias), 44);
    assert_eq!(std::mem::offset_of!(vk::SamplerCreateInfo, anisotropy_enable), 48);
    assert_eq!(std::mem::offset_of!(vk::SamplerCreateInfo, max_anisotropy), 52);
    assert_eq!(std::mem::offset_of!(vk::SamplerCreateInfo, compare_enable), 56);
    assert_eq!(
        std::mem::offset_of!(vk::SamplerCreateInfo, compare_op),
        60,
        "compareOp 必须在 compareEnable **之后**（顺序错不会编译报错）"
    );
    assert_eq!(std::mem::offset_of!(vk::SamplerCreateInfo, min_lod), 64);
    assert_eq!(std::mem::offset_of!(vk::SamplerCreateInfo, max_lod), 68);
    assert_eq!(std::mem::offset_of!(vk::SamplerCreateInfo, border_color), 72);
    assert_eq!(
        std::mem::offset_of!(vk::SamplerCreateInfo, unnormalized_coordinates),
        76,
        "最后一个字段在 76..80 ⇒ 结构体正好 80、无需尾部补齐"
    );

    // 描述符集绑定：`pImmutableSamplers` 被前面 4 个 u32 顶到偏移 16
    assert_eq!(std::mem::offset_of!(vk::DescriptorSetLayoutBinding, descriptor_type), 4);
    assert_eq!(std::mem::offset_of!(vk::DescriptorSetLayoutBinding, descriptor_count), 8);
    assert_eq!(std::mem::offset_of!(vk::DescriptorSetLayoutBinding, stage_flags), 12);
    assert_eq!(
        std::mem::offset_of!(vk::DescriptorSetLayoutBinding, p_immutable_samplers),
        16
    );

    // 池创建：flags 紧贴 pNext 之后（注意 sType+pNext 占了 16 字节，不是 12）
    assert_eq!(std::mem::offset_of!(vk::DescriptorPoolCreateInfo, flags), 16);
    assert_eq!(std::mem::offset_of!(vk::DescriptorPoolCreateInfo, max_sets), 20);
    assert_eq!(
        std::mem::offset_of!(vk::DescriptorPoolCreateInfo, pool_size_count),
        24,
        "flags/maxSets/poolSizeCount 连着排在 16/20/24"
    );
    assert_eq!(
        std::mem::offset_of!(vk::DescriptorPoolCreateInfo, p_pool_sizes),
        32,
        "poolSizeCount(24..28) 后有 4 字节 pad ⇒ 指针在 32（少算这个 pad 会让指针错位 4 字节）"
    );

    // 集分配：`pSetLayouts` 同理
    assert_eq!(std::mem::offset_of!(vk::DescriptorSetAllocateInfo, descriptor_pool), 16);
    assert_eq!(std::mem::offset_of!(vk::DescriptorSetAllocateInfo, descriptor_set_count), 24);
    assert_eq!(
        std::mem::offset_of!(vk::DescriptorSetAllocateInfo, p_set_layouts),
        32,
        "descriptorSetCount 后有 4 字节 pad ⇒ 指针在 32"
    );

    // 图像信息：sampler / imageView（两个 8 字节句柄）之后才是 imageLayout
    assert_eq!(std::mem::offset_of!(vk::DescriptorImageInfo, sampler), 0);
    assert_eq!(std::mem::offset_of!(vk::DescriptorImageInfo, image_view), 8);
    assert_eq!(std::mem::offset_of!(vk::DescriptorImageInfo, image_layout), 16);

    // 写描述符：三个指针必须连续排在 64 字节结构体的末尾三分之一
    assert_eq!(std::mem::offset_of!(vk::WriteDescriptorSet, dst_binding), 24);
    assert_eq!(std::mem::offset_of!(vk::WriteDescriptorSet, dst_array_element), 28);
    assert_eq!(std::mem::offset_of!(vk::WriteDescriptorSet, descriptor_count), 32);
    assert_eq!(
        std::mem::offset_of!(vk::WriteDescriptorSet, descriptor_type),
        36,
        "descriptorType 在 descriptorCount 之后、pImageInfo **之前**"
    );
    assert_eq!(std::mem::offset_of!(vk::WriteDescriptorSet, p_image_info), 40);
    assert_eq!(std::mem::offset_of!(vk::WriteDescriptorSet, p_buffer_info), 48);
    assert_eq!(std::mem::offset_of!(vk::WriteDescriptorSet, p_texel_buffer_view), 56);
}

/// **M3b：新结构体的 `sType` 常量值必须与 Vulkan 规范一致**。
///
/// 为什么 `sType` 值得单独一组断言：它是驱动**分派结构体类型**的唯一依据 ——
/// 写错（例如把 `DESCRIPTOR_POOL_CREATE_INFO`(33) 当成 `DESCRIPTOR_SET_LAYOUT_
/// CREATE_INFO`(32)）时结构体**大小可能刚好也对**，于是编译过、运行也不再是
/// 「立刻报错」，而是驱动按错误的布局解读内存。这类错误极难从症状反推。
///
/// 这里同时断言**常量之间的相对关系**（例如 SET_LAYOUT < POOL < POOL_SIZE <
/// SET_ALLOCATE < WRITE），这样即使有人整体挪动编号也能被拦住。
#[test]
fn m3b_structure_type_constants_match_spec() {
    assert_eq!(vk::VK_STRUCTURE_TYPE_SAMPLER_CREATE_INFO, 31);
    assert_eq!(vk::VK_STRUCTURE_TYPE_DESCRIPTOR_SET_LAYOUT_CREATE_INFO, 32);
    assert_eq!(vk::VK_STRUCTURE_TYPE_DESCRIPTOR_POOL_CREATE_INFO, 33);
    assert_eq!(vk::VK_STRUCTURE_TYPE_DESCRIPTOR_SET_ALLOCATE_INFO, 34);
    assert_eq!(vk::VK_STRUCTURE_TYPE_WRITE_DESCRIPTOR_SET, 35);

    // 相对关系（顺序即规范里的编号顺序）。
    // 用 `const { assert!(..) }` 而不是运行期 `assert!`：两边都是常量，
    // 于是这些断言在**编译期**就成立 —— 编号一旦被改乱，编译就失败，
    // 而不是等到跑测试（clippy 的 `assertions_on_constants` 也正是指这一点）。
    const {
        assert!(
            vk::VK_STRUCTURE_TYPE_SAMPLER_CREATE_INFO
                < vk::VK_STRUCTURE_TYPE_DESCRIPTOR_SET_LAYOUT_CREATE_INFO
        );
        assert!(
            vk::VK_STRUCTURE_TYPE_DESCRIPTOR_SET_LAYOUT_CREATE_INFO
                < vk::VK_STRUCTURE_TYPE_DESCRIPTOR_POOL_CREATE_INFO
        );
        assert!(
            vk::VK_STRUCTURE_TYPE_DESCRIPTOR_POOL_CREATE_INFO
                < vk::VK_STRUCTURE_TYPE_DESCRIPTOR_SET_ALLOCATE_INFO
        );
        assert!(
            vk::VK_STRUCTURE_TYPE_DESCRIPTOR_SET_ALLOCATE_INFO
                < vk::VK_STRUCTURE_TYPE_WRITE_DESCRIPTOR_SET
        );
    }

    // 与**全部**既有常量比对编号是否撞车 —— 见下面 `m3b_constants_do_not_collide_with_any_existing_constant`。
    // 原来这里内联了一小张「既有常量」名单（只挑 5 个比），reviewer 实测那个写法
    // **抓不住**把编号改成 37/38/39 这类撞车（名单里正好没有这几个值）。
    // 现在改成**从源码里解析出全部常量**，见下一条测试。
}

/// 直接从**源码文本**里解析出所有 `VK_STRUCTURE_TYPE_*` 常量，返回 `(名字, 值)`。
///
/// 为什么解析源码而不是维护一张硬编码名单（review N-3 的修法）：
/// 硬编码名单会**悄悄过期** —— 有人加了新常量但没更新名单，护栏就漏了；
/// 而「护栏漏了」这件事本身没有任何信号（测试照样绿）。解析源码时，
/// 新常量自动进入比对范围，且 [#\[test\] `m3b_constants...`] 里那条
/// 「既有常量必须无重复」的自检能反过来证明解析确实抓到了完整清单。
///
/// 用 `include_str!` 而不是运行期读文件：让「源码内容」成为**编译期依赖** ——
/// 改了 `ffi_dev.rs` / `spirv.rs` 常量定义就会重新编译本测试，不会出现
/// 「测试跑的是旧文件」的错觉。
fn parse_structure_type_constants(src: &str) -> Vec<(String, i32)> {
    let mut out = Vec::new();
    for line in src.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix("pub const VK_STRUCTURE_TYPE_") else {
            continue;
        };
        let Some((name, rest)) = rest.split_once(':') else {
            continue;
        };
        let Some((_, value)) = rest.split_once('=') else {
            continue;
        };
        let value = value.trim().trim_end_matches(';').trim();
        let Ok(value) = value.parse::<i32>() else {
            // 非字面量（例如 `= SOMETHING + 1`）—— 本项目里没有，出现时应当显式处理，
            // 所以这里静默跳过并在下面用「解析数量」断言兜住（见测试里的 >= 30）。
            continue;
        };
        out.push((name.trim().to_string(), value));
    }
    out
}

/// **M3b 常量不得与任何既有 `VK_STRUCTURE_TYPE_*` 撞编号**（review N-3）。
///
/// 覆盖范围 = **源码里的全部常量**（当前 34 个 + M3b 的 5 个 = 39 个），
/// 而不是先前那张 5 个的手写名单。
#[test]
fn m3b_constants_do_not_collide_with_any_existing_constant() {
    let mut all = parse_structure_type_constants(include_str!("../src/ffi_dev.rs"));
    all.extend(parse_structure_type_constants(include_str!("../src/spirv.rs")));

    // ① 解析器自检：必须抓到足够多的常量，否则「零撞车」是假的
    //    （解析失败会静默返回空列表 ⇒ 下面所有断言都平凡通过）
    assert!(
        all.len() >= 30,
        "只从源码解析出 {} 个 VK_STRUCTURE_TYPE_* 常量 —— 解析器或源码结构变了，\
         请先修解析器（否则这条最全撞车检查会静默失效）",
        all.len()
    );

    // ② 既有常量之间也必须**无重复**：这同时证明了「解析确实抓到了完整清单」——
    //    若解析漏掉了某些定义，这里未必能发现，但若两个不同名字共享同一个值，
    //    强烈暗示某处复制粘贴没改。
    for (i, (name_a, val_a)) in all.iter().enumerate() {
        for (name_b, val_b) in all.iter().skip(i + 1) {
            assert_ne!(
                val_a, val_b,
                "两个不同的 sType 常量共享同一个值：{name_a} 与 {name_b} 都是 {val_a} \
                 —— 典型原因是复制粘贴没改常量（本项目已有 5 个 M3b 常量，容易再犯）"
            );
        }
    }

    // ③ M3b 的 5 个常量在**全量集合**里都能找到一样名字且一样的值
    for (name, value) in [
        ("SAMPLER_CREATE_INFO", vk::VK_STRUCTURE_TYPE_SAMPLER_CREATE_INFO),
        (
            "DESCRIPTOR_SET_LAYOUT_CREATE_INFO",
            vk::VK_STRUCTURE_TYPE_DESCRIPTOR_SET_LAYOUT_CREATE_INFO,
        ),
        (
            "DESCRIPTOR_POOL_CREATE_INFO",
            vk::VK_STRUCTURE_TYPE_DESCRIPTOR_POOL_CREATE_INFO,
        ),
        (
            "DESCRIPTOR_SET_ALLOCATE_INFO",
            vk::VK_STRUCTURE_TYPE_DESCRIPTOR_SET_ALLOCATE_INFO,
        ),
        ("WRITE_DESCRIPTOR_SET", vk::VK_STRUCTURE_TYPE_WRITE_DESCRIPTOR_SET),
    ] {
        let hit = all
            .iter()
            .find(|(n, _)| n == name)
            .unwrap_or_else(|| panic!("源码里找不到 VK_STRUCTURE_TYPE_{name}（解析器漏了？）"));
        assert_eq!(hit.1, value, "VK_STRUCTURE_TYPE_{name} 的解析值与编译值不一致");
    }
}
