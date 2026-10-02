# deer-gui 后续更新规划书

> **文档定位**：项目级更新规划（路线 / 规范 / 风险 / 分工）。功能状态以 [`FEATURES.md`](../FEATURES.md) 为唯一真相，任务级验收标准见 [`DEV-PLAN.md`](DEV-PLAN.md)，现状架构见 [`ARCHITECTURE.md`](ARCHITECTURE.md) —— 本文**不重复**它们的内容，只做总纲与绑定；冲突时以那三份 + [`ROADMAP.md`](../ROADMAP.md) 为准。
> **基线**：HEAD `36e7b93`（2026-09-29 静态审阅）；本文遵守仓库口径 —— 不固化测试条数（以运行输出为准），不写「完全零依赖」。

## 0. 文档族与真相源（防漂移约定）

| 文档 | 角色 | 谁维护 | 更新时机 |
|---|---|---|---|
| [`FEATURES.md`](../FEATURES.md) | 功能状态唯一真相 | 每个功能作者 | 四件套提交内 |
| [`ROADMAP.md`](../ROADMAP.md) | 里程碑 M1–M7 与验收判据 | 维护者 | 里程碑收口时 |
| [`ARCHITECTURE.md`](ARCHITECTURE.md) | 现状架构（as-is） | 架构改动者 | 大的结构变化后 |
| [`DEV-PLAN.md`](DEV-PLAN.md) | 任务级计划：D1–D12 技术债、Phase 0–6、验收标准 | 维护者 | 阶段收口时 |
| **本文（UPDATE-PLAN.md）** | 总纲：路线 / 规范 / 风险 / 分工 | 维护者 | 每阶段复盘时 |
| [`CONTRIBUTING.md`](../CONTRIBUTING.md) | 面向人的提交与评审规矩 | 维护者 | 新坑入册时 |

---

## 1. 现状与目标

### 1.1 当前版本

| 项 | 值 | 出处 |
|---|---|---|
| 版本 | `0.0.0`（未发布 crates.io，path 依赖消费） | `Cargo.toml` |
| 工具链 | Rust ≥ 1.85，edition 2024，`Cargo.lock` 入库 | `Cargo.toml`、`AGENTS.md` §1 |
| 依赖口径 | 除窗口层（`winit 0.30`，Q-1 已登记）外**零第三方依赖** | `ROADMAP.md` 依赖纪律 |
| 平台 | 布局/离屏全平台；Vulkan 离屏全平台；**窗口与呈现仅 Windows** | `README.md` |
| 已交付里程碑 | M1 · M2a · M2b · M3a/b/c · M3+（统一管线+跨帧复用）· M4（真字形）· M5/M5b/M5c（输入/焦点/省电重绘/唤醒） | `ROADMAP.md` 里程碑表 |

### 1.2 已有功能范围（摘要，状态以 FEATURES.md 为准）

| 层 | 能力 | 状态 |
|---|---|---|
| 构筑 | 命令式 Builder / `.dui` 场景文件 / 两路同树（结构相等不变式） | ✅ |
| 布局 | 布局代数（I-1…I-8 不变式）、对齐/百分比/grow、多行文本、滚动容器、命中测试 | ✅ |
| 渲染 | CPU 参考后端、Vulkan 后端（自研 FFI+SPIR-V）、离屏 PNG、GPU 几何+文本（与 CPU 逐像素对照）、窗口上屏、统一管线（1 draw/帧） | ✅ |
| 文本 | TTF 解析、光栅化、图集、真实度量换行；亚像素定位 opt-in（未接引擎） | ✅ / 🔄 |
| 输入 | 事件通路、命中/状态机、Tab 焦点、文本输入、脚本重放、事件驱动重绘、滚轮垂直滚动、**可视滚动条（拖滑块 + 点轨道跳转）**、**惯性驱动收口**（T3.2/T3.2b ✅）、**输入框光标**（T3.5 + T3.8 渲染竖线）、**IME 预编辑**（T3.4）、**方向键导航/按键滚动/右键透传/按键重复**（T3.1–T3.6 ✅） | ✅ **输入地基全清** |
| 待补 | 通用纹理/间接绘制（**缺指南与示例** ⇒ 🔄） | 🔄 |
| 未做 | M6 控件族（现仅 5 种节点）、M7 DX12/Metal、多窗口/dock、~~纹理 RGB 调制~~（T1.3 ✅）、~~方向键导航/按键滚动/右键/按键重复~~（T3.1–T3.6 ✅） | ⬜ |

### 1.3 后续迭代核心目标与优先级依据

| # | 目标 | 一句话定义 | 优先级依据 |
|---|---|---|---|
| G1 | **HAL 契约收敛** | `Frame::record` 真正消费 `DrawList`，消除「UI 渲染走 `draw_and_present` 直连」的分叉 | **阻塞 M7**（DX12/Metal 的硬前置）；架构级债（`hal.rs:168`） |
| G2 | **交付物补全** | 通用纹理/间接绘制补指南+示例，🔄→✅；亚像素接入或登记不做 | 成本最低、收益即时（代码已就绪，纯文档+示例） |
| G3 | **输入面完善** | 方向键导航、滚动条/惯性、右/中键、IME、光标、按键重复 | M6 控件（`Switch`/`NumberField`/`DropMenu`）的地基 |
| G4 | **窗口层跨平台** | X11/Wayland/macOS 句柄填法 + 对应 surface | 消除「仅 Windows」的平台叙事缺口；可与 G1 并行 |
| G5 | **M6 控件族** | 12+ 个 `deer-ui` 控件语义迁移 | 产品价值主线；依赖 G3（输入）+ G1（稳定性）+ T1.3（Icon 需纹理） |
| G6 | **M7 后端扩展** | DX12 / Metal 实现 HAL trait | 长期；硬依赖 G1 完成 |

**排序原则**（按序应用）：① 阻塞链优先（G1 → G6）；② 摘果子优先（G2 先于一切新能力）；③ 产品缺口优先于平台广度（G3/G5 先于 G4）；④ 每项均受「四件套」交付纪律约束（见 §3）。

---

## 2. 迭代路线

> 时间为**相对排期**（S/M/L 累计），日历节点按「1 名全职开发」折算；人力配置变化时须整体平移（见 §6.3 跟踪机制）。任务编号沿用 `DEV-PLAN.md`。

### 2.1 短期（约 2026-10 ～ 2026-11，Phase 0 + Phase 1）

| 周期 | 任务 | 交付物 | 里程碑判据 |
|---|---|---|---|
| W1–W2 | T0.1 纹理指南+示例、T0.2 间接绘制指南+示例、T0.3 亚像素决策 | `docs/features/textures.md`、`indirect-draw.md`、`examples/textures.rs`、`indirect_draw.rs`、`FEATURES.md` 两行 🔄→✅ | `docs_consistency` 全绿；示例 `exit=0` 带自检断言 |
| W2–W3 | T1.2 HAL 纹理契约（`create_texture`/`upload_texture` 接已落地实现） | `hal.rs` 两方法实现 + 单测 | HAL 路径建/传/回读纹理四通道保真 |
| W3–W6 | **T1.1 HAL `record` 接线**（含 `TextEngine` 传递契约设计评审） | `VulkanFrame::record` 消费 DrawList；`hal_window_path` 走 UI 像素 | `record(形状+文本)` 真窗出像素；`window_parity` 判据不变（不透明逐字节/半透明 ≤1 LSB） |
| W6–W8 | T1.3 纹理 RGB 调制 FS + 窗口路径入口；T1.4 `read_pixels` 语义决策 | 新片元着色器（过 `spirv-val`）、`draw_textured_quad` 窗口入口 | 离屏纹理 quad 与 CPU 参考逐字节对照 |
| 收口 | **E1：HAL 收敛完成** | 版本升 `0.1.0-rc`，`ROADMAP.md` 登记设计决策 | 全部四口径门禁 0 failed（见 §3.4） |

### 2.2 中期（约 2026-12 ～ 2027-02，Phase 2/3/4 可并行）

| 周期 | 任务 | 交付物 | 里程碑判据 |
|---|---|---|---|
| M1 ✅ | T3.5 `texts` 光标 → T3.4 IME 预编辑（**另补 T3.8 光标渲染** —— 执行中发现「光标从未被画出来」，否则 T3.4 的「预编辑可见」写不出判据） | `InputEvent::ImePreedit`、光标建模、Field 光标渲染 | 中文输入法预编辑可见、Commit 一次上屏；多字节边界单测 |
| M2 ✅ | T3.1 方向键导航（D2 几何邻近）、T3.6 按键重复（`repeat: bool`）、T3.3 右/中键（Q1 纯透传 `PointerRight`） | `handle` 扩展 + `input.md` 更新 | 纯逻辑单测全绿（无窗口） |
| M2 ✅ | T3.2 全部：滚动条（拖滑块 + 点轨道跳转）+ 惯性滚动（T3.2b 收口 `advance_inertia`/`inertia_deadline` + 真窗口参考实现） | 可视滚动条命中/拖动、`Waker::wake_after` 惯性衰减 | 惯性停在边界内；唤醒账本证明无空转 |
| M1–M2 | T2.1 X11 句柄 + surface（有 Linux 环境窗口期插入） | `raw_handle_from_rwh06` X11 臂 | Linux `window_preview` 开窗呈现、`window_parity` 通过 |
| M2 | T4.1 `unify` 零分配、T4.2 线程模型决策（Q-4）、T4.3 示例工程 path 修复 | 稳态零分配；Q-4 决策记录；可移植 path 方案 | `RenderStats` allocs=0；像素判据不变 |
| M3 | T4.4 多窗口基建 | 渲染器多交换链 + 事件路由 | 两窗口各自 parity；`ROADMAP.md` M3+(b) 勾掉 |
| 收口 | **E2：输入面完整 + 多窗口**；**E3：Linux 窗口可用** | `FEATURES.md` 第四节输入清单清空；版本 `0.1.0` 发布评估 | 见 §3.3 发布判据 |

### 2.3 长期（2027-03 起，Phase 5/6）

| 阶段 | 任务 | 交付物 | 里程碑判据 |
|---|---|---|---|
| 5a–5c | 基础/布局/选择类控件（`Btn` 语义对齐、`Row`/`Keep`/`Overlay`/`Dialog`/`HoverTip`/`Segmented`/`ChipGroup`/`TabBar`） | `Kind` 扩展或组合层 + `build_draw_list` 分支 + 每控件四件套 | testkit 可注入输入并断言状态；后端零改动 |
| 5d–5f | 数值/图标/菜单类控件（`NumberField`/`ScrubNum`/`Switch`/`ColorField`/`Icon`/`DropMenu`） | 同上（`Icon` 依赖 T1.3，`DropMenu` 依赖 T3.3） | 同上；**M6 收口** |
| T6.1 | DX12 后端（新 crate `deer-dx`） | COM 符号加载、设备/交换链/管线、HAL trait 实现 | 与 CPU 逐像素 parity（复用 testkit `GpuProbe`）；四件套 |
| T6.2 | Metal 后端（或登记 MoltenVK 路线降级） | `deer-mt` 或决策记录 | 同上；**M7 收口** |

### 2.4 里程碑与交付物对照

| 里程碑 | 内容 | 前置 | 发布物 |
|---|---|---|---|
| **E1** | HAL 契约收敛（Phase 0+1 全部） | — | `0.1.0-rc`；`ARCHITECTURE.md` 更新（分叉消除） |
| **E2** | 输入面 + 多窗口（Phase 3+4 核心） | E1 | `input.md` §6 清单清空 |
| **E3** | Linux 窗口（T2.1/2.2） | E1（可穿插） | 平台支持表更新 |
| **M6** | 控件族 12+ | E1 + Phase 3 对应项 | `FEATURES.md` 一/二节扩容；`TUTORIAL.md` 新章节 |
| **0.1.0** | 首个对外版本（crates.io 发布决策） | E1（API 面冻结审查） | `cargo publish --dry-run` 通过 + prelude 审查记录 |
| **M7** | DX12/Metal | **E1 硬阻塞** | 后端对照报告 + 指南 |

---

## 3. 开发规范

> 本节是 [`CONTRIBUTING.md`](../CONTRIBUTING.md) 的执行摘要 + 本计划新增的流程约定；冲突时以 CONTRIBUTING.md 为准。

### 3.1 分支管理策略

| 项 | 约定 |
|---|---|
| 主干 | `master`（默认分支），保持随时可发布状态：全部四口径门禁 0 failed |
| 特性分支 | `feat/<name>`（功能）/ `fix/<name>` / `test/<name>`（测试基建），从 `master` 切出 |
| Worktree | 多任务并行用 `git worktree`（仓库现状即此模式）；**动工前先 `git status`/`git log` 弄清在途改动归属**（`AGENTS.md` 规矩 7） |
| 合并 | PR → `master`（先例：PR #1 `test/window-barrier-assertions`）；PR 必须写明「验了什么（命令+环境+结果）/没验什么」 |
| 保护 | 不 `--force` 推主干；不覆盖他人暂存（共享工作区里裸 `commit` 会吞别人的暂存 —— 已有事故案例） |
| 清理 | WIP 分支合并/废弃后删除 worktree（残留分支有「落后基点合并会倒删功能」的风险，见 2026-09-29 `feat/scroll-multiline-2` 案例） |

### 3.2 提交与评审流程

| 环节 | 规则 |
|---|---|
| 提交信息 | Conventional Commits + 中文：`feat|fix|docs|test|chore(范围): 描述`；**修 bug 正文写根因**，不只写改了哪行 |
| 提交方式 | **一步到位**：`git commit -F <msg> -- <显式路径>`，随后 `git show --stat HEAD` 核对恰好是预期文件；错了 `git reset --soft HEAD~1` 重来（**绝不 `--hard`**） |
| 评审门（PR 前） | ① 最小相关门禁通过并**写明跑的是哪个**；② 新行为带齐四件套；③ 行尾 LF（`.gitattributes` 强制）；④ 未手改 `Cargo.lock`；⑤ 新依赖已登记 `ROADMAP.md` |
| 四件套（功能完成的定义） | 同一提交内：`examples/<name>.rs`（顶部说明+自检断言+真跑过）→ `docs/features/<name>.md`（照 TEMPLATE 七节，含「做不到什么」）→ `FEATURES.md` 登记（状态/指南链接/示例命令三者都对）→ `TUTORIAL.md`（若属新手主线）。由 `docs_consistency.rs` 红绿强制 |
| 评审重点 | 判据是否被削弱（删测试/放宽阈值/改 `assert!(true)` = 作弊）；断言是否有**前置条件断言**与**双向变异**（单侧护栏已 4 次失效） |

### 3.3 版本号与发布流程

| 项 | 约定 |
|---|---|
| 版本方案 | 语义化版本；`0.0.0`（实验）→ `0.1.0-rc`（E1 后）→ `0.1.0`（首次发布评估） |
| `0.1.0` 发布判据 | ① E1 完成（HAL 无分叉）；② prelude/HAL 公开面审查（发布 = API 冻结承诺）；③ `cargo publish --dry-run` 通过；④ 全平台构建矩阵过（Linux 窗口若未落地须在 README 平台表如实标注） |
| 发布物 | crates.io（5 个 crate 按依赖序）；git tag `v0.1.0`；`ROADMAP.md` 登记 |
| 不许 | 手改 `Cargo.lock`（改 `Cargo.toml` 让 cargo 自己更新）；悄悄加依赖（图形库/GUI 框架明令禁止清单见 `ROADMAP.md`） |

### 3.4 测试与验收标准

**四口径门禁**（每个 PR 至少跑最小相关口径；发布前全跑）：

| 口径 | 命令 | 覆盖 |
|---|---|---|
| ① 默认 | `cargo test --workspace` | 全部无窗口断言 |
| ② 窗口门禁 | `DEER_VK_WINDOW_TESTS=1`（+ `DEER_VK_VALIDATION=1`） | 真窗口 e2e（**不设 = 显式跳过，跳过不是证据**） |
| ③ 文档一致性 | `cargo test -p deer-gui --test docs_consistency` | 四件套纪律 |
| ④ 静态 | `cargo clippy --workspace`（无告警） | 代码质量 |

**判据红线**（继承仓库既定口径，不可协商）：

| 类别 | 标准 |
|---|---|
| 像素 parity | 不透明**逐字节差 0**；半透明 **≤1 LSB**（实测上限口径，不写成「保证 ≤1」）；四状态越界期望矩形 = 0 |
| 门槛自证 | 门控用例**打印解析到的门槛状态**，验收 grep 该标记（`set "VAR=1" &&` 带引号形式） |
| 护栏自检 | 新断言须有前置条件断言 + 双向变异验证（改坏实现 ⇒ 确定性变红）；字节/编码级判据先 dump 真实产物再写 matcher |
| 数字口径 | 文档不写测试条数（写「全部通过 / 0 failed」）；要数字就读运行输出 |
| 环境陷阱 | `example` 与 `cargo test` 不并发；就地还原后 `cargo clean -p <crate>`；feature 组合是缓存键（带 `--features` 的验收必须用同组合构建） |
| 报告三段式 | 改了什么（文件级）/ 验过什么（命令+环境+结果）/ **没验什么**（及原因） |

---

## 4. 技术设计

### 4.1 关键技术方向与受影响模块

| 方向 | 内容 | 受影响模块/接口 | 设计要点 |
|---|---|---|---|
| **HAL `record` 接线**（T1.1） | `VulkanFrame::record(DrawList)` 消费 UI 命令，替代直连分叉 | `deer-vk/src/hal.rs:168-184`、`windowed.rs`、`deer-gpu/src/lib.rs`（`Frame` trait 可能扩签名） | **`TextEngine` 传递契约是核心决策**：`record` 加参数 vs `Device` 挂引擎 vs 预编译顶点流 —— 须先在 `ROADMAP.md` 登记设计再动手（改公开 API 先问的纪律） |
| **纹理 RGB 调制**（T1.3） | 统一 FS 增加「读 rgba 并与颜色调制」分支，保留覆盖率语义为特例 | `deer-vk/src/spirv.rs`（新 FS，**过 `spirv-val`**）、`device.rs`（描述符）、`windowed.rs`（`draw_textured_quad` 窗口入口） | 判别符沿用 `uv.x < 0`（形状）/`≥0`（文本）之外需第三态或独立管线；与 CPU 参考逐字节对照 |
| **方向键焦点序**（T3.1） | 定义「容器内方向移动」的焦点规则 | `deer-gui/src/interaction.rs`（`handle`）、`deer-layout`（可能需几何邻近查询） | 与 `Tab` 焦点序同源定义；纯逻辑可单测；**先修 `interaction.rs:755` 的过期注释**（「无滚动所以方向键没意义」已不成立） |
| **IME 预编辑**（T3.4，**已落地** ✅） | `InputEvent::ImePreedit` + 预编辑缓冲状态 + Field 光标处显示 | `deer-window/src/lib.rs`（T3.4 前 `Preedit` 只用于抑制重复文本；现在**仍然用它抑制按键文本**避免双写，**并且**真的派发 `ImePreedit`）、`interaction.rs` | 依赖 T3.5 光标建模；`Preedit`/`Commit` 状态机须单测。**真机输入法**仍只能人肉验证 |
| **多窗口**（T4.4） | 渲染器多交换链 + `WindowId` 事件路由 | `deer-vk/src/hal.rs`（现「只支持一个交换链」）、`deer-window`（`WindowId` 现弃用） | `Rc<RefCell<Option<WindowedRenderer>>` 改为按窗口索引的表 |
| **线程模型**（T4.2 / Q-4） | 决定 `Device` 是否 `Send`、渲染是否独占线程 | `deer-gpu` HAL 契约、`windowed.rs` | 现状 `Rc<RefCell>` 隐含单线程；与 `Waker`（已 `Send`）的分工；决策需驱动行为实测支撑 |
| **`unify` 零分配**（T4.1） | 跨帧复用目标缓冲消掉每帧 `Vec` | `deer-vk/src/vertex_unify.rs` | **先测量后动手**：`RenderStats` 的 alloc 口径已存在 |

### 4.2 数据结构调整

| 调整 | 位置 | 兼容性策略 |
|---|---|---|
| `InputEvent::KeyDown` 加 `repeat: bool`（T3.6） | `deer-window/src/lib.rs` + `deer-gui/src/interaction.rs` | **公开枚举变更**：先冻结审查（`match` 破坏面）；`mirror` 模块与 `deer-window` 定义**逐字同步**（两份定义漂移 = 语义分叉，仓库既有纪律） |
| `UiState.texts` 增光标（T3.5） | `interaction.rs`（`BTreeMap<String,String>` → 带光标的结构） | 默认光标=末尾 ⇒ 既有「追加+删末尾」行为逐字节不变；多字节字符边界单测（中文/emoji） |
| `InputEvent` 增 `ImePreedit`（T3.4） | 同上两处 + mirror | 纯新增变体，`_` 分支不受影响；预编辑缓冲不进 `texts`（Commit 才入） |
| HAL `Frame`/`Device` 签名（T1.1/T1.4） | `deer-gpu/src/lib.rs` | 0.1.0 前的最后窗口；**变更须走「设计登记 → 评审 → 实现」三步** |
| `UnifiedVertex` 布局 | **冻结不改**（stride 52 / 偏移 0/8/24/28/44，`offset_of!` 钉住） | 新增顶点属性 ⇒ 新顶点类型 + 新管线，不动旧契约 |

### 4.3 性能与兼容性考虑

| 项 | 考虑 |
|---|---|
| 性能基线 | 每帧 1 bind/1 draw/1 submit 已达；剩余热点是 `unify` 每帧分配与 host→vertex 上传 —— 全部**先以 `RenderStats` 量化再优化**（纪律：不许只说「可忽略」） |
| 像素判据即回归网 | 任何渲染/布局/文本改动，`gpu_vs_cpu` + `window_parity` 不透明逐字节对照是**不可协商的验收**；判据变化（如亚像素接入改变默认落位）必须显式升版说明 |
| MSRV | Rust 1.85 / edition 2024 不动；升级 MSRV = 破坏性变更，须独立 PR + 全矩阵验证 |
| winit | 锁 `0.30`（解析 `0.30.13`）；升 `0.31+` 是破坏性变更，按 §5.2 依赖风险流程处理 |
| 驱动差异 | 「本机实测 ≠ 跨设备结论」：viewport 动态、1 LSB、零校验消息等结论换机器须重跑探针（`viewport_dynamic_probe`）后才可泛化 |
| 平台 | Linux/macOS 窗口路径返回明确的 `UNSUPPORTED_PLATFORM_MSG`（不静默）；E3 前发布物须在平台支持表如实标注 |

---

## 5. 风险与应对

### 5.1 技术债务风险

| 债（DEV-PLAN 编号） | 风险 | 概率×影响 | 应对 |
|---|---|---|---|
| D1 HAL 分叉 | M7 时每个后端重造直连路径；HAL 层测试语义不完整 | 高×高 | **Phase 1 头号任务**；E1 收口前不启动 M7 |
| D4 纹理 RGB 缺失 | M6 `Icon` 无法实现 | 高×中 | T1.3 排在控件族前 |
| D5 输入缺口 | `Switch`/`ScrubNum`/`DropMenu` 缺地基，返工 | 高×中 | Phase 3 先于 Phase 5 |
| D7 单窗口 | dock/多窗口（M5 剩余）被阻 | 中×中 | T4.4 排 E2；在此之前 FEATURES.md 如实登记 |
| D8 Q-4 悬置 | 后台线程驱动 UI 只能靠 Waker；架构级返工风险 | 中×高 | T4.2 **先决策后实现**；0.1.0 发布前必须关闭 |
| D6 `unify` 分配 | 大型界面帧率风险（未测量） | 未知×低 | 先测量；`RenderStats` 已有口径 |
| D11 示例工程 path | 外部使用者无法直接构建 | 高×低 | T4.3 与维护者确认方案后修 |
| 已知坏 SPIR-V（Q-5） | 误用 ⇒ 校验层下崩溃 | 低×高 | 源码已标「不要用」；最终删除该着色器（M3 收尾） |

### 5.2 依赖升级风险

| 依赖 | 风险 | 应对 |
|---|---|---|
| `winit 0.30` → 0.31+ | breaking（事件模型历史上每版都破）；窗口/输入两层的翻译层要跟着改 | ① 升级 = 独立 PR + 全门禁；② 翻译层（`map_key` 等）是纯函数、单测齐全 ⇒ 改动面收敛在映射表；③ 不追新，只在需要新能力时升 |
| Rust 工具链 | edition 2024 语义变化、clippy 新 lint | MSRV 锁 1.85；工具链升级独立验证，不与功能 PR 混合 |
| Windows SDK / 驱动 | `vulkan-1.dll` loader 行为差异、结构体布局变化 | `offset_of!` 断言 + `DEER_VK_VALIDATION` 门禁是检测网；换机结论须重跑探针 |
| 新依赖引入 | **纪律红线**：默认禁止；图形库/GUI 框架明令不引 | 任何新依赖：先登记 `ROADMAP.md`（理由+影响面+撤回方案）+ 人同意，再进 `Cargo.toml` |

### 5.3 回滚方案

| 场景 | 方案 |
|---|---|
| 功能回归 | 分支未合：直接废弃分支；已合：`git revert`（不用 reset 掩盖历史）；像素判据（`gpu_vs_cpu`/`window_parity`）+ 四口径门禁是回归检测网 |
| 发布事故（0.1.0 后） | yank + `0.1.1` 修复发布；tag 不可动 |
| HAL 大改失败（T1.1） | 直连路径（`draw_and_present`）在过渡期**保留不删**，HAL 接线合入且判据全绿后才移除 —— 双路径并存是显式回滚保险 |
| 判据误删/削弱 | 评审红线（§3.2）+ 变异验证；发现后按「根因」修复并补回归 |
| 环境级误判 | 「`git status` 干净 ≠ `target/` 干净」：任何还原后 `cargo clean -p <crate>` 再下结论；feature 组合用错构建 ⇒ 用验收同组合重建 |

### 5.4 应急预案

| 事件 | 预案 |
|---|---|
| 校验层下崩溃 | 先辨是否触达已知坏着色器（Q-5）；是 ⇒ 走既定跳过标记；否 ⇒ 二分定位 + `viewport_dynamic_probe` 类探针复现 |
| 平台特异行为（新机器/新驱动） | 不泛化本机结论；跑四口径 + 探针；结论登记 `AGENTS.md` §3.1 只增不删 |
| 测试基建限制 | 「每进程一个窗口」（M3+(b) 未修）窗口期：多窗口相关 e2e 用多进程脚本拼接，单窗口断言不放宽 |
| 文档漂移 | `docs_consistency` 是第一道网；本文档族真相源表（§0）是第二道 —— 每 PR 只改一个真相源，其余引用 |

---

## 6. 协作分工

### 6.1 所需角色

> 当前仓库实际为「维护者 + AI 协作者」模式（见 `docs/superpowers/plans/STATE.md` 的任务板机制）；下表同时适配人类协作者加入的情形。

| 角色 | 职责 | 对应 Phase |
|---|---|---|
| **维护者（lead）** | 里程碑裁决、依赖例外审批、公开 API 变更审批、发布、`ROADMAP.md`/本文档维护 | 全程 |
| **渲染开发（vk-dev）** | `deer-vk`：HAL 接线、纹理、SPIR-V、多窗口渲染侧 | Phase 1/4/6 |
| **窗口/输入开发（window-dev）** | `deer-window` + `interaction.rs`：跨平台句柄、IME、事件路由 | Phase 2/3 |
| **布局/文本开发（layout-dev）** | `deer-layout` + `deer-gpu` 文本栈：焦点序几何、亚像素接入 | Phase 3/5 |
| **文档/测试工程（docs-dev）** | 四件套交付、指南、testkit 用例、验收报告 | 全程（每阶段收口密集） |
| **常驻复审（verifier）** | 独立复审：判据可红性（变异验证）、门槛自证、报告三段式完整性 | 每任务合并前 |

### 6.2 任务拆分方式

1. **单元 = DEV-PLAN 任务**（T 编号）：每个任务自带目标/验收标准/依赖/工作量，直接作为 issue/任务板的卡片粒度；
2. **并行分组沿用 DEV-PLAN §9.2**：A 组（窗口/跨平台）、B 组（输入/纯逻辑）、C 组（性能/工程化）互不触碰同一文件，可同时推进；主干（Phase 1）串行；
3. **一人多角色**时按「摘果子 → 阻塞链 → 产品缺口」的顺序从 DEV-PLAN 拉任务（即 §1.3 排序原则）；
4. **拆分红线**：改公开 API（HAL trait、`InputEvent`）的任务不拆给并行执行者 —— 设计登记后才动手。

### 6.3 进度跟踪机制

| 机制 | 用法 | 频率 |
|---|---|---|
| `FEATURES.md` | 功能级进度的唯一真相（✅/🔄/⬜ 即燃尽图） | 每四件套提交 |
| 任务板 | T 编号任务的状态（待办/进行/待复审/已合并）+ owner | 每任务变更 |
| 里程碑检查点 | E1/E2/E3/M6/0.1.0/M7：收口时跑**全四口径门禁** + 更新 `ROADMAP.md` 状态列 | 每阶段 |
| 判据不退化基线 | 每阶段收口记录 `cargo test --workspace` 运行输出（**不写进活文档**，只在带日期的快照/PR 描述里留档） | 每阶段 |
| 阶段复盘 | 更新本文档 §2 的实际进度 vs 排期，平移后续日历节点 | 每阶段 |
| 每任务收口报告 | 三段式：改了什么 / 验过什么 / 没验什么（缺一段 = 打回） | 每任务 |

---

## 7. 待补充字段（需维护者/团队确认后填入）

| 字段 | 说明 | 影响 |
|---|---|---|
| 人力配置 | §2 日历折算按 1 名全职；实际人数与投入比例 | 决定并行组数量与日历节点 |
| `0.1.0` 发布决策 | 是否发 crates.io（= API 冻结承诺的起点） | 决定 E1 后的 prelude 审查排期 |
| 右键语义（T3.3） | 上下文菜单 vs 纯事件透传 | 影响 `DropMenu`（5f）的设计 |
| `TextEngine` HAL 契约形态（T1.1） | `record` 加参数 / `Device` 挂引擎 / 预编译流 三选一 | Phase 1 的核心设计评审项 |
| macOS 路线（T2.3/T6.2） | Metal 自研 vs MoltenVK 复用 Vulkan 后端 | 决定 M7 是否含 Metal 或降级 |
| `test_project` path 方案（T4.3） | 相对路径 / 文档说明 / `[patch]` | 外部使用者体验 |
| CI 基建 | 当前仓库无 CI 配置；是否补 GitHub Actions（四口径门禁自动化） | 决定验收人工成本与「假绿」风险 |
