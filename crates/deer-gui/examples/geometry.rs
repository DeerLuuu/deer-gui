//! 功能示例：**只看布局，不渲染**。
//!
//! ```sh
//! cargo run -p deer-gui --example geometry
//! ```
//!
//! 调界面尺寸时最有用：直接看每个控件被算到哪个矩形。

use deer_gui::prelude::*;

fn main() {
    // 建一棵典型的表单树
    let mut app = Builder::new(Kind::Column, "app").padding(16.0).gap(10.0);
    app.text("账户设置");
    let email = app.field("邮箱");
    app.container_opts(Kind::Row, "actions", L::new().gap(8.0).to_props(), |r| {
        r.button("保存");
        r.button("取消");
    });
    let tree = app.build();

    let theme = Theme::default();
    let (w, h) = (360u32, 200u32);

    // 只要几何：nodeId → Rect。第三个参数是画布大小（根盒子由宿主给）。
    let geo = deer_gui::layout_tree(&tree, w, h, theme);

    println!("画布 {w}×{h}，共 {} 个节点\n", geo.len());
    println!("{:<12} {:>7} {:>7} {:>7} {:>7}", "节点", "x", "y", "宽", "高");
    println!("{}", "-".repeat(46));
    for (id, r) in &geo {
        println!("{id:<12} {:>7.1} {:>7.1} {:>7.1} {:>7.1}", r.x, r.y, r.w, r.h);
    }

    // 命中的坐标 → 哪个控件？（这是输入路由的基础，见 features/hit-testing.md）
    let btn = geo["button_1"];
    let hit = hit_test(&tree, &geo, btn.x + 2.0, btn.y + 2.0);
    println!(
        "\n({:.0}, {:.0}) 处命中：{}",
        btn.x + 2.0,
        btn.y + 2.0,
        hit.map(|n| n.id.as_str()).unwrap_or("<无>")
    );

    // 自检：几何不能是空的（否则「跑成功了」是假的），且命中必须真的是按钮
    assert!(geo.len() >= 5, "应当至少算出 5 个节点的几何");
    assert_eq!(
        hit.map(|n| n.id.as_str()),
        Some("button_1"),
        "按钮内部一点必须命中它自己（而不是容器）"
    );

    // 顺带验证「不在任何控件上」时返回 None
    let outside = hit_test(&tree, &geo, w as f32 + 10.0, h as f32 + 10.0);
    assert!(outside.is_none(), "画布外不应命中任何节点");
    println!("框外命中：<无> ✅");
    let _ = email;
}
