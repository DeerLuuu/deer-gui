//! `input_probe` —— **真开窗口**看输入事件（M5-1 接线的验证用；所以它不是 `#[test]`：
//! CI 无桌面会假红）。
//!
//! 跑法（在仓库根目录）：
//!
//! ```text
//! cmd /c "cargo run -q -p deer-window --example input_probe"
//! ```
//!
//! 默认行为：开一个 480×240 的窗口，跑 **2 秒**（连续重绘、不画东西）后返回
//! `Flow::Exit`，把期间收到的每个输入事件**逐条打印**，最后打一行汇总。窗口拿到焦点时
//! 至少会有一条 `focus true` —— 那一条就是「事件循环 → `InputEvent` → `App::input`」
//! 通电的现场证据。
//!
//! **M5b**：本示例按**墙钟**退，靠的是「一直有帧」才轮得到检查时间 ⇒ 显式声明
//! [`RedrawPolicy::Continuous`]（默认的 `OnDemand` 下这里会一帧都不画、也就永远等不到退出时机）。
//!
//! **人肉验证**（真正看事件长什么样）：
//!
//! ```text
//! cmd /c "set DEER_INPUT_HOLD=1 && cargo run -q -p deer-window --example input_probe"
//! ```
//!
//! 然后动鼠标 / 点键 / 打字：每个事件一行。按 `Esc` 会演示 `App::input` 返回 `Flow::Exit`
//! （Esc ⇒ 事件循环结束）。关窗（点 × / Alt+F4）走 `close_requested`。
//!
//! 环境变量：
//!
//! - `DEER_INPUT_HOLD=1`：不自动退，一直跑到手动关窗/Esc（人肉看事件用）。
//! - `DEER_INPUT_SECONDS=<s>`：改自动退的**墙钟**上限（默认 2.0 秒，浮点数）。
//! - `DEER_INPUT_REQUIRE=1`：**一个事件都没收到就 exit=1**。默认不这样（无桌面/无人操作时
//!   收到 0 个事件是正常的，不该假红）；脚本化验证「通电了」时打开它。
//!
//! 期望看到的分工（M5 的「字符 vs 物理键」约定）：
//!
//! - 敲 `a` ⇒ `key down Char('a')` **和** `text "a"`（两条）；
//! - 敲 `Esc`/`Tab`/`Enter`/`Backspace` ⇒ 只有 `key down ...`（**没有** `text`）；
//! - 按住 Shift 敲 `a` ⇒ `key down Char('A') mods=shift` + `text "A"`；
//! - 中文输入法上屏 ⇒ `text "汉字"`（来自 `Ime::Commit`，没有对应的 `key down`）。

use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use deer_window::{
    App, Flow, InputEvent, Key, Mods, PointerButton, RedrawPolicy, WindowConfig, WindowInfo, run,
};

/// 默认自动退的墙钟上限（秒）。连续重绘下没有 vsync 节流，帧数跑得飞快 ——
/// 按**帧数**退会在 Windows 投递 `Focused`/`CursorMoved` 之前就退出（实测 300 帧 ≈ 0 事件），
/// 所以这里按时间退，才看得到真事件。
const DEFAULT_AUTO_EXIT_SECONDS: f64 = 2.0;

struct Probe {
    /// 事件计数用 `AtomicU64` 共享：`run()` 拿走了 `App`，`main` 事后还要读它做汇总。
    events: Arc<AtomicU64>,
    /// 文本事件里一共收到多少个字符。
    chars: Arc<AtomicU64>,
    /// 画过的帧数（只为打印时给个参照）。
    frames: u64,
    /// 从 `init` 起算：自动退的墙钟基准。
    started: Option<Instant>,
    /// `Some(上限)` = 跑到上限就 `Flow::Exit`；`None` = 一直跑（hold 模式）。
    budget: Option<Duration>,
}

impl Probe {
    /// 打印一条事件（**这就是本示例的全部目的**：把本层模型原样打出来）。
    fn print_event(&self, index: u64, info: &WindowInfo, ev: &InputEvent) {
        let head = format!(
            "[input {index:>3}] extent={}x{}",
            info.extent.width, info.extent.height
        );
        match ev {
            // T3.4 起 `InputEvent` 多了 `ImePreedit`（预编辑）。本探针只打印事件，
            // 预编辑留给 UI 层用 —— 但**穷尽性由编译期强制**：不加这一臂就编不过，
            // 这样「新增事件变体、下游忘了处理」不可能悄悄发生。
            InputEvent::ImePreedit { text } => println!("ImePreedit({text:?})"),
            InputEvent::PointerMoved { x, y } => println!("{head} move      ({x:.1}, {y:.1})"),
            InputEvent::PointerDown { button, x, y } => {
                println!("{head} down      {} ({x:.1}, {y:.1})", button_name(*button));
            }
            InputEvent::PointerUp { button, x, y } => {
                println!("{head} up        {} ({x:.1}, {y:.1})", button_name(*button));
            }
            InputEvent::Wheel { dx, dy } => println!("{head} wheel     dx={dx:.2} dy={dy:.2}"),
            InputEvent::KeyDown { key, mods, .. } => {
                println!("{head} key down  {key:?} mods={}", mods_name(*mods));
            }
            InputEvent::KeyUp { key, mods } => {
                println!("{head} key up    {key:?} mods={}", mods_name(*mods));
            }
            InputEvent::TextInput { text } => {
                // 文本的字符数（不是字节数）：中文一个字 3 字节。
                println!("{head} text      {text:?}（chars={}）", text.chars().count());
            }
            InputEvent::FocusChanged { focused } => println!("{head} focus     {focused}"),
        }
    }
}

impl App for Probe {
    fn init(&mut self, info: &WindowInfo) -> Result<(), String> {
        println!(
            "[input_probe] init：extent={}x{} handle(HWND)=0x{:X}",
            info.extent.width, info.extent.height, info.raw.handle
        );
        if info.raw.handle == 0 {
            return Err("HWND 为 0：原生句柄没填对".to_string());
        }
        self.started = Some(Instant::now());
        Ok(())
    }

    fn resized(&mut self, width: u32, height: u32) -> Result<(), String> {
        println!("[input_probe] resized：{width}x{height}");
        Ok(())
    }

    fn input(&mut self, info: &WindowInfo, ev: &InputEvent) -> Result<Flow, String> {
        let index = self.events.fetch_add(1, Ordering::SeqCst) + 1;
        if let InputEvent::TextInput { text } = ev {
            self.chars.fetch_add(text.chars().count() as u64, Ordering::SeqCst);
        }
        self.print_event(index, info, ev);

        // 演示「输入也能结束事件循环」：Esc ⇒ Exit（与 `redraw` 的 Flow 语义一致）。
        if matches!(ev, InputEvent::KeyDown { key: Key::Escape, .. }) {
            println!("[input_probe] 收到 Esc ⇒ App::input 返回 Flow::Exit");
            return Ok(Flow::Exit);
        }
        Ok(Flow::Continue)
    }

    fn redraw(&mut self) -> Result<Flow, String> {
        self.frames += 1;
        // 不用 let-chain（本 crate 声明的 MSRV 是 1.85）——`zip` + `is_some_and` 一样干净。
        let over_budget = self
            .budget
            .zip(self.started)
            .is_some_and(|(budget, started)| started.elapsed() >= budget);
        if over_budget {
            println!("[input_probe] 到达墙钟上限（frames={}）⇒ Flow::Exit", self.frames);
            return Ok(Flow::Exit);
        }
        Ok(Flow::Continue)
    }

    fn close_requested(&mut self) -> Flow {
        println!("[input_probe] close_requested：允许关闭");
        Flow::Exit
    }

    /// M5b：本示例按墙钟退（要一直有帧才轮得到检查时间）⇒ 显式声明 `Continuous`。
    /// 代价：空闲也烧 CPU（想省电就用默认的 `OnDemand`，见 `--example idle_probe`）。
    fn redraw_policy(&self) -> RedrawPolicy {
        RedrawPolicy::Continuous
    }
}

/// 鼠标按键的可读名（与 [`PointerButton`] 一一对应）。
fn button_name(button: PointerButton) -> &'static str {
    match button {
        PointerButton::Left => "left",
        PointerButton::Right => "right",
        PointerButton::Middle => "middle",
    }
}

/// 修饰键的可读名：一个都没按 ⇒ `-`，否则 `shift+ctrl` 这样。
fn mods_name(mods: Mods) -> String {
    let mut parts = Vec::new();
    if mods.shift {
        parts.push("shift");
    }
    if mods.ctrl {
        parts.push("ctrl");
    }
    if mods.alt {
        parts.push("alt");
    }
    if mods.sup {
        parts.push("sup");
    }
    if parts.is_empty() { "-".to_string() } else { parts.join("+") }
}

fn main() -> ExitCode {
    // 门槛判定先 `trim()`：`cmd` 的 `set DEER_INPUT_HOLD=1 && cargo run …` 会把 `&&` 前的
    // 空格也算进变量值 —— `cmd /c "set X=1 && set X"` 实测打印 `X=1 `（**带一个尾空格**）⇒
    // 严格 `== "1"` 会把「设了」判成「没设」（不留窗 / 不做事件断言）却照样 exit=0。
    // 同一条实测与更完整的理由见 `crates/deer-gui/src/env_gate.rs`（那边的判定有单测守着）。
    let hold = std::env::var("DEER_INPUT_HOLD").is_ok_and(|v| v.trim() == "1");
    let require_events = std::env::var("DEER_INPUT_REQUIRE").is_ok_and(|v| v.trim() == "1");
    // 数值门槛同样先 `trim()`：`set DEER_INPUT_SECONDS=5 && …` 的值是 `"5 "`，
    // 不 trim 则 `parse()` 失败 ⇒ 静默跑默认秒数。
    let auto_exit_seconds: f64 = std::env::var("DEER_INPUT_SECONDS")
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(DEFAULT_AUTO_EXIT_SECONDS);

    println!(
        "[input_probe] 启动：hold={hold} auto_exit_seconds={auto_exit_seconds} require={require_events}\
         （Esc 退出；期望：a ⇒ key down Char('a') + text \"a\"，Esc/Tab/Enter ⇒ 只有 key down）"
    );

    let events = Arc::new(AtomicU64::new(0));
    let chars = Arc::new(AtomicU64::new(0));
    let app = Probe {
        events: Arc::clone(&events),
        chars: Arc::clone(&chars),
        frames: 0,
        started: None,
        budget: if hold { None } else { Some(Duration::from_secs_f64(auto_exit_seconds)) },
    };

    let result = run(WindowConfig::new("input_probe", 480, 240), app);
    let total = events.load(Ordering::SeqCst);
    let total_chars = chars.load(Ordering::SeqCst);

    if let Err(err) = result {
        eprintln!("[input_probe] 失败：run() 返回 Err：{err}");
        return ExitCode::from(1);
    }

    println!("[input_probe] 汇总：收到 {total} 个输入事件（其中文本 {total_chars} 个字符）");
    if total == 0 {
        println!(
            "[input_probe] 没有收到任何输入事件（窗口没拿到焦点，或没人操作）。人肉验证：\n  \
             cmd /c \"set DEER_INPUT_HOLD=1 && cargo run -q -p deer-window --example input_probe\"\n  \
             然后动鼠标 / 按键 / 打字（Esc 结束）。"
        );
        if require_events {
            eprintln!("[input_probe] 自检失败：DEER_INPUT_REQUIRE=1 要求至少一个输入事件，实际 0 个");
            return ExitCode::from(1);
        }
    }
    println!("[input_probe] 自检通过：run() 返回 Ok，input 通路已接通（事件数 {total}）");
    ExitCode::SUCCESS
}
