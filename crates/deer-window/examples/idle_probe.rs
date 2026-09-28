//! `idle_probe` —— **真开窗口**量「省电模式到底省没省」（M5b 的验收探针；不是 `#[test]`：
//! CI 无桌面会假红）。
//!
//! 跑法（在仓库根目录）：
//!
//! ```text
//! cmd /c "cargo run -q -p deer-window --example idle_probe"
//! ```
//!
//! 默认行为：开一个 400×200 的窗口，声明 **`OnDemand`**（省电），**不画东西**，跑
//! `DEER_IDLE_SECONDS`（默认 4 秒）后汇总退出。汇总里是**数得出来**的四个数：
//! 收到多少条输入、其中几条改了状态、`wants_redraw` 被问了几次/答真几次、真画了几帧。
//!
//! ## 三个用法（对照着跑才看得出差别）
//!
//! | 命令 | 期望 |
//! |---|---|
//! | `cargo run -q -p deer-window --example idle_probe` | **省电**：动鼠标/滚轮不产生重绘 ⇒ 画帧数停在引导帧那 1 帧，`wants_redraw` 答真 0 次 |
//! | `set DEER_IDLE_DIRTY=1 && cargo run … --example idle_probe` | **变化探针**：把每条输入都当「改了状态」⇒ 每条输入请求一次重绘，画帧数与输入条数相当 |
//! | `set DEER_WINDOW_REDRAW=continuous && cargo run … --example idle_probe` | **关掉省电**：与 App 声明无关地强制连续重绘 ⇒ 画帧数暴涨（空闲也烧 CPU） |
//!
//! 三种模式的**输入条数**要靠人肉动鼠标（脚本人肉驱动见报告里的实测记录）。
//!
//! ## ⚠️ 前置：本次**必须真的收到过输入**（「省电」与「变化」两档在 0 输入时**一律 exit=1**）
//!
//! 本探针的全部判据都建立在**「`run()` 确实派发过输入」**之上：`wants_redraw` 是**每条输入之后**
//! 被问一次的，一条输入都没有 ⇒ 它**一次都没被问过** ⇒ `答真=0`、`画帧=1` 与「省电」**完全无法
//! 区分**。也就是说：这个前置不成立时，下面的判据不是「变弱」而是**空转** —— 任何变异
//! （例如「无条件请求重绘」）都能顶着它绿过去。
//!
//! 所以「省电」与「变化」两档在**输入 = 0** 时**显式判失败**（`exit=1`，stderr 写明是「前置不成立」
//! 而不是「判据失败」），不再打一行「没验到」就绿 —— 本项目纪律：**前置不成立不会报错 ⇒ 护栏会
//! 悄悄失效**，「没验到」与「验到了且通过」必须能分开。要拿到绿，就让窗口**真的**收到输入
//! （人肉动鼠标/滚轮，或用 `PostMessage(WM_MOUSEMOVE)` 注入；注入侧同样要断言「派发出去的事件数 > 0」，
//! 见 `Z:\deer-gui\.superpowers\sdd\m5b-guardfix-report.md` 的实测脚本与红/绿记录）。
//!
//! **例外只有一个**：`DEER_WINDOW_REDRAW=continuous` 那一档**不下**「输入有没有改状态」的结论
//! （它验的是「策略被环境变量覆盖」，看的是 `requests`/`frames` 与那行门槛自证日志）⇒ 0 输入下仍 `exit=0`。
//!
//! ## 诚实的边界（这条本身就是 M5b 的结论）
//!
//! `OnDemand` 且没人操作时，本层**不排任何唤醒**（没有定时器、没有用户事件）⇒ 事件循环没有
//! 「醒过来的理由」，而探针**没法自己按时间退**（它只在 `redraw` 里被叫到）：
//!
//! - 若窗口在跑（`Continuous` 或被持续输入）：`redraw` 里检查墙钟，到点返回 `Flow::Exit`；
//! - 没人操作时：由**看门狗线程**打汇总并 `process::exit(0)` —— 看门狗晚于预算 1.5 秒醒来。
//!   ⚠️ **「看到看门狗打的那行汇总」只是观测**：它**不**等于「事件循环一直没醒」的证据
//!   （循环在打转时看门狗同样会先到；复审 I1 的 `Wait⇒Poll` 实测就是这样）。
//!   是否空转只看 `print_summary` 里那组**计数门槛**：`wants_redraw` 答真 **0** 次 /
//!   画帧 ≤ **1 + 系统帧**。
//!
//! 环境变量：
//!
//! - `DEER_IDLE_SECONDS=<s>`：墙钟预算（默认 4.0 秒，浮点数；先 `trim()` 再 `parse()`）。
//! - `DEER_IDLE_DIRTY=1`：每条输入都算「改了状态」（模拟 M5b 之前「每次输入都重绘」的行为）。
//! - `DEER_WINDOW_REDRAW=continuous`：强制连续重绘（关掉省电模式），由 `deer-window` 解析并自证。
//!
//! **已删除**：`DEER_IDLE_REQUIRE=1`（改前的语义是「只有设了它才把 0 输入判失败，默认不算失败」）。
//! 那正是「0 输入假通过」的根源：默认档顶着它假绿，而那条「没验到」的诚实分支**不可达**。
//! 现在 0 输入**一律**失败 ⇒ 不需要一个开关来保证这件事，留着它只会是一个「看起来有牙、
//! 其实默认就没牙」的空转旋钮（本项目最忌讳的东西）。

use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use deer_window::{
    App, Flow, InputEvent, RedrawPolicy, WindowConfig, WindowInfo, run,
};

/// 默认墙钟预算（秒）。
const DEFAULT_SECONDS: f64 = 4.0;

/// 看门狗比预算晚多久醒来。它只把「本层自己退」与「兜底触发」在时间上分开：
/// **「看门狗先到」≠「事件循环一直没醒」**（循环在打转时它同样会先到）—— 是否空转看
/// `print_summary` 的**计数门槛**，不看这里。
const WATCHDOG_LAG_SECONDS: f64 = 1.5;

/// 每多少帧打一行心跳（连续模式下帧数极大，逐帧打会把终端淹了）。
const PRINT_EVERY_FRAMES: u64 = 5000;

/// 探针的账本：全部用 `AtomicU64` 共享 —— `run()` 拿走了 `App`，`main`/看门狗事后还要读。
#[derive(Default)]
struct Ledger {
    /// 收到的输入事件条数。
    inputs: AtomicU64,
    /// 其中「改了状态」的条数（省电模式下应当为 0，除非 `DEER_IDLE_DIRTY=1`）。
    state_changes: AtomicU64,
    /// **系统事件**条数（`resized` + `FocusChanged`）：它们**不看** `wants_redraw`，一律请求重绘 ——
    /// 数出来才能把「输入没请求重绘」和「系统事件请求了重绘」分开，而不是笼统看帧数。
    system_events: AtomicU64,
    /// `App::wants_redraw` 被 `run()` 问了几次（每次派发输入后问一次）。
    wants_asked: AtomicU64,
    /// 其中答「要重绘」几次。
    wants_true: AtomicU64,
    /// `App::redraw` 真被调用几次（= 真画了几帧）。
    redraws: AtomicU64,
}

struct Idle {
    ledger: Arc<Ledger>,
    /// 脏标记：`input` 置、`redraw` 清、`wants_redraw` 读（M5b 里 App 侧的全部状态）。
    dirty: bool,
    /// 从 `init` 起算的墙钟基准。
    started: Option<Instant>,
    /// 墙钟预算（`Continuous` 下由 `redraw` 检查）。
    budget: Duration,
    /// 每条输入都算「改了状态」（`DEER_IDLE_DIRTY=1`）。
    every_input_dirty: bool,
}

/// 真实 App 的粗略模型：**点击/按键/文本**会改界面，**hover/滚轮/抬键**不会。
///
/// M5b 的省电核心就在这条判断上：`PointerMoved` 是数量最多的事件（鼠标动一下就一条），
/// 它不该产生重绘 —— 旧版「每个输入都请求重绘」正是空闲烧 CPU 的原因之一。
fn is_state_changing(ev: &InputEvent) -> bool {
    matches!(
        ev,
        InputEvent::PointerDown { .. } | InputEvent::KeyDown { .. } | InputEvent::TextInput { .. }
    )
}

impl App for Idle {
    fn init(&mut self, info: &WindowInfo) -> Result<(), String> {
        println!(
            "[idle_probe] init：extent={}x{} handle(HWND)=0x{:X}",
            info.extent.width, info.extent.height, info.raw.handle
        );
        if info.raw.handle == 0 {
            return Err("HWND 为 0：原生句柄没填对".to_string());
        }
        self.started = Some(Instant::now());
        Ok(())
    }

    fn resized(&mut self, width: u32, height: u32) -> Result<(), String> {
        self.ledger.system_events.fetch_add(1, Ordering::SeqCst);
        println!("[idle_probe] resized：{width}x{height}（系统事件 ⇒ 一定重绘）");
        Ok(())
    }

    fn input(&mut self, _info: &WindowInfo, ev: &InputEvent) -> Result<Flow, String> {
        self.ledger.inputs.fetch_add(1, Ordering::SeqCst);
        // 焦点变化是**系统事件**（`run()` 对它走「一律置位」那一档），单独数，别混进「输入改状态」里。
        if matches!(ev, InputEvent::FocusChanged { .. }) {
            self.ledger.system_events.fetch_add(1, Ordering::SeqCst);
        }
        if self.every_input_dirty || is_state_changing(ev) {
            self.ledger.state_changes.fetch_add(1, Ordering::SeqCst);
            self.dirty = true;
        }
        Ok(Flow::Continue)
    }

    /// **M5b 的核心问题**：这一条输入改了状态吗？
    ///
    /// 注意它只是**回答**（`&self`，改不了状态）：清账在 `redraw`（画完才不脏）。
    /// 这里顺带数「`run()` 到底问了几次、答真几次」—— 那就是「空闲不请求重绘」的可观测证据。
    fn wants_redraw(&self) -> bool {
        self.ledger.wants_asked.fetch_add(1, Ordering::SeqCst);
        if self.dirty {
            self.ledger.wants_true.fetch_add(1, Ordering::SeqCst);
        }
        self.dirty
    }

    /// **默认省电**：本探针声明 `OnDemand`（也就是用 `App` 的默认实现）。
    ///
    /// 想关掉省电模式不必改这里：`DEER_WINDOW_REDRAW=continuous` 会在**运行时**覆盖它
    /// （`deer-window` 启动时会打一行自证日志）。
    fn redraw_policy(&self) -> RedrawPolicy {
        RedrawPolicy::OnDemand
    }

    fn redraw(&mut self) -> Result<Flow, String> {
        let n = self.ledger.redraws.fetch_add(1, Ordering::SeqCst) + 1;
        // 画完这一帧 = 界面跟状态一致了 ⇒ 清脏标记（不清的话「改了状态」会永久为真）。
        self.dirty = false;
        if n == 1 {
            println!("[idle_probe] frame 1（建窗引导帧：一次性，不是空转）");
        } else if n % PRINT_EVERY_FRAMES == 0 {
            println!("[idle_probe] frame {n}");
        }
        // 只有 `Continuous`（或一直有输入）才会走到这里；到点就退，让 `run()` 打它自己的账本行。
        let over = self
            .started
            .is_some_and(|started| started.elapsed() >= self.budget);
        if over {
            println!("[idle_probe] 到达墙钟预算（frames={n}）⇒ Flow::Exit");
            return Ok(Flow::Exit);
        }
        Ok(Flow::Continue)
    }

    fn close_requested(&mut self) -> Flow {
        println!("[idle_probe] close_requested：允许关闭");
        Flow::Exit
    }
}

/// 打汇总（**数得出来**的五个数）+ 给出结论。看门狗与正常退出共用这一个函数。
fn print_summary(ledger: &Ledger, continuous: bool, dirty_mode: bool, via_watchdog: bool) -> bool {
    let inputs = ledger.inputs.load(Ordering::SeqCst);
    let changes = ledger.state_changes.load(Ordering::SeqCst);
    let system = ledger.system_events.load(Ordering::SeqCst);
    let asked = ledger.wants_asked.load(Ordering::SeqCst);
    let wants_true = ledger.wants_true.load(Ordering::SeqCst);
    let redraws = ledger.redraws.load(Ordering::SeqCst);

    println!(
        "[idle_probe] 汇总：输入={inputs} 条（其中改状态={changes}） 系统事件={system} \
         wants_redraw 被问={asked} 答真={wants_true} 画帧={redraws}"
    );
    println!(
        "[idle_probe] 收尾方式：{} —— {}",
        if via_watchdog { "看门狗线程" } else { "事件循环里 Flow::Exit" },
        if via_watchdog {
            "无人唤醒它到预算之后（观测：本层没排任何唤醒 ⇒ 没有唤醒来源）。\
             **不**由此推出「睡在 Wait」或「没有空转」—— 循环在打转时看门狗同样会先到；\
             有没有空转只看上面的计数门槛（wants_redraw 答真 0 次 / 画帧 ≤ 1 + 系统帧）"
        } else {
            "有帧流动（Continuous 或持续输入）⇒ App 自己按墙钟退"
        }
    );

    if continuous {
        // 这一档**不下**「输入有没有改状态」的结论：它验的是「策略被环境变量覆盖」。
        // 所以它不需要「输入 > 0」这个前置（0 输入下它照样是合法证据：`redraw` 在不断被调用，
        // 看的是 `requests`/`frames` 与那行门槛自证日志），只报数。
        println!(
            "[idle_probe] 说明：DEER_WINDOW_REDRAW=continuous 强制连续重绘 ⇒ \
             期望画帧数很大（这一档**不是**省电模式）"
        );
        return true;
    }

    // ===== 前置断言：**本次必须真的派发过输入**（先判它，再判任何结论）=====
    //
    // 为什么必须放在最前面：`wants_redraw` 是「每条输入之后」被 `run()` 问一次的，一条输入都没有
    // ⇒ 它**一次都没被问过** ⇒ `答真 == 0` 必然成立、`画帧 == 1` 也必然成立。于是**任何**变异
    // （「无条件请求重绘」最典型）都能顶着它绿过去 —— 判据在这里不是变弱，而是**空转**。
    //
    // 复审实测（M5b 终审 Minor 3）：改前的分支顺序是「省电判据先判」⇒
    //   ① 会打印「自检通过（省电）：0 条输入一条都没请求重绘」（**假绿**）；
    //   ② 作者写的那条诚实分支「没有收到任何输入 ⇒ 这一趟**没验到**置位规则」**不可达**；
    //   ③ 那次做「无条件请求重绘」变异时，就跑出过一次「输入=0」、**顶着变异绿了过去**
    //      （同一个二进制在 60 条输入时才红）。
    //
    // 纪律：**前置不成立不会报错 ⇒ 护栏会悄悄失效**。所以这里把它判成**失败**（exit=1），
    // 而且用的词是「前置不成立」而不是「判据失败」—— 这两种红必须能分开看。
    if inputs == 0 {
        eprintln!(
            "[idle_probe] 自检失败（**前置不成立**）：本次派发的输入事件 = 0 条 ⇒ \
             `wants_redraw` 一次都没被问过（被问={asked} 次）⇒「输入改状态吗」这条规则**一条都没验到**。\
             这不是通过：改前这里会先命中省电分支打出「自检通过（省电）：0 条输入」，\
             连「无条件请求重绘」这种变异都会顶着它绿过去。\
             人肉验证：跑起来后把鼠标在窗口里动一动 / 滚个轮子；脚本注入（`PostMessage(WM_MOUSEMOVE)`）\
             见 `.superpowers/sdd/m5b-guardfix-report.md`。"
        );
        return false;
    }

    if dirty_mode {
        // 这一档把每条输入都当「改了状态」，只报数、不下「省电」的结论；但「输入 = 0」已经在上面
        // 被前置拦掉了 —— 否则它会打印「期望画帧数很大」却一帧都没多画（同样是空转）。
        println!(
            "[idle_probe] 说明：DEER_IDLE_DIRTY=1：每条输入都算改状态 ⇒ \
             期望画帧数很大（这一档**不是**省电模式）"
        );
        return true;
    }

    // **省电模式的判据**（两个数，都是可观测计数；前置「输入 > 0」已在上方**显式断言**过）：
    // ① 输入从未请求重绘（`wants_true == 0`）；
    // ② 画帧数不超过「引导帧 1 + 系统事件数」（系统事件一律置位，那是设计的一部分）。
    if wants_true == 0 && redraws <= 1 + system {
        println!(
            "[idle_probe] 自检通过（省电）：{inputs} 条输入一条都没请求重绘；画帧={redraws} \
             ≤ 1（引导帧）+ {system}（系统事件）"
        );
        true
    } else {
        eprintln!(
            "[idle_probe] 自检失败：期望「输入不改状态 ⇒ 不请求重绘」，实际答真 {wants_true} 次、\
             画帧 {redraws} 帧（输入 {inputs} 条、系统事件 {system} 个）"
        );
        false
    }
}

fn main() -> ExitCode {
    // 门槛判定先 `trim()`：`cmd` 的 `set X=1 && …` 会把 `&&` 前的空格算进变量值
    // （`cmd /c "set X=1 && set X"` 实测打印 `X=1 `）⇒ 严格 `== "1"` 会把「设了」判成「没设」。
    // 同一条实测与理由见 `crates/deer-gui/src/env_gate.rs`。
    let dirty_mode = std::env::var("DEER_IDLE_DIRTY").is_ok_and(|v| v.trim() == "1");
    // 改前这里还有一个 `DEER_IDLE_REQUIRE=1`（「设了才把 0 输入判失败」）—— 已删除：
    // 0 输入现在是**无条件**失败（`print_summary` 里的前置断言），这个开关不再是判据的一部分。
    // 数值门槛同样先 `trim()`：`set DEER_IDLE_SECONDS=5 && …` 的值是 `"5 "`。
    // 非法值（负数/NaN/过大）不 panic，退回默认 —— 本 crate 的纪律是「不 panic」。
    let seconds: f64 = std::env::var("DEER_IDLE_SECONDS")
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .filter(|s: &f64| s.is_finite() && *s >= 0.1 && *s <= 600.0)
        .unwrap_or(DEFAULT_SECONDS);

    // 本示例声明的策略是 OnDemand；连续模式只可能来自环境变量覆盖 ——
    // 用 `deer-window` 导出的**同一个纯函数**判定，不在这里另写一套（免得与 run() 判得不一样）。
    let forced_continuous = std::env::var(deer_window::REDRAW_ENV)
        .ok()
        .as_deref()
        .and_then(deer_window::parse_redraw_policy)
        == Some(RedrawPolicy::Continuous);

    println!(
        "[idle_probe] 启动：seconds={seconds} dirty_mode={dirty_mode} \
         强制连续={forced_continuous}（App 声明=OnDemand）"
    );

    let ledger = Arc::new(Ledger::default());

    // 看门狗：只在没人操作（`OnDemand`，本层不排任何唤醒）的档才会先到 ——
    // 但「它先到」只说明**本层没有自己退**，**不等于**「事件循环睡在 `Wait`」（打转时它同样先到）。
    // 它**不是**重绘的来源 —— 只是收尾手段（App 侧没有任何唤醒事件循环的接口，见模块文档）。
    {
        let ledger = Arc::clone(&ledger);
        let budget = Duration::from_secs_f64(seconds);
        std::thread::spawn(move || {
            std::thread::sleep(budget + Duration::from_secs_f64(WATCHDOG_LAG_SECONDS));
            println!(
                "[idle_probe] 看门狗：墙钟预算 + {WATCHDOG_LAG_SECONDS}s 到了，事件循环还没自己退出"
            );
            let ok = print_summary(&ledger, forced_continuous, dirty_mode, true);
            std::process::exit(if ok { 0 } else { 1 });
        });
    }

    let app = Idle {
        ledger: Arc::clone(&ledger),
        dirty: false,
        started: None,
        budget: Duration::from_secs_f64(seconds),
        every_input_dirty: dirty_mode,
    };

    let result = run(WindowConfig::new("idle_probe", 400, 200), app);
    if let Err(err) = result {
        eprintln!("[idle_probe] 失败：run() 返回 Err：{err}");
        return ExitCode::from(1);
    }

    // 走到这里 = 事件循环自己退了（连续模式到点 / 手动关窗）⇒ 正常汇总。
    // 「输入 = 0 ⇒ 失败」现在在 `print_summary` 里（前置断言），两条收尾路径（看门狗 / 这里）
    // 共用同一份判据，不可能漂。
    let ok = print_summary(&ledger, forced_continuous, dirty_mode, false);
    if ok { ExitCode::SUCCESS } else { ExitCode::from(1) }
}
