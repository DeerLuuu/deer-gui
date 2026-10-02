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

/// **流外定位**（L1 · D6/D10）：[`LayoutProps::position`] 的取值。
///
/// **本轮只有 [`Pos::Offset`] 一个变体**（登记口径：anchors 属 L4，本轮不做）。
/// 设了 `position` 的子节点**脱离流内布局** —— 不参与主轴分配、不占流内空间
/// （含间隙）、不计入父容器固有尺寸（measure 阶段同样跳过）；
/// 位置 = **父容器内容盒原点** + 偏移（D10 定死的参照矩形 = 父内容盒，与百分比解析基准一致）。
///
/// **默认 `None` ⇒ 既有树逐字节不变**（opt-in，最高红线；沿用 `scroll` / `wrap` 的先例）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pos {
    /// 相对父容器内容盒原点的**像素**偏移。可为负（节点可以伸出父盒子之外；
    /// 能不能被画到 / 点到由裁剪与命中边界决定，见绝对定位指南的「做不到什么」）。
    Offset { x: i32, y: i32 },
}

impl Pos {
    /// 解析 `.dui` 属性值：`"x,y"`（两个整数，逗号分隔，允许负号）。
    ///
    /// 场景侧与命令侧**共用这一份语法** —— 各写一份迟早漂（同 `Align::parse` 的理由）。
    pub fn parse(s: &str) -> Option<Pos> {
        let (x, y) = s.split_once(',')?;
        Some(Pos::Offset {
            x: x.trim().parse().ok()?,
            y: y.trim().parse().ok()?,
        })
    }

    /// 编码回 `.dui` 属性值（与 [`Pos::parse`] 互逆 ⇒ 往返逐字节稳定）。
    pub fn to_attr(self) -> String {
        match self {
            Pos::Offset { x, y } => format!("{x},{y}"),
        }
    }
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
    /// **流外绝对定位**（L1 · opt-in，最高红线）。取值见 [`Pos`]。
    ///
    /// 语义（三条，都有测试钉，见 `crates/deer-core/tests/l1_position.rs`）：
    /// 1. 设了它的子节点**脱离流内** —— 不参与主轴分配、不占流内空间（含间隙）；
    /// 2. **不计入父容器固有尺寸**（measure 阶段同样跳过）；
    /// 3. 位置 = 父**内容盒**原点 + 偏移；层叠序 = **声明序**（后声明者后画、命中优先）。
    ///
    /// **默认 `None` ⇒ 既有树逐字节不变**。
    pub position: Option<Pos>,
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
    /// **节点上方的注释行**（`.dui` 的 `#` 行，**含 `#` 本身**，按出现顺序）。
    ///
    /// 为什么保留：`.dui` 是**用户会手写、编辑器会存回**的文件 ——
    /// 存一次就把人家的注释吃掉，是**数据丢失**（与 D8 对未知属性的理由同源）。
    ///
    /// **不参与 [`Node::structurally_eq`]**：注释不影响布局/命中/渲染，
    /// 而「两条构筑路径产出结构相等的树」是核心不变式 —— 若注释参与比较，
    /// 一个带注释的 `.dui` 与等价的命令式树就「不相等」了，那条不变式立刻失效。
    pub comments: Vec<String>,
}

impl Node {
    pub fn new(kind: Kind, id: impl Into<String>) -> Node {
        Node {
            kind,
            id: id.into(),
            layout: LayoutProps::default(),
            props: NodeProps::default(),
            children: Vec::new(),
                    comments: Vec::new(),
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

    /// **流外定位的节点**：设了 `layout.position`（L1）。
    ///
    /// 「什么算脱离流内」的**唯一判据** —— measure（固有尺寸跳过）、place（不占流、
    /// 按偏移落位）都读它，免得两处各写一份条件而悄悄分叉（与 `is_scroll_container` 同理）。
    pub fn is_positioned(&self) -> bool {
        self.layout.position.is_some()
    }

    /// 前序遍历（父先于子）。
    pub fn walk(&self, f: &mut impl FnMut(&Node, usize), depth: usize) {
        f(self, depth);
        for c in &self.children {
            c.walk(f, depth + 1);
        }
    }

    /// 结构相等：两条构筑路径的产出必须满足（见模块注释的 B-1）。
    /// **结构相等**：`kind` / `id` / `props` / `layout` / `children` 递归比较。
    ///
    /// ⚠️ **刻意不比 `comments`**：它是文件层的信息，不影响布局/命中/渲染。
    /// 「两条构筑路径（命令式 / `.dui`）产出结构相等的树」是核心不变式 ——
    /// 若注释参与比较，同一个界面「带注释的 `.dui`」与「命令式建的树」就会判不等，
    /// 那条不变式当场失效。注释的往返保真由 `scene.rs` 的往返判据单独负责。
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
