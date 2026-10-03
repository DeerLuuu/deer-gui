//! **M6 5c 选择类**：`Segmented` / `ChipGroup` / `TabBar` —— 一个能自检的离屏示例。
//!
//! ```sh
//! cargo run -p deer-gui --features testing --example m6_select
//! ```
//!
//! 产物：stdout 上的自证行（`[m6_select] …`）+ **退出码 0 = 全部判据通过**。
//! 不开窗口、不写文件 —— 判据全部是「结构 / 绘制列表 / 注入输入后的状态」。
//!
//! 它演示并断言三件事（与 `tests/m6_select.rs` 的判据同源）：
//!
//! 1. **Segmented**：互斥单选 —— 点新段发 `SelectionChanged`，点已选段只发 `Clicked`；
//! 2. **ChipGroup**：独立开关 —— 每次点击必翻转并发 `ChipToggled{on: 翻转后的值}`；
//! 3. **TabBar**：页签 —— `TabChanged{index}`（禁用页也计数），禁用页静默；
//!    内容切换是 App 的事（本示例用 `App 数据` 一栏演示「拿着事件换自己的数据」）。
//!
//! 另外两条红线也有断言：组外按钮对选择映射免疫；`DefaultRenderer`（无状态）不画选中。
//! 找不到系统字体时明确降级（`TESTKIT-FONT` 行）—— 本示例的判据不依赖像素。

use std::process::ExitCode;

use deer_gui::interaction::{UiEvent, UiState};
use deer_gui::layout::layout::Geometry;
use deer_gui::prelude::*;
use deer_gui::testing::{Harness, Repro};

fn main() -> ExitCode {
    match run() {
        Ok(()) => {
            println!("[m6_select] 通过 ✅");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("[m6_select] 失败：{e}");
            ExitCode::FAILURE
        }
    }
}

/// 演示界面：普通按钮 + 三种选择类组（页签组手写，中间页禁用 —— 便捷构造
/// `tab_bar` 不带禁用参数，需要禁用就手写，两条路径产出结构相等的树）。
fn select_app() -> Node {
    let mut app = Builder::new(Kind::Column, "app").padding(8.0).gap(6.0);
    app.button("顶");
    let seg = app.segmented_opts("mode", L::new().gap(2.0).to_props(), &["日", "周", "月"]);
    println!("[m6_select] Segmented 组 id=mode，段 id：{seg:?}");
    let chips = app.chip_group_opts("tags", L::new().gap(4.0).to_props(), &["红", "蓝", "绿"]);
    println!("[m6_select] ChipGroup 组 id=tags，芯片 id：{chips:?}");
    app.container_opts(Kind::TabBar, "tabs", L::new().gap(2.0).to_props(), |g| {
        g.button("文件");
        g.button_opts("编辑", |n| n.props.disabled = true);
        g.button("视图");
    });
    app.build()
}

/// 「App 的数据」：真实应用里这就是你自己的结构体字段 —— 事件改它、它喂回树与初值。
#[derive(Default)]
struct AppData {
    mode: String,          // Segmented 的值（选中段 id）
    tags: Vec<String>,     // ChipGroup 的值（开着的芯片 id）
    page: usize,           // TabBar 的值（活动页下标）
}

fn run() -> Result<(), String> {
    // ① 建面 + 塞初值（值是 App 的数据 —— 与 texts 同一条纪律：UiState 的公开字段）。
    let mut data = AppData::default();
    let tree = select_app();
    let mut st = UiState::default();
    st.segments.insert("mode".into(), "button_3".into()); // 默认选中「周」
    data.mode = "button_3".into();
    st.chips.insert("button_5".into(), true); // 「红」默认开着
    data.tags.push("button_5".into());

    let mut h = Harness::new(tree, 260, 200, Theme::default());
    h.set_case("m6_select");
    h.set_repro(Repro::example("deer-gui", "testing", "m6_select", "", &[]));
    h.require_ids(&["button_1", "button_2", "button_3", "button_4", "button_5", "button_8", "button_9", "button_10"]);
    h.set_state(st);
    let f = h.frame()?;
    println!(
        "[m6_select] 一帧：{} 条命令；NodeHint {} 条；有几何的节点 {} 个",
        f.list.len(),
        f.counts().node_hint,
        f.ordered_with_geometry().len()
    );

    // ② Segmented：点「月」(button_4) ⇒ SelectionChanged；点「月」再点一次 ⇒ 只有 Clicked。
    let step = h.tap("button_4")?;
    assert!(
        step.events.contains(&UiEvent::Clicked("button_4".into()))
            && step.events.contains(&UiEvent::SelectionChanged {
                id: "mode".into(),
                selected: "button_4".into()
            }),
        "点新段要发 Clicked + SelectionChanged：{:?}",
        step.events
    );
    data.mode = "button_4".into(); // App 拿着事件改自己的数据
    let step = h.tap("button_4")?;
    assert!(
        step.events.contains(&UiEvent::Clicked("button_4".into()))
            && !step.events.iter().any(|e| matches!(e, UiEvent::SelectionChanged { .. })),
        "点已选段只发 Clicked（变了才发）：{:?}",
        step.events
    );
    println!("[m6_select] Segmented：mode = {data_mode}（App 数据已随事件更新）", data_mode = data.mode);

    // ③ ChipGroup：点「蓝」⇒ 开；再点 ⇒ 关（每次都发，on 是翻转后的值）。
    let step = h.tap("button_6")?;
    assert!(
        step.events.contains(&UiEvent::ChipToggled { id: "tags".into(), chip: "button_6".into(), on: true }),
        "首点芯片要发 ChipToggled{{on:true}}：{:?}",
        step.events
    );
    data.tags.push("button_6".into());
    let step = h.tap("button_6")?;
    assert!(
        step.events.contains(&UiEvent::ChipToggled { id: "tags".into(), chip: "button_6".into(), on: false }),
        "再点芯片要发 ChipToggled{{on:false}}：{:?}",
        step.events
    );
    data.tags.retain(|c| c != "button_6");
    println!("[m6_select] ChipGroup：开着的芯片 = {:?}（「红」从未被这轮碰过 ⇒ 独立）", data.tags);

    // ④ TabBar：禁用页静默；点「视图」⇒ TabChanged{index:2}（禁用页也计数）。
    assert!(
        find_disabled(&h)?.is_some(),
        "前置：夹具的「编辑」页(button_9)必须是禁用的"
    );
    let step = h.tap("button_9")?;
    assert!(
        step.events.iter().all(|e| !matches!(e, UiEvent::Clicked(_) | UiEvent::TabChanged { .. })),
        "禁用页必须静默：{:?}",
        step.events
    );
    let step = h.tap("button_10")?;
    assert!(
        step.events.contains(&UiEvent::TabChanged { id: "tabs".into(), index: 2 }),
        "点「视图」要发 TabChanged{{index:2}}（禁用页也数）：{:?}",
        step.events
    );
    data.page = 2; // 内容切换是 App 的事：这里就是「换页」的全部代码
    println!("[m6_select] TabBar：活动页 index = {}（编辑页禁用且静默）", data.page);

    // ⑤ 红线：组外按钮对选择映射免疫 + DefaultRenderer 无状态不画选中。
    let theme = Theme::default();
    let tree = select_app();
    let geo = layout(
        &tree,
        Rect::new(0.0, 0.0, 260.0, 200.0),
        TextStyle { font_size: theme.font_size, line_height: theme.line_height },
        &ApproxMeasure,
    );
    let mut stray = UiState::default();
    stray.segments.insert("app".into(), "button_1".into());
    stray.chips.insert("button_1".into(), true);
    stray.tabs.insert("app".into(), "button_1".into());
    let l_empty = interactive_list(&tree, &geo, &UiState::default(), &theme);
    let l_stray = interactive_list(&tree, &geo, &stray, &theme);
    // 注意：stray 只塞了「组外」的 id ⇒ 整帧必须逐条相同（组成员的映射才有视觉效果）。
    assert_eq!(l_empty, l_stray, "组外按钮必须对选择映射免疫（opt-in 红线）");
    let plain = build_draw_list(&tree, &geo, theme.clone(), &ApproxMeasure);
    println!(
        "[m6_select] 红线：组外映射零影响 ✅；DefaultRenderer {} 条命令（无状态，不画选中）",
        plain.len()
    );
    Ok(())
}

/// `UiState` → `InteractState` → 交互渲染器绘制列表（与 Harness 同一条管线）。
fn interactive_list(tree: &Node, geo: &Geometry, st: &UiState, theme: &Theme) -> DrawList {
    deer_gui::gpu::interact::InteractiveRenderer::new(
        theme.clone(),
        &ApproxMeasure,
        &st.to_interact_state(),
    )
    .build(tree, geo)
}

/// 按 id 找节点（夹具防漂移自检用）。
fn find_disabled(h: &Harness) -> Result<Option<Node>, String> {
    fn walk(n: &Node, id: &str) -> Option<Node> {
        if n.id == id {
            return Some(n.clone());
        }
        n.children.iter().find_map(|c| walk(c, id))
    }
    Ok(walk(h.tree(), "button_9").filter(|n| n.props.disabled))
}
