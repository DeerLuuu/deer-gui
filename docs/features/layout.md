# 功能指南：布局（layout）

> 状态 ✅ · 示例 `cargo run -p deer-gui --example geometry` ·
> 清单条目见 [`FEATURES.md`](../../FEATURES.md)

## 1. 这是什么 / 什么时候用它

**布局** = 把「界面树 + 画布大小」算成「每个控件的矩形 `(x, y, 宽, 高)`」。

什么时候用它：
- 调界面尺寸时想**看清**每个控件被算到哪（比改代码猜快得多）；
- 你想自己做命中测试、或者别的渲染后端要消费几何；
- 排查「为什么这个控件位置不对」。

什么时候**不**用它：只想出图的话直接用 `render_tree_to_png`，它内部已经算了布局。

**关键特性**：布局是**纯函数** —— 不改输入树、不读环境、同样的输入永远给同样的输出。
所以它能在没有 GPU、没有浏览器的环境里被完整测试（`crates/deer-core/tests/` 下的断言就是这么来的）。

## 2. 最小示例

```rust
use deer_gui::prelude::*;

let mut app = Builder::new(Kind::Column, "app").padding(16.0).gap(10.0);
app.text("标题");
app.button("确定");
let tree = app.build();

// 第二个参数：画布宽；第三个：画布高；第四个：主题（字号影响文本尺寸）
let geo = deer_gui::layout_tree(&tree, 360, 200, Theme::default());

// geo 是 nodeId → Rect 的表
for (id, r) in &geo {
    println!("{id}: x={} y={} w={} h={}", r.x, r.y, r.w, r.h);
}
```

跑完整版本：`cargo run -p deer-gui --example geometry`

## 3. 完整 API

### 入口

| 函数 | 输入 | 输出 |
|---|---|---|
| `deer_gui::layout_tree(&tree, w, h, theme)` | 树、画布宽高、主题 | `Geometry`（`HashMap<String, Rect>`） |
| `deer_core::layout(&tree, Rect, TextStyle, &Measure)` | 底层版：自己给根盒子与度量 | `Geometry` |
| `deer_core::layout_with_scroll(&tree, Rect, TextStyle, &Measure, &ScrollOffsets)` | 带滚动偏移 | `(Geometry, ScrollMetrics)`（上限表） |

`Rect` 的字段：`x` / `y` / `w` / `h`，都是 `f32`（但**一定是整数**，见下面的不变式）。

### 布局参数（`LayoutProps`）

命令式 API 用 `L::new()` 构造，`.dui` 场景文件用属性名：

| 作用 | 命令式 | `.dui` 属性 | 说明 |
|---|---|---|---|
| 内边距 | `.pad(10.0)` | `pad=10` | 容器四周留白；**也只有给了它容器才画底色** |
| 子元素间距 | `.gap(8.0)` | `gap=8` | 相邻子节点之间的距离 |
| 固定宽/高 | `.w(200.0)` / `.h(40.0)` | `w=200` / `h=40` | 像素 |
| 百分比尺寸 | `L { width: Some(Size::Pct(50.0)), .. }` | `w=50%` | 相对**父内容盒** |
| 主轴分配权重 | `.grow(1.0)` | `grow=1` | 剩余空间按权重分 |
| 主轴对齐 | `.main(Align::Center)` | `main=center` | `start`/`center`/`end`/`stretch` |
| 交叉轴对齐 | `.cross(Align::Stretch)` | `cross=stretch` | 同上（容器级，管全体子节点） |
| **每子节点交叉轴对齐** | `.cross_self(Align::End)` | `cross-self=end` | **覆盖**容器级 `cross`，只对这一个流内子节点生效；流外不生效，见 [`align-self.md`](align-self.md) |
| **最小/最大尺寸** | `.min_w(80.0)` / `.max_w(200.0)` / `.min_h(v)` / `.max_h(v)` | `min-w=80` / `max-w=200` / `min-h=` / `max-h=` | min 下限、max 上限，measure 与 place 两处都夹（固有聚合 / 显式 / grow/stretch 都过 `[min,max]`）；`min > max` ⇒ min 赢，见 [`min-max-sizes.md`](min-max-sizes.md) |
| **垂直滚动容器** | `.scroll(true)` | `scroll`（裸属性） | 只对 `column` 有意义 ⇒ `max_scroll` + 视口裁剪，见 [`scroll-and-multiline.md`](scroll-and-multiline.md) |
| **文本换行** | `n.layout.wrap = true` | `wrap`（裸属性） | 只对 `text` 有意义；换行宽度 = 节点自己的 `w=` |

> **主轴 vs 交叉轴**：`Row` 的主轴是水平（左→右）、交叉轴是垂直；`Column` 反过来。

## 4. 自检

布局的不变式（都有对应测试，你也可以在自己的代码里断言）：

```rust
// ① 几何一定是整数（避免累积小数误差，也让快照稳定）
for (id, r) in &geo {
    assert!(r.x.fract() == 0.0 && r.w.fract() == 0.0, "{id} 的几何不是整数");
}

// ② 根不撑满画布（除非根自己声明了 w/h）——
//    「宿主给的盒子是上限，不是命令」，这是运行时**不假设拥有窗口**的体现
let root = geo["app"];
assert!(root.w <= 360.0);

// ③ 确定性：同样的输入必须给逐位相同的输出
let a = deer_gui::layout_tree(&tree, 360, 200, Theme::default());
let b = deer_gui::layout_tree(&tree, 360, 200, Theme::default());
assert_eq!(a, b);
```

## 5. 常见坑

| 现象 | 原因 | 怎么改 |
|---|---|---|
| **容器宽度是子节点之和**，出乎意料地宽 | 主轴方向是对的（`Row` 的主轴尺寸 = 子节点之和），但你可能是想知道**交叉轴**：`Column` 的宽度 = `max(子宽)` | 确认容器的方向。`Row` = 横排 ⇒ 宽是 sum；`Column` = 竖排 ⇒ 宽是 max |
| 控件被挤成 0 尺寸 | 画布太小，或父容器分配不到空间（`grow` 为 0 且没有显式尺寸） | 用 `layout_tree` 打印几何看是哪一层被压扁了 |
| 改了 `pad` 但没看到底色变化 | 容器**只有给了 `pad` 才画底色**（刻意的设计，否则整屏都是方块） | 给该容器加 `pad`，或改 `DefaultRenderer`（自定义渲染） |
| `50%` 算出来不是预期值 | 百分比是相对**父的内容盒**（即扣掉父的 `pad` 之后），不是相对画布 | 确认父的 `pad`；必要时直接给像素值 |
| `center` / `end` 看起来没生效 | 如果子节点有 `grow`，剩余空间会被 `grow` 吃光 ⇒ 没有「剩余」可居中 | 去掉 `grow`，或改用 `<容器> main=center` 而不给子节点 `grow` |
| 设了 `scroll` 但子节点还是被压进视口 | 只有**滚动容器自己**的子节点不被视口夹取；更深层的容器照旧按 flex 分配 | 把内容直接放进滚动容器；详见 [`scroll-and-multiline.md`](scroll-and-multiline.md) 第 5 节 |
| 设了 `wrap` 却还是一行 | 换行宽度取节点的**像素宽度**（`w=<像素>`）；百分比宽度在测量阶段还不知道 | 给 `w=` 像素值 |

## 6. 相关

- 命中测试：[`hit-testing.md`](hit-testing.md)（坐标 → 控件）
- 主题（字号影响文本尺寸）：[`theme.md`](theme.md)
- **滚动容器 + 多行文本**：[`scroll-and-multiline.md`](scroll-and-multiline.md)（`scroll` / `wrap`
  两个开关、`max_scroll` 的口径、换行宽度取哪里）
- 内部原理：[`../M1-report.md`](../M1-report.md)（含 8 条布局不变量与踩过的坑）
- **做不到**：没有绝对定位、没有 z-index 层叠、**没有水平滚动**（`row` 上的 `scroll` 被忽略）、
  没有网格/表格布局。多行文本与垂直滚动容器**已有**（见
  [`scroll-and-multiline.md`](scroll-and-multiline.md)），但都是 opt-in 的开关。

## 7. 检查清单

- [x] 示例能跑：`cargo run -p deer-gui --example geometry` → `exit=0`
- [x] 示例有自检断言（几何数量 + 命中正确 + 框外无命中）
- [x] `FEATURES.md` 已登记
- [x] `docs/TUTORIAL.md` 已包含布局章节
- [x] 明确写了「做不到什么」
