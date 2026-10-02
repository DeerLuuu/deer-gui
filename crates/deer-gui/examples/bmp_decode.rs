//! 功能示例：**AF-1 BMP 图像解码**（`deer_gpu::image`，零第三方依赖）。
//!
//! ```sh
//! cargo run -p deer-gui --example bmp_decode
//! ```
//!
//! ## 这是什么
//!
//! 「图片文件 → RGBA8 像素 → 纹理」这条链的最小可用路径：
//!
//! ```text
//!   deer_gpu::image::decode_bmp(&bytes)      →  BmpImage { width, height, pixels }
//!   deer_gpu::png::encode_rgba(w, h, &px)    →  重编码成 PNG（回环判据：逐字节可复现）
//!   deer_gpu::image::upload_bmp_to_texture() →  直喂 create_texture + upload_texture
//! ```
//!
//! 跑完会写 `render_out/bmp_decode.png`（24 位语料的解码结果，放大 8 倍）与
//! `render_out/bmp_decode_bitfields.png`（带 alpha 的 BITFIELDS 变体）。
//!
//! ## 判据
//!
//! 解码正确性不靠「看起来对」：语料的期望 RGBA **独立推导**，解码结果与之逐字节对照；
//! 再把两份像素各自经自有 PNG 编码器重编码 —— **两份 PNG 必须逐字节相同**。
//! 语料刻意 R≠B、每行颜色不同 ⇒ 「通道序 BGR→RGB 改错」「底行优先漏翻转」的变异必红。
//!
//! ## 无 GPU 时
//!
//! 解码、回环、HAL 上传（CPU 参考后端）都**不需要 GPU**；只有最后一步
//! 「Vulkan 纹理回读保真」需要 —— 没有就打印原因并跳过（与既有 GPU 示例一致）。

use deer_gpu::image::{decode_bmp, upload_bmp_to_texture};
use deer_gpu::png;

const W: u32 = 20;
const H: u32 = 16;

fn main() {
    println!("=== AF-1 BMP 图像解码：24/32 位、底行优先 → RGBA8 → 纹理 ===\n");

    // ─────────────────────────────────────────────────────────────────────
    // ① 造语料：三份**同一画面**、三种格式的 BMP（Windows 画图就能产出这类文件）
    // ─────────────────────────────────────────────────────────────────────
    let px = corpus();
    let bmp24 = build_bmp24(W, H, &px);
    let bmp32 = build_bmp32_rgb(W, H, &px);
    let bmp_bits = build_bmp32_bitfields_v4(W, H, &px);
    let expected = expected_rgba(&px);
    // BITFIELDS 变体的 alpha 是棋盘（255/128）——期望也按同一函数独立推导。
    let mut expected_bits = Vec::with_capacity(expected.len());
    for y in 0..H {
        for x in 0..W {
            let p = px[(y * W + x) as usize];
            expected_bits.extend_from_slice(&[p[0], p[1], p[2], corpus_alpha_checker(x, y)]);
        }
    }
    assert!(
        expected_bits.chunks_exact(4).any(|p| p[3] != 255),
        "前置条件不成立：BITFIELDS 语料没有真半透明像素 ⇒「alpha 生效」断言是空的"
    );
    println!("① 语料：{W}×{H} 一幅渐变+对角线画面 × 3 种格式（24 位 BI_RGB / 32 位 BI_RGB / 32 位 BI_BITFIELDS V4）");

    // 前置断言：语料必须能抓住「通道序」与「行序」两类变异，否则回环判据是空的。
    assert!(
        px.iter().any(|p| p[0] != p[2]),
        "前置条件不成立：语料里没有 R≠B 的像素 ⇒ 通道序变异抓不到"
    );
    let top = &px[..W as usize];
    let bottom = &px[(px.len() - W as usize)..];
    assert_ne!(top, bottom, "前置条件不成立：首末行相同 ⇒ 行序变异抓不到");
    println!("   前置断言 ✅（R≠B 有区分度、首末行互不相同）");

    // ─────────────────────────────────────────────────────────────────────
    // ② 三种变体逐一解码：与独立推导的期望 RGBA 逐字节对照
    // ─────────────────────────────────────────────────────────────────────
    for (name, file, exp) in [
        ("24 位 BI_RGB（底行优先 + 行 padding）", &bmp24, &expected),
        ("32 位 BI_RGB（保留位按不透明，GDI 语义）", &bmp32, &expected),
        ("32 位 BI_BITFIELDS V4（掩码 + 文件 alpha）", &bmp_bits, &expected_bits),
    ] {
        let img = decode_bmp(file).unwrap_or_else(|e| panic!("{name} 解码失败：{e}"));
        assert_eq!(img.width, W);
        assert_eq!(img.height, H);
        assert_eq!(img.pixels, *exp, "{name}：解码结果必须与期望 RGBA 逐字节相同");
        println!("② {name} ✅ 解码 → 期望像素逐字节相同");
    }

    // ─────────────────────────────────────────────────────────────────────
    // ③ 编码回环：解码结果重编码的 PNG == 期望像素重编码的 PNG（逐字节），且可复现
    // ─────────────────────────────────────────────────────────────────────
    let img = decode_bmp(&bmp24).expect("解码");
    let from_decode = png::encode_rgba(W, H, &img.pixels).expect("重编码");
    let from_expected = png::encode_rgba(W, H, &expected).expect("重编码");
    assert_eq!(
        from_decode, from_expected,
        "回环判据：两份 PNG 必须逐字节相同"
    );
    let again = png::encode_rgba(W, H, &decode_bmp(&bmp24).expect("再解码").pixels).expect("再编码");
    assert_eq!(from_decode, again, "字节可复现：重跑一遍必须得到同样的 PNG 字节");
    println!("③ 编码回环 ✅ 解码 → png.rs 重编码，逐字节可复现");

    // ─────────────────────────────────────────────────────────────────────
    // ④ 落盘：解码结果写成 PNG（放大 8 倍便于肉眼核对朝向与颜色）
    // ─────────────────────────────────────────────────────────────────────
    std::fs::create_dir_all("render_out").ok();
    write_scaled_png("render_out/bmp_decode.png", W, H, &img.pixels, 8);
    let bits_img = decode_bmp(&bmp_bits).expect("解码");
    write_scaled_png("render_out/bmp_decode_bitfields.png", W, H, &bits_img.pixels, 8);
    println!("④ 产物 ✅ render_out/bmp_decode.png 与 bmp_decode_bitfields.png（放大 8 倍）");

    // ─────────────────────────────────────────────────────────────────────
    // ⑤ 负例：登记「不做」的形态必须显式报错（指名道姓，不静默给空图）
    // ─────────────────────────────────────────────────────────────────────
    let mut bad16 = bmp24.clone();
    bad16[28..30].copy_from_slice(&16u16.to_le_bytes()); // bpp → 16
    let e16 = decode_bmp(&bad16).expect_err("16 位必须报错");
    assert!(e16.to_string().contains("16"), "错误信息要点名 16 位：{e16}");

    let mut top_down = bmp24.clone();
    top_down[22..26].copy_from_slice(&(-3i32).to_le_bytes()); // height → 负（top-down）
    assert!(decode_bmp(&top_down).is_err(), "top-down 必须显式报错");

    let cut = bmp24.len() - 7;
    assert!(decode_bmp(&bmp24[..cut]).is_err(), "截断必须显式报错");
    println!("⑤ 负例 ✅ 16 位 / top-down / 截断都被显式拒绝（含可读的错误信息）");

    // ─────────────────────────────────────────────────────────────────────
    // ⑥ 直喂 HAL：create_texture + upload_texture（CPU 参考后端，无需 GPU）
    // ─────────────────────────────────────────────────────────────────────
    use deer_gpu::Backend;
    let backend = deer_gpu::null::CpuBackend::new();
    let mut cpu = backend.open(0).expect("CPU 适配器");
    let _tex = upload_bmp_to_texture(&img, cpu.as_mut()).expect("上传到 CPU 后端");
    println!("⑥ 直喂 HAL ✅ upload_bmp_to_texture → create_texture + upload_texture（CPU 参考后端）");

    // ─────────────────────────────────────────────────────────────────────
    // ⑦ 有 Vulkan 时：解码结果上真 GPU，回读逐字节保真（没有则跳过并说明）
    // ─────────────────────────────────────────────────────────────────────
    match deer_vk::GpuGeometryRenderer::new(0, deer_gpu::Extent { width: W, height: H }, deer_core::Color::rgb(16, 16, 16)) {
        Ok(r) => {
            println!("\n设备：{}", r.device().adapter().name);
            let tex = r
                .device()
                .create_texture_rgba8(W, H, &img.pixels)
                .expect("创建 RGBA8 纹理");
            let back = r.device().read_texture_bytes(&tex).expect("回读");
            assert_eq!(back.len(), img.pixels.len(), "回读长度必须 = w×h×4");
            assert_eq!(back, img.pixels, "解码 → 上传 → 回读必须逐字节保真");
            println!("⑦ GPU 回读 ✅ 解码结果上 Vulkan 纹理，回读逐字节相同");
            r.device().wait_idle().expect("空闲等待");
        }
        Err(e) => {
            println!("\n⑦ 本机没有可用的 Vulkan GPU：{e}");
            println!("   ⇒ 该步跳过（解码/回环/HAL 上传已在上面验证；CPU 出图看 render_out/）");
        }
    }

    // ─────────────────────────────────────────────────────────────────────
    // ⑧ 边界（随里程碑推进要跟着改，别留旧说法）
    // ─────────────────────────────────────────────────────────────────────
    println!("\n=== 当前边界 ===");
    println!("  ✅ 能：24/32 位、底行优先 BMP → RGBA8（顶行在前）；32 位 BI_RGB 按不透明处理");
    println!("  ✅ 能：BI_BITFIELDS（V1 紧跟掩码 / V2–V5 头内嵌掩码，V3 起带 alpha）");
    println!("  ❌ 不能：**PNG 解码未做**（自有 png.rs 只是**编码**器；自写 inflate 单独立项）");
    println!("  ❌ 不能：**16 位 BMP 未做**（还有：调色板、RLE/内嵌 JPEG·PNG 压缩、top-down、OS/2 core 头）");
    println!("  ❌ 不能：色彩管理 / gamma（字节直通，与全仓 `*_UNORM` 口径一致）");
}

/// 语料画面：水平红移 + 垂直绿移的渐变、蓝底稳定在高位，叠一条**左上→右下**的白对角线
/// （朝向肉眼可查：PNG 左上角必须是白的）。R≠B、逐行不同 ⇒ 变异可观测。
fn corpus() -> Vec<[u8; 3]> {
    let mut px = Vec::new();
    for y in 0..H {
        for x in 0..W {
            let mut p = [
                (30 + 9 * x) as u8,          // R：30…201
                (20 + 14 * y) as u8,         // G：20…230
                (220 - 3 * x - 2 * y) as u8, // B：220…130
            ];
            if x == y * W / H {
                p = [255, 255, 255];
            }
            px.push(p);
        }
    }
    px
}

fn expected_rgba(px: &[[u8; 3]]) -> Vec<u8> {
    let mut out = Vec::with_capacity(px.len() * 4);
    for p in px {
        out.extend_from_slice(&[p[0], p[1], p[2], 255]);
    }
    out
}

/// 把 RGBA8 画面放大 `scale` 倍写成 PNG（自检：非全一种颜色）。
fn write_scaled_png(path: &str, w: u32, h: u32, pixels: &[u8], scale: usize) {
    let (bw, bh) = (w as usize * scale, h as usize * scale);
    let mut big = vec![0u8; bw * bh * 4];
    for y in 0..bh {
        for x in 0..bw {
            let si = ((y / scale) * w as usize + (x / scale)) * 4;
            let di = (y * bw + x) * 4;
            big[di..di + 4].copy_from_slice(&pixels[si..si + 4]);
        }
    }
    let colors: std::collections::BTreeSet<[u8; 4]> = big.chunks_exact(4).map(|p| [p[0], p[1], p[2], p[3]]).collect();
    assert!(colors.len() > 1, "{path} 只有 {} 种颜色 —— 画面什么都没画？", colors.len());
    let png = deer_gpu::png::encode_rgba(bw as u32, bh as u32, &big).expect("编码 PNG");
    std::fs::write(path, png).expect("写 PNG");
}

// ── BMP 手工构造（与 tests/image_bmp.rs 同一套格式事实：底行优先、行 4 字节对齐）──

fn le32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}

struct Spec<'a> {
    bpp: u16,
    compression: u32,
    header_size: u32,
    inline_masks: &'a [u32],
    /// 存储序（底行在前）、不含 padding 的像素字节。
    storage: &'a [u8],
    bpp_bytes: usize,
}

fn build_bmp(w: u32, h: u32, spec: &Spec<'_>) -> Vec<u8> {
    let row = (w as usize * spec.bpp_bytes + 3) & !3;
    let pixel_offset = 14 + spec.header_size as usize;
    let mut out = Vec::new();
    out.extend_from_slice(b"BM");
    le32(&mut out, (pixel_offset + row * h as usize) as u32);
    le32(&mut out, 0);
    le32(&mut out, pixel_offset as u32);
    le32(&mut out, spec.header_size);
    le32(&mut out, w);
    le32(&mut out, h); // 正 ⇒ 底行优先
    out.extend_from_slice(&1u16.to_le_bytes()); // planes
    out.extend_from_slice(&spec.bpp.to_le_bytes());
    le32(&mut out, spec.compression);
    le32(&mut out, (row * h as usize) as u32);
    le32(&mut out, 2835);
    le32(&mut out, 2835);
    le32(&mut out, 0);
    le32(&mut out, 0);
    for &m in spec.inline_masks {
        le32(&mut out, m);
    }
    if spec.header_size >= 108 {
        out.resize(pixel_offset, 0); // V4 头剩余部分（CSType/gamma，解码器不读）
    }
    for y in (0..h).rev() {
        let at = y as usize * w as usize * spec.bpp_bytes;
        out.extend_from_slice(&spec.storage[at..at + w as usize * spec.bpp_bytes]);
        out.extend(std::iter::repeat_n(0xAB, row - w as usize * spec.bpp_bytes));
    }
    out
}

/// 24 位 `BI_RGB`：图像空间像素 → B、G、R 存储 + padding。
fn build_bmp24(w: u32, h: u32, px: &[[u8; 3]]) -> Vec<u8> {
    let mut storage = Vec::new();
    for p in px {
        storage.extend_from_slice(&[p[2], p[1], p[0]]);
    }
    build_bmp(
        w,
        h,
        &Spec {
            bpp: 24,
            compression: 0,
            header_size: 40,
            inline_masks: &[],
            storage: &storage,
            bpp_bytes: 3,
        },
    )
}

/// 32 位 `BI_RGB`：B、G、R + 保留位（现实常见值 0 ⇒ 解码按不透明）。
fn build_bmp32_rgb(w: u32, h: u32, px: &[[u8; 3]]) -> Vec<u8> {
    let mut storage = Vec::new();
    for p in px {
        storage.extend_from_slice(&[p[2], p[1], p[0], 0]);
    }
    build_bmp(
        w,
        h,
        &Spec {
            bpp: 32,
            compression: 0,
            header_size: 40,
            inline_masks: &[],
            storage: &storage,
            bpp_bytes: 4,
        },
    )
}

/// BITFIELDS 变体的 alpha 棋盘（构造器与期望共用同一函数）。
fn corpus_alpha_checker(x: u32, y: u32) -> u8 {
    if (x + y) % 2 == 0 { 255 } else { 128 }
}

/// 32 位 `BI_BITFIELDS` + V4 头：内嵌 BGRA 掩码，alpha 做棋盘（255/128）。
fn build_bmp32_bitfields_v4(w: u32, h: u32, px: &[[u8; 3]]) -> Vec<u8> {
    let mut storage = Vec::new();
    for (i, p) in px.iter().enumerate() {
        let x = i as u32 % W;
        let y = i as u32 / W;
        let a = corpus_alpha_checker(x, y);
        le32(
            &mut storage,
            ((a as u32) << 24) | ((p[0] as u32) << 16) | ((p[1] as u32) << 8) | p[2] as u32,
        );
    }
    build_bmp(
        w,
        h,
        &Spec {
            bpp: 32,
            compression: 3,
            header_size: 108,
            inline_masks: &[0x00FF_0000, 0x0000_FF00, 0x0000_00FF, 0xFF00_0000],
            storage: &storage,
            bpp_bytes: 4,
        },
    )
}
