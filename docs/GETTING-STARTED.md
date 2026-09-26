# 上手指南（假设你从没写过 Rust）

> 本指南的每条命令都**实际跑过**。你按顺序做就行，遇到不对的看 §9 排查。

---

## 1. 一句话：这个库是干什么的

**用代码描述界面（哪些控件、怎么排），它算出每个控件的位置，然后画成图片。**

现在的定位是「**离屏渲染**」：输入一棵界面树，输出一张 PNG 图片或一块像素缓冲。
**还不能开窗口显示**（那是里程碑 M2–M3）。

---

## 2. 先搞清楚：现在能做什么、不能做什么

| 你想做的事 | 现在行不行 |
|---|---|
| 描述界面、算出布局、导出图片 | ✅ **行** |
| 把界面渲染到窗口里显示、用鼠标点 | ❌ 不行（M2–M5） |
| 图片上的字是真的字体 | ✅ **可以**（离屏 CPU）：带字体入口见[第 11 章](TUTORIAL.md#11-真实文字) / [`features/text-rendering.md`](features/text-rendering.md)。**不带字体的老入口仍是方块占位** |

**所以它现在适合**：验证布局、生成界面设计稿/示意图、给文档配图、做布局算法的实验。
**不适合**：做一个真正能用的桌面软件（那要等窗口和输入做完）。

---

## 3. 30 秒看到第一个结果（推荐从这里开始）

打开 PowerShell，逐行执行：

```powershell
cd Z:\deer-gui
cargo run -p deer-gui --example tutorial
```

**你会看到**（大意）：

```
输出目录：render_out
=== 第 1 步：最小界面 ===
  render_out\01-hello.png （360×120，172998 字节，5 种颜色）
=== 第 2 步：横排 + 间距 ===
  render_out\02-row.png （360×120，172998 字节，6 种颜色）
...
```

**然后打开** `Z:\deer-gui\render_out\` 文件夹，双击 `03-form.png` 看效果。

这个 `tutorial` 示例本身就是一个**可以按顺序读的教程**，源码在
[`crates/deer-gui/examples/tutorial.rs`](../crates/deer-gui/examples/tutorial.rs)，
每一步都写了注释解释「为什么这么写」。

> 第一次跑会编译，可能要 1–3 分钟（之后是秒级，因为编译结果被缓存了）。

---

## 4. 建你自己的项目（独立于这个仓库）

### 4.1 建项目

```powershell
cd Z:\                      # 或者任何你放代码的地方
cargo new deer-hello
cd deer-hello
```

这会生成两个文件：`Cargo.toml`（项目配置）和 `src\main.rs`（源码）。

### 4.2 告诉它「deer-gui 在哪」

把 `Cargo.toml` **整个内容**替换成：

```toml
[package]
name = "deer-hello"
version = "0.1.0"
edition = "2024"

[dependencies]
deer-gui = { path = "Z:/deer-gui/crates/deer-gui" }
```

关键就是最后一行：`path` 指向本地那份库。
（注意路径用**正斜杠 `/`**，Windows 反斜杠在 TOML 里是转义字符。）

### 4.3 写第一个程序

把 `src\main.rs` **整个内容**替换成：

```rust
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
```

### 4.4 跑

```powershell
cargo run
```

**你会看到**：`写出 hello.png（179418 字节）`，并且当前目录多出一个 `hello.png`。

打开它，应该是一个深色面板，里面有「Hello deer-gui」的文字块、两个按钮（一蓝一灰）：

![第一个程序的结果](images/hello.png)

---

## 5. 核心心智模型：三步走

整个库就三件事，记住这个顺序就不会乱：

```
   ① 建树              ② 算几何            ③ 渲染
描述界面有哪些控件  →  算出每个控件的矩形  →  变成命令 → 像素 → 图片
```

| 步骤 | 你写的代码 | 发生了什么 |
|---|---|---|
| ① 建树 | `Builder` + `.text()` / `.button()` / `.container_opts()` | 得到一个 `Node` 树（纯数据） |
| ② 算几何 | `render_tree_to_*` 内部自动做；想自己看就用 `deer_gui::layout_tree()` | nodeId → 矩形 `(x, y, 宽, 高)` |
| ③ 渲染 | `render_tree_to_png()` / `render_tree_to_rgba()` | 绘制命令 → 像素 |

**为什么这样设计**：① 和 ③ 之间是纯数学，所以布局能被测试钉死（断言清单见 `cargo test --workspace` 输出），
也让「同一棵界面树换一个渲染后端」变得可能（CPU 后端已有，Vulkan 在做）。

---

## 6. 三种用法，按需要选

### 6.1 最简单：一步出图

```rust
use deer_gui::prelude::*;
use deer_gui::render_tree_to_png;

let tree = /* 你的树 */;
let png: Vec<u8> = render_tree_to_png(&tree, 320, 200, Theme::default())?;
std::fs::write("ui.png", png)?;
```

### 6.2 要像素自己处理

```rust
use deer_gui::prelude::*;

let (w, h, pixels) = deer_gui::render_tree_to_rgba(&tree, 320, 200, Theme::default())?;
// pixels 是「行优先、无 padding 的 RGBA8」：每 4 字节一个像素，顺序 红绿蓝透明
// 第 (x, y) 个像素从 pixels[(y * w + x) * 4] 开始
let idx = ((10 * w + 20) * 4) as usize;
println!("(20,10) 的颜色 = R{} G{} B{} A{}", pixels[idx], pixels[idx+1], pixels[idx+2], pixels[idx+3]);
```

### 6.3 只要布局（调界面尺寸时最有用）

```rust
use deer_gui::prelude::*;

let geo = deer_gui::layout_tree(&tree, 320, 200, Theme::default());
for (id, rect) in &geo {
    println!("{id}: x={} y={} w={} h={}", rect.x, rect.y, rect.w, rect.h);
}
```

---

## 7. 建树的两套写法（产出**同一棵树**）

### 7.1 代码里写（命令式，适合动态生成）

```rust
let mut app = Builder::new(Kind::Column, "app").padding(12.0).gap(8.0);
app.text("标题");
app.field("输入框提示");                       // 输入框
app.container_opts(Kind::Row, "bar", L::new().gap(8.0).to_props(), |r| {
    r.button("确定");                        // 普通按钮
    r.button_opts("禁用", |n| n.props.disabled = true);  // 改过属性的按钮
});
let tree = app.build();
```

| 方法 | 作用 | 返回 |
|---|---|---|
| `Builder::new(类型, 名字)` | 建根节点。类型：`Kind::Column`（竖排）/ `Kind::Row`（横排） | `Builder` |
| `.padding(12.0)` / `.gap(8.0)` | 内边距 / 子元素间距。**注意：会移动 self，必须链式写** | `Builder` |
| `.text("字")` | 加一段文字 | 它的 id (`String`) |
| `.button("字")` | 加一个按钮 | 它的 id |
| `.button_opts("字", \|n\| {…})` | 加按钮并改它的属性（闭包拿到节点本身） | 它的 id |
| `.field("提示")` | 加输入框 | 它的 id |
| `.container_opts(类型, 名字, 布局, \|r\| {…})` | 加容器，闭包里加的东西都是它的子节点 | — |
| `.container_auto(类型, \|r\| {…})` | 同上，但名字自动生成 | — |
| `.build()` | 定稿成只读的 `Node` 树 | `Node` |

### 7.2 写在 `.dui` 文件里（适合手改、像 Godot 的 `.tscn`）

```
[column name=app pad=12 gap=8]
  [text label=标题]
  [field label=请输入]
  [row name=bar gap=8]
    [button label=确定]
    [button label=取消 disabled]
```

```rust
let text = std::fs::read_to_string("ui.dui")?;
let tree = parse_scene(&text, "ui.dui")?;   // 第二个参数只用于报错时显示文件名
```

**语法规则**（很简单，就四条）：

| 规则 | 说明 |
|---|---|
| `[类型 ...]` | 一行一个节点。类型：`column` / `row` / `text` / `button` / `field` |
| 缩进 2 空格 | 表示「是上一个节点的子节点」。**必须是 2 的倍数** |
| `key=value` | 属性。值含空格就加引号：`label="确定 或 取消"` |
| `# 开头` | 注释（行首，或前面有空格） |

**可用属性**：

| 属性 | 含义 | 例子 |
|---|---|---|
| `name=` | 节点 id（不写就自动生成） | `name=bar` |
| `pad=` | 内边距 | `pad=12` |
| `gap=` | 子元素间距 | `gap=8` |
| `w=` / `h=` | 固定宽/高，可用像素或百分比 | `w=200` / `w=50%` |
| `grow=` | 剩余空间的分配权重（越大越占地方） | `grow=1` |
| `main=` / `cross=` | 主轴/交叉轴对齐：`start`/`center`/`end`/`stretch` | `main=center` |
| `label=` | 显示的文字 | `label=确定` |
| `disabled` | 裸写一个词 = 设为禁用 | `[button label=删除 disabled]` |

**写错了不会静默忽略** —— 会报带行号的错，例如：

```
bad.dui:3: 未知属性 "nope"（可用：name/w/h/pad/gap/main/cross/grow/label/disabled）
```

---

## 8. 小白最容易踩的 6 个坑

| # | 现象 | 原因与解法 |
|---|---|---|
| **1** | `use of moved value: app` | `padding()`/`gap()` 这类方法**按值接收 self**，会把 `app` 移动走。必须写成 `let mut app = Builder::new(...).padding(12.0);`，**不能分两行** |
| **2** | `expected f32, found integer` | Rust 小数必须带小数点：写 `12.0` 不是 `12` |
| **3** | 改完 `.dui` 没反应 | 要重新 `cargo run`；程序启动时读一次文件 |
| **4** | 图片全是一个颜色，什么都没画 | 常见的两个原因：① 画布太小，内容被挤到 0 尺寸；② 容器没给 `pad` ⇒ 按设计**不画底色**。用 `layout_tree()` 打印几何确认 |
| **5** | `error: linker not found` / 各种链接错误 | 缺 MSVC 工具链。装 [Visual Studio Build Tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/)（勾「C++ 生成工具」） |
| **6** | 图片里「文字」是方块 | 你用的是**不带字体**的入口（`render_tree_to_png` / `CpuRenderer::new()`），它按设计画等宽占位格。要真字形：`cargo run -p deer-gui --example text_render`，或读[第 11 章](TUTORIAL.md#11-真实文字) |

---

## 9. 排查

| 命令 | 用处 |
|---|---|
| `cargo build` | 只编译，看有没有语法/类型错误 |
| `cargo run` | 编译并运行 |
| `cargo run 2>&1 \| Select-Object -Last 30` | 只看输出末尾（报错通常在那） |
| `cargo test --workspace` | 在 `Z:\deer-gui` 里跑库自己的全部断言（数量见输出，随里程碑增长；确认库没坏） |
| `cargo test -p deer-vk -- --nocapture` | 看这台机器枚举到了哪几块 GPU |

**报错看不懂时**：Rust 的报错信息通常直接告诉你「哪一行、什么原因、怎么改」，
把那一整段贴给我（或贴到搜索引擎），比猜快得多。

---

## 10. 你迟早要改的几个东西

| 想改什么 | 改哪里 |
|---|---|
| 颜色、字号 | 构造 `Theme { … }` 传给渲染函数。字段见 [§11](#11-接口速查) |
| 画布大小 | `render_tree_to_png(&tree, 宽, 高, theme)` 的第 2、3 个参数 |
| 界面结构 | 改 `main.rs` 里的建树代码，或改 `.dui` 文件 |
| 某个控件怎么画 | 目前写死在 `crates/deer-gpu/src/render.rs` 的 `DefaultRenderer`（后续会开放自定义） |

---

## 11. 接口速查

### 渲染入口（`deer_gui::` 下）

| 函数 | 输入 | 输出 |
|---|---|---|
| `render_tree_to_png(&tree, w, h, theme)` | 树、宽、高、主题 | `Result<Vec<u8>, String>`（PNG 字节） |
| `render_tree_to_rgba(&tree, w, h, theme)` | 同上 | `Result<(u32, u32, Vec<u8>), GpuError>` |
| `layout_tree(&tree, w, h, theme)` | 同上 | `Geometry`：`nodeId → Rect` |
| `gpu::build_draw_list(&tree, &geo, theme, &measure)` | 树、几何、主题、文本度量 | `DrawList`（绘制命令） |

### `Theme` 的字段（都有默认值，用 `..Theme::default()` 只改你要改的）

```rust
Theme {
    text: Color,        // 正文颜色
    text_dim: Color,    // 次要/禁用颜色
    surface: Color,     // 面板底色（也是画布背景）
    border: Color,      // 边框与禁用按钮底色
    accent: Color,      // 强调色（按钮底色）
    on_accent: Color,   // 强调色上的文字
    font_size: f32,     // 字号
    line_height: f32,   // 行高
}
```

颜色用 `Color::rgb(红, 绿, 蓝)` 或 `Color::rgba(红, 绿, 蓝, 透明度0.0~1.0)`，取值 0–255。

### `prelude` 里有什么（`use deer_gui::prelude::*;` 之后可直接用）

**构建/布局**：`Builder`、`L`、`Kind`、`Node`、`Rect`、`Size`、`Align`、`Theme`、`Color`、`RectI`、
`Extent`、`ApproxMeasure`、`Measure`、`TextStyle`、`parse_scene`、`SceneError`、`encode_scene`、
`hit_test`、`layout`、`measure_tree`。

**渲染/绘制**：`CpuRenderer`、`Framebuffer`、`DrawCmd`、`DrawList`、`DefaultRenderer`、`build_draw_list`。

**M4 文字（真实字形）**：`TextEngine`、`GlyphPlacement`、`FontMeasure`、`Rasterizer`、
`GlyphImage`、`GlyphKey`、`GlyphAtlas`。

> 唯一的完整清单是 `crates/deer-gui/src/lib.rs` 里的 `pub mod prelude` —— 本清单与它不一致时以它为准。
> 用法见 [`TUTORIAL.md` §11](TUTORIAL.md#11-真实文字) 与 [`features/text-rendering.md`](features/text-rendering.md)。

---

## 12. 完整可跑示例

| 示例 | 命令 | 看什么 |
|---|---|---|
| 分步教程（6 步） | `cargo run -p deer-gui --example tutorial` | `render_out/*.png` |
| 最小出图 | `cargo run -p deer-gui --example render_to_png` | `render_out/render_to_png.png` |
| 真实文字（真字形） | `cargo run -p deer-gui --example text_render` | `render_out/text_render.png`（外加 `text_render_atlas.png` 字形图集） |
| 字形光栅化 + 图集 | `cargo run -p deer-gui --example glyph_atlas` | `render_out/glyph_atlas.png` |

代码分别在
[`examples/tutorial.rs`](../crates/deer-gui/examples/tutorial.rs)、
[`examples/render_to_png.rs`](../crates/deer-gui/examples/render_to_png.rs)、
[`examples/text_render.rs`](../crates/deer-gui/examples/text_render.rs) 与
[`examples/glyph_atlas.rs`](../crates/deer-gui/examples/glyph_atlas.rs)。

---

## 13. 下一步

- 想**动手**：先跑 `tutorial`，再改 `render_out/05-form.dui` 重新跑，观察布局变化。
- 想**理解原理**：读 [`M1-report.md`](M1-report.md)（布局不变量与踩过的坑）。
- 想**知道何时能开窗**：看 [`../ROADMAP.md`](../ROADMAP.md)——窗口是 M2–M3，
  真实字形**本轮已落地**（离屏 CPU，见[第 11 章](TUTORIAL.md#11-真实文字)），鼠标键盘交互是 M5。
