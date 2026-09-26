# deer-gui 教程

> 从零开始的**一步一步**教程。每章都能独立运行，命令都实际跑过。
>
> - 想知道**有哪些功能**：[`../FEATURES.md`](../FEATURES.md)
> - 想看**某个功能的完整用法**：[`features/`](features/)
> - 第一次用 Rust：[`GETTING-STARTED.md`](GETTING-STARTED.md)（更啰嗦的版本）

## 目录

| 章 | 内容 | 你会做出 |
|---|---|---|
| [0](#0-先跑起来) | 先跑起来 | 6 张示例图 |
| [1](#1-第一张图) | 第一张图 | 一个按钮 |
| [2](#2-横排与间距) | 横排与间距 | 一行按钮 |
| [3](#3-容器嵌套与禁用态) | 容器嵌套与禁用态 | 一个表单 |
| [4](#4-换主题) | 换主题 | 浅色 / 暖色配色 |
| [5](#5-用-dui-文件写界面) | 用 `.dui` 文件写界面 | 可手改的设置面板 |
| [6](#6-只看布局不出图) | 只看布局不出图 | 一张几何表 |
| [7](#7-拿到绘制命令) | 拿到绘制命令 | 一份命令清单 |
| [8](#8-拿原始像素) | 拿原始像素 | 自己读/写像素 |
| [9](#9-建你自己的项目) | 建你自己的项目 | 独立工程 |
| [10](#10-现状与边界) | 现状与边界 | 知道什么还不能做 |

---

## 0. 先跑起来

```powershell
cd Z:\deer-gui
cargo run -p deer-gui --example tutorial
```

浏览器里打开 `Z:\deer-gui\render_out\`，你会看到 6 张图。**先看一眼 `03-form.png`**，
那是本教程第 3 章的成果。

> 每个示例都可以单独跑，产物统一在 `render_out/`。列出全部：
> `cargo run -p deer-gui --example <名字>`，名字见 [`../FEATURES.md`](../FEATURES.md)。

---

## 1. 第一张图

**目标**：画出一个按钮。

```rust
use deer_gui::prelude::*;
use deer_gui::render_tree_to_png;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // ① 建树：根是「竖排容器」，四周留 12 像素
    //    注意 .padding(12.0) 必须**链式**写 —— 它按值接收 self，会移动 app
    let mut app = Builder::new(Kind::Column, "app").padding(12.0);

    // ② 加一个按钮
    app.button("点我");

    // ③ 定稿成只读的树
    let tree = app.build();

    // ④ 渲染：宽 360、高 120、默认主题（深色）
    let png = render_tree_to_png(&tree, 360, 120, Theme::default())?;
    std::fs::write("render_out/01.png", png)?;
    println!("写出 render_out/01.png");
    Ok(())
}
```

放进 `main.rs` 后 `cargo run`，打开 `render_out/01.png`：**深色背景上一个蓝色圆角按钮**。

**这一章的关键概念**：
- `Kind::Column` = 竖排容器（`Kind::Row` 是横排）
- 第一个参数 `"app"` 是**节点 id** —— 后面查几何、接事件都靠它
- `.0` 不能省：Rust 里 `12` 是整数、`12.0` 才是小数

---

## 2. 横排与间距

**目标**：一行放三个按钮，彼此隔 8 像素。

```rust
let mut app = Builder::new(Kind::Column, "app").padding(12.0).gap(8.0);
app.text("按钮排成一行");

// container_opts(容器类型, 名字, 布局参数, 闭包)
// 闭包参数 `r` 就是「往这个容器里加东西」的句柄 —— 一定用它，别用外面的 app
app.container_opts(Kind::Row, "bar", L::new().gap(8.0).to_props(), |r| {
    r.button("确定");
    r.button("取消");
    r.button("稍后");
});

let tree = app.build();
```

**这一章的关键概念**：
- `.gap(8.0)` = 子元素间距（在根上写就是根的子元素之间）
- `L::new().gap(8.0).to_props()` —— `L` 是布局参数构造器，`.to_props()` 是必须的收尾
- `container_opts` 的闭包**决定层级**：闭包里加的东西是那个容器的子节点

> 对应示例：`cargo run -p deer-gui --example tutorial`（第 2 步）

---

## 3. 容器嵌套与禁用态

**目标**：一个带边框面板的表单。

```rust
let mut app = Builder::new(Kind::Column, "app").padding(12.0).gap(8.0);
app.text("新建项目");

// 带内边距的面板 —— **只有给了 pad 的容器才画底色和边框**
app.container_opts(Kind::Column, "panel", L::new().pad(10.0).gap(6.0).to_props(), |p| {
    p.text("名称");
    p.field("请输入项目名");                 // 输入框

    p.text("操作");
    p.container_opts(Kind::Row, "actions", L::new().gap(8.0).to_props(), |r| {
        r.button("保存");
        r.button("另存为");

        // button_opts：需要改按钮属性时用。闭包 `n` 拿到的是**节点本身**
        r.button_opts("删除", |n| {
            n.props.disabled = true;
        });
    });
});

let tree = app.build();
```

产物：深色面板上有三段文字、一个输入框、两个蓝色按钮、**一个灰色禁用按钮**。

**这一章的关键概念**：
- 容器可以任意嵌套（`p` → `r` 就是两层）
- **禁用按钮是灰的**（用 `border` 色，不是强调色）—— 这是渲染器的既定行为，有测试钉住
- 只有带 `pad` 的容器才画底色，否则整屏都是方块、看不出层次

---

## 4. 换主题

**目标**：同一棵界面树，换两套配色。

```rust
let light = Theme {
    text: Color::rgb(0x1a, 0x1d, 0x28),
    text_dim: Color::rgb(0x6b, 0x73, 0x88),
    surface: Color::rgb(0xf5, 0xf6, 0xfa),   // 面板底色，也是画布背景
    border: Color::rgb(0xd8, 0xdd, 0xe8),    // 边框 + 禁用按钮底色
    accent: Color::rgb(0x2f, 0x6f, 0xe0),    // 按钮底色
    on_accent: Color::rgb(0xff, 0xff, 0xff), // 按钮文字
    ..Theme::default()                        // 其余字段不变（字号等）
};

let png = render_tree_to_png(&tree, 300, 200, light)?;
```

`..Theme::default()` 是 Rust 的「结构体更新语法」：**只改列出的字段，其余沿用默认**。

**常见陷阱**：`font_size` 参与文本尺寸计算，所以**换字号会改变布局**——
如果你在断言具体几何，改字号后要重算期望值。

> 对应示例：`cargo run -p deer-gui --example theme`（一次出深/浅/暖三张）

---

## 5. 用 `.dui` 文件写界面

**目标**：把界面写进文本文件，改一个数字就能重跑看效果。

文件 `settings.dui`：

```
# 注释用 # 开头
[column name=app pad=14 gap=8]
  [text label=设置]
  [column name=card pad=10 gap=6]
    [text label=外观]
    [row name=themeRow gap=6]
      [button label=浅色]
      [button label=深色]
  [row name=actions gap=8 main=end]
    [button label=取消]
    [button label=保存]
```

代码：

```rust
let text = std::fs::read_to_string("render_out/settings.dui")?;
let tree = parse_scene(&text, "settings.dui")?;   // 第二个参数用于报错显示文件名
let png = render_tree_to_png(&tree, 300, 260, Theme::default())?;
```

**语法（四条）**：

| 规则 | 说明 |
|---|---|
| `[类型 属性…]` | 类型：`column` / `row` / `text` / `button` / `field` |
| 缩进 2 空格 | 表示「子节点」，必须是 2 的倍数 |
| `key=value` | 值含空格就加引号：`label="确定 或 取消"` |
| `#` 开头 | 注释 |

**属性**：`name` `label` `disabled` `pad` `gap` `w` `h` `grow` `main` `cross`

**写错会报错并给行号**（不静默忽略）：

```
bad.dui:2: 未知属性 "nope"（可用：name/w/h/pad/gap/main/cross/grow/label/disabled）
```

> 对应示例：`cargo run -p deer-gui --example scene_file`

**两条路径等价**：同一份界面，用代码写和用 `.dui` 写，产出的树**结构完全相等**
（`from_code.structurally_eq(&from_file)` 为真）。这是本库的核心不变式。

---

## 6. 只看布局，不出图

**目标**：界面的每个控件到底被算到哪个矩形。

```rust
let geo = deer_gui::layout_tree(&tree, 360, 200, Theme::default());

println!("{:<12} {:>7} {:>7} {:>7} {:>7}", "节点", "x", "y", "宽", "高");
for (id, r) in &geo {
    println!("{id:<12} {:>7.1} {:>7.1} {:>7.1} {:>7.1}", r.x, r.y, r.w, r.h);
}
```

**调界面尺寸时这是最有用的工具** —— 比改代码猜快得多。定位到具体哪一层被压扁了，
问题就明确了。

**顺带能做命中测试**（坐标 → 哪个控件）：

```rust
let btn = geo["button_1"];
let hit = hit_test(&tree, &geo, btn.x + 2.0, btn.y + 2.0);
assert_eq!(hit.map(|n| n.id.as_str()), Some("button_1"));
```

> 对应示例：`cargo run -p deer-gui --example geometry`

---

## 7. 拿到绘制命令

**目标**：看「布局」和「像素」中间那层到底是什么。

```rust
let list = deer_gpu::build_draw_list(&tree, &geo, theme.clone(), &ApproxMeasure);

for cmd in &list.cmds {
    match cmd {
        DrawCmd::FillRoundRect { rect, .. } => println!("圆角块 ({},{}) {}×{}", rect.x, rect.y, rect.w, rect.h),
        DrawCmd::Text { text, .. } => println!("文字 \"{text}\""),
        _ => {}
    }
}

let counts = list.counts();       // 按类型统计 —— 性能预算的抓手
assert!(list.clip_balanced());    // 裁剪栈必须平衡
```

**为什么这层值得单独看**：
- 它是**与后端无关**的中间表示 —— CPU 和 Vulkan 后端都只消费它；
- 你能在**不渲染**的情况下断言绘制行为（例如「禁用按钮必须用 border 色」）；
- 命令数就是性能预算。

> 对应示例：`cargo run -p deer-gui --example draw_list`

---

## 8. 拿原始像素

**目标**：不用 PNG，直接操作 RGBA8 缓冲。

```rust
let (w, h, pixels) = deer_gui::render_tree_to_rgba(&tree, 120, 80, Theme::default())?;

// **行优先、无 padding**，每 4 字节一个像素，顺序 R G B A
// 第 (x, y) 个像素从 pixels[(y * w + x) * 4] 开始
let i = ((10 * w + 20) * 4) as usize;
println!("(20,10) = R{} G{} B{} A{}", pixels[i], pixels[i+1], pixels[i+2], pixels[i+3]);

// 自己编码成 PNG（库里的零依赖编码器）
let png = deer_gpu::png::encode_rgba(w, h, &pixels).map_err(|e| e.to_string())?;
std::fs::write("render_out/pixels.png", png)?;
```

> 对应示例：`cargo run -p deer-gui --example pixels`（还会写一个 `.ppm` 便于交叉验证）

---

## 9. 建你自己的项目

```powershell
cd Z:\
cargo new my-ui
cd my-ui
```

`Cargo.toml` 整个换成：

```toml
[package]
name = "my-ui"
version = "0.1.0"
edition = "2024"

[dependencies]
deer-gui = { path = "Z:/deer-gui/crates/deer-gui" }
```

`src/main.rs` 用第 1 章的代码，然后 `cargo run`。

**仓库里有一个现成的例子**：`Z:\deer-gui\test_project\deer-hello`
—— 它就是这样建的（独立于 workspace），可以直接 `cd` 进去 `cargo run`。

---

## 10. 现状与边界

**能做的**：描述界面 → 算布局 → 离屏渲染成 PNG / 像素。
**不能做的**（别误以为能）：

| 想做的事 | 现状 |
|---|---|
| 界面显示在**窗口**里 | ❌ M2b（要先定窗口方案） |
| **GPU 渲染**出图 | ❌ M2a-3..6（Vulkan 只到「设备 + 着色器」） |
| 图片里的字是**真字体** | ❌ M4（现在是等宽方块占位） |
| **鼠标点击 / 键盘输入** | ❌ M5（`hit_test` 有了，但没有事件派发） |
| **Tab 焦点** / 方向键导航 | ❌ M5 |
| **可停靠面板 dock** | ❌ M5 |
| 12 个 `deer-ui` 控件的语义 | ❌ M6（现在只有 5 种节点） |

完整清单与每个功能的边界：[`../FEATURES.md`](../FEATURES.md)。
里程碑与顺序：[`../ROADMAP.md`](../ROADMAP.md)。

---

## 附：可运行示例一览

| 示例 | 命令 | 你会看到 |
|---|---|---|
| 分步教程 | `cargo run -p deer-gui --example tutorial` | `render_out/01..06.png` |
| 最小出图 | `cargo run -p deer-gui --example render_to_png` | 一张完整界面图 |
| 场景文件 | `cargo run -p deer-gui --example scene_file` | 设置面板 + 可编辑的 `.dui` |
| 只看布局 | `cargo run -p deer-gui --example geometry` | 几何表 + 命中结果 |
| 换主题 | `cargo run -p deer-gui --example theme` | 深/浅/暖三张图 |
| 绘制命令 | `cargo run -p deer-gui --example draw_list` | 命令清单 + 统计 |
| 原始像素 | `cargo run -p deer-gui --example pixels` | PNG + PPM |
| Vulkan 现状 | `cargo run -p deer-gui --example vulkan_devices` | 本机 GPU + 着色器验收 |
