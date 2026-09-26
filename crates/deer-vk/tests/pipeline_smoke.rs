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

fn open() -> Option<VkDevice> {
    match VkDevice::open(0) {
        Ok(d) => Some(d),
        Err(e) => {
            println!("跳过：本机没有可用的 Vulkan（{e}）");
            None
        }
    }
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
