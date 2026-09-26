# 功能指南：窗口（window）

> 状态 ✅（**仅 Windows**）· 示例 `cargo run -p deer-gui --features window --example window_preview` ·
> 清单条目见 [`FEATURES.md`](../../FEATURES.md)

## 1. 这是什么 / 什么时候用它

打开一个**真实的操作系统窗口**，跑**事件循环**，并把窗口的原生句柄以 HAL 的不透明形式
（`deer_gpu::RawWindowHandle`）交给渲染后端。这是 M2b 的第一步：**从「离屏出图」走到「屏幕上能看到」**。

**什么时候用它**：你要把 GPU 画出来的东西显示在一个真窗口里、或者你要给 `deer-vk`
的 surface / 交换链接一个真窗口句柄。

**什么时候不该用它**：只想出图/做断言（用 [`rendering.md`](rendering.md) 的
`render_tree_to_png`，无窗口、CI 友好）；想用鼠标键盘交互（**输入事件是 M5**，见第 6 节）；
想要窗口里显示**界面**（现在窗口里是 GPU 清屏色 + M2a 验证过的几何，**把 `DrawList` 送上 GPU 是 M3**）。

> ⚠️ **窗口层是本项目唯一引入第三方依赖的地方**（`winit`，登记在
> [`ROADMAP.md`](../../ROADMAP.md) 的 Q-1）。`deer-layout` / `deer-gpu` / `deer-vk`
> 仍然零第三方依赖；只有 `deer-window`、以及**开了 `window` feature** 的 `deer-gui` 会拿到它。

## 2. 最小示例

```rust
use deer_gui::window::{run, App, Flow, WindowConfig, WindowInfo};

/// 自己实现 `App`：窗口的生命周期由 `run()` 管，你只管三件事（init / redraw / 关闭）。
struct Preview {
    frames: u32,
}

impl App for Preview {
    /// 窗口建好后**调一次**：在这里创建渲染器（`WindowedRenderer` / 交换链）。
    fn init(&mut self, info: &WindowInfo) -> Result<(), String> {
        println!(
            "窗口 {}×{}，原生句柄 {:#x}",
            info.extent.width, info.extent.height, info.raw.handle
        );
        Ok(())
    }

    /// 每帧一次；返回 `Flow::Exit` 请求结束事件循环。
    fn redraw(&mut self) -> Result<Flow, String> {
        self.frames += 1;
        Ok(if self.frames >= 60 { Flow::Exit } else { Flow::Continue })
    }
}

fn main() -> Result<(), String> {
    // 必须在**主线程**调用（winit 的要求，见第 5 节）
    run(WindowConfig::new("hello", 320, 200), Preview { frames: 0 })
}
```

跑本仓库的完整示例（**feature 必须带上**，否则 Cargo 会报「requires the features: window」）：

```sh
cargo run -p deer-gui --features window --example window_preview
```

产物/现象：弹出一个真窗口，连续呈现若干帧后自动退出（退出码 0）。
帧数与「是否保持开着」由环境变量控制（见第 3 节）。

**本机实测**（Windows，`DEER_VK_VALIDATION=1` + `DEER_WINDOW_FRAMES=30`，`exit=0`）：

```text
连续呈现 30 帧后自动退出（DEER_WINDOW_FRAMES 可改）
第一帧会回读像素并核对（DEER_WINDOW_READBACK=0 可关，关掉就没有像素级证据）
[deer-window] 窗口已建：title="deer-gui — M2b 窗口预览（GPU 清屏 + 几何）" extent=960x600 platform=Windows handle(HWND)=0xFE0BA8 display(HINSTANCE)=0x7FF7514D0000
[deer-vk] 窗口路径已启用 VK_LAYER_KHRONOS_validation（消息打到 stderr）
适配器      : Intel(R) RaptorLake-S Mobile Graphics Controller（index=0）
交换链      : 960×600 / format 0x00000032 / present mode 2 / 3 张图

呈现帧数    : 30
交换链过期  : 0 次（过期必须重建后重试，不能当成功）
像素回读    : 四角 [71, 79, 105, 255] = sRGB 编码后的清屏色 rgb(0x10,0x14,0x24)
              中心 [160, 206, 255, 255]；非清屏色像素 99360 / 576000（17.25%）
自检通过 ✅（帧数达标、交换链未持续过期、**呈现帧像素已核对**、事件循环正常退出）
[deer-window] 事件循环结束：frames=30 extent=960x600 result=ok
```

> **像素回读 + sRGB 坑**：示例第一帧会用 `WindowedRenderer::read_back_last_frame()` 取样一次
> （会**强制一次 GPU→CPU 同步**，所以别每帧调）。注意回读到的**不是** `rgb(0x10,0x14,0x24)` 而是
> **`[71,79,105]`** —— sRGB 附件的驱动编码，详见 [`vulkan-swapchain.md`](vulkan-swapchain.md) 第 5 节。

（窗口尺寸/适配器帧率随机器而变，**以你自己的输出为准**。）

还有一个**专测 HAL 路径**的示例（同一棵树走 `VkBackend::open(0)` → `Device::create_swapchain` →
`begin_frame` / `record` / `submit_and_present` → `wait_idle`，并断言 `record(含绘制命令)` ⇒ `Unsupported(M3)`）：

```powershell
$env:DEER_VK_WINDOW_TESTS='1'; $env:DEER_VK_VALIDATION='1'; cargo run -q -p deer-gui --features window --example hal_window_path
# 本机实测：exit=0、640×480 / Bgra8Srgb、边界检查通过、校验层零消息
# 帧数用 DEER_HAL_FRAMES 改（默认 30）；不设 DEER_VK_WINDOW_TESTS 时会**显式跳过并说明「这不是通过」**
```

## 3. 完整 API

入口在 `deer_gui::window`（它就是 `deer-window` 的再导出）。**这个模块只在 `window` feature 下存在**
（`#[cfg(feature = "window")]`）—— 不开 feature 时整个模块不存在，这正是「不写窗口代码就不引入 winit」的实现方式。

### `WindowConfig` —— 窗口参数

| 项 | 语义 |
|---|---|
| `WindowConfig { pub title: String, pub width: u32, pub height: u32 }` | 标题 + **逻辑**尺寸（建窗时用 `LogicalSize`，由系统按 DPI 换算成物理像素） |
| `WindowConfig::new(title: impl Into<String>, width: u32, height: u32)` | 三个字段**逐字存入，不做钳制**（0 也照存，由调用方负责合理值） |
| `WindowConfig::default()` | 空标题 + **800×600** |
| `display_title() -> String` | 标题栏实际显示的文字：`deer-gui — <title>`；**标题为空时只显示 `deer-gui`**（不留孤立分隔符） |

- 目前没有「是否可缩放 / 置顶 / 无边框」之类的开关 —— 只有标题与尺寸。

### `WindowInfo` —— 交给渲染层的窗口信息

```rust
WindowInfo {
    pub raw: deer_gpu::RawWindowHandle,  // Windows: handle=HWND，display=HINSTANCE
    pub extent: deer_gpu::Extent,        // 当前尺寸（物理像素）
}
```

- `raw` 是 **HAL 的不透明句柄**：`deer-vk` 只认它，**不依赖 winit** —— 所以换窗口实现不用动渲染层。
- `init(&WindowInfo)` 拿到的是**建窗后**的尺寸；之后尺寸变化走 `resized`。

### `Flow` —— 事件循环的去留

```rust
pub enum Flow { Continue, Exit }
```

`App::redraw` 返回它；`Flow::Exit` ⇒ `run()` 请求 `event_loop.exit()` 并正常返回 `Ok(())`。

### `App` trait —— 你实现它

| 方法 | 何时被调用 | 必须实现 |
|---|---|---|
| `init(&mut self, info: &WindowInfo) -> Result<(), String>` | 窗口建好后**一次**（创建渲染器的地方） | ✅ |
| `resized(&mut self, width: u32, height: u32) -> Result<(), String>` | 尺寸变化（含 DPI 变化），**物理像素**直接透传；应当重建交换链 | 默认空实现 |
| `redraw(&mut self) -> Result<Flow, String>` | 每帧（窗口要求重绘时），**含呈现** | ✅ |
| `close_requested(&mut self) -> Flow` | 点关闭按钮 / 系统关闭请求 | 默认 `Flow::Exit`（允许关闭） |

回调返回 `Err(String)`：`run()` 会把原因打到 stderr、请求退出，并**最终把 `Err` 返回给你**
（不会吞掉错误假装正常退出）。

### `run()` —— 建窗口 + 跑事件循环

```rust
pub fn run<A: App + 'static>(config: WindowConfig, app: A) -> Result<(), String>
```

- **必须在主线程调用**（winit 的要求；`EventLoop::new()` 失败时 `run()` 会把原因写进 `Err`）。
- 正常结束（`Flow::Exit` / 窗口关闭）返回 `Ok(())`，并往 stdout 打一行摘要，便于脚本断言：
  `[deer-window] 事件循环结束：frames=<n> extent=<w>x<h> result=ok|error`
  建窗时还会打一行：`[deer-window] 窗口已建：title="..." extent=<w>x<h> platform=... handle(HWND)=0x... display(HINSTANCE)=0x...`
- 建窗失败 / 回调返回 `Err` ⇒ 原因打到 stderr、摘要里 `result=error`、**最终把 `Err` 返回给你**（不吞错、不 panic）。
- 帧数是 `run()` 自己数的：`App::redraw` **成功**返回那次才算一帧（`Flow::Exit` 的那一帧也算）。
- 非 Windows 平台：winit 那边**能开窗**，但「原生句柄 → HAL 句柄」这一步未实现，
  `run()` 会明确返回 `Err(UNSUPPORTED_PLATFORM_MSG)`（**不静默填 0**）。

### 诊断 / 测试用的小件（公开）

| 项 | 语义 |
|---|---|
| `FrameCounter` | `run()` 内部账本的**纯逻辑核心**（不碰窗口，可单测）：`new()`、`on_redraw(result)`（`Ok` 才 +1，`Flow::Exit` 那帧也算画成功；`Err` 原样返回且不计数）、`frames()`、`exit_requested()` |
| `raw_handle_from_win32(hwnd, hinstance)` | 把 `(HWND, HINSTANCE)` 打包成不透明的 `RawWindowHandle`（纯函数） |
| `raw_handle_from_rwh06(raw)` | winit / `raw-window-handle 0.6` 句柄 → HAL 句柄；非 Win32 或 Win32 缺 `hinstance` ⇒ `Err` |
| `UNSUPPORTED_PLATFORM_MSG` | 非 Windows 平台 `run()` 返回的那句话（文案常量） |

### 示例 `window_preview` 的环境变量

| 变量 | 作用 |
|---|---|
| `DEER_WINDOW_FRAMES` | 呈现多少帧后自动退出（示例默认 **120**，至少 1） |
| `DEER_WINDOW_HOLD` | 设为 `1`（或 `true`）时**不自动退出**，一直开着直到你手动关窗口（给人肉眼看） |
| `DEER_WINDOW_ADAPTER` | 用第几张显卡（默认 `0`；示例里打印实际用的适配器名） |
| `DEER_WINDOW_READBACK` | 设为 `0` 关掉第一帧的像素回读；自检会**显式降级**（明说「本次没有像素级证据」），而不是假装通过 |
| `DEER_VK_VALIDATION` | 设为 `1` 打开 Vulkan 校验层（诊断用，消息打到 stderr） |

示例打开的窗口是 **960×600**（标题带 `deer-gui` 前缀）。

## 4. 自检（怎么确认你真的用对了）

窗口程序「跑成功」不等于「画上了」，所以断言要落在**帧数**、**句柄**与**退出码**上：

```rust
// 在 App::init 里断言句柄真的拿到了（非 Windows 走不到这里，run() 会先返回 Err）
fn init(&mut self, info: &WindowInfo) -> Result<(), String> {
    assert!(info.raw.handle != 0, "HWND 不该是 0");
    assert!(info.raw.display != 0, "HINSTANCE 不该是 0");
    Ok(())
}
```

命令行自检（真窗口，10 帧自动退；**`exit=0` 且 stdout 摘要 `result=ok`** 才算通过）：

```powershell
$env:DEER_WINDOW_FRAMES='10'; cargo run -p deer-gui --features window --example window_preview
# 期望 stdout 里有两行：
#   [deer-window] 窗口已建：title="deer-gui — ..." extent=... handle(HWND)=0x...
#   [deer-window] 事件循环结束：frames=<≥10> extent=<w>x<h> result=ok
```

示例自己还会断言「呈现帧数 ≥ `DEER_WINDOW_FRAMES`」「耗时/帧率为正」，并在第一帧用
`read_back_last_frame()` **核对呈现帧的像素**（几何真的画上去了、清屏色是 sRGB 编码后的已知值）——
所以「跑了但一帧都没呈现」或「只有清屏色没有几何」都会直接失败，而不是静默成功。

**「HAL 路径」为什么是示例而不是 `#[test]`**：winit 要求事件循环在**主线程**，
而 `cargo test` 的 harness 在**子线程**里跑每个测试 ⇒ 真窗口那条链只能在示例里驱动。
于是覆盖被刻意拆成两半：

| 半边 | 在哪 | 守什么 |
|---|---|---|
| **映射契约**（`OutOfDate` 绝不当成功） | `deer-vk/src/hal.rs` 的纯函数单测 `present_outcome_mapping_never_treats_out_of_date_as_success` | 在 `cargo test` 里；把 `OutOfDate` 映成 `Presented` 会立刻变红 |
| **真实路径**（真窗口 + 真交换链 + 真提交/呈现 + `wait_idle`） | 示例 `hal_window_path`（本仓库门禁里跑） | `DEER_VK_WINDOW_TESTS=1` 打开；不设时示例会打印「这不是通过」 |

## 5. 常见坑

| 现象 | 原因 | 怎么改 |
|---|---|---|
| `target ... requires the features: window` | 忘了 feature：`window_preview` 是 `required-features = ["window"]` 的示例 | 命令带上 `--features window` |
| 想跑窗口却报错说没实现 | 非 Windows 上 winit **能开窗**，但「原生句柄 → HAL 句柄」这一步未实现 ⇒ `run()` 返回 `Err(UNSUPPORTED_PLATFORM_MSG)` | 换 Windows，或用离屏路径（[`rendering.md`](rendering.md)） |
| 在子线程里调 `run()` 失败 | 事件循环**必须在主线程**（winit 的硬要求） | 把 `run()` 放在 `main`；渲染/计算可以另开线程 |
| 窗口拉大后画面拉伸/报 swapchain 过期 | 尺寸变化没重建交换链 | 在 `App::resized` 里 `WindowedRenderer::resize`；`render_and_present` 返回 `OutOfDate` 时也是 resize 后重试（见 [`vulkan-swapchain.md`](vulkan-swapchain.md)） |
| 窗口一闪就没了 | `redraw` 第一帧就返回了 `Flow::Exit` | 用 `DEER_WINDOW_HOLD=1` 先看窗口，再决定退出条件 |
| 窗口里没有「界面」，只有纯色/三角形 | **这是 M2b 的真实状态**：把 `DrawList` 送上 GPU 是 M3 | 现在想看界面用离屏出图；M3 之后窗口才会有界面 |
| 帧率不受控、风扇狂转 | 事件循环是「请求重绘就画」，**没有帧率上限** | 自己数帧 / 加节流；本项目没有内置 vsync 之外的限帧 |
| 以为 `cargo test -p deer-vk` 已经验过真窗口链 | 真窗口 e2e **默认跳过**（要 `DEER_VK_WINDOW_TESTS=1`），而**跳过也算 pass** | 门禁用完整形式 `$env:DEER_VK_WINDOW_TESTS='1'; $env:DEER_VK_VALIDATION='1'; cargo test -p deer-vk`；HAL 路径另跑 `--example hal_window_path` |

## 6. 相关

- 上屏原理（surface / 交换链 / 呈现）：[`vulkan-swapchain.md`](vulkan-swapchain.md)
- HAL 路径示例（真窗口 + 真交换链 + 真呈现）：`crates/deer-gui/examples/hal_window_path.rs`
- 离屏出图（无窗口，CI 友好）：[`rendering.md`](rendering.md)
- 依赖例外登记（为什么引 winit、怎么撤回）：[`ROADMAP.md`](../../ROADMAP.md) 的 Q-1
- **做不到**（本模块的边界）：
  - **只有 Windows 的句柄映射**：winit 在别的平台也能开窗，但「原生句柄 → HAL 句柄」未实现 ⇒ `run()` 明确返回 `Err`（不静默填 0）。
  - **没有输入事件**：键盘 / 鼠标 / 焦点是 **M5**，现在窗口不派发任何事件。
  - **窗口里还没有界面**：`DrawList` 上屏属于 **M3**；现在显示的是 GPU 清屏色 + M2a 验证过的几何。
  - **不支持多窗口、全屏 / 无边框、HDR**，也**没有帧率上限**。
  - **DPI**：`Resized` 给的是物理像素，直接透传；不做额外的缩放换算。

## 7. 检查清单（发布前过一遍）

- [x] 示例能跑：`cargo run -p deer-gui --features window --example window_preview` → `exit=0`
- [x] HAL 路径示例能跑：`DEER_VK_WINDOW_TESTS=1 DEER_VK_VALIDATION=1 cargo run -q -p deer-gui --features window --example hal_window_path` → `exit=0`
- [x] 示例有自检断言（帧数达标 + 句柄非 0 + 退出干净）
- [x] `FEATURES.md` 已登记（状态 / 指南链接 / 示例命令都对）
- [x] 如果属于新手主线，`docs/TUTORIAL.md` 已更新（第 12 章）
- [x] 明确写了「做不到什么」
