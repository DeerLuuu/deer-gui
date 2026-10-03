//! **M6 5a 基础控件**：`Btn`（语义）/ `RowActions` / `Keep` —— 一个能自检的离屏示例。
//!
//! ```sh
//! cargo run -p deer-gui --features testing --example m6_basics
//! ```
//!
//! 产物：stdout 上的自证行（`[m6_basics] …`）+ **退出码 0 = 全部判据通过**。
//! 不开窗口、不写文件 —— 判据全部是「结构 / 绘制列表 / 注入输入后的状态」。
//!
//! 它演示并断言三件事（与本仓库 `tests/m6_basics.rs` 的判据同源）：
//!
//! 1. **Btn**：点击只落在捕获者身上（拖出树再抬起也算它的）、disabled 静默
//!    （无 `Clicked` / 无 `FocusChanged` / 抢不走焦点）、`Enter` = 点击；
//! 2. **RowActions**：`row_actions_opts` 造出来的树与「手写 `Row` + `button`」
//!    **逐条绘制命令相同**（组合层没有自己的渲染路径），返回的按钮 id 可直接
//!    `tap`；
//! 3. **Keep**：App 整树重建（`Harness::set_tree`）后 `texts` / 焦点 / 光标
//!    **原样保留且仍然可用** —— 因为 `UiState` 按 id 键控、`IdGen` 确定性。
//!    （滚动偏移的重建保留由 `tests/m6_basics.rs` 钉住：testkit 的 `Harness`
//!    刻意不接滚动度量，见该文件与 `keep.md` 的说明。）
//!
//! 找不到系统字体时明确降级（`TESTKIT-FONT` 行）—— 本示例的判据不依赖像素，
//! 降级档照样完整可跑。

use std::process::ExitCode;

use deer_gui::interaction::{InputEvent, Key, Mods, PointerButton, UiEvent};
use deer_gui::prelude::*;
use deer_gui::testing::{Harness, Repro};

fn main() -> ExitCode {
    match run() {
        Ok(()) => {
            println!("[m6_basics] 通过 ✅");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("[m6_basics] 失败：{e}");
            ExitCode::FAILURE
        }
    }
}

/// 演示界面：输入框 + 可滚动列表 + **行动作按钮组**（其中一个按钮禁用）。
fn m6_app(with_disabled: bool) -> Node {
    let mut app = Builder::new(Kind::Column, "app").padding(8.0).gap(6.0);
    app.field("备注");
    app.container_opts(
        Kind::Column,
        "list",
        L::new().w(200.0).h(60.0).scroll(true).to_props(),
        |c| {
            for i in 1..=8 {
                c.text(format!("条目 {i}"));
            }
        },
    );
    let ok = app.button("确定");
    println!("[m6_basics] 按钮自动 id：{ok}（IdGen 按 kind 计数，重建也拿到同样的名字）");
    if with_disabled {
        // 禁用按钮：`button_opts` 的闭包里改 props（先设参数、id 最后定 —— 仓库纪律）。
        let off = app.button_opts("禁用", |n| n.props.disabled = true);
        println!("[m6_basics] 禁用按钮 id：{off}");
    }
    // RowActions：**组合层** —— 返回的 id 与树里的按钮一一对应。
    let actions = app.row_actions_opts("actions", L::new().gap(8.0).to_props(), &["编辑", "删除"]);
    println!("[m6_basics] RowActions 返回的按钮 id：{actions:?}");
    app.build()
}

/// 按 id 找节点（自检用：夹具防漂移）。
fn find<'a>(n: &'a Node, id: &str) -> Option<&'a Node> {
    if n.id == id {
        return Some(n);
    }
    n.children.iter().find_map(|c| find(c, id))
}

fn run() -> Result<(), String> {
    // ① 建面：`Harness` 自带前置断言（node_hint > 0、裁剪快照非空、要求的 id 有几何）。
    let tree = m6_app(true);
    let ok = "button_1".to_string();
    let off = "button_2".to_string();
    let actions = ["button_3".to_string(), "button_4".to_string()];

    let mut h = Harness::new(tree, 260, 220, Theme::default());
    h.set_case("m6_basics");
    h.set_repro(Repro::example("deer-gui", "testing", "m6_basics", "", &[]));
    h.require_ids(&[ok.as_str(), off.as_str(), actions[0].as_str(), actions[1].as_str()]);
    let f = h.frame()?;
    println!(
        "[m6_basics] 一帧：{} 条命令；NodeHint {} 条；有几何的节点 {} 个",
        f.list.len(),
        f.counts().node_hint,
        f.ordered_with_geometry().len()
    );

    // ② RowActions 判据：与手写等价树**逐条绘制命令相同**（渲染复用现有 Row/Button）。
    let mut hand = Builder::new(Kind::Column, "app").padding(8.0).gap(6.0);
    hand.field("备注");
    hand.container_opts(
        Kind::Column,
        "list",
        L::new().w(200.0).h(60.0).scroll(true).to_props(),
        |c| {
            for i in 1..=8 {
                c.text(format!("条目 {i}"));
            }
        },
    );
    hand.button("确定");
    hand.button_opts("禁用", |n| n.props.disabled = true);
    hand.container_opts(Kind::Row, "actions", L::new().gap(8.0).to_props(), |r| {
        r.button("编辑");
        r.button("删除");
    });
    let convenient = m6_app(true);
    assert!(
        convenient.structurally_eq(&hand.build()),
        "RowActions 造出的树必须与手写 Row+button 结构相等"
    );
    let theme = Theme::default();
    let geo = layout(
        &convenient,
        Rect::new(0.0, 0.0, 260.0, 220.0),
        TextStyle { font_size: theme.font_size, line_height: theme.line_height },
        &ApproxMeasure,
    );
    let list_a = build_draw_list(&convenient, &geo, theme.clone(), &ApproxMeasure);
    let list_b = build_draw_list(&hand.build(), &geo, theme, &ApproxMeasure);
    let counts = list_a.counts();
    println!(
        "[m6_basics] 绘制命令：fill_round={} text={} stroke={}（actions 行自身 0 条 —— 无 padding 容器不画底）",
        counts.fill_round_rect, counts.text, counts.stroke_rect
    );
    assert_eq!(list_a, list_b, "RowActions 的绘制命令必须与手写等价树逐条相同");

    // ③ Btn 语义（全部经 testkit 注入，坐标由布局算出）：
    let step = h.tap(ok.as_str())?;
    assert!(
        step.events.contains(&UiEvent::Clicked(ok.clone())),
        "点击必须发 Clicked：{:?}",
        step.events
    );
    h.assert_focus(Some(ok.as_str()))?;
    let enter = h.send(&InputEvent::KeyDown { key: Key::Enter, mods: Mods::default(), repeat: false })?;
    assert_eq!(enter.events, vec![UiEvent::Clicked(ok.clone())], "Enter = 点击");
    println!("[m6_basics] Btn：点击 + Enter 都发 Clicked({ok})，并拿到焦点");

    // disabled：静默 —— 无 Clicked / 无 FocusChanged / 抢不走焦点。
    assert!(find(h.tree(), &off).is_some_and(|n| n.props.disabled), "前置：夹具的第二个按钮必须禁用");
    let step = h.tap(off.as_str())?;
    assert!(
        step.events.iter().all(|e| !matches!(e, UiEvent::Clicked(_) | UiEvent::FocusChanged(_))),
        "禁用按钮必须静默：{:?}",
        step.events
    );
    h.assert_focus(Some(ok.as_str()))?;
    println!("[m6_basics] Btn：disabled 点击无事件、焦点不被抢（渲染走 text_dim/border 底）");

    // 捕获结算：按住拖出树再抬起 ⇒ Clicked 归按下（捕获）的那个按钮（T3.7/D7）。
    let (x, y) = h.center_of(ok.as_str())?;
    h.send(&InputEvent::PointerMoved { x, y })?;
    h.send(&InputEvent::PointerDown { button: PointerButton::Left, x, y })?;
    h.send(&InputEvent::PointerMoved { x: -50.0, y: -50.0 })?;
    let up = h.send(&InputEvent::PointerUp { button: PointerButton::Left, x: -50.0, y: -50.0 })?;
    assert!(
        up.events.contains(&UiEvent::Clicked(ok.clone())),
        "拖出树再抬起也要按捕获者结算：{:?}",
        up.events
    );
    println!("[m6_basics] Btn：拖出树再抬起，Clicked 仍归捕获者 {ok}");

    // ④ Keep：先打字，再**整树重建**，状态与可用性双双保留。
    h.tap("field_1")?; // 聚焦输入框
    h.send(&InputEvent::TextInput { text: "你好".into() })?;
    h.assert_text("field_1", "你好")?;
    let rebuilt = m6_app(true);
    h.set_tree(rebuilt); // App 的惯例：内容变了整树重建；UiState 原样带过去
    h.assert_focus(Some("field_1"))?;
    h.assert_text("field_1", "你好")?;
    h.send(&InputEvent::TextInput { text: "!".into() })?;
    h.assert_text("field_1", "你好!")?;
    println!("[m6_basics] Keep：重建树后 focus/texts/光标保留，输入仍落到同一个输入框");

    // ⑤ RowActions 的按钮走同一条点击路径（组合层没有第二条路）。
    let step = h.tap(actions[1].as_str())?;
    assert!(
        step.events.contains(&UiEvent::Clicked(actions[1].clone())),
        "RowActions 按钮点击必须发它自己的 Clicked：{:?}",
        step.events
    );
    println!("[m6_basics] RowActions：tap({}) 照常发 Clicked", actions[1]);
    Ok(())
}
