# 功能指南：最小/最大尺寸（min-max-sizes）

> 状态 ✅ · 示例 `cargo run -p deer-gui --example layout_refine_demo`（与 [align-self](align-self.md) 共用）·
> 清单条目见 [`FEATURES.md`](../../FEATURES.md)

## 1. 这是什么 / 什么时候用它

**给尺寸上夹具**：`min_w` / `max_w` / `min_h` / `max_h`（`Option<Size>`，与 `w`/`h` 同型，
支持像素与百分比）。**min 是下限、max 是上限**，同时在两处生效：

1. **measure（测固有尺寸）**：固有尺寸先算，再夹进 `[min, max]` —— 容器的固有尺寸聚合
   （主轴求和 / 交叉轴取最大）用的是**夹过**的值；
2. **place（落位）**：显式尺寸、grow 分配结果、stretch 吃满的结果同样被夹 ——
   兄弟的落位、`max_scroll` 全部基于**夹过**的尺寸。

什么时候用它：按钮不许被压扁（`min_w`）、文本列不许无限变宽（`max_w`）、
grow 分区要有下限保证可用性 —— 「布局算出来多大都行，但别小于/大于这个数」的时候。

什么时候**不**用它：想直接定尺寸 —— 那是 `w`/`h`；min/max 是**修复**不是**来源**，
它不参与「显式 vs 固有」的取舍（见第 3 节的顺序）。

**opt-in，最高红线**：四个都未设 ⇒ 既有树逐字节不变。

## 2. 最小示例

```rust
use deer_gui::prelude::*;

let mut app = Builder::new(Kind::Column, "app");
app.container_opts(Kind::Row, "bar", L::new().w(300.0).h(40.0).to_props(), |r| {
    r.button_opts("A", |n| {
        n.layout.grow = 1.0;
        n.layout.min_w = Some(Size::Px(80.0));  // grow 分得再少也至少 80
        n.layout.max_w = Some(Size::Px(120.0)); // 分得再多也不超过 120
    });
});
let tree = app.build();
```

跑完整版（L2 + L3 一起演示，数值断言 + 像素断言，产物在 `render_out/layout_refine_demo.png`）：

```sh
cargo run -p deer-gui --example layout_refine_demo
```

`.dui` 写法（kebab：`min-w=` / `max-w=` / `min-h=` / `max-h=`，数字或百分比）：

```
[column name=bar w=300 h=40]
  [button name=a label=A grow=1 min-w=80 max-w=120]
  [text name=t label=长文本 max-w=200]
```

## 3. 完整 API

| 项 | 类型 / 签名 | 含义 |
|---|---|---|
| `LayoutProps::min_w` / `max_w` | `Option<Size>` | 宽度的下限 / 上限；**默认 `None`** = 不夹（既有行为） |
| `LayoutProps::min_h` / `max_h` | `Option<Size>` | 高度的下限 / 上限；语义同上 |
| `Size` | `Px(f32)` / `Pct(f32)` | 百分比相对**父内容盒**解析（与 `w`/`h` 同一基准） |
| `L::min_w(v)` / `max_w(v)` / `min_h(v)` / `max_h(v)` | `f32 -> L` | 像素便捷构造；百分比用 `L { min_w: Some(Size::Pct(50.0)), .. }` 直设字段 |
| `.dui` 属性 | `min-w=36` / `max-w=50%` / `min-h=10` / `max-h=200` | 与 `as_size` 共用一份语法（数字或 `%`）；裸属性报错，坏值报错并点名属性 |
| 注册表 | `PropType::Size` · 适用面 ANY | E1 属性注册表四条（见 [prop-registry](prop-registry.md)） |

**语义细则**（都有测试钉，见 `crates/deer-core/tests/l3_min_max.rs`）：

- **修复顺序**：先定**来源**（显式 > 父分配 > 固有），经 I-6 bound（可用空间），
  **最后**夹 `[min, max]` —— min/max 不参与「显式 vs 固有」的来源之争；
- **`min > max` ⇒ min 赢**（先夹 max 再托 min）：下限是「不能更小」的硬承诺，
  上限被下限打破时问题显式暴露（节点撑破上限看得见），不会静默缩成一团；
- **grow / stretch 都被夹**：grow 分配结果被 `max` 封顶（省下的空间不二次分配，停在主轴
  末端前）、被 `min` 托底（可以让子节点总和**溢出**容器）；主轴 stretch 均分后同样再夹一遍；
- **滚动容器照常生效**：滚动子节点的主轴 bound 是无穷（不被视口夹取），但 min/max 照夹
  —— 它是节点自身的声明，与视口无关；`max_scroll` 反映夹过的内容高；
- **流外（`position`）子节点同样受夹**（流外的显式/固有尺寸都过 `[min, max]`）；
- **百分比在 measure 阶段不生效**：measure 自底向上没有父宽度（与 `w`/`h` 的百分比
  同一惯例），百分比 min/max 只在 place 生效。

## 4. 自检（怎么确认你真的用对了）

```rust
// ① measure 侧：固有尺寸被夹（文本固有 32 → max_w=20 ⇒ 20），容器聚合用的是 20
assert_eq!(measure_tree(&tree, style, &m)["t"].0, 20.0);
// ② place 侧：grow 分配被 max 封顶，兄弟落位用夹过的尺寸
assert_eq!(geo["g1"].w, 80.0);
assert_eq!(geo["g2"].x, 80.0, "不是未夹的 150");
// ③ 冲突：min=60 > max=30 ⇒ 60
assert_eq!(geo["x"].w, 60.0);
```

跑 `cargo run -p deer-gui --example layout_refine_demo`，②③④ 段输出矩形数值 +
像素证据（`min_w` 托出来的区域真的有按钮像素），末行是「全部自检通过 ✅」。

## 5. 常见坑

| 现象 | 原因 | 怎么改 |
|---|---|---|
| 设了 `max_w` 但子节点还是那么宽 | 尺寸来源是**显式 `w=`** 且比 max 小 —— max 只封**上限**，不缩「来源本身合法」的值 | max 只在「值想超过它」时起作用；要缩小就改 `w=` |
| `min_w` 让内容超出容器了 | min 托底是**明确语义**（硬承诺优先），总和溢出时可见性由裁剪决定 | 调小 min，或给容器更多空间，或接受溢出 |
| `min > max` 时结果等于 min | 这是**钉死的规则**：min 赢（先夹 max 再托 min） | 别依赖「上限一定成立」；冲突时上限会输 |
| `min-w=50%` 没影响固有尺寸 | 百分比在 measure 阶段不解析（没有父宽度），只在 place 生效 | 用像素 min 影响固有尺寸，或接受「百分比只在落位夹」 |
| 滚动容器里的子节点被 `max_h` 压短了还以为没生效 | 恰恰是**生效了**：视口夹不住它，但 min/max 是节点自己的声明，照夹 | 想要「不被任何东西夹」就别设 max |
| grow 分区被 max 封顶后，剩下的空间没分给别的 grow 节点 | 封顶省下的空间**不二次分配**（简单优先的语义决策） | 需要精确分配就调 grow 权重，或去掉 max |

## 6. 相关

- 布局基础（`w`/`h`、`grow`、I-6/I-7）：[layout](layout.md)
- 每子节点交叉轴对齐（同一批落地的 L2，示例共用）：[align-self](align-self.md)
- 滚动容器（min/max 与视口的关系见第 3 节）：[scroll-and-multiline](scroll-and-multiline.md)
- 流外定位（流外也受 min/max 约束）：[absolute-positioning](absolute-positioning.md)
- **做不到什么**（都如实登记，不要误以为能跑）：
  - **不是 aspect-ratio**：宽高各自独立夹取，没有「保持比例」的联动；
  - **`min > max` 不能报错后自动取中间值**：规则就是 min 赢，没有第三种裁断；
  - **封顶省下的空间不二次分配**：grow 被 max 封顶后，剩余空间停在主轴末端前
    （不会按权重分给其它 grow 节点）；
  - **负值没有意义**：最终尺寸有 `.max(0)` 兜底，负的 min/max 等价于 0 那一侧的空约束；
  - **百分比 min/max 不影响固有尺寸**（measure 无父宽度），只在落位阶段夹。

## 7. 检查清单（发布前过一遍）

- [x] 示例能跑：`cargo run -p deer-gui --example layout_refine_demo` → `exit=0`
- [x] 示例有自检断言（measure/place/grow/冲突/滚动/流外 + 渲染像素 + `.dui` 往返）
- [x] `FEATURES.md` 已登记（状态 / 指南链接 / 示例命令都对）
- [x] `docs/TUTORIAL.md` 已提及（第 6 章布局段）
- [x] 明确写了「做不到什么」（非 aspect-ratio / min 赢 / 不二次分配 / 负值 / 百分比边界）
