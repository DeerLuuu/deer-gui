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

/// 从**源码文本**里解析 `VK_STRUCTURE_TYPE_*` 常量，返回
/// `(解析出的常量, 源码里出现的定义总数)`。
///
/// 第二个返回值是**覆盖率对账**用的（review N-6 的修法）：光断言「解析出 ≥N 个」
/// 抓不住「少解析了几个」—— 有人把某个定义折成两行（rustfmt 对超长 sType 名就会这么
/// 做）时，**行式**解析会静默漏掉它，而测试仍然绿、零信号。这里改成
/// 「源码里 `pub const VK_STRUCTURE_TYPE_` 的出现次数」与「成功解析出的条数」
/// 对账，两者不等就报出**漏掉的名字**。
///
/// 实现要点：
/// - **先去注释**：`//` 后的整段（含文档注释里的示例）会被丢掉，避免把说明文字里的
///   `pub const VK_STRUCTURE_TYPE_…` 当成真定义，也避免注释里的 `;` 截断定义。
/// - **按出现位置往后找 `=` 与 `;`**（跨行），所以折行的定义也能解析。
/// - 值必须是**字面量**（可带 `_` 分隔符）；本项目全部是字面量，若将来出现
///   `= OTHER + 1` 这类表达式，本函数会把它算进「解析失败」，从而由对账把它暴露出来
///   （而不是静默漏检）。
///
/// 用 `include_str!` 而不是运行期读文件：让「源码内容」成为**编译期依赖** ——
/// 改了常量定义就会重新编译本测试，不会出现「测试跑的是旧文件」的错觉。
fn parse_structure_type_constants(src: &str) -> (Vec<(String, i32)>, usize) {
    // ① 去掉行注释（本项目没有块注释里的常量定义）
    let mut cleaned = String::with_capacity(src.len());
    for line in src.lines() {
        match line.find("//") {
            Some(i) => cleaned.push_str(&line[..i]),
            None => cleaned.push_str(line),
        }
        cleaned.push('\n');
    }

    const MARK: &str = "pub const VK_STRUCTURE_TYPE_";
    let bytes = cleaned.as_bytes();
    let mut out = Vec::new();
    let mut total = 0usize;
    let mut from = 0usize;

    while let Some(rel) = cleaned[from..].find(MARK) {
        let start = from + rel;
        total += 1;
        from = start + MARK.len();

        // 跳过名字结尾的空白，取到 `:` 为止
        let Some(colon_rel) = cleaned[from..].find(':') else {
            continue;
        };
        let name = cleaned[from..from + colon_rel].trim().to_string();
        let after_colon = from + colon_rel + 1;

        // 值 = `=` 与 `;` 之间（可跨行）
        let Some(eq_rel) = cleaned[after_colon..].find('=') else {
            continue;
        };
        let after_eq = after_colon + eq_rel + 1;
        let Some(semi_rel) = cleaned[after_eq..].find(';') else {
            continue;
        };
        let raw = cleaned[after_eq..after_eq + semi_rel]
            .trim()
            .replace('_', "");
        if let Ok(value) = raw.parse::<i32>() {
            out.push((name, value));
        }
        // 解析失败（非字面量）⇒ 不进 `out`，但 `total` 已计 —— 由对账暴露。
        let _ = bytes;
    }

    (out, total)
}

/// 找出「源码里有定义、但没进解析结果」的名字（**只用于报错信息**）。
///
/// 有它，覆盖率对账失败时会直接说出**是哪几个常量漏了**，而不是只给一个数字差 ——
/// 后者会让人先花时间自己找漏了谁。
fn missing_names(src: &str, parsed: &[(String, i32)]) -> Vec<String> {
    let cleaned: String = src
        .lines()
        .map(|l| match l.find("//") {
            Some(i) => &l[..i],
            None => l,
        })
        .collect::<Vec<_>>()
        .join("\n");
    let mark = "pub const VK_STRUCTURE_TYPE_";
    let mut missing = Vec::new();
    let mut from = 0usize;
    while let Some(rel) = cleaned[from..].find(mark) {
        let s = from + rel + mark.len();
        let e = cleaned[s..].find(':').map(|r| s + r).unwrap_or(s);
        let name = cleaned[s..e].trim().to_string();
        if !parsed.iter().any(|(p, _)| *p == name) {
            missing.push(name);
        }
        from = s;
    }
    missing
}

/// **M3b 常量不得与任何既有 `VK_STRUCTURE_TYPE_*` 撞编号**（review N-3 / N-5 / N-6）。
///
/// 覆盖范围 = `deer-vk` 源码里的**全部** 40 个 sType 常量，分布是
/// `ffi.rs` 3 + `ffi_dev.rs` 34 + `surface.rs` 1 + `swapchain.rs` 2。
///
/// ⚠️ **更正（review N-5）**：本测试初版只 `include_str!` 了 `ffi_dev.rs` 与
/// `spirv.rs` —— 而 `spirv.rs` 里**根本没有** `VK_STRUCTURE_TYPE_*`（它的常量是
/// SPIR-V 的 `OP_*`），所以实际只覆盖 **34/40**，还剩前一条注释里写的
/// 「34 + 5 = 39」也把 M3b 那 5 个**重复计了一次**（34 已含那 5 个）。
/// 现在 include 到 4 个真正含常量的文件，并把数字改成 40。
#[test]
fn m3b_constants_do_not_collide_with_any_existing_constant() {
    // (文件名, 源码, 该文件里的定义数) —— 每个文件都要**点名核对**，
    // 不能只靠「总数够大」（review N-6）。
    let sources = [
        ("ffi.rs", include_str!("../src/ffi.rs"), 3usize),
        ("ffi_dev.rs", include_str!("../src/ffi_dev.rs"), 34usize),
        ("surface.rs", include_str!("../src/surface.rs"), 1usize),
        ("swapchain.rs", include_str!("../src/swapchain.rs"), 2usize),
    ];

    let mut all: Vec<(String, i32)> = Vec::new();
    for (file, src, expected) in sources {
        let (parsed, total) = parse_structure_type_constants(src);
        assert_eq!(
            total, expected,
            "{file}: 源码里找到 {total} 个 `pub const VK_STRUCTURE_TYPE_*` 定义，\
             预期 {expected} 个 —— 常量增删了就要同步更新本测试的清单，\
             否则「撞车检查」的覆盖范围会悄悄偏离"
        );
        assert_eq!(
            parsed.len(),
            total,
            "{file}: 找到 {total} 个定义，但只成功解析出 {} 个 —— \
             漏掉的是 {:?}。常见原因：定义被折成两行（行式解析会漏）、\
             或值不是字面量（例如 `= OTHER + 1`）。**必须修解析器或显式处理**，\
             否则这条撞车检查会静默漏检。",
            parsed.len(),
            missing_names(src, &parsed)
        );
        all.extend(parsed);
    }

    // ① 覆盖率总账：4 个文件的定义数之和必须等于解析总数
    assert_eq!(
        all.len(),
        40,
        "一共应解析出 40 个 sType 常量（ffi.rs 3 + ffi_dev.rs 34 + surface.rs 1 + swapchain.rs 2），\
         实得 {} —— 数量不符说明覆盖范围偏离",
        all.len()
    );

    // ② 既有常量之间**无重复**（复制粘贴没改常量的典型症状）
    for (i, (name_a, val_a)) in all.iter().enumerate() {
        for (name_b, val_b) in all.iter().skip(i + 1) {
            assert_ne!(
                val_a, val_b,
                "两个不同的 sType 常量共享同一个值：{name_a} 与 {name_b} 都是 {val_a} \
                 —— 典型原因是复制粘贴没改常量"
            );
        }
    }

    // ③ **点名核对**那些「最容易撞、也最容易被解析漏掉」的既有常量（review N-5）：
    //    最低两个编号（0/1）与三个 KHR/EXT 编号。它们此前**不在** include 范围内，
    //    所以「与 0/1 或 KHR/EXT 编号撞车」这种情形根本没被检查过。
    for (name, value) in [
        ("APPLICATION_INFO", 0),
        ("INSTANCE_CREATE_INFO", 1),
        ("DEBUG_UTILS_MESSENGER_CREATE_INFO_EXT", 1_000_128_004),
        ("WIN32_SURFACE_CREATE_INFO_KHR", 1_000_009_000),
        ("SWAPCHAIN_CREATE_INFO_KHR", 1_000_001_000),
        ("PRESENT_INFO_KHR", 1_000_001_001),
    ] {
        let hit = all
            .iter()
            .find(|(n, _)| n == name)
            .unwrap_or_else(|| {
                panic!(
                    "源码里找不到 VK_STRUCTURE_TYPE_{name} —— 它此前不在 include 范围内，\
                     正是「与 0/1 或 KHR/EXT 编号撞车」未被检查的原因（review N-5）"
                )
            });
        assert_eq!(hit.1, value, "VK_STRUCTURE_TYPE_{name} 的解析值与预期不符");
    }

    // ④ M3b 的 5 个常量在全量集合里名字与值都对得上
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
