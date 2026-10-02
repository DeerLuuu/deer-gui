//! 功能示例：**L2 每子节点交叉轴对齐（cross_self）+ L3 最小/最大尺寸（min/max）**。
//!
//! ```sh
//! cargo run -p deer-gui --example layout_refine_demo
//! ```
//!
//! 一个示例同时演示两个特性（每件都有数值断言 + 像素断言，不是「看起来对」）：
//!
//! 1. **L2**：容器 `cross=start` 时，三个子节点分别用默认 / `cross_self=center` /
//!    `cross_self=end` —— 各自落到自己的交叉轴位置，兄弟不受影响
//!    （矩形数值断言 + 渲染像素断言：end 对齐的按钮在**只有它该在的位置**是 accent 色）；
//! 2. **L3 grow**：grow 分配结果被 `max_w` 封顶（兄弟落位用**夹过**的尺寸）、
//!    被 `min_w` 托底（可溢出容器）；
//! 3. **L3 显式夹取 + 冲突**：显式尺寸超 max 被夹、低于 min 被托；
//!    `min > max` ⇒ **min 赢**；
//! 4. **L3 像素证据**：固有 28px 的按钮被 `min_w=60` 托底 —— 第 50px 处只有托过才有像素；
//! 5. **`.dui` 语法**：`cross-self=center min-w=36 max-h=200` 解析 / 编码往返稳定。
//!
//! 产物：`render_out/layout_refine_demo.png`（① 的语料：一行三个交叉轴对齐各异的按钮）。

use deer_gui::prelude::*;

const W: u32 = 320;
const H: u32 = 220;

/// 取渲染缓冲里 (x, y) 的像素 RGB。
fn px_at(px: &[u8], w: u32, x: i32, y: i32) -> (u8, u8, u8) {
    let i = ((y as u32 * w) + x as u32) as usize * 4;
    (px[i], px[i + 1], px[i + 2])
}

fn main() {
    let theme = Theme::default();
    let style = TextStyle {
        font_size: theme.font_size,
        line_height: theme.line_height,
    };
    let accent = theme.accent;
    let is_accent = |c: (u8, u8, u8)| (c.0, c.1, c.2) == (accent.r, accent.g, accent.b);

    // ---- ① L2：cross_self 覆盖容器级 cross，只对该子节点生效 ----
    // Row(pad=10, 300×80)：inner_h=60；三个按钮只声明主轴宽 20（交叉轴用固有高 22 ——
    // 显式交叉轴尺寸优先于 stretch/对齐，是 I-7 的既有优先级）。
    let mut app = Builder::new(Kind::Column, "app");
    let mut id_a = String::new();
    let mut id_b = String::new();
    let mut id_c = String::new();
    app.container_opts(Kind::Row, "bar", L::new().pad(10.0).w(300.0).h(80.0).to_props(), |r| {
        id_a = r.button_opts("A", |n| n.layout.width = Some(Size::Px(20.0)));
        id_b = r.button_opts("B", |n| {
            n.layout.width = Some(Size::Px(20.0));
            n.layout.cross_self = Some(Align::Center);
        });
        id_c = r.button_opts("C", |n| {
            n.layout.width = Some(Size::Px(20.0));
            n.layout.cross_self = Some(Align::End);
        });
    });
    let tree = app.build();
    let tree_l2 = tree.clone(); // ① 的语料留给末尾出 PNG
    let geo = layout(&tree, Rect::new(0.0, 0.0, W as f32, H as f32), style, &ApproxMeasure);
    println!("① L2：A={:?} B={:?} C={:?}", geo[&id_a], geo[&id_b], geo[&id_c]);
    // 前置：inner_h=60，子高 22 ⇒ slack=38 > 0，对齐才有区分度
    assert_eq!(geo["bar"].h - 20.0 - 22.0, 38.0, "前置：交叉轴确有剩余空间");
    // A 未设 ⇒ 容器级默认 Start；B center；C end —— 互不干扰
    assert_eq!(geo[&id_a], Rect::new(10.0, 10.0, 20.0, 22.0), "None ⇒ 回落容器级 Start");
    assert_eq!(geo[&id_b], Rect::new(30.0, 29.0, 20.0, 22.0), "cross_self=center：y = 10 + 38/2");
    assert_eq!(geo[&id_c], Rect::new(50.0, 48.0, 20.0, 22.0), "cross_self=end：y = 10 + 38");

    let (w, _h, px) = deer_gui::render_tree_to_rgba(&tree, W, H, theme.clone())
        .map_err(|e| format!("离屏渲染失败：{e}"))
        .unwrap();
    // 像素证据：C 的**左下角填充**是按钮色（若 cross_self 被忽略，C 会在 y=10，这里是容器底色）。
    // 探测点特意避开居中画的 label 字形（那是 on_accent 白色，不是按钮填充色）。
    let c_probe = (52i32, 67i32);
    assert!(
        c_probe.0 >= geo[&id_c].x as i32
            && c_probe.1 >= geo[&id_c].y as i32
            && c_probe.0 < (geo[&id_c].x + geo[&id_c].w) as i32
            && c_probe.1 < (geo[&id_c].y + geo[&id_c].h) as i32,
        "前置：探测点在 C 矩形内"
    );
    let c_pix = px_at(&px, w, c_probe.0, c_probe.1);
    println!("   C 中心 {c_probe:?} = rgb({},{},{})（accent = rgb({},{},{})）", c_pix.0, c_pix.1, c_pix.2, accent.r, accent.g, accent.b);
    assert!(is_accent(c_pix), "end 对齐的按钮必须真的画在 end 位置");
    // 判别力对照：同一行里按钮外的容器底色不是 accent（否则上面的断言在测空气）
    assert!(!is_accent(px_at(&px, w, 60, 75)), "对照点必须是容器底色");

    // ---- ② L3：grow 分配结果被 max 封顶 / 被 min 托底 ----
    let mut app = Builder::new(Kind::Column, "app");
    let mut id_g1 = String::new();
    let mut id_g2 = String::new();
    app.container_opts(Kind::Row, "row", L::new().w(300.0).h(40.0).to_props(), |r| {
        id_g1 = r.button_opts("G1", |n| {
            n.layout.grow = 1.0;
            n.layout.max_w = Some(Size::Px(80.0));
        });
        id_g2 = r.button_opts("G2", |n| {
            n.layout.grow = 1.0;
            n.layout.max_w = Some(Size::Px(80.0));
        });
    });
    let tree = app.build();
    let geo = layout(&tree, Rect::new(0.0, 0.0, W as f32, H as f32), style, &ApproxMeasure);
    println!("② grow+max：G1={:?} G2={:?}", geo[&id_g1], geo[&id_g2]);
    assert_eq!(geo[&id_g1].w, 80.0, "grow 分配结果（本来 150）被 max_w 封顶");
    assert_eq!(geo[&id_g2].x, 80.0, "兄弟落位用的是**夹过**的尺寸（不是 150）");
    assert_eq!(geo[&id_g2].w, 80.0);

    let mut app = Builder::new(Kind::Column, "app");
    let mut id_m1 = String::new();
    let mut id_m2 = String::new();
    app.container_opts(Kind::Row, "row", L::new().w(100.0).h(40.0).to_props(), |r| {
        id_m1 = r.button_opts("M1", |n| {
            n.layout.grow = 1.0;
            n.layout.min_w = Some(Size::Px(80.0));
        });
        id_m2 = r.button_opts("M2", |n| {
            n.layout.grow = 1.0;
            n.layout.min_w = Some(Size::Px(80.0));
        });
    });
    let tree = app.build();
    let geo = layout(&tree, Rect::new(0.0, 0.0, W as f32, H as f32), style, &ApproxMeasure);
    println!("   grow+min：M1={:?} M2={:?}（总和 160 > 容器 100 ⇒ 明确的溢出语义）", geo[&id_m1], geo[&id_m2]);
    assert_eq!(geo[&id_m1].w, 80.0, "grow 分配结果（本来 50）被 min_w 托底");
    assert_eq!(geo[&id_m2].x, 80.0);
    assert!(geo[&id_m2].x + geo[&id_m2].w > 100.0, "min 托底可溢出容器（可见性由裁剪决定）");

    // ---- ③ L3：显式夹取 + min > max ⇒ min 赢 ----
    let mut app = Builder::new(Kind::Column, "app");
    let mut id_x1 = String::new();
    let mut id_x2 = String::new();
    let mut id_x3 = String::new();
    app.container_opts(Kind::Row, "row", L::new().pad(10.0).w(200.0).h(120.0).to_props(), |r| {
        id_x1 = r.button_opts("X1", |n| {
            n.layout.width = Some(Size::Px(100.0));
            n.layout.height = Some(Size::Px(20.0));
            n.layout.max_w = Some(Size::Px(50.0));
        });
        id_x2 = r.button_opts("X2", |n| {
            n.layout.width = Some(Size::Px(10.0));
            n.layout.height = Some(Size::Px(20.0));
            n.layout.min_w = Some(Size::Px(30.0));
        });
        id_x3 = r.button_opts("X3", |n| {
            n.layout.width = Some(Size::Px(100.0));
            n.layout.height = Some(Size::Px(10.0));
            n.layout.min_w = Some(Size::Px(60.0));
            n.layout.max_w = Some(Size::Px(30.0));
            n.layout.min_h = Some(Size::Px(80.0));
            n.layout.max_h = Some(Size::Px(40.0));
        });
    });
    let tree = app.build();
    let geo = layout(&tree, Rect::new(0.0, 0.0, W as f32, H as f32), style, &ApproxMeasure);
    println!("③ 显式夹取：X1={:?} X2={:?} X3={:?}", geo[&id_x1], geo[&id_x2], geo[&id_x3]);
    assert_eq!(geo[&id_x1].w, 50.0, "显式 100 超 max_w=50 被夹");
    assert_eq!(geo[&id_x2].w, 30.0, "显式 10 低于 min_w=30 被托");
    assert_eq!(geo[&id_x3].w, 60.0, "min(60) > max(30) ⇒ min 赢");
    assert_eq!(geo[&id_x3].h, 80.0, "高度方向同样 min 赢");
    assert_eq!(geo[&id_x3].x, 90.0, "兄弟落位用夹过的宽度（50+30 后才轮到 X3）");

    // ---- ④ L3 像素证据：min_w 托底把按钮从固有 28 撑到 60 ----
    let mut app = Builder::new(Kind::Column, "app");
    let mut id_p = String::new();
    app.container_opts(Kind::Row, "row", L::new().pad(10.0).w(200.0).h(60.0).to_props(), |r| {
        id_p = r.button_opts("M", |n| n.layout.min_w = Some(Size::Px(60.0)));
    });
    let tree = app.build();
    let geo = layout(&tree, Rect::new(0.0, 0.0, W as f32, H as f32), style, &ApproxMeasure);
    println!("④ min 托底：M={:?}（固有宽 28 → 60）", geo[&id_p]);
    assert_eq!(geo[&id_p], Rect::new(10.0, 10.0, 60.0, 22.0), "固有 28 被 min_w=60 托底");
    let (w, _h, px) = deer_gui::render_tree_to_rgba(&tree, W, H, theme.clone())
        .map_err(|e| format!("离屏渲染失败：{e}"))
        .unwrap();
    // 第 50px 处（超出固有 28，落在托出来的 32px 里）只有托过才有按钮像素
    let grown = px_at(&px, w, 60, 21);
    assert!(is_accent(grown), "min_w 托出来的区域必须真的有按钮像素：{grown:?}");
    assert!(!is_accent(px_at(&px, w, 80, 21)), "托出区域之外是容器底色（判别力对照）");

    // ---- ⑤ `.dui`：cross-self / min-w / max-h 往返 ----
    let src = "[button name=b label=B cross-self=center min-w=36 max-h=200]\n";
    let (tree, warnings) = deer_gui::layout::scene::parse_scene_collect(src, "refine.dui").unwrap();
    assert!(warnings.is_empty(), "都是已知属性，不该有警告：{warnings:?}");
    assert_eq!(tree.layout.cross_self, Some(Align::Center));
    assert_eq!(tree.layout.min_w, Some(Size::Px(36.0)));
    assert_eq!(tree.layout.max_h, Some(Size::Px(200.0)));
    let once = encode_scene(&tree);
    for attr in ["cross-self=center", "min-w=36", "max-h=200"] {
        assert!(once.contains(attr), "{attr} 必须被编码回去：{once}");
    }
    let back = parse_scene(&once, "refine.dui").expect("往返可解析");
    assert!(tree.structurally_eq(&back), "往返必须结构相等");
    assert_eq!(encode_scene(&back), once, "二次编码逐字节稳定");
    println!("⑤ .dui：cross-self / min-w / max-h 往返稳定 ✅");

    // ---- 产物 ----
    let png = deer_gui::render_tree_to_png(&tree_l2, W, H, theme)
        .map_err(|e| format!("PNG 编码失败：{e}"))
        .unwrap();
    std::fs::create_dir_all("render_out").unwrap();
    std::fs::write("render_out/layout_refine_demo.png", png).unwrap();
    println!("\n产物：render_out/layout_refine_demo.png（cross-self / min-w / max-h 演示按钮）");
    println!("L2 cross_self + L3 min/max：全部自检通过 ✅");
}
