# 功能指南：滚动容器 + 多行文本（scroll-and-multiline）

> 状态 ✅ · 示例 `cargo run -p deer-gui --example scroll` ·
> 清单条目见 [`FEATURES.md`](../../FEATURES.md)

## 1. 这是什么 / 什么时候用它

两个一起解决「界面装得下内容」这件最小的事：

- **多行文本**（`text` + `wrap`）：一段文字按节点自己的像素宽度**换行**，每行画一条
  `DrawCmd::Text`；节点高度自动按「行数 × 行高」预留；
- **滚动容器**（`column` + `scroll`）：内容高于视口时给出可滚动范围 `max_scroll`，
  滚动偏移参与几何与绘制，滚轮驱动偏移，**视口外的内容既不画出来、也点不到**。

什么时候用它：列表/日志/长文本/表单太长，一屏放不下。
什么时候**不**用它：需要表格那种二维滚动、需要**主题化**（自定义颜色/粗细）的滚动条、
需要**按键滚动**（`PageUp` / `Home` / `End`）、需要开箱即用的**惯性滑动**（只有纯逻辑，要自己驱动）——
这些目前都没做（见第 6 节）。
**滚动条本体已经会自己长出来**（轨道 + 滑块 + 拖动改偏移，见 [`scrollbar.md`](scrollbar.md)），
不再是这条「什么时候不用它」的理由。

两者的开关都是**默认关闭**（opt-in）：既有界面树一个像素都不会变。

## 2. 最小示例

```rust
use deer_gui::interaction::{InputEvent, UiEvent, UiState, handle};
use deer_gui::prelude::*;

// ① 多行文本：声明**像素宽度** + `wrap`
let mut app = Builder::new(Kind::Column, "app");
app.text_opts("alpha beta gamma delta epsilon", |n| {
    n.layout.width = Some(Size::Px(120.0));
    n.layout.wrap = true;
});

// ② 滚动容器：`Column` + `scroll`（视口高 = 它自己的 `h`）
app.container_opts(
    Kind::Column,
    "list",
    L::new().w(200.0).h(120.0).gap(4.0).scroll(true).to_props(),
    |c| {
        for i in 1..=10 {
            c.button(format!("行 {i}"));
        }
    },
);
let tree = app.build();

// ③ 每帧：布局（带偏移）→ 拿回 max_scroll → 灌进状态
let mut state = UiState::default();
let (geo, metrics) = deer_gui::layout::layout::layout_with_scroll(
    &tree,
    Rect::new(0.0, 0.0, 220.0, 240.0),
    TextStyle { font_size: 13.0, line_height: 18.0 },
    &ApproxMeasure,
    &state.scroll.offsets,     // 输入：当前偏移
);
state.scroll.set_metrics(&metrics);   // 输出：每个容器的 max_scroll（**必须灌**）

// ④ 滚轮：`Wheel` 没有坐标 ⇒ 滚「hover 命中节点」最近的可滚动祖先
handle(&mut state, &tree, &geo, clip, &InputEvent::PointerMoved { x: 13.0, y: 68.0 });
let evs = handle(&mut state, &tree, &geo, clip, &InputEvent::Wheel { dx: 0.0, dy: -1.0 });
assert!(matches!(evs.first(), Some(UiEvent::Scrolled { .. })));
```

跑完整版本（含自检断言与打印的真实数字）：`cargo run -p deer-gui --example scroll`

`.dui` 场景文件里是同名的**裸属性**：

```text
[column name=app]
  [column name=list w=200 h=120 gap=4 scroll]
    [button label="行 1"]
  [text w=120 wrap label="alpha beta gamma delta epsilon"]
```

## 3. 完整 API

### 3.1 两个开关（`LayoutProps`）

| 作用 | 命令式 | `.dui` 属性 | 说明 |
|---|---|---|---|
| 垂直滚动容器 | `L::new().scroll(true)` | `scroll`（裸属性） | 只对 `column` 有意义；`row` 上设了会被**忽略** |
| 文本换行 | `n.layout.wrap = true` | `wrap`（裸属性） | 只对 `text` 有意义；换行宽度取节点自己的 `w=` |

判据函数（布局/渲染/交互三处**共用**，免得各写一份条件）：

| 函数 | 真值 |
|---|---|
| `Node::is_scroll_container()` | `kind == Column && layout.scroll` |
| `Node::wraps_text()` | `kind == Text && layout.wrap` |

### 3.2 布局

| 函数 / 类型 | 说明 |
|---|---|
| `layout(tree, box, style, &Measure)` | **不变**：等价于「所有偏移为 0」 |
| `layout_with_scroll(tree, box, style, &Measure, &ScrollOffsets)` | 回 `(Geometry, ScrollMetrics)`；偏移参与几何，上限是输出 |
| `ScrollOffsets::new() / with(id, px) / get(id)` | 偏移（**整数像素**；没登记 ⇒ 0） |
| `ScrollMetrics::max_of(id) / clamp(id, px)` | 每个可滚动容器的 `max_scroll`（没登记 ⇒ 0，**fail-closed**） |

数字口径（都能一眼算出来）：

```text
内容高 = Σ 子节点主轴尺寸 + gap × (n−1) + padding × 2
max_scroll = max(0, ceil(内容高 − 视口高))      // 视口高 = 容器自己的 h
子节点位置 = 容器起点 + padding (+ 主轴对齐量) − 夹取后的偏移
```

滚动容器里**子节点的主轴尺寸不再被视口夹取**（否则「内容高于视口」这个前提自己就不成立），
`grow` 也随之失效（没有「剩余空间」可分）。

### 3.3 换行

| API | 说明 |
|---|---|
| `Measure::wrap(text, style, max_width) -> Vec<String>` | **新增的 trait 方法**，默认实现是「不换行」（一行）—— 不覆盖就不会有多行绘制 |
| `deer_core::layout::wrap_greedy(text, max_width, width_of)` | 唯一的换行算法（按空格/制表切词、超宽词按字符硬切） |
| `ApproxMeasure::wrap` / `FontMeasure::wrap` | 两个实现都委托给 `wrap_greedy`，只差「宽度怎么算」 |
| `Measure::height(text, style, max_width)` | **= `wrap(..).len() * line_height`**（行数与绘制同源，不许两套算法） |
| `deer_gpu::render::text_lines(&Measure, node, rect, style)` | 渲染器用：回 `(每行矩形, 该行文本)` |

### 3.4 交互（滚轮 → 偏移）

| API | 说明 |
|---|---|
| `UiState::scroll: ScrollState` | 偏移 + 上限（`UiState` 是交互的唯一真相） |
| `ScrollState::set_metrics(&ScrollMetrics)` | **每帧灌一次**；顺带把已有偏移夹回新上限 |
| `ScrollState::offset_of(id)` / `max_of(id)` | 读当前偏移 / 上限 |
| `ScrollState::scroll_to(id, px)` / `scroll_by(id, delta)` | 夹取后写偏移；**没变**就回 `None`（不发事件） |
| `UiEvent::Scrolled { id, offset }` | 偏移**真的变了**才发（窗口层据此置 dirty） |
| `interaction::WHEEL_STEP_PX`（= 40） | 滚轮「一格」对应的像素数；`dy < 0`（向下拨）⇒ 偏移**增大**（内容上移） |
| `interaction::handle` 的 `Wheel` 分支 | 目标 = `hover` 命中节点**最近的可滚动祖先（含自身）**；禁用子树不响应 |

### 3.5 绘制

- 换行文本 ⇒ **每行一条 `DrawCmd::Text`**（矩形 = 节点矩形按行高下移，宽度 = 节点宽度）；
  不换行时**恰好一条**、矩形就是节点矩形（既有语料逐字节不变）；
- 可滚动容器 ⇒ 在容器**自己的视觉之后**推 `PushClip`（= 视口矩形），走完子节点再 `PopClip`；
  裁剪栈语义与 `null.rs` 完全一致（求交 / 出栈），**GPU 后端不用改一行**。

## 4. 自检（怎么确认你真的用对了）

```rust
// ① 上限 = 内容高 − 视口高（拿真实布局的数字算，不要凭感觉）
assert_eq!(metrics.max_of("list"), 136, "内容 256 − 视口 120 = 136");

// ② 偏移进几何：每个子节点整体位移 −offset，容器自身一个像素不动
let (g0, _) = layout_with_scroll(&tree, box_, style, &ApproxMeasure, &ScrollOffsets::new());
let (g1, _) = layout_with_scroll(&tree, box_, style, &ApproxMeasure, &ScrollOffsets::new().with("list", 40));
assert_eq!(g1["button_1"].y, g0["button_1"].y - 40.0);
assert_eq!(g1["list"], g0["list"]);

// ③ 滚到边界不越界：偏移被夹进 [0, max_scroll]
let (g2, _) = layout_with_scroll(&tree, box_, style, &ApproxMeasure, &ScrollOffsets::new().with("list", 9999));
assert_eq!(g2["button_1"].y, g0["button_1"].y - 136.0);

// ④ 视口外的点不命中（`ClipSnapshot` 把视口裁剪绑到了子节点上）
assert!(!clip.allows("button_2", 10.0, 50.0), "视口外的点必须被裁剪拒绝");

// ⑤ 多行：行数 = 命令数，每行矩形 = i × 行高
assert_eq!(lines_i, i as i32 * 18, "第 i 行必须落在 i × 行高 上");
```

`cargo run -p deer-gui --example scroll` 把上面这些数字**打印出来**（内容高、视口、
`max_scroll`、行数与每行矩形、滚动前后的命中与几何），并逐条断言。

## 5. 常见坑

| 现象 | 原因 | 怎么改 |
|---|---|---|
| 设了 `wrap` 却还是一行 | 换行宽度取节点的**像素宽度**；没写 `w=` 时节点宽 = 文本宽 ⇒ 无处可换；`w=50%` 也一样（百分比要到排布阶段才知道） | 给 `w=<像素>`；需要「撑满父容器再换行」时，用父容器的宽度算好像素值 |
| 换行后少了几行 | 父容器把节点夹得比它声明的 `w` 更窄 ⇒ 行数变多，但**预留高度**是按声明宽度算的；装不下的行不画（命令矩形绝不越出节点自己的矩形） | 让父容器至少和声明的宽度一样宽；或显式给 `h=` 足够的高度 |
| 滚动容器里子节点还是被压扁 | 只有**滚动容器自己**的子节点不被视口夹取；再往下的层级照旧（那是正常的 flex 语义） | 把「内容」直接放进滚动容器，或给它显式高度 |
| `grow` 在滚动容器里没反应 | 滚动容器的主轴没有「剩余空间」可分 ⇒ `grow` 被忽略（刻意的） | 给子节点显式高度，或不要用滚动容器 |
| 滚轮完全没反应 | ① 调用方**没灌** `set_metrics`（上限表为空 ⇒ 一律 0，fail-closed）；② 指针从没进入容器（`Wheel` 没有坐标 ⇒ 没有 `hover` 就没有目标）；③ 容器是禁用的 | 每帧 `state.scroll.set_metrics(&metrics)`；先让指针进容器；别禁用要滚的容器 |
| 滚一格跳得太远/太近 | `dy` 的单位由窗口层给（winit 的 `LineDelta` 是**行**、`PixelDelta` 是像素，`map_wheel` 原样透传），状态机按「行」解释、一格 = `WHEEL_STEP_PX`(40 px) | 想要别的步长就改 `WHEEL_STEP_PX`（一处定义），或让窗口层先把 `dy` 换算成像素 |
| 滚轮方向反了 | 约定是 `dy < 0`（向下拨）⇒ 内容上移、偏移增大 | 在窗口层把 `dy` 取负；状态机的约定在 `WHEEL_STEP_PX` 的文档里 |
| `.dui` 里写 `scroll=1` 报错 | 开关属性是**裸属性**（`scroll`），带值会被拒绝（防「看起来生效、实际没生效」） | 写成 `scroll` / `wrap` |
| `row` 上设 `scroll` 没反应 | 本期只做**垂直**滚动（`Row` 上的 `scroll` 被忽略，且被测试钉住） | 用 `column` + `scroll` |
| 滚动之后再滚动「没事件」 | 到顶/到底、或 `max_scroll == 0`、或 `dy == 0` 时偏移**没变** ⇒ 不发 `Scrolled`（窗口层的 dirty 约定靠它） | 这是刻意的：没变就不该重绘 |

## 6. 相关

- 布局（几何、`grow`、对齐）：[`layout.md`](layout.md)
- 命中测试的具体判据：[`hit-testing.md`](hit-testing.md)
- 事件模型与状态机：[`input.md`](input.md)
- 绘制列表与裁剪栈：[`draw-list.md`](draw-list.md) · [`rendering.md`](rendering.md)
- 场景文件语法：[`scene-file.md`](scene-file.md)
- 内部原理：`crates/deer-gui/tests/scroll_multiline.rs`（回归 + 冻结摘要）、
  `crates/deer-gpu/tests/scroll_multiline.rs`（命令与裁剪）、
  `crates/deer-core/tests/scroll_multiline.rs`（布局数字）
- **做不到**（本期边界，都如实登记）：
  - **没有水平滚动**（`row` 上的 `scroll` 被忽略）；**滚动条已落地** —— 内容装不下视口的容器会自己长出
    「轨道 + 滑块」，按滑块拖动改偏移（默认 opt-in，见 [`scrollbar.md`](scrollbar.md)）；
  - **惯性滚动已收口**（T3.2b）：App 在 `redraw`/`next_deadline` 里接 `advance_inertia`/`inertia_deadline`
    两行即可（参考 `--example scroll_inertia_window`，见 [`scrollbar.md`](scrollbar.md)）；
  - `scroll_to` 只能按像素设偏移，**没有**「把某个节点滚到可见」（nearest-into-view）的 API；
  - 命中侧有两道闸：**几何**（滚到容器矩形之外的子节点根本探不到，因为 `hit_test` 只从
    「矩形包含该点」的祖先往下探）与**裁剪**（`ClipSnapshot` 的视口裁剪）。两者在滚动容器上
    **结论一致但冗余**——裁剪那道在**渲染**侧才是必需的（没有它，被滚上去的内容会画到视口外）；
  - 被视口挡住的点**不命中，也不会回退到祖先**（与既有「裁剪外的点不命中」同一条语义）；
  - 滚动偏移是**整数像素**（几何本身就是整数）；没有亚像素滚动；
  - `wrap` 只在**按宽度**换行这一件事上生效：没有省略号截断、没有「最多 N 行」、没有
    富文本/多段样式（行距来自 `TextStyle::line_height`，全节点一致）。

## 7. 检查清单

- [x] 示例能跑：`cargo run -p deer-gui --example scroll` → `exit=0`
- [x] 示例有自检断言（行数与每行矩形、`max_scroll`、越界夹取、命中跟着偏移走）
- [x] `FEATURES.md` 已登记（状态 / 指南链接 / 示例命令都对）
- [x] 明确写了「做不到什么」
