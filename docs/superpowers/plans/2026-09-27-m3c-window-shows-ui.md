# M3c — 窗口里显示界面（把 M3a/M3b 的画法接上交换链并呈现）

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers `subagent-driven-development`（逐任务：实现者 → 独立 reviewer → fix round）；DSH 侧用共享任务板 + `send_message`。

**Goal:** 让 `window_preview` 窗口里显示**真实的界面树**（形状 + 文本），而不是 M2a 那个硬编码三角形；且**上屏的像素**与 CPU 基准一致（可复现判据，不是「看起来对」）。

**Architecture:** 复用 M3a/M3b 已冻结的**顶点流构造**（`gpu_geom::build_stream` / `gpu_text::build_text_stream`，两者都是 extent 驱动 + CPU 侧裁剪）与两条管线状态（形状管线 / 文本管线）；把它们从 `gpu_render.rs`（离屏）**提到共用层**，让 `windowed.rs` 也能用同一套状态与同一批着色器。窗口路径**继续用它既有的动态 viewport/scissor**（M2b 已实证能上屏），但每帧必须显式设置成与离屏路径**同向**（`OriginUpperLeft` + 像素 y 从上往下），否则 `fragment_shader_rect_shape` 里的 `gl_FragCoord` 判据会整片错位。

**Tech Stack:** Rust 2024；自研 SPIR-V 汇编器；手写 Vulkan 绑定；winit 0.30（窗口层唯一第三方依赖）。

**Spec:** `docs/features/gpu-geometry.md`（M3a/M3b 的不可回退前提）、`ROADMAP.md` M3c 行、`crates/deer-vk/src/{windowed,gpu_render,gpu_geom,gpu_text}.rs`、`crates/deer-gui/examples/window_preview.rs`。

## Global Constraints

- 零新增第三方依赖；`deer-vk`/`deer-gpu` 保持零依赖。
- CPU 后端是基准；不得为让对照通过而改 `null.rs`。
- 静态/动态 viewport 的选择**按路径**：离屏用静态（~~M2a 实测动态在 Intel 上零像素~~ —— **该理由已标为存疑的旧结论**）、窗口沿用其既有动态（M2b 已实证能上屏）——**不得互相照搬**。
  > **本计划收尾时的实测（M3c）**：同一台 Intel 集显上**动态与静态各跑 30 帧结果相同**（各 93900 界面像素）；
  > 「声明动态却从不调 `vkCmdSetViewport`」会崩（窗口 `0xC000041D`、离屏 `0xC0000005` / 0-21 跑完）。
  > 但 M2a 记的是「无像素」而非崩溃 ⇒ **症状不同，不能断定同因**（定性：高度可能）；
  > **待办**：重跑当年的**离屏 + 三角形 + 动态**场景。详见 `docs/features/window.md` 第 5.2 节。
- `DEER_VK_VALIDATION=1` 下零校验消息**且**消息计数断言为 0（精确匹配 `^\[VK ERROR\]|^\[VALIDATION\]`）。
- 不跑 `cargo fmt`；**遇到红先 `cargo clean -p deer-vk`**（`git status` 干净 ≠ `target/` 干净）。
- 窗口相关验证**必须是 example**（winit 要求主线程，`#[test]` 跑不了，证据见 `examples/hal_window_path.rs:8-12`）。

---

### Task 1: 共用管线层（把两条管线状态从离屏提到共用）

**Files:** Create `crates/deer-vk/src/pipelines.rs`；Modify `crates/deer-vk/src/{gpu_render.rs,windowed.rs,lib.rs}`
**Interfaces — Produces:**
```rust
pub enum PipelineKind { Shape, Text }
pub struct PipelineSet { /* 两条管线 + 顶点布局 + 描述符布局 */ }
/// 只为**颜色附件格式**与 **viewport 策略**参数化；其余状态（混合/剔除/拓扑）两路径必须逐字相同。
pub fn build_pipelines(device: &VkDevice, render_pass: &RenderPass, color_format: i32,
                       viewport: ViewportStrategy) -> GpuResult<PipelineSet>;
pub enum ViewportStrategy { Static { extent: (u32, u32) }, Dynamic }
```
- **冻结**：混合状态 `SRC_ALPHA / ONE_MINUS_SRC_ALPHA`（color 与 alpha 同）、`cull_mode = NONE`、拓扑 `TRIANGLE_LIST`、文本采样 `NEAREST + ClampToEdge`、`mip_levels = 1`。
- [ ] Step 1：先写**纯函数**测试：断言两路径产出的管线状态描述符逐字段相同（除 color format 与 viewport 策略）——**无 GPU 也能跑**。
- [ ] Step 2：离屏路径改为调用它，**行为不得变**（M3a/M3b 的 207 条测试必须继续全绿）。
- [ ] Step 3：Commit。

### Task 2: **sRGB 交换链的像素语义**（本轮最大风险，先做）

**Files:** Modify `crates/deer-vk/src/pipelines.rs`、`crates/deer-vk/src/spirv.rs`（如需）、`crates/deer-vk/tests/gpu_vs_cpu.rs`
**背景（M2b 实测，必须继承）**：窗口交换链格式是 **`B8G8R8A8_SRGB`**（M2b 记录 `0x32`），而离屏路径用的是 `R8G8B8A8_UNORM`；M2b 实测**驱动会把清屏色做 sRGB 编码**（`rgb(0x10,0x14,0x24)` 回读成 `[71,79,105,255]`），当时用 `srgb_encoded_byte` 记账。
- **要求**：给出**明确且可复现**的语义选择，二选一并写进文档：
  (a) 片元输出**线性**值，接受硬件 sRGB 编码，并用**同一套换算**去和 CPU 字节对照（判据 = 换算后逐字节/≤1 LSB）；或
  (b) 让交换链用非 SRGB 格式（若该格式在目标设备上可作为呈现格式），从而与离屏路径同语义。
- 必须给**实测数字**与命令；**不许**「看起来颜色对」了事。
- [ ] Step 1：写**纯函数**换算测试（`srgb_encode`/`srgb_decode` 的往返与已知值，例如 0x10→71）。
- [ ] Step 2：在窗口路径上实测并记录数字。
- [ ] Step 3：Commit。

### Task 3: 窗口路径画真实界面树 + 呈现

**Files:** Modify `crates/deer-vk/src/windowed.rs`、`crates/deer-gui/examples/window_preview.rs`
**Interfaces — Produces:**
```rust
impl WindowedRenderer {
    /// 用同一套顶点流构造把 DrawList 画到交换链图像上并呈现。
    pub fn draw_and_present(&mut self, list: &DrawList, text: Option<&mut TextEngine>) -> GpuResult<FrameOutcome>;
}
```
- 每帧：`build_stream` / `build_text_stream` → 上传两块顶点缓冲 → 设 viewport/scissor（**与离屏同向**）→ 按 `DrawCall` 顺序切管线/缓冲/描述符 → 呈现。
- `window_preview` 改为渲染**真实界面树**（形状 + 文本，用 `deer-gui` 的门面产出 `DrawList`），并保留「跑 N 帧后退出」。
- [ ] Step 1：跑 `DEER_VK_FRAMES=30 cargo run --example window_preview` → 期望 `frames_presented > 0`、exit 0、校验层零消息。
- [ ] Step 2：**读回最后一帧**（`read_back_last_frame`）并与 CPU 后端对照，给实测最大通道差。
- [ ] Step 3：Commit。

### Task 4: 上屏判据固化为 example（不是 `#[test]`）

**Files:** Create `crates/deer-gui/examples/window_parity.rs`；Modify `docs/features/window.md`
- 门禁式 example：渲染固定界面树 → 读回 → 与 CPU 逐像素对照 → 打印最大差与结论；**默认跳过**（沿用 `DEER_VK_WINDOW_TESTS=1` 的既有开关），但跳过时**必须打印「这不是通过」**。
- [ ] Step 1：`DEER_VK_WINDOW_TESTS=1 DEER_VK_VALIDATION=1 cargo run --example window_parity` → exit 0 + 实测差。
- [ ] Step 2：Commit。

### Task 5: 文档

**Files:** Modify `FEATURES.md`、`ROADMAP.md`（M3c ⬜→✅）、`docs/features/{window,gpu-geometry}.md`、`docs/TUTORIAL.md`
- 「做不到」里删掉「窗口里仍只有清屏色 + M2a 几何」；**新增** sRGB 交换链的语义说明（Task 2 的结论）。
- **按新口径不写死测试条数**（远端已统一：只写命令与「以运行输出为准」）。
- [ ] Step 1：`docs_consistency` 5 passed + 全部示例 exit 0。

---

## Self-Review

**Spec 覆盖**：ROADMAP M3c 的「把 GPU 渲染器接上交换链/呈现」由 Task 1/3/4 覆盖；M2b 遗留的 `srgb_encoded_byte` 记账在 Task 2 正式化为**语义选择 + 判据**；示例与文档在 Task 4/5。
**未做且必须写明**：批处理优化、通用纹理、输入/焦点（M5）。
**Type consistency**：`PipelineSet` / `PipelineKind` / `DrawCall` 在 Task 1（产出）、Task 3（消费）一致；`TextEngine` 一律 `&mut`（M3b 已冻结）。
**风险登记**：① **sRGB 语义**（Task 2 先做，做不出结论就不动 Task 3）；② **viewport 朝向**导致 FragCoord 判据整片错位（Task 3 Step 2 的读回对照是唯一判据）；③ 交换链重建（`OutOfDate`）时顶点缓冲/管线不得失效。
