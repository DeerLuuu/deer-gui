//! M4 **合成字体测试**：测试自己构造一份最小但合法的 TrueType 字体。
//!
//! ## 为什么需要它
//!
//! `font_parse.rs` 依赖系统字体 —— 在别的机器上会跳过，等于没验证。
//! 这里手工构造一份 TTF，每个字节都由测试控制，**在任何机器上都有确定性对照组**。
//!
//! ## 合成字体的设计（数字都是手选的，便于人工核对）
//!
//! ```text
//!   unitsPerEm = 1000
//!   glyph 0 = .notdef   （空白，advance 500）
//!   glyph 1 = ' '       （空白，advance 250）
//!   glyph 2 = 'A'       （正方形轮廓 (200,200)-(700,700)，advance 800）
//!   cmap: format 4 子表（Windows BMP）
//!   loca: **长格式**（indexToLocFormat = 1）
//!   hmtx: numHMetrics = 3
//! ```
//!
//! 于是「'A' 的轮廓」是一个 500×500 units 的正方形 ——
//! 在 1000 unitsPerEm 下就是 **0.5 em**，缩放与光栅化时非常容易核对。

use deer_text::font::{Font, Segment};

// ── 小端/大端写入辅助 ────────────────────────────────────────────────────────

fn be16(v: &mut Vec<u8>, x: u16) {
    v.extend_from_slice(&x.to_be_bytes());
}
fn be32(v: &mut Vec<u8>, x: u32) {
    v.extend_from_slice(&x.to_be_bytes());
}
fn bei16(v: &mut Vec<u8>, x: i16) {
    v.extend_from_slice(&x.to_be_bytes());
}

/// 把表补齐到 4 字节边界并返回其校验和（表目录里要写）。
fn pad4(v: &mut Vec<u8>) {
    while v.len() % 4 != 0 {
        v.push(0);
    }
}

/// 4 字节表名的校验和（大端 u32 求和）。
fn checksum(data: &[u8]) -> u32 {
    let mut sum = 0u32;
    let mut i = 0;
    while i < data.len() {
        let mut w = [0u8; 4];
        let n = (data.len() - i).min(4);
        w[..n].copy_from_slice(&data[i..i + n]);
        sum = sum.wrapping_add(u32::from_be_bytes(w));
        i += 4;
    }
    sum
}

// ── 各表构造 ────────────────────────────────────────────────────────────────

fn table_head() -> Vec<u8> {
    let mut t = Vec::new();
    be32(&mut t, 0x0001_0000); // version 1.0
    be32(&mut t, 0x0001_0000); // fontRevision
    be32(&mut t, 0); // checkSumAdjustment（解析器不校验）
    be32(&mut t, 0x5F0F_3CF5); // magicNumber
    be16(&mut t, 0); // flags
    be16(&mut t, 1000); // unitsPerEm
    be32(&mut t, 0); // created（8 字节日期，用 0 占位）
    be32(&mut t, 0);
    be32(&mut t, 0); // modified
    be32(&mut t, 0);
    bei16(&mut t, 200); // xMin
    bei16(&mut t, 0); // yMin
    bei16(&mut t, 700); // xMax
    bei16(&mut t, 700); // yMax
    be16(&mut t, 0); // macStyle
    be16(&mut t, 8); // lowestRecPPEM
    bei16(&mut t, 2); // fontDirectionHint
    bei16(&mut t, 1); // indexToLocFormat = 1（长格式）
    bei16(&mut t, 0); // glyphDataFormat
    assert_eq!(t.len(), 54, "head 表必须是 54 字节");
    t
}

fn table_hhea() -> Vec<u8> {
    let mut t = Vec::new();
    be32(&mut t, 0x0001_0000); // version
    bei16(&mut t, 800); // ascender
    bei16(&mut t, -200); // descender
    bei16(&mut t, 0); // lineGap
    be16(&mut t, 800); // advanceWidthMax
    bei16(&mut t, 0); // minLeftSideBearing
    bei16(&mut t, 0); // minRightSideBearing
    bei16(&mut t, 700); // xMaxExtent
    bei16(&mut t, 1); // caretSlopeRise
    bei16(&mut t, 0); // caretSlopeRun
    bei16(&mut t, 0); // caretOffset
    for _ in 0..4 {
        bei16(&mut t, 0); // reserved ×4
    }
    bei16(&mut t, 0); // metricDataFormat
    be16(&mut t, 3); // numberOfHMetrics
    assert_eq!(t.len(), 36, "hhea 表必须是 36 字节");
    t
}

fn table_maxp() -> Vec<u8> {
    let mut t = Vec::new();
    be32(&mut t, 0x0001_0000); // version 1.0
    be16(&mut t, 3); // numGlyphs = 3
    // 其余 13 个 u16 字段（maxPoints/maxContours/...）全 0 即可
    for _ in 0..13 {
        be16(&mut t, 0);
    }
    assert_eq!(t.len(), 32, "maxp 1.0 必须是 32 字节");
    t
}

/// 一个「正方形」字形轮廓：(200,200)-(700,700)，逆时针。
fn glyph_square() -> Vec<u8> {
    let mut g = Vec::new();
    bei16(&mut g, 1); // numberOfContours = 1
    bei16(&mut g, 200); // xMin
    bei16(&mut g, 200); // yMin
    bei16(&mut g, 700); // xMax
    bei16(&mut g, 700); // yMax
    be16(&mut g, 3); // endPtsOfContours[0] = 3 ⇒ 4 个点
    be16(&mut g, 0); // instructionLength
    // 4 个点的标志：全部 on-curve + 用 16 位增量（稳妥起见不用 short）
    // 位含义：0x01 = ON_CURVE_POINT；0x02 = X_SHORT；0x04 = Y_SHORT
    // 0x02=0 且 0x10=0 ⇒ 读 16 位 x 增量；0x04=0 且 0x20=0 ⇒ 读 16 位 y 增量
    for _ in 0..4 {
        g.extend_from_slice(&[0x01]);
    }
    // x 增量（相对前一点，起点 0）：200, 500, 0, -500
    for dx in [200i16, 500, 0, -500] {
        bei16(&mut g, dx);
    }
    // y 增量：200, 0, 500, 0
    for dy in [200i16, 0, 500, 0] {
        bei16(&mut g, dy);
    }
    g
}

fn table_loca() -> Vec<u8> {
    // 长格式：numGlyphs + 1 个 u32 偏移
    // glyph 0 (.notdef): 空 ⇒ 偏移相同
    // glyph 1 (' '):      空 ⇒ 偏移相同
    // glyph 2 ('A'):      有轮廓
    let square_len = glyph_square().len() as u32;
    let mut t = Vec::new();
    be32(&mut t, 0); // glyph 0 起
    be32(&mut t, 0); // glyph 0 止 = glyph 1 起（空）
    be32(&mut t, 0); // glyph 1 止 = glyph 2 起（空）
    be32(&mut t, square_len); // glyph 2 止
    t
}

fn table_glyf() -> Vec<u8> {
    // glyph 0 与 glyph 1 是空的（长度为 0，所以 glyf 里不占字节）
    glyph_square()
}

fn table_hmtx() -> Vec<u8> {
    let mut t = Vec::new();
    // 3 个字形各 4 字节：advance(2) + lsb(2)
    be16(&mut t, 500); // .notdef advance
    bei16(&mut t, 0);
    be16(&mut t, 250); // 空格 advance
    bei16(&mut t, 0);
    be16(&mut t, 800); // 'A' advance
    bei16(&mut t, 200);
    t
}

/// `cmap`：一个 format 4 子表，映射 U+0020 与 U+0041。
fn table_cmap() -> Vec<u8> {
    // ── format 4 子表内容 ──
    let mut sub = Vec::new();
    // idDelta 设计：让 U+0020 → 1、U+0041 → 2
    //   delta(0x20) = 1 - 0x20 = 0xFFE1
    //   delta(0x41) = 2 - 0x41 = 0xFFC1
    // 用「每段一个字符」的两段 + 终止段
    let seg_count = 3u16; // 段数（含终止段）
    let seg_x2 = seg_count * 2;
    // 搜索参数（规范要求；解析器不读，但真字体会写）。
    // 不把它写进字节流，只是为了让 seg_count 的取值有据可依。
    let _search_range = 2 * (1u16 << (15 - seg_count.leading_zeros()).min(15));
    be16(&mut sub, 4); // format
    be16(&mut sub, 16 + seg_x2 * 4 + 2); // length（下面会重算，这里占位）
    be16(&mut sub, 0); // language
    be16(&mut sub, seg_x2);
    be16(&mut sub, 0); // searchRange（解析器不读）
    be16(&mut sub, 0); // entrySelector
    be16(&mut sub, 0); // rangeShift
    // endCode[]
    be16(&mut sub, 0x0020);
    be16(&mut sub, 0x0041);
    be16(&mut sub, 0xFFFF);
    // reservedPad
    be16(&mut sub, 0);
    // startCode[]
    be16(&mut sub, 0x0020);
    be16(&mut sub, 0x0041);
    be16(&mut sub, 0xFFFF);
    // idDelta[]
    bei16(&mut sub, 0xFFE1u16 as i16);
    bei16(&mut sub, 0xFFC1u16 as i16);
    bei16(&mut sub, 1); // 终止段：0xFFFF + 1 = 0（回绕）
    // idRangeOffset[]：全 0 ⇒ 用 idDelta
    be16(&mut sub, 0);
    be16(&mut sub, 0);
    be16(&mut sub, 0);
    // glyphIdArray 为空
    let real_len = sub.len() as u16;
    sub[2..4].copy_from_slice(&real_len.to_be_bytes());

    // ── cmap 头 + 一条编码记录 ──
    let mut t = Vec::new();
    be16(&mut t, 0); // version
    be16(&mut t, 1); // numTables = 1
    be16(&mut t, 3); // platformID = Windows
    be16(&mut t, 1); // encodingID = Unicode BMP
    be32(&mut t, 12); // 子表偏移（= 头 4 字节 + 记录 8 字节）
    t.extend_from_slice(&sub);
    t
}

/// 构造完整的合成 TTF。
fn build_font() -> Vec<u8> {
    // (tag, data)
    let mut tables: Vec<([u8; 4], Vec<u8>)> = vec![
        (*b"cmap", table_cmap()),
        (*b"glyf", table_glyf()),
        (*b"head", table_head()),
        (*b"hhea", table_hhea()),
        (*b"hmtx", table_hmtx()),
        (*b"loca", table_loca()),
        (*b"maxp", table_maxp()),
    ];
    // 表目录要求按 tag 升序
    tables.sort_by_key(|(tag, _)| *tag);
    for (_, d) in tables.iter_mut() {
        pad4(d);
    }

    let num_tables = tables.len() as u16;
    // 搜索参数（规范要求；解析器不读，但真字体会写）
    let entry_selector = (15 - num_tables.leading_zeros() as u16).min(15);
    let search_range = (1u16 << entry_selector) * 16;
    let range_shift = num_tables * 16 - search_range;

    let mut out = Vec::new();
    be32(&mut out, 0x0001_0000); // sfnt version = TrueType
    be16(&mut out, num_tables);
    be16(&mut out, search_range);
    be16(&mut out, entry_selector);
    be16(&mut out, range_shift);

    // 先算各表在文件中的偏移
    let dir_size = 12 + num_tables as usize * 16;
    let mut offset = dir_size as u32;
    let mut entries = Vec::new();
    for (tag, data) in &tables {
        entries.push((*tag, checksum(data), offset, data.len() as u32));
        offset += data.len() as u32;
    }
    for (tag, cs, off, len) in &entries {
        out.extend_from_slice(tag);
        be32(&mut out, *cs);
        be32(&mut out, *off);
        be32(&mut out, *len);
    }
    for (_, data) in &tables {
        out.extend_from_slice(data);
    }
    out
}

// ── 测试 ────────────────────────────────────────────────────────────────────

#[test]
fn synthetic_font_parses_with_exact_values() {
    let data = build_font();
    println!("合成字体 {} 字节", data.len());
    let f = Font::parse(data).expect("合成字体必须能解析");

    assert_eq!(f.units_per_em, 1000, "手选的 unitsPerEm");
    assert_eq!(f.num_glyphs, 3, "手选的字形数");
    assert_eq!(f.ascender, 800, "手选的 ascender");
    assert_eq!(f.descender, -200, "手选的 descender");
    assert_eq!(f.line_gap, 0, "手选的 lineGap");
    assert_eq!(f.line_height_units(), 1000.0, "800 - (-200) + 0 = 1000");
    println!("度量全部匹配手选值 ✅");
}

#[test]
fn synthetic_font_cmap_maps_chars_to_expected_glyphs() {
    let f = Font::parse(build_font()).expect("解析");

    assert_eq!(f.glyph_index(' ').expect("查询"), Some(1), "空格 → 字形 1");
    assert_eq!(f.glyph_index('A').expect("查询"), Some(2), "'A' → 字形 2");
    assert_eq!(
        f.glyph_index('B').expect("查询"),
        None,
        "'B' 没有映射，必须返回 None（不能返回 0 = .notdef）"
    );
    assert_eq!(f.glyph_index('\u{4E2D}').expect("查询"), None, "汉字没有映射");
    println!("cmap 映射与手选值一致 ✅");
}

#[test]
fn synthetic_square_glyph_has_exact_geometry() {
    let f = Font::parse(build_font()).expect("解析");
    let gi = f.glyph_index('A').expect("查询").expect("有映射");
    assert_eq!(gi, 2);
    let g = f.glyph(gi).expect("取字形");

    // 手选：一个 (200,200)-(700,700) 的正方形
    assert_eq!(g.advance_width, 800, "手选的 advance");
    assert_eq!(g.bbox, (200, 200, 700, 700), "手选的 bbox");
    assert_eq!(g.contours.len(), 1, "正方形只有一条轮廓");

    let c = &g.contours[0];
    // 4 条直线（闭合），起点 (200,200)
    assert_eq!(c.start, (200.0, 200.0), "起点应是第一个点");
    assert_eq!(c.segments.len(), 4, "正方形应有 4 条边，实际 {}", c.segments.len());
    for s in &c.segments {
        assert!(
            matches!(s, Segment::Line { .. }),
            "正方形不该有曲线段，实际 {s:?}"
        );
    }
    let ob = g.outline_bbox().expect("bbox");
    assert_eq!(
        ob,
        (200.0, 200.0, 700.0, 700.0),
        "轮廓自算 bbox 必须精确等于手选值"
    );
    println!("正方形几何完全匹配：bbox={ob:?} advance={}", g.advance_width);
}

#[test]
fn synthetic_blank_glyphs_are_empty_not_missing() {
    let f = Font::parse(build_font()).expect("解析");

    // .notdef (0) 与空格 (1) 都是「空字形」：有度量但没有轮廓
    for gi in [0u16, 1] {
        let g = f.glyph(gi).expect("取字形");
        assert!(g.is_blank(), "字形 {gi} 应当是空轮廓");
        assert!(g.contours.is_empty());
        assert!(g.outline_bbox().is_none(), "空字形没有 bbox");
    }
    assert_eq!(f.glyph(1).expect("取").advance_width, 250, "空格的 advance");
    assert_eq!(f.glyph(0).expect("取").advance_width, 500, ".notdef 的 advance");
}

#[test]
fn synthetic_font_rejects_cff_variant() {
    // `OTTO` = OpenType/CFF：必须明确报「不支持 CFF」，不能静默给空轮廓
    let mut d = build_font();
    d[0..4].copy_from_slice(b"OTTO");
    let e = match Font::parse(d) {
        Ok(_) => panic!("OTTO 应当被拒绝"),
        Err(e) => e.to_string(),
    };
    assert!(
        e.contains("CFF"),
        "错误信息必须点名 CFF 并给出建议，实际：{e}"
    );
    println!("CFF 被明确拒绝：{e}");
}

#[test]
fn synthetic_ttc_container_is_handled() {
    // 把合成字体包成 ttcf 容器，应当仍能取到第一个字体。
    //
    // ⚠️ 注意：TTF 表目录里的偏移是**相对于该字体起始位置**的（不是文件绝对偏移）。
    // 所以「子字体从文件第 16 字节开始」时，只需要让容器记录 offset=16，
    // **不要**去改子字体内部的表偏移 —— 第一版就是在这里搞错，导致解析到错误位置
    // （症状：`head.unitsPerEm` 读成 0）。
    const SUB_OFFSET: u32 = 16;
    let inner = build_font();

    let mut ttc = Vec::new();
    ttc.extend_from_slice(b"ttcf");
    be32(&mut ttc, 0x0001_0000); // version
    be32(&mut ttc, 1); // numFonts
    be32(&mut ttc, SUB_OFFSET); // 第一个字体的偏移
    assert_eq!(ttc.len(), SUB_OFFSET as usize, "头部长度必须是 16");
    ttc.extend_from_slice(&inner);

    let f = match Font::parse(ttc.clone()) {
        Ok(f) => f,
        Err(e) => {
            // 诊断：把 ttcf 头与子字体表目录打出来
            let n = u16::from_be_bytes([ttc[SUB_OFFSET as usize + 4], ttc[SUB_OFFSET as usize + 5]]);
            println!("解析失败：{e}");
            println!("ttcf 头: {:?}", &ttc[0..16]);
            println!(
                "子字体起点 {SUB_OFFSET}: {:?}",
                &ttc[SUB_OFFSET as usize..SUB_OFFSET as usize + 12]
            );
            println!("子字体表数 = {n}");
            let head_abs = find_table_offset(&ttc[SUB_OFFSET as usize..], b"head");
            println!("head 的相对偏移 = {head_abs}（相对子字体起点）");
            println!("head 绝对偏移 = {}", head_abs + SUB_OFFSET as usize);
            let ha = head_abs + SUB_OFFSET as usize;
            // 按解析器的算法独立复算一遍 base 与 head_off
            let b = |o: usize| u32::from_be_bytes([ttc[o], ttc[o + 1], ttc[o + 2], ttc[o + 3]]);
            println!("解析器会算出的 base = {}", b(12));
            println!("  （= u32_at(data,12)，期望 16）");
            println!(
                "  注意：ttcf 头的 offsets[0] 在偏移 **12**，所以 base 应当是 {}",
                b(12)
            );
            println!("head 绝对偏移 = {ha}");
            println!("该处字节: {:?}", &ttc[ha..ha + 24]);
            println!(
                "解析器会算出的 unitsPerEm: {}",
                u16::from_be_bytes([ttc[ha + 18], ttc[ha + 19]])
            );
            panic!("ttcf 应能解析出第一个字体");
        }
    };
    assert_eq!(f.units_per_em, 1000, "必须取到子字体的 unitsPerEm（不是 0）");
    assert_eq!(f.glyph_index('A').expect("查询"), Some(2));
    assert_eq!(
        f.glyph(2).expect("取字形").bbox,
        (200, 200, 700, 700),
        "ttcf 里的子字体几何应与直接解析一致"
    );
    println!("ttcf 容器取第一个字体 ✅（子字体位于偏移 {SUB_OFFSET}）");
}

/// 短格式 `loca`（`indexToLocFormat = 0`）也要支持 —— 小字体常用它。
#[test]
fn short_loca_format_is_supported() {
    let mut data = build_font();
    // 找到 head 表并把 indexToLocFormat 改成 0（偏移 50）
    let head_off = find_table_offset(&data, b"head");
    data[head_off + 50..head_off + 52].copy_from_slice(&0i16.to_be_bytes());
    // 找到 loca 表并把长格式换成短格式（偏移 / 2，每项 u16）
    let loca_off = find_table_offset(&data, b"loca");
    let square_len = glyph_square().len() as u16;
    let short: Vec<u8> = [0u16, 0, 0, square_len / 2]
        .iter()
        .flat_map(|v| v.to_be_bytes())
        .collect();
    data[loca_off..loca_off + short.len()].copy_from_slice(&short);

    let f = Font::parse(data).expect("短 loca 应能解析");
    let g = f.glyph(2).expect("取字形");
    assert_eq!(g.bbox, (200, 200, 700, 700), "短 loca 下几何应一致");
    println!("短格式 loca 生效 ✅");
}

fn find_table_offset(data: &[u8], name: &[u8; 4]) -> usize {
    let n = u16::from_be_bytes([data[4], data[5]]) as usize;
    for i in 0..n {
        let o = 12 + i * 16;
        if &data[o..o + 4] == name {
            return u32::from_be_bytes([data[o + 8], data[o + 9], data[o + 10], data[o + 11]])
                as usize;
        }
    }
    panic!("找不到表 {}", String::from_utf8_lossy(name));
}
