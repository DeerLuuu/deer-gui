//! 功能示例：**原始像素**（不用 PNG，直接拿 RGBA8 缓冲）。
//!
//! ```sh
//! cargo run -p deer-gui --example pixels
//! ```
//!
//! 什么时候需要它：想把结果喂给别的库（缩放、编码、比对）、想自己分析像素、
//! 或者想验证「某个位置到底是什么颜色」。

use deer_gui::prelude::*;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut app = Builder::new(Kind::Column, "app").padding(10.0).gap(6.0);
    app.text("像素");
    app.button("按钮");
    let tree = app.build();

    let (w, h) = (120u32, 80u32);
    let theme = Theme::default();

    // 返回 (宽, 高, RGBA8 字节)。**行优先、无 padding**：每 4 字节一个像素。
    let (width, height, pixels) = deer_gui::render_tree_to_rgba(&tree, w, h, theme.clone())?;

    assert_eq!(width, w);
    assert_eq!(height, h);
    assert_eq!(
        pixels.len(),
        (w as usize) * (h as usize) * 4,
        "长度必须是 宽×高×4"
    );
    println!("拿到 {width}×{height} 的 RGBA8 缓冲，共 {} 字节", pixels.len());

    // 读取任意像素的写法：下标 = (y * 宽 + x) * 4
    let read = |x: u32, y: u32| -> [u8; 4] {
        let i = ((y as usize) * (width as usize) + (x as usize)) * 4;
        [pixels[i], pixels[i + 1], pixels[i + 2], pixels[i + 3]]
    };

    println!("\n取几个点看看：");
    for (label, x, y) in [
        ("画布左上角（应是面板底色）", 1u32, 1u32),
        ("文字区域", 14, 14),
        ("按钮上", 20, 40),
        ("画布右下角（应是空白背景）", w - 2, h - 2),
    ] {
        let p = read(x, y);
        println!("  ({x:>3},{y:>3}) {label:<24} R{} G{} B{} A{}", p[0], p[1], p[2], p[3]);
    }

    // 统计不同颜色数 —— 只有一种颜色说明什么都没画上（这种「假成功」很坑）
    let mut colors = std::collections::BTreeSet::new();
    for p in pixels.chunks_exact(4) {
        colors.insert([p[0], p[1], p[2], p[3]]);
    }
    println!("\n画面里不同颜色数 = {}", colors.len());
    assert!(colors.len() > 1, "只有一种颜色 ⇒ 什么都没画上");
    assert!(colors.contains(&[255, 255, 255, 255]) || colors.len() >= 3, "应当有文字/按钮的颜色");

    // 自己编码成 PNG（用库里的零依赖编码器）
    std::fs::create_dir_all("render_out")?;
    let png = deer_gpu::png::encode_rgba(width, height, &pixels).map_err(|e| format!("编码失败：{e}"))?;
    std::fs::write("render_out/pixels.png", &png)?;
    println!("自己编码并写出 render_out/pixels.png（{} 字节）", png.len());

    // 也可以手写一个 PPM（无依赖、纯文本头 + 原始字节），便于用别的工具看
    let mut ppm = format!("P6\n{width} {height}\n255\n").into_bytes();
    for p in pixels.chunks_exact(4) {
        ppm.extend_from_slice(&p[..3]);
    }
    std::fs::write("render_out/pixels.ppm", &ppm)?;
    println!("同时写出 render_out/pixels.ppm（PPM 格式，可直接被多数图片工具打开）");
    Ok(())
}
