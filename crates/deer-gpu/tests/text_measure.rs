//! M4 **真实字体度量与换行**测试（t2）。
//!
//! 两条腿走路：
//! - **合成字体**（`tests/support/mod.rs`）给出任何机器上都确定的手选数字
//!   （`unitsPerEm = 1000`、`.notdef` = 500、空格 = 250、'A' = 800）；
//! - **系统字体**（`consola.ttf` 等）提供真实字形做真值对照 —— 找不到就
//!   **打印明确的跳过原因**，不伪装通过。
//!
//! 覆盖：① advance 双路径交叉验证 ② consola 等宽性与 0.6em 对比
//! ③ text_width = ceil(Σ advance) ④ wrap 不超宽/不丢字/行首无空白
//! ⑤ trait 实现与 inherent API 同源 ⑥ .notdef 回退 + 坏字体 0.5em 兜底。
//!
//! ## 变异测试记录（改坏 → 哪条断言变红 → 改回）
//!
//! | # | 变异 | 变红的断言（实测） |
//! |---|---|---|
//! | M4 | `text_width` 的 `.ceil()` → `.floor()` | ③「text_width 必须是 ceil（79），实测和是 79.2」（floor 会输出 79） |
//! | M5 | `advance` 第三级兜底 `0.5 * font_size` → `0.0` | ⑥b 坏字体兜底：`left: 0.0` vs `right: 50.0` |
//! | M6 | `wrap` 去掉「单词超宽时按字符硬切」 | ④「第 0 行超宽（240 > 100）且不是单字符硬切：\"AAA\"」（不丢字性质仍通过 —— 说明两条断言互补，缺一不可） |

mod support;

use deer_gpu::font::Font;
use deer_gpu::measure::{find_system_font, FontMeasure};
use deer_layout::layout::{ApproxMeasure, Measure, TextStyle};

/// 合成字体：`unitsPerEm = 1000`。
const UPEM: f32 = 1000.0;
/// 合成字体里 'A' 的 advance（font units）。
const A_ADVANCE: f32 = 800.0;
/// 合成字体里空格的 advance。
const SPACE_ADVANCE: f32 = 250.0;
/// 合成字体里 `.notdef`（glyph 0）的 advance。
const NOTDEF_ADVANCE: f32 = 500.0;

fn synthetic() -> Font {
    Font::parse(support::build_font()).expect("合成字体必须能解析")
}

fn synthetic_measure(font_size: f32) -> FontMeasure<'static> {
    // 注意：FontMeasure 借用 Font，这里为了让 helper 好写，把 Font 泄漏成 'static
    // （测试进程结束即回收，不会积累）。
    let font: &'static Font = Box::leak(Box::new(synthetic()));
    FontMeasure::new(font, font_size)
}

fn approx_f32(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-3
}

/// ① `advance(ch)` 与「cmap → glyph → hmtx」这条**独立路径**结果一致。
#[test]
fn advance_matches_hmtx_through_independent_path() {
    let font = synthetic();
    let m = FontMeasure::new(&font, 100.0);

    // 换算因子本身
    println!("scale = {}（font_size 100 / unitsPerEm 1000）", m.scale());
    assert!(approx_f32(m.scale(), 0.1), "scale 实测 {}", m.scale());
    assert!(approx_f32(m.ascent(), 80.0), "ascent 实测 {}", m.ascent());
    assert!(approx_f32(m.descent(), 20.0), "descent 实测 {}", m.descent());
    assert!(
        approx_f32(m.line_height(), 100.0),
        "line_height 实测 {}",
        m.line_height()
    );

    // 两条独立路径：一条走 FontMeasure::advance，另一条自己查 cmap → glyph → hmtx，
    // 并用 f64 重新算一遍比例（避免「同一个 f32 表达式抄两遍」的假交叉验证）。
    for ch in ['A', ' ', '\u{4E2D}'] {
        let mapped = font.glyph_index(ch).expect("查询 cmap");
        let (idx, path) = match mapped {
            Some(i) => (i, "cmap 命中"),
            None => (0u16, "cmap 未命中 → .notdef"),
        };
        let g = font.glyph(idx).expect("取字形");
        let expected = f64::from(g.advance_width) * (100.0f64 / f64::from(font.units_per_em));
        let got = f64::from(m.advance(ch));
        println!(
            "{ch:?}: {path} glyph={idx} hmtx.advance_width={} → 期望 {expected} px，实测 {got} px",
            g.advance_width
        );
        assert!(
            (got - expected).abs() < 1e-4,
            "{ch:?} 的 advance 与独立路径不一致：{got} vs {expected}"
        );
    }

    assert!(approx_f32(m.advance('A'), A_ADVANCE * 0.1), "手选值 80px");
    assert!(
        approx_f32(m.advance(' '), SPACE_ADVANCE * 0.1),
        "手选值 25px"
    );
}

/// ② consola 的 94 个可见 ASCII advance 全部相同，并给出与 `ApproxMeasure`（0.6em）的数字差异。
#[test]
fn consola_visible_ascii_is_monospace_with_approx_comparison() {
    let Some(path) = find_system_font() else {
        println!("【跳过】%WINDIR%\\Fonts 下没有 consola.ttf / arial.ttf / segoeui.ttf");
        return;
    };
    let data = std::fs::read(&path).expect("读系统字体文件");
    let font = Font::parse(data).expect("解析系统字体");
    println!(
        "系统字体：{}（unitsPerEm={}，字形数={}）",
        path.display(),
        font.units_per_em,
        font.num_glyphs
    );

    let size = 16.0f32;
    let m = FontMeasure::new(&font, size);
    let chars = support::visible_ascii();
    let advances: Vec<f32> = chars.iter().map(|&c| m.advance(c)).collect();
    assert_eq!(advances.len(), 94, "可见 ASCII 必须是 94 个（! 到 ~）");

    let is_consola = path
        .file_name()
        .map(|n| n.to_string_lossy().to_ascii_lowercase().contains("consola"))
        .unwrap_or(false);
    if is_consola {
        let first = advances[0];
        for (i, a) in advances.iter().enumerate() {
            assert!(
                approx_f32(*a, first),
                "consola 是等宽字体，但 {:?} 的 advance {} ≠ {}",
                chars[i],
                a,
                first
            );
        }
        println!("consola 等宽实测：94 个可见 ASCII 的 advance 全部 = {first} px");
    } else {
        println!(
            "【部分跳过】当前系统字体不是 consola（{}），不适用等宽断言；\
             其 'A' advance = {} px",
            path.file_name().unwrap().to_string_lossy(),
            m.advance('A')
        );
    }

    // ── 与 ApproxMeasure（每字符 0.6em）对比 ──
    let approx = ApproxMeasure;
    let style = TextStyle {
        font_size: size,
        line_height: 18.0,
    };
    let real_per_char = advances[0];
    let approx_per_char = 0.6 * size;
    println!(
        "单字符对比：真实 {real_per_char} px vs 0.6em 近似 {approx_per_char} px，差 {} px",
        real_per_char - approx_per_char
    );
    for n in [1usize, 10, 94] {
        let s: String = chars.iter().take(n).collect();
        let real = m.text_width(&s);
        let ap = approx.width(&s, style);
        println!("n={n:>2}: FontMeasure={real:>6} px，ApproxMeasure={ap:>6} px，差 {:>5}", real - ap);
        // 量级一致性（抓「忘了除 units_per_em」这类单位错误；0.6em 本身允许有偏差）
        assert!(
            (real - ap).abs() <= 0.1 * ap + 1.0,
            "n={n} 的真实度量与 0.6em 差得离谱（单位可能错了）：{real} vs {ap}"
        );
    }

    // 合成字体（'A' = 0.8em）上，近似值明显偏低 —— 证明这里测的是**真度量**。
    let synth = synthetic_measure(100.0);
    let real = synth.text_width("AA");
    let ap = approx.width(
        "AA",
        TextStyle {
            font_size: 100.0,
            line_height: 18.0,
        },
    );
    println!("合成字体 'AA'：真实 {real} px vs 0.6em 近似 {ap} px（差 {}）", real - ap);
    assert!(
        real > ap,
        "'A' 的真实 advance 是 0.8em，应当大于 0.6em 近似：{real} vs {ap}"
    );
}

/// ③ `text_width` = ceil(逐字符 advance 之和)；空串 = 0。
#[test]
fn text_width_is_ceil_of_advance_sum() {
    let m = synthetic_measure(100.0);
    assert_eq!(m.text_width(""), 0.0, "空串宽度必须是 0");
    assert_eq!(m.text_width("A"), 80.0);
    assert_eq!(m.text_width("A A"), 80.0 + 25.0 + 80.0);

    // 取一个逐字符和为**非整数**的字号，才能区分 ceil / floor / 不取整
    let m2 = synthetic_measure(33.0);
    let sample = "AAA";
    let raw: f32 = sample.chars().map(|c| m2.advance(c)).sum();
    let w = m2.text_width(sample);
    println!("font_size=33：逐字符和 = {raw}，text_width = {w}（floor 会是 79）");
    assert!(raw.fract() != 0.0, "样本必须是非整数和，实测 {raw}");
    assert!(w > raw, "text_width 必须是 ceil（{w}），实测和是 {raw}");
    assert_eq!(w, 80.0, "ceil(79.2) = 80");

    // 一般性质：任意字符串都等于其逐字符和向上取整
    for s in ["", "A", "AAA", "A A", "AAA A AAA"] {
        let sum: f32 = s.chars().map(|c| m.advance(c)).sum();
        assert_eq!(m.text_width(s), sum.ceil(), "{s:?} 的宽度不等于和取整");
    }
}

/// ④ `wrap`：每行 ≤ max_width（单字符硬切除外）、行首无空白、行数正确、**不丢字符**。
#[test]
fn wrap_respects_max_width_without_losing_characters() {
    let m = synthetic_measure(100.0); // 'A' = 80px，空格 = 25px

    // 手算的精确期望：font_size 100、max_width 250
    //   "AAA" = 240 ≤ 250；再接 " A" 会到 345 > 250 ⇒ 换行
    let lines = m.wrap("AAA A AAA", 250.0);
    println!("wrap(\"AAA A AAA\", 250) = {lines:?}");
    assert_eq!(lines, vec!["AAA".to_string(), "A".into(), "AAA".into()]);

    // 一般性质（多组句子 × 多个宽度）
    let cases = [
        ("AAA A AAA", 250.0f32),
        ("AAA A AAA", 100.0),
        ("A A A A A A", 120.0),
        ("AAAAA AA AAA", 200.0),
        ("  A", 250.0),
        ("A", 10.0),
        ("", 250.0),
        ("AAA\tA", 250.0),
    ];
    for (text, max_w) in cases {
        let lines = m.wrap(text, max_w);
        println!("wrap({text:?}, {max_w}) = {lines:?}");

        assert_eq!(m.wrap(text, max_w), lines, "wrap 必须确定性可重放");
        assert!(!lines.is_empty(), "至少要返回 1 行（空串也算 1 行）");

        for (i, line) in lines.iter().enumerate() {
            let w = m.text_width(line);
            if w > max_w {
                // 唯一例外：单个字符本身就超宽
                assert_eq!(
                    line.chars().count(),
                    1,
                    "第 {i} 行超宽（{w} > {max_w}）且不是单字符硬切：{line:?}"
                );
            }
            assert!(
                !line.starts_with(' ') && !line.starts_with('\t'),
                "行首不许有空白：{line:?}"
            );
            assert!(
                !line.ends_with(' ') && !line.ends_with('\t'),
                "行尾不许有空白：{line:?}"
            );
            assert!(!line.is_empty() || lines.len() == 1, "空行只允许出现在空串");
        }

        // **不丢字符**：去掉空白后，逐字符与原文相同
        let stripped_in: String = text.chars().filter(|c| !c.is_whitespace()).collect();
        let stripped_out: String = lines
            .concat()
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect();
        assert_eq!(stripped_out, stripped_in, "{text:?} 换行后丢了/多了字符");
    }

    // 空串 / 全空白 ⇒ 1 行（空行）
    assert_eq!(m.wrap("", 250.0), vec![String::new()]);
    assert_eq!(m.wrap("   \t ", 250.0), vec![String::new()], "全空白 ⇒ 一行空行");

    // 行首空白丢弃
    assert_eq!(m.wrap("   A", 250.0), vec!["A".to_string()]);

    // 单词超宽 ⇒ 按字符硬切
    assert_eq!(
        m.wrap("AA", 100.0),
        vec!["A".to_string(), "A".into()],
        "AA = 160 > 100 ⇒ 每行一个字符"
    );
    // 单字符仍超宽 ⇒ 允许该行超宽（否则无法前进）
    let tight = m.wrap("AA", 50.0);
    println!("wrap(\"AA\", 50) = {tight:?}（单字符 80 > 50，允许超宽）");
    assert_eq!(tight, vec!["A".to_string(), "A".into()]);
    assert!(m.text_width(&tight[0]) > 50.0, "这就是那条例外");

    // max_width <= 0 ⇒ 不换行，原样返回（含首尾空白）
    for bad in [0.0f32, -1.0] {
        assert_eq!(m.wrap("  AAA A  ", bad), vec!["  AAA A  ".to_string()]);
    }
}

/// ⑤ trait 实现与 inherent API **同源**（width/height 不是另一套算法）。
#[test]
fn measure_trait_matches_inherent_api() {
    let m = synthetic_measure(100.0);
    let style = TextStyle {
        font_size: 13.0, // 故意与 FontMeasure 的 font_size 不同
        line_height: 18.0,
    };

    for text in ["", "A", "AAA A AAA", "AAAAA AA AAA"] {
        // width == text_width（忽略 style.font_size：字号由构造时给定）
        assert_eq!(
            m.width(text, style),
            m.text_width(text),
            "{text:?} 的 Measure::width 与 FontMeasure::text_width 不一致"
        );
        for max_w in [0.0f32, 50.0, 100.0, 250.0, 10_000.0] {
            let lines = m.wrap(text, max_w);
            let expected = lines.len() as f32 * style.line_height;
            assert_eq!(
                m.height(text, style, max_w),
                expected,
                "{text:?} @ {max_w} 的 Measure::height 与 wrap().len()*line_height 不一致"
            );
        }
    }

    // 空串算 1 行
    assert_eq!(m.wrap("", 250.0).len(), 1);
    assert_eq!(m.height("", style, 250.0), 18.0);
    // 行高来自 style，不是字体自带行高（合成字体 line_height = 100px）
    assert!(approx_f32(m.line_height(), 100.0));
    assert_eq!(m.height("", style, 250.0), style.line_height);
    println!("trait 与 inherent API 完全一致（含空串 1 行、行高取自 style）");
}

/// ⑥ 未映射字符走 `.notdef` 回退（确定性），不 panic。
#[test]
fn unmapped_characters_fall_back_to_notdef() {
    let font = synthetic();
    let m = FontMeasure::new(&font, 100.0);

    assert_eq!(font.glyph_index('B').expect("查询 cmap"), None, "'B' 没有映射");
    let notdef = font.glyph(0).expect(".notdef 可读").advance_width;
    println!("合成字体 .notdef advance = {notdef} units → {} px", m.advance('B'));
    assert_eq!(notdef, NOTDEF_ADVANCE as u16);
    assert!(
        approx_f32(m.advance('B'), NOTDEF_ADVANCE * 0.1),
        "未映射字符必须用 .notdef 的 advance（50px），实测 {}",
        m.advance('B')
    );
    assert!(
        approx_f32(m.advance('\u{4E2D}'), m.advance('B')),
        "所有未映射字符共用同一个 .notdef 回退值（确定性）"
    );
    // 不是 0.6em 猜测，而是 hmtx 里的真实 .notdef 值
    assert!(!approx_f32(m.advance('B'), 0.6 * 100.0));

    // 回退路径下换行/度量都不 panic
    assert_eq!(m.text_width("BBB"), 150.0);
    assert_eq!(m.wrap("BBB", 100.0), vec!["BB".to_string(), "B".into()]);
    let style = TextStyle::default();
    assert_eq!(m.height("BBB", style, 100.0), 2.0 * style.line_height);
}

/// ⑥b 坏字体（`hmtx` 不可读）⇒ `0.5em` 兜底，**不 panic**。
///
/// 这是 `advance` 的第三条回退分支：合法字体上不可达，但坏字体不该让布局崩掉。
#[test]
fn broken_hmtx_falls_back_to_half_em_without_panic() {
    let data = support::build_font_with_unreadable_hmtx();
    let font = Font::parse(data).expect("表存在且边界合法 ⇒ parse 仍应成功");
    assert!(
        font.glyph(0).is_err() && font.glyph(2).is_err(),
        "hmtx 被指向文件末尾后，读字形度量必须报错（否则这个测试没有区分度）"
    );

    let m = FontMeasure::new(&font, 100.0);
    println!(
        "坏字体兜底实测：advance('A') = {}，advance(' ') = {}（0.5 × font_size = 50）",
        m.advance('A'),
        m.advance(' ')
    );
    assert_eq!(m.advance('A'), 0.5 * 100.0);
    assert_eq!(m.advance(' '), 0.5 * 100.0);
    // 即便度量全走了兜底，换行也必须能跑完
    assert_eq!(m.text_width("AAAA"), 200.0);
    assert!(!m.wrap("AAAA", 100.0).is_empty());
}

/// 系统字体探测：返回的路径必须真实存在（找不到时明确跳过）。
#[test]
fn system_font_probe_returns_existing_file() {
    match find_system_font() {
        Some(p) => {
            println!("find_system_font() = {}", p.display());
            assert!(p.is_file(), "返回的路径必须存在：{}", p.display());
            let name = p.file_name().unwrap().to_string_lossy().to_ascii_lowercase();
            assert!(
                ["consola.ttf", "arial.ttf", "segoeui.ttf"].contains(&name.as_str()),
                "优先级顺序里的字体名，实际 {name}"
            );
        }
        None => println!("【跳过】这台机器上三个候选字体都不存在"),
    }
}

/// `UPEM` 常量与字体实际 `units_per_em` 对齐（防止 support 里的字体被改坏）。
#[test]
fn synthetic_font_unitchoice_is_stable() {
    let font = synthetic();
    assert_eq!(f32::from(font.units_per_em), UPEM);
    assert_eq!(font.ascender, 800);
    assert_eq!(font.descender, -200);
    assert_eq!(font.num_glyphs, 3);
}
