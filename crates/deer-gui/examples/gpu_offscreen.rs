//! 功能示例：**GPU 离屏渲染 + 回读像素**（M2a-4..6 的成果）。
//!
//! ```sh
//! cargo run -p deer-gui --example gpu_offscreen
//! ```
//!
//! 这个示例把「GPU 渲染」这条链路**完整走通到像素**：
//! 创建离屏图像 → 渲染通道 → 图形管线 → 命令缓冲 → 提交 → 栅栏等待 →
//! `copyImageToBuffer` → map → RGBA8 像素。
//!
//! ## 曾经的「已知缺陷」与它的根因
//!
//! 有一段时间 `vkCmdDraw` **不产生任何像素**，而所有 Vulkan API 都返回成功。
//! 根因是**自研 SPIR-V 汇编器的段序错误**：
//! `OpEntryPoint` 被排在类型/常量之后、`OpFunction` 掉进了类型段。
//! 驱动对这类错误**既不报错也不画**，直到用官方 `spirv-val` 才看到
//! `EntryPoint is in an invalid layout section`。
//!
//! 现在本示例会画出三角形，并且**断言像素数量与位置**（见代码末尾）。

use deer_gpu::Backend;
use deer_vk::ffi_dev as vk;
use deer_vk::offscreen;
use deer_vk::{spirv, VkDevice};

const W: u32 = 96;
const H: u32 = 96;

fn main() {
    println!("=== GPU 离屏渲染 + 回读（M2a-4..5）===\n");

    let vk_backend = match deer_vk::VkBackend::new() {
        Ok(b) => b,
        Err(e) => {
            println!("本机没有可用的 Vulkan：{e}");
            println!("⇒ 出图请用 CPU 后端：cargo run -p deer-gui --example render_to_png");
            return;
        }
    };
    let dev = match VkDevice::open(0) {
        Ok(d) => d,
        Err(e) => {
            println!("打开设备失败：{e}");
            return;
        }
    };
    println!("设备：{}", dev.adapter().name);
    println!("适配器数：{}\n", vk_backend.adapters().len());

    // ① 渲染通道
    let pass = dev
        .create_render_pass(
            vk::VK_FORMAT_R8G8B8A8_UNORM,
            vk::VK_ATTACHMENT_LOAD_OP_CLEAR,
            vk::VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL,
        )
        .expect("渲染通道");
    println!("① 渲染通道 ✅");

    // ② 管线（顶点 + 片段）
    let layout = dev.create_pipeline_layout(None).expect("管线布局");
    let vs = dev
        .create_shader_module(&spirv::vertex_shader_triangle([
            [-0.8, -0.8],
            [0.8, -0.8],
            [-0.8, 0.8],
        ]))
        .expect("vs");
    let fs = dev
        .create_shader_module(&spirv::fragment_shader_solid([0.0, 1.0, 0.0, 1.0]))
        .expect("fs");
    let pipeline = dev
        .create_graphics_pipeline(&vs, &fs, &layout, &pass)
        .expect("图形管线");
    println!("② 图形管线 ✅");

    // ③ 离屏设施（图像 + 视图 + 帧缓冲 + 命令池 + 暂存缓冲）
    let off = offscreen::offscreen_for(&dev, &pass, W, H).expect("离屏设施");
    println!("③ 离屏设施 ✅（{W}×{H}，含命令池与回读暂存缓冲）");

    // ④ 渲染 + 提交 + 栅栏等待 + 回读
    let start = std::time::Instant::now();
    let pixels = off
        .render_and_read_back(&pass, &pipeline, 3, [0.1, 0.1, 0.2, 1.0])
        .expect("渲染并回读");
    let elapsed = start.elapsed();
    println!("④ 提交 + 栅栏 + 回读 ✅（耗时 {elapsed:?}）");

    assert_eq!(
        pixels.len(),
        (W as usize) * (H as usize) * 4,
        "回读长度必须是 宽×高×4"
    );

    // ⑤ 像素统计
    let mut colors = std::collections::BTreeMap::new();
    for p in pixels.chunks_exact(4) {
        *colors.entry([p[0], p[1], p[2], p[3]]).or_insert(0usize) += 1;
    }
    println!("\n回读到的颜色（共 {} 像素）：", W * H);
    for (c, n) in &colors {
        println!("  {c:?} × {n}");
    }

    // ⑥ 把回读结果写成 PNG（放大 3 倍便于看）
    std::fs::create_dir_all("render_out").ok();
    let scale = 3usize;
    let (bw, bh) = (W as usize * scale, H as usize * scale);
    let mut big = vec![0u8; bw * bh * 4];
    for y in 0..bh {
        for x in 0..bw {
            let si = ((y / scale) * W as usize + (x / scale)) * 4;
            let di = (y * bw + x) * 4;
            big[di..di + 4].copy_from_slice(&pixels[si..si + 4]);
        }
    }
    if let Ok(png) = deer_gpu::png::encode_rgba(bw as u32, bh as u32, &big) {
        let path = "render_out/gpu_offscreen.png";
        std::fs::write(path, &png).ok();
        println!("\n已写出 {path}（放大 {scale} 倍，你可以打开看 GPU 实际画了什么）");
    }

    // ⑦ 验收：三角形必须真的被画出来，且位置正确
    //
    // 顶点 (-0.8,-0.8) (0.8,-0.8) (-0.8,0.8)，清屏用深色。
    // Vulkan 的 NDC 是 y 向下 ⇒ 顶点 y=-0.8 在屏幕上方 ⇒ 三角形覆盖**左上**。
    let green = pixels
        .chunks_exact(4)
        .filter(|p| p[0] < 50 && p[1] > 200 && p[2] < 50)
        .count();
    let total = (W * H) as usize;
    let ratio = green as f64 / total as f64;
    println!("\n=== 验收 ===");
    println!("绿色（三角形）像素 {green}/{total} = {:.1}%（理论约 32%）", ratio * 100.0);
    assert!(green > 0, "三角形没有被画出来（绘制缺陷回归！）");
    assert!(
        (0.25..0.40).contains(&ratio),
        "绿色占比 {:.1}% 偏离理论值 32%",
        ratio * 100.0
    );

    let at = |x: u32, y: u32| -> [u8; 4] {
        let i = ((y as usize) * (W as usize) + (x as usize)) * 4;
        [pixels[i], pixels[i + 1], pixels[i + 2], pixels[i + 3]]
    };
    println!("(24,12) = {:?}（应在三角形内）", at(24, 12));
    println!("(72,72) = {:?}（应在三角形外）", at(72, 72));
    assert_eq!(at(24, 12), [0, 255, 0, 255], "(24,12) 应在三角形内");
    assert_ne!(at(72, 72), [0, 255, 0, 255], "(72,72) 应在三角形外");
    println!("像素位置也正确 ✅");

    // ⑧ 边界
    println!("\n=== 当前边界 ===");
    println!("  ✅ 能：建渲染通道/管线、录制命令缓冲、提交、栅栏同步、清屏、**绘制几何**、回读 RGBA8");
    println!("  ❌ 不能：把界面树（DrawList）渲染到 GPU —— 那是 M3（GPU 渲染器）");
    println!("  ❌ 不能：渲染到窗口（M2b，需先定窗口方案）");
    println!("  ❌ 推送常量在 Intel 驱动上仍不可用（矩形绘制改走顶点缓冲）");
    println!("\n要出可看的界面图请用 CPU 后端：cargo run -p deer-gui --example render_to_png");

    dev.wait_idle().expect("空闲等待");
}
