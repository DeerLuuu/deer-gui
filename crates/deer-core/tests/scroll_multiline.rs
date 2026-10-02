//! **滚动容器 + 多行文本**的布局判据（剩余工作第 1 项，A/B 两半的布局面）。
//!
//! 这一层是纯函数，所以所有数字都可以一眼算出来：
//!
//! ```text
//! app (Column, 200×222)
//! ├── spacer  (Column, 200×100)     ← 把下面的容器压到 y=100
//! ├── outer   (Column, 200×100, scroll=true)
//! │   ├── button_1 … button_8  各 28×22 ⇒ 内容高 176、视口 100 ⇒ max_scroll = 76
//! └── below   (Button, 28×22)        ← y = 200
//! ```
//!
//! 判据一览：
//!
//! | 测试 | 判据 |
//! |---|---|
//! | `max_scroll_is_content_minus_viewport` | `max_scroll = 76`（内容 176 − 视口 100），且**只有**滚动容器有上限 |
//! | `offset_shifts_children_and_never_moves_the_container` | 偏移 40 ⇒ 每个子节点上移 40；容器自身矩形一个像素不动 |
//! | `offset_is_clamped_into_zero_to_max` | 999 ⇒ 76、−50 ⇒ 0（滚到边界不越界） |
//! | `scroll_container_does_not_clamp_its_children_main_axis` | 300 高的子节点在滚动容器里仍是 300（非滚动容器里被夹成 100） |
//! | `layout_without_offsets_equals_layout` | 空偏移时 `layout_with_scroll` 的几何与既有 `layout` **逐字段相同** |
//! | `row_scroll_flag_is_ignored` | `Row` 上设 `scroll` 完全无效（几何相同、上限表为空）——「忽略」是被钉住的行为，不是沉默的 |
//! | `wrap_needs_an_explicit_width_and_reserves_one_line_per_wrapped_line` | `wrap` + `w=60` ⇒ 固有高 = 行数 × 18；`wrap` 关 ⇒ 18 |
//! | `approx_measure_wrap_agrees_with_its_height_model` | `wrap().len()*line_height == height(...)`（与 `FontMeasure` 同一条一致性判据） |

use deer_core::builder::{Builder, L};
use deer_core::layout::{
    ApproxMeasure, Geometry, Measure, ScrollOffsets, TextStyle, layout, layout_with_scroll,
    measure_tree,
};
use deer_core::node::{Kind, Node, Rect, Size};

const STYLE: TextStyle = TextStyle {
    font_size: 13.0,
    line_height: 18.0,
};
const W: f32 = 200.0;
const VIEW_H: f32 = 100.0;
const BTN_H: f32 = 22.0;
const N_BTN: usize = 8;
/// 内容高 = 8 × 22 = 176。
const CONTENT_H: f32 = BTN_H * N_BTN as f32;
/// `max_scroll` = 176 − 100 = 76。
const MAX_SCROLL: i32 = 76;
/// 滚动容器的绝对 y（上面垫了一个 100 高的 spacer）。
const OUTER_Y: f32 = 100.0;
/// 子节点（按钮）的固有宽（`BUTTON_MIN_W` = 28）。
const BTN_W: f32 = 28.0;

fn scroll_corpus() -> Node {
    let mut app = Builder::new(Kind::Column, "app");
    app.container_opts(
        Kind::Column,
        "spacer",
        L::new().w(W).h(VIEW_H).to_props(),
        |_| {},
    );
    app.container_opts(
        Kind::Column,
        "outer",
        L::new().w(W).h(VIEW_H).scroll(true).to_props(),
        |o| {
            for i in 1..=N_BTN {
                o.button(format!("{i}"));
            }
        },
    );
    app.button("below");
    app.build()
}

/// 同构但**不滚动**的对照语料（用来证明上面那些数字确实来自 `scroll`）。
fn no_scroll_corpus() -> Node {
    let mut app = Builder::new(Kind::Column, "app");
    app.container_opts(
        Kind::Column,
        "spacer",
        L::new().w(W).h(VIEW_H).to_props(),
        |_| {},
    );
    app.container_opts(
        Kind::Column,
        "outer",
        L::new().w(W).h(VIEW_H).to_props(),
        |o| {
            for i in 1..=N_BTN {
                o.button(format!("{i}"));
            }
        },
    );
    app.button("below");
    app.build()
}

/// `button_i`（`i` 从 1 起，与 `IdGen` 的按钮序号一致）。
fn bid(i: usize) -> String {
    format!("button_{i}")
}

fn box_() -> Rect {
    Rect::new(0.0, 0.0, W, 260.0)
}

fn geo_with(offsets: &ScrollOffsets) -> (Geometry, deer_core::layout::ScrollMetrics) {
    layout_with_scroll(&scroll_corpus(), box_(), STYLE, &ApproxMeasure, offsets)
}

/// 前置：语料本身必须**真的**是「内容高于视口」的（否则 `max_scroll` 那几条判据在测空气）。
fn assert_scroll_precondition() {
    let t = scroll_corpus();
    let intr = measure_tree(&t, STYLE, &ApproxMeasure);
    // 内容高 = 子节点主轴固有尺寸之和（`outer` 自己的固有高是它声明的视口高 ⇒ 不能拿它当内容）。
    let content: f32 = t.children[1]
        .children
        .iter()
        .map(|c| intr[&c.id].1)
        .sum::<f32>()
        + t.children[1].layout.gap * (t.children[1].children.len() - 1) as f32;
    println!(
        "前置：内容高 {content}｜视口 {VIEW_H}｜子节点固有尺寸 button_1={:?}",
        intr["button_1"]
    );
    assert_eq!(content, CONTENT_H, "前置：内容高必须等于子节点之和");
    assert!(
        content > VIEW_H,
        "前置：内容高 {content} 必须严格大于视口 {VIEW_H}，否则没有可滚动范围"
    );
    assert_eq!(
        intr["button_1"],
        (BTN_W, BTN_H),
        "前置：按钮固有尺寸变了 ⇒ 下面所有数字都要重算"
    );
}

#[test]
fn max_scroll_is_content_minus_viewport() {
    assert_scroll_precondition();
    let (geo, metrics) = geo_with(&ScrollOffsets::new());
    println!(
        "outer = {:?}｜below(button_9) = {:?}",
        geo["outer"], geo["button_9"]
    );
    println!("max_scroll = {}｜metrics 里的容器 = {:?}", metrics.max_of("outer"), metrics.ids().collect::<Vec<_>>());
    assert_eq!(
        metrics.max_of("outer"),
        MAX_SCROLL,
        "max_scroll 必须 = 内容高 {} − 视口 {} = {MAX_SCROLL}",
        CONTENT_H,
        VIEW_H
    );
    // 上限表只登记**可滚动容器**：别的节点一律 0（fail-closed，不是「未知」）。
    assert_eq!(metrics.len(), 1, "只有 outer 是可滚动容器");
    for id in ["app", "spacer", "button_1", "button_9"] {
        assert_eq!(metrics.max_of(id), 0, "{id} 不是滚动容器 ⇒ 上限必须是 0");
    }
    assert!(!metrics.is_empty());
    // 偏移为 0 时子节点照固有尺寸顺排（内容超出视口 ⇒ 后面的子节点落在视口外）。
    for i in 1..=N_BTN {
        let r = geo[&bid(i)];
        assert_eq!(
            r,
            Rect::new(0.0, OUTER_Y + BTN_H * (i - 1) as f32, BTN_W, BTN_H),
            "偏移 0 时 {} 必须按固有尺寸顺排",
            bid(i)
        );
    }
    // `below`（= `button_9`，按钮计数器在 8 个滚动内容之后）在容器**下面**，不受滚动影响。
    // 它的宽 = 标签宽 + 左右内边距（`button_opts` 的固有尺寸规则），不是 `BUTTON_MIN_W`。
    let below_w = Measure::width(&ApproxMeasure, "below", STYLE) + 20.0;
    assert_eq!(geo["button_9"], Rect::new(0.0, 200.0, below_w, BTN_H));
}

#[test]
fn offset_shifts_children_and_never_moves_the_container() {
    assert_scroll_precondition();
    let (g0, _) = geo_with(&ScrollOffsets::new());
    let (g40, _) = geo_with(&ScrollOffsets::new().with("outer", 40));

    assert_eq!(
        g40["outer"], g0["outer"],
        "滚动**只**移动内容：容器自身的矩形一个像素都不许动"
    );
    assert_eq!(g0["outer"], Rect::new(0.0, OUTER_Y, W, VIEW_H));
    assert_eq!(g40["app"], g0["app"], "容器之外的节点也不动");
    assert_eq!(g40["button_9"], g0["button_9"]);

    for i in 1..=N_BTN {
        let (a, b) = (g0[&bid(i)], g40[&bid(i)]);
        println!("{}: 偏移 0 ⇒ {a:?}｜偏移 40 ⇒ {b:?}", bid(i));
        assert_eq!(b.y, a.y - 40.0, "{} 必须恰好上移 40", bid(i));
        assert_eq!((b.x, b.w, b.h), (a.x, a.w, a.h), "只有 y 位移");
    }
    // 位移之后的数字（便于复核）：button_1 从 100 移到 60，button_3 从 144 移到 104。
    assert_eq!(g40["button_1"].y, 60.0);
    assert_eq!(g40["button_3"].y, 104.0);
}

#[test]
fn offset_is_clamped_into_zero_to_max() {
    assert_scroll_precondition();
    let (g0, _) = geo_with(&ScrollOffsets::new());
    let (gbig, _) = geo_with(&ScrollOffsets::new().with("outer", 999));
    let (gneg, _) = geo_with(&ScrollOffsets::new().with("outer", -50));

    println!(
        "button_1.y：偏移 0 ⇒ {}｜偏移 999 ⇒ {}｜偏移 −50 ⇒ {}",
        g0["button_1"].y, gbig["button_1"].y, gneg["button_1"].y
    );
    assert_eq!(
        gbig["button_1"].y,
        OUTER_Y - MAX_SCROLL as f32,
        "滚到底：偏移被夹到 max_scroll = {MAX_SCROLL}（不是 999）"
    );
    assert_eq!(gneg["button_1"].y, OUTER_Y, "滚到顶：偏移被夹到 0（不是 −50）");
    // 夹取之后**最后一个**子节点的下边界不该越过「内容底 − 视口」之外：
    assert_eq!(
        gbig["button_8"].y,
        OUTER_Y + BTN_H * (N_BTN - 1) as f32 - MAX_SCROLL as f32
    );
}

#[test]
fn scroll_container_does_not_clamp_its_children_main_axis() {
    // 一个 300 高的子节点：非滚动容器会把它夹到视口高（100），滚动容器不许夹。
    let tall = |scroll: bool| {
        let mut app = Builder::new(Kind::Column, "app");
        app.container_opts(
            Kind::Column,
            "outer",
            L::new().w(W).h(VIEW_H).scroll(scroll).to_props(),
            |o| {
                o.container_opts(Kind::Column, "tall", L::new().w(W).h(300.0).to_props(), |_| {});
            },
        );
        layout_with_scroll(
            &app.build(),
            box_(),
            STYLE,
            &ApproxMeasure,
            &ScrollOffsets::new(),
        )
    };
    let (g_scroll, m_scroll) = tall(true);
    let (g_plain, m_plain) = tall(false);
    println!(
        "300 高的子节点：滚动容器 ⇒ {:?}（max_scroll {}）｜非滚动容器 ⇒ {:?}",
        g_scroll["tall"], m_scroll.max_of("outer"), g_plain["tall"]
    );
    assert_eq!(g_scroll["tall"].h, 300.0, "滚动容器不许把子节点主轴尺寸夹到视口");
    assert_eq!(
        m_scroll.max_of("outer"),
        200,
        "内容 300 − 视口 100 = 200"
    );
    assert_eq!(g_plain["tall"].h, VIEW_H, "非滚动容器仍然是既有语义：夹到视口");
    assert_eq!(m_plain.max_of("outer"), 0, "非滚动容器没有上限");
}

#[test]
fn layout_without_offsets_equals_layout() {
    assert_scroll_precondition();
    // 滚动语料：空偏移下两份几何必须**逐字段相同**，且上限表非空（滚动是加法，不是新语义）。
    for tree in [scroll_corpus(), no_scroll_corpus()] {
        let plain = layout(&tree, box_(), STYLE, &ApproxMeasure);
        let (with_fn, metrics) =
            layout_with_scroll(&tree, box_(), STYLE, &ApproxMeasure, &ScrollOffsets::new());
        assert_eq!(
            plain, with_fn,
            "空偏移时 `layout_with_scroll` 必须与 `layout` 逐字段相同（滚动是加法，不是新语义）"
        );
        println!("上限表：{:?}", metrics.iter().collect::<Vec<_>>());
        if tree.children.iter().any(|c| c.is_scroll_container()) {
            assert!(
                !metrics.is_empty(),
                "有滚动容器的语料必须有上限条目（fail-closed 的空表会让滚轮永远无效）"
            );
        }
    }
}

#[test]
fn row_scroll_flag_is_ignored() {
    // `scroll` 只对 `Column` 有意义（本期只做垂直滚动）。`Row` 上设它必须**完全无效**。
    let row = |scroll: bool| {
        let mut app = Builder::new(Kind::Column, "app");
        app.container_opts(
            Kind::Row,
            "bar",
            L::new()
                .w(W)
                .h(VIEW_H)
                .gap(4.0)
                .scroll(scroll)
                .to_props(),
            |r| {
                for i in 1..=6 {
                    r.button_opts(format!("{i}"), |n| {
                        n.layout.width = Some(Size::Px(60.0));
                    });
                }
            },
        );
        let (geo, metrics) = layout_with_scroll(
            &app.build(),
            box_(),
            STYLE,
            &ApproxMeasure,
            &ScrollOffsets::new().with("bar", 30),
        );
        (geo, metrics)
    };
    let (g_on, m_on) = row(true);
    let (g_off, m_off) = row(false);
    println!("Row + scroll：几何相同 = {}｜上限 {}", g_on == g_off, m_on.max_of("bar"));
    assert_eq!(g_on, g_off, "`Row` 上的 `scroll` 必须被忽略（几何逐字段相同）");
    assert_eq!(m_on.max_of("bar"), 0, "`Row` 不产生可滚动上限");
    assert!(m_on.is_empty(), "上限表里不该有任何容器");
    assert!(m_off.is_empty());
}

// ---------------------------------------------------------------------------
// A 半：多行文本（布局面）
// ---------------------------------------------------------------------------

/// `wrap` 语料：一个声明了宽度 60 的文本节点，标签 `"alpha beta gamma delta"`。
fn wrap_corpus(wrap: bool) -> Node {
    let mut app = Builder::new(Kind::Column, "app");
    app.text_opts(LABEL, |n| {
        n.layout.width = Some(Size::Px(60.0));
        n.layout.wrap = wrap;
    });
    app.build()
}

const LABEL: &str = "alpha beta gamma delta";
/// `ApproxMeasure`（每字符 0.6em）在本语料下的换行点（由下面的测试打印核对）。
const EXPECTED_LINES: [&str; 4] = ["alpha", "beta", "gamma", "delta"];

#[test]
fn wrap_needs_an_explicit_width_and_reserves_one_line_per_wrapped_line() {
    let lines = ApproxMeasure.wrap(LABEL, STYLE, 60.0);
    println!("换行点（w=60）= {lines:?}");
    assert_eq!(lines, EXPECTED_LINES, "换行点必须是这四个词");

    let with = wrap_corpus(true);
    let without = wrap_corpus(false);
    let iw = measure_tree(&with, STYLE, &ApproxMeasure);
    let io = measure_tree(&without, STYLE, &ApproxMeasure);
    println!(
        "固有尺寸：wrap=true ⇒ {:?}｜wrap=false ⇒ {:?}",
        iw["text_1"], io["text_1"]
    );
    assert_eq!(
        iw["text_1"].1,
        lines.len() as f32 * STYLE.line_height,
        "wrap 节点的固有高 = 行数 × line_height"
    );
    assert_eq!(iw["text_1"].1, 72.0);
    assert_eq!(
        io["text_1"].1,
        STYLE.line_height,
        "wrap 关掉 ⇒ 仍是 1 行（**既有语义不变**）"
    );

    let g_with = layout(&with, box_(), STYLE, &ApproxMeasure);
    let g_without = layout(&without, box_(), STYLE, &ApproxMeasure);
    println!("几何：wrap=true ⇒ {:?}｜wrap=false ⇒ {:?}", g_with["text_1"], g_without["text_1"]);
    assert_eq!(g_with["text_1"], Rect::new(0.0, 0.0, 60.0, 72.0));
    assert_eq!(
        g_without["text_1"],
        Rect::new(0.0, 0.0, 60.0, 18.0),
        "开关是**opt-in**：既有语料（不设 wrap）的几何必须一个像素都不变"
    );
    // 宽度由节点的显式宽度决定（换行宽度就是它）。
    assert_eq!(g_with["text_1"].w, 60.0);
    // 没有显式宽度 ⇒ 只有 1 行（无处可换；这是被钉住的边界，不是沉默行为）。
    let mut no_w = Builder::new(Kind::Column, "app");
    no_w.text_opts(LABEL, |n| n.layout.wrap = true);
    let g_no_w = layout(&no_w.build(), box_(), STYLE, &ApproxMeasure);
    println!("wrap 但没给宽度 ⇒ {:?}", g_no_w["text_1"]);
    assert_eq!(
        g_no_w["text_1"].h,
        STYLE.line_height,
        "没声明宽度 ⇒ 换不了行（节点宽 = 文本宽），高度仍是 1 行"
    );
}

#[test]
fn approx_measure_wrap_agrees_with_its_height_model() {
    // 与 `FontMeasure` 同一条一致性判据（见 `deer-gpu/tests/text_measure.rs`）：
    // 「度量高度」与「画出来的行数」不能是两套算法，否则节点高度装不下画出来的行。
    let cases = [
        (LABEL, 60.0f32),
        (LABEL, 200.0),
        ("短", 100.0),
        ("", 100.0),
        ("单个超宽词", 1.0),
        ("word", 0.0),
    ];
    for (text, max_w) in cases {
        let lines = ApproxMeasure.wrap(text, STYLE, max_w);
        let h = ApproxMeasure.height(text, STYLE, max_w);
        println!("{text:?} @ {max_w} ⇒ {lines:?}｜height={h}");
        assert_eq!(
            h,
            lines.len() as f32 * STYLE.line_height,
            "{text:?} @ {max_w}：`height` 与 `wrap().len()` 不一致"
        );
        assert!(!lines.is_empty(), "换行结果永远是 ≥1 行（空串算 1 行）");
        assert_eq!(
            lines.concat().replace([' ', '\t'], ""),
            text.replace([' ', '\t'], ""),
            "换行不许丢字（除了被丢掉的空白）"
        );
    }
    // 边界：`max_width <= 0` ⇒ 不换行（与 `FontMeasure` 同一约定）。
    assert_eq!(ApproxMeasure.wrap(LABEL, STYLE, 0.0), vec![LABEL.to_string()]);
    assert_eq!(ApproxMeasure.wrap(LABEL, STYLE, -5.0), vec![LABEL.to_string()]);
    // 无限宽 ⇒ 1 行（`measure_into` 用的就是这一档）。
    assert_eq!(
        ApproxMeasure.wrap(LABEL, STYLE, f32::INFINITY),
        vec![LABEL.to_string()]
    );
    // 超宽词：按字符硬切，且**每行都能前进**（不死循环）。
    let hard = ApproxMeasure.wrap("wwwwwwwwww", STYLE, 20.0);
    println!("超宽词 ⇒ {hard:?}");
    assert!(hard.len() > 1, "超宽词必须被切开");
    assert_eq!(hard.concat(), "wwwwwwwwww");
}

// ---------------------------------------------------------------------------
// 两条构筑路径（命令式 / `.dui` 场景文件）必须都能表达新属性
// ---------------------------------------------------------------------------

/// `scroll` / `wrap` 是**裸属性**：解析成 `true`，编码回去也在，往返结构相等。
#[test]
fn scene_files_can_express_scroll_and_wrap() {
    let src = "\
[column name=app]
  [column name=outer w=200 h=100 scroll]
    [button label=A]
    [button label=B]
  [text w=60 wrap label=\"alpha beta gamma delta\"]
";
    let tree = deer_core::scene::parse_scene(src, "scroll.dui").expect("场景应能解析");
    println!("解析结果：outer.scroll={}｜t.wrap={}", tree.children[0].layout.scroll, tree.children[1].layout.wrap);
    assert!(tree.children[0].layout.scroll, "`scroll` 裸属性 ⇒ true");
    assert!(tree.children[1].layout.wrap, "`wrap` 裸属性 ⇒ true");
    assert!(
        tree.children[0].is_scroll_container(),
        "`column` + `scroll` ⇒ 可滚动容器（与命令式同一条判据）"
    );
    assert!(tree.children[1].wraps_text(), "`text` + `wrap` ⇒ 换行文本");

    // 往返：编码必须带上这两个开关，否则「解析回来的树」会丢掉它们。
    let text = deer_core::scene::encode_scene(&tree);
    println!("编码：\n{text}");
    assert!(text.contains("scroll"), "编码必须写出 `scroll`");
    assert!(text.contains("wrap"), "编码必须写出 `wrap`");
    let back = deer_core::scene::parse_scene(&text, "roundtrip.dui").expect("往返应可解析");
    assert!(
        tree.structurally_eq(&back),
        "往返必须结构相等（两条构筑路径同一份真相）"
    );
    assert_eq!(text, deer_core::scene::encode_scene(&back), "二次编码逐字稳定");

    // 场景与命令式两条路径产出**同一棵树**（B-1 不变式）。
    let mut b = Builder::new(Kind::Column, "app");
    b.container_opts(
        Kind::Column,
        "outer",
        L::new().w(W).h(VIEW_H).scroll(true).to_props(),
        |o| {
            o.button("A");
            o.button("B");
        },
    );
    b.text_opts(LABEL, |n| {
        n.layout.width = Some(Size::Px(60.0));
        n.layout.wrap = true;
    });
    let imperative = b.build();
    if !imperative.structurally_eq(&tree) {
        println!("命令式：\n{imperative:#?}\n场景：\n{tree:#?}");
        panic!("两条构筑路径必须产出结构相等的树（B-1）");
    }
    // 布局面：两份树给出**逐字段相同**的几何。
    assert_eq!(
        layout(&imperative, box_(), STYLE, &ApproxMeasure),
        layout(&tree, box_(), STYLE, &ApproxMeasure),
        "两条路径的几何必须相同（构造语法不该改变布局）"
    );
}

/// 开关属性**带值**要大声报错（`scroll=1` 这种写法看起来生效、实际不生效 —— 最难查的一类 bug）。
#[test]
fn flag_attributes_with_a_value_are_rejected() {
    for src in ["[column name=app scroll=1]\n", "[text name=t wrap=true label=x]\n", "[button name=b disabled=false]\n"] {
        let err = deer_core::scene::parse_scene(src, "flag.dui").unwrap_err();
        println!("{src:?} ⇒ {}", err.message);
        assert!(
            err.message.contains("开关属性"),
            "带值的开关必须报「开关属性」错误，实际：{}",
            err.message
        );
    }
    // 前置：不带值时它照样能解析（否则上面那条在测一个「永远报错」的解析器）。
    assert!(
        deer_core::scene::parse_scene("[column name=app scroll]\n", "flag.dui").is_ok(),
        "裸属性必须能解析"
    );
}
