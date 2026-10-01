# 功能指南：窗口（window）

> 状态 ✅（**仅 Windows**；M2b 开窗上屏 → **M3c 窗口里显示真实界面**）·
> 示例 `cargo run -p deer-gui --features window --example window_preview` ·
> 上屏 parity `DEER_VK_WINDOW_TESTS=1 cargo run -p deer-gui --features window --example window_parity` ·
> 清单条目见 [`FEATURES.md`](../../FEATURES.md)

## 1. 这是什么 / 什么时候用它

打开一个**真实的操作系统窗口**，跑**事件循环**，并把窗口的原生句柄以 HAL 的不透明形式
（`deer_gpu::RawWindowHandle`）交给渲染后端。这是 M2b 的第一步：**从「离屏出图」走到「屏幕上能看到」**。

**什么时候用它**：你要把 GPU 画出来的东西显示在一个真窗口里、或者你要给 `deer-vk`
的 surface / 交换链接一个真窗口句柄。

**什么时候不该用它**：只想出图/做断言（用 [`rendering.md`](rendering.md) 的
`render_tree_to_png`，无窗口、CI 友好）；想用鼠标键盘交互（**已支持**，见 [`input.md`](input.md) —— 但注意真窗口 e2e 默认跳过）；
需要无人值守的 CI（真窗口链只能在有桌面的机器上跑，且默认跳过 —— 见第 5 节的门槛）。

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
交换链      : 960×600 / format 0x0000002c / present mode 2 / 3 张图

呈现帧数    : 30
交换链过期  : 0 次（过期必须重建后重试，不能当成功）
像素回读    : 左上角 [42, 47, 63, 255]
              非清屏色像素 93900 / 576000（16.30%）；界面底色 [8, 9, 12, 255]
自检通过 ✅（帧数达标、**界面树已上屏**：非清屏色 93900 个像素是形状与字形）
[deer-window] 事件循环结束：frames=30 extent=960x600 result=ok
```

（数字随机器/主题而变，**以你自己的输出为准**。`format 0x0000002c = B8G8R8A8_UNORM` = **线性**，
所以回读到的字节**就是**写进去的字节。）

> **像素回读**：示例第一帧会用 `WindowedRenderer::read_back_last_frame()` 取样一次
> （会**强制一次 GPU→CPU 同步**，所以别每帧调），用来证明「界面真的上了屏」而不是只有清屏色。
> M3c 起交换链是**线性** `*_UNORM` ⇒ 回读**不做** sRGB 编码/解码，字节直通；
> 只有当驱动把这个 surface 的线性格式全排除、退回到 sRGB 格式时，才会看到
> 「回读值 ≠ 写入值」（那时的算法见 [`vulkan-swapchain.md`](vulkan-swapchain.md) 第 5 节）。

（窗口尺寸/适配器帧率随机器而变，**以你自己的输出为准**。）

还有一个**专测 HAL 路径**的示例（同一棵树走 `VkBackend::open(0)` → `Device::create_swapchain` →
`begin_frame` / `record` / `submit_and_present` → `wait_idle`，并断言 `record` 边界：
空列表 ✅ / 含形状的列表 ✅ / **含文本却缺 `TextEngine` ⇒ `Unsupported`**）：

```powershell
$env:DEER_VK_WINDOW_TESTS='1'; $env:DEER_VK_VALIDATION='1'; cargo run -q -p deer-gui --features window --example hal_window_path
# 本机实测：exit=0、640×480 / Rgba8Unorm（M3c 起交换链优先线性格式）、边界检查通过、校验层零消息
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
`read_back_last_frame()` **核对呈现帧的像素**（界面真的画上去了、清屏色就是线性 UNORM 的字节值）——
所以「跑了但一帧都没呈现」或「只有清屏色没有界面」都会直接失败，而不是静默成功。

**「HAL 路径」为什么是示例而不是 `#[test]`**：winit 要求事件循环在**主线程**，
而 `cargo test` 的 harness 在**子线程**里跑每个测试 ⇒ 真窗口那条链只能在示例里驱动。
于是覆盖被刻意拆成两半：

| 半边 | 在哪 | 守什么 |
|---|---|---|
| **映射契约**（`OutOfDate` 绝不当成功） | `deer-vk/src/hal.rs` 的纯函数单测 `present_outcome_mapping_never_treats_out_of_date_as_success` | 在 `cargo test` 里；把 `OutOfDate` 映成 `Presented` 会立刻变红 |
| **真实路径**（真窗口 + 真交换链 + 真提交/呈现 + `wait_idle`） | 示例 `hal_window_path`（本仓库门禁里跑） | `DEER_VK_WINDOW_TESTS=1` 打开；不设时示例会打印「这不是通过」 |

## 5. 窗口里显示真实界面（M3c 上屏路径）

M3c 之后，窗口里显示的是**真的界面树**（形状 + 真实字形文本），而不是 M2a 那个硬编码三角形。
它复用了离屏路径的**同一批**顶点流构造与**同一套**管线状态（共用层），两条路径只允许在
**颜色格式**与 **viewport 策略**上不同。

```powershell
# ① 真窗口里看到界面树（30 帧后退出；DEER_WINDOW_HOLD=1 可留窗观察）
DEER_VK_FRAMES=30 cargo run -q -p deer-gui --features window --example window_preview

# ② 上屏 parity：把**窗口里真实的像素**读回来与 CPU 后端逐像素对照（需要真窗口，必须到第 3 节的门槛）
DEER_VK_WINDOW_TESTS=1 cargo run -q -p deer-gui --features window --example window_parity
```

| 语料 | 判据 |
|---|---|
| **不透明**界面树（形状 + 真实字形） | **逐字节相同（最大通道差 0）** |
| **半透明**（`α=0.5` 的矩形 / 文字 / 圆角，叠在不透明底上） | **≤ 1 LSB**（CPU `round()` vs GPU UNORM 定点混合） |

两者都要带 `--features window`；`window_parity` 不设 `DEER_VK_WINDOW_TESTS=1` 时会打印
「**这不是通过，是被跳过**」并 `exit=0`（跳过不算证据）。

### 5.1 交换链是**线性** `*_UNORM`（不是 sRGB）

`swapchain::pick_config` 现在按 **线性优先** 选格式：
`B8G8R8A8_UNORM` → `R8G8B8A8_UNORM` → `B8G8R8A8_SRGB` → `R8G8B8A8_SRGB`
（格式列表只有 `VK_FORMAT_UNDEFINED` 时按规范「任意格式」处理 ⇒ 取线性 BGRA）。

**为什么不能用 sRGB 附件**：sRGB 附件不只在写入时编码，**混合本身也发生在线性空间**；
而 CPU 参考实现（`null.rs::blend_cov`）是在**字节空间**混合的。实测同一半透明像素
（`src=0xC0, a=0.5, dst=0`）：字节空间 `96` vs sRGB 线性 `140`，**差 44**——
「窗口像素 == CPU 像素」这条判据在 sRGB 附件上**根本不成立**。改线性后，同一个半透明语料
从「差几十」变成 **≤1 LSB**（只剩舍入），而**不透明仍然要求 0**（判据没有放宽）。
`window_parity` 会**先把实际拿到的格式打印出来并断言它是线性的**。

> ⚠️ **格式传错不会被驱动拦住**：管线 `colorAttachment` 格式与渲染通道附件格式**不一致**时，本机驱动
> `vkCreateGraphicsPipelines` **返回成功、静默接受**（可重跑用例：
> `cargo test -p deer-vk --test pipeline_smoke format_mismatch_is_accepted_by_this_driver_and_must_be_guarded_by_the_caller`）。
> 所以窗口路径必须把 `swapchain.format()` 传进管线，**并且**靠**上屏像素对照**兜底 ——
> 这正是本节 `window_parity` 存在的理由（同族清单位于 [`vulkan.md`](vulkan.md) 第 6 节）。

### 5.2 viewport 策略：窗口用**动态**（默认）；「动态画不出像素」这条旧理由**已被实测推翻**

窗口路径**每帧显式设置** `vkCmdSetViewport`/`vkCmdSetScissor`（动态，产品行为）。
早期文档写过「动态 viewport/scissor 在本机 Intel 核显上**画不出任何像素**」——**这条旧断言在
本机 / 离屏 / 三角形场景下被三组对照推翻**（C1，2026-09-28）：

| 组 | 形态 | 结果 |
|---|---|---|
| **①** | 动态 + **每帧真的调** `vkCmdSetViewport`/`Scissor` | 退出码 0、**1326/4096 像素（32.4%，几何自洽）**、与静态**逐字节相同**、校验消息 **0** ⇒ **能画出像素** |
| **②** | 动态 + **从不设置**（M2a 形态） | **崩 `0xC0000005`** |
| **③** | 静态（产品现状基线） | 退出码 0、**1326/4096** |

可重跑探针：`crates/deer-vk/examples/viewport_dynamic_probe.rs`（`cargo run -p deer-vk`；
加 `--group=0|1|2` 单跑某组，`DEER_VK_VALIDATION=1` 得校验层对照）。

**⚠️ 三条边界，一条都不能省**：

1. **这是本机实测**（这台 Intel 集显 + 离屏 + 三角形场景）⇒ **不等于**「动态 viewport 现已支持」这种
   跨设备结论；换机器/驱动要重跑探针。
2. **症状不吻合**：M2a 记的是「**零像素且不崩溃**」，而 ② 是「**崩溃**」⇒ **不能断定当年成因与本次同源**；
   **「当年为什么零像素仍未解释」**。所以**不写**「已证实是误诊」。
3. **产品行为不变**：**离屏路径继续用静态** —— C1 只推翻了旧结论的**理由**，并没有测「动态 + 尺寸变化 +
   多帧复用命令缓冲」这类产品级场景，而静态已被既有判据覆盖 ⇒ **是否改用动态是独立的产品决策**。

**②的致命性是独立复现的硬要求**：声明动态状态却从不设置 ⇒ 离屏 `0xC0000005`、窗口 `0xC000041D`
⇒ **声明了动态状态，就必须在录制时真的设置它**；反过来，对**静态**管线发这两个命令会报校验错
（声明与调用必须一致）。

**诊断开关**：`DEER_VK_WINDOW_VIEWPORT=static|dynamic`（默认 `dynamic`；**只作实测/诊断**，
产品行为以默认值为准）。静态是「写进管线」的策略，所以尺寸一变就必须重建管线 ——
实测 4 次 resize：动态**重建 0 次**、静态**重建 4 次**，而**存活界面资源恒为 1**（无泄漏）。

## 6. 重绘策略：默认**省电**（M5b）

M5b 把事件循环从「一直要下一帧」改成**事件驱动**：

| 项 | 事实 |
|---|---|
| 控制流 | `ControlFlow::Poll` → **`Wait`**（**纯阻塞**：没有事件就睡）—— **不用 `WaitUntil` 兜底**，所以没有「超时轮询」 |
| `App::wants_redraw(&self) -> bool` | **默认 `false`**；语义 = 「**本次输入是否改变了状态**」；只有它为真才请求重绘 |
| `App::redraw_policy(&self) -> RedrawPolicy` | **默认 `OnDemand`**（省电：空闲时零重绘）；`Continuous` = 每画完一帧再请求下一帧 |
| **一律**置位的事件 | **系统事件**（`Resized` / `Focused` / `Occluded(false)` 窗口重新暴露）与**建窗引导帧** —— 不经过 `wants_redraw` |
| `Flow` 枚举 | **未动**（既有 5 个实现零改动） |

**可关闭省电**：环境变量 **`DEER_WINDOW_REDRAW=continuous`** ⇒ **无条件**强制 `Continuous`；
`on-demand` / `demand` / 未设 / 无法识别 ⇒ 用 `App` 自己声明的策略。
取值**先 `trim()` 再比、大小写不敏感**（与门槛自证同源的真事故：`cmd` 的 `set X=continuous && …` 会让值是 `"continuous "` 带尾空格）。

**自证标记（验收要 grep 它，而不是只看退出码）**：

```text
[deer-window] 重绘策略：请求=… 实际=…
[deer-window] 重绘账本：requests=… skipped=… frames=…
```

**省电的判据 = 计数门槛，不是 CPU 比例**（重要，别再拿 CPU 当验收）：

- **判据（确定性的）**：`idle_probe` 的「**`wants_redraw` 答真 0 次** 且 **画帧 ≤ 1 + 系统帧**」，以及
  `redraw_policy` 的**两条单测**（策略解析的真值表：`continuous` ⇒ 强制 `Continuous`；`on-demand`/未设/无法识别 ⇒ 用 App 策略）；
- **CPU 比例只是本机实测**，**永久不作为断言** —— 理由：CPU 时间依赖**机器 / 驱动 / 负载**，
  写成阈值**必然 flaky**；计数判据是**确定性**的。谁要把 CPU 变断言，先解决 flakiness。

**⚠️ 这个判据有前置：本次必须真的派发过输入** —— 照下面的命令跑，别只把二进制跑起来：

- 一条输入都没有 ⇒ `wants_redraw` **一次都没被问过** ⇒ 「答真 0 次」「画帧 ≤ 1 + 系统帧」**必然成立**：
  判据在这里**不是变弱，而是空转**（「无条件请求重绘」这种变异都能顶着它绿）。
  所以探针把前置**显式断言并放在最前面**：**输入 = 0 ⇒ `exit=1`**，stderr 明确写「**前置不成立**」
  —— 与「**判据失败**」**分开报**，两种红必须能分开看。
- **行为变化**：默认（`OnDemand`）档**不注入输入现在 `exit=1`** —— 这正是修法的目的
  （改前它会打一行「自检通过（省电）：0 条输入」的**假绿**）。
- **不受影响的那一档**：`DEER_WINDOW_REDRAW=continuous` **不下**「输入有没有改状态」的结论
  （它验的是「策略被环境变量覆盖」，看 `requests`/`frames` 与门槛自证日志）⇒ **0 输入仍 `exit=0`**。
- **可复现的注入命令**（等价做法：`PostMessage(WM_MOUSEMOVE)` 注入悬停；**不动用户鼠标、不抢焦点**）：

```powershell
cargo build -p deer-window --examples          # 与验收同一组合（deer-window 无 feature）
$env:DEER_IDLE_SECONDS='3'
$p = Start-Process -PassThru .\target\debug\examples\idle_probe.exe
$h = [IntPtr]::Zero
for ($i=0; $i -lt 40 -and $h -eq [IntPtr]::Zero; $i++) { Start-Sleep -Milliseconds 100; $h = (Get-Process -Id $p.Id).MainWindowHandle }
Add-Type -Namespace P -Name W -MemberDefinition '[DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint m, IntPtr w, IntPtr l);'
1..40 | ForEach-Object { [void][P.W]::PostMessage($h, 0x0200, [IntPtr]::Zero, [IntPtr](30 -shl 16)); Start-Sleep -Milliseconds 15 }
$p.WaitForExit(); "exit=$($p.ExitCode)"        # 期望 0
```

> **本机实测**：注入 40 条悬停 ⇒ **`exit=0`**；不注入 ⇒ **`exit=1`**（stderr 首行即「自检失败（**前置不成立**）」）；
> `DEER_WINDOW_REDRAW=continuous` + 不注入 ⇒ **`exit=0`**。
> 更完整的跑法（按 **PID** 枚举窗口、逐档抓退出码的 runner）另见 M5b 跟进留下的验证脚本 ——
> 那是**便利**、不是唯一出处：上面这段**自足**。

**本机实测的省电效果**（同一 5 秒窗口 + 40 条悬停输入；**实测值，不是断言；量级事实，具体数随机器/负载浮动**）：

| 档 | 画帧 | 进程 CPU（每 5 秒窗口） |
|---|---|---|
| 默认 `OnDemand` | **1** | **0.05 s** |
| `DEER_WINDOW_REDRAW=continuous` | **418007** | **4.59 s**（≈ **1.1%** 的墙钟时间） |

> 另一次独立实测给出同量级（约 **27 万**帧 / **4.08 s**）—— 说明这是**量级**事实：
> 「省电档 1 帧 vs 连续档 10⁵ 量级帧」，而**不是**某个固定数字。所以文档里**只给量级与命令**，
> 验收看**计数门槛**，不看 CPU。

复现（探针源码：`crates/deer-window/examples/idle_probe.rs`；在 `deer-window` 里跑该示例，
`--example` 后写该文件名去掉 `.rs` 的部分）：

```powershell
cargo run -q -p deer-window --example <去掉 .rs 的文件名>                                   # 默认：省电
set "DEER_IDLE_DIRTY=1" && cargo run -q -p deer-window --example <去掉 .rs 的文件名>         # 每条输入都当「改了状态」
set "DEER_WINDOW_REDRAW=continuous" && cargo run -q -p deer-window --example <去掉 .rs 的文件名>  # 关掉省电
```

**谁声明什么**（各示例**显式**声明，不靠默认值隐式生效）：

| 示例 | 策略 | 理由 |
|---|---|---|
| `window_preview` / `hal_window_path` / `interactive_form` | **`Continuous`** | 演示类要连续推进。**注意（M5c 后口径变了）**：脚本重放**已经能在 `OnDemand` 下做** —— `interactive_form` 的 `DEER_FORM_ONDEMAND=1` 档就是用唤醒面自驱的重放；默认仍声明 `Continuous` 是**演示类的选择**，不再是「接口上做不到」 |
| `window_parity` | **`OnDemand`** | 它只需要 1 帧；判据与退出码**零改动** |

**`skipped_frames` 与重绘策略正交**（别写混）：它的含义是「**派发了输入但没有请求重绘**」的次数，
属**输入门禁**度量 ⇒ 在 `Continuous` 档**它照样增长**；**不要**写成「连续模式下恒 0」。

**已知边界**：

- `Occluded(false)` 已接线，但 **winit 0.30 文档写明 Windows 不支持该事件** ⇒ **Windows 上未实测**；
- **`DEER_IDLE_REQUIRE` 已删除**：它的旧语义是「**只有**设了它才把 0 输入判失败，**默认不算失败**」——
  一个**默认没牙**的旋钮正是上面那个前置漏洞的根源。删掉只会**更严**（0 输入一律 `exit=1`），
  不会让任何东西**静默变绿**；如果你在脚本里还设着它，那是**无效**的，删掉即可。
- **真窗口的 `DEER_IDLE_DIRTY=1`（变化档）目前只报数、不下结论** ⇒ 「**从不请求重绘**」这类错误
  在**真窗口路径**上**只能靠单测**兜住（该档的「0 输入」也已被上面的前置拦掉，所以它至少不会假绿）。

**唤醒面（M5c）：`OnDemand` 下 App **也能**自己唤醒事件循环** —— 上面那条「`OnDemand` 下没有唤醒手段」的旧边界已由 M5c 取代：

| 手段 | 语义 | 到点画不画 |
|---|---|---|
| `App::wake_handle(Waker)` | 建窗后**调一次**，把可克隆的 `Waker` 交给 App（**默认实现什么都不做** ⇒ M5c 之前的 `App` 实现一行都不用改） | — |
| `Waker::wake()` | 「**看一眼**」（提示性：也许别处改了状态） | 与输入同一把尺：问 `App::wants_redraw`，**答真才画** |
| `Waker::wake_after(Duration)` | 「**d 之后叫醒我**」（预约：定时动画 / 脚本重放） | 到点**一律画一帧** |
| `App::next_deadline() -> Option<Instant>` | 同上，但**拉**式（App 声明「我希望被唤醒的最近时刻」） | 到点**一律画一帧** |

**这「不是」先前被否掉的 `WaitUntil` 兜底**（最容易读成自相矛盾的一条）：被否掉的是**本层凭空造超时** ——
没人要求、到点也没事干，纯空转。现在本层**只在 App 显式声明了 deadline 时**才用 `WaitUntil`，
而**声明这个动作是 App 主动做的**：**它不声明，本层就在 `Wait` 上睡死 —— 开机不会自己醒来，
只有 App 说「那个时刻叫我」才醒。**

**`iters` 的最终口径（M5c 复审 I1 + fix 轮）：它是观测值，但「空闲档的上界」是**判据**：

- 账本行：`[deer-window] 唤醒账本：wake=… wake_after=… fired=… requested=… skipped=… iters=…`。
- **本层刻意不设全局阈值**：`DEER_WINDOW_REDRAW=continuous` 档下 `iters` 与帧数同阶（实测 229365）
  是**合法**的 ⇒ 把阈值做进 `run()` 的收尾路径会假红。上界只能由**声明了空闲语义的那一档**自己下。
- **空闲档的上界门槛（2026-09-29 fix 轮补上）**：`App::on_wake_stats(&WakeStats)` 在收尾时把账本
  交给 App（默认实现什么都不做 ⇒ 老代码一行不用改）⇒ `examples/wake_probe.rs` 的
  `DEER_WAKE_TICKS=0` 档断言 **`iters ∈ [1, 32]`**（本机实测基线 **5–7**，含启动期 3 次 `resized`）。
- **缺口 → 已闭环的实证**：复审当年只把 `Wait` 换成 `Poll` ⇒ `iters` **11,695,034**、CPU 4.30 s / 4.5 s、
  **退出码 0、每条判据 ✅**。现在同样的变异 ⇒ `iters` **11,667,271**，**只有这一条判据 ❌**、`exit=1`；
  另有单测 `crates/deer-window/src/lib.rs::wake_policy_control_flow_mapping` 直接钉住
  `WakePlan::Wait ⇒ ControlFlow::Wait`（翻译只有一处，`plan_to_control_flow`）。
- **已知缺口（诚实，别再当它万能）**：周期 **> ~140 ms** 的慢超时在这条上界里看不出来
  （32 轮 / 这一档约 4.5 s）；窗口收到**外来输入**时会被 `wake_probe` 的「前置：外来输入」先判成
  「前置不成立」（措辞与「判据失败」分开），**不会**伪装成通过。
- **别误读**：高 `iters` 不等于 bug（`continuous` 档）；是否空转要看**空闲档的上界**，以及那组
  **计数门槛**（「`wants_redraw` **答真 0 次** + 画帧 ≤ **1 + 系统帧**」）。

⚠️ **两条使用须知**（「App 自己的要求」的直接后果，不是本层的 bug）：

- **`next_deadline()` 必须返回固定时刻并自己往前走**：别每次都返回 `Instant::now() + 50ms` ——
  那样它**永远不到点**，事件循环每 50ms 醒一次却**一帧都不画**（空转）。要「每 50ms 来一次」就用
  `Waker::wake_after`：在 `App::redraw` 里排下一次（推式）。**本层不做防护** —— 那等于凭空造超时。
- **已过期**的 deadline 视为「立刻到点」⇒ 画一帧；App 不把它清掉 / 往前推，就等于自己要求连续重绘
  （账本上 `fired` 会跟着 `iters` 一起涨）。

⚠️ **`Send` 是契约，`Sync` 是巧合（别在别处依赖 `Sync`）**：`EventLoopProxy` 的 **`Send` 是 winit 的显式承诺**
（编译期断言 + `waker_is_send` 单测钉着它）；但 **`Sync` 在 Windows 后端没有 impl** —— 它今天成立纯属两个字段的
**自动 trait 巧合**（`HWND` = `isize`、`mpsc::Sender` 恰好 `Sync`）。
**而且这个「哪天」已经来了**（复审核实、可直接查证）：**`windows-sys` 0.59 / 0.60 / 0.61 已把 `HWND` 改成 `*mut c_void`**
（本机缓存源码对照：`0.52.0` = `pub type HWND = isize;`、`0.61.2` = `pub type HWND = *mut core::ffi::c_void;`）。
我们**今天还是 `isize`**，只是因为 `winit 0.30` 仍钉在 `windows-sys 0.52`（`Cargo.lock` 里同时存在 0.52 / 0.59 / 0.61，但
`winit` 用的是 0.52）⇒ **一旦 winit 升到用新 `windows-sys`：`Send` 会编译期炸（好事），`Sync` 会静默消失（坏事）。**

详见 `crates/deer-window/src/lib.rs` 的「唤醒面」一节与 `crates/deer-window/examples/wake_probe.rs`
（配套单测 `crates/deer-window/tests/wake_policy.rs`）。**仍未做**：自定义用户事件类型（对外**只有 `Waker`** 这一个面）、
跨进程唤醒、多窗口唤醒。
  这是**已知限制**，不是判据。

## 7. 常见坑

| 现象 | 原因 | 怎么改 |
|---|---|---|
| `target ... requires the features: window` | 忘了 feature：`window_preview` 是 `required-features = ["window"]` 的示例 | 命令带上 `--features window` |
| 想跑窗口却报错说没实现 | 非 Windows 上 winit **能开窗**，但「原生句柄 → HAL 句柄」这一步未实现 ⇒ `run()` 返回 `Err(UNSUPPORTED_PLATFORM_MSG)` | 换 Windows，或用离屏路径（[`rendering.md`](rendering.md)） |
| 在子线程里调 `run()` 失败 | 事件循环**必须在主线程**（winit 的硬要求） | 把 `run()` 放在 `main`；渲染/计算可以另开线程 |
| 窗口拉大后画面拉伸/报 swapchain 过期 | 尺寸变化没重建交换链 | 在 `App::resized` 里 `WindowedRenderer::resize`；`render_and_present` 返回 `OutOfDate` 时也是 resize 后重试（见 [`vulkan-swapchain.md`](vulkan-swapchain.md)） |
| 窗口一闪就没了 | `redraw` 第一帧就返回了 `Flow::Exit` | 用 `DEER_WINDOW_HOLD=1` 先看窗口，再决定退出条件 |
| 窗口里没有「界面」，只有纯色/三角形 | **旧状态**（M2a/M2b 时代）。M3c 已把界面呈到窗口；若仍只看到纯色，多半是没跑对示例或用了旧二进制 | 跑 `DEER_VK_FRAMES=30 cargo run -q -p deer-gui --features window --example window_preview`；要判据就跑 `window_parity`（见第 5 节） |
| 上屏像素与 CPU 对不上、半透明差几十个字节 | 交换链是 **sRGB** 附件：混合发生在线性空间，而 CPU 按字节混合（`blend_cov`） | 让 `pick_config` 选到**线性 `*_UNORM`**（第 5.1 节）；`window_parity` 会断言拿到的格式是线性的 |
| 以为「动态 viewport 在本机 Intel 上不可用」 | 旧结论，**已被 C1 实测推翻**：动态 + 每帧真的调 ⇒ 与静态逐字节相同、能画出像素；而「声明动态却从不设置」会**崩** | 用动态就**每帧真的设置**；边界（本机实测、症状不吻合）见第 5.2 节，别外推成跨设备结论 |
| 反复 resize 后内存/资源涨 | 静态 viewport 策略下尺寸变化要重建管线与交换链相关资源 | 用默认的**动态**策略（重建 0 次）；静态只作诊断，重建期间要保证旧资源被 `Drop`（存活计数恒 1） |
| 帧率不受控、风扇狂转 | 事件循环是「请求重绘就画」，**没有帧率上限** | 自己数帧 / 加节流；本项目没有内置 vsync 之外的限帧 |
| 以为 `cargo test -p deer-vk` 已经验过真窗口链 | 真窗口 e2e **默认跳过**（要 `DEER_VK_WINDOW_TESTS=1`），而**跳过也算 pass** | 门禁用完整形式 `$env:DEER_VK_WINDOW_TESTS='1'; $env:DEER_VK_VALIDATION='1'; cargo test -p deer-vk`；HAL 路径另跑 `--example hal_window_path` |

## 8. 相关

- 上屏原理（surface / 交换链 / 呈现）：[`vulkan-swapchain.md`](vulkan-swapchain.md)
- HAL 路径示例（真窗口 + 真交换链 + 真呈现）：`crates/deer-gui/examples/hal_window_path.rs`
- 离屏出图（无窗口，CI 友好）：[`rendering.md`](rendering.md)
- 依赖例外登记（为什么引 winit、怎么撤回）：[`ROADMAP.md`](../../ROADMAP.md) 的 Q-1
- **做不到**（本模块的边界）：
  - **只有 Windows 的句柄映射**：winit 在别的平台也能开窗，但「原生句柄 → HAL 句柄」未实现 ⇒ `run()` 明确返回 `Err`（不静默填 0）。
  - **输入事件**：**已支持**（M5-1..M5-4：`InputEvent` + winit 映射 + `App::input` → 命中/状态机 → 重绘；
    见 [`input.md`](input.md)）。**M5b 起重绘也改成事件驱动**（默认省电，见第 6 节）；
    **仍未做**：方向键**上下**导航 / 右键中键 / IME 预编辑（**左右**方向键已做：输入框光标，T3.5）。（**滚轮已消费**：`MouseWheel` → `InputEvent::Wheel`
    → 滚动偏移 → 几何/绘制/命中，见 [`scroll-and-multiline.md`](scroll-and-multiline.md)；
    仍未做的是**滚动条**、惯性滚动与按键滚动。）
  - **窗口里显示的界面**：**已支持**（M3c，见第 5 节）——窗口里是真实的形状 + 文本，且上屏像素与 CPU 逐像素对照过
    （不透明 0、半透明 ≤1 LSB）。窗口里画的是**当前帧的界面快照**：**没有动画/时间系统**（按时间的动画需自行声明
    `Continuous`，见第 6 节）；**滚动容器已有**，但滚动偏移要由应用自己喂进布局（见
    [`scroll-and-multiline.md`](scroll-and-multiline.md)）。
  - **不支持多窗口、全屏 / 无边框、HDR**，也**没有帧率上限**。
  - **DPI**：`Resized` 给的是物理像素，直接透传；不做额外的缩放换算。
  - **✅ 窗口侧 host→vertex 屏障已有断言（原覆盖缺口，已补上）**：
    **原现象**：窗口路径的屏障计数已就位（与 `cmd_pipeline_barrier` 同一处 ⇒ 删发射必然也删掉计数），
    但**缺一条能咬住它的断言** ⇒ 「**窗口路径从不发屏障**」这类变异**曾经**在门禁下会全绿。
    **现状**：`windowed_chain_end_to_end` 里已补上三条断言 —— **首帧上传顶点 ⇒ +1**、
    **同语料第二帧 ⇒ 不增**（B3 跳过重传）、**空帧 ⇒ 不增**，并带**前置断言**
    （首帧 `buffer_uploads` 必须增长，否则「没增长」可能是因为压根没上传 ⇒ 判据被架空）。
    **⚠️ 挂在哪条路径上很关键**：上述断言必须走 `draw_and_present`（→ `record_ui`）。
    `render_and_present` 走的是 **M2b 三角形路径**（`record`），**不碰 UI 顶点缓冲**，
    在那条链上读 `ui_host_to_vertex_barrier_count()` 恒为 0 —— 那是条永远绿也永远没用的判据。
    **双向变异（已实测、确定性变红）**：① 只删 `ui_host_to_vertex_barriers += 1;`（保留发射）⇒ 红；
    ② 删掉整个 `if self.ui_barrier { … }` 块（连发射带计数）⇒ 红。
    **索引 / 间接两条也已补齐**：`ui_index_barrier_count()` / `ui_indirect_barrier_count()`
    与顶点那条**同一套三条判据**（首帧各 +1 / 同语料不增 / 空帧不增），实测
    `(0,0,0) → (1,1,1) → (1,1,1) → (1,1,1)`；**双向变异均已实测变红**：
    只删 index 自增 ⇒ `(1,0,1)` 红；只删 indirect 自增 ⇒ `(1,1,0)` 红；删掉 index 整个发射块 ⇒ 红。
    **边界**：本判据证明「这段被执行了、且时机符合 B3 语义」；**屏障参数的确切位**由
    `ui_barrier_constants_pin_the_exact_bits` 钉，「驱动真的按语义用了它」属真机逐像素对照的范畴。

## 9. 检查清单（发布前过一遍）

- [x] 示例能跑：`cargo run -p deer-gui --features window --example window_preview` → `exit=0`（窗口里是真实界面树）
- [x] 上屏 parity 示例能跑：`DEER_VK_WINDOW_TESTS=1 cargo run -q -p deer-gui --features window --example window_parity` → `exit=0`（不透明 0 / 半透明 ≤1 LSB）
- [x] HAL 路径示例能跑：`DEER_VK_WINDOW_TESTS=1 DEER_VK_VALIDATION=1 cargo run -q -p deer-gui --features window --example hal_window_path` → `exit=0`
- [x] 示例有自检断言（帧数达标 + 句柄非 0 + 退出干净）
- [x] `FEATURES.md` 已登记（状态 / 指南链接 / 示例命令都对）
- [x] 如果属于新手主线，`docs/TUTORIAL.md` 已更新（第 12 章）
- [x] 明确写了「做不到什么」
- [x] 重绘策略已写明（默认省电 / 如何关掉 / 两个自证标记要 grep），并登记 `Occluded` 的 Windows 边界、`DEER_IDLE_REQUIRE` 的删除、M5c 唤醒面（含「`iters` 是观测值 + **空闲档 `[1,32]` 上界**（`App::on_wake_stats`）、`next_deadline` 陷阱、`Send` vs `Sync` 的上游现状）
