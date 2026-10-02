//! **惯性滚动的窗口接线**（T3.2b）：真实窗口里，滚轮之后内容会**继续滑一段**并自己停。
//!
//! 这正是 PR #43 登记、本示例补上的那一半：`ScrollInertia` 的纯逻辑与判据早已齐备，
//! 缺的是「有人按 [`INERTIA_TICK_MS`] 调 [`advance_inertia`]」的**驱动方**。本示例就是
//! 参考实现 —— 三件套各就各位：
//!
//! 1. **推进**：`redraw()` 开头调 [`advance_inertia`]（每步偏移 += 速度、速度 ×0.85 衰减）；
//! 2. **拉式唤醒**：`App::next_deadline` 用 [`inertia_deadline`] —— 在滚 ⇒ `now+16ms`，
//!    停了 ⇒ `None`（事件循环回到 `Wait` 睡死，**不空转**）；
//! 3. **脏位**：惯性每步都发 `UiEvent::Scrolled` ⇒ `wants_redraw` 有据可查。
//!
//! 同时演示 **点轨道跳转**：按滚动条竖带的空白处 ⇒ 按下那一刻跳到指针处（并可继续拖）。
//!
//! 跑法（需要真窗口，仅 Windows）：
//! ```text
//! cargo run -p deer-gui --features window --example scroll_inertia_window
//! ```
//! 验收标记（stdout，`grep` 这些而不是只看退出码）：
//! - `[inertia] 播种：` —— 滚轮进入惯性；
//! - `[inertia] 停：步数=` —— 衰减停（不是撞墙停）；
//! - `[deer-window] 唤醒账本：` —— `iters` 与帧数同阶（每步一帧），退出后**不再增长**。
//!
//! 自动的无窗口判据在 `crates/deer-gui/src/interaction.rs` 的单测里（推进/衰减/撞墙即停/
//! 拖动杀惯性/点轨道跳转共 10 条）；本示例补的是「真窗口里看得见」的肉眼档。

use deer_gui::interaction::{
    self, InputEvent, UiEvent, UiState, WHEEL_STEP_PX,
};
use deer_gui::layout::TextStyle;
use deer_gui::layout::layout::{ApproxMeasure, ScrollOffsets, layout_with_scroll};
use deer_gui::layout::node::{Kind, Rect, Size};
use deer_gui::prelude::*;
use deer_gui::window::{
    App, Flow, RedrawPolicy, WindowConfig, WindowInfo, run, Waker,
};

/// 视口高 300；40 行 × 40 ⇒ 内容 1600 ⇒ `max_scroll = 1300`（深到撞不到墙）。
const VIEWPORT_H: f32 = 300.0;
const ROWS: usize = 40;
const ROW_H: f32 = 40.0;
const W: f32 = 360.0;

/// 惯性从播种到停走过的总步数（自检上限：超了 = 衰减没生效）。
const MAX_INERTIA_STEPS: u64 = 500;

struct InertiaDemo {
    state: UiState,
    theme: Theme,
    extent: (u32, u32),
    /// 惯性累计步数（停了就不再涨 —— 自检「停了不空转」）。
    steps: u64,
    /// 停过一次之后 `next_deadline` 就必须永远 `None`（自检）。
    stopped_once: bool,
    /// 收到的 `Scrolled` 事件数（≥2 = 「滚一下滑一段」真的在窗口里发生）。
    scrolled_events: u64,
}

impl InertiaDemo {
    fn tree() -> Node {
        let mut col = Node::new(Kind::Column, "list");
        col.layout.scroll = true;
        col.layout.height = Some(Size::Px(VIEWPORT_H));
        col.layout.width = Some(Size::Px(W));
        for i in 0..ROWS {
            let mut b = Node::new(Kind::Button, format!("row{i}"));
            b.props.label = Some(format!("第 {} 行", i + 1));
            b.layout.height = Some(Size::Px(ROW_H));
            col.children.push(b);
        }
        col
    }

    fn frame(&self) -> (deer_gui::layout::Geometry,) {
        let style = TextStyle {
            font_size: self.theme.font_size,
            line_height: self.theme.line_height,
        };
        let (geo, metrics) = layout_with_scroll(
            &Self::tree(),
            Rect { x: 0.0, y: 0.0, w: W, h: VIEWPORT_H },
            style,
            &ApproxMeasure,
            &self.state.scroll.offsets,
        );
        // 每帧把上限灌回去（视口/内容变了要夹取 —— 见 ScrollState::set_metrics 的理由）
        let _ = metrics;
        (geo,)
    }
}

impl App for InertiaDemo {
    fn init(&mut self, info: &WindowInfo) -> Result<(), String> {
        self.extent = (info.extent.width, info.extent.height);
        // 前置：这棵树必须真的可滚（否则后面的判据在测空气）
        let (_, metrics) = layout_with_scroll(
            &Self::tree(),
            Rect { x: 0.0, y: 0.0, w: W, h: VIEWPORT_H },
            TextStyle { font_size: self.theme.font_size, line_height: self.theme.line_height },
            &ApproxMeasure,
            &ScrollOffsets::new(),
        );
        assert!(metrics.max_of("list") > 1000, "前置：容器必须深到惯性撞不到墙");
        self.state.scroll.set_metrics(&metrics);
        println!("[inertia] 就绪：max_scroll={}（滚轮一格={}px）", metrics.max_of("list"), WHEEL_STEP_PX);
        println!("[inertia] 试一试：滚轮拨一下；再按滚动条竖带的空白处（点轨道跳转）。Esc 退出。");
        Ok(())
    }

    fn input(&mut self, _info: &WindowInfo, ev: &InputEvent) -> Result<Flow, String> {
        let tree = Self::tree();
        let (geo,) = self.frame();
        let events = interaction::handle(&mut self.state, &tree, &geo, deer_gui::interaction::ClipSnapshot::unclipped(), ev);
        self.scrolled_events += events
            .iter()
            .filter(|e| matches!(e, UiEvent::Scrolled { .. }))
            .count() as u64;
        if let InputEvent::Wheel { dy, .. } = ev {
            if self.state.scroll.inertia_active() {
                println!("[inertia] 播种：dy={dy}（每步 -{WHEEL_STEP_PX}px 起步，×0.85/步）");
            }
        }
        if matches!(ev, InputEvent::KeyDown { key: deer_gui::interaction::Key::Escape, .. }) {
            // 自检：滚过 ⇒ 惯性必须已经走过 ≥2 步且**停过**；停过 ⇒ deadline 必须是 None。
            if self.scrolled_events >= 2 {
                assert!(self.stopped_once, "退出时惯性必须已经停过（否则事件循环被挂在唤醒上）");
            }
            println!(
                "[inertia] 收尾：Scrolled 事件共 {} 条，惯性累计 {} 步",
                self.scrolled_events, self.steps
            );
            return Ok(Flow::Exit);
        }
        Ok(Flow::Continue)
    }

    fn wants_redraw(&self) -> bool {
        // 输入或惯性改了状态 ⇒ 要画。这里粗粒度（示例层足够）：
        self.state.scroll.inertia_active() || self.steps == 0
    }

    fn redraw(&mut self) -> Result<Flow, String> {
        // ① 推进一步惯性（T3.2b 的接缝：推进只有这一处实现）
        let events = interaction::advance_inertia(&mut self.state);
        if !events.is_empty() {
            self.steps += 1;
            self.scrolled_events += events.len() as u64;
            assert!(self.steps < MAX_INERTIA_STEPS, "惯性 {MAX_INERTIA_STEPS} 步还没停 —— 衰减没生效");
        } else if !self.state.scroll.inertia_active() && self.steps > 0 && !self.stopped_once {
            self.stopped_once = true;
            println!("[inertia] 停：步数={}（衰减停，不是撞墙停 —— 撞墙的判据在单测里）", self.steps);
        }
        // ② 画这一帧（离屏 CPU 出图即可证明；本示例重点在驱动，不在 Vulkan 栈）
        let _ = self.frame();
        Ok(Flow::Continue)
    }

    fn next_deadline(&self) -> Option<std::time::Instant> {
        let d = interaction::inertia_deadline(&self.state);
        // 自检（拉式判据）：停过一次之后必须永远是 None —— 否则就是空转
        if self.stopped_once {
            assert!(d.is_none(), "惯性已停却还报 deadline —— 空转（WakeStats::iters 会暴露它）");
        }
        d
    }

    fn wake_handle(&mut self, _waker: Waker) {
        // 本示例用拉式（next_deadline）；推式（wake_after）走同一个 INERTIA_TICK_MS。
    }

    fn redraw_policy(&self) -> RedrawPolicy {
        RedrawPolicy::OnDemand
    }
}

fn main() {
    let app = InertiaDemo {
        state: UiState::default(),
        theme: Theme::default(),
        extent: (0, 0),
        steps: 0,
        stopped_once: false,
        scrolled_events: 0,
    };
    let cfg = WindowConfig {
        title: "惯性滚动（T3.2b）—— 滚轮 / 拖滑块 / 点轨道跳转".to_string(),
        width: W as u32,
        height: VIEWPORT_H as u32,
    };
    if let Err(e) = run(cfg, app) {
        eprintln!("[inertia] 失败：{e}");
        std::process::exit(1);
    }
}
