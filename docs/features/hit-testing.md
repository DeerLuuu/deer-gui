# 功能指南：命中测试（hit-testing）

> 状态 ✅ · 示例 `cargo run -p deer-gui --example geometry` ·
> 清单条目见 [`FEATURES.md`](../../FEATURES.md)

## 1. 这是什么 / 什么时候用它

**给一个坐标，告诉你命中哪个节点**。这是「鼠标点到谁了」的底层能力。

现在它能做什么、不能做什么要说清：
- ✅ **能**：在几何上算出命中的节点 id（且是**最深**命中者，不会被容器挡住）
- ❌ **不能**：没有事件派发、没有回调、没有焦点 —— 那些是 M5

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

- 布局：[`layout.md`](layout.md)
- **做不到**：事件派发 / 回调 / 焦点 / Tab 顺序 / 方向键导航（全是 M5）；没有滚动与裁剪对命中的影响（因为还没有滚动容器）

## 7. 检查清单

- [x] 示例能跑：`cargo run -p deer-gui --example geometry` → `exit=0`
- [x] 示例有自检断言（命中按钮 + 框外无命中）
- [x] `FEATURES.md` 已登记
- [x] `docs/TUTORIAL.md` 已包含
- [x] 明确写了「做不到什么」
