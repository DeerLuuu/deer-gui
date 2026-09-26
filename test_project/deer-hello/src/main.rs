use deer_gui::prelude::*;
use deer_gui::render_tree_to_png;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // ① 建一棵界面树：根是「竖排容器」，四周留 12 像素，子元素间距 8 像素
    let mut app = Builder::new(Kind::Column, "app").padding(12.0).gap(8.0);

    // ② 往根里加东西（Column ⇒ 从上往下排）
    app.text("Hello deer-gui");
    app.container_opts(Kind::Row, "bar", L::new().gap(8.0).to_props(), |r| {
        r.button("确定");
        r.button_opts("禁用", |n| {
            n.props.disabled = true;
        });
    });

    // ③ 定稿成一棵只读的树
    let tree = app.build();

    // ④ 渲染成 PNG 并写文件：宽 320、高 140、用默认主题
    let png = render_tree_to_png(&tree, 320, 140, Theme::default())?;
    std::fs::write("hello.png", &png)?;
    println!("写出 hello.png（{} 字节）", png.len());
    Ok(())
}