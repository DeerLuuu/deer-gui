# 功能指南：间接绘制（indirect-draw）

> 状态 ✅（**离屏 + 窗口两条路径**）· 示例 `cargo run -p deer-gui --example indirect_draw` ·
> 清单条目见 [`FEATURES.md`](../../FEATURES.md)

## 1. 这是什么 / 什么时候用它

每次绘制不调 `vkCmdDraw`，而是把「画几个索引」写进一块**间接命令缓冲**，
再调 `vkCmdDrawIndexedIndirect(drawCount = 1, stride = 20)` —— 让**驱动从缓冲里读**绘制参数。

```text
  顶点缓冲（统一顶点流）  ─┐
  索引缓冲（0..N 顺序）  ─┼─→  vkCmdDrawIndexedIndirect(drawCount = 1, stride = 20)
  间接命令缓冲（20 字节）─┘         ↑ VkDrawIndexedIndirectCommand { indexCount, ... }
```

**什么时候用它**：
- 你的绘制参数在**提交前**才知道（本项目的顶点数按帧变化，正是这种情况）；
- 你想让「绘制参数」和「顶点数据」走**同一条上传路径**（都是主机可见缓冲 + 同一个屏障机制）；
- 你打算以后引入**多批次 / GPU 侧决定画什么**（`drawCount > 1`、GPU 生成命令）——
  间接路径是那条路的前置基建。

**什么时候不该用它**：
- 你只是想「画快一点」—— 间接绘制在这里**不是合并批次**：本项目每帧仍是
  **1 次** draw（统一管线本来就是 `1 bind / 1 draw / 1 submit`）。换的是**发命令的方式**；
- 你以为 `drawCount > 1` 能一次发多批 —— **做不到**：本项目形状是单一顺序索引流，没有多批要发（见第 6 节）。

## 2. 最小示例

间接绘制**不是**你手写的 API：它是 `GpuGeometryRenderer::render` / `WindowedRenderer::draw_and_present`
内部**默认**的发命令方式。你要做的是**验证它真的走了 indirect**：

```rust
use deer_gpu::null::CpuRenderer;
use deer_gpu::{Color, DrawCmd, DrawList, Extent, RectI};
use deer_vk::GpuGeometryRenderer;

fn main() -> Result<(), String> {
    let extent = Extent { width: 32, height: 24 };
    let clear = Color::rgb(16, 16, 16);
    let mut r = GpuGeometryRenderer::new(0, extent, clear).map_err(|e| e.to_string())?;

    let mut list = DrawList::new();
    list.push(DrawCmd::FillRect { rect: RectI::new(2, 1, 9, 5), color: Color::WHITE });

    let before = r.render_stats();
    let gpu = r.render(&list).map_err(|e| e.to_string())?;
    let after = r.render_stats();

    // ① 真的走了 indirect：计数与真实 vkCmdDrawIndexedIndirect 同处自增
    assert_eq!(
        after.indirect_draws - before.indirect_draws,
        1,
        "有顶点的帧必须恰好 1 次间接绘制"
    );
    // ② 像素判据一字不变：不透明语料与 CPU 逐字节相同
    let cpu_frame = CpuRenderer::new().render(extent, &list, clear).map_err(|e| e.to_string())?;
    assert_eq!(gpu, cpu_frame.to_rgba(), "换了发命令的方式，像素不许变");
    Ok(())
}
```

跑完整示例（打印逐条语料的计数表 + 稳态 3 帧 + 边界说明）：

```sh
cargo run -p deer-gui --example indirect_draw
```

窗口路径那一半（需要真窗口 ⇒ 必须设门禁变量）：

```sh
$env:DEER_VK_WINDOW_TESTS='1'; cargo run -q -p deer-gui --features window --example indirect_draw
```

本机实测输出（Windows / Intel RaptorLake-S 集显）：

```text
② 逐条语料：像素对照 + 每帧计数
   empty      draw=0 switch=0 submit=1 indirect=0 idx_up=0 ind_up=0 alloc=0
   fill       draw=1 switch=1 submit=1 indirect=1 idx_up=1 ind_up=1 alloc=3
   round      draw=1 switch=1 submit=1 indirect=1 idx_up=0 ind_up=0 alloc=0
   stroke     draw=1 switch=1 submit=1 indirect=1 idx_up=1 ind_up=1 alloc=0
   ✅ 累计 draw=3 / indirect=3（一一对应）

③ 稳态零分配（同一份语料，连续 3 帧）
   第 0 帧: draw=1 switch=1 submit=1 indirect=1 vtx_up=1 idx_up=1 ind_up=1 alloc=0
   第 1 帧: draw=1 switch=1 submit=1 indirect=1 vtx_up=0 idx_up=0 ind_up=0 alloc=0
   第 2 帧: draw=1 switch=1 submit=1 indirect=1 vtx_up=0 idx_up=0 ind_up=0 alloc=0
```

## 3. 完整 API

间接绘制**没有**面向使用者的开关（它恒开）。你能用的是**观测面**：

### `RenderStats`（`GpuGeometryRenderer::render_stats()` / `WindowedRenderer::render_stats()`）

| 字段 | 含义 | 判据用法 |
|---|---|---|
| `draw_calls` | **派发次数**（`vkCmdDraw` 与 `vkCmdDrawIndexedIndirect` 都算） | 稳态每帧 = 1 |
| `indirect_draws` | `vkCmdDrawIndexedIndirect` 的调用次数 | **证明走的是间接路径**：= `draw_calls` |
| `pipeline_switches` | 管线绑定次数 | 统一管线 ⇒ 有绘制时 = 1 |
| `submits` | `vkQueueSubmit` 次数 | 每帧 = 1（含 clear-only 帧） |
| `buffer_uploads` / `index_uploads` / `indirect_uploads` | 顶点 / 索引 / 间接命令缓冲的**上传次数** | 稳态（内容不变）= 0 |
| `buffer_allocations` | 缓冲分配次数 | 稳态 = 0 |

**口径**：`RenderStats` 是**累计值**；差值才是「这一帧发生了什么」。示例里的 `delta()` 就是逐字段相减。

**为什么 `draw_calls` 之外还要 `indirect_draws`**：`draw_calls` 是**派发次数**，两种发法都算 ——
把 `vkCmdDrawIndexedIndirect` 换回 `vkCmdDraw`（或整段删掉）时 `draw_calls` **可能仍然对**，
但 `indirect_draws` 会掉到 0。所以「真的走了 indirect」这件事**只有 `indirect_draws` 能证伪**。

### `DrawIndexedIndirectCommand`（`deer_vk::device`）

`#[repr(C)]` 的 5 个 `u32` = **20 字节**、无 padding（手写 ABI，`offset_of!`/`size_of` 断言钉住）：

| 字段 | 说明 |
|---|---|
| `index_count` | 画几个索引（本项目 = 顶点数，顺序索引 `0..N`） |
| `instance_count` | 实例数（本项目恒 1） |
| `first_index` | 从索引缓冲的哪个位置开始 |
| `vertex_offset` | 顶点偏移（`i32`） |
| `first_instance` | 起始实例号 |

`DrawIndexedIndirectCommand::for_vertex_count(n)` 是「把 n 个顶点按顺序索引画一遍」，`to_bytes()` 给出
**驱动从缓冲里读到的**小端字节。本项目形状是顺序索引 ⇒ **只有 `index_count` 随帧变化** ——
这正是「命令内容不变就不重传」那条复用判据的依据。

## 4. 自检（怎么确认你真的用对了）

```rust
// ① 计数证明走了 indirect（这条能证伪「换回 vkCmdDraw」）
assert_eq!(after.indirect_draws - before.indirect_draws, 1);

// ② 稳态零分配零上传（内容不变 ⇒ 不重传，B3 语义）
assert_eq!(after.buffer_allocations - before.buffer_allocations, 0);
assert_eq!(after.index_uploads - before.index_uploads, 0);
assert_eq!(after.indirect_uploads - before.indirect_uploads, 0);
assert_eq!(after.indirect_draws - before.indirect_draws, 1, "但仍要发 indirect");
```

**但要注意计数也能造假**：如果 `indirect_draws += 1` 被挪到「有没有绘制」的判断**外面**，
clear-only 帧也会报 1。所以必须**同时**断言：

```rust
// 没有绘制的帧（clear-only）不许报 indirect —— 钉住「计数在绘制分支之内」
if draw_calls == 0 {
    assert_eq!(indirect_draws, 0);
}
```

**前置断言不能省**：语料里若**一条绘制都没发生**（全是空帧），「像素不变」这个结论是空的。
示例断言**整轮累计** `total_draws > 0 && total_indirect == total_draws`。

## 5. 常见坑

| 现象 | 原因 | 怎么改 |
|---|---|---|
| 「像素没变，所以一定没坏」 | 换了发命令方式但**根本没画**也满足「没变」 | 用 `indirect_draws > 0` 做**前置断言**，再谈像素 |
| 「`draw_calls` 是 1，所以肯定走了 indirect」 | `draw_calls` 两种发法都算 | 看 `indirect_draws`，不是 `draw_calls` |
| 「clear-only 帧也报 indirect=1」 | 计数被挪到绘制分支之外 | 断言 `draw_calls == 0 ⇒ indirect_draws == 0` |
| 「第 2 帧 index_upload 还是 1」 | 语料变了（顶点数变）⇒ 索引内容变 ⇒ 重传 | 这是对的。稳态要用**同一份语料**连跑 |
| 「改完实现后示例/测试没红」 | 该实现路径没被断言覆盖（假绿） | 变异验证：把 `indirect_draws += 1` 注释掉必须**确定性变红** |
| 「窗口路径没变绿」 | 没设 `DEER_VK_WINDOW_TESTS=1` ⇒ **显式跳过** | 设门禁变量；**跳过不是证据** |

## 6. 相关

- **相关功能**：[`gpu-geometry.md`](gpu-geometry.md)（形状管线；间接绘制是它的发命令方式）、
  [`textures.md`](textures.md)（纹理 quad 与它共用同一条录制/提交/回读路径 ⇒ 间接绘制同样成立）、
  [`gpu-offscreen.md`](gpu-offscreen.md)（离屏设施）、[`window.md`](window.md)（窗口侧同一套计数）、
  [`pixels.md`](pixels.md)（像素判据口径）
- **内部原理**：`crates/deer-vk/src/gpu_render.rs`（`ensure_vertex_capacity` 三种 usage、
  `RenderStats` 计数、间接命令上传与屏障）、`crates/deer-vk/src/device.rs`
  （`DrawIndexedIndirectCommand`、`cmd_draw_indexed_indirect` 符号加载）
- **判据的测试版本**：`crates/deer-vk/tests/texture_indirect.rs`
  （`indirect_draw_keeps_pixels_identical_and_is_actually_used`、
  `counter_table_is_reproducible_across_frames`、20 字节 ABI 断言）

### 做不到什么

- **多批次提交**：本项目统一管线已经是 **1 bind / 1 draw / 1 submit 每帧**（计数表三项恒为 1）
  ⇒ **没有可合并的批次**。「多批」的前提是**多张纹理 / 多个渲染目标**（bindless / 多 pass 范畴），
  属另一项需求，**未做**。
- **`drawCount > 1`**：本项目形状是**单一顺序索引流**，没有多批要发；`drawCount` 恒为 1。
- **GPU 侧生成绘制命令**（compute 写间接缓冲 / `vkCmdDrawIndexedIndirectCount`）：
  **未做** —— 本项目的间接缓冲是**主机写**的（这正是它有 host→indirect 屏障的原因）。
- **性能收益**：间接绘制**不提速**（发命令次数没变、还多了一次索引/间接缓冲上传）。
  它的价值是「绘制参数可来自缓冲」这条**能力**，以及为将来的多批次铺路。

## 7. 检查清单（发布前过一遍）

- [x] 示例能跑：`cargo run -p deer-gui --example indirect_draw` → `exit=0`
- [x] 示例有自检断言（`indirect_draws` 与 `draw_calls` 一一对应 + 稳态零分配 + 像素逐字节，
      且带前置断言）
- [x] `FEATURES.md` 已登记（状态 / 指南链接 / 示例命令都对）
- [x] `docs/TUTORIAL.md` 判断：**不属新手主线** —— 新手主线是「建树 → 出图」，
      间接绘制是**渲染层的低阶机制**（使用者甚至没有开关可拨），不放进教程
- [x] 明确写了「做不到什么」（第 6 节）
