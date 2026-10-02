//! M5-2/M5-3 的**端到端纯逻辑验证**（只用公开 API，不开 `window` feature、不碰 GPU）。
//!
//! 与 `src/interaction.rs` 里的单测分工：
//! - 单测：每条交互规则一例（手写几何，边界精确）；
//! - 这里：**真实布局 + 真实绘制列表**上跑一遍脚本化重放，并断言重放**可重复**。
//!
//! 字符串脚本格式（`DEER_INPUT_SCRIPT="move:120,80;down:left;..."`）属于 M5-4
//! 的 `examples/interactive_form.rs`（那是窗口侧的例子）；这里重放的是**同一串事件的
//! 值形式**，避免两边各定义一套脚本语法。

use deer_core::{DrawCmd, DrawList, RectI};
use deer_gui::gpu::NullRenderer;
use deer_gui::interaction::{
    ClipSnapshot, InputEvent, Key, Mods, PointerButton, UiEvent, UiState, focusables, handle, hit,
};
use deer_gui::layout::Geometry;
use deer_gui::prelude::*;

const W: f32 = 320.0;
const H: f32 = 200.0;

/// 一棵用**命令式 API** 建的树（id 由 `IdGen` 确定性生成，所以断言里可以写死名字）。
fn app_tree() -> Node {
    let mut b = Builder::new(Kind::Column, "app").padding(8.0).gap(6.0);
    b.button("确定");
    b.button_opts("禁用", |n| n.props.disabled = true);
    b.field("名字");
    b.build()
}

fn layout_of(t: &Node) -> Geometry {
    layout(t, Rect::new(0.0, 0.0, W, H), TextStyle::default(), &ApproxMeasure)
}

fn center(g: &Geometry, id: &str) -> (f32, f32) {
    let r = g
        .get(id)
        .copied()
        .unwrap_or_else(|| panic!("测试前置：布局必须给 {id} 几何"));
    (r.x + r.w / 2.0, r.y + r.h / 2.0)
}

fn hint_cmd(id: &str, g: &Geometry) -> DrawCmd {
    let r = g.get(id).copied().unwrap_or_else(|| panic!("测试前置：{id} 没有几何"));
    DrawCmd::node_hint(RectI::new(r.x as i32, r.y as i32, r.w as i32, r.h as i32), id)
}

fn press(k: Key, shift: bool) -> InputEvent {
    InputEvent::KeyDown {
        key: k,
        mods: Mods { shift, ..Default::default() },
        repeat: false,
    }
}

/// 脚本化重放：返回（事件日志、最终状态）。
///
/// 顺序刻意写成「每个动词各司其职」（M5-4 起 pointer-down 会**聚焦**命中的可聚焦控件，
/// 所以「Tab 走两格」那种隐式依赖不再是好写法 —— 它会随焦点规则变化而静默失效）：
///
/// ① 点 `button_1` ⇒ hover + 点击聚焦 + `Clicked`；
/// ② 点 `field_1` ⇒ hover + 聚焦它（**直接**，不靠 Tab 计数）；
/// ③ 打字 + Backspace ⇒ 缓冲变化；④ Escape ⇒ 清焦点；⑤ 移出 ⇒ hover 清空。
fn replay(t: &Node, g: &Geometry, snap: &ClipSnapshot) -> (Vec<UiEvent>, UiState) {
    let (bx, by) = center(g, "button_1");
    let (fx, fy) = center(g, "field_1");
    let script: Vec<(&str, InputEvent)> = vec![
        ("move:button_1", InputEvent::PointerMoved { x: bx, y: by }),
        ("down:left", InputEvent::PointerDown { button: PointerButton::Left, x: bx, y: by }),
        ("up:left", InputEvent::PointerUp { button: PointerButton::Left, x: bx, y: by }),
        ("move:field_1", InputEvent::PointerMoved { x: fx, y: fy }),
        ("down:left", InputEvent::PointerDown { button: PointerButton::Left, x: fx, y: fy }),
        ("up:left", InputEvent::PointerUp { button: PointerButton::Left, x: fx, y: fy }),
        ("text:hi", InputEvent::TextInput { text: "hi".into() }),
        ("text:你好", InputEvent::TextInput { text: "你好".into() }),
        ("key:Backspace", press(Key::Backspace, false)),
        ("key:Escape", press(Key::Escape, false)),
        ("move:outside", InputEvent::PointerMoved { x: -1.0, y: -1.0 }),
    ];

    let mut state = UiState::default();
    let mut log = Vec::new();
    for (label, ev) in &script {
        let out = handle(&mut state, t, g, snap.clone(), ev);
        println!("  {label:<16} -> {out:?}");
        log.extend(out);
    }
    (log, state)
}

#[test]
fn scripted_replay_is_deterministic_and_matches_expected_state() {
    let t = app_tree();
    let g = layout_of(&t);
    let list = NullRenderer::build(&t, &g);
    let snap = ClipSnapshot::from_draw_list(&list, &t, &g);

    // 前置：真实列表确实给每个有几何的节点发了 NodeHint，快照里有名字。
    println!("node_hint={} 命令数={}", list.counts().node_hint, list.len());
    for id in ["app", "button_1", "button_2", "field_1"] {
        assert!(g.contains_key(id), "测试前置：{id} 必须有几何");
        assert!(snap.is_known(id), "护栏前置：{id} 必须在裁剪快照里（否则裁剪判据没有判别力）");
        assert_eq!(snap.clip_of(id), None, "{id} 不该被裁剪");
    }
    println!("焦点树序 = {:?}", focusables(&t));
    assert_eq!(
        focusables(&t),
        vec!["button_1".to_string(), "field_1".into()],
        "禁用按钮必须不在焦点序列里"
    );

    println!("第一次重放：");
    let (log1, state1) = replay(&t, &g, &snap);
    println!("最终状态 = {state1:?}");

    let expected = vec![
        UiEvent::HoverChanged(Some("button_1".into())),
        // M5-4：按下可聚焦控件时**当场**聚焦它（`FocusChanged` 在 `PointerDown` 那一步产生，
        // 早于 `PointerUp` 才产生的 `Clicked`）。
        UiEvent::FocusChanged(Some("button_1".into())),
        UiEvent::Clicked("button_1".into()),
        UiEvent::HoverChanged(Some("field_1".into())),
        UiEvent::FocusChanged(Some("field_1".into())),
        UiEvent::Clicked("field_1".into()),
        UiEvent::TextChanged { id: "field_1".into(), value: "hi".into() },
        UiEvent::TextChanged { id: "field_1".into(), value: "hi你好".into() },
        UiEvent::TextChanged { id: "field_1".into(), value: "hi你".into() },
        UiEvent::FocusChanged(None),
        UiEvent::HoverChanged(None),
    ];
    assert_eq!(log1, expected, "事件序列必须逐条相等");

    let expect_state = UiState {
        hover: None,
        focus: None,
        pressed: None,
        texts: std::collections::BTreeMap::from([("field_1".to_string(), "hi你".to_string())]),
        // 打完字光标停在**末尾**（3 个字符：`h`、`i`、`你`）—— 这是「表里没有 id 就等于末尾」
        // 的另一半：一旦动过文本，`carets` 就会被显式写上，但它的值仍是「追加」该有的值。
        carets: std::collections::BTreeMap::from([("field_1".to_string(), 3usize)]),
        scroll: Default::default(),
        preedit: None,
    };
    assert_eq!(state1, expect_state, "最终状态必须逐字段相等");

    // 可回归：同一串事件再放一遍 ⇒ 逐条相同。
    println!("第二次重放（应当逐条相同）：");
    let (log2, state2) = replay(&t, &g, &snap);
    assert_eq!(log1, log2, "重放必须确定性");
    assert_eq!(state1, state2);
}

#[test]
fn disabled_button_ignores_pointer_and_clipped_pointer_is_not_hit() {
    let t = app_tree();
    let g = layout_of(&t);
    let (dx, dy) = center(&g, "button_2");

    // ① 禁用按钮：几何上打得中，`hit` 必须拒绝（前置先钉死「几何上确实打得中」）。
    assert_eq!(
        hit_test(&t, &g, dx, dy).map(|n| n.id.as_str()),
        Some("button_2"),
        "测试前置：hit_test 会命中禁用按钮"
    );
    let mut state = UiState::default();
    let out = handle(
        &mut state,
        &t,
        &g,
        ClipSnapshot::unclipped(),
        &InputEvent::PointerMoved { x: dx, y: dy },
    );
    println!("禁用按钮上 hover：{out:?}");
    assert!(out.is_empty() && state.hover.is_none(), "禁用按钮不响应指针");

    // ② 裁剪：只留左上角一小块，输入框（在下方）必须打不中；而 `hit_test` 单独仍会命中它。
    let (fx, fy) = center(&g, "field_1");
    let mut cmds = vec![DrawCmd::PushClip { rect: RectI::new(0, 0, 60, 40) }];
    for id in ["app", "button_1", "button_2", "field_1"] {
        cmds.push(hint_cmd(id, &g));
    }
    cmds.push(DrawCmd::PopClip);
    let clipped = DrawList::from_cmds(cmds);
    assert_eq!(
        (clipped.counts().node_hint, clipped.counts().push_clip, clipped.counts().pop_clip),
        (4, 1, 1),
        "测试前置：这份列表 4 条节点提示 + 一对裁剪命令"
    );
    let snap = ClipSnapshot::from_draw_list(&clipped, &t, &g);
    assert!(snap.is_known("field_1"), "护栏前置：field_1 必须在快照里");
    assert_eq!(snap.clip_of("field_1"), Some(RectI::new(0, 0, 60, 40)));

    println!("field_1 中心 = ({fx},{fy})，裁剪 = {:?}", snap.clip_of("field_1"));
    assert!(fx >= 60.0 || fy >= 40.0, "测试前置：输入框中心必须在裁剪外");
    assert_eq!(
        hit_test(&t, &g, fx, fy).map(|n| n.id.as_str()),
        Some("field_1"),
        "测试前置：hit_test 会命中输入框"
    );
    assert_eq!(
        hit(&t, &g, snap.clone(), fx, fy).map(|n| n.id.as_str()),
        None,
        "被裁掉的点不命中"
    );

    // 裁剪内的点仍然能命中（否则「不命中」可能只是整体哑掉了）。
    let inside = (4.0f32, 4.0f32);
    println!(
        "裁剪内 {inside:?} ⇒ hit={:?} / hit_test={:?}",
        hit(&t, &g, snap, inside.0, inside.1).map(|n| n.id.as_str()),
        hit_test(&t, &g, inside.0, inside.1).map(|n| n.id.as_str())
    );
    assert!(
        hit(&t, &g, ClipSnapshot::from_draw_list(&clipped, &t, &g), inside.0, inside.1).is_some(),
        "裁剪内的点必须还能命中"
    );
}
