# 功能指南：日志（logging）

> 跑 `cargo run -p deer-gui --example logging` · crate `crates/deer-log` ·
> 开关是环境变量 `DEER_LOG` · **默认完全静默**

## 1. 这是什么 / 什么时候用它

**零依赖、默认静默**的日志门面。用来「快速调试 / 定位错误」—— 尤其是那些
**不报错但结果不对**的地方。

什么时候用它：

- 界面「看起来只是没生效」（滚动条没出现、控件不响应、文字没换行），
  需要知道**是哪一步的输入不对**；
- 想知道某条路径**到底走没走**（`DEER_LOG=deer_gpu=debug` 看它打不打）；
- 排查数量/性能问题（缓存命中、上传次数、图集增长）。

**不要**用它替代断言：日志是给人看的，判据要靠测试。

## 2. 最小示例

```bash
# 什么都不开（默认）—— 一个字节都不输出
cargo run -p deer-gui --example scroll_bar

# 只开 deer_gpu 这一路（debug 及以上）
DEER_LOG=deer_gpu=debug cargo run -p deer-gui --example scroll_bar

# 全局 info + deer_vk 更详细
DEER_LOG=info,deer_vk=trace cargo run -p deer-vk --example vulkan_devices
```

实测（同一份示例，开与不开的差别）：

```text
$ cargo run -p deer-gui --example scroll_bar 2>&1 | grep -c '\[DEBUG\]'
0

$ DEER_LOG=deer_gpu=debug cargo run -p deer-gui --example scroll_bar 2>&1 | grep '\[DEBUG\]'
[DEBUG] deer_gpu::interact: 可滚动容器 `small` 上限为 0（内容装得下视口）⇒ 不画滚动条（正常）
[DEBUG] deer_gpu::interact: 可滚动容器 `list` 没有滚动上限（`ScrollView::metrics` 为空）⇒ 不画滚动条；多半是忘了把 `layout_with_scroll` 的 `ScrollMetrics` 灌回状态（`set_metrics`）
```

**第二条就是这套日志的价值**：两种情况在画面上**完全一样**（都只是「没有滚动条」），
日志把它们分开了 —— 一个是正常，另一个是调用方漏了 `set_metrics`。
这正是本轮加日志之前真实踩过的坑。

## 3. 完整 API

```rust
pub enum Level { Trace, Debug, Info, Warn, Error }   // 越靠后越严重
pub fn enabled(level: Level, target: &str) -> bool;  // 热路径先问这个
pub fn log_at(level: Level, target: &str, args: fmt::Arguments<'_>) -> bool;

pub struct Filter;                                    // 纯逻辑，可单测
impl Filter {
    pub fn off() -> Filter;
    pub fn parse(spec: &str) -> Filter;               // 解析 DEER_LOG 的语法
    pub fn allows(&self, level: Level, target: &str) -> bool;
    pub fn is_off(&self) -> bool;
}

// 宏：target 自动取 module_path!()
deer_log::trace! / debug! / info! / warn! / error!
```

`DEER_LOG` 的语法：**逗号分隔**，每项是 `级别` 或 `target=级别`。
级别名大小写不敏感、两侧空白容忍；**后写的规则覆盖先写的**（同 target）；
**解析不了的项忽略**（配错不会让程序起不来，也不会意外把日志开出来）。
target 是**前缀匹配且落在 `::` 边界**：`deer_gui=debug` 命中 `deer_gui::interaction`，
但不会命中 `deer_guide`。

开某个级别 ⇒ 允许**所有 ≥ 它**的级别（`debug` 打开 error/info/debug，但不开 trace）。

## 4. 自检（怎么确认你真的用对了）

`--example logging` 结尾有五条自检：默认全关 / 开一路不牵连别的 / 阈值单调 /
后者覆盖前者 / 配错仍静默。`crates/deer-log` 里另有 10 条单测覆盖同一批语义。

**最要紧的一条**：不设 `DEER_LOG` 时**任何**级别、**任何** target 都不输出 ——
这条是「不许污染既有 stderr 判据」的保证（校验层回调的 `[VK ERROR]`、
示例的「跳过」提示都按 stderr 断言，日志默认开口会把它们弄坏）。

## 5. 常见坑

- **以为改了环境变量会立刻生效** —— 不会。过滤器在**首次** `enabled()` 时读一次并缓存
  （`OnceLock`）。这是为「运行期几百万次调用」付的代价，也让一次进程内的行为确定。
  ⇒ 想换配置就**重开进程**。
- **在热路径直接拼字符串** —— `enabled()` 关着时几乎零成本，但 `format!` 不是。
  正确写法是先 `if enabled(..) { log_at(..) }`；宏已经这么做了（参数是 `format_args!`，惰性）。
- **把日志当判据** —— 日志会被关掉（默认就是关的）。要判据就写测试。
- **以为它写 stdout** —— 本 crate 只写 **stderr**。把示例产物重定向到文件（`> out.txt`）时，
  日志不会混进去（日志走 stderr，需要时用 `2> log.txt` 单独收）。
- **不设 `DEER_LOG` 时什么都看不到就以为坏了** —— 默认静默是**刻意**的，见第 4 节。

## 6. 相关

- 环境变量门槛判定（另一套开关纪律）：`env_gate`，见 [`input.md`](input.md)
- 测试与判据：[`testing.md`](testing.md)
- 滚动条的诊断点（本系统第一个真实用户）：[`scrollbar.md`](scrollbar.md)

### 做不到什么

- **没有日志文件轮转/落盘**：只写 stderr，重定向交给 shell（`2> log.txt`）；
- **没有结构化字段 / JSON**：只有 `[级别] target: 消息` 这一种格式；
- **没有采样 / 限流**：逐帧日志会把 stderr 淹掉 —— 级别与条件得你自己控；
- **不能在运行期改级别**（见常见坑第一条）；
- **不接管已有的输出**：校验层回调那套 `[VK ERROR]` 前缀**没有**并进这套系统
  —— 它的格式是既有测试的判据，动它等于改判据；
- **尚未铺满所有 crate**：目前 `deer-gpu` 接了诊断点、`deer-gui` 可用；
  `deer-vk` / `deer-layout` / `deer-window` 要用时各自加一条依赖即可
  （**内部依赖，不算第三方**，不需要走依赖例外登记）。

## 7. 检查清单（发布前过一遍）

- [x] `--example logging` 真的跑过，`exit = 0`
- [x] 示例结尾有自检断言（五条），不是「跑成功就算」
- [x] 默认静默有判据（单测 + 示例自检各一条）
- [x] 至少有一个**真实诊断点**（`deer_gpu::interact` 的滚动条那一处）
- [x] 本指南含「做不到什么」一节
- [x] 登记进 `FEATURES.md` 且指南链接 + 示例命令都对
