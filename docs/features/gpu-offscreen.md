# 功能指南：GPU 离屏渲染与回读（gpu-offscreen）

> 状态 ✅ **可用**（绘制缺陷已修复）·
> 示例 `cargo run -p deer-gui --example gpu_offscreen` ·
> 清单条目见 [`FEATURES.md`](../../FEATURES.md)

## 1. 这是什么 / 什么时候用它

**把 GPU 渲染的结果读回成像素**：创建离屏图像 → 渲染通道 → 管线 → 命令缓冲 →
提交 → 栅栏等待 → `copyImageToBuffer` → map → RGBA8。

什么时候用它：
- 你想在**不依赖窗口**的情况下验证 GPU 渲染（CI 也能跑）；
- 你想做「GPU 输出 vs CPU 后端输出」的像素级对照；
- 你在排查 GPU 渲染问题（第 5 节有一份完整的排查方法论）。

**能力边界**：

| 环节 | 状态 |
|---|---|
| 命令缓冲录制 + 提交 + 栅栏 signal | ✅（一帧约 1 ms） |
| 渲染通道清屏 | ✅（四种清屏色回读值精确正确） |
| 回读像素（`copyImageToBuffer` + map） | ✅ |
| **`vkCmdDraw` 绘制几何** | ✅ **（曾经不通，根因已找到并修复）** |
| 把界面树（`DrawList`）渲染到 GPU | ❌ 需要 M3 的 GPU 渲染器 |
| 渲染到窗口（`VkSurfaceKHR` + 交换链 + 呈现） | ✅ **M2b 已打通**（见 [`vulkan-swapchain.md`](vulkan-swapchain.md)）；本节讲的是**离屏**回读路径 |

## 2. 最小示例

```rust
use deer_vk::{ffi_dev as vk, offscreen, spirv, VkDevice};

let dev = VkDevice::open(0)?;

let pass = dev.create_render_pass(
    vk::VK_FORMAT_R8G8B8A8_UNORM,
    vk::VK_ATTACHMENT_LOAD_OP_CLEAR,
    vk::VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL,
)?;
let layout = dev.create_pipeline_layout(None)?;
let vs = dev.create_shader_module(&spirv::vertex_shader_triangle(
    [[-0.8, -0.8], [0.8, -0.8], [-0.8, 0.8]],
))?;
let fs = dev.create_shader_module(&spirv::fragment_shader_solid([0.0, 1.0, 0.0, 1.0]))?;
let pipeline = dev.create_graphics_pipeline(&vs, &fs, &layout, &pass)?;

let off = offscreen::offscreen_for(&dev, &pass, 96, 96)?;
let pixels = off.render_and_read_back(&pass, &pipeline, 3, [0.1, 0.1, 0.2, 1.0])?;
assert_eq!(pixels.len(), 96 * 96 * 4);
# Ok::<(), deer_gpu::GpuError>(())
```

完整可运行版：`cargo run -p deer-gui --example gpu_offscreen`
（会写出 `render_out/gpu_offscreen.png`，可以直接打开看）

## 3. 完整 API

| 函数 | 说明 |
|---|---|
| `offscreen::offscreen_for(&dev, &render_pass, w, h)` | 一步建好离屏设施 |
| `OffscreenRenderer::render_and_read_back(&pass, &pipeline, vertex_count, clear)` | 录制 + 提交 + 等栅栏（**1 秒超时**）+ 回读 → `Vec<u8>` |
| `.width()` / `.height()` | 图像尺寸 |

**返回的像素格式**：`R8G8B8A8_UNORM` ⇒ 字节顺序 **R, G, B, A**、行优先、无 padding。
下标公式：`(y * width + x) * 4`。

> ⚠️ 若换成 `B8G8R8A8`，红蓝会互换 —— 极容易踩。

**NDC 与屏幕方向**：本项目把 NDC 坐标**原样**写入 `gl_Position`。
Vulkan 的 NDC 是 **y 向下**（与 OpenGL 相反），所以顶点 `y = -0.8` 在屏幕**上方**。
写位置断言时务必按这个约定（我在这里错过两次）。

**RAII 句柄**（都实现 `Drop`）：`Image` / `ImageView` / `Framebuffer` /
`CommandPool` / `Buffer` / `Memory` / `Fence`。

## 4. 自检

```rust
// ① 回读长度必须精确
assert_eq!(pixels.len(), (w as usize) * (h as usize) * 4);

// ② 清屏色回读值必须精确
for (clear, expect) in [([1.0,0.0,0.0,1.0], [255,0,0,255]),
                        ([0.0,0.0,1.0,1.0], [0,0,255,255])] {
    let px = off.render_and_read_back(&pass, &pipeline, 3, clear)?;
    assert_eq!(&px[0..4], &expect);
}

// ③ **几何必须真的被画出来**（这是回归判据）
let green = pixels.chunks_exact(4).filter(|p| p[1] > 200 && p[0] < 50).count();
assert!(green > 0, "vkCmdDraw 没产生像素 —— 先跑 spirv_val 测试");

// ④ 面积必须符合几何：两直角边各 0.8 屏宽 ⇒ 约 32%
let ratio = green as f64 / (w * h) as f64;
assert!((0.25..0.40).contains(&ratio));

// ⑤ 确定性：连续几帧逐字节相同
let a = off.render_and_read_back(&pass, &pipeline, 3, clear)?;
let b = off.render_and_read_back(&pass, &pipeline, 3, clear)?;
assert_eq!(a, b);
```

## 5. 曾经的「驱动不报错也不画」缺陷 —— 完整排查记录

这是本项目**最难定位**的一个缺陷，过程值得记录。

### 症状

`vkCmdDraw` **一个像素都不产生**，而所有 Vulkan API 都返回成功：

| 环节 | 现象 |
|---|---|
| `vkCreateShaderModule` | ✅ 接受（它只存字节，**不编译**） |
| `vkCreateGraphicsPipelines` | ✅ 返回成功、句柄非空 |
| `vkCmdDraw` | ❌ **静默不产生任何片元** |

### 走过弯路（都被排除）

着色器内容、几何裁剪范围、动态 vs 静态 viewport、alpha 混合开关、
清屏与回读路径、管线是否建成功 —— **八类假设全部排除**。

> **补记（M3c，2026-09-27）**：上面「动态 vs 静态 viewport」这条假设当年被排除是对的；
> 但后来文档里演化出的一句结论「**动态 viewport 在本机 Intel 上画不出像素**」目前应标为
> **存疑（未证实）**：对照实验显示同一台 Intel 集显上**动态与静态各跑 30 帧结果完全相同**
> （各 93900 界面像素），而「**声明**动态却**从不调** `vkCmdSetViewport`」会让进程**崩溃**
> （窗口 `0xC000041D`；**离屏 `0xC0000005`，0/21 跑完**）。
> **caveat**：M2a 当年记的症状是「无像素」而非崩溃，**症状不同 ⇒ 不能断定同因**（定性：高度可能）。
> **待办**：重跑当年的**离屏 + 三角形 + 动态状态**场景逐格记录，才能钉死或钉倒这条旧结论。
> 详见 [`window.md`](window.md) 第 5.2 节与 [`gpu-geometry.md`](gpu-geometry.md) 第 3 节前提 1。

两个关键实验把范围收敛了：
- **手写裸 FFI 渲染路径**（`tests/raw_ffi_probe.rs`）也画不出 ⇒ **不是封装的 bug**；
- **真实顶点缓冲 + 顶点属性**（`tests/vbo_probe.rs`）也画不出 ⇒ **不是顶点来源的问题**。

### 真正的根因：SPIR-V 段序

装上 Vulkan SDK、用官方 `spirv-val` 一跑：

```text
error: EntryPoint is in an invalid layout section
```

**我的 SPIR-V 汇编器有两个段序缺陷**：

1. `OpEntryPoint` 被排在**类型/常量之后** ⇒ 整份模块的段全部错位；
2. `OpFunction` 没有映射到函数段 ⇒ 掉进 `_ => TypeConstGlobal`，函数头排进了类型段。

SPIR-V 的逻辑布局段顺序是**规范强制**的（`OpEntryPoint` 必须在类型之前，
`OpFunction` 是「图定义段」的终止者）。驱动对这两者**既不报错也不画**。

**修复**：`spirv.rs` 改成**按段累积**，`finish()` 时按规范顺序拼接；
路由由 `section_of_opcode` + 「是否在函数体内」共同决定。
详见该文件里 `Section` 的文档与 `assemble()` 的注释（那里记了两次踩坑经历）。

### 留下的防线

- **`tests/spirv_val.rs`**：用官方 `spirv-val` 校验全部 10 支着色器
  （找不到 `spirv-val` 时明确跳过，不伪装通过）；
- 同一个文件里还有**纯字节段序检查**（不依赖 SDK，任何机器都能跑）；
- `tests/raw_ffi_probe.rs` / `vbo_probe.rs` / `offscreen_render.rs`
  都断言**像素数量与位置**，而不只是「跑通了」。

### 附带修掉的两个真 bug

排查中读官方头文件发现我自己写错了一批**从记忆里来的常量**：

| 项 | 我写的 | 正确 |
|---|---|---|
| `VK_STRUCTURE_TYPE_DEBUG_UTILS_MESSENGER_CREATE_INFO_EXT` | 1000128001 | **1000128004** |
| `DEBUG_UTILS_MESSAGE_TYPE_VALIDATION_BIT_EXT` | 0x10 | **0x02** |
| `DEBUG_UTILS_MESSAGE_TYPE_PERFORMANCE_BIT_EXT` | 0x100 | **0x04** |

教训：**扩展的 `sType` 与位标志值不要凭记忆写**，从 SDK 头文件核对。
（其余 27 个 `sType` 已逐个核对，全部正确。）

## 6. 相关

- 图形管线：[`vulkan-pipeline.md`](vulkan-pipeline.md)
- SPIR-V 汇编器：[`vulkan.md`](vulkan.md)
- GPU HAL：[`gpu-hal.md`](gpu-hal.md)
- **做不到**：把 `DrawList` 渲染到 GPU（M3）、推送常量（Intel 上不可用）；窗口呈现**已在 M2b 打通**（见 [`window.md`](window.md)、[`vulkan-swapchain.md`](vulkan-swapchain.md)），但那不属于本节这条离屏路径

## 7. 检查清单

- [x] 示例能跑：`cargo run -p deer-gui --example gpu_offscreen` → `exit=0`
- [x] 示例有自检（像素数量 + 位置 + 写出 PNG 便于肉眼看）
- [x] `FEATURES.md` 已登记
- [x] `docs/TUTORIAL.md` 已包含（作为「GPU 现状」章节）
- [x] 明确写了「做不到什么」与**排查方法论的完整记录**
