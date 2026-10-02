//! 场景文件解析（`.dui`，类似 Godot 的 `.tscn`）。
//!
//! 为什么是「缩进 + 段头」而不是 JSON/TOML：
//! 1. 与 `.tscn` 同族手感（`[node name="X" type="Y"]`、children 缩进）；
//! 2. **零依赖**（本仓库的硬不变量）；
//! 3. 行式解析能给出**行号**，错误信息可用。
//!
//! 格式：
//! ```text
//! [column name=app pad=12 gap=8]
//!   [text label=标题]
//!   [row gap=8]
//!     [button label=确定]
//!     [button label=取消 disabled]
//! ```
//!
//! **安全性**：纯解析，不执行任何东西（场景文件可能来自数据）。

use std::fmt;

use crate::node::{Align, IdGen, Kind, LayoutProps, Node, NodeProps, Size};

#[derive(Debug, Clone, PartialEq)]
pub struct SceneError {
    pub message: String,
    pub line: usize,
    pub source: String,
}

impl fmt::Display for SceneError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}: {}", self.source, self.line, self.message)
    }
}

impl std::error::Error for SceneError {}

fn err(message: impl Into<String>, line: usize, source: &str) -> SceneError {
    SceneError {
        message: message.into(),
        line,
        source: source.to_string(),
    }
}

struct RawLine {
    indent: usize,
    text: String,
    line: usize,
}

/// 去注释（`#` 起，前面必须有空白或行首；引号内的 `#` 不算）并去空行。
fn preprocess(src: &str, source: &str) -> Result<Vec<RawLine>, SceneError> {
    let mut out = Vec::new();
    for (i, raw) in src.lines().enumerate() {
        let line_no = i + 1;
        let mut line = String::new();
        let mut in_quote = false;
        for ch in raw.chars() {
            if ch == '"' {
                in_quote = !in_quote;
            }
            if ch == '#' && !in_quote && (line.is_empty() || line.ends_with(char::is_whitespace)) {
                break;
            }
            line.push(ch);
        }
        if line.trim().is_empty() {
            continue;
        }
        let indent = line.len() - line.trim_start().len();
        if indent % 2 != 0 {
            return Err(err(
                format!("缩进必须是 2 的倍数，实际 {indent}"),
                line_no,
                source,
            ));
        }
        out.push(RawLine {
            indent,
            text: line.trim().to_string(),
            line: line_no,
        });
    }
    Ok(out)
}

#[derive(Debug, Clone, PartialEq)]
enum AttrVal {
    Str(String),
    Bare,
}

/// 解析属性串：`k=v`、裸 `k`、`k="带 空格"`。
fn parse_attrs(s: &str, line: usize, source: &str) -> Result<Vec<(String, AttrVal)>, SceneError> {
    let chars: Vec<char> = s.chars().collect();
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < chars.len() {
        while i < chars.len() && chars[i].is_whitespace() {
            i += 1;
        }
        if i >= chars.len() {
            break;
        }
        let mut key = String::new();
        while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_' || chars[i] == '-') {
            key.push(chars[i]);
            i += 1;
        }
        if key.is_empty() {
            return Err(err(format!("无法解析属性（位置 {i}）：\"{s}\""), line, source));
        }
        if i < chars.len() && chars[i] == '=' {
            i += 1;
            if i < chars.len() && chars[i] == '"' {
                i += 1;
                let mut val = String::new();
                while i < chars.len() && chars[i] != '"' {
                    val.push(chars[i]);
                    i += 1;
                }
                if i >= chars.len() {
                    return Err(err(format!("字符串未闭合：{key}"), line, source));
                }
                i += 1;
                out.push((key, AttrVal::Str(val)));
            } else {
                let mut val = String::new();
                while i < chars.len() && !chars[i].is_whitespace() {
                    val.push(chars[i]);
                    i += 1;
                }
                out.push((key, AttrVal::Str(val)));
            }
        } else {
            out.push((key, AttrVal::Bare));
        }
    }
    Ok(out)
}

const KNOWN_ATTRS: &[&str] = &[
    "name", "w", "h", "pad", "gap", "main", "cross", "grow", "scroll", "wrap", "label", "disabled",
];

/// 开关属性（裸属性 = 真；**带值就报错**）。
///
/// 为什么带值要报错而不是「看不懂就当假」：`scroll=1` 这种写法看起来生效、实际没生效，
/// 属于 `scene-file.md` 明说的那一类最难查的 bug（**写错了但不生效**）。
/// `disabled` 也走这一条：它以前默默把任何值当假。
fn as_flag(v: Option<&AttrVal>, key: &str, line: usize, source: &str) -> Result<bool, SceneError> {
    match v {
        None => Ok(false),
        Some(AttrVal::Bare) => Ok(true),
        Some(AttrVal::Str(s)) => Err(err(
            format!("{key} 是开关属性，不接受值（写成裸属性 `{key}`），实际 \"{s}\""),
            line,
            source,
        )),
    }
}

fn as_num(v: &AttrVal, key: &str, line: usize, source: &str) -> Result<f32, SceneError> {
    match v {
        AttrVal::Bare => Err(err(format!("{key} 需要一个值"), line, source)),
        AttrVal::Str(s) => s
            .parse::<f32>()
            .map_err(|_| err(format!("{key} 必须是数字，实际 \"{s}\""), line, source)),
    }
}

fn as_size(v: &AttrVal, key: &str, line: usize, source: &str) -> Result<Size, SceneError> {
    match v {
        AttrVal::Bare => Err(err(format!("{key} 需要一个值"), line, source)),
        AttrVal::Str(s) => {
            if let Some(pct) = s.strip_suffix('%') {
                let p = pct
                    .parse::<f32>()
                    .map_err(|_| err(format!("{key} 百分比非法，实际 \"{s}\""), line, source))?;
                Ok(Size::Pct(p))
            } else {
                let p = s
                    .parse::<f32>()
                    .map_err(|_| err(format!("{key} 必须是数字或百分比，实际 \"{s}\""), line, source))?;
                Ok(Size::Px(p))
            }
        }
    }
}

fn as_align(v: &AttrVal, key: &str, line: usize, source: &str) -> Result<Align, SceneError> {
    match v {
        AttrVal::Bare => Err(err(format!("{key} 需要一个值"), line, source)),
        AttrVal::Str(s) => Align::parse(s).ok_or_else(|| {
            err(
                format!("{key} 只能是 start/center/end/stretch，实际 \"{s}\""),
                line,
                source,
            )
        }),
    }
}

struct Open {
    indent: usize,
    /// 该节点在 `root` 里的索引路径（根为空路径）。
    path: Vec<usize>,
}

fn node_at<'a>(root: &'a mut Node, path: &[usize]) -> &'a mut Node {
    let mut cur = root;
    for &i in path {
        cur = &mut cur.children[i];
    }
    cur
}

/// 把 `.dui` 文本解析成节点树。
/// `.dui` 场景格式的**当前版本**。写进文件头的就是它。
///
/// ## 为什么现在就要版本头（而不是等需要时再加）
///
/// 编辑器**一旦开始产出用户文件**，格式就冻结了：以后再改语法，「旧文件在新库里怎么读、
/// 新文件在旧库里怎么读」这两件事都必须有答案。**在有用户文件之前锁格式是最便宜的时刻**
/// —— 之后每一次改动都要带着历史包袱。
///
/// 版本头用 `#` 注释行（`preprocess` 会把它当注释剥掉，所以**必须在本函数里先读**）：
/// ```text
/// # deer-gui-scene: 1
/// [column name=app]
///   [button name=ok text="好"]
/// ```
///
/// **没有头的文件 = v0**（头还没发明出来的那一版）—— 向后兼容是硬要求，
/// 否则仓库里既有的 `.dui` 语料全得改一遍。
pub const SCENE_FORMAT_VERSION: u32 = 1;

/// 版本头的前缀（`: ` 之后是数字）。
const SCENE_HEADER_PREFIX: &str = "# deer-gui-scene:";

/// 读文件头声明的版本。
///
/// 返回 `None` = **没有头**（老文件 ⇒ 按 v0 处理）；
/// `Some(Err(msg))` = **有头但写坏了**（这必须报错，不能当成「没有头」——
/// 静默降级会让一个手滑的版本号变成「读进去了但属性全丢」那种最难查的错）。
fn declared_version(src: &str) -> Option<Result<u32, String>> {
    // 只看**第一个非空行**：头必须在最前面，否则它只是普通注释
    let first = src.lines().find(|l| !l.trim().is_empty())?;
    let t = first.trim();
    let rest = t.strip_prefix(SCENE_HEADER_PREFIX)?;
    let rest = rest.trim();
    Some(
        rest.parse::<u32>()
            .map_err(|_| format!("版本号 `{rest}` 不是数字")),
    )
}

pub fn parse_scene_collect(src: &str, source: &str) -> Result<(Node, Vec<String>), SceneError> {
    // **版本头必须先读**：`preprocess` 会把 `#` 行当注释剥掉，之后就再也看不见它了。
    match declared_version(src) {
        Some(Ok(v)) if v > SCENE_FORMAT_VERSION => {
            return Err(err(
                format!(
                    "场景格式版本 v{v} **高于**本库支持的 v{SCENE_FORMAT_VERSION} \
                     —— 这不是文件坏了，是这个文件用了更晚的格式。请升级 deer-gui，\
                     或用能读懂 v{v} 的版本打开（**不要**手工删掉头，那会读出错的内容）"
                ),
                1,
                source,
            ));
        }
        Some(Err(msg)) => {
            return Err(err(
                format!("场景版本头无法解析：{msg}（期望形如 `{SCENE_HEADER_PREFIX} 1`）"),
                1,
                source,
            ));
        }
        // 没头（老文件）或版本不高于当前 ⇒ 正常解析
        _ => {}
    }

    let lines = preprocess(src, source)?;
    if lines.is_empty() {
        return Err(err("场景为空（至少要有一个根节点）", 1, source));
    }

    let mut ids = IdGen::new();
    // 载入过程中的**非致命**提示（未知属性等）—— 由 `parse_scene_collect` 交出去。
    let mut warnings: Vec<String> = Vec::new();
    let mut open: Vec<Open> = Vec::new();
    let mut root: Option<Node> = None;

    for rl in &lines {
        let line = rl.line;
        let text = &rl.text;
        if !text.starts_with('[') {
            return Err(err(format!("节点必须以 '[' 开头，实际 \"{text}\""), line, source));
        }
        if !text.ends_with(']') {
            return Err(err(format!("节点行必须以 ']' 结尾，实际 \"{text}\""), line, source));
        }
        let inner = text[1..text.len() - 1].trim();
        let (type_name, attrs_str) = match inner.find(char::is_whitespace) {
            Some(i) => (&inner[..i], &inner[i + 1..]),
            None => (inner, ""),
        };
        let kind = Kind::parse(type_name)
            .ok_or_else(|| err(format!("未知节点类型 \"{type_name}\""), line, source))?;
        let attrs = parse_attrs(attrs_str, line, source)?;

        // **未知属性 = 警告 + 结构化保留**（D8 / Q6）。
        //
        // 为什么不再是硬报错：`.dui` 是**用户会手写、编辑器会存回**的文件。
        // 用户在新版里写的属性被旧版读一次就丢掉，那是**数据丢失** ——
        // 而且往往过很久才发现（「我那个设置怎么没了」）。
        // 所以：**保留**（进 `extra`、存盘原样写回）+ **警告**（让人知道本版不认识它）。
        //
        // ⚠️ 只对**未知**属性放宽：**已知**属性写错**仍然硬报错** ——
        // 那类错是「写错了但不生效」，最难查（见 `as_flag` 的说明），必须继续拦。
        let mut extra: std::collections::BTreeMap<String, Option<String>> =
            std::collections::BTreeMap::new();
        for (k, v) in &attrs {
            if !KNOWN_ATTRS.contains(&k.as_str()) {
                // `AttrVal::Str(s)` ⇒ `k=s`；裸属性 ⇒ 只写 key（`None`）。
                // 两者必须分开存：`foo` 与 `foo=""` 不是同一件事（见 `NodeProps::extra`）。
                let val = match v {
                    AttrVal::Str(s) => Some(s.clone()),
                    AttrVal::Bare => None,
                };
                warnings.push(format!(
                    "第 {line} 行：属性 `{k}` 本版不认识 —— 已原样保留并写回，但不会生效"
                ));
                extra.insert(k.clone(), val);
            }
        }
        let get = |k: &str| attrs.iter().find(|(a, _)| a == k).map(|(_, v)| v);

        // id：显式 name 优先（并占号，避免自动 id 撞上它）；否则按 kind 确定性编号
        let id = match get("name") {
            Some(AttrVal::Str(s)) => {
                // **载入时查重**（E2）：同一个文件里两个节点用同一个 `name` 是**静默灾难** ——
                // 几何按 id 查（`geo.get`）、命中按 id 判、事件按 id 路由，
                // 重名会让「后写的那个」悄悄顶掉前者，而**没有任何一处会报错**。
                //
                // 不查的后果具体长这样：两个 `name=ok` 的按钮，只有一个能收到点击，
                // 另一个看起来「点了没反应」—— 而代码里找不到任何 bug。
                if ids.is_used(s) {
                    return Err(err(
                        format!(
                            "id \"{s}\" 重复：同一个场景里每个节点的 `name` 必须唯一 \
                             （重名会让几何/命中/事件按 id 查时张冠李戴，而且不会报错）"
                        ),
                        line,
                        source,
                    ));
                }
                ids.reserve(s);
                s.clone()
            }
            _ => ids.next(kind),
        };

        let mut layout = LayoutProps::default();
        if let Some(v) = get("w") {
            layout.width = Some(as_size(v, "w", line, source)?);
        }
        if let Some(v) = get("h") {
            layout.height = Some(as_size(v, "h", line, source)?);
        }
        if let Some(v) = get("pad") {
            layout.padding = as_num(v, "pad", line, source)?;
        }
        if let Some(v) = get("gap") {
            layout.gap = as_num(v, "gap", line, source)?;
        }
        if let Some(v) = get("main") {
            layout.main_axis = Some(as_align(v, "main", line, source)?);
        }
        if let Some(v) = get("cross") {
            layout.cross_axis = Some(as_align(v, "cross", line, source)?);
        }
        if let Some(v) = get("grow") {
            layout.grow = as_num(v, "grow", line, source)?;
        }
        // 垂直滚动容器（只对 `column` 有意义；`row` 上设了会被布局忽略 —— 与命令式 API 同语义）。
        layout.scroll = as_flag(get("scroll"), "scroll", line, source)?;
        // 文本按宽度换行（只对 `text` 有意义；换行宽度取节点的 `w=`）。
        layout.wrap = as_flag(get("wrap"), "wrap", line, source)?;

        let mut nprops = NodeProps::default();
        if let Some(AttrVal::Str(s)) = get("label") {
            nprops.label = Some(s.clone());
        }
        nprops.disabled = as_flag(get("disabled"), "disabled", line, source)?;
        nprops.extra = extra;

        let node = Node::new(kind, id).with_layout(layout).with_props(nprops);

        // 弹出缩进 >= 当前的祖先，找到真正的父
        while let Some(o) = open.last() {
            if o.indent >= rl.indent {
                open.pop();
            } else {
                break;
            }
        }

        match open.last() {
            None => {
                if root.is_some() {
                    return Err(err("只能有一个根节点（与根同级的节点不允许）", line, source));
                }
                root = Some(node);
                open.push(Open {
                    indent: rl.indent,
                    path: Vec::new(),
                });
            }
            Some(o) => {
                let parent_path = o.path.clone();
                let parent_id;
                let child_index;
                {
                    let root_ref = root.as_mut().expect("根已建立");
                    let parent = node_at(root_ref, &parent_path);
                    if !parent.kind.is_container() {
                        return Err(err(
                            format!("\"{}\" 是叶子节点，不能有子节点", parent.id),
                            line,
                            source,
                        ));
                    }
                    parent_id = parent.id.clone();
                    let _ = parent_id;
                    parent.children.push(node);
                    child_index = parent.children.len() - 1;
                }
                let mut path = parent_path;
                path.push(child_index);
                open.push(Open {
                    indent: rl.indent,
                    path,
                });
            }
        }
    }

    root.map(|r| (r, warnings))
        .ok_or_else(|| err("场景为空", 1, source))
}

/// 解析 `.dui` 场景（**签名与行为不变**，警告丢弃）。
///
/// 要拿警告（例如编辑器提示「本版不认识某个属性」）就用 [`parse_scene_collect`]。
pub fn parse_scene(src: &str, source: &str) -> Result<Node, SceneError> {
    parse_scene_collect(src, source).map(|(node, _warnings)| node)
}

/// 编码回 `.dui` 文本。`parse_scene(encode_scene(t))` 必须与 `t` 结构相等。
pub fn encode_scene(root: &Node) -> String {
    fn emit(n: &Node, depth: usize, out: &mut String) {
        let pad = "  ".repeat(depth);
        let mut attrs = vec![format!("name={}", n.id)];
        let l = &n.layout;
        match l.width {
            Some(Size::Px(v)) => attrs.push(format!("w={}", fmt_num(v))),
            Some(Size::Pct(p)) => attrs.push(format!("w={}%", fmt_num(p))),
            None => {}
        }
        match l.height {
            Some(Size::Px(v)) => attrs.push(format!("h={}", fmt_num(v))),
            Some(Size::Pct(p)) => attrs.push(format!("h={}%", fmt_num(p))),
            None => {}
        }
        if l.padding != 0.0 {
            attrs.push(format!("pad={}", fmt_num(l.padding)));
        }
        if l.gap != 0.0 {
            attrs.push(format!("gap={}", fmt_num(l.gap)));
        }
        if let Some(a) = l.main_axis {
            attrs.push(format!("main={}", align_str(a)));
        }
        if let Some(a) = l.cross_axis {
            attrs.push(format!("cross={}", align_str(a)));
        }
        if l.grow != 0.0 {
            attrs.push(format!("grow={}", fmt_num(l.grow)));
        }
        // 开关属性：**必须编码**，否则 `parse_scene(encode_scene(t))` 会丢掉它们
        // （往返结构相等是 `t10_scene_roundtrip` 的判据）。
        if l.scroll {
            attrs.push("scroll".to_string());
        }
        if l.wrap {
            attrs.push("wrap".to_string());
        }
        if let Some(label) = &n.props.label {
            attrs.push(format!("label={}", quote(label)));
        }
        if n.props.disabled {
            attrs.push("disabled".to_string());
        }
        // **未知属性原样写回**（D8）：写在已知属性**之后**，顺序 = `BTreeMap` 键序
        // ⇒ 同一棵树编码结果逐字节相同（round-trip² 的前提）。
        // `None` = 原本就是**裸属性**，写回去也还是裸的（`foo` 不会变成 `foo=""`）。
        for (k, v) in &n.props.extra {
            match v {
                Some(val) => attrs.push(format!("{k}={}", quote(val))),
                None => attrs.push(k.clone()),
            }
        }
        out.push_str(&format!("{pad}[{} {}]\n", n.kind.as_str(), attrs.join(" ")));
        for c in &n.children {
            emit(c, depth + 1, out);
        }
    }
    // **版本头写在最前面**（`#` 注释行）：它就是「这个文件按哪一版语法写的」的声明。
    // 解析侧会先读它、再解析（`preprocess` 之后这行就没了，所以必须抢在前面）。
    let mut out = format!("{SCENE_HEADER_PREFIX} {SCENE_FORMAT_VERSION}\n");
    emit(root, 0, &mut out);
    out
}

/// 整数就不写小数点，保证往返文本稳定。
fn fmt_num(v: f32) -> String {
    if v.fract() == 0.0 {
        format!("{}", v as i64)
    } else {
        format!("{v}")
    }
}

fn align_str(a: Align) -> &'static str {
    match a {
        Align::Start => "start",
        Align::Center => "center",
        Align::End => "end",
        Align::Stretch => "stretch",
    }
}

fn quote(s: &str) -> String {
    if s.chars()
        .any(|c| c.is_whitespace() || c == '"' || c == '#' || c == '=')
    {
        format!("\"{}\"", s.replace('"', "\\\""))
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod e2_version_header_tests {
    use super::*;

    /// 老文件（**没有头**）：仓库里既有的 `.dui` 语料就是这一种，必须照样能读。
    fn tree() -> Node {
        parse_scene("[column name=app]\n  [button name=ok label=\"好\"]\n", "t.dui")
            .expect("无头的老文件必须能读")
    }

    /// ① 编码出来的文件**第一行就是版本头**。
    #[test]
    fn encode_writes_the_version_header() {
        let out = encode_scene(&tree());
        let first = out.lines().next().unwrap_or_default();
        assert!(
            first.starts_with("# deer-gui-scene:"),
            "第一行应当是版本头，实际 {first:?}"
        );
        assert!(
            first.contains(&SCENE_FORMAT_VERSION.to_string()),
            "头里应当是当前版本 {SCENE_FORMAT_VERSION}：{first:?}"
        );
    }

    /// ② **往返两次逐字节相同**（round-trip²）—— 编辑器反复存盘的稳定性就靠这条。
    #[test]
    fn round_trip_is_stable_twice() {
        let once = encode_scene(&tree());
        let t2 = parse_scene(&once, "t.dui").expect("带头的文件必须能读");
        let twice = encode_scene(&t2);
        assert_eq!(once, twice, "往返两次必须逐字节相同（round-trip²）");
        // 第三次也一样（防止「稳在第二版」这种假稳定）
        let t3 = parse_scene(&twice, "t.dui").expect("再读一次");
        assert_eq!(encode_scene(&t3), twice, "第三次往返也要稳定");
    }

    /// ③ **更高版本 ⇒ 明确说「版本不支持」，不是一堆奇怪的解析错**。
    #[test]
    fn newer_version_is_an_explicit_error() {
        let src = format!(
            "# deer-gui-scene: {}\n[column name=app]\n",
            SCENE_FORMAT_VERSION + 1
        );
        let e = parse_scene(&src, "new.dui").expect_err("更高版本应当报错");
        let m = format!("{e}");
        assert!(
            m.contains("高于") && m.contains("升级"),
            "错误信息要说清是**版本**问题（而不是文件坏了，更不是一堆看不懂的语法错）：{m}"
        );
    }

    /// ④ **头写坏了 ⇒ 明确报错**（不许静默当成「没有头」——
    /// 那会让一个手滑的版本号变成「读进去了但内容不对」）。
    #[test]
    fn malformed_header_is_an_explicit_error() {
        let e = parse_scene("# deer-gui-scene: abc\n[column name=app]\n", "bad.dui")
            .expect_err("坏头应当报错");
        assert!(
            format!("{e}").contains("版本头"),
            "应当指出是版本头的问题：{e}"
        );
    }

    /// ⑤ 头必须在**最前面**：写在中间的 `# deer-gui-scene:` 只是普通注释，不影响解析。
    #[test]
    fn header_must_be_the_first_non_empty_line() {
        let t = parse_scene("[column name=app]\n# deer-gui-scene: 999\n", "mid.dui")
            .expect("中间的注释不该当版本头");
        assert_eq!(t.id, "app");
    }
}

#[cfg(test)]
mod e2_id_uniqueness_tests {
    use super::*;

    /// ① **同一个文件里重名 ⇒ 报错**，且信息里点名是哪个 id。
    #[test]
    fn duplicate_ids_are_rejected_with_the_id_named() {
        let src = "[column name=app]\n  [button name=ok]\n  [button name=ok]\n";
        let e = parse_scene(src, "dup.dui").expect_err("重名必须报错");
        let m = format!("{e}");
        assert!(m.contains("ok"), "错误信息应当点名重复的 id：{m}");
        assert!(m.contains("重复"), "应当说清是重复：{m}");
    }

    /// ② **嵌套层级之间也算重名**（不只是兄弟节点）—— id 是全树唯一的，不是同层唯一。
    #[test]
    fn duplicates_are_caught_across_nesting_levels() {
        let src = "[column name=app]\n  [row name=bar]\n    [button name=app]\n";
        let e = parse_scene(src, "dup2.dui").expect_err("跨层重名也必须报错");
        assert!(format!("{e}").contains("重复"));
    }

    /// ③ **自动 id 不会撞上显式 id**（这正是 `IdGen::reserve` 存在的理由）：
    /// 手写了 `button_1`，后面那个没写名字的按钮应当拿到 **`button_2`**。
    #[test]
    fn auto_ids_skip_explicitly_named_ones() {
        let src = "[column name=app]\n  [button name=button_1]\n  [button]\n";
        let t = parse_scene(src, "auto.dui").expect("能读");
        let ids: Vec<&str> = t.children.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(
            ids,
            vec!["button_1", "button_2"],
            "自动 id 必须**跳过被显式占用的号**（否则两个节点同名 ⇒ 静默张冠李戴）"
        );
    }

    /// ④ 唯一命名的文件照常能读（这条防「查重把正常文件也拦了」）。
    #[test]
    fn unique_ids_still_parse() {
        let src = "[column name=app]\n  [button name=a]\n  [button name=b]\n";
        let t = parse_scene(src, "ok.dui").expect("唯一命名应当能读");
        assert_eq!(t.children.len(), 2);
    }
}

#[cfg(test)]
mod e2_unknown_attrs_tests {
    use super::*;

    /// ① **未知属性：保留 + 警告 + 写回**（这是 D8 的核心三件事，缺一不算）
    #[test]
    fn unknown_attrs_are_warned_kept_and_written_back() {
        let src = "[column name=app future=on]\n";
        let (t, warn) = parse_scene_collect(src, "f.dui").expect("未知属性不该让解析失败");
        assert_eq!(t.props.extra.get("future"), Some(&Some("on".to_string())));
        assert_eq!(warn.len(), 1, "应当有**恰好一条**警告：{warn:?}");
        assert!(warn[0].contains("future") && warn[0].contains("保留"), "{warn:?}");

        let out = encode_scene(&t);
        assert!(out.contains("future=on"), "未知属性必须被**写回**（否则就是数据丢失）：{out}");
    }

    /// ② **裸属性 `foo` 与 `foo=""` 必须区分**（它们在 `AttrVal` 里本来就是两种东西；
    /// 合成一个类型就会在往返里改掉文件内容）
    #[test]
    fn bare_and_empty_valued_attrs_stay_distinct_through_round_trip() {
        let src = "[column name=app flag bareval=\"\"]\n";
        let t = parse_scene(src, "f.dui").expect("能读");
        assert_eq!(t.props.extra.get("flag"), Some(&None), "裸属性应当存成 None");
        assert_eq!(
            t.props.extra.get("bareval"),
            Some(&Some(String::new())),
            "带空值的属性应当存成 Some(\"\")"
        );
        let out = encode_scene(&t);
        let again = parse_scene(&out, "f.dui").expect("再读一次");
        assert_eq!(
            again.props.extra, t.props.extra,
            "往返之后两种形态必须还是两种（否则「存一次就改内容」）：{out}"
        );
    }

    /// ③ **已知属性写错仍然硬报错** —— 只对**未知**属性放宽，别把校验一起放松了。
    #[test]
    fn known_attr_with_a_bad_value_still_errors() {
        // `scroll` 是开关属性：带值就是错（既有哲学）
        let e = parse_scene("[column name=app scroll=1]\n", "bad.dui")
            .expect_err("已知属性的错误写法必须继续报错");
        assert!(format!("{e}").contains("scroll"), "{e}");
        // 未知属性则不报错（同一次对照，证明放宽只针对未知）
        assert!(parse_scene("[column name=app scrollx=1]\n", "ok.dui").is_ok());
    }

    /// ④ 带未知属性的文件**往返两次逐字节相同**（round-trip² 在有 extra 时也成立）
    #[test]
    fn round_trip_is_stable_with_unknown_attrs() {
        let once = encode_scene(&parse_scene("[column name=app z=1 a=2 bare]\n", "f.dui").unwrap());
        let twice = encode_scene(&parse_scene(&once, "f.dui").unwrap());
        assert_eq!(once, twice, "带未知属性时也必须往返稳定：\n{once}\n---\n{twice}");
    }

    /// ⑤ **没有未知属性 ⇒ 逐字节不变**（既有语料的编码不许因为 D8 而变）
    #[test]
    fn no_unknown_attrs_means_byte_identical_output() {
        let out = encode_scene(&parse_scene("[column name=app]\n  [button name=ok]\n", "f.dui").unwrap());
        assert!(
            !out.contains("extra") && !out.contains("None"),
            "不该多写任何东西：{out}"
        );
        assert_eq!(out.lines().count(), 3, "头 + 两行节点：{out}");
    }

    /// ⑥ 老入口 `parse_scene` **签名与行为不变**（警告被丢掉）
    #[test]
    fn old_entry_point_still_works() {
        let t = parse_scene("[column name=app future=on]\n", "f.dui").expect("老入口照常能读");
        assert_eq!(t.id, "app");
    }
}
