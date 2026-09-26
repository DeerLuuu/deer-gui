# M1 报告：核心 + HAL + Vulkan 设备枚举

> 日期：2026-09-21 · 状态：**完成** · 仓库：`Z:\deer-gui`（本地，未建远端）

## 一、M1 要证明什么

新定位（「自己开窗、自己拿输入、自己渲染的 Rust GUI 运行时」）有三个必须先立住的前提：

1. **节点树是唯一数据模型**，且两种构筑方式（命令式 / `.dui`）产出同一棵树；
2. **布局能在无 GPU 环境下被完整断言**（否则这个仓库的测试纪律用不上）；
3. **「自己写 GPU 后端」物理上可行** —— 在**没有 Vulkan SDK** 的机器上也能拿到设备。

M1 就是拿最小实现撞这三条。

## 二、结果

```
cargo test --workspace
  deer-layout  18 passed（布局不变量 + 场景解析 + 回归守卫）
  deer-vk       5 passed（结构体布局断言）
  deer-vk       2 passed（真机 Vulkan）
  合计 25 passed / 0 failed
```

**真机 Vulkan 证据**（`cargo test -p deer-vk -- --nocapture`）：

```
Vulkan 后端名: vulkan
枚举到 2 个物理设备：
  [0] Intel(R) RaptorLake-S Mobile Graphics Controller | IntegratedGpu | Vulkan 1.4.309
  [1] NVIDIA GeForce RTX 5070 Ti Laptop GPU            | DiscreteGpu   | Vulkan 1.4.341
```

设备名与版本号都是**从驱动读回来的真实值** —— 这说明：
- 符号通过 `LoadLibraryW` + `GetProcAddress` 解析正确；
- `VkInstanceCreateInfo` / `VkApplicationInfo` 的结构体布局正确（否则 `vkCreateInstance` 会失败）；
- `VkPhysicalDeviceProperties` 的 **offset 正确**（否则 `device_name` 会读到垃圾 ——
  这类错误不会崩溃，只会给出幻觉般的字符串，是最难查的一类）。

## 三、关键发现

### 3.1 `#[link(name = "vulkan-1")]` 在无 SDK 机器上**链接失败**

```
error: linking with `link.exe` failed: exit code: 1181
LINK : fatal error LNK1181: 无法打开输入文件“vulkan-1.lib”
```

系统只有 `vulkan-1.dll`（loader），而**导入库 `.lib` 属于 Vulkan SDK**（本机 `VULKAN_SDK` 为空、
无 `vk.xml`）。**修法**：改用运行时动态加载（`LoadLibraryW` + `GetProcAddress`），
只依赖 `kernel32` —— 于是**既不需要 SDK，也不需要导入库**。

这是「不装 SDK」这条约束的真实代价，也是 M1 最有价值的工程结论：
**「自己写 Vulkan」的第一步不是写命令缓冲，而是先解决「怎么拿到符号」。**

### 3.2 Rust 移植过程中抓到的 5 个缺陷

全部有「改坏 → 红 → 改回」的验证（V0 的 B-1/B-2/B-3 是继承来的守卫；R-1/R-2 是本次新抓的）：

| # | 缺陷 | 后果 | 守卫 |
|---|---|---|---|
| **B-1** | 两条构筑路径 id 规则不一致（单计数器 vs 按类型计数） | 两棵树结构不等，核心命题直接失败 | `t1_*`（2 条） |
| **B-2** | 布局把「父分配尺寸」当成「可用空间上限」 | `grow` 分配被丢弃（`a+gap+b=66` 而非 300） | `t5_grow_fills_the_row` |
| **B-3** | 容器固有尺寸漏算子节点显式尺寸 | 父容器比子节点还窄 | `t13` |
| **R-1** | 建造顺序：`with_props`/`with_layout` 是整体赋值，先定 id 再设它们会**连 id 一起改掉** | 控件 label 被静默清空（布局尺寸全错） | `t1`、`t4` |
| **R-2** | 主轴对齐的剩余空间算在 `grow` **之前** | `grow` 一旦生效，`center`/`end` **静默失效** | `t6_main_axis_alignment` |
| **I-8** | 让子节点显式尺寸污染容器**交叉轴**固有尺寸 | `Column` 宽度变成子节点**之和**而膨胀 | `t14_container_cross_axis_is_max_not_sum` |

**R-2 与 I-8 是同一类毛病**：布局仍然「有值」，只是值错了 —— 没有断言就永远发现不了。
这正好印证了新定位与本仓库测试纪律的契合：**布局代数能在无 GPU 环境下被完整钉死**。

## 四、M1 的产物

| crate | 文件 | 内容 |
|---|---|---|
| `deer-layout` | `node.rs` / `layout.rs` / `builder.rs` / `scene.rs` / `lib.rs` | 节点树、布局代数、命令式 API、`.dui` 解析 |
| `deer-gpu` | `lib.rs`（HAL trait）/ `draw.rs` / `error.rs` / `null.rs` | HAL 契约、绘制列表、CPU 参考后端（软件光栅化） |
| `deer-vk` | `ffi.rs` / `lib.rs` | Vulkan 符号声明 + 动态加载 + 实例/枚举 |

零第三方依赖。

## 五、M1 **没有**验证的事（诚实边界）

> 注（M2b 之后）：本报告是 M1 当时的快照。此后变化：窗口层已引入 winit（**唯一登记在案的第三方依赖**，见 ROADMAP Q-1），且窗口与 Vulkan 上屏（M2b）已落地 —— 故「零第三方依赖」「没有窗口」在**今天**应读作「除窗口层外零第三方依赖」「有窗口」。
> 注（M4 之后）：本节描述的是 M1 当时的交付状态；现在离屏 CPU 路径已支持真实字形，见 [text-rendering](features/text-rendering.md)。

- **没有出图**：`deer-vk` 只到实例与设备枚举；逻辑设备、交换链、管线、着色器全在 M2–M3。
- **没有窗口**：`RawWindowHandle` 只是定义，没有 Win32/X11 实现（Q-1 未决）。
- **CPU 后端能出像素，但不是「正确渲染」**：文字是**等宽字形格占位**，不是排版。
- **没有输入与焦点**：`hit_test` 只证明「能在几何上找命中」，事件派发与焦点系统在 M5。
- **`.dui` 与 Godot `.tscn` 不兼容**：只是同族手感（缩进 + 段头）。
- **没有跨平台验证**：只在 Windows / MSVC / Vulkan 上跑过。

## 六、下一步（M2）

M2 的目标是**第一次真正出图**：Vulkan 逻辑设备 + 交换链 + 清屏 + 呈现。

阻塞 M2 的两个决定（见 `ROADMAP.md` Q-1 / Q-4）：
1. **窗口**：自己写 Win32（`CreateWindowExW` + 消息循环），还是允许 `winit`？
   —— 自己写更符合「从零」的定位，但会显著增加工作量并引入平台分支。
2. **线程模型**：渲染线程与 UI 线程的边界（HAL 故意不给 `Send`/`Sync`）。

**建议**：M2 先做**无窗口的离屏渲染**（渲染到 `VkImage` + 回读像素 + 与 CPU 后端逐像素对照），
把「管线/着色器/命令缓冲/同步」验证完，再决定窗口方案 ——
这样 M2 的可验证性不依赖 Q-1 的答案。
