//! 零依赖 PNG 编码器（RGBA8）。
//!
//! 为什么自己写：本项目**不允许引第三方依赖**，而渲染结果要能落成图片才看得见。
//! PNG 的最小可用子集不难：
//! - 用 **zlib 的 stored（未压缩）deflate 块** —— 合法 zlib 流，且**绕开压缩算法**；
//! - CRC32 与 Adler32 各二十行。
//!
//! 代价：文件比真实压缩大（约等于原始像素 + 少量开销）。对「看一眼渲染结果」足够，
//! 后续要减小体积再接 deflate 压缩（或换更省的字形/图集策略）。

/// 把 RGBA8 像素编码成 PNG 字节流。
///
/// `pixels` 长度必须是 `width * height * 4`（行优先、无 padding）。
pub fn encode_rgba(width: u32, height: u32, pixels: &[u8]) -> Result<Vec<u8>, String> {
    let expect = (width as usize) * (height as usize) * 4;
    if pixels.len() != expect {
        return Err(format!(
            "像素长度不匹配：期望 {expect}（{width}×{height}×4），实际 {}",
            pixels.len()
        ));
    }
    if width == 0 || height == 0 {
        return Err("宽高必须大于 0".to_string());
    }

    // 原始数据：每行前置一个 filter 字节（0 = None）
    let mut raw = Vec::with_capacity(expect + height as usize);
    for y in 0..height as usize {
        raw.push(0u8);
        let start = y * (width as usize) * 4;
        raw.extend_from_slice(&pixels[start..start + (width as usize) * 4]);
    }

    let mut out = Vec::new();
    out.extend_from_slice(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]);

    // IHDR
    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.push(8); // bit depth
    ihdr.push(6); // color type: RGBA
    ihdr.push(0); // compression: deflate
    ihdr.push(0); // filter method
    ihdr.push(0); // interlace: none
    write_chunk(&mut out, b"IHDR", &ihdr);

    // IDAT（zlib 包装的 stored deflate）
    let idat = zlib_store(&raw);
    write_chunk(&mut out, b"IDAT", &idat);

    // IEND
    write_chunk(&mut out, b"IEND", &[]);
    Ok(out)
}

fn write_chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let mut crc_input = Vec::with_capacity(4 + data.len());
    crc_input.extend_from_slice(kind);
    crc_input.extend_from_slice(data);
    out.extend_from_slice(&crc32(&crc_input).to_be_bytes());
}

/// zlib 容器：2 字节头 + deflate stored 块 + 4 字节 Adler32。
fn zlib_store(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() + 16);
    out.push(0x78); // CMF: deflate, 32K window
    out.push(0x01); // FLG: 无字典，压缩级别最低 ⇒ 校验位使 (0x78<<8|0x01) % 31 == 0
                    // deflate: 每个 stored 块最多 65535 字节
    if data.is_empty() {
        out.extend_from_slice(&[0x01, 0x00, 0x00, 0xff, 0xff]);
    } else {
        let mut offset = 0usize;
        while offset < data.len() {
            let len = (data.len() - offset).min(0xffff);
            let is_last = offset + len >= data.len();
            out.push(if is_last { 0x01 } else { 0x00 }); // BFINAL + BTYPE=00(stored)
            let n = len as u16;
            out.extend_from_slice(&n.to_le_bytes());
            out.extend_from_slice(&(!n).to_le_bytes());
            out.extend_from_slice(&data[offset..offset + len]);
            offset += len;
        }
    }
    out.extend_from_slice(&adler32(data).to_be_bytes());
    out
}

fn crc32(data: &[u8]) -> u32 {
    // 反射多项式 0xEDB88320，逐位实现（表驱动可后置；这里只求正确）
    let mut crc = 0xffff_ffffu32;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xedb8_8320 & mask);
        }
    }
    !crc
}

fn adler32(data: &[u8]) -> u32 {
    const MOD: u32 = 65521;
    let (mut a, mut b) = (1u32, 0u32);
    for &byte in data {
        a = (a + byte as u32) % MOD;
        b = (b + a) % MOD;
    }
    (b << 16) | a
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn png_header_and_chunks_are_well_formed() {
        let px = vec![255u8; 2 * 2 * 4];
        let png = encode_rgba(2, 2, &px).expect("编码");
        // 8 字节签名
        assert_eq!(&png[..8], &[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]);
        // IHDR 长度必须是 13
        assert_eq!(&png[8..12], &13u32.to_be_bytes());
        assert_eq!(&png[12..16], b"IHDR");
        // 宽高
        assert_eq!(&png[16..20], &2u32.to_be_bytes());
        assert_eq!(&png[20..24], &2u32.to_be_bytes());
        // 位深 8 / 颜色类型 6(RGBA)
        assert_eq!(png[24], 8);
        assert_eq!(png[25], 6);
        // 以 IEND 结尾
        assert_eq!(&png[png.len() - 8..png.len() - 4], b"IEND");
    }

    #[test]
    fn crc32_known_vector() {
        // "123456789" 的 CRC32 是 0xCBF43926
        assert_eq!(crc32(b"123456789"), 0xcbf4_3926);
    }

    #[test]
    fn adler32_known_vector() {
        // "Wikipedia" 的 Adler32 是 0x11E60398
        assert_eq!(adler32(b"Wikipedia"), 0x11e6_0398);
    }

    #[test]
    fn zlib_header_check_bits() {
        let z = zlib_store(&[1, 2, 3]);
        let header = (z[0] as u32) << 8 | z[1] as u32;
        assert_eq!(header % 31, 0, "zlib 头的校验位必须满足 % 31 == 0");
    }

    #[test]
    fn rejects_wrong_pixel_length() {
        assert!(encode_rgba(2, 2, &[0u8; 3]).is_err());
        assert!(encode_rgba(0, 1, &[]).is_err());
    }

    #[test]
    fn large_image_splits_into_multiple_stored_blocks() {
        // 超过 65535 字节 ⇒ 必须分块，否则非法
        let px = vec![7u8; 200 * 200 * 4];
        let png = encode_rgba(200, 200, &px).expect("编码大图");
        assert!(png.len() > px.len(), "PNG 至少应包含原始数据");
    }
}
