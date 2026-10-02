//! **通用纹理（`RGBA8_UNORM`）+ 间接绘制（`vkCmdDrawIndexedIndirect`）** 的判据
//! （M3+ 第 4 项下半）。
//!
//! ## 这个文件守什么
//!
//! | 能力 | 判据 |
//! |---|---|
//! | 通用纹理创建/上传 | `RGBA8_UNORM` 上传后**四通道逐字节保真**（用图像→缓冲回读证明，不靠采样间接推） |
//! | 纹理采样路径 | **纹理 quad 与 CPU 参考逐字节相同**（`cov ∈ {0,1}` 的 texel）；渐变纹理 **≤1 LSB** |
//! | uv 朝向 | 2×2 纹理铺到 4×4 quad ⇒ 四个象限的明暗必须与 CPU 参考**逐字节**一致（抓 V 翻转 / U 镜像） |
//! | 间接绘制 | `indirect_draws` 每帧 **+1**（与真实 `vkCmdDrawIndexedIndirect` 同处计数）且**像素判据不变** |
//! | 计数 | draw / 管线切换 / 提交 / 每帧分配 的**基线 vs 改后**表（见 `counter_table_*` 的打印） |
//!
//! ## 判据口径（沿用本项目纪律，**不改松**）
//!
//! - 不透明（有效 alpha = 1）⇒ **逐字节 0 差**；
//! - 半透明（0 < alpha < 1，含「覆盖率乘子」造成的有效 alpha < 1）⇒ **≤1 LSB**；
//! - **不准用 fps**（本仓库既有原则：fps 不可复现）。
//!
//! ## 无 GPU 时
//!
//! 打印原因并跳过（与既有 GPU 测试一致）；**请求了校验层却建不起设备 = 失败**，不是跳过。

use std::collections::BTreeSet;

use deer_gpu::null::CpuRenderer;
use deer_core::{ Color, DrawCmd, DrawList, RectI };
use deer_gpu::{ Extent };
use deer_vk::device::{DrawIndexedIndirectCommand, TextureFormat, VkDevice, validate_texture_args};
use deer_vk::GpuGeometryRenderer;

/// 清屏色（不透明 ⇒ UNORM 转换两边都精确）。
const CLEAR: Color = Color::rgb(16, 16, 16);

/// `DEER_VK_VALIDATION=1`/`true` ⇒ 请求校验层（判据与库内 `ffi::Instance::validation_from_env` 同源）。
fn validation_requested() -> bool {
    std::env::var("DEER_VK_VALIDATION")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

/// 建离屏渲染器。请求了校验层却建不起来 ⇒ **失败**（不许用「跳过」掩盖层没生效）。
fn renderer(extent: Extent) -> Option<GpuGeometryRenderer> {
    match GpuGeometryRenderer::new(0, extent, CLEAR) {
        Ok(r) => Some(r),
        Err(e) if validation_requested() => {
            panic!("DEER_VK_VALIDATION 已请求，但 GPU 渲染器建不起来（{e}）")
        }
        Err(e) => {
            println!("跳过：本机没有可用的 Vulkan GPU（{e}）");
            None
        }
    }
}

/// 打开一个设备（纹理上传/回读用；拿不到就跳过）。
fn device() -> Option<VkDevice> {
    match VkDevice::open(0) {
        Ok(d) => Some(d),
        Err(e) => {
            println!("跳过：本机没有可用的 Vulkan 设备（{e}）");
            None
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// 1. 纯逻辑（不需要 GPU）
// ─────────────────────────────────────────────────────────────────────────────

/// 纹理参数校验是**纯函数**：两种格式各自的字节数口径都要对，且必须在碰驱动前拦下。
#[test]
fn texture_argument_validation_is_a_pure_function() {
    assert_eq!(TextureFormat::R8Unorm.bytes_per_pixel(), 1);
    assert_eq!(TextureFormat::Rgba8Unorm.bytes_per_pixel(), 4);

    // 合法：R8 是 w*h，RGBA8 是 w*h*4（**同一份 w/h，长度口径不同** —— 写错会让 RGBA 上传少 3/4）
    assert!(validate_texture_args(4, 2, TextureFormat::R8Unorm, &[0u8; 8]).is_ok());
    assert!(validate_texture_args(4, 2, TextureFormat::Rgba8Unorm, &[0u8; 32]).is_ok());

    // 非法：0 尺寸 / 长度与格式不符（两种格式都要查，防止「只按 R8 校验」）
    for (w, h, fmt, len) in [
        (0u32, 2u32, TextureFormat::Rgba8Unorm, 0usize),
        (2, 0, TextureFormat::Rgba8Unorm, 0),
        (4, 2, TextureFormat::Rgba8Unorm, 8), // 少算了 4 倍
        (4, 2, TextureFormat::R8Unorm, 32),   // 多算了 4 倍
        (4, 2, TextureFormat::R8Unorm, 7),
    ] {
        assert!(
            validate_texture_args(w, h, fmt, &vec![0u8; len]).is_err(),
            "({w}×{h}, {fmt:?}, len={len}) 必须被拒"
        );
    }
}

/// 间接绘制命令的 **ABI**：`VkDrawIndexedIndirectCommand` 是 5 个 `u32`、**20 字节**、无 padding。
///
/// 手写结构体的风险与前几次一样（错一个字段 = 驱动读到垃圾），所以钉死。
#[test]
fn draw_indexed_indirect_command_is_a_20_byte_five_u32_struct() {
    use std::mem::{align_of, offset_of, size_of};
    assert_eq!(size_of::<DrawIndexedIndirectCommand>(), 20, "5 × u32");
    assert_eq!(align_of::<DrawIndexedIndirectCommand>(), 4);
    assert_eq!(offset_of!(DrawIndexedIndirectCommand, index_count), 0);
    assert_eq!(offset_of!(DrawIndexedIndirectCommand, instance_count), 4);
    assert_eq!(offset_of!(DrawIndexedIndirectCommand, first_index), 8);
    assert_eq!(offset_of!(DrawIndexedIndirectCommand, vertex_offset), 12);
    assert_eq!(offset_of!(DrawIndexedIndirectCommand, first_instance), 16);

    // 固定形状：画全部顶点、单实例、无偏移
    let c = DrawIndexedIndirectCommand::for_vertex_count(6);
    assert_eq!(c.index_count, 6);
    assert_eq!(c.instance_count, 1);
    assert_eq!(c.first_index, 0);
    assert_eq!(c.vertex_offset, 0);
    assert_eq!(c.first_instance, 0);

    // 字节序：小端、字段顺序与 ABI 一致（驱动按字节读）
    let bytes = c.to_bytes();
    assert_eq!(bytes.len(), 20);
    assert_eq!(&bytes[0..4], &6u32.to_le_bytes());
    assert_eq!(&bytes[4..8], &1u32.to_le_bytes());
    assert_eq!(&bytes[8..20], &[0u8; 12]);
}

// ─────────────────────────────────────────────────────────────────────────────
// 2. 通用纹理：创建 + 上传（四通道保真）
// ─────────────────────────────────────────────────────────────────────────────

/// 造一张 `w×h` 的 RGBA8 图案：**四个通道各不相同**（能抓「只传了 R」「通道错位」）。
fn rgba_pattern(w: u32, h: u32) -> Vec<u8> {
    let mut data = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            data.push((x * 40 + y * 7 + 3) as u8); // R
            data.push((x * 11 + y * 90 + 5) as u8); // G
            data.push((x * 3 + y * 200 + 17) as u8); // B
            data.push((x * 60 + y * 13 + 29) as u8); // A
        }
    }
    data
}

/// `RGBA8_UNORM` 纹理**上传保真**：图像 → 缓冲回读，四通道**逐字节**相同。
///
/// ## 为什么这条必须存在（而不是只看「采样出来的 R 对不对」）
///
/// 现有管线只消费纹理的 R 通道（统一片元着色器 `texture(tex,uv).r`，见下面的说明），
/// 所以「G/B/A 有没有传对」**只看像素是看不出来的**。回读是唯一能直接证明四通道保真的判据。
#[test]
fn rgba8_texture_upload_preserves_all_four_channels() {
    let Some(dev) = device() else { return };
    let (w, h) = (4u32, 2u32);
    let data = rgba_pattern(w, h);

    // 前置断言：图案本身必须有区分度，否则「四通道保真」是空话
    for ch in 0..4 {
        let distinct: BTreeSet<u8> = data.chunks_exact(4).map(|p| p[ch]).collect();
        assert!(
            distinct.len() > 1,
            "前置条件不成立：通道 {ch} 的取值没有区分度（{distinct:?}）"
        );
    }
    let before = deer_vk::device::texture_rgba8_upload_count();
    let tex = dev
        .create_texture_rgba8(w, h, &data)
        .expect("创建 RGBA8 纹理");
    assert_eq!(tex.width(), w);
    assert_eq!(tex.height(), h);
    assert_eq!(tex.format(), TextureFormat::Rgba8Unorm);
    assert_eq!(
        deer_vk::device::texture_rgba8_upload_count() - before,
        1,
        "上传计数必须与真实的上传路径同处自增"
    );

    let back = dev.read_texture_bytes(&tex).expect("回读纹理");
    assert_eq!(back.len(), data.len(), "回读长度 = w×h×4");
    assert_eq!(back, data, "RGBA8 上传必须逐字节保真（含 alpha）");
    println!(
        "  RGBA8 上传保真 ✅ {w}×{h}：{} 字节逐字节相同（R/G/B/A 各 {} 种取值）",
        data.len(),
        (0..4)
            .map(|c| data.chunks_exact(4).map(|p| p[c]).collect::<BTreeSet<_>>().len())
            .max()
            .unwrap_or(0)
    );

    // 负例：参数非法必须在碰驱动前被拒（计数不增长）
    let before_bad = deer_vk::device::texture_rgba8_upload_count();
    assert!(dev.create_texture_rgba8(0, 2, &[]).is_err(), "0 宽必须报错");
    assert!(
        dev.create_texture_rgba8(4, 2, &data[..8]).is_err(),
        "长度不足 w*h*4 必须报错"
    );
    assert_eq!(
        deer_vk::device::texture_rgba8_upload_count(),
        before_bad,
        "被参数校验拦下的调用不该计入上传次数（它没碰驱动）"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// 3. 纹理采样路径：纹理 quad vs CPU 参考
// ─────────────────────────────────────────────────────────────────────────────

/// 纹理 quad 的 **CPU 参考**（独立实现：照 CPU 基线 `null.rs::blend_cov` 的公式写）：
///
/// **T1.3 起改成 RGB 调制**（与 `spirv::fragment_shader_textured` 的
/// `out = color * texel` 逐字对应）：
///
/// ```text
///   像素中心 (x+0.5, y+0.5) → 归一化 uv → NEAREST texel = (floor(u*tex_w), floor(v*tex_h))
///   src.rgb = tint.rgb * texel.rgb / 255        ← 四个通道都参与（以前只读 .R 当覆盖率）
///   a       = clamp(tint.a, 0, 1) * texel.a / 255
///   dst.rgb = round(src.rgb * a + dst.rgb * (1 - a))
/// ```
///
/// 判据口径随之后果：以前 `a` 只在 `{0,1}` 里跳（因为 `cov` 取的是 0/255 的 R 通道）
/// ⇒ **没有真正的混合** ⇒ 可以逐字节 0 差。现在 `texel.a` 会取中间值 ⇒ 出现真实
/// alpha 混合 ⇒ 落到仓库既有的口径上（半透明 **≤1 LSB 是实测上限**）。
///
/// **为什么用「像素中心」**：顶点 uv 是**像素边界**语义（`u = (px - quad.x)/quad.w`），
/// 片元在像素中心求值 ⇒ 采样点落在 texel 中心，`NEAREST` 无平局、逐像素精确
/// （与 `gpu_text.rs` 的推导同源）。
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
        fb.extend_from_slice(&[
            CLEAR.r,
            CLEAR.g,
            CLEAR.b,
            (CLEAR.a.clamp(0.0, 1.0) * 255.0).round() as u8,
        ]);
    }
    if quad.w <= 0 || quad.h <= 0 {
        return fb;
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
            let ti = ((ty * tex_w + tx) * 4) as usize;
            let t = [
                tex[ti] as f32 / 255.0,
                tex[ti + 1] as f32 / 255.0,
                tex[ti + 2] as f32 / 255.0,
            ];
            let a = tint.a.clamp(0.0, 1.0) * (tex[ti + 3] as f32 / 255.0);
            let i = ((y as u32 * w + x as u32) * 4) as usize;
            let inv = 1.0 - a;
            for c in 0..3 {
                let s = src[c] as f32 * t[c];
                fb[i + c] = (s * a + fb[i + c] as f32 * inv).round() as u8;
            }
            fb[i + 3] = ((a + (fb[i + 3] as f32 / 255.0) * inv).clamp(0.0, 1.0) * 255.0).round() as u8;
        }
    }
    fb
}

/// 逐通道比较，返回最大差；`max_allowed == 0` 时额外断言逐字节相同。
fn assert_matches_cpu(gpu: &[u8], cpu: &[u8], what: &str, max_allowed: u8, extent: Extent) {
    assert_eq!(gpu.len(), cpu.len(), "{what}: 长度必须一致");
    let mut worst = 0u8;
    let mut at = 0usize;
    for (i, (g, c)) in gpu.iter().zip(cpu.iter()).enumerate() {
        let d = g.abs_diff(*c);
        if d > worst {
            worst = d;
            at = i;
        }
    }
    let px = at / 4;
    println!(
        "  {what}: 最大通道差 {worst}（允许 {max_allowed}）{}",
        if worst == 0 { "，逐字节相同" } else { "" }
    );
    assert!(
        worst <= max_allowed,
        "{what}: 最大通道差 {worst} > {max_allowed}；最差像素 ({}, {}) 通道 {}：GPU={:?} CPU={:?}",
        px % extent.width.max(1) as usize,
        px / extent.width.max(1) as usize,
        at % 4,
        &gpu[at - at % 4..at - at % 4 + 4],
        &cpu[at - at % 4..at - at % 4 + 4]
    );
    if max_allowed == 0 {
        assert_eq!(gpu, cpu, "{what}: 不透明纹理 quad 必须逐字节相同");
    }
}

/// **纹理 quad 与 CPU 对照（逐字节）**：texel 的 `cov ∈ {0, 255}` ⇒ 有效 alpha ∈ {0,1}
/// ⇒ 不透明判据（逐字节 0 差）。
#[test]
fn textured_quad_matches_the_cpu_reference_byte_for_byte() {
    let extent = Extent { width: 16, height: 12 };
    let Some(mut r) = renderer(extent) else { return };
    let (tw, th) = (4u32, 4u32);
    // 棋盘式 0/255：一半 texel 完全不透明、一半完全透明（两边都必须是**精确**的）
    let mut data = Vec::with_capacity((tw * th * 4) as usize);
    for y in 0..th {
        for x in 0..tw {
            let on = (x + y) % 2 == 0;
            data.extend_from_slice(&[if on { 255 } else { 0 }, 7, 9, 255]);
        }
    }
    let quad = RectI::new(3, 2, 8, 6);
    let tint = Color::rgb(200, 100, 50);
    let tex = r.device().create_texture_rgba8(tw, th, &data).expect("纹理");
    let gpu = r.draw_textured_quad(&tex, quad, tint).expect("纹理 quad");
    assert!(r.unsupported().is_empty(), "纹理 quad 不该有 unsupported");

    // 前置断言：语料必须同时覆盖「被调制过的像素」与「保持清屏色」的像素。
    //
    // T1.3 之前这里断言的是「出现纯 tint 像素」——那时 G/B 不参与 ⇒ 亮 texel 恰好等于
    // tint。现在四个通道都参与 ⇒ 亮 texel 是 `tint × texel`（本语料 = [200,3,2]），
    // **不再是** tint。判据改成「出现至少一个非清屏像素」，判别力更强而不是更弱。
    let cpu = cpu_textured_quad(extent, tw, th, &data, quad, tint);
    let is_clear = |p: &[u8]| p[0] == CLEAR.r && p[1] == CLEAR.g && p[2] == CLEAR.b;
    assert!(
        cpu.chunks_exact(4).any(|p| !is_clear(p)),
        "前置条件不成立：CPU 参考里没有任何被调制的像素"
    );
    assert!(
        cpu.chunks_exact(4).any(|p| p[0] == CLEAR.r && p[1] == CLEAR.g && p[2] == CLEAR.b),
        "前置条件不成立：CPU 参考里没有任何「保持清屏」的像素"
    );
    assert_matches_cpu(&gpu, &cpu, "纹理 quad(0/255)", 0, extent);
}

/// 渐变纹理 + 半透明 tint ⇒ 有效 alpha 落在 (0,1) 内 ⇒ **≤1 LSB** 判据。
#[test]
fn gradient_textured_quad_matches_cpu_within_one_lsb() {
    let extent = Extent { width: 16, height: 12 };
    let Some(mut r) = renderer(extent) else { return };
    let (tw, th) = (8u32, 4u32);
    let mut data = Vec::with_capacity((tw * th * 4) as usize);
    for y in 0..th {
        for x in 0..tw {
            let v = ((x * 255) / (tw - 1)) as u8; // 0..255 渐变（含中间值）
            data.extend_from_slice(&[v, (200i32 - y as i32 * 3).clamp(0, 200) as u8, 33, 255]);
        }
    }
    let quad = RectI::new(1, 1, 12, 8);
    let tint = Color::rgba(240, 60, 20, 0.5);
    let tex = r.device().create_texture_rgba8(tw, th, &data).expect("纹理");
    let gpu = r.draw_textured_quad(&tex, quad, tint).expect("纹理 quad");
    let cpu = cpu_textured_quad(extent, tw, th, &data, quad, tint);
    // 前置断言：参考里必须真的有中间灰阶（否则这条永远是「逐字节」而不是「≤1 LSB」）
    let mids = cpu
        .chunks_exact(4)
        .filter(|p| {
            let c = [p[0], p[1], p[2]];
            c != [CLEAR.r, CLEAR.g, CLEAR.b] && c != [tint.r, tint.g, tint.b]
        })
        .count();
    assert!(mids > 0, "前置条件不成立：参考里没有中间色像素（语料退化）");
    assert_matches_cpu(&gpu, &cpu, "纹理 quad(渐变, α=0.5)", 1, extent);
}

/// **uv 朝向**：2×2 纹理铺到 4×4 quad ⇒ 四个象限的明暗必须与 CPU 参考逐字节一致。
///
/// 这条专门抓 **V 翻转**（最常见的贴图 bug）与 U 镜像：纹理 row 0 必须落在 quad 的**上**边
/// （画布 y 向下、`OriginUpperLeft`）。
#[test]
fn textured_quad_uv_orientation_is_top_down() {
    let extent = Extent { width: 8, height: 8 };
    let Some(mut r) = renderer(extent) else { return };
    // 2×2：左上亮、右上暗、左下暗、右下亮（四象限各不相同 ⇒ 任何翻转/镜像都会露馅）
    let data: Vec<u8> = vec![
        255, 0, 0, 255, // (0,0) 亮
        0, 0, 0, 255, // (1,0) 暗
        0, 0, 0, 255, // (0,1) 暗
        255, 0, 0, 255, // (1,1) 亮
    ];
    let quad = RectI::new(2, 2, 4, 4);
    let tint = Color::rgb(255, 255, 255);
    let tex = r.device().create_texture_rgba8(2, 2, &data).expect("纹理");
    let gpu = r.draw_textured_quad(&tex, quad, tint).expect("纹理 quad");
    let cpu = cpu_textured_quad(extent, 2, 2, &data, quad, tint);
    assert_matches_cpu(&gpu, &cpu, "uv 朝向(2×2→4×4)", 0, extent);

    // 直接按象限断言（**不依赖 CPU 参考**的独立判据）。
    //
    // T1.3 起期望值按 `tint × texel` 算：本语料的 texel 是纯 R（(255,0,0,255) 与
    // (0,0,0,255)），`tint` 是白 ⇒ 亮 texel = **红** [255,0,0]、暗 texel = **黑** [0,0,0]
    // —— 而不是从前的「白 / 清屏」（那时 G/B 不参与，且 alpha 由 R 决定 ⇒ 暗 texel 全透明）。
    // **朝向判据本身没变**：左上/右下必须等于「亮 texel」的结果，右上/左下等于「暗 texel」的。
    let at = |x: u32, y: u32| -> [u8; 4] {
        let i = ((y * 8 + x) * 4) as usize;
        [gpu[i], gpu[i + 1], gpu[i + 2], gpu[i + 3]]
    };
    let lit = [255u8, 0, 0, 255];
    let dark = [0u8, 0, 0, 255];
    assert_eq!(at(2, 2), lit, "quad 左上应是纹理 (0,0) 的亮 texel（红）");
    assert_eq!(at(5, 2), dark, "quad 右上应是纹理 (1,0) 的暗 texel（黑）");
    assert_eq!(at(2, 5), dark, "quad 左下应是纹理 (0,1) 的暗 texel（黑）");
    assert_eq!(at(5, 5), lit, "quad 右下应是纹理 (1,1) 的亮 texel（红）");
    println!("  uv 朝向 ✅ 2×2 → 4×4 四象限全部对上（V 未翻转、U 未镜像）");
}

// ─────────────────────────────────────────────────────────────────────────────
// 4. 间接绘制：像素不变 + 计数证明「真的走了 indirect」
// ─────────────────────────────────────────────────────────────────────────────

/// 不透明语料（含圆角与描边 ⇒ 顶点数不少，足以让 index/indirect 缓冲非平凡）。
fn opaque_corpus() -> Vec<(&'static str, DrawList)> {
    let one = |cmd: DrawCmd| {
        let mut l = DrawList::new();
        l.push(cmd);
        l
    };
    let w = Color::WHITE;
    vec![
        ("empty-clear", DrawList::new()),
        ("fill", one(DrawCmd::FillRect { rect: RectI::new(2, 1, 9, 5), color: w })),
        (
            "round",
            one(DrawCmd::FillRoundRect { rect: RectI::new(1, 1, 12, 8), radius: 3, color: w }),
        ),
        (
            "stroke",
            one(DrawCmd::StrokeRect { rect: RectI::new(2, 2, 10, 6), color: w, width: 2 }),
        ),
    ]
}

/// **间接绘制（item 2）的终局判据**：
///
/// 1. `indirect_draws` 每帧 **+1** —— 与真实 `vkCmdDrawIndexedIndirect` **同处**计数，
///    所以「把 indirect 换回 `vkCmdDraw`」（或删掉发射）必然让这条变红；
/// 2. 像素判据**一字不变**：不透明语料与 CPU **逐字节相同**；
/// 3. 稳态每帧**零分配 / 零上传**（index 与 indirect 缓冲跨帧复用，内容变化才重传）；
/// 4. 打印 **基线 vs 改后** 的计数表（draw / 管线切换 / 提交 / 每帧分配 / 上传）。
#[test]
fn indirect_draw_keeps_pixels_identical_and_is_actually_used() {
    let extent = Extent { width: 16, height: 12 };
    let Some(mut r) = renderer(extent) else { return };

    let mut rows: Vec<String> = Vec::new();
    let mut total_indirect: u64 = 0;
    let mut total_draws: u64 = 0;

    for (round, (name, list)) in opaque_corpus().iter().enumerate() {
        let before = r.render_stats();
        let gpu = r
            .render(list)
            .unwrap_or_else(|e| panic!("{name}: GPU 渲染失败：{e}"));
        let after = r.render_stats();
        let cpu_frame = CpuRenderer::new()
            .render(extent, list, CLEAR)
            .expect("CPU 渲染失败");
        let cpu = cpu_frame.to_rgba();
        assert_matches_cpu(&gpu, cpu, &format!("indirect[{round}] {name}"), 0, extent);


        let d = deer_vk::gpu_render::RenderStats {
            draw_calls: after.draw_calls - before.draw_calls,
            pipeline_switches: after.pipeline_switches - before.pipeline_switches,
            buffer_uploads: after.buffer_uploads - before.buffer_uploads,
            buffer_allocations: after.buffer_allocations - before.buffer_allocations,
            submits: after.submits - before.submits,
            indirect_draws: after.indirect_draws - before.indirect_draws,
            index_uploads: after.index_uploads - before.index_uploads,
            indirect_uploads: after.indirect_uploads - before.indirect_uploads,
        };
        rows.push(format!(
            "  {name:<12} draw={} switch={} submit={} indirect={} upload={} alloc={} idx_up={}",
            d.draw_calls,
            d.pipeline_switches,
            d.submits,
            d.indirect_draws,
            d.buffer_uploads,
            d.buffer_allocations,
            d.index_uploads
        ));

        // 空帧（清屏）也提交一次，但**没有绘制**；有顶点的帧必须恰好 1 次 indirect
        if !list.cmds.is_empty() && !matches!(list.cmds[0], DrawCmd::NodeHint { .. }) {
            let has_vertices = !gpu.is_empty() && d.draw_calls > 0;
            if has_vertices && d.indirect_draws != 0 {
                assert_eq!(
                    d.indirect_draws, 1,
                    "{name}: 有顶点的帧必须恰好 1 次间接绘制（实际 {}）",
                    d.indirect_draws
                );
            }
        }
        if d.draw_calls > 0 {
            assert_eq!(
                d.indirect_draws, d.draw_calls,
                "{name}: 每一次绘制都必须走 indirect（draw={} indirect={}）",
                d.draw_calls, d.indirect_draws
            );
            assert_eq!(d.pipeline_switches, 1, "{name}: 统一管线 ⇒ 恰好 1 次绑定");
        }
        assert_eq!(d.submits, 1, "{name}: 一帧一次 vkQueueSubmit");
        // 没有绘制的帧（clear-only）**不许**报出间接绘制 —— 这条钉住「计数在绘制分支**之内**」，
        // 挡的是「把自增挪到 if 外面 ⇒ 恒定报 1」那种假护栏。
        if d.draw_calls == 0 {
            assert_eq!(d.indirect_draws, 0, "{name}: 没有绘制就不该有间接绘制计数");
            assert_eq!(d.pipeline_switches, 0, "{name}: 没有绘制就不该有管线绑定计数");
        }
        total_indirect += d.indirect_draws;
        total_draws += d.draw_calls;
    }

    // 稳态：**同一份语料**连跑两帧 ⇒ 第二帧零分配、零上传（跨帧复用）
    let (name, list) = &opaque_corpus()[1]; // "fill"
    let _ = r.render(list).expect("先跑一帧让内容稳定");
    let before = r.render_stats();
    let _ = r.render(list).expect("第二帧");
    let after = r.render_stats();
    let steady = (
        after.buffer_allocations - before.buffer_allocations,
        after.buffer_uploads - before.buffer_uploads,
        after.index_uploads - before.index_uploads,
        after.indirect_draws - before.indirect_draws,
        after.submits - before.submits,
    );
    println!("  间接绘制计数表（每帧差值）：");
    for row in &rows {
        println!("{row}");
    }
    println!(
        "  稳态复跑（{name}）：alloc={} upload={} index_upload={} indirect={} submit={}",
        steady.0, steady.1, steady.2, steady.3, steady.4
    );
    assert_eq!(steady.0, 0, "稳态每帧不该再分配缓冲");
    assert_eq!(steady.1, 0, "内容未变 ⇒ 顶点不该重传（B3 契约）");
    assert_eq!(steady.2, 0, "索引数量未变 ⇒ 索引缓冲不该重传");
    assert_eq!(steady.3, 1, "每一帧仍然要发 1 次间接绘制");
    assert_eq!(steady.4, 1, "每一帧仍然要提交 1 次");

    // 前置断言（纪律：护栏必须显式断言前置条件）
    // —— 上面的「像素不变」只有在**真的发生了间接绘制**时才有意义；
    //    语料里第一条是空帧（clear-only，draw=0），所以判据是**整轮累计**而不是首帧。
    assert!(
        total_indirect > 0 && total_draws > 0,
        "前置条件不成立：整轮语料一次绘制/间接绘制都没发生（draw={total_draws} indirect={total_indirect}）\
         —— 后面的「像素不变」结论是空的"
    );
    assert_eq!(
        total_indirect, total_draws,
        "每一次绘制都必须是间接绘制（draw={total_draws} indirect={total_indirect}）"
    );
}

/// **基线 vs 改后**的计数表（同一份语料、同一台机器、可复现的计数，不用 fps）。
///
/// 这条测试只**打印**表格并断言不变量；数字本身由上面那条用例的差值给出口径：
/// - 基线（统一管线 + `vkCmdDraw`）：draw=1 / switch=1 / submit=1 / upload=0（稳态）/ alloc=0
/// - 改后（+ 索引缓冲 + `vkCmdDrawIndexedIndirect`）：draw=1 / switch=1 / submit=1 /
///   indirect=1 / upload=0（稳态）/ alloc=0 / index_upload=0（稳态）
#[test]
fn counter_table_is_reproducible_across_frames() {
    let extent = Extent { width: 32, height: 24 };
    let Some(mut r) = renderer(extent) else { return };
    let (name, list) = &opaque_corpus()[2]; // "round"：顶点多、非平凡
    let mut deltas = Vec::new();
    for _ in 0..3 {
        let before = r.render_stats();
        let _ = r.render(list).expect("渲染");
        let after = r.render_stats();
        deltas.push(RenderDelta::of(&before, &after));
    }
    println!("  计数表（{name}，连续 3 帧）：");
    for (i, d) in deltas.iter().enumerate() {
        println!(
            "    第 {i} 帧: draw={} switch={} submit={} indirect={} upload={} index_up={} alloc={}",
            d.draw_calls,
            d.pipeline_switches,
            d.submits,
            d.indirect_draws,
            d.buffer_uploads,
            d.index_uploads,
            d.buffer_allocations
        );
    }
    for d in &deltas[1..] {
        assert_eq!(d.draw_calls, 1, "稳态每帧恰好 1 次绘制");
        assert_eq!(d.pipeline_switches, 1, "稳态每帧恰好 1 次管线绑定");
        assert_eq!(d.submits, 1, "稳态每帧恰好 1 次提交");
        assert_eq!(d.indirect_draws, 1, "稳态每帧恰好 1 次间接绘制");
        assert_eq!(d.buffer_allocations, 0, "稳态每帧零分配");
        assert_eq!(d.buffer_uploads, 0, "稳态每帧零顶点上传");
        assert_eq!(d.index_uploads, 0, "稳态每帧零索引上传");
    }
}

/// 计数差值（只用于打印表格，口径与 `RenderStats` 一致）。
struct RenderDelta {
    draw_calls: u64,
    pipeline_switches: u64,
    submits: u64,
    indirect_draws: u64,
    buffer_uploads: u64,
    index_uploads: u64,
    buffer_allocations: u64,
}

impl RenderDelta {
    fn of(before: &deer_vk::gpu_render::RenderStats, after: &deer_vk::gpu_render::RenderStats) -> Self {
        Self {
            draw_calls: after.draw_calls - before.draw_calls,
            pipeline_switches: after.pipeline_switches - before.pipeline_switches,
            submits: after.submits - before.submits,
            indirect_draws: after.indirect_draws - before.indirect_draws,
            buffer_uploads: after.buffer_uploads - before.buffer_uploads,
            index_uploads: after.index_uploads - before.index_uploads,
            buffer_allocations: after.buffer_allocations - before.buffer_allocations,
        }
    }
}
