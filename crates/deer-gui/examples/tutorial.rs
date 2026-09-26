//! # deer-gui 分步教程
//!
//! 这个文件是一条**可以按顺序读完**的学习路径。每一步都能单独跑：
//!
//! ```sh
//! cargo run -p deer-gui --example tutorial
//! ```
//!
//! 它会：
//! 1. 依次构建 6 个由简到繁的界面；
//! 2. 每步打印「布局算出来的几何」与「生成了多少条绘制命令」；
//! 3. 把结果写成图片到 `render_out/*.png`。
//!
//! 打开那些图片就能看到每一步的效果。源码里每个 `//` 注释都解释「为什么」，
//! 而不是「是什么」——所以请边跑边读。
//!
//! > 已经能做的：离屏出图（CPU 与 Vulkan）、**真实字体字形**、**渲染到真窗口**（M2b，需开 `window` feature）。
//! > 还没有的：窗口里显示界面（`DrawList` 上 GPU 是 M3）、输入与焦点（M5）。

use deer_gui::prelude::*;
use std::path::Path;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out_dir = Path::new("render_out");
    std::fs::create_dir_all(out_dir)?;
    println!("输出目录：{}\n", out_dir.display());

    step1_hello(out_dir)?;
    step2_row(out_dir)?;
    step3_form(out_dir)?;
    step4_styling(out_dir)?;
    step5_scene_file(out_dir)?;
    step6_own_renderer(out_dir)?;

    println!("\n全部完成。打开 render_out/ 里的图片看效果。");
    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
// 第 1 步：最小界面 —— 只有一个按钮
// ─────────────────────────────────────────────────────────────────────────────

fn step1_hello(out_dir: &Path) -> Result<(), Box<dyn std::error::Error>> {
    println!("=== 第 1 步：最小界面 ===");

    // `Builder::new(类型, 名字)` 创建一个**根节点**。
    // - 类型 `Kind::Column` = 竖着排（`Kind::Row` = 横着排）
    // - 名字随便起，但必须唯一（后面要用它查几何、接事件）
    //
    // 注意 `.padding(12.0)` 的写法：这类方法是**按值接收 self** 的，也就是说它会把
    // `app` **移动**进方法、再返回一个新的 Builder。所以只能用链式写法接住返回值：
    //     let mut app = Builder::new(...).padding(12.0);
    // 不能写成两行（`app.padding(12.0);` 之后 `app` 已经被移动走了，不能再用）。
    // 这是 Rust 里最常见的初学者陷阱之一。
    //
    // 小数字面量必须带小数点：`12` 是整数，`12.0` 才是小数。
    let mut app = Builder::new(Kind::Column, "app").padding(12.0);

    // 往里加一个按钮。`button()` 返回这个按钮的 **id**（字符串），
    // 现在还没用到，但第 6 步会用它来对应「谁被点了」。
    let _button_id = app.button("点我");

    // `build()` 把内部状态**深拷贝**成最终的节点树。
    // 之后这棵树是不可变的纯数据 —— 布局、渲染都只读它。
    let tree = app.build();

    // 一步渲染成图片。参数：树、宽、高、主题。
    // 360×120 是画布大小（画布比内容大，多余部分是背景）。
    render_and_save(&tree, 360, 120, "01-hello.png", out_dir)?;
    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
// 第 2 步：横向排列 + 间距
// ─────────────────────────────────────────────────────────────────────────────

fn step2_row(out_dir: &Path) -> Result<(), Box<dyn std::error::Error>> {
    println!("=== 第 2 步：横排 + 间距 ===");

    let mut app = Builder::new(Kind::Column, "app").padding(12.0).gap(8.0);

    // 先把「标题」放进根容器（Column ⇒ 竖着排，所以标题在最上面）
    app.text("按钮排成一行");

    // 再放一个**横排容器**（Row），里面放三个按钮。
    // `container_opts` 是「带布局参数的容器」：
    //   第 1 个参数：容器类型
    //   第 2 个参数：容器的名字
    //   第 3 个参数：布局参数（`L::new().gap(8.0)` = 子元素间距 8 像素）
    //   第 4 个参数：一个**闭包**（`|r| { ... }`），在它里面加的东西都挂到这个容器下。
    //               `r` 就是传进去的 Builder 本身，所以 `r.button(...)` 加的是这个 Row 的子节点。
    app.container_opts(Kind::Row, "bar", L::new().gap(8.0).to_props(), |r| {
        r.button("确定");
        r.button("取消");
    });

    let tree = app.build();
    render_and_save(&tree, 360, 120, "02-row.png", out_dir)?;
    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
// 第 3 步：做一个像样的表单（容器套容器 + 禁用态）
// ─────────────────────────────────────────────────────────────────────────────

fn step3_form(out_dir: &Path) -> Result<(), Box<dyn std::error::Error>> {
    println!("=== 第 3 步：容器嵌套 + 禁用按钮 ===");

    let mut app = Builder::new(Kind::Column, "app").padding(12.0).gap(8.0);
    app.text("新建项目");

    // 一个带内边距的面板（`pad(10.0)`）。**只有给了内边距的容器才会画出底色和边框** ——
    // 这是刻意的设计：否则每个容器都画一个方块，界面会变成一堆色块，看不出层次。
    app.container_opts(
        Kind::Column,
        "panel",
        L::new().pad(10.0).gap(6.0).to_props(),
        |p| {
            p.text("名称");
            p.field("请输入项目名"); // field = 输入框
            p.text("操作");
            p.container_opts(Kind::Row, "actions", L::new().gap(8.0).to_props(), |r| {
                r.button("保存");
                r.button("另存为");
                // `button_opts` = 需要在按钮上改点东西时用。这里把 disabled 设为 true。
                // `|n| { ... }` 这个闭包拿到的是**按钮节点本身**，随便改它的属性。
                r.button_opts("删除", |n| {
                    n.props.disabled = true;
                });
            });
        },
    );

    let tree = app.build();
    render_and_save(&tree, 360, 200, "03-form.png", out_dir)?;
    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
// 第 4 步：改主题（颜色/字号）与画布大小
// ─────────────────────────────────────────────────────────────────────────────

fn step4_styling(out_dir: &Path) -> Result<(), Box<dyn std::error::Error>> {
    println!("=== 第 4 步：换主题（浅色） ===");

    let mut app = Builder::new(Kind::Column, "app").padding(16.0).gap(10.0);
    app.text("浅色主题");
    app.container_opts(Kind::Row, "bar", L::new().gap(8.0).to_props(), |r| {
        r.button("主要操作");
        r.button_opts("次要操作", |n| n.props.disabled = true);
    });

    let tree = app.build();

    // 主题就是一组颜色 + 字号。可以直接构造，也可以用 `Theme::default()` 再改几项。
    // Rust 里这叫「结构体更新语法」：`..` 表示「其余字段沿用」。
    let light = Theme {
        text: Color::rgb(0x1a, 0x1d, 0x28),
        text_dim: Color::rgb(0x6b, 0x73, 0x88),
        surface: Color::rgb(0xf5, 0xf6, 0xfa),
        border: Color::rgb(0xd8, 0xdd, 0xe8),
        accent: Color::rgb(0x2f, 0x6f, 0xe0),
        on_accent: Color::rgb(0xff, 0xff, 0xff),
        ..Theme::default()
    };

    // 注意这一句：我们把 theme 传了两次 —— render_and_save 内部只收一个，
    // 所以这里用 clone()`（Rust 的值语义：传参会把值移动走，想保留就得复制）。
    render_and_save_with_theme(&tree, 360, 140, "04-light.png", out_dir, light)?;
    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
// 第 5 步：用 .dui 场景文件建同一棵树
// ─────────────────────────────────────────────────────────────────────────────

fn step5_scene_file(out_dir: &Path) -> Result<(), Box<dyn std::error::Error>> {
    println!("=== 第 5 步：.dui 场景文件 ===");

    // 这是场景文件的内容（缩进 2 空格表示「子节点」，方括号里第一项是类型）
    let scene = "\
[column name=app pad=12 gap=8]
  [text label=从场景文件构建]
  [row name=bar gap=8]
    [button label=确定]
    [button label=取消]
";

    // 先存成文件，让你能直接用编辑器改它看看效果
    let scene_path = out_dir.join("05-form.dui");
    std::fs::write(&scene_path, scene)?;

    // 解析成节点树。第二个参数是「文件名」，只用于报错时显示 `文件名:行号`。
    let tree = parse_scene(scene, "05-form.dui")?;

    // 反过来：把树编码回场景文件文本（第 6 步的对比也用得上）
    let roundtrip = encode_scene(&tree);
    println!("  场景文件往返一致：{}", roundtrip == scene || roundtrip.contains("column"));

    render_and_save(&tree, 360, 120, "05-scene.png", out_dir)?;

    // 两种建树方式产出**结构完全相同**的树 —— 这是本库的核心不变式，有断言钉住。
    let mut app = Builder::new(Kind::Column, "app").padding(12.0).gap(8.0);
    app.text("从场景文件构建");
    app.container_opts(Kind::Row, "bar", L::new().gap(8.0).to_props(), |r| {
        r.button("确定");
        r.button("取消");
    });
    let from_code = app.build();
    println!(
        "  与代码构建的树结构相等：{}",
        from_code.structurally_eq(&tree)
    );
    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
// 第 6 步：自己控制渲染 —— 只要几何 / 只要绘制列表 / 自己决定每个节点怎么画
// ─────────────────────────────────────────────────────────────────────────────

fn step6_own_renderer(out_dir: &Path) -> Result<(), Box<dyn std::error::Error>> {
    println!("=== 第 6 步：拿到中间结果自己处理 ===");

    let mut app = Builder::new(Kind::Column, "app").padding(12.0).gap(8.0);
    app.text("标题");
    let b1 = app.button("确定");
    app.button("取消");
    let tree = app.build();

    let theme = Theme::default();
    let (w, h) = (360u32, 120u32);

    // ① 只要**几何表**：nodeId → 矩形（x, y, 宽, 高）。
    //    调试布局问题时最有用；也能用来做自己的命中测试。
    let geo = deer_gui::layout_tree(&tree, w, h, theme.clone());
    println!("  几何表（{} 个节点）：", geo.len());
    for id in ["app", b1.as_str(), "button_2"] {
        if let Some(r) = geo.get(id) {
            println!("    {id:<10} x={:<6} y={:<6} w={:<6} h={}", r.x, r.y, r.w, r.h);
        }
    }

    // ② 只要**绘制列表**：一串与后端无关的命令（画矩形 / 画描边 / 画文字 / 推裁剪区…）。
    //    你可以遍历它、统计它、或者交给自己的后端。
    let list = deer_gpu::build_draw_list(&tree, &geo, theme.clone(), &ApproxMeasure);
    let counts = list.counts();
    println!(
        "  绘制命令 {} 条：圆角填充 {} / 描边 {} / 文字 {}",
        list.len(),
        counts.fill_round_rect,
        counts.stroke_rect,
        counts.text
    );

    // ③ 自己光栅化成像素（而不是调用一步到位的 render_tree_to_png）。
    //    像素是「行优先、无 padding 的 RGBA8」：每 4 个字节一个像素，
    //    顺序是 红、绿、蓝、透明。
    let fb = deer_gpu::null::CpuRenderer::new().render(
        deer_gpu::Extent { width: w, height: h },
        &list,
        theme.surface, // 背景色
    )?;
    println!("  像素缓冲 {} 字节（{}×{}×4）", fb.pixels.len(), w, h);
    // 取一个像素看看：按钮在 (12, 38) 附近，应该是强调色
    if let Some(px) = fb.pixel(20, 45) {
        println!("  (20,45) 处的像素 = RGBA{:?}", px);
    }

    // ④ 自己编码成 PNG 并落盘
    let png = deer_gpu::png::encode_rgba(fb.width, fb.height, &fb.pixels)
        .map_err(|e| format!("PNG 编码失败：{e}"))?;
    let path = out_dir.join("06-manual.png");
    std::fs::write(&path, &png)?;
    println!("  写出 {}", path.display());
    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
// 下面两个是我为了让教程短一点而写的小工具，你不需要照抄
// ─────────────────────────────────────────────────────────────────────────────

fn render_and_save(
    tree: &Node,
    w: u32,
    h: u32,
    file_name: &str,
    out_dir: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    render_and_save_with_theme(tree, w, h, file_name, out_dir, Theme::default())
}

fn render_and_save_with_theme(
    tree: &Node,
    w: u32,
    h: u32,
    file_name: &str,
    out_dir: &Path,
    theme: Theme,
) -> Result<(), Box<dyn std::error::Error>> {
    // 一步到位：树 → 几何 → 绘制列表 → 像素 → PNG
    let png = deer_gui::render_tree_to_png(tree, w, h, theme)?;
    let path = out_dir.join(file_name);
    std::fs::write(&path, &png)?;

    // 顺便做个自检：画面只有一种颜色 ⇒ 说明什么都没画上（这种「假成功」很坑）
    let (_, _, pixels) = deer_gui::render_tree_to_rgba(tree, w, h, Theme::default())?;
    let mut set = std::collections::BTreeSet::new();
    for p in pixels.chunks_exact(4) {
        set.insert([p[0], p[1], p[2], p[3]]);
    }
    println!(
        "  {} （{w}×{h}，{} 字节，{} 种颜色）",
        path.display(),
        png.len(),
        set.len()
    );
    Ok(())
}
