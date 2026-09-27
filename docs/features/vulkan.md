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

## 6. 静默不一致清单（**驱动不会替你拦**）

本仓库反复踩到同一类缺陷：**驱动接受「非法/不一致的组合」，症状是「不报错也不画」或「画出错色」**，
而不是一个清晰的错误码。下面每条都附**证据出处**；新增发现请续在这张表里（**不写死测试条数**，只写命令）。

| 不一致 | 驱动行为（本机实测） | 怎么兜住 |
|---|---|---|
| 管线 `colorAttachment` 格式 **≠** 渲染通道附件格式 | `vkCreateGraphicsPipelines` **返回成功、静默接受**（原文曾写「会拒绝」，与实测相反，已改） | 调用方必须传**同一格式**（离屏 `COLOR_FORMAT`、窗口 `swapchain.format()`）**且**靠**像素对照**兜底 |
| 管线**声明**了动态 viewport/scissor 却**从不调** `vkCmdSetViewport`/`vkCmdSetScissor` | 规范未定义行为：本机驱动**崩**（离屏 `0xC0000005`、窗口 `0xC000041D`）—— 已由 `viewport_dynamic_probe` **独立复现** | 声明与调用必须一致：声明动态就每帧真的设置；声明静态就不能设 |
| SPIR-V 自身非法（如 `bound = 0`） | `vkCreateShaderModule` **宽容接受**（见上一节） | 我们自己的结构检查 + `spirv-val` |
| 损坏的推送常量着色器（`spirv.rs::vertex_shader_rect_pushconstant`） | 普通驱动**宽容接受**；**开校验层**会 `0xc0000005` 崩溃 | 别用它建管线（矩形走顶点缓冲）；见 `FEATURES.md` 与 `ROADMAP.md` 的 Q-5 |

**第一条（格式不一致）的证据与用法**：

```powershell
# 可重跑用例：拿 B8G8R8A8_SRGB 的渲染通道 + R8G8B8A8_UNORM 的 color_format 建管线
cargo test -p deer-vk --test pipeline_smoke format_mismatch_is_accepted_by_this_driver_and_must_be_guarded_by_the_caller
```

- **症状**：不被驱动拦住 ⇒ **只能靠调用方自觉 + 像素对照**兜住（像素对照是唯一能咬住它的手段）。
- **`DEER_VK_VALIDATION=1` 时才有人提醒**：校验层会报 `VUID-VkGraphicsPipelineCreateInfo-renderPass-06043`
  一类；**普通运行不会**。所以「零校验消息」这条门禁是这类问题的**唯一自动护栏**。
- **与 M3c 的具体关联**：这正是 **M3c 必须把交换链换成线性 `*_UNORM`** 的**同族问题** ——
  格式（以及由此决定的编码/混合空间）**传错也能建成功**，但**像素语义**会按错误格式走；
  窗口路径若把 `color_format` 传成离屏那个 UNORM 常量、或交换链退回 sRGB，都会「跑得通、画得不对」，
  只有**上屏像素对照**（[`window.md`](window.md) 第 5 节的 `window_parity`）能发现。
- 实现侧的说明与这条契约写在 `crates/deer-vk/src/pipelines.rs` 的 `build_pipelines` 文档里
  （`RenderPass` 没有公开的 format getter ⇒ 这条约束**无法由 `build_pipelines` 强制**，是**调用方契约**）。

**第二条（声明动态却不设置）的证据与硬要求**：

```powershell
# 三组对照探针的源码：crates/deer-vk/examples/viewport_dynamic_probe.rs
# 在 deer-vk 里运行它（--example 后面写该文件名去掉 .rs 的部分），加 --group=0|1|2 单跑某组
cargo run -p deer-vk --example <去掉 .rs 的文件名> -- --group=0
```

- **硬要求**：**声明了动态状态，就必须在录制时真的设置它**（离屏 `0xC0000005`、窗口 `0xC000041D` 两次独立复现）。
- **它与「零校验消息」的关系**：这一组**崩得比校验层报错还早** ⇒ **这类情形下「零校验消息」没有证明力**。
  这和我们文档里既有的另一条是同族的：**「零消息」也不能证明内存域依赖正确**（VVL 不做通用同步验证）。
  ⇒ 「零消息」只说明「校验层跑了且没说话」，**不等于**「这段调用合法」。
- **产品行为不变**：**离屏仍用静态**（这条对照只推翻了旧**理由**；是否改用动态是独立的产品决策）。

### 6.1 像素类实验的报数规范（**新条目，来自一次真实误判**）

**不要把「不同像素数」当成唯一输出。** 至少要给：**① 直方图 ② 一个「已知答案」的对照帧。**

- **反例（真实踩坑）**：C1 的探针第一版拿**理想**清屏色 `round(0.1 * 255) = 26` 去比，
  而驱动实际清成 **25**（`25.5` **向偶数舍入**）⇒ 每个背景像素都被判「不同」，
  于是恒报 **4096/4096 全不同**，看上去像「三角形铺满了整帧」—— 结论完全错。
  **抓出它的是直方图**（一眼看出背景是单一值 `25`，而不是被几何覆盖）。
- **对照帧（已知答案）**：`vkCmdDraw(0)` 的**满清屏帧**必须 **0 个非背景像素**（C1 实测 **0** ✅）——
  这条不通过就说明「比较基准 / 取色 / 回读」本身有问题，后面所有像素结论都不可信。
- 这条与第 6 节清单同源：**驱动/回读不会替你报错**，所以像素实验必须自带一个能自我打假的对照。
- 反面措辞检查：**只报「非背景像素数」的实验不算完成** —— 至少附直方图与对照帧结果。

## 7. 自检

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

## 8. 常见坑

| 现象 | 原因 | 怎么改 |
|---|---|---|
| 找不到 `name()` / `adapters()` | 忘了 `use deer_gpu::Backend;` | trait 方法必须引进作用域 |
| 以为调了 `dev.create_shader_module` 成功就万事大吉 | 驱动很宽容（见第 5 节） | 用我们自己的护栏 + 等管线测试 |
| 以为现在能出图 | 不能。只到设备与着色器 | 用 CPU 后端；GPU 出图在 M2a-3..6 |
| 编译报找不到 `vulkan-1.lib` | 你在尝试静态链接 | 本 crate 已改成动态加载；别自己加 `#[link]` |
| `VkDevice` 不能跨线程随便用 | 句柄在后台线程里被持有 | 现在只支持**单线程**用 `&mut self`；线程模型是 Q-4（未决） |

## 9. 相关

- GPU HAL 契约：[`gpu-hal.md`](gpu-hal.md)
- 里程碑与剩余步骤：[`../../ROADMAP.md`](../../ROADMAP.md)（M2a-3..6、M2b）
- 内部原理：[`../M1-report.md`](../M1-report.md)
- **做不到**：把界面（`DrawList`）送上 GPU（M3）、纹理上传（M3）、多线程（HAL 无 `Send`/`Sync`，见 Q-4）。
  交换链回读**不走 HAL**：`Frame::read_pixels()` 明确 `Unsupported`，用 `WindowedRenderer::read_back_last_frame()`（见 [`vulkan-swapchain.md`](vulkan-swapchain.md)）

## 10. 检查清单

- [x] 示例能跑：`cargo run -p deer-gui --example vulkan_devices` → `exit=0`
- [x] 示例有自检断言（适配器非空；边界如实打印）
- [x] `FEATURES.md` 已登记（标 🔄 并写明未完成部分）
- [x] `docs/TUTORIAL.md` 已包含（作为「进阶/现状」章节）
- [x] 明确写了「做不到什么」
- [x] 「静默不一致清单」已登记（第 6 节：格式不一致 / 动态状态声明未设置 / SPIR-V 非法 / 损坏的推送常量）
