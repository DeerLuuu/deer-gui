# 功能指南：命令式 API（imperative-api）

> 状态 ✅ · 示例 `cargo run -p deer-gui --example tutorial` ·
> 清单条目见 [`FEATURES.md`](../../FEATURES.md)

## 1. 这是什么 / 什么时候用它

**在 Rust 代码里描述界面**：调用点即控件，链式写法，不用 JSX、不用组件类。

什么时候用它：界面结构**由代码/数据动态决定**（例如按配置生成一堆按钮）。
什么时候用另一套：界面需要**人工反复调**（用 [`scene-file.md`](scene-file.md) 的 `.dui` 文件更顺手）。

**两者可以混用**——因为它们产出**同一种节点树**，并且可以互相转换（见第 4 节）。

## 2. 最小示例

```rust
use deer_gui::prelude::*;

// ① 建根节点：竖排容器，四周留 12 像素，子元素间距 8 像素
let mut app = Builder::new(Kind::Column, "app").padding(12.0).gap(8.0);

// ② 往里加东西
app.text("标题");                                  // 文本
app.field("请输入");                               // 输入框
app.container_opts(Kind::Row, "bar", L::new().gap(8.0).to_props(), |r| {
    r.button("确定");                              // 按钮
    r.button_opts("禁用", |n| { n.props.disabled = true; }); // 带属性改动的按钮
});

// ③ 定稿成只读的树
let tree = app.build();
```

## 3. 完整 API

### `Builder`

| 方法 | 作用 | 返回 |
|---|---|---|
| `Builder::new(kind, id)` | 建根节点。`kind` 用 `Kind::Column` / `Kind::Row` | `Builder` |
| `Builder::auto(kind)` | 同上，但 id 自动生成 | `Builder` |
| `.padding(f32)` / `.gap(f32)` | 根容器的内边距 / 子元素间距 | `Builder` |
| `.size(Some(Size::Px(w)), Some(Size::Px(h)))` | 根的固定尺寸（也可 `Size::Pct`） | `Builder` |
| `.text(label)` | 加文本 | 它的 id（`String`） |
| `.button(label)` | 加按钮 | 它的 id |
| `.button_opts(label, \|n\| {…})` | 加按钮并改属性（闭包拿到**节点本身**） | 它的 id |
| `.field(label)` | 加输入框 | 它的 id |
| `.container_opts(kind, id, layout, \|b\| {…})` | 加容器（带布局参数），闭包里加的都是它的子节点 | — |
| `.container_auto(kind, \|b\| {…})` | 同上去掉 id | — |
| `.container(kind, id, \|b\| {…})` | 同上，布局用默认值 | — |
| `.mount(node)` | 把一棵现成的子树挂进来（两套写法互通的入口） | `Builder` |
| `.build()` | 定稿成只读的 `Node` 树（**深拷贝**） | `Node` |

### `L`（布局参数构造器）

用 `L::new()` 起手，链式设置，最后 `.to_props()` 交给 `container_opts`：

```rust
L::new().w(200.0).h(40.0).pad(8.0).gap(6.0)
        .main(Align::Center).cross(Align::Stretch).grow(1.0)
        .to_props()
```

各字段含义见 [`layout.md`](layout.md#布局参数layoutprops)。

### `Kind`

| 值 | 含义 | 能有子节点吗 |
|---|---|---|
| `Kind::Column` | 竖排容器（子节点从上往下） | ✅ |
| `Kind::Row` | 横排容器（子节点从左往右） | ✅ |
| `Kind::Text` | 纯文本 | ❌ |
| `Kind::Button` | 按钮 | ❌ |
| `Kind::Field` | 输入框 | ❌ |

### `Node`（只读的树）

| 方法 | 作用 |
|---|---|
| `.walk(&mut \|node, depth\| {…}, 0)` | 前序遍历 |
| `.structurally_eq(&other)` | 结构比较（两套写法用它验证等价） |
| `.is_container()` | 是不是容器 |
| `.children` | 子节点切片 |

## 4. 两条路径等价

命令式与 `.dui` 产出的树**结构完全相等** —— 这是本库的核心不变式（有测试钉住）：

```rust
// 命令式
let mut app = Builder::new(Kind::Column, "app").padding(12.0).gap(8.0);
app.text("标题");
app.container_opts(Kind::Row, "bar", L::new().gap(8.0).to_props(), |r| {
    r.button("确定");
});
let from_code = app.build();

// 同一份界面的场景文件
let scene = "[column name=app pad=12 gap=8]\n  [text label=标题]\n  [row name=bar gap=8]\n    [button label=确定]\n";
let from_file = parse_scene(scene, "ui.dui").unwrap();

assert!(from_code.structurally_eq(&from_file));   // ✅
```

**注意**：要让两者相等，**每个字段都要写对**。最容易漏的是容器的 `gap`（命令式里容易忘写，而场景文件里写了）。示例 `scene_file` 就是这么发现问题的。

也可以把命令式的树转成场景文件文本（用于让人接手改）：

```rust
let text = encode_scene(&tree);
std::fs::write("ui.dui", text)?;
```

## 5. 自检

```rust
// ① 节点数对不对
let mut n = 0;
tree.walk(&mut |_, _| n += 1, 0);
assert_eq!(n, 5, "期望 5 个节点");

// ② 两套写法等价（见上）

// ③ 别忘了渲染出来看看 —— 「树建对了」不等于「画出来对」
let png = deer_gui::render_tree_to_png(&tree, 320, 200, Theme::default())?;
std::fs::write("out.png", png)?;
```

## 6. 常见坑

| 现象 | 原因 | 怎么改 |
|---|---|---|
| `use of moved value: app` | `.padding()` / `.gap()` **按值接收 `self`**，会把 `app` 移动走 | 必须链式：`let mut app = Builder::new(...).padding(12.0);`，**不能分两行** |
| `expected f32, found integer` | `12` 是整数，`12.0` 才是小数 | 小数都带 `.0` |
| 忘了 `.to_props()` | `L::new()...` 返回的是 `L`，`container_opts` 要 `LayoutProps` | 结尾加 `.to_props()` |
| 加进去的东西跑到别处 | `container_opts` 的闭包参数名写错，或用了外面的 `app` 而不是闭包参数 `\|r\|` | 闭包里一定用参数（约定叫 `r` / `c` / `p`） |
| 两套写法结构不相等 | 某个布局字段只写在一侧（最常见：容器的 `gap`） | 用 `structurally_eq` 断言，看哪一层不同 |

## 7. 相关

- 场景文件写法：[`scene-file.md`](scene-file.md)
- 节点树结构：[`node-tree.md`](node-tree.md)
- **做不到**：**没有回调式事件**（事件走 [`input.md`](input.md) 的「`hit`/`handle` + `UiState`」值模型，不是注册回调）、没有「局部更新」（每次都重建整棵树）、没有组件复用/状态（那是 React 的职责，这里刻意没有）

## 8. 检查清单

- [x] 示例能跑：`cargo run -p deer-gui --example tutorial` → `exit=0`
- [x] 示例有自检断言
- [x] `FEATURES.md` 已登记
- [x] `docs/TUTORIAL.md` 已包含
- [x] 明确写了「做不到什么」
