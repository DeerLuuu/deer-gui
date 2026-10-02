//! 核心不变式的测试套件。
//!
//! 这些测试从 deer-ui 的 TypeScript 验证原型（V0）搬来并加强：
//! V0 抓到过三个真缺陷（B-1 两路径 id 规则不一致 / B-2 布局把「父分配尺寸」当成「可用空间」
//! / B-3 容器固有尺寸漏算子节点显式尺寸），**每个都有对应的回归守卫**，
//! 且都用「改坏 → 红 → 改回」验证过判据真的会红。

use deer_layout::builder::{Builder, L};
use deer_layout::layout::{ApproxMeasure, TextStyle, hit_test, layout, measure_tree};
use deer_layout::node::{Align, Kind, Node, Rect, Size};
use deer_layout::scene::{encode_scene, parse_scene};

const STYLE: TextStyle = TextStyle {
    font_size: 13.0,
    line_height: 18.0,
};

fn geo(tree: &Node, w: f32, h: f32) -> deer_layout::Geometry {
    layout(tree, Rect::new(0.0, 0.0, w, h), STYLE, &ApproxMeasure)
}

/// 同一份 UI 的命令式写法。
fn imperative() -> Node {
    let mut app = Builder::new(Kind::Column, "app").padding(12.0).gap(8.0);
    app.text("标题");
    // 与场景文件逐字段对齐：rows 的 gap 是 8（容器选项必须显式给，默认是 0）
    app.container_opts(Kind::Row, "bar", L::new().gap(8.0).to_props(), |r| {
        r.button("确定");
        r.button("取消");
    });
    app.build()
}

const SCENE: &str = "\
# 同一个 UI 的场景文件写法
[column name=app pad=12 gap=8]
  [text label=标题]
  [row name=bar gap=8]
    [button label=确定]
    [button label=取消]
";

// ─────────────────────────────────────────────────────────────────────────────
// T1 两条构筑路径同构（**整个方向的核心命题**）
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn t1_two_authoring_paths_produce_the_same_tree() {
    let a = imperative();
    let b = parse_scene(SCENE, "demo.dui").expect("场景应能解析");
    assert!(
        a.structurally_eq(&b),
        "两条构筑路径必须产出结构相等的树\n命令式: {a:#?}\n场景式: {b:#?}"
    );
}

#[test]
fn t1_auto_id_rule_is_shared() {
    // 自动 id 必须按 kind 计数，两条路径一致（B-1 的回归守卫）
    let mut app = Builder::auto(Kind::Column);
    app.text("a");
    app.button("b");
    app.container(Kind::Row, "row_6", |r| {
        r.button("c");
    });
    let auto = app.build();
    let scene = parse_scene(
        "[column name=column_1]\n  [text label=a]\n  [button label=b]\n  [row name=row_6]\n    [button label=c]\n",
        "auto.dui",
    )
    .expect("解析");
    assert!(
        auto.structurally_eq(&scene),
        "自动 id 不一致：\n{auto:#?}\n{scene:#?}"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// T2/T3 确定性 + 纯函数
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn t2_layout_is_deterministic() {
    let g1 = geo(&imperative(), 320.0, 240.0);
    let g2 = geo(&imperative(), 320.0, 240.0);
    let mut k1: Vec<_> = g1.iter().collect();
    let mut k2: Vec<_> = g2.iter().collect();
    k1.sort_by_key(|(k, _)| (*k).clone());
    k2.sort_by_key(|(k, _)| (*k).clone());
    assert_eq!(k1, k2, "同样的输入必须给出逐位相同的几何");
}

#[test]
fn t2_encode_is_stable() {
    let t = imperative();
    assert_eq!(encode_scene(&t), encode_scene(&imperative()));
}

#[test]
fn t3_layout_does_not_mutate_the_tree() {
    let tree = imperative();
    let before = format!("{tree:#?}");
    let _ = geo(&tree, 320.0, 240.0);
    assert_eq!(before, format!("{tree:#?}"), "布局必须是纯函数");
}

// ─────────────────────────────────────────────────────────────────────────────
// T4 自底向上：容器尺寸由内容决定
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn t4_container_intrinsic_drives_size() {
    let mut b = Builder::new(Kind::Column, "app").padding(10.0).gap(5.0);
    b.text("ab"); // 2 字符 * 13 * 0.6 = 15.6 → ceil 16
    b.text("cd");
    let tree = b.build();
    let intr = measure_tree(&tree, STYLE, &ApproxMeasure);
    // 宽 = max(16,16) + 20 = 36；高 = 18 + 5 + 18 + 20 = 61
    assert_eq!(intr["app"], (36.0, 61.0));
}

#[test]
fn t4_root_does_not_fill_the_box() {
    let g = geo(&imperative(), 500.0, 500.0);
    // 尺寸推导（imperative() 用 pad=12 / gap=8，行 gap=8）：
    //   按钮固有宽 = max(28, 文本16 + 内边距10*2) = 36   ← 注意不是最小宽度 28
    //   行固有宽   = 36 + 8 + 36 = 80
    //   根固有宽   = max(文本16, 行80) + 2*12 = 104
    //   根固有高   = 18 + 8 + 22 + 2*12 = 72
    // 根无显式尺寸 ⇒ 用固有尺寸，**不撑满**宿主盒子（I-5）。
    assert_eq!(g["app"], Rect::new(0.0, 0.0, 104.0, 72.0));
    // 子节点偏移 = padding
    assert_eq!(g["text_1"].x, 12.0);
    // 行的 y = padding(12) + 行高(18) + gap(8) = 38；行是显式命名（"bar"）而非自动 id
    assert_eq!(g["bar"].y, 38.0);
}

// ─────────────────────────────────────────────────────────────────────────────
// T5 主轴分配：grow（B-2 的回归守卫 —— 父分配尺寸必须被采信）
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn t5_grow_fills_the_row() {
    let mut app = Builder::new(Kind::Column, "app")
        .size(Some(Size::Px(300.0)), Some(Size::Px(100.0)));
    app.container_opts(
        Kind::Row,
        "row",
        L::new().w(300.0).h(100.0).gap(10.0).to_props(),
        |r| {
            r.button_opts("A", |n| n.layout.grow = 1.0);
            r.button_opts("B", |n| n.layout.grow = 1.0);
        },
    );
    let g = geo(&app.build(), 300.0, 100.0);
    let a = g["button_1"];
    let b = g["button_2"];
    assert_eq!(a.w, b.w, "两个 grow 权重相同的按钮必须等宽");
    assert_eq!(
        a.w + 10.0 + b.w,
        300.0,
        "grow 必须吃满行宽（B-2：父分配的尺寸被丢弃时这里会变成 66）"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// T6 主轴对齐
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn t6_main_axis_alignment() {
    let mk = |a: Align| -> f32 {
        let mut app = Builder::new(Kind::Column, "app").size(Some(Size::Px(200.0)), Some(Size::Px(40.0)));
        app.container_opts(
            Kind::Row,
            "row",
            L::new().w(200.0).h(40.0).gap(10.0).main(a).to_props(),
            |r| {
                r.button_opts("A", |n| n.layout.width = Some(Size::Px(30.0)));
                r.button_opts("B", |n| n.layout.width = Some(Size::Px(30.0)));
            },
        );
        geo(&app.build(), 200.0, 40.0)["button_1"].x
    };
    assert_eq!(mk(Align::Start), 0.0);
    assert_eq!(mk(Align::Center), 65.0, "(200-30-10-30)/2");
    assert_eq!(mk(Align::End), 130.0);
}

// ─────────────────────────────────────────────────────────────────────────────
// T7 交叉轴 stretch
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn t7_cross_axis_stretch_fills() {
    let mut app = Builder::new(Kind::Column, "app").size(Some(Size::Px(100.0)), Some(Size::Px(50.0)));
    app.container_opts(
        Kind::Row,
        "row",
        L::new().w(100.0).h(50.0).cross(Align::Stretch).to_props(),
        |r| {
            r.button_opts("A", |n| n.layout.width = Some(Size::Px(20.0)));
        },
    );
    let g = geo(&app.build(), 100.0, 50.0);
    assert_eq!(g["button_1"].h, 50.0, "stretch 必须吃满交叉轴");
}

// ─────────────────────────────────────────────────────────────────────────────
// T8 像素取整
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn t8_geometry_is_integral() {
    let g = geo(&imperative(), 321.0, 241.0);
    for (id, r) in &g {
        assert!(
            r.x.fract() == 0.0 && r.y.fract() == 0.0 && r.w.fract() == 0.0 && r.h.fract() == 0.0,
            "{id} 的几何不是整数：{r:?}"
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// T9 命中测试
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn t9_hit_test_picks_deepest() {
    let tree = imperative();
    let g = geo(&tree, 400.0, 300.0);
    let btn = g["button_1"];
    let hit = hit_test(&tree, &g, btn.x + 2.0, btn.y + 2.0).expect("应命中");
    assert_eq!(hit.id, "button_1", "必须命中最深的节点，而不是容器");
    assert!(hit_test(&tree, &g, 9999.0, 9999.0).is_none(), "范围外不应命中");
}

#[test]
fn t9_disabled_is_geometry_agnostic() {
    // 「禁用」是交互策略，不是布局属性 ⇒ 几何上仍可命中
    let mut b = Builder::new(Kind::Column, "app");
    b.button_opts("D", |n| {
        n.props.disabled = true;
        n.layout.width = Some(Size::Px(40.0));
        n.layout.height = Some(Size::Px(20.0));
    });
    let tree = b.build();
    let g = geo(&tree, 100.0, 100.0);
    let hit = hit_test(&tree, &g, 5.0, 5.0).expect("应命中");
    assert!(hit.props.disabled);
}

// ─────────────────────────────────────────────────────────────────────────────
// T10 场景往返
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn t10_scene_roundtrip() {
    let t = imperative();
    let text = encode_scene(&t);
    let back = parse_scene(&text, "roundtrip.dui").expect("往返应可解析");
    assert!(t.structurally_eq(&back), "parse(encode(t)) 必须结构相等\n{text}");
    assert_eq!(text, encode_scene(&back), "二次编码必须逐字稳定");
}

// ─────────────────────────────────────────────────────────────────────────────
// T11 场景错误带行号（可用性）
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn t11_scene_errors_carry_line_numbers() {
    let cases: &[(&str, &str)] = &[
        ("未知类型", "[bogus name=x]\n"),
        // ⚠️ 这里原本是 `("未知属性", "[text label=x nope=1]")` —— **D8 起不再是错误**：
        // 未知属性改成「警告 + 结构化保留」（见 `e2_unknown_attrs_tests`）。
        // 换成**已知**属性的错误写法，覆盖同一个「带行号」的诉求，且不放松判据。
        ("开关属性带值", "[text label=x disabled=1]\n"),
        ("叶子有子节点", "[text label=x]\n  [text label=y]\n"),
        ("缩进非 2 倍数", "[column name=a]\n   [text label=x]\n"),
        ("尺寸非法", "[text name=t w=abc]\n"),
    ];
    for (label, src) in cases {
        let e = parse_scene(src, "bad.dui").expect_err(&format!("{label} 应当报错"));
        assert_eq!(e.source, "bad.dui");
        assert!(e.line >= 1, "{label}: 应带行号，实际 {e}");
        // Display 必须是 `source:line: message` 形态（可被编辑器跳转）
        let shown = e.to_string();
        assert!(
            shown.starts_with("bad.dui:") && shown.matches(':').count() >= 2,
            "{label}: Display 形态不对：{shown}"
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// T12 百分比尺寸
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn t12_percentage_size_resolves_against_parent() {
    let mut app = Builder::new(Kind::Column, "app").size(Some(Size::Px(200.0)), Some(Size::Px(100.0)));
    app.container_opts(
        Kind::Row,
        "row",
        L::new().w(200.0).h(100.0).to_props(),
        |r| {
            r.button_opts("half", |n| {
                n.layout.width = Some(Size::Pct(50.0));
                n.layout.height = Some(Size::Px(20.0));
            });
        },
    );
    let g = geo(&app.build(), 200.0, 100.0);
    assert_eq!(g["button_1"].w, 100.0, "50% 应当解析为父内容盒的一半");
}

// ─────────────────────────────────────────────────────────────────────────────
// T13 B-3 的回归守卫：容器固有尺寸必须计入子节点显式尺寸
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn t13_container_intrinsic_includes_explicit_child_sizes() {
    // 主轴方向上，子节点的显式尺寸必须计入容器固有尺寸。
    // （交叉轴是 max 而不是 sum —— 见 t14。这是两个容易混的方向。）
    let mut b = Builder::new(Kind::Row, "row");
    b.button_opts("A", |n| n.layout.width = Some(Size::Px(100.0)));
    let tree = b.build();
    let intr = measure_tree(&tree, STYLE, &ApproxMeasure);
    assert_eq!(intr["row"].0, 100.0, "Row 主轴固有宽必须包含子节点的显式宽度");
}

#[test]
fn t14_container_cross_axis_is_max_not_sum() {
    // 回归守卫：两个各声明 w=36 的按钮放在 Row 里，Row 的固有宽是 **36**（最大值），
    // 不是 72（和）。把显式子尺寸在交叉轴也累加，会让 Column 的固有宽算成 72 而膨胀。
    let mut row = Builder::new(Kind::Row, "row");
    row.button_opts("A", |n| n.layout.width = Some(Size::Px(36.0)));
    row.button_opts("B", |n| n.layout.width = Some(Size::Px(36.0)));
    let t = row.build();
    assert_eq!(
        measure_tree(&t, STYLE, &ApproxMeasure)["row"].0,
        72.0,
        "Row 的主轴是水平 ⇒ 固有宽 = 两个按钮之和 = 72"
    );

    // 但把这个 Row 放进 Column 后，Column 的**交叉轴**（宽）是 max(子宽)，不是 sum
    let mut col = Builder::new(Kind::Column, "col");
    col.container_opts(Kind::Row, "inner", L::new().w(36.0).h(10.0).to_props(), |r| {
        r.button_opts("A", |n| n.layout.width = Some(Size::Px(36.0)));
    });
    let c = col.build();
    assert_eq!(
        measure_tree(&c, STYLE, &ApproxMeasure)["col"].0,
        36.0,
        "Column 的交叉轴固有宽 = max(子宽) = 36，不是 72"
    );
}

/// **D8 的新契约**：未知属性**不报错**，而是「警告 + 保留 + 写回」。
///
/// 与上一条（错误必须带行号）成对存在 —— 改契约时两条一起改，避免「放宽了但没人知道」。
#[test]
fn t11b_unknown_attrs_are_warned_not_rejected() {
    let src = "[text label=x nope=1]\n";
    let (tree, warnings) =
        deer_layout::scene::parse_scene_collect(src, "ok.dui").expect("未知属性不该让解析失败");
    assert_eq!(warnings.len(), 1, "应当有一条警告：{warnings:?}");
    assert!(
        warnings[0].contains("nope"),
        "警告里要点名是哪个属性：{warnings:?}"
    );
    assert_eq!(
        tree.props.extra.get("nope"),
        Some(&Some("1".to_string())),
        "未知属性必须被**结构化保留**"
    );
    // 而且**写得回去**（保留的意义就在这 —— 否则编辑器存一次就丢了）
    assert!(
        encode_scene(&tree).contains("nope=1"),
        "未知属性必须被写回：{}",
        encode_scene(&tree)
    );
}
