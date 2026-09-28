//! **环境变量门槛**判定（`DEER_VK_WINDOW_TESTS` / `DEER_WINDOW_HOLD` / `DEER_WINDOW_READBACK` 这类开关）。
//!
//! # 为什么这份判定要住在库里，而不是各 example 里各写一份
//!
//! 1. **判据只有一处**才有意义：一处分叉就会出现「同一个命令，一个示例认为设了门槛、
//!    另一个认为没设」，而这个错位的方向恰好是**静默**的（跳过也算 pass）。
//! 2. `cargo test` **不跑** example 里的 `#[test]`（只编译它们）⇒ 判定写在 example 里
//!    就永远没有护栏。放在这里，才有 `cargo test -p deer-gui --lib env_gate` 守着。
//!
//! # 为什么必须**先 `trim()` 再比**（实测，不是猜的）
//!
//! `cmd` 的 `set DEER_VK_WINDOW_TESTS=1 && cargo run …` 会把 `&&` 前的空格也算进变量值 ——
//! `cmd /c "set X=1 && set X"` 实测打印 `X=1 `（**带一个尾空格**）；写成 `set X=1&& …`
//! （不留空格）或 `set "X=1" && …`（带引号）才是不带空格的 `1`。
//!
//! 严格 `== "1"` 会把「**已经设了**门槛」判成「没设」⇒ 明明建了窗口、跑完了全程、
//! `exit=0`，却打印「这不是通过，是被跳过」，**与事实相反**；反向（`=0` 关不掉）
//! 同样会静默生效。所以本模块先 `trim()` 再比，断言见下方单测（含 `"1 "` / `" 1 "` 两档）。

/// 环境变量 `name` 的**真值**判定：`"1"` / `"true"`（大小写不敏感、两侧空白先去掉）⇒ `true`；
/// 未设、空值或其它值 ⇒ `false`。
///
/// 门槛的「有没有设」用法：`env_gate::flag("DEER_VK_WINDOW_TESTS")`。
pub fn flag(name: &str) -> bool {
    std::env::var(name).map(|v| truthy(&v)).unwrap_or(false)
}

/// [`flag`] 的**纯逻辑**部分（不读环境 ⇒ 可单测）。空白容忍的理由见模块文档。
pub fn truthy(v: &str) -> bool {
    let v = v.trim();
    v == "1" || v.eq_ignore_ascii_case("true")
}

/// 默认**开**、可用 `=0`/`false` 关掉的开关（例如 `DEER_WINDOW_READBACK`）：
/// 未设 ⇒ `true`；`"0"` / `"false"`（大小写不敏感、两侧空白先去掉）⇒ `false`。
pub fn flag_default_true(name: &str) -> bool {
    std::env::var(name).map(|v| !falsey(&v)).unwrap_or(true)
}

/// [`flag_default_true`] 的**纯逻辑**部分（不读环境 ⇒ 可单测）。
///
/// `falsey` 只认 `"0"` / `"false"`（两侧空白先去掉）：`"0 "` 必须真的关掉开关 ——
/// 否则「显式关掉」会因尾空格被当成「没关」。理由与 [`truthy`] 同一条（见模块文档）。
pub fn falsey(v: &str) -> bool {
    let v = v.trim();
    v == "0" || v.eq_ignore_ascii_case("false")
}

#[cfg(test)]
mod tests {
    use super::{falsey, truthy};

    /// **护栏**：`cmd /c "set X=1 && …"` 的实际值是 `"1 "`（带尾空格），
    /// 必须与带引号的 `set "X=1" && …`（`"1"`）**同判** —— 否则「设了门槛」会被判成
    /// 「没设」⇒ 用例静默跳过却报 pass。
    #[test]
    fn truthy_tolerates_surrounding_whitespace() {
        for v in [
            "1", "1 ", " 1", " 1 ", "\t1\t", "true", "TRUE", "True", " true ",
        ] {
            assert!(truthy(v), "{v:?} 必须判为「开」");
        }
        for v in [
            "", " ", "\t", "0", "0 ", "false", "FALSE", "yes", "on", "2", "1 0",
        ] {
            assert!(!truthy(v), "{v:?} 必须判为「关」");
        }
    }

    /// 反向开关（默认开）同样要容忍尾空格：`DEER_WINDOW_READBACK=0 ` 必须**真的**关掉回读。
    #[test]
    fn falsey_tolerates_surrounding_whitespace() {
        for v in ["0", "0 ", " 0 ", "\t0", "false", "FALSE", " False "] {
            assert!(falsey(v), "{v:?} 必须判为「关」");
        }
        for v in ["", " ", "1", "1 ", "true", "TRUE", "yes"] {
            assert!(!falsey(v), "{v:?} 不是「关」");
        }
    }

    /// 实证数据（见模块文档）：这两档就是 `set X=1 && …` / `set "X=1" && …` 的实际值。
    #[test]
    fn cmd_trailing_space_form_is_the_enabled_one() {
        assert_eq!(
            truthy("1 "),
            truthy("1"),
            "`set X=1 && …`（值 `\"1 \"`）必须与 `set \"X=1\" && …`（值 `\"1\"`）同判"
        );
    }
}
