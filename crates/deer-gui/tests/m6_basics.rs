//! M6 5a 基础控件（`Btn` 语义 / `RowActions` / `Keep`）的**集成判据**。
//!
//! 三块判据各钉什么、出处在哪里：
//!
//! | 判据 | 钉的是什么 | 更底层的出处 |
//! |---|---|---|
//! | `row_actions_*` | 组合层（`Row` + `button`）产出的树与手写等价、绘制命令**逐条相同** | `crates/deer-core/src/builder.rs` 的 `mod tests`（构造） |
//! | `btn_*`（testkit 档） | 点击=捕获者结算、disabled 静默、Enter 激活、焦点序排除禁用 | `src/interaction.rs` 的 r2/r3/r3c/r4/r12 |
//! | `keep_*` | **重建树后 `texts`/焦点/光标/滚动偏移保留**（`UiState` 按 id 键控 + id 确定性） | 本文件（此前没有判据钉这条承诺） |
//!
//! 运行：
//!
//! ```text
//! cargo test -p deer-gui --test m6_basics                        # 纯逻辑档（默认门禁内）
//! cargo test -p deer-gui --features testing --test m6_basics     # 加上 testkit 档
//! ```
//!
//! testkit 那半挂在 `#[cfg(feature = "testing")]` 下（仓库惯例，见
//! `tests/testkit_counter.rs` 的说明）：默认 `cargo test --workspace` 不编译它，
//! `cargo check -p deer-gui --all-targets --features window,testing` 与上面的
//! `--features testing` 命令负责把它带进门禁。

use deer_gui::interaction::{
    handle, ClipSnapshot, InputEvent, Key, Mods, PointerButton, UiEvent, UiState,
};
use deer_gui::layout::layout::{layout_with_scroll, Geometry};
use deer_gui::prelude::*;

/// 测试用的统一字号/行高（与 `src/interaction.rs` 的 scroller fixture 同一套口径）。
const STYLE: TextStyle = TextStyle { font_size: 14.0, line_height: 18.0 };
/// 画布（固定 ⇒ 布局确定性，判据不随环境漂）。
const CANVAS: (f32, f32) = (240.0, 160.0);

/// Keep 判据的界面：输入框 + **可滚动列表**（12 行，超出视口）+ 行动作按钮组。
///
/// `with_field = false` 的变体给「App 删掉了输入框」的场景用（`keep_removed_*`）。
fn m6_app(with_field: bool) -> Node {
    let mut app = Builder::new(Kind::Column, "app").padding(8.0);
    if with_field {
        app.field("备注");
    }
    app.container_opts(
        Kind::Column,
        "list",
        L::new().w(120.0).h(50.0).scroll(true).to_props(),
        |c| {
            for i in 1..=12 {
                c.text(format!("条目 {i}"));
            }
        },
    );
    let actions = app.row_actions_opts("actions", L::new().gap(8.0).to_props(), &["编辑", "删除"]);
    assert_eq!(actions.len(), 2, "测试前置：RowActions 返回两个按钮 id");
    app.build()
}

/// 一帧：`layout_with_scroll`（偏移参与几何）+ 把上限灌回 `UiState` ——
/// 这就是真实 App 每帧对交互层做的事（滚动指南的接线纪律）。
fn frame(tree: &Node, state: &mut UiState) -> Geometry {
    let (geo, metrics) = layout_with_scroll(
        tree,
        Rect::new(0.0, 0.0, CANVAS.0, CANVAS.1),
        STYLE,
        &ApproxMeasure,
        &state.scroll.offsets,
    );
    state.scroll.set_metrics(&metrics);
    geo
}

fn key(k: Key) -> InputEvent {
    InputEvent::KeyDown { key: k, mods: Mods::default(), repeat: false }
}

// ---------------------------------------------------------------------------
// 一、RowActions：构造出的树 + `build_draw_list` 命令数（批次判据）
// ---------------------------------------------------------------------------

/// **判据本体**：便捷构造与「手写 Row + button」的绘制命令**逐条相同** ——
/// RowActions 没有任何自己的渲染路径（组合层红线）。
#[test]
fn row_actions_draw_list_is_identical_to_the_hand_written_row() {
    let mut a = Builder::new(Kind::Column, "app");
    let ids = a.row_actions_opts("actions", L::new().gap(8.0).to_props(), &["编辑", "删除"]);
    let ta = a.build();

    let mut b = Builder::new(Kind::Column, "app");
    b.container_opts(Kind::Row, "actions", L::new().gap(8.0).to_props(), |r| {
        r.button("编辑");
        r.button("删除");
    });
    let tb = b.build();

    assert!(ta.structurally_eq(&tb), "前置：两条构筑路径产出同一棵树（组合层承诺）");

    let theme = Theme::default();
    let geo = layout(
        &ta,
        Rect::new(0.0, 0.0, CANVAS.0, CANVAS.1),
        STYLE,
        &ApproxMeasure,
    );
    for id in &ids {
        assert!(geo.contains_key(id), "前置：返回的按钮 id `{id}` 必须真的有几何");
    }

    let list_a = build_draw_list(&ta, &geo, theme.clone(), &ApproxMeasure);
    let list_b = build_draw_list(&tb, &geo, theme, &ApproxMeasure);

    // 前置断言：按钮真的画了东西 —— 2 个按钮 ⇒ 2 条圆角底 + 2 条文本。
    let c = list_a.counts();
    assert_eq!(
        (c.fill_round_rect, c.text),
        (2, 2),
        "前置：RowActions(2) 的命令构成必须是 2 底 + 2 文本，实际 {c:?}"
    );
    // 判据：两条路径的绘制列表**逐条相同**（`DrawList: PartialEq`，含命令顺序）。
    assert_eq!(list_a, list_b, "RowActions 的绘制命令必须与手写 Row+button 逐条相同");
    // 命令数判据：无 padding 的 Row **零**自身命令 ⇒ 全列表恰为 4 条。
    assert_eq!(list_a.len(), 4, "Row 本身不许产生命令（无 padding 容器不画底）");
}

// ---------------------------------------------------------------------------
// 二、Keep：重建树后瞬态交互状态保留（本批次的语义裁定：显式文档化既有保证）
// ---------------------------------------------------------------------------

/// **Keep 的判据**：App 整树重建（同规格、新 `Builder`、新几何、重新灌滚动上限）后，
/// `texts` / 焦点 / 光标 / 滚动偏移**全部保留，且仍然可用**（不是死的字符串）。
///
/// 这条此前没有判据：`UiState` 按 id 键控 + `IdGen` 确定性 ⇒ 保证天然成立 ——
/// 但「天然成立」与「有测试钉住」是两回事（设计政策 5）。
#[test]
fn keep_rebuilding_the_tree_preserves_texts_focus_and_scroll() {
    let mut state = UiState::default();
    let tree = m6_app(true);
    let geo = frame(&tree, &mut state);

    // ① 点输入框 → 聚焦；抬起（松开捕获）→ 打字进缓冲。
    let f = geo.get("field_1").expect("前置：field_1 有几何");
    handle(
        &mut state, &tree, &geo, ClipSnapshot::unclipped(),
        &InputEvent::PointerDown { button: PointerButton::Left, x: f.x + 5.0, y: f.y + 5.0 },
    );
    assert_eq!(state.focus.as_deref(), Some("field_1"), "前置：点输入框得到焦点");
    handle(
        &mut state, &tree, &geo, ClipSnapshot::unclipped(),
        &InputEvent::PointerUp { button: PointerButton::Left, x: f.x + 5.0, y: f.y + 5.0 },
    );
    handle(
        &mut state, &tree, &geo, ClipSnapshot::unclipped(),
        &InputEvent::TextInput { text: "你好".into() },
    );
    assert_eq!(
        state.texts.get("field_1").map(String::as_str),
        Some("你好"),
        "前置：文本已进缓冲"
    );
    assert_eq!(state.carets.get("field_1"), Some(&2), "前置：光标在「你好」之后（字符位）");

    // ② hover 到**视口内**的列表条目 → 滚轮下滚一档（120px，上限内）。
    //（滚出视口的条目点不中 —— hit_test 按矩形层级剪枝，这正是「视口外点不到」的机制。）
    let item = geo.get("text_2").expect("前置：列表条目有几何");
    handle(
        &mut state, &tree, &geo, ClipSnapshot::unclipped(),
        &InputEvent::PointerMoved { x: item.x + 2.0, y: item.y + 2.0 },
    );
    assert_eq!(state.hover.as_deref(), Some("text_2"), "前置：hover 必须落在列表条目上");
    let wheel = handle(
        &mut state, &tree, &geo, ClipSnapshot::unclipped(),
        &InputEvent::Wheel { dx: 0.0, dy: -3.0 },
    );
    assert!(
        matches!(wheel.last(), Some(UiEvent::Scrolled { id, .. }) if id == "list"),
        "前置：滚轮真的滚了 list，实际 {wheel:?}"
    );
    let scrolled = state.scroll.offset_of("list");
    assert!(scrolled > 0, "前置：偏移 {scrolled} 必须已经离开 0");

    // ③ **重建树**：App 的惯例 —— 内容变了整树重建；`UiState` 原样带过去。
    let rebuilt = m6_app(true);
    assert!(
        rebuilt.structurally_eq(&tree),
        "前置：重建与原树结构相等 —— id 稳定是 Keep 的地基（`IdGen` 确定性）"
    );
    let geo2 = frame(&rebuilt, &mut state);

    // ④ 保留：四个键控状态逐一比对。
    assert_eq!(state.focus.as_deref(), Some("field_1"), "焦点保留");
    assert_eq!(
        state.texts.get("field_1").map(String::as_str),
        Some("你好"),
        "texts 保留（值是应用数据，不随树重建丢失）"
    );
    assert_eq!(state.carets.get("field_1"), Some(&2), "光标保留");
    assert_eq!(state.scroll.offset_of("list"), scrolled, "滚动偏移保留");

    // ⑤ 仍可用：焦点不是死的字符串 —— 继续打字落到同一个输入框。
    let typing = handle(
        &mut state, &rebuilt, &geo2, ClipSnapshot::unclipped(),
        &InputEvent::TextInput { text: "!".into() },
    );
    assert_eq!(
        typing,
        vec![UiEvent::TextChanged { id: "field_1".into(), value: "你好!".into() }],
        "重建后输入仍作用于同一个输入框"
    );
    assert_eq!(state.carets.get("field_1"), Some(&3), "光标跟着走到「你好!」之后");
    // 滚动也仍然作用于同一个容器（还能继续滚 ⇒ 没有停在原地装死）。
    let wheel2 = handle(
        &mut state, &rebuilt, &geo2, ClipSnapshot::unclipped(),
        &InputEvent::Wheel { dx: 0.0, dy: -3.0 },
    );
    assert!(
        matches!(wheel2.last(), Some(UiEvent::Scrolled { id, .. }) if id == "list"),
        "重建后滚轮仍可用，实际 {wheel2:?}"
    );
    assert!(
        state.scroll.offset_of("list") > scrolled,
        "重建后偏移继续前进：{} > {scrolled}",
        state.scroll.offset_of("list")
    );
}

/// Keep 的**边界**（文档承诺的反面）：App 从树里**删掉**输入框后 ——
/// 值作为应用数据**仍在** `texts` 里（库不偷删）；stale 焦点下输入**空转**；
/// `Tab` 从 stale 焦点恢复到树序第一个可聚焦控件。
#[test]
fn keep_removed_field_keeps_its_value_and_focus_recovers() {
    let mut state = UiState::default();
    let tree = m6_app(true);
    let geo = frame(&tree, &mut state);

    let f = geo.get("field_1").expect("前置：field_1 有几何");
    handle(
        &mut state, &tree, &geo, ClipSnapshot::unclipped(),
        &InputEvent::PointerDown { button: PointerButton::Left, x: f.x + 5.0, y: f.y + 5.0 },
    );
    handle(
        &mut state, &tree, &geo, ClipSnapshot::unclipped(),
        &InputEvent::PointerUp { button: PointerButton::Left, x: f.x + 5.0, y: f.y + 5.0 },
    );
    handle(
        &mut state, &tree, &geo, ClipSnapshot::unclipped(),
        &InputEvent::TextInput { text: "你好".into() },
    );
    assert_eq!(state.focus.as_deref(), Some("field_1"), "前置：焦点在输入框上");

    // 重建：**这个版本没有输入框**（App 删了它）。
    let rebuilt = m6_app(false);
    assert!(
        !rebuilt.structurally_eq(&tree),
        "前置：这次两棵树必须不相等（控件真的没了）"
    );
    assert!(
        !deer_gui::interaction::focusables(&rebuilt).iter().any(|i| i == "field_1"),
        "前置：stale id 不在新树的可聚焦集合里"
    );
    let geo2 = frame(&rebuilt, &mut state);

    // 库不偷删焦点（它不知道 App 想怎么处置）……
    assert_eq!(state.focus.as_deref(), Some("field_1"), "焦点串保留（App 的数据）");
    // ……但输入**空转**：焦点指向不存在的节点 ⇒ 不追加、不发事件。
    let out = handle(
        &mut state, &rebuilt, &geo2, ClipSnapshot::unclipped(),
        &InputEvent::TextInput { text: "x".into() },
    );
    assert!(out.is_empty(), "stale 焦点下输入必须空转：{out:?}");
    assert_eq!(
        state.texts.get("field_1").map(String::as_str),
        Some("你好"),
        "值是应用数据：控件删了，值还在（清理是 App 的责任）"
    );
    // `Tab` 恢复：stale id 不在可聚焦集合 ⇒ 按树序取第一个（`next_focus` 的既有语义）。
    let tab = handle(&mut state, &rebuilt, &geo2, ClipSnapshot::unclipped(), &key(Key::Tab));
    assert_eq!(
        tab,
        vec![UiEvent::FocusChanged(Some("button_1".into()))],
        "Tab 从 stale 焦点恢复到树序第一个可聚焦控件"
    );
}

// ---------------------------------------------------------------------------
// 三、Btn / RowActions 的 testkit 档（`--features testing`；仓库惯例见文件头）
// ---------------------------------------------------------------------------

#[cfg(feature = "testing")]
mod via_testkit {
    use deer_gui::interaction::{InputEvent, Key, Mods, PointerButton, UiEvent};
    use deer_gui::prelude::*;
    use deer_gui::testing::{Harness, Repro};

    /// 按 id 找节点（测试夹具防漂移用）。
    fn find<'a>(n: &'a Node, id: &str) -> Option<&'a Node> {
        if n.id == id {
            return Some(n);
        }
        n.children.iter().find_map(|c| find(c, id))
    }

    /// 判据面：**RowActions 造出来的按钮 + 既有按钮语义**，全程经 testkit 注入。
    ///
    /// 覆盖的语义（每条的更底层单测见文件头表格；这里钉的是「testkit 注入路径」）：
    /// 点击 ⇒ `Clicked` + 聚焦；`Enter` = 点击；disabled ⇒ 无 Clicked/无 FocusChanged/
    /// 抢不走焦点；拖出树再抬起 ⇒ 按**捕获者**结算（D7/T3.7）。
    #[test]
    fn btn_and_row_actions_semantics_via_testkit() -> Result<(), String> {
        let mut app = Builder::new(Kind::Column, "app").padding(8.0).gap(6.0);
        let ok = app.button("确定");
        let off = app.button_opts("禁用", |n| n.props.disabled = true);
        let actions = app.row_actions_opts("actions", L::new().gap(8.0).to_props(), &["编辑", "删除"]);

        let mut h = Harness::new(app, 240, 200, Theme::default());
        h.set_case("m6_btn_row_actions");
        h.set_repro(Repro::test("deer-gui", "testing", "m6_basics", "btn_and_row_actions", &[]));
        h.require_ids(&[ok.as_str(), off.as_str(), actions[0].as_str(), actions[1].as_str()]);

        // 前置断言：夹具真的长这样（防漂移 —— 否则后面全在测空气）。
        let tree = h.tree().clone();
        assert!(find(&tree, &off).is_some_and(|n| n.props.disabled), "前置：第二个按钮必须是禁用的");
        assert_eq!(actions, vec!["button_3".to_string(), "button_4".to_string()], "前置：RowActions 的自动 id");
        h.assert_focus_order_excludes(&[off.as_str()])?;

        // ① 点启用的按钮：`Clicked` + 聚焦它。
        let step = h.tap(ok.as_str())?;
        assert!(
            step.events.contains(&UiEvent::Clicked(ok.clone())),
            "点击必须发 Clicked：{:?}",
            step.events
        );
        h.assert_focus(Some(ok.as_str()))?;

        // ② `Enter` 在启用的焦点按钮上 = 点击（键激活语义）。
        let enter = h.send(&InputEvent::KeyDown { key: Key::Enter, mods: Mods::default(), repeat: false })?;
        assert_eq!(enter.events, vec![UiEvent::Clicked(ok.clone())], "Enter = 点击");

        // ③ 点禁用按钮：无 `Clicked` / 无 `FocusChanged`，焦点抢不走，`pressed` 不落。
        let step = h.tap(off.as_str())?;
        assert!(
            step.events.iter().all(|e| !matches!(e, UiEvent::Clicked(_) | UiEvent::FocusChanged(_))),
            "禁用按钮必须静默：{:?}",
            step.events
        );
        h.assert_focus(Some(ok.as_str()))?;
        h.assert_pressed(None)?;

        // ④ 拖出树再抬起：`Clicked` 归**捕获者**（T3.7/D7 —— 拖出即丢是旧行为）。
        let (x, y) = h.center_of(ok.as_str())?;
        h.send(&InputEvent::PointerMoved { x, y })?;
        h.send(&InputEvent::PointerDown { button: PointerButton::Left, x, y })?;
        let moved = h.send(&InputEvent::PointerMoved { x: -50.0, y: -50.0 })?;
        assert!(moved.events.is_empty(), "捕获中的移动不换 hover：{:?}", moved.events);
        let up = h.send(&InputEvent::PointerUp { button: PointerButton::Left, x: -50.0, y: -50.0 })?;
        assert!(
            up.events.contains(&UiEvent::Clicked(ok.clone())),
            "抬起按捕获者结算：{:?}",
            up.events
        );
        h.assert_pressed(None)?;

        // ⑤ RowActions 造出来的按钮走的是**同一条**点击路径（组合层没有第二条路）。
        let step = h.tap(actions[1].as_str())?;
        assert!(
            step.events.contains(&UiEvent::Clicked(actions[1].clone())),
            "RowActions 按钮点击必须发它自己的 Clicked：{:?}",
            step.events
        );
        Ok(())
    }
}
