//! 功能示例：**换主题（深色/浅色）+ 自定义颜色**。
//!
//! ```sh
//! cargo run -p deer-gui --example theme
//! ```
//!
//! 产物：`render_out/theme-dark.png` 与 `render_out/theme-light.png` —— 并排对比。

use deer_gui::prelude::*;

/// 深色主题（也是默认值，这里显式写出来方便对照）。
fn dark() -> Theme {
    Theme {
        text: Color::rgb(0xe6, 0xe8, 0xef),
        text_dim: Color::rgb(0x8b, 0x93, 0xa7),
        surface: Color::rgb(0x14, 0x16, 0x20),
        border: Color::rgb(0x2a, 0x2f, 0x3f),
        accent: Color::rgb(0x4c, 0x8d, 0xff),
        on_accent: Color::rgb(0xff, 0xff, 0xff),
        font_size: 13.0,
        line_height: 18.0,
    }
}

/// 浅色主题：把 6 个颜色换掉，字号沿用默认（`..Theme::default()` 表示「其余字段不变」）。
fn light() -> Theme {
    Theme {
        text: Color::rgb(0x1a, 0x1d, 0x28),
        text_dim: Color::rgb(0x6b, 0x73, 0x88),
        surface: Color::rgb(0xf5, 0xf6, 0xfa),
        border: Color::rgb(0xd8, 0xdd, 0xe8),
        accent: Color::rgb(0x2f, 0x6f, 0xe0),
        on_accent: Color::rgb(0xff, 0xff, 0xff),
        ..Theme::default()
    }
}

/// 一个暖色主题，用来说明「不只是明暗，颜色可以任意」。
fn warm() -> Theme {
    Theme {
        text: Color::rgb(0x3a, 0x24, 0x18),
        text_dim: Color::rgb(0x8a, 0x6a, 0x52),
        surface: Color::rgb(0xfd, 0xf3, 0xe6),
        border: Color::rgb(0xe8, 0xd2, 0xb8),
        accent: Color::rgb(0xd9, 0x6b, 0x2b),
        on_accent: Color::rgb(0xff, 0xff, 0xff),
        font_size: 14.0,
        line_height: 20.0,
    }
}

fn demo_tree() -> Node {
    let mut app = Builder::new(Kind::Column, "app").padding(14.0).gap(8.0);
    app.text("主题预览");
    app.container_opts(Kind::Column, "card", L::new().pad(10.0).gap(6.0).to_props(), |c| {
        c.text("这是一段正文");
        c.field("输入框");
        c.container_opts(Kind::Row, "row", L::new().gap(8.0).to_props(), |r| {
            r.button("主要按钮");
            r.button_opts("禁用按钮", |n| n.props.disabled = true);
        });
    });
    app.build()
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    std::fs::create_dir_all("render_out")?;
    let tree = demo_tree();

    for (name, theme) in [("dark", dark()), ("light", light()), ("warm", warm())] {
        let png = deer_gui::render_tree_to_png(&tree, 300, 200, theme)?;
        let out = format!("render_out/theme-{name}.png");
        std::fs::write(&out, &png)?;
        println!("{out}  ({} 字节)", png.len());
    }

    // 自检：不同主题必须产出**不同**的像素，否则说明主题没生效
    let (_, _, a) = deer_gui::render_tree_to_rgba(&tree, 300, 200, dark())?;
    let (_, _, b) = deer_gui::render_tree_to_rgba(&tree, 300, 200, light())?;
    assert_ne!(a, b, "深色与浅色的像素必须不同 —— 否则主题没被用上");
    println!("\n深色与浅色像素确实不同 ✅");
    println!("打开 render_out/theme-*.png 对比三者。");
    Ok(())
}
