# 项目状态（供新会话/压缩后接续 —— 只读这个就够）

> **用法**：新会话从这里开始，**不要**去读历史对话。需要细节时按下面的路径去读文件。
> 更新时间：本轮（M5c 已推送后）。

## 1. 代码状态

| 项 | 值 |
|---|---|
| 远端 / 本地 `master` | 见 `git log --oneline -1`（本轮最后推送：`66ec218` 计划文件；`dc0ca23` = M5b/M5c/testkit 之前的里程碑收口） |
| 判据规模 | `cargo test --workspace` = **447 passed / 0 failed**（基准；任何一轮**不许退化**） |
| 已完成的里程碑 | M1 · M2a · M2b · M3a · M3b · M3c · M4（真字形）· M5（输入/焦点/重绘闭环）· M5b（事件驱动重绘 + 省电开关）· M5c（唤醒面 `Waker`/deadline）· M3+（统一管线 + 跨帧复用） |
| 零依赖口径 | 除窗口层 `winit 0.30` 外**零第三方依赖**（手写 Vulkan 绑定 + 自研 SPIR-V 汇编器） |

## 2. 在飞的工作（worktree / 分支 / 任务板）

```powershell
git worktree list    # 看全部；下面只列"在飞"的
```

| 内容 | worktree | 分支 | owner | 任务板 |
|---|---|---|---|---|
| **第 0 项 testkit（紧急）** | `..\deer-gui-testkit` | `feat/testkit` | 一次性 worker（**不可 steer**） | — |
| 第 1 项 滚动 + 多行文本 | `..\deer-gui-scroll` | `feat/scroll-multiline` | 一次性 worker | — |
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
