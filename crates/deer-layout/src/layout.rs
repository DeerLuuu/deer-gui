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

    fn height(&self, text: &str, style: TextStyle, max_width: f32) -> f32 {
        let w = self.width(text, style);
        if max_width <= 0.0 {
            return style.line_height;
        }
        let lines = (w / max_width).ceil().max(1.0);
        lines * style.line_height
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
            (m.width(label, style), m.height(label, style, f32::INFINITY))
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

/// 排布：把树算成几何表。
pub fn layout(root: &Node, box_: Rect, style: TextStyle, m: &impl Measure) -> Geometry {
    let intrinsic = measure_tree(root, style, m);
    let mut geo = Geometry::new();
    let root_intrinsic = intrinsic.get(&root.id).copied().unwrap_or((0.0, 0.0));
    // 根的「分配尺寸」= 它的固有尺寸（**不撑满盒子**）—— 除非根自己有显式尺寸。
    // 这是 I-5 的体现：宿主给的盒子是上限，不是命令。
    let mut ctx = PlaceCtx {
        intrinsic: &intrinsic,
        geo: &mut geo,
    };
    ctx.place(
        root,
        box_,
        root_intrinsic,
        box_.w.round().max(0.0),
        box_.h.round().max(0.0),
    );
    geo
}

/// 排布的递归上下文。
///
/// 为什么要包成结构体：`style`/`m`/`intrinsic`/`geo` 在整个递归里是**不变的**，
/// 逐个当参数传会让每个调用点重复一串实参（clippy 的 `only_used_in_recursion`
/// 指出的正是这个异味：参数只在递归里传递、函数体根本不用）。
struct PlaceCtx<'a> {
    intrinsic: &'a Intrinsics,
    geo: &'a mut Geometry,
}

impl PlaceCtx<'_> {
    fn place(&mut self, n: &Node, rect: Rect, assigned: (f32, f32), avail_w: f32, avail_h: f32) {
        let ex_w = resolve(n.layout.width, avail_w);
        let ex_h = resolve(n.layout.height, avail_h);

        // I-7：显式尺寸 > 父分配尺寸；两者都没有才回退固有尺寸。
        let mut w = ex_w.unwrap_or(assigned.0);
        let mut h = ex_h.unwrap_or(assigned.1);
        // I-6：显式尺寸也不许超出可用空间
        if ex_w.is_some() {
            w = w.min(avail_w);
        }
        if ex_h.is_some() {
            h = h.min(avail_h);
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
            base.min(main_avail)
        })
        .collect();

    let total_gap = gap * (n.children.len().saturating_sub(1)) as f32;
    let used: f32 = fixed.iter().sum::<f32>() + total_gap;
    let remaining = (main_avail - used).max(0.0);
    let grow_sum: f32 = n.children.iter().map(|c| c.layout.grow).sum();

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
    let mut cursor = if horizontal { rect.x + pad } else { rect.y + pad } + main_offset;

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
        // 注意 `avail_*` 传的是**父的内容盒**（inner_w / inner_h），不是分配尺寸 ——
        // 百分比尺寸必须相对父内容盒解析。传分配尺寸会让 `50%` 变成「已分配空间的一半」，
        // 那是个静默错误：布局仍然「有值」，只是值错了。
        let assigned = if horizontal {
            (main_size, cross_size)
        } else {
            (cross_size, main_size)
        };

        self.place(c, child_rect, assigned, inner_w, inner_h);
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
