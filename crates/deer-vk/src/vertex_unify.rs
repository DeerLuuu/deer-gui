//! **统一顶点与转换**（M3+ B5-2 Step 1，纯逻辑）：把形状段与文本段**交错合成一条**顶点流，
//! 供**单管线、单缓冲、一次 draw** 的绘制路径使用（8 draw + 8 switch → 1 + 1）。
//!
//! ## 判别符：`uv.x < 0.0 ⇒ 形状`
//!
//! 统一着色器要在一趟里同时处理两种图元，就得有个**哑元判别符**。这里用 `uv.x` 的符号：
//!
//! | 段类型 | `uv` | 含义 |
//! |---|---|---|
//! | 形状 | **恒为 `(-1, -1)`** | `uv.x < 0` ⇒ 按 M3a 的 `rect`/`radius_kind` 判据画 |
//! | 文本 | **保留真实 uv**（图集坐标，≥0） | `uv.x >= 0` ⇒ 采样字形图集覆盖率 |
//!
//! 形状段填 `(-1, -1)` 而不是 `(0, 0)` 是刻意的：`0.0` 是**合法纹理坐标**，
//! 用它当判别符会让「取样图集左上角」与「形状」混淆；负数在纹理坐标里无意义 ⇒ 不歧义。
//!
//! ## 为什么 `unify` 必须保持**段的相对顺序**
//!
//! 段的顺序**就是 z 序**（M3b 已用变异证明：打乱顺序 ⇒ 像素变）。所以本函数**只做搬运**：
//! 按 `segments` 给定的顺序，把每段对应的顶点原样搬到输出里 —— 既不排序、也不合并
//! （合并相邻同管线是 B2 的事，已在别处做过；这里面对的是**统一后**的一条管线）。
//!
//! ## CPU 成本（可计数的量，不许只说「可忽略」）
//!
//! 转换是 `O(顶点数)` 的一次线性搬运：**输出长度恒等于 `形状顶点数 + 文本顶点数`**
//! （`vertex_count()` 断言这一点）。调用方拿得到这个数 ⇒ 代价可观测，而不是一句「很便宜」。
//!
//! ## 布局（冻结，别改）
//!
//! `#[repr(C)]` + **stride 52** + 偏移 **0 / 8 / 24 / 28 / 44**
//! （`pos` 2×4 + `rect` 4×4 + `radius_kind` 4 + `color` 4×4 + `uv` 2×4）。
//! 这条布局是给 `VkVertexInputAttributeDescription` 的硬契约，由
//! [`tests::unified_vertex_layout_is_frozen`] 用 `offset_of!`/`size_of` 钉住。

use crate::gpu_geom::GpuVertex;
use crate::gpu_render::{DrawCall, PipelineKind};
use crate::gpu_text::TextVertex;

/// 形状段的判别符（`uv.x < 0` 即形状）。文本段的 `uv` 是真实图集坐标（≥ 0）。
pub const SHAPE_UV_SENTINEL: [f32; 2] = [-1.0, -1.0];

/// 统一顶点：**形状与文本共用同一种顶点**（多了 `uv`；形状段把它当判别符用）。
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UnifiedVertex {
    /// NDC 位置（与 M3a/M3b 同一约定：`y` 向下为正）。
    pub pos: [f32; 2],
    /// 形状的**原始矩形**（Ruling 5：`pos` 是裁剪后的矩形，`rect` 是原始矩形）。
    /// 文本段填 0：统一 FS 在 `uv.x >= 0` 时**不读**它。
    pub rect: [f32; 4],
    /// 形状判据：`>= 0` 圆角半径，`< 0` 描边带宽（Ruling 6）。文本段填 0。
    pub radius_kind: f32,
    /// 颜色（**不预乘**，与两条旧管线一致）。
    pub color: [f32; 4],
    /// 形状段 = [`SHAPE_UV_SENTINEL`]（判别符）；文本段 = 图集坐标。
    pub uv: [f32; 2],
}

impl UnifiedVertex {
    /// 这一顶点属于**形状**吗（判别符：`uv.x < 0`）。
    pub fn is_shape(&self) -> bool {
        self.uv[0] < 0.0
    }
}

/// 把「形状顶点 + 文本顶点 + 段表」合成一条统一顶点流。
///
/// `segments` 的**顺序即 z 序**：输出严格按它搬运，不做任何排序/合并。
/// `shape` / `text` 里的顶点由各自的翻译层产出（`gpu_geom::build_stream` /
/// `gpu_text::build_text_stream`，语义与布局**不动**）。
///
/// **前置条件**（不满足会 `panic`，因为那是调用方的编程错误，静默截断只会更难查）：
/// 每段的 `[first, first+count)` 必须落在对应数组内 —— 这也是本函数唯一的失败模式。
/// ⚠️ **Step 1 里还没有调用点**：接进绘制路径是 B5-2 Step 2（依赖 `task-30` 的统一着色器）。
/// 这里先落地实现 + 单测（零依赖），所以标 `allow(dead_code)` —— 由单测覆盖行为。
#[allow(dead_code)]
pub(crate) fn unify(shape: &[GpuVertex], text: &[TextVertex], segments: &[DrawCall]) -> Vec<UnifiedVertex> {
    // 成本可计数：输出长度恒等于两路顶点数之和（下面的 `debug_assert` 与单测钉住）
    let mut out: Vec<UnifiedVertex> = Vec::with_capacity(shape.len() + text.len());
    for seg in segments {
        let (first, count) = (seg.first as usize, seg.count as usize);
        // 前置条件：段必须落在对应数组内（越界 = 调用方的编程错误）
        let end = first.checked_add(count).expect("段区间溢出");
        match seg.kind {
            PipelineKind::Shape => {
                let src = shape
                    .get(first..end)
                    .expect("形状段越界：段表与形状顶点数组不一致");
                out.extend(src.iter().map(|v| UnifiedVertex {
                    pos: v.pos,
                    rect: v.rect,
                    radius_kind: v.radius_kind,
                    color: v.color,
                    uv: SHAPE_UV_SENTINEL,
                }));
            }
            PipelineKind::Text => {
                let src = text
                    .get(first..end)
                    .expect("文本段越界：段表与文本顶点数组不一致");
                out.extend(src.iter().map(|v| UnifiedVertex {
                    pos: v.pos,
                    // 文本段不读 `rect`/`radius_kind`（统一 FS 在 `uv.x >= 0` 时只采样图集）
                    rect: [0.0; 4],
                    radius_kind: 0.0,
                    color: v.color,
                    uv: v.uv,
                }));
            }
        }
    }
    // 不变式：输出长度 = **段表里所有段的顶点数之和**（段表覆盖不全 ⇒ 有顶点被静默丢弃）
    debug_assert_eq!(
        out.len(),
        segments.iter().map(|s| s.count as usize).sum::<usize>(),
        "统一顶点数必须恒等于段表声明的顶点数之和（有顶点被静默丢弃）"
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use deer_gpu::draw::Color;

    /// 造一个形状顶点（M3a 布局：stride 44）。
    fn shape_v(pos: [f32; 2], rect: [f32; 4], radius_kind: f32) -> GpuVertex {
        GpuVertex {
            pos,
            rect,
            radius_kind,
            color: [0.1, 0.2, 0.3, 1.0],
        }
    }

    /// 造一个文本顶点（M3b 布局：stride 32）。
    fn text_v(pos: [f32; 2], uv: [f32; 2]) -> TextVertex {
        TextVertex {
            pos,
            uv,
            color: [0.9, 0.8, 0.7, 1.0],
        }
    }

    fn seg(kind: PipelineKind, first: u32, count: u32) -> DrawCall {
        DrawCall { kind, first, count }
    }

    /// **布局冻结**：stride 52、偏移 0 / 8 / 24 / 28 / 44（给顶点输入属性表的硬契约）。
    #[test]
    fn unified_vertex_layout_is_frozen() {
        assert_eq!(std::mem::size_of::<UnifiedVertex>(), 52, "stride 必须是 52");
        assert_eq!(std::mem::align_of::<UnifiedVertex>(), 4, "全是 f32");
        assert_eq!(std::mem::offset_of!(UnifiedVertex, pos), 0);
        assert_eq!(std::mem::offset_of!(UnifiedVertex, rect), 8);
        assert_eq!(std::mem::offset_of!(UnifiedVertex, radius_kind), 24);
        assert_eq!(std::mem::offset_of!(UnifiedVertex, color), 28);
        assert_eq!(std::mem::offset_of!(UnifiedVertex, uv), 44);
        let _ = Color::WHITE; // 沿用同一套颜色类型（未预乘）
    }

    /// **判别符**：形状段一律 `uv = SHAPE_UV_SENTINEL`（`uv.x < 0`），且形状自身的数据被原样保留。
    #[test]
    fn shape_segments_get_the_discriminant_uv() {
        let shape = vec![
            shape_v([0.0, 0.0], [1.0, 2.0, 3.0, 4.0], 5.0),
            shape_v([0.5, 0.5], [6.0, 7.0, 8.0, 9.0], -1.0),
        ];
        let out = unify(&shape, &[], &[seg(PipelineKind::Shape, 0, 2)]);
        // 前置条件：真的搬了 2 个顶点（前置不成立不会报错 ⇒ 必须显式断言）
        assert_eq!(out.len(), 2, "前置条件：两路顶点数之和 = 2");
        for (i, v) in out.iter().enumerate() {
            assert_eq!(v.uv, SHAPE_UV_SENTINEL, "形状段第 {i} 个顶点必须是判别符");
            assert!(v.is_shape(), "判别符必须满足 uv.x < 0");
            assert_eq!(v.pos, shape[i].pos, "形状的 pos 必须原样保留");
            assert_eq!(v.rect, shape[i].rect, "形状的 rect 必须原样保留（Ruling 5）");
            assert_eq!(v.radius_kind, shape[i].radius_kind, "radius_kind 必须原样保留（Ruling 6）");
            assert_eq!(v.color, shape[i].color, "颜色必须原样保留（未预乘）");
        }
    }

    /// **文本段保留真实 uv**（≥ 0 ⇒ 被判别为文本），且不污染形状字段。
    #[test]
    fn text_segments_keep_their_uv() {
        let text = vec![
            text_v([1.0, 2.0], [0.25, 0.5]),
            text_v([3.0, 4.0], [0.75, 0.125]),
        ];
        let out = unify(&[], &text, &[seg(PipelineKind::Text, 0, 2)]);
        assert_eq!(out.len(), 2, "前置条件：两路顶点数之和 = 2");
        for (i, v) in out.iter().enumerate() {
            assert!(
                !v.is_shape(),
                "文本段第 {i} 个顶点不得被判成形状（uv.x 必须 ≥ 0）"
            );
            assert_eq!(v.uv, text[i].uv, "文本的 uv 必须**逐位保留**（采样正确性的前提）");
            assert_eq!(v.pos, text[i].pos);
            assert_eq!(v.color, text[i].color);
            assert_eq!(v.rect, [0.0; 4], "文本段不读 rect，填 0");
            assert_eq!(v.radius_kind, 0.0, "文本段不读 radius_kind，填 0");
        }
    }

    /// **段顺序不变（z 序是语义）** —— 本条是「打乱段顺序 ⇒ 必须红」的判据。
    ///
    /// 语料刻意让两段的顶点可区分（形状 `pos=(0,0)`、文本 `pos=(7,7)`），
    /// 于是**输出序列本身就是顺序证据**：任何排序/重排（例如「先形状后文本」）都会让
    /// 序列签名变化 ⇒ 红。
    #[test]
    fn segment_order_is_preserved() {
        let shape = vec![shape_v([0.0, 0.0], [1.0, 1.0, 1.0, 1.0], 0.0)];
        let text = vec![text_v([7.0, 7.0], [0.5, 0.5])];
        // 交错：形状 → 文本 → 形状
        let out = unify(
            &shape,
            &text,
            &[
                seg(PipelineKind::Shape, 0, 1),
                seg(PipelineKind::Text, 0, 1),
                seg(PipelineKind::Shape, 0, 1),
            ],
        );
        assert_eq!(out.len(), 3, "前置条件：段表覆盖两路全部顶点");
        let signature: Vec<([f32; 2], bool)> = out.iter().map(|v| (v.pos, v.is_shape())).collect();
        assert_eq!(
            signature,
            vec![
                ([0.0, 0.0], true),
                ([7.0, 7.0], false),
                ([0.0, 0.0], true),
            ],
            "段的相对顺序必须**原封不动**（z 序是语义）：若实现按管线分组重排，这里会变"
        );
    }

    /// **成本可计数**：输出长度恒等于 `形状顶点数 + 文本顶点数`（没有静默丢弃/补齐）。
    #[test]
    fn vertex_count_equals_inputs_sum() {
        let shape = vec![
            shape_v([0.0, 0.0], [0.0; 4], 0.0),
            shape_v([1.0, 1.0], [0.0; 4], 0.0),
            shape_v([2.0, 2.0], [0.0; 4], 0.0),
        ];
        let text = vec![text_v([3.0, 3.0], [0.1, 0.1]), text_v([4.0, 4.0], [0.2, 0.2])];
        let out = unify(
            &shape,
            &text,
            &[
                seg(PipelineKind::Shape, 0, 1),
                seg(PipelineKind::Text, 0, 1),
                seg(PipelineKind::Shape, 1, 2),
                seg(PipelineKind::Text, 1, 1),
            ],
        );
        assert_eq!(
            out.len(),
            shape.len() + text.len(),
            "本语料的段表覆盖两路全部顶点（每个顶点恰好被一段引用）⇒ 输出数 = 两路之和"
        );
    }

    /// **空段表 ⇒ 空输出**（调用方在一帧里没有任何命令时的行为）。
    #[test]
    fn empty_segments_produce_empty_output() {
        assert!(unify(&[], &[], &[]).is_empty());
        let one = vec![shape_v([0.0, 0.0], [0.0; 4], 0.0)];
        assert!(
            unify(&one, &[], &[]).is_empty(),
            "段表为空时**不得**自作主张搬运顶点（那会破坏 z 序与计数）"
        );
    }
}
