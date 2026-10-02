# 功能指南：绝对定位 / 层叠（absolute-positioning）

> 状态 ✅ · 示例 `cargo run -p deer-gui --example overlay_demo` ·
> 清单条目见 [`FEATURES.md`](../../FEATURES.md)

## 1. 这是什么 / 什么时候用它

**让一个子节点脱离流内布局**：它不参与主轴分配、不占流内空间、不计入父容器固有尺寸，
位置 = **父容器内容盒原点 + 像素偏移**（`LayoutProps::position`，取值 `Pos::Offset { x, y }`）。
层叠序 = **声明序**：后声明者后画（在上）且命中优先。

什么时候用它：徽标（角标）、悬浮提示、下拉菜单这类「盖在别人上面、不挤动兄弟」的东西。
什么时候**不**用它：想让父容器跟着它变大变高的时候 —— 流外节点不计入父固有尺寸，
父容器不会为它撑大；那是要流内布局，或者（要跟随父盒子 resize 的）[anchors](anchors.md)。

**opt-in，最高红线**：不设 `position`（默认 `None`）的树，布局、绘制、命中**逐字节不变**。

## 2. 最小示例

```rust
use deer_gui::prelude::*;

let mut app = Builder::new(Kind::Column, "app");
app.container_opts(Kind::Row, "bar", L::new().pad(10.0).to_props(), |r| {
    r.button("普通按钮");                       // 流内：照常排
    r.button_opts("徽标", |n| {
        n.layout.position = Some(Pos::Offset { x: 120, y: 30 }); // 流外：不占槽
    });
});
let tree = app.build();
```

跑完整版（四个演示 + 自检断言，产物在 `render_out/overlay_demo.png`）：

```sh
cargo run -p deer-gui --example overlay_demo
```

`.dui` 写法（`pos=x,y`，整数像素，可为负）：

```
[column name=app]
  [button name=badge label=徽标 pos=120,30]
```

## 3. 完整 API

| 项 | 类型 / 签名 | 含义 |
|---|---|---|
| `LayoutProps::position` | `Option<Pos>` | **默认 `None`** = 参与正常流布局（既有行为，一个字节都不变） |
| `Pos::Offset { x, y }` | `i32` × 2 | 相对**父容器内容盒原点**的像素偏移，可为负（允许伸出父盒子） |
| `L::pos(x, y)` | `(i32, i32) -> L` | 便捷构造：`L::new().pad(10.0).pos(40, -8).to_props()` |
| `.dui` 属性 `pos=` | `pos=x,y` | 与 `Pos::parse` / `Pos::to_attr` 互逆 ⇒ 往返逐字节稳定；**裸 `pos`（无值）报错** |
| 注册表 `position` | `PropType::Pos` | E1 属性注册表的一条（编辑器 Inspector 的数据源，见 [prop-registry](prop-registry.md)） |

**语义细则**（都有测试钉，见 `crates/deer-core/tests/l1_position.rs`）：

- **脱离流内**：不参与主轴分配与对齐、不占流内槽、**不产生间隙**、`grow` 不算它；
- **父固有尺寸不含它**（measure 阶段同样跳过）⇒ 父容器不会为它撑大；
- **自身尺寸**：显式 > 固有（和流内同一套），但**不被父内容盒夹取** —— 流外节点本来就
  允许伸出父盒子；百分比仍相对父内容盒解析；
- **滚动容器里**：内容平移（滚动偏移）同样作用于流外子节点，但 `max_scroll` 不含它；
- **层叠 = 声明序**：与流内/流外无关，同父兄弟间谁后声明谁在上、谁命中优先
  （绘制列表与 `hit_test` 是同一个声明序遍历，天然一致）。

## 4. 自检（怎么确认你真的用对了）

```rust
// ① 流外判据：父固有尺寸不含它（这里 56 = 流内 36 + padding 20，不是 92）
assert_eq!(intr["bar"], (56.0, 42.0), "流外子节点不许计入父容器固有尺寸");
// ② 位置判据：恰好等于「父内容盒原点 + 偏移」
assert_eq!(geo["b"], Rect::new(130.0, 40.0, 36.0, 22.0));
// ③ 层叠判据：重叠点上后声明者胜出
assert_eq!(hit_test(&tree, &geo, 20.0, 20.0).map(|n| n.id.as_str()), Some("badge"));
```

跑 `cargo run -p deer-gui --example overlay_demo`，四段输出全部带断言，末行是
「全部自检通过 ✅」。

## 5. 常见坑

| 现象 | 原因 | 怎么改 |
|---|---|---|
| 父容器没有跟着流外子节点变大 | 这是**语义**：流外节点不计入父固有尺寸 | 需要撑大父容器就用流内布局，或给父容器显式尺寸 |
| 写 `[button pos]`（裸属性）报错 | `pos` 需要值（形如 `pos=10,-20`）—— 开关写法是 `scroll`/`wrap` 那类布尔属性的惯例 | 写 `pos=x,y` |
| `pos=10`（漏了逗号）报错 | 偏移是**一对**整数 | `pos=10,0` |
| 设了 `pos` 但节点还占着流内位置 | 设错了对象（设到了父容器上，不是子节点） | `position` 设在**要脱离流**的那个子节点上 |
| 面板明明画出来了，点它没反应 | 节点被偏移到了**父矩形之外** —— `hit_test` 只从「矩形包含该点」的祖先往下探（与滚动视口同源的已知边界） | 让流外节点至少与父矩形相交，或把它的父链上某层改成能包住它 |
| 两个流外节点重叠，想让「数值大的在上」 | 层叠只有**声明序**，没有数值层级 | 调整声明顺序（把要在上的节点挪到后面声明） |

## 6. 相关

- 布局基础：[layout](layout.md)；命中测试：[hit-testing](hit-testing.md)
- 滚动容器与裁剪（流外节点在滚动容器里的行为见第 3 节）：[scroll-and-multiline](scroll-and-multiline.md)
- 属性注册表（`position` 已登记）：[prop-registry](prop-registry.md)
- **做不到什么**（都如实登记，不要误以为能跑）：
  - **anchors 已落地（L4）**：按比例锚定四边（如 `l=0, r=1` 撑满、resize 跟随）见
    [anchors](anchors.md) —— 它是**同一个机制的第二种取值**（`Pos::Anchors`，Q5 裁定），
    不是第二套定位；本篇的 `Offset` 仍是「位置固定、不跟盒子走」的最短写法；
  - **相对视口 fixed 未做**：偏移的参照物永远是**父容器内容盒**，没有「钉在窗口角落」的语义；
  - **z-index 数值层级未做**：层叠只有**声明序**（后声明者在上、命中优先），没有数值；
  - 被偏移出**父矩形之外**的部分**点不到**（`hit_test` 按祖先矩形剪枝）；能不能「画出来」
    取决于所在裁剪栈（滚动视口外就画不出）；
  - 没有 `cross_self` / 最小最大尺寸（L2/L3 的活）。

## 7. 检查清单（发布前过一遍）

- [x] 示例能跑：`cargo run -p deer-gui --example overlay_demo` → `exit=0`
- [x] 示例有自检断言（固有尺寸 / 偏移数值 / 重叠点像素 / 命中优先 / `.dui` 往返）
- [x] `FEATURES.md` 已登记（状态 / 指南链接 / 示例命令都对）
- [x] `docs/TUTORIAL.md` 已提及（第 6 章与第 10 章边界表）
- [x] 明确写了「做不到什么」（fixed / z-index / 父矩形外命中；anchors 已落地，
      见 [anchors.md](anchors.md)）
