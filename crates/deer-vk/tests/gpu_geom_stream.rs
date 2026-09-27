//! `DrawList` → GPU 顶点流的**纯逻辑**断言（不需要 GPU、不需要着色器）。
//!
//! 这一层要锁死三件事：
//! 1. **像素 → NDC 的换算**（`y` 向下为正 ⇒ 像素 (0,0) 左上角映射到 `(-1,-1)`）；
//! 2. **裁剪在 CPU 侧做几何裁剪**，且 `pos`（光栅化范围）与 `rect` 属性（圆角/描边判据）
//!    必须**分开** —— `pos` 用裁剪后的矩形，`rect` 保持原始矩形（见 ledger Ruling 5）；
//! 3. **文本不静默丢弃**：`DrawCmd::Text` 记入 `unsupported`，让调用方看得见。

use deer_gpu::{Color, DrawCmd, DrawList, Extent, RectI};
use deer_vk::gpu_geom;

/// 浮点近似比较（NDC 是算出来的，不能指望逐位相等）。
fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-6
}

#[test]
fn fill_rect_becomes_two_triangles_in_ndc() {
    let mut l = DrawList::new();
    l.push(DrawCmd::FillRect { rect: RectI::new(0, 0, 8, 4), color: Color::WHITE });
    let s = gpu_geom::build_stream(&l, Extent { width: 8, height: 4 });
    assert_eq!(s.vertices.len(), 6);
    // (0,0) 像素中心 → NDC (-1,-1)（y 向下）
    assert_eq!(s.vertices[0].pos, [-1.0, -1.0]);
    assert_eq!(s.vertices[0].rect, [0.0, 0.0, 8.0, 4.0]);
    assert_eq!(s.vertices[0].radius_kind, gpu_geom::RADIUS_FILL);
    assert!(s.unsupported.is_empty());
}

#[test]
fn text_is_reported_not_dropped() {
    let mut l = DrawList::new();
    l.push(DrawCmd::Text { rect: RectI::new(0,0,20,10), text: "hi".into(), color: Color::WHITE, size: 12.0, align: 0 });
    let s = gpu_geom::build_stream(&l, Extent { width: 32, height: 16 });
    assert_eq!(s.vertices.len(), 0);
    assert_eq!(s.unsupported.len(), 1, "文本必须被**报告**，不能静默丢弃");
}

#[test]
fn clip_is_intersected_on_the_cpu() {
    let mut l = DrawList::new();
    l.push(DrawCmd::PushClip { rect: RectI::new(2, 2, 4, 4) });
    l.push(DrawCmd::FillRect { rect: RectI::new(0, 0, 8, 8), color: Color::WHITE });
    l.push(DrawCmd::PopClip);
    let s = gpu_geom::build_stream(&l, Extent { width: 8, height: 8 });
    // **Ruling（见 ledger Ruling 5）**：`pos`（光栅化范围）用**裁剪后**的矩形；
    // `rect` 属性（圆角/描边判据）用**原始**矩形 —— 两者必须分开，否则圆角会被裁剪挪位。
    assert_eq!(s.vertices[0].rect, [0.0, 0.0, 8.0, 8.0], "rect 属性保持原始矩形");
    let (x0, y0) = (s.vertices[0].pos[0], s.vertices[0].pos[1]);
    // 裁剪后矩形 (2,2,4,4) 在 8×8 画布上的左上角 → NDC
    assert!((x0 - (2.0 * 2.0 / 8.0 - 1.0)).abs() < 1e-6, "pos.x 应落在裁剪后矩形左边界");
    assert!((y0 - (2.0 * 2.0 / 8.0 - 1.0)).abs() < 1e-6, "pos.y 同理（y 向下为正）");
}

// ---------------------------------------------------------------------------
// 追加用例（brief 之外，用来钉住实现细节 —— 特别是 T3 消费顶点流时会依赖的部分）
// ---------------------------------------------------------------------------

/// 一个矩形展开成两个三角形 = 6 顶点，且 6 个顶点共享同一组属性（rect/radius_kind/颜色）。
#[test]
fn quad_is_two_triangles_with_shared_attributes() {
    let mut l = DrawList::new();
    l.push(DrawCmd::FillRect { rect: RectI::new(2, 1, 4, 2), color: Color::rgb(10, 20, 30) });
    let s = gpu_geom::build_stream(&l, Extent { width: 8, height: 8 });
    assert_eq!(s.vertices.len(), 6);
    let v0 = s.vertices[0];
    for v in &s.vertices {
        assert_eq!(v.rect, [2.0, 1.0, 4.0, 2.0], "同一四边形的 rect 属性必须一致");
        assert_eq!(v.radius_kind, gpu_geom::RADIUS_FILL);
        assert_eq!(v.color, v0.color);
    }
    // 左上 / 右上 / 右下 / 左上 / 右下 / 左下
    let expect = [
        [-0.5, -0.75],
        [0.5, -0.75],
        [0.5, -0.25],
        [-0.5, -0.75],
        [0.5, -0.25],
        [-0.5, -0.25],
    ];
    for (v, e) in s.vertices.iter().zip(expect.iter()) {
        assert!(close(v.pos[0], e[0]) && close(v.pos[1], e[1]), "顶点 {v:?} 应为 {e:?}");
    }
}

/// 颜色按 `u8 / 255.0` 归一化（目标格式 `Rgba8Unorm` 的通道语义），alpha 直通。
#[test]
fn color_is_normalized_to_unit_range() {
    let mut l = DrawList::new();
    l.push(DrawCmd::FillRect { rect: RectI::new(0, 0, 2, 2), color: Color::rgba(255, 128, 0, 0.5) });
    let s = gpu_geom::build_stream(&l, Extent { width: 2, height: 2 });
    let c = s.vertices[0].color;
    assert!(close(c[0], 1.0));
    assert!(close(c[1], 128.0 / 255.0));
    assert!(close(c[2], 0.0));
    assert!(close(c[3], 0.5));
}

/// 圆角填充：`radius_kind` 携带**半径本身**（片元着色器据此复刻 CPU 的整数像素判据）。
#[test]
fn round_rect_carries_radius_in_radius_kind() {
    let mut l = DrawList::new();
    l.push(DrawCmd::FillRoundRect { rect: RectI::new(0, 0, 10, 10), radius: 3, color: Color::WHITE });
    let s = gpu_geom::build_stream(&l, Extent { width: 10, height: 10 });
    assert_eq!(s.vertices.len(), 6);
    assert_eq!(s.vertices[0].radius_kind, 3.0);
    assert_eq!(s.vertices[0].rect, [0.0, 0.0, 10.0, 10.0]);
}

/// 负半径当作「无圆角」（与 CPU `fill(..., radius.max(0))` 一致）。
#[test]
fn negative_radius_falls_back_to_plain_fill() {
    let mut l = DrawList::new();
    l.push(DrawCmd::FillRoundRect { rect: RectI::new(0, 0, 4, 4), radius: -2, color: Color::WHITE });
    let s = gpu_geom::build_stream(&l, Extent { width: 4, height: 4 });
    assert_eq!(s.vertices[0].radius_kind, gpu_geom::RADIUS_FILL);
}

/// `radius_kind_for_stroke`（Ruling 6）：1px ⇒ `RADIUS_STROKE`，更宽 ⇒ `-width`。
#[test]
fn radius_kind_for_stroke_encodes_band_width() {
    assert_eq!(gpu_geom::radius_kind_for_stroke(1), gpu_geom::RADIUS_STROKE);
    assert_eq!(gpu_geom::radius_kind_for_stroke(2), -2.0);
    assert_eq!(gpu_geom::radius_kind_for_stroke(4), -4.0);
}

/// 描边展开为 CPU `stroke()` 的 **4 条边带**（每带 6 顶点 ⇒ 共 24），
/// 每条厚度 = `width`，`radius_kind = radius_kind_for_stroke(width)`；
/// `rect` 属性仍是**原始**矩形（描边判据的输入）。
#[test]
fn stroke_rect_expands_to_four_1px_bands() {
    let mut l = DrawList::new();
    l.push(DrawCmd::StrokeRect { rect: RectI::new(1, 1, 6, 4), color: Color::WHITE, width: 1 });
    let s = gpu_geom::build_stream(&l, Extent { width: 8, height: 8 });
    assert_eq!(s.vertices.len(), 24, "4 条边带 × 6 顶点");
    for v in &s.vertices {
        assert_eq!(v.radius_kind, gpu_geom::RADIUS_STROKE);
        assert_eq!(v.rect, [1.0, 1.0, 6.0, 4.0]);
    }
    // 顺序与 null.rs::stroke() 一致：上 / 下 / 左 / 右
    let bands: [([f32; 2], [f32; 2]); 4] = [
        ([-0.75, -0.75], [0.75, -0.5]),  // 上：y ∈ [1,2)
        ([-0.75, 0.0], [0.75, 0.25]),    // 下：y ∈ [4,5)
        ([-0.75, -0.75], [-0.5, 0.25]),  // 左：x ∈ [1,2)
        ([0.5, -0.75], [0.75, 0.25]),    // 右：x ∈ [6,7)
    ];
    for (i, (tl, br)) in bands.iter().enumerate() {
        let v = &s.vertices[i * 6];
        assert!(close(v.pos[0], tl[0]) && close(v.pos[1], tl[1]), "边带 {i} 左上角 {:?}", v.pos);
        let v = &s.vertices[i * 6 + 2];
        assert!(close(v.pos[0], br[0]) && close(v.pos[1], br[1]), "边带 {i} 右下角 {:?}", v.pos);
    }
}

/// 带宽 > 1（Ruling 6）：仍是 4 条边带，但每条**厚度 = width**，
/// `radius_kind = -width`（片元着色器用 `-radius_kind` 当带宽）。
#[test]
fn thick_stroke_bands_have_width_thickness() {
    let mut l = DrawList::new();
    l.push(DrawCmd::StrokeRect { rect: RectI::new(1, 1, 6, 4), color: Color::WHITE, width: 2 });
    let s = gpu_geom::build_stream(&l, Extent { width: 8, height: 8 });
    assert_eq!(s.vertices.len(), 24, "厚度变了，边带条数不变");
    for v in &s.vertices {
        assert_eq!(v.radius_kind, -2.0, "width=2 ⇒ radius_kind = -2.0");
        assert_eq!(v.rect, [1.0, 1.0, 6.0, 4.0]);
    }
    // 上 (1,1,6,2) / 下 (1,3,6,2) / 左 (1,1,2,4) / 右 (5,1,2,4)
    let bands: [([f32; 2], [f32; 2]); 4] = [
        ([-0.75, -0.75], [0.75, -0.25]), // 上：y ∈ [1,3)
        ([-0.75, -0.25], [0.75, 0.25]),  // 下：y ∈ [3,5)
        ([-0.75, -0.75], [-0.25, 0.25]), // 左：x ∈ [1,3)
        ([0.25, -0.75], [0.75, 0.25]),   // 右：x ∈ [5,7)
    ];
    for (i, (tl, br)) in bands.iter().enumerate() {
        let v = &s.vertices[i * 6];
        assert!(close(v.pos[0], tl[0]) && close(v.pos[1], tl[1]), "边带 {i} 左上角 {:?}", v.pos);
        let v = &s.vertices[i * 6 + 2];
        assert!(close(v.pos[0], br[0]) && close(v.pos[1], br[1]), "边带 {i} 右下角 {:?}", v.pos);
    }
}

/// 每条边带**各自**与 clip 求交：被裁掉的带整条跳过，被裁一半的带只保留可见部分。
#[test]
fn stroke_bands_are_clipped_individually() {
    let mut l = DrawList::new();
    l.push(DrawCmd::PushClip { rect: RectI::new(0, 0, 8, 2) });
    l.push(DrawCmd::StrokeRect { rect: RectI::new(1, 1, 6, 4), color: Color::WHITE, width: 1 });
    l.push(DrawCmd::PopClip);
    let s = gpu_geom::build_stream(&l, Extent { width: 8, height: 8 });
    // 上 (1,1,6,1) 可见；下带 y ∈ [4,5) 完全在 clip 之外 ⇒ 丢；左/右带被截到高 1
    assert_eq!(s.vertices.len(), 18, "3 条可见边带 × 6 顶点");
    assert!(close(s.vertices[0].pos[1], -0.75) && close(s.vertices[2].pos[1], -0.5), "上带");
    // 左带 (1,1,1,1)：x ∈ [1,2)，y ∈ [1,2)
    assert!(close(s.vertices[6].pos[0], -0.75) && close(s.vertices[8].pos[0], -0.5), "左带被截高");
    // 右带 (6,1,1,1)
    assert!(close(s.vertices[12].pos[0], 0.5) && close(s.vertices[12].pos[1], -0.75), "右带");
}

/// `width < 1` 按 1 处理（与 CPU `stroke()` 的 `width.max(1)` 一致）。
#[test]
fn non_positive_stroke_width_is_clamped_to_one_pixel() {
    let mut l = DrawList::new();
    l.push(DrawCmd::StrokeRect { rect: RectI::new(1, 1, 6, 4), color: Color::WHITE, width: 0 });
    l.push(DrawCmd::StrokeRect { rect: RectI::new(1, 1, 6, 4), color: Color::WHITE, width: -3 });
    let s = gpu_geom::build_stream(&l, Extent { width: 8, height: 8 });
    assert_eq!(s.vertices.len(), 48);
    for v in &s.vertices {
        assert_eq!(v.radius_kind, gpu_geom::RADIUS_STROKE, "负宽不能被当成圆角半径");
    }
}

/// 裁剪后为空 ⇒ 整条命令跳过（不产出顶点，也不报 unsupported）。
#[test]
fn empty_intersection_drops_the_command() {
    let mut l = DrawList::new();
    l.push(DrawCmd::PushClip { rect: RectI::new(100, 100, 4, 4) });
    l.push(DrawCmd::FillRect { rect: RectI::new(0, 0, 8, 8), color: Color::WHITE });
    l.push(DrawCmd::StrokeRect { rect: RectI::new(0, 0, 8, 8), color: Color::WHITE, width: 1 });
    l.push(DrawCmd::PopClip);
    let s = gpu_geom::build_stream(&l, Extent { width: 8, height: 8 });
    assert_eq!(s.vertices.len(), 0);
    assert!(s.unsupported.is_empty(), "被裁剪掉的几何不是「不支持」");
}

/// 裁剪栈是**嵌套**的：内层 Pop 之后要回到外层裁剪，而不是回到全画布。
#[test]
fn nested_clip_restores_outer_clip() {
    let mut l = DrawList::new();
    l.push(DrawCmd::PushClip { rect: RectI::new(0, 0, 4, 8) });
    l.push(DrawCmd::PushClip { rect: RectI::new(2, 2, 4, 4) });
    l.push(DrawCmd::FillRect { rect: RectI::new(0, 0, 8, 8), color: Color::WHITE });
    l.push(DrawCmd::PopClip);
    l.push(DrawCmd::FillRect { rect: RectI::new(0, 0, 8, 8), color: Color::WHITE });
    l.push(DrawCmd::PopClip);
    let s = gpu_geom::build_stream(&l, Extent { width: 8, height: 8 });
    assert_eq!(s.vertices.len(), 12);
    // 内层：(2,2,4,4) ∩ (2,2,4,4) = (2,2,4,4)
    assert!(close(s.vertices[0].pos[0], -0.5) && close(s.vertices[0].pos[1], -0.5));
    // 外层：(0,0,8,8) ∩ (0,0,4,8) = (0,0,4,8)
    assert!(close(s.vertices[6].pos[0], -1.0) && close(s.vertices[6].pos[1], -1.0));
    assert!(close(s.vertices[6 + 2].pos[0], 0.0) && close(s.vertices[6 + 2].pos[1], 1.0));
}

/// 原始矩形部分越出画布：`pos` 被画布边界夹住，`rect` 属性仍保留越界坐标
/// （圆角圆心要按**原始**矩形算，夹过就错了）。
#[test]
fn partially_offscreen_rect_keeps_original_attributes() {
    let mut l = DrawList::new();
    l.push(DrawCmd::FillRoundRect { rect: RectI::new(-4, -4, 8, 8), radius: 2, color: Color::WHITE });
    let s = gpu_geom::build_stream(&l, Extent { width: 8, height: 8 });
    assert_eq!(s.vertices.len(), 6);
    assert_eq!(s.vertices[0].pos, [-1.0, -1.0], "pos 被画布夹住");
    assert_eq!(s.vertices[0].rect, [-4.0, -4.0, 8.0, 8.0], "rect 属性保持原始矩形");
    assert_eq!(s.vertices[0].radius_kind, 2.0);
}

/// 零面积矩形：不产出顶点（与 CPU 的 `for y in y..bottom()` 空循环一致）。
#[test]
fn degenerate_rect_produces_nothing() {
    let mut l = DrawList::new();
    l.push(DrawCmd::FillRect { rect: RectI::new(3, 3, 0, 5), color: Color::WHITE });
    l.push(DrawCmd::FillRect { rect: RectI::new(3, 3, 5, -1), color: Color::WHITE });
    let s = gpu_geom::build_stream(&l, Extent { width: 8, height: 8 });
    assert_eq!(s.vertices.len(), 0);
    assert!(s.unsupported.is_empty());
}

/// `NodeHint` 是诊断信息，必须被**安静忽略**（既不出顶点，也不算「不支持」）。
#[test]
fn node_hint_is_ignored_silently() {
    let mut l = DrawList::new();
    l.push(DrawCmd::NodeHint { rect: RectI::new(0, 0, 8, 8), node_id_len: 7 });
    let s = gpu_geom::build_stream(&l, Extent { width: 8, height: 8 });
    assert_eq!(s.vertices.len(), 0);
    assert!(s.unsupported.is_empty());
}

/// 文本夹在几何中间：只报告一次，且不影响前后几何的产出。
#[test]
fn text_between_geometry_does_not_break_the_stream() {
    let mut l = DrawList::new();
    l.push(DrawCmd::FillRect { rect: RectI::new(0, 0, 4, 4), color: Color::WHITE });
    l.push(DrawCmd::Text { rect: RectI::new(0,0,20,10), text: "hi".into(), color: Color::WHITE, size: 12.0, align: 0 });
    l.push(DrawCmd::FillRect { rect: RectI::new(4, 4, 4, 4), color: Color::WHITE });
    let s = gpu_geom::build_stream(&l, Extent { width: 8, height: 8 });
    assert_eq!(s.vertices.len(), 12);
    assert_eq!(s.unsupported.len(), 1);
    assert_eq!(s.vertices[6].rect, [4.0, 4.0, 4.0, 4.0], "文本之后的几何照常产出");
}

/// 顶点布局是给 T3（`device.rs` 的顶点缓冲绑定）的**硬契约**：
/// `VkVertexInputBindingDescription.stride` 与各属性的 `offset` 都是手写数字，
/// 错一个字节就是「画出来是垃圾」而不是编译错误 —— 所以在这里钉死（同 `struct_layout.rs` 的思路）。
#[test]
fn vertex_layout_is_stable_for_the_vertex_buffer() {
    assert_eq!(
        std::mem::size_of::<gpu_geom::GpuVertex>(),
        44,
        "stride = 2 + 4 + 1 + 4 = 11 个 f32"
    );
    assert_eq!(std::mem::align_of::<gpu_geom::GpuVertex>(), 4, "全是 f32 ⇒ 不需要补齐");
    assert_eq!(std::mem::offset_of!(gpu_geom::GpuVertex, pos), 0, "location 0：vec2");
    assert_eq!(std::mem::offset_of!(gpu_geom::GpuVertex, rect), 8, "location 1：vec4");
    assert_eq!(
        std::mem::offset_of!(gpu_geom::GpuVertex, radius_kind),
        24,
        "location 2：float"
    );
    assert_eq!(std::mem::offset_of!(gpu_geom::GpuVertex, color), 28, "location 3：vec4");
}

/// `extent` 为 0 时按 1 处理（CPU 后端的 `Framebuffer::new(..max(1))` 同一约定）。
///
/// **这条断言必须真的会咬人**（fix round 1 / I1）：第一版只写了
/// `for v in &s.vertices { assert!(v.pos[..].is_finite()) }` —— 而 `2*px/0` 一旦发生，
/// 裁剪区 `(0,0,0,0)` 会让**每个**四边形求交后为空，`vertices` 是**空的**，
/// 于是循环一次都不执行、测试假绿（reviewer 用「删掉 `max(1)`」的变异验证抓到）。
/// 所以这里钉**顶点条数**与**精确 NDC**：退化 extent 必须被当成 1×1 画布。
#[test]
fn zero_extent_is_treated_as_one_by_one_canvas() {
    let mut l = DrawList::new();
    l.push(DrawCmd::FillRect { rect: RectI::new(0, 0, 1, 1), color: Color::WHITE });
    let s = gpu_geom::build_stream(&l, Extent { width: 0, height: 0 });
    // 1×1 画布上的 (0,0,1,1) ⇒ 铺满整块画布 ⇒ 两个三角形
    assert_eq!(s.vertices.len(), 6, "退化 extent 不能被当成「0 尺寸画布」而丢掉几何");
    assert_eq!(s.vertices[0].pos, [-1.0, -1.0], "左上角");
    assert_eq!(s.vertices[2].pos, [1.0, 1.0], "右下角（2*1/1 - 1）");
    assert_eq!(s.vertices[4].pos, s.vertices[2].pos);
    assert_eq!(s.vertices[5].pos, [-1.0, 1.0], "左下角");
    for v in &s.vertices {
        assert!(v.pos[0].is_finite() && v.pos[1].is_finite(), "NDC 不得为 NaN/Inf：{:?}", v.pos);
    }
}

/// 退化 extent 下，**超出 1×1 画布**的几何被裁掉 —— 这证明裁剪区真的跟着 `max(1)` 走，
/// 而不是「恰好因为 rect 本身就是 1×1 才通过」。
#[test]
fn zero_extent_still_clips_to_the_one_by_one_canvas() {
    let mut l = DrawList::new();
    l.push(DrawCmd::FillRect { rect: RectI::new(0, 0, 8, 8), color: Color::WHITE });
    let s = gpu_geom::build_stream(&l, Extent { width: 0, height: 0 });
    assert_eq!(s.vertices.len(), 6);
    assert_eq!(s.vertices[0].pos, [-1.0, -1.0]);
    assert_eq!(s.vertices[2].pos, [1.0, 1.0], "光栅化范围被夹进 1×1");
    assert_eq!(s.vertices[0].rect, [0.0, 0.0, 8.0, 8.0], "rect 属性仍是原始矩形（Ruling 5）");
}

/// 带宽**超过矩形边长**时，边带会沿短边方向**伸出矩形之外** —— 这不是笔误，
/// 而是 `null.rs::stroke()` 的真实语义（它按 `for k in 0..width` 反复画 4 条 1px 线，
/// **没有**把 `k` 限制在矩形内）。T1 的片元判据是按同一语义写的闭式
/// （`spirv.rs` 里记着「5 个看起来对的候选公式被穷举推翻」），所以顶点流这一侧
/// 也必须一致：本文钉住 `width > 矩形的宽/高` 时的 4 条边带几何。
///
/// rect = (2,3,4,2)、width = 5、画布 16×16 ⇒ 带宽 5 > rw(4) 也 > rh(2)：
/// - 上带 (2,3,4,5) → y ∈ [3,8)
/// - 下带 (2,0,4,5) → y ∈ [0,5) —— **行 0 在矩形上方**（ry = 3）
/// - 左带 (2,3,5,2) → x ∈ [2,7)
/// - 右带 (1,3,5,2) → x ∈ [1,6) —— **列 1 在矩形左侧**（rx = 2）
#[test]
fn thick_stroke_bands_extend_outside_the_rect() {
    let mut l = DrawList::new();
    l.push(DrawCmd::StrokeRect { rect: RectI::new(2, 3, 4, 2), color: Color::WHITE, width: 5 });
    let e = Extent { width: 16, height: 16 };
    let s = gpu_geom::build_stream(&l, e);
    assert_eq!(s.vertices.len(), 24);
    for v in &s.vertices {
        assert_eq!(v.rect, [2.0, 3.0, 4.0, 2.0], "rect 属性始终是原始矩形（Ruling 5）");
        assert_eq!(v.radius_kind, -5.0, "带宽 5 ⇒ radius_kind = -5.0（Ruling 6）");
    }
    let tl = |band: usize| s.vertices[band * 6].pos;
    let br = |band: usize| s.vertices[band * 6 + 2].pos;
    // ndc(p) = 2p/16 - 1 = p/8 - 1
    assert!(close(tl(0)[0], -0.75) && close(tl(0)[1], -0.625), "上带左上 {:?}", tl(0));
    assert!(close(br(0)[0], -0.25) && close(br(0)[1], 0.0), "上带右下 {:?}", br(0));
    assert!(close(tl(1)[1], -1.0), "下带**伸出到矩形上方**（y0 = 0 < ry = 3）：{:?}", tl(1));
    assert!(close(br(1)[1], -0.375), "下带右下 {:?}", br(1));
    assert!(close(tl(2)[0], -0.75) && close(br(2)[0], -0.125), "左带 x ∈ [2,7)");
    assert!(close(tl(3)[0], -0.875), "右带**伸出到矩形左侧**（x0 = 1 < rx = 2）：{:?}", tl(3));
    assert!(close(br(3)[0], -0.25), "右带右下 {:?}", br(3));
    assert!(close(tl(3)[1], -0.625) && close(br(3)[1], -0.375), "右带 y ∈ [3,5)");
}

/// 裁剪栈配平 ⇒ `clip_unbalanced == false`（含嵌套与正常弹出）。
#[test]
fn balanced_clip_stack_is_reported_as_balanced() {
    let mut l = DrawList::new();
    l.push(DrawCmd::FillRect { rect: RectI::new(0, 0, 4, 4), color: Color::WHITE });
    l.push(DrawCmd::PushClip { rect: RectI::new(1, 1, 4, 4) });
    l.push(DrawCmd::PushClip { rect: RectI::new(2, 2, 4, 4) });
    l.push(DrawCmd::FillRect { rect: RectI::new(0, 0, 4, 4), color: Color::WHITE });
    l.push(DrawCmd::PopClip);
    l.push(DrawCmd::PopClip);
    assert!(l.clip_balanced());
    let s = gpu_geom::build_stream(&l, Extent { width: 8, height: 8 });
    assert!(!s.clip_unbalanced, "配平的列表不该被标记为不平衡");
}

/// 少一个 `PopClip` ⇒ 必须被报告（CPU 后端在 `CpuFrame::record` 直接报错，T3 要能对齐）。
#[test]
fn missing_pop_clip_is_reported() {
    let mut l = DrawList::new();
    l.push(DrawCmd::PushClip { rect: RectI::new(1, 1, 4, 4) });
    l.push(DrawCmd::FillRect { rect: RectI::new(0, 0, 8, 8), color: Color::WHITE });
    let s = gpu_geom::build_stream(&l, Extent { width: 8, height: 8 });
    assert!(s.clip_unbalanced, "PushClip 未配对必须上报");
    assert_eq!(s.vertices.len(), 6, "几何照常产出（宽容处理，由调用方决定是否报错）");
    assert!(s.unsupported.is_empty(), "「列表坏了」不是「命令不支持」");
}

/// 多余的 `PopClip`（把初始的「全画布」弹出去）⇒ 必须被报告。
#[test]
fn extra_pop_clip_is_reported() {
    let mut l = DrawList::new();
    l.push(DrawCmd::FillRect { rect: RectI::new(0, 0, 8, 8), color: Color::WHITE });
    l.push(DrawCmd::PopClip);
    l.push(DrawCmd::PushClip { rect: RectI::new(1, 1, 4, 4) });
    l.push(DrawCmd::FillRect { rect: RectI::new(0, 0, 8, 8), color: Color::WHITE });
    l.push(DrawCmd::PopClip);
    let s = gpu_geom::build_stream(&l, Extent { width: 8, height: 8 });
    // 计数最后是配平的（多出的 Pop + Push + Pop ⇒ depth 回到 0），
    // 但「把全画布弹出栈」这件事本身已经让列表坏了
    assert!(s.clip_unbalanced, "多出的 PopClip 也算不平衡（计数配对但栈已坏）");
}

/// 直接改写 `DrawList::cmds`（`pub` 字段）会绕过列表自身的记账 ——
/// 本层因此**独立数一遍**，不信 `list.clip_balanced()`。
#[test]
fn unbalanced_cmds_field_is_still_detected() {
    let mut l = DrawList::new();
    l.cmds.push(DrawCmd::PushClip { rect: RectI::new(1, 1, 4, 4) });
    l.cmds.push(DrawCmd::FillRect { rect: RectI::new(0, 0, 8, 8), color: Color::WHITE });
    assert!(l.clip_balanced(), "列表自身的记账被绕过了（clip_balance 仍是 0）");
    let s = gpu_geom::build_stream(&l, Extent { width: 8, height: 8 });
    assert!(s.clip_unbalanced, "独立计数能抓到被绕过的记账");
}
