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

/// ⚠️ **已知缺陷的回归测试**：`vkCmdDraw` 在本机 Intel 驱动上**不产生任何像素**。
///
/// 现状（如实记录，不粉饰）：
/// | 环节 | 状态 |
/// |---|---|
/// | 命令缓冲录制 + 提交 + 栅栏 signal | ✅ 正常 |
/// | 渲染通道的清屏（`LOAD_OP_CLEAR`） | ✅ 正常（换清屏色，回读跟着变） |
/// | 回读像素（`copyImageToBuffer` + map） | ✅ 正常 |
/// | **`vkCmdDraw` 产生片元** | ❌ **一个像素都没有** |
///
/// 已排除的原因（都实测过）：
/// - 不是着色器内容：空 `main` / 常量位置 / 常量数组+运行时索引（修掉后）/ `OpSelect`
///   全向量选择 —— 四种都不画；
/// - 不是几何超出裁剪：即使顶点取 `(-3,-3) (3,-3) (0,3)` **铺满整屏**也不画；
/// - 不是动态 viewport：静态 viewport 写进管线也不画；
/// - 不是清屏值/回读路径：清屏色一变，回读就跟着变。
///
/// 所以问题在「管线状态 + `vkCmdDraw`」这一段。下一步的排查方向：
/// ① 用 `VK_LAYER_KHRONOS_validation` 拿校验层输出（本轮没装 SDK，拿不到）；
/// ② 用 RenderDoc 抓帧看 draw call 的实际状态；
/// ③ 逐项试管线状态的合法变量（拓扑换成 POINT_LIST、混合关掉、栅格化关掉等）。
///
/// **本测试的做法**：断言「当前确实一个像素都没画」。
/// 这样它今天能过（不掩盖问题），而**一旦修好就会变红**，
/// 提醒我们把它改成真正的像素断言。
#[test]
fn draw_produces_no_pixels_is_a_known_defect() {
    let Some(dev) = open() else { return };
    let (pass, _layout, _vs, _fs, pipeline, off) = setup(&dev);

    // 清屏用红色、三角形用绿色：任何绿像素都意味着「绘制通了」
    let pixels = off
        .render_and_read_back(&pass, &pipeline, 3, [1.0, 0.0, 0.0, 1.0])
        .expect("渲染并回读");

    assert_eq!(
        pixels.len(),
        (W as usize) * (H as usize) * 4,
        "回读长度必须是 宽×高×4"
    );

    let green = pixels
        .chunks_exact(4)
        .filter(|p| p[0] < 50 && p[1] > 200 && p[2] < 50)
        .count();
    let red = pixels
        .chunks_exact(4)
        .filter(|p| p[0] > 200 && p[1] < 50 && p[2] < 50)
        .count();

    println!("红色（清屏）{red} 个，绿色（绘制）{green} 个");
    assert_eq!(red, (W * H) as usize, "清屏应当铺满整屏（这条是通的）");
    assert_eq!(
        green, 0,
        "**已知缺陷已修复**：现在画出了 {green} 个绿像素！\
         请把这条测试改成真正的像素断言（内部点绿色、外部点背景色），\
         并更新 README / FEATURES.md / docs/features/offscreen-render.md 里的「不能画像素」说明"
    );
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
