//! **M5-4 的回归**：脚本化重放 + dirty 重绘 + 裁剪感知命中 + 四档状态的像素证明。
//!
//! 与 `examples/interactive_form.rs` 的分工：example 是**真窗口**那一路（跑不进 `cargo test`，
//! winit 要求事件循环在主线程）；这里跑的是**同一条链的离屏版本** ——
//! 真实布局、真实绘制列表（含 `NodeHint`）、真实 `interaction::handle`、真实 CPU/GPU 像素。
//!
//! ```sh
//! cargo test -q -p deer-gui --test interactive_form
//! ```
//!
//! ## 判据一览
//!
//! | 测试 | 判据 |
//! |---|---|
//! | `scripted_replay_is_deterministic_and_matches_expected_state` | 脚本 → 事件 → 状态：终态逐字段相等 + 重放两次逐条相同 + dirty 序列 |
//! | `a_pointer_press_that_changes_nothing_does_not_set_dirty` | 点空白处：**没有事件**、状态却从 `None`→`None` 无变化 ⇒ `dirty=false`（「收到事件就重绘」是错的判据） |
//! | `hits_respect_the_clip_snapshot_from_the_real_draw_list` | 快照**非空**（前置）；同一脚本在「输入框被裁掉」的快照下拿不到事件、在未裁快照下拿得到（反向自检） |
//! | `four_states_differ_only_inside_the_expected_rectangles` | 四档状态两两像素差异**全部**落在期望矩形内，越界字节 = 0；每档矩形内差异 ≥ 登记下限；**焦点环再加一条「平均通道差 ≥ [`FOCUS_MIN_CONTRAST`]」**（细带的可见性靠对比度，不靠面积） |
//! | `four_states_match_the_cpu_backend_offline` | 离屏 GPU vs CPU：**逐字节相同**（不透明语料） |
//!
//! 前置条件（每一条都**显式断言**，因为前置不成立时护栏会**静默失效**）：
//! 快照里有 `button_1`/`field_1`；`field_1` 的中心确实在裁剪外／内；像素差异确实 > 0。

use std::collections::BTreeMap;

use deer_gpu::interact::{
    FILL_ROUND_RADIUS, FOCUS_RING_INSET, FOCUS_STROKE_WIDTH, FieldText, InteractState,
    InteractiveRenderer,
};
use deer_gui::gpu::null::CpuRenderer;
use deer_core::{Color, DrawList};
use deer_gpu::{Extent, Theme};
// LY2：文本栈来自 L1 crate `deer-text`。
use deer_text::TextEngine;
use deer_gui::input_script;
use deer_gui::interaction::{ClipSnapshot, UiEvent, UiState, focusables};
use deer_gui::layout::builder::{Builder, L};
use deer_gui::layout::layout::{self, Geometry, TextStyle};
use deer_gui::layout::node::{Kind, Node, Rect};
use deer_gui::prelude::*;
use deer_gui::vk::GpuGeometryRenderer;

const W: u32 = 420;
const H: u32 = 220;
const CLEAR: Color = Color::rgb(0x08, 0x09, 0x0c);

// ---------------------------------------------------------------------------
// 语料
// ---------------------------------------------------------------------------

/// 与 example 同构的界面树（id 由 `IdGen` 按树序生成 ⇒ 断言里可以写死）。
fn app_tree() -> Node {
    let mut app = Builder::new(Kind::Column, "app").padding(12.0).gap(10.0);
    app.text("M5-4 interactive form");
    app.container_opts(Kind::Row, "row", L::new().gap(8.0).to_props(), |r| {
        r.button("OK");
        r.field("name");
    });
    app.button_opts("disabled", |n| n.props.disabled = true);
    app.build()
}

fn theme() -> Theme {
    Theme::default()
}

fn geo_of(tree: &Node, theme: &Theme) -> Geometry {
    layout::layout(
        tree,
        Rect::new(0.0, 0.0, W as f32, H as f32),
        TextStyle {
            font_size: theme.font_size,
            line_height: theme.line_height,
        },
        &ApproxMeasure,
    )
}

/// 一帧：状态 + 文本缓冲 ⇒ 绘制列表 + 裁剪快照。
struct Frame {
    list: DrawList,
    clip: ClipSnapshot,
}

fn frame_with(tree: &Node, theme: &Theme, geo: &Geometry, state: &UiState) -> Frame {
    let interact = InteractState {
        hover: state.hover.clone(),
        focus: state.focus.clone(),
        pressed: state.pressed.clone(),
        scroll: Default::default(),
        carets: Default::default(),
        preedit: None,
    };
    let list = InteractiveRenderer::with_texts(
        theme.clone(),
        &ApproxMeasure,
        &interact,
        FieldText::Content,
        &state.texts,
    )
    .build(tree, geo);
    let clip = ClipSnapshot::from_draw_list(&list, tree, geo);
    Frame { list, clip }
}

fn center(g: &Geometry, id: &str) -> (f32, f32) {
    let r = g
        .get(id)
        .copied()
        .unwrap_or_else(|| panic!("测试前置：布局必须给 {id} 几何"));
    (r.x + r.w / 2.0, r.y + r.h / 2.0)
}

fn rect_i(g: &Geometry, id: &str) -> RectI {
    let r = g.get(id).copied().unwrap_or_else(|| panic!("测试前置：{id} 没有几何"));
    RectI::new(r.x as i32, r.y as i32, r.w as i32, r.h as i32)
}

fn fmt_state(s: &UiState) -> String {
    format!(
        "hover={:?} focus={:?} pressed={:?} texts={:?}",
        s.hover, s.focus, s.pressed, s.texts
    )
}

// ---------------------------------------------------------------------------
// 一、脚本化重放（纯逻辑；真实布局 + 真实列表派生的裁剪快照）
// ---------------------------------------------------------------------------

/// 脚本用的界面与断言（**先打印真实数据，再定期望值**）。
#[test]
fn scripted_replay_is_deterministic_and_matches_expected_state() {
    let theme = theme();
    let tree = app_tree();
    let geo = geo_of(&tree, &theme);
    let base = frame_with(&tree, &theme, &geo, &UiState::default());

    // 前置①：真实绘制列表确实发得出 NodeHint，且快照里有两个控件。
    println!(
        "列表：{} 条命令（NodeHint {}）｜快照 {} 个节点",
        base.list.len(),
        base.list.counts().node_hint,
        base.clip.len()
    );
    assert!(
        base.list.counts().node_hint > 0,
        "护栏前置：这一帧必须发 NodeHint（否则裁剪快照恒为空，裁剪判据静默失效）"
    );
    for id in ["app", "row", "button_1", "field_1", "button_2", "text_1"] {
        assert!(geo.contains_key(id), "测试前置：{id} 必须有几何");
        assert!(
            base.clip.is_known(id),
            "护栏前置：{id} 必须在裁剪快照里（否则它那一条裁剪断言没有判别力）"
        );
        assert_eq!(base.clip.clip_of(id), None, "{id} 不该被裁剪");
    }
    // 前置②：焦点序列（`Tab` 的落点由它决定；禁用按钮必须不在里面）。
    println!("焦点树序 = {:?}", focusables(&tree));
    assert_eq!(
        focusables(&tree),
        vec!["button_1".to_string(), "field_1".into()],
        "禁用按钮必须不在焦点序列里"
    );

    // 脚本（坐标由布局算出来 —— 不写死，布局一变脚本跟着变）。
    //
    // 顺序刻意选成「每个动词各司其职」，这样每一步的期望都能一眼读出来：
    // `move+down/up` 点在按钮上 ⇒ hover + 点击聚焦 + `Clicked`；`key:Tab` ⇒ 焦点挪到输入框；
    // `move` 到输入框 ⇒ hover；`text:hi` ⇒ 焦点在输入框 ⇒ 缓冲变成 `hi`。
    let (bx, by) = center(&geo, "button_1");
    let (fx, fy) = center(&geo, "field_1");
    let src = format!("move:{bx},{by};down:left;up:left;key:Tab;move:{fx},{fy};text:hi");
    println!("脚本：{src}");

    let r1 = input_script::replay(&src, &tree, &geo, base.clip.clone()).expect("脚本必须合法");
    println!("事件日志：");
    for (i, e) in r1.events.iter().enumerate() {
        println!("  {i}: {e:?}");
    }
    println!("真实终态 = {}", fmt_state(&r1.state));
    println!(
        "dirty 序列 = {:?}（重绘 {} 次 / {} 步）",
        r1.redrew,
        r1.redraw_count(),
        r1.redrew.len()
    );

    let expected_events = vec![
        UiEvent::HoverChanged(Some("button_1".into())),
        UiEvent::FocusChanged(Some("button_1".into())),
        UiEvent::Clicked("button_1".into()),
        UiEvent::FocusChanged(Some("field_1".into())),
        UiEvent::HoverChanged(Some("field_1".into())),
        UiEvent::TextChanged {
            id: "field_1".into(),
            value: "hi".into(),
        },
    ];
    println!("期望事件序列 = {expected_events:?}");
    assert_eq!(r1.events, expected_events, "事件序列必须逐条相等");

    let expected_state = UiState {
        hover: Some("field_1".into()),
        focus: Some("field_1".into()),
        pressed: None,
        texts: BTreeMap::from([("field_1".to_string(), "hi".to_string())]),
        // 打完 "hi" 光标停在末尾（2 个字符）—— 这张表现在会被显式写上（值仍是「追加」的值）。
        carets: BTreeMap::from([("field_1".to_string(), 2usize)]),
        scroll: Default::default(),
        preedit: None,
    };
    println!("期望终态 = {}", fmt_state(&expected_state));
    assert_eq!(r1.state, expected_state, "终态必须逐字段相等");

    // dirty 序列：每一步都必须**真的**改了状态（这条脚本里没有空转事件）。
    assert!(
        r1.redrew.iter().all(|d| *d),
        "这条脚本的每一步都该改变状态 ⇒ 每一步都该置脏，实际 {:?}",
        r1.redrew
    );
    assert_eq!(r1.redraw_count(), 6, "6 条事件 ⇒ 6 次重绘");

    // 可回归：同一串脚本再放一遍 ⇒ 逐条相同、终态相同、dirty 序列相同。
    let r2 = input_script::replay(&src, &tree, &geo, base.clip.clone()).expect("脚本必须合法");
    assert_eq!(r1, r2, "重放必须确定性（事件、终态、dirty 序列都要一样）");
    println!("第二次重放与第一次逐条相同 ✅");
}

/// **dirty 的判据是状态、不是事件**：点空白处 ⇒ `hover` 从 `Some` 变 `None`（有事件），
/// 而「已经悬停在空白上」再点一次 ⇒ **没有事件**、状态也没变 ⇒ **不置脏**。
#[test]
fn a_pointer_press_that_changes_nothing_does_not_set_dirty() {
    let theme = theme();
    let tree = app_tree();
    let geo = geo_of(&tree, &theme);
    let base = frame_with(&tree, &theme, &geo, &UiState::default());

    // 一个既不在任何控件上、也不在容器上的点（容器 app 从 (0,0) 开始铺满，所以用负坐标）。
    let src = "move:-5,-5;down:left;up:left";
    let r = input_script::replay(src, &tree, &geo, base.clip.clone()).expect("脚本必须合法");
    println!("脚本 `{src}`");
    println!("事件日志 = {:?}", r.events);
    println!("终态 = {}", fmt_state(&r.state));
    println!("dirty 序列 = {:?}", r.redrew);

    // 前置：这三次事件确实**一次都没**改变状态（否则下面「不置脏」的断言没有判别力）。
    assert!(
        r.events.is_empty(),
        "点空白处不该产生 UI 事件，实际 {:?}",
        r.events
    );
    assert_eq!(
        r.state,
        UiState::default(),
        "点空白处不该改变任何状态（`pressed` 也是 None→None）"
    );
    assert_eq!(
        r.redrew,
        vec![false, false, false],
        "状态没变 ⇒ 一步都不该置脏（-5,-5 不在任何节点上）"
    );

    // 反向自检：**同样的按键序列**打在一个真控件上 ⇒ 有事件、且**置脏**。
    let (bx, by) = center(&geo, "button_1");
    let src2 = format!("move:{bx},{by};down:left;up:left");
    let r2 = input_script::replay(&src2, &tree, &geo, base.clip.clone()).expect("脚本必须合法");
    println!("脚本 `{src2}` ⇒ 事件 {:?}｜dirty {:?}", r2.events, r2.redrew);
    assert!(!r2.events.is_empty(), "落在按钮上必须产生事件");
    assert!(
        r2.redrew.iter().all(|d| *d),
        "落在按钮上每一步都在改状态 ⇒ 每步都该置脏，实际 {:?}",
        r2.redrew
    );
}

/// 命中必须受**真实绘制列表派生的裁剪快照**约束：把输入框裁掉之后，打在它中心的点
/// 拿不到任何事件；而同一脚本在**未裁剪**的快照下拿得到（否则「没事件」可能只是整体哑了）。
#[test]
fn hits_respect_the_clip_snapshot_from_the_real_draw_list() {
    let theme = theme();
    let tree = app_tree();
    let geo = geo_of(&tree, &theme);
    let base = frame_with(&tree, &theme, &geo, &UiState::default());

    let (fx, fy) = center(&geo, "field_1");
    let (bx, by) = center(&geo, "button_1");
    println!("button_1 中心 = ({bx},{by})｜field_1 中心 = ({fx},{fy})");

    // 构造一份「把 field_1 裁掉」的列表：`field_1` 的几何中心必须落在裁剪区外。
    let clip_rect = RectI::new(0, 0, 60, 40);
    assert!(
        !clip_rect.contains(fx as i32, fy as i32),
        "测试前置：field_1 的中心必须在裁剪区 {clip_rect:?} 之外（否则这个测试在测空气）"
    );
    let clipped_list = {
        let mut l = DrawList::new();
        for cmd in &base.list.cmds {
            match cmd {
                // 在 `field_1`（以及它之后的节点）之前推入裁剪 —— 位置由 `NodeHint` 的
                // 出现次数定位，不依赖命令种类的巧合。
                DrawCmd::NodeHint { .. } => {
                    let seen = l
                        .cmds
                        .iter()
                        .filter(|c| matches!(c, DrawCmd::NodeHint { .. }))
                        .count();
                    if seen == 3 {
                        l.push(DrawCmd::PushClip { rect: clip_rect });
                    }
                    l.push(cmd.clone());
                }
                other => l.push(other.clone()),
            }
        }
        l.push(DrawCmd::PopClip);
        l
    };
    let clipped = ClipSnapshot::from_draw_list(&clipped_list, &tree, &geo);
    println!(
        "裁剪后快照：field_1 的裁剪 = {:?}｜button_1 的裁剪 = {:?}",
        clipped.clip_of("field_1"),
        clipped.clip_of("button_1")
    );
    // 前置①：快照里**真的**登记了 field_1 的裁剪（否则「不命中」可能只是因为 id 不在快照里）。
    assert!(
        clipped.is_known("field_1"),
        "护栏前置：裁剪快照必须登记 field_1"
    );
    assert_eq!(
        clipped.clip_of("field_1"),
        Some(clip_rect),
        "field_1 的裁剪必须是我们推入的那个矩形"
    );
    // 前置②：几何上那个点确实命中 field_1（所以拒绝只能来自裁剪）。
    assert_eq!(
        deer_gui::layout::hit_test(&tree, &geo, fx, fy).map(|n| n.id.as_str()),
        Some("field_1"),
        "测试前置：几何上命中的是 field_1"
    );

    // ① 裁剪快照：同一脚本 ⇒ 拿不到任何事件，状态不动。
    //
    //    脚本顺序有讲究：`key:Tab` **先**把焦点给 `button_1`，之后点 `field_1` 才把焦点
    //    挪到被裁掉的那个输入框上（顺序反过来的话，`key:Tab` 会把焦点从 field_1 挪到
    //    button_1 ⇒ 文本输入打在按钮上，一个字符都进不来 —— 那测的就不是裁剪了）。
    let src = format!("key:Tab;move:{fx},{fy};down:left;up:left;text:hi");
    println!(
        "脚本：{src}（先 Tab 再用指针去点 field_1；后者在【不裁剪】时应当生效、在【裁剪】时不生效）"
    );
    let rc = input_script::replay(&src, &tree, &geo, clipped.clone()).expect("脚本必须合法");
    println!("[被裁剪] 事件 = {:?}｜终态 = {}", rc.events, fmt_state(&rc.state));
    for e in &rc.events {
        println!("  {e:?}");
    }
    assert!(
        !rc.events
            .iter()
            .any(|e| matches!(e, UiEvent::HoverChanged(Some(id)) if id == "field_1")),
        "被裁掉的输入框不该拿到 hover，实际 {:?}",
        rc.events
    );
    assert!(
        !rc.state.texts.contains_key("field_1"),
        "被裁掉的输入框不该吃文本输入，实际 {:?}",
        rc.state.texts
    );

    // ② 反向自检：未裁剪 ⇒ 同一脚本拿到 hover + 文本。
    let ru = input_script::replay(&src, &tree, &geo, base.clip.clone()).expect("脚本必须合法");
    println!("[未裁剪] 事件 = {:?}｜终态 = {}", ru.events, fmt_state(&ru.state));
    assert_eq!(
        ru.state.hover.as_deref(),
        Some("field_1"),
        "未裁剪时必须命中（否则上一条的「不命中」可能只是整体哑了）"
    );
    assert_eq!(
        ru.state.texts.get("field_1").map(String::as_str),
        Some("hi"),
        "未裁剪时点中输入框之后，文本输入必须落在它身上"
    );
    assert_eq!(
        ru.state.focus.as_deref(),
        Some("field_1"),
        "未裁剪时点击可聚焦控件 ⇒ 它拿到焦点（M5-4 的点击聚焦）"
    );

    // ③ 命中判据本身（不经状态机）：未裁剪命中、裁剪后不命中。
    assert_eq!(
        deer_gui::interaction::hit(&tree, &geo, base.clip.clone(), fx, fy).map(|n| n.id.clone()),
        Some("field_1".to_string())
    );
    assert_eq!(
        deer_gui::interaction::hit(&tree, &geo, clipped.clone(), fx, fy),
        None,
        "裁剪外的点不命中"
    );
    // ④ 裁剪区内仍然有东西能命中（否则「不命中」可能只是快照把一切都拒绝了）。
    let inside = (4.0f32, 4.0f32);
    println!(
        "裁剪区内的点 {inside:?} ⇒ {:?}",
        deer_gui::interaction::hit(&tree, &geo, clipped.clone(), inside.0, inside.1).map(|n| n.id.clone())
    );
    assert!(
        deer_gui::interaction::hit(&tree, &geo, clipped, inside.0, inside.1).is_some(),
        "裁剪区内的点必须还能命中"
    );
}

// ---------------------------------------------------------------------------
// 二、四档状态的像素证明
// ---------------------------------------------------------------------------

/// 四档状态的列表（配同一份几何 ⇒ 可直接逐像素比较）。
fn states(theme: &Theme, tree: &Node, geo: &Geometry) -> Vec<(&'static str, Frame, Vec<RectI>)> {
    // 「允许变化的矩形」= 被驱动状态的控件（button_1）与显示文本的控件（field_1）。
    // 两个控件的几何由调用方给（GPU 用例用的是**真实字体度量**的那份，不是近似度量）。
    let allowed = vec![rect_i(geo, "button_1"), rect_i(geo, "field_1")];
    let mut texts = BTreeMap::new();
    texts.insert("field_1".to_string(), "hi".to_string());

    // 前置：两个矩形的中心必须**分别**落在自己身上（否则下面「差异落在矩形内」的
    // 断言可能只是因为这两个矩形空了）。
    for id in ["button_1", "field_1"] {
        let (x, y) = center(geo, id);
        println!("  [{id}] 中心 ({x},{y}) ＝ 矩形 {:?}", rect_i(geo, id));
    }
    let mut out = Vec::new();
    let mut push = |name: &'static str, st: UiState| {
        let f = frame_with(tree, theme, geo, &st);
        out.push((name, f, allowed.clone()));
    };
    push("idle", UiState::default());
    push(
        "hover",
        UiState {
            hover: Some("button_1".into()),
            ..Default::default()
        },
    );
    push(
        "pressed",
        UiState {
            hover: Some("button_1".into()),
            pressed: Some("button_1".into()),
            ..Default::default()
        },
    );
    push(
        "focused",
        UiState {
            focus: Some("button_1".into()),
            ..Default::default()
        },
    );
    push(
        "typed",
        UiState {
            hover: Some("button_1".into()),
            focus: Some("field_1".into()),
            pressed: None,
            texts,
            carets: Default::default(),
            scroll: Default::default(),
            preedit: None,
        },
    );
    out
}

/// 每个**视觉命令**都必须落在「它前面最近一个 `NodeHint` 的矩形」之内。
///
/// 这是「状态差异只在期望矩形内」的**结构性**前提，比逐像素更早、更容易定位：
/// 若某个命令跑出了它自己的节点矩形（例如把焦点描边画到外面），像素判据会越界，
/// 而这里能直接指出是哪一条命令。
fn command_bounds_ok(name: &str, f: &Frame) -> Result<(), String> {
    let mut current: Option<RectI> = None;
    let mut node_index = 0usize;
    for cmd in &f.list.cmds {
        match cmd {
            DrawCmd::NodeHint { rect, .. } => {
                current = Some(*rect);
                node_index += 1;
            }
            DrawCmd::FillRect { rect, .. }
            | DrawCmd::StrokeRect { rect, .. }
            | DrawCmd::FillRoundRect { rect, .. }
            | DrawCmd::Text { rect, .. } => {
                let Some(owner) = current else {
                    return Err(format!(
                        "[{name}] 第 {node_index} 个节点之前就有绘制命令 ⇒ 列表结构不对（提示必须先发）"
                    ));
                };
                let inside = rect.x >= owner.x
                    && rect.y >= owner.y
                    && rect.right() <= owner.right()
                    && rect.bottom() <= owner.bottom();
                if !inside {
                    return Err(format!(
                        "[{name}] 第 {node_index} 个节点的命令矩形 {rect:?} 落在它自己的矩形 {owner:?} 之外"
                    ));
                }
            }
            DrawCmd::PushClip { .. } | DrawCmd::PopClip => {}
        }
    }
    Ok(())
}

#[test]
fn four_states_differ_only_inside_the_expected_rectangles() {
    let theme = theme();
    let tree = app_tree();
    let geo = geo_of(&tree, &theme);
    let ext = Extent {
        width: W,
        height: H,
    };
    let cases = states(&theme, &tree, &geo);
    assert_eq!(cases.len(), 5, "测试前置：一共五档状态");

    // 前置：裁剪快照非空且两个控件都在里面（裁掉的点会**静默**影响命中）。
    for (name, f, allowed) in &cases {
        assert!(!f.clip.is_empty(), "[{name}] 快照不能为空");
        assert!(f.clip.is_known("button_1"), "[{name}] button_1 必须在快照里");
        assert!(f.clip.is_known("field_1"), "[{name}] field_1 必须在快照里");
        assert_eq!(allowed.len(), 2, "允许变化的矩形：button_1 与 field_1");
        command_bounds_ok(name, f).unwrap_or_else(|e| panic!("{e}"));
    }

    let render = |f: &Frame| {
        CpuRenderer::new()
            .render(ext, &f.list, CLEAR)
            .expect("CPU 渲染失败")
            .pixels
    };
    let px: Vec<(&str, Vec<u8>)> = cases.iter().map(|(n, f, _)| (*n, render(f))).collect();
    println!();
    println!("—— 四档状态的像素证明（CPU 后端，{W}×{H}）——");

    for (name, f, _allowed) in &cases {
        let mine = &px.iter().find(|(n, _)| n == name).expect("刚渲染过").1;
        let idle = &px.iter().find(|(n, _)| *n == "idle").expect("idle 在列表里").1;
        if *name == "idle" {
            continue;
        }
        let btn = rect_i(&geo, "button_1");
        let field = rect_i(&geo, "field_1");
        let (in_btn, _) = diff_split(idle, mine, ext, btn);
        let (in_field, _) = diff_split(idle, mine, ext, field);
        let total = mine.iter().zip(idle.iter()).filter(|(a, b)| a != b).count();
        let outside = total - in_btn - in_field;
        let px_btn = diff_pixels(idle, mine, ext, btn);
        let px_field = diff_pixels(idle, mine, ext, field);
        println!(
            "  {name:<8} 与 idle 的差异：共 {total} 字节（按钮内 {in_btn} 字节/{px_btn} px｜输入框内 {in_field} 字节/{px_field} px｜**矩形外 {outside}**）\
             ｜命令 {} 条（hint {}）",
            f.list.len(),
            f.list.counts().node_hint
        );
        assert_eq!(
            outside, 0,
            "{name}: 有 {outside} 个字节的差异落在 button_1 {btn:?} 与 field_1 {field:?} **之外** \
             ⇒ 状态视觉溢出到别的控件上了"
        );
        // 这一档必须**真的**画出了**够多**的东西 —— 判据是「差异 ≥ 明确下限」，**不是**「差异 > 0」。
        //
        // 为什么「> 0」不够：按钮的焦点环改前与填充**同色**（`tint(accent, Idle) == accent`），
        // 差异仍有 144 字节 / 56 px（32 px 方角补角 + 24 px 描边压字形）⇒ 「focused 有差异」一直绿着，
        // 而屏幕上根本没有焦点环（独立复审 F-2）。各档下限的出处见 `STATE_MIN_PX`。
        let expected_region = if *name == "typed" { px_field } else { px_btn };
        let floor = STATE_MIN_PX
            .iter()
            .find(|(n, _)| n == name)
            .unwrap_or_else(|| panic!("状态 `{name}` 没有登记下限 —— 新增状态必须显式给下限，否则它在空转"))
            .1;
        println!("           └ 期望区域 {expected_region} px｜下限 {floor}");
        assert!(
            expected_region >= floor,
            "{name}: 期望变化的那个矩形里只有 {expected_region} px 差异 < 下限 {floor} \
             ⇒ 这一档状态视觉等于没画出来（「> 0」抓不住这种情况）"
        );
    }

    // ---- 焦点环判据（两条下限）+ 反向自检 ----
    //
    // 焦点环的判据是**两条**，缺一不可：
    //   ① 按钮内差异像素数 ≥ [`FOCUS_MIN_PX`]（「环有没有**被画出来**」）；
    //   ② 这些差异像素的**平均通道差** ≥ [`FOCUS_MIN_CONTRAST`]（「画出来了**看不看得出**」）。
    //
    // 为什么要第二条（本次补的漏洞，M5b 终审 Minor 1）：环是一条**细带**，它「看得见」靠的是
    // **对比度**而不是面积。只数像素数时，「填充色提亮 5%」这种**技术上不同色、肉眼看不出**的环
    // **照样**改掉整条环带（像素数一个不少）⇒ `deer-gpu` 单测当场红、而这条 parity 用例仍然绿。
    //
    // 反向自检是**同一次运行里真把环色换掉**（几何、宽度、其他命令一字不动）：两种坏环各撞一条下限，
    // 判据必须两次都拒绝 —— 不拒绝就说明这条下限抓不住本次要修的缺陷，等于没加。
    let focused_frame = cases
        .iter()
        .find(|(n, _, _)| *n == "focused")
        .expect("focused 在列表里");
    let btn = rect_i(&geo, "button_1");
    let band = RectI::new(
        btn.x + FOCUS_RING_INSET,
        btn.y + FOCUS_RING_INSET,
        btn.w - 2 * FOCUS_RING_INSET,
        btn.h - 2 * FOCUS_RING_INSET,
    );
    // 前置①：焦点环**恰好一条**、矩形就是内缩那一圈、宽度就是 `FOCUS_STROKE_WIDTH`
    //（判据的落点由它决定，错了下面全在测空气）。
    let mut owner: Option<RectI> = None;
    let mut fill = None;
    let mut ring_index = None;
    for (i, cmd) in focused_frame.1.list.cmds.iter().enumerate() {
        match cmd {
            DrawCmd::NodeHint { rect, .. } => owner = Some(*rect),
            DrawCmd::FillRoundRect { color, .. } if owner == Some(btn) => fill = Some(*color),
            DrawCmd::StrokeRect { rect, width, .. } if owner == Some(btn) => {
                assert_eq!(*width, FOCUS_STROKE_WIDTH, "按钮上只允许焦点环这一条描边");
                assert_eq!(*rect, band, "前置：焦点环的矩形就是按钮内缩 FOCUS_RING_INSET 的那一圈");
                assert!(
                    ring_index.is_none(),
                    "前置：按钮上出现了第二条焦点环描边（判据只喂得了第一条）"
                );
                ring_index = Some(i);
            }
            _ => {}
        }
    }
    let ring_index = ring_index
        .unwrap_or_else(|| panic!("前置：`focused` 那一帧里按钮必须有焦点环描边（否则这条自检测的是空气）"));
    let fill_color = fill.expect("按钮必须有填充命令");
    assert_eq!(
        fill_color, theme.accent,
        "前置：按钮填充就是 accent ⇒「环 = accent」等于「环与填充同色」"
    );
    let ring_color = if let DrawCmd::StrokeRect { color, .. } = &focused_frame.1.list.cmds[ring_index] {
        *color
    } else {
        panic!("前置：第 {ring_index} 条命令必须是焦点环描边");
    };
    assert_ne!(
        ring_color, fill_color,
        "前置：环与填充同色 ⇒ 对比度判据测的是空气（同色时差异只剩「描边压过字形」那几像素）"
    );
    println!(
        "  焦点环：命令 #{ring_index}｜环色 {ring_color:?}｜填充 {fill_color:?}｜环带 {band:?}\
         ｜下限 {FOCUS_MIN_PX} px + 平均通道差 {FOCUS_MIN_CONTRAST:.1}"
    );

    // 焦点环的两条下限（函数化 ⇒ 同一次运行里被「实测 + 两个坏环」喂三次）。
    let focus_verdict = |n: usize, c: f64| -> Result<(), String> {
        if n < FOCUS_MIN_PX {
            return Err(format!("按钮内差异 {n} px < 下限 {FOCUS_MIN_PX}"));
        }
        if c < FOCUS_MIN_CONTRAST {
            return Err(format!(
                "平均通道差 {c:.1} < 下限 {FOCUS_MIN_CONTRAST:.1} —— 环画上去了，但看不出"
            ));
        }
        Ok(())
    };
    // 只换**环色**的两个语料（其余命令逐字节相同 ⇒ 差异只能来自环色）。
    let with_ring_color = |c: Color| {
        let mut l = focused_frame.1.list.clone();
        match &mut l.cmds[ring_index] {
            DrawCmd::StrokeRect { color, .. } => *color = c,
            _ => unreachable!("ring_index 指向的就是焦点环描边"),
        }
        l
    };
    let render_list = |l: &DrawList| {
        CpuRenderer::new()
            .render(ext, l, CLEAR)
            .expect("CPU 渲染失败")
            .pixels
    };
    let idle_px: &Vec<u8> = &px.iter().find(|(n, _)| *n == "idle").expect("idle 已渲染").1;
    let focused_px: &Vec<u8> = &px
        .iter()
        .find(|(n, _)| *n == "focused")
        .expect("focused 已渲染")
        .1;

    // ---- ② 实测：把两个数打出来，再判 ----
    let (n_real, c_real) = mean_channel_diff(idle_px, focused_px, ext, btn);
    println!(
        "  focused 实测：按钮内差异 {n_real} px（下限 {FOCUS_MIN_PX}）｜平均通道差 {c_real:.1}\
         （下限 {FOCUS_MIN_CONTRAST:.1}）｜判据 {:?}",
        focus_verdict(n_real, c_real).err()
    );
    focus_verdict(n_real, c_real).unwrap_or_else(|e| panic!("focused 焦点环判据失败：{e}"));

    // ---- ③ 坏环 a：环 = 填充色（改前那一版）⇒ 差异像素数塌掉 ⇒ 撞第①条 ----
    let same_px = render_list(&with_ring_color(fill_color));
    let d_same = diff_pixels(idle_px, &same_px, ext, btn);
    let (n_same, c_same) = mean_channel_diff(idle_px, &same_px, ext, btn);
    println!(
        "  反向自检 a（环 = 填充色）：按钮内差异 {n_same} px（可见时 {n_real} px）｜平均通道差 {c_same:.1}\
         ｜判据 {:?}",
        focus_verdict(n_same, c_same).err()
    );
    assert_eq!(
        n_same, d_same,
        "前置：两条数法（`diff_pixels` / `mean_channel_diff`）必须数**同一个**像素集合"
    );
    assert!(
        d_same < FOCUS_MIN_PX,
        "下限 {FOCUS_MIN_PX} 抓不住「环与填充同色」（同色时仍有 {d_same} px 差异）⇒ 这条下限在空转"
    );
    assert!(
        focus_verdict(n_same, c_same).is_err(),
        "判据必须拒绝「环与填充同色」，否则它抓不住本次要修的缺陷"
    );

    // ---- ③ 坏环 b：环 = 填充色**提亮 5%**（技术上不同色、肉眼看不出）⇒ 撞第②条 ----
    //
    // 这是 parity 侧原先的覆盖缺口：`n_faint` **过得了**像素数下限（近似色照样改掉整条环带），
    // 只有对比度下限咬得住它。所以这里两条都要断言：**前置**（像素数确实过关）与**判据必须拒绝**。
    let faint_px = render_list(&with_ring_color(fill_color.lighten(0.05)));
    let (n_faint, c_faint) = mean_channel_diff(idle_px, &faint_px, ext, btn);
    println!(
        "  反向自检 b（环 = 填充提亮 5%）：按钮内差异 {n_faint} px｜平均通道差 {c_faint:.1}\
         ｜判据 {:?}",
        focus_verdict(n_faint, c_faint).err()
    );
    assert!(
        n_faint >= FOCUS_MIN_PX,
        "反向自检 b 的前置：近似色**照样**改掉整条环带（{n_faint} px ≥ 下限 {FOCUS_MIN_PX}）\
         ⇒ 光数像素数抓不住它，必须有第②条"
    );
    assert!(
        c_faint < FOCUS_MIN_CONTRAST,
        "反向自检 b：近似色的平均通道差 {c_faint:.1} 必须低于下限 {FOCUS_MIN_CONTRAST:.1}（否则对比度下限在空转）"
    );
    assert!(
        focus_verdict(n_faint, c_faint).is_err(),
        "判据必须拒绝「技术上有差异、肉眼看不出」的环，否则对比度下限是摆设"
    );

    // ---- 焦点环判据（`typed` 档的**输入框**）+ 反向自检 ----
    //
    // `typed` 档的差异有**两个来源**：输入框文字（占位标签 → `hi`）与输入框**焦点环**。
    // 要单独咬住环，就把文字这个变量固定住：拿一个「同样显示 `hi`、但**没有焦点**」的帧，
    // 与 `typed` 相减 ⇒ 两帧的填充/边框/文字逐字节相同，差异**只剩环带**。
    //
    // 为什么输入框也要这一条：它的环改前**贴在框边画**（直角描边压在圆角填充上，补出 32 px
    // 方角补块，与按钮当初同一根因）⇒ 而旧的 `typed` 判据只看「框内差异 ≥ 500」，
    // 684 px 的坏几何照样过关。现在判据是「**框内差异一个都不许落在环带之外**」+「环像素
    // 一个都不许落在圆角剪影之外」+ 像素数下限 + 对比度下限，并当场把坏几何喂回去。
    let typed_state = UiState {
        hover: Some("button_1".into()),
        focus: Some("field_1".into()),
        pressed: None,
        texts: BTreeMap::from([("field_1".to_string(), "hi".to_string())]),
        carets: Default::default(),
        // 合并提示：这条线给 `UiState` 新增了 `scroll`（合并另一条线时漏它 ⇒ E0063 全 test build 崩）。
        scroll: Default::default(),
        preedit: None,
    };
    let no_focus_state = UiState {
        focus: None,
        ..typed_state.clone()
    };
    let typed_frame = cases.iter().find(|(n, _, _)| *n == "typed").expect("typed 在列表里");
    let no_focus_frame = frame_with(&tree, &theme, &geo, &no_focus_state);
    println!(
        "  输入框焦点环：#1 前置｜两帧状态除 `focus` 外必须相同（typed={}）",
        fmt_state(&typed_state)
    );
    assert_eq!(
        UiState {
            focus: Some("field_1".into()),
            ..no_focus_state.clone()
        },
        typed_state,
        "前置：对照帧只差一个 `focus` 字段"
    );

    let field = rect_i(&geo, "field_1");
    let fband = RectI::new(
        field.x + FOCUS_RING_INSET,
        field.y + FOCUS_RING_INSET,
        field.w - 2 * FOCUS_RING_INSET,
        field.h - 2 * FOCUS_RING_INSET,
    );
    // 前置：输入框上恰好一条 1px 边框 + 一条环，且环的矩形就是内缩那一圈。
    let (mut f_owner, mut f_border, mut f_ring, mut f_ring_index) = (None, None, None, None);
    for (i, cmd) in typed_frame.1.list.cmds.iter().enumerate() {
        match cmd {
            DrawCmd::NodeHint { rect, .. } => f_owner = Some(*rect),
            DrawCmd::StrokeRect { rect, color, width } if f_owner == Some(field) => {
                if *width == FOCUS_STROKE_WIDTH {
                    assert!(
                        f_ring_index.is_none(),
                        "前置：输入框上出现了第二条焦点环描边（判据只喂得了第一条）"
                    );
                    f_ring_index = Some(i);
                    f_ring = Some((*rect, *color));
                } else {
                    f_border = Some((*rect, *width, *color));
                }
            }
            _ => {}
        }
    }
    let f_ring_index = f_ring_index.expect("前置：`typed` 那一帧的输入框必须有焦点环描边");
    let (f_ring_rect, f_ring_color) = f_ring.expect("刚设过");
    let (f_border_rect, f_border_w, f_border_color) = f_border.expect("前置：输入框必须保留 1px 边框");
    assert_eq!(
        (f_border_rect, f_border_w),
        (field, 1),
        "前置：边框仍是 1px、且铺满框（差异才恰好等于环带）"
    );
    assert_eq!(f_border_color, theme.text_dim, "前置：边框色是 text_dim");
    assert_eq!(f_ring_rect, fband, "前置：环矩形 = 框内缩 FOCUS_RING_INSET 的那一圈");
    let f_fill = typed_frame
        .1
        .list
        .cmds
        .iter()
        .find_map(|c| match c {
            DrawCmd::FillRoundRect { rect, color, .. } if *rect == field => Some(*color),
            _ => None,
        })
        .expect("前置：输入框必须有填充命令");
    assert_eq!(f_fill, theme.border, "前置：输入框填充是 border（环色必须与它分得开）");
    assert_ne!(f_ring_color, f_fill, "前置：环与填充同色 ⇒ 对比度判据测的是空气");
    let f_band_px = (fband.w * fband.h
        - inset_rect(fband, FOCUS_STROKE_WIDTH).w * inset_rect(fband, FOCUS_STROKE_WIDTH).h)
        as usize;
    println!(
        "  输入框 {field:?}｜环 #{f_ring_index} {f_ring_rect:?} 色 {f_ring_color:?}｜填充 {f_fill:?}\
         ｜边框 1px {f_border_color:?}｜环带 {f_band_px} px｜下限 {FIELD_RING_MIN_PX} px + 平均通道差 {FOCUS_MIN_CONTRAST:.1}"
    );

    // 只换**输入框描边命令**的三个语料（其余命令逐字节相同 ⇒ 差异只能来自环）。
    let with_field_strokes = |ring: RectI, color: Color, keep_border: bool| {
        let mut l = typed_frame.1.list.clone();
        let mut out: Vec<DrawCmd> = Vec::with_capacity(l.cmds.len());
        let (mut owner, mut done) = (None, false);
        for cmd in l.cmds.drain(..) {
            match &cmd {
                DrawCmd::NodeHint { rect, .. } => {
                    owner = Some(*rect);
                    out.push(cmd);
                }
                DrawCmd::StrokeRect { .. } if owner == Some(field) => {
                    if !done {
                        done = true;
                        if keep_border {
                            out.push(DrawCmd::StrokeRect {
                                rect: field,
                                color: theme.text_dim,
                                width: 1,
                            });
                        }
                        out.push(DrawCmd::StrokeRect {
                            rect: ring,
                            color,
                            width: FOCUS_STROKE_WIDTH,
                        });
                    }
                }
                _ => out.push(cmd),
            }
        }
        assert!(done, "前置：输入框上必须有描边命令可替换");
        DrawList::from_cmds(out)
    };
    let f_idle_px = render(&no_focus_frame);
    let f_real_px = render(&typed_frame.1);
    // 前置：这一对帧只在输入框的描边上不同（把环命令之外的差异排除掉）。
    assert_eq!(
        no_focus_frame.list.counts().node_hint,
        typed_frame.1.list.counts().node_hint,
        "前置：两帧的提示条数必须相同"
    );
    let s_field = ring_band(
        &f_idle_px,
        &f_real_px,
        ext,
        field,
        fband,
        f_ring_rect,
        FILL_ROUND_RADIUS,
    );
    println!(
        "  输入框环实测：环带内 {}（下限 {FIELD_RING_MIN_PX}）｜环带外 {}（要求 0）｜框外 {}（要求 0）\
         ｜方角补块 {}（要求 0）｜平均通道差 {:.1}（下限 {FOCUS_MIN_CONTRAST:.1}）",
        s_field.in_band,
        s_field.out_band_in_rect,
        s_field.out_rect,
        s_field.patch,
        s_field.mean
    );
    field_ring_verdict("实测", &s_field).unwrap_or_else(|e| panic!("{e}"));

    // 反向自检 a：环色 = 填充色（`border`）⇒ 环带内差异塌成 0。
    let s_same = ring_band(
        &f_idle_px,
        &render_list(&with_field_strokes(fband, f_fill, true)),
        ext,
        field,
        fband,
        fband,
        FILL_ROUND_RADIUS,
    );
    println!(
        "  反向自检 a（环 = 填充色）：环带内 {}｜平均通道差 {:.1} ⇒ {:?}",
        s_same.in_band,
        s_same.mean,
        field_ring_verdict("a", &s_same).err()
    );
    assert!(
        field_ring_verdict("反自检 a：环与填充同色", &s_same).is_err(),
        "判据必须拒绝「环与填充同色」"
    );

    // 反向自检 b：环色 = 填充色提亮 5%（技术上不同色、肉眼看不出）⇒ 靠**对比度**下限咬住。
    let s_faint = ring_band(
        &f_idle_px,
        &render_list(&with_field_strokes(fband, f_fill.lighten(0.05), true)),
        ext,
        field,
        fband,
        fband,
        FILL_ROUND_RADIUS,
    );
    println!(
        "  反向自检 b（环 = 填充提亮 5%）：环带内 {}｜平均通道差 {:.1} ⇒ {:?}",
        s_faint.in_band,
        s_faint.mean,
        field_ring_verdict("b", &s_faint).err()
    );
    assert!(
        s_faint.in_band >= FIELD_RING_MIN_PX,
        "反向自检 b 的前置：近似色**照样**改掉整条环带（{} px ≥ 下限 {FIELD_RING_MIN_PX}）",
        s_faint.in_band
    );
    assert!(
        field_ring_verdict("反自检 b：环色几乎与填充相同", &s_faint).is_err(),
        "判据必须拒绝「技术上有差异、肉眼看不出」的环"
    );

    // 反向自检 c：**几何**退回贴边画（= 改前的代码路径）⇒ 方角补块与「环带外差异」都必须被抓出来。
    let s_flush = ring_band(
        &f_idle_px,
        &render_list(&with_field_strokes(field, f_ring_color, false)),
        ext,
        field,
        fband,
        field,
        FILL_ROUND_RADIUS,
    );
    println!(
        "  反向自检 c（环贴边画 = 改前几何）：环带内 {}｜环带外 {}｜方角补块 {}｜平均通道差 {:.1} ⇒ {:?}",
        s_flush.in_band,
        s_flush.out_band_in_rect,
        s_flush.patch,
        s_flush.mean,
        field_ring_verdict("c", &s_flush).err()
    );
    assert_eq!(
        s_flush.patch, 32,
        "前置：贴边画的 3px 直角环在半径 {FILL_ROUND_RADIUS} 的圆角上必留 32 px 方角补块（与按钮同源）"
    );
    assert!(
        field_ring_verdict("反自检 c：环贴边画", &s_flush).is_err(),
        "判据必须拒绝「环贴边画」—— 这正是本轮修的形状缺陷"
    );
    // 而且**旧的 `typed` 判据会原样放行它** —— 这是「旧判据强度不够」的现场证据：
    // 同一份坏几何（贴边、不留 1px 边框）与 idle 相比，框内差异仍然远超旧下限 500。
    let flush_vs_idle = diff_pixels(
        &px.iter().find(|(n, _)| *n == "idle").expect("idle 已渲染").1,
        &render_list(&with_field_strokes(field, f_ring_color, false)),
        ext,
        field,
    );
    println!(
        "  对照：改前几何下 `typed` 档**框内**差异 = {flush_vs_idle} px（旧下限 {} ⇒ 旧判据原样放行）",
        STATE_MIN_PX
            .iter()
            .find(|(n, _)| *n == "typed")
            .expect("typed 有下限")
            .1
    );
    assert!(
        flush_vs_idle >= FOCUS_MIN_PX,
        "前置：这份坏几何在框内留下了 {flush_vs_idle} px 差异（远超旧下限）⇒ 「只数框内像素」抓不住它"
    );

    // 反向自检：五档状态**两两不同**（否则「状态之间可区分」是空话）。
    for (i, (n1, p1)) in px.iter().enumerate() {
        for (n2, p2) in px.iter().skip(i + 1) {
            let d = p1.iter().zip(p2.iter()).filter(|(a, b)| a != b).count();
            assert!(d > 0, "{n1} 与 {n2} 的像素完全相同 ⇒ 两档状态不可区分");
            println!("  {n1} vs {n2}：差异 {d} 字节");
        }
    }
    println!("四档状态：越界字节 = 0，且五档两两不同 ✅");
}

/// 每档状态**差异像素数**的下限（判据是「差异 ≥ 明确下限」，**不是**「差异 > 0」）。
///
/// 出处：这三档像素判据跑的是 `ApproxMeasure`（占位度量）+ 固定窗口 `420×220` + 固定树
/// ⇒ 完全确定，所以每个数字都是**实测值向下取整**并留出余量；「上限」是几何上界（被字形盖住的
/// 部分不产生差异）：
///
/// | 状态 | 期望矩形 | 上限（几何） | 实测 | 下限 |
/// |---|---|---|---|---|
/// | `hover` | 按钮 | 792 px（按钮 36×22） | 628 | 600 |
/// | `pressed` | 按钮 | 792 | 628 | 600 |
/// | `focused` | 按钮 | 焦点环环带 264 px | 228 | [`FOCUS_MIN_PX`] = 180 |
/// | `typed` | 输入框 | 2156 px（输入框 98×22） | 702 | 500 |
///
/// `typed` 的下限留得多（702 → 500）是有意的：它数的是「占位标签换成缓冲内容」的占位格差异，
/// 随度量模型而变；而「什么都没画出来」是 **0 px**，所以 500 仍然有判别力。
/// `hover`/`pressed` 的下限（628 → 600）离实测更近，因为它们的上界是**几何**给的
/// （整块填充 = 按钮面积减去被字形盖住的部分），不是模型给的。
///
/// ⚠️ `typed` 这一档在 **2b 修输入框焦点环**前后都被实测过，两次数字都在本测试里当场打印：
/// 改前几何（环贴边、focus 不留 1px 边框）**816 px**（正是旧表格里记的那个数），交付几何 **702 px**。
/// 下限**保持 500 不动**：绝对值没降，而「下限 / 实测」从 0.61 升到 **0.71** ⇒ 强度不降反升；
/// 同时输入框的环另有**更强**的一条判据（`FIELD_RING_MIN_PX` + 环带外差异 = 0 + 方角补块 = 0
/// + 反向自检 a/b/c）—— 那才是对本次缺陷真正有判别力的那一层。
const STATE_MIN_PX: [(&str, usize); 4] = [
    ("hover", 600),
    ("pressed", 600),
    ("focused", FOCUS_MIN_PX),
    ("typed", 500),
];

/// 按钮**焦点环**的差异像素数下限（本次从「> 0」加严出来的那一条）。
///
/// 出处（不是随手填的数字；下面三个数都是**本语料**的实测值，由本测试的打印逐行给出）：
/// - 环带 = 按钮 36×22（本语料的 `button_1`）内缩 `FOCUS_RING_INSET`=2、宽 `FOCUS_STROKE_WIDTH`=3
///   ⇒ `32×18 − 26×12 = 576 − 312 = `**264** px（环带矩形实测 `RectI { x: 14, y: 42, w: 32, h: 18 }`）；
/// - 环画在标签**之上**，标签与环同色（都是 `theme.on_accent`）的地方**不产生差异** ⇒ 实测 **228** px；
/// - 下限取 **180**：落在两个锚点之间 ——「可见环 228 px」与「环与填充同色 36 px」
///   （后者由本测试的**反向自检 a** 当场打印，可复核）。两侧余量 48 / 144，
///   而改前那条判据（`> 0`）在同色环的 36 px 面前**永远是绿的**。
///
/// ⚠️ 这一条只保证「环**被画出来**了」；「画出来了**看不出**」（近似色）由 [`FOCUS_MIN_CONTRAST`] 负责 ——
/// 两条是**互补**的，缺一条就有一类坏环能过去（见 M5b 终审 Minor 1）。
const FOCUS_MIN_PX: usize = 180;

/// 按钮焦点环的**对比度下限**：按钮内差异像素的「三通道平均差」（量纲 0..255）。
///
/// 出处（三个数字都是**同一份语料里当场实测**出来的，`mean_channel_diff` 打印的那三行）：
///
/// | 语料 | 差异像素数 | 平均通道差 | 该被哪条下限拒绝 |
/// |---|---|---|---|
/// | 环 = `theme.on_accent`（**交付的环**） | 228 | **97.7** | 两条都过（这正是「看得见」） |
/// | 环 = 填充色 `accent`（改前那一版） | 36 | 97.7 | 第①条（像素数）—— 只画在字上，环**没画出来** |
/// | 环 = `accent.lighten(0.05)`（近似色） | 264 | **17.0** | 第②条（对比度）—— 环**画出来了**，但看不出 |
///
/// 下限取 **64**（= 256 的四分之一）：落在两个锚点 97.7 / 17.0 之间，两侧余量 33.7 / 47.0。
/// 「近似色」那一行是本次补的漏洞现场：它 264 px **一个不少**，只数像素数的判据（264 ≥ 180）
/// 会放行（M5b 终审 Minor 1 实测：`deer-gpu` 单测红、parity 仍绿）。
/// **这个数值必须与 `crates/deer-gpu/src/interact.rs` 的 `FOCUS_RING_MIN_CONTRAST` 相同** ——
/// 两边口径一致（同为「差异像素的三通道平均差」），只是分类范围不同（那边按环带内外，
/// 这边按按钮矩形）；两个数的依据也一致：`on_accent` 压 `accent` = 97.7、提亮 5% = 17.0。
///
/// **输入框的环用同一个下限**（`field_ring_verdict`）：环色 `accent` 压在填充 `border` 上 =
/// **106.7**（放行），同色 = **0.0**、提亮 5% = **10.3**（都拒绝）—— 两个锚点与按钮那条同量级。
const FOCUS_MIN_CONTRAST: f64 = 64.0;

/// 在 `rect` 内逐**像素**比较（4 字节一组），返回差异像素数。
fn diff_pixels(a: &[u8], b: &[u8], ext: Extent, rect: RectI) -> usize {
    let w = ext.width.max(1) as usize;
    let mut n = 0usize;
    for i in (0..a.len().min(b.len())).step_by(4) {
        if a[i..i + 4] == b[i..i + 4] {
            continue;
        }
        let x = ((i / 4) % w) as i32;
        let y = ((i / 4) / w) as i32;
        if rect.contains(x, y) {
            n += 1;
        }
    }
    n
}

/// 在 `rect` 内数差异像素数**并**算它们的「三通道平均差」（返回 `(像素数, 平均通道差)`）。
///
/// **口径照抄** `crates/deer-gpu/src/interact.rs` 的 `ring_stats.mean_contrast`：对每个差异像素取
/// `|ΔR| + |ΔG| + |ΔB|`，再除以 `3 × 差异像素数`（量纲 0..255）。两边同口径 ⇒ 下限可以互相印证。
///
/// 为什么焦点环需要这个数：环是一条**细带**，面积本身就小，「看得见」靠的是**对比度**。
/// 只数像素数会把「填充色提亮 5%」这种技术上不同色、肉眼看不出 的环放过去 —— 它的差异像素数
/// 一个不少，只是每个像素都几乎没变（详见 [`FOCUS_MIN_CONTRAST`]）。
fn mean_channel_diff(a: &[u8], b: &[u8], ext: Extent, rect: RectI) -> (usize, f64) {
    let w = ext.width.max(1) as usize;
    let (mut n, mut sum) = (0usize, 0u64);
    for i in (0..a.len().min(b.len())).step_by(4) {
        if a[i..i + 4] == b[i..i + 4] {
            continue;
        }
        let x = ((i / 4) % w) as i32;
        let y = ((i / 4) / w) as i32;
        if !rect.contains(x, y) {
            continue;
        }
        n += 1;
        for c in 0..3 {
            sum += a[i + c].abs_diff(b[i + c]) as u64;
        }
    }
    let mean = if n == 0 {
        0.0
    } else {
        sum as f64 / (3.0 * n as f64)
    };
    (n, mean)
}

/// 把差异按「按钮矩形内 / 输入框矩形内 / 其它」分开计数。
fn diff_split(a: &[u8], b: &[u8], ext: Extent, rect: RectI) -> (usize, usize) {
    let w = ext.width.max(1) as usize;
    let mut inside = 0usize;
    let mut outside = 0usize;
    for i in 0..a.len().min(b.len()) {
        if a[i] == b[i] {
            continue;
        }
        let x = ((i / 4) % w) as i32;
        let y = ((i / 4) / w) as i32;
        if rect.contains(x, y) {
            inside += 1;
        } else {
            outside += 1;
        }
    }
    (inside, outside)
}

// ---------------------------------------------------------------------------
// 三、离屏 GPU vs CPU（四档状态逐像素）
// ---------------------------------------------------------------------------

/// 系统字体引擎（拿不到就**明确跳过**并打印原因，不伪装成通过）。
fn text_engine(size: f32) -> Option<TextEngine> {
    match TextEngine::from_system_font(size) {
        Ok(e) => Some(e),
        Err(e) => {
            eprintln!("跳过：这台机器上拿不到系统字体（{e}）");
            None
        }
    }
}

/// 逐像素对照的**打印 + 断言**：`max_allowed = 0` 时额外断言逐字节相同。
///
/// 打印**两组数字**：① 最大通道差（GPU vs CPU 的对齐判据）；
/// ② 与 idle 的差异（状态判据）。两组都是「可断言的数字」，不是一句「看起来对」。
fn compare(
    name: &str,
    gpu: &[u8],
    cpu: &[u8],
    idle_cpu: &[u8],
    ext: Extent,
    allowed: &[RectI],
    max_allowed: u8,
) {
    assert_eq!(
        gpu.len(),
        cpu.len(),
        "{name}: GPU 回读 {} 字节与 CPU {} 字节不一致",
        gpu.len(),
        cpu.len()
    );
    let mut worst = 0u8;
    let mut at = 0usize;
    for (i, (g, c)) in gpu.iter().zip(cpu.iter()).enumerate() {
        let d = g.abs_diff(*c);
        if d > worst {
            worst = d;
            at = i;
        }
    }
    let w = ext.width.max(1) as usize;
    // 与 idle 的差异（**只统计 CPU 侧**：GPU 侧的差异位置与 CPU 一致是上面那条判据的推论）。
    let mut diff_total = 0usize;
    let mut diff_outside = 0usize;
    for i in 0..idle_cpu.len() {
        if idle_cpu[i] == cpu[i] {
            continue;
        }
        diff_total += 1;
        let x = ((i / 4) % w) as i32;
        let y = ((i / 4) / w) as i32;
        if !allowed.iter().any(|r| r.contains(x, y)) {
            diff_outside += 1;
        }
    }
    println!(
        "  {name:<8} GPU vs CPU：最大通道差 {worst}（允许 {max_allowed}）最差在 ({}, {})｜\
         与 idle 差异 {diff_total} 字节（越界 {diff_outside}）",
        (at / 4) % w,
        (at / 4) / w
    );
    assert!(
        worst <= max_allowed,
        "{name}: 最大通道差 {worst} > 允许的 {max_allowed}；最差处像素 ({}, {})：GPU={:?} CPU={:?}",
        (at / 4) % w,
        (at / 4) / w,
        &gpu[at - at % 4..at - at % 4 + 4],
        &cpu[at - at % 4..at - at % 4 + 4]
    );
    if max_allowed == 0 {
        assert_eq!(gpu, cpu, "{name}: 不透明绘制必须逐字节相同");
    }
    assert_eq!(diff_outside, 0, "{name}: 与 idle 的差异必须全在期望矩形内");
}

/// **四档状态的离屏 GPU vs CPU 对照**（真实字形，不透明语料 ⇒ 逐字节相同）。
///
/// 用真实字形而不是占位格：`CpuRenderer::new()` 的占位格模型与 GPU 的图集采样**模型不同**
/// （见 `deer-vk/tests/gpu_vs_cpu.rs` 的三条硬前提）⇒ 必须两边都挂 `TextEngine`。
#[test]
fn four_states_match_the_cpu_backend_offline() {
    let size = 20.0f32;
    let theme = Theme {
        font_size: size,
        line_height: 26.0,
        ..Theme::default()
    };
    let Some(e_gpu) = text_engine(size) else { return };
    let Some(e_cpu) = text_engine(size) else { return };
    let ext = Extent {
        width: 320,
        height: 160,
    };

    // 语料：与真实界面同构的树，但用真实字体度量布局（GPU 与 CPU 用同一套命令）。
    let tree = app_tree();
    let style = TextStyle {
        font_size: size,
        line_height: theme.line_height,
    };
    let geo = layout::layout(
        &tree,
        Rect::new(0.0, 0.0, ext.width as f32, ext.height as f32),
        style,
        &e_cpu.measure(),
    );
    let gpu = match GpuGeometryRenderer::new(0, ext, CLEAR) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("跳过：本机没有可用的 Vulkan GPU（{e}）");
            return;
        }
    };
    // 两个引擎由**同一个系统字体**、按**完全相同的命令顺序**填充图集 ⇒ 槽位布局逐字节相同。
    let mut gpu = gpu
        .with_text(e_gpu)
        .unwrap_or_else(|e| panic!("with_text 失败（文本管线建不起来）：{e}"));
    let mut cpu = CpuRenderer::with_text(e_cpu);

    let cases = states(&theme, &tree, &geo);
    // 前置：语料必须**同时**含形状与文本（否则「逐字节相同」可能平凡成立）。
    let idle = cases
        .iter()
        .find(|(n, _, _)| *n == "idle")
        .expect("idle 在列表里");
    let c = idle.1.list.counts();
    println!(
        "语料：形状 {}（fill {} / round {} / stroke {}）+ 文本 {}｜NodeHint {}｜{ext:?}",
        c.fill_rect + c.fill_round_rect + c.stroke_rect,
        c.fill_rect,
        c.fill_round_rect,
        c.stroke_rect,
        c.text,
        c.node_hint
    );
    assert!(
        c.fill_rect + c.fill_round_rect + c.stroke_rect > 0 && c.text > 0,
        "测试前置：语料必须同时含形状与文本"
    );
    assert!(c.node_hint > 0, "测试前置：语料必须发 NodeHint");

    let cpu_px: Vec<(&str, Vec<u8>)> = cases
        .iter()
        .map(|(n, f, _)| {
            (
                *n,
                cpu.render(ext, &f.list, CLEAR)
                    .expect("CPU 渲染失败")
                    .pixels,
            )
        })
        .collect();
    let idle_cpu = cpu_px
        .iter()
        .find(|(n, _)| *n == "idle")
        .expect("idle 已渲染")
        .1
        .clone();

    println!();
    println!("—— 四档状态：离屏 GPU vs CPU（真实字形，不透明 ⇒ 要求逐字节相同）——");
    let mut worst_overall = 0u8;
    for (name, f, allowed) in &cases {
        let gpu_px = gpu
            .render(&f.list)
            .unwrap_or_else(|e| panic!("{name}: GPU 渲染失败：{e}"));
        assert!(
            gpu.unsupported().is_empty(),
            "{name}: 文本已被接管 ⇒ 不该有 unsupported（{:?}）",
            gpu.unsupported()
        );
        let cpu_px = &cpu_px.iter().find(|(n, _)| n == name).expect("刚渲染过").1;
        compare(
            name,
            &gpu_px,
            cpu_px,
            &idle_cpu,
            ext,
            allowed,
            0,
        );
        worst_overall = worst_overall.max(
            gpu_px
                .iter()
                .zip(cpu_px.iter())
                .map(|(a, b)| a.abs_diff(*b))
                .max()
                .unwrap_or(0),
        );
    }
    // 语料里**同时**有不透明与半透明命令（容器底色 `Theme::surface` 是 α=0.55）：
    // 于是下面「最大通道差 0」既覆盖不透明（要求 0），也覆盖半透明（要求 ≤1 LSB ——
    // 这里实测更强，是 0）。先钉住这个前提，否则「≤1 LSB」那条判据可能是空转的。
    let alpha_cmds = idle.1.list.cmds.iter().filter(|c| {
        let a = match c {
            DrawCmd::FillRect { color, .. }
            | DrawCmd::StrokeRect { color, .. }
            | DrawCmd::FillRoundRect { color, .. }
            | DrawCmd::Text { color, .. } => color.a,
            _ => 1.0,
        };
        a > 0.0 && a < 1.0
    });
    let alpha_count = alpha_cmds.count();
    println!("语料里的半透明命令：{alpha_count} 条（α ∈ (0,1)）");
    assert!(
        alpha_count > 0,
        "测试前置：语料必须含半透明命令，否则「半透明 ≤1 LSB」这条判据是空转的"
    );
    println!(
        "四档状态：GPU vs CPU 最大通道差 {worst_overall}\
         （不透明要求 0、半透明要求 ≤1 LSB；两项都满足且这里是 0）✅"
    );
    assert!(worst_overall <= 1, "半透明也不能超过 1 LSB");
    assert_eq!(worst_overall, 0, "本语料实测逐字节相同");
}

// ---------------------------------------------------------------------------
// 四、输入框焦点环的诊断与判据（口径与 `crates/deer-gpu/src/interact.rs` 的 `ring_stats` 一致）
// ---------------------------------------------------------------------------

/// 输入框**焦点环**的差异像素数下限（本语料 `field_1` = 98×22）。
///
/// 出处：
/// - 期望环带 = 框内缩 `FOCUS_RING_INSET`=2、宽 `FOCUS_STROKE_WIDTH`=3 ⇒ `94×18 − 88×12 = 636` px；
/// - 环画在文字之上，而文字在两帧里**相同** ⇒ 被字形盖住的环像素不产生差异，实测 **594** px；
/// - 下限取环带的 **3/4 = 636×3/4 = 477**（与按钮那条 `264×3/4 = 198` 同一条推导），
///   实测 594 留余量 117；而「环与填充同色」实测 **0 px**、坏几何的方角补块是 **32 px** ⇒
///   两个锚点都远在下限之下（这条下限只负责「环有没有被画出来」，形状由 `patch == 0` 与
///   「环带外差异 == 0」负责）。
const FIELD_RING_MIN_PX: usize = 477;

/// 向内缩 `n` 像素（与 `deer-gpu` 的 `inset` 同口径）。
fn inset_rect(r: RectI, n: i32) -> RectI {
    RectI::new(r.x + n, r.y + n, (r.w - 2 * n).max(1), (r.h - 2 * n).max(1))
}

/// 圆角剪影判定（**口径照抄** `deer-gpu/src/null.rs` 的 `inside_rounded`）。
fn inside_rounded(rect: RectI, x: i32, y: i32, r: i32) -> bool {
    let corners = [
        (rect.x + r, rect.y + r, -1, -1),
        (rect.right() - 1 - r, rect.y + r, 1, -1),
        (rect.x + r, rect.bottom() - 1 - r, -1, 1),
        (rect.right() - 1 - r, rect.bottom() - 1 - r, 1, 1),
    ];
    for (ccx, ccy, sx, sy) in corners {
        let in_corner_x = if sx < 0 { x < ccx } else { x > ccx };
        let in_corner_y = if sy < 0 { y < ccy } else { y > ccy };
        if in_corner_x && in_corner_y {
            let dx = (x - ccx) as f32;
            let dy = (y - ccy) as f32;
            if dx * dx + dy * dy > (r * r) as f32 {
                return false;
            }
        }
    }
    true
}

/// 焦点环的像素诊断。
#[derive(Debug)]
struct RingBand {
    /// 期望环带内的差异像素数。
    in_band: usize,
    /// 差异像素里落在期望环带**之外、但仍在控件矩形内**的数量（必须为 0）。
    out_band_in_rect: usize,
    /// 差异像素里落在**控件矩形之外**的数量（必须为 0）。
    out_rect: usize,
    /// 差异像素的三通道平均差。
    mean: f64,
    /// **方角补块**：环实际画在圆角剪影之外的像素数（必须为 0）。
    patch: usize,
}

fn ring_band(
    a: &[u8],
    b: &[u8],
    ext: Extent,
    owner: RectI,
    band: RectI,
    ring_rect: RectI,
    radius: i32,
) -> RingBand {
    let hole = inset_rect(band, FOCUS_STROKE_WIDTH);
    let w = ext.width.max(1) as usize;
    let (mut in_band, mut out_band, mut out_rect, mut sum) = (0usize, 0usize, 0usize, 0u64);
    for i in (0..a.len().min(b.len())).step_by(4) {
        if a[i..i + 4] == b[i..i + 4] {
            continue;
        }
        let x = ((i / 4) % w) as i32;
        let y = ((i / 4) / w) as i32;
        if !owner.contains(x, y) {
            out_rect += 1;
        } else if band.contains(x, y) && !hole.contains(x, y) {
            in_band += 1;
        } else {
            out_band += 1;
        }
        for c in 0..3 {
            sum += a[i + c].abs_diff(b[i + c]) as u64;
        }
    }
    let mut patch = 0usize;
    for y in ring_rect.y..ring_rect.bottom() {
        for x in ring_rect.x..ring_rect.right() {
            if ring_rect.contains(x, y) && !inside_rounded(owner, x, y, radius) {
                patch += 1;
            }
        }
    }
    let n = in_band + out_band + out_rect;
    RingBand {
        in_band,
        out_band_in_rect: out_band,
        out_rect,
        mean: if n == 0 { 0.0 } else { sum as f64 / (3.0 * n as f64) },
        patch,
    }
}

/// **输入框焦点环判据本体**（函数化 ⇒ 同一次运行里被「实测 + 三个坏环」喂四次）。
fn field_ring_verdict(name: &str, s: &RingBand) -> Result<(), String> {
    if s.out_rect != 0 {
        return Err(format!("[{name}] 有 {} 个差异像素落在控件矩形之外", s.out_rect));
    }
    if s.out_band_in_rect != 0 {
        return Err(format!(
            "[{name}] 有 {} 个差异像素落在期望环带之外",
            s.out_band_in_rect
        ));
    }
    if s.patch != 0 {
        return Err(format!(
            "[{name}] 环有 {} 个像素画在圆角剪影之外（直角描边把圆角补成了方角）",
            s.patch
        ));
    }
    if s.in_band < FIELD_RING_MIN_PX {
        return Err(format!(
            "[{name}] 环带内只有 {} 个差异像素 < 下限 {FIELD_RING_MIN_PX}",
            s.in_band
        ));
    }
    if s.mean < FOCUS_MIN_CONTRAST {
        return Err(format!(
            "[{name}] 平均通道差 {:.1} < 下限 {FOCUS_MIN_CONTRAST:.1} —— 环画上去了，但看不出",
            s.mean
        ));
    }
    Ok(())
}
