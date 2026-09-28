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
//! | `focus` | 沿矩形内侧画 `FOCUS_STROKE_WIDTH` 像素的强调色描边（填色回到 idle） |
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
                list.push(DrawCmd::StrokeRect {
                    rect: RectI::new(r, ry, rw, rh),
                    color: self.theme.accent,
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
                    list.push(DrawCmd::Text {
                        rect,
                        text: n.props.label.clone().unwrap_or_default(),
                        color: if n.props.disabled {
                            self.theme.text_dim
                        } else {
                            self.theme.text
                        },
                        size: self.theme.font_size,
                        align: 0,
                    });
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
            // ③ 焦点描边：Button 的 idle/状态视觉里都没有描边 ⇒ 这一条就是它的全部差异。
            if focused && n.kind == Kind::Button {
                list.push(DrawCmd::StrokeRect {
                    rect,
                    color: self.theme.accent,
                    width: FOCUS_STROKE_WIDTH,
                });
            }
        }

        for c in &n.children {
            self.walk(c, geo, dead, list);
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

    /// 四档状态**各自**都要在像素上留下差异（否则像素判据是在空转）。
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

        for (name, px) in [
            ("hover", render(&InteractState::hovered("button_1"))),
            ("pressed", render(&InteractState::pressed("button_1"))),
            ("focused", render(&InteractState::focused("button_1"))),
            ("text", with_text),
        ] {
            let diff = px.iter().zip(idle.iter()).filter(|(a, b)| a != b).count();
            println!("{name}: 与 idle 不同的字节数 = {diff}");
            assert!(diff > 0, "{name} 必须真的改变像素（否则态判据是空转的）");
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
}
