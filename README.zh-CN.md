# deer-gui

从零实现的 Rust GUI 运行时 —— 不依赖 web、不依赖 DOM，也不依赖 `wgpu` / `ash` / `vulkano`。

[![Rust](https://img.shields.io/badge/rust-1.85%2B-blue.svg)](#环境要求)
[![Edition](https://img.shields.io/badge/edition-2024-blue.svg)](#环境要求)
[![依赖](https://img.shields.io/badge/第三方依赖-无（winit%20除外）-green.svg)](#依赖)
[![平台](https://img.shields.io/badge/窗口层-Windows-lightgrey.svg)](#平台支持)
[![许可证](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

[English](README.md) | **中文**

以 MIT 协议开源。除窗口层 `winit` 外没有第三方依赖（该例外[已登记在路线图](ROADMAP.md)）。

---

## 这是什么

deer-gui 是一个以**节点树**为核心的 GUI 运行时：你描述一棵界面树，由本仓库自研的布局引擎算出几何，再交给可插拔的渲染后端。

两条构筑路径产出**同一棵树**：

- **命令式 API**（`Builder`，imgui 式手感）；
- **场景文件**（`.dui`，思路类似 Godot 的 `.tscn`）。

布局代数来自一个 TypeScript 验证原型（当时跑了 28 条断言，是那个原型的历史值，不是本仓库当前的测试数），控件语义来自 `deer-ui`。这**不是** web 项目的移植 —— DOM 与 CSS 被完全丢弃，窗口、输入、渲染都由本项目实现。

> [!NOTE]
> 项目仍在活跃开发中。**「有哪些功能、做到哪一步、哪些还没做」以 [`FEATURES.md`](FEATURES.md) 为唯一真相**，在假设某个功能可用之前请先读它。

## 现状

| 模块 / 能力 | 状态 |
|---|---|
| `deer-layout` —— 节点树、布局代数、命中测试、`.dui` 场景解析 | ✅ 每个布局不变式一条测试，见 `crates/deer-layout/tests/layout_invariants.rs` |
| `deer-gpu` —— GPU HAL + CPU 参考后端（软件光栅化） | ✅ 类型与契约就位；CPU 后端能把绘制列表渲成像素，并能离屏贴真实字形 |
| `deer-vk` —— Vulkan 后端（自己声明符号 + 运行时动态加载） | ✅ 真机枚举到 2 个 GPU（Intel RaptorLake / NVIDIA RTX 5070 Ti，Vulkan 1.4.341）；设备、管线、离屏回读，以及 surface / 交换链 / 呈现 |
| `deer-window` —— 窗口层（winit） | ✅ **Windows** 上可建真窗口 + 事件循环；本 workspace 唯一第三方依赖 |
| 真实字形 —— 字体解析、光栅化、图集、真实度量、换行 | 🔄 离屏 CPU 已能出真字；**GPU 侧文本**、hinting、亚像素未做 |
| 窗口里显示界面 | 🔄 窗口里目前是清屏色 + 几何，还不是一棵排好版的界面 |
| 输入事件、焦点、可停靠面板 | ⬜ 未实现（里程碑 M5/M6） |

逐功能细节见 [`FEATURES.md`](FEATURES.md)，里程碑与验收判据见 [`ROADMAP.md`](ROADMAP.md)。

## 环境要求

- **Rust 1.85 或更高**（workspace 使用 edition 2024）。
- **Windows**：窗口层以及任何要呈现到窗口的东西都需要它。其余能力（布局、CPU 光栅化、离屏出 PNG、Vulkan 离屏路径）不需要窗口。
- **不需要 Vulkan SDK。** `deer-vk` 自己声明 Vulkan 符号，运行时用 `LoadLibraryW` + `GetProcAddress` 加载 `vulkan-1.dll`，因此只链接 `kernel32`。结构体布局手写并用 `offset_of!` 断言钉住。

## 安装

这些 crate 尚未发布，直接使用 workspace：

```sh
git clone https://github.com/DeerLuuu/deer-gui.git
cd deer-gui
```

要在别的项目里使用，按路径依赖：

```toml
[dependencies]
deer-gui = { path = "path/to/deer-gui/crates/deer-gui" }
```

> [!IMPORTANT]
> 只有需要真窗口时才加 `--features window`。不加这个 feature 就不会引入 `winit`，`deer-gui` 也就没有任何第三方依赖。

## 快速开始

```sh
# 跑全部断言（数量随里程碑增长，以运行输出为准）
cargo test --workspace

# 看本机枚举到哪些 GPU
cargo test -p deer-vk -- --nocapture

# 离屏渲染一张图（CI 友好）→ render_out/render_to_png.png
cargo run -p deer-gui --example render_to_png

# 渲染真实字形 → render_out/text_render.png
cargo run -p deer-gui --example text_render

# 开一个真窗口（仅 Windows；feature 必须带上）
cargo run -p deer-gui --features window --example window_preview
```

从没写过 Rust，可以先看更啰嗦的[上手指南](docs/GETTING-STARTED.md)。

### 自己写代码渲染

```rust
use deer_gui::prelude::*;
use deer_gui::render_tree_to_png;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // ① 建树（也可用 .dui 场景文件：parse_scene(text, "ui.dui")?）
    let mut app = Builder::new(Kind::Column, "app").padding(12.0).gap(8.0);
    app.text("标题");
    app.container_opts(Kind::Row, "bar", L::new().gap(8.0).to_props(), |r| {
        r.button("确定");
        r.button("取消");
    });
    let tree = app.build();

    // ② 渲染成 PNG
    std::fs::write("ui.png", render_tree_to_png(&tree, 320, 200, Theme::default())?)?;
    Ok(())
}
```

- 要拿像素而不是文件：`render_tree_to_rgba(&tree, w, h, theme)` → `(宽, 高, RGBA8)`。
- 只想看布局：`deer_gui::layout_tree(&tree, w, h, theme)` → nodeId 到 `Rect` 的几何表。

### 现在能渲染到什么程度

| 能力 | 状态 |
|---|---|
| 建树（命令式 API / `.dui` 场景文件） | ✅ |
| 布局计算、命中测试 | ✅ |
| 树 + 几何 → 绘制列表 → **像素**（CPU 光栅化） | ✅ |
| 写 PNG 文件 | ✅ |
| **真实字形**（字体解析 → 光栅化 → 图集 → 贴像素） | ✅ 仅离屏 / CPU 后端 |
| **GPU 侧文本**（把字形图集上传给 Vulkan） | ❌ 图集已就绪，后端还没接 |
| **渲染到窗口 / 屏幕上** | 🔄 目前是清屏色 + 几何，还不是界面 |
| 输入事件与焦点 | ❌ 未实现 |

一个容易踩的点：老入口 `render_tree_to_png` 不加载字体，所以经它画出的「字」仍是等宽占位方块；真实字形来自文本引擎（`text_render`）。两条路径上，几何、层次、颜色都是真的。

## 架构

```
crates/
├── deer-layout/   语言无关核心：Node 树、布局代数、命中测试、.dui 场景解析
│                  ── 零平台依赖，可在没有 GPU 的 CI 里完整断言
├── deer-gpu/      GPU HAL：Backend/Device/Swapchain/Frame trait、DrawList、CPU 参考后端
│                  ── 加一个后端 = 实现一个 trait
├── deer-vk/       Vulkan 后端：自己声明 extern 符号 + 运行时动态加载；surface/交换链/呈现
└── deer-window/   窗口层：原生窗口 + 事件循环（winit）
                   ── 唯一引入第三方依赖的地方（winit）；
                      只把不透明的 RawWindowHandle 交给渲染层，所以换窗口实现不动渲染层
```

### 两条构筑路径，一棵树

```text
  Builder（命令式，imgui 式手感）─┐
                                  ├─→ Node 树 ─→ layout() → 几何表 ─→ 后端
  parse_scene(.dui，.tscn 式）   ─┘               └─→ hit_test() → 输入路由
```

两条路径产出**结构相等**的树（`Node::structurally_eq`），这是核心不变式，由测试 `t1_two_authoring_paths_produce_the_same_tree` 钉住。

```text
# 等价的场景文件（.dui）
[column name=app pad=12 gap=8]
  [text label=标题]
  [row name=bar gap=8]
    [button label=确定]
    [button label=取消]
```

### 布局引擎的不变式

每条都有对应测试（`crates/deer-layout/tests/layout_invariants.rs`）：

| # | 不变式 |
|---|---|
| I-1 | **纯函数**：不改输入树，只回几何表 |
| I-2 | **确定性**：同输入 ⇒ 逐位相同输出（无时间/随机/环境探测） |
| I-3 | **自底向上**：先算子节点固有尺寸，父容器再分配 |
| I-4 | **像素取整**：几何全部整数 |
| I-5 | **不假设拥有窗口**：根盒子由宿主给，根不撑满 |
| I-6 | **不越界**：结果夹在可用空间内 |
| I-7 | **分配尺寸 ≠ 可用空间**：父分配的主轴尺寸必须被采信；百分比相对父内容盒解析 |
| I-8 | **主轴 = sum(子)，交叉轴 = max(子)**：容器的固有尺寸按方向语义不同 |

### 设计约束与已知陷阱

看起来「反直觉」的行为通常是刻意的。完整清单、每个约束背后的真缺陷、以及守护它们的测试，见 [CONTRIBUTING.md](CONTRIBUTING.md#design-constraints-and-known-traps)（英文）：

- 布局、绘制列表、光栅化必须用**同一个字号、同一个度量**。
- Vulkan 用**静态 viewport/scissor**；动态状态在某些核显上画不出任何像素。
- 颜色附件必须是 `R8G8B8A8_UNORM`，**不能**用 `_SRGB` —— CPU 基准不做 gamma 转换。
- 验证过程中抓到的 5 个真缺陷（3 个继承自原型，2 个在 Rust 移植时发现）全部保留为回归守卫。

## 文档

| 文档 | 内容 |
|---|---|
| [教程](docs/TUTORIAL.md) | 一步一步，14 节（§0–§13），每节可独立运行 |
| [功能清单](FEATURES.md) | **有哪些功能、做到哪一步、哪些还没做** —— 唯一真相 |
| [逐功能指南](docs/features/) | 某个功能的完整用法与坑 |
| [路线图](ROADMAP.md) | 里程碑 M1–M7 与验收判据 |
| [上手指南](docs/GETTING-STARTED.md) | 从没写过 Rust 时的慢速入口 |
| [Agent 须知](agent.md) | 仓库纪律与踩坑记录（写给 AI 协作者与新维护者） |

### 可运行示例

[`crates/deer-gui/examples/`](crates/deer-gui/examples/) 是看到功能跑起来的最快方式 —— 15 个示例，每个都带自检断言：

```sh
cargo run -p deer-gui --example tutorial        # 导览，产出 render_out/*.png
cargo run -p deer-gui --example geometry        # 布局与命中测试
cargo run -p deer-gui --example scene_file      # .dui 建树，与命令式等价
cargo run -p deer-gui --example gpu_geometry    # GPU 几何，与 CPU 后端逐像素对照
cargo run -p deer-gui --example glyph_atlas     # 字形图集打包
```

哪个功能对应哪个示例，见 [`FEATURES.md`](FEATURES.md)。

## 依赖

除窗口层外，本 workspace **没有第三方依赖**：

| Crate | 第三方依赖 |
|---|---|
| `deer-layout` | 无 |
| `deer-gpu` | 无 |
| `deer-vk` | 无 |
| `deer-gui`（不开 `window` feature） | 无 |
| `deer-window`、`deer-gui --features window` | `winit 0.30` |

`winit` 是唯一登记在案的例外，记录在 [`ROADMAP.md`](ROADMAP.md) 的 Q-1。新增依赖必须先在那里登记并说明理由。

## 平台支持

| 平台 | 布局 / CPU 光栅化 / 出 PNG | Vulkan 离屏 | 窗口 / 呈现 |
|---|---|---|---|
| Windows | ✅ | ✅（Vulkan 1.4，由驱动提供） | ✅ |
| Linux / macOS | ✅ | ✅ | ❌ 返回「平台不支持」错误 |

## 参与贡献

本仓库把文档当作交付物的一部分：新功能必须同时带上示例、指南和 `FEATURES.md` 登记，缺一不算完成。规则、门禁与命令速查见 [CONTRIBUTING.md](CONTRIBUTING.md)（英文）。

## 许可证

MIT —— 全文见 [LICENSE](LICENSE)，同一标识也声明在 [`Cargo.toml`](Cargo.toml) 的 `license` 字段。
