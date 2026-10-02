# 功能指南：剪贴板（clipboard）

> 跑 `cargo run -p deer-gui --features window --example clipboard_probe` ·
> 实现在 `crates/deer-window/src/display.rs`（L1 DisplayServer）·
> 出处：App 地基任务书 **AF-2**（Q3 裁断：**自写 Win32**，不引 `arboard`）

## 1. 这是什么 / 什么时候用它

**你的程序与系统剪贴板互通纯文本**（Win32 `CF_UNICODETEXT`：UTF-16 + 结尾 NUL）——
Ctrl+C 要放进剪贴板的那份、Ctrl+V 要从剪贴板读出的那份，就是它。

什么时候你会碰到它：

- 你的输入框要支持**粘贴**（把用户在别处复制的文字读进来）；
- 你的程序要支持**复制**（把一段文字交给系统，别的程序都能粘走）；
- 你要读用户在**别的程序**里复制的内容（记事本、浏览器…）。

**一句话记法**：句柄经 `Clipboard::new(hwnd)` 现场构造，读写在调用处原子完成 ——
不挂事件、不占剪贴板、不引第三方依赖。

## 2. 最小示例

```rust
use deer_gui::window::{App, Clipboard, WindowInfo};

struct Editor {
    /// 建窗后存下来；写入**必须**用真实窗口作属主（见第 5 节第 1 条）
    clipboard: Option<Clipboard>,
}

impl App for Editor {
    fn init(&mut self, info: &WindowInfo) -> Result<(), String> {
        self.clipboard = Some(Clipboard::new(info.raw.handle)?);
        Ok(())
    }

    // 粘贴（在你处理 Ctrl+V 的地方，现场读）：
    //   let cb = self.clipboard.as_ref().unwrap();
    //   match cb.get_text() {
    //       Ok(text) => insert_at_cursor(text),          // 别处复制的文字进来了
    //       Err(e) => println!("剪贴板里没有文本：{e}"),   // 空/图片/文件 ⇒ 明确 Err
    //   }
    //
    // 复制（在你处理 Ctrl+C 的地方，现场写）：
    //   cb.set_text(selected_text)?;                     // 所有权交系统，别的程序都能粘走
}
```

配套命令（真窗口 + 真 OS 剪贴板的往返保真判据）：

```sh
cargo run -p deer-gui --features window --example clipboard_probe
```

实测输出（`--example clipboard_probe`，具体条数以运行输出为准）：

```text
[clipboard_probe] 剪贴板句柄就绪：HWND=0x1040B48（写入用它作属主 —— NULL 属主会让写入按 Win32 语义失败）
[clipboard_probe] 原剪贴板是文本（15 字符）⇒ 探针结束时还原
[clipboard_probe] ① 往返保真 ✅：7 条用例（空串 / ASCII / 中文 / é / emoji 代理对 / ZWJ / 混合）逐字符相等
[clipboard_probe] ② 明确拒绝 ✅：含 NUL 的文本、NULL 属主的写入都当场报错（不静默）
[clipboard_probe] ③ 原剪贴板文本已还原并复核 ✅
[clipboard_probe] 自检全部通过 ✅
```

## 3. 完整 API

| 条目 | 签名 / 语义 |
|---|---|
| `Clipboard::new` | `(hwnd: usize) -> Result<Clipboard, String>`。**Windows**：构造句柄，不碰剪贴板（`hwnd` 传 `WindowInfo::raw.handle`；`0` 只够 `get_text` 用，`set_text` 明确拒绝）。**非 Windows**：`Err(CLIPBOARD_UNSUPPORTED_MSG)` —— 明确 Unsupported，不静默 |
| `Clipboard::set_text` | `(&self, text: &str) -> Result<(), String>`。写 `CF_UNICODETEXT`：内部原子完成 **Open（带约 100ms 重试）→ EmptyClipboard → SetClipboardData → Close**；成功后**所有权归系统**，我们不再释放。含 NUL 的文本、`hwnd = 0` 都在动手前明确拒绝。⚠️ `EmptyClipboard` 会**清掉剪贴板里的所有格式**（不只是文本） |
| `Clipboard::get_text` | `(&self) -> Result<String, String>`。读 `CF_UNICODETEXT`：内部原子完成 **Open（带重试）→ Get → Close**。剪贴板为空或放的是图片/文件等非文本 ⇒ `Err`（**不静默给空串**）；内容是非法 UTF-16（残缺代理对）⇒ `Err`（**不做 lossy 替换**） |
| `CLIPBOARD_UNSUPPORTED_MSG` | 非 Windows 上构造与两个方法返回的错误文案（常量，可 grep） |

共享的语义：`Clipboard` 是 `Copy` 的小句柄（只装 HWND），**可在任意线程调用**（开与关
发生在同一次方法调用里，满足 Win32 的线程约束）；不跨调用持有剪贴板 —— 跨调用持有会
卡住全系统其它程序的复制粘贴。

### 为什么是「公共构造器」，不是「`init` 交出句柄（与 `Waker` 同型）」

（任务书 D4 给了两案，实现择一，理由登记在此，免得后人再争一遍：）

1. `Waker` 之所以要经 `App::wake_handle` **交接**，是因为 `EventLoopProxy` 只有事件循环
   内部造得出来 —— 它是 host 私产的边角。剪贴板相反：它是 **OS 全局资源**，跟窗口、
   事件循环都没有生命周期耦合 ⇒ 为它扩 `App` trait（**永久面**）买不到任何东西；
2. **非 Windows 的「明确 Unsupported」必须真的可达**：交接式在非 Windows 根本走不到
   （`run()` 在 `window_info` 一步就已 `Err`，任何 App 回调都不会被调）——「Unsupported」
   会变成没人能触发的死字。公共构造器让任何平台的用户都能调 `Clipboard::new`
   并拿到明确的错误文案；
3. **调用点最短**：粘贴发生在输入处理处，而 `App::input` 的签名里就带着
   `info: &WindowInfo`（`info.raw.handle` 即 HWND）—— 现场 `Clipboard::new` 即用即走，
   App 不必为存句柄加状态。

## 4. 自检（怎么确认你真的用对了）

`--example clipboard_probe` 结尾有三条自检：**多字节往返逐字符相等**（空串 / ASCII /
中文 / é / emoji 代理对 / ZWJ / 混合）/ **两条明确拒绝**（含 NUL、NULL 属主）/
**原剪贴板文本还原并复核**。任何一条不满足 ⇒ 进程 `exit=1`。

`cargo test -p deer-window` 另有纯判据单测（任何平台都能跑，不碰系统剪贴板）：
编解码往返保真 / 含 NUL 拒绝 / 非法 UTF-16 拒绝（不做 lossy）/ Unsupported 文案
是真明话 / `hwnd = 0` 的写入在打开剪贴板之前被拒。

## 5. 常见坑

- **写入用了 `hwnd = 0`**：现象 → `set_text` 当场报「需要真实窗口句柄」。
  原因 → Win32 明文（`EmptyClipboard` 的 Remarks）：用 NULL 窗口句柄打开剪贴板时
  `EmptyClipboard` 会把属主设成 NULL，这让 `SetClipboardData` 失败 —— 且实测是
  **时灵时不灵**的静默坏法，比直接失败难查得多。怎么改 → 建窗后用
  `WindowInfo::raw.handle` 构造句柄。本层把这个坑变成了**确定性的前置拒绝**；
- **以为 `get_text()` 返回 `Err` 说明程序坏了**：不是。剪贴板为空、或放的是图片 /
  文件等非文本格式，`get_text` 都返回 `Err` —— 这是「明确不静默」的契约
  （装作读到空串才会把「没读到」藏起来）；
- **以为 `set_text` 只动文本**：它先 `EmptyClipboard` —— **所有格式都被清掉**
  （用户刚复制的图片/文件也没了）。这是 Win32「写文本」的语义边界，不是本层的 bug；
  要「多格式共存」的复制，本层不做（见「做不到什么」）；
- **剪贴板被别的进程占着**：剪贴板是全系统互斥资源。剪贴板管理器（Win+V 历史、
  同步盘）会短暂持有它 —— 本层每次打开都带约 100ms 重试，通常足够；若对方**长期**
  占用，`set_text` / `get_text` 会明确报错，重试即可；
- **想监听「剪贴板变了」**：没有这个面。本层只提供读/写两个动作，
  不做 `AddClipboardFormatListener` 之类的事件通知。

### 做不到什么

- **只有 `CF_UNICODETEXT` 纯文本**：图片 / HTML / 文件 / 自定义格式**不读不写**；
  `set_text` 还会清掉其它格式（上一节第 3 条）；
- **非 Windows 平台**：构造与读写都明确 `Err(CLIPBOARD_UNSUPPORTED_MSG)`，
  不静默、不降级；
- **含 NUL 的 Rust 字符串**：明确拒绝 —— `CF_UNICODETEXT` 以第一个 NUL 结尾，
  写进去会被所有读取方截断，那是静默丢数据；
- **剪贴板历史 / 跨设备同步（Win+V、云剪贴板）**：系统自己的事，本层不参与、不设置；
- **「剪贴板变化」事件**：没有（上一节第 5 条）；
- **延迟渲染（`WM_RENDERFORMAT`）**：不做 —— 本层是立即渲染（`SetClipboardData`
  直接给全量数据），超大文本也是一次性给；
- **跨进程互贴未自动化**：探针自动验证的是「同进程经真 OS 剪贴板往返 + 真实属主窗口」
  这条链；「从记事本复制 → 本库读出」与「本库写入 → 记事本粘出」需要人手工跑一次
  （步骤见下）。

**手工验证步骤**（需要 Windows + 桌面会话，用探针本身就能做完）：

```sh
# ① 在记事本里选中一段含中文+emoji 的文字，Ctrl+C 复制
cargo run -p deer-gui --features window --example clipboard_probe
# 期望输出里「原剪贴板是文本（N 字符）」的 N 与你复制的字符数一致；
# ② 探针结束后回到记事本 Ctrl+V —— 粘出来的应当就是你当初复制的那段
#    （探针已把它原样还原回剪贴板）。
```

## 6. 相关

- 窗口层（句柄从哪来）：[`window.md`](window.md)
- 输入与焦点（Ctrl+C/V 的按键从哪来）：[`input.md`](input.md)
- 依赖纪律（为什么不引 `arboard`）：`ROADMAP.md` 的 Q3 与「依赖例外登记（Q-1）」

## 7. 检查清单（发布前过一遍）

- [x] `--example clipboard_probe` 真的跑过，`exit = 0`
- [x] 示例结尾有自检断言（逐字符相等 + 两条拒绝 + 还原复核），不是「跑成功就算」
- [x] 单测判据在 `cargo test -p deer-window`（编解码往返 / NUL 拒绝 / 非法 UTF-16 / Unsupported 文案 / NULL 属主拒写）
- [x] `FEATURES.md` 已登记（状态 / 指南链接 / 示例命令都对）
- [x] 不属于新手主线（窗口层的系统能力，不在 `docs/TUTORIAL.md` 主线上）—— 已按此口径核对
- [x] 「做不到什么」已写明（纯文本格式 / 非 Windows / 含 NUL / 无变化通知 / 跨进程互贴未自动化）
