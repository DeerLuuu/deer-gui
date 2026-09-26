# 功能指南：GPU 离屏渲染与回读（gpu-offscreen）

> 状态 🔄（**清屏与回读可用；绘制有已知缺陷**）·
> 示例 `cargo run -p deer-gui --example gpu_offscreen` ·
> 清单条目见 [`FEATURES.md`](../../FEATURES.md)

## 1. 这是什么 / 什么时候用它

**把 GPU 渲染的结果读回成像素**：创建离屏图像 → 渲染通道 → 管线 → 命令缓冲 →
提交 → 栅栏等待 → `copyImageToBuffer` → map → RGBA8。

什么时候用它：
- 你想知道 GPU 那条路走到哪了；
- 你想验证「渲染结果是否与 CPU 后端一致」（这是 M2a 的终点目标）；
- 你在排查 GPU 渲染问题（本页第 5 节有一份完整的排查记录）。

## ⚠️ 先说结论：绘制有已知缺陷

| 环节 | 状态 |
|---|---|
| 命令缓冲录制 + 提交 + **栅栏 signal** | ✅ 正常（一帧约 `0.8–1.0 ms`） |
| 渲染通道的**清屏** | ✅ 正常（四种清屏色回读值**精确正确**） |
| **回读像素**（`copyImageToBuffer` + map） | ✅ 正常（长度与数值都对） |
| **`vkCmdDraw` 产生片元** | ❌ **一个像素都没有** |
| 把界面树渲染到 GPU | ❌ 要先修好绘制 |

**已排除的原因**（都实测过，不是推测）：

| 假设 | 实测结果 |
|---|---|
| 着色器内容有问题 | 空 `main` / 常量位置 / 常量数组+运行时索引（已修）/ `OpSelect` 全向量选择 —— **四种都不画** |
| 几何超出裁剪范围 | 顶点取 `(-3,-3) (3,-3) (0,3)` **铺满整屏**也不画 |
| 动态 viewport/scissor 的问题 | 改用**静态 viewport**写进管线，同样不画 |
| 清屏值或回读路径有问题 | 换清屏色，回读值跟着变 ⇒ 这条链是对的 |
| 管线没建成功 | 句柄非空，`vkCreateGraphicsPipelines` 返回成功 |

**下一步排查方向**（未做）：
1. `VK_LAYER_KHRONOS_validation` 校验层输出 —— 本机没装 Vulkan SDK，拿不到；
2. RenderDoc 抓帧看 draw call 的实际状态；
3. 逐项试管线状态变量（拓扑换 `POINT_LIST`、关掉 alpha 混合、`rasterizerDiscardEnable` 等）。

## 2. 最小示例

```rust
use deer_vk::{ffi_dev as vk, offscreen, spirv, VkDevice};

let dev = VkDevice::open(0)?;

// 渲染通道：RGBA8、每帧清屏、结束后可直接回读
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

// 离屏设施（图像 + 视图 + 帧缓冲 + 命令池 + 暂存缓冲）
let off = offscreen::offscreen_for(&dev, &pass, 96, 96)?;

// 渲染 → 提交 → 栅栏等待 → 回读 RGBA8
let pixels = off.render_and_read_back(&pass, &pipeline, 3, [0.1, 0.1, 0.2, 1.0])?;
assert_eq!(pixels.len(), 96 * 96 * 4);
# Ok::<(), deer_gpu::GpuError>(())
```

完整可运行版：`cargo run -p deer-gui --example gpu_offscreen`

## 3. 完整 API

| 函数 | 说明 |
|---|---|
| `offscreen::offscreen_for(&dev, &render_pass, w, h)` | 一步建好离屏设施 |
| `OffscreenRenderer::render_and_read_back(&pass, &pipeline, vertex_count, clear)` | 录制 + 提交 + 等栅栏（**1 秒超时**）+ 回读 → `Vec<u8>` |
| `.width()` / `.height()` | 图像尺寸 |

**返回的像素格式**：`R8G8B8A8_UNORM` ⇒ 字节顺序 **R, G, B, A**，
行优先、**无 padding**。下标公式：`(y * width + x) * 4`。

> ⚠️ 若换成 `B8G8R8A8`，红蓝会互换 —— 这是极容易踩的坑。

**RAII 句柄**（都实现了 `Drop`）：`Image` / `ImageView` / `Framebuffer` /
`CommandPool` / `Buffer` / `Memory` / `Fence`。

**栅栏超时是有限的（1 秒）**：驱动出问题时宁可失败，也不要永久挂住 ——
测试挂死比测试失败难查得多。

## 4. 自检

```rust
// ① 回读长度必须精确
assert_eq!(pixels.len(), (w as usize) * (h as usize) * 4);

// ② 清屏色回读值必须精确（这条链是通的，所以严判）
for (clear, expect) in [([1.0,0.0,0.0,1.0], [255,0,0,255]),
                        ([0.0,0.0,1.0,1.0], [0,0,255,255])] {
    let px = off.render_and_read_back(&pass, &pipeline, 3, clear)?;
    assert_eq!(&px[0..4], &expect);
}

// ③ 确定性：连续几帧必须逐字节相同
let a = off.render_and_read_back(&pass, &pipeline, 3, clear)?;
let b = off.render_and_read_back(&pass, &pipeline, 3, clear)?;
assert_eq!(a, b);

// ④ 非法参数必须返回错误，不许崩
assert!(offscreen::offscreen_for(&dev, &pass, 0, 64).is_err());
assert!(off.render_and_read_back(&pass, &pipeline, 0, clear).is_err());
```

## 5. 常见坑

| 现象 | 原因 | 怎么改 |
|---|---|---|
| **画不出任何几何** | **已知缺陷**（见本页开头）。清屏与回读是通的 | 要出图先用 CPU 后端；或按上面三个方向排查 |
| 清屏色对了，但 `vertex_count` 传错 | 传 `0` 会被我们拦下返回错误 | 传实际顶点数（一个三角形 = 3） |
| 红蓝互换 | 用了 `B8G8R8A8` 格式 | 用 `R8G8B8A8_UNORM`，或自己换字节序 |
| 测试永久挂住 | 栅栏没被 signal（驱动问题） | 我们已用 **1 秒有限超时**；若你改了，务必保留有限超时 |
| 图像有 padding、下标算错 | `bufferRowLength` 传了非 0 值 | 保持 `0`（表示与图像宽度一致） |
| `expect_err` 编译不过 | 那些 RAII 类型没实现 `Debug` | 用 `match` 取错误 |

## 6. 相关

- 图形管线：[`vulkan-pipeline.md`](vulkan-pipeline.md)
- SPIR-V 汇编器：[`vulkan.md`](vulkan.md)
- 已知缺陷的回归测试：`crates/deer-vk/tests/offscreen_render.rs`
  （`draw_produces_no_pixels_is_a_known_defect` —— **修好后它会变红**，提醒更新文档）
- **做不到**：绘制几何、把 `DrawList` 渲染到 GPU、窗口呈现、抗锯齿

## 7. 检查清单

- [x] 示例能跑：`cargo run -p deer-gui --example gpu_offscreen` → `exit=0`
- [x] 示例有自检（回读长度 + 如实报告绘制缺陷）
- [x] `FEATURES.md` 已登记
- [x] `docs/TUTORIAL.md` 已包含（作为「GPU 现状」章节）
- [x] 明确写了「做不到什么」与**已知缺陷的排查记录**
