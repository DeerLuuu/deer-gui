# 多窗口（T4.4）

## 1. 这是什么 / 什么时候用它

**在一个进程里开多扇真窗口**：主窗之外经 `WindowSpawner` **动态 spawn** 新窗，事件按窗路由
（每扇窗一个 id，输入/重绘/尺寸/关闭都带 id），渲染侧所有窗口**共享一个 `VkDevice`**、
各持各的 surface/swapchain/帧资源（决策 3「Godot 同款」），关闭语义是「只关该窗、全关才退出」。

**什么时候不用**：只是想换内容/换场景 —— 改同一扇窗的树就够；需要跨窗拖放（DnD）、
父子窗口（owned）、窗口间消息传递 —— 这些**明确不做**（见第 6 节）。

## 2. 最小示例

```sh
DEER_VK_WINDOW_TESTS=1 DEER_VK_VALIDATION=1 cargo run -p deer-gui --features window --example dual_window
# 交互档：DEER_WINDOW_HOLD=1 cargo run -p deer-gui --features window --example dual_window
```

门槛档自动跑完整套出口判据并退出（动态 spawn → 双窗各自 parity → 各自收事件 →
关一窗另一窗存活 → 全关退出）；交互档窗口留着给人看，Esc 退出。完整可跑的骨架
（动态 spawn + 按窗路由 + 按窗状态机 + 双窗 parity + 关闭语义）见
`crates/deer-gui/examples/dual_window.rs` —— 本目录每份指南都直接指向真实示例源码。

关键组装形态（映射是**恒等式**，见第 5 节）：

```rust
// deer-window 的 WindowId::raw() 直传渲染层当表键 —— 两层同源，禁止 0/1 错位的隐式约定。
fn window_init(&mut self, id: WindowId, info: &WindowInfo) -> Result<(), String> {
    let key = id.raw();
    if self.renderer.is_none() {
        self.renderer = Some(WindowedRenderer::new_with_primary_id(adapter, key, info.raw, info.extent, clear)?);
    } else {
        self.renderer.as_mut().unwrap().add_window(key, info.raw, info.extent, clear)?;
    }
    Ok(())
}
fn window_redraw(&mut self, id: WindowId) -> Result<Flow, String> {
    self.renderer.as_mut().unwrap().draw_and_present_window(id.raw(), &list, Some(&mut engine))
}
```

## 3. 完整 API

### 窗口层（`deer_window`，特征名 App 的生命周期钩子）

| 钩子 | 时机 / 语义 | 默认实现 |
|---|---|---|
| `App::window_init(id, info)` | 每建一扇窗调一次（渲染器在这里创建） | 转发 `App::init` |
| `App::window_redraw(id)` | 该窗的 `RedrawRequested` 到达（只画这一扇） | 转发 `App::redraw` |
| `App::window_input(id, info, ev)` | 该窗的输入事件（T4.4-R1） | 转发 `App::input` |
| `App::window_resized(id, w, h)` | 该窗尺寸变化（物理像素） | 转发 `App::resized` |
| `App::window_close_requested(id)` | 用户点了该窗的 X；返回 `Flow::Exit` = **允许关这一扇**（不是退出事件循环） | 转发 `App::close_requested` |
| `App::window_destroyed(id)` | 该窗已从活窗表移除（释放该窗渲染资源的地方） | 什么都不做 |
| `WindowId::raw() -> u64` | 本层自发序号：主窗 = 1，按建窗顺序自增；**直传渲染层当表键** | — |
| `WindowSpawner::spawn_window(config)` | **排队**建新窗（决策 2：真正的建窗在事件循环安全点）；句柄经 `App::window_spawner` 交付（主窗 init 之后一次） | — |

**「默认转发」只适用于有旧方法对应的四个钩子**（`window_init`/`window_redraw`/`window_input`/
`window_resized`/`window_close_requested` —— 覆盖后对应的旧方法（`init`/`redraw`/`input`/`resized`/
`close_requested`）不再被调用，转发只发生在默认实现里，两条路不能同时响）；
**`window_destroyed` 是纯新增钩子**：没有旧方法可转发，默认实现什么都不做（单窗口的收尾本来就随
事件循环结束一起发生），多窗口 App 覆盖它来释放该窗的渲染资源。单窗口 App 一行不用改。

### 渲染层（`deer_vk::windowed::WindowedRenderer`，按 `WindowId` 索引的窗口表）

| API | 语义 |
|---|---|
| `new_with_primary_id(adapter, id, window, want, clear)` | 开渲染器，主窗链占用**调用方给的** `id`（同源直传；保留键 0 留空 ⇒ 「0 在表 = 错位」是现成回归判据） |
| `add_window(id, window, want, clear)` | 新窗入表（共享同一 `VkDevice`/实例/字形图集；先验证该设备的呈现队列族支持新 surface；重复 id 明确报错） |
| `draw_and_present_window(id, list, text)` | 画**这一扇**窗并呈现 |
| `resize_window(id, extent)` | 只重建**这一扇**的交换链 |
| `read_back_last_frame_window(id)` | 回读**这一扇**刚呈现的帧（RGBA8）—— 双窗各自 parity 的判据 |
| `remove_window(id) -> bool` | 整条链移出并销毁（先 `vkDeviceWaitIdle` 再销毁；幂等） |
| `window_ids() / window_count() / contains_window(id)` | 表内诊断 |
| `extent_of / format_of / present_mode_of / image_count_of / frames_presented_of(id)` | 每窗读数（`Result`，id 不在表里明确报错） |

既有的单窗口 API（`new` / `draw_and_present` / `resize` / `read_back_last_frame` / 各种读数）
作用于 `new()` 建的主窗（保留键 `0`），签名与行为不变。

## 4. 自检（怎么确认你真的用对了）

门槛档（`dual_window`）把下面每条都断言了；自己集成时至少照着查：

```rust
// ① 两层同源：渲染表键 == 窗口层 id.raw()，保留键 0 不在表里（0 在 = 错位 = 串链）
assert_eq!(renderer.window_ids(), vec![1, 2]);
assert!(!renderer.contains_window(PRIMARY_WINDOW_ID));
// ② 各自渲染：两窗首帧各自回读，与 CPU 对**同一份列表**的渲染逐字节比（线性附件 ⇒ 差为 0）
assert_eq!(max_diff_window_a, 0);
assert_eq!(max_diff_window_b, 0);
// ③ 各自收事件：点 A 的按钮只让 A 的状态机出 Clicked，B 同理
assert_eq!(clicks[1], 1);
assert_eq!(clicks[2], 1);
// ④ 关一窗另一窗存活：remove 后另一窗仍能呈现且回读正确
assert!(!renderer.contains_window(2));
```

`dual_window` 门槛档跑法：`DEER_VK_WINDOW_TESTS=1 DEER_VK_VALIDATION=1 cargo run -p deer-gui
--features window --example dual_window` → `exit=0` 且**零校验消息**（校验层会把「在用对象被销毁」
这类错误当场抓出来 —— 别在关校验层的口径下验收）。

## 5. 常见坑

| 现象 | 原因 | 怎么改 |
|---|---|---|
| 主窗/第二窗的像素互串（A 窗出现 B 的内容、或 parity 红） | 两层编号不同源：窗口层主窗是 1、渲染层旧构造器保留键是 0，靠「恰好错位对上」的隐式约定迟早串链 | 用 `WindowId::raw()` **直传**：`new_with_primary_id` / `add_window(id.raw(), …)`；把「保留键 0 不在表里」写成断言 |
| 覆盖了 `window_redraw` 之后单窗回调还响（或反过来） | 转发只发生在**默认实现**里：覆盖 `window_*` 钩子后旧方法不再被调；不覆盖则走旧方法 | 多窗口 App：覆盖 `window_*` 系列，旧方法留空实现即可（`init`/`redraw` 是必选方法） |
| `CloseRequested` 返回 `Flow::Exit` 把整个应用退了 | `window_close_requested` 里 `Flow::Exit` 的语义是「**允许关闭这一扇**」，不是「退出事件循环」；全关才会退出（决策 4 由窗口层保证） | 想否决关闭返回 `Flow::Continue`；想按窗拒绝/允许就按 id 分支 |
| 多窗下某扇窗「只画一帧就不动了」（尤其它触发过 `spawn_window`） | winit 0.30（Windows）的 `request_redraw`（`RDW_INTERNALPAINT`）在「续帧请求恰逢建窗的嵌套消息泵」时会被吞；per-window 的请求投递在多窗 + 建窗竞态下不可依赖 | 帧供给改成「任一扇窗收到请求 ⇒ 全部活窗各画一帧」（`dual_window` 的做法），或用 `Waker::wake_after` 做应用级全窗节拍兜底 |
| 校验层报 `vkDestroyBuffer … in use by VkCommandBuffer` | 内容变化触发**顶点缓冲容量增长**时，旧缓冲在 prepare 阶段被销毁，而上一帧的命令缓冲还在飞（窗口渲染器已修：增长前排空；若你自管缓冲请注意同一条纪律） | 升级到含修复的版本；自管 Vulkan 资源时「先 `wait_idle`/等栅栏，再销毁」 |
| 关窗时校验层报 `vkDestroySemaphore/SwapchainKHR … in use by VkQueue` | 在飞提交未完成就销毁该窗资源 | 用 `WindowedRenderer::remove_window(id)`（内部先 `vkDeviceWaitIdle` 再整链析构），不要自己拆 |
| 窗口销毁了还收到它的输入/重绘回调 | 销毁竞态下的迟到事件（正常现象） | 回调里按「表里没有该窗就丢弃」处理（窗口层对未知窗已经这么做了；App 侧查自己的每窗状态表） |

## 6. 相关

- 单窗口路径与重绘策略：[`window.md`](window.md)；输入/状态机：[`input.md`](input.md)；
  上屏 parity 的判据口径：[`window.md`](window.md) 第 5 节与 `window_parity` 示例。
- 交互状态机（每窗一个 `UiState`）与脚本重放：[`input.md`](input.md)、[`testing.md`](testing.md)。
- 设计登记（八项决策 + 三步切分）：`ROADMAP.md`「设计登记：多窗口（T4.4）」。

**做不到什么**（决策 8，登记在案，不是「还没做」）：

- **跨窗口拖放（DnD）**：不做。
- **owned / 父子窗口**（模态、置顶跟随）：不做。
- **窗口间消息传递**（框架层）：不做 —— 窗口是平级的，App 自己持有每窗状态即通信。
- **每窗独立 GPU 实例 / 独立设备**：不做 —— 所有窗口共享一个 `VkDevice`（决策 3）。
- **多线程渲染**：不做 —— 单线程轮转（决策 7；Q-4 线程模型仍悬置）。
- 平台边界：原生句柄→HAL 句柄只有 **Windows** 实现（与窗口层同一条边界）；
  `HAL` trait 的 `begin_frame` 没有窗口参数（改 trait 属公开 API 变更需先登记），
  HAL 层的帧固定作用于主窗 —— 多窗帧提交走 `WindowedRenderer` 的 `*_window(id)` API。
- 唤醒是 **App 级**的（`Waker` 唤醒会让**所有**活窗重画），没有按窗定向唤醒。

## 7. 检查清单（发布前过一遍）

- [x] 示例能跑：`DEER_VK_WINDOW_TESTS=1 DEER_VK_VALIDATION=1 cargo run -p deer-gui --features window --example dual_window` → `exit=0`
- [x] 示例有自检断言（双窗 parity 逐字节 0、映射判据、按窗点击到账、关一窗另一窗存活、全关退出）
- [x] `FEATURES.md` 已登记（状态 / 指南链接 / 示例命令都对）
- [x] 不属于新手主线（`docs/TUTORIAL.md` 仅修正「做不到多窗口」的过期表述，不新增章节）
- [x] 明确写了「做不到什么」（决策 8 的五项 + 平台/HAL/唤醒边界）
