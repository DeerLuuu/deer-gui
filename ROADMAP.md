# deer-gui 路线图

> 定位：**从零实现的 Rust GUI 运行时**。不依赖 web/DOM，不依赖 `wgpu`/`ash`/`vulkano`。
> 依赖边界：**连 GPU 后端也自己写**（Vulkan 先行，DX12/Metal 后续；抽象层保证「加后端 = 实现一个 trait」）。
> **唯一的第三方依赖例外**是窗口层的 `winit`（Q-1，M2b 引入），逐条登记见下面「依赖例外登记」。

## 里程碑

| # | 里程碑 | 内容 | 状态 |
|---|---|---|---|
| **M1** | **核心 + HAL + Vulkan 设备枚举** | 节点树、布局代数、命中测试、`.dui` 解析；GPU HAL + CPU 参考后端；Vulkan 实例与物理设备枚举（自己声明符号 + 动态加载） | ✅ **完成** |
| **M2** | Vulkan 逻辑设备 + 交换链 | M2a：逻辑设备/管线/命令/离屏回读；M2b：窗口 + `VkSurfaceKHR` + 交换链 + 帧同步 + 呈现 | ✅ **完成（M2a + M2b）** |
| **M3** | 渲染器 + 管线（矩形/圆角/裁剪/文本） | M3a：形状的 `DrawList` → GPU（顶点缓冲 + 静态管线 + 与 CPU 逐像素对照）；M3b：文本/字形 → GPU（第二条管线 + 图集纹理 + 逐像素对照）；M3c：把界面**呈到窗口**（共用管线层 + 线性交换链 + 上屏 parity）；还欠批处理优化 | 🔄 **进行中（M3a + M3b + M3c 完成，仅剩 M3+ 批处理优化）** |
| **M4** | 文本 | 字体解析（TTF/OTF）+ 字形光栅化 + 图集 + 文本度量（替换 `ApproxMeasure`）+ 换行 | 🔄 **进行中（解析 / 光栅化 / 图集 / 度量与换行 / CPU 真实字形 ✅；hinting 与亚像素待做）** |
| **M5** | 输入 + 焦点 + dock | 事件循环、命中测试路由（`hit_test` 已就位）、焦点系统（含方向键）、**可停靠面板布局**（拖动改位置 / 边缘折叠） | ⬜ |
| **M6** | 控件族 | 从 `deer-ui` 迁移 12 个控件的**语义**：`Btn`/`ChipGroup`/`Segmented`/`TabBar`/`Switch`/`NumberField`/`ScrubNum`/`ColorField`/`Dialog`/`Overlay`/`DropMenu`/`HoverTip`/`Icon`/`Row`/`RowActions`/`Keep` | ⬜ |
| **M7** | DX12 / Metal 后端 | 各自实现 HAL trait；用 `deer-gpu` 的 CPU 参考后端做像素级对照 | ⬜ |

### M2 的拆分（为什么拆）

M2 原本写成一条「设备 + 交换链 + 清屏出图」。但**交换链必须有窗口**，而窗口方案（Q-1）
当时未定。更要紧的是：把「设备/队列/命令/同步」与「窗口/呈现」**分开验证**，
出问题时才定位得到。**现在两半都完成了**：M2a 在无窗口环境下把设备/管线/回读做扎实，
M2b 拿到窗口后接上 surface/交换链/呈现（Q-1 已决：引 `winit`）。

| 子阶段 | 内容 | 是否需要窗口 | 状态 |
|---|---|---|---|
| **M2a** | 逻辑设备 + 队列 + 着色器模块 + 渲染通道 + 管线 + 命令缓冲 + **离屏图像 + 回读** + **与 CPU 后端逐像素对照** | ❌ 不需要 | ✅ 完成 |
| **M2b** | 窗口 + `VkSurfaceKHR` + 交换链 + 帧同步（信号量/栅栏）+ 呈现 | ✅ 需要（Q-1 已决） | ✅ 完成 |

#### M2a 的细步与状态

| 步 | 交付 | 状态 | 验收判据 |
|---|---|---|---|
| **M2a-1** | **自研 SPIR-V 汇编器**（`spirv.rs`） —— 解 Q-2 阻断点 | ✅ 完成 | 3 支着色器被驱动 `vkCreateShaderModule` 接受；模块自洽性有结构护栏（条数以 `cargo test -p deer-vk --test spirv_val -- --list` 的输出为准） |
| **M2a-2** | **逻辑设备 + 队列**（`device.rs`，含线程生命周期解法） | ✅ 完成 | 真机打开设备、拿到图形队列、内存类型数正确；连续 3 次打开/关闭稳定 |
| **M2a-3** | 渲染通道 + 管线布局 + **图形管线** | ✅ **完成** | `vkCreateGraphicsPipelines` 成功且句柄非空（真机）。**推送常量矩形着色器已知不可用**（三种写法分别导致空句柄/访问违例），矩形绘制后续改顶点缓冲 —— 不阻塞后续步 |
| **M2a-4** | 命令池/命令缓冲 + 提交 + 栅栏同步 | ✅ 完成 | 栅栏在 ~1ms 内 signal（有限超时 1 秒）；提交后回读可用 |
| **M2a-5** | 离屏 `VkImage` + 渲染通道 + 回读像素 | ✅ 完成 | 四种清屏色的回读值**精确正确**；回读长度与确定性都对 |
| **M2a-6** | **绘制几何 + 像素判据**（SPIR-V 段序修复） | ✅ **完成** | 根因是**自研 SPIR-V 汇编器的段序错误**（`OpEntryPoint` 排在类型之后、`OpFunction` 掉进类型段）—— 驱动**既不报错也不画**，靠官方 `spirv-val` 定位。现 GPU 画出正确像素（面积 33% vs 理论 32%、位置正确、逐帧确定）|

#### M2b 的交付与验收

| 部分 | 交付 | 状态 | 验收判据 |
|---|---|---|---|
| **窗口层** | `crates/deer-window`：`WindowConfig` / `WindowInfo` / `App` / `Flow` / `run()`（**唯一引入 winit 的 crate**，见依赖例外登记） | ✅ 完成 | 真窗口可建（320×200）；`Resized` / `RedrawRequested` 到达；`DEER_WINDOW_HOLD=1` 能留窗观察；非 Windows 明确 `Err` |
| **Vulkan 上屏** | `crates/deer-vk`：`surface.rs` / `swapchain.rs` / `windowed.rs`（`VkSurfaceKHR` + 交换链 + 信号量/栅栏 + 呈现） | ✅ 完成 | `pick_config` 确定性（FIFO、**线性 `*_UNORM` 优先**、extent 夹取；M3c 起格式优先级改为线性，理由见 M3c 行）；`OutOfDate`/`Suboptimal` 正确映射；真机呈现 |
| **接入 + 示例** | `deer-gui` 的 `window` feature + `window_preview` 示例（`deer-vk` 的 HAL 接线同步完成） | ✅ 完成 | `cargo run -p deer-gui --features window --example window_preview` → 真窗口、自检全过、`exit=0` |
| **HAL 路径真实覆盖** | `crates/deer-gui/examples/hal_window_path.rs`：真窗口下走 `VkBackend::open(0)` → `create_swapchain` → `begin_frame/record/submit_and_present` → `wait_idle`，并断言 `record(含绘制命令)` ⇒ `Unsupported(M3)` | ✅ 完成 | `DEER_VK_WINDOW_TESTS=1 DEER_VK_VALIDATION=1 cargo run -q -p deer-gui --features window --example hal_window_path` → `exit=0`、640×480 / `Rgba8Unorm`（M3c 前是 `Bgra8Srgb`，因格式优先级已改为线性）、校验层零消息 |

**M2b 的完整门禁命令**（真窗口 e2e **默认跳过**，必须显式打开）：

```powershell
$env:DEER_VK_WINDOW_TESTS='1'; $env:DEER_VK_VALIDATION='1'; cargo test -p deer-vk
```

> **不设 `DEER_VK_WINDOW_TESTS` 时，真窗口 e2e 会显式跳过 —— 跳过也算 pass，所以别把它当证据**
> （验证者实测：把 `windowed.rs::resize` 改成空操作后，不设该变量时 25/25 全绿、设了才 1 failed）。
> HAL 路径的真人肉验收是上面的 `hal_window_path` 示例（`DEER_HAL_FRAMES` 可改帧数，默认 30）。

> **M2b 的边界**：它只保证「GPU 画的像素能出现在窗口上」—— 当窗口里是清屏色 + M2a 验证过的几何。
> 把 `DrawList` 送上 GPU 属于 **M3**：**M3a（形状）、M3b（文本）、M3c（呈到窗口）都已完成**，
> 三条都能与 CPU 逐像素对照；输入事件属于 **M5**。

### M3 的子步与状态

M3 原写成一条「渲染器 + 管线（矩形/圆角/裁剪/文本）」。实际按「**先几何、再文本、最后上屏**」拆：

| 子步 | 交付 | 状态 | 验收判据 / 说明 |
|---|---|---|---|
| **M3a-1** | **顶点流 + 着色器**：`gpu_geom.rs`（`DrawList` → `GpuVertex` 流，CPU 侧几何裁剪）、`spirv.rs` 的矩形属性 VS/FS | ✅ 完成 | 顶点布局 `#[repr(C)]` stride 44（`pos`/`rect`/`radius_kind`/`color`）；**静态** viewport/scissor（**当前实现事实**；当年「动态画不出像素」那条理由已**被本机实测推翻**，见下方前提 1）；着色器经 `spirv-val` |
| **M3a-2** | **离屏 GPU 几何渲染器**：`gpu_render.rs` 的 `GpuGeometryRenderer`（顶点缓冲 + 静态管线 + 回读） | ✅ 完成 | `cargo run -p deer-gui --example gpu_geometry` → `exit=0`；与 CPU **逐字节相同** |
| **M3a-3** | **与 CPU 逐像素对照**：`crates/deer-vk/tests/gpu_vs_cpu.rs` | ✅ 完成 | 不透明语料**最大通道差 0**（逐字节相同）；半透明语料**最大差 1 LSB**（实测仅 `alpha-clip` 非 0，**是实测上限不是证明上界**）；覆盖矩形/圆角（含超大半径）/描边（含带宽 > 边长）/裁剪/**嵌套裁剪**/退化 extent/清屏/连续多帧；`DEER_VK_VALIDATION=1` 下 parity **零校验消息**（**「层确实在跑」与「消息为零」都是可回归断言** —— 进程级计数 + parity 用例里的 `assert_no_validation_messages`，当前覆盖单帧/不透明/半透明/越界 alpha；但 VVL 不做通用同步验证 ⇒ 零消息**不能**证明内存域依赖正确） |
| **M3b-1** | **文本顶点流**：`gpu_text.rs`（`DrawCmd::Text` → `TextVertex` 四边形流，图集 `uv` + 屏幕 `pos` + 颜色） | ✅ 完成 | 顶点布局 `#[repr(C)]` **stride 32**（`pos`/`uv`/`color`）；空串 / `size <= 0` / 被裁空 / 图集放不下 ⇒ **跳过并计入 `skipped`，不报错**（修掉 M3a 的假阳性） |
| **M3b-2** | **R8 覆盖率纹理 + 最近邻采样 + 描述符集**（`device.rs`） | ✅ 完成 | 图集纹理 `R8_UNORM`、`mip_levels = 1`、**NEAREST + ClampToEdge**；采样着色器过官方 `spirv-val` |
| **M3b-3** | **第二条管线 + 单次遍历保 z 序**（`gpu_render.rs::with_text`） | ✅ 完成 | 独立顶点缓冲/管线/描述符集；文本与形状**交错**时按命令顺序绘制；图集只在**指纹变化**时重传（指纹 = `(图集宽, 图集高, 已光栅化字形数)`） |
| **M3b-4** | **文本与 CPU 逐像素对照**（`tests/gpu_vs_cpu.rs`） | ✅ 完成 | 不透明文本**逐字节相同（最大通道差 0）**、半透明文本 **≤ 1 LSB**；语料含单/多字符、`align=0/1/2`、超大 size、空串、零面积、被裁空、局部裁剪、缺字豆腐、z 序、连续 4 帧；**假阳性三类 ⇒ 跳过并计数**；图集零重传有护栏 |
| **M3b-5** | **文档 + 示例**（本指南 + `gpu_geometry` 示例扩成含文本） | ✅ 完成 | `cargo run -p deer-gui --example gpu_geometry` → `exit=0`（形状 + 文本仍与 CPU 逐字节相同） |
| **M3c** | **窗口里显示界面**（上屏路径：共用管线层 + 线性交换链 + 上屏 parity） | ✅ 完成 | `DEER_VK_WINDOW_TESTS=1 cargo run -q -p deer-gui --features window --example window_parity` → 不透明像素**逐字节相同（0）**、半透明 **≤1 LSB**；交换链改**线性 `*_UNORM`**（sRGB 附件的**混合在线性空间**，与 CPU 字节空间 `blend_cov` 实测差 **44 字节**：`src=0xC0,a=0.5,dst=0` ⇒ 96 vs 140）；resize×4 无泄漏（存活资源恒 1）；**动态与静态 viewport 都实测能上屏**（各 30 帧、93900 像素；见下方前提 1 的证据与边界） |
| **M3+** | **批处理优化** | ⬜ | 形状与文本**各自**每帧一个顶点缓冲、一次 draw；属性能项，不影响正确性 |

**M3 的不可回退前提**（细节见 [`docs/features/gpu-geometry.md`](docs/features/gpu-geometry.md) 第 3 节）：

1. **viewport/scissor：离屏用「静态」是当前实现事实；旧理由（「动态画不出像素」）已被实测推翻**。
   旧说法（`~~动态版在本机 Intel 核显上画不出任何像素~~`）**在本机 / 离屏 / 三角形场景下被三组对照推翻**（C1，2026-09-28）：
   - **①动态 + 每帧真的调** `vkCmdSetViewport`/`Scissor`：退出码 0、**1326/4096 像素（32.4%，几何自洽）**、
     与静态**逐字节相同**、校验消息 0 ⇒ **能画出像素**；
   - **②动态 + 从不设置**（M2a 形态）：**崩 `0xC0000005`**；
   - **③静态**（产品现状基线）：退出码 0、1326/4096。
   可重跑：`cargo run -p deer-vk --example viewport_dynamic_probe`（`--group=0|1|2` 单跑；`DEER_VK_VALIDATION=1` 加校验层）。
   - **⚠️ 边界（不许读成跨设备结论）**：这是**本机实测**，**不等于**「动态 viewport 现已支持」；换机器/驱动要重跑探针。
   - **⚠️ 症状不吻合**：M2a 记的是「**零像素且不崩溃**」，而 ② 是「**崩溃**」⇒ **不能断定当年成因与本次同源**；
     「**当年为什么零像素仍未解释**」。所以**不写**「已证实是误诊」。
   - **②的致命性是独立复现的硬要求**：声明动态却从不设置 ⇒ 离屏 `0xC0000005`、窗口 `0xC000041D`
     ⇒ **声明了动态状态，就必须在录制时真的设置它**。
   - **产品行为不变**：**离屏路径继续用静态**（C1 只推翻了旧结论的**理由**；没测「动态 + 尺寸变化 +
     多帧复用命令缓冲」等产品级场景，而静态已被既有判据覆盖）⇒ **是否改用动态是独立的产品决策**。
     窗口路径仍用动态（每帧真的设置；`DEER_VK_WINDOW_VIEWPORT=static|dynamic` 只是诊断开关，默认 `dynamic`）。
2. **颜色附件必须 `R8G8B8A8_UNORM`（不是 `_SRGB`）** —— CPU 基准不做 gamma，用 SRGB 会系统性偏差。
   **上屏交换链同理**（M3c）：sRGB 附件的**混合在线性空间**，与 CPU 字节空间 `blend_cov` 实测差
   **44 字节** ⇒ `pick_config` 改成**线性 `*_UNORM` 优先**（sRGB 只作退回）。
3. **「零校验消息」是可回归断言，但有明确覆盖边界** —— 「层确实在跑」由
   `validation_layer_state_matches_the_request` 钉住；「消息为零」由 `ffi::validation_message_count()`
   （进程级计数）配合 parity 用例里的 `assert_no_validation_messages`（单帧/不透明/半透明/越界 alpha/文本）钉住（不再靠人眼看 stderr）。
   **限制仍在**：计数只在 `DEER_VK_VALIDATION=1` 时有判别力，且 **VVL 不做通用同步验证** ⇒
   「零消息」**不能**证明内存域依赖正确（如 host→vertex 屏障；那条由
   `host_to_vertex_barrier_is_emitted_once_per_non_empty_frame` 单独守）。
4. **半透明 1 LSB 是实测上限而非证明上界**。
5. **文本必须用「最近邻 + ClampToEdge + `R8_UNORM` + `mip_levels = 1`」这一组**：
   CPU 是**整数查表**，线性过滤会把邻居纹素混进来；换成别的采样方式就不再逐像素等价。

### M4 的细步与状态

M4 原来写成一条「字体解析 + 光栅化 + 图集 + 度量 + 换行」。实际按「**先让字变成像素**、
再谈 GPU 侧」拆开做：前五步都已完成，且**全程零第三方依赖**（没有引 `ttf-parser`/`fontdue`；
M4 当时还没有窗口层的 `winit` 例外 —— 该例外是 M2b 引入的，见依赖纪律）。

| 步 | 交付 | 状态 | 说明 |
|---|---|---|---|
| **M4-1** | **零依赖 TrueType 解析**（`crates/deer-gpu/src/font.rs`） | ✅ 完成 | `head`/`hhea`/`hmtx`/`maxp`/`cmap`(0/4/6/12)/`loca`/`glyf`（简单 + 复合轮廓）；**CFF（`OTTO`）明确报错**，不静默给空轮廓 |
| **M4-2** | **字形光栅化**（`raster.rs` 的 `Rasterizer`） | ✅ 完成 | 轮廓自适应展平 → **nonzero** 扫描填充 → 超采样抗锯齿；确定性 |
| **M4-3** | **字形图集**（`atlas.rs` 的 `GlyphAtlas`） | ✅ 完成 | 货架打包 + 1px padding + **增高不搬动已有槽位** + 幂等 + 超限返回 `None` |
| **M4-4** | **真实度量与换行**（`measure.rs` 的 `FontMeasure`） | ✅ 完成 | 用 `hmtx` 真实 advance + `hhea` 升降部替换「每字符 0.6em」；`ApproxMeasure` 保留作确定性测试用 |
| **M4-5** | **CPU 后端真实字形**（`text.rs` + `null.rs` + 门面 + 示例） | ✅ 完成 | `TextEngine` 串起度量/光栅化/图集；`CpuRenderer::with_text` 贴真实字形，`CpuRenderer::new()` 旧占位行为不变 |
| **M4-6** | **hinting 与亚像素定位** | ⬜ | 现在用**超采样抗锯齿** + **整数像素落位**代替；不做 `glyph` instructions、不做 LCD 子像素 |

> **GPU 侧文本不在这张表里**：它属于 **M3b，且已完成**（第二条管线 + 图集纹理，与 CPU 逐像素对齐；
> 见上面「M3 的子步与状态」）。本里程碑交付的**字形图集**正是它的前置依赖。

## 依赖纪律（硬性）

1. **不引图形抽象库**：无 `wgpu`、`ash`、`vulkano`、`glow`。
2. **不引 GUI 框架**：无 `egui`、`iced`、`tauri`。
3. **最小生态依赖**：**目前 1 个登记在案的例外** —— `deer-window` 的 `winit`（窗口 / 事件循环，Q-1 已决，
   逐条登记见下面「依赖例外登记」）。其余仍然零第三方依赖：`deer-layout` / `deer-gpu` / `deer-vk`，
   以及**没开 `window` feature** 的 `deer-gui`。
   **「零依赖」这个说法此后一律写成「除窗口层（`winit`，已登记）外零第三方依赖」** —— 不要再写「完全零依赖」。
   后续新增例外必须逐条登记并说明理由；字体解析原先预计可能需要 `ttf-parser`，
   **实际未引**：M4-1 自研（见 `crates/deer-gpu/src/font.rs`）。

### 依赖例外登记（Q-1：窗口层引 `winit`）

**① 引了什么**：`winit = "0.30"`（`Cargo.lock` 解析为 **0.30.13**），只出现在
`crates/deer-window/Cargo.toml`。Windows 目标下
`cargo tree -p deer-window --target x86_64-pc-windows-msvc -e normal` 实际拉进的第三方包：

```text
deer-window v0.0.0
├── deer-gpu v0.0.0            （内部）
└── winit v0.30.13
    ├── bitflags v2.13.2
    ├── cursor-icon v1.2.0
    ├── dpi v0.1.2
    ├── raw-window-handle v0.6.2
    ├── smol_str v0.2.2
    ├── tracing v0.1.44
    │   ├── pin-project-lite v0.2.17
    │   └── tracing-core v0.1.36
    ├── unicode-segmentation v1.13.3
    └── windows-sys v0.52.0
        └── windows-targets v0.52.6
            └── windows_x86_64_msvc v0.52.6
```

（其它平台的依赖是 `cfg` 条件编译，不参与本目标构建。）

**② 为什么**（对比自写 Win32）：

- 自写 `CreateWindowExW` + 消息循环只是**开始**：还要自己做 DPI 感知、IME、多显示器、
  窗口生命周期，以及 **M5 要用的键盘/鼠标/滚轮事件翻译** —— 工作量远大于「窗口能出现在屏幕上」本身；
- `winit` 的事件模型与 **M5 的输入事件**天然对齐（`WindowEvent` 就是输入路由的入口）；
- 它**只负责窗口与事件，不碰渲染**：GPU 侧仍是自研的 `deer-vk`（自己声明 Vulkan 符号、自写 SPIR-V 汇编器）。

**③ 影响面**：只有 `deer-window`，以及**开了 `window` feature** 的 `deer-gui` 会拿到 `winit`。
`deer-layout` / `deer-gpu` / `deer-vk` 仍然零第三方依赖（可用上面的 `cargo tree` 复现）；
`deer-gui` 默认（不开 feature）也仍然零第三方依赖 —— 窗口层是可选的 `dep:deer-window`。

**④ 如何撤回**：HAL 只传**不透明的** `deer_gpu::RawWindowHandle`（`platform` + `handle` + `display`）。
换窗口实现（自写 Win32 / 换别的窗口库）只需重写 `deer-window`，
`deer-vk` 的 `Surface::create`、交换链与整个渲染层**一行都不用动**。

## 与 deer-ui 的关系

- **只取控件语义**（有哪些控件、什么行为、什么状态），**不取实现**（DOM/CSS/React 全部丢弃）。
- 布局代数来自 deer-ui 的 TypeScript 验证原型（V0）：那个原型当时跑了 28 条断言（**历史值**，
  不是本仓库当前的测试数），在这里以 Rust 测试重建，并保留了它抓到过的三个缺陷的回归守卫。
- `deer-ui` 本身不动，仍可作为 web 场景的组件库存在。

## 已知未决 / 遗留（登记，不阻塞）

| # | 项 | 说明 |
|---|---|---|
| Q-1 | ~~窗口抽象~~ | ✅ **已决（M2b）：引 `winit`**（本 workspace 唯一第三方依赖，只被 `deer-window` 与开了 `window` feature 的 `deer-gui` 拿到）。**①②③④ 逐条登记见上面「依赖例外登记（Q-1：窗口层引 `winit`）」**。 |
| Q-2 | ~~SPIR-V 来源~~ | ✅ **已解决（M2a-1）**：自写极简 SPIR-V 汇编器（`crates/deer-vk/src/spirv.rs`），**零外部依赖**，产物已被真机驱动接受（3 支着色器）。附带一条经验：`vkCreateShaderModule` **很宽容**（连 `bound=0` 都接受），所以「驱动接受」≠「SPIR-V 正确」，必须自己加结构护栏。 |
| Q-3 | ~~文本度量与布局的耦合~~ | ✅ **已落实（M4-4/M4-5）**：`FontMeasure` 通过 `Measure` 注入点接入布局；`ApproxMeasure` 保留为**确定性测试用**实现（不带字体的 `render_tree_to_png` 仍用它）。**新纪律**：布局、绘制列表、光栅化必须用同一个字号、同一个度量。 |
| Q-4 | 线程模型 | HAL 故意不实现 `Send`/`Sync`；M2 需要定「渲染线程 vs UI 线程」的边界。 |
| Q-5 | **推送常量矩形着色器损坏（遗留缺陷，M2b 登记）** | **现象**：`crates/deer-vk/src/spirv.rs::vertex_shader_rect_pushconstant` 产出的 SPIR-V 被校验层判 `VUID-StandaloneSpirv-PushConstant-06808`（源码里已标「已知不工作，不要用它建管线」）。**影响**：普通驱动会「宽容接受」（不带校验层的验收照跑）；**请求校验层时**该 SPIR-V 会让进程 **`0xc0000005` 访问违例崩溃** —— 所以从 **t15** 起，`tests/device_smoke.rs` 与 `tests/pipeline_smoke.rs` 里涉及它的测试在**请求了校验层时显式跳过**（stderr 写明「这不是通过，是被显式跳过」），于是 `DEER_VK_VALIDATION=1` 跑全量 deer-vk 是安全动作。**现状（task-18 后）**：设备/离屏路径曾经**故意不读**这个环境变量，是因为 offscreen 侧有 3 个真缺陷（barrier `sType` 写成 47、`oldLayout` 与渲染通道 `finalLayout` 不符、图像内存 `mem::forget` 泄漏）；**t18 已全部修掉**，所以现在 `VkBackend::new`、设备/离屏路径与窗口路径 `WindowedRenderer` **三条路一致尊重** `DEER_VK_VALIDATION`。本机实测：`DEER_VK_VALIDATION=1 cargo test -p deer-vk` → **全部通过 / 0 failed、零条校验消息**（**具体条数以运行输出为准**）；`DEER_VK_VALIDATION=1` 跑窗口示例 30 帧 → 零消息、`exit=0`。**修好它**归属 **M3**：矩形绘制改走**顶点缓冲**（与 M2a-3 记录的同一根因）。 |
