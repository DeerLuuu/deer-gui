# 功能指南：节点树（node-tree）

> 状态 ✅ · 示例 `cargo run -p deer-gui --example tutorial`（第 6 步）·
> 清单条目见 [`FEATURES.md`](../../FEATURES.md)

## 1. 这是什么 / 什么时候用它

**整个运行时的唯一数据模型**。两种构筑方式都产出它，布局/命中测试/渲染都只消费它。

```
  Builder（命令式）─┐
                     ├─→ Node 树 ─→ 布局 → 几何 ─→ 后端
  parse_scene(.dui) ─┘              └─→ 命中测试
```

什么时候直接操作它：
- 想把两种写法**混用**（`Builder::mount(子树)`）；
- 想自己遍历/统计/校验界面结构；
- 想给别的工具（导出器、检查器）喂数据。

## 2. 结构

```rust
pub struct Node {
    pub kind: Kind,          // 节点类型
    pub id: String,          // 唯一标识（布局、命中、事件都靠它对应）
    pub layout: LayoutProps, // 布局参数
    pub props: NodeProps,    // 内容参数（label / disabled）
    pub children: Vec<Node>,
}
```

**它是纯数据**：不含函数、不含回调。事件用 `id` 关联（M5 的交互层负责）。

三个字段类型：

| 类型 | 字段 |
|---|---|
| `LayoutProps` | `width` / `height`（`Option<Size>`）、`padding` / `gap` / `grow`（`f32`）、`main_axis` / `cross_axis`（`Option<Align>`） |
| `NodeProps` | `label: Option<String>`、`disabled: bool` |
| `Size` | `Px(f32)` 或 `Pct(f32)` |

## 3. 树上的方法

| 方法 | 作用 |
|---|---|
| `.walk(&mut \|node, depth\| {…}, 0)` | **前序**遍历（父先于子），`depth` 从 0 开始 |
| `.is_container()` | `Kind::Column` / `Kind::Row` 为真 |
| `.structurally_eq(&other)` | 结构比较（逐字段，含递归子节点） |
| `.children` | 子节点切片 |
| `.kind` / `.id` / `.layout` / `.props` | 字段直取 |

自己建节点时（不经过 Builder）用链式构造器：

```rust
let n = Node::new(Kind::Button, "ok")
    .with_label("确定")
    .with_layout(L::new().w(80.0).to_props());
assert!(!n.id.is_empty());
```

> ⚠️ `with_label` / `with_layout` / `with_props` 都是**整体赋值**。
> 顺序要紧：**先设 layout/props，最后 `with_id`** —— 反过来会被覆盖掉
> （本项目踩过这个坑：`with_props` 把之前 `with_label` 设的 label 抹掉了）。

## 4. id 的规则

| 情况 | id |
|---|---|
| 显式给了（`Builder::new(kind, "app")` / `.dui` 的 `name=app`） | 用给的那个 |
| 没给 | **按类型计数**自动生成：第一个 `button` 是 `button_1`，第二个是 `button_2`… |

**显式命名会「占号」**：如果场景文件里手写了 `column_1`，那自动生成的第一个 column 会跳过 1。
这条规则让**两套写法产出的 id 也一致**（否则树不可能相等）。

## 5. 自检

```rust
// ① 节点数
let mut n = 0;
tree.walk(&mut |_, _| n += 1, 0);
assert_eq!(n, 5);

// ② id 唯一（重复 id 会让几何互相覆盖）
let mut ids = std::collections::BTreeSet::new();
tree.walk(&mut |node, _| assert!(ids.insert(node.id.clone()), "id 重复：{}", node.id), 0);

// ③ 结构与另一套写法等价
assert!(from_code.structurally_eq(&from_file));

// ④ 两套写法混用：把子树挂进别的 Builder
let mut app = Builder::new(Kind::Column, "root");
app.mount(subtree);
```

## 6. 常见坑

| 现象 | 原因 | 怎么改 |
|---|---|---|
| id 重复导致几何被覆盖 | 手写了两处 `name=app`；或显式 id 与自动 id 撞了（会自动跳过，但同名仍是错） | 用上面的「id 唯一」自检 |
| 设了 `label` 但渲染没有 | 建造顺序错：`with_label(...)` 之后又 `with_props(...)` 把 props 整体覆盖了 | 先 `with_props`/`with_layout`，最后 `with_id` |
| `walk` 的 depth 对不上预期 | 它从你传的初始值开始（约定传 `0`） | 传 `0` |
| 想改树做局部更新 | `Node` 可以 clone 后改，但没有「增量更新」机制 | 现在每次都是重建整棵树（渲染是幂等的） |

## 7. 相关

- 命令式建树：[`imperative-api.md`](imperative-api.md)
- 场景文件建树：[`scene-file.md`](scene-file.md)
- **做不到**：增量更新（每次都重建）；节点间引用；事件回调挂在树上（树是纯数据）

## 8. 检查清单

- [x] 示例能跑：`cargo run -p deer-gui --example tutorial` → `exit=0`
- [x] 示例有自检断言（两套写法 `structurally_eq`）
- [x] `FEATURES.md` 已登记
- [x] `docs/TUTORIAL.md` 已包含
- [x] 明确写了「做不到什么」
