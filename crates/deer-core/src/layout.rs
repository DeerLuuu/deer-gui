//! 布局引擎：纯函数、确定性、可重放。
//!
//! 不变量（每条都有对应测试）：
//! - **I-1 纯函数**：不改输入树，只回一张几何表。
//! - **I-2 确定性**：同树 + 同盒子 ⇒ 逐位相同的几何（无时间/随机/环境探测）。
//! - **I-3 自底向上**：先算子节点固有尺寸，父容器再据此分配。
//! - **I-4 像素取整**：几何全部落到整数像素。
//! - **I-5 不假设拥有窗口**：根盒子由调用方给。
//! - **I-6 不越界**：结果被夹在可用空间内。
//! - **I-7 分配尺寸 vs 可用空间**（**deer-ui V0 的 B-2 缺陷，这里显式建模**）：
//!   父容器**分配**给子节点的主轴尺寸必须被子节点采信；可用空间只是上限。
//!   把两者混为一谈会导致 `grow` 分配的空间被静默丢弃。

use std::collections::HashMap;

use crate::node::{Align, Kind, Node, Rect, Size};

/// 文本样式。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextStyle {
    pub font_size: f32,
    pub line_height: f32,
}

impl Default for TextStyle {
    fn default() -> Self {
        TextStyle {
            font_size: 13.0,
            line_height: 18.0,
        }
    }
}

/// 文本度量接口。真实字体度量属于**后端**（GPU 侧要字形图集），
/// 所以这里抽象出来，默认给一个确定性近似实现 —— 让布局能在无 GPU 环境下断言。
pub trait Measure {
    fn width(&self, text: &str, style: TextStyle) -> f32;
    fn height(&self, text: &str, style: TextStyle, max_width: f32) -> f32;

    /// **换行点**（多行文本）：把 `text` 按 `max_width` 切成若干行。
    ///
    /// 为什么换行必须住在 `Measure` 里（而不是渲染器自己再写一套）：布局用
    /// 「行数 × 行高」预留高度、渲染器按「行数」发绘制命令 —— 两者各算各的就会出现
    /// 「预留 2 行、画出来 3 行」这种**节点高度装不下自己内容**的静默错误。
    /// 所以行数只有一处定义，`height()` 必须与它一致（两个实现都有一致性测试）。
    ///
    /// **默认实现是「不换行」**（一行、原样返回）：实现者不覆盖它就不会有多行绘制 ——
    /// 这是刻意的保守默认（「不假装能做」），不是遗漏。
    ///
    /// 约定（两个实现都遵守）：`max_width <= 0` 或非有限 ⇒ 不换行；空串 ⇒ 一行空串。
    fn wrap(&self, text: &str, _style: TextStyle, _max_width: f32) -> Vec<String> {
        vec![text.to_string()]
    }
}

/// **贪心的按词换行**（`wrap` 的唯一算法；「宽度怎么算」由调用方注入）。
///
/// 规则（确定性、有测试）：
/// - 按**空格 / 制表**切词（其它空白不切，避免悄悄改变文本）；
/// - 贪心塞进当前行，**行首不留空白**（切词时空白本身就被丢弃）；
/// - 单词自身宽度 > `max_width` 时**按字符硬切**（一个字符就超宽时该行允许超宽 ——
///   唯一例外，否则会死循环）；
/// - `max_width <= 0` 或非有限 ⇒ 不换行，返回 `vec![text.to_string()]`；
/// - 空串 / 全空白 ⇒ `vec![String::new()]`（**算 1 行**）。
///
/// 为什么注入的是闭包而不是 `&dyn Measure`：两个实现（`ApproxMeasure` 的近似宽度、
/// `FontMeasure` 的真实 advance）只差「宽度怎么算」，词切分与硬切规则**必须逐字相同** ——
/// 否则「近似度量下能换行、真实字体下换不了」这类分叉没人能一眼看出来。
pub fn wrap_greedy(text: &str, max_width: f32, width_of: impl Fn(&str) -> f32) -> Vec<String> {
    // `is_finite()` 顺手把 `NaN` 也挡掉（NaN 的所有比较都是 false ⇒ 会走进按字符硬切）。
    if !max_width.is_finite() || max_width <= 0.0 {
        return vec![text.to_string()];
    }

    let mut lines: Vec<String> = Vec::new();
    let mut cur = String::new();

    for word in text.split([' ', '\t']).filter(|w| !w.is_empty()) {
        let candidate = if cur.is_empty() {
            word.to_string()
        } else {
            format!("{cur} {word}")
        };
        if width_of(&candidate) <= max_width {
            cur = candidate;
            continue;
        }
        if !cur.is_empty() {
            lines.push(std::mem::take(&mut cur));
        }
        if width_of(word) <= max_width {
            cur = word.to_string();
        } else {
            let mut piece = String::new();
            for ch in word.chars() {
                let mut trial = String::with_capacity(piece.len() + 4);
                trial.push_str(&piece);
                trial.push(ch);
                if !piece.is_empty() && width_of(&trial) > max_width {
                    lines.push(std::mem::take(&mut piece));
                }
                piece.push(ch);
            }
            cur = piece;
        }
    }

    if !cur.is_empty() {
        lines.push(cur);
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

/// 确定性近似度量：每字符 0.6em。
///
/// **不是**真实字体度量 —— 它的价值是「同样的输入永远给同样的输出」，
/// 让布局的断言不受字体环境影响。真实度量由后端注入（接口相同）。
#[derive(Debug, Default, Clone, Copy)]
pub struct ApproxMeasure;

impl Measure for ApproxMeasure {
    fn width(&self, text: &str, style: TextStyle) -> f32 {
        let chars = text.chars().count() as f32;
        (chars * style.font_size * 0.6).ceil()
    }

    /// `wrap().len() * line_height` —— **不是**另起一套算法。
    ///
    /// 历史：改前这里是 `ceil(width / max_width) * line_height`（宽度比例模型）。
    /// 那个模型与 `wrap` 的按词换行**必然对不上**（同一段文本 4 行 vs 5 行），
    /// 于是「布局预留的高度」与「渲染器画出来的行数」会分叉。现在两者同源。
    fn height(&self, text: &str, style: TextStyle, max_width: f32) -> f32 {
        self.wrap(text, style, max_width).len() as f32 * style.line_height
    }

    /// 与 `deer_text::measure::FontMeasure::wrap` 同一套词切分规则，词宽用本度量的近似宽度。
    fn wrap(&self, text: &str, style: TextStyle, max_width: f32) -> Vec<String> {
        wrap_greedy(text, max_width, |s| self.width(s, style))
    }
}

/// 非文本控件的固有尺寸。
pub mod metrics {
    pub const BUTTON_PAD_X: f32 = 10.0;
    pub const BUTTON_MIN_W: f32 = 28.0;
    pub const BUTTON_MIN_H: f32 = 22.0;
    pub const FIELD_MIN_W: f32 = 60.0;
    pub const FIELD_H: f32 = 22.0;
}

fn resolve(size: Option<Size>, parent: f32) -> Option<f32> {
    match size? {
        Size::Px(v) => Some(v),
        Size::Pct(p) => Some(parent * p / 100.0),
    }
}

/// 测量阶段的中间量：每节点在「无限约束」下的固有尺寸。
pub type Intrinsics = HashMap<String, (f32, f32)>;

pub fn measure_tree(root: &Node, style: TextStyle, m: &impl Measure) -> Intrinsics {
    let mut out = Intrinsics::new();
    measure_into(root, style, m, &mut out);
    out
}

fn measure_into(n: &Node, style: TextStyle, m: &impl Measure, out: &mut Intrinsics) -> (f32, f32) {
    let (mut w, mut h) = match n.kind {
        Kind::Column | Kind::Row => {
            let pad = n.layout.padding;
            let gap = n.layout.gap;
            // 先无条件测子节点（**不做任何覆盖** —— 这一点很关键，见下）。
            let raw: Vec<(f32, f32)> = n.children.iter().map(|c| measure_into(c, style, m, out)).collect();
            // **只把「显式像素」尺寸计入容器的固有尺寸，且只在主轴方向。**
            //
            // 为什么不能在交叉轴也计入：容器的交叉轴固有尺寸是**子节点的最大值**，
            // 而主轴才是**子节点之和**。若把显式宽度也加进 Row 的宽度，
            // 两个各声明 `w=36` 的按钮会让行算成 72 宽 —— 而行的宽度本应由
            // 它自己的宽度约束（或父容器）决定。这是本项目踩过的第二个布局语义坑：
            // 「容器的交叉轴尺寸 = max(子)，主轴尺寸 = sum(子)」。
            //
            // 百分比不在此解析（需要父的实际宽度），留给排布阶段。
            let main: Vec<f32> = n
                .children
                .iter()
                .zip(&raw)
                .map(|(c, (cw, ch))| {
                    let explicit = if n.kind == Kind::Row { c.layout.width } else { c.layout.height };
                    match explicit {
                        Some(Size::Px(v)) => v.max(0.0),
                        _ => {
                            if n.kind == Kind::Row {
                                *cw
                            } else {
                                *ch
                            }
                        }
                    }
                })
                .collect();
            let count = n.children.len();
            let gaps = if count > 0 { gap * (count - 1) as f32 } else { 0.0 };
            if n.kind == Kind::Row {
                let inner: f32 = main.iter().sum::<f32>() + gaps;
                let max_h = raw.iter().map(|(_, h)| *h).fold(0.0_f32, f32::max);
                (inner + pad * 2.0, max_h + pad * 2.0)
            } else {
                let inner: f32 = main.iter().sum::<f32>() + gaps;
                let max_w = raw.iter().map(|(w, _)| *w).fold(0.0_f32, f32::max);
                (max_w + pad * 2.0, inner + pad * 2.0)
            }
        }
        Kind::Text => {
            let label = n.props.label.as_deref().unwrap_or("");
            let w = m.width(label, style);
            // `wrap` 节点的固有高 = **实际行数** × 行高。换行宽度取节点自己声明的像素宽度 ——
            // 这是唯一能在测量阶段知道的宽度（百分比 / 父容器分配都要到排布阶段才知道）。
            // 没声明像素宽度 ⇒ 节点宽 = 文本宽 ⇒ 换不了行 ⇒ 1 行（被钉住的边界，见指南）。
            let h = match n.layout.width {
                Some(Size::Px(px)) if n.layout.wrap && px > 0.0 => {
                    m.wrap(label, style, px).len() as f32 * style.line_height
                }
                _ => m.height(label, style, f32::INFINITY),
            };
            (w, h)
        }
        Kind::Field => {
            let label_w = n
                .props
                .label
                .as_deref()
                .map(|l| m.width(l, style) + 6.0)
                .unwrap_or(0.0);
            (
                label_w + metrics::FIELD_MIN_W,
                metrics::FIELD_H.max(style.line_height),
            )
        }
        Kind::Button => {
            let label = n.props.label.as_deref().unwrap_or("");
            let label_w = m.width(label, style);
            (
                metrics::BUTTON_MIN_W.max(label_w + metrics::BUTTON_PAD_X * 2.0),
                metrics::BUTTON_MIN_H.max(style.line_height),
            )
        }
    };

    // 显式**像素**尺寸覆盖固有尺寸（百分比留给排布阶段）
    if let Some(Size::Px(v)) = n.layout.width {
        w = v;
    }
    if let Some(Size::Px(v)) = n.layout.height {
        h = v;
    }

    let result = (w.ceil(), h.ceil());
    out.insert(n.id.clone(), result);
    result
}

/// 几何表：nodeId -> Rect。用 `HashMap` 存，但**取用时按树的顺序**保证遍历确定。
pub type Geometry = HashMap<String, Rect>;

// ---------------------------------------------------------------------------
// 滚动容器（垂直）：偏移是布局的**输入**，上限是布局的**输出**
// ---------------------------------------------------------------------------

/// 可滚动容器的**滚动偏移**（整数像素 —— 几何本身就是整数像素，见 I-4）。
///
/// 为什么是独立类型而不是 `HashMap` 直传：偏移只能来自「状态」，不该与几何混在一个
/// 返回值里；而且整型让 `Eq` 成立（`UiState` 的 dirty 判据靠逐字段相等）。
///
/// **没登记的 id ⇒ 偏移 0**（不是「未知」）：布局永远给出确定结果。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScrollOffsets {
    offsets: std::collections::BTreeMap<String, i32>,
}

impl ScrollOffsets {
    pub fn new() -> ScrollOffsets {
        ScrollOffsets::default()
    }

    /// 链式登记一个偏移（测试与调用方构造用）。
    pub fn with(mut self, id: impl Into<String>, offset_px: i32) -> ScrollOffsets {
        self.set(id, offset_px);
        self
    }

    pub fn set(&mut self, id: impl Into<String>, offset_px: i32) {
        self.offsets.insert(id.into(), offset_px);
    }

    /// 该容器的偏移（没登记 ⇒ 0）。
    pub fn get(&self, id: &str) -> i32 {
        self.offsets.get(id).copied().unwrap_or(0)
    }

    pub fn len(&self) -> usize {
        self.offsets.len()
    }

    pub fn is_empty(&self) -> bool {
        self.offsets.is_empty()
    }

    pub fn ids(&self) -> impl Iterator<Item = &str> {
        self.offsets.keys().map(String::as_str)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, i32)> {
        self.offsets.iter().map(|(k, v)| (k.as_str(), *v))
    }
}

/// 布局产出的**每个可滚动容器的 `max_scroll`**（整数像素）。
///
/// `max_scroll = max(0, 内容高 − 视口高)`；内容高 = 子节点主轴尺寸之和 + 间隙 + 上下内边距。
///
/// **没登记的 id ⇒ 0**（fail-closed）：调用方忘了灌这份表，滚轮就什么都滚不动 ——
/// 而不是滚到一个「没有上限」的虚空里。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScrollMetrics {
    max: std::collections::BTreeMap<String, i32>,
}

impl ScrollMetrics {
    pub fn new() -> ScrollMetrics {
        ScrollMetrics::default()
    }

    /// 该容器的滚动上限（没登记 ⇒ 0）。
    pub fn max_of(&self, id: &str) -> i32 {
        self.max.get(id).copied().unwrap_or(0)
    }

    /// 把偏移夹进 `[0, max_of(id)]`。
    pub fn clamp(&self, id: &str, offset_px: i32) -> i32 {
        offset_px.clamp(0, self.max_of(id))
    }

    pub fn len(&self) -> usize {
        self.max.len()
    }

    pub fn is_empty(&self) -> bool {
        self.max.is_empty()
    }

    pub fn ids(&self) -> impl Iterator<Item = &str> {
        self.max.keys().map(String::as_str)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, i32)> {
        self.max.iter().map(|(k, v)| (k.as_str(), *v))
    }

    /// 布局内部用：登记一个可滚动容器的上限。
    pub(crate) fn insert(&mut self, id: String, max: i32) {
        self.max.insert(id, max);
    }
}

// ---------------------------------------------------------------------------
// 滚动条几何（T3.2 的第一块：绘制与命中**共用**的唯一来源）
// ---------------------------------------------------------------------------

/// 滚动条**轨道**的宽度（像素）。
///
/// **唯一来源**：绘制侧（`deer-gpu` 画轨道/滑块）与命中侧（`deer-gui` 判「按下点在滚动条上」）
/// 都读这一个常量。各写一份的话，「看得见的滚动条」与「点得到的滚动条」迟早错开 ——
/// 而那种错**不报错**，只是偶尔点不中。
pub const SCROLLBAR_W: f32 = 8.0;

/// 轨道与视口边缘的间距（像素）—— 让滚动条不贴着边框。
pub const SCROLLBAR_INSET: f32 = 2.0;

/// 滑块的**最小**高度（像素）：内容极长时滑块仍要看得见、点得中。
pub const SCROLLBAR_MIN_THUMB: f32 = 24.0;

/// 一个可滚动容器的滚动条几何。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScrollbarGeom {
    /// 轨道（视口右侧的一条竖带）。
    pub track: Rect,
    /// 滑块（在轨道内；位置由 `offset / max_scroll` 决定）。
    pub thumb: Rect,
}

/// 由「视口矩形 + 当前偏移 + 滚动上限」算滚动条几何。
///
/// ## 语义（三条，都有单测钉）
///
/// 1. **`max_scroll <= 0` ⇒ `None`** —— 内容没超出视口就不画：一条满格滑块是纯噪音，
///    而且它会盖住内容最右边 8 像素；
/// 2. **滑块高 = `max(SCROLLBAR_MIN_THUMB, 轨道高 × 视口高 / 内容高)`**，
///    其中 `内容高 = 视口高 + max_scroll` ⇒ 这就是「可见比例」，被下限与轨道高夹住；
/// 3. **滑块顶 = `offset / max_scroll × (轨道高 − 滑块高)`** ⇒ 到顶贴顶、到底贴底，
///    中间不会越界（`max_scroll > 0` 才走到这里，除数取不到 0）。
///
/// ## 为什么返回**浮点**矩形
///
/// 取整交给调用方：绘制要整数（转 `RectI`）、命中要浮点（判「按下的点落在滑块里」）。
/// 在这里取整会让两侧各取一次整、可能差 1 像素 ⇒ 边界上「看得见却点不中」。
/// 偏移本身是整数，所以 `frac` 是确定的 —— 同一组输入永远给出同一组几何。
pub fn scrollbar_geom(viewport: Rect, offset: i32, max_scroll: i32) -> Option<ScrollbarGeom> {
    if max_scroll <= 0 {
        return None;
    }
    let track_x = viewport.x + viewport.w - SCROLLBAR_INSET - SCROLLBAR_W;
    let track_y = viewport.y + SCROLLBAR_INSET;
    let track_h = (viewport.h - 2.0 * SCROLLBAR_INSET).max(1.0);
    let track = Rect {
        x: track_x,
        y: track_y,
        w: SCROLLBAR_W,
        h: track_h,
    };

    let viewport_h = viewport.h.max(1.0);
    let content_h = viewport_h + max_scroll as f32;
    let visible_ratio = track_h * viewport_h / content_h;
    let thumb_h = visible_ratio.max(SCROLLBAR_MIN_THUMB).min(track_h);
    // 夹过再算：偏移越界时几何不该跟着越界（越界偏移本身由 `clamp` 那一层挡）
    let frac = offset.clamp(0, max_scroll) as f32 / max_scroll as f32;
    let travel = (track_h - thumb_h).max(0.0);
    let thumb = Rect {
        x: track_x,
        y: track_y + frac * travel,
        w: SCROLLBAR_W,
        h: thumb_h,
    };
    Some(ScrollbarGeom { track, thumb })
}

/// 排布：把树算成几何表（**不滚动** —— 等价于所有偏移为 0）。
pub fn layout(root: &Node, box_: Rect, style: TextStyle, m: &impl Measure) -> Geometry {
    layout_with_scroll(root, box_, style, m, &ScrollOffsets::new()).0
}

/// 排布 **+ 滚动**：把树算成几何表，并回一份「每个可滚动容器的 `max_scroll`」。
///
/// - `offsets` 里的偏移**参与几何**（子节点整体位移 `-offset`），容器自身的矩形不受影响；
/// - 偏移被夹进 `[0, max_scroll]`（滚到边界不越界 —— 越界的偏移是**静默丢弃**的，
///   不会渗进几何）；
/// - 偏移为空 ⇒ 与 [`layout`] 逐字段相同（同一条代码路径，滚动只是加法）。
///
/// 「可滚动容器」的定义见 [`Node::is_scroll_container`]（`Column` + `layout.scroll`）。
pub fn layout_with_scroll(
    root: &Node,
    box_: Rect,
    style: TextStyle,
    m: &impl Measure,
    offsets: &ScrollOffsets,
) -> (Geometry, ScrollMetrics) {
    let intrinsic = measure_tree(root, style, m);
    let mut geo = Geometry::new();
    let mut metrics = ScrollMetrics::new();
    let root_intrinsic = intrinsic.get(&root.id).copied().unwrap_or((0.0, 0.0));
    // 根的「分配尺寸」= 它的固有尺寸（**不撑满盒子**）—— 除非根自己有显式尺寸。
    // 这是 I-5 的体现：宿主给的盒子是上限，不是命令。
    let mut ctx = PlaceCtx {
        intrinsic: &intrinsic,
        geo: &mut geo,
        offsets,
        metrics: &mut metrics,
    };
    ctx.place(
        root,
        box_,
        root_intrinsic,
        (box_.w.round().max(0.0), box_.h.round().max(0.0)),
        (box_.w.round().max(0.0), box_.h.round().max(0.0)),
    );
    (geo, metrics)
}

/// 排布的递归上下文。
///
/// 为什么要包成结构体：`style`/`m`/`intrinsic`/`geo` 在整个递归里是**不变的**，
/// 逐个当参数传会让每个调用点重复一串实参（clippy 的 `only_used_in_recursion`
/// 指出的正是这个异味：参数只在递归里传递、函数体根本不用）。
struct PlaceCtx<'a> {
    intrinsic: &'a Intrinsics,
    geo: &'a mut Geometry,
    /// 滚动偏移（布局的输入）。
    offsets: &'a ScrollOffsets,
    /// 滚动上限（布局的输出；每个可滚动容器一条，**含 max = 0 的**）。
    metrics: &'a mut ScrollMetrics,
}

impl PlaceCtx<'_> {
    /// `avail` = **百分比解析基准**（父的内容盒）；`bound` = **显式尺寸的上限**（I-6）。
    ///
    /// 两者平时是同一个值；只有「滚动容器的子节点」在**主轴**上不同：百分比仍然相对
    /// 视口解析（唯一已知的长度），而显式尺寸**不被视口夹取** —— 否则「内容高于视口」
    /// 这个前提自己就不成立（`h=300` 的子节点会被压成 100，永远滚不动）。
    fn place(
        &mut self,
        n: &Node,
        rect: Rect,
        assigned: (f32, f32),
        avail: (f32, f32),
        bound: (f32, f32),
    ) {
        let ex_w = resolve(n.layout.width, avail.0);
        let ex_h = resolve(n.layout.height, avail.1);

        // I-7：显式尺寸 > 父分配尺寸；两者都没有才回退固有尺寸。
        let mut w = ex_w.unwrap_or(assigned.0);
        let mut h = ex_h.unwrap_or(assigned.1);
        // I-6：显式尺寸也不许超出可用空间（滚动容器的主轴例外，见上面的 `bound` 说明）
        if ex_w.is_some() {
            w = w.min(bound.0);
        }
        if ex_h.is_some() {
            h = h.min(bound.1);
        }
        let w = w.max(0.0).round();
    let h = h.max(0.0).round();

    self.geo.insert(
        n.id.clone(),
        Rect::new(rect.x.round(), rect.y.round(), w, h),
    );

    if !n.is_container() || n.children.is_empty() {
        return;
    }

    let pad = n.layout.padding;
    let gap = n.layout.gap;
    let inner_w = (w - pad * 2.0).max(0.0);
    let inner_h = (h - pad * 2.0).max(0.0);
    let horizontal = n.kind == Kind::Row;
    let main_avail = if horizontal { inner_w } else { inner_h };
    let cross_avail = if horizontal { inner_h } else { inner_w };
    // 垂直滚动容器：主轴**不再按视口夹取**子节点（否则「内容高于视口」这个前提本身
    // 就不成立 —— 内容会被压进视口，永远不会溢出），`grow` 也随之失效（没有「剩余空间」
    // 可分：可滚动内容本来就不该被压缩，见指南的「滚动容器」一节）。
    let scrollable = n.is_scroll_container();

    // 主轴：先给每个子节点固有主轴尺寸（或显式），剩余按 grow 权重分。
    let fixed: Vec<f32> = n
        .children
        .iter()
        .map(|c| {
            let ownk = self.intrinsic.get(&c.id).copied().unwrap_or((0.0, 0.0));
            let explicit = if horizontal {
                resolve(c.layout.width, inner_w)
            } else {
                resolve(c.layout.height, inner_h)
            };
            let base = explicit.unwrap_or(if horizontal { ownk.0 } else { ownk.1 });
            if scrollable {
                base
            } else {
                base.min(main_avail)
            }
        })
        .collect();

    let total_gap = gap * (n.children.len().saturating_sub(1)) as f32;
    let used: f32 = fixed.iter().sum::<f32>() + total_gap;
    let remaining = (main_avail - used).max(0.0);
    let grow_sum: f32 = if scrollable {
        0.0
    } else {
        n.children.iter().map(|c| c.layout.grow).sum()
    };

    let mut main_sizes: Vec<f32> = n
        .children
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let g = c.layout.grow;
            if grow_sum > 0.0 && g > 0.0 {
                let extra = (remaining * g / grow_sum).floor();
                main_avail.min(fixed[i] + extra)
            } else {
                fixed[i]
            }
        })
        .collect();

    // 主轴对齐：把剩余空间分到起点/两端/终点；stretch 在主轴上 = 均分吃掉剩余。
    //
    // 关键：剩余空间必须在 `grow` 分配**之后**重新计算。早期实现用的是 `grow` 之前的
    // 剩余量，于是 `grow` 一旦生效，「剩余」就已经被吃光，而 center/end 会**静默失效**
    // （布局仍有值，只是不再居中/靠后）。
    let used_after_grow: f32 = main_sizes.iter().sum::<f32>() + total_gap;
    let remaining = (main_avail - used_after_grow).max(0.0);
    let main_align = n.layout.main_axis.unwrap_or(Align::Start);
    let mut main_offset = 0.0_f32;
    if remaining > 0.0 && grow_sum == 0.0 && !n.children.is_empty() {
        match main_align {
            Align::Center => main_offset = (remaining / 2.0).floor(),
            Align::End => main_offset = remaining,
            Align::Stretch => {
                let each = (remaining / n.children.len() as f32).floor();
                for s in main_sizes.iter_mut() {
                    *s = main_avail.min(*s + each);
                }
            }
            Align::Start => {}
        }
    }

    let cross_align = n.layout.cross_axis.unwrap_or(Align::Start);
    // 滚动：`max_scroll` 由**内容主轴尺寸之和 + 间隙 + 上下内边距**减去**视口**得到；
    // 偏移被夹进 `[0, max_scroll]`，然后子节点整体位移 `-offset`。
    //
    // 注意「容器自身的矩形不动」：`rect` 是几何表里这个节点自己的位置，滚动只改它**内部**
    // 内容的相对位置 —— 所以 `max_scroll` 与 `rect` 无关，只与 `h`（视口）有关。
    let offset = if scrollable {
        let content_main = main_sizes.iter().sum::<f32>() + total_gap;
        let max_scroll = (content_main + pad * 2.0 - h).max(0.0).ceil() as i32;
        self.metrics.insert(n.id.clone(), max_scroll);
        self.offsets.get(&n.id).clamp(0, max_scroll) as f32
    } else {
        0.0
    };
    let mut cursor = if horizontal { rect.x + pad } else { rect.y + pad } + main_offset - offset;

    for (i, c) in n.children.iter().enumerate() {
        let main_size = main_sizes[i];
        let ownc = self.intrinsic.get(&c.id).copied().unwrap_or((0.0, 0.0));

        // 交叉轴：显式 > 固有；stretch 吃满。
        let cross_explicit = if horizontal {
            resolve(c.layout.height, inner_h)
        } else {
            resolve(c.layout.width, inner_w)
        };
        let mut cross_size = cross_explicit.unwrap_or(if horizontal { ownc.1 } else { ownc.0 });
        if cross_align == Align::Stretch {
            cross_size = cross_avail;
        }
        let cross_size = cross_size.max(0.0).min(cross_avail).round();

        let cross_slack = cross_avail - cross_size;
        let mut cross_pos = if horizontal { rect.y + pad } else { rect.x + pad };
        match cross_align {
            Align::Center => cross_pos += (cross_slack / 2.0).floor(),
            Align::End => cross_pos += cross_slack,
            _ => {}
        }

        let child_rect = if horizontal {
            Rect::new(cursor, cross_pos, main_size, cross_size)
        } else {
            Rect::new(cross_pos, cursor, cross_size, main_size)
        };
        // I-7：把父**分配**的尺寸显式传下去。
        // 注意 `avail` 传的是**父的内容盒**（inner_w / inner_h），不是分配尺寸 ——
        // 百分比尺寸必须相对父内容盒解析。传分配尺寸会让 `50%` 变成「已分配空间的一半」，
        // 那是个静默错误：布局仍然「有值」，只是值错了。
        let assigned = if horizontal {
            (main_size, cross_size)
        } else {
            (cross_size, main_size)
        };
        // 滚动容器的子节点：**主轴**的显式尺寸上限不再等于视口（内容空间是没有上界的），
        // 交叉轴仍然夹到内容盒（宽度不该超出视口）—— 这是「内容高于视口」能成立的前提。
        let child_bound = if scrollable {
            if horizontal {
                (f32::INFINITY, inner_h)
            } else {
                (inner_w, f32::INFINITY)
            }
        } else {
            (inner_w, inner_h)
        };

        self.place(c, child_rect, assigned, (inner_w, inner_h), child_bound);
        cursor += main_size + gap;
    }
}
}

/// 命中测试：在几何表上找**最深**命中节点。
///
/// 这是输入路由的唯一依据 —— 与渲染后端无关（DOM/GPU 后端共用）。
pub fn hit_test<'a>(root: &'a Node, geo: &Geometry, px: f32, py: f32) -> Option<&'a Node> {
    fn probe<'a>(n: &'a Node, geo: &Geometry, px: f32, py: f32, found: &mut Option<&'a Node>) {
        let Some(f) = geo.get(&n.id) else { return };
        if px < f.x || py < f.y || px >= f.x + f.w || py >= f.y + f.h {
            return;
        }
        *found = Some(n); // 后序覆盖 ⇒ 最深命中者胜出
        for c in &n.children {
            probe(c, geo, px, py, found);
        }
    }
    let mut found = None;
    probe(root, geo, px, py, &mut found);
    found
}

/// **反解**：指针在 `pointer_y`、且抓取点距滑块顶 `grab_dy` 像素时，偏移该是多少。
///
/// 与 [`scrollbar_geom`] 是**同一套映射的两个方向**，所以放在一起 ——
/// 各写一份的话「拖到哪儿对应哪个偏移」迟早对不上，而那种错**只在拖动时**能看出来。
///
/// 三条语义：
/// 1. 滑块高度只由视口与上限决定（与偏移无关）⇒ 这里先用 `offset = 0` 求一次几何拿到它；
/// 2. 指针位置换算成「滑块顶应该在哪儿」，再沿轨道长度归一化 ⇒ 映射回 `[0, max_scroll]`；
/// 3. 结果**夹在 `[0, max_scroll]`**（拖出轨道两端不该越界）。
pub fn scrollbar_offset_for_pointer(
    viewport: Rect,
    max_scroll: i32,
    pointer_y: f32,
    grab_dy: f32,
) -> i32 {
    if max_scroll <= 0 {
        return 0;
    }
    let Some(g) = scrollbar_geom(viewport, 0, max_scroll) else {
        return 0;
    };
    let travel = g.track.h - g.thumb.h;
    if travel <= 0.0 {
        return 0; // 滑块占满轨道 ⇒ 没有可移动的余量
    }
    let want_thumb_top = pointer_y - grab_dy;
    let frac = ((want_thumb_top - g.track.y) / travel).clamp(0.0, 1.0);
    (frac * max_scroll as f32).round() as i32
}

#[cfg(test)]
mod scrollbar_tests {
    use super::*;

    fn viewport() -> Rect {
        Rect {
            x: 10.0,
            y: 20.0,
            w: 100.0,
            h: 200.0,
        }
    }

    /// ① 内容没超出 ⇒ 不画（`max_scroll == 0` 与负数都算）。
    #[test]
    fn no_scrollbar_when_content_fits() {
        assert!(scrollbar_geom(viewport(), 0, 0).is_none(), "max_scroll=0 不该画");
        assert!(scrollbar_geom(viewport(), 0, -5).is_none(), "负上限不该画");
    }

    /// ② 到顶贴顶、到底贴底（`thumb.y` 的两个端点）。
    #[test]
    fn thumb_is_pinned_to_both_ends() {
        let v = viewport();
        let max = 400;
        let top = scrollbar_geom(v, 0, max).expect("要画").thumb;
        assert!(
            (top.y - (v.y + SCROLLBAR_INSET)).abs() < 0.01,
            "偏移 0 时滑块该贴轨道顶：{top:?}"
        );

        let bottom = scrollbar_geom(v, max, max).expect("要画");
        let track_bottom = bottom.track.y + bottom.track.h;
        let thumb_bottom = bottom.thumb.y + bottom.thumb.h;
        assert!(
            (thumb_bottom - track_bottom).abs() < 0.01,
            "到底时滑块该贴轨道底（滑块底 {thumb_bottom} vs 轨道底 {track_bottom}）"
        );
    }

    /// ③ 位移单调：偏移增大，滑块**不会**往上跑。
    #[test]
    fn thumb_moves_monotonically_with_offset() {
        let v = viewport();
        let max = 400;
        let mut prev = f32::NEG_INFINITY;
        for off in [0, 1, 50, 100, 200, 399, 400] {
            let y = scrollbar_geom(v, off, max).expect("要画").thumb.y;
            assert!(y >= prev, "偏移 {off} 时滑块往上跑了（{y} < {prev}）");
            prev = y;
        }
    }

    /// ④ **滑块永远在轨道内** —— 含越界偏移与极端内容长度（这是「不越界」的判据）。
    #[test]
    fn thumb_never_escapes_the_track() {
        let v = viewport();
        for max in [1, 2, 50, 400, 100_000] {
            for off in [-100, -1, 0, 1, max / 2, max - 1, max, max + 1, max * 3] {
                let g = scrollbar_geom(v, off, max).expect("要画");
                assert!(
                    g.thumb.y >= g.track.y - 0.01,
                    "max={max} off={off}: 滑块顶越过轨道顶 {:?} vs {:?}",
                    g.thumb,
                    g.track
                );
                assert!(
                    g.thumb.y + g.thumb.h <= g.track.y + g.track.h + 0.01,
                    "max={max} off={off}: 滑块底越过轨道底 {:?} vs {:?}",
                    g.thumb,
                    g.track
                );
                assert!(g.thumb.h >= 0.0 && g.thumb.h <= g.track.h + 0.01);
            }
        }
    }

    /// ⑤ 内容越长滑块越矮；但**不低于下限**（否则极端内容下会矮成一条看不见的线）。
    #[test]
    fn thumb_shrinks_with_content_but_honors_the_minimum() {
        let v = viewport();
        let short = scrollbar_geom(v, 0, 10).expect("要画").thumb.h;
        let long = scrollbar_geom(v, 0, 10_000).expect("要画").thumb.h;
        assert!(long < short, "内容更长时滑块该更矮（{long} vs {short}）");
        assert!(
            (long - SCROLLBAR_MIN_THUMB).abs() < 0.01,
            "极长内容下滑块该停在最小高度 {SCROLLBAR_MIN_THUMB}，实际 {long}"
        );
    }

    /// ⑥ 轨道贴在视口**右**边且不越出视口（右边界的容纳关系，不是「大概在那个位置」）。
    #[test]
    fn track_sits_inside_the_right_edge() {
        let v = viewport();
        let t = scrollbar_geom(v, 0, 100).expect("要画").track;
        assert!(
            (t.x + t.w - (v.x + v.w - SCROLLBAR_INSET)).abs() < 0.01,
            "轨道右边该离视口右边 {SCROLLBAR_INSET} 像素：{t:?} vs {v:?}"
        );
        assert!(t.x >= v.x, "轨道不能跑到视口左边去：{t:?}");
        assert!(
            t.y >= v.y && t.y + t.h <= v.y + v.h + 0.01,
            "轨道不能超出视口上下：{t:?} vs {v:?}"
        );
    }

    /// ⑦ 同一组输入 ⇒ 逐位相同的输出（滚动条几何也必须是**确定性纯函数**）。
    #[test]
    fn geometry_is_deterministic() {
        let v = viewport();
        let a = scrollbar_geom(v, 137, 999).expect("要画");
        let b = scrollbar_geom(v, 137, 999).expect("要画");
        assert_eq!(a, b);
    }

    /// ⑧ **往返一致**：偏移 → 几何 → 反解 ⇒ 回到同一个偏移。
    ///
    /// 这条是「正解与反解是同一套映射」的判据 —— 只测单方向的话，两边各错一点也能过。
    #[test]
    fn pointer_inverse_round_trips() {
        let v = viewport();
        let max = 400;
        for off in [0, 7, 100, 199, 200, 399, 400] {
            let g = scrollbar_geom(v, off, max).expect("要画");
            // 抓在滑块正中
            let grab = g.thumb.h / 2.0;
            let pointer_y = g.thumb.y + grab;
            let back = scrollbar_offset_for_pointer(v, max, pointer_y, grab);
            // 浮点往返允许 1 像素误差（偏移是整数、轨道是浮点）
            assert!(
                (back - off).abs() <= 1,
                "偏移 {off} 往返后变成 {back}（几何 {g:?}）"
            );
        }
    }

    /// ⑨ 拖出轨道两端 ⇒ 夹在 `[0, max_scroll]`，不越界。
    #[test]
    fn pointer_inverse_clamps_to_bounds() {
        let v = viewport();
        let max = 400;
        let grab = 10.0;
        // 指针远在轨道上方 / 下方
        assert_eq!(scrollbar_offset_for_pointer(v, max, -10_000.0, grab), 0);
        assert_eq!(scrollbar_offset_for_pointer(v, max, 10_000.0, grab), max);
        // 上限为 0 ⇒ 永远是 0（连几何都不该求）
        assert_eq!(scrollbar_offset_for_pointer(v, 0, 123.0, 0.0), 0);
    }

    /// ⑩ 单调：指针越往下，反解出的偏移越大（否则拖动方向会反）。
    #[test]
    fn pointer_inverse_is_monotonic() {
        let v = viewport();
        let max = 400;
        let mut prev = -1;
        for py in [20.0, 60.0, 100.0, 140.0, 180.0, 220.0] {
            let off = scrollbar_offset_for_pointer(v, max, py, 5.0);
            assert!(off >= prev, "指针 y={py} 时偏移 {off} 比上一个小（{prev}）");
            prev = off;
        }
    }
}
