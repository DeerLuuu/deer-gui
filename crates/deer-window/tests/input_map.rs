//! deer-window 的**输入映射**单测（M5-1）：不建窗口、不跑事件循环。
//!
//! 关键纪律：映射函数吃的是 winit 的**字段**（`&Key`、`MouseButton`、`MouseScrollDelta`…），
//! 不是 winit 的**事件对象** —— 所以这些测试不用构造 `WindowEvent`/`KeyEvent`
//! （那种构造又难写、又容易随 winit 版本脆断）。
//!
//! 真开窗 + 真按键的验证在 `examples/input_probe.rs`（CI 无桌面，`#[test]` 里真开窗会假红）。

use deer_gpu::Extent;
use deer_window::{
    App, Flow, InputEvent, Key, Mods, PointerButton, WindowInfo, map_key, map_mods,
    map_mouse_button, map_wheel, printable_text, raw_handle_from_win32,
};
use winit::dpi::PhysicalPosition;
use winit::event::{MouseButton, MouseScrollDelta};
use winit::keyboard::{Key as WinitKey, ModifiersState, NamedKey};

/// 表驱动：命名键 → [`Key`]。**未知/不映射的命名键一律落到 `Key::Other`**（不猜、不 panic）。
#[test]
fn map_key_named_table() {
    let table: &[(NamedKey, Key)] = &[
        // —— 接口冻结的那几个：一一对应 ——
        (NamedKey::Tab, Key::Tab),
        (NamedKey::Escape, Key::Escape),
        (NamedKey::Enter, Key::Enter),
        (NamedKey::Backspace, Key::Backspace),
        (NamedKey::ArrowLeft, Key::Left),
        (NamedKey::ArrowRight, Key::Right),
        (NamedKey::ArrowUp, Key::Up),
        (NamedKey::ArrowDown, Key::Down),
        // 空格是**可打印字符**（不是命令键）⇒ 走 Char(' ')，且会被送进 TextInput。
        (NamedKey::Space, Key::Char(' ')),
        // —— 未知键 ⇒ Other ——
        (NamedKey::Shift, Key::Other),
        (NamedKey::Control, Key::Other),
        (NamedKey::Alt, Key::Other),
        (NamedKey::AltGraph, Key::Other),
        (NamedKey::Super, Key::Other),
        (NamedKey::CapsLock, Key::Other),
        (NamedKey::NumLock, Key::Other),
        (NamedKey::ScrollLock, Key::Other),
        (NamedKey::F1, Key::Other),
        (NamedKey::F35, Key::Other),
        (NamedKey::Delete, Key::Other),
        (NamedKey::Insert, Key::Other),
        // T3.2 按键滚动：四个滚动键首次有了自己的变体（此前落进 Other）
        (NamedKey::Home, Key::Home),
        (NamedKey::End, Key::End),
        (NamedKey::PageUp, Key::PageUp),
        (NamedKey::PageDown, Key::PageDown),
        (NamedKey::ContextMenu, Key::Other),
        (NamedKey::Pause, Key::Other),
        (NamedKey::PrintScreen, Key::Other),
        (NamedKey::Clear, Key::Other),
    ];

    for (named, expected) in table {
        let got = map_key(&WinitKey::Named(*named), Mods::default());
        assert_eq!(got, *expected, "NamedKey::{named:?} 应映射成 {expected:?}，实际 {got:?}");
    }
}

/// 表驱动：字符键 → [`Key::Char`]；多字符/空串 → [`Key::Other`]。
///
/// 用 `chars()` 计数（不是字节长度）：`"中"` 是 3 字节、`"😀"` 是 4 字节，但都只有 **1** 个字符。
#[test]
fn map_key_character_table() {
    let table: &[(&str, Key)] = &[
        ("a", Key::Char('a')),
        ("A", Key::Char('A')), // Shift 的作用已由 winit 写进 logical_key
        ("0", Key::Char('0')),
        ("!", Key::Char('!')),
        (" ", Key::Char(' ')),
        ("中", Key::Char('中')),
        ("😀", Key::Char('😀')),
        ("", Key::Other),   // 空串（理论上不出现）：不许 panic
        ("ab", Key::Other), // 死键组合出的多字符串 ⇒ Other
        ("中a", Key::Other),
    ];

    for (text, expected) in table {
        let got = map_key(&WinitKey::Character((*text).into()), Mods::default());
        assert_eq!(got, *expected, "Character({text:?}) 应映射成 {expected:?}，实际 {got:?}");
    }
}

/// 死键 / 无法识别的键：`Key::Other`（不是 panic，也不是「猜一个字符」）。
#[test]
fn map_key_dead_and_unidentified_are_other() {
    // 死键（`Dead(Some('^'))` 是「按下了死键」）本层不建模 ⇒ Other。
    let dead: WinitKey = WinitKey::Dead(None);
    assert_eq!(map_key(&dead, Mods::default()), Key::Other);
    let dead_with_char: WinitKey = WinitKey::Dead(Some('^'));
    assert_eq!(map_key(&dead_with_char, Mods::default()), Key::Other);
}

/// 修饰键组合**不改变** `map_key` 的结果 —— 这是冻结契约：
/// `mods` 只随 [`InputEvent`] 透传，判定「按的是哪个键」不看修饰键。
#[test]
fn map_key_ignores_mods() {
    let combos = [
        Mods::default(),
        Mods { shift: true, ..Mods::default() },
        Mods { ctrl: true, ..Mods::default() },
        Mods { alt: true, ..Mods::default() },
        Mods { sup: true, ..Mods::default() },
        Mods { shift: true, ctrl: true, ..Mods::default() },
        Mods { shift: true, ctrl: true, alt: true, sup: true },
    ];

    for mods in combos {
        assert_eq!(map_key(&WinitKey::Named(NamedKey::Tab), mods), Key::Tab, "mods={mods:?}");
        assert_eq!(
            map_key(&WinitKey::Character("A".into()), mods),
            Key::Char('A'),
            "mods={mods:?}"
        );
        assert_eq!(map_key(&WinitKey::Named(NamedKey::F1), mods), Key::Other, "mods={mods:?}");
    }
}

/// 表驱动：winit 修饰键位标志 → [`Mods`]（含组合与「一个都没按」）。
#[test]
fn map_mods_table() {
    let table: &[(ModifiersState, Mods)] = &[
        (ModifiersState::empty(), Mods::default()),
        (ModifiersState::SHIFT, Mods { shift: true, ..Mods::default() }),
        (ModifiersState::CONTROL, Mods { ctrl: true, ..Mods::default() }),
        (ModifiersState::ALT, Mods { alt: true, ..Mods::default() }),
        (ModifiersState::SUPER, Mods { sup: true, ..Mods::default() }),
        (
            ModifiersState::SHIFT | ModifiersState::CONTROL,
            Mods { shift: true, ctrl: true, ..Mods::default() },
        ),
        (
            ModifiersState::SHIFT | ModifiersState::ALT,
            Mods { shift: true, alt: true, ..Mods::default() },
        ),
        (
            ModifiersState::SHIFT
                | ModifiersState::CONTROL
                | ModifiersState::ALT
                | ModifiersState::SUPER,
            Mods { shift: true, ctrl: true, alt: true, sup: true },
        ),
    ];

    for (state, expected) in table {
        let got = map_mods(*state);
        assert_eq!(got, *expected, "ModifiersState({state:?}) 应映射成 {expected:?}，实际 {got:?}");
    }
}

/// 表驱动：鼠标按键 → [`PointerButton`]。**侧键/未知键 ⇒ `None`**（丢弃），
/// 绝不降级成左键（那会凭空产生点击）。
#[test]
fn map_mouse_button_table() {
    let table: &[(MouseButton, Option<PointerButton>)] = &[
        (MouseButton::Left, Some(PointerButton::Left)),
        (MouseButton::Right, Some(PointerButton::Right)),
        (MouseButton::Middle, Some(PointerButton::Middle)),
        (MouseButton::Back, None),
        (MouseButton::Forward, None),
        (MouseButton::Other(0), None),
        (MouseButton::Other(7), None),
        (MouseButton::Other(u16::MAX), None),
    ];

    for (button, expected) in table {
        let got = map_mouse_button(*button);
        assert_eq!(got, *expected, "MouseButton::{button:?} 应映射成 {expected:?}，实际 {got:?}");
    }
}

/// 表驱动：滚轮增量原样透传（行/像素两种口径都不改数值）。
#[test]
fn map_wheel_table() {
    let line: &[(f32, f32)] = &[(0.0, 0.0), (1.0, -1.0), (-2.5, 3.25), (0.0, 120.0)];
    for (dx, dy) in line {
        let got = map_wheel(&MouseScrollDelta::LineDelta(*dx, *dy));
        assert_eq!(got, (*dx, *dy), "LineDelta({dx}, {dy}) 应原样透传");
    }

    let pixel: &[(f64, f64)] = &[(0.0, 0.0), (12.5, -3.25), (-1024.0, 768.0)];
    for (dx, dy) in pixel {
        let got = map_wheel(&MouseScrollDelta::PixelDelta(PhysicalPosition::new(*dx, *dy)));
        assert_eq!(
            got,
            (*dx as f32, *dy as f32),
            "PixelDelta({dx}, {dy}) 应转成 f32 物理像素"
        );
    }
}

/// 表驱动：**可打印文本**的过滤器 —— 这是「字符 vs 物理键」分工的关键一环。
#[test]
fn printable_text_table() {
    let table: &[(Option<&str>, Option<&str>)] = &[
        // —— 算文本 ——
        (Some("a"), Some("a")),
        (Some("A"), Some("A")),
        (Some(" "), Some(" ")),
        (Some("中"), Some("中")),
        (Some("😀"), Some("😀")),
        (Some("汉字abc"), Some("汉字abc")),
        (Some("!@#"), Some("!@#")),
        // —— 不算文本：winit 给 Enter/Tab/Backspace/Esc/Ctrl+字母 的“文本”是控制字符 ——
        (Some("\r"), None),
        (Some("\n"), None),
        (Some("\t"), None),
        (Some("\x08"), None),
        (Some("\x1b"), None),
        (Some("\u{1}"), None),
        (Some("a\r"), None), // 混了一个控制字符 ⇒ 整条不算文本（不切一半）
        // —— 不算文本：没有文本 ——
        (None, None),
        (Some(""), None),
    ];

    for (input, expected) in table {
        let got = printable_text(*input);
        assert_eq!(
            got.as_deref(),
            *expected,
            "printable_text({input:?}) 应得到 {expected:?}，实际 {got:?}"
        );
    }
}

/// `InputEvent` 可比较、可克隆 —— M5-4 的脚本化重放要靠它做断言/复制。
#[test]
fn input_event_is_comparable_and_cloneable() {
    let down = InputEvent::PointerDown { button: PointerButton::Left, x: 12.5, y: -3.0 };
    assert_eq!(down.clone(), down);
    assert_ne!(down, InputEvent::PointerUp { button: PointerButton::Left, x: 12.5, y: -3.0 });
    assert_ne!(down, InputEvent::PointerDown { button: PointerButton::Right, x: 12.5, y: -3.0 });

    let text = InputEvent::TextInput { text: "汉字".to_string() };
    let copy = text.clone();
    assert_eq!(copy, text);

    let key = InputEvent::KeyDown { key: Key::Char('中'), mods: Mods { ctrl: true, ..Mods::default() }, repeat: false };
    assert_eq!(key.clone(), key);
    assert_ne!(
        key,
        InputEvent::KeyDown { key: Key::Char('中'), mods: Mods::default(), repeat: false },
        "mods 参与相等判定（Ctrl+中 与 中 不是同一个事件）"
    );
    assert_ne!(key, InputEvent::KeyUp { key: Key::Char('中'), mods: Mods { ctrl: true, ..Mods::default() } });
}

/// `Key`/`Mods`/`PointerButton` 是 Copy + Eq 的小值类型（可随便传、可当 map 键）。
#[test]
fn input_model_values_are_copy_and_eq() {
    let key = Key::Char('中');
    let key_copy = key; // Copy
    assert_eq!(key_copy, Key::Char('中'));
    assert_ne!(Key::Char('中'), Key::Other);
    assert_ne!(Key::Left, Key::Right);

    let mods = Mods { shift: true, ctrl: false, alt: false, sup: false };
    let mods_copy = mods;
    assert_eq!(mods_copy, mods);
    assert_ne!(mods, Mods::default());

    let button = PointerButton::Middle;
    let button_copy = button;
    assert_eq!(button_copy, PointerButton::Middle);
    assert_ne!(PointerButton::Left, PointerButton::Right);
}

/// `App::input` 的默认实现 = **无副作用 + `Flow::Continue`**：
/// M5 之前写的 `App` 实现（只实现 init/redraw）不用改一行也照样编译、照样跑。
#[test]
fn app_input_default_is_side_effect_free() {
    /// 刻意**不**实现 `input` —— 这条测试就是来钉住默认实现的。
    struct Silent;

    impl App for Silent {
        fn init(&mut self, _info: &WindowInfo) -> Result<(), String> {
            Ok(())
        }
        fn redraw(&mut self) -> Result<Flow, String> {
            Ok(Flow::Continue)
        }
    }

    let mut app = Silent;
    let info = WindowInfo {
        raw: raw_handle_from_win32(0x1A2B, 0x7FF6_0000),
        extent: Extent { width: 320, height: 200 },
        scale_factor: 1.0,
    };
    let events = [
        InputEvent::PointerMoved { x: 0.0, y: 0.0 },
        InputEvent::PointerDown { button: PointerButton::Left, x: 1.0, y: 2.0 },
        InputEvent::PointerUp { button: PointerButton::Right, x: 1.0, y: 2.0 },
        InputEvent::Wheel { dx: 1.0, dy: -1.0 },
        InputEvent::KeyDown { key: Key::Tab, mods: Mods::default(), repeat: false },
        InputEvent::KeyUp { key: Key::Char('x'), mods: Mods { shift: true, ..Mods::default() } },
        InputEvent::TextInput { text: "hi".to_string() },
        InputEvent::FocusChanged { focused: false },
        // AF-3：新事件也必须被**默认实现**安静接住 —— M5 之前写的 App 一行不用改。
        InputEvent::ScaleFactorChanged { scale_factor: 1.25 },
    ];

    for ev in &events {
        assert_eq!(
            app.input(&info, ev).expect("默认实现永不报错"),
            Flow::Continue,
            "默认实现必须返回 Continue（否则既有 App 会莫名退出）：{ev:?}"
        );
    }
}
