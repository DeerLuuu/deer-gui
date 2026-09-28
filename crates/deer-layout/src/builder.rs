//! 命令式声明 API（imgui 式手感）。
//!
//! 调用点即控件、无样板、可链式；**内部维持一棵保留式节点树** —— 因为布局、
//! 命中测试、渲染都需要树（imgui 内部同样维持状态与布局，「立即」的只是提交方式）。
//!
//! ```ignore
//! let mut app = Builder::new(Kind::Column, "app").padding(12.0).gap(8.0);
//! app.text("标题");
//! app.container(Kind::Row, "bar", |r| {
//!     r.button("确定");
//!     r.button("取消");
//! });
//! let tree = app.build();
//! ```
//!
//! **关键约束**：`IdGen` 按 kind 计数，必须与 `scene` 的规则逐字一致，否则
//! 两条构筑路径产出的树不等（deer-ui V0 的 B-1 缺陷）。

use crate::node::{Align, IdGen, Kind, LayoutProps, Node, NodeProps, Size};

pub struct Builder {
    root: Node,
    /// 当前挂载点路径（索引链），避免借用 `&mut Node` 与递归冲突。
    path: Vec<usize>,
    ids: IdGen,
}

impl Builder {
    pub fn new(kind: Kind, id: impl Into<String>) -> Builder {
        let id: String = id.into();
        let mut ids = IdGen::new();
        // 根是显式命名 ⇒ 占号，避免子节点的自动 id 撞上根名
        ids.reserve(&id);
        Builder {
            root: Node::new(kind, id),
            path: Vec::new(),
            ids,
        }
    }

    /// 不带 id 的根：由 `IdGen` 生成（与场景文件默认行为一致）。
    pub fn auto(kind: Kind) -> Builder {
        let mut ids = IdGen::new();
        let id = ids.next(kind);
        Builder {
            root: Node::new(kind, id),
            path: Vec::new(),
            ids,
        }
    }

    /// 仅对容器有效，内边距。
    pub fn padding(mut self, v: f32) -> Builder {
        self.root.layout.padding = v;
        self
    }

    /// 仅对容器有效，子节点间距。
    pub fn gap(mut self, v: f32) -> Builder {
        self.root.layout.gap = v;
        self
    }

    /// 仅对容器有效，主轴对齐。
    pub fn size(mut self, w: Option<Size>, h: Option<Size>) -> Builder {
        self.root.layout.width = w;
        self.root.layout.height = h;
        self
    }

    /// 仅对容器有效，主轴对齐。
    fn node_at_mut<'a>(root: &'a mut Node, path: &[usize]) -> &'a mut Node {
        let mut cur = root;
        for &i in path {
            cur = &mut cur.children[i];
        }
        cur
    }

    /// 挂载一个节点到当前路径的容器下。
    fn push(&mut self, n: Node) {
        let path = self.path.clone();
        let parent = Self::node_at_mut(&mut self.root, &path);
        parent.children.push(n);
    }

    /// 追加一个叶子控件，返回其 id（事件关联用 id，不进树 —— 树是纯数据）。
    ///
    /// 注意建造顺序：**先设 layout/props，最后用 `with_id` 定 id** ——
    /// 反过来写会被 `with_props` 整块覆盖而丢掉 label（本项目踩过一次）。
    pub fn text(&mut self, label: impl Into<String>) -> String {
        let n = Node::new(Kind::Text, "").with_label(label);
        self.push_named(n, Kind::Text)
    }

    /// 追加一个叶子控件，返回其 id（事件关联用 id，不进树 —— 树是纯数据）。
    pub fn button(&mut self, label: impl Into<String>) -> String {
        let n = Node::new(Kind::Button, "").with_label(label);
        self.push_named(n, Kind::Button)
    }

    /// 文本 + 就地改参数（换行等）：`b.text_opts("…", |n| n.layout.wrap = true)`。
    ///
    /// 与 `button_opts` 同一个形状：**先设参数、最后定 id**（见 `text` 的注释）。
    pub fn text_opts(&mut self, label: impl Into<String>, f: impl FnOnce(&mut Node)) -> String {
        let mut n = Node::new(Kind::Text, "").with_label(label);
        f(&mut n);
        self.push_named(n, Kind::Text)
    }

    /// 追加一个叶子控件，返回其 id（事件关联用 id，不进树 —— 树是纯数据）。
    pub fn button_opts(&mut self, label: impl Into<String>, f: impl FnOnce(&mut Node)) -> String {
        let mut n = Node::new(Kind::Button, "").with_label(label);
        f(&mut n);
        self.push_named(n, Kind::Button)
    }

    pub fn field(&mut self, label: impl Into<String>) -> String {
        let n = Node::new(Kind::Field, "").with_label(label);
        self.push_named(n, Kind::Field)
    }

    /// 挂载一个节点；`id` 为空时按 kind 自动生成。
    ///
    /// 显式名字会被 [`IdGen::reserve`] 记下并占号 —— 否则自动 id 可能撞上它。
    fn push_named(&mut self, mut n: Node, kind: Kind) -> String {
        if n.id.is_empty() {
            n = n.with_id(self.ids.next(kind));
        } else {
            self.ids.reserve(&n.id);
        }
        let id = n.id.clone();
        self.push(n);
        id
    }

    /// 容器 + 闭包式嵌套（**不指定 id**，自动生成）。
    pub fn container_auto(&mut self, kind: Kind, body: impl FnOnce(&mut Builder)) {
        let id = self.ids.next(kind);
        self.container_with(kind, id, LayoutProps::default(), body);
    }

    /// 容器 + 闭包式嵌套。**闭包内 new 出来的节点挂在容器下。**
    pub fn container(&mut self, kind: Kind, id: impl Into<String>, body: impl FnOnce(&mut Builder)) {
        assert!(kind.is_container(), "container() 只接受 Column/Row");
        let n = Node::new(kind, id);
        self.push(n);
        let parent_path = self.path.clone();
        let mut cur_path = parent_path.clone();
        // 找到刚 push 的索引
        {
            let p = Self::node_at_mut(&mut self.root, &parent_path);
            cur_path.push(p.children.len() - 1);
        }
        self.path = cur_path;
        body(self);
        self.path = parent_path;
    }

    /// 容器 + 布局参数 + 显式 id。
    pub fn container_opts(
        &mut self,
        kind: Kind,
        id: impl Into<String>,
        layout: LayoutProps,
        body: impl FnOnce(&mut Builder),
    ) {
        assert!(kind.is_container(), "container_opts() 只接受 Column/Row");
        let id: String = id.into();
        let resolved = if id.is_empty() { self.ids.next(kind) } else { id };
        self.container_with(kind, resolved, layout, body);
    }

    fn container_with(
        &mut self,
        kind: Kind,
        id: impl Into<String>,
        layout: LayoutProps,
        body: impl FnOnce(&mut Builder),
    ) {
        // 顺序要紧：**先 layout，最后 `with_id`**。
        // `layout`/`props` 都是整体赋值 —— 先定 id 再设它们会连着把 id 一起改掉。
        let id: String = id.into();
        let n = Node::new(kind, "").with_layout(layout).with_id(id.clone());
        self.ids.reserve(&id);
        self.push(n);
        let parent_path = self.path.clone();
        let mut cur_path = parent_path.clone();
        {
            let p = Self::node_at_mut(&mut self.root, &parent_path);
            cur_path.push(p.children.len() - 1);
        }
        self.path = cur_path;
        body(self);
        self.path = parent_path;
    }

    pub fn build(&self) -> Node {
        self.root.clone()
    }

    pub fn root_id(&self) -> &str {
        &self.root.id
    }
}

/// 便捷构造 `LayoutProps`。
#[derive(Debug, Clone, Copy, Default)]
pub struct L {
    pub width: Option<Size>,
    pub height: Option<Size>,
    pub padding: Option<f32>,
    pub gap: Option<f32>,
    pub main_axis: Option<Align>,
    pub cross_axis: Option<Align>,
    pub grow: Option<f32>,
    pub scroll: Option<bool>,
    pub wrap: Option<bool>,
}

impl L {
    /// 便捷构造 `LayoutProps`。
    pub fn new() -> L {
        L::default()
    }
    /// 便捷构造 `LayoutProps`。
    pub fn w(mut self, v: f32) -> L {
        self.width = Some(Size::Px(v));
        self
    }
    /// 便捷构造 `LayoutProps`。
    pub fn h(mut self, v: f32) -> L {
        self.height = Some(Size::Px(v));
        self
    }
    /// 便捷构造 `LayoutProps`。
    pub fn pad(mut self, v: f32) -> L {
        self.padding = Some(v);
        self
    }
    /// 便捷构造 `LayoutProps`。
    pub fn gap(mut self, v: f32) -> L {
        self.gap = Some(v);
        self
    }
    /// 便捷构造 `LayoutProps`。
    pub fn main(mut self, a: Align) -> L {
        self.main_axis = Some(a);
        self
    }
    /// 便捷构造 `LayoutProps`。
    pub fn cross(mut self, a: Align) -> L {
        self.cross_axis = Some(a);
        self
    }
    /// 便捷构造 `LayoutProps`。
    pub fn grow(mut self, v: f32) -> L {
        self.grow = Some(v);
        self
    }
    /// 垂直滚动容器（只对 `Column` 有意义）。
    pub fn scroll(mut self, v: bool) -> L {
        self.scroll = Some(v);
        self
    }
    /// 文本按节点像素宽度换行（只对 `Kind::Text` 有意义）。
    pub fn wrap(mut self, v: bool) -> L {
        self.wrap = Some(v);
        self
    }
    /// 便捷构造 `LayoutProps`。
    pub fn to_props(self) -> LayoutProps {
        LayoutProps {
            width: self.width,
            height: self.height,
            padding: self.padding.unwrap_or(0.0),
            gap: self.gap.unwrap_or(0.0),
            main_axis: self.main_axis,
            cross_axis: self.cross_axis,
            grow: self.grow.unwrap_or(0.0),
            scroll: self.scroll.unwrap_or(false),
            wrap: self.wrap.unwrap_or(false),
        }
    }
}

/// 便捷构造 `NodeProps`。
pub fn props(label: Option<&str>, disabled: bool) -> NodeProps {
    NodeProps {
        label: label.map(|s| s.to_string()),
        disabled,
    }
}
