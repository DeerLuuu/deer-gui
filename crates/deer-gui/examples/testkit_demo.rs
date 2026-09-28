//! **testkit 演示**：用 `deer_gui::testing` 写一条 UI 测试要写多少行。
//!
//! ```sh
//! cargo run -p deer-gui --features testing --example testkit_demo
//! ```
//!
//! 它做的事，正是原来每个 example 各抄一遍的那套样板（树 → 几何 → 绘制列表 →
//! 裁剪快照 → 离屏像素 → 断言），只是现在**全在库里**，而且**默认带上本项目全部纪律**：
//!
//! 1. **前置断言**由 [`Harness::frame`] 每次做（`node_hint > 0`、裁剪快照非空、
//!    用到的 id 有几何且在快照里）—— 抄漏一处就会**静默「全不裁剪」**，这里不可能漏；
//! 2. **像素判据**：主矩形内必须有变化（否则「它没画出来」），**框外必须为 0**；
//! 3. **CPU↔GPU 对照**一条调用给出「不透明逐字节 0 / 半透明 ≤1 LSB」的结论
//!    （本示例用默认主题 ⇒ `surface` 半透明 ⇒ 按 ≤1 LSB 判）；
//! 4. **失败信息**自带可直接复制的复现命令与真实数字（见 [`Repro`]）。
//!
//! 找不到系统字体时**明确降级**（打印 `TESTKIT-FONT` 说明）—— 那时文本类像素断言
//! 会失去意义，[`Harness::require_glyph_pixels`] 会**明确报错**而不是给你一条假红。

use std::process::ExitCode;

use deer_gui::prelude::*;
use deer_gui::testing::{Harness, ParityRule, Repro};

fn main() -> ExitCode {
    match run() {
        Ok(()) => {
            println!("[testkit_demo] 通过 ✅");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("[testkit_demo] 失败：{e}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    // ① 建树（`Builder` 直接交给 `Harness`；`Node` 也可以）
    let mut app = Builder::new(Kind::Column, "app").padding(10.0).gap(8.0);
    let plus = app.button("+"); // 返回自动生成的 id（形如 `button_1`）
    app.text("testkit demo");

    // ② 建面：真字体度量 + 离屏 CPU 像素（找不到字体就打印降级说明）
    let mut h = Harness::new(app, 220, 120, Theme::default());
    h.set_case("testkit_demo");
    h.set_repro(Repro::example("deer-gui", "testing", "testkit_demo", "", &[]));
    h.require_ids(&[plus.as_str()]);
    // 这一档的结论有一半在像素上 ⇒ 没有真字形就**明确报错**（不给假红也不给假绿）
    h.require_glyph_pixels()?;

    // ③ 一帧 + 内置前置断言（`node_hint > 0`、裁剪快照非空、`plus` 在快照里）
    let f = h.frame()?;
    println!(
        "一帧：{} 条命令；NodeHint {} 条；裁剪快照 {} 个节点；有几何的节点 {}",
        f.list.len(),
        f.counts().node_hint,
        f.clip.len(),
        f.ordered_with_geometry().len()
    );
    h.assert_focus_order(&[plus.as_str()])?; // 焦点树序（禁用节点不在内）
    h.assert_drawn_text(&f, "text_1", "testkit demo")?; // 读绘制列表，不读树

    // ④ 像素：点一下按钮，差异必须只落在它自己的矩形内
    let before = h.shoot_named("点击前")?;
    let step = h.tap(&plus)?;
    if !step.changed {
        return Err(format!("点 `{plus}` 必须改状态，实际 events={:?}", step.events));
    }
    let after = h.shoot_named("点击后")?;
    after.assert_state_change_only(&before, &plus)?;

    // ⑤ CPU↔GPU 对照（本机没有可用 Vulkan 时打印「这是跳过，不是通过」并返回 `None`）
    let rule = ParityRule::for_list(&f.list);
    match h.compare_cpu_gpu("testkit_demo/点击后", &f.list, rule)? {
        Some(report) => println!("{}", report.line()),
        None => println!("（GPU 档跳过 —— 上面那行 TESTKIT-SKIP 就是证据）"),
    }
    Ok(())
}
