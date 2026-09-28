//! 功能示例：**滚动容器 + 多行文本**（剩余工作第 1 项）。
//!
//! ```sh
//! cargo run -p deer-gui --example scroll
//! ```
//!
//! 演示三件事（每件都有可数断言，不是「看起来对」）：
//!
//! 1. **多行文本**：`layout.wrap = true` + 节点自己的像素宽度 ⇒ 文本按宽度换行，
//!    每行一条 `DrawCmd::Text`（打印行数与每行矩形）；
//! 2. **滚动容器**：`Column` + `layout.scroll = true`，内容高于视口 ⇒ `max_scroll > 0`；
//!    内容高、视口高、`max_scroll` 三个数字都打印出来；
//! 3. **滚轮驱动偏移**：`Wheel` → `UiEvent::Scrolled` → 偏移进几何 → 命中跟着走
//!    （同一个坐标在滚动前后命中**不同**的控件）；到边界**不越界**。

use deer_gpu::interact::{InteractState, InteractiveRenderer};
use deer_gui::interaction::{ClipSnapshot, InputEvent, UiEvent, UiState, handle, hit};
use deer_gui::prelude::*;

const W: u32 = 220;
const H: u32 = 240;

fn main() {
    let theme = Theme::default();
    let style = TextStyle {
        font_size: theme.font_size,
        line_height: theme.line_height,
    };

    // ---- ① 多行文本 ----
    let mut app = Builder::new(Kind::Column, "app");
    let long = app.text_opts(
        "alpha beta gamma delta epsilon zeta eta theta iota kappa",
        |n| {
            n.layout.width = Some(Size::Px(120.0));
            n.layout.wrap = true;
        },
    );
    // 同一个标签、**不开** wrap：用来对照（它只有一行）。
    let single = app.text_opts(
        "alpha beta gamma delta epsilon zeta eta theta iota kappa",
        |n| n.layout.width = Some(Size::Px(120.0)),
    );
    let tree = app.build();

    let geo = layout(
        &tree,
        Rect::new(0.0, 0.0, W as f32, H as f32),
        style,
        &ApproxMeasure,
    );
    let list = build_draw_list(&tree, &geo, theme.clone(), &ApproxMeasure);
    let lines_of = |id: &str| -> Vec<RectI> {
        let node = geo[id];
        let (y0, y1) = (node.y as i32, (node.y + node.h) as i32);
        list.cmds
            .iter()
            .filter_map(|c| match c {
                // 归给这个节点的行 = 矩形**落在它的竖直区间内**的命令（两个文本节点同 x 同宽，
                // 靠 x/w 分不干净 —— 这也是本项目「命令矩形必须落在自己节点矩形内」的用处）。
                DrawCmd::Text { rect, .. } if rect.y >= y0 && rect.bottom() <= y1 => Some(*rect),
                _ => None,
            })
            .collect()
    };
    let wrapped = lines_of(&long);
    let unwrapped = lines_of(&single);
    println!("画布 {W}×{H}｜节点 {} 个｜绘制命令 {} 条", geo.len(), list.len());
    println!("\n① 多行文本（w=120）：");
    println!("   wrap 开的节点矩形 = {:?}", geo[&long]);
    println!("   wrap 开的行数 = {}，每行矩形：", wrapped.len());
    for (i, r) in wrapped.iter().enumerate() {
        println!("     第 {i} 行 {r:?}");
    }
    println!("   wrap 关的节点矩形 = {:?}｜行数 = {}", geo[&single], unwrapped.len());
    assert!(wrapped.len() >= 3, "这段文本在 120 px 宽下应当换出 ≥3 行");
    assert_eq!(unwrapped.len(), 1, "不换行必须**恰好一行**（既有语义）");
    let line_h = theme.line_height as i32;
    for (i, r) in wrapped.iter().enumerate() {
        assert_eq!(r.y, i as i32 * line_h, "第 {i} 行必须落在 i × 行高 上");
        assert_eq!(r.h, line_h, "每行高度 = 行高");
    }

    // ---- ② 滚动容器 + ③ 滚轮 ----
    let mut app = Builder::new(Kind::Column, "app").padding(8.0).gap(6.0);
    app.container_opts(
        Kind::Column,
        "list",
        L::new().w(200.0).h(120.0).gap(4.0).scroll(true).to_props(),
        |c| {
            for i in 1..=10 {
                c.button(format!("行 {i}"));
            }
        },
    );
    let tree = app.build();

    let mut state = UiState::default();
    let (geo, metrics) = deer_layout::layout::layout_with_scroll(
        &tree,
        Rect::new(0.0, 0.0, W as f32, H as f32),
        style,
        &ApproxMeasure,
        &state.scroll.offsets,
    );
    // 每帧要把布局算出的上限灌回状态（滚轮靠它夹取偏移）。
    state.scroll.set_metrics(&metrics);
    let viewport = geo["list"];
    println!("\n② 滚动容器：");
    println!("   视口 {viewport:?}｜max_scroll = {}", state.scroll.max_of("list"));
    println!("   内容（10 个按钮 + gap）= {}", 10.0 * 22.0 + 9.0 * 4.0);
    assert!(
        state.scroll.max_of("list") > 0,
        "内容必须高于视口，否则这个示例没有可滚动范围"
    );

    let frame = |tree: &Node, state: &UiState| {
        let (geo, metrics) = deer_layout::layout::layout_with_scroll(
            tree,
            Rect::new(0.0, 0.0, W as f32, H as f32),
            style,
            &ApproxMeasure,
            &state.scroll.offsets,
        );
        let list = InteractiveRenderer::new(
            theme.clone(),
            &ApproxMeasure,
            &InteractState::default(),
        )
        .build(tree, &geo);
        let clip = ClipSnapshot::from_draw_list(&list, tree, &geo);
        (geo, metrics, list, clip)
    };

    let (geo0, _, list0, clip0) = frame(&tree, &state);
    let probe = (
        (viewport.x + 5.0),
        (viewport.y + viewport.h / 2.0),
    ); // 视口内一点
    let hit_before = hit(&tree, &geo0, clip0.clone(), probe.0, probe.1).map(|n| n.id.clone());
    println!("   视口内 {probe:?} 命中 = {hit_before:?}");

    println!("\n③ 滚轮：");
    // `Wheel` 没有坐标 ⇒ 滚谁由 hover 决定：先把指针移到视口里的按钮上。
    handle(
        &mut state,
        &tree,
        &geo0,
        clip0.clone(),
        &InputEvent::PointerMoved { x: probe.0, y: probe.1 },
    );
    println!("   hover = {:?}", state.hover);
    for step in 1..=3 {
        let evs = handle(
            &mut state,
            &tree,
            &geo0,
            clip0.clone(),
            &InputEvent::Wheel { dx: 0.0, dy: -1.0 },
        );
        println!(
            "   第 {step} 次滚轮（dy=-1）⇒ {evs:?}｜offset = {}",
            state.scroll.offset_of("list")
        );
        match evs.first() {
            Some(UiEvent::Scrolled { offset, .. }) => assert_eq!(
                *offset,
                state.scroll.offset_of("list"),
                "事件的 offset 必须就是状态里的偏移"
            ),
            other => panic!("滚轮应当产生 Scrolled，实际 {other:?}"),
        }
        assert!(
            state.scroll.offset_of("list") <= state.scroll.max_of("list"),
            "偏移永远不许越界"
        );
    }
    // 一直滚到底：之后**不再有事件**（状态没变）。
    let mut guard = 0;
    while !handle(
        &mut state,
        &tree,
        &geo0,
        clip0.clone(),
        &InputEvent::Wheel { dx: 0.0, dy: -1.0 },
    )
    .is_empty()
    {
        guard += 1;
        assert!(guard < 100, "滚到底之后必须停下来");
    }
    let bottom = state.scroll.offset_of("list");
    println!("   滚到底：offset = {bottom}（= max_scroll {}）", state.scroll.max_of("list"));
    assert_eq!(bottom, state.scroll.max_of("list"), "到底时必须恰好等于 max_scroll");
    assert!(
        handle(
            &mut state,
            &tree,
            &geo0,
            clip0.clone(),
            &InputEvent::Wheel { dx: 0.0, dy: -1.0 }
        )
        .is_empty(),
        "到底之后不该再产生事件"
    );

    // 偏移进几何、进绘制、进命中。
    let (geo1, _, list1, clip1) = frame(&tree, &state);
    let hit_after = hit(&tree, &geo1, clip1, probe.0, probe.1).map(|n| n.id.clone());
    println!(
        "   同一坐标 {probe:?}：滚动前 {hit_before:?}｜滚动后 {hit_after:?}（几何 y 从 {} 变 {}）",
        geo0[hit_before.as_deref().unwrap_or("list")].y,
        geo1[hit_after.as_deref().unwrap_or("list")].y
    );
    assert_ne!(hit_before, hit_after, "滚动之后同一个坐标必须命中**不同**的控件");
    assert_eq!(
        list0.len(),
        list1.len(),
        "命令**条数**不变（滚动只改矩形，不增删命令）—— 这正是「像素判据之外还有几何判据」的意义"
    );
    assert_ne!(list0.cmds, list1.cmds, "绘制命令的矩形必须跟着滚动偏移变");
    assert_eq!(geo0["list"], geo1["list"], "容器自身的矩形不受滚动影响");

    println!("\n滚动 + 多行：全部自检通过 ✅");
}
