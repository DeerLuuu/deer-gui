//! **M3+ I-1 的咬住断言（离屏）**：`with_text` 二次接管 ⇒ 文本顶点缓冲**换新**，
//! 此时即使顶点**字节与上次完全相同**，也必须**重新上传**。
//!
//! 这是 reviewer 点名的唯一「缓冲换新但字节不变」路径；B3 的跳过重传若**只比字节、不比句柄**，
//! 就会把新（空）缓冲误判成"已上传" ⇒ 画的是一块未初始化缓冲 ⇒ **画面陈旧/垃圾**
//! （reviewer 在窗口路径实测 75% 画面不动）。
//!
//! 本轮教训（写进说明）：我先前用「容量增长」做换新触发，**变异后用例仍绿** ——
//! 根因是**前置条件没成立**（中间那次渲染把上传记录覆盖了，第三步的字节与记录并不相同），
//! 而前置条件不成立**不会报错**。所以本用例显式断言前置条件（句柄变了、字节没变）。

use deer_gpu::draw::{Color, DrawCmd, DrawList};
use deer_gpu::text::TextEngine;
use deer_gpu::{Extent, RectI};
use deer_vk::GpuGeometryRenderer;

fn engine(size: f32) -> Option<TextEngine> {
    match TextEngine::from_system_font(size) {
        Ok(e) => Some(e),
        Err(e) => {
            eprintln!("跳过：这台机器上拿不到系统字体（{e}）");
            None
        }
    }
}

fn renderer(extent: Extent) -> Option<GpuGeometryRenderer> {
    match GpuGeometryRenderer::new(0, extent, Color::rgb(16, 16, 16)) {
        Ok(r) => Some(r),
        Err(e) => {
            eprintln!("跳过：本机没有可用的 Vulkan GPU（{e}）");
            None
        }
    }
}

fn text_list() -> DrawList {
    let mut l = DrawList::new();
    l.push(DrawCmd::Text {
        rect: RectI::new(2, 2, 60, 28),
        text: "Wg".into(),
        color: Color::WHITE,
        size: 16.0,
        align: 0,
    });
    l
}

/// **绿**：修复后的实现（记录带缓冲句柄）⇒ 换新后**必须**重新上传，且像素与第一次一致。
#[test]
fn text_buffer_swap_forces_a_reupload() {
    let extent = Extent {
        width: 64,
        height: 32,
    };
    let Some(r) = renderer(extent) else {
        return;
    };
    let Some(e1) = engine(16.0) else {
        return;
    };
    let Some(e2) = engine(16.0) else {
        return;
    };
    let list = text_list();

    // ① 接管引擎 e1 ⇒ 首帧：文本缓冲被创建 + 上传
    let mut r = r.with_text(e1).expect("with_text(e1)");
    let p1 = r.render(&list).expect("首帧");
    let s1 = r.render_stats();
    assert_eq!(
        s1.buffer_uploads, 1,
        "首帧必须恰好上传一次文本顶点（前置条件）"
    );

    // ② **二次接管** e2 ⇒ 文本资源整体重建（`vertex: None`）⇒ **句柄必然换新**
    let r2 = r.with_text(e2).expect("with_text(e2)");
    r = r2;

    // ③ 同一份列表再渲染：**字节与记录相同、但缓冲是新的** ⇒ 必须重新上传
    let p3 = r.render(&list).expect("换新后重画");
    let s3 = r.render_stats();

    // ★ 前置条件断言（这次的教训：前置不成立不会报错，必须自己查）
    assert_eq!(
        s3.buffer_uploads,
        2,
        "缓冲换新后必须**重新上传**：若跳过（只比字节的实现），这里会停在 1"
    );
    // ★ 像素断言：若跳过上传，画的就是那块**未初始化**的新缓冲 ⇒ 与第一次不同
    assert_eq!(
        p3, p1,
        "换新后重画同一份语料，像素必须与第一次**逐字节相同**"
    );
}
