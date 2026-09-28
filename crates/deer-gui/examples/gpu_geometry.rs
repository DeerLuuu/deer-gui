//! 功能示例：把 `DrawList`（**含文本**）交给 Vulkan 画出来，并与 CPU 后端**逐像素对照**（M3a 形状 + M3b 文本）。
//!
//! ```sh
//! cargo run -p deer-gui --example gpu_geometry
//! ```
//!
//! 产物：`render_out/gpu_geometry.png`（GPU 回读）与 `render_out/gpu_geometry_cpu.png`（CPU 基准）——
//! **两块像素缓冲逐字节相同**，连编码后的 PNG 文件也逐字节相同（本示例直接断言 PNG 相等）。
//!
//! 这条链是：`Builder` 建一棵**含文本**的树 → 用 `FontMeasure` 布局 → `build_draw_list` →
//! 同一份 `DrawList` 分别交给 `CpuRenderer::with_text`（真实字形参考实现）与
//! `GpuGeometryRenderer::with_text`（第二条 GPU 管线 + 字形图集纹理），然后逐字节比较。
//!
//! 三条硬前提（照 `crates/deer-vk/tests/gpu_vs_cpu.rs` 的文本对照节）：
//! ① **CPU 侧必须 `with_text`**：`CpuRenderer::new()` 走的是**占位格**模型（0.6em 方块 + i32 截断除法），
//!    与真实字形模型不同，误用它文本对照会全红；
//! ② 颜色必须**全不透明**且附件是线性 `R8G8B8A8_UNORM`（CPU 基准不做 gamma）；
//! ③ 版面必须用**同一个字体、同一个字号**的度量（本示例给 GPU/CPU 各建一个**同源** `TextEngine`）。
//!
//! 环境变量：
//! - `DEER_GPU_ADAPTER`：用第几张显卡（默认 `0`）。
//!
//! 边界（详见 `docs/features/gpu-geometry.md`）：只有**离屏**路径；没有通用 RGBA 纹理、
//! 没有窗口呈现（M3c）、没有批处理优化；`size <= 0` 的文本 GPU 会跳过而 CPU 会画出 1px 字形（**已知有意差异**）。

use std::path::Path;
use std::process::ExitCode;

use deer_gui::prelude::*;
use deer_gui::vk::GpuGeometryRenderer;

/// 画布尺寸（与 GPU 渲染器的离屏 extent 一致）。
const W: u32 = 360;
const H: u32 = 220;
/// 字号（px）：布局、GPU、CPU 三处都必须用它。
const FONT_SIZE: f32 = 16.0;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("gpu_geometry 示例失败：{e}");
            ExitCode::FAILURE
        }
    }
}

/// 建一棵**含文本**的界面树：文本节点 / 按钮 / 输入框都会产生 `DrawCmd::Text`。
fn build_tree() -> Node {
    let mut app = Builder::new(Kind::Column, "app").padding(12.0).gap(8.0);

    app.text("GPU text 真实字形 Hello 你好");

    app.container_opts(Kind::Row, "bar", L::new().pad(10.0).gap(6.0).to_props(), |r| {
        r.button("Apply");
        r.button("Cancel");
    });

    app.container_opts(
        Kind::Column,
        "panel",
        L::new().pad(10.0).gap(6.0).grow(1.0).to_props(),
        |p| {
            p.text("The quick brown fox jumps over the lazy dog.");
            p.field("field hint");
            // 纯几何（对照里同时覆盖形状与文本）
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

/// GPU 侧字形图集纹理的**按线程上传计数**（用来断言「图集没变就不重传」）。
///
/// ⚠️ 是 **`thread_local`** 而不是进程级：本示例的渲染与读取都在 `run()` 所在的
/// 同一个线程上同步完成，所以这里的差值语义成立（见 `deer_vk::device` 里
/// `texture_r8_upload_count()` 的说明）。
fn atlas_upload_count() -> usize {
    deer_gui::vk::device::texture_r8_upload_count()
}

fn run() -> Result<(), String> {
    // ① 主题：**全不透明**（CPU 基准不做 gamma，GPU 附件是线性 UNORM ⇒ 逐字节可比）
    let theme = Theme {
        surface: Color::rgb(0x10, 0x14, 0x24),
        ..Theme::default()
    };
    let extent = Extent { width: W, height: H };

    // ② 字体：找不到就**明确失败**（示例是给人看的，不伪装成功）
    let font_path = deer_gpu::measure::find_system_font()
        .ok_or_else(|| "找不到系统字体（consola.ttf / arial.ttf / segoeui.ttf）——本示例需要真实字体".to_string())?;
    println!("字体        : {}", font_path.display());

    // ③ **同源**的两个引擎：同一字体、同一字号、按同样命令顺序填图集 ⇒ 槽位布局逐字节相同
    let engine_gpu = TextEngine::from_font_file(Path::new(&font_path), FONT_SIZE)
        .map_err(|e| format!("GPU 侧解析字体失败：{e}"))?;
    let engine_cpu = TextEngine::from_font_file(Path::new(&font_path), FONT_SIZE)
        .map_err(|e| format!("CPU 侧解析字体失败：{e}"))?;

    // ④ 建树 → 用**真实字体度量**布局 → 绘制列表（与两个渲染器同一个字号）
    let tree = build_tree();
    let style = TextStyle {
        font_size: FONT_SIZE,
        line_height: theme.line_height,
    };
    let geo = layout(&tree, Rect::new(0.0, 0.0, W as f32, H as f32), style, &engine_gpu.measure());
    let list = deer_gui::gpu::build_draw_list(&tree, &geo, theme.clone(), &engine_gpu.measure());

    let counts = list.counts();
    assert!(counts.text > 0, "这棵树应当产生文本命令（否则本示例没覆盖 M3b）");
    assert!(list.clip_balanced(), "绘制列表的裁剪栈必须平衡");

    // ⑤ GPU：几何管线 + **文本管线**（`with_text`）
    // 数值门槛先 `trim()`：`cmd /c "set DEER_GPU_ADAPTER=1 && …"` 的值是 `"1 "`，
    // 不 trim 则 `parse()` 失败 ⇒ **静默退回适配器 0**（以为在测指定 GPU，实际测的是另一块）。
    let adapter = std::env::var("DEER_GPU_ADAPTER")
        .ok()
        .and_then(|v| v.trim().parse::<usize>().ok())
        .unwrap_or(0);
    let base = GpuGeometryRenderer::new(adapter, extent, theme.surface)
        .map_err(|e| format!("建 GPU 渲染器失败（adapter={adapter}）：{e}"))?;
    let mut gpu = base
        .with_text(engine_gpu)
        .map_err(|e| format!("建 GPU 文本管线失败（with_text）：{e}"))?;
    assert!(gpu.text_enabled(), "with_text 之后文本管线必须已接管");

    let gpu_px = gpu.render(&list).map_err(|e| format!("GPU 渲染失败：{e}"))?;
    assert!(
        gpu.unsupported().is_empty(),
        "文本已被接管 ⇒ 不该有 unsupported：{:?}",
        gpu.unsupported()
    );
    assert_eq!(gpu.text_skipped(), 0, "本示例的文本都应当画出东西（不该有 skipped）");
    assert_eq!(gpu.extent(), extent, "实际渲染尺寸应当等于请求尺寸");

    // ⑥ CPU 基准：**必须 `with_text`**（真实字形路径）
    let mut cpu_renderer = CpuRenderer::with_text(engine_cpu);
    let cpu = cpu_renderer
        .render(extent, &list, theme.surface)
        .map_err(|e| format!("CPU 渲染失败：{e}"))?;

    // ⑦ 硬性判据：不透明内容（形状 + 文本）⇒ 逐字节相同
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
        "不透明内容（含文本）必须与 CPU 逐字节相同；最差字节偏移 {at}（像素 {}）",
        at / 4
    );

    // ⑧ 确定性：同一列表重复渲染两次，GPU 与 CPU 都必须逐字节相同；
    //    并且「图集没变 ⇒ 不重传纹理」（指纹判断的护栏）。
    let uploads_before = atlas_upload_count();
    let gpu_px2 = gpu
        .render(&list)
        .map_err(|e| format!("GPU 第二次渲染失败：{e}"))?;
    let uploads_after = atlas_upload_count();
    assert_eq!(gpu_px, gpu_px2, "GPU 重复渲染必须逐字节相同");
    assert_eq!(
        uploads_after - uploads_before,
        0,
        "同一图集重复渲染不该重传纹理（实际多了 {} 次上传）",
        uploads_after - uploads_before
    );
    let cpu2 = cpu_renderer
        .render(extent, &list, theme.surface)
        .map_err(|e| format!("CPU 第二次渲染失败：{e}"))?;
    assert_eq!(cpu.pixels, cpu2.pixels, "CPU 重复渲染必须逐字节相同");

    // ⑨ 额外场景：裁剪 + **嵌套裁剪** + 粗描边（带宽 > 边长）+ 被裁空的文本
    //    （最后一条在 M3b 里是「跳过并计入 skipped」，不再是 M3a 的假阳性报错）
    let mut extra = DrawList::new();
    extra.push(DrawCmd::PushClip { rect: RectI::new(20, 20, 140, 100) });
    extra.push(DrawCmd::FillRoundRect {
        rect: RectI::new(10, 10, 220, 150),
        radius: 24, // 半径远大于半宽/半高
        color: Color::WHITE,
    });
    extra.push(DrawCmd::Text {
        rect: RectI::new(24, 24, 200, 40),
        text: "clipped text".to_string(),
        color: Color::WHITE,
        size: FONT_SIZE,
        align: 0,
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
    // 整块被裁空的文本：CPU 一个像素都不画，GPU 也不该报错（M3b 假阳性修复）
    extra.push(DrawCmd::PushClip { rect: RectI::new(300, 190, 20, 10) });
    extra.push(DrawCmd::Text {
        rect: RectI::new(0, 0, 40, 20),
        text: "off-clip".to_string(),
        color: Color::WHITE,
        size: FONT_SIZE,
        align: 0,
    });
    extra.push(DrawCmd::PopClip);

    let extra_gpu = gpu
        .render(&extra)
        .map_err(|e| format!("GPU 渲染额外场景失败：{e}"))?;
    assert!(
        gpu.unsupported().is_empty(),
        "被裁空的文本不该报 Unsupported：{:?}",
        gpu.unsupported()
    );
    let extra_cpu = cpu_renderer
        .render(extent, &extra, theme.surface)
        .map_err(|e| format!("CPU 渲染额外场景失败：{e}"))?;
    let (extra_diff, extra_at) = max_channel_diff(&extra_gpu, &extra_cpu.pixels);
    assert_eq!(
        extra_diff, 0,
        "裁剪/嵌套裁剪/粗描边/被裁空文本场景必须逐字节相同；最差字节偏移 {extra_at}"
    );
    assert!(
        pixels_differing_from(&extra_gpu, theme.surface) > 0,
        "额外场景不该只有清屏色"
    );

    // ⑩ 产物：GPU 图与 CPU 基准图各写一份；**两份 PNG 文件必须逐字节相同**
    std::fs::create_dir_all("render_out").map_err(|e| format!("建不了 render_out：{e}"))?;
    let gpu_png = deer_gui::gpu::png::encode_rgba(W, H, &gpu_px)?;
    let cpu_png = deer_gui::gpu::png::encode_rgba(W, H, &cpu.pixels)?;
    assert_eq!(gpu_png, cpu_png, "两份 PNG 文件必须逐字节相同");
    std::fs::write("render_out/gpu_geometry.png", &gpu_png)
        .map_err(|e| format!("写 render_out/gpu_geometry.png 失败：{e}"))?;
    std::fs::write("render_out/gpu_geometry_cpu.png", &cpu_png)
        .map_err(|e| format!("写 render_out/gpu_geometry_cpu.png 失败：{e}"))?;

    // ⑪ 统计
    println!("画布        : {W}×{H}（字号 {FONT_SIZE}px）");
    println!(
        "绘制命令    : {} 条（填充 {} / 圆角 {} / 描边 {} / 裁剪 {} 对 / 文本 {}）",
        list.len(),
        counts.fill_rect,
        counts.fill_round_rect,
        counts.stroke_rect,
        counts.push_clip,
        counts.text
    );
    println!("文本管线    : {}", if gpu.text_enabled() { "已接管（with_text）" } else { "未接管" });
    println!("跳过文本    : {}（空串 / size<=0 / 被裁空 / 图集放不下）", gpu.text_skipped());
    println!("适配器      : {}", gpu_adapter_name(adapter));
    println!("非清屏色像素: {drawn} / {}", W as usize * H as usize);
    println!("最大通道差  : {max_diff}（要求 0，逐字节相同）");
    println!("额外场景    : 裁剪 + 嵌套裁剪 + 粗描边 + 被裁空文本 → 最大通道差 {extra_diff}");
    println!("重复渲染    : 像素逐字节相同、图集**零重传**（上传计数差 0）");
    println!("产物        : render_out/gpu_geometry.png 与 render_out/gpu_geometry_cpu.png（两份文件逐字节相同，{} 字节）", gpu_png.len());
    println!("\n自检全部通过 ✅（形状 + 文本 GPU↔CPU 逐字节相同、确定性、图集不重传、产物已写出）");
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
