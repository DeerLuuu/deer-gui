//! **多行文本的绘制命令** + **滚动容器的视口裁剪**（剩余工作第 1 项的渲染面）。
//!
//! 两个缺口（`docs/features/layout.md:104` 记的就是它们）在这里被钉住：
//!
//! 1. `Kind::Text` + `layout.wrap` ⇒ 渲染器把文本按**行**展开成 N 条 `DrawCmd::Text`
//!    （每行一条、各有自己的矩形）。单行语料仍然是**一条**命令 —— 这是「既有像素判据不变」
//!    的前提，所以它也被显式断言；
//! 2. `Column` + `layout.scroll` ⇒ 渲染器在视口上推 `PushClip` / `PopClip`，
//!    子节点的绘制命令落在裁剪栈**里面**（`NodeHint` 也在里面 ⇒ 命中侧派生出的
//!    `ClipSnapshot` 会把视口裁剪绑到子节点上）。
//!
//! ```sh
//! cargo test -q -p deer-gpu --test scroll_multiline
//! ```

use deer_gpu::render::{DefaultRenderer, NullRenderer, text_lines};
use deer_core::{ DrawCmd, RectI };
use deer_gpu::{ Theme };
use deer_core::builder::{Builder, L};
use deer_core::layout::{
    ApproxMeasure, Geometry, ScrollOffsets, TextStyle, layout, layout_with_scroll,
};
use deer_core::node::{Kind, Node, Rect, Size};

const STYLE: TextStyle = TextStyle {
    font_size: 13.0,
    line_height: 18.0,
};
const LABEL: &str = "alpha beta gamma delta";
/// `ApproxMeasure` @ 宽 60 的换行点（布局测试里同一份期望）。
const LINES: [&str; 4] = ["alpha", "beta", "gamma", "delta"];

// ---------------------------------------------------------------------------
// 语料
// ---------------------------------------------------------------------------

/// 一个滚动的列：视口 200×100、内容 8 × 22 = 176 ⇒ `max_scroll = 76`。
fn scroll_tree() -> Node {
    let mut app = Builder::new(Kind::Column, "app");
    app.container_opts(Kind::Column, "outer", L::new().w(200.0).h(100.0).scroll(true).to_props(), |o| {
        for i in 1..=8 {
            o.button(format!("{i}"));
        }
    });
    app.build()
}

/// 同构但**不滚动**的对照语料。
fn plain_tree() -> Node {
    let mut app = Builder::new(Kind::Column, "app");
    app.container_opts(Kind::Column, "outer", L::new().w(200.0).h(100.0).to_props(), |o| {
        for i in 1..=8 {
            o.button(format!("{i}"));
        }
    });
    app.build()
}

/// 一个换行的文本节点（`w=60`），可选显式高度。
fn wrap_tree(wrap: bool, height: Option<f32>) -> Node {
    let mut b = Builder::new(Kind::Column, "app");
    b.text_opts(LABEL, |n| {
        n.layout.width = Some(Size::Px(60.0));
        n.layout.height = height.map(Size::Px);
        n.layout.wrap = wrap;
    });
    b.build()
}

fn geo_of(tree: &Node, offsets: &ScrollOffsets) -> Geometry {
    layout_with_scroll(tree, Rect::new(0.0, 0.0, 200.0, 260.0), STYLE, &ApproxMeasure, offsets).0
}

fn kind_of(c: &DrawCmd) -> &'static str {
    match c {
        DrawCmd::FillRect { .. } => "fill",
        DrawCmd::StrokeRect { .. } => "stroke",
        DrawCmd::FillRoundRect { .. } => "round",
        DrawCmd::Text { .. } => "text",
        DrawCmd::PushClip { .. } => "push_clip",
        DrawCmd::PopClip => "pop_clip",
        DrawCmd::NodeHint { .. } => "node_hint",
    }
}

// ---------------------------------------------------------------------------
// 一、多行文本：一行一条命令
// ---------------------------------------------------------------------------

/// **换行 ⇒ 每行一条 `Text` 命令；不换行 ⇒ 恰好一条**（后者是既有像素判据的前提）。
#[test]
fn wrapped_text_emits_one_command_per_line_and_single_line_stays_one() {
    let theme = Theme::default();

    let tree_w = wrap_tree(true, None);
    let geo_w = layout(&tree_w, Rect::new(0.0, 0.0, 200.0, 260.0), STYLE, &ApproxMeasure);
    let node_rect = {
        let r = geo_w["text_1"];
        RectI::new(r.x as i32, r.y as i32, r.w as i32, r.h as i32)
    };
    println!("换行节点的矩形 = {node_rect:?}");
    assert_eq!(node_rect, RectI::new(0, 0, 60, 72), "前置：4 行 × 18 = 72");

    let list_w = DefaultRenderer::new(theme.clone(), &ApproxMeasure).build(&tree_w, &geo_w);
    let texts: Vec<(RectI, String)> = list_w
        .cmds
        .iter()
        .filter_map(|c| match c {
            DrawCmd::Text { rect, text, .. } => Some((*rect, text.clone())),
            _ => None,
        })
        .collect();
    println!("换行 ⇒ {} 条 Text：{texts:?}", texts.len());
    assert_eq!(texts.len(), LINES.len(), "行数 = 命令数（每行一条）");
    for (i, (rect, text)) in texts.iter().enumerate() {
        assert_eq!(*text, LINES[i], "第 {i} 行的换行点");
        assert_eq!(
            *rect,
            RectI::new(0, 18 * i as i32, 60, 18),
            "第 {i} 行的矩形 = 节点矩形按行高下移 {i} 行"
        );
        // 每一行都必须落在**节点自己**的矩形内（列表的几何不变式）。
        assert!(
            rect.x >= node_rect.x
                && rect.y >= node_rect.y
                && rect.right() <= node_rect.right()
                && rect.bottom() <= node_rect.bottom(),
            "第 {i} 行 {rect:?} 落在节点矩形 {node_rect:?} 之外"
        );
    }

    // 不换行：**恰好一条**命令，矩形就是节点矩形（与改动前逐字节相同）。
    let tree_p = wrap_tree(false, None);
    let geo_p = layout(&tree_p, Rect::new(0.0, 0.0, 200.0, 260.0), STYLE, &ApproxMeasure);
    let list_p = DefaultRenderer::new(theme, &ApproxMeasure).build(&tree_p, &geo_p);
    let texts_p: Vec<&DrawCmd> = list_p
        .cmds
        .iter()
        .filter(|c| matches!(c, DrawCmd::Text { .. }))
        .collect();
    println!(
        "不换行 ⇒ {} 条 Text｜节点矩形 {:?}",
        texts_p.len(),
        geo_p["text_1"]
    );
    assert_eq!(texts_p.len(), 1, "不换行必须**恰好一条**命令（既有像素判据的前提）");
    assert_eq!(
        *texts_p[0],
        DrawCmd::Text {
            rect: RectI::new(0, 0, 60, 18),
            text: LABEL.to_string(),
            color: theme_text(),
            size: theme_font_size(),
            align: 0,
        },
        "不换行的命令必须与既有行为逐字段相同"
    );
}

fn theme_text() -> deer_core::Color {
    Theme::default().text
}

fn theme_font_size() -> f32 {
    Theme::default().font_size
}

/// 节点装不下的行**不画**（命令矩形绝不越出节点矩形）—— 被钉住的边界。
#[test]
fn lines_that_do_not_fit_the_node_rect_are_dropped() {
    let theme = Theme::default();
    let tree = wrap_tree(true, Some(36.0));
    let geo = layout(&tree, Rect::new(0.0, 0.0, 200.0, 260.0), STYLE, &ApproxMeasure);
    println!("显式高 36 的换行节点 = {:?}", geo["text_1"]);
    assert_eq!(geo["text_1"].h, 36.0);
    let list = DefaultRenderer::new(theme, &ApproxMeasure).build(&tree, &geo);
    let rects: Vec<RectI> = list
        .cmds
        .iter()
        .filter_map(|c| match c {
            DrawCmd::Text { rect, .. } => Some(*rect),
            _ => None,
        })
        .collect();
    println!("只画得下 {rects:?}");
    assert_eq!(
        rects,
        vec![RectI::new(0, 0, 60, 18), RectI::new(0, 18, 60, 18)],
        "36 高只装得下 2 行（4 行里的后 2 行不画）"
    );
}

/// 行的矩形也夹到**行高**（节点很矮时不会画出一条比节点还高的行）。
#[test]
fn line_rect_is_clamped_to_the_node_bottom() {
    let tree = wrap_tree(true, None);
    let node = &tree.children[0];
    assert_eq!(node.id, "text_1", "测试前置：拿到的必须是那个文本节点");
    let rect = RectI::new(0, 0, 60, 25); // 1 行 18 + 尾巴 7
    let lines = text_lines(&ApproxMeasure, node, rect, STYLE);
    println!("矮节点的行 = {lines:?}");
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0].0, RectI::new(0, 0, 60, 18));
    assert_eq!(lines[1].0, RectI::new(0, 18, 60, 7), "第二行夹到节点下边界");
}

/// 关闭 `wrap` 时 `text_lines` 只回一行（**单行语料的字节不变**靠这条）。
#[test]
fn wrap_off_returns_exactly_the_node_rect() {
    let tree = wrap_tree(false, None);
    let node = &tree.children[0];
    assert_eq!(node.id, "text_1", "测试前置：拿到的必须是那个文本节点");
    assert_eq!(node.props.label.as_deref(), Some(LABEL), "测试前置：标签在");
    let r = RectI::new(3, 4, 60, 30);
    let lines = text_lines(&ApproxMeasure, node, r, STYLE);
    println!("wrap 关 ⇒ {lines:?}");
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0], (r, LABEL.to_string()));
}

// ---------------------------------------------------------------------------
// 二、滚动容器：视口裁剪
// ---------------------------------------------------------------------------

/// **可滚动容器 ⇒ 视口裁剪；不可滚动 ⇒ 一条裁剪命令都没有**（后者是「不动滚动时输出不变」）。
#[test]
fn scroll_container_clips_its_content_to_the_viewport() {
    let theme = Theme::default();
    let offsets = ScrollOffsets::new().with("outer", 40);
    let geo = geo_of(&scroll_tree(), &offsets);
    let list = DefaultRenderer::new(theme.clone(), &ApproxMeasure).build(&scroll_tree(), &geo);
    assert!(list.clip_balanced(), "裁剪栈必须平衡");

    let counts = list.counts();
    println!(
        "滚动语的列表：{} 条命令（push_clip {} / pop_clip {}）｜种类 {:?}",
        list.len(),
        counts.push_clip,
        counts.pop_clip,
        list.cmds.iter().map(kind_of).collect::<Vec<_>>()
    );
    assert_eq!(
        (counts.push_clip, counts.pop_clip),
        (1, 1),
        "恰好一对（外层视口）"
    );

    // 裁剪矩形 = 容器自己的矩形；且它**在容器自己的视觉之后、子节点之前**。
    let viewport = {
        let r = geo["outer"];
        RectI::new(r.x as i32, r.y as i32, r.w as i32, r.h as i32)
    };
    assert_eq!(viewport, RectI::new(0, 0, 200, 100), "前置：视口几何");
    let push_at = list
        .cmds
        .iter()
        .position(|c| matches!(c, DrawCmd::PushClip { .. }))
        .expect("有 PushClip");
    let pop_at = list
        .cmds
        .iter()
        .position(|c| matches!(c, DrawCmd::PopClip))
        .expect("有 PopClip");
    match &list.cmds[push_at] {
        DrawCmd::PushClip { rect } => assert_eq!(*rect, viewport, "裁剪矩形 = 视口矩形"),
        _ => unreachable!(),
    }
    assert!(push_at < pop_at, "推在前、弹在后");
    assert_eq!(pop_at, list.len() - 1, "外层容器的 PopClip 在最后");
    // 子节点的绘制命令**全部**落在裁剪栈里面（视口外的内容因此画不出来）。
    let child_cmds: Vec<&DrawCmd> = list.cmds[push_at + 1..pop_at].iter().collect();
    println!("裁剪内的子节点命令 = {} 条", child_cmds.len());
    assert_eq!(
        child_cmds.len(),
        8 * 2,
        "8 个按钮 × (底色 + 文字) —— 全部在裁剪栈内"
    );
    assert!(
        child_cmds
            .iter()
            .all(|c| !matches!(c, DrawCmd::NodeHint { .. })),
        "DefaultRenderer 不发 NodeHint（既有行为）"
    );

    // 对照组：**不滚动**的同一棵树 ⇒ 一条裁剪命令都没有（这就是「不动滚动 ⇒ 输出不变」）。
    let geo_plain = geo_of(&plain_tree(), &ScrollOffsets::new());
    let list_plain = DefaultRenderer::new(theme, &ApproxMeasure).build(&plain_tree(), &geo_plain);
    let c_plain = list_plain.counts();
    println!(
        "不滚动的对照：{} 条命令（push_clip {} / pop_clip {}）",
        list_plain.len(),
        c_plain.push_clip,
        c_plain.pop_clip
    );
    assert_eq!((c_plain.push_clip, c_plain.pop_clip), (0, 0), "非滚动容器不发裁剪栈");
}

/// `NullRenderer`（命中侧派生 `ClipSnapshot` 用的那份列表）也要表达视口裁剪。
#[test]
fn null_renderer_expresses_the_viewport_clip_too() {
    let offsets = ScrollOffsets::new().with("outer", 40);
    let geo = geo_of(&scroll_tree(), &offsets);
    let list = NullRenderer::build(&scroll_tree(), &geo);
    let counts = list.counts();
    println!(
        "NullRenderer：hint {}｜push_clip {} / pop_clip {}｜配平 {}",
        counts.node_hint,
        counts.push_clip,
        counts.pop_clip,
        list.clip_balanced()
    );
    assert!(list.clip_balanced());
    assert_eq!((counts.push_clip, counts.pop_clip), (1, 1));
    assert_eq!(counts.node_hint, geo.len(), "每个有几何的节点一条提示（既有协议）");
    // 顺序：容器自己的提示 → PushClip → 子节点的提示 → PopClip。
    let outer_hint = list
        .cmds
        .iter()
        .position(|c| matches!(c, DrawCmd::NodeHint { rect, .. } if *rect == RectI::new(0, 0, 200, 100)))
        .expect("outer 的提示");
    let push_at = list
        .cmds
        .iter()
        .position(|c| matches!(c, DrawCmd::PushClip { .. }))
        .expect("有 PushClip");
    println!("outer 提示在 #{outer_hint}、PushClip 在 #{push_at}");
    assert!(
        outer_hint < push_at,
        "容器自己的提示必须在裁剪**之前**（否则容器自身也被算成「被视口裁掉」）"
    );
}
