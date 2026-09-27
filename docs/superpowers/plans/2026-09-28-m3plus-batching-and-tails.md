# M3+ 批处理（B）+ 清尾（C）计划

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers `subagent-driven-development`（实现者 → 独立 reviewer → fix round），DSH 侧用共享任务板 + `send_message`。

**Goal:** **B** = 在不改变任何一个像素的前提下，减少 GPU 路径的 draw call 数与每帧资源上传次数；**C** = 清掉 M3c 留下的两条尾巴：复现/推翻 M2a 原始症状（离屏+三角形+动态 viewport），并把「格式不一致被驱动静默接受」写进受跟踪文档。

**Architecture:**
- **B**：当前 `render`/`draw_and_present` 是「每帧两块顶点缓冲 + 逐 `DrawCall` 切管线/缓冲/描述符」。批处理分三层，**逐层独立可验收**：① **合段**（相邻同管线的段合成一次 `vkCmdDraw`）；② **跨帧复用缓冲**（按容量增长的持久缓冲 + ring，避免每帧重建/重传）；③ （若 ① 未达预期）**单缓冲两段式**（形状与文本各占一段，仍两次 draw，但只一次缓冲绑定/上传）。
- **C**：`deer-vk` 里做一次**对照实验**（离屏 + 三角形 + 动态 viewport，分别「每帧真的设置」与「声明了但从不设置」），逐帧记录；结论写进**受跟踪文档**。

**Tech Stack:** Rust 2024；自研 SPIR-V 汇编器；手写 Vulkan 绑定；winit 0.30。

**Spec:** `docs/features/gpu-geometry.md`、`docs/features/window.md`（§5 上屏路径）、`ROADMAP.md`（M3+ 行）、`crates/deer-vk/src/{gpu_render,windowed,pipelines}.rs`。

## Global Constraints

- **像素判据不得改变**：不透明**逐字节 0**、半透明 **≤1 LSB**（`gpu_vs_cpu` + `window_parity` 两处都要跑）。
- **验收用可计数的量**（draw call 数 / 每帧缓冲上传次数 / 管线切换次数 / 缓冲分配次数），**不用 fps**（本仓库早先吃过「不可复现的 fps 快照」的亏）；每个数字都要能由一条命令复现。
- 两档必须都跑：默认档 + **`DEER_VK_WINDOW_TESTS=1`**（跳过不算证据）。
- 零新增第三方依赖；不跑 `cargo fmt`；**先 `cargo clean -p deer-vk`**；不用 shell 文本管道改源码；提交用**显式路径**。
- `DEER_VK_VALIDATION=1` 下零校验消息，且**强证据取自 example / `--nocapture`**（`cargo test` 会捕获通过用例的输出，其 0 属弱证据）。

---

### Task B1: 度量基线（先量后改）

**Files:** Modify `crates/deer-vk/src/gpu_render.rs`、`crates/deer-vk/src/windowed.rs`
**Produces:** `pub fn render_stats(&self) -> RenderStats { draw_calls, pipeline_switches, buffer_uploads, buffer_allocations }`
- [ ] Step 1：加计数（放**被调用方**、与真实 Vulkan 调用同处 —— 本项目既有原则），并为**基线**记下一组数字（固定语料：`window_parity` 的界面树）。
- [ ] Step 2：报告基线数字 + 复现命令。Commit。

### Task B2: 合段（相邻同管线合成一次 draw）

**Files:** Modify `gpu_render.rs`、`windowed.rs`、`tests/gpu_vs_cpu.rs`
- [ ] Step 1（失败测试）：断言「语料里相邻同管线段被合并」——例如 `pipeline_switches` 从 N 降到 M（**给出确切数字**），且**像素判据不变**。
- [ ] Step 2：实现；两档门禁 + `window_parity` 复跑。Commit。

### Task B3: 跨帧复用缓冲（容量增长 + 不每帧重传）

- [ ] Step 1（失败测试）：断言「连续 N 帧、语料不变 ⇒ `buffer_allocations` 为 0 或 1、`buffer_uploads` 不再每帧 +2」。
- [ ] Step 2：实现（持久缓冲 + 按容量增长；写前等设备空闲或按槽同步，**沿用 `release_ui_resources` 的既有契约**）。Commit。

### Task B4: （可选）统一管线 / 单缓冲两段式

**本轮未做**（未达触发条件或未排期）—— 但**存在可行路径**，文档口径必须写成
「**本轮未做批处理的高级形态；存在可行路径（统一管线），留作后续**」：

- reviewer 已**按字面证伪**了旧说法「切换次数由 z 序决定，降低就只能重排（被禁止）」：
  把形状与文本**统一到一条管线**即可做到 **1 draw + 1 次管线切换**，且**不动一个像素**；
- 因此**不得**在文档里保留「不可能 / 只能靠重排」这类被证伪的表述。
- 若将来做 B4，判据同本计划 Global Constraints：像素判据不变（不透明 0 / 半透明 ≤1 LSB）+ 两档门禁都跑。

### Task C1: M2a 原始症状复现实验（离屏 + 三角形 + 动态 viewport）

**Files:** Create `crates/deer-vk/examples/viewport_dynamic_probe.rs`（gated）；Modify `docs/features/{gpu-geometry,vulkan}.md`
- 三组：① 动态 + **每帧真的 `vkCmdSetViewport/Scissor`**；② 动态 + **从不设置**（M2a 形态）；③ 静态（现状）。
- [ ] Step 1：逐组跑，记录「呈现成功/崩溃/零像素」+ 错误码 + 逐帧像素。
- [ ] Step 2：把结论写进**受跟踪文档**（口径：实现事实 + 证据 + **症状差异是否足以断定同因**；**不许**写「已证实/是误诊」，除非三组证据确实闭合）。
- [ ] Step 3：Commit。

### Task C2: 「格式不一致被静默接受」进受跟踪文档

**Files:** Modify `docs/features/*.md`（静默不一致清单）
- [ ] Step 1：把 `pipelines.rs` 里的实证搬成文档条目：**驱动不拦格式与渲染通道不一致，只能靠调用方自觉 + 像素对照**，并指向可重跑用例。Commit。

---

## Self-Review

**Spec 覆盖**：ROADMAP M3+ 的「批处理」= B1–B4；M3c 尾巴 O-1 = C1、O-3 = C2。
**未做且必须写明**：通用纹理、M5 输入与焦点。
**Type consistency**：`RenderStats` 字段在 B1（产出）、B2/B3（消费/断言）一致；`TextEngine` 一律 `&mut`。
**风险登记**：① 合段若改变了绘制顺序 ⇒ **像素判据会红**，那是正确信号（顺序即语义，M3b 已实证 z 序能被变异抓住）；② 跨帧复用缓冲若与 `FRAMES_IN_FLIGHT=2` 配合不当 ⇒ 沿用「先等设备空闲」的既有契约，别发明新的同步；③ C1 若复现出「动态零像素且不崩」⇒ **M2a 旧结论成立**，届时文档要改回强定性（以证据为准，不预设结论）。
