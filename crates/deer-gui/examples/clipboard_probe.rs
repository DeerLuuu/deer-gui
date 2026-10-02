//! **clipboard_probe —— AF-2 剪贴板的最小端到端探针（真窗口 + 真 OS 剪贴板）**。
//!
//! **是什么**：把一组探针文本（空串 / 中文 / 拼音变音符 / emoji（代理对）/ ZWJ 序列）
//! 逐条写进**系统剪贴板**再读回来，断言**逐字符相等**（往返保真）；顺带钉住两条
//! 「明确拒绝、不静默」的边界（含 NUL 的文本、NULL 属主）。它就是
//! `docs/features/clipboard.md` 第 4 节的「怎么确认你真的用对了」。
//!
//! **怎么跑**（真窗口路径 ⇒ 要 `window` feature；仅 Windows）：
//!
//! ```sh
//! cargo run -p deer-gui --features window --example clipboard_probe
//! ```
//!
//! **判据**：全部用例逐字符相等 + 两条拒绝用例真的拒绝 + 原剪贴板文本尽量还原，
//! 结尾打 `自检全部通过`；任何一条不满足 ⇒ `run()` 返回 `Err` ⇒ 进程 exit=1。
//! **产物**：只写 stdout（自证行），不改剪贴板之外的任何东西 —— 原剪贴板文本会先
//! 读出、结束时还原（原内容不是文本时如实说明「无法还原」，不谎报）。
//!
//! **为什么是 example 不是 `#[test]`**：写入必须用**真实窗口**作属主（`hwnd = 0` 的
//! NULL 属主会让 `SetClipboardData` 按 Win32 语义失败，且实测时灵时不灵），而建窗
//! 要桌面会话 —— 与 `input_probe` 的先例同一纪律：CI 无桌面，`#[test]` 里真开窗会假红。

use deer_gui::window::{App, Clipboard, Flow, WindowConfig, WindowInfo, run};
use std::process::ExitCode;

/// 探针文本表：**多字节往返保真**的用例全在这里（加新用例就往表里添一行）。
const CASES: &[&str] = &[
    "",
    "plain ascii 123",
    "中文往返：你好，世界",
    "é è ñ（2 字节）",
    "emoji 😀🎉（代理对）",
    "ZWJ 序列 👨‍👩‍👧 与组合 ✔︎",
    "mixed aA1 中 😀 tail",
];

/// 探针应用：`init` 拿句柄并记住原剪贴板；第一帧做完往返就退。
struct Probe {
    /// [`App::init`] 里拿到的剪贴板句柄（写入用真实窗口作属主）。
    clipboard: Option<Clipboard>,
    /// 原剪贴板文本（**读得到才还原**；原本不是文本 ⇒ None，结束时如实说明）。
    original: Option<String>,
}

impl App for Probe {
    fn init(&mut self, info: &WindowInfo) -> Result<(), String> {
        let cb = Clipboard::new(info.raw.handle)
            .map_err(|e| format!("Windows 上构造 Clipboard 不应失败：{e}"))?;
        println!(
            "[clipboard_probe] 剪贴板句柄就绪：HWND=0x{:X}（写入用它作属主 —— NULL 属主会让写入按 Win32 语义失败）",
            info.raw.handle
        );
        match cb.get_text() {
            Ok(prev) => {
                println!(
                    "[clipboard_probe] 原剪贴板是文本（{} 字符）⇒ 探针结束时还原",
                    prev.chars().count()
                );
                self.original = Some(prev);
            }
            Err(e) => {
                println!(
                    "[clipboard_probe] 原剪贴板读不出文本（{e}）⇒ 结束后**无法**还原文本（不谎报）；\
                     探针留下的最后一条文本是它自己写的那条"
                );
            }
        }
        self.clipboard = Some(cb);
        Ok(())
    }

    fn redraw(&mut self) -> Result<Flow, String> {
        let cb = self
            .clipboard
            .as_ref()
            .ok_or("init 没拿到剪贴板句柄（时序错：redraw 先于 init？）")?;

        // ① 往返保真：逐条 写 → 读 → 逐字符比对。任何一条不相等 ⇒ Err ⇒ exit=1。
        for text in CASES {
            cb.set_text(text).map_err(|e| format!("set_text({text:?}) 失败：{e}"))?;
            let got = cb
                .get_text()
                .map_err(|e| format!("get_text()（刚写入 {text:?} 之后）失败：{e}"))?;
            if got != *text {
                return Err(format!(
                    "往返不保真：写入 {text:?}（{} 字符）读回 {got:?}（{} 字符）",
                    text.chars().count(),
                    got.chars().count()
                ));
            }
        }
        println!(
            "[clipboard_probe] ① 往返保真 ✅：{} 条用例（空串 / ASCII / 中文 / é / emoji 代理对 / ZWJ / 混合）逐字符相等",
            CASES.len()
        );

        // ② 明确拒绝，不静默：含 NUL 的文本（CF_UNICODETEXT 以 NUL 结尾，写了必被
        //    所有读取方截断）与 NULL 属主的写入（hwnd = 0），都必须当场报错。
        for (label, attempt) in [
            ("含 NUL 的文本", cb.set_text("a\0b")),
            // 借一个没碰过剪贴板的新句柄（hwnd=0）来钉「NULL 属主拒写」这条契约。
            ("NULL 属主", Clipboard::new(0).and_then(|c| c.set_text("x"))),
        ] {
            if attempt.is_ok() {
                return Err(format!("「{label}」应当被 set_text 明确拒绝，实际却成功了"));
            }
        }
        println!("[clipboard_probe] ② 明确拒绝 ✅：含 NUL 的文本、NULL 属主的写入都当场报错（不静默）");

        // ③ 还原原剪贴板文本（礼貌：探针不该留下自己的痕迹）。还原失败不装作成功。
        if let Some(prev) = self.original.take() {
            cb.set_text(&prev).map_err(|e| format!("还原原剪贴板文本失败：{e}"))?;
            let back = cb.get_text().map_err(|e| format!("还原后复核失败：{e}"))?;
            if back != prev {
                return Err(format!(
                    "还原后不一致：原 {prev:?} 读回 {back:?} —— 探针留下了错误的内容"
                ));
            }
            println!("[clipboard_probe] ③ 原剪贴板文本已还原并复核 ✅");
        } else {
            println!(
                "[clipboard_probe] ③ 原剪贴板本来就读不出文本 ⇒ 无从还原（探针留下的最后一条文本是它自己写的，见上）"
            );
        }

        println!("[clipboard_probe] 自检全部通过 ✅");
        Ok(Flow::Exit)
    }
}

fn main() -> ExitCode {
    println!(
        "[clipboard_probe] AF-2 剪贴板探针：真窗口 + 真 OS 剪贴板往返（判据 = 逐字符相等 + 两条拒绝 + 还原）"
    );
    let app = Probe { clipboard: None, original: None };
    match run(WindowConfig::new("clipboard probe", 320, 200), app) {
        Ok(()) => {
            println!("[clipboard_probe] 通过 ✅（exit=0）");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("[clipboard_probe] 失败：{e}");
            ExitCode::FAILURE
        }
    }
}
