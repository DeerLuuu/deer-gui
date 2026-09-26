# 功能指南：GPU HAL（gpu-hal）

> 状态 🔄（trait 契约完整；CPU 后端完整；Vulkan 后端的**设备/交换链/呈现已通**，
> 但 `DrawList → GPU` 仍是 M3）·
> 示例 `cargo run -p deer-gui --example vulkan_devices` ·
> 清单条目见 [`FEATURES.md`](../../FEATURES.md)

## 1. 这是什么 / 什么时候用它

**GPU 硬件抽象层**：一组 trait，规定「一个后端必须能做什么」。
本项目「自己写渲染后端」的关键就在这一层 —— **加一个后端 = 实现一个 trait**。

什么时候用它：
- 你想写自己的后端（软渲染 / Canvas / 别的图形 API）；
- 你想把 deer-gui 接到已有的绘制体系（例如嵌进你自己的引擎）；
- 你想知道「现在到底能做到哪一步」。

## 2. 五个 trait

| trait | 职责 | 关键方法 |
|---|---|---|
| `Backend` | 后端的入口 | `name()` / `adapters()` / `open(i)` |
| `Device` | 打开的设备 | `create_swapchain()` / `create_texture()` / `begin_frame()` / `wait_idle()` |
| `Swapchain` | 呈现在哪、多大 | `extent()` / `format()` / `resize()` |
| `Frame` | 一帧 | `record(&DrawList)` / `read_pixels()` / `submit_and_present()` |
| `Renderer` | 树+几何 → `DrawList` | `build_draw_list()` |

**约定（不是建议，是契约）**：
1. **初始化失败不许 panic** —— 必须返回 `GpuError`，让上层决定回退（例如换 CPU 后端）；
2. 资源句柄用不透明 id，**不暴露后端类型**；
3. 尺寸变化通过 `Swapchain::resize()` 表达，后端负责重建。

## 3. 现有两个后端

### CPU 后端（`deer_gpu::null`）—— ✅ 完整

纯软件光栅化，**不需要 GPU**。用途有两个：
1. 让整条渲染链在**无显卡/无驱动的 CI 里可断言**；
2. 作为 GPU 后端的**参考实现** —— 画面对不上时先拿它取正确像素。

```rust
let fb = deer_gpu::null::CpuRenderer::new()
    .render(deer_gpu::Extent { width: 320, height: 200 }, &list, theme.surface)?;
```

### Vulkan 后端（`deer_vk`）—— 🔄 部分（M2b 后：设备/交换链/呈现已通）

见 [`vulkan.md`](vulkan.md)。M2b 之后的真实状态：

| HAL 调用 | 现状 |
|---|---|
| `VkBackend::open(0)` | ✅ 返回**真设备**（M1 时是「未实现」，现在不是了）；越界索引返回 `Err` |
| `Device::create_swapchain(...)` | ✅ 真能建出可呈现的交换链（真机 `B8G8R8A8_SRGB` / FIFO / 3 张图） |
| `Device::begin_frame()` | ⚠️ **还没建交换链时明确报错**（错误信息带「交换链」），不给空帧 |
| `Frame::record(&DrawList)` | 空列表 / 只有 `NodeHint` ✅；**含真实绘制命令 ⇒ `Unsupported`**（文案指明「送上 GPU 是 M3」），**不静默忽略** |
| `Frame::read_pixels()` | ❌ `Unsupported`：HAL 在**提交前**调用，而交换链图像的回读数据只有**呈现之后**才有效 ⇒ 错误信息指向 `WindowedRenderer::read_back_last_frame()`；离屏回读用 `OffscreenRenderer`（见 [`gpu-offscreen.md`](gpu-offscreen.md)） |
| `Device::create_texture()` / `upload_texture()` | ❌ `Unsupported`（字形图集上传是 M3） |

上屏那条链走的是**专用快路** `deer_vk::windowed::WindowedRenderer`，见
[`vulkan-swapchain.md`](vulkan-swapchain.md)。

**HAL 窗口链的真实覆盖**在示例 `crates/deer-gui/examples/hal_window_path.rs`（进仓库门禁里跑）：
真窗口下走 `VkBackend::open(0)` → `Device::create_swapchain` → `begin_frame` / `record` / `submit_and_present` → `wait_idle`，
并断言 `record(含绘制命令)` ⇒ `Unsupported(M3)`：

```powershell
$env:DEER_VK_WINDOW_TESTS='1'; $env:DEER_VK_VALIDATION='1'; cargo run -q -p deer-gui --features window --example hal_window_path
# 本机实测 exit=0、640×480 / Bgra8Srgb、边界检查通过、校验层零消息（DEER_HAL_FRAMES 可改帧数，默认 30）
```

> **为什么这是示例而不是 `#[test]`**：winit 要求事件循环在**主线程**，而 `cargo test` 的 harness
> 在**子线程**里跑每个测试 ⇒ 真窗口那条链只能在示例里驱动。覆盖因此拆成两半：
> **映射契约**（`OutOfDate` 绝不当成功）由 `deer-vk/src/hal.rs` 的纯函数单测
> `present_outcome_mapping_never_treats_out_of_date_as_success` 守着（在 `cargo test` 里）；
> **真实路径**由上面的示例 + 门禁跑。
>
> 真窗口 e2e 测试（`swapchain_smoke`）**默认跳过**：完整门禁是
> `$env:DEER_VK_WINDOW_TESTS='1'; $env:DEER_VK_VALIDATION='1'; cargo test -p deer-vk` ——
> **不设 `DEER_VK_WINDOW_TESTS` 时跳过也算 pass，别把它当证据**。

## 4. 自己实现一个后端

最小骨架：

```rust
use deer_gpu::{Backend, Device, GpuError, GpuResult, AdapterInfo, AdapterKind};

struct MyBackend;

impl Backend for MyBackend {
    fn name(&self) -> &'static str { "my-backend" }

    fn adapters(&self) -> Vec<AdapterInfo> {
        vec![AdapterInfo { name: "my".into(), kind: AdapterKind::Cpu, driver: "built-in".into() }]
    }

    fn open(&self, adapter: usize) -> GpuResult<Box<dyn Device>> {
        if adapter != 0 { return Err(GpuError::NoAdapter); }
        Ok(Box::new(MyDevice))
    }
}

struct MyDevice;
impl Device for MyDevice {
    // …按 trait 补齐；**失败返回 Err，不要 panic**
}
# // 上面是示意，未实现全部方法
```

**实现顺序建议**：`Frame::record`（收下 `DrawList`）→ `read_pixels()`（回读）
→ 与 `CpuRenderer` 的输出**逐像素对照**。先对齐「矩形填充」一种命令，再加圆角、文字、裁剪。

## 5. 自检

```rust
// ① HAL 契约：越界适配器必须返回错误，不许 panic
assert!(backend.open(999).is_err());

// ② 后端名要有意义
assert_eq!(backend.name(), "cpu");

// ③ 若你的后端也走 CPU 路径，与参考实现逐字节对照
let mine = MyBackend::render(...)?;
let reference = deer_gpu::null::CpuRenderer::new().render(...)?;
assert!(mine.bytes_eq(&reference), "与参考实现的像素必须一致");
```

**本仓库自己的 HAL 覆盖怎么跑**（不是伪代码）：

```powershell
# ① 纯逻辑（在 cargo test 里）：映射契约 + 结构体布局等
cargo test -p deer-vk

# ② 真窗口 e2e 测试：**默认跳过**，要显式打开（跳过也算 pass，别当证据）
$env:DEER_VK_WINDOW_TESTS='1'; $env:DEER_VK_VALIDATION='1'; cargo test -p deer-vk

# ③ HAL 窗口链的真人肉验收（示例，主线程事件循环）
$env:DEER_VK_WINDOW_TESTS='1'; $env:DEER_VK_VALIDATION='1'; cargo run -q -p deer-gui --features window --example hal_window_path
```

## 6. 常见坑

| 现象 | 原因 | 怎么改 |
|---|---|---|
| 找不到 `name()` / `adapters()` | 忘了 `use deer_gpu::Backend;` | **trait 方法必须把 trait 引进作用域**（Rust 规则，新手常困惑） |
| 后端初始化 panic 了 | 违反了 HAL 契约 | 一律返回 `GpuError::NoAdapter` / `Unsupported` |
| 不知道 `RawWindowHandle` 怎么填 | 它是不透明形式，只带原生标识（`platform` / `handle` / `display`） | Q-1 **已决：引 `winit`**（`crates/deer-window`，已登记的依赖例外）；HAL 只收这个不透明句柄，所以 **`deer-vk` 仍零第三方依赖**。Windows 的填法见 [`window.md`](window.md) |
| 想直接拿到像素看 | `Frame::read_pixels()` 是 HAL 的回读接口，但它在**提交前**调用 | 自己实现后端时按契约实现它；**`deer-vk` 的 `read_pixels` 明确 `Unsupported` 并指向 `WindowedRenderer::read_back_last_frame()`**（回读数据只有呈现后有效）。CPU 路径可直接 `CpuRenderer::render`，离屏 Vulkan 用 `OffscreenRenderer` |

## 7. 相关

- 绘制列表（后端消费的东西）：[`draw-list.md`](draw-list.md)
- Vulkan 后端现状：[`vulkan.md`](vulkan.md)
- HAL 窗口路径示例：`crates/deer-gui/examples/hal_window_path.rs`（`--features window`，`DEER_VK_WINDOW_TESTS=1` 开关）
- **做不到**：`DrawList → GPU`（矩形/圆角/文本；M3）；纹理上传（M3）；
  推送常量矩形着色器（损坏，见 [`ROADMAP.md`](../../ROADMAP.md) Q-5）。
  **`deer-vk` 的 `Frame::read_pixels()` 明确 `Unsupported`** —— 不是「以后再补」而是语义选择：
  HAL 在提交前调用，回读数据只有呈现后才有效，所以请用 `WindowedRenderer::read_back_last_frame()`
  （见 [`vulkan-swapchain.md`](vulkan-swapchain.md)）。
  窗口/交换链**已有平台实现并真机验证**（Windows：`B8G8R8A8_SRGB` / FIFO / 3 张图，
  30 帧、校验层零消息、呈现帧像素已回读核对、`exit=0`），`Frame` 的 GPU 绘制路径**有明确边界**（非空 `DrawList` ⇒ `Unsupported`），
  只是「真的把命令画出来」还属于 M3。

## 8. 检查清单

- [x] 示例能跑：`cargo run -p deer-gui --example vulkan_devices` → `exit=0`
- [x] HAL 窗口路径示例能跑：`DEER_VK_WINDOW_TESTS=1 DEER_VK_VALIDATION=1 cargo run -q -p deer-gui --features window --example hal_window_path` → `exit=0`
- [x] 示例有自检断言（适配器非空）
- [x] `FEATURES.md` 已登记
- [x] `docs/TUTORIAL.md` 已包含
- [x] 明确写了「做不到什么」
