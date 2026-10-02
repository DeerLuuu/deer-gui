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
| **属性注册表**（按 `Kind` 枚举可编辑属性：名/类型/取值域/默认值/适用面；编辑器 Inspector、undo 粒度、`.dui` 语法面、拖拽写回四个消费者的共同上游） | ✅ | [prop-registry](docs/features/prop-registry.md) | `cargo run -p deer-gui --example prop_registry` |
| **内边距与间距**（`pad` / `gap`） | ✅ | [layout](docs/features/layout.md#内边距与间距) | `cargo run -p deer-gui --example tutorial`（第 2 步） |
| **主轴分配**（`grow` 权重） | ✅ | [layout](docs/features/layout.md#主轴分配-grow) | `cargo run -p deer-gui --example geometry` |
| **对齐**（`main` / `cross`：start/center/end/stretch） | ✅ | [layout](docs/features/layout.md#对齐) | `cargo run -p deer-gui --example scene_file`（`main=end`） |
| **固定与百分比尺寸**（`w` / `h`，`50%`） | ✅ | [layout](docs/features/layout.md#尺寸) | `cargo run -p deer-gui --example geometry` |
| **多行文本 + 垂直滚动容器**（`text` + `wrap` ⇒ 每行一条 `Text` 命令；`column` + `scroll` ⇒ `max_scroll` + 视口裁剪 + 滚轮驱动偏移，到边界不越界、视口外不命中） | ✅ | [scroll-and-multiline](docs/features/scroll-and-multiline.md) | `cargo run -p deer-gui --example scroll` |
| **滚动条**（可视：轨道 + 滑块，位置与高度反映当前偏移；**拖滑块改偏移** + **点轨道空白跳到指针处**并可续拖（T3.2b）；内容装得下 ⇒ 不画；默认 opt-in。惯性驱动收口为 `advance_inertia`/`inertia_deadline` 两行接线（参考 `scroll_inertia_window`）） | ✅ | [scrollbar](docs/features/scrollbar.md) | `cargo run -p deer-gui --example scroll_bar` |
| **绝对定位 / 层叠**（L1：`position: Pos::Offset` ⇒ 子节点脱离流内（不占槽、不计入父固有尺寸），位置 = 父内容盒原点 + 像素偏移（可负）；层叠 = **声明序**（后声明者后画且命中优先）；`.dui` 写法 `pos=x,y`；默认 opt-in ⇒ 未设时既有树逐字节不变） | ✅ | [absolute-positioning](docs/features/absolute-positioning.md) | `cargo run -p deer-gui --example overlay_demo` |
| **每子节点交叉轴对齐**（L2：`cross_self: Option<Align>` **覆盖**容器级 `cross_axis`，只对该流内子节点生效（对齐与 stretch 吃满都按它算）；`None` 回落容器级；流外（`position`）子节点不生效；`.dui` 写法 `cross-self=`；默认 opt-in ⇒ 未设时既有树逐字节不变） | ✅ | [align-self](docs/features/align-self.md) | `cargo run -p deer-gui --example layout_refine_demo` |
| **最小/最大尺寸**（L3：`min_w`/`max_w`/`min_h`/`max_h`（`Option<Size>`，像素/百分比），min 下限、max 上限，**measure 与 place 两处都生效**（固有尺寸聚合、显式尺寸、grow/stretch 分配结果都夹进 `[min,max]`）；`min > max` ⇒ min 赢；滚动容器主轴 bound=无穷时照常生效；流外子节点同样受夹；`.dui` 写法 `min-w=`/`max-w=`/`min-h=`/`max-h=`；默认 opt-in ⇒ 未设时既有树逐字节不变） | ✅ | [min-max-sizes](docs/features/min-max-sizes.md) | `cargo run -p deer-gui --example layout_refine_demo` |
| **命中测试**（坐标 → 哪个控件） | ✅ | [hit-testing](docs/features/hit-testing.md) | `cargo run -p deer-gui --example geometry` |

## 三、渲染与自检

| 功能 | 状态 | 指南 | 可运行示例 |
|---|---|---|---|
| **离屏渲染出 PNG** | ✅ | [rendering](docs/features/rendering.md) | `cargo run -p deer-gui --example render_to_png` |
| **原始像素**（RGBA8 缓冲） | ✅ | [pixels](docs/features/pixels.md) | `cargo run -p deer-gui --example pixels` |
| **零依赖 PNG 编码器** | ✅ | [pixels](docs/features/pixels.md#自己编码-png) | `cargo run -p deer-gui --example pixels` |
| **绘制列表**（树 → 与后端无关的命令） | ✅ | [draw-list](docs/features/draw-list.md) | `cargo run -p deer-gui --example draw_list` |
| **主题**（颜色 + 字号） | ✅ | [theme](docs/features/theme.md) | `cargo run -p deer-gui --example theme` |
| **GPU HAL**（后端抽象：`Backend`/`Device`/`Frame`/`Renderer`；设备/交换链/呈现已通，**T1.1 起 `record(&DrawList, Option<&mut TextEngine>)` 真的消费绘制列表**；`read_pixels` 的 `Unsupported` 是 T1.4 的**语义决策**并指向呈现帧回读） | ✅ | [gpu-hal](docs/features/gpu-hal.md) | `cargo run -p deer-gui --example vulkan_devices` |
| **CPU 参考后端**（软件光栅化） | ✅ | [rendering](docs/features/rendering.md#cpu-后端软件光栅化) | `cargo run -p deer-gui --example draw_list` |
| **Vulkan 后端**（设备 + 着色器 + 管线，真机真跑） | ✅ | [vulkan](docs/features/vulkan.md) | `cargo run -p deer-gui --example vulkan_pipeline` |
| **Vulkan 图形管线**（渲染通道 + 管线 + 绘制，像素经真机验证） | ✅ | [vulkan-pipeline](docs/features/vulkan-pipeline.md) | `cargo run -p deer-gui --example vulkan_pipeline` |
| **Vulkan 离屏渲染 + 回读**（含**绘制几何**，已修复段序缺陷） | ✅ | [gpu-offscreen](docs/features/gpu-offscreen.md) | `cargo run -p deer-gui --example gpu_offscreen` |
| **字形光栅化**（轮廓 → 覆盖率位图，超采样抗锯齿） | ✅ | [glyph-raster](docs/features/glyph-raster.md) | `cargo run -p deer-gui --example glyph_atlas` |
| **字形图集**（货架打包 + 1px padding + 按需增高） | ✅ | [glyph-atlas](docs/features/glyph-atlas.md) | `cargo run -p deer-gui --example glyph_atlas` |
| **真实字体度量与换行**（`FontMeasure` 替换「每字符 0.6em」近似） | ✅ | [text-rendering](docs/features/text-rendering.md) | `cargo run -p deer-gui --example text_render` |
| **真实字形渲染**（CPU 后端贴真实字形，不再是方块占位） | ✅ | [text-rendering](docs/features/text-rendering.md) | `cargo run -p deer-gui --example text_render` |
| **窗口**（真窗口 + 事件循环，**仅 Windows**） | ✅ | [window](docs/features/window.md) | `cargo run -p deer-gui --features window --example window_preview` |
| **Vulkan 上屏**（`VkSurfaceKHR` + 交换链 + 帧同步 + 呈现） | ✅ | [vulkan-swapchain](docs/features/vulkan-swapchain.md) | `cargo run -p deer-gui --features window --example window_preview` |
| **GPU 几何渲染**（`DrawList` 形状命令 → GPU，与 CPU **逐像素对照**） | ✅ | [gpu-geometry](docs/features/gpu-geometry.md) | `cargo run -p deer-gui --example gpu_geometry` |
| **GPU 文本渲染**（字形四边形 + 图集纹理 + 最近邻采样，与 CPU **逐字节对照**） | ✅ | [gpu-geometry](docs/features/gpu-geometry.md#3-完整-api) | `cargo run -p deer-gui --example gpu_geometry` |
| **窗口里显示界面**（把 `DrawList` 的形状与文本**呈到窗口**，上屏像素与 CPU 逐像素对照） | ✅ | [window](docs/features/window.md) | `DEER_VK_WINDOW_TESTS=1 cargo run -p deer-gui --features window --example window_parity` |
| **输入与焦点**（事件通路 + 命中/状态机（含裁剪与禁用）+ Tab/Shift+Tab/Escape 焦点 + 文本输入 + **输入框光标**（T3.5：在光标处插入 / `Backspace` 删**光标前一个 Unicode 字符** / **左右方向键**移动光标，单位是**字符位**）+ 脚本化重放 + **只在状态变化时重绘**） | ✅ | [input](docs/features/input.md) | `cargo run -p deer-gui --features window --example interactive_form` |
| **日志**（零依赖门面；分级 + 按 target 过滤；`DEER_LOG` 开关；**默认完全静默**、只写 stderr） | ✅ | [logging](docs/features/logging.md) | `cargo run -p deer-gui --example logging` |
| **IME 预编辑**（中文/日文「还没上屏的那一段」：只进 `UiState::preedit` 缓冲、**不进 `texts`**；提交进 `texts` 并清缓冲 ⇒ **无双写**；画在光标处 + 下划线、**光标推到它之后**） | ✅ | [ime](docs/features/ime.md) | `cargo run -p deer-gui --example ime_preedit` |
| **测试接口（testkit）**（建面 + 输入注入（单事件/脚本 + `move @id`）+ 一帧**内置前置断言** + 离屏 CPU/GPU 渲染 + 像素/状态/绘制列表断言 + **CPU↔GPU 对照**（不透明 0 / 半透明 ≤1 LSB）+ 门槛自证 + **可复制的复现命令**） | ✅ | [testing](docs/features/testing.md) | `cargo run -p deer-gui --features testing --example testkit_demo` |
| **通用纹理 / `RGBA8_UNORM`**（创建 + 上传 + **回读四通道保真**；离屏**纹理 quad** 与 CPU 参考逐字节相同；uv 朝向钉住 V 翻转） | ✅ | [textures](docs/features/textures.md) | `cargo run -p deer-gui --example textures` |
| **间接绘制**（`vkCmdBindIndexBuffer` + `vkCmdDrawIndexedIndirect(drawCount = 1, stride = 20)`，**离屏 + 窗口两条路径**；`RenderStats::indirect_draws` 与真实调用同处计数 ⇒ 换回 `vkCmdDraw` 必然变红；索引/间接缓冲惰性创建 + 跨帧复用 ⇒ 稳态零分配零上传） | ✅ | [indirect-draw](docs/features/indirect-draw.md) | `cargo run -p deer-gui --example indirect_draw` |

## 四、还没做的（**不要以为能跑**）

| 功能 | 里程碑 | 现状说明 |
|---|---|---|
| **批处理优化**（统一管线 + 跨帧复用顶点缓冲） | M3+ | **已落地**：**统一管线**（形状 + 文本合成**一条顶点流 + 一条管线** ⇒ 每帧**一次** `vkCmdDraw` / **一次**管线切换，改造前是每段一次；原「合段」函数已随统一管线删除）+ **跨帧复用缓冲**（语料不变时不再每帧重建/重传）。`RenderStats` 可复现：`DEER_VK_WINDOW_TESTS=1 cargo run -q -p deer-gui --features window --example window_parity`（**具体数字以运行输出为准**）。**仍未做**：**多批次提交** —— 登记理由：统一管线已经是 **1 bind / 1 draw / 1 submit 每帧**（计数表三项恒为 1）⇒ **没有可合并的批次**；「多批」的前提是**多张纹理 / 多个渲染目标**（bindless / 多 pass 范畴），属另一项需求。另有**每帧 `unify` 的 `Vec` 堆分配**、窗口侧屏障断言缺口（见 [`window.md`](docs/features/window.md) 第 7 节）。性能项，**像素判据不变** |
| **纹理作为 `DrawCmd` 进 `DrawList`** | M6 | 「一张任意纹理铺到矩形上」目前只有测试/诊断需要 ⇒ **没有**进 `DrawCmd`；界面树里贴图 = 控件的活（M6 `Icon`）。**T1.3 的另两项已补齐**：按 **RGB 调制**（`fragment_shader_textured` ⇒ 四通道都进像素）与**窗口路径贴任意纹理**（`WindowedRenderer::draw_textured_quad`），见 [`textures.md`](docs/features/textures.md)。纹理趟是单独一趟 present（会清屏）⇒ 纹理**没法与界面同帧叠加**（要叠需同趟两次 draw + 中途换描述符，未做） |
| **推送常量矩形着色器**（`spirv.rs::vertex_shader_rect_pushconstant`） | M3 | **这支 SPIR-V 是损坏的**（校验层 `VUID-StandaloneSpirv-PushConstant-06808`）；请求校验层时会让进程 `0xc0000005` 崩溃，所以 `device_smoke` / `pipeline_smoke` 里涉及它的测试在**校验层下显式跳过**（t15）。**task-18 后三条路径（`VkBackend::new` / 设备·离屏 / 窗口）都读 `DEER_VK_VALIDATION`**（当时 offscreen 的 3 个真缺陷已修）；实测 `DEER_VK_VALIDATION=1 cargo test -p deer-vk` → **全部通过 / 0 failed**、零校验消息（**具体条数以运行输出为准**，本仓库不在文档里固化测试条数）。矩形改**顶点缓冲**即可修（M3a 的新几何路径已用顶点缓冲，不再走这支着色器） |
| **字形 hinting**（小字号像素对齐） | M4 残余 | **不做，但有实测依据**：最省的 hinting-lite（垂直两极对齐）已实现并量过 —— 最大形变 **12.5%**、部分覆盖质量区间 **[-10.3%, +12.8%]**、均值 **+0.4%** ⇒ **没有净收益**；真 hinting 需指令虚拟机 + stem 识别 + CVT |
| **亚像素水平定位**（1/4 相位档） | M4 残余 · **上半已落地 · 下半已决策（不接入）** | **光栅化层已提供 opt-in 路径**（`Rasterizer::rasterize_at` / `rasterize_char_at` + `split_subpixel_x`；实测落位误差 RMSE **0.2890→0.0733 px**）；**默认的 `rasterize` 仍整数落位、逐字节不变**；**不接进** `TextEngine`/`CpuRenderer` —— 已按「实测否定」登记（理由：代价是**间距精度换边缘锐度**，部分覆盖质量占比最高上升 **+64.3%**，**无净视觉收益**；图集记录 ×4；且会改动默认落位 ⇒ 破坏全仓像素 parity 判据。见 `ROADMAP.md` 的「M4-6 决策登记」）。**不做** LCD 子像素（RGB 三通道） |
| **字距与连字**（`kern` / `GSUB` / `GPOS`） | M4 残余 | 不做整形，`advance` 就是 `hmtx` 的原始值 |
| **CFF / OpenType-CFF 字体**（`OTTO`） | M4 残余 | 解析层直接报错，不静默给空轮廓；只支持 `glyf` 轮廓 |
| **竖排 / RTL / 复杂脚本整形** | M4 残余 | 完全没有；不读 `GSUB`/`GPOS` |
| **输入与焦点的剩余部分**（仅剩：停靠 / 多窗口） | M5/M6 | **输入地基全部落地**：事件通路、命中/状态机、Tab/Escape 焦点、文本输入、脚本重放、事件驱动重绘、滚轮垂直滚动、可视滚动条（拖滑块 + 点轨道跳转）、惯性收口（T3.2b）、输入框光标（T3.5/T3.8）、IME 预编辑（T3.4）、**方向键上下导航（T3.1 几何邻近）**、**按键滚动（`PageUp`/`PageDown`/`Home`/`End`）**、**右键透传 `PointerRight`（T3.3）**、**按键重复 `repeat: bool`（T3.6）**（见 [`input.md`](docs/features/input.md) 第 6 节）；**真机输入法**只能人肉验证 |
| **可停靠面板 dock**（拖动改位置 / 边缘折叠） | M6 | 完全没有 |
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
