//! M3a-T4：**GPU 几何渲染器 vs CPU 后端逐像素对照**（本切片的终局判据）。
//!
//! ## 判据
//!
//! - **不透明（`a == 1`）⇒ 逐字节相同**；
//! - **半透明（`0 < a < 1`）⇒ 最大通道差 ≤ 1 LSB**。CPU 用 `round()`、GPU 走固定功能
//!   `float → unorm8` 转换（舍入时机与平局规则都不同），差 1 是**允许的**，不是缺陷；
//! - 另外两条行为也要对齐：**裁剪栈不平衡 ⇒ 报错**（与 `null.rs` 同一判据）、
//!   **`DrawCmd::Text` ⇒ `Unsupported`**（不静默丢弃）。
//!
//! ## 语料
//!
//! 填充 / 圆角（含超大半径）/ 描边（含**带宽 > 矩形边长** —— 那边带会伸出矩形之外）/
//! 退化描边宽度 / 裁剪 / **嵌套裁剪** / 退化 extent / 全画布清屏 / 半透明混合 /
//! 同一渲染器连续两帧 / 「净计数配平但有多余 PopClip」的列表（CPU 画得出来，GPU 也必须画）。
//!
//! ## 无 GPU 时
//!
//! 优雅跳过（打印原因），与既有 GPU 测试（`offscreen_render.rs` 等）一致 ——
//! 绝不伪装成通过。

use deer_gpu::null::CpuRenderer;
use deer_gpu::{Color, DrawCmd, DrawList, Extent, RectI};
use deer_vk::gpu_geom::GpuVertex;
use deer_vk::GpuGeometryRenderer;

/// 清屏色（不透明 ⇒ UNORM 转换两边都是精确的）。
const CLEAR: Color = Color::rgb(16, 16, 16);

/// `DEER_VK_VALIDATION=1`/`true` ⇒ 请求校验层。
///
/// **直接调库里的那个判据**（fix round 2 / R4）：测试不该手抄一份「什么算请求」——
/// 抄了就有两份口径，而口径分叉正是「同一个二进制只有一半开了校验」那类问题的来源。
fn validation_requested() -> bool {
    deer_vk::ffi::Instance::validation_from_env()
}

/// 断言「到目前为止校验层**没有**报过任何消息」（进程级计数，见 `ffi::validation_message_count`）。
///
/// 只在 `DEER_VK_VALIDATION=1` 下有意义（层没开时回调根本不会跑，计数恒为 0）——
/// 所以它与 `validation_layer_state_matches_the_request` 配套：那条保证**层真的在跑**，
/// 这条保证跑了之后**一条消息都没有**。两件事合起来才把「零校验消息」变成可回归结论
/// （fix round 2 / R1-2；此前它只靠人眼看输出）。
fn assert_no_validation_messages(context: &str) {
    let n = deer_vk::ffi::validation_message_count();
    assert_eq!(
        n,
        0,
        "{context}: 校验层报了 {n} 条消息（DEER_VK_VALIDATION={:?}）—— \
         这是「零校验消息」的自动断言版",
        std::env::var("DEER_VK_VALIDATION").ok()
    );
}

/// 建渲染器。**请求了校验层时「建不起来」就是测试失败，不是「跳过」**（T3 review F7）：
/// 否则「层没装好 / 被静默降级」会表现成一次安静的跳过，而验收里最重要的那条
/// （零校验消息）就变成了空话。
fn renderer(extent: Extent) -> Option<GpuGeometryRenderer> {
    match GpuGeometryRenderer::new(0, extent, CLEAR) {
        Ok(r) => Some(r),
        Err(e) if validation_requested() => panic!(
            "DEER_VK_VALIDATION 已请求，但 GPU 渲染器建不起来（{e}）—— \
             要么校验层没生效（`ffi::Instance::create_with_extensions` 本该直接报错），\
             要么设备本身起不来；这两种都不能打印一句「跳过」就当通过"
        ),
        Err(e) => {
            println!("跳过：本机没有可用的 Vulkan GPU（{e}）");
            None
        }
    }
}

/// **F7：把「零校验消息」从人工观察升级为可回归断言。**
///
/// 校验层的全部价值在于「它真的在跑」。「跑起来没看到消息」可能意味着两件事：
/// ① 真的没问题；② 层根本没加载（或有人把「层缺失就报错」改成静默降级）。
/// 只看输出无法区分，所以这里硬断言：
/// - 请求了 ⇒ `validation_enabled()` 必须为真（设备建不起来时上面的 `renderer()` 已经 panic）；
/// - 没请求 ⇒ 必须为假（挡住「永远返回 true」的假实现）。
#[test]
fn validation_layer_state_matches_the_request() {
    let Some(mut r) = renderer(Extent { width: 8, height: 8 }) else {
        return;
    };
    let requested = validation_requested();
    let actual = r.validation_enabled();
    println!("校验层：请求={requested} 实际={actual}");
    assert_eq!(
        actual,
        requested,
        "DEER_VK_VALIDATION={:?}（请求={requested}）但设备报告 validation_enabled={actual}；\
         「零校验消息」必须建立在「校验层确实在跑」之上，否则它什么也没证明",
        std::env::var("DEER_VK_VALIDATION").ok()
    );

    // 真的画一帧（让校验层有东西可校验），再断言「一条消息都没有」。
    // 这一帧刻意同时用上裁剪 / 圆角 / 半透明厚描边 —— 也就是最容易被校验层挑出问题的几条路径。
    let mut l = DrawList::new();
    l.push(DrawCmd::PushClip { rect: RectI::new(1, 1, 5, 5) });
    l.push(DrawCmd::FillRoundRect { rect: RectI::new(0, 0, 8, 8), radius: 2, color: Color::WHITE });
    l.push(DrawCmd::StrokeRect {
        rect: RectI::new(0, 0, 8, 8),
        color: Color::rgba(255, 0, 0, 0.5),
        width: 2,
    });
    l.push(DrawCmd::PopClip);
    r.render(&l).expect("这一帧应当成功");
    assert_no_validation_messages("单帧（裁剪 + 圆角 + 半透明厚描边）");
}

/// 对照一帧，返回**最大通道差**。
///
/// `max_allowed = 0` ⇒ 额外断言逐字节相同（不透明）；`= 1` ⇒ 半透明（UNORM 舍入）。
fn compare(r: &mut GpuGeometryRenderer, name: &str, list: &DrawList, max_allowed: u8) -> u8 {
    let extent = r.extent();
    let gpu = r
        .render(list)
        .unwrap_or_else(|e| panic!("{name}: GPU 渲染失败：{e}"));
    assert!(r.unsupported().is_empty(), "{name}: 本用例不该有 unsupported");
    let cpu = CpuRenderer::new()
        .render(extent, list, CLEAR)
        .expect("CPU 渲染失败");
    let cpu = cpu.to_rgba();
    assert_eq!(
        gpu.len(),
        cpu.len(),
        "{name}: 回读长度必须等于 CPU 帧缓冲长度（{} vs {}）",
        gpu.len(),
        cpu.len()
    );

    let mut worst = 0u8;
    let mut at = 0usize;
    for (i, (g, c)) in gpu.iter().zip(cpu.iter()).enumerate() {
        let d = g.abs_diff(*c);
        if d > worst {
            worst = d;
            at = i;
        }
    }
    let (px, ch) = (at / 4, at % 4);
    println!(
        "  {name}: 最大通道差 {worst}（允许 {max_allowed}）{}",
        if worst == 0 { "，逐字节相同" } else { "" }
    );
    assert!(
        worst <= max_allowed,
        "{name}: 最大通道差 {worst} > 允许的 {max_allowed}；最差处像素 ({}, {}) 通道 {ch}：\
         GPU={:?} CPU={:?}",
        px % extent.width.max(1) as usize,
        px / extent.width.max(1) as usize,
        &gpu[at - ch..at - ch + 4],
        &cpu[at - ch..at - ch + 4]
    );
    if max_allowed == 0 {
        assert_eq!(gpu, cpu, "{name}: 不透明绘制必须逐字节相同");
    }
    worst
}

/// 不透明语料（**逐字节**判据）。
fn opaque_corpus() -> Vec<(&'static str, DrawList)> {
    let one = |cmd: DrawCmd| {
        let mut l = DrawList::new();
        l.push(cmd);
        l
    };
    let w = Color::WHITE;
    vec![
        // 全画布清屏：空列表 ⇒ 全是清屏色
        ("empty-clear", DrawList::new()),
        ("fill", one(DrawCmd::FillRect { rect: RectI::new(3, 2, 8, 5), color: w })),
        // 部分越出画布：CPU 的 blend 本来就不写越界像素，GPU 靠静态 scissor 裁
        ("fill-offscreen", one(DrawCmd::FillRect { rect: RectI::new(-4, -3, 8, 7), color: w })),
        // 零面积：两边都什么都不画
        ("fill-degenerate", one(DrawCmd::FillRect { rect: RectI::new(5, 5, 0, 4), color: w })),
        ("round-1", one(DrawCmd::FillRoundRect { rect: RectI::new(2, 2, 10, 8), radius: 1, color: w })),
        ("round-3", one(DrawCmd::FillRoundRect { rect: RectI::new(2, 2, 10, 8), radius: 3, color: w })),
        // 超大圆角：半径 > 半宽，四角判据必须与 CPU 逐字一致
        ("round-huge", one(DrawCmd::FillRoundRect { rect: RectI::new(2, 1, 9, 7), radius: 9, color: w })),
        ("stroke-1px", one(DrawCmd::StrokeRect { rect: RectI::new(2, 2, 12, 9), color: w, width: 1 })),
        ("stroke-w2", one(DrawCmd::StrokeRect { rect: RectI::new(2, 2, 12, 9), color: w, width: 2 })),
        ("stroke-w3", one(DrawCmd::StrokeRect { rect: RectI::new(2, 2, 12, 9), color: w, width: 3 })),
        // ★ 带宽 > 矩形边长：边带会沿短边**伸出矩形之外**，GPU 只光栅化图元覆盖区
        ("stroke-band-exceeds-rect", one(DrawCmd::StrokeRect { rect: RectI::new(3, 3, 4, 2), color: w, width: 5 })),
        ("stroke-band-exceeds-tall", one(DrawCmd::StrokeRect { rect: RectI::new(4, 2, 2, 9), color: w, width: 6 })),
        // 退化描边宽度：CPU `stroke()` 是 `width.max(1)` ⇒ 1px 边框（不是实心！）
        ("stroke-width-zero", one(DrawCmd::StrokeRect { rect: RectI::new(2, 2, 10, 6), color: w, width: 0 })),
        ("stroke-width-negative", one(DrawCmd::StrokeRect { rect: RectI::new(2, 2, 10, 6), color: w, width: -3 })),
        // 1×1 矩形配 3px 带宽
        ("stroke-tiny-rect-thick", one(DrawCmd::StrokeRect { rect: RectI::new(6, 5, 1, 1), color: w, width: 3 })),
        // 裁剪 × 各种形状
        (
            "clip-fill",
            {
                let mut l = DrawList::new();
                l.push(DrawCmd::PushClip { rect: RectI::new(4, 3, 8, 6) });
                l.push(DrawCmd::FillRect { rect: RectI::new(0, 0, 24, 16), color: w });
                l.push(DrawCmd::PopClip);
                l
            },
        ),
        (
            "clip-round",
            {
                let mut l = DrawList::new();
                l.push(DrawCmd::PushClip { rect: RectI::new(3, 2, 10, 8) });
                l.push(DrawCmd::FillRoundRect { rect: RectI::new(2, 1, 14, 10), radius: 3, color: w });
                l.push(DrawCmd::PopClip);
                l
            },
        ),
        (
            "clip-stroke-thick",
            {
                let mut l = DrawList::new();
                l.push(DrawCmd::PushClip { rect: RectI::new(5, 4, 9, 7) });
                l.push(DrawCmd::StrokeRect { rect: RectI::new(4, 3, 12, 9), color: w, width: 3 });
                l.push(DrawCmd::PopClip);
                l
            },
        ),
        // 嵌套裁剪：内层 Pop 之后必须回到**外层**裁剪
        (
            "nested-clip",
            {
                let mut l = DrawList::new();
                l.push(DrawCmd::PushClip { rect: RectI::new(1, 1, 14, 12) });
                l.push(DrawCmd::PushClip { rect: RectI::new(5, 3, 10, 8) });
                l.push(DrawCmd::FillRect { rect: RectI::new(0, 0, 24, 16), color: w });
                l.push(DrawCmd::PopClip);
                l.push(DrawCmd::FillRect { rect: RectI::new(0, 0, 24, 16), color: w });
                l.push(DrawCmd::PopClip);
                l
            },
        ),
        // 多条命令叠加（顺序敏感）+ `NodeHint` 不产生像素
        (
            "mixed",
            {
                let mut l = DrawList::new();
                l.push(DrawCmd::FillRect { rect: RectI::new(0, 0, 24, 16), color: w });
                l.push(DrawCmd::NodeHint { rect: RectI::new(0, 0, 24, 16), node_id_len: 3 });
                l.push(DrawCmd::FillRect { rect: RectI::new(2, 2, 12, 9), color: Color::rgb(0, 0, 0) });
                l.push(DrawCmd::StrokeRect { rect: RectI::new(1, 1, 20, 13), color: w, width: 2 });
                l.push(DrawCmd::FillRoundRect { rect: RectI::new(7, 4, 8, 7), radius: 2, color: w });
                l
            },
        ),
    ]
}

/// **Ruling 19 的正面用例**：`[PopClip, PushClip]` 这种「净计数配平但有多余 PopClip」的列表
/// CPU 画得出来，GPU 也必须画（**不**拿更严的 `GpuStream::clip_unbalanced` 当报错条件）。
///
/// 净计数：`PopClip`（-1）+ `PushClip`（+1）= **0** ⇒ `clip_balanced()` 为真，
/// 但栈里被弹过一次「全画布」⇒ `GpuStream::clip_unbalanced` 为真。两者必须分开对待。
fn net_balanced_with_extra_pop() -> DrawList {
    let mut l = DrawList::new();
    l.push(DrawCmd::PopClip);
    l.push(DrawCmd::PushClip { rect: RectI::new(4, 4, 8, 8) });
    l.push(DrawCmd::FillRect { rect: RectI::new(0, 0, 24, 16), color: Color::WHITE });
    l
}

/// 半透明语料（**≤1 LSB** 判据）。
fn alpha_corpus() -> Vec<(&'static str, DrawList)> {
    let half = Color::rgba(255, 0, 0, 0.5);
    let quarter = Color::rgba(0, 128, 255, 0.25);
    let mut v: Vec<(&'static str, DrawList)> = Vec::new();

    let mut l = DrawList::new();
    l.push(DrawCmd::FillRect { rect: RectI::new(3, 2, 12, 9), color: half });
    v.push(("alpha-fill", l));

    let mut l = DrawList::new();
    l.push(DrawCmd::FillRect { rect: RectI::new(0, 0, 20, 14), color: quarter });
    l.push(DrawCmd::FillRoundRect { rect: RectI::new(3, 2, 14, 10), radius: 3, color: half });
    l.push(DrawCmd::StrokeRect { rect: RectI::new(2, 1, 18, 12), color: quarter, width: 1 });
    v.push(("alpha-mixed", l));

    // 2px 半透明描边：四角像素落在两条边带里 ⇒ 被混合**两次**（CPU 也如此）
    let mut l = DrawList::new();
    l.push(DrawCmd::StrokeRect { rect: RectI::new(3, 2, 14, 10), color: half, width: 2 });
    v.push(("alpha-stroke-overlap", l));

    let mut l = DrawList::new();
    l.push(DrawCmd::FillRect { rect: RectI::new(2, 2, 10, 8), color: Color::TRANSPARENT });
    v.push(("alpha-zero", l));

    // 半透明 + 裁剪
    let mut l = DrawList::new();
    l.push(DrawCmd::PushClip { rect: RectI::new(4, 3, 10, 8) });
    l.push(DrawCmd::FillRoundRect { rect: RectI::new(2, 1, 16, 12), radius: 4, color: half });
    l.push(DrawCmd::StrokeRect { rect: RectI::new(2, 1, 16, 12), color: quarter, width: 3 });
    l.push(DrawCmd::PopClip);
    v.push(("alpha-clip", l));

    v
}

/// 不透明绘制：**逐字节相同**（这是本切片的硬性验收）。
#[test]
fn opaque_drawings_match_cpu_byte_for_byte() {
    let Some(mut r) = renderer(Extent { width: 24, height: 16 }) else {
        return;
    };
    let mut worst = 0u8;
    for (name, list) in opaque_corpus() {
        worst = worst.max(compare(&mut r, name, &list, 0));
    }
    worst = worst.max(compare(&mut r, "net-balanced-extra-pop", &net_balanced_with_extra_pop(), 0));
    println!("不透明语料最大通道差 = {worst}（要求 0）");
    assert_eq!(worst, 0, "不透明绘制必须逐字节相同");
    assert_no_validation_messages("不透明语料");
}

/// 半透明绘制：最大通道差 **≤ 1 LSB**。
#[test]
fn semi_transparent_drawings_match_cpu_within_one_lsb() {
    let Some(mut r) = renderer(Extent { width: 24, height: 16 }) else {
        return;
    };
    let mut worst = 0u8;
    for (name, list) in alpha_corpus() {
        worst = worst.max(compare(&mut r, name, &list, 1));
    }
    println!("半透明语料最大通道差 = {worst}（要求 ≤1）");
    assert!(worst <= 1, "半透明最大通道差 {worst} 超过 1 LSB");
    assert_no_validation_messages("半透明语料");
}

/// **越界 alpha（`a > 1.0` 或 `< 0`）也必须与 CPU 逐字节一致**（T2 review M1）。
///
/// 依据：CPU 基线 `null.rs::blend_cov` 第一步就是 `c.a.clamp(0.0, 1.0)`，而 `Color::rgba`
/// 对 alpha **没有校验**。`build_stream` 现在同样夹 ⇒ 两边都把 `a = 1.5` 当**不透明**、
/// 把 `a = -1.0` 当**全透明**（都不写像素）。
///
/// 所以这里的判据是**逐字节相同**（`max_allowed = 0`），而不是「≤1 LSB」。
///
/// ⚠️ **诚实说明这条测试的判别力**（实测，别高估它）：把 `color_f32` 的 `clamp` 去掉后，
/// **本测试仍然通过** —— 因为 UNORM 颜色附件在固定功能混合阶段会**隐式**把源 alpha 夹到
/// `[0,1]`（本机 Intel 驱动实测如此）。也就是说：真正能抓住「忘了 clamp」的是**发射侧**的
/// 断言 `gpu_geom_stream.rs::out_of_range_alpha_is_clamped_like_the_cpu_baseline`，
/// 而本测试守的是「**两边在越界 alpha 上仍然等价**」这个结论本身（若将来换成浮点附件、
/// 或驱动不再隐式夹，它就会变成有判别力的那一条）。
#[test]
fn out_of_range_alpha_matches_cpu_byte_for_byte() {
    let Some(mut r) = renderer(Extent { width: 16, height: 12 }) else {
        return;
    };
    let mut l = DrawList::new();
    l.push(DrawCmd::FillRect {
        rect: RectI::new(1, 1, 12, 8),
        color: Color::rgba(255, 0, 0, 1.5), // ⇒ 两边都 clamp 到 1.0（不透明）
    });
    l.push(DrawCmd::FillRoundRect {
        rect: RectI::new(3, 3, 8, 6),
        radius: 2,
        color: Color::rgba(0, 128, 255, 2.0), // ⇒ 两边都 clamp 到 1.0
    });
    l.push(DrawCmd::StrokeRect {
        rect: RectI::new(2, 2, 10, 7),
        color: Color::rgba(255, 255, 255, -1.0), // ⇒ 两边都 clamp 到 0.0（全透明，不写像素）
        width: 2,
    });
    assert_eq!(compare(&mut r, "alpha-out-of-range", &l, 0), 0);
    assert_no_validation_messages("越界 alpha");
}

/// 另一个静态 viewport 尺寸（证明 viewport/scissor 是**跟着 extent 建进管线**的）。
#[test]
fn a_different_extent_uses_its_own_static_viewport() {
    let Some(mut r) = renderer(Extent { width: 8, height: 6 }) else {
        return;
    };
    assert_eq!(r.extent(), Extent { width: 8, height: 6 });
    let mut l = DrawList::new();
    l.push(DrawCmd::FillRoundRect { rect: RectI::new(1, 1, 6, 4), radius: 2, color: Color::WHITE });
    l.push(DrawCmd::StrokeRect { rect: RectI::new(0, 0, 8, 6), color: Color::WHITE, width: 1 });
    assert_eq!(compare(&mut r, "small-canvas", &l, 0), 0);
}

/// 退化 extent（0×0）：按 1×1 渲染（与 CPU `Framebuffer::new(..max(1))` 同一约定），
/// 且画出来的内容与 CPU 的 1×1 帧缓冲**逐字节相同**。
#[test]
fn degenerate_extent_renders_as_one_by_one_like_the_cpu() {
    let Some(mut r) = renderer(Extent { width: 0, height: 0 }) else {
        return;
    };
    assert_eq!(
        r.extent(),
        Extent { width: 1, height: 1 },
        "0 尺寸必须按 1×1 渲染（否则 Vulkan 图像非法，而 CPU 那边能画 1×1）"
    );
    // 全画布清屏
    assert_eq!(compare(&mut r, "degenerate-clear", &DrawList::new(), 0), 0);
    // 铺满 1×1 的填充
    let mut l = DrawList::new();
    l.push(DrawCmd::FillRect { rect: RectI::new(0, 0, 1, 1), color: Color::WHITE });
    assert_eq!(compare(&mut r, "degenerate-fill", &l, 0), 0);
}

/// 同一个渲染器连续两帧：第二帧必须是**第二帧的内容**（顶点缓冲每帧重传，不能留上一帧）。
#[test]
fn consecutive_frames_do_not_leak_vertex_data() {
    let Some(mut r) = renderer(Extent { width: 24, height: 16 }) else {
        return;
    };
    // 第一帧：一个铺满的大矩形（顶点数与第二帧不同，且覆盖面积更大）
    let mut big = DrawList::new();
    big.push(DrawCmd::FillRect { rect: RectI::new(0, 0, 24, 16), color: Color::WHITE });
    let _ = compare(&mut r, "first-frame-big", &big, 0);
    // 第二帧：一个小矩形（如果顶点缓冲没重传，会看到上一帧的残留）
    let mut small = DrawList::new();
    small.push(DrawCmd::FillRect { rect: RectI::new(5, 4, 3, 2), color: Color::WHITE });
    assert_eq!(compare(&mut r, "second-frame-small", &small, 0), 0);
    // 第三帧：回到空列表 ⇒ 应该只剩清屏色
    assert_eq!(compare(&mut r, "third-frame-clear", &DrawList::new(), 0), 0);
}

/// `DrawCmd::Text`：**报 `Unsupported`** 而不是静默丢弃；报告里能看到是哪条命令；
/// 而且失败之后渲染器仍然可用（状态没被弄坏）。
#[test]
fn text_is_reported_as_unsupported_and_the_renderer_survives() {
    let Some(mut r) = renderer(Extent { width: 24, height: 16 }) else {
        return;
    };
    let mut l = DrawList::new();
    l.push(DrawCmd::FillRect { rect: RectI::new(0, 0, 8, 8), color: Color::WHITE });
    l.push(DrawCmd::Text {
        rect: RectI::new(0, 0, 20, 10),
        text: "hi".into(),
        color: Color::WHITE,
        size: 12.0,
        align: 0,
    });
    let err = r.render(&l).expect_err("含文本的列表必须报错，不能静默画一半");
    let msg = format!("{err}");
    assert!(msg.contains("Unsupported") || msg.contains("不支持"), "错误应当是 Unsupported：{msg}");
    assert_eq!(r.unsupported().len(), 1, "报告里要有那一条文本命令");
    assert!(r.unsupported()[0].contains("Text"), "报告应当指认命令：{:?}", r.unsupported());

    // 失败之后仍能正常画（没有半提交、没有卡死的栅栏）
    let mut ok = DrawList::new();
    ok.push(DrawCmd::FillRect { rect: RectI::new(1, 1, 6, 4), color: Color::WHITE });
    assert_eq!(compare(&mut r, "after-unsupported", &ok, 0), 0);
}

/// 裁剪栈**不平衡**：GPU 必须像 CPU 一样**报错**（判据相同 ⇒ 行为相同）。
#[test]
fn unbalanced_clip_is_rejected_like_the_cpu_backend() {
    let Some(mut r) = renderer(Extent { width: 24, height: 16 }) else {
        return;
    };
    let mut l = DrawList::new();
    l.push(DrawCmd::PushClip { rect: RectI::new(1, 1, 8, 8) });
    l.push(DrawCmd::FillRect { rect: RectI::new(0, 0, 24, 16), color: Color::WHITE });

    let cpu = CpuRenderer::new().render(r.extent(), &l, CLEAR);
    assert!(cpu.is_err(), "CPU 后端本来就拒收不平衡的列表（null.rs:227-236）");
    let gpu = r.render(&l);
    assert!(gpu.is_err(), "GPU 也必须拒收（判据与 CPU 相同）");
    println!("两边都拒收未配对的 PushClip：CPU={:?} / GPU={:?}", cpu.err(), gpu.err().map(|e| e.to_string()));

    // 反面：多一个 PopClip 但净计数配平 ⇒ **两边都要画得出来**（Ruling 19）
    let odd = net_balanced_with_extra_pop();
    assert!(odd.clip_balanced(), "这份列表的净计数是配平的");
    let cpu = CpuRenderer::new().render(r.extent(), &odd, CLEAR);
    assert!(cpu.is_ok(), "CPU 能画这种列表");
    assert_eq!(compare(&mut r, "net-balanced-extra-pop", &odd, 0), 0);
}

/// `GpuVertex` 的布局是给 `VkVertexInputAttributeDescription` 的硬契约：
/// 这里钉**字面数字**（`gpu_render.rs` 用 `offset_of!` 取偏移，两者必须一致）。
#[test]
fn vertex_layout_matches_the_hand_written_attribute_offsets() {
    assert_eq!(std::mem::size_of::<GpuVertex>(), 44, "stride");
    assert_eq!(std::mem::align_of::<GpuVertex>(), 4);
    assert_eq!(std::mem::offset_of!(GpuVertex, pos), 0, "location 0：vec2 pos");
    assert_eq!(std::mem::offset_of!(GpuVertex, rect), 8, "location 1：vec4 rect");
    assert_eq!(std::mem::offset_of!(GpuVertex, radius_kind), 24, "location 2：float radius_kind");
    assert_eq!(std::mem::offset_of!(GpuVertex, color), 28, "location 3：vec4 color");
}

/// **R1-1**：一个「有顶点」的帧必须发出**恰好一条** host→vertex 屏障；空帧不得发。
///
/// 为什么要有这条（而不是只靠 `vertex_barrier_params_pin_the_exact_masks` 那个单元测试）：
/// 单元测试只能钉「参数对不对」，钉不住「屏障有没有真的被发出来」。reviewer 的变异证明
/// **把整段屏障删掉**（= 原始缺陷复原）在 16 个测试靶上**全绿** —— 本测试就是那条变异的判据：
/// 删掉屏障 ⇒ 计数不再增长 ⇒ **变红**。
#[test]
fn host_to_vertex_barrier_is_emitted_once_per_non_empty_frame() {
    let Some(mut r) = renderer(Extent { width: 8, height: 8 }) else {
        return;
    };
    let base = r.host_to_vertex_barrier_count();

    // 空列表（只剩清屏）：没有顶点要读 ⇒ 不该发
    r.render(&DrawList::new()).expect("空帧应当成功");
    assert_eq!(
        r.host_to_vertex_barrier_count(),
        base,
        "空帧没有顶点缓冲要读，不得发 host→vertex 屏障"
    );

    // 有顶点 ⇒ 恰好 +1
    let mut l = DrawList::new();
    l.push(DrawCmd::FillRect { rect: RectI::new(1, 1, 4, 4), color: Color::WHITE });
    r.render(&l).expect("有顶点的帧应当成功");
    assert_eq!(
        r.host_to_vertex_barrier_count(),
        base + 1,
        "有顶点的帧必须发**且只发一条** host→vertex 屏障"
    );

    // 每帧都重写顶点缓冲 ⇒ 每帧都要重新建立这条依赖
    r.render(&l).expect("第二帧应当成功");
    assert_eq!(
        r.host_to_vertex_barrier_count(),
        base + 2,
        "每帧重传顶点 ⇒ 每帧都要发屏障"
    );
}

/// **R1-3**：「上次提交未确认完成」之后，`render` 必须**必定报错**，且不去碰任何资源。
///
/// 真实触发路径是栅栏超时/设备丢失（测试里无法稳定复现），所以用
/// [`GpuGeometryRenderer::force_unconfirmed_submit_for_test`] 这个明确的测试入口进入该状态。
///
/// **变异验证**：把 `render` 开头的 `ensure_reusable()?` 改成 `let _ =`（reviewer 的变异 C）
/// ⇒ 本测试**仍然通过**（这正是修复的目的：守卫不依赖调用方的写法）；但把
/// `record_and_submit` 与 `ensure_vertex_capacity` 内部的守卫**也**去掉 ⇒ **本测试变红**。
#[test]
fn render_is_refused_after_an_unconfirmed_submit() {
    let Some(mut r) = renderer(Extent { width: 8, height: 8 }) else {
        return;
    };
    let mut l = DrawList::new();
    l.push(DrawCmd::FillRect { rect: RectI::new(1, 1, 4, 4), color: Color::WHITE });

    // 先正常画一帧，确认这个渲染器本来是可用的（否则下面的断言可能因为别的原因通过）
    r.render(&l).expect("正常帧应当成功");

    r.force_unconfirmed_submit_for_test();
    let err = r.render(&l).expect_err("上次提交未确认完成 ⇒ 必须拒绝复用");
    let msg = format!("{err}");
    assert!(
        msg.contains("不可复用") || msg.contains("没有确认完成"),
        "错误信息要说清原因是「上次提交未确认完成」：{msg}"
    );
    // 状态不会被「用掉」：第二次仍然拒绝，而不是只报一次错
    assert!(r.render(&l).is_err(), "Broken 必须**持续**拒绝，而不是一次性报错");
    // 空帧同样要复用命令缓冲/栅栏 ⇒ 也必须拒绝
    assert!(r.render(&DrawList::new()).is_err(), "空帧也要复用命令缓冲 ⇒ 同样拒绝");
}
