//! **滚动条**：内容超出视口的容器会画一条「轨道 + 滑块」，且滑块位置跟着滚动偏移走。
//!
//! 跑法：
//! ```text
//! cargo run -p deer-gui --example scroll_bar
//! ```
//! 产物：**标准输出**（本示例不写文件）。结尾有自检断言，防止「跑成功但什么都没做」。
//!
//! 指南：[`docs/features/scrollbar.md`](../../../docs/features/scrollbar.md)

use deer_gui::gpu::interact::{InteractiveRenderer, InteractState};
use deer_core::{DrawCmd, RectI};
use deer_gui::gpu::Theme;
use deer_gui::layout::TextStyle;
use deer_gui::layout::layout::{ApproxMeasure, ScrollOffsets, layout_with_scroll};
use deer_gui::layout::node::{Kind, Node, Rect, Size};
use deer_gui::interaction::UiState;

/// 视口高 120；8 个子节点各高 40 ⇒ 内容高 320 ⇒ `max_scroll = 200`。
const VIEWPORT_H: f32 = 120.0;
const ROWS: usize = 8;
const ROW_H: f32 = 40.0;
const W: f32 = 200.0;

fn main() {
    println!("=== 滚动条（T3.2）===\n");

    let mut col = Node::new(Kind::Column, "list");
    col.layout.scroll = true;
    // ⚠️ 视口高度必须**显式给**：滚动容器的语义是「内容超出**自己的**高度」，
    //    而不是超出父给的盒子。只给父盒子的话 `max_scroll` 恒为 0（滚动条根本不出现）。
    col.layout.height = Some(Size::Px(VIEWPORT_H));
    col.layout.width = Some(Size::Px(W));
    for i in 0..ROWS {
        let mut b = Node::new(Kind::Button, format!("row{i}"));
        b.props.label = Some(format!("第 {} 行", i + 1));
        b.layout.height = Some(Size::Px(ROW_H));
        col.children.push(b);
    }

    let style = TextStyle {
        font_size: Theme::default().font_size,
        line_height: Theme::default().line_height,
    };
    let (geo, metrics) = layout_with_scroll(
        &col,
        Rect {
            x: 0.0,
            y: 0.0,
            w: W,
            h: VIEWPORT_H,
        },
        style,
        &ApproxMeasure,
        &ScrollOffsets::new(),
    );
    let max_scroll = metrics.max_of("list");
    println!("① 布局：视口高 {VIEWPORT_H}，{ROWS} 行 × {ROW_H} ⇒ max_scroll = {max_scroll}");
    assert!(
        max_scroll > 0,
        "测试前置：内容必须超出视口，否则滚动条不该出现（本示例也就没东西可讲）"
    );

    // 滚动状态：① 先把布局给出的上限灌进去（这是调用方每帧要做的事）
    let mut state = UiState::default();
    state.scroll.set_metrics(&metrics);

    // 三个偏移各看一次：顶、中、底
    let mut thumb_ys = Vec::new();
    println!("\n② 滑块位置随偏移移动（`scrollbar_geom` 是绘制与命中**共用**的实现）：");
    for (name, off) in [("顶", 0), ("中", max_scroll / 2), ("底", max_scroll)] {
        // 注意：`scroll_to` 在「值没变」时返回 `None`（不是失败）—— 所以断言看的是
        // **结果偏移**，而不是返回值。
        let _ = state.scroll.scroll_to("list", off);
        assert_eq!(
            state.scroll.offset_of("list"),
            off,
            "偏移 {off} 应被接受（上限 {max_scroll}）"
        );

        let bars = bars_in(&col, &geo, &state, "list");
        assert_eq!(bars.len(), 2, "{name}：应当恰好有「轨道 + 滑块」两条命令");
        let (track, thumb) = (bars[0], bars[1]);
        assert!(
            thumb.y >= track.y && thumb.y + thumb.h <= track.y + track.h + 1,
            "{name}：滑块必须在轨道内 {thumb:?} vs {track:?}"
        );
        println!(
            "  偏移 {off:>3} ⇒ 轨道 y={} h={}｜滑块 y={} h={}",
            track.y, track.h, thumb.y, thumb.h
        );
        thumb_ys.push(thumb.y);
    }

    // ─────────────────────────────────────────────────────────────────────
    // 自检：跑成功不等于做对了
    // ─────────────────────────────────────────────────────────────────────
    println!("\n=== 自检 ===");

    // ① 越滚滑块越往下（单调）——「到顶贴顶、到底贴底」的直接推论
    assert!(
        thumb_ys[0] < thumb_ys[1] && thumb_ys[1] < thumb_ys[2],
        "滑块应当随偏移单调下移，实际 {thumb_ys:?}"
    );
    println!("  ① 滑块随偏移单调下移 {thumb_ys:?} ✅");

    // ② 内容没超出 ⇒ 一条都不画（把上限换成一个「能装下」的容器来验证）
    let mut small = Node::new(Kind::Column, "small");
    small.layout.scroll = true;
    small.layout.height = Some(Size::Px(400.0));
    for i in 0..2 {
        let mut b = Node::new(Kind::Button, format!("s{i}"));
        b.props.label = Some("x".into());
        b.layout.height = Some(Size::Px(20.0));
        small.children.push(b);
    }
    let (geo2, m2) = layout_with_scroll(
        &small,
        Rect {
            x: 0.0,
            y: 0.0,
            w: W,
            h: 400.0,
        },
        style,
        &ApproxMeasure,
        &ScrollOffsets::new(),
    );
    assert_eq!(m2.max_of("small"), 0, "前置：这个小容器不该需要滚动");
    let mut s2 = UiState::default();
    s2.scroll.set_metrics(&m2);
    let bars2 = bars_in(&small, &geo2, &s2, "small");
    assert!(
        bars2.is_empty(),
        "内容装得下时不该画滚动条（满格滑块是噪音），实际 {bars2:?}"
    );
    println!("  ② 内容装得下 ⇒ 不画 ✅");

    // ③ 不喂滚动状态 ⇒ 也不画（opt-in：既有语料一个像素都不变）
    let bars3 = bars_in(&col, &geo, &UiState::default(), "list");
    assert!(
        bars3.is_empty(),
        "没喂滚动状态时不该画任何滚动条（这是让既有像素判据不受影响的保证）"
    );
    println!("  ③ 不喂滚动状态 ⇒ 不画（opt-in）✅");

    println!("\n全部自检通过。把 `UiState::scroll` 灌进交互状态这一件事由");
    println!("`UiState::to_interact_state()` 统一完成 —— 它把偏移与上限一起带过去。");
}

/// 这一帧里落在**视口右边缘那条轨道**上的 `FillRoundRect` 命令（轨道 + 滑块）。
///
/// 判据刻意按**位置**筛，而不是「列表里第几条」—— 后者会被无关的绘制命令改动弄红。
fn bars_in(
    tree: &Node,
    geo: &deer_gui::layout::Geometry,
    state: &UiState,
    container: &str,
) -> Vec<RectI> {
    let interact: InteractState = state.to_interact_state();
    let list = InteractiveRenderer::new(Theme::default(), &ApproxMeasure, &interact).build(tree, geo);
    // 轨道条带按**容器的真实矩形**算（不要用「我以为的视口尺寸」—— 实测这一步最容易错：
    // 容器的宽度是它的固有宽度，不一定等于外面那个盒子）
    let f = geo.get(container).expect("容器必须有几何");
    let strip_x = f.x as i32 + f.w as i32 - deer_gui::layout::SCROLLBAR_W as i32 - 2;
    list.cmds
        .iter()
        .filter_map(|c| match c {
            DrawCmd::FillRoundRect { rect, .. } if rect.x >= strip_x => Some(*rect),
            _ => None,
        })
        .collect()
}
