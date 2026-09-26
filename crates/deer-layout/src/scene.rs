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
    "name", "w", "h", "pad", "gap", "main", "cross", "grow", "label", "disabled",
];

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
pub fn parse_scene(src: &str, source: &str) -> Result<Node, SceneError> {
    let lines = preprocess(src, source)?;
    if lines.is_empty() {
        return Err(err("场景为空（至少要有一个根节点）", 1, source));
    }

    let mut ids = IdGen::new();
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

        for (k, _) in &attrs {
            if !KNOWN_ATTRS.contains(&k.as_str()) {
                return Err(err(
                    format!("未知属性 \"{k}\"（可用：{}）", KNOWN_ATTRS.join("/")),
                    line,
                    source,
                ));
            }
        }
        let get = |k: &str| attrs.iter().find(|(a, _)| a == k).map(|(_, v)| v);

        // id：显式 name 优先（并占号，避免自动 id 撞上它）；否则按 kind 确定性编号
        let id = match get("name") {
            Some(AttrVal::Str(s)) => {
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

        let mut nprops = NodeProps::default();
        if let Some(AttrVal::Str(s)) = get("label") {
            nprops.label = Some(s.clone());
        }
        nprops.disabled = matches!(get("disabled"), Some(AttrVal::Bare));

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

    root.ok_or_else(|| err("场景为空", 1, source))
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
        if let Some(label) = &n.props.label {
            attrs.push(format!("label={}", quote(label)));
        }
        if n.props.disabled {
            attrs.push("disabled".to_string());
        }
        out.push_str(&format!("{pad}[{} {}]\n", n.kind.as_str(), attrs.join(" ")));
        for c in &n.children {
            emit(c, depth + 1, out);
        }
    }
    let mut out = String::new();
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
