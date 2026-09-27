# 功能指南：GPU 几何渲染（gpu-geometry）

> 状态 ✅（**仅离屏**；形状 + 文本）· 示例 `cargo run -p deer-gui --example gpu_geometry` ·
> 清单条目见 [`FEATURES.md`](../../FEATURES.md)

## 1. 这是什么 / 什么时候用它

把 `DrawList` 里的绘制命令（填充矩形、圆角填充、描边、裁剪，**以及文本/字形**）**真的交给 Vulkan 画出来**，
回读成 RGBA8 像素，并且能与 CPU 参考后端**逐像素对照**。
- **M3a** 做了形状（静态管线 + 顶点缓冲 + CPU 侧几何裁剪）；
- **M3b** 加了**第二条管线**做文本：每个字形一个四边形，采样字形图集纹理（覆盖率 → 乘色）；
- **M3c** 把这套顶点流与管线状态**接到窗口路径**（两条路径共用同一批构造函数与着色器），
  窗口里能直接看到界面，且上屏像素与 CPU 逐像素对照（见 [`window.md`](window.md) 第 5 节）。

**什么时候用它**：
- 你要在**无窗口**环境里用 GPU 出图（离屏），并想确认它和 CPU 基准一致；
- 你在做 GPU 渲染器的改动，需要一个「与 CPU 逐像素对照」的回归抓手；
- 你要拿 GPU 渲染的像素做后续处理（回读是 RGBA8 缓冲）。

**什么时候不该用它**：
- 你要把结果**显示在窗口**里 —— 窗口呈现是 **M3c**（见 [`vulkan-swapchain.md`](vulkan-swapchain.md)）；
- 你要画**任意图片**（RGBA 纹理）—— 目前只有字形图集这一条**专用** `R8_UNORM` 纹理；
- 你要吃满性能 —— 形状与文本各自**每帧一个顶点缓冲、一次 draw**，没有批处理优化（见第 7 节）。

## 2. 最小示例

```rust
use deer_gui::prelude::*;
use deer_gui::vk::GpuGeometryRenderer;
use std::path::Path;

fn main() -> Result<(), String> {
    let extent = Extent { width: 320, height: 200 };
    let font_size = 16.0;
    let theme = Theme { surface: Color::rgb(0x10, 0x14, 0x24), ..Theme::default() };

    // ① 一棵**含文本**的树 → 用真实字体度量布局 → 绘制列表
    let font_path = deer_gpu::measure::find_system_font()
        .ok_or_else(|| "找不到系统字体".to_string())?;
    let engine_gpu = TextEngine::from_font_file(Path::new(&font_path), font_size)
        .map_err(|e| e.to_string())?;
    let engine_cpu = TextEngine::from_font_file(Path::new(&font_path), font_size)
        .map_err(|e| e.to_string())?;

    let mut app = Builder::new(Kind::Column, "app").padding(12.0).gap(8.0);
    app.text("GPU text 真实字形");
    app.button("Apply");
    let tree = app.build();
    let style = TextStyle { font_size, line_height: theme.line_height };
    let geo = layout(&tree, Rect::new(0.0, 0.0, 320.0, 200.0), style, &engine_gpu.measure());
    let list = deer_gui::gpu::build_draw_list(&tree, &geo, theme.clone(), &engine_gpu.measure());

    // ② 同一份列表：GPU 画一遍（几何管线 + 文本管线）、CPU 画一遍（**必须 with_text**）
    let mut gpu = GpuGeometryRenderer::new(0, extent, theme.surface)
        .map_err(|e| e.to_string())?
        .with_text(engine_gpu)                       // ← 不调它 ⇒ Text 仍按 M3a 行为报 Unsupported
        .map_err(|e| e.to_string())?;
    let gpu_px = gpu.render(&list).map_err(|e| e.to_string())?;
    let mut cpu_renderer = CpuRenderer::with_text(engine_cpu);   // ← `new()` 是占位格模型，不能用
    let cpu = cpu_renderer.render(extent, &list, theme.surface).map_err(|e| e.to_string())?;

    // ③ 不透明内容（形状 + 文本）⇒ 必须逐字节相同
    assert_eq!(gpu_px, cpu.pixels, "GPU 与 CPU 必须逐字节相同");
    Ok(())
}
```

跑完整示例（写 `render_out/gpu_geometry.png` 与 `render_out/gpu_geometry_cpu.png`，
并做逐字节对照 + 确定性 + **图集不重传** + 额外裁剪场景断言）：

```sh
cargo run -p deer-gui --example gpu_geometry
```

本机实测输出（Windows / Intel RaptorLake-S 集显 / `consola.ttf`）：

```text
字体        : C:\Windows\Fonts\consola.ttf
画布        : 360×220（字号 16px）
绘制命令    : 17 条（填充 0 / 圆角 7 / 描边 5 / 裁剪 0 对 / 文本 5）
文本管线    : 已接管（with_text）
跳过文本    : 1（空串 / size<=0 / 被裁空 / 图集放不下）
适配器      : Intel(R) RaptorLake-S Mobile Graphics Controller（index=0）
非清屏色像素: 10324 / 79200
最大通道差  : 0（要求 0，逐字节相同）
额外场景    : 裁剪 + 嵌套裁剪 + 粗描边 + 被裁空文本 → 最大通道差 0
重复渲染    : 像素逐字节相同、图集**零重传**（上传计数差 0）
产物        : render_out/gpu_geometry.png 与 render_out/gpu_geometry_cpu.png（两份文件逐字节相同，317108 字节）
```

（`跳过文本 : 1` 来自额外场景里那条**被 clip 裁空**的文本；主场景是 0。详见第 6 节「已知差异」。）

## 3. 完整 API

### `deer_vk::GpuGeometryRenderer`（`deer_gui::vk::GpuGeometryRenderer`）

| 方法 | 说明 |
|---|---|
| `GpuGeometryRenderer::new(adapter_index: usize, extent: Extent, clear: Color) -> GpuResult<Self>` | 打开 Vulkan 设备并建一整套离屏设施（静态管线、离屏图像、暂存缓冲、栅栏）。**`extent` 为 0 时按 1 处理**（与 CPU `Framebuffer::new(..max(1))` 同一约定） |
| `with_text(self, engine: TextEngine) -> GpuResult<Self>` | **消费式**开启文本管线：建第二条管线（`TextVertex`，stride 32）+ 描述符集 + 最近邻采样器，并接管 `TextEngine`。**不调用它时 `DrawCmd::Text` 仍按 M3a 行为报 `Unsupported`**（不会静默丢弃） |
| `text_enabled(&self) -> bool` | 文本管线是否已接管（是否调过 `with_text`） |
| `text_skipped(&self) -> usize` | 上一帧**被跳过的文本命令数**（没有产出任何顶点）：空串 / `size <= 0` / 与 clip 求交后无可见字形 / 字形全放不进图集（含只有空白字形）。**跳过不报错** |
| `extent(&self) -> Extent` | **实际**渲染尺寸（请求 0 尺寸时是 1×1） |
| `validation_enabled(&self) -> bool` | **校验层是否真的在跑**（不是「是否请求」）。测试据此断言 `DEER_VK_VALIDATION=1 ⇒ 层确实启用`，把「零校验消息」从空话变成有前提的结论 |
| `render(&mut self, list: &DrawList) -> GpuResult<Vec<u8>>` | 画一帧并回读：**RGBA8，长度 = 宽 × 高 × 4**，行优先无 padding。每帧把顶点重新灌进缓冲（容量不足才重建） |
| `unsupported(&self) -> &[String]` | 上一帧**未能翻译**的命令说明（只有「没调 `with_text` 而列表里有 `Text`」这一种）。`render` 返回 `Unsupported` 时用它看具体是哪条 |

### 支持 / 不支持的 `DrawCmd`

| 命令 | 支持 | 说明 |
|---|---|---|
| `FillRect` | ✅ | |
| `FillRoundRect` | ✅ | 半径可以大于半宽/半高（`round-huge` 语料） |
| `StrokeRect` | ✅ | `width` 先 `max(1)`；**带宽可以大于矩形边长**（边带伸出矩形之外也照画，与 CPU 一致） |
| `PushClip` / `PopClip` | ✅ | **CPU 侧几何裁剪**（与 CPU 语义逐字对齐），不依赖 scissor（也不用动态状态） |
| `NodeHint` | ✅（无像素） | 诊断信息，安静忽略 |
| `Text` | ✅（需 `with_text`） | 每个字形一个四边形 + 图集 `uv`；不透明文本与 CPU **逐字节相同**，半透明 ≤ 1 LSB。**空串 / `size <= 0` / 被裁空 / 图集放不下 ⇒ 跳过并计入 `text_skipped()`，不报错** |

### 数据流与格式约定

```text
DrawList ──gpu_geom::build_stream──→ GpuVertex 流 ──┐
         └─gpu_text::build_text_stream─→ TextVertex 流 ──┤
                                                          ▼
                                    静态管线 A（形状）/ 静态管线 B（文本，绑定字形图集）
                                                          ▼
                                  离屏 R8G8B8A8_UNORM 图像 → 屏障 → copyImageToBuffer → map 回读
```

| 项 | 值 / 约定 |
|---|---|
| 形状顶点（管线 A） | `#[repr(C)]`：`pos: vec2`（NDC）@0、`rect: vec4`（**原始**像素矩形）@8、`radius_kind: float` @24、`color: vec4` @28，**stride 44** |
| 文本顶点（管线 B） | `#[repr(C)]`：`pos: vec2` @0、`uv: vec2` @8、`color: vec4` @16，**stride 32**（`pos` 8 + `uv` 8 + `color` 16） |
| `radius_kind` | `0` = 普通填充；`> 0` = 圆角半径；`< 0` = 描边（`-带宽`；`width == 1` 用 `-1.0`） |
| NDC | `x = 2*px/w - 1`、`y = 2*py/h - 1`（**y 向下为正**）⇒ 像素 (0,0) 映射到 `(-1,-1)` |
| 顶点颜色 | `Color` 的通道 `u8 / 255`，**alpha 会 `clamp(0.0, 1.0)`**（与 CPU `blend_cov` 的第一步一致：`Color::rgba` 不校验 alpha，越界值不能靠「没人会传」来保证一致）；**不预乘** |
| 混合 | `src-alpha / one-minus-src-alpha`（与 CPU `blend_cov` 同式） |
| 颜色附件 | **`R8G8B8A8_UNORM`**（不是 `_SRGB`） |
| 裁剪（形状） | CPU 侧几何裁剪；顶点 `pos` 用**裁剪后**矩形，顶点属性 `rect` 保留**原始**矩形（圆角/描边判据要按原始矩形算） |
| 裁剪（文本） | 同样是 CPU 侧几何裁剪：把字形四边形裁到可见区（文本着色器没有矩形判据 ⇒ 与 CPU 的逐像素裁剪等价）；**局部裁剪后 `uv` 必须按可见边界重算**，照抄整块 uv 会采样错位 |
| 字形图集纹理 | **`R8_UNORM`**、`mip_levels = 1`、**最近邻 + ClampToEdge** 采样；只在**指纹变化**时重传（指纹 = `(图集宽, 图集高, 已光栅化字形数)`），不每帧重传 |
| `uv` 精度 | 取「图集纹素的像素边界」⇒ **NEAREST** 下逐像素命中与 CPU 直接查表**同一个纹素**（线性过滤会把邻居纹素混进来，两边就对不上了） |
| 图集尺寸读取时机 | 必须在**所有字形都入图集之后**再读尺寸（图集会按需增高，先读会让 `v` 偏小）；CPU 侧同样顺序 |

**不可回退前提（改代码前先读这四条）**：

1. **viewport / scissor：离屏用「静态」是当前实现事实；「动态画不出像素」是存疑的旧结论（未证实）** ——
   管线把 viewport/scissor **写死**成整幅 extent，录制时**不调用** `vkCmdSetViewport` / `vkCmdSetScissor`；
   换 extent 就**新建一个渲染器**（静态状态跟着 extent 走）。
   > ⚠️ **这里原本写的理由是「动态版在本机 Intel 核显上画不出任何像素」—— 它既没被证实、也没被推翻**
   > （M3c 的对照实验只是**动摇**了它）：
   > - **支持「动态可用」**：同一台 Intel 集显（窗口路径）**动态与静态各跑 30 帧都出 93900 界面像素，且完全相同**；
   > - **新发现的失败模式**：「管线**声明**了动态状态却**从不调** `vkCmdSetViewport`」⇒ 规范未定义行为 ⇒ 本机驱动**崩**：
   >   窗口路径 `0xC000041D`、**离屏路径 `0xC0000005`（0/21 用例跑完）**；
   > - **为什么不能定为「M2a 是误诊」**：M2a 当年记的症状是「**画不出任何像素**」（清屏正常、绘制为零），
   >   而这两次复现出的症状是「**崩溃**」—— **症状不同**，不能据此断定同一个根因 ⇒ 定性「**高度可能**」。
   > - **可执行的待办**：重跑**当年的场景（离屏 + 三角形管线 + 动态状态）**，按「清屏像素 / 绘制像素 / 是否崩溃」
   >   逐格记录 —— 这是把这条旧结论钉死或钉倒的唯一实验。
   > **当前事实**：动态在窗口路径上就是产品行为（每帧真的设置）；离屏继续用静态只是因为**已够用**
   > （不必每帧设置、不必因 resize 重建管线）。将来若让离屏改走动态，**必须每帧真的设置**
   > `vkCmdSetViewport`/`vkCmdSetScissor`（否则就是上面那种崩溃）。
   > 反过来，**静态管线的动态状态列表是空的** ⇒ 对它调这两个命令会报校验错（`device.rs` 注释明确「不要再调」）——
   > 两条约束是一体两面：声明了动态就必须设，声明了静态就不能设。
   > 诊断开关 `DEER_VK_WINDOW_VIEWPORT=static|dynamic`（默认 dynamic）见 [`window.md`](window.md) 第 5.2 节。
2. **颜色附件必须是 `R8G8B8A8_UNORM`，不能是 `_SRGB`**：CPU 基准**不做 gamma 转换**，
   用 SRGB 格式会让 GPU 多一次编码 ⇒ 两边**系统性对不上**。
   （上屏路径同理但理由更硬：sRGB 附件**连混合都发生在线性空间**，与 CPU 的字节空间
   `blend_cov` 实测差 **44 字节** ⇒ 窗口交换链也改成**线性 `*_UNORM`**，见
   [`window.md`](window.md) 第 5.1 节。两条路径现在都刻意避开 sRGB 附件。）
3. **「零校验消息」是可回归断言，但它有明确的覆盖边界**：
   **「层确实在跑」**由 `validation_layer_state_matches_the_request` 钉住（对照 `DEER_VK_VALIDATION` 的请求与
   `GpuGeometryRenderer::validation_enabled()` 的实际状态）；**「消息为零」**由
   `ffi::validation_message_count()`（进程级 `AtomicUsize` 计数）配合 parity 用例里的
   `assert_no_validation_messages`（单帧 / 不透明语料 / 半透明语料 / 越界 alpha 四处 `assert_eq!(count, 0)`）钉住——
   不再靠人眼看 stderr。**限制仍在**：计数只在 `DEER_VK_VALIDATION=1` 时有判别力（层没开时回调不跑、计数恒为 0），
   且 **VVL 不做通用同步验证** ⇒ 「零消息」**不能**证明内存域依赖是对的
   （例如删掉 host→vertex 屏障它也不报错；那条依赖由
   `host_to_vertex_barrier_is_emitted_once_per_non_empty_frame` 单独守着）。
4. **半透明 1 LSB 是实测上限、不是证明上界**：8 位 UNORM 的目标舍入与 CPU 的 `f32` 舍入在
   个别像素上会差 1；实测最大差就是 1，但**没有证明**它不可能更大。
5. **文本必须用「最近邻 + ClampToEdge + `R8_UNORM`」这一组**：CPU 是**整数查表**，
   线性过滤会把邻居纹素混进来；`mip_levels = 1`（不需要 LOD）。换采样方式就不是「逐像素等价」了。

## 4. 自检（怎么确认你真的用对了）

「跑成功」不等于「画对了」，所以断言要落在**缓冲长度、最大通道差、确定性**上：

```rust
// ① 回读长度 = 宽×高×4（先排除「尺寸算错」这类低级问题）
assert_eq!(gpu_px.len(), (extent.width * extent.height * 4) as usize);

// ② 不透明几何 ⇒ 逐字节相同（这是本切片的硬性判据）
assert_eq!(gpu_px, cpu.pixels, "GPU 与 CPU 必须逐字节相同");

// ③ 画面不能只有清屏色 —— 否则「什么都没画」也会「通过」
let clear = theme.surface;
assert!(gpu_px.chunks_exact(4).any(|p| !(p[0] == clear.r && p[1] == clear.g && p[2] == clear.b)));

// ④ 确定性：同一列表重复渲染必须逐字节相同
let again = gpu.render(&list)?;
assert_eq!(gpu_px, again);

// ⑤ 实际 extent 必须等于请求（0 尺寸会变成 1×1）
assert_eq!(gpu.extent(), extent);
```

本仓库的**权威判据**在 `crates/deer-vk/tests/gpu_vs_cpu.rs`（测试数**随加固增长**，
以 `cargo test -p deer-vk --test gpu_vs_cpu -- --list` 的输出为准 —— 这里刻意不写死数字）：

**形状（M3a）**：

| 测试 | 判据 | 本机实测 |
|---|---|---|
| `opaque_drawings_match_cpu_byte_for_byte` | 不透明语料**逐字节相同**（`max_allowed = 0`，并在 `compare` 里额外 `assert_eq!(gpu, cpu)`） | **最大通道差 0** |
| `semi_transparent_drawings_match_cpu_within_one_lsb` | 半透明语料 ≤ 1 LSB | **最大通道差 1**（实测**只有 `alpha-clip` 非 0**，其余场景均 0） |
| `a_different_extent_uses_its_own_static_viewport` | 换 extent ⇒ 管线里的静态 viewport 跟着变 | 8×6 场景差 0 |
| `degenerate_extent_renders_as_one_by_one_like_the_cpu` | 退化 extent 两边都按 1×1 | 差 0 |
| `consecutive_frames_do_not_leak_vertex_data` | 连续多帧不串上一帧的顶点 | 三帧差 0 |
| `text_is_reported_as_unsupported_and_the_renderer_survives` | **没调 `with_text`** 的渲染器保持 M3a 行为：`Text` ⇒ `Unsupported`，之后仍能正常渲染 | 之后一帧差 0 |
| `unbalanced_clip_is_rejected_like_the_cpu_backend` | 裁剪栈不平衡 ⇒ GPU 与 CPU **同样**拒收 | 两边都 `Err` |
| `vertex_layout_matches_the_hand_written_attribute_offsets` | 顶点布局的字面偏移（stride 44 / 0 / 8 / 24 / 28） | 通过 |
| `validation_layer_state_matches_the_request` | 请求了校验层 ⇒ 层**确实在跑**（`validation_enabled()`），没请求 ⇒ 必须为假 | 通过 |
| `host_to_vertex_barrier_is_emitted_once_per_non_empty_frame` | 有顶点的帧**恰好 +1** 条 host→vertex 屏障；空帧不增；每帧重传顶点 ⇒ 每帧都发（删掉那条屏障 ⇒ 变红） | 通过 |
| `render_is_refused_after_an_unconfirmed_submit` | 上次提交未确认完成之后 `render` **必定报错**且**持续拒绝**（含空帧），不去碰任何资源 | 通过 |
| `out_of_range_alpha_matches_cpu_byte_for_byte` | 越界 alpha（`1.5` / `2.0` / `-1.0`）两边都按 `clamp(0.0, 1.0)` 混合 ⇒ 逐字节相同 | 最大通道差 0 |

**文本（M3b）**：

| 测试 | 判据 | 本机实测 |
|---|---|---|
| `text_drawings_match_cpu_pixel_for_pixel` | 文本语料（单/多字符、`align=0/1/2`、超大 size、空串、零面积、被裁空、局部裁剪、缺字豆腐、z 序…）**逐字节相同** | **最大通道差 0**；空串/零面积/被裁空**不报错**、计入 `text_skipped` |
| `semi_transparent_text_matches_cpu_within_one_lsb` | 半透明文本 ≤ 1 LSB（与形状同源：CPU `round()` vs GPU UNORM 定点） | = 1 |
| `text_and_shapes_keep_z_order` | 文本与形状**交错**时按命令顺序绘制（两条管线不能各自成批） | 与 CPU 一致 |
| `text_false_positives_are_skipped_and_counted_not_errors` | M3a 会假阳性报 `Unsupported` 的三类（空串 / 零面积 / 被裁空）现在**跳过并计数** | `render` 返回 `Ok`、`unsupported()` 空、`text_skipped()` 增加 |
| `consecutive_text_frames_stay_in_sync` | 连续 4 帧同一图集：像素稳定（不串图集/不漂） | 逐字节相同 |
| `repeated_frames_with_unchanged_atlas_do_not_reupload_texture` | 图集未变 ⇒ **零重传**（用 `device::texture_r8_upload_count()` 的差值断言；删掉指纹判断 ⇒ 变红） | 首帧 1 次上传，之后 0 次 |

跑法：`cargo test -p deer-vk --test gpu_vs_cpu -- --nocapture`（会逐场景打印最大通道差）。

## 5. 常见坑

| 现象 | 原因 | 怎么改 |
|---|---|---|
| 颜色整体偏亮/偏暗，和 CPU 对不上 | 颜色附件用了 `_SRGB`：GPU 多做一次 sRGB 编码，CPU 不做 | 用 **`R8G8B8A8_UNORM`**（见第 3 节的不可回退前提 2） |
| 画面上什么都没有（甚至进程崩） | **声明了动态状态却从不设置**（`vkCmdSetViewport`/`SetScissor`）—— 规范未定义行为，本机 Intel 会崩（`0xC000041D`） | 本模块走**静态** viewport/scissor（写进管线）；**若改用动态，就必须每帧真的设置**（见第 3 节前提 1 的更正） |
| 圆角/描边位置在裁剪后错位 | 把顶点 `pos`（裁剪后矩形）也当成了形状判据输入 | 形状判据一律用**顶点属性 `rect`（原始矩形）**；`pos` 只决定光栅化范围 |
| 带文字的树 `render` 报 `Unsupported` | 建渲染器时**没调 `with_text`** ⇒ 文本仍按 M3a 行为被拒收（刻意：不静默丢弃） | 用 `GpuGeometryRenderer::new(..)?.with_text(engine)?`；`text_enabled()` 可查 |
| 文本对照全红、偏差很大（不是 1 LSB） | CPU 侧用了 `CpuRenderer::new()` —— 那是**占位格**模型（0.6em 等宽方块 + i32 截断除法），而 GPU 画的是真字形 | CPU 侧必须 `CpuRenderer::with_text(engine)`，且与 GPU 各持一个**同源**引擎（同字体、同字号、同命令顺序 ⇒ 图集槽位一致） |
| 文本边缘脏边 / 采样整体错位 | ① 采样用了线性过滤；② **局部裁剪后照抄整块 `uv`**（没按可见边界重算） | 采样必须 **NEAREST + ClampToEdge**；`uv` 按可见区重算（`gpu_text` 已如此，改代码时别破） |
| 图集变大后文本竖向错位 | 在**所有字形入图集之前**读了图集尺寸（`v` 的分母偏小） | 先把字形全部解析/光栅化完，再读图集尺寸（CPU 侧也是这个顺序） |
| `align` 传了 `3` 或更大，位置「莫名在左边」 | `align` 是 `u8`，只有 `0/1/2` 有定义 | **未定义值 ⇒ 左对齐**，与 CPU 一致（契约；测试 `unknown_align_values_fall_back_to_left_alignment` 钉住） |
| `size` 传 `0`/负数时两边画面差很多 | **已知有意差异**：GPU 跳过、CPU 画 1px 字形 | 见第 6 节；做对照时别用 `size <= 0` |
| 半透明边缘差 1 | 8 位 UNORM 舍入 vs CPU `f32` 舍入 | 这是实测上限；对照时用 `≤1`（不透明场景仍要求 0） |
| 画面比预期「少了一块」（整条几何消失） | 绘制列表的**裁剪栈净计数不平衡**（`PushClip`/`PopClip` 未配对）⇒ CPU 与 GPU **同样拒收** | 看 `list.clip_balanced()`；两边行为一致是有意的 |
| 以为「多一个 `PopClip`」也会被拒 | 只有**帧末净计数不平衡**才两边都拒；`[PopClip, PushClip]` 这种**净计数配平但弹过一次全画布**的列表，**CPU 与 GPU 都照画**（`GpuStream::clip_unbalanced` 只是诊断，不作为报错条件 —— Ruling 19 的正面用例） | 用 `net_balanced_with_extra_pop` 那种列表时，两边仍然逐字节一致，别自行加拒收 |
| 请求 0×0 却拿到 1×1 | `new` 把 0 尺寸按 1 处理（Vulkan 图像不能是 0），与 CPU 的 `max(1)` 同约定 | 用 `gpu.extent()` 拿**实际**尺寸再比长度 |
| 换了 `extent` 但画面是旧的 | 这个渲染器的静态状态是**建的时候**定死的 | 新建一个 `GpuGeometryRenderer` |

## 6. 已知差异（**有意的**，不是 bug）

| 差异 | 触发条件 | GPU 行为 | CPU 行为 |
|---|---|---|---|
| **`size <= 0` 的文本** | `DrawCmd::Text` 的 `size <= 0.0`（含 `0` 与负数） | **跳过**该命令，并计入 `GpuGeometryRenderer::text_skipped()` | 把 `size` 交给 `TextEngine::glyph`，后者把字号夹到 `>= 1` ⇒ **画出 1px 的字形** |

**为什么这样定**：GPU 侧按计划（M3b-T3）只接受正字号；这条差异**显式记录**在这里，
而 parity 语料**刻意不含** `size <= 0` 的场景（否则两边必然不等，会把「有意差异」伪装成「回归」）。

**判断「我会不会踩到」**：只要绘制列表里出现 `size <= 0` 的 `DrawCmd::Text`，两块像素就会不同
（通常**只有那一小块**不同，其余仍逐字节相同）。

**绕法**：把 `size` 夹到 `>= 1`（与 CPU 对齐），或在交给 GPU 前丢掉这类命令。

> 其余「看起来像差异」的都不是差异：**空串 / 零面积 / 被裁空 / 图集放不下**这四类，
> GPU 跳过（计入 `text_skipped`）、CPU 也一个像素都不画 ⇒ 两边**逐字节相同**
> （M3b 修掉了 M3a 的无条件报错假阳性）。

## 7. 相关

- 绘制列表（GPU 消费的东西）：[`draw-list.md`](draw-list.md)
- CPU 参考后端（对照的基准）：[`rendering.md`](rendering.md)、[`pixels.md`](pixels.md)
- 上屏（surface/交换链/呈现，M2b）：[`vulkan-swapchain.md`](vulkan-swapchain.md)
- GPU HAL（`Device`/`Frame` 抽象层）：[`gpu-hal.md`](gpu-hal.md)
- 字形上 GPU 的相关：[`text-rendering.md`](text-rendering.md)（CPU 侧真实字形）、[`glyph-atlas.md`](glyph-atlas.md)（图集打包）、[`glyph-raster.md`](glyph-raster.md)（覆盖率位图）
- **做不到**（本模块的边界）：
  - **通用图像 / RGBA 纹理**：目前只有**字形图集**这一条专用 `R8_UNORM` 覆盖率纹理；
    通用纹理创建/上传（`create_texture`/`upload_texture` 的 HAL 路径）仍未实现；
  - **窗口呈现**：本模块只出离屏像素；把界面呈到窗口是 **M3c**；
  - **批处理优化**：形状与文本**各自**每帧一个顶点缓冲、一次 draw；没有跨命令合批/跨帧复用；
  - **没有 sRGB / 色彩管理**：刻意用线性 UNORM（为了与 CPU 对齐）；
  - **没有 MSAA**：形状的抗锯齿由 CPU 侧的整数像素判据决定；文本的覆盖率来自字形位图（不是 MSAA）。
  - **文本的 `size <= 0` 与 CPU 不一致**（有意，见第 6 节）。

## 8. 检查清单（发布前过一遍）

- [x] 示例能跑：`cargo run -p deer-gui --example gpu_geometry` → `exit=0`（形状 + 文本）
- [x] 示例有自检断言（逐字节对照 + 非清屏色 + 确定性 + **图集零重传** + 额外裁剪/被裁空文本场景）
- [x] `FEATURES.md` 已登记（状态 / 指南链接 / 示例命令都对）
- [x] 如果属于新手主线，`docs/TUTORIAL.md` 已更新（第 13 章）
- [x] 明确写了「做不到什么」，并单独写了「已知差异」（`size <= 0`）
