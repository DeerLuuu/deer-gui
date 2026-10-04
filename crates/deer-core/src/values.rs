//! **值解析**（M6 5d）：数值与 hex 颜色的**唯一**解析/格式化实现。
//!
//! # 为什么住在 `deer-core`
//!
//! 同一句「这个串算不算数」有**两个**消费者：
//!
//! 1. `deer-gui` 的交互层 —— `NumberField`/`ColorField` **提交时**解析（发不发事件）；
//!    `ScrubNum` **按下时**解析 label（拖动基准值）；
//! 2. `deer-gpu` 的绘制层 —— `NumberField` 的「不可解析」下划线、`ColorField`
//!    的色块颜色，都要**当场再判一次**（画的是草稿串的当前状态，判据必须与
//!    交互层逐字相同，否则「界面说非法、事件却发出去了」）。
//!
//! `deer-gpu` 不能反向依赖 `deer-gui`（与 `InteractState` 同一个理由），而 `deer-core`
//! 是两边共同的 L0 —— 所以解析只有这一份实现，两边都来这里调用。
//! 各写一份 = 「什么算合法数字」出现第二个真相，必然漂。

/// 解析一个**数值**（`NumberField` 提交 / `ScrubNum` 基准的唯一切入点）。
///
/// 规则（最小正确版，逐条写明）：
///
/// - 先 `trim()` 两侧空白（「 3」是 3 —— 输入框里的手滑空格不该判死整个值）；
/// - 主体交给 [`f64::from_str`]（接受 `3` / `-1.5` / `1e3` / `+7` / `inf`）；
/// - **`NaN` 判为失败**：它不是值域里的一个点（任何比较都是 false），夹取与
///   「变了才发」的判等对它全部失效 —— 与其让一个幽灵值流进状态，不如按
///   「不可解析」处理（不发事件 + 标记）；
/// - 空串（或全是空白）判为失败 —— 但「空草稿」在语义上是「还没输入」，
///   **不画**不可解析标记（这是绘制层的职责，见 `deer-gpu/src/interact.rs`）。
pub fn parse_num(s: &str) -> Option<f64> {
    let t = s.trim();
    if t.is_empty() {
        return None;
    }
    t.parse::<f64>().ok().filter(|v| !v.is_nan())
}

/// 数值的**规范化显示串**（提交成功后回写 `texts` 的格式）。
///
/// 用 [`std::fmt::Display`] 的默认浮点格式（`7.0 ⇒ "7"`、`2.5 ⇒ "2.5"`）——
/// 刻意不写千分位/固定小数位：那属于格式化配置，本轮没有承载位（登记为开放问题）。
/// `inf` 按 Rust 默认打成 `inf`，能被 [`parse_num`] 原样读回（往返一致）。
pub fn format_num(v: f64) -> String {
    format!("{v}")
}

/// 解析一个 **hex 颜色**（`ColorField` 提交的唯一切入点）。
///
/// 规则：`#RRGGBB`（6 位 hex，大小写都可以）；**`#` 可省**（输入框里敲 `ff8800`
/// 与 `#ff8800` 等价）。`#RGB` 短式 / `#RGBA` / `rgb()` 函数式**不做**（最小版，
/// 见 color-field.md「做不到什么」）。
pub fn parse_hex_color(s: &str) -> Option<[u8; 3]> {
    let t = s.trim().strip_prefix('#').unwrap_or(s.trim());
    if t.len() != 6 || !t.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let byte = |i: usize| u8::from_str_radix(&t[i..i + 2], 16).ok();
    Some([byte(0)?, byte(2)?, byte(4)?])
}

/// hex 颜色的**规范化显示串**（提交成功后回写 `texts` 的格式）：
/// `#rrggbb` 小写带 `#` —— 同一个颜色永远打出同一个串。
pub fn format_hex_color(rgb: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", rgb[0], rgb[1], rgb[2])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_num_accepts_the_documented_grammar() {
        // 前置判据：文档承诺的每一条都要真的成立。
        assert_eq!(parse_num("3"), Some(3.0));
        assert_eq!(parse_num("  -1.5 "), Some(-1.5), "两侧空白要 trim");
        assert_eq!(parse_num("1e3"), Some(1000.0), "科学计数法按 f64::from_str");
        assert_eq!(parse_num("+7"), Some(7.0));
        assert_eq!(parse_num("inf"), Some(f64::INFINITY), "inf 是合法值（可再读回）");
        // 失败档。
        assert_eq!(parse_num(""), None, "空串 = 还没输入");
        assert_eq!(parse_num("   "), None);
        assert_eq!(parse_num("abc"), None);
        assert_eq!(parse_num("3px"), None, "带单位不是数字（不做单位解析）");
        assert_eq!(parse_num("3.4.5"), None);
        assert!(
            parse_num("NaN").is_none(),
            "NaN 判为失败：它不是值域里的点，夹取/判等对它全部失效"
        );
    }

    #[test]
    fn parse_num_and_format_num_round_trip() {
        for s in ["7", "2.5", "-0.25", "1e3", "inf"] {
            let v = parse_num(s).expect("前置：样例必须都能解析");
            let back = format_num(v);
            assert_eq!(
                parse_num(&back),
                Some(v),
                "format_num 的产出必须能被 parse_num 原样读回（{s} → {back}）"
            );
        }
        assert_eq!(format_num(7.0), "7", "Display 默认格式不带尾巴 .0");
    }

    #[test]
    fn parse_hex_color_accepts_only_rrggbb() {
        assert_eq!(parse_hex_color("#ff8800"), Some([255, 136, 0]));
        assert_eq!(parse_hex_color("#FF8800"), Some([255, 136, 0]), "大小写都行");
        assert_eq!(parse_hex_color("ff8800"), Some([255, 136, 0]), "# 可省");
        assert_eq!(parse_hex_color(" #000000 "), Some([0, 0, 0]));
        // 失败档：长度不对 / 非法字符 / 其它语法 —— 一律 None（最小版不猜意图）。
        assert_eq!(parse_hex_color("#fff"), None, "#RGB 短式不做");
        assert_eq!(parse_hex_color("#ff8800ff"), None, "#RRGGBBAA 不做");
        assert_eq!(parse_hex_color("rgb(255,0,0)"), None, "函数式不做");
        assert_eq!(parse_hex_color("#ff88gg"), None, "非 hex 字符");
        assert_eq!(parse_hex_color(""), None);
        assert_eq!(parse_hex_color("#"), None);
    }

    #[test]
    fn format_hex_color_is_lowercase_with_hash_and_round_trips() {
        assert_eq!(format_hex_color([255, 136, 0]), "#ff8800");
        assert_eq!(format_hex_color([0, 0, 0]), "#000000");
        for s in ["#FF8800", "ff8800", "#000000"] {
            let rgb = parse_hex_color(s).expect("前置：样例必须能解析");
            let canon = format_hex_color(rgb);
            assert_eq!(
                parse_hex_color(&canon),
                Some(rgb),
                "规范化串必须能被原样读回（{s} → {canon}）"
            );
            assert!(
                canon == format_hex_color(parse_hex_color(&canon).unwrap()),
                "规范化必须是幂等的（{canon}）"
            );
        }
    }
}
