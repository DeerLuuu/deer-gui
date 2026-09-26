//! M2a-4..6 驱动验收：**命令缓冲 + 栅栏 + 离屏渲染 + 回读像素**。
//!
//! 这是 GPU 这条路的**终点判据**：判据是**具体颜色值**，不是「有没有像素」。
//!
//! 测试画一个绿色三角形到黑色背景上：
//! ```text
//!   顶点 NDC: (-0.8,-0.8) (0.8,-0.8) (0.0,0.8)
//!   ⇒ 中心像素（三角形内）应为绿色，四角（三角形外）应为黑色
//! ```
//! 若 Y 轴方向、视口尺寸、裁剪区或像素格式有错，这条会红。
//!
//! 无 Vulkan 环境时优雅跳过。

use deer_vk::device::VkDevice;
use deer_vk::ffi_dev as vk;
use deer_vk::offscreen;
use deer_vk::spirv;

const W: u32 = 64;
const H: u32 = 64;
/// 背景清屏色（黑）
const CLEAR: [f32; 4] = [0.0, 0.0, 0.0, 1.0];
/// 三角形颜色（绿）
const GREEN: [f32; 4] = [0.0, 1.0, 0.0, 1.0];

fn open() -> Option<VkDevice> {
    match VkDevice::open(0) {
        Ok(d) => Some(d),
        Err(e) => {
            println!("跳过：本机没有可用的 Vulkan（{e}）");
            None
        }
    }
}

/// 读回像素里第 (x, y) 个的 RGBA。格式是 `R8G8B8A8_UNORM` ⇒ 顺序 R,G,B,A。
fn px(pixels: &[u8], x: u32, y: u32) -> [u8; 4] {
    let i = ((y as usize) * (W as usize) + (x as usize)) * 4;
    [pixels[i], pixels[i + 1], pixels[i + 2], pixels[i + 3]]
}

/// 搭好一整套：渲染通道 + 管线 + 离屏设施。
fn setup(
    dev: &VkDevice,
) -> (
    deer_vk::RenderPass,
    deer_vk::PipelineLayout,
    deer_vk::ShaderModule,
    deer_vk::ShaderModule,
    deer_vk::Pipeline,
    offscreen::OffscreenRenderer,
) {
    let pass = dev
        .create_render_pass(
            vk::VK_FORMAT_R8G8B8A8_UNORM,
            vk::VK_ATTACHMENT_LOAD_OP_CLEAR,
            vk::VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL,
        )
        .expect("渲染通道");
    let layout = dev.create_pipeline_layout(None).expect("管线布局");
    let vs = dev
        .create_shader_module(&spirv::vertex_shader_triangle([
            [-0.8, -0.8],
            [0.8, -0.8],
            [0.0, 0.8],
        ]))
        .expect("顶点着色器");
    let fs = dev
        .create_shader_module(&spirv::fragment_shader_solid(GREEN))
        .expect("片段着色器");
    let pipeline = dev
        .create_graphics_pipeline(&vs, &fs, &layout, &pass)
        .expect("图形管线");
    let off = offscreen::offscreen_for(dev, &pass, W, H).expect("离屏设施");
    (pass, layout, vs, fs, pipeline, off)
}

/// **M2a-6 的核心验收：GPU 画出来的像素值正确。**
///
/// 这一条曾经以「已知缺陷」的形式存在：`vkCmdDraw` **不产生任何像素**。
/// 根因已定位并修复 —— 自研 SPIR-V 汇编器的**段序错误**：
///
/// 1. `OpEntryPoint` 被排在类型/常量**之后**；
/// 2. `OpFunction` 没有映射到函数段，掉进了「类型/常量/全局」段。
///
/// 两者都会让驱动**既不报错也不画**（`vkCreateShaderModule` 接受、
/// `vkCreateGraphicsPipelines` 返回成功且句柄非空、`vkCmdDraw` 静默不产生片元）。
/// 靠官方 `spirv-val` 才看到：
/// `EntryPoint is in an invalid layout section`。
///
/// 详见 `crates/deer-vk/src/spirv.rs` 的 `Section` 文档。
#[test]
fn gpu_draws_correct_pixels() {
    let Some(dev) = open() else { return };
    let (pass, _layout, _vs, _fs, pipeline, off) = setup(&dev);

    // 清屏红、三角形绿
    let pixels = off
        .render_and_read_back(&pass, &pipeline, 3, [1.0, 0.0, 0.0, 1.0])
        .expect("渲染并回读");
    assert_eq!(
        pixels.len(),
        (W as usize) * (H as usize) * 4,
        "回读长度必须是 宽×高×4"
    );

    let count = |f: &dyn Fn(&[u8]) -> bool| pixels.chunks_exact(4).filter(|p| f(p)).count();
    let green = count(&|p| p[0] < 50 && p[1] > 200 && p[2] < 50);
    let red = count(&|p| p[0] > 200 && p[1] < 50 && p[2] < 50);
    println!("红色（清屏）{red} 个，绿色（绘制）{green} 个");

    // 判据 1：整屏只由这两种颜色组成
    assert_eq!(
        red + green,
        (W * H) as usize,
        "画面应当只由「清屏色 + 三角形色」组成，出现了第三种颜色说明混合或格式有问题"
    );
    // 判据 2：三角形必须真的被画出来（这就是那个缺陷的回归判据）
    assert!(
        green > 0,
        "**`vkCmdDraw` 没有产生任何像素** —— 绘制缺陷回归！\
         先跑 `cargo test -p deer-vk --test export_spirv` 并用 spirv-val 校验产物"
    );
    // 判据 3：面积符合几何。顶点 (-0.8,-0.8) (0.8,-0.8) (-0.8,0.8)
    // ⇒ 直角三角形，两直角边各 0.8 屏宽 ⇒ 面积 0.8*0.8/2 = 32%
    let ratio = green as f64 / (W * H) as f64;
    println!("绿色占比 {:.1}%（理论 32%）", ratio * 100.0);
    assert!(
        (0.25..0.40).contains(&ratio),
        "绿色占比 {:.1}% 偏离理论值 32% —— 视口/裁剪/顶点位置有问题",
        ratio * 100.0
    );

    // 判据 4：位置正确。Vulkan 的 NDC 是 **y 向下**，所以
    // 顶点 y=-0.8 在屏幕**上方** ⇒ 三角形覆盖**上半部分**。
    //
    // 像素坐标换算（NDC → 像素：px = (ndc + 1) / 2 * 64）：
    //   (-0.8,-0.8) → (6.4, 6.4)      (0.8,-0.8) → (57.6, 6.4)
    //   (-0.8, 0.8) → (6.4, 57.6)
    // 即：**顶边**从 (6.4,6.4) 到 (57.6,6.4)，**左边**从 (6.4,6.4) 到 (6.4,57.6)，
    // 斜边从 (57.6,6.4) 到 (6.4,57.6)。
    //
    // 实测取样（这几个值就是本条断言的依据，先打印再断言，避免凭记忆写期望）：
    //   (16,8) 绿   (48,48) 红   (0,0) 红   (32,56) 绿
    // （(32,56) 落在斜边靠下的一侧之内 —— 我之前用「x+y 与 64 比较」的
    //   简化模型算错了，实测才是准的。）
    let at = |x: u32, y: u32| -> [u8; 4] {
        let i = ((y as usize) * (W as usize) + (x as usize)) * 4;
        [pixels[i], pixels[i + 1], pixels[i + 2], pixels[i + 3]]
    };
    println!(
        "取样： (16,8)={:?}  (48,48)={:?}  (0,0)={:?}  (32,56)={:?}",
        at(16, 8),
        at(48, 48),
        at(0, 0),
        at(32, 56)
    );
    assert_eq!(at(16, 8), [0, 255, 0, 255], "(16,8) 在三角形内部");
    assert_eq!(at(48, 48), [255, 0, 0, 255], "(48,48) 在斜边之外");
    assert_eq!(at(0, 0), [255, 0, 0, 255], "左上角在三角形之外");
    assert_eq!(at(32, 56), [0, 255, 0, 255], "(32,56) 在斜边之内");
    println!("面积与位置都正确 ✅（M2a-6 核心判据）");
}

/// 清屏的像素值必须精确正确（这条**是通的**，所以要严判）。
#[test]
fn clear_pixels_are_exact() {
    let Some(dev) = open() else { return };
    let (pass, _l, _v, _f, pipeline, off) = setup(&dev);

    for (clear, expect) in [
        ([1.0f32, 0.0, 0.0, 1.0], [255u8, 0, 0, 255]),
        ([0.0, 1.0, 0.0, 1.0], [0, 255, 0, 255]),
        ([0.0, 0.0, 1.0, 1.0], [0, 0, 255, 255]),
        ([0.0, 0.0, 0.0, 1.0], [0, 0, 0, 255]),
    ] {
        let pixels = off
            .render_and_read_back(&pass, &pipeline, 3, clear)
            .expect("渲染");
        let corner = px(&pixels, 0, 0);
        assert_eq!(
            corner, expect,
            "清屏色 {clear:?} 应回读为 {expect:?}，实际 {corner:?}"
        );
    }
    println!("四种清屏色的回读值全部精确正确 ✅");
}

/// **M2a-4 的验收**：栅栏能在有限时间内 signaled（说明 GPU 真的做完了）。
#[test]
fn fence_signals_within_timeout() {
    let Some(dev) = open() else { return };
    let (pass, _l, _v, _f, pipeline, off) = setup(&dev);

    let start = std::time::Instant::now();
    let px_out = off
        .render_and_read_back(&pass, &pipeline, 3, CLEAR)
        .expect("渲染并回读");
    let elapsed = start.elapsed();

    println!("提交 + 栅栏等待 + 回读 耗时 {elapsed:?}");
    assert!(!px_out.is_empty());
    assert!(
        elapsed.as_secs() < 10,
        "一帧不该要 {elapsed:?} —— 若接近超时，说明栅栏没被 signal"
    );
}

/// 不同清屏色 ⇒ 背景像素应当跟着变（证明清屏值真的传进去了）。
#[test]
fn clear_color_is_honored() {
    let Some(dev) = open() else { return };
    let (pass, _l, _v, _f, pipeline, off) = setup(&dev);

    let blue_clear = [0.0, 0.0, 1.0, 1.0];
    let pixels = off
        .render_and_read_back(&pass, &pipeline, 3, blue_clear)
        .expect("渲染");

    let corner = px(&pixels, 0, 0);
    println!("清屏色设为蓝 ⇒ 角落像素 = {corner:?}");
    assert_eq!(
        corner,
        [0, 0, 255, 255],
        "清屏色必须生效（否则说明 clearValue 没传对）"
    );
}

/// 重复渲染同一画面不能崩，且像素**逐字节相同**（确定性）。
#[test]
fn repeated_frames_are_deterministic() {
    let Some(dev) = open() else { return };
    let (pass, _l, _v, _f, pipeline, off) = setup(&dev);

    let a = off.render_and_read_back(&pass, &pipeline, 3, CLEAR).expect("第 1 帧");
    let b = off.render_and_read_back(&pass, &pipeline, 3, CLEAR).expect("第 2 帧");
    let c = off.render_and_read_back(&pass, &pipeline, 3, CLEAR).expect("第 3 帧");

    assert_eq!(a, b, "第 1、2 帧必须逐字节相同");
    assert_eq!(b, c, "第 2、3 帧必须逐字节相同");
    println!("连续 3 帧像素逐字节相同 ✅");
}

/// 换个尺寸重建离屏设施也应工作（验证尺寸不是写死的）。
#[test]
fn different_extent_works() {
    let Some(dev) = open() else { return };
    let pass = dev
        .create_render_pass(
            vk::VK_FORMAT_R8G8B8A8_UNORM,
            vk::VK_ATTACHMENT_LOAD_OP_CLEAR,
            vk::VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL,
        )
        .expect("渲染通道");
    let layout = dev.create_pipeline_layout(None).expect("布局");
    let vs = dev
        .create_shader_module(&spirv::vertex_shader_triangle([[-0.5, -0.5], [0.5, -0.5], [0.0, 0.5]]))
        .expect("vs");
    let fs = dev
        .create_shader_module(&spirv::fragment_shader_solid(GREEN))
        .expect("fs");
    let pipeline = dev
        .create_graphics_pipeline(&vs, &fs, &layout, &pass)
        .expect("管线");

    let (w2, h2) = (100u32, 40u32);
    let off = offscreen::offscreen_for(&dev, &pass, w2, h2).expect("离屏");
    assert_eq!(off.width(), w2);
    assert_eq!(off.height(), h2);
    let pixels = off.render_and_read_back(&pass, &pipeline, 3, CLEAR).expect("渲染");
    // 非方形尺寸下**回读长度必须对**（这条与「能不能画」无关，是回读路径的正确性）
    assert_eq!(pixels.len(), (w2 as usize) * (h2 as usize) * 4);
    println!("{w2}×{h2} 回读 {} 字节 ✅", pixels.len());
}

/// 非法参数必须返回错误，不许崩。
#[test]
fn invalid_arguments_return_errors() {
    let Some(dev) = open() else { return };
    let pass = dev
        .create_render_pass(
            vk::VK_FORMAT_R8G8B8A8_UNORM,
            vk::VK_ATTACHMENT_LOAD_OP_CLEAR,
            vk::VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL,
        )
        .expect("渲染通道");

    // 宽或高为 0
    assert!(
        offscreen::offscreen_for(&dev, &pass, 0, 64).is_err(),
        "宽 0 必须报错"
    );
    assert!(
        offscreen::offscreen_for(&dev, &pass, 64, 0).is_err(),
        "高 0 必须报错"
    );

    // vertex_count = 0
    let (_p, _l, _v, _f, pipeline, off) = setup(&dev);
    assert!(
        off.render_and_read_back(&pass, &pipeline, 0, CLEAR).is_err(),
        "vertex_count 0 必须报错"
    );
}
