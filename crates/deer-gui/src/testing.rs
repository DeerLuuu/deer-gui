//! # testkit —— `deer-gui` 的**测试接口**（把既有测试纪律变成 API）
//!
//! ## 这个模块解决什么
//!
//! 本项目「测试能力很强」，但**重复且不外露**：每个 example 都要自己重写一遍
//! 「树 → 几何 → 绘制列表 → 裁剪快照 → 离屏渲染 → 像素断言 → 前置断言」
//! （`examples/counter.rs` / `examples/hello_window.rs` 各 600–900 行，大半是这套样板）；
//! 每个 `crates/*/tests/*.rs` 各自重写 env 门槛判定、像素容差、CPU-vs-GPU 对照；
//! 下游使用者想写测试**没有可用入口**，只能抄 600 行。
//!
//! 更危险的是**关键前置断言**（`node_hint > 0`、clip 非空、id 在快照内）散落各处：
//! 抄漏一处就会**静默「全不裁剪」**——界面照样能点，但「被裁掉就不命中」这条护栏
//! 已经不存在了，而且一行报错都没有（`ClipSnapshot::allows` 对未知 id **放行**）。
//!
//! 本模块把这些**变成 API**。它**不放宽任何阈值**：每一个默认值都是从既有代码里
//! 读出来的判据（出处逐条标在下面），调用方**没有**把阈值调松的入口。
//!
//! ## 最小可用示例（十几行）
//!
//! ```no_run
//! use deer_gui::prelude::*;
//! use deer_gui::testing::{Harness, ParityRule, Repro};
//!
//! # fn main() -> Result<(), String> {
//! let mut app = Builder::new(Kind::Column, "app").padding(12.0).gap(10.0);
//! app.container_opts(Kind::Row, "bar", L::new().w(160.0).gap(8.0).to_props(), |r| {
//!     r.button("+");
//! });
//! let mut h = Harness::new(app, 200, 120, Theme::default());
//! h.set_repro(Repro::test("deer-gui", "testing", "my_test", "plus_click", &[]));
//!
//! let before = h.shoot_named("点击前")?;                    // 离屏 CPU 像素（真字形）
//! let step = h.tap("button_1")?;                           // 单事件注入（含 move @id）
//! assert!(step.changed, "点按钮必须改状态");
//! let after = h.shoot_named("点击后")?;
//! after.assert_state_change_only(&before, "button_1")?;    // 差异只落在变化了的节点矩形内
//! # Ok(()) }
//! ```
//!
//! ## 能力面（与计划第 0 项的表格逐条对应）
//!
//! | 能力 | 本模块的入口 |
//! |---|---|
//! | 建面（fixture） | [`Harness::new`] / [`Harness::without_font`]（接受 [`Node`] 或 [`Builder`]，经 [`IntoTree`]） |
//! | 输入注入 | [`Harness::send`]（单事件）/ [`Harness::tap`] / [`Harness::run_script`]（脚本 + `move @id`） |
//! | 一帧 | [`Harness::frame`]（**内置前置断言**）/ [`Frame`] |
//! | 离屏渲染 | [`Harness::shoot`]（CPU 真字形）/ [`Harness::shoot_png`] / [`GpuProbe`]（离屏 Vulkan） |
//! | 像素断言 | [`Shot::assert_state_change_only`] / [`Shot::assert_diff_only_inside`] / [`assert_no_diff_outside`] / [`pixel_diff_split`] |
//! | CPU↔GPU 对照 | [`GpuProbe::compare`] → [`ParityReport`]（不规则差异：最大通道差 / 不同像素 / 最差点） |
//! | 状态断言 | [`Harness::assert_state`] / `assert_hover` / `assert_focus` / `assert_pressed` / `assert_text` / [`Harness::assert_focus_order`] |
//! | 绘制列表断言 | [`Harness::assert_counts`] / [`Harness::assert_drawn_text`] / [`Harness::assert_text_size`] / [`Harness::assert_clip_nodes`] |
//! | 门槛与自证 | [`gate`] / [`print_gate`] / [`require_gate`] / [`window::frames_from_env`]（`set "VAR=1" &&` 形态写进文档） |
//! | 窗口 e2e | [`window`]（**只含可判定的那半**，理由见该模块文档） |
//! | 失败信息 | [`Repro`] + 每个断言失败都附带「可复制复现命令 + 关键数字」 |
//!
//! ## 判据的出处（**不是**本模块自己定的）
//!
//! | 判据 | 出处 |
//! |---|---|
//! | 不透明 ⇒ 逐字节相同（差 `0`） | `crates/deer-vk/tests/gpu_vs_cpu.rs:5`、`window_parity.rs:25` |
//! | 半透明 ⇒ 最大通道差 ≤ 1 LSB | `crates/deer-vk/tests/gpu_vs_cpu.rs:6-7`（CPU `round()` vs GPU UNORM 定点） |
//! | 多状态差异**框外必须为 0** | `examples/counter.rs` 的 `assert_pixels_show_count` |
//! | `node_hint > 0` + clip 非空 + 关键 id 在快照内 | `examples/counter.rs:739-758`、`hello_window.rs:195-236` |
//! | env 门槛先 `trim()` 再比 | `src/env_gate.rs`（`set X=1 && …` 的值是 `"1 "`，实测） |
//! | `move @id` 由布局算坐标（不写死） | `examples/counter.rs` 的 `resolve_script` |
//!
//! ## 纪律：每个断言助手都有**反向自检**
//!
//! 「一条永远不会红的断言」不是护栏，是装饰。本模块的 `#[cfg(test)] mod tests` 里
//! 有一条表驱动的 `every_assertion_helper_can_go_red`：**每一个**断言助手都被喂一个
//! 故意错误的期望值，并断言它**确实返回 `Err`**（且错误里带着复现命令）。
//! 修断言助手时若把它改成恒真，那条测试立刻变红。
//!
//! ## 为什么这里的阈值没有「放宽」入口
//!
//! [`ParityRule`] 只有两档（[`ParityRule::Opaque`] / [`ParityRule::Translucent`]），
//! 上限分别是 [`BYTE_EXACT`]（0）与 [`LSB_TOLERANCE`]（1 LSB）。
//! **刻意不提供 `max_allowed: u8` 这样的参数**：一旦有，第一个「先松一点让它绿」的
//! 补丁就会进来，而这条判据的全部价值就在于它不能松。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use deer_gpu::interact::{FieldText, InteractiveRenderer, InteractState};
use deer_gpu::measure::find_system_font;
use deer_gpu::null::{CpuRenderer, Framebuffer};
use deer_gpu::draw::DrawCounts;
use deer_gpu::{Color, DrawCmd, DrawList, Extent, RectI, TextEngine, Theme};
use deer_layout::builder::Builder;
use deer_layout::layout::{self, ApproxMeasure, Geometry, Measure, TextStyle};
use deer_layout::node::{Node, Rect};

use crate::input_script;
use crate::interaction::{self, ClipSnapshot, InputEvent, PointerButton, UiEvent, UiState};
use crate::vk::GpuGeometryRenderer;

// ---------------------------------------------------------------------------
// 一、门槛与自证
// ---------------------------------------------------------------------------

/// 门槛**自证标记**：验收时 `grep TESTKIT-GATE` 就能看到「每个门槛实际解析成了什么」。
///
/// 为什么门槛必须自证：`set X=1 && …` 的值是 `"1 "`（带尾空格，见 `env_gate` 的模块文档）。
/// 判定写错时，错的方向恰好是**静默**的——明明设了门槛却打印「这是跳过」，
/// 或者明明没设却当成跑了。**把解析结果印出来**是唯一能让人一眼看见的办法。
pub const GATE_MARKER: &str = "TESTKIT-GATE";

/// 字体状态的自证标记（「降级了」必须是**看得见**的，不许静默）。
pub const FONT_MARKER: &str = "TESTKIT-FONT";

/// 「跳过」的自证标记。跳过**不是**通过，这句话要能被 grep 到。
pub const SKIP_MARKER: &str = "TESTKIT-SKIP";

/// CPU↔GPU 对照结论的自证标记。
pub const PARITY_MARKER: &str = "TESTKIT-PARITY";

/// 读一个**布尔门槛**（`"1"` / `"true"`，两侧空白先去掉）。
///
/// 直接转发 [`crate::env_gate::flag`]：门槛判定**只有一处实现**，
/// 否则「同一个命令，一个测试认为设了门槛、另一个认为没设」。
pub fn gate(name: &str) -> bool {
    crate::env_gate::flag(name)
}

/// 读一个**默认开**的门槛（未设 ⇒ `true`；`"0"` / `"false"` 关掉）。
pub fn gate_default_true(name: &str) -> bool {
    crate::env_gate::flag_default_true(name)
}

/// 打印某个门槛的**自证行**并返回其真值（grep 标记 = [`GATE_MARKER`]）。
///
/// 打的是**原始值**（`Some("1 ")` / `None`），因为「你以为设了、其实没设」这件事
/// 只能从原始值里看出来。
pub fn print_gate(name: &str) -> bool {
    let raw = std::env::var(name).ok();
    let on = gate(name);
    println!(
        "{GATE_MARKER} {name}={raw:?} ⇒ {}",
        if on { "ON" } else { "OFF" }
    );
    on
}

/// 把一组门槛的自证状态打成**一行**（在测试开头调一次，验收 grep 这一行）。
///
/// 返回这一行本身（便于断言里引用）。
pub fn print_gates(gates: &[&str]) -> String {
    let parts: Vec<String> = gates
        .iter()
        .map(|g| format!("{g}={:?}⇒{}", std::env::var(g).ok(), if gate(g) { "ON" } else { "OFF" }))
        .collect();
    let line = format!("{GATE_MARKER} [{}]", parts.join(" "));
    println!("{line}");
    line
}

/// **未设门槛 ⇒ 打印跳过说明 + 自证标记，返回 `false`**；设了 ⇒ `true`。
///
/// 用法（门控档的标准形状）：
///
/// ```no_run
/// # use deer_gui::testing;
/// if !testing::require_gate("DEER_VK_WINDOW_TESTS") {
///     return; // 这是**跳过**，不是通过 —— 自证标记已经打出来了
/// }
/// ```
pub fn require_gate(name: &str) -> bool {
    if print_gate(name) {
        return true;
    }
    println!("{SKIP_MARKER} 门槛 {name} 未启用 ⇒ **跳过**（这是跳过，不是通过）");
    false
}

// ---------------------------------------------------------------------------
// 二、容差（集中在这里，不散落）
// ---------------------------------------------------------------------------

/// 不透明语料的允许差：**逐字节 0**。
pub const BYTE_EXACT: u8 = 0;

/// 半透明语料的允许差：**≤ 1 LSB**（CPU `round()` vs GPU 固定功能 UNORM 舍入）。
pub const LSB_TOLERANCE: u8 = 1;

/// CPU↔GPU 对照的容差档（**只有两档，没有「再松一点」的入口**）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParityRule {
    /// 语料**全不透明**（每条绘制命令的 `color.a == 1.0`）⇒ 逐字节相同。
    Opaque,
    /// 语料含半透明（`0 < a < 1`）⇒ 最大通道差 ≤ 1 LSB。
    Translucent,
}

impl ParityRule {
    /// 允许的最大通道差。
    pub fn max_allowed(self) -> u8 {
        match self {
            ParityRule::Opaque => BYTE_EXACT,
            ParityRule::Translucent => LSB_TOLERANCE,
        }
    }

    /// 档名（打印用）。
    pub fn name(self) -> &'static str {
        match self {
            ParityRule::Opaque => "不透明（逐字节 0）",
            ParityRule::Translucent => "半透明（≤1 LSB）",
        }
    }

    /// **按绘制列表挑档**：每条命令的颜色都不透明 ⇒ [`ParityRule::Opaque`]，否则
    /// [`ParityRule::Translucent`]。
    ///
    /// 这就是 `gpu_vs_cpu.rs` 的口径（它的不透明语料传 `max_allowed = 0`、
    /// 半透明语料传 `1`），只是把它从「调用方手挑」变成「从语料里读」。
    /// 刻意**不**按帧缓冲的 alpha 逐像素推断：帧缓冲不告诉你「盖住这个像素的那次绘制
    /// 的 alpha 是多少」——不透明底色上的半透明填充，结果 alpha 也是 255。
    pub fn for_list(list: &DrawList) -> ParityRule {
        for cmd in &list.cmds {
            let a = match cmd {
                DrawCmd::FillRect { color, .. }
                | DrawCmd::StrokeRect { color, .. }
                | DrawCmd::FillRoundRect { color, .. }
                | DrawCmd::Text { color, .. } => color.a,
                DrawCmd::PushClip { .. } | DrawCmd::PopClip | DrawCmd::NodeHint { .. } => continue,
            };
            if a < 1.0 {
                return ParityRule::Translucent;
            }
        }
        ParityRule::Opaque
    }

    /// **按主题挑档**（粗略版，等价于「`surface` 半透明就按半透明算」）。
    ///
    /// 首选 [`ParityRule::for_list`]（它看的是**这一帧真正要画的命令**）。
    pub fn for_theme(theme: &Theme) -> ParityRule {
        if theme.surface.a < 1.0 {
            ParityRule::Translucent
        } else {
            ParityRule::Opaque
        }
    }
}

// ---------------------------------------------------------------------------
// 三、失败信息：可复制的复现命令
// ---------------------------------------------------------------------------

/// 一条**可直接复制**的复现命令。
///
/// 为什么要有这个类型：断言失败时最有用的东西是「怎么把它单独跑一遍」。
/// 库**无法知道**调用方的包名/测试名/门槛（那些只有调用方知道），所以由调用方
/// [`Harness::set_repro`] 告诉它；库负责「**每一次失败都把它印出来**」这件事。
/// 没设时用一条**通用但可复制**的命令（整包跑一遍），而不是印一句「（无复现命令）」。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Repro {
    cmd: String,
}

/// `set "A=1" && set "B=1" && ` 形态的门槛前缀。
///
/// 这个形态是**实测**出来的（见 `env_gate`）：`set X=1 && …` 的值是 `"1 "`（带尾空格），
/// 带引号的 `set "X=1" && …` 才是干净的 `"1"`。两者本项目的判定都认（先 `trim()`），
/// 但文档与示例统一用带引号那种。
pub fn gate_prefix(gates: &[&str]) -> String {
    let mut s = String::new();
    for g in gates {
        s.push_str(&format!("set \"{g}=1\" && "));
    }
    s
}

impl Repro {
    /// 一条 `cargo test` 复现命令（`gates` 会变成 `set "VAR=1" &&` 前缀）。
    pub fn test(pkg: &str, features: &str, test_target: &str, filter: &str, gates: &[&str]) -> Repro {
        let feat = if features.trim().is_empty() {
            String::new()
        } else {
            format!(" --features {}", features.trim())
        };
        let tgt = if test_target.trim().is_empty() {
            String::new()
        } else {
            format!(" --test {}", test_target.trim())
        };
        let flt = if filter.trim().is_empty() {
            String::new()
        } else {
            format!(" {}", filter.trim())
        };
        Repro {
            cmd: format!(
                "{}cargo test -p {pkg}{feat}{tgt}{flt} -- --nocapture",
                gate_prefix(gates)
            ),
        }
    }

    /// 一条 `cargo run --example` 复现命令（离屏自检档用）。
    pub fn example(pkg: &str, features: &str, example: &str, args: &str, gates: &[&str]) -> Repro {
        let feat = if features.trim().is_empty() {
            String::new()
        } else {
            format!(" --features {}", features.trim())
        };
        let args = if args.trim().is_empty() {
            String::new()
        } else {
            format!(" {}", args.trim())
        };
        Repro {
            cmd: format!(
                "{}cargo run -q -p {pkg}{feat} --example {example}{args}",
                gate_prefix(gates)
            ),
        }
    }

    /// 命令原文（**可直接粘贴**）。
    pub fn cmd(&self) -> &str {
        &self.cmd
    }
}

impl std::fmt::Display for Repro {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.cmd)
    }
}

/// 不设 `Repro` 时的兜底命令：**仍然可复制**，只是范围粗一点（整包测试 + `--nocapture`）。
pub fn default_repro() -> Repro {
    Repro::test("deer-gui", "testing", "", "", &[])
}

// ---------------------------------------------------------------------------
// 四、像素：纯函数（不碰 Harness / 不碰 GPU ⇒ 可以逐条反向自检）
// ---------------------------------------------------------------------------

/// 两个帧缓冲的差异分布：**每块期望矩形内的差异像素数** + **所有矩形之外的差异像素数**。
///
/// 数的是**像素**不是字节 —— 一个像素 4 个通道，混用会让数字虚高 4 倍
/// （`counter.rs` 的 `pixel_diff_split` 同一条口径）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffSplit {
    /// 与入参 `rects` 一一对应。
    pub inside: Vec<usize>,
    /// 落在**所有**期望矩形之外的差异像素数。**判据要求它是 0。**
    pub outside: usize,
    /// 差异像素总数（`inside` 之和 + `outside`）。
    pub total: usize,
}

impl DiffSplit {
    /// `inside` 之和。
    pub fn inside_total(&self) -> usize {
        self.inside.iter().sum()
    }
}

/// 逐像素比较两块 RGBA8（行优先、无 padding），按 `rects` 分类差异。
///
/// `width` 是像素宽度（**不是字节宽度**）。两块长度必须相同。
pub fn pixel_diff_split(
    a: &[u8],
    b: &[u8],
    width: u32,
    rects: &[RectI],
) -> Result<DiffSplit, String> {
    if a.len() != b.len() {
        return Err(format!(
            "两块帧缓冲长度不一致：{} vs {} 字节（同为 RGBA8 且同一 extent 才可比）",
            a.len(),
            b.len()
        ));
    }
    if a.len() % 4 != 0 {
        return Err(format!("帧缓冲长度 {} 不是 4 的倍数（不是 RGBA8）", a.len()));
    }
    let w = width.max(1) as usize;
    let mut inside = vec![0usize; rects.len()];
    let mut outside = 0usize;
    let mut total = 0usize;
    for (i, (x, y)) in a.chunks_exact(4).zip(b.chunks_exact(4)).enumerate() {
        if x == y {
            continue;
        }
        total += 1;
        let px = (i % w) as i32;
        let py = (i / w) as i32;
        match rects.iter().position(|r| r.contains(px, py)) {
            Some(k) => inside[k] += 1,
            None => outside += 1,
        }
    }
    Ok(DiffSplit {
        inside,
        outside,
        total,
    })
}

/// **框外必须为 0** 这条判据的独立助手（`ctx` 会进错误信息）。
pub fn assert_no_diff_outside(
    before: &[u8],
    after: &[u8],
    width: u32,
    rects: &[RectI],
    ctx: &str,
) -> Result<DiffSplit, String> {
    let d = pixel_diff_split(before, after, width, rects)?;
    if d.outside != 0 {
        return Err(format!(
            "{ctx}: 有 {} 个差异像素落在期望矩形 {rects:?} **之外**（矩形内 {} 个，差异共 {} 个）\
             ⇒ 要么变化视觉溢出了，要么**断言漏列了同时变了的东西**（两条都要查）",
            d.outside,
            d.inside_total(),
            d.total
        ));
    }
    Ok(d)
}

// ---------------------------------------------------------------------------
// 五、CPU↔GPU 对照：报告与判据（纯逻辑，GPU 只负责给字节）
// ---------------------------------------------------------------------------

/// 一次 CPU↔GPU 对照的**结构化差异**。
///
/// 字段全部来自实测字节，**没有一个是推断的**。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParityReport {
    /// 语料名（打印与断言里用）。
    pub name: String,
    /// 本次用的容差档。
    pub rule: ParityRule,
    pub extent: Extent,
    /// 最大通道差（0 ⇒ 逐字节相同）。
    pub max_channel_diff: u8,
    /// **不同像素**数（一个像素 4 个通道里任一不同即计一个）。
    pub differing_pixels: usize,
    /// 不同**字节**数（诊断用；不要拿它跟像素数混用）。
    pub differing_bytes: usize,
    pub total_pixels: usize,
    /// 最差处像素坐标 `(x, y)`。
    pub worst_pixel: (u32, u32),
    /// 最差处像素里出问题的通道（0=R 1=G 2=B 3=A）。
    pub worst_channel: usize,
    /// 最差处像素的 GPU / CPU 四通道值（**贴原始数字**，不是描述）。
    pub gpu: [u8; 4],
    pub cpu: [u8; 4],
    /// 这一帧被 GPU 侧**跳过**的文本命令数（`GpuGeometryRenderer::text_skipped`）。
    ///
    /// ⚠️ 它不是「零」就说明有文本什么都没画 —— 那时候两边「逐字节相同」是因为
    /// **都没画**，结论会假绿。所以它是报告的一部分，不是可选项。
    pub text_skipped: usize,
}

impl ParityReport {
    /// 按 [`ParityRule`] 断言。**判据只有这一处**。
    pub fn check(&self) -> Result<(), String> {
        let allowed = self.rule.max_allowed();
        if self.text_skipped != 0 {
            return Err(format!(
                "{}: 本帧 GPU 侧跳过 {} 条文本命令（空串 / size<=0 / 被裁空）—— \
                 这时候「逐字节相同」可能只是因为**两边都没画**，结论不可用",
                self.name, self.text_skipped
            ));
        }
        // 不透明语料那条要**先**判：它的文案比通用的「超过允许值」清楚得多
        // （并且它的强度更高 —— 要求的是逐字节相同，不是「≤0」）。
        if self.rule == ParityRule::Opaque && self.max_channel_diff != BYTE_EXACT {
            return Err(format!(
                "{}: 不透明语料必须**逐字节相同**，实际最大通道差 {}（最差处 ({}, {}) 通道 {}：\
                 GPU={:?} CPU={:?}；不同像素 {} / {}）",
                self.name,
                self.max_channel_diff,
                self.worst_pixel.0,
                self.worst_pixel.1,
                self.worst_channel,
                self.gpu,
                self.cpu,
                self.differing_pixels,
                self.total_pixels
            ));
        }
        if self.max_channel_diff > allowed {
            return Err(format!(
                "{}: 最大通道差 {} > 允许的 {}（档：{}）；最差处像素 ({}, {}) 通道 {}：\
                 GPU={:?} CPU={:?}；不同像素 {} / {}",
                self.name,
                self.max_channel_diff,
                allowed,
                self.rule.name(),
                self.worst_pixel.0,
                self.worst_pixel.1,
                self.worst_channel,
                self.gpu,
                self.cpu,
                self.differing_pixels,
                self.total_pixels
            ));
        }
        Ok(())
    }

    /// 一行可 grep 的结论（验收时贴这一行）。
    pub fn line(&self) -> String {
        format!(
            "{PARITY_MARKER} {} [{}] 最大通道差 {}（允许 {}）；不同像素 {} / {}；最差 ({}, {}) 通道 {} GPU={:?} CPU={:?}",
            self.name,
            self.rule.name(),
            self.max_channel_diff,
            self.rule.max_allowed(),
            self.differing_pixels,
            self.total_pixels,
            self.worst_pixel.0,
            self.worst_pixel.1,
            self.worst_channel,
            self.gpu,
            self.cpu
        )
    }
}

/// 从两块**已经拿到**的字节算对照报告（**纯函数，不需要 GPU**）。
///
/// GPU 路径只负责把字节交出来（[`GpuProbe::render`]），比较逻辑全在这里 ——
/// 于是「比较逻辑本身对不对」可以用不依赖 GPU 的测试咬死。
pub fn parity_report(
    name: &str,
    rule: ParityRule,
    extent: Extent,
    gpu: &[u8],
    cpu: &[u8],
    text_skipped: usize,
) -> Result<ParityReport, String> {
    let want = (extent.width as usize) * (extent.height as usize) * 4;
    if gpu.len() != cpu.len() {
        return Err(format!(
            "{name}: GPU 回读 {} 字节 ≠ CPU 帧缓冲 {} 字节（extent {}×{} ⇒ 期望 {want}）",
            gpu.len(),
            cpu.len(),
            extent.width,
            extent.height
        ));
    }
    if gpu.len() != want {
        return Err(format!(
            "{name}: 回读长度 {} 字节 ≠ extent {}×{} 应有的 {want} 字节",
            gpu.len(),
            extent.width,
            extent.height
        ));
    }
    let w = extent.width.max(1);
    let mut max_channel_diff = 0u8;
    let mut differing_bytes = 0usize;
    let mut worst_byte = 0usize;
    for (i, (g, c)) in gpu.iter().zip(cpu.iter()).enumerate() {
        let d = g.abs_diff(*c);
        if d > 0 {
            differing_bytes += 1;
        }
        if d > max_channel_diff {
            max_channel_diff = d;
            worst_byte = i;
        }
    }
    let mut differing_pixels = 0usize;
    for (g, c) in gpu.chunks_exact(4).zip(cpu.chunks_exact(4)) {
        if g != c {
            differing_pixels += 1;
        }
    }
    let worst_px = worst_byte / 4;
    let base = worst_px * 4;
    let gpu_px = [
        gpu[base],
        gpu[base + 1],
        gpu[base + 2],
        gpu[base + 3],
    ];
    let cpu_px = [
        cpu[base],
        cpu[base + 1],
        cpu[base + 2],
        cpu[base + 3],
    ];
    Ok(ParityReport {
        name: name.to_string(),
        rule,
        extent,
        max_channel_diff,
        differing_pixels,
        differing_bytes,
        total_pixels: (extent.width as usize) * (extent.height as usize),
        worst_pixel: ((worst_px as u32) % w, (worst_px as u32) / w),
        worst_channel: worst_byte % 4,
        gpu: gpu_px,
        cpu: cpu_px,
        text_skipped,
    })
}

/// **离屏 Vulkan** 渲染探针：给一份 [`DrawList`]，回读 RGBA8，并与 CPU 后端对照。
///
/// 没有可用 GPU 时 [`GpuProbe::new`] 返回 `Ok(None)` 并打印原因（`TESTKIT-SKIP` 标记）
/// —— **绝不伪装成通过**。但若 `DEER_VK_VALIDATION=1` 已经请求了校验层，
/// 建不起渲染器就是**测试失败**而不是跳过（否则「校验层在跑」那条就变成了空话，
/// 口径同 `crates/deer-vk/tests/gpu_vs_cpu.rs` 的 `renderer()`）。
pub struct GpuProbe {
    renderer: GpuGeometryRenderer,
    cpu: CpuRenderer,
    extent: Extent,
    clear: Color,
}

impl GpuProbe {
    /// 建探针。`font` = `(字体文件, 字号)`：给了就让 GPU 也画**真字形**（与 CPU 同源同字号）。
    ///
    /// CPU 侧也用**同一个字体文件、同一个字号**另建一个引擎（`CpuRenderer` 会拿走所有权）。
    /// 两个引擎解析同一个文件、同一个字号 ⇒ 度量与光栅化逐字节一致。
    pub fn new(
        extent: Extent,
        clear: Color,
        font: Option<(&Path, f32)>,
    ) -> Result<Option<GpuProbe>, String> {
        let mut renderer = match GpuGeometryRenderer::new(0, extent, clear) {
            Ok(r) => r,
            Err(e) => {
                if crate::env_gate::flag("DEER_VK_VALIDATION") {
                    return Err(format!(
                        "DEER_VK_VALIDATION 已请求，但 GPU 渲染器建不起来（{e}）—— \
                         要么校验层没生效，要么设备起不来；两种都不能打印一句「跳过」就当通过"
                    ));
                }
                println!("{SKIP_MARKER} 本机没有可用的 Vulkan GPU（{e}）⇒ 本档**跳过**（这是跳过，不是通过）");
                return Ok(None);
            }
        };
        let mut cpu = CpuRenderer::new();
        if let Some((path, size)) = font {
            let gpu_engine = TextEngine::from_font_file(path, size)
                .map_err(|e| format!("给 GPU 建字体引擎失败（{} / {size} px）：{e}", path.display()))?;
            let cpu_engine = TextEngine::from_font_file(path, size)
                .map_err(|e| format!("给 CPU 建字体引擎失败（{} / {size} px）：{e}", path.display()))?;
            renderer = renderer
                .with_text(gpu_engine)
                .map_err(|e| format!("GPU 接管文本管线失败：{e}"))?;
            cpu = CpuRenderer::with_text(cpu_engine);
        }
        Ok(Some(GpuProbe {
            renderer,
            cpu,
            extent,
            clear,
        }))
    }

    pub fn extent(&self) -> Extent {
        self.extent
    }

    /// 文本管线是否已接管（`false` ⇒ 语料里不能有 `DrawCmd::Text`）。
    pub fn text_enabled(&self) -> bool {
        self.renderer.text_enabled()
    }

    /// **校验层是否真的在跑**（不是「是否请求」）。
    pub fn validation_enabled(&self) -> bool {
        self.renderer.validation_enabled()
    }

    /// GPU 画一帧并回读 RGBA8。
    pub fn render(&mut self, list: &DrawList) -> Result<Vec<u8>, String> {
        let bytes = self
            .renderer
            .render(list)
            .map_err(|e| format!("GPU 渲染失败：{e}"))?;
        if !self.renderer.unsupported().is_empty() {
            return Err(format!(
                "GPU 未能翻译这些命令：{:?}（语料里含文本时，先 `GpuProbe::new(.., Some((font, size)))`）",
                self.renderer.unsupported()
            ));
        }
        Ok(bytes)
    }

    /// CPU 画**同一份**列表（真字形，与 GPU 同源）。
    pub fn cpu_render(&mut self, list: &DrawList) -> Result<Vec<u8>, String> {
        let fb = self
            .cpu
            .render(self.extent, list, self.clear)
            .map_err(|e| format!("CPU 渲染失败：{e}"))?;
        Ok(fb.pixels)
    }

    /// 用**已经拿到**的 GPU 回读字节算报告（不重复渲染；反向自检与「与手工版逐字节相同」
    /// 的对照都用它）。
    pub fn report_from(
        &self,
        name: &str,
        rule: ParityRule,
        gpu: &[u8],
        cpu: &[u8],
    ) -> Result<ParityReport, String> {
        parity_report(
            name,
            rule,
            self.extent,
            gpu,
            cpu,
            self.renderer.text_skipped(),
        )
    }

    /// 完整对照：GPU 画一帧 + CPU 画同一帧 + 算报告 + **按档断言**。
    pub fn compare(
        &mut self,
        name: &str,
        list: &DrawList,
        rule: ParityRule,
    ) -> Result<ParityReport, String> {
        let gpu = self.render(list)?;
        let cpu = self.cpu_render(list)?;
        let report = self.report_from(name, rule, &gpu, &cpu)?;
        println!("{}", report.line());
        report.check()?;
        Ok(report)
    }

    /// 一次对照的**CPU 侧帧缓冲**（与 [`GpuProbe::compare`] 同一份语料），便于出 PNG。
    pub fn cpu_frame(&mut self, list: &DrawList) -> Result<Framebuffer, String> {
        self.cpu
            .render(self.extent, list, self.clear)
            .map_err(|e| format!("CPU 渲染失败：{e}"))
    }
}

// ---------------------------------------------------------------------------
// 六、一帧
// ---------------------------------------------------------------------------

/// 一帧的**纯数据**（全部可以留到断言里用；没有任何借用）。
#[derive(Debug, Clone, PartialEq)]
pub struct Frame {
    pub tree: Node,
    pub geo: Geometry,
    pub list: DrawList,
    pub clip: ClipSnapshot,
    /// 这一帧的交互状态（像素断言用它解释「哪个节点也变了」）。
    pub state: UiState,
}

impl Frame {
    /// 命令计数（含 `NodeHint` / `PushClip`）。
    pub fn counts(&self) -> DrawCounts {
        self.list.counts()
    }

    /// 节点矩形（整数口径，像素断言用）。
    pub fn rect_of(&self, id: &str) -> Option<RectI> {
        self.geo
            .get(id)
            .map(|r| RectI::new(r.x as i32, r.y as i32, r.w as i32, r.h as i32))
    }

    /// 节点 id 的**中心点** —— 脚本的 `move @id` 用它算坐标（布局一变，脚本跟着变）。
    ///
    /// 没有几何 ⇒ `Err`（**硬错**）：按 id 定位的全部价值就是「不许静默点到空处」。
    pub fn center(&self, id: &str) -> Result<(f32, f32), String> {
        let r = self
            .geo
            .get(id)
            .ok_or_else(|| format!("布局没有给 `{id}` 几何（脚本的 `move @{id}` 会点在空处）"))?;
        Ok((r.x + r.w / 2.0, r.y + r.h / 2.0))
    }

    /// 前序遍历、只收集**有几何**的节点（顺序与 [`ClipSnapshot::from_draw_list`] 的绑定口径一致）。
    pub fn ordered_with_geometry(&self) -> Vec<&Node> {
        let mut out = Vec::new();
        collect_with_geometry(&self.tree, &self.geo, &mut out);
        out
    }

    /// 有几何的节点 id（树序）。
    pub fn ids_with_geometry(&self) -> Vec<String> {
        self.ordered_with_geometry()
            .iter()
            .map(|n| n.id.clone())
            .collect()
    }

    /// 画出来的文本（`节点 id → 该节点段里的文本`）。**这是「渲染出来的文本」的直接证据**：
    /// 它来自绘制列表，不是从状态里读回来的。
    ///
    /// 绑定口径与 [`ClipSnapshot::from_draw_list`] **逐字相同**：列表里第 k 条 `NodeHint`
    /// 对应前序里第 k 个「有几何的节点」；某条 `NodeHint` 之后、下一条之前的 `Text` 命令
    /// 就属于它。
    ///
    /// 绑定错位时**报错**，不静默丢（`k == 0` = 提示之前就有文本；`k > len` = 提示比节点多）。
    pub fn drawn_texts(&self) -> Result<BTreeMap<String, String>, String> {
        let mut out: BTreeMap<String, String> = BTreeMap::new();
        let ordered = self.ordered_with_geometry();
        let mut k = 0usize;
        for cmd in &self.list.cmds {
            match cmd {
                DrawCmd::NodeHint { .. } => k += 1,
                DrawCmd::Text { text, .. } => {
                    if k == 0 {
                        return Err(format!(
                            "绘制列表里第 1 条 `Text` 出现在任何 `NodeHint` **之前**（文本 `{text}`）\
                             —— 「第 k 条提示 = 第 k 个有几何的节点」这个绑定不成立，\
                             文本与节点的对应关系不可信"
                        ));
                    }
                    let node = ordered.get(k - 1).ok_or_else(|| {
                        format!(
                            "第 {k} 条 `NodeHint` 没有对应的节点（有几何的节点只有 {} 个）\
                             ⇒ 列表与树不同源",
                            ordered.len()
                        )
                    })?;
                    out.entry(node.id.clone()).or_default().push_str(text);
                }
                _ => {}
            }
        }
        Ok(out)
    }

    /// 画出来的文本的**字号**（`节点 id → 该节点的每个 `Text` 命令的 size`）。
    pub fn drawn_text_sizes(&self) -> Result<BTreeMap<String, Vec<f32>>, String> {
        let mut out: BTreeMap<String, Vec<f32>> = BTreeMap::new();
        let ordered = self.ordered_with_geometry();
        let mut k = 0usize;
        for cmd in &self.list.cmds {
            match cmd {
                DrawCmd::NodeHint { .. } => k += 1,
                DrawCmd::Text { size, .. } => {
                    let node = ordered
                        .get(k.checked_sub(1).ok_or_else(|| {
                            "绘制列表里出现「任何 `NodeHint` 之前」的 `Text`".to_string()
                        })?)
                        .ok_or_else(|| format!("第 {k} 条 `NodeHint` 没有对应的节点"))?;
                    out.entry(node.id.clone()).or_default().push(*size);
                }
                _ => {}
            }
        }
        Ok(out)
    }
}

/// 前序遍历、只收集有几何的节点。
fn collect_with_geometry<'a>(n: &'a Node, geo: &Geometry, out: &mut Vec<&'a Node>) {
    if geo.contains_key(&n.id) {
        out.push(n);
    }
    for c in &n.children {
        collect_with_geometry(c, geo, out);
    }
}

/// **前置条件**检查（纯函数 ⇒ 可以被反向自检）。
///
/// 三条都在这里，且都在**同一处**：
///
/// 1. 绘制列表的裁剪栈必须配平；
/// 2. `node_hint > 0` 且裁剪快照非空 —— 否则命中会**静默退化成「全不裁剪」**
///    （`ClipSnapshot::allows` 对未知 id 放行）。用不发 `NodeHint` 的渲染器
///    （`DefaultRenderer`）就会走到这个坑里，**而且一行报错都没有**；
/// 3. `required` 里每个 id 都要有几何、且**在裁剪快照里**（`is_known`）——
///    抄漏这一条，「被裁掉就不命中」这条护栏就静默失效了。
pub fn check_preconditions(
    list: &DrawList,
    clip: &ClipSnapshot,
    geo: &Geometry,
    required: &[&str],
) -> Result<(), String> {
    if !list.clip_balanced() {
        return Err("绘制列表的裁剪栈不平衡（PushClip/PopClip 未配对）".to_string());
    }
    let hints = list.counts().node_hint;
    if hints == 0 || clip.is_empty() {
        return Err(format!(
            "前置条件不成立：这一帧的绘制列表没有节点提示（node_hint={hints}，裁剪快照 len={}）\
             —— 命中会退化成「全不裁剪」而**不报错**。\
             检查是不是用了不发 NodeHint 的渲染器（`DefaultRenderer` 不发；`InteractiveRenderer` 才发）",
            clip.len()
        ));
    }
    for id in required {
        if !geo.contains_key(*id) {
            return Err(format!(
                "前置条件不成立：布局没有给 `{id}` 几何 ⇒ 后面所有按 id 的定位（脚本 `move @{id}`、\
                 按 id 取矩形做像素断言）都会点空 / 报假数字"
            ));
        }
        if !clip.is_known(id) {
            return Err(format!(
                "前置条件不成立：裁剪快照里没有 `{id}`（`allows()` 对未知 id **放行**）—— \
                 那会让「被裁掉就不命中」这条护栏静默失效"
            ));
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// 七、建面：`Harness`
// ---------------------------------------------------------------------------

/// 「能变成一棵 [`Node`]」的东西：`Node` / `&Node` / `Builder` / `&Builder`。
///
/// 有了它，[`Harness::new`] 既能收 `Builder`（教程里那种写法），也能收已经建好的 `Node`。
pub trait IntoTree {
    fn into_tree(self) -> Node;
}

impl IntoTree for Node {
    fn into_tree(self) -> Node {
        self
    }
}

impl IntoTree for &Node {
    fn into_tree(self) -> Node {
        self.clone()
    }
}

impl IntoTree for Builder {
    fn into_tree(self) -> Node {
        self.build()
    }
}

impl IntoTree for &Builder {
    fn into_tree(self) -> Node {
        self.build()
    }
}

/// 找不到系统字体时退回的字号。
pub const FALLBACK_FONT_SIZE: f32 = 16.0;

/// **一次输入注入的结果**（状态变了没有 + 产生了哪些 `UiEvent` + 之后的状态）。
#[derive(Debug, Clone, PartialEq)]
pub struct Step {
    /// 这一条输入事件原文。
    pub input: InputEvent,
    /// **状态真的变了**没有（判据是 `UiState` 逐字段比较，不是「收到事件了」）。
    pub changed: bool,
    /// `interaction::handle` 产出的 UI 事件。
    pub events: Vec<UiEvent>,
    /// 这一步**之后**的状态。
    pub state: UiState,
}

/// 一次脚本重放的账本。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ScriptRun {
    pub steps: Vec<Step>,
    /// 解析时把 `move @id` 展开成的**最终**坐标脚本（可复制、可回归）。
    pub resolved: String,
}

impl ScriptRun {
    /// 真的改了状态的步数。
    pub fn changed_steps(&self) -> usize {
        self.steps.iter().filter(|s| s.changed).count()
    }

    /// 所有 UI 事件（按时间序）。
    pub fn events(&self) -> Vec<UiEvent> {
        self.steps.iter().flat_map(|s| s.events.clone()).collect()
    }

    /// 每一步之后「状态变了没有」的序列（与 `input_script::replay` 的 `redrew` 同义）。
    pub fn redrew(&self) -> Vec<bool> {
        self.steps.iter().map(|s| s.changed).collect()
    }
}

/// 一次离屏渲染的**像素 + 它是在什么状态下画出来的**。
///
/// 带上 `state` 与每个节点的矩形，是为了让「差异只落在变化了的节点矩形内」这条断言
/// **自己算得出期望矩形**，而不必让调用方手抄一遍（`counter.rs` 的实测教训：
/// 只列 `count` 矩形时框外有 **393** 个差异像素 —— 那不是溢出，是断言漏列了
/// 同时变了的按钮）。
#[derive(Debug, Clone, PartialEq)]
pub struct Shot {
    pub width: u32,
    pub height: u32,
    /// RGBA8，行优先、无 padding。
    pub rgba: Vec<u8>,
    /// 画这一帧时的交互状态。
    pub state: UiState,
    /// 这一帧**每个有几何的节点**的像素矩形。
    pub rects: BTreeMap<String, RectI>,
    /// 标签（打印用）。
    pub label: String,
    /// 这张图是在哪一次测试里画的（失败时附上**可复制的复现命令**）。
    pub repro: Repro,
    /// 用例名（失败信息的抬头）。
    pub case: String,
}

impl Shot {
    /// 与 [`Harness::carry`] 同一条纪律：**每一次失败都带可复制的复现命令**。
    fn carry(&self, detail: impl std::fmt::Display) -> String {
        let head = if self.case.is_empty() {
            String::new()
        } else {
            format!("[{}] ", self.case)
        };
        format!("{head}{detail}\n复现：{}", self.repro.cmd())
    }

    /// 读一个像素（越界 `None`）。
    pub fn pixel(&self, x: i32, y: i32) -> Option<[u8; 4]> {
        if x < 0 || y < 0 || x >= self.width as i32 || y >= self.height as i32 {
            return None;
        }
        let i = ((y as usize) * (self.width as usize) + (x as usize)) * 4;
        Some([
            self.rgba[i],
            self.rgba[i + 1],
            self.rgba[i + 2],
            self.rgba[i + 3],
        ])
    }

    /// 与另一帧的差异分布。
    pub fn diff_split(&self, before: &Shot, rects: &[RectI]) -> Result<DiffSplit, String> {
        if self.width != before.width || self.height != before.height {
            return Err(format!(
                "两帧尺寸不同（{}×{} vs {}×{}）⇒ 逐像素不可比",
                self.width, self.height, before.width, before.height
            ));
        }
        pixel_diff_split(&before.rgba, &self.rgba, self.width, rects)
    }

    /// **逐字节相同**（不透明语料的判据）。
    pub fn assert_bytes_eq(&self, before: &Shot) -> Result<(), String> {
        if self.rgba == before.rgba {
            return Ok(());
        }
        let d = self.diff_split(before, &[])?;
        let (i, g, c) = worst_byte(&before.rgba, &self.rgba);
        Err(self.carry(format!(
            "「{}」与「{}」必须逐字节相同，实际有 {} 个像素 / {} 个字节不同；\
             最差字节 #{} 通道 {}：后={g:?} 前={c:?}",
            self.label,
            before.label,
            d.total,
            d.inside_total() + d.outside,
            i / 4,
            i % 4
        )))
    }

    /// **框外必须为 0**（调用方**显式**给期望矩形）。
    pub fn assert_no_diff_outside(&self, before: &Shot, rects: &[RectI]) -> Result<(), String> {
        assert_no_diff_outside(
            &before.rgba,
            &self.rgba,
            self.width,
            rects,
            &format!("「{}」vs「{}」", self.label, before.label),
        )
        .map(|_| ())
        .map_err(|e| self.carry(e))
    }

    /// 期望矩形 = `primary` 自己 + **所有视觉状态真的变了**的节点（自动算，不手抄）。
    ///
    /// ## 面积棘轮（复审 HIGH-1 的处置，**为什么是这条而不是「求交」**）
    ///
    /// 自动补齐**只看状态差**（`hover`/`focus`/`pressed` 的成员变化）。复审给出的可行修法有两条：
    ///
    /// ① **求交 + 漏报 `Err`**：把自动补齐改成「状态差 **∩** 逐节点绘制切片变化」，并对
    ///    「有切片变化却没被覆盖」直接 `Err`。**这是更彻底的修法**，但它需要 `Shot` 里
    ///    多出一份「逐节点绘制命令的指纹」—— 那是 **harness 采集侧的新通路**（不是一两行），
    ///    本修复轮不动它，作为**后续**（已登记进报告）。
    /// ② **面积棘轮**（本处采用）：自动补进来的矩形**总面积 / 画布面积**超过阈值就直接 `Err`。
    ///
    /// 采用 ② 的理由（代价 vs 收益）：反例的杀伤力**完全来自「补齐把整块画布放行」**，
    /// 而 ② 正好在这一步拦住它，**代码量与风险都是一行常数的量级**；代价是它**不检测**
    /// 中等面积容器（< 阈值）造成的放宽 —— 那条残余风险在报告里写明，不假装已闭。
    ///
    /// 阈值 `0.25` 的**依据**（实测，不是拍脑袋）：仓库内合法的自动补齐是按钮/输入框这类小控件
    /// （360×200 画布上实测占比 ≤ ~3%），而根容器 = **100%** ⇒ 25% 留了一个量级的余量。
    fn expected_rects(&self, before: &Shot, primary: &str) -> Result<(Vec<RectI>, Vec<String>), String> {
        /// 自动补齐的总面积 / 画布面积 的**棘轮**（超过就拒绝这次自动比较）。
        const AUTO_PATCH_AREA_RATCHET: f64 = 0.25;
        let p = self.rects.get(primary).ok_or_else(|| {
            self.carry(format!(
                "期望的 `{primary}` 没有几何（这一帧有几何的节点：{:?}）",
                self.rects.keys().collect::<Vec<_>>()
            ))
        })?;
        let mut rects = vec![*p];
        let ids: Vec<String> = self.rects.keys().cloned().collect();
        let mut also = Vec::new();
        for id in visually_changed_ids(&before.state, &self.state, &ids) {
            if id == primary {
                continue;
            }
            if let Some(r) = self.rects.get(&id) {
                rects.push(*r);
                also.push(id);
            }
        }
        // ★ 面积棘轮：自动补齐不得把判据放松到「几乎整屏」
        if !also.is_empty() {
            let canvas = f64::from(self.width) * f64::from(self.height);
            let patched: f64 = rects[1..]
                .iter()
                .map(|r| f64::from(r.w) * f64::from(r.h))
                .sum();
            if canvas > 0.0 && patched / canvas > AUTO_PATCH_AREA_RATCHET {
                return Err(self.carry(format!(
                    "自动补齐的节点 {also:?} 覆盖画布 {:.0}%（棘轮 {:.0}%）⇒ 这次比较的\
                     「框外为 0」几乎没有判别力（复审 HIGH-1：hover 落在根容器上就是这么绕过的）。\
                     请把同时变了的东西**指名**为 `primary`、或用 `assert_no_diff_outside` 显式列出矩形",
                    patched / canvas * 100.0,
                    AUTO_PATCH_AREA_RATCHET * 100.0
                )));
            }
        }
        Ok((rects, also))
    }

    /// **差异只落在 `primary` 与「视觉真的变了的节点」的矩形内**（框外 = 0），
    /// 但**不**要求 `primary` 矩形内非空（用于「主矩形本来就该不动」的场合）。
    pub fn assert_diff_only_inside(&self, before: &Shot, primary: &str) -> Result<(), String> {
        let (rects, also) = self.expected_rects(before, primary)?;
        let d = self.diff_split(before, &rects)?;
        println!(
            "{} 像素断言「{}」vs「{}」: 主矩形 `{primary}` 内 {} 像素；同时变了的视觉节点 {also:?} 内 {:?}；**框外 {}**",
            PARITY_MARKER,
            self.label,
            before.label,
            d.inside[0],
            &d.inside[1..],
            d.outside
        );
        if d.outside != 0 {
            return Err(self.carry(format!(
                "「{}」vs「{}」: 有 {} 个差异像素落在期望矩形 {rects:?} **之外**\
                 ⇒ 要么变化视觉溢出了，要么**断言漏列了同时变了的东西**",
                self.label, before.label, d.outside
            )));
        }
        Ok(())
    }

    /// **计数/多状态判据**：`primary` 矩形内**必须**有差异（否则「它没画出来」），
    /// 且**框外为 0**。语义逐字取自 `counter.rs::assert_pixels_show_count`。
    pub fn assert_state_change_only(&self, before: &Shot, primary: &str) -> Result<(), String> {
        let (rects, also) = self.expected_rects(before, primary)?;
        let d = self.diff_split(before, &rects)?;
        println!(
            "{} 像素断言「{}」vs「{}」: 主矩形 `{primary}` 内 {} 像素；同时变了的视觉节点 {also:?} 内 {:?}；**框外 {}**",
            PARITY_MARKER,
            self.label,
            before.label,
            d.inside[0],
            &d.inside[1..],
            d.outside
        );
        if d.inside[0] == 0 {
            return Err(self.carry(format!(
                "「{}」vs「{}」: `{primary}` 矩形 {rects:?} 内**一个像素都没变** \
                 ⇒ 期望的变化没有画出来。⚠️ 若无字库（降级档）每个字符都是同一个等宽方块 \
                 ⇒ 文本类的这条必然**假红**，先调 `Harness::require_glyph_pixels()`",
                self.label, before.label
            )));
        }
        if d.outside != 0 {
            return Err(self.carry(format!(
                "「{}」vs「{}」: 有 {} 个差异像素落在期望矩形 {rects:?} **之外**（矩形内 {} 个）",
                self.label,
                before.label,
                d.outside,
                d.inside_total()
            )));
        }
        Ok(())
    }

    /// 出 PNG 字节流（可 `std::fs::write`）。
    pub fn to_png(&self) -> Result<Vec<u8>, String> {
        deer_gpu::png::encode_rgba(self.width, self.height, &self.rgba)
    }

    /// 出 PNG 文件（顺带回显路径）。
    pub fn write_png(&self, path: impl AsRef<Path>) -> Result<(), String> {
        let p = path.as_ref();
        std::fs::write(p, self.to_png()?).map_err(|e| format!("写 PNG 失败（{}）：{e}", p.display()))
    }
}

fn worst_byte(a: &[u8], b: &[u8]) -> (usize, u8, u8) {
    let mut at = 0usize;
    let mut worst = 0u8;
    for (i, (x, y)) in a.iter().zip(b.iter()).enumerate() {
        let d = x.abs_diff(*y);
        if d > worst {
            worst = d;
            at = i;
        }
    }
    (at, b[at], a[at])
}

/// 「视觉真的变了的节点」：`hover`/`focus`/`pressed` 三个字段里，**某一个**的指向
/// 在这两个状态之间发生了「是不是这个 id」的变化。
///
/// 这三个字段正是 [`InteractState`] 的全部输入 ⇒ 渲染器只可能因它们改变某个节点的视觉。
/// `texts` 不在这里：文本变化是**应用数据**的事，由调用方指名（`primary`）。
pub fn visually_changed_ids(a: &UiState, b: &UiState, ids: &[String]) -> Vec<String> {
    ids.iter()
        .filter(|id| {
            let key = |s: &UiState| {
                (
                    s.hover.as_deref() == Some(id.as_str()),
                    s.focus.as_deref() == Some(id.as_str()),
                    s.pressed.as_deref() == Some(id.as_str()),
                )
            };
            key(a) != key(b)
        })
        .cloned()
        .collect()
}

/// **testkit 的主入口**：一棵树 + 一份状态 ⇒ 一帧、像素、断言。
///
/// 它做的事原来散在每个 example 里：几何每帧重算（保证「画的东西」与「命中的东西」同源）、
/// 绘制列表由 [`InteractiveRenderer`] 产出（**只有它发 `NodeHint`**）、裁剪快照从**本帧真实
/// 绘制列表**派生、度量接真实字体（找不到就**打印降级说明**，不静默）。
pub struct Harness {
    tree: Node,
    extent: Extent,
    theme: Theme,
    state: UiState,
    /// 布局/绘制列表用的度量来源（有引擎 ⇒ 真字形）。
    engine: Option<TextEngine>,
    /// 像素路径（CPU 离屏渲染）。
    cpu: CpuRenderer,
    font_path: Option<PathBuf>,
    font_size: f32,
    /// 字体状态说明（**降级不静默**）。
    font_note: String,
    /// 必须出现在帧里的节点 id（`frame()` 每次都断言）。
    required_ids: Vec<String>,
    clear: Color,
    case: String,
    repro: Repro,
}

impl Harness {
    /// 建面：`tree` 可以是 [`Node`] 或 [`Builder`]（见 [`IntoTree`]）。
    ///
    /// 字体：**默认接上真实字体度量**（`%WINDIR%\Fonts` 下的 consola/arial/segoeui）。
    /// 找不到或解析失败时**打印降级说明**（`TESTKIT-FONT` 标记）并退回确定性近似度量
    /// （每字符 `0.6em`），交互链不受影响；但**文本类的像素断言会失去意义** ——
    /// 那时候请先调 [`Harness::require_glyph_pixels`]（它会**明确报错**，而不是给你一条假红）。
    ///
    /// 字号**一处定义**：`TextEngine` 会把字号取整（实测 `16.0` → `font_size()` 报 `16`），
    /// 所以顺序是**先建引擎、再用 `engine.font_size()` 去改 `theme.font_size`**，
    /// 否则「布局算出来的宽度」与「画出来的宽度」会漂。
    pub fn new(tree: impl IntoTree, width: u32, height: u32, theme: Theme) -> Harness {
        Harness::build(tree.into_tree(), width, height, theme, true)
    }

    /// 同上，但**明确不使用系统字体**（降级档）。
    ///
    /// 存在理由有两个：① 让人能**故意**跑降级路径；② 让「无字库 ⇒ 文本像素断言会假红」
    /// 这条纪律有一个**能红的**护栏测试。
    pub fn without_font(tree: impl IntoTree, width: u32, height: u32, theme: Theme) -> Harness {
        Harness::build(tree.into_tree(), width, height, theme, false)
    }

    fn build(tree: Node, width: u32, height: u32, mut theme: Theme, use_font: bool) -> Harness {
        let extent = Extent {
            width: width.max(1),
            height: height.max(1),
        };
        let mut font_size = if theme.font_size > 0.0 {
            theme.font_size
        } else {
            FALLBACK_FONT_SIZE
        };
        // 清屏色默认与 `render_tree_to_rgba` 一致（`theme.surface`），离屏与上屏才是同一套口径。
        let clear = theme.surface;
        let mut engine = None;
        let mut cpu = CpuRenderer::new();
        let mut font_path = None;
        let font_note;
        match if use_font { find_system_font() } else { None } {
            Some(p) => match TextEngine::from_font_file(&p, font_size) {
                Ok(e) => {
                    font_size = e.font_size();
                    match TextEngine::from_font_file(&p, font_size) {
                        Ok(e2) => {
                            cpu = CpuRenderer::with_text(e2);
                            font_note = format!(
                                "真字形：{}（{font_size} px）—— 布局度量与离屏像素都用它",
                                p.display()
                            );
                        }
                        Err(e) => {
                            font_note = format!(
                                "**降级**：字体 {} 第二次解析失败（{e}）⇒ 布局用近似度量、像素用占位方块；\
                                 文本类像素断言不可用",
                                p.display()
                            );
                        }
                    }
                    font_path = Some(p.clone());
                    theme.font_size = font_size;
                    engine = Some(e);
                }
                Err(e) => {
                    font_note = format!(
                        "**降级**：字体 {} 解析失败（{e}）⇒ 布局用近似度量（每字符 0.6em）、\
                         像素用等宽占位方块；文本类像素断言不可用",
                        p.display()
                    );
                }
            },
            None => {
                font_note = if use_font {
                    "**降级**：没找到系统字体（`%WINDIR%\\Fonts` 下的 consola.ttf / arial.ttf / segoeui.ttf）\
                     ⇒ 布局用近似度量（每字符 0.6em）、像素用等宽占位方块；文本类像素断言不可用"
                        .to_string()
                } else {
                    "**降级**（显式要求）：不使用系统字体 ⇒ 近似度量 + 占位方块".to_string()
                };
            }
        }
        println!("{FONT_MARKER} {font_note}");
        Harness {
            tree,
            extent,
            theme,
            state: UiState::default(),
            engine,
            cpu,
            font_path,
            font_size,
            font_note,
            required_ids: Vec::new(),
            clear,
            case: String::new(),
            repro: default_repro(),
        }
    }

    // -- 配置 ---------------------------------------------------------------

    /// 清屏色（默认 = `Theme::default().surface`，与 `render_tree_to_rgba` 一致）。
    pub fn set_clear(&mut self, color: Color) {
        self.clear = color;
    }

    /// 给这一次测试起个名字（进失败信息与打印）。
    pub fn set_case(&mut self, name: impl Into<String>) {
        self.case = name.into();
    }

    /// 设「可复制的复现命令」（[`Repro`]）。不设时用 [`default_repro`]（仍然可复制）。
    pub fn set_repro(&mut self, repro: Repro) {
        self.repro = repro;
    }

    /// 声明「这些 id 必须出现在帧里」—— 从此**每一次** [`Harness::frame`] 都断言它们
    /// 有几何且**在裁剪快照里**。
    ///
    /// 这是「抄漏一处就静默全不裁剪」那条坑的正面修法：[`Harness::tap`] /
    /// [`Harness::run_script`] 会**自动**把用到的 id 登记进来，所以你不必记得。
    pub fn require_ids(&mut self, ids: &[&str]) {
        for id in ids {
            if !self.required_ids.iter().any(|r| r == id) {
                self.required_ids.push((*id).to_string());
            }
        }
    }

    /// 已登记的必查 id。
    pub fn required_ids(&self) -> &[String] {
        &self.required_ids
    }

    /// **换一棵树**（内容变了就整树重建）。
    ///
    /// 为什么必须有：本项目的交互 API 是「树 + 几何 → 绘制列表」，**没有事件回调**
    /// （变更靠整树重建，见计划的「已知边界」）。所以「点了 `+` ⇒ 计数显示变成 2」
    /// 这条闭环在测试里的形状就是：注入输入 → 改自己的数据 → **重建树** → 再断言画出来的东西。
    pub fn set_tree(&mut self, tree: impl IntoTree) {
        self.tree = tree.into_tree();
    }

    // -- 只读视图 -----------------------------------------------------------

    pub fn tree(&self) -> &Node {
        &self.tree
    }

    pub fn extent(&self) -> Extent {
        self.extent
    }

    pub fn theme(&self) -> &Theme {
        &self.theme
    }

    pub fn state(&self) -> &UiState {
        &self.state
    }

    pub fn clear(&self) -> Color {
        self.clear
    }

    /// 字体状态说明（**降级了要看得见**）。
    pub fn font_note(&self) -> &str {
        &self.font_note
    }

    /// 像素路径有没有**真字形**（`false` ⇒ 文本类像素断言不可用）。
    pub fn has_glyph_pixels(&self) -> bool {
        self.cpu.text().is_some()
    }

    /// `(字体文件, 字号)`（给 [`GpuProbe`] 用；降级档是 `None`）。
    pub fn font(&self) -> Option<(&Path, f32)> {
        self.font_path
            .as_deref()
            .map(|p| (p, self.font_size))
    }

    /// **文本类像素断言的前置**：没有真字形就明确报错。
    ///
    /// 为什么必须显式：无字库的 [`CpuRenderer::new`] 给**每个字符画同一个等宽方块**
    /// ⇒ `count = 0` 与 `count = 1` 的像素**逐个相同**（`counter.rs` 实测：矩形内差异
    /// **0** 像素，而绘制列表里文本确实变了）—— 拿它做文本断言只会得到一条**假红**。
    pub fn require_glyph_pixels(&self) -> Result<(), String> {
        if self.has_glyph_pixels() {
            return Ok(());
        }
        Err(self.carry(format!(
            "本档没有真字形（{}）⇒ 文本类的像素断言会得到**假红**（每个字符都是同一个方块）。\
             要么修字体，要么这一档只做绘制列表/状态级断言",
            self.font_note
        )))
    }

    /// 把「这一次测试是谁、怎么复现」附到一条失败信息上。
    ///
    /// **每一个**断言失败都走这里 ⇒ 「失败时打印可直接复制的复现命令」是结构性的，
    /// 不靠每个断言作者记得写。
    pub fn carry(&self, detail: impl std::fmt::Display) -> String {
        let head = if self.case.is_empty() {
            String::new()
        } else {
            format!("[{}] ", self.case)
        };
        format!("{head}{detail}\n复现：{}", self.repro.cmd())
    }

    /// 改状态（直接摆一个状态，用于「四状态各画一帧」那类断言）。
    pub fn set_state(&mut self, state: UiState) {
        self.state = state;
    }

    // -- 一帧 ---------------------------------------------------------------

    /// 一帧：树 → 几何 → 绘制列表 → 裁剪快照，**并断言前置条件**。
    ///
    /// 前置断言在这个方法里、**每次**都做（不是「记得的时候做一次」）：
    /// 裁剪栈配平、`node_hint > 0`、裁剪快照非空、已登记 id 都有几何且在快照里。
    pub fn frame(&self) -> Result<Frame, String> {
        let (geo, list) = self.build_geo_list(&self.state);
        let clip = ClipSnapshot::from_draw_list(&list, &self.tree, &geo);
        let required: Vec<&str> = self.required_ids.iter().map(String::as_str).collect();
        check_preconditions(&list, &clip, &geo, &required).map_err(|e| self.carry(e))?;
        Ok(Frame {
            tree: self.tree.clone(),
            geo,
            list,
            clip,
            state: self.state.clone(),
        })
    }

    fn build_geo_list(&self, state: &UiState) -> (Geometry, DrawList) {
        match &self.engine {
            Some(e) => build_with(&self.tree, &self.theme, self.extent, state, &e.measure()),
            None => build_with(&self.tree, &self.theme, self.extent, state, &ApproxMeasure),
        }
    }

    // -- 输入注入 -----------------------------------------------------------

    /// 喂**一条**输入事件。返回「状态是否变化 + 产生了哪些 `UiEvent`」。
    pub fn send(&mut self, ev: &InputEvent) -> Result<Step, String> {
        let (tree, geo, clip) = {
            let f = self.frame()?;
            (f.tree, f.geo, f.clip)
        };
        let before = self.state.clone();
        let events = interaction::handle(&mut self.state, &tree, &geo, clip, ev);
        let changed = self.state != before;
        Ok(Step {
            input: ev.clone(),
            changed,
            events,
            state: self.state.clone(),
        })
    }

    /// 把指针移到 `id` 的中心（几何算出来，**不写死坐标**）。
    pub fn move_to(&mut self, id: &str) -> Result<Step, String> {
        // 先算坐标、再登记必查 id：这样「id 根本不存在」报的是**布局**那条消息
        // （更准），而不是笼统的「前置条件不成立」。
        let (x, y) = self.center_of(id)?;
        self.require_ids(&[id]);
        self.send(&InputEvent::PointerMoved { x, y })
    }

    /// `id` 在当前这一帧里的中心点（几何算出来）。
    pub fn center_of(&self, id: &str) -> Result<(f32, f32), String> {
        let f = self.frame()?;
        f.center(id).map_err(|e| self.carry(e))
    }

    /// 在 `id` 上**左键点一下**（move → down → up，返回合成结果）。
    ///
    /// 「合成结果」= 三步里**任一步**改了状态就算 `changed`；`events` 是三步的并集。
    /// 这是「点一下」的真实形状（真实鼠标就是三个事件）。
    pub fn tap(&mut self, id: &str) -> Result<Step, String> {
        let events_in = self.tap_events(id)?;
        let mut all = Vec::new();
        let mut changed = false;
        let mut last = InputEvent::PointerMoved { x: 0.0, y: 0.0 };
        for ev in events_in {
            let s = self.send(&ev)?;
            changed |= s.changed;
            all.extend(s.events);
            last = ev;
        }
        Ok(Step {
            input: last,
            changed,
            events: all,
            state: self.state.clone(),
        })
    }

    fn tap_events(&self, id: &str) -> Result<Vec<InputEvent>, String> {
        let (x, y) = self.center_of(id)?;
        Ok(vec![
            InputEvent::PointerMoved { x, y },
            InputEvent::PointerDown {
                button: PointerButton::Left,
                x,
                y,
            },
            InputEvent::PointerUp {
                button: PointerButton::Left,
                x,
                y,
            },
        ])
    }

    /// 重放一段脚本，**支持 `move @id`**（坐标由布局算出来）。
    ///
    /// 语法与 [`crate::input_script::parse_script`] **同一份**（`;` 分隔、`#` 注释、
    /// `move:X,Y` / `down:left` / `up:left` / `key:Tab` / `shift+key:Tab` / `text:hi` /
    /// `focus:off` / `wheel:0,3`），只是多了一条 `move @id`。
    ///
    /// # 逐条展开、整段解析（两件事各归其位）
    ///
    /// - `move @id` 的坐标取的是**那一条语句执行时**的布局（树内容变了、坐标跟着变），
    ///   于是「点了两次 `+`、计数变宽、按钮移位」这类情况不会静默点空；
    /// - 游标语义（`down`/`up` 的坐标 = **最近一次 `move`**）与**语句序号**都交给
    ///   `parse_script` 自己负责 —— 于是这里没有第二套语法实现，报错里的
    ///   「第 N 条语句」也一定是**用户脚本里**的那个 N。
    ///   （做法：把已展开的语句累积成 `resolved`，每加一条就把**整段**重新解析一次、
    ///   只喂最后一条事件。代价 O(n²) 次解析，n 是语句数。）
    ///
    /// `@id` 在那一帧没有几何 ⇒ **硬错**（按 id 定位的全部价值就是不许静默点空）。
    ///
    /// [`ScriptRun::resolved`] 是展开后的**纯坐标脚本**，可以直接喂给
    /// [`crate::input_script::replay`] 做交叉验证。
    pub fn run_script(&mut self, src: &str) -> Result<ScriptRun, String> {
        let mut run = ScriptRun::default();
        for (idx, raw) in src.split([';', '\n']).enumerate() {
            let stmt = raw.split('#').next().unwrap_or("").trim();
            if stmt.is_empty() {
                continue;
            }
            if let Some(rest) = stmt.strip_prefix("move @") {
                let id = rest.trim();
                // 先按当前布局算坐标（id 不存在时报的是「布局没有给它几何」，带语句序号），
                // 再把它登记为必查 id 并重验一次（几何 + `is_known` 都在这一步咬住）。
                let (x, y) = self.center_of(id).map_err(|e| {
                    self.carry(format!("第 {} 条语句 `{stmt}`：{e}", idx + 1))
                })?;
                self.require_ids(&[id]);
                self.frame()?;
                run.resolved.push_str(&format!("move:{x},{y};"));
            } else {
                run.resolved.push_str(stmt);
                run.resolved.push(';');
            }
            // 整段解析：错误里的「第 N 条语句」由解析器给出（= 用户脚本里的那个 N）。
            let events = input_script::parse_script(&run.resolved).map_err(|e| self.carry(e))?;
            let ev = events.into_iter().next_back().ok_or_else(|| {
                self.carry(format!("第 {} 条语句 `{stmt}` 没解析出事件", idx + 1))
            })?;
            let step = self.send(&ev)?;
            run.steps.push(step);
        }
        Ok(run)
    }

    // -- 离屏渲染与像素 -----------------------------------------------------

    /// 把**当前这一帧**用 CPU 后端画出来（真字形）⇒ [`Shot`]（带状态与节点矩形）。
    pub fn shoot(&mut self) -> Result<Shot, String> {
        self.shoot_named("")
    }

    /// 同上，带标签（打印里能看出是哪一帧）。
    pub fn shoot_named(&mut self, label: &str) -> Result<Shot, String> {
        let f = self.frame()?;
        let fb = self
            .cpu
            .render(self.extent, &f.list, self.clear)
            .map_err(|e| self.carry(format!("CPU 离屏渲染失败：{e}")))?;
        let mut rects = BTreeMap::new();
        for n in f.ordered_with_geometry() {
            if let Some(r) = f.rect_of(&n.id) {
                rects.insert(n.id.clone(), r);
            }
        }
        Ok(Shot {
            width: fb.width,
            height: fb.height,
            rgba: fb.pixels,
            state: f.state.clone(),
            rects,
            label: label.to_string(),
            repro: self.repro.clone(),
            case: self.case.clone(),
        })
    }

    /// 当前这一帧的 PNG 字节流。
    pub fn shoot_png(&mut self) -> Result<Vec<u8>, String> {
        self.shoot()?.to_png()
    }

    /// 当前这一帧写成 PNG 文件。
    pub fn shoot_png_file(&mut self, path: impl AsRef<Path>) -> Result<(), String> {
        self.shoot()?.write_png(path)
    }

    /// 用离屏 Vulkan 对照**当前这一帧**的绘制列表（`rule` 决定容差档）。
    ///
    /// 返回 `Ok(None)` = 本机没有可用的 Vulkan GPU（**跳过，不是通过**）。
    pub fn compare_frame_cpu_gpu(
        &self,
        rule: ParityRule,
    ) -> Result<Option<ParityReport>, String> {
        let f = self.frame()?;
        self.compare_cpu_gpu("当前帧", &f.list, rule)
    }

    /// 用离屏 Vulkan 对照给定的一份绘制列表。
    ///
    /// `font` 取自本面（有真字形就把真字形交给 GPU，否则纯形状）—— 所以 CPU 与 GPU
    /// 用的是**同源同字号**的字体。
    pub fn compare_cpu_gpu(
        &self,
        name: &str,
        list: &DrawList,
        rule: ParityRule,
    ) -> Result<Option<ParityReport>, String> {
        let font = self.font();
        let Some(mut probe) = GpuProbe::new(self.extent, self.clear, font)? else {
            return Ok(None);
        };
        let report = probe.compare(name, list, rule).map_err(|e| self.carry(e))?;
        Ok(Some(report))
    }

    /// 挑一份绘制列表该用哪档容差（看**这一帧真正要画的命令**）。
    pub fn rule_for(&self, list: &DrawList) -> ParityRule {
        ParityRule::for_list(list)
    }

    // -- 状态断言 -----------------------------------------------------------

    /// 逐字段断言「三个视觉字段」：`(hover, focus, pressed)`。
    pub fn assert_visual_state(
        &self,
        want_hover: Option<&str>,
        want_focus: Option<&str>,
        want_pressed: Option<&str>,
    ) -> Result<(), String> {
        let real = (
            self.state.hover.as_deref(),
            self.state.focus.as_deref(),
            self.state.pressed.as_deref(),
        );
        let want = (want_hover, want_focus, want_pressed);
        println!(
            "状态断言(hover,focus,pressed): 真实 {real:?}（texts={:?}）",
            self.state.texts
        );
        if real != want {
            return Err(self.carry(format!(
                "状态不符：真实 (hover,focus,pressed) = {real:?}，期望 {want:?}；texts = {:?}",
                self.state.texts
            )));
        }
        Ok(())
    }

    pub fn assert_hover(&self, want: Option<&str>) -> Result<(), String> {
        println!("状态断言 hover: 真实 {:?}，期望 {want:?}", self.state.hover);
        if self.state.hover.as_deref() != want {
            return Err(self.carry(format!(
                "hover 不符：真实 {:?}，期望 {want:?}",
                self.state.hover
            )));
        }
        Ok(())
    }

    pub fn assert_focus(&self, want: Option<&str>) -> Result<(), String> {
        println!("状态断言 focus: 真实 {:?}，期望 {want:?}", self.state.focus);
        if self.state.focus.as_deref() != want {
            return Err(self.carry(format!(
                "focus 不符：真实 {:?}，期望 {want:?}",
                self.state.focus
            )));
        }
        Ok(())
    }

    pub fn assert_pressed(&self, want: Option<&str>) -> Result<(), String> {
        println!(
            "状态断言 pressed: 真实 {:?}，期望 {want:?}",
            self.state.pressed
        );
        if self.state.pressed.as_deref() != want {
            return Err(self.carry(format!(
                "pressed 不符：真实 {:?}，期望 {want:?}",
                self.state.pressed
            )));
        }
        Ok(())
    }

    /// 某个输入框的文本缓冲（不在里面视为空串 —— 与 `handle` 的口径一致）。
    pub fn text_of(&self, id: &str) -> &str {
        self.state.texts.get(id).map(String::as_str).unwrap_or("")
    }

    /// 某个输入框的文本缓冲必须等于期望值。
    pub fn assert_text(&self, id: &str, want: &str) -> Result<(), String> {
        let got = self.text_of(id);
        println!("状态断言 texts[{id}]: 真实 {got:?}，期望 {want:?}");
        if got != want {
            return Err(self.carry(format!(
                "texts[{id}] 不符：真实 {got:?}，期望 {want:?}（整表 {:?}）",
                self.state.texts
            )));
        }
        Ok(())
    }

    /// 多处文本缓冲一次断言（**只**断言列出来的那些 id）。
    pub fn assert_texts(&self, want: &[(&str, &str)]) -> Result<(), String> {
        for (id, w) in want {
            self.assert_text(id, w)?;
        }
        Ok(())
    }

    /// `UiState` **逐字段全等**（含 `texts` 整表）。
    pub fn assert_state(&self, want: &UiState) -> Result<(), String> {
        println!("状态断言（全等）: 真实 {:?}", self.state);
        if &self.state != want {
            return Err(self.carry(format!(
                "状态不符：真实 {:?}\n              期望 {want:?}",
                self.state
            )));
        }
        Ok(())
    }

    /// 焦点树序必须**恰好**是这些（`Tab` 循环的依据）。
    pub fn assert_focus_order(&self, want: &[&str]) -> Result<(), String> {
        let got = interaction::focusables(&self.tree);
        println!("焦点树序: 真实 {got:?}，期望 {want:?}");
        let w: Vec<String> = want.iter().map(|s| (*s).to_string()).collect();
        if got != w {
            return Err(self.carry(format!("焦点树序不符：真实 {got:?}，期望 {w:?}")));
        }
        Ok(())
    }

    /// 焦点树序里**不许**出现这些 id（禁用子树必须整棵不在内）。
    pub fn assert_focus_order_excludes(&self, forbidden: &[&str]) -> Result<(), String> {
        let got = interaction::focusables(&self.tree);
        println!("焦点树序: 真实 {got:?}（不许含 {forbidden:?}）");
        for id in forbidden {
            if got.iter().any(|g| g == id) {
                return Err(self.carry(format!(
                    "焦点树序里不该有 `{id}`（禁用节点不该收输入），真实 {got:?}"
                )));
            }
        }
        Ok(())
    }

    // -- 绘制列表断言 -------------------------------------------------------

    pub fn assert_command_count(&self, f: &Frame, want: usize) -> Result<(), String> {
        println!("绘制列表断言 命令数: 真实 {}，期望 {want}", f.list.len());
        if f.list.len() != want {
            return Err(self.carry(format!(
                "命令数不符：真实 {}，期望 {want}（计数明细 {:?}）",
                f.list.len(),
                f.counts()
            )));
        }
        Ok(())
    }

    pub fn assert_node_hint_count(&self, f: &Frame, want: usize) -> Result<(), String> {
        let got = f.counts().node_hint;
        println!("绘制列表断言 NodeHint 数: 真实 {got}，期望 {want}");
        if got != want {
            return Err(self.carry(format!(
                "NodeHint 数不符：真实 {got}，期望 {want}（有几何的节点 {} 个）",
                f.ordered_with_geometry().len()
            )));
        }
        Ok(())
    }

    /// 裁剪快照里的节点数。
    pub fn assert_clip_nodes(&self, f: &Frame, want: usize) -> Result<(), String> {
        println!("绘制列表断言 裁剪快照节点数: 真实 {}，期望 {want}", f.clip.len());
        if f.clip.len() != want {
            return Err(self.carry(format!(
                "裁剪快照节点数不符：真实 {}，期望 {want}（ids {:?}）",
                f.clip.len(),
                f.clip.ids().collect::<Vec<_>>()
            )));
        }
        Ok(())
    }

    /// 裁剪快照里必须有这些 id（前置断言的**显式**版，用于「这一刻我关心这几个人」）。
    pub fn assert_clip_known(&self, f: &Frame, ids: &[&str]) -> Result<(), String> {
        println!(
            "绘制列表断言 裁剪快照 is_known: ids {:?}",
            f.clip.ids().collect::<Vec<_>>()
        );
        check_preconditions(&f.list, &f.clip, &f.geo, ids).map_err(|e| self.carry(e))
    }

    /// 逐个字段断言命令计数。
    pub fn assert_counts(&self, f: &Frame, want: DrawCounts) -> Result<(), String> {
        let got = f.counts();
        println!("绘制列表断言 计数明细: 真实 {got:?}，期望 {want:?}");
        if got != want {
            return Err(self.carry(format!("命令计数不符：真实 {got:?}，期望 {want:?}")));
        }
        Ok(())
    }

    /// 某个节点**画出来的文本**必须等于期望值（读的是绘制列表，不是树）。
    pub fn assert_drawn_text(&self, f: &Frame, id: &str, want: &str) -> Result<(), String> {
        let drawn = f.drawn_texts().map_err(|e| self.carry(e))?;
        let got = drawn.get(id);
        println!("绘制列表断言 画出来的文本[{id}]: 真实 {got:?}，期望 {want:?}（整帧 {drawn:?}）");
        match got {
            Some(t) if t == want => Ok(()),
            Some(t) => Err(self.carry(format!(
                "`{id}` 画出来的文本是 `{t}`，期望 `{want}` —— 期望的内容**没有**走到渲染这一步"
            ))),
            None => Err(self.carry(format!(
                "绘制列表里没有 `{id}` 的文本命令（期望 `{want}`）—— \
                 检查 `{id}` 是不是文本类节点、有没有几何、`NodeHint` 是否齐全"
            ))),
        }
    }

    /// 某个节点画出来的文本必须**含**某个子串（输入框内容是追加语义，用 `contains` 更贴切）。
    pub fn assert_drawn_text_contains(&self, f: &Frame, id: &str, needle: &str) -> Result<(), String> {
        let drawn = f.drawn_texts().map_err(|e| self.carry(e))?;
        let got = drawn.get(id).cloned().unwrap_or_default();
        println!("绘制列表断言 画出来的文本[{id}]: 真实 {got:?}，期望含 {needle:?}");
        if !got.contains(needle) {
            return Err(self.carry(format!(
                "`{id}` 画出来的文本是 `{got}`，不含 `{needle}`"
            )));
        }
        Ok(())
    }

    /// 某个节点画出来的文本**字号**必须等于期望值。
    pub fn assert_text_size(&self, f: &Frame, id: &str, want: f32) -> Result<(), String> {
        let sizes = f.drawn_text_sizes().map_err(|e| self.carry(e))?;
        let got = sizes.get(id).cloned().unwrap_or_default();
        println!("绘制列表断言 文本字号[{id}]: 真实 {got:?}，期望 {want}");
        if got.len() != 1 {
            return Err(self.carry(format!(
                "`{id}` 的 `Text` 命令有 {} 条（期望恰好 1 条），字号 {got:?}",
                got.len()
            )));
        }
        if got[0] != want {
            return Err(self.carry(format!(
                "`{id}` 的文本字号是 {}，期望 {want} —— 字号三处不一致时\
                 「布局算出来的宽度」与「画出来的宽度」会漂",
                got[0]
            )));
        }
        Ok(())
    }
}

/// 布局 → 绘制列表（**一份**实现，两条度量路径共用）。
fn build_with<M: Measure>(
    tree: &Node,
    theme: &Theme,
    extent: Extent,
    state: &UiState,
    measure: &M,
) -> (Geometry, DrawList) {
    let geo = layout::layout(
        tree,
        Rect::new(0.0, 0.0, extent.width as f32, extent.height as f32),
        TextStyle {
            font_size: theme.font_size,
            line_height: theme.line_height,
        },
        measure,
    );
    // `UiState` 是交互层的真相；渲染层只认它的**只读子集** `InteractState`。
    let interact = InteractState {
        hover: state.hover.clone(),
        focus: state.focus.clone(),
        pressed: state.pressed.clone(),
    };
    // `FieldText::Content` ⇒ 输入框画的是 `state.texts[id]`；
    // `InteractiveRenderer` 会给**每个有几何的节点**发一条 `NodeHint` ⇒ 裁剪快照非空。
    let list = InteractiveRenderer::with_texts(
        theme.clone(),
        measure,
        &interact,
        FieldText::Content,
        &state.texts,
    )
    .build(tree, &geo);
    (geo, list)
}

// ---------------------------------------------------------------------------
// 八、窗口 e2e（**只含可判定的那半**）
// ---------------------------------------------------------------------------

/// 窗口档的**门槛与自证**。
///
/// # 为什么这里没有「建真窗口跑 N 帧」的 runner
///
/// 本项目现在的窗口测试全是 `examples/`（`required-features = ["window"]`），
/// 不是 `#[test]`：**winit 要求事件循环在主线程**，而 `cargo test` 的测试线程不是主线程。
/// 一个「库里的 window runner」只能被 example 调用，而 example **跑不进 `cargo test`**
/// ⇒ 它永远没有护栏。本项目的纪律是「护栏必须有判据」，所以这里**只放能被
/// `cargo test` 咬住的那半**（门槛判定 + 帧数解析 + 自证标记），
/// 「建窗跑 N 帧」继续由 example 承担，**配方与命令写在 `docs/features/testing.md`**。
///
/// 这是**明确的未做项**，不是「顺手省掉」：见报告的「未做项」一节。
pub mod window {
    use super::{print_gate, SKIP_MARKER};

    /// 窗口测试门槛（与既有 example 用**同一个**变量名 —— 一处定义，不许分叉）。
    pub const GATE: &str = "DEER_VK_WINDOW_TESTS";

    /// 帧数门槛（同上）。
    pub const FRAMES_ENV: &str = "DEER_VK_FRAMES";

    /// 窗口测试门槛是否已设（带自证标记）。
    pub fn gate() -> bool {
        print_gate(GATE)
    }

    /// [`gate`] 的静默版（只读，不打印）。
    pub fn gate_quiet() -> bool {
        super::gate(GATE)
    }

    /// 门控跳过（打印 `TESTKIT-SKIP`，返回 `false` 表示「跳过」）。
    pub fn require() -> bool {
        if gate() {
            return true;
        }
        println!("{SKIP_MARKER} {GATE} 未启用 ⇒ 窗口档**跳过**（这是跳过，不是通过）");
        false
    }

    /// 从 [`FRAMES_ENV`] 读帧数（**先 `trim()` 再 parse**）。
    ///
    /// 不 `trim()` 的话 `set DEER_VK_FRAMES=3 && …`（值是 `"3 "`）会让 `parse()` 失败
    /// ⇒ **静默退回默认帧数**：你要求 3 帧，它跑了别的帧数，却什么也不说。
    /// 解析不出来时**明确报错**（不静默退回）。
    ///
    /// 判定逻辑在 [`frames_from_value`]（纯函数 ⇒ 能单测）；这里只负责读环境。
    pub fn frames_from_env(default: u64) -> Result<u64, String> {
        let raw = std::env::var(FRAMES_ENV).ok();
        let n = frames_from_value(raw.as_deref(), default)?;
        match raw {
            Some(v) => println!("{SKIP_MARKER} {FRAMES_ENV}={v:?} ⇒ 帧数 {n}"),
            None => println!("{SKIP_MARKER} {FRAMES_ENV} 未设 ⇒ 用默认帧数 {n}"),
        }
        Ok(n)
    }

    /// [`frames_from_env`] 的**纯逻辑**部分（不读环境 ⇒ 可单测）。
    ///
    /// `None` / 空串（trim 后）⇒ `default`（至少 1）；能 parse 成 ≥1 的整数 ⇒ 它；
    /// 其它 ⇒ `Err`（**不静默退回默认**）。
    pub fn frames_from_value(raw: Option<&str>, default: u64) -> Result<u64, String> {
        let Some(v) = raw.map(str::trim).filter(|s| !s.is_empty()) else {
            return Ok(default.max(1));
        };
        let n: u64 = v
            .parse()
            .map_err(|e| format!("{FRAMES_ENV}={raw:?} 不是帧数（trim 后是 {v:?}）：{e}"))?;
        if n == 0 {
            return Err(format!("{FRAMES_ENV}={raw:?} ⇒ 0 帧没有意义（至少 1 帧）"));
        }
        Ok(n)
    }
}

// ---------------------------------------------------------------------------
// 九、testkit 自身的测试（**每个断言助手都有一条「它会红」的反向自检**）
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use deer_gpu::draw::DrawCounts;
    use deer_layout::builder::L;
    use deer_layout::node::{Kind, LayoutProps};

    /// 计数显示的前缀（与 `examples/counter.rs` 的 `COUNT_PREFIX` 逐字相同）。
    const PREFIX: &str = "count = ";

    /// 与 `examples/counter.rs` 的 `BUILTIN_SCRIPT` 逐字相同的脚本：
    /// `+` 两次 + 输入框打字 `ok`。用它当「重写前后结论一致」的语料。
    const BUILTIN: &str = "move @plus;down:left;up:left;move @plus;down:left;up:left;\
                           move @input;down:left;up:left;text:ok";

    /// 与 `examples/counter.rs::counter_tree` **逐字同构**的界面树。
    fn counter_tree(count: i32) -> Node {
        let app = Node::new(Kind::Column, "app").with_layout(LayoutProps {
            padding: 12.0,
            gap: 10.0,
            ..Default::default()
        });
        let bar = Node::new(Kind::Row, "bar")
            .with_layout(L::new().w(300.0).gap(8.0).to_props())
            .push(Node::new(Kind::Button, "plus").with_label("+"))
            .push(Node::new(Kind::Button, "minus").with_label("-"));
        let info = Node::new(Kind::Row, "info")
            .with_layout(L::new().w(300.0).gap(8.0).to_props())
            .push(Node::new(Kind::Text, "count").with_label(format!("{PREFIX}{count}")))
            .push(Node::new(Kind::Field, "input").with_label("type here"));
        app.push(Node::new(Kind::Text, "title").with_label("deer-gui counter"))
            .push(bar)
            .push(info)
    }

    /// 全不透明主题 ⇒ 这一帧的语料是 [`ParityRule::Opaque`]（逐字节 0）。
    fn opaque_theme() -> Theme {
        Theme {
            surface: Color::rgb(0x14, 0x16, 0x20),
            ..Theme::default()
        }
    }

    /// 与 counter 同一套常量：清屏色 + 窗口尺寸。
    const CLEAR: Color = Color::rgb(0x08, 0x09, 0x0c);
    const W: u32 = 360;
    const H: u32 = 200;

    fn harness(count: i32) -> Harness {
        let mut h = Harness::new(counter_tree(count), W, H, opaque_theme());
        h.set_clear(CLEAR);
        h
    }

    /// 整棵树的所有 id（前序）。
    fn all_ids(n: &Node) -> Vec<String> {
        fn walk(n: &Node, out: &mut Vec<String>) {
            out.push(n.id.clone());
            for c in &n.children {
                walk(c, out);
            }
        }
        let mut out = Vec::new();
        walk(n, &mut out);
        out
    }

    /// 没有真字形 ⇒ 打印跳过说明并返回 `true`（调用方 `return`）。
    ///
    /// 这是本项目既有的「明确降级、不假装通过」口径（`counter.rs` 的离屏自检同款）。
    fn skip_without_glyphs(h: &Harness) -> bool {
        if h.has_glyph_pixels() {
            return false;
        }
        println!(
            "{SKIP_MARKER} 本机没有真字形（{}）⇒ 跳过像素断言（这是跳过，不是通过）",
            h.font_note()
        );
        true
    }

    fn expect_err_carried(name: &str, needle: &str, r: Result<(), String>) {
        match r {
            Ok(()) => panic!(
                "反向自检失败：`{name}` 在**错误的期望值**下仍然返回 `Ok` \
                 ⇒ 它永远不会红，不是护栏（请给它补一条真的会红的用例）"
            ),
            Err(e) => {
                assert!(
                    e.contains(needle),
                    "`{name}` 的错误文案该含 `{needle}`，实际：{e}"
                );
                assert!(
                    e.contains("复现："),
                    "`{name}` 的错误信息必须带**可复制的复现命令**，实际：{e}"
                );
                println!("反向自检 ✅ `{name}` 会红，且带复现命令：\n{e}\n");
            }
        }
    }

    fn expect_err_raw(name: &str, needle: &str, r: Result<(), String>) {
        match r {
            Ok(()) => panic!("反向自检失败：`{name}`（纯函数判据）在错误输入下返回了 `Ok`"),
            Err(e) => {
                assert!(
                    e.contains(needle),
                    "`{name}` 的错误文案该含 `{needle}`，实际：{e}"
                );
                println!("反向自检 ✅ `{name}` 会红：{e}\n");
            }
        }
    }

    // -- 建面 / 一帧 / 前置断言 --------------------------------------------

    /// 建面：`Builder` 与 `Node` 两条入口都要能用（[`IntoTree`]），
    /// 且 `frame()` 拿到的**真实数字**要能打印出来（先 dump 再写期望值）。
    #[test]
    fn harness_accepts_builder_or_node_and_frame_reports_real_numbers() {
        let mut b = Builder::new(Kind::Column, "app");
        let bid = b.button("x");
        let by_builder = Harness::new(&b, 120, 60, opaque_theme());
        assert_eq!(by_builder.tree().id, "app");
        assert!(!bid.is_empty());

        let by_node = Harness::new(counter_tree(0), W, H, opaque_theme());
        assert_eq!(by_node.tree().id, "app");

        let h = harness(0);
        let f = h.frame().expect("前置条件必须成立");
        println!(
            "真实数据：命令 {} 条，计数 {:?}，有几何的节点 {}，裁剪快照 {}",
            f.list.len(),
            f.counts(),
            f.ordered_with_geometry().len(),
            f.clip.len()
        );
        // 下列数字是**实测**（见报告里的原始输出），不是估的。
        h.assert_command_count(&f, 19).unwrap();
        h.assert_node_hint_count(&f, 8).unwrap();
        h.assert_clip_nodes(&f, 8).unwrap();
        assert_eq!(f.counts().node_hint, f.ordered_with_geometry().len());
        // 同一份输入 ⇒ 同一份输出（后面所有断言都建立在这条确定性之上）。
        assert_eq!(f, h.frame().unwrap(), "同一状态下的两帧必须逐字段相同");
    }

    /// **前置断言**：登记了不存在的 id ⇒ `frame()` 必须红（而且带复现命令）。
    #[test]
    fn frame_goes_red_when_a_required_id_has_no_geometry() {
        let mut h = harness(0);
        h.require_ids(&["nope"]);
        expect_err_carried(
            "Harness::frame（登记的 id 没有几何）",
            "前置条件不成立",
            h.frame().map(|_| ()),
        );
    }

    /// `DefaultRenderer`（`build_draw_list`）**不发 `NodeHint`** ⇒ 前置断言必须红。
    ///
    /// 这是那条「抄漏一处就静默全不裁剪」的坑的**正面护栏**：
    /// 同一个界面树，用不发提示的渲染器 ⇒ 裁剪快照为空 ⇒ 命中退化，而**代码不会报错**。
    #[test]
    fn preconditions_reject_a_list_without_node_hints() {
        let h = harness(0);
        let good = h.frame().expect("InteractiveRenderer 的帧必须过前置");
        let theme = h.theme().clone();
        let measure = ApproxMeasure;
        let tree = counter_tree(0);
        let geo = layout::layout(
            &tree,
            Rect::new(0.0, 0.0, W as f32, H as f32),
            TextStyle {
                font_size: theme.font_size,
                line_height: theme.line_height,
            },
            &measure,
        );
        let plain = deer_gpu::build_draw_list(&tree, &geo, theme, &measure);
        let clip = ClipSnapshot::from_draw_list(&plain, &tree, &geo);
        println!(
            "DefaultRenderer 的列表：{} 条命令，NodeHint {}，裁剪快照 {}",
            plain.len(),
            plain.counts().node_hint,
            clip.len()
        );
        expect_err_raw(
            "check_preconditions（没有任何 NodeHint）",
            "全不裁剪",
            check_preconditions(&plain, &clip, &geo, &[]),
        );
        // 反过来：同一棵树用 InteractiveRenderer ⇒ 必须**过**（判据是能分辨的，不是恒红）。
        check_preconditions(&good.list, &good.clip, &good.geo, &["plus"])
            .expect("InteractiveRenderer 的帧必须过前置");
    }

    /// 关键 id **不在裁剪快照里** ⇒ 红（`allows()` 对未知 id 放行 ⇒ 护栏静默失效）。
    #[test]
    fn preconditions_reject_an_id_outside_the_clip_snapshot() {
        let h = harness(0);
        let f = h.frame().unwrap();
        // 一个「非空、但没有 `plus`」的快照 ⇒ 绕开「快照为空」那条，直接打中 `is_known` 那条。
        let clip = ClipSnapshot::unclipped().with_node_clip("other", None);
        expect_err_raw(
            "check_preconditions（id 不在快照里）",
            "静默失效",
            check_preconditions(&f.list, &clip, &f.geo, &["plus"]),
        );
    }

    /// 裁剪栈不配平 ⇒ 红（CPU/GPU 两条后端都对它报错，前置也要提前拦）。
    #[test]
    fn preconditions_reject_unbalanced_clip() {
        let h = harness(0);
        let f = h.frame().unwrap();
        let mut bad = DrawList::new();
        bad.push(DrawCmd::PushClip {
            rect: RectI::new(1, 1, 5, 5),
        });
        expect_err_raw(
            "check_preconditions（裁剪栈不配平）",
            "裁剪栈不平衡",
            check_preconditions(&bad, &f.clip, &f.geo, &[]),
        );
    }

    /// `drawn_texts` 的绑定口径被破坏时**报错**，不静默给错映射。
    #[test]
    fn drawn_texts_reports_a_broken_binding_instead_of_lying() {
        let h = harness(0);
        let theme = h.theme().clone();
        let measure = ApproxMeasure;
        let tree = counter_tree(0);
        let geo = layout::layout(
            &tree,
            Rect::new(0.0, 0.0, W as f32, H as f32),
            TextStyle {
                font_size: theme.font_size,
                line_height: theme.line_height,
            },
            &measure,
        );
        let list = deer_gpu::build_draw_list(&tree, &geo, theme, &measure);
        let clip = ClipSnapshot::from_draw_list(&list, &tree, &geo);
        let f = Frame {
            tree,
            geo,
            list,
            clip,
            state: UiState::default(),
        };
        expect_err_raw(
            "Frame::drawn_texts（文本出现在任何 NodeHint 之前）",
            "之前",
            f.drawn_texts().map(|_| ()),
        );
        // 反过来：正常的帧必须给出**逐字正确**的映射。
        let good = h.frame().unwrap();
        let drawn = good.drawn_texts().unwrap();
        println!("正常帧画出来的文本 = {drawn:?}");
        assert_eq!(drawn.get("count").map(String::as_str), Some("count = 0"));
        assert_eq!(drawn.get("plus").map(String::as_str), Some("+"));
        assert_eq!(drawn.get("title").map(String::as_str), Some("deer-gui counter"));
    }

    // -- 输入注入 -----------------------------------------------------------

    #[test]
    fn tap_produces_a_click_and_changes_state() {
        let mut h = harness(0);
        let before = h.state().clone();
        let step = h.tap("plus").expect("点在 plus 上必须成功");
        println!("tap(plus) ⇒ 状态变了={} events={:?}", step.changed, step.events);
        assert!(step.changed, "点一下必然改状态（hover/focus/pressed 至少一个）");
        assert!(
            step.events
                .iter()
                .any(|e| matches!(e, UiEvent::Clicked(id) if id == "plus")),
            "「点一下」必须产出 Clicked(plus)，实际 {:?}",
            step.events
        );
        assert_ne!(&before, h.state());
        h.assert_hover(Some("plus")).unwrap();
        h.assert_focus(Some("plus")).unwrap();
        h.assert_pressed(None).unwrap();
    }

    #[test]
    fn tap_on_an_unknown_id_is_a_hard_error() {
        let mut h = harness(0);
        expect_err_carried(
            "Harness::tap（不存在的 id）",
            "布局没有给",
            h.tap("nope").map(|_| ()),
        );
    }

    #[test]
    fn run_script_supports_move_at_id_and_keeps_the_input_script_semantics() {
        let mut h = harness(0);
        let run = h.run_script(BUILTIN).expect("内置脚本必须重放成功");
        println!("解析并展开后的坐标脚本 = {}", run.resolved);
        assert_eq!(run.steps.len(), 10, "内置脚本有 10 条语句");
        assert!(
            !run.resolved.contains('@'),
            "展开后的脚本不该再有 `@id`：{}",
            run.resolved
        );
        let clicked: Vec<String> = run
            .events()
            .into_iter()
            .filter_map(|e| match e {
                UiEvent::Clicked(id) => Some(id),
                _ => None,
            })
            .collect();
        assert_eq!(clicked, vec!["plus", "plus", "input"], "两次 +、一次输入框");
        h.assert_text("input", "ok").unwrap();
        h.assert_visual_state(Some("input"), Some("input"), None).unwrap();

        // **与既有实现同口径**：`input_script::replay` 重放同一段**展开后**的脚本，
        // 终态必须逐字段相同（testkit 没有另立一套状态机语义）。
        let f = h.frame().unwrap();
        let snap = f.clip.clone();
        let replayed =
            input_script::replay(&run.resolved, &f.tree, &f.geo, snap).expect("既有重放路径");
        println!(
            "testkit 终态 {:?}\n既有 replay 终态 {:?}（重绘序列 {:?}）",
            h.state(),
            replayed.state,
            replayed.redrew
        );
        assert_eq!(
            h.state(),
            &replayed.state,
            "testkit 的脚本重放终态必须与 `input_script::replay` 逐字段相同"
        );
    }

    #[test]
    fn run_script_rejects_unknown_ids_and_syntax_errors() {
        let mut a = harness(0);
        expect_err_carried(
            "Harness::run_script（`move @` 的 id 不存在）",
            "第 1 条语句",
            a.run_script("move @nope").map(|_| ()),
        );
        let mut b = harness(0);
        expect_err_carried(
            "Harness::run_script（语法错）",
            "第 1 条语句",
            b.run_script("key:bogus").map(|_| ()),
        );
        let mut c = harness(0);
        expect_err_carried(
            "Harness::run_script（第 3 条语句错，序号要指对）",
            "第 3 条语句",
            c.run_script("move @plus;down:left;nope:1").map(|_| ()),
        );
    }

    // -- 状态断言 -----------------------------------------------------------

    /// 禁用子树的整棵都不在焦点序里（`focusables` 的既有语义），且助手能分辨。
    #[test]
    fn focus_order_skips_the_disabled_subtree_and_the_assertion_can_tell() {
        let tree = counter_tree(0).push(
            Node::new(Kind::Button, "ghost")
                .with_label("ghost")
                .disabled(),
        );
        let h = Harness::new(tree, W, H, opaque_theme());
        h.assert_focus_order(&["plus", "minus", "input"]).unwrap();
        h.assert_focus_order_excludes(&["ghost"]).unwrap();
        expect_err_carried(
            "Harness::assert_focus_order_excludes（禁用的 id 出现在序里）",
            "不该有",
            h.assert_focus_order_excludes(&["plus"]),
        );
    }

    // -- 像素 ---------------------------------------------------------------

    /// **计数的像素闭环**：`+` 点一次 ⇒ 绘制列表画出来的文本变 ⇒ `count` 矩形内像素变，
    /// 且**框外为 0**。
    #[test]
    fn tapping_plus_changes_the_count_pixels_and_nothing_outside() {
        let mut h = harness(0);
        if skip_without_glyphs(&h) {
            return;
        }
        h.require_glyph_pixels().unwrap();
        let before = h.shoot_named("count=0").unwrap();
        let step = h.tap("plus").unwrap();
        assert!(step.changed);
        h.set_tree(counter_tree(1));
        let after = h.shoot_named("count=1").unwrap();
        let f = h.frame().unwrap();
        h.assert_drawn_text(&f, "count", "count = 1").unwrap();
        after
            .assert_state_change_only(&before, "count")
            .expect("计数变化的像素必须只落在 count（与真的变了的按钮）矩形内");
    }

    /// **整段脚本的重写版**：与 `counter.rs::headless_selfcheck` 同一批结论
    /// （终态 `count = 2` + 输入框画出 `ok`），且像素判据的期望矩形由 testkit **自己算**
    /// —— 这一步会带上「输入框的文本也变了」，手抄矩形很容易漏（漏了就是一条假红）。
    #[test]
    fn full_script_run_reaches_the_same_conclusion_as_the_hand_written_selfcheck() {
        let mut h = harness(0);
        if skip_without_glyphs(&h) {
            return;
        }
        let before = h.shoot_named("初始 count=0").unwrap();
        let run = h.run_script(BUILTIN).unwrap();
        h.set_tree(counter_tree(2));
        let after = h.shoot_named("终态 count=2").unwrap();

        let f = h.frame().unwrap();
        h.assert_drawn_text(&f, "count", "count = 2").unwrap();
        h.assert_drawn_text_contains(&f, "input", "ok").unwrap();
        h.assert_text("input", "ok").unwrap();
        h.assert_visual_state(Some("input"), Some("input"), None).unwrap();
        println!(
            "脚本重放账本：{} 步，其中改了状态的 {} 步",
            run.steps.len(),
            run.changed_steps()
        );
        // **实测**：内置脚本 10 条语句里 9 条改了状态 —— 第 4 条（第二次 `move @plus`）
        // 是**空操作**（指针已经在 `plus` 上）⇒ 状态没变 ⇒ 窗口路径下这一帧不会重绘。
        // 这正是「dirty 由状态变化决定、不由有没有事件决定」那条纪律的形状。
        assert_eq!(run.changed_steps(), 9);
        assert!(
            !run.steps[3].changed,
            "第 4 条语句是重复的 move @plus，不该改状态：{:?}",
            run.steps[3]
        );

        // 主矩形 = `count`；testkit 自动把「视觉真的变了的节点」（这里是 `input`，
        // 它的文本从占位变成 `ok`、且它拿到了焦点）也纳入期望矩形。
        after
            .assert_state_change_only(&before, "count")
            .expect("终态像素差异必须只落在 count 与真的变了的节点矩形内");
    }

    /// **差异算的是像素不是字节**（一个像素 4 通道 ⇒ 混用会让数字虚高 4 倍）。
    #[test]
    fn diff_split_counts_pixels_not_bytes() {
        let mut h = harness(0);
        if skip_without_glyphs(&h) {
            return;
        }
        let a = h.shoot().unwrap();
        let mut b = a.clone();
        b.rgba[0] = b.rgba[0].wrapping_add(1);
        b.rgba[4] = b.rgba[4].wrapping_add(1);
        let d = b.diff_split(&a, &[]).unwrap();
        assert_eq!(d.total, 2, "只有 2 个像素变了（不是 2 个字节就对了）");
        assert_eq!(d.outside, 2);
        assert_eq!(d.inside_total(), 0);

        // 落在矩形里的要算进对应的那一块。
        let r = RectI::new(0, 0, 2, 1);
        let d2 = b.diff_split(&a, &[r]).unwrap();
        assert_eq!(d2.inside, vec![2]);
        assert_eq!(d2.outside, 0);
    }

    /// **D6 绕过（复审 HIGH-1）**：状态差落在**根容器**上时，自动补齐不得把期望矩形放大到整块画布。
    ///
    /// ## 为什么必须有这条（复审给的反例）
    ///
    /// `expected_rects` 的自动补齐**只看状态差**（`hover`/`focus`/`pressed` 的成员变化）。
    /// 于是只要这次比较里有一个**画布级节点**的交互态变了（最典型：`hover` 落在根容器上），
    /// 自动补齐就把整块画布放行 ⇒ 任何**与状态无关**的远端差异（例如另一处数据变化）
    /// 都变成「框内」⇒ **判据等于没有**。复审的原始探针：同一批 584 个差异像素，
    /// 对照1（hover 在小控件上）**抓到**、对照2（hover 在根容器上）**逃过**。
    ///
    /// 本用例把对照2 固化：`btn` 之外的一个像素变了 ⇒ 必须被抓住（**Err**）。
    #[test]
    fn root_level_hover_cannot_widen_the_auto_expected_rects() {
        // 根容器**给显式尺寸** ⇒ 它的几何就是整块画布（复现反例的前提）
        let tree = Node::new(Kind::Column, "app")
            .with_layout(L::new().w(160.0).h(120.0).pad(4.0).to_props())
            .push(Node::new(Kind::Button, "btn").with_label("x"));
        let mut h = Harness::new(tree, 160, 120, opaque_theme());
        h.set_clear(CLEAR);

        let a = h.shoot_named("A：hover 在小控件上").unwrap();
        let f = h.frame().unwrap();
        // ★ 前置断言（前置不成立不会报错 ⇒ 护栏会悄悄失效）
        let app = f.rect_of("app").expect("根容器必须有几何");
        let btn = f.rect_of("btn").expect("按钮必须有几何");
        assert!(
            app.w >= 160 && app.h >= 120,
            "前置条件：根容器几何必须 = 整块画布（否则本用例证明不了绕过）；实测 {app:?}"
        );
        assert!(
            btn.w < app.w / 2 && btn.h < app.h / 2,
            "前置条件：按钮必须**明显小于**画布（否则「放大到整块画布」无从谈起）；实测 {btn:?}"
        );

        // B：状态差落在**根容器**上（这是反例的关键），差异像素落在 `btn` **之外**
        let mut b = a.clone();
        b.state.hover = Some("app".into());
        b.label = "B：hover 在根容器上".into();
        // 模拟「与状态无关的远端差异」：改左上角（在 btn 之外）的一个像素
        b.rgba[0] = b.rgba[0].wrapping_add(9);
        assert!(
            !(0 >= btn.x && 0 < btn.x + btn.w && 0 >= btn.y && 0 < btn.y + btn.h),
            "前置条件：被改的像素必须落在 `btn` 之外（否则这条断言没有意义）"
        );

        expect_err_carried(
            "Shot::assert_diff_only_inside（hover 落在根容器上 ⇒ 自动补齐必须被面积棘轮拦住）",
            "棘轮",
            b.assert_diff_only_inside(&a, "btn"),
        );
        // 对照1 的判别力不能退化：**只**把 hover 放在小控件上、差异仍在外 ⇒ 必须红
        let mut c = a.clone();
        c.state.hover = Some("btn".into());
        c.rgba[0] = c.rgba[0].wrapping_add(9);
        expect_err_carried(
            "对照1：hover 在小控件上、差异在框外仍必须红",
            "之外",
            c.assert_diff_only_inside(&a, "btn"),
        );
    }

    /// **框外必须为 0**：让差异落在框外 ⇒ 红。
    #[test]
    fn leaked_changes_outside_the_expected_rects_go_red() {
        let mut h = harness(0);
        if skip_without_glyphs(&h) {
            return;
        }
        let a = h.shoot().unwrap();
        let mut b = a.clone();
        let last = b.rgba.len() - 4;
        b.rgba[last] = b.rgba[last].wrapping_add(7);
        let count_rect = h.frame().unwrap().rect_of("count").unwrap();
        expect_err_carried(
            "Shot::assert_no_diff_outside（差异溢出期望矩形）",
            "之外",
            b.assert_no_diff_outside(&a, &[count_rect]),
        );
        expect_err_carried(
            "Shot::assert_diff_only_inside（差异溢出期望矩形）",
            "之外",
            b.assert_diff_only_inside(&a, "count"),
        );
        expect_err_carried(
            "Shot::assert_bytes_eq（两帧本该逐字节相同）",
            "逐字节相同",
            b.assert_bytes_eq(&a),
        );
        // 反过来：一模一样的两帧必须过。
        a.assert_bytes_eq(&a).unwrap();
        a.assert_no_diff_outside(&a, &[]).unwrap();
    }

    /// 差异真的发生了但**主矩形内一个像素都没变** ⇒ 红（「它没画出来」那条）。
    #[test]
    fn a_missing_change_inside_the_primary_rect_goes_red() {
        let mut h = harness(0);
        if skip_without_glyphs(&h) {
            return;
        }
        let a = h.shoot_named("同一状态").unwrap();
        let b = h.shoot_named("同一状态（第二张）").unwrap();
        expect_err_carried(
            "Shot::assert_state_change_only（主矩形内 0 像素变化）",
            "一个像素都没变",
            b.assert_state_change_only(&a, "count"),
        );
        expect_err_carried(
            "Shot::assert_diff_only_inside（primary 没有几何）",
            "没有几何",
            b.assert_diff_only_inside(&a, "nope"),
        );
    }

    /// 清屏色是**真的**清屏色：没盖住的地方就是它。
    #[test]
    fn set_clear_is_what_the_uncovered_area_shows() {
        // ⚠️ 根容器**必须**给显式内边距才会画底/描边
        // （`interact.rs`：只有 `layout.padding > 0` 的 `Column`/`Row` 才画 surface + border）
        // —— 不给的话 (0,0) 就是清屏色，这条测试就什么也证明不了。
        let tree = Node::new(Kind::Column, "root")
            .with_layout(L::new().w(40.0).h(30.0).pad(4.0).to_props())
            .push(Node::new(Kind::Button, "b").with_label("x"));
        let mut h = Harness::new(tree, 120, 80, opaque_theme());
        if skip_without_glyphs(&h) {
            return;
        }
        h.set_clear(Color::rgb(1, 2, 3));
        let a = h.shoot_named("clear=1,2,3").unwrap();
        println!("右下角像素 = {:?}", a.pixel(119, 79));
        assert_eq!(a.pixel(119, 79), Some([1, 2, 3, 255]));
        h.set_clear(Color::rgb(9, 8, 7));
        let b = h.shoot_named("clear=9,8,7").unwrap();
        assert_eq!(b.pixel(119, 79), Some([9, 8, 7, 255]));

        // 清屏色只决定「没被画到」的地方：根节点**左上角**（描边落点）两帧必须逐字节相同，
        // 而且它不是清屏色。
        let r = h.frame().unwrap().rect_of("root").unwrap();
        println!("root 矩形 = {r:?}；左上角 = {:?}", a.pixel(r.x, r.y));
        assert_eq!(
            a.pixel(r.x, r.y),
            b.pixel(r.x, r.y),
            "被画到的像素不该随清屏色变"
        );
        assert_ne!(
            a.pixel(r.x, r.y),
            Some([1, 2, 3, 255]),
            "根节点左上角必须真的被画了（否则这条测试什么也没证明）"
        );
    }

    /// PNG 出图：签名、IHDR 尺寸、落盘读回逐字节相同。
    #[test]
    fn shot_writes_a_real_png() {
        let mut h = harness(0);
        if skip_without_glyphs(&h) {
            return;
        }
        let s = h.shoot().unwrap();
        let png = s.to_png().unwrap();
        assert_eq!(
            &png[0..8],
            &[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a],
            "PNG 签名不对"
        );
        let iw = u32::from_be_bytes([png[16], png[17], png[18], png[19]]);
        let ih = u32::from_be_bytes([png[20], png[21], png[22], png[23]]);
        assert_eq!((iw, ih), (W, H), "IHDR 里的尺寸必须是这一帧的尺寸");
        let path = std::env::temp_dir().join("deer_testkit_shot.png");
        s.write_png(&path).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), png);
        println!("PNG {} 字节 ⇒ {}", png.len(), path.display());
        let _ = std::fs::remove_file(&path);
    }

    // -- 容差档与 CPU↔GPU 对照 ---------------------------------------------

    #[test]
    fn parity_rule_is_read_from_the_corpus() {
        let mut opaque = DrawList::new();
        opaque.push(DrawCmd::FillRect {
            rect: RectI::new(0, 0, 4, 4),
            color: Color::WHITE,
        });
        assert_eq!(ParityRule::for_list(&opaque), ParityRule::Opaque);
        opaque.push(DrawCmd::FillRect {
            rect: RectI::new(0, 0, 2, 2),
            color: Color::rgba(255, 0, 0, 0.5),
        });
        assert_eq!(
            ParityRule::for_list(&opaque),
            ParityRule::Translucent,
            "只要有一条半透明命令，整份语料就按 ≤1 LSB 判"
        );
        assert_eq!(ParityRule::for_theme(&opaque_theme()), ParityRule::Opaque);
        assert_eq!(
            ParityRule::for_theme(&Theme::default()),
            ParityRule::Translucent,
            "默认主题的 surface 是半透明（a=0.55）"
        );
        assert_eq!(ParityRule::Opaque.max_allowed(), 0);
        assert_eq!(ParityRule::Translucent.max_allowed(), 1);
    }

    /// 判据本身（纯函数）：两档的边界必须咬住。
    #[test]
    fn parity_report_check_goes_red_for_each_rule() {
        let ext = Extent {
            width: 1,
            height: 1,
        };
        let ok = parity_report("same", ParityRule::Opaque, ext, &[1, 2, 3, 4], &[1, 2, 3, 4], 0)
            .unwrap();
        ok.check().unwrap();
        assert_eq!(ok.max_channel_diff, 0);
        assert_eq!(ok.differing_pixels, 0);
        assert_eq!(ok.total_pixels, 1);
        assert_eq!(ok.gpu, [1, 2, 3, 4]);

        // 不透明语料差 1 ⇒ 红（哪怕 ≤1 LSB 也不许）。
        let one = parity_report("off-by-one", ParityRule::Opaque, ext, &[1, 2, 3, 5], &[1, 2, 3, 4], 0)
            .unwrap();
        expect_err_raw(
            "ParityReport::check（不透明语料差 1）",
            "逐字节相同",
            one.check(),
        );
        // 半透明语料差 1 ⇒ 绿（CPU round vs GPU UNORM）。
        let half = parity_report(
            "lsb-ok",
            ParityRule::Translucent,
            ext,
            &[1, 2, 3, 5],
            &[1, 2, 3, 4],
            0,
        )
        .unwrap();
        half.check().unwrap();
        assert_eq!(half.max_channel_diff, 1);
        assert_eq!(half.worst_channel, 3);
        assert_eq!(half.worst_pixel, (0, 0));
        // 半透明语料差 2 ⇒ 红。
        let two = parity_report(
            "lsb-exceeded",
            ParityRule::Translucent,
            ext,
            &[1, 2, 3, 6],
            &[1, 2, 3, 4],
            0,
        )
        .unwrap();
        expect_err_raw(
            "ParityReport::check（半透明语料差 2）",
            "> 允许的 1",
            two.check(),
        );
        // 文本被跳过 ⇒ 红（「逐字节相同」可能只是两边都没画）。
        let skipped = parity_report(
            "text-skipped",
            ParityRule::Opaque,
            ext,
            &[1, 2, 3, 4],
            &[1, 2, 3, 4],
            3,
        )
        .unwrap();
        expect_err_raw(
            "ParityReport::check（有文本被跳过）",
            "跳过 3 条文本命令",
            skipped.check(),
        );
        expect_err_raw(
            "parity_report（长度不一致）",
            "≠ CPU 帧缓冲",
            parity_report("bad", ParityRule::Opaque, ext, &[0; 4], &[0; 8], 0).map(|_| ()),
        );
        expect_err_raw(
            "parity_report（长度与 extent 不符）",
            "应有的",
            parity_report("bad2", ParityRule::Opaque, ext, &[0; 8], &[0; 8], 0).map(|_| ()),
        );
    }

    /// **testkit 的对照结论必须与手写版逐字节相同。**
    ///
    /// 手写版就是 `crates/deer-vk/tests/gpu_vs_cpu.rs::compare` 的口径：
    /// 同一个循环数最大通道差 / 不同像素 / 最差点。这里把它照抄一遍当**独立对照**，
    /// 而不是拿 testkit 自己跟自己对。
    #[test]
    fn cpu_gpu_parity_matches_the_hand_written_loop_byte_for_byte() {
        let h = harness(2);
        let f = h.frame().unwrap();
        let rule = ParityRule::for_list(&f.list);
        println!(
            "语料：{} 条命令，容差档 {:?}；有半透明命令 ⇒ 按 ≤1 LSB 判",
            f.list.len(),
            rule
        );
        let Some(mut probe) = GpuProbe::new(h.extent(), h.clear(), h.font()).expect("建探针")
        else {
            return;
        };
        let report = probe.compare("counter-帧", &f.list, rule).expect("对照必须过");
        println!("{}", report.line());

        // —— 独立手写版（不复用 testkit 的比较逻辑）——
        let gpu = probe.render(&f.list).unwrap();
        let cpu = probe.cpu_render(&f.list).unwrap();
        let w = h.extent().width.max(1);
        let mut m_max = 0u8;
        let mut m_bytes = 0usize;
        let mut m_at = 0usize;
        for (i, (g, c)) in gpu.iter().zip(cpu.iter()).enumerate() {
            let d = g.abs_diff(*c);
            if d > 0 {
                m_bytes += 1;
            }
            if d > m_max {
                m_max = d;
                m_at = i;
            }
        }
        let mut m_px = 0usize;
        for (g, c) in gpu.chunks_exact(4).zip(cpu.chunks_exact(4)) {
            if g != c {
                m_px += 1;
            }
        }
        let m_px_index = (m_at / 4) as u32;
        println!(
            "手写版：最大通道差 {m_max}，不同字节 {m_bytes}，不同像素 {m_px}，最差像素 ({}, {})，\
             GPU={:?} CPU={:?}",
            m_px_index % w,
            m_px_index / w,
            &gpu[m_at / 4 * 4..m_at / 4 * 4 + 4],
            &cpu[m_at / 4 * 4..m_at / 4 * 4 + 4]
        );
        assert_eq!(report.max_channel_diff, m_max, "最大通道差必须逐数字相同");
        assert_eq!(report.differing_bytes, m_bytes, "不同字节数必须逐数字相同");
        assert_eq!(report.differing_pixels, m_px, "不同像素数必须逐数字相同");
        assert_eq!(report.worst_channel, m_at % 4, "最差通道必须相同");
        assert_eq!(
            report.worst_pixel,
            (m_px_index % w, m_px_index / w),
            "最差点必须相同"
        );
        assert_eq!(
            report.gpu,
            [
                gpu[m_at / 4 * 4],
                gpu[m_at / 4 * 4 + 1],
                gpu[m_at / 4 * 4 + 2],
                gpu[m_at / 4 * 4 + 3]
            ],
            "最差点的 GPU 四通道必须逐字节相同"
        );
        assert_eq!(
            report.cpu,
            [
                cpu[m_at / 4 * 4],
                cpu[m_at / 4 * 4 + 1],
                cpu[m_at / 4 * 4 + 2],
                cpu[m_at / 4 * 4 + 3]
            ],
            "最差点的 CPU 四通道必须逐字节相同"
        );
        assert_eq!(report.text_skipped, 0, "这一帧不该有被跳过的文本");
        assert_eq!(report.total_pixels, (W as usize) * (H as usize));

        // 只读版本与完整版本必须给出同一份报告。
        let again = probe
            .report_from("counter-帧", rule, &gpu, &cpu)
            .expect("同一批字节 ⇒ 同一份报告");
        assert_eq!(again, report, "同一批字节两次算出来的报告必须逐字段相同");

        // 通过 `Harness` 的便捷入口拿到的也必须是同一份结论。
        let via_harness = h.compare_cpu_gpu("counter-帧", &f.list, rule).unwrap();
        if let Some(v) = via_harness {
            assert_eq!(v, report, "Harness 的便捷入口必须与显式探针给出同一份结论");
        }
    }

    // -- 门槛、自证、复现命令 -----------------------------------------------

    /// 门槛读取的**自证**：这条测试在任何环境下都必须「说的和事实一致」。
    ///
    /// 验收时跑两遍（一遍不带变量、一遍带 `set "DEER_TESTKIT_GATE_PROBE=1 " &&`），
    /// 两遍的输出就是「尾空格也被认成开」这条判据的证据（`env_gate` 的实测理由）。
    #[test]
    fn gate_self_evidence_matches_the_real_environment() {
        let line = print_gates(&["DEER_TESTKIT_GATE_PROBE", "DEER_VK_WINDOW_TESTS"]);
        assert!(line.starts_with(GATE_MARKER));
        let raw = std::env::var("DEER_TESTKIT_GATE_PROBE").ok();
        let want = crate::env_gate::truthy(raw.as_deref().unwrap_or(""));
        assert_eq!(
            gate("DEER_TESTKIT_GATE_PROBE"),
            want,
            "门槛的解析必须与 `env_gate::truthy` 逐字一致（原始值 {raw:?}）"
        );
        // 未设的门槛：`require_gate` 必须返回 false 并打出**跳过**标记。
        assert!(
            !require_gate("DEER_TESTKIT_DEFINITELY_NOT_SET_XYZ"),
            "未设的门槛必须返回 false（跳过）"
        );
        assert!(gate_prefix(&["A", "B"]).contains(r#"set "A=1" && set "B=1" && "#));
    }

    /// 帧数门槛（纯函数）：尾空格、空串、非法值、0 各一条。
    #[test]
    fn window_frames_are_trim_tolerant_and_never_silently_default() {
        use super::window::{frames_from_value, FRAMES_ENV};
        assert_eq!(frames_from_value(Some("3 "), 9).unwrap(), 3, "`set X=3 &&` 的值是 `\"3 \"`");
        assert_eq!(frames_from_value(Some(" 4 "), 9).unwrap(), 4);
        assert_eq!(frames_from_value(None, 9).unwrap(), 9);
        assert_eq!(frames_from_value(Some("   "), 9).unwrap(), 9);
        assert!(frames_from_value(Some("0"), 9).unwrap_err().contains(FRAMES_ENV));
        expect_err_raw(
            "window::frames_from_value（非法值不许静默退回默认）",
            "不是帧数",
            frames_from_value(Some("abc"), 9).map(|_| ()),
        );
        // 环境未设时的真实路径也必须能用。
        assert!(super::window::frames_from_env(3).is_ok());
    }

    /// 复现命令的**形状**：`set "VAR=1" &&` 前缀 + 包/feature/过滤 + `--nocapture`。
    #[test]
    fn repro_commands_are_copy_pasteable() {
        let t = Repro::test("deer-gui", "testing", "testkit_counter", "tap_plus", &["DEER_VK_WINDOW_TESTS"]);
        println!("{}", t.cmd());
        assert_eq!(
            t.cmd(),
            r#"set "DEER_VK_WINDOW_TESTS=1" && cargo test -p deer-gui --features testing --test testkit_counter tap_plus -- --nocapture"#
        );
        let e = Repro::example("deer-gui", "window,testing", "counter", "-- --headless", &[]);
        assert_eq!(
            e.cmd(),
            "cargo run -q -p deer-gui --features window,testing --example counter -- --headless"
        );
        // 没设 repro 时的兜底命令**仍然可复制**（不是一句「（无）」）。
        assert!(default_repro().cmd().starts_with("cargo test -p deer-gui"));
        assert!(default_repro().cmd().ends_with("-- --nocapture"));
    }

    /// 每一次失败都带复现命令 —— 用一条**真的会红**的断言证明它是结构性的。
    #[test]
    fn every_failure_carries_the_repro_command() {
        let mut h = harness(0);
        h.set_case("demo/计数显示");
        h.set_repro(Repro::test("deer-gui", "testing", "testkit_counter", "demo", &[]));
        let f = h.frame().unwrap();
        let e = h.assert_drawn_text(&f, "count", "错的文本").unwrap_err();
        println!("{e}");
        assert!(e.starts_with("[demo/计数显示] "), "失败信息要带用例名：{e}");
        assert!(
            e.contains("cargo test -p deer-gui --features testing --test testkit_counter demo -- --nocapture"),
            "失败信息要带可复制的复现命令：{e}"
        );
        assert!(e.contains("count = 0"), "失败信息要带**真实数据**：{e}");
    }

    // -- 降级档 -------------------------------------------------------------

    /// `without_font` ⇒ 布局用近似度量、像素用占位方块；**文本像素断言必须明确报错**。
    #[test]
    fn the_degraded_harness_refuses_text_pixel_assertions() {
        let h = Harness::without_font(counter_tree(0), W, H, opaque_theme());
        println!("降级档字体说明：{}", h.font_note());
        assert!(!h.has_glyph_pixels());
        assert!(h.font().is_none());
        expect_err_carried(
            "Harness::require_glyph_pixels（降级档）",
            "假红",
            h.require_glyph_pixels(),
        );
        // 反过来：有字形的档必须过。
        let good = harness(0);
        if good.has_glyph_pixels() {
            good.require_glyph_pixels().unwrap();
        }
    }

    /// 视觉状态变化集合的计算（[`visually_changed_ids`]）必须**精确**。
    #[test]
    fn visually_changed_ids_is_exact() {
        let ids: Vec<String> = all_ids(&counter_tree(0));
        let a = UiState::default();
        let b = UiState {
            hover: Some("plus".into()),
            ..UiState::default()
        };
        assert_eq!(visually_changed_ids(&a, &b, &ids), vec!["plus".to_string()]);
        assert!(
            visually_changed_ids(&a, &a, &ids).is_empty(),
            "没有任何变化 ⇒ 空集合"
        );
        let b2 = UiState {
            hover: Some("plus".into()),
            focus: Some("input".into()),
            ..UiState::default()
        };
        assert_eq!(
            visually_changed_ids(&a, &b2, &ids),
            vec!["plus".to_string(), "input".to_string()]
        );
        // `texts` **不**参与：文本变化是应用数据，由调用方指名 primary。
        let c = UiState {
            texts: [("input".to_string(), "ok".to_string())]
                .into_iter()
                .collect(),
            ..UiState::default()
        };
        assert!(
            visually_changed_ids(&a, &c, &ids).is_empty(),
            "texts 变化不是「视觉状态」变化（渲染器不看它决定高亮）"
        );
    }

    // -- **反向自检的汇总表** ----------------------------------------------

    /// **每一个**断言助手都必须有一条「它会红」的反向自检。
    ///
    /// 这张表同时挡住两个方向的假护栏：
    /// ① 助手恒返回 `Ok`（这里必然红）；
    /// ② 助手恒返回 `Err`（`greens` 表那条会红）。
    #[test]
    fn every_assertion_helper_can_go_red_and_each_green_case_passes() {
        let mut h = harness(0);
        let f = h.frame().unwrap();
        let font_size = h.theme().font_size;

        // —— 红：带复现命令的那一类（Harness 的助手 + Shot 的助手）——
        let mut reds: Vec<(&str, Result<(), String>)> = Vec::new();
        if !skip_without_glyphs(&h) {
            let a = h.shoot().unwrap();
            let mut tampered = a.clone();
            tampered.rgba[0] = tampered.rgba[0].wrapping_add(3); // 像素 (0,0) 的 R
            tampered.label = "被人为改了一像素的一帧".to_string();
            reds.push(("Shot::assert_bytes_eq", tampered.assert_bytes_eq(&a)));
            reds.push((
                "Shot::assert_no_diff_outside",
                tampered.assert_no_diff_outside(&a, &[RectI::new(30, 30, 4, 4)]),
            ));
            reds.push((
                "Shot::assert_diff_only_inside",
                tampered.assert_diff_only_inside(&a, "count"),
            ));
            reds.push((
                "Shot::assert_state_change_only",
                a.assert_state_change_only(&a, "count"),
            ));
            reds.push((
                "Shot::assert_state_change_only（primary 无几何）",
                a.assert_state_change_only(&a, "nope"),
            ));
        }
        reds.push(("Harness::assert_hover", h.assert_hover(Some("minus"))));
        reds.push(("Harness::assert_focus", h.assert_focus(Some("minus"))));
        reds.push(("Harness::assert_pressed", h.assert_pressed(Some("plus"))));
        reds.push((
            "Harness::assert_visual_state",
            h.assert_visual_state(Some("minus"), None, None),
        ));
        reds.push(("Harness::assert_text", h.assert_text("input", "zzz")));
        reds.push(("Harness::assert_texts", h.assert_texts(&[("input", "zzz")])));
        let mut wrong_state = h.state().clone();
        wrong_state.hover = Some("zzz".into());
        reds.push(("Harness::assert_state", h.assert_state(&wrong_state)));
        reds.push((
            "Harness::assert_focus_order",
            h.assert_focus_order(&["minus", "plus"]),
        ));
        reds.push((
            "Harness::assert_focus_order_excludes",
            h.assert_focus_order_excludes(&["plus"]),
        ));
        reds.push(("Harness::assert_command_count", h.assert_command_count(&f, 0)));
        reds.push((
            "Harness::assert_node_hint_count",
            h.assert_node_hint_count(&f, 0),
        ));
        reds.push(("Harness::assert_clip_nodes", h.assert_clip_nodes(&f, 0)));
        reds.push((
            "Harness::assert_clip_known",
            h.assert_clip_known(&f, &["nope"]),
        ));
        reds.push((
            "Harness::assert_counts",
            h.assert_counts(&f, DrawCounts::default()),
        ));
        reds.push((
            "Harness::assert_drawn_text（文本不对）",
            h.assert_drawn_text(&f, "count", "错的"),
        ));
        reds.push((
            "Harness::assert_drawn_text（该节点没有文本命令）",
            h.assert_drawn_text(&f, "app", "x"),
        ));
        reds.push((
            "Harness::assert_drawn_text_contains",
            h.assert_drawn_text_contains(&f, "count", "zzz"),
        ));
        reds.push((
            "Harness::assert_text_size",
            h.assert_text_size(&f, "count", font_size + 9.0),
        ));
        reds.push((
            "Harness::assert_text_size（该节点没有文本命令 ⇒ 命令数不为 1）",
            h.assert_text_size(&f, "app", font_size),
        ));

        // —— 红：另一类输入（各自用新面，避免把必查 id 污染到上面那些用例）——
        let mut hb = harness(0);
        reds.push(("Harness::run_script（语法错）", hb.run_script("key:bogus").map(|_| ())));
        let mut hc = harness(0);
        reds.push((
            "Harness::run_script（@id 不存在）",
            hc.run_script("move @nope").map(|_| ()),
        ));
        let mut hd = harness(0);
        reds.push(("Harness::tap（id 不存在）", hd.tap("nope").map(|_| ())));        let mut he = harness(0);
        he.require_ids(&["nope"]);
        reds.push(("Harness::frame（必查 id 没有几何）", he.frame().map(|_| ())));
        reds.push((
            "Harness::require_glyph_pixels（降级档）",
            Harness::without_font(counter_tree(0), W, H, opaque_theme()).require_glyph_pixels(),
        ));

        let n_reds = reds.len();
        for (name, r) in reds {
            expect_err_carried(name, "", r);
        }

        // —— 红：纯函数判据（没有 Harness 的「复现命令」包装，单独一张表）——
        // 一次建好（`vec![]`）而不是 `Vec::new()` 后连推：本项目 `#![deny(clippy::all)]`，
        // clippy 的 `vec_init_then_push` 会直接报错。
        let raw: Vec<(&str, Result<(), String>)> = vec![
            (
                "pixel_diff_split（长度不一致）",
                pixel_diff_split(&[0u8; 4], &[0u8; 8], 1, &[]).map(|_| ()),
            ),
            (
                "pixel_diff_split（长度不是 4 的倍数）",
                pixel_diff_split(&[0u8; 6], &[0u8; 6], 1, &[]).map(|_| ()),
            ),
            (
                "assert_no_diff_outside",
                super::assert_no_diff_outside(&[0, 0, 0, 0], &[1, 0, 0, 0], 1, &[], "raw")
                    .map(|_| ()),
            ),
            (
                "parity_report（长度不一致）",
                parity_report(
                    "x",
                    ParityRule::Opaque,
                    Extent {
                        width: 1,
                        height: 1,
                    },
                    &[0; 4],
                    &[0; 8],
                    0,
                )
                .map(|_| ()),
            ),
            (
                "ParityReport::check（不透明语料差 1）",
                parity_report(
                    "x",
                    ParityRule::Opaque,
                    Extent {
                        width: 1,
                        height: 1,
                    },
                    &[1, 0, 0, 0],
                    &[0, 0, 0, 0],
                    0,
                )
                .unwrap()
                .check(),
            ),
            (
                "ParityReport::check（半透明语料差 2）",
                parity_report(
                    "x",
                    ParityRule::Translucent,
                    Extent {
                        width: 1,
                        height: 1,
                    },
                    &[2, 0, 0, 0],
                    &[0, 0, 0, 0],
                    0,
                )
                .unwrap()
                .check(),
            ),
            (
                "ParityReport::check（有文本被跳过）",
                parity_report(
                    "x",
                    ParityRule::Opaque,
                    Extent {
                        width: 1,
                        height: 1,
                    },
                    &[0; 4],
                    &[0; 4],
                    1,
                )
                .unwrap()
                .check(),
            ),
            (
                "window::frames_from_value（非法值）",
                super::window::frames_from_value(Some("zzz"), 3).map(|_| ()),
            ),
        ];
        let n_raw = raw.len();
        for (name, r) in raw {
            expect_err_raw(name, "", r);
        }

        // —— 绿：同一批助手在**正确期望**下必须过（挡住「恒返回 Err」的假护栏）——
        let greens: Vec<(&str, Result<(), String>)> = vec![
            ("assert_hover", h.assert_hover(None)),
            ("assert_focus", h.assert_focus(None)),
            ("assert_pressed", h.assert_pressed(None)),
            ("assert_visual_state", h.assert_visual_state(None, None, None)),
            ("assert_text", h.assert_text("input", "")),
            ("assert_texts", h.assert_texts(&[("input", "")])),
            ("assert_state", h.assert_state(&UiState::default())),
            (
                "assert_focus_order",
                h.assert_focus_order(&["plus", "minus", "input"]),
            ),
            (
                "assert_focus_order_excludes",
                h.assert_focus_order_excludes(&["title", "count"]),
            ),
            ("assert_command_count", h.assert_command_count(&f, 19)),
            ("assert_node_hint_count", h.assert_node_hint_count(&f, 8)),
            ("assert_clip_nodes", h.assert_clip_nodes(&f, 8)),
            (
                "assert_clip_known",
                h.assert_clip_known(&f, &["plus", "input", "count"]),
            ),
            (
                "assert_counts",
                h.assert_counts(
                    &f,
                    DrawCounts {
                        fill_rect: 0,
                        stroke_rect: 2,
                        fill_round_rect: 4,
                        text: 5,
                        push_clip: 0,
                        pop_clip: 0,
                        node_hint: 8,
                    },
                ),
            ),
            (
                "assert_drawn_text",
                h.assert_drawn_text(&f, "count", "count = 0"),
            ),
            (
                "assert_drawn_text_contains",
                h.assert_drawn_text_contains(&f, "count", "count"),
            ),
            (
                "assert_text_size",
                h.assert_text_size(&f, "count", font_size),
            ),
            (
                "check_preconditions",
                check_preconditions(&f.list, &f.clip, &f.geo, &["plus"]),
            ),
            (
                "pixel_diff_split（无差异）",
                pixel_diff_split(&[0u8; 8], &[0u8; 8], 2, &[]).map(|_| ()),
            ),
            (
                "assert_no_diff_outside（无差异）",
                pixel_diff_split(&[0u8; 8], &[0u8; 8], 2, &[]).map(|_| ()),
            ),
            (
                "window::frames_from_value（尾空格）",
                super::window::frames_from_value(Some("3 "), 9).map(|_| ()),
            ),
        ];
        let n_greens = greens.len();
        for (name, r) in greens {
            if let Err(e) = r {
                panic!("`{name}` 在**正确期望**下必须过，实际红了：{e}");
            }
        }

        println!(
            "反向自检汇总：{n_reds} 个断言助手「会红」✅ + {n_raw} 个纯函数判据「会红」✅ \
             + {n_greens} 个助手在正确期望下「会绿」✅"
        );
        // 棘轮：覆盖面不许悄悄缩水（加助手时这两条下限要跟着往上走）。
        assert!(n_reds >= 24, "带复现命令的反向自检条数不许少于 24，实际 {n_reds}");
        assert!(n_raw >= 8, "纯函数判据的反向自检条数不许少于 8，实际 {n_raw}");
        assert!(n_greens >= 20, "正面用例条数不许少于 20，实际 {n_greens}");
    }
}

