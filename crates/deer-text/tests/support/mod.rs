//! 测试用**合成字体**构造器（M4，t2）。
//!
//! ## 为什么复制一份而不是复用 `tests/font_synthetic.rs`
//!
//! 集成测试每个文件是**独立 crate**，`tests/font_synthetic.rs` 里的
//! `build_font()` 是私有的、不能跨文件 `use`。而任务纪律要求
//! **不改 `font_synthetic.rs` 的断言与测试名**，所以这里放一份**同源**的构造器：
//! 字节级设计与那边完全一致（`unitsPerEm = 1000`、`.notdef` advance 500、
//! 空格 250、'A' = 800 且轮廓是 (200,200)-(700,700) 的正方形），
//! 于是所有断言都能用手选数字人工核对。
//!
//! 多出来的一处：`build_font_with_unreadable_hmtx()` —— 把 `hmtx` 的目录项
//! 指到文件最后一个字节（长度 0），从而让 `Font::glyph()` **必然报错**，
//! 用来覆盖 `FontMeasure::advance` 的第三条回退分支（`0.5em` 兜底）。
//! 这是「坏字体」的人造样本，不是合法字体。

#![allow(dead_code)] // 集成测试各自只用到一部分构造器

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

/// 把表补齐到 4 字节边界。
fn pad4(v: &mut Vec<u8>) {
    while v.len() % 4 != 0 {
        v.push(0);
    }
}

/// 4 字节字校验和（大端 u32 求和）。
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

// ── 各表构造（数字与 font_synthetic.rs 同源，全是手选值） ────────────────────

fn table_head() -> Vec<u8> {
    let mut t = Vec::new();
    be32(&mut t, 0x0001_0000); // version 1.0
    be32(&mut t, 0x0001_0000); // fontRevision
    be32(&mut t, 0); // checkSumAdjustment（解析器不校验）
    be32(&mut t, 0x5F0F_3CF5); // magicNumber
    be16(&mut t, 0); // flags
    be16(&mut t, 1000); // unitsPerEm
    be32(&mut t, 0); // created
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
    for _ in 0..13 {
        be16(&mut t, 0);
    }
    assert_eq!(t.len(), 32, "maxp 1.0 必须是 32 字节");
    t
}

/// 一个「正方形」字形轮廓：(200,200)-(700,700)。
fn glyph_square() -> Vec<u8> {
    let mut g = Vec::new();
    bei16(&mut g, 1); // numberOfContours = 1
    bei16(&mut g, 200); // xMin
    bei16(&mut g, 200); // yMin
    bei16(&mut g, 700); // xMax
    bei16(&mut g, 700); // yMax
    be16(&mut g, 3); // endPtsOfContours[0] ⇒ 4 个点
    be16(&mut g, 0); // instructionLength
    for _ in 0..4 {
        g.extend_from_slice(&[0x01]); // 全部 on-curve、16 位增量
    }
    for dx in [200i16, 500, 0, -500] {
        bei16(&mut g, dx);
    }
    for dy in [200i16, 0, 500, 0] {
        bei16(&mut g, dy);
    }
    g
}

fn table_loca() -> Vec<u8> {
    let square_len = glyph_square().len() as u32;
    let mut t = Vec::new();
    be32(&mut t, 0); // glyph 0 起
    be32(&mut t, 0); // glyph 0 止 = glyph 1 起（空）
    be32(&mut t, 0); // glyph 1 止 = glyph 2 起（空）
    be32(&mut t, square_len); // glyph 2 止
    t
}

fn table_glyf() -> Vec<u8> {
    glyph_square()
}

fn table_hmtx() -> Vec<u8> {
    let mut t = Vec::new();
    be16(&mut t, 500); // glyph 0 (.notdef) advance
    bei16(&mut t, 0);
    be16(&mut t, 250); // glyph 1 (' ') advance
    bei16(&mut t, 0);
    be16(&mut t, 800); // glyph 2 ('A') advance
    bei16(&mut t, 200);
    t
}

/// `cmap`：一个 format 4 子表，映射 U+0020 → 字形 1、U+0041 → 字形 2。
fn table_cmap() -> Vec<u8> {
    let mut sub = Vec::new();
    let seg_count = 3u16; // 两段 + 终止段
    let seg_x2 = seg_count * 2;
    be16(&mut sub, 4); // format
    be16(&mut sub, 16 + seg_x2 * 4 + 2); // length（下面重算）
    be16(&mut sub, 0); // language
    be16(&mut sub, seg_x2);
    be16(&mut sub, 0); // searchRange（解析器不读）
    be16(&mut sub, 0); // entrySelector
    be16(&mut sub, 0); // rangeShift
    be16(&mut sub, 0x0020); // endCode
    be16(&mut sub, 0x0041);
    be16(&mut sub, 0xFFFF);
    be16(&mut sub, 0); // reservedPad
    be16(&mut sub, 0x0020); // startCode
    be16(&mut sub, 0x0041);
    be16(&mut sub, 0xFFFF);
    bei16(&mut sub, 0xFFE1u16 as i16); // idDelta：U+0020 → 1
    bei16(&mut sub, 0xFFC1u16 as i16); // idDelta：U+0041 → 2
    bei16(&mut sub, 1); // 终止段：0xFFFF + 1 = 0
    be16(&mut sub, 0); // idRangeOffset ×3
    be16(&mut sub, 0);
    be16(&mut sub, 0);
    let real_len = sub.len() as u16;
    sub[2..4].copy_from_slice(&real_len.to_be_bytes());

    let mut t = Vec::new();
    be16(&mut t, 0); // version
    be16(&mut t, 1); // numTables = 1
    be16(&mut t, 3); // platformID = Windows
    be16(&mut t, 1); // encodingID = Unicode BMP
    be32(&mut t, 12); // 子表偏移
    t.extend_from_slice(&sub);
    t
}

/// 构造完整的合成 TTF。
pub fn build_font() -> Vec<u8> {
    let mut tables: Vec<([u8; 4], Vec<u8>)> = vec![
        (*b"cmap", table_cmap()),
        (*b"glyf", table_glyf()),
        (*b"head", table_head()),
        (*b"hhea", table_hhea()),
        (*b"hmtx", table_hmtx()),
        (*b"loca", table_loca()),
        (*b"maxp", table_maxp()),
    ];
    tables.sort_by_key(|(tag, _)| *tag);
    for (_, d) in tables.iter_mut() {
        pad4(d);
    }

    let num_tables = tables.len() as u16;
    let entry_selector = (15 - num_tables.leading_zeros() as u16).min(15);
    let search_range = (1u16 << entry_selector) * 16;
    let range_shift = num_tables * 16 - search_range;

    let mut out = Vec::new();
    be32(&mut out, 0x0001_0000); // sfnt version = TrueType
    be16(&mut out, num_tables);
    be16(&mut out, search_range);
    be16(&mut out, entry_selector);
    be16(&mut out, range_shift);

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

/// 表目录里某个表的**目录项偏移**（不是表数据偏移）。
pub fn dir_entry_offset(data: &[u8], name: &[u8; 4]) -> usize {
    let n = u16::from_be_bytes([data[4], data[5]]) as usize;
    for i in 0..n {
        let o = 12 + i * 16;
        if &data[o..o + 4] == name {
            return o;
        }
    }
    panic!("找不到表 {}", String::from_utf8_lossy(name));
}

/// 表目录里某个表的**数据偏移**。
pub fn table_offset(data: &[u8], name: &[u8; 4]) -> usize {
    let o = dir_entry_offset(data, name);
    u32::from_be_bytes([data[o + 8], data[o + 9], data[o + 10], data[o + 11]]) as usize
}

/// 一份**坏字体**：`hmtx` 的目录项被指到文件最后一个字节且长度 0。
///
/// 目的：让 `Font::parse` 仍然成功（表存在、边界检查通过），但
/// `Font::glyph(任何)` 在读 `hmtx` 时必然越界报错 —— 用来覆盖
/// `FontMeasure::advance` 的第三条回退分支（`0.5 * font_size`）。
///
/// 注意：这份数据**不是合法字体**，只用于「防御性分支」的测试。
pub fn build_font_with_unreadable_hmtx() -> Vec<u8> {
    let mut data = build_font();
    let entry = dir_entry_offset(&data, b"hmtx");
    let last = (data.len() - 1) as u32;
    data[entry + 8..entry + 12].copy_from_slice(&last.to_be_bytes()); // offset
    data[entry + 12..entry + 16].copy_from_slice(&0u32.to_be_bytes()); // length = 0
    data
}

/// 94 个可见 ASCII（`!`..=`~`）。测试里的「可见 ASCII」都指这一串。
pub fn visible_ascii() -> Vec<char> {
    ('!'..='~').collect()
}
