# 功能指南：Vulkan 图形管线（vulkan-pipeline）

> 状态 ✅（**管线能建成功，且已能在真机上画出正确像素**）·
> 示例 `cargo run -p deer-gui --example vulkan_pipeline` ·
> 清单条目见 [`FEATURES.md`](../../FEATURES.md)

## 1. 这是什么 / 什么时候用它

**从设备到「可用的图形管线」的那条路**：渲染通道 → 管线布局 → 着色器模块 →
`vkCreateGraphicsPipelines`。

什么时候用它：你想知道 GPU 那条路走到哪一步了、想自己接一个渲染后端、
或者想验证自研 SPIR-V 汇编器的产物是否真的被驱动编译。

**什么时候不能用它**：想看到像素 —— **现在还不行**。

| 能力 | 状态 |
|---|---|
| 枚举 GPU / 打开设备 | ✅ |
| 渲染通道 / 管线布局 / 图形管线 | ✅ **本页** |
| **画出像素**（命令缓冲 + 离屏图像 + 回读 + 绘制） | ✅ M2a-4..6 → 见 [`gpu-offscreen.md`](gpu-offscreen.md) |
| **渲染到窗口**（`VkSurfaceKHR` + 交换链 + 呈现） | ✅ **M2b** → 见 [`vulkan-swapchain.md`](vulkan-swapchain.md) |

## 2. 最小示例

```rust
use deer_gpu::Backend;          // trait 方法要引进作用域
use deer_vk::{ffi_dev as vk, spirv, VkDevice};

let dev = VkDevice::open(0)?;

// ① 渲染通道：RGBA8、每帧清屏、最终布局可回读
let pass = dev.create_render_pass(
    vk::VK_FORMAT_R8G8B8A8_UNORM,
    vk::VK_ATTACHMENT_LOAD_OP_CLEAR,
    vk::VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL,
)?;

// ② 管线布局：无推送常量、无描述符集（零资源管线）
let layout = dev.create_pipeline_layout(None)?;

// ③ 两个着色器（由自研 SPIR-V 汇编器生成）
let vs = dev.create_shader_module(&spirv::vertex_shader_triangle(
    [[-0.8, -0.8], [0.8, -0.8], [0.0, 0.8]],
))?;
let fs = dev.create_shader_module(&spirv::fragment_shader_solid([0.3, 0.7, 1.0, 1.0]))?;

// ④ 图形管线 —— 这一步才是 SPIR-V 的真正验收
let pipeline = dev.create_graphics_pipeline(&vs, &fs, &layout, &pass)?;
assert!(!pipeline.handle().is_null());
# Ok::<(), deer_gpu::GpuError>(())
```

完整可运行版：`cargo run -p deer-gui --example vulkan_pipeline`

## 3. 完整 API

| 方法 | 参数 | 说明 |
|---|---|---|
| `VkDevice::open(adapter_index)` | 适配器序号（`0` = 第一个） | 打开逻辑设备 + 图形队列 |
| `.create_render_pass(format, load_op, final_layout)` | 见下 | 单颜色附件、单子通道 |
| `.create_pipeline_layout(push_constant)` | `Option<(stage_flags, offset, size)>` | `None` = 无推送常量 |
| `.create_shader_module(&[u8])` | SPIR-V 字节 | 只做**结构护栏** + 建模块（驱动不编译） |
| `.create_graphics_pipeline(&vs, &fs, &layout, &pass)` | — | 顶点 + 片段两阶段 |
| `.create_graphics_pipeline_raw(&stages, &layout, &pass)` | 显式阶段列表 | 支持非「顶点+片段」组合 |
| `.wait_idle()` | — | 空闲等待 |

**`create_render_pass` 的三个参数**：

| 参数 | 常用值 | 含义 |
|---|---|---|
| `format` | 离屏 `VK_FORMAT_R8G8B8A8_UNORM`；**上屏优先线性 `*_UNORM`**（`B8G8R8A8_UNORM` → `R8G8B8A8_UNORM` → 才退 `_SRGB`，M3c 起；理由：sRGB 附件的混合在线性空间，与 CPU 字节空间差 44 字节） | 附件格式 |
| `load_op` | `VK_ATTACHMENT_LOAD_OP_CLEAR`（每帧清屏）/ `..._LOAD`（保留上一帧） | 开始时做什么 |
| `final_layout` | `VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL`（要回读）/ `..._PRESENT_SRC_KHR`（要呈现） | 结束时布局 |

**管线创建时的固定状态**（都写在 `device.rs` 里）：三角形列表、无顶点输入、
动态 viewport/scissor（避免改尺寸就重建管线）、无背面剔除、
标准 alpha 混合开（GUI 有半透明面板）。

**返回的句柄类型**（都实现了 `Drop`，会自动销毁）：
`RenderPass` / `PipelineLayout` / `Pipeline` / `ShaderModule`。

## 4. 自检

```rust
// ① 句柄非空 —— 驱动返回成功却不写输出参数时，这条会红（本页第 5 节有实测）
assert!(!pipeline.handle().is_null(), "管线句柄不能为空");

// ② 三种目标格式都应被支持（GUI 会用到不同格式）
for fmt in [vk::VK_FORMAT_R8G8B8A8_UNORM,
            vk::VK_FORMAT_R8G8B8A8_SRGB,
            vk::VK_FORMAT_B8G8R8A8_SRGB] {
    dev.create_render_pass(fmt, vk::VK_ATTACHMENT_LOAD_OP_CLEAR,
                           vk::VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL)?;
}

// ③ 推送常量大小必须是 4 的倍数 —— 我们会在**调用驱动前**拦下
let e = dev.create_pipeline_layout(Some((vk::VK_SHADER_STAGE_VERTEX_BIT, 0, 6)));
assert!(e.is_err());

// ④ 反复创建/销毁不能崩（验证 RAII Drop 顺序）
for _ in 0..5 {
    let pass = dev.create_render_pass(/* … */)?;
    let layout = dev.create_pipeline_layout(None)?;
    let vs = dev.create_shader_module(&spirv::vertex_shader_triangle([[0.0,0.0],[1.0,0.0],[0.0,1.0]]))?;
    let fs = dev.create_shader_module(&spirv::fragment_shader_solid([0.1,0.2,0.3,1.0]))?;
    let _p = dev.create_graphics_pipeline(&vs, &fs, &layout, &pass)?;
}
```

## 5. 常见坑（**这一节是血泪**）

| 现象 | 原因 | 怎么改 |
|---|---|---|
| `create_shader_module` 成功，但建管线崩（`0xc0000409`）/ 句柄为空 | **驱动建模块时只存字节，真正编译在建管线时**。SPIR-V 有问题要到这一步才暴露 | 先只用 `vertex_shader_empty` / `const_position` / `triangle` 这三支**已验证可用**的；改动着色器后务必建一次管线来验 |
| 用 `OpAccessChain` + **运行时索引**访问 `OpConstantComposite` 数组 | 本机驱动**直接崩**（`STATUS_STACK_BUFFER_OVERRUN`）。这就是崩溃的真凶 | 不要用常量数组做动态索引。改用 `OpCompositeExtract`（静态下标）或位运算算角点 |
| 推送常量块建管线失败 | **已知问题**：三种写法（`{vec4}`+Block+访问链 / 直接 load Block / 纯 `vec4`）分别导致「空句柄」或「访问违例」 | 暂时别用推送常量。矩形绘制后续改**顶点缓冲**方案（见 `spirv::vertex_shader_rect_pushconstant` 的文档） |
| 非法阶段配置（如重复 `VERTEX`）让进程崩 | 本机驱动对非法阶段配置**崩溃而不返回错误码** | 别用非法输入做校验测试；用**合法**输入 + 断言句柄非空 |
| `expect_err` 编译不过（`PipelineLayout doesn't implement Debug`） | `expect_err` 需要 `T: Debug` | 用 `match` 取错误 |
| 找不到 `open()` / `adapters()` | 忘了 `use deer_gpu::Backend;` | trait 方法必须引进作用域 |

## 6. 相关

- SPIR-V 汇编器与它的血泪史：[`vulkan.md`](vulkan.md)
- GPU HAL 契约：[`gpu-hal.md`](gpu-hal.md)
- 里程碑：[`../../ROADMAP.md`](../../ROADMAP.md)（M2a-4..6、M2b）
- **做不到**：画像素、窗口、推送常量、顶点缓冲、纹理、裁剪

## 7. 检查清单

- [x] 示例能跑：`cargo run -p deer-gui --example vulkan_pipeline` → `exit=0`
- [x] 示例有自检（建管线成功 + 如实打印边界）
- [x] `FEATURES.md` 已登记
- [x] `docs/TUTORIAL.md` 已包含（作为「GPU 现状」章节）
- [x] 明确写了「做不到什么」
