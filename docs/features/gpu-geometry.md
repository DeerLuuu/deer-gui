# 功能指南：GPU 几何渲染（gpu-geometry）

> 状态 ✅（**仅离屏 / 仅非文本命令**）· 示例 `cargo run -p deer-gui --example gpu_geometry` ·
> 清单条目见 [`FEATURES.md`](../../FEATURES.md)

## 1. 这是什么 / 什么时候用它

把 `DrawList` 里的**非文本**绘制命令（填充矩形、圆角填充、描边、裁剪）**真的交给 Vulkan 画出来**，
回读成 RGBA8 像素，并且能与 CPU 参考后端**逐像素对照**。这是 M3a 的终点：
从「GPU 能画一个三角形」走到「GPU 能画出**一棵界面树的几何**，且与 CPU 基准一致」。

**什么时候用它**：
- 你要在**无窗口**环境里用 GPU 出图（离屏），并想确认它和 CPU 基准一致；
- 你在做 GPU 渲染器的改动，需要一个「与 CPU 逐像素对照」的回归抓手；
- 你要拿 GPU 渲染的像素做后续处理（回读是 RGBA8 缓冲）。

**什么时候不该用它**：
- 树里有**文本** —— `DrawCmd::Text` 会明确返回 `Unsupported`（字形上 GPU 属 **M3b**），
  现在要出带文字的图请用 CPU 路径（[`text-rendering.md`](text-rendering.md)）；
- 你要把结果**显示在窗口**里 —— 窗口呈现是 **M3c**（见 [`vulkan-swapchain.md`](vulkan-swapchain.md)）；
- 你要吃满性能 —— 当前**每帧一个顶点缓冲、一次 draw**，没有批处理优化（见第 6 节）。

## 2. 最小示例

```rust
use deer_gui::prelude::*;
use deer_gui::vk::GpuGeometryRenderer;

fn main() -> Result<(), String> {
    let extent = Extent { width: 320, height: 200 };
    let theme = Theme { surface: Color::rgb(0x10, 0x14, 0x24), ..Theme::default() };

    // ① 一棵**不含文本**的树 → 布局 → 绘制列表
    let mut app = Builder::new(Kind::Column, "app").padding(12.0).gap(8.0);
    app.container_opts(Kind::Row, "card", L::new().pad(10.0).w(120.0).h(48.0).to_props(), |_| {});
    let tree = app.build();
    let geo = deer_gui::layout_tree(&tree, extent.width, extent.height, theme.clone());
    let list = deer_gui::gpu::build_draw_list(&tree, &geo, theme.clone(), &ApproxMeasure);
    assert_eq!(list.counts().text, 0, "本示例的树不含文本");

    // ② 同一份列表：GPU 画一遍、CPU 画一遍
    let mut gpu = GpuGeometryRenderer::new(0, extent, theme.surface)
        .map_err(|e| format!("建 GPU 几何渲染器失败：{e}"))?;
    let gpu_px = gpu.render(&list).map_err(|e| format!("GPU 渲染失败：{e}"))?;
    let cpu = CpuRenderer::new()
        .render(extent, &list, theme.surface)
        .map_err(|e| format!("CPU 渲染失败：{e}"))?;

    // ③ 不透明几何 ⇒ 必须逐字节相同
    assert_eq!(gpu_px, cpu.pixels, "GPU 与 CPU 必须逐字节相同");
    Ok(())
}
```

跑完整示例（写 `render_out/gpu_geometry.png` 与 `render_out/gpu_geometry_cpu.png`，
并做逐字节对照 + 确定性 + 额外裁剪场景断言）：

```sh
cargo run -p deer-gui --example gpu_geometry
```

本机实测输出（Windows / Intel RaptorLake-S 集显）：

```text
画布        : 320×200
绘制命令    : 14 条（填充 0 / 圆角 7 / 描边 7 / 裁剪 0 对 / 文本 0）
顶点数      : 210（每帧一个顶点缓冲、一次 draw）
适配器      : Intel(R) RaptorLake-S Mobile Graphics Controller（index=0）
非清屏色像素: 3152 / 64000
最大通道差  : 0（要求 0，逐字节相同）
额外场景    : 裁剪 + 嵌套裁剪 + 粗描边（带宽 > 边长）→ 最大通道差 0
```

## 3. 完整 API

### `deer_vk::GpuGeometryRenderer`（`deer_gui::vk::GpuGeometryRenderer`）

| 方法 | 说明 |
|---|---|
| `GpuGeometryRenderer::new(adapter_index: usize, extent: Extent, clear: Color) -> GpuResult<Self>` | 打开 Vulkan 设备并建一整套离屏设施（静态管线、离屏图像、暂存缓冲、栅栏）。**`extent` 为 0 时按 1 处理**（与 CPU `Framebuffer::new(..max(1))` 同一约定） |
| `extent(&self) -> Extent` | **实际**渲染尺寸（请求 0 尺寸时是 1×1） |
| `validation_enabled(&self) -> bool` | **校验层是否真的在跑**（不是「是否请求」）。测试据此断言 `DEER_VK_VALIDATION=1 ⇒ 层确实启用`，把「零校验消息」从空话变成有前提的结论 |
| `render(&mut self, list: &DrawList) -> GpuResult<Vec<u8>>` | 画一帧并回读：**RGBA8，长度 = 宽 × 高 × 4**，行优先无 padding。每帧把顶点重新灌进缓冲（容量不足才重建） |
| `unsupported(&self) -> &[String]` | 上一帧**未能翻译**的命令说明（目前只有 `DrawCmd::Text`）。`render` 返回 `Unsupported` 时用它看具体是哪条 |

### 支持 / 不支持的 `DrawCmd`

| 命令 | 支持 | 说明 |
|---|---|---|
| `FillRect` | ✅ | |
| `FillRoundRect` | ✅ | 半径可以大于半宽/半高（`round-huge` 语料） |
| `StrokeRect` | ✅ | `width` 先 `max(1)`；**带宽可以大于矩形边长**（边带伸出矩形之外也照画，与 CPU 一致） |
| `PushClip` / `PopClip` | ✅ | **CPU 侧几何裁剪**（与 CPU 语义逐字对齐），不依赖动态 scissor |
| `NodeHint` | ✅（无像素） | 诊断信息，安静忽略 |
| `Text` | ❌ | 明确 `Unsupported`（M3b），**不静默丢弃**——静默丢弃会让「少画了东西」变成一张「看起来很合理」的图 |

### 数据流与格式约定

```text
DrawList ──gpu_geom::build_stream──→ GpuVertex 流 ──memcpy──→ 顶点缓冲
                                                              │  静态管线（静态 viewport/scissor）
                                                              ▼
                                         离屏 R8G8B8A8_UNORM 图像 → 屏障 → copyImageToBuffer → map 回读
```

| 项 | 值 / 约定 |
|---|---|
| 顶点布局 | `#[repr(C)]`：`pos: vec2`（NDC）@0、`rect: vec4`（**原始**像素矩形）@8、`radius_kind: float` @24、`color: vec4` @28，**stride 44** |
| `radius_kind` | `0` = 普通填充；`> 0` = 圆角半径；`< 0` = 描边（`-带宽`；`width == 1` 用 `-1.0`） |
| NDC | `x = 2*px/w - 1`、`y = 2*py/h - 1`（**y 向下为正**）⇒ 像素 (0,0) 映射到 `(-1,-1)` |
| 顶点颜色 | `Color` 原值，0–1，**不预乘** |
| 混合 | `src-alpha / one-minus-src-alpha`（与 CPU `blend_cov(cov = 1.0)` 同式） |
| 颜色附件 | **`R8G8B8A8_UNORM`**（不是 `_SRGB`） |
| 裁剪 | CPU 侧几何裁剪；顶点 `pos` 用**裁剪后**矩形，顶点属性 `rect` 保留**原始**矩形（圆角/描边判据要按原始矩形算） |

**不可回退前提（改代码前先读这四条）**：

1. **静态 viewport / scissor**：管线把 viewport/scissor **写死**成整幅 extent，录制时**不调用**
   `vkCmdSetViewport` / `vkCmdSetScissor`。理由：**动态版在本机 Intel 核显上画不出任何像素**（M2a 实测），
   而对静态管线发动态设置命令会触发校验层报错。换 extent 就**新建一个渲染器**（静态状态跟着 extent 走）。
2. **颜色附件必须是 `R8G8B8A8_UNORM`，不能是 `_SRGB`**：CPU 基准**不做 gamma 转换**，
   用 SRGB 格式会让 GPU 多一次编码 ⇒ 两边**系统性对不上**（这与上屏路径刻意相反：
   窗口呈现用 `B8G8R8A8_SRGB`，因为那里要和系统窗口合成）。
3. **「零校验消息」是可回归断言，但它有明确的覆盖边界**：
   **「层确实在跑」**由 `validation_layer_state_matches_the_request` 钉住（对照 `DEER_VK_VALIDATION` 的请求与
   `GpuGeometryRenderer::validation_enabled()` 的实际状态）；**「消息为零」**由
   `ffi::validation_message_count()`（进程级 `AtomicUsize` 计数）配合 parity 用例里的
   `assert_no_validation_messages`（`tests/gpu_vs_cpu.rs:108/315/330` 三处 `assert_eq!(count, 0)`）钉住——
   不再靠人眼看 stderr。**限制仍在**：计数只在 `DEER_VK_VALIDATION=1` 时有判别力（层没开时回调不跑、计数恒为 0），
   且 **VVL 不做通用同步验证** ⇒ 「零消息」**不能**证明内存域依赖是对的
   （例如删掉 host→vertex 屏障它也不报错；那条依赖由
   `host_to_vertex_barrier_is_emitted_once_per_non_empty_frame` 单独守着）。
4. **半透明 1 LSB 是实测上限、不是证明上界**：8 位 UNORM 的目标舍入与 CPU 的 `f32` 舍入在
   个别像素上会差 1；实测最大差就是 1，但**没有证明**它不可能更大。

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

本仓库的**权威判据**在 `crates/deer-vk/tests/gpu_vs_cpu.rs`（本机实跑：`cargo test -p deer-vk --test gpu_vs_cpu -- --list`
→ **11 tests**；测试数/语料随加固增长，**以该命令输出为准**）：

| 测试 | 判据 | 本机实测 |
|---|---|---|
| `opaque_drawings_match_cpu_byte_for_byte` | 不透明语料**逐字节相同**（`max_allowed = 0`，并在 `compare` 里额外 `assert_eq!(gpu, cpu)`） | **最大通道差 0** |
| `semi_transparent_drawings_match_cpu_within_one_lsb` | 半透明语料 ≤ 1 LSB | **最大通道差 1**（实测**只有 `alpha-clip` 非 0**，其余场景均 0） |
| `a_different_extent_uses_its_own_static_viewport` | 换 extent ⇒ 管线里的静态 viewport 跟着变 | 8×6 场景差 0 |
| `degenerate_extent_renders_as_one_by_one_like_the_cpu` | 退化 extent 两边都按 1×1 | 差 0 |
| `consecutive_frames_do_not_leak_vertex_data` | 连续多帧不串上一帧的顶点 | 三帧差 0 |
| `text_is_reported_as_unsupported_and_the_renderer_survives` | `Text` ⇒ `Unsupported`，之后仍能正常渲染 | 之后一帧差 0 |
| `unbalanced_clip_is_rejected_like_the_cpu_backend` | 裁剪栈不平衡 ⇒ GPU 与 CPU **同样**拒收 | 两边都 `Err` |
| `vertex_layout_matches_the_hand_written_attribute_offsets` | 顶点布局的字面偏移（stride 44 / 0 / 8 / 24 / 28） | 通过 |
| `validation_layer_state_matches_the_request` | 请求了校验层 ⇒ 层**确实在跑**（`validation_enabled()`），没请求 ⇒ 必须为假 | 通过 |
| `host_to_vertex_barrier_is_emitted_once_per_non_empty_frame` | 有顶点的帧**恰好 +1** 条 host→vertex 屏障；空帧不增；每帧重传顶点 ⇒ 每帧都发（删掉那条屏障 ⇒ 变红） | 通过 |
| `render_is_refused_after_an_unconfirmed_submit` | 上次提交未确认完成之后 `render` **必定报错**且**持续拒绝**（含空帧），不去碰任何资源；`ensure_reusable()` 的守卫不依赖调用方写法 | 通过 |

跑法：`cargo test -p deer-vk --test gpu_vs_cpu -- --nocapture`（会逐场景打印最大通道差）。

## 5. 常见坑

| 现象 | 原因 | 怎么改 |
|---|---|---|
| 颜色整体偏亮/偏暗，和 CPU 对不上 | 颜色附件用了 `_SRGB`：GPU 多做一次 sRGB 编码，CPU 不做 | 用 **`R8G8B8A8_UNORM`**（见第 3 节的不可回退前提 2） |
| 画面上什么都没有 | 管线是动态 viewport，且录制时没（或不该）调 `vkCmdSetViewport` | 本模块刻意用**静态** viewport/scissor；换 extent 就**新建渲染器** |
| 圆角/描边位置在裁剪后错位 | 把顶点 `pos`（裁剪后矩形）也当成了形状判据输入 | 形状判据一律用**顶点属性 `rect`（原始矩形）**；`pos` 只决定光栅化范围 |
| 带文字的树 `render` 报 `Unsupported` | `DrawCmd::Text` 还不会画（M3b） | 移掉文本（本示例的树只用带 `pad` 的容器），或先用 CPU 路径；错误信息里能查到是哪条 |
| 半透明边缘差 1 | 8 位 UNORM 舍入 vs CPU `f32` 舍入 | 这是实测上限；对照时用 `≤1`（不透明场景仍要求 0） |
| 画面比预期「少了一块」（整条几何消失） | 绘制列表的**裁剪栈净计数不平衡**（`PushClip`/`PopClip` 未配对）⇒ CPU 与 GPU **同样拒收** | 看 `list.clip_balanced()`；两边行为一致是有意的 |
| 以为「多一个 `PopClip`」也会被拒 | 只有**帧末净计数不平衡**才两边都拒；`[PopClip, PushClip]` 这种**净计数配平但弹过一次全画布**的列表，**CPU 与 GPU 都照画**（`GpuStream::clip_unbalanced` 只是诊断，不作为报错条件 —— Ruling 19 的正面用例） | 用 `net_balanced_with_extra_pop` 那种列表时，两边仍然逐字节一致，别自行加拒收 |
| 请求 0×0 却拿到 1×1 | `new` 把 0 尺寸按 1 处理（Vulkan 图像不能是 0），与 CPU 的 `max(1)` 同约定 | 用 `gpu.extent()` 拿**实际**尺寸再比长度 |
| 换了 `extent` 但画面是旧的 | 这个渲染器的静态状态是**建的时候**定死的 | 新建一个 `GpuGeometryRenderer` |

## 6. 相关

- 绘制列表（GPU 消费的东西）：[`draw-list.md`](draw-list.md)
- CPU 参考后端（对照的基准）：[`rendering.md`](rendering.md)、[`pixels.md`](pixels.md)
- 上屏（surface/交换链/呈现，M2b）：[`vulkan-swapchain.md`](vulkan-swapchain.md)
- GPU HAL（`Device`/`Frame` 抽象层）：[`gpu-hal.md`](gpu-hal.md)
- 字形上 GPU 的前置（图集）：[`glyph-atlas.md`](glyph-atlas.md)
- **做不到**（本模块的边界）：
  - **文本 / 字形**：`DrawCmd::Text` ⇒ `Unsupported`（M3b；需要把字形图集作为纹理采样）。
    ⚠️ **已知限制（未修，本轮明确 defer）**：这个 `Unsupported` 是**无条件**的 —— 下面**任一**条件成立时
    GPU 侧仍会报错，而 **CPU 后端在这些情况下能正常出图**，于是你会看到一个**假阳性**：
    ① 文本是**空串**（`text == ""`）；② 文本矩形**零面积**（`rect.w == 0` 或 `rect.h == 0`）；
    ③ 整块文本**被 clip 完全裁掉**（与裁剪区求交后为空）。
    **判断「我会不会踩到」**：只要绘制列表里出现过 `DrawCmd::Text`（哪怕它画不出任何像素），
    `GpuGeometryRenderer::render` 就会返回 `Unsupported`，`unsupported()` 里能看到是哪条、多少字符。
    **绕法**：在把列表交给 GPU 前自行过滤掉这三类文本命令（或整棵树的文本节点）。
    **为什么现在不修**：这属于「错误策略」的行为变更，需要单独任务 + 单独 review，不能在收尾轮里改；
    **M3b 做文本时必须一并处理**（见 [`ROADMAP.md`](../../ROADMAP.md) 的 M3b 行）。
  - **纹理**：没有纹理绑定/采样（`create_texture`/`upload_texture` 仍是 `Unsupported`）；
  - **窗口呈现**：本模块只出离屏像素；把界面呈到窗口是 **M3c**；
  - **批处理优化**：当前**每帧一个顶点缓冲、一次 draw**；没有按命令合批/多帧复用；
  - **没有 sRGB / 色彩管理**：刻意用线性 UNORM（为了与 CPU 对齐）；
  - **没有 MSAA**：抗锯齿/覆盖率由 CPU 侧的整数像素判据决定（字形那套覆盖率采样不在本路径）。

## 7. 检查清单（发布前过一遍）

- [x] 示例能跑：`cargo run -p deer-gui --example gpu_geometry` → `exit=0`
- [x] 示例有自检断言（逐字节对照 + 非清屏色 + 确定性 + 额外裁剪场景）
- [x] `FEATURES.md` 已登记（状态 / 指南链接 / 示例命令都对）
- [x] 如果属于新手主线，`docs/TUTORIAL.md` 已更新（第 13 章）
- [x] 明确写了「做不到什么」
