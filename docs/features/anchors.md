# 功能指南：anchors 锚定（anchors）

> 状态 ✅ · 示例 `cargo run -p deer-gui --example anchors_demo` ·
> 清单条目见 [`FEATURES.md`](../../FEATURES.md)

## 1. 这是什么 / 什么时候用它

**让一个子节点的边「钉」在父容器内容盒的比例位置上**：`l/t/r/b` 是父内容盒的锚点比例
（0.0 = 左/上边、1.0 = 右/下边），`ox/oy` 是像素修正。它是流外定位 `position` 的**第二种
取值**（`Pos::Anchors`，与 `Pos::Offset` 同一个机制，Q5 裁定），同样脱离流内布局。

**本特性的存在意义是 resize**：父盒子变大，锚定边跟随新盒（`r=1` 的边贴新右缘）、偏移保持
—— `l=0, r=1` 的面板在窗口拉伸时自动撑满，不需要重新布局代码。

什么时候用它：侧栏面板、随窗口缩放的背景、钉在父容器角落的徽标。
什么时候**不**用它：位置固定不跟盒子走的东西用 `pos=x,y`（[绝对定位](absolute-positioning.md)）；
需要父容器跟着变大的用流内布局（`grow` / 尺寸）—— 锚定节点**不计入**父固有尺寸。

**opt-in，最高红线**：不用 anchors 的树，布局、绘制、命中**逐字节不变**。

## 2. 最小示例

```rust
use deer_gui::prelude::*;

// 撑满父内容盒、四边各内缩 8px：父盒子 resize 时自动跟随
n.layout.position = Some(Pos::Anchors {
    l: Some(0.0), t: Some(0.0), r: Some(1.0), b: Some(1.0),
    ox: 8, oy: 8,
});
// 右下角徽标：四边全锚 + 负偏移 ⇒ 跨越内容盒角点居中（显式 w/h 被忽略）
n.layout.position = Some(Pos::Anchors {
    l: Some(1.0), t: Some(1.0), r: Some(1.0), b: Some(1.0),
    ox: -8, oy: -8,
});
```

跑完整版（撑满 + 角标 + resize 前后两组数字断言，产物在 `render_out/anchors_demo.png`）：

```sh
cargo run -p deer-gui --example anchors_demo
```

`.dui` 写法（逐边属性；没写的边 = 没有锚，偏移缺省 0）：

```
[column name=app pad=10]
  [button name=fill label=满 anchor-l=0 anchor-t=0 anchor-r=1 anchor-b=1 anchor-ox=8 anchor-oy=8]
  [button name=pin label=P anchor-r=1 anchor-ox=-6]
```

## 3. 完整 API

| 项 | 类型 / 签名 | 含义 |
|---|---|---|
| `LayoutProps::position` | `Option<Pos>` | 与 `Offset` **同一个字段**：设了 `Anchors` 同样脱离流内（默认 `None` = 流内，一个字节都不变） |
| `Pos::Anchors { l, t, r, b, ox, oy }` | `Option<f32>` × 4 + `i32` × 2 | 四边锚点比例（`None` = 该边无锚）+ 像素修正；锚可超 `[0,1]`（伸出父盒子） |
| `L::anchors(l, t, r, b, ox, oy)` | 构造器 | `L::new().anchors(Some(0.0), None, Some(1.0), None, 0, -6).to_props()` |
| `.dui` 逐边属性 | `anchor-l/t/r/b=<数字>`、`anchor-ox/oy=<整数>` | 任一出现即 `Anchors`；**与 `pos=` 同写报错**（同一个 `position`，二选一） |
| `.dui` 规范形 | `pos=anchors:l,t,r,b,ox,oy` | `Pos::to_attr` 的输出（无锚边写 `-`），与逐边写法同一个机制、可逆解析 |
| 注册表 `position` | `PropType::Pos` | E1 属性注册表同一条（见 [prop-registry](prop-registry.md)） |

**语义细则**（都有测试钉，见 `crates/deer-core/tests/l4_anchors.rs`）：

- **参照矩形 = 父内容盒**（去掉 padding 之后那块，D10 定死，与百分比解析基准一致）；
- **偏移是内缩式**：起点边（l/t）加 `ox`、终点边（r/b）**减** `ox` —— 正 = 向内容盒内缩、
  负 = 向外。这套代数让「撑满内缩」（`l=0,r=1, ox=8` = 四边各缩 8px）与「角标跨越角点」
  （`l=r=1, ox=-8` = 徽标中心在角点上）共用同一对偏移；
- **一轴两侧都有锚 ⇒ 该轴尺寸由锚点对导出**（宽 = `(r−l)×内宽 − 2ox`），显式 `w`/`h`
  **被忽略**（不是报错，D10 定死）；只锚一边（或都不锚）⇒ 显式/固有尺寸（与 `Offset` 同一套）；
- **min/max（L3）照常夹取最终尺寸**；夹取生效时**起点锚保持、终点边让步**；
- **流外共则**：不占流内槽、不计入父固有尺寸、层叠 = 声明序、滚动容器的内容平移同样作用
  （与 `Offset` 完全同一条判据 `is_positioned`）。

## 4. 自检（怎么确认你真的用对了）

```rust
// ① resize 判据：同一棵树、两种盒子 ⇒ 锚定边跟随（r=1 的边贴新右缘）、偏移保持
assert_eq!(geo_small["fill"], Rect::new(18.0, 18.0, 164.0, 64.0));   // 200×100
assert_eq!(geo_big["fill"],   Rect::new(18.0, 18.0, 284.0, 184.0));  // 320×220，右缘都贴内容盒右缘 −8
// ② 内缩判据：撑满 + ox=oy=8 ⇒ 内容盒四边各缩 8px
// ③ 角标判据：四边全锚 + 负偏移 ⇒ 徽标中心 = 内容盒角点
assert_eq!((geo["badge"].x + 8.0, geo["badge"].y + 8.0), (190.0, 90.0));
```

跑 `cargo run -p deer-gui --example anchors_demo`，四段输出全部带断言，末行是
「全部自检通过 ✅」。

## 5. 常见坑

| 现象 | 原因 | 怎么改 |
|---|---|---|
| 设了显式 `w` 但被「无视」 | 一轴两侧都有锚 ⇒ 该轴尺寸由锚点对导出，显式值**被忽略**（D10 定死的规则，防「两种设法静默胜出」） | 想用显式尺寸就只锚一边（如只 `anchor-r=1`） |
| 想钉**右缘**却写了 `anchor-l=1` | `l` 是**左缘**的比例；`anchor-l=1` ⇒ 左缘贴到内容盒右缘、整节点伸出去 | 钉右缘用 `anchor-r=1`（只写它 ⇒ 右缘贴盒、向左退自身尺寸） |
| `r=1, b=0.5` 的节点宽/高是 0 | 锚点对写反（终点 < 起点）⇒ 距离为负，被钳成 0 | 起点/终点别写反；确认要用的是「上-下」「左-右」哪一对 |
| 想让偏移把节点**推出**盒子，设了正 `ox` 却向内缩 | 偏移是内缩式：正 = 向内、负 = 向外（起点边 +、终点边 −） | 向外用负值 |
| 写了 `anchor-ox=8.5` 报错 | 偏移是**整数像素**，小数直接报错（不许静默截断） | 用整数；要亚像素精度另立登记 |
| `pos=0,0` 和 `anchor-*` 同时写报错 | 两者写的是**同一个** `position` 字段，「同时设了谁赢」是无解问题（Q5 合并机制就是为了消掉它） | 二选一 |
| 窗口拉伸时锚定节点没跟着走 | 参照物是**父内容盒**——父容器自己没跟着窗口变大（比如父是固有尺寸） | 把锚定节点的**父链**上某层设成 `100%` / 显式尺寸，让它随盒子伸缩 |

## 6. 相关

- 偏移版流外定位：[absolute-positioning](absolute-positioning.md)（同一 `position` 字段的
  `Offset` 取值；层叠 / 命中 / 滚动行为两边完全一致）
- min/max 夹取（照常作用于锚定尺寸）：[min-max-sizes](min-max-sizes.md)
- 布局基础与 `100%` 尺寸：[layout](layout.md)
- 滚动容器（锚定节点跟着内容平移）：[scroll-and-multiline](scroll-and-multiline.md)
- **做不到什么**（都如实登记，不要误以为能跑）：
  - **数值 z-index 未做**：层叠只有**声明序**（后声明者在上、命中优先），没有数值层级；
  - **相对视口 fixed 未做**：参照物永远是**父容器内容盒**，没有「钉在窗口角落」的语义；
  - **百分比偏移未做**：`ox/oy` 是**像素**（`i32`）—— 锚点是比例、偏移不是；两者别混；
  - **跨层锚定未做**：只锚**直接父**的内容盒，没有「锚到任意祖先」；
  - 被 `min/max` 夹取时终点边**让步**（起点锚保持）—— 没有 «两侧都硬钉、中间挤压内容» 的模式；
  - 被锚出**父矩形之外**的部分**点不到**（`hit_test` 按祖先矩形剪枝，与 `Offset` 同源边界）。

## 7. 检查清单（发布前过一遍）

- [x] 示例能跑：`cargo run -p deer-gui --example anchors_demo` → `exit=0`
- [x] 示例有自检断言（resize 两组数字 / 撑满内缩 / 角标居中 / 显式被忽略 / 像素证据 / `.dui` 往返）
- [x] `FEATURES.md` 已登记（状态 / 指南链接 / 示例命令都对）
- [x] `docs/TUTORIAL.md` 已提及（第 6 章与第 10 章边界表）
- [x] 明确写了「做不到什么」（z-index / 视口 fixed / 百分比偏移 / 跨层锚定 / 父矩形外命中）
