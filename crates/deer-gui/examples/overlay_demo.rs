//! 功能示例：**L1 绝对定位 / 层叠**（ROADMAP D6/D10，Q5 裁定）。
//!
//! ```sh
//! cargo run -p deer-gui --example overlay_demo
//! ```
//!
//! 演示四件事（每件都有可数断言，不是「看起来对」）：
//!
//! 1. **流外**：设了 `position` 的子节点脱离流内布局 —— 兄弟位置不动、
//!    父容器固有尺寸不含它（数字断言）；
//! 2. **负偏移**：面板可以伸出父盒子之外（矩形数值断言，偏移不夹取）；
//! 3. **层叠 = 声明序**：后声明者后画（重叠点像素证据）且命中优先（`hit` 证据）；
//! 4. **`.dui` 语法**：`pos=x,y` 解析 / 编码往返稳定。
//!
//! 产物：`render_out/overlay_demo.png`（③ 的语料：文字上盖一个流外菜单面板）。

use deer_gui::interaction::{ClipSnapshot, hit};
use deer_gui::prelude::*;

const W: u32 = 220;
const H: u32 = 140;

fn main() {
    let theme = Theme::default();
    let style = TextStyle {
        font_size: theme.font_size,
        line_height: theme.line_height,
    };

    // ---- ① 流外：兄弟不动、父固有尺寸不含它 ----
    // Row（pad=10）：流内按钮 A(36×22) + 流外徽标 B(36×22, pos=120,30)。
    // 若 B 占流内槽，Row 的固有宽会是 36+36+20=92；流外 ⇒ 只有 36+20=56。
    let mut app = Builder::new(Kind::Column, "app");
    let mut id_a = String::new();
    let mut id_b = String::new();
    app.container_opts(Kind::Row, "bar", L::new().pad(10.0).to_props(), |r| {
        id_a = r.button_opts("A", |n| {
            n.layout.width = Some(Size::Px(36.0));
            n.layout.height = Some(Size::Px(22.0));
        });
        id_b = r.button_opts("B", |n| {
            n.layout.width = Some(Size::Px(36.0));
            n.layout.height = Some(Size::Px(22.0));
            n.layout.position = Some(Pos::Offset { x: 120, y: 30 });
        });
    });
    let tree = app.build();
    let intr = measure_tree(&tree, style, &ApproxMeasure);
    println!("① 流外：Row 固有 = {:?}（若 B 参与流会是 (92, 42)）", intr["bar"]);
    assert_eq!(
        intr["bar"], (56.0, 42.0),
        "设了 position 的子节点不许计入父容器固有尺寸"
    );
    let geo = layout(&tree, Rect::new(0.0, 0.0, W as f32, H as f32), style, &ApproxMeasure);
    println!("   A = {:?}（流内，x=pad）", geo[&id_a]);
    println!("   B = {:?}（流外，= 内容盒原点 + (120,30)）", geo[&id_b]);
    assert_eq!(geo[&id_a].x, 10.0, "流内兄弟不许被流外节点挤动");
    assert_eq!(geo[&id_b], Rect::new(130.0, 40.0, 36.0, 22.0), "位置 = 父内容盒原点 + 偏移");
    assert_eq!(geo["bar"].w, 56.0, "父容器的宽不被流外子节点撑大");
    assert!(
        geo[&id_b].x + geo[&id_b].w > geo["bar"].x + geo["bar"].w,
        "流外节点允许伸出父盒子（这是下面负偏移的前提）"
    );

    // ---- ② 负偏移：不夹取 ----
    let mut app = Builder::new(Kind::Column, "app");
    let mut id_n = String::new();
    app.container_opts(Kind::Row, "bar", L::new().pad(10.0).to_props(), |r| {
        id_n = r.button_opts("N", |n| {
            n.layout.width = Some(Size::Px(36.0));
            n.layout.height = Some(Size::Px(22.0));
            n.layout.position = Some(Pos::Offset { x: -5, y: -8 });
        });
    });
    let tree = app.build();
    let geo = layout(&tree, Rect::new(0.0, 0.0, W as f32, H as f32), style, &ApproxMeasure);
    println!("② 负偏移：N = {:?}（内容盒原点 (10,10) + (-5,-8)）", geo[&id_n]);
    assert_eq!(geo[&id_n], Rect::new(5.0, 2.0, 36.0, 22.0), "负偏移不许被夹取");

    // ---- ③ 层叠 = 声明序（这条语料同时是 PNG 产物）----
    // host 里先放一行文字，再声明一个流外菜单面板（后声明 ⇒ 画在最上、命中优先），
    // 面板用负偏移稍微伸出 host 左缘。重叠点上：文字先画、面板后画。
    let mut app = Builder::new(Kind::Column, "app").padding(10.0);
    let mut menu_btn = String::new();
    let mut base_text = String::new();
    app.container_opts(Kind::Column, "host", L::new().pad(6.0).to_props(), |h| {
        base_text = h.text("被盖住的底图文本");
        h.container_opts(Kind::Column, "menu", L::new().pad(6.0).pos(-4, 8).to_props(), |m| {
            menu_btn = m.button("菜单项");
        });
    });
    let tree = app.build();
    let geo = layout(&tree, Rect::new(0.0, 0.0, W as f32, H as f32), style, &ApproxMeasure);
    let (w, _h, px) = deer_gui::render_tree_to_rgba(&tree, W, H, theme.clone())
        .map_err(|e| format!("离屏渲染失败：{e}"))
        .unwrap();
    // 重叠点：同时落在底图文本行与菜单按钮内（几何前置断言，别测空气）
    let probe = (22i32, 31i32);
    let inside = |r: deer_core::Rect, p: (i32, i32)| {
        p.0 >= r.x as i32 && p.1 >= r.y as i32 && p.0 < (r.x + r.w) as i32 && p.1 < (r.y + r.h) as i32
    };
    assert!(inside(geo[&base_text], probe), "前置：探测点必须在底图文本内 {:?}", geo[&base_text]);
    assert!(inside(geo[&menu_btn], probe), "前置：探测点必须在菜单按钮内 {:?}", geo[&menu_btn]);
    let i = ((probe.1 as u32 * w) + probe.0 as u32) as usize * 4;
    let (pr, pg, pb) = (px[i], px[i + 1], px[i + 2]);
    let accent = theme.accent;
    println!("③ 层叠：重叠点 {probe:?} 像素 = rgb({pr},{pg},{pb})（accent = rgb({}, {}, {})）", accent.r, accent.g, accent.b);
    assert_eq!(
        (pr, pg, pb),
        (accent.r, accent.g, accent.b),
        "后声明的流外面板必须盖在底图文本上（像素 = 按钮底色）"
    );
    let got = hit(&tree, &geo, ClipSnapshot::unclipped(), probe.0 as f32, probe.1 as f32)
        .map(|n| n.id.to_string())
        .expect("重叠点必须命中");
    println!("   命中 = {got:?}（不是底图文本）");
    assert_eq!(got, menu_btn, "命中优先级必须跟层叠序一致：面板子树胜出，而不是底图文本");

    // ---- ④ `.dui` 语法：pos=x,y 往返 ----
    let src = "[column name=app]\n  [button name=b label=B pos=10,-20]\n";
    let (tree, warnings) = deer_gui::layout::scene::parse_scene_collect(src, "overlay.dui").unwrap();
    assert!(warnings.is_empty(), "pos 是已知属性，不该有警告：{warnings:?}");
    let once = encode_scene(&tree);
    assert!(once.contains("pos=10,-20"), "pos 必须被编码回去：{once}");
    let back = parse_scene(&once, "overlay.dui").expect("往返可解析");
    assert!(tree.structurally_eq(&back), "往返必须结构相等");
    assert_eq!(encode_scene(&back), once, "二次编码逐字节稳定");
    println!("④ .dui：pos=10,-20 往返稳定 ✅");

    // ---- 产物 ----
    let png = deer_gui::render_tree_to_png(&tree, W, H, theme)
        .map_err(|e| format!("PNG 编码失败：{e}"))
        .unwrap();
    std::fs::create_dir_all("render_out").unwrap();
    std::fs::write("render_out/overlay_demo.png", png).unwrap();
    println!("\n产物：render_out/overlay_demo.png（流外菜单面板盖在文字上，左缘伸出 host）");
    println!("L1 绝对定位 / 层叠：全部自检通过 ✅");
}
