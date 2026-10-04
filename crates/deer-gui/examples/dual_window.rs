//! **双窗口 demo（T4.4-R3 整合验收）**：主窗 + **动态 spawn** 的第二窗，
//! 两窗共享同一个 `VkDevice`，各自一棵树、各自一个交互状态机、各自收事件、
//! 各自与 CPU 逐像素对照，关一扇另一扇存活，全部关闭应用退出
//! —— 「设计登记：多窗口（T4.4）」的 R3 出口判据，在这一个示例里端到端跑完。
//!
//! ```sh
//! # 门槛档（判据全跑、自动退出 —— 验收用这个）
//! DEER_VK_WINDOW_TESTS=1 DEER_VK_VALIDATION=1 cargo run -p deer-gui --features window --example dual_window
//! # 交互档（窗口留着给人看：Esc 退出；点 X 关窗 —— 关到最后一扇才退出）
//! DEER_WINDOW_HOLD=1 cargo run -p deer-gui --features window --example dual_window
//! ```
//!
//! 产物：stdout 的逐项判据输出（每个判据一行 + 退出码）。
//!
//! ## 门槛档的自动剧本（不碰鼠标键盘，全程可脚本化）
//!
//! 1. 主窗建好（`window_init`，本层 id = **1**）⇒ 渲染链入表（键 = `id.raw()`，**同源直传**）；
//! 2. 主窗第一帧：经 `WindowSpawner` **动态 spawn** 第二窗（决策 2 的排队建窗）；
//! 3. 第二窗建好（id = **2**）⇒ 断言两层映射：渲染表 `window_ids() == [1, 2]`、
//!    **保留键 0 不在表里**（0 在 = 0/1 错位 = 串链），两窗各是一**条不同的链**；
//! 4. 两窗各自画满 `DEER_WINDOW_FRAMES` 帧（默认 4）；各自**首帧**回读上屏像素，
//!    与 **CPU 后端对同一份 `DrawList`** 的渲染逐字节比（线性附件 ⇒ 差必须为 **0**）
//!    ——「各自 parity」，且两窗语料不同 ⇒ 串链渲染在这里必然红；
//! 5. 向第二窗投递一次系统关闭（Win32 `WM_CLOSE`，等价于人点它的 X）⇒
//!    `window_close_requested` 允许 ⇒ **只关这一扇**（决策 4）；主窗继续画
//!    `SURVIVE_FRAMES` 帧（默认 3）⇒「关一窗另一窗存活」；
//! 6. 向主窗投递关闭 ⇒ 最后一扇 ⇒ 事件循环退出（决策 4 的另一半），`exit=0`。
//!
//! ## 各自收事件（决策 1/6 的上层整合）
//!
//! 每扇窗一棵**独立**的树 + 一个**独立**的 `UiState`：`window_input(id, …)` 只把事件喂给
//! 那一扇的状态机（命中/焦点/点击都在它自己的树与几何上结算）；点它自己的按钮让
//! **那一扇**的计数 +1（按钮文字跟着变）。同一份组装代码按窗各跑各的。
//!
//! ## 两层 id 的唯一映射（接缝裁决，禁止隐式约定）
//!
//! deer-window 的 `WindowId` 从 **1** 起（主窗 = 1）；deer-vk 旧构造器 `new()` 的
//! 保留键是 **0**。本示例是唯一映射点：**`id.raw()` 直传渲染层当表键**
//! （`new_with_primary_id` / `add_window` / `*_window(id.raw())`），映射是恒等式；
//! 第 3 步那条「0 不在表里」的断言就是它的回归判据。
//!
//! ## 重绘策略与帧供给（实测决定，不是猜的）
//!
//! 策略声明 [`RedrawPolicy::Continuous`]；**帧供给**是两层兜底：
//!
//! 1. **「任一扇窗收到重绘请求 ⇒ 全部活窗各画一帧」**（`window_redraw` 先画被请求的那扇，
//!    再把其余活窗各画一帧）。为什么不全靠 per-window 续帧：实测 winit 0.30（Windows）的
//!    `request_redraw`（`RDW_INTERNALPAINT`）在「续帧请求恰逢 spawn 的建窗嵌套消息泵」时
//!    会被吞（第一次跑就撞上：主窗只画 1 帧、spawn 窗一直画；连应用级全窗请求里主窗那一次
//!    也不投递）—— per-window 投递在「多窗 + 建窗竞态」下不可依赖；
//! 2. `wake_after(16ms)` 的全窗唤醒（`Wake::Look` ⇒ `wants_redraw()` 为真 ⇒ 全窗请求）
//!    作为第二拍 —— 只要还有一扇窗活着，所有窗都继续动。
//!
//! ⚠️ **不许与 `cargo test` 并发跑在同一个 `target/` 上**（抢 build 锁 + 抢同一块 GPU，
//! 退出码不可信 —— 与 `window_parity` 同一条实测纪律）。

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::rc::Rc;

use deer_gui::gpu::interact::{FieldText, InteractiveRenderer};
use deer_gui::gpu::null::CpuRenderer;
use deer_gui::gpu::{Extent, Theme};
use deer_gui::interaction::{self, ClipSnapshot, InputEvent, Key, UiEvent, UiState};
use deer_gui::layout::builder::{Builder, L};
use deer_gui::layout::layout::{self, Geometry, Measure, TextStyle};
use deer_gui::layout::node::{Kind, Node, Rect};
use deer_gui::vk::windowed::{FrameOutcome, WindowId as ChainKey, WindowedRenderer, PRIMARY_WINDOW_ID};
use deer_gui::window::{
    App, Flow, RedrawPolicy, Waker, WindowConfig, WindowId as HostWindowId, WindowInfo,
    WindowSpawner, run,
};
use deer_core::{Color, DrawList};
use deer_text::measure::find_system_font;
use deer_text::TextEngine;

/// 两窗的清屏色（**互不相同** ⇒ 串链渲染肉眼与 parity 判据都能看见）。
const CLEAR_A: Color = Color::rgb(0x08, 0x09, 0x0C);
const CLEAR_B: Color = Color::rgb(0x0C, 0x12, 0x08);

/// 线性格式的规范值（`VK_FORMAT_B8G8R8A8_UNORM` / `VK_FORMAT_R8G8B8A8_UNORM`）。
/// parity 判据要求线性附件（sRGB 的混合在线性空间，与 CPU 字节空间对不上）。
const FMT_B8G8R8A8_UNORM: i32 = 0x2c;
const FMT_R8G8B8A8_UNORM: i32 = 0x25;

/// 每窗树里按钮的节点 id（`Builder` 的 `IdGen` 按树序生成；两窗各有各的树与状态，
/// 同一个 id 在两扇窗里指两棵不同树里的两个按钮 —— 这正是「每窗一棵树」的含义）。
const BUTTON_ID: &str = "button_1";

/// 门槛档：每扇窗要画满的帧数（`DEER_WINDOW_FRAMES`，默认 4）。
fn target_frames() -> u64 {
    std::env::var("DEER_WINDOW_FRAMES")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .unwrap_or(4)
        .max(1)
}

/// 门槛档：B 关闭之后 A 必须继续存活的帧数。
const SURVIVE_FRAMES: u64 = 3;

/// 帧供给的节拍（`wake_after` 的间隔；~60fps）。
const FRAME_INTERVAL: std::time::Duration = std::time::Duration::from_millis(16);

/// 门槛判定（`DEER_VK_WINDOW_TESTS`）。**没设时明确打印「没有门槛证据」**
/// —— 跳过也算 pass，别把它当证据（本仓库纪律）。
fn window_tests_enabled() -> bool {
    deer_gui::env_gate::flag("DEER_VK_WINDOW_TESTS")
}

/// 一扇活窗的全部**每窗状态**（决策 6：每窗一棵树 + 一个状态机 + 自己的计数）。
struct WinState {
    /// 渲染层窗口表键（= `id.raw()`，同源直传 —— 全示例唯一映射点）。
    key: ChainKey,
    /// 原生窗口句柄（门槛档向它投递系统关闭用）。
    hwnd: usize,
    /// 该窗最近一次已知的物理尺寸（`window_resized` 同步更新）。
    extent: Extent,
    clear: Color,
    /// 该窗自己的交互状态机（决策 6 的「独立 UiState」）。
    ui: UiState,
    /// 该窗自己的点击计数（点**本窗**按钮 +1 —— 「各自收事件」的可观察证据）。
    clicks: u32,
    presented: u64,
    /// 首帧 parity 结论（`None` = 还没查；`Some(0)` 才是过）。
    parity_max_diff: Option<u8>,
    /// 本窗按钮的几何中心（每帧从布局读；门槛档的自动点击瞄准这里）。
    button_center: Option<(f32, f32)>,
}

impl WinState {
    fn new(key: ChainKey, hwnd: usize, extent: Extent, clear: Color) -> WinState {
        WinState {
            key,
            hwnd,
            extent,
            clear,
            ui: UiState::default(),
            clicks: 0,
            presented: 0,
            parity_max_diff: None,
            button_center: None,
        }
    }

    /// 该窗自己的树（按钮文字带着它**自己的**点击计数 ⇒ 串链渲染必然可见）。
    fn tree(&self, name: &str) -> Node {
        let mut b = Builder::new(Kind::Column, "root").padding(24.0).gap(14.0);
        b.text(name);
        b.text(format!("本窗键 = {}（deer-window WindowId::raw() 直传渲染层）", self.key));
        b.container_opts(Kind::Row, "bar", L::new().gap(10.0).to_props(), |r| {
            r.button(format!("点我（本窗已 {} 次）", self.clicks));
        });
        b.text("两扇窗共享同一个 VkDevice；交换链/帧资源/树/交互状态各自独立。");
        b.build()
    }
}

/// 一帧的产物：树 + 几何 + 绘制列表（**带 NodeHint** ⇒ 命中可裁剪）+ 裁剪快照。
/// 几何与列表**每帧重算**（便宜，且保证与状态同源 —— 与 interactive_form 同一套做法）。
struct Frame {
    tree: Node,
    geo: Geometry,
    list: DrawList,
    clip: ClipSnapshot,
}

impl Frame {
    fn build(win: &WinState, name: &str, theme: &Theme, measure: &impl Measure) -> Frame {
        let tree = win.tree(name);
        let geo = layout::layout(
            &tree,
            Rect::new(0.0, 0.0, win.extent.width as f32, win.extent.height as f32),
            TextStyle {
                font_size: theme.font_size,
                line_height: theme.line_height,
            },
            measure,
        );
        let interact = win.ui.to_interact_state();
        let list = InteractiveRenderer::with_texts(
            theme.clone(),
            measure,
            &interact,
            FieldText::Content,
            &win.ui.texts,
        )
        .build(&tree, &geo);
        let clip = ClipSnapshot::from_draw_list(&list, &tree, &geo);
        Frame {
            tree,
            geo,
            list,
            clip,
        }
    }
}

/// 一次 parity 对照的结论（同一份列表：GPU 上屏帧 vs CPU 渲染，逐字节比）。
struct ParityRecord {
    window: String,
    max_diff: u8,
    differing: usize,
    total: usize,
}

/// 门槛档的自动剧本状态机（交互档不进入）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// 两窗都活着：各自画满 target 帧（首帧 parity）。
    Rendering,
    /// 已向两窗各投递一次「点它自己的按钮」，等两边的 `UiEvent::Clicked` 都到账
    /// （各自收事件 + 命中只落在各自的树上 —— 决策 1/6 的回归判据）。
    Clicking,
    /// 已向 B 投递系统关闭，等关闭落地。
    ClosingB,
    /// B 已关：A 继续画 `SURVIVE_FRAMES` 帧（存活证据）。
    Surviving,
    /// 已向 A 投递关闭 ⇒ 最后一扇 ⇒ 事件循环自行退出（决策 4）。
    ClosingA,
    /// 收尾（不再画任何东西）。
    Done,
}

/// 跨过 `run()` 带回 main 的验收数据（App 被值消费 ⇒ 用 Rc 共享；单线程，无并发）。
#[derive(Default)]
struct Summary {
    failed: Option<String>,
    parity: Vec<ParityRecord>,
    /// 两窗都在表上时采样的渲染表 `window_ids()`（判据：== 两窗同源键的升序）。
    chain_ids_when_both: Option<Vec<ChainKey>>,
    /// 保留键 0 当时是否在表上（必须 false —— 0/1 错位判据）。
    zero_key_present_when_both: Option<bool>,
    /// 两窗的键是否不同（必须 true —— 「不同的渲染链」判据）。
    keys_distinct: Option<bool>,
    b_closed: bool,
    /// 点击按窗到账的账本（门槛档判据：两窗各 ≥ 1，且各只算**自己**的点击）。
    clicks: HashMap<ChainKey, u32>,
    /// B 关闭那一刻 A 已呈现的帧数（存活观察期的起点）。
    a_presented_at_b_close: u64,
    /// B 关闭那一刻 A 是否仍在表上（必须 true）。
    a_alive_when_b_closed: Option<bool>,
    /// 按窗计数的事件条数（「各自收事件」的账本）。
    events: HashMap<ChainKey, u64>,
}

struct DualApp {
    spawner: Option<WindowSpawner>,
    waker: Option<Waker>,
    spawned: bool,
    renderer: Option<WindowedRenderer>,
    engine: Option<TextEngine>,
    font_path: Option<PathBuf>,
    adapter: usize,
    theme: Theme,
    hold: bool,
    target_frames: u64,
    phase: Phase,
    windows: HashMap<ChainKey, WinState>,
    first_key: Option<ChainKey>,
    summary: Rc<RefCell<Summary>>,
    /// spawn 出那扇窗的尺寸。
    spawned_extent: (u32, u32),
    /// 点击判据阶段的已画帧数（超时护栏：300+ 帧还没到账就判红，绝不挂住）。
    click_phase_draws: u32,
}

impl DualApp {
    fn name_of(&self, key: ChainKey) -> &'static str {
        if Some(key) == self.first_key {
            "窗口 A（主窗）"
        } else {
            "窗口 B（spawn 出来的）"
        }
    }

    /// 每帧排一次下一次全窗唤醒（见 `wake_handle` 的说明）。
    fn rearm_frame_tick(&mut self) {
        if let Some(w) = self.waker.as_ref() {
            w.wake_after(FRAME_INTERVAL);
        }
    }

    /// 画一扇窗（`key` 定位）的全流程：spawn 排队 → 建 frame → 呈现 → 首帧 parity →
    /// 接缝判据 → 剧本推进。失败记进 summary 并返回 `Err`（调用方转成流程退出）。
    fn draw_one_frame(&mut self, key: ChainKey) -> Result<(), String> {
        {
            let s = self.summary.borrow();
            if s.failed.is_some() || self.phase == Phase::Done {
                return Ok(()); // 收尾态：什么都不画（退出由别的路径落地）
            }
        }
        // 动态 spawn（决策 2）：主窗第一帧经 WindowSpawner 排队，安全点才真建。
        if !self.spawned && Some(key) == self.first_key {
            self.spawned = true;
            if let Some(sp) = self.spawner.as_ref() {
                let (w, h) = self.spawned_extent;
                println!("[dual_window] 主窗首帧 ⇒ WindowSpawner 排队 spawn 第二窗（{w}×{h}）");
                sp.spawn_window(WindowConfig::new(
                    "deer-gui 双窗 demo — 窗口 B（spawn）",
                    w,
                    h,
                ));
            }
        }
        // 已销毁窗口的迟到重绘请求：没有可画的链 ⇒ 跳过（不是错误）。
        let name = self.name_of(key);
        let theme = &self.theme;
        let engine = self.engine.as_mut().ok_or("还没有字体引擎")?;
        let Some(win) = self.windows.get(&key) else {
            return Ok(());
        };
        let first_frame = win.presented == 0;
        let f = Frame::build(win, name, theme, &engine.measure());
        // 记下本窗按钮的几何中心（门槛档的自动点击瞄准它；客户区物理像素口径）。
        let button_center = f.geo.get(BUTTON_ID).map(|r| (r.x + r.w / 2.0, r.y + r.h / 2.0));
        if let Some(w) = self.windows.get_mut(&key) {
            w.button_center = button_center;
        }
        let r = self.renderer.as_mut().ok_or("没有渲染器")?;
        match r.draw_and_present_window(key, &f.list, Some(engine)) {
            Ok(FrameOutcome::Presented) => {
                if let Some(w) = self.windows.get_mut(&key) {
                    w.presented += 1;
                    if w.presented <= 8 {
                        // 逐帧诊断（前 8 帧）：帧供给断在哪一扇，从这里一眼可见。
                        println!("[dual_window] 帧：窗 {key} 第 {} 帧呈现成功", w.presented);
                    }
                }
            }
            Ok(FrameOutcome::OutOfDate) => {
                // 交换链过期是正常路径：**只重建这一扇**（多窗口 resize 路由）。
                println!(
                    "[dual_window] ⚠️ 窗 {key} 第 {} 帧呈现报 OutOfDate ⇒ 只重建这一扇",
                    self.windows.get(&key).map(|w| w.presented).unwrap_or(0)
                );
                let extent = self
                    .windows
                    .get(&key)
                    .map(|w| w.extent)
                    .ok_or("窗不在表上")?;
                r.resize_window(key, extent)
                    .map_err(|e| format!("窗 {key} 过期后重建失败：{e}"))?;
                return Ok(());
            }
            Err(e) => {
                self.fail(format!("窗 {key} 呈现失败：{e}"));
                return Err("呈现失败（详见上方 ❌ 行）".to_string());
            }
        }
        // 首帧：回读上屏像素，与 CPU 对**同一份列表**逐字节比（各自 parity）。
        if first_frame {
            self.check_parity(key, &f.list)?;
        }
        // 两窗都建好后采一次接缝判据（只采一次）。
        self.sample_seam_criteria_once()?;
        // 门槛档剧本推进。
        self.advance_script(key)?;
        if self.summary.borrow().failed.is_some() {
            return Err("剧本推进中有失败记录（详见上方 ❌ 行）".to_string());
        }
        Ok(())
    }

    fn fail(&mut self, msg: String) -> Flow {
        eprintln!("[dual_window] ❌ {msg}");
        let mut s = self.summary.borrow_mut();
        if s.failed.is_none() {
            s.failed = Some(msg);
        }
        Flow::Exit
    }

    /// B 的键（非主窗的那扇；没有就回 0 —— 调用方在两窗都在时才会用它）。
    fn b_key(&self) -> Option<ChainKey> {
        self.windows.keys().copied().find(|k| Some(*k) != self.first_key)
    }

    /// 首帧 parity：把**同一份**列表交给 CPU 后端渲染，与刚回读的上屏像素逐字节比。
    fn check_parity(&mut self, key: ChainKey, list: &DrawList) -> Result<(), String> {
        let font_path = self
            .font_path
            .clone()
            .ok_or("parity 前置不成立：没有字体路径")?;
        let (extent, clear) = {
            let win = self.windows.get(&key).ok_or("parity 前置不成立：窗不在表上")?;
            (win.extent, win.clear)
        };
        // CPU 侧用**同源**引擎（同字体同字号）—— 与 window_parity 同一套对照法。
        let mut cpu = CpuRenderer::with_text(
            TextEngine::from_font_file(Path::new(&font_path), self.theme.font_size)
                .map_err(|e| format!("parity 前置不成立（解析字体失败）：{e}"))?,
        );
        let cpu_fb = cpu
            .render(extent, list, clear)
            .map_err(|e| format!("parity：CPU 渲染失败：{e}"))?;
        let cpu_px = cpu_fb.to_rgba();
        let gpu = self
            .renderer
            .as_mut()
            .ok_or("parity 前置不成立：没有渲染器")?
            .read_back_last_frame_window(key)
            .map_err(|e| format!("parity：回读该窗呈现帧失败：{e}"))?;
        if gpu.len() != cpu_px.len() {
            return Err(format!(
                "parity 前置不成立：窗 {key} 回读 {} 字节 ≠ CPU {} 字节（{}×{}）",
                gpu.len(),
                cpu_px.len(),
                extent.width,
                extent.height
            ));
        }
        let mut max_diff = 0u8;
        let mut differing = 0usize;
        for (g, c) in gpu.iter().zip(cpu_px.iter()) {
            let d = g.abs_diff(*c);
            if d > 0 {
                differing += 1;
            }
            if d > max_diff {
                max_diff = d;
            }
        }
        let name = self.name_of(key);
        let total = extent.width as usize * extent.height as usize;
        println!("[dual_window] parity[{name}]：最大通道差 {max_diff}，不同像素 {differing} / {total}（要求 0）");
        self.summary.borrow_mut().parity.push(ParityRecord {
            window: name.to_string(),
            max_diff,
            differing,
            total,
        });
        if let Some(win) = self.windows.get_mut(&key) {
            win.parity_max_diff = Some(max_diff);
        }
        if max_diff != 0 {
            return Err(format!(
                "窗 {key}（{name}）的上屏像素与 CPU 不逐字节相同：最大通道差 {max_diff}、\
                 不同像素 {differing} —— 串链渲染 / 描述符污染 / 附件格式非线性都会在这里红"
            ));
        }
        Ok(())
    }

    /// 两窗都在表上时采样一次**接缝回归判据**（只采一次）：
    /// `window_ids()` 必须是两窗同源键的升序、不含保留键 0、两键不同。
    fn sample_seam_criteria_once(&mut self) -> Result<(), String> {
        if self.summary.borrow().chain_ids_when_both.is_some() || self.windows.len() < 2 {
            return Ok(());
        }
        let expect: Vec<ChainKey> = {
            let mut k: Vec<ChainKey> = self.windows.keys().copied().collect();
            k.sort_unstable();
            k
        };
        {
            let r = self.renderer.as_ref().ok_or("没有渲染器")?;
            let mut s = self.summary.borrow_mut();
            s.chain_ids_when_both = Some(r.window_ids());
            s.zero_key_present_when_both = Some(r.contains_window(PRIMARY_WINDOW_ID));
            s.keys_distinct = Some(self.windows.len() == 2 && expect[0] != expect[1]);
        }
        let r = self.renderer.as_ref().ok_or("没有渲染器")?;
        println!(
            "[dual_window] 接缝判据：两窗键 = {expect:?} ⇒ 渲染表 window_ids() = {:?}；保留键 0 在表上 = {}",
            r.window_ids(),
            r.contains_window(PRIMARY_WINDOW_ID)
        );
        let s = self.summary.borrow();
        let ids = s.chain_ids_when_both.clone().ok_or("内部错误：刚采过样却没有值")?;
        if ids != expect {
            return Err(format!(
                "两层 id 映射串链：渲染表 {ids:?} ≠ 两窗同源键的升序 {expect:?} —— `id.raw()` 直传的恒等映射被破坏"
            ));
        }
        if s.zero_key_present_when_both == Some(true) {
            return Err(
                "保留键 0 出现在窗口表里 ⇒ 0/1 错位（有链没挂在它该挂的窗上）—— 接缝映射被破坏"
                    .to_string(),
            );
        }
        if s.keys_distinct != Some(true) {
            return Err("两扇窗拿到了同一个渲染链键 ⇒ 「各自渲染」不成立".to_string());
        }
        drop(s);
        println!("[dual_window] 接缝判据 ✅（window_ids 同源、0 不在表、两键不同）");
        Ok(())
    }

    /// 门槛档剧本推进（每窗画完一帧后调；交互档不进入）。
    fn advance_script(&mut self, key: ChainKey) -> Result<(), String> {
        if self.hold {
            return Ok(());
        }
        match self.phase {
            Phase::Rendering => {
                // 两窗各画满目标帧、parity 全 0 ⇒ 进入点击判据。
                let ready = self.windows.len() == 2
                    && self
                        .windows
                        .values()
                        .all(|w| w.presented >= self.target_frames && w.parity_max_diff == Some(0));
                if ready {
                    // 向两窗各投递一次「点它自己的按钮」—— 真实的 winit 鼠标事件
                    // （WM_MOUSEMOVE/DOWN/UP，客户区坐标）⇒ 按窗路由 → 各自状态机 → 各自命中。
                    let mut posts: Vec<(ChainKey, usize, i32, i32)> = Vec::new();
                    for (k, w) in self.windows.iter() {
                        match w.button_center {
                            Some((cx, cy)) => posts.push((*k, w.hwnd, cx as i32, cy as i32)),
                            None => {
                                return Err(format!(
                                    "窗 {k} 没有按钮几何（布局没给 `{BUTTON_ID}`）⇒ 点击判据无法进行"
                                ));
                            }
                        }
                    }
                    for (k, hwnd, cx, cy) in posts {
                        println!(
                            "[dual_window] 向窗 {k} 的按钮中心 ({cx},{cy}) 投递一次真实点击（WM_MOUSEMOVE/DOWN/UP）"
                        );
                        post_click(hwnd, cx, cy)?;
                    }
                    self.phase = Phase::Clicking;
                }
            }
            Phase::Clicking => {
                let s = self.summary.borrow();
                let a = s
                    .clicks
                    .get(&self.first_key.unwrap_or(0))
                    .copied()
                    .unwrap_or(0);
                let b_key = self
                    .windows
                    .keys()
                    .copied()
                    .find(|k| Some(*k) != self.first_key)
                    .unwrap_or(0);
                let b = s.clicks.get(&b_key).copied().unwrap_or(0);
                drop(s);
                if a >= 1 && b >= 1 {
                    println!(
                        "[dual_window] 点击按窗到账 ✅（窗 {} 计 {a} 次、窗 {b_key} 计 {b} 次 —— 各自命中各自的按钮）",
                        self.first_key.unwrap_or(0)
                    );
                    // 点击判据完成 ⇒ 向 B 投递系统关闭。
                    let Some((bkey, bhwnd)) = self.b_key().and_then(|k| {
                        self.windows.get(&k).map(|w| (k, w.hwnd))
                    }) else {
                        return Ok(());
                    };
                    println!(
                        "[dual_window] 向窗 {bkey} 投递系统关闭（WM_CLOSE）"
                    );
                    self.phase = Phase::ClosingB;
                    post_wm_close(bhwnd)?;
                } else {
                    self.click_phase_draws += 1;
                    if self.click_phase_draws > 300 {
                        return Err(format!(
                            "点击判据超时：投递后画了 300+ 帧仍没有两窗的 Clicked 到账（A={a} B={b}）\
                             ⇒ 按窗路由或命中结算断了"
                        ));
                    }
                }
            }
            Phase::Surviving => {
                let a_key = match self.first_key {
                    Some(k) if k == key => k,
                    _ => return Ok(()),
                };
                let since = self.summary.borrow().a_presented_at_b_close;
                let Some(a) = self.windows.get(&a_key) else {
                    return Err("存活阶段主窗状态丢失".to_string());
                };
                let survived = a.presented.saturating_sub(since);
                if survived >= SURVIVE_FRAMES {
                    // 存活证据落袋：B 已从窗口层消失、渲染表里也不再有条目。
                    let r = self.renderer.as_ref().ok_or("没有渲染器")?;
                    let b_gone = match self.b_key() {
                        Some(k) => !r.contains_window(k),
                        None => true,
                    };
                    let a_here = r.contains_window(a_key);
                    println!(
                        "[dual_window] B 关闭后 A 存活 {survived} 帧：B 已不在渲染表 = {b_gone}（要求 true）、A 仍在渲染表 = {a_here}（要求 true）"
                    );
                    if !b_gone || !a_here {
                        return Err("关一窗另一窗存活的判据不成立".to_string());
                    }
                    let ahwnd = a.hwnd;
                    println!(
                        "[dual_window] 向主窗 {a_key} 投递系统关闭 ⇒ 最后一扇 ⇒ 事件循环应自行退出（决策 4）"
                    );
                    self.phase = Phase::ClosingA;
                    post_wm_close(ahwnd)?;
                }
            }
            Phase::ClosingB | Phase::ClosingA | Phase::Done => {}
        }
        Ok(())
    }
}

/// 向一扇窗投递系统关闭（Win32 `WM_CLOSE`）—— 等价于人点它的 X，
/// 走的是**真实的** winit 关闭路径（`CloseRequested` ⇒ 决策 4 的关闭语义）。
/// 只在门槛档用（交互档由人点 X）。声明在示例内，不引任何依赖。
#[cfg(windows)]
fn post_wm_close(hwnd: usize) -> Result<(), String> {
    #[link(name = "user32")]
    unsafe extern "system" {
        fn PostMessageW(
            hwnd: *mut core::ffi::c_void,
            msg: u32,
            w_param: usize,
            l_param: isize,
        ) -> isize;
    }
    const WM_CLOSE: u32 = 0x0010;
    // SAFETY: `hwnd` 是本进程刚创建、仍存活的窗口句柄（`WindowInfo.raw.handle`）；
    // `PostMessageW` 只是把一条消息排进该窗的消息队列，不等待、不触碰任何指针。
    let posted = unsafe { PostMessageW(hwnd as *mut core::ffi::c_void, WM_CLOSE, 0, 0) };
    if posted == 0 {
        return Err(format!("PostMessageW(WM_CLOSE) 失败（hwnd=0x{hwnd:X}）"));
    }
    Ok(())
}

#[cfg(not(windows))]
fn post_wm_close(_hwnd: usize) -> Result<(), String> {
    Err("门槛档的自动关闭只在 Windows 上实现（与窗口层同一平台边界）".to_string())
}

/// 向一扇窗的 `(x, y)`（**客户区物理像素**）投递一次真实鼠标点击：
/// `WM_MOUSEMOVE` → `WM_LBUTTONDOWN` → `WM_LBUTTONUP`。走的是 winit 的
/// 真实输入通路（映射成 `InputEvent` → 按窗路由 → 该窗状态机命中结算），
/// 等价于人把鼠标移过去点一下 —— 门槛档的「各自收事件」判据用。
#[cfg(windows)]
fn post_click(hwnd: usize, x: i32, y: i32) -> Result<(), String> {
    #[link(name = "user32")]
    unsafe extern "system" {
        fn PostMessageW(
            hwnd: *mut core::ffi::c_void,
            msg: u32,
            w_param: usize,
            l_param: isize,
        ) -> isize;
    }
    const WM_MOUSEMOVE: u32 = 0x0200;
    const WM_LBUTTONDOWN: u32 = 0x0201;
    const WM_LBUTTONUP: u32 = 0x0202;
    const MK_LBUTTON: usize = 0x0001;
    // lParam = MAKELPARAM(x, y)：低 16 位 x、高 16 位 y（客户区坐标）。
    let lparam: isize = ((((y as u16) as usize) << 16) | (x as u16 as usize)) as isize;
    // SAFETY: `hwnd` 是本进程刚创建、仍存活的窗口句柄；PostMessageW 只是把消息
    // 排进该窗的消息队列，不等待、不触碰任何指针。
    for (msg, wparam) in [(WM_MOUSEMOVE, 0usize), (WM_LBUTTONDOWN, MK_LBUTTON), (WM_LBUTTONUP, 0)] {
        let ok = unsafe { PostMessageW(hwnd as *mut core::ffi::c_void, msg, wparam, lparam) };
        if ok == 0 {
            return Err(format!("PostMessageW(msg={msg:#x}) 失败（hwnd=0x{hwnd:X}）"));
        }
    }
    Ok(())
}

#[cfg(not(windows))]
fn post_click(_hwnd: usize, _x: i32, _y: i32) -> Result<(), String> {
    Err("门槛档的自动点击只在 Windows 上实现（与窗口层同一平台边界）".to_string())
}

impl App for DualApp {
    /// 必选方法（trait 要求）。多窗口实现的重头在 [`App::window_init`]（带 id）；
    /// 覆盖了它之后本方法不再被调 —— 这里留一个显式的「不该被调」哨兵。
    fn init(&mut self, _info: &WindowInfo) -> Result<(), String> {
        Err("内部错误：覆盖了 window_init 之后 App::init 不应再被调（转发只发生在默认实现里）".to_string())
    }

    /// 必选方法（trait 要求）。多窗口实现的重头在 [`App::window_redraw`]（带 id）；
    /// 覆盖了它之后本方法不再被调 —— 同一个哨兵。
    fn redraw(&mut self) -> Result<Flow, String> {
        Err("内部错误：覆盖了 window_redraw 之后 App::redraw 不应再被调（转发只发生在默认实现里）".to_string())
    }

    /// 每建一扇窗调一次（带 id）：渲染链入表，键 = `id.raw()`（同源直传）。
    fn window_init(&mut self, id: HostWindowId, info: &WindowInfo) -> Result<(), String> {
        let key: ChainKey = id.raw();
        let is_first = self.renderer.is_none();
        // 字体（两窗共用一份 TextEngine —— 决策 3：字形图集/TextEngine 全局一份）。
        if self.engine.is_none() {
            let font_path = find_system_font().ok_or(
                "找不到系统字体（consola.ttf / arial.ttf / segoeui.ttf）—— 本示例需要真实字形",
            )?;
            let engine = TextEngine::from_font_file(Path::new(&font_path), self.theme.font_size)
                .map_err(|e| format!("解析字体失败（{}）：{e}", font_path.display()))?;
            println!(
                "[dual_window] 字体：{}（两窗共享一份 TextEngine/图集 —— 决策 3）",
                font_path.display()
            );
            self.font_path = Some(font_path);
            self.engine = Some(engine);
        }
        let clear = if is_first { CLEAR_A } else { CLEAR_B };
        if is_first {
            // 首窗：主窗链直接占用**窗口层的 id**（同源直传；保留键 0 留空 ⇒ 回归判据）。
            let renderer = WindowedRenderer::new_with_primary_id(
                self.adapter,
                key,
                info.raw,
                info.extent,
                clear,
            )
            .map_err(|e| format!("创建主窗渲染链失败（key={key}）：{e}"))?;
            println!(
                "[dual_window] 主窗链入表：key={key}（= deer-window WindowId::raw()；本层主窗从 1 起）"
            );
            self.renderer = Some(renderer);
        } else {
            let r = self
                .renderer
                .as_mut()
                .ok_or("内部错误：非首窗必须有已存在的渲染器")?;
            r.add_window(key, info.raw, info.extent, clear)
                .map_err(|e| format!("第二窗入表失败（key={key}）：{e}"))?;
            println!("[dual_window] spawn 窗链入表：key={key}");
        }
        {
            let r = self.renderer.as_ref().ok_or("没有渲染器")?;
            let fmt = r.format_of(key).map_err(|e| e.to_string())?;
            let linear = fmt == FMT_B8G8R8A8_UNORM || fmt == FMT_R8G8B8A8_UNORM;
            println!(
                "[dual_window] 窗 {key}：{}×{} / format {fmt:#010x}（{}）/ 表内 {} 条链",
                info.extent.width,
                info.extent.height,
                if linear { "线性 ✅" } else { "⚠️ 非线性 —— parity 判据将失败" },
                r.window_count()
            );
            if !linear {
                return Err(format!(
                    "窗 {key} 的交换链格式 {fmt:#010x} 不是线性 UNORM ⇒ 与 CPU 的字节空间混合不一致，\
                     parity 判据不成立（与 window_parity 同一条前提）"
                ));
            }
        }
        self.windows
            .insert(key, WinState::new(key, info.raw.handle, info.extent, clear));
        if self.first_key.is_none() {
            self.first_key = Some(key);
        }
        Ok(())
    }

    /// 建窗句柄：主窗 init 之后交付一次（与 wake_handle 同批）。存下来，第一帧排队 spawn。
    fn window_spawner(&mut self, spawner: WindowSpawner) {
        self.spawner = Some(spawner);
    }

    /// 唤醒句柄（与 window_spawner 同批交付）：**帧供给的第二条腿**。
    ///
    /// 每画完一帧都排一次 `wake_after(FRAME_INTERVAL)`：到点由窗口层对**所有活窗**
    /// 各请求一帧（`Wake::Look` ⇒ `wants_redraw()` 为真 ⇒ `request_redraw_all`）。
    /// 为什么不只靠 `Continuous` 的「单窗续帧」：实测 winit 0.30（Windows）的
    /// `request_redraw`（`RDW_INTERNALPAINT`）在「续帧请求恰逢 spawn 的建窗嵌套消息泵」
    /// 时会被吞一次（本示例第一次跑就撞上：主窗只画了 1 帧，spawn 窗一直画）——
    /// 应用级的全窗唤醒不经过那条每窗通道，任何一扇窗的链路断了都会被它救活。
    fn wake_handle(&mut self, waker: Waker) {
        println!("[dual_window] 唤醒句柄已交付（wake_after 驱动的全窗帧供给启用）");
        self.waker = Some(waker);
    }

    /// 该窗的输入只进**该窗**的状态机（决策 1/6）：命中/点击都在它自己的树上结算。
    fn window_input(
        &mut self,
        id: HostWindowId,
        _info: &WindowInfo,
        ev: &InputEvent,
    ) -> Result<Flow, String> {
        let key: ChainKey = id.raw();
        *self.summary.borrow_mut().events.entry(key).or_insert(0) += 1;
        if matches!(ev, InputEvent::KeyDown { key: Key::Escape, .. }) {
            println!("[dual_window] 窗 {key} 收到 Esc ⇒ 退出（人工出口）");
            self.phase = Phase::Done;
            return Ok(Flow::Exit);
        }
        let name = self.name_of(key);
        let engine = self.engine.as_ref().ok_or("还没有字体引擎")?;
        let theme = &self.theme;
        // 命中/状态机需要该窗**当前帧**的树与几何 —— 每帧重算很便宜，且保证同源。
        // （作用域到 `events` 为止：之后要记账 summary，`win` 的借用必须已经结束。）
        let (clicks, events_len) = {
            let Some(win) = self.windows.get_mut(&key) else {
                // 已销毁窗口的迟到事件：没有属主 ⇒ 丢弃（与窗口层同一口径）。
                return Ok(Flow::Continue);
            };
            let f = Frame::build(win, name, theme, &engine.measure());
            let events = interaction::handle(&mut win.ui, &f.tree, &f.geo, f.clip, ev);
            let mut clicked = 0u32;
            for e in &events {
                if matches!(e, UiEvent::Clicked(id) if id == BUTTON_ID) {
                    win.clicks += 1;
                    clicked += 1;
                }
            }
            (clicked, events.len())
        };
        if clicks > 0 {
            // 点击按窗到账（门槛档判据的账本）：记录到 summary 供收尾断言。
            *self.summary.borrow_mut().clicks.entry(key).or_insert(0) += clicks;
            let total = self.windows.get(&key).map(|w| w.clicks).unwrap_or(0);
            println!(
                "[dual_window] 窗 {key} 的按钮被点击（本窗累计 {total} 次，本次 {clicks} 条事件 / 共 {events_len} 条）\
                 —— 只有这一扇的状态变了"
            );
        }
        Ok(Flow::Continue)
    }

    /// 该窗的尺寸变化只重建**该窗**的交换链（多窗口 resize 路由）。
    fn window_resized(&mut self, id: HostWindowId, width: u32, height: u32) -> Result<(), String> {
        let key: ChainKey = id.raw();
        let Some(win) = self.windows.get_mut(&key) else {
            return Ok(());
        };
        win.extent = Extent {
            width: width.max(1),
            height: height.max(1),
        };
        let extent = win.extent;
        let r = self.renderer.as_mut().ok_or("没有渲染器")?;
        r.resize_window(key, extent)
            .map_err(|e| format!("窗 {key} resize 失败：{e}"))
    }

    /// 重绘派发：`window_redraw(id)` 收到的是「**这一扇**窗该画了」（决策 1 的定向请求），
    /// 本示例的帧供给策略是「**任一扇**窗收到请求 ⇒ **全部活窗各画一帧**」：
    ///
    /// ## 为什么不全靠 per-window 请求投递（实测，不是猜的）
    ///
    /// winit 0.30（Windows）的 `Window::request_redraw` 走 `RedrawWindow(RDW_INTERNALPAINT)`，
    /// 而 **WM_PAINT 是最低优先级、只在消息队列空时生成**，且「内部绘制」标志会被嵌套
    /// 消息泵（建窗就带一个）清掉。实测形态：主窗在自己的 `RedrawRequested` 里 spawn 第二窗
    /// ⇒ 主窗的续帧请求发出后**石沉大海**（连应用级 `wake_after` 的全窗请求里，主窗那一次
    /// 也不投递），只有 spawn 窗一直画 —— per-window 的请求投递在「多窗 + 建窗竞态」下
    /// **不可依赖**。把帧供给改成「以任一扇窗的请求为节拍、全窗各画一帧」之后，
    /// 只要还有一扇窗的请求活着，所有窗都继续动 —— 这正是集成 demo 需要的确定性。
    /// winit 的契约仍被遵守：被请求的那一扇**第一个**画。
    fn window_redraw(&mut self, id: HostWindowId) -> Result<Flow, String> {
        let key: ChainKey = id.raw();
        self.draw_one_frame(key)?;
        // 兜底：其余活窗各画一帧（顺序 = 窗口表键序，可复现）。
        let others: Vec<ChainKey> = self
            .windows
            .keys()
            .copied()
            .filter(|k| *k != key)
            .collect();
        for k in others {
            self.draw_one_frame(k)?;
        }
        // 帧供给的第二条腿：排下一次全窗唤醒（见 wake_handle 的说明）。
        self.rearm_frame_tick();
        Ok(Flow::Continue)
    }

    /// 用户点了那一扇窗的 X：允许关闭**这一扇**（决策 4：只关该窗；全关才退出）。
    fn window_close_requested(&mut self, id: HostWindowId) -> Flow {
        println!(
            "[dual_window] 窗 {} 收到关闭请求 ⇒ 允许（只关这一扇；全关才会退出）",
            id.raw()
        );
        Flow::Exit
    }

    /// 该窗已从活窗表移除：释放**这一扇**的渲染链（渲染侧「只关该窗」的半边）。
    fn window_destroyed(&mut self, id: HostWindowId) {
        let key: ChainKey = id.raw();
        let was_b = Some(key) != self.first_key;
        if let Some(r) = self.renderer.as_mut() {
            match r.remove_window(key) {
                Ok(true) => println!("[dual_window] 窗 {key} 的渲染链已整条释放（remove_window）"),
                Ok(false) => {}
                Err(e) => {
                    self.fail(format!("窗 {key} 的渲染链释放失败：{e}"));
                }
            }
        }
        self.windows.remove(&key);
        if was_b && self.phase == Phase::ClosingB {
            // B 关了，A 还活着 ⇒ 进入存活观察期。
            let a_presented = self.first_key.and_then(|k| self.windows.get(&k)).map(|w| w.presented);
            {
                let mut s = self.summary.borrow_mut();
                s.b_closed = true;
                s.a_presented_at_b_close = a_presented.unwrap_or(0);
                s.a_alive_when_b_closed =
                    Some(self.first_key.is_some_and(|k| self.windows.contains_key(&k)));
            }
            if let Some(k) = self.first_key {
                if let Some(a) = self.windows.get(&k) {
                    println!(
                        "[dual_window] 窗 {key} 已关；窗 {k} 仍在表上（presented={}）⇒ 进入存活观察期（{} 帧）",
                        a.presented, SURVIVE_FRAMES
                    );
                }
            }
            self.phase = Phase::Surviving;
        }
        if self.windows.is_empty() {
            println!("[dual_window] 最后一扇窗已关 ⇒ 事件循环自行退出（决策 4：全关退出）");
            self.phase = Phase::Done;
        }
    }

    fn wants_redraw(&self) -> bool {
        // 本示例声明 Continuous（门槛档要帧数、交互档要常驻）；这里恒真与策略一致。
        true
    }

    fn redraw_policy(&self) -> RedrawPolicy {
        RedrawPolicy::Continuous
    }
}

fn main() -> ExitCode {
    let hold = deer_gui::env_gate::flag("DEER_WINDOW_HOLD");
    let gated = window_tests_enabled();
    let adapter = std::env::var("DEER_WINDOW_ADAPTER")
        .ok()
        .and_then(|v| v.trim().parse::<usize>().ok())
        .unwrap_or(0);
    let target_frames = target_frames();
    println!(
        "模式：{}",
        if hold {
            "交互档（Esc 退出；点 X 关窗，全关才退出）".to_string()
        } else if gated {
            "门槛档（自动剧本：spawn → 双窗 parity → 关 B 验存活 → 关 A 全关退出）".to_string()
        } else {
            "⚠️ 未设 DEER_VK_WINDOW_TESTS：窗口会开、剧本会跑，但没有门槛证据（跳过也算 pass）".to_string()
        }
    );
    println!(
        "每窗目标帧数：{target_frames}（DEER_WINDOW_FRAMES 可改）；B 关闭后 A 存活观察 {SURVIVE_FRAMES} 帧"
    );

    let summary = Rc::new(RefCell::new(Summary::default()));
    let app = DualApp {
        spawner: None,
        waker: None,
        spawned: false,
        renderer: None,
        engine: None,
        font_path: None,
        adapter,
        theme: Theme::default(),
        hold,
        target_frames,
        phase: Phase::Rendering,
        windows: HashMap::new(),
        first_key: None,
        summary: Rc::clone(&summary),
        spawned_extent: (520, 400),
        click_phase_draws: 0,
    };
    let cfg = WindowConfig::new("deer-gui 双窗 demo — 窗口 A（主窗）", 480, 360);
    let result = run(cfg, app);

    // —— 收尾断言（读的是 App 记在 summary 里的账）——
    let s = summary.borrow();
    let mut problems: Vec<String> = Vec::new();
    if let Err(e) = &result {
        problems.push(format!("run() 返回 Err：{e}"));
    }
    if let Some(f) = &s.failed {
        problems.push(format!("运行中有失败记录：{f}"));
    }
    if s.parity.len() != 2 {
        problems.push(format!(
            "parity 对照应有两份（两窗各一份），实际 {} —— 某扇窗首帧没跑完就退了",
            s.parity.len()
        ));
    }
    for p in &s.parity {
        println!(
            "parity[{}]：最大通道差 {}（要求 0）、不同像素 {}/{}",
            p.window, p.max_diff, p.differing, p.total
        );
        if p.max_diff != 0 {
            problems.push(format!(
                "窗 {} 的 parity 不成立：最大通道差 {}（要求 0）、不同像素 {}/{}",
                p.window, p.max_diff, p.differing, p.total
            ));
        }
    }
    match (
        s.chain_ids_when_both.clone(),
        s.zero_key_present_when_both,
        s.keys_distinct,
    ) {
        (Some(ids), Some(zero), Some(distinct)) => {
            println!("接缝判据：渲染表键 = {ids:?}；保留键 0 在表 = {zero}；两键不同 = {distinct}");
            if zero {
                problems.push("保留键 0 出现在渲染表里 ⇒ 0/1 错位（接缝映射被破坏）".to_string());
            }
            if !distinct {
                problems.push("两扇窗拿到同一个渲染链键 ⇒ 「各自渲染」不成立".to_string());
            }
            if ids.len() != 2 || ids[0] == ids[1] {
                problems.push(format!("渲染表键异常：{ids:?}（应为两扇窗的同源键升序）"));
            }
        }
        _ => problems.push("接缝判据没采到样（两窗没有同时在表上）".to_string()),
    }
    let mut events_lines = String::new();
    let mut sorted_events: Vec<_> = s.events.iter().collect();
    sorted_events.sort_by_key(|(k, _)| **k);
    for (k, n) in sorted_events {
        events_lines.push_str(&format!(" 窗{k}={n}条;"));
    }
    println!("事件按窗路由：{events_lines}");
    let mut clicks_lines = String::new();
    let mut sorted_clicks: Vec<_> = s.clicks.iter().collect();
    sorted_clicks.sort_by_key(|(k, _)| **k);
    for (k, n) in sorted_clicks {
        clicks_lines.push_str(&format!(" 窗{k}={n}次;"));
    }
    println!("点击按窗到账：{clicks_lines}");
    if !hold {
        // 门槛档：点击判据也必须成立（各自收事件 + 命中各自的按钮）。
        let click_keys: Vec<ChainKey> = s.clicks.keys().copied().collect();
        for expected in [1u64, 2u64] {
            let n = s.clicks.get(&expected).copied().unwrap_or(0);
            if n < 1 || !click_keys.contains(&expected) {
                problems.push(format!(
                    "窗 {expected} 没有收到它自己按钮的点击（到账 {n} 次）⇒ 「各自收事件」不成立"
                ));
            }
        }
        // 存活与关闭语义判据只在门槛档断言（交互档由人操作关窗，节奏不定）。
        if !s.b_closed {
            problems.push("门槛档没有走到「关 B」这一步（剧本中断？）".to_string());
        }
        match s.a_alive_when_b_closed {
            Some(true) => println!("B 关闭时 A 仍在表上 ✅（关一窗另一窗存活）"),
            Some(false) => {
                problems.push("B 关闭时 A 已经不在表上 ⇒ 不是「只关该窗」".to_string())
            }
            None => problems.push("没有采到「B 关闭时 A 是否存活」的样本".to_string()),
        }
        if s.a_presented_at_b_close > 0 {
            println!(
                "B 关闭时 A 已呈现 {} 帧；此后 A 又画满 {SURVIVE_FRAMES} 帧才被关闭（存活观察期）",
                s.a_presented_at_b_close
            );
        }
    }
    println!();
    if problems.is_empty() {
        println!("双窗 demo 验收通过 ✅（动态 spawn、各自渲染、各自收事件、各自 parity、关一窗另一窗存活、全关退出）");
        ExitCode::SUCCESS
    } else {
        for p in &problems {
            eprintln!("[dual_window] ❌ {p}");
        }
        ExitCode::FAILURE
    }
}
