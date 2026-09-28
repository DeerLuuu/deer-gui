# 功能指南：`.dui` 场景文件（scene-file）

> 状态 ✅ · 示例 `cargo run -p deer-gui --example scene_file` ·
> 清单条目见 [`FEATURES.md`](../../FEATURES.md)

## 1. 这是什么 / 什么时候用它

**用文本文件描述界面**，语法像 Godot 的 `.tscn`：缩进表示层级、方括号里第一项是类型。

什么时候用它：界面需要**人工反复调**（改一个数字就能重跑看效果），
或者你想让**不懂 Rust 的人**也能改界面。
什么时候用另一套：界面由代码/数据动态生成（用 [`imperative-api.md`](imperative-api.md)）。

**两者产出结构相等的树**，可以互相转换，也可以混用。

## 2. 最小示例

场景文件 `ui.dui`：

```
# 注释用 # 开头
[column name=app pad=12 gap=8]
  [text label=标题]
  [field label=请输入]
  [row name=bar gap=8]
    [button label=确定]
    [button label=取消 disabled]
```

代码：

```rust
use deer_gui::prelude::*;

let text = std::fs::read_to_string("ui.dui")?;
let tree = parse_scene(&text, "ui.dui")?;    // 第二个参数只用于报错时显示文件名
```

跑完整版：`cargo run -p deer-gui --example scene_file`
（它会把场景文件写到 `render_out/settings.dui`，你可以直接改那个文件再重跑）

## 3. 语法与完整属性表

### 四条语法规则

| 规则 | 说明 |
|---|---|
| `[类型 属性…]` | 一行一个节点。类型：`column` / `row` / `text` / `button` / `field` |
| **缩进 2 空格** | 表示「是上一节点的子节点」。必须是 **2 的倍数** |
| `key=value` | 属性。值含空格/`#`/`=` 时加双引号：`label="确定 或 取消"` |
| `#` 开头 | 注释（行首，或前面有空白）。引号内的 `#` 不算注释 |

### 属性表

| 属性 | 类型 | 含义 |
|---|---|---|
| `name=` | 字符串 | 节点 id。不写则自动生成（按类型计数：`button_1`、`button_2`…） |
| `label=` | 字符串 | 显示文本（`text`/`button`/`field`） |
| `disabled` | 裸标记 | 写一个词就表示禁用：`[button label=删除 disabled]` |
| `pad=` | 数字 | 内边距。**只有给了它容器才画底色** |
| `gap=` | 数字 | 子元素间距 |
| `w=` / `h=` | 数字或百分比 | 固定尺寸：`w=200` 或 `w=50%`（相对父内容盒） |
| `grow=` | 数字 | 主轴分配权重，剩余空间按权重分 |
| `main=` | `start`/`center`/`end`/`stretch` | 主轴对齐 |
| `cross=` | 同上 | 交叉轴对齐 |
| `scroll` | 裸标记 | **垂直滚动容器**（只对 `column` 有意义）：`[column name=list w=200 h=120 scroll]` |
| `wrap` | 裸标记 | **文本按宽度换行**（只对 `text` 有意义，换行宽度取节点的 `w=`）：`[text w=120 wrap label="…"]` |

> 写**未知属性会被报错**（不静默忽略）——因为「场景写错了但不生效」是最难查的一类 bug。
> **开关属性（`disabled` / `scroll` / `wrap`）带值也会报错**（`scroll=1` 这种写法看起来生效、
> 实际不生效，属于同一类 bug）：写裸属性 `scroll`，不要写 `scroll=1`。
> 滚动与换行的完整语义见 [`scroll-and-multiline.md`](scroll-and-multiline.md)。

## 4. 报错带行号

```rust
match parse_scene("[column name=a]\n  [button label=确定 nope=1]\n", "bad.dui") {
    Err(e) => println!("{e}"),
    Ok(_) => {}
}
// 输出：bad.dui:2: 未知属性 "nope"（可用：name/w/h/pad/gap/main/cross/grow/scroll/wrap/label/disabled）
```

`SceneError` 的字段：`message` / `line` / `source`。`Display` 形态是 `文件名:行号: 消息`，
所以多数编辑器的错误跳转能直接用。

## 5. 往返与互转

```rust
// 树 → 场景文件文本（让人接手改）
let text = encode_scene(&tree);
std::fs::write("ui.dui", text)?;

// 往返稳定：parse(encode(t)) ≡ t
let back = parse_scene(&encode_scene(&tree), "rt.dui")?;
assert!(tree.structurally_eq(&back));
```

## 6. 自检

```rust
let tree = parse_scene(text, "ui.dui")?;

// ① 节点数
let mut n = 0;
tree.walk(&mut |_, _| n += 1, 0);
assert!(n >= 5);

// ② 往返稳定（见上）

// ③ 与代码写法等价（最有价值的自检）
let mut app = Builder::new(Kind::Column, "app").padding(12.0).gap(8.0);
app.text("标题");
/* …照场景文件写一遍… */
assert!(app.build().structurally_eq(&tree), "两套写法必须等价");
```

## 7. 常见坑

| 现象 | 原因 | 怎么改 |
|---|---|---|
| `缩进必须是 2 的倍数` | 用了 Tab 或 3 个空格 | 统一用 2 空格 |
| `只能有一个根节点` | 顶层写了两个节点 | 外面套一个 `[column]` |
| `"x" 是叶子节点，不能有子节点` | 给 `text`/`button`/`field` 缩进了子节点 | 只有 `column`/`row` 能有子节点 |
| 两套写法不等价 | 常见是容器 `gap` 只写在场景文件里 | 用 `structurally_eq` 定位到具体是哪一层 |
| 改了 `.dui` 没反应 | 程序启动时读一次文件 | 重新 `cargo run` |
| 值里有空格导致解析怪怪的 | 没加引号 | `label="确定 或 取消"` |

## 8. 相关

- 命令式写法：[`imperative-api.md`](imperative-api.md)
- **做不到**：与 Godot `.tscn` **不兼容**（只是同族手感）；没有 `%ExtResource`/脚本引用；没有继承/复用机制

## 9. 检查清单

- [x] 示例能跑：`cargo run -p deer-gui --example scene_file` → `exit=0`
- [x] 示例有自检断言（往返一致 + 与代码写法等价 + 报错可读）
- [x] `FEATURES.md` 已登记
- [x] `docs/TUTORIAL.md` 已包含
- [x] 明确写了「做不到什么」
