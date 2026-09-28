//! `window_smoke` —— **真开窗口**的冒烟验证（所以它不是 `#[test]`：CI 无桌面会假红）。
//!
//! 跑法（在仓库根目录）：
//!
//! ```text
//! cmd /c "cargo run -q -p deer-window --example window_smoke"
//! ```
//!
//! 默认行为：开一个 320×200 的窗口，**连续重绘 30 帧**后返回 `Flow::Exit`；
//! 然后自检 `redraw` 至少被成功调用 30 次（不是「跑完就算过」），全对才 exit=0。
//!
//! 环境变量：
//!
//! - `DEER_WINDOW_HOLD=1`：不自动退，一直画到手动关窗/Alt+F4 —— 给人肉眼看窗口用。
//! - `DEER_WINDOW_FAIL=1`：`App::init` 故意返回 `Err`，验证「回调出错 ⇒ `run()` 返回 Err」
//!   这条路径真的成立（此时本示例断言 `run()` 必须返回 `Err`，也 exit=0）。
//! - `DEER_WINDOW_FRAMES=<n>`：改自动退的帧数（默认 30）。

use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use deer_window::{App, Flow, WindowConfig, WindowInfo, run};

/// 默认自动退的帧数（任务要求：连续重绘 30 帧）。
const DEFAULT_AUTO_EXIT_FRAMES: u64 = 30;

/// 每多少帧打一行（避免 30 行刷屏）。
const PRINT_EVERY: u64 = 10;

/// hold 模式下每多少帧打一行：Poll 下没有 vsync 节流，帧率极高，打太密没意义。
const PRINT_EVERY_HOLD: u64 = 100;

struct Smoke {
    /// 帧数用 `AtomicU64` 共享：`run()` 拿走了 `App`，`main` 事后还要读它做断言。
    frames: Arc<AtomicU64>,
    /// `Some(n)` = 画满 n 帧就 `Flow::Exit`；`None` = 一直画（hold 模式）。
    target: Option<u64>,
    /// 打日志的间隔。
    print_every: u64,
    /// 让 `init` 失败的开关（验证错误路径）。
    fail_init: bool,
}

impl App for Smoke {
    fn init(&mut self, info: &WindowInfo) -> Result<(), String> {
        println!(
            "[smoke] init：platform={:?} handle(HWND)=0x{:X} display(HINSTANCE)=0x{:X} extent={}x{}",
            info.raw.platform, info.raw.handle, info.raw.display, info.extent.width, info.extent.height
        );
        // 自检：Windows 上句柄/实例句柄必须是真的（0 说明句柄没填对 —— 那必须当失败）。
        if info.raw.handle == 0 {
            return Err("HWND 为 0：原生句柄没填对".to_string());
        }
        if info.raw.display == 0 {
            return Err("HINSTANCE 为 0：原生句柄没填对".to_string());
        }
        if self.fail_init {
            return Err("DEER_WINDOW_FAIL=1：故意让 init 失败（验证错误路径）".to_string());
        }
        Ok(())
    }

    fn resized(&mut self, width: u32, height: u32) -> Result<(), String> {
        println!("[smoke] resized：{width}x{height}");
        Ok(())
    }

    fn redraw(&mut self) -> Result<Flow, String> {
        let n = self.frames.fetch_add(1, Ordering::SeqCst) + 1;
        if n == 1 || n % self.print_every == 0 {
            println!("[smoke] frame {n}");
        }
        match self.target {
            Some(target) if n >= target => Ok(Flow::Exit),
            _ => Ok(Flow::Continue),
        }
    }

    fn close_requested(&mut self) -> Flow {
        println!("[smoke] close_requested：允许关闭");
        Flow::Exit
    }
}

fn main() -> ExitCode {
    // 门槛判定先 `trim()`：`cmd` 的 `set DEER_WINDOW_HOLD=1 && cargo run …` 会把 `&&` 前的
    // 空格也算进变量值 —— `cmd /c "set X=1 && set X"` 实测打印 `X=1 `（**带一个尾空格**）⇒
    // 严格 `== "1"` 会把「设了」判成「没设」（窗口不留 / 故障注入没生效）却照样 exit=0。
    // 同一条实测与更完整的理由见 `crates/deer-gui/src/env_gate.rs`（那边的判定有单测守着）。
    let hold = std::env::var("DEER_WINDOW_HOLD").is_ok_and(|v| v.trim() == "1");
    let fail_init = std::env::var("DEER_WINDOW_FAIL").is_ok_and(|v| v.trim() == "1");
    // 数值门槛同样先 `trim()`：`set DEER_WINDOW_FRAMES=10 && …` 的值是 `"10 "`，
    // 不 trim 则 `parse()` 失败 ⇒ 静默跑默认帧数。
    let auto_exit_frames: u64 = std::env::var("DEER_WINDOW_FRAMES")
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(DEFAULT_AUTO_EXIT_FRAMES);

    println!(
        "[smoke] 启动：hold={hold} fail_init={fail_init} auto_exit_frames={auto_exit_frames} \
         （窗口标题前缀 deer-gui）"
    );

    let frames = Arc::new(AtomicU64::new(0));
    let app = Smoke {
        frames: Arc::clone(&frames),
        target: if hold { None } else { Some(auto_exit_frames) },
        print_every: if hold { PRINT_EVERY_HOLD } else { PRINT_EVERY },
        fail_init,
    };

    let result = run(WindowConfig::new("window_smoke", 320, 200), app);
    let total = frames.load(Ordering::SeqCst);

    // —— 自检：不看「跑完没崩」，看 run() 的返回值与帧数是否都对。 ——
    match (&result, fail_init) {
        (Err(err), true) => {
            println!("[smoke] 预期错误已捕获：{err}");
            println!("[smoke] 自检通过（错误路径）：run() 返回 Err 且 error 文案非空；实际画了 {total} 帧");
            return ExitCode::SUCCESS;
        }
        (Ok(()), true) => {
            eprintln!("[smoke] 自检失败：DEER_WINDOW_FAIL=1 期望 run() 返回 Err，实际返回 Ok");
            return ExitCode::from(1);
        }
        (Err(err), false) => {
            eprintln!("[smoke] 自检失败：run() 返回 Err：{err}");
            return ExitCode::from(1);
        }
        (Ok(()), false) => {}
    }

    if hold {
        println!("[smoke] hold 模式结束（手动关窗）：共画 {total} 帧");
        return ExitCode::SUCCESS;
    }
    if total < auto_exit_frames {
        eprintln!("[smoke] 自检失败：只成功画了 {total} 帧，少于要求的 {auto_exit_frames} 帧");
        return ExitCode::from(1);
    }
    println!("[smoke] 自检通过：run() 返回 Ok，成功重绘 {total} 帧（>= {auto_exit_frames}）");
    ExitCode::SUCCESS
}
