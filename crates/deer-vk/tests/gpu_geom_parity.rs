//! **T2 语义守卫（CPU 侧复算）—— 这不是 GPU 证据；GPU 逐像素对照在 T4。**
//!
//! ## 这个文件在守什么
//!
//! `gpu_geom::build_stream` 把 `DrawList` 翻译成顶点流。它的正确性最终要靠「GPU 画出来的
//! 像素 == CPU 后端画出来的像素」来证明 —— 那是 **T4** 的活。但在那之前，顶点流**几何**
//! 这一层可以先用**同一条 CPU 路径**自证：
//!
//! 1. 取 `build_stream` 的顶点流；
//! 2. 按**片元着色器判据**（`deer-vk/src/spirv.rs::fragment_shader_rect_shape`，T1）
//!    在 CPU 上把每个四边形光栅化 —— 覆盖像素由 `pos` 反推、形状判据用顶点属性 `rect`；
//! 3. 与 `deer_gpu::null::CpuRenderer` 的帧缓冲逐字节比较。
//!
//! 这样能咬住的错误（都是**几何/语义**错误，不是 GPU 驱动问题）：
//! `pos` 的 NDC 换算错、裁剪求交错、Ruling 5 的「两个矩形」被合并、描边边带位置/厚度错、
//! 命令顺序错、把不该丢的几何丢掉。
//!
//! ## 与 T1 的同步约束（**改一边必须改另一边**）
//!
//! 本文件里 `inside_rounded` / `stroke_inside` / `blend` 三个函数是
//! `fragment_shader_rect_shape`（T1）的**判据的 CPU 版本**，而它们同时又必须与
//! `deer-gpu/src/null.rs` 的参考实现一致。也就是说这三处是**同一个判据的三个副本**：
//!
//! | 副本 | 位置 |
//! |---|---|
//! | GPU 片元着色器 | `crates/deer-vk/src/spirv.rs::fragment_shader_rect_shape`（T1） |
//! | CPU 参考实现 | `crates/deer-gpu/src/null.rs`（`inside_rounded` / `stroke` / `blend_cov`） |
//! | 本文件（复算） | 下面的三个 `fn` |
//!
//! 任何一处改了判据（例如圆角用浮点像素中心、描边改成 4 条独立 1px 带），这里会立刻红 ——
//! 这正是它的价值：**T1 改了着色器而没改 CPU 判据（或反过来）时，CI 会拦住。**
//!
//! ## 为什么「半透明一致」不是 GPU 证据（fix round 1 / I3）
//!
//! 不透明（`a == 1.0`）场景里，两边都是「把颜色写进像素」，一致是**构造性必然**；
//! 半透明场景里，本文件的 `blend` 是把 `null.rs::blend_cov`（`cov = 1.0`）**照抄**过来的，
//! 所以「一致」同样是**同式复算**的结果 —— 它证明的是**覆盖范围与绘制顺序**没错，
//! **不**证明 GPU 会算出同样的字节。
//!
//! GPU 上 `0 < a < 1` 的逐字节一致**不可先验保证**：`float → unorm8` 的舍入（含平局规则）
//! 与 CPU 的 `.round()` 可能差 **1 LSB**。**容差策略归 T4**（建议：不透明逐字节相等；
//! 半透明每通道允许 ≤1 LSB）。因此本文件对半透明场景**不写「逐字节」结论**，
//! 只写「同式复算一致」。
//!
//! 运行：`cargo test -p deer-vk --test gpu_geom_parity`

use deer_gpu::null::CpuRenderer;
use deer_gpu::{Color, DrawCmd, DrawList, Extent, RectI};
use deer_vk::gpu_geom;

// ---------------------------------------------------------------------------
// 判据的三个副本之一：必须与 null.rs 和 T1 的片元着色器保持同步（见文件头）
// ---------------------------------------------------------------------------

/// 复刻 `null.rs::inside_rounded`（整数像素、圆心取 `rect + r`、四角 `dx²+dy² > r²`）。
fn inside_rounded(rect: [f32; 4], x: i32, y: i32, r: i32) -> bool {
    let (rx, ry, rw, rh) = (rect[0] as i32, rect[1] as i32, rect[2] as i32, rect[3] as i32);
    let corners = [
        (rx + r, ry + r, -1, -1),
        (rx + rw - 1 - r, ry + r, 1, -1),
        (rx + r, ry + rh - 1 - r, -1, 1),
        (rx + rw - 1 - r, ry + rh - 1 - r, 1, 1),
    ];
    for (ccx, ccy, sx, sy) in corners {
        let in_corner_x = if sx < 0 { x < ccx } else { x > ccx };
        let in_corner_y = if sy < 0 { y < ccy } else { y > ccy };
        if in_corner_x && in_corner_y {
            let dx = (x - ccx) as f32;
            let dy = (y - ccy) as f32;
            if dx * dx + dy * dy > (r * r) as f32 {
                return false;
            }
        }
    }
    true
}

/// 复刻 `null.rs::stroke()` 的逐像素并集（`for k in 0..width` 的 4 条 1px 带）。
///
/// 与顶点流一侧的对应关系（Ruling 6）：顶点流把 4 条**厚 `width`** 的边带各发一个四边形，
/// 而它们的并集正是这里的并集 —— 所以「四角像素被两条带各混合一次」两边一致。
fn stroke_inside(rect: [f32; 4], x: i32, y: i32, width: i32) -> bool {
    let (rx, ry, rw, rh) = (rect[0] as i32, rect[1] as i32, rect[2] as i32, rect[3] as i32);
    let (right, bottom) = (rx + rw, ry + rh);
    for k in 0..width {
        if (y == ry + k || y == bottom - 1 - k) && x >= rx && x < right {
            return true;
        }
        if (x == rx + k || x == right - 1 - k) && y >= ry && y < bottom {
            return true;
        }
    }
    false
}

/// `null.rs::blend_cov`（`cov = 1.0`）的照抄 —— 见文件头：同式复算，不是 GPU 证据。
fn blend(px: &mut [u8], i: usize, c: [f32; 4]) {
    let a = c[3].clamp(0.0, 1.0);
    if a <= 0.0 {
        return;
    }
    let inv = 1.0 - a;
    let ch = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    let src = [ch(c[0]), ch(c[1]), ch(c[2])];
    for k in 0..3 {
        px[i + k] = (src[k] as f32 * a + px[i + k] as f32 * inv).round() as u8;
    }
    let da = px[i + 3] as f32 / 255.0;
    px[i + 3] = ((a + da * inv).clamp(0.0, 1.0) * 255.0).round() as u8;
}

// ---------------------------------------------------------------------------
// 把顶点流按片元判据光栅化
// ---------------------------------------------------------------------------

/// 按 `fragment_shader_rect_shape` 的语义把顶点流光栅化。
///
/// 三个关键点（与 T1 的着色器逐条对应）：
/// - 覆盖像素由 `pos` 反推（这里是整数对齐的矩形，所以 `round` 能精确还原）；
/// - 形状判据用**顶点属性 `rect`（原始矩形）**，不是 `pos` 反推的矩形；
/// - `radius_kind`：`0` = 填充、`> 0` = 圆角半径、`< 0` = 描边（带宽 = `-radius_kind`）。
fn rasterize_stream(stream: &gpu_geom::GpuStream, extent: Extent, clear: Color) -> Vec<u8> {
    let (w, h) = (extent.width.max(1) as i32, extent.height.max(1) as i32);
    let (fw, fh) = (w as f32, h as f32);
    let mut px = vec![0u8; (w as usize) * (h as usize) * 4];
    for p in px.chunks_exact_mut(4) {
        p[0] = clear.r;
        p[1] = clear.g;
        p[2] = clear.b;
        p[3] = (clear.a.clamp(0.0, 1.0) * 255.0) as u8;
    }
    for q in stream.vertices.chunks_exact(6) {
        let v = q[0];
        let to_px = |ndc: f32, n: f32| ((ndc + 1.0) * n / 2.0).round() as i32;
        let (x0, y0) = (to_px(v.pos[0], fw), to_px(v.pos[1], fh));
        let (x1, y1) = (to_px(q[2].pos[0], fw), to_px(q[2].pos[1], fh));
        for y in y0..y1 {
            for x in x0..x1 {
                let inside = if v.radius_kind == gpu_geom::RADIUS_FILL {
                    true
                } else if v.radius_kind > 0.0 {
                    inside_rounded(v.rect, x, y, v.radius_kind as i32)
                } else {
                    stroke_inside(v.rect, x, y, -v.radius_kind as i32)
                };
                if inside && x >= 0 && y >= 0 && x < w && y < h {
                    let i = ((y as usize) * (w as usize) + (x as usize)) * 4;
                    blend(&mut px, i, v.color);
                }
            }
        }
    }
    px
}

/// 顶点流（按片元判据光栅化）必须与 CPU 后端的帧缓冲**逐字节相同**。
fn assert_parity(name: &str, list: &DrawList, extent: Extent) {
    let clear = Color::rgb(16, 16, 16);
    let stream = gpu_geom::build_stream(list, extent);
    assert!(stream.unsupported.is_empty(), "{name}: 本用例不该有 unsupported");
    assert!(!stream.clip_unbalanced, "{name}: 本用例的裁剪栈应当配平");
    let mine = rasterize_stream(&stream, extent, clear);
    let cpu = CpuRenderer::new().render(extent, list, clear).expect("CPU 渲染失败");
    let theirs = cpu.to_rgba();
    assert_eq!(mine.len(), theirs.len(), "{name}: 帧缓冲尺寸");
    if mine != theirs {
        let w = extent.width as usize;
        let mut diffs = 0;
        for (i, (a, b)) in mine.iter().zip(theirs.iter()).enumerate() {
            if a != b {
                if diffs < 8 {
                    let p = i / 4;
                    eprintln!(
                        "{name}: 像素 ({}, {}) 顶点流={:?} CPU={:?}",
                        p % w,
                        p / w,
                        &mine[i - i % 4..i - i % 4 + 4],
                        &theirs[i - i % 4..i - i % 4 + 4]
                    );
                }
                diffs += 1;
            }
        }
        panic!("{name}: {diffs} 个字节不同（顶点流 vs CPU 后端，同式复算）");
    }
}

const W: Color = Color::WHITE;
const E8: Extent = Extent { width: 8, height: 8 };
const E16: Extent = Extent { width: 16, height: 12 };

// ---------------------------------------------------------------------------
// 用例：不透明（构造性必然）
// ---------------------------------------------------------------------------

#[test]
fn fill_rect_parity() {
    let mut l = DrawList::new();
    l.push(DrawCmd::FillRect { rect: RectI::new(2, 1, 4, 2), color: W });
    assert_parity("fill", &l, E8);

    // 部分越出画布（负坐标）：pos 该被夹住，CPU 的 blend 本来就不写越界像素
    let mut l = DrawList::new();
    l.push(DrawCmd::FillRect { rect: RectI::new(-3, -2, 6, 5), color: W });
    assert_parity("fill-offscreen", &l, E8);

    // 零面积：两边都应当什么都不画
    let mut l = DrawList::new();
    l.push(DrawCmd::FillRect { rect: RectI::new(3, 3, 0, 5), color: W });
    assert_parity("fill-degenerate", &l, E8);
}

#[test]
fn round_rect_parity() {
    for radius in [1, 2, 3] {
        let mut l = DrawList::new();
        l.push(DrawCmd::FillRoundRect { rect: RectI::new(1, 1, 6, 6), radius, color: W });
        assert_parity(&format!("round-{radius}"), &l, E8);
    }
    // 圆角半径很大（超过半宽）：两边的显式判据都必须给出一致结果
    let mut l = DrawList::new();
    l.push(DrawCmd::FillRoundRect { rect: RectI::new(1, 1, 6, 6), radius: 5, color: W });
    assert_parity("round-huge", &l, E8);
}

#[test]
fn stroke_rect_parity() {
    // 1px 与厚边（Ruling 6：band 厚度 = width，带宽编码在 radius_kind 里）
    for width in [1, 2, 3] {
        let mut l = DrawList::new();
        l.push(DrawCmd::StrokeRect { rect: RectI::new(1, 1, 6, 4), color: W, width });
        assert_parity(&format!("stroke-w{width}"), &l, E8);
    }
    let mut l = DrawList::new();
    l.push(DrawCmd::StrokeRect { rect: RectI::new(0, 0, 8, 8), color: W, width: 1 });
    assert_parity("stroke-full", &l, E8);

    // 带宽超过半宽：4 条带两两重叠，两边的**重叠次数**必须一致
    let mut l = DrawList::new();
    l.push(DrawCmd::StrokeRect { rect: RectI::new(1, 1, 6, 4), color: W, width: 4 });
    assert_parity("stroke-overlap", &l, E8);

    // ★ 带宽**超过矩形边长** ⇒ 边带沿短边方向**伸出矩形之外**
    //   （`null.rs::stroke()` 的 `for k in 0..width` 不把 k 限制在矩形内；
    //    T1 的着色器为这个语义专门写了闭式，且记录「5 个候选公式被穷举推翻」）。
    //   顶点流这一侧：`rect.bottom() - band` / `right() - band` 会走到矩形外面去。
    let mut l = DrawList::new();
    l.push(DrawCmd::StrokeRect { rect: RectI::new(1, 1, 6, 4), color: W, width: 6 });
    assert_parity("stroke-w6-extends-above", &l, E8);

    let mut l = DrawList::new();
    l.push(DrawCmd::StrokeRect { rect: RectI::new(2, 3, 4, 2), color: W, width: 5 });
    assert_parity("stroke-w5-both-axes", &l, E16);

    // 伸出矩形之后又被画布裁掉一部分（行/列到负数）
    let mut l = DrawList::new();
    l.push(DrawCmd::StrokeRect { rect: RectI::new(1, 0, 6, 3), color: W, width: 6 });
    assert_parity("stroke-w6-offscreen", &l, E8);

    // 退化矩形 × 厚带宽（1×1 的矩形配 3px 边框）
    let mut l = DrawList::new();
    l.push(DrawCmd::StrokeRect { rect: RectI::new(3, 3, 1, 1), color: W, width: 3 });
    assert_parity("stroke-degenerate-thick", &l, E8);
}

#[test]
fn clip_parity() {
    // 裁剪 × 填充（Ruling 5 的核心场景）
    let mut l = DrawList::new();
    l.push(DrawCmd::PushClip { rect: RectI::new(2, 2, 4, 4) });
    l.push(DrawCmd::FillRect { rect: RectI::new(0, 0, 8, 8), color: W });
    l.push(DrawCmd::PopClip);
    assert_parity("clip-fill", &l, E8);

    // 裁剪 × 圆角：`rect` 属性必须是原始矩形，否则圆角会被裁剪挪位
    let mut l = DrawList::new();
    l.push(DrawCmd::PushClip { rect: RectI::new(2, 1, 4, 5) });
    l.push(DrawCmd::FillRoundRect { rect: RectI::new(1, 1, 6, 6), radius: 2, color: W });
    l.push(DrawCmd::PopClip);
    assert_parity("clip-round", &l, E8);

    // 裁剪 × 描边：4 条边带**各自**求交（上带可见、下带整条被丢、左右带被截半）
    let mut l = DrawList::new();
    l.push(DrawCmd::PushClip { rect: RectI::new(0, 0, 8, 2) });
    l.push(DrawCmd::StrokeRect { rect: RectI::new(1, 1, 6, 4), color: W, width: 1 });
    l.push(DrawCmd::PopClip);
    assert_parity("clip-stroke-w1", &l, E8);

    // 裁剪 × 厚描边
    let mut l = DrawList::new();
    l.push(DrawCmd::PushClip { rect: RectI::new(2, 0, 4, 3) });
    l.push(DrawCmd::StrokeRect { rect: RectI::new(1, 1, 6, 6), color: W, width: 2 });
    l.push(DrawCmd::PopClip);
    assert_parity("clip-stroke-w2", &l, E8);

    // 裁剪区完全在几何之外：两边都什么都不画
    let mut l = DrawList::new();
    l.push(DrawCmd::PushClip { rect: RectI::new(100, 100, 4, 4) });
    l.push(DrawCmd::FillRect { rect: RectI::new(0, 0, 8, 8), color: W });
    l.push(DrawCmd::PopClip);
    assert_parity("clip-empty", &l, E8);
}

#[test]
fn nested_clip_parity() {
    // 内层 Pop 之后必须回到**外层**裁剪，而不是全画布
    let mut l = DrawList::new();
    l.push(DrawCmd::PushClip { rect: RectI::new(0, 0, 4, 8) });
    l.push(DrawCmd::PushClip { rect: RectI::new(2, 2, 4, 4) });
    l.push(DrawCmd::FillRect { rect: RectI::new(0, 0, 8, 8), color: W });
    l.push(DrawCmd::PopClip);
    l.push(DrawCmd::FillRect { rect: RectI::new(0, 0, 8, 8), color: W });
    l.push(DrawCmd::PopClip);
    assert_parity("nested-clip", &l, E8);
}

#[test]
fn mixed_command_parity() {
    // 命令顺序敏感：后画的覆盖先画的；`NodeHint` 必须不影响任何像素
    let mut l = DrawList::new();
    l.push(DrawCmd::FillRect { rect: RectI::new(0, 0, 16, 12), color: W });
    l.push(DrawCmd::NodeHint { rect: RectI::new(0, 0, 16, 12), node_id_len: 3 });
    l.push(DrawCmd::FillRect { rect: RectI::new(2, 2, 8, 6), color: Color::rgb(0, 0, 0) });
    l.push(DrawCmd::StrokeRect { rect: RectI::new(1, 1, 12, 8), color: W, width: 2 });
    l.push(DrawCmd::FillRoundRect { rect: RectI::new(4, 3, 5, 5), radius: 2, color: W });
    assert_parity("mixed", &l, E16);
}

/// `DrawCmd::Text` 目前画不出来 ⇒ 它进 `unsupported`，但**不得影响同帧其它几何**。
#[test]
fn text_does_not_change_geometry() {
    let mut with_text = DrawList::new();
    with_text.push(DrawCmd::FillRect { rect: RectI::new(0, 0, 4, 4), color: W });
    with_text.push(DrawCmd::Text {
        rect: RectI::new(0, 0, 20, 10),
        text: "hi".into(),
        color: W,
        size: 12.0,
        align: 0,
    });
    let s = gpu_geom::build_stream(&with_text, E16);
    assert_eq!(s.unsupported.len(), 1, "文本必须被报告");

    let mut only_geometry = DrawList::new();
    only_geometry.push(DrawCmd::FillRect { rect: RectI::new(0, 0, 4, 4), color: W });

    let clear = Color::rgb(16, 16, 16);
    assert_eq!(
        rasterize_stream(&s, E16, clear),
        rasterize_stream(&gpu_geom::build_stream(&only_geometry, E16), E16, clear),
        "文本命令夹在中间不应改变几何产出"
    );
}

// ---------------------------------------------------------------------------
// 用例：半透明（**同式复算（构造性必然）**，不是 GPU 证据 —— 见文件头 I3）
// ---------------------------------------------------------------------------

/// 半透明场景的一致性是**同式复算**：本文件的 `blend` 就是 `null.rs::blend_cov` 的照抄，
/// 所以「像素相同」是构造出来的必然结果。它验证的是**覆盖范围、绘制顺序、重叠次数**
/// （例如描边四角被两条边带各混合一次），**不**证明 GPU 会算出同样的字节 ——
/// `0 < a < 1` 的 GPU 逐字节一致不可先验保证（`float → unorm8` 舍入 vs `.round()` 可能差
/// 1 LSB），容差策略归 T4。
#[test]
fn semi_transparent_parity_is_same_formula_recomputation() {
    let half = Color::rgba(255, 0, 0, 0.5);
    let quarter = Color::rgba(0, 128, 255, 0.25);

    // 2px 半透明描边：四角像素落在两条边带里 ⇒ 被混合两次
    let mut l = DrawList::new();
    l.push(DrawCmd::StrokeRect { rect: RectI::new(1, 1, 12, 8), color: half, width: 2 });
    assert_parity("alpha-stroke-w2", &l, E16);

    // 三条半透明命令叠加：顺序 + 覆盖都要一致
    let mut l = DrawList::new();
    l.push(DrawCmd::FillRect { rect: RectI::new(0, 0, 12, 10), color: quarter });
    l.push(DrawCmd::FillRoundRect { rect: RectI::new(2, 2, 8, 6), radius: 2, color: half });
    l.push(DrawCmd::StrokeRect { rect: RectI::new(1, 1, 10, 8), color: quarter, width: 1 });
    assert_parity("alpha-mixed", &l, E16);

    // 半透明 + 厚描边 + 裁剪：Ruling 5 + Ruling 6 一起上
    let mut l = DrawList::new();
    l.push(DrawCmd::PushClip { rect: RectI::new(2, 2, 8, 6) });
    l.push(DrawCmd::FillRoundRect { rect: RectI::new(1, 1, 10, 8), radius: 3, color: half });
    l.push(DrawCmd::StrokeRect { rect: RectI::new(1, 1, 10, 8), color: quarter, width: 3 });
    l.push(DrawCmd::PopClip);
    assert_parity("alpha-clip", &l, E16);

    // 全透明：两边都必须什么都不写（alpha 累计也不该变）
    let mut l = DrawList::new();
    l.push(DrawCmd::FillRect { rect: RectI::new(1, 1, 6, 6), color: Color::TRANSPARENT });
    assert_parity("alpha-zero", &l, E8);
}
