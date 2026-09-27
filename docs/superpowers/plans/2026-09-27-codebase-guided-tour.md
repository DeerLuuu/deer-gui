# deer-gui 代码库导览计划（带你读懂整个项目，以便自己拓展功能）

> **For agentic workers:** REQUIRED SUB-SKILL: 无需 subagent-driven-development（这是**教学/导览**计划，不是实现计划）。步骤用 `- [ ]` 勾选跟踪。

**Goal:** 让项目作者**自己**能在不依赖我的情况下读懂并扩展 deer-gui：每一讲给出「这一层是什么、代码在哪、数据怎么流、想改 X 动哪里、怎么自证没改坏」。

**Architecture:** 导览按**数据流**而不是按目录顺序推进 —— 从「一个 UI 树如何变成屏幕上一块像素」这条主线出发，每讲只引入一个新概念，并始终挂回主线的位置。每讲产出一份留存文档（`docs/tour/*.md`），仓库里已有的 README/TUTORIAL/`docs/features/*` 作为**延伸阅读**而不是被重复。

**Tech Stack:** Rust 2024（rust-version 1.85）、手写 Vulkan 绑定（无 ash/vulkano）、自研 SPIR-V 汇编器（无 shaderc/glslc）、winit 0.30（唯一第三方依赖，仅窗口层）。

**Spec:** `README.md`、`docs/TUTORIAL.md`（14 章）、`FEATURES.md`、`ROADMAP.md`、`docs/features/*.md`、根 `AGENTS.md`（若存在）——本计划是这些文档的**地图与阅读顺序**，不替代它们。

## Global Constraints

- **每讲必须带 `path:line` 证据锚点**；讲完即停，不编造「当时的想法」（沿用 `explaining-changes` 的证据规则）。
- **每讲必须给一条可运行命令**（`cmd /c "cargo ..."`）让你自己复现结论。
- **每讲末尾三问自测**（答不出就退回上一讲）。
- 事实以**仓库现状**为准；发现「文档与代码不一致」要当场指出（本项目已出现过多次这类问题）。
- 教学文档一律**中文**；不跑 `cargo fmt`；新增文档不影响 `docs_consistency` 门禁（它只校验 FEATURES 的 ✅ 行）。

---

### 第 0 讲：全景图 —— 五个 crate 各干什么、数据怎么流

**Materials:** `Cargo.toml`（workspace 成员）、`crates/*/Cargo.toml`（依赖关系）、`README.md` 架构段
**逐步做到:** 能画出「UI 树 → 布局 → DrawList → (CPU 像素 | GPU 顶点流 → Vulkan → 像素) → 窗口」的方框图；能说出每个 crate 的一句话职责与**不许越界**的规则（如 deer-gpu 不得依赖 Vulkan）。
**自测:** ① 谁产生 `DrawList`？② 谁把 `DrawList` 变成像素？③ 唯一允许用第三方库的是哪个 crate，为什么？

### 第 1 讲：数据模型 —— `DrawList` / `DrawCmd` / 几何与颜色

**Materials:** `crates/deer-gpu/src/draw.rs`、`paint.rs`（若存在）、颜色与矩形类型定义
**逐步做到:** 会手写一段 `DrawList` 并用 CPU 后端渲染成 PNG；能说清每个 `DrawCmd` 的语义（含 `PushClip/PopClip` 的配对规则与 `ClipBalanced` 判据）。
**自测:** ① 一条 `StrokeRect{width:3}` 在 CPU 后端等价于哪几条填充？② 裁剪栈不平衡时两边的行为差在哪？

### 第 2 讲：布局层 —— 盒模型与测量

**Materials:** `crates/deer-layout/src/*.rs`
**逐步做到:** 会给一个容器树设定尺寸/间距并断言结果矩形；理解「测量与摆放分两趟」的原因。
**自测:** ① 百分比/固定/自动尺寸分别在哪一步解析？② 改一个布局算法要动哪些测试？

### 第 3 讲：CPU 参考后端 —— 为什么它是最重要的代码

**Materials:** `crates/deer-gpu/src/null.rs`（`fill`/`inside_rounded`/`stroke`/`blend`/`blend_cov`）
**逐步做到:** 能逐行读懂圆角与描边的**整数像素判据**，并解释它为何是 GPU 侧的**黄金基准**。
**自测:** ① 圆角判据用的是像素中心还是整数坐标？② `blend` 为什么用 `round()` 而不是浮点直通？

### 第 4 讲：字形与文本 —— 从字体文件到像素覆盖度

**Materials:** `crates/deer-gpu/src/{glyph,raster,atlas,measure,text}.rs`、`tests/{text_raster,text_measure,glyph_atlas,text_pixels}.rs`
**逐步做到:** 能把一个字符光栅化成覆盖率位图并写进图集、按度量换行；理解「glyph 索引 + 像素尺寸」做键的原因。
**自测:** ① 缺字为什么走 `.notdef` 而不是报错？② 图集扩容为什么必须保留旧槽位？

### 第 5 讲：Vulkan 底座 —— 手写绑定与 HAL

**Materials:** `crates/deer-vk/src/{ffi,ffi_dev}.rs`、`device.rs`、`hal.rs`
**逐步做到:** 能读出手写绑定与 `ash` 的取舍；能建实例/设备/渲染通道/管线；理解 `hal.rs` 抽象给上层的契约。
**自测:** ① 为什么 `record()` 遇到非空 DrawList 要返回 `Unsupported`？② `present_result_of()` 为什么被单独抽成纯函数？

### 第 6 讲：自研 SPIR-V 汇编器 —— 最容易踩坑的一层

**Materials:** `crates/deer-vk/src/spirv.rs`（模块文档 + `Module` API + 各着色器函数）、`tests/spirv_val.rs`
**逐步做到:** 能读/改一支着色器；能解释**段序**规则与「为什么必须过官方 `spirv-val`」。
**自测:** ① `OpFloor` 为什么不能直接用 core 的 8 号？② 为什么不能用运行时常量数组索引？

### 第 7 讲：GPU 几何渲染器与 parity —— 本项目的验收方式

**Materials:** `crates/deer-vk/src/{gpu_geom,gpu_render}.rs`、`tests/{gpu_geom_stream,gpu_geom_parity,gpu_vs_cpu}.rs`
**逐步做到:** 能把一个 `DrawList` 送上 GPU 并回读像素；能解释「不透明逐字节 / 半透明 ≤1 LSB」这条验收线的由来与边界。
**自测:** ① 为什么裁剪在 CPU 侧做而不是用 scissor？② 半透明的 1 LSB 为什么不许断言成 0？

### 第 8 讲：窗口与交换链 —— 让画面出现在屏幕上

**Materials:** `crates/deer-window/src/lib.rs`、`crates/deer-vk/src/{surface,swapchain,windowed}.rs`、`crates/deer-gui/examples/{window_preview,hal_window_path}.rs`
**逐步做到:** 能跑起窗口示例并解释事件循环、实例/表面/交换链重建、呈现结果映射。
**自测:** ① 动态 viewport/scissor 在本机为什么会画不出像素？② 交换链 `OutOfDate` 应该怎么处理？

### 第 9 讲：应用层与工程纪律

**Materials:** `crates/deer-gui/src/lib.rs`、`examples/*.rs`、`crates/deer-gui/tests/docs_consistency.rs`、`AGENTS.md`
**逐步做到:** 能新增一个示例、跑齐四道门禁，并说出「每条结论都要有命令+输出」的纪律来源。
**自测:** ① 新增一个 ✅ FEATURES 行需要同时补什么？② 为什么仓库不用 `cargo fmt`？

### 第 10 讲：拓展指南 —— 加一个 DrawCmd / 加一个控件 / 加一个后端

**Materials:** 前九讲的全部锚点
**逐步做到:** 沿一条真实路径走完三种扩展，并知道每一步该跑哪个测试、哪些文件必须同步改（含文档契约）。
**自测:** ① 新增 `DrawCmd::Line` 需要动哪些文件、哪几处对照测试？② 若要把 GPU 后端换成别的 API，边界应该划在哪一层？

---

## 测绘校正（导览过程中被实测纠正的事实，先记下再讲）

| # | 我先前的说法 | 实测事实 | 来源 |
|---|---|---|---|
| 1 | 「根 `AGENTS.md`」可作素材 | **`AGENTS.md` 不存在**（glob ×2 + `git log --all -- AGENTS.md` 三路确认）⇒ 本仓库的纪律只能从 `FEATURES.md:3-8/89-97`、`README.md:24-25`、`ROADMAP.md:109-159`、两个 plan 的 Global Constraints、`docs/features/TEMPLATE.md` 提取 | `docs/tour/03-app-and-process.md` §5 |
| 2 | 「四道门禁」是既有约定 | **没有单一权威文档**：`docs/superpowers/plans/2026-09-27-m3a-…md:20` 只列**三条**（workspace 测试 / clippy / `DEER_VK_VALIDATION=1`）；第四道「示例全跑」是测绘员**本轮补实跑**后写进去的 | 同上 §8 |
| 3 | 门禁数字 | **条数不写进文档**（本仓库现行口径，远端 `cb7291e` 统一）：`cargo test --workspace` 与 `DEER_VK_VALIDATION=1 cargo test -p deer-vk` 都只看 `test result:` 行是否全过，条数用 `cargo test --workspace 2>&1 \| Select-String "^test result:"` **现取**（**以运行输出为准**）；校验消息计数必须用**行首括号前缀 + `-CaseSensitive`**（`^\[VK ERROR\]\|^\[VALIDATION\]`，松散模式实测假命中 5 条全是用例名）。结构：五个 crate 各有 lib + 若干 `tests/*.rs` 靶（清单见 `docs/tour/03-app-and-process.md` §4.2）；15 个 deer-gui 示例 + `window_smoke` 的判据是 `exit=0` | 同上 §4 |

**顺带修掉我的一处含糊**：第 0 讲里我说的「四道门禁」应改写为「**三条有文档依据的门禁 + 示例全跑（补充项）**」，避免把自造约定讲成既有约定。

---

## 交付物与节奏

- 每讲产出 `docs/tour/0N-<slug>.md`（带 `path:line` 锚点、命令、三问自测），**读完一讲再进下一讲**。
- 讲完第 0–4 讲后，你会具备「改 CPU 侧一切东西」的能力；第 5–7 讲之后具备「改 GPU 侧」；第 8–10 讲之后具备「改窗口/拓展架构」。
- 测绘员并行产出的事实地图（`docs/tour/01-core-and-cpu.md` / `02-vulkan.md` / `03-app-and-process.md`）是本计划的**素材**，由我整合进各讲。
