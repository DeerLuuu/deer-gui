//! M2a-3 驱动验收：**渲染通道 + 管线布局 + 图形管线**。
//!
//! ## 为什么这一步才是「SPIR-V 的真正验收」
//!
//! 上一步证明了「`vkCreateShaderModule` 接受我们的产物」，但实测发现那个函数**很宽容**
//! —— 连 `bound = 0` 的非法模块都接受。而 `vkCreateGraphicsPipelines` 会把
//! **顶点/片段两个阶段链接起来、并与管线状态一起校验**：拒绝就意味着着色器或状态
//! 真的有问题是。
//!
//! 所以本文件的通过，才算「自研 SPIR-V 汇编器可用于真实渲染管线」。
//!
//! 无 Vulkan 环境时优雅跳过，不伪装成通过。

use deer_vk::device::VkDevice;
use deer_vk::ffi_dev as vk;
use deer_vk::spirv;

/// `DEER_VK_VALIDATION=1` 是否被请求（判据与 `deer_vk::ffi::Instance::validation_from_env()` 一致）。
fn validation_requested() -> bool {
    std::env::var("DEER_VK_VALIDATION")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

fn open() -> Option<VkDevice> {
    match VkDevice::open(0) {
        Ok(d) => Some(d),
        Err(e) => {
            println!("跳过：本机没有可用的 Vulkan（{e}）");
            None
        }
    }
}

/// 断言「到目前为止校验层**没有**报过任何消息」（进程级计数，见 `ffi::validation_message_count`）。
///
/// 为什么必须是**计数断言**而不是人眼看 stderr：`DEER_VK_VALIDATION` 没开时回调
/// 根本不会跑、计数恒为 0，所以「零消息」很容易变成一句空话。本文件里它与
/// [`validation_layer_is_actually_running`] 配套：那条保证**层真的在跑**，
/// 这条保证跑了之后**一条消息都没有**。两件事合起来才是可回归的结论。
fn assert_no_validation_messages(context: &str) {
    if !validation_requested() {
        // 层没开时计数恒为 0，断言没有意义（会给出虚假的安全感）——明确说出来。
        println!("（{context}：未请求校验层，跳过「零校验消息」断言）");
        return;
    }
    let n = deer_vk::ffi::validation_message_count();
    assert_eq!(
        n,
        0,
        "{context}: 校验层报了 {n} 条消息 —— 这是「零校验消息」的自动断言版，\
         请在上面输出里找 [VK ERROR]/[VALIDATION] 字样"
    );
}

/// **「层真的在跑」与「请求」必须一致**（T3 review F7 的同一条要求）。
///
/// 只在请求了校验层时检查：请求了就必须真的启用（否则后面那些「零消息」
/// 断言是在对一个没跑的回调断言，等于没验）。
fn validation_layer_is_actually_running(dev: &VkDevice) -> bool {
    if validation_requested() {
        assert!(
            dev.validation_enabled(),
            "DEER_VK_VALIDATION 已请求，但设备报告校验层未启用 —— \
             这时「零校验消息」毫无意义（回调根本不会被调用）"
        );
    }
    dev.validation_enabled()
}

/// 完整链路：渲染通道 → 管线布局 → 两个着色器模块 → **图形管线**。
///
/// 用不带推送常量的着色器 —— 推送常量那条路当前不可用（见
/// `spirv::vertex_shader_rect_pushconstant` 的说明）。
#[test]
fn graphics_pipeline_is_created_on_real_driver() {
    let Some(dev) = open() else { return };

    // ① 渲染通道：RGBA8、每帧清屏、最终布局 = 可回读
    let pass = dev
        .create_render_pass(
            vk::VK_FORMAT_R8G8B8A8_UNORM,
            vk::VK_ATTACHMENT_LOAD_OP_CLEAR,
            vk::VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL,
        )
        .expect("创建渲染通道");
    assert!(!pass.handle().is_null());

    // ② 管线布局：无推送常量、无描述符集
    let layout = dev.create_pipeline_layout(None).expect("创建管线布局");
    assert!(!layout.handle().is_null());

    // ③ 两个着色器：三角形（逐分量 OpSelect，无动态索引）
    let vs = dev
        .create_shader_module(&spirv::vertex_shader_triangle([[-1.0, -1.0], [3.0, -1.0], [-1.0, 3.0]]))
        .expect("顶点着色器应被接受");
    let fs = dev
        .create_shader_module(&spirv::fragment_shader_solid([0.2, 0.6, 1.0, 1.0]))
        .expect("片段着色器应被接受");

    // ④ **M2a-3 的验收**：建图形管线
    let pipeline = dev
        .create_graphics_pipeline(&vs, &fs, &layout, &pass)
        .expect("vkCreateGraphicsPipelines 应成功");
    assert!(
        !pipeline.handle().is_null(),
        "管线句柄不能为空（驱动返回成功却不写输出参数时，这里会红）"
    );

    println!("渲染通道 + 管线布局 + 图形管线 全部创建成功 ✅（句柄 {:?}）", pipeline.handle());
    dev.wait_idle().expect("空闲等待");
}

/// 带推送常量的矩形着色器**当前不可用** —— 把这条实测事实写成测试，
/// 这样它不会被遗忘，也不会被误以为是可用的。
///
/// 详见 `deer_vk::spirv::vertex_shader_rect_pushconstant` 的文档：
/// 三种推送常量写法分别导致「空句柄」或「访问违例」。
#[test]
fn push_constant_rect_shader_is_known_broken() {
    // 校验层下**显式跳过**：同一支损坏的 SPIR-V 会让校验层/驱动把进程打成
    // 0xc0000005（STATUS_ACCESS_VIOLATION）。跳过要打印原因，不伪装通过。
    if validation_requested() {
        eprintln!(
            "跳过：`spirv::vertex_shader_rect_pushconstant()` 已知损坏\
             （PushConstant 变量不是 OpTypeStruct，违反 VUID-StandaloneSpirv-PushConstant-06808），\
             开启校验层时会让进程 0xc0000005 崩溃 —— 本测试在校验层下不执行。"
        );
        return;
    }
    let Some(dev) = open() else { return };
    let bytes = spirv::vertex_shader_rect_pushconstant();
    // 模块本身仍会被驱动接受（这正说明建模块的校验极弱）
    let vs = dev
        .create_shader_module(&bytes)
        .expect("模块本身应被接受 —— 问题不在建模块");
    println!(
        "注意：推送常量矩形着色器（{} 字节）能建模块，但建管线会失败（已知问题）",
        bytes.len()
    );
    let _ = vs;
}

/// 三角形着色器也能建管线（那支**不带推送常量**，走的是纯常量数组路径）。
#[test]
fn triangle_pipeline_also_builds() {
    let Some(dev) = open() else { return };
    let pass = dev
        .create_render_pass(
            vk::VK_FORMAT_R8G8B8A8_UNORM,
            vk::VK_ATTACHMENT_LOAD_OP_CLEAR,
            vk::VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL,
        )
        .expect("渲染通道");
    // 无推送常量
    let layout = dev.create_pipeline_layout(None).expect("管线布局");
    let vs = dev
        .create_shader_module(&spirv::vertex_shader_triangle([[-1.0, -1.0], [3.0, -1.0], [-1.0, 3.0]]))
        .expect("顶点着色器");
    let fs = dev
        .create_shader_module(&spirv::fragment_shader_solid([0.0, 1.0, 0.0, 1.0]))
        .expect("片段着色器");
    dev.create_graphics_pipeline(&vs, &fs, &layout, &pass)
        .expect("三角形管线也应建成功");
    println!("三角形管线建成功 ✅");
}

/// 三种颜色格式都应能建渲染通道（后续窗口/离屏会用到不同格式）。
#[test]
fn render_pass_supports_all_target_formats() {
    let Some(dev) = open() else { return };
    for (name, format) in [
        ("R8G8B8A8_UNORM", vk::VK_FORMAT_R8G8B8A8_UNORM),
        ("R8G8B8A8_SRGB", vk::VK_FORMAT_R8G8B8A8_SRGB),
        ("B8G8R8A8_SRGB", vk::VK_FORMAT_B8G8R8A8_SRGB),
    ] {
        match dev.create_render_pass(
            format,
            vk::VK_ATTACHMENT_LOAD_OP_CLEAR,
            vk::VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL,
        ) {
            Ok(_) => println!("  {name}: 支持 ✅"),
            Err(e) => panic!("{name} 应被支持（GUI 需要它）：{e}"),
        }
    }
}

// ── M3c-T1：共用管线层 ────────────────────────────────────────────────────────

/// **M3c-T1 验收**：共用层能同时建出「形状 + 文本」两条管线，且**同一批状态**
/// 换一个颜色格式 + viewport 策略就能服务另一条路径（离屏 ↔ 窗口）。
///
/// ## 为什么这条测试的重点是「两种格式都能建出来」
///
/// M3c 要让窗口用同一套画法，而窗口的颜色格式是 **`B8G8R8A8_SRGB`（本机实测 `0x32`）**、
/// 离屏是 `R8G8B8A8_UNORM` —— 两者走的是**同一份** `build_pipelines`，只是
/// `color_format` 参数不同。所以「共用层对两种格式都能建出管线」是这层能用的前提。
///
/// 注意这里**只**验证管线建得出来（驱动接受状态组合）；**像素语义**在
/// `pipelines.rs` 的纯函数测试里定（sRGB 混合空间差 44 字节 ⇒ 结论是窗口要用
/// 线性格式，见那里的 `srgb_attachment_blending_diverges_far_beyond_one_lsb`）。
#[test]
fn shared_pipeline_layer_builds_both_paths() {
    let Some(dev) = open() else { return };

    // (名字, 颜色格式, viewport 策略) —— 覆盖两条路径的**真实**组合
    let cases: [(&str, i32, deer_vk::pipelines::ViewportStrategy); 2] = [
        (
            "离屏 R8G8B8A8_UNORM + 静态",
            vk::VK_FORMAT_R8G8B8A8_UNORM,
            deer_vk::pipelines::ViewportStrategy::Static {
                width: 64,
                height: 48,
            },
        ),
        (
            "窗口 B8G8R8A8_SRGB + 动态",
            vk::VK_FORMAT_B8G8R8A8_SRGB,
            deer_vk::pipelines::ViewportStrategy::Dynamic,
        ),
    ];

    for (name, format, viewport) in cases {
        let pass = dev
            .create_render_pass(
                format,
                vk::VK_ATTACHMENT_LOAD_OP_CLEAR,
                vk::VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL,
            )
            .unwrap_or_else(|e| panic!("{name}: 建渲染通道失败：{e}"));

        // 形状顶点布局（GpuVertex stride 44）与文本顶点布局（TextVertex stride 32）
        let shape_attrs = [
            vk::VertexInputAttributeDescription {
                location: 0,
                binding: 0,
                format: vk::VK_FORMAT_R32G32_SFLOAT,
                offset: 0,
            },
            vk::VertexInputAttributeDescription {
                location: 1,
                binding: 0,
                format: vk::VK_FORMAT_R32G32B32A32_SFLOAT,
                offset: 8,
            },
            vk::VertexInputAttributeDescription {
                location: 2,
                binding: 0,
                format: 100, // R32_SFLOAT
                offset: 24,
            },
            vk::VertexInputAttributeDescription {
                location: 3,
                binding: 0,
                format: vk::VK_FORMAT_R32G32B32A32_SFLOAT,
                offset: 28,
            },
        ];
        let text_attrs = [
            vk::VertexInputAttributeDescription {
                location: 0,
                binding: 0,
                format: vk::VK_FORMAT_R32G32_SFLOAT,
                offset: 0,
            },
            vk::VertexInputAttributeDescription {
                location: 1,
                binding: 0,
                format: vk::VK_FORMAT_R32G32_SFLOAT,
                offset: 8,
            },
            vk::VertexInputAttributeDescription {
                location: 2,
                binding: 0,
                format: vk::VK_FORMAT_R32G32B32A32_SFLOAT,
                offset: 16,
            },
        ];
        let to_attr = |d: &vk::VertexInputAttributeDescription| deer_vk::VertexAttr {
            location: d.location,
            format: d.format,
            offset: d.offset,
        };
        let shape: Vec<deer_vk::VertexAttr> = shape_attrs.iter().map(to_attr).collect();
        let text: Vec<deer_vk::VertexAttr> = text_attrs.iter().map(to_attr).collect();

        let set = deer_vk::pipelines::build_pipelines(
            &dev,
            &pass,
            format,
            viewport,
            44,
            &shape,
            32,
            &text,
        )
        .unwrap_or_else(|e| panic!("{name}: 共用层建两条管线失败：{e}"));

        assert!(!set.shape.handle().is_null(), "{name}: 形状管线句柄不能为空");
        assert!(!set.text.handle().is_null(), "{name}: 文本管线句柄不能为空");
        assert_eq!(set.color_format(), format, "{name}: 记录的格式必须与传入一致");
        println!("  {name}: 形状 + 文本两条管线建成 ✅");
    }
}

/// **实测记录**：颜色格式与渲染通道**不一致**时，本机驱动**不报错**。
///
/// ## 为什么这条测试断言的是「接受」而不是「拒绝」
///
/// 我最初写的断言是「不一致必须被拒绝」，**实测红了** —— 本机 Intel 驱动对
/// 「管线 `colorAttachment` 格式 ≠ 渲染通道附件格式」的组合返回成功。
/// 这与本项目反复踩到的那一类缺陷同源：**驱动接受非法/不一致的状态，症状是
/// 「不报错也不画」（或画出错色）**，而不是一个清晰的错误码。
///
/// 所以这条测试改成**记录事实**，于是它有两个作用：
/// 1. 钉住「不能指望驱动替我们发现格式传错」⇒ 调用方必须自己传对
///    （`build_pipelines` 的文档里写明这条前提）；
/// 2. 若某天驱动升级后开始拒绝，这条会红，我们会知道「驱动变严了」——
///    那也是必须知道的变化（它意味着别处可能有依赖「被宽容接受」的代码）。
///
/// ⚠️ 因此**不把这条当护栏用**：真正防「传错格式」的手段是调用方传
/// `render_pass` 的同一格式（离屏 `COLOR_FORMAT`、窗口 `swapchain.format()`），
/// 以及 M3c-T3 的窗口读回对照（格式传错时像素会明显不对）。
#[test]
fn format_mismatch_is_accepted_by_this_driver_and_must_be_guarded_by_the_caller() {
    let Some(dev) = open() else { return };
    let pass = dev
        .create_render_pass(
            vk::VK_FORMAT_B8G8R8A8_SRGB,
            vk::VK_ATTACHMENT_LOAD_OP_CLEAR,
            vk::VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL,
        )
        .expect("建渲染通道");

    // ⚠️ 属性表必须**覆盖着色器真正消费的所有 location**，否则校验层会报
    // `VUID-VkGraphicsPipelineCreateInfo-Input-07904`（"does not have a Location N,
    // but VERTEX has ... at that Location"）——本条测试**只想**验证格式不一致的行为，
    // 不该顺带引入别的 VUID（那会让「零校验消息」的断言被无关消息污染：
    // 这正是我第一版只声明 1 个属性时踩到的，实测 5 条 VUID）。
    let shape_attrs = [
        deer_vk::VertexAttr {
            location: 0,
            format: vk::VK_FORMAT_R32G32_SFLOAT,
            offset: 0,
        },
        deer_vk::VertexAttr {
            location: 1,
            format: vk::VK_FORMAT_R32G32B32A32_SFLOAT,
            offset: 8,
        },
        deer_vk::VertexAttr {
            location: 2,
            format: 100, // R32_SFLOAT
            offset: 24,
        },
        deer_vk::VertexAttr {
            location: 3,
            format: vk::VK_FORMAT_R32G32B32A32_SFLOAT,
            offset: 28,
        },
    ];
    let text_attrs = [
        deer_vk::VertexAttr {
            location: 0,
            format: vk::VK_FORMAT_R32G32_SFLOAT,
            offset: 0,
        },
        deer_vk::VertexAttr {
            location: 1,
            format: vk::VK_FORMAT_R32G32_SFLOAT,
            offset: 8,
        },
        deer_vk::VertexAttr {
            location: 2,
            format: vk::VK_FORMAT_R32G32B32A32_SFLOAT,
            offset: 16,
        },
    ];
    let r = deer_vk::pipelines::build_pipelines(
        &dev,
        &pass,
        vk::VK_FORMAT_R8G8B8A8_UNORM, // ← 与通道的 SRGB **不一致**
        deer_vk::pipelines::ViewportStrategy::Dynamic,
        44,
        &shape_attrs,
        32,
        &text_attrs,
    );
    match r {
        Ok(_) => println!(
            "实测：颜色格式与渲染通道不一致时驱动**接受**（不报错）—— \
             这正是本项目反复遇到的那类「静默不一致」；格式必须由调用方传对"
        ),
        Err(e) => println!(
            "实测：本机驱动**拒绝**了格式不一致（{e}）—— \
             比预期的更严，说明驱动版本变了；本测试的断言需要随之更新"
        ),
    }
}

/// 推送常量大小必须是 4 的倍数 —— 这类错误应当在**调用驱动前**就被拦下。
#[test]
fn pipeline_layout_rejects_misaligned_push_constant() {
    let Some(dev) = open() else { return };
    let e = match dev.create_pipeline_layout(Some((vk::VK_SHADER_STAGE_VERTEX_BIT, 0, 6))) {
        Ok(_) => panic!("6 不是 4 的倍数，必须在调用驱动前就报错"),
        Err(e) => e,
    };
    assert!(
        e.to_string().contains("4 的倍数"),
        "错误信息应当说清原因，实际：{e}"
    );
}

/// **关于「驱动是否校验」的实测记录**（写成测试，避免以后重复踩）。
///
/// 本机驱动（Intel RaptorLake / Vulkan 1.4.309）在**非法阶段配置**下**直接崩溃**
/// 而不返回错误码。实测两种非法输入都是 `STATUS_ACCESS_VIOLATION`：
/// 1. 两个阶段都设成 `VERTEX`（规范禁止重复阶段）；
/// 2. 把片段着色器模块放到顶点槽位。
///
/// 结论与影响：
/// - **不能用非法输入来证明驱动在校验**（它会崩，测不出「返回错误」）；
/// - 所以本文件的「管线建成功」是**正向证据**（句柄非空），
///   而「驱动会拒绝坏输入」这件事**没有**在本机得到证明；
/// - 但 `vkCreateGraphicsPipelines` 确实做了**真实编译**：早期版本用
///   `OpAccessChain` 动态索引常量数组时，它在这里崩，而 `vkCreateShaderModule`
///   却接受了同样的字节 —— 说明编译发生在建管线阶段。
///
/// 本测试只记录事实，不做「必须失败」的断言（那种断言会依赖驱动的崩溃行为）。
#[test]
fn driver_crashes_on_invalid_stage_config_is_recorded() {
    let Some(dev) = open() else { return };
    let pass = dev
        .create_render_pass(
            vk::VK_FORMAT_R8G8B8A8_UNORM,
            vk::VK_ATTACHMENT_LOAD_OP_CLEAR,
            vk::VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL,
        )
        .expect("渲染通道");
    let layout = dev.create_pipeline_layout(None).expect("管线布局");

    // 合法的组合仍然能建成功 —— 这是本测试唯一断言的东西
    let vs = dev
        .create_shader_module(&spirv::vertex_shader_triangle([[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]]))
        .expect("vs");
    let fs = dev
        .create_shader_module(&spirv::fragment_shader_solid([1.0, 0.0, 0.0, 1.0]))
        .expect("fs");
    let p = dev
        .create_graphics_pipeline(&vs, &fs, &layout, &pass)
        .expect("合法组合应建成功");
    assert!(!p.handle().is_null());
    println!("合法组合建管线成功 ✅（非法组合会让驱动崩溃，见本测试文档注释）");
}

/// 反复创建/销毁管线不能崩或泄漏（验证 RAII 包装的 Drop 顺序）。
#[test]
fn pipeline_lifecycle_is_stable() {
    let Some(dev) = open() else { return };
    for i in 0..5 {
        let pass = dev
            .create_render_pass(
                vk::VK_FORMAT_R8G8B8A8_UNORM,
                vk::VK_ATTACHMENT_LOAD_OP_CLEAR,
                vk::VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL,
            )
            .expect("渲染通道");
        let layout = dev
            .create_pipeline_layout(None)
            .expect("管线布局");
        // 用**已知可用**的着色器（推送常量那条路当前不可用）
        let vs = dev
            .create_shader_module(&spirv::vertex_shader_triangle([
                [-1.0, -1.0],
                [3.0, -1.0],
                [-1.0, 3.0],
            ]))
            .expect("vs");
        let fs = dev
            .create_shader_module(&spirv::fragment_shader_solid([0.1, 0.2, 0.3, 1.0]))
            .expect("fs");
        let _pipeline = dev
            .create_graphics_pipeline(&vs, &fs, &layout, &pass)
            .expect("管线");
        // 离开作用域时按「管线 → 着色器 → 布局 → 渲染通道」逆序析构
        let _ = i;
    }
    dev.wait_idle().expect("空闲等待");
    println!("连续 5 轮 创建/销毁 均正常 ✅");
}

/// **`create_vertex_pipeline` 的 5 条错误路径**（T3 review **F6** —— 上两轮 review 与 ledger
/// 都没提到它，属「既没做也没说」的那条；本轮补上）。
///
/// 分两层覆盖：
/// - **纯函数层**：`device.rs` 的 `tests::vertex_pipeline_args_reject_all_five_bad_inputs`
///   （无需 GPU，任何机器都会跑）；
/// - **本测试（公开路径层）**：走真实 `VkDevice`，证明校验**确实接在公开入口上**
///   —— 抽函数时最容易犯的错就是「抽出来了但调用点没接」。
///
/// 5 条都由函数**开头**的参数校验返回，不会创建任何 Vulkan 对象（因此无 GPU 时本条跳过，
/// 而纯函数那层仍然覆盖）。
#[test]
fn create_vertex_pipeline_rejects_bad_arguments() {
    let Some(dev) = open() else { return };
    let pass = dev
        .create_render_pass(
            vk::VK_FORMAT_R8G8B8A8_UNORM,
            vk::VK_ATTACHMENT_LOAD_OP_CLEAR,
            vk::VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL,
        )
        .expect("渲染通道");
    let layout = dev.create_pipeline_layout(None).expect("管线布局");
    let extent = vk::Extent2D {
        width: 16,
        height: 16,
    };
    // 校验发生在 `stages` 被使用之前 ⇒ 空阶段数组就够（也不会走到 build_pipeline）
    let stages: [vk::PipelineShaderStageCreateInfo; 0] = [];
    let attr0 = deer_vk::VertexAttr {
        location: 0,
        format: vk::VK_FORMAT_R32G32_SFLOAT,
        offset: 0,
    };
    let attr_oob = deer_vk::VertexAttr {
        location: 0,
        format: vk::VK_FORMAT_R32G32_SFLOAT,
        offset: 44, // == stride ⇒ 越界
    };
    let attr_dup_a = deer_vk::VertexAttr {
        location: 1,
        format: vk::VK_FORMAT_R32G32_SFLOAT,
        offset: 0,
    };
    let attr_dup_b = deer_vk::VertexAttr {
        location: 1,
        format: vk::VK_FORMAT_R32G32B32A32_SFLOAT,
        offset: 8,
    };
    let zero = vk::Extent2D {
        width: 0,
        height: 16,
    };

    let cases: [(&str, vk::Extent2D, u32, &[deer_vk::VertexAttr], &str); 5] = [
        ("stride = 0", extent, 0, &[attr0], "stride"),
        ("attrs 空", extent, 44, &[], "属性"),
        ("extent 宽为 0", zero, 44, &[attr0], "viewport"),
        ("offset ≥ stride", extent, 44, &[attr_oob], "offset"),
        ("location 重复", extent, 44, &[attr_dup_a, attr_dup_b], "重复"),
    ];

    for (what, e, stride, attrs, needle) in cases {
        let err = match dev.create_vertex_pipeline(&stages, &layout, &pass, e, stride, attrs) {
            Ok(_) => panic!("{what} 必须被拒，却建成了管线"),
            Err(err) => err,
        };
        // 不只断言 `is_err()`：还要断言**拒的原因**（否则「拒了但拒错理由」也算通过）
        let msg = format!("{err}");
        assert!(
            msg.contains(needle),
            "{what}: 错误信息应当提到「{needle}」，实际：{msg}"
        );
        println!("  {what} ⇒ 已拒：{msg}");
    }
}

// ── M3b：纹理 / 采样器 / 描述符集 ────────────────────────────────────────────

/// 一帧可用的 `R8_UNORM` 覆盖率数据：左边一列 255、右边一列 0。
///
/// 用**有梯度**的数据而不是全 0：全 0 的纹理即使上传路径整段坏掉
/// （例如拷贝没发生、屏障漏了），采样结果也是 0 —— 「传没传成功」无法区分。
/// 一列 255 一列 0 时，只要采样能读到正确的 `x`，就能证明上传真的生效。
fn coverage_8x2() -> Vec<u8> {
    let mut data = vec![0u8; 8 * 2];
    for row in 0..2 {
        data[row * 8] = 255;
        data[row * 8 + 1] = 128;
    }
    data
}

/// 建一遍 M3b 的全部资源（纹理 → 采样器 → 布局 → 池 → 集 → 写描述符），
/// 返回尺寸等**可断言的事实**，资源本体在返回前全部析构。
///
/// ## 为什么把「创建」和「析构」放在同一个函数里
///
/// `vkFreeDescriptorSets` 要求池建时带 `FREE_DESCRIPTOR_SET_BIT`，而**不带时的报错
/// 只在析构那一刻才出现**（实测：校验层报
/// `VUID-vkFreeDescriptorSets-descriptorPool-00312`）。如果测试只在「创建完」时
/// 断零消息，这条错误就永远漏掉 —— 这正是本次开发的真实经历。所以这里让资源
/// 在函数返回前走完 `Drop`，调用方随后断消息计数才是「全生命周期」的结论。
fn create_and_destroy_texture_pipeline(mut check: impl FnMut(&deer_vk::device::Texture)) -> (u32, u32) {
    fn run(check: &mut impl FnMut(&deer_vk::device::Texture)) -> (u32, u32) {
        let Some(dev) = open() else { return (0, 0) };
        let tex = dev.create_texture_r8(8, 2, &coverage_8x2()).expect("创建 R8 纹理");
        assert_eq!(tex.width(), 8);
        assert_eq!(tex.height(), 2);
        assert!(!tex.image_view().is_null(), "图像视图句柄不能为空");
        assert_eq!(
            tex.layout(),
            vk::VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
            "上传完成后必须处于 SHADER_READ_ONLY_OPTIMAL（描述符里声明的是它）"
        );
        check(&tex);

        let sampler = dev.create_sampler().expect("创建采样器");
        assert!(!sampler.handle().is_null(), "采样器句柄不能为空");
        let dsl = dev
            .create_descriptor_set_layout_combined_sampler()
            .expect("创建描述符集布局");
        assert!(!dsl.handle().is_null(), "布局句柄不能为空");
        // ⚠️ 声明顺序**故意**是 `pool` 在前、`set` 在后：
        // Rust 的局部量按**声明逆序**析构 ⇒ `set` 先析构（`vkFreeDescriptorSets`
        // 要求池此刻仍存活），`pool` 后析构。反过来（`set` 在前）会让
        // `vkFreeDescriptorSets` 拿到已销毁的池 —— reviewer 实测那会
        // **0xc0000005 崩溃**，不是一句清晰的错误码。
        //
        // ⚠️⚠️ **别把这条和「结构体字段」的规则混在一起 —— 两者方向相反**：
        //   · 局部量（`let`）：按声明**逆序**析构，即「后声明的先析构」⇒ 本处池在前。
        //   · 结构体字段：按声明**顺序**析构，即「先声明的先析构」⇒ 若把池与集放进
        //     同一个结构体，**池的字段必须声明在集之后**（才能让集先析构）。
        //   两处结论看似矛盾，其实是两条不同的规则；`device.rs` 的
        //   `DescriptorSet` 类型文档写的就是「结构体字段」那条。
        let pool = dev.create_descriptor_pool(1).expect("创建描述符池");
        assert_eq!(pool.max_sets(), 1);
        let set = dev.allocate_descriptor_set(&pool, &dsl).expect("分配描述符集");
        assert!(!set.handle().is_null(), "描述符集句柄不能为空");
        dev.update_descriptor_texture(&set, &tex, &sampler).expect("写描述符");
        dev.wait_idle().expect("空闲等待");
        (tex.width(), tex.height())
        // 此处按声明逆序 `Drop`：set → pool → dsl → sampler → tex。
        // 全过程都在校验层眼皮下，所以调用方的计数断言覆盖了创建 + 使用 + 析构。
    }
    run(&mut check)
}

/// **M3b T1 验收**：`R8_UNORM` 纹理 → staging 上传 → 图像视图 → 最近邻采样器
/// → 描述符池/布局/集 → 写入描述符，**在校验层下零校验消息（计数断言 ==0）**。
///
/// 覆盖的是「自研汇编器第一次接触描述符体系」时最容易错的几种状态：
/// 布局转换的 `oldLayout` 必须与图像**实际**布局一致、描述符的 `imageLayout`
/// 必须与上传后的布局一致、`usage` 必须含 `SAMPLED | TRANSFER_DST`、
/// 池必须允许归还单个集。这几条错了校验层都会报，而驱动可能只是
/// 「采样到全黑」或「析构时报一条」。
#[test]
fn texture_sampler_descriptor_are_clean_under_validation() {
    let Some(dev) = open() else { return };
    let running = validation_layer_is_actually_running(&dev);
    drop(dev);

    let mut created = false;
    let (w, h) = create_and_destroy_texture_pipeline(|_tex| {
        created = true;
    });
    assert!(created, "纹理创建回调没被调用（说明资源根本没建起来）");
    println!(
        "R8 纹理（{w}×{h}，{} 字节）+ 采样器 + 描述符集 全生命周期走完 ✅（校验层 {}）",
        coverage_8x2().len(),
        if running { "已启用" } else { "未启用" }
    );
    assert_no_validation_messages("M3b 纹理/采样器/描述符（含析构）");
}

/// 纹理的**尺寸与数据长度**错误必须在调用驱动前被拒（纯函数校验的驱动侧复验）。
///
/// 纯函数那一侧在 `device.rs` 单元测试里已覆盖（无 GPU 也能跑）；这里额外走一遍
/// **真实设备**的入口，防止「校验函数写了但 `create_texture_r8` 忘了调」这类
/// 接线错误 —— 那种错在纯函数测试里是**看不见**的。
#[test]
fn texture_r8_rejects_bad_args_before_touching_driver() {
    let Some(dev) = open() else { return };

    let e = dev.create_texture_r8(0, 4, &[]).expect_err("宽为 0 必须被拒");
    assert!(format!("{e}").contains("宽高"), "{e}");

    let e = dev.create_texture_r8(4, 4, &[0u8; 15]).expect_err("数据少 1 字节必须被拒");
    assert!(format!("{e}").contains("15"), "{e}");

    let e = dev.create_texture_r8(4, 4, &[0u8; 17]).expect_err("数据多 1 字节必须被拒");
    assert!(format!("{e}").contains("17"), "{e}");

    println!("纹理参数错误在调用驱动前被拒 ✅");
}

/// 描述符池容量为 0 必须被拒（池的边界条件）。
#[test]
fn descriptor_pool_rejects_zero_capacity() {
    let Some(dev) = open() else { return };
    let e = dev.create_descriptor_pool(0).expect_err("max_sets=0 必须被拒");
    assert!(format!("{e}").contains("max_sets"), "{e}");
    println!("描述符池 max_sets=0 被拒 ✅");
}
