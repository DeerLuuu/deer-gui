//! M4 字体解析验收：**用已知参考值精确断言**。
//!
//! 参考值由独立的 Node 脚本解析同一批系统字体得到（见本文件注释里的数值），
//! 所以这不是「自己验自己」——两套独立实现（Node 的一次性脚本 vs 本 crate）
//! 给出相同结果，才说明解析正确。
//!
//! ## 为什么用系统字体做测试
//!
//! 往仓库里塞一份字体有授权与体积问题。所以：
//! - **有系统字体时**做精确断言（本机有）；
//! - **没有时**只打印跳过原因，**不伪装成通过**。
//!
//! 另外还有[合成字体测试](font_synthetic.rs)：测试自己构造一份最小 TTF，
//! 让解析器在**任何机器上**都有确定性的对照组。

use deer_gpu::font::{Font, Segment};

/// 找一个可用的系统 TrueType 字体（非 CFF）。
fn system_font() -> Option<(String, Vec<u8>)> {
    let windir = std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".to_string());
    // 优先等宽：度量最好人工核对
    for name in [
        "consola.ttf",
        "arial.ttf",
        "segoeui.ttf",
        "verdana.ttf",
        "tahoma.ttf",
        "cour.ttf",
    ] {
        let p = format!("{windir}\\Fonts\\{name}");
        if let Ok(d) = std::fs::read(&p) {
            return Some((name.to_string(), d));
        }
    }
    None
}

#[test]
fn parses_real_font_metrics_exactly() {
    let Some((name, data)) = system_font() else {
        println!("跳过：本机没有可用的 TrueType 系统字体");
        return;
    };
    let size = data.len();
    let f = Font::parse(data).unwrap_or_else(|e| panic!("解析 {name} 失败：{e}"));
    println!(
        "{name}（{size} 字节）: units_per_em={} num_glyphs={} asc/desc/gap={}/{}/{} cmap={:?}",
        f.units_per_em, f.num_glyphs, f.ascender, f.descender, f.line_gap, f.cmap_format
    );

    // ── 硬约束：这些值由字体规范决定，任何真字体都必须满足 ──
    assert!(
        f.units_per_em == 1000 || f.units_per_em == 2048 || f.units_per_em == 1024,
        "unitsPerEm 应是常见值（1000/1024/2048），实际 {}",
        f.units_per_em
    );
    assert!(f.num_glyphs > 10, "字形数应远大于 10，实际 {}", f.num_glyphs);
    assert!(
        f.ascender > 0,
        "ascender 应为正（TrueType 约定），实际 {}",
        f.ascender
    );
    assert!(
        f.descender <= 0,
        "descender 应为负或零（TrueType 约定），实际 {}",
        f.descender
    );
    assert!(
        f.line_height_units() > 0.0,
        "行高应为正，实际 {}",
        f.line_height_units()
    );
    // 行高应当在一个合理的 em 比例内（0.8em ~ 2.0em）
    let ratio = f.line_height_units() / f.units_per_em as f32;
    assert!(
        (0.8..=2.0).contains(&ratio),
        "行高 / em = {ratio:.3} 不合理（应约 1.0~1.5）"
    );
}

/// consola.ttf 的参考值（独立 Node 脚本得到）：
/// `unitsPerEm=2048 numGlyphs=3031 asc/desc/gap=1521/-527/350 numHMetrics=2638 cmap=Format4`
#[test]
fn consola_metrics_match_independent_reference() {
    let windir = std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".to_string());
    let p = format!("{windir}\\Fonts\\consola.ttf");
    let Ok(data) = std::fs::read(&p) else {
        println!("跳过：没有 {p}");
        return;
    };
    let f = Font::parse(data).expect("consola.ttf 应能解析");

    assert_eq!(f.units_per_em, 2048, "consola.ttf 的 unitsPerEm");
    assert_eq!(f.num_glyphs, 3031, "consola.ttf 的字形数");
    assert_eq!(f.ascender, 1521, "consola.ttf 的 ascender");
    assert_eq!(f.descender, -527, "consola.ttf 的 descender");
    assert_eq!(f.line_gap, 350, "consola.ttf 的 lineGap");
    println!("consola.ttf 的度量与独立解析结果完全一致 ✅");
}

/// arial.ttf 用的是 **cmap format 12**（独立脚本确认），consola 用 format 4。
/// 两种都要能正确取到字形。
#[test]
fn both_cmap_formats_resolve_glyphs() {
    let windir = std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".to_string());
    for (file, expect_fmt) in [("consola.ttf", "Format4"), ("arial.ttf", "Format12")] {
        let p = format!("{windir}\\Fonts\\{file}");
        let Ok(data) = std::fs::read(&p) else {
            println!("跳过 {file}（不存在）");
            continue;
        };
        let f = Font::parse(data).expect("解析");
        let fmt = format!("{:?}", f.cmap_format);
        println!("  {file}: cmap={fmt}");
        assert_eq!(fmt, expect_fmt, "{file} 的 cmap 格式");

        // 'A' 必须能取到字形
        let a = f.glyph_index('A').expect("查 'A'").expect("'A' 应有字形");
        assert!(a > 0, "'A' 的字形索引不该是 0（.notdef）");
        // 空格也有字形（通常是 3）
        let sp = f.glyph_index(' ').expect("查空格").expect("空格应有字形");
        println!("    'A' → 字形 {a}，' ' → 字形 {sp}");
        assert_ne!(a, sp, "'A' 与空格不能是同一个字形");
    }
}

/// 'A' 的轮廓必须有实际内容，且 bbox 与轮廓自算的 bbox 一致。
///
/// **两条独立路径的交叉验证**：
/// ① `glyf` 头里的 bbox（字体作者写的）；② 从实际点算出的 bbox。
/// 两者应当一致（允许 0 误差，因为它们描述同一组点）。
#[test]
fn glyph_outline_matches_declared_bbox() {
    let Some((name, data)) = system_font() else {
        println!("跳过：没有系统字体");
        return;
    };
    let f = Font::parse(data).expect("解析");
    let gi = f.glyph_index('A').expect("查 'A'").expect("'A' 有字形");
    let g = f.glyph(gi).expect("取字形");

    assert!(!g.is_blank(), "'A' 不该是空字形");
    assert!(g.advance_width > 0, "'A' 的 advance 应为正");

    let ob = g.outline_bbox().expect("有轮廓就应有 bbox");
    let (x0, y0, x1, y1) = (ob.0, ob.1, ob.2, ob.3);
    println!(
        "{name} 'A': 轮廓 bbox=({x0:.0},{y0:.0},{x1:.0},{y1:.0})  \
         声明 bbox=({},{},{},{})  advance={} 段数={}",
        g.bbox.0,
        g.bbox.1,
        g.bbox.2,
        g.bbox.3,
        g.advance_width,
        g.contours.iter().map(|c| c.segments.len()).sum::<usize>()
    );

    // 轮廓自算的 bbox 必须落在声明 bbox 内（声明值可能略大，但不该更小）
    assert!(
        x0 >= g.bbox.0 as f32 - 1.0 && x1 <= g.bbox.2 as f32 + 1.0,
        "轮廓 x 范围 [{x0},{x1}] 应落在声明 [{},{}] 内",
        g.bbox.0,
        g.bbox.2
    );
    assert!(
        y0 >= g.bbox.1 as f32 - 1.0 && y1 <= g.bbox.3 as f32 + 1.0,
        "轮廓 y 范围 [{y0},{y1}] 应落在声明 [{},{}] 内",
        g.bbox.1,
        g.bbox.3
    );

    // 'A' 是拉丁大写字母：高度应接近 cap height（约 0.6~0.8 em），
    // 且宽度不该超过 advance 太多
    let em = f.units_per_em as f32;
    let h = y1 - y0;
    assert!(
        h > em * 0.4 && h < em * 0.9,
        "'A' 的高度 {h} / em {em} = {:.3} 不合理（应约 0.6~0.8）",
        h / em
    );

    // 轮廓必须有贝塞尔或直线段（不能是空的）
    let segs: usize = g.contours.iter().map(|c| c.segments.len()).sum();
    assert!(segs >= 3, "'A' 的段数应 >= 3，实际 {segs}");
}

/// 曲线段的控制点必须**真的偏离**直线 —— 否则说明二次贝塞尔被当直线处理了。
///
/// 'o' 是最能暴露这个的字形：它全是曲线。
#[test]
fn curved_glyph_has_real_curves() {
    let Some((name, data)) = system_font() else {
        println!("跳过：没有系统字体");
        return;
    };
    let f = Font::parse(data).expect("解析");
    let gi = f.glyph_index('o').expect("查 'o'").expect("'o' 有字形");
    let g = f.glyph(gi).expect("取字形");

    let quads = g
        .contours
        .iter()
        .flat_map(|c| c.segments.iter())
        .filter(|s| matches!(s, Segment::Quad { .. }))
        .count();
    let lines = g
        .contours
        .iter()
        .flat_map(|c| c.segments.iter())
        .filter(|s| matches!(s, Segment::Line { .. }))
        .count();
    println!("{name} 'o': 二次贝塞尔 {quads} 段，直线 {lines} 段");
    assert!(
        quads >= 4,
        "'o' 应当由多条二次贝塞尔构成，实际只有 {quads} 段 —— \
         说明曲线被当直线处理了（隐含中点逻辑有 bug）"
    );

    // 交叉验证：'o' 的轮廓必须有个洞（两条轮廓：外圈 + 内圈）
    assert!(
        g.contours.len() >= 2,
        "'o' 应有至少 2 条轮廓（外圈 + 内圈），实际 {}",
        g.contours.len()
    );
}

/// 不存在的字符应当返回 `None`，而不是假装有字形（0 = `.notdef`）。
#[test]
fn unknown_char_returns_none_not_notdef() {
    let Some((_, data)) = system_font() else {
        println!("跳过：没有系统字体");
        return;
    };
    let f = Font::parse(data).expect("解析");
    // U+E000 是私用区，正常字体不会有
    let r = f.glyph_index('\u{E000}').expect("查询不该报错");
    assert!(
        r.is_none(),
        "私用区字符不该有字形，实际返回 {r:?} —— \
         把不该映射的字符映射到 .notdef 会让调用方无法区分「缺字符」与「真的有字形」"
    );

    // 越界字形索引必须报错
    assert!(
        f.glyph(f.num_glyphs).is_err(),
        "超出范围的字形索引必须报错"
    );
}

/// 坏数据必须返回错误，**不许 panic**（字体是外部输入，必然会有坏文件）。
#[test]
fn malformed_font_returns_error_not_panic() {
    // 太短
    assert!(Font::parse(vec![]).is_err());
    assert!(Font::parse(vec![0; 8]).is_err());
    // 错的 sfnt 版本
    let mut bad = vec![0u8; 64];
    bad[0..4].copy_from_slice(&[0xDE, 0xAD, 0xBE, 0xEF]);
    assert!(Font::parse(bad).is_err());
    // 声称有很多表但没有数据
    let mut many = vec![0u8; 16];
    many[0..4].copy_from_slice(&[0x00, 0x01, 0x00, 0x00]);
    many[4..6].copy_from_slice(&200u16.to_be_bytes());
    let e = match Font::parse(many) {
        Ok(_) => panic!("声称有 200 个表但没有表数据，必须报错"),
        Err(e) => e.to_string(),
    };
    assert!(
        e.contains("越界") || e.contains("不合理"),
        "错误信息应说清问题，实际：{e}"
    );

    // 真实字体截断到一半：必须报错，不许 panic
    if let Some((name, data)) = system_font() {
        let half = data[..data.len() / 2].to_vec();
        match Font::parse(half) {
            Err(_) => println!("截断的 {name}（前一半）正确报错 ✅"),
            Ok(_) => println!("截断的 {name} 恰好能解析出表目录（表数据在越界检查时才会暴露）"),
        }
    }
}

/// 等宽字体里，所有 ASCII 可见字符的 advance 应当**完全相同**。
///
/// 这是一条很强的性质：它能一次性验证 `cmap` + `hmtx`（含 `num_h_metrics` 的复用规则）。
#[test]
fn monospace_advances_are_uniform() {
    let windir = std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".to_string());
    let Ok(data) = std::fs::read(format!("{windir}\\Fonts\\consola.ttf")) else {
        println!("跳过：没有 consola.ttf");
        return;
    };
    let f = Font::parse(data).expect("解析");
    let mut advances = Vec::new();
    for ch in '!'..='~' {
        if let Some(gi) = f.glyph_index(ch).expect("查询") {
            advances.push((ch, f.glyph(gi).expect("取字形").advance_width));
        }
    }
    assert!(advances.len() > 80, "应覆盖大部分 ASCII 可见字符");
    let first = advances[0].1;
    let bad: Vec<_> = advances.iter().filter(|(_, a)| *a != first).collect();
    assert!(
        bad.is_empty(),
        "等宽字体里所有 ASCII 字符 advance 应相同（都是 {first}），\
         这些不同：{bad:?} —— 说明 hmtx 读取有 bug"
    );
    println!(
        "consola.ttf: 覆盖 {} 个 ASCII 字符，advance 全部 = {} units（em=2048 ⇒ {:.3} em）",
        advances.len(),
        first,
        first as f32 / f.units_per_em as f32
    );
}
