//! L4 anchors 锚定（D6/D10/Q5 裁定：与 position **同一个机制**）的测试套件。
//!
//! 语义红线（登记在 `ROADMAP.md` D10 + `node.rs::Pos::Anchors`）：
//! 1. **opt-in**：不用 anchors 的树逐字节不变（判据 = 既有全套测试保持绿 + ⑦-e 编码不变）；
//! 2. **同属流外**：与 `Offset` 共用同一条判据（`is_positioned`）—— 不占流、不计入父固有尺寸；
//! 3. **一轴双锚 ⇒ 尺寸由锚点对导出**（显式 w/h 不参与）；只锚一边 ⇒ 显式/固有尺寸；
//! 4. **resize 是本特性的存在意义**：父盒子变大，锚定边跟随新盒、偏移保持（② 专项）；
//! 5. min/max（L3）照常夹取最终尺寸；滚动平移与 `Offset` 同一套；
//! 6. 声明序层叠、`hit_test` 零改动（同 L1）。
//!
//! 偏移是**内缩式**：起点边（l/t）+ox、终点边（r/b）−ox（正 = 向内、负 = 向外）。
//! 这套代数是 L4 落地时定死的（理由写在 `layout.rs` 的 L4 落位段）：
//! 「撑满内缩」（③）与「角标跨越角点」（④）两个场景共用同一对 ox/oy 的唯一解。
//!
//! 每条断言都先有**前置断言**（内容盒确实变了 / 几何确实重叠），再做目标断言 ——
//! 否则守卫自己在测空气。双向变异（把「锚点比例 × 内宽」改坏成 l 恒按 0 ⇒
//! ② 必须红）的记录见交付报告。

use deer_core::builder::L;
use deer_core::layout::{ApproxMeasure, ScrollOffsets, TextStyle, hit_test, layout, layout_with_scroll, measure_tree};
use deer_core::node::{Kind, Node, Pos, Rect, Size};
use deer_core::scene::{encode_scene, parse_scene};

const STYLE: TextStyle = TextStyle {
    font_size: 13.0,
    line_height: 18.0,
};

fn geo(tree: &Node, w: f32, h: f32) -> deer_core::Geometry {
    layout(tree, Rect::new(0.0, 0.0, w, h), STYLE, &ApproxMeasure)
}

fn anchors(l: Option<f32>, t: Option<f32>, r: Option<f32>, b: Option<f32>, ox: i32, oy: i32) -> Option<Pos> {
    Some(Pos::Anchors { l, t, r, b, ox, oy })
}

/// 锚定树通用的父容器：`w=h=100%` ⇒ 根矩形 = 给的盒子（盒变 ⇒ 内容盒跟着变），
/// `pad=10` ⇒ 内容盒 = 盒子四边各缩 10。
fn sized_root() -> Node {
    let mut app = Node::new(Kind::Column, "app");
    app.layout.width = Some(Size::Pct(100.0));
    app.layout.height = Some(Size::Pct(100.0));
    app.layout.padding = 10.0;
    app
}

// ─────────────────────────────────────────────────────────────────────────────
// ② resize 专项（本特性核心）：同一棵锚定树，两种盒子 ⇒ 锚定边跟随、偏移保持
// ─────────────────────────────────────────────────────────────────────────────

/// ② 的语料：两个锚定子节点。
/// - `half`：l=0.5, t=0.25, r=1, b=1（两轴双锚 ⇒ 尺寸全由锚点对导出；
///   l/t **非 0** ⇒ 「锚点比例 × 内宽」这项有判别力，变异轮改坏它 ② 必红）；
/// - `pin`：只锚右缘（r=1，无 l/t/b），显式 40×20，ox=-6 ⇒ 右缘悬出内容盒右缘 6px。
fn resize_tree() -> Node {
    let mut app = sized_root();
    let mut half = Node::new(Kind::Button, "half").with_label("H");
    half.layout.position = anchors(Some(0.5), Some(0.25), Some(1.0), Some(1.0), 0, 0);
    let mut pin = Node::new(Kind::Button, "pin").with_label("P");
    pin.layout.width = Some(Size::Px(40.0));
    pin.layout.height = Some(Size::Px(20.0));
    pin.layout.position = anchors(None, None, Some(1.0), None, -6, 0);
    app.children.push(half);
    app.children.push(pin);
    app
}

#[test]
fn t2_anchored_edges_follow_the_box_on_resize() {
    let tree = resize_tree();

    // 小盒 200×100：内容盒 = (10,10,180,80)
    let small = geo(&tree, 200.0, 100.0);
    // 前置：内容盒确实是「盒子内缩 10」，且两种盒子下**不同**（否则这不是 resize 测试）
    assert_eq!(small["app"], Rect::new(0.0, 0.0, 200.0, 100.0), "前置：根 = 给的盒子（100%）");
    assert_eq!(
        (small["app"].x + 10.0, small["app"].y + 10.0, small["app"].w - 20.0, small["app"].h - 20.0),
        (10.0, 10.0, 180.0, 80.0),
        "前置：pad=10 ⇒ 内容盒 = (10,10,180,80)"
    );

    // half：x0 = 10 + 0.5×180 = 100，w = 0.5×180 = 90；y0 = 10 + 0.25×80 = 30，h = 0.75×80 = 60
    assert_eq!(
        small["half"],
        Rect::new(100.0, 30.0, 90.0, 60.0),
        "双锚 ⇒ 位置与尺寸都由锚点比例 × 内宽决定（小盒）"
    );
    // 锚定边贴盒：右/下缘 = 内容盒右/下缘
    assert_eq!(small["half"].x + small["half"].w, 190.0, "r=1 的边贴内容盒右缘（小盒）");
    assert_eq!(small["half"].y + small["half"].h, 90.0, "b=1 的边贴内容盒下缘（小盒）");

    // 大盒 400×300：内容盒 = (10,10,380,280) —— 同一棵树，只改盒子
    let big = geo(&tree, 400.0, 300.0);
    assert_eq!(big["app"], Rect::new(0.0, 0.0, 400.0, 300.0), "前置：盒子确实变了");
    assert_eq!(
        big["half"],
        Rect::new(200.0, 80.0, 190.0, 210.0),
        "锚定边跟随新盒：0.5×380=190 起点、0.5×380=190 宽；0.25×280=70 起点、0.75×280=210 高"
    );
    assert_eq!(big["half"].x + big["half"].w, 390.0, "r=1 的边贴**新**内容盒右缘（resize 的存在意义）");
    assert_eq!(big["half"].y + big["half"].h, 290.0, "b=1 的边贴**新**内容盒下缘");

    // pin（只锚右缘）：右缘 = 内容盒右缘 − ox = +6（悬出），左缘退自身显式宽 40
    assert_eq!(small["pin"], Rect::new(156.0, 10.0, 40.0, 20.0), "小盒：右缘 190+6=196");
    assert_eq!(big["pin"], Rect::new(356.0, 10.0, 40.0, 20.0), "大盒：右缘 390+6=396");
    // 偏移保持：右缘相对内容盒右缘的距离在两种盒子下**相等**
    assert_eq!(
        (small["pin"].x + small["pin"].w) - (small["app"].x + small["app"].w - 10.0),
        (big["pin"].x + big["pin"].w) - (big["app"].x + big["app"].w - 10.0),
        "resize 时偏移保持（悬出量恒为 6px）"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// ③ 撑满：l=0,t=0,r=1,b=1 + ox=oy=8 ⇒ 内容盒四边各内缩 8px
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn t3_full_fill_with_inset_offsets_equals_inset_content_box() {
    let mut app = sized_root();
    let mut fill = Node::new(Kind::Button, "fill").with_label("F");
    fill.layout.position = anchors(Some(0.0), Some(0.0), Some(1.0), Some(1.0), 8, 8);
    app.children.push(fill);
    let tree = app;

    let g = geo(&tree, 200.0, 100.0);
    // 内容盒 (10,10,180,80)，四边各缩 8 ⇒ (18,18,164,64)
    assert_eq!(
        g["fill"],
        Rect::new(18.0, 18.0, 164.0, 64.0),
        "撑满 + 内缩式偏移 = 内容盒内缩 8px（内缩代数：左 +8、右 −8）"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// ④ 角标：l=r=1,t=b=1 + 负偏移 ⇒ 徽标跨越内容盒角点居中；显式 w/h 被忽略（不是报错）
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn t4_corner_badge_straddles_the_content_box_corner() {
    let mut app = sized_root();
    let mut badge = Node::new(Kind::Button, "badge").with_label("!");
    badge.layout.width = Some(Size::Px(99.0)); // 混进一个显式宽 —— 双锚轴必须**忽略**它（D10）
    badge.layout.height = Some(Size::Px(99.0));
    badge.layout.position = anchors(Some(1.0), Some(1.0), Some(1.0), Some(1.0), -8, -8);
    app.children.push(badge);
    let tree = app;

    let g = geo(&tree, 200.0, 100.0);
    // 内容盒角点 = (10+180, 10+80) = (190,90)；ox=oy=-8 ⇒ 两缘在角点两侧各 8 ⇒ 16×16 居中于角点
    assert_eq!(
        g["badge"],
        Rect::new(182.0, 82.0, 16.0, 16.0),
        "角标 = 跨越角点居中（(l−r)×内宽 = 0 ⇒ 尺寸只由偏移撑出）"
    );
    assert_eq!(g["badge"].w, 16.0, "双锚轴的显式 w 被忽略（不是报错，D10 定死）");
    assert_eq!(
        (g["badge"].x + g["badge"].w / 2.0, g["badge"].y + g["badge"].h / 2.0),
        (190.0, 90.0),
        "徽标中心 = 内容盒角点"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// ⑤ 只锚一边 ⇒ 另一边/另一轴用显式或固有尺寸
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn t5_single_anchor_keeps_explicit_or_intrinsic_size_on_the_other_side() {
    let mut app = sized_root();
    // t=0.5 单锚 + 显式 30×20：y 按锚、x 无锚 ⇒ x = 内容盒原点 + ox(0)
    let mut exp = Node::new(Kind::Button, "exp").with_label("E");
    exp.layout.width = Some(Size::Px(30.0));
    exp.layout.height = Some(Size::Px(20.0));
    exp.layout.position = anchors(None, Some(0.5), None, None, 0, 0);
    // r=1.0 单锚 + 无显式 ⇒ 右缘贴内容盒右缘、向左退**固有**尺寸（1 字符按钮 = 28×22）
    let mut int = Node::new(Kind::Button, "int").with_label("I");
    int.layout.position = anchors(None, None, Some(1.0), None, 0, 0);
    app.children.push(exp);
    app.children.push(int);
    let tree = app;

    let g = geo(&tree, 200.0, 100.0);
    // 前置：按钮固有尺寸确实是 28×22（ApproxMeasure：max(28, 8+20)=28，max(22,18)=22）
    assert_eq!((g["int"].w, g["int"].h), (28.0, 22.0), "前置：固有尺寸 28×22");
    assert_eq!(g["exp"], Rect::new(10.0, 50.0, 30.0, 20.0), "y 单锚 t=0.5 ⇒ y0 = 10+0.5×80；x 用显式");
    assert_eq!(
        g["int"],
        Rect::new(10.0 + 180.0 - 28.0, 10.0, 28.0, 22.0),
        "l=1 单锚 ⇒ 右缘贴盒，宽取**固有** 28（显式没有）"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// ⑥ min/max 与 anchors 共存：L3 夹取**最终尺寸**（夹取时起点锚保持、终点边让步）
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn t6_min_max_clamps_the_anchor_derived_size() {
    // 控制组：无 min/max ⇒ 宽 = 锚点距 = 180（前置基准）
    let mut app = sized_root();
    let mut base = Node::new(Kind::Button, "base").with_label("B");
    base.layout.position = anchors(Some(0.0), Some(0.0), Some(1.0), Some(1.0), 0, 0);
    app.children.push(base);
    let g = geo(&app, 200.0, 100.0);
    assert_eq!(g["base"].w, 180.0, "前置：无 min/max ⇒ 宽 = 内容盒内宽 180");

    // min 托底：锚点距 180 < min 220 ⇒ min 赢（起点锚 l=0 保持，右缘让步伸出盒子）
    let mut app = sized_root();
    let mut lo = Node::new(Kind::Button, "lo").with_label("L");
    lo.layout.min_w = Some(Size::Px(220.0));
    lo.layout.position = anchors(Some(0.0), Some(0.0), Some(1.0), Some(1.0), 0, 0);
    app.children.push(lo);
    let g = geo(&app, 200.0, 100.0);
    assert_eq!(g["lo"], Rect::new(10.0, 10.0, 220.0, 80.0), "min_w 夹取锚定尺寸：min 赢、起点锚保持");

    // max 封顶：锚点距 80 > max 30 ⇒ max 赢
    let mut app = sized_root();
    let mut hi = Node::new(Kind::Button, "hi").with_label("H");
    hi.layout.max_h = Some(Size::Px(30.0));
    hi.layout.position = anchors(Some(0.0), Some(0.0), Some(1.0), Some(1.0), 0, 0);
    app.children.push(hi);
    let g = geo(&app, 200.0, 100.0);
    assert_eq!(g["hi"], Rect::new(10.0, 10.0, 180.0, 30.0), "max_h 夹取锚定尺寸：终点边让步");
}

// ─────────────────────────────────────────────────────────────────────────────
// 流外判据共用 + 滚动平移 + 层叠命中（与 Offset 同一套的三条红线）
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn anchored_child_shares_the_out_of_flow_rule() {
    // 与 l1_position.rs ② 同款语料，只是 pos 换成 anchors ⇒ 同一条判据必须同样生效
    let mut bar = Node::new(Kind::Row, "bar").with_layout(L::new().pad(10.0).to_props());
    let mut a = Node::new(Kind::Button, "a").with_label("A");
    a.layout.width = Some(Size::Px(36.0));
    a.layout.height = Some(Size::Px(22.0));
    let mut p = Node::new(Kind::Button, "p").with_label("P");
    p.layout.width = Some(Size::Px(36.0));
    p.layout.height = Some(Size::Px(22.0));
    p.layout.position = anchors(Some(0.0), Some(0.0), None, None, 100, 50);
    assert!(p.is_positioned(), "前置：anchors 与 Offset 共用 is_positioned 判据");
    bar.children.push(a);
    bar.children.push(p);
    let mut app = Node::new(Kind::Column, "app");
    app.children.push(bar);
    let tree = app;

    // 父固有尺寸不含它：56 = 流内 36 + padding 20（不是 92）
    let intr = measure_tree(&tree, STYLE, &ApproxMeasure);
    assert_eq!(intr["bar"], (56.0, 42.0), "锚定子节点同样不计入父容器固有尺寸");

    let g = geo(&tree, 400.0, 300.0);
    assert_eq!(g["a"].x, 10.0, "流内兄弟不许被锚定节点挤动");
    assert_eq!(g["p"], Rect::new(110.0, 60.0, 36.0, 22.0), "l=0/t=0 + 偏移 ⇒ 与 Offset 同款落位");
}

#[test]
fn scroll_translation_applies_to_anchored_children() {
    // list：可滚动 Column（视口 100 高、内容 300 高 ⇒ max_scroll=200）；
    // tip：锚定在 list 左上角（l=0,t=0,ox=oy=4）。滚动 50 ⇒ tip 与流内内容一起上移 50。
    let mut app = sized_root();
    let mut list = Node::new(Kind::Column, "list").with_layout(L::new().scroll(true).to_props());
    list.layout.width = Some(Size::Pct(100.0));
    list.layout.height = Some(Size::Px(100.0));
    let mut body = Node::new(Kind::Text, "body").with_label("x");
    body.layout.height = Some(Size::Px(300.0));
    let mut tip = Node::new(Kind::Button, "tip").with_label("T");
    tip.layout.width = Some(Size::Px(20.0));
    tip.layout.height = Some(Size::Px(10.0));
    tip.layout.position = anchors(Some(0.0), Some(0.0), None, None, 4, 4);
    list.children.push(body);
    list.children.push(tip);
    app.children.push(list);
    let tree = app;

    let scrolled = layout_with_scroll(
        &tree,
        Rect::new(0.0, 0.0, 220.0, 160.0),
        STYLE,
        &ApproxMeasure,
        &ScrollOffsets::new().with("list", 50),
    );
    // 前置：确实滚得动（内容 300 > 视口 100）
    assert_eq!(scrolled.1.max_of("list"), 200, "前置：max_scroll = 300−100 = 200");

    let g = scrolled.0;
    // app pad=10 ⇒ list 在 (10,10)、w = 100% × 内宽 200 = 200（容器自身矩形不动）
    assert_eq!(g["list"], Rect::new(10.0, 10.0, 200.0, 100.0), "容器自身矩形不动");
    // tip 静止位 = (10+0×200+4, 10+0×100+4) = (14,14)；滚动 50 ⇒ y = 14−50 = −36
    assert_eq!(g["tip"], Rect::new(14.0, -36.0, 20.0, 10.0), "锚定节点 = 静止位 (14,14) 平移 −50");

    // 对照：偏移 0 ⇒ 与静止位逐位相同（同一条代码路径，滚动只是加法）
    let rest = geo(&tree, 220.0, 160.0);
    assert_eq!(rest["tip"], Rect::new(14.0, 14.0, 20.0, 10.0), "偏移 0 ⇒ 锚定节点停在锚点位");
}

#[test]
fn later_declared_anchored_sibling_wins_the_hit() {
    let mut bar = Node::new(Kind::Row, "bar").with_layout(L::new().pad(10.0).to_props());
    let mut a = Node::new(Kind::Button, "a").with_label("A");
    a.layout.width = Some(Size::Px(36.0));
    a.layout.height = Some(Size::Px(22.0));
    let mut z = Node::new(Kind::Button, "z").with_label("Z");
    z.layout.width = Some(Size::Px(36.0));
    z.layout.height = Some(Size::Px(22.0));
    z.layout.position = anchors(Some(0.0), Some(0.0), None, None, 0, 0); // 恰好压在 a 上
    bar.children.push(a);
    bar.children.push(z);
    let mut app = Node::new(Kind::Column, "app");
    app.children.push(bar);
    let tree = app;

    let g = geo(&tree, 400.0, 300.0);
    assert_eq!(g["a"], g["z"], "前置：两矩形重合（l=t=0 + 零偏移 = 内容盒原点）");
    let hit = hit_test(&tree, &g, 20.0, 20.0).expect("重叠点应命中");
    assert_eq!(hit.id, "z", "hit_test 零改动：声明序层叠照常（后声明的锚定节点胜出）");
}

// ─────────────────────────────────────────────────────────────────────────────
// ⑦ `.dui` 语法：逐边属性 + 规范形往返 + 坏值硬报错 + opt-in 编码不变
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn t7_dui_per_side_attrs_parse_and_round_trip() {
    let src = "[column name=app pad=10]\n\
               \x20 [button name=fill label=F anchor-l=0 anchor-t=0 anchor-r=1 anchor-b=1 anchor-ox=8 anchor-oy=8]\n\
               \x20 [button name=pin label=P anchor-r=1 anchor-ox=-6]\n";
    let (tree, warnings) = deer_core::scene::parse_scene_collect(src, "t.dui").expect("能解析");
    assert!(warnings.is_empty(), "anchor-* 是已知属性，不该有警告：{warnings:?}");
    let fill = &tree.children[0];
    let pin = &tree.children[1];
    assert_eq!(
        fill.layout.position,
        anchors(Some(0.0), Some(0.0), Some(1.0), Some(1.0), 8, 8),
        "逐边属性 ⇒ Pos::Anchors"
    );
    assert_eq!(
        pin.layout.position,
        anchors(None, None, Some(1.0), None, -6, 0),
        "没写的边 = 无锚（不是 0），偏移缺省 0"
    );
    assert!(pin.is_positioned(), "只写一个 anchor-* 也算流外（同一个 position 字段）");

    // 往返：编码成规范形 `pos=anchors:…`，再读回结构相等，二次编码逐字节稳定
    let once = encode_scene(&tree);
    assert!(
        once.contains("pos=anchors:0,0,1,1,8,8"),
        "规范形必须可逆编码：{once}"
    );
    assert!(once.contains("pos=anchors:-,-,1,-,-6,0"), "无锚边编码成 `-`：{once}");
    let back = parse_scene(&once, "rt.dui").expect("规范形必须能读回");
    assert!(tree.structurally_eq(&back), "逐边写法 ↔ 规范形：往返必须结构相等\n{once}");
    assert_eq!(encode_scene(&back), once, "二次编码必须逐字节稳定");
}

#[test]
fn t7_dui_canonical_pos_form_parses_directly() {
    let tree = parse_scene("[button name=b pos=anchors:0.5,0.25,1,1,0,0]\n", "c.dui").expect("能读");
    assert_eq!(
        tree.layout.position,
        anchors(Some(0.5), Some(0.25), Some(1.0), Some(1.0), 0, 0),
        "pos=anchors:… 是 to_attr 的规范形，Pos::parse 直接认得"
    );
}

#[test]
fn t7_parse_and_to_attr_are_mutual_inverses() {
    let cases = vec![
        Pos::Offset { x: -3, y: 17 },
        anchors(Some(0.0), None, Some(1.0), Some(0.25), -8, 8).unwrap(),
        anchors(None, None, None, None, 0, 0).unwrap(),
        anchors(Some(1.5), Some(-0.5), Some(2.0), None, 100, -100).unwrap(),
    ];
    for p in cases {
        assert_eq!(
            Pos::parse(&p.to_attr()),
            Some(p),
            "parse(to_attr(p)) == p 对两个变体都成立（往返判据的 API 层）"
        );
    }
}

#[test]
fn t7_anchor_bad_values_are_hard_errors() {
    // 已知属性写错必须硬报错（Q6 的放宽只针对「未知」属性）
    let bad = [
        "[button name=b anchor-l=abc]\n",                       // 比例不是数字
        "[button name=b anchor-l]\n",                           // 裸属性（开关写法）不行
        "[button name=b anchor-ox=8.5]\n",                      // 偏移必须是整数像素
        "[button name=b anchor-oy=q]\n",                        // 偏移不是数字
        "[button name=b pos=anchors:1,2,3]\n",                  // 规范形缺字段
        "[button name=b pos=anchors:1,2,3,4,5,6,7]\n",          // 规范形多字段
        "[button name=b pos=anchors:a,b,c,d,e,f]\n",            // 规范形非数字
        "[button name=b pos=0,0 anchor-l=1]\n",                 // pos 与 anchor-* 混用（同一字段两种设法）
        "[button name=b pos=anchors:1,2,3,4,5,6 anchor-r=1]\n", // 规范形与逐边也互斥
    ];
    for src in bad {
        let e = parse_scene(src, "bad.dui").expect_err(&format!("{src} 应当报错"));
        let m = format!("{e}");
        assert!(
            m.contains("anchor") || m.contains("pos"),
            "错误信息要点名属性：{m}"
        );
    }
}

#[test]
fn t7_dui_without_anchors_is_byte_identical() {
    // opt-in 红线在语法面的体现：不用 anchors ⇒ 编码不多写一个字节
    let out = encode_scene(&parse_scene("[column name=app]\n  [button name=ok]\n", "f.dui").unwrap());
    assert_eq!(out, "# deer-gui-scene: 1\n[column name=app]\n  [button name=ok]\n");
    assert!(!out.contains("anchor"), "未设 anchors 时编码不多写任何东西：{out}");
}

// ─────────────────────────────────────────────────────────────────────────────
// 边界数值：锚可以超 [0,1]；负锚点距不产生负宽
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn anchors_beyond_unit_range_and_degenerate_pairs() {
    let mut app = sized_root();
    // l=-0.5（伸出左缘）、r=1.25（伸出右缘）：可超 [0,1]
    let mut wide = Node::new(Kind::Button, "wide").with_label("W");
    wide.layout.position = anchors(Some(-0.5), Some(0.0), Some(1.25), Some(1.0), 0, 0);
    // l=r=1、ox=0：锚点距 = 0 ⇒ 宽被钳成 0（不许出现负宽的矩形）
    let mut zero = Node::new(Kind::Button, "zero").with_label("0");
    zero.layout.position = anchors(Some(1.0), None, Some(1.0), None, 0, 0);
    app.children.push(wide);
    app.children.push(zero);
    let tree = app;

    let g = geo(&tree, 200.0, 100.0);
    assert_eq!(
        g["wide"],
        Rect::new(10.0 + -0.5 * 180.0, 10.0, 1.75 * 180.0, 80.0),
        "锚点比例可超 [0,1]：l=-0.5 ⇒ 左缘伸出 90px，r=1.25 ⇒ 右缘伸出 45px"
    );
    assert_eq!(g["zero"].w, 0.0, "锚点距为 0（r−l=0、ox=0）⇒ 宽 0，不是负数");
}
