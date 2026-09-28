//! **状态感知**的绘制列表（M5-4）：把「交互状态」翻成**像素差异**。
//!
//! # 为什么必须有这一层
//!
//! `interaction::UiState`（`hover`/`focus`/`pressed`/`texts`）本身**不改变树**。
//! `DefaultRenderer::build(&tree, &geo)` 只吃「树 + 几何 + 主题」⇒ 状态怎么变，画出来
//! 都是同一帧。那样「像素判据」就没有落点：任何判据都会在「状态没生效」和
//! 「状态生效了但没画出来」之间二义。
//!
//! 所以这一层把状态**映射成确定的视觉差异**，并且这份映射本身是可断言的：
//!
//! | 状态 | 视觉（都发生在该节点**自己的矩形内**） |
//! |---|---|
//! | `hover`（未按下、未聚焦） | 底色**提亮** `LIGHTEN` 比例（`Button` 的 `FillRoundRect` / `Field` 的填充换成提亮色） |
//! | `pressed` | 底色**加深** `DARKEN` 比例 |
//! | `focus` | 沿矩形内侧画 `FOCUS_STROKE_WIDTH` 像素的描边（填色回到 idle）；**颜色按控件分**：`Field` 用 `theme.accent`（它的填充是 `border` 色 ⇒ 本就分得出），`Button` 用 `theme.on_accent` 且再内缩 `FOCUS_RING_INSET`（它的填充**就是** `accent`，同色描边等于没画 —— 见下面「焦点环」一节） |
//! | `texts` | `Field` 的文字换成**文本缓冲内容**（`FieldText::Content`；见下） |
//!
//! ## 三条刻意的选择（都有代价，写在这里免得被当缺陷）
//!
//! 1. **叠加色一律不透明**（`Color::rgb`，alpha = 1.0）。半透明会在 CPU（`round()` 字节混合）
//!    与 GPU（固定功能 UNORM 混合）之间引入 ≤1 LSB 的差 —— 那是**允许**的，不是缺陷，
//!    但会让「逐字节相同」这条最强的判据没法用在不透明语料上。提亮/加深**本来就不需要 alpha**。
//! 2. **优先序是确定的**（否则「同时 hover + focus」的帧会随实现抖动）：
//!    填充 = `pressed` > `hover` > idle；描边 = `focus` > idle。
//! 3. **禁用节点不画任何状态视觉**（`state_of` 直接返回 idle）。禁用子树不响应输入是
//!    `interaction` 层的判据；这里跟着做，是为了让「禁用节点上**一定**没有状态色」也能被
//!    像素判据咬住（否则一个错误的 `focus` 会被画出来而没人发现）。
//!
//! ## 焦点环：按钮的环**必须**换色（附改前的实测数字）
//!
//! 按钮（未 hover、未按下）的填充是 `tint(accent, Idle) == accent`，而焦点描边原来也用
//! `theme.accent` ⇒ **环与底色同色**。探针实测（`button_1` = 36×22 = 792 px，画布 200×120，
//! 黑底与 `#08090c` 底两种 clear 色下**完全一致**）：
//!
//! | 帧 | 与 idle 不同的像素 | 构成 |
//! |---|---|---|
//! | 改前 `focused` | **56 px**（= 144 字节） | 32 px 是**方角补角**（圆角被直角环补上）、其余 24 px 是**描边压过标签字形** |
//! | 改后 `focused` | **228 px**（= 456 字节） | 全在环带内（标签与环同色 ⇒ 重叠处**不**产生差异） |
//!
//! 也就是说：改前那 56 px **不是**「一条看得见的焦点环」，而是两个副产物 —— 而当时的判据是
//! `diff > 0`，所以它一直是绿的。两处改动（都只改**按钮自己的矩形内**的像素，
//! hover/pressed/idle 的像素一个都不动）：
//!
//! 1. **颜色**：环改用 `theme.on_accent`（= 该控件的内容色；与 `accent` 填充的通道平均差
//!    **97.7**）。不新增硬编码颜色；也不用半透明 —— 半透明会把 CPU/GPU 的逐字节对照
//!    拖进「≤1 LSB」那一档（见本文档第 1 条）。
//! 2. **内缩 `FOCUS_RING_INSET`**：描边命令只有**直角**矩形，而填充是**圆角**（半径 4）⇒
//!    贴着边画会把四个圆角补成方角（实测 32 px）。内缩 2 之后环的直角顶点落回圆角内
//!    （`√2·(4−2) ≈ 2.83 < 4`），于是按钮的**外轮廓一个像素都不变**。
//!
//! 判据同时从「差异 > 0」加严成三条：环带内差异像素数 ≥ 下限、环带内**平均通道差** ≥ 下限、
//! 且差异**一个都不许**落在环带之外。这条判据在同一份模块测试里**反向自检**：
//! 把环色改回填充色（以及改成一个「技术上不同、肉眼看不出」的近似色）⇒ 它必须拒绝
//! （`button_focus_ring_is_visible_against_the_fill`）。
//!
//! ## `FieldText`：文本内容要不要上屏
//!
//! `UiState::texts` 唯一改变几何的是「输入框显示什么」。但**显示缓冲内容会牵连文本命令的
//! 结构**（空缓冲要退回占位标签，否则用户看到的是一个空框 —— 与无状态时的表现不一致）。
//! 所以这里把这件事做成显式开关：
//!
//! - [`FieldText::Label`]：与 `DefaultRenderer` **同结构**（无状态渲染；示例的窗口路径
//!   用这条，它不需要文本引擎）。**只有 `Button`/`Field` 的文字命令刻意不同**
//!   （按钮的文字矩形夹在按钮内、输入框的文字矩形内缩 + 用 `theme.text` 而非 `text_dim`），
//!   差异由模块测试逐条钉住 —— 不写成「逐字节等价」，因为那与事实不符；
//! - [`FieldText::Content`]：有缓冲时显示缓冲内容、没有缓冲时退回标签（像素判据用这条，
//!   它把「状态真的改变了几何」变成可比较的字节）。
//!
//! ## `NodeHint`：这份列表是**裁剪感知命中**的前提
//!
//! `interaction::ClipSnapshot::from_draw_list` 的绑定协议要求「第 k 个 `NodeHint` = 前序遍历
//! 里第 k 个**有几何**的节点」。`DefaultRenderer` **不发** `NodeHint` ⇒ 快照恒为空 ⇒
//! 命中不受裁剪约束（而且**看不出来**）。所以本模块的骨架逐字照抄
//! `NullRenderer::build`：先给每个有几何的节点发提示（不因零面积而跳过），再画视觉。
//! 零面积节点只画描边（与 `DefaultRenderer` 的早退不同），并在模块测试里钉住这条差异。

use deer_layout::layout::{Geometry, Measure};
use deer_layout::node::{Kind, Node};

use crate::draw::{Color, DrawCmd, DrawList, RectI};
use crate::Theme;

/// 交互状态的视觉输入（`UiState` 的**只读子集**，避免 `deer-gpu` 反向依赖 `deer-gui`）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InteractState {
    pub hover: Option<String>,
    pub focus: Option<String>,
    pub pressed: Option<String>,
}

impl InteractState {
    pub fn hovered(id: &str) -> InteractState {
        InteractState {
            hover: Some(id.to_string()),
            ..Default::default()
        }
    }
    pub fn pressed(id: &str) -> InteractState {
        InteractState {
            pressed: Some(id.to_string()),
            ..Default::default()
        }
    }
    pub fn focused(id: &str) -> InteractState {
        InteractState {
            focus: Some(id.to_string()),
            ..Default::default()
        }
    }
}

/// `Field` 的文字来源。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FieldText {
    /// 恒显示 `props.label`（占位标签）。与 `DefaultRenderer` 同结构
    /// （只有文字矩形的内缩与颜色不同，见模块文档）。
    #[default]
    Label,
    /// 有缓冲（含空串）时显示缓冲内容；没有缓冲条目时退回 `props.label`。
    Content,
}

/// 提亮比例（`Color::lighten` 的参数；0 = 不变、1 = 纯白）。
pub const HOVER_LIGHTEN: f32 = 0.22;
/// 加深比例（`Color::darken` 的参数；0 = 不变、1 = 纯黑）。
pub const PRESSED_DARKEN: f32 = 0.30;
/// 焦点描边宽度（像素，**向内**画 ⇒ 不会跑到矩形之外）。
pub const FOCUS_STROKE_WIDTH: i32 = 3;
/// **按钮**焦点环相对按钮矩形再内缩的像素数（原因见模块文档「焦点环」一节）。
///
/// 按钮的填充是**圆角**（半径 4），而描边命令只有直角矩形：贴着边画时环的四个直角顶点会伸到
/// 圆角之外，把圆角「补成方角」。内缩 `n` 后环的直角顶点到圆角圆心的距离是 `√2·(4−n)`，
/// `n ≥ 4·(1−1/√2) ≈ 1.17` 时顶点落回圆角内 ⇒ 取整数下限 **2**。
/// 这条关系由模块测试显式断言（`FOCUS_RING_INSET` 相对填充半径的判别式）—— 谁把圆角改大
/// 而不动这个内缩，测试会红，而不是悄悄又出现方角补块。
pub const FOCUS_RING_INSET: i32 = 2;
/// 文本相对控件矩形的内缩（像素）：让文字与描边不重叠 —— 于是「文本变化」只发生在框内。
pub const TEXT_INSET: i32 = 2;

/// 一个节点的**状态视觉等级**。
///
/// 三档互斥，判定顺序**确定**：`pressed` > `hover`（填色），`focus` 单独管描边。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Visual {
    Idle,
    Hover,
    Pressed,
}

/// 把状态翻成像素的渲染器。
pub struct InteractiveRenderer<'a, M: Measure> {
    pub theme: Theme,
    pub measure: &'a M,
    pub state: &'a InteractState,
    pub field_text: FieldText,
    /// `Field` 的文本缓冲（`id → 内容`）；只有 `field_text == Content` 时被读。
    pub texts: &'a std::collections::BTreeMap<String, String>,
}

impl<'a, M: Measure> InteractiveRenderer<'a, M> {
    /// 便捷构造：只有状态、没有文本缓冲（`FieldText::Label`）。
    pub fn new(theme: Theme, measure: &'a M, state: &'a InteractState) -> Self {
        static EMPTY: std::sync::OnceLock<std::collections::BTreeMap<String, String>> =
            std::sync::OnceLock::new();
        InteractiveRenderer {
            theme,
            measure,
            state,
            field_text: FieldText::Label,
            texts: EMPTY.get_or_init(std::collections::BTreeMap::new),
        }
    }

    /// 带文本缓冲（一般配 [`FieldText::Content`]）。
    pub fn with_texts(
        theme: Theme,
        measure: &'a M,
        state: &'a InteractState,
        field_text: FieldText,
        texts: &'a std::collections::BTreeMap<String, String>,
    ) -> Self {
        InteractiveRenderer {
            theme,
            measure,
            state,
            field_text,
            texts,
        }
    }

    /// 树 + 几何 + 状态 → 绘制列表（每个有几何的节点**先**发一条 `NodeHint`）。
    pub fn build(&self, tree: &Node, geo: &Geometry) -> DrawList {
        let mut list = DrawList::new();
        self.walk(tree, geo, false, &mut list);
        list
    }

    /// 某个节点的状态视觉等级（禁用 ⇒ 恒 `Idle`；父禁用 ⇒ 子树恒 `Idle`）。
    fn visual_of(&self, n: &Node, dead: bool) -> Visual {
        if dead || n.props.disabled {
            return Visual::Idle;
        }
        if self.state.pressed.as_deref() == Some(n.id.as_str()) {
            Visual::Pressed
        } else if self.state.hover.as_deref() == Some(n.id.as_str()) {
            Visual::Hover
        } else {
            Visual::Idle
        }
    }

    fn is_focused(&self, n: &Node, dead: bool) -> bool {
        !dead && !n.props.disabled && self.state.focus.as_deref() == Some(n.id.as_str())
    }

    /// `Field` 这一帧显示的文本（见 [`FieldText`]）。
    fn field_label(&self, n: &Node) -> String {
        match self.field_text {
            FieldText::Label => n.props.label.clone().unwrap_or_default(),
            FieldText::Content => self
                .texts
                .get(&n.id)
                .cloned()
                .unwrap_or_else(|| n.props.label.clone().unwrap_or_default()),
        }
    }

    fn walk(&self, n: &Node, geo: &Geometry, dead: bool, list: &mut DrawList) {
        let Some(f) = geo.get(&n.id) else { return };
        let dead = dead || n.props.disabled;
        let visual = self.visual_of(n, dead);
        let focused = self.is_focused(n, dead);

        // ① 提示**先发**：与 `NullRenderer::build` 同序（每个有几何的节点一条，零面积也算）。
        list.push(DrawCmd::NodeHint {
            rect: RectI::new(f.x as i32, f.y as i32, f.w as i32, f.h as i32),
            node_id_len: n.id.len() as u32,
        });

        // ② 视觉。零面积只画描边（`DefaultRenderer` 是直接早退 ⇒ 两者在这条边界上不同，
        //    模块测试钉住了这一点）。
        let r = f.x as i32;
        let ry = f.y as i32;
        let rw = f.w as i32;
        let rh = f.h as i32;
        if rw <= 0 || rh <= 0 {
            if focused {
                // 零面积：什么都画不出来，但描边命令照样发（与 `DefaultRenderer` 的早退刻意
                // 不同，模块测试钉住）。颜色与按钮的焦点环**同一处来源**，免得「焦点是什么色」
                // 有两份答案；内缩**不**在这里做 —— 零面积矩形内缩会把命令挪出自己的矩形。
                list.push(DrawCmd::StrokeRect {
                    rect: RectI::new(r, ry, rw, rh),
                    color: self.theme.on_accent,
                    width: FOCUS_STROKE_WIDTH,
                });
            }
        } else {
            let rect = RectI::new(r, ry, rw, rh);
            match n.kind {
                Kind::Column | Kind::Row => {
                    // 只有显式给了内边距的容器才画底（与 `DefaultRenderer` 逐字相同）。
                    if n.layout.padding > 0.0 {
                        list.push(DrawCmd::FillRoundRect {
                            rect,
                            radius: 6,
                            color: self.theme.surface,
                        });
                        list.push(DrawCmd::StrokeRect {
                            rect,
                            color: self.theme.border,
                            width: 1,
                        });
                    }
                }
                Kind::Text => {
                    // 多行：每行一条命令（`wrap` 关 ⇒ 恰好一条、矩形不变 ⇒ 既有语料逐字节不变）。
                    // 换行点与布局预留高度同源（`Measure::wrap`），见 `render::text_lines`。
                    let color = if n.props.disabled {
                        self.theme.text_dim
                    } else {
                        self.theme.text
                    };
                    for (line_rect, line) in
                        crate::render::text_lines(self.measure, n, rect, self.text_style())
                    {
                        list.push(DrawCmd::Text {
                            rect: line_rect,
                            text: line,
                            color,
                            size: self.theme.font_size,
                            align: 0,
                        });
                    }
                }
                Kind::Button => {
                    let base = if n.props.disabled {
                        self.theme.border
                    } else {
                        self.theme.accent
                    };
                    list.push(DrawCmd::FillRoundRect {
                        rect,
                        radius: 4,
                        color: tint(base, visual),
                    });
                    let label = n.props.label.clone().unwrap_or_default();
                    let tw = self.measure.width(&label, self.text_style());
                    // 文字居中，但**矩形必须留在按钮内**（统一管线的裁剪与「命令不许越界」
                    // 这条不变式都建立在命令矩形之上；标签比按钮宽时**不**向外伸）。
                    let tx = (rect.x + ((rect.w as f32 - tw) / 2.0).floor() as i32).max(rect.x);
                    list.push(DrawCmd::Text {
                        rect: RectI::new(tx, rect.y, rect.right() - tx, rect.h),
                        text: label,
                        color: if n.props.disabled {
                            self.theme.text_dim
                        } else {
                            self.theme.on_accent
                        },
                        size: self.theme.font_size,
                        align: 0,
                    });
                    // ③ 焦点环：**对比色**（`theme.on_accent`）+ 内缩。按钮的填充就是
                    //    `theme.accent`，环再用 accent 等于没画（改前实测 idle vs focused
                    //    只差 56 px，且 32 px 还是方角补角）—— 见模块文档「焦点环」。
                    if focused {
                        list.push(DrawCmd::StrokeRect {
                            rect: focus_ring_rect(rect),
                            color: self.theme.on_accent,
                            width: FOCUS_STROKE_WIDTH,
                        });
                    }
                }
                Kind::Field => {
                    // 填充：pressed/hover 改色；描边：focus 改成强调色（填充**回到 idle 色**）。
                    let fill = if focused {
                        self.theme.border
                    } else {
                        tint(self.theme.border, visual)
                    };
                    list.push(DrawCmd::FillRoundRect {
                        rect,
                        radius: 4,
                        color: fill,
                    });
                    let border = if focused {
                        self.theme.accent
                    } else {
                        self.theme.text_dim
                    };
                    list.push(DrawCmd::StrokeRect {
                        rect,
                        color: border,
                        width: if focused { FOCUS_STROKE_WIDTH } else { 1 },
                    });
                    // 文本**内缩**：文字与描边不重叠 ⇒「文本变化」只发生在框内（判据更强）。
                    list.push(DrawCmd::Text {
                        rect: inset(rect, TEXT_INSET),
                        text: self.field_label(n),
                        color: self.theme.text,
                        size: self.theme.font_size,
                        align: 0,
                    });
                }
            }
        }

        // 滚动容器：视口裁剪把子节点（含它们的 `NodeHint`）包在里面 ——
        // `ClipSnapshot::from_draw_list` 于是把「视口矩形」绑成子节点的有效裁剪，
        // **视口外的点自动不命中**（与既有「裁剪外的点不命中」同一条语义，没有第二套规则）。
        // 容器自己的提示已经发过了（在裁剪**之前**）⇒ 容器自身不被自己的视口裁掉。
        let clip = n.is_scroll_container();
        if clip {
            list.push(DrawCmd::PushClip {
                rect: RectI::new(r, ry, rw, rh),
            });
        }

        for c in &n.children {
            self.walk(c, geo, dead, list);
        }

        if clip {
            list.push(DrawCmd::PopClip);
        }
    }

    fn text_style(&self) -> deer_layout::TextStyle {
        deer_layout::TextStyle {
            font_size: self.theme.font_size,
            line_height: self.theme.line_height,
        }
    }
}

/// 便捷函数：树 + 几何 + 状态 → 绘制列表（`FieldText::Label`）。
pub fn build_interactive_draw_list<M: Measure>(
    tree: &Node,
    geo: &Geometry,
    theme: Theme,
    measure: &M,
    state: &InteractState,
) -> DrawList {
    InteractiveRenderer::new(theme, measure, state).build(tree, geo)
}

/// 便捷函数：带文本缓冲的版本（像素判据用）。
pub fn build_interactive_draw_list_with_texts<M: Measure>(
    tree: &Node,
    geo: &Geometry,
    theme: Theme,
    measure: &M,
    state: &InteractState,
    field_text: FieldText,
    texts: &std::collections::BTreeMap<String, String>,
) -> DrawList {
    InteractiveRenderer::with_texts(theme, measure, state, field_text, texts).build(tree, geo)
}

/// 状态提亮/加深（**不透明**，见模块文档第 1 条）。
fn tint(base: Color, visual: Visual) -> Color {
    match visual {
        Visual::Idle => base,
        Visual::Hover => base.lighten(HOVER_LIGHTEN),
        Visual::Pressed => base.darken(PRESSED_DARKEN),
    }
}

/// 向内缩 `n` 像素（宽高至少留 1；缩没了就退化成 1×1 —— 文本命令不接受空矩形）。
fn inset(r: RectI, n: i32) -> RectI {
    RectI::new(
        r.x + n,
        r.y + n,
        (r.w - 2 * n).max(1),
        (r.h - 2 * n).max(1),
    )
}

/// 按钮焦点环的矩形：内缩 [`FOCUS_RING_INSET`]，**但绝不越出原矩形**。
///
/// `inset` 的 `max(1)` 会在「矩形小到缩不动」时把矩形推到原矩形之外（1×1 的按钮内缩 2 ⇒
/// 命令落到 (x+2, y+2)）—— 那会破坏「命令矩形必须落在自己节点矩形内」这条不变式。
/// 所以先把内缩量夹到 `(边长-1)/2`（半径 4 的圆角按钮在正常尺寸下取满 2，退化成 0 也安全）。
fn focus_ring_rect(r: RectI) -> RectI {
    let n = FOCUS_RING_INSET
        .min((r.w - 1).max(0) / 2)
        .min((r.h - 1).max(0) / 2);
    inset(r, n)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::null::CpuRenderer;
    use crate::render::{DefaultRenderer, NullRenderer, build_draw_list};
    use crate::{Extent, Theme};
    use deer_layout::layout::{ApproxMeasure, layout};
    use deer_layout::node::Rect;

    fn tree() -> Node {
        let mut b = deer_layout::builder::Builder::new(Kind::Column, "app")
            .padding(0.0)
            .gap(0.0);
        b.button("OK");
        b.field("name");
        b.build()
    }

    fn geo() -> Geometry {
        layout(
            &tree(),
            Rect::new(0.0, 0.0, 200.0, 120.0),
            deer_layout::TextStyle::default(),
            &ApproxMeasure,
        )
    }

    /// 前置：这份列表必须**真的**发得出裁剪快照所需的提示（否则裁剪感知命中静默失效）。
    #[test]
    fn hints_cover_every_node_with_geometry() {
        let t = tree();
        let g = geo();
        let s = InteractState::default();
        let list = InteractiveRenderer::new(Theme::default(), &ApproxMeasure, &s).build(&t, &g);
        let hints = list.counts().node_hint;
        let nodes_with_geo = ["app", "button_1", "field_1"]
            .iter()
            .filter(|id| g.contains_key(**id))
            .count();
        println!("node_hint={hints} 有几何的节点={nodes_with_geo}");
        assert_eq!(hints, nodes_with_geo, "每个有几何的节点都要有一条提示");
        // 与 `NullRenderer` 的提示条数必须一致（那就是绑定协议的期望值）。
        assert_eq!(hints, NullRenderer::build(&t, &g).counts().node_hint);
    }

    /// `FieldText::Label` + 无状态 ⇒ 与 `DefaultRenderer` **同结构**，只有**三处刻意**不同。
    ///
    /// 这条是「示例的窗口路径可以继续用 `DefaultRenderer` 的 baseline 视觉」的依据。
    /// 刻意不同的三处（**都在控件自己的矩形内**，所以不影响「状态差异只在框内」）：
    ///
    /// 1. `Button` 的**文字矩形**被夹在按钮内（`DefaultRenderer` 用 `rx+居中偏移` 当起点、
    ///    宽度仍取整个按钮 ⇒ 文字比按钮宽时**伸到按钮外**）。统一管线的裁剪与「命令不许越界」
    ///    这条不变式都建立在命令矩形上，所以这里必须夹住；
    /// 2. `Field` 的**文字矩形内缩** `TEXT_INSET`（`DefaultRenderer` 用整个框 ⇒ 文字与 1px 描边重叠）；
    /// 3. `Field` 的**文字颜色**用 `theme.text`（`DefaultRenderer` 用 `theme.text_dim` ⇒ 与占位
    ///    标签同色，真输入内容的可读性差）。
    ///
    /// 这里**显式钉住**这三处：谁把它们改回等价（或改成第四样）都会红，
    /// 而不是让「差不多等价」这种模糊说法留在注释里。
    #[test]
    fn label_mode_without_state_matches_default_renderer_except_pinned_field_text() {
        let t = tree();
        let g = geo();
        let theme = Theme::default();
        let s = InteractState::default();
        let mine = InteractiveRenderer::new(theme.clone(), &ApproxMeasure, &s).build(&t, &g);
        let base = build_draw_list(&t, &g, theme.clone(), &ApproxMeasure);
        let mine_wo_hints: Vec<DrawCmd> = mine
            .cmds
            .iter()
            .filter(|c| !matches!(c, DrawCmd::NodeHint { .. }))
            .cloned()
            .collect();
        assert_eq!(
            mine_wo_hints.len(),
            base.cmds.len(),
            "去掉 NodeHint 之后命令条数必须相同"
        );

        let mut differing = Vec::new();
        for (a, b) in mine_wo_hints.iter().zip(base.cmds.iter()) {
            if a != b {
                differing.push((a.clone(), b.clone()));
            }
        }
        println!("与 DefaultRenderer 不同的命令数 = {}", differing.len());
        for (a, b) in &differing {
            println!("  本模块: {a:?}\n  默认  : {b:?}");
        }
        assert_eq!(differing.len(), 2, "只允许两个 Text 命令不同（见本测试的文档）");

        // 用「命令所属节点的矩形」判定这条差异是按钮还是输入框（**不靠宽度猜**——
        // 输入框的文字也变窄了，靠宽度分不出谁是谁）。
        let owned_by = |rect: RectI| -> String {
            let mut owner: Option<RectI> = None;
            for cmd in &mine.cmds {
                match cmd {
                    DrawCmd::NodeHint { rect, .. } => owner = Some(*rect),
                    DrawCmd::Text { rect: cmd_rect, .. } if *cmd_rect == rect => {
                        let o = owner.expect("提示必须先于绘制命令");
                        return g
                            .iter()
                            .find(|(_, r)| {
                                RectI::new(r.x as i32, r.y as i32, r.w as i32, r.h as i32) == o
                            })
                            .map(|(id, _)| id.clone())
                            .unwrap_or_else(|| format!("<未知 {o:?}>"));
                    }
                    _ => {}
                }
            }
            "<没找到>".to_string()
        };

        let mut seen_button = false;
        let mut seen_field = false;
        for (a, b) in &differing {
            let (
                DrawCmd::Text {
                    rect: r_mine,
                    color: c_mine,
                    ..
                },
                DrawCmd::Text { rect: r_base, .. },
            ) = (a, b)
            else {
                panic!("不同的命令必须是 Text，实际 {a:?} / {b:?}");
            };
            let owner = owned_by(*r_mine);
            println!("差异命令属于节点 `{owner}`：本模块 {r_mine:?}／默认 {r_base:?}");
            match owner.as_str() {
                "button_1" => {
                    // ① 按钮：宽度被夹住 ⇒ 右边界＝按钮右边界（左边界仍是居中偏移）。
                    seen_button = true;
                    let btn = *g.get("button_1").expect("测试前置：button_1 有几何");
                    let frame =
                        RectI::new(btn.x as i32, btn.y as i32, btn.w as i32, btn.h as i32);
                    assert_eq!(r_mine.x, r_base.x, "左边界（居中偏移）不该变");
                    assert_eq!(
                        r_mine.right(),
                        frame.right(),
                        "夹住之后右边界必须恰好是按钮的右边界"
                    );
                    assert!(
                        r_base.right() > frame.right(),
                        "默认渲染器在这里**伸出了**按钮（这正是本模块要夹住的那件事）"
                    );
                }
                "field_1" => {
                    seen_field = true;
                    let field = *g.get("field_1").expect("测试前置：field_1 有几何");
                    let frame =
                        RectI::new(field.x as i32, field.y as i32, field.w as i32, field.h as i32);
                    assert_eq!(r_mine.x, frame.x + TEXT_INSET, "输入框文字矩形内缩 TEXT_INSET");
                    assert_eq!(*c_mine, theme.text, "输入框文字用 theme.text");
                    assert_eq!(*r_base, frame, "默认渲染器用整个框");
                    assert!(
                        r_mine.x >= frame.x
                            && r_mine.y >= frame.y
                            && r_mine.right() <= frame.right()
                            && r_mine.bottom() <= frame.bottom(),
                        "内缩矩形 {r_mine:?} 必须完全落在框 {frame:?} 内"
                    );
                }
                other => panic!("不允许节点 `{other}` 上出现差异"),
            }
        }
        assert!(seen_button, "必须看到按钮的文字矩形差异");
        assert!(seen_field, "必须看到输入框的文字矩形差异");

        // ④ 所有命令都必须落在**它自己节点的矩形**内（列表的几何不变式）。
        let mut owner: Option<RectI> = None;
        for cmd in &mine.cmds {
            match cmd {
                DrawCmd::NodeHint { rect, .. } => owner = Some(*rect),
                DrawCmd::FillRect { rect, .. }
                | DrawCmd::StrokeRect { rect, .. }
                | DrawCmd::FillRoundRect { rect, .. }
                | DrawCmd::Text { rect, .. } => {
                    let o = owner.expect("提示必须先于绘制命令");
                    assert!(
                        rect.x >= o.x
                            && rect.y >= o.y
                            && rect.right() <= o.right()
                            && rect.bottom() <= o.bottom(),
                        "命令矩形 {rect:?} 落在自己的节点矩形 {o:?} 之外"
                    );
                }
                DrawCmd::PushClip { .. } | DrawCmd::PopClip => {}
            }
        }
    }

    /// 四档状态**各自**都要在像素上留下**够多**的差异 —— 判据是「差异 ≥ 明确下限」，不是「差异 > 0」。
    ///
    /// 为什么「> 0」不够：按钮的焦点环改前与填充**同色**，差异仍有 144 字节（方角补角 + 描边压字形），
    /// 于是「focused 有差异」一直绿着，而屏幕上的焦点环根本不存在（详见模块文档「焦点环」）。
    /// 下限的出处见下面那张表 —— 每个数字都是**实测值向下取整**，并说明它为什么与 0 之间隔着数量级。
    #[test]
    fn each_state_changes_pixels_inside_its_own_rect() {
        let t = tree();
        let g = geo();
        let theme = Theme::default();
        let ext = Extent {
            width: 200,
            height: 120,
        };
        let render = |s: &InteractState| {
            let list = InteractiveRenderer::new(theme.clone(), &ApproxMeasure, s).build(&t, &g);
            CpuRenderer::new().render(ext, &list, Color::rgb(0, 0, 0)).unwrap().pixels
        };
        let idle = render(&InteractState::default());
        let mut texts = std::collections::BTreeMap::new();
        texts.insert("field_1".to_string(), "用户名".to_string());
        let with_text = {
            let s = InteractState::default();
            let list = InteractiveRenderer::with_texts(
                theme.clone(),
                &ApproxMeasure,
                &s,
                FieldText::Content,
                &texts,
            )
            .build(&t, &g);
            CpuRenderer::new().render(ext, &list, Color::rgb(0, 0, 0)).unwrap().pixels
        };

        // 每一档的**像素**下限（本语料完全确定：固定树 + `ApproxMeasure` 占位度量 + 固定画布）。
        //
        // | 状态 | 视觉落点 | 上限（几何） | 实测 | 下限 | 为什么不是随手填的 |
        // |---|---|---|---|---|---|
        // | `hover` | 按钮填充整块提亮 | 792（按钮 36×22） | **628** | 600 | 792 减去被占位字形盖住的 164（提亮只改 R/G 两个通道 ⇒ 1256 字节 ÷ 2 = 628） |
        // | `pressed` | 按钮填充整块加深 | 792 | **628** | 600 | 同上（加深三通道都变 ⇒ 1884 字节 ÷ 3 = 628 像素） |
        // | `focused` | 焦点环（环带 264 px） | 264 | **228** | **198** | 上限的 3/4；与「环同色」时的 **56 px** 之间隔着一整个数量级（改前实测） |
        // | `text` | 输入框文字换成缓冲内容 | 占位格模型 | **66** | 40 | 上限即「缓冲文字」与「占位标签」的占位格之差（模型确定 ⇒ 可给紧下限） |
        //
        // 「环同色时 56 px」是**改前的实测**（= 144 字节）—— 那正是「> 0」抓不住、而这条下限必须抓住的点。
        let floors = [
            ("hover", 600usize),
            ("pressed", 600),
            ("focused", FOCUS_RING_MIN_PIXELS),
            ("text", 40),
        ];
        for ((name, px), (fname, floor)) in [
            ("hover", render(&InteractState::hovered("button_1"))),
            ("pressed", render(&InteractState::pressed("button_1"))),
            ("focused", render(&InteractState::focused("button_1"))),
            ("text", with_text),
        ]
        .into_iter()
        .zip(floors)
        {
            assert_eq!(name, fname, "前置：下限表与状态列表必须一一对应（对错了就是在测空气）");
            let bytes = px.iter().zip(idle.iter()).filter(|(a, b)| a != b).count();
            let pixels = px
                .chunks_exact(4)
                .zip(idle.chunks_exact(4))
                .filter(|(a, b)| a != b)
                .count();
            println!("{name}: 与 idle 不同的像素 = {pixels}（字节 {bytes}）｜下限 {floor}");
            assert!(
                pixels >= floor,
                "{name}: 只差 {pixels} 像素 < 下限 {floor} ⇒ 这一档的状态视觉等于没画出来"
            );
        }

        // 反向自检：三个状态**互不相同**（否则「按下了」和「悬停了」在像素上不可区分）。
        let h = render(&InteractState::hovered("button_1"));
        let p = render(&InteractState::pressed("button_1"));
        let f = render(&InteractState::focused("button_1"));
        assert_ne!(h, p, "hover 与 pressed 的像素必须不同");
        assert_ne!(h, f, "hover 与 focused 的像素必须不同");
        assert_ne!(p, f, "pressed 与 focused 的像素必须不同");
    }

    /// **禁用节点上不许出现任何状态视觉**（禁用子树不响应输入的像素面判据）。
    #[test]
    fn disabled_node_never_shows_state_visuals() {
        let t = Node::new(Kind::Column, "app").push(Node::new(Kind::Button, "off").disabled());
        let mut g = Geometry::new();
        g.insert("app".into(), Rect::new(0.0, 0.0, 100.0, 60.0));
        g.insert("off".into(), Rect::new(0.0, 0.0, 100.0, 20.0));
        let theme = Theme::default();
        let ext = Extent {
            width: 100,
            height: 60,
        };
        let pixels = |s: &InteractState| {
            let list = InteractiveRenderer::new(theme.clone(), &ApproxMeasure, s).build(&t, &g);
            CpuRenderer::new().render(ext, &list, Color::rgb(0, 0, 0)).unwrap().pixels
        };
        let idle = pixels(&InteractState::default());
        for (name, s) in [
            ("hover", InteractState::hovered("off")),
            ("pressed", InteractState::pressed("off")),
            ("focus", InteractState::focused("off")),
        ] {
            let px = pixels(&s);
            let diff = px.iter().zip(idle.iter()).filter(|(a, b)| a != b).count();
            println!("禁用节点 {name}: 差异字节 = {diff}");
            assert_eq!(diff, 0, "禁用节点不许被画出 {name} 视觉");
        }
    }

    /// 按钮焦点环的**像素数下限**（环带 = 按钮矩形内缩 `FOCUS_RING_INSET`、宽 `FOCUS_STROKE_WIDTH` 的一圈）。
    ///
    /// 出处（不是随手填的数字）：
    /// - 环带像素数 = `32×18 − 26×12 = 576 − 312 = `**264**（本语料 `button_1` = 36×22）；
    /// - 环画在标签**之上**，而标签与环同色（都是 `theme.on_accent`）⇒ 重叠处**不**产生差异，
    ///   实测 228（= 264 − 36 个被占位字形盖住的环像素）；
    /// - 下限取 264 的 **3/4 = 198**：给「字形盖住环」留出余量，同时**远高于**「环与填充同色」
    ///   时的实测 36 px（新的内缩几何；改前那种贴边几何是 56 px）—— 两种情形之间有一整条
    ///   数量级的空档，而这条空档由反向自检当场钉住。
    const FOCUS_RING_MIN_PIXELS: usize = 198;

    /// 焦点环的**对比度下限**：环带内每个差异像素的「三通道平均差」。
    ///
    /// 两个**实测锚点**（都在反向自检里当场跑）：
    /// - `on_accent`(255,255,255) 压在 `accent`(76,141,255) 上 ⇒ **97.7**（按下限放行）；
    /// - 「填充色提亮 5%」这种**技术上不同色、肉眼看不出**的环 ⇒ **17.0**（按下限拒绝）。
    ///
    /// 下限取 **64**（= 256 的四分之一）：落在两个锚点中间，两侧余量都 ≥ 33。
    /// 只数「差异像素数」是不够的 —— 那个近似色**照样**改掉整条环带（264 px，一个不少）。
    const FOCUS_RING_MIN_CONTRAST: f64 = 64.0;

    /// 一份逐像素对照的统计（打印用 + 判据用）。
    #[derive(Debug)]
    struct RingStats {
        /// 环带内的差异像素数。
        diff_inside: usize,
        /// 差异像素里落在环带**之外**的数量（必须为 0）。
        diff_outside: usize,
        /// 差异像素的「三通道平均差」。
        mean_contrast: f64,
    }

    /// 数出 `focused` 相对 `idle` 在环带内外各差了多少像素、以及平均通道差。
    ///
    /// `band` = 环带外沿矩形，`hole` = 环带内沿（`band` 内缩 `FOCUS_STROKE_WIDTH`）。
    fn ring_stats(idle: &[u8], focused: &[u8], ext: Extent, band: RectI, hole: RectI) -> RingStats {
        let w = ext.width as usize;
        let (mut inside, mut outside, mut sum) = (0usize, 0usize, 0u64);
        for p in 0..(ext.width as usize * ext.height as usize) {
            let i = p * 4;
            if focused[i..i + 4] == idle[i..i + 4] {
                continue;
            }
            let x = ((i / 4) % w) as i32;
            let y = ((i / 4) / w) as i32;
            let in_band = band.contains(x, y) && !hole.contains(x, y);
            if in_band {
                inside += 1;
            } else {
                outside += 1;
            }
            for c in 0..3 {
                sum += focused[i + c].abs_diff(idle[i + c]) as u64;
            }
        }
        let n = inside + outside;
        RingStats {
            diff_inside: inside,
            diff_outside: outside,
            mean_contrast: if n == 0 {
                0.0
            } else {
                sum as f64 / (3.0 * n as f64)
            },
        }
    }

    /// **焦点环判据本体**（函数化是为了让它在同一次运行里被反向自检）。
    ///
    /// 三条都要成立：① 差异**一个都不许**落在环带之外（否则焦点视觉溢出了自己的矩形）；
    /// ② 环带内差异像素数 ≥ [`FOCUS_RING_MIN_PIXELS`]；③ 平均通道差 ≥ [`FOCUS_RING_MIN_CONTRAST`]。
    fn focus_ring_verdict(name: &str, s: &RingStats) -> Result<(), String> {
        if s.diff_outside != 0 {
            return Err(format!(
                "[{name}] 有 {} 个差异像素落在环带之外",
                s.diff_outside
            ));
        }
        if s.diff_inside < FOCUS_RING_MIN_PIXELS {
            return Err(format!(
                "[{name}] 环带内只有 {} 个差异像素 < 下限 {}",
                s.diff_inside, FOCUS_RING_MIN_PIXELS
            ));
        }
        if s.mean_contrast < FOCUS_RING_MIN_CONTRAST {
            return Err(format!(
                "[{name}] 平均通道差 {:.1} < 下限 {:.1} —— 环画上去了，但看不出",
                s.mean_contrast, FOCUS_RING_MIN_CONTRAST
            ));
        }
        Ok(())
    }

    /// **按钮的焦点视觉必须真的看得见**（改前的实测：idle vs focused 只差 **56 px**，其中 32 px 还是
    /// 方角补角、24 px 是描边压过字形 —— 而当时的判据是 `diff > 0`，于是它一直绿着）。
    ///
    /// 本用例做三件事：① 打印真实数据（环带、差异像素数、平均通道差）；② 断言判据；
    /// ③ **反向自检**：在同一次运行里把环色换成「与填充同色」和「技术上不同、肉眼看不出」，
    /// 判据必须**两次都拒绝**（否则这条门槛就是在空转）。
    #[test]
    fn button_focus_ring_is_visible_against_the_fill() {
        let t = tree();
        let g = geo();
        let theme = Theme::default();
        let ext = Extent {
            width: 200,
            height: 120,
        };
        let br = *g.get("button_1").expect("测试前置：button_1 必须有几何");
        let btn = RectI::new(br.x as i32, br.y as i32, br.w as i32, br.h as i32);

        let px_of = |state: &InteractState, ring_color: Option<Color>| -> Vec<u8> {
            let mut list =
                InteractiveRenderer::new(theme.clone(), &ApproxMeasure, state).build(&t, &g);
            if let Some(c) = ring_color {
                let mut hit = 0;
                for cmd in list.cmds.iter_mut() {
                    if let DrawCmd::StrokeRect { color, width, .. } = cmd {
                        if *width == FOCUS_STROKE_WIDTH {
                            *color = c;
                            hit += 1;
                        }
                    }
                }
                assert_eq!(hit, 1, "前置：恰好一条焦点环命令（按钮的 idle/hover 视觉里没有描边）");
            }
            CpuRenderer::new()
                .render(ext, &list, Color::rgb(0, 0, 0))
                .unwrap()
                .pixels
        };

        // ---- 前置①②：命令层面的几何（判据的落点由它们决定，错了下面全在测空气）----
        let list = InteractiveRenderer::new(theme.clone(), &ApproxMeasure, &InteractState::focused("button_1"))
            .build(&t, &g);
        let mut owner: Option<RectI> = None;
        let (mut fill, mut ring) = (None, None);
        for cmd in &list.cmds {
            match cmd {
                DrawCmd::NodeHint { rect, .. } => owner = Some(*rect),
                DrawCmd::FillRoundRect { rect: r, radius, color } if owner == Some(btn) => {
                    assert_eq!(*r, btn, "按钮的填充必须铺满按钮矩形");
                    fill = Some((*radius, *color));
                }
                DrawCmd::StrokeRect { rect, color, width } if owner == Some(btn) => {
                    assert_eq!(*width, FOCUS_STROKE_WIDTH, "按钮上只允许焦点环这一条描边");
                    ring = Some((*rect, *color));
                }
                _ => {}
            }
        }
        let (radius, fill_color) = fill.expect("前置：按钮必须有填充命令");
        let (ring_rect, ring_color) = ring.expect("前置：focused 必须给按钮画出焦点环");
        assert_eq!(
            ring_rect,
            focus_ring_rect(btn),
            "前置：环带矩形 = 按钮矩形内缩 FOCUS_RING_INSET（判据的落点就是它）"
        );
        assert_eq!(fill_color, theme.accent, "前置：这一档的填充就是 accent（否则焦点环同色也不影响可见性）");
        // 内缩必须够得着圆角：环的直角顶点到圆角圆心的距离 √2·(radius − inset) 要 ≤ radius。
        assert!(
            (FOCUS_RING_INSET as f32) >= radius as f32 * (1.0 - 1.0 / 2.0f32.sqrt()),
            "前置：内缩 {FOCUS_RING_INSET} 不够 —— 填充半径 {radius} 的圆角会被直角环补成方角"
        );
        let hole = inset(ring_rect, FOCUS_STROKE_WIDTH);
        let band_pixels = (ring_rect.w * ring_rect.h - hole.w * hole.h) as usize;
        println!(
            "按钮 {btn:?}｜环带 {ring_rect:?}（宽 {FOCUS_STROKE_WIDTH}、内缩 {FOCUS_RING_INSET}）\
             ⇒ 环带像素 {band_pixels}｜填充 {fill_color:?}｜环 {ring_color:?}"
        );
        assert_eq!(
            band_pixels, 264,
            "前置：本语料的环带像素数变了 ⇒ 下面那条下限（由 264 推出）必须跟着重算"
        );
        assert_ne!(
            ring_color, fill_color,
            "前置：环与填充同色 ⇒ 下面的下限不可能成立（0 与 264 之间没有余地）"
        );

        // ---- ② 实测 + 判据 ----
        let idle = px_of(&InteractState::default(), None);
        let focused = px_of(&InteractState::focused("button_1"), None);
        let s = ring_stats(&idle, &focused, ext, ring_rect, hole);
        println!(
            "focused vs idle：环带内差异像素 {}（下限 {}）｜环带外 {}（要求 0）｜平均通道差 {:.1}（下限 {:.1}）",
            s.diff_inside, FOCUS_RING_MIN_PIXELS, s.diff_outside, s.mean_contrast, FOCUS_RING_MIN_CONTRAST
        );
        println!(
            "对照：差异字节数 = {}（改前是 144；其中 32 px 是方角补角、24 px 是描边压字形）",
            focused.iter().zip(idle.iter()).filter(|(a, b)| a != b).count()
        );
        focus_ring_verdict("实测", &s).unwrap_or_else(|e| panic!("{e}"));

        // ---- ③ 反向自检（每次运行都跑）----
        //    (a) 环色改回填充色 —— 本次要修的那一版就是它（那时环还贴着边画，多出 32 px
        //        方角补角 ⇒ 56 px；这里的新几何下只剩「环压过字形」的那些像素 ⇒ 36 px，
        //        两者都远低于下限，判据必须拒绝）。
        let same = px_of(&InteractState::focused("button_1"), Some(fill_color));
        let s_same = ring_stats(&idle, &same, ext, ring_rect, hole);
        println!(
            "反向自检 a（环 = 填充色）：环带内差异 {}｜平均通道差 {:.1} ⇒ {:?}",
            s_same.diff_inside,
            s_same.mean_contrast,
            focus_ring_verdict("a", &s_same).err()
        );
        assert!(
            focus_ring_verdict("反向自检 a：环与填充同色", &s_same).is_err(),
            "判据必须拒绝「环与填充同色」，否则它抓不住本次要修的缺陷"
        );
        //    (b) 环色 = 填充色提亮 5%（技术上不同色、肉眼看不出）⇒ 差异像素数几乎不少，靠**对比度**下限咬住。
        let faint = px_of(
            &InteractState::focused("button_1"),
            Some(fill_color.lighten(0.05)),
        );
        let s_faint = ring_stats(&idle, &faint, ext, ring_rect, hole);
        println!(
            "反向自检 b（环 = 填充提亮 5%）：环带内差异 {}｜平均通道差 {:.1} ⇒ {:?}",
            s_faint.diff_inside,
            s_faint.mean_contrast,
            focus_ring_verdict("b", &s_faint).err()
        );
        assert!(
            s_faint.diff_inside >= FOCUS_RING_MIN_PIXELS,
            "反向自检 b 的前置：近似色**照样**改掉整条环带（{} px）⇒ 只数像素数抓不住它",
            s_faint.diff_inside
        );
        assert!(
            focus_ring_verdict("反向自检 b：环色几乎与填充相同", &s_faint).is_err(),
            "判据必须拒绝「技术上有差异、肉眼看不出」的环，否则对比度下限是摆设"
        );

        // ---- ④ 越界：差异不许出现在**别的控件**上（输入框/容器一个字节都不许动）----
        let fr = *g.get("field_1").expect("测试前置：field_1 必须有几何");
        let field = RectI::new(fr.x as i32, fr.y as i32, fr.w as i32, fr.h as i32);
        let stray = focused
            .iter()
            .zip(idle.iter())
            .enumerate()
            .filter(|(_, (a, b))| a != b)
            .filter(|(i, _)| {
                let (x, y) = ((i / 4 % ext.width as usize) as i32, (i / 4 / ext.width as usize) as i32);
                !(btn.contains(x, y) || field.contains(x, y))
            })
            .count();
        println!("按钮/输入框之外的差异字节 = {stray}");
        assert_eq!(stray, 0, "按钮焦点视觉溢到了别的控件上");
    }

    /// 零面积节点：本模块（照抄 `NullRenderer`）**发提示且发描边**，`DefaultRenderer` 直接早退。
    /// 差异是刻意的、被钉住的 —— 将来谁改了一边，这里会红。
    #[test]
    fn zero_area_node_is_differently_handled_than_default_renderer() {
        let t = Node::new(Kind::Column, "app").push(Node::new(Kind::Button, "zero"));
        let mut g = Geometry::new();
        g.insert("app".into(), Rect::new(0.0, 0.0, 50.0, 50.0));
        g.insert("zero".into(), Rect::new(0.0, 0.0, 0.0, 0.0));
        let theme = Theme::default();
        let s = InteractState::default();

        let mine = InteractiveRenderer::new(theme.clone(), &ApproxMeasure, &s).build(&t, &g);
        assert_eq!(mine.counts().node_hint, 2, "零面积节点也要有提示（绑定协议）");
        let base = DefaultRenderer::new(theme, &ApproxMeasure).build(&t, &g);
        assert_eq!(base.counts().stroke_rect, 0, "默认渲染器对零面积节点早退");
        assert_eq!(base.counts().node_hint, 0, "默认渲染器不发提示");

        // 并且零面积 + 焦点时，本模块会画出描边（默认渲染器画不出任何东西）。
        let f = InteractState::focused("zero");
        let focused = InteractiveRenderer::new(Theme::default(), &ApproxMeasure, &f).build(&t, &g);
        assert_eq!(focused.counts().stroke_rect, 1, "零面积的焦点节点也要有描边");
    }

    // ---- 剩余工作第 1 项：滚动容器的视口裁剪 / 多行文本 ----

    /// 本模块的列表是**裁剪感知命中的唯一输入**（`ClipSnapshot::from_draw_list` 的绑定协议），
    /// 所以「滚动容器把视口裁剪推在子节点的提示之前」必须在这里钉住。
    ///
    /// 三条：① 可滚动容器**恰好一对** `PushClip`/`PopClip`；② 容器自己的 `NodeHint` 在
    /// `PushClip` **之前**（否则容器自身会被自己的视口裁掉）；③ 子节点的提示在裁剪**里面**。
    #[test]
    fn scroll_container_clips_its_children_but_not_itself() {
        let theme = Theme::default();
        let s = InteractState::default();
        let build = |scroll: bool| {
            let mut b = deer_layout::builder::Builder::new(Kind::Column, "app");
            b.container_opts(
                Kind::Column,
                "outer",
                deer_layout::builder::L::new()
                    .w(200.0)
                    .h(100.0)
                    .scroll(scroll)
                    .to_props(),
                |o| {
                    for i in 1..=3 {
                        o.button(format!("{i}"));
                    }
                },
            );
            let tree = b.build();
            let geo = deer_layout::layout::layout(
                &tree,
                Rect::new(0.0, 0.0, 200.0, 100.0),
                deer_layout::TextStyle::default(),
                &ApproxMeasure,
            );
            let list = InteractiveRenderer::new(theme.clone(), &ApproxMeasure, &s).build(&tree, &geo);
            (tree, geo, list)
        };

        let (tree, geo, list) = build(true);
        assert!(list.clip_balanced(), "裁剪栈必须平衡（否则 CPU/GPU 后端会直接报错）");
        let c = list.counts();
        println!(
            "滚动容器：hint {}｜push_clip {} / pop_clip {}｜命令 {}",
            c.node_hint,
            c.push_clip,
            c.pop_clip,
            list.len()
        );
        assert_eq!((c.push_clip, c.pop_clip), (1, 1), "恰好一对视口裁剪");
        assert_eq!(c.node_hint, geo.len(), "每个有几何的节点一条提示（既有协议不变）");

        let viewport = {
            let r = geo["outer"];
            RectI::new(r.x as i32, r.y as i32, r.w as i32, r.h as i32)
        };
        let hint_at = list
            .cmds
            .iter()
            .position(|cmd| matches!(cmd, DrawCmd::NodeHint { rect, .. } if *rect == viewport))
            .expect("outer 的提示");
        let push_at = list
            .cmds
            .iter()
            .position(|cmd| matches!(cmd, DrawCmd::PushClip { .. }))
            .expect("有 PushClip");
        let pop_at = list
            .cmds
            .iter()
            .position(|cmd| matches!(cmd, DrawCmd::PopClip))
            .expect("有 PopClip");
        match &list.cmds[push_at] {
            DrawCmd::PushClip { rect } => assert_eq!(*rect, viewport, "裁剪矩形 = 视口矩形"),
            _ => unreachable!(),
        }
        println!("outer 提示 #{hint_at}｜PushClip #{push_at}｜PopClip #{pop_at}");
        assert!(hint_at < push_at, "容器自己的提示必须在裁剪之前");
        assert!(push_at < pop_at);
        // 子节点（按钮）的提示在裁剪里面 ⇒ 它们的有效裁剪 = 视口。
        let child_hints = (push_at + 1..pop_at)
            .filter(|i| matches!(list.cmds[*i], DrawCmd::NodeHint { .. }))
            .count();
        println!("裁剪之内的子节点提示 = {child_hints}");
        assert_eq!(child_hints, 3, "三个按钮的提示都在视口裁剪里面");

        // 对照组：不滚动 ⇒ **一条裁剪命令都没有**（「不动滚动时输出不变」的最直接证据）。
        let (_, geo_plain, plain) = build(false);
        assert_eq!(geo_plain["outer"], geo["outer"], "容器几何相同（只有子节点顺序无位移）");
        let cp = plain.counts();
        println!("不滚动的对照：push_clip {} / pop_clip {}", cp.push_clip, cp.pop_clip);
        assert_eq!((cp.push_clip, cp.pop_clip), (0, 0));
        assert_eq!(cp.node_hint, list.counts().node_hint);
        let _ = tree;
    }

    /// 换行文本：**每行一条命令**，且每条命令的矩形都落在节点自己的矩形内。
    #[test]
    fn wrapped_text_emits_one_command_per_line_inside_the_node_rect() {
        let theme = Theme::default();
        let s = InteractState::default();
        let build = |wrap: bool| {
            let mut b = deer_layout::builder::Builder::new(Kind::Column, "app");
            b.text_opts("alpha beta gamma delta", |n| {
                n.layout.width = Some(deer_layout::node::Size::Px(60.0));
                n.layout.wrap = wrap;
            });
            let tree = b.build();
            let geo = deer_layout::layout::layout(
                &tree,
                Rect::new(0.0, 0.0, 200.0, 260.0),
                deer_layout::TextStyle::default(),
                &ApproxMeasure,
            );
            let list = InteractiveRenderer::new(theme.clone(), &ApproxMeasure, &s).build(&tree, &geo);
            let node = {
                let r = geo["text_1"];
                RectI::new(r.x as i32, r.y as i32, r.w as i32, r.h as i32)
            };
            (list, node)
        };

        let (list, node) = build(true);
        let texts: Vec<(RectI, String)> = list
            .cmds
            .iter()
            .filter_map(|c| match c {
                DrawCmd::Text { rect, text, .. } => Some((*rect, text.clone())),
                _ => None,
            })
            .collect();
        println!("节点 {node:?}｜换行 ⇒ {texts:?}");
        assert_eq!(node, RectI::new(0, 0, 60, 72), "前置：4 行 × 18");
        assert_eq!(texts.len(), 4);
        for (i, (rect, text)) in texts.iter().enumerate() {
            assert_eq!(
                *rect,
                RectI::new(0, 18 * i as i32, 60, 18),
                "第 {i} 行的矩形"
            );
            assert!(!text.is_empty(), "第 {i} 行不该是空串");
            assert!(
                rect.x >= node.x
                    && rect.y >= node.y
                    && rect.right() <= node.right()
                    && rect.bottom() <= node.bottom(),
                "第 {i} 行 {rect:?} 越出了节点矩形 {node:?}"
            );
        }

        // 不换行 ⇒ 恰好一条，矩形 = 节点矩形（既有语料的前提）。
        let (plain, node_p) = build(false);
        let texts_p: Vec<(RectI, String)> = plain
            .cmds
            .iter()
            .filter_map(|c| match c {
                DrawCmd::Text { rect, text, .. } => Some((*rect, text.clone())),
                _ => None,
            })
            .collect();
        println!("节点 {node_p:?}｜不换行 ⇒ {texts_p:?}");
        assert_eq!(texts_p.len(), 1);
        assert_eq!(texts_p[0], (node_p, "alpha beta gamma delta".to_string()));
    }
}
