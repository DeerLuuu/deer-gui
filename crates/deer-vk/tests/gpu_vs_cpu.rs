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
use deer_vk::device::VkDevice;
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

/// 断言「到目前为止校验层**没有**报过任何消息」。
///
/// 只在 `DEER_VK_VALIDATION=1` 下有意义（层没开时回调根本不会跑，计数恒为 0）——
/// 所以它与 `validation_layer_state_matches_the_request` 配套：那条保证**层真的在跑**，
/// 这条保证跑了之后**一条消息都没有**。两件事合起来才把「零校验消息」变成可回归结论
/// （fix round 2 / R1-2；此前它只靠人眼看输出）。
///
/// ## 用**按线程**计数 + 窗口差值（M3c 复查修正）
///
/// 原来用**进程级**计数 + `== 0`——那是**绝对**断言，在并行测试里会被别的测试的
/// 消息顶红（M3c 期间真实发生过：一条新测试产生 5 条 VUID 把这类断言全部假红）。
/// 改成按线程后，差值只反映**本测试自己**触发的消息。
/// 「本进程常规路径不产生消息」这条更弱的覆盖由 `pipeline_smoke.rs` 的
/// `process_reports_zero_validation_messages_in_a_fresh_child`（干净子进程里的绝对断言）负责。
fn assert_no_validation_messages(context: &str) {
    let n = deer_vk::ffi::validation_message_count();
    assert_eq!(
        n,
        0,
        "{context}: **本线程**收到 {n} 条校验层消息（DEER_VK_VALIDATION={:?}）—— \
         这是「零校验消息」的自动断言版（按线程 ⇒ 不含别的测试）",
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
                l.push(DrawCmd::node_hint(RectI::new(0, 0, 24, 16), "mix"));
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

/// **统一顶点**的布局是给 `VkVertexInputAttributeDescription` 的硬契约（B5-2）：
/// 这里钉**字面数字**（`gpu_render.rs::unified_attrs` 用 `offset_of!` 取偏移，两者必须一致）。
///
/// 与 [`vertex_layout_matches_the_hand_written_attribute_offsets`] 同一做法，
/// 对象换成统一顶点：**stride 52 / 偏移 0 / 8 / 24 / 28 / 44**（与
/// `spirv::vertex_shader_unified` 的 location 0..4 逐字段对应）。
/// `vertex_unify.rs` 里另有一条同样的单元测试 —— 两处都留着是刻意的：
/// 这条守「测试侧的字面数字」，那条守「实现侧的结构」。
#[test]
fn unified_vertex_layout_matches_the_unified_attribute_offsets() {
    use deer_vk::UnifiedVertex;
    assert_eq!(std::mem::size_of::<UnifiedVertex>(), 52, "stride");
    assert_eq!(std::mem::align_of::<UnifiedVertex>(), 4, "全是 f32");
    assert_eq!(std::mem::offset_of!(UnifiedVertex, pos), 0, "location 0：vec2 pos");
    assert_eq!(std::mem::offset_of!(UnifiedVertex, rect), 8, "location 1：vec4 rect");
    assert_eq!(
        std::mem::offset_of!(UnifiedVertex, radius_kind),
        24,
        "location 2：float radius_kind"
    );
    assert_eq!(std::mem::offset_of!(UnifiedVertex, color), 28, "location 3：vec4 color");
    assert_eq!(
        std::mem::offset_of!(UnifiedVertex, uv),
        44,
        "location 4：vec2 uv（判别符：形状段 = (-1,-1)）"
    );
    // 判别符常量本身（形状段的约定值）
    assert_eq!(deer_vk::SHAPE_UV_SENTINEL, [-1.0, -1.0]);
}

/// **R1-1（语义经 M3+ B3 收紧后的版本）**：**真的上传了**的帧才发 host→vertex 屏障。
///
/// 为什么要有这条（而不是只靠 `vertex_barrier_params_pin_the_exact_masks` 那个单元测试）：
/// 单元测试只能钉「参数对不对」，钉不住「屏障有没有真的被发出来」。reviewer 的变异证明
/// **把整段屏障删掉**（= 原始缺陷复原）在 16 个测试靶上**全绿** —— 本测试就是那条变异的判据：
/// 删掉屏障 ⇒ 计数不再增长 ⇒ **变红**。
///
/// ## 与 M3a 版本的差别（B3 引入，**如实标注**）
///
/// B3 之前是「有顶点就每帧重传、因此每帧都发屏障」；B3 起**内容不变的帧跳过上传**
/// ⇒ 没有主机写入 ⇒ **不需要**新的依赖（上一次那条已经给同一块缓冲建立过）。
/// 所以语义从「有顶点就发」收紧为「**上传了**才发」——老断言「每帧都要发屏障」
/// 在 B3 之后不再成立，这里按新语义重写（而不是把它删掉）。
#[test]
fn host_to_vertex_barrier_is_emitted_only_when_vertices_are_uploaded() {
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

    // 有顶点且**首次上传** ⇒ 恰好 +1
    let mut l = DrawList::new();
    l.push(DrawCmd::FillRect {
        rect: RectI::new(1, 1, 4, 4),
        color: Color::WHITE,
    });
    let s0 = r.render_stats();
    r.render(&l).expect("有顶点的帧应当成功");
    assert_eq!(
        r.host_to_vertex_barrier_count(),
        base + 1,
        "首次上传必须发**且只发一条** host→vertex 屏障"
    );
    assert_eq!(
        r.render_stats().buffer_uploads - s0.buffer_uploads,
        1,
        "首帧必须真的上传一次"
    );

    // **同一份顶点再画一帧**：B3 跳过上传 ⇒ 不发新屏障（没有新的主机写入）
    let s1 = r.render_stats();
    r.render(&l).expect("第二帧应当成功");
    assert_eq!(
        r.render_stats().buffer_uploads - s1.buffer_uploads,
        0,
        "B3：内容逐字节相同 ⇒ 跳过重传（buffer_uploads 不增长）"
    );
    assert_eq!(
        r.host_to_vertex_barrier_count(),
        base + 1,
        "没有上传就不需要新的 host→vertex 依赖（上一次那条仍然有效）"
    );

    // **内容变了** ⇒ 重新上传 ⇒ 屏障再次出现
    l.push(DrawCmd::FillRect {
        rect: RectI::new(2, 2, 4, 4),
        color: Color::WHITE,
    });
    let s2 = r.render_stats();
    r.render(&l).expect("内容变化后应当成功");
    assert_eq!(
        r.render_stats().buffer_uploads - s2.buffer_uploads,
        1,
        "内容变化 ⇒ 必须重新上传"
    );
    assert_eq!(
        r.host_to_vertex_barrier_count(),
        base + 2,
        "重新上传 ⇒ 必须重新建立 host→vertex 依赖"
    );
}

/// **M3+ B2（合段）**：相邻同管线**且区间连续**的绘制段必须合成**一次** `vkCmdDraw`。
///
/// 语料：5 个相邻的 `FillRect` ⇒ 形状顶点 30 个**连续**。
///
/// | 量 | 合段前 | 合段后（本测试断言） |
/// |---|---|---|
/// | `draw_calls` | 5 | **1** |
/// | `pipeline_switches` | 1 | **1**（切换 = 连续段数，本来就 1） |
///
/// 同时断言**像素判据不变**（与 CPU 逐字节比）：合段若改变了绘制顺序，这条会先红
/// —— 顺序即语义（M3b 已实证 z 序能被变异抓住）。
///
/// **计数是累计值** ⇒ 断言一律用差值。
///
/// ⚠️ **B5-2 之后这条用例的判别力变了（如实登记）**：整帧恒为「1 draw + 1 switch」
/// （见 [`unified_pipeline_costs_one_draw_and_one_switch`]），所以它不再能分辨
/// 「合段有没有生效」——「合段」在统一之后已无调用点。保留它是因为**像素判据**仍然有效
/// （与 CPU 逐字节相同），而「计数 = 1」这条现在由统一管线保证。
#[test]
fn adjacent_same_pipeline_segments_are_merged_into_one_draw_call() {
    let extent = Extent {
        width: 64,
        height: 32,
    };
    let Some(mut r) = renderer(extent) else {
        return;
    };

    let mut l = DrawList::new();
    for x in [2, 12, 22, 32, 42] {
        l.push(DrawCmd::FillRect {
            rect: RectI::new(x, 2, 8, 8),
            color: Color::WHITE,
        });
    }
    let before = r.render_stats();
    r.render(&l).expect("渲染 5 个相邻矩形");
    let after = r.render_stats();
    assert_eq!(
        after.draw_calls - before.draw_calls,
        1,
        "5 个相邻同管线矩形 ⇒ 合段后应当只有 **1** 次 draw（合段前是 5），实际 {}",
        after.draw_calls - before.draw_calls
    );
    assert_eq!(
        after.pipeline_switches - before.pipeline_switches,
        1,
        "切换次数 = 形状/文本的连续段数（由 z 序决定），这里是 1"
    );
    // 像素判据：逐字节不变
    assert_eq!(compare(&mut r, "merged-5-rects-vs-cpu", &l, 0), 0);
}

/// **B5-2 的终局判据（离屏）**：形状 + 文本合流后整帧 **1 draw + 1 switch**。
///
/// | 量 | 基线（两条管线） | 统一后（本测试断言） |
/// |---|---|---|
/// | `draw_calls` | 8（段数） | **1** |
/// | `pipeline_switches` | 8 | **1** |
///
/// 语料刻意让形状与文本**在像素上重叠**且**交替**（形状 → 文本 → 形状），
/// 于是「为了少一次 draw 而重排顺序」会**同时**在计数与像素上露馅。
///
/// ## 前置条件（**显式断言**，本项目纪律）
///
/// - 渲染器必须**真的接上了文本引擎**（`text_enabled()`），否则文本整段进 `unsupported`
///   ⇒ 语料退化成「只有形状」，这条用例就变成了空转（前任踩过「变异后仍绿」的假护栏）；
/// - 文本必须**真的产出了顶点**（`unify_output_vertex_count` 的增量 > 形状段顶点数）
///   —— 空串/被裁空的文本会让「1 draw」平凡成立。
#[test]
fn unified_pipeline_costs_one_draw_and_one_switch() {
    let extent = Extent {
        width: 96,
        height: 40,
    };
    let Some(mut pair) = text_pair(extent, 20.0) else {
        return;
    };
    // ★ 前置条件 ①：文本引擎真的接上了
    assert!(
        pair.0.text_enabled(),
        "前置条件：这条用例必须跑在 `with_text` 之后的渲染器上（否则文本走 Unsupported，用例空转）"
    );

    let mut l = DrawList::new();
    l.push(DrawCmd::FillRect {
        rect: RectI::new(0, 0, 96, 40),
        color: Color::rgb(20, 60, 120),
    });
    l.push(text_cmd("Wg", RectI::new(4, 4, 88, 32), 20.0, 0));
    l.push(DrawCmd::FillRect {
        rect: RectI::new(10, 6, 40, 20),
        color: Color::rgb(200, 30, 30),
    });
    let before = pair.0.render_stats();
    let v_before = pair.0.unify_output_vertex_count();
    let c_before = pair.0.unify_call_count();
    pair.0.render(&l).expect("交错帧");
    let after = pair.0.render_stats();
    let verts = pair.0.unify_output_vertex_count() - v_before;
    println!(
        "  unified-1-draw: draw_calls {} / pipeline_switches {} / buffer_uploads {} / \
         unify 调用 {} 次、输出顶点 {verts}（= CPU 转换 O(N) 口径）",
        after.draw_calls - before.draw_calls,
        after.pipeline_switches - before.pipeline_switches,
        after.buffer_uploads - before.buffer_uploads,
        pair.0.unify_call_count() - c_before,
    );

    // ★ 前置条件 ②：三种图元都真的产出了顶点（形状 6 + 文本 N + 形状 6 > 12）
    assert!(
        verts > 12,
        "前置条件：这一帧必须同时含形状段与**非空**文本段（实测统一顶点数 {verts}）；\
         若文本被跳过（空串/被裁空），这条用例对「1 draw」而言是空转"
    );
    // ★ 前置条件 ③：CPU 转换恰好被调用一次/帧
    assert_eq!(
        pair.0.unify_call_count() - c_before,
        1,
        "每帧恰好一次 `unify`（CPU 成本口径）"
    );

    assert_eq!(
        after.draw_calls - before.draw_calls,
        1,
        "统一管线：整帧**一次** `vkCmdDraw(0, 全部顶点数)`（基线是段数 = 8）"
    );
    assert_eq!(
        after.pipeline_switches - before.pipeline_switches,
        1,
        "统一管线：整帧**一次** `vkCmdBindPipeline`（基线是 8）"
    );
    assert_eq!(
        after.buffer_uploads - before.buffer_uploads,
        1,
        "统一管线：只有**一块**顶点缓冲 ⇒ 一次上传（基线是两块各一次）"
    );
    // 像素判据：与 CPU 逐字节相同（z 序没被重排）
    assert_eq!(
        compare_text(&mut pair, "unified-1-draw-vs-cpu", &l, 0),
        0,
        "统一之后像素必须**一字不变**"
    );
}

/// **哑纹理必须**绑**着，且形状帧的像素仍然对**（B5-2 的隐式依赖，见 `gpu_render::DUMMY_COVERAGE`）。
///
/// 统一片元着色器**无条件采样** ⇒ 形状帧也必须绑一张有效纹理。本用例断言两件事：
///
/// 1. **没有 `TextEngine`** 的渲染器画形状帧时，`set 0` 指着的是 **1×1 哑纹理**
///    （`bound_texture_size()`，读的是**描述符真实指向**）—— 「反正不读」的假设不成立，这条依赖必须在；
/// 2. 图形像素仍然与 CPU **逐字节相同**。
///
/// ## ⚠️ 承重的是**绑定**，不是哑纹理的**取值**（B5-3 更正，之前这里写反了）
///
/// 早先这段文档写着「承重性由变异 **B**（哑纹理换成 `cov=0`）证明：形状帧必须变红」——
/// **与实测相反**：`[0]`、`[128]` 两种取值下本用例仍然**逐字节相同**（形状那一支的结果被
/// `OpSelect` 丢弃，采样值影响不了任何像素）。
///
/// 真正的承重判据是**本用例本身**（要求形状帧逐字节 0）在下面这两种变异下变红：
///
/// | 变异 | 默认档 | 校验层档 |
/// |---|---|---|
/// | 删掉录制里的 `cmd_bind_descriptor_sets` | **18 failed / 9 passed**（形状帧读回**整幅 0**、差 255） | `VUID-vkCmdDraw-None-08600`（statically uses set n 但没绑） |
/// | 跳过 `update_descriptor_texture`（集从不写入） | **9 failed** | `VUID-vkCmdDraw-None-08114` |
///
/// 注意表现是「**读回整幅全 0**」（连清屏色都没了 ⇒ 整条 draw 未定义），
/// **不是**「形状的颜色不对」—— 后者会让人误以为是着色器判据的问题。
#[test]
fn a_shape_only_frame_binds_the_one_by_one_dummy_texture() {
    let extent = Extent {
        width: 32,
        height: 24,
    };
    let Some(mut r) = renderer(extent) else {
        return;
    };
    // ★ 前置条件：这个渲染器**没有**文本引擎（否则下面绑的是字形图集，用例空转）
    assert!(
        !r.text_enabled(),
        "前置条件：本用例必须在**没有 TextEngine** 的渲染器上跑"
    );
    assert_eq!(
        r.bound_texture_size(),
        (1, 1),
        "没有 TextEngine ⇒ `set 0` 必须绑 1×1 哑纹理（统一 FS 无条件采样）"
    );

    let mut l = DrawList::new();
    l.push(DrawCmd::FillRect {
        rect: RectI::new(2, 2, 28, 20),
        color: Color::rgb(180, 60, 20),
    });
    assert_eq!(compare(&mut r, "shape-only-with-dummy-texture", &l, 0), 0);
    // 画完之后**仍然是** 1×1（没有文本 ⇒ 没有图集上传把它挤掉）
    assert_eq!(
        r.bound_texture_size(),
        (1, 1),
        "形状帧不得把哑纹理换成别的东西（那样 `set 0` 会指向未上传的图集）"
    );
}

/// **B2 的「不合并」那一半（B5-2 更新后的语义）**：交错语料在统一之后是 **1 draw + 1 switch**，
/// 但**顺序仍然是语义**（z 序不能为了少画而重排）。
///
/// ## 这条用例为什么从「3/3」改成「1/1」（**如实登记，不是把断言改松了**）
///
/// 从前形状与文本是两条管线 ⇒ `矩形, 文本, 矩形` 必须切管线 3 次、发 3 次 draw。
/// B5-2 把两者合成**一条**管线 + **一条**顶点流 ⇒ 段数不再决定 draw 次数。
/// 「不许为了少一次 draw 而重排」这条**不变式仍然有效**，但它现在由
/// **像素**守护（`compare_text` 逐字节）而不是计数：
/// 语料刻意让形状与文本**在像素上重叠**（矩形盖住文字），重排会红 255。
#[test]
fn interleaved_shapes_and_text_are_not_reordered_by_merging() {
    let extent = Extent {
        width: 96,
        height: 32,
    };
    let Some(mut pair) = text_pair(extent, 16.0) else {
        return;
    };
    let mut l = DrawList::new();
    l.push(DrawCmd::FillRect {
        rect: RectI::new(2, 2, 8, 8),
        color: Color::WHITE,
    });
    l.push(DrawCmd::Text {
        rect: RectI::new(2, 12, 90, 18),
        text: "M".into(),
        color: Color::WHITE,
        size: 16.0,
        align: 0,
    });
    l.push(DrawCmd::FillRect {
        rect: RectI::new(40, 2, 8, 8),
        color: Color::WHITE,
    });
    let before = pair.0.render_stats();
    let gpu = pair.0.render(&l).expect("渲染 矩形/文本/矩形");
    let after = pair.0.render_stats();
    assert_eq!(
        after.draw_calls - before.draw_calls,
        1,
        "统一管线：三段合流 ⇒ **一次** draw（B5-2 之前是 3）"
    );
    assert_eq!(
        after.pipeline_switches - before.pipeline_switches,
        1,
        "统一管线：只有一条管线 ⇒ **一次** bind（B5-2 之前是 3）"
    );
    let _ = gpu;
    // 像素判据：与 CPU 逐字节相同。
    //
    // ⚠️ **诚实说明（review 覆盖漏洞，仍然有效）**：本用例的矩形在 `y∈[2,10)`、文本在 `y∈[12,30)`，
    // **两者在像素上不重叠** ⇒ 就算实现把「形状全部提前、文本全部置后」这种重排做出来，
    // 这里的像素**也可能全绿** —— 所以本用例对 z 序而言**实质是计数护栏**
    // （B5-2 之后连计数也不再分辨重排了 ⇒ 它现在**只是像素护栏的一个弱例**）。
    // **真正的像素级 z 序护栏是 `overlapping_shape_after_text_is_detected_by_pixels_*`
    //   与 `unified_pipeline_costs_one_draw_and_one_switch`（后者的语料是重叠的）。**
    assert_eq!(compare_text(&mut pair, "interleaved-not-reordered", &l, 0), 0);
}

/// **z 序的像素级护栏（review 覆盖漏洞的补丁）**：形状与文本在**像素上重叠**，形状后画。
///
/// 为什么必须补：上面那条交错用例的矩形与文本**不重叠** ⇒ 「允许重排的合段」这类变异
/// 只在计数上露馅、**像素全绿**（reviewer 用重叠语料做探针，红了 255）。
/// 本用例让**不透明矩形完整盖住文本**：
/// - 顺序 = 文本先、矩形后 ⇒ 文本必须被完全盖住；
/// - 顺序 = 矩形先、文本后 ⇒ 文本必须可见（两条都查，避免「永远盖住」的假实现）。
///
/// 任何「为了少一次 draw 而重排绘制顺序」的实现都会在这里的像素上红（RGB 差可达 255）。
#[test]
fn overlapping_shape_after_text_is_detected_by_pixels_not_only_counts() {
    let extent = Extent {
        width: 96,
        height: 40,
    };
    let Some(mut pair) = text_pair(extent, 20.0) else {
        return;
    };
    // ① 文本先画
    let mut l = DrawList::new();
    l.push(text_cmd("Wg", RectI::new(4, 4, 88, 32), 20.0, 0));
    // ② **同一块区域**再盖一个不透明矩形 ⇒ 文本必须被完全盖住
    l.push(DrawCmd::FillRect {
        rect: RectI::new(0, 0, 96, 40),
        color: Color::rgb(200, 30, 30),
    });
    assert_eq!(compare_text(&mut pair, "z-order-overlap-shape-wins", &l, 0), 0);

    // 反向顺序：矩形先、文本后 ⇒ 文本必须可见
    let mut l2 = DrawList::new();
    l2.push(DrawCmd::FillRect {
        rect: RectI::new(0, 0, 96, 40),
        color: Color::rgb(200, 30, 30),
    });
    l2.push(text_cmd("Wg", RectI::new(4, 4, 88, 32), 20.0, 0));
    assert_eq!(compare_text(&mut pair, "z-order-overlap-text-wins", &l2, 0), 0);
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

// ===========================================================================
// M3b-T4：**文本**的逐像素对照（GPU ↔ CPU）
//
// 三条硬前提（都是 T3 / M3a 的实证结论）：
// ① **CPU 侧必须用 `CpuRenderer::with_text(engine)`** —— `CpuRenderer::new()` 走的是
//    **占位格**模型（0.6em 等宽方块 + i32 截断除法），与真实字形**模型不同**，
//    误用它会让文本对照全红且极难定位；
// ② 片元着色器输出**非预乘** `vec4(rgb, a*cov)`，配管线的 `SRC_ALPHA/ONE_MINUS_SRC_ALPHA`
//    —— 这正是 CPU `blend_cov` 的形式（顶点颜色也是非预乘的）；
// ③ 采样 **NEAREST**：线性过滤会把邻居纹素混进来，与 CPU 的整数查表不一致。
// ===========================================================================

/// 系统字体引擎（拿不到就跳过并打印原因）。
fn text_engine(font_size: f32) -> Option<deer_gpu::text::TextEngine> {
    match deer_gpu::text::TextEngine::from_system_font(font_size) {
        Ok(e) => Some(e),
        Err(e) => {
            eprintln!("跳过：这台机器上拿不到系统字体（{e}）");
            None
        }
    }
}

/// GPU（文本管线）+ CPU（真实字形路径）这一对，**各自持有一个同源引擎**。
///
/// 两个引擎由同一个系统字体、按**完全相同的命令顺序**填充图集 ⇒ 槽位布局逐字节相同
/// （`atlas.rs` 的确定性不变式），所以两边的像素可直接比较。
type TextPair = (GpuGeometryRenderer, CpuRenderer);

fn text_pair(extent: Extent, font_size: f32) -> Option<TextPair> {
    let base = renderer(extent)?;
    let e_gpu = text_engine(font_size)?;
    let e_cpu = text_engine(font_size)?;
    let gpu = base
        .with_text(e_gpu)
        .unwrap_or_else(|e| panic!("with_text 失败（文本管线建不起来）：{e}"));
    // ★ 必须 `with_text`：占位格模型与真实字形对不上（见本节开头的 ①）
    Some((gpu, CpuRenderer::with_text(e_cpu)))
}

/// 对照一帧**文本**：返回最大通道差（`max_allowed = 0` ⇒ 额外断言逐字节相同）。
fn compare_text(pair: &mut TextPair, name: &str, list: &DrawList, max_allowed: u8) -> u8 {
    let extent = pair.0.extent();
    let gpu = pair
        .0
        .render(list)
        .unwrap_or_else(|e| panic!("{name}: GPU 渲染失败：{e}"));
    assert!(
        pair.0.unsupported().is_empty(),
        "{name}: 文本已被接管 ⇒ 不该有 unsupported（{:?}）",
        pair.0.unsupported()
    );
    let cpu = pair
        .1
        .render(extent, list, CLEAR)
        .expect("CPU 渲染失败");
    let cpu = cpu.to_rgba();
    assert_eq!(gpu.len(), cpu.len(), "{name}: 回读长度必须等于 CPU 帧缓冲长度");

    let mut worst = 0u8;
    let mut at = 0usize;
    for (i, (g, c)) in gpu.iter().zip(cpu.iter()).enumerate() {
        let d = g.abs_diff(*c);
        if d > worst {
            worst = d;
            at = i;
        }
    }
    let w = extent.width.max(1) as usize;
    println!("  {name}: 最大通道差 {worst}（允许 {max_allowed}）");
    assert!(
        worst <= max_allowed,
        "{name}: 最大通道差 {worst} > 允许的 {max_allowed}；最差处像素 ({}, {})：GPU={:?} CPU={:?}",
        (at / 4) % w,
        (at / 4) / w,
        &gpu[at - at % 4..at - at % 4 + 4],
        &cpu[at - at % 4..at - at % 4 + 4]
    );
    if max_allowed == 0 {
        assert_eq!(gpu, cpu, "{name}: 不透明文本必须逐字节相同");
    }
    worst
}

/// 一条文本命令（不透明白字，便于与清屏色区分）。
fn text_cmd(text: &str, rect: RectI, size: f32, align: u8) -> DrawCmd {
    DrawCmd::Text {
        rect,
        text: text.to_string(),
        color: Color::WHITE,
        size,
        align,
    }
}

/// **文本对照的语料**：单字符 / 多字符 / 超大 size / `align=0,1,2` / 空串 / 零面积 /
/// 被 clip 裁空 / 缺字（豆腐）/ 形状与文本交错（z 序）。
#[test]
fn text_drawings_match_cpu_pixel_for_pixel() {
    let extent = Extent {
        width: 128,
        height: 48,
    };
    let Some(mut pair) = text_pair(extent, 20.0) else {
        return;
    };
    let mut worst = 0u8;

    // ① 单字符 / ② 多字符
    let mut l = DrawList::new();
    l.push(text_cmd("A", RectI::new(4, 4, 60, 30), 24.0, 0));
    worst = worst.max(compare_text(&mut pair, "text-single", &l, 0));

    let mut l = DrawList::new();
    l.push(text_cmd("Hello, GPU!", RectI::new(2, 2, 124, 40), 20.0, 0));
    worst = worst.max(compare_text(&mut pair, "text-multi", &l, 0));

    // ③ 三种对齐（align=1/2 在 T3 才第一次有覆盖）
    for align in [0u8, 1, 2] {
        let mut l = DrawList::new();
        l.push(text_cmd("Align", RectI::new(4, 4, 110, 36), 20.0, align));
        worst = worst.max(compare_text(
            &mut pair,
            &format!("text-align-{align}"),
            &l,
            0,
        ));
    }

    // ④ 超大 size：放不进图集 ⇒ 两边都什么都不画（不该报错）
    let mut l = DrawList::new();
    l.push(text_cmd("W", RectI::new(0, 0, 128, 48), 1500.0, 0));
    worst = worst.max(compare_text(&mut pair, "text-huge-size", &l, 0));

    // ⑤ 空串 / ⑥ 零面积 rect（CPU 侧仍会画：rect 只决定起点与基线）
    let mut l = DrawList::new();
    l.push(text_cmd("", RectI::new(4, 4, 60, 30), 20.0, 0));
    worst = worst.max(compare_text(&mut pair, "text-empty", &l, 0));

    let mut l = DrawList::new();
    l.push(text_cmd("L", RectI::new(6, 6, 0, 30), 20.0, 0));
    worst = worst.max(compare_text(&mut pair, "text-zero-area", &l, 0));

    // ⑦ 被 clip 完全裁空 / ⑧ 局部裁剪
    let mut l = DrawList::new();
    l.push(DrawCmd::PushClip {
        rect: RectI::new(200, 200, 8, 8),
    });
    l.push(text_cmd("Clipped", RectI::new(4, 4, 110, 36), 20.0, 0));
    l.push(DrawCmd::PopClip);
    worst = worst.max(compare_text(&mut pair, "text-clipped-away", &l, 0));

    let mut l = DrawList::new();
    l.push(DrawCmd::PushClip {
        rect: RectI::new(20, 8, 60, 32),
    });
    l.push(text_cmd("Clipped", RectI::new(4, 4, 110, 36), 20.0, 0));
    l.push(DrawCmd::PopClip);
    worst = worst.max(compare_text(&mut pair, "text-clipped-partial", &l, 0));

    // ⑨ 缺字（私用区字符 ⇒ 走 `.notdef` 豆腐）
    let mut l = DrawList::new();
    l.push(text_cmd("A\u{E123}B", RectI::new(4, 4, 110, 36), 20.0, 0));
    worst = worst.max(compare_text(&mut pair, "text-missing-glyph", &l, 0));

    println!("文本语料最大通道差 = {worst}（要求 0）");
    assert_eq!(worst, 0, "不透明文本必须逐字节相同");
    assert_no_validation_messages("文本语料");
}

/// **z 序**：文本与形状按 `DrawList` 顺序交错，后画的必须盖住先画的。
///
/// 这是本任务最容易做错的一条：如果把绘制段排成「先所有形状、再所有文本」，
/// 下面 `text-under-shape` 那条会红（矩形本该盖住文字），而单看文本对照是看不出来的。
#[test]
fn text_and_shapes_keep_z_order() {
    let extent = Extent {
        width: 128,
        height: 48,
    };
    let Some(mut pair) = text_pair(extent, 20.0) else {
        return;
    };

    // 文字在形状**之下**：矩形后画 ⇒ 必须把文字盖住
    let mut l = DrawList::new();
    l.push(text_cmd("Covered", RectI::new(4, 4, 110, 36), 20.0, 0));
    l.push(DrawCmd::FillRect {
        rect: RectI::new(0, 0, 60, 48),
        color: Color::rgb(200, 30, 30),
    });
    assert_eq!(compare_text(&mut pair, "text-under-shape", &l, 0), 0);

    // 文字在形状**之上**
    let mut l = DrawList::new();
    l.push(DrawCmd::FillRect {
        rect: RectI::new(0, 0, 128, 48),
        color: Color::rgb(20, 60, 120),
    });
    l.push(text_cmd("Over", RectI::new(4, 4, 110, 36), 20.0, 0));
    assert_eq!(compare_text(&mut pair, "text-over-shape", &l, 0), 0);

    // 交替：矩形 → 文字 → 矩形 → 文字（形状与文本各两段，z 序交错四处）
    let mut l = DrawList::new();
    l.push(DrawCmd::FillRect {
        rect: RectI::new(0, 0, 128, 24),
        color: Color::rgb(20, 60, 120),
    });
    l.push(text_cmd("First", RectI::new(2, 0, 60, 24), 18.0, 0));
    l.push(DrawCmd::FillRect {
        rect: RectI::new(0, 20, 128, 28),
        color: Color::rgb(200, 30, 30),
    });
    l.push(text_cmd("Second", RectI::new(2, 20, 124, 28), 18.0, 0));
    assert_eq!(compare_text(&mut pair, "text-interleaved", &l, 0), 0);
}

/// 半透明文本：与 M3a 同样的口径 —— **≤1 LSB**（CPU `round()` vs GPU UNORM 舍入）。
///
/// review M4 要求「不止一例」，所以这里给三例，覆盖不同的混合底色：
/// ① 纯背景上 α=0.5；② 形状之上 α=0.5（**文本要按形状混合过的底色再混一次**）；
/// ③ α=0.25（权重更小，对舍入更敏感）。
#[test]
fn semi_transparent_text_matches_cpu_within_one_lsb() {
    let extent = Extent {
        width: 128,
        height: 48,
    };
    let Some(mut pair) = text_pair(extent, 20.0) else {
        return;
    };
    let alpha_text = |a: f32| DrawCmd::Text {
        rect: RectI::new(4, 4, 110, 36),
        text: "Fade".into(),
        color: Color::rgba(255, 80, 0, a),
        size: 20.0,
        align: 0,
    };

    // ① 背景 + α=0.5
    let mut l = DrawList::new();
    l.push(alpha_text(0.5));
    let worst1 = compare_text(&mut pair, "text-semi-transparent-0.5", &l, 1);

    // ② 形状之上 α=0.5（嵌套混合：先不透明形状、再半透明文本）
    let mut l = DrawList::new();
    l.push(DrawCmd::FillRect {
        rect: RectI::new(0, 0, 128, 48),
        color: Color::rgb(40, 90, 160),
    });
    l.push(alpha_text(0.5));
    let worst2 = compare_text(&mut pair, "text-semi-over-shape", &l, 1);

    // ③ 背景 + α=0.25
    let mut l = DrawList::new();
    l.push(alpha_text(0.25));
    let worst3 = compare_text(&mut pair, "text-semi-transparent-0.25", &l, 1);

    println!("半透明文本最大通道差：{worst1} / {worst2} / {worst3}（要求 ≤1）");
    for (name, w) in [("0.5", worst1), ("over-shape", worst2), ("0.25", worst3)] {
        assert!(w <= 1, "半透明文本（{name}）最大通道差 {w} 超过 1 LSB");
    }
}

/// **嵌套裁剪 × 文本**（review M4）：两层/三层 `PushClip` 相交之后才轮到文本。
///
/// 覆盖的是「裁剪栈被逐命令翻译拆开之后，**多层**求交是否仍与整份列表一致」——
/// 单层裁剪已有用例，多层是另一条路径（`full ∩ r1 ∩ r2 [∩ r3]`），而且第 ② 例是
/// 「求交**之后**才为空」这种假阳性（单层裁空已有用例，求交为空是新的一类）。
#[test]
fn nested_clips_around_text_match_cpu() {
    let extent = Extent {
        width: 128,
        height: 48,
    };
    let Some(mut pair) = text_pair(extent, 20.0) else {
        return;
    };

    // ① 两层相交后仍留出文本的一部分
    let mut l = DrawList::new();
    l.push(DrawCmd::PushClip {
        rect: RectI::new(8, 4, 100, 40),
    });
    l.push(DrawCmd::PushClip {
        rect: RectI::new(24, 10, 60, 28),
    });
    l.push(text_cmd("Nested", RectI::new(4, 4, 110, 36), 20.0, 0));
    l.push(DrawCmd::PopClip);
    l.push(DrawCmd::PopClip);
    assert_eq!(compare_text(&mut pair, "text-nested-clip", &l, 0), 0);

    // ② 两层**求交之后为空** ⇒ 跳过 + 计数，不报错
    let mut l = DrawList::new();
    l.push(DrawCmd::PushClip {
        rect: RectI::new(0, 0, 20, 20),
    });
    l.push(DrawCmd::PushClip {
        rect: RectI::new(60, 30, 40, 18), // 与上一层不相交
    });
    l.push(text_cmd("Nested", RectI::new(4, 4, 110, 36), 20.0, 0));
    l.push(DrawCmd::PopClip);
    l.push(DrawCmd::PopClip);
    let before = pair.0.text_skipped();
    let gpu = pair.0.render(&l).expect("两层裁剪求交为空 ⇒ 跳过，不报错");
    assert!(
        pair.0.unsupported().is_empty(),
        "求交为空的文本不该进 unsupported"
    );
    assert_eq!(
        pair.0.text_skipped(),
        before + 1,
        "求交为空的那条文本应被计入 skipped"
    );
    let clear = [CLEAR.r, CLEAR.g, CLEAR.b, 255];
    assert!(
        gpu.chunks_exact(4).all(|px| px == clear),
        "求交为空 ⇒ 帧里不该有被画过的像素"
    );

    // ③ 三层，且形状与文本都在嵌套裁剪里（顺带确认形状路径的裁剪栈不受拆分影响）
    let mut l = DrawList::new();
    l.push(DrawCmd::PushClip {
        rect: RectI::new(4, 2, 120, 44),
    });
    l.push(DrawCmd::PushClip {
        rect: RectI::new(10, 6, 110, 38),
    });
    l.push(DrawCmd::PushClip {
        rect: RectI::new(16, 10, 96, 30),
    });
    l.push(DrawCmd::FillRect {
        rect: RectI::new(0, 0, 128, 48),
        color: Color::rgb(30, 30, 90),
    });
    l.push(text_cmd("Deep", RectI::new(4, 4, 110, 36), 20.0, 0));
    l.push(DrawCmd::PopClip);
    l.push(DrawCmd::PopClip);
    l.push(DrawCmd::PopClip);
    assert_eq!(compare_text(&mut pair, "text-nested-3-with-shape", &l, 0), 0);
}

/// **统一顶点缓冲的 host→vertex 屏障**（原 `text_barriers_are_emitted_per_buffer` 的 B5-2 版本）。
///
/// ## 为什么这条用例被改写（而不是删掉）—— **如实登记**
///
/// 原用例守的是「形状与文本**两块**顶点缓冲**各有**一条屏障」：交错帧断言总数 +2、
/// 两个分项各 +1。B5-2 把两块缓冲合成**一块** ⇒ 那个断言**不可能**再成立
/// （不是「太严」，而是它描述的结构已经不存在了）。所以这里按新结构重写：
/// **只有一块缓冲 ⇒ 一帧最多一条屏障**，而「总计数器」就是唯一的判据
/// （两个分项计数器已随字段一起删除，留两个恒等的数只会让人以为它们还分辨着什么）。
///
/// ## 覆盖的四种帧（每一种都断言确切增量）
///
/// | 帧 | 期望增量 |
/// |---|---|
/// | ① 文本帧（内容首次上传） | **+1** |
/// | ② 形状+文本交错、内容变了 | **+1**（不是 +2：只有一块缓冲） |
/// | ③ 同一份内容重复帧（B3 跳过上传） | **0** |
/// | ④ 文本全被跳过（空串，没有顶点） | **0** |
///
/// 删掉发射 ⇒ ①② 不再增长 ⇒ 红。
#[test]
fn unified_vertex_barrier_is_emitted_once_per_upload() {
    let extent = Extent {
        width: 64,
        height: 32,
    };
    let Some(mut pair) = text_pair(extent, 16.0) else {
        return;
    };
    assert!(
        pair.0.text_enabled(),
        "前置条件：本用例必须跑在真的接上文本引擎的渲染器上"
    );
    let count = |r: &GpuGeometryRenderer| r.host_to_vertex_barrier_count();
    let t0 = count(&pair.0);

    // ① 文本帧 ⇒ 恰好 +1
    let mut l = DrawList::new();
    l.push(text_cmd("Bar", RectI::new(2, 2, 60, 28), 16.0, 0));
    pair.0.render(&l).expect("文本帧");
    let t1 = count(&pair.0);
    assert_eq!(t1, t0 + 1, "文本帧应当发出 **1** 条屏障（只有一块统一缓冲）");

    // ② 形状+文本交错、**两者内容都变了** ⇒ 仍然只有 1 条
    //    （B5-2 之前这里是 +2：两块独立缓冲各一条）
    let mut l = DrawList::new();
    l.push(DrawCmd::FillRect {
        rect: RectI::new(0, 0, 64, 32),
        color: Color::rgb(10, 10, 40),
    });
    l.push(text_cmd("Baz", RectI::new(2, 2, 60, 28), 16.0, 0));
    pair.0.render(&l).expect("交错帧");
    let t2 = count(&pair.0);
    assert_eq!(
        t2,
        t1 + 1,
        "统一缓冲只有一块 ⇒ 交错帧也是 **1** 条（B5-2 之前是 2 条，那是两块缓冲的语义）"
    );

    // ③ 同一份内容再画一帧 ⇒ B3 跳过上传 ⇒ 不发新屏障
    let s = pair.0.render_stats();
    pair.0.render(&l).expect("重复帧");
    assert_eq!(
        pair.0.render_stats().buffer_uploads - s.buffer_uploads,
        0,
        "B3：内容逐字节相同 ⇒ 不重传"
    );
    assert_eq!(
        count(&pair.0),
        t2,
        "没有上传 ⇒ 不需要新的 host→vertex 依赖（B3 收紧后的语义）"
    );

    // ④ 文本全被跳过（没有顶点）⇒ 没有上传 ⇒ 不发屏障
    let mut l = DrawList::new();
    l.push(text_cmd("", RectI::new(2, 2, 60, 28), 16.0, 0));
    pair.0.render(&l).expect("空串帧");
    let t4 = count(&pair.0);
    assert_eq!(t4, t2, "没有顶点要画 ⇒ 屏障计数不该动（实测：{:?} vs {:?}）", t4, t2);
}

/// **假阳性已修**：空串 / `size <= 0` / 被裁空的文本**不再报错**，而是被跳过并计数
/// （M3a 里它们会让整帧报 `Unsupported`；M3b 只记 `text_skipped`）。
///
/// ⚠️ 这里**只断言「不报错 + 计数正确」**，不做像素对照：`size <= 0` 与 CPU **有意不同**
/// （CPU 把字号夹到 `>= 1` ⇒ 会画 1px 的字形；本模块按 M3b 计划跳过）——
/// **parity 语料不含 `size <= 0`** 是控制者裁定的硬约束。
/// 而「空串 / 被裁空」这两类是 CPU 也不画的，所以另有一条逐字节对照（下面第二个块）。
#[test]
fn text_false_positives_are_skipped_and_counted_not_errors() {
    let extent = Extent {
        width: 128,
        height: 48,
    };
    let Some(mut pair) = text_pair(extent, 20.0) else {
        return;
    };
    let mut l = DrawList::new();
    l.push(text_cmd("", RectI::new(4, 4, 60, 30), 20.0, 0)); // 空串
    l.push(text_cmd("Hi", RectI::new(4, 4, 60, 30), 0.0, 0)); // size = 0
    l.push(text_cmd("Hi", RectI::new(4, 4, 60, 30), -5.0, 0)); // size < 0
    l.push(DrawCmd::PushClip {
        rect: RectI::new(200, 200, 4, 4), // 裁空
    });
    l.push(text_cmd("Hi", RectI::new(4, 4, 60, 30), 20.0, 0));
    l.push(DrawCmd::PopClip);

    let gpu = pair.0.render(&l).expect("这些文本不该再让整帧报错");
    assert!(
        pair.0.unsupported().is_empty(),
        "文本已被接管 ⇒ 这些「画不出东西」的命令不该进 unsupported"
    );
    // 四条都被跳过 ⇒ 这一帧除了清屏色什么都没有
    let clear = [CLEAR.r, CLEAR.g, CLEAR.b, 255];
    let mut painted = 0usize;
    for px in gpu.chunks_exact(4) {
        if px != clear {
            painted += 1;
        }
    }
    assert_eq!(painted, 0, "四条全被跳过 ⇒ 帧里不该有任何被画过的像素");
    assert_eq!(
        pair.0.text_skipped(),
        4,
        "空串 / size=0 / size<0 / 被裁空 四条都应被计数（而不是报错）"
    );
    assert_no_validation_messages("文本假阳性");

    // 只有「CPU 也不画」的两类做像素对照 ⇒ 必须逐字节相同（含清屏色）
    let mut l = DrawList::new();
    l.push(text_cmd("", RectI::new(4, 4, 60, 30), 20.0, 0));
    l.push(DrawCmd::PushClip {
        rect: RectI::new(200, 200, 4, 4),
    });
    l.push(text_cmd("Hi", RectI::new(4, 4, 60, 30), 20.0, 0));
    l.push(DrawCmd::PopClip);
    assert_eq!(
        compare_text(&mut pair, "text-empty-and-clipped-away", &l, 0),
        0
    );
}

/// 连续多帧：同一渲染器反复渲染不同文本 —— 图集增长时纹理**只在变化时重传**，
/// 且每帧结果仍与 CPU 一致（覆盖「图集指纹」那条路径）。
#[test]
fn consecutive_text_frames_stay_in_sync() {
    let extent = Extent {
        width: 128,
        height: 48,
    };
    let Some(mut pair) = text_pair(extent, 20.0) else {
        return;
    };
    for (i, text) in ["one", "two", "three", "one"].iter().enumerate() {
        let mut l = DrawList::new();
        l.push(text_cmd(text, RectI::new(4, 4, 120, 36), 20.0, 0));
        assert_eq!(
            compare_text(&mut pair, &format!("text-frame-{i}-{text}"), &l, 0),
            0
        );
    }
}

/// **M-4 护栏**：同一图集连续多帧 ⇒ 纹理**只上传一次**（不重传）。
///
/// ## 为什么必须有这条（reviewer 实测的护栏缺口）
///
/// `refresh_atlas_texture()` 里有一个指纹判断（`(宽, 高, 已光栅化字形数)`）——
/// 「图集没变就不重传」。reviewer 把这个判断**删掉、改成每帧重传**之后，
/// `consecutive_text_frames_stay_in_sync` **仍然全绿**：那个测试只看最终像素，
/// 而重传几次对像素没有影响。
///
/// 也就是说「只在指纹变化时重传」这条**性能前提**此前只有实现、没有护栏 ——
/// 删掉实现测试照样绿。这条测试补上护栏：用
/// [`deer_vk::device::texture_r8_upload_count()`]（**按线程**计数，在被调用方自增）
/// 的**差值**断言「同一图集不应触发上传」。
///
/// ## 为什么是「先渲染一次收干，再连续两次同文本」
///
/// 第一次渲染必然上传（图集从无到有）。之后**第三次**渲染前，图集里已经含有了
/// 这次要用的全部字形 ⇒ 如果指纹判断在，这几次的上传次数必须为 **0**；
/// 一旦有人删掉指纹、改成每帧重传，次数就会变成 2 ⇒ 立刻红。
///
/// 用同一段文本（没有新字形）而不是不同文本，是为了让「应有 0 次上传」这个
/// 结论不依赖「哪些字形已在图集里」的推断 —— 文本完全相同就没有新字形，
/// 这是最不容易随实现变化的判据。
#[test]
fn repeated_frames_with_unchanged_atlas_do_not_reupload_texture() {
    let extent = Extent {
        width: 128,
        height: 48,
    };
    let Some(mut pair) = text_pair(extent, 20.0) else {
        return;
    };

    // 固定一段含多个字形的文本；三次渲染用的是**完全相同**的绘制列表。
    let text = "atlas";
    let mut l = DrawList::new();
    l.push(text_cmd(text, RectI::new(4, 4, 120, 36), 20.0, 0));

    // ① 首次渲染：图集从无到有（会先把所有字形光栅化进图集，然后上传一次）
    assert_eq!(compare_text(&mut pair, "atlas-warmup", &l, 0), 0);

    // ② 之后再渲染两次：文本相同 ⇒ 没有新字形 ⇒ 图集未变 ⇒ **零**上传
    for i in 0..2 {
        let before = deer_vk::device::texture_r8_upload_count();
        assert_eq!(compare_text(&mut pair, &format!("atlas-repeat-{i}"), &l, 0), 0);
        let after = deer_vk::device::texture_r8_upload_count();
        assert_eq!(
            after - before,
            0,
            "第 {i} 次重复渲染触发了 {} 次纹理上传 —— \
             「图集未变则不重传」这条前提被破坏了（每帧重传会让 vkQueueWaitIdle \
             卡在每一帧上）。注意：这条断言就是它的护栏，删掉指纹判断必须让它变红。",
            after - before
        );
    }

    println!("同一图集连续 3 帧：只上传 1 次（后续 2 帧零重传）✅");
}

/// **并行安全**：别的线程在疯狂上传纹理时，本线程的计数器差值**不受影响**。
///
/// ## 为什么要有这条（一个真实缺陷的回归锁，review 复审 I-1）
///
/// `texture_r8_upload_count()` 的第一版是**进程级 `static AtomicUsize`**，
/// 而 [`repeated_frames_with_unchanged_atlas_do_not_reupload_texture`] 断言的是
/// **帧间差值** —— 于是在 `cargo test` 的**默认多线程**下，别的测试并行上传
/// 会让差值变成非零。reviewer 实测：`cargo test -p deer-vk` **1/8 FAILED**、
/// `--test gpu_vs_cpu` 并行 **2/20**、`--test-threads=1` 则 **0/15**；
/// 而报错恒为「触发了 1 次纹理上传」，**把排查者指向 `refresh_atlas_texture`，
/// 可那里根本没有问题**。
///
/// 假红的必然结局是「又是那个 flaky」→ 护栏被删掉 —— 那比没有护栏更糟。
/// 所以这条测试**主动制造**并行上传（起 4 个线程各上传 8 次），然后在主线程断言
/// 「同一个窗口内本线程的差值仍为 0」。若有人把计数器改回进程级 `static`，
/// 这条会**稳定**变红（而不是偶发）—— 它把 flaky 变成了确定性失败。
#[test]
fn upload_counter_is_thread_local_not_process_wide() {
    let extent = Extent {
        width: 128,
        height: 48,
    };
    let Some(mut pair) = text_pair(extent, 20.0) else {
        return;
    };

    // 先渲染一帧收干图集（这一帧的上传记在**本线程**的计数里）
    let text = "local";
    let mut l = DrawList::new();
    l.push(text_cmd(text, RectI::new(4, 4, 120, 36), 20.0, 0));
    assert_eq!(compare_text(&mut pair, "thread-local-warmup", &l, 0), 0);

    // 在别的线程上并行做**大量真实上传**（各用自己打开的设备，互不干扰）
    let workers: Vec<std::thread::JoinHandle<Result<usize, String>>> = (0..4)
        .map(|_| {
            std::thread::spawn(|| {
                let d = VkDevice::open(0).map_err(|e| e.to_string())?;
                for _ in 0..8 {
                    // 8×2 覆盖率纹理；内容不重要，只要真的走上传路径
                    d.create_texture_r8(8, 2, &[7u8; 16]).map_err(|e| e.to_string())?;
                }
                // 返回该**工作线程自己**看到的计数，作为「TLS 确实是按线程算」的证据
                Ok(deer_vk::device::texture_r8_upload_count())
            })
        })
        .collect();

    // 主线程这边同时只做「同一图集、不重传」的渲染，并断言差值为 0
    let mut window_max = 0usize;
    for i in 0..8 {
        let before = deer_vk::device::texture_r8_upload_count();
        assert_eq!(compare_text(&mut pair, &format!("thread-local-{i}"), &l, 0), 0);
        let after = deer_vk::device::texture_r8_upload_count();
        window_max = window_max.max(after - before);
    }
    assert_eq!(
        window_max, 0,
        "主线程「同一图集」的差值出现了 {window_max} 次上传 —— \
         若计数器是进程级 static，这里就会被别的线程的上传污染（review 复审 I-1）。\
         它必须是 thread_local 或每设备字段。"
    );

    // 每个工作线程都必须看到**自己**的 8 次上传（证明 TLS 语义、而不是全局累加）
    for (i, h) in workers.into_iter().enumerate() {
        let seen = h.join().expect("上传线程 panic").expect("上传失败");
        assert_eq!(
            seen, 8,
            "第 {i} 个工作线程应看到自己的 8 次上传，实得 {seen} —— \
             计数器不是按线程隔离的（若为进程级 static，会看到远大于 8 的累计值）"
        );
    }

    println!("并行上传下主线程差值恒为 0，且每个工作线程各看到自己的 8 次 ✅（TLS 免疫）");
}
