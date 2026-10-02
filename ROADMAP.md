# deer-gui 路线图

> 定位：**从零实现的 Rust GUI 运行时**。不依赖 web/DOM，不依赖 `wgpu`/`ash`/`vulkano`。
> 依赖边界：**连 GPU 后端也自己写**（Vulkan 先行，DX12/Metal 后续；抽象层保证「加后端 = 实现一个 trait」）。
> **唯一的第三方依赖例外**是窗口层的 `winit`（Q-1，M2b 引入），逐条登记见下面「依赖例外登记」。

## 里程碑

| # | 里程碑 | 内容 | 状态 |
|---|---|---|---|
| **M1** | **核心 + HAL + Vulkan 设备枚举** | 节点树、布局代数、命中测试、`.dui` 解析；GPU HAL + CPU 参考后端；Vulkan 实例与物理设备枚举（自己声明符号 + 动态加载） | ✅ **完成** |
| **M2** | Vulkan 逻辑设备 + 交换链 | M2a：逻辑设备/管线/命令/离屏回读；M2b：窗口 + `VkSurfaceKHR` + 交换链 + 帧同步 + 呈现 | ✅ **完成（M2a + M2b）** |
| **M3** | 渲染器 + 管线（矩形/圆角/裁剪/文本） | M3a：形状的 `DrawList` → GPU（顶点缓冲 + 静态管线 + 与 CPU 逐像素对照）；M3b：文本/字形 → GPU（第二条管线 + 图集纹理 + 逐像素对照）；M3c：把界面**呈到窗口**（共用管线层 + 线性交换链 + 上屏 parity）；M3+：批处理（**统一管线（形状 + 文本 → 一条管线）+ 跨帧复用缓冲已落地**） | 🔄 **进行中（M3a + M3b + M3c 完成；M3+ 批处理部分完成）** |
| **M4** | 文本 | 字体解析（TTF/OTF）+ 字形光栅化 + 图集 + 文本度量（替换 `ApproxMeasure`）+ 换行 | 🔄 **进行中（解析 / 光栅化 / 图集 / 度量与换行 / CPU 真实字形 ✅；hinting 与亚像素待做）** |
| **M5** | 输入 + 焦点 + dock | **已落地**：输入事件通路（`InputEvent` + winit 映射 + `App::input`）、命中与状态机（`hit`/`handle`/`ClipSnapshot`，含裁剪与禁用感知）、点击 / `Tab` / `Shift+Tab` / `Escape` 焦点、文本输入（追加 + `Backspace` 按 Unicode 字符删末尾）、脚本化事件重放；**M5b：事件驱动重绘（默认省电）** —— `ControlFlow::Wait` + `App::wants_redraw()` + `RedrawPolicy`（默认 `OnDemand`），可用 `DEER_WINDOW_REDRAW=continuous` 关掉省电（见 [`docs/features/window.md`](docs/features/window.md) 第 6 节）。**仍未做**：**dock**、多窗口、方向键导航、滚动条 / 惯性滚动、右键/中键、IME 预编辑、按键重复、`texts` 光标位置（**滚轮驱动的垂直滚动已落地**，见 [`docs/features/scroll-and-multiline.md`](docs/features/scroll-and-multiline.md)） | 🔄 **部分完成（见 [`docs/features/input.md`](docs/features/input.md) 第 6 节）** |
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
| **HAL 路径真实覆盖** | `crates/deer-gui/examples/hal_window_path.rs`：真窗口下走 `VkBackend::open(0)` → `create_swapchain` → `begin_frame/record/submit_and_present` → `wait_idle`，并断言 `record` 边界：空列表 ✅ / 含形状的列表 ✅ / **含文本却缺 `TextEngine` ⇒ `Unsupported`**（**T1.1 起**；M2b 时代是「含绘制命令 ⇒ `Unsupported(M3)`」） | ✅ 完成 | `DEER_VK_WINDOW_TESTS=1 DEER_VK_VALIDATION=1 cargo run -q -p deer-gui --features window --example hal_window_path` → `exit=0`、640×480 / `Rgba8Unorm`（M3c 前是 `Bgra8Srgb`，因格式优先级已改为线性）、校验层零消息 |

**M2b 的完整门禁命令**（真窗口 e2e **默认跳过**，必须显式打开）：

```powershell
$env:DEER_VK_WINDOW_TESTS='1'; $env:DEER_VK_VALIDATION='1'; cargo test -p deer-vk
```

> **不设 `DEER_VK_WINDOW_TESTS` 时，真窗口 e2e 会显式跳过 —— 跳过也算 pass，所以别把它当证据**
> （验证者实测：把 `windowed.rs::resize` 改成空操作后，不设该变量时 25/25 全绿、设了才 1 failed）。
> HAL 路径的真人肉验收是上面的 `hal_window_path` 示例（`DEER_HAL_FRAMES` 可改帧数，默认 30）。

> **M2b 的边界**：它只保证「GPU 画的像素能出现在窗口上」—— 当窗口里是清屏色 + M2a 验证过的几何。
> 把 `DrawList` 送上 GPU 属于 **M3**：**M3a（形状）、M3b（文本）、M3c（呈到窗口）都已完成**，
> 三条都能与 CPU 逐像素对照；**输入与焦点也已落地**（M5，见 [`docs/features/input.md`](docs/features/input.md)）。

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
| **M3+** | **批处理优化** | 🔄 **部分完成** | **已落地**：① **统一管线**（形状 + 文本合成**一条**顶点流 + **一条**管线 ⇒ 每帧一次 draw、一次管线切换；改造前是每段一次）；② **跨帧复用缓冲**（按容量增长的持久缓冲，语料不变时**不再每帧重建/重传**）。可用 `RenderStats` 复现：`DEER_VK_WINDOW_TESTS=1 cargo run -q -p deer-gui --features window --example window_parity`（打印 draw call / 管线切换 / 缓冲上传 / 缓冲分配，以及 CPU 侧 `unify` 调用次数与顶点数；**具体数字以运行输出为准**）。**仍未做**：**多批次提交** —— 登记理由：统一管线已经是 **1 bind / 1 draw / 1 submit 每帧** ⇒ **没有可合并的批次**（「多批」的前提是**多张纹理 / 多个渲染目标**，属 bindless / 多 pass 范畴），以及把「每帧一次 `unify` 会新建一个 `Vec`」的堆分配消掉。**已落地并单独登记**：**间接绘制**（`vkCmdBindIndexBuffer` + `vkCmdDrawIndexedIndirect(drawCount = 1, stride = 20)`，离屏 + 窗口两条路径）与**通用纹理**（`RGBA8_UNORM` 创建 / 上传 / **回读四通道保真**）—— 两者**尚无指南与示例**，故在 [`FEATURES.md`](FEATURES.md) 第三节记为 🔄、**不标 ✅**（该文件的规矩：✅ 必须指南 + 示例）。**像素判据不变**（不透明逐字节 0 / 半透明 ≤1 LSB）。<br>**✅ 原待办已补（覆盖缺口，非正确性缺陷）**：**窗口侧 host→vertex 屏障**原「只有计数、没有断言」⇒ 已按 (a) 方案在 `windowed_chain_end_to_end` 里补上三条断言（**首帧上传 ⇒ +1** / **同语料第二帧 ⇒ 不增** / **空帧 ⇒ 不增**）+ **前置断言**（首帧 `buffer_uploads` 必须增长）+ **双向变异**（只删计数自增 ⇒ 红；删整个发射块 ⇒ 红，均已实测）。**关键：断言必须挂在 `draw_and_present`（→ `record_ui`）上** —— `render_and_present` 走 M2b 三角形路径、不碰 UI 顶点缓冲，在那条链上读计数恒为 0（永远绿的死判据）。**三类屏障（顶点 / 索引 / 间接）现已全部有断言**：索引 / 间接两条也已各加 `u64` 计数
（`ui_index_barrier_count()` / `ui_indirect_barrier_count()`），与顶点那条**同一套三条判据**
（首帧各 +1 / 同语料不增 / 空帧不增），实测 `(0,0,0) → (1,1,1) → (1,1,1) → (1,1,1)`；
**双向变异均已实测变红**（只删 index 自增 ⇒ `(1,0,1)` 红 / 只删 indirect 自增 ⇒ `(1,1,0)` 红 /
删掉 index 整个发射块 ⇒ 红）。**仍未做**：「每进程只支持一个窗口」的测试基建限制（方案 (b)）未修。详见 [`docs/features/window.md`](docs/features/window.md) 第 7 节的边界条目。 |

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

### M3 之后的 HAL 补课：`Frame::record` 接线（T1.1）

M3 把界面**画上了窗口**（M3c），但那是走**专用快路** `WindowedRenderer::draw_and_present`；
**HAL 的 `Frame::record` 仍然直连分叉**——M2b 时代它只接受 `NodeHint`，任何真实绘制命令一律
`Unsupported(M3)`（理由是「不让『窗口里什么都没有』变成查不出的 bug」）。M3a/M3b/M3c/M3+ 全部落地后，
**那条分支的理由消失了**，T1.1 把它接线到同一条 UI 录制链。

#### 设计登记（改 HAL 公开 trait ⇒ 按 `AGENTS.md` §6「改公开 API 先问」+ `DEV-PLAN.md` §3 三步走）

**① 要改的公开 API**：`deer_gpu::Frame::record`
- 改前：`fn record(&mut self, list: &DrawList) -> GpuResult<()>`
- 改后：`fn record(&mut self, list: &DrawList, text: Option<&mut TextEngine>) -> GpuResult<()>`

**② 核心决策：`TextEngine` 怎么过 HAL？** 三个候选，**选了 ③**：

| # | 方案 | 为什么否决 / 采纳 |
|---|---|---|
| ① | `Device` 挂引擎（`device.set_text_engine(engine)`，`record` 不带参） | **否决**：HAL 的 `Device` **刻意无跨调用状态**（`create_swapchain`/`create_texture`/`begin_frame` 都不留状态）——「挂一个引擎」直接破坏这条不变式，且「引擎属于哪一层」会被 HAL 擅自决定 |
| ② | 由后端**自己建**引擎（`record` 里 new 一个） | **否决**：字体文件由**上层**（`deer-gui` 门面）选并提供，后端（`deer-vk`）**不该**知道字体从哪来；这会把「字体是应用资产」这个事实藏进后端 |
| ③ | **`record` 加参数 `Option<&mut TextEngine>`（逐帧传入）** | **采纳**：与**已存在**的 `WindowedRenderer::draw_and_present(list, text: Option<&mut TextEngine>)` **签名同形** —— 是既有先例，不是新发明的模式；HAL 保持无状态 |

**③ 与 Godot 的对照（记录，不采纳）**：Godot 用 `TextServer` **全局单例**（按 `RID` 寻址字体/图集），
形态接近候选 ①。**不照做**的理由：那需要一层「专门的服务层」+ 生命周期管理，属**架构级变更**，
超出 T1.1 的范围（`UPDATE-PLAN.md` 只要求「接线」，不是「重设架构」）。记在这里备查。

**④ 错误语义（两端一致，写进 `Frame::record` 文档）**：
- 无文本命令 ⇒ `text = None` **合法**（形状列表照常上屏）；
- **有**文本命令但 `text = None` ⇒ `GpuError::Unsupported`（**绝不静默丢弃**——那正是「形状都在、文字全没」的隐形 bug）；
- 文本假阳性（空串 / `size <= 0` / 被裁空 / 图集放不下）⇒ **跳过并计数**，**不报错**（沿用 M3b-1 的既有语义）。

**⑤ 不改的东西（T1.1 的边界）**：
- **不改 `read_pixels`**：HAL 的 `Frame::read_pixels` **仍** `Unsupported`（交换链图像回读只在呈现后有效）—— 属 **T1.4** 的语义决策，不在本步。
- **不改像素判据**：`window_parity` 的不透明逐字节 / 半透明 ≤1 LSB **必须不变**（`record` 与 `draw_and_present` 走**同一段实现** ⇒ 不应有像素差异）。

#### 实现登记（落点）

- `crates/deer-vk/src/windowed.rs`：把 `draw_and_present` 拆成
  `prepare_ui(list, text) -> Vec<UnifiedVertex>`（步骤 ①–⑤：建流 → `unify` → 上传顶点/索引/间接命令 → 刷新图集纹理）
  + `present_prepared(&[UnifiedVertex]) -> FrameOutcome`（步骤 ⑥：帧舞蹈）；
  `draw_and_present` 退化成两者的**顺序调用** ⇒ **两条路径共用同一段实现**。
- `crates/deer-vk/src/hal.rs`：`VulkanFrame` 加 `pending_ui: Option<Vec<UnifiedVertex>>`，
  `record` 里 `prepare_ui` 后**暂存在帧对象上**（**不是**共享的 `chain` —— 否则两次 `begin_frame` 会互相覆盖），
  `submit_and_present` 再取出来 `present_prepared`。
- **实现中发现的真实缺陷（已修 + 已锁）**：`submit_and_present` 改走 UI 路径后，
  `record_ui` 仍 `expect`「界面资源已建」（`self.ui` 非空），而**没调过 `record` 的空帧**
  从未经过 `prepare_ui`（`ensure_ui` 在那里才调）⇒ `self.ui == None` ⇒ **panic**。
  修法：`WindowedRenderer::present_prepared` 开头补一次 `ensure_ui(false)?`。
  回归锁：示例 `hal_window_path` 把「空帧」挪到**任何 `record` 之前**（此时 `ui` 必为 `None`），
  **双向变异已实测**（去掉 `ensure_ui(false)?` ⇒ `windowed.rs:1820` panic、exit 101 红）。
- `crates/deer-gpu/src/null.rs`：`CpuFrame::record` 加同一条文本契约（**存在性校验**；CPU 帧的契约仍是「先收命令、提交时出图」）。

#### 验收（四口径 + 双向变异）

- ① `cargo test --workspace`、② `DEER_VK_WINDOW_TESTS=1 DEER_VK_VALIDATION=1 cargo test -p deer-vk`、
  ③ `cargo test -p deer-gui --test docs_consistency`、④ `cargo clippy --workspace --all-targets` —— 全绿。
- 端到端：`DEER_VK_WINDOW_TESTS=1 DEER_VK_VALIDATION=1 DEER_HAL_FRAMES=3 cargo run -q -p deer-gui --features window --example hal_window_path`
  ⇒ 真窗口、校验层零消息、`[hal] record 边界 : 空=OK / 形状=OK / 文本缺引擎=Unsupported(TextEngine) ✅`。
- **新增断言（均过双向变异）**：
  - `cpu_backend_reports_text_without_engine_instead_of_dropping_it`（`deer-gpu` 集成测试）
    —— 变异 A：禁掉 `has_text && text.is_none()` ⇒ 红；变异 B：把 `Unsupported` 换成 `Driver` ⇒ 红。
  - `record_reports_a_missing_engine_instead_of_silently_dropping_text`（`deer-vk` 单测，**改写**）
    —— 变异 C：把空链错误信息里的「交换链」抹掉 ⇒ 红。
  - 示例 `hal_window_path` 的「空帧」用例（见上方实现登记）
    —— 变异 D：去掉 `present_prepared` 里的 `ensure_ui(false)?` ⇒ panic 红。

> **T1.1 只做「接线」，不做架构**：`TextEngine` 逐帧传入（与既有 `draw_and_present` 同形），
> HAL 无状态不变，像素判据不变。**Godot 式全局 `TextServer` 单例**已评估但**不采纳**（架构级，超范围）。

### 设计登记：T1.3「第三态」不是加一位那么简单（2026-10-01）

**① 原始设想**（来自 `DEV-PLAN.md` §3 与任务书任务 A）：统一 FS 现在两态
（`uv.x < 0` ⇒ 形状 / `uv.x >= 0` ⇒ 文本）。贴 RGBA 纹理的 quad 也要 `uv >= 0`
才能采样 ⇒ **与文本撞车**，所以要「第三态或独立管线」。

**② 读代码后新增的两条硬约束（这条发现改变了候选集）**

| # | 约束 | 出处 |
|---|---|---|
| C1 | **`uv` 背不动第三态** —— 纹理 quad 必须携带**真实**的 `[0,1]` 纹理坐标去采样，把它改成负值哨兵就等于毁掉它自己要用的值 | 逻辑推论 + `spirv.rs:2329-2335` 的接口表（`uv` 对文本段是「归一化图集坐标 `[0,1]`」） |
| C2 | **不能用 `radius_kind` 当主判别符** —— 它的**负值已被「描边带宽」占用**（Ruling 6） | `spirv.rs:2345` 明文裁决 |
| C3 | 统一着色器必须**单基本块、零 `OpPhi`**（判别一律用 `OpSelect`，不用分支） | `spirv.rs:2348-2353`；由 `unified_shaders_have_no_control_flow` 钉住 |
| C4 | 描述符集目前**只有一个 binding**（`binding 0 = COMBINED_IMAGE_SAMPLER`，字形图集 / 形状帧的 1×1 哑元纹理） | `pipelines.rs:360`；`a_shape_only_frame_binds_the_one_by_one_dummy_texture` 盯着它 |

**③ 候选与取舍**（`⚑` = 本轮推荐）

| # | 方案 | 代价 | 判据影响 |
|---|---|---|---|
| ① | **`uv` 再切一个负区间**（如 `uv.x <= -2` ⇒ 纹理） | 违反 C1 —— 纹理 quad 拿不到真实 uv，**直接出局** | — |
| ② | **借未被使用的属性做二级判别**：在 `uv >= 0` 那一支里再用一个记号分「覆盖率 vs RGBA」。候选是文本段**未使用**的 `rect`（vec4）或 `radius_kind`（float） | 不动 `UnifiedVertex` **布局**（stride 52 冻结）；但要在 FS 里多一层 `OpSelect`（C3 允许）；代价是要把「这两个属性对非形状段不再是『未使用』」写成正式契约（现在那两栏写的是「未使用（填 0）」） | 需 **第二个 binding**（C4）⇒ 描述符布局从 1 binding 变 2 binding，会牵动 `a_shape_only_frame_binds_the_one_by_one_dummy_texture` 与 `update_descriptor_texture` 的 VUID 断言 |
| ③ | **纹理走独立管线**（形状+文本统一管线之外） | 破坏「每帧 **1 bind / 1 draw / 1 submit**」这个已登记的事实（`FEATURES.md` 第四节「批处理优化」行），`RenderStats` 三项恒为 1 的判据要重写 | 最大 |
| ⚑ | **先补 ② 的前半（二级判别 + 第二 binding），窗口侧入口随后** | 比 ③ 小得多，但仍然要动描述符布局 ⇒ 建议单独一轮，与本轮的其它交付分开 | 见下 |

**④ 本轮结论**：**不实现** —— T1.3 会同时改 `spirv.rs`（2 支着色器）、`pipelines.rs`（描述符布局）
、`vertex_unify.rs`（段类型/判别符契约）、`windowed.rs`（入口）、`null.rs`（CPU 参考）与三份文档，
跨 5+ 文件、要**手写** SPIR-V 并过 `spirv-val`；按 `AGENTS.md` §7 的 P2 约束「控制单批改动范围」，
它**必须自己一轮**。本轮只把上面四条约束登记下来 —— 尤其是 **C1**，它让任务书里
「第三态」的说法不再成立（第三态不能在 `uv` 上），真正可选的是 ② 与 ③。

**⑤ 待确认**：② 里借 `rect` 还是 `radius_kind`（倾向 `rect`：它是 vec4，可以只约定**一个分量**当记号，
且与已明确禁止借用负值的 `radius_kind` 划清界限）。

### 设计登记：`texts` 光标建模（T3.5，2026-10-01）

**① 要动的公开面**：`deer_gui::UiState` 加字段 ⇒ 属公开 API 变更（`AGENTS.md` §6 要求先登记）。
现有 27 处 `UiState { … }` 字面量构造（分布在 6 个文件）会随之编译不过 ⇒ 需要一个专为此展开的提交。

**② 三个候选**

| # | 方案 | 取舍 |
|---|---|---|
| ① | 新增字段 `carets: BTreeMap<String, usize>`（id → **字符位**，不是字节位） | **采纳**：**纯追加**，读 `texts` 的既有代码一行不改；不在表里 = 默认「光标在末尾」⇒ **既有行为逐字节不变**（与 `Backspace` 已按 Unicode 字符删的先例一致） |
| ② | 把 `texts` 换成 `BTreeMap<String, TextEditState>`（内含 value + caret） | **否决**：破坏性变更，所有读写 `texts` 的地方都要改；「文本值」这个最常被读的东西被包了一层 |
| ③ | 光标存在窗口层 / 调用方 | **否决**：滚轮住的同一条理由 —— 输入必须在**唯一入口** `handle` 消费，否则另一条路径上的方向键会静默无效（`ScrollState` 的登记原话） |

**③ dirty 语义（任务书 §七 待确认点 2）**：**光标移动不发 `UiEvent`**。
理由：`TextChanged` 的语义是「文本值变了」，光标没改值；而现在**还没有任何东西渲染光标**
⇒ 现在也谈不上「要重绘一个看不见的东西」。**登记为延迟决策**：一旦真的开始画光标，
就照 `ScrollState` 的同样做法补一个 `UiEvent::CaretChanged`，届时一并定重绘。

### 设计登记：App 地基任务书（2026-10-02）的已确认决定

任务书本体在 [`docs/superpowers/plans/2026-10-02-app-foundation.md`](docs/superpowers/plans/2026-10-02-app-foundation.md)
（P1 策划：A 输入语义 / B 图像 / C App 运行时 / D 窗口模型 / L 布局表达力 / E 编辑器地基 六线）。
按 `AGENTS.md` §7.1 第 4 条，**公开 API 变更与选型须先登记并获人确认** —— 本节就是那份登记，
**决定人已裁断**，实现按此走。

#### 已确认（可直接开工）

| # | 项 | 决定 | 对实现的影响 |
|---|---|---|---|
| **D7** | 指针捕获（T3.7） | **默认捕获** —— 按下即捕获，拖出节点后事件仍路由给它，直至抬起 | ⚠️ **会改掉既有「拖出即丢」行为** ⇒ 实现时必须**先实测现状并登记差异**，再改；未捕获路径的既有判据要逐条核对 |
| **D1** | `InputEvent::ImePreedit { text }`（T3.4） | **各自独立 PR**（不与 D3 合并成批） | `deer-window` + `interaction.rs` 的 mirror **两处逐字同步**，一次只动一个变体 |
| **D3** | `KeyDown::repeat: bool`（T3.6）与 `InputEvent::ScaleFactorChanged`（AF-3） | 同上：**各自独立 PR** | 同上；既有 `match` 的穷尽性由编译期兜住 |
| **Q1** | 右键语义（T3.3） | **先纯透传**（`UiEvent::PointerRight` 级别），上下文菜单属 M6 | 地基只保证事件能到上层 |
| **Q2** | PNG 解码（AF-1） | **先只做 BMP**（零依赖）；PNG 解码单独立项按需决策 | 自写 inflate 的收益要等真实需求 |
| **Q3** | 剪贴板（AF-2） | **自写 Win32**（`CF_UNICODETEXT`，约 60 行），不引 `arboard` | 与「除 `winit` 外零第三方依赖」的口径一致 |
| **Q4** | DPI（AF-3） | **只透传 `scale_factor`，不下沉进布局** | 布局是像素级确定性纯函数（I-1/I-2），自动缩放会破坏逐字节判据 |
| **Q6** | `.dui` 未知属性（E2） | **警告 + 结构化保留**（编辑器往返不能默默吃掉用户文件里的未来字段） | 与 `scene.rs` 既有「写错即报错」哲学有张力，故此处**明确登记为例外**：**未知**属性保留，**已知属性写错**仍报错 |
| **Q7** | 撤销栈上限（E3） | **固定条数上限**（如 100 步），不做命令式反转 | 小树快照便宜 |
| **D2** | 方向键上下焦点序（T3.1） | **几何邻近**（不复用 `Tab` 的构建序） | `Tab` 是构建序、方向键是空间语义，两者本就不同 |

#### D6：`LayoutProps` 扩展批（L1–L4）—— **已裁断：一个机制**（2026-10-02）

| # | 项 | 决定 |
|---|---|---|
| **Q5** | anchors（L4）与 position（L1）是**一个机制还是两个** | **一个** —— `anchors` 是 `position` 的参数化形态：`Pos::Anchors { l, t, r, b, offset }` 一个变体同时覆盖「绝对偏移」（四锚点取同一点时退化成绝对定位）。**理由不是「代码少」，是消歧义**：两个机制必然造出「`pos` 和 `anchor` 同时设了谁赢」这个无解问题，而它**不报错、只是结果不对** —— 最难查的一类。Godot 也是单一锚点体系。编辑器的 `E6` 拖拽因此有唯一答案：**拖动永远改 offset，锚点由显式 UI 改**（不做「模式切换」） |
| **D6** | `LayoutProps` 的字段扩展 | **一批登记、按 L1→L2→L3→L4 分 PR 实现**（沿用 T1.1 三步走的先例）。**L1 实现时就要把 `Pos` 定义成含 `Anchors` 变体的形态**（哪怕 L4 才用），否则后面统一要改一次公开契约 |

**未定但已记下判断的两点**（不阻塞开工，到对应 PR 时定）：
- **`width`/`height` 与锚点的优先级**：`l=0, r=1` 撑满时宽度由锚点决定 ⇒ 此时 `width` 应被忽略。
  **同一个尺寸有两种设法**必须登记成明确规则（否则又是「两个真值」）—— 在 L1 的 PR 里定死；
- **参照矩形**（父的 padding box 还是 content box）：合并成一个机制后这是**全局**决定，
  在 L1 的 PR 里定死并写测试。

> **D6 为什么必须一批登记**：`LayoutProps` 是**三契约的公共部分**（数据模型 / `.dui` 语法 /
> 两路结构相等断言）⇒ 加**任何**一个字段都要同步改 `node.rs` + `scene.rs` + `builder.rs` 的 `L`
> 三处并补 `.dui` 语法测试。
>
> **Q5 的完整分析**（为什么合并、代价是什么、四个优点与四个缺点）见
> [`docs/superpowers/plans/2026-10-02-app-foundation.md`](docs/superpowers/plans/2026-10-02-app-foundation.md)
> 的评审记录；本节只留**决定与理由摘要**。

#### D8：`NodeProps` 增 `extra` —— 未知属性的「警告 + 结构化保留」（E2，Q6 已裁断）

**决定（人已裁断，2026-10-02）**：`.dui` 载入遇到**当前版本不认识的属性**时，
**警告 + 结构化保留**，不是丢弃、也不是硬报错。理由：编辑器往返**不能默默吃掉**用户文件
里的未来字段 —— 用户在新版里写的东西，被旧版存一次就没了，那是数据丢失。

**为什么这必须登记**：`NodeProps` 与 `LayoutProps` 同属**三契约的公共部分**
（数据模型 / `.dui` 语法 / 两路结构相等断言）⇒ 加字段要同步改多处。

##### 设计细节（都是必须定死、否则会漂的点）

| # | 决定 | 理由 |
|---|---|---|
| 1 | 新字段 `NodeProps.extra: BTreeMap<String, Option<String>>` | `BTreeMap` 而非 `HashMap`：**顺序确定** ⇒ 往返逐字节稳定（round-trip² 的前提）。值是 `Option<String>` 而不是 `String`，因为 `AttrVal` 有两种：`Str(s)` 与**裸属性 `Bare`** —— `foo` 与 `foo=""` **不是同一件事**，合成一个类型就再也分不出来，往返会改文件内容 |
| 2 | 已知属性写错**仍然硬报错**；只有**未知**属性进 `extra` | 与 `scene-file.md` 既有的「写错即报错」哲学并存。这是 Q6 明确登记的**例外**，不是放松 |
| 3 | encode 时 `extra` 写在**已知属性之后**，顺序 = `BTreeMap` 的键序 | 顺序确定 ⇒ 同一棵树编码结果逐字节相同 |
| 4 | `structurally_eq` **自动**把 `extra` 纳入比较 | 两棵只有 `extra` 不同的树，**本来就不是**结构相等的树。不改判定逻辑，靠 `NodeProps` 的派生相等自动覆盖 |
| 5 | E1 属性注册表**必须加一条**（新 `PropType::Opaque`） | E1 的**穷尽解构**防漂移测试会**编译失败**，逼着登记 —— 这正是那条护栏的用途。`Opaque` 的语义是「往返保真的载体，**编辑器不该直接编辑它**」 |
| 6 | 新 API：`parse_scene_collect(src, source) -> Result<(Node, Vec<String>), SceneError>` | 警告要有地方给。`parse_scene` **签名不变**、内部调用新函数并丢掉警告 ⇒ 既有调用点零改动 |
| 7 | `.dui` 既有语料**逐字节不变** | `extra` 默认空 ⇒ 编码不多写任何东西 |

**影响面（实现时逐一核对）**：`node.rs`（字段）/ `scene.rs`（解析 + 编码 + 警告）/
`builder.rs::props`（构造要带上）/ `registry.rs`（加 `PropType::Opaque` 一条）/
`scene-file.md`（语法表要说明「未知属性会被保留」）。

#### D10：`Pos` 的**形状**（L1 的公开契约，先定死再实现）

D6 已经裁断「anchors 与 position **一个机制**」，并留了一条硬约束：
**L1 就要把 `Pos` 定义成含 `Anchors` 变体的形态**（哪怕 L4 才用），否则后面从单变体改成分体
等于**再动一次公开契约**。本节把形状与两条遗留决定定死。

```rust
/// 节点在**流之外**的定位方式（`LayoutProps::position`，默认 `None` = 参与正常流布局）。
pub enum Pos {
    /// 相对**参照矩形**左上角的偏移（`px` 或 `pct`）—— 90% 的用法，也是 `Anchors` 的糖。
    At { x: Size, y: Size },
    /// 四边锚定：`l/t/r/b` 是参照矩形的**比例**（0.0..=1.0），`offset` 是**像素**修正。
    /// `l=0, r=1` ⇒ 宽度由锚点撑满（此时 `w=` 被忽略，见下）。
    Anchors { l: f32, t: f32, r: f32, b: f32, offset: [Size; 4] },
}
```

**两条遗留决定（D6 说「到 L1 的 PR 必须定死」，这里定死）**

| # | 决定 | 依据（**不是**口味，是本仓既有语义） |
|---|---|---|
| 1 | **参照矩形 = 父的内容盒**（即去掉 padding 之后那块） | 布局里百分比解析的基准（`avail`）本来就是**父的内容盒**。参照矩形若取别的，`w=50%` 与「锚在 0.5」就会差一个 padding 的宽度 —— 同一件事两种结果，正是要避免的 |
| 2 | **锚点撑满时 `w=`/`h=` 被忽略**（不是报错） | 与 Godot 一致，也避免「同一个尺寸有两种设法」变成**静默胜出**。忽略是**明确规则**，不是「谁后写谁赢」 |

**一条实现纪律（写进登记以免漂）**：`position: None` ⇒ **与现在逐字节相同**（opt-in，
沿用 `scroll`/`wrap` 的先例）；层叠序 = **声明序**（后画的在上），命中测试**先层叠后流内** ——
层序是**绘制/命中的顺序**，与 `Pos` 的取值无关，不塞进 `Pos`。

**影响面（实现 L1 时逐一核对）**：`node.rs`（`Pos` + `LayoutProps::position`）/
`layout.rs`（脱离流 + 定位） / `render.rs`+`interact.rs`（层序） / `hit_test`（层叠优先） /
`scene.rs`（`.dui` 语法 + 版本头是否要升到 v2 —— 见下） / `builder.rs::L` /
`registry.rs`（加一条 `PropType::Pos`）。

> **`.dui` 版本头要不要升 v2？** 本项是**纯新增属性**：老文件（无该属性）行为不变 ⇒
> **不需要升版本**（升了反而让老库拒绝读新文件，而新文件其实老库读得动 —— 只是忽略那个属性）。
> 记在这里，免得实现时顺手把版本号加了。

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
| **M4-6** | **hinting 与亚像素定位** | 🔄 **上半已落地** | **亚像素水平定位**：**已落地为 opt-in 路径**（`Rasterizer::rasterize_at` / `rasterize_char_at` + `split_subpixel_x`，1/4 相位档；实测落位误差 RMSE **0.2890→0.0733 px、3.94×**）—— 代价是「**间距精度换边缘锐度**」（部分覆盖质量占比上升，`l` **+64.3%**），且**尚未接进** `TextEngine`（默认路径**逐字节不变**）。**hinting 仍不做，但这次有实测依据**：最省的 hinting-lite 已实现并量过（最大形变 **12.5%**、质量区间 **[-10.3%, +12.8%]**、均值 **+0.4%**）⇒ **没有净收益**；不做 `glyph` instructions、不做 LCD 子像素（RGB 三通道） |

> **GPU 侧文本不在这张表里**：它属于 **M3b，且已完成**（第二条管线 + 图集纹理，与 CPU 逐像素对齐；
> 见上面「M3 的子步与状态」）。本里程碑交付的**字形图集**正是它的前置依赖。

### M4-6 决策登记：亚像素定位**不接入** `TextEngine`（T0.3，2026-09-30）

**决策**：① 光栅化层保留 opt-in 路径（已落地、逐字节中性）；② **不**把它接进
`TextEngine` / `CpuRenderer` —— 与 hinting 同样按「**实测否定**」登记，不是「忘了做」。

**实测依据**（真机 Windows / Intel RaptorLake-S，`cargo test -p deer-gpu --test text_raster -- --nocapture`）：

| 指标 | 整数落位（现状） | 1/4 亚像素落位 | 结论 |
|---|---|---|---|
| 落位 RMSE | 0.2890 px | **0.0733 px** | 改善 **3.94×** —— 真的更准 |
| 落位最坏误差 | 0.5000 px | 0.1250 px | 上界从 1/2 收到 1/8 px |
| 相邻间距最坏误差 | 0.5938 px | 0.1562 px | 间距节奏确实更稳 |
| 部分覆盖质量（`l`） | 0.255 | **0.420（+64.3%）** | **代价**：边缘明显更灰 |
| 部分覆盖质量（`H`） | 0.308 | 0.446（+45.0%） | 同上 |
| 墨迹总量漂移 | — | ≤0.09% | 守恒（不是「变糊」而是「摊开」） |

**为什么仍然不接**（理由是**取舍**，不是「做不了」）：

1. **没有净视觉收益**：落位更准换来的代价是**边缘锐度下降**（部分覆盖质量最高 +64.3%）——
   小字号下「更灰」比「间距差 1/4 px」更显眼。这与 hinting-lite 的结论同型（形变 12.5% 换来
   质量均值仅 +0.4% ⇒ 没净收益）。
2. **代价明确且不可忽略**：接入需把 `GlyphKey` 加相位档、`GlyphPlacement` 暴露相位、
   `draw_text_real` 改用 `whole + left` ⇒ **图集记录 ×4**（`SUBPIXEL_LEVELS = 4`）。
3. **违背当前最高优先级的判据**：默认路径**逐字节不变**是全仓像素 parity 判据的基石
   （`gpu_vs_cpu` / `window_parity` 不透明逐字节）。接入会改变默认落位 ⇒
   所有像素判据的期望值都要重算，而收益是「见仁见智」的观感。

**保留的 opt-in 路径**（不改）：`Rasterizer::rasterize_at` / `rasterize_char_at` +
`split_subpixel_x`（`SUBPIXEL_LEVELS = 4`）**仍然可用** —— 谁要做字号自适应或诊断实验，
直接调它们即可，`subpixel_x == 0.0` 与 `rasterize` **逐字节相同**（黄金指纹测试钉住）。

**重新评估的触发条件**（写清楚，避免这条决策变成不可推翻的教条）：
若将来引入 **LCD 子像素渲染**（RGB 三通道，需要亚像素相位）或**可变字体 / 高 DPI 缩放**
（同一个字号要按 0.5px 步进落位），应重新评估 —— 那时亚像素相位是**必要前置**而非观感取舍。

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
