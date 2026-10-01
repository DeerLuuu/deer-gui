# 功能指南：命中测试（hit-testing）

> 状态 ✅ · 示例 `cargo run -p deer-gui --example geometry` ·
> 清单条目见 [`FEATURES.md`](../../FEATURES.md)

## 1. 这是什么 / 什么时候用它

**给一个坐标，告诉你命中哪个节点**。这是「鼠标点到谁了」的底层能力。

现在它能做什么、不能做什么要说清：
- ✅ **能**：在几何上算出命中的节点 id（且是**最深**命中者，不会被容器挡住）
- **本页只讲几何命中**；**事件派发、焦点、点击/打字已由 M5 落地**（路由复用本页的 `hit_test`，另加「查裁剪 + 查禁用」），见 [`input.md`](input.md)

所以现在它的用途是：**自己做交互实验**、给未来的输入系统打基础、调试布局。

## 2. 最小示例

```rust
use deer_gui::prelude::*;

let geo = deer_gui::layout_tree(&tree, 360, 200, Theme::default());

// 命中测试：几何表 + 坐标 → 节点
match hit_test(&tree, &geo, 25.0, 45.0) {
    Some(node) => println!("命中 {}", node.id),
    None => println!("这里什么都没有"),
}
```

跑完整版：`cargo run -p deer-gui --example geometry`

## 3. 完整 API

| 函数 | 输入 | 输出 |
|---|---|---|
| `hit_test(&tree, &geo, x, y)` | 树、几何表、坐标（`f32`） | `Option<&Node>` |

**「最深命中者胜出」**：如果一个大容器里包着一个按钮，点在按钮范围内会返回**按钮**，
不是容器。实现上是「后序覆盖」——遍历整棵子树，最后一个命中的（也就是最深的）赢。

## 4. 自检

```rust
let btn = geo["button_1"];

// ① 按钮内部一点必须命中它自己（不是容器）
assert_eq!(
    hit_test(&tree, &geo, btn.x + 2.0, btn.y + 2.0).map(|n| n.id.as_str()),
    Some("button_1")
);

// ② 画布外必须返回 None
assert!(hit_test(&tree, &geo, 9999.0, 9999.0).is_none());

// ③ 禁用节点**几何上仍可命中** —— 「禁用」是交互策略，不是布局属性
//    （所以拦截禁用要写在交互层，别指望 hit_test 帮你挡）
```

## 5. 常见坑

| 现象 | 原因 | 怎么改 |
|---|---|---|
| 以为 `hit_test` 会拦住禁用节点 | 它**只做几何判断**，禁用是交互层的事 | 自己在交互层检查 `node.props.disabled` |
| 点容器内部却返回了容器 | 该位置确实没有子节点覆盖（容器有 `pad` 时四周是空白） | 正常；用几何表确认 |
| 坐标是 `i32` 传不进去 | 参数是 `f32` | 加 `.0` 或用 `as f32` |
| 没有几何表 | `hit_test` 需要先 `layout_tree` | 先算几何 |

## 6. 相关

`hit_test` **本身只看几何**；「禁用 / 裁剪」是 [`input.md`](input.md) 的 `hit()` 在它之上叠的两道判据。

**滚动容器已落地**（[`scroll-and-multiline.md`](scroll-and-multiline.md)）：滚动偏移会改变几何 ⇒ 同一个坐标
在滚动前后命中**不同**的控件。而「视口外的点不命中」由**两道闸**保证：

1. **几何**：滚到容器矩形之外的子节点**根本探不到** —— `hit_test` 只从「矩形包含该点」的祖先往下探；
2. **裁剪**：`ClipSnapshot` 把视口矩形绑成子节点的有效裁剪（护栏里要先断言 `is_known(id)`）。

两道闸在滚动容器上**结论一致、彼此冗余**；**渲染**侧只有裁剪一道（没有它，被滚上去的内容会画到视口外）。

## 6. 相关

- 布局：[`layout.md`](layout.md) · 滚动与换行：[`scroll-and-multiline.md`](scroll-and-multiline.md)
- **做不到**：本页**只**回答「几何上谁被命中」—— 事件派发 / 焦点 / Tab 顺序已在 [`input.md`](input.md) 落地（在那里复用本页判据，并叠加裁剪与禁用）；**仍缺**方向键**上下**导航（焦点在容器内移动；`input.md` 第 6.2 节）—— **左右**方向键已做，那是输入框光标；被视口/裁剪挡住的点**不回退到祖先**（有意语义，见 `input.md` 第 3.2 节）

## 7. 检查清单

- [x] 示例能跑：`cargo run -p deer-gui --example geometry` → `exit=0`
- [x] 示例有自检断言（命中按钮 + 框外无命中）
- [x] `FEATURES.md` 已登记
- [x] `docs/TUTORIAL.md` 已包含
- [x] 明确写了「做不到什么」
