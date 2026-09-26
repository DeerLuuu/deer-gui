//! **HAL 窗口路径的端到端验证**（M2b）。
//!
//! ```sh
//! $env:DEER_VK_WINDOW_TESTS='1'; cargo run -q -p deer-gui --features window --example hal_window_path
//! ```
//!
//! ## 为什么这是示例而不是 `#[test]`
//!
//! 独立验证者发现：`crates/deer-vk/src/hal.rs` 的 `VulkanFrame::submit_and_present` /
//! `VulkanDevice::create_swapchain` 在仓库里**零自动化覆盖**（把 `OutOfDate` 映射成 `Presented`
//! 时全仓测试无一变红）。补覆盖本来该写成测试，但 **winit 要求事件循环在主线程**，
//! 而 `cargo test` 的 harness 在子线程里跑每个测试 ⇒ 只能在**示例**里驱动它。
//!
//! 于是覆盖被拆成两半（都在 `cargo test` / 门禁里）：
//! 1. **映射契约**：`deer-vk/src/hal.rs` 的 `present_result_of` 有纯函数单测
//!    （把 `OutOfDate` 改成 `Presented` 会立刻变红）—— 那部分在 `cargo test` 里守着；
//! 2. **真实路径**：本示例（真窗口 + 真交换链 + 真提交/呈现）—— 由门禁命令跑。
//!
//! 不设 `DEER_VK_WINDOW_TESTS=1` 时**显式跳过并说明**（不是伪装通过）。
//!
//! ## 覆盖的边界
//! - `VkBackend::open(0)` → 真设备；`create_swapchain(window, extent, format)` → 真交换链；
//! - `begin_frame` + `record(空列表)` ✅ / `record(含绘制命令)` ⇒ **明确报 M3**（不许静默忽略）；
//! - `submit_and_present` → `PresentResult`（`Presented` 计数；过期如实上报，绝不当成功）；
//! - `wait_idle` 干净收尾。

use std::process::ExitCode;

use deer_gui::gpu::{Backend, Color, Device, DrawCmd, DrawList, PresentResult, RectI, TargetFormat};
use deer_gui::vk::VkBackend;
use deer_gui::window::{App, Flow, WindowConfig, WindowInfo, run};

fn enabled() -> bool {
    std::env::var("DEER_VK_WINDOW_TESTS")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

struct HalPath {
    device: Option<Box<dyn Device>>,
    target_frames: u64,
    presented: u64,
    out_of_date: u64,
    m3_boundary_checked: bool,
    waited_idle: bool,
}

impl HalPath {
    fn new(target_frames: u64) -> HalPath {
        HalPath {
            device: None,
            target_frames,
            presented: 0,
            out_of_date: 0,
            m3_boundary_checked: false,
            waited_idle: false,
        }
    }
}

impl App for HalPath {
    fn init(&mut self, info: &WindowInfo) -> Result<(), String> {
        let backend = VkBackend::new().map_err(|e| format!("VkBackend::new 失败：{e}"))?;
        let mut device = backend
            .open(0)
            .map_err(|e| format!("VkBackend::open(0) 失败：{e}"))?;
        let sc = device
            .create_swapchain(info.raw, info.extent, TargetFormat::Bgra8Srgb)
            .map_err(|e| format!("Device::create_swapchain 失败：{e}"))?;
        println!(
            "[hal] 适配器      : {}",
            device.info().name
        );
        println!(
            "[hal] 交换链      : {}×{} / 格式 {:?}",
            sc.extent().width,
            sc.extent().height,
            sc.format()
        );
        self.device = Some(device);
        Ok(())
    }

    fn redraw(&mut self) -> Result<Flow, String> {
        let Some(device) = self.device.as_mut() else {
            return Ok(Flow::Exit);
        };

        // ① 边界：空列表可以记录；含真实绘制命令必须**明确报 M3**
        if !self.m3_boundary_checked {
            let mut frame = device
                .begin_frame()
                .map_err(|e| format!("begin_frame 失败：{e}"))?;
            frame
                .record(&DrawList::new())
                .map_err(|e| format!("空绘制列表应当可以记录，却报错：{e}"))?;
            let mut real = DrawList::new();
            real.push(DrawCmd::FillRect {
                rect: RectI::new(0, 0, 8, 8),
                color: Color::WHITE,
            });
            let err = frame
                .record(&real)
                .expect_err("含绘制命令的列表必须明确报 Unsupported（不许静默忽略）");
            let msg = err.to_string();
            assert!(msg.contains("M3"), "报错要指明是 M3，实际：{msg}");
            self.m3_boundary_checked = true;
            println!("[hal] 边界        : record(空)=OK / record(FillRect)=Unsupported(M3) ✅");
        }

        // ② 提交并呈现
        let frame = device
            .begin_frame()
            .map_err(|e| format!("begin_frame 失败：{e}"))?;
        match frame
            .submit_and_present()
            .map_err(|e| format!("submit_and_present 失败：{e}"))?
        {
            PresentResult::Presented => self.presented += 1,
            // 过期是正常路径：如实计数（HAL 没有暴露「按当前尺寸重建」的入口，
            // 上层应当持有 Swapchain 调 resize —— 见 window_preview 的做法）
            PresentResult::OutOfDate => self.out_of_date += 1,
        }

        if self.presented >= self.target_frames {
            device.wait_idle().map_err(|e| format!("wait_idle 失败：{e}"))?;
            self.waited_idle = true;
            return Ok(Flow::Exit);
        }
        Ok(Flow::Continue)
    }

    fn close_requested(&mut self) -> Flow {
        Flow::Exit
    }
}

fn main() -> ExitCode {
    if !enabled() {
        println!("跳过：HAL 窗口路径需要真实窗口（设 DEER_VK_WINDOW_TESTS=1 启用）。");
        println!("本次**没有**验证 HAL 的 create_swapchain / submit_and_present —— 这不是通过。");
        return ExitCode::SUCCESS;
    }

    let target: u64 = std::env::var("DEER_HAL_FRAMES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(30);
    println!("HAL 窗口路径：目标呈现 {target} 帧");

    let cfg = WindowConfig::new("HAL 路径端到端（M2b）", 640, 480);
    match run(cfg, HalPath::new(target)) {
        Ok(()) => {
            println!("HAL 路径跑完 ✅（边界检查已执行、wait_idle 已调用）");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("HAL 窗口路径失败：{e}");
            ExitCode::FAILURE
        }
    }
}
