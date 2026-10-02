# 功能指南：每子节点交叉轴对齐（align-self）

> 状态 ✅ · 示例 `cargo run -p deer-gui --example layout_refine_demo`（与 [min-max-sizes](min-max-sizes.md) 共用）·
> 清单条目见 [`FEATURES.md`](../../FEATURES.md)

## 1. 这是什么 / 什么时候用它

**让一个子节点不跟大队**：容器里大家都按 `cross`（容器级交叉轴对齐）排，
唯独它可以另设一套 —— `LayoutProps::cross_self`（取值与 `cross_axis` 同一类型 `Align`：
start / center / end / stretch）。对齐位置与 stretch 吃满都按它算，**兄弟不受影响**。

什么时候用它：工具条里某个按钮要居中、其它靠上；列表里某一行要拉伸、其它保持固有高 ——
不想为了一个子节点多包一层容器的时候。

什么时候**不**用它：想改的是**整批子节点**的对齐 —— 那是容器级的 `cross`（见
[layout](layout.md)；`cross_self` 只管「这一个」）；想改的是**主轴**位置 ——
那是 `main` / `grow` 的事，`cross_self` 不参与主轴。

**opt-in，最高红线**：不设 `cross_self`（默认 `None`）⇒ 完全回落容器级 `cross_axis` ⇒
既有树逐字节不变。

## 2. 最小示例

```rust
use deer_gui::prelude::*;

let mut app = Builder::new(Kind::Column, "app");
app.container_opts(Kind::Row, "bar", L::new().pad(10.0).w(300.0).h(80.0).to_props(), |r| {
    r.button_opts("普通", |n| n.layout.width = Some(Size::Px(20.0)));            // 跟容器级（默认 Start）
    r.button_opts("居中", |n| {
        n.layout.width = Some(Size::Px(20.0));
        n.layout.cross_self = Some(Align::Center);   // 只这一个居中
    });
    r.button_opts("靠底", |n| {
        n.layout.width = Some(Size::Px(20.0));
        n.layout.cross_self = Some(Align::End);      // 只这一个靠底
    });
});
let tree = app.build();
```

跑完整版（L2 + L3 一起演示，数值断言 + 像素断言，产物在 `render_out/layout_refine_demo.png`）：

```sh
cargo run -p deer-gui --example layout_refine_demo
```

`.dui` 写法（kebab：`cross-self=`，取值同 `cross=`）：

```
[column name=bar w=300 h=80 pad=10]
  [button name=a label=普通 w=20]
  [button name=b label=居中 w=20 cross-self=center]
  [button name=c label=靠底 w=20 cross-self=end]
```

## 3. 完整 API

| 项 | 类型 / 签名 | 含义 |
|---|---|---|
| `LayoutProps::cross_self` | `Option<Align>` | **默认 `None`** = 完全回落容器级 `cross_axis`（既有行为，一个字节都不变） |
| 取值 | `Align::Start / Center / End / Stretch` | 与容器级 `cross_axis` 同一类型、同一语义，只是作用面是**单个流内子节点** |
| `L::cross_self(a)` | `Align -> L` | 便捷构造：`L::new().w(20.0).cross_self(Align::End).to_props()` |
| `.dui` 属性 `cross-self=` | `start/center/end/stretch` | 与 `Align::parse` 共用一份语法；**裸属性（无值）报错**，坏值报错并点名属性 |
| 注册表 `cross_self` | `PropType::Align` · 适用面 ANY | E1 属性注册表的一条（见 [prop-registry](prop-registry.md)） |

**语义细则**（都有测试钉，见 `crates/deer-core/tests/l2_cross_self.rs`）：

- **覆盖，不是合并**：`Some(a)` 时容器级 `cross` 对这个子节点**完全不参与**；
- **只对流内子节点生效**（见第 6 节「做不到什么」——流外 `position` 子节点不看它）；
- **stretch 同样受它管**：`cross_self=stretch` 吃满交叉轴；反过来，容器 `cross=stretch`
  时某子节点设 `cross_self=start` 就**不被拉伸**；
- **显式交叉轴尺寸优先于 stretch**：子节点声明了显式 `h=`（Row 中）/ `w=`（Column 中）时，
  对齐仍按 `cross_self` 算位置，但尺寸保持显式值 —— 这是 I-7「显式 > 分配」的既有优先级，
  与容器级 `cross=stretch` 的行为一致；
- **measure 阶段不受影响**：固有尺寸的聚合与对齐无关（stretch 吃满发生在 place）。

## 4. 自检（怎么确认你真的用对了）

```rust
// ① 覆盖判据：容器 start、子 center ⇒ 只有它在中间，兄弟不动
assert_eq!(geo["b"], Rect::new(30.0, 29.0, 20.0, 22.0), "y = 10 + (60-22)/2");
assert_eq!(geo["a"].y, 10.0, "兄弟不受影响");
// ② 回落判据：cross_self=None ⇒ 跟容器级
assert_eq!(geo["a"].y, 10.0);
// ③ stretch 判据：cross_self=stretch 吃满交叉轴
assert_eq!(geo["d"].h, 60.0);
```

跑 `cargo run -p deer-gui --example layout_refine_demo`，① 段输出矩形数值 +
像素证据（end 对齐的按钮真的画在 end 位置），末行是「全部自检通过 ✅」。

## 5. 常见坑

| 现象 | 原因 | 怎么改 |
|---|---|---|
| 设了 `cross_self=stretch` 但没被拉伸 | 子节点声明了**显式交叉轴尺寸**（Row 里是 `h=`）—— 显式 > 分配（I-7），stretch 只是「分配」的一种 | 去掉那个显式尺寸，或接受「位置按对齐、尺寸按显式」的行为 |
| 主轴方向想错位排列 | `cross_self` 只管**交叉轴**；主轴错位是 `main`（容器级）的事，主轴没有 per-child 对齐 | 用嵌套容器（每个子节点包一层）实现主轴错位 |
| 设在父容器上没效果 | `cross_self` 是**子节点自己的属性**（谁改谁的对齐），容器级用的是 `cross` | 设到要对齐的那个子节点上 |
| `.dui` 里写了 `cross-self`（无值）报错 | 它需要值（同 `cross=`），裸属性写法是 `scroll`/`wrap` 那类开关的惯例 | 写 `cross-self=center` |
| 流外（`pos=`）子节点没对齐 | 流外节点不参与交叉轴对齐（语义边界，见下） | 想要「对齐的流外节点」就自己算偏移，或改用流内布局 |

## 6. 相关

- 布局基础（容器级 `main` / `cross`）：[layout](layout.md)
- 最小/最大尺寸（同一批落地的 L3，示例共用）：[min-max-sizes](min-max-sizes.md)
- 流外定位（`cross_self` 对它不生效的另一方）：[absolute-positioning](absolute-positioning.md)
- 属性注册表（`cross_self` 已登记）：[prop-registry](prop-registry.md)
- **做不到什么**（都如实登记，不要误以为能跑）：
  - **对流外（`position`）子节点不生效**：流外只看自己的显式/固有尺寸 + 偏移，
    不参与任何对齐（`cross_self` 与容器级 `cross` 在这一点上一致）；
  - **主轴没有 per-child 对齐**：只有交叉轴；主轴的逐节点错位要嵌套容器；
  - **不能同时设两套对齐**：它是「覆盖」，没有「在容器级基础上再偏一点」的相对语义；
  - **不参与固有尺寸**：measure 阶段忽略它 —— 容器的固有交叉轴尺寸不因对齐方式而变。

## 7. 检查清单（发布前过一遍）

- [x] 示例能跑：`cargo run -p deer-gui --example layout_refine_demo` → `exit=0`
- [x] 示例有自检断言（矩形数值 + 渲染像素 + `.dui` 往返）
- [x] `FEATURES.md` 已登记（状态 / 指南链接 / 示例命令都对）
- [x] `docs/TUTORIAL.md` 已提及（第 6 章布局段）
- [x] 明确写了「做不到什么」（流外不生效 / 主轴无 per-child / 覆盖非合并 / 不参与固有尺寸）
