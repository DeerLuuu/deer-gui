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
| `min-w=` / `max-w=` / `min-h=` / `max-h=` | 数字或百分比 | **最小/最大尺寸**（L3）：min 下限、max 上限，测固有尺寸与落位**两处都夹**；`min > max` ⇒ min 赢。见 [min-max-sizes](min-max-sizes.md) |
| `grow=` | 数字 | 主轴分配权重，剩余空间按权重分 |
| `main=` | `start`/`center`/`end`/`stretch` | 主轴对齐 |
| `cross=` | 同上 | 交叉轴对齐（容器级，管全体子节点） |
| `cross-self=` | 同上 | **每子节点交叉轴对齐**（L2）：覆盖容器级 `cross`，只对这一个流内子节点生效。见 [align-self](align-self.md) |
| `scroll` | 裸标记 | **垂直滚动容器**（只对 `column` 有意义）：`[column name=list w=200 h=120 scroll]` |
| `wrap` | 裸标记 | **文本按宽度换行**（只对 `text` 有意义，换行宽度取节点的 `w=`）：`[text w=120 wrap label="…"]` |
| `pos=` | `x,y`（整数，可为负） | **流外绝对定位**（L1）：脱离流内布局，位置 = 父内容盒原点 + 偏移；层叠 = 声明序。见 [absolute-positioning](absolute-positioning.md) |
| `anchor-l=` / `anchor-t=` / `anchor-r=` / `anchor-b=` | 数字（比例，可缺省） | **流外锚定**（L4）：父内容盒的锚点比例（0=左/上边、1=右/下边，可超 [0,1]）；没写的边 = 没有锚。一轴两侧都有锚 ⇒ 该轴尺寸由锚点对导出（显式 `w=`/`h=` 被忽略）。**与 `pos=` 同写报错**（同一个 `position`）。见 [anchors](anchors.md) |
| `anchor-ox=` / `anchor-oy=` | 整数（像素） | 锚定的像素修正（L4）：内缩式 —— 起点边（l/t）加、终点边（r/b）减，正 = 向内、负 = 向外。也可写规范形 `pos=anchors:l,t,r,b,ox,oy`（无锚边写 `-`），两者可逆互转 |

> 写**未知属性（**D8 起改为「警告 + 原样保留」**，不再报错）会被报错**（不静默忽略）——因为「场景写错了但不生效」是最难查的一类 bug。
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

- **`name` 重名会报错**（E2 起）：同一个场景里每个节点的 id 必须**全树唯一**，
  不只同层唯一。重名以前是**静默灾难** —— 几何按 id 查、命中按 id 判、事件按 id 路由，
  后写的会悄悄顶掉前者，表现是「某个按钮点了没反应」而代码里找不到 bug。
  手写了 `button_1`、后面再写一个不带名字的按钮时，自动编号会**跳过** `button_1` 给到 `button_2`。
- **版本头不要手工删**：文件第一行的 `# deer-gui-scene: N` 是格式声明。删掉它不会「修好」什么，
  只会让一个用了更晚格式的文件被当成老格式读（见 [`../features/scene-file.md`](scene-file.md) 第 4 节）。

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

> **未知属性会被保留**（D8 / E2 / Q6）：本版**不认识**的属性不再报错，而是**原样存起来、存盘时写回**，
> 并在载入时给一条**警告**（用 `parse_scene_collect` 取）。理由：编辑器往返不能默默吃掉用户文件里的
> 未来字段。注意**已知属性写错仍然硬报错** —— 放宽只针对「本版不认识」这一种。

> **注释会被保留**（D9）：写在节点**上方**的 `#` 注释与**行内**注释都会跟着那个节点存回原处
> （缩进与节点一致）。**已知限制**：文件**末尾**（最后一个节点之后）的注释没有可挂的节点，**会丢**
> —— 这条如实登记。另：注释**不参与**结构相等（它不影响布局/命中/渲染），
> 所以「命令式建的树」与「带注释的 `.dui` 树」仍然结构相等。
