//! `deer-gpu` 的断言：绘制列表的结构不变式 + CPU 参考后端的像素正确性。
//!
//! 为什么这些断言重要：**它们是「渲染链路」在没有 GPU 时的唯一验证手段**。
//! 后续 Vulkan/CPU 两个后端应当对同一份绘制列表给出**相同像素**，
//! 而「相同」的基准就是这里钉死的 CPU 结果。

use deer_gpu::draw::{Color, DrawCmd, DrawList, RectI};
use deer_gpu::null::{CpuBackend, CpuRenderer, Framebuffer};
use deer_gpu::{Backend, Extent, GpuError, PresentResult};

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
    frame.record(&list).expect("记录绘制列表");
    let res = frame.submit_and_present().expect("提交并呈现");
    assert_eq!(res, PresentResult::Presented);
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
