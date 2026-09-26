//! 功能示例：**GPU 图形管线**（M2a-3 的成果）。
//!
//! ```sh
//! cargo run -p deer-gui --example vulkan_pipeline
//! ```
//!
//! 这一步走完了「从设备到可用的图形管线」：渲染通道 → 管线布局 → 着色器模块 →
//! **`vkCreateGraphicsPipelines`**。最后那一步是**自研 SPIR-V 汇编器的真正验收**
//! —— 因为 `vkCreateShaderModule` 极宽容（连非法 `bound` 都接受），
//! 驱动真正编译着色器是在建管线的时候。
//!
//! **边界（务必读）**：这一步**还不会画出任何像素**。命令缓冲、离屏图像、回读
//! 在 M2a-4..6；渲染到窗口在 M2b。所以本示例只证明「管线可以建出来」。

use deer_gpu::Backend;
use deer_vk::ffi_dev as vk;
use deer_vk::spirv;
use deer_vk::VkDevice;

fn main() {
    println!("=== GPU 图形管线（M2a-3）===\n");

    // ① 枚举设备（挑独显更好，但这里只用第一个可用的）
    let vk_backend = match deer_vk::VkBackend::new() {
        Ok(b) => b,
        Err(e) => {
            println!("本机没有可用的 Vulkan：{e}");
            println!("⇒ 出图请用 CPU 后端：cargo run -p deer-gui --example render_to_png");
            return;
        }
    };
    let adapters = vk_backend.adapters();
    println!("枚举到 {} 个适配器：", adapters.len());
    for (i, a) in adapters.iter().enumerate() {
        println!("  [{i}] {} | {:?}", a.name, a.kind);
    }

    // ② 打开逻辑设备
    let dev = match VkDevice::open(0) {
        Ok(d) => d,
        Err(e) => {
            println!("打开设备失败：{e}");
            return;
        }
    };
    println!(
        "\n设备：{}\n图形队列族 {}，内存类型 {} 种",
        dev.adapter().name,
        dev.queue_family_index(),
        dev.memory_type_count()
    );

    // ③ 渲染通道：一个颜色附件、每帧清屏、最终布局可回读
    let pass = match dev.create_render_pass(
        vk::VK_FORMAT_R8G8B8A8_UNORM,
        vk::VK_ATTACHMENT_LOAD_OP_CLEAR,
        vk::VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL,
    ) {
        Ok(p) => p,
        Err(e) => {
            println!("创建渲染通道失败：{e}");
            return;
        }
    };
    println!("① 渲染通道 ✅");

    // ④ 管线布局：无推送常量、无描述符集（现在是「零资源」管线）
    let layout = dev.create_pipeline_layout(None).expect("管线布局");
    println!("② 管线布局 ✅");

    // ⑤ 两个着色器模块
    let vs = dev
        .create_shader_module(&spirv::vertex_shader_triangle([
            [-0.8, -0.8],
            [0.8, -0.8],
            [0.0, 0.8],
        ]))
        .expect("顶点着色器");
    let fs = dev
        .create_shader_module(&spirv::fragment_shader_solid([0.3, 0.7, 1.0, 1.0]))
        .expect("片段着色器");
    println!("③ 着色器模块 ✅（顶点 + 片段，由自研 SPIR-V 汇编器生成）");

    // ⑥ **真正的验收**：建图形管线
    match dev.create_graphics_pipeline(&vs, &fs, &layout, &pass) {
        Ok(p) => {
            println!("④ 图形管线 ✅（句柄 {:?}）", p.handle());
            println!("\n=== 成功 ===");
            println!("这说明：渲染通道 / 管线布局 / 着色器 / 管线状态都通过了驱动的真实校验。");
        }
        Err(e) => {
            println!("④ 图形管线 ❌ {e}");
            println!("这通常意味着着色器或管线状态有问题 —— 而此时");
            println!("`vkCreateShaderModule` 往往还是「成功」的（它不编译）。");
        }
    }

    // ⑦ 顺带展示已知不可用的那条路（如实告知边界）
    println!("\n=== 已知不可用：推送常量矩形着色器 ===");
    match dev.create_shader_module(&spirv::vertex_shader_rect_pushconstant()) {
        Ok(m) => {
            println!("  模块能被接受（句柄 {:?}）—— 但用它建管线会失败。", m.handle());
            println!("  三种推送常量写法分别导致「空句柄」或「访问违例」，已在代码里记录。");
            println!("  矩形绘制后续改走顶点缓冲方案（M2a 出图之后）。");
        }
        Err(e) => println!("  模块被拒：{e}"),
    }

    // ⑧ 边界
    println!("\n=== 当前边界 ===");
    println!("  ✅ 能：枚举 GPU / 打开设备 / 建渲染通道 / 建图形管线");
    println!("  ❌ 不能：**画出像素**（命令缓冲 + 离屏图像 + 回读在 M2a-4..6）");
    println!("  ❌ 不能：渲染到窗口（M2b，需先定窗口方案）");
    println!("\n要出图请用 CPU 后端：cargo run -p deer-gui --example render_to_png");

    dev.wait_idle().expect("空闲等待");
}
