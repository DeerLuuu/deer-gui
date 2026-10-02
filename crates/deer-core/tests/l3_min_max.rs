//! L3 最小/最大尺寸（`min_w` / `max_w` / `min_h` / `max_h`，app-foundation 计划 L 线 ·
//! 设计登记 D6）的测试套件。
//!
//! 语义红线（登记在 `node.rs` 与 `docs/features/min-max-sizes.md`）：
//! 1. **opt-in**：四个都未设 ⇒ 既有树逐字节不变（判据 = 既有全套测试保持绿 +
//!    `.dui` 编码不多一字节）；
//! 2. **min 是下限、max 是上限，measure 与 place 两处都生效**（L1 的教训：只改一处必漏）：
//!    ① measure：固有尺寸先算再夹进 `[min, max]` ⇒ 容器聚合的是夹过的值；
//!    ② place：显式尺寸与 grow 分配结果同样被夹；
//! 3. **修复顺序**：来源（显式 > 父分配 > 固有）→ I-6 bound → min/max。
//!    min/max 是「修复」不是「来源」—— 它不参与「显式 vs 固有」的取舍；
//! 4. **min > max ⇒ min 赢**（先夹 max 再托 min）；
//! 5. **与视口无关**：滚动容器子节点主轴 bound = 无穷时 min/max 仍然生效；
//!    流外（positioned）子节点同样受 min/max 约束；
//! 6. 百分比相对**父内容盒**解析；measure 阶段没有父宽度 ⇒ 百分比只在 place 生效
//!    （与 `width`/`height` 的百分比同一惯例）。
//!
//! 每条断言先有**前置断言**（基线行为确实如此），再断言目标语义。
//! 双向变异记录见交付报告。

use deer_core::builder::L;
use deer_core::layout::{ApproxMeasure, ScrollOffsets, TextStyle, layout_with_scroll, measure_tree};
use deer_core::node::{Kind, Node, Rect, Size};
use deer_core::scene::{encode_scene, parse_scene};

const STYLE: TextStyle = TextStyle {
    font_size: 13.0,
    line_height: 18.0,
};

fn geo(tree: &Node, w: f32, h: f32) -> deer_core::Geometry {
    deer_core::layout::layout(tree, Rect::new(0.0, 0.0, w, h), STYLE, &ApproxMeasure)
}

/// 显式尺寸 (w×h) 的按钮（不设则是固有 (28, 22)）。
fn btn(id: &str, w: f32, h: f32) -> Node {
    let mut n = Node::new(Kind::Button, id).with_label(id);
    n.layout.width = Some(Size::Px(w));
    n.layout.height = Some(Size::Px(h));
    n
}

/// 固有尺寸按钮（label 一个字符 ⇒ (28, 22)，ApproxMeasure）。
fn bare_btn(id: &str) -> Node {
    Node::new(Kind::Button, id).with_label(id)
}

// ─────────────────────────────────────────────────────────────────────────────
// ① opt-in：不设 min/max ⇒ 逐字节不变
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn t1_unset_min_max_changes_nothing() {
    let mut bar = Node::new(Kind::Row, "bar").with_layout(L::new().pad(10.0).to_props());
    bar.children.push(btn("a", 20.0, 10.0));
    let mut app = Node::new(Kind::Column, "app");
    app.children.push(bar);
    let tree = app;

    let g = geo(&tree, 400.0, 300.0);
    assert_eq!(g["bar"], Rect::new(0.0, 0.0, 40.0, 30.0), "前置基线");
    assert_eq!(g["a"], Rect::new(10.0, 10.0, 20.0, 10.0));

    // 语法面：未设 ⇒ 编码不多写任何东西
    let out = encode_scene(&parse_scene("[column name=app]\n  [button name=ok]\n", "f.dui").unwrap());
    assert_eq!(out, "# deer-gui-scene: 1\n[column name=app]\n  [button name=ok]\n");
    assert!(
        !out.contains("min-w") && !out.contains("max-w") && !out.contains("min-h") && !out.contains("max-h"),
        "未设 min/max 时编码不多写任何东西：{out}"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// ② measure 侧：固有尺寸先算再夹 ⇒ 容器聚合的是夹过的值
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn t2_measure_intrinsic_is_capped_by_max() {
    let mut t = Node::new(Kind::Text, "t").with_label("abcd");
    // 前置：无 max ⇒ 固有宽 = ceil(4 × 13 × 0.6) = 32
    assert_eq!(measure_tree(&t, STYLE, &ApproxMeasure)["t"].0, 32.0, "前置基线");

    t.layout.max_w = Some(Size::Px(20.0));
    let intr = measure_tree(&t, STYLE, &ApproxMeasure);
    assert_eq!(intr["t"].0, 20.0, "固有宽被 max_w 封顶");

    // 容器聚合（交叉轴 = max(子)）用的也是夹过的值
    let mut col = Node::new(Kind::Column, "col");
    col.children.push(t);
    assert_eq!(
        measure_tree(&col, STYLE, &ApproxMeasure)["col"].0,
        20.0,
        "容器固有宽 = max(夹过的子宽)，不是 32"
    );

    // 高度方向同理：单行文本固有高 18，max_h=10 ⇒ 10
    let mut t2 = Node::new(Kind::Text, "t2").with_label("abcd");
    t2.layout.max_h = Some(Size::Px(10.0));
    assert_eq!(measure_tree(&t2, STYLE, &ApproxMeasure)["t2"].1, 10.0, "max_h 同样封顶固有高");
}

#[test]
fn t3_measure_intrinsic_is_floored_by_min() {
    let b = bare_btn("a");
    // 前置：按钮固有宽 = max(28, 8+20) = 28
    assert_eq!(measure_tree(&b, STYLE, &ApproxMeasure)["a"].0, 28.0, "前置基线");

    let mut b2 = bare_btn("a");
    b2.layout.min_w = Some(Size::Px(50.0));
    assert_eq!(measure_tree(&b2, STYLE, &ApproxMeasure)["a"].0, 50.0, "固有宽被 min_w 托底");

    // 主轴聚合（Row 主轴 = sum）用的也是托过的值
    let mut row = Node::new(Kind::Row, "row");
    row.children.push(b2);
    assert_eq!(
        measure_tree(&row, STYLE, &ApproxMeasure)["row"].0,
        50.0,
        "Row 固有宽 = sum(托过的子宽)"
    );

    // 高度方向：min_h=36 ⇒ 固有高 22 → 36
    let mut b3 = bare_btn("a");
    b3.layout.min_h = Some(Size::Px(36.0));
    assert_eq!(measure_tree(&b3, STYLE, &ApproxMeasure)["a"].1, 36.0, "min_h 同样托底固有高");
}

// ─────────────────────────────────────────────────────────────────────────────
// ③ place 侧：显式尺寸超 max 被夹、低于 min 被托；顺序 = 来源 → bound → min/max
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn t4_place_explicit_size_is_clamped() {
    // 超过 max 被夹：Row(inner 180×100)，按钮 w=100 max_w=50 ⇒ 50
    let mut bar = Node::new(Kind::Row, "bar").with_layout(L::new().pad(10.0).w(200.0).h(120.0).to_props());
    let mut a = btn("a", 100.0, 20.0);
    a.layout.max_w = Some(Size::Px(50.0));
    bar.children.push(a);
    let mut app = Node::new(Kind::Column, "app");
    app.children.push(bar);
    let g = geo(&app, 400.0, 300.0);
    assert_eq!(g["a"].w, 50.0, "显式宽度超 max_w 被封顶");
    assert_eq!(g["a"].x, 10.0, "后续兄弟的落位基于夹过的尺寸（这里只有一个子节点）");

    // 低于 min 被托：w=10 min_w=30 ⇒ 30
    let mut bar = Node::new(Kind::Row, "bar").with_layout(L::new().pad(10.0).w(200.0).h(120.0).to_props());
    let mut b = btn("b", 10.0, 20.0);
    b.layout.min_w = Some(Size::Px(30.0));
    bar.children.push(b);
    let mut app = Node::new(Kind::Column, "app");
    app.children.push(bar);
    let g = geo(&app, 400.0, 300.0);
    assert_eq!(g["b"].w, 30.0, "显式宽度低于 min_w 被托底");

    // 高度方向（Row 的交叉轴）：h=100 max_h=40，inner_h=100 ⇒ I-6 不夹、max_h 夹 ⇒ 40
    let mut bar = Node::new(Kind::Row, "bar").with_layout(L::new().pad(10.0).w(200.0).h(120.0).to_props());
    let mut c = btn("c", 20.0, 100.0);
    c.layout.max_h = Some(Size::Px(40.0));
    bar.children.push(c);
    let mut app = Node::new(Kind::Column, "app");
    app.children.push(bar);
    let g = geo(&app, 400.0, 300.0);
    assert_eq!(g["c"].h, 40.0, "交叉轴显式高度超 max_h 被封顶（inner_h=100 不背锅）");

    // 顺序钉：「显式 > 固有」是**来源**之争、「min/max」是事后的**修复** ——
    // 固有 28 的按钮显式 w=10、min_w=15 ⇒ 取显式 10 托到 15；
    // 若实现成「固有 28 赢了来源之争」，这里会是 28（必红）
    let mut bar = Node::new(Kind::Row, "bar").with_layout(L::new().pad(10.0).w(200.0).h(120.0).to_props());
    let mut d = btn("d", 10.0, 20.0);
    d.layout.min_w = Some(Size::Px(15.0));
    bar.children.push(d);
    let mut app = Node::new(Kind::Column, "app");
    app.children.push(bar);
    let g = geo(&app, 400.0, 300.0);
    assert_eq!(g["d"].w, 15.0, "来源 = 显式 10（不是固有 28），再被 min_w=15 托底");
}

// ─────────────────────────────────────────────────────────────────────────────
// ④ grow 分配结果：max 封顶（省下的空间停在原地）、min 托底（可溢出容器）
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn t5_grow_result_is_capped_by_max() {
    let mk = |max_w: Option<Size>| -> deer_core::Geometry {
        let mut bar = Node::new(Kind::Row, "bar").with_layout(L::new().w(300.0).h(40.0).to_props());
        for id in ["a", "b"] {
            let mut n = bare_btn(id);
            n.layout.grow = 1.0;
            n.layout.max_w = max_w;
            bar.children.push(n);
        }
        let mut app = Node::new(Kind::Column, "app");
        app.children.push(bar);
        geo(&app, 400.0, 300.0)
    };

    // 前置基线：无 max ⇒ 各分 (300-0)/2 = 150
    let base = mk(None);
    assert_eq!(base["a"].w, 150.0, "前置基线：grow 均分 300");
    assert_eq!(base["b"].x, 150.0);

    let g = mk(Some(Size::Px(80.0)));
    assert_eq!(g["a"].w, 80.0, "grow 分配结果被 max_w 封顶");
    assert_eq!(g["b"].x, 80.0, "光标推进用的是**夹过**的尺寸（若用 150，b.x 会是 150）");
    assert_eq!(g["b"].w, 80.0);
    // 封顶省下的空间不二次分配（简单优先，指南「做不到什么」登记）：停在 b 的右侧
    assert_eq!(g["b"].x + g["b"].w, 160.0, "剩 140 停在主轴末端前");
}

#[test]
fn t6_grow_result_is_floored_by_min() {
    // min_w 故意用**百分比**：measure 阶段不解析百分比（固有仍是 28）⇒ 托底只能发生在
    // place 的 grow 分配之后 —— 这条专门钉 **place 侧**的下限（measure 侧由 t3 钉）。
    let mk = |min_w: Option<Size>| -> deer_core::Geometry {
        let mut bar = Node::new(Kind::Row, "bar").with_layout(L::new().w(180.0).h(40.0).to_props());
        for id in ["a", "b"] {
            let mut n = bare_btn(id);
            n.layout.grow = 1.0;
            n.layout.min_w = min_w;
            bar.children.push(n);
        }
        let mut app = Node::new(Kind::Column, "app");
        app.children.push(bar);
        geo(&app, 400.0, 300.0)
    };

    // 前置基线：无 min ⇒ 固有 28 + 各分 (180-56)/2=62 ⇒ 90
    let base = mk(None);
    assert_eq!(base["a"].w, 90.0, "前置基线：grow 分到 90");
    assert_eq!(base["b"].x, 90.0);
    // 前置：百分比 min 不影响 measure（固有仍是 28，托底没在 measure 抢跑）
    let mut probe = bare_btn("a");
    probe.layout.grow = 1.0;
    probe.layout.min_w = Some(Size::Pct(75.0));
    let mut probe_row = Node::new(Kind::Row, "row");
    probe_row.children.push(probe);
    assert_eq!(
        measure_tree(&probe_row, STYLE, &ApproxMeasure)["a"].0,
        28.0,
        "前置：百分比 min 在 measure 阶段不生效（否则这条测不到 place 侧）"
    );

    let g = mk(Some(Size::Pct(75.0)));
    // 75% × 180 = 135 > grow 分到的 90 ⇒ 托到 135
    assert_eq!(g["a"].w, 135.0, "grow 分配结果被 min_w（place 侧）托底");
    assert_eq!(g["b"].x, 135.0, "光标推进用的是托过的尺寸");
    assert_eq!(g["b"].w, 135.0);
    // min 托底可以让子节点总和超出容器（溢出的可见性由裁剪决定 —— 指南登记的边界）
    assert!(g["b"].x + g["b"].w > 180.0, "托底后总和 270 > 容器 180 ⇒ 溢出是明确语义");
}

// ─────────────────────────────────────────────────────────────────────────────
// ⑤ 冲突：min > max ⇒ min 赢（先夹 max 再托 min）
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn t7_min_wins_when_min_exceeds_max() {
    // place 侧：显式 w=100，max_w=30 / min_w=60 ⇒ 60
    let mut bar = Node::new(Kind::Row, "bar").with_layout(L::new().pad(10.0).w(200.0).h(120.0).to_props());
    let mut a = btn("a", 100.0, 20.0);
    a.layout.max_w = Some(Size::Px(30.0));
    a.layout.min_w = Some(Size::Px(60.0));
    bar.children.push(a);
    let mut app = Node::new(Kind::Column, "app");
    app.children.push(bar);
    let g = geo(&app, 400.0, 300.0);
    assert_eq!(g["a"].w, 60.0, "min > max ⇒ min 赢（place 侧）");

    // 高度：h=10，min_h=80 / max_h=40 ⇒ 80
    let mut bar = Node::new(Kind::Row, "bar").with_layout(L::new().pad(10.0).w(200.0).h(120.0).to_props());
    let mut b = btn("b", 20.0, 10.0);
    b.layout.min_h = Some(Size::Px(80.0));
    b.layout.max_h = Some(Size::Px(40.0));
    bar.children.push(b);
    let mut app = Node::new(Kind::Column, "app");
    app.children.push(bar);
    let g = geo(&app, 400.0, 300.0);
    assert_eq!(g["b"].h, 80.0, "min > max ⇒ min 赢（高度）");

    // measure 侧：文本固有 32，min_w=60 / max_w=20 ⇒ 60
    let mut t = Node::new(Kind::Text, "t").with_label("abcd");
    t.layout.min_w = Some(Size::Px(60.0));
    t.layout.max_w = Some(Size::Px(20.0));
    assert_eq!(measure_tree(&t, STYLE, &ApproxMeasure)["t"].0, 60.0, "min > max ⇒ min 赢（measure 侧）");
}

// ─────────────────────────────────────────────────────────────────────────────
// ⑥ 滚动容器：主轴 bound = 无穷 ⇒ min/max 仍然生效（节点自身的声明，与视口无关）
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn t8_scroll_container_main_axis_still_clamped() {
    // Column(scroll) w=100 h=100（视口），子按钮显式 h=300 —— 本来就不被视口夹取
    let mk = |min_h: Option<Size>, max_h: Option<Size>| -> (f32, i32) {
        let mut col = Node::new(Kind::Column, "col")
            .with_layout(L::new().w(100.0).h(100.0).scroll(true).to_props());
        let mut n = btn("n", 20.0, 300.0);
        n.layout.min_h = min_h;
        n.layout.max_h = max_h;
        col.children.push(n);
        let mut app = Node::new(Kind::Column, "app");
        app.children.push(col);
        let (g, metrics) = layout_with_scroll(&app, Rect::new(0.0, 0.0, 400.0, 300.0), STYLE, &ApproxMeasure, &ScrollOffsets::new());
        (g["n"].h, metrics.max_of("col"))
    };

    // 前置基线：无 min/max ⇒ 300（不被视口夹取），max_scroll = 300-100 = 200
    let (h0, m0) = mk(None, None);
    assert_eq!(h0, 300.0, "前置基线：滚动容器子节点主轴不被视口夹取");
    assert_eq!(m0, 200, "前置基线：max_scroll=200");

    // max_h=200 ⇒ 200（仍高于视口 100 ⇒ 仍可滚动，max_scroll=100）
    let (h1, m1) = mk(None, Some(Size::Px(200.0)));
    assert_eq!(h1, 200.0, "max_h 在滚动容器主轴上照样封顶");
    assert_eq!(m1, 100, "max_scroll 反映夹过的内容高");

    // min_h=400 ⇒ 400（视口 100 / bound 无穷都拦不住它 —— min/max 是节点自己的声明）
    let (h2, m2) = mk(Some(Size::Px(400.0)), None);
    assert_eq!(h2, 400.0, "min_h 在滚动容器主轴上照样托底");
    assert_eq!(m2, 300, "max_scroll=400-100");
}

// ─────────────────────────────────────────────────────────────────────────────
// ⑦ 流外（positioned）子节点同样受 min/max 约束
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn t9_positioned_children_are_clamped_too() {
    let mut bar = Node::new(Kind::Row, "bar").with_layout(L::new().pad(10.0).w(200.0).h(80.0).to_props());
    let mut p1 = btn("p1", 100.0, 20.0);
    p1.layout.max_w = Some(Size::Px(40.0));
    p1.layout.position = Some(deer_core::Pos::Offset { x: 0, y: 0 });
    let mut p2 = bare_btn("p2");
    p2.layout.min_w = Some(Size::Px(60.0));
    p2.layout.position = Some(deer_core::Pos::Offset { x: 50, y: 0 });
    bar.children.push(p1);
    bar.children.push(p2);
    let mut app = Node::new(Kind::Column, "app");
    app.children.push(bar);
    let tree = app;

    let g = geo(&tree, 400.0, 300.0);
    assert_eq!(g["p1"], Rect::new(10.0, 10.0, 40.0, 20.0), "流外显式宽超 max_w 被夹");
    assert_eq!(g["p2"], Rect::new(60.0, 10.0, 60.0, 22.0), "流外固有宽低于 min_w 被托（位置 = 内容盒原点+偏移）");
}

// ─────────────────────────────────────────────────────────────────────────────
// ⑧ 百分比：place 相对父内容盒解析；measure 阶段不生效（无父宽度，惯例同 w/h）
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn t10_pct_resolves_against_parent_content_box_in_place_only() {
    // Row w=200 pad=0 ⇒ inner_w=200；按钮固有 28 + min_w=50% ⇒ 托到 100
    let mut bar = Node::new(Kind::Row, "bar").with_layout(L::new().w(200.0).h(40.0).to_props());
    let mut b = bare_btn("b");
    b.layout.min_w = Some(Size::Pct(50.0));
    bar.children.push(b);
    let mut app = Node::new(Kind::Column, "app");
    app.children.push(bar);
    let tree = app;

    // measure 侧：百分比不解析（前置基线 = 固有 28 不变）
    assert_eq!(
        measure_tree(&tree, STYLE, &ApproxMeasure)["b"].0,
        28.0,
        "百分比 min 在 measure 阶段不生效（没有父宽度可依，惯例同 w/h）"
    );

    let g = geo(&tree, 400.0, 300.0);
    assert_eq!(g["b"].w, 100.0, "百分比 min 相对父内容盒（200 的一半 = 100）");

    // max_w 百分比：显式 300 → I-6 bound 200 → max_w=25%（×200=50）⇒ 50。
    // （min 系约束（bound 与 max）取 min 可交换；「下限在 bound 之后 ⇒ 可能溢出」由 t6 钉住）
    let mut bar = Node::new(Kind::Row, "bar").with_layout(L::new().w(200.0).h(40.0).to_props());
    let mut c = btn("c", 300.0, 20.0);
    c.layout.max_w = Some(Size::Pct(25.0));
    bar.children.push(c);
    let mut app = Node::new(Kind::Column, "app");
    app.children.push(bar);
    let g = geo(&app, 400.0, 300.0);
    assert_eq!(g["c"].w, 50.0, "显式 300 经 bound 与 max_w=25% 夹成 50");
}

// ─────────────────────────────────────────────────────────────────────────────
// ⑨ `.dui` 语法：min-w / max-w / min-h / max-h（kebab，数字或百分比）往返 + 坏值报错
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn t11_dui_min_max_round_trip() {
    let src = "[button name=b label=B min-w=36 max-w=50% min-h=10 max-h=200]\n";
    let (tree, warnings) = deer_core::scene::parse_scene_collect(src, "t.dui").expect("应能解析");
    assert!(warnings.is_empty(), "min/max 是已知属性，不该有警告：{warnings:?}");
    let l = &tree.layout;
    assert_eq!(l.min_w, Some(Size::Px(36.0)));
    assert_eq!(l.max_w, Some(Size::Pct(50.0)));
    assert_eq!(l.min_h, Some(Size::Px(10.0)));
    assert_eq!(l.max_h, Some(Size::Px(200.0)));

    let once = encode_scene(&tree);
    for attr in ["min-w=36", "max-w=50%", "min-h=10", "max-h=200"] {
        assert!(once.contains(attr), "{attr} 必须被编码回去：{once}");
    }
    let back = parse_scene(&once, "rt.dui").expect("往返可解析");
    assert!(tree.structurally_eq(&back), "往返必须结构相等\n{once}");
    assert_eq!(encode_scene(&back), once, "二次编码必须逐字节稳定");
}

#[test]
fn t11_dui_min_max_bad_values_are_hard_errors() {
    for (attr, bad) in [("min-w", "abc"), ("max-w", "50x"), ("min-h", ""), ("max-h", "1.2.3")] {
        let src = format!("[button name=b {attr}={bad}]\n");
        let e = parse_scene(&src, "bad.dui").expect_err(&format!("{attr}={bad} 应当报错"));
        assert!(format!("{e}").contains(attr), "错误信息要点名属性：{e}");
    }
    // 裸属性（开关写法）也不行 —— min/max 需要值
    let e = parse_scene("[button name=b min-w]\n", "bad.dui").expect_err("裸 min-w 应当报错");
    assert!(format!("{e}").contains("min-w"), "{e}");
}
