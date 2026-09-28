//! `wake_probe` —— **真开窗口**量「唤醒面」（M5c 的验收探针；不是 `#[test]`：CI 无桌面会假红）。
//!
//! 跑法（在仓库根目录；先 `cargo build -p deer-window --examples`）：
//!
//! ```text
//! cmd /c "cargo run -q -p deer-window --example wake_probe"
//! ```
//!
//! ## 三档（对照着跑才看得出差别）
//!
//! | 命令 | 期望 |
//! |---|---|
//! | 默认（`DEER_WAKE_TICKS=5` / `=50ms`，推式 `wake_after`） | **恰好 5 次定时唤醒 / 5 个定时帧**；`wants_redraw` **一次都没被问过**；exit=0 |
//! | `set DEER_WAKE_PULL=1 && …` | 同上，但走**拉式** `App::next_deadline()`（`wake_after` 一次都不调） |
//! | `set DEER_WAKE_TICKS=0 && …` | **一声都不叫**（省电档）：事件循环在 `Wait` 上睡死 ⇒ 由**看门狗**兜底收尾；窗口内零唤醒、零 `wants_redraw`；exit=0 |
//! | `set DEER_WAKE_LOOK=20 && …` | 后台线程叫 20 声 `Waker::wake()`；**答真才画** ⇒ 只画 1 帧，其余 19 声记成「唤醒被跳过」 |
//!
//! ## 这一轮的判据为什么必须自带「迭代次数」
//!
//! M5b 立下的省电判据（`wants_redraw` 答真 0 次、画帧 ≤ 1）**抓不住**一种退化：把
//! `ControlFlow::Wait` 换成「1ms 超时」（凭空造超时），或者干脆换成 `Poll`。那种退化
//! **不会多画一帧**（没人 `request_redraw`）⇒ 帧数照旧是 1，只有 CPU 在烧。
//! 所以收尾的窗口层账本里有一个 **`iters`**（事件循环迭代次数），空闲时它应当是**个位数**。
//!
//! ⚠️ **M5c 复审 I1 的更正（本节原文曾把它写成「唯一的可数证据」，那是 overclaim）**：
//! 只把 `Wait` 换成 `Poll` ⇒ `iters` 5 → **11 695 034**、CPU 4.30s/4.5s，而当时
//! **退出码 0、每条判据仍是 ✅** —— 因为**没有任何断言读它**。现在补上了：
//! **空闲档**（`DEER_WAKE_TICKS=0`）通过 [`App::on_wake_stats`] 拿到账本，断言
//! `iters <= IDLE_ITERS_MAX`（= 32，实测基线 5–7）⇒ `Wait => Poll` 这类量级退化**会红**。
//! 本层**刻意不做全局阈值**：`DEER_WINDOW_REDRAW=continuous` 档下 `iters` 与帧数同阶是合法的。
//!
//! ## 前置断言（**0 次唤醒一律判失败**，措辞与「判据失败」分开）
//!
//! 本探针的全部结论都建立在「**唤醒真的到了**」之上：一次都没到 ⇒ 「定时唤醒 ⇒ 一帧」
//! 这条链**一条都没验到**，而 `定时帧=0`、`wants_redraw=0` 这些数字**照样成立**（空转的判据）。
//! 所以「该来的唤醒没来」（看门狗兜底收尾）在**每一档**都显式判失败，并写明是「**前置不成立**」。
//! 本项目已经栽过 4 次「前置不成立 ⇒ 护栏悄悄失效」，这里是第 5 次预防。
//!
//! ## 变异的红/绿（本探针的用途，实测见报告）
//!
//! - **不投递**（`Waker::wake_after` 里不 `send_event`）⇒ 看门狗先到 ⇒ **前置不成立 ⇒ exit=1**；
//! - **无条件投递/无条件重绘**（没 deadline 也 `request_redraw` + 超时）⇒ 这一档的
//!   `定时帧` 会远超 `ticks`、`iters` 暴涨（省电档 `DEER_WAKE_TICKS=0` 尤其明显）。
//! - **`Wait` → `Poll`**（只空转、不多画帧）⇒ 别的判据**全绿**，只有空闲档的
//!   `iters ≤ IDLE_ITERS_MAX` 会红（这是 M5c 复审 I1 指出的那个缺口，现已补上）。
//!
//! 环境变量（数值一律**先 `trim()` 再 `parse()`**：`cmd` 的 `set X=1 && …` 会把空格算进值里）：
//!
//! - `DEER_WAKE_TICKS=<n>`：定时唤醒次数（默认 5；0 = 省电档，一声都不叫）
//! - `DEER_WAKE_MS=<n>`：每次间隔毫秒（默认 50）
//! - `DEER_WAKE_PULL=1`：拉式（`App::next_deadline`）而不是推式（`Waker::wake_after`）
//! - `DEER_WAKE_LOOK=<n>`：后台线程叫 `n` 声 `Waker::wake()`（设了它就只跑这一档）

use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use deer_window::{App, Flow, Waker, WindowConfig, WindowInfo, run};

/// 默认定时唤醒次数。
const DEFAULT_TICKS: u64 = 5;

/// 默认间隔（毫秒）。
const DEFAULT_MS: u64 = 50;

/// 看门狗比预算晚多久醒来：晚这一截 ⇒「看门狗先到」= 事件循环一直没醒的现场证据。
const WATCHDOG_LAG: Duration = Duration::from_millis(1500);

/// 每多少帧打一行（退化时帧数会到几十万 —— 逐帧打会把终端淹了，日志反倒看不见）。
const PRINT_EVERY_FRAMES: u64 = 1000;

/// **空闲档** `iters` 的**上界**（M5c fix 的 I1 落点：`Wait => Poll` 那种空转必须变红）。
///
/// 诚实基线：`DEER_WAKE_TICKS=0` 档实测 **5–7**（含启动期的 `Resized`/`Focused` 等系统批次）
/// ⇒ 上界取 **32**（≈5× 余量）。它抓的是**量级**退化：复审实测 `Wait => Poll` ⇒ **11 695 034**；
/// 「凭空造 1ms 超时」兜底 ⇒ 这一档约 4.5s，也会到**数千**。
/// **已知抓不到的**：周期 > 140ms 的慢超时（32 轮 / 4.5s），以及大量外来输入（每条 +1 轮 ——
/// 那种情况被 `summarize` 里「前置：外来输入」先拦掉，不会伪装成通过）。
const IDLE_ITERS_MAX: u64 = 32;

/// 唤醒面（本探针）的账本：全部 `Atomic`，因为 `run()` 拿走了 `App`，`main`/看门狗事后还要读。
#[derive(Default)]
struct Ledger {
    /// `App::redraw` 真被调用几次（= 真画了几帧）。
    frames: AtomicU64,
    /// 其中由**系统事件**（`Resized` / `Focused`）要来的帧。
    system_frames: AtomicU64,
    /// 其中由**定时唤醒**（deadline 到点）兑现的帧 —— 本档的主判据。
    tick_frames: AtomicU64,
    /// 其中由 **`Waker::wake()` 答真**换来的帧。
    look_frames: AtomicU64,
    /// 其中由**看门狗兜底**（预算到了还没收到唤醒）叫来的收尾帧。
    exit_frames: AtomicU64,
    /// 「系统事件帧」与「到点的预约」**撞在同一帧**的次数（winit 会把并发的请求合成一帧）。
    /// 只影响帧的分类计数，不影响 `tick_frames`（那一帧照样算一次定时唤醒）。
    coincidences: AtomicU64,
    /// `App::wants_redraw` 被问了几次 / 其中答真几次（**只有 `wake()` 会问它**）。
    wants_asked: AtomicU64,
    wants_true: AtomicU64,
    /// 自己排了几次唤醒（推式 = `wake_after` 调用次数；拉式 = 声明 `next_deadline` 的次数）。
    schedules: AtomicU64,
    /// 后台线程叫了几声 `wake()`（App 侧只能靠「被问了几次」间接看到投递）。
    looks_seen: AtomicU64,
    /// 看门狗是否已经兜底（它**先**置这一位、再叫那一声 deadline）。
    watchdog_fired: AtomicBool,
    /// 收尾时 [`App::on_wake_stats`] 报来的 `iters`（窗口层的账本；M5c fix 的 I1 落点）。
    iters: AtomicU64,
    /// [`App::on_wake_stats`] 被调了几次。**前置：必须恰好 1 次** —— 没被调就说明账本没到手，
    /// 上界断言会「拿 0 去比」而**永远绿**（本项目已栽过 4 次「前置不成立 ⇒ 护栏悄悄失效」）。
    stats_calls: AtomicU64,
    /// 窗口收到的**外来输入**条数（本探针**不注入**任何输入 ⇒ 每条都是环境噪声）。
    ///
    /// 它必须被**先判**：复审实测 2 条误击键就会把「`wants_redraw` 被问 0 次」/`seen == looks`
    /// 打成 `判据 ❌`（措辞还指向产品缺陷）。那不是产品缺陷，是测量被污染 ⇒ 判「前置不成立」。
    foreign_inputs: AtomicU64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// 推式：`Waker::wake_after` 排下一次。
    Push,
    /// 拉式：`App::next_deadline` 声明下一次。
    Pull,
    /// `Waker::wake()`（从后台线程叫）。
    Look,
}

#[derive(Clone)]
struct Cfg {
    mode: Mode,
    ticks: u64,
    ms: u64,
    looks: u64,
    /// 看门狗预算（墙钟）。
    budget: Duration,
}

struct Probe {
    ledger: Arc<Ledger>,
    cfg: Cfg,
    /// 建窗后拿到的唤醒句柄（`wake_handle` 里存下来）。
    waker: Option<Waker>,
    /// `Waker` 交给后台「叫醒」线程的通道（Look 档）—— 走通道即**用到**了 `Waker: Send`。
    look_tx: Option<Sender<Waker>>,
    /// 交给看门狗线程的通道（每一档都有）。
    watch_tx: Sender<Waker>,
    /// 推式：本帧之前排下的那个**绝对时刻**（`None` = 没有未兑现的预约）。
    appointment: Option<Instant>,
    /// 拉式：`App::next_deadline` 报出去的时刻。
    declared: Option<Instant>,
    /// 已经画过第一帧（建窗引导帧）。
    boot_done: bool,
    /// 有系统事件（`Resized`/`Focused`）请求过一帧、还没兑现。
    system_pending: bool,
    /// `wake()` 答过一次真（窗口层据此请求了一帧）、那一帧还没来。
    ///
    /// 用 `Cell`：`wants_redraw(&self)` 改不了自己的字段，而「一帧还没兑现」这个标记正是在
    /// 那里置的（与 `wants_redraw` 是 `&self` 这条接口事实和解的方式）。
    look_pending: std::cell::Cell<bool>,
}

impl Probe {
    /// 排下一次定时唤醒（推式调 `wake_after`；拉式只改自己声明的那个时刻）。
    fn schedule_next(&mut self) {
        let n = self.ledger.schedules.fetch_add(1, Ordering::SeqCst) + 1;
        let at = Instant::now() + Duration::from_millis(self.cfg.ms);
        self.appointment = Some(at);
        match self.cfg.mode {
            Mode::Pull => {
                self.declared = Some(at);
                println!("[wake_probe] 第 {n} 次预约（拉式：next_deadline 报了 {}ms 之后）", self.cfg.ms);
            }
            Mode::Push => {
                let waker = self.waker.as_ref().expect("wake_handle 之后才有 waker");
                waker.wake_after(Duration::from_millis(self.cfg.ms));
                println!("[wake_probe] 第 {n} 次预约（推式：wake_after({}ms)，不睡线程）", self.cfg.ms);
            }
            Mode::Look => unreachable!("Look 档不排定时唤醒"),
        }
    }

    /// 兑现一次**定时唤醒**：记一笔、排下一次（**到数了就退** —— 这就是本档的收尾路径）。
    fn tick(&mut self) -> Result<Flow, String> {
        let n = self.ledger.tick_frames.fetch_add(1, Ordering::SeqCst) + 1;
        let late = self.appointment.take().map(|at| at.elapsed());
        self.declared = None;
        println!(
            "[wake_probe] 定时唤醒 {n}/{}：预约迟到 {}（= 事件循环真的睡到那一刻才醒）",
            self.cfg.ticks,
            match late {
                Some(d) => format!("{:.1}ms", d.as_secs_f64() * 1000.0),
                None => "未知".to_string(),
            }
        );
        if n < self.cfg.ticks {
            self.schedule_next();
            Ok(Flow::Continue)
        } else {
            println!("[wake_probe] 定时唤醒到数（{} 次）⇒ 收尾", self.cfg.ticks);
            Ok(Flow::Exit)
        }
    }
}

impl App for Probe {
    fn init(&mut self, info: &WindowInfo) -> Result<(), String> {
        println!(
            "[wake_probe] init：extent={}x{} handle(HWND)=0x{:X}",
            info.extent.width, info.extent.height, info.raw.handle
        );
        if info.raw.handle == 0 {
            return Err("HWND 为 0：原生句柄没填对".to_string());
        }
        Ok(())
    }

    /// M5c：建窗后**调一次** —— 拿到唤醒句柄，并（本档）立刻排第一次定时唤醒。
    fn wake_handle(&mut self, waker: Waker) {
        println!(
            "[wake_probe] wake_handle：拿到唤醒句柄（mode={:?} ticks={} ms={}）",
            self.cfg.mode, self.cfg.ticks, self.cfg.ms
        );
        // 把句柄**送进别的线程**（`Waker: Send` 在这里被真的用上：通道要求 `T: Send`）。
        let _ = self.watch_tx.send(waker.clone());
        if let Some(tx) = &self.look_tx {
            if tx.send(waker.clone()).is_err() {
                println!("[wake_probe] ⚠️ 叫醒线程已经不在了");
            }
        }
        self.waker = Some(waker);
        if self.cfg.ticks > 0 {
            self.schedule_next();
        }
    }

    /// M5c（拉式）：把「下一次希望被叫醒的时刻」报给窗口层。
    fn next_deadline(&self) -> Option<Instant> {
        self.declared
    }

    fn resized(&mut self, width: u32, height: u32) -> Result<(), String> {
        self.ledger.system_frames.fetch_add(0, Ordering::SeqCst); // 帧的归属在 redraw 里数
        self.system_pending = true;
        println!("[wake_probe] resized：{width}x{height}（系统事件 ⇒ 会要一帧，不算唤醒）");
        Ok(())
    }

    fn input(&mut self, _info: &WindowInfo, ev: &deer_window::InputEvent) -> Result<Flow, String> {
        if matches!(ev, deer_window::InputEvent::FocusChanged { .. }) {
            self.system_pending = true;
            println!("[wake_probe] 焦点变化：{ev:?}（系统事件 ⇒ 会要一帧，不算唤醒）");
        } else {
            // 本探针**不注入**输入；真收到输入说明有人在动鼠标/键盘 —— 如实报出来（否则计数会
            // 莫名其妙）。M5c fix：**显式计数**，好让 `summarize` 把它当「前置不成立」**先判**
            // （复审实测：2 条误击键会把与输入相关的判据打成假红，而那不是产品缺陷）。
            let n = self.ledger.foreign_inputs.fetch_add(1, Ordering::SeqCst) + 1;
            println!("[wake_probe] ⚠️ 收到**外来输入**（第 {n} 条；本档不注入输入）：{ev:?}");
        }
        Ok(Flow::Continue)
    }

    /// **M5c fix（复审 I1）**：收尾时窗口层把唤醒账本交过来 ⇒ 存下 `iters`（空闲档的上界判据）。
    fn on_wake_stats(&mut self, stats: &deer_window::WakeStats) {
        self.ledger.stats_calls.fetch_add(1, Ordering::SeqCst);
        self.ledger.iters.store(stats.iters(), Ordering::SeqCst);
        println!(
            "[wake_probe] 收尾账本（App::on_wake_stats）：iters={} wake={} wake_after={} fired={} requested={} skipped={}",
            stats.iters(),
            stats.looks(),
            stats.arms(),
            stats.fired(),
            stats.requested(),
            stats.skipped()
        );
    }

    /// **本档的帧只有三个来源**：建窗引导帧、系统事件、唤醒面。这里按来源分类记账。
    ///
    /// ⚠️ 一帧可能**同时**兑现多个来源：winit 会把并发的 `request_redraw()` 合并成**一条**
    /// `RedrawRequested` ⇒ 不能假设「系统事件」与「到点的预约」会各给一帧。所以这里先算出
    /// 这一帧到底兑现了哪几件事，再分类计数（重合单独记一笔，好让「帧的分类对得上账」这条
    /// 恒等式成立）。
    ///
    /// `wants_redraw` **只有 `wake()` 会问** ⇒ 定时档里它应当一次都没被问过。
    fn wants_redraw(&self) -> bool {
        self.ledger.wants_asked.fetch_add(1, Ordering::SeqCst);
        // Look 档：**攒够 N 声之后才答真一次**（这样「恰好一帧」是确定性的，不靠抢时序）。
        let answer = self.cfg.mode == Mode::Look
            && self.ledger.wants_asked.load(Ordering::SeqCst) >= self.cfg.looks;
        if answer {
            self.ledger.wants_true.fetch_add(1, Ordering::SeqCst);
            // 窗口层会据此请求一帧（那一帧由 redraw 分类处置）。`wants_redraw` 是 `&self`
            // ⇒ 只能用 `Cell` 记这个「一帧还没兑现」的标记（不能改自己的字段）。
            self.look_pending.set(true);
        }
        answer
    }

    fn redraw(&mut self) -> Result<Flow, String> {
        let n = self.ledger.frames.fetch_add(1, Ordering::SeqCst) + 1;
        // 逐帧打印要**节流**：退化时（凭空重绘/超时打转）帧数会到几十万，把终端淹了 ——
        // 上面那行 `unknown` 分支本来就是给这种时候用的，日志反倒看不见就白搭了。
        let loud = n <= 8 || n % PRINT_EVERY_FRAMES == 0;

        // ① 建窗引导帧（一次性，不是空转）—— 不占用任何预约。
        if !self.boot_done {
            self.boot_done = true;
            if loud {
                println!("[wake_probe] frame {n}：建窗引导帧（一次性）");
            }
            return Ok(Flow::Continue);
        }

        // ② 看门狗兜底：它已经置过位 ⇒ 该来的唤醒没来。
        if self.ledger.watchdog_fired.load(Ordering::SeqCst) {
            self.ledger.exit_frames.fetch_add(1, Ordering::SeqCst);
            println!("[wake_probe] frame {n}：**看门狗兜底**的收尾帧（唤醒没到 ⇒ 前置不成立）");
            return Ok(Flow::Exit);
        }

        // ③ 这一帧兑现了哪几件事？
        let tick_due = self.appointment.is_some_and(|at| Instant::now() >= at);
        let look_due = self.look_pending.replace(false);
        let sys = std::mem::take(&mut self.system_pending);
        if sys {
            self.ledger.system_frames.fetch_add(1, Ordering::SeqCst);
        }
        // 这一帧兑现了**几件**事？≥2 件记一次「重合」（好让「帧的分类对得上账」那条恒等式成立）。
        if u64::from(tick_due) + u64::from(look_due) + u64::from(sys) >= 2 {
            self.ledger.coincidences.fetch_add(1, Ordering::SeqCst);
        }
        if loud {
            match (tick_due, look_due, sys) {
                (true, _, true) => println!("[wake_probe] frame {n}：系统事件帧（**同时**是一次到点的预约）"),
                (true, _, false) => println!("[wake_probe] frame {n}：到点的预约兑现"),
                (false, true, true) => {
                    println!("[wake_probe] frame {n}：系统事件帧（**同时**是 wake() 答真换来的）")
                }
                (false, true, false) => println!("[wake_probe] frame {n}：`wake()` 答真换来的帧"),
                (false, false, true) => {
                    println!("[wake_probe] frame {n}：系统事件帧（不看 wants_redraw，不算唤醒）")
                }
                (false, false, false) => {
                    // 没有来源却来了帧 ⇒ 有人凭空请求重绘（这正是要抓的退化的样子）。
                    println!("[wake_probe] frame {n}：⚠️ **没有任何来源**的帧（凭空重绘？）");
                }
            }
        }

        // ④ 分类后的处置：定时唤醒优先（它要排下一次或收尾），其次 wake()，最后才是纯系统帧。
        if tick_due {
            // ⚠️ 定时档（Push/Pull）里 `look_due` 恒为假：那两个档一次都不叫 `wake()`。
            return self.tick();
        }
        if look_due {
            let k = self.ledger.look_frames.fetch_add(1, Ordering::SeqCst) + 1;
            println!("[wake_probe] frame {n}：`wake()` 答真换来的帧（第 {k} 次）⇒ 收尾");
            return Ok(Flow::Exit);
        }
        Ok(Flow::Continue)
    }

    fn close_requested(&mut self) -> Flow {
        println!("[wake_probe] close_requested：允许关闭");
        Flow::Exit
    }
}

/// 从环境变量读一个整数（**先 `trim()` 再 `parse()`**；非法值不 panic，退回默认）。
fn env_u64(name: &str, default: u64, min: u64, max: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|v| *v >= min && *v <= max)
        .unwrap_or(default)
}

/// 打汇总 + 给结论。正常收尾与看门狗兜底共用这一个函数（判据不可能在两个地方漂）。
///
/// `via_dog = true` ⇒ 这次是看门狗硬收尾（连「叫一声立刻到点的 deadline」都没被响应），
/// 那种情况下**窗口层的账本行打不出来**（`process::exit`），所以只能靠 App 侧的数下结论。
fn summarize(ledger: &Ledger, cfg: &Cfg, via_dog: bool) -> bool {
    let frames = ledger.frames.load(Ordering::SeqCst);
    let system = ledger.system_frames.load(Ordering::SeqCst);
    let ticks = ledger.tick_frames.load(Ordering::SeqCst);
    let looks = ledger.look_frames.load(Ordering::SeqCst);
    let exits = ledger.exit_frames.load(Ordering::SeqCst);
    let coinc = ledger.coincidences.load(Ordering::SeqCst);
    let asked = ledger.wants_asked.load(Ordering::SeqCst);
    let wants_true = ledger.wants_true.load(Ordering::SeqCst);
    let schedules = ledger.schedules.load(Ordering::SeqCst);
    let seen = ledger.looks_seen.load(Ordering::SeqCst);
    let iters = ledger.iters.load(Ordering::SeqCst);
    let stats_calls = ledger.stats_calls.load(Ordering::SeqCst);
    let foreign = ledger.foreign_inputs.load(Ordering::SeqCst);

    println!();
    println!(
        "[wake_probe] 汇总（mode={:?} ticks={} ms={} look={}）：画帧={frames} \
         （引导 1｜系统事件={system}｜定时唤醒={ticks}｜wake()答真={looks}｜看门狗收尾={exits}；重合={coinc}）",
        cfg.mode, cfg.ticks, cfg.ms, cfg.looks
    );
    println!(
        "[wake_probe] 预约={schedules}　收到 wake()={seen}　wants_redraw 被问={asked} 答真={wants_true}　收尾={}",
        if via_dog { "看门狗硬收尾（窗口层账本打不出来）" } else { "事件循环自己退" }
    );
    println!(
        "[wake_probe] 窗口层账本（App::on_wake_stats 被调 {stats_calls} 次）：iters={iters}　\
         外来输入={foreign} 条"
    );
    if cfg.looks == 0 && cfg.ticks > 0 {
        println!(
            "[wake_probe] 期望：定时唤醒 {0} 次 ⇒ 定时帧 {0} 个、预约 {0} 次；\
             wants_redraw **一次都不该被问**（deadline 面不看它）",
            cfg.ticks
        );
    } else if cfg.looks > 0 {
        println!(
            "[wake_probe] 期望：叫 {0} 声 wake() ⇒ 被问 {0} 次、只答真 1 次 ⇒ 只画 1 帧（其余记「唤醒被跳过」）",
            cfg.looks
        );
    } else {
        println!("[wake_probe] 期望：**一声都不叫** ⇒ 零唤醒、零 wants_redraw、只有引导帧（+ 系统事件帧）");
    }

    // ===== 前置断言：**测量环境必须是干净的**（M5c fix / 复审 M6） =====
    //
    // 本探针**不注入**任何输入，所以「窗口收到输入」= 有人在动这台机器。复审实测：2 条误击键
    // 就会把 `wants_redraw 被问 0 次` / `seen == looks` 打成 `判据 ❌`（措辞却指向产品缺陷）。
    // **先判它**：措辞用「前置不成立」，与「判据失败」分开 —— 这两种红必须能一眼分开。
    if foreign > 0 {
        eprintln!(
            "[wake_probe] 自检失败（**前置不成立**）：本次窗口收到了 {foreign} 条**外来输入**\
             （本档不注入任何输入 ⇒ 有人在动鼠标/键盘）⇒ 与输入相关的计数（`wants_redraw` 被问\
             {asked} 次、`wake()` 的 seen={seen}）不再可信。**这不是判据失败，也不是产品缺陷** ——\
             请重跑（重跑时别碰窗口）。"
        );
        return false;
    }

    // ===== 前置断言：**这一档必须真的收到过唤醒**（否则下面的判据全是空转的） =====
    if via_dog {
        eprintln!(
            "[wake_probe] 自检失败（**前置不成立**）：看门狗兜底了 —— 连「叫一声立刻到点的 deadline」\
             都没被响应 ⇒ 唤醒面根本没通（`Waker` 的投递/`ControlFlow` 的收敛有一处坏了）。\
             这不是「判据失败」，是**这次压根没验到唤醒**。"
        );
        return false;
    }
    if cfg.looks > 0 {
        if seen == 0 {
            eprintln!(
                "[wake_probe] 自检失败（**前置不成立**）：`Waker::wake()` 一声都没到（被问 {asked} 次）\
                 ⇒ 「答真才画」这条规则一条都没验到。"
            );
            return false;
        }
    } else if cfg.ticks > 0 && ticks == 0 {
        eprintln!(
            "[wake_probe] 自检失败（**前置不成立**）：一次定时唤醒都没到（预约了 {schedules} 次）\
             ⇒ 「deadline 到点 ⇒ 一定画一帧」这条规则**一条都没验到**；此时 `定时帧=0`、\
             `wants_redraw=0` 照样成立 —— 那是空转的判据，不是通过。"
        );
        return false;
    } else if cfg.ticks == 0 && exits == 0 {
        eprintln!(
            "[wake_probe] 自检失败（**前置不成立**）：省电档本该**睡死**到看门狗兜底，\
             却在预算内自己退了（画帧 {frames}）⇒ 有人在凭空唤醒事件循环。"
        );
        return false;
    }

    // ===== 判据（每一档都是可数的等式/不等式；先打印真实数字，再判） =====
    let mut ok = true;
    let mut check = |cond: bool, msg: String| {
        if cond {
            println!("[wake_probe] 判据 ✅ {msg}");
        } else {
            eprintln!("[wake_probe] 判据 ❌ {msg}");
            ok = false;
        }
    };

    if cfg.looks > 0 {
        // 口径说清楚（复审 M6）：`seen` 是**发送侧**（叫醒线程数自己叫了几声）、`asked` 是
        // **接收侧**（窗口层问了 App 几次）—— 两者之间隔一次事件投递，本来就有一个竞态窗口，
        // 所以在**无外来输入**时它们才应当相等（外来输入会先把这一档判成「前置不成立」）。
        check(
            seen == cfg.looks && asked == cfg.looks,
            format!(
                "叫了 {} 声 wake()（发送侧 seen={seen}）⇒ 接收侧 wants_redraw 被问 {asked} 次",
                cfg.looks
            ),
        );
        check(
            wants_true == 1 && looks == 1,
            format!("答真 {wants_true} 次 ⇒ 画了 {looks} 帧（其余 {} 声按「没变化」跳过）", cfg.looks - 1),
        );
        // 帧数的**上下界**先算成变量：判据写「引导 1 + 唤醒 N」（而不是 N），读起来才是那个意思
        // （顺手也避开 clippy 的 `int_plus_one`：它只看字面量 `1 + x` 的形式）。
        let lo = 1 + looks;
        let hi = lo + system;
        check(
            frames >= lo && frames <= hi,
            format!("画帧 {frames} 落在 [引导 1 + wake()答真 {looks}] = {lo} .. + 系统事件 {system} = {hi} 之间（没有凭空多出来的帧）"),
        );
    } else if cfg.ticks > 0 {
        check(
            ticks == cfg.ticks,
            format!("定时唤醒 {ticks} 次 = 要求 {} 次", cfg.ticks),
        );
        check(
            schedules == cfg.ticks,
            format!("预约 {schedules} 次 = 要求 {} 次（第一次在建窗后，其余每帧排下一次）", cfg.ticks),
        );
        check(
            asked == 0 && wants_true == 0,
            format!("wants_redraw 被问 {asked} 次、答真 {wants_true} 次 —— deadline 面**不看**它"),
        );
        check(
            seen == 0,
            format!("没叫过 wake()（收到 {seen} 声）"),
        );
        check(
            exits == 0,
            format!("没有看门狗兜底收尾（{exits} 次）"),
        );
        // 帧数的**上下界**（同 Look 档的理由）：下界 = 建窗引导帧 1 + K 个定时帧。
        let lo = 1 + ticks;
        let hi = lo + system;
        check(
            frames >= lo && frames <= hi,
            format!("画帧 {frames} 落在 [引导 1 + 定时 {ticks}] = {lo} .. + 系统事件 {system} = {hi} 之间（没有凭空多出来的帧）"),
        );
    } else {
        // ===== 空闲档：**唯一**读 `iters` 下断言的地方（M5c fix 的 I1 落点） =====
        //
        // 前置（**显式**，否则护栏会悄悄失效）：账本回调必须真的被调过一次 —— 没被调就说明
        // `iters` 没到手，此时 `iters=0` 会让「≤ 上界」这条**永远绿**（本项目的第 5 次预防）。
        check(
            stats_calls == 1,
            format!(
                "前置：收尾账本回调 App::on_wake_stats 被调 {stats_calls} 次（要求 **1**）\
                 —— 没拿到账本就没法对 iters 下断言"
            ),
        );
        check(
            (1..=IDLE_ITERS_MAX).contains(&iters),
            format!(
                "空闲档事件循环迭代 iters={iters} 落在 [1, {IDLE_ITERS_MAX}]（实测基线 5–7）\
                 —— 退化成空转/超时打转时这里会到百万级（复审实测 Wait⇒Poll = 11 695 034）"
            ),
        );
        check(
            ticks == 0 && looks == 0,
            format!("窗口内零唤醒（定时帧 {ticks}、wake() 帧 {looks}）"),
        );
        check(
            asked == 0 && wants_true == 0,
            format!("窗口内 wants_redraw 被问 {asked} 次、答真 {wants_true} 次"),
        );
        check(
            schedules == 0,
            format!("没有排过任何唤醒（{schedules} 次）"),
        );
        check(
            exits == 1,
            format!("收尾帧来自看门狗兜底（{exits} 次）—— 这正是「事件循环在 Wait 上睡死」的现场证据"),
        );
    }

    // 每一档共用的**恒等式**：每一帧都被归到了某一类（引导 / 定时 / wake() / 系统事件 / 兜底），
    // 同时兑现两件事的帧被记了一次「重合」⇒ 这个式子必须**严格相等**（差一就说明分类漏了）。
    check(
        frames == 1 + ticks + looks + system + exits - coinc,
        format!(
            "帧的分类对得上账：{frames} = 引导 1 + 定时 {ticks} + wake() {looks} + 系统事件 {system} + 兜底 {exits} - 重合 {coinc}"
        ),
    );
    ok
}

fn main() -> ExitCode {
    let ticks = env_u64("DEER_WAKE_TICKS", DEFAULT_TICKS, 0, 1000);
    let ms = env_u64("DEER_WAKE_MS", DEFAULT_MS, 1, 5000);
    let looks = env_u64("DEER_WAKE_LOOK", 0, 0, 1000);
    let pull = std::env::var("DEER_WAKE_PULL").is_ok_and(|v| v.trim() == "1");
    let mode = if looks > 0 {
        Mode::Look
    } else if pull {
        Mode::Pull
    } else {
        Mode::Push
    };
    let ticks = if mode == Mode::Look { 0 } else { ticks };
    // 预算：够跑完所有唤醒 + 一大截余量（余量不足会让看门狗**正常**地先到 ⇒ 假红）。
    let budget = Duration::from_millis(ms * ticks + 3000);
    let cfg = Cfg { mode, ticks, ms, looks, budget };

    println!(
        "[wake_probe] 启动：mode={:?} ticks={ticks} ms={ms} look={looks} pull={pull} \
         预算={}ms（看门狗再晚 {}ms）",
        cfg.mode,
        budget.as_millis(),
        WATCHDOG_LAG.as_millis()
    );

    let ledger = Arc::new(Ledger::default());

    // 后台「叫醒」线程（Look 档）：**阻塞等** `Waker` 从主线程送过来 ——
    // 这一条通道就是 `Waker: Send` 的现场证据（要跨线程走）。
    let (look_tx, look_rx): (Option<Sender<Waker>>, Option<Receiver<Waker>>) = if looks > 0 {
        let (tx, rx) = channel();
        (Some(tx), Some(rx))
    } else {
        (None, None)
    };
    if let Some(rx) = look_rx {
        let ledger = Arc::clone(&ledger);
        let looks = cfg.looks;
        std::thread::spawn(move || {
            let waker = match rx.recv() {
                Ok(w) => w,
                Err(_) => {
                    eprintln!("[wake_probe] 叫醒线程：没等到 Waker（窗口没建起来？）");
                    return;
                }
            };
            // 等引导帧先画完，免得第一声 wake() 与引导帧抢时序。
            std::thread::sleep(Duration::from_millis(400));
            for i in 1..=looks {
                waker.wake();
                ledger.looks_seen.fetch_add(1, Ordering::SeqCst);
                if i == 1 || i == looks {
                    println!("[wake_probe] 叫醒线程：第 {i}/{looks} 声 wake()（不睡线程，只是叫一声）");
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            println!("[wake_probe] 叫醒线程：{looks} 声都叫完了");
        });
    }

    // 看门狗：**只**在「预算到了还没收到唤醒」时兜底。它先置位、再叫一声「立刻到点的 deadline」
    // （不睡线程）—— 那一帧就是收尾帧；连这一声都没人应（唤醒面整体坏了）才硬收尾。
    let (watch_tx, watch_rx) = channel::<Waker>();
    {
        let ledger = Arc::clone(&ledger);
        let cfg = cfg.clone();
        std::thread::spawn(move || {
            let waker = match watch_rx.recv_timeout(Duration::from_secs(10)) {
                Ok(w) => w,
                Err(_) => {
                    eprintln!("[wake_probe] 看门狗：10s 都没等到 Waker（窗口没建起来？）");
                    std::process::exit(1);
                }
            };
            std::thread::sleep(cfg.budget + WATCHDOG_LAG);
            println!("[wake_probe] 看门狗：预算 + {}ms 到了，事件循环还没自己退出", WATCHDOG_LAG.as_millis());
            ledger.watchdog_fired.store(true, Ordering::SeqCst);
            waker.wake_after(Duration::ZERO);
            std::thread::sleep(Duration::from_millis(1500));
            // 走到这里 = 连「立刻到点」那一帧都没来 ⇒ 硬收尾（窗口层账本打不出来）。
            let ok = summarize(&ledger, &cfg, true);
            std::process::exit(if ok { 0 } else { 1 });
        });
    }

    let app = Probe {
        ledger: Arc::clone(&ledger),
        cfg: cfg.clone(),
        waker: None,
        look_tx,
        watch_tx,
        appointment: None,
        declared: None,
        boot_done: false,
        system_pending: false,
        look_pending: std::cell::Cell::new(false),
    };

    let result = run(WindowConfig::new("wake_probe", 400, 200), app);
    if let Err(err) = result {
        eprintln!("[wake_probe] 失败：run() 返回 Err：{err}");
        return ExitCode::from(1);
    }
    let ok = summarize(&ledger, &cfg, false);
    if ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}
