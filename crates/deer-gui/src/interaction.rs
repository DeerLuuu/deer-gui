//! M5-2 / M5-3：交互核心（**纯逻辑**）—— 命中测试、裁剪快照、状态机。
//!
//! 这一层**不碰窗口、不碰 GPU**：输入是值、输出是值。所以整条交互链可以在
//! 没有 winit、没有 Vulkan 的环境里被单测覆盖（`cargo test -p deer-gui --lib interaction`）。
//!
//! # 先读再定：下面这些是**从代码里读出来的事实**，不是记忆里的印象
//!
//! | 事实 | 出处 |
//! |---|---|
//! | `hit_test` **最深命中者胜出**（后序覆盖）、**半开区间** `px ∈ [x, x+w)`、注释写明它是「输入路由的唯一依据」 | `crates/deer-layout/src/layout.rs:350-367` |
//! | `DrawCmd::NodeHint { rect, node_id_len, node_id_fp }` **不含 id 本身**；**全仓库没有 id 侧表** —— 两个校验和由**唯一**构造点 `DrawCmd::node_hint`（`draw.rs`）产出，全部后端消费点（`null.rs:352`、`gpu_geom.rs:193`、`gpu_render.rs:1444`、`windowed.rs:1363`、`hal.rs:170`）都**忽略**它（它不产生像素） | `crates/deer-gpu/src/draw.rs`（`NodeHint` 与 `node_id_fp`） |
//! | 裁剪语义：`PushClip` 与当前裁剪**求交**、`PopClip` 出栈、**空栈 `PopClip` ⇒ 退回全画布** | `crates/deer-gpu/src/null.rs:313-329`（GPU 侧同语义 `deer-vk/src/gpu_text.rs:126-130`） |
//! | `NodeProps::disabled` 已存在 | `crates/deer-layout/src/node.rs:119-122` |
//! | `InputEvent`/`Key`/`Mods`/`PointerButton` 已在 `deer-window` 落地（M5-1），与本层的镜像逐字相同 | `crates/deer-window/src/lib.rs:127-210` |
//!
//! ## 由事实推出的三个设计结论
//!
//! 1. **命中必须复用 `hit_test`**（`hit()` 只做两件它不做的事：查裁剪、查禁用），
//!    不另写一套遍历 —— 否则「谁是输入路由的唯一依据」就有两份，必然漂。
//! 2. **`ClipSnapshot` 无法只从 `DrawList` 派生**：`NodeHint` 不带 id 本身（只有它的
//!    长度与指纹），也没有侧表。所以构造时要**与树 + 几何共走**，把第 k 个 `NodeHint`
//!    绑到第 k 个「有几何的节点」上，并用 `node_id_len` **和** `node_id_fp`（id 的
//!    确定性指纹）当校验和（不一致就断言失败，而不是悄悄错位）。
//!    计划里「id 在侧表」的前提经核实**不成立**，这里按实际情况实现。
//!    ⚠️ 只比长度是**不够**的：等长 id 互换会让长度校验和逐项相同 ⇒ 静默错位
//!    （改前实测：`aaaa`/`bbbb` 互换后不 panic，且裁剪张冠李戴）—— 指纹就是为了堵这个盲区
//!    （回归见 `r19_equal_length_id_swap_must_be_caught_by_the_id_fingerprint`）。
//! 3. **命中落在禁用子树或被裁掉的点上 ⇒ 这个点没有命中**（**不回退**到祖先）。
//!    回退需要「第二套路由规则」（谁是次优候选），与结论 1 冲突；代价见模块末尾「已知边界」。

use std::collections::BTreeMap;

use deer_gpu::{DrawCmd, DrawList, RectI};
use deer_layout::layout::{Geometry, ScrollMetrics, ScrollOffsets};
use deer_layout::node::{Kind, Node};

// ---------------------------------------------------------------------------
// 一、输入事件模型 —— M5-1 冻结定义的**本地镜像**
// ---------------------------------------------------------------------------

// 开了 `window` feature ⇒ 直接用 `deer-window` 里那一份（M5-1 的冻结定义，**不复制模型**；
// 事件位是物理像素、窗口左上角原点，与 `WindowInfo::extent` 同一套口径）。
#[cfg(feature = "window")]
pub use deer_window::{InputEvent, Key, Mods, PointerButton};

// 没开 `window` feature ⇒ 用 M5-1 冻结定义的**原样镜像**（默认 feature 路径就是这条）。
//
// 为什么必须有镜像：交互层是纯逻辑，要在**不拉进 `winit`** 的情况下跑单测
// （`cargo test -p deer-gui --lib interaction`）。镜像与 `crates/deer-window/src/lib.rs`
// 的定义**逐字相同**（变体名、字段名、字段类型、`Key` 的变体顺序都核对过），
// 所以两条路径下 `hit`/`handle` 的类型与语义完全一致，单测也是同一套。
#[cfg(not(feature = "window"))]
pub use mirror::{InputEvent, Key, Mods, PointerButton};

#[cfg(not(feature = "window"))]
mod mirror {
    /// 指针按键。
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub enum PointerButton {
        Left,
        Right,
        Middle,
    }

    /// 修饰键状态（与 winit 的 `ModifiersState` 字段一一对应）。
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
    pub struct Mods {
        pub shift: bool,
        pub ctrl: bool,
        pub alt: bool,
        pub sup: bool,
    }

    /// **物理键**：不含文本语义 —— 文本一律走 [`InputEvent::TextInput`]。
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub enum Key {
        Tab,
        Escape,
        Enter,
        Backspace,
        Left,
        Right,
        Up,
        Down,
        /// 可打印字符键（逻辑字符，`Shift+a` ⇒ `Char('A')`）。
        Char(char),
        /// 其它一切（修饰键、功能键、死键、无法识别的键）。
        Other,
    }

    /// 输入事件（可脚本化重放：全部是值，没有句柄、没有闭包）。
    #[derive(Debug, Clone, PartialEq)]
    pub enum InputEvent {
        PointerMoved {
            x: f32,
            y: f32,
        },
        PointerDown {
            button: PointerButton,
            x: f32,
            y: f32,
        },
        PointerUp {
            button: PointerButton,
            x: f32,
            y: f32,
        },
        Wheel {
            dx: f32,
            dy: f32,
        },
        KeyDown {
            key: Key,
            mods: Mods,
        },
        KeyUp {
            key: Key,
            mods: Mods,
        },
        TextInput {
            text: String,
        },
        /// 窗口焦点（**不是** UI 里的控件焦点）。
        FocusChanged {
            focused: bool,
        },
    }
}

// 上面这 4 个类型在**两条 feature 路径**下是同一个名字：开 `window` 时是 `deer-window`
// 的真实类型（M5-1），关 `window` 时是逐字相同的镜像。所以 `hit`/`handle`/`UiState`/
// `UiEvent` 的签名与语义只有一份，M5-4 的窗口层把 `deer_window::InputEvent` 直接喂进来即可
// （不需要任何翻译层）。

// ---------------------------------------------------------------------------
// 二、状态与「发生了什么」
// ---------------------------------------------------------------------------

/// 交互状态：**唯一真相**（窗口层渲染的树由它派生）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UiState {
    /// 指针当前压在哪个节点上（含容器；`None` = 没压在任何节点上）。
    pub hover: Option<String>,
    /// 键盘焦点（`Tab`/`Shift+Tab` 改它，`Escape` 清它）。
    pub focus: Option<String>,
    /// 被**左键**按下的节点（按下时记住，抬起时判定是否成 Clicked）。
    pub pressed: Option<String>,
    /// 输入框的文本缓冲（id → 内容）。不在里面的输入框视为空串。
    pub texts: BTreeMap<String, String>,
    /// **滚动状态**（偏移 + 每个容器的上限）。偏移是**布局的输入**（调用方每帧喂给
    /// `layout_with_scroll`），上限是布局的输出（调用方每帧 `set_metrics` 灌回来）。
    ///
    /// 为什么偏移住在 `UiState` 里：滚轮必须在**唯一入口**（[`handle`]）被消费 ——
    /// 若另开一个「带滚动的 handle」，那条路径上的滚轮就会静默无效，而没人能一眼看出来。
    pub scroll: ScrollState,
}

/// 滚动状态：**偏移**（布局的输入）+ **上限**（布局的输出）。
///
/// 两者分开存是刻意的：偏移是「状态」，上限是「这一帧的几何事实」。调用方每帧
/// 把布局结果灌进来（[`ScrollState::set_metrics`]），`handle` 用上限把偏移夹在
/// `[0, max_scroll]` 内 —— 于是「滚到边界不越界」只有一处实现（不依赖调用方自觉）。
///
/// **灌漏了会怎样**：上限表为空 ⇒ 每个容器的 `max_of` 都是 0 ⇒ 滚轮什么都不做
/// （fail-closed：不会滚进一个「没有上限」的虚空，也不会产生假事件）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScrollState {
    /// 当前偏移（整数像素）。
    pub offsets: ScrollOffsets,
    /// 本帧的滚动上限（来自 [`deer_layout::layout::layout_with_scroll`]）。
    pub metrics: ScrollMetrics,
}

impl ScrollState {
    pub fn new() -> ScrollState {
        ScrollState::default()
    }

    /// 某个容器的当前偏移（没滚过 ⇒ 0）。
    pub fn offset_of(&self, id: &str) -> i32 {
        self.offsets.get(id)
    }

    /// 某个容器的滚动上限（没灌 / 不是滚动容器 ⇒ 0）。
    pub fn max_of(&self, id: &str) -> i32 {
        self.metrics.max_of(id)
    }

    /// 灌入本帧的布局结果，并把**已有的偏移夹回新的上限**。
    ///
    /// 为什么必须夹：视口/内容一变（窗口缩放、内容增减），旧偏移可能已经越界；
    /// 不夹就会留下一个「状态里存着、几何里却用不到」的偏移 —— 下一次内容变高时
    /// 它又会**突然生效**（用户看到界面像被谁滚了一下）。
    pub fn set_metrics(&mut self, metrics: &ScrollMetrics) {
        self.metrics = metrics.clone();
        let ids: Vec<String> = self.offsets.ids().map(str::to_string).collect();
        for id in ids {
            let clamped = self.metrics.clamp(&id, self.offsets.get(&id));
            self.offsets.set(id, clamped);
        }
    }

    /// 把某个容器的偏移设成 `v`（夹取）；**真的变了**才返回新值（否则 `None` ⇒ 不发事件）。
    pub fn scroll_to(&mut self, id: &str, v: i32) -> Option<i32> {
        let next = self.metrics.clamp(id, v);
        if next == self.offsets.get(id) {
            return None;
        }
        self.offsets.set(id, next);
        Some(next)
    }

    /// 相对滚动：`delta_px > 0` = 内容上移（偏移增大）。
    pub fn scroll_by(&mut self, id: &str, delta_px: i32) -> Option<i32> {
        let cur = self.offsets.get(id);
        self.scroll_to(id, cur.saturating_add(delta_px))
    }
}

/// 一次 `handle` 产生的「发生了什么」。窗口层据此置 dirty 并重绘。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UiEvent {
    HoverChanged(Option<String>),
    FocusChanged(Option<String>),
    Clicked(String),
    TextChanged { id: String, value: String },
    /// 某个可滚动容器的偏移变了（滚轮驱动）。`offset` 是**夹取之后**的整数像素值。
    ///
    /// 只有**真的变了**才发（到顶/到底再滚、或 `max_scroll == 0` 都是「没变」⇒ 不发），
    /// 于是窗口层的 dirty 约定照旧：有事件 = 需要重绘。
    Scrolled { id: String, offset: i32 },
}

/// 滚轮的**每「一格」对应的像素数**（`InputEvent::Wheel::dy` 的单位由窗口层决定：
/// winit 的 `LineDelta` 是**行**、`PixelDelta` 是像素，`map_wheel` 原样透传 ⇒ 这里按「行」解释）。
///
/// 为什么是常量而不是「主题行高 × dy」：`handle` 拿不到主题，而从别处把主题传进来会让
/// 「同一份输入滚多远」随主题变化 ⇒ 同一份脚本的判据不再确定。40 px ≈ 两行文本
/// （`Theme::line_height` = 18）。
///
/// **符号约定**：`dy < 0`（滚轮向下拨）⇒ 内容上移 ⇒ 偏移**增大**。
pub const WHEEL_STEP_PX: i32 = 40;

impl UiState {
    /// 三个**视觉**字段（`hover`/`focus`/`pressed`）是否完全一样（`texts` 不参与）。
    ///
    /// 窗口层的 dirty 约定靠它：`PointerDown` 这类**不发 `UiEvent`** 却会改 `pressed`
    /// 的事件，只有比对状态才能判定「这一帧到底要不要重绘」。判据若写成「有没有事件」，
    /// `pressed` 那条路就会被漏掉（而且漏得很安静 —— 界面看起来只是不响应按下）。
    pub fn same_visual(&self, other: &UiState) -> bool {
        self.hover == other.hover && self.focus == other.focus && self.pressed == other.pressed
    }
}

// ---------------------------------------------------------------------------
// 三、裁剪快照（M5-3）
// ---------------------------------------------------------------------------

/// 每个节点的**有效裁剪**（已把嵌套 `PushClip` 求交完），供命中测试用。
///
/// - 值 `None` = 该节点不裁剪（= 全画布）；**没有条目** = 未知。
/// - `allows()` 对**未知 id 放行**（fail-open）。这是刻意的：`ClipSnapshot::unclipped()`
///   必须让整个界面可用。代价是「快照漏了某个节点 ⇒ 裁剪护栏静默失效」——
///   所以**测试里必须先断言 `is_known()`** 再断言「被裁掉不命中」（见测试模块的写法）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClipSnapshot {
    clips: BTreeMap<String, Option<RectI>>,
}

impl ClipSnapshot {
    /// 全不裁剪（全画布）。
    pub fn unclipped() -> ClipSnapshot {
        ClipSnapshot::default()
    }

    /// 手工登记一个节点的有效裁剪（`None` = 不裁剪）。测试与窗口层都可直接用。
    pub fn with_node_clip(mut self, id: impl Into<String>, clip: Option<RectI>) -> ClipSnapshot {
        self.clips.insert(id.into(), clip);
        self
    }

    /// 该节点有**明确登记**吗（无论裁不裁剪）。
    ///
    /// 护栏用：不先断言它，「被裁掉不命中」可能只是因为快照里根本没这个 id。
    pub fn is_known(&self, id: &str) -> bool {
        self.clips.contains_key(id)
    }

    /// 该节点的有效裁剪（`None` = 不裁剪**或**未知）。
    pub fn clip_of(&self, id: &str) -> Option<RectI> {
        self.clips.get(id).copied().flatten()
    }

    pub fn len(&self) -> usize {
        self.clips.len()
    }

    pub fn is_empty(&self) -> bool {
        self.clips.is_empty()
    }

    /// 已登记的节点 id（树序无关，按字典序 —— 只用于诊断与断言）。
    pub fn ids(&self) -> impl Iterator<Item = &str> {
        self.clips.keys().map(String::as_str)
    }

    /// 点 `(x, y)` 在这个节点上是否**没被裁掉**。未知 id ⇒ 放行。
    pub fn allows(&self, id: &str, x: f32, y: f32) -> bool {
        match self.clips.get(id) {
            None | Some(None) => true,
            Some(Some(r)) => point_in_rect(r, x, y),
        }
    }

    /// 从一份绘制列表派生「每个节点的有效裁剪」。
    ///
    /// **协议**（因为 `NodeHint` 不带 id 本身，只能这样绑定）：
    /// 把列表里第 k 个 `NodeHint` 绑到**前序遍历**里第 k 个「有几何的节点」上，
    /// 并用 `node_id_len` **和** `node_id_fp`（id 的确定性指纹）双重校验
    /// （对不上就 panic，不静默错位；长度单独用是不够的 —— 等长 id 互换躲得过它）。
    /// 列表里**一个 `NodeHint` 都没有**时返回**空快照**（例如 `DefaultRenderer`
    /// 不产出 `NodeHint`）—— 这时没有绑定信息，只能全不裁剪；`is_empty()` 为真，
    /// 调用方可以据此断言前置条件，而不是误以为「裁剪已生效」。
    ///
    /// 裁剪栈语义逐字照抄 CPU 后端（`null.rs:313-329`）：`PushClip` 求交、
    /// `PopClip` 出栈、**空栈 `PopClip` ⇒ 退回全画布**。
    pub fn from_draw_list(list: &DrawList, root: &Node, geo: &Geometry) -> ClipSnapshot {
        let hints = list
            .cmds
            .iter()
            .filter(|c| matches!(c, DrawCmd::NodeHint { .. }))
            .count();
        if hints == 0 {
            return ClipSnapshot::unclipped();
        }

        let mut ordered: Vec<&Node> = Vec::new();
        collect_with_geometry(root, geo, &mut ordered);
        assert_eq!(
            hints,
            ordered.len(),
            "NodeHint 的数量（{hints}）必须等于「有几何的节点」的数量（{}）\
             —— 否则「第 k 个提示 = 第 k 个节点」的绑定不成立，派生出的裁剪会**张冠李戴**。\
             （一份绘制列表要么给每个有几何的节点都发 NodeHint，要么一个都不发。）",
            ordered.len()
        );

        let mut out: BTreeMap<String, Option<RectI>> = BTreeMap::new();
        let mut stack: Vec<Option<RectI>> = Vec::new();
        // `None` = 全画布（栈空时的当前裁剪）。
        let mut clip: Option<RectI> = None;
        let mut next = ordered.iter();

        for cmd in &list.cmds {
            match cmd {
                DrawCmd::PushClip { rect } => {
                    stack.push(clip);
                    // 与 `null.rs` 的求交逐字一致（夹到相交区，负宽度夹成 0）。
                    clip = Some(match clip {
                        Some(c) => intersect(&c, rect),
                        None => *rect,
                    });
                }
                DrawCmd::PopClip => {
                    // 空栈：`unwrap_or(full)` ⇒ 退回全画布（既有语义，不是新发明）。
                    clip = stack.pop().flatten();
                }
                DrawCmd::NodeHint {
                    node_id_len,
                    node_id_fp,
                    ..
                } => {
                    let n = next
                        .next()
                        .expect("NodeHint 比「有几何的节点」还多：绘制列表与这棵树不是同一份");
                    // ① 长度（便宜的前置检查，失败信息最直白）。
                    assert_eq!(
                        *node_id_len,
                        n.id.len() as u32,
                        "NodeHint 的长度校验和与节点 `{}` 对不上（顺序错位）",
                        n.id
                    );
                    // ② **指纹**：只有长度时「等长 id 互换」会逐项相同、静默错位
                    //    （改前实测：不 panic，且命中节点的裁剪张冠李戴）。
                    assert_eq!(
                        *node_id_fp,
                        deer_gpu::draw::node_id_fp(&n.id),
                        "NodeHint 的 **id 指纹** 与节点 `{}` 对不上：绘制列表与这棵树不是同一份\
                         （长度校验和看不见这类错位 —— 等长 id 互换就是典型）。",
                        n.id
                    );
                    out.insert(n.id.clone(), clip);
                }
                _ => {}
            }
        }
        assert!(
            next.next().is_none(),
            "还有「有几何的节点」没有对应的 NodeHint：绑定不完整，裁剪快照会漏节点"
        );

        ClipSnapshot { clips: out }
    }
}

/// 前序遍历，只收集**有几何**的节点（顺序与 `NullRenderer::build` 的产出顺序一致）。
fn collect_with_geometry<'a>(n: &'a Node, geo: &Geometry, out: &mut Vec<&'a Node>) {
    if geo.contains_key(&n.id) {
        out.push(n);
    }
    for c in &n.children {
        collect_with_geometry(c, geo, out);
    }
}

/// 两个裁剪矩形求交（与 `crates/deer-gpu/src/null.rs:319-326` 同一算例）。
fn intersect(a: &RectI, b: &RectI) -> RectI {
    let x = a.x.max(b.x);
    let y = a.y.max(b.y);
    let r = a.right().min(b.right());
    let bottom = a.bottom().min(b.bottom());
    RectI::new(x, y, (r - x).max(0), (bottom - y).max(0))
}

/// **半开区间**判据：与 `hit_test` 逐字一致（`px ∈ [x, x+w)`、`py ∈ [y, y+h)`）。
///
/// 这里是 f32 版本（指针位置是 f32，裁剪矩形是整数）；对 `i32` 范围内的值，
/// 与 `RectI::contains` 的整数判据等价。
fn point_in_rect(r: &RectI, x: f32, y: f32) -> bool {
    x >= r.x as f32 && y >= r.y as f32 && x < r.right() as f32 && y < r.bottom() as f32
}

// ---------------------------------------------------------------------------
// 四、命中测试（M5-2）
// ---------------------------------------------------------------------------

/// 命中测试：**复用 `deer_layout::hit_test`** 取最深命中者，再叠加两条本层的规则：
///
/// 1. **禁用**：命中节点自身或任一祖先 `props.disabled` ⇒ 整个点不命中（禁用子树不响应输入）；
/// 2. **裁剪**：该点在命中节点的**有效裁剪**外（含从没有被登记的祖先继承来的裁剪）⇒ 不命中。
///
/// **不回退到祖先**（理由见模块注释的结论 3）。
pub fn hit<'a>(root: &'a Node, geo: &Geometry, clip: ClipSnapshot, x: f32, y: f32) -> Option<&'a Node> {
    let deepest = deer_layout::hit_test(root, geo, x, y)?;
    let verdict = probe_input_path(root, deepest as *const Node, false, None, &clip).expect(
        "内部不变式：`hit_test` 的返回值必定取自这棵树 —— 走到这里说明路径遍历写错了",
    );
    if verdict.dead {
        return None;
    }
    match verdict.clip {
        Some(r) if !point_in_rect(&r, x, y) => None,
        _ => Some(deepest),
    }
}

/// 沿「根 → `target`」这条链求出的输入判据。
struct PathVerdict {
    /// 自身或任一祖先被禁用。
    dead: bool,
    /// 最近的显式裁剪（自身优先，否则继承祖先）；`None` = 全画布。
    clip: Option<RectI>,
}

/// 找到 `target` 并带回它这条链上的判据。
///
/// 这**不是**第二套命中测试：它不做「点在哪」的判断（那是 `hit_test` 的活），
/// 只按**指针同一性**找到 `hit_test` 已经选出的那个节点，再读它这条链上的禁用与裁剪。
fn probe_input_path(
    n: &Node,
    target: *const Node,
    dead_above: bool,
    clip_above: Option<RectI>,
    snap: &ClipSnapshot,
) -> Option<PathVerdict> {
    let dead = dead_above || n.props.disabled;
    // 自身的显式裁剪优先；没有就继承最近祖先的（父被裁剪 ⇒ 子也在裁剪内）。
    let clip = match snap.clips.get(&n.id) {
        Some(v) => *v,
        None => clip_above,
    };
    if std::ptr::eq(n as *const Node, target) {
        return Some(PathVerdict { dead, clip });
    }
    for c in &n.children {
        if let Some(v) = probe_input_path(c, target, dead, clip, snap) {
            return Some(v);
        }
    }
    None
}

/// 该节点（或其祖先）是否禁用 —— 键盘输入用（键盘不看指针，所以不查裁剪）。
fn path_is_dead(root: &Node, node: &Node) -> bool {
    let no_clip = ClipSnapshot::unclipped();
    probe_input_path(root, node as *const Node, false, None, &no_clip)
        .map(|v| v.dead)
        // 找不到就当「死」：拿不准就不给它输入（保守方向）。
        .unwrap_or(true)
}

/// 按 id 找节点（树序前序，确定性）。
fn node_by_id<'a>(n: &'a Node, id: &str) -> Option<&'a Node> {
    if n.id == id {
        return Some(n);
    }
    n.children.iter().find_map(|c| node_by_id(c, id))
}

/// 这个 id 是不是**可聚焦**的控件（`Button`/`Field` 且不在禁用子树里）。
///
/// 与 [`focusables`] 的可聚焦集合**共用同一条判据**（`kind` + 禁用），所以
/// 「点击能聚焦谁」与「`Tab` 能走到谁」不会分叉 —— 否则会出现
/// 「`Tab` 走不到的控件，点一下就能聚焦」这种两套规则。
fn node_is_focusable(root: &Node, id: &str) -> bool {
    node_by_id(root, id)
        .filter(|n| matches!(n.kind, Kind::Button | Kind::Field))
        .is_some_and(|n| !path_is_dead(root, n))
}

/// 指针位置上的节点 id（`hit` 的薄封装，省得每个分支都写一遍）。
fn node_id_at(root: &Node, geo: &Geometry, clip: ClipSnapshot, x: f32, y: f32) -> Option<String> {
    hit(root, geo, clip, x, y).map(|n| n.id.clone())
}

// ---------------------------------------------------------------------------
// 四b、滚轮 → 滚动偏移（剩余工作第 1 项）
// ---------------------------------------------------------------------------

/// `id` 这条链上**最深**的可滚动容器（含 `id` 自身）；没有 ⇒ `None`。
///
/// 判据是 [`Node::is_scroll_container`] —— 布局、渲染、这里三处**同一个条件**，
/// 免得「谁能被滚动」出现第三种说法。
fn nearest_scroll_container<'a>(n: &'a Node, id: &str, best: Option<&'a Node>) -> Option<&'a Node> {
    let best = if n.is_scroll_container() { Some(n) } else { best };
    if n.id == id {
        return best;
    }
    for c in &n.children {
        if let Some(found) = nearest_scroll_container(c, id, best) {
            return Some(found);
        }
    }
    None
}

/// 处理一次滚轮：**悬停节点最近的可滚动祖先（含自身）**滚动一个步长。
///
/// 为什么目标来自 `hover`：`InputEvent::Wheel` **没有坐标**（M5-1 冻结的事件模型），
/// 而「滚轮滚谁」必须确定。`hover` 是「指针当前压在谁身上」的唯一真相（由
/// `PointerMoved`/`PointerDown` 维护），所以它就是唯一合理的答案 —— 而不是另记一个
/// 「最后一次指针位置」（那会让滚轮在指针从未移动过时命中 `(0,0)`）。
///
/// 三道前置（缺一条就 `None`，不改任何状态）：① 有 `hover`；② 该节点不在**禁用子树**里
/// （与「禁用子树不响应输入」同一条规则）；③ 这条链上有可滚动容器，且偏移**真的变了**
/// （到顶/到底、`max_scroll == 0`、`dy == 0` 都不发事件）。
fn wheel_scroll(state: &mut UiState, root: &Node, dy: f32) -> Option<(String, i32)> {
    let hover = state.hover.clone()?;
    let node = node_by_id(root, &hover)?;
    if path_is_dead(root, node) {
        return None;
    }
    let target = nearest_scroll_container(root, &hover, None)?.id.clone();
    // `dy < 0`（向下拨）⇒ 内容上移 ⇒ 偏移增大。
    let delta = (-(dy * WHEEL_STEP_PX as f32)).round() as i32;
    state
        .scroll
        .scroll_by(&target, delta)
        .map(|offset| (target, offset))
}

// ---------------------------------------------------------------------------
// 五、焦点顺序（M5-2）
// ---------------------------------------------------------------------------

/// 可聚焦节点的**树序**（`Tab` 循环用）。
///
/// 可聚焦 = `Kind::Button` / `Kind::Field`，且**不在禁用子树里**（禁用的整棵子树跳过）。
/// 只依赖树，不依赖几何 —— 所以「零尺寸的按钮仍在焦点序列里」，
/// 但它在屏幕上按不到（`hit` 不给它）。这是刻意的：焦点序不引入第二个几何真相。
pub fn focusables(root: &Node) -> Vec<String> {
    fn walk(n: &Node, dead: bool, out: &mut Vec<String>) {
        // 禁用节点的**整棵子树**都不响应输入 ⇒ 直接停在这里。
        if dead || n.props.disabled {
            return;
        }
        if matches!(n.kind, Kind::Button | Kind::Field) {
            out.push(n.id.clone());
        }
        for c in &n.children {
            walk(c, false, out);
        }
    }
    let mut out = Vec::new();
    walk(root, false, &mut out);
    out
}

/// 树序里的下一个焦点。`back` = `Shift` 按下（反向）。
///
/// - 当前焦点为 `None`（或已不在可聚焦集合里）⇒ 正向取第一个、反向取最后一个；
/// - 只有**一个**可聚焦节点时停在原地（调用方据此不发 `FocusChanged`）。
fn next_focus(ids: &[String], current: Option<&str>, back: bool) -> Option<String> {
    let n = ids.len();
    if n == 0 {
        return None;
    }
    let idx = current.and_then(|c| ids.iter().position(|i| i == c));
    let next = match idx {
        Some(i) if back => (i + n - 1) % n,
        Some(i) => (i + 1) % n,
        None if back => n - 1,
        None => 0,
    };
    Some(ids[next].clone())
}

// ---------------------------------------------------------------------------
// 六、状态机（M5-2）
// ---------------------------------------------------------------------------

/// 纯状态机：喂一个输入事件，产出「发生了什么」，并就地更新 `state`。
///
/// | 事件 | 效果 |
/// |---|---|
/// | `PointerMoved` | 同步 `hover`（变了才发 `HoverChanged`） |
/// | `PointerDown { Left }` | 同步 `hover`；**命中的可聚焦控件 ⇒ 聚焦它**（变了才发 `FocusChanged`）；记 `pressed` |
/// | `PointerUp { Left }` | 同步 `hover`；**抬起处的节点 == 按下时的节点** ⇒ `Clicked`；无论如何清 `pressed` |
/// | `KeyDown { Tab }` | 树序循环焦点（`Shift` 反向）⇒ `FocusChanged` |
/// | `KeyDown { Escape }` | 清焦点 ⇒ `FocusChanged(None)` |
/// | `KeyDown { Enter }` | 焦点在**启用的按钮**上 ⇒ `Clicked`（键激活 = 点击） |
/// | `KeyDown { Backspace }` | 焦点是启用的输入框 ⇒ 删**一个字符**（Unicode 字符，不是字节）⇒ `TextChanged` |
/// | `TextInput` | 焦点是启用的输入框 ⇒ 追加 ⇒ `TextChanged` |
/// | `FocusChanged { focused: false }` | 窗口失焦：清 `hover`/`pressed`（`focus`/`texts` 不动）|
/// | `Wheel { dy }` | **滚动**：`hover` 最近的可滚动祖先（含自身）偏移 `-dy ×` [`WHEEL_STEP_PX`]，夹在 `[0, max_scroll]`；变了才发 `Scrolled` |
/// | 其余（`KeyUp`、右/中键、方向键、`Key::Char`/`Other`、`focused: true`） | 本里程碑不消费（见「已知边界」） |
///
/// 只有**状态真的变了**才产出事件（`M5-4` 的 dirty 约定依赖这一点）。
///
/// 匹配是**穷尽**的（没有 `_` 兜底）：将来给 `InputEvent` 加分支会**编译报错**，
/// 不会变成一条被静默忽略的输入。
pub fn handle(
    state: &mut UiState,
    root: &Node,
    geo: &Geometry,
    clip: ClipSnapshot,
    ev: &InputEvent,
) -> Vec<UiEvent> {
    /// 指针事件都会先同步 `hover`：脚本可能不发 `PointerMoved`（键盘重放、窗口重进）。
    fn sync_hover(state: &mut UiState, out: &mut Vec<UiEvent>, next: Option<String>) {
        if state.hover != next {
            state.hover = next.clone();
            out.push(UiEvent::HoverChanged(next));
        }
    }

    let mut out: Vec<UiEvent> = Vec::new();

    match ev {
        InputEvent::PointerMoved { x, y } => {
            let id = node_id_at(root, geo, clip, *x, *y);
            sync_hover(state, &mut out, id);
        }
        InputEvent::PointerDown {
            button: PointerButton::Left,
            x,
            y,
        } => {
            let id = node_id_at(root, geo, clip, *x, *y);
            sync_hover(state, &mut out, id.clone());
            // **点击可聚焦控件 ⇒ 聚焦它**（M5-4）。`id` 来自 `hit`，所以它已经过了
            // 「禁用子树」与「裁剪」两道判据 —— 被裁掉/被禁用的点根本拿不到 id。
            //
            // 只认 `Button`/`Field`（与 `focusables` 的可聚焦集合逐字一致）：点到容器或
            // 文本上**不改焦点**（那会让 `App` 的 `hover` 语义悄悄变成第二套焦点规则）。
            if let Some(id) = id
                .as_deref()
                .filter(|id| node_is_focusable(root, id))
                .map(str::to_string)
            {
                if state.focus.as_deref() != Some(id.as_str()) {
                    state.focus = Some(id.clone());
                    out.push(UiEvent::FocusChanged(Some(id)));
                }
            }
            state.pressed = id;
        }
        // 右/中键不参与按下与点击（右键菜单/中键滚动是 M6+ 的事）。
        InputEvent::PointerDown { .. } => {}
        InputEvent::PointerUp {
            button: PointerButton::Left,
            x,
            y,
        } => {
            let id = node_id_at(root, geo, clip, *x, *y);
            sync_hover(state, &mut out, id.clone());
            if let Some(pressed) = state.pressed.take() {
                // 抬起处必须是**按下时那个节点**：按下后移出再抬起不算点击。
                if id.as_deref() == Some(pressed.as_str()) {
                    out.push(UiEvent::Clicked(pressed));
                }
            }
        }
        InputEvent::PointerUp { .. } => {}
        InputEvent::Wheel { dy, .. } => {
            // 滚轮滚「`hover` 最近的可滚动祖先（含自身）」，一个步长见 [`WHEEL_STEP_PX`]。
            // `dx`（水平）**本期忽略**：只做垂直滚动（`Row` 上的 `scroll` 也被忽略）。
            // 边界与「没变」都由 `ScrollState::scroll_by` 负责（夹取 + 变了才回 `Some`）。
            if let Some((id, offset)) = wheel_scroll(state, root, *dy) {
                out.push(UiEvent::Scrolled { id, offset });
            }
        }
        InputEvent::KeyDown {
            key: Key::Tab,
            mods,
        } => {
            let ids = focusables(root);
            if let Some(next) = next_focus(&ids, state.focus.as_deref(), mods.shift) {
                if state.focus.as_deref() != Some(next.as_str()) {
                    state.focus = Some(next.clone());
                    out.push(UiEvent::FocusChanged(Some(next)));
                }
            }
        }
        InputEvent::KeyDown {
            key: Key::Escape, ..
        } => {
            if state.focus.is_some() {
                state.focus = None;
                out.push(UiEvent::FocusChanged(None));
            }
        }
        InputEvent::KeyDown {
            key: Key::Enter, ..
        } => {
            let live_button = state
                .focus
                .as_deref()
                .and_then(|id| node_by_id(root, id))
                .filter(|n| n.kind == Kind::Button && !path_is_dead(root, n))
                .map(|n| n.id.clone());
            if let Some(id) = live_button {
                out.push(UiEvent::Clicked(id));
            }
        }
        InputEvent::KeyDown {
            key: Key::Backspace,
            ..
        } => {
            // `String::pop` 就是「删最后一个 **char**」—— 不会把多字节字符切成半个。
            if let Some((id, value)) = edit_focused_text(root, state, |buf| buf.pop().is_some()) {
                out.push(UiEvent::TextChanged { id, value });
            }
        }
        // 方向键 / `Key::Char` / `Key::Other`：本里程碑没有可移动的东西（无滚动、无光标移动）。
        InputEvent::KeyDown { .. } => {}
        InputEvent::KeyUp { .. } => {}
        InputEvent::TextInput { text } => {
            // 空串不算变化（否则会白白置一次 dirty）。
            if !text.is_empty() {
                let edit = edit_focused_text(root, state, |buf| {
                    buf.push_str(text);
                    true
                });
                if let Some((id, value)) = edit {
                    out.push(UiEvent::TextChanged { id, value });
                }
            }
        }
        InputEvent::FocusChanged { focused: false } => {
            // 窗口失焦：抬起事件可能永远不来了 ⇒ 按下状态必须丢，否则回来一点就成点击。
            state.pressed = None;
            if state.hover.is_some() {
                state.hover = None;
                out.push(UiEvent::HoverChanged(None));
            }
        }
        InputEvent::FocusChanged { focused: true } => {}
    }

    out
}

/// 对**焦点输入框**的文本缓冲做一次编辑；`Some((id, 新值))` = 真的变了。
///
/// 三道前置（缺一条就 `None`，不改任何状态）：① 有焦点；② 焦点节点是 `Field`；
/// ③ 它不在禁用子树里。顺带把「首次输入」变成 `texts` 里的一条空缓冲。
fn edit_focused_text(
    root: &Node,
    state: &mut UiState,
    edit: impl FnOnce(&mut String) -> bool,
) -> Option<(String, String)> {
    let id = state.focus.clone()?;
    let is_field = node_by_id(root, &id).is_some_and(|n| n.kind == Kind::Field);
    if !is_field {
        return None;
    }
    let node = node_by_id(root, &id)?;
    if path_is_dead(root, node) {
        return None;
    }
    let buf = state.texts.entry(id.clone()).or_default();
    if !edit(buf) {
        return None;
    }
    Some((id, buf.clone()))
}

// ---------------------------------------------------------------------------
// 七、单测：每条规则一例（`cargo test -p deer-gui --lib interaction`）
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use deer_layout::node::Rect;
    use deer_layout::node::Size;

    /// 手写几何（**不经过布局**）：断言依赖的矩形必须一眼能读出来。
    ///
    /// ```text
    /// app(0,0,200,120)
    /// ├── btn_ok      (10, 10, 40, 20)      启用
    /// ├── btn_no      (10, 40, 40, 20)      禁用
    /// │   └── btn_no_label (14, 44, 20, 12) 禁用子树里的叶子
    /// ├── name        (60, 10,100, 20)      输入框
    /// ├── off         (60, 40,100, 20)      禁用的输入框
    /// └── btn_last    (10, 70, 40, 20)      启用
    /// ```
    fn tree() -> Node {
        Node::new(Kind::Column, "app")
            .with_layout(deer_layout::node::LayoutProps {
                width: Some(Size::Px(200.0)),
                height: Some(Size::Px(120.0)),
                ..Default::default()
            })
            .push(Node::new(Kind::Button, "btn_ok").with_label("确定"))
            .push(
                Node::new(Kind::Button, "btn_no")
                    .with_label("禁用")
                    .disabled()
                    .push(Node::new(Kind::Text, "btn_no_label").with_label("子")),
            )
            .push(Node::new(Kind::Field, "name").with_label(""))
            .push(Node::new(Kind::Field, "off").with_label("").disabled())
            .push(Node::new(Kind::Button, "btn_last").with_label("最后"))
    }

    fn geo() -> Geometry {
        let mut g = Geometry::new();
        for (id, (x, y, w, h)) in [
            ("app", (0.0, 0.0, 200.0, 120.0)),
            ("btn_ok", (10.0, 10.0, 40.0, 20.0)),
            ("btn_no", (10.0, 40.0, 40.0, 20.0)),
            ("btn_no_label", (14.0, 44.0, 20.0, 12.0)),
            ("name", (60.0, 10.0, 100.0, 20.0)),
            ("off", (60.0, 40.0, 100.0, 20.0)),
            ("btn_last", (10.0, 70.0, 40.0, 20.0)),
        ] {
            g.insert(id.to_string(), Rect::new(x, y, w, h));
        }
        g
    }

    /// 每个测试开始前先钉住前置：几何表必须真的有这些矩形（不然期望值就是空谈）。
    fn fixture() -> (Node, Geometry) {
        let t = tree();
        let g = geo();
        // 前置①：树里的每个节点都有几何。
        let mut ids = Vec::new();
        collect_with_geometry(&t, &g, &mut ids);
        assert_eq!(
            ids.len(),
            count_nodes(&t),
            "测试前置：树里的节点数与几何条目数必须相等"
        );
        // 前置②：这条几何确实会被 `hit_test` 判为命中（否则裁剪/禁用测试没有判别力）。
        assert!(
            deer_layout::hit_test(&t, &g, 30.0, 20.0).is_some_and(|n| n.id == "btn_ok"),
            "测试前置：`)` 点 (30,20) 应当落在 btn_ok 上"
        );
        (t, g)
    }

    fn count_nodes(n: &Node) -> usize {
        1 + n.children.iter().map(count_nodes).sum::<usize>()
    }

    fn ev_at(id: &str, g: &Geometry) -> (f32, f32) {
        let r = g
            .get(id)
            .copied()
            .unwrap_or_else(|| panic!("测试前置：没有 {id} 的几何"));
        (r.x + r.w / 2.0, r.y + r.h / 2.0)
    }

    fn key(k: Key, shift: bool) -> InputEvent {
        InputEvent::KeyDown {
            key: k,
            mods: Mods {
                shift,
                ..Default::default()
            },
        }
    }

    /// 一份 `NodeHint`（真实用法：`NullRenderer` 给每个有几何的节点都发一条）。
    ///
    /// 这里**照样**走 `DrawCmd::node_hint`（唯一定义了「校验和怎么算」的地方）——
    /// 手写字面量会让测试与产出路径各有一份算法，护栏就会在测试里绿、在真实列表上红。
    fn hint(n: &Node, g: &Geometry) -> DrawCmd {
        let r = g
            .get(&n.id)
            .copied()
            .unwrap_or_else(|| panic!("测试前置：节点 {} 没有几何", n.id));
        DrawCmd::node_hint(RectI::new(r.x as i32, r.y as i32, r.w as i32, r.h as i32), &n.id)
    }

    /// **2a 的变异夹具**：两棵树只在**两个等长**兄弟 id 的**顺序**上不同。
    ///
    /// ```text
    /// app(0,0,60,40)
    /// ├── aaaa (0, 0,10,10)   ← 两个 id 都是 4 字节
    /// └── bbbb (20,0,10,10)
    /// ```
    ///
    /// `swapped = false` ⇒ 前序 `[app, aaaa, bbbb]`；`swapped = true` ⇒ `[app, bbbb, aaaa]`。
    /// 两棵树的 id 集合与几何**逐项相同**，只有兄弟顺序不同 —— 于是「第 k 条提示 =
    /// 第 k 个有几何的节点」这条绑定在两棵树上把**不同的 id** 绑到同一个 `NodeHint` 上。
    fn swap_fixture(swapped: bool) -> (Node, Geometry) {
        let (first, second) = if swapped {
            ("bbbb", "aaaa")
        } else {
            ("aaaa", "bbbb")
        };
        let t = Node::new(Kind::Column, "app")
            .push(Node::new(Kind::Button, first))
            .push(Node::new(Kind::Button, second));
        let mut g = Geometry::new();
        g.insert("app".into(), Rect::new(0.0, 0.0, 60.0, 40.0));
        // 几何**按 id** 登记 ⇒ 两棵树共用同一份几何，「错位」只可能来自绑定顺序。
        g.insert("aaaa".into(), Rect::new(0.0, 0.0, 10.0, 10.0));
        g.insert("bbbb".into(), Rect::new(20.0, 0.0, 10.0, 10.0));
        (t, g)
    }

    /// 前序里「有几何的节点」的 id（**顺序**就是绑定协议里的第 k 个）。
    fn ordered_ids(root: &Node, g: &Geometry) -> Vec<String> {
        let mut v = Vec::new();
        collect_with_geometry(root, g, &mut v);
        v.into_iter().map(|n| n.id.clone()).collect()
    }

    fn panic_text(e: &(dyn std::any::Any + Send)) -> String {
        if let Some(s) = e.downcast_ref::<&str>() {
            (*s).to_string()
        } else if let Some(s) = e.downcast_ref::<String>() {
            s.clone()
        } else {
            "<非字符串 panic>".to_string()
        }
    }

    /// **2a**：**等长 id 互换**的变异必须被拦下 —— 而且必须由 **id 指纹**拦下
    /// （长度校验和看不见它：两个 id 都是 4 字节）。
    ///
    /// 这条测试同时钉住两件事（改前它在第一件上红）：
    /// ① 改前**静默**给出错快照（`aaaa` 的裁剪被错位成 `None`）—— 这才是缺陷；
    /// ② 改后由**指纹**确定性 panic（断言消息里就是「id 指纹」，不是长度）。
    #[test]
    fn r19_equal_length_id_swap_must_be_caught_by_the_id_fingerprint() {
        let (a, ga) = swap_fixture(false);
        let (b, gb) = swap_fixture(true);

        // 前置①：这次变异**真的**换了前序（否则下面全在测空气）。
        let (ids_a, ids_b) = (ordered_ids(&a, &ga), ordered_ids(&b, &gb));
        println!("树 A 前序 = {ids_a:?}｜树 B 前序 = {ids_b:?}");
        assert_ne!(ids_a, ids_b, "测试前置：变异必须真的换了绑定顺序");
        // 前置②：**等长**互换 ⇒ 长度序列逐项相同 ⇒ 长度校验和对此完全免疫。
        let lens = |v: &[String]| v.iter().map(|s| s.len()).collect::<Vec<_>>();
        assert_eq!(
            lens(&ids_a),
            lens(&ids_b),
            "测试前置：这是「等长 id 互换」—— 长度序列必须逐项相同，\
             否则长度校验和就能抓住它，这条测试测的就不是那个盲区了"
        );
        assert_eq!(lens(&ids_a), vec![3, 4, 4], "测试前置：本夹具的长度序列");

        // 绘制列表来自树 A：**`aaaa` 那一刻正在裁剪里**，`bbbb` 在裁剪之外。
        // （裁剪的落点由列表本身编码 ⇒ 它就是「本应如此」的地面真相。）
        let list = build_list(&a, &ga, |id, l| match id {
            "aaaa" => l.push(DrawCmd::PushClip {
                rect: RectI::new(0, 0, 5, 5),
            }),
            "bbbb" => l.push(DrawCmd::PopClip),
            _ => {}
        });
        let counts = list.counts();
        println!(
            "列表：{} 条命令（node_hint {}｜push {}｜pop {}｜balanced {}）",
            list.len(),
            counts.node_hint,
            counts.push_clip,
            counts.pop_clip,
            list.clip_balanced()
        );
        assert_eq!(
            (counts.node_hint, counts.push_clip, counts.pop_clip),
            (3, 1, 1),
            "测试前置：3 条节点提示 + 一对裁剪命令"
        );
        assert!(list.clip_balanced(), "测试前置：裁剪栈配平");
        // 地面真相：按**列表来源树**，`aaaa` 的裁剪是 Some(0,0,5,5)、`bbbb` 是 None。
        let truth = ClipSnapshot::from_draw_list(&list, &a, &ga);
        println!(
            "地面真相（列表来源树 A）：aaaa={:?}｜bbbb={:?}",
            truth.clip_of("aaaa"),
            truth.clip_of("bbbb")
        );
        assert_eq!(truth.clip_of("aaaa"), Some(RectI::new(0, 0, 5, 5)));
        assert_eq!(truth.clip_of("bbbb"), None);

        // 变异：把**同一份列表**绑到兄弟顺序互换的树 B 上。
        // 改前实测：`node_id_len` 校验和全过 ⇒ **不 panic**，静默给出错快照
        // （`aaaa` 的裁剪变 `None`，`bbbb` 反而拿到 `Some(0,0,5,5)`）。
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            ClipSnapshot::from_draw_list(&list, &b, &gb)
        }));
        match outcome {
            Ok(snap) => panic!(
                "等长 id 互换**没有被拦下** ⇒ 静默给出错快照：\
                 aaaa 的裁剪 = {:?}（列表来源树给的是 {:?}）、bbbb 的裁剪 = {:?}（应为 None）。\
                 长度校验和（3,4,4 逐项相同）看不见这次错位。",
                snap.clip_of("aaaa"),
                Some(RectI::new(0, 0, 5, 5)),
                snap.clip_of("bbbb")
            ),
            Err(e) => {
                let msg = panic_text(&*e);
                println!("拦下了：{msg}");
                assert!(
                    msg.contains("id 指纹"),
                    "必须由 **id 指纹** 拦下（长度校验和看不见这次变异），实际消息：{msg}"
                );
            }
        }
    }

    /// 按前序给**每个有几何的节点**发一条 `NodeHint`，并允许在指定节点之前插裁剪命令。
    fn build_list(root: &Node, g: &Geometry, mut before: impl FnMut(&str, &mut DrawList)) -> DrawList {
        fn walk(
            n: &Node,
            g: &Geometry,
            list: &mut DrawList,
            before: &mut impl FnMut(&str, &mut DrawList),
        ) {
            if g.contains_key(&n.id) {
                before(&n.id, list);
                list.push(hint(n, g));
            }
            for c in &n.children {
                walk(c, g, list, before);
            }
        }
        let mut list = DrawList::new();
        walk(root, g, &mut list, &mut before);
        list
    }

    // ---- 规则 1：悬停进入 / 离开 ------------------------------------------

    #[test]
    fn r1_hover_enter_and_leave() {
        let (t, g) = fixture();
        let mut s = UiState::default();
        let (x, y) = ev_at("btn_ok", &g);

        let e1 = handle(&mut s, &t, &g, ClipSnapshot::unclipped(), &InputEvent::PointerMoved { x, y });
        println!("进入 btn_ok：event={e1:?} hover={:?}", s.hover);
        assert_eq!(e1, vec![UiEvent::HoverChanged(Some("btn_ok".into()))]);
        assert_eq!(s.hover.as_deref(), Some("btn_ok"));

        // 再移到同一处 ⇒ **没有事件**（状态没变）。
        let e2 = handle(&mut s, &t, &g, ClipSnapshot::unclipped(), &InputEvent::PointerMoved { x, y });
        assert!(e2.is_empty(), "同位置的 PointerMoved 不该产生事件，实际 {e2:?}");

        // 离开所有节点（几何之外）⇒ None。
        let e3 = handle(
            &mut s,
            &t,
            &g,
            ClipSnapshot::unclipped(),
            &InputEvent::PointerMoved { x: -5.0, y: -5.0 },
        );
        println!("移出到 (-5,-5)：event={e3:?} hover={:?}", s.hover);
        assert_eq!(e3, vec![UiEvent::HoverChanged(None)]);
        assert_eq!(s.hover, None);
    }

    /// 记录一条语义边界：**容器也是节点**，命中落在容器上时 `hover` 报容器。
    /// 所以「离开控件」不等于「没有 hover」。
    #[test]
    fn r1b_hover_on_container_reports_container() {
        let (t, g) = fixture();
        let mut s = UiState::default();
        // (110, 90)：在 app 内，但不在任何子节点上。
        let e = handle(
            &mut s,
            &t,
            &g,
            ClipSnapshot::unclipped(),
            &InputEvent::PointerMoved { x: 110.0, y: 90.0 },
        );
        println!("容器上的 hover：{e:?}");
        assert_eq!(e, vec![UiEvent::HoverChanged(Some("app".into()))]);
    }

    // ---- 规则 2：按下 → 抬起 ⇒ Clicked ------------------------------------

    #[test]
    fn r2_press_then_release_on_same_node_emits_clicked() {
        let (t, g) = fixture();
        let mut s = UiState::default();
        let (x, y) = ev_at("btn_ok", &g);

        let down = handle(
            &mut s,
            &t,
            &g,
            ClipSnapshot::unclipped(),
            &InputEvent::PointerDown { button: PointerButton::Left, x, y },
        );
        println!("按下：event={down:?} pressed={:?} focus={:?}", s.pressed, s.focus);
        // M5-4：按下可聚焦控件同时**聚焦它**（顺序：hover 先、focus 后，与事件表一致）。
        assert_eq!(
            down,
            vec![
                UiEvent::HoverChanged(Some("btn_ok".into())),
                UiEvent::FocusChanged(Some("btn_ok".into())),
            ]
        );
        assert_eq!(s.pressed.as_deref(), Some("btn_ok"));
        assert_eq!(s.focus.as_deref(), Some("btn_ok"), "点击可聚焦控件 ⇒ 它拿到焦点");

        let up = handle(
            &mut s,
            &t,
            &g,
            ClipSnapshot::unclipped(),
            &InputEvent::PointerUp { button: PointerButton::Left, x, y },
        );
        println!("抬起：event={up:?} pressed={:?}", s.pressed);
        assert_eq!(up, vec![UiEvent::Clicked("btn_ok".into())]);
        assert_eq!(s.pressed, None, "抬起后必须清 pressed");
    }

    // ---- 规则 3：按下后移出再抬起 ⇒ 不产生 Clicked -------------------------

    #[test]
    fn r3_press_then_release_outside_emits_no_clicked() {
        let (t, g) = fixture();
        let mut s = UiState::default();
        let (x, y) = ev_at("btn_ok", &g);

        handle(
            &mut s,
            &t,
            &g,
            ClipSnapshot::unclipped(),
            &InputEvent::PointerDown { button: PointerButton::Left, x, y },
        );
        assert_eq!(s.pressed.as_deref(), Some("btn_ok"), "前置：确实按下了");

        // 移到另一个按钮上（按下仍保持），再抬起。
        let (x2, y2) = ev_at("btn_last", &g);
        let moved = handle(
            &mut s,
            &t,
            &g,
            ClipSnapshot::unclipped(),
            &InputEvent::PointerMoved { x: x2, y: y2 },
        );
        println!("按下后移出：event={moved:?} hover={:?} pressed={:?}", s.hover, s.pressed);
        assert_eq!(moved, vec![UiEvent::HoverChanged(Some("btn_last".into()))]);
        assert_eq!(s.pressed.as_deref(), Some("btn_ok"), "移动不该清 pressed");
        assert_eq!(
            s.focus.as_deref(),
            Some("btn_ok"),
            "移动不改焦点（只有按下才聚焦）"
        );

        let up = handle(
            &mut s,
            &t,
            &g,
            ClipSnapshot::unclipped(),
            &InputEvent::PointerUp { button: PointerButton::Left, x: x2, y: y2 },
        );
        println!("移出后抬起：event={up:?}");
        assert!(up.is_empty(), "移出后抬起不该有 Clicked，实际 {up:?}");
        assert_eq!(s.pressed, None, "无论是否成点击，pressed 都要清掉");

        // 反面补充：直接移出**整个树**再抬起，同样没有 Clicked。
        handle(
            &mut s,
            &t,
            &g,
            ClipSnapshot::unclipped(),
            &InputEvent::PointerDown { button: PointerButton::Left, x, y },
        );
        handle(
            &mut s,
            &t,
            &g,
            ClipSnapshot::unclipped(),
            &InputEvent::PointerMoved { x: -1.0, y: -1.0 },
        );
        let up2 = handle(
            &mut s,
            &t,
            &g,
            ClipSnapshot::unclipped(),
            &InputEvent::PointerUp { button: PointerButton::Left, x: -1.0, y: -1.0 },
        );
        assert!(up2.is_empty(), "移出树外抬起不该有 Clicked，实际 {up2:?}");
    }

    // ---- 规则 3b（M5-4）：点击可聚焦控件 ⇒ 聚焦它 --------------------------

    /// 点击聚焦必须与 `focusables` 同集合、且**只在真的变了**时才发 `FocusChanged`
    /// （否则窗口层每按一下都会白置一次 dirty）。
    #[test]
    fn r3b_pointer_down_focuses_the_clicked_focusable_widget() {
        let (t, g) = fixture();
        let focusable = focusables(&t);
        println!("焦点树序={focusable:?}");

        // ① 点在**输入框**上 ⇒ 焦点变成它，并且**紧接着就能打字**（这才是「点击可聚焦」的用处）。
        let (fx, fy) = ev_at("name", &g);
        let mut s = UiState::default();
        let down = handle(
            &mut s,
            &t,
            &g,
            ClipSnapshot::unclipped(),
            &InputEvent::PointerDown { button: PointerButton::Left, x: fx, y: fy },
        );
        println!("点输入框：event={down:?} focus={:?} pressed={:?}", s.focus, s.pressed);
        assert_eq!(
            down,
            vec![
                UiEvent::HoverChanged(Some("name".into())),
                UiEvent::FocusChanged(Some("name".into())),
            ]
        );
        assert_eq!(s.focus.as_deref(), Some("name"));
        assert_eq!(s.pressed.as_deref(), Some("name"));
        let typed = handle(
            &mut s,
            &t,
            &g,
            ClipSnapshot::unclipped(),
            &InputEvent::TextInput { text: "hi".into() },
        );
        assert_eq!(
            typed,
            vec![UiEvent::TextChanged { id: "name".into(), value: "hi".into() }],
            "点击聚焦之后必须能直接输入"
        );

        // ② 点在**容器**上（app 的空白处，不在任何子节点里）⇒ 焦点**不变**。
        //    前置：这个点确实落在 app 上、且不在两个按钮/输入框上。
        let container_pt = (110.0f32, 90.0f32);
        assert_eq!(
            deer_layout::hit_test(&t, &g, container_pt.0, container_pt.1).map(|n| n.id.as_str()),
            Some("app"),
            "测试前置：这个点必须命中容器 app"
        );
        assert!(
            !focusable.contains(&"app".to_string()),
            "测试前置：app 不在焦点序列里"
        );
        let down2 = handle(
            &mut s,
            &t,
            &g,
            ClipSnapshot::unclipped(),
            &InputEvent::PointerDown {
                button: PointerButton::Left,
                x: container_pt.0,
                y: container_pt.1,
            },
        );
        println!("点容器：event={down2:?} focus={:?}", s.focus);
        assert_eq!(
            down2,
            vec![UiEvent::HoverChanged(Some("app".into()))],
            "点容器只改 hover，**不许**改焦点（不许有第二套焦点规则）"
        );
        assert_eq!(s.focus.as_deref(), Some("name"), "焦点停在原处");

        // ③ 再点**已经聚焦**的那个控件 ⇒ 只发 hover（焦点没变 ⇒ **不发** `FocusChanged`，
        //    所以窗口层不会因为「焦点事件」而白重绘一帧）。
        let again = handle(
            &mut s,
            &t,
            &g,
            ClipSnapshot::unclipped(),
            &InputEvent::PointerDown { button: PointerButton::Left, x: fx, y: fy },
        );
        println!("再点同一处：event={again:?}");
        assert_eq!(
            again,
            vec![UiEvent::HoverChanged(Some("name".into()))],
            "已聚焦控件上的重复按下不该再发 FocusChanged"
        );
        assert_eq!(s.focus.as_deref(), Some("name"));

        // ④ **点击能聚焦的集合 == `Tab` 能走到的集合**（同一条判据，不分叉）。
        for id in &focusable {
            let (x, y) = ev_at(id, &g);
            let mut s2 = UiState::default();
            handle(
                &mut s2,
                &t,
                &g,
                ClipSnapshot::unclipped(),
                &InputEvent::PointerDown { button: PointerButton::Left, x, y },
            );
            println!("点 {id} ⇒ focus={:?}", s2.focus);
            assert_eq!(s2.focus.as_deref(), Some(id.as_str()), "点 {id} 应当聚焦它");
        }
        // 反向：禁用按钮点不出焦点。
        let (nx, ny) = (45.0f32, 50.0f32);
        assert_eq!(
            deer_layout::hit_test(&t, &g, nx, ny).map(|n| n.id.as_str()),
            Some("btn_no"),
            "测试前置：这个点命中禁用按钮 btn_no"
        );
        let mut s3 = UiState::default();
        handle(
            &mut s3,
            &t,
            &g,
            ClipSnapshot::unclipped(),
            &InputEvent::PointerDown { button: PointerButton::Left, x: nx, y: ny },
        );
        assert_eq!(s3.focus, None, "禁用按钮点不出焦点（与 focusables 一致）");
    }

    // ---- 规则 4：禁用节点（及其子树）不响应 --------------------------------
    #[test]
    fn r4_disabled_node_and_subtree_ignore_input() {
        let (t, g) = fixture();
        let mut s = UiState::default();
        // 取一个在 btn_no 内、但**不在**它的子节点 btn_no_label 内的点 ——
        // 否则最深命中会是那个叶子（后序覆盖），测的就不是「禁用按钮自己」了。
        let (dx, dy) = (45.0f32, 50.0f32);
        let r_no = *g.get("btn_no").expect("测试前置：btn_no 有几何");
        let r_lab = *g.get("btn_no_label").expect("测试前置：btn_no_label 有几何");
        assert!(
            point_in_rect(&RectI::new(r_no.x as i32, r_no.y as i32, r_no.w as i32, r_no.h as i32), dx, dy),
            "测试前置：({dx},{dy}) 在 btn_no 里"
        );
        assert!(
            !point_in_rect(&RectI::new(r_lab.x as i32, r_lab.y as i32, r_lab.w as i32, r_lab.h as i32), dx, dy),
            "测试前置：({dx},{dy}) 不在它的子节点里"
        );

        // 前置：`hit_test` **会**命中禁用节点（否则这个测试在测空气）。
        let raw = deer_layout::hit_test(&t, &g, dx, dy).map(|n| n.id.clone());
        println!("hit_test 原始结果（不看禁用）：{raw:?} / hit() 结果：{:?}", hit(&t, &g, ClipSnapshot::unclipped(), dx, dy).map(|n| n.id.as_str()));
        assert_eq!(raw.as_deref(), Some("btn_no"), "测试前置：hit_test 必须命中 btn_no");

        let hover = handle(
            &mut s,
            &t,
            &g,
            ClipSnapshot::unclipped(),
            &InputEvent::PointerMoved { x: dx, y: dy },
        );
        assert!(hover.is_empty(), "禁用节点不该产生 hover，实际 {hover:?}");
        assert_eq!(s.hover, None);

        // 按下 + 抬起都不成点击。
        let down = handle(
            &mut s,
            &t,
            &g,
            ClipSnapshot::unclipped(),
            &InputEvent::PointerDown { button: PointerButton::Left, x: dx, y: dy },
        );
        assert!(down.is_empty(), "禁用节点不该产生 FocusChanged/HoverChanged，实际 {down:?}");
        assert_eq!(s.pressed, None, "禁用节点不该进入 pressed");
        assert_eq!(s.focus, None, "禁用节点不该拿到焦点（点击聚焦也不越权）");
        let up = handle(
            &mut s,
            &t,
            &g,
            ClipSnapshot::unclipped(),
            &InputEvent::PointerUp { button: PointerButton::Left, x: dx, y: dy },
        );
        assert!(up.is_empty(), "禁用节点不该有 Clicked，实际 {up:?}");

        // 禁用节点**子树里的叶子**同样不响应（几何上它在 btn_no 内）。
        let (lx, ly) = ev_at("btn_no_label", &g);
        let raw2 = deer_layout::hit_test(&t, &g, lx, ly).map(|n| n.id.clone());
        let h2 = hit(&t, &g, ClipSnapshot::unclipped(), lx, ly).map(|n| n.id.clone());
        println!("禁用子树：hit_test={raw2:?} / hit={h2:?}");
        assert_eq!(raw2.as_deref(), Some("btn_no_label"), "测试前置：hit_test 命中子树叶子");
        assert_eq!(h2, None, "禁用子树的叶子也不响应");

        // 禁用节点不进焦点序列（含子树）。
        let f = focusables(&t);
        println!("focusables={f:?}");
        assert_eq!(f, vec!["btn_ok".to_string(), "name".into(), "btn_last".into()]);
    }

    // ---- 规则 5：被裁剪掉的点不命中 ---------------------------------------

    #[test]
    fn r5_point_outside_clip_is_not_hit() {
        let (t, g) = fixture();
        // 裁到 btn_ok 的左半：x ∈ [10, 20)。
        let snap = ClipSnapshot::unclipped().with_node_clip("btn_ok", Some(RectI::new(10, 10, 10, 20)));
        assert!(snap.is_known("btn_ok"), "护栏前置：快照必须真的登记了 btn_ok");

        let inside = (15.0, 20.0);
        let outside_clip = (30.0, 20.0);

        // 前置：几何上两点都在 btn_ok 里（所以拒绝只能来自裁剪，不可能是几何）。
        for (x, y) in [inside, outside_clip] {
            let raw = deer_layout::hit_test(&t, &g, x, y).map(|n| n.id.clone());
            println!("({x},{y}) hit_test={raw:?} clip_of={:?}", snap.clip_of("btn_ok"));
            assert_eq!(raw.as_deref(), Some("btn_ok"), "测试前置：几何命中 btn_ok");
        }

        let h_in = hit(&t, &g, snap.clone(), inside.0, inside.1).map(|n| n.id.clone());
        let h_out = hit(&t, &g, snap, outside_clip.0, outside_clip.1).map(|n| n.id.clone());
        println!("裁剪内={h_in:?} 裁剪外={h_out:?}");
        assert_eq!(h_in.as_deref(), Some("btn_ok"));
        assert_eq!(h_out, None, "裁剪外的点不该命中");

        // 也走一遍状态机：hover 不会被裁掉的部分污染。
        let mut s = UiState::default();
        let e = handle(
            &mut s,
            &t,
            &g,
            ClipSnapshot::unclipped().with_node_clip("btn_ok", Some(RectI::new(10, 10, 10, 20))),
            &InputEvent::PointerMoved { x: outside_clip.0, y: outside_clip.1 },
        );
        assert!(e.is_empty(), "被裁掉的点不该产生 hover，实际 {e:?}");
        assert_eq!(s.hover, None);
    }

    // ---- 规则 6：Tab / Shift+Tab 按树序循环焦点 ----------------------------

    #[test]
    fn r6_tab_and_shift_tab_cycle_in_tree_order() {
        let (t, g) = fixture();
        let mut s = UiState::default();
        let order = focusables(&t);
        println!("焦点树序={order:?}");
        assert_eq!(order, vec!["btn_ok".to_string(), "name".into(), "btn_last".into()]);

        let tab = |s: &mut UiState, shift: bool| handle(s, &t, &g, ClipSnapshot::unclipped(), &key(Key::Tab, shift));

        // None → 第一个 → 第二个 → 第三个 → 回绕到第一个。
        assert_eq!(tab(&mut s, false), vec![UiEvent::FocusChanged(Some("btn_ok".into()))]);
        assert_eq!(tab(&mut s, false), vec![UiEvent::FocusChanged(Some("name".into()))]);
        assert_eq!(tab(&mut s, false), vec![UiEvent::FocusChanged(Some("btn_last".into()))]);
        assert_eq!(tab(&mut s, false), vec![UiEvent::FocusChanged(Some("btn_ok".into()))]);
        println!("正向一圈后 focus={:?}", s.focus);

        // Shift+Tab 反向：btn_ok → btn_last。
        assert_eq!(tab(&mut s, true), vec![UiEvent::FocusChanged(Some("btn_last".into()))]);
        assert_eq!(tab(&mut s, true), vec![UiEvent::FocusChanged(Some("name".into()))]);
        println!("反向两步后 focus={:?}", s.focus);

        // 焦点指向一个**已不可聚焦**的 id（例如节点被禁用）⇒ 正向回到第一个、反向到最后一个。
        s.focus = Some("btn_no".into());
        assert_eq!(tab(&mut s, false), vec![UiEvent::FocusChanged(Some("btn_ok".into()))]);
        s.focus = Some("btn_no".into());
        assert_eq!(tab(&mut s, true), vec![UiEvent::FocusChanged(Some("btn_last".into()))]);
    }

    /// 只有一个可聚焦节点时 `Tab` 停在原地 ⇒ 不发 `FocusChanged`（否则每次都白置 dirty）。
    #[test]
    fn r6b_tab_with_single_focusable_stays_put() {
        let t = Node::new(Kind::Column, "app").push(Node::new(Kind::Button, "only"));
        let mut g = Geometry::new();
        g.insert("app".into(), Rect::new(0.0, 0.0, 50.0, 50.0));
        g.insert("only".into(), Rect::new(0.0, 0.0, 50.0, 20.0));
        assert_eq!(focusables(&t), vec!["only".to_string()], "测试前置：只有一个可聚焦节点");

        let mut s = UiState::default();
        let e = handle(&mut s, &t, &g, ClipSnapshot::unclipped(), &key(Key::Tab, false));
        assert_eq!(e, vec![UiEvent::FocusChanged(Some("only".into()))]);
        let e2 = handle(&mut s, &t, &g, ClipSnapshot::unclipped(), &key(Key::Tab, false));
        assert!(e2.is_empty(), "只有一个焦点时不该再发 FocusChanged，实际 {e2:?}");
    }

    // ---- 规则 7：Escape 清焦点 --------------------------------------------

    #[test]
    fn r7_escape_clears_focus() {
        let (t, g) = fixture();
        let mut s = UiState {
            focus: Some("name".into()),
            ..Default::default()
        };
        let e = handle(&mut s, &t, &g, ClipSnapshot::unclipped(), &key(Key::Escape, false));
        println!("Escape：event={e:?} focus={:?}", s.focus);
        assert_eq!(e, vec![UiEvent::FocusChanged(None)]);
        assert_eq!(s.focus, None);

        // 已经没有焦点 ⇒ 无事件（状态没变）。
        let e2 = handle(&mut s, &t, &g, ClipSnapshot::unclipped(), &key(Key::Escape, false));
        assert!(e2.is_empty(), "无焦点时 Escape 不该有事件，实际 {e2:?}");
    }

    // ---- 规则 8：TextInput 追加 / Backspace 删一字符 -----------------------

    #[test]
    fn r8_text_input_appends_and_backspace_removes_one_char() {
        let (t, g) = fixture();
        let mut s = UiState {
            focus: Some("name".into()),
            ..Default::default()
        };
        let type_text = |s: &mut UiState, text: &str| {
            handle(
                s,
                &t,
                &g,
                ClipSnapshot::unclipped(),
                &InputEvent::TextInput { text: text.to_string() },
            )
        };
        let backspace = |s: &mut UiState| handle(s, &t, &g, ClipSnapshot::unclipped(), &key(Key::Backspace, false));

        let e1 = type_text(&mut s, "hi");
        println!("输入 hi：event={e1:?} texts={:?}", s.texts);
        assert_eq!(
            e1,
            vec![UiEvent::TextChanged { id: "name".into(), value: "hi".into() }]
        );
        assert_eq!(s.texts.get("name").map(String::as_str), Some("hi"));

        // 追加是**拼接**，不是覆盖。
        let e2 = type_text(&mut s, "你好");
        assert_eq!(
            e2,
            vec![UiEvent::TextChanged { id: "name".into(), value: "hi你好".into() }]
        );
        assert_eq!(s.texts["name"], "hi你好");

        // Backspace 删的是**一个 Unicode 字符**（不是字节）——「好」是 3 字节，删成半个就是缺陷。
        let e3 = backspace(&mut s);
        println!("Backspace：event={e3:?} texts={:?}", s.texts);
        assert_eq!(
            e3,
            vec![UiEvent::TextChanged { id: "name".into(), value: "hi你".into() }]
        );
        assert_eq!(s.texts["name"], "hi你", "必须是合法 UTF-8 的「hi你」");

        assert_eq!(
            backspace(&mut s),
            vec![UiEvent::TextChanged { id: "name".into(), value: "hi".into() }]
        );
        backspace(&mut s);
        backspace(&mut s);
        assert_eq!(s.texts["name"], "", "删空为止");
        // 已经空了 ⇒ 没有变化 ⇒ 没有事件。
        let e_empty = backspace(&mut s);
        assert!(e_empty.is_empty(), "空缓冲上的 Backspace 不该有事件，实际 {e_empty:?}");
    }

    #[test]
    fn r8b_text_input_needs_a_live_focused_field() {
        let (t, g) = fixture();
        let mut s = UiState::default();

        // 无焦点 ⇒ 忽略。
        let e0 = handle(
            &mut s,
            &t,
            &g,
            ClipSnapshot::unclipped(),
            &InputEvent::TextInput { text: "x".into() },
        );
        assert!(e0.is_empty() && s.texts.is_empty(), "无焦点时不该吃输入");

        // 焦点在按钮上 ⇒ 忽略。
        s.focus = Some("btn_ok".into());
        let e1 = handle(
            &mut s,
            &t,
            &g,
            ClipSnapshot::unclipped(),
            &InputEvent::TextInput { text: "x".into() },
        );
        assert!(e1.is_empty() && s.texts.is_empty(), "按钮上的文字输入不该被吞");

        // 焦点在**被禁用的**输入框上 ⇒ 忽略。
        s.focus = Some("off".into());
        let e2 = handle(
            &mut s,
            &t,
            &g,
            ClipSnapshot::unclipped(),
            &InputEvent::TextInput { text: "x".into() },
        );
        assert!(e2.is_empty() && s.texts.is_empty(), "被禁用的输入框不该吃输入");
        let e2b = handle(&mut s, &t, &g, ClipSnapshot::unclipped(), &key(Key::Backspace, false));
        assert!(e2b.is_empty() && s.texts.is_empty(), "被禁用的输入框不该响应 Backspace");

        // 空串输入不算变化。
        s.focus = Some("name".into());
        let e3 = handle(
            &mut s,
            &t,
            &g,
            ClipSnapshot::unclipped(),
            &InputEvent::TextInput { text: String::new() },
        );
        assert!(e3.is_empty() && s.texts.is_empty(), "空串输入不该产生 TextChanged");
    }

    // ---- 规则 9：边界点（半开区间） ---------------------------------------

    #[test]
    fn r9_boundaries_are_half_open() {
        let (t, g) = fixture();
        // btn_ok = (10,10,40,20) ⇒ x ∈ [10,50)、y ∈ [10,30)。
        for (x, y, expect) in [
            (10.0, 10.0, true),   // 左上角：含
            (49.999, 29.999, true),
            (50.0, 20.0, false),  // 右边界：不含
            (30.0, 30.0, false),  // 下边界：不含
            (9.999, 20.0, false), // 左外侧
            (30.0, 9.999, false), // 上外侧
        ] {
            let via_layout = deer_layout::hit_test(&t, &g, x, y).map(|n| n.id.clone());
            let via_hit = hit(&t, &g, ClipSnapshot::unclipped(), x, y).map(|n| n.id.clone());
            println!("({x},{y}) hit_test={via_layout:?} hit={via_hit:?}");
            // 不裁剪、不禁用时，本层的 `hit` 必须与 `hit_test` 逐点一致。
            assert_eq!(via_layout, via_hit, "({x},{y}) 本层命中必须与 hit_test 一致");
            assert_eq!(via_hit.as_deref() == Some("btn_ok"), expect, "({x},{y}) 期望 {expect}");
        }
    }

    /// 裁剪矩形用**同一套**半开区间判据（否则「裁剪内」与「几何内」会有两种边界）。
    #[test]
    fn r9b_clip_boundaries_are_half_open() {
        let (t, g) = fixture();
        // 裁剪 = (10,10,10,20) ⇒ x ∈ [10,20)、y ∈ [10,30)。
        let snap = ClipSnapshot::unclipped().with_node_clip("btn_ok", Some(RectI::new(10, 10, 10, 20)));
        assert!(snap.is_known("btn_ok"), "护栏前置：快照登记了 btn_ok");
        for (x, expect) in [(10.0, true), (19.999, true), (20.0, false), (9.999, false)] {
            let h = hit(&t, &g, snap.clone(), x, 20.0).map(|n| n.id.clone());
            println!("裁剪边界 x={x} ⇒ {h:?}");
            assert_eq!(h.as_deref() == Some("btn_ok"), expect, "x={x} 期望 {expect}");
        }
    }

    // ---- 规则 10：嵌套裁剪求交 --------------------------------------------

    #[test]
    fn r10_nested_clips_intersect() {
        let (t, g) = fixture();
        // 外层 (5,5,150,60)，内层 (60,5,200,60) ⇒ 交集 (60,5,95,60)。
        let list = build_list(&t, &g, |id, l| match id {
            "btn_ok" => l.push(DrawCmd::PushClip { rect: RectI::new(5, 5, 150, 60) }),
            "name" => l.push(DrawCmd::PushClip { rect: RectI::new(60, 5, 200, 60) }),
            "off" => l.push(DrawCmd::PopClip),  // 关掉内层
            "btn_last" => l.push(DrawCmd::PopClip), // 关掉外层
            _ => {}
        });
        assert!(list.clip_balanced(), "测试前置：本用例的裁剪栈必须配平");

        let snap = ClipSnapshot::from_draw_list(&list, &t, &g);
        println!("嵌套裁剪快照：{:?}", snap.ids().map(|i| (i, snap.clip_of(i))).collect::<Vec<_>>());
        assert!(snap.is_known("btn_ok") && snap.is_known("name"), "护栏前置：两个节点都在快照里");
        assert_eq!(snap.clip_of("btn_ok"), Some(RectI::new(5, 5, 150, 60)));
        assert_eq!(
            snap.clip_of("name"),
            Some(RectI::new(60, 5, 95, 60)),
            "嵌套裁剪必须求交，不是取内层"
        );
        assert_eq!(snap.clip_of("off"), Some(RectI::new(5, 5, 150, 60)), "内层关掉后只剩外层");
        assert_eq!(snap.clip_of("btn_last"), None, "两层都关掉 ⇒ 不裁剪");

        // 行为面：(70,20) 在交集内 ⇒ 命中 name；
        // 而 (157,20) 在 name 的几何内、却被**内层**裁掉（交集 x ∈ [60,155)）⇒ 不命中。
        // 注意 `btn_ok` 的判据是它**被画的那一刻**的裁剪（只有外层），这是栈语义，不是缺陷。
        let mut s = UiState::default();
        handle(
            &mut s,
            &t,
            &g,
            snap.clone(),
            &InputEvent::PointerMoved { x: 70.0, y: 20.0 },
        );
        assert_eq!(s.hover.as_deref(), Some("name"));
        // 前置：几何上 (157,20) 确实落在 name 上（矩形 x∈[60,160)）。
        assert_eq!(
            deer_layout::hit_test(&t, &g, 157.0, 20.0).map(|n| n.id.as_str()),
            Some("name"),
            "测试前置：几何命中 name"
        );
        let h = hit(&t, &g, snap, 157.0, 20.0).map(|n| n.id.clone());
        println!("name 几何内、内层裁剪外的点 (157,20) ⇒ {h:?}");
        assert_eq!(h, None, "被内层裁掉的点不命中");
    }

    // ---- 规则 11：空栈 PopClip ⇒ 退回全画布 -------------------------------

    #[test]
    fn r11_pop_clip_on_empty_stack_falls_back_to_full_canvas() {
        let (t, g) = fixture();
        // btn_ok 处推入裁剪；随后**连弹两次**（第二次是空栈 PopClip）。
        let list = build_list(&t, &g, |id, l| match id {
            "btn_ok" => l.push(DrawCmd::PushClip { rect: RectI::new(10, 10, 10, 20) }),
            "btn_no" => l.push(DrawCmd::PopClip),
            "name" => l.push(DrawCmd::PopClip), // ← 空栈
            _ => {}
        });
        let counts = list.counts();
        println!(
            "列表：push_clip={} pop_clip={} clip_balanced={}",
            counts.push_clip,
            counts.pop_clip,
            list.clip_balanced()
        );
        // 前置：确实构造出了「多一个 PopClip」这份列表（它的语义是既有事实，不是本层发明的）。
        assert_eq!((counts.push_clip, counts.pop_clip), (1, 2), "测试前置：1 推 2 弹");

        let snap = ClipSnapshot::from_draw_list(&list, &t, &g);
        assert!(snap.is_known("name"), "护栏前置：btn_no/name 都必须在快照里");
        assert_eq!(snap.clip_of("btn_ok"), Some(RectI::new(10, 10, 10, 20)), "推入的裁剪生效");
        assert_eq!(snap.clip_of("btn_no"), None, "第一次 PopClip 退出裁剪 ⇒ 全画布");
        assert_eq!(snap.clip_of("name"), None, "空栈 PopClip ⇒ 退回全画布（既有语义）");
        assert_eq!(snap.clip_of("off"), None, "后续节点同样不受裁剪");
        assert_eq!(snap.clip_of("btn_last"), None);

        // 行为面：**弹出之后**的节点已回到全画布，应当照常命中。
        // （注意 `btn_ok` 自己的判据是**它被画的那一刻**的裁剪，仍然是被裁过的 —— 这是栈语义。）
        let (nx, ny) = ev_at("name", &g); // name 在两次 PopClip 之后
        let h = hit(&t, &g, snap, nx, ny).map(|n| n.id.clone());
        println!("退回全画布后 name 中心 ({nx},{ny}) ⇒ {h:?}");
        assert_eq!(h.as_deref(), Some("name"));
    }

    // ---- 规则 12：Enter 在聚焦按钮上等同点击 -------------------------------

    #[test]
    fn r12_enter_on_focused_button_is_a_click() {
        let (t, g) = fixture();
        let mut s = UiState {
            focus: Some("btn_ok".into()),
            ..Default::default()
        };
        let e = handle(&mut s, &t, &g, ClipSnapshot::unclipped(), &key(Key::Enter, false));
        println!("Enter：{e:?}");
        assert_eq!(e, vec![UiEvent::Clicked("btn_ok".into())]);

        // 焦点在输入框上 ⇒ 不产生 Clicked（也没有「提交」语义）。
        s.focus = Some("name".into());
        let e2 = handle(&mut s, &t, &g, ClipSnapshot::unclipped(), &key(Key::Enter, false));
        assert!(e2.is_empty(), "输入框上的 Enter 不该成点击，实际 {e2:?}");

        // 焦点在**被禁用的**按钮上 ⇒ 不产生 Clicked。
        s.focus = Some("btn_no".into());
        let e3 = handle(&mut s, &t, &g, ClipSnapshot::unclipped(), &key(Key::Enter, false));
        assert!(e3.is_empty(), "禁用按钮上的 Enter 不该成点击，实际 {e3:?}");

        // 无焦点 ⇒ 无事件。
        s.focus = None;
        let e4 = handle(&mut s, &t, &g, ClipSnapshot::unclipped(), &key(Key::Enter, false));
        assert!(e4.is_empty(), "无焦点时 Enter 不该有事件，实际 {e4:?}");
    }

    // ---- 规则 13：窗口失焦 ⇒ 清 hover/pressed（不清 focus/texts） ----------

    #[test]
    fn r13_window_blur_clears_hover_and_press() {
        let (t, g) = fixture();
        let mut s = UiState::default();
        let (x, y) = ev_at("btn_ok", &g);
        handle(
            &mut s,
            &t,
            &g,
            ClipSnapshot::unclipped(),
            &InputEvent::PointerMoved { x, y },
        );
        handle(
            &mut s,
            &t,
            &g,
            ClipSnapshot::unclipped(),
            &InputEvent::PointerDown { button: PointerButton::Left, x, y },
        );
        s.focus = Some("name".into());
        s.texts.insert("name".into(), "hi".into());

        let e = handle(
            &mut s,
            &t,
            &g,
            ClipSnapshot::unclipped(),
            &InputEvent::FocusChanged { focused: false },
        );
        println!("失焦：event={e:?} state={s:?}");
        assert_eq!(e, vec![UiEvent::HoverChanged(None)]);
        assert_eq!(s.hover, None);
        assert_eq!(s.pressed, None, "失焦后抬起事件可能不再来 ⇒ pressed 必须丢");
        assert_eq!(s.focus.as_deref(), Some("name"), "控件焦点与窗口焦点是两回事");
        assert_eq!(s.texts["name"], "hi", "文本不能被窗口失焦吃掉");

        // 失焦后回来：`focused: true` 不改任何状态（hover 由下一次 PointerMoved 恢复）。
        let e2 = handle(
            &mut s,
            &t,
            &g,
            ClipSnapshot::unclipped(),
            &InputEvent::FocusChanged { focused: true },
        );
        assert!(e2.is_empty(), "重新获得窗口焦点本身不该产生 UI 事件，实际 {e2:?}");
    }

    // ---- 规则 14（M5-3）：从真实绘制列表派生快照 ---------------------------

    #[test]
    fn r14_clip_snapshot_from_real_layout_and_null_renderer() {
        // 真实布局 + `NullRenderer`（它给每个有几何的节点发一条 NodeHint）。
        let t = tree();
        let style = deer_layout::layout::TextStyle::default();
        let g = deer_layout::layout::layout(
            &t,
            Rect::new(0.0, 0.0, 200.0, 120.0),
            style,
            &deer_layout::layout::ApproxMeasure,
        );
        let list = deer_gpu::NullRenderer::build(&t, &g);
        let counts = list.counts();
        println!("真实列表：node_hint={} 命令总数={}", counts.node_hint, list.len());
        assert!(!list.is_empty() && counts.node_hint > 0, "测试前置：确实有 NodeHint");
        assert!(list.clip_balanced(), "测试前置：没有裁剪命令 ⇒ 配平");

        let snap = ClipSnapshot::from_draw_list(&list, &t, &g);
        assert_eq!(snap.len(), counts.node_hint, "每个 NodeHint 都该绑到一个节点");
        assert!(snap.is_known("btn_ok"), "护栏前置：btn_ok 在快照里");
        for id in snap.ids() {
            assert_eq!(snap.clip_of(id), None, "{id} 不裁剪");
        }

        // 行为面：点到 btn_ok 的中心 ⇒ 命中它。
        let r = g.get("btn_ok").copied().expect("布局必须给 btn_ok 几何");
        let (x, y) = (r.x + r.w / 2.0, r.y + r.h / 2.0);
        let h = hit(&t, &g, snap, x, y).map(|n| n.id.clone());
        println!("btn_ok 中心 ({x},{y}) ⇒ {h:?}");
        assert_eq!(h.as_deref(), Some("btn_ok"));
    }

    /// 没有 `NodeHint` 的列表（`DefaultRenderer` 就是这样）⇒ 空快照。
    /// **这不是裁剪生效**，而是「没法绑定」—— 所以 `is_empty()` 必须能被断言。
    #[test]
    fn r15_snapshot_is_empty_when_list_has_no_node_hints() {
        let (t, g) = fixture();
        let mut list = DrawList::new();
        list.push(DrawCmd::PushClip { rect: RectI::new(0, 0, 10, 10) });
        list.push(DrawCmd::PopClip);
        assert_eq!(list.counts().node_hint, 0, "测试前置：这份列表没有 NodeHint");

        let snap = ClipSnapshot::from_draw_list(&list, &t, &g);
        assert!(snap.is_empty(), "没有节点提示 ⇒ 空快照");
        assert!(!snap.is_known("btn_ok"), "空快照里的 id 一律未知");

        // fail-open 是刻意的：空快照不能让整个界面失去命中。
        let (x, y) = ev_at("btn_ok", &g);
        assert_eq!(
            hit(&t, &g, snap, x, y).map(|n| n.id.as_str()),
            Some("btn_ok"),
            "空快照 ⇒ 不裁剪（fail-open），否则界面全哑"
        );
    }

    /// 护栏：绑定协议被破坏时必须**大声失败**，不能悄悄错位。
    #[test]
    #[should_panic(expected = "长度校验和")]
    fn r16_node_hint_length_mismatch_panics() {
        let (t, g) = fixture();
        let mut list = build_list(&t, &g, |_, _| {});
        // 把第一条提示的长度改错（模拟「列表与树不是同一份」）。
        if let Some(DrawCmd::NodeHint { node_id_len, .. }) = list.cmds.first_mut() {
            *node_id_len += 1;
        }
        let _ = ClipSnapshot::from_draw_list(&list, &t, &g);
    }

    /// 指纹这一条**自己**也要有判别力：长度**不动**、只把指纹改错 ⇒ 必须由指纹拦下。
    ///
    /// （否则「加了指纹」可能只是摆设 —— 长度那条正好也红了，就没人知道指纹有没有生效。）
    #[test]
    #[should_panic(expected = "id 指纹")]
    fn r16b_node_hint_fingerprint_mismatch_panics_even_when_length_matches() {
        let (t, g) = fixture();
        let mut list = build_list(&t, &g, |_, _| {});
        let first_len = match &list.cmds[0] {
            DrawCmd::NodeHint { node_id_len, .. } => *node_id_len,
            other => panic!("测试前置：第一条命令必须是 NodeHint，实际 {other:?}"),
        };
        if let Some(DrawCmd::NodeHint {
            node_id_len,
            node_id_fp,
            ..
        }) = list.cmds.first_mut()
        {
            *node_id_fp ^= 0xdead_beef; // 只动指纹
            assert_eq!(*node_id_len, first_len, "测试前置：长度必须**没**变");
        }
        let _ = ClipSnapshot::from_draw_list(&list, &t, &g);
    }

    #[test]
    #[should_panic(expected = "必须等于")]
    fn r17_node_hint_count_mismatch_panics() {
        let (t, g) = fixture();
        let mut list = build_list(&t, &g, |_, _| {});
        list.cmds.pop(); // 少一条提示 ⇒ 有节点没被绑定
        let _ = ClipSnapshot::from_draw_list(&list, &t, &g);
    }

    // ---- 附：状态机不消费的事件不能悄悄改状态 ------------------------------

    /// 本语料里**没有可滚动容器** ⇒ `Wheel` 也是空转（滚轮**已被消费**，只是这棵树滚不动）。
    /// 所以这条判据现在覆盖两件事：① 真正「不消费」的事件（`KeyUp`/右中键/方向键…）；
    /// ② 「消费了但无事可做」的 `Wheel`（没有可滚动祖先 ⇒ 不产生事件、不改状态）。
    #[test]
    fn r18_unconsumed_events_change_nothing() {
        let (t, g) = fixture();
        let (x, y) = ev_at("btn_ok", &g);
        let mut s = UiState {
            hover: Some("btn_ok".into()),
            focus: Some("name".into()),
            pressed: Some("btn_ok".into()),
            texts: BTreeMap::from([("name".to_string(), "hi".to_string())]),
            scroll: Default::default(),
        };
        // 前置：这份语料确实一个可滚动容器都没有（否则下面的 `Wheel` 断言在测空气）。
        let mut scrollers = Vec::new();
        t.walk(&mut |n, _| {
            if n.is_scroll_container() {
                scrollers.push(n.id.clone());
            }
        }, 0);
        assert!(
            scrollers.is_empty(),
            "测试前置：这棵树里不该有可滚动容器，实际 {scrollers:?}"
        );
        let before = s.clone();
        let events = [
            InputEvent::Wheel { dx: 0.0, dy: 3.0 },
            InputEvent::KeyUp { key: Key::Tab, mods: Mods::default() },
            InputEvent::PointerDown { button: PointerButton::Right, x, y },
            InputEvent::PointerUp { button: PointerButton::Middle, x, y },
            key(Key::Left, false),
            key(Key::Char('a'), false),
            key(Key::Other, false),
        ];
        for e in &events {
            let out = handle(&mut s, &t, &g, ClipSnapshot::unclipped(), e);
            assert!(out.is_empty(), "{e:?} 不该产生事件，实际 {out:?}");
        }
        assert_eq!(s, before, "不消费的事件不得改变任何状态");
    }

    /// 滚轮的三条前置：**没有 hover / 没有可滚动祖先 / 上限是 0** ⇒ 什么都不做。
    ///
    /// （「有 hover 且在可滚动容器里」的完整链路在
    /// `tests/scroll_multiline.rs` 里跑，那条用真实布局与真实绘制列表。）
    #[test]
    fn r19_wheel_needs_a_hover_inside_a_scrollable_container() {
        let (t, g) = fixture();
        let (x, y) = ev_at("btn_ok", &g);
        let wheel = InputEvent::Wheel { dx: 0.0, dy: -1.0 };

        // ① 没有 hover（指针从未移动过）⇒ 不改状态、不发事件。
        let mut s = UiState::default();
        let before = s.clone();
        let out = handle(&mut s, &t, &g, ClipSnapshot::unclipped(), &wheel);
        println!("无 hover ⇒ {out:?}");
        assert!(out.is_empty());
        assert_eq!(s, before);

        // ② 有 hover 但树里没有可滚动容器 ⇒ 同上。
        let mut s = UiState::default();
        handle(&mut s, &t, &g, ClipSnapshot::unclipped(), &InputEvent::PointerMoved { x, y });
        assert_eq!(s.hover.as_deref(), Some("btn_ok"), "前置：悬停在按钮上");
        let before = s.clone();
        let out = handle(&mut s, &t, &g, ClipSnapshot::unclipped(), &wheel);
        println!("无可滚动祖先 ⇒ {out:?}");
        assert!(out.is_empty());
        assert_eq!(s, before);

        // ③ `dy = 0`：即使上限非 0 也不是「变化」⇒ 不发事件（`ScrollState` 的判据）。
        let mut st = ScrollState::new();
        st.set_metrics(&deer_layout::layout::ScrollMetrics::new());
        assert_eq!(st.scroll_by("any", 0), None, "dy=0 时偏移没变 ⇒ None");
        assert_eq!(st.scroll_by("any", 40), None, "上限 0 ⇒ 夹取后仍是 0 ⇒ None（fail-closed）");
        assert_eq!(st.offset_of("any"), 0);
    }
}
