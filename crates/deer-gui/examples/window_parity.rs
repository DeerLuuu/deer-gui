//! **窗口上屏像素 vs CPU 后端**的逐像素对照（M3c Step 2/3 的终局判据）。
//!
//! ```sh
//! DEER_VK_WINDOW_TESTS=1 cargo run -q -p deer-gui --features window --example window_parity
//! ```
//!
//! ## 为什么必须是 example，不能是 `#[test]`
//!
//! winit 要求事件循环在**主线程**，而 `cargo test` 在子线程跑测试
//! （证据见 `crates/deer-gui/examples/hal_window_path.rs:8-12`）。
//! 沿用 `DEER_VK_WINDOW_TESTS=1` 门槛；**没设时打印「这不是通过，是被跳过」**并 exit 0
//! —— 退出码为 0 但**不算通过**，CI 里谁都能看见这句话。
//!
//! ## 判据建立在明确的空间约定上
//!
//! 交换链格式由 `swapchain::pick_config` 选**线性 `*_UNORM`**（M3c 的裁决）：
//! sRGB 附件连**混合**都在线性空间，而 CPU 基准（`null.rs::blend_cov`）按**字节**混合
//! ⇒ 半透明像素两边差几十字节（实测 `src=0xC0,a=0.5,dst=0`：96 vs 140，差 **44**）。
//! 所以本示例**先把实际格式打印出来并断言它是线性的** —— 若这台机器只有 sRGB，
//! 它会**明确失败**而不是拿一个前提不成立的对照冒充结论。
//!
//! | 语料 | 判据 |
//! |---|---|
//! | 不透明界面树（形状 + 真实字形） | **逐字节相同**（`max_diff == 0`） |
//! | 半透明（矩形/文字/圆角各一份 α=0.5） | **≤1 LSB**（CPU `round()` vs GPU UNORM 定点混合），打印实测值 |
//!
//! ## 顺带验一条：`resize` 失效重建不泄漏
//!
//! 反复 `resize` + 每轮画一帧，断言 `live_ui_resource_count() == 1`（存活资源恒一份）
//! 且重建次数与策略一致（动态不必重建、静态必须重建）。这两个计数记在**资源类型自己**
//! 身上（构造 +1 / `Drop` −1），不靠调用方上报 ⇒ 删掉 `Drop` 或漏置 `None` 都会红。

use std::path::Path;
use std::process::ExitCode;

use deer_gui::gpu::null::CpuRenderer;
use deer_gui::gpu::text::TextEngine;
use deer_gui::gpu::{self, Color, DrawCmd, DrawList, Extent, RectI, Theme};
use deer_gui::layout::builder::{Builder, L};
use deer_gui::layout::layout;
use deer_gui::layout::layout::TextStyle;
use deer_gui::layout::node::{Kind, Rect};
use deer_gui::vk::pipelines::ViewportStrategy;
use deer_gui::vk::windowed::{
    live_ui_resource_count, viewport_strategy_from_env, FrameOutcome, WindowedRenderer,
};
use deer_gui::vk::RenderStats;
use deer_gui::window::{App, Flow, WindowConfig, WindowInfo, run};

/// 清屏色（线性 UNORM ⇒ 回读到的字节就是它本身）。
const CLEAR: Color = Color::rgb(0x08, 0x09, 0x0C);

/// 线性格式的规范值（`VK_FORMAT_B8G8R8A8_UNORM` / `VK_FORMAT_R8G8B8A8_UNORM`）。
const FMT_B8G8R8A8_UNORM: i32 = 0x2c;
const FMT_R8G8B8A8_UNORM: i32 = 0x25;

fn window_tests_enabled() -> bool {
    matches!(
        std::env::var("DEER_VK_WINDOW_TESTS").ok().as_deref(),
        Some("1") | Some("true")
    )
}

fn frames_from_env() -> u64 {
    std::env::var("DEER_VK_FRAMES")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(3)
        .max(1)
}

/// **固定**的界面树（形状 + 文本）：语料必须确定，否则「差多少」没有意义。
fn fixed_tree() -> deer_gui::layout::Node {
    let mut app = Builder::new(Kind::Column, "parity").padding(12.0).gap(8.0);
    app.text("window parity (M3c)");
    app.container_opts(Kind::Row, "row", L::new().gap(6.0).to_props(), |r| {
        r.button("OK");
        r.button("No");
        r.field("f");
    });
    app.text("0123456789 ABC xyz");
    app.build()
}

/// 半透明语料 —— **这是那 44 字节的终局检验**：不透明底 + α=0.5 的矩形 + α=0.5 的文字
/// + α=0.5 的圆角矩形，四者都会经过「字节空间 vs 线性空间」这条分岔。
fn semi_transparent_list(extent: Extent) -> DrawList {
    let mut l = DrawList::new();
    l.push(DrawCmd::FillRect {
        rect: RectI::new(0, 0, extent.width as i32, extent.height as i32),
        color: Color::rgb(0x20, 0x30, 0x40),
    });
    l.push(DrawCmd::FillRect {
        rect: RectI::new(10, 10, extent.width as i32 - 20, 30),
        color: Color::rgba(0xC0, 0x60, 0x00, 0.5),
    });
    l.push(DrawCmd::Text {
        rect: RectI::new(10, 6, extent.width as i32 - 20, 30),
        text: "semi".into(),
        color: Color::rgba(0xFF, 0xFF, 0xFF, 0.5),
        size: 14.0,
        align: 0,
    });
    l.push(DrawCmd::FillRoundRect {
        rect: RectI::new(10, 50, 120, 40),
        radius: 6,
        color: Color::rgba(0x00, 0x80, 0xFF, 0.5),
    });
    l
}

/// 一次对照的结果。
struct ParityResult {
    name: &'static str,
    max_diff: u8,
    differing: usize,
    total: usize,
    at: (u32, u32),
    gpu: [u8; 4],
    cpu: [u8; 4],
    /// **每帧**的渲染统计（两次读取的差值 ÷ 帧数；M3+ B1 的验收量）。
    stats_per_frame: RenderStats,
}

/// 两个统计快照的差（都是累计值 ⇒ **差值**才是「这一段」的代价）。
fn stats_delta(before: RenderStats, after: RenderStats) -> RenderStats {
    RenderStats {
        draw_calls: after.draw_calls - before.draw_calls,
        pipeline_switches: after.pipeline_switches - before.pipeline_switches,
        buffer_uploads: after.buffer_uploads - before.buffer_uploads,
        buffer_allocations: after.buffer_allocations - before.buffer_allocations,
    }
}

struct Parity {
    renderer: Option<WindowedRenderer>,
    engine: Option<TextEngine>,
    font_path: Option<std::path::PathBuf>,
    extent: Extent,
    frames: u64,
    done: bool,
}

impl Parity {
    /// 渲染若干帧后回读**最后一帧**，与 CPU 逐像素比。
    fn compare(
        &mut self,
        name: &'static str,
        list: &DrawList,
    ) -> Result<ParityResult, String> {
        let font_path = self
            .font_path
            .clone()
            .ok_or("还没有字体路径（init 没跑？）")?;
        let extent = self.extent;
        let frames = self.frames;
        // CPU 侧用**同源**引擎（同字体同字号）；它自己跟自己是一致的，图集布局不影响结果
        let mut cpu = CpuRenderer::with_text(
            TextEngine::from_font_file(&font_path, 16.0)
                .map_err(|e| format!("解析字体失败：{e}"))?,
        );
        let r = self.renderer.as_mut().ok_or("还没有渲染器")?;
        let engine = self.engine.as_mut().ok_or("还没有字体引擎")?;
        // M3+ B1：统计是**累计值** ⇒ 取画这些帧前后的差值、再除以帧数 = 每帧代价
        let stats_before = r.render_stats();
        for _ in 0..frames {
            match r.draw_and_present(list, Some(engine)) {
                Ok(FrameOutcome::Presented) => {}
                Ok(FrameOutcome::OutOfDate) => {
                    r.resize(extent)
                        .map_err(|e| format!("交换链过期后重建失败：{e}"))?;
                }
                Err(e) => return Err(format!("{name}: 呈现失败：{e}")),
            }
        }
        let gpu = r
            .read_back_last_frame()
            .map_err(|e| format!("{name}: 回读失败：{e}"))?;
        let cpu_fb = cpu
            .render(extent, list, CLEAR)
            .map_err(|e| format!("{name}: CPU 渲染失败：{e}"))?;
        let cpu_px = cpu_fb.to_rgba().to_vec();
        if gpu.len() != cpu_px.len() {
            return Err(format!(
                "{name}: GPU 回读 {} 字节与 CPU {} 字节不一致",
                gpu.len(),
                cpu_px.len()
            ));
        }
        let mut max_diff = 0u8;
        let mut differing = 0usize;
        let mut worst = 0usize;
        for (i, (g, c)) in gpu.iter().zip(cpu_px.iter()).enumerate() {
            let d = g.abs_diff(*c);
            if d > 0 {
                differing += 1;
            }
            if d > max_diff {
                max_diff = d;
                worst = i / 4 * 4;
            }
        }
        let w = extent.width.max(1);
        let px = (worst / 4) as u32;
        let delta = stats_delta(stats_before, r.render_stats());
        let n = frames;
        if n == 0 {
            return Err("帧数必须 ≥ 1".to_string());
        }
        Ok(ParityResult {
            name,
            max_diff,
            differing,
            total: (extent.width as usize) * (extent.height as usize),
            at: (px % w, px / w),
            stats_per_frame: RenderStats {
                draw_calls: delta.draw_calls / n,
                pipeline_switches: delta.pipeline_switches / n,
                buffer_uploads: delta.buffer_uploads / n,
                buffer_allocations: delta.buffer_allocations / n,
            },
            gpu: [gpu[worst], gpu[worst + 1], gpu[worst + 2], gpu[worst + 3]],
            cpu: [
                cpu_px[worst],
                cpu_px[worst + 1],
                cpu_px[worst + 2],
                cpu_px[worst + 3],
            ],
        })
    }

    /// 反复 `resize` + 每轮画一帧 ⇒ 断言「失效重建不泄漏」。
    ///
    /// ## 为什么要先做一次**主动释放**（review I-2）
    ///
    /// `resize()` 只在**静态**策略下销毁界面资源（动态不必重建）⇒ 只跑 resize 的话，
    /// 默认（动态）口径下「析构」这条路径根本不会发生：变异「删掉 `Drop` 里的计数递减」
    /// 在默认口径下**不会变红**（reviewer 实测）。所以这里在 resize 之前**先主动释放一次**
    /// 并断言「存活数掉到 0」—— 两种策略下都真的走到析构，断言才咬得住。
    fn resize_leak_check(&mut self, rounds: u32) -> Result<String, String> {
        let dynamic = viewport_strategy_from_env() == ViewportStrategy::Dynamic;
        let mut engine = self.engine.take().ok_or("还没有字体引擎")?;
        let list = semi_transparent_list(self.extent);

        // ⓪ warm-up 一帧：确保资源已建
        {
            let r = self.renderer.as_mut().ok_or("还没有渲染器（init 没跑？）")?;
            r.draw_and_present(&list, Some(&mut engine))
                .map_err(|e| format!("warm-up 呈现失败：{e}"))?;
        }
        let live_before = live_ui_resource_count();
        if live_before != 1 {
            return Err(format!("warm-up 之后存活界面资源应为 1，实测 {live_before}"));
        }

        // ① **主动释放**：两种策略下都必须真的析构
        let builds_before_release = self
            .renderer
            .as_ref()
            .ok_or("还没有渲染器")?
            .ui_build_count();
        self.renderer
            .as_mut()
            .ok_or("还没有渲染器")?
            .release_ui_resources()
            .map_err(|e| format!("释放界面资源失败：{e}"))?;
        let live_after_release = live_ui_resource_count();
        if live_after_release != 0 {
            return Err(format!(
                "调用 release_ui_resources() 之后存活界面资源应为 0，实测 {live_after_release} \
                 ⇒ **析构/计数没生效**（`UiResources::drop` 或计数递减被删掉了？）"
            ));
        }

        // ② 再画一帧 ⇒ 必须**恰好**重建一次（这是「释放后按需重建」的契约）
        {
            let r = self.renderer.as_mut().ok_or("还没有渲染器")?;
            r.draw_and_present(&list, Some(&mut engine))
                .map_err(|e| format!("释放后重建呈现失败：{e}"))?;
        }
        let builds_after_rebuild = self
            .renderer
            .as_ref()
            .ok_or("还没有渲染器")?
            .ui_build_count();
        if builds_after_rebuild != builds_before_release + 1 {
            return Err(format!(
                "释放后再画一帧应当**恰好**重建 1 次：之前 {}、之后 {}",
                builds_before_release, builds_after_rebuild
            ));
        }
        if live_ui_resource_count() != 1 {
            return Err(format!(
                "重建之后存活数应为 1，实测 {}",
                live_ui_resource_count()
            ));
        }

        // ③ 反复 resize + 每轮画一帧，且**每一轮都查存活数**
        let before = builds_after_rebuild;
        for i in 0..rounds {
            let r = self.renderer.as_mut().ok_or("还没有渲染器")?;
            let w = 640 + (i % 2) * 40;
            let h = 400 + (i % 2) * 40;
            r.resize(Extent {
                width: w,
                height: h,
            })
            .map_err(|e| format!("第 {i} 次 resize 失败：{e}"))?;
            self.extent = r.extent();
            match r.draw_and_present(&list, Some(&mut engine)) {
                Ok(FrameOutcome::Presented) => {}
                Ok(FrameOutcome::OutOfDate) => {
                    let e = self.extent;
                    r.resize(e)
                        .map_err(|err| format!("过期后重建失败：{err}"))?;
                }
                Err(e) => return Err(format!("resize 后呈现失败：{e}")),
            }
            let live = live_ui_resource_count();
            if live != 1 {
                return Err(format!(
                    "第 {i} 次 resize 之后存活数应为 1，实测 {live} ⇒ **资源泄漏**"
                ));
            }
        }
        self.engine = Some(engine);
        let r = self.renderer.as_ref().ok_or("还没有渲染器")?;
        let rebuilt = r.ui_build_count() - before;
        let live = live_ui_resource_count();
        let expect = if dynamic { 0 } else { u64::from(rounds) };
        let msg = format!(
            "资源生命周期：主动释放后存活 0 ✅、释放后重建 1 次 ✅；\
             resize×{rounds}（{} 策略）：重建 {rebuilt} 次（期望 {expect}）、存活界面资源 {live} 份（期望 1）",
            if dynamic { "动态" } else { "静态" }
        );
        if live != 1 {
            return Err(format!("{msg} ⇒ **资源泄漏**：存活数应恒为 1"));
        }
        if rebuilt != expect {
            return Err(format!("{msg} ⇒ 重建次数与策略不符（动态不必重建、静态必须重建）"));
        }
        Ok(msg)
    }
}

impl App for Parity {
    fn init(&mut self, info: &WindowInfo) -> Result<(), String> {
        let font_path = gpu::measure::find_system_font().ok_or_else(|| {
            "找不到系统字体（consola.ttf / arial.ttf / segoeui.ttf）—— 本示例需要真实字形"
                .to_string()
        })?;
        let engine = TextEngine::from_font_file(Path::new(&font_path), 16.0)
            .map_err(|e| format!("解析字体失败（{}）：{e}", font_path.display()))?;
        let adapter = std::env::var("DEER_WINDOW_ADAPTER")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(0);
        let r = WindowedRenderer::new(adapter, info.raw, info.extent, CLEAR)
            .map_err(|e| format!("创建窗口渲染器失败（adapter={adapter}）：{e}"))?;

        // ★ 格式是**运行期事实**：打印 + 断言，不让「选到了什么」成为隐式前提
        let fmt = r.format();
        let linear = fmt == FMT_B8G8R8A8_UNORM || fmt == FMT_R8G8B8A8_UNORM;
        println!(
            "交换链格式  : {fmt:#010x}（{}）",
            if linear {
                "线性 UNORM ✅"
            } else {
                "⚠️ 不是线性"
            }
        );
        if !linear {
            return Err(format!(
                "交换链格式 {fmt:#010x} 不是线性 UNORM（期望 {FMT_B8G8R8A8_UNORM:#x} 或 \
                 {FMT_R8G8B8A8_UNORM:#x}）⇒ 与 CPU 的字节空间混合不一致，逐像素对照不成立。\
                 请把本机实测格式回报控制者。"
            ));
        }
        println!("适配器      : {}", r.adapter().name);
        println!(
            "viewport    : {} 策略",
            if viewport_strategy_from_env() == ViewportStrategy::Dynamic {
                "动态（默认）"
            } else {
                "静态（由 DEER_VK_WINDOW_VIEWPORT 强制）"
            }
        );
        println!(
            "窗口        : {}×{}，每用例渲染 {} 帧后回读最后一帧",
            r.extent().width,
            r.extent().height,
            self.frames
        );
        self.extent = r.extent();
        self.font_path = Some(font_path);
        self.renderer = Some(r);
        self.engine = Some(engine);
        Ok(())
    }

    fn resized(&mut self, width: u32, height: u32) -> Result<(), String> {
        let e = Extent {
            width: width.max(1),
            height: height.max(1),
        };
        if let Some(r) = self.renderer.as_mut() {
            r.resize(e)
                .map_err(|err| format!("窗口尺寸变化后重建交换链失败：{err}"))?;
        }
        self.extent = e;
        Ok(())
    }

    fn redraw(&mut self) -> Result<Flow, String> {
        if self.done {
            return Ok(Flow::Exit);
        }
        self.done = true;

        // ① 语料
        let extent = self.extent;
        let theme = Theme {
            font_size: 16.0,
            ..Theme::default()
        };
        let tree = fixed_tree();
        let style = TextStyle {
            font_size: theme.font_size,
            line_height: theme.line_height,
        };
        let opaque_list = {
            let engine = self.engine.as_ref().ok_or("还没有字体引擎")?;
            let geo = layout::layout(
                &tree,
                Rect::new(0.0, 0.0, extent.width as f32, extent.height as f32),
                style,
                &engine.measure(),
            );
            gpu::build_draw_list(&tree, &geo, theme.clone(), &engine.measure())
        };
        let semi_list = semi_transparent_list(extent);

        // ② 对照
        let mut results = vec![
            self.compare("opaque-ui-tree", &opaque_list)?,
            self.compare("semi-transparent", &semi_list)?,
        ];

        // ③ resize 失效重建不泄漏
        let leak = self.resize_leak_check(4)?;

        // ④ 汇总 + 断言
        println!();
        println!("—— 窗口上屏像素 vs CPU（同一份 `DrawList`、同一个清屏色）——");
        for r in &results {
            println!(
                "  {}: 最大通道差 {}，不同像素 {} / {}，最差在 ({}, {}) GPU={:?} CPU={:?}",
                r.name, r.max_diff, r.differing, r.total, r.at.0, r.at.1, r.gpu, r.cpu
            );
            println!(
                "      每帧统计（M3+ B1）：draw_calls {} / pipeline_switches {} / buffer_uploads {} / buffer_allocations {}",
                r.stats_per_frame.draw_calls,
                r.stats_per_frame.pipeline_switches,
                r.stats_per_frame.buffer_uploads,
                r.stats_per_frame.buffer_allocations
            );
        }
        println!("  {leak}");
        let opaque = results.remove(0);
        let semi = results.remove(0);
        println!(
            "结论：不透明最大通道差 **{}**（要求 0）；半透明最大通道差 **{}**（要求 ≤1）",
            opaque.max_diff, semi.max_diff
        );
        if opaque.max_diff != 0 {
            return Err(format!(
                "不透明界面树必须**逐字节相同**：实测最大通道差 {} @({}, {}) GPU={:?} CPU={:?}",
                opaque.max_diff, opaque.at.0, opaque.at.1, opaque.gpu, opaque.cpu
            ));
        }
        if semi.max_diff > 1 {
            return Err(format!(
                "半透明语料最大通道差 {} > 1 LSB @({}, {}) GPU={:?} CPU={:?} \
                 —— 差到几十就说明附件/混合空间不是字节空间（退回 sRGB 了？）",
                semi.max_diff, semi.at.0, semi.at.1, semi.gpu, semi.cpu
            ));
        }

        // ★ **M3+ B5-2 的终局判据**：形状 + 文本合流 ⇒ 每帧 **1 draw + 1 switch**。
        //
        //   | 量 | 基线（两条管线） | 统一后（这里断言） |
        //   |---|---|---|
        //   | `draw_calls` | 8 | **1** |
        //   | `pipeline_switches` | 8 | **1** |
        //
        //   ## 前置条件（**显式断言** —— 本项目纪律：前置不成立不会报错）
        //
        //   1. 这条语料必须**同时**含形状与文本，否则「1 draw」平凡成立；
        //   2. 每帧的统计必须**是整数**：`compare()` 用「N 帧增量 ÷ N」算每帧值，
        //      若顶点数据每帧都变（B3 不生效）会让 uploads 抖动，但 draw/switch 恒为 1；
        //   3. `buffer_uploads` 只在**第一帧**是 1（之后内容不变 ⇒ 跳过），
        //      所以这里**不**断言 uploads 的每帧值（它依赖帧数，不是本判据）。
        let shapes_and_text = opaque_list.cmds.iter().any(|c| {
            matches!(
                c,
                DrawCmd::FillRect { .. }
                    | DrawCmd::FillRoundRect { .. }
                    | DrawCmd::StrokeRect { .. }
            )
        }) && opaque_list
            .cmds
            .iter()
            .any(|c| matches!(c, DrawCmd::Text { .. }));
        if !shapes_and_text {
            return Err(format!(
                "前置条件不成立：`opaque-ui-tree` 语料必须**同时**含形状与文本命令\
                 （实测形状={} 文本={}）—— 否则「1 draw + 1 switch」平凡成立，判据是空转的",
                opaque_list
                    .cmds
                    .iter()
                    .filter(|c| matches!(
                        c,
                        DrawCmd::FillRect { .. }
                            | DrawCmd::FillRoundRect { .. }
                            | DrawCmd::StrokeRect { .. }
                    ))
                    .count(),
                opaque_list
                    .cmds
                    .iter()
                    .filter(|c| matches!(c, DrawCmd::Text { .. }))
                    .count()
            ));
        }
        if opaque.stats_per_frame.draw_calls != 1 || opaque.stats_per_frame.pipeline_switches != 1 {
            return Err(format!(
                "`opaque-ui-tree` 每帧必须是 **1 draw + 1 switch**（B5-2 统一管线的收益；\
                 基线是 8/8）：实测 draw_calls {} / pipeline_switches {}",
                opaque.stats_per_frame.draw_calls, opaque.stats_per_frame.pipeline_switches
            ));
        }
        println!(
            "统一管线计数 ✅：`opaque-ui-tree` 每帧 draw_calls {} / pipeline_switches {}（基线 8/8）",
            opaque.stats_per_frame.draw_calls, opaque.stats_per_frame.pipeline_switches
        );
        println!(
            "窗口 parity 通过 ✅（不透明逐字节相同；半透明 ≤1 LSB；统一管线 1 draw + 1 switch；\
             resize 重建无泄漏；校验零消息由外层命令核对）"
        );
        Ok(Flow::Exit)
    }

    fn close_requested(&mut self) -> Flow {
        Flow::Exit
    }
}

fn main() -> ExitCode {
    if !window_tests_enabled() {
        println!("⚠️ **这不是通过，是被跳过**：没有设 DEER_VK_WINDOW_TESTS=1");
        println!("要跑窗口 parity：");
        println!(
            "  DEER_VK_WINDOW_TESTS=1 cargo run -q -p deer-gui --features window --example window_parity"
        );
        return ExitCode::SUCCESS;
    }
    let frames = frames_from_env();
    let cfg = WindowConfig::new("M3c 窗口 parity（上屏像素 vs CPU）", 960, 600);
    let app = Parity {
        renderer: None,
        engine: None,
        font_path: None,
        extent: Extent {
            width: 0,
            height: 0,
        },
        frames,
        done: false,
    };
    match run(cfg, app) {
        Ok(()) => {
            println!("窗口事件循环正常退出 ✅");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("窗口 parity 失败：{e}");
            ExitCode::FAILURE
        }
    }
}
