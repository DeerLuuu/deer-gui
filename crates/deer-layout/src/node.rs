//! 节点树数据模型 —— deer-gui 的**唯一真相**。
//!
//! 两条构筑路径（命令式声明 / `.tscn` 式场景文件）都产出这个结构，下游的布局、
//! 命中测试、渲染全部只消费它。这是整个运行时的核心不变式：
//!
//! ```text
//!   命令式 API ─┐
//!               ├─→ Node 树 ─→ 布局(纯函数) ─→ 几何 ─→ GPU 后端
//!   场景文件   ─┘                     └─→ 命中测试 ─→ 输入路由
//! ```
//!
//! 设计取舍（与 deer-ui(V0, TypeScript) 一致，因为那部分验证过）：
//! - **树是纯数据**：不含函数/回调。事件用 `id` 关联，见 `deer-gui` 的交互层。
//! - **确定性 id**：`kind_N`，N 是该 kind 在树里的出现序号。两条构筑路径共用同一规则，
//!   否则 V0 的 B-1 缺陷会复现（两棵树结构不等）。

use std::collections::BTreeMap;

/// 节点类型。第一步只做够验证「容器 + 布局 + 交互」的最小集。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Kind {
    /// 竖排容器（子节点从上往下排）
    Column,
    /// 横排容器（子节点从左往右排）
    Row,
    /// 纯文本
    Text,
    /// 按钮
    Button,
    /// 输入框
    Field,
}

impl Kind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Kind::Column => "column",
            Kind::Row => "row",
            Kind::Text => "text",
            Kind::Button => "button",
            Kind::Field => "field",
        }
    }

    pub fn parse(s: &str) -> Option<Kind> {
        Some(match s {
            "column" => Kind::Column,
            "row" => Kind::Row,
            "text" => Kind::Text,
            "button" => Kind::Button,
            "field" => Kind::Field,
            _ => return None,
        })
    }

    /// 容器可含子节点；叶子不可。
    pub const fn is_container(self) -> bool {
        matches!(self, Kind::Column | Kind::Row)
    }
}

/// 主轴 / 交叉轴对齐。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    Start,
    Center,
    End,
    Stretch,
}

impl Align {
    pub fn parse(s: &str) -> Option<Align> {
        Some(match s {
            "start" => Align::Start,
            "center" => Align::Center,
            "end" => Align::End,
            "stretch" => Align::Stretch,
            _ => return None,
        })
    }
}

/// 尺寸：像素或百分比。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Size {
    Px(f32),
    Pct(f32),
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect { x, y, w, h }
    }
}

/// 布局参数。
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct LayoutProps {
    pub width: Option<Size>,
    pub height: Option<Size>,
    pub padding: f32,
    pub gap: f32,
    pub main_axis: Option<Align>,
    pub cross_axis: Option<Align>,
    /// 生长权重（分配剩余空间）。
    pub grow: f32,
}

/// 结构 / 内容参数。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct NodeProps {
    pub label: Option<String>,
    pub disabled: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    pub kind: Kind,
    pub id: String,
    pub layout: LayoutProps,
    pub props: NodeProps,
    pub children: Vec<Node>,
}

impl Node {
    pub fn new(kind: Kind, id: impl Into<String>) -> Node {
        Node {
            kind,
            id: id.into(),
            layout: LayoutProps::default(),
            props: NodeProps::default(),
            children: Vec::new(),
        }
    }

    /// 链式构造（场景文件与测试用；命令式 API 见 `builder`）。
    pub fn with_layout(mut self, l: LayoutProps) -> Node {
        self.layout = l;
        self
    }

    pub fn with_props(mut self, p: NodeProps) -> Node {
        self.props = p;
        self
    }

    /// 只改 id（用于「先挂载、后补自动 id」的构建流程）。
    pub fn with_id(mut self, id: impl Into<String>) -> Node {
        self.id = id.into();
        self
    }

    pub fn with_label(mut self, label: impl Into<String>) -> Node {
        self.props.label = Some(label.into());
        self
    }

    pub fn disabled(mut self) -> Node {
        self.props.disabled = true;
        self
    }

    pub fn push(mut self, child: Node) -> Node {
        self.children.push(child);
        self
    }

    pub fn is_container(&self) -> bool {
        self.kind.is_container()
    }

    /// 前序遍历（父先于子）。
    pub fn walk(&self, f: &mut impl FnMut(&Node, usize), depth: usize) {
        f(self, depth);
        for c in &self.children {
            c.walk(f, depth + 1);
        }
    }

    /// 结构相等：两条构筑路径的产出必须满足（见模块注释的 B-1）。
    pub fn structurally_eq(&self, other: &Node) -> bool {
        self.kind == other.kind
            && self.id == other.id
            && self.props == other.props
            && self.layout == other.layout
            && self.children.len() == other.children.len()
            && self
                .children
                .iter()
                .zip(&other.children)
                .all(|(a, b)| a.structurally_eq(b))
    }
}

/// 确定性 id 生成器：**按 kind** 计数（两条构筑路径共用）。
///
/// **显式命名的节点也要「占号」**（见 [`IdGen::reserve`]）：
/// 否则「自动 id」会撞上同 kind 的显式名字 —— 例如场景文件里手动写了
/// `column_1`，而第一个自动生成的 column 也叫 `column_1`，两棵树立刻不等。
#[derive(Debug, Default, Clone)]
pub struct IdGen {
    counters: BTreeMap<Kind, u32>,
    used: std::collections::BTreeSet<String>,
}

impl IdGen {
    pub fn new() -> IdGen {
        IdGen::default()
    }

    /// 记录一个**显式** id，并推进对应 kind 的计数（若它形如 `kind_N`）。
    ///
    /// 这样自动生成的名字永远不会与显式名字冲突，且两条构筑路径的计数保持同步。
    pub fn reserve(&mut self, id: &str) {
        if !self.used.insert(id.to_string()) {
            return;
        }
        for kind in [Kind::Column, Kind::Row, Kind::Text, Kind::Button, Kind::Field] {
            let prefix = format!("{}_", kind.as_str());
            if let Some(n) = id.strip_prefix(&prefix).and_then(|s| s.parse::<u32>().ok()) {
                let slot = self.counters.entry(kind).or_insert(0);
                if n > *slot {
                    *slot = n;
                }
            }
        }
    }

    /// 取下一个可用 id（跳过已被显式占用的号）。
    pub fn next(&mut self, kind: Kind) -> String {
        loop {
            let n = self.counters.entry(kind).or_insert(0);
            *n += 1;
            let candidate = format!("{}_{}", kind.as_str(), *n);
            if !self.used.contains(&candidate) {
                self.used.insert(candidate.clone());
                return candidate;
            }
        }
    }
}
