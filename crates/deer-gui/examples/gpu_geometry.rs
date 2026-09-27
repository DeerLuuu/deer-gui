//! 功能示例：**把 `DrawList` 的非文本命令交给 Vulkan 画出来**，并与 CPU 后端**逐像素对照**（M3a）。
//!
//! ```sh
//! cargo run -p deer-gui --example gpu_geometry
//! ```
//!
//! 产物：`render_out/gpu_geometry.png`（GPU 回读）与 `render_out/gpu_geometry_cpu.png`（CPU 基准），
//! 两张图**逐字节相同**是这里的硬性断言（不透明几何 ⇒ 最大通道差 0）。
//!
//! 这一条链是：`Builder` 建一棵**不含文本**的树 → `layout_tree` → `build_draw_list` → 同一份 `DrawList`
//! 分别交给 `deer_gpu::null::CpuRenderer`（参考实现）与 `deer_vk::GpuGeometryRenderer`（真 GPU），
//! 然后比较两块 RGBA8 缓冲。**不含文本**是刻意的：`DrawCmd::Text` 目前会明确返回 `Unsupported`
//! （字形上 GPU 属 M3b），所以本示例的树只用带 `pad` 的容器 ⇒ 只产生 `FillRoundRect` / `StrokeRect`。
//!
//! 环境变量：
//! - `DEER_GPU_ADAPTER`：用第几张显卡（默认 `0`）。
//!
//! 边界（详见 `docs/features/gpu-geometry.md`）：只有**离屏**路径、只有**非文本**命令、
//! 没有纹理、没有窗口呈现、没有批处理（每帧一个顶点缓冲、一次 draw）。

use std::process::ExitCode;

use deer_gui::prelude::*;
use deer_gui::vk::GpuGeometryRenderer;

/// 画布尺寸（与 GPU 渲染器的离屏 extent 一致）。
const W: u32 = 320;
const H: u32 = 200;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("gpu_geometry 示例失败：{e}");
            ExitCode::FAILURE
        }
    }
}

/// 建一棵**不含文本**的界面树：只有带 `pad` 的容器，于是只产生圆角填充与描边。
fn build_tree() -> Node {
    let mut app = Builder::new(Kind::Column, "app").padding(12.0).gap(8.0);

    app.container_opts(Kind::Row, "bar", L::new().pad(10.0).gap(6.0).to_props(), |r| {
        r.container_opts(Kind::Column, "card_a", L::new().pad(8.0).w(96.0).h(48.0).to_props(), |_| {});
        r.container_opts(Kind::Column, "card_b", L::new().pad(8.0).w(120.0).h(48.0).to_props(), |_| {});
    });

    app.container_opts(
        Kind::Column,
        "panel",
        L::new().pad(10.0).gap(6.0).grow(1.0).to_props(),
        |p| {
            p.container_opts(Kind::Row, "inner", L::new().pad(6.0).w(150.0).h(44.0).to_props(), |_| {});
            // 显式大圆角走 `FillRoundRect { radius }`；容器渲染用固定半径，
            // 想要「超大半径」这类极端形状请看 `tests/gpu_vs_cpu.rs::opaque_corpus` 的 `round-huge`。
            p.container_opts(Kind::Row, "wide", L::new().pad(4.0).grow(1.0).h(30.0).to_props(), |_| {});
        },
    );

    app.build()
}

/// 两块 RGBA8 缓冲的最大通道差与最差字节偏移。
fn max_channel_diff(a: &[u8], b: &[u8]) -> (u8, usize) {
    let mut worst = 0u8;
    let mut at = 0usize;
    for (i, (x, y)) in a.iter().zip(b.iter()).enumerate() {
        let d = x.abs_diff(*y);
        if d > worst {
            worst = d;
            at = i;
        }
    }
    (worst, at)
}

/// 与清屏色不同的像素数（「画面不能只有一种颜色」）。
fn pixels_differing_from(buffer: &[u8], c: Color) -> usize {
    buffer
        .chunks_exact(4)
        .filter(|p| !(p[0] == c.r && p[1] == c.g && p[2] == c.b))
        .count()
}

fn run() -> Result<(), String> {
    // ① 主题：**全不透明**（CPU 基准不做 gamma，GPU 侧也是 UNORM ⇒ 逐字节可比）
    let theme = Theme {
        surface: Color::rgb(0x10, 0x14, 0x24),
        ..Theme::default()
    };
    let extent = Extent { width: W, height: H };

    let tree = build_tree();
    let geo = deer_gui::layout_tree(&tree, W, H, theme.clone());
    let list = deer_gui::gpu::build_draw_list(&tree, &geo, theme.clone(), &ApproxMeasure);

    // ② 自检：树里没有文本命令、裁剪栈平衡、全部命令可翻译
    let counts = list.counts();
    assert_eq!(
        counts.text, 0,
        "本示例的树刻意不含文本；`DrawCmd::Text` 目前会返回 Unsupported（M3b）"
    );
    assert!(list.clip_balanced(), "绘制列表的裁剪栈必须平衡");
    let stream = deer_gui::vk::gpu_geom::build_stream(&list, extent);
    assert!(
        stream.unsupported.is_empty(),
        "非文本命令应当全部可翻译，实际有：{:?}",
        stream.unsupported
    );
    assert!(
        !stream.vertices.is_empty(),
        "这棵树应当产生几何（否则两边都只是清屏色，对照没有意义）"
    );

    // ③ CPU 基准（参考实现）
    let mut cpu_renderer = CpuRenderer::new();
    let cpu = cpu_renderer
        .render(extent, &list, theme.surface)
        .map_err(|e| format!("CPU 渲染失败：{e}"))?;

    // ④ GPU：同一份 DrawList
    let adapter = std::env::var("DEER_GPU_ADAPTER")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(0);
    let mut gpu = GpuGeometryRenderer::new(adapter, extent, theme.surface)
        .map_err(|e| format!("建 GPU 几何渲染器失败（adapter={adapter}）：{e}"))?;
    let gpu_px = gpu
        .render(&list)
        .map_err(|e| format!("GPU 渲染失败：{e}"))?;
    assert!(
        gpu.unsupported().is_empty(),
        "GPU 侧不该出现 unsupported：{:?}",
        gpu.unsupported()
    );
    assert_eq!(gpu.extent(), extent, "实际渲染尺寸应当等于请求尺寸");

    // ⑤ 硬性判据：不透明几何 ⇒ 逐字节相同
    assert_eq!(
        gpu_px.len(),
        cpu.pixels.len(),
        "GPU 回读长度必须等于 CPU 帧缓冲长度（{} vs {}）",
        gpu_px.len(),
        cpu.pixels.len()
    );
    let (max_diff, at) = max_channel_diff(&gpu_px, &cpu.pixels);
    let drawn = pixels_differing_from(&gpu_px, theme.surface);
    assert!(drawn > 0, "GPU 画面不能只有清屏色（说明什么都没画）");
    assert_eq!(
        max_diff, 0,
        "不透明几何必须与 CPU 逐字节相同；最差字节偏移 {at}（像素 {}）",
        at / 4
    );

    // ⑥ 确定性：同一列表重复渲染两次，GPU 与 CPU 都必须逐字节相同
    let gpu_px2 = gpu
        .render(&list)
        .map_err(|e| format!("GPU 第二次渲染失败：{e}"))?;
    assert_eq!(gpu_px, gpu_px2, "GPU 重复渲染必须逐字节相同");
    let cpu2 = cpu_renderer
        .render(extent, &list, theme.surface)
        .map_err(|e| format!("CPU 第二次渲染失败：{e}"))?;
    assert_eq!(cpu.pixels, cpu2.pixels, "CPU 重复渲染必须逐字节相同");

    // ⑦ 额外场景：裁剪 + **嵌套裁剪** + 粗描边（`width` > 边长，边带伸出矩形之外）
    //    同一 extent、同一份手写列表，两边仍必须逐字节相同。
    let mut extra = DrawList::new();
    extra.push(DrawCmd::PushClip { rect: RectI::new(20, 20, 120, 90) });
    extra.push(DrawCmd::FillRoundRect {
        rect: RectI::new(10, 10, 200, 140),
        radius: 24, // 半径远大于半宽/半高：超大半角判据
        color: Color::WHITE,
    });
    extra.push(DrawCmd::PushClip { rect: RectI::new(50, 40, 60, 50) });
    extra.push(DrawCmd::FillRect {
        rect: RectI::new(0, 0, W as i32, H as i32),
        color: Color::rgb(0xE0, 0x30, 0x30),
    });
    extra.push(DrawCmd::PopClip);
    extra.push(DrawCmd::StrokeRect {
        rect: RectI::new(30, 30, 6, 6),
        color: Color::WHITE,
        width: 5, // 带宽 > 边长
    });
    extra.push(DrawCmd::PopClip);

    let extra_gpu = gpu
        .render(&extra)
        .map_err(|e| format!("GPU 渲染额外场景失败：{e}"))?;
    let extra_cpu = cpu_renderer
        .render(extent, &extra, theme.surface)
        .map_err(|e| format!("CPU 渲染额外场景失败：{e}"))?;
    let (extra_diff, extra_at) = max_channel_diff(&extra_gpu, &extra_cpu.pixels);
    assert_eq!(
        extra_diff, 0,
        "裁剪/嵌套裁剪/粗描边场景必须逐字节相同；最差字节偏移 {extra_at}"
    );
    assert!(
        pixels_differing_from(&extra_gpu, theme.surface) > 0,
        "额外场景不该只有清屏色"
    );

    // ⑧ 产物：GPU 图与 CPU 基准图各写一份（可直接用看图工具逐像素比）
    std::fs::create_dir_all("render_out").map_err(|e| format!("建不了 render_out：{e}"))?;
    let gpu_png = deer_gui::gpu::png::encode_rgba(W, H, &gpu_px)?;
    std::fs::write("render_out/gpu_geometry.png", &gpu_png)
        .map_err(|e| format!("写 render_out/gpu_geometry.png 失败：{e}"))?;
    let cpu_png = deer_gui::gpu::png::encode_rgba(W, H, &cpu.pixels)?;
    std::fs::write("render_out/gpu_geometry_cpu.png", &cpu_png)
        .map_err(|e| format!("写 render_out/gpu_geometry_cpu.png 失败：{e}"))?;

    // ⑨ 统计
    println!("画布        : {W}×{H}");
    println!(
        "绘制命令    : {} 条（填充 {} / 圆角 {} / 描边 {} / 裁剪 {} 对 / 文本 {}）",
        list.len(),
        counts.fill_rect,
        counts.fill_round_rect,
        counts.stroke_rect,
        counts.push_clip,
        counts.text
    );
    println!("顶点数      : {}（每帧一个顶点缓冲、一次 draw）", stream.vertices.len());
    println!("适配器      : {}", gpu_adapter_name(adapter));
    println!("非清屏色像素: {drawn} / {}", W as usize * H as usize);
    println!("最大通道差  : {max_diff}（要求 0，逐字节相同）");
    println!("额外场景    : 裁剪 + 嵌套裁剪 + 粗描边（带宽 > 边长）→ 最大通道差 {extra_diff}");
    println!("产物        : render_out/gpu_geometry.png 与 render_out/gpu_geometry_cpu.png");
    println!("\n自检全部通过 ✅（非文本几何 GPU↔CPU 逐字节相同、确定性、产物已写出）");
    Ok(())
}

/// 适配器名（只用于打印；拿不到就退回索引）。
fn gpu_adapter_name(adapter: usize) -> String {
    use deer_gui::gpu::Backend;
    match deer_gui::vk::VkBackend::new() {
        Ok(b) => b
            .adapters()
            .get(adapter)
            .map(|a| format!("{}（index={adapter}）", a.name))
            .unwrap_or_else(|| format!("index={adapter}（枚举不到名字）")),
        Err(e) => format!("index={adapter}（VkBackend::new 失败：{e}）"),
    }
}
