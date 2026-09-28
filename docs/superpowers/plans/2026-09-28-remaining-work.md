# 剩余工作总计划（用户指定顺序）

> **For agentic workers:** 本文件是**跨轮次的工作清单与纪律**。每一项走同一套：**设计（含决策 subagent 审批）→ 实现（独立 worktree）→ 独立复审 → 冻结树四口径门禁 → 合并推送 → 收口报告**。
>
> **决策规则（用户授权）**：过程中遇到「本需要问用户」的情况（接口取舍、阈值取值、优先级冲突、发现旧结论被推翻等），**开一个 subagent 让它决定并留下理由**，不要阻塞等待用户。仅以下情况必须停下来问用户：**不可逆/破坏性操作**、**安全敏感**、**整条计划被打断**。

## 纪律基线（每一步都要满足）

- 像素判据：**不透明逐字节 0 / 半透明 ≤1 LSB**；四状态**越界期望矩形 = 0**。
- 四口径门禁：默认 / `set "DEER_VK_WINDOW_TESTS=1" &&` / `set "DEER_VK_VALIDATION=1" &&` / `--workspace` + clippy 两条路径；**先 `cargo clean -p <crate>`**；**`example` 与 `cargo test` 不可并发**。
- 门槛必须**自证**（打印解析状态 + 验收 grep 标记），命令行用 `set "VAR=1" &&` 形态。
- 护栏必须**显式断言前置条件**；必须先 dump 真实数据再写期望值；变异必须**打中目标**且**确定性**变红。
- 独立 worktree 干活；提交用 `git commit -F <msg> -- <显式路径>` 一步到位 + `git show --stat HEAD` 核对。
- **不写死测试条数**；文档口径与实现同步（不许留反述）。

---

## 1. 滚动容器 + 多行文本

**为什么**：`layout.md:104` 明确写着「**没有多行文本节点**（`FontMeasure::wrap` 有换行能力，但只用于度量高度；`DrawCmd::Text` 仍是单个字符串 ⇒ 不产生多行绘制）、**没有滚动**」⇒ 这是「界面装得下内容」的最小前提。

**范围（拟，由决策 subagent 定稿）**
- 布局：可滚动容器的概念（内容高度 > 视口高度 ⇒ 记 `max_scroll`），滚动偏移参与几何计算；
- 绘制：`DrawCmd::Text` 支持**多行**（或新增多行文本命令）；裁剪栈复用既有 `PushClip/PopClip`；
- 交互：滚轮事件驱动偏移（现在 `Wheel` 已被解析但**未消费**）、滚动条（可选，若做要进焦点序决策）、命中与滚动偏移一致（**裁剪后的点不命中**这条既有语义要延伸到滚动视口）；
- 必须解决：**滚动会不会动既有像素判据**（不动容器时输出必须逐字节不变）。

**验收（可数）**：滚动前后同一语料的绘制命令/几何数字；滚到边界不越界；`FRAMES` 一致；离线 GPU vs CPU 仍 0 / ≤1 LSB；多行文本行数与换行点有断言。

## 2. 两条已知缺陷

**2a. `node_id_len` 校验和盲区**（`ClipSnapshot::from_draw_list`）：只比长度 ⇒ **等长 id 互换不 panic 且静默给错快照**。修法需让 id 参与校验（例如 `NodeHint` 携带 id 的哈希/索引），**属绘列表契约变更** ⇒ 要评估对 `DefaultRenderer`/`InteractiveRenderer`/既有语料的影响。

**2b. `Field` 焦点环贴边**（`crates/deer-gpu/src/interact.rs`）：与按钮同源的 **32 px 方角补块**（直角描边压在圆角填充上）。改它会动 `typed` 档既有数字 ⇒ 需同步更新下限；**判据必须保持「像素下限 + 平均通道差 ≥ 64 + 越界 = 0」的强度**。

## 3. M5c 独立复审（欠账）+ ①②③ 的 fix 轮

- **复审对象**：`58271cb`（唤醒面：`Waker`/`wake_after`/`next_deadline`/`iters`）。重点：`wake_after` 的「唤醒 = 重绘」计数相等、变异②**打不到**的那两格是否真的打不到、`iters` 判据能否被绕过、`Send`/`Sync` 巧合结论的复核。
- **fix 轮**：复审 findings + 第 1、2 项的定向修复。

## 4. hinting / 亚像素 → 文本清晰度；间接绘制 / 纹理 → 性能与图形能力

- **hinting / 亚像素**：`glyph-raster.md:182` 的边界；先在**离屏 PNG** 上做前后对比（可数：边缘灰阶分布、字形覆盖率差异），并保证**不破坏**既有像素判据（要么新开关，要么证明现有语料不受影响）。
- **间接绘制 / 多批次提交**：把统一管线推到 `vkCmdDrawIndexedIndirect` 级别；度量用**可计数**（draw 数 / 提交次数 / 每帧分配），**不用 fps**。
- **通用纹理**：`gpu-hal.md:155` 的「纹理上传」缺口（非 R8 字形图集的通用 RGBA 纹理）。

## 5. M6 控件族

从 `deer-ui` 迁移 15 个控件的**语义**：`Btn`/`ChipGroup`/`Segmented`/`TabBar`/`Switch`/`NumberField`/`ScrubNum`/`ColorField`/`Dialog`/`Overlay`/`DropMenu`/`HoverTip`/`Icon`/`Row`/`RowActions`/`Keep`。
**前置**：第 1 项（滚动/多行）与**键盘可达性**（方向键导航、`hit-testing.md:71` 的缺口）宜先落地，否则 `TabBar`/`DropMenu`/`Dialog` 的语义无处安放。**每个控件一条交互单测 + 一条像素判据**。

---

## 已知边界（**不要**当作已完成，也不要顺手"修"成别的语义）

停靠（dock）/ 多窗口 · IME 预编辑 · 按键重复未建模 · `texts` 无光标位置 · 事件回调式 API（现在必须整树重建）· 自定义着色 · 推送常量在 Intel 不可用 · 非 Windows 窗口未实现 · 单窗口测试基建限制 · 校验层零消息**不能**证明内存域依赖正确 · `Occluded(false)` 在 Windows 未实测 · 真窗口 `DEER_IDLE_DIRTY=1` 变化档只报数不下结论 · 窗口侧 host→vertex 屏障只有计数没有断言 · README「15 runnable examples」数字已漂。
