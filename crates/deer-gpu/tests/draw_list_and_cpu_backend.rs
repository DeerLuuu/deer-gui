//! `deer-gpu` 的断言：绘制列表的结构不变式 + CPU 参考后端的像素正确性。
//!
//! 为什么这些断言重要：**它们是「渲染链路」在没有 GPU 时的唯一验证手段**。
//! 后续 Vulkan/CPU 两个后端应当对同一份绘制列表给出**相同像素**，
//! 而「相同」的基准就是这里钉死的 CPU 结果。

use deer_core::draw::{Color, DrawCmd, DrawList, RectI};
use deer_gpu::null::{CpuBackend, CpuRenderer, Framebuffer};
use deer_core::{ GpuError };
use deer_gpu::{ Backend, Extent, PresentResult };

const RED: Color = Color::rgb(255, 0, 0);
const BG: Color = Color::rgb(0, 0, 0);
const BLUE: Color = Color::rgb(0, 0, 255);

fn ext(w: u32, h: u32) -> Extent {
    Extent {
        width: w,
        height: h,
    }
}

// ── 绘制列表的结构不变式 ─────────────────────────────────────────────────────

#[test]
fn clip_stack_must_balance() {
    let mut list = DrawList::new();
    assert!(list.clip_balanced(), "空列表必须平衡");
    list.push(DrawCmd::PushClip {
        rect: RectI::new(0, 0, 10, 10),
    });
    assert!(!list.clip_balanced(), "只 push 不平衡");
    list.push(DrawCmd::PopClip);
    assert!(list.clip_balanced(), "push+pop 必须平衡");
}

#[test]
fn from_cmds_recomputes_balance() {
    // `from_cmds` 必须与逐个 push 等价（否则后端重建列表时会悄悄丢掉平衡检查）
    let cmds = vec![
        DrawCmd::PushClip {
            rect: RectI::new(0, 0, 5, 5),
        },
        DrawCmd::FillRect {
            rect: RectI::new(0, 0, 5, 5),
            color: RED,
        },
        DrawCmd::PopClip,
    ];
    let rebuilt = DrawList::from_cmds(cmds.clone());
    assert_eq!(rebuilt.len(), 3);
    assert!(rebuilt.clip_balanced());
    let c = rebuilt.counts();
    assert_eq!(c.push_clip, 1);
    assert_eq!(c.pop_clip, 1);
    assert_eq!(c.fill_rect, 1);
}

// ── CPU 参考后端的像素正确性 ─────────────────────────────────────────────────

#[test]
fn fill_rect_paints_exactly_its_area() {
    let mut list = DrawList::new();
    list.push(DrawCmd::FillRect {
        rect: RectI::new(2, 3, 4, 5),
        color: RED,
    });
    let fb = CpuRenderer::new().render(ext(16, 16), &list, BG).unwrap();

    assert_eq!(fb.pixel(2, 3), Some([255, 0, 0, 255]), "左上角应被填充");
    assert_eq!(fb.pixel(5, 7), Some([255, 0, 0, 255]), "右下角（含）应被填充");
    assert_eq!(fb.pixel(6, 3), Some([0, 0, 0, 255]), "右边界外不应被填充");
    assert_eq!(fb.pixel(2, 8), Some([0, 0, 0, 255]), "下边界外不应被填充");
    assert_eq!(fb.pixel(1, 3), Some([0, 0, 0, 255]), "左边界外不应被填充");
    assert_eq!(
        fb.count_color(RED),
        4 * 5,
        "填充像素数必须正好等于矩形面积"
    );
}

#[test]
fn clip_actually_clips() {
    let mut list = DrawList::new();
    list.push(DrawCmd::PushClip {
        rect: RectI::new(4, 4, 2, 2), // 只允许 4..6 × 4..6
    });
    list.push(DrawCmd::FillRect {
        rect: RectI::new(0, 0, 16, 16), // 想画整个画面
        color: RED,
    });
    list.push(DrawCmd::PopClip);
    let fb = CpuRenderer::new().render(ext(16, 16), &list, BG).unwrap();
    assert_eq!(fb.count_color(RED), 4, "裁剪后只应剩下 2×2 像素");
    assert_eq!(fb.pixel(4, 4), Some([255, 0, 0, 255]));
    assert_eq!(fb.pixel(3, 4), Some([0, 0, 0, 255]), "裁剪区外不许被画到");
}

#[test]
fn nested_clips_intersect() {
    let mut list = DrawList::new();
    list.push(DrawCmd::PushClip {
        rect: RectI::new(0, 0, 10, 10),
    });
    list.push(DrawCmd::PushClip {
        rect: RectI::new(5, 5, 10, 10), // 与上一层求交 ⇒ 5..10 × 5..10
    });
    list.push(DrawCmd::FillRect {
        rect: RectI::new(0, 0, 32, 32),
        color: RED,
    });
    list.push(DrawCmd::PopClip);
    list.push(DrawCmd::PopClip);
    let fb = CpuRenderer::new().render(ext(32, 32), &list, BG).unwrap();
    assert_eq!(fb.count_color(RED), 5 * 5, "嵌套裁剪必须取交集");
    assert_eq!(fb.pixel(9, 9), Some([255, 0, 0, 255]));
    assert_eq!(fb.pixel(10, 10), Some([0, 0, 0, 255]), "交集之外不许被画到");
}

#[test]
fn stroke_draws_only_the_border() {
    let mut list = DrawList::new();
    list.push(DrawCmd::StrokeRect {
        rect: RectI::new(0, 0, 6, 6),
        color: RED,
        width: 1,
    });
    let fb = CpuRenderer::new().render(ext(8, 8), &list, BG).unwrap();
    // 边框像素 = 周长 - 4 个角被重复计 = 6*4 - 4 = 20
    assert_eq!(fb.count_color(RED), 20, "1px 描边应只画边框");
    assert_eq!(fb.pixel(0, 0), Some([255, 0, 0, 255]), "角在边框上");
    assert_eq!(fb.pixel(3, 3), Some([0, 0, 0, 255]), "内部必须是空的");
}

#[test]
fn round_rect_removes_corners() {
    let mut list = DrawList::new();
    list.push(DrawCmd::FillRoundRect {
        rect: RectI::new(0, 0, 10, 10),
        radius: 3,
        color: RED,
    });
    let fb = CpuRenderer::new().render(ext(10, 10), &list, BG).unwrap();
    assert_eq!(fb.pixel(0, 0), Some([0, 0, 0, 255]), "圆角必须挖掉角");
    assert_eq!(fb.pixel(5, 5), Some([255, 0, 0, 255]), "中心必须保留");
    let filled = fb.count_color(RED);
    assert!(filled < 100, "圆角后面积必须小于整块 100，实际 {filled}");
    assert!(filled > 60, "圆角不该吃掉太多面积，实际 {filled}");
}

#[test]
fn text_occupies_a_bounded_region() {
    let mut list = DrawList::new();
    list.push(DrawCmd::Text {
        rect: RectI::new(0, 0, 60, 18),
        text: "abcd".to_string(),
        color: RED,
        size: 13.0,
        align: 0,
    });
    let fb = CpuRenderer::new().render(ext(64, 20), &list, BG).unwrap();
    let painted = fb.count_color(RED);
    assert!(painted > 0, "文字命令必须画出东西（哪怕是占位字形格）");
    // 4 个字符 × advance(≈8) 的内缩格 ⇒ 远小于整块 60×18
    assert!(
        painted < 60 * 18,
        "文字不该填满整个矩形，实际 {painted}"
    );
    // 最后一个字符的右侧不该被画到（advance 之外）
    assert_eq!(fb.pixel(63, 1), Some([0, 0, 0, 255]), "矩形右端外不应有像素");
}

#[test]
fn render_is_deterministic() {
    // 同一份绘制列表渲染两次必须**逐字节相同** —— 这是「CPU 后端作为 GPU 对照基准」的前提
    let mut list = DrawList::new();
    list.push(DrawCmd::FillRect {
        rect: RectI::new(1, 1, 5, 5),
        color: RED,
    });
    list.push(DrawCmd::FillRoundRect {
        rect: RectI::new(3, 3, 8, 8),
        radius: 2,
        color: BLUE,
    });
    list.push(DrawCmd::Text {
        rect: RectI::new(2, 2, 20, 10),
        text: "hi".to_string(),
        color: Color::WHITE,
        size: 10.0,
        align: 0,
    });
    let a = CpuRenderer::new().render(ext(24, 24), &list, BG).unwrap();
    let b = CpuRenderer::new().render(ext(24, 24), &list, BG).unwrap();
    assert!(a.bytes_eq(&b), "两次渲染必须逐字节相同");
    assert_eq!(a.to_rgba().len(), 24 * 24 * 4, "RGBA8 缓冲长度必须正确");
}

#[test]
fn unbalanced_clip_is_an_error_not_a_panic() {
    // 后端遇到不平衡的裁剪栈必须**返回错误**，而不是 panic 或画出不可预期的结果
    let mut list = DrawList::new();
    list.push(DrawCmd::PushClip {
        rect: RectI::new(0, 0, 4, 4),
    });
    let err = CpuRenderer::new()
        .render(ext(8, 8), &list, BG)
        .expect_err("不平衡的裁剪栈必须报错");
    assert!(matches!(err, GpuError::Driver { .. }), "应是 Driver 错误：{err}");
}

// ── HAL 契约 ─────────────────────────────────────────────────────────────────

#[test]
fn cpu_backend_satisfies_the_hal_contract() {
    let backend = CpuBackend::new();
    assert_eq!(backend.name(), "cpu");
    let adapters = backend.adapters();
    assert_eq!(adapters.len(), 1, "CPU 后端必须提供一个适配器");
    assert!(!adapters[0].name.is_empty());

    let mut device = backend.open(0).expect("打开 CPU 设备");
    assert_eq!(device.info().name, adapters[0].name);
    assert!(device.wait_idle().is_ok());

    // 越界适配器索引必须报错（不得 panic）
    assert!(backend.open(7).is_err(), "越界索引必须返回错误");
}

#[test]
fn frame_path_renders_and_can_be_read_back() {
    // 走 HAL 的 Frame 路径（而不是直接调 CpuRenderer）：验证 trait 契约可用
    let backend = CpuBackend::new();
    let mut device = backend.open(0).expect("打开 CPU 设备");
    let mut frame = device.begin_frame().expect("开始帧");

    let mut list = DrawList::new();
    list.push(DrawCmd::FillRect {
        rect: RectI::new(0, 0, 4, 4),
        color: RED,
    });
    frame
        .record(&list, None)
        .expect("无文本的绘制列表应当可以记录");
    let res = frame.submit_and_present().expect("提交并呈现");
    assert_eq!(res, PresentResult::Presented);
}

/// T1.1 新增的 HAL 契约（`Frame::record` 的第二个参数）：
/// **有文本命令却没有引擎 ⇒ 明确报错，不许静默丢弃**。
///
/// 为什么这条断言值钱：`record` 从「一个参数」变成「两个参数」，
/// 最容易被误用的形式就是「调用方忘了传引擎，后端把文本命令悄悄跳过」——
/// 结果是一帧「形状都在、文字全没」的画面，且**没有任何报错**。
/// 本用例就是钉死这个退化行为。
///
/// 与它互补的另一半（形状列表 + `None` 必须成功）在
/// [`frame_path_renders_and_can_be_read_back`] 里 —— 两条一起才说明
/// 「报错」是针对**文本**而不是针对 `None` 本身。
#[test]
fn cpu_backend_reports_text_without_engine_instead_of_dropping_it() {
    let backend = CpuBackend::new();
    let mut device = backend.open(0).expect("打开 CPU 设备");
    let mut frame = device.begin_frame().expect("开始帧");

    let mut list = DrawList::new();
    list.push(DrawCmd::FillRect {
        rect: RectI::new(0, 0, 4, 4),
        color: RED,
    });
    list.push(DrawCmd::Text {
        rect: RectI::new(0, 0, 4, 4),
        text: "字".to_string(),
        color: RED,
        size: 12.0,
        align: 0,
    });

    // 前置断言：列表**确实**含文本命令（否则下面测的不是这条契约）。
    assert_eq!(list.counts().text, 1, "前置：列表里必须有一条文本命令");

    let err = frame
        .record(&list, None)
        .expect_err("有文本命令却没给引擎，必须报错而不是静默丢弃");
    assert!(
        matches!(err, GpuError::Unsupported(_)),
        "应是 `Unsupported`（与 Vulkan 后端同语义），实际：{err}"
    );
    assert!(
        err.to_string().contains("TextEngine"),
        "错误信息要说清是缺 `TextEngine`，实际：{err}"
    );
}

#[test]
fn cpu_framebuffer_helpers_work() {
    let fb = Framebuffer::new(4, 4, RED);
    assert_eq!(fb.pixel(0, 0), Some([255, 0, 0, 255]));
    assert_eq!(fb.pixel(-1, 0), None, "越界必须返回 None 而不是 panic");
    assert_eq!(fb.pixel(4, 0), None);
    assert_eq!(fb.count_color(RED), 16);
    let fb2 = Framebuffer::new(4, 4, RED);
    assert!(fb.bytes_eq(&fb2));
}

// ── 节点提示的 id 指纹（`ClipSnapshot` 绑定校验的另一半） ──────────────────

/// `node_id_fp` 是**确定性**指纹 —— 护栏跨进程/跨平台必须给出同一个结论。
///
/// 这条测试同时也是「不许把它换成退化实现」的锁：若有人把它改成
/// `id.len()`（就是本次要修的那个盲区）或换成带随机种子的 `DefaultHasher`，
/// 下面「等长不同 id 必须不同指纹」与**钉住的具体值**至少有一条会红。
#[test]
fn node_id_fingerprint_is_deterministic_and_discriminates_equal_length_ids() {
    use deer_core::draw::node_id_fp;

    // ① 确定性：同一输入每次都一样（没有随机种子）。
    for id in ["app", "button_1", "button_2", "名字", ""] {
        assert_eq!(node_id_fp(id), node_id_fp(id), "`{id}` 的指纹必须确定");
    }
    // ② **等长 id 必须给出不同指纹** —— 这正是本次修的盲区（长度看不见它）。
    for (a, b) in [
        ("button_1", "button_2"),
        ("aaaa", "bbbb"),
        ("field_1", "field_2"),
        ("field_1", "other_1"), // 等长但不同前缀（FNV 会走不同路径）
    ] {
        assert_eq!(a.len(), b.len(), "测试前置：`{a}` 与 `{b}` 必须等长（否则测的不是盲区）");
        assert_ne!(node_id_fp(a), node_id_fp(b), "等长 id `{a}`/`{b}` 必须有不同指纹");
    }
    // ③ 字节级（不是字符级）：中文 id 按 UTF-8 字节参与。
    assert_ne!(node_id_fp("名"), node_id_fp("字"));
    println!(
        "指纹：app={:#x}｜button_1={:#x}｜button_2={:#x}",
        node_id_fp("app"),
        node_id_fp("button_1"),
        node_id_fp("button_2")
    );
    // ④ 钉住具体值 —— 换成退化实现（只返回长度、换哈希、加随机种子）都会在这里红。
    //    前两行是 **FNV-1a 64 的公开测试向量**（`""` = 偏移基、`"foobar"` 是标准算例），
    //    所以这个算法本身可以被独立复核，而不是「我们自己说了算」。
    assert_eq!(node_id_fp(""), 0xcbf2_9ce4_8422_2325, "FNV-1a 64 偏移基");
    assert_eq!(node_id_fp("foobar"), 0x8594_4171_f739_67e8, "FNV-1a 64 标准算例");
    assert_eq!(node_id_fp("button_1"), 0x8574_78d6_5872_a2e1);
    assert_eq!(node_id_fp("button_2"), 0x8574_75d6_5872_9dc8);
    println!(
        "指纹：app={:#018x}｜button_1={:#018x}｜button_2={:#018x}",
        node_id_fp("app"),
        node_id_fp("button_1"),
        node_id_fp("button_2")
    );
}

/// `DrawCmd::node_hint` 是**唯一**构造点：长度与指纹都必须与 id 一致。
#[test]
fn node_hint_constructor_fills_both_checksums() {
    use deer_core::draw::node_id_fp;

    let c = DrawCmd::node_hint(RectI::new(1, 2, 3, 4), "button_1");
    assert_eq!(
        c,
        DrawCmd::NodeHint {
            rect: RectI::new(1, 2, 3, 4),
            node_id_len: 8,
            node_id_fp: node_id_fp("button_1"),
        },
        "构造点必须把长度与指纹一起写对（手写字面量会漂）"
    );
    // 中文 id：长度是**字节**长度（与 `str::len` 一致），不是字符数。
    let c2 = DrawCmd::node_hint(RectI::new(0, 0, 1, 1), "名字");
    match c2 {
        DrawCmd::NodeHint { node_id_len, .. } => {
            assert_eq!(node_id_len, 6, "「名字」是 6 字节（2 个 3 字节字符）")
        }
        other => panic!("必须是 NodeHint，实际 {other:?}"),
    }
}
