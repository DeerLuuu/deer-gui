# 功能指南：Vulkan 后端（vulkan）

> 状态 🔄 **设备 + 管线 + 离屏 + 上屏都通了；还没有「把 `DrawList` 送上 GPU」** ·
> 示例 `cargo run -p deer-gui --example vulkan_devices` ·
> 清单条目见 [`FEATURES.md`](../../FEATURES.md)

## 1. 这是什么 / 现在能做到哪一步

`deer-vk` 是**自己写的** Vulkan 后端：不依赖 `ash` / `vulkano` / `wgpu`，
Vulkan 符号自己声明、运行时动态加载。

**现在的真实边界（务必先读）**：

| 能力 | 状态 |
|---|---|
| 枚举物理设备（拿到显卡名/类型/版本） | ✅ |
| 打开逻辑设备 + 图形队列 | ✅ |
| 自研 SPIR-V 汇编器的产物被驱动接受 | ✅ |
| **创建管线**（`vkCreateGraphicsPipelines`） | ✅ **M2a-3** |
| **渲染出图**（渲染通道 + 命令缓冲 + 回读 + 绘制几何） | ✅ **M2a-4..6** → 见 [`gpu-offscreen.md`](gpu-offscreen.md) |
| **渲染到窗口**（`VkSurfaceKHR` + 交换链 + 帧同步 + 呈现） | ✅ **M2b** → 见 [`vulkan-swapchain.md`](vulkan-swapchain.md) |
| 把界面（`DrawList`）送上 GPU | ❌ **M3**（`Frame::record` 对非空绘制列表明确返回 `Unsupported`；空列表/只有 `NodeHint` 可以记录） |
| 交换链回读（HAL 的 `Frame::read_pixels`） | ❌ **明确 `Unsupported`**：HAL 的 `read_pixels` 在**提交前**调用，而交换链图像的回读数据只有**呈现之后**才有效 ⇒ 错误信息指向 `WindowedRenderer::read_back_last_frame()`。离屏回读请用 [`gpu-offscreen.md`](gpu-offscreen.md) 的 `OffscreenRenderer` |
| 纹理（`create_texture` / `upload_texture`，字形图集上传） | ❌ **M3**（明确报 `Unsupported`） |

**所以现在要出「界面」的图仍然用 CPU 后端**（见 [`rendering.md`](rendering.md)）：
Vulkan 能离屏画几何、也能把像素呈现在窗口里，但**还不消费 `DrawList`**（那是 M3）。

> **校验层开关（如实登记）**：`DEER_VK_VALIDATION=1` 在**三条路径上一致生效** —— `VkBackend::new`、
> 设备/离屏路径（`create_device`）与窗口路径（`WindowedRenderer`）都读它。
> 唯一的遗留是推送常量矩形着色器 `spirv.rs::vertex_shader_rect_pushconstant` 损坏
> （校验层报 `VUID-StandaloneSpirv-PushConstant-06808`，请求校验层时会让进程 `0xc0000005` 崩溃；
> `device_smoke` / `pipeline_smoke` 里涉及它的测试因此在校验层下**显式跳过**，t15）。
> 设备/离屏路径曾经**故意不读**这个变量（那时 offscreen 有 3 个真缺陷），**t18 修完后重新接回**。
> 本机实测 `DEER_VK_VALIDATION=1 cargo test -p deer-vk` → **全部通过 / 0 failed**、零校验消息（**具体条数以运行输出为准**，本仓库不在文档里固化测试条数）；
> 窗口示例 30 帧同样零消息、`exit=0`。细节与「修好它属于 M3」见 [`ROADMAP.md`](../../ROADMAP.md) 的 Q-5。

## 2. 最小示例

```rust
use deer_gpu::Backend;   // trait 方法必须引进作用域

let vk = deer_vk::VkBackend::new()?;
println!("后端：{}", vk.name());
for (i, a) in vk.adapters().iter().enumerate() {
    println!("[{i}] {} | {:?} | {}", a.name, a.kind, a.driver);
}

// 打开逻辑设备
let dev = deer_vk::VkDevice::open(0)?;
println!("图形队列族：{}", dev.queue_family_index());
println!("内存类型数：{}", dev.memory_type_count());

// 着色器驱动验收：自研 SPIR-V 汇编器的产物是否被驱动接受
let vs = deer_vk::spirv::vertex_shader_triangle([[-1.0, -1.0], [3.0, -1.0], [-1.0, 3.0]]);
dev.create_shader_module(&vs)?;
```

跑完整版：`cargo run -p deer-gui --example vulkan_devices`

## 3. 为什么不需要 Vulkan SDK

M1 实测：`#[link(name = "vulkan-1")]` 在**没装 SDK** 的机器上**链接失败**
（`LNK1181: 无法打开输入文件 "vulkan-1.lib"`）—— 系统只带 `vulkan-1.dll`（loader），
导入库 `.lib` 属于 SDK。

所以走**运行时动态加载**：`LoadLibraryW("vulkan-1.dll")` + `GetProcAddress`，
只依赖 `kernel32`。结构体布局在 crate 内手写，并用 `size_of` / `offset_of!` 断言
+ **驱动验收**双重防守。

另外，没有 SDK 就拿不到 `glslc`，所以 SPIR-V 由**自研汇编器**生成
（`deer_vk::spirv`，见下）。

## 4. 自研 SPIR-V 汇编器

`deer_vk::spirv::Module` —— 分配 Id + 写指令，**词数由代码自动算**（SPIR-V 最容易写错的地方）。
已提供三支着色器：

| 函数 | 用途 |
|---|---|
| `vertex_shader_triangle([[f32;2];3])` | 3 个常量顶点，按 `gl_VertexIndex` 选一个 ⇒ **不需要顶点缓冲** |
| `fragment_shader_solid([f32;4])` | 常量颜色 + `OriginUpperLeft` |
| `vertex_shader_rect_pushconstant()` | 用**推送常量**传矩形边界，6 个顶点画两个三角形（GUI 的常规手段） |

每条着色器都有自洽性断言（指令流词数、bound、字面量补齐、执行模型、Output 无初值）。

## 5. 一个重要的实测发现

> **`vkCreateShaderModule` 很宽容** —— 它只读头部与指令流，不做完整校验。
> 实测一个 `bound = 0`（非法，必须 > 所有 Id）的模块**被驱动接受了**。

所以：**「驱动接受」≠「SPIR-V 正确」**。真正暴露问题是在 `vkCreateGraphicsPipelines`
或执行时。因此两道护栏都要有：
1. **调用前的结构检查**（长度、4 字节对齐、20 字节最小头部、魔数、`bound > 0`）—— 归我们；
2. **驱动验收** —— 归驱动，但只对「管线可创建性」负责。

## 6. 自检

```rust
// ① 设备必须真的能拿到队列与内存类型
assert!(!dev.queue().is_null());
assert!(dev.memory_type_count() > 0 && dev.memory_type_count() <= 32);

// ② 设备名不能是空的（结构体布局错会读到垃圾字符串 —— 最隐蔽的错误）
assert!(!dev.adapter().name.is_empty());

// ③ 畸形输入必须被**我们的护栏**拦下（不依赖驱动是否宽容）
assert!(dev.create_shader_module(&[]).is_err());
assert!(dev.create_shader_module(&[1, 2, 3]).is_err());     // 非 4 字节对齐
let mut bad = vec![0u8; 64];
dev.create_shader_module(&bad).is_err();                     // 魔数错

// ④ 连续打开/关闭不能崩（验证线程生命周期）
for _ in 0..3 { let d = deer_vk::VkDevice::open(0)?; d.wait_idle()?; }
```

## 7. 常见坑

| 现象 | 原因 | 怎么改 |
|---|---|---|
| 找不到 `name()` / `adapters()` | 忘了 `use deer_gpu::Backend;` | trait 方法必须引进作用域 |
| 以为调了 `dev.create_shader_module` 成功就万事大吉 | 驱动很宽容（见第 5 节） | 用我们自己的护栏 + 等管线测试 |
| 以为现在能出图 | 不能。只到设备与着色器 | 用 CPU 后端；GPU 出图在 M2a-3..6 |
| 编译报找不到 `vulkan-1.lib` | 你在尝试静态链接 | 本 crate 已改成动态加载；别自己加 `#[link]` |
| `VkDevice` 不能跨线程随便用 | 句柄在后台线程里被持有 | 现在只支持**单线程**用 `&mut self`；线程模型是 Q-4（未决） |

## 8. 相关

- GPU HAL 契约：[`gpu-hal.md`](gpu-hal.md)
- 里程碑与剩余步骤：[`../../ROADMAP.md`](../../ROADMAP.md)（M2a-3..6、M2b）
- 内部原理：[`../M1-report.md`](../M1-report.md)
- **做不到**：把界面（`DrawList`）送上 GPU（M3）、纹理上传（M3）、多线程（HAL 无 `Send`/`Sync`，见 Q-4）。
  交换链回读**不走 HAL**：`Frame::read_pixels()` 明确 `Unsupported`，用 `WindowedRenderer::read_back_last_frame()`（见 [`vulkan-swapchain.md`](vulkan-swapchain.md)）

## 9. 检查清单

- [x] 示例能跑：`cargo run -p deer-gui --example vulkan_devices` → `exit=0`
- [x] 示例有自检断言（适配器非空；边界如实打印）
- [x] `FEATURES.md` 已登记（标 🔄 并写明未完成部分）
- [x] `docs/TUTORIAL.md` 已包含（作为「进阶/现状」章节）
- [x] 明确写了「做不到什么」
