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
//! | `shapes-only`（**只有**形状，且**先跑**） | **逐字节相同**；`set 0` 指着 **1×1 哑纹理** |
//! | `opaque-ui-tree`（形状 + 真实字形） | **逐字节相同**（`max_diff == 0`）；`set 0` 指着**字形图集**；每帧 **1 draw + 1 switch** |
//! | `semi-transparent`（矩形/文字/圆角各一份 α=0.5） | **≤1 LSB**（CPU `round()` vs GPU UNORM 定点混合），打印实测值 |
//!
//! ## B5-3 补齐的两组读数（复审 F-9：窗口侧原本一个调用点都没有）
//!
//! 1. **`set 0` 的真实指向**（`WindowedRenderer::bound_texture_size()`）——
//!    「统一片元着色器**无条件采样** ⇒ 形状帧也必须绑一张有效纹理」这条**静默依赖**，
//!    在真正的验收语料（960×600 界面树）上原先**没有判据**。现在两条都断言：
//!    `shapes-only` 跑完必须是 `(1,1)`、`opaque-ui-tree` 跑完必须是图集（> 1×1）；
//! 2. **CPU 侧成本口径**（`unify_call_count` / `unify_output_vertex_count`）——
//!    每帧恰好 1 次 `unify`、顶点数 > 0，且带文本的语料**多于**只有形状的语料
//!    （多出来的就是字形四边形 ⇒ 证明文本段真的进了统一顶点流）。
//!    这两条读数在报告里是**可打印的数字**，而不是一句「很便宜」。
//!
//! ## 顺带验一条：`resize` 失效重建不泄漏
//!
//! 反复 `resize` + 每轮画一帧，断言 `live_ui_resource_count() == 1`（存活资源恒一份）
//! 且重建次数与策略一致（动态不必重建、静态必须重建）。这两个计数记在**资源类型自己**
//! 身上（构造 +1 / `Drop` −1），不靠调用方上报 ⇒ 删掉 `Drop` 或漏置 `None` 都会红。
//!
//! ⚠️ **不许与 `cargo test` 并发跑在同一个 `target/` 上**：复审实测过并发时退出码不可信
//! （抢 build 锁 + 抢同一块 Intel GPU）。串行跑并各留完整日志。
//!
//! ## 重绘策略（M5b）：本示例**不需要额外帧**，显式声明 `OnDemand`
//!
//! 判据全部在**一次** `App::redraw` 调用里做完：3 个语料各自在 `compare()` 内部
//! `for _ in 0..frames` 呈现 N 帧、回读最后一帧再与 CPU 逐像素比。也就是说——
//!
//! - 它**不靠事件循环续帧**：`OnDemand`（M5b 的新默认，省电）下只消耗建窗引导帧那 1 帧；
//! - 它**不靠墙钟**：`DEER_VK_FRAMES` 数的是 `compare()` 内部的呈现次数，帧数一改，
//!   被平均的每帧统计（`stats / n`）跟着改，但结论（最大通道差）不变；
//! - M5b-A 曾把它列为「按帧退出 ⇒ 不退出」，**实测不成立**：本文件在 `OnDemand` 下
//!   一次就退（M5b-A2 实测 `exit=0`、0.93 s、`frames=1 requests=4 skipped=0`，
//!   见 `.superpowers/sdd/m5b-a2-report.md`）。所以这里**不**声明 `Continuous`
//!   —— 那是假的（本示例没有连续重绘的需求）。
//!
//! 验收的可数证据：`[deer-window] 重绘账本：… frames=1`（帧供给 = 1 次引导帧）。

use std::path::Path;
use std::process::ExitCode;

use deer_gui::gpu::null::CpuRenderer;
use deer_gui::gpu::{self, Extent, Theme};
use deer_gui::layout::builder::{Builder, L};
use deer_gui::layout::layout;
use deer_gui::layout::layout::TextStyle;
use deer_gui::layout::node::{Kind, Rect};
// LY1/LY2：L0 类型来自 `deer-core`；文本栈来自 L1 crate `deer-text`。
use deer_core::{Color, DrawCmd, DrawList, RectI};
use deer_text::measure::find_system_font;
use deer_text::TextEngine;
use deer_gui::vk::pipelines::ViewportStrategy;
use deer_gui::vk::windowed::{
    live_ui_resource_count, viewport_strategy_from_env, FrameOutcome, WindowedRenderer,
};
use deer_gui::vk::RenderStats;
use deer_gui::window::{App, Flow, RedrawPolicy, WindowConfig, WindowInfo, run};

/// 清屏色（线性 UNORM ⇒ 回读到的字节就是它本身）。
const CLEAR: Color = Color::rgb(0x08, 0x09, 0x0C);

/// 线性格式的规范值（`VK_FORMAT_B8G8R8A8_UNORM` / `VK_FORMAT_R8G8B8A8_UNORM`）。
const FMT_B8G8R8A8_UNORM: i32 = 0x2c;
const FMT_R8G8B8A8_UNORM: i32 = 0x25;

/// 门槛判定（`DEER_VK_WINDOW_TESTS`）。判据住在 `deer_gui::env_gate`（有单测守着）。
///
/// **为什么不能自己手写严格 `== "1"`**（实测，不是猜的）：`cmd` 的
/// `set DEER_VK_WINDOW_TESTS=1 && cargo run …` 会把 `&&` 前的空格也算进变量值 ——
/// `cmd /c "set X=1 && set X"` 实测打印 `X=1 `（**带一个尾空格**）⇒ 本示例会被判成
/// 「没设门槛」，于是明明建了窗口、读回并对照完像素、exit=0，却打印
/// 「这不是通过，是被跳过」，**与事实相反**。`trim` 的完整理由见 `env_gate`。
fn window_tests_enabled() -> bool {
    deer_gui::env_gate::flag("DEER_VK_WINDOW_TESTS")
}

/// **非法值 ⇒ 报错**（不静默退回默认帧数）。
///
/// ## 口径统一（复审 MEDIUM-1）
///
/// 同一个 `DEER_VK_FRAMES` 原先在两处口径**相反**：testkit
/// （`testing::window::frames_from_env`）对非法值**报错**，而这里 `.unwrap_or(3)` **静默退回 3**；
/// `docs/features/testing.md:144` 又断言「解析不出来就报错、**不静默**」⇒ 该断言对这个消费者不成立。
/// 现在**两处同向：非法即报错**。理由：帧数是**度量的一部分**（跑了几帧决定对比多少次），
/// 「你要求 30 帧、我悄悄跑了 3 帧还报绿」正是本项目反复吃过的那类不可复现结论。
fn frames_from_env() -> Result<u64, String> {
    // 数值门槛同样先 `trim()`：`set DEER_VK_FRAMES=3 && …` 的值是 `"3 "`，不 trim 会误判非法。
    let raw = match std::env::var("DEER_VK_FRAMES") {
        Ok(v) => v,
        Err(_) => return Ok(3), // 没设 ⇒ 默认 3 帧（这是**默认值**，不是「静默吞掉非法值」）
    };
    match raw.trim().parse::<u64>() {
        Ok(n) if n >= 1 => Ok(n),
        Ok(n) => Err(format!(
            "DEER_VK_FRAMES = {n} 非法：至少要 1 帧（原值 {raw:?}）"
        )),
        Err(_) => Err(format!(
            "DEER_VK_FRAMES = {raw:?} 不是合法帧数（要正整数）。\
             ⚠️ 本示例**不静默退回默认值**（与 testkit 同口径，复审 MEDIUM-1）"
        )),
    }
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

/// **只有形状、没有文本**的语料：哑纹理那条契约的窗口侧判据（见 `compare` 的 `with_text`）。
///
/// 为什么单独一条：B5-2 的统一片元着色器**无条件采样** ⇒ 形状帧也必须绑一张有效纹理；
/// 而「绑的是 1×1 哑纹理」只有在**这个渲染器从没上传过图集**时才成立 ——
/// 所以这条语料必须**排在**带文本的语料**之前**跑（见 `redraw` 的调用顺序）。
fn shapes_only_list(extent: Extent) -> DrawList {
    let mut l = DrawList::new();
    l.push(DrawCmd::FillRect {
        rect: RectI::new(0, 0, extent.width as i32, extent.height as i32),
        color: Color::rgb(0x20, 0x30, 0x40),
    });
    l.push(DrawCmd::FillRect {
        rect: RectI::new(8, 8, 60, 24),
        color: Color::rgb(0xC0, 0x40, 0x20),
    });
    l.push(DrawCmd::FillRoundRect {
        rect: RectI::new(8, 40, 80, 30),
        radius: 6,
        color: Color::rgb(0x30, 0x90, 0x40),
    });
    l.push(DrawCmd::StrokeRect {
        rect: RectI::new(100, 40, 60, 30),
        width: 2,
        color: Color::rgb(0xE0, 0xE0, 0xE0),
    });
    l
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
    /// **每帧**的 `unify` 调用次数（B5-3：窗口路径的 CPU 成本读数，复审 F-9 的缺口）。
    /// 这里用的是 `WindowedRenderer::unify_call_count` 的差值 ÷ 帧数。
    unify_calls_per_frame: u64,
    /// **每帧** `unify` 输出的统一顶点数（= 这一帧 CPU 搬运的顶点数）。
    unify_vertices_per_frame: u64,
    /// 画完这些帧之后，`set 0` **指着**哪张纹理（宽, 高）——
    /// `WindowedRenderer::bound_texture_size()`，即真实描述符指向。
    bound_texture: (u32, u32),
}

/// 两个统计快照的差（都是累计值 ⇒ **差值**才是「这一段」的代价）。
fn stats_delta(before: RenderStats, after: RenderStats) -> RenderStats {
    RenderStats {
        draw_calls: after.draw_calls - before.draw_calls,
        pipeline_switches: after.pipeline_switches - before.pipeline_switches,
        buffer_uploads: after.buffer_uploads - before.buffer_uploads,
        buffer_allocations: after.buffer_allocations - before.buffer_allocations,
        // M3+ 第 4 项下半新增的判据（间接绘制 / 索引 / 提交）：窗口路径也要能看见
        // —— 少了这几个字段，那个 struct 字面量就编译不过（这本身就是「计数被删就会红」）。
        submits: after.submits - before.submits,
        indirect_draws: after.indirect_draws - before.indirect_draws,
        index_uploads: after.index_uploads - before.index_uploads,
        indirect_uploads: after.indirect_uploads - before.indirect_uploads,
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
    ///
    /// ## `with_text`：这条语料要不要把文本引擎交给窗口渲染器
    ///
    /// - `false` ⇒ 传 `None`（语料里**不能有** `Text` 命令，否则渲染器会按 M3a 行为报
    ///   `Unsupported` —— 那是契约，不是缺陷）。此时渲染器**从没上传过图集** ⇒
    ///   `set 0` 必须指着 1×1 哑纹理（断言在 `redraw` 里）；
    /// - `true` ⇒ 传 `Some(engine)`，图集会被上传、`set 0` 改指到它。
    fn compare(
        &mut self,
        name: &'static str,
        list: &DrawList,
        with_text: bool,
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
        let mut engine = self.engine.as_mut();
        // M3+ B1：统计是**累计值** ⇒ 取画这些帧前后的差值、再除以帧数 = 每帧代价。
        // B5-3 新增的两组读数（`unify` 调用/顶点数、描述符指向）同样用**差值**。
        let stats_before = r.render_stats();
        let unify_calls_before = r.unify_call_count();
        let unify_verts_before = r.unify_output_vertex_count();
        for _ in 0..frames {
            let text_arg = if with_text {
                Some(engine.as_deref_mut().ok_or("还没有字体引擎")?)
            } else {
                None
            };
            match r.draw_and_present(list, text_arg) {
                Ok(FrameOutcome::Presented) => {}
                Ok(FrameOutcome::OutOfDate) => {
                    r.resize(extent)
                        .map_err(|e| format!("交换链过期后重建失败：{e}"))?;
                }
                Err(e) => return Err(format!("{name}: 呈现失败：{e}")),
            }
        }
        let bound_texture = r.bound_texture_size();
        let unify_calls = r.unify_call_count() - unify_calls_before;
        let unify_verts = r.unify_output_vertex_count() - unify_verts_before;
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
                submits: delta.submits / n,
                indirect_draws: delta.indirect_draws / n,
                index_uploads: delta.index_uploads / n,
                indirect_uploads: delta.indirect_uploads / n,
            },
            unify_calls_per_frame: unify_calls / n,
            unify_vertices_per_frame: unify_verts / n,
            bound_texture,
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
        let font_path = find_system_font().ok_or_else(|| {
            "找不到系统字体（consola.ttf / arial.ttf / segoeui.ttf）—— 本示例需要真实字形"
                .to_string()
        })?;
        let engine = TextEngine::from_font_file(Path::new(&font_path), 16.0)
            .map_err(|e| format!("解析字体失败（{}）：{e}", font_path.display()))?;
        // 数值门槛先 `trim()`（`set DEER_WINDOW_ADAPTER=1 && …` 的值是 `"1 "`）：
        // 不 trim 则 `parse()` 失败 ⇒ 静默退回适配器 0，等于在测另一块 GPU。
        let adapter = std::env::var("DEER_WINDOW_ADAPTER")
            .ok()
            .and_then(|v| v.trim().parse::<usize>().ok())
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
        // M5b：帧供给**数得出来**（别只看退出码）。本示例整段判据就在这一次 `redraw` 里跑完，
        // 每语料 `frames` 帧的呈现发生在 `compare()` 内部 ⇒ 事件循环只要给 1 帧（建窗引导帧）。
        // 收尾的账本行应当是 `frames=1`（由 `deer-window` 打），验收 grep 它。
        println!(
            "帧供给      : 本示例只要 1 帧（建窗引导帧）—— 每语料 {} 帧的呈现在 compare() 内部完成，\
             不靠事件循环续帧、也不看墙钟",
            self.frames
        );

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
        let shapes_only = shapes_only_list(extent);

        // ★ 前置条件（**显式查**）：`shapes_only` 语料里**不能**有 `Text` 命令 ——
        //   它要求窗口渲染器从没上传过图集（下面断言 `set 0` 指着 1×1 哑纹理）。
        //   若将来有人往这条语料里加了文字，本判据会**静默失效**（图集一上传，
        //   哑纹理断言就没意义了）⇒ 所以先查再跑。
        let shapes_only_has_text = shapes_only
            .cmds
            .iter()
            .any(|c| matches!(c, DrawCmd::Text { .. }));
        if shapes_only_has_text {
            return Err(
                "前置条件不成立：`shapes-only` 语料必须**不含**文本命令 —— 否则图集会被上传，\
                 「形状帧绑 1×1 哑纹理」那条断言就不再指向哑纹理（判据静默失效）"
                    .to_string(),
            );
        }
        let shapes_only_has_shape = shapes_only.cmds.iter().any(|c| {
            matches!(
                c,
                DrawCmd::FillRect { .. }
                    | DrawCmd::FillRoundRect { .. }
                    | DrawCmd::StrokeRect { .. }
            )
        });
        if !shapes_only_has_shape {
            return Err("前置条件不成立：`shapes-only` 语料里必须有形状命令".to_string());
        }

        // ② 对照（**顺序有意义**：`shapes-only` 必须第一个跑 —— 那时还没上传过图集）
        let mut results = vec![
            self.compare("shapes-only", &shapes_only, false)?,
            self.compare("opaque-ui-tree", &opaque_list, true)?,
            self.compare("semi-transparent", &semi_list, true)?,
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
            // ★ M3+ 第 4 项下半：**间接绘制**的可数证据（窗口路径与离屏同一套计数）。
            //   `indirect_draws` 必须等于 `draw_calls`（每一次绘制都走 indirect）；
            //   稳态下 `buffer_allocations`/`index_uploads` 应该都是 0（跨帧复用）。
            println!(
                "      每帧间接绘制（M3+ 第 4 项下半）：submits {} / indirect_draws {} / \
                 index_uploads {} / indirect_uploads {}",
                r.stats_per_frame.submits,
                r.stats_per_frame.indirect_draws,
                r.stats_per_frame.index_uploads,
                r.stats_per_frame.indirect_uploads
            );
            // ★ B5-3：窗口路径的 CPU 侧读数（`unify` 调用/顶点数）+ 描述符**真实指向**
            println!(
                "      每帧 CPU 侧（B5-2 口径）：unify 调用 {} 次 / 输出顶点 {} 个；\
                 画完后 `set 0` 指着 {}×{}（1×1 = 哑纹理，>1 = 字形图集）",
                r.unify_calls_per_frame,
                r.unify_vertices_per_frame,
                r.bound_texture.0,
                r.bound_texture.1
            );
        }
        println!("  {leak}");
        let shapes_only = results.remove(0);
        let opaque = results.remove(0);
        let semi = results.remove(0);
        println!(
            "结论：形状-only 最大通道差 **{}**（要求 0）；\
             不透明最大通道差 **{}**（要求 0）；半透明最大通道差 **{}**（要求 ≤1）",
            shapes_only.max_diff, opaque.max_diff, semi.max_diff
        );
        if shapes_only.max_diff != 0 {
            return Err(format!(
                "只有形状的语料也必须**逐字节相同**：实测最大通道差 {} @({}, {}) GPU={:?} CPU={:?}",
                shapes_only.max_diff,
                shapes_only.at.0,
                shapes_only.at.1,
                shapes_only.gpu,
                shapes_only.cpu
            ));
        }
        // ★ **哑纹理契约（窗口侧）**：从没上传过图集的渲染器，`set 0` 必须指着 1×1 哑纹理。
        //   这是复审 F-9 指出的缺口 —— 「形状帧也必须绑一张有效纹理」这条**静默依赖**
        //   原本只在离屏的合成语料上有读数，而真正的验收语料（960×600 界面树）上没有。
        if shapes_only.bound_texture != (1, 1) {
            return Err(format!(
                "`shapes-only` 画完后 `set 0` 必须指着 **1×1 哑纹理**（统一 FS 无条件采样 ⇒ \
                 形状帧也必须有一张有效纹理），实测 {}×{} —— 说明描述符集被改指到了别的纹理，\
                 或者读数不是从描述符真实指向来的（见 `bound_texture_size` 的文档）",
                shapes_only.bound_texture.0, shapes_only.bound_texture.1
            ));
        }
        // ★ **图集契约（窗口侧）**：接管了 `TextEngine` 的渲染器画完带文本的语料后，
        //   `set 0` 必须**已经改指到字形图集**（不是哑纹理）。真实图集的宽 = 字号 ≥ 2
        //   ⇒ 宽高都 > 1 是可靠判据（与 `(1,1)` 不可能混淆）。
        if opaque.bound_texture.0 <= 1 || opaque.bound_texture.1 <= 1 {
            return Err(format!(
                "`opaque-ui-tree`（含真实字形）画完后 `set 0` 必须指着**字形图集**，\
                 实测 {}×{} —— 仍是 1×1 说明描述符集**没有**被改指到图集\
                 （`refresh_ui_atlas_texture` 的改指被跳过了？）",
                opaque.bound_texture.0, opaque.bound_texture.1
            ));
        }
        // ★ **CPU 侧读数必须言之有物**：每帧恰好一次 `unify`、且搬运的顶点数 > 0。
        for r in [&shapes_only, &opaque] {
            if r.unify_calls_per_frame != 1 {
                return Err(format!(
                    "`{}`: 每帧必须恰好调用一次 `unify`（一帧一次合流），实测 {}",
                    r.name, r.unify_calls_per_frame
                ));
            }
            if r.unify_vertices_per_frame == 0 {
                return Err(format!(
                    "`{}`: 每帧 `unify` 输出的统一顶点数必须 > 0（空帧才允许为 0）\
                     —— 否则这条读数什么都没测",
                    r.name
                ));
            }
        }
        // ★ 形状与文本**都要真的进到统一顶点流里**（否则「CPU 侧口径」只覆盖了一半）：
        //   带文本的语料每帧搬运的顶点数必须**多于**只有形状的语料（多出的就是字形四边形）。
        if opaque.unify_vertices_per_frame <= shapes_only.unify_vertices_per_frame {
            return Err(format!(
                "`opaque-ui-tree` 每帧的统一顶点数（{}）必须**多于** `shapes-only`（{}）\
                 —— 否则说明文本段没进统一顶点流（或 shapes-only 语料意外含了文本）",
                opaque.unify_vertices_per_frame, shapes_only.unify_vertices_per_frame
            ));
        }
        println!(
            "统一管线 CPU 侧 ✅：`shapes-only` {} 顶点/帧、`opaque-ui-tree` {} 顶点/帧\
             （各 1 次 unify）；`set 0` 分别指向 {}×{}（哑纹理）与 {}×{}（字形图集）",
            shapes_only.unify_vertices_per_frame,
            opaque.unify_vertices_per_frame,
            shapes_only.bound_texture.0,
            shapes_only.bound_texture.1,
            opaque.bound_texture.0,
            opaque.bound_texture.1
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
            "窗口 parity 通过 ✅（不透明与 shapes-only 逐字节相同；半透明 ≤1 LSB；\
             统一管线 1 draw + 1 switch；`set 0` 指向被断言（哑纹理 / 字形图集）；\
             CPU 侧 unify 读数 > 0；resize 重建无泄漏；校验零消息由外层命令核对）"
        );
        Ok(Flow::Exit)
    }

    fn close_requested(&mut self) -> Flow {
        Flow::Exit
    }

    /// **M5b：显式声明「我不需要连续帧」**（判据全在一次 `redraw` 里跑完，见模块文档）。
    ///
    /// 显式写出来（而不是靠默认值）是为了让「本使用方的重绘意图」在代码里可读：
    /// 判据类示例**不**靠帧数、**不**靠墙钟 ⇒ 声明 `OnDemand`（省电，M5b 的默认）。
    /// `DEER_WINDOW_REDRAW=continuous` 只把帧供给打开（第一帧就 `Flow::Exit`，
    /// 像素结论必须**一字不变** —— 实测两档的 `最大通道差` 完全相同）。
    fn redraw_policy(&self) -> RedrawPolicy {
        RedrawPolicy::OnDemand
    }

    /// **M5b：本示例与输入无关** —— 没有任何一条输入会改变要画的东西（它不看窗口输入），
    /// 所以一律不请求重绘。显式写出来表明意图（默认值也是 `false`）。
    fn wants_redraw(&self) -> bool {
        false
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
    let frames = match frames_from_env() {
        Ok(n) => n,
        Err(e) => {
            eprintln!("❌ {e}");
            return ExitCode::from(2);
        }
    };
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
