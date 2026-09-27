# 03 · 应用层与工程纪律 —— 事实地图

> **测绘对象**：`crates/deer-window`、`crates/deer-gui`、示例清单、测试与门禁、工程纪律、文档地图、拓展指引。
> **测绘方式**：只读代码 + 实跑命令。每条结论带 `path:line`；读不到的写「未确认」，不臆测。
> **测绘时间基线**：`git log -1` = `9df8ade merge(m3a): DrawList 的非文本命令上 GPU，并与 CPU 后端逐像素对照`。
> **本文件的所有实测数字**都是在本机（Windows / MSVC / Rust 2024、`Z:\deer-gui`）当场跑出来的，命令与输出都写在下面。

---

## 0. 一句话结论

- `deer-window` 只做两件事：**建窗口 + 跑事件循环**，并把原生句柄翻译成 HAL 的**不透明** `RawWindowHandle`（`crates/deer-window/src/lib.rs:1-42`）。
- `deer-gui` 是**门面 crate**：一条 `use deer_gui::prelude::*` 拿到布局/GPU/Vulkan，另有 5 个 `render_tree_to_*` 便利入口（`crates/deer-gui/src/lib.rs:1-37`、`77-193`）。
- 工程纪律的可执行形式是**四道门禁**（workspace 测试 / clippy 带 `window` feature / `docs_consistency` / 示例全跑），其中 `docs_consistency` 把「文档与示例必须齐备」变成了一组可执行的契约测试（`crates/deer-gui/tests/docs_consistency.rs:1-11`；条数以 `cargo test -p deer-gui --test docs_consistency -- --list` 的输出为准）。

---

## 1. `crates/deer-window`

### 1.1 文件清单

| 文件 | 行数 | 作用 |
|---|---|---|
| `crates/deer-window/Cargo.toml` | 17 | 声明 `winit = "0.30"`，**本 workspace 唯一第三方依赖**（`Cargo.toml:13-17`） |
| `crates/deer-window/src/lib.rs` | 400 | 全部实现：类型 + 事件循环接线 + 句柄翻译 |
| `crates/deer-window/tests/window_logic.rs` | 161 | 纯逻辑单测，10 条 `#[test]`，**不建窗口** |
| `crates/deer-window/examples/window_smoke.rs` | 138 | 真开窗冒烟，**故意不是 `#[test]`** |

### 1.2 类型与函数完整签名（带行号）

```rust
// crates/deer-window/src/lib.rs:57-58
pub const UNSUPPORTED_PLATFORM_MSG: &str =
    "deer-window 目前只实现了 Windows 窗口（winit 后端已就绪，但 RawWindowHandle 的填法未实现）";

// :61-66
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowConfig {
    pub title: String,
    pub width: u32,
    pub height: u32,
}

// :70-72
pub fn new(title: impl Into<String>, width: u32, height: u32) -> WindowConfig
// :77-83
pub fn display_title(&self) -> String          // 空标题 ⇒ "deer-gui"；否则 "deer-gui — {title}"
// :86-91  impl Default for WindowConfig        // "" / 800 / 600

// :94-100
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowInfo {
    pub raw: deer_gpu::RawWindowHandle,        // platform=Windows, handle=HWND, display=HINSTANCE
    pub extent: Extent,                        // 当前**物理**像素尺寸
}

// :103-109
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flow { Continue, Exit }

// :114-137
pub trait App {
    fn init(&mut self, info: &WindowInfo) -> Result<(), String>;              // :118 建好后调一次
    fn resized(&mut self, width: u32, height: u32) -> Result<(), String> { .. } // :123-128 默认空实现
    fn redraw(&mut self) -> Result<Flow, String>;                             // :131
    fn close_requested(&mut self) -> Flow { Flow::Exit }                       // :134-136 默认允许关闭
}

// :143-147  帧计数 + 退出标记（run() 账本的纯逻辑核心）
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct FrameCounter { frames: u64, exit_requested: bool }
// :150  pub fn new() -> FrameCounter
// :157  pub fn on_redraw(&mut self, result: Result<Flow, String>) -> Result<(), String>
// :171  pub fn frames(&self) -> u64
// :176  pub fn exit_requested(&self) -> bool

// :185-191
pub fn raw_handle_from_win32(hwnd: usize, hinstance: usize) -> deer_gpu::RawWindowHandle

// :202-204
pub fn raw_handle_from_rwh06(raw: RwhRawWindowHandle) -> Result<deer_gpu::RawWindowHandle, String>

// :221  fn window_info(window: &Window) -> Result<WindowInfo, String>   // 私有
// :242  pub fn run<A: App + 'static>(config: WindowConfig, app: A) -> Result<(), String>
// :265  struct RunHandler<A: App>          // run() 的 ApplicationHandler 实现
// :282  fn fail(&mut self, event_loop: &ActiveEventLoop, msg: String)
// :291  fn finish(self, loop_error: Option<String>) -> Result<(), String>
```

### 1.3 事件循环怎么接 winit

- `run()` 在**主线程**建 `EventLoop`，失败信息明说「run() 必须在主线程调用」（`crates/deer-window/src/lib.rs:243-244`）。
- 用 **winit 0.30 的 `ApplicationHandler` 模型**（不是 0.29 的闭包式）：`RunHandler` 实现 `ApplicationHandler`，`run()` 调 `event_loop.run_app(&mut handler)` 并把它的返回值也接住，避免错误被吞（`:256-261`）。
- 事件 → `App` 回调的映射表（`:351-390`）：
  | winit 事件 | 行号 | 动作 |
  |---|---|---|
  | `resumed` | `:308-349` | `create_window`（逻辑尺寸 `LogicalSize`）→ `window_info()` → 打印一行含 HWND/HINSTANCE 的摘要 → `app.init(&info)` → `set_control_flow(ControlFlow::Poll)` |
  | `Resized` | `:361-369` | 更新物理 `extent` → `app.resized(w, h)` |
  | `RedrawRequested` | `:370-381` | `app.redraw()` → `FrameCounter::on_redraw` → 若 `Exit` 则 `event_loop.exit()` |
  | `CloseRequested` | `:383-386` | 只有 `app.close_requested() == Flow::Exit` 才退 |
  | 其它（键盘/鼠标/IME/滚轮） | `:387-388` | **丢弃**，输入层留给 M5 |
- **连续重绘**：`ControlFlow::Poll`（`:347`）+ `about_to_wait` 里 `window.request_redraw()`（`:392-399`）形成「一直要下一帧」；文档明说节奏本层不管，真正上屏时由呈现（vsync）决定（`:41-42`）。
- **退出时的可断言输出**：无论成败都往 stdout 打一行摘要，供脚本断言（`:293-299`）：
  `[deer-window] 事件循环结束：frames=<n> extent=<w>x<h> result=ok|error`
  实测（`window_preview`，见 §3）：`[deer-window] 事件循环结束：frames=120 extent=960x600 result=ok`。
- 错误策略：任何回调返回 `Err` 都 `eprintln!` + 记住 + `event_loop.exit()`，**绝不吞掉**（`:282-288`）；最终 `Err` 优先给回调错误，其次给事件循环自身的错误（`:291-304`）。

### 1.4 `raw_handle_from_win32` / `raw_handle_from_rwh06` 的作用

- **为什么需要**：渲染后端 `deer-vk` **不依赖 winit**，它只认 `deer_gpu::RawWindowHandle` 这个**不透明**三字段句柄（`platform` / `handle` / `display`）。所以窗口层必须把 winit 的 rwh-0.6 句柄翻译过去。这条边界写在 crate 文档里：`crates/deer-window/src/lib.rs:3-5`，以及 `crates/deer-gui/src/lib.rs:46-52`。
- `raw_handle_from_win32(hwnd, hinstance)`（`:185-191`）：**纯打包**，不做平台判定 —— `platform = Platform::Windows`、`handle = hwnd`、`display = hinstance`，两个值都按 `usize` 原样存。测试钉住「0 也照打包，判定在上层」（`crates/deer-window/tests/window_logic.rs:71-81`）。
- `raw_handle_from_rwh06(raw)`（`:202-218`）：**带平台判定**的翻译器：
  - `Win32` ⇒ 成功；缺 `hinstance` ⇒ `Err`（`display` 没法编，错误文案点明缺的是 hinstance，`:206-213`）；
  - 其它平台（Xlib/Wayland/AppKit…）⇒ `Err(UNSUPPORTED_PLATFORM_MSG)`，**不静默填 0**（`:214-217`）。
  - 因为是纯函数，**在 Windows 上也能单测非 Windows 分支**（直接喂 `Xlib`/`Wayland` 句柄）：`crates/deer-window/tests/window_logic.rs:101-116`。
- 接线点：`window_info()` 组合两者（`window.window_handle()` → `raw_handle_from_rwh06` → 加上 `inner_size()`），产出给 HAL 的 `WindowInfo`（`:221-228`）。

### 1.5 「为什么 `#[test]` 不能驱动 winit」的注释证据

四处独立证据，措辞一致：

| 位置 | 原文要点 |
|---|---|
| `crates/deer-window/src/lib.rs:230` | 「建窗口并跑事件循环（**必须在主线程**调用；winit 的要求）」 |
| `crates/deer-window/src/lib.rs:113` | 「所有回调都在**主线程**（事件循环线程）上被调用」 |
| `crates/deer-window/tests/window_logic.rs:1-3` | 「deer-window 的**纯逻辑**单测：不建窗口、不跑事件循环。真开窗的验证在 `examples/window_smoke.rs` 里（CI 无桌面，`#[test]` 里真开窗会假红）」 |
| `crates/deer-window/examples/window_smoke.rs:1` | 「**真开窗口**的冒烟验证（所以它不是 `#[test]`：CI 无桌面会假红）」 |
| `crates/deer-gui/examples/hal_window_path.rs:8-12` | 「**winit 要求事件循环在主线程**，而 `cargo test` 的 harness 在子线程里跑每个测试 ⇒ 只能在**示例**里驱动它」 |
| `docs/features/window.md:207-208` | 同上，并给出「覆盖被拆成两半」的表格（映射契约在 `cargo test`；真实路径在示例） |

> **结论**：真窗口路径**不能**写成 `#[test]`；替代做法是 ① 纯逻辑部分进单测（`tests/window_logic.rs` 10 条）、② 真路径进**示例**并由门禁跑（§4 第 4 道门禁）。

---

## 2. `crates/deer-gui`

### 2.1 `crates/deer-gui/src/lib.rs` 的渲染入口与调用链

| 函数（行号） | 签名要点 | 调用链 |
|---|---|---|
| `render_tree_to_rgba`（`:77-100`） | `(&Node, width: u32, height: u32, theme: Theme) -> GpuResult<(u32, u32, Vec<u8>)>` | `ApproxMeasure` → `layout::layout::layout(tree, Rect::new(0,0,w,h), TextStyle{theme.font_size, theme.line_height}, &measure)`（`:84-92`）→ `gpu::build_draw_list(tree, &geo, theme.clone(), &measure)`（`:93`）→ `gpu::null::CpuRenderer::new().render(Extent{w,h}, &list, theme.surface)`（`:94-98`） |
| `render_tree_to_png`（`:103-112`） | `(…) -> Result<Vec<u8>, String>` | 调 `render_tree_to_rgba`（`:109-110`）→ `gpu::png::encode_rgba(w, h, &px)`（`:111`）；错误包装成 `"渲染失败：{e}"` |
| `render_tree_to_rgba_with_font`（`:125-135`） | 多两个参数 `font_path: &Path, font_size: f32` | `gpu::TextEngine::from_font_file(font_path, font_size)`（`:133`）→ 委托 `render_tree_to_rgba_with_engine`（`:134`） |
| `render_tree_to_rgba_with_engine`（`:139-166`） | 由调用方给 `engine: gpu::TextEngine` | **钉死字号**：`theme.font_size = font_size`（`:149`）→ `layout(..., &engine.measure())`（`:155-160`）→ `build_draw_list(..., &engine.measure())`（`:161`）→ `CpuRenderer::with_text(engine)`（`:163`）→ `render(...)`（`:164`） |
| `render_tree_to_png_with_font`（`:169-180`） | `(…, font_path, font_size) -> Result<Vec<u8>, String>` | `render_tree_to_rgba_with_font` → `png::encode_rgba`（`:177-179`） |
| `layout_tree`（`:183-193`） | `(&Node, w, h, Theme) -> layout::layout::Geometry` | 只算几何，用 `ApproxMeasure`；调试布局用 |

`prelude`（`:59-72`）重导出常用类型；`pub use deer_gpu::{self as gpu, …}` / `deer_layout as layout` / `deer_vk as vk` 在 `:42-44`。crate 级 lint：`#![forbid(unsafe_code)]`、`#![deny(clippy::all)]`（`:39-40`）—— 这两条是「门禁为什么必须 clippy 干净」的来源之一。

### 2.2 树构建入口在哪

- **命令式**：`crates/deer-layout/src/builder.rs` 的 `Builder` / `L`（经 prelude 暴露，`:68`），用法见 `docs/features/imperative-api.md`。
- **场景文件**：`crates/deer-layout/src/scene.rs` 的 `parse_scene` / `encode_scene`（prelude `:71`），用法见 `docs/features/scene-file.md`。
- 两条路径产出**结构相等**的树：`examples/render_to_png.rs:31-37` 用 `encode_scene` → `parse_scene` → `tree.structurally_eq(&from_file)` 现场断言。

### 2.3 `window` feature 与 `deer-window` 的可选依赖关系

`crates/deer-gui/Cargo.toml`：

```toml
: 9  [dependencies]
:10  deer-layout.workspace = true
:11  deer-gpu.workspace = true
:12  deer-vk.workspace = true
:13  # 窗口层是**可选**的：只有开 `window` feature 才会把 winit 拉进来，
:14  # 这样「只用离屏渲染 / 布局」的使用者不会被动拿到第三方依赖。
:15  deer-window = { workspace = true, optional = true }
:17  [features]
:18  default = []
:19  # 真窗口预览（M2b）：`cargo run -p deer-gui --features window --example window_preview`
:20  window = ["dep:deer-window"]
:22  [[example]] name = "window_preview"  required-features = ["window"]    # :22-24
:26  [[example]] name = "hal_window_path" required-features = ["window"]   # :26-28
```

- Rust 侧的开关：`crates/deer-gui/src/lib.rs:51-52` 的 `#[cfg(feature = "window")] pub use deer_window as window;`（文档注释在 `:46-50`）。
- 于是：**不开 feature 时 `deer-gui` 也零第三方依赖**；开了才拿到 `winit`。这条口径在 `README.md:39-41`、`ROADMAP.md:113-118`、`docs/features/window.md:100-105` 反复登记。

---

## 3. 示例清单

`crates/deer-gui/examples/` 共 **15** 个示例（另有 `crates/deer-window/examples/window_smoke.rs` 一个）。下表「实测输出/退出码」是我本轮**实跑**得到的（命令：`cmd /c "cargo run -q -p deer-gui --example <名>"`）。

| 示例 | 做什么（照抄源码注释/断言） | 怎么跑 | 实测产物 / 退出码 |
|---|---|---|---|
| `tutorial.rs`（270 行） | 分步教程：依次构建 6 个由简到繁的界面，每步打印几何与命令数，写图到 `render_out/*.png`；注释解释「为什么」（`examples/tutorial.rs:1-13`、`:36-44`） | `cargo run -p deer-gui --example tutorial` | `render_out/01-hello.png …06-manual.png`；实测 `exit=0`，首行 `输出目录：render_out`、`render_out\01-hello.png （360×120，172998 字节，5 种颜色）` |
| `render_to_png.rs`（79 行） | 「现在就能跑」的最小示例：建树 → 往返验证 → 算几何 → 写 `render_out/render_to_png.png`；断言 `distinct > 1`（「渲染结果只有一种颜色 ⇒ 什么都没画上」，`:69`）、两条路径结构相等（`:33-36`） | `cargo run -p deer-gui --example render_to_png` | 实测 `exit=0`，打印 `场景文件往返一致，节点数 = 10` + 几何表 |
| `scene_file.rs`（59 行） | 把 `.dui` 场景写盘（`render_out/settings.dui`，可手改重跑）→ `parse_scene` → `encode_scene` 往返断言（`:39`）→ 出图 | `cargo run -p deer-gui --example scene_file` | 实测 `exit=0`：`解析成功：14 个节点`／`往返一致 ✅`／`渲染出图：render_out/scene_file.png（312348 字节）` |
| `geometry.rs`（58 行） | 只看布局：打印 `nodeId → x,y,w,h` 表 + `hit_test` 命中结果；断言 `geo.len() >= 5`、按钮内部命中自己、画布外命中 `None`（`:46-55`） | `cargo run -p deer-gui --example geometry` | 实测 `exit=0`：`画布 360×200，共 6 个节点` + 表格 |
| `theme.rs`（78 行） | 一套树换三套配色（dark/light/warm），各出一张图（`:9-45`） | `cargo run -p deer-gui --example theme` | 实测 `exit=0`，三行：`render_out/theme-dark.png (240283 字节)` / `theme-light.png` / `theme-warm.png` |
| `draw_list.rs`（71 行） | 拿到绘制命令清单并逐条打印 + `counts()` 统计；断言 `clip_balanced()`（`:65`）与「禁用按钮必须用 border 色填充」（`:76`） | `cargo run -p deer-gui --example draw_list` | 实测 `exit=0`：`绘制命令共 12 条` + 逐条明细 |
| `pixels.rs`（69 行） | 直接拿 RGBA8 缓冲，打印 4 个采样点；断言长度 `w*h*4`（`:22-26`）、颜色数 > 1（`:56`） | `cargo run -p deer-gui --example pixels` | 实测 `exit=0`：`拿到 120×80 的 RGBA8 缓冲，共 38400 字节`；产物 `render_out/pixels.png` + `.ppm` |
| `text_render.rs`（126 行） | 真实字形渲染；自检墨迹像素 > 500、灰阶 > 8 种、逐帧确定性、缺字 0、字形数 > 20（`:109-117`） | `cargo run -p deer-gui --example text_render` | 实测 `exit=0`：`字体：C:\Windows\Fonts\consola.ttf`、`光栅化字形 57 个 / 缺字 0 / 图集 512×512 / 墨迹像素 9879 / 灰阶 52` |
| `glyph_atlas.rs`（200 行） | 把 94 个可见 ASCII 在 @16/@24 两档光栅化并打进货架打包图集，导出 `render_out/glyph_atlas.png`；5 条自检：槽位字节一致、两两不重叠、利用率 > 实测下界 `MIN_UTILIZATION = 0.30`（`:30`）、至少一字形 `max_coverage()==255`、两次构建逐字节相同（`:104-139`）；**找不到系统字体就明确报错并非 0 退出**（`:44-45`、`:51-57`） | `cargo run -p deer-gui --example glyph_atlas` | 实测 `exit=0`：`插入字形数：188（2 档字号 × 可见 ASCII）`、`图集尺寸：256 × 256` |
| `gpu_geometry.rs`（245 行） | 把**非文本** `DrawList` 交给 Vulkan 画，并与 CPU 基准**逐字节对照**（M3a）；断言 `counts.text == 0`、裁剪栈平衡、`stream.unsupported.is_empty()`、GPU/CPU 长度相等、`max_diff == 0`、两次渲染逐字节相同、额外场景（裁剪+嵌套裁剪+粗描边）差 0（`:98-202`）；产物 `render_out/gpu_geometry.png` 与 `gpu_geometry_cpu.png`（`:204-209`） | `cargo run -p deer-gui --example gpu_geometry`（`DEER_GPU_ADAPTER` 选卡，默认 0，`:16`） | 实测 `exit=0`：`绘制命令 : 14 条（填充 0 / 圆角 7 / 描边 7 / 裁剪 0 对 / 文本 0）`、`顶点数 : 210`、适配器 Intel RaptorLake-S |
| `gpu_offscreen.rs`（161 行） | GPU 离屏渲染链路全程走通到像素（建 image → 渲染通道 → 管线 → 命令缓冲 → 提交 → 栅栏 → `copyImageToBuffer` → map）；断言回读长度 `w*h*4`（`:89-93`）、三角形真被画出来（`:135`）、`(24,12)` 是绿色（`:148`）；产物 `render_out/gpu_offscreen.png`（放大 3 倍） | `cargo run -p deer-gui --example gpu_offscreen` | 实测 `exit=0`：`设备：Intel(R) RaptorLake-S Mobile Graphics Controller`、`适配器数：2` |
| `vulkan_devices.rs`（70 行） | 打印 CPU 后端与 Vulkan 后端的适配器、打开逻辑设备；`assert!(!vk.adapters().is_empty())`（`:31`）；**无 Vulkan 时优雅退出**（打印说明后 `return`，`:33-37`） | `cargo run -p deer-gui --example vulkan_devices` | 实测 `exit=0` |
| `vulkan_pipeline.rs`（125 行） | 从设备到「可用的图形管线」：渲染通道 → 管线布局 → 着色器模块 → `vkCreateGraphicsPipelines`；**明确标注它用的推送常量着色器已知损坏**，检测到 `DEER_VK_VALIDATION` 时打印警告并 `return`（显式跳过，不是通过）（`:19-28`） | `cargo run -p deer-gui --example vulkan_pipeline` | 实测 `exit=0`（未设校验层，正常跑完） |
| `window_preview.rs`（288 行） | 真窗口里看 GPU 画面（M2b）：呈现 N 帧后自动退出；第一帧回读像素核对（几何真画上去了 + 清屏色是 sRGB 编码后的已知值）；断言帧数达标、耗时/帧率为正（`:164-200`） | `cargo run -p deer-gui --features window --example window_preview`（`required-features = ["window"]`） | 实测 `exit=0`：`呈现帧数 : 120`、`交换链过期 : 0 次`、`耗时 : 2.16 s（55.5 帧/秒）`、`最终交换链 : 960×600`、`像素回读 : 四角 [71, 79, 105, 255] = sRGB 编码后的清屏色 rgb(0x10,0x14,0x24)`、`非清屏色像素 99360 / 576000（17.25%）`、`[deer-window] 事件循环结束：frames=120 extent=960x600 result=ok` |
| `hal_window_path.rs`（162 行） | HAL 窗口链端到端：`VkBackend::open(0)` → `create_swapchain` → `begin_frame/record/submit_and_present` → `wait_idle`；断言 `record(含绘制命令)` ⇒ `Unsupported` 且错误信息里含 `M3`（`:102-106`） | ① 不设变量：`cargo run -q -p deer-gui --features window --example hal_window_path`；② 完整：`$env:DEER_VK_WINDOW_TESTS='1'; $env:DEER_VK_VALIDATION='1'; cargo run -q -p deer-gui --features window --example hal_window_path`（`DEER_HAL_FRAMES` 默认 30） | ① 实测 `exit=0`，打印 `跳过：HAL 窗口路径需要真实窗口（设 DEER_VK_WINDOW_TESTS=1 启用）。` + `本次**没有**验证 HAL 的 create_swapchain / submit_and_present —— 这不是通过。`（`:139-143`）—— **跳过也算 exit=0，别当证据**。② 实测 `exit=0`：`[hal] 适配器 : Intel(R) RaptorLake-S Mobile Graphics Controller`、`[hal] 交换链 : 640×480 / 格式 Bgra8Srgb`、`[hal] 边界 : record(空)=OK / record(FillRect)=Unsupported(M3) ✅`、`[deer-window] 事件循环结束：frames=30 extent=640x480 result=ok`、`HAL 路径跑完 ✅（边界检查已执行、wait_idle 已调用）` |
| `crates/deer-window/examples/window_smoke.rs`（138 行） | deer-window 的真开窗冒烟：默认开 320×200 窗口连续重绘 30 帧后 `Flow::Exit`，然后自检 `redraw` 成功次数 ≥ 30；`DEER_WINDOW_HOLD=1` 留窗、`DEER_WINDOW_FAIL=1` 验证错误路径、`DEER_WINDOW_FRAMES=<n>` 改帧数（`:9-17`） | `cmd /c "cargo run -q -p deer-window --example window_smoke"`（错误路径加 `$env:DEER_WINDOW_FAIL='1'`） | 默认：实测 `exit=0`，`[deer-window] 事件循环结束：frames=30 extent=320x200 result=ok`、`[smoke] 自检通过：run() 返回 Ok，成功重绘 30 帧（>= 30）`。`DEER_WINDOW_FAIL=1`：实测 **`exit=0`**（示例把「预期失败」当成功），`[deer-window] 事件循环结束：frames=0 extent=320x200 result=error`、`[smoke] 预期错误已捕获：App::init 失败：DEER_WINDOW_FAIL=1：故意让 init 失败（验证错误路径）`、`[smoke] 自检通过（错误路径）：run() 返回 Err 且 error 文案非空；实际画了 0 帧` |

---

## 4. 测试与门禁

### 4.1 `crates/deer-gui/tests/*.rs`

**只有一个测试文件**：`crates/deer-gui/tests/docs_consistency.rs`（209 行；条数以 `--list` 输出为准）。它把「文档契约」变成可执行规则（`:1-11` 的背景：M1 交付后出现过「库能跑，但使用者不知道如何渲染任何东西」）。

它**具体校验什么规则**（逐条带行号）：

| # | 测试（行号） | 校验的规则 | 失败信息长什么样 |
|---|---|---|---|
| 1 | `features_manifest_guides_exist`（`:62-78`） | `FEATURES.md` 里每个 `](xxx.md)` 链接（**只取 .md，去掉 `#` 锚点**，`:46-56`）在仓库根下**必须存在** | `FEATURES.md 里有指向不存在的指南的链接：{missing:#?}` （`:74-77`） |
| 2 | `features_manifest_examples_exist`（`:80-100`） | ① 清单里至少提到 **5** 个示例；② 每个 `--example <名字>`（解析见 `:31-43`，允许字母数字/`_`/`-`）必须有 `crates/deer-gui/examples/<名字>.rs` | ① `清单里应当提到至少 5 个示例，实际 {} —— 若你删了示例，也要同步清单`（`:85-89`）；② `FEATURES.md 提到的示例没有源码文件：{missing:#?}（应在 crates/deer-gui/examples/<名字>.rs）`（`:96-99`） |
| 3 | `every_completed_feature_has_guide_and_example`（`:102-140`） | 只看**功能表格行**（以 `\|` 开头、含 `✅`、且不含「完成，有示例」图例行，`:111-118`）：每行**必须同时**有 ① `.md` 指南链接、② `--example` 命令；并要求这样的行**至少 8 行** | 行数不足：`应当有至少 8 行已完成功能，实际 {checked_rows} —— 若这是真的，说明清单被删减了`（`:131-134`）；缺项：`以下 ✅ 功能没有「指南 + 示例」齐备（规则见 FEATURES.md）：\n  {缺失项逐行}`，缺失项文案是 `缺少指南链接：{第一单元格}` / `缺少可运行示例：{第一单元格}`（`:122-128`、`:135-139`） |
| 4 | `guide_files_mention_existing_examples_and_are_complete`（`:142-180`） | 遍历 `docs/features/*.md`（**跳过 `TEMPLATE.md`**，`:156-158`），对每份指南：① 提到的 `--example` 源码必须存在；② 必须含字符串 **`做不到`**（即必须有「做不到什么」一节）；③ **不能**残留未勾选的 `- [ ]`；并要求指南**至少 6 份** | 指南数不足：`应当有至少 6 份指南，实际 {guides}`（`:178`）；逐项：`{name}: 提到示例 \`{ex}\`，但没有 examples/{ex}.rs` / `{name}: 缺少「做不到什么」（见 TEMPLATE.md 第 6 节）` / `{name}: 检查清单里还有未勾选项（- [ ]）`，最后汇总成 `指南检查失败：\n  {problems}`（`:162-179`） |
| 5 | `tutorial_and_manifest_are_linked_from_readme`（`:182-198`） | `README.md` 必须同时含三个字符串：`docs/TUTORIAL.md`、`FEATURES.md`、`docs/features/` | `README 必须链接教程（否则新手找不到入口）` / `README 必须链接功能清单` / `README 必须链接逐功能指南目录`（`:186-197`） |

辅助函数：`repo_root()`（`:17-23`，`CARGO_MANIFEST_DIR` 上两级）、`read()`（`:25-28`，读不到就 `panic!("读不到 {}：{e}")`）、`first_cell()`（`:201-209`，报错时指明是哪个功能）。

> **注意**：规则 3 只在**行级**要求「同一行里同时有链接和 `--example`」；指南的完整性（「做不到」、勾选项）由规则 4 管；`docs/tour/*` 这类新文档**不在** `docs_consistency` 的扫描范围里（规则 4 只遍历 `docs/features`）。

### 4.2 四道门禁的确切命令与取数方法

命令统一按仓库约定写成 `cmd /c "cargo ..."`（`docs/superpowers/plans/2026-09-27-m3a-drawlist-to-gpu-geometry.md:19`）。

> **本仓库不在文档里固化测试条数**（数字随每次加测试漂移：本会话就漂过多轮）。
> 下表只给**确切命令**与**怎么取数** —— 一律**以运行输出为准**。

| 门禁 | 确切命令 | 取数方法 / 判据（可直接复制） |
|---|---|---|
| **① workspace 测试** | `cmd /c "cargo test --workspace"` | `cargo test --workspace 2>&1 \| Select-String "^test result:"`（每个 target 一行汇总；判据：全 `0 failed`） |
| **② clippy（带 window feature）** | `cmd /c "cargo clippy --workspace --all-targets --features deer-gui/window"` | 判据：**0 warning / 0 error** 且 exit code 0 |
| **③ docs_consistency** | `cmd /c "cargo test -p deer-gui --test docs_consistency"` | 判据：`test result:` 行全过（这份文档契约测试的条数同样以输出为准） |
| **④ 示例全跑** | 13 个 `cmd /c "cargo run -q -p deer-gui --example <名>"` + 2 个 `--features window` 示例 + `cargo run -q -p deer-window --example window_smoke` | 判据：逐个 `exit=0`（`hal_window_path` 未设 `DEER_VK_WINDOW_TESTS` 时是「显式跳过」，也算 `exit=0`） |
| **（附加）deer-vk 带校验层** | `$env:DEER_VK_VALIDATION='1'; cmd /c "cargo test -p deer-vk"` | `$env:DEER_VK_VALIDATION='1'; cargo test -p deer-vk 2>&1 \| Select-String "^test result:"`；消息计数见下方**精确命令** |
| **（附加）真窗口完整门禁** | `$env:DEER_VK_WINDOW_TESTS='1'; $env:DEER_VK_VALIDATION='1'; cmd /c "cargo test -p deer-vk"` | 本轮**未实测**（见 §8）；依据 `ROADMAP.md` 的 M2b 门禁段、`README.md`、`docs/features/window.md` |

**校验消息计数：必须用「行首括号前缀 + 大小写敏感」**（本轮踩过的坑）：

`Select-String` **默认不区分大小写**，用松散模式（例如 `VALIDATION|VK ERROR`）数消息会**假命中** ——
实测命中 **5** 条，**全是用例名/靶名**（`validation_probe`、`text_is_reported_...` 之类），一条真校验消息都没有。
判据必须是 0，命令照抄：

```powershell
# 真校验消息（行首 [VK ERROR] / [VALIDATION] 前缀、区分大小写）
$env:DEER_VK_VALIDATION='1'
cargo test -p deer-vk 2>&1 |
  Select-String -CaseSensitive -Pattern '^\[VK ERROR\]|^\[VALIDATION\]' |
  Measure-Object | Select-Object -ExpandProperty Count   # 判据：0
```

**各 crate 的测试靶构成（结构，不是条数）** —— 要知道某个靶多少条，用 `--list` 现取：

| crate | 测试靶（`tests/*.rs`）与 lib |
|---|---|
| `deer-gpu` | lib + `draw_list_and_cpu_backend`、`font_parse`、`font_synthetic`、`glyph_atlas`、`render_pipeline`、`text_measure`、`text_pixels`、`text_raster` |
| `deer-gui` | lib + `docs_consistency`（文档契约） + doc-test |
| `deer-layout` | lib + `layout_invariants`（doc-test 里有 1 条 **ignored**，是 workspace `ignored` 计数的来源之一） |
| `deer-vk` | lib + `device_smoke`、`export_spirv`、`gpu_geom_parity`、`gpu_geom_stream`、`gpu_text_stream`、`gpu_vs_cpu`、`offscreen_render`、`pipeline_smoke`、`raw_ffi_probe`、`spirv_val`、`struct_layout`、`swapchain_smoke`、`validation_probe`、`vbo_probe`、`vulkan_smoke` + doc-test |
| `deer-window` | lib + `window_logic` + doc-test |

取数示例（照抄即可）：

```powershell
cargo test -p deer-vk --test gpu_vs_cpu -- --list                 # 末尾一行 "N tests, 0 benchmarks"
cargo test --workspace 2>&1 | Select-String "^test result:"       # 每个靶一行汇总
```

> **「以运行输出为准」是本仓库的明文规矩**：`ROADMAP.md` 的 Q-5、`FEATURES.md` 的推送常量着色器行、
> `docs/features/vulkan.md`、`docs/features/vulkan-swapchain.md` 现在都写成
> 「**全部通过 / 0 failed（具体条数以运行输出为准）**」。`docs/superpowers/plans/2026-09-27-m3a-drawlist-to-gpu-geometry.md:37-38`
> 还留了一条历史：计划里写 `203 passed`，实测基线却是 `277 passed` —— 这类被固化的数字迟早会撒谎，
> 所以**本文不再持有它们**（要数字就现跑，commands 见上）。

---

## 5. 纪律与约定

> ⚠️ **`AGENTS.md` 不存在**：根目录、`crates/`、`docs/` 下都**没有** `AGENTS.md`（`glob "**/AGENTS.md"`、`glob "**/*AGENT*"` 均为空；`git log --all -- AGENTS.md` 无输出）。它只被两处**条件引用**：`docs/superpowers/plans/2026-09-27-codebase-guided-tour.md:11`（「根 `AGENTS.md`（**若存在**）」）与同文件 `:79`（把 `AGENTS.md` 列为第 9 讲材料）。所以本节的纪律**全部来自仓库内的实际文档**，不是 `AGENTS.md`。

### ① 每条结论要「命令 + 输出」

- 导览计划把它写成硬性 Global Constraint：**「每讲必须带 `path:line` 证据锚点」**、**「每讲必须给一条可运行命令（`cmd /c "cargo ..."`）让你自己复现结论」**、**「事实以仓库现状为准；发现文档与代码不一致要当场指出」**（`docs/superpowers/plans/2026-09-27-codebase-guided-tour.md:15-18`）。
- 第 9 讲的自测题就是「能…说出『每条结论都要有命令+输出』的纪律来源」（`:80`）。
- 落地形态：文档里的数字都附命令，例如 `docs/features/gpu-geometry.md:177`「跑法：`cargo test -p deer-vk --test gpu_vs_cpu -- --nocapture`（会逐场景打印最大通道差）」；`docs/features/window.md:196-201` 给出「期望 stdout 里有两行」。
- **数字不许当常数**：所有引用测试数的地方都跟一句「以运行输出为准」（见 §4.2 末的引用列表）。

### ② 变异测试（改坏必须变红）的实际做法与已有例子

做法：**临时把被守护的行为改坏 → 对应测试/断言必须变红 → 改回**。已有例子：

| 例子 | 位置 | 说明 |
|---|---|---|
| 计划里的步骤化变异 | `docs/superpowers/plans/2026-09-27-m3a-drawlist-to-gpu-geometry.md:266` | 「**变异验证判据有区分度**：临时把片元着色器的圆角判定改为『总是 inside』→ 圆角用例必须变红；改回」 |
| 5 个真实缺陷的守卫 | `docs/M1-report.md:56-70` | 「全部有『改坏 → 红 → 改回』的验证（V0 的 B-1/B-2/B-3 是继承来的守卫；R-1/R-2 是本次新抓的）」，逐条列出缺陷 + 后果 + 守卫测试名 |
| `OutOfDate` 映射 | `docs/features/window.md:213` | 「把 `OutOfDate` 映成 `Presented` 会立刻变红」 |
| host→vertex 屏障 | `docs/features/gpu-geometry.md:173` | 「（删掉那条屏障 ⇒ 变红）」 |
| 「跳过也算 pass」的对照实验 | `ROADMAP.md:57-59` | 「验证者实测：把 `windowed.rs::resize` 改成空操作后，**不设该变量时 25/25 全绿、设了才 1 failed**」——这是**用变异发现「假绿」**的实例 |

### ③ 为什么不用 `cargo fmt`

- **直接理由**：仓库**非 fmt-clean**，跑 `cargo fmt` 会产生大量无关 diff。原话：「**不运行 `cargo fmt`（仓库非 fmt-clean）**。命令一律 `cmd /c "cargo ..."`」（`docs/superpowers/plans/2026-09-27-m3a-drawlist-to-gpu-geometry.md:19`）。
- 导览计划同样列为 Global Constraint：「教学文档一律**中文**；**不跑 `cargo fmt`**；新增文档不影响 `docs_consistency` 门禁（它只校验 FEATURES 的 ✅ 行）」（`docs/superpowers/plans/2026-09-27-codebase-guided-tour.md:19`），并把它变成第 9 讲的自测题（`:81`）。
- 代码里的佐证：多处刻意保留**手工对齐**的写法（例如 `crates/deer-window/src/lib.rs:89` 的单行结构体字面量、`crates/deer-gui/tests/docs_consistency.rs:42-43` 的换行），说明格式化是**手写风格**而非 rustfmt 产物。

### ④ 文档契约（✅ FEATURES 行需要指南链接 + `--example` 命令）

- **规矩（三处重复声明）**：`FEATURES.md:3-8`「每个功能必须有：**状态 + 使用指南 + 可运行示例**」；`README.md:24-25`「**维护约定**：新增功能时**必须同时**加示例 + 加指南 + 登记 `FEATURES.md`（缺一不算完成）」；`FEATURES.md:89-97` 的四步清单（加示例 → 加指南 → 登记本文件 → 更新 TUTORIAL）。
- **可执行形式**：`crates/deer-gui/tests/docs_consistency.rs` 规则 3（`:102-140`）就是这条契约的机器化。
- **示例自身的额外要求**（`FEATURES.md:91-94`）：顶部注释写明「这是什么 + 怎么跑 + 产物在哪」；结尾有**自检断言**（例如「画面不能只有一种颜色」）；真跑一遍确认 `exit=0`。
- **指南的额外要求**：照 `docs/features/TEMPLATE.md` 七节写（`FEATURES.md:95`），其中**第 6 节必须写「做不到什么」**，且发布前检查清单**不能留 `- [ ]`** —— 两条都由 `docs_consistency.rs:168-175` 强制。

### ⑤ 中文注释 / 提交信息约定

- **文档与注释一律中文**：导览计划 Global Constraint「教学文档一律**中文**」（`docs/superpowers/plans/2026-09-27-codebase-guided-tour.md:19`）；仓库内所有 `//!` 文档注释、错误信息、`println!` 输出都是中文（例：`crates/deer-window/src/lib.rs:57-58`、`crates/deer-window/tests/window_logic.rs:98`、`docs_consistency.rs:76` 的断言文案）。
- **注释写「为什么」不写「是什么」**：`crates/deer-gui/examples/tutorial.rs:11-12`「源码里每个 `//` 注释都解释『为什么』，而不是『是什么』」。
- **提交信息**：用 Conventional Commits 前缀 + **中文正文**，实测（`git log --oneline -8`）：
  `9df8ade merge(m3a): DrawList 的非文本命令上 GPU，并与 CPU 后端逐像素对照`、`6317afb docs(m3a): 校验层测试数 161 -> 167（自身复核实测）+ 注明以运行输出为准`、`38f23bb fix(deer-vk): 终轮 —— 越界 alpha 按 CPU 基线 clamp（M1）+ 关掉 radius_kind 陷阱值（M4-3）+ Text 假阳性明确 defer`。计划里的示例提交命令也是这个形状（`docs/superpowers/plans/2026-09-27-m3a-drawlist-to-gpu-geometry.md:136`）。
- **`unsafe` 必须带 `// SAFETY:`**：`docs/superpowers/plans/2026-09-27-m3a-drawlist-to-gpu-geometry.md:17`（「每处 `unsafe` 写 `// SAFETY:`；`cargo clippy …` 必须 0 warning」）。`deer-gui` 自身更严：`#![forbid(unsafe_code)]`（`crates/deer-gui/src/lib.rs:39`）。

### ⑥ 依赖规则（deer-vk / deer-gpu / deer-layout 零第三方依赖，窗口层 winit 例外）

- **规则条文**：`ROADMAP.md:109-118`「依赖纪律（硬性）」三条 —— ① 不引图形抽象库（无 `wgpu`/`ash`/`vulkano`/`glow`）；② 不引 GUI 框架（无 `egui`/`iced`/`tauri`）；③ **最小生态依赖**：目前**1 个登记在案的例外** = `deer-window` 的 `winit`；`deer-layout`/`deer-gpu`/`deer-vk` 与**没开 `window` feature 的 `deer-gui`** 仍然零第三方依赖。
- **口径纪律（原文）**：`ROADMAP.md:116`「**『零依赖』这个说法此后一律写成『除窗口层（`winit`，已登记）外零第三方依赖』—— 不要再写『完全零依赖』**」。
- **例外登记位置**：`ROADMAP.md:120-159`「依赖例外登记（Q-1：窗口层引 `winit`）」，四小节 ① 引了什么（含 `cargo tree -p deer-window --target x86_64-pc-windows-msvc -e normal` 的实测依赖树，`:122-144`）② 为什么（与自写 Win32 的对比，`:146-151`）③ 影响面（`:153-155`）④ 如何撤回（`:157-159`）。
- **其它登记点**：`ROADMAP.md:172`（Q-1 表格行）、`README.md:34` 与 `:39-41`、`FEATURES.md` 相关行、`crates/deer-window/Cargo.toml:3` 与 `:15-16`、`crates/deer-gui/Cargo.toml:13-15`、`docs/features/window.md:100-105`。
- **「未引」也要登记**：`ROADMAP.md:117-118`「字体解析原先预计可能需要 `ttf-parser`，**实际未引**：M4-1 自研」；`ROADMAP.md:94-95` 同口径（「M4 当时还没有窗口层的 `winit` 例外 —— 该例外是 M2b 引入的」）。

---

## 6. 文档地图（仓库里所有 `.md`）

### 6.1 根目录

| 路径 | 一句话内容 | 什么时候读 |
|---|---|---|
| `README.md` | 项目首页：定位、现状表、快速开始、架构图、布局不变式、三个继承缺陷、为什么不需要 Vulkan SDK | 第一次接触项目；想知道「现在能做什么」 |
| `FEATURES.md` | **功能状态的唯一真相**：✅/🔄/⬜ 三分区 + 每个 ✅ 的指南链接与 `--example` 命令 + 维护者四步清单 | 想知道某个功能能不能用；新增功能前后对账 |
| `ROADMAP.md` | 里程碑 M1–M7 与子步状态、**依赖纪律 + 依赖例外登记（Q-1）**、Q-1..Q-5 遗留登记 | 想知道「下一步做什么」；任何涉及第三方依赖的决策 |
| `.superpowers/sdd/2026-09-27-m3a-drawlist-to-gpu-geometry/progress.md` 等 14 份 brief/report/review | M3a 这一轮的 SDD 过程记录（task-1..5 的 brief/report/review、final-review） | 想知道 M3a 某个决定是怎么来的；复盘过程 |

### 6.2 `docs/`

| 路径 | 一句话内容 | 什么时候读 |
|---|---|---|
| `docs/TUTORIAL.md` | 14 节（§0–§13）分步教程，每节可独立运行；末尾有「可运行示例一览」表 | 想按顺序上手；照抄某个功能的完整用法 |
| `docs/GETTING-STARTED.md` | 「假设你从没写过 Rust」的更啰嗦版本：13 节（含 6.3 只要布局、8 小白六个坑、11 接口速查） | Rust 新手；想查 `Theme` 字段或 prelude 内容 |
| `docs/M1-report.md` | M1 快照报告：要证明什么、实测输出、5 个缺陷的守卫、M1 没验证的事、下一步 | 想看「早期决策与缺陷史」；找变异测试的既有例子 |
| `docs/superpowers/plans/2026-09-27-m3a-drawlist-to-gpu-geometry.md` | M3a 的实现计划（Task 0–5 + Self-Review），含 Ruling（执行时改判）记录 | 想知道 M3a 的原始设计意图与改判点 |
| `docs/superpowers/plans/2026-09-27-codebase-guided-tour.md` | 本导览（`docs/tour/*`）的 11 讲计划 + Global Constraints | 想知道这些导览文档的组织方式与阅读顺序 |

### 6.3 `docs/features/*`（19 个 `.md`：18 份指南 + 1 份模板）

| 路径 | 内容一句话 | 什么时候读 |
|---|---|---|
| `docs/features/TEMPLATE.md` | 指南模板：七节（这是什么/最小示例/完整 API/自检/常见坑/相关/检查清单） | **写新指南前必读** |
| `imperative-api.md` | 命令式 `Builder`/`L`/`Kind`/`Node` + 两条路径等价 | 用 Rust 代码建树时 |
| `scene-file.md` | `.dui` 语法四条 + 属性表 + 报错带行号 + 往返互转 | 想在文本文件里写界面时 |
| `node-tree.md` | `Node` 结构、树上的方法、**id 规则** | 想遍历/校验/转换树时 |
| `layout.md` | 布局入口 `layout()`/`measure_tree()`、`LayoutProps`（w/h/pad/gap/grow/main/cross） | 调界面尺寸时 |
| `hit-testing.md` | `hit_test(几何表, 坐标) → 节点`（最浅命中）；无事件派发 | 自己做交互实验/调试布局时 |
| `rendering.md` | 三个渲染入口 + 四步管线 + CPU 软件光栅化 | 只想出图时 |
| `pixels.md` | RGBA8 缓冲布局 + 零依赖 PNG 编码器 | 想自己处理像素/自己编码 PNG |
| `draw-list.md` | `DrawList`/`DrawCmd` 全部变体、`counts()`、`clip_balanced()` | 想看「布局→像素」之间那层时 |
| `theme.md` | `Theme` 字段（6 色 + font_size + line_height）与换主题 | 换配色/调字号时 |
| `text-rendering.md` | `FontMeasure`/`TextEngine`/`GlyphPlacement`/`CpuRenderer::with_text` 全链 | 想让图里出现真字时 |
| `glyph-raster.md` | 轮廓 → 覆盖率位图（nonzero + 超采样）、`GlyphImage` 坐标系 | 自己控制「字形→位图」时 |
| `glyph-atlas.md` | 货架打包 + 1px padding + 按需增高；`GlyphKey`/`AtlasSlot` | 要把字形传 GPU 或导出一张图集时 |
| `gpu-hal.md` | 五个 HAL trait（`Backend`/`Device`/`Swapchain`/`Frame`/`Renderer`）+ 两个后端 + 自己实现后端 | 想加一个渲染后端时 |
| `vulkan.md` | `deer-vk` 的真实边界表（能到哪一步、什么明确 `Unsupported`） | 想用 `deer-vk` 底层 API 时 |
| `vulkan-pipeline.md` | 渲染通道 → 管线布局 → 着色器模块 → `vkCreateGraphicsPipelines` | 排查管线创建问题时 |
| `gpu-offscreen.md` | 离屏 image + 渲染通道 + 提交 + 栅栏 + `copyImageToBuffer` 回读；含一份完整排查方法论 | 想在无窗口环境验证 GPU 渲染时 |
| `vulkan-swapchain.md` | `VkSurfaceKHR` + 交换链 + 帧同步 + 呈现；`OutOfDate` 的处理 | 要让 GPU 画面出现在窗口里时 |
| `gpu-geometry.md` | `GpuGeometryRenderer`：`DrawList` 的形状与**文本**命令上 GPU + 与 CPU 逐像素对照（用例清单见该指南第 4 节） | 做 GPU 几何/文本/parity 相关改动时 |
| `window.md` | `deer-window` 用法 + 环境变量 + **「为什么真窗口不是 `#[test]`」** + 8 条常见坑 | 开窗口/接表面时 |

### 6.4 `docs/TUTORIAL.md` 的章节标题列表（带行号）

| 章 | 标题 | 行号 |
|---|---|---|
| — | `# deer-gui 教程` | `:1` |
| — | `## 目录`（表格） | `:9` |
| §0 | `## 0. 先跑起来` | `:30` |
| §1 | `## 1. 第一张图` | `:45` |
| §2 | `## 2. 横排与间距` | `:81` |
| §3 | `## 3. 容器嵌套与禁用态` | `:109` |
| §4 | `## 4. 换主题` | `:146` |
| §5 | `## 5. 用 \`.dui\` 文件写界面` | `:173` |
| §6 | `## 6. 只看布局，不出图` | `:225` |
| §7 | `## 7. 拿到绘制命令` | `:253` |
| §8 | `## 8. 拿原始像素` | `:284` |
| §9 | `## 9. 建你自己的项目` | `:305` |
| §10 | `## 10. 现状与边界` | `:332` |
| §11 | `## 11. 真实文字` | `:353` |
| §12 | `## 12. 在窗口里看到画面` | `:400` |
| §13 | `## 13. 用 GPU 画界面` | `:448` |
| 附 | `## 附：可运行示例一览` | `:504` |

### 6.5 `docs/superpowers/plans/*` 的章节标题

**`2026-09-27-codebase-guided-tour.md`（95 行）**：

`# deer-gui 代码库导览计划…`（`:1`）、`## Global Constraints`（`:13`）、`### 第 0 讲：全景图 —— 五个 crate 各干什么、数据怎么流`（`:23`）、`### 第 1 讲：数据模型 —— DrawList / DrawCmd / 几何与颜色`（`:29`）、`### 第 2 讲：布局层 —— 盒模型与测量`（`:35`）、`### 第 3 讲：CPU 参考后端 —— 为什么它是最重要的代码`（`:41`）、`### 第 4 讲：字形与文本 —— 从字体文件到像素覆盖度`（`:47`）、`### 第 5 讲：Vulkan 底座 —— 手写绑定与 HAL`（`:53`）、`### 第 6 讲：自研 SPIR-V 汇编器 —— 最容易踩坑的一层`（`:59`）、`### 第 7 讲：GPU 几何渲染器与 parity —— 本项目的验收方式`（`:65`）、`### 第 8 讲：窗口与交换链 —— 让画面出现在屏幕上`（`:71`）、`### 第 9 讲：应用层与工程纪律`（`:77`）、`### 第 10 讲：拓展指南 —— 加一个 DrawCmd / 加一个控件 / 加一个后端`（`:83`）、`## 交付物与节奏`（`:91`）。

**`2026-09-27-m3a-drawlist-to-gpu-geometry.md`（291 行）**：

`# M3a — DrawList 上 GPU…`（`:1`）、`## Global Constraints`（`:13`）、`### Task 0: 隔离工作区`（`:24`）、`### Task 1: 汇编器补齐算子 + 两支新着色器`（`:48`）、`### Task 2: 纯逻辑 — DrawList → 顶点流（无需 GPU）`（`:141`）、`### Task 3: 离屏 GPU 几何渲染器`（`:210`）、`### Task 4: 与 CPU 后端逐像素对照（本计划的终局判据）`（`:241`）、`### Task 5: 文档 + 收尾`（`:271`）、`## Self-Review`（`:285`）。

### 6.6 其它

| 路径 | 内容 |
|---|---|
| `.superpowers/sdd/2026-09-27-m3a-drawlist-to-gpu-geometry/` | 14 份过程文档：`task-1…5-brief/report/review`、`task-2-fix1-review`、`task-2-fix1-extra-review`、`task-3-fix1-review`、`final-review.md`、`progress.md` |
| `docs/tour/03-app-and-process.md` | **本文件**（应用层与工程纪律事实地图） |

---

## 7. 「想改 X 该动哪里」

> 每条都给出**改动清单 + 必须跑的命令**；纪律依据见 §5。

### 7.1 加一个示例

1. 新建 `crates/deer-gui/examples/<名字>.rs`：顶部注释写「这是什么 + 怎么跑 + 产物在哪」，结尾写**自检断言**（`FEATURES.md:91-94`）。
2. 若需要窗口，在 `crates/deer-gui/Cargo.toml` 加 `[[example]] name = "<名字>" required-features = ["window"]`（照 `:22-24` / `:26-28`）。
3. 登记 `FEATURES.md` 对应表格行：状态 + `.md` 链接 + `--example` 命令（`FEATURES.md:96`）。
4. （属新手主线时）更新 `docs/TUTORIAL.md`（`FEATURES.md:97`）。
5. **必须跑**：`cmd /c "cargo run -q -p deer-gui --example <名字>"`（确认 `exit=0`）→ `cmd /c "cargo test -p deer-gui --test docs_consistency"` → 门禁①②。

### 7.2 加一份指南并过 `docs_consistency`

1. 复制 `docs/features/TEMPLATE.md` 为 `docs/features/<名字>.md`，填七节（`TEMPLATE.md:3-4`）。
2. **第 6 节必须出现「做不到」三个字**，否则测试 4 报 `缺少「做不到什么」（见 TEMPLATE.md 第 6 节）`（`crates/deer-gui/tests/docs_consistency.rs:168-171`）。
3. **检查清单里不能留 `- [ ]`**，否则报 `检查清单里还有未勾选项（- [ ]）`（`:172-175`）。
4. 指南里每个 `--example <名>` 都必须有对应源码，否则报 `提到示例 … 但没有 examples/….rs`（`:162-166`）。
5. 在 `FEATURES.md` 的 ✅ 行里加上指向它的 `.md` 链接（否则测试 3 报 `缺少指南链接`，`:122-124`）。
6. **必须跑**：`cmd /c "cargo test -p deer-gui --test docs_consistency"`（期望 `test result:` 行全过；条数以输出为准）。注意测试 4 还有下限「至少 6 份指南」、测试 3 有「至少 8 行 ✅」（`:178`、`:131-134`）。

### 7.3 把某个第三方依赖引进来：先做什么登记

1. 先判断是否真需要：`ROADMAP.md:109-118` 的三条硬性纪律（不引图形抽象库 / 不引 GUI 框架 / 最小生态依赖）。
2. 若确需新增，**在 `ROADMAP.md` 的「依赖例外登记」小节加一节**，照 Q-1 的四件套写：① 引了什么（含 `cargo tree -p <crate> --target <triple> -e normal` 的实测依赖树）② 为什么（与替代方案的对比）③ 影响面（哪些 crate 会拿到它）④ 如何撤回（`ROADMAP.md:120-159`）。
3. 在 `ROADMAP.md:172` 的 Q-1 表格区域新增一行（形如「Q-n」），并同步 `README.md:39-41` 的「依赖口径」段与 `crates/<crate>/Cargo.toml` 的注释（照 `crates/deer-window/Cargo.toml:15-16`）。
4. 口径用语：**不许再写「完全零依赖」**，改写「除窗口层（`winit`，已登记）外零第三方依赖」（`ROADMAP.md:116`）。
5. **必须跑**：`cmd /c "cargo clippy --workspace --all-targets --features deer-gui/window"`（新依赖可能引入 lint）+ 门禁①（确认没有破坏既有断言）+ `cargo tree` 复现登记里的依赖树。

### 7.4 加一个 `DrawCmd` 变体

1. 定义在 `crates/deer-gpu/src/draw.rs`（`DrawCmd` 契约，见 `docs/features/draw-list.md:47-59`）。
2. CPU 后端要消费它：`crates/deer-gpu/src/null.rs`（**它是像素语义基准**，改它等于改基准 —— `docs/superpowers/plans/2026-09-27-m3a-drawlist-to-gpu-geometry.md:16`）。
3. GPU 侧要么实现、要么**明确 `Unsupported` 并报出来**：`crates/deer-vk/src/gpu_geom.rs` 的 `build_stream`（把不支持的命令记进 `GpuStream::unsupported`，**不许静默丢弃**，见计划 `:180-186`）。
4. 需要的对着测试：`crates/deer-vk/tests/gpu_geom_stream.rs`（纯逻辑，28 条）+ `crates/deer-vk/tests/gpu_vs_cpu.rs`（与 CPU 逐像素，12 条）+ `crates/deer-gpu/tests/draw_list_and_cpu_backend.rs`。
5. 文档：`docs/features/draw-list.md` 的变体表 + 若影响 GPU 侧则 `docs/features/gpu-geometry.md` 的「支持 / 不支持的 `DrawCmd`」表。
6. **必须跑**：门禁①②③④，尤其 `DEER_VK_VALIDATION=1` 下的 `cargo test -p deer-vk --test gpu_vs_cpu`（零校验消息）。

### 7.5 改窗口行为（例如加一个新事件转发 / 加一个环境变量）

1. 事件 → 回调的映射表在 `crates/deer-window/src/lib.rs:351-390`；加事件就在 `window_event` 的 `match` 里加臂，**其余事件仍然丢弃**（`:387-388`，输入层留给 M5）。
2. 若是新的 `App` 回调：改 `trait App`（`:114-137`）—— 加**带默认实现**的方法可保持既有调用方不破；同时更新 crate 文档里的用法示例（`:12-30`）。
3. 纯逻辑部分必须能单测：把状态机放进像 `FrameCounter`（`:143-179`）那样的「不碰窗口」结构，测试写进 `crates/deer-window/tests/window_logic.rs`。
4. 真窗口行为只能靠**示例**验证：`crates/deer-window/examples/window_smoke.rs` 或 `crates/deer-gui/examples/window_preview.rs`（原因见 §1.5）。
5. 文档：`docs/features/window.md`（含常见坑表 `:216-227`）；若涉及上屏/交换链，还要动 `docs/features/vulkan-swapchain.md`。
6. **必须跑**：`cmd /c "cargo test -p deer-window"`（期望 `test result:` 行全过；条数以输出为准）→ `cmd /c "cargo run -q -p deer-window --example window_smoke"` → `cargo run -p deer-gui --features window --example window_preview`（`exit=0`、帧数达标、像素核对通过）→ 门禁①②③。

### 7.6 新增一项 ✅ 功能（完整「四件事」）

按 `FEATURES.md:89-97` 的顺序：**加示例 → 加指南 → 登记 FEATURES.md → 更新 TUTORIAL**（缺一不算完成）。改完跑门禁①②③④；其中③会替你检查「✅ 行是否有指南链接 + `--example` 命令」「指南是否有『做不到』且清单已勾完」。

---

## 8. 未确认项（读不到 / 未实测）

1. **`AGENTS.md` 不存在** —— 已用 `glob "**/AGENTS.md"`、`glob "**/*AGENT*"`、`git log --all -- AGENTS.md` 三路确认均无结果。它只被 `docs/superpowers/plans/2026-09-27-codebase-guided-tour.md:11` 以「**若存在**」的措辞条件引用。**因此本文件 §5 的纪律来源全部是仓库内实际文档，不含 `AGENTS.md`。**
2. **`docs/tour/*` 不在 `docs_consistency` 的扫描面内**：规则 4 只遍历 `docs/features`（`crates/deer-gui/tests/docs_consistency.rs:145`），规则 1/3/5 只看 `FEATURES.md` 与 `README.md`。所以新增导览文档**不影响**该门禁（与 `docs/superpowers/plans/2026-09-27-codebase-guided-tour.md:19` 的说法一致）。
3. **真窗口完整门禁**（`$env:DEER_VK_WINDOW_TESTS='1'; $env:DEER_VK_VALIDATION='1'; cargo test -p deer-vk`）本轮**未实测**；文档记载它「必须显式打开」，且「不设变量时真窗口 e2e 显式跳过 —— **跳过也算 pass**」（`ROADMAP.md:51-59`、`README.md:56-57`、`docs/features/window.md:227`）。（注：该变量下的 `hal_window_path` **示例**已实测通过，见 §3。）
4. **四道门禁的「确切命令」清单没有单一权威来源**：`docs/superpowers/plans/2026-09-27-codebase-guided-tour.md:80` 只说「跑齐四道门禁」，`docs/superpowers/plans/2026-09-27-m3a-drawlist-to-gpu-geometry.md:20` 列的是**三条**（workspace 测试 / clippy / `DEER_VK_VALIDATION=1 cargo test -p deer-vk`）。本文件 §4.2 的第四道（示例全跑）是**我在本轮把「示例」也当作门禁实跑后补上的**，命令并非抄自某一份文档；若上游另有明文，请以那份为准。
5. **`docs/features/*.md` 的逐节标题**只对 §1 段落做了抽样核对；§6.3 表格里的「内容一句话」是**按各指南第 1 节的实读**概括的，未逐节通读全部 18 份指南。
6. `test_project/deer-hello` 的**内容未逐文件读**（只确认目录存在、且被 `Cargo.toml:10-14` 的 `exclude` 排除在 workspace 之外）。
7. `docs/superpowers/plans/*` 之外的 `.superpowers/sdd/**` 14 份文档只按文件名归类，**未逐份通读**。
8. 仓库内**没有 CI 配置文件**（根目录无 `.github/`、无 `*.yml`；见 `Get-ChildItem -Force` 输出），所以「四道门禁」是**人工执行的约定**，不是流水线强制 —— 这一点是观察所得，未见明文声明。
