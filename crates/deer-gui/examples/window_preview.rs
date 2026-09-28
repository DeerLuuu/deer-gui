//! 在**真窗口**里看到 GPU 画的**真实界面树**（形状 + 文本；里程碑 M3c）。
//!
//! ```sh
//! cargo run -p deer-gui --features window --example window_preview
//! ```
//!
//! 与 M2b 的区别：窗口里不再只是「清屏色 + 一个三角形」，而是 `deer-gui` 门面从
//! **节点树**产出的 `DrawList`（面板、按钮、正文文字），逐条命令送上 GPU：
//!
//! ```text
//!  Node 树 ──layout──▶ Geometry ──build_draw_list──▶ DrawList ──▶ WindowedRenderer::draw_and_present
//!                                                                    ├ 形状：gpu_geom::build_stream
//!                                                                    └ 文本：gpu_text::build_text_stream
//! ```
//!
//! | 环境变量 | 作用 |
//! |---|---|
//! | `DEER_VK_FRAMES=N` / `DEER_WINDOW_FRAMES=N` | 呈现 N 帧后自动退出（默认 120；两个名字等价） |
//! | `DEER_WINDOW_HOLD=1` | 一直开着，直到你手动关窗（肉眼看窗口用） |
//! | `DEER_WINDOW_ADAPTER=N` | 用第 N 张显卡（默认 0；本机 0=Intel 集显、1=RTX 5070 Ti） |
//! | `DEER_WINDOW_READBACK=0` | 关掉「呈现帧像素核对」（关掉后自检明确降级，不再有像素证据） |
//! | `DEER_VK_VALIDATION=1` | 开 Vulkan 校验层（诊断用，消息打到 stderr） |
//!
//! ## 本轮的判据边界（重要）
//!
//! 这是 **M3c 的 Step 1**：判据是「界面树真的**呈现到了窗口上**」——
//! `frames_presented > 0` + 呈现帧的像素里**有非清屏色**（证明画出了东西，不只是清屏）。
//!
//! **「窗口像素与 CPU 逐像素一致」不在本示例**：交换链现在优先选**线性** `*_UNORM`
//! （M3c 裁决；本机实测 `0x2c = B8G8R8A8_UNORM`），所以呈现字节与 CPU 帧缓冲可以逐字节比较 ——
//! 那条判据已落在 **`window_parity.rs`**（同目录，`DEER_VK_WINDOW_TESTS=1` 时跑）。
//! 本示例只证明「**上屏**」，两条判据分工不同、都保留。
//!
//! **必须是 example 而不是 `#[test]`**：winit 要求事件循环在**主线程**，而 `cargo test`
//! 在子线程跑测试（证据见 `crates/deer-gui/examples/hal_window_path.rs:8-12`）。

use std::path::Path;
use std::process::ExitCode;
use std::time::Instant;

use deer_gui::gpu::text::TextEngine;
use deer_gui::gpu::{self, Color, Extent, Theme};
use deer_gui::layout::builder::{Builder, L};
use deer_gui::layout::layout;
use deer_gui::layout::layout::TextStyle;
use deer_gui::layout::node::{Kind, Rect};
use deer_gui::vk::windowed::{FrameOutcome, WindowedRenderer};
use deer_gui::window::{App, Flow, WindowConfig, WindowInfo, run};

/// 清屏色：**界面没盖住的地方**就是这个颜色（界面树的根面板会盖住大部分窗口）。
///
/// **颜色空间（M3c 定稿）**：交换链现在优先选**线性** `*_UNORM` 格式（本机实测
/// `0x2c = B8G8R8A8_UNORM`）⇒ 写进去的字节**就是**回读到的字节，不再是 M2b 时代
/// 「sRGB 编码后的值」。
///
/// 为什么必须改成线性：sRGB 附件连**混合**都发生在线性空间，而 CPU 参考实现
/// （`null.rs::blend_cov`）按**字节空间**混合 ⇒ 半透明像素两边差几十字节
/// （实测 `src=0xC0,a=0.5,dst=0`：96 vs 140，差 **44**），
/// 「窗口像素 == CPU 像素」这条判据在 sRGB 附件上**根本不成立**。
const CLEAR: Color = Color::rgb(0x08, 0x09, 0x0C);

/// 线性格式的规范值（`VK_FORMAT_B8G8R8A8_UNORM` / `VK_FORMAT_R8G8B8A8_UNORM`）。
///
/// `init` 里用它断言「实际格式是线性的」—— 本示例的清屏色基准依赖这一点。
const FMT_B8G8R8A8_UNORM: i32 = 0x2c;
const FMT_R8G8B8A8_UNORM: i32 = 0x25;

/// 回读开关（默认**开**，`DEER_WINDOW_READBACK=0` 关）。判据住在 `deer_gui::env_gate`。
///
/// **为什么不能自己手写严格判等**（实测，不是猜的）：`cmd` 的
/// `set DEER_WINDOW_READBACK=0 && cargo run …` 会把 `&&` 前的空格也算进变量值 ——
/// `cmd /c "set X=0 && set X"` 实测打印 `X=0 `（**带一个尾空格**）⇒ 显式关掉回读会被判成
/// 「没关」，像素证据照旧产生（反向的静默偏差）。`trim` 的完整理由见 `env_gate`。
fn readback_enabled() -> bool {
    deer_gui::env_gate::flag_default_true("DEER_WINDOW_READBACK")
}

/// 「跑 N 帧就退出」的帧数。两个环境变量名等价（`DEER_VK_FRAMES` 是本轮验收用的名字）。
fn target_frames_from_env() -> u64 {
    // 数值门槛先 `trim()`：`set DEER_VK_FRAMES=30 && …` 的值是 `"30 "`，
    // 不 trim 则 `parse()` 失败 ⇒ **静默退回默认帧数 120**（跑了几帧与要求的不一致）。
    let read = |k: &str| {
        std::env::var(k)
            .ok()
            .and_then(|v| v.trim().parse::<u64>().ok())
    };
    read("DEER_VK_FRAMES")
        .or_else(|| read("DEER_WINDOW_FRAMES"))
        .unwrap_or(120)
        .max(1)
}

/// 呈现帧的像素证据（回读一次，用于断言「窗口里显示的确实是界面，而不是一片清屏色」）。
struct PixelCheck {
    corner: [u8; 4],
    center: [u8; 4],
    /// 与「清屏色在该附件格式下的期望值」不同的像素数 / 总像素数。
    ///
    /// 线性 `*_UNORM` 附件下就是**直通的清屏色字节**（本机实测 `0x2c`）；
    /// 若格式不是线性的，`init` 里的断言会先失败（本示例的清屏色基准依赖线性附件）。
    non_clear: usize,
    /// **出现最多的那个颜色**（= 界面的底色）以及与之不同的像素数。
    ///
    /// 为什么要第二把尺子：界面树的根面板会盖住整个窗口 ⇒ `non_clear` 会是 100%，
    /// 光靠它无法区分「画了界面」与「只画了一块纯色面板」。所以再数一次
    /// **不等于底色**的像素 —— 那些才是按钮边框、分隔线和**字形**。
    bg: [u8; 4],
    content: usize,
    total: usize,
}

/// 界面树预览。
struct Preview {
    renderer: Option<WindowedRenderer>,
    /// 字体引擎（**调用方持有**，每帧 `&mut` 传给 `draw_and_present`）。
    engine: Option<TextEngine>,
    tree: deer_gui::layout::Node,
    /// 当前尺寸下的绘制列表（尺寸变化时重算）。
    list: Option<deer_gui::gpu::DrawList>,
    last_extent: Extent,
    target_frames: u64,
    hold: bool,
    readback: bool,
    presented: u64,
    out_of_date: u64,
    started: Instant,
    summarized: bool,
    pixels: Option<PixelCheck>,
    /// 上一帧被跳过的文本命令数（空串/被裁空之类；M3b 的假阳性修复在窗口路径上同样生效）。
    skipped_text: usize,
}

impl Preview {
    fn new(target_frames: u64, hold: bool, readback: bool, tree: deer_gui::layout::Node) -> Preview {
        Preview {
            renderer: None,
            engine: None,
            tree,
            list: None,
            last_extent: Extent {
                width: 0,
                height: 0,
            },
            target_frames,
            hold,
            readback,
            presented: 0,
            out_of_date: 0,
            started: Instant::now(),
            summarized: false,
            pixels: None,
            skipped_text: 0,
        }
    }

    /// 按当前窗口尺寸重算**布局 + 绘制列表**（用引擎的真实字体度量，不是近似测量）。
    fn rebuild_list(&mut self, extent: Extent) -> Result<(), String> {
        let engine = self.engine.as_ref().ok_or("还没有字体引擎")?;
        let theme = Theme {
            font_size: engine.font_size(),
            ..Theme::default()
        };
        let style = TextStyle {
            font_size: theme.font_size,
            line_height: theme.line_height,
        };
        let geo = layout::layout(
            &self.tree,
            Rect::new(0.0, 0.0, extent.width as f32, extent.height as f32),
            style,
            &engine.measure(),
        );
        let list = gpu::build_draw_list(&self.tree, &geo, theme, &engine.measure());
        println!(
            "绘制列表    : {} 条命令（形状 {} / 文本 {}）",
            list.len(),
            list.counts().fill_rect
                + list.counts().stroke_rect
                + list.counts().fill_round_rect,
            list.counts().text
        );
        self.list = Some(list);
        self.last_extent = extent;
        Ok(())
    }

    /// 回读**刚呈现的那一帧**并核对像素（只在第一帧做一次：回读会强制 GPU→CPU 同步）。
    fn check_pixels(&mut self) -> Result<PixelCheck, String> {
        let r = self.renderer.as_mut().ok_or("还没有渲染器")?;
        let (w, h) = (r.extent().width, r.extent().height);
        let px = r
            .read_back_last_frame()
            .map_err(|e| format!("回读呈现帧失败：{e}"))?;
        if px.len() != (w as usize) * (h as usize) * 4 {
            return Err(format!("回读长度 {} 与 {}×{}×4 不符", px.len(), w, h));
        }
        let at = |x: u32, y: u32| -> [u8; 4] {
            let i = ((y as usize) * (w as usize) + (x as usize)) * 4;
            [px[i], px[i + 1], px[i + 2], px[i + 3]]
        };
        // 基准 = 「界面没盖上时」应当看到的颜色。
        //
        // 线性 `*_UNORM` 交换链 ⇒ 写入的字节原样出现在回读里（**不再** sRGB 编码）。
        // 这个基准**依赖格式是线性的**，所以 `init` 里有一条 `assert!` 直接钉住它
        // （review Minor-1：此前注释声称有那条断言，实际不存在 —— 现在真的存在）。
        let clear = [CLEAR.r, CLEAR.g, CLEAR.b, 255];
        let non_clear = px
            .chunks_exact(4)
            .filter(|p| p[0] != clear[0] || p[1] != clear[1] || p[2] != clear[2])
            .count();
        // 底色 = 出现最多的颜色（不假设它在哪个像素上）；内容 = 其余像素（边框/文字/控件）
        let mut hist: std::collections::HashMap<[u8; 3], usize> = std::collections::HashMap::new();
        for p in px.chunks_exact(4) {
            *hist.entry([p[0], p[1], p[2]]).or_insert(0) += 1;
        }
        let (bg_rgb, bg_count) = hist
            .iter()
            .max_by_key(|(_, n)| **n)
            .map(|(c, n)| (*c, *n))
            .unwrap_or(([0, 0, 0], 0));
        let bg = [bg_rgb[0], bg_rgb[1], bg_rgb[2], 255];
        Ok(PixelCheck {
            corner: at(0, 0),
            center: at(w / 2, h / 2),
            non_clear,
            bg,
            content: (w as usize) * (h as usize) - bg_count,
            total: (w as usize) * (h as usize),
        })
    }

    /// 打印统计并做**自检断言**。
    fn summarize_and_check(&mut self) {
        if self.summarized {
            return;
        }
        self.summarized = true;
        let elapsed = self.started.elapsed().as_secs_f64();
        let fps = if elapsed > 0.0 {
            self.presented as f64 / elapsed
        } else {
            0.0
        };
        println!();
        println!("呈现帧数    : {}", self.presented);
        println!(
            "交换链过期  : {} 次（过期必须重建后重试，不能当成功）",
            self.out_of_date
        );
        println!("耗时        : {elapsed:.2} s（{fps:.1} 帧/秒）");
        println!("跳过文本    : {} 条（空串/被裁空 ⇒ 跳过而不是报错）", self.skipped_text);
        if let Some(r) = self.renderer.as_mut() {
            println!("最终交换链  : {}×{}", r.extent().width, r.extent().height);
            println!("渲染器计帧  : {}", r.frames_presented());
        }
        match &self.pixels {
            Some(p) => {
                let pct = 100.0 * p.non_clear as f64 / p.total.max(1) as f64;
                println!("像素回读    : 左上角 {:?}；中心 {:?}", p.corner, p.center);
                println!(
                    "              非清屏色像素 {} / {}（{pct:.2}%）",
                    p.non_clear, p.total
                );
                println!(
                    "              界面底色 {:?}；**不等于底色的像素（边框/字形/控件）{} 个**",
                    p.bg, p.content
                );
                assert!(
                    p.non_clear > 0,
                    "窗口里必须真的画出界面：非清屏色像素为 0 说明只有清屏色（界面没上屏）"
                );
                assert!(
                    p.content > 0,
                    "界面不能只是一块纯色面板：不等于底色的像素为 0 ⇒ 形状与字形都没画出来"
                );
                println!(
                    "自检通过 ✅（帧数达标、**界面树已上屏**：非清屏色 {} / {}，其中非底色 {} 个像素是形状与字形）",
                    p.non_clear, p.total, p.content
                );
                println!(
                    "（逐像素与 CPU 对照的判据在 `window_parity` 那个 example：\
                     本示例只证明「界面真的上屏」，不做像素级判定）"
                );
            }
            None => {
                println!("像素回读    : 未做（DEER_WINDOW_READBACK=0）—— **本次没有像素级证据**");
            }
        }
        if self.hold {
            println!("（DEER_WINDOW_HOLD=1，未做「帧数达标」断言 —— 这是给人看窗口的模式）");
            return;
        }
        assert!(
            self.presented >= self.target_frames,
            "实际呈现 {} 帧，少于目标 {} 帧",
            self.presented,
            self.target_frames
        );
        assert!(elapsed > 0.0, "耗时必须为正");
        assert!(fps > 0.0, "帧率必须为正，实际 {fps}");
    }
}

impl App for Preview {
    fn init(&mut self, info: &WindowInfo) -> Result<(), String> {
        // 字体：**找不到就明确失败**（窗口预览的意义就是「看到真实界面」，不许退化成方块）
        let font_path = gpu::measure::find_system_font().ok_or_else(|| {
            "找不到系统字体（consola.ttf / arial.ttf / segoeui.ttf）—— 本示例需要真实字形".to_string()
        })?;
        let font_size = 16.0f32;
        let engine = TextEngine::from_font_file(Path::new(&font_path), font_size)
            .map_err(|e| format!("解析字体失败（{}）：{e}", font_path.display()))?;
        println!("字体        : {}", font_path.display());
        self.engine = Some(engine);

        // 数值门槛先 `trim()`（`set DEER_WINDOW_ADAPTER=1 && …` 的值是 `"1 "`）：
        // 不 trim 则 `parse()` 失败 ⇒ 静默退回适配器 0，等于在测另一块 GPU。
        let adapter = std::env::var("DEER_WINDOW_ADAPTER")
            .ok()
            .and_then(|v| v.trim().parse::<usize>().ok())
            .unwrap_or(0);
        let r = WindowedRenderer::new(adapter, info.raw, info.extent, CLEAR)
            .map_err(|e| format!("创建窗口渲染器失败（adapter={adapter}）：{e}"))?;
        println!("窗口        : {}×{}", info.extent.width, info.extent.height);
        println!("适配器      : {}（index={adapter}）", r.adapter().name);
        println!(
            "交换链      : {}×{} / format {:#010x} / present mode {} / {} 张图",
            r.extent().width,
            r.extent().height,
            r.format(),
            r.present_mode(),
            r.image_count()
        );
        // ★ 本示例的像素基准（「清屏色就是 CLEAR 本身」）**依赖附件是线性的**：
        //   sRGB 附件会把写入值编码后再回读 ⇒ 基准就错了。所以这里直接钉住它，
        //   而不是让基准悄悄错（review Minor-1：此前只写在注释里）。
        let fmt = r.format();
        let linear = fmt == FMT_B8G8R8A8_UNORM || fmt == FMT_R8G8B8A8_UNORM;
        println!(
            "颜色附件    : {fmt:#010x}（{}）",
            if linear { "线性 UNORM ✅" } else { "⚠️ 非线性" }
        );
        assert!(
            linear,
            "本示例要求线性 `*_UNORM` 交换链（实测 {fmt:#010x}）：sRGB 附件会编码回读值，\
             清屏色基准与半透明混合都不再与 CPU 一致。请跑 `window_parity` 看具体差多少，\
             或确认这台机器的 surface 是否真的只有 sRGB 格式。"
        );
        let extent = r.extent();
        self.renderer = Some(r);
        self.rebuild_list(extent)?;
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
        // 尺寸变了 ⇒ 布局与绘制列表都要重算（界面按新窗口尺寸排版）
        self.rebuild_list(e)?;
        Ok(())
    }

    fn redraw(&mut self) -> Result<Flow, String> {
        let Some(r) = self.renderer.as_mut() else {
            return Ok(Flow::Exit);
        };
        let Some(engine) = self.engine.as_mut() else {
            return Ok(Flow::Exit);
        };
        let list = self
            .list
            .clone()
            .ok_or("还没有绘制列表（init/rebuild_list 没成功？）")?;

        match r.draw_and_present(&list, Some(engine)) {
            Ok(FrameOutcome::Presented) => {
                self.presented += 1;
                self.skipped_text = r.ui_text_skipped();
                if self.readback && self.pixels.is_none() {
                    let p = self.check_pixels()?;
                    self.pixels = Some(p);
                }
            }
            Ok(FrameOutcome::OutOfDate) => {
                // 交换链过期是**正常路径**：重建后重试，绝不能当成功。
                self.out_of_date += 1;
                let e = self.last_extent;
                r.resize(e)
                    .map_err(|err| format!("交换链过期后重建失败：{err}"))?;
                // 重建会让界面资源失效（静态 viewport）⇒ 下一帧会自动按新尺寸重建
                return Ok(Flow::Continue);
            }
            Err(e) => return Err(format!("呈现失败：{e}")),
        }
        if !self.hold && self.presented >= self.target_frames {
            self.summarize_and_check();
            return Ok(Flow::Exit);
        }
        Ok(Flow::Continue)
    }

    fn close_requested(&mut self) -> Flow {
        self.summarize_and_check();
        Flow::Exit
    }
}

/// 一棵**普通界面树**：标题 + 一行按钮/输入框 + 几行正文（形状与文字都有）。
fn demo_tree() -> deer_gui::layout::Node {
    let mut app = Builder::new(Kind::Column, "app").padding(20.0).gap(12.0);
    app.text("deer-gui in a real window (M3c)");
    app.container_opts(Kind::Row, "bar", L::new().gap(10.0).to_props(), |r| {
        r.button("Apply");
        r.button("Cancel");
        r.field("field");
        r.button("Quit");
    });
    app.text("Shapes and real glyphs, drawn by two Vulkan pipelines.");
    app.text("The quick brown fox jumps over the lazy dog.");
    app.text("0123456789 +-*/=()[]{} #@!?");
    app.build()
}

/// 窗口预览的入口。
///
/// 本示例**只**证明「真实界面树上屏」（帧数 + 呈现帧里有非底色像素）；
/// **逐像素与 CPU 对照在 `window_parity`**（那才需要线性附件与空间约定）。
fn main() -> ExitCode {
    let target_frames = target_frames_from_env();
    // `DEER_WINDOW_HOLD` 的判定委托给 `deer_gui::env_gate`：`cmd` 的
    // `set DEER_WINDOW_HOLD=1 && …` 的值实测是 `"1 "`（带尾空格），严格判等会把
    // 「设了」判成「没设」⇒ 窗口不留、人肉观察悄悄降级成自动退出。理由见 `env_gate`。
    let hold = deer_gui::env_gate::flag("DEER_WINDOW_HOLD");
    let readback = readback_enabled();

    if hold {
        println!("DEER_WINDOW_HOLD=1：窗口会一直开着，关掉它即退出");
    } else {
        println!("连续呈现 {target_frames} 帧后自动退出（DEER_VK_FRAMES / DEER_WINDOW_FRAMES 可改）");
    }
    if readback {
        println!("第一帧会回读像素并核对（DEER_WINDOW_READBACK=0 可关，关掉就没有像素级证据）");
    } else {
        println!("⚠️ 像素回读已关闭（DEER_WINDOW_READBACK=0）—— 自检降级，不做像素断言");
    }

    let cfg = WindowConfig::new("M3c 窗口预览（真实界面树：形状 + 文本）", 960, 600);
    let preview = Preview::new(target_frames, hold, readback, demo_tree());
    match run(cfg, preview) {
        Ok(()) => {
            println!("窗口事件循环正常退出 ✅");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("窗口预览失败：{e}");
            ExitCode::FAILURE
        }
    }
}
