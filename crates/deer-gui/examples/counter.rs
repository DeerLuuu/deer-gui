//! **counter —— 最小但真的有功能的交互界面**（配套教程：`docs/GETTING-STARTED-UI.md`）。
//!
//! 界面：标题文本 + 计数显示 + `+` / `-` 两个按钮 + 一个文本输入框
//! （`Text` / `Button` / `Field` 三种控件都用上了）。
//!
//! ```sh
//! # ① 真实窗口、**纯交互**（默认）：自己点 + / -、点输入框打字、Tab 换焦点、Esc 退出
//! cargo run -q -p deer-gui --features window --example counter
//!
//! # ② 确定性重放（有期望终态，跑完自己退；退出码 0 = 断言全过）—— 用**环境变量**，别用命令行参数
//! cmd /c "set DEER_COUNTER_SCRIPT=@builtin&& cargo run -q -p deer-gui --features window --example counter"
//! cmd /c "set DEER_COUNTER_SCRIPT=move @plus;down:left;up:left;move @plus;down:left;up:left&& cargo run -q -p deer-gui --features window --example counter"
//!
//! # ③ **离屏自检**（不需要窗口；默认跑内置脚本 + 真字形 + 像素断言）
//! cargo run -q -p deer-gui --features window --example counter -- --headless
//! ```
//!
//! > ### ⚠️ 为什么「默认必须是纯交互」（**第一版搞错了，被使用者一眼看穿**）
//! >
//! > 第一版把「没给脚本」实现成「自动重放内置脚本」，然后交给 `Waker::wake_after` 的 deadline
//! > 去推进。结果是：窗口在**人**看来**开着不动**，而**真实鼠标事件**成了那个 deadline 的
//! > 替代品 —— 使用者的原话是「我鼠标不进入，窗口就一直在；鼠标进入/退出，它才动一下然后退出」。
//! > 一个示例的默认行为不该让人猜它到底在等谁。所以现在：
//! >
//! > - **默认 = 纯交互**：不重放任何事件，窗口一直开着直到 `Esc` / 关窗（这才是「自己点着玩」）；
//! > - **要重放就显式说**：`DEER_COUNTER_SCRIPT=@builtin`（内置脚本 + 终态断言，跑完自己退）。
//! >
//! > 这条也是**给人看的示例**必须遵守的纪律：默认档的行为要**一眼可预期**，
//! > 不能靠「反正它会自己退出」把不确定性藏起来。
//! >
//! > **为什么脚本建议走环境变量**（实测，不是猜的）：`--script "move @plus;down:left;…"`
//! > 这种带空格/分号/`@` 的长参数在不同 shell 里会被吃掉内容 —— 第一次实测时
//! > PowerShell 把脚本截成了 `move;`，**解析器没报错**（它是一段合法脚本），于是「重放通过」
//! > 变成一句空话。（`--script` 仍然可用；示例会在拿到它时把**实际收到的字符串**打出来。）
//! >
//! > `set` 的两种写法差别（也是实测）：`cmd /c "set X=1 && …"` 的值是 `"1 "`（**带尾空格**），
//! > `set X=1&& …` 才不带。本示例的脚本判定先 `trim()` 再比，两种写法都认；
//! > 但你自己写的判定别忘了这一条（教程第 8 节）。
//!
//! # 这个示例在证明什么
//!
//! 1. **「输入 → 命中 → 状态 → 画出来的文本」是一条闭环**：点 `+` 改的不是某个内部计数，
//!    而是**下一帧真正画出来的那串字**（`count` 这个 `Text` 节点的 `props.label`）；
//!    所以「点两次 `+` ⇒ 计数显示为 2」是**可自动判定**的（[`assert_count_shown`] +
//!    `--headless` 那条离屏自检，两处都断言）。
//! 2. **只在状态真的变了时才重绘**：`dirty` 由「这一条输入到底改没改东西」决定，
//!    不看「有没有收到事件」（比的是状态，见 [`Counter::feed`]）。
//! 3. **脚本重放与人手点击走同一个入口**（[`Counter::handle_input`]）：状态机、命中、
//!    置脏只有一份实现 ⇒「脚本跑得通」与「人手点得动」不可能漂。
//!
//! 与 `interactive_form.rs` 的关系：**同一个骨架**（连「`OnDemand` + `Waker::wake_after`
//! 自驱重放」这条也留着），但砍掉了它的四档像素数字、安静窗口、`DEER_FORM_ONDEMAND` 分档，
//! 只留「建树 → 接输入 → 重绘 → 断言」这条主线，便于逐段阅读。

use std::collections::VecDeque;
use std::path::Path;
use std::process::ExitCode;

use deer_gui::gpu::interact::{FieldText, InteractState, InteractiveRenderer};
use deer_gui::gpu::measure::find_system_font;
use deer_gui::gpu::null::CpuRenderer;
use deer_gui::gpu::{Color, DrawCmd, DrawList, Extent, RectI, TextEngine, Theme};
use deer_gui::input_script::ENV_VAR;
use deer_gui::interaction::{self, ClipSnapshot, InputEvent, Key, UiEvent, UiState};
use deer_gui::layout::builder::L;
use deer_gui::layout::layout::{self, Geometry, TextStyle};
use deer_gui::layout::node::{Kind, LayoutProps, Node, Rect};
use deer_gui::prelude::*;
use deer_gui::vk::windowed::{FrameOutcome, WindowedRenderer};
use deer_gui::window::{App, Flow, RedrawPolicy, WindowConfig, WindowInfo, Waker, run};

// ---------------------------------------------------------------------------
// 一、常量（教程第 2 节）
// ---------------------------------------------------------------------------

/// 清屏色：界面没盖住的地方就是这个颜色（离屏与上屏用同一个值）。
const CLEAR: Color = Color::rgb(0x08, 0x09, 0x0c);

/// 窗口尺寸（**物理像素**；脚本里的坐标与它同一套口径）。
const WIDTH: u32 = 360;
const HEIGHT: u32 = 200;

/// 拿不到系统字体时退回的字号 —— 让「布局度量」与「画出来的字号」用**同一个**数字
/// （字号三处不一致，布局算出来的宽度与画出来的宽度就会漂）。
const FALLBACK_FONT_SIZE: f32 = 16.0;

/// 计数显示的前缀：期望值就是 `format!("{COUNT_PREFIX}{n}")`。
const COUNT_PREFIX: &str = "count = ";

/// 本示例自己的脚本环境变量（优先于库里的 [`ENV_VAR`]）：`set "DEER_COUNTER_SCRIPT=…"`。
const COUNTER_SCRIPT_ENV: &str = "DEER_COUNTER_SCRIPT";

/// 内置脚本（没给脚本时用它）—— 它**有期望终态**，所以它是断言的一部分。
///
/// `move @plus` 读作「指针移到 `plus` 节点中心」（坐标由**布局**算出来，不写死）。
const BUILTIN_SCRIPT: &str =
    "move @plus;down:left;up:left;move @plus;down:left;up:left;move @input;down:left;up:left;text:ok";

/// 内置脚本跑完之后的期望终态（`+` 两次 ⇒ 2；`text:ok` ⇒ `ok`）。
const BUILTIN_EXPECTED_COUNT: i32 = 2;
const BUILTIN_EXPECTED_INPUT: &str = "ok";

// ---------------------------------------------------------------------------
// 二、界面树（教程第 3 节：`Builder` / `Node` / `L` / 尺寸与对齐）
// ---------------------------------------------------------------------------

/// 界面树。`count` 进树 ⇒ **计数变化必然体现在这一帧的画面上**。
///
/// 这里用 [`Node`] 的链式构造（而不是 `Builder` 的 `app.text(...)`）是因为计数显示需要
/// **显式 id**：`Builder::text()` 的 id 由 `IdGen` 自动生成（形如 `text_1`），而它的内容
/// 每帧都在变 —— id 不稳定，脚本里的 `move @count` 就会点空。显式 id 也让断言能写死
/// （`assert_counter_tree` 逐条断言两个结构不变式）。
///
/// 两个 `Row` 都给了**固定宽度** `300`：否则根 Column 的宽度会随「计数显示的字符数」变化
/// ⇒ 按钮坐标跟着漂 ⇒ 脚本 `move @plus` 会在计数从 `0` 变成 `12` 之后**静默点空**。
///
/// 四个叶子控件（标题/计数/两个按钮）**都没有子节点**：渲染器画叶子用的是它自己的
/// `props.label`（`interact.rs` 的 `Kind::Text` / `Kind::Button` 分支），所以
/// 「画出来的字 == 节点标签」这条对应关系是**一对一**的，断言才能咬死。
fn counter_tree(count: i32) -> Node {
    let app = Node::new(Kind::Column, "app").with_layout(LayoutProps {
        padding: 12.0,
        gap: 10.0,
        ..Default::default()
    });
    let bar = Node::new(Kind::Row, "bar")
        .with_layout(L::new().w(300.0).gap(8.0).to_props())
        .push(Node::new(Kind::Button, "plus").with_label("+"))
        .push(Node::new(Kind::Button, "minus").with_label("-"));
    let info = Node::new(Kind::Row, "info")
        .with_layout(L::new().w(300.0).gap(8.0).to_props())
        .push(Node::new(Kind::Text, "count").with_label(format!("{COUNT_PREFIX}{count}")))
        .push(Node::new(Kind::Field, "input").with_label("type here"));
    app.push(Node::new(Kind::Text, "title").with_label("deer-gui counter"))
        .push(bar)
        .push(info)
}

/// 按 id 找节点（前序，确定性）。**这不是第二套命中测试** —— 只用来读/改树内容。
fn node_by_id<'a>(n: &'a Node, id: &str) -> Option<&'a Node> {
    if n.id == id {
        return Some(n);
    }
    n.children.iter().find_map(|c| node_by_id(c, id))
}

/// 前置不变式：树的结构必须与「按 id 读标签」这个假设一致（不成立就该红，不许静默）。
fn assert_counter_tree(tree: &Node) -> Result<(), String> {
    for id in ["title", "count", "plus", "minus", "input"] {
        if node_by_id(tree, id).is_none() {
            return Err(format!("界面树里没有节点 `{id}`（断言与脚本都按 id 定位）"));
        }
    }
    if node_by_id(tree, "count").is_some_and(|n| n.kind != Kind::Text) {
        return Err("`count` 必须是 Text 节点 —— 「计数变了」与「画出来的字变了」要同一件事".into());
    }
    if node_by_id(tree, "input").is_some_and(|n| n.kind != Kind::Field) {
        return Err("`input` 必须是 Field 节点（只有 Field 收文本输入）".into());
    }
    for (id, label, kind) in [
        ("title", "deer-gui counter", Kind::Text),
        ("plus", "+", Kind::Button),
        ("minus", "-", Kind::Button),
    ] {
        let n = node_by_id(tree, id).expect("上面已确认存在");
        if n.kind != kind {
            return Err(format!("`{id}` 必须是 {kind:?} 节点（当前 {:?}）", n.kind));
        }
        if !n.children.is_empty() {
            return Err(format!(
                "`{id}` 必须是**叶子**：渲染器画它用的是自己的 `props.label`，\
                 带子节点会让「第 k 条 NodeHint ⇄ 第 k 个有几何的节点」这个绑定多出一层，\
                 本示例的 `drawn_texts()` 就会把子节点的文本并进父节点（当前 {} 个子节点）",
                n.children.len()
            ));
        }
        if n.props.label.as_deref() != Some(label) {
            return Err(format!(
                "`{id}` 的标签是 {:?}，期望 `{label}` —— 断言与文档都按这个字符串写",
                n.props.label
            ));
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// 三、一帧：树 + 几何 + 状态 → 绘制列表 + 裁剪快照（教程第 2、3 节）
// ---------------------------------------------------------------------------

/// 一帧的四样东西（都是**纯数据**，可以留到断言里用）。
struct Frame {
    tree: Node,
    geo: Geometry,
    list: DrawList,
    clip: ClipSnapshot,
    /// 这一帧的交互状态（像素断言要拿它解释「哪个按钮也变了」）。
    state: UiState,
}

impl Frame {
    /// 几何**每帧重算**（很便宜，而且保证与列表同源）；列表由「树 + 状态」派生。
    ///
    /// `measure` 决定文本宽度 ⇒ 布局与绘制必须用**同一份**度量（窗口路径用
    /// [`FontMeasure`]，离屏自检用 [`ApproxMeasure`]，两者接口相同）。
    fn build(
        tree: &Node,
        theme: &Theme,
        extent: Extent,
        state: &UiState,
        measure: &impl Measure,
    ) -> Frame {
        let geo = layout::layout(
            tree,
            Rect::new(0.0, 0.0, extent.width as f32, extent.height as f32),
            TextStyle {
                font_size: theme.font_size,
                line_height: theme.line_height,
            },
            measure,
        );
        // `UiState` 是交互层的真相；渲染层只认它的**只读子集** `InteractState`。
        let interact = state.to_interact_state();
        // `FieldText::Content` ⇒ 输入框画的是 `state.texts["input"]`（不是占位 label）；
        // `InteractiveRenderer` 会给**每个有几何的节点**发一条 `NodeHint` ⇒ 裁剪快照非空。
        let list = InteractiveRenderer::with_texts(
            theme.clone(),
            measure,
            &interact,
            FieldText::Content,
            &state.texts,
        )
        .build(tree, &geo);
        let clip = ClipSnapshot::from_draw_list(&list, tree, &geo);
        Frame {
            tree: tree.clone(),
            geo,
            list,
            clip,
            state: state.clone(),
        }
    }

    /// 节点 id 的中心点：**脚本的 `move @id` 用它算坐标**（布局一变，脚本跟着变）。
    fn center(&self, id: &str) -> Result<(f32, f32), String> {
        let r = self
            .geo
            .get(id)
            .copied()
            .ok_or_else(|| format!("布局没有给 `{id}` 几何"))?;
        Ok((r.x + r.w / 2.0, r.y + r.h / 2.0))
    }

    /// 节点矩形（整数口径，像素断言用）。
    fn rect_of(&self, id: &str) -> Option<RectI> {
        self.geo
            .get(id)
            .map(|r| RectI::new(r.x as i32, r.y as i32, r.w as i32, r.h as i32))
    }

    /// 画出来的文本（`节点 id → 该节点段里的文本`）。**这是「渲染出来的文本」的直接证据**：
    /// 它来自绘制列表，不是从状态里读回来的。
    ///
    /// 绑定口径与 [`ClipSnapshot::from_draw_list`] **逐字相同**：列表里第 k 条 `NodeHint`
    /// 对应前序里第 k 个「有几何的节点」；某条 `NodeHint` 之后、下一条之前的 `Text` 命令
    /// 就属于它（`InteractiveRenderer` 的产出顺序）。
    fn drawn_texts(&self) -> std::collections::BTreeMap<String, String> {
        let mut out = std::collections::BTreeMap::new();
        let ordered = ordered_with_geometry(&self.tree, &self.geo);
        let mut k = 0usize;
        for cmd in &self.list.cmds {
            match cmd {
                DrawCmd::NodeHint { .. } => {
                    k += 1;
                }
                // `k == 0` = 第一条提示之前（本渲染器不产出这种）；`k > ordered.len()`
                // = 提示比节点多（绑定已错位）⇒ 明确报错，不静默丢。
                DrawCmd::Text { text, .. } => {
                    if k == 0 {
                        return std::collections::BTreeMap::from([(
                            "<NodeHint 之前的文本>".to_string(),
                            text.clone(),
                        )]);
                    }
                    let node = ordered
                        .get(k - 1)
                        .unwrap_or_else(|| panic!("第 {k} 条 NodeHint 没有对应的节点（列表与树不同源）"));
                    out.entry(node.id.clone())
                        .or_insert_with(String::new)
                        .push_str(text);
                }
                _ => {}
            }
        }
        out
    }
}

/// 前序遍历、只收集**有几何**的节点（顺序与 `ClipSnapshot::from_draw_list` 的绑定口径一致）。
fn ordered_with_geometry<'a>(n: &'a Node, geo: &Geometry) -> Vec<&'a Node> {
    let mut out = Vec::new();
    fn walk<'a>(n: &'a Node, geo: &Geometry, out: &mut Vec<&'a Node>) {
        if geo.contains_key(&n.id) {
            out.push(n);
        }
        for c in &n.children {
            walk(c, geo, out);
        }
    }
    walk(n, geo, &mut out);
    out
}

// ---------------------------------------------------------------------------
// 四、断言：计数变化**必须**体现到渲染出来的文本上（教程第 6 节）
// ---------------------------------------------------------------------------

/// **核心判据**：绘制列表里 `count` 这一项画出来的字，必须等于期望的计数文本。
///
/// 为什么读绘制列表而不是读 `tree`：`tree` 是「我们打算画什么」，绘制列表是「实际要画的
/// 命令」——只有后者能证明「计数变化走到了渲染这一步」。想再往下钉一层到**像素**，
/// 见 [`assert_pixels_show_count`]。
fn assert_count_shown(frame: &Frame, expected: i32) -> Result<(), String> {
    let want = format!("{COUNT_PREFIX}{expected}");
    let drawn = frame.drawn_texts();
    let got = drawn.get("count");
    println!(
        "断言 count 显示 : 期望 `{want}`；绘制列表实际 {:?}（整帧文本 {:?}）",
        got, drawn
    );
    match got {
        Some(t) if *t == want => Ok(()),
        Some(t) => Err(format!(
            "计数显示是 `{t}`，期望 `{want}` —— 计数没有被渲染出来（闭环断了）"
        )),
        None => Err(format!(
            "绘制列表里没有 `count` 节点的文本命令（期望 `{want}`）—— \
             检查 `count` 是不是 Text 节点、有没有几何、`NodeHint` 是否齐全"
        )),
    }
}

/// 像素级佐证：把两帧都在 CPU 后端画出来，比对差异**只落在给定的矩形内**。
///
/// 返回 `(每块期望矩形内的差异像素数, 所有期望矩形之外的差异像素数)`。
/// 返回的是**像素数**不是字节数 —— 一个像素 4 个通道，混用会让数字虚高 4 倍。
///
/// ⚠️ **必须把「这一帧同时变了的**所有**东西」都列进 `rects`**：鼠标从 `+` 上按下去，
/// 除了计数文本，按钮自己的 hover/pressed/focus 视觉也会变（本机实测：只列 count 矩形时
/// 框外有 393 个差异像素）—— 那不是「溢出」，是断言漏列了。框外必须为 0 才是真判据。
///
/// ⚠️ 还有一条**前置**：渲染必须用**[`CpuRenderer::with_text`]**（真字形）。
/// 无字库的 [`CpuRenderer::new`] 给**每个字符画同一个等宽方块**（`null.rs::draw_text`），
/// 于是「`count = 0`」与「`count = 1`」的像素**逐个相同**（本机实测：矩形内差异 **0** 像素，
/// 而绘制列表里文本确实变了）—— 拿它做文本断言只会得到一条**假红**。
fn pixel_diff_split(
    a: &Frame,
    b: &Frame,
    extent: Extent,
    rects: &[RectI],
    renderer: &mut CpuRenderer,
) -> Result<(Vec<usize>, usize), String> {
    // 同一个渲染器连续渲染两帧：字形图集只是缓存，不影响像素（`null.rs::render` 的确定性断言）。
    let pa = renderer
        .render(extent, &a.list, CLEAR)
        .map_err(|e| format!("CPU 渲染失败：{e}"))?
        .pixels;
    let pb = renderer
        .render(extent, &b.list, CLEAR)
        .map_err(|e| format!("CPU 渲染失败：{e}"))?
        .pixels;
    if pa.len() != pb.len() {
        return Err(format!("两帧像素长度不一致：{} vs {}", pa.len(), pb.len()));
    }
    let w = extent.width.max(1) as usize;
    let mut inside = vec![0usize; rects.len()];
    let mut outside = 0usize;
    for (idx, (x, y)) in pa.chunks_exact(4).zip(pb.chunks_exact(4)).enumerate() {
        if x == y {
            continue;
        }
        let px = (idx % w) as i32;
        let py = (idx / w) as i32;
        match rects.iter().position(|r| r.contains(px, py)) {
            Some(k) => inside[k] += 1,
            None => outside += 1,
        }
    }
    Ok((inside, outside))
}

/// 这一帧里**真的换了视觉的按钮**（拿两帧的交互状态比对，不猜）。
///
/// 只算 `hover`/`focus`/`pressed` 三个字段：它们正是 [`InteractState`] 的全部输入。
fn buttons_whose_visual_changed(before: &Frame, after: &Frame) -> Vec<String> {
    let st = |f: &Frame| f.state.to_interact_state();
    let (a, b) = (st(before), st(after));
    let ids = ["plus", "minus"];
    let mut out = Vec::new();
    for id in ids {
        let key = |s: &InteractState| {
            (
                s.hover.as_deref() == Some(id),
                s.focus.as_deref() == Some(id),
                s.pressed.as_deref() == Some(id),
            )
        };
        if key(&a) != key(&b) {
            out.push(id.to_string());
        }
    }
    out
}

/// 计数变化的**像素级**判据：
/// ① `count` 矩形内必须有差异（文本真的换了字形）；② 框外必须为 0（没溢出到别的控件）。
fn assert_pixels_show_count(
    before: &Frame,
    after: &Frame,
    extent: Extent,
    label: &str,
    renderer: &mut CpuRenderer,
) -> Result<(), String> {
    let count_rect = after
        .rect_of("count")
        .ok_or("布局没有给 `count` 几何 ⇒ 没法做像素断言")?;
    // 期望矩形 = count 自己 + 这一帧视觉真的变了的按钮（框外为 0 才是判据）。
    let mut rects = vec![count_rect];
    let mut whose = Vec::new();
    for id in buttons_whose_visual_changed(before, after) {
        if let Some(r) = after.rect_of(&id) {
            rects.push(r);
            whose.push(id);
        }
    }
    let (inside, outside) = pixel_diff_split(before, after, extent, &rects, renderer)?;
    println!(
        "像素断言 {label:<12}: 差异 count 矩形内 {} 像素（框 {count_rect:?}）；\
         同时变了的按钮 {whose:?} 内 {:?}；**框外 {outside}** 像素",
        inside[0],
        &inside[1..]
    );
    if inside[0] == 0 {
        return Err(format!(
            "{label}: 计数矩形内一个像素都没变 ⇒ 计数**没有**画出来（期望文本变了）。\
             ⚠️ 若无字库（`CpuRenderer::new()`）每个字符都是同一个方块 ⇒ 这条必然**假红**"
        ));
    }
    if outside != 0 {
        return Err(format!(
            "{label}: 有 {outside} 个像素差异落在期望矩形 {rects:?} **之外** ⇒ 变化视觉溢出了"
        ));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// 五、应用状态与输入路径（教程第 4、5 节）
// ---------------------------------------------------------------------------

/// 一条输入的**来源**：只影响打印与「脚本模式下忽略窗口事件」的判定，**不影响状态机**。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Source {
    Script,
    Window,
}

fn fmt_state(s: &UiState) -> String {
    format!(
        "hover={:?} focus={:?} pressed={:?} texts={:?}",
        s.hover, s.focus, s.pressed, s.texts
    )
}

/// 应用状态：**`UiState` 是交互的真相，`count`/`input` 是界面自己的数据**。
///
/// `dirty` 的判据是「这三样里**任何一个**真的变了」——脚本事件与窗口事件共用它。
struct Counter {
    theme: Theme,
    extent: Extent,
    state: UiState,
    /// 计数（`+`/`-` 改它，它进树 ⇒ 改它必然改画面）。
    count: i32,
    /// 输入框缓冲（`texts["input"]` 的**只读镜像**，方便断言与打印）。
    input: String,
    /// 脏位：`redraw()` 只在它置上时才真的碰 GPU。
    dirty: bool,
    /// 每一帧的账：`true` = 真的重绘了；`false` = 状态没变、跳过了。
    redraw_log: Vec<bool>,
    /// 还没喂掉的脚本事件（空 ⇒ 交互模式）。
    queue: VecDeque<InputEvent>,
    /// 脚本模式：喂完之后要断言的期望计数/输入（内置脚本才有）。
    expected: Option<(i32, String)>,
    /// 建窗后拿到的唤醒句柄（脚本模式用它排下一步；`OnDemand` 下这是唯一的推进手段）。
    waker: Option<Waker>,
    /// 已真的画过几帧。
    rendered: u64,
    engine: Option<TextEngine>,
    renderer: Option<WindowedRenderer>,
    /// 是不是**重放档**（有脚本要喂）。`false` = 纯交互：队列空了也**不许退出**，
    /// 窗口就该一直开着等人操作（这是默认档的本分，也是第一版搞错的地方）。
    script_mode: bool,
    /// 脚本模板（`move @id` 还没展开）；`None` ⇒ 纯交互。
    script_src: Option<String>,
    done: bool,
}

impl Counter {
    /// 当前这一帧用的**度量**：有字体引擎就用真实度量，否则用确定性近似。
    ///
    /// 写成两支而不是 `Box<dyn Measure>`：`FontMeasure` 借用了引擎，装进 trait object
    /// 会逼出生命周期体操，而这里只需要「两条具体路径」二选一。
    fn frame(&self) -> Frame {
        let tree = counter_tree(self.count);
        match &self.engine {
            Some(engine) => Frame::build(&tree, &self.theme, self.extent, &self.state, &engine.measure()),
            None => Frame::build(&tree, &self.theme, self.extent, &self.state, &ApproxMeasure),
        }
    }

    /// 把一条 `UiEvent` 落到**界面自己的数据**上（`UiState` 已由 `interaction::handle` 改好）。
    ///
    /// 这一步是「交互层」与「你的应用逻辑」的分界：交互层只说「`plus` 被点了」，
    /// 「点了要干什么」是你的代码（这里就是 `count += 1`）。
    fn apply_events(&mut self, events: &[UiEvent]) {
        for e in events {
            match e {
                UiEvent::Clicked(id) if id == "plus" => self.count += 1,
                UiEvent::Clicked(id) if id == "minus" => self.count -= 1,
                // 文本缓冲的唯一真相在 `state.texts`；`input` 只是它的只读镜像。
                UiEvent::TextChanged { id, value } if id == "input" => self.input = value.clone(),
                _ => {}
            }
        }
    }

    /// 喂一条输入事件：`handle` + **「真的变了才置脏」**。返回是否请求退出（`key:Escape`）。
    fn feed(&mut self, ev: &InputEvent) -> Result<bool, String> {
        // ① 命中 + 状态机（裁剪快照来自**本帧真实绘制列表**）。
        let (tree, geo, clip) = {
            let f = self.frame();
            (f.tree, f.geo, f.clip)
        };
        let before_state = self.state.clone();
        let before_count = self.count;
        let events = interaction::handle(&mut self.state, &tree, &geo, clip, ev);
        // ② 应用逻辑改界面自己的数据。
        self.apply_events(&events);
        // ③ 脏判据：**状态真的变了**（不是「收到事件了」）。
        let changed = self.state != before_state || self.count != before_count;
        self.dirty |= changed;
        println!(
            "input: {ev:?} => 状态变了={changed} dirty={} events={events:?} {} count={}",
            self.dirty,
            fmt_state(&self.state),
            self.count
        );
        if !changed {
            println!("        （什么都没变 ⇒ 不置脏 ⇒ 这一帧不会重绘）");
        }
        Ok(matches!(ev, InputEvent::KeyDown { key: Key::Escape, .. }))
    }

    /// **唯一的输入入口**：脚本事件与真实窗口事件都在这里落地。
    fn handle_input(&mut self, ev: &InputEvent, source: Source) -> Result<Flow, String> {
        if self.done {
            return Ok(Flow::Exit);
        }
        if self.feed(ev)? {
            println!(
                "{} ⇒ 退出",
                if source == Source::Script {
                    "脚本里的 Escape"
                } else {
                    "窗口按 Esc"
                }
            );
            self.finish_script()?;
            return Ok(Flow::Exit);
        }
        Ok(Flow::Continue)
    }

    /// 喂**一条**脚本事件（`redraw` 每帧喂一条 ⇒「一条输入 ⇒ 一帧画面」是可数的）。
    fn step_script(&mut self) -> Result<Option<Flow>, String> {
        let Some(ev) = self.queue.pop_front() else {
            return Ok(None);
        };
        let flow = self.handle_input(&ev, Source::Script)?;
        Ok(Some(flow))
    }

    /// 脚本**喂完**了：重放档要收尾 + 退出；**纯交互档要接着开**（这才是默认档的本分）。
    ///
    /// 返回 `Some(flow)` = 用这个 flow 继续；`None` = 「脚本重放结束，请退出」。
    ///
    /// ⚠️ 第一版就是在这里把纯交互档也退掉了（实测：窗口开了 1 帧就自己关），
    /// 这正是「默认必须一眼可预期」那条纪律的落点。
    fn after_queue_drained(&mut self) -> Result<Option<Flow>, String> {
        if !self.script_mode {
            println!("[counter] 没有待重放的脚本 ⇒ 继续开着等你操作（Esc 退出）");
            return Ok(Some(Flow::Continue));
        }
        println!("[counter] 脚本队列已空 ⇒ 收尾（已画 {} 帧）", self.rendered);
        self.finish_script()?;
        Ok(None)
    }

    /// 脚本收尾：先打印真实数据，再打期望值，最后**逐字段断言**。
    fn finish_script(&mut self) -> Result<(), String> {
        if self.done {
            return Ok(());
        }
        self.done = true;
        println!();
        println!("—— 脚本重放结束 ——");
        println!("真实终态    : count={} input={:?} {}", self.count, self.input, fmt_state(&self.state));
        let painted = self.redraw_log.iter().filter(|d| **d).count();
        let skipped = self.redraw_log.iter().filter(|d| !**d).count();
        println!(
            "dirty 账本  : 真的重绘 {painted} 次 / 跳过 {skipped} 帧（共记账 {} 帧）；序列 = {:?}",
            self.redraw_log.len(),
            self.redraw_log
        );
        // 判据（**数得清、不含糊**）：
        // - 建窗首帧 + 每条改了状态的输入 ⇒ 各换来一次**真重绘**（`painted` 里数得到）；
        // - 每条**没**改状态的输入 ⇒ 只记一笔跳过、**不碰 GPU**（`skipped` 里数得到）；
        // - 两者相加 == 记账帧数 == 每一帧都记了账（不漏记）。
        //
        // 🚫 不要写成「`painted == self.rendered` 且 `redraw_log.len() == rendered`」：本示例
        //    **第一版就是这么写的**，实测在「被跳过的那一帧喂掉的事件改了状态」时立刻假红
        //    （实测：重绘 10 次 / 记账 11 帧 —— 多出来的那一笔正是跳过帧，它**没有**画）。
        if painted == 0 {
            return Err("dirty 账本：一次真重绘都没有 —— 界面根本没画出来".to_string());
        }
        if self.rendered != painted as u64 {
            return Err(format!(
                "dirty 账本不一致：`rendered` 计数 {} ≠ 账本里的真重绘 {painted} 次",
                self.rendered
            ));
        }
        println!("              （painted={painted} + skipped={skipped} = 记账 {} 帧；`rendered`={}）", self.redraw_log.len(), self.rendered);
        // 「点两次 + ⇒ 显示 2」这条断言：**读的是绘制列表画出来的文本**。
        let f = self.frame();
        assert_count_shown(&f, self.count)?;
        if let Some((want_count, want_input)) = self.expected.clone() {
            println!(
                "期望终态    : count={want_count} input={want_input:?}\
                 （计数显示应当画成 `{COUNT_PREFIX}{want_count}`）"
            );
            if self.count != want_count || self.input != want_input {
                return Err(format!(
                    "脚本重放终态不符：实际 count={} input={:?} / 期望 count={want_count} input={want_input:?}",
                    self.count, self.input
                ));
            }
            assert_count_shown(&f, want_count)?;
            println!("脚本重放终态断言 ✅（计数显示 = `{COUNT_PREFIX}{want_count}`，输入框 = `{want_input}`）");
        } else {
            println!("（自定义脚本：只断言「解析 + 重放 + 计数显示与 count 一致」）");
        }
        Ok(())
    }

    /// 按 id 把脚本模板里的 `move @id` 展开成 `move:X,Y`。
    ///
    /// 写死坐标的脚本在布局改变后会**静默点到空处**，于是「重放通过」变成一句空话；
    /// 按 id 定位让脚本与布局解耦，而且仍然可读。
    fn resolve_script(&self, tpl: &str) -> Result<String, String> {
        let f = self.frame();
        let mut out = String::new();
        for stmt in tpl.split(';') {
            let stmt = stmt.trim();
            if stmt.is_empty() {
                continue;
            }
            if let Some(id) = stmt.strip_prefix("move @") {
                let (x, y) = f.center(id.trim())?;
                out.push_str(&format!("move:{x},{y};"));
            } else {
                out.push_str(stmt);
                out.push(';');
            }
        }
        Ok(out)
    }

    /// 声明重绘策略（教程第 4 节）：两种模式都用默认的省钱档 —— `OnDemand`。
    ///
    /// 脚本模式的推进**不靠连续重绘**，而靠 [`Counter::schedule_next`] 的
    /// `Waker::wake_after`（deadline，不睡线程）；交互模式则彻底空闲零重绘。
    fn redraw_policy(&self) -> RedrawPolicy {
        RedrawPolicy::OnDemand
    }
}

impl App for Counter {
    fn init(&mut self, info: &WindowInfo) -> Result<(), String> {
        self.extent = Extent {
            width: info.extent.width.max(1),
            height: info.extent.height.max(1),
        };
        println!("[counter] 窗口 {}×{}（物理像素）", self.extent.width, self.extent.height);

        // ① 字体：找到就加载（真实字形）；找不到明确降级（占位文本），交互不受影响。
        let mut size = FALLBACK_FONT_SIZE;
        if let Some(font) = find_system_font() {
            match TextEngine::from_font_file(Path::new(&font), FALLBACK_FONT_SIZE) {
                Ok(e) => {
                    println!("[counter] 字体：{}", font.display());
                    size = e.font_size();
                    self.engine = Some(e);
                }
                Err(e) => println!("[counter] 字体解析失败，退回占位文本：{e}"),
            }
        } else {
            println!("[counter] 没找到系统字体 ⇒ 降级：占位文本（交互链不受影响）");
        }
        // 字号一处定义：`theme.font_size` 同时喂给布局的 `TextStyle` 与绘制命令。
        self.theme = Theme {
            font_size: size,
            ..Theme::default()
        };

        // ② 渲染器（窗口 + 交换链 + 呈现）。
        let r = WindowedRenderer::new(0, info.raw, self.extent, CLEAR)
            .map_err(|e| format!("创建窗口渲染器失败：{e}"))?;
        println!("[counter] 适配器：{}", r.adapter().name);
        self.renderer = Some(r);

        // ③ 前置断言：这一帧的列表必须发得出 `NodeHint`，否则命中会退化成「全不裁剪」
        //    而**不报错**（本项目纪律：前置不成立不会自己喊疼）。
        let f = self.frame();
        assert_counter_tree(&f.tree)?;
        let hints = f.list.counts().node_hint;
        if hints == 0 || f.clip.is_empty() {
            return Err(format!(
                "前置条件不成立：这一帧的列表没有节点提示（node_hint={hints}，快照 len={}）—— \
                 命中会退化成「全不裁剪」而不报错；检查是不是用了不发 NodeHint 的渲染器",
                f.clip.len()
            ));
        }
        for id in ["plus", "minus", "input"] {
            if !f.clip.is_known(id) {
                return Err(format!(
                    "前置条件不成立：裁剪快照里没有 `{id}`（`allows()` 对未知 id **放行**）—— \
                     那会让「被裁掉就不命中」这条护栏静默失效"
                ));
            }
        }
        println!(
            "[counter] 绘制列表：{} 条命令（NodeHint {hints} 条）；裁剪快照 {} 个节点",
            f.list.len(),
            f.clip.len()
        );
        println!("[counter] 焦点树序：{:?}", interaction::focusables(&f.tree));
        assert_count_shown(&f, self.count)?;

        // ④ 脚本：**没给脚本就一条都不重放**（纯交互）。内置脚本档有期望终态（它是判据）；
        //    自定义脚本只保证「解析 + 重放 + 计数显示与 count 一致」。
        match self.script_src.clone() {
            None => {
                println!(
                    "[counter] 没有脚本 ⇒ 纯交互：**没有任何事件会被重放**，窗口一直开着直到你按 Esc 或关窗"
                );
                println!(
                    "          （要跑确定性重放：{COUNTER_SCRIPT_ENV}=@builtin；内置脚本 = {BUILTIN_SCRIPT}）"
                );
            }
            Some(tpl) => {
                let custom = tpl != BUILTIN_SCRIPT;
                let src = self.resolve_script(&tpl)?;
                println!("[counter] 脚本（{}）：{src}", if custom { "自定义" } else { "内置" });
                self.queue = deer_gui::input_script::parse_script(&src)?.into_iter().collect();
                if !custom {
                    self.expected = Some((BUILTIN_EXPECTED_COUNT, BUILTIN_EXPECTED_INPUT.to_string()));
                }
                println!("[counter] 解析出 {} 条输入事件（每帧喂一条）", self.queue.len());
            }
        }

        // 首帧必然要画（窗口刚建好）。
        self.dirty = true;
        Ok(())
    }

    fn resized(&mut self, width: u32, height: u32) -> Result<(), String> {
        let e = Extent {
            width: width.max(1),
            height: height.max(1),
        };
        if let Some(r) = self.renderer.as_mut() {
            r.resize(e).map_err(|err| format!("重建交换链失败：{err}"))?;
        }
        self.extent = e;
        self.dirty = true;
        Ok(())
    }

    fn input(&mut self, _info: &WindowInfo, ev: &InputEvent) -> Result<Flow, String> {
        if self.done {
            return Ok(Flow::Exit);
        }
        if !self.queue.is_empty() {
            // 脚本模式下**忽略**窗口事件：重放必须确定性（跑脚本时动鼠标不该改结果）。
            println!("input: 脚本模式，忽略窗口事件 {ev:?}");
            return Ok(Flow::Continue);
        }
        self.handle_input(ev, Source::Window)
    }

    /// 「这一条输入改了状态吗」——报的就是 `redraw()` 用的**同一份** `dirty`。
    ///
    /// `deer-window` 在每条输入派发之后**立刻**读一次它：真 ⇒ 请求一帧；假 ⇒ 省下这一帧。
    fn wants_redraw(&self) -> bool {
        self.dirty
    }

    fn redraw_policy(&self) -> RedrawPolicy {
        Counter::redraw_policy(self)
    }

    fn close_requested(&mut self) -> Flow {
        println!("[counter] 窗口关闭请求 ⇒ 退出");
        Flow::Exit
    }

    fn wake_handle(&mut self, waker: Waker) {
        println!("[counter] 拿到唤醒句柄（脚本模式用它排下一步：deadline，不睡线程）");
        self.waker = Some(waker);
    }

    /// 画一帧（**只在 `dirty` 时真的碰 GPU**）。
    fn redraw(&mut self) -> Result<Flow, String> {
        if self.done {
            return Ok(Flow::Exit);
        }
        if !self.dirty {
            // 不脏 ⇒ 不碰 GPU。脚本模式下还要把脚本往前推一步（否则重放会停在这）。
            self.redraw_log.push(false);
            let stepped = self.step_script()?;
            println!("redraw: no（状态没变，跳过这一帧；已画 {} 帧）", self.rendered);
            // ⚠️ **容易漏的一步**（本示例第一版就在这里挂住过，实测）：
            // 被跳过的这一帧**仍然喂掉了一条脚本事件**；若那条事件改了状态，`dirty` 现在
            // 是 true，而窗口层早在「派发输入之前」就读过 `wants_redraw()`（那时是 false）
            // ⇒ 它不会再请求一帧，脚本就**停在这里**（进程不退出，实测卡在第 5 帧）。
            //
            // 补法用 [`Waker::wake`] 而不是 `wake_after`：这里要的是「**立刻**再来一帧」，
            // 而 `wake()` 的语义正是「叫一声，窗口层随后问一次 `wants_redraw()`」——
            // 此刻 `dirty` 是 true ⇒ 一定拿得到那一帧，且不依赖定时器的到点精度。
            // （脚本的**常规**推进仍用 `wake_after`：那是「排一步」的语义，见 [`Counter::schedule_next`]。）
            if self.dirty && !self.done {
                println!("           被跳过的那一帧喂掉的事件改了状态 ⇒ 自己叫一次 wake()，补画它");
                let waker = self.waker.clone().ok_or(
                    "`OnDemand` 脚本重放需要唤醒句柄（`App::wake_handle` 没被调用？）",
                )?;
                waker.wake();
            }
            return match stepped {
                Some(flow) => Ok(flow),
                None => match self.after_queue_drained()? {
                    Some(flow) => Ok(flow),
                    None => Ok(Flow::Exit),
                },
            };
        }
        self.redraw_log.push(true);
        self.rendered += 1;
        self.dirty = false;

        // 脚本模式：**先喂一条事件、再画** —— 于是「输入 ⇒ 状态 ⇒ 这一帧画出来的东西」
        // 是同一条因果链（与窗口路径的区别只是事件从哪来）。
        let stepped = self.step_script()?;
        if stepped == Some(Flow::Exit) {
            return Ok(Flow::Exit);
        }
        let f = self.frame();
        println!(
            "redraw: yes（第 {} 帧）count={} 命令数={}",
            self.rendered,
            self.count,
            f.list.len()
        );
        let mut engine = self.engine.take();
        let r = self.renderer.as_mut().ok_or("还没有窗口渲染器")?;
        let outcome = r.draw_and_present(&f.list, engine.as_mut());
        let skipped = r.ui_text_skipped();
        self.engine = engine;
        match outcome {
            Ok(FrameOutcome::Presented) => {}
            Ok(FrameOutcome::OutOfDate) => {
                // 交换链过期是**正常路径**：重建后重试，绝不能当成功。
                let e = self.extent;
                if let Some(r) = self.renderer.as_mut() {
                    r.resize(e).map_err(|err| format!("交换链过期后重建失败：{err}"))?;
                }
                self.dirty = true;
            }
            Err(e) => return Err(format!("呈现失败：{e}")),
        }
        println!("           本帧被跳过的字形：{skipped} 个（累计）");
        match stepped {
            Some(flow) => {
                // 脚本还没喂完 ⇒ 排下一次唤醒（`OnDemand` 下这是唯一的推进手段）。
                self.schedule_next()?;
                Ok(flow)
            }
            None => match self.after_queue_drained()? {
                Some(flow) => Ok(flow),
                None => Ok(Flow::Exit),
            },
        }
    }
}

impl Counter {
    /// `OnDemand` + 脚本 ⇒ 每画完一帧排一次唤醒，把脚本推进一步（不睡线程，只装 deadline）。
    fn schedule_next(&mut self) -> Result<(), String> {
        if self.queue.is_empty() || self.done {
            return Ok(());
        }
        let waker = self
            .waker
            .clone()
            .ok_or("`OnDemand` 脚本重放需要唤醒句柄（`App::wake_handle` 没被调用？）")?;
        waker.wake_after(std::time::Duration::from_millis(16));
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// 六、离屏自检（不需要窗口；教程第 7 节）
// ---------------------------------------------------------------------------

/// 「点两次 `+` ⇒ 计数显示为 2」的**可自动判定**版本，全在离屏跑（无窗口、无 GPU）。
///
/// 三步：① 内置脚本重放（走 [`Counter::feed`] 这条输入路径，与窗口路径**同一个入口**）；
/// ② 读**绘制列表画出来的文本**断言 `count = 2`；
/// ③ 用 [`CpuRenderer::with_text`]（**真字形**）出像素，断言计数变化只落在 `count` 矩形内。
///
/// **为什么这一步必须真字形**：无字库的 [`CpuRenderer::new`] 给每个字符画同一个等宽方块
/// ⇒ `count = 0` 与 `count = 1` 的像素逐个相同，像素判据会**假红**（本机实测过）。
/// 找不到系统字体时明确降级（只做文本级断言，并打印「像素断言被跳过」），不假装通过。
fn headless_selfcheck(script: &str) -> Result<(), String> {
    // 字体先建，再定字号：`theme.font_size` / `TextStyle.font_size` / 引擎字号必须是**同一个**
    // 数字，否则「布局算的宽度」与「画出来的宽度」会漂（`TextEngine` 会把字号取整）。
    let mut engine = None;
    if let Some(font) = find_system_font() {
        match TextEngine::from_font_file(Path::new(&font), FALLBACK_FONT_SIZE) {
            Ok(e) => {
                println!("字体        : {}（{} px）", font.display(), e.font_size());
                engine = Some(e);
            }
            Err(e) => println!("字体解析失败 ⇒ 降级：占位文本（像素断言会跳过）：{e}"),
        }
    } else {
        println!("没找到系统字体 ⇒ 降级：占位文本（像素断言会跳过）");
    }
    let font_size = engine
        .as_ref()
        .map(|e| e.font_size())
        .unwrap_or(FALLBACK_FONT_SIZE);
    let theme = Theme {
        font_size,
        ..Theme::default()
    };
    let extent = Extent {
        width: WIDTH,
        height: HEIGHT,
    };
    let mut app = Counter {
        theme,
        extent,
        state: UiState::default(),
        count: 0,
        input: String::new(),
        dirty: true,
        redraw_log: Vec::new(),
        queue: VecDeque::new(),
        expected: None,
        waker: None,
        rendered: 0,
        engine,
        renderer: None,
        script_mode: false,     // 离屏自检不算「窗口档的重放」，队列喂完就收尾
        script_src: None,
        done: false,
    };

    // ① 前置：树的结构与「按 id 读标签」的假设一致，且裁剪快照覆盖了要点的节点。
    let f0 = app.frame();
    assert_counter_tree(&f0.tree)?;
    if f0.clip.is_empty() || !f0.clip.is_known("plus") {
        return Err(format!(
            "前置不成立：裁剪快照为空或缺 `plus`（len={}, known={}）——\
             命中会退化成「全不裁剪」而不报错",
            f0.clip.len(),
            f0.clip.is_known("plus")
        ));
    }
    assert_count_shown(&f0, 0)?;
    println!("初始一帧    : count 显示 `{COUNT_PREFIX}0`；命令 {} 条", f0.list.len());

    // ② 重放脚本（`move @id` 先展开成坐标）。
    let src = app.resolve_script(script)?;
    println!("脚本        : {src}");
    let events = deer_gui::input_script::parse_script(&src)?;
    println!("解析出 {} 条输入事件", events.len());
    let has_engine = app.engine.is_some();
    let mut pixels = app.engine.take().map(CpuRenderer::with_text);
    if pixels.is_none() {
        println!("⚠️ 无字库 ⇒ 本档**不做像素断言**（占位方块让文本变化不可见），只做文本级断言");
    }
    let mut frames = vec![f0];
    for ev in &events {
        let before = app.count;
        app.feed(ev)?;
        // 每条事件之后重建一帧 —— 这就是「重绘」在离屏路径上的对应物。
        frames.push(app.frame());
        // 只有**真的改了计数**的那一步才做像素断言（`move` 只改 hover，不改计数文本）。
        if app.count != before {
            let n = frames.len();
            match pixels.as_mut() {
                Some(r) => assert_pixels_show_count(
                    &frames[n - 2],
                    &frames[n - 1],
                    extent,
                    &format!("count {before}→{}", app.count),
                    r,
                )?,
                None => println!("（无字库：跳过 count {before}→{} 的像素断言）", app.count),
            }
        }
    }
    // `pixels` 在循环里被借用，这里显式丢弃（`TextEngine` 不实现 `Clone`，装不回 `app.engine`；
    // 后续断言只用到绘制列表与 `app` 的状态，不需要引擎）。
    drop(pixels);

    // ③ 终态断言：**计数显示必须是 `count = 2`**，且输入框内容进到了画面里。
    let last = frames.last().expect("至少有一帧");
    assert_count_shown(last, 2)?;
    let drawn = last.drawn_texts();
    let field = drawn.get("input").cloned().unwrap_or_default();
    println!("终态断言    : count={} input={:?}；整帧画出来的文本 = {drawn:?}", app.count, app.input);
    if app.count != 2 {
        return Err(format!("点两次 `+` 之后 count={}，期望 2", app.count));
    }
    if !field.contains("ok") {
        return Err(format!(
            "输入框那一帧画出来的文本是 `{field}`，期望含 `ok` —— 文本输入没有走到渲染"
        ));
    }
    println!(
        "离屏自检 ✅：点两次 `+` ⇒ 计数显示 `{COUNT_PREFIX}2`（绘制列表证据{}）；输入框画出 `ok`",
        if has_engine {
            " + 像素证据"
        } else {
            "；像素证据本档缺席"
        }
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// 七、入口（教程第 1 节）
// ---------------------------------------------------------------------------

/// 脚本来源，三档（**这是本示例第一版搞错、被使用者一眼看穿的地方**，见模块文档）：
///
/// | 输入 | 含义 |
/// |---|---|
/// | 什么都没给 | `NoScript` ⇒ **纯交互**：窗口一直开着，你点、你打字、`Esc` 退出。**不重放任何脚本** |
/// | `@builtin` / `@none` / 空串 | `@builtin` ⇒ 用内置脚本（**带期望终态**，跑完自己退）；`@none` ⇒ 同「什么都没给」 |
/// | 其它 | `Custom(脚本)` ⇒ 重放它，跑完自己退（不断言终态，只断言「计数显示与 count 一致」） |
///
/// ⚠️ **为什么「什么都不给」不能顺手重放内置脚本**（第一版的错）：
/// `OnDemand` 下脚本推进靠 `Waker::wake_after` 的 deadline —— 那本来能自己跑完，
/// 但一旦「默认就重放」，窗口在**人**看来就是「开着不动」；实测使用者会看到
/// 「鼠标移进/移出窗口时它才动一下、然后退出」——**真实鼠标事件成了那个 deadline 的替代品**。
/// 一个示例的默认行为不该让人猜它到底在等谁。所以默认是**真交互**：它就该一直开着，
/// 直到你按 `Esc`（或关窗）。
fn script_from_args() -> Result<ScriptSource, String> {
    let args: Vec<String> = std::env::args().collect();
    let mut from_arg = None;
    for (i, a) in args.iter().enumerate() {
        if a == "--script" {
            from_arg = args.get(i + 1).cloned();
        } else if let Some(v) = a.strip_prefix("--script=") {
            from_arg = Some(v.to_string());
        }
    }
    let from_arg_present = from_arg.is_some();
    let raw = from_arg
        .or_else(|| std::env::var(COUNTER_SCRIPT_ENV).ok())
        .or_else(|| std::env::var(ENV_VAR).ok());
    // **先 `trim()` 再判空**：`cmd` 的 `set X=… && …` 会把尾空格算进值里（见 `env_gate` 的说明）。
    let Some(raw) = raw.filter(|s| !s.trim().is_empty()) else {
        return Ok(ScriptSource::NoScript);
    };
    let trimmed = raw.trim();
    match trimmed {
        "@builtin" | "@builtin-script" => Ok(ScriptSource::Builtin),
        "@none" | "@interactive" => Ok(ScriptSource::NoScript),
        other if other.starts_with('@') => Err(format!(
            "`{other}` 不是已知的脚本标记（`@builtin` = 内置脚本并断言；`@none` = 纯交互）\
             —— 别把它当脚本喂给解析器"
        )),
        other => {
            // `--script` 传来的长参数容易被 shell 吃掉内容（实测被截成过 `move;`，
            // **解析器不会报错**，那是一段合法脚本）⇒ 打出来，让人一眼看见自己拿到了什么。
            if from_arg_present {
                println!("[counter] ⚠️ 脚本来自命令行参数：{other:?}（若与你写的不一致，改用环境变量）");
            }
            Ok(ScriptSource::Custom(other.to_string()))
        }
    }
}

/// 脚本来源（三档，语义见 [`script_from_args`]）。
///
/// 实现 `PartialEq` 是为了让 `main` 能直接判断「这一档到底要不要重放」，
/// 而不是靠 `Option::is_some()` 那种间接推断。
#[derive(Debug, Clone, PartialEq, Eq)]
enum ScriptSource {
    /// 没给脚本 ⇒ **纯交互**（窗口一直开着，`Esc` 退出）。
    NoScript,
    /// `@builtin` ⇒ 内置脚本 + 期望终态断言。
    Builtin,
    /// 自定义脚本 ⇒ 重放，只断言「计数显示与 count 一致」。
    Custom(String),
}

impl ScriptSource {
    /// 这一档的脚本模板：`None` = 不重放。
    fn default_script(&self) -> Option<&str> {
        match self {
            ScriptSource::NoScript => None,
            ScriptSource::Builtin => Some(BUILTIN_SCRIPT),
            ScriptSource::Custom(s) => Some(s.as_str()),
        }
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    let headless = args.iter().any(|a| a == "--headless");

    if headless {
        println!("[counter] --headless：离屏自检（不建窗；输入路径与窗口路径共用 `Counter::feed`）");
        let source = match script_from_args() {
            Ok(s) => s,
            Err(e) => {
                eprintln!("[counter] 参数错误：{e}");
                return ExitCode::FAILURE;
            }
        };
        // 离屏档**默认就是内置脚本**：它不是「人看的窗口」，没有「默认该干什么」的歧义
        // （窗口档的默认必须是纯交互，理由是 `script_from_args` 上那段注释）。
        let src = source.default_script().unwrap_or(BUILTIN_SCRIPT).to_string();
        return match headless_selfcheck(&src) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("[counter] 离屏自检失败：{e}");
                ExitCode::FAILURE
            }
        };
    }

    // 真窗口：**本进程必须在主线程建事件循环**（winit 要求），所以这是 example 而不是 `#[test]`。
    let source = match script_from_args() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[counter] 参数错误：{e}");
            return ExitCode::FAILURE;
        }
    };
    let script = source.default_script().map(str::to_string);
    match &source {
        ScriptSource::NoScript => {
            println!("[counter] **交互模式**：点 + / -、点输入框打字、Tab 换焦点、**Esc 退出**");
            println!("          （没给脚本 ⇒ 不做任何重放。要跑确定性重放：{COUNTER_SCRIPT_ENV}=@builtin）");
        }
        ScriptSource::Builtin => {
            println!("[counter] 内置脚本档（{COUNTER_SCRIPT_ENV}=@builtin）：有期望终态，跑完自己退");
        }
        ScriptSource::Custom(s) => {
            println!("[counter] 脚本档（重放期间忽略窗口事件）：{s}");
        }
    }
    let replay = source != ScriptSource::NoScript;
    let app = Counter {
        theme: Theme {
            font_size: FALLBACK_FONT_SIZE,
            ..Theme::default()
        },
        extent: Extent {
            width: WIDTH,
            height: HEIGHT,
        },
        state: UiState::default(),
        count: 0,
        input: String::new(),
        dirty: false,
        redraw_log: Vec::new(),
        queue: VecDeque::new(),
        expected: None,
        waker: None,
        rendered: 0,
        engine: None,
        renderer: None,
        script_mode: replay,
        script_src: script,
        done: false,
    };
    match run(WindowConfig::new("deer-gui counter", WIDTH, HEIGHT), app) {
        Ok(()) => {
            println!(
                "[counter] 通过 ✅（{}）",
                if replay {
                    "脚本重放：终态逐字段断言"
                } else {
                    "交互模式：窗口正常关闭（Esc / 关闭按钮）"
                }
            );
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("[counter] 失败：{e}");
            ExitCode::FAILURE
        }
    }
}
