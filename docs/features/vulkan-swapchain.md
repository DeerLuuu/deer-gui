# 功能指南：Vulkan 上屏（surface / 交换链 / 呈现）

> 状态 ✅（**仅 Windows**）· 示例 `cargo run -p deer-gui --features window --example window_preview` ·
> 清单条目见 [`FEATURES.md`](../../FEATURES.md)

## 1. 这是什么 / 什么时候用它

把 **Vulkan 的 `VkSurfaceKHR` + 交换链 + 帧同步 + 呈现**接起来：让 GPU 画的东西真的出现在窗口上。
这条链是 M2b 的主体，位于 [`window.md`](window.md)（窗口/事件循环）与 `deer-vk`（Vulkan）之间。

**什么时候用它**：你要在自己的窗口里呈现 GPU 画面，或者你想直接用 `deer-vk` 的
surface/交换链 API（而不是通过门面）。

**什么时候不该用它**：只想出图/断言（用离屏路径，见 [`rendering.md`](rendering.md)）；
想在窗口里显示**界面**（`DrawList` 上屏是 **M3** —— 现在窗口里是清屏色 + M2a 验证过的几何）。

## 2. 最小示例

最省事的用法是**一步建好整条链**（`WindowedRenderer`）：

```rust
use deer_gui::window::{run, App, Flow, WindowConfig, WindowInfo};

struct Preview {
    renderer: Option<deer_gui::vk::windowed::WindowedRenderer>,
    frames: u32,
}

impl App for Preview {
    fn init(&mut self, info: &WindowInfo) -> Result<(), String> {
        // adapter 0 / 想要的尺寸 / 清屏色
        let r = deer_gui::vk::windowed::WindowedRenderer::new(
            0,
            info.raw,
            info.extent,
            deer_gui::gpu::Color::rgb(0x10, 0x14, 0x20),
        )
        .map_err(|e| format!("建渲染器失败：{e}"))?;
        println!(
            "适配器={} 格式={} present mode={} 图像数={}",
            r.adapter().name,
            r.format(),
            r.present_mode(),
            r.image_count()
        );
        self.renderer = Some(r);
        Ok(())
    }

    fn redraw(&mut self) -> Result<Flow, String> {
        let r = self.renderer.as_mut().expect("init 里建好了");
        match r.render_and_present().map_err(|e| e.to_string())? {
            deer_gui::vk::windowed::FrameOutcome::Presented => self.frames += 1,
            // **过期不是成功**：resize 之后重试，绝不要当成已经画好
            deer_gui::vk::windowed::FrameOutcome::OutOfDate => {
                r.resize(r.extent()).map_err(|e| e.to_string())?;
            }
        }
        Ok(if self.frames >= 120 { Flow::Exit } else { Flow::Continue })
    }
}

fn main() -> Result<(), String> {
    run(WindowConfig::new("swapchain", 320, 200), Preview { renderer: None, frames: 0 })
}
```

跑本仓库的完整示例（**feature 必须带上**）：

```sh
cargo run -p deer-gui --features window --example window_preview
```

## 3. 完整 API

### `deer_gpu::RawWindowHandle` —— 唯一的耦合点

窗口层与渲染层**只通过它耦合**：`platform` + `handle`（Windows 上是 HWND）+ `display`（HINSTANCE）。
`deer-vk` **不依赖 winit**，所以换窗口实现不影响这里。

### `deer_vk::surface::Surface`

| 项 | 语义 |
|---|---|
| `SURFACE_EXTENSION: &str` | `"VK_KHR_surface"` |
| `WIN32_SURFACE_EXTENSION` / `XLIB_...` / `XCB_...` / `WAYLAND_...` / `METAL_...` | 各平台 surface 扩展名常量 |
| `Surface::create(instance, window: RawWindowHandle) -> GpuResult<Surface>` | 句柄 → `VkSurfaceKHR`（Windows 走 `vkCreateWin32SurfaceKHR`）；**非 Windows 明确返回 `Unsupported`**，不静默 |
| `Surface::handle() -> SurfaceHandle` | 原始句柄 |
| `Surface::platform_extension(platform) -> &'static str` | 平台扩展名（Windows → `"VK_KHR_win32_surface"`） |
| `Surface::platform() -> Platform` | 这个 surface 是哪个平台的 |
| `Drop` | `vkDestroySurfaceKHR` |

实例侧配套（`deer_vk::ffi::Instance`）：

| 方法 | 语义 |
|---|---|
| `Instance::create_with_extensions(validation: bool, extensions: &[&str])` | 按需带实例扩展创建（既有 `create_with_validation` 行为不变） |
| `Instance::enabled_extensions() -> &[String]` | 实际启用的扩展（不是"请求了就算"） |
| `Instance::extension_available(name: &str) -> bool` | 查 `vkEnumerateInstanceExtensionProperties`，**不靠猜** |

### `deer_vk::device::VkDevice`（呈现相关）

| 方法 | 语义 |
|---|---|
| `VkDevice::open_with_present(adapter_index, &Surface) -> GpuResult<VkDevice>` | 打开设备并拿到一个**同时支持图形 + 呈现**的队列族；拿不到就明确报错（「本机这个设备不支持在该窗口上呈现」） |
| `present_queue() -> QueueHandle` | 呈现队列 |
| `present_queue_family_index() -> u32` | 队列族索引 |

### `deer_vk::swapchain::{Swapchain, SwapchainConfig, Acquire, Present, Semaphore}`

```rust
SwapchainConfig { pub extent: Extent, pub format: i32, pub present_mode: i32, pub image_count: u32 }
pub enum Acquire { Image(u32), OutOfDate, Suboptimal }
pub enum Present { Presented, OutOfDate, Suboptimal }
```

| 方法 | 语义 |
|---|---|
| `choose_config(&Surface, pd, want: Extent) -> GpuResult<SwapchainConfig>` | 挑一组**确定性**的合法配置（策略见下） |
| `create(&VkDevice, &Surface, cfg, old: Option<&Swapchain>) -> GpuResult<Swapchain>` | 建交换链（`old` 非空时走 `oldSwapchain` 复用；**新链建好后由调用方销毁旧链**，`windowed.rs::resize` 负责顺序）；同时建每个 image 的 `VkImageView`。`extent` 有 0 或 `image_count == 0` ⇒ 明确 `Err` |
| `extent()` / `format()` / `present_mode()` / `image_count()` | 查询 |
| `images()` / `image_views()` | 图像与视图句柄切片 |
| `acquire(image_available: SemaphoreHandle) -> GpuResult<Acquire>` | 取下一张图；内部有 **1 秒**等待上限（`DEFAULT_ACQUIRE_TIMEOUT_NS`，见下） |
| `acquire_with_timeout(image_available, timeout_ns) -> GpuResult<Acquire>` | 同上，自己给等待上限 |
| `present(queue, render_finished: SemaphoreHandle, image_index: u32) -> GpuResult<Present>` | 呈现 |
| `Semaphore` | 信号量；`Semaphore::create(device)` / `handle()` / `Drop` 由本模块管 |

**纯逻辑助手**（不碰真机、可单测 —— 也是「返回码映射」的唯一真相）：

| 项 | 语义 |
|---|---|
| `pick_config(caps, formats, present_modes, want) -> GpuResult<SwapchainConfig>` | `choose_config` 的纯函数内核（能力列表 → 配置） |
| `map_acquire_result(rc, image_index) -> GpuResult<Acquire>` | 返回码 → `Acquire` |
| `map_present_result(rc) -> GpuResult<Present>` | 返回码 → `Present` |
| `DEFAULT_ACQUIRE_TIMEOUT_NS` | `1_000_000_000`（1 秒）：宁可失败也不无限挂住 |
| `SWAPCHAIN_EXTENSION` | `"VK_KHR_swapchain"`（实例/设备扩展名） |

**`choose_config` 的确定性策略**（同一台机器同样的能力列表 ⇒ 同样的结果）：

1. **格式**：优先 `B8G8R8A8_SRGB`，其次 `R8G8B8A8_SRGB`；都没有就取第一个非 `UNDEFINED` 的格式；
   全是 `UNDEFINED`（规范允许任意格式）时用 `B8G8R8A8_SRGB` + 驱动报的 color space；
2. **present mode**：优先 **FIFO**（规范保证支持）；没有 FIFO 就 `FIFO_RELAXED`；再没有就取驱动报的**第一个**。
   **不做** mailbox / 立即模式的自适应选择；
3. **extent**：驱动给了 `currentExtent` 就用它（仍会夹取），否则把 `want` **夹到**
   `minImageExtent..=maxImageExtent`（并且每个维度至少 1 像素；`min > max` 的退化能力也不 panic）；
4. **image_count**：`minImageCount + 1`（多一张少等一次），上限 `maxImageCount`（`0` = 无上限），下限 1；
5. 能力列表为空（没有任何格式 / 没有任何 present mode）⇒ 明确 `Err(Unsupported)`，不猜；
6. `want` 只在能力允许时被采信 —— 不会被静默忽略，也不会越界。

**返回码怎么处理**（这是上屏最常见的崩溃源）：

| 返回码 | 映射到 | 你必须做什么 |
|---|---|---|
| `VK_SUCCESS` | `Acquire::Image(i)` / `Present::Presented` | 正常继续 |
| `VK_ERROR_OUT_OF_DATE_KHR` | `Acquire::OutOfDate` / `Present::OutOfDate` | **resize 重建交换链后重试** —— 不许当成功 |
| `VK_SUBOPTIMAL_KHR` | `Acquire::Suboptimal` / `Present::Suboptimal` | 本帧可以先画/先呈现，但**应当尽快重建**交换链以适应新尺寸 |
| `VK_TIMEOUT` / `VK_NOT_READY` / 其它 | `Err(...)` | 明确报错，**不得当成功**（`Acquire::Image` 的索引在非成功码下是未初始化的，拿去录命令就是崩溃源） |

### `deer_vk::windowed::WindowedRenderer` —— 一条完整链

`WindowedRenderer::new(adapter_index, window, want: Extent, clear: Color)` 内部按顺序做：

```
实例（含 VK_KHR_surface + VK_KHR_win32_surface，缺失即报错）
  → 设备（图形 + 呈现队列族）
  → Surface（从 RawWindowHandle）
  → 交换链（choose_config）
  → 渲染通道（格式 = 交换链格式）
  → 三角形管线（复用 M2a 的 SPIR-V）
  → 每帧命令缓冲 + 信号量/栅栏同步
```

| 方法 | 语义 |
|---|---|
| `adapter() -> &AdapterInfo` | 用的是哪块卡 |
| `extent()` / `format()` / `present_mode()` / `image_count()` | 实际配置（**真机值**） |
| `frames_presented() -> u64` | **已提交并呈现**的帧数（`Suboptimal` 那帧也算 —— 它确实呈现了） |
| `suboptimal_frames() -> u64` | 出现过 `Suboptimal` 的帧数（诊断「该重建交换链了」） |
| `instance_extensions() -> &[String]` | 这条链**实际启用**的实例扩展（自检 `VK_KHR_surface` 真的开了） |
| `resize(Extent) -> GpuResult<()>` | 重建交换链以及依赖它的帧缓冲/命令缓冲 |
| `render_and_present() -> GpuResult<FrameOutcome>` | 画一帧并呈现；`FrameOutcome::{Presented, OutOfDate}`，**过期不 panic、不假装成功** |
| `wait_idle() -> GpuResult<()>` | 等 GPU 空闲（关闭/截图/销毁资源前） |
| `read_back_last_frame() -> GpuResult<Vec<u8>>` | 取回**刚呈现那帧**的像素：**RGBA8**，长度 = `宽 × 高 × 4`（`B8G8R8A8_*` 会自动换成 R/B 顺序）。**必须在 `render_and_present()` 之后**、且中间没 `resize` 过；**会强制一次 GPU→CPU 同步**（等最近一帧的 in-flight 栅栏，5 秒有限超时）⇒ 适合验收/截图，**不适合每帧调**。关掉回读或 resize 之后调用会**明确报错，绝不用陈旧数据冒充** |
| `set_readback_enabled(bool) -> GpuResult<()>` / `readback_enabled()` / `readback_available()` | 回读开关（**默认开**）。关掉可省每帧一次全屏 `image → buffer` 复制（`宽×高×4` 字节带宽 + barrier 往返），代价是 `read_back_last_frame()` 明确报错 |
| `clear_color_value(color) -> [f32; 4]`（模块级纯函数） | HAL 颜色 → Vulkan 清屏值，**不做 sRGB 变换、不换通道、不 clamp** —— 「传进去什么色就清什么色」能被逐位断言 |
| `srgb_encoded_byte(linear: u8) -> u8`（模块级纯函数） | 线性字节 → **sRGB 编码字节**（sRGB 传输函数：阈值 `0.0031308` 分段）。回读断言的**正确参考值**，见第 5 节的 sRGB 坑 |

公开常量 `FRAMES_IN_FLIGHT = 2`：每帧 in-flight 的槽位数（命令缓冲/信号量/栅栏按它分配）。

**回读的代价（诚实边界）**：开启回读后，每帧在 present 前多录一次 `image → buffer` 复制 + 两次 barrier；
`read_back_last_frame()` 本身还会**强制一次 GPU→CPU 同步**。所以**不适合每帧调**——本仓库的示例
只在第一帧取样一次。FIFO 垂直同步下这份开销基本被垂直同步掩盖（**某次实测快照**：960×600、60 帧，
回读开/关约 52.1 vs 52.2 帧/秒；**随机器与负载浮动** —— 验证者复现为 59.3 vs 60.2、30 帧约 45.2 fps，
结论一致）。

`render_and_present()` 的精确语义（**与「OutOfDate 怎么处理」直接相关**）：

- 每帧走「等栅栏（上限 5 秒）→ acquire → 录命令 → submit → present」，**用了 2 份帧资源**（`FRAMES_IN_FLIGHT`）；
- `Acquire::OutOfDate` ⇒ **不提交、不 reset 栅栏**，直接返回 `OutOfDate`（所以重试很便宜）；
- `Acquire::Suboptimal` ⇒ 驱动仍写回了有效图像索引，照常画完；
- **`FrameOutcome::OutOfDate` 同时覆盖「真过期」与「`Suboptimal`」**：两者都表示「该 `resize` 重建交换链了」，
  区别用 `suboptimal_frames()` 看（`Suboptimal` 的那一帧**已经呈现成功**，所以 `frames_presented()` 仍会 +1）；
- 帧缓冲数与交换链图像数不一致（例如上一次 resize 失败）⇒ 明确 `Err`，提示「再调一次 `resize`」；
- `Drop`：先 `vkDeviceWaitIdle`（失败也继续，只打 stderr），再按依赖倒序销毁 frames → views → swapchain → 管线/渲染通道 → surface → device → instance。

### HAL 侧（`VkBackend` / `VulkanDevice`）的 M2b 边界

窗口路径（`WindowedRenderer`）是**专用快路**；如果你走 HAL（`deer_gpu::Backend`/`Device`/`Frame`），
M2b 的真实状态是：

| HAL 调用 | M2b 现状 |
|---|---|
| `VkBackend::open(0)` | ✅ **返回真设备**（不再是「未实现」）；越界索引返回 `Err`，不 panic |
| `Device::create_swapchain(window, extent, format)` | ✅ 真能建出可呈现的交换链；图像用途申请 `COLOR_ATTACHMENT \| TRANSFER_SRC`（回读需要后者），`supportedUsageFlags` 缺任何一个都**明确报错**，不静默去掉 |
| `Device::begin_frame()` | ⚠️ **还没建交换链时明确报错**（错误信息里带「交换链」），不给空帧 |
| `Frame::record(&DrawList)` | 空列表 / 只有 `NodeHint` ✅；**含真实绘制命令 ⇒ `Unsupported`**（文案指明「送上 GPU 是 M3」）。**绝不静默忽略** |
| `Frame::read_pixels()` | ❌ `Unsupported`，**错误信息直接指向 `WindowedRenderer::read_back_last_frame()`** —— HAL 的 `read_pixels` 在**提交前**调用，而交换链图像的回读数据只有**呈现之后**才有效；deer-vk 刻意不做「隐式呈现」这种惊吓式语义。离屏回读请用 `OffscreenRenderer` |
| `Device::create_texture` / `upload_texture` | ❌ `Unsupported`（字形图集上传是 M3） |

所以 **HAL 的 `Frame` 还不是「能画界面」的帧**：窗口里目前只有清屏色 + M2a 的几何三角形。

> **校验层开关（如实登记）**：`DEER_VK_VALIDATION=1` 在三条路径上一致生效 —— `VkBackend::new`、
> 设备/离屏路径与**窗口路径**（`WindowedRenderer`）都读它（设备路径一度故意不读，t18 修完 3 个
> offscreen 真缺陷后重新接回）。唯一遗留是 `spirv.rs::vertex_shader_rect_pushconstant` 那支
> 推送常量矩形着色器是坏的（校验层报 `VUID-StandaloneSpirv-PushConstant-06808`，
> **请求校验层时会让进程访问违例崩溃**，0xc0000005）—— 所以 `tests/device_smoke.rs` 与
> `tests/pipeline_smoke.rs` 里涉及它的测试**在校验层下显式跳过**（t15，明确打印「不是通过，是被跳过」）。
> 本机实测：`DEER_VK_VALIDATION=1 cargo test -p deer-vk` → **167 passed / 0 failed、零校验消息**（数字随测试增减漂移，**以运行输出为准**）；
> 窗口示例 30 帧 → 零消息、`exit=0`。修好那支着色器属于 **M3**（矩形改顶点缓冲），登记见
> [`ROADMAP.md`](../../ROADMAP.md) 的 Q-5。

## 4. 自检（怎么确认你真的用对了）

上屏最容易「看着像在跑，其实一帧都没呈现」，所以断言要落在**帧数**和**过期处理**上：

```rust
// ① 帧数必须是「成功呈现」的次数，而不是「redraw 被调了几次」
assert!(r.frames_presented() >= 10, "至少要有 10 帧真的呈现成功");

// ② 交换链配置必须与窗口尺寸一致（extent 被夹取过也要一致）
let (w, h) = (r.extent().width, r.extent().height);
assert!(w > 0 && h > 0, "交换链 extent 不可能是 0");

// ③ 过期绝不能当成功：render_and_present 返回 OutOfDate 时必须 resize 后重试
match r.render_and_present()? {
    FrameOutcome::Presented => {}
    FrameOutcome::OutOfDate => { r.resize(r.extent())?; }  // 下一帧再试
}
```

整条链的端到端自检就是示例本身（真窗口、真呈现、退出干净）：

```powershell
$env:DEER_WINDOW_FRAMES='30'; cargo run -p deer-gui --features window --example window_preview
# 期望：打印窗口尺寸 / 适配器 / 交换链格式 / present mode / 图像数 / 呈现帧数 / 过期次数 / 帧率，退出码 0
```

**本机实测**（Windows / Intel RaptorLake-S 集显 / `DEER_VK_VALIDATION=1` + `DEER_WINDOW_FRAMES=30`，`exit=0`）：

```text
[deer-window] 窗口已建：title="deer-gui — M2b 窗口预览（GPU 清屏 + 几何）" extent=960x600 platform=Windows handle(HWND)=0xFE0BA8 display(HINSTANCE)=0x7FF7514D0000
适配器      : Intel(R) RaptorLake-S Mobile Graphics Controller（index=0）
交换链      : 960×600 / format 0x00000032 / present mode 2 / 3 张图
呈现帧数    : 30 ｜ 交换链过期 : 0 次 ｜ 耗时 : 0.66 s（45.1 帧/秒）
像素回读    : 四角 [71, 79, 105, 255] = sRGB 编码后的清屏色 rgb(0x10,0x14,0x24)
              中心 [160, 206, 255, 255]；非清屏色像素 99360 / 576000（17.25%）
自检通过 ✅（帧数达标、交换链未持续过期、**呈现帧像素已核对**、事件循环正常退出）
```

`format 0x32 = 50 = VK_FORMAT_B8G8R8A8_SRGB`、`present mode 2 = VK_PRESENT_MODE_FIFO_KHR`、
`3 张图 = minImageCount(2) + 1` —— 与上面的确定性策略逐条对得上。
（显示设备/帧率不同会有不同数值，**以你自己的输出为准**。）

带 Vulkan 校验层再跑（t13 的验收项）：

```powershell
$env:DEER_VK_VALIDATION='1'; $env:DEER_WINDOW_FRAMES='30'; cargo run -p deer-gui --features window --example window_preview
# 会看到 [deer-vk] 窗口路径已启用 VK_LAYER_KHRONOS_validation；本机实测零条 VUID/Validation Error，exit=0

$env:DEER_VK_VALIDATION='1'; cargo test -p deer-vk
# 本机实测 167 passed / 0 failed、零条校验消息（设备/离屏/窗口三条路径都开了校验层，t18 起；数字随测试增减漂移，以运行输出为准）

# 真窗口 e2e（swapchain_smoke 里的那条）**默认跳过**，必须显式打开：
$env:DEER_VK_WINDOW_TESTS='1'; $env:DEER_VK_VALIDATION='1'; cargo test -p deer-vk
```

> **不设 `DEER_VK_WINDOW_TESTS` 时真窗口 e2e 会显式跳过 —— 跳过也算 pass，所以别把它当证据**
> （验证者实测：把 `windowed.rs::resize` 改成空操作后，不设该变量时 25/25 全绿、设了才 1 failed）。
> HAL 路径的真实验收是示例 `hal_window_path`（`DEER_VK_WINDOW_TESTS=1` 开关，
> `DEER_HAL_FRAMES` 可改帧数，默认 30）：
>
> ```powershell
> $env:DEER_VK_WINDOW_TESTS='1'; $env:DEER_VK_VALIDATION='1'; cargo run -q -p deer-gui --features window --example hal_window_path
> # 本机实测 exit=0、640×480 / Bgra8Srgb、校验层零消息
> ```

示例还认这几个环境变量：`DEER_WINDOW_ADAPTER`（选显卡，默认 0）、`DEER_WINDOW_HOLD=1`
（留窗观察，不做帧数断言）、`DEER_VK_VALIDATION=1`（开校验层）、`DEER_WINDOW_READBACK=0`
（关掉第一帧的像素回读，自检随之**显式降级**、不再给像素级证据）。

## 5. 常见坑

| 现象 | 原因 | 怎么改 |
|---|---|---|
| **回读/截图的颜色比预期亮**（清屏 `rgb(0x10,0x14,0x24)` 回读成 `[71,79,105]` 而不是 `[16,20,36]`） | 交换链是 **`B8G8R8A8_SRGB`**：Vulkan 规定写进 sRGB 附件的**颜色值在线性空间、由驱动做 sRGB 编码**，所以回读到的字节是**编码后**的值。这是规范行为，不是 bug | 用公开纯函数 `deer_vk::windowed::srgb_encoded_byte()` 算期望值（`0x10→0x47`、`0x14→0x4F`、`0x24→0x69`，本机 Intel/NVIDIA 实测逐位一致）；想要「逐字节直通」就改用**线性格式**（`*_UNORM`）的交换链 |
| 花大力气断言像素却和窗口上看到的不一样 | `read_back_last_frame()` 返回的是**图像里的字节**（sRGB 格式下即 sRGB 编码值），**不做线性化** —— 这样才和窗口/截图工具逐字节一致 | 按「编码值」断言（见上一行）；要线性值自己反编码 |
| 每帧都调 `read_back_last_frame()` 后帧率掉 | 回读**强制一次 GPU→CPU 同步**（等最近一帧的 in-flight 栅栏），外加每帧一次 `宽×高×4` 的 image→buffer copy | **只在需要证据的那一帧调一次**（示例只在第一帧）；不需要就 `set_readback_enabled(false)` 省掉每帧 copy |
| 关掉回读后 `read_back_last_frame()` 报错 | 这是**刻意的**：宁可用错也不返回陈旧数据 | 先 `set_readback_enabled(true)`，再 `render_and_present()`，然后才回读 |
| 窗口一拉大就崩 / 花屏 | 把 `OutOfDate` 当成功继续用了 | `OutOfDate` ⇒ `resize` 重建后重试；`Suboptimal` 尽快重建（见上表） |
| 每帧都 OutOfDate，永远画不出 | 交换链创建时的 extent 与窗口实际尺寸不一致（比如自己算错了 DPI 缩放） | 用 `WindowInfo.extent`（物理像素）原样传；resize 也用事件给的物理像素 |
| `VK_ERROR_SURFACE_LOST_KHR` 之类报错 | surface 句柄在窗口销毁后还在用 | 先 `wait_idle()` 再销毁；窗口生命周期由 `run()` 管，别在窗口外持有句柄 |
| 建渲染器时报「本机这个设备不支持在该窗口上呈现」 | 选的适配器（比如独显）没有该表面的呈现队列 | 换适配器索引；或用带呈现支持的设备 |
| 窗口最小化后 `resize`/`create` 返回 `Err`（尺寸为 0） | 最小化时 `Resized` 给 0×0，交换链不允许 0 尺寸 | 把尺寸夹到 **≥1**（示例就是这么做的），或最小化期间**先不重建** |
| `render_and_present` 报「帧缓冲与交换链图像数不一致 ⇒ 请再调一次 resize」 | 上一次 `resize` 失败过（半成品状态），或自己直接改了交换链 | 再调一次 `resize` 让它重建到一致；不要绕过 `WindowedRenderer` 自己动交换链 |
| 非 Windows 报 `Unsupported` | surface 的平台分支没实现（`vkCreateWin32SurfaceKHR` 是唯一实现） | 见 [`window.md`](window.md) 第 6 节 |
| 画面撕裂 / 帧率不受控 | present mode 是 FIFO（跟随显示刷新），且**没有帧率上限** | 这是刻意的确定性策略；要别的模式得自己改 `choose_config` |
| 窗口里没有界面 | **M2b 的真实边界**：这条链送上去的是清屏色 + 三角形，不是 `DrawList` | 把界面送上 GPU 是 M3 |

## 6. 相关

- 窗口与事件循环：[`window.md`](window.md)
- 离屏渲染：[`rendering.md`](rendering.md)
- Vulkan 设备/管线/离屏回读（M2a）：[`vulkan.md`](vulkan.md)、[`vulkan-pipeline.md`](vulkan-pipeline.md)、[`gpu-offscreen.md`](gpu-offscreen.md)
- **做不到**（本模块的边界）：
  - **只有 Windows**：`VK_KHR_win32_surface` 是唯一实现的平台分支。
  - **没有 mailbox / 立即呈现模式**：`choose_config` 只挑 FIFO。
  - **没有独占全屏、没有 HDR / 色彩管理**（不做 `VK_EXT_swapchain_colorspace` 之类的选择）。
  - **没有帧率上限**，也没有「跳帧」策略。
  - **不消费 `DrawList`**：界面（文本/控件）上屏属于 **M3**；M2b 只保证「GPU 画的像素能出现在窗口上」。
  - **交换链重建会重建帧缓冲/命令缓冲**：没有做「按 image 预分配并跨 resize 复用」的小优化。

## 7. 检查清单（发布前过一遍）

- [x] 示例能跑：`cargo run -p deer-gui --features window --example window_preview` → `exit=0`
- [x] 示例有自检断言（帧数 / 无 OutOfDate 泄漏 / 退出干净）
- [x] `FEATURES.md` 已登记（状态 / 指南链接 / 示例命令都对）
- [x] 如果属于新手主线，`docs/TUTORIAL.md` 已更新（第 12 章）
- [x] 明确写了「做不到什么」
