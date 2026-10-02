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
    /// **垂直滚动容器**（只对 `Column` 有意义；`Row` 上设了会被**忽略** —— 本期只做垂直滚动）。
    ///
    /// 语义：内容主轴尺寸不再按视口夹取，`max_scroll = max(0, 内容高 − 视口高)`，
    /// 子节点整体位移 `-滚动偏移`，视口外的内容由渲染器的裁剪栈挡掉。
    /// **默认 `false`** ⇒ 既有语料一个像素都不变。
    pub scroll: bool,
    /// **文本按宽度换行**（只对 `Kind::Text` 有意义）。
    ///
    /// 换行宽度 = 节点自己的**像素**宽度（`width: Some(Size::Px(..))`）—— 测量阶段
    /// 只知道这一个宽度。**默认 `false`** ⇒ 既有语料一行、一个字节都不变（opt-in）。
    pub wrap: bool,
}

/// 结构 / 内容参数。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct NodeProps {
    pub label: Option<String>,
    pub disabled: bool,
    /// **未知属性的原样保留**（D8 / E2 / Q6）：`.dui` 载入时遇到当前版本**不认识**的属性，
    /// 不丢弃、也不硬报错，而是原样存这里、存盘时再写回去。
    ///
    /// 为什么要有它：编辑器往返**不能默默吃掉**用户文件里的未来字段 ——
    /// 用户在新版里写的东西被旧版存一次就没了，那是**数据丢失**（而受害者往往过很久才发现）。
    ///
    /// - **值是 `Option<String>`**：`Some(v)` = `k=v`；`None` = **裸属性 `k`**。
    ///   两者**不是同一件事**（`foo` vs `foo=""`）⇒ 合成一个 `String` 就再也分不出来，
    ///   往返会**改掉文件内容**；
    /// - **`BTreeMap` 而非 `HashMap`**：键序确定 ⇒ 编码逐字节可复现（round-trip² 的前提）。
    pub extra: std::collections::BTreeMap<String, Option<String>>,
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

    /// **垂直可滚动容器**：`Column` + `layout.scroll`。
    ///
    /// 这是「什么算可滚动容器」的**唯一判据** —— 布局（不夹取子节点主轴、算 `max_scroll`）、
    /// 渲染（在视口上推裁剪栈）、交互（滚轮找目标容器）都用它，免得三处各写一份条件而在
    /// 某处悄悄分叉。`Row` 上的 `scroll` 被**忽略**（本期只做垂直滚动；这是被测试钉住的行为）。
    pub fn is_scroll_container(&self) -> bool {
        self.layout.scroll && self.kind == Kind::Column
    }

    /// **需要换行的文本节点**：`Text` + `layout.wrap`。
    pub fn wraps_text(&self) -> bool {
        self.layout.wrap && self.kind == Kind::Text
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
    /// 这个 id 是否**已经被别的节点占了**。
    ///
    /// 只读、不改状态 —— 给「载入时查重」用。为什么不直接改 `reserve` 的返回值：
    /// 那是一次**签名变更**，而这里只需要一个查询；查询与占号分开，语义也更清楚
    /// （`reserve` 是「占号」，不是「问号」）。
    pub fn is_used(&self, id: &str) -> bool {
        self.used.contains(id)
    }

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
