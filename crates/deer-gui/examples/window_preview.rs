//! 在**真窗口**里看到 GPU 画面（里程碑 M2b）。
//!
//! ```sh
//! cargo run -p deer-gui --features window --example window_preview
//! ```
//!
//! 会弹出一个窗口，里面是 Vulkan 画的清屏色 + M2a 已验证的几何，连续呈现若干帧后自动退出。
//!
//! | 环境变量 | 作用 |
//! |---|---|
//! | `DEER_WINDOW_FRAMES=N` | 呈现 N 帧后自动退出（默认 120） |
//! | `DEER_WINDOW_HOLD=1` | 一直开着，直到你手动关窗（肉眼看窗口用） |
//! | `DEER_WINDOW_ADAPTER=N` | 用第 N 张显卡（默认 0；本机 0=Intel 集显、1=RTX 5070 Ti） |
//! | `DEER_VK_VALIDATION=1` | 开 Vulkan 校验层（诊断用，消息打到 stderr） |
//!
//! **本轮的边界（别误以为已经能看到界面）**：窗口里只有清屏色与几何；
//! 把节点树渲染出的界面（`DrawList`，含真实字形）送上 GPU 属于 **M3**。
//! 现在想看界面本身，走离屏路径：`cargo run -p deer-gui --example text_render`。

use std::process::ExitCode;
use std::time::Instant;

use deer_gui::gpu::{Color, Extent};
use deer_gui::vk::windowed::{srgb_encoded_byte, FrameOutcome, WindowedRenderer};
use deer_gui::window::{App, Flow, WindowConfig, WindowInfo, run};

/// 清屏色。示例的自检要断言「呈现出去的确实是这个颜色」，所以它是显式常量。
///
/// **注意 sRGB 的坑**：交换链格式通常是 `B8G8R8A8_SRGB`，写入的颜色值在线性空间，
/// 驱动负责 sRGB 编码。所以回读到的字节**不是** `0x10/0x14/0x24`，而是
/// [`srgb_encoded_byte`] 编码后的值（`0x47/0x4F/0x69`）。示例按后者断言 —— 这不是放宽，
/// 而是把「GPU 真的按 sRGB 语义写了附件」这件事也一并验了。
const CLEAR: Color = Color::rgb(0x10, 0x14, 0x24);

/// 每帧是否做像素回读（`DEER_WINDOW_READBACK=0` 可关；关掉后自检会明确降级说明）。
fn readback_enabled() -> bool {
    std::env::var("DEER_WINDOW_READBACK")
        .map(|v| !(v == "0" || v.eq_ignore_ascii_case("false")))
        .unwrap_or(true)
}

/// 呈现帧的像素证据（回读一次，用于断言「窗口里显示的确实是我们要的画面」）。
struct PixelCheck {
    /// 四角实测值（应当是 sRGB 编码后的清屏色）。
    corner: [u8; 4],
    /// 中心实测值（应当落在几何上，与清屏色不同）。
    center: [u8; 4],
    /// 与清屏色不同的像素数 / 总像素数。
    non_clear: usize,
    total: usize,
}

struct Preview {
    renderer: Option<WindowedRenderer>,
    target_frames: u64,
    hold: bool,
    last_extent: Extent,
    presented: u64,
    out_of_date: u64,
    started: Instant,
    summarized: bool,
    readback: bool,
    pixels: Option<PixelCheck>,
}

impl Preview {
    fn new(target_frames: u64, hold: bool, readback: bool) -> Preview {
        Preview {
            renderer: None,
            target_frames,
            hold,
            last_extent: Extent { width: 0, height: 0 },
            presented: 0,
            out_of_date: 0,
            started: Instant::now(),
            summarized: false,
            readback,
            pixels: None,
        }
    }

    /// 回读**刚呈现的那一帧**并核对像素。
    ///
    /// 只在第一帧做一次：回读会强制一次 GPU→CPU 同步 + 每帧 `W×H×4` 的 copy，
    /// 不适合每帧调（详见 `docs/features/vulkan-swapchain.md`）。
    fn check_pixels(&mut self) -> Result<PixelCheck, String> {
        let r = self.renderer.as_mut().ok_or("还没有渲染器")?;
        let (w, h) = (r.extent().width, r.extent().height);
        let px = r
            .read_back_last_frame()
            .map_err(|e| format!("回读呈现帧失败：{e}"))?;
        if px.len() != (w as usize) * (h as usize) * 4 {
            return Err(format!(
                "回读长度 {} 与 {}×{}×4 不符",
                px.len(),
                w,
                h
            ));
        }
        let at = |x: u32, y: u32| -> [u8; 4] {
            let i = ((y as usize) * (w as usize) + (x as usize)) * 4;
            [px[i], px[i + 1], px[i + 2], px[i + 3]]
        };
        // sRGB 附件：写入的是线性值，驱动负责编码 ⇒ 期望值是**编码后**的字节。
        let expect = [
            srgb_encoded_byte(CLEAR.r),
            srgb_encoded_byte(CLEAR.g),
            srgb_encoded_byte(CLEAR.b),
            255,
        ];
        for (label, x, y) in [
            ("左上", 0, 0),
            ("右上", w - 1, 0),
            ("左下", 0, h - 1),
            ("右下", w - 1, h - 1),
        ] {
            let got = at(x, y);
            if got != expect {
                return Err(format!(
                    "{label} ({x},{y}) 期望 sRGB 编码后的清屏色 {expect:?}，实测 {got:?}\
                     —— 说明呈现/回读/通道顺序有问题"
                ));
            }
        }
        let non_clear = px
            .chunks_exact(4)
            .filter(|p| p[0] != expect[0] || p[1] != expect[1] || p[2] != expect[2])
            .count();
        Ok(PixelCheck {
            corner: expect,
            center: at(w / 2, h / 2),
            non_clear,
            total: (w as usize) * (h as usize),
        })
    }

    /// 打印统计并做**自检断言**（不是「跑成功就算」）。
    fn summarize_and_check(&mut self) {
        if self.summarized {
            return;
        }
        self.summarized = true;
        let elapsed = self.started.elapsed().as_secs_f64();
        let fps = if elapsed > 0.0 { self.presented as f64 / elapsed } else { 0.0 };
        println!();
        println!("呈现帧数    : {}", self.presented);
        println!("交换链过期  : {} 次（过期必须重建后重试，不能当成功）", self.out_of_date);
        println!("耗时        : {elapsed:.2} s（{fps:.1} 帧/秒）");
        if let Some(r) = self.renderer.as_mut() {
            println!("最终交换链  : {}×{}", r.extent().width, r.extent().height);
            println!("渲染器计帧  : {}", r.frames_presented());
        }
        match &self.pixels {
            Some(p) => {
                let pct = 100.0 * p.non_clear as f64 / p.total.max(1) as f64;
                println!(
                    "像素回读    : 四角 {:?} = sRGB 编码后的清屏色 rgb({:#04x},{:#04x},{:#04x})",
                    p.corner, CLEAR.r, CLEAR.g, CLEAR.b
                );
                println!(
                    "              中心 {:?}；非清屏色像素 {} / {}（{pct:.2}%）",
                    p.center, p.non_clear, p.total
                );
                assert_eq!(
                    p.corner,
                    [
                        srgb_encoded_byte(CLEAR.r),
                        srgb_encoded_byte(CLEAR.g),
                        srgb_encoded_byte(CLEAR.b),
                        255
                    ],
                    "呈现帧的四角必须是我们要求的清屏色（sRGB 编码后）"
                );
                assert!(
                    p.non_clear > 0,
                    "窗口里必须画出了几何：非清屏色像素为 0 说明只有清屏色"
                );
                println!(
                    "自检通过 ✅（帧数达标、交换链未持续过期、**呈现帧像素已核对**、事件循环正常退出）"
                );
            }
            None => {
                println!("像素回读    : 未做（DEER_WINDOW_READBACK=0）—— **本次没有像素级证据**");
                println!(
                    "自检通过（降级）✅（帧数达标、交换链未持续过期；像素级证据被显式关闭）"
                );
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
        let adapter = std::env::var("DEER_WINDOW_ADAPTER")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
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
        self.last_extent = r.extent();
        self.renderer = Some(r);
        Ok(())
    }

    fn resized(&mut self, width: u32, height: u32) -> Result<(), String> {
        let e = Extent {
            width: width.max(1),
            height: height.max(1),
        };
        self.last_extent = e;
        if let Some(r) = self.renderer.as_mut() {
            r.resize(e).map_err(|err| format!("窗口尺寸变化后重建交换链失败：{err}"))?;
        }
        Ok(())
    }

    fn redraw(&mut self) -> Result<Flow, String> {
        let Some(r) = self.renderer.as_mut() else {
            return Ok(Flow::Exit);
        };
        match r.render_and_present() {
            Ok(FrameOutcome::Presented) => {
                self.presented += 1;
                // 第一帧就取像素证据（只做一次：回读会强制 GPU→CPU 同步）
                if self.readback && self.pixels.is_none() {
                    let p = self.check_pixels()?;
                    self.pixels = Some(p);
                }
            }
            Ok(FrameOutcome::OutOfDate) => {
                // 交换链过期是**正常路径**：重建后重试，绝不能当成功。
                self.out_of_date += 1;
                let e = self.last_extent;
                r.resize(e).map_err(|err| format!("交换链过期后重建失败：{err}"))?;
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
        // 手动关窗也要给出统计（`hold` 模式下这是唯一的收尾点）。
        self.summarize_and_check();
        Flow::Exit
    }
}

fn main() -> ExitCode {
    let target_frames = std::env::var("DEER_WINDOW_FRAMES")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(120)
        .max(1);
    let hold = std::env::var("DEER_WINDOW_HOLD")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    let readback = readback_enabled();

    if hold {
        println!("DEER_WINDOW_HOLD=1：窗口会一直开着，关掉它即退出");
    } else {
        println!("连续呈现 {target_frames} 帧后自动退出（DEER_WINDOW_FRAMES 可改）");
    }
    if readback {
        println!("第一帧会回读像素并核对（DEER_WINDOW_READBACK=0 可关，关掉就没有像素级证据）");
    } else {
        println!("⚠️ 像素回读已关闭（DEER_WINDOW_READBACK=0）—— 自检会降级，不做像素断言");
    }

    let cfg = WindowConfig::new("M2b 窗口预览（GPU 清屏 + 几何）", 960, 600);
    match run(cfg, Preview::new(target_frames, hold, readback)) {
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
