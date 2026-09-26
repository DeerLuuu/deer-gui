# deer-gui

> **从零实现的 Rust GUI 运行时** —— 不依赖 web、不依赖 DOM、不依赖 `wgpu`/`ash`/`vulkano`。
> 以**节点树**为核心数据模型，支持两种构筑方式（imgui 式命令式 API / `.tscn` 式场景文件），
> 由自研布局引擎驱动，渲染后端可插拔。

来源：`deer-ui` 的控件语义 + 一个 TypeScript 验证原型（V0，28 条断言）验证过的布局代数。
**不是** web 项目的移植：DOM/CSS 全部丢弃，窗口、输入、渲染自己实现。

> ### 🚀 第一次用？看这三份文档
> | 文档 | 用途 |
> |---|---|
> | [**教程**](docs/TUTORIAL.md) | 一步一步，10 章，每章可独立运行 |
> | [**功能清单**](FEATURES.md) | **有哪些功能、做到哪一步、哪些还不能** ← 唯一真相 |
> | [**逐功能指南**](docs/features/) | 某个功能的完整用法与坑 |
>
> 从没写过 Rust：[上手指南](docs/GETTING-STARTED.md)（更啰嗦的版本）。
> 只想立刻看到结果：
> ```sh
> cd Z:\deer-gui
> cargo run -p deer-gui --example tutorial     # → render_out/*.png
> ```

> **维护约定**：新增功能时**必须同时**加示例 + 加指南 + 登记 `FEATURES.md`
> （缺一不算完成）。理由见 `FEATURES.md` 结尾 —— M1 交付后曾出现「库能跑但没人知道怎么用」。

## 现状（里程碑 M1 完成）

| 项 | 状态 |
|---|---|
| `deer-layout`：节点树 + 布局代数 + 命中测试 + `.dui` 解析 | ✅ **24 条断言全绿**（18 布局不变量 + 5 FFI 布局 + 1 忽略） |
| `deer-gpu`：GPU HAL + CPU 参考后端（软件光栅化） | ✅ 类型与契约就位；CPU 后端能把绘制列表渲成像素 |
| `deer-vk`：Vulkan 后端（**自己声明符号 + 运行时动态加载**） | ✅ **真机枚举到 2 个 GPU**（Intel RaptorLake / NVIDIA RTX 5070 Ti, Vulkan 1.4.341） |
| 窗口 / 交换链 / 管线 / 出图 | ⬜ 里程碑 M2–M3 |
| 文本排版与字形图集 | ⬜ 里程碑 M4 |
| dock（可停靠面板布局） | ⬜ 里程碑 M5 |

## 快速开始

```sh
cargo test --workspace          # 全部断言（52 条）
cargo test -p deer-vk -- --nocapture   # 看本机枚举到的 GPU

# 渲染一张图（现在唯一能「看见」东西的方式）
cargo run -p deer-gui --example render_to_png
# → 在当前目录写出 render_to_png.png
```

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

要拿到像素而不是文件：`render_tree_to_rgba(&tree, w, h, theme)` → `(宽, 高, RGBA8)`。

要单独看布局：`deer_gui::layout_tree(&tree, w, h, theme)` → nodeId 到 `Rect` 的几何表。

### ⚠️ 现在能渲染到什么程度

| 能力 | 状态 |
|---|---|
| 建树（命令式 / `.dui` 场景文件） | ✅ |
| 布局计算、命中测试 | ✅ |
| 树 + 几何 → 绘制列表 → **像素**（CPU 软件光栅化） | ✅ |
| 写 PNG 文件 | ✅ |
| **渲染到窗口 / 屏幕上** | ❌ 里程碑 M2–M3 |
| 真实字形排版（现在是**等宽格占位**） | ❌ 里程碑 M4 |
| 输入事件与焦点 | ❌ 里程碑 M5 |

**所以现在只能离屏出图**，不能像正常 GUI 那样开窗。上面那张示例图里的「字」是占位方块，
不是字体渲染 —— 几何、层次、颜色是真的，字形不是。

## 架构

```
crates/
├── deer-layout/   语言无关核心：Node 树、布局代数、命中测试、.dui 场景解析
│                  ── 零平台依赖，可在无 GPU 的 CI 里完整断言
├── deer-gpu/      GPU HAL：Backend/Device/Swapchain/Frame trait、DrawList、CPU 参考后端
│                  ── 加一个后端 = 实现一个 trait
└── deer-vk/       Vulkan 后端：自己声明 extern 符号 + LoadLibraryW 动态加载
```

### 两条构筑路径，一棵树

```text
  Builder（命令式，imgui 式手感）─┐
                                  ├─→ Node 树 ─→ layout() → 几何表 ─→ 后端
  parse_scene(.dui，.tscn 式）   ─┘                └─→ hit_test() → 输入路由
```

两条路径产出**结构相等**的树（`Node::structurally_eq`），这是核心不变式，
也是测试 `t1_two_authoring_paths_produce_the_same_tree` 钉住的东西。

```rust
// 命令式
let mut app = Builder::new(Kind::Column, "app").padding(12.0).gap(8.0);
app.text("标题");
app.container_opts(Kind::Row, "bar", L::new().gap(8.0).to_props(), |r| {
    r.button("确定");
    r.button("取消");
});
let tree = app.build();
```

```text
# 等价的场景文件（.dui）
[column name=app pad=12 gap=8]
  [text label=标题]
  [row name=bar gap=8]
    [button label=确定]
    [button label=取消]
```

## 布局引擎的不变式

每条都有对应测试（`crates/deer-layout/tests/layout_invariants.rs`）：

| # | 不变式 |
|---|---|
| I-1 | **纯函数**：不改输入树，只回几何表 |
| I-2 | **确定性**：同输入 ⇒ 逐位相同输出（无时间/随机/环境探测） |
| I-3 | **自底向上**：先算子节点固有尺寸，父容器再分配 |
| I-4 | **像素取整**：几何全部整数 |
| I-5 | **不假设拥有窗口**：根盒子由宿主给，根**不撑满** |
| I-6 | **不越界**：结果夹在可用空间内 |
| I-7 | **分配尺寸 ≠ 可用空间**：父分配的主轴尺寸必须被采信；百分比相对**父内容盒**解析 |
| I-8 | **主轴 = sum(子)，交叉轴 = max(子)**：容器的固有尺寸按方向语义不同 |

I-7 与 I-8 都是**踩过坑之后立的**（见下）。

## 从验证原型继承的三个真缺陷

`deer-ui` 的 TypeScript 原型（V0）用「改坏 → 红 → 改回」抓到了三个真缺陷，
Rust 版把它们的**回归守卫**全部保留：

| # | 缺陷 | 守卫 |
|---|---|---|
| **B-1** | 两条构筑路径的 id 规则不一致（单计数器 vs 按类型计数）⇒ 树不等 | `t1_two_authoring_paths_produce_the_same_tree`、`t1_auto_id_rule_is_shared` |
| **B-2** | 布局把「父分配尺寸」当成「可用空间上限」⇒ `grow` 分配被静默丢弃 | `t5_grow_fills_the_row` |
| **B-3** | 容器固有尺寸漏算子节点显式尺寸（且方向语义错误） | `t13`、`t14_container_cross_axis_is_max_not_sum` |

Rust 移植过程中又抓到两个（同样已修 + 立守卫）：
- **R-1** 建造顺序：`with_props`/`with_layout` 是**整体赋值**，先定 id 再设它们会连 id 一起改掉
- **R-2** 主轴对齐的剩余空间算在 `grow` **之前** ⇒ `grow` 一旦生效，`center`/`end` **静默失效**

## 为什么不需要 Vulkan SDK

`#[link(name = "vulkan-1")]` 在**没有 SDK** 的机器上会链接失败
（`LNK1181: 无法打开输入文件 "vulkan-1.lib"` —— 系统只带 `vulkan-1.dll`，导入库属 SDK）。

所以 `deer-vk` 走**运行时动态加载**：`LoadLibraryW("vulkan-1.dll")` + `GetProcAddress`，
只依赖 `kernel32`。结构体布局在 crate 内手写并用 `offset_of!` 断言钉住。
结果：**不需要 SDK、不需要 ash**，且在真机上枚举到了 2 个 GPU。

## 待办

见 [`ROADMAP.md`](ROADMAP.md) 与 [`docs/M1-report.md`](docs/M1-report.md)。
