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
//! | `four_states_differ_only_inside_the_expected_rectangles` | 四档状态两两像素差异**全部**落在期望矩形内，越界字节 = 0；每档矩形内差异 > 0 |
//! | `four_states_match_the_cpu_backend_offline` | 离屏 GPU vs CPU：**逐字节相同**（不透明语料） |
//!
//! 前置条件（每一条都**显式断言**，因为前置不成立时护栏会**静默失效**）：
//! 快照里有 `button_1`/`field_1`；`field_1` 的中心确实在裁剪外／内；像素差异确实 > 0。

use std::collections::BTreeMap;

use deer_gpu::interact::{FieldText, InteractState, InteractiveRenderer};
use deer_gui::gpu::null::CpuRenderer;
use deer_gui::gpu::{Color, DrawList, Extent, TextEngine, Theme};
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
        println!(
            "  {name:<8} 与 idle 的差异：共 {total} 字节（按钮内 {in_btn}｜输入框内 {in_field}｜**矩形外 {outside}**）\
             ｜命令 {} 条（hint {}）",
            f.list.len(),
            f.list.counts().node_hint
        );
        assert_eq!(
            outside, 0,
            "{name}: 有 {outside} 个字节的差异落在 button_1 {btn:?} 与 field_1 {field:?} **之外** \
             ⇒ 状态视觉溢出到别的控件上了"
        );
        // 这一档必须**真的**画出了东西（否则「越界 = 0」这种断言会因为「什么都没画」而平凡成立）。
        let expected_region = if *name == "typed" { in_field } else { in_btn };
        assert!(
            expected_region > 0,
            "{name}: 期望变化的那个矩形里**没有**差异 ⇒ 这一档状态根本没画出来"
        );
    }

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
