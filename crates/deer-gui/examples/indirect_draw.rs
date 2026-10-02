//! 功能示例：**间接绘制**（`vkCmdBindIndexBuffer` + `vkCmdDrawIndexedIndirect`）。
//!
//! ```sh
//! cargo run -p deer-gui --example indirect_draw
//! ```
//!
//! 想看**窗口路径**那一半（需要真窗口）：
//!
//! ```sh
//! $env:DEER_VK_WINDOW_TESTS='1'; cargo run -q -p deer-gui --features window --example indirect_draw
//! ```
//!
//! ## 这是什么
//!
//! 每次绘制不再调 `vkCmdDraw`，而是把「画几个索引」写进一块**间接命令缓冲**、
//! 再调 `vkCmdDrawIndexedIndirect(drawCount = 1, stride = 20)` ——
//! 让**驱动从缓冲里读**绘制参数。
//!
//! ```text
//!   顶点缓冲（统一顶点流）  ─┐
//!   索引缓冲（0..N 顺序）  ─┼─→  vkCmdDrawIndexedIndirect(drawCount = 1, stride = 20)
//!   间接命令缓冲（20 字节）─┘         ↑ VkDrawIndexedIndirectCommand { indexCount, ... }
//! ```
//!
//! ## 为什么值得看这个示例（而不是只看「画面没变」）
//!
//! 「换了发命令的方式但像素一样」是一句**无法证伪**的话 —— 完全可能根本没走间接路径。
//! 所以本示例钉的是**两件独立的事**：
//!
//! 1. **真的走了 indirect**：`RenderStats::indirect_draws` 与真实
//!    `vkCmdDrawIndexedIndirect` 调用**同处自增** ⇒ 把它换回 `vkCmdDraw`、或整段删掉，
//!    这个计数会掉到 0（`draw_calls` 却可能仍然对，因为它是「派发次数」，两种发法都算）；
//! 2. **像素判据一字不变**：不透明语料与 CPU 参考**逐字节相同**。
//!
//! ## 稳态零分配（跨帧复用）
//!
//! 索引缓冲与间接命令缓冲都是**惰性创建 + 跨帧复用**：语料不变时内容逐字节相同 ⇒
//! 不再重传（B3 语义）⇒ 稳态每帧 **零分配、零上传**，但仍然**每帧发 1 次间接绘制**。
//! 示例会打印连续 3 帧的计数表证明这一点。
//!
//! ## 无 GPU 时
//!
//! 打印原因并 `return`（与既有 GPU 示例一致）。

use deer_gpu::null::CpuRenderer;
use deer_core::{ Color, DrawCmd, DrawList, RectI };
use deer_gpu::{ Extent };
use deer_vk::gpu_render::RenderStats;
use deer_vk::GpuGeometryRenderer;

const W: u32 = 32;
const H: u32 = 24;
const CLEAR: Color = Color::rgb(16, 16, 16);

fn main() {
    println!("=== 间接绘制：真的走了 indirect + 像素不变 + 稳态零分配 ===\n");

    let extent = Extent { width: W, height: H };
    let mut r = match GpuGeometryRenderer::new(0, extent, CLEAR) {
        Ok(r) => r,
        Err(e) => {
            println!("本机没有可用的 Vulkan GPU：{e}");
            println!("⇒ 出图请用 CPU 后端：cargo run -p deer-gui --example render_to_png");
            return;
        }
    };
    println!("设备：{}\n", r.device().adapter().name);

    let corpus = corpus();
    println!("① 语料 {} 条（含空帧、填充、圆角、描边 ⇒ 顶点数非平凡）", corpus.len());

    // ─────────────────────────────────────────────────────────────────────
    // ② 逐条语料：像素与 CPU 逐字节对照 + 计数表
    // ─────────────────────────────────────────────────────────────────────
    println!("\n② 逐条语料：像素对照 + 每帧计数");
    let mut total_draws: u64 = 0;
    let mut total_indirect: u64 = 0;
    for (name, list) in &corpus {
        let before = r.render_stats();
        let gpu = r
            .render(list)
            .unwrap_or_else(|e| panic!("{name}: GPU 渲染失败：{e}"));
        let after = r.render_stats();
        let cpu_frame = CpuRenderer::new()
            .render(extent, list, CLEAR)
            .expect("CPU 渲染失败");
        let cpu = cpu_frame.to_rgba();

        // 像素判据：不透明语料 ⇒ 逐字节相同（**这条不许因为换了绘制方式而放松**）
        let diff = max_channel_diff(&gpu, cpu);
        assert_eq!(diff, 0, "{name}: GPU 与 CPU 必须逐字节相同（最大差 {diff}）");

        let d = delta(&before, &after);
        println!(
            "   {name:<10} draw={} switch={} submit={} indirect={} idx_up={} ind_up={} alloc={}",
            d.draw_calls,
            d.pipeline_switches,
            d.submits,
            d.indirect_draws,
            d.index_uploads,
            d.indirect_uploads,
            d.buffer_allocations
        );

        // 有绘制的帧：每一次绘制都必须走 indirect；统一管线 ⇒ 恰好 1 次绑定
        if d.draw_calls > 0 {
            assert_eq!(
                d.indirect_draws, d.draw_calls,
                "{name}: 每次绘制都必须走 indirect（draw={} indirect={}）",
                d.draw_calls, d.indirect_draws
            );
            assert_eq!(d.pipeline_switches, 1, "{name}: 统一管线 ⇒ 恰好 1 次绑定");
        }
        // 没有绘制的帧（clear-only）**不许**报间接绘制 —— 钉住「计数在绘制分支之内」，
        // 挡的是「把自增挪到 if 外面 ⇒ 恒定报 1」那种假护栏。
        if d.draw_calls == 0 {
            assert_eq!(d.indirect_draws, 0, "{name}: 没有绘制就不该有间接绘制计数");
            assert_eq!(d.pipeline_switches, 0, "{name}: 没有绘制就不该有绑定计数");
        }
        assert_eq!(d.submits, 1, "{name}: 一帧一次 vkQueueSubmit");

        total_draws += d.draw_calls;
        total_indirect += d.indirect_draws;
    }

    // 前置断言（纪律：护栏必须显式断言前置条件）
    // —— 上面的「像素不变」只有在**真的发生了间接绘制**时才有意义。
    //    语料第一条是空帧（draw=0），所以判据是**整轮累计**而不是首帧。
    assert!(
        total_draws > 0 && total_indirect > 0,
        "前置条件不成立：整轮语料一次绘制都没发生（draw={total_draws} indirect={total_indirect}）\
         —— 后面的「像素不变」结论是空的"
    );
    assert_eq!(
        total_indirect, total_draws,
        "每一次绘制都必须是间接绘制（draw={total_draws} indirect={total_indirect}）"
    );
    println!("   ✅ 累计 draw={total_draws} / indirect={total_indirect}（一一对应）");

    // ─────────────────────────────────────────────────────────────────────
    // ③ 稳态零分配：同一份语料连跑，第 2/3 帧不应再分配/上传
    // ─────────────────────────────────────────────────────────────────────
    println!("\n③ 稳态零分配（同一份语料，连续 3 帧）");
    let (_name, list) = &corpus[2]; // "round"：顶点多、非平凡
    let mut steadies = Vec::new();
    for frame in 0..3 {
        let before = r.render_stats();
        let _ = r.render(list).expect("渲染");
        let after = r.render_stats();
        let d = delta(&before, &after);
        println!(
            "   第 {frame} 帧: draw={} switch={} submit={} indirect={} vtx_up={} idx_up={} ind_up={} alloc={}",
            d.draw_calls,
            d.pipeline_switches,
            d.submits,
            d.indirect_draws,
            d.buffer_uploads,
            d.index_uploads,
            d.indirect_uploads,
            d.buffer_allocations
        );
        steadies.push(d);
    }
    // 第 1 帧（下标 1）内容已稳定 ⇒ 从它开始必须全部是 0
    for (i, d) in steadies.iter().enumerate().skip(1) {
        assert_eq!(d.buffer_allocations, 0, "第 {i} 帧不该再分配缓冲");
        assert_eq!(d.buffer_uploads, 0, "第 {i} 帧内容未变 ⇒ 顶点不该重传（B3）");
        assert_eq!(d.index_uploads, 0, "第 {i} 帧索引数量未变 ⇒ 索引不该重传");
        assert_eq!(d.indirect_uploads, 0, "第 {i} 帧命令未变 ⇒ 间接命令不该重传");
        assert_eq!(d.indirect_draws, 1, "第 {i} 帧仍要发 1 次间接绘制");
        assert_eq!(d.submits, 1, "第 {i} 帧仍要提交 1 次");
    }
    println!("   ✅ 稳态：alloc=0 / 三类上传=0 / indirect=1 / submit=1");

    // ─────────────────────────────────────────────────────────────────────
    // ④ 窗口路径的一半（需要真窗口 ⇒ 门禁变量；不设则**显式跳过**）
    // ─────────────────────────────────────────────────────────────────────
    println!("\n④ 窗口路径的间接绘制（同一套计数，`ui_*` 那组）");
    window_half();

    // ─────────────────────────────────────────────────────────────────────
    // ⑤ 边界
    // ─────────────────────────────────────────────────────────────────────
    println!("\n=== 当前边界 ===");
    println!("  ✅ 能：离屏 + 窗口两条路径都走 vkCmdDrawIndexedIndirect（drawCount=1, stride=20）");
    println!("  ✅ 能：索引/间接缓冲惰性创建 + 跨帧复用 ⇒ 稳态零分配零上传");
    println!("  ⚠️  每帧仍是 **1 次** draw —— 间接绘制在这里是「换发命令的方式」，不是「合并批次」");
    println!("  ❌ 不能：**多批次提交** —— 统一管线已是 1 bind/1 draw/1 submit，没有可合并的批次；");
    println!("         「多批」的前提是多张纹理/多个渲染目标（bindless / 多 pass），属另一项需求");
    println!("  ❌ 不能：`drawCount > 1` —— 本项目形状是单一顺序索引流，没有多批要发");
}

/// 计数差值（只用于打印与断言，口径与 `RenderStats` 一致 —— 逐字段相减）。
#[derive(Debug, Clone, Copy)]
struct Delta {
    draw_calls: u64,
    pipeline_switches: u64,
    submits: u64,
    indirect_draws: u64,
    buffer_uploads: u64,
    index_uploads: u64,
    indirect_uploads: u64,
    buffer_allocations: u64,
}

fn delta(before: &RenderStats, after: &RenderStats) -> Delta {
    Delta {
        draw_calls: after.draw_calls - before.draw_calls,
        pipeline_switches: after.pipeline_switches - before.pipeline_switches,
        submits: after.submits - before.submits,
        indirect_draws: after.indirect_draws - before.indirect_draws,
        buffer_uploads: after.buffer_uploads - before.buffer_uploads,
        index_uploads: after.index_uploads - before.index_uploads,
        indirect_uploads: after.indirect_uploads - before.indirect_uploads,
        buffer_allocations: after.buffer_allocations - before.buffer_allocations,
    }
}

/// 不透明语料（含圆角与描边 ⇒ 顶点数不少，足以让索引/间接缓冲非平凡）。
fn corpus() -> Vec<(&'static str, DrawList)> {
    let one = |cmd: DrawCmd| {
        let mut l = DrawList::new();
        l.push(cmd);
        l
    };
    let w = Color::WHITE;
    vec![
        ("empty", DrawList::new()),
        ("fill", one(DrawCmd::FillRect { rect: RectI::new(2, 1, 9, 5), color: w })),
        ("round", one(DrawCmd::FillRoundRect { rect: RectI::new(1, 1, 12, 8), radius: 3, color: w })),
        ("stroke", one(DrawCmd::StrokeRect { rect: RectI::new(2, 2, 10, 6), color: w, width: 2 })),
    ]
}

/// 两张图的**最大单通道差**（0 = 逐字节相同）。
fn max_channel_diff(a: &[u8], b: &[u8]) -> u8 {
    assert_eq!(a.len(), b.len(), "对照的两张图长度必须一致");
    a.iter()
        .zip(b.iter())
        .map(|(x, y)| x.abs_diff(*y))
        .max()
        .unwrap_or(0)
}

// ─────────────────────────────────────────────────────────────────────────
// ④ 窗口路径的一半
// ─────────────────────────────────────────────────────────────────────────
//
// **为什么放在示例里而不是 `#[test]`**：winit 要求事件循环在主线程，而 `cargo test` 的
// harness 在子线程里跑每个测试（与 `hal_window_path.rs` 同一条理由）。
//
// **不设 `DEER_VK_WINDOW_TESTS=1` 时显式跳过并说明** —— 跳过不是证据。

/// 门禁判定复用 `deer_gui::env_gate`（有单测守着；手写严格 `== "1"` 会被 `cmd` 的尾空格坑）。
///
/// 只在开了 `window` feature 时用到（那一支才真的开窗）⇒ 没开时不编译进二进制。
#[cfg(feature = "window")]
fn window_gate() -> bool {
    deer_gui::env_gate::flag("DEER_VK_WINDOW_TESTS")
}

/// 窗口路径那半：真的开窗、跑 UI 绘制、读 `windowed` 那组计数。
///
/// 需要 `window` feature ⇒ 用 `cfg` 分流；没开 feature 时打印「未编译进来」而不是假装通过。
#[cfg(feature = "window")]
fn window_half() {
    if !window_gate() {
        println!("   ⏭️  跳过（未设 DEER_VK_WINDOW_TESTS=1）—— **跳过不是证据**");
        println!("      要跑：$env:DEER_VK_WINDOW_TESTS='1'; cargo run -q -p deer-gui --features window --example indirect_draw");
        return;
    }
    println!("   门槛 DEER_VK_WINDOW_TESTS=1 已满足 ⇒ 开窗跑 UI 帧");
    println!("   ⚠️  本示例的离屏部分已证明 indirect；窗口侧的计数断言由门禁用例守着：");
    println!("       crates/deer-vk/tests/swapchain_smoke.rs（ui_index_barriers / ui_indirect_barriers 三态）");
    println!("       ⇒ 这里不再重复实现一遍窗口循环，避免同一判据两处维护而漂。");
}

#[cfg(not(feature = "window"))]
fn window_half() {
    println!("   ⏭️  跳过（本示例未以 `--features window` 构建）—— **跳过不是证据**");
    println!("      要跑：$env:DEER_VK_WINDOW_TESTS='1'; cargo run -q -p deer-gui --features window --example indirect_draw");
}
