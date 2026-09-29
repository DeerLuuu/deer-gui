# AGENT.md —— 给 AI 协作者的须知

> 本文件写给**在本仓库里干活的 AI agent**（也包括第一次接手的人类协作者）。
> 它只讲两件事：**这个仓库的硬规矩**，以及**这个仓库踩过的坑**。
> 功能"有哪些、做到哪一步"看 [`FEATURES.md`](FEATURES.md)（唯一真相），
> 计划与验收判据看 [`ROADMAP.md`](ROADMAP.md)，用法看 [`README.md`](README.md)（英文）/
> [`README.zh-CN.md`](README.zh-CN.md)（中文）与 [`docs/`](docs/)，提交与评审规矩看 [`CONTRIBUTING.md`](CONTRIBUTING.md)。
> **本文件不重复它们的内容** —— 数字与状态一旦出现第二份，就一定会漂。
>
> 约定：本文件固定放**仓库根目录**，与 `README.md` / `README.zh-CN.md` / `FEATURES.md` / `ROADMAP.md` 同级。
> 面向人类的规则、门禁与陷阱清单在 [`CONTRIBUTING.md`](CONTRIBUTING.md) —— 那是与人共用的权威位置，
> 本文件是同一套纪律的 agent 视角摘要，**两者冲突时以 `CONTRIBUTING.md` 为准**。
> 若你的工具按大写文件名自动发现（`AGENTS.md` / `CLAUDE.md`），把它指到本文件即可，
> **不要另建一份** —— 两份一定会漂。

---

## 0. 三十秒速览

| 问题 | 答案 |
|---|---|
| 这是什么 | 从零实现的 Rust GUI 运行时：自研节点树、布局代数、Vulkan 绑定、字形光栅化；winit 是唯一例外依赖（Q-1） |
| 工具链 | Rust **1.85+**、**edition 2024**、Cargo workspace（`Cargo.lock` 入库，请勿手改） |
| 能不能引依赖 | **默认不能**。新增第三方依赖必须先登记进 `ROADMAP.md` 的「依赖例外登记」并说明理由 |
| 全量自检 | `cargo test --workspace` |
| 真正的门禁 | 见 §4 —— 真窗口 e2e **默认跳过**，不开环境变量时"全绿"不是证据 |
| 改文档的代价 | 有测试（`crates/deer-gui/tests/docs_consistency.rs`）把文档纪律变成**红/绿**，见 §2 |
| 本机能不能跑 | 见 §5 —— 当前镜像里**没有 cargo**，读代码可以、构建不行 |

---

## 1. 硬规矩（违反即返工）

1. **文档是交付物的一部分，不是附属品。** 新增功能必须在**同一个提交**里做完四件事（见 §2），
   缺任何一件都不算完成。面向人的完整版在 [`CONTRIBUTING.md`](CONTRIBUTING.md)，
   `FEATURES.md` 第五节的原文是「**缺一不算完成**」；本节与它是同一套规矩的两处表述，
   **冲突时以 `CONTRIBUTING.md` 为准**。
2. **不许悄悄加依赖。** 目前登记在案的第三方依赖只有 `deer-window` 的 `winit`
   （见 `ROADMAP.md`「依赖例外登记（Q-1）」）。`deer-layout` / `deer-gpu` / `deer-vk`
   以及**没开 `window` feature** 的 `deer-gui` 都必须保持只有内部依赖。
   `ROADMAP.md` 明令不引的：图形抽象库（`wgpu` / `ash` / `vulkano` / `glow`）
   与 GUI 框架（`egui` / `iced` / `tauri`）。**想引任何新依赖 = 先登记 + 说明理由 + 得到人同意。**
   - **措辞纪律**：写"零依赖"要写成"**除窗口层（`winit`，已登记）外零第三方依赖**"，
     不要写"完全零依赖"。
3. **不许手改 `Cargo.lock`。** 要变依赖 → 先改 `Cargo.toml`，再用 cargo 命令让它自己更新。
4. **行尾一律 LF。** `.gitattributes` 强制 `eol=lf`（含 `*.dui`）；理由写在文件里：
   CRLF 混入会破坏字节级可复现性。
5. **数字不许凭记忆写。** 断言数、测试数、通过数这类数字**每轮都在漂**，
   仓库里的原话是「**以运行输出为准**」。你自己没跑过，就不要在文档里写具体数字。
   完整口径见 [`CONTRIBUTING.md` 的 “How to write test counts”](CONTRIBUTING.md#how-to-write-test-counts-repository-wide-convention)：
   **活文档只写「全部通过 / 0 failed」，不写绝对条数**；只有带日期的历史快照（`docs/M1-report.md`、
   `docs/superpowers/plans/*.md`）才允许保留当时的实测值。
6. **不许为了变绿而削弱判据。** 删测试、放宽阈值、把真断言改成 `assert!(true)`、
   把失败用例改成"跳过"——都算作弊；不能真跑就如实说"本环境未验证"。
7. **不要覆盖别人的在途工作。** 动手前先 `git status` / `git log`；
   工作区脏时先弄清那些改动是谁的、是否该保留。
8. **改动只做必要的那一小步。** 不做顺手重构、不重排无关文件、不批量改格式。
9. **提交与推送需要授权。** 除非用户明确要求，否则**不要 `git commit` / `git push`**，
   留下工作区改动供审阅即可。
10. **不要动用户的其它文件。** 本仓库之外的路径默认只读。

---

## 2. 「每完成一个功能就教怎么用」—— 这条纪律**有测试在强制**

`crates/deer-gui/tests/docs_consistency.rs` 会把下面的失误变成**测试失败**。
改 `FEATURES.md`、加示例、加指南之前，先读它（209 行，很短）。

新增功能时，**按顺序**做四件事：

1. **加示例** `crates/deer-gui/examples/<名字>.rs`：
   - 顶部注释写清：这是什么 + 怎么跑 + 产物在哪；
   - 结尾要有**自检断言**（例如"画面不能只有一种颜色"），
     防止"跑成功但什么都没做"；
   - **真的跑一遍**，确认 `exit=0`。
2. **加指南** `docs/features/<名字>.md`，照 [`docs/features/TEMPLATE.md`](docs/features/TEMPLATE.md) 的 7 节写。
3. **登记 `FEATURES.md`**：在对应表里加一行，**状态 / 指南链接 / 示例命令三者都要对**。
4. **更新 `docs/TUTORIAL.md`**：若该功能属于新手主线，加进对应章节。

测试实际钉住的规则（别猜，按这些做）：

| 规则 | 谁在管 | 触发的做法 |
|---|---|---|
| `FEATURES.md` 里每个 `.md` 链接目标必须存在 | `features_manifest_guides_exist` | 链接写错路径 / 指南还没建 |
| 每条 `--example <名>` 必须有 `examples/<名>.rs` | `features_manifest_examples_exist` | 清单里写了不存在的示例名 |
| **每个 ✅ 行必须同时有指南链接 + 示例命令** | `every_completed_feature_has_guide_and_example` | 「只登记不交付」 |
| 每份指南**必须含「做不到什么」**、**不能留 `- [ ]` 未勾选项** | `guide_files_mention_existing_examples_and_are_complete` | 抄模板忘了填 / 清单没走完 |
| `README.md` 必须链接 `docs/TUTORIAL.md`、`FEATURES.md`、`docs/features/` | `tutorial_and_manifest_are_linked_from_readme` | 改 README 时删掉了入口 |

> 几个"别踩"的推论：`FEATURES.md` 里的 ✅ 行**加链接=承诺文件存在**；
> 模板的第 7 节检查清单必须全勾（`- [ ]` 一个都不许留）；
> 新建指南时别忘第 6 节 —— 没有「做不到什么」这一节就是失败。

---

## 3. 这个仓库踩过的坑：**这些结论不是建议，是不回退前提**

改渲染/布局/字体相关代码前先读这一节。详细根因在各指南里，这里只给结论。

### 3.1 Vulkan 侧

| 结论 | 理由 / 后果 |
|---|---|
| **静态 viewport/scissor**（离屏路径），录制时不要调 `vkCmdSetViewport`/`vkCmdSetScissor` | 这是**实现事实**（已够用），**不是**「动态不可用」：旧说法「动态版在本机 Intel 核显上画不出任何像素」**已被本机三组对照推翻**（动态 + 每帧真的调 ⇒ 与静态逐字节相同、校验消息 0、能画出像素；动态 + 从不设置 ⇒ **崩**：离屏 `0xC0000005`、窗口 `0xC000041D`）。**边界**：本机实测，别读成「动态现已支持」；M2a 记的是「零像素不崩溃」≠ 本次「崩溃」⇒ 不能断定当年同源。**产品行为不变**：离屏仍用静态（C1 只推翻了旧理由）。重跑：`cargo run -p deer-vk --example viewport_dynamic_probe`；详见 `docs/features/window.md` 第 5.2 节 |
| **颜色附件必须 `R8G8B8A8_UNORM`，不是 `_SRGB`** | CPU 基准不做 gamma，用 SRGB 会**系统性偏差**，parity 必挂；**上屏交换链同理**（M3c 改成**线性 `*_UNORM` 优先**：sRGB 附件的混合在线性空间，与 CPU 字节空间实测差 **44 字节**） |
| 推送常量矩形着色器 `spirv.rs::vertex_shader_rect_pushconstant` **是坏的，别用它建管线** | 校验层报 `VUID-StandaloneSpirv-PushConstant-06808`；**普通驱动会宽容接受**，但开校验层会 **`0xc0000005` 崩溃**。矩形走顶点缓冲（M3a 已这么做） |
| **自研 SPIR-V 汇编器必须过 `spirv-val`** | 曾经的根因是**段序错误**（`OpEntryPoint` 排在类型之后）——驱动**既不报错也不画**。`vkCreateShaderModule` 很宽容，**"驱动接受" ≠ "SPIR-V 正确"** |
| **`DEER_VK_VALIDATION=1` 下"零校验消息"有边界** | VVL **不做通用同步验证** ⇒ 零消息**不能**证明内存域依赖正确（host→vertex 屏障另有专门断言守） |
| **半透明 1 LSB 是实测上限，不是证明上界** | 写文档时别升级成"保证 ≤1" |
| 设备/离屏与窗口三条路径**现在都读 `DEER_VK_VALIDATION`** | 别再按"离屏路径故意不读它"的旧印象解释测试行为 |

### 3.2 布局与文本侧

| 结论 | 理由 |
|---|---|
| 布局是**纯函数 + 确定性**：不改输入树、同输入 ⇒ 逐位相同输出（无时间/随机/环境探测） | 布局不变式 I-1/I-2，有测试 |
| **父分配尺寸 ≠ 可用空间上限**；容器固有尺寸按主轴/交叉轴语义不同 | 踩过的真缺陷 B-2、B-3，守卫测试在 `crates/deer-layout/tests/layout_invariants.rs` |
| 两条构筑路径（命令式 / `.dui`）产出**结构相等**的树 | 核心不变式，`t1_two_authoring_paths_produce_the_same_tree` |
| `with_props` / `with_layout` 是**整体赋值** | 先定 id 再设它们会把 id 一起改掉（缺陷 R-1） |
| 主轴对齐的剩余空间必须在 `grow` **之后**算 | 否则 `grow` 一生效，`center`/`end` **静默失效**（缺陷 R-2） |
| 布局、绘制列表、光栅化**必须用同一个字号、同一个度量** | Q-3 定下的新纪律；`ApproxMeasure` 只是确定性测试用的近似 |

### 3.3 已经明确"做不到"的（**不要以为能跑**）

- **GPU 侧文本**（`DrawCmd::Text` → Vulkan）：**M3b 已完成** —— 字形四边形 + `R8_UNORM` 覆盖率纹理 + 最近邻采样，
  与 CPU 后端逐像素对照（不透明逐字节 0 / 半透明 ≤1 LSB）。注意仍有一条**刻意保留**的行为：
  **不调 `GpuGeometryRenderer::with_text(engine)` 时，`DrawCmd::Text` 仍按 M3a 行为报 `Unsupported`**（不静默丢弃）。
- **纹理 / 字形图集上 GPU**：**已落地** —— 字形图集走专用 `R8_UNORM` 覆盖率纹理；通用纹理（`RGBA8_UNORM`）
  也已能创建/上传/回读（离屏纹理 quad 与 CPU 逐字节相同）。**仍未做**：按 RGB 调制（需新片元着色器）、窗口侧任意纹理入口。
- **窗口里显示界面**（M3c）：**已完成** —— 形状与文本经共用管线层呈到线性交换链，上屏像素与 CPU 逐像素对照（`window_parity`）。
- **CFF / OpenType-CFF（`OTTO`）字体**：解析层直接报错 —— **故意不静默给空轮廓**。
  只支持 `glyf` 轮廓。
- **hinting / 字距连字（`kern`/`GSUB`/`GPOS`）/ 竖排 RTL**：都不做（hinting 现在有**实测依据**：最省的 hinting-lite 量下来没有净收益）。
- **亚像素水平定位**：**光栅化层已落地 opt-in 路径**（1/4 相位档；实测落位误差 RMSE 0.2890→0.0733 px），
  但**默认路径仍整数落位**、**尚未接进文本引擎** ⇒ 产品像素没变；代价是「间距精度换边缘锐度」（不是「清晰度提升」）。
- **输入与焦点**：**已落地**（`InputEvent` + winit 映射 + `App::input`、命中/状态机（含裁剪与禁用）、点击 / `Tab`·`Shift+Tab`·`Escape` 焦点、文本输入、脚本重放、**事件驱动重绘（默认省电）**）；**仍缺**：dock / 多窗口、方向键导航、滚动条 / 惯性滚动、右键中键、IME 预编辑（见 `docs/features/input.md` 第 6 节）；**滚轮驱动的垂直滚动已落地**（`docs/features/scroll-and-multiline.md`）。
- **12 个控件族、DX12/Metal**：M6/M7，完全没有。

---

## 4. 验证：怎么做才算"验过"

### 4.1 命令

严格照 `ROADMAP.md` 与各指南的记录执行，别自己发明变体：

```sh
cargo test --workspace                      # 全部断言（数量见输出）
cargo test -p deer-vk -- --nocapture        # 看本机枚举到的 GPU
cargo run -p deer-gui --example render_to_png    # 离屏出图（CI 友好）
cargo run -p deer-gui --features window --example window_preview   # 真窗口（仅 Windows）
```

示例工程（刻意排除在 workspace 之外，`test_project/deer-hello`）：

```sh
cd test_project/deer-hello && cargo build
```

### 4.2 门禁环境变量（**漏了就得到假绿**）

| 变量 | 作用 | 不设的后果 |
|---|---|---|
| `DEER_VK_WINDOW_TESTS=1` | 启用真窗口 e2e | **显式跳过，跳过也算 pass** —— 所以"全绿"不是证据 |
| `DEER_VK_VALIDATION=1` | 请求校验层 | 零校验消息的断言失去判别力 |
| `DEER_HAL_FRAMES` | `hal_window_path` 帧数（默认 30） | 用默认值即可 |
| `DEER_WINDOW_FRAMES` / `DEER_WINDOW_HOLD=1` | 示例帧数 / 留窗观察 | 只影响人工观察 |
| `DEER_FONT_DEBUG` | 字体解析调试输出 | 只影响调试 |

不负责的"绿"有四种，写结论时别踩：

1. **跳过**（门禁变量没设）—— 仓库原话：**跳过也算 pass，别当证据**；
2. **零校验消息** —— **不等于**同步正确（VVL 不做通用同步验证）；
3. **1 LSB 差** —— 是**实测上限**，不是证明上界；
4. **只在无窗口路径过** —— 真窗口/上屏路径可能完全没被覆盖。

---

## 5. 本机（当前环境）能做什么、不能做什么

> 这一节描述**当前运行环境**，换机器请重新核对。

**能做**：读全部源码与文档；`git` 全操作；静态审阅、补文档、写补丁/diff；跨 crate 的检索与对账。

**不能做（不要假装做了）**：

- **没有 cargo / rustc** —— 无法构建、无法跑测试、无法跑示例。
  所以**任何"测试全绿 / 已验证"的结论都不能由本机给出**。
- **窗口层只在 Windows 可用**：真窗口 + Vulkan 上屏 + `window_preview`
  在非 Windows 上会返回 `UNSUPPORTED_PLATFORM_MSG`（`crates/deer-window/src/lib.rs`）。
- **真机 GPU 相关验证**依赖具体机器（仓库记录的是 Windows + Intel/ NVIDIA 那张卡）。
- `test_project/deer-hello/Cargo.toml` 里的 `path` **硬编码为 `Z:/deer-gui/...`**
  （作者的 Windows 盘符）—— 在本环境里**不能直接 build**，要改路径才行；
  **不要**为了"跑一下"就把这个路径随手改成自己机器的路径再提交。

因此本环境下的交付说法应当是：
**"已静态审阅 + 已给 diff，未编译验证（本环境无 cargo 且窗口层仅 Windows）"**，
而不是"已完成、测试通过"。

### 5.1 这份克隆的来路（本机特有）

- 路径 `/sdcard/Download/deer_gui`，远端 `https://github.com/DeerLuuu/deer-gui.git`，
  分支 `master`。这是**设备上的工作副本**。
  > **别在这里写死 HEAD**：写的时候是 `9df8ade`，几个里程碑后就成了假信息（本文件已被这条坑过一次）。
  > 要看当前值请跑 `git log --oneline -1`。
- **`origin` 读走 HTTPS、写走 SSH**（`git remote -v` 可见两个 URL）：该网络下 SSH 22 端口会间歇性会话超时，
  而 HTTPS 的 HTTP/2 会被重置 —— 所以 fetch 用 HTTPS、push 用 SSH 各个绕开一半问题。
  若 push 失败先 `ssh -T git@github.com` 验证握手；公钥在 `/root/.ssh/id_ed25519`（无口令）。
  传输若卡死，可加 `git config http.version HTTP/1.1`。
- **这台设备的 `/sdcard` 是 Android FUSE 挂载，重命名目录有坑**：
  对一个**曾经被删除过**的目录名做 `mv`，可能返回成功、同进程内看着也"对"，
  但换独立进程复核时**内容是空的**。规避方式：**用全新的目录名**，
  且改名后**必须用独立进程（新命令）复核**，别信同一条命令里的结果。
  优先"复制到新名字 + 校验 + 再删旧目录"，而不是直接 `mv`。

---

## 6. 干活的方式（协作与报告）

1. **先读后写**：动手前读 `FEATURES.md`（现状）、`ROADMAP.md`（当前里程碑与验收判据）、
   相关 `docs/features/<功能>.md`（已知坑），以及要改文件附近**已有的注释**——
   这个仓库的注释密度很高，很多"为什么这么做"都写在注释里。
2. **改坏先问**：需要改公开 API、改测试判据、动别人改过的文件、或需要新增依赖时，
   **先停下来问**，不要自行决定。
3. **提交信息风格**（本仓库实际用法，Conventional Commits + 中文）：
   `feat|fix|docs|test|chore[(范围)]: 中文描述`，里程碑写进范围或正文，例如
   `fix(deer-vk): 越界 alpha 按 CPU 基线 clamp`、`docs(m3a): ...`。
   修 bug 时在正文写**根因**，不只写改了哪一行。
4. **报告要说清三件事**：改了什么（文件级）、**验过什么**（命令 + 环境 + 结果）、
   **没验什么**（以及为什么）。三者缺一项，读者就得自己猜。
5. **不吹**：不写"完全零依赖"、不写没跑过的数字、"跳过"不写成"通过"。
   仓库里已有的措辞（"以运行输出为准"、"实测上限而非证明上界"）照抄它们的口径。

---

## 7. 边界：本文件不负责什么

- **不定义功能状态** —— 那是 [`FEATURES.md`](FEATURES.md)，它才是唯一真相；
  本文件提到 ✅/🔴/⬜ 只是索引。
- **不定义路线图与验收判据** —— 那是 [`ROADMAP.md`](ROADMAP.md)。
- **不教怎么用库** —— 那是 [`README.md`](README.md)（英文）与 [`README.zh-CN.md`](README.zh-CN.md)（中文）、
  [`docs/TUTORIAL.md`](docs/TUTORIAL.md) 与 [`docs/features/`](docs/features/)。
- **不承载面向人的提交/评审规矩** —— 那是 [`CONTRIBUTING.md`](CONTRIBUTING.md)（英文）。
- **不记录里程碑进度** —— 进度属于 `ROADMAP.md`；在本文件里记进度，两边一定会漂。

> 维护本文件的准则是**只增不删地记纪律与坑**：新踩一个坑，就加一行；
> 结论被推翻，就改那一行并写明为什么被推翻 —— 别让下一个人再踩一遍。
