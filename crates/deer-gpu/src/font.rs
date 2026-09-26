//! **零依赖 TrueType 字体解析**（M4）。
//!
//! 为什么不引 `ttf-parser` / `fontdue` / `ab_glyph`：本项目的硬约束是**零第三方依赖**
//! （见 `README.md`）。字形解析是纯数据读取 + 纯算法，自己写完全可控，
//! 而且**每一处偏移都有断言兜住**（这比信任一个黑盒库更适合本项目的验证纪律）。
//!
//! ## 支持范围（先说清边界）
//!
//! | 项 | 状态 |
//! |---|---|
//! | sfnt 表目录 | ✅ |
//! | `head`（`unitsPerEm` / `indexToLocFormat` / bbox） | ✅ |
//! | `hhea` / `hmtx`（水平度量、advance width） | ✅ |
//! | `maxp`（字形数） | ✅ |
//! | `cmap` **format 4**（BMP） | ✅ |
//! | `cmap` **format 12**（全 Unicode） | ✅ |
//! | `cmap` format 0 / 6 | ✅（简单映射） |
//! | `loca`（短/长两种格式） | ✅ |
//! | `glyf` 简单轮廓（含二次贝塞尔、重复标志） | ✅ |
//! | `glyf` 复合字形（`ARGS_ARE_XY_VALUES`、缩放） | ✅ |
//! | **CFF / OpenType-CFF 轮廓** | ❌ 报错，不静默给空轮廓 |
//! | 竖排、变体、着色、GSUB/GPOS、字距 | ❌ 未实现 |
//! | hinting | ❌ 不做（光栅化时用超采样代替） |
//!
//! ## 坐标与单位
//!
//! `glyf` 里的坐标是 **font units**（通常是 2048 或 1000 / em）。
//! 本模块**不改坐标**，只把「每 em 多少单位」(`units_per_em`) 给出去 ——
//! 缩放到像素是调用方的事（见 `deer-gpu::text`）。
//! 这样解析层保持纯粹，度量与缩放可以在别处单测。

use crate::error::{GpuError, GpuResult};

/// 一个轮廓点：`(x, y, on_curve)`。
///
/// TrueType 的二次贝塞尔用「隐含中点」表示：连续两个**非**on-curve 点之间
/// 隐含一个 on-curve 中点。`Outline` 已经把它展平成显式段（见 [`Segment`]）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Point {
    pub x: f32,
    pub y: f32,
    pub on_curve: bool,
}

/// 展平后的轮廓段。已经消掉了「隐含中点」这层间接。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Segment {
    /// 直线到终点
    Line { to: (f32, f32) },
    /// 二次贝塞尔（`ctrl` 是控制点）
    Quad { ctrl: (f32, f32), to: (f32, f32) },
    /// 三次贝塞尔（TrueType 里没有，保留给未来 CFF 支持）
    Cubic {
        c1: (f32, f32),
        c2: (f32, f32),
        to: (f32, f32),
    },
}

/// 一个封闭轮廓：起点 + 若干段。坐标是 **font units**。
#[derive(Debug, Clone, PartialEq)]
pub struct Contour {
    pub start: (f32, f32),
    pub segments: Vec<Segment>,
}

/// 一个字形。
#[derive(Debug, Clone, PartialEq)]
pub struct Glyph {
    /// `glyf` 里的轮廓（可能为空：空格等无轮廓字形）
    pub contours: Vec<Contour>,
    /// 原始 bbox（font units），来自 `glyf` 头；无轮廓字形全 0
    pub bbox: (i16, i16, i16, i16),
    /// advance width（font units）
    pub advance_width: u16,
    /// 左边距（来自 `hmtx` 的 lsb 或 glyf bbox.x_min）
    pub left_side_bearing: i16,
}

impl Glyph {
    /// 是否是「无轮廓」字形（空格、组合记号等）。
    pub fn is_blank(&self) -> bool {
        self.contours.is_empty()
    }

    /// 轮廓的 bbox（font units，浮点）。空轮廓返回 `None`。
    ///
    /// 与 [`Glyph::bbox`] 的区别：这个是从**实际点**算出来的，
    /// 可以用来交叉验证 `glyf` 头里的 bbox 有没有读错。
    pub fn outline_bbox(&self) -> Option<(f32, f32, f32, f32)> {
        let mut it = self.contours.iter().flat_map(|c| {
            std::iter::once(c.start).chain(c.segments.iter().flat_map(|s| match s {
                Segment::Line { to } => vec![*to],
                Segment::Quad { ctrl, to } => vec![*ctrl, *to],
                Segment::Cubic { c1, c2, to } => vec![*c1, *c2, *to],
            }))
        });
        let first = it.next()?;
        let (mut x0, mut y0, mut x1, mut y1) = (first.0, first.1, first.0, first.1);
        for (x, y) in it {
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x);
            y1 = y1.max(y);
        }
        Some((x0, y0, x1, y1))
    }
}

/// `cmap` 子表的格式（用于诊断「为什么找不到某个字符」）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CmapFormat {
    Format0,
    Format4,
    Format6,
    Format12,
}

/// 已解析的字体。
pub struct Font {
    data: Vec<u8>,
    /// 表名 → (偏移, 长度)
    tables: Vec<([u8; 4], u32, u32)>,
    pub units_per_em: u16,
    pub num_glyphs: u16,
    /// `hhea` 的升部（font units，通常正）
    pub ascender: i16,
    /// 降部（font units，通常负）
    pub descender: i16,
    pub line_gap: i16,
    /// `cmap` 选中子表的格式
    pub cmap_format: CmapFormat,
    index_to_loc_format: i16,
    num_h_metrics: u16,
    /// 这个字体在文件里的起始偏移（`ttcf` 容器里 > 0，普通 ttf 是 0）
    face_offset: usize,
}

/// 从 4 字节 tag 读表名。
fn tag(b: &[u8], off: usize) -> GpuResult<[u8; 4]> {
    let s = b.get(off..off + 4).ok_or_else(|| GpuError::Unsupported(format!(
        "字体数据在偏移 {off} 处越界（tag）"
    )))?;
    Ok([s[0], s[1], s[2], s[3]])
}

fn u8_at(b: &[u8], off: usize) -> GpuResult<u8> {
    b.get(off)
        .copied()
        .ok_or_else(|| GpuError::Unsupported(format!("字体数据在偏移 {off} 处越界（u8）")))
}

fn u16_at(b: &[u8], off: usize) -> GpuResult<u16> {
    let s = b
        .get(off..off + 2)
        .ok_or_else(|| GpuError::Unsupported(format!("字体数据在偏移 {off} 处越界（u16）")))?;
    Ok(u16::from_be_bytes([s[0], s[1]]))
}

fn i16_at(b: &[u8], off: usize) -> GpuResult<i16> {
    Ok(u16_at(b, off)? as i16)
}

fn u32_at(b: &[u8], off: usize) -> GpuResult<u32> {
    let s = b
        .get(off..off + 4)
        .ok_or_else(|| GpuError::Unsupported(format!("字体数据在偏移 {off} 处越界（u32）")))?;
    Ok(u32::from_be_bytes([s[0], s[1], s[2], s[3]]))
}

impl Font {
    /// 解析字体。
    ///
    /// 支持 sfnt 包装（`\x00\x01\x00\x00`）与 **TrueType collection**（`ttcf`）。
    /// 对 `ttcf` 取第一个字体（`face_index = 0`）。
    pub fn parse(data: Vec<u8>) -> GpuResult<Font> {        if data.len() < 12 {
            return Err(GpuError::Unsupported("字体数据太短（< 12 字节）".to_string()));
        }
        // ttcf：`ttcf` + version + numFonts + offsets[]
        let base = if &data[0..4] == b"ttcf" {
            let n = u32_at(&data, 8)?;
            if n == 0 {
                return Err(GpuError::Unsupported("ttcf 里没有字体".to_string()));
            }
            u32_at(&data, 12)? as usize
        } else {
            0
        };

        let sfnt = u32_at(&data, base)?;
        // 0x00010000 = TrueType；`true` = 老式 Mac TrueType；`OTTO` = CFF
        match sfnt {
            0x0001_0000 | 0x7472_7565 => {}
            0x4F54_544F => {
                return Err(GpuError::Unsupported(
                    "这是 OpenType/CFF 字体（`OTTO`），本项目暂不支持 CFF 轮廓。\
                     请用 TrueType 字体（含 `glyf` 表）"
                        .to_string(),
                ));
            }
            other => {
                return Err(GpuError::Unsupported(format!(
                    "未知的 sfnt 版本 {other:#010x}（期望 0x00010000 / 'true'）"
                )));
            }
        }

        let num_tables = u16_at(&data, base + 4)?;
        if num_tables == 0 || num_tables > 512 {
            return Err(GpuError::Unsupported(format!(
                "表目录数量不合理：{num_tables}"
            )));
        }
        let mut tables = Vec::with_capacity(num_tables as usize);
        for i in 0..num_tables as usize {
            let off = base + 12 + i * 16;
            let t = tag(&data, off)?;
            let toff = u32_at(&data, off + 8)?;
            let tlen = u32_at(&data, off + 12)?;
            // 边界检查：表必须落在数据内（防止后面越界读到垃圾）
            let end = toff as usize + tlen as usize;
            if end > data.len() {
                return Err(GpuError::Unsupported(format!(
                    "表 {} 越界：偏移 {toff} + 长度 {tlen} > 数据 {} 字节",
                    String::from_utf8_lossy(&t),
                    data.len()
                )));
            }
            tables.push((t, toff, tlen));
        }

        // 注意：表目录里的偏移是**相对于该字体起始位置**的。这里统一加上 `base`
        // ⇒ `find` 返回的**永远是绝对文件偏移**，调用方不必也不该再加 `base`。
        //
        // 这一条踩过坑：`ttcf` 容器里 `head`/`maxp`/`hhea` 三个表的处理不一致
        // （只有 head 加了 base），于是 `hhea` 读到错误位置、报出莫名其妙的越界。
        let find = |name: &[u8; 4]| -> Option<(u32, u32)> {
            tables
                .iter()
                .find(|(t, _, _)| t == name)
                .map(|(_, o, l)| ((*o as usize + base) as u32, *l))
        };

        // ── head ──
        let (head_off, _) = find(b"head").ok_or_else(|| {
            GpuError::Unsupported("字体缺少 `head` 表".to_string())
        })? as (u32, u32);
        let head_off = head_off as usize;
        // 临时诊断（用 DEER_FONT_DEBUG=1 打开）
        if std::env::var("DEER_FONT_DEBUG").is_ok() {
            eprintln!(
                "[font] base={base} num_tables={num_tables} head_off={head_off} \
                 head_abs={} unitsPerEm读取处字节={:?}",
                head_off,
                data.get(head_off + 18..head_off + 20)
            );
        }
        let units_per_em = u16_at(&data, head_off + 18)?;
        if units_per_em == 0 {
            return Err(GpuError::Unsupported("`head.unitsPerEm` 为 0".to_string()));
        }
        let index_to_loc_format = i16_at(&data, head_off + 50)?;
        if index_to_loc_format != 0 && index_to_loc_format != 1 {
            return Err(GpuError::Unsupported(format!(
                "`head.indexToLocFormat` 必须是 0 或 1，实际 {index_to_loc_format}"
            )));
        }

        // ── maxp ──
        let (maxp_off, _) = find(b"maxp")
            .ok_or_else(|| GpuError::Unsupported("字体缺少 `maxp` 表".to_string()))?;
        let num_glyphs = u16_at(&data, maxp_off as usize + 4)?;

        // ── hhea ──
        let (hhea_off, _) = find(b"hhea")
            .ok_or_else(|| GpuError::Unsupported("字体缺少 `hhea` 表".to_string()))?;
        let hhea_off = hhea_off as usize;
        let ascender = i16_at(&data, hhea_off + 4)?;
        let descender = i16_at(&data, hhea_off + 6)?;
        let line_gap = i16_at(&data, hhea_off + 8)?;
        let num_h_metrics = u16_at(&data, hhea_off + 34)?;

        // ── 必需表存在性 ──
        for required in [b"cmap", b"loca", b"glyf", b"hmtx"] {
            if find(required).is_none() {
                return Err(GpuError::Unsupported(format!(
                    "字体缺少 `{}` 表（本实现只支持 TrueType `glyf` 轮廓）",
                    String::from_utf8_lossy(required)
                )));
            }
        }

        // ── cmap：挑一个可用的子表（优先 format 12 全 Unicode，其次 format 4） ──
        let cmap_format = {
            let (cm_off, _) = find(b"cmap").unwrap();
            let cm_off = cm_off as usize;
            let n = u16_at(&data, cm_off + 2)?;
            let mut best: Option<CmapFormat> = None;
            for i in 0..n as usize {
                let rec = cm_off + 4 + i * 8;
                let pid = u16_at(&data, rec)?;
                let eid = u16_at(&data, rec + 2)?;
                let sub = cm_off + u32_at(&data, rec + 4)? as usize;
                let fmt = u16_at(&data, sub)?;
                // Windows(3)：eid 10 = UCS-4，eid 1 = BMP；Unicode(0) 平台也接受
                let windows_ok = pid == 3 && (eid == 10 || eid == 1);
                let unicode_ok = pid == 0;
                if !(windows_ok || unicode_ok) {
                    continue;
                }
                let cand = match fmt {
                    0 => CmapFormat::Format0,
                    4 => CmapFormat::Format4,
                    6 => CmapFormat::Format6,
                    12 => CmapFormat::Format12,
                    _ => continue,
                };
                // format 12 优先（能覆盖 BMP 之外）
                best = match (best, cand) {
                    (Some(CmapFormat::Format12), _) => Some(CmapFormat::Format12),
                    (_, CmapFormat::Format12) => Some(CmapFormat::Format12),
                    (Some(CmapFormat::Format4), _) => Some(CmapFormat::Format4),
                    (_, c) => Some(c),
                };
            }
            best.ok_or_else(|| {
                GpuError::Unsupported(
                    "`cmap` 里没有可用的子表（只支持 format 0/4/6/12 的 Unicode 子表）".to_string(),
                )
            })?
        };

        Ok(Font {
            data,
            tables,
            units_per_em,
            num_glyphs,
            ascender,
            descender,
            line_gap,
            cmap_format,
            index_to_loc_format,
            num_h_metrics,
            face_offset: base,
        })
    }

    /// 取某个表在**文件里的绝对偏移**与长度。找不到返回 `None`。
    ///
    /// 诊断用：当解析出的度量明显不对时，先确认真实表目录里各表的偏移，
    /// 比对着猜快得多。
    pub fn table_range(&self, name: &[u8; 4]) -> Option<(u32, u32)> {
        self.tables
            .iter()
            .find(|(t, _, _)| t == name)
            .map(|(_, o, l)| (*o, *l))
    }

    /// 表目录里所有表的名字与偏移（诊断用）。
    pub fn table_directory(&self) -> Vec<([u8; 4], u32, u32)> {
        self.tables.clone()
    }

    /// 生成这个字体的数据来自文件里的哪个字节偏移（`ttcf` 里 > 0）。
    pub fn face_offset(&self) -> usize {
        self.face_offset
    }

    /// 取表的 `(绝对偏移, 长度)`，并**校验表确实落在文件内**。
    ///
    /// **表目录里的偏移是相对于字体起始位置的**（`ttcf` 容器下也如此），
    /// 所以这里统一加上 `face_offset` 再返回绝对偏移 —— 所有调用点因此
    /// 不需要各自记得加 `base`（这一点踩过坑：`head` 被读成了 0）。
    ///
    /// 边界校验也是必需的：坏字体会声明超出文件的表长度，而后续按该长度
    /// 读取会读到垃圾或越界。这里一次性拦掉，错误信息里带上表名。
    fn table(&self, name: &[u8; 4]) -> GpuResult<(usize, usize)> {
        let (off, len) = self
            .tables
            .iter()
            .find(|(t, _, _)| t == name)
            .map(|(_, o, l)| (*o as usize, *l as usize))
            .ok_or_else(|| {
                GpuError::Unsupported(format!("缺少表 {}", String::from_utf8_lossy(name)))
            })?;
        let abs = self.face_offset + off;
        if abs + len > self.data.len() {
            return Err(GpuError::Unsupported(format!(
                "表 {} 越界：偏移 {abs} + 长度 {len} > 数据 {} 字节",
                String::from_utf8_lossy(name),
                self.data.len()
            )));
        }
        Ok((abs, len))
    }

    /// 字符 → 字形索引。找不到返回 `None`（**不返回 0**，因为 0 是 `.notdef`，
    /// 两者含义不同：缺字符应当由调用方决定画什么）。
    pub fn glyph_index(&self, ch: char) -> GpuResult<Option<u16>> {
        let cp = ch as u32;
        let (cm_off, _) = self.table(b"cmap")?;
        let n = u16_at(&self.data, cm_off + 2)?;
        for i in 0..n as usize {
            let rec = cm_off + 4 + i * 8;
            let pid = u16_at(&self.data, rec)?;
            let eid = u16_at(&self.data, rec + 2)?;
            let sub = cm_off + u32_at(&self.data, rec + 4)? as usize;
            let fmt = u16_at(&self.data, sub)?;
            let windows_ok = pid == 3 && (eid == 10 || eid == 1);
            let unicode_ok = pid == 0;
            if !(windows_ok || unicode_ok) {
                continue;
            }
            let hit = match fmt {
                0 => self.cmap0(sub, cp),
                4 => self.cmap4(sub, cp),
                6 => self.cmap6(sub, cp),
                12 => self.cmap12(sub, cp),
                _ => None,
            };
            if let Some(g) = hit {
                // 0xFFFF / 0 视为「未映射」（规范允许用 0 表示缺失）
                if g != 0 {
                    return Ok(Some(g));
                }
            }
        }
        Ok(None)
    }

    fn cmap0(&self, sub: usize, cp: u32) -> Option<u16> {
        if cp > 255 {
            return None;
        }
        u8_at(&self.data, sub + 6 + cp as usize).ok().map(u16::from)
    }

    fn cmap4(&self, sub: usize, cp: u32) -> Option<u16> {
        if cp > 0xFFFF {
            return None;
        }
        let seg_x2 = u16_at(&self.data, sub + 6).ok()? as usize;
        let seg_count = seg_x2 / 2;
        let end_base = sub + 14;
        let start_base = end_base + seg_x2 + 2;
        let delta_base = start_base + seg_x2;
        let range_base = delta_base + seg_x2;
        let c = cp as u16;
        for i in 0..seg_count {
            let end = u16_at(&self.data, end_base + i * 2).ok()?;
            if c > end {
                continue;
            }
            let start = u16_at(&self.data, start_base + i * 2).ok()?;
            if c < start {
                return None;
            }
            let delta = i16_at(&self.data, delta_base + i * 2).ok()?;
            let range_off = u16_at(&self.data, range_base + i * 2).ok()?;
            if range_off == 0 {
                return Some((c.wrapping_add(delta as u16)) as u16);
            }
            // glyphIdArray 起点 = range_base + i*2 + rangeOffset + (c - start)*2
            let addr = range_base + i * 2 + range_off as usize + (c - start) as usize * 2;
            let g = u16_at(&self.data, addr).ok()?;
            if g == 0 {
                return None;
            }
            return Some((g.wrapping_add(delta as u16)) as u16);
        }
        None
    }

    fn cmap6(&self, sub: usize, cp: u32) -> Option<u16> {
        let first = u16_at(&self.data, sub + 6).ok()? as u32;
        let count = u16_at(&self.data, sub + 8).ok()? as u32;
        if cp < first || cp >= first + count {
            return None;
        }
        u16_at(&self.data, sub + 10 + (cp - first) as usize * 2).ok()
    }

    fn cmap12(&self, sub: usize, cp: u32) -> Option<u16> {
        let n = u32_at(&self.data, sub + 12).ok()? as usize;
        // 二分查找（format 12 的组按 startCharCode 升序）
        let (mut lo, mut hi) = (0usize, n);
        while lo < hi {
            let mid = (lo + hi) / 2;
            let g = sub + 16 + mid * 12;
            let start = u32_at(&self.data, g).ok()?;
            let end = u32_at(&self.data, g + 4).ok()?;
            if cp < start {
                hi = mid;
            } else if cp > end {
                lo = mid + 1;
            } else {
                let gid = u32_at(&self.data, g + 8).ok()?;
                let v = gid.checked_add(cp - start)?;
                return if v > 0xFFFF { None } else { Some(v as u16) };
            }
        }
        None
    }

    /// 取某个字形的轮廓与度量。未知字形索引返回错误。
    pub fn glyph(&self, glyph_index: u16) -> GpuResult<Glyph> {
        if glyph_index >= self.num_glyphs {
            return Err(GpuError::Unsupported(format!(
                "字形索引 {glyph_index} 超出范围（共 {}）",
                self.num_glyphs
            )));
        }
        let (glyf_off, _) = self.table(b"glyf")?;
        let (loca_off, loca_len) = self.table(b"loca")?;
        let (start, end) = self.loca_range(loca_off, glyph_index)?;
        if std::env::var("DEER_FONT_DEBUG").is_ok() {
            eprintln!(
                "[font] glyph({glyph_index}) glyf_off={glyf_off} loca_off={loca_off} loca_len={loca_len} start={start} end={end}"
            );
        }

        let (adv, lsb) = self.h_metrics(glyph_index)?;

        // end <= start ⇒ 空字形（没有轮廓）
        if end <= start {
            return Ok(Glyph {
                contours: Vec::new(),
                bbox: (0, 0, 0, 0),
                advance_width: adv,
                left_side_bearing: lsb,
            });
        }

        let g_off = glyf_off + start as usize;
        let n_contours = i16_at(&self.data, g_off)?;
        let x_min = i16_at(&self.data, g_off + 2)?;
        let y_min = i16_at(&self.data, g_off + 4)?;
        let x_max = i16_at(&self.data, g_off + 6)?;
        let y_max = i16_at(&self.data, g_off + 8)?;

        let mut contours = Vec::new();
        if n_contours >= 0 {
            contours = self.parse_simple_glyph(g_off, n_contours as usize)?;
        } else {
            // 复合字形：把各组件轮廓平移/缩放后拼接
            self.parse_composite_glyph(g_off, 0, &mut contours)?;
        }

        Ok(Glyph {
            contours,
            bbox: (x_min, y_min, x_max, y_max),
            advance_width: adv,
            left_side_bearing: lsb,
        })
    }

    /// `loca` → 该字形的 `(start, end)`（相对 `glyf` 的字节偏移）。
    fn loca_range(&self, loca_off: usize, glyph_index: u16) -> GpuResult<(u32, u32)> {
        let i = glyph_index as usize;
        if self.index_to_loc_format == 0 {
            // 短格式：偏移 / 2
            let a = u16_at(&self.data, loca_off + i * 2)? as u32;
            let b = u16_at(&self.data, loca_off + (i + 1) * 2)? as u32;
            Ok((a * 2, b * 2))
        } else {
            let a = u32_at(&self.data, loca_off + i * 4)?;
            let b = u32_at(&self.data, loca_off + (i + 1) * 4)?;
            Ok((a, b))
        }
    }

    /// `hmtx` → `(advance_width, left_side_bearing)`。
    ///
    /// 注意 `hmtx` 的紧凑规则：只有前 `num_h_metrics` 个字形有独立的 advance，
    /// 之后的字形**复用最后一个** advance（只有 lsb 继续列出）。
    fn h_metrics(&self, glyph_index: u16) -> GpuResult<(u16, i16)> {
        let (hmtx_off, hmtx_len) = self.table(b"hmtx")?;
        if std::env::var("DEER_FONT_DEBUG").is_ok() {
            eprintln!(
                "[font] hmtx_off={hmtx_off} hmtx_len={hmtx_len} num_h_metrics={} glyph_index={glyph_index} data_len={}",
                self.num_h_metrics,
                self.data.len()
            );
        }
        let n = self.num_h_metrics.max(1) as u32;
        let gi = glyph_index as u32;
        let adv_idx = gi.min(n - 1);
        let adv = u16_at(&self.data, hmtx_off + adv_idx as usize * 4)?;
        // lsb：前 n 个字形在 hmtx 的前 4n 字节里；之后的字形接在 4n 之后，每项 2 字节
        let lsb = if gi < n {
            i16_at(&self.data, hmtx_off + gi as usize * 4 + 2)?
        } else {
            let extra = (gi - n) as usize * 2;
            i16_at(&self.data, hmtx_off + n as usize * 4 + extra)?
        };
        Ok((adv, lsb))
    }

    /// 解析简单字形：轮廓端点数组 + 标志 + 坐标增量。
    fn parse_simple_glyph(&self, g_off: usize, n_contours: usize) -> GpuResult<Vec<Contour>> {
        let mut p = g_off + 10;
        let mut ends = Vec::with_capacity(n_contours);
        for _ in 0..n_contours {
            ends.push(u16_at(&self.data, p)? as usize);
            p += 2;
        }
        let n_points = ends.last().map(|e| e + 1).unwrap_or(0);
        let ins_len = u16_at(&self.data, p)? as usize;
        p += 2 + ins_len; // 跳过 instructions（我们不做 hinting）

        // 标志（带重复压缩）
        let mut flags = Vec::with_capacity(n_points);
        while flags.len() < n_points {
            let f = u8_at(&self.data, p)?;
            p += 1;
            flags.push(f);
            if f & 0x08 != 0 {
                // REPEAT
                let rep = u8_at(&self.data, p)?;
                p += 1;
                for _ in 0..rep {
                    if flags.len() >= n_points {
                        break;
                    }
                    flags.push(f);
                }
            }
        }

        // x 坐标增量
        let mut xs = Vec::with_capacity(n_points);
        let mut x = 0i32;
        for &f in &flags {
            let dx = if f & 0x02 != 0 {
                // X_SHORT_VECTOR
                let v = u8_at(&self.data, p)? as i32;
                p += 1;
                if f & 0x10 != 0 { v } else { -v }
            } else if f & 0x10 != 0 {
                0 // 同前一点
            } else {
                let v = i16_at(&self.data, p)? as i32;
                p += 2;
                v
            };
            x += dx;
            xs.push(x as i16);
        }

        // y 坐标增量
        let mut ys = Vec::with_capacity(n_points);
        let mut y = 0i32;
        for &f in &flags {
            let dy = if f & 0x04 != 0 {
                let v = u8_at(&self.data, p)? as i32;
                p += 1;
                if f & 0x20 != 0 { v } else { -v }
            } else if f & 0x20 != 0 {
                0
            } else {
                let v = i16_at(&self.data, p)? as i32;
                p += 2;
                v
            };
            y += dy;
            ys.push(y as i16);
        }

        let points: Vec<Point> = (0..n_points)
            .map(|i| Point {
                x: xs[i] as f32,
                y: ys[i] as f32,
                on_curve: flags[i] & 0x01 != 0,
            })
            .collect();

        // 按轮廓端点切分并展平
        let mut contours = Vec::with_capacity(n_contours);
        let mut start = 0usize;
        for &e in &ends {
            if e >= points.len() {
                return Err(GpuError::Unsupported(format!(
                    "轮廓端点 {e} 越界（共 {} 点）",
                    points.len()
                )));
            }
            let sub = &points[start..=e];
            if !sub.is_empty() {
                contours.push(flatten_contour(sub));
            }
            start = e + 1;
        }
        Ok(contours)
    }

    /// 解析复合字形：递归取出各组件轮廓并施加变换。
    fn parse_composite_glyph(
        &self,
        g_off: usize,
        depth: u32,
        out: &mut Vec<Contour>,
    ) -> GpuResult<()> {
        if depth > 8 {
            return Err(GpuError::Unsupported(
                "复合字形嵌套超过 8 层（字体可能损坏）".to_string(),
            ));
        }
        let (glyf_off, _) = self.table(b"glyf")?;
        let (loca_off, _) = self.table(b"loca")?;

        let mut p = g_off + 10;
        loop {
            let flags = u16_at(&self.data, p)?;
            let comp_index = u16_at(&self.data, p + 2)?;
            p += 4;

            let args_are_xy = flags & 0x0002 != 0;
            let (dx, dy);
            if flags & 0x0001 != 0 {
                // ARG_1_AND_2_ARE_WORDS
                if args_are_xy {
                    dx = i16_at(&self.data, p)? as f32;
                    dy = i16_at(&self.data, p + 2)? as f32;
                } else {
                    // 点匹配模式：我们用 0 偏移兜住（不支持精确点对齐）
                    dx = 0.0;
                    dy = 0.0;
                }
                p += 4;
            } else if args_are_xy {
                dx = u8_at(&self.data, p)? as i8 as f32;
                dy = u8_at(&self.data, p + 1)? as i8 as f32;
                p += 2;
            } else {
                dx = 0.0;
                dy = 0.0;
                p += 2;
            }

            // 可选变换。规范定义了三种编码，尺寸不同：
            //   0x0008 WE_HAVE_A_SCALE      ⇒ 2 个 F2Dot14（a 与 d，b=c=0）
            //   0x0040 WE_HAVE_AN_X_AND_Y_SCALE ⇒ 2 个 F2Dot14（同 0x0008，语义别名）
            //   0x0080 WE_HAVE_A_TWO_BY_TWO ⇒ 4 个 F2Dot14（a b c d）
            let mut a = 1.0f32;
            let mut b = 0.0f32;
            let mut c = 0.0f32;
            let mut d = 1.0f32;
            if flags & 0x0008 != 0 || flags & 0x0040 != 0 {
                a = f2dot14(i16_at(&self.data, p)?);
                d = f2dot14(i16_at(&self.data, p + 2)?);
                p += 4;
            } else if flags & 0x0080 != 0 {
                a = f2dot14(i16_at(&self.data, p)?);
                b = f2dot14(i16_at(&self.data, p + 2)?);
                c = f2dot14(i16_at(&self.data, p + 4)?);
                d = f2dot14(i16_at(&self.data, p + 6)?);
                p += 8;
            }

            // 取出组件轮廓（递归，支持嵌套复合）
            let (cs, ce) = self.loca_range(loca_off, comp_index)?;
            if ce > cs {
                let sub_off = glyf_off + cs as usize;
                let sub_n = i16_at(&self.data, sub_off)?;
                let mut sub: Vec<Contour> = Vec::new();
                if sub_n >= 0 {
                    sub = self.parse_simple_glyph(sub_off, sub_n as usize)?;
                } else {
                    self.parse_composite_glyph(sub_off, depth + 1, &mut sub)?;
                }
                // 应用变换 + 平移
                for ctn in sub {
                    out.push(transform_contour(&ctn, a, b, c, d, dx, dy));
                }
            }

            if flags & 0x0020 == 0 {
                // 没有 MORE_COMPONENTS ⇒ 结束
                break;
            }
        }
        Ok(())
    }

    /// 行高（font units）：`ascender - descender + line_gap`。
    pub fn line_height_units(&self) -> f32 {
        (self.ascender as f32 - self.descender as f32) + self.line_gap as f32
    }
}

/// F2Dot14 → f32（复合字形变换矩阵用的定点格式）。
fn f2dot14(v: i16) -> f32 {
    v as f32 / 16384.0
}

/// 对轮廓施加 2×2 线性变换 + 平移。
fn transform_contour(
    c: &Contour,
    a: f32,
    b: f32,
    cc: f32,
    d: f32,
    dx: f32,
    dy: f32,
) -> Contour {
    let tf = |x: f32, y: f32| (a * x + cc * y + dx, b * x + d * y + dy);
    Contour {
        start: tf(c.start.0, c.start.1),
        segments: c
            .segments
            .iter()
            .map(|s| match *s {
                Segment::Line { to } => Segment::Line { to: tf(to.0, to.1) },
                Segment::Quad { ctrl, to } => Segment::Quad {
                    ctrl: tf(ctrl.0, ctrl.1),
                    to: tf(to.0, to.1),
                },
                Segment::Cubic { c1, c2, to } => Segment::Cubic {
                    c1: tf(c1.0, c1.1),
                    c2: tf(c2.0, c2.1),
                    to: tf(to.0, to.1),
                },
            })
            .collect(),
    }
}

/// 把一个轮廓的点列表展平成 [`Contour`]，消掉 TrueType 的「隐含中点」。
///
/// 规则（规范原文）：轮廓首尾相接；连续两个**非**on-curve 点之间隐含一个
/// on-curve 中点；若首点是非 on-curve，起点取「末点与首点的中点」。
fn flatten_contour(pts: &[Point]) -> Contour {
    // 单点轮廓：退化成一个点（面积 0）
    if pts.len() == 1 {
        return Contour {
            start: (pts[0].x, pts[0].y),
            segments: Vec::new(),
        };
    }
    let n = pts.len();
    // 起点
    let start = if pts[0].on_curve {
        (pts[0].x, pts[0].y)
    } else if pts[n - 1].on_curve {
        (pts[n - 1].x, pts[n - 1].y)
    } else {
        (
            (pts[0].x + pts[n - 1].x) / 2.0,
            (pts[0].y + pts[n - 1].y) / 2.0,
        )
    };

    let mut segments = Vec::new();
    let mut cur = start;
    let mut i = if pts[0].on_curve { 1 } else { 0 };
    while i < n {
        let p = pts[i];
        if p.on_curve {
            // 直线（若与当前点重合则跳过，避免零长段）
            if (p.x, p.y) != cur {
                segments.push(Segment::Line { to: (p.x, p.y) });
                cur = (p.x, p.y);
            }
            i += 1;
        } else {
            // 非 on-curve ⇒ 二次贝塞尔；终点是下一个 on-curve 点，
            // 或（若下一个也不是 on-curve）两者中点
            let ctrl = (p.x, p.y);
            let next = if i + 1 < n { pts[i + 1] } else { pts[0] };
            let to = if next.on_curve {
                (next.x, next.y)
            } else {
                ((p.x + next.x) / 2.0, (p.y + next.y) / 2.0)
            };
            segments.push(Segment::Quad { ctrl, to });
            cur = to;
            i += if next.on_curve { 2 } else { 1 };
        }
    }
    // 闭合回起点
    if cur != start {
        segments.push(Segment::Line { to: start });
    }
    Contour { start, segments }
}
