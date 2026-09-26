# 功能指南：GPU HAL（gpu-hal）

> 状态 🔄（trait 契约完整，CPU 后端已实现；Vulkan 后端部分实现）·
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

### Vulkan 后端（`deer_vk`）—— 🔄 部分

见 [`vulkan.md`](vulkan.md)。当前只到「设备 + 着色器」，**不能出图**。

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

## 6. 常见坑

| 现象 | 原因 | 怎么改 |
|---|---|---|
| 找不到 `name()` / `adapters()` | 忘了 `use deer_gpu::Backend;` | **trait 方法必须把 trait 引进作用域**（Rust 规则，新手常困惑） |
| 后端初始化 panic 了 | 违反了 HAL 契约 | 一律返回 `GpuError::NoAdapter` / `Unsupported` |
| 不知道 `RawWindowHandle` 怎么填 | 它是不透明形式，只带原生标识（`platform` / `handle` / `display`） | 窗口层还没定（Q-1），现在用不到 |
| 想直接拿到像素看 | `Frame::read_pixels()` | 也可绕过 HAL 用 `CpuRenderer::render` |

## 7. 相关

- 绘制列表（后端消费的东西）：[`draw-list.md`](draw-list.md)
- Vulkan 后端现状：[`vulkan.md`](vulkan.md)
- **做不到**：现在只有一个可用后端（CPU）；窗口/交换链没有平台实现；`Frame` 的 GPU 路径没验证过

## 8. 检查清单

- [x] 示例能跑：`cargo run -p deer-gui --example vulkan_devices` → `exit=0`
- [x] 示例有自检断言（适配器非空）
- [x] `FEATURES.md` 已登记
- [x] `docs/TUTORIAL.md` 已包含
- [x] 明确写了「做不到什么」
