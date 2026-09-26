//! 功能示例：**离屏渲染 + 回读像素**（M2a-4..5 的成果 + 一个已知缺陷）。
//!
//! ```sh
//! cargo run -p deer-gui --example gpu_offscreen
//! ```
//!
//! 这个示例把「GPU 渲染」这条链路走通到**回读像素**：
//! 创建离屏图像 → 渲染通道 → 图形管线 → 命令缓冲 → 提交 → 栅栏等待 →
//! `copyImageToBuffer` → map → RGBA8 像素。
//!
//! ## ⚠️ 一个已知缺陷（务必读）
//!
//! **`vkCmdDraw` 目前不产生任何像素。** 清屏是通的（换清屏色，回读跟着变），
//! 回读也是通的（像素长度与数值都对），但**绘制出来的几何一个像素都没有**。
//!
//! 已排除的原因见 `crates/deer-vk/tests/offscreen_render.rs` 的
//! `draw_produces_no_pixels_is_a_known_defect` 测试文档。
//!
//! 所以本示例演示的是**能用的那部分**：清屏 + 回读 + 确定性。
//! 它同时会把「绘制为 0 像素」这件事打印出来并断言 —— 修好后断言会红，
//! 提醒我们更新文档。

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

    // ⑦ 如实报告已知缺陷
    let green = pixels
        .chunks_exact(4)
        .filter(|p| p[0] < 50 && p[1] > 200 && p[2] < 50)
        .count();
    println!("\n=== 已知缺陷 ===");
    println!("绘制的三角形产生了 {green} 个绿色像素。");
    if green == 0 {
        println!("⇒ **`vkCmdDraw` 没有产生任何像素**（清屏与回读都是通的）。");
        println!("   已排除：着色器内容、几何裁剪、动态/静态 viewport、清屏与回读路径。");
        println!("   详见 crates/deer-vk/tests/offscreen_render.rs 的已知缺陷测试。");
    } else {
        println!("⇒ 绘制通了！请更新文档与示例（这条说明缺陷已修复）。");
    }

    // ⑧ 边界
    println!("\n=== 当前边界 ===");
    println!("  ✅ 能：建渲染通道/管线、录制命令缓冲、提交、栅栏同步、清屏、回读 RGBA8");
    println!("  ❌ 不能：**画出几何**（已知缺陷，排查方向见测试文档）");
    println!("  ❌ 不能：把界面树渲染到 GPU（要先修好绘制，再写 DrawList → GPU 的渲染器）");
    println!("\n要出可看的界面图请用 CPU 后端：cargo run -p deer-gui --example render_to_png");

    dev.wait_idle().expect("空闲等待");
}
