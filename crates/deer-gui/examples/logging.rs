//! **日志系统**（`deer-log`）：默认**完全静默**，要开就设 `DEER_LOG`。
//!
//! 跑法：
//! ```text
//! cargo run -p deer-gui --example logging
//! ```
//! 本示例**不读环境变量**（过滤器是纯逻辑，构造出来直接问）—— 所以它的输出与你的环境无关。
//! 真实的开关是进程级 `DEER_LOG`（首次使用时读一次并缓存），见
//! [`docs/features/logging.md`](../../../docs/features/logging.md)。

use deer_log::{Filter, Level};

fn main() {
    println!("=== 日志系统（deer-log）===\n");

    println!("① 级别（越靠后越严重；开某个级别 ⇒ 允许**所有 ≥ 它**的级别）：");
    for lv in Level::ALL {
        println!("  {:<6} 标签 {:<6}", format!("{lv:?}"), lv.label());
    }

    println!("\n② 过滤器是**纯逻辑**（与真实用法同一份实现）：\n");
    println!("  {:<26} trace/debug/info 分别能不能输出", "DEER_LOG");
    println!("  {}", "-".repeat(70));
    for spec in [
        "",
        "off",
        "debug",
        "deer_vk=trace",
        "info,deer_vk=trace",
        "deer_vk=error,deer_vk=trace",
    ] {
        let f = Filter::parse(spec);
        let row: Vec<String> = ["deer_gui", "deer_vk"]
            .iter()
            .map(|t| {
                let s = |lv: Level| if f.allows(lv, t) { "Y" } else { "-" };
                format!(
                    "{t}: {}{}{}",
                    s(Level::Trace),
                    s(Level::Debug),
                    s(Level::Info)
                )
            })
            .collect();
        let label = if spec.is_empty() {
            "(未设)".to_string()
        } else {
            format!("`{spec}`")
        };
        println!("  {label:<26} {}", row.join("  |  "));
    }

    // ─────────────────────────────────────────────────────────────────────
    // 自检：跑成功不等于做对了
    // ─────────────────────────────────────────────────────────────────────
    println!("\n=== 自检 ===");

    // ① **默认静默** —— 这是「不许污染既有 stderr 判据」的判据
    let off = Filter::parse("");
    assert!(off.is_off(), "不设 DEER_LOG 必须全关");
    for lv in Level::ALL {
        assert!(!off.allows(lv, "any::target"), "{lv:?} 不该被允许");
    }
    println!("  ① 不设 DEER_LOG ⇒ 任何级别、任何 target 都不输出 ✅");

    // ② 开一个 target 不会顺带开别的
    let only_vk = Filter::parse("deer_vk=trace");
    assert!(only_vk.allows(Level::Trace, "deer_vk"));
    assert!(
        !only_vk.allows(Level::Error, "deer_gui"),
        "别的 target 必须还是关的"
    );
    println!("  ② `deer_vk=trace` 只开 deer_vk，别的仍静默 ✅");

    // ③ 阈值语义：开 debug ⇒ error/info/debug 都开，trace 仍然关
    let dbg = Filter::parse("deer_gui=debug");
    assert!(dbg.allows(Level::Error, "deer_gui") && dbg.allows(Level::Debug, "deer_gui"));
    assert!(!dbg.allows(Level::Trace, "deer_gui"), "debug 不该把 trace 也开了");
    println!("  ③ 开 debug ⇒ error/info/debug 开、trace 关（单调）✅");

    // ④ 后写的覆盖先写的
    let last = Filter::parse("deer_vk=error,deer_vk=trace");
    assert!(last.allows(Level::Trace, "deer_vk"), "后写的规则应当生效");
    println!("  ④ 同 target 后写的规则覆盖先写的 ✅");

    // ⑤ 认不出的配置**不会把日志意外打开**
    assert!(Filter::parse("nonsense").is_off(), "认不出的项应当忽略");
    assert!(
        Filter::parse("deer_vk=nonsense").is_off(),
        "认不出的级别应当按关处理"
    );
    println!("  ⑤ 配置写错 ⇒ 仍然静默（不会意外开口）✅");

    println!("\n全部自检通过。真实用法：");
    println!("  DEER_LOG=deer_vk=trace cargo run -p deer-gui --example scroll_bar");
    println!("日志只写 **stderr** ⇒ `> out.txt` 抓到的产物不会被污染。");
}
