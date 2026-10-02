//! L1 绝对定位 / 层叠（D6/D10，Q5 裁定）的测试套件。
//!
//! 语义红线（登记在 `ROADMAP.md` D10 + `node.rs::LayoutProps::position`）：
//! 1. **opt-in**：`position: None` ⇒ 既有树逐字节不变（判据 = 既有全套测试保持绿）；
//! 2. 设了 `position` 的子节点**脱离流内** —— 不参与主轴分配、不占流内空间（含间隙）、
//!    **不计入父容器固有尺寸**（measure 阶段同样跳过）；
//! 3. 位置 = **父内容盒原点 + 偏移**（可为负）；
//! 4. **层叠序 = 声明序**：后声明者后画（在上）且命中优先。
//!
//! 每条断言都先有**前置断言**（几何确实重叠 / 命中确实发生），再断言目标语义 ——
//! 否则守卫自己可能在测空气。双向变异（把「跳过流内分配」改坏 ⇒ 本文件 ②③ 必须红）
//! 的记录见交付报告。

use deer_core::builder::L;
use deer_core::layout::{ApproxMeasure, TextStyle, hit_test, layout, measure_tree};
use deer_core::node::{Kind, Node, Pos, Rect, Size};
use deer_core::scene::{encode_scene, parse_scene};

const STYLE: TextStyle = TextStyle {
    font_size: 13.0,
    line_height: 18.0,
};

fn geo(tree: &Node, w: f32, h: f32) -> deer_core::Geometry {
    layout(tree, Rect::new(0.0, 0.0, w, h), STYLE, &ApproxMeasure)
}

fn offset(x: i32, y: i32) -> Option<Pos> {
    Some(Pos::Offset { x, y })
}

/// ② 的语料：Row 里「流内 A + 流外 P」各一个按钮（A 先声明、P 后声明）。
///
/// 数值（ApproxMeasure）：两个按钮都显式 (36, 22)；pad=10 ⇒
/// Row 固有 = (36 + 20, 22 + 20) = (56, 42) —— **P 不计入**（它设了 `pos`）。
fn out_of_flow_tree() -> Node {
    let mut bar = Node::new(Kind::Row, "bar").with_layout(L::new().pad(10.0).to_props());
    let mut a = Node::new(Kind::Button, "a").with_label("A");
    a.layout.width = Some(Size::Px(36.0));
    a.layout.height = Some(Size::Px(22.0));
    let mut p = Node::new(Kind::Button, "p").with_label("P");
    p.layout.width = Some(Size::Px(36.0));
    p.layout.height = Some(Size::Px(22.0));
    p.layout.position = offset(100, 50);
    bar.children.push(a);
    bar.children.push(p);
    let mut app = Node::new(Kind::Column, "app");
    app.children.push(bar);
    app
}

// ─────────────────────────────────────────────────────────────────────────────
// ② 流外：兄弟位置不动、父固有尺寸不含它
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn t2_positioned_child_is_out_of_flow() {
    let tree = out_of_flow_tree();
    let intr = measure_tree(&tree, STYLE, &ApproxMeasure);
    // 前置：确实只有一个流内子节点 + 一个流外子节点
    let bar = tree.children.first().expect("语料要有 bar");
    assert_eq!(bar.children.len(), 2, "前置：两个按钮");
    assert!(!bar.children[0].is_positioned() && bar.children[1].is_positioned());

    // 父容器固有尺寸**不含**流外子节点（不占主轴、也不抬交叉轴 max）：
    // 宽 = 36（只有 A）+ 2×10 = 56；高 = 22 + 2×10 = 42
    assert_eq!(
        intr["bar"],
        (56.0, 42.0),
        "设了 pos 的子节点不许计入父容器固有尺寸"
    );

    let g = geo(&tree, 400.0, 300.0);
    // 流内兄弟的位置与「没有 P 时」完全一致：x = pad = 10
    assert_eq!(g["a"].x, 10.0, "流内兄弟不能被流外节点挤动");
    assert_eq!(g["a"].y, 10.0);
    // 流外节点 = 父内容盒原点 + 偏移 = (10 + 100, 10 + 50)
    assert_eq!(g["p"], Rect::new(110.0, 60.0, 36.0, 22.0), "位置 = 内容盒原点 + 偏移");
}

#[test]
fn t2_positioned_child_takes_no_flow_slot_and_no_gap() {
    // 同一个 Row：A、B 流内，P 流外。若 P 占了流内槽，B 的 x 会变成 10+36+g 或更多。
    let mut bar = Node::new(Kind::Row, "bar").with_layout(L::new().pad(10.0).gap(4.0).to_props());
    for (id, pos) in [("a", None), ("b", None), ("p", offset(200, 0))] {
        let mut n = Node::new(Kind::Button, id).with_label(id);
        n.layout.width = Some(Size::Px(36.0));
        n.layout.height = Some(Size::Px(22.0));
        n.layout.position = pos;
        bar.children.push(n);
    }
    let mut app = Node::new(Kind::Column, "app");
    app.children.push(bar);
    let tree = app;

    let intr = measure_tree(&tree, STYLE, &ApproxMeasure);
    // 只有 A、B 计入：36 + 4 + 36 + 20 = 96（P 不占间隙、不占主轴）
    assert_eq!(intr["bar"].0, 96.0, "流外子节点不占主轴、不产生间隙");

    let g = geo(&tree, 400.0, 100.0);
    assert_eq!(g["a"].x, 10.0);
    assert_eq!(g["b"].x, 50.0, "A(gap)B 的流内排布不许被 P 打断");
    assert_eq!(g["p"].x, 10.0 + 200.0, "P 按偏移落位，不吃流内槽");
}

// ─────────────────────────────────────────────────────────────────────────────
// ④ 偏移定位的数值（含负偏移）
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn t4_offset_positions_are_exact_including_negative() {
    let mut bar = Node::new(Kind::Row, "bar").with_layout(L::new().pad(10.0).to_props());
    for (id, pos) in [("pos", offset(30, 40)), ("neg", offset(-20, -15))] {
        let mut n = Node::new(Kind::Button, id).with_label(id);
        n.layout.width = Some(Size::Px(20.0));
        n.layout.height = Some(Size::Px(10.0));
        n.layout.position = pos;
        bar.children.push(n);
    }
    let mut app = Node::new(Kind::Column, "app");
    app.children.push(bar);
    let tree = app;

    let g = geo(&tree, 400.0, 300.0);
    // 内容盒原点 = (pad, pad) = (10, 10)
    assert_eq!(g["pos"], Rect::new(40.0, 50.0, 20.0, 10.0), "正偏移");
    assert_eq!(g["neg"], Rect::new(-10.0, -5.0, 20.0, 10.0), "负偏移不夹取：允许伸出父盒子");
}

#[test]
fn t4_container_with_only_positioned_children_has_padding_only_intrinsic() {
    let mut bar = Node::new(Kind::Row, "bar").with_layout(L::new().pad(10.0).gap(4.0).to_props());
    let mut n = Node::new(Kind::Button, "p").with_label("p");
    n.layout.position = offset(0, 0);
    bar.children.push(n);
    let mut app = Node::new(Kind::Column, "app");
    app.children.push(bar);
    let tree = app;

    // 全部子节点都在流外 ⇒ 容器固有尺寸只剩 padding（没有「空容器」的特判路径）
    let intr = measure_tree(&tree, STYLE, &ApproxMeasure);
    assert_eq!(intr["bar"], (20.0, 20.0), "全流外 ⇒ 固有尺寸 = padding×2");

    let g = geo(&tree, 400.0, 300.0);
    // 显式尺寸没有 ⇒ 用按钮固有尺寸（1 字符 label：宽 max(28, 8+20)=28，高 22）
    assert_eq!(g["p"], Rect::new(10.0, 10.0, 28.0, 22.0), "零偏移 ⇒ 恰好落在内容盒原点");
}

// ─────────────────────────────────────────────────────────────────────────────
// ③ 层叠序 = 声明序：命中优先（绘制列表顺序断言在 deer-gui 侧测试，见交付说明）
// ─────────────────────────────────────────────────────────────────────────────

/// 流内 A（先声明）、流外 P（后声明）重叠在 (20, 20)。
fn overlap_tree() -> Node {
    let mut bar = Node::new(Kind::Row, "bar").with_layout(L::new().pad(10.0).to_props());
    let mut a = Node::new(Kind::Button, "a").with_label("A");
    a.layout.width = Some(Size::Px(36.0));
    a.layout.height = Some(Size::Px(22.0));
    let mut p = Node::new(Kind::Button, "p").with_label("P");
    p.layout.width = Some(Size::Px(36.0));
    p.layout.height = Some(Size::Px(22.0));
    p.layout.position = offset(0, 0); // 恰好压在 A 上
    bar.children.push(a);
    bar.children.push(p);
    let mut app = Node::new(Kind::Column, "app");
    app.children.push(bar);
    app
}

#[test]
fn t3_later_declared_positioned_sibling_wins_the_hit() {
    let tree = overlap_tree();
    let g = geo(&tree, 400.0, 300.0);
    let probe = (20.0, 20.0); // A 与 P 的矩形内部（两者都含这个点）
    // 前置断言：这个点真的同时落在两个矩形里（否则下面的优先级断言在测空气）
    let in_a = {
        let r = g["a"];
        probe.0 >= r.x && probe.1 >= r.y && probe.0 < r.x + r.w && probe.1 < r.y + r.h
    };
    let in_p = {
        let r = g["p"];
        probe.0 >= r.x && probe.1 >= r.y && probe.0 < r.x + r.w && probe.1 < r.y + r.h
    };
    assert!(in_a && in_p, "前置：探测点必须同时命中 A 与 P（a={:?} p={:?}）", g["a"], g["p"]);

    // 声明序层叠：后声明的 P 在上 ⇒ 命中 P
    let hit = hit_test(&tree, &g, probe.0, probe.1).expect("应命中");
    assert_eq!(hit.id, "p", "后声明的流外兄弟必须优先命中（层叠序 = 声明序）");
}

#[test]
fn t3_declaration_order_decides_hit_not_flow_position() {
    // 反向语料：流外 P **先**声明、流内 B **后**声明，两者重叠。
    // 「流内/流外」不参与层叠判定 —— 后声明的 B 必须胜出（与绘制顺序一致）。
    let mut bar = Node::new(Kind::Row, "bar").with_layout(L::new().pad(10.0).to_props());
    let mut p = Node::new(Kind::Button, "p").with_label("P");
    p.layout.width = Some(Size::Px(36.0));
    p.layout.height = Some(Size::Px(22.0));
    p.layout.position = offset(0, 0);
    let mut b = Node::new(Kind::Button, "b").with_label("B");
    b.layout.width = Some(Size::Px(36.0));
    b.layout.height = Some(Size::Px(22.0));
    bar.children.push(p);
    bar.children.push(b);
    let mut app = Node::new(Kind::Column, "app");
    app.children.push(bar);
    let tree = app;

    let g = geo(&tree, 400.0, 300.0);
    let probe = (20.0, 20.0);
    let hit = hit_test(&tree, &g, probe.0, probe.1).expect("应命中");
    assert_eq!(
        hit.id, "b",
        "后声明的流内兄弟同样要胜出 —— 层叠只看声明序，不看流内/流外"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// ⑤ `.dui` 语法往返（pos=x,y）
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn t5_dui_pos_attr_round_trips() {
    let src = "[column name=app]\n  [button name=p label=P pos=100,-20]\n";
    let (tree, warnings) = parse_scene_collect_public(src);
    assert!(warnings.is_empty(), "pos 是已知属性，不该有警告：{warnings:?}");
    assert_eq!(
        tree.children[0].layout.position,
        offset(100, -20),
        "pos 解析成 Pos::Offset"
    );

    // 往返：parse(encode(t)) 结构相等，且二次编码逐字节稳定
    let once = encode_scene(&tree);
    assert!(once.contains("pos=100,-20"), "pos 必须被编码回去：{once}");
    let back = parse_scene(&once, "rt.dui").expect("往返可解析");
    assert!(tree.structurally_eq(&back), "往返必须结构相等\n{once}");
    assert_eq!(encode_scene(&back), once, "二次编码必须逐字节稳定");
}

#[test]
fn t5_dui_without_pos_is_byte_identical() {
    // 既有语料逐字节不变（opt-in 红线在语法面的体现）
    let out = encode_scene(&parse_scene("[column name=app]\n  [button name=ok]\n", "f.dui").unwrap());
    assert_eq!(out, "# deer-gui-scene: 1\n[column name=app]\n  [button name=ok]\n");
    assert!(!out.contains("pos"), "未设 position 时编码不多写任何东西：{out}");
}

#[test]
fn t5_dui_pos_bad_values_are_hard_errors() {
    // 已知属性写错仍然硬报错（Q6 的放宽只针对「未知」属性）
    for bad in ["abc", "10", "1,2,3", "", "\" \""] {
        let src = format!("[button name=p pos={bad}]\n");
        let e = parse_scene(&src, "bad.dui").expect_err(&format!("pos={bad} 应当报错"));
        assert!(
            format!("{e}").contains("pos"),
            "错误信息要点名属性：{e}"
        );
    }
    // 裸属性（开关写法）也不行 —— pos 需要值
    let e = parse_scene("[button name=p pos]\n", "bad.dui").expect_err("裸 pos 应当报错");
    assert!(format!("{e}").contains("pos"), "{e}");
}

/// `parse_scene_collect` 的薄封装（本文件只关心「有没有警告」这一件事）。
fn parse_scene_collect_public(src: &str) -> (Node, Vec<String>) {
    deer_core::scene::parse_scene_collect(src, "t.dui").expect("语料应能解析")
}
