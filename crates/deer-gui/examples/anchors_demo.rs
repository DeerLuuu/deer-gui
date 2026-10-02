//! 功能示例：**L4 anchors 锚定**（ROADMAP D6/D10/Q5 裁定：与 position 同一个机制）。
//!
//! ```sh
//! cargo run -p deer-gui --example anchors_demo
//! ```
//!
//! 演示四件事（每件都有可数断言，不是「看起来对」）：
//!
//! 1. **撑满 + resize（本特性的存在意义）**：同一棵锚定树分别用两种盒子 layout，
//!    锚定边跟随新盒、内缩偏移保持（两组数字断言）；
//! 2. **角标**：右下角徽标（四边全锚 + 负偏移）跨越内容盒角点居中，
//!    双锚轴的显式 w/h 被忽略（数字断言 + 像素证据）；
//! 3. **单边锚**：只锚右缘 ⇒ 右缘贴新盒、尺寸用显式值（resize 前后各一组数字）；
//! 4. **`.dui` 语法**：逐边属性 `anchor-*=…` 解析，编码成规范形 `pos=anchors:…` 往返稳定。
//!
//! 产物：`render_out/anchors_demo.png`（② 的语料：锚定背景 + 角标徽标）。

use deer_gui::prelude::*;

const W: u32 = 200;
const H: u32 = 100;

/// 锚定演示树：根 100%×100%（⇒ 根矩形 = 给的盒子）、pad=10；
/// - `backdrop`：撑满（l=0,t=0,r=1,b=1）+ 内缩 8px，禁用态（border 底色当背景板）；
/// - `hint`：流内文本（锚定节点不占流，照常排在内容盒原点）；
/// - `badge`：角标（四边全锚 + 负偏移），显式 99×99 必须被双锚轴忽略。
///
/// 百分比只设在**根**上 —— 根的 `avail` 才是宿主给的盒子（子节点的百分比相对
/// 父内容盒，父是固有尺寸的话就撑不满盒子了）。
fn demo_tree() -> (Node, String, String, String) {
    let mut host = Builder::new(Kind::Column, "host").padding(10.0);
    let id_backdrop = host.button_opts("背景板", |n| {
        n.props.disabled = true;
        n.layout.position = Some(Pos::Anchors {
            l: Some(0.0),
            t: Some(0.0),
            r: Some(1.0),
            b: Some(1.0),
            ox: 8,
            oy: 8,
        });
    });
    let id_hint = host.text("流内文本照常排布");
    let id_badge = host.button_opts("!", |n| {
        n.layout.width = Some(Size::Px(99.0));
        n.layout.height = Some(Size::Px(99.0));
        n.layout.position = Some(Pos::Anchors {
            l: Some(1.0),
            t: Some(1.0),
            r: Some(1.0),
            b: Some(1.0),
            ox: -8,
            oy: -8,
        });
    });
    let mut tree = host.build();
    tree.layout.width = Some(Size::Pct(100.0));
    tree.layout.height = Some(Size::Pct(100.0));
    (tree, id_backdrop, id_hint, id_badge)
}

fn main() {
    let theme = Theme::default();
    let style = TextStyle {
        font_size: theme.font_size,
        line_height: theme.line_height,
    };

    // ---- ① 撑满 + resize：锚定边跟随新盒，偏移保持（两组数字）----
    let (tree, id_backdrop, _id_hint, id_badge) = demo_tree();
    let geo_small = layout(&tree, Rect::new(0.0, 0.0, 200.0, 100.0), style, &ApproxMeasure);
    let geo_big = layout(&tree, Rect::new(0.0, 0.0, 320.0, 220.0), style, &ApproxMeasure);
    // 前置：两种盒子下根都恰好撑满盒子（w=h=100%），内容盒 = 盒子内缩 10
    assert_eq!(geo_small["host"], Rect::new(0.0, 0.0, 200.0, 100.0), "前置：根 = 小盒");
    assert_eq!(geo_big["host"], Rect::new(0.0, 0.0, 320.0, 220.0), "前置：根 = 大盒（确实 resize 了）");

    println!("① 撑满（l=0,t=0,r=1,b=1 + 内缩 8px）：");
    println!("   小盒 200×100 ⇒ backdrop = {:?}（内容盒 (10,10,180,80) 内缩 8）", geo_small[&id_backdrop]);
    println!("   大盒 320×220 ⇒ backdrop = {:?}（内容盒 (10,10,300,200) 内缩 8）", geo_big[&id_backdrop]);
    assert_eq!(
        geo_small[&id_backdrop],
        Rect::new(18.0, 18.0, 164.0, 64.0),
        "撑满 + 内缩式偏移 = 内容盒四边各内缩 8px（小盒）"
    );
    assert_eq!(
        geo_big[&id_backdrop],
        Rect::new(18.0, 18.0, 284.0, 184.0),
        "撑满 + 内缩式偏移 = 内容盒四边各内缩 8px（大盒）—— 锚定边跟随新盒"
    );

    // ---- ② 角标：四边全锚 + 负偏移 ⇒ 跨越内容盒角点居中；显式 w/h 被忽略 ----
    let corner = (
        geo_small["host"].x + 10.0 + geo_small["host"].w - 20.0,
        geo_small["host"].y + 10.0 + geo_small["host"].h - 20.0,
    ); // 内容盒右下角 = (190, 90)
    println!("② 角标（l=r=1,t=b=1, 偏移 −8）：badge = {:?}，内容盒角点 = {corner:?}", geo_small[&id_badge]);
    assert_eq!(geo_small[&id_badge], Rect::new(182.0, 82.0, 16.0, 16.0), "角标 = 角点两侧各 8px（16×16）");
    assert_eq!(geo_small[&id_badge].w, 16.0, "双锚轴的显式 99px 被忽略（不是报错，D10）");
    assert_eq!(
        (geo_small[&id_badge].x + geo_small[&id_badge].w / 2.0, geo_small[&id_badge].y + geo_small[&id_badge].h / 2.0),
        corner,
        "徽标中心 = 内容盒右下角点"
    );

    // ---- ③ 单边锚：只锚右缘 ⇒ 右缘贴新盒、尺寸用显式值、偏移保持 ----
    let mut host = Builder::new(Kind::Column, "host").padding(10.0);
    let id_pin = host.button_opts("钉住", |n| {
        n.layout.width = Some(Size::Px(40.0));
        n.layout.height = Some(Size::Px(20.0));
        n.layout.position = Some(Pos::Anchors {
            l: None,
            t: None,
            r: Some(1.0),
            b: None,
            ox: -6,
            oy: 0,
        });
    });
    let mut pin_tree = host.build();
    pin_tree.layout.width = Some(Size::Pct(100.0));
    pin_tree.layout.height = Some(Size::Pct(100.0));
    let pin_small = layout(&pin_tree, Rect::new(0.0, 0.0, 200.0, 100.0), style, &ApproxMeasure);
    let pin_big = layout(&pin_tree, Rect::new(0.0, 0.0, 320.0, 220.0), style, &ApproxMeasure);
    println!("③ 单边锚 r=1 + 偏移 −6：小盒 = {:?}，大盒 = {:?}", pin_small[&id_pin], pin_big[&id_pin]);
    assert_eq!(pin_small[&id_pin], Rect::new(156.0, 10.0, 40.0, 20.0), "右缘 = 内容盒右缘悬出 6px（小盒）");
    assert_eq!(pin_big[&id_pin], Rect::new(276.0, 10.0, 40.0, 20.0), "右缘跟随**新**内容盒（大盒）");
    for g in [&pin_small, &pin_big] {
        let overhang = (g[&id_pin].x + g[&id_pin].w) - (g["host"].x + g["host"].w - 10.0);
        assert_eq!(overhang, 6.0, "resize 时偏移保持（悬出量恒为 6px）");
    }

    // ---- 像素证据（①② 的语料即 PNG 产物）----
    // 徽标内避开字形「!」的两个点必须是按钮底色（accent）⇒ 角标真的画在了角点上；
    // 背景板上避开文字的一点必须是禁用态底色（border）⇒ 撑满的锚定背景真的画出来了。
    let (w, _h, px) = deer_gui::render_tree_to_rgba(&tree, W, H, theme.clone())
        .map_err(|e| format!("离屏渲染失败：{e}"))
        .unwrap();
    let pixel = |x: u32, y: u32| {
        let i = ((y * w) + x) as usize * 4;
        (px[i], px[i + 1], px[i + 2])
    };
    let (accent, border) = (theme.accent, theme.border);
    for p in [(185u32, 85u32), (195u32, 95u32)] {
        let got = pixel(p.0, p.1);
        assert_eq!(
            got,
            (accent.r, accent.g, accent.b),
            "角标内 {p:?} 应是徽标按钮底色 accent —— 角标没画出来或层序坏了"
        );
    }
    let got = pixel(30, 60);
    assert_eq!(
        got,
        (border.r, border.g, border.b),
        "背景板内 (30,60) 应是禁用态底色 border —— 撑满锚定没画出来"
    );
    println!("   像素证据：角标内 = accent，背景板内 = border ✅");

    // ---- ④ `.dui` 语法：逐边属性解析 + 规范形往返 ----
    let src = "[column name=app pad=10]\n\
               \x20 [button name=badge label=! anchor-l=1 anchor-t=1 anchor-r=1 anchor-b=1 anchor-ox=-8 anchor-oy=-8]\n\
               \x20 [button name=pin anchor-r=1 anchor-ox=-6]\n";
    let (dui_tree, warnings) = deer_gui::layout::scene::parse_scene_collect(src, "anchors.dui").unwrap();
    assert!(warnings.is_empty(), "anchor-* 是已知属性，不该有警告：{warnings:?}");
    let once = encode_scene(&dui_tree);
    assert!(once.contains("pos=anchors:1,1,1,1,-8,-8"), "编码成规范形：{once}");
    assert!(once.contains("pos=anchors:-,-,1,-,-6,0"), "无锚边编码成 `-`：{once}");
    let back = parse_scene(&once, "anchors.dui").expect("规范形能读回");
    assert!(dui_tree.structurally_eq(&back), "逐边写法 ↔ 规范形：往返结构相等");
    assert_eq!(encode_scene(&back), once, "二次编码逐字节稳定");
    println!("④ .dui：anchor-* 逐边写法 ⇄ pos=anchors:… 规范形，往返稳定 ✅");

    // ---- 产物 ----
    let png = deer_gui::render_tree_to_png(&tree, W, H, theme)
        .map_err(|e| format!("PNG 编码失败：{e}"))
        .unwrap();
    std::fs::create_dir_all("render_out").unwrap();
    std::fs::write("render_out/anchors_demo.png", png).unwrap();
    println!("\n产物：render_out/anchors_demo.png（锚定背景撑满内缩 8px + 右下角角标徽标）");
    println!("L4 anchors 锚定：全部自检通过 ✅");
}
