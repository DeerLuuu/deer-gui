//! 把一份真实的 UI 渲染成 PNG 文件 —— **现在就能跑**的最小示例。
//!
//! ```sh
//! cargo run -p deer-gui --example render_to_png
//! ```
//!
//! 产物：`render_out/render_to_png.png`（相对于仓库根运行时的路径）。
//!
//! 说明：渲染是**离屏**的（CPU 软件光栅化），不涉及窗口。窗口渲染在里程碑 M2b。
//! 这条路径**不带字体**，所以文字仍是等宽格占位（`ApproxMeasure` 度量）；
//! 要真实字形请看 `examples/text_render.rs`（里程碑 M4）。

use deer_gui::prelude::*;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // ── ① 用命令式 API 构筑界面 ───────────────────────────────────────────
    let mut app = Builder::new(Kind::Column, "app")
        .padding(12.0)
        .gap(8.0);
    app.text("deer-gui 渲染示例");
    app.container_opts(Kind::Column, "panel", L::new().pad(10.0).gap(6.0).to_props(), |p| {
        p.text("用户名");
        p.field("请输入");
        p.text("选项");
        p.container_opts(Kind::Row, "row", L::new().gap(8.0).to_props(), |r| {
            r.button("确定");
            r.button("取消");
            r.button_opts("禁用", |n| n.props.disabled = true);
        });
    });
    let tree = app.build();

    // ── ② 也可以从 .dui 场景文件建同一棵树 ──────────────────────────────
    // 两种写法产出**结构相等**的树（README 的 t1 断言钉住这一点）。
    let scene_text = encode_scene(&tree);
    let from_file = parse_scene(&scene_text, "example.dui")?;
    assert!(
        tree.structurally_eq(&from_file),
        "两条构筑路径应当产出同一棵树"
    );
    println!("场景文件往返一致，节点数 = {}", count_nodes(&tree));

    // ── ③ 算几何（想看布局就打印它） ────────────────────────────────────
    let (w, h) = (360u32, 220u32);
    let theme = Theme::default();
    let geo = deer_gui::layout_tree(&tree, w, h, theme.clone());
    println!("\n几何表（nodeId -> x,y,w,h）:");
    let mut ids: Vec<_> = geo.keys().cloned().collect();
    ids.sort();
    for id in ids {
        let r = geo[&id];
        println!("  {id:<12} {:>6.1},{:>6.1}  {:>6.1}×{:<6.1}", r.x, r.y, r.w, r.h);
    }

    // ── ④ 渲染成像素 → PNG ──────────────────────────────────────────────
    let png = deer_gui::render_tree_to_png(&tree, w, h, theme)?;
    // 输出到 render_out/（示例产物集中放，不散落在仓库根）
    std::fs::create_dir_all("render_out")?;
    let out = "render_out/render_to_png.png";
    std::fs::write(out, &png)?;

    // 顺手做个自检：画出来的不能是纯背景色（否则「渲染成功」是假的）
    let (_, _, px) = deer_gui::render_tree_to_rgba(&tree, w, h, Theme::default())?;
    let distinct = distinct_colors(&px);
    println!("\n写出 {out}：{} 字节 / {w}×{h}", png.len());
    println!("画面里不同颜色数 = {distinct}（>1 才说明真的画了东西）");
    assert!(distinct > 1, "渲染结果只有一种颜色 ⇒ 什么都没画上");

    println!("\n用图片查看器打开 {out} 即可看到结果。");
    Ok(())
}

fn count_nodes(n: &Node) -> usize {
    let mut c = 0;
    n.walk(&mut |_, _| c += 1, 0);
    c
}

fn distinct_colors(px: &[u8]) -> usize {
    let mut set = std::collections::BTreeSet::new();
    for p in px.chunks_exact(4) {
        set.insert([p[0], p[1], p[2], p[3]]);
    }
    set.len()
}
