//! # deer-log
//!
//! **零依赖、默认完全静默**的日志门面 —— 给「快速调试 / 定位错误」用。
//!
//! ## 为什么不引 `log` / `tracing` / `env_logger`
//!
//! 仓库纪律：**除窗口层（`winit`，已登记）外零第三方依赖**；新增依赖要先走
//! `ROADMAP.md` 的依赖例外登记并获人同意。日志是最底层的东西，**给它引一棵传递依赖树
//! 是反的**（而且 `log` 只管门面、`env_logger` 只管后端，要凑齐两层还得引两个）。
//! 这里要的能力很有限：分级 + 按 target 过滤 + 写 stderr。自研约 200 行，可控。
//!
//! ## 默认**完全静默**（这一条是硬要求，不是偏好）
//!
//! 不设 `DEER_LOG` ⇒ 所有级别都关 ⇒ **一个字节都不输出**。理由是本仓库有多处
//! **按 stderr 判据**的测试与工具（校验层回调打印 `[VK ERROR]`/`[VALIDATION]` 前缀、
//! 示例的「跳过」提示），日志一旦默认开口就会污染它们 —— 那属于「为了加功能把既有判据弄坏」。
//! 所以：**要日志必须显式开**，与滚动条的 opt-in 是同一条纪律。
//!
//! ## 开关：`DEER_LOG`
//!
//! ```text
//! DEER_LOG=off                     # 默认（等价于不设）
//! DEER_LOG=debug                   # 全局默认级别
//! DEER_LOG=deer_vk=trace           # 只开某个 target（其余关）
//! DEER_LOG=info,deer_vk=trace      # 默认 info + deer_vk 更详细
//! ```
//!
//! 规则与直觉一致：**逗号分隔**，每一项是 `级别` 或 `target=级别`；
//! **后写的规则覆盖先写的**（同名 target 以后者为准）；解析不了的那一项**忽略**（不致命）。
//!
//! ## 怎么用
//!
//! ```no_run
//! // 热路径：先问一句再拼字符串（关着的时候几乎零成本）
//! if deer_log::enabled(deer_log::Level::Trace, "deer_vk") {
//!     deer_log::trace_at("deer_vk", format_args!("draw 命令数 = {}", 3));
//! }
//!
//! // 或者用宏（`target` 自动取 `module_path!()`）
//! deer_log::trace!("draw 命令数 = {}", 3);
//! ```
//!
//! **注意**：`enabled()` 第一次调用时会读一次环境变量并**缓存**（`OnceLock`）。
//! 之后改环境变量不再生效 —— 这对「运行期几百万次调用」是必要的代价，
//! 也让行为在**一次进程内**是确定的（与布局的确定性纪律同源）。

use std::fmt;
use std::sync::OnceLock;

/// 日志级别（由轻到重）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    /// 最啰嗦：逐帧、逐命令级别。
    Trace,
    /// 调试：关键分支、状态迁移。
    Debug,
    /// 常规信息：启动、设备选择、一次性的决策。
    Info,
    /// 可恢复的问题（有回退路径）。
    Warn,
    /// 出错（有 `Result` 但被吞掉/降级的地方）。
    Error,
}

impl Level {
    /// 全部级别（由轻到重），供遍历/测试。
    pub const ALL: [Level; 5] = [
        Level::Trace,
        Level::Debug,
        Level::Info,
        Level::Warn,
        Level::Error,
    ];

    /// 解析级别名（大小写不敏感、两侧空白去掉）。`off` **不是**级别 ⇒ `None`。
    pub fn parse(s: &str) -> Option<Level> {
        Some(match s.trim().to_ascii_lowercase().as_str() {
            "trace" => Level::Trace,
            "debug" => Level::Debug,
            "info" => Level::Info,
            "warn" | "warning" => Level::Warn,
            "error" => Level::Error,
            _ => return None,
        })
    }

    /// 打印用的短标签（定长 5 字符，便于对齐）。
    pub fn label(self) -> &'static str {
        match self {
            Level::Trace => "TRACE",
            Level::Debug => "DEBUG",
            Level::Info => "INFO ",
            Level::Warn => "WARN ",
            Level::Error => "ERROR",
        }
    }
}

impl fmt::Display for Level {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label().trim_end())
    }
}

/// 一条 `target=级别` 规则。
#[derive(Debug, Clone, PartialEq, Eq)]
struct Rule {
    target: String,
    level: Option<Level>,
}

/// 过滤规则：**默认关**（这是「默认静默」的实现）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Filter {
    /// 没被任何 target 规则命中的目标用什么级别（`None` = 关）。
    default_level: Option<Level>,
    /// 按 target 的规则；**后面的覆盖前面的**。
    rules: Vec<Rule>,
}

impl Filter {
    /// 什么都不开（与「不设 `DEER_LOG`」等价）。
    pub fn off() -> Filter {
        Filter::default()
    }

    /// 解析 `DEER_LOG` 的语法。**解析不了的项忽略**（不致命）—— 日志配置写错
    /// 不该让程序起不来。
    ///
    /// 语法：逗号分隔；每项是 `级别` 或 `target=级别`；`off`/`none` 表示关掉该项。
    pub fn parse(spec: &str) -> Filter {
        let mut f = Filter::off();
        for item in spec.split(',') {
            let item = item.trim();
            if item.is_empty() {
                continue;
            }
            match item.split_once('=') {
                // `target=级别`
                Some((target, lv)) => {
                    let target = target.trim();
                    if target.is_empty() {
                        continue;
                    }
                    let level = parse_level_or_off(lv);
                    // 后写的覆盖先写的：同 target 的旧规则直接删掉
                    f.rules.retain(|r| r.target != target);
                    f.rules.push(Rule {
                        target: target.to_string(),
                        level,
                    });
                }
                // 裸级别 ⇒ 默认级别
                None => {
                    f.default_level = parse_level_or_off(item);
                }
            }
        }
        f
    }

    /// 该 target 在该级别下是否要输出。**没有任何规则命中 ⇒ false**（默认静默）。
    pub fn allows(&self, level: Level, target: &str) -> bool {
        // 后写的覆盖先写的 ⇒ 从后往前找第一条命中的
        let hit = self
            .rules
            .iter()
            .rev()
            .find(|r| target_matches(&r.target, target))
            .map(|r| r.level)
            .unwrap_or(self.default_level);
        match hit {
            // `Level` 的序是「越靠后越严重」（Trace 最小）⇒ 「阈值 = debug」意味着
            // **允许所有 ≥ debug 的级别**（debug/info/warn/error），而 trace 被挡掉。
            Some(threshold) => level >= threshold,
            None => false,
        }
    }

    /// 是不是「什么都不输出」。热路径可以据此走短路。
    pub fn is_off(&self) -> bool {
        self.default_level.is_none() && self.rules.iter().all(|r| r.level.is_none())
    }
}

/// `off` / `none` ⇒ `None`（关）；认不出 ⇒ `None`（**并且**这一项会被调用方忽略）。
fn parse_level_or_off(s: &str) -> Option<Level> {
    let t = s.trim().to_ascii_lowercase();
    if t == "off" || t == "none" {
        return None;
    }
    Level::parse(&t)
}

/// target 匹配：**前缀匹配**，且必须落在 `::` 边界上。
///
/// 于是 `DEER_LOG=deer_gui=debug` 会同时命中 `deer_gui`、`deer_gui::interaction`，
/// 但**不会**命中 `deer_guide`（边界判断，避免前缀误伤）。
fn target_matches(rule: &str, target: &str) -> bool {
    if rule == target {
        return true;
    }
    target
        .strip_prefix(rule)
        .is_some_and(|rest| rest.starts_with("::"))
}

/// 进程级过滤器（**首次 `enabled()` 时从环境读一次并缓存**）。
static FILTER: OnceLock<Filter> = OnceLock::new();

fn filter() -> &'static Filter {
    FILTER.get_or_init(|| {
        std::env::var("DEER_LOG")
            .map(|v| Filter::parse(&v))
            .unwrap_or_else(|_| Filter::off())
    })
}

/// 该级别在该 target 下要不要输出。**热路径请先问这个再拼字符串。**
pub fn enabled(level: Level, target: &str) -> bool {
    filter().allows(level, target)
}

/// 输出一条日志（一般不用直接调 —— 用下面的宏）。返回是否真的写了。
pub fn log_at(level: Level, target: &str, args: fmt::Arguments<'_>) -> bool {
    if !enabled(level, target) {
        return false;
    }
    // 只写 stderr：日志与「程序的正常产出」（stdout）必须分得开，
    // 这样 `cargo run --example x > out.txt` 抓到的产物不会被日志污染。
    eprintln!("[{level_label}] {target}: {args}", level_label = level.label());
    true
}

/// [`log_at`] 的便捷形式（供宏使用）。
pub fn trace_at(target: &str, args: fmt::Arguments<'_>) -> bool {
    log_at(Level::Trace, target, args)
}
/// 见 [`trace_at`]。
pub fn debug_at(target: &str, args: fmt::Arguments<'_>) -> bool {
    log_at(Level::Debug, target, args)
}
/// 见 [`trace_at`]。
pub fn info_at(target: &str, args: fmt::Arguments<'_>) -> bool {
    log_at(Level::Info, target, args)
}
/// 见 [`trace_at`]。
pub fn warn_at(target: &str, args: fmt::Arguments<'_>) -> bool {
    log_at(Level::Warn, target, args)
}
/// 见 [`trace_at`]。
pub fn error_at(target: &str, args: fmt::Arguments<'_>) -> bool {
    log_at(Level::Error, target, args)
}

/// `target` 自动取 `module_path!()`。
#[macro_export]
macro_rules! trace {
    ($($arg:tt)*) => { $crate::trace_at(module_path!(), format_args!($($arg)*)) };
}
/// 见 [`trace!`]。
#[macro_export]
macro_rules! debug {
    ($($arg:tt)*) => { $crate::debug_at(module_path!(), format_args!($($arg)*)) };
}
/// 见 [`trace!`]。
#[macro_export]
macro_rules! info {
    ($($arg:tt)*) => { $crate::info_at(module_path!(), format_args!($($arg)*)) };
}
/// 见 [`trace!`]。
#[macro_export]
macro_rules! warn {
    ($($arg:tt)*) => { $crate::warn_at(module_path!(), format_args!($($arg)*)) };
}
/// 见 [`trace!`]。
#[macro_export]
macro_rules! error {
    ($($arg:tt)*) => { $crate::error_at(module_path!(), format_args!($($arg)*)) };
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **默认静默**：什么都不开时，任何级别、任何 target 都不许输出。
    /// 这条是「不许污染既有 stderr 判据」的判据。
    #[test]
    fn off_filter_allows_nothing() {
        let f = Filter::off();
        assert!(f.is_off());
        for lv in Level::ALL {
            assert!(!f.allows(lv, "deer_vk"), "{lv:?} 不该被允许");
            assert!(!f.allows(lv, ""));
            assert!(!f.allows(lv, "anything::deep"));
        }
    }

    /// 空串与只有空白 ⇒ 仍然是「全关」（解析不出的项忽略，不该意外变成开机）。
    #[test]
    fn empty_spec_stays_off() {
        for spec in ["", "   ", ",", " , ,, "] {
            let f = Filter::parse(spec);
            assert!(f.is_off(), "`{spec:?}` 应当解析成全关，实际 {f:?}");
        }
        // 认不出的东西也不该开启任何东西
        assert!(Filter::parse("nonsense").is_off());
        assert!(Filter::parse("nonsense=1").is_off());
    }

    /// 裸级别 = 全局默认级别。
    #[test]
    fn bare_level_sets_the_default() {
        let f = Filter::parse("debug");
        assert!(f.allows(Level::Error, "any"));
        assert!(f.allows(Level::Debug, "any"));
        assert!(!f.allows(Level::Trace, "any"), "debug 不该把 trace 也开了");
    }

    /// 级别名大小写不敏感、两侧空白容忍（对齐 `env_gate::truthy` 的既有纪律）。
    #[test]
    fn level_names_are_case_and_whitespace_tolerant() {
        for s in ["warn", "WARN", " Warn ", "warning", "WARNING"] {
            assert_eq!(Level::parse(s), Some(Level::Warn), "`{s}` 应当解析成 WARN");
        }
        assert_eq!(Level::parse("off"), None, "off 不是级别");
        assert_eq!(Level::parse(""), None);
        assert_eq!(Level::parse("verbose"), None);
    }

    /// `target=级别` 只开那个 target（**其余保持关**）—— 这是「按需开一路」的核心。
    #[test]
    fn target_rule_does_not_open_others() {
        let f = Filter::parse("deer_vk=trace");
        assert!(f.allows(Level::Trace, "deer_vk"));
        assert!(!f.allows(Level::Error, "deer_gui"), "别的 target 必须还是关的");
    }

    /// target 匹配是**前缀 + `::` 边界**：命中子模块，但不误伤同前缀的别的名字。
    #[test]
    fn target_matching_respects_module_boundaries() {
        let f = Filter::parse("deer_gui=debug");
        assert!(f.allows(Level::Debug, "deer_gui"));
        assert!(f.allows(Level::Debug, "deer_gui::interaction"));
        assert!(
            !f.allows(Level::Debug, "deer_guide"),
            "`deer_guide` 不是 `deer_gui` 的子模块，不该被前缀误伤"
        );
    }

    /// **后写的规则覆盖先写的**（同 target）。
    #[test]
    fn later_rule_wins() {
        let f = Filter::parse("deer_vk=error,deer_vk=trace");
        assert!(f.allows(Level::Trace, "deer_vk"), "后写的 trace 应当生效");
        let f2 = Filter::parse("deer_vk=trace,deer_vk=off");
        assert!(!f2.allows(Level::Trace, "deer_vk"), "后写的 off 应当生效");
    }

    /// 默认级别 + 按 target 覆盖可以共存（最常见的用法）。
    #[test]
    fn default_plus_target_override() {
        let f = Filter::parse("info,deer_vk=trace");
        assert!(f.allows(Level::Info, "deer_gui"));
        assert!(!f.allows(Level::Debug, "deer_gui"), "默认是 info ⇒ debug 关");
        assert!(f.allows(Level::Trace, "deer_vk"), "deer_vk 被单独拉高");
        assert!(f.allows(Level::Error, "deer_vk"));
    }

    /// 级别是**单调**的：开了 debug ⇒ error/info 也开，但 trace 不开。
    #[test]
    fn level_filter_is_monotonic() {
        let f = Filter::parse("deer_vk=debug");
        assert!(f.allows(Level::Error, "deer_vk"));
        assert!(f.allows(Level::Warn, "deer_vk"));
        assert!(f.allows(Level::Info, "deer_vk"));
        assert!(f.allows(Level::Debug, "deer_vk"));
        assert!(!f.allows(Level::Trace, "deer_vk"));
    }

    /// `is_off` 与实际行为一致（它是热路径的短路依据，不能骗人）。
    #[test]
    fn is_off_agrees_with_allows() {
        for spec in ["", "off", "deer_vk=off", "x=off,y=off"] {
            let f = Filter::parse(spec);
            assert!(f.is_off(), "`{spec}` 应当是全关");
            assert!(!f.allows(Level::Error, "x"));
        }
        let f = Filter::parse("deer_vk=nonsense");
        assert!(f.is_off(), "认不出的级别应当按关处理（而不是意外开启）");
    }
}
