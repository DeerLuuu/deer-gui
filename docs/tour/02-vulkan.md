# 02 · deer-vk 事实地图（手写 Vulkan 绑定 + 自研 SPIR-V 汇编器）

> 本文是**只读测绘**产出：每条结论都带 `path:line` 锚点，签名／常量值／行号照抄代码。
> 读不到的地方写「未确认」，不臆测。行号以本文件写作时的仓库状态为准。
>
> **约束（读这份图前先知道）**：`crates/deer-vk` **没有** `ash` / `vulkano` / `wgpu` /
> `shaderc` / `glslc`；`Cargo.toml` 只有两个 workspace 依赖（`crates/deer-vk/Cargo.toml:13-15`：
> `deer-gpu.workspace` / `deer-layout.workspace`）。Vulkan 符号、结构体布局、SPIR-V 字节流
> **全部手写**。

---

## 1. 一句话职责（每个文件一行）

| 文件 | 一句话职责 |
|---|---|
| `src/lib.rs` | crate 门面：`VkBackend`（建实例 + 枚举物理设备）+ 全部 `pub use` 重导出（`src/lib.rs:42-49`）。 |
| `src/loader.rs` | 运行时 `LoadLibraryW("vulkan-1.dll")` + `GetProcAddress` 的**共享**封装（`Lib::open()` / `Lib::sym()`），给设备级路径复用（`src/loader.rs:40`、`:57`）。 |
| `src/ffi.rs` | **实例级** FFI：魔数／`VkResult` 常量、`VkPhysicalDeviceProperties` 等结构体、**自己的** `Loader`、`Instance`（含校验层 `debug_utils` 回调与 `validation_message_count()`）、`CoreFns`、`device_extension_available()`。 |
| `src/ffi_dev.rs` | **设备级** FFI：句柄别名、~90 个 `Vk*` 常量、~60 个 `#[repr(C)]` 结构体、全部 `Pfn*` 函数指针类型。零逻辑、零 `unsafe fn`。 |
| `src/device.rs` | 逻辑设备 + 队列 + 着色器模块 + 渲染通道 + 管线布局 + 图形管线；**生命周期线程**模型（Vulkan 对象留在后台线程，主线程只持句柄）。 |
| `src/spirv.rs` | 极简 SPIR-V **汇编器**（`Module`：按段累积、`finish()` 按规范顺序拼接）+ 14 支着色器构造函数 + 片元判据的 CPU 复算测试。 |
| `src/offscreen.rs` | M2a 的离屏渲染会话：`OffscreenRenderer`（图像／视图／帧缓冲／命令池／栅栏／暂存缓冲 RAII）+ `offscreen_for()`。 |
| `src/gpu_geom.rs` | **纯逻辑**翻译层：`DrawList` → `GpuVertex` 顶点流（NDC 换算 + CPU 侧几何裁剪 + `radius_kind` 编码），不碰 GPU。 |
| `src/gpu_render.rs` | M3a-T3：`GpuGeometryRenderer` —— 顶点缓冲 + 静态管线 + 离屏图像 + 命令录制 + 提交 + 回读，并带 `SubmitState` 状态机。 |
| `src/hal.rs` | 把窗口链包成 `deer_gpu` 的 HAL 形状（`VulkanDevice` / `VulkanSwapchain` / `VulkanFrame`），含两条纯函数映射 + 单测。 |
| `src/surface.rs` | `VkSurfaceKHR`：HAL 的不透明窗口句柄 → 平台 surface（本仓库只实现 Windows），并持有 4 个 surface 查询函数指针。 |
| `src/swapchain.rs` | 交换链 + 呈现 + 信号量：`Swapchain::choose_config/create/acquire/present`，以及纯函数 `pick_config` / `map_acquire_result` / `map_present_result` / `readback_row_pitch` / `reorder_to_rgba8`。 |
| `src/windowed.rs` | M2b 的**窗口出图链**：`WindowedRenderer`（实例 + surface + 设备 + 交换链 + 渲染通道 + 三角形管线 + 双帧同步 + 可选呈现帧回读）。 |

---

## 2. 文件清单表

行数由 `[System.IO.File]::ReadAllLines(path, UTF8).Count` 实测。

### 2.1 `src/*.rs`（13 个文件，全部覆盖）

| 文件 | 行数 | 职责 | 关键类型/函数（行号） |
|---|---|---|---|
| `src/lib.rs` | 145 | crate 门面 + `VkBackend` | `pub mod`×13（`:29-40`）；重导出（`:42-49`）；`pub struct VkBackend`（`:58`）；`VkBackend::new()`（`:73`，读 `DEER_VK_VALIDATION` 在 `:74-76`）；`instance()`（`:116`）；`physical_devices()`（`:121`）；`impl Backend for VkBackend`（`:126`）→ `open()`（`:135`，返回 `hal::VulkanDevice::new`，`:143`） |
| `src/loader.rs` | 93 | 运行时加载 `vulkan-1.dll` | `Module`/`FARPROC` 别名（`:17-18`）；`#[link(name="kernel32")]` 三个符号（`:20-25`）；`pub struct Lib`（`:28`）+ `unsafe impl Send/Sync`（`:34-35`）；`Lib::open()`（`:40`）；`pub unsafe fn sym<T: Copy>(&self, name: &str)`（`:57`，含 `size_of::<T>() == size_of::<FARPROC>()` 断言，`:68-72`）；`Drop`（`:78`）；`cstr()`（`:91`） |
| `src/ffi.rs` | 1207 | 实例级 FFI + 校验层 | `pub mod vk`（`:33`，把 `SurfaceHandle` 与 `ffi_dev::*` 收到一条路径）；`InstanceHandle`/`PhysicalDeviceHandle`/`SurfaceHandle`（`:38-41`）；`VkResult` 常量（`:44-54`，含 `VK_ERROR_OUT_OF_DATE_KHR = -1_000_001_004`、`VK_SUBOPTIMAL_KHR = 1_000_001_003`）；`PhysicalDeviceType`（`:61`）＋`from_raw`（`:70`）；`VALIDATION_LAYER`（`:87`）；`PhysicalDeviceLimits`（`:92`）、`PhysicalDeviceSparseProperties`（`:202`）、`PhysicalDeviceProperties`（`:213`）；`Pfn*` 类型（`:252-268`）；`struct Loader`（`:295`）→ `load()`（`:301`）／`available_layers()`（`:438`）／`available_extensions()`（`:478`）／`inst_sym()`（约 `:390-405`）／`Drop`（`:520`）；`pub(crate) struct CoreFns`（`:563`）＋`impl`（`:568`）；`device_extension_available()`（`:634`）；debug_utils 常量（`:680-701`）；**`static VALIDATION_MESSAGE_COUNT`（`:756`）**；**`pub fn validation_message_count()`（`:761`）**；`validation_callback()`（`:773`，计数点在 `:799`）；`pub struct Instance`（`:808`）→ `create`（`:828`）／`create_with_validation`（`:842`）／`create_with_extensions`（`:855`）／`validation_enabled`（`:1020`）／`enabled_extensions`（`:1025`）／`extension_available`（`:1032`）／`validation_from_env`（`:1047`）／`core_fns`（`:1054`）／`proc`（`:1070`）／`handle`（`:1075`）／`enumerate_physical_devices`（`:1080`）／`physical_device_properties`（`:1090`）；`Drop`（`:1099`）；`pub struct DeviceProperties`（`:1117`）；`pub(crate) fn result_name(rc)`（`:1128`）；布局测试 `mod layout_tests`（`:1161`） |
| `src/ffi_dev.rs` | 1084 | 设备级 FFI（纯声明） | 16 个句柄别名（`:23-37`）；`WHOLE_SIZE`/`QUEUE_FAMILY_IGNORED`（`:42`/`:44`）；`VK_STRUCTURE_TYPE_*`（`:67-98`，注意 `VK_STRUCTURE_TYPE_GRAPHICS_PIPELINE_CREATE_INFO = 28` 在 `:80`）；格式常量（`:105-109`，`R8G8B8A8_UNORM = 37`）；布局常量（`:114-120`）；用途位（`:124-131`）；内存属性位（`:135-137`）；缓冲用途位（`:145-149`）；attachment/管线状态常量（`:153-194`）；**`VK_PIPELINE_STAGE_*`（`:196-200`，`TOP_OF_PIPE=1<<0`、`COLOR_ATTACHMENT_OUTPUT=1<<10`、`TRANSFER=1<<12`、`BOTTOM_OF_PIPE=1<<13`、`ALL_COMMANDS=1<<16`）**；**`VK_ACCESS_*`（`:205-209`，注意 `TRANSFER_WRITE=1<<12`、`TRANSFER_READ=1<<11`、`COLOR_ATTACHMENT_WRITE=1<<8`）**；命令缓冲/队列/阶段（`:213-227`）；结构体定义（`:235-848`，例：`ImageMemoryBarrier`（`:333`）、`BufferMemoryBarrier`（`:348`）、`MemoryRequirements`（`:719`）、`PhysicalDeviceMemoryProperties`（`:727`，`memory_types: [MemoryType; 32]`）、`PushConstantRange`（`:569`）、`GraphicsPipelineCreateInfo`（`:588`））；`VK_FORMAT_R32_SFLOAT` **不在本文件**（见 `gpu_render.rs:97`），本文件只有 `R32G32_SFLOAT=103`（`:462`）、`R32G32B32_SFLOAT=106`（`:464`）、`R32G32B32A32_SFLOAT=109`（`:466`） |
| `src/device.rs` | 1571 | 逻辑设备 + 管线 + 生命周期线程 | `SPIRV_MAGIC`（`:26`）；`pub struct DeviceFns`（`:30`，54 个函数指针）；`pub struct VkDevice`（`:90`）＋`unsafe impl Send/Sync`（`:121-122`）；`open()`（`:126`）／`open_with_present()`（`:150`）／`open_inner()`（`:163`）；getter：`handle`（`:209`）、`queue`（`:213`）、`queue_family_index`（`:217`）、`present_queue`（`:222`）、`present_queue_family_index`（`:227`）、`pub(crate) physical_device`（`:232`）、`fns`（`:236`）、`adapter`（`:240`）；`create_shader_module()`（`:247`，结构护栏 `:248-275`）；`wait_idle()`（`:306`）；`memory_type_count()`（`:319`）；`validation_enabled()`（`:339`）；`memory_properties()`（`:344`）；`create_render_pass()`（`:353`）；`create_pipeline_layout()`（`:441`）；`create_vertex_pipeline()`（`:508`）；`create_graphics_pipeline()`（`:569`）；`create_graphics_pipeline_static_viewport()`（`:607`）；`create_graphics_pipeline_static_viewport_ex()`（`:632`）；`create_graphics_pipeline_raw()`（`:690`）；`build_pipeline()`（`:705`，私有核心，静态/动态分支在 `:729-764`）；`pub struct VertexAttr`（`:875`）；`empty_vertex_input()`（`:889`）；`validate_vertex_pipeline_args()`（`:906`，5 条错误路径）；`RenderPass`（`:943`）+ `handle`（`:958`）+ `final_layout`（`:963`）+ `Drop`（`:968`）；`PipelineLayout`（`:979`）／`Pipeline`（`:1002`）／`ShaderModule`（`:1037`）；`impl Drop for VkDevice`（`:1024`）；`struct ReadyInfo`（`:1060`）；`enum InstancePlan`（`:1082`）／`struct PresentTarget`（`:1090`）；`lifetime_thread()`（`:1102`，`vkDeviceWaitIdle` 在销毁前，`:1138`）；`create_device()`（`:1158`）；`resolve_device_fns()`（`:1382`）；`pub fn vk_result_name(rc)`（`:1446`）；单测（`:1464-1571`） |
| `src/spirv.rs` | 2219 | 自研 SPIR-V 汇编器 + 14 支着色器 | **见第 4 章专章** |
| `src/offscreen.rs` | 848 | 离屏渲染 + 回读 | `use` 在 `:25-29`；模块文档（`:1-23`）；RAII 类型：`Memory`（`:32`）、`Image`（`:55`）、`ImageView`（`:81`）、`Framebuffer`（`:104`）、`Buffer`（`:127`，`pub size`）、`CommandPool`（`:152`）、`Fence`（`:179`，`wait(timeout_ns)` 在 `:196`、`reset()` 在 `:211`）；`pub struct OffscreenRenderer`（`:244`）；**私有** `OffscreenRenderer::new()`（`:270`，`#[allow(clippy::too_many_arguments)]` 在 `:269`）；`width/height/image_memory`（`:512/:515/:520`）；`render_and_read_back()`（`:527`）；`offscreen_for()`（`:768`，唯一公开构造入口）；`color_range()`（`:786`）；`pick_memory_type()`（`:800`）；`alloc_memory()`（`:822`） |
| `src/gpu_geom.rs` | 301 | `DrawList` → 顶点流（纯逻辑） | **见第 5 章专章** |
| `src/gpu_render.rs` | 1256 | GPU 几何渲染器 | **见第 5 章专章** |
| `src/hal.rs` | 325 | HAL 接线 | 模块文档（`:1-30`）；`type Chain = Rc<RefCell<Option<WindowedRenderer>>>`（`:44`）；`pub struct VulkanDevice`（`:47`）→ `new()`（`:55`）／`adapter_index()`（`:64`）；`impl Device`（`:69`）→ `info`（`:70`）／`create_swapchain`（`:74`）／`create_texture`（`:98`，`Unsupported`）／`upload_texture`（`:104`，`Unsupported`）／`begin_frame`（`:115`）／`wait_idle`（`:127`）；`pub struct VulkanSwapchain`（`:136`）＋`impl Swapchain`（`:142`）；`pub struct VulkanFrame`（`:164`）＋`impl Frame`（`:168`）→ `record`（`:169`）／`read_pixels`（`:186`）／`submit_and_present`（`:196`）；**`present_result_of()`（`:212`）**；`target_format_of()`（`:221`）；单测（`:229-325`） |
| `src/surface.rs` | 527 | `VkSurfaceKHR` | 模块文档（`:1-18`）；扩展名常量（`:29-39`）；`VK_STRUCTURE_TYPE_WIN32_SURFACE_CREATE_INFO_KHR`（`:42`）；`SurfaceCapabilitiesKHR`（`:76`）／`SurfaceFormatKHR`（`:97`）；`#[link(name="kernel32")] GetModuleHandleW`（`:145`）；`pub struct Surface`（`:153`）；`Surface::create()`（`:172`，平台分支 `:173-180`，`handle == 0 ⇒ BadWindowHandle` 在 `:181-183`，6 个符号走 `instance.proc` `:186-201`）；`handle()`（`:257`）；`platform_extension()`（`:265`）；`platform()`（`:275`）；`pub(crate)` 借用接口：`instance_handle`（`:280`）／`core_fns`（`:285`）／`support_fn`（`:290`）／`capabilities`（`:295`）／`formats`（`:317`）／`present_modes`（`:358`）；`supports_present()`（`:402`）；`Drop`（`:423`） |
| `src/swapchain.rs` | 979 | 交换链 + 呈现 + 信号量 | `use` 在 `:24-34`；模块文档（`:1-22`，返回码表 `:7-12`）；`SWAPCHAIN_EXTENSION`（`:40`）；`VK_FORMAT_B8G8R8A8_SRGB = 50`（`:71`）／`VK_FORMAT_B8G8R8A8_UNORM = 44`（`:73`）／`VK_FORMAT_R8G8B8A8_SRGB = 43`（`:75`）；`SwapchainCreateInfoKHR`（`:91`）／`PresentInfoKHR`（`:122`）；`SurfaceCapabilities`（`:139`）＋`from_raw`（`:158`）；`SurfaceFormat`（`:190`）＋`from_raw`（`:196`）；`SwapchainConfig`（`:206`）；`Acquire`（`:215`）／`Present`（`:226`）；**`pick_config()`（`:247`）**；`clamp_dim()`（`:318`）／`pick_extent()`（`:325`）；**`map_acquire_result()`（`:340`）**；**`map_present_result()`（`:356`）**；`DEFAULT_ACQUIRE_TIMEOUT_NS = 1_000_000_000`（`:373`）；`readback_row_pitch()`（`:383`）；`reorder_to_rgba8()`（`:395`）；`struct SwapchainFns`（`:419`）／`load_swapchain_fns()`（`:480`）；`pub struct Semaphore`（`:499`）＋`create`（`:507`）；`pub struct Swapchain`（`:556`，`last_acquired: Cell<Option<u32>>` 在 `:569`）→ `choose_config`（`:576`）／`create`（`:596`）／`extent`（`:858`）／`format`（`:862`）／`present_mode`（`:866`）／`images`（`:870`）／`image_views`（`:874`）／`image_count`（`:878`）／`acquire`（`:883`）／`acquire_with_timeout`（`:893`）／`present`（`:929`）／`pub(crate) last_acquired_image`（`:920`）；`Drop`（`:962`） |
| `src/windowed.rs` | 1306 | 窗口出图链 | 模块文档（`:1-39`）；`VK_FENCE_CREATE_SIGNALED_BIT = 0x1`（`:58`）；**`FRAMES_IN_FLIGHT = 2`（`:64`）**；`FENCE_TIMEOUT_NS = 5_000_000_000`（`:68`）；`TRIANGLE_NDC`（`:71`）／`TRIANGLE_COLOR`（`:73`）；`FrameOutcome`（`:77`）；**`clear_color_value()`（`:89`）**；**`srgb_encoded_byte()`（`:116`）**；私有 RAII：`OwnedFramebuffer`（`:128`）、`OwnedCommandPool`（`:152`）、`OwnedFence`（`:175`）、`OwnedBuffer`（`:254`）、`OwnedMemory`（`:277`）、`struct ReadbackTarget`（`:306`）；`pub struct WindowedRenderer`（`:323`）；`new()`（`:361`）；getter：`adapter`（`:463`）／`extent`（`:467`）／`format`（`:471`）／`present_mode`（`:475`）／`image_count`（`:479`）／`frames_presented`（`:484`）／`suboptimal_frames`（`:492`）／`instance_extensions`（`:497`）；`set_readback_enabled`（`:506`）／`readback_enabled`（`:519`）／`readback_available`（`:524`）；**`read_back_last_frame()`（`:541`）**；`resize()`（`:608`）；**`render_and_present()`（`:698`）**；`wait_idle()`（`:806`）；`record()`（`:811`，私有）；`Drop`（`:997`）；模块级私有函数：`color_subresource_range()`（`:1009`）／`create_readback_target()`（`:1022`）／`pick_memory_type()`（`:1116`）／`create_framebuffers()`（`:1137`）／`create_command_pool()`（`:1182`）／`allocate_command_buffers()`（`:1207`）；单测（`:1241-1306`） |

### 2.2 `tests/*.rs`（14 个文件）

行数与内容见第 7 章。

### 2.3 非 `.rs` 资产

| 路径 | 内容 |
|---|---|
| `Cargo.toml` | 15 行；`[lib] name = "deer_vk"`（`:10`）；依赖只有 `deer-gpu`、`deer-layout`（`:13-15`） |
| `spirv_probe/*.spv` | 11 个 `.spv`（`export_spirv.rs` 的产物，供官方 `spirv-val` 校验）；`vs_rect_pushconstant.spv` 也在其中 |
| `render_out/gpu_raw.png` | 一次真机渲染的输出截图（262488 字节）；未在代码里找到引用（**未确认**它的生成者） |

---

## 3. 分层图：谁依赖谁、哪些是 pub

```text
                ┌──────────────────────────────────────────────────────────┐
 原始绑定层      │  ffi.rs            （实例级：Instance/CoreFns/Loader）      │
 （全部 pub）     │  ffi_dev.rs        （设备级：句柄/常量/结构体/Pfn*）        │
                │  loader.rs         （Lib: LoadLibraryW + sym<T>）          │
                └───────────┬──────────────────────────────────────────────┘
                            │ 被 device.rs / surface.rs / swapchain.rs 直接 use
                            ▼
                ┌──────────────────────────────────────────────────────────┐
 对象封装层      │  device.rs   VkDevice / ShaderModule / RenderPass /        │
 （pub struct）   │              PipelineLayout / Pipeline / VertexAttr        │
                │  offscreen.rs OffscreenRenderer + 6 个 RAII 类型           │
                │  surface.rs   Surface                                     │
                │  swapchain.rs Swapchain / Semaphore（+ 5 个纯函数）        │
                └───────────┬──────────────────────────────────────────────┘
                            │
       ┌────────────────────┴─────────────────────┐
       ▼                                          ▼
┌──────────────────────────────┐   ┌──────────────────────────────────────┐
│ 消费路径 A：离屏 GPU 几何      │   │ 消费路径 B：窗口链                     │
│ gpu_geom.rs（纯逻辑，无 GPU）  │   │ windowed.rs  WindowedRenderer         │
│      ↓ build_stream()         │   │      ↑                                │
│ spirv.rs（生成两支着色器）     │   │ hal.rs  VulkanDevice/VulkanSwapchain/ │
│      ↓                        │   │         VulkanFrame（HAL 形状）        │
│ gpu_render.rs                 │   │      ↑                                │
│   GpuGeometryRenderer         │   │ deer_gpu::{Device,Swapchain,Frame}    │
│      ↓ render() -> Vec<u8>    │   │                                      │
│ tests/gpu_vs_cpu.rs 与 CPU 对照│   │ deer-gui/examples/window_preview.rs   │
└──────────────────────────────┘   └──────────────────────────────────────┘
```

### 3.1 关键依赖边（代码级）

| 从 | 到 | 证据 |
|---|---|---|
| `device.rs` | `ffi`, `ffi_dev`, `loader` | `src/device.rs:21-23` |
| `device.rs` | `offscreen.rs`（反向：`offscreen` 用 `device`） | `src/offscreen.rs:14`（未确认具体 use 行，函数签名用 `crate::device::VkDevice`，`src/offscreen.rs:769`） |
| `gpu_render.rs` | `device`, `ffi`, `ffi_dev`, `gpu_geom`, `spirv` | `src/gpu_render.rs:84-90` |
| `windowed.rs` | `device`, `ffi`, `ffi_dev`, `spirv`, `surface`, `swapchain` | `src/windowed.rs:46-53` |
| `surface.rs` | `ffi`（`instance.proc` 取 WSI 函数） | `src/surface.rs:186-201` |
| `swapchain.rs` | `device::{VkDevice, vk_result_name}` | `src/swapchain.rs` 的 `use`（**未确认具体行号**；`vk_result_name` 在 `src/swapchain.rs:349` 使用） |
| `hal.rs` | `windowed::{FrameOutcome, WindowedRenderer}` | `src/hal.rs:40` |
| `lib.rs` | 全部模块 + 重导出 | `src/lib.rs:29-49` |
| `deer-gui` | `deer_vk`（重导出为 `deer_gui::vk`） | `crates/deer-gui/src/lib.rs:44` |

### 3.2 pub 边界（谁对外可见）

- **整模块 pub**：`device` / `ffi` / `ffi_dev` / `gpu_geom` / `gpu_render` / `hal` / `loader` /
  `offscreen` / `spirv` / `surface` / `swapchain` / `windowed`（`src/lib.rs:29-40`）。
- **crate 根重导出**（`src/lib.rs:42-49`）：`Pipeline`、`PipelineLayout`、`RenderPass`、
  `ShaderModule`、`VertexAttr`、`VkDevice`、`GpuStream`、`GpuVertex`、`GpuGeometryRenderer`、
  `VulkanDevice`、`VulkanFrame`、`VulkanSwapchain`、`Buffer`、`CommandPool`、`Fence`、
  `Framebuffer`、`Image`、`ImageView`、`Memory`、`OffscreenRenderer`、`Surface`、`Acquire`、
  `Present`、`Semaphore`、`Swapchain`、`SwapchainConfig`、`FrameOutcome`、`WindowedRenderer`。
- **刻意 `pub(crate)`**：
  - `VkDevice::physical_device()`（`src/device.rs:232`）；
  - `Instance::core_fns()` / `Instance::proc()`（`src/ffi.rs:1054` / `:1070`）；
  - `Surface::instance_handle/core_fns/support_fn/capabilities/formats/present_modes`
    （`src/surface.rs:280/285/290/295/317/358`）；
  - `Swapchain::last_acquired_image()`（`src/swapchain.rs:920`）；
  - `ffi::CoreFns`、`ffi::PfnEnumeratePhysicalDevices`、`ffi::PfnGetPhysicalDeviceProperties`
    （`src/ffi.rs:257/260/563`）。
- **刻意不 pub 的构造**：`OffscreenRenderer::new()`（`src/offscreen.rs:270`）—— 理由写在
  `src/offscreen.rs:263-268`：它收裸 Vulkan 句柄，公开会触发 clippy `not_unsafe_ptr_arg_deref`；
  公开入口只有 `offscreen_for()`（`src/offscreen.rs:768`）。
- **`#[doc(hidden)]`**：`GpuGeometryRenderer::force_unconfirmed_submit_for_test()`
  （`src/gpu_render.rs:801`）。

### 3.3 谁调谁（调用点）

| 被调 | 调用点 |
|---|---|
| `hal::VulkanDevice::new` | `src/lib.rs:143` |
| `WindowedRenderer::new` | `src/hal.rs:82`；`crates/deer-gui/examples/window_preview.rs:210` |
| `GpuGeometryRenderer::new` | `crates/deer-gui/examples/gpu_geometry.rs:127`；`crates/deer-vk/tests/gpu_vs_cpu.rs:59` |
| `VkBackend::new` | `crates/deer-gui/examples/{gpu_offscreen.rs:32, vulkan_devices.rs:29, vulkan_pipeline.rs:40, hal_window_path.rs:63, gpu_geometry.rs:237}` |
| `spirv::vertex_shader_triangle` | `src/windowed.rs:403`；`examples/vulkan_devices.rs:55` |
| `spirv::fragment_shader_solid` | `src/windowed.rs:404`；`examples/vulkan_devices.rs:56` |
| `spirv::vertex_shader_rect_attrs` | `src/gpu_render.rs:542` |
| `spirv::fragment_shader_rect_shape` | `src/gpu_render.rs:543` |
| `gpu_geom::build_stream` | `src/gpu_render.rs:833` |
| `ffi::Instance::validation_from_env` | `src/lib.rs`（内联读环境变量，未调用该函数）；`src/device.rs:1173/1181`；`src/windowed.rs:372`；`tests/gpu_vs_cpu.rs:35`；`examples/vulkan_pipeline.rs:30` |
| `ffi::validation_message_count` | `tests/gpu_vs_cpu.rs:45` |
| `windowing` → HAL：`Device::create_swapchain` | `examples/hal_window_path.rs:68` |

---

## 4. `spirv.rs` 专章（最重要）

### 4.1 模块自述的格式与边界

- 头部 5 个字（20 字节）：魔数 `0x07230203`、版本、生成器、`bound`、保留（`src/spirv.rs:23`）。
- 每条指令：首字高 16 位 = **词数（含首字）**，低 16 位 = 操作码（`src/spirv.rs:24`）。
- `Id` 从 1 开始；头部 `bound` 必须 **>** 所有用到的 Id（`src/spirv.rs:25`）。
- **边界（诚实说明）**：不是通用编译器，**没有类型检查、没有 `spirv-val`**；
  只保证生成的模块语法自洽，正确性由驱动验收（`src/spirv.rs:14-19`）。
- 三条路线取舍：手写字节数组 / 极简汇编器（**选了它**）/ 可选外部 `glslc`
  （`src/spirv.rs:8-12`）。

### 4.2 `Module` 的 API（签名 + 行号，逐个）

按文件出现顺序。全部是 `pub`，除标注外均返回 `&mut Module`（链式）或 `u32`（新 Id）。

| 类别 | 签名 | 行号 |
|---|---|---|
| 构造 | `pub fn new() -> Module` | `:299` |
| 构造 | `impl Default for Module`（`Self::new()`） | `:292-296` |
| Id | `pub fn id(&mut self) -> u32` | `:312` |
| 头部 | `pub fn shader_capability(&mut self) -> &mut Module` | `:386` |
| 头部 | `pub fn memory_model_glsl450(&mut self) -> &mut Module`（`OpMemoryModel 0 1`） | `:391` |
| 头部 | `pub fn source_unknown(&mut self) -> &mut Module` | `:395` |
| 头部 | `pub fn entry_point(&mut self, model: u32, fn_id: u32, name: &str, interface: &[u32]) -> &mut Module` | `:402` |
| 头部 | `pub fn execution_mode(&mut self, entry: u32, mode: u32, params: &[u32]) -> &mut Module` | `:410` |
| 调试 | `pub fn debug_name(&mut self, target: u32, what: &str) -> &mut Module` | `:416` |
| 注解 | `pub fn decorate(&mut self, target: u32, decoration: u32, params: &[u32]) -> &mut Module` | `:423` |
| 注解 | `#[allow(dead_code)] pub fn member_decorate(&mut self, ty: u32, member: u32, decoration: u32, params: &[u32]) -> &mut Module` | `:429-430` |
| 类型 | `pub fn type_void(&mut self) -> u32` | `:438` |
| 类型 | `pub fn type_float(&mut self) -> u32`（`OpTypeFloat %r 32`） | `:444` |
| 类型 | `pub fn type_uint(&mut self) -> u32`（`OpTypeInt %r 32 0`） | `:450` |
| 类型 | `pub fn type_vector(&mut self, component: u32, count: u32) -> u32` | `:456` |
| 类型 | `pub fn type_array(&mut self, element: u32, length_const: u32) -> u32` | `:463` |
| 类型 | `pub fn type_struct(&mut self, members: &[u32]) -> u32` | `:470` |
| 类型 | `pub fn type_pointer(&mut self, storage_class: u32, pointee: u32) -> u32` | `:478` |
| 类型 | `pub fn type_function(&mut self, ret: u32, params: &[u32]) -> u32` | `:484` |
| 类型 | `pub fn type_bool(&mut self) -> u32` | `:676` |
| 常量 | `pub fn constant_f32(&mut self, ty: u32, value: f32) -> u32`（用 `value.to_bits()`） | `:492` |
| 常量 | `pub fn constant_u32(&mut self, ty: u32, value: u32) -> u32` | `:498` |
| 常量 | `pub fn constant_composite(&mut self, ty: u32, parts: &[u32]) -> u32` ⚠️ 成员必须是常量 | `:510` |
| 常量 | `pub fn composite_construct(&mut self, ty: u32, parts: &[u32]) -> u32` ✅ 成员可以是运行时值 | `:522` |
| 变量 | `pub fn variable(&mut self, ptr_ty: u32, storage_class: u32) -> u32` | `:534` |
| 函数 | `pub fn function(&mut self, ret: u32, fn_id: u32, fn_ty: u32, first_block: u32) -> &mut Module`（内部连发 `OpFunction` + `OpLabel`） | `:542` |
| 函数 | `pub fn return_void(&mut self) -> &mut Module` | `:708` |
| 函数 | `pub fn function_end(&mut self) -> &mut Module` | `:712` |
| 内存 | `pub fn load(&mut self, ty: u32, ptr: u32) -> u32` | `:550` |
| 内存 | `pub fn store(&mut self, ptr: u32, value: u32) -> &mut Module` | `:556` |
| 内存 | `pub fn access_chain(&mut self, ty: u32, base: u32, indexes: &[u32]) -> u32`（会置 `needs_in_bounds`，见 `assemble()` `:733-743`） | `:560` |
| 复合 | `pub fn composite_extract(&mut self, ty: u32, composite: u32, indexes: &[u32]) -> u32` | `:569` |
| 算术 | `pub fn f_add(&mut self, ty: u32, a: u32, b: u32) -> u32` | `:577` |
| 算术 | `pub fn f_sub(&mut self, ty: u32, a: u32, b: u32) -> u32` | `:583` |
| 算术 | `pub fn f_mul(&mut self, ty: u32, a: u32, b: u32) -> u32` | `:589` |
| 算术 | `pub fn bitwise_and(&mut self, ty: u32, a: u32, b: u32) -> u32` | `:596` |
| 算术 | `pub fn op_fnegate(&mut self, ty: u32, value: u32) -> u32` | `:603` |
| 算术 | `pub fn op_udiv(&mut self, ty: u32, result: u32, a: u32, b: u32) -> &mut Module`（⚠️ 与邻居不同：**调用方给 result Id**） | `:697` |
| 算术 | `pub fn convert_u_to_f(&mut self, ty: u32, value: u32) -> u32` | `:702` |
| 扩展集 | `pub fn ext_inst_import_glsl_std_450(&mut self) -> u32` | `:613` |
| 扩展集 | `pub fn op_ext_inst(&mut self, ty: u32, set: u32, instruction: u32, operands: &[u32]) -> u32` | `:625` |
| 扩展集 | `pub fn op_floor(&mut self, ty: u32, set: u32, value: u32) -> u32` | `:638` |
| 比较 | `pub fn op_ford_less_than(&mut self, bool_ty: u32, a: u32, b: u32) -> u32` | `:646` |
| 比较 | `pub fn op_ford_greater_than(&mut self, bool_ty: u32, a: u32, b: u32) -> u32` | `:653` |
| 逻辑 | `pub fn op_logical_or(&mut self, bool_ty: u32, a: u32, b: u32) -> u32` | `:663` |
| 逻辑 | `pub fn op_logical_and(&mut self, bool_ty: u32, a: u32, b: u32) -> u32` | `:670` |
| 逻辑 | `pub fn op_i_equal(&mut self, bool_ty: u32, a: u32, b: u32) -> u32` | `:683` |
| 逻辑 | `pub fn op_select(&mut self, ty: u32, cond: u32, true_val: u32, false_val: u32) -> u32` | `:690` |
| 收尾 | `pub fn finish(mut self) -> Vec<u8>`（调 `assemble()` 后转小端字节流） | `:720` |
| 收尾 | `fn assemble(&mut self) -> Vec<u32>`（私有；`finish` 与 `describe` 共用） | `:731` |
| 诊断 | `pub fn word_count(&self) -> usize`（各段之和 + 5） | `:778` |
| 诊断 | `pub fn section_word_counts(&self) -> [usize; SECTION_COUNT]` | `:783` |
| 诊断 | `pub fn describe(&mut self) -> String`（打印 **`assemble()` 的真实输出顺序**） | `:797` |
| 私有 | `fn section_mut`（`:318`）；`fn op(&mut self, opcode: u16, operands: &[u32]) -> &mut Module`（`:329`）；`fn literal_string(s: &str) -> Vec<u32>`（`:372`）；`fn words` / `assert_instruction_stream_well_formed`（测试，`mod tests` `:1856-2219`） |

私有辅助函数：`section_of_opcode`（`:228`）、`is_type_opcode`（`:246`）、
`is_constant_opcode`（`:262`）、`opcode_name`（`:825`）、`extract_access_chain_result_ids`（`:887`）。

公开常量：`SPIRV_VERSION_1_0 = 0x0001_0000`（`:123`）、`EXT_INST_GLSL_STD_450 = "GLSL.std.450"`
（`:128`）、`GLSL_STD_450_FLOOR = 8`（`:158`）、`SC_PUSH_CONSTANT = 9`（`:92`）、
`BUILTIN_VERTEX_INDEX = 42`（`:107`）、`BUILTIN_FRAG_COORD = 15`（`:113`）。

### 4.3 段序规则（模块文档 `:162-213` + 实现）

`enum Section`（`:194-213`）的**枚举值等于真实输出顺序**，`assemble()` 直接按它遍历（`:211`）：

```text
Capability = 0        ① OpCapability
Extension = 1         ② OpExtension
ExtInstImport = 2     ③ OpExtInstImport
MemoryModel = 3       ④ OpMemoryModel
EntryPoint = 4        ⑤ OpEntryPoint          ← 必须在**类型/常量之前**
ExecutionMode = 5     ⑥ OpExecutionMode
Debug = 6             ⑦ OpSource / OpName / OpMemberName / OpString / OpLine
Annotation = 7        ⑧ OpDecorate / OpMemberDecorate / OpGroupDecorate /
                        OpDecorationGroup / OpGroupMemberDecorate
TypeConstGlobal = 8   ⑨ 类型 + 常量 + 全局变量
Function = 9          ⑩ 函数体（含 OpFunction / 基本块 / 指令 / OpFunctionEnd）
```

（段序表原文见 `src/spirv.rs:168-179`；`SECTION_COUNT = 10` 在 `:215`。）

**为什么必须有段**（`:164-192`）：第一版按调用顺序线性写出，`OpEntryPoint` 落在类型/常量
之后 ⇒ 整份模块段全部错位。后果极具误导性：`vkCreateShaderModule` **接受**、
`vkCreateGraphicsPipelines` 也返回成功句柄非空，**但 `vkCmdDraw` 一个像素都不画**；
直到装上 SDK 用官方 `spirv-val` 才看到 `error: EntryPoint is in an invalid layout section`。

**路由规则**（`section_of_opcode` `:228-243`）与两条易错规则（`:221-227`）：

1. `OpFunction` 在「进入函数体之前」发射（那时 `in_function` 还是 false），**必须**显式映射到
   `Section::Function`（`:240`）；漏了会掉进 `_ => TypeConstGlobal`（`:241`），把函数头排到类型段里。
2. `OpLoad` / `OpStore` / 算术等「既可在全局也可在函数内」的指令不能靠 opcode 判断 ——
   归属由 `Module::in_function` 决定（`:343-361`）。

**`op()` 里顺序敏感的一条**（`:330-332`）：必须**先**决定这条指令进哪个段，**再**更新状态；
反过来的话 `OpFunctionEnd` 会先把 `in_function` 置 false，于是它自己被判成「函数外」而掉进类型段。

**函数体内延迟声明的三段顺序**（`assemble()` `:745-774`，注释在 `:755-765`）：

```text
  ... TypeConstGlobal 段 ...
  ① deferred_types   （函数体内延迟声明的**类型**）
  ② deferred_consts  （函数体内延迟声明的**常量**）
  ③ globals          （全局变量，可能引用 ①）
  ... Function 段（必须最后）...
  words[3] = next_id  （bound 必须 > 所有用过的 Id，`:773`）
```

三条实测教训（`:761-765`）：延迟声明放段首 ⇒ 模块级类型被挤到后面，报
「Type Id is not a type」；放段尾 ⇒ 落到 `OpFunction` 之后，报
「OpConstant cannot appear in the graph definitions section」；全局变量**不能**在类型段里
逐条插入。

**`InBounds` 装饰自动补齐**（`:732-743`）：只要出现过 `OpAccessChain`（`needs_in_bounds` 在
`op()` `:339-341` 置位），`assemble()` 会扫出所有 `OpAccessChain` 的结果 Id
（`extract_access_chain_result_ids()` `:887`：结果 Id 在操作数下标 1 ⇒ `words[i + 2]`，`:898`）
并补 `OpDecorate %id InBounds`（`DECORATION_IN_BOUNDS = 16`，`:100`）。

### 4.4 所有 `*_shader_*()` 函数：输入 / 输出 / 用途

（文件名 `vertex_shader_*` / `fragment_shader_*`，共 14 支。产物行号指 `pub fn` 行。）

| 函数（行号） | 输入 | 输出 | 用途 |
|---|---|---|---|
| `vertex_shader_empty()`（`:910`） | 无 | 空 `main`，无变量 | 诊断：连它建管线都崩 ⇒ 问题不在 SPIR-V 内容，而在管线状态或模块句柄（`:907-909`） |
| `fragment_shader_empty()`（`:925`） | 无 | 空 `main`，不写输出 | 同上（片段侧） |
| `vertex_shader_const_position()`（`:941`） | 无 | 常量 `gl_Position = (0,0,0,1)`，用 `OpConstantComposite` | 诊断：隔离「常量路径」，不用 `gl_VertexIndex`、不用数组（`:940`） |
| `vertex_shader_reads_vertex_index()`（`:968`） | 无 | 读 `gl_VertexIndex` 但不用它的值（`let _idx = m.load(...)` `:993`），写常量位置 | 诊断：隔离「读内置变量」这一步 |
| `vertex_shader_hardcoded_position()`（`:1005`） | 无 | 三顶点同一常量位置 `(0,0,0,1)` | 诊断：隔离「`OpSelect` 顶点选择逻辑」还是「管线/光栅化」；三顶点重合 ⇒ 退化 ⇒ 只验证「VS 有没有被跑起来」（`:1002-1004`） |
| `vertex_shader_select_full_vec4(positions: [[f32; 4]; 3])`（`:1035`） | 3 个完整 `vec4` | 按 `gl_VertexIndex` **逐分量** `OpSelect` 后 `OpCompositeConstruct` 组回 `vec4` | 诊断：与 `vertex_shader_triangle` 的区别是「v4 三个分量全部参与选择」，排查「先选标量再 `OpConstantComposite`」是否有问题（`:1031-1034`） |
| `vertex_shader_single_constant()`（`:1103`） | 无 | 常量位置 `(-0.8,-0.8,0,1)` | 诊断：**最干净的判别** —— 三个顶点都执行同一条 `OpStore` ⇒ 退化三角形、不产生像素；配合 `POINT_LIST` 用（`:1092-1102`） |
| `vertex_shader_from_vertex_buffer()`（`:1137`） | location 0 的 `vec2`（`SC_INPUT`） | `gl_Position = vec4(x, y, 0, 1)`（`OpCompositeConstruct`） | **完全标准**的 Vulkan 顶点路径，不依赖内置变量；判别「驱动能不能画出任何几何」（`:1129-1136`） |
| `vertex_shader_triangle(positions_ndc: [[f32; 2]; 3])`（`:1186`） | 3 个 NDC `vec2` | 用 `gl_VertexIndex` + `OpIEqual` + `OpSelect` 逐分量选 x/y，再 `OpCompositeConstruct` | M2a 窗口路径实际使用的 VS（`src/windowed.rs:403`） |
| `fragment_shader_solid(color: [f32; 4])`（`:1248`） | 无（不读位置） | location 0 输出 `vec4` 常量颜色 | M2a 三角形成色（`src/windowed.rs:404`） |
| `vertex_shader_rect_pushconstant()`（`:1331`） | **`SC_PUSH_CONSTANT` 的 `vec4`**（`ptr_pc_v4` 在 `:1348` 声明了两次，第一次在 `:1341` 被 `let _ =` 丢弃） | 由推送常量的 `(x0,y0,x1,y1)` 与 `idx & 1` / `idx / 2 & 1` 算出位置 | ⚠️ **已知不可用**，见 4.5 |
| `vertex_shader_rect_attrs()`（`:1433`） | location 0 `vec2` pos / 1 `vec4` rect / 2 `float` radius_kind / 3 `vec4` color | `gl_Position = vec4(pos, 0, 1)`；output location 0 `vec4` rect、1 `float` rk、2 `vec4` color（全部 `OpLoad`→`OpStore` 透传） | **M3a 实际使用的 VS**（`src/gpu_render.rs:542`） |
| `fragment_shader_rect_shape()`（`:1568`） | input location 0 `vec4` rect、1 `float` rk、2 `vec4` color；内建 `gl_FragCoord` | location 0 `vec4` 颜色（命中则原色，否则 `(0,0,0,0)`） | **M3a 实际使用的 FS**（`src/gpu_render.rs:543`），逐像素复刻 CPU 的 `fill`/`inside_rounded`/`stroke` |

补充：`vertex_shader_rect_attrs` 的接口表（VS 输出 → FS 输入）写在 `src/spirv.rs:1415-1423`，
`FS 自己从 gl_FragCoord 拿像素坐标 ⇒ VS 不传位置`（`:1421`）。

### 4.5 踩坑注释（逐条带行号）

**(a) 为什么不能用「运行时常量数组索引」**（`src/spirv.rs:1177-1185`）

早期版本把 3 个顶点放进 `OpConstantComposite` 数组，再用 `gl_VertexIndex` 做**运行时索引**
（`OpAccessChain`）。实测：**驱动在编译期直接崩**（`STATUS_STACK_BUFFER_OVERRUN`），
而 `vkCreateShaderModule` 明明接受了它 —— 因为驱动建模块时只存字节，**建管线时才真正编译**。
现实现改成 `OpSelect` 逐分量选，**没有任何动态索引**。

关联：`gpu_render.rs` 因此选择「每顶点重复一份 rect 属性」而不是按命令切换常量
（`src/gpu_geom.rs:26-32`）。

**(b) `OpFloor` 属于 `GLSL.std.450`，core SPIR-V 没有 `OpFloor`**（`src/spirv.rs:130-158`）

计划初稿写着「`OP_FLOOR=8`，照 `f_mul` 同构写一个 `op_floor`」——**两处错**：

1. **8 不是 `OpFloor`**：core SPIR-V 的 `8` 是 `OpLine`（本文件已有 `OP_LINE = 8`，`:33`）；
   按「同构」发出 `[8, ty, result, operand]` 会被 `section_of_opcode` 路由到**调试段**，
   操作数全被当成文件名/行号 ⇒ 校验器只报一堆无关的段序错误。
2. **更根本**：core SPIR-V 根本没有 `OpFloor`（也没有 `OpSqrt`/`OpSin`…）。这类数学函数全在
   `GLSL.std.450` 扩展指令集里，必须 `OpExtInstImport "GLSL.std.450"` 再用
   `OpExtInst resultType result set instruction operands...` 引用。`8` 是**扩展指令编号**
   `GLSLstd450Floor`（SDK 头文件：`Round=1, RoundEven=2, Trunc=3, …, Floor=8`）。
3. 这两处错**都不会让建模块失败**，实测是 `spirv-val` 报 `error: line 38: Invalid opcode: 9`
   才暴露（`:150-152`）。
4. 基准编号也在同一段核对过：`OpFNegate=127`、`OpFAdd=129`、`OpFSub=131`、`OpFMul=133`、
   `OpLogicalOr=166`、`OpLogicalAnd=167`、`OpFOrdLessThan=184`、`OpFOrdGreaterThan=186`、
   `OpExtInst=12`（`:154-157`）。
5. 使用点：`fragment_shader_rect_shape` 里 `let glsl = m.ext_inst_import_glsl_std_450();`
   （`:1573`），注释在 `:1571-1572`：「`glsl450` 是内存模型，**不是**扩展指令集」。

**(c) 推送常量为何不可用**（`src/spirv.rs:1279-1330`）

- 实测环境：Intel RaptorLake，Vulkan 1.4.309（`:1283`）。用该着色器建图形管线时
  `vkCreateGraphicsPipelines` **要么返回成功但不写管线句柄（空句柄）、要么直接
  `STATUS_ACCESS_VIOLATION`**（`:1284-1285`）。
- 试过三种写法都不行（`:1286-1289`）：`{vec4}` struct + Block + `OpAccessChain` ⇒ 返回成功句柄为空；
  同一 struct 改直接 `OpLoad` ⇒ 返回成功句柄为空；直接声明为 `vec4`（不加 Block）⇒ 访问违例。
- **规范违规（2025 补记）**：校验层直接报
  `PushConstant OpVariable <id> '10[%10]' has illegal type. Such variables must be typed as OpTypeStruct`
  （`VUID-StandaloneSpirv-PushConstant-06808`，`:1296-1301`）；报错之后进程以
  **`0xc0000005`（STATUS_ACCESS_VIOLATION）** 结束，`--test-threads=1` 也可复现，
  且只发生在 `vkCreateShaderModule` 这一步（`:1303-1306`）。
- 处置（`:1308-1319`）：`tests/device_smoke.rs::push_constant_rect_shader_is_accepted_by_driver` 与
  `tests/pipeline_smoke.rs::push_constant_rect_shader_is_known_broken` 在校验层下**显式跳过**；
  修好它的正路是声明成 `OpTypeStruct` + `Block` 并给出匹配的 `VkPushConstantRange`，
  然后靠**管线创建**（不是建模块）验收；在修好之前**不要让 `VkDevice::open()` 去读
  `DEER_VK_VALIDATION`**（`:1314-1315`）；
  `crates/deer-gui/examples/vulkan_pipeline.rs` 也调用本函数，开校验层跑它同样会崩（`:1316`）。
- 对 M2a 的影响：验收已由不带推送常量的着色器达成，**不阻塞** M2a-4..6（`:1321-1328`）；
  矩形绘制后续改走**顶点缓冲**方案。

**(d) `OpConstantComposite` vs `OpCompositeConstruct`**（`src/spirv.rs:504-528`）

`OpConstantComposite` 的**所有** `constituents` 必须都是常量；若成员是运行时值
（`OpSelect` / `OpLoad` / 算术结果），必须改用 `composite_construct`，否则 SPIR-V 非法，
而**驱动不报错、只是不画** —— 三角形着色器曾因此完全画不出像素（`:508-509`）。
使用点：`vertex_shader_triangle` 的 `:1237-1239`、`vertex_shader_from_vertex_buffer` 的
`:1167-1168`、`vertex_shader_rect_attrs` 的 `:1497-1498`、`vertex_shader_rect_pushconstant`
的 `:1393-1394`。

**(e) 不能对向量直接 `OpSelect` 配标量 bool 条件**（两处）

- `vertex_shader_select_full_vec4`：`:1072-1075` —— 规范要求「Result Type 与 condition 的分量数
  相等」，否则 `spirv-val` 报
  `Expected vector sizes of Result Type and the condition to be equal: Select`。
- `fragment_shader_rect_shape` 的最终输出：`:1835-1840` —— 本机 `spirv-val`（SDK 1.4.357.0）
  实测拒绝 `OpSelect %v4float %scalar_bool %color %zero`。
- 两处都改成「抽分量 → 逐分量 `OpSelect` → `OpCompositeConstruct` 组回」。

**(f) 向量布尔需要 `Vector16` 能力，本模块不开**（`src/spirv.rs:659-662`）

`OpLogicalOr` 只能逐**标量** `bool` 再串起来，片元判据就是这么用的。

**(g) Output 变量不要给初值**（`src/spirv.rs:530-533`）

Vulkan/SPIR-V 1.0 下带初值的 Output 全局变量会让后续读取失去「已定义」性
（校验报 use before definition）。测试 `output_variables_have_no_initializer`（`:1974`）
断言 Output 的 `OpVariable` 必须是 4 个词（无初值）。

**(h) `!x` 不引入 `OpLogicalNot`**（`src/spirv.rs:1544-1547`、`:1694-1706`）

两处逻辑非都用「把 `OpSelect` 的 true/false 两支对调」实现（`not_bool` 闭包在 `:1706`），
避免引入新算子，也让着色器**完全无分支**。

**(i) 描边公式**（`src/spirv.rs:1708-1758`）

没有照抄任务书给的 `stroke_mask` 公式，两个原因：① 任务书给的是「四条半平面的或」，
半平面在另一轴无限延伸 ⇒ 矩形外整片都会被判成描边；② 边带会沿短边向矩形外伸出 `bw`
（`null.rs::stroke` 的 `for k in 0..w` **没有**把 k 限制在矩形内；例：`(2,3,9,7)`、宽度 8 时
下边框在 `k=7` 填的是 `y == bottom-1-7 == 2`，在 `ry == 3` 之上一行）。
最终闭式（`:1724-1730`）：上/下带只受 `x_span` 约束、左/右带只受 `y_span` 约束。
**验证是穷举不是推理**：`checked++` 在 `:2174`，打印在 `:2178`。

⚠️ **数字纪律**（`:1740-1754`）：本注释曾写「14 个矩形 × 13 种宽度 = 466,901 个采样点」，
提交 `0b07997` 的信息里还有第三个数字 523,248 —— 那两个来自**早期一次性裸程序**、
口径不同且**在本仓库里复现不出来**，属**历史记录，不再引用**。以 CI 打印的
**20,663** 为准（`cases` 数组 17 组在 `:2137-2155`，`pad = |rk| + 3` 在 `:2161`）。
注释还记着「先后有 5 个『看起来对』的候选公式被穷举推翻」（`:1756-1757`）。

**(j) `OpUDiv` 的签名与邻居不同**（`src/spirv.rs:697`）

`op_udiv(ty, result, a, b)` —— **调用方传 result Id**（不分配新 Id），因为
`vertex_shader_rect_pushconstant` 里先用 `let half = m.id();` 手工取 Id（`:1374-1375`）。
用整数除法当右移，避免再引入移位操作码（`:1372`）。

**(k) `OpEntryPoint` 的 `interface` 必须是该执行模型下所有静态使用的 Input/Output 变量**
（`src/spirv.rs:399-401`）。各着色器的 interface 列表见 `:1462-1467`（rect VS 传 8 个）、
`:1592-1597`（rect FS 传 5 个）。

### 4.6 `spirv.rs` 自带的测试（`mod tests` `:1856-2219`）

| 测试（行号） | 守什么 |
|---|---|
| `assert_instruction_stream_well_formed`（辅助 `:1869`） | 逐条指令校验「首字高 16 位声明的词数」与总词数自洽；这是最容易写错、驱动只给含糊错误的地方（`:1867-1868`） |
| `all_shaders_are_well_formed`（`:1888`） | 三角形 VS / solid FS / rect-pushconstant VS 三条 |
| `bound_exceeds_every_id`（`:1898`） | `bound > max_id`（从字节 `[12..16]` 读） |
| `string_literals_are_nul_terminated_and_padded`（`:1910`） | `"main"` ⇒ 2 个字；正好 4 的倍数时不多补空字 |
| `execution_models_are_correct`（`:1921`） | VS 模型 = 0、FS 模型 = 4 |
| `fragment_shader_declares_origin_upper_left`（`:1940`） | FS 必须声明 `OriginUpperLeft` |
| `rect_shader_uses_push_constants`（`:1956`） | 矩形推送常量着色器必须有 1 个 `SC_PUSH_CONSTANT` 变量 |
| `output_variables_have_no_initializer`（`:1974`） | 见 4.5(g) |
| `gpu_mask_predicate`（辅助 `:2024`） | `fragment_shader_rect_shape` 运算的**忠实转写**（同样运算顺序、同样 float 语义），不是第二套实现（`:1998-2005`） |
| **`cpu_reference_mask_matches_null_rs`（`:2080`）** | 按 `null.rs` 原式独立写一遍 CPU 版，17 组 `(rect, radius_kind)` × `pad = |rk|+3` 的**全部**整数像素逐点比对；`--nocapture` 打印实测点数（`:2178`） |
| `rect_shaders_have_no_control_flow`（`:2187`） | M3a 两支着色器**只用声明的算子**、`OpLabel` 只出现一次（单基本块）；显式禁 `OP_BRANCH=249` / `OP_BRANCH_CONDITIONAL=250` / `OP_LOOP_MERGE=246` / `OP_PHI=245` / `OP_SWITCH=247`（`:2188-2192`） |

---

## 5. `gpu_geom.rs` 与 `gpu_render.rs` 专章

### 5.1 顶点布局（stride / 偏移 / 各 location 含义）

**唯一权威表**：`src/gpu_geom.rs:9-14`（模块文档）与 `src/spirv.rs:1406-1411`（着色器侧），
两处逐字段对齐、**不允许单边改动**。

| location | 类型 | `GpuVertex` 字段 | 含义 |
|---|---|---|---|
| 0 | `vec2` | `pos`（`[f32; 2]`，`src/gpu_geom.rs:80`） | 位置（**NDC**，`y` 向下为正） |
| 1 | `vec4` | `rect`（`[f32; 4]`，`:82`） | **原始**矩形 `(x, y, w, h)`，像素单位 |
| 2 | `float` | `radius_kind`（`f32`，`:84`） | `0` = 普通填充；`> 0` = 圆角半径；`< 0` = 描边（`-带宽`） |
| 3 | `vec4` | `color`（`[f32; 4]`，`:86`） | 颜色（0–1，**不预乘**，直接 src-alpha 混合） |

**结构体与字节布局**：`#[repr(C)] #[derive(Debug, Clone, Copy, PartialEq)] pub struct GpuVertex`
（`src/gpu_geom.rs:76-87`）。`#[repr(C)]` 是**必须的** —— 默认 `repr(Rust)` 不保证字段顺序，
实测编译器会把 `pos` 排到偏移 32（`:70-75`）。锁定后：

```text
pos @ 0 / rect @ 8 / radius_kind @ 24 / color @ 28，stride = 44   （src/gpu_geom.rs:75）
```

**表由 `offset_of!` 生成**（不是手抄）：`fn vertex_attrs() -> [VertexAttr; 4]`
（`src/gpu_render.rs:135-158`）：

| location | format 常量 | 值 | offset |
|---|---|---|---|
| 0 | `vk::VK_FORMAT_R32G32_SFLOAT` | 103（`src/ffi_dev.rs:462`） | `offset_of!(GpuVertex, pos)` = 0 |
| 1 | `vk::VK_FORMAT_R32G32B32A32_SFLOAT` | 109（`src/ffi_dev.rs:466`） | `offset_of!(GpuVertex, rect)` = 8 |
| 2 | `VK_FORMAT_R32_SFLOAT` | **100**（`src/gpu_render.rs:97`，**在本模块定义**，理由见 `:94-96`） | `offset_of!(GpuVertex, radius_kind)` = 24 |
| 3 | `vk::VK_FORMAT_R32G32B32A32_SFLOAT` | 109 | `offset_of!(GpuVertex, color)` = 28 |

stride = `std::mem::size_of::<GpuVertex>() as u32`（`src/gpu_render.rs:586`）。

**字面数字被两处钉住**（布局改了都会红）：
- `src/gpu_geom.rs:75`（文档）+ `tests/gpu_geom_stream.rs::vertex_layout_is_stable_for_the_vertex_buffer`（`:333`）；
- `tests/gpu_vs_cpu.rs::vertex_layout_matches_the_hand_written_attribute_offsets`（`:477`）——
  断言 `size_of::<GpuVertex>() == 44`（`:478`）、偏移 `0 / 8 / 24 / 28`（`:480-483`）。

**顶点展开**：一个矩形 = 6 个顶点（两个三角形 `TL,TR,BR` + `TL,BR,BL`），顺序在
`emit_quad` 的 `for pos in [[x0,y0],[x1,y0],[x1,y1],[x0,y0],[x1,y1],[x0,y1]]`（`src/gpu_geom.rs:243`）。
每个顶点**重复**一份 `rect`/`radius_kind`/`color`（`:26-32`）—— 代价是冗余，换来
**一条静态管线画完所有非文本命令**。

### 5.2 `radius_kind` 三态语义

| 值 | 含义 | 产生处 |
|---|---|---|
| `0.0` | 普通填充，不做圆角 | `RADIUS_FILL`（`src/gpu_geom.rs:37`），用于 `FillRect`（`:156`） |
| `> 0.0` | 圆角半径 | `FillRoundRect` ⇒ `(*radius).max(0) as f32`（`:160`，负半径视同无圆角，与 CPU `fill(.., radius.max(0))` 一致 `:159`） |
| `-1.0` | 1px 描边 | `RADIUS_STROKE`（`:40`），`width == 1` 时由 `radius_kind_for_stroke` 返回（`:61-62`） |
| `-w`（`w > 1`） | 描边，带宽 = `w` | `radius_kind_for_stroke` 的 `-(w as f32)`（`:64`） |

片元着色器侧：`bw = m.op_fnegate(f32_ty, rk)`（`src/spirv.rs:1759`），即带宽 = `-radius_kind`；
`r_eff = select(rk < 0, 0.0, rk)`（`:1666-1668`）—— 描边不做圆角；
`mask = select(rk < 0, stroke_mask, fill_mask)`（`:1825-1829`）。

**`width <= 0` 为何在这里夹到 1**（`src/gpu_geom.rs:47-58`，三条理由，缺一不可）：
① **与 CPU 基线一致** —— `null.rs::stroke()` 第一行就是 `let w = width.max(1)`，
CPU 语义里根本不存在「0 宽描边」，所以夹到 1 是**复刻**不是发明；
② **消灭两个会被静默误读的值** —— 不夹时 `0 ⇒ -0.0`（片元着色器 `rk < 0` 为假 ⇒ 被当成
**填充**，画出一整块实心），`-3 ⇒ +3.0`（正数 ⇒ 被当成**圆角半径**）；两者都不报错，只是画错；
③ **双层防护** —— `build_stream` 内部也先 `width.max(1)` 一次（`:166`），将来有别的调用方
绕过它直接调本函数，仍然拿不到那两个危险值。
钉它的测试：`tests/gpu_geom_stream.rs::radius_kind_for_stroke_traps_are_closed`（`:151`）。

描边展开：`stroke_bands(rect, band) -> [RectI; 4]`（`src/gpu_geom.rs:259-267`），顺序
**上 / 下 / 左 / 右**（与 `null.rs` 一致），**带间会重叠** —— 四角像素被画两次，CPU 那边同样重叠
（`:253-258`）。每条带**各自**与 clip 求交，但顶点属性 `rect` 始终是**原始**矩形（`:169-170`）。

### 5.3 裁剪为何在 CPU 侧做

模块文档三条已裁定的约定（`src/gpu_geom.rs:16-24`）：

1. **NDC**：`x = 2*px/w - 1`、`y = 2*py/h - 1`，`y` **向下为正** ⇒ 像素原点 (0,0)（左上角）映射到
   `(-1,-1)`，与 Vulkan NDC 朝向一致（`:18-19`）。实现在 `struct Ndc`（`:202-215`）。
2. **裁剪在 CPU 侧做几何裁剪**（**不依赖动态 viewport/scissor**），且**两个矩形必须分开**：
   顶点 `pos` 用**裁剪后**的矩形（决定光栅化范围），顶点属性 `rect` 保持**原始**矩形
   （圆角圆心、描边边带都要按原始矩形算，否则会被裁剪挪位）—— 见 ledger Ruling 5（`:20-22`）。
3. **描边带宽编码在 `radius_kind` 里** —— 见 ledger Ruling 6（`:23-24`）。

**底层理由（写在 `gpu_render.rs`）**：本模块用**静态** viewport/scissor，因为
**动态版在本机 Intel 驱动上画不出任何像素**；而对着静态状态的管线发动态设置命令会触发校验层
报错（`src/gpu_render.rs:24-31`，实测记录在 `tests/vbo_probe.rs`）。
`docs/TUTORIAL.md:490`、`docs/features/gpu-geometry.md:118-119` 同述。

裁剪栈语义（`src/gpu_geom.rs:105-121`）：初始为全画布，`PushClip` 求交、`PopClip` 出栈
（越界即退回全画布，`:153`）；与裁剪区求交后为空的几何**整条跳过**（`emit_quad` 的
`if vis.w <= 0 || vis.h <= 0 { return; }`，`:234-236`）；`NodeHint` 安静忽略（`:193`）；
`Text` 记入 `unsupported`（`:184-191`）。

**`clip_unbalanced`** 的判据（`:113-121`、`:197`）：`depth != 0 || pop_without_push`。
**独立数一遍**而不复用 `list.clip_balanced()`，因为 `DrawList::cmds` 是 `pub` 字段，
直接改写它的列表不会被内部记账看到（`:120-121`）。

### 5.4 屏障与内存类型选择

**常量定义位置**（`src/gpu_render.rs`，因为 `ffi_dev.rs` 不在允许改动清单里）：

| 常量 | 值 | 行号 | 备注 |
|---|---|---|---|
| `VK_FORMAT_R32_SFLOAT` | `100` | `:97` | `ffi_dev` 只声明了 R32G32/R32G32B32/R32G32B32A32 |
| `VK_PIPELINE_STAGE_HOST_BIT` | `1 << 14` | `:104` | |
| **`VK_PIPELINE_STAGE_VERTEX_INPUT_BIT`** | **`1 << 2`**（`0x4`） | `:112` | ⚠️ **不是 `1 << 5`** —— `1<<5` 是 `TESSELLATION_EVALUATION_SHADER`（`:107-109`） |
| `VK_ACCESS_HOST_WRITE_BIT` | `1 << 14` | `:114` | |
| **`VK_ACCESS_VERTEX_ATTRIBUTE_READ_BIT`** | **`1 << 2`**（`0x4`） | `:119` | 同样**不是** `1<<5`（那是 `SHADER_READ`）；与阶段位**数值相同但属于不同枚举**，不是笔误（`:117-118`） |
| `COLOR_FORMAT` | `vk::VK_FORMAT_R8G8B8A8_UNORM` = 37 | `:122` | **线性 UNORM**（不是 `_SRGB`），见模块文档三条前提 `:35-41` |
| `TIMEOUT_NS` | `1_000_000_000`（1 秒） | `:125` | 宁可失败也不要永久挂住 |
| `MIN_VERTEX_BYTES` | `4096` | `:128` | 顶点缓冲最小分配 |

**host → vertex 屏障**（`vertex_buffer_barrier_params()` `:190-197`）：

```text
srcStageMask  = VK_PIPELINE_STAGE_HOST_BIT                 (1<<14)
dstStageMask  = VK_PIPELINE_STAGE_VERTEX_INPUT_BIT         (1<<2)
srcAccessMask = VK_ACCESS_HOST_WRITE_BIT                   (1<<14)
dstAccessMask = VK_ACCESS_VERTEX_ATTRIBUTE_READ_BIT        (1<<2)
```

参数抽成 `struct BarrierParams`（`:174-180`）+ 纯函数，**理由**（`:168-173`）：reviewer 的变异
证明「代码语义对了」不等于「**可回归**」—— 把 `dstStage` 写成 `1<<5`、或把整段屏障删掉，
16 个测试靶**全绿**。发射点在 `record_and_submit` 的 `:942-975`（放在渲染通道**之前**，
`:940-941`），并自增 `host_to_vertex_barriers`（`:974`）。

**为什么加这条屏障**（模块文档 `:43-66`，依据 Vulkan §7.9 Host Write Ordering Guarantees）：
`vkQueueSubmit` 对 happened-before 的 host 写入**已经隐式建立 HOST → ALL_COMMANDS 依赖**，
因此像素一直是对的、加屏障前后也不会变。这不是「观察到过错误像素」的缺陷，而是
**依赖没有被写出来** —— 一旦将来出现下列任一改动，它会**静默**出错（驱动不报错、校验层也不查）：
主机写入挪到 `vkQueueSubmit` **之后**；顶点缓冲改用**非相干**的 `HOST_VISIBLE` 内存；
数据来自**另一个队列**。

**另一个确定的事实**（`:59-60`）：这类内存域依赖**校验层不查**（它做对象/参数/布局类校验，
不做通用同步验证）—— 所以「`DEER_VK_VALIDATION=1` 零消息」**不能**当作同步正确的证据。

**内存类型选择**：`pick_memory_type(props, type_bits, required)`（`:361-378`），遍历
`props.memory_type_count.min(32)`（`:366`），要求 `type_bits & (1 << i) != 0` 且
`m.property_flags & required == required`（`:368-373`），否则报 `Unsupported`（`:375-377`）。
调用点：

| 用途 | required 位 | 行号 |
|---|---|---|
| 顶点缓冲 / 回读暂存（`create_host_buffer`） | `HOST_VISIBLE \| HOST_COHERENT` | `:467` |
| 离屏图像 | `DEVICE_LOCAL` | `:624` |

**为什么只选 `HOST_COHERENT`**（`:62-66`）：**只**选相干内存，拿不到就报 `Unsupported`，
所以不需要 `vkFlushMappedMemoryRanges`。**不能顺手加上它**：`ffi_dev` 里没有
`vkFlushMappedMemoryRanges` 符号，而 `ffi_dev.rs` 不在允许改动清单里。
**将来若放开「必须相干」这个约束，必须同时补上 flush**，否则主机写入对设备不可见。
读回方向靠**栅栏**保证（`:68-69`）。钉住「绝不用非相干内存」的单测：
`host_buffers_never_use_non_coherent_memory`（`:1225`）。

**图像屏障**（`record_and_submit` `:1020-1074`）：
`old_layout: self.pass.final_layout()`（`:1028`）—— 注释在 `:1020-1022`：
曾经在别处写死 `COLOR_ATTACHMENT_OPTIMAL` 而渲染通道给的是 `TRANSFER_SRC_OPTIMAL`，
校验层直接报 `cannot transition the layout`。`src_access_mask = COLOR_ATTACHMENT_WRITE`、
`dst_access_mask = TRANSFER_READ`、`new_layout = TRANSFER_SRC_OPTIMAL`（`:1026-1029`），
阶段 `COLOR_ATTACHMENT_OUTPUT → TRANSFER`（`:1056-1057`）。

**资源全部包成 `VkObject`**（`struct VkObject` `:203-207`，`impl Drop` `:215-224`），
析构顺序由字段顺序保证（`:32-33`）。`VertexBuffer` 的字段顺序 = 析构顺序
（`buffer` 在前 ⇒ 先 `vkDestroyBuffer` 再 `vkFreeMemory`，`:226-234`）。

### 5.5 `SubmitState` 状态机

定义在 `src/gpu_render.rs:242-251`：

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum SubmitState {
    #[default] Idle,     // 没有在飞的提交 —— 可复用命令缓冲与顶点缓冲
    InFlight,            // 已提交、正在等栅栏 —— 只允许走「等 → resolve」
    Broken,              // 等待**失败** ⇒ 无法证明 GPU 已不再访问 ⇒ 永久不可复用
}
```

| 方法 | 行号 | 语义 |
|---|---|---|
| `ensure_reusable(self) -> GpuResult<()>` | `:255` | `Broken ⇒ Err(Unsupported)`（`:257-261`）；`Idle`/`InFlight ⇒ Ok`（`:262`） |
| `begin(&mut self)` | `:267` | `*self = InFlight` |
| `resolve(&mut self, wait_rc: i32) -> GpuResult<()>` | `:272` | `VK_SUCCESS ⇒ Idle + Ok`（`:273-276`）；否则 `Broken` + `Driver{code: wait_rc, ...}`（`:277-286`） |
| `mark_broken(&mut self)` | `:289` | 提交阶段出现别的错误时：**同样**不敢假设资源空闲 |

**存在理由**（`:236-241`、模块文档 `:71-78`）：`vkWaitForFences` 失败（超时）**不等于**提交完成。
旧实现直接把错误往上抛，但渲染器**状态不变** —— 调用方（或同一次 `render` 的重试）会接着
`vkResetCommandBuffer` / 重写顶点缓冲 / `vkResetFences`，而这些东西可能**仍在被 GPU 使用**。
那是未定义行为，且驱动往往不报错（症状是偶尔的垃圾像素或挂死）。

**守卫放在会做破坏性操作的地方本身**（模块文档 `:77-78`、`:914-917`）：
`render` 开头（`:823`）、`ensure_vertex_capacity`（`:860`）、`record_and_submit`（`:917`）
三处都查 `ensure_reusable()?` —— 这样即便调用方漏写 `?`，也不会真的去碰那些资源。

**状态迁移点**：`self.sync.begin()` 在 `vkQueueSubmit` **之前**（`:1098`）；
`queue_submit` 失败 ⇒ `mark_broken()`（`:1104`）；等待结果 ⇒ `self.sync.resolve(rc)`（`:1120`）。

**销毁侧的对应修复**（`src/device.rs:1129-1144`）：必须先 `vkDeviceWaitIdle` 再销毁设备
（T3 fix round 2 / R3）—— 否则「超时后不再复用」这条修复只是把 UB 从「复用」搬到了「销毁」。

**测试入口**：`force_unconfirmed_submit_for_test()`（`#[doc(hidden)]`，`:801`）。

### 5.6 `validation_message_count()` 机制

**定义**：`src/ffi.rs:756` 的 `static VALIDATION_MESSAGE_COUNT: AtomicUsize = AtomicUsize::new(0)`；
读取入口 `pub fn validation_message_count() -> usize`（`src/ffi.rs:761`，`Ordering::Relaxed`）。

**为什么要有它**（`src/ffi.rs:750-755`）：`DEER_VK_VALIDATION=1` 的「零消息」原先只能
**人眼看输出**，测试里没有任何断言 —— 于是「层没装好」或「有人把报错改成静默降级」都会让测试
照样全绿。**计数发生在回调里**（消息真的到达回调才自增），所以它统计的是「驱动真的报了什么」，
不是「我们以为会报什么」。

**计数点**在 `validation_callback` 的两个提前返回**之后**（`src/ffi.rs:798-799`）：
`if data.is_null() { return 0; }`（`:779-781`）与 `p_message.is_null()`（`:786-788`）
都不计数。

**标签映射的坑**（`src/ffi.rs:769-772`）：曾把 `WARNING = 0x100` 标成 `VK ERROR`、
`ERROR = 0x1000` 标成 `VALIDATION`、还引用了一个不存在的 `0x2000`。后果不是漏报而是
**误分类**：真正的校验错误被打成 `[VALIDATION]`，而警告被打成 `[VK ERROR]`，
让人按前缀筛选时得出**相反结论**。现按位值正确定义（`:791-797`）。

**配套的「层真的在跑吗」断言**：`VkDevice::validation_enabled()`（`src/device.rs:339`，
文档 `:323-341`）与 `GpuGeometryRenderer::validation_enabled()`（`src/gpu_render.rs:769`）。
两者合起来才把「零校验消息」变成可回归结论（fix round 2 / R1-2）。

**测试用法**：`tests/gpu_vs_cpu.rs` 的 `assert_no_validation_messages()`（`:44-53`）+
`validation_layer_state_matches_the_request`（`:81`）。

---

## 6. 公开 API 清单（签名 + 调用点）

### 6.1 crate 根重导出（`src/lib.rs:42-49`）

| 签名 | 调用点（grep 结果） |
|---|---|
| `pub struct VkBackend`（`src/lib.rs:58`） | `examples/{vulkan_devices.rs:29, gpu_offscreen.rs:32, vulkan_pipeline.rs:40, hal_window_path.rs:63, gpu_geometry.rs:237}`；`tests/vulkan_smoke.rs:11,15` |
| `VkBackend::new() -> GpuResult<VkBackend>`（`:73`） | 同上 5 个 example + `tests/vulkan_smoke.rs:15` |
| `VkBackend::instance()`（`:116`） / `physical_devices()`（`:121`） | **未确认**（在 deer-gui 侧未找到调用点） |
| `VkDevice::open(adapter_index: usize) -> GpuResult<VkDevice>`（`src/device.rs:126`） | `tests/{device_smoke.rs, pipeline_smoke.rs:26, offscreen_render.rs:27, vbo_probe.rs:25, raw_ffi_probe.rs:28}`；`examples/vulkan_devices.rs:46` |
| `VkDevice::open_with_present(adapter_index, &Surface)`（`:150`） | `src/windowed.rs:386`（唯一调用点） |
| `GpuGeometryRenderer::new(adapter_index, extent, clear) -> GpuResult<Self>`（`src/gpu_render.rs:531`） | `tests/gpu_vs_cpu.rs:59`；`examples/gpu_geometry.rs:127` |
| `GpuGeometryRenderer::render(&mut self, list: &DrawList) -> GpuResult<Vec<u8>>`（`:819`） | `tests/gpu_vs_cpu.rs`（多处，经 `compare()` `:114`）；`examples/gpu_geometry.rs`（未确认具体行） |
| `GpuGeometryRenderer::{extent, validation_enabled, unsupported, host_to_vertex_barrier_count}`（`:761/:769/:777/:791`） | `tests/gpu_vs_cpu.rs`（`:91` 附近用 `validation_enabled`；`:493` 附近用 `host_to_vertex_barrier_count`） |
| `GpuGeometryRenderer::force_unconfirmed_submit_for_test()`（`:801`） | `tests/gpu_vs_cpu.rs:535`（`render_is_refused_after_an_unconfirmed_submit`） |
| `fn vertex_attrs()` | **私有**（`src/gpu_render.rs:135`） |
| `WindowedRenderer::new(adapter_index, window, want, clear) -> GpuResult<Self>`（`src/windowed.rs:361`） | `src/hal.rs:82`；`examples/window_preview.rs:210` |
| `WindowedRenderer::render_and_present(&mut self) -> GpuResult<FrameOutcome>`（`:698`） | `src/hal.rs:202`；`examples/window_preview.rs`（未确认具体行）；`tests/swapchain_smoke.rs:517` 起的 `run_windowed_e2e` |
| `WindowedRenderer::read_back_last_frame(&mut self) -> GpuResult<Vec<u8>>`（`:541`） | `tests/swapchain_smoke.rs`（未确认具体行） |
| `WindowedRenderer::resize(&mut self, extent) -> GpuResult<()>`（`:608`） | `src/hal.rs:157`；`examples/window_preview.rs`（未确认具体行） |
| `WindowedRenderer::{extent, format, frames_presented, suboptimal_frames, instance_extensions}`（`:467/:471/:484/:492/:497`） | `src/hal.rs:88`（`extent`）、`:89`（`format`） |
| `WindowedRenderer::{set_readback_enabled, readback_enabled, readback_available}`（`:506/:519/:524`） | **未确认**（未在 deer-gui 侧找到调用点） |
| `WindowedRenderer::wait_idle()`（`:806`） | `src/hal.rs:129` |
| `fn clear_color_value(color: Color) -> [f32; 4]`（`src/windowed.rs:89`） | `src/windowed.rs:842`；`tests/windowed.rs` 类断言 `tests/windowed.rs::clear_color_value_is_exact`（`src/windowed.rs:1267`） |
| `fn srgb_encoded_byte(linear: u8) -> u8`（`:116`） | `examples/window_preview.rs:24`（import）；`tests/swapchain_smoke.rs:639-641` |
| `pub const FRAMES_IN_FLIGHT: usize = 2`（`:64`） | `src/windowed.rs`（内部）；测试 `frames_in_flight_is_at_least_one`（`:1261`） |
| `pub enum FrameOutcome { Presented, OutOfDate }`（`:77`） | `src/hal.rs:212-217`（映射）；`examples/window_preview.rs:24`；`tests/swapchain_smoke.rs:31` |
| `hal::VulkanDevice::new(adapter_index, adapter)`（`src/hal.rs:55`） | `src/lib.rs:143` |
| `fn hal::present_result_of(FrameOutcome) -> PresentResult` | **私有**（`src/hal.rs:212`），单测 `:235` |
| `fn hal::target_format_of(vk_format: i32) -> TargetFormat` | **私有**（`:221`），单测 `:255` |
| `offscreen::offscreen_for(&VkDevice, &RenderPass, w, h)`（`src/offscreen.rs:768`） | `tests/offscreen_render.rs`（经 `use deer_vk::offscreen`）；`examples/gpu_offscreen.rs`（未确认具体行） |
| `OffscreenRenderer::render_and_read_back(&self, pass, pipeline, vertex_count, clear)`（`:527`） | `tests/offscreen_render.rs`（未确认具体行） |
| `OffscreenRenderer::{width, height, image_memory}`（`:512/:515/:520`） | `tests/offscreen_render.rs`（未确认具体行） |
| `spirv::*`（14 个着色器函数 + `Module`） | 见 4.4 表与 `tests/export_spirv.rs:20-59`、`tests/spirv_val.rs:40-45` |
| `ffi::Instance::{create, create_with_validation, create_with_extensions, validation_enabled, enabled_extensions, extension_available, validation_from_env}`（`src/ffi.rs:828/842/855/1020/1025/1032/1047`） | `tests/validation_probe.rs:20/46`；`tests/swapchain_smoke.rs:440-473`；`tests/gpu_vs_cpu.rs:35`；`examples/vulkan_pipeline.rs:30` |
| `ffi::validation_message_count()`（`:761`） | `tests/gpu_vs_cpu.rs:45` |
| `ffi::VALIDATION_LAYER`（`:87`） | `tests/validation_probe.rs:26` |
| `loader::Lib::{open, sym}`（`src/loader.rs:40/57`） | `tests/validation_probe.rs:12`（`Lib::open`）；`src/device.rs:1383`（`Lib::open` + 54 次 `sym`） |
| `swapchain::{pick_config, map_acquire_result, map_present_result, readback_row_pitch, reorder_to_rgba8}`（`:247/:340/:356/:383/:395`） | `tests/swapchain_smoke.rs:25`（import）+ 各测试 |
| `swapchain::Swapchain::{choose_config, create, acquire, present, ...}`（`:576/:596/:883/:929`） | `src/windowed.rs:390/391/712/784` |
| `surface::Surface::create(&Instance, RawWindowHandle)`（`src/surface.rs:172`） | `src/windowed.rs:383` |
| `surface::Surface::{platform_extension, platform, handle, supports_present}`（`:265/:275/:257/:402`） | `src/windowed.rs:373`；`tests/swapchain_smoke.rs` |
| `surface::{SURFACE_EXTENSION, WIN32_SURFACE_EXTENSION, ...}`（`:29-39`） | `src/windowed.rs:376` |
| `device::vk_result_name(rc)`（`src/device.rs:1446`） | `src/gpu_render.rs`（多处）、`src/windowed.rs`（多处）、`src/swapchain.rs:349`、`src/offscreen.rs` |
| `device::SPIRV_MAGIC`（`:26`） | `src/device.rs:265`（内部） |
| `device::VertexAttr { location, format, offset }`（`:875`） | `src/gpu_render.rs:136-157`；`tests/pipeline_smoke.rs:272-297` |
| `gpu_geom::{build_stream, radius_kind_for_stroke, GpuVertex, GpuStream, RADIUS_FILL, RADIUS_STROKE}`（`:122/:59/:78/:94/:37/:40`） | `src/gpu_render.rs:833`；`tests/{gpu_geom_stream.rs, gpu_geom_parity.rs}`；`tests/gpu_vs_cpu.rs:24` |
| `ffi_dev::*`（常量/结构体/句柄） | `src/{device,gpu_render,windowed,swapchain,surface,offscreen}.rs`；`tests/{struct_layout.rs:19, vbo_probe.rs:13, raw_ffi_probe.rs:19, offscreen_render.rs:15, pipeline_smoke.rs:15, swapchain_smoke.rs:22}` |

### 6.2 冻结的 API 路径（`deer_vk::ffi::vk::*`）

`src/ffi.rs:27-36` 的 `pub mod vk` 把 `SurfaceHandle` 与 `ffi_dev::*` 收到同一条路径下，
于是 `crate::ffi::vk::SurfaceHandle` / `ImageHandle` / `ImageViewHandle` / `QueueHandle` /
`SemaphoreHandle` / `PhysicalDeviceHandle` / `InstanceHandle` 全都能逐字解析。
被测试钉住：`tests/swapchain_smoke.rs::frozen_api_handle_paths_resolve`（`:401`，断言在 `:402-409`）。

---

## 7. 测试清单（`tests/*.rs` 各守什么 + 运行命令）

### 7.1 逐个文件

| 文件 | 行数 | 守什么（行号范围） | 是否需要 GPU |
|---|---|---|---|
| `tests/vulkan_smoke.rs` | 88 | 真机上 `VkBackend::new()` 能枚举到适配器，且设备名不是乱码（结构体布局正确性的证据）。文件头 `:1-8`；`vk_backend_enumerates_real_adapters` `:14` | 需要 loader；没有则优雅跳过（`:1-6`） |
| `tests/validation_probe.rs` | 56 | 校验层**可用性如实上报**：`create_with_validation(true)` 成功 ⇒ `validation_enabled()` 必须为真（`:20-29`）；失败 ⇒ 错误信息必须说清「为什么」与「怎么办」（`:30-38`）。`validation_layer_availability_is_reported_honestly` `:10`；`creating_instance_without_validation_still_works` `:44` | 需要 loader |
| `tests/device_smoke.rs` | 175 | M2a 驱动验收：设备 + 着色器模块（文件头 `:1-11`）；`validation_requested()` `:20`；**地雷门** `skip_known_broken_push_constant_shader` `:29`；推送常量着色器在校验层下**显式跳过**（`:34`、`:104`） | 需要真驱动 |
| `tests/pipeline_smoke.rs` | 318 | M2a-3：渲染通道 + 管线布局 + 图形管线（文件头 `:1-12`，`:3-10` 解释「这才是 SPIR-V 的真正验收」）；`graphics_pipeline_is_created_on_real_driver` `:40`；推送常量已知损坏 `:81`；`create_vertex_pipeline` 的 5 条错误路径走真实 `VkDevice`（`:272-297`，`cases` 在 `:297`） | 需要真驱动 |
| `tests/offscreen_render.rs` | 304 | M2a-4..6：命令缓冲 + 栅栏 + 离屏渲染 + 回读像素；**判据是具体颜色值**（文件头 `:1-12`）：画绿色三角形到黑背景，`64×64`（`W/H` 在 `:19-20`），断言中心像素绿、四角黑；段序说明在 `:91`、`:122` | 需要真驱动 |
| `tests/struct_layout.rs` | 149 | 手写 Vulkan 结构体的**大小与偏移断言**（文件头 `:1-17`，含「第一版手算错导致 4 条断言假红」的教训 `:11-12`）；`base_structs_match_measured_sizes` `:22`（`Extent2D=8`、`Viewport=24`、`ClearColorValue=16` … ） | **不需要**（纯 `size_of`/`offset_of`） |
| `tests/raw_ffi_probe.rs` | 624 | **完全绕开 `deer-vk` 封装**的手写裸 FFI 渲染路径（文件头 `:1-16`）：只用 loader 的函数指针手写渲染通道/帧缓冲/管线/命令缓冲/内存/栅栏/拷贝；画绿三角形到红背景，断言两类像素数量与比例（`:13-14`）。它证明了 M2a 的 bug 不在封装里（`:8-11`）。段序根因说明在 `:544` | 需要真驱动（探针，不销毁） |
| `tests/vbo_probe.rs` | 599 | **构造性实验**：完全标准的顶点缓冲 + 顶点属性路径（文件头 `:1-10`，判别规则在 `:6-8`）。`stride = 8` 的 `vec2` 顶点（`:20-21`）；**静态 viewport/scissor** 的实测记录在 `:465-467`（对静态状态调动态设置命令违反 `VUID-vkCmdDraw-None-08608`）；`dst_access_mask` 曾写错的记录在 `:479`；`stencil_store_op` 用错枚举的记录在 `:137-138` | 需要真驱动（探针，不销毁） |
| `tests/export_spirv.rs` | 67 | 把 11 支着色器写到 `spirv_probe/*.spv`（`:20-59`），供官方 `spirv-val` 校验；文件头给出运行命令 `:4-7`。**不依赖 SDK**，总是跑 | **不需要** |
| `tests/spirv_val.rs` | 344 | 用**官方** `spirv-val` 校验汇编器全部产物（文件头 `:1-29`，含「所有 Vulkan API 都返回成功但 `vkCmdDraw` 静默不产生片元」的三行现象表 `:8-12`）。`rect_attrs_shaders()` `:40`；路径可用环境变量 `SPIRV_VAL` 覆盖（`:29`）；找不到 `spirv-val` 时打印跳过原因（`:26-28`） | **不需要 GPU**，需要 `spirv-val` 可执行文件 |
| `tests/swapchain_smoke.rs` | 1061 | M2b 驱动验收，**两层**（文件头 `:1-18`）。第一层（默认跑，不需窗口）：`pick_config_*` 8 条（`:85-216`）、`acquire_result_mapping_is_exact`（`:229`）、`present_result_mapping_is_exact`（`:252`）、`reorder_*` 3 条（`:279/304/318`）、`readback_row_pitch_is_tight`（`:328`）、`swapchain_create_info_layout`（`:342`）、`present_info_layout`（`:366`）、`surface_struct_layouts`（`:379`）、`platform_extension_names_are_exact`（`:390`）、`frozen_api_handle_paths_resolve`（`:401`）、`extension_availability_is_queried_not_guessed`（`:419`）、`instance_enables_requested_extensions_and_rejects_missing`（`:440`）、`win32_surface_extension_is_available_on_windows`（`:474`）。第二层（`DEER_VK_WINDOW_TESTS=1` 才跑，`:497` 起）：`windowed_chain_end_to_end` → `run_windowed_e2e`（`:517`），原生 Win32 `CreateWindowExW`（`mod win32_window` `:814`），断言逐字节颜色用 `srgb_encoded_byte`（`:639-641`） | 第一层只需 loader；第二层需要桌面 |
| `tests/gpu_geom_stream.rs` | 473 | `DrawList` → 顶点流的**纯逻辑**断言（文件头 `:1-7`，锁死三件事）。23 条测试：NDC/两三角形共享属性（`:18/:61`）、颜色归一化与 **alpha 夹取**（`:88/:105`）、圆角与 `radius_kind` 陷阱（`:116/:127/:136/:151`）、描边 4 带与厚带/伸出矩形（`:169/:196/:222/:239/:398`）、裁剪（`:40/:252/:265/:285`）、退化（`:297/:358/:376`）、`NodeHint`/`Text`（`:31/:308/:318`）、**顶点布局稳定**（`:333`）、`clip_unbalanced` 三态（`:423/:438/:450/:466`） | **不需要 GPU** |
| `tests/gpu_geom_parity.rs` | 513 | **T2 语义守卫（CPU 侧复算）—— 这不是 GPU 证据**（文件头 `:1`，`:1-50` 详述定位）。三个 CPU 复算函数 `inside_rounded`（`:61`）/`stroke_inside`（`:87`）/`blend`（`:102`）；`rasterize_stream`（`:127`）/`assert_parity`（`:162`）；31 条 `*_parity` 测试（`:205-509`）。**三处同一判据的三个副本**表在 `:29-33`；半透明为什么不算 GPU 证据在 `:38-48` | **不需要 GPU** |
| `tests/gpu_vs_cpu.rs` | 556 | **M3a-T4 终局判据**：GPU 与 CPU **逐像素**对照（文件头 `:1-20`）。判据：不透明 ⇒ **逐字节相同**；半透明 ⇒ 最大通道差 **≤ 1 LSB**（`:5-7`）。`validation_requested()` `:34`；`assert_no_validation_messages()` `:44`；`renderer()` `:58`（**请求了校验层时「建不起来」就是失败，不是跳过** `:55-57`）；`validation_layer_state_matches_the_request` `:81`；`compare()` `:114`；语料 `opaque_corpus` `:162` / `alpha_corpus` `:267`；`net_balanced_with_extra_pop` `:258`（Ruling 19 正面用例）；13 条测试 `:304-556`（含越界 alpha `:348`、换 extent 的静态 viewport `:373`、退化 extent `:387`、连续两帧不串数据 `:406`、Text 报 Unsupported 后渲染器仍可用 `:425`、裁剪不平衡 `:452`、顶点布局字面数字 `:477`、**屏障计数** `:493`、**Broken 后拒绝渲染** `:535`） | 需要真驱动；无 GPU 时优雅跳过（`:17-20`） |

### 7.2 运行命令

```powershell
# 0) 纯逻辑 + 单元测试（无需 GPU / SDK）：这是最快的回归面
cargo test -p deer-vk --lib

# 0b) 打印片元判据的穷举采样点数（可复现的 20663）
cargo test -p deer-vk --lib cpu_reference_mask_matches_null_rs -- --nocapture

# 1) 全部 deer-vk 测试（无 GPU 环境下 GPU 类会优雅跳过并打印原因）
cargo test -p deer-vk

# 2) 开校验层的全量运行（安全动作：两处推送常量地雷测试会显式跳过）
$env:DEER_VK_VALIDATION=1; cargo test -p deer-vk -- --test-threads=1

# 3) 真窗口端到端（需要桌面；CI 会假红所以默认跳过）
$env:DEER_VK_WINDOW_TESTS=1; cargo test -p deer-vk --test swapchain_smoke -- --nocapture
#    换适配器：$env:DEER_WINDOW_ADAPTER=1

# 4) 导出 SPIR-V 并用官方 spirv-val 校验（权威判据）
cargo test -p deer-vk --test export_spirv
& C:/VulkanSDK/1.4.357.0/Bin/spirv-val.exe crates/deer-vk/spirv_probe/vs_rect_attrs.spv
cargo test -p deer-vk --test spirv_val
#    覆盖 spirv-val 路径：$env:SPIRV_VAL="C:/VulkanSDK/1.4.357.0/Bin/spirv-val.exe"

# 5) 环境变量语义（判据统一在 ffi::Instance::validation_from_env()）
#    DEER_VK_VALIDATION = "1" 或 大小写不敏感的 "true"  ⇒ 启用 VK_LAYER_KHRONOS_validation
#    启用后消息以 [VK ERROR]/[VK WARN]/[VALIDATION]/[VK VERBOSE]/[VK INFO] 前缀打到 stderr
```

**与 VALIDATION=1 相关的两条硬事实**：
- 请求了校验层但层**不存在** ⇒ `create_with_extensions` 直接返回 `Err(Unsupported)`，
  **不静默降级**（`src/ffi.rs:840-841`、`:865-869`）；
- 校验层的回调计数**只在层真的跑起来时**才可能自增，所以「零消息」必须与
  `validation_enabled()` 一起断言才有意义（`tests/gpu_vs_cpu.rs:73-80`）。

---

## 8. 「想改 X 该动哪里」

### ① 加一支新着色器（最常用）

1. 在 `src/spirv.rs` 的「着色器」段（`:905` 起）加一个 `pub fn xxx_shader_yyy(...) -> Vec<u8>`，
   照抄一个现成模板：**顶点属性透传**照 `vertex_shader_rect_attrs`（`:1433`）；
   **逐像素判据**照 `fragment_shader_rect_shape`（`:1568`）。
2. 若用到 core SPIR-V 没有的算子（`floor`/`sqrt`/`sin`…）⇒ 走 `ext_inst_import_glsl_std_450()`
   （`:613`）+ `op_ext_inst`（`:625`），**不要**猜 opcode 编号（教训见 `:130-158`）。
3. 若用到新的 opcode ⇒ 加 `const OP_*`（`:29-84`）→ 加进 `section_of_opcode`（`:228`）
   的段归属 → 加进 `opcode_name`（`:825`，否则诊断里显示成 `Op#N`）→ 若是类型/常量，
   还要加进 `is_type_opcode`（`:246`）/ `is_constant_opcode`（`:262`）。
4. 加进 `tests/export_spirv.rs` 的 `shaders` 数组（`:20-59`）与 `tests/spirv_val.rs` 的
   `all_shaders()` / `rect_attrs_shaders()`（`:40`）。
5. 跑：`cargo test -p deer-vk --lib` → `cargo test -p deer-vk --test export_spirv`
   → 官方 `spirv-val` → `cargo test -p deer-vk --test spirv_val`。
6. 若它要真正进管线 ⇒ 见 ①b。

### ①b 把它接进渲染器

- **离屏几何路径**：`src/gpu_render.rs:542-543`（`GpuGeometryRenderer::new` 里建模块）+
  `:578-588`（`create_vertex_pipeline` 的 stride/attrs）。
- **窗口路径**：`src/windowed.rs:403-406`（`create_shader_module` ×2 + `create_graphics_pipeline`）。
- 若 VS 的输入/输出 location 变了 ⇒ 同步改 `spirv.rs:1469-1489`（装饰）、`gpu_render.rs:135-158`
  （顶点属性表）、`gpu_geom.rs:9-14`（布局表）、三处测试的断言
  （`tests/gpu_vs_cpu.rs:477`、`tests/gpu_geom_stream.rs:333`）。

### ② 改顶点格式（stride / 字段顺序）

**这是最危险的一类改动**：契约同时存在于 5 个地方。

1. `src/gpu_geom.rs:76-87` 的 `GpuVertex` 结构（`#[repr(C)]` 不能去掉）。
2. `src/gpu_geom.rs:68-75` 的**文档表**（`pos @ 0 / rect @ 8 / radius_kind @ 24 / color @ 28`，
   stride 44）—— 手写数字，必须跟着改。
3. `src/gpu_render.rs:135-158` 的 `vertex_attrs()`（用 `offset_of!`，会自动跟；
   但 `format` 常量要人对）与 `:586` 的 stride。
4. `src/spirv.rs:1433-1512`（`vertex_shader_rect_attrs` 的 input location 0/1/2/3 与类型）。
5. 测试：`tests/gpu_vs_cpu.rs:477-483`（字面数字 44/0/8/24/28）、
   `tests/gpu_geom_stream.rs:333`（`vertex_layout_is_stable_for_the_vertex_buffer`）。
6. `src/device.rs:906-940`（`validate_vertex_pipeline_args`：stride>0、attrs 非空、
   extent>0、offset<stride、location 不重复）。

### ③ 加一个 `DrawCmd` 的 GPU 支持

1. `crates/deer-gpu` 侧先有该变体（`gpu_geom.rs` 的 `match cmd` 是穷尽匹配，`:142-194`）。
2. `src/gpu_geom.rs:141-195` 的 `match` 里加分支：决定 `pos` 矩形与 `attr` 矩形是否同一份
   （见 `:217-223` 的 `emit_quad` 契约）。
3. 若新形状需要新的逐像素判据 ⇒ 走 ①（改 `fragment_shader_rect_shape`）并在
   `spirv.rs:2024-2072` 的 `gpu_mask_predicate` **同步复算**、`cases` 数组（`:2137-2155`）加语料；
   同时改 `tests/gpu_geom_parity.rs:61-110` 的三个 CPU 副本。
4. 若新形状需要新顶点属性 ⇒ 回到 ②。
5. 语料：`tests/gpu_geom_stream.rs` + `tests/gpu_geom_parity.rs` + `tests/gpu_vs_cpu.rs` 的
   `opaque_corpus`（`:162`）/`alpha_corpus`（`:267`）。
6. 若涉及文本 ⇒ 现在**不能**只改 `gpu_geom`：`src/gpu_geom.rs:173-183` 明确登记了
   「无条件登记 `Text` ⇒ GPU 会拒收 CPU 正常出得出来的图（假阳性）」，修它属于
   **错误策略的行为变更**，必须独立任务 + 单独 review。
7. 若涉及窗口路径：`src/hal.rs:169-184`（`Frame::record` 对非空 `DrawList` 报 `Unsupported`）
   是那条「M3 才做」的边界。

### ④ 改错误策略（Unsupported vs Driver 的分界）

现有约定（照 `gpu_render.rs` 的文档与实现）：

- **`Driver.code` 只承载真实的 `VkResult`**（`src/gpu_render.rs:343-348`）——
  「返回成功但句柄为空」这种情形的 `rc == VK_SUCCESS`，塞进 `Driver` 只会打印「驱动错误 0」，
  是个**假的错误码**，所以归到 `Unsupported`。
- 渲染器级别的分类写在 `src/gpu_render.rs:807-818`（裁剪栈不平衡 / `unsupported` 非空 /
  上一次提交未确认 / 其余是 `Driver`）。
- **不要用 `GpuStream::clip_unbalanced` 当报错条件**（`:812-815`）：它更严
  （额外拒绝「多出的 `PopClip`」这种 CPU 画得出来的列表），拿它报错会让 GPU 拒收 CPU 能画的输入。
- HAL 层对应三条：`src/hal.rs:16-21`（三条刻意设计）与 `:23-30`（诚实边界）。
- 改完必须复核 `src/hal.rs:208-211` 那条「把映射改反了全仓 202 条测试无一变红」的记录 ——
  纯函数级单测是这类契约唯一的守卫。

### ⑤ 接第二个渲染目标（深度附件 / 第二个颜色附件）

1. `src/device.rs:353-435`（`create_render_pass`）—— 现在**硬编码** `attachment_count: 1`
   （`:411`）、`color_attachment_count: 1`（`:379`）、`deps` 两条（`:387-406`）、
   且 `initial_layout` 固定用 `VK_IMAGE_LAYOUT_UNDEFINED_ATTACHMENT`（`:367`）。
2. `src/gpu_render.rs:546-550`（调 `create_render_pass` 的地方）与 `:591-671`（图像/视图/帧缓冲）。
3. 管线侧：`src/device.rs:705-867` 的 `build_pipeline` —— 现在 `p_depth_stencil_state: null`
   （`:824`）、`attachment_count: 1`（`:808`）。
4. 清理/加载语义：`vk::AttachmentDescription`（`src/ffi_dev.rs:612`）、
   `SubpassDependency`（`:648`）已声明，够用。
5. 回读：`color_range()`（`src/gpu_render.rs:1141`）/`src/offscreen.rs:786` 都是单层；
   深度回读需要新的 aspect mask 与拷贝路径。
6. 窗口路径**不要**顺手改：`WindowedRenderer::resize` 里格式变了会重建渲染通道与管线
   （`src/windowed.rs:662-678`），顺序是**先换管线再换渲染通道**（`:663`）。

### ⑥ 换驱动 / 换机器后要重跑什么

| 变化 | 必须重跑 | 为什么 |
|---|---|---|
| 换了 GPU 或驱动版本 | `cargo test -p deer-vk`（含 `tests/gpu_vs_cpu.rs`）+ `$env:DEER_VK_VALIDATION=1; cargo test -p deer-vk -- --test-threads=1` | 静态/动态 viewport 的行为（`src/gpu_render.rs:24-31`）、sRGB 编码字节（`src/windowed.rs:98-115`）、推送常量崩溃（`src/spirv.rs:1279-1330`）都是**真机实测**结论 |
| 装了 Vulkan SDK | `cargo test -p deer-vk --test spirv_val` | 从「跳过」变成真正校验；`tests/validation_probe.rs` 的结论也会反转（文件头 `:4-5`） |
| 改了任何着色器汇编逻辑 | `cargo test -p deer-vk --lib` → `--test export_spirv` → 官方 `spirv-val` → `--test spirv_val` → `--test pipeline_smoke` → `--test gpu_vs_cpu` | 段序错误**只**会被 `spirv-val` 抓到（`tests/spirv_val.rs:21-24`） |
| 改了 `GpuVertex` / 顶点属性 | `cargo test -p deer-vk --test gpu_vs_cpu --test gpu_geom_stream --test struct_layout` | 布局是字面数字契约 |
| 改了同步/屏障 | `$env:DEER_VK_VALIDATION=1` 跑全量 **并** 看 `tests/gpu_vs_cpu.rs:493` 的屏障计数 | 校验层**不查**内存域依赖（`src/gpu_render.rs:59-60`），计数是唯一能咬住「删掉屏障」的判据 |
| 在别的平台（非 Windows） | 先看 `src/surface.rs:173-180` | surface 只实现了 Windows，其它平台**明确报 Unsupported** |

---

## 9. 已知坑 / 边界

### 9.1 静态 viewport 的由来

- **事实**：`gpu_render.rs` 用的管线把 viewport/scissor **写死**在管线创建信息里，录制时
  **不调** `vkCmdSetViewport`/`vkCmdSetScissor`（`src/gpu_render.rs:1005-1007`，
  `device.rs::create_vertex_pipeline` 的静态分支在 `src/device.rs:542-561`）。
- **为什么**（`src/gpu_render.rs:24-31`）：实测动态版在本机 Intel 驱动上**画不出任何像素**
  （清屏正常、绘制为零）；而对着静态状态的管线发动态设置命令会触发校验层报错。
- **实测记录**：`tests/vbo_probe.rs:465-467` —— 违反 `VUID-vkCmdDraw-None-08608`。
- **对照版本**：`device.rs` **同时**保留动态版（`create_graphics_pipeline` `:569`，
  `build_pipeline` 的 `None` 分支 `:729-738`），理由写在 `:600-606`
  「两个都要有：既能定位问题，也给调用方一个可用选择」。
- **窗口路径反着来**：`WindowedRenderer` 用的是**动态** viewport/scissor
  （`src/windowed.rs:870-887`，管线由 `create_graphics_pipeline` 建于 `:406`），
  因为那里每帧都要按当前 extent 给值、resize 时不用重建管线（`:679`）。
  **两条路径的 viewport 策略不同，这是刻意的。**
- 文档侧同述：`docs/TUTORIAL.md:490`、`docs/features/gpu-geometry.md:118-119`。

### 9.2 VVL（校验层）**不检查**哪些类别

- **通用同步验证：不做**。校验层做对象/参数/布局类校验，**不**做通用同步验证 ⇒
  「`DEER_VK_VALIDATION=1` 零消息」**不能**当作同步正确的证据
  （`src/gpu_render.rs:59-60`）。这条是「加了屏障但没用」比「没加屏障」更难发现的原因
  （`src/gpu_render.rs:110-111`）。
- **SPIR-V 的编译期语义：建模块时极弱**。`vkCreateShaderModule` **只存字节、不编译**，
  连 `bound = 0` 都接受（`src/device.rs:244-246`，实测记录）；真正的编译发生在
  **建管线时**（`src/spirv.rs:1181-1182`、`:1318-1319`）。所以段序错误、`OpConstantComposite`
  误用、推送常量类型违规都能通过建模块。
- **内存域依赖（host write → vertex input）：不查** —— 见上一条。
- **反例（它确实会抓的）**：`VUID-vkCmdPipelineBarrier-dstStageMask-04091`（细分阶段位写错，
  `src/gpu_render.rs:107-109`）、`VUID-StandaloneSpirv-PushConstant-06808`
  （`src/spirv.rs:1300`）、`cannot transition the layout`（`src/device.rs:949-952`、
  `src/offscreen.rs:612-618`）、`Expected vector sizes of Result Type and the condition to be equal`
  （`src/spirv.rs:1073-1074`、`:1836-1838`）。
- **因此**：`tests/gpu_vs_cpu.rs:73-80` 把「层真的在跑」（`validation_enabled()`）与
  「跑了之后一条消息都没有」（`validation_message_count()`）**分开断言**，两件事合起来才把
  「零校验消息」变成可回归结论。

### 9.3 其它已知坑（带行号）

| 坑 | 位置 | 症状 / 处置 |
|---|---|---|
| 推送常量着色器 = 地雷 | `src/spirv.rs:1279-1330` | 校验层下 `vkCreateShaderModule` 报 VU 后进程 `0xc0000005`；两处测试**显式跳过**（`tests/device_smoke.rs:29`、`tests/pipeline_smoke.rs:81`）；`examples/vulkan_pipeline.rs:30` 也走这条 |
| 运行时常量数组索引 | `src/spirv.rs:1177-1185` | 驱动编译期崩 `STATUS_STACK_BUFFER_OVERRUN`，建模块却成功 |
| `OpConstantComposite` 用错 | `src/spirv.rs:504-509` | 生成非法 SPIR-V，**驱动不报错、只是不画** |
| 对 `vec4` 直接 `OpSelect` 标量条件 | `src/spirv.rs:1072-1075`、`:1835-1840` | `spirv-val` 报向量分量数不等 |
| `bound = 0` 也能过 `vkCreateShaderModule` | `src/device.rs:244-246` | 所以 `device.rs` 自己加了结构护栏（`:248-275`），`SPIRV_MAGIC` 在 `:26` |
| 手写结构体「比规范小」 | `tests/struct_layout.rs:1-17` | 驱动按官方大小写入就越界 ⇒ `0xc0000409`/`0xc0000005`，**不是**清晰错误码；第一版手算错（`ClearValue` 写 20 实际 16、偏移写 152 实际 104）导致 4 条假红 |
| 图像屏障的 `oldLayout` 写死 | `src/offscreen.rs:612-623`、`src/device.rs:947-954`、`src/gpu_render.rs:1020-1022` | 必须问渲染通道要 `final_layout()`；写死会报 `cannot transition the layout` |
| 帧缓冲必须比交换链先销毁 | `src/windowed.rs:320`、`:654-655` | 帧缓冲引用交换链的图像视图 |
| `resize` 后回读数据必须失效 | `src/windowed.rs:659-660`、`:511-514` | 否则会拿陈旧数据冒充（`:540`） |
| 交换链 `SUBOPTIMAL` 不许当成功 | `src/windowed.rs:10`、`:714-725`、`:792-802`；`src/swapchain.rs:340-366` | 上报 `FrameOutcome::OutOfDate` 并计 `suboptimal_frames`（`src/windowed.rs:492`） |
| `acquire` 的 TIMEOUT/NOT_READY 不许当成功 | `src/swapchain.rs:12`、`:338-339` | 当成功用会画到未分配的图像索引上 |
| HAL 把 `OutOfDate` 映射成 `Presented` 的退化 | `src/hal.rs:206-218` | 纯函数 + 单测（`:235`）；记录里写明「独立验证者曾把这里改反，而当时全仓 202 条测试无一变红」 |
| `record` 对非空 `DrawList` 静默忽略 | `src/hal.rs:16-18`、`:169-184` | 刻意**明确报** `Unsupported`，因为静默会让「窗口里什么都没有」变成查不出的 bug |
| `GpuStream::clip_unbalanced` 过严 | `src/gpu_geom.rs:98-103`、`src/gpu_render.rs:812-815` | 它额外拒绝「多出的 `PopClip`」；**只当诊断**，不当报错条件 |
| `Text` 无条件登记（假阳性） | `src/gpu_geom.rs:173-183` | GPU 会拒收 CPU 正常出得出来的图；控制者已裁定本轮 defer，代码旁有可见登记 |
| `read_pixels` 在提交前调用无效 | `src/hal.rs:25-29`、`:186-194` | 交换链图像回读数据只有**呈现之后**才有效；指向 `read_back_last_frame()` |
| 只支持一个交换链 | `src/hal.rs:30` | HAL 侧的诚实边界 |
| 纹理创建/上传未实现 | `src/hal.rs:98-113` | 报 `Unsupported`，字形图集上传是 M3 |
| `Lib` 被三处各写一遍 | `src/ffi.rs:288-292`、`src/loader.rs:20-25`、`src/surface.rs:145` | `loader.rs:10-11` 的注释说「避免两处各写一遍 `LoadLibraryW`」，但 `ffi.rs` 仍内联了自己的 `extern` 声明（`:287-292`），`surface.rs` 另有 `GetModuleHandleW`。**文档与现状不完全一致**，属于文字与代码的小偏差 |
| `vkFlushMappedMemoryRanges` 缺失 | `src/gpu_render.rs:62-66` | 所以**只**选 `HOST_COHERENT`；将来放开这个约束**必须同时补 flush** |
| 推送常量路径下的 `create_pipeline_layout` 尺寸校验 | `src/device.rs:437-457` | 推送常量 `size` 必须是 4 的倍数（`Unsupported`） |
| `gpu_geom.rs:58` 的测试引用名 | `src/gpu_geom.rs:58` 写的是 `gpu_geom_stream.rs::radius_kind_for_stroke_traps_are_closed` | 该测试实际在 `crates/deer-vk/tests/gpu_geom_stream.rs:151`，路径写法是**相对文件名**（不是完整路径），可解析 |

### 9.4 「陈旧 `target/` 造成假红」的复现方式

⚠️ **本节未在本仓库任何文件里找到记载**（grep `陈旧` / `stale` / `删除 target` 只命中
`read_back_last_frame` 语义里的「陈旧数据」，与构建缓存无关）。因此下面是**按本 crate 的
构建特征推出的排查步骤**，不是仓库里已有的结论 —— 请按「待验证」对待：

1. **现象特征**：改了 `src/spirv.rs` 或 `src/ffi*.rs` 之后，`cargo test -p deer-vk` 报出的
   行号/行为与刚读到的源码对不上；或 `tests/spirv_val.rs` 校验的是上一次导出的 `.spv`。
2. **本 crate 特有的两个「跨运行状态」**：
   - `crates/deer-vk/spirv_probe/*.spv` 是 **`export_spirv.rs` 写出来的产物文件**，
     不是源码（`tests/export_spirv.rs:17`、`:64`）。`spirv_val.rs` 校验的可能是**上一轮**
     留下的 `.spv` ⇒ 先跑 `export_spirv` 再跑 `spirv_val`。
   - `crates/deer-vk/render_out/gpu_raw.png` 同理（未确认生成者）。
3. **复现方式**：
   ```powershell
   # 记录当前哈希
   Get-FileHash Z:\deer-gui\crates\deer-vk\spirv_probe\*.spv | Format-Table
   # 删掉产物与构建缓存，从零重建
   Remove-Item -Recurse -Force Z:\deer-gui\target
   Remove-Item Z:\deer-gui\crates\deer-vk\spirv_probe\*.spv
   cargo test -p deer-vk --test export_spirv          # 重新生成 .spv
   Get-FileHash Z:\deer-gui\crates\deer-vk\spirv_probe\*.spv | Format-Table   # 对比
   cargo test -p deer-vk --test spirv_val
   ```
4. **最可能的「假红」形态**：`spirv_val.rs` 报的段序/opcode 错误对应**旧版汇编器**的产物。
   判定依据是**文件 mtime 与哈希**，不是日志文本。

### 9.5 其它边界（诚实记录，来自模块文档）

- `spirv.rs` **没有类型检查、没有内建 `spirv-val`**（`src/spirv.rs:16-17`）；
- `gpu_render.rs` 不复用 `offscreen.rs`，因为后者用**动态** viewport 且没有
  `vkCmdBindVertexBuffers`（`src/gpu_render.rs:24-31`）；
- 与 CPU 逐像素可比的**三条前提**：颜色格式 `R8G8B8A8_UNORM`（非 `_SRGB`）、alpha 混合
  `src-alpha / one-minus-src-alpha`、**不预乘**（`src/gpu_render.rs:35-41`）；
- 半透明只保证 **≤ 1 LSB**：CPU 用 `round()`，GPU 走固定功能 `float → unorm8`，
  舍入时机与平局规则都不同（`tests/gpu_vs_cpu.rs:5-7`）；
- 越界 alpha（`a > 1.0`）的 clamp 在 GPU 侧**有冗余**：去掉 `color_f32` 的 `clamp` 后
  `out_of_range_alpha_matches_cpu_byte_for_byte` **仍然通过**，因为 UNORM 附件在固定功能
  混合阶段会隐式夹（`tests/gpu_vs_cpu.rs:341-346`）；真正能抓住它的是
  `tests/gpu_geom_stream.rs:105`；
- `windowed.rs` 的诚实边界（M3 才做）：mailbox 呈现模式、独占全屏、HDR/色彩空间选择、
  帧率限制、深度/多重采样（`src/windowed.rs:35-39`）；
- `lib.rs` 的里程碑表：M1 ✅ / M2a ✅ / M2b ✅ / M3 ⬜（`src/lib.rs:15-20`）。

---

## 10. 未确认条目汇总

以下条目在本次只读测绘中**未能确认**，不要在后续文档里当作事实引用：

1. `swapchain.rs` 顶部 `use` 语句 —— **已补**（`:24-34`）。
2. `offscreen.rs` 顶部 `use` 语句 —— **已补**（`:25-29`）。
3. `windowed.rs` 各私有 RAII 类型的 `struct` 起止行 —— **已补 `struct` 行**（`:128/152/175/254/277/306`）；各自的 `Drop` 实现边界未逐行核对。
4. `VkBackend::instance()` / `physical_devices()` 在 deer-gui 侧的调用点（grep 未命中）。
5. `WindowedRenderer::{set_readback_enabled, readback_enabled, readback_available}` 的外部调用点（grep 未命中；可能只被测试或示例使用，也可能目前无调用者）。
6. `GpuGeometryRenderer::render` 在 `examples/gpu_geometry.rs` 里的具体调用行号。
7. `WindowedRenderer::{render_and_present, resize, read_back_last_frame}` 在 `examples/window_preview.rs` 与 `tests/swapchain_smoke.rs` 里的具体调用行号。
8. `crates/deer-vk/render_out/gpu_raw.png` 的生成者（未在 `crates/deer-vk` 或 `crates/deer-gui` 找到写它的代码）。
9. `swapchain.rs` 的 `SwapchainFns`（`:419`）／`load_swapchain_fns`（`:480`）与 `Swapchain::create`（`:596`）之间的内部细节（未逐行读）。
10. `ffi.rs` 的 `Loader::inst_sym`（约 `:390-405`）与 `available_layers`/`available_extensions`（`:438`/`:478`）的内部实现细节（只读了文档级注释与 `:406-411` 的实测记录）。
11. 第 9.4 节「陈旧 `target/` 造成假红」—— 仓库里**没有**该现象的现成记载，该节内容是按构建特征推出的排查步骤，**未经实测验证**。
12. `tests/offscreen_render.rs` / `examples/gpu_offscreen.rs` 内部各调用的精确行号（只读了文件头与常量）。
