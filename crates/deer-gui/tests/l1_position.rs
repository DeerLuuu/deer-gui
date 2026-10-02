//! L1 层叠序（声明序）在**绘制与交互层**的判据。
//!
//! deer-core 侧的几何 / 命中判据见 `crates/deer-core/tests/l1_position.rs`；
//! 本文件补齐 ③ 的另一半：
//! 1. **绘制列表顺序断言** —— 流外节点按**声明序**发命令（后声明者后画）；
//! 2. **交互层 `hit` 的层叠优先** —— 输入路由与绘制用同一个层叠序；
//! 3. 一条**像素级**证据：后声明的流外节点真的盖在先声明的流内兄弟上面。

use deer_gui::interaction::{ClipSnapshot, hit};
use deer_gui::prelude::*;

use deer_core::draw::DrawCmd;

const STYLE: TextStyle = TextStyle {
    font_size: 13.0,
    line_height: 18.0,
};

const W: f32 = 200.0;
const H: f32 = 120.0;

/// 重叠语料：流内按钮 `a`（先声明）、流外按钮 `p`（后声明，偏移 (0,0) ⇒ 恰好压在 a 上）。
fn overlap_tree(overlay_disabled: bool) -> deer_gui::Node {
    let mut bar = deer_gui::layout::node::Node::new(deer_gui::layout::node::Kind::Row, "bar")
        .with_layout(L::new().pad(10.0).to_props());
    let mut a = deer_gui::layout::node::Node::new(deer_gui::layout::node::Kind::Button, "a")
        .with_label("A");
    a.layout.width = Some(Size::Px(36.0));
    a.layout.height = Some(Size::Px(22.0));
    let mut p = deer_gui::layout::node::Node::new(deer_gui::layout::node::Kind::Button, "p")
        .with_label("P");
    p.layout.width = Some(Size::Px(36.0));
    p.layout.height = Some(Size::Px(22.0));
    p.layout.position = Some(Pos::Offset { x: 0, y: 0 });
    if overlay_disabled {
        p.props.disabled = true;
    }
    bar.children.push(a);
    bar.children.push(p);
    let mut app = deer_gui::layout::node::Node::new(deer_gui::layout::node::Kind::Column, "app");
    app.children.push(bar);
    app
}

/// ③-a 绘制列表顺序：**后声明者后画**。
///
/// `NullRenderer` 给每个有几何的节点发一条 `NodeHint`，顺序 = 树的先序（= 声明序）。
/// 这里用 id 指纹认领各自的提示，断言流外 `p` 的提示排在流内 `a` **之后** ——
/// 与「positioned 只是脱离流内布局、不改绘制顺序」一致。
#[test]
fn t3_draw_list_paints_in_declaration_order() {
    let tree = overlap_tree(false);
    let geo = deer_gui::layout::layout::layout(&tree, Rect::new(0.0, 0.0, W, H), STYLE, &ApproxMeasure);
    // 前置：两个节点几何重叠（否则「谁后画」没有判别力）
    assert_eq!(geo["a"], geo["p"], "前置：a 与 p 的矩形必须重合");

    let list = deer_gui::gpu::render::NullRenderer::build(&tree, &geo);
    let fp_of = |c: &DrawCmd| match c {
        DrawCmd::NodeHint { node_id_fp, .. } => Some(*node_id_fp),
        _ => None,
    };
    let fp = |id: &str| deer_gui::layout::draw::node_id_fp(id);
    let ia = list.cmds.iter().filter_map(fp_of).position(|f| f == fp("a"));
    let ip = list.cmds.iter().filter_map(fp_of).position(|f| f == fp("p"));
    assert_eq!(ia.map(|_| ()), Some(()), "前置：a 的提示必须存在");
    assert_eq!(ip.map(|_| ()), Some(()), "前置：p 的提示必须存在");
    assert!(
        ia.unwrap() < ip.unwrap(),
        "流外节点必须按**声明序**绘制：p（后声明）的命令要排在 a（先声明）之后"
    );
}

/// ③-b 交互层的命中优先级：`hit`（= `hit_test` + 裁剪/禁用）同样**后声明者胜出**。
#[test]
fn t3_interaction_hit_prefers_the_later_declared_overlay() {
    let tree = overlap_tree(false);
    let geo = deer_gui::layout::layout::layout(&tree, Rect::new(0.0, 0.0, W, H), STYLE, &ApproxMeasure);
    let probe = (20.0, 20.0);
    let got = hit(&tree, &geo, ClipSnapshot::unclipped(), probe.0, probe.1)
        .map(|n| n.id.to_string())
        .expect("重叠点上必须有命中");
    assert_eq!(got, "p", "交互层命中必须与绘制层叠一致：后声明的流外节点优先");
}

/// ③-c 像素级证据：后声明的**禁用**流外按钮（border 底色）盖住先声明的普通按钮（accent 底色）。
///
/// 禁用只是换色（禁用子树照样画），所以用它当「上层是谁」的**可读色标**；
/// 若层叠序坏掉（先声明者后画 / position 失效），这个像素会是 accent 而不是 border。
#[test]
fn t3_topmost_pixel_belongs_to_the_later_declared_overlay() {
    let theme = Theme::default();
    let tree = overlap_tree(true);
    let (w, _h, px) = deer_gui::render_tree_to_rgba(&tree, W as u32, H as u32, theme.clone())
        .expect("离屏渲染");
    // (12,15) 在两个按钮矩形内、避开按钮文字（文字从 x≈24 开始）
    let i = ((15 * w as usize) + 12) * 4;
    let (r, g, b) = (px[i], px[i + 1], px[i + 2]);
    let border = theme.border;
    let accent = theme.accent;
    assert_eq!(
        (r, g, b),
        (border.r, border.g, border.b),
        "重叠点的像素必须是后声明（禁用流外）按钮的 border 色，实际 rgb({r},{g},{b})"
    );
    assert_ne!(
        (r, g, b),
        (accent.r, accent.g, accent.b),
        "该像素不可能是先声明按钮的 accent 色 —— 否则层叠序坏了"
    );
}
