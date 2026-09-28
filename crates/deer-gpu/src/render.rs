//! 默认渲染器：把「节点树 + 几何 + 主题」翻译成平台无关的 [`DrawList`]。
//!
//! 这是「渲染」这条链上**唯一需要为控件类型改动的地方**：新增一种控件只是多一个
//! `match` 分支，后端（CPU / Vulkan）完全不用动 —— 它们只认 [`DrawCmd`]。
//!
//! 与真实 GUI 的差距（诚实说明）：字形目前是**等宽格占位**，不是排版。
//! 真实字形需要字体解析 + 图集（里程碑 M4）。但**几何、裁剪、层次、颜色**都是真的，
//! 所以现在就能用来验证布局是否正确。
//!
//! ## 多行文本与滚动容器（剩余工作第 1 项）
//!
//! 两件事都**不新增 `DrawCmd` 变体**，因此 CPU / Vulkan 后端一行都不用改：
//!
//! - **多行文本**：`Kind::Text` + `layout.wrap` ⇒ 渲染器按 [`text_lines`] 把文本展开成
//!   **每行一条 `DrawCmd::Text`**（各自矩形 = 节点矩形按行高下移）。单行语料仍然是
//!   一条命令、矩形不变 ⇒ **既有像素判据逐字节不变**；
//! - **滚动容器**：`Column` + `layout.scroll` ⇒ 在容器**自己的视觉之后**推
//!   `PushClip`（= 视口矩形），走完子节点再 `PopClip`。裁剪栈的语义照抄
//!   `null.rs`（求交 / 出栈），所以 GPU 后端与 CPU 后端看到的完全是同一份数据。

use deer_layout::node::{Kind, Node};
use deer_layout::layout::{Geometry, Measure, TextStyle};

use crate::draw::{Color, DrawCmd, DrawList, RectI};
use crate::Theme;

/// **一个文本节点的每一行**：`(该行的矩形, 该行的文本)`。
///
/// 规则（确定性；`wrap` 关闭时**恰好一行**，矩形就是节点矩形 —— 这条让既有语料
/// 逐字节不变）：
///
/// - `wrap` 关闭 ⇒ `vec![(rect, 全文)]`；
/// - `wrap` 打开 ⇒ 换行点来自 [`Measure::wrap`]（行数与布局预留高度**同源**）；
/// - 行矩形 = `(rect.x, rect.y + i*line_height, rect.w, 行高夹到节点下边界)`；
/// - **装不下的行不画**（`y >= rect.bottom()` 即停）：命令矩形绝不越出节点自己的矩形
///   ——这是列表的几何不变式（统一管线的裁剪建立在它之上）。节点高度 = 行数 × 行高时
///   一行都不会丢；显式高度更矮时就是「画得下几行画几行」（被测试钉住的行为）。
pub fn text_lines<M: Measure>(
    measure: &M,
    n: &Node,
    rect: RectI,
    style: TextStyle,
) -> Vec<(RectI, String)> {
    let label = n.props.label.clone().unwrap_or_default();
    if !n.wraps_text() {
        return vec![(rect, label)];
    }
    let mut lines = measure.wrap(&label, style, rect.w as f32);
    if lines.is_empty() {
        // 防御：自定义度量回空 ⇒ 退回「一行空串」（空串也算 1 行，与度量约定一致）。
        lines.push(String::new());
    }
    let line_h = style.line_height.round().max(1.0) as i32;
    let mut out = Vec::new();
    for (i, line) in lines.into_iter().enumerate() {
        let y = rect.y + line_h * i as i32;
        if y >= rect.bottom() {
            break;
        }
        let h = line_h.min(rect.bottom() - y);
        out.push((RectI::new(rect.x, y, rect.w, h), line));
    }
    out
}

/// 默认渲染器。`measure` 用于把文本对齐到几何盒里（与布局阶段同一个度量）。
pub struct DefaultRenderer<'a, M: Measure> {
    pub theme: Theme,
    pub measure: &'a M,
}

impl<'a, M: Measure> DefaultRenderer<'a, M> {
    pub fn new(theme: Theme, measure: &'a M) -> Self {
        DefaultRenderer { theme, measure }
    }

    /// 把树 + 几何翻成绘制列表。
    pub fn build(&self, tree: &Node, geo: &Geometry) -> DrawList {
        let mut list = DrawList::new();
        self.emit(tree, geo, &mut list);
        list
    }

    fn emit(&self, n: &Node, geo: &Geometry, list: &mut DrawList) {
        let Some(f) = geo.get(&n.id) else { return };
        let rect = RectI::new(f.x as i32, f.y as i32, f.w as i32, f.h as i32);
        if rect.w <= 0 || rect.h <= 0 {
            return;
        }
        let disabled = n.props.disabled;

        match n.kind {
            Kind::Column | Kind::Row => {
                // 只有显式给了内边距的容器才画底（否则整屏都是方块，无法看清层次）
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
                let color = if disabled { self.theme.text_dim } else { self.theme.text };
                // 多行：每行一条命令（`wrap` 关时恰好一条，矩形不变 ⇒ 既有语料逐字节不变）。
                for (line_rect, line) in text_lines(self.measure, n, rect, self.text_style()) {
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
                let bg = if disabled {
                    self.theme.border
                } else {
                    self.theme.accent
                };
                list.push(DrawCmd::FillRoundRect {
                    rect,
                    radius: 4,
                    color: bg,
                });
                // 文字在按钮里居中（用与布局同一个度量算宽度，避免两处不一致）
                let label = n.props.label.clone().unwrap_or_default();
                let tw = self.measure.width(&label, self.text_style());
                let tx = rect.x + ((rect.w as f32 - tw) / 2.0).floor() as i32;
                list.push(DrawCmd::Text {
                    rect: RectI::new(tx, rect.y, rect.w, rect.h),
                    text: label,
                    color: if disabled { self.theme.text_dim } else { self.theme.on_accent },
                    size: self.theme.font_size,
                    align: 0,
                });
            }
            Kind::Field => {
                list.push(DrawCmd::FillRoundRect {
                    rect,
                    radius: 4,
                    color: self.theme.border,
                });
                list.push(DrawCmd::StrokeRect {
                    rect,
                    color: self.theme.text_dim,
                    width: 1,
                });
                list.push(DrawCmd::Text {
                    rect,
                    text: n.props.label.clone().unwrap_or_default(),
                    color: self.theme.text_dim,
                    size: self.theme.font_size,
                    align: 0,
                });
            }
        }

        // 滚动容器：内容裁剪到视口（与 `null.rs` 的裁剪栈语义一致：求交 / 出栈）。
        // 位置在**容器自己的视觉之后**、子节点之前 —— 于是视口外的内容画不出来，
        // 而容器自身（背景/边框）照常画满视口。
        let clip = n.is_scroll_container();
        if clip {
            list.push(DrawCmd::PushClip { rect });
        }
        for c in &n.children {
            self.emit(c, geo, list);
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

/// 一个不画任何东西、只回命令的渲染器（诊断 / 测试用）。
pub struct NullRenderer;

impl NullRenderer {
    /// 每个有几何的节点一条 `NodeHint`（既有协议）；**可滚动容器另外推一对视口裁剪**，
    /// 这样从这份列表派生出的 `ClipSnapshot` 也带着「视口外的点不命中」这条语义
    /// （`interaction::ClipSnapshot::from_draw_list` 的绑定协议：提示顺序不变）。
    pub fn build(tree: &Node, geo: &Geometry) -> DrawList {
        let mut list = DrawList::new();
        fn walk(n: &Node, geo: &Geometry, list: &mut DrawList) {
            let hint = geo.get(&n.id).map(|f| {
                RectI::new(f.x as i32, f.y as i32, f.w as i32, f.h as i32)
            });
            if let Some(rect) = hint {
                // `NodeHint` 现在是**三字段**（含 `node_id_fp`）⇒ 必须走构造函数，别手写字面量。
                list.push(DrawCmd::node_hint(rect, &n.id));
            }
            let clip = n.is_scroll_container().then_some(hint).flatten();
            if let Some(rect) = clip {
                list.push(DrawCmd::PushClip { rect });
            }
            for c in &n.children {
                walk(c, geo, list);
            }
            if clip.is_some() {
                list.push(DrawCmd::PopClip);
            }
        }
        walk(tree, geo, &mut list);
        list
    }
}

/// 便捷函数：一步完成「树 + 几何 → 绘制列表」。
pub fn build_draw_list<M: Measure>(tree: &Node, geo: &Geometry, theme: Theme, measure: &M) -> DrawList {
    DefaultRenderer::new(theme, measure).build(tree, geo)
}

/// 空颜色常量（便于调用方显式表达「透明」）。
pub const TRANSPARENT: Color = Color::TRANSPARENT;
