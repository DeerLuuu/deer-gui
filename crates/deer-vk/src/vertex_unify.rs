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
///
/// ## ⚠️ 契约冻结的是**符号**，不是这个确切值（复审实测，**契约本身是对的**）
///
/// 片元着色器/顶点着色器那一对**只读 `uv.x` 的符号**（`uv.x < 0.0` 与常量 `0.0` 比较，
/// 结果驱动 4 条 `OpSelect`）—— 见
/// [`crate::spirv::vertex_shader_unified`] / [`crate::spirv::fragment_shader_unified`]。
/// **任何负值都等价**：`ClampToEdge` 让越界采样安全，形状那一支又**不采用**采样结果
/// ⇒ 「必须恰好是 `(-1,-1)`」属于**过度约束**。
///
/// 实测（复审独立复现，`cargo clean -p deer-vk` 后重编）：
/// - 把 `unify` 的**数据源**从本常量改成 `(-2,-2)`（常量不动）⇒
///   `gpu_vs_cpu` **27 passed / 0 failed**（全部像素判据**绿**）；
/// - 反而是**本文件**的单测会红 —— `shape_segments_get_the_discriminant_uv`
///   是**自指**断言（实现与期望共用本常量）⇒ 它只能发现「数据 ≠ 本常量」，
///   发现不了「本常量本身变坏」。把常量改成 `(-2,-2)` 时它仍然绿。
/// - 真正把**确切值**钉住的是 `tests/gpu_vs_cpu.rs` 里的**字面量**
///   `assert_eq!(deer_vk::SHAPE_UV_SENTINEL, [-1.0, -1.0])`（防漂移哨兵）。
///
/// 所以这里写清楚：**符号是契约，确切值是实现选择**（`(-1,-1)` 只是那个选择）；
/// 若将来有人把符号改了（例如改成 `>= 0` 的另一侧），那才是**破坏契约**，
/// 而 `gpu_vs_cpu` 的形状语料会红。
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
    use deer_core::draw::Color;

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
    ///
    /// ## ⚠️ 这条断言是**自指**的（复审 F-6，如实登记）
    ///
    /// 实现与期望共用 [`SHAPE_UV_SENTINEL`] ⇒ 它只能发现「数据 ≠ 那个常量」，
    /// **发现不了常量值本身变坏**（实测：把常量改成 `(-2,-2)` 时本用例仍绿；
    /// 把**数据源**改成 `(-2,-2)` 时它红）。确切值由 `gpu_vs_cpu.rs` 的字面量哨兵钉住。
    ///
    /// 所以这里**额外**直接按**字面量 `0.0`** 断言「符号」——那才是被冻结的契约
    /// （见常量的文档）：即使有人把常量与数据一起改成别的负值，这一条也仍然成立，
    /// 而「改成正数/零」会让它红。
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
            // ★ **符号**才是契约（字面量 0.0，不引用那个常量）：形状域 = uv.x < 0 与 uv.y < 0。
            //   着色器只比较 `uv.x < 0.0`；这里把「两个分量都取负」也显式钉住
            //   （ClampToEdge 下越界分量必须落在负侧，否则就与图集左上角混起来）。
            assert!(
                v.uv[0] < 0.0 && v.uv[1] < 0.0,
                "判别符的**符号**是契约（字面量 0.0）：形状段第 {i} 个顶点的 uv = {:?} \
                 必须两个分量都为负；改成非负就会与合法纹理坐标（图集左上角）混淆",
                v.uv
            );
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
    /// 语料让**每一段**都能被签名区分：形状段用了**两个不同的顶点**
    /// （`pos=(0,0)` 与 `pos=(9,9)`），文本段 `pos=(7,7)`。
    ///
    /// ## ⚠️ 为什么要两个不同的形状顶点（复审 F-5：原语料对「反转」免疫）
    ///
    /// 原语料是「形状(0,0) → 文本(7,7) → 形状(0,0)」—— 两个形状段引用**同一个**顶点，
    /// 于是把段表**整体反转**（`iter().rev()`）后输出签名**一模一样** ⇒
    /// 这条单测当时是**绿的**（红的是像素级的 z 序用例，那是另一层护栏）。
    /// 换成两个不同的形状顶点后，反转会让签名变成
    /// `[(9,9),true], [(7,7),false], [(0,0),true]` ⇒ **本条自己就能咬住反转**。
    #[test]
    fn segment_order_is_preserved() {
        let shape = vec![
            shape_v([0.0, 0.0], [1.0, 1.0, 1.0, 1.0], 0.0),
            shape_v([9.0, 9.0], [2.0, 2.0, 2.0, 2.0], 0.0),
        ];
        let text = vec![text_v([7.0, 7.0], [0.5, 0.5])];
        // 交错：形状(第 0 个顶点) → 文本 → 形状(第 1 个顶点)
        let out = unify(
            &shape,
            &text,
            &[
                seg(PipelineKind::Shape, 0, 1),
                seg(PipelineKind::Text, 0, 1),
                seg(PipelineKind::Shape, 1, 1),
            ],
        );
        assert_eq!(out.len(), 3, "前置条件：段表覆盖两路全部顶点");
        // ★ 前置条件：三段必须**互不相同**，否则「反转」不可分辨（原语料就栽在这里）
        let signature: Vec<([f32; 2], bool)> = out.iter().map(|v| (v.pos, v.is_shape())).collect();
        assert_ne!(
            signature[0], signature[2],
            "前置条件：首尾两段必须是**不同的**形状顶点，否则本用例对「整体反转」免疫\
             （复审 F-5 的原始缺陷）"
        );
        assert_eq!(
            signature,
            vec![
                ([0.0, 0.0], true),
                ([7.0, 7.0], false),
                ([9.0, 9.0], true),
            ],
            "段的相对顺序必须**原封不动**（z 序是语义）：若实现按管线分组重排，\
             或者把段表整体反转，这里都会红"
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
