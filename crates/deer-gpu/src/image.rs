//! 零依赖 BMP 解码器（AF-1）：**文件字节 → RGBA8 像素 → 直喂纹理**。
//!
//! ## 为什么是 BMP、为什么自己写
//!
//! 本项目不引第三方依赖（`ROADMAP.md` 的依赖例外登记里没有图像库），而「界面里贴一张图」
//! （M6 `Icon`）需要至少一种**能从真实工具产出的**图像格式：BMP 格式无压缩、头结构简单，
//! Windows 画图就能产出，是零依赖解码的自然起点。PNG **解码**需要 inflate，单独立项按需决策
//! （`ROADMAP.md` Q2）；PNG **编码**已有 [`crate::png`]。
//!
//! ## 支持与不支持的边界（都有测试钉住）
//!
//! **支持**：
//! - 24 位 `BI_RGB`（行按 4 字节对齐的 padding 会被跳过）；
//! - 32 位 `BI_RGB` —— **按不透明处理**（GDI 语义：这种文件的 alpha 字节是保留位，
//!   常见取值 0；当真当 alpha 会让整张图变透明）；
//! - 32 位 `BI_BITFIELDS`（压缩 3）：V1 头读**紧跟其后的 3 个掩码 DWORD**（无 alpha ⇒ 不透明），
//!   V2/V3/V4/V5 头读**头内嵌掩码**（V3 起有 alpha）；
//! - **底行优先**（height 为正，BMP 的存储约定）：解码输出翻成**顶行在前**，
//!   与 [`crate::png`] / `create_texture` 的行序一致。
//!
//! **不支持**（`Unsupported` 报错指名道姓，不静默给空图）：16 位、调色板（bpp ≤ 8）、
//! RLE / 内嵌 JPEG·PNG 压缩、top-down（height 为负）、OS/2 `BITMAPCOREHEADER`（头 < 40）。
//!
//! ## 判据：编码回环
//!
//! 解码正确性的判据不是「看起来对」，而是**逐字节可复现**：
//! 解码 → 用自有 [`crate::png::encode_rgba`] 重编码 ⇒ 与「期望像素」重编码的 PNG
//! **逐字节相同**（见 `crates/deer-gpu/tests/image_bmp.rs`）。语料刻意让 R≠B、
//! 每行颜色不同 —— 通道序（BGR→RGB）或行序（底行优先）改错的变异必红。

use deer_core::error::GpuError;
use deer_core::{GpuResult, TextureId};

use crate::{Device, TargetFormat, TextureDesc, TextureRegion};

/// 一次成功解码的产物：尺寸 + RGBA8 像素。
///
/// `pixels` 的排布与 [`crate::png::encode_rgba`]、`Device::upload_texture` 的期望**完全一致**：
/// 行优先、无 padding、每像素 4 字节（R、G、B、A）、**顶行在前**（`pixels[0..4]` 是左上角）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BmpImage {
    pub width: u32,
    pub height: u32,
    /// 长度恒为 `width * height * 4`（`upload_bmp_to_texture` 会复核）。
    pub pixels: Vec<u8>,
}

/// BMP 解码失败的原因。
///
/// 设计口径与 `deer-core` 的错误哲学一致：**报错要说清是什么、为什么、（能指出的）怎么办**，
/// 不静默降级成空图。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BmpError {
    /// 文件头两字节不是 `"BM"` —— 这不是 BMP 文件。
    BadMagic,
    /// 数据在读取 `what` 时被截断（需要 `need` 字节，实际只有 `have` 字节）。
    Truncated { what: String, need: usize, have: usize },
    /// 文件**本身合法**但落在本解码器登记「不做」的形态里（16 位、RLE、top-down……）。
    /// 每条消息都指名道姓 —— 「报错但不告诉你为什么」等于没报错。
    Unsupported(String),
    /// 字段取值荒谬（宽/高 ≤ 0、像素偏移落在头区内、尺寸溢出等）。
    Invalid(String),
}

impl std::fmt::Display for BmpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BmpError::BadMagic => write!(f, "不是 BMP 文件：文件头魔数不是 \"BM\""),
            BmpError::Truncated { what, need, have } => write!(
                f,
                "BMP 数据被截断：读{what}需要 {need} 字节，实际只有 {have} 字节"
            ),
            BmpError::Unsupported(what) => write!(f, "不支持的 BMP 形态：{what}"),
            BmpError::Invalid(what) => write!(f, "BMP 字段非法：{what}"),
        }
    }
}

impl std::error::Error for BmpError {}

/// 解码一份完整的 BMP 文件字节（`BITMAPFILEHEADER` + `DIB 头` + 像素）。
///
/// 成功时返回 [`BmpImage`]：RGBA8、顶行在前，可直接交给
/// [`upload_bmp_to_texture`] 或 [`crate::png::encode_rgba`]。
pub fn decode_bmp(data: &[u8]) -> Result<BmpImage, BmpError> {
    ensure_len(data, 14, "文件头（BITMAPFILEHEADER 14 字节）")?;
    if &data[0..2] != b"BM" {
        return Err(BmpError::BadMagic);
    }
    let pixel_offset = u32_le(data, 10) as usize;

    ensure_len(data, 18, "DIB 头大小字段")?;
    let header_size = u32_le(data, 14) as usize;
    if header_size < 40 {
        return Err(BmpError::Unsupported(format!(
            "DIB 头只有 {header_size} 字节（OS/2 BITMAPCOREHEADER 族），\
             只支持 ≥40 的 BITMAPINFOHEADER 及其 V2–V5 扩展"
        )));
    }
    ensure_len(data, 54, "DIB 头（BITMAPINFOHEADER 40 字节）")?;
    let width = i32_le(data, 18);
    let height_raw = i32_le(data, 22);
    let bpp = u16_le(data, 28) as u32;
    let compression = u32_le(data, 30);

    if width <= 0 {
        return Err(BmpError::Invalid(format!("宽 {width} 必须为正")));
    }
    let height = match height_raw {
        0 => return Err(BmpError::Invalid("高 0 必须为正".to_string())),
        // BMP 约定：height 为负 = top-down（顶行先存）。本解码器只做底行优先，
        // 这条是**登记过的边界**（见模块注释与指南「做不到什么」），不是 TODO。
        h if h < 0 => {
            return Err(BmpError::Unsupported(
                "top-down 行序（height 为负，顶行先存）未做：只支持底行优先（height 为正）"
                    .to_string(),
            ))
        }
        h => h as u32,
    };
    let w = width as u32;

    // 像素的存储布局与「头 + 掩码区」的结束位置（用于像素偏移的合法性检查）。
    let mut fields_end = 54usize; // BITMAPINFOHEADER 本体（文件偏移 14..54）
    let layout = match (bpp, compression) {
        (24, 0) => Layout::Bgr24,
        (32, 0) => Layout::Bgr32Opaque,
        (32, 3) => {
            let (r, g, b, a) = read_bitfield_masks(data, header_size, &mut fields_end)?;
            Layout::Bitfields { r, g, b, a }
        }
        (16, 0 | 3) => {
            return Err(BmpError::Unsupported(
                "16 位 BMP 未做（需要 5-5-5 / 5-6-5 位域展开）——见指南「做不到什么」".to_string(),
            ))
        }
        (1..=8, _) => {
            return Err(BmpError::Unsupported(format!(
                "调色板格式未做（bpp = {bpp}，需要读调色板索引）"
            )))
        }
        (_, 1 | 2) => {
            return Err(BmpError::Unsupported(
                "RLE 压缩（BI_RLE8 / BI_RLE4）未做".to_string(),
            ))
        }
        (_, 4 | 5) => {
            return Err(BmpError::Unsupported(
                "内嵌 JPEG / PNG 压缩（BI_JPEG / BI_PNG）未做".to_string(),
            ))
        }
        (bpp, compression) => {
            return Err(BmpError::Unsupported(format!(
                "位深 {bpp} + 压缩 {compression} 的组合未做（支持：24/32 位，BI_RGB 或 BI_BITFIELDS）"
            )))
        }
    };

    if pixel_offset < fields_end {
        return Err(BmpError::Invalid(format!(
            "像素数据偏移 {pixel_offset} 落在头/掩码区之内（该区到 {fields_end} 为止）"
        )));
    }

    // 行字节数：BMP 每行对齐到 4 字节（24 位且宽不是 4 的倍数时 padding 才真实存在）。
    let raw_row = match layout {
        Layout::Bgr24 => (w as usize).checked_mul(3),
        _ => (w as usize).checked_mul(4),
    }
    .ok_or_else(|| BmpError::Invalid("图像宽度过大（行字节数溢出）".to_string()))?;
    let row_bytes = raw_row
        .checked_add(3)
        .map(|v| v & !3)
        .ok_or_else(|| BmpError::Invalid("图像宽度过大（行对齐溢出）".to_string()))?;
    let total = row_bytes
        .checked_mul(height as usize)
        .ok_or_else(|| BmpError::Invalid("图像尺寸过大（像素区总长溢出）".to_string()))?;
    ensure_len(data, pixel_offset + total, "全部像素行")?;

    let mut out = vec![0u8; (w as usize) * (height as usize) * 4];
    let bpp_bytes = match layout {
        Layout::Bgr24 => 3usize,
        _ => 4usize,
    };
    for stored_row in 0..height as usize {
        // **底行优先**：存储的第 0 行是图像**最下面**一行 ⇒ 输出坐标要翻。
        // 语料的每行颜色互不相同，这条翻转被测试直接钉住（漏翻必红）。
        let out_y = height as usize - 1 - stored_row;
        let row_at = pixel_offset + stored_row * row_bytes;
        for x in 0..w as usize {
            let (r, g, b, a) = match layout {
                Layout::Bitfields { r, g, b, a } => {
                    let px = u32_le(data, row_at + x * 4);
                    (extract8(px, r), extract8(px, g), extract8(px, b), extract8(px, a))
                }
                _ => {
                    // 存储序是 B、G、R（,X）⇒ 读成 R、G、B；这就是「BGR→RGB 改错必红」的那一处。
                    let i = row_at + x * bpp_bytes;
                    (data[i + 2], data[i + 1], data[i], 255u8)
                }
            };
            let o = (out_y * (w as usize) + x) * 4;
            out[o] = r;
            out[o + 1] = g;
            out[o + 2] = b;
            out[o + 3] = a;
        }
    }

    Ok(BmpImage {
        width: w,
        height,
        pixels: out,
    })
}

/// 像素的存储布局（`decode_bmp` 内部的分支依据）。
#[derive(Clone, Copy)]
enum Layout {
    /// 24 位 `BI_RGB`：每像素 3 字节 B、G、R。
    Bgr24,
    /// 32 位 `BI_RGB`：每像素 4 字节 B、G、R、X —— X 是保留位，**按不透明处理**（GDI 语义）。
    Bgr32Opaque,
    /// 32 位 `BI_BITFIELDS`：掩码来自文件；掩码为 0 的 alpha 通道按不透明处理。
    Bitfields { r: u32, g: u32, b: u32, a: u32 },
}

/// 读 `BI_BITFIELDS` 的通道掩码，返回 `(R, G, B, A)`；`fields_end` 带回头/掩码区的结束偏移。
///
/// - **V2（52）/ V3（56）/ V4（108）/ V5（124）**：掩码内嵌在 DIB 头固定偏移
///   （40/44/48/52，即文件偏移 54/58/62/66）；V3 起才有 alpha；
/// - **V1（40）**：掩码是**紧跟 DIB 头的 3 个 DWORD**（红/绿/蓝），没有 alpha ⇒ 不透明。
fn read_bitfield_masks(
    data: &[u8],
    header_size: usize,
    fields_end: &mut usize,
) -> Result<(u32, u32, u32, u32), BmpError> {
    if header_size >= 52 {
        let alpha_end = if header_size >= 56 { 56 } else { 52 };
        ensure_len(data, 14 + alpha_end, "DIB 头内嵌的位域掩码")?;
        *fields_end = 14 + alpha_end;
        let a = if header_size >= 56 { u32_le(data, 66) } else { 0 };
        Ok((u32_le(data, 54), u32_le(data, 58), u32_le(data, 62), a))
    } else {
        ensure_len(data, 14 + 40 + 12, "紧跟 DIB 头的 3 个位域掩码 DWORD")?;
        *fields_end = 14 + 40 + 12;
        Ok((u32_le(data, 54), u32_le(data, 58), u32_le(data, 62), 0))
    }
}

/// 按位域掩码从像素里取通道并折算成 8 位。
///
/// 位宽 > 8 取**最高 8 位**；位宽 < 8 做 `round(v * 255 / max)`；
/// 掩码为 0 表示该通道不存在 —— 只对 alpha 合法：缺 alpha ⇒ **不透明**（255）。
fn extract8(px: u32, mask: u32) -> u8 {
    if mask == 0 {
        return 255;
    }
    let shift = mask.trailing_zeros();
    let bits = mask.count_ones();
    let ones = if bits >= 32 { u32::MAX } else { (1u32 << bits) - 1 };
    let v = ((px & mask) >> shift) & ones;
    if bits >= 8 {
        (v >> (bits - 8)) as u8
    } else {
        (((v * 255) + ones / 2) / ones) as u8
    }
}

/// 把解码结果**直喂** HAL：`create_texture` + `upload_texture`（整幅一次上传）。
///
/// 纹理用 `TargetFormat::Rgba8Unorm`（线性、与 CPU 字节空间一致 —— 仓库颜色附件
/// 一律 `*_UNORM` 的同一口径），`readable: true`（截图与测试要回读）。
pub fn upload_bmp_to_texture(img: &BmpImage, device: &mut dyn Device) -> GpuResult<TextureId> {
    let expect = (img.width as usize) * (img.height as usize) * 4;
    if img.pixels.len() != expect {
        return Err(GpuError::Driver {
            code: -1,
            message: format!(
                "BmpImage 像素长度 {} 与 {}×{}×4 = {expect} 不符（解码器不该产出这种数据；\
                 若是手工构造的 BmpImage，请修正 pixels）",
                img.pixels.len(),
                img.width,
                img.height
            ),
        });
    }
    let id = device.create_texture(TextureDesc {
        width: img.width,
        height: img.height,
        format: TargetFormat::Rgba8Unorm,
        readable: true,
    })?;
    device.upload_texture(
        id,
        &img.pixels,
        TextureRegion {
            x: 0,
            y: 0,
            width: img.width,
            height: img.height,
        },
    )?;
    Ok(id)
}

// ── 内部字节读取 ─────────────────────────────────────────────────────────────

fn ensure_len(data: &[u8], end: usize, what: &str) -> Result<(), BmpError> {
    if data.len() < end {
        return Err(BmpError::Truncated {
            what: what.to_string(),
            need: end,
            have: data.len(),
        });
    }
    Ok(())
}

fn u16_le(data: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([data[at], data[at + 1]])
}

fn u32_le(data: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([data[at], data[at + 1], data[at + 2], data[at + 3]])
}

fn i32_le(data: &[u8], at: usize) -> i32 {
    i32::from_le_bytes([data[at], data[at + 1], data[at + 2], data[at + 3]])
}
