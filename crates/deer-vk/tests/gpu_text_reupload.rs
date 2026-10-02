//! **M3+ I-1 的咬住断言（离屏）**：`with_text` 二次接管 ⇒ 文本相关资源**换新**，
//! 此时即使顶点**字节与上次完全相同**，也必须**重新上传**。
//!
//! 这是 reviewer 点名的唯一「换新但字节不变」路径；B3 的跳过重传若**只比字节、不比句柄**，
//! 就会把新（空/陈旧）的状态误判成"已上传" ⇒ 画的是一块未初始化缓冲 / 采样的是旧图集
//! ⇒ **画面陈旧/垃圾**（reviewer 在窗口路径实测 75% 画面不动）。
//!
//! 本轮教训（写进说明）：我先前用「容量增长」做换新触发，**变异后用例仍绿** ——
//! 根因是**前置条件没成立**（中间那次渲染把上传记录覆盖了，第三步的字节与记录并不相同），
//! 而前置条件不成立**不会报错**。所以本用例显式断言前置条件（句柄变了、字节没变）。
//!
//! ## M3+ B5-2（统一管线）之后这条用例守的是什么（**语义更新，如实登记**）
//!
//! 统一之后**只有一块**顶点缓冲（形状与文本共用）⇒ 「文本缓冲换新」这件事**不存在了**。
//! 于是「换新 ⇒ 必须重传」的**机制**变了，但**契约**没变：接管新引擎之后，
//! 下一帧**必须**重新上传顶点、并**必须**把描述符集改指到新引擎的图集。
//! 实现侧对应 `GpuGeometryRenderer::with_text` 里那句**显式**的
//! `self.uploaded_vertices = None`（换引擎 ⇒ 作废上传记录）。
//! 本用例因此多了两条断言：
//!
//! - `bound_texture_size()` 必须**不是** `(1,1)`（描述符集真的指向了字形图集，不是哑纹理）；
//! - 第二次接管后那一帧必须触发一次**图集纹理上传**（`deer_vk::device::texture_r8_upload_count()`）。
//!
//! 这样「删掉那句作废」或「忘了改指描述符集」都会红。
//!
//! ## ⚠️ B5-3：第二条断言原先**不咬**（复审 Important F-7），现在改了读数来源
//!
//! 先前的 `bound_texture_size()` 读的是 `TextResources.texture` 这个**代理字段** ——
//! 变异「**跳过** `update_descriptor_texture`（创建了新图集纹理，但没把描述符集改指过去）」
//! 下它**照样**返回图集尺寸 ⇒ 本用例**仍然绿**（当时只有 `gpu_vs_cpu` 的 10 条像素判据接住，
//! 也就是断言名不副实）。
//!
//! 现在它读的是**描述符集真实指向**（`GpuGeometryRenderer::descriptor_points_at`，
//! 由 `gpu_render::point_descriptor_at` 在发 `vkUpdateDescriptorSets` 的**同一处**、
//! 以**返回值**赋值）⇒ 「跳过改指」必然让这里的读数停在 `(1,1)` 并变红。
//! 这是本项目「**读数与真实调用同处**」规矩的一次直接应用。

use deer_core::draw::{Color, DrawCmd, DrawList};
use deer_gpu::text::TextEngine;
use deer_core::{ RectI };
use deer_gpu::{ Extent };
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

    // ② **二次接管** e2 ⇒ 图集与上传记录整体换新
    let r2 = r.with_text(e2).expect("with_text(e2)");
    r = r2;

    // ③ 同一份列表再渲染：**字节与记录相同、但资源是新的** ⇒ 必须重新上传
    let uploads_before = deer_vk::device::texture_r8_upload_count();
    let p3 = r.render(&list).expect("换新后重画");
    let s3 = r.render_stats();
    let uploads_after = deer_vk::device::texture_r8_upload_count();

    // ★ 前置条件断言（这次的教训：前置不成立不会报错，必须自己查）
    assert_eq!(
        s3.buffer_uploads,
        2,
        "换引擎后必须**重新上传**顶点：若跳过（只比字节的实现），这里会停在 1"
    );
    // ★ 描述符集必须真的指到**字形图集**（不是 1×1 哑纹理）—— 否则采样的是别的图
    let (tw, th) = r.bound_texture_size();
    assert!(
        tw > 1 && th > 1,
        "二次接管之后 `set 0` 必须绑**字形图集**（实测 {tw}×{th}）；\
         仍是 1×1 说明描述符集没被改指到新引擎的图集"
    );
    // ★ 图集纹理必须真的重传了一次（新引擎 = 新图集；指纹不匹配 ⇒ 必须重传）
    assert_eq!(
        uploads_after - uploads_before,
        1,
        "二次接管后的第一帧必须重新上传一次图集纹理（新引擎的图集与旧的不可能同指纹）"
    );
    // ★ 像素断言：若跳过上传 / 采错图集，画出来就不是第一次那张图
    assert_eq!(
        p3, p1,
        "换新后重画同一份语料，像素必须与第一次**逐字节相同**"
    );
}
