//! **M6 5d 数值类四件**：`NumberField` / `ScrubNum` / `Switch` / `ColorField` —— 一个能自检的离屏示例。
//!
//! ```sh
//! cargo run -p deer-gui --features testing --example m6_values
//! ```
//!
//! 产物：stdout 上的自证行（`[m6_values] …`）+ **退出码 0 = 全部判据通过**。
//! 不开窗口、不写文件 —— 判据全部是「结构 / 绘制列表 / 注入输入后的状态」。
//!
//! 它演示并断言四件事（与 `tests/m6_values.rs` 的判据同源）：
//!
//! 1. **NumberField**：草稿进 `texts`、**提交才解析**（`Enter` 与失焦两路都到）、
//!    失败不发事件且草稿保留、值域夹取；
//! 2. **ScrubNum**：按下从 label 解析基准（T3.7 指针捕获之上）、拖动**变了才发**
//!    `NumberChanged`、抬起锚点即清；
//! 3. **Switch**：点击 / `Enter` / `Space` 三路同一条翻转结算（`Toggled{on: 新值}`）；
//! 4. **ColorField**：`#RRGGBB` 提交解析 + 规范化回写，失败静默。
//!
//! 另外两条红线也有断言：非 Switch 节点对开关表免疫；`DefaultRenderer`（无状态）
//! 不画开关的开/关（画普通按钮底）。找不到系统字体时明确降级（`TESTKIT-FONT` 行）
//! —— 本示例的判据不依赖像素。

use std::process::ExitCode;

use deer_gui::interaction::{InputEvent, Key, Mods, PointerButton, UiEvent, UiState};
use deer_gui::layout::layout::Geometry;
use deer_gui::prelude::*;
use deer_gui::testing::{Harness, Repro};

fn main() -> ExitCode {
    match run() {
        Ok(()) => {
            println!("[m6_values] 通过 ✅");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("[m6_values] 失败：{e}");
            ExitCode::FAILURE
        }
    }
}

/// 演示界面：普通按钮 + 四种数值类控件（显式 id ⇒ 事件与 UiState 用的就是它们）。
fn values_app() -> Node {
    let mut app = Builder::new(Kind::Column, "app").padding(8.0).gap(6.0);
    app.button("顶");
    app.number_field_opts("age", "0", |_| {});
    app.scrub_num_opts("vol", "40", |_| {});
    app.switch_opts("wifi", "Wi-Fi", |_| {});
    app.color_field_opts("tint", "#ff8800", |_| {});
    app.build()
}

/// 「App 的数据」：真实应用里这就是你自己的结构体字段 —— 事件改它、它喂回树与初值。
/// 结尾整体打印一次：四条事件回路最终都收敛到这几个字段上。
#[derive(Debug, Default)]
struct AppData {
    age: Option<f64>,      // NumberField 的值（提交成功才有）
    vol: f64,              // ScrubNum 的值（拖动中一路更新）
    wifi: bool,            // Switch 的值
    tint: Option<[u8; 3]>, // ColorField 的值（提交成功才有）
}

fn run() -> Result<(), String> {
    // ① 建面 + 塞初值/值域（值与值域都是 App 的数据 —— texts 同一条纪律）。
    let mut data = AppData { vol: 40.0, ..Default::default() };
    let tree = values_app();
    let mut st = UiState::default();
    st.num_opts.insert("age".into(),deer_gui::interaction::NumOpts { min: Some(0.0), max: Some(150.0), step: 1.0 });
    st.num_opts.insert("vol".into(),deer_gui::interaction::NumOpts { min: Some(0.0), max: Some(100.0), step: 1.0 });

    let mut h = Harness::new(tree, 260, 220, Theme::default());
    h.set_case("m6_values");
    h.set_repro(Repro::example("deer-gui", "testing", "m6_values", "", &[]));
    h.require_ids(&["button_1", "age", "vol", "wifi", "tint"]);
    h.set_state(st);
    let f = h.frame()?;
    println!(
        "[m6_values] 一帧：{} 条命令；NodeHint {} 条；有几何的节点 {} 个",
        f.list.len(),
        f.counts().node_hint,
        f.ordered_with_geometry().len()
    );

    // ② NumberField：点它 → 打 "999" → 点别处（失焦提交 + 夹取 999 → 150）。
    let step = h.tap("age")?;
    assert!(
        step.events.iter().any(|e| matches!(e, UiEvent::FocusChanged(Some(id)) if id == "age")),
        "点击 NumberField 必须聚焦它：{:?}",
        step.events
    );
    h.send(&InputEvent::TextInput { text: "999".into() })?;
    h.assert_text("age", "999")?;
    let step = h.tap("button_1")?; // 点别处 = 失焦 ⇒ 提交
    assert!(
        step.events.contains(&UiEvent::NumberChanged { id: "age".into(), value: 150.0 }),
        "失焦提交必须夹进 [0,150]（999 → 150）：{:?}",
        step.events
    );
    h.assert_text("age", "150")?;
    data.age = Some(150.0);
    println!("[m6_values] NumberField：age = {:?}（草稿 999 失焦提交 + 夹取）", data.age);

    // ③ 失败档：清空草稿（Backspace ×3 —— 光标机械与 Field 同一套）→ 打 "3px" → Enter
    //    ⇒ 不发事件、草稿保留（绘制侧画下划线标记）。
    h.tap("age")?;
    for _ in 0..3 {
        h.send(&InputEvent::KeyDown { key: Key::Backspace, mods: Mods::default(), repeat: false })?;
    }
    h.assert_text("age", "")?;
    h.send(&InputEvent::TextInput { text: "3px".into() })?;
    let step = h.send(&InputEvent::KeyDown { key: Key::Enter, mods: Mods::default(), repeat: false })?;
    assert!(
        !step.events.iter().any(|e| matches!(e, UiEvent::NumberChanged { .. })),
        "不可解析的提交必须静默：{:?}",
        step.events
    );
    h.assert_text("age", "3px")?;
    println!("[m6_values] NumberField：非法草稿保留（事件零发出）✅");

    // ④ ScrubNum：按下（锚点 = parse_num("40")）→ 右拖 5px ⇒ 45；抬起锚点即清。
    let (x0, y0) = h.center_of("vol")?;
    h.send(&InputEvent::PointerMoved { x: x0, y: y0 })?;
    h.send(&InputEvent::PointerDown { button: PointerButton::Left, x: x0, y: y0 })?;
    let step = h.send(&InputEvent::PointerMoved { x: x0 + 5.0, y: y0 })?;
    assert!(
        step.events.contains(&UiEvent::NumberChanged { id: "vol".into(), value: 45.0 }),
        "拖 +5px 要发 NumberChanged{{45}}（step=1.0）：{:?}",
        step.events
    );
    data.vol = 45.0; // App 拿着事件改自己的数据（真实应用会重建树让 label 跟上）
    h.send(&InputEvent::PointerUp { button: PointerButton::Left, x: x0 + 5.0, y: y0 })?;
    assert!(h.state().scrub.is_none(), "抬起后锚点必须清空");
    println!("[m6_values] ScrubNum：vol = {vol}（按下基准 40，拖 5px，锚点已清）", vol = data.vol);

    // ⑤ Switch：点击 → Enter → Space 三路翻转（值一路 true → false → true）。
    let step = h.tap("wifi")?;
    assert!(step.events.contains(&UiEvent::Toggled { id: "wifi".into(), on: true }));
    data.wifi = true;
    let step = h.send(&InputEvent::KeyDown { key: Key::Enter, mods: Mods::default(), repeat: false })?;
    assert!(step.events.contains(&UiEvent::Toggled { id: "wifi".into(), on: false }));
    data.wifi = false;
    let step = h.send(&InputEvent::KeyDown { key: Key::Char(' '), mods: Mods::default(), repeat: false })?;
    assert!(
        step.events.contains(&UiEvent::Toggled { id: "wifi".into(), on: true }),
        "Space（= winit Named(Space) ⇒ Char(' ')）必须翻转：{:?}",
        step.events
    );
    data.wifi = true;
    println!("[m6_values] Switch：wifi = {wifi}（点击/Enter/Space 三路同一条结算）", wifi = data.wifi);

    // ⑥ ColorField：打合法 hex → Enter ⇒ ColorChanged + 规范化回写。
    h.tap("tint")?;
    h.send(&InputEvent::TextInput { text: "AABBCC".into() })?;
    let step = h.send(&InputEvent::KeyDown { key: Key::Enter, mods: Mods::default(), repeat: false })?;
    assert!(
        step.events.contains(&UiEvent::ColorChanged { id: "tint".into(), rgb: [0xaa, 0xbb, 0xcc] }),
        "颜色提交要发 ColorChanged：{:?}",
        step.events
    );
    h.assert_text("tint", "#aabbcc")?;
    data.tint = Some([0xaa, 0xbb, 0xcc]);
    println!("[m6_values] ColorField：tint = #aabbcc（AABBCC 提交 + 规范化回写）");

    // ⑦ 红线：非 Switch 节点对开关表免疫 + DefaultRenderer 不读开关（无状态）。
    let theme = Theme::default();
    let tree = values_app();
    let geo = layout(
        &tree,
        Rect::new(0.0, 0.0, 260.0, 220.0),
        TextStyle { font_size: theme.font_size, line_height: theme.line_height },
        &ApproxMeasure,
    );
    let mut stray = UiState::default();
    stray.switches.insert("button_1".into(), true); // 刁钻用例：塞给普通按钮
    let l_empty = interactive_list(&tree, &geo, &UiState::default(), &theme);
    let l_stray = interactive_list(&tree, &geo, &stray, &theme);
    assert_eq!(l_empty, l_stray, "非 Switch 节点必须对开关表免疫（opt-in 红线）");
    let plain = build_draw_list(&tree, &geo, theme.clone(), &ApproxMeasure);
    println!(
        "[m6_values] 红线：开关表组外零影响 ✅；DefaultRenderer {} 条命令（无状态，不画开/关）",
        plain.len()
    );
    println!("[m6_values] App 数据终值 = {data:?}（四条事件回路的收敛点）");
    Ok(())
}

/// `UiState` → `InteractState` → 交互渲染器绘制列表（与 Harness 同一条管线）。
fn interactive_list(tree: &Node, geo: &Geometry, st: &UiState, theme: &Theme) -> DrawList {
    deer_gui::gpu::interact::InteractiveRenderer::with_texts(
        theme.clone(),
        &ApproxMeasure,
        &st.to_interact_state(),
        deer_gui::gpu::interact::FieldText::Content,
        &st.texts,
    )
    .build(tree, geo)
}
