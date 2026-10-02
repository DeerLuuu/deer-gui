//! L2 每子节点交叉轴对齐（`cross_self`，app-foundation 计划 L 线 · 设计登记 D6）的测试套件。
//!
//! 语义红线（登记在 `node.rs::LayoutProps::cross_self` 与 `docs/features/align-self.md`）：
//! 1. **opt-in**：`cross_self: None` ⇒ 完全回落容器级 `cross_axis` ⇒ 既有树逐字节不变
//!    （判据 = 既有全套测试保持绿 + `.dui` 编码不多一字节）；
//! 2. `Some(a)` ⇒ **覆盖**父容器的 `cross_axis`，**只对这一个流内子节点生效**
//!    （对齐位置与 stretch 吃满都按 `a` 算，兄弟不受影响）；
//! 3. **对流外（`position`）子节点不生效**：流外按 L1 语义只看自己的显式/固有尺寸落位。
//!
//! 每条断言都先有**前置断言**（容器确有剩余空间 / 基线行为确实如此），
//! 再断言目标语义 —— 否则守卫自己可能在测空气。双向变异（把「cross_self 覆盖」改坏
//! ⇒ 本文件 t2/t3/t4/t5 必须红）的记录见交付报告。

use deer_core::builder::L;
use deer_core::layout::{ApproxMeasure, TextStyle, layout, measure_tree};
use deer_core::node::{Align, Kind, Node, Rect, Size};
use deer_core::scene::{encode_scene, parse_scene};

const STYLE: TextStyle = TextStyle {
    font_size: 13.0,
    line_height: 18.0,
};

fn geo(tree: &Node, w: f32, h: f32) -> deer_core::Geometry {
    layout(tree, Rect::new(0.0, 0.0, w, h), STYLE, &ApproxMeasure)
}

/// 显式尺寸 (w×h) 的按钮。
fn btn(id: &str, w: f32, h: f32) -> Node {
    let mut n = Node::new(Kind::Button, id).with_label(id);
    n.layout.width = Some(Size::Px(w));
    n.layout.height = Some(Size::Px(h));
    n
}

/// 只声明**主轴**宽度的按钮（交叉轴用固有高 22）——
/// 给 stretch 的参与者用：**显式交叉轴尺寸优先于 stretch**（I-7 的既有优先级，
/// 容器级 `cross=stretch` 与 `cross_self=stretch` 同一套），声明了显式高就不会被拉伸。
fn btn_w(id: &str, w: f32) -> Node {
    let mut n = Node::new(Kind::Button, id).with_label(id);
    n.layout.width = Some(Size::Px(w));
    n
}

/// 只声明**主轴**高度的按钮（交叉轴用固有宽 28，一个字符 label ⇒ `max(28, 8+20)`）。
fn btn_h(id: &str, h: f32) -> Node {
    let mut n = Node::new(Kind::Button, id).with_label(id);
    n.layout.height = Some(Size::Px(h));
    n
}

// ─────────────────────────────────────────────────────────────────────────────
// ① opt-in：不设 cross_self ⇒ 逐字节不变
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn t1_unset_cross_self_changes_nothing() {
    // 基线语料（无 cross_self）：Row(pad=10, 200×80) 里两个 20×10 按钮。
    // 未设 cross_axis ⇒ 默认 Start ⇒ 两个都在 y=10。
    let mut bar = Node::new(Kind::Row, "bar").with_layout(L::new().pad(10.0).w(200.0).h(80.0).to_props());
    bar.children.push(btn("a", 20.0, 10.0));
    bar.children.push(btn("b", 20.0, 10.0));
    let mut app = Node::new(Kind::Column, "app");
    app.children.push(bar);
    let tree = app;

    let g = geo(&tree, 400.0, 300.0);
    assert_eq!(g["a"], Rect::new(10.0, 10.0, 20.0, 10.0), "前置基线：默认 Start");
    assert_eq!(g["b"], Rect::new(30.0, 10.0, 20.0, 10.0));

    // 语法面：未设 ⇒ 编码不多写任何东西（`.dui` 语料逐字节不变）
    let out = encode_scene(&parse_scene("[column name=app]\n  [button name=ok]\n", "f.dui").unwrap());
    assert_eq!(out, "# deer-gui-scene: 1\n[column name=app]\n  [button name=ok]\n");
    assert!(!out.contains("cross-self"), "未设 cross_self 时编码不多写任何东西：{out}");
}

// ─────────────────────────────────────────────────────────────────────────────
// ② Row（交叉轴 = 垂直）：center / end / stretch 各自正确
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn t2_row_cross_self_overrides_per_child() {
    let mut bar = Node::new(Kind::Row, "bar").with_layout(L::new().pad(10.0).w(200.0).h(80.0).to_props());
    let a = btn("a", 20.0, 10.0);
    let mut b = btn("b", 20.0, 10.0);
    b.layout.cross_self = Some(Align::Center);
    let mut c = btn("c", 20.0, 10.0);
    c.layout.cross_self = Some(Align::End);
    // stretch 参与者只声明主轴宽（交叉轴留给固有高 22 —— 显式交叉轴尺寸优先于 stretch）
    let mut d = btn_w("d", 20.0);
    d.layout.cross_self = Some(Align::Stretch);
    for n in [a, b, c, d] {
        bar.children.push(n);
    }
    let mut app = Node::new(Kind::Column, "app");
    app.children.push(bar);
    let tree = app;

    // 前置：容器交叉轴确有剩余空间（inner_h=60，子高 10/22 ⇒ slack > 0）
    let g = geo(&tree, 400.0, 300.0);
    assert_eq!(g["bar"].h, 80.0, "前置：容器高 80");
    assert!(g["bar"].h - 20.0 - 22.0 > 0.0, "前置：inner_h=60 > 子高，对齐才有区分度");

    // A 未设 ⇒ 回落容器级（未设 cross_axis ⇒ Start）
    assert_eq!(g["a"], Rect::new(10.0, 10.0, 20.0, 10.0), "None ⇒ 回落容器级 Start");
    // B center：y = 10 + (60-10)/2 = 35
    assert_eq!(g["b"], Rect::new(30.0, 35.0, 20.0, 10.0), "cross_self=center 只动 B");
    // C end：y = 10 + (60-10) = 60
    assert_eq!(g["c"], Rect::new(50.0, 60.0, 20.0, 10.0), "cross_self=end 只动 C");
    // D stretch：吃满 inner_h=60（固有高 22 被拉伸）
    assert_eq!(g["d"], Rect::new(70.0, 10.0, 20.0, 60.0), "cross_self=stretch 只拉伸 D");
}

// ─────────────────────────────────────────────────────────────────────────────
// ③ Column（交叉轴 = 水平）：同一套语义在另一个方向
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn t3_column_cross_self_overrides_per_child() {
    let mut col = Node::new(Kind::Column, "col").with_layout(L::new().pad(10.0).w(80.0).h(200.0).to_props());
    let a = btn_h("a", 20.0);
    let mut b = btn_h("b", 20.0);
    b.layout.cross_self = Some(Align::Center);
    let mut c = btn_h("c", 20.0);
    c.layout.cross_self = Some(Align::End);
    // stretch 参与者只声明主轴高（交叉轴留给固有宽 28 —— 显式交叉轴尺寸优先于 stretch）
    let mut d = btn_h("d", 20.0);
    d.layout.cross_self = Some(Align::Stretch);
    for n in [a, b, c, d] {
        col.children.push(n);
    }
    let mut app = Node::new(Kind::Column, "app");
    app.children.push(col);
    let tree = app;

    // 前置：inner_w=60，子宽 28 ⇒ slack=32 > 0
    let g = geo(&tree, 400.0, 300.0);
    assert_eq!(g["col"].w - 20.0 - 28.0, 32.0, "前置：slack=32 > 0");

    // A 未设 ⇒ Start（x=10，固有宽 28）；主轴位置照旧（y = 10/30/50/70）
    assert_eq!(g["a"], Rect::new(10.0, 10.0, 28.0, 20.0));
    assert_eq!(g["b"], Rect::new(26.0, 30.0, 28.0, 20.0), "center：x = 10 + 32/2");
    assert_eq!(g["c"], Rect::new(42.0, 50.0, 28.0, 20.0), "end：x = 10 + 32");
    assert_eq!(g["d"], Rect::new(10.0, 70.0, 60.0, 20.0), "stretch：吃满 inner_w=60");
}

// ─────────────────────────────────────────────────────────────────────────────
// ④ None 回落容器级：容器 cross=end 时，未设的落 end、设了 start 的落 start
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn t4_none_falls_back_to_container_cross_axis() {
    let mut bar = Node::new(Kind::Row, "bar")
        .with_layout(L::new().pad(10.0).w(200.0).h(80.0).cross(Align::End).to_props());
    let a = btn("a", 20.0, 10.0); // 未设 ⇒ 跟容器 End
    let mut b = btn("b", 20.0, 10.0);
    b.layout.cross_self = Some(Align::Start); // 覆盖容器级
    bar.children.push(a);
    bar.children.push(b);
    let mut app = Node::new(Kind::Column, "app");
    app.children.push(bar);
    let tree = app;

    let g = geo(&tree, 400.0, 300.0);
    assert_eq!(g["a"].y, 60.0, "未设 ⇒ 回落容器级 end（y = 10 + (60-10)）");
    assert_eq!(g["b"].y, 10.0, "cross_self=start 覆盖容器级 end");
}

// ─────────────────────────────────────────────────────────────────────────────
// ⑤ 覆盖在 stretch 方向也成立：容器 stretch 拉伸全部，cross_self=start 的除外
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn t5_override_beats_container_stretch() {
    let mut bar = Node::new(Kind::Row, "bar")
        .with_layout(L::new().pad(10.0).w(200.0).h(80.0).cross(Align::Stretch).to_props());
    let a = btn_w("a", 20.0); // 未设 ⇒ 被容器 stretch 拉满（固有高 22 → 60）
    let mut b = btn_w("b", 20.0);
    b.layout.cross_self = Some(Align::Start); // 覆盖 ⇒ 保持固有高 22
    bar.children.push(a);
    bar.children.push(b);
    let mut app = Node::new(Kind::Column, "app");
    app.children.push(bar);
    let tree = app;

    let g = geo(&tree, 400.0, 300.0);
    assert_eq!(g["a"].h, 60.0, "未设 ⇒ 容器级 stretch 生效");
    assert_eq!(g["b"], Rect::new(30.0, 10.0, 20.0, 22.0), "cross_self=start 覆盖 stretch（尺寸与位置都不拉伸）");
}

// ─────────────────────────────────────────────────────────────────────────────
// ⑥ 流外（positioned）子节点：cross_self 不生效（L2 的边界，写进指南）
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn t6_cross_self_does_not_affect_positioned_children() {
    let mut bar = Node::new(Kind::Row, "bar").with_layout(L::new().pad(10.0).w(200.0).h(80.0).to_props());
    let mut p = btn("p", 20.0, 10.0);
    p.layout.position = Some(deer_core::Pos::Offset { x: 0, y: 0 });
    p.layout.cross_self = Some(Align::End); // 流外 ⇒ 不生效
    bar.children.push(p);
    let mut app = Node::new(Kind::Column, "app");
    app.children.push(bar);
    let tree = app;

    let g = geo(&tree, 400.0, 300.0);
    // 若 cross_self 对流外生效，p 会落到 y=60（end）；实际按 L1 语义：内容盒原点 + 偏移
    assert_eq!(
        g["p"],
        Rect::new(10.0, 10.0, 20.0, 10.0),
        "流外子节点只看显式/固有尺寸 + 偏移，cross_self 不参与"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// ⑦ `.dui` 语法：cross-self=…（kebab）往返 + 坏值硬报错
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn t7_dui_cross_self_round_trips() {
    let src = "[column name=app cross=end]\n  [button name=b label=B cross-self=center]\n";
    let (tree, warnings) = deer_core::scene::parse_scene_collect(src, "t.dui").expect("应能解析");
    assert!(warnings.is_empty(), "cross-self 是已知属性，不该有警告：{warnings:?}");
    assert_eq!(
        tree.children[0].layout.cross_self,
        Some(Align::Center),
        "cross-self 解析成 Option<Align>"
    );

    let once = encode_scene(&tree);
    assert!(once.contains("cross-self=center"), "必须被编码回去：{once}");
    assert!(once.contains("cross=end"), "容器级 cross 照常编码：{once}");
    let back = parse_scene(&once, "rt.dui").expect("往返可解析");
    assert!(tree.structurally_eq(&back), "往返必须结构相等\n{once}");
    assert_eq!(encode_scene(&back), once, "二次编码必须逐字节稳定");
}

#[test]
fn t7_dui_cross_self_bad_values_are_hard_errors() {
    // 已知属性写错仍然硬报错（Q6 的放宽只针对「未知」属性）
    for bad in ["middle", "CENTER", ""] {
        let src = format!("[button name=b cross-self={bad}]\n");
        let e = parse_scene(&src, "bad.dui").expect_err(&format!("cross-self={bad} 应当报错"));
        assert!(format!("{e}").contains("cross-self"), "错误信息要点名属性：{e}");
    }
    // 裸属性（开关写法）也不行 —— cross-self 需要值
    let e = parse_scene("[button name=b cross-self]\n", "bad.dui").expect_err("裸 cross-self 应当报错");
    assert!(format!("{e}").contains("cross-self"), "{e}");
}

// ─────────────────────────────────────────────────────────────────────────────
// ⑧ measure 阶段不受 cross_self 影响（它只管 place 的对齐，不改固有尺寸）
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn t8_measure_is_unaffected_by_cross_self() {
    let mut col = Node::new(Kind::Column, "col").with_layout(L::new().pad(10.0).to_props());
    let mut b = btn("b", 30.0, 20.0);
    b.layout.cross_self = Some(Align::Stretch);
    col.children.push(b);
    let tree = col;

    let intr = measure_tree(&tree, STYLE, &ApproxMeasure);
    // 固有尺寸只看内容/显式声明，与对齐无关（stretch 吃满发生在 place）
    assert_eq!(intr["col"], (50.0, 40.0), "cross_self 不该影响固有尺寸聚合");
}
