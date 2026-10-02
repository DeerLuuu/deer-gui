//! **IME 预编辑**：中文/日文输入法「还没上屏的那一段」怎么进状态、怎么被画出来。
//!
//! 跑法：
//! ```text
//! cargo run -p deer-gui --example ime_preedit
//! ```
//! 产物：**标准输出**（本示例不写文件）。结尾有自检断言。
//!
//! 指南：[`docs/features/ime.md`](../../../docs/features/ime.md)

use deer_gui::gpu::interact::{InteractiveRenderer, InteractState};
use deer_core::DrawCmd;
use deer_gui::gpu::Theme;
use deer_gui::interaction::{InputEvent, UiState, handle};
use deer_gui::layout::TextStyle;
use deer_gui::layout::layout::{ApproxMeasure, ScrollOffsets, layout_with_scroll};
use deer_gui::layout::node::{Kind, Node, Rect};

/// 一个聚焦的输入框（标签 = 已经上屏的内容）。
fn field_tree() -> (Node, deer_gui::layout::Geometry) {
    let mut f = Node::new(Kind::Field, "name");
    f.props.label = Some("ab".into());
    let mut root = Node::new(Kind::Column, "root");
    root.children.push(f);
    let (geo, _m) = layout_with_scroll(
        &root,
        Rect { x: 0.0, y: 0.0, w: 240.0, h: 60.0 },
        TextStyle { font_size: 14.0, line_height: 18.0 },
        &ApproxMeasure,
        &ScrollOffsets::new(),
    );
    (root, geo)
}

fn main() {
    println!("=== IME 预编辑（T3.4）===\n");

    let (tree, geo) = field_tree();
    // 聚焦到输入框、把光标放到末尾（= 字符位 2）
    let mut state = UiState {
        focus: Some("name".into()),
        carets: std::collections::BTreeMap::from([("name".to_string(), 2)]),
        ..Default::default()
    };

    println!("① 预编辑只进**缓冲**，不进 `texts`（还没上屏）");
    handle(
        &mut state,
        &tree,
        &geo,
        deer_gui::interaction::ClipSnapshot::unclipped(),
        &InputEvent::ImePreedit {
            text: "zhong".into(),
        },
    );
    println!(
        "   preedit = {:?}",
        state.preedit.as_ref().map(|p| (p.id.as_str(), p.text.as_str()))
    );
    println!("   texts[\"name\"] = {:?}（没被预编辑污染）", state.texts.get("name"));

    let frame_preedit = draw(&tree, &geo, &state);
    println!("\n② 这一帧画了什么（与「没有预编辑」那帧比，应当**恰好多两条**）：");
    let (x_text, uw) = preedit_facts(&frame_preedit);
    println!("   预编辑文字 x = {x_text}，下划线宽 = {uw}");
    let caret = caret_x(&frame_preedit);
    println!("   光标 x = {caret:?}");

    println!("\n③ 提交（`Commit`）⇒ 进 `texts`，**并且清空缓冲**（无双写）");
    handle(
        &mut state,
        &tree,
        &geo,
        deer_gui::interaction::ClipSnapshot::unclipped(),
        &InputEvent::TextInput { text: "中".into() },
    );
    println!("   texts[\"name\"] = {:?}", state.texts.get("name"));
    println!("   preedit = {:?}（必须清空）", state.preedit);
    let frame_after = draw(&tree, &geo, &state);
    println!(
        "   提交后那两条（预编辑文字 + 下划线）应当消失：{}",
        if preedit_facts(&frame_after).0 < 0 { "已消失 ✅" } else { "❌ 还在" }
    );

    // ─────────────────────────────────────────────────────────────────────
    println!("\n=== 自检 ===");
    assert!(state.preedit.is_none(), "提交之后缓冲必须清空");
    assert_eq!(state.texts.get("name").map(String::as_str), Some("中"));
    assert!(x_text >= 0, "预编辑那一段应当被画出来");
    assert!(uw > 0, "下划线应当盖住它的宽度");
    // 光标在预编辑**之后**（IME 常态）
    assert!(
        caret[0] >= x_text + uw,
        "光标应当在预编辑之后：caret={:?}, 预编辑 x={x_text} 宽={uw}",
        caret[0]
    );
    println!("  ① 预编辑不进 texts、提交后清缓冲 ✅");
    println!("  ② 预编辑文字 + 下划线被画出来 ✅");
    println!("  ③ 光标在预编辑之后 ✅");
    println!("\n全部自检通过。真实用法见指南：把 `UiState` 交给 `to_interact_state()`，");
    println!("`ImePreedit` / `TextInput` 由窗口层从 winit 的 `Ime` 事件映射而来。");
}

/// 用当前状态画一帧（走**唯一**的状态转换）。
fn draw(tree: &Node, geo: &deer_gui::layout::Geometry, state: &UiState) -> deer_core::DrawList {
    let interact: InteractState = state.to_interact_state();
    InteractiveRenderer::new(Theme::default(), &ApproxMeasure, &interact).build(tree, geo)
}

/// 找出预编辑那段文字与下划线：返回 (文字 x, 下划线宽)；没有则 (-1, -1)。
fn preedit_facts(list: &deer_core::DrawList) -> (i32, i32) {
    let text = list.cmds.iter().find_map(|c| match c {
        DrawCmd::Text { rect, text, .. } if text == "zhong" => Some(rect.x),
        _ => None,
    });
    let under = list.cmds.iter().find_map(|c| match c {
        DrawCmd::FillRoundRect { rect, radius, .. } if *radius == 0 && rect.h == 1 => Some(rect.w),
        _ => None,
    });
    (text.unwrap_or(-1), under.unwrap_or(-1))
}

/// 光标（宽 1、半径 0 的填充矩形）的 x。
fn caret_x(list: &deer_core::DrawList) -> Vec<i32> {
    list.cmds
        .iter()
        .filter_map(|c| match c {
            DrawCmd::FillRoundRect { rect, radius, .. } if *radius == 0 && rect.w == 1 => {
                Some(rect.x)
            }
            _ => None,
        })
        .collect()
}
