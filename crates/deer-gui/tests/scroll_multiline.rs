//! **滚动容器 + 多行文本**的回归（剩余工作第 1 项）。
//!
//! 分两半，各自独立可验收：
//!
//! - **A 多行文本**：`Kind::Text` 且 `layout.wrap = true` 的节点按宽度换行，
//!   每行一条 `DrawCmd::Text`（行数、换行点、每行矩形都有断言）；
//! - **B 滚动容器**：`Kind::Column` 且 `layout.scroll = true` 的容器，内容高于视口 ⇒
//!   `max_scroll > 0`；偏移参与几何与绘制；滚轮驱动偏移；到边界不越界；
//!   **视口外的点不命中**（裁剪快照把视口裁剪绑到子节点上）。
//!
//! ## 本文件的判据一览
//!
//! | 测试 | 判据 |
//! |---|---|
//! | `baseline_digests_are_frozen` | 非滚动语料的**几何 / 三份绘制列表 / 像素**摘要等于冻结常量（改动的「逐字节不变」护栏） |
//! | `golden_corpus_is_single_line_and_unscrolled` | 前置：冻结语料里**没有** `wrap`/`scroll` 节点、也没有 PushClip（否则上面的护栏在空转） |
//!
//! 前置条件每条都**显式断言**（前置不成立时护栏会静默失效 —— 这是本项目已经踩过的坑）。

use deer_gpu::interact::{InteractState, InteractiveRenderer};
use deer_gpu::null::CpuRenderer;
use deer_gpu::render::{DefaultRenderer, NullRenderer};
use deer_core::{ DrawCmd, DrawList, RectI };
use deer_gpu::{ Extent, Theme };
// LY2：文本栈来自 L1 crate `deer-text`。
use deer_text::TextEngine;
use deer_gui::interaction::{self, ClipSnapshot, InputEvent, UiEvent, UiState};
use deer_core::builder::Builder;
use deer_core::layout::{ApproxMeasure, Geometry, ScrollOffsets, TextStyle, layout};
use deer_core::node::{Kind, Node, Rect};

/// 某个节点几何的中心（键盘/鼠标脚本里用得最多）。
fn center(geo: &Geometry, id: &str) -> (f32, f32) {
    let r = geo
        .get(id)
        .copied()
        .unwrap_or_else(|| panic!("测试前置：{id} 没有几何"));
    (r.x + r.w / 2.0, r.y + r.h / 2.0)
}

// ---------------------------------------------------------------------------
// 语料
// ---------------------------------------------------------------------------

const A_W: u32 = 320;
const A_H: u32 = 200;

/// 语料 A：单行文本 + 一行按钮（含禁用按钮）+ 带内边距的容器。**不含滚动/换行**。
fn corpus_a() -> Node {
    let mut app = Builder::new(Kind::Column, "app").padding(12.0).gap(8.0);
    app.text("标题");
    app.container_opts(
        Kind::Row,
        "bar",
        deer_core::builder::L::new().gap(8.0).to_props(),
        |r| {
            r.button("确定");
            r.button_opts("禁用", |n| n.props.disabled = true);
        },
    );
    app.build()
}

/// 语料 B：与 M5-4 的界面同构（输入框 + 按钮 + 禁用按钮 + 文本），**不含滚动/换行**。
fn corpus_b() -> Node {
    let mut app = Builder::new(Kind::Column, "app").padding(12.0).gap(10.0);
    app.text("M5-4 interactive form");
    app.container_opts(
        Kind::Row,
        "row",
        deer_core::builder::L::new().gap(8.0).to_props(),
        |r| {
            r.button("OK");
            r.field("name");
        },
    );
    app.button_opts("disabled", |n| n.props.disabled = true);
    app.build()
}

fn theme() -> Theme {
    Theme::default()
}

fn style(t: &Theme) -> TextStyle {
    TextStyle {
        font_size: t.font_size,
        line_height: t.line_height,
    }
}

fn geo_a() -> Geometry {
    let t = theme();
    layout(
        &corpus_a(),
        Rect::new(0.0, 0.0, A_W as f32, A_H as f32),
        style(&t),
        &ApproxMeasure,
    )
}

// ---------------------------------------------------------------------------
// 摘要（determinism / 逐字节不变 的载体）
// ---------------------------------------------------------------------------

/// FNV-1a 64 —— 就为了「同一份数据 ⇒ 同一个数」，与哈希表迭代顺序无关（调用方自己排序）。
fn digest(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// 几何表的规范文本：**按 id 排序**（`HashMap` 的迭代顺序不是判据的一部分）。
fn canon_geo(geo: &Geometry) -> String {
    let mut ids: Vec<&String> = geo.keys().collect();
    ids.sort();
    ids.iter()
        .map(|id| format!("{id}={:?}", geo[*id]))
        .collect::<Vec<_>>()
        .join("\n")
}

/// 绘制列表的规范文本（逐命令 `Debug`）。
fn canon_list(list: &DrawList) -> String {
    list.cmds
        .iter()
        .map(|c| format!("{c:?}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn px_digest(px: &[u8]) -> u64 {
    digest(px)
}

fn render(list: &DrawList, w: u32, h: u32) -> Vec<u8> {
    CpuRenderer::new()
        .render(Extent { width: w, height: h }, list, clear_color())
        .expect("CPU 渲染失败")
        .pixels
}

/// 与既有像素语料同一个 clear 色（`interactive_form.rs` 的 `CLEAR`）。
fn clear_color() -> deer_core::Color {
    deer_core::Color::rgb(0x08, 0x09, 0x0c)
}

/// 语料里**一条 `PushClip` 都没有**吗（冻结护栏的前置）。
fn no_clip_cmds(list: &DrawList) -> bool {
    list.counts().push_clip == 0 && list.counts().pop_clip == 0
}

/// 语料里**没有** `wrap` / `scroll`（冻结护栏的前置）。
fn no_wrap_no_scroll(n: &Node) -> bool {
    let here = !n.layout.wrap && !n.layout.scroll;
    here && n.children.iter().all(no_wrap_no_scroll)
}

// ---------------------------------------------------------------------------
// 一、冻结基线：非滚动语料的输出必须逐字节不变
// ---------------------------------------------------------------------------

/// **本文件的核心回归护栏**：非滚动/非换行语料的几何、绘制列表、像素的摘要**冻结**。
///
/// 为什么用摘要常量而不是「再算一遍比较」：`滚动` 的实现如果偷偷改了**非滚动**路径的
/// 命令序列（多一条 `PushClip`、文本被展开成多行、矩形被夹短），只有与**改动之前**的
/// 实测值比对才抓得住 —— 自己跟自己比永远是绿的。
///
/// 常量来源：在实现滚动/多行**之前**，用同一份代码路径（`layout` + 三个渲染器 + CPU 后端）
/// 打印出来的实测摘要（`cargo test -p deer-gui --test scroll_multiline -- --nocapture`）。
#[test]
fn baseline_digests_are_frozen() {
    let t = theme();

    // ---- 语料 A ----
    let a = corpus_a();
    assert!(no_wrap_no_scroll(&a), "前置：语料 A 不含 wrap/scroll 节点");
    let ga = geo_a();
    let lists_a = [
        ("default", DefaultRenderer::new(t.clone(), &ApproxMeasure).build(&a, &ga)),
        ("hints", NullRenderer::build(&a, &ga)),
        (
            "interactive",
            deer_gpu::interact::InteractiveRenderer::new(
                t.clone(),
                &ApproxMeasure,
                &deer_gpu::interact::InteractState::default(),
            )
            .build(&a, &ga),
        ),
    ];
    println!("—— 语料 A（{A_W}×{A_H}，无滚动/无换行）——");
    println!("几何摘要 = {:#018x}", digest(canon_geo(&ga).as_bytes()));
    for (name, l) in &lists_a {
        assert!(no_clip_cmds(l), "前置：语料 A 的 `{name}` 列表里没有裁剪命令");
        println!(
            "  {name:<12} 命令 {} 条（text {}）摘要 = {:#018x}",
            l.len(),
            l.counts().text,
            digest(canon_list(l).as_bytes())
        );
    }
    let px_a = render(&lists_a[0].1, A_W, A_H);
    println!("  pixels 摘要 = {:#018x}", px_digest(&px_a));

    // ---- 语料 B ----
    let b = corpus_b();
    assert!(no_wrap_no_scroll(&b), "前置：语料 B 不含 wrap/scroll 节点");
    let gb = layout(
        &b,
        Rect::new(0.0, 0.0, 420.0, 220.0),
        style(&t),
        &ApproxMeasure,
    );
    let lb = NullRenderer::build(&b, &gb);
    assert!(no_clip_cmds(&lb), "前置：语料 B 的提示列表里没有裁剪命令");
    println!("—— 语料 B（420×220，无滚动/无换行）——");
    println!("几何摘要 = {:#018x}", digest(canon_geo(&gb).as_bytes()));
    println!(
        "  hints 命令 {} 条（node_hint {}）摘要 = {:#018x}",
        lb.len(),
        lb.counts().node_hint,
        digest(canon_list(&lb).as_bytes())
    );

    // ---- 冻结值（改动前实测；见本测试文档）----
    let golden = [
        ("A geometry", digest(canon_geo(&ga).as_bytes()), GOLDEN_A_GEO),
        (
            "A default list",
            digest(canon_list(&lists_a[0].1).as_bytes()),
            GOLDEN_A_DEFAULT,
        ),
        (
            "A hints list",
            digest(canon_list(&lists_a[1].1).as_bytes()),
            GOLDEN_A_HINTS,
        ),
        (
            "A interactive list",
            digest(canon_list(&lists_a[2].1).as_bytes()),
            GOLDEN_A_INTERACTIVE,
        ),
        ("A pixels", px_digest(&px_a), GOLDEN_A_PIXELS),
        ("B geometry", digest(canon_geo(&gb).as_bytes()), GOLDEN_B_GEO),
        ("B hints list", digest(canon_list(&lb).as_bytes()), GOLDEN_B_HINTS),
    ];
    for (name, got, want) in golden {
        assert_eq!(
            got, want,
            "{name}: 摘要 {got:#018x} ≠ 冻结值 {want:#018x} —— \
             非滚动/非换行语料的输出被改动了（这就是「不动滚动时输出必须逐字节不变」那条护栏）"
        );
    }
    println!("非滚动语料的几何/列表/像素摘要全部等于冻结值 ✅");
}

/// 冻结常量的**出处**（改动前实测；每个数字都由上面那个测试在 `--nocapture` 下重新打印）。
///
/// ⚠️ **合并轮（`b8a031b` + `feat/scroll-multiline`@`5a14d1f`）重取过 3 个含 `NodeHint` 的摘要**：
/// 另一条线（known-defects）给 `NodeHint` 加了第三个字段 **`node_id_fp`**（id 指纹），
/// 而本文件的规范文本是逐命令 `Debug` ⇒ 该字段**必然**进摘要 ⇒ 这三条与合并前的冻结值不同。
/// **为什么可以重取（有硬证据，不是「绿了就改」）**：把 `node_id_fp` 从规范文本里**遮蔽掉**后，
/// 7 个摘要**全部逐位等于旧冻结值**（`cargo test -p deer-gui --test scroll_multiline -- --nocapture`
/// 实测：A hints `0x69df639d5317e06f`、A interactive `0x7c2c1d0a39c7ade5`、B hints `0x3f66b3d0b20264d5`）
/// ⇒ 命令**序列/种类/矩形/长度/几何/像素**一个字节都没变，变的只有那个新字段。
/// ⇒ 下面 3 个常量换成合并后实测值；**其余 4 个（几何 / default / pixels）原样未动**（合并后仍然相等）。
mod goldens {
    /// 语料 A 几何表的规范文本摘要。
    pub const GOLDEN_A_GEO: u64 = 0x660c_e366_cf7a_a4de;
    /// 语料 A `DefaultRenderer` 列表摘要。
    pub const GOLDEN_A_DEFAULT: u64 = 0xe559_6079_6561_e3cf;
    /// 语料 A `NullRenderer` 列表摘要（合并轮重取；见 mod 文档：只多了 `node_id_fp`）。
    pub const GOLDEN_A_HINTS: u64 = 0x4217_0511_ceee_2eac;
    /// 语料 A `InteractiveRenderer`（idle）列表摘要（合并轮重取；同上）。
    pub const GOLDEN_A_INTERACTIVE: u64 = 0xdb42_c0e9_098b_c6b2;
    /// 语料 A CPU 像素摘要。
    pub const GOLDEN_A_PIXELS: u64 = 0xf9ae_2a5a_76b1_479d;
    /// 语料 B 几何表摘要。
    pub const GOLDEN_B_GEO: u64 = 0x2d75_c275_d610_8b47;
    /// 语料 B `NullRenderer` 列表摘要（合并轮重取；同上）。
    pub const GOLDEN_B_HINTS: u64 = 0x5583_dfd9_4b40_3d0b;
}

use goldens::*;

/// 每条 `DrawCmd` 的**种类序列**（诊断用：摘要不一致时能一眼看出多/少了哪类命令）。
#[allow(dead_code)]
fn cmd_kinds(list: &DrawList) -> String {
    list.cmds
        .iter()
        .map(|c| match c {
            DrawCmd::FillRect { .. } => "fill",
            DrawCmd::StrokeRect { .. } => "stroke",
            DrawCmd::FillRoundRect { .. } => "round",
            DrawCmd::Text { .. } => "text",
            DrawCmd::PushClip { .. } => "push",
            DrawCmd::PopClip => "pop",
            DrawCmd::NodeHint { .. } => "hint",
        })
        .collect::<Vec<_>>()
        .join(",")
}

// ---------------------------------------------------------------------------
// 二、滚动语料：几何 / 滚轮 / 命中 / 像素
// ---------------------------------------------------------------------------

/*
```text
app (Column, 200×222)
├── spacer  (Column, 200×100)                ← 把滚动容器压到 y=100
├── outer   (Column, 200×100, scroll=true)   ← 视口；内容 = 8 × 22 = 176 ⇒ max_scroll = 76
│   └── button_1 … button_8
└── button_9（"below"）                       ← y = 200，**不在**视口里
```
*/
const SW: u32 = 200;
const SH: u32 = 260;
const OUTER_Y: i32 = 100;
const MAX_SCROLL: i32 = 76;
/// 滚轮步长（`interaction::WHEEL_STEP_PX`；判据里写死这个数字，改了这里会红）。
const STEP: i32 = 40;

fn scroll_corpus() -> Node {
    let mut app = Builder::new(Kind::Column, "app");
    app.container_opts(
        Kind::Column,
        "spacer",
        deer_core::builder::L::new().w(200.0).h(100.0).to_props(),
        |_| {},
    );
    app.container_opts(
        Kind::Column,
        "outer",
        deer_core::builder::L::new()
            .w(200.0)
            .h(100.0)
            .scroll(true)
            .to_props(),
        |o| {
            for i in 1..=8 {
                o.button(format!("{i}"));
            }
        },
    );
    app.button("below");
    app.build()
}

fn scroll_style() -> TextStyle {
    let t = theme();
    style(&t)
}

/// 一帧：几何（带偏移）+ 绘制列表（含 `NodeHint` 与视口裁剪）+ 裁剪快照。
struct ScrollFrame {
    geo: Geometry,
    metrics: deer_core::layout::ScrollMetrics,
    list: DrawList,
    clip: ClipSnapshot,
}

fn scroll_frame(tree: &Node, offsets: &ScrollOffsets) -> ScrollFrame {
    let t = theme();
    let (geo, metrics) = deer_core::layout::layout_with_scroll(
        tree,
        Rect::new(0.0, 0.0, SW as f32, SH as f32),
        scroll_style(),
        &ApproxMeasure,
        offsets,
    );
    let state = InteractState::default();
    let list = InteractiveRenderer::new(t, &ApproxMeasure, &state).build(tree, &geo);
    let clip = ClipSnapshot::from_draw_list(&list, tree, &geo);
    ScrollFrame {
        geo,
        metrics,
        list,
        clip,
    }
}

fn bid(i: usize) -> String {
    format!("button_{i}")
}

/// 与语料的前置条件一起用：几何、上限、快照都必须真的成立（否则下面的判据在测空气）。
fn precondition(tree: &Node, f: &ScrollFrame, offsets: &ScrollOffsets) {
    println!(
        "偏移 {:?}｜max_scroll {}｜列表 {} 条（push_clip {}）｜快照 {} 个节点",
        offsets.iter().collect::<Vec<_>>(),
        f.metrics.max_of("outer"),
        f.list.len(),
        f.list.counts().push_clip,
        f.clip.len()
    );
    assert_eq!(f.metrics.max_of("outer"), MAX_SCROLL, "前置：max_scroll 变了");
    assert!(f.list.clip_balanced(), "前置：裁剪栈平衡");
    assert_eq!(
        (f.list.counts().push_clip, f.list.counts().pop_clip),
        (1, 1),
        "前置：视口裁剪恰好一对"
    );
    assert_eq!(
        f.clip.len(),
        count_nodes(tree),
        "前置：快照必须覆盖每个有几何的节点（否则裁剪判据静默失效）"
    );
    for i in 1..=8 {
        assert!(
            f.clip.is_known(&bid(i)),
            "护栏前置：{} 必须在快照里",
            bid(i)
        );
        assert_eq!(
            f.clip.clip_of(&bid(i)),
            Some(RectI::new(0, OUTER_Y, SW as i32, 100)),
            "{} 的有效裁剪必须 = 视口",
            bid(i)
        );
    }
    assert!(
        f.clip.is_known("outer") && f.clip.clip_of("outer").is_none(),
        "护栏前置：容器自身**不**被自己的视口裁掉"
    );
}

fn count_nodes(n: &Node) -> usize {
    1 + n.children.iter().map(count_nodes).sum::<usize>()
}

/// **滚轮驱动偏移**：悬停在容器内 ⇒ 该容器滚动；偏移参与几何（子节点整体位移）。
#[test]
fn wheel_scrolls_the_hovered_container_and_moves_the_geometry() {
    let tree = scroll_corpus();
    let base = scroll_frame(&tree, &ScrollOffsets::new());
    precondition(&tree, &base, &ScrollOffsets::new());
    assert_eq!(base.geo["button_1"].y, OUTER_Y as f32);
    assert_eq!(base.geo["button_9"].y, 200.0);

    let mut s = UiState::default();
    s.scroll.set_metrics(&base.metrics);

    // 先把指针移到容器内的按钮上（hover 决定滚轮滚谁 —— `Wheel` 事件本身没有坐标）。
    let hover_pt = (10.0f32, OUTER_Y as f32 + 50.0);
    let evs = interaction::handle(&mut s, &tree, &base.geo, base.clip.clone(), &InputEvent::PointerMoved { x: hover_pt.0, y: hover_pt.1 });
    println!("移到 {hover_pt:?} ⇒ {evs:?}｜hover={:?}", s.hover);
    assert_eq!(evs, vec![UiEvent::HoverChanged(Some(bid(3)))], "前置：悬停在 button_3 上");
    assert!(s.scroll.offsets.is_empty(), "前置：还没滚动");

    // 滚轮向下拨（`dy < 0`）= 内容上移 ⇒ 偏移增大一个步长。
    let evs = interaction::handle(&mut s, &tree, &base.geo, base.clip.clone(), &InputEvent::Wheel { dx: 0.0, dy: -1.0 });
    println!("wheel dy=-1 ⇒ {evs:?}｜offset={}", s.scroll.offset_of("outer"));
    assert_eq!(
        evs,
        vec![UiEvent::Scrolled {
            id: "outer".into(),
            offset: STEP
        }],
        "滚一格 = 一个步长（{STEP} px）"
    );

    // 偏移参与几何：重新布局（调用方每帧都把 `offsets` 喂给布局）。
    let scrolled = scroll_frame(&tree, &s.scroll.offsets);
    println!(
        "滚动后：button_1.y {}（原 {}）｜button_3.y {}｜容器 {:?}",
        scrolled.geo["button_1"].y, base.geo["button_1"].y, scrolled.geo["button_3"].y, scrolled.geo["outer"]
    );
    assert_eq!(scrolled.geo["button_1"].y, (OUTER_Y - STEP) as f32);
    assert_eq!(scrolled.geo["button_3"].y, (OUTER_Y + 44 - STEP) as f32);
    assert_eq!(scrolled.geo["outer"], base.geo["outer"], "容器自身不动");
    assert_eq!(scrolled.geo["button_9"].y, 200.0, "视口外的兄弟节点不动");
    // 绘制列表也跟着变（多余的命令、不同的矩形）——用摘要证明「真的重画了」。
    assert_ne!(
        digest(canon_list(&scrolled.list).as_bytes()),
        digest(canon_list(&base.list).as_bytes()),
        "滚动之后绘制列表必须不同（否则偏移没进绘制）"
    );
    // 快照里的视口裁剪**不随偏移改变**（裁剪的是视口，不是内容）。
    assert_eq!(
        scrolled.clip.clip_of(&bid(1)),
        base.clip.clip_of(&bid(1)),
        "视口裁剪与偏移无关"
    );
}

/// **滚到边界不越界**：到顶/到底都不产生事件（状态没变），偏移永远夹在 `[0, max_scroll]`。
#[test]
fn wheel_clamps_at_both_ends_without_emitting() {
    let tree = scroll_corpus();
    let frame = scroll_frame(&tree, &ScrollOffsets::new());
    precondition(&tree, &frame, &ScrollOffsets::new());
    let geov = frame.geo.clone();
    let clip = frame.clip.clone();
    let mut s = UiState::default();
    s.scroll.set_metrics(&frame.metrics);
    interaction::handle(
        &mut s,
        &tree,
        &geov,
        clip.clone(),
        &InputEvent::PointerMoved {
            x: 10.0,
            y: OUTER_Y as f32 + 50.0,
        },
    );
    assert!(s.hover.is_some(), "前置：悬停到了容器内的控件");

    let wheel = |s: &mut UiState, dy: f32| {
        interaction::handle(s, &tree, &geov, clip.clone(), &InputEvent::Wheel { dx: 0.0, dy })
    };

    // 往下滚一次到 40，再一次被夹到 76，第三次**不再产生事件**。
    assert_eq!(wheel(&mut s, -1.0).len(), 1);
    assert_eq!(s.scroll.offset_of("outer"), STEP);
    assert_eq!(
        wheel(&mut s, -1.0),
        vec![UiEvent::Scrolled {
            id: "outer".into(),
            offset: MAX_SCROLL
        }],
        "第二次被夹到 max_scroll"
    );
    let at_bottom = s.scroll.offset_of("outer");
    let again = wheel(&mut s, -1.0);
    println!("已到底（{at_bottom}）再滚 ⇒ {again:?}");
    assert!(again.is_empty(), "到底之后不该再产生事件");
    assert_eq!(s.scroll.offset_of("outer"), MAX_SCROLL);
    assert_eq!(s.scroll.offset_of("outer"), at_bottom);

    // 往上滚：一次回到 36（76−40），再一次被夹到 0；再往上滚无事件。
    assert_eq!(
        wheel(&mut s, 1.0),
        vec![UiEvent::Scrolled {
            id: "outer".into(),
            offset: MAX_SCROLL - STEP
        }]
    );
    assert_eq!(
        wheel(&mut s, 1.0),
        vec![UiEvent::Scrolled {
            id: "outer".into(),
            offset: 0
        }],
        "被夹到 0（不是 −4）"
    );
    assert!(wheel(&mut s, 1.0).is_empty(), "到顶之后不该再产生事件");
    assert!(wheel(&mut s, 5.0).is_empty(), "大增量同样被夹住且无事件");
    assert_eq!(s.scroll.offset_of("outer"), 0);

    // dy = 0 不是「变化」⇒ 无事件（也不该写状态）。
    assert!(wheel(&mut s, 0.0).is_empty(), "dy=0 不该产生事件");
}

/// **没有可滚动祖先 ⇒ 滚轮什么都不做**（既有 `r18_unconsumed_events_change_nothing` 的前提，
/// 也是「不动滚动时输出不变」在状态机侧的对应物）。
#[test]
fn wheel_without_a_scrollable_ancestor_changes_nothing() {
    let t = theme();
    let tree = corpus_a();
    let geo = geo_a();
    let list = DefaultRenderer::new(t, &ApproxMeasure).build(&tree, &geo);
    let clip = ClipSnapshot::from_draw_list(&list, &tree, &geo);
    let mut s = UiState::default();
    s.scroll.set_metrics(&deer_core::layout::ScrollMetrics::new());
    let (x, y) = center(&geo, "button_1");
    interaction::handle(&mut s, &tree, &geo, clip.clone(), &InputEvent::PointerMoved { x, y });
    assert_eq!(s.hover.as_deref(), Some("button_1"), "前置：悬停在按钮上");
    let before = s.clone();
    let evs = interaction::handle(&mut s, &tree, &geo, clip, &InputEvent::Wheel { dx: 0.0, dy: -3.0 });
    println!("无可滚动祖先：evs={evs:?}");
    assert!(evs.is_empty(), "不该产生事件");
    assert_eq!(s, before, "不该改任何状态");
}

/// **禁用的滚动容器不响应滚轮**（与「禁用子树不响应输入」同一条规则）。
#[test]
fn wheel_on_a_disabled_scroll_container_changes_nothing() {
    let mut app = Builder::new(Kind::Column, "app");
    app.container_opts(
        Kind::Column,
        "outer",
        deer_core::builder::L::new()
            .w(200.0)
            .h(100.0)
            .scroll(true)
            .to_props(),
        |o| {
            for i in 1..=8 {
                o.button(format!("{i}"));
            }
            // 最后一个按钮之后把容器禁用（`disabled` 在 props 上）。
            o.button_opts("x", |n| n.props.disabled = true);
        },
    );
    let tree = {
        let mut t = app.build();
        t.children[0].props.disabled = true;
        t
    };
    let frame = scroll_frame(&tree, &ScrollOffsets::new());
    let mut s = UiState::default();
    s.scroll.set_metrics(&frame.metrics);
    assert!(frame.metrics.max_of("outer") > 0, "前置：容器确实可滚动");
    let (x, y) = center(&frame.geo, "button_3");
    interaction::handle(&mut s, &tree, &frame.geo, frame.clip.clone(), &InputEvent::PointerMoved { x, y });
    println!("禁用容器上的 hover = {:?}", s.hover);
    let evs = interaction::handle(&mut s, &tree, &frame.geo, frame.clip.clone(), &InputEvent::Wheel { dx: 0.0, dy: -1.0 });
    println!("禁用容器上的滚轮 ⇒ {evs:?}｜offset={}", s.scroll.offset_of("outer"));
    assert!(evs.is_empty(), "禁用容器不该响应滚轮");
    assert_eq!(s.scroll.offset_of("outer"), 0);
}

// ---------------------------------------------------------------------------
// 三、命中：视口外的点不命中（滚动语义延伸到既有裁剪语义）
// ---------------------------------------------------------------------------

/// **视口外的点不命中，视口内的点命中正确的 id（含滚动偏移）**。
///
/// 命中侧的机制有两道闸，本测试把**两道都钉住**：
///
/// 1. **几何**：子节点被整体位移 ⇒ 滚到视口外的子节点矩形已经**不在容器矩形内**，
///    而 `hit_test` 只从「矩形包含该点」的祖先往下探 ⇒ 它**根本不可达**；
/// 2. **裁剪**：`ClipSnapshot` 把视口裁剪绑到了子节点上（`clip_of(child) == 视口`）⇒
///    就算几何那一道不存在，`hit` 也会在「裁剪外的点」上拒绝它。
///
/// 第 2 道在**渲染**侧是必需的（没有它，被滚上去的内容会画到视口外 —— 见
/// `scrolled_pixels_are_confined_to_the_viewport`）；在命中侧它与第 1 道**冗余但一致**，
/// 这里把两者都断言出来，免得将来 `hit_test` 换了剪枝策略而没人发现。
#[test]
fn hits_follow_the_scroll_offset_and_respect_the_viewport() {
    let tree = scroll_corpus();
    let offsets = ScrollOffsets::new().with("outer", MAX_SCROLL);
    let at_bottom = scroll_frame(&tree, &offsets);
    precondition(&tree, &at_bottom, &offsets);

    // ---- 闸 ①：几何 ----
    // 偏移 76 之后 button_1/button_2/button_3 的几何跑到视口**上方**（y = 24..90）。
    let outside = (10.0f32, 50.0f32);
    let b2 = at_bottom.geo[&bid(2)];
    println!(
        "偏移 {MAX_SCROLL}：button_2 = {b2:?}｜outer = {:?}",
        at_bottom.geo["outer"]
    );
    assert_eq!(b2, Rect::new(0.0, 46.0, 28.0, 22.0), "前置：button_2 被滚到 y=46");
    assert!(
        b2.y + b2.h <= at_bottom.geo["outer"].y,
        "前置：button_2 已经完全在视口**上方**（几何上不在容器矩形内）"
    );
    let raw = deer_core::hit_test(&tree, &at_bottom.geo, outside.0, outside.1).map(|n| n.id.clone());
    println!("视口外的点 {outside:?}：hit_test = {raw:?}");
    assert_ne!(
        raw.as_deref(),
        Some(bid(2).as_str()),
        "几何上不可能探到容器矩形之外的子节点（hit_test 的剪枝）"
    );
    let h = interaction::hit(&tree, &at_bottom.geo, at_bottom.clip.clone(), outside.0, outside.1);
    println!("  ⇒ hit（含裁剪）= {:?}", h.map(|n| n.id.as_str()));
    assert_ne!(
        h.map(|n| n.id.as_str()),
        Some(bid(2).as_str()),
        "视口外的点**不许**命中被滚走的 button_2"
    );
    assert_eq!(raw, h.map(|n| n.id.clone()), "两道闸的结论必须一致");

    // ---- 闸 ②：裁剪 ----
    assert!(
        at_bottom.clip.is_known(&bid(2)),
        "护栏前置：button_2 必须登记在快照里（否则这条断言在测空气）"
    );
    assert_eq!(
        at_bottom.clip.clip_of(&bid(2)),
        Some(RectI::new(0, OUTER_Y, SW as i32, 100)),
        "button_2 的有效裁剪 = 视口"
    );
    assert!(
        !at_bottom.clip.allows(&bid(2), outside.0, outside.1),
        "就算几何那一道不存在，裁剪也必须拒绝视口外的点（`allows` 的直接判据）"
    );
    assert!(
        at_bottom.clip.allows(&bid(2), 10.0, (OUTER_Y + 50) as f32),
        "反向自检：`allows` 对同一个节点的**视口内**点是放行的（否则它拒绝一切）"
    );

    // ---- 视口内的点：命中必须跟着偏移走 ----
    let inside = (10.0f32, (OUTER_Y + 50) as f32); // y = 150
    let base = scroll_frame(&tree, &ScrollOffsets::new());
    let hb = interaction::hit(&tree, &base.geo, base.clip.clone(), inside.0, inside.1);
    let hs = interaction::hit(&tree, &at_bottom.geo, at_bottom.clip.clone(), inside.0, inside.1);
    println!(
        "视口内 {inside:?}：偏移 0 ⇒ {:?}｜偏移 {MAX_SCROLL} ⇒ {:?}",
        hb.map(|n| n.id.as_str()),
        hs.map(|n| n.id.as_str())
    );
    assert_eq!(
        hb.map(|n| n.id.as_str()),
        Some(bid(3).as_str()),
        "偏移 0：y=150 落在 button_3（144..166）"
    );
    assert_eq!(
        hs.map(|n| n.id.as_str()),
        Some(bid(6).as_str()),
        "偏移 76：内容上移 ⇒ 同一坐标落到 button_6（134..156）"
    );
    // 前置：这两个 id 确实不同（否则「命中跟着偏移走」是空话）。
    assert_ne!(hb.map(|n| n.id.clone()), hs.map(|n| n.id.clone()));

    // 状态机面：被滚走的子节点不会被误报成 hover。
    let mut s = UiState::default();
    s.scroll.set_metrics(&at_bottom.metrics);
    let evs = interaction::handle(
        &mut s,
        &tree,
        &at_bottom.geo,
        at_bottom.clip.clone(),
        &InputEvent::PointerMoved {
            x: outside.0,
            y: outside.1,
        },
    );
    println!("视口外的点（偏移 {MAX_SCROLL}）：事件 = {evs:?}｜hover = {:?}", s.hover);
    assert_ne!(
        s.hover.as_deref(),
        Some(bid(2).as_str()),
        "hover 不许落在被滚走的子节点上"
    );
    assert_eq!(
        s.hover.as_deref(),
        Some("spacer"),
        "该点确实压在 spacer 上（这条断言把「几何剪枝」这件事说清楚）"
    );
}

// ---------------------------------------------------------------------------
// 四、像素：裁剪真的挡在视口上；多行真的画在各自的 y 带里
// ---------------------------------------------------------------------------

/// **滚动的像素差异必须全部落在视口内**（否则内容溢出到视口外的其它区域）。
///
/// 这条判据的判别力来自「不裁剪时会怎样」：偏移 76 之后 button_1..button_3 的几何
/// 落在视口**上方**（y = 24..90）。若渲染器不发视口裁剪，它们会把强调色画到 spacer 上
/// ⇒ 视口外的差异 > 0。
#[test]
fn scrolled_pixels_are_confined_to_the_viewport() {
    let tree = scroll_corpus();
    let base = scroll_frame(&tree, &ScrollOffsets::new());
    let scrolled = scroll_frame(&tree, &ScrollOffsets::new().with("outer", MAX_SCROLL));
    precondition(&tree, &base, &ScrollOffsets::new());
    // 前置：这两个偏移下绘制列表确实不同（否则像素比较是在比较同一帧）。
    assert_ne!(canon_list(&base.list), canon_list(&scrolled.list));

    let px0 = render(&base.list, SW, SH);
    let px1 = render(&scrolled.list, SW, SH);
    let viewport = RectI::new(0, OUTER_Y, SW as i32, 100);
    let mut inside = 0usize;
    let mut outside = 0usize;
    for i in (0..px0.len()).step_by(4) {
        if px0[i..i + 4] == px1[i..i + 4] {
            continue;
        }
        let x = ((i / 4) % SW as usize) as i32;
        let y = ((i / 4) / SW as usize) as i32;
        if viewport.contains(x, y) {
            inside += 1;
        } else {
            outside += 1;
        }
    }
    println!(
        "偏移 0 vs 偏移 {MAX_SCROLL}：视口内差异像素 {inside}｜视口外差异像素 {outside}（必须为 0）"
    );
    assert!(inside > 0, "视口内必须有差异（否则滚动没画出来）");
    assert_eq!(outside, 0, "滚动的像素差异溢出到了视口之外 ⇒ 视口裁剪没生效");

    // 视口外**不允许**出现强调色（那是被滚上去的按钮的颜色）。
    let accent = theme().accent;
    let mut stray_accent = 0usize;
    for y in 0..SH as i32 {
        for x in 0..SW as i32 {
            if viewport.contains(x, y) {
                continue;
            }
            let i = ((y as usize) * SW as usize + x as usize) * 4;
            // 屏幕上视口之外的强调色只能来自 `button_9`（它在 y ≥ 200 的视口下方，
            // 本就是独立节点，不属于被裁的内容）。
            if y >= 200 {
                continue;
            }
            if px1[i] == accent.r && px1[i + 1] == accent.g && px1[i + 2] == accent.b {
                stray_accent += 1;
            }
        }
    }
    println!("视口上方（y<100）的强调色像素 = {stray_accent}（必须为 0）");
    assert_eq!(stray_accent, 0, "被滚上去的按钮在视口上方漏了出来");
}

const WRAP_LABEL: &str = "alpha beta gamma delta";

fn wrap_corpus_gui(wrap: bool) -> Node {
    let mut b = Builder::new(Kind::Column, "app");
    b.text_opts(WRAP_LABEL, |n| {
        n.layout.width = Some(deer_core::node::Size::Px(60.0));
        n.layout.wrap = wrap;
    });
    b.build()
}

/// **多行真的画在各自的 y 带里**：4 行的墨迹落在 4 条互不重叠的横带；关掉换行只剩第 1 带。
#[test]
fn wrapped_text_puts_ink_in_each_line_band() {
    let t = theme();
    let line_h = t.line_height as i32;
    let bands: Vec<(i32, i32)> = (0..4).map(|i| (i * line_h, (i + 1) * line_h)).collect();

    let ink_in = |px: &[u8], (y0, y1): (i32, i32)| -> usize {
        let mut n = 0usize;
        for y in y0..y1 {
            for x in 0..60 {
                let i = ((y as usize) * SW as usize + x as usize) * 4;
                if px[i..i + 4] != [0x08, 0x09, 0x0c, 0xff] {
                    n += 1;
                }
            }
        }
        n
    };

    let tree_w = wrap_corpus_gui(true);
    let geo_w = layout(
        &tree_w,
        Rect::new(0.0, 0.0, SW as f32, SH as f32),
        scroll_style(),
        &ApproxMeasure,
    );
    println!("换行节点几何 = {:?}｜行带 {bands:?}", geo_w["text_1"]);
    assert_eq!(geo_w["text_1"].h, 72.0, "前置：4 行 × 18");
    let state = InteractState::default();
    let list_w = InteractiveRenderer::new(t.clone(), &ApproxMeasure, &state).build(&tree_w, &geo_w);
    let texts = list_w.counts().text;
    let px_w = render(&list_w, SW, SH);
    let ink_w: Vec<usize> = bands.iter().map(|b| ink_in(&px_w, *b)).collect();
    println!("换行：{texts} 条 Text 命令｜各带墨迹 {ink_w:?}");
    assert_eq!(texts, 4, "4 行 ⇒ 4 条命令");
    for (i, n) in ink_w.iter().enumerate() {
        assert!(
            *n >= 100,
            "第 {i} 行（y {}-{}）只有 {n} 像素有墨迹 —— 那一行没画出来",
            bands[i].0,
            bands[i].1
        );
    }

    // 对照：关掉换行 ⇒ 1 条命令、只有第 1 带有墨迹（像素上证明「多行」不是假象）。
    let tree_p = wrap_corpus_gui(false);
    let geo_p = layout(
        &tree_p,
        Rect::new(0.0, 0.0, SW as f32, SH as f32),
        scroll_style(),
        &ApproxMeasure,
    );
    let list_p = InteractiveRenderer::new(t, &ApproxMeasure, &state).build(&tree_p, &geo_p);
    let px_p = render(&list_p, SW, SH);
    let ink_p: Vec<usize> = bands.iter().map(|b| ink_in(&px_p, *b)).collect();
    println!(
        "不换行：{} 条 Text 命令｜各带墨迹 {ink_p:?}",
        list_p.counts().text
    );
    assert_eq!(list_p.counts().text, 1);
    assert!(ink_p[0] >= 100, "第 1 行必须有墨迹");
    for (i, n) in ink_p.iter().enumerate().skip(1) {
        assert_eq!(*n, 0, "不换行时第 {i} 带不该有墨迹（实际 {n}）");
    }
}

// ---------------------------------------------------------------------------
// 五、跨后端：滚动 + 换行的语料在离屏 GPU 与 CPU 上必须逐字节相同
// ---------------------------------------------------------------------------

/// 系统字体引擎（拿不到就**明确跳过**并打印原因，不伪装成通过）。
fn text_engine(size: f32) -> Option<TextEngine> {
    match TextEngine::from_system_font(size) {
        Ok(e) => Some(e),
        Err(e) => {
            eprintln!("跳过：这台机器上拿不到系统字体（{e}）");
            None
        }
    }
}

/// **离屏 GPU vs CPU：不透明语料要求逐字节相同**（`docs/features/pixels.md` 的判据）。
///
/// 为什么这条必须对新功能跑：`PushClip` 与「多行 = 多条 `Text` 命令」都是**后端要消费**的
/// 数据。CPU 后端对得上不能证明 GPU 对得上 —— 两个后端各自实现裁剪与字形采样。
/// 语料用**真实字体度量**（两边都挂 `TextEngine`），否则占位格/图集采样两套模型不可比。
#[test]
fn scrolled_and_wrapped_corpus_matches_the_gpu_backend() {
    let size = 20.0f32;
    let t = Theme {
        font_size: size,
        line_height: 26.0,
        ..Theme::default()
    };
    let ext = Extent {
        width: SW,
        height: SH,
    };
    let Some(e_gpu) = text_engine(size) else { return };
    let Some(e_cpu) = text_engine(size) else { return };
    let style = TextStyle {
        font_size: size,
        line_height: t.line_height,
    };

    // 语料：一个滚动的列（8 个按钮）+ 一个换行的文本节点（w=120）。
    let build = || {
        let mut b = Builder::new(Kind::Column, "app");
        b.container_opts(
            Kind::Column,
            "outer",
            deer_core::builder::L::new()
                .w(200.0)
                .h(100.0)
                .scroll(true)
                .to_props(),
            |o| {
                for i in 1..=8 {
                    o.button(format!("{i}"));
                }
            },
        );
        b.button("below");
        b.text_opts("alpha beta gamma delta epsilon", |n| {
            n.layout.width = Some(deer_core::node::Size::Px(120.0));
            n.layout.wrap = true;
        });
        b.build()
    };
    let tree = build();
    assert!(tree.children[0].is_scroll_container(), "前置：第一个子节点是滚动容器");
    assert!(tree.children[2].wraps_text(), "前置：第三个子节点是换行文本");

    let gpu = match deer_gui::vk::GpuGeometryRenderer::new(0, ext, clear_color()) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("跳过：本机没有可用的 Vulkan GPU（{e}）");
            return;
        }
    };
    let mut gpu = gpu
        .with_text(e_gpu)
        .unwrap_or_else(|e| panic!("with_text 失败（文本管线建不起来）：{e}"));

    // 先把四个偏移的列表都建好（`gpu`/`cpu` 都要吃**同一份**列表），再把引擎交给 CPU 渲染器
    // （`with_text` 会拿走所有权，之后就拿不到 `measure()` 了）。
    let state = InteractState::default();
    let mut cases: Vec<(i32, i32, DrawList)> = Vec::new();
    for offset in [0i32, 26, 40, MAX_SCROLL] {
        let offsets = ScrollOffsets::new().with("outer", offset);
        let (geo, metrics) = deer_core::layout::layout_with_scroll(
            &tree,
            Rect::new(0.0, 0.0, SW as f32, SH as f32),
            style,
            &e_cpu.measure(),
            &offsets,
        );
        let list = InteractiveRenderer::new(t.clone(), &e_cpu.measure(), &state).build(&tree, &geo);
        cases.push((offset, metrics.max_of("outer"), list));
    }
    let mut cpu = CpuRenderer::with_text(e_cpu);

    println!();
    println!("—— 滚动 + 换行：离屏 GPU vs CPU（真实字形，不透明 ⇒ 要求逐字节相同）——");
    let mut worst_overall = 0u8;
    let mut offsets_checked = 0usize;
    for (offset, max_scroll, list) in cases {
        let c = list.counts();
        // 前置：这一帧必须**同时**含裁剪与多行文本，否则「逐字节相同」可能平凡成立。
        assert_eq!((c.push_clip, c.pop_clip), (1, 1), "前置：视口裁剪在");
        assert!(c.text >= 9, "前置：多行文本命令在（实际 {} 条）", c.text);
        assert!(list.clip_balanced());

        let gpu_px = gpu.render(&list).unwrap_or_else(|e| panic!("GPU 渲染失败：{e}"));
        assert!(gpu.unsupported().is_empty(), "文本已被接管 ⇒ 不该有 unsupported");
        let cpu_px = cpu.render(ext, &list, clear_color()).expect("CPU 渲染失败").pixels;
        let worst = gpu_px
            .iter()
            .zip(cpu_px.iter())
            .map(|(a, b)| a.abs_diff(*b))
            .max()
            .unwrap_or(0);
        let diff_bytes = gpu_px.iter().zip(cpu_px.iter()).filter(|(a, b)| a != b).count();
        println!(
            "  偏移 {offset:>2}（max_scroll {max_scroll}）：命令 {} 条（text {}）｜最大通道差 {worst}｜差异字节 {diff_bytes}",
            list.len(),
            c.text
        );
        assert_eq!(gpu_px, cpu_px, "偏移 {offset}：不透明语料必须逐字节相同");
        worst_overall = worst_overall.max(worst);
        offsets_checked += 1;
    }
    assert_eq!(offsets_checked, 4, "四个偏移都要真的跑过");
    assert_eq!(worst_overall, 0, "最大通道差必须为 0");
    println!("四个偏移：GPU vs CPU 逐字节相同 ✅");
}

