# deer-gui 功能清单

> **这是功能状态的唯一真相。** 每个功能必须有：**状态 + 使用指南 + 可运行示例**。
>
> 规矩（由本仓库的纪律强制）：
> 1. 标为 ✅ 的功能**必须有一个能跑的示例**（`cargo run -p deer-gui --example <名字>`）；
> 2. 每个 ✅ 功能**必须有一份指南** `docs/features/<名字>.md`；
> 3. **新增功能时同时更新本文件、指南、示例** —— 三者缺一不算完成。
>
> 新手入口：[`TUTORIAL.md`](docs/TUTORIAL.md)（一步一步）。逐功能用法：[`docs/features/`](docs/features/)。

## 图例

| 标记 | 含义 |
|---|---|
| ✅ | 完成，有示例 + 指南，可放心用 |
| 🔄 | 部分完成（已实现的能力能用，未实现的部分会明确写出） |
| ⬜ | 未实现（**不要以为能跑**） |

---

## 一、构筑界面（写一棵界面树）

| 功能 | 状态 | 指南 | 可运行示例 |
|---|---|---|---|
| **命令式 API**（imgui 式手感） | ✅ | [imperative-api](docs/features/imperative-api.md) | `cargo run -p deer-gui --example tutorial`（第 1–3 步） |
| **`.dui` 场景文件**（`tscn` 式） | ✅ | [scene-file](docs/features/scene-file.md) | `cargo run -p deer-gui --example scene_file` |
| **两条路径产出同一棵树** | ✅ | [imperative-api](docs/features/imperative-api.md#两条路径等价) | `cargo run -p deer-gui --example scene_file` |
| **容器嵌套**（`Row`/`Column`） | ✅ | [imperative-api](docs/features/imperative-api.md#容器嵌套) | `cargo run -p deer-gui --example tutorial`（第 3 步） |
| **节点树**（数据模型、id 规则、两套写法互转） | ✅ | [node-tree](docs/features/node-tree.md) | `cargo run -p deer-gui --example tutorial`（第 6 步） |

## 二、布局

| 功能 | 状态 | 指南 | 可运行示例 |
|---|---|---|---|
| **布局计算**（树 + 画布 → 几何表） | ✅ | [layout](docs/features/layout.md) | `cargo run -p deer-gui --example geometry` |
| **内边距与间距**（`pad` / `gap`） | ✅ | [layout](docs/features/layout.md#内边距与间距) | `cargo run -p deer-gui --example tutorial`（第 2 步） |
| **主轴分配**（`grow` 权重） | ✅ | [layout](docs/features/layout.md#主轴分配-grow) | `cargo run -p deer-gui --example geometry` |
| **对齐**（`main` / `cross`：start/center/end/stretch） | ✅ | [layout](docs/features/layout.md#对齐) | `cargo run -p deer-gui --example scene_file`（`main=end`） |
| **固定与百分比尺寸**（`w` / `h`，`50%`） | ✅ | [layout](docs/features/layout.md#尺寸) | `cargo run -p deer-gui --example geometry` |
| **命中测试**（坐标 → 哪个控件） | ✅ | [hit-testing](docs/features/hit-testing.md) | `cargo run -p deer-gui --example geometry` |

## 三、渲染

| 功能 | 状态 | 指南 | 可运行示例 |
|---|---|---|---|
| **离屏渲染出 PNG** | ✅ | [rendering](docs/features/rendering.md) | `cargo run -p deer-gui --example render_to_png` |
| **原始像素**（RGBA8 缓冲） | ✅ | [pixels](docs/features/pixels.md) | `cargo run -p deer-gui --example pixels` |
| **零依赖 PNG 编码器** | ✅ | [pixels](docs/features/pixels.md#自己编码-png) | `cargo run -p deer-gui --example pixels` |
| **绘制列表**（树 → 与后端无关的命令） | ✅ | [draw-list](docs/features/draw-list.md) | `cargo run -p deer-gui --example draw_list` |
| **主题**（颜色 + 字号） | ✅ | [theme](docs/features/theme.md) | `cargo run -p deer-gui --example theme` |
| **GPU HAL**（后端抽象：`Backend`/`Device`/`Frame`/`Renderer`） | 🔄 | [gpu-hal](docs/features/gpu-hal.md) | `cargo run -p deer-gui --example vulkan_devices` |
| **CPU 参考后端**（软件光栅化） | ✅ | [rendering](docs/features/rendering.md#cpu-后端软件光栅化) | `cargo run -p deer-gui --example draw_list` |
| **Vulkan 后端**（设备 + 着色器） | 🔄 | [vulkan](docs/features/vulkan.md) | `cargo run -p deer-gui --example vulkan_devices` |
| **Vulkan 图形管线**（渲染通道 + 管线，**还画不出像素**） | 🔄 | [vulkan-pipeline](docs/features/vulkan-pipeline.md) | `cargo run -p deer-gui --example vulkan_pipeline` |

## 四、还没做的（**不要以为能跑**）

| 功能 | 里程碑 | 现状说明 |
|---|---|---|
| **渲染到窗口**（屏幕上显示） | M2b | 完全不能。需要先定窗口方案（`ROADMAP.md` Q-1） |
| **GPU 渲染出图**（Vulkan 画像素） | M2a-4..6 | 管线已能建成功，但命令缓冲 / 离屏图像 / 回读还没做 |
| **真实字形**（现在图片里是方块占位） | M4 | 需要字体解析 + 字形光栅化 + 图集 |
| **文字换行与文本度量** | M4 | 现在是「每字符 0.6em」的近似度量（确定性，但不是真实字体） |
| **输入事件**（鼠标/键盘点击回调） | M5 | `hit_test` 有了（能算命中），但没有事件派发 |
| **焦点系统**（Tab / Enter / 方向键） | M5 | 完全没有 |
| **可停靠面板 dock**（拖动改位置 / 边缘折叠） | M5 | 完全没有 |
| **控件族**（12 个 `deer-ui` 控件的语义） | M6 | 现在只有 5 种节点：`column`/`row`/`text`/`button`/`field` |
| **DX12 / Metal 后端** | M7 | 完全没有 |

---

## 五、给维护者：新增功能时的清单

每完成一个新功能，**按顺序**做这四件事（缺一不算完成）：

1. **加示例**：`crates/deer-gui/examples/<名字>.rs`，要求：
   - 顶部注释写明「这是什么 + 怎么跑 + 产物在哪」；
   - 结尾有**自检断言**（例如「画面不能只有一种颜色」），避免「跑成功但什么都没做」；
   - 真的跑一遍确认 `exit=0`。
2. **加指南**：`docs/features/<名字>.md`，照 [`TEMPLATE.md`](docs/features/TEMPLATE.md) 写。
3. **登记本文件**：在对应的表里加一行，状态、指南链接、示例命令都要对。
4. **更新 `docs/TUTORIAL.md`**：如果这个功能属于新手主线，加进教程的对应章节。

> **为什么这么严**：M1 交付后曾经出现过「库能跑，但使用者不知道如何渲染任何东西」——
> 因为渲染链中间缺了一段（`Renderer` 只有 trait、没有实现），而且没有面向使用者的文档。
> 本清单就是为了让那个缺口不再出现。
