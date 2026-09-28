# M5 — 输入、焦点与重绘闭环（让界面「能用」）

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers `subagent-driven-development`（实现者 → 独立 reviewer → fix round）；DSH 侧用共享任务板 + 一次性执行者（`subagent`）。

**Goal:** 补上「**输入事件 → 命中测试 → 状态变更 → 重绘**」这条应用闭环：窗口里能**悬停/按下/点击/聚焦/输入文字**，并且整条链**可确定性回归**（脚本化事件重放 + 像素判据）。

**Architecture:** 分三层，**交互逻辑是纯函数**（不碰窗口、不碰 GPU），窗口层只负责把 winit 事件翻成我们自己的事件并在需要时重绘。

```
winit 事件 ──(M5-1 deer-window::map_*)──▶ InputEvent（我们自己的模型，可序列化/可重放）
                                              │
                    (M5-2/3 deer-gui::interaction，纯函数，无窗口无 GPU)
                                              ▼
        hit() ──▶ UiState{hover,focus,pressed,texts} ──▶ Vec<UiEvent>{Clicked,TextChanged,FocusChanged}
                                              │
                                    (M5-4) dirty ⇒ 重绘 ⇒ 新的一帧
```

**已核实的既有接口（不要凭记忆改）**
- `deer_layout::hit_test(root, geo, px, py) -> Option<&Node>`（`layout.rs:353`）：**最深命中胜出**（后序覆盖）、半开区间 `px ∈ [x, x+w)`、注释明确「**输入路由的唯一依据**」。
- `deer_gpu::DrawCmd::NodeHint { rect: RectI, node_id_len: u32 }`：**id 不在指令里**（只有长度）⇒ id 在侧表，实现前先读清 `draw.rs` 里它的实际用法。
- `deer_window::App`（`lib.rs:114`）：目前只有 `init/resized/redraw/close_requested`。
- `deer_gpu::NodeProps::disabled` 已存在 ⇒ **禁用节点必须不响应输入**。

**Spec:** `ROADMAP.md` M5 行、`crates/deer-layout/src/layout.rs:353`、`crates/deer-gpu/src/draw.rs`、`crates/deer-window/src/lib.rs`。

## Global Constraints

- **交互逻辑必须是纯函数**（可无窗口、无 GPU 测试）；窗口层只做翻译与重绘调度。
- **确定性可回归**：事件可**脚本化重放**；每条交互规则都要有单测。
- **不改变既有渲染行为**：像素判据（不透明逐字节 0 / 半透明 ≤1 LSB）与四档门禁不得退化。
- **停靠（docking）本轮不做**，登记在 `ROADMAP.md`（口径：实现事实 + 未做 + 可复现命令）。
- 零新增第三方依赖；不跑 `cargo fmt`；**先 `cargo clean -p <crate>`**；不用 shell 文本管道改源码；**提交用显式路径**；`example` 与 `cargo test` **不可并发跑**。

---

### Task M5-1（`deer-window`）：输入事件模型 + winit 映射 + `App::input`

**Produces:**（冻结）
```rust
pub enum PointerButton { Left, Right, Middle }
pub struct Mods { pub shift: bool, pub ctrl: bool, pub alt: bool, pub sup: bool }
pub enum Key { Tab, Escape, Enter, Backspace, Left, Right, Up, Down, Char(char), Other }
pub enum InputEvent {
    PointerMoved { x: f32, y: f32 },
    PointerDown  { button: PointerButton, x: f32, y: f32 },
    PointerUp    { button: PointerButton, x: f32, y: f32 },
    Wheel        { dx: f32, dy: f32 },
    KeyDown      { key: Key, mods: Mods },
    KeyUp        { key: Key, mods: Mods },
    TextInput    { text: String },
    FocusChanged { focused: bool },
}
// App 新增（默认实现 = 忽略，保证既有实现不破）：
fn input(&mut self, info: &WindowInfo, ev: &InputEvent) -> Result<Flow, String> { let _ = (info, ev); Ok(Flow::Continue) }
// 纯映射函数（可单测，不需要真窗口）：
pub fn map_key(key: &winit_key, mods: Mods) -> Key;      // 传字段而非 winit 事件，便于单测
pub fn map_mouse_button(b: winit_button) -> PointerButton;
```
- [ ] Step 1（失败测试）：`map_key`/`map_mouse_button` 的表驱动单测（含 Tab/Escape/Enter/Backspace/方向键/字符键/未知键 → `Other`）。
- [ ] Step 2：事件循环接线：`CursorMoved/MouseInput/MouseWheel/KeyboardInput/Ime/Focused` → `InputEvent` → `App::input`；**只在事件确实改变状态时**才 `request_redraw`（见 M5-4 的 dirty 约定）。
- [ ] Step 3：`examples/` 里加一个最小 example 打印收到的事件（验证接线）。Commit。

### Task M5-2（`deer-gui::interaction`，**纯逻辑**）：命中与状态机

**Produces:**（冻结）
```rust
pub struct UiState { pub hover: Option<String>, pub focus: Option<String>, pub pressed: Option<String>, pub texts: BTreeMap<String, String> }
pub enum UiEvent { HoverChanged(Option<String>), FocusChanged(Option<String>), Clicked(String), TextChanged { id: String, value: String } }
/// 命中：最深命中者胜出（复用 `hit_test`）+ 跳过禁用节点 + 跳过被裁剪掉的点。
pub fn hit<'a>(root: &'a Node, geo: &Geometry, clip: ClipSnapshot, x: f32, y: f32) -> Option<&'a Node>;
/// 纯状态机：喂事件，产出「发生了什么」。
pub fn handle(state: &mut UiState, root: &Node, geo: &Geometry, clip: ClipSnapshot, ev: &InputEvent) -> Vec<UiEvent>;
/// 可聚焦节点的树序（Tab 循环用）。
pub fn focusables(root: &Node) -> Vec<String>;
```
- [ ] Step 1（失败测试，逐条规则各一例）：悬停进入/离开；按下→抬起产生 `Clicked`；**按下后移出再抬起不产生 Clicked**；禁用节点**不响应**（hover/press/click 全忽略）；**被裁剪的点不命中**；`Tab`/`Shift+Tab` 循环焦点；`Escape` 清焦点；聚焦文本框后 `TextInput` 追加、`Backspace` 删一个字符；`Enter` 在按钮上等同点击（若按钮语义如此则实现，否则**明确登记为不做**）。
- [ ] Step 2：实现；`cmd /c "cargo test -p deer-gui --lib interaction"` 绿。Commit。

### Task M5-3（`deer-window` 窗口侧裁剪快照的**来源**）

- `hit()` 需要「该点在不在当前裁剪内」。请先读清 `DrawCmd::NodeHint { node_id_len }` 与 `DrawList` 的裁剪记账，给出**最小且不重复造轮子**的来源（例如从 `DrawList` 派生一份 `ClipSnapshot`）。
- [ ] Step 1：给出 `ClipSnapshot` 的定义与构造（纯函数，可单测：读一份含 `PushClip/PopClip` 的 `DrawList` ⇒ 得到每个节点的有效裁剪矩形；`PopClip` 空栈按既有语义）。
- [ ] Step 2：单测覆盖「嵌套裁剪」「空栈 `PopClip`」「点恰在边界（半开区间）」。Commit。

### Task M5-4：窗口集成 + 脚本化重放 example

**Produces:** `crates/deer-gui/examples/interactive_form.rs`
- 持有 `UiState` + 渲染器；`input()` → `interaction::handle()` → 变更 ⇒ 置 dirty；`redraw()` 只在 dirty 时重绘（**并打印每帧是否重绘**）。
- **脚本化重放模式**：`DEER_INPUT_SCRIPT="move:120,80;down:left;up:left;key:Tab;text:hi"`（或等价的内置脚本）⇒ 确定性重放 ⇒ **打印最终状态**并断言（`hover/focus/pressed/texts` 与期望一致）。
- **像素证明**：渲染 `idle` / `hover` / `pressed` / `focused` 四个状态，与 CPU 后端对照（**0 / ≤1 LSB 不变**），并断言**状态差异只出现在期望的矩形内**（越界像素计数 = 0）。
- [ ] Step 1：实现 + 脚本重放断言。Step 2：跑 `DEER_VK_WINDOW_TESTS=1` 与 `DEER_VK_VALIDATION=1`。Commit。

### Task M5-5：文档

- `FEATURES.md`/`ROADMAP.md`（M5 部分完成：列出**已做**的交互与**仍未做**的停靠/多窗口/滚动）；新增 `docs/features/input.md`（事件模型、`hit`/`handle` 语义、如何写脚本重放、**已知边界**：无 shaping/无滚动/无 IME 预编辑）；`docs/TUTORIAL.md` 一章「让它能用」。
- [ ] Step 1：`docs_consistency` 5 passed + 示例 exit 0。Commit。

---

## Self-Review

**Spec 覆盖**：M5「输入与焦点」= M5-1（事件模型与接线）+ M5-2/3（命中与状态机）+ M5-4（闭环与可回归）；文档由 M5-5。**停靠明确不做**（登记）。
**Type consistency**：`InputEvent`/`Mods`/`Key` 在 M5-1（产出）与 M5-2/4（消费）一致；`UiState`/`UiEvent` 在 M5-2（产出）与 M5-4（消费）一致。
**风险登记**：① `NodeHint` 的真实语义未读清 ⇒ M5-3 Step 1 必须先读再定 `ClipSnapshot`（**不许猜**）；② 交互会让「渲染同一棵树」变成「渲染随状态变化的树」⇒ 像素判据必须按**状态分别**对照，而不是只对照一帧；③ `Tab` 焦点顺序必须是**确定的**（树序），否则不可回归。
