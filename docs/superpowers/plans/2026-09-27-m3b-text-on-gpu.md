# M3b — 文本/字形上 GPU（字形图集纹理 + 文本管线），并修掉 `Text` 假阳性

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers `subagent-driven-development` 执行本计划（逐任务：新实现者 → 独立 reviewer → fix round）；DSH 侧用共享任务板 + `send_message`。

**Goal:** 让 `DrawCmd::Text` 也由 Vulkan 画出来，与 CPU 参考后端**在同一背景/颜色下逐像素对照**，并修掉 M3a defer 的假阳性（空串/零面积/被裁空的文本不该报错）。

**Architecture:** **新增第二条管线**（textured），而不是把文本塞进 M3a 的 shape 管线 —— 因为 `radius_kind` 的负值已被「描边带宽」占用（`-1.0` = 1px，见 `gpu_geom.rs:37,40,59-66`），硬塞会破坏已冻结的契约。文本管线顶点 = `pos(vec2) + uv(vec2) + color(vec4)`；字形覆盖率位图（`R8_UNORM`）整幅上传为纹理，采样用**最近邻**（与 CPU `draw_text_real` 的最近邻点采样一致）。z 序用**单次遍历 + 多段 draw call** 保持：形状与文本各自进一个顶点缓冲，但按 `DrawList` 顺序记录 `DrawCall{pipeline, first, count}`。

**Tech Stack:** Rust 2024；自研 SPIR-V 汇编器（`spirv.rs`，需新增 `OpImageSampleImplicitLod`/采样器/纹理类型 + `OpTypeImage`/`OpTypeSampledImage`）；手写 Vulkan 绑定（`ffi_dev.rs` 需补 image view / sampler / descriptor 相关函数指针）。

**Spec:** `docs/features/gpu-geometry.md`（M3a 的四条不可回退前提）、`ROADMAP.md` M3b 段、`crates/deer-gpu/src/null.rs:493-554`（`draw_text_real` = 文本的黄金基准）、`crates/deer-gpu/src/{text,glyph,atlas}.rs`。

## Global Constraints

- **零新增第三方依赖**；`deer-vk`/`deer-gpu` 保持零依赖。
- **CPU 后端是基准**：不得为让对照通过而改 `null.rs` 的像素语义。
- 静态 viewport/scissor（动态版在本机 Intel 核显上画不出像素，`gpu_render.rs:24-31`）；颜色附件 `R8G8B8A8_UNORM`（非 `_SRGB`）。
- 新增 SPIR-V 必须过官方 `spirv-val`；`DEER_VK_VALIDATION=1` 下零校验消息**且**消息计数断言为 0。
- 门禁：`cargo test --workspace`、`cargo clippy --workspace --all-targets --features deer-gui/window`、`docs_consistency`、示例 exit 0。
- 不跑 `cargo fmt`；**遇到 SPIR-V 相关红先 `cargo clean -p deer-vk`**（陈旧 target 会造成假红）。

---

### Task 1: 采样器与纹理 —— `device.rs` 补能力

**Files:** Modify `crates/deer-vk/src/device.rs`、`crates/deer-vk/src/ffi_dev.rs`
**Interfaces — Produces:**
```rust
pub struct Sampler { /* ... */ }                                   // 最近邻、ClampToEdge、无 mipmap
pub struct DescriptorSetLayout { /* ... */ }
pub struct DescriptorSet { /* ... */ }
pub fn create_sampler(&self) -> GpuResult<Sampler>;
pub fn create_texture_r8(&self, w: u32, h: u32, data: &[u8]) -> GpuResult<Texture>;  // R8_UNORM, 主机可见或 staging
pub fn update_descriptor_texture(&self, set: &DescriptorSet, tex: &Texture, s: &Sampler) -> GpuResult<()>;
```
- [ ] Step 1（失败测试）：`create_texture_r8` 尺寸校验（`w==0 || h==0` 报错、`data.len() != w*h` 报错）—— 照 T3 的 `validate_vertex_pipeline_args()` 模式抽**纯函数**校验 + 单测（无 GPU 也能跑）。
- [ ] Step 2：实现并在 `DEER_VK_VALIDATION=1` 下建纹理/视图/采样器/描述符集，零校验消息。
- [ ] Step 3：Commit。

### Task 2: 纹理采样着色器 —— `spirv.rs`

**Files:** Modify `crates/deer-vk/src/spirv.rs`、`tests/spirv_val.rs`、`tests/export_spirv.rs`
**Interfaces — Produces:** `pub fn vertex_shader_text() -> Vec<u8>`、`pub fn fragment_shader_text() -> Vec<u8>`
- 顶点：`in loc0 pos(vec2)` → `gl_Position`；`in loc1 uv(vec2)`、`in loc2 color(vec4)` 透传到片段。
- 片段：`out = vec4(color.rgb * cov, color.a * cov)`，其中 `cov = texture(tex, uv).r`；`OriginUpperLeft`。
- 需要的 SPIR-V 新算子：`OpTypeImage`/`OpTypeSampler`/`OpTypeSampledImage`/`OpTypeVoid` 已有、`OpLoad`/`OpImageSampleImplicitLod`/`OpCompositeExtract`/`OpVectorTimesScalar`；**若某算子缺失，照既有 `op_*` 模式补，并加常量单测**。
- [ ] Step 1（失败测试）：把两支着色器加进 `spirv_val.rs` 既有流程（**不要自造 helper 名**）。
- [ ] Step 2：实现；`cmd /c "cargo test -p deer-vk --test spirv_val"` 必须过（含官方 `spirv-val`）。
- [ ] Step 3：Commit。

### Task 3: 文本顶点流（纯逻辑，**无需 GPU**）

**Files:** Create `crates/deer-vk/src/gpu_text.rs`；Modify `crates/deer-vk/src/lib.rs`
**Interfaces — Consumes:** `deer_gpu::{DrawCmd, DrawList, Extent}`、`deer_gpu::text::TextEngine`。
**Produces:**
```rust
#[derive(Debug, Clone, Copy, PartialEq)] #[repr(C)]
pub struct TextVertex { pub pos: [f32; 2], pub uv: [f32; 2], pub color: [f32; 4] }   // stride 32
pub struct TextStream { pub vertices: Vec<TextVertex>, pub skipped: usize }
pub fn build_text_stream(list: &DrawList, extent: Extent, engine: &TextEngine) -> TextStream;
```
- **必须与 CPU `null.rs:493-554` 逐字一致**：`pen.round()`、基线 `rect.y + ((h-(asc+desc))/2).round() + ascent.round()`、逐字形 `advance`、**最近邻**采样、缺字走 `.notdef`。
- **假阳性修复（M3a defer 项）**：`text.is_empty()` / 与 clip 求交后为空 / `size <= 0` ⇒ **跳过该命令**（计入 `skipped`），**不报错**。
- [ ] Step 1（失败测试）：① 单字符四边形的**精确** pos/uv；② `size`/`align`/空串/裁空/缺字 五类边界；③ 与 CPU 侧同一 `TextEngine` 推出的期望值一致。
- [ ] Step 2：实现 → `cmd /c "cargo test -p deer-vk --test gpu_text_stream"` 绿。
- [ ] Step 3：Commit。

### Task 4: 集成进 `GpuGeometryRenderer`（两条管线、保 z 序）

**Files:** Modify `crates/deer-vk/src/gpu_render.rs`、`tests/gpu_vs_cpu.rs`
- `GpuGeometryRenderer::new(adapter, extent, clear)` 增加可选 `TextEngine`（`with_text(engine)`）；无引擎时 `Text` 仍报 `Unsupported`（保持 M3a 行为不变）。
- 单次遍历 `DrawList`，按顺序记录 `DrawCall{pipeline: Shape|Text, first, count}`；纹理只在图集变化时重传。
- [ ] Step 1：`Text` 现在**能画**（不再报错）；`unsupported` 字段语义改为「真正不支持的东西」（纹理/图像以外的未来命令）。
- [ ] Step 2：**逐像素对照**：不透明文本**目标为逐字节相同**；若实测差 ≤1 LSB 且原因是 UNORM 舍入，**必须贴出实测数字与原因**并据此定容差（照 M3a 处理 1 LSB 的方式，不许悄悄放宽）。
- [ ] Step 3：**变异验证**：把片段着色器的 `cov` 改成常量 `1.0` ⇒ 文本对照**必须变红** ⇒ 改回。
- [ ] Step 4：`$env:DEER_VK_VALIDATION='1'` 下零消息 + 消息计数断言 ==0。
- [ ] Step 5：Commit。

### Task 5: 文档与示例

**Files:** Modify `docs/features/gpu-geometry.md`、`FEATURES.md`、`ROADMAP.md`（M3b ⬜→✅）、`docs/TUTORIAL.md`、`crates/deer-gui/examples/gpu_geometry.rs`
- 把「做不到」里的**文本**条目改成「已支持（M3b）」，并把 M3a defer 的 `Text` 假阳性从限制里**移除**（已修）。
- 新增/扩展示例：GPU 渲染一份**含文本**的界面树 → PNG，并与 CPU 产物对照（SHA-256 相同或有据可查的容差）。
- [ ] Step 1：写文档 + 示例 → `docs_consistency` 5 passed、示例 exit 0。
- [ ] Step 2：Commit。

---

## Self-Review

**Spec 覆盖**：ROADMAP M3b 的「文本/字形上 GPU」由 Task 1–4 覆盖；M3a 遗留的 `Text` 假阳性由 Task 3 覆盖；纹理上传与采样属 M3b 必需（虽 ROADMAP 把「纹理」另列，但字形图集本身就是纹理 —— 本计划只做 `R8_UNORM` 覆盖度纹理，**通用图像纹理仍不在范围**，会在文档里写明）。
**未做且必须写明**：通用图像绘制、批处理优化、窗口呈现（M3c）。
**Type consistency**：`TextVertex` 的 `pos/uv/color` 在 Task 3（产出）、Task 4（绑定属性）、Task 2（着色器 location 0/1/2）三处一致；stride 32 = 8+8+16。
**风险登记**：① 自研汇编器首次涉及**采样**指令与描述符集，若 `spirv-val` 反复不过，允许把 Task 2 拆成「先出合法模块」与「再接描述符」两步并记账；② 文本逐字节一致可能达不到，届时按「实测数字 + 原因 + 明确容差」处理，**不许静默放宽**。
