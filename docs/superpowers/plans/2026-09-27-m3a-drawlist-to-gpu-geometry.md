# M3a — DrawList 上 GPU（几何：矩形/描边/圆角/裁剪）与 CPU 逐像素对照

> **For agentic workers:** REQUIRED SUB-SKILL: Use subagent-driven-development (recommended) or executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让 `DrawCmd` 的**非文本**命令由 Vulkan 真正画出来，并且与 CPU 参考后端的像素结果一致（不透明绘制逐字节相同；半透明叠加最大通道差 ≤1）。

**Architecture:** 一条**静态 viewport/scissor** 的图形管线；每个顶点携带矩形元数据（`rect`、`radius_kind`、`color`），圆角/描边的**逐像素判据在片元着色器里复刻 CPU 的整数像素判据**（`gl_FragCoord` → `floor` 得到整数像素）；**裁剪在 CPU 侧做几何裁剪**（避开 M2a 实测的「动态 viewport/scissor 在本机 Intel 上画不出像素」）。文本（`DrawCmd::Text`）本计划**不实现**，命中即返回 `Unsupported`。

**Tech Stack:** Rust 2024 / 自研 SPIR-V 汇编器（`crates/deer-vk/src/spirv.rs`）/ `deer-vk` 既有离屏 image + staging 回读 / `deer_gpu::null::CpuRenderer` 作为像素基准。

**Spec:** `ROADMAP.md`（M3 行）、`FEATURES.md` 第四节、`crates/deer-gpu/src/draw.rs`（`DrawCmd` 契约）、`crates/deer-gpu/src/null.rs`（CPU 像素语义基准：`fill`/`inside_rounded`/`stroke`/`blend`）。

## Global Constraints

- **零新增第三方依赖**；`deer-vk` 保持零第三方依赖（窗口层的 winit 是已登记的唯一例外）。
- **CPU 后端是基准**：不得为了让对照通过而修改 `null.rs` 的像素语义；确需改动时先改基准并单独提交说明。
- 每处 `unsafe` 写 `// SAFETY:`；`cargo clippy --workspace --all-targets --features deer-gui/window` 必须 0 warning。
- 新增 SPIR-V 必须过官方 `spirv-val`（SDK 在 `C:\VulkanSDK\1.4.357.0`）**且**带 `DEER_VK_VALIDATION=1` 时建管线零校验消息。
- 不运行 `cargo fmt`（仓库非 fmt-clean）。命令一律 `cmd /c "cargo ..."`。
- 门禁三连：`cargo test --workspace`、`cargo clippy --workspace --all-targets --features deer-gui/window`、`DEER_VK_VALIDATION=1 cmd /c "cargo test -p deer-vk"`。

---

### Task 0: 隔离工作区

**Files:** 无（git 操作）

- [ ] **Step 1: 建工作区**（`using-git-worktrees` 的 fallback 路径）
```powershell
cd Z:\deer-gui
git worktree add ..\deer-gui-m3a -b feat/m3a-gpu-geometry
cd ..\deer-gui-m3a
```
- [ ] **Step 2: 验证基线绿**
Run: `cmd /c "cargo test --workspace"` → Expected: `203 passed / 0 failed`
- [ ] **Step 3: 提交锚点**
```powershell
git commit --allow-empty -m "chore(m3a): 从 master 切出 feat/m3a-gpu-geometry 工作区"
```

---

### Task 1: 汇编器补齐算子 + 两支新着色器

**Files:**
- Modify: `crates/deer-vk/src/spirv.rs`（新增常量与算子，文件末尾新增两支着色器）
- Modify: `crates/deer-vk/tests/export_spirv.rs`（把新着色器一并导出到 `spirv_probe/`）
- Test: `crates/deer-vk/tests/spirv_val.rs`（追加两支着色器的 `spirv-val` 用例）

**Interfaces:**
- Consumes: `Module`（`type_vector`/`variable`/`decorate`/`load`/`store`/`f_mul`/`f_add`/`f_sub`/`op_select`/`composite_extract`/`composite_construct`/`entry_point`/`execution_mode`/`finish`）、既有 `SC_INPUT`/`SC_OUTPUT`/`DECORATION_LOCATION`/`DECORATION_BUILT_IN`。
- Produces:
  - `pub const BUILTIN_FRAG_COORD: u32 = 15;`
  - `pub fn vertex_shader_rect_attrs() -> Vec<u8>`
  - `pub fn fragment_shader_rect_shape() -> Vec<u8>`

顶点布局（**写进计划，不允许实现时改**）：
| location | 类型 | 含义 |
|---|---|---|
| 0 | `vec2` | 位置（NDC，y 向下） |
| 1 | `vec4` | `rect = (x, y, w, h)`，像素单位 |
| 2 | `float` | `radius_kind`：`0` = 普通填充；`>0` = 圆角半径；`-1` = 描边 |
| 3 | `vec4` | 颜色（预乘不做，直接 src-alpha 混合） |

- [ ] **Step 1: 写失败测试**（`spirv_val.rs` 追加）
```rust
#[test]
fn rect_attrs_shaders_validate() {
    for (name, code) in [
        ("vs_rect_attrs", spirv::vertex_shader_rect_attrs()),
        ("fs_rect_shape", spirv::fragment_shader_rect_shape()),
    ] {
        // 先做纯字节段序自检（不依赖 SDK），再交给官方 spirv-val
        let words: Vec<u32> = code.chunks_exact(4)
            .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
        assert_eq!(words[0], 0x0723_0203, "{name}: magic");
        assert!(words[3] == 0, "{name}: bound 必须非 0 且 opcode 段序合法");
        crate::spirv_val_helpers::run_spirv_val(name, &code).expect("spirv-val 必须接受");
    }
}
```
Run: `cmd /c "cargo test -p deer-vk --test spirv_val rect_attrs_shaders_validate"` → Expected: FAIL（函数不存在）
- [ ] **Step 2: 加常量与算子**
```rust
pub const BUILTIN_FRAG_COORD: u32 = 15;
pub const OP_FLOOR: u32 = 8;
pub const OP_FORD_GREATER_THAN: u32 = 186;

impl Module {
    pub fn op_floor(&mut self, ty: u32, value: u32) -> u32 { /* 与 f_mul 同构：写 opcode+type+result+operand */ }
    pub fn op_ford_greater_than(&mut self, bool_ty: u32, a: u32, b: u32) -> u32 { /* opcode 186 */ }
}
```
- [ ] **Step 3: 写顶点着色器**（透传属性）
```rust
pub fn vertex_shader_rect_attrs() -> Vec<u8> {
    // in location0 vec2 pos → gl_Position = vec4(pos, 0, 1)
    // in location1 vec4 rect  → out location0
    // in location2 float rk   → out location1
    // in location3 vec4 col   → out location2
    // （实现方式照抄 vertex_shader_from_vertex_buffer 的写法，逐属性 load + store）
}
```
- [ ] **Step 4: 写片元着色器**（复刻 CPU 判据）
```rust
pub fn fragment_shader_rect_shape() -> Vec<u8> {
    // in vec4 gl_FragCoord (BuiltIn FragCoord)
    // in vec4 rect; in float rk; in vec4 color   (locations 0/1/2 与 VS 输出对齐)
    // px = floor(gl_FragCoord.x); py = floor(gl_FragCoord.y)
    // inside = (px >= rect.x) && (px < rect.x+rect.w) && (py >= rect.y) && (py < rect.y+rect.h)
    // if rk > 0: inside &= !(圆角判定: 四角 dx*dx+dy*dy > r*r，dx/dy 取整数像素到圆心距离)
    //           圆心 (rect.x + r, rect.y + r) 等，与 null.rs::inside_rounded 逐字对应
    // out_color = inside ? color : vec4(0.0)
}
```
> 判据必须与 `crates/deer-gpu/src/null.rs` 的 `inside_rounded`（四角 `dx*dx + dy*dy > r*r`，坐标取**整数像素**）逐字一致；描边（`rk == -1`）按 CPU `stroke()` 的 4 条 1px 边实现。
- [ ] **Step 5: 跑测试**
Run: `cmd /c "cargo test -p deer-vk --test spirv_val"` → Expected: PASS（含官方 spirv-val）
- [ ] **Step 6: 校验层下建管线不崩**
Run: `$env:DEER_VK_VALIDATION='1'; cmd /c "cargo test -p deer-vk --test pipeline_smoke"` → Expected: 全绿、零校验消息
- [ ] **Step 7: Commit**
```powershell
git add crates/deer-vk/src/spirv.rs crates/deer-vk/tests/spirv_val.rs crates/deer-vk/tests/export_spirv.rs
git commit -m "feat(deer-vk): 汇编器补 OpFloor/OpFOrdGreaterThan/gl_FragCoord + 矩形属性着色器（含 spirv-val）"
```

---

### Task 2: 纯逻辑 — DrawList → 顶点流（**无需 GPU**）

**Files:**
- Create: `crates/deer-vk/src/gpu_geom.rs`
- Modify: `crates/deer-vk/src/lib.rs`（`pub mod gpu_geom;`）
- Test: `crates/deer-vk/tests/gpu_geom_stream.rs`

**Interfaces:**
- Consumes: `deer_gpu::{DrawCmd, DrawList, RectI, Color, Extent}`。
- Produces:
```rust
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GpuVertex { pub pos: [f32; 2], pub rect: [f32; 4], pub radius_kind: f32, pub color: [f32; 4] }
#[derive(Debug, Clone, PartialEq)]
pub struct GpuStream { pub vertices: Vec<GpuVertex>, pub unsupported: Vec<String> }
pub const RADIUS_FILL: f32 = 0.0;
pub const RADIUS_STROKE: f32 = -1.0;
/// 把绘制列表翻译成顶点流；`extent` 用于像素→NDC 换算与裁剪。
pub fn build_stream(list: &DrawList, extent: Extent) -> GpuStream;
```

- [ ] **Step 1: 写失败测试**
```rust
#[test]
fn fill_rect_becomes_two_triangles_in_ndc() {
    let mut l = DrawList::new();
    l.push(DrawCmd::FillRect { rect: RectI::new(0, 0, 8, 4), color: Color::WHITE });
    let s = gpu_geom::build_stream(&l, Extent { width: 8, height: 4 });
    assert_eq!(s.vertices.len(), 6);
    // (0,0) 像素中心 → NDC (-1,-1)（y 向下）
    assert_eq!(s.vertices[0].pos, [-1.0, -1.0]);
    assert_eq!(s.vertices[0].rect, [0.0, 0.0, 8.0, 4.0]);
    assert_eq!(s.vertices[0].radius_kind, gpu_geom::RADIUS_FILL);
    assert!(s.unsupported.is_empty());
}
#[test]
fn text_is_reported_not_dropped() {
    let mut l = DrawList::new();
    l.push(DrawCmd::Text { rect: RectI::new(0,0,20,10), text: "hi".into(), color: Color::WHITE, size: 12.0, align: 0 });
    let s = gpu_geom::build_stream(&l, Extent { width: 32, height: 16 });
    assert_eq!(s.vertices.len(), 0);
    assert_eq!(s.unsupported.len(), 1, "文本必须被**报告**，不能静默丢弃");
}
#[test]
fn clip_is_intersected_on_the_cpu() {
    let mut l = DrawList::new();
    l.push(DrawCmd::PushClip { rect: RectI::new(2, 2, 4, 4) });
    l.push(DrawCmd::FillRect { rect: RectI::new(0, 0, 8, 8), color: Color::WHITE });
    l.push(DrawCmd::PopClip);
    let s = gpu_geom::build_stream(&l, Extent { width: 8, height: 8 });
    assert_eq!(s.vertices[0].rect, [2.0, 2.0, 4.0, 4.0], "矩形被裁到 clip 内");
}
```
Run: `cmd /c "cargo test -p deer-vk --test gpu_geom_stream"` → Expected: FAIL（模块不存在）
- [ ] **Step 2: 实现**：维护 clip 栈（初始 = 全画布）；`FillRect`/`FillRoundRect`/`StrokeRect` 各展开为 6 顶点矩形（`FillRoundRect` 带 `radius`，`StrokeRect` 带 `RADIUS_STROKE` 且几何按 CPU `stroke()` 的 4 条边展开为 4×6 顶点）；与 clip **求交**后若为空则整条跳过；`NodeHint` 忽略；`Text` 记入 `unsupported`。
- [ ] **Step 3: 跑测试** → `... gpu_geom_stream` → Expected: PASS
- [ ] **Step 4: Commit** → `feat(deer-vk): DrawList → GPU 顶点流（CPU 侧裁剪；文本显式 Unsupported）`

---

### Task 3: 离屏 GPU 几何渲染器

**Files:**
- Create: `crates/deer-vk/src/gpu_render.rs`
- Modify: `crates/deer-vk/src/device.rs`（新增 `create_vertex_pipeline(&self, stages, layout, render_pass, attrs: &[vk::VertexInputAttributeDescription], stride: u32) -> GpuResult<Pipeline>`，参照 `vbo_probe.rs:273-276` 的顶点输入写法，**静态 viewport/scissor**）
- Modify: `crates/deer-vk/src/lib.rs`（导出 `GpuGeometryRenderer`）
- Test: `crates/deer-vk/tests/gpu_vs_cpu.rs`（Task 4 使用）

**Interfaces:**
- Produces:
```rust
pub struct GpuGeometryRenderer { /* instance/device/render_pass/pipeline/image/framebuffer/vertex buffer/staging */ }
impl GpuGeometryRenderer {
    pub fn new(adapter_index: usize, extent: deer_gpu::Extent, clear: deer_gpu::Color) -> GpuResult<Self>;
    pub fn extent(&self) -> deer_gpu::Extent;
    /// 画一帧并回读 RGBA8（长度 = w*h*4）。文本命令会返回 `Unsupported`。
    pub fn render(&mut self, list: &DrawList) -> GpuResult<Vec<u8>>;
    pub fn unsupported(&self) -> &[String];
}
```
- [ ] **Step 1: 写失败测试**（`gpu_vs_cpu.rs` 里先写「单块不透明矩形 == CPU」）
- [ ] **Step 2: 实现**：`new` 建实例/设备（复用 `VkBackend` + `VkDevice::open`）、离屏 image + framebuffer（复用 `offscreen.rs` 的创建模式）、渲染通道（清屏色 + final layout = `TRANSFER_SRC_OPTIMAL`）、**静态 viewport/scissor = 全 extent**、顶点缓冲（`VK_BUFFER_USAGE_VERTEX_BUFFER_BIT` + 主机可见内存，每帧重传）；`render` 录制 `bind pipeline → bind vertex buffer → draw(vertices.len()) → copy image→buffer → 等栅栏 → map 读回`。
- [ ] **Step 3: 跑测试** → Expected: PASS
- [ ] **Step 4: 校验层**：`$env:DEER_VK_VALIDATION='1'; cmd /c "cargo test -p deer-vk --test gpu_vs_cpu"` → Expected: 零校验消息
- [ ] **Step 5: Commit** → `feat(deer-vk): 离屏 GPU 几何渲染器（静态管线 + 顶点缓冲 + 回读）`

---

### Task 4: 与 CPU 后端逐像素对照（本计划的**终局判据**）

**Files:**
- Test: `crates/deer-vk/tests/gpu_vs_cpu.rs`

**Interfaces:** Consumes `deer_gpu::null::{CpuRenderer, Framebuffer}`（`render(extent, &list, clear)`）、`GpuGeometryRenderer`。

- [ ] **Step 1: 写对照用例**（无 GPU 时 `eprintln!` 跳过并说明，不算通过）
```rust
fn assert_matches_cpu(list: &DrawList, w: u32, h: u32, clear: Color, max_diff: u8) {
    let cpu = CpuRenderer::new().render(Extent{width:w,height:h}, list, clear).unwrap();
    let mut gpu = GpuGeometryRenderer::new(0, Extent{width:w,height:h}, clear).unwrap();
    let gpu_px = gpu.render(list).unwrap();
    let mut worst = 0u8;
    for (a, b) in cpu.pixels.chunks_exact(4).zip(gpu_px.chunks_exact(4)) {
        for i in 0..4 { worst = worst.max(a[i].abs_diff(b[i])); }
    }
    assert!(worst <= max_diff, "最大通道差 {worst} 超过允许 {max_diff}");
}
#[test] fn opaque_fill_and_stroke_match_cpu_exactly() { /* max_diff = 0 */ }
#[test] fn rounded_rect_matches_cpu_exactly()      { /* max_diff = 0，含 4 个圆角 */ }
#[test] fn clip_matches_cpu_exactly()              { /* max_diff = 0 */ }
#[test] fn translucent_overlap_is_within_one_level() { /* max_diff = 1 */ }
```
- [ ] **Step 2: 跑** → Expected: 全绿（若圆角差 1 级，**先查是不是 `inside_rounded` 复刻错了**，不许直接放宽 `max_diff`）
- [ ] **Step 3: 变异验证判据有区分度**：临时把片元着色器的圆角判定改为「总是 inside」→ 圆角用例必须变红；改回。
- [ ] **Step 4: Commit** → `test(deer-vk): GPU 几何与 CPU 逐像素对照（不透明逐字节 / 半透明 ≤1）`

---

### Task 5: 文档 + 收尾

**Files:**
- Create: `docs/features/gpu-geometry.md`（照 `docs/features/TEMPLATE.md` 七节，含「做不到」）
- Modify: `FEATURES.md`（第三节加一行 ✅：GPU 几何渲染，指南 + 示例）、`ROADMAP.md`（M3 子步表）、`docs/TUTORIAL.md`（第 13 章：GPU 画界面）
- Create: `crates/deer-gui/examples/gpu_geometry.rs`（渲染一份不含文本的界面树 → PNG + 与 CPU 对照断言）

- [ ] Step 1: 写指南与示例 → `cmd /c "cargo run -q -p deer-gui --example gpu_geometry"` → exit=0
- [ ] Step 2: `cmd /c "cargo test -p deer-gui --test docs_consistency"` → 5 passed
- [ ] Step 3: 门禁三连全绿
- [ ] Step 4: Commit +（经你同意后）推送

---

## Self-Review

**Spec coverage:** ROADMAP M3 写的是「顶点/片段着色器、顶点缓冲、批处理、裁剪栈映射到 scissor、`DrawList` → GPU」。本计划覆盖：着色器（Task 1）、顶点缓冲（Task 3）、`DrawList`→GPU（Task 2/3）、裁剪（Task 2 的 CPU 侧几何裁剪，**刻意不用 scissor**，理由已写在 Architecture）、与 CPU 对照（Task 4）。**缺口（有意留下）**：批处理（当前每帧一个缓冲、一次 draw；批处理属性能优化，不影响正确性，留给 M3a+）、文本（M3b）、窗口呈现（M3c）—— 这三条不在本计划的 Goal 里，ROADMAP 会标注为未完成。

**Placeholder scan:** 无 TBD/TODO；每步要么给代码、要么给可复制的命令与期望输出。Task 1 Step 3/4 以「照抄 `vertex_shader_from_vertex_buffer` 的写法」给出实现路径并锁死了顶点布局表，未写整段 SPIR-V 生成代码——这是**刻意的**：那段代码必须由实现在 `spirv-val` 反馈下迭代，写死在计划里反而会误导。

**Type consistency:** `GpuVertex.radius_kind`（`RADIUS_FILL=0` / `RADIUS_STROKE=-1` / `>0` 半径）在 Task 1（着色器语义）、Task 2（生成）、Task 3（顶点属性）三处一致；`build_stream` 的返回类型 `GpuStream` 在 Task 2/3 一致；`GpuGeometryRenderer::render(&DrawList) -> GpuResult<Vec<u8>>` 在 Task 3/4/5 一致。
