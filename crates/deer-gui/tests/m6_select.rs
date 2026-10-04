//! M6 5c 选择类（`Segmented` / `ChipGroup` / `TabBar`）的**集成判据**。
//!
//! 判据面与出处（指南在 `docs/features/{segmented,chip-group,tab-bar}.md`）：
//!
//! | 判据 | 钉的是什么 | 更底层的出处 |
//! |---|---|---|
//! | `segmented_*` | 互斥单选：点了才换、换了才发 `SelectionChanged`、`Enter` 与指针同路 | `src/interaction.rs` 的 r14a + `resolve_selection` |
//! | `chip_group_*` | 独立开关：每次点击必翻转发 `ChipToggled{on: 新值}`、互不干扰、初值可塞 | r14b |
//! | `tab_bar_*` | 页签：`TabChanged{index}`（**禁用页也计数**）、禁用页静默、点当前页不发 | r14c |
//! | `keep_*` | 三张值表是应用数据：整树重建后保留且仍可用 | `UiState` 按 id 键控（texts 同一条纪律） |
//! | `*_visual_*` | 选中视觉只落在组内成员上；组外按钮对映射免疫（opt-in 红线） | `deer-gpu/src/interact.rs` 的 muted 档 |
//! | `via_testkit` | testkit 注入路径：点击/禁用/`Enter`/**T3.1 方向键跨控件导航** | 同上 + `focusables`/`nearest_focusable` |
//!
//! 自动 id（`IdGen` 按 kind 计数，夹具按此序建 ⇒ 断言里写死）：
//! `button_1`=「顶」（普通按钮）· `segmented_1`(`button_2..4`=日/周/月) ·
//! `chip_group_1`(`button_5..7`=红/蓝/绿) · `tab_bar_1`(`button_8..10`=文件/编辑/视图，
//! 其中 `button_9`「编辑」**禁用**）。
//!
//! 运行：
//!
//! ```text
//! cargo test -p deer-gui --test m6_select                        # 纯逻辑档（默认门禁内）
//! cargo test -p deer-gui --features testing --test m6_select     # 加上 testkit 档
//! ```

use deer_gui::interaction::{
    handle, ClipSnapshot, InputEvent, Key, Mods, PointerButton, UiEvent, UiState,
};
use deer_gui::layout::layout::{layout, Geometry};
use deer_gui::prelude::*;

/// 测试用的统一字号/行高（与 `tests/m6_basics.rs` 同一套口径）。
const STYLE: TextStyle = TextStyle { font_size: 14.0, line_height: 18.0 };
/// 画布（固定 ⇒ 布局确定性，判据不随环境漂）。
const CANVAS: (f32, f32) = (260.0, 200.0);

/// 判据界面：一个普通按钮 + 三种选择类组（页签组**手写**，中间页禁用）。
fn select_app() -> Node {
    let mut app = Builder::new(Kind::Column, "app").padding(8.0).gap(6.0);
    app.button("顶");
    app.segmented_opts("mode", L::new().gap(2.0).to_props(), &["日", "周", "月"]);
    app.chip_group_opts("tags", L::new().gap(4.0).to_props(), &["红", "蓝", "绿"]);
    // 页签组手写：便捷构造 `tab_bar` 不带禁用参数（见 builder.rs 的文档），
    // 两条路径产出结构相等的树 —— 这一条本身由 `tab_bar_hand_written_equivalence` 钉住。
    app.container_opts(Kind::TabBar, "tabs", L::new().gap(2.0).to_props(), |g| {
        g.button("文件");
        g.button_opts("编辑", |n| n.props.disabled = true);
        g.button("视图");
    });
    app.build()
}

fn geo_of(tree: &Node) -> Geometry {
    layout(
        tree,
        Rect::new(0.0, 0.0, CANVAS.0, CANVAS.1),
        STYLE,
        &ApproxMeasure,
    )
}

fn tap_at(state: &mut UiState, tree: &Node, geo: &Geometry, id: &str) -> Vec<UiEvent> {
    let f = geo.get(id).unwrap_or_else(|| panic!("测试前置：{id} 必须有几何"));
    let (x, y) = (f.x + f.w / 2.0, f.y + f.h / 2.0);
    let mut out = Vec::new();
    out.extend(handle(
        state, tree, geo, ClipSnapshot::unclipped(),
        &InputEvent::PointerMoved { x, y },
    ));
    out.extend(handle(
        state, tree, geo, ClipSnapshot::unclipped(),
        &InputEvent::PointerDown { button: PointerButton::Left, x, y },
    ));
    out.extend(handle(
        state, tree, geo, ClipSnapshot::unclipped(),
        &InputEvent::PointerUp { button: PointerButton::Left, x, y },
    ));
    out
}

fn key(k: Key) -> InputEvent {
    InputEvent::KeyDown { key: k, mods: Mods::default(), repeat: false }
}

/// 收集事件里的某个变体（断言辅助，避免一长串 matches!）。
fn find(events: &[UiEvent], f: impl Fn(&UiEvent) -> bool) -> Option<&UiEvent> {
    events.iter().find(|e| f(e))
}

// ---------------------------------------------------------------------------
// 一、Segmented：互斥单选（r14a）
// ---------------------------------------------------------------------------

#[test]
fn segmented_selection_semantics() {
    let tree = select_app();
    let geo = geo_of(&tree);
    let mut s = UiState::default();

    // ① 点「周」(button_3)：Clicked + SelectionChanged（新段），值表落位。
    let ev = tap_at(&mut s, &tree, &geo, "button_3");
    assert!(
        ev.contains(&UiEvent::Clicked("button_3".into())),
        "点击段必发它自己的 Clicked：{ev:?}"
    );
    assert_eq!(
        find(&ev, |e| matches!(e, UiEvent::SelectionChanged { .. })),
        Some(&UiEvent::SelectionChanged { id: "mode".into(), selected: "button_3".into() }),
        "换选中必发 SelectionChanged：{ev:?}"
    );
    assert_eq!(s.segments.get("mode").map(String::as_str), Some("button_3"), "值表必须落位");

    // ② 再点「周」：没有变化 ⇒ 只发 Clicked，不发 SelectionChanged（r14a）。
    let ev = tap_at(&mut s, &tree, &geo, "button_3");
    assert!(ev.contains(&UiEvent::Clicked("button_3".into())), "前置：点击仍要发 Clicked：{ev:?}");
    assert!(
        !ev.iter().any(|e| matches!(e, UiEvent::SelectionChanged { .. })),
        "点已选中段不得重发 SelectionChanged：{ev:?}"
    );

    // ③ 点「月」(button_4)：换人 ⇒ 事件 + 值表跟走。
    let ev = tap_at(&mut s, &tree, &geo, "button_4");
    assert_eq!(
        find(&ev, |e| matches!(e, UiEvent::SelectionChanged { .. })),
        Some(&UiEvent::SelectionChanged { id: "mode".into(), selected: "button_4".into() }),
    );
    assert_eq!(s.segments.get("mode").map(String::as_str), Some("button_4"));

    // ④ 键盘同路：`Tab` 只挪焦点（不发选择事件），`Enter` 激活 = 点击 + 换选中。
    let ev = tap_at(&mut s, &tree, &geo, "button_2"); // 焦点 + 选中都到「日」
    assert!(ev.iter().any(|e| matches!(e, UiEvent::FocusChanged(_))), "前置：点击要聚焦");
    let tab = handle(&mut s, &tree, &geo, ClipSnapshot::unclipped(), &key(Key::Tab));
    assert!(
        tab.iter().any(|e| matches!(e, UiEvent::FocusChanged(Some(id)) if id == "button_3")),
        "Tab 移到下一个段（只挪焦点）：{tab:?}"
    );
    assert!(
        !tab.iter().any(|e| matches!(e, UiEvent::SelectionChanged { .. })),
        "Tab 不得改变选中：{tab:?}"
    );
    assert_eq!(s.segments.get("mode").map(String::as_str), Some("button_2"), "前置：选中还在「日」");
    let enter = handle(&mut s, &tree, &geo, ClipSnapshot::unclipped(), &key(Key::Enter));
    assert!(enter.contains(&UiEvent::Clicked("button_3".into())), "Enter = 点击：{enter:?}");
    assert_eq!(
        find(&enter, |e| matches!(e, UiEvent::SelectionChanged { .. })),
        Some(&UiEvent::SelectionChanged { id: "mode".into(), selected: "button_3".into() }),
        "Enter 换选中必须与指针同一条结算路径（r14）：{enter:?}"
    );
}

// ---------------------------------------------------------------------------
// 二、ChipGroup：独立开关（r14b）
// ---------------------------------------------------------------------------

#[test]
fn chip_group_toggle_semantics() {
    let tree = select_app();
    let geo = geo_of(&tree);
    let mut s = UiState::default();

    // ① 点「红」(button_5)：开（on=true 是**翻转后**的值）。
    let ev = tap_at(&mut s, &tree, &geo, "button_5");
    assert_eq!(
        find(&ev, |e| matches!(e, UiEvent::ChipToggled { .. })),
        Some(&UiEvent::ChipToggled { id: "tags".into(), chip: "button_5".into(), on: true }),
        "首点芯片必发 ChipToggled{{on:true}}：{ev:?}"
    );
    assert_eq!(s.chips.get("button_5"), Some(&true), "值表必须落位");

    // ② 再点同一个：关（**每次点击都发** —— 翻转本身就是动作，与单选的「变了才发」不同）。
    let ev = tap_at(&mut s, &tree, &geo, "button_5");
    assert_eq!(
        find(&ev, |e| matches!(e, UiEvent::ChipToggled { .. })),
        Some(&UiEvent::ChipToggled { id: "tags".into(), chip: "button_5".into(), on: false }),
        "再点必发 ChipToggled{{on:false}}（翻转语义）：{ev:?}"
    );
    assert_eq!(s.chips.get("button_5"), Some(&false));

    // ③ 互相独立：点「蓝」(button_6) 不影响「红」。
    let ev = tap_at(&mut s, &tree, &geo, "button_6");
    assert_eq!(
        find(&ev, |e| matches!(e, UiEvent::ChipToggled { .. })),
        Some(&UiEvent::ChipToggled { id: "tags".into(), chip: "button_6".into(), on: true }),
    );
    assert_eq!(s.chips.get("button_5"), Some(&false), "芯片互相独立");
    assert_eq!(s.chips.get("button_6"), Some(&true));

    // ④ 初值是 App 的数据：先塞「绿=开」，点它 ⇒ 翻转成 off（on=false）。
    let mut s2 = UiState::default();
    s2.chips.insert("button_7".into(), true);
    let ev = tap_at(&mut s2, &tree, &geo, "button_7");
    assert_eq!(
        find(&ev, |e| matches!(e, UiEvent::ChipToggled { .. })),
        Some(&UiEvent::ChipToggled { id: "tags".into(), chip: "button_7".into(), on: false }),
        "初值 on 的芯片点击后翻成 off：{ev:?}"
    );

    // ⑤ 键盘同路：焦点在芯片上按 `Enter` = 一次翻转。（注意 tap 本身就翻转 ——
    //    ③里点开的「蓝」这次 tap 后回到关，Enter 再翻回开。）
    tap_at(&mut s, &tree, &geo, "button_6"); // 开 → 关（tap 即翻转）
    assert_eq!(s.chips.get("button_6"), Some(&false), "前置：tap 已把「蓝」翻回关");
    let enter = handle(&mut s, &tree, &geo, ClipSnapshot::unclipped(), &key(Key::Enter));
    assert_eq!(
        find(&enter, |e| matches!(e, UiEvent::ChipToggled { .. })),
        Some(&UiEvent::ChipToggled { id: "tags".into(), chip: "button_6".into(), on: true }),
        "Enter 翻转必须与指针同一条路径（r14）：{enter:?}"
    );
}

// ---------------------------------------------------------------------------
// 三、TabBar：页签 + 禁用页（r14c）
// ---------------------------------------------------------------------------

#[test]
fn tab_bar_semantics_and_disabled_tabs() {
    let tree = select_app();
    let geo = geo_of(&tree);
    // 前置：手写夹具里「编辑」页(button_9)必须真的是禁用的。
    let tab_bar = tree.children.iter().find(|c| c.id == "tabs").expect("前置：tabs 组存在");
    assert!(tab_bar.children[1].props.disabled, "前置：第二页（button_9）必须禁用");

    let mut s = UiState::default();

    // ① 点「文件」(button_8)：TabChanged{index:0}。
    let ev = tap_at(&mut s, &tree, &geo, "button_8");
    assert!(ev.contains(&UiEvent::Clicked("button_8".into())), "前置：点击要发 Clicked：{ev:?}");
    assert_eq!(
        find(&ev, |e| matches!(e, UiEvent::TabChanged { .. })),
        Some(&UiEvent::TabChanged { id: "tabs".into(), index: 0 }),
        "换页必发 TabChanged：{ev:?}"
    );
    assert_eq!(s.tabs.get("tabs").map(String::as_str), Some("button_8"));

    // ② 禁用页（编辑，树序 1）：点不到 ⇒ 无 Clicked、无 TabChanged、焦点不换。
    let ev = tap_at(&mut s, &tree, &geo, "button_9");
    assert!(
        ev.iter().all(|e| !matches!(e, UiEvent::Clicked(_) | UiEvent::TabChanged { .. })),
        "禁用页必须静默：{ev:?}"
    );
    assert_eq!(s.tabs.get("tabs").map(String::as_str), Some("button_8"), "活动页不得被禁用页抢走");

    // ③ 点「视图」(button_10，树序 2)：index = **2** —— 禁用页也一起数
    //    （App 自己的页签数组含禁用项，按同一个下标对齐）。
    let ev = tap_at(&mut s, &tree, &geo, "button_10");
    assert_eq!(
        find(&ev, |e| matches!(e, UiEvent::TabChanged { .. })),
        Some(&UiEvent::TabChanged { id: "tabs".into(), index: 2 }),
        "index 必须按「含禁用页」的树序数：{ev:?}"
    );

    // ④ 点当前活动页：只发 Clicked。
    let ev = tap_at(&mut s, &tree, &geo, "button_10");
    assert!(ev.contains(&UiEvent::Clicked("button_10".into())), "前置：点击仍要发 Clicked");
    assert!(
        !ev.iter().any(|e| matches!(e, UiEvent::TabChanged { .. })),
        "点当前页不得重发 TabChanged：{ev:?}"
    );

    // ⑤ 键盘同路：Tab/Shift+Tab 挪焦点（禁用页**不在**焦点序里），Enter 换页。
    //    此时活动页 = 视图(button_10)、焦点也在它上面 ⇒ `Shift+Tab` 把焦点挪回
    //    「文件」(button_8) 而**不换页**；Enter 再换页（换页动作只属于点击/激活）。
    tap_at(&mut s, &tree, &geo, "button_10");
    let tab = handle(
        &mut s, &tree, &geo, ClipSnapshot::unclipped(),
        &InputEvent::KeyDown { key: Key::Tab, mods: Mods { shift: true, ..Default::default() }, repeat: false },
    );
    assert!(
        tab.iter().any(|e| matches!(e, UiEvent::FocusChanged(Some(id)) if id == "button_8")),
        "Shift+Tab 必须把焦点挪回「文件」（跳过禁用页）：{tab:?}"
    );
    assert_eq!(
        s.tabs.get("tabs").map(String::as_str),
        Some("button_10"),
        "前置：挪焦点不得换页"
    );
    let enter = handle(&mut s, &tree, &geo, ClipSnapshot::unclipped(), &key(Key::Enter));
    assert!(enter.contains(&UiEvent::Clicked("button_8".into())), "前置：Enter = 点击：{enter:?}");
    assert_eq!(
        find(&enter, |e| matches!(e, UiEvent::TabChanged { .. })),
        Some(&UiEvent::TabChanged { id: "tabs".into(), index: 0 }),
        "Enter 在「文件」上 = 换回第 0 页（与指针同路）：{enter:?}"
    );
}

// ---------------------------------------------------------------------------
// 四、Keep：三张值表是应用数据（重建保留）
// ---------------------------------------------------------------------------

#[test]
fn keep_rebuild_preserves_selection_values() {
    let tree = select_app();
    let geo = geo_of(&tree);
    let mut s = UiState::default();
    tap_at(&mut s, &tree, &geo, "button_3"); // mode → 周
    tap_at(&mut s, &tree, &geo, "button_5"); // 红 → 开
    tap_at(&mut s, &tree, &geo, "button_10"); // tabs → 视图
    assert_eq!(s.segments.get("mode").map(String::as_str), Some("button_3"), "前置：mode=周");
    assert_eq!(s.chips.get("button_5"), Some(&true), "前置：红=开");
    assert_eq!(s.tabs.get("tabs").map(String::as_str), Some("button_10"), "前置：tabs=视图");

    // 整树重建（App 惯例：内容变了整树重建；UiState 原样带过去）。
    let rebuilt = select_app();
    assert!(rebuilt.structurally_eq(&tree), "前置：重建必须与原树结构相等（id 确定）");
    let geo2 = geo_of(&rebuilt);

    // 保留：三张表逐一比对。
    assert_eq!(s.segments.get("mode").map(String::as_str), Some("button_3"), "segments 保留");
    assert_eq!(s.chips.get("button_5"), Some(&true), "chips 保留");
    assert_eq!(s.tabs.get("tabs").map(String::as_str), Some("button_10"), "tabs 保留");

    // 仍可用：继续点别的段/芯片，事件照常、值表跟走。
    let ev = tap_at(&mut s, &rebuilt, &geo2, "button_4");
    assert_eq!(
        find(&ev, |e| matches!(e, UiEvent::SelectionChanged { .. })),
        Some(&UiEvent::SelectionChanged { id: "mode".into(), selected: "button_4".into() }),
        "重建后选择仍可用"
    );
    let ev = tap_at(&mut s, &rebuilt, &geo2, "button_5");
    assert_eq!(
        find(&ev, |e| matches!(e, UiEvent::ChipToggled { .. })),
        Some(&UiEvent::ChipToggled { id: "tags".into(), chip: "button_5".into(), on: false }),
        "重建后开关仍可用（且记得它开着 ⇒ 翻成关）"
    );
}

// ---------------------------------------------------------------------------
// 五、视觉判据：muted 只落在组内成员；组外按钮与无状态渲染器不受影响
// ---------------------------------------------------------------------------

/// 收集一份绘制列表里「矩形 == 给定节点矩形」的填充命令**颜色**（按出现序）。
/// 按钮的 `FillRoundRect` 与节点矩形完全重合 ⇒ 这就是它的底色。
fn fill_colors_over(list: &DrawList, id_rect: RectI) -> Vec<Color> {
    list.cmds
        .iter()
        .filter_map(|c| match c {
            DrawCmd::FillRoundRect { rect, color, .. } if *rect == id_rect => Some(*color),
            _ => None,
        })
        .collect()
}

fn rect_of(geo: &Geometry, id: &str) -> RectI {
    let f = geo.get(id).unwrap_or_else(|| panic!("测试前置：{id} 必须有几何"));
    RectI::new(f.x as i32, f.y as i32, f.w as i32, f.h as i32)
}

#[test]
fn selection_visual_only_tints_group_members() {
    let tree = select_app();
    let geo = geo_of(&tree);
    let theme = Theme::default();

    // 前置：三个段的矩形互不相同（否则「按矩形找命令」找不到人）。
    let r2 = rect_of(&geo, "button_2");
    let r3 = rect_of(&geo, "button_3");
    let r4 = rect_of(&geo, "button_4");
    assert_ne!(r2, r3, "前置：段的矩形必须互异");
    assert_ne!(r3, r4, "前置：段的矩形必须互异");

    let built = |st: &UiState| {
        let list = deer_gui::gpu::interact::InteractiveRenderer::new(
            theme.clone(),
            &ApproxMeasure,
            &st.to_interact_state(),
        )
        .build(&tree, &geo);
        (
            list.cmds.len(),
            fill_colors_over(&list, r2),
            fill_colors_over(&list, r3),
            fill_colors_over(&list, r4),
        )
    };

    // ① 空表（没有任何选中）：三个段全部是「未选中 muted」档（组内默认让位）。
    let (n0, c2_0, c3_0, c4_0) = built(&UiState::default());
    assert!(!c3_0.is_empty(), "前置：每个段至少要有一条填充命令（否则在测空气）");
    // ② 选中「周」：button_3 的颜色必须变（muted → 选中 accent 档），button_2/4 不变。
    let mut st = UiState::default();
    st.segments.insert("mode".into(), "button_3".into());
    let (n1, c2_1, c3_1, c4_1) = built(&st);
    assert_ne!(c3_0, c3_1, "选中段的颜色必须变（muted → 选中）");
    assert_eq!(c2_0, c2_1, "未选中段的颜色不得因换人选而变");
    assert_eq!(c4_0, c4_1, "未选中段的颜色不得因换人选而变");
    // ③ 换选「月」：button_3（选中→muted）与 button_4（muted→选中）必须变，button_2 不变。
    let mut st2 = UiState::default();
    st2.segments.insert("mode".into(), "button_4".into());
    let (n2, c2_2, c3_2, c4_2) = built(&st2);
    assert_eq!(c2_1, c2_2, "button_2 在两个状态下都是未选中 ⇒ 颜色逐字节相同");
    assert_ne!(c3_1, c3_2, "button_3 从选中变未选中 ⇒ 颜色必须变");
    assert_ne!(c4_1, c4_2, "button_4 从未选中变选中 ⇒ 颜色必须变");
    assert_eq!(n0, n1, "换选不得增删命令条数（只许换颜色）");
    assert_eq!(n1, n2, "换选不得增删命令条数（只许换颜色）");
}

#[test]
fn plain_buttons_and_default_renderer_are_blind_to_selection_maps() {
    let tree = select_app();
    let geo = geo_of(&tree);
    let theme = Theme::default();

    // ① **红线判据（组外按钮）**：普通按钮「顶」(button_1) 不在任何组里，
    //    选择映射里就算塞了它的 id，也**一个字节都不能变**。
    let plain_id = tree.children[0].id.clone();
    assert_eq!(plain_id, "button_1", "前置：第一个孩子应是普通按钮「顶」");
    let plain_rect = rect_of(&geo, &plain_id);
    let build_at = |st: &UiState| {
        deer_gui::gpu::interact::InteractiveRenderer::new(
            theme.clone(),
            &ApproxMeasure,
            &st.to_interact_state(),
        )
        .build(&tree, &geo)
    };
    let mut st = UiState::default();
    // 刁钻用例：把普通按钮的 id 同时塞进三张表（App 手滑时的最坏情形）。
    st.segments.insert("app".into(), plain_id.clone());
    st.chips.insert(plain_id.clone(), true);
    st.tabs.insert("app".into(), plain_id.clone());
    let l_empty = build_at(&UiState::default());
    let l_stray = build_at(&st);
    assert_eq!(
        fill_colors_over(&l_empty, plain_rect),
        fill_colors_over(&l_stray, plain_rect),
        "组外按钮必须对选择映射免疫（opt-in 红线）"
    );
    assert_eq!(l_empty, l_stray, "整帧绘制列表必须逐条相同（映射只对组成员生效）");

    // ② **DefaultRenderer 无状态**：它根本不接选择状态 ⇒ 组内成员一律画普通按钮底
    //    （accent），与 Field 只画占位标签同理。选中视觉是 InteractiveRenderer 的活。
    let d = deer_gui::gpu::render::DefaultRenderer::new(theme, &ApproxMeasure).build(&tree, &geo);
    for id in ["button_2", "button_3", "button_4"] {
        let colors = fill_colors_over(&d, rect_of(&geo, id));
        assert!(
            colors.iter().all(|c| *c == Theme::default().accent),
            "DefaultRenderer 的段 `{id}` 必须画普通按钮底（accent），实际 {colors:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// 六、页签组：便捷构造与手写等价（tab_bar 不带禁用参数的替代路径）
// ---------------------------------------------------------------------------

#[test]
fn tab_bar_hand_written_equivalence() {
    let mut a = Builder::new(Kind::Column, "app");
    let tabs = a.tab_bar_opts("tabs", L::new().gap(2.0).to_props(), &["文件", "编辑", "视图"]);
    let ta = a.build();

    let mut b = Builder::new(Kind::Column, "app");
    b.container_opts(Kind::TabBar, "tabs", L::new().gap(2.0).to_props(), |g| {
        g.button("文件");
        g.button("编辑");
        g.button("视图");
    });
    let tb = b.build();

    assert!(ta.structurally_eq(&tb), "tab_bar 便捷构造必须与手写组结构相等");
    assert_eq!(
        tabs,
        tb.children[0].children.iter().map(|c| c.id.clone()).collect::<Vec<_>>(),
        "返回的页签 id 必须与树里的孩子逐一对上"
    );
}

// ---------------------------------------------------------------------------
// 七、testkit 档：注入路径 + T3.1 方向键跨控件导航
// ---------------------------------------------------------------------------

#[cfg(feature = "testing")]
mod via_testkit {
    use deer_gui::interaction::{InputEvent, Key, Mods, UiEvent, UiState};
    use deer_gui::prelude::*;
    use deer_gui::testing::{Harness, Repro};

    use super::select_app;

    /// testkit 注入路径的全量语义（与纯逻辑档同源，这里钉「Harness 注入也能到达」），
    /// 外加 **T3.1 方向键**在「段 → 芯片 → 页签 → 普通按钮」之间的跨控件导航。
    ///
    /// 几何（`ApproxMeasure`：单字标签按钮 29×22、两字 37×22；行距 gap 6）：
    /// 行中心的 x 为 日/红/文件≈22.5、周/蓝≈53.5、月/绿≈84.5（tabs 行 26.5/65.5/105.5）。
    /// 方向键取「严格同方向 + 垂距最近 + 水平更近」，下面的每一步都按这套数推出并留有余量。
    #[test]
    fn selection_semantics_and_arrow_navigation_via_testkit() -> Result<(), String> {
        let tree = select_app();
        let mut h = Harness::new(tree, 260, 200, Theme::default());
        h.set_case("m6_select");
        h.set_repro(Repro::test("deer-gui", "testing", "m6_select", "selection_semantics", &[]));
        h.require_ids(&[
            "button_1", "button_2", "button_3", "button_4", "button_5", "button_6", "button_8",
            "button_10",
        ]);

        // 前置：禁用页不在焦点序里；其余全在。
        h.assert_focus_order_excludes(&["button_9"])?;

        // ① 指针点「周」：Clicked + SelectionChanged + 聚焦。
        let step = h.tap("button_3")?;
        assert!(
            step.events.contains(&UiEvent::Clicked("button_3".into()))
                && step.events.contains(&UiEvent::SelectionChanged {
                    id: "mode".into(),
                    selected: "button_3".into()
                }),
            "点段要发 Clicked + SelectionChanged：{:?}",
            step.events
        );
        h.assert_focus(Some("button_3"))?;

        // ② 禁用页静默（焦点抢不走）。
        let step = h.tap("button_9")?;
        assert!(
            step.events.iter().all(|e| !matches!(e, UiEvent::Clicked(_) | UiEvent::TabChanged { .. })),
            "禁用页必须静默：{:?}",
            step.events
        );
        h.assert_focus(Some("button_3"))?;

        // ③ 指针点「红」：翻转语义。**点击同时聚焦它** ⇒ 此刻焦点已在芯片行。
        let step = h.tap("button_5")?;
        assert!(
            step.events.contains(&UiEvent::ChipToggled {
                id: "tags".into(),
                chip: "button_5".into(),
                on: true
            }),
            "点芯片要发 ChipToggled{{on:true}}：{:?}",
            step.events
        );
        h.assert_focus(Some("button_5"))?;

        // ④ **T3.1 方向键跨控件导航**（几何邻近；焦点序按树序参与）：
        //    焦点在「红」(x≈22.5, y≈75)，正下方是页签行 —— 「文件」(x≈26.5, dx=4)
        //    比「视图」(x≈105.5, dx=83) 近得多（禁用页根本不在候选里）。
        let down = |h: &mut Harness| {
            h.send(&InputEvent::KeyDown { key: Key::Down, mods: Mods::default(), repeat: false })
        };
        let up = |h: &mut Harness| {
            h.send(&InputEvent::KeyDown { key: Key::Up, mods: Mods::default(), repeat: false })
        };
        let step = down(&mut h)?;
        assert!(
            step.events.iter().any(|e| matches!(e, UiEvent::FocusChanged(Some(id)) if id == "button_8")),
            "Down 必须从芯片落到页签行的「文件」（几何最近）：{:?}",
            step.events
        );
        // ⑤ 键盘激活换页（与指针同一条结算路径）。
        let step = h.send(&InputEvent::KeyDown { key: Key::Enter, mods: Mods::default(), repeat: false })?;
        assert!(
            step.events.contains(&UiEvent::Clicked("button_8".into()))
                && step.events.contains(&UiEvent::TabChanged { id: "tabs".into(), index: 0 }),
            "Enter 在「文件」上 = 点击 + 换到第 0 页：{:?}",
            step.events
        );
        // ⑥ Up 逐行回走：文件(x≈26.5) → 红(x≈22.5, dx=4) → 日(x≈22.5, dx=0) → 顶 ——
        //    芯片 → 页签 → 芯片 → 段 → 普通按钮，全链在同一个导航图里。
        let step = up(&mut h)?;
        assert!(
            step.events.iter().any(|e| matches!(e, UiEvent::FocusChanged(Some(id)) if id == "button_5")),
            "Up 必须回到芯片行的「红」：{:?}",
            step.events
        );
        let step = up(&mut h)?;
        assert!(
            step.events.iter().any(|e| matches!(e, UiEvent::FocusChanged(Some(id)) if id == "button_2")),
            "Up 必须到段行的「日」：{:?}",
            step.events
        );
        let step = up(&mut h)?;
        assert!(
            step.events.iter().any(|e| matches!(e, UiEvent::FocusChanged(Some(id)) if id == "button_1")),
            "Up 必须到普通按钮「顶」——三种组与普通按钮在同一条导航链上：{:?}",
            step.events
        );
        let step = down(&mut h)?;
        assert!(
            step.events.iter().any(|e| matches!(e, UiEvent::FocusChanged(Some(id)) if id == "button_2")),
            "Down 从普通按钮回到段行（同一条链可逆）：{:?}",
            step.events
        );
        Ok(())
    }

    /// 选中视觉经 Harness 帧（`build_with` 已带三张值表）：换选只允许改动
    /// 「新旧两个选中段」的矩形，其余像素逐字节相同。
    #[test]
    fn selection_change_is_visually_confined_to_the_two_picks() -> Result<(), String> {
        let tree = select_app();
        let mut h = Harness::new(tree, 260, 200, Theme::default());
        h.set_case("m6_select_visual");
        h.require_ids(&["button_2", "button_3", "button_4"]);

        // 前置：先选中「周」（值直接塞进状态 —— 值是 App 的数据）。
        let mut st = UiState::default();
        st.segments.insert("mode".into(), "button_3".into());
        h.set_state(st);
        let before = h.shoot()?;

        // 换选「月」：状态里只有 mode 变了。
        let mut st2 = UiState::default();
        st2.segments.insert("mode".into(), "button_4".into());
        h.set_state(st2);
        let after = h.shoot()?;

        // 判据：差异只许落在 button_3 / button_4 两个矩形里（其余逐字节相同）。
        let f = h.frame()?;
        let r3 = f.rect_of("button_3").expect("前置：button_3 有几何");
        let r4 = f.rect_of("button_4").expect("前置：button_4 有几何");
        after.assert_no_diff_outside(&before, &[r3, r4])?;
        Ok(())
    }
}
