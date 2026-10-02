# 功能指南：测试接口（testkit）

> 状态 ✅ · 示例 `cargo run -p deer-gui --features testing --example testkit_demo` ·
> 清单条目见 [`FEATURES.md`](../../FEATURES.md) ·
> 纪律来源（每个判据的出处）见 `crates/deer-gui/src/testing.rs` 的模块文档表格

## 1. 这是什么 / 什么时候用它

**把本项目既有的测试纪律变成 API**：让「写一条 UI 测试」从抄 600 行样板变成**十几行**，
并且**默认带上**前置断言、门槛自证、越界 = 0、CPU/GPU 对照容差、失败时可复制的复现命令。

它替你做的，正是原来每个 example / 每个 `tests/*.rs` 各抄一遍的那套：

| 原来要自己写 | 现在 |
|---|---|
| 树 → 几何 → 绘制列表 → 裁剪快照（`examples/counter.rs` 里那个 `Frame` 那一整段） | `Harness::frame()`（**并断言前置条件**） |
| `node_hint > 0`、裁剪快照非空、关键 id 在快照里（抄漏一处就**静默「全不裁剪」**） | 同上，**每次**都做；`tap`/`move @id` 用到的 id **自动**登记 |
| 像素差异分类（框内 / 框外）、容差（不透明 0 / 半透明 ≤1 LSB） | `Shot::assert_state_change_only` / `ParityRule` |
| `move @id` → 按布局算坐标（写死坐标会**静默点空**） | `Harness::run_script("move @plus;down:left;up:left")` |
| 「这一帧同时变的东西要列全」（实测漏列 ⇒ 框外出现一批本该在框内的差异像素，于是一条假红） | 期望矩形**自动**算出来 |
| CPU↔GPU 对照（另写一套 `window_parity` / `gpu_vs_cpu`） | `Harness::compare_cpu_gpu(...)` 一条调用 |
| 失败信息里手写 `format!` 拼数字 | 每个断言都自动附**可复制的复现命令** + 真实数字 |

**什么时候不该用它**：要在**真窗口**里验证（winit 要求事件循环在主线程 ⇒ 只能写在
`examples/` 里，见第 6 节「做不到什么」）；只做纯布局/纯解析时（那些有更轻的入口）。

## 2. 最小示例

`crates/deer-gui/tests/testkit_counter.rs::minimal_downstream_test` 就是这个形状：

```rust
use deer_gui::prelude::*;
use deer_gui::testing::{Harness, Repro};

#[test]
fn my_ui_test() -> Result<(), String> {
    let mut app = Builder::new(Kind::Column, "app").padding(12.0).gap(8.0);
    let plus = app.button("+");
    let mut h = Harness::new(app, 220, 120, Theme::default());
    h.set_repro(Repro::test("deer-gui", "testing", "my_test", "my_ui_test", &[]));
    let before = h.shoot_named("点击前")?;         // 离屏 CPU 像素（真字形）
    let step = h.tap(&plus)?;                      // 单事件注入（坐标由布局算）
    assert!(step.changed, "点一下 + 必须改状态");
    let after = h.shoot_named("点击后")?;
    after.assert_state_change_only(&before, &plus)?; // 差异只落在变了的节点矩形内
    Ok(())
}
```

跑起来（**两条命令都能照抄**）：

```sh
# ① 可运行的示例：同样的事，但会逐条打印证据（命令数 / NodeHint / 像素 / CPU↔GPU）
cargo run -p deer-gui --features testing --example testkit_demo

# ② 用 testkit 重写的 counter 自检段（含「结论与手工版一致」的证据）
cargo test -p deer-gui --features testing --test testkit_counter -- --nocapture
```

门槛形态的命令（**`set "VAR=1" &&` —— 写错会静默不生效**，理由见
[`GETTING-STARTED-UI-STEPS.md`](../GETTING-STARTED-UI-STEPS.md) 的坑③）：

```sh
set "DEER_VK_VALIDATION=1" && cargo test -p deer-gui --features testing --test testkit_counter -- --nocapture
set "DEER_VK_WINDOW_TESTS=1" && cargo test -p deer-gui --features testing -- --nocapture
set "DEER_VK_FRAMES=3" && cargo test -p deer-gui --features testing -- --nocapture
```

## 3. 完整 API

**为什么是 feature**：`#[cfg(any(test, feature = "testing"))]`。
`test` 那一支让 `cargo test --workspace`（默认 feature）**自动跑 testkit 自身的单测**
（护栏进默认门禁）；`feature` 那一支给下游用。普通生产构建里这个模块**不存在**（零成本）。
零新增第三方依赖。

```toml
[dev-dependencies]
deer-gui = { path = "path/to/crates/deer-gui", features = ["testing"] }
```

### 3.1 建面与一帧

| 项 | 说明 |
|---|---|
| `Harness::new(tree, w, h, theme)` | `tree` 可以是 `Node` 或 `Builder`（经 `IntoTree`）。默认接**真实字体度量**（`%WINDIR%\Fonts` 的 consola/arial/segoeui 之一）；找不到就打印 `TESTKIT-FONT` 降级说明，**不静默** |
| `Harness::without_font(...)` | **显式**降级档（用来验证降级路径本身） |
| `Harness::set_clear(color)` | 清屏色（默认 = `Theme::default().surface`，与 `render_tree_to_rgba` 同口径） |
| `Harness::set_tree(t)` | 换树。本项目**没有事件回调** ⇒ 改了数据就整树重建 |
| `Harness::set_case(name)` / `set_repro(Repro)` | 失败信息的抬头与复现命令 |
| `Harness::require_ids(&[...])` | 从此**每次** `frame()` 都断言这些 id 有几何、且在裁剪快照里 |
| `Harness::frame() -> Result<Frame, String>` | 树 → 几何 → 绘制列表 → 裁剪快照，**并断言前置条件**：裁剪栈配平、`node_hint > 0`、快照非空、登记 id 齐全 |
| `Harness::shoot()` / `shoot_named(label)` | 当前这一帧的离屏 CPU 像素 ⇒ `Shot`（带状态与每个节点的矩形） |

### 3.2 输入注入

| 项 | 说明 |
|---|---|
| `Harness::send(&InputEvent)` | 喂**一条**事件 ⇒ `Step { input, changed, events, state }` |
| `Harness::tap(id)` | 左键点一下（`move → down → up` 三条事件，坐标由布局算） |
| `Harness::move_to(id)` / `center_of(id)` | 指针移到节点中心 |
| `Harness::run_script(src)` | 脚本重放，**多一条 `move @id`**（其余语法与 `deer_gui::input_script` **同一份解析器**）。`@id` 在那一帧没有几何 ⇒ **硬错**（不许静默点空）。返回 `ScriptRun { steps, resolved }`，`resolved` 是展开后的纯坐标脚本，可直接喂 `input_script::replay` 交叉验证 |

`Step.changed` 的判据是 **`UiState` 逐字段比较**（不是「收到事件了」）—— 与窗口层
`App::wants_redraw` 同一条规则。

### 3.3 断言

| 项 | 说明 |
|---|---|
| `Shot::assert_state_change_only(before, primary)` | **主矩形内必须有差异**（否则「它没画出来」）+ **框外必须为 0**。期望矩形 = `primary` ∪ 所有 `hover/focus/pressed` 真的变了的节点（**自动算，不手抄**）。**自动补齐有面积棘轮，且「框外为 0」的判别力有边界 —— 见下面的方框** |
| `Shot::assert_diff_only_inside(before, primary)` | 同上但**不**要求主矩形内非空 |
| `Shot::assert_no_diff_outside(before, rects)` | 显式给矩形版的「框外 = 0」 |
| `Shot::assert_bytes_eq(before)` | **逐字节相同**（不透明语料的判据） |
| `Harness::assert_hover/assert_focus/assert_pressed/assert_visual_state/assert_text/assert_texts/assert_state` | `UiState` 逐字段（`texts` 可整表全等） |
| `Harness::assert_focus_order(&[...])` / `assert_focus_order_excludes(&[...])` | 焦点树序（**禁用子树整棵不在内**） |
| `Harness::assert_command_count/assert_node_hint_count/assert_clip_nodes/assert_clip_known/assert_counts` | 绘制列表与裁剪快照的计数 |
| `Harness::assert_drawn_text/assert_drawn_text_contains/assert_text_size` | **画出来的文本**（读绘制列表，不是读树）与字号 |
| `Harness::require_glyph_pixels()` | 文本类像素断言的**前置**：没有真字形就明确报错（不给假红） |

**⚠️ 「框外为 0」的判别力边界：一个面积棘轮 + 一个**未闭**的残余**

- **棘轮存在，而且会 `Err`**：自动补齐的那些节点**占画布面积 ≥25% ⇒ 直接报错**（`AUTO_PATCH_AREA_RATCHET = 0.25`，判定在 `crates/deer-gui/src/testing.rs` 的自动比较路径上）。
  阈值 **25% 的依据是实测、不是拍脑袋**：本仓库合法的自动补齐都是**按钮 / 输入框这类小控件**（360×200 画布上占比 **≤ ~3%**），
  而**根容器 = 100%** ⇒ 取 25% 等于留了**一个量级**的余量。没有它，自动补齐会把判据悄悄放松成「几乎整屏」，而使用者看不出来。
- **<25% 的已知残余（必须知道：这条**未闭**）**：**任何 ≤25% 的节点，只要它的交互态变了，落在它矩形内、与这次比较无关的差异都会被放行**。
  `verifier` 已**实测绕过**：**160×120** 画布上 `panel` **40×90 = 18.75%** ⇒ hover 落到它上面时，「**框外为 0**」**失去判别力**
  （那些差异被自动补进「期望矩形」里了）。⇒ 正确口径：
  **「框外为 0」只在你显式给的 `primary` + 自动补齐的**小**节点上保证判别力**；
  **大 / 中等面积节点上，请自己用 `assert_no_diff_outside(before, rects)` 显式给窄矩形。**
- **完整解法是登记未做的后续（当前实现只是这一档）**：正解应为「**状态差 ∩ 逐节点绘制命令变化**」——
  只有「这次状态真的变了 **且** 该节点的绘制命令真的变了」的节点才进期望矩形。
  **代价**：要给 `Shot` 加**逐节点绘制指纹**，那是 **harness 采集侧的新通路**。
  在它落地之前，**别把面积棘轮当成「中等面积也安全」**。

### 3.4 CPU↔GPU 对照

| 项 | 说明 |
|---|---|
| `ParityRule::Opaque` | 语料全不透明 ⇒ **逐字节 0** |
| `ParityRule::Translucent` | 含半透明 ⇒ 最大通道差 **≤1 LSB**（CPU `round()` vs GPU 固定功能 UNORM） |
| `ParityRule::for_list(&list)` | **从这一帧真正要画的命令**里读该用哪档（首选）；`for_theme` 是粗略版 |
| `Harness::compare_cpu_gpu(name, &list, rule)` | 离屏 Vulkan 画一帧 + CPU 画同一帧 + 算报告 + 断言。**`Ok(None)` = 本机没有可用 GPU（跳过，不是通过）** |
| `GpuProbe` | 需要分开拿字节时用它：`render`（GPU 回读）/ `cpu_render` / `report_from`（用**已拿到的**字节算报告，不重复渲染） |
| `ParityReport` | 结构化差异：`max_channel_diff`、`differing_pixels`、`differing_bytes`、`worst_pixel`、`worst_channel`、最差点的 `gpu`/`cpu` 四通道原值、`text_skipped` |

**容差没有「放宽」入口**：只有这两档，上限是常量 `BYTE_EXACT = 0` / `LSB_TOLERANCE = 1`。
一旦有 `max_allowed: u8` 这样的参数，第一个「先松一点让它绿」的补丁就会进来。

`ParityReport::check` 里 **`text_skipped != 0` 也算不通过**：那时候「逐字节相同」
可能只是因为**两边都没画**（空串 / `size<=0` / 被裁空），结论不可用。

### 3.5 门槛、自证与复现命令

| 项 | 说明 |
|---|---|
| `gate(name)` / `gate_default_true(name)` | 转发 `deer_gui::env_gate`（**先 `trim()` 再比**，`set X=1 &&` 的值是 `"1 "`） |
| `print_gate(name)` / `print_gates(&[...])` | 打出**自证行**（含**原始值**）：`TESTKIT-GATE NAME=Some("1 ") ⇒ ON`。验收 `grep TESTKIT-GATE` 就有证据 |
| `require_gate(name)` | 未设 ⇒ 打印 `TESTKIT-SKIP …（这是跳过，不是通过）` 并返回 `false` |
| `window::gate()` / `window::require()` / `window::frames_from_env(n)` / `window::frames_from_value(raw, n)` | 窗口档的门槛与帧数（`DEER_VK_WINDOW_TESTS` / `DEER_VK_FRAMES`）；帧数先 `trim()` 再 parse（`set X=3 &&` 的值是 `"3 "`），**解析不出来就报错、不静默退回默认** —— `None`/空串 ⇒ 默认值（≥1）、能解析成 ≥1 的整数 ⇒ 用它、**其它一律 `Err`（含 `0`）**。理由：**帧数本身就是这次度量的参数**，静默换成别的值等于「测的不是你要的那件事」。签名是 `Result<u64, String>`，由调用方决定退出码。**实测**：`DEER_VK_FRAMES=abc` + 门槛开 ⇒ **`exit 2`**（打印 `❌ DEER_VK_FRAMES = "abc" 不是合法帧数（要正整数）`）；`=3` ⇒ `exit 0` 且窗口真的跑了 3 帧。另一个消费者 `examples/window_parity.rs` **已同步为同一口径**（`frames_from_env() -> Result<u64, String>`，非法即 `ExitCode::from(2)`） |
| `Repro::test(pkg, features, test_target, filter, gates)` / `Repro::example(...)` | 生成可复制的复现命令；`gate_prefix(&[...])` 生成 `set "VAR=1" && ` 形态 |
| `default_repro()` | 没设 `set_repro` 时的兜底（仍然可复制，只是范围粗一点） |

**每一次**断言失败都带复现命令 —— 这是**结构性**的（`Harness::carry` / `Shot::carry`），
不靠每个断言作者记得写。

## 4. 自检（怎么确认你真的用对了）

testkit 自己**有**护栏，而且每个断言助手都有一条「**它会红**」的反向自检：

```sh
# ① testkit 自身的单测（反向自检表 + 正面表 + 门槛自证 + CPU↔GPU 逐字节对照）
cargo test -p deer-gui --features testing --lib testing -- --nocapture

# ② 汇总那一行是**运行输出**里的（本仓库不在文档里固化测试条数）
cargo test -p deer-gui --features testing --lib testing -- --nocapture
#   ⇒ 反向自检汇总：N 个断言助手「会红」✅ + M 个纯函数判据「会红」✅ + K 个助手在正确期望下「会绿」✅
```

`every_assertion_helper_can_go_red_and_each_green_case_passes` 是一张**表**：
每个断言助手都被喂一个故意错误的期望值，并断言它**确实返回 `Err`**（且错误里带复现命令）；
另有 `greens` 表挡住反方向（「恒返回 `Err`」的假护栏）。两张表都有**条数下限**
（棘轮：加助手时下限要跟着涨）。

**变异自检**（把判据改坏，必须打中目标）：

| 变异 | 变红的测试 |
|---|---|
| 前置判据「`node_hint>0` / 快照非空」改成恒真 | `preconditions_reject_a_list_without_node_hints` |
| `ParityRule::Translucent` 的容差 1 → 255 | `parity_report_check_goes_red_for_each_rule`、`parity_rule_is_read_from_the_corpus`、`every_assertion_helper_can_go_red_and_each_green_case_passes` |
| 删掉三处「框外必须为 0」 | `leaked_changes_outside_the_expected_rects_go_red`、`every_assertion_helper_can_go_red_and_each_green_case_passes` |
| `visually_changed_ids` 恒返回空 | `visually_changed_ids_is_exact`、`tapping_plus_changes_the_count_pixels_and_nothing_outside`、`full_script_run_reaches_the_same_conclusion_as_the_hand_written_selfcheck` |
| `require_glyph_pixels` 恒返回 `Ok` | `the_degraded_harness_refuses_text_pixel_assertions`、`every_assertion_helper_can_go_red_and_each_green_case_passes` |

> 复现做法：改一处判据 → `cargo clean -p deer-gui` → 跑上面那条 `cargo test` →
> 记录哪些测试变红 → 还原 → **再 `cargo clean -p deer-gui`** 复验全绿。

**结论一致性**：`cpu_gpu_parity_matches_the_hand_written_loop_byte_for_byte` 把
`crates/deer-vk/tests/gpu_vs_cpu.rs` 的比较循环**照抄一遍**当独立对照，
逐数字断言 testkit 的报告与它相同（最大通道差 / 不同字节 / 不同像素 / 最差点 / 最差点的四通道原值）。
`run_script_supports_move_at_id_and_keeps_the_input_script_semantics` 断言 testkit 重放的
**终态与既有 `input_script::replay` 逐字段相同**（含重绘序列）。

## 5. 常见坑

| 现象 | 原因 | 怎么改 |
|---|---|---|
| 文本类的像素断言「明明变了却是 0 像素」 | 这一档**没有真字形** ⇒ 每个字符都是同一个等宽方块 ⇒ **假红** | 先调 `Harness::require_glyph_pixels()`（它会**明确报错**）；修字体或只做绘制列表/状态级断言 |
| 框外有一堆差异像素，看着像「溢出」 | 断言**漏列了同时变的东西**（`counter.rs` 实测踩过：只列计数矩形时框外出现一批本该在框内的像素） | 用 `assert_state_change_only` / `assert_diff_only_inside`：期望矩形**自动**包含视觉真的变了的节点 |
| 「被裁掉的控件还能点中」 | 用的是不发 `NodeHint` 的渲染器（`DefaultRenderer`）⇒ 裁剪快照为空 ⇒ 命中**静默退化**成「全不裁剪」 | `Harness::frame()` 会直接报错；别绕开它自己建列表 |
| 点了 `@id` 但什么都没发生 | 坐标写死了，布局一变就点空 | 用 `move @id`（坐标由布局算）；`@id` 没有几何 ⇒ testkit **硬错** |
| 点了按钮，但画面还是旧内容 | 本项目**没有事件回调**，改的是你自己的数据 | 改完数据调 `Harness::set_tree(...)` 重建树 |
| 「门槛设了却什么都没跑」 | `set X=1 && …` 在 cmd 里值可能是 `"1 "` 或干脆没设上 | 用 `set "X=1" && …`；用 `print_gates` 打自证行确认（`TESTKIT-GATE …`） |
| 无 GPU 的机器上对照「通过」了 | 那是**跳过** | `compare_cpu_gpu` 返回 `Ok(None)` 并打印 `TESTKIT-SKIP`；只有 `Some(report)` 才是结论 |

## 6. 相关

- 事件模型与状态机：[`input.md`](input.md)、[`hit-testing.md`](hit-testing.md)
- 绘制列表（`NodeHint` / 裁剪栈）：[`draw-list.md`](draw-list.md)
- CPU 后端与像素：[`pixels.md`](pixels.md)、[`rendering.md`](rendering.md)
- CPU↔GPU 对照的完整语料：`crates/deer-vk/tests/gpu_vs_cpu.rs`（testkit 只搬口径，不搬语料）
- 门槛与 `set "VAR=1" &&`：[`GETTING-STARTED-UI-STEPS.md`](../GETTING-STARTED-UI-STEPS.md) 的「三个实测坑」
- **做不到**：① **真窗口 e2e 不在库里** —— winit 要求事件循环在**主线程**，`#[test]` 的线程不是，
  所以「建窗跑 N 帧」只能写在 `examples/`（`cargo run -p deer-gui --features window --example window_parity`）；
  testkit 只提供能进 `cargo test` 的那半（门槛判定 + 帧数解析 + 自证标记）。
  ② **不替代**该功能自己的语料判据：testkit 是「把纪律变成 API」，语料仍然要你自己写。
  ③ **不放宽**任何阈值：容差只有两档，没有 `max_allowed` 参数。
  ④ 只有**一个**窗口/一次一帧：没有多窗口；**脚本动词**是
  `move` / `down` / `up` / `key` / `keyup` / `text` / `focus` / `wheel` 这一组，**没有 IME 预编辑的动词**
  （`ImePreedit` 只能直接 `send` 事件对象注入）。功能本身都已落地 —— 见
  [`ime.md`](ime.md) 与 [`scrollbar.md`](scrollbar.md)。

## 7. 检查清单

- [x] 示例能跑：`cargo run -p deer-gui --features testing --example testkit_demo` → `exit=0`
- [x] 示例有自检断言（前置断言 + 状态变化 + 像素框外 = 0 + CPU↔GPU 对照）
- [x] `FEATURES.md` 已登记
- [x] testkit 自身有单测，且**每个断言助手都有反向自检**（含变异证据，见第 4 节）
- [x] 明确写了「做不到什么」
