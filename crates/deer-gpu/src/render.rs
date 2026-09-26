//! 默认渲染器：把「节点树 + 几何 + 主题」翻译成平台无关的 [`DrawList`]。
//!
//! 这是「渲染」这条链上**唯一需要为控件类型改动的地方**：新增一种控件只是多一个
//! `match` 分支，后端（CPU / Vulkan）完全不用动 —— 它们只认 [`DrawCmd`]。
//!
//! 与真实 GUI 的差距（诚实说明）：字形目前是**等宽格占位**，不是排版。
//! 真实字形需要字体解析 + 图集（里程碑 M4）。但**几何、裁剪、层次、颜色**都是真的，
//! 所以现在就能用来验证布局是否正确。

use deer_layout::node::{Kind, Node};
use deer_layout::layout::{Geometry, Measure};

use crate::draw::{Color, DrawCmd, DrawList, RectI};
use crate::Theme;

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
                list.push(DrawCmd::Text {
                    rect,
                    text: n.props.label.clone().unwrap_or_default(),
                    color,
                    size: self.theme.font_size,
                    align: 0,
                });
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

        for c in &n.children {
            self.emit(c, geo, list);
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
    pub fn build(tree: &Node, geo: &Geometry) -> DrawList {
        let mut list = DrawList::new();
        fn walk(n: &Node, geo: &Geometry, list: &mut DrawList) {
            if let Some(f) = geo.get(&n.id) {
                list.push(DrawCmd::NodeHint {
                    rect: RectI::new(f.x as i32, f.y as i32, f.w as i32, f.h as i32),
                    node_id_len: n.id.len() as u32,
                });
            }
            for c in &n.children {
                walk(c, geo, list);
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
