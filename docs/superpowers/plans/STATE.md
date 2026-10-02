# 项目状态（供新会话/压缩后接续 —— 只读这个就够）

> **用法**：新会话从这里开始，**不要**去读历史对话。需要细节时按下面的路径去读文件。
> 更新时间：分层物理化（L0–L3）完成并推送后。

## 1. 代码状态

| 项 | 值 |
|---|---|
| 远端 / 本地 `master` | 见 `git log --oneline -1`（最近推送：`0013ca7` = L2+L3 落地） |
| 判据规模 | `cargo test --workspace` = **660 passed / 0 failed**（L2+L3 落地轮实测；口径以运行输出为准，任何一轮**不许退化**） |
| 已完成的里程碑 | M1 · M2a · M2b · M3a · M3b · M3c · M4（真字形）· M5（输入/焦点/重绘闭环）· M5b（事件驱动重绘 + 省电开关）· M5c（唤醒面 `Waker`/deadline）· M3+（统一管线 + 跨帧复用）· **分层物理化（L0–L3，Godot 式）** · L2 align-self · L3 min/max 尺寸 |
| 零依赖口径 | 除窗口层 `winit 0.30` 外**零第三方依赖**（手写 Vulkan 绑定 + 自研 SPIR-V 汇编器） |

## 2. 在飞的工作（worktree / 分支 / 任务板）

```powershell
git worktree list    # 看全部；下面只列"在飞"的
```

| 内容 | worktree | 分支 | owner | 任务板 |
|---|---|---|---|---|
| （无在飞 —— 五项计划与分层物理化均已合并推送） | 见 `git worktree list`（多数为历史现场，可清理） | — | — | task-34/39/40–45 均 completed |

### 分层物理化（最新一轮，已推送至 `6ad26bc`）

- **拓扑**：`deer-core`（L0 = 原 deer-layout + draw/error）· `deer-text`（L1 TextServer = font/glyph/raster/atlas/text/measure/png）· `deer-gpu`（L1 RenderServer·CPU + HAL traits 暂驻）· `deer-vk` · `deer-window`（内部 display/host）· `deer-gui` · `deer-log`
- **关键裁定**（ROADMAP 登记 ⑨⑩）：HAL traits 暂驻 deer-gpu（`Frame::record` 签名引用 `TextEngine`，RID 化后才下沉 L0）；`FontMeasure` 归 TextServer。门禁盲区（`required-features` 示例不在 `--workspace` 编译面）登记于 CONTRIBUTING Verification 节。
- **终验**（lead 执行，verifier 故障改由 lead 顶）：四口径 + feature 口径全绿；`window_parity` 形状/不透明逐字节 0、半透明 ≤1；M-A 迁移后复验 = 恰好 5 红（deer-core 2 + deer-gui 3，与原指纹一致）。
- **L1 绝对定位/层叠已落地**（`feat/l1-position` @ `3f57ba1`，**glm-5.3-flash workflow 团队**交付：实现 + 独立验证双变异互补放行）：`LayoutProps.position: Option<Pos>`（本轮仅 `Offset`，opt-in 流外布局 + 声明序层叠；`hit_test` 零改动靠声明序先序遍历）；639/0。遗留：anchors=L4；`Pos` 形状与 D10 登记字差异待 ROADMAP 补裁定注（docs 待办）。
- **L2/L3 已落地**（`feat/l2-l3-sizes` @ `110f405`，glm-5.3-flash workflow 第二期）：`cross_self: Option<Align>`（流内覆盖容器级交叉轴）+ `min_w/max_w/min_h/max_h`（**measure/place 双站点**夹取，min>max ⇒ min 赢，grow 被 max 封顶不二次分配）；660/0，验证者四变异独立复现（含「单点删除被冗余防线吸收」的发现）。遗留：L4 anchors（L3 已就位）、`docs/features/layout.md` §6「没有绝对定位」过时句待清。
| 第 2 项 `node_id_len` 盲区 + `Field` 焦点环 | `..\deer-gui-defects` | `feat/known-defects` | 一次性 worker | — |
| M5c 复审（欠账） | `%TEMP%` 克隆 | — | 一次性 reviewer | — |
| 常驻复审岗 | 只读 | — | **`verifier`** | **task-34 (t36)** |
| 第 4 项上 hinting/亚像素 | `..\deer-gui-hinting` | `feat/hinting-subpixel` | **`raster-dev`** | **task-35 (t37)** |
| 第 4 项下 纹理/间接绘制 | `..\deer-gui-indirect` | `feat/indirect-texture` | **`vk-surface-dev`** | **task-36 (t38)** |

**待命队友**（可 `send_message` 唤醒）：`impl-geom`（渲染路径）、`impl-shader`（SPIR-V）、`window-dev`（窗口层）、`metric-atlas-dev`（度量/图集）、`docs-dev`（文档）。**队友上限 8，已满 ⇒ 复用，不要 `spawn_teammate`。**

## 3. 关键约束（**硬性，不许违反**）

1. **像素判据**：不透明**逐字节 0** / 半透明 **≤1 LSB**；四状态**越界期望矩形 = 0**。
2. **门槛必须自证**：门控用例/示例要**打印解析到的门槛状态**，验收 **grep 该标记**，**不能只看退出码**；命令行用 **`set "VAR=1" &&`（带引号）** —— 不带引号会把 `&&` 前的空格算进值（`"1 "`）⇒ 门槛**静默失效**（已实际废掉过一整档验证）。
3. **护栏必须显式断言前置条件**（前置不成立**不会报错** ⇒ 护栏悄悄失效；已发生 4 次）。
4. **变异必须打中目标**（改坏实现 ⇒ 对应断言**确定性变红**）；「跑了没报错」≠「守住了」。
5. **`example` 与 `cargo test` 不可并发**（退出码不可信）。
6. **就地还原/变异后必须 `cargo clean -p <crate>`**（mtime 陷阱 ⇒ 不重编 ⇒ 结论完全错）。
7. **提交用 `git commit -F <msg> -- <显式路径>` 一步到位 + `git show --stat HEAD` 核对**（共享工作区里裸 `commit` 会吞别人的暂存内容）。
8. **不用 shell 文本管道改源码**（UTF-8 被按 ANSI 解码 ⇒ 中文变 `?` 不可逆；已 3 次事故）。
9. **零新增第三方依赖**；**不跑 `cargo fmt`**（仓库非 fmt-clean）。

完整版（含 11 条纪律与真实案例）：`CONTRIBUTING.md`。

## 4. 总计划（0–5 项与验收口径）

`docs/superpowers/plans/2026-09-28-remaining-work.md` —— **第 0 项（testkit，紧急）+ 1–5 项 + 纪律基线 + 已知边界清单**。

## 5. 未决事项（需要人或上层会话处理）

1. **`test_project/deer-hello`**：使用者私有测试工程，里面有**另一个 agent 会话在持续改写 `src/main.rs`**（每约 2 秒一次）。可用版本已放在 `test_project/deer-hello/src/main.rs.new`（sha256 `CEBF93DE…8747`，实测：编译 0 warning、`--headless` 缺字形 0、`--frames 30` exit 0、PNG 中文非方块）。**是否落盘由使用者决定**。
2. **`crates/deer-layout/src/builder.rs`** 有 **+5 行未提交改动**（非我方任何 worker 所改）⇒ 需 review 或回退。
3. **M5c 唤醒面（`58271cb`）缺独立复审** —— 已派一次性 reviewer 补做，裁定与结论落 `.superpowers/sdd/m5c-review.md`。
4. **testkit 的 8 项设计裁定**：`.superpowers/sdd/testkit-decisions.md`（由 lead 派的决策 subagent 产出；因 depth 限制，depth-1 执行者无法自行派生裁定者）。

## 6. 机制限制（**重要**，避免重蹈覆辙）

- **一次性 `subagent` 不能被 `send_message` 追发**，且**不能再派生 subagent**（`depth 2 exceeds maxDepth 1`）⇒ 它一旦派出就**无法中途纠偏**。⇒ 首轮实现可用一次性 worker，**复审与修复轮必须交给队友**（可 steer）。
- **队友上限 8**，已满。
- **`git status` 干净 ≠ `target/` 干净**；**feature 组合也是缓存键**（不带 `--features window` 的构建不会重建窗口示例）。

## 7. 本轮我（lead）的主要失误（供后人避免）

1. **把门禁证据建立在空转的门槛上**（`"1 "` 尾空格 + 严格比较）⇒ 报过的「开开关档」长期无效（已修 + 已立规矩）。
2. **在派活前没问「这个目录还有谁在写」** ⇒ deer-hello 交付被并发写者反复踩掉。
3. **整块 dump 工具输出**（`team_task_list` 一次吐 39 KB 溢出到临时文件）⇒ 污染上下文。

## 8. 本轮进度（滚动更新 —— 以 `git log` 与任务板为准）

**已进主干并推送**（`master` = 见 `git log --oneline -1`）：
- **第 2 项** 两条已知缺陷 → `570e917`：`NodeHint` 增 `node_id_fp`（FNV-1a 64）堵住「等长 id 互换静默错快照」；`Field` 焦点环内缩（框内 684→570、**环带外 464→0**、方角补块 32→0）。
- **第 4 项上** 亚像素定位 → `a2bfab6`：`rasterize_at`/`rasterize_char_at` + `split_subpixel_x`（opt-in；落位 RMSE **0.2890→0.0733 px**；**代价**：部分覆盖质量 `l` +64.3% ⇒ **间距精度换边缘锐度**；**旧默认 20/20 产物逐字节不变**；hinting 仍不做但有实测依据 + 可重评棘轮）。
- 文档：`6d77a53`（`iters` 降级为观测值）、`380b928`（亚像素口径 9 处副本）、`7ec38eb`（去掉写死的示例/套件计数）。
- **判据规模**：`cargo test --workspace` = **462 passed / 0 failed**（本会话起点 205）。

**进行中**（4 名队友在跑）：
| 内容 | owner | 任务板 |
|---|---|---|
| testkit 修复轮（**D6 绕过 HIGH-1** + `DEER_VK_FRAMES` 双口径） | `impl-geom` | task-38 (t40) |
| M5c fix 轮（`iters` 二选一 + OnDemand 档假红与误诊） | `window-dev` | task-37 (t39) |
| 第 4 项下 纹理 / 间接绘制 | `vk-surface-dev` | task-36 (t38) |
| 常驻复审（四条已出结论；等 scroll/indirect 落地） | `verifier` | task-34 (t36) |

**未落地**：`feat/scroll-multiline`（**第 1 项**，一次性 worker，队友面板不可见 ⇒ 只能等它的消息）、`feat/testkit`（`62339a6`，**待 HIGH-1 修复后合并**）。

**停泊**：`docs/features/testing.md:144`（「非法即报错、不静默」那句）—— **该文件只在 testkit 分支上**，等 testkit 合并进 master 后由 `docs-dev` 改。

**等你（使用者）一句**：`crates/deer-layout/src/builder.rs` 的 **+5 行未提交改动**（非任何 agent 所派）—— 保留还是回退。另：`test_project/deer-hello` 的并发写者与可用版 `src/main.rs.new`（sha `CEBF93DE…`）仍未裁决。
