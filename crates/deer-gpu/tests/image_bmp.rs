//! AF-1 BMP 解码测试：**格式变体 × 编码回环判据**。
//!
//! 判据（任务书原文口径）：解码 → 用自有 `png.rs` 重编码 → **字节可复现**。
//! 做法：每种格式变体各建一份**手工 BMP 语料**（底行优先写入，padding 填可辨识的垃圾），
//! 与**独立推导的期望 RGBA**（图像空间、顶行在前）做逐字节对照，再各自经
//! `deer_gpu::png::encode_rgba` 重编码 ⇒ 两份 PNG 必须逐字节相同。
//!
//! 前置断言纪律：语料必须让「BGR→RGB 通道序改错」与「底行优先漏翻转」两类变异
//! **可观测**（存在 R≠B 的像素、首末行三通道各不相同）——断言先于回环判据。

use deer_gpu::image::{decode_bmp, upload_bmp_to_texture, BmpError, BmpImage};
use deer_gpu::png;

// ── 语料 ────────────────────────────────────────────────────────────────────

/// 图像空间（顶行在前）的 RGB 语料：R/G/B 三通道互不相同、逐像素渐变。
///
/// 取值刻意让 **R 与 B 拉开距离**（16…42 对 200…182）——通道序互换必被抓到；
/// 每行基色不同 —— 行序漏翻必被抓到。
fn corpus_rgb(w: u32, h: u32) -> Vec<[u8; 3]> {
    (0..h)
        .flat_map(|y| {
            (0..w).map(move |x| {
                [
                    (16 + 8 * x + y) as u8,     // R：16…42
                    (64 + 16 * x + 2 * y) as u8, // G：64…116
                    (200 - 4 * x - 3 * y) as u8, // B：200…182
                ]
            })
        })
        .collect()
}

/// BITFIELDS 变体专用的逐像素 alpha（127…255，文件里真有半透明）。
fn corpus_alpha(w: u32, h: u32) -> Vec<u8> {
    (0..h)
        .flat_map(|y| (0..w).map(move |x| (255 - 32 * x - 16 * y) as u8))
        .collect()
}

/// **前置断言**：语料必须能抓住「通道序」「行序」两类变异，否则回环判据是空的。
fn assert_corpus_catches_mutations(w: u32, h: u32, px: &[[u8; 3]]) {
    assert_eq!(px.len(), (w * h) as usize, "语料长度必须与 w×h 一致");
    assert!(
        px.iter().any(|p| p[0] != p[2]),
        "前置条件不成立：语料里没有 R≠B 的像素 ⇒ BGR→RGB 通道序变异抓不到"
    );
    let row = |y: u32| &px[(y * w) as usize..((y + 1) * w) as usize];
    let (first, last) = (row(0), row(h - 1));
    for c in 0..3 {
        assert!(
            first.iter().any(|p| p[c] != last[0][c]) || first[0][c] != last[0][c],
            "前置条件不成立：首行与末行的通道 {c} 取值相同 ⇒ 行序（底行优先）变异抓不到"
        );
        assert_ne!(
            first[0][c], last[0][c],
            "前置条件不成立：首末行左上像素通道 {c} 相同 ⇒ 行序变异在 (0,0) 处抓不到"
        );
    }
}

fn expected_rgba(w: u32, h: u32, px: &[[u8; 3]]) -> Vec<u8> {
    let mut out = Vec::with_capacity((w * h * 4) as usize);
    for p in px {
        out.extend_from_slice(&[p[0], p[1], p[2], 255]);
    }
    out
}

// ── BMP 文件手工构造（存储序 = 底行优先；padding 填 0xAB 可辨识垃圾）──────────

fn le16(out: &mut Vec<u8>, v: u16) {
    out.extend_from_slice(&v.to_le_bytes());
}
fn le32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}

struct BmpSpec<'a> {
    w: u32,
    h: u32,
    bpp: u16,
    compression: u32,
    header_size: u32,
    /// V1 BITFIELDS：紧跟 40 字节头的 3 个掩码 DWORD（R,G,B）。
    trailing_masks: &'a [u32],
    /// V2+ 头：内嵌掩码（R,G,B[,A]），写进 DIB 头偏移 40 起。
    inline_masks: &'a [u32],
    /// 像素字节，**已按存储序**（底行在前）排好、不含 padding。
    storage: &'a [u8],
    bytes_per_pixel: usize,
}

fn build_bmp(spec: &BmpSpec<'_>) -> Vec<u8> {
    let row = ((spec.w as usize * spec.bytes_per_pixel) + 3) & !3;
    let masks_len = spec.trailing_masks.len() * 4;
    let pixel_offset = 14 + spec.header_size as usize + masks_len;
    let mut out = Vec::new();
    out.extend_from_slice(b"BM");
    le32(&mut out, (pixel_offset + row * spec.h as usize) as u32);
    le32(&mut out, 0); // 保留
    le32(&mut out, pixel_offset as u32);
    // DIB 头：头 40 字节与 BITMAPINFOHEADER 相同，header_size 声明总长（V2+ 更长）。
    le32(&mut out, spec.header_size);
    le32(&mut out, spec.w);
    le32(&mut out, spec.h);
    le16(&mut out, 1); // planes
    le16(&mut out, spec.bpp);
    le32(&mut out, spec.compression);
    le32(&mut out, (row * spec.h as usize) as u32); // 像素区大小
    le32(&mut out, 2835); // 水平分辨率（真实值，解码器不读）
    le32(&mut out, 2835); // 垂直分辨率
    le32(&mut out, 0); // 调色板色数
    le32(&mut out, 0); // 重要色数
    debug_assert_eq!(out.len(), 54, "DIB 头本体必须恰好 40 字节");
    for &m in spec.inline_masks {
        le32(&mut out, m);
    }
    if spec.header_size >= 108 {
        // V4/V5 头的剩余部分：CSType + 端点 + gamma，全 0（解码器不读）。
        out.resize(14 + spec.header_size as usize, 0);
    }
    for &m in spec.trailing_masks {
        le32(&mut out, m);
    }
    assert_eq!(out.len(), pixel_offset, "构造器偏移自洽");
    // 存储序由调用方排好；这里只补每行的 4 字节对齐 padding（0xAB ⇒ 若被当像素读出来必红）。
    let pad = row - spec.w as usize * spec.bytes_per_pixel;
    for stored_row in 0..spec.h as usize {
        let at = stored_row * spec.w as usize * spec.bytes_per_pixel;
        out.extend_from_slice(&spec.storage[at..at + spec.w as usize * spec.bytes_per_pixel]);
        out.extend(std::iter::repeat_n(0xAB, pad));
    }
    out
}

/// 24 位 `BI_RGB`（V1 头）。`px` 是**图像空间**（顶行在前）。
fn bmp24(w: u32, h: u32, px: &[[u8; 3]]) -> Vec<u8> {
    let mut storage = Vec::new();
    for y in (0..h).rev() {
        for x in 0..w {
            let p = px[(y * w + x) as usize];
            storage.extend_from_slice(&[p[2], p[1], p[0]]); // B G R
        }
    }
    build_bmp(&BmpSpec {
        w,
        h,
        bpp: 24,
        compression: 0,
        header_size: 40,
        trailing_masks: &[],
        inline_masks: &[],
        storage: &storage,
        bytes_per_pixel: 3,
    })
}

/// 32 位 `BI_RGB`（BGX 存储；`alpha_byte` 是文件里的保留位实际值）。
fn bmp32_rgb(w: u32, h: u32, px: &[[u8; 3]], alpha_byte: u8) -> Vec<u8> {
    let mut storage = Vec::new();
    for y in (0..h).rev() {
        for x in 0..w {
            let p = px[(y * w + x) as usize];
            storage.extend_from_slice(&[p[2], p[1], p[0], alpha_byte]); // B G R X
        }
    }
    build_bmp(&BmpSpec {
        w,
        h,
        bpp: 32,
        compression: 0,
        header_size: 40,
        trailing_masks: &[],
        inline_masks: &[],
        storage: &storage,
        bytes_per_pixel: 4,
    })
}

/// 32 位 `BI_BITFIELDS` + **V4 头**（108 字节，内嵌 BGRA 掩码，alpha 生效）。
fn bmp32_bitfields_v4(w: u32, h: u32, px: &[[u8; 3]], alpha: &[u8]) -> Vec<u8> {
    let mut storage = Vec::new();
    for y in (0..h).rev() {
        for x in 0..w {
            let p = px[(y * w + x) as usize];
            let a = alpha[(y * w + x) as usize];
            // 内存序 B,G,R,A ⇒ 小端 u32 = (A<<24)|(R<<16)|(G<<8)|B
            le32(&mut storage, ((a as u32) << 24) | ((p[0] as u32) << 16) | ((p[1] as u32) << 8) | p[2] as u32);
        }
    }
    build_bmp(&BmpSpec {
        w,
        h,
        bpp: 32,
        compression: 3, // BI_BITFIELDS
        header_size: 108,
        trailing_masks: &[],
        inline_masks: &[0x00FF_0000, 0x0000_FF00, 0x0000_00FF, 0xFF00_0000],
        storage: &storage,
        bytes_per_pixel: 4,
    })
}

/// 32 位 `BI_BITFIELDS` + **V1 头**（40 字节 + 紧跟 3 个掩码 DWORD，无 alpha ⇒ 不透明）。
fn bmp32_bitfields_v1(w: u32, h: u32, px: &[[u8; 3]]) -> Vec<u8> {
    let mut storage = Vec::new();
    for y in (0..h).rev() {
        for x in 0..w {
            let p = px[(y * w + x) as usize];
            le32(&mut storage, ((p[0] as u32) << 16) | ((p[1] as u32) << 8) | p[2] as u32);
        }
    }
    build_bmp(&BmpSpec {
        w,
        h,
        bpp: 32,
        compression: 3,
        header_size: 40,
        trailing_masks: &[0x00FF_0000, 0x0000_FF00, 0x0000_00FF],
        inline_masks: &[],
        storage: &storage,
        bytes_per_pixel: 4,
    })
}

/// 在一份合法 BMP 上按偏移打补丁（构造「字段非法 / 不支持形态」的负例语料）。
fn patch(file: &mut [u8], at: usize, bytes: &[u8]) {
    file[at..at + bytes.len()].copy_from_slice(bytes);
}

// ── 回环判据 ────────────────────────────────────────────────────────────────

/// 回环判据本体：解码结果 == 期望像素（逐字节），两份各自重编码的 PNG ==（逐字节）。
fn roundtrip(img: &BmpImage, w: u32, h: u32, expected: &[u8]) {
    assert_eq!(img.width, w, "宽度必须与语料一致");
    assert_eq!(img.height, h, "高度必须与语料一致");
    assert_eq!(img.pixels.len(), (w * h * 4) as usize, "像素长度必须 = w×h×4");
    assert_eq!(img.pixels, expected, "解码结果必须与期望 RGBA 逐字节相同");
    let from_decode = png::encode_rgba(w, h, &img.pixels).expect("解码结果重编码");
    let from_expected = png::encode_rgba(w, h, expected).expect("期望像素重编码");
    assert_eq!(
        from_decode, from_expected,
        "编码回环：解码结果重编码的 PNG 必须与期望像素的 PNG 逐字节相同"
    );
}

// ── 变体测试（每个格式变体各一条）────────────────────────────────────────────

#[test]
fn bmp24_bi_rgb_bottom_up_roundtrip_is_byte_reproducible() {
    let (w, h) = (4u32, 3u32);
    let px = corpus_rgb(w, h);
    assert_corpus_catches_mutations(w, h, &px);
    let file = bmp24(w, h, &px);
    let img = decode_bmp(&file).expect("24 位 BI_RGB 应能解码");
    roundtrip(&img, w, h, &expected_rgba(w, h, &px));
}

#[test]
fn bmp24_row_padding_is_skipped() {
    // 宽 3 ⇒ 每行 9 字节 ⇒ 对齐到 12（构造器填 0xAB）。前置断言：padding 真实存在。
    let (w, h) = (3u32, 2u32);
    let px = corpus_rgb(w, h);
    let file = bmp24(w, h, &px);
    let row = (w as usize * 3 + 3) & !3;
    assert_eq!(row, 12, "前置条件不成立：宽 3 的行不应恰好对齐（padding 未被构造出来）");
    assert_eq!(file.len(), 14 + 40 + row * h as usize, "语料文件长度必须含 padding");
    let img = decode_bmp(&file).expect("带 padding 的 24 位 BMP 应能解码");
    roundtrip(&img, w, h, &expected_rgba(w, h, &px));
}

#[test]
fn bmp32_bi_rgb_is_read_as_opaque() {
    // 前置断言：文件里的保留位是 **0**（现实中常见的「alpha 全 0」文件）。
    // 期望：仍解出**不透明**（GDI 语义：BI_RGB 32 位的第 4 字节不是 alpha）——
    // 若有人把它当真 alpha 读，这张图会整张透明，此测必红。
    let (w, h) = (4u32, 3u32);
    let px = corpus_rgb(w, h);
    let file = bmp32_rgb(w, h, &px, 0);
    let px_start = 14 + 40;
    assert!(
        file[px_start..].chunks_exact(4).all(|p| p[3] == 0),
        "前置条件不成立：语料的保留位不是全 0"
    );
    let img = decode_bmp(&file).expect("32 位 BI_RGB 应能解码");
    roundtrip(&img, w, h, &expected_rgba(w, h, &px));
}

#[test]
fn bmp32_bitfields_v4_honors_file_alpha_and_masks() {
    let (w, h) = (4u32, 3u32);
    let px = corpus_rgb(w, h);
    let alpha = corpus_alpha(w, h);
    let file = bmp32_bitfields_v4(w, h, &px, &alpha);
    let img = decode_bmp(&file).expect("32 位 BI_BITFIELDS（V4 头）应能解码");
    // 期望：RGB 来自掩码、alpha 逐像素取自文件。
    let mut expected = Vec::with_capacity((w * h * 4) as usize);
    for (p, a) in px.iter().zip(&alpha) {
        expected.extend_from_slice(&[p[0], p[1], p[2], *a]);
    }
    roundtrip(&img, w, h, &expected);
    assert!(
        img.pixels.chunks_exact(4).any(|p| p[3] != 255),
        "前置条件不成立：语料应有真半透明像素，否则「alpha 生效」断言是空的"
    );
}

#[test]
fn bmp32_bitfields_v1_masks_follow_header_and_stay_opaque() {
    let (w, h) = (4u32, 3u32);
    let px = corpus_rgb(w, h);
    let file = bmp32_bitfields_v1(w, h, &px);
    let img = decode_bmp(&file).expect("32 位 BI_BITFIELDS（V1 头 + 3 掩码）应能解码");
    roundtrip(&img, w, h, &expected_rgba(w, h, &px));
}

// ── 字节可复现（同一份字节，两次解码 / 两次编码逐字节相同）───────────────────

#[test]
fn decoding_is_deterministic_down_to_png_bytes() {
    let (w, h) = (4u32, 3u32);
    let px = corpus_rgb(w, h);
    let file = bmp24(w, h, &px);
    let a = decode_bmp(&file).expect("第一次解码");
    let b = decode_bmp(&file).expect("第二次解码");
    assert_eq!(a, b, "同一份字节两次解码必须逐字节相同");
    let png1 = png::encode_rgba(w, h, &a.pixels).expect("第一次编码");
    let png2 = png::encode_rgba(w, h, &a.pixels).expect("第二次编码");
    assert_eq!(png1, png2, "同一份像素两次编码的 PNG 必须逐字节相同（字节可复现）");
}

// ── 负例：错误必须指名道姓，不静默给空图 ─────────────────────────────────────

#[test]
fn bad_magic_is_rejected() {
    let mut file = bmp24(4, 3, &corpus_rgb(4, 3));
    patch(&mut file, 0, b"XZ");
    let err = decode_bmp(&file).expect_err("非 BM 魔数必须报错");
    assert_eq!(err, BmpError::BadMagic);
    assert!(err.to_string().contains("BM"), "错误信息要点名魔数：{err}");
}

#[test]
fn truncated_pixel_data_is_rejected() {
    let file = bmp24(4, 3, &corpus_rgb(4, 3));
    let cut = file.len() - 5;
    let err = decode_bmp(&file[..cut]).expect_err("像素区截断必须报错");
    match &err {
        BmpError::Truncated { need, have, .. } => {
            assert_eq!(*have, cut, "have 必须是实际长度");
            assert!(*need > *have, "need 必须大于实际长度");
        }
        other => panic!("应是 Truncated，实际 {other:?}"),
    }
}

#[test]
fn bpp16_is_reported_as_unsupported_not_crash() {
    // 登记的边界：16 位 BMP 未做 ⇒ 必须显式 Unsupported（信息里点名「16」），不是 panic / 空图。
    let mut file = bmp24(4, 3, &corpus_rgb(4, 3));
    patch(&mut file, 28, &16u16.to_le_bytes());
    let err = decode_bmp(&file).expect_err("16 位 BMP 必须显式报错");
    assert!(matches!(err, BmpError::Unsupported(_)), "应是 Unsupported，实际 {err:?}");
    assert!(err.to_string().contains("16"), "错误信息要点名 16 位：{err}");
}

#[test]
fn top_down_negative_height_is_rejected() {
    let mut file = bmp24(4, 3, &corpus_rgb(4, 3));
    patch(&mut file, 22, &(-3i32).to_le_bytes());
    let err = decode_bmp(&file).expect_err("top-down（负高）必须显式报错");
    assert!(matches!(err, BmpError::Unsupported(_)));
    assert!(err.to_string().contains("top-down"), "错误信息要点名 top-down：{err}");
}

#[test]
fn rle_compression_is_rejected() {
    let mut file = bmp24(4, 3, &corpus_rgb(4, 3));
    patch(&mut file, 30, &1u32.to_le_bytes()); // BI_RLE8
    let err = decode_bmp(&file).expect_err("RLE 压缩必须显式报错");
    assert!(matches!(err, BmpError::Unsupported(_)));
    assert!(err.to_string().contains("RLE"), "错误信息要点名 RLE：{err}");
}

#[test]
fn os2_core_header_is_rejected() {
    let mut file = bmp24(4, 3, &corpus_rgb(4, 3));
    patch(&mut file, 14, &12u32.to_le_bytes()); // BITMAPCOREHEADER
    let err = decode_bmp(&file).expect_err("OS/2 core 头必须显式报错");
    assert!(matches!(err, BmpError::Unsupported(_)));
}

#[test]
fn zero_dimensions_are_rejected() {
    let mut file = bmp24(4, 3, &corpus_rgb(4, 3));
    patch(&mut file, 18, &0u32.to_le_bytes());
    assert!(matches!(
        decode_bmp(&file),
        Err(BmpError::Invalid(_))
    ));
}

#[test]
fn pixel_offset_inside_header_is_rejected() {
    let mut file = bmp24(4, 3, &corpus_rgb(4, 3));
    patch(&mut file, 10, &20u32.to_le_bytes()); // 像素偏移落进头区
    let err = decode_bmp(&file).expect_err("像素偏移落在头区必须报错");
    assert!(matches!(err, BmpError::Invalid(_)), "实际 {err:?}");
}

// ── 直喂 HAL：create_texture + upload_texture ───────────────────────────────

#[test]
fn decode_feeds_create_and_upload_texture() {
    use deer_gpu::Backend;

    let img = decode_bmp(&bmp24(4, 3, &corpus_rgb(4, 3))).expect("解码");
    let backend = deer_gpu::null::CpuBackend::new();
    let mut device = backend.open(0).expect("打开 CPU 适配器");

    let id0 = upload_bmp_to_texture(&img, device.as_mut()).expect("上传第一张");
    let id1 = upload_bmp_to_texture(&img, device.as_mut()).expect("上传第二张");
    assert_ne!(id0, id1, "两次 create_texture 必须得到不同的纹理 id");

    // 手工构造的 BmpImage 尺寸不符 ⇒ 必须报错，不能把错误数据交给后端。
    let broken = BmpImage {
        width: 2,
        height: 2,
        pixels: vec![0; 4],
    };
    assert!(upload_bmp_to_texture(&broken, device.as_mut()).is_err());
}
