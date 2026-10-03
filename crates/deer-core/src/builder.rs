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

use crate::node::{Align, IdGen, Kind, LayoutProps, Node, NodeProps, Pos, Size};

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
        assert!(kind.is_container(), "container() 只接受容器 Kind（Column/Row/选择类组）");
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
        assert!(kind.is_container(), "container_opts() 只接受容器 Kind（Column/Row/选择类组）");
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

    // —— M6 5a：RowActions（行尾动作按钮组 · **组合层**） ————————————————

    /// 行尾动作按钮组：**`Row` + 每个标签一个 `button`**（M6 控件族 5a · 组合层）。
    ///
    /// 为什么是组合而不是扩 `Kind`：RowActions 的每个像素、每条 `Clicked`、每个
    /// 可聚焦项都**已经**来自 `Row`（布局/命中）与 `Button`（绘制/交互）——
    /// 扩 `Kind` 要同步 registry/scene/builder/绘制四处，却不会带来任何新行为；
    /// 组合层只是把「建一个 Row、逐个建 button、收好 id」这段样板收进库里
    /// （判据：与手写等价树 `structurally_eq` 且绘制命令逐条相同，见
    /// `crates/deer-gui/tests/m6_basics.rs`）。
    ///
    /// - 返回**按钮 id 列表**（顺序与 `labels` 一致；事件关联用 id，不进树）；
    /// - Row 自身 id 自动生成（`row_N`）；要指定 id / 布局（间距、对齐、尺寸）用
    ///   [`Builder::row_actions_opts`]；
    /// - `labels` 为空 ⇒ 一个没有子节点的 Row（合法；无 padding 的容器零绘制命令）。
    pub fn row_actions(&mut self, labels: &[&str]) -> Vec<String> {
        self.row_actions_opts("", LayoutProps::default(), labels)
    }

    /// 同 [`Builder::row_actions`]，但可指定 Row 的 id 与整体布局参数。
    ///
    /// 典型用法：`L::new().gap(8.0).main(Align::End).to_props()` —— 间距防粘连、
    /// 主轴 `end` 让动作组贴到行尾（组在父 Row 里时）。`id` 传空串 = 自动生成。
    pub fn row_actions_opts(
        &mut self,
        id: impl Into<String>,
        layout: LayoutProps,
        labels: &[&str],
    ) -> Vec<String> {
        let mut buttons: Vec<String> = Vec::with_capacity(labels.len());
        self.container_opts(Kind::Row, id, layout, |r| {
            for label in labels {
                buttons.push(r.button(*label));
            }
        });
        buttons
    }

    // —— M6 5c：选择类三组（Segmented / ChipGroup / TabBar · **Kind 扩展**）———

    /// 分段选择组：互斥单选，**`Kind::Segmented` + 每标签一个 `button`**（M6 5c）。
    ///
    /// 与 RowActions（组合层）不同，三个选择类组**扩了 `Kind`**，理由（逐条写进
    /// 指南的「语义」）：① 交互层必须在树里认出「这是一组选择」才发得出
    /// `SelectionChanged` 等值事件（组合层没有任何识别通道；给 `NodeProps` 加
    /// 「角色」字段要同步 registry/scene/.dui 三处，代价更大还得多一个字段）；
    /// ② 「选中」是本系统**第一种持久视觉**（hover/pressed/focus 全是瞬态），
    /// 需要绘制侧新的一档状态色 —— 满足「需要新视觉形态才扩 Kind」的判据。
    ///
    /// - 返回**段 id 列表**（顺序与 `labels` 一致；`UiState::segments` 的值、事件里的
    ///   `selected` 用的就是它）；
    /// - 组 id 自动生成（`segmented_N`）；要指定 id / 布局（间距、尺寸）用
    ///   [`Builder::segmented_opts`]；
    /// - 当前选中**不在树里**：App 把初值塞进 `UiState::segments`（组 id → 段 id），
    ///   没塞 = 没有选中段（合法状态）；
    /// - `labels` 为空 ⇒ 一个没有子节点的组（合法；什么也选不了）。
    pub fn segmented(&mut self, labels: &[&str]) -> Vec<String> {
        self.selection_group(Kind::Segmented, "", LayoutProps::default(), labels)
    }

    /// 同 [`Builder::segmented`]，但可指定组 id 与整体布局参数（gap 防粘连是主用途）。
    /// `id` 传空串 = 自动生成。
    pub fn segmented_opts(
        &mut self,
        id: impl Into<String>,
        layout: LayoutProps,
        labels: &[&str],
    ) -> Vec<String> {
        self.selection_group(Kind::Segmented, id, layout, labels)
    }

    /// 标签组：多选，**`Kind::ChipGroup` + 每标签一个 `button`**（M6 5c）。
    /// 每个芯片独立开/关；开关表住 `UiState::chips`（芯片 id → bool，**表里没有 = 关**）。
    /// 其余约定见 [`Builder::segmented`]（Kind 扩展的同一份理由）。
    pub fn chip_group(&mut self, labels: &[&str]) -> Vec<String> {
        self.selection_group(Kind::ChipGroup, "", LayoutProps::default(), labels)
    }

    /// 同 [`Builder::chip_group`]，但可指定组 id 与整体布局参数。`id` 传空串 = 自动生成。
    pub fn chip_group_opts(
        &mut self,
        id: impl Into<String>,
        layout: LayoutProps,
        labels: &[&str],
    ) -> Vec<String> {
        self.selection_group(Kind::ChipGroup, id, layout, labels)
    }

    /// 页签栏：单选页签，**`Kind::TabBar` + 每标签一个 `button`**（M6 5c）。
    /// 点击发 `TabChanged { id, index }`（index = 页签在组**直接子节点**里的树序下标，
    /// **禁用页也一起数**）；内容切换是 App 的事。单个页签禁用 = 该页签自己的
    /// `props.disabled`：本便捷构造不带禁用参数，需要时用
    /// `container_opts(Kind::TabBar, …)` + `button_opts(…, |n| n.props.disabled = true)`
    /// 手写这个组（两条路径产出**结构相等**的树，判据见 `tests/m6_select.rs`）。
    /// 其余约定见 [`Builder::segmented`]（Kind 扩展的同一份理由）。
    pub fn tab_bar(&mut self, labels: &[&str]) -> Vec<String> {
        self.selection_group(Kind::TabBar, "", LayoutProps::default(), labels)
    }

    /// 同 [`Builder::tab_bar`]，但可指定组 id 与整体布局参数。`id` 传空串 = 自动生成。
    pub fn tab_bar_opts(
        &mut self,
        id: impl Into<String>,
        layout: LayoutProps,
        labels: &[&str],
    ) -> Vec<String> {
        self.selection_group(Kind::TabBar, id, layout, labels)
    }

    /// 三个选择类组的**共同构造**：`Kind` 对应的组容器 + 每标签一个 `button`，
    /// 返回子按钮 id 列表。语义差异（单选/多选/页签）在交互层，不在构造层。
    fn selection_group(
        &mut self,
        kind: Kind,
        id: impl Into<String>,
        layout: LayoutProps,
        labels: &[&str],
    ) -> Vec<String> {
        assert!(
            kind.is_selection_group(),
            "selection_group() 只接受 Segmented/ChipGroup/TabBar"
        );
        let mut items: Vec<String> = Vec::with_capacity(labels.len());
        self.container_opts(kind, id, layout, |g| {
            for label in labels {
                items.push(g.button(*label));
            }
        });
        items
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
    /// 流外定位（L1 偏移 / L4 锚定）：`Some(Pos::Offset { .. })` 或
    /// `Some(Pos::Anchors { .. })` ⇒ 脱离流内布局。
    pub position: Option<Pos>,
    /// **每子节点交叉轴对齐**（L2）：`Some` ⇒ 覆盖父容器的 `cross_axis`（仅该子节点）。
    pub cross_self: Option<Align>,
    /// **最小/最大尺寸**（L3）。像素版便捷构造见 [`L::min_w`] 等；百分比用
    /// `L { min_w: Some(Size::Pct(50.0)), .. }` 直设字段。
    pub min_w: Option<Size>,
    pub max_w: Option<Size>,
    pub min_h: Option<Size>,
    pub max_h: Option<Size>,
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
    /// **流外定位**（L1）：相对父内容盒原点的像素偏移，可为负。
    /// 设了它，该节点脱离流内布局（不参与主轴分配、不占流内空间、不计入父固有尺寸）。
    pub fn pos(mut self, x: i32, y: i32) -> L {
        self.position = Some(Pos::Offset { x, y });
        self
    }
    /// **流外锚定**（L4）：四边锚点比例 + 像素修正，同为流外（与 [`L::pos`] 共用
    /// `position` 字段 —— Q5：一个机制，不另起第二套定位）。
    ///
    /// - `l/t/r/b`：父内容盒的**锚点比例**（0.0 = 左/上边、1.0 = 右/下边，可超 `[0,1]`；
    ///   `None` = 该边没有锚）；
    /// - `ox/oy`：**像素**修正，内缩式 —— 起点边（l/t）加、终点边（r/b）减
    ///   （正 = 向内容盒内缩，负 = 向外）；
    /// - 一轴两侧都有锚 ⇒ 该轴尺寸由锚点对导出（显式 w/h 不参与）；只锚一边 ⇒
    ///   显式/固有尺寸；min/max 照常夹取；**父盒子 resize 时锚定边跟随**。
    ///
    /// 语义与数值例子见 `docs/features/anchors.md`。
    pub fn anchors(
        mut self,
        l: Option<f32>,
        t: Option<f32>,
        r: Option<f32>,
        b: Option<f32>,
        ox: i32,
        oy: i32,
    ) -> L {
        self.position = Some(Pos::Anchors { l, t, r, b, ox, oy });
        self
    }
    /// **每子节点交叉轴对齐**（L2）：覆盖父容器的 `cross`，只对这一个子节点生效
    /// （只对**流内**子节点有意义；流外节点不受它影响）。
    pub fn cross_self(mut self, a: Align) -> L {
        self.cross_self = Some(a);
        self
    }
    /// **最小宽度**（L3，像素）：节点宽被托底到它（measure 与 place 两处都生效）。
    pub fn min_w(mut self, v: f32) -> L {
        self.min_w = Some(Size::Px(v));
        self
    }
    /// **最大宽度**（L3，像素）：显式尺寸与 grow 分配结果都被封顶到它。
    pub fn max_w(mut self, v: f32) -> L {
        self.max_w = Some(Size::Px(v));
        self
    }
    /// **最小高度**（L3，像素）。语义同 [`L::min_w`]，作用在高度上。
    pub fn min_h(mut self, v: f32) -> L {
        self.min_h = Some(Size::Px(v));
        self
    }
    /// **最大高度**（L3，像素）。语义同 [`L::max_w`]，作用在高度上。
    pub fn max_h(mut self, v: f32) -> L {
        self.max_h = Some(Size::Px(v));
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
            position: self.position,
            cross_self: self.cross_self,
            min_w: self.min_w,
            max_w: self.max_w,
            min_h: self.min_h,
            max_h: self.max_h,
        }
    }
}

/// 便捷构造 `NodeProps`。
pub fn props(label: Option<&str>, disabled: bool) -> NodeProps {
    NodeProps {
        label: label.map(|s| s.to_string()),
        disabled,
            extra: Default::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RowActions 的**构造判据**：便捷构造出的树必须与「手写 Row + button」
    /// 结构相等（组合层 = 既有控件的等价书写，不产生第二种树形）。
    #[test]
    fn row_actions_builds_exactly_the_hand_written_row_of_buttons() {
        let mut a = Builder::new(Kind::Column, "app");
        let ids = a.row_actions_opts("actions", L::new().gap(8.0).to_props(), &["编辑", "删除"]);

        let mut b = Builder::new(Kind::Column, "app");
        b.container_opts(Kind::Row, "actions", L::new().gap(8.0).to_props(), |r| {
            r.button("编辑");
            r.button("删除");
        });

        let ta = a.build();
        let tb = b.build();
        // 前置断言：返回的 id 必须真的是树里那两个按钮的 id（否则「收 id」收错了人，
        // 后面按 id 关联事件的用法全会静默落空）。
        let row = &ta.children[0];
        assert_eq!(
            ids,
            row.children.iter().map(|c| c.id.clone()).collect::<Vec<_>>(),
            "返回的按钮 id 必须与树里的按钮逐一对上"
        );
        assert!(ta.structurally_eq(&tb), "组合层必须产出与手写等价的树");
    }

    /// 自动 id 规则与两条构筑路径**同一份**（`IdGen` 按 kind 计数 —— B-1 的前提）：
    /// 第一次调用得 `row_1` + `button_1/2`，第二次调用不得撞号。
    #[test]
    fn row_actions_auto_ids_follow_the_kind_counters() {
        let mut app = Builder::new(Kind::Column, "app");
        let first = app.row_actions(&["a", "b"]);
        let second = app.row_actions(&["c"]);
        let tree = app.build();

        assert_eq!(first, vec!["button_1", "button_2"]);
        assert_eq!(second, vec!["button_3"]);
        let rows: Vec<&Node> = tree
            .children
            .iter()
            .filter(|c| c.kind == Kind::Row)
            .collect();
        assert_eq!(
            rows.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
            vec!["row_1", "row_2"],
            "两次便捷构造的 Row 自动 id 不得撞号"
        );
        // 前置断言：标签按序进树（「顺序与 labels 一致」是文档承诺，得真钉住）。
        assert_eq!(rows[0].children[0].props.label.as_deref(), Some("a"));
        assert_eq!(rows[0].children[1].props.label.as_deref(), Some("b"));
        assert_eq!(rows[1].children[0].props.label.as_deref(), Some("c"));
    }

    /// `row_actions_opts` 的参数直通：指定 id 就用指定 id（并进 `IdGen` 占号），
    /// 布局参数原样落在 Row 上（ gap 是防粘连的主用途）。
    #[test]
    fn row_actions_opts_passes_id_and_layout_through() {
        let mut app = Builder::new(Kind::Column, "app");
        let ids =
            app.row_actions_opts("actions", L::new().gap(8.0).to_props(), &["编辑"]);
        let tree = app.build();
        let row = &tree.children[0];
        assert_eq!(row.id, "actions");
        assert_eq!(row.layout.gap, 8.0);
        assert_eq!(ids, vec!["button_1"], "显式 Row id 不影响 button 的自动编号");
        // 占号：之后再自动生成 Row 不得撞上显式名（`IdGen::reserve` 的职责）。
        let mut app2 = Builder::new(Kind::Column, "app");
        let _ = app2.row_actions_opts("actions", L::new().to_props(), &["x"]);
        let again = app2.row_actions(&["y"]);
        let tree2 = app2.build();
        assert_ne!(
            tree2.children[0].id, tree2.children[1].id,
            "显式 id 之后的自动 Row id 不得与显式名冲突"
        );
        assert_eq!(again, vec!["button_2"], "button 计数跨调用连续");
    }

    /// 空标签组：合法（空 Row）。这条钉的是「不 panic、不静默吞容器」。
    #[test]
    fn row_actions_with_no_labels_is_an_empty_row() {
        let mut app = Builder::new(Kind::Column, "app");
        let ids = app.row_actions(&[]);
        let tree = app.build();
        assert!(ids.is_empty());
        assert_eq!(tree.children.len(), 1, "Row 本身还在");
        assert_eq!(tree.children[0].kind, Kind::Row);
        assert!(tree.children[0].children.is_empty());
    }

    // —— M6 5c：选择类三组的构造判据（与 RowActions 同一套纪律）———————

    /// **判据本体**：三个便捷构造产出的树必须与「手写容器 + button」**结构相等**
    /// （Kind 扩展改变的是「组是什么」，不是「怎么建组」—— 两条构筑路径仍同一份规则）。
    #[test]
    fn selection_groups_build_hand_written_equivalent_trees() {
        // 便捷路径
        let mut a = Builder::new(Kind::Column, "app").gap(4.0);
        let seg = a.segmented_opts("seg", L::new().gap(2.0).to_props(), &["日", "周", "月"]);
        let chips = a.chip_group_opts("chips", L::new().gap(6.0).to_props(), &["红", "蓝"]);
        let tabs = a.tab_bar_opts("tabs", L::new().to_props(), &["A", "B"]);
        let ta = a.build();

        // 手写路径（container_opts + button，逐字对应）
        let mut b = Builder::new(Kind::Column, "app").gap(4.0);
        b.container_opts(Kind::Segmented, "seg", L::new().gap(2.0).to_props(), |g| {
            g.button("日");
            g.button("周");
            g.button("月");
        });
        b.container_opts(Kind::ChipGroup, "chips", L::new().gap(6.0).to_props(), |g| {
            g.button("红");
            g.button("蓝");
        });
        b.container_opts(Kind::TabBar, "tabs", L::new().to_props(), |g| {
            g.button("A");
            g.button("B");
        });
        let tb = b.build();

        assert!(ta.structurally_eq(&tb), "便捷构造必须与手写组结构相等");
        // 前置断言：返回的 id 必须真的是组孩子的 id（事件关联与 UiState 键全靠它）。
        for (ids, group) in [(&seg, "seg"), (&chips, "chips"), (&tabs, "tabs")] {
            let node = ta.children.iter().find(|c| c.id == group).unwrap();
            assert_eq!(
                ids,
                &node.children.iter().map(|c| c.id.clone()).collect::<Vec<_>>(),
                "`{group}` 返回的 id 必须与树里的孩子逐一对上"
            );
        }
        // 前置断言：组 Kind 与孩子 Kind 都对（防「便捷构造建错容器」这类低级漂移）。
        assert_eq!(ta.children[0].kind, Kind::Segmented);
        assert_eq!(ta.children[1].kind, Kind::ChipGroup);
        assert_eq!(ta.children[2].kind, Kind::TabBar);
        assert!(
            ta.children
                .iter()
                .all(|g| g.children.iter().all(|c| c.kind == Kind::Button)),
            "组的直接子节点必须是 Button"
        );
    }

    /// 自动 id 规则与两条构筑路径**同一份**（`IdGen` 按 kind 计数）：
    /// 组 id 用新 kind 的计数器（`segmented_1` …），按钮计数跨组连续。
    #[test]
    fn selection_group_auto_ids_follow_the_kind_counters() {
        let mut app = Builder::new(Kind::Column, "app");
        let seg = app.segmented(&["日", "周"]);
        let chips = app.chip_group(&["红"]);
        let tabs = app.tab_bar(&["A"]);
        let again = app.segmented(&["x"]);
        let tree = app.build();

        assert_eq!(seg, vec!["button_1", "button_2"]);
        assert_eq!(chips, vec!["button_3"]);
        assert_eq!(tabs, vec!["button_4"]);
        assert_eq!(again, vec!["button_5"], "按钮计数跨组连续");
        let kinds: Vec<&str> = tree.children.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(
            kinds,
            vec!["segmented_1", "chip_group_1", "tab_bar_1", "segmented_2"],
            "组 id 按 kind 计数，第二次 Segmented 不得撞号"
        );
    }

    /// 组是**横排容器**：布局按 Row 同一套数学落位（孩子 x 递增），
    /// 且吃容器参数（gap 生效）。这是「is_horizontal 收口」的行为判据。
    #[test]
    fn selection_groups_lay_out_horizontally_with_gap() {
        let mut app = Builder::new(Kind::Column, "app");
        app.segmented_opts("seg", L::new().gap(8.0).to_props(), &["日", "周", "月"]);
        let tree = app.build();
        assert!(tree.children[0].layout.gap == 8.0, "前置：opts 的 gap 直通到组");

        let geo = crate::layout::layout(
            &tree,
            crate::node::Rect::new(0.0, 0.0, 300.0, 100.0),
            crate::layout::TextStyle { font_size: 14.0, line_height: 18.0 },
            &crate::layout::ApproxMeasure,
        );
        let xs: Vec<f32> = tree.children[0]
            .children
            .iter()
            .map(|c| geo.get(&c.id).expect("前置：孩子必须有几何").x)
            .collect();
        assert!(
            xs[0] < xs[1] && xs[1] < xs[2],
            "三个段必须横排（x 递增），实际 x = {xs:?}"
        );
        // 间隙判据：x 间距 = 前段宽 + gap（横排数学真的吃到了 gap）。
        let w0 = geo.get(&tree.children[0].children[0].id).unwrap().w;
        assert!(
            (xs[1] - xs[0] - w0 - 8.0).abs() < 0.5,
            "段间距必须是「前段宽 + gap(8)」，实际 {xs:?} w0={w0}"
        );
    }
}
