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
| [11](#11-真实文字) | 真实文字 | 一张**真字形**的图 |
| [12](#12-在窗口里看到画面) | 在窗口里看到画面 | 一个**真窗口** |
| [13](#13-用-gpu-画界面) | 用 GPU 画界面 | 一张 **GPU 画的**界面图 |

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

> 这里的 `ApproxMeasure` 是「每字符 0.6em」的**占位度量**（确定性，便于断言）。
> 要真实字体度量与真实字形，看第 11 章：`build_draw_list` 的最后一个参数换成 `engine.measure()`。

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

**能做的**：描述界面 → 算布局 → 离屏渲染成 PNG / 像素；在 Windows 上还能**开真窗口**，
用 GPU 把画面呈现上去（M2b，见第 12 章）。
**不能做的**（别误以为能）：

| 想做的事 | 现状 |
|---|---|
| 界面显示在**窗口**里 | ✅ **能开窗**（M2b，**仅 Windows**）：窗口里是 GPU 清屏色 + 几何；**界面（控件/文字）上屏是 M3** |
| **GPU 渲染**出图 | ✅ 离屏 Vulkan 已画出正确像素（M2a-6 修复了 SPIR-V 段序缺陷）；✅ 窗口呈现已打通（M2b）；❌ **消费 `DrawList`（界面）是 M3** |
| 图片里的字是**真字体** | ✅ **离屏**已支持（第 11 章）；**GPU 侧文本**仍 ❌（M3） |
| **鼠标点击 / 键盘输入** | ❌ M5（`hit_test` 有了，但没有事件派发） |
| **Tab 焦点** / 方向键导航 | ❌ M5 |
| **可停靠面板 dock** | ❌ M5 |
| 12 个 `deer-ui` 控件的语义 | ❌ M6（现在只有 5 种节点） |

完整清单与每个功能的边界：[`../FEATURES.md`](../FEATURES.md)。
里程碑与顺序：[`../ROADMAP.md`](../ROADMAP.md)。

---

## 11. 真实文字

**目标**：让图里的字是**真字形**（不再是等宽方块占位）。

```rust
use deer_gui::prelude::*;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut app = Builder::new(Kind::Column, "app").padding(12.0).gap(8.0);
    app.text("Hello 你好");
    app.button("确定");
    let tree = app.build();

    // 字体：consola → arial → segoeui；找不到返回 None（不静默降级）
    let font = deer_gpu::measure::find_system_font().ok_or("找不到系统字体")?;

    // 带字体的渲染：布局、绘制列表、光栅化用**同一个字号、同一个度量**
    let png = deer_gui::render_tree_to_png_with_font(&tree, 360, 140, Theme::default(), &font, 16.0)?;
    std::fs::write("render_out/text.png", png)?;
    Ok(())
}
```

> 对应示例：`cargo run -p deer-gui --example text_render`
> （写真 `render_out/text_render.png` 与 `render_out/text_render_atlas.png` 字形图集，
> 并断言墨迹像素、抗锯齿灰阶、确定性、以及「带字库与不带字库必须画出不同画面」）

**这一章的关键概念**：
- 不带字体的 `render_tree_to_png` 仍然用 `ApproxMeasure` + 占位方块 —— 这是**像素级可区分**的；
  想确认真实字形那条路真的生效，就让两条路径各出一张图对比。
- 最后那个参数 `16.0` 是**唯一**的字号来源：它同时决定 `TextEngine` 的字号和布局的
  `TextStyle.font_size`，并**覆盖** `theme.font_size`。和第 4 章的提醒是一回事：**字号参与布局**。
- **铁律**：布局、绘制列表、光栅化必须用同一个字号、同一个度量。违反的典型症状是
  「文字和它的盒子错位、换行位置对不上」，而图看起来「差不多」，很难靠眼睛抓。
- `.dui` 场景文件同样可用：字体是**渲染期**的事，与怎么建树无关
  （`parse_scene` 出来的树直接传给 `render_tree_to_png_with_font`）。
- 想复用同一个字库（例如 `TextEngine::from_system_font(16.0)?`）连渲染多张图，
  用 `render_tree_to_rgba_with_engine(&tree, w, h, theme, font_size, engine)`。
- 想深入底层（自己把字形打进图集）：`cargo run -p deer-gui --example glyph_atlas`，
  配合 [`features/glyph-raster.md`](features/glyph-raster.md) 与 [`features/glyph-atlas.md`](features/glyph-atlas.md)。

**仍然做不到**：GPU 侧文本（M3）、**窗口里显示界面**（M3：窗口能开，但里面还没有控件/文字）、
子像素定位、多字体回退、富文本/图标字体、hinting、CFF 字体。完整边界见
[`features/text-rendering.md`](features/text-rendering.md) 第 6 节。

---

## 12. 在窗口里看到画面

**目标**：弹出一个**真窗口**，用 GPU 把画面呈现上去（M2b）。

```powershell
cargo run -p deer-gui --features window --example window_preview
```

**你会看到**：弹出一个窗口（标题带 `deer-gui` 前缀），里面是 GPU 画的清屏色 + 一块几何三角形，
连续呈现若干帧后**自动退出**（退出码 0），终端里打印适配器 / 交换链格式 / present mode / 图像数 / 帧数。

| 环境变量 | 作用 |
|---|---|
| `DEER_WINDOW_FRAMES` | 呈现多少帧后退出（示例默认 **120**） |
| `DEER_WINDOW_HOLD` | 设为 `1`（或 `true`）时**不自动退出**，一直开着直到你手动关窗口 |
| `DEER_WINDOW_ADAPTER` | 用第几张显卡（默认 `0`；示例会打印实际适配器名） |
| `DEER_WINDOW_READBACK` | 设为 `0` 关掉第一帧的像素回读（自检显式降级，不再给像素级证据） |
| `DEER_VK_VALIDATION` | 设为 `1` 打开 Vulkan 校验层（诊断用） |

```powershell
# 只看 10 帧（适合快速自检）
$env:DEER_WINDOW_FRAMES='10'; cargo run -p deer-gui --features window --example window_preview
# 想慢慢看：窗口会一直开着，点右上角 × 关闭
$env:DEER_WINDOW_HOLD='1'; cargo run -p deer-gui --features window --example window_preview
```

**这一章的关键概念**：
- **`--features window` 不能省**：窗口层是可选的 `deer-window`（winit），不开 feature 时
  `deer-gui` 仍然零第三方依赖。示例是 `required-features = ["window"]` 的，漏了会报
  `target ... requires the features: window`。
- **事件循环必须在主线程**：窗口与事件循环由 `deer_window::run(config, app)` 管，
  你实现 `App`（`init` 建渲染器 / `redraw` 画一帧 / 可选 `resized`）。
- **窗口里还不是界面**：M2b 只保证「GPU 画的像素能出现在窗口上」——现在显示的是清屏色 + M2a
  验证过的几何。**M3a 已能把非文本界面几何画到 GPU（离屏，见第 13 章）**，但把它呈到窗口里是 **M3c**、
  文本上 GPU 是 **M3b**。
- **交换链会过期**：`render_and_present()` 返回 `OutOfDate` 时**必须 resize 后重试**，不许当成功。
- **第一帧会回读像素核对**（`read_back_last_frame()`）：终端里能看到
  `像素回读 : 四角 [71, 79, 105, 255] = sRGB 编码后的清屏色 rgb(0x10,0x14,0x24)` ——
  注意**不是** `[16,20,36]`（sRGB 附件的驱动编码）；回读**强制一次 GPU→CPU 同步**，所以只在第一帧做一次。
- 目前**只有 Windows** 实现了窗口句柄的填充；非 Windows 会明确返回 `Err`。

**仍然做不到**：鼠标/键盘输入（M5）、窗口里的界面（M3c）、文本上 GPU（M3b）、
多窗口 / 全屏 / HDR / 帧率上限。
完整边界见 [`features/window.md`](features/window.md) 与
[`features/vulkan-swapchain.md`](features/vulkan-swapchain.md) 第 6 节。

---

## 13. 用 GPU 画界面

**目标**：让 **Vulkan** 画出界面树的几何（**不含文本**），并且与 CPU 参考实现**逐像素一致**（M3a）。

```powershell
cargo run -p deer-gui --example gpu_geometry
```

**你会看到**（本机实测）：

```text
画布        : 320×200
绘制命令    : 14 条（填充 0 / 圆角 7 / 描边 7 / 裁剪 0 对 / 文本 0）
顶点数      : 210（每帧一个顶点缓冲、一次 draw）
非清屏色像素: 3152 / 64000
最大通道差  : 0（要求 0，逐字节相同）
额外场景    : 裁剪 + 嵌套裁剪 + 粗描边（带宽 > 边长）→ 最大通道差 0
```

产物：`render_out/gpu_geometry.png`（GPU 回读）与 `render_out/gpu_geometry_cpu.png`（CPU 基准）——
两张图逐字节相同（可以直接用看图工具对比）。

```rust
// 片段（放在返回 Result<(), String> 的函数里）：extent / theme / list 见上一段
use deer_gui::prelude::*;
use deer_gui::vk::GpuGeometryRenderer;

// 同一份 DrawList：GPU 画一遍，CPU 画一遍，再逐字节比
let mut gpu = GpuGeometryRenderer::new(0, extent, theme.surface).map_err(|e| e.to_string())?;
let gpu_px = gpu.render(&list).map_err(|e| e.to_string())?;
let mut cpu_renderer = CpuRenderer::new();
let cpu = cpu_renderer
    .render(extent, &list, theme.surface)
    .map_err(|e| e.to_string())?;
assert_eq!(gpu_px, cpu.pixels, "不透明几何必须逐字节相同");
```

**这一章的关键概念**：
- **这条链不含文本**：`DrawCmd::Text` 目前明确 `Unsupported`（文本上 GPU 是 **M3b**）。
  示例的树只用带 `pad` 的容器 —— 它们只产生 `FillRoundRect` / `StrokeRect`。
- **硬判据是与 CPU 逐字节相同**：不透明几何最大通道差 **0**；半透明场景实测最大差 **1 LSB**
  （**实测上限，不是证明上界**），全量对照见 `crates/deer-vk/tests/gpu_vs_cpu.rs`。
- **静态 viewport/scissor**：动态版在本机 Intel 核显上**画不出任何像素**（M2a 实测），
  所以管线把 viewport/scissor 写死；**换画布尺寸要新建渲染器**。
- **颜色附件是线性 `R8G8B8A8_UNORM`**（不是 `_SRGB`）：CPU 基准不做 gamma，用 SRGB 会系统性偏差。
- **`DEER_VK_VALIDATION=1` 下 parity 零校验消息是「可回归断言」**：「层确实在跑」与「消息为零」
  都有断言（`validation_layer_state_matches_the_request` + `ffi::validation_message_count()` 的三处
   `assert_no_validation_messages`）。但**它不能证明内存域依赖正确**：VVL 不做通用同步验证，
   删掉 host→vertex 屏障它也不报错（那条由 `host_to_vertex_barrier_is_emitted_once_per_non_empty_frame` 守）。
- 当前**每帧一个顶点缓冲、一次 draw**（没有批处理优化），属性能项、不影响正确性。

**仍然做不到**：文本/字形（M3b）、纹理、窗口里显示界面（M3c）、批处理优化、sRGB/色彩管理、MSAA。
完整边界见 [`features/gpu-geometry.md`](features/gpu-geometry.md) 第 6 节。

---

## 附：可运行示例一览

| 示例 | 命令 | 你会看到 |
|---|---|---|
| 分步教程 | `cargo run -p deer-gui --example tutorial` | `render_out/01-hello.png` … `render_out/06-manual.png`（共 6 张） |
| 最小出图 | `cargo run -p deer-gui --example render_to_png` | 一张完整界面图 |
| 场景文件 | `cargo run -p deer-gui --example scene_file` | 设置面板 + 可编辑的 `.dui` |
| 只看布局 | `cargo run -p deer-gui --example geometry` | 几何表 + 命中结果 |
| 换主题 | `cargo run -p deer-gui --example theme` | 深/浅/暖三张图 |
| 绘制命令 | `cargo run -p deer-gui --example draw_list` | 命令清单 + 统计 |
| 原始像素 | `cargo run -p deer-gui --example pixels` | PNG + PPM |
| 真实文字 | `cargo run -p deer-gui --example text_render` | 真字形界面图 + 度量/像素自检 |
| 字形光栅化 + 图集 | `cargo run -p deer-gui --example glyph_atlas` | 图集 PNG + 覆盖率/利用率统计 |
| 真窗口预览 | `cargo run -p deer-gui --features window --example window_preview` | 一个真窗口（GPU 清屏色 + 几何）+ 帧数统计 |
| GPU 画界面（非文本） | `cargo run -p deer-gui --example gpu_geometry` | GPU 出的界面图 + 与 CPU 逐字节对照 |
| Vulkan 现状 | `cargo run -p deer-gui --example vulkan_devices` | 本机 GPU + 着色器验收 |
