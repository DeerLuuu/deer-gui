//! **输入脚本**：把 `DEER_INPUT_SCRIPT` 那种字符串翻成一串 [`InputEvent`]（M5-4）。
//!
//! # 为什么脚本语法要住在库里，而不是住在 example 里
//!
//! 窗口侧的重放（`examples/interactive_form.rs`）需要真窗口，跑不进 `cargo test`；
//! 而「脚本 → 事件 → 状态」这条链**是纯逻辑**，能在任何环境里回归。两边共用**同一份**
//! 解析器（`App` 只负责把事件一个个喂给 `interaction::handle`），脚本语法就只有一处定义，
//! 不会出现「example 里能跑、测试里的脚本是另一套」这种分叉。
//!
//! # 语法
//!
//! 语句用 `;`（或换行）分隔；**空语句被忽略**（`;;`、结尾多一个 `;` 都不算错）。
//! `#` 到行尾是注释。
//!
//! | 语法 | 含义 |
//! |---|---|
//! | `move:X,Y` | 指针移到 `(X, Y)`（f32，贴边/负数都允许） |
//! | `down:left` / `down:right` / `down:middle` | 指针按下（坐标为**最近一次 `move`**；还没 move 过就是 `(0,0)`） |
//! | `up:left`（同上三档） | 指针抬起（坐标口径同 `down`） |
//! | `key:Tab` | `KeyDown`。可写 `Escape`/`Enter`/`Backspace`/`Left`/`Right`/`Up`/`Down`/`Other`/`Char(a)`。**键名后加 `*`（`key:Tab*`）= 系统按键重复**（`repeat: true`，T3.6） |
//! | `shift+key:Tab`（或 `key:shift+Tab`） | 同上，带 Shift。`ctrl+`/`alt+`/`sup+` 同档，用 `+` 可叠加；前缀写在**动词一侧或键名一侧都行** |
//! | `text:hi` | 一段文本输入（原样，含空格与中文；是**追加**不是覆盖） |
//! | `focus:off` / `focus:on` | 窗口失焦 / 重新获得窗口焦点（`FocusChanged`） |
//! | `wheel:0,3` | 滚轮（本期状态机不消费，脚本里可以写出来验证「不消费的事件不改状态」） |
//!
//! 语法错（未知语句、未知键名、坐标不是数字…）⇒ `Err(String)`，**带语句序号与原文**。
//! 刻意不「跳过看不懂的语句」：脚本是判据的一部分，静默跳过等于判据悄悄失效。
//!
//! # `move:` 为什么必须显式写坐标
//!
//! `down`/`up` 的坐标来自「最近一次 `move`」——这正是窗口层的行为（winit 的
//! `MouseInput` 只给按键，坐标由 `CursorMoved` 记账）。写成显式坐标是**故意**的：
//! 脚本必须自足，「指针现在在哪」不能是脚本之外的隐式状态。
//!
//! ```
//! use deer_gui::input_script::parse_script;
//! use deer_gui::interaction::{InputEvent, Key, PointerButton};
//!
//! let evs = parse_script("move:10,20; down:left; up:left; key:Tab; text:hi").unwrap();
//! assert_eq!(evs.len(), 5);
//! assert_eq!(evs[0], InputEvent::PointerMoved { x: 10.0, y: 20.0 });
//! assert_eq!(
//!     evs[1],
//!     InputEvent::PointerDown { button: PointerButton::Left, x: 10.0, y: 20.0 }
//! );
//! assert_eq!(evs[3], InputEvent::KeyDown { key: Key::Tab, mods: Default::default(), repeat: false });
//! assert_eq!(evs[4], InputEvent::TextInput { text: "hi".to_string() });
//! ```

use std::collections::BTreeMap;

use deer_core::layout::Geometry;
use deer_core::node::Node;

use crate::interaction::{
    ClipSnapshot, InputEvent, Key, Mods, PointerButton, UiEvent, UiState, handle,
};

/// 环境变量名（窗口侧 example 从这里读默认脚本）。
pub const ENV_VAR: &str = "DEER_INPUT_SCRIPT";

/// 默认脚本：用来演示「输入 → 命中 → 状态 → 重绘」整条链，而且每一步都有可断言的效果。
pub const DEFAULT_SCRIPT: &str = "move:40,20;down:left;up:left;key:Tab;text:hi";

/// 把脚本字符串解析成一串输入事件。
pub fn parse_script(src: &str) -> Result<Vec<InputEvent>, String> {
    let mut out = Vec::new();
    // `down`/`up` 的坐标 = 最近一次 `move`（与窗口层的记账口径一致，见模块文档）。
    let mut cursor = (0.0f32, 0.0f32);

    for (idx, raw) in src.split([';', '\n']).enumerate() {
        let stmt = raw.split('#').next().unwrap_or("").trim();
        if stmt.is_empty() {
            continue;
        }
        let where_ = format!("第 {} 条语句 `{stmt}`", idx + 1);
        let (verb, arg) = stmt
            .split_once(':')
            .ok_or_else(|| format!("{where_} 没有 `:`（语法是 `动词:参数`，例如 `key:Tab`）"))?;
        // 修饰键前缀可以写在**动词**一侧（`shift+key:Tab`，与用户预期一致）。
        let (verb, mods_prefix) = strip_mods_prefix(verb.trim());
        let arg = arg.trim();
        match verb {
            "move" => {
                let (x, y) = parse_pair(arg).map_err(|e| format!("{where_} {e}"))?;
                cursor = (x, y);
                out.push(InputEvent::PointerMoved { x, y });
            }
            "down" | "up" => {
                let button = parse_button(arg).map_err(|e| format!("{where_} {e}"))?;
                let (x, y) = cursor;
                out.push(if verb == "down" {
                    InputEvent::PointerDown { button, x, y }
                } else {
                    InputEvent::PointerUp { button, x, y }
                });
            }
            "wheel" => {
                let (dx, dy) = parse_pair(arg).map_err(|e| format!("{where_} {e}"))?;
                out.push(InputEvent::Wheel { dx, dy });
            }
            "key" | "keydown" => {
                // `repeat` 默认 false：脚本重放写的是「用户按下一次」；
                // 要模拟长按重复，键名后加 `*`（`key:Tab*`）—— 与坐标的 `@id` 定位
                // 一样是「动词参数的修饰」，不另立新动词。
                let (name, repeat) = match arg.strip_suffix('*') {
                    Some(rest) => (rest, true),
                    None => (arg, false),
                };
                let (key, mods) = parse_key(name, mods_prefix)
                    .map_err(|e| format!("{where_} {e}"))?;
                out.push(InputEvent::KeyDown { key, mods, repeat });
            }
            "keyup" => {
                let (key, mods) = parse_key(arg, mods_prefix).map_err(|e| format!("{where_} {e}"))?;
                out.push(InputEvent::KeyUp { key, mods });
            }
            "text" => out.push(InputEvent::TextInput {
                text: arg.to_string(),
            }),
            "focus" => {
                let focused = match arg {
                    "on" => true,
                    "off" => false,
                    other => {
                        return Err(format!(
                            "{where_} `focus:` 只接受 `on`/`off`，实际 `{other}`"
                        ));
                    }
                };
                out.push(InputEvent::FocusChanged { focused });
            }
            other => {
                return Err(format!(
                    "{where_} 未知动词 `{other}`（可用：move/down/up/wheel/key/keyup/text/focus）"
                ));
            }
        }
    }
    Ok(out)
}

/// 剥掉动词一侧的修饰键前缀（`shift+ctrl+key` ⇒ `("key", {shift, ctrl})`）。
///
/// 只认已知前缀：`foo+key` 会**原样返回** `foo+key`，于是落进「未知动词」那条错误里
/// —— 不静默丢掉看不懂的前缀（那样脚本会少一个修饰键而没人发现）。
fn strip_mods_prefix(verb: &str) -> (&str, Mods) {
    let mut mods = Mods::default();
    let mut rest = verb;
    while let Some((head, tail)) = rest.split_once('+') {
        match head.trim() {
            "shift" => mods.shift = true,
            "ctrl" => mods.ctrl = true,
            "alt" => mods.alt = true,
            "sup" | "super" | "meta" => mods.sup = true,
            _ => return (verb, Mods::default()),
        }
        rest = tail.trim();
    }
    if mods == Mods::default() {
        (verb, mods)
    } else {
        (rest, mods)
    }
}

/// `X,Y`（f32；两个数都必须能解析）。
fn parse_pair(s: &str) -> Result<(f32, f32), String> {
    let (a, b) = s
        .split_once(',')
        .ok_or_else(|| format!("`{s}` 不是 `X,Y` 形式"))?;
    let x = a
        .trim()
        .parse::<f32>()
        .map_err(|e| format!("`{}` 不是数字：{e}", a.trim()))?;
    let y = b
        .trim()
        .parse::<f32>()
        .map_err(|e| format!("`{}` 不是数字：{e}", b.trim()))?;
    Ok((x, y))
}

fn parse_button(s: &str) -> Result<PointerButton, String> {
    match s {
        "left" => Ok(PointerButton::Left),
        "right" => Ok(PointerButton::Right),
        "middle" => Ok(PointerButton::Middle),
        other => Err(format!(
            "`{other}` 不是指针键（可用：left/right/middle —— 侧键在事件模型里**没有**变体，写不出来）"
        )),
    }
}

/// 裸键名 / `Char(x)` ⇒ `Key`（修饰键前缀两侧都收，见 [`strip_mods_prefix`]）。
fn parse_key(s: &str, mods: Mods) -> Result<(Key, Mods), String> {
    let (name, extra) = strip_mods_prefix(s.trim());
    let mods = Mods {
        shift: mods.shift || extra.shift,
        ctrl: mods.ctrl || extra.ctrl,
        alt: mods.alt || extra.alt,
        sup: mods.sup || extra.sup,
    };
    let key = match name {
        "Tab" => Key::Tab,
        "Escape" | "Esc" => Key::Escape,
        "Enter" => Key::Enter,
        "Backspace" => Key::Backspace,
        "Left" => Key::Left,
        "Right" => Key::Right,
        "Up" => Key::Up,
        "PageUp" => Key::PageUp,
        "PageDown" => Key::PageDown,
        "Home" => Key::Home,
        "End" => Key::End,
        "Down" => Key::Down,
        "Other" => Key::Other,
        other => {
            // `Char(x)`：恰一个字符。多字符（死键组合）没有对应变体 ⇒ 报错而不是猜。
            if let Some(inner) = other
                .strip_prefix("Char(")
                .and_then(|r| r.strip_suffix(')'))
            {
                let mut it = inner.chars();
                match (it.next(), it.next()) {
                    (Some(c), None) => Key::Char(c),
                    _ => {
                        return Err(format!(
                            "`Char({inner})` 必须是**恰好一个字符**（多字符是 `Other`，不是 `Char`）"
                        ));
                    }
                }
            } else {
                return Err(format!(
                    "`{other}` 不是键名（可用：Tab/Escape/Esc/Enter/Backspace/Left/Right/Up/Down/Other/Char(x)）\
                     —— 单字符键请写成 `key:Char(a)`（`key:Tab` 这种物理键**不会**产生文本输入）"
                ));
            }
        }
    };
    Ok((key, mods))
}

/// 一次脚本重放的**账本**（可断言、可打印）。
///
/// ⚠️ M6 5d 起只保 `PartialEq`：`UiEvent::NumberChanged` 携带 `f64`、`UiState`
/// 也含浮点字段（`NumOpts`/`ScrubAnchor`）⇒ `Eq` 不再成立（与 `UiEvent`/`UiState`
/// 的 derive 同步收窄，理由见 `interaction.rs`）。
#[derive(Debug, Clone, PartialEq)]
pub struct ReplayResult {
    /// 脚本里每一条事件产生的 UI 事件（拼接在一起，按时间序）。
    pub events: Vec<UiEvent>,
    /// 重放的**最终**状态（窗口层渲染的就是它）。
    pub state: UiState,
    /// 每一帧**是否真的需要重绘**（判据：这一步之后 `hover/focus/pressed` 与上一步不同）。
    ///
    /// 长度与脚本事件数相同（含第一条）。
    pub redrew: Vec<bool>,
}

impl ReplayResult {
    /// 真的触发了重绘的步数。
    pub fn redraw_count(&self) -> usize {
        self.redrew.iter().filter(|d| **d).count()
    }
}

/// 纯逻辑重放：脚本 → 事件 →（`handle`）→ 状态 + 事件日志 + dirty 序列。
///
/// `snap` 是命中用的裁剪快照（窗口层从**真实绘制列表**派生；纯逻辑侧可以给
/// [`ClipSnapshot::unclipped()`] 或手工登记的快照）。
pub fn replay(
    src: &str,
    root: &Node,
    geo: &Geometry,
    snap: ClipSnapshot,
) -> Result<ReplayResult, String> {
    let events = parse_script(src)?;
    let mut state = UiState::default();
    let mut log = Vec::new();
    let mut redrew = Vec::new();
    for ev in &events {
        let before = state.clone();
        let out = handle(&mut state, root, geo, snap.clone(), ev);
        log.extend(out);
        // dirty 判据：**状态**（含 `texts`）真的不同了才需要重绘 —— 与窗口层同一条规则。
        redrew.push(before != state);
    }
    Ok(ReplayResult {
        events: log,
        state,
        redrew,
    })
}

/// 状态里的文本缓冲（便于断言与打印）。
pub fn texts_of(state: &UiState) -> BTreeMap<String, String> {
    state.texts.clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// T3.6：键名后加 `*` ⇒ `repeat: true`（模拟系统按键重复）；不加 ⇒ `false`。
    #[test]
    fn key_star_suffix_means_repeat() {
        let evs = parse_script("key:Tab*; key:Tab").unwrap();
        assert_eq!(evs.len(), 2);
        assert_eq!(
            evs[0],
            InputEvent::KeyDown { key: Key::Tab, mods: Default::default(), repeat: true },
            "`key:Tab*` 应解析成带 repeat 的 KeyDown"
        );
        assert_eq!(
            evs[1],
            InputEvent::KeyDown { key: Key::Tab, mods: Default::default(), repeat: false },
            "不带 `*` 的仍是普通按下（repeat=false）—— 既有脚本语义不回退"
        );
    }

    #[test]
    fn parses_the_documented_syntax() {
        let evs = parse_script("move:120,80;down:left;up:left;key:Tab;text:hi").unwrap();
        println!("{evs:#?}");
        assert_eq!(evs.len(), 5);
        assert_eq!(evs[0], InputEvent::PointerMoved { x: 120.0, y: 80.0 });
        assert_eq!(
            evs[1],
            InputEvent::PointerDown {
                button: PointerButton::Left,
                x: 120.0,
                y: 80.0
            },
            "down 的坐标必须来最近一次 move"
        );
        assert_eq!(
            evs[2],
            InputEvent::PointerUp {
                button: PointerButton::Left,
                x: 120.0,
                y: 80.0
            }
        );
        assert_eq!(
            evs[3],
            InputEvent::KeyDown {
                key: Key::Tab,
                mods: Mods::default(),
                repeat: false,
            }
        );
        assert_eq!(evs[4], InputEvent::TextInput { text: "hi".into() });

        // 未 `move` 就先 `down` ⇒ 坐标是 (0,0)（不是「上一条语句的坐标」，不是随机）。
        let evs = parse_script("down:left").unwrap();
        assert_eq!(
            evs[0],
            InputEvent::PointerDown {
                button: PointerButton::Left,
                x: 0.0,
                y: 0.0
            }
        );
    }

    /// 修饰键前缀写在动词一侧或键名一侧都必须被接受（两种写法逐字得到同一个事件）。
    #[test]
    fn modifier_prefix_may_sit_on_either_side_of_the_colon() {
        let a = parse_script("shift+key:Tab").unwrap();
        let b = parse_script("key:shift+Tab").unwrap();
        println!("{a:?} / {b:?}");
        assert_eq!(a, b, "两种相等写法必须解析成同一个事件");
        assert_eq!(
            a[0],
            InputEvent::KeyDown {
                key: Key::Tab,
                mods: Mods {
                    shift: true,
                    ..Default::default()
                },
                repeat: false,
            }
        );

        // 多个修饰键叠加。
        let c = parse_script("ctrl+alt+key:Enter").unwrap();
        assert_eq!(
            c[0],
            InputEvent::KeyDown {
                key: Key::Enter,
                mods: Mods {
                    ctrl: true,
                    alt: true,
                    ..Default::default()
                },
                repeat: false,
            }
        );

        // 看不懂的前缀**不静默丢弃**：`foo+key` 落进「未知动词」。
        let e = parse_script("foo+key:Tab").expect_err("未知前缀必须报错");
        println!("{e}");
        assert!(e.contains("未知动词"), "实际 `{e}`");
    }

    #[test]
    fn supports_modifiers_comments_blank_statements_and_text_with_spaces() {
        let evs = parse_script(
            "  # 注释\nshift+key:Tab ;; text:hello world ; keyup:Char(a); focus:off; wheel:0,3\n",
        )
        .unwrap();
        println!("{evs:#?}");
        assert_eq!(evs.len(), 5, "空语句与注释不该产生事件");
        assert_eq!(
            evs[0],
            InputEvent::KeyDown {
                key: Key::Tab,
                mods: Mods {
                    shift: true,
                    ..Default::default()
                },
                repeat: false,
            }
        );
        assert_eq!(evs[1], InputEvent::TextInput { text: "hello world".into() });
        assert_eq!(
            evs[2],
            InputEvent::KeyUp {
                key: Key::Char('a'),
                mods: Mods::default()
            }
        );
        assert_eq!(evs[3], InputEvent::FocusChanged { focused: false });
        assert_eq!(evs[4], InputEvent::Wheel { dx: 0.0, dy: 3.0 });
    }

    /// 语法错必须**报错**（带语句序号），不许静默跳过 —— 否则脚本判据会悄悄失效。
    #[test]
    fn malformed_scripts_are_rejected_with_the_statement_number() {
        for (src, needle) in [
            ("key", "没有 `:`"),
            ("frob:1", "未知动词"),
            ("move:1", "不是 `X,Y`"),
            ("move:a,1", "不是数字"),
            ("down:back", "不是指针键"),
            ("key:F1", "不是键名"),
            ("key:Char(ab)", "恰好一个字符"),
            ("focus:maybe", "只接受 `on`/`off`"),
            ("move:1,2;bad:3", "第 2 条语句"),
        ] {
            let e = parse_script(src).expect_err("这条脚本必须被拒绝");
            println!("`{src}` ⇒ {e}");
            assert!(e.contains(needle), "错误文案该含 `{needle}`，实际 `{e}`");
        }
    }

    /// 同一串脚本解析两次 ⇒ 逐条相同（可回归的前提）。
    #[test]
    fn parsing_is_deterministic() {
        let src = "move:1,2;down:left;key:shift+Tab;key:Backspace;text:你好;up:left;focus:off";
        let a = parse_script(src).unwrap();
        let b = parse_script(src).unwrap();
        assert_eq!(a, b);
        assert_eq!(a.len(), 7);
    }
}
