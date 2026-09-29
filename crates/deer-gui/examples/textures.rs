//! 功能示例：**通用纹理**（`RGBA8_UNORM` 的创建 / 上传 / 回读 / 采样成像素）。
//!
//! ```sh
//! cargo run -p deer-gui --example textures
//! ```
//!
//! ## 这是什么
//!
//! 「把一张任意 RGBA 图铺到屏幕上」这条链的**最小可用路径**：
//!
//! ```text
//!   create_texture_rgba8(w, h, &data)   →  上传（四通道保真）
//!   read_texture_bytes(&tex)            →  回读（证明真的传对了）
//!   draw_textured_quad(&tex, rect, tint) →  采样成像素（与 CPU 参考逐字节对照）
//! ```
//!
//! 跑完会写 `render_out/textures.png`（放大 6 倍，便于肉眼看纹理朝向是否正确）。
//!
//! ## 为什么要有这个示例（而不是「测试已经覆盖了」）
//!
//! 测试能证明**代码正确**，但证明不了**使用者拿得到**：本仓库的纪律是
//! 「每个 ✅ 功能都要有能跑的示例 + 一份指南」（见 `FEATURES.md` 第五节）。
//! 这个示例就是 `docs/features/textures.md` 的可执行版本。
//!
//! ## 这条链最反直觉的一点
//!
//! **现有片元着色器只读纹理的 R 通道**（`texture(tex, uv).r`，把 R 当覆盖率用），
//! 所以「G/B/A 有没有传对」**只看渲染出来的像素是看不出来的** ——
//! 这就是为什么本示例必须先**回读**纹理、再谈采样：
//! 回读证明「四通道都上去了」，采样证明「R 通道按预期变成了像素」。
//! 缺任何一半，判据都是空的。
//!
//! ## 无 GPU 时
//!
//! 打印原因并 `return`（与既有 GPU 示例一致）；CPU 侧出图请用
//! `cargo run -p deer-gui --example render_to_png`。

use deer_gpu::{Color, Extent, RectI};
use deer_vk::device::TextureFormat;
use deer_vk::GpuGeometryRenderer;

/// 画布尺寸：小一点，便于逐像素看。
const W: u32 = 32;
const H: u32 = 24;

/// 纹理尺寸：**与 quad 不等**，这样「缩放采样 + 朝向」两件事都被覆盖到。
const TEX_W: u32 = 4;
const TEX_H: u32 = 4;

/// 清屏色（不透明 ⇒ UNORM 转换两边都精确，逐字节判据才成立）。
const CLEAR: Color = Color::rgb(16, 16, 16);

fn main() {
    println!("=== 通用纹理：创建 / 上传 / 回读 / 采样 ===\n");

    let extent = Extent { width: W, height: H };
    let mut r = match GpuGeometryRenderer::new(0, extent, CLEAR) {
        Ok(r) => r,
        Err(e) => {
            println!("本机没有可用的 Vulkan GPU：{e}");
            println!("⇒ 出图请用 CPU 后端：cargo run -p deer-gui --example render_to_png");
            return;
        }
    };
    println!("设备：{}\n", r.device().adapter().name);

    // ─────────────────────────────────────────────────────────────────────
    // ① 造一张图：**四个通道各不相同**
    // ─────────────────────────────────────────────────────────────────────
    //
    // 只用「红色方块 + 绿色背景」那种图，抓不到「只传了 R」「通道错位」这类缺陷 ——
    // 因为 G/B/A 传错时像素仍然是红的。所以四通道必须各走各的。
    // 同时 R 通道做成**棋盘 0/255**：这样有效 alpha 只有 {0,1} 两态 ⇒ 采样判据可以是
    // **逐字节**（而不是「≤1 LSB」那种带容差的）。
    let data = texture_data();
    println!("① 纹理语料：{TEX_W}×{TEX_H}，R 通道棋盘 0/255、G/B/A 各不相同");

    // 前置断言：语料必须有区分度，否则后面的「四通道保真」是空话。
    for ch in 0..4 {
        let mut seen = std::collections::BTreeSet::new();
        for px in data.chunks_exact(4) {
            seen.insert(px[ch]);
        }
        assert!(
            seen.len() > 1,
            "前置条件不成立：通道 {ch} 的取值没有区分度（{seen:?}）—— 四通道保真判据会被架空"
        );
    }

    // ─────────────────────────────────────────────────────────────────────
    // ② 创建 + 上传
    // ─────────────────────────────────────────────────────────────────────
    let tex = r
        .device()
        .create_texture_rgba8(TEX_W, TEX_H, &data)
        .expect("创建 RGBA8 纹理");
    assert_eq!(tex.width(), TEX_W);
    assert_eq!(tex.height(), TEX_H);
    assert_eq!(tex.format(), TextureFormat::Rgba8Unorm);
    println!("② 创建 + 上传 ✅（{}×{}，RGBA8_UNORM）", tex.width(), tex.height());

    // 负例：非法参数必须在碰驱动前被拦下（这里只用 `expect_err` 断言语义，
    // 不依赖任何计数器）。
    assert!(
        r.device().create_texture_rgba8(0, TEX_H, &[]).is_err(),
        "宽为 0 必须报错，不能静默建出一张空纹理"
    );
    assert!(
        r.device()
            .create_texture_rgba8(TEX_W, TEX_H, &data[..8])
            .is_err(),
        "数据长度不足 w×h×4 必须报错"
    );
    println!("   非法参数（0 宽 / 数据不足）都被拒绝 ✅");

    // ─────────────────────────────────────────────────────────────────────
    // ③ 回读：证明**四通道**都真的上去了
    // ─────────────────────────────────────────────────────────────────────
    //
    // 这是整条链唯一能直接证明「G/B/A 传对了」的判据 —— 见文件头的说明。
    let back = r.device().read_texture_bytes(&tex).expect("回读纹理");
    assert_eq!(back.len(), data.len(), "回读长度必须 = w×h×4");
    assert_eq!(back, data, "RGBA8 上传必须逐字节保真（含 alpha）");
    println!("③ 回读 ✅ {} 字节、四通道逐字节相同", back.len());

    // ─────────────────────────────────────────────────────────────────────
    // ④ 采样成像素：纹理 quad，与 CPU 参考对照
    // ─────────────────────────────────────────────────────────────────────
    let quad = RectI::new(4, 3, 24, 18);
    let tint = Color::rgb(200, 100, 50);
    let gpu = r.draw_textured_quad(&tex, quad, tint).expect("纹理 quad");
    assert!(r.unsupported().is_empty(), "纹理 quad 不该有 unsupported");

    // 前置断言：GPU 结果里必须同时出现「画到了 tint 色」和「保持清屏色」两种像素，
    // 否则「与 CPU 一致」可能只是「两边都是纯色」这种平凡情形。
    let has_tint = gpu.chunks_exact(4).any(|p| p == [200, 100, 50, 255]);
    let has_clear = gpu
        .chunks_exact(4)
        .any(|p| p[0] == CLEAR.r && p[1] == CLEAR.g && p[2] == CLEAR.b);
    assert!(
        has_tint && has_clear,
        "前置条件不成立：GPU 结果里没有同时出现 tint 像素与清屏像素\
         （tint={has_tint} clear={has_clear}）—— 对照判据是空的"
    );

    let cpu = cpu_textured_quad(extent, TEX_W, TEX_H, &data, quad, tint);
    let worst = max_channel_diff(&gpu, &cpu);
    println!("④ 采样 ✅ 与 CPU 参考最大通道差 {worst}（棋盘 0/255 ⇒ 判据是逐字节 0 差）");
    assert_eq!(
        worst, 0,
        "纹理 quad 必须与 CPU 参考逐字节相同（最大通道差 {worst}）"
    );

    // ─────────────────────────────────────────────────────────────────────
    // ⑤ UV 朝向：直接按象限断言（**不依赖 CPU 参考**的独立判据）
    // ─────────────────────────────────────────────────────────────────────
    //
    // V 翻转是贴图最常见的 bug，而且它「看起来也是一张图」，只靠像素数量抓不到。
    // 这里用「纹理左上角必须落在 quad 左上角」这条独立事实钉住它 ——
    // 4×4 texel 铺到 24×18 像素 ⇒ 每 6×4.5 像素对应一个 texel。
    let at = |x: i32, y: i32| -> [u8; 4] {
        let i = ((y as u32 * W + x as u32) * 4) as usize;
        [gpu[i], gpu[i + 1], gpu[i + 2], gpu[i + 3]]
    };
    // 纹理 (0,0) 的 R=255（亮）⇒ quad 左上角内侧应是 tint 色；
    // 纹理 (1,0) 的 R=0（透明）⇒ 同一条行往右应是清屏色。
    assert_eq!(
        at(quad.x + 2, quad.y + 1),
        [200, 100, 50, 255],
        "quad 左上角内侧应是纹理 (0,0) 的亮 texel（若这里是清屏色 ⇒ V 翻转或 U 镜像）"
    );
    assert_eq!(
        at(quad.x + 8, quad.y + 1),
        [CLEAR.r, CLEAR.g, CLEAR.b, 255],
        "quad 左上角右侧应是纹理 (1,0) 的透明 texel"
    );
    println!("⑤ UV 朝向 ✅ 纹理 (0,0) 落在 quad 左上角（V 未翻转、U 未镜像）");

    // ─────────────────────────────────────────────────────────────────────
    // ⑥ 写出 PNG（放大 6 倍，肉眼可查）
    // ─────────────────────────────────────────────────────────────────────
    std::fs::create_dir_all("render_out").ok();
    let scale = 6usize;
    let (bw, bh) = (W as usize * scale, H as usize * scale);
    let mut big = vec![0u8; bw * bh * 4];
    for y in 0..bh {
        for x in 0..bw {
            let si = ((y / scale) * W as usize + (x / scale)) * 4;
            let di = (y * bw + x) * 4;
            big[di..di + 4].copy_from_slice(&gpu[si..si + 4]);
        }
    }
    if let Ok(png) = deer_gpu::png::encode_rgba(bw as u32, bh as u32, &big) {
        let path = "render_out/textures.png";
        std::fs::write(path, &png).ok();
        println!("\n已写出 {path}（{W}×{H} 放大 {scale} 倍）");
    }

    // ─────────────────────────────────────────────────────────────────────
    // ⑦ 边界（随里程碑推进要跟着改，别留旧说法）
    // ─────────────────────────────────────────────────────────────────────
    println!("\n=== 当前边界 ===");
    println!("  ✅ 能：建 RGBA8_UNORM 纹理、上传（四通道逐字节保真）、回读、铺到矩形上采样");
    println!("  ⚠️  采样只消费 **R 通道**（当覆盖率）⇒ G/B/A 只靠回读证明，不体现在像素里");
    println!("  ❌ 不能：按 **RGB 调制** —— 需要新的片元着色器（尚未做，见 FEATURES.md 第四节）");
    println!("  ❌ 不能：在**窗口路径**贴任意纹理（`draw_textured_quad` 目前只有离屏侧）");
    println!("  ❌ 不能：把纹理作为 `DrawCmd` 进 `DrawList`（`DrawList` 是 deer-gpu 的契约，不在本项 scope）");

    r.device().wait_idle().expect("空闲等待");
}

/// 4×4 纹理语料：R 通道棋盘 0/255（→ 有效 alpha 只有 {0,1}），G/B/A 各不相同。
///
/// **为什么 R 走棋盘**：R 是唯一被采样的通道，做成 0/255 两态 ⇒ 不透明判据成立（逐字节），
/// 而不是退化成「≤1 LSB」。**为什么 G/B/A 各不相同**：让「只传了 R」「通道错位」当场露馅。
fn texture_data() -> Vec<u8> {
    let mut data = Vec::with_capacity((TEX_W * TEX_H * 4) as usize);
    for y in 0..TEX_H {
        for x in 0..TEX_W {
            let on = (x + y) % 2 == 0;
            data.push(if on { 255 } else { 0 }); // R：棋盘（被采样）
            data.push((x * 60 + 7) as u8); // G
            data.push((y * 60 + 11) as u8); // B
            data.push((x * 16 + y * 48 + 31) as u8); // A
        }
    }
    data
}

/// 纹理 quad 的 **CPU 参考**（独立实现，照 CPU 基线 `null.rs::blend_cov` 的公式写）：
///
/// ```text
///   像素中心 (x+0.5, y+0.5) → 归一化 uv → NEAREST texel
///   cov = tex[texel].R / 255
///   a   = clamp(tint.a, 0, 1) * cov
///   dst.rgb = round(tint.rgb * a + dst.rgb * (1 - a))
/// ```
///
/// uv 用**像素中心**求值：顶点 uv 是「像素边界」语义（`u = (px - quad.x)/quad.w`），
/// 片元在像素中心求值 ⇒ 采样点落在 texel 中心，`NEAREST` 无平局、逐像素精确。
fn cpu_textured_quad(
    extent: Extent,
    tex_w: u32,
    tex_h: u32,
    tex: &[u8],
    quad: RectI,
    tint: Color,
) -> Vec<u8> {
    let w = extent.width.max(1);
    let h = extent.height.max(1);
    let mut fb = Vec::with_capacity((w * h * 4) as usize);
    for _ in 0..(w * h) {
        fb.extend_from_slice(&[CLEAR.r, CLEAR.g, CLEAR.b, 255]);
    }
    let src = [tint.r, tint.g, tint.b];
    for y in quad.y..quad.bottom() {
        for x in quad.x..quad.right() {
            if x < 0 || y < 0 || x >= w as i32 || y >= h as i32 {
                continue;
            }
            let u = (x as f32 + 0.5 - quad.x as f32) / quad.w as f32;
            let v = (y as f32 + 0.5 - quad.y as f32) / quad.h as f32;
            let tx = ((u * tex_w as f32).floor() as i32).clamp(0, tex_w as i32 - 1) as u32;
            let ty = ((v * tex_h as f32).floor() as i32).clamp(0, tex_h as i32 - 1) as u32;
            let cov = tex[((ty * tex_w + tx) * 4) as usize] as f32 / 255.0;
            let a = tint.a.clamp(0.0, 1.0) * cov.clamp(0.0, 1.0);
            let i = ((y as u32 * w + x as u32) * 4) as usize;
            let inv = 1.0 - a;
            for c in 0..3 {
                fb[i + c] = (src[c] as f32 * a + fb[i + c] as f32 * inv).round() as u8;
            }
            fb[i + 3] = ((a + (fb[i + 3] as f32 / 255.0) * inv).clamp(0.0, 1.0) * 255.0).round() as u8;
        }
    }
    fb
}

/// 两张图的**最大单通道差**（0 = 逐字节相同）。
fn max_channel_diff(a: &[u8], b: &[u8]) -> u8 {
    assert_eq!(a.len(), b.len(), "对照的两张图长度必须一致");
    a.iter()
        .zip(b.iter())
        .map(|(x, y)| x.abs_diff(*y))
        .max()
        .unwrap_or(0)
}
