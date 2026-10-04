//! M6 5d 数值类四件（`NumberField` / `ScrubNum` / `Switch` / `ColorField`）的**集成判据**。
//!
//! 判据面与出处（指南在 `docs/features/{number-field,scrub-num,switch,color-field}.md`）：
//!
//! | 判据 | 钉的是什么 | 更底层的出处 |
//! |---|---|---|
//! | `number_field_*` | 编辑期草稿进 `texts`（与 `Field` 同一套机械）、**提交才解析**（`Enter`/失焦两路都到）、解析失败不发事件且草稿保留、值域夹取 | `src/interaction.rs` 的 `commit_value_field` + 失焦钩子 |
//! | `scrub_num_*` | 按下时从 label 解析基准（T3.7 捕获之上）、拖动中**变了才发** `NumberChanged`、步长/值域、抬起锚点即清、label 不可解析 = 不进入 | `PointerDown`/`PointerMoved` 的锚点块 |
//! | `switch_*` | 点击 / `Enter` / `Space`（= winit 的 `Named(Space) ⇒ Key::Char(' ')`）翻转并发 `Toggled{on: 翻转后的值}`、初值可塞、禁用静默、`ScrubNum` 不进焦点序 | `resolve_switch`（r16） |
//! | `color_field_*` | 提交按 `#RRGGBB` 解析（大小写/无 `#` 都行）、成功回写规范化串、失败不发 | 同 `commit_value_field` |
//! | `keep_*` | 值表是应用数据：整树重建后保留且仍可用 | `UiState` 按 id 键控（texts 同一条纪律） |
//! | `*_visual_*` | 开/关视觉只落在开关矩形内、关 = 5c 的 muted 档、不可解析下划线只在草稿真坏时出现、色块颜色 = 草稿解析结果 | `deer-gpu/src/interact.rs` |
//! | `via_testkit` | testkit 注入路径（点击/打字/`Enter`/拖动） | `src/testing.rs` |
//!
//! 夹具（显式 id ⇒ 断言里直接写）：`button_1`=「顶」（组外/控件外红线）·
//! `age`（NumberField，占位 "0"）· `vol`（ScrubNum，占位 "40"）· `wifi`（Switch）·
//! `tint`（ColorField，占位 "#ff8800"）· `field_1`（普通 `Field` 对照）。
//!
//! 运行：
//!
//! ```text
//! cargo test -p deer-gui --test m6_values                        # 纯逻辑档（默认门禁内）
//! cargo test -p deer-gui --features testing --test m6_values     # 加上 testkit 档
//! ```

use deer_gui::interaction::{
    handle, ClipSnapshot, InputEvent, Key, Mods, PointerButton, UiEvent, UiState,
};
use deer_gui::layout::layout::{layout, Geometry};
use deer_gui::prelude::*;

/// 测试用的统一字号/行高（与 `tests/m6_select.rs` 同一套口径）。
const STYLE: TextStyle = TextStyle { font_size: 14.0, line_height: 18.0 };
/// 画布（固定 ⇒ 布局确定性，判据不随环境漂）。
const CANVAS: (f32, f32) = (260.0, 220.0);

/// 判据界面：普通按钮 + 四种数值类控件 + 普通 `Field` 对照。
fn values_app() -> Node {
    let mut app = Builder::new(Kind::Column, "app").padding(8.0).gap(6.0);
    app.button("顶");
    app.number_field_opts("age", "0", |_| {});
    app.scrub_num_opts("vol", "40", |_| {});
    app.switch_opts("wifi", "Wi-Fi", |_| {});
    app.color_field_opts("tint", "#ff8800", |_| {});
    app.field("备注");
    app.build()
}

fn geo_of(tree: &Node) -> Geometry {
    layout(
        tree,
        Rect::new(0.0, 0.0, CANVAS.0, CANVAS.1),
        STYLE,
        &ApproxMeasure,
    )
}

fn key(k: Key) -> InputEvent {
    InputEvent::KeyDown { key: k, mods: Mods::default(), repeat: false }
}

/// 指针事件的三步合成（move → down → up 的各自一半由调用方拆开用）。
fn center_of(geo: &Geometry, id: &str) -> (f32, f32) {
    let f = geo.get(id).unwrap_or_else(|| panic!("测试前置：{id} 必须有几何"));
    (f.x + f.w / 2.0, f.y + f.h / 2.0)
}

fn move_to(state: &mut UiState, tree: &Node, geo: &Geometry, x: f32, y: f32) -> Vec<UiEvent> {
    handle(state, tree, geo, ClipSnapshot::unclipped(), &InputEvent::PointerMoved { x, y })
}

fn down_at(state: &mut UiState, tree: &Node, geo: &Geometry, x: f32, y: f32) -> Vec<UiEvent> {
    handle(
        state, tree, geo, ClipSnapshot::unclipped(),
        &InputEvent::PointerDown { button: PointerButton::Left, x, y },
    )
}

fn up_at(state: &mut UiState, tree: &Node, geo: &Geometry, x: f32, y: f32) -> Vec<UiEvent> {
    handle(
        state, tree, geo, ClipSnapshot::unclipped(),
        &InputEvent::PointerUp { button: PointerButton::Left, x, y },
    )
}

/// 「点一下」= move → down → up 三步的并集。
fn tap_at(state: &mut UiState, tree: &Node, geo: &Geometry, id: &str) -> Vec<UiEvent> {
    let (x, y) = center_of(geo, id);
    let mut out = move_to(state, tree, geo, x, y);
    out.extend(down_at(state, tree, geo, x, y));
    out.extend(up_at(state, tree, geo, x, y));
    out
}

/// 焦点放进某个控件（点击路径，与真实用户一致）。
fn focus_by_tap(state: &mut UiState, tree: &Node, geo: &Geometry, id: &str) {
    tap_at(state, tree, geo, id);
    assert_eq!(
        state.focus.as_deref(),
        Some(id),
        "测试前置：点击可聚焦控件后焦点必须在它上"
    );
}

/// 收集事件里的某个变体（断言辅助，避免一长串 matches!）。
fn find(events: &[UiEvent], f: impl Fn(&UiEvent) -> bool) -> Option<&UiEvent> {
    events.iter().find(|e| f(e))
}

// ---------------------------------------------------------------------------
// 一、NumberField：编辑草稿 + 提交才解析（Enter 与失焦两路）
// ---------------------------------------------------------------------------

#[test]
fn number_field_edits_a_draft_and_commits_on_enter() {
    let tree = values_app();
    let geo = geo_of(&tree);
    let mut s = UiState::default();

    // 前置：age 在焦点序里（值输入框必须可聚焦，否则打字无从谈起）。
    assert!(
        deer_gui::interaction::focusables(&tree).contains(&"age".to_string()),
        "测试前置：NumberField 必须可聚焦"
    );

    focus_by_tap(&mut s, &tree, &geo, "age");
    // 打字进**草稿**（texts）—— 与 Field 同一套机械，逐字符发 TextChanged。
    let ev = handle(&mut s, &tree, &geo, ClipSnapshot::unclipped(),
        &InputEvent::TextInput { text: "-1.50".into() });
    assert!(
        ev.iter().any(|e| matches!(e, UiEvent::TextChanged { id, .. } if id == "age")),
        "打字必须发 TextChanged（草稿在长）：{ev:?}"
    );
    assert_eq!(s.texts.get("age").map(String::as_str), Some("-1.50"), "草稿必须落位");

    // `Enter` = 提交：解析成功 ⇒ NumberChanged（f64）+ 草稿**规范化回写**（-1.50 → -1.5），
    // 且回写**不发** TextChanged（值已由 NumberChanged 报告，不是用户新输入）。
    let ev = handle(&mut s, &tree, &geo, ClipSnapshot::unclipped(), &key(Key::Enter));
    assert_eq!(
        find(&ev, |e| matches!(e, UiEvent::NumberChanged { .. })),
        Some(&UiEvent::NumberChanged { id: "age".into(), value: -1.5 }),
        "Enter 必须提交出 NumberChanged：{ev:?}"
    );
    assert!(
        !ev.iter().any(|e| matches!(e, UiEvent::TextChanged { .. })),
        "规范化回写不得发 TextChanged：{ev:?}"
    );
    assert_eq!(s.texts.get("age").map(String::as_str), Some("-1.5"), "回写必须是规范化串");
}

#[test]
fn number_field_rejects_unparseable_and_keeps_the_draft() {
    let tree = values_app();
    let geo = geo_of(&tree);
    let mut s = UiState::default();

    focus_by_tap(&mut s, &tree, &geo, "age");
    handle(&mut s, &tree, &geo, ClipSnapshot::unclipped(),
        &InputEvent::TextInput { text: "3px".into() });
    assert_eq!(s.texts.get("age").map(String::as_str), Some("3px"), "前置：草稿不可解析");

    // 提交失败：**不发**任何值事件，草稿原样保留（绘制侧按「非空且不可解析」画下划线）。
    let ev = handle(&mut s, &tree, &geo, ClipSnapshot::unclipped(), &key(Key::Enter));
    assert!(
        !ev.iter().any(|e| matches!(e, UiEvent::NumberChanged { .. } | UiEvent::ColorChanged { .. })),
        "不可解析的提交必须静默（不发值事件）：{ev:?}"
    );
    assert_eq!(s.texts.get("age").map(String::as_str), Some("3px"), "失败的提交不得动草稿");
}

#[test]
fn number_field_commits_on_blur_and_clamps_into_the_range() {
    let tree = values_app();
    let geo = geo_of(&tree);
    let mut s = UiState::default();
    // 值域是 App 的数据：age ∈ [0, 150]（texts 同一条纪律：先塞 UiState）。
    s.num_opts.insert("age".into(), deer_gui::interaction::NumOpts {
        min: Some(0.0),
        max: Some(150.0),
        step: 1.0,
    });

    focus_by_tap(&mut s, &tree, &geo, "age");
    handle(&mut s, &tree, &geo, ClipSnapshot::unclipped(),
        &InputEvent::TextInput { text: "999".into() });

    // `Tab` = 失焦（age → 下一个可聚焦 wifi）：失焦钩子必须替它提交 + 夹取。
    let ev = handle(&mut s, &tree, &geo, ClipSnapshot::unclipped(), &key(Key::Tab));
    assert!(
        ev.iter().any(|e| matches!(e, UiEvent::FocusChanged(Some(id)) if id == "wifi")),
        "前置：Tab 必须把焦点挪走（失焦才有得提交）：{ev:?}"
    );
    assert_eq!(
        find(&ev, |e| matches!(e, UiEvent::NumberChanged { .. })),
        Some(&UiEvent::NumberChanged { id: "age".into(), value: 150.0 }),
        "失焦必须提交，且 999 必须被夹进 [0,150]：{ev:?}"
    );
    assert_eq!(s.texts.get("age").map(String::as_str), Some("150"), "夹取后的值必须回写");

    // `Escape` 清焦点同样是失焦 ⇒ 同一路提交（草稿 "abc" 解析失败 ⇒ 什么都不发）。
    let mut s2 = UiState::default();
    focus_by_tap(&mut s2, &tree, &geo, "age");
    handle(&mut s2, &tree, &geo, ClipSnapshot::unclipped(),
        &InputEvent::TextInput { text: "abc".into() });
    let ev = handle(&mut s2, &tree, &geo, ClipSnapshot::unclipped(), &key(Key::Escape));
    assert!(
        !ev.iter().any(|e| matches!(e, UiEvent::NumberChanged { .. })),
        "Escape 触发的失焦提交同样受「解析失败不发」约束：{ev:?}"
    );
    assert_eq!(s2.texts.get("age").map(String::as_str), Some("abc"), "失败不得动草稿");
}

/// 对照组：普通 `Field` **没有**提交语义 —— `Enter` 不解析、不发值事件、草稿原样。
#[test]
fn plain_field_is_untouched_by_the_commit_machinery() {
    let tree = values_app();
    let geo = geo_of(&tree);
    let mut s = UiState::default();

    focus_by_tap(&mut s, &tree, &geo, "field_1");
    handle(&mut s, &tree, &geo, ClipSnapshot::unclipped(),
        &InputEvent::TextInput { text: " 42 ".into() });
    let ev = handle(&mut s, &tree, &geo, ClipSnapshot::unclipped(), &key(Key::Enter));
    assert!(
        !ev.iter().any(|e| matches!(e, UiEvent::NumberChanged { .. } | UiEvent::ColorChanged { .. })),
        "普通 Field 的 Enter 不得触发任何提交：{ev:?}"
    );
    assert_eq!(s.texts.get("field_1").map(String::as_str), Some(" 42 "), "普通 Field 草稿不得被规范化");
}

// ---------------------------------------------------------------------------
// 二、ScrubNum：按下解析基准 + 拖动变了才发
// ---------------------------------------------------------------------------

#[test]
fn scrub_num_drags_from_the_parsed_anchor_and_clears_on_release() {
    let tree = values_app();
    let geo = geo_of(&tree);
    let mut s = UiState::default();
    let (x0, y0) = center_of(&geo, "vol");

    // 按下：捕获 + 建立锚点（基准 = parse_num("40") = 40）。step 缺省 = 1.0/px。
    move_to(&mut s, &tree, &geo, x0, y0);
    down_at(&mut s, &tree, &geo, x0, y0);
    assert_eq!(s.pressed.as_deref(), Some("vol"), "前置：按下即捕获（T3.7）");
    assert_eq!(s.scrub.as_ref().map(|a| a.base), Some(40.0), "锚点基准必须来自 label 解析");

    // 右拖 5 px：40 + 5×1.0 = 45 ⇒ 发一次 NumberChanged。
    let ev = move_to(&mut s, &tree, &geo, x0 + 5.0, y0);
    assert_eq!(
        find(&ev, |e| matches!(e, UiEvent::NumberChanged { .. })),
        Some(&UiEvent::NumberChanged { id: "vol".into(), value: 45.0 }),
        "拖 +5px 必须发 NumberChanged{{45}}（step=1.0）：{ev:?}"
    );
    // 原地再动一次：值没变 ⇒ **不发**（与 `Scrolled`「变了才发」同纪律）。
    let ev = move_to(&mut s, &tree, &geo, x0 + 5.0, y0);
    assert!(
        !ev.iter().any(|e| matches!(e, UiEvent::NumberChanged { .. })),
        "值没变的移动不得重发 NumberChanged：{ev:?}"
    );
    // 左拖到 x0 − 3：不是「相对上一步 −8」，而是**从锚点反解**（base + (x − start_x)·step
    // = 40 − 3 = 37）—— 锚点固定不动，所以「App 慢一帧没重建树」也不影响后续值。
    let ev = move_to(&mut s, &tree, &geo, x0 - 3.0, y0);
    assert_eq!(
        find(&ev, |e| matches!(e, UiEvent::NumberChanged { .. })),
        Some(&UiEvent::NumberChanged { id: "vol".into(), value: 37.0 }),
        "值必须从锚点反解（40 − 3 = 37，与上一步的 45 无关）：{ev:?}"
    );
    // 拖出控件再抬起：捕获者结算 Clicked（D7），锚点即清（瞬态）。
    let ev = up_at(&mut s, &tree, &geo, x0 - 3.0, y0);
    assert!(ev.contains(&UiEvent::Clicked("vol".into())), "前置：抬起按捕获者结算：{ev:?}");
    assert!(
        !ev.iter().any(|e| matches!(e, UiEvent::NumberChanged { .. })),
        "抬起本身不得再发值事件：{ev:?}"
    );
    assert!(s.scrub.is_none(), "抬起后锚点必须清空");
}

#[test]
fn scrub_num_uses_step_and_range_from_num_opts() {
    let tree = values_app();
    let geo = geo_of(&tree);
    let mut s = UiState::default();
    // vol：step = 2.0/px，值域 [0, 42]（App 塞，控件只读）。
    s.num_opts.insert("vol".into(), deer_gui::interaction::NumOpts {
        min: Some(0.0),
        max: Some(42.0),
        step: 2.0,
    });

    let (x0, y0) = center_of(&geo, "vol");
    move_to(&mut s, &tree, &geo, x0, y0);
    down_at(&mut s, &tree, &geo, x0, y0);
    // +5 px × 2.0 = +10 ⇒ 原始 50，夹进 [0,42] = 42。
    let ev = move_to(&mut s, &tree, &geo, x0 + 5.0, y0);
    assert_eq!(
        find(&ev, |e| matches!(e, UiEvent::NumberChanged { .. })),
        Some(&UiEvent::NumberChanged { id: "vol".into(), value: 42.0 }),
        "拖动值必须过值域夹取（50 → 42）：{ev:?}"
    );
    // 反向拖 −25 px × 2.0 = −50 ⇒ 原始 −10（从基准 40），夹到 0。
    let ev = move_to(&mut s, &tree, &geo, x0 - 20.0, y0);
    assert_eq!(
        find(&ev, |e| matches!(e, UiEvent::NumberChanged { .. })),
        Some(&UiEvent::NumberChanged { id: "vol".into(), value: 0.0 }),
        "反向越界同样必须夹到下界（−10 → 0）：{ev:?}"
    );
    up_at(&mut s, &tree, &geo, x0 - 20.0, y0);
}

#[test]
fn scrub_num_with_unparseable_label_never_enters_scrubbing() {
    // 专门造一棵 vol 的 label 不可解析的树（按下不得建立锚点，拖动不得发事件）。
    let mut app = Builder::new(Kind::Column, "app").padding(8.0).gap(6.0);
    app.scrub_num_opts("vol", "abc", |_| {});
    let tree = app.build();
    let geo = geo_of(&tree);
    let mut s = UiState::default();

    let (x0, y0) = center_of(&geo, "vol");
    move_to(&mut s, &tree, &geo, x0, y0);
    down_at(&mut s, &tree, &geo, x0, y0);
    assert!(s.scrub.is_none(), "label 不可解析 ⇒ 按下不得建立锚点");
    let ev = move_to(&mut s, &tree, &geo, x0 + 20.0, y0);
    assert!(
        !ev.iter().any(|e| matches!(e, UiEvent::NumberChanged { .. })),
        "没有锚点 ⇒ 拖动不得发 NumberChanged：{ev:?}"
    );
    up_at(&mut s, &tree, &geo, x0 + 20.0, y0);
}

// ---------------------------------------------------------------------------
// 三、Switch：点击 / Enter / Space 三路同一条结算（r16）
// ---------------------------------------------------------------------------

#[test]
fn switch_toggles_on_click_enter_and_space() {
    let tree = values_app();
    let geo = geo_of(&tree);
    let mut s = UiState::default();

    // ① 点击：Clicked + Toggled{on: true}（翻转语义：每次激活必发），值表落位。
    let ev = tap_at(&mut s, &tree, &geo, "wifi");
    assert!(ev.contains(&UiEvent::Clicked("wifi".into())), "前置：点击要发 Clicked：{ev:?}");
    assert_eq!(
        find(&ev, |e| matches!(e, UiEvent::Toggled { .. })),
        Some(&UiEvent::Toggled { id: "wifi".into(), on: true }),
        "首点开关必发 Toggled{{on:true}}：{ev:?}"
    );
    assert_eq!(s.switches.get("wifi"), Some(&true), "值表必须落位");
    assert_eq!(s.focus.as_deref(), Some("wifi"), "点击开关同时聚焦它（Space/Enter 的前提）");

    // ② `Enter`（焦点已在它上）：同一条 resolve_switch（键盘与指针不分叉）。
    let ev = handle(&mut s, &tree, &geo, ClipSnapshot::unclipped(), &key(Key::Enter));
    assert_eq!(
        find(&ev, |e| matches!(e, UiEvent::Toggled { .. })),
        Some(&UiEvent::Toggled { id: "wifi".into(), on: false }),
        "Enter 必须与指针同一条翻转路径：{ev:?}"
    );
    assert_eq!(s.switches.get("wifi"), Some(&false));

    // ③ `Space`（= winit 的 Named(Space) ⇒ Key::Char(' ')，display.rs 的冻结映射）：
    //    与 Enter 完全同路。真实窗口流里 KeyDown 后还会跟 TextInput{" "}，
    //    但 Switch 不是值输入框 ⇒ 那条被忽略 ⇒ 不会双翻转。
    let ev = handle(&mut s, &tree, &geo, ClipSnapshot::unclipped(),
        &InputEvent::KeyDown { key: Key::Char(' '), mods: Mods::default(), repeat: false });
    assert_eq!(
        find(&ev, |e| matches!(e, UiEvent::Toggled { .. })),
        Some(&UiEvent::Toggled { id: "wifi".into(), on: true }),
        "Space 必须与 Enter 同一条翻转路径：{ev:?}"
    );
    let ev = handle(&mut s, &tree, &geo, ClipSnapshot::unclipped(),
        &InputEvent::TextInput { text: " ".into() });
    assert!(
        !ev.iter().any(|e| matches!(e, UiEvent::Toggled { .. })),
        "随后的 TextInput{{\" \"}} 不得再翻转一次（Switch 不是值输入框）：{ev:?}"
    );
    assert_eq!(s.switches.get("wifi"), Some(&true), "前置：此刻开关是开的");

    // ④ 初值是 App 的数据：先塞「开」，点击 ⇒ 翻成关（on=false）。
    let mut s2 = UiState::default();
    s2.switches.insert("wifi".into(), true);
    let ev = tap_at(&mut s2, &tree, &geo, "wifi");
    assert_eq!(
        find(&ev, |e| matches!(e, UiEvent::Toggled { .. })),
        Some(&UiEvent::Toggled { id: "wifi".into(), on: false }),
        "初值 on 的开关点击后翻成 off：{ev:?}"
    );
}

#[test]
fn disabled_switch_is_silent_and_scrub_num_stays_out_of_the_focus_order() {
    // 专用夹具：一个禁用的开关（disabled 的整棵子树静默 —— 既有规则原样生效）。
    let mut app = Builder::new(Kind::Column, "app").padding(8.0).gap(6.0);
    app.switch_opts("wifi", "Wi-Fi", |_| {});
    app.switch_opts("locked", "锁定", |n| n.props.disabled = true);
    let tree = app.build();
    let geo = geo_of(&tree);
    // 前置：夹具的第二个开关必须真的是禁用的。
    let locked = tree.children.iter().find(|c| c.id == "locked").expect("前置：locked 存在");
    assert!(locked.props.disabled, "测试前置：locked 必须禁用");

    let mut s = UiState::default();
    let ev = tap_at(&mut s, &tree, &geo, "locked");
    assert!(
        ev.iter().all(|e| !matches!(e, UiEvent::Clicked(_) | UiEvent::Toggled { .. })),
        "禁用开关必须静默：{ev:?}"
    );
    assert!(!s.switches.contains_key("locked"), "禁用开关不得写值表");

    // 焦点序：开关在、ScrubNum 刻意不在（拖动控件没有键盘语义，不占 Tab 一站）。
    let order = deer_gui::interaction::focusables(&tree);
    assert!(order.contains(&"wifi".to_string()), "开关必须在焦点序里：{order:?}");
    assert!(
        !order.contains(&"locked".to_string()),
        "禁用开关不得在焦点序里：{order:?}"
    );
    let with_scrub = values_app();
    assert!(
        !deer_gui::interaction::focusables(&with_scrub).contains(&"vol".to_string()),
        "ScrubNum 刻意不进焦点序：{:?}",
        deer_gui::interaction::focusables(&with_scrub)
    );
}

// ---------------------------------------------------------------------------
// 四、ColorField：提交解析 + 规范化回写
// ---------------------------------------------------------------------------

#[test]
fn color_field_commits_canonical_hex_and_rejects_the_rest() {
    let tree = values_app();
    let geo = geo_of(&tree);
    let mut s = UiState::default();

    focus_by_tap(&mut s, &tree, &geo, "tint");
    // `#` 可省、大小写都行（values.rs 的唯一解析实现）。
    handle(&mut s, &tree, &geo, ClipSnapshot::unclipped(),
        &InputEvent::TextInput { text: "FF8800".into() });
    let ev = handle(&mut s, &tree, &geo, ClipSnapshot::unclipped(), &key(Key::Enter));
    assert_eq!(
        find(&ev, |e| matches!(e, UiEvent::ColorChanged { .. })),
        Some(&UiEvent::ColorChanged { id: "tint".into(), rgb: [255, 136, 0] }),
        "提交成功必发 ColorChanged（三通道）：{ev:?}"
    );
    assert_eq!(s.texts.get("tint").map(String::as_str), Some("#ff8800"), "回写必须是 #rrggbb 规范形");

    // 失败档：#RGB 短式不做 ⇒ 不发事件、草稿保留。
    let mut s2 = UiState::default();
    focus_by_tap(&mut s2, &tree, &geo, "tint");
    handle(&mut s2, &tree, &geo, ClipSnapshot::unclipped(),
        &InputEvent::TextInput { text: "#fff".into() });
    let ev = handle(&mut s2, &tree, &geo, ClipSnapshot::unclipped(), &key(Key::Enter));
    assert!(
        !ev.iter().any(|e| matches!(e, UiEvent::ColorChanged { .. })),
        "#fff 必须被判为不可解析：{ev:?}"
    );
    assert_eq!(s2.texts.get("tint").map(String::as_str), Some("#fff"), "失败的提交不得动草稿");
}

// ---------------------------------------------------------------------------
// 五、Keep：值表是应用数据（重建保留且仍可用）
// ---------------------------------------------------------------------------

#[test]
fn keep_rebuild_preserves_value_tables() {
    let tree = values_app();
    let geo = geo_of(&tree);
    let mut s = UiState::default();
    s.num_opts.insert("age".into(), deer_gui::interaction::NumOpts {
        min: Some(0.0),
        max: Some(150.0),
        step: 1.0,
    });

    tap_at(&mut s, &tree, &geo, "wifi"); // wifi → 开
    focus_by_tap(&mut s, &tree, &geo, "age");
    handle(&mut s, &tree, &geo, ClipSnapshot::unclipped(),
        &InputEvent::TextInput { text: "30".into() });
    handle(&mut s, &tree, &geo, ClipSnapshot::unclipped(), &key(Key::Enter)); // 提交 30
    focus_by_tap(&mut s, &tree, &geo, "tint");
    handle(&mut s, &tree, &geo, ClipSnapshot::unclipped(),
        &InputEvent::TextInput { text: "#112233".into() });
    handle(&mut s, &tree, &geo, ClipSnapshot::unclipped(), &key(Key::Enter)); // 提交颜色
    assert_eq!(s.switches.get("wifi"), Some(&true), "前置：wifi=开");
    assert_eq!(s.texts.get("age").map(String::as_str), Some("30"), "前置：age=30");
    assert_eq!(s.texts.get("tint").map(String::as_str), Some("#112233"), "前置：tint 已规范化");

    // 整树重建（App 惯例）：UiState 原样带过去，值一个不丢。
    let rebuilt = values_app();
    assert!(rebuilt.structurally_eq(&tree), "前置：重建必须与原树结构相等（id 确定）");
    let geo2 = geo_of(&rebuilt);
    assert_eq!(s.switches.get("wifi"), Some(&true), "开关值保留");
    assert_eq!(s.texts.get("age").map(String::as_str), Some("30"), "数值草稿保留");
    assert_eq!(s.num_opts.get("age").map(|o| o.max), Some(Some(150.0)), "值域配置保留");

    // 仍可用：开关继续翻转、数值继续提交。
    let ev = tap_at(&mut s, &rebuilt, &geo2, "wifi");
    assert_eq!(
        find(&ev, |e| matches!(e, UiEvent::Toggled { .. })),
        Some(&UiEvent::Toggled { id: "wifi".into(), on: false }),
        "重建后开关仍可用（记得它开着 ⇒ 翻成关）"
    );
    focus_by_tap(&mut s, &rebuilt, &geo2, "age");
    handle(&mut s, &rebuilt, &geo2, ClipSnapshot::unclipped(),
        &InputEvent::TextInput { text: "999".into() });
    let ev = handle(&mut s, &rebuilt, &geo2, ClipSnapshot::unclipped(), &key(Key::Enter));
    assert_eq!(
        find(&ev, |e| matches!(e, UiEvent::NumberChanged { .. })),
        Some(&UiEvent::NumberChanged { id: "age".into(), value: 150.0 }),
        "重建后值域照常生效（999 → 150）"
    );
}

// ---------------------------------------------------------------------------
// 六、视觉判据：开关的开/关、不可解析下划线、色块颜色
// ---------------------------------------------------------------------------

/// 收集一份绘制列表里「矩形 == 给定节点矩形」的填充命令**颜色**（按出现序）。
fn fill_colors_over(list: &DrawList, id_rect: RectI) -> Vec<Color> {
    list.cmds
        .iter()
        .filter_map(|c| match c {
            DrawCmd::FillRoundRect { rect, color, .. } if *rect == id_rect => Some(*color),
            _ => None,
        })
        .collect()
}

fn rect_of(geo: &Geometry, id: &str) -> RectI {
    let f = geo.get(id).unwrap_or_else(|| panic!("测试前置：{id} 必须有几何"));
    RectI::new(f.x as i32, f.y as i32, f.w as i32, f.h as i32)
}

fn interactive_list(tree: &Node, geo: &Geometry, st: &UiState, theme: &Theme) -> DrawList {
    deer_gui::gpu::interact::InteractiveRenderer::with_texts(
        theme.clone(),
        &ApproxMeasure,
        &st.to_interact_state(),
        deer_gui::gpu::interact::FieldText::Content,
        &st.texts,
    )
    .build(tree, geo)
}

#[test]
fn switch_visual_differs_only_inside_the_switch_rect() {
    let tree = values_app();
    let geo = geo_of(&tree);
    let theme = Theme::default();

    // 前置：开关矩形有正面积（否则「差异只在框内」在测空气）。
    let r_wifi = rect_of(&geo, "wifi");
    assert!(r_wifi.w > 0 && r_wifi.h > 0, "前置：开关必须有正面积");

    let built = |st: &UiState| interactive_list(&tree, &geo, st, &theme);
    // ① 关（缺省）与显式 false：逐条相同（表里没有 = 关）。
    let mut off = UiState::default();
    off.switches.insert("wifi".into(), false);
    assert_eq!(built(&UiState::default()), built(&off), "缺省与显式关必须逐条相同");

    // ② 开：整帧差异必须全部落在开关矩形内（opt-in 红线：别的控件一个字节不动）。
    let mut on = UiState::default();
    on.switches.insert("wifi".into(), true);
    let l_off = built(&off);
    let l_on = built(&on);
    assert_ne!(l_off, l_on, "前置：开/关必须真的改变了绘制列表");
    let colors_off = fill_colors_over(&l_off, r_wifi);
    let colors_on = fill_colors_over(&l_on, r_wifi);
    assert!(!colors_off.is_empty(), "前置：开关至少要有一条填充命令");
    assert_ne!(
        colors_off, colors_on,
        "开/关的底色必须不同（关 = border 的 muted 档，开 = accent）"
    );
    assert_eq!(l_off.cmds.len(), l_on.cmds.len(), "开/关不得增删命令条数（只许换颜色）");
    // 像素级：差异只许落在 wifi 矩形内。
    let render = |list: &DrawList| {
        CpuRenderer::new()
            .render(
                Extent { width: CANVAS.0 as u32, height: CANVAS.1 as u32 },
                list,
                theme.surface,
            )
            .unwrap()
            .pixels
    };
    let px_off = render(&l_off);
    let px_on = render(&l_on);
    let stride = CANVAS.0 as usize * 4;
    for y in 0..CANVAS.1 as usize {
        for x in 0..CANVAS.0 as usize {
            let i = y * stride + x * 4;
            let inside = x >= r_wifi.x as usize
                && x < r_wifi.right() as usize
                && y >= r_wifi.y as usize
                && y < r_wifi.bottom() as usize;
            if !inside {
                assert_eq!(
                    px_off[i..i + 4],
                    px_on[i..i + 4],
                    "开关矩形外的像素不得因开/关而变（{x},{y}）"
                );
            }
        }
    }

    // ③ 杂散 id：把「开着」塞给一个**非 Switch** 节点 ⇒ 整帧逐字节不变
    //    （switches 表只被 `Kind::Switch` 读 —— opt-in 红线，与 5c 三张表同一条）。
    let mut stray = UiState::default();
    stray.switches.insert("button_1".into(), true);
    assert_eq!(built(&UiState::default()), built(&stray), "非 Switch 节点必须对开关表免疫");
}

#[test]
fn invalid_draft_underline_and_color_swatch_follow_the_displayed_text() {
    let tree = values_app();
    let geo = geo_of(&tree);
    let theme = Theme::default();

    // ① NumberField：草稿 "abc"（非空且不可解析）⇒ 文本区底部出现 1px 下划线（text_dim）；
    //    草稿 "42" ⇒ 没有。草稿住在 texts（FieldText::Content 口径）。
    let mut bad = UiState::default();
    bad.texts.insert("age".into(), "abc".into());
    let mut good = UiState::default();
    good.texts.insert("age".into(), "42".into());
    let l_bad = interactive_list(&tree, &geo, &bad, &theme);
    let l_good = interactive_list(&tree, &geo, &good, &theme);
    let underline_count = |list: &DrawList| {
        list.cmds
            .iter()
            .filter(|c| matches!(c, DrawCmd::FillRoundRect { rect, color, radius: 0 }
                if rect.h == deer_gui::gpu::interact::INVALID_UNDERLINE_H
                    && *color == theme.text_dim
                    && rect.x >= rect_of(&geo, "age").x
                    && rect.right() <= rect_of(&geo, "age").right()
                    && rect.y >= rect_of(&geo, "age").y
                    && rect.bottom() <= rect_of(&geo, "age").bottom()))
            .count()
    };
    assert!(underline_count(&l_bad) >= 1, "不可解析草稿必须画出下划线标记");
    assert_eq!(underline_count(&l_good), 0, "可解析草稿不得画标记（空草稿同理，缺省态也没有）");

    // ② ColorField：合法 hex ⇒ 色块 = 该颜色；非法 ⇒ 色块退回 border 色（与壳同色）。
    //    色块是节点矩形内的一块**小**填充（右侧正方形）—— 按「矩形 ≠ 节点矩形且完全
    //    落在节点矩形内」识别（整个矩形的那条填充是壳的底色，1px 描边不是填充）。
    let r_tint = rect_of(&geo, "tint");
    let inside_fills = |list: &DrawList| -> Vec<(RectI, Color)> {
        list.cmds
            .iter()
            .filter_map(|c| match c {
                DrawCmd::FillRoundRect { rect, color, .. }
                    if *rect != r_tint
                        && rect.x >= r_tint.x
                        && rect.y >= r_tint.y
                        && rect.right() <= r_tint.right()
                        && rect.bottom() <= r_tint.bottom() =>
                {
                    Some((*rect, *color))
                }
                _ => None,
            })
            .collect()
    };
    let mut ok = UiState::default();
    ok.texts.insert("tint".into(), "#112233".into());
    let mut ng = UiState::default();
    ng.texts.insert("tint".into(), "nope".into());
    let f_ok = inside_fills(&interactive_list(&tree, &geo, &ok, &theme));
    let f_ng = inside_fills(&interactive_list(&tree, &geo, &ng, &theme));
    // 前置：识别到的「小填充」恰好一块（= 色块），且它是正方形。
    assert_eq!(f_ok.len(), 1, "前置：节点矩形内应恰好识别出一块小填充（色块），实际 {f_ok:?}");
    assert_eq!(f_ok[0].0.w, f_ok[0].0.h, "前置：色块必须是正方形，实际 {:?}", f_ok[0].0);
    let expected = Color::rgb(0x11, 0x22, 0x33);
    assert_eq!(f_ok[0].1, expected, "合法 hex 的色块必须是该颜色（{expected:?}）");
    assert_eq!(f_ng[0].1, theme.border, "非法草稿的色块必须退回 border 色，实际 {:?}", f_ng);

    // ③ DefaultRenderer（无状态）：NumberField = 普通输入框；ColorField 的色块从 label
    //    解析（树数据，无状态可画）—— label 合法 ⇒ 色块颜色出现。
    let d = deer_gui::gpu::render::DefaultRenderer::new(theme, &ApproxMeasure).build(&tree, &geo);
    let mut d_swatch = Vec::new();
    for c in &d.cmds {
        if let DrawCmd::FillRoundRect { rect, color, .. } = c {
            if *rect != r_tint
                && rect.x >= r_tint.x
                && rect.y >= r_tint.y
                && rect.right() <= r_tint.right()
                && rect.bottom() <= r_tint.bottom()
            {
                d_swatch.push(*color);
            }
        }
    }
    assert_eq!(d_swatch.len(), 1, "前置：DefaultRenderer 的色块应恰好一块，实际 {d_swatch:?}");
    assert_eq!(
        d_swatch[0],
        Color::rgb(0xff, 0x88, 0x00),
        "DefaultRenderer 的色块必须从 label（#ff8800）解析出颜色"
    );
}

// ---------------------------------------------------------------------------
// 七、testkit 档：注入路径
// ---------------------------------------------------------------------------

#[cfg(feature = "testing")]
mod via_testkit {
    use deer_gui::interaction::{InputEvent, Key, Mods, UiEvent, UiState};
    use deer_gui::prelude::*;
    use deer_gui::testing::{Harness, Repro};

    use super::values_app;

    /// testkit 注入路径的全量语义（与纯逻辑档同源，这里钉「Harness 注入也能到达」）。
    #[test]
    fn values_semantics_via_testkit() -> Result<(), String> {
        let tree = values_app();
        let mut h = Harness::new(tree, 260, 220, Theme::default());
        h.set_case("m6_values");
        h.set_repro(Repro::test("deer-gui", "testing", "m6_values", "values_semantics", &[]));
        h.require_ids(&["button_1", "age", "vol", "wifi", "tint", "field_1"]);

        // ① 点击开关：翻转 + 聚焦。
        let step = h.tap("wifi")?;
        assert!(
            step.events.contains(&UiEvent::Toggled { id: "wifi".into(), on: true }),
            "点开关要发 Toggled{{on:true}}：{:?}",
            step.events
        );
        h.assert_focus(Some("wifi"))?;

        // ② 点数值框 → 打字 → Enter 提交：草稿、事件、规范化回写三样都要到位。
        let step = h.tap("age")?;
        assert!(step.events.iter().any(|e| matches!(e, UiEvent::FocusChanged(Some(id)) if id == "age")));
        h.send(&InputEvent::TextInput { text: "42".into() })?;
        h.assert_text("age", "42")?;
        let step = h.send(&InputEvent::KeyDown { key: Key::Enter, mods: Mods::default(), repeat: false })?;
        assert!(
            step.events.contains(&UiEvent::NumberChanged { id: "age".into(), value: 42.0 }),
            "Enter 提交要发 NumberChanged{{42}}：{:?}",
            step.events
        );
        h.assert_text("age", "42")?;

        // ③ 拖动调值：down 在中心 → move +5px ⇒ NumberChanged{{45}}（step 缺省 1.0）。
        let (x0, y0) = h.center_of("vol")?;
        h.send(&InputEvent::PointerMoved { x: x0, y: y0 })?;
        h.send(&InputEvent::PointerDown {
            button: deer_gui::interaction::PointerButton::Left, x: x0, y: y0,
        })?;
        let step = h.send(&InputEvent::PointerMoved { x: x0 + 5.0, y: y0 })?;
        assert!(
            step.events.contains(&UiEvent::NumberChanged { id: "vol".into(), value: 45.0 }),
            "拖 +5px 要发 NumberChanged{{45}}：{:?}",
            step.events
        );
        assert!(h.state().scrub.is_some(), "前置：拖动中锚点在手");
        h.send(&InputEvent::PointerUp {
            button: deer_gui::interaction::PointerButton::Left, x: x0 + 5.0, y: y0,
        })?;
        assert!(h.state().scrub.is_none(), "抬起后锚点必须清空");

        // ④ 颜色框：打合法 hex → Enter ⇒ ColorChanged + 规范化回写。
        h.tap("tint")?;
        h.send(&InputEvent::TextInput { text: "AABBCC".into() })?;
        let step = h.send(&InputEvent::KeyDown { key: Key::Enter, mods: Mods::default(), repeat: false })?;
        assert!(
            step.events.contains(&UiEvent::ColorChanged { id: "tint".into(), rgb: [0xaa, 0xbb, 0xcc] }),
            "颜色提交要发 ColorChanged：{:?}",
            step.events
        );
        h.assert_text("tint", "#aabbcc")?;
        Ok(())
    }

    /// Harness 帧（`build_with` 已带开关表）：开着的开关与关着的开关必须真的画出差异，
    /// 且差异只落在开关矩形内 —— 这是「值表 → 像素」在 testkit 路径上的闭环。
    #[test]
    fn switch_value_reaches_the_pixels_via_harness() -> Result<(), String> {
        let tree = values_app();
        let mut h = Harness::new(tree, 260, 220, Theme::default());
        h.set_case("m6_values_visual");
        h.require_ids(&["wifi"]);

        h.set_state(UiState::default());
        let off = h.shoot()?;
        let mut on = UiState::default();
        on.switches.insert("wifi".into(), true);
        h.set_state(on);
        let opened = h.shoot()?;

        let f = h.frame()?;
        let r = f.rect_of("wifi").expect("前置：wifi 有几何");
        opened.assert_no_diff_outside(&off, &[r])?;
        Ok(())
    }
}
