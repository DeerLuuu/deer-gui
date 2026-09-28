# B 高级形态：统一两条管线（1 draw + 1 switch，不动一个像素）

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers `subagent-driven-development`（实现者 → 独立 reviewer → fix round）；DSH 侧用共享任务板 + `send_message`。

**Goal:** 把形状管线与文本管线**合成一条**，使「形状/文本交错」的语料从 **8 draw + 8 switch 降到 1 draw + 1 switch**，且**像素判据一字不变**（不透明逐字节 0 / 半透明 ≤1 LSB）。

**Architecture（接口已冻结，两边照此实现）:**
- **统一顶点**：`UnifiedVertex { pos: [f32;2], rect: [f32;4], radius_kind: f32, color: [f32;4], uv: [f32;2] }`，`#[repr(C)]`，**stride 52**，偏移 **pos 0 / rect 8 / radius_kind 24 / color 28 / uv 44**。
- **判别符**：**`uv.x < 0.0` ⇒ 形状**（文本 uv 恒 ≥ 0）。**不新增属性、不用控制流**。
- **统一片元着色器**（无分支，全用 `OpSelect`）：
  ```
  shape_mask = <现有 frag 判据，基于 rect 属性与整数像素>
  shape_out  = select(shape_mask, color, vec4(0.0))
  cov_text   = texture(tex, uv).r                    // R8_UNORM, NEAREST, ClampToEdge
  text_out   = vec4(color.rgb, color.a * cov_text)   // 非预乘（配 SRC_ALPHA）
  is_text    = uv.x < 0.0 ? false : true             // 用 OpFOrdLessThan 得到
  out        = select(is_text, text_out, shape_out)
  ```
- **哑纹理**：统一 FS **无条件采样** ⇒ 形状帧也必须绑一张纹理 ⇒ 无 `TextEngine` 或该帧无文本时绑 **1×1 全覆盖度（cov=1）** 的 `R8_UNORM` 纹理。
- **顶点转换**：现有 `gpu_geom::build_stream` / `gpu_text::build_text_stream` 的**契约不动**（它们各自产 44/32 字节顶点）；在绘制路径里做一次**纯函数转换**成统一顶点（形状填 `uv = (-1.0, -1.0)`）。

**Spec:** `docs/superpowers/plans/2026-09-28-m3plus-batching-and-tails.md`、`docs/features/gpu-geometry.md`（§6.2 判据强度）、`docs/features/vulkan.md`（§6 静默不一致）。

## Global Constraints

- **像素判据不得改变**：不透明**逐字节 0**、半透明 **≤1 LSB**（`gpu_vs_cpu` + `window_parity` 两处都跑，`FRAMES=1/2/3/30` 数字需一致）。
- **验收用可计数的量**（draw call / switch / 上传 / 分配），**不用 fps**；并**新增一个 CPU 侧量**（顶点转换耗时可忽略但要有计数或断言，见 B5-2）。
- 计数与断言**与真实 Vulkan 调用同处**（删发射必删计数）。
- **两档都跑**：默认 + `DEER_VK_WINDOW_TESTS=1`；`DEER_VK_VALIDATION=1` 精确匹配 `^\[VK ERROR\]|^\[VALIDATION\]`，**强证据取自 example / `--nocapture`**。
- **旧行为**：无 `TextEngine` 时文本仍报 `Unsupported`；`release_ui_resources` / `resize` 语义不变（含「**任何主动释放先等设备空闲**」）。
- 先 `cargo clean -p deer-vk`；不跑 `cargo fmt`；不用 shell 文本管道改源码；提交用**显式路径**。

---

### Task B5-1（shader 侧）：统一着色器对

**Files:** Modify `crates/deer-vk/src/spirv.rs`；`tests/{spirv_val.rs,export_spirv.rs}`
**Produces:** `pub fn vertex_shader_unified() -> Vec<u8>`、`pub fn fragment_shader_unified() -> Vec<u8>`
- [ ] Step 1：把两支加进 `spirv_val.rs` **既有**流程（不自造 helper 名），过官方 `spirv-val`。
- [ ] Step 2：**接口一致性护栏**：解 `OpDecorate(Location)`，断言「VS 输出 ⊇ FS 输入」且**恰好 5 个 location**（0..4）——照 `shape_pipeline_shader_interface_matches` 的既有做法。
- [ ] Step 3：**非预乘回归锁**（M3b 的教训：`spirv-val` 放行预乘）⇒ 断言 alpha 是 `OpFMul(cov, color.a)` 且 rgb 不被任何 `OpFMul` 消费。
- [ ] Step 4：Commit。

### Task B5-2（绘制侧）：统一顶点 + 转换 + 单管线绘制

**Files:** Create `crates/deer-vk/src/vertex_unify.rs`；Modify `crates/deer-vk/src/{gpu_render.rs,windowed.rs,lib.rs}`；`tests/{gpu_vs_cpu.rs,gpu_text_reupload.rs}`
- [ ] Step 1（纯逻辑失败测试）：`unify(shape_vertices, text_vertices, segments) -> Vec<UnifiedVertex>`：断言 stride 52、偏移、形状段 `uv = (-1,-1)`、文本段 uv 保留、**段的相对顺序不变**（z 序）。变异：把顺序打乱 ⇒ 必须红。
- [ ] Step 2：接进两条路径，**只有一条管线、一个顶点缓冲、一次 draw**（无 `TextEngine` 时绑 1×1 哑纹理；仍报 `Unsupported` 的语义不变）。
- [ ] Step 3（度量）：断言语料 `opaque-ui-tree` 的 **draw_calls == 1 且 pipeline_switches == 1**（基线是 8/8），且**像素判据不变**。
- [ ] Step 4（变异）：把 `uv.x < 0` 的判别符改成「总是形状」/「总是文本」⇒ **像素必须红**；把哑纹理换成 cov=0 的纹理 ⇒ **形状帧必须红**（证明哑纹理真的被用上）。
- [ ] Step 5：两档 + 校验层 + clippy。Commit。

### Task B5-3：旧路清理与文档

- 若统一后 `pipelines.rs` 的形状/文本两条管线仍有调用点（例如 `window_preview` 旧路径），**逐个确认**；无用则删除（**不要留死代码**），并在报告里列出删除清单。
- 文档：`ROADMAP.md` M3+ 行、`docs/features/gpu-geometry.md` 的「仍未做」段 ⇒ 更新为「**统一管线已做**；仍未做 = 其它高级形态」；口径照旧（实现事实 + 收益 + 边界、不写死数字、给可复现命令）。
- [ ] Commit。

---

## B5-2 执行要点（实现者交接，落盘以免只活在会话里）

> 来源：`impl-geom` 在 Step 1（`15757ab`）后因预算耗尽交回的可执行交接。**Step 1 的变异红/绿（打乱段顺序）仍未跑**，与本清单的变异 C 是同一件事，一起做。

1. **两条渲染路径都改成**：`unify(shape_verts, text_verts, segments)` → **一个** `UnifiedVertex` 缓冲 → **一条**管线（`vertex_shader_unified` / `fragment_shader_unified`，5 个 location、stride 52）→ **一次** `vkCmdDraw(first=0, count=全部)`。
2. **哑纹理必须有**：统一 FS 无条件 `OpImageSampleImplicitLod` ⇒ 建 **1×1、`cov=1` 的 `R8_UNORM`**（`device.create_texture_r8(1,1,&[255])`）；采样器 `create_sampler` 已是 **NEAREST + ClampToEdge** ✓（形状段 `uv=(-1,-1)` 的越界采样必须被 Clamp）。无 `TextEngine` 或该帧无文本时绑它；**无 `TextEngine` 时文本仍报 `Unsupported`**（契约不变）。
3. **不动**：顶点缓冲/上传/屏障/`release_ui_resources`（先 `vkDeviceWaitIdle`）/`resize` 语义；只把「两块缓冲 + 两段式切管线」换成「一块 + 一管线 + 一次 draw」。

**四条变异 ↔ 对应判据（都要红/绿两次）**

| 变异 | 做法 | 必须变红的判据 |
|---|---|---|
| **A** 判别符恒真/恒假 | 改 `unify` 里形状段填 `(-1,-1)` 处，或改 `is_shape` 阈值 | `gpu_vs_cpu` 的**不透明逐字节 0** 用例 |
| **B** 哑纹理 `cov=0` | 传 `&[0]` | **形状帧必红**（证明哑纹理**承重**）⚠️ **这条期望是错的，已作废**：实测 `[0]`/`[128]` 形状帧**逐字节不变**（采样结果被 `OpSelect` 丢弃）。承重的是「**绑**一个已写入的有效描述符」，判据是删 `cmd_bind_descriptor_sets` / 跳过 `update_descriptor_texture` ⇒ 默认档 18 failed / 9 failed（读回**整幅 0**）。详见 `crates/deer-vk/src/gpu_render.rs::DUMMY_COVERAGE` 与 `.superpowers/sdd/b5-final-review.md` F-1/F-2 |
| **C** 打乱段顺序 | `unify` 里重排 | `window_parity` / `gpu_vs_cpu` 的 **z 序重叠语料**用例（同时补 Step 1 欠的那次） |
| **D** 形状段 `uv` 填 `(0,0)` | 违反「判别符是隐式契约」 | **必须红**；**若不变红即 finding**（该契约无判据覆盖），照实登记 |

**度量**：`opaque-ui-tree` 断言 `draw_calls == 1 && pipeline_switches == 1`（基线 **8/8**）；像素不透明 **0** / 半透明 **≤1 LSB**（`gpu_vs_cpu` + `window_parity`，`FRAMES=1/2/3/30` 数字一致）；**CPU 转换成本**报「`unify` 输出顶点数（= 段表和）+ 调用次数」。

---

## Self-Review

**Spec 覆盖**：reviewer 指出的可行路径（统一两条管线 ⇒ 1 draw + 1 switch）由 B5-1/B5-2 覆盖；旧路清理与文档由 B5-3。
**未做且必须写明**：其它高级形态（多帧环形缓冲、间接绘制）与 M5 输入焦点。
**Type consistency**：`UnifiedVertex` 的 5 个 location 与 B5-1 的着色器 loc0..4 一致；stride 52 = 8+16+4+16+8。
**风险登记**：① **哑纹理**是统一方案的隐式依赖（形状帧也采样）——必须用变异证明它承重；② 统一 FS 的 `select` 复用两条既有数学，**任何一处抄错都会在像素对照上立刻显形**（这是好事）；③ 顶点转换的 CPU 成本要**计入度量**，不许只说「可忽略」。
