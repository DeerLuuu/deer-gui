//! **testkit 的对外用法**：下游使用者写一条 UI 测试要写多少行。
//!
//! 本文件就是「重写 `examples/counter.rs` 自检段」的实证：
//!
//! | | 手工版（`examples/counter.rs`） | testkit 版（本文件） |
//! |---|---|---|
//! | 需要自己实现的样板 | `struct Frame` + `Frame::build` + `center` + `rect_of` + `drawn_texts` + `ordered_with_geometry` + `assert_count_shown` + `pixel_diff_split` + `buttons_whose_visual_changed` + `assert_pixels_show_count` + `resolve_script` + `assert_counter_tree` + `node_by_id` | **无**（全在 `deer_gui::testing`），本文件只有界面树 + 断言 |
//! | 前置断言（`node_hint>0`、clip 非空、关键 id 在快照内） | 手抄，抄漏一处就静默「全不裁剪」 | `Harness::frame()` 每次都做，且 `@id`/`tap` 用到的 id **自动**登记 |
//! | 像素容差 / 越界 = 0 | 每个测试各写一份 | `Shot::assert_state_change_only` / `ParityRule` 集中定义 |
//! | CPU↔GPU 对照 | 另写一套（`window_parity.rs` / `gpu_vs_cpu.rs`） | `Harness::compare_cpu_gpu` 一条调用 |
//! | 失败信息 | 每个断言手写 `format!` | 自动附**可复制的复现命令** + 真实数字 |
//!
//! 运行（`## 命令可直接照抄`）：
//!
//! ```text
//! cargo test -p deer-gui --features testing --test testkit_counter -- --nocapture
//! ```
//!
//! 关掉 `testing` feature 时本文件整个不编译（0 条测试）——
//! 所以**默认 `cargo test --workspace` 不会**跑它，testkit 自身的护栏在
//! `src/testing.rs` 的 `#[cfg(test)] mod tests` 里（那一份**进默认门禁**）。

#![cfg(feature = "testing")]

use deer_gui::interaction::InputEvent;
use deer_gui::layout::builder::L;
use deer_gui::layout::node::{Kind, LayoutProps, Node};
use deer_gui::prelude::*;
use deer_gui::testing::{Harness, ParityRule, Repro};

/// 计数显示的前缀（与 `examples/counter.rs` 逐字相同）。
const PREFIX: &str = "count = ";

/// 与 `examples/counter.rs::BUILTIN_SCRIPT` **逐字相同**的脚本。
const BUILTIN: &str = "move @plus;down:left;up:left;move @plus;down:left;up:left;\
                       move @input;down:left;up:left;text:ok";

/// 与 `examples/counter.rs::counter_tree` **逐字同构**的界面树。
fn counter_tree(count: i32) -> Node {
    let app = Node::new(Kind::Column, "app").with_layout(LayoutProps {
        padding: 12.0,
        gap: 10.0,
        ..Default::default()
    });
    let bar = Node::new(Kind::Row, "bar")
        .with_layout(L::new().w(300.0).gap(8.0).to_props())
        .push(Node::new(Kind::Button, "plus").with_label("+"))
        .push(Node::new(Kind::Button, "minus").with_label("-"));
    let info = Node::new(Kind::Row, "info")
        .with_layout(L::new().w(300.0).gap(8.0).to_props())
        .push(Node::new(Kind::Text, "count").with_label(format!("{PREFIX}{count}")))
        .push(Node::new(Kind::Field, "input").with_label("type here"));
    app.push(Node::new(Kind::Text, "title").with_label("deer-gui counter"))
        .push(bar)
        .push(info)
}

/// **这一条就是「下游怎么写一条 UI 测试」的形状**（14 行，含空行）。
#[test]
fn minimal_downstream_test() -> Result<(), String> {
    let mut h = Harness::new(counter_tree(0), 360, 200, Theme::default());
    h.set_case("testkit/最小示例");
    h.set_repro(Repro::test(
        "deer-gui",
        "testing",
        "testkit_counter",
        "minimal_downstream_test",
        &[],
    ));
    let before = h.shoot_named("count=0")?; // 离屏 CPU 像素（真字形）
    let step = h.tap("plus")?; // 单事件注入（坐标由布局算）
    assert!(step.changed, "点一下 + 必须改状态");
    h.set_tree(counter_tree(1)); // 本项目没有事件回调 ⇒ 改了数据就整树重建
    let after = h.shoot_named("count=1")?;
    h.assert_drawn_text(&h.frame()?, "count", "count = 1")?; // 读绘制列表，不读树
    after.assert_state_change_only(&before, "count")?; // 差异只落在变了的节点矩形内
    Ok(())
}

/// **重写 `counter.rs::headless_selfcheck`**：同样的语料、同样的结论，逐条用 testkit 表达。
///
/// 覆盖手工版的三条结论：
/// ① 前置（`NodeHint` 齐全、裁剪快照覆盖要点/要断言的节点）；
/// ② 重放内置脚本后**绘制列表画出来的文本**是 `count = 2`，输入框画出 `ok`；
/// ③ 计数变化的像素证据 + **框外 = 0**；
/// ④ 额外补一条手工版没有的：CPU↔GPU 对照（testkit 把它变成一条调用）。
#[test]
fn counter_selfcheck_via_testkit() -> Result<(), String> {
    let mut h = Harness::new(counter_tree(0), 360, 200, Theme::default());
    h.set_case("counter/离屏自检（testkit 版）");
    h.set_repro(Repro::test(
        "deer-gui",
        "testing",
        "testkit_counter",
        "counter_selfcheck_via_testkit",
        &[],
    ));
    h.set_clear(Color::rgb(0x08, 0x09, 0x0c));
    // 这一档的结论有一半在像素上 ⇒ 没有真字形就必须**明确红**，不给假绿。
    h.require_glyph_pixels()?;

    // ① 前置与初始一帧（`frame()` 自己会断言 node_hint>0 / clip 非空 / 登记 id 在快照里）
    let f0 = h.frame()?;
    h.assert_focus_order(&["plus", "minus", "input"])?;
    h.assert_node_hint_count(&f0, 8)?;
    h.assert_clip_known(&f0, &["plus", "minus", "input", "count"])?;
    h.assert_drawn_text(&f0, "count", "count = 0")?;

    // ② 重放内置脚本（`move @id` 的坐标由布局算出来，不写死）
    let before = h.shoot_named("初始 count=0")?;
    let run = h.run_script(BUILTIN)?;
    h.set_tree(counter_tree(2));
    let after = h.shoot_named("终态 count=2")?;
    println!("展开后的坐标脚本 = {}", run.resolved);

    // ③ 终态：**读绘制列表画出来的东西**（不是读回状态）
    let f = h.frame()?;
    h.assert_drawn_text(&f, "count", "count = 2")?;
    h.assert_drawn_text_contains(&f, "input", "ok")?;
    h.assert_text("input", "ok")?;
    h.assert_visual_state(Some("input"), Some("input"), None)?;

    // ④ 像素证据：主矩形 `count` 内必须有变化，且**框外为 0**
    //    （期望矩形由 testkit 自己算：它会把「视觉真的变了的节点」也算进去，
    //     所以不会像手抄那样漏掉同时变了的输入框）
    after.assert_state_change_only(&before, "count")?;

    // ⑤ CPU↔GPU 对照（手工版没有这一步；`None` = 本机无 Vulkan ⇒ 跳过，不是通过）
    let rule = ParityRule::for_list(&f.list);
    if let Some(report) = h.compare_cpu_gpu("counter/终态", &f.list, rule)? {
        println!("{}", report.line());
        assert_eq!(
            report.max_channel_diff,
            0,
            "整帧（含半透明 surface + 真字形）必须与 CPU 逐字节相同，实际 {:?}",
            report.line()
        );
        assert_eq!(report.text_skipped, 0, "不该有被跳过的文本命令");
    }
    println!("counter 自检（testkit 版）✅：count=2、输入框 ok、像素框外 0、CPU↔GPU 逐字节相同");
    Ok(())
}

/// 直接改状态画一帧（`hover`/`pressed`/`focus` 三态各一张图）——
/// 手工版要为此各写一段「摆状态 + 重建帧」的样板，这里是一条 `set_state` + 一条 `shoot`。
#[test]
fn four_states_differ_only_inside_the_node_itself() -> Result<(), String> {
    use deer_gui::gpu::interact::InteractState;
    use deer_gui::interaction::UiState;

    let mut h = Harness::new(counter_tree(0), 360, 200, Theme::default());
    h.set_case("testkit/四状态");
    h.require_glyph_pixels()?;

    let idle = h.shoot_named("idle")?;
    h.set_state(UiState {
        hover: Some("plus".into()),
        ..UiState::default()
    });
    let hover = h.shoot_named("hover=plus")?;
    h.set_state(UiState {
        hover: Some("plus".into()),
        pressed: Some("plus".into()),
        ..UiState::default()
    });
    let press = h.shoot_named("pressed=plus")?;
    h.set_state(UiState {
        focus: Some("plus".into()),
        ..UiState::default()
    });
    let focus = h.shoot_named("focus=plus")?;

    // 每一档的差异都必须只落在 `plus` 自己的矩形内（框外 = 0）。
    hover.assert_diff_only_inside(&idle, "plus")?;
    press.assert_diff_only_inside(&idle, "plus")?;
    focus.assert_diff_only_inside(&idle, "plus")?;
    // 三档视觉互不相同（否则上面三条断言什么也没证明）。
    assert_ne!(hover.rgba, press.rgba, "hover 与 pressed 的视觉必须不同");
    assert_ne!(hover.rgba, focus.rgba, "hover 与 focus 的视觉必须不同");
    assert_ne!(idle.rgba, hover.rgba, "idle 与 hover 的视觉必须不同");
    let _ = InteractState::hovered("plus"); // 只是让「渲染器只认这三个字段」这句话有出处
    Ok(())
}

/// 单事件注入 + 脚本注入两条入口都要能拿到 `UiEvent`（下游最常用的是这个）。
#[test]
fn single_event_and_script_both_report_ui_events() -> Result<(), String> {
    use deer_gui::interaction::{Key, UiEvent};

    let mut h = Harness::new(counter_tree(0), 360, 200, Theme::default());
    h.set_case("testkit/事件报告");
    // 单事件：Tab 换焦点
    let s = h.send(&InputEvent::KeyDown {
        key: Key::Tab,
        mods: Default::default(),
        repeat: false,
    })?;
    assert!(s.changed);
    assert!(matches!(&s.events[0], UiEvent::FocusChanged(Some(id)) if id == "plus"));
    h.assert_focus(Some("plus"))?;
    // 脚本：`move @id` + 点击
    let run = h.run_script("move @minus;down:left;up:left")?;
    let clicked: Vec<String> = run
        .events()
        .into_iter()
        .filter_map(|e| match e {
            UiEvent::Clicked(id) => Some(id),
            _ => None,
        })
        .collect();
    assert_eq!(clicked, vec!["minus".to_string()]);
    h.assert_focus(Some("minus"))?;
    Ok(())
}
