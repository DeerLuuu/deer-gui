//! 文本顶点流（M3b-T3）：`DrawCmd::Text` → **字形四边形**顶点流（**纯逻辑，不需要 GPU**）。
//!
//! 这是 M3b 的翻译层：`deer-gpu` 的文字引擎（字体 + 图集）在这里被翻译成「每个字形一个
//! 四边形」的顶点，Task 4 的渲染器只负责把它灌进顶点缓冲、绑定字形图集纹理、按 NEAREST 采样。
//!
//! | 层 | 模块 | 职责 |
//! |---|---|---|
//! | 翻译（纯逻辑） | **本模块** | `Text` → [`TextVertex`]（图集 `uv` + 屏幕 `pos` + 颜色） |
//! | 着色器（M3b-T2） | `spirv.rs` | `out = vec4(color.rgb * cov, color.a * cov)`，`cov = texture(tex, uv).r` |
//! | 渲染（M3b-T4） | `gpu_render.rs` | 独立顶点缓冲 + 独立管线 + 纹理上传 + 保 z 序的 draw call |
//!
//! ## 为什么是「第二条管线」而不是塞进 M3a 的 shape 管线
//!
//! M3a 用 `radius_kind` 的负值编码**描边带宽**（`-1.0` = 1px，见 `gpu_geom`），
//! 那套顶点属性（`pos`/`rect`/`radius_kind`/`color`，stride 44）已经冻结；
//! 文本要的是「`uv` + 采样纹理」，硬塞进去会破坏已冻结的契约 ⇒ 独立顶点类型 + 独立管线。
//!
//! ## 与 CPU 参考实现逐字对齐（`null.rs::draw_text_real`）
//!
//! | CPU（`draw_text_real`） | 本模块 |
//! |---|---|
//! | `placements = text.chars().map(\|c\| engine.glyph(c, size))` | 同（**首次出现会光栅化入图集**，缺字回退 `.notdef`） |
//! | `total = Σ flatten(advance)` | 同 |
//! | `start_x = match align { 1 => rect.x + (rect.w - total)/2, 2 => rect.right() - total, _ => rect.x }` | 同。**注意 `align` 是 `u8`、只有 0/1/2 有定义**：未定义值（`>= 3`）与 CPU 一样走**左对齐**兜底 —— 这是契约的一部分，有测试 `unknown_align_values_fall_back_to_left_alignment` 钉住。另外这里是 **f32 `/ 2.0`**（真实字形路径），不是占位路径的 i32 截断除法 |
//! | `baseline = rect.y + ((rect.h - (asc+desc))/2).round() + asc.round()`（再 `.round()`） | 同 |
//! | `gx0 = pen.round() + left`、`gy0 = baseline - top` | 同 |
//! | 逐 texel `coverage[slot] → blend_cov(cov/255)` | 四边形 + `uv`（**NEAREST** 采样后由片元着色器乘 `cov`） |
//! | `pen += advance`（每个字形后） | 同 |
//! | 裁剪：`clip.contains(px, py)` **逐像素** | 裁剪：`pos` 用「字形四边形 ∩ clip」**逐几何**（见下） |
//!
//! **裁剪为什么等价**：M3a 的形状着色器要在片元里用 `rect` 复刻圆角/描边判据，所以必须把
//! 「光栅化范围」与「判据矩形」分开；而**文本着色器没有任何矩形判据**，只做「按 uv 采样 × cov 乘色」
//! ⇒ 把四边形裁到可见区，等价于 CPU 的逐像素裁剪（被裁掉的像素本来 `cov` 也乘不到颜色）。
//!
//! ## `uv` 的推导（**最近邻下逐像素精确**）
//!
//! 顶点 `uv` 取的是「图集纹素的**像素边界**」：槽位 `slot` 覆盖纹素 `[slot.x, slot.x+w) × [slot.y, slot.y+h)`，
//! 与之对应的屏幕像素范围是 `[gx0, gx0+w) × [gy0, gy0+h)`，于是
//!
//! ```text
//! u(px) = (slot.x + (px - gx0)) / 图集宽         （v 同理）
//! ```
//!
//! 像素 `px` 的中心在 `px + 0.5` ⇒ `u * 图集宽 = slot.x + (px - gx0) + 0.5` ⇒
//! **NEAREST** 取「最近的纹素中心」正好是 `slot.x + (px - gx0)` —— 与 CPU 直接查表的那个纹素**一模一样**。
//! （这也是为什么采样必须是 NEAREST：线性过滤会把邻居纹素混进来，与 CPU 的整数查表不一致。）
//!
//! 注意 `u` 是**像素位置的线性函数** ⇒ **局部裁剪后必须按可见边界重算**（不是照抄整块 uv）：
//! 照抄会让 GPU 把被裁掉的左半边也拉伸进可见区，采样整体错位。有测试钉住这条。
//!
//! ## 图集尺寸必须在**所有字形入图集之后**读
//!
//! `GlyphAtlas` 会按需**增高**（`atlas.rs::ensure_height`），而 `uv` 的分母是图集尺寸 ⇒
//! 先读尺寸再光栅化新字形会让 `v` 偏小。CPU 侧同样是「先解析完所有字形、再取图集」
//! （`draw_text_real` 的顺序），本模块照抄这个顺序。
//!
//! ## `skipped`：**不报错的跳过**（M3a defer 的假阳性修复）
//!
//! M3a 里 `DrawCmd::Text` 是**无条件**报 `Unsupported`，于是「空串 / 零面积 / 被裁空的文本」
//! 这种 **CPU 一个像素都不画** 的帧会被 GPU 拒收（假阳性）。M3b 改成：**跳过并计数**。
//!
//! `skipped` = **没有产出任何顶点的 `Text` 命令数**，四种原因：
//! ① `text.is_empty()`；② `size <= 0`；③ 与 clip 求交后没有可见字形（含字形在画布外）；
//! ④ 所有字形都放不进图集（例如字号比图集还大）或只有空白字形（如 `" "`）。
//!
//! ⚠️ **与 CPU 的唯一有意差异**：`size <= 0`。CPU 把 `size` 交给 `TextEngine::glyph`，
//! 而后者会把字号夹到 `>= 1` ⇒ CPU 会画出 1px 的字形；本模块按计划（Task 3）**跳过**。
//! 这条差异在这里显式记录，并且在 `gpu_text_stream.rs` 的用例注释里也写了。
//!
//! ## 与计划签名的一处偏离（诚实记录）
//!
//! 计划写的是 `build_text_stream(&DrawList, Extent, &TextEngine)`（`&TextEngine`），
//! 实际签名是 **`&mut TextEngine`** —— 因为字形是**懒光栅化**的：`TextEngine::glyph(&mut self, ..)`
//! 会在首次遇到某字号时把位图光栅化并**插入图集**。CPU 的 `draw_text_real` 也是
//! `engine: &mut TextEngine`。若强行要 `&TextEngine`，就只能要求调用方预先「把要画的字全热一遍」，
//! 否则新字形会被静默跳过（那才是真的会画错）。以「与 CPU 逐字一致」为准 ⇒ 取 `&mut`。

use deer_core::draw::{Color, DrawCmd, DrawList, RectI};
// LY2：文本栈（TextEngine / GlyphPlacement / AtlasSlot）来自 L1 crate `deer-text`。
use deer_gpu::{ Extent };
use deer_text::glyph::AtlasSlot;
use deer_text::text::GlyphPlacement;
use deer_text::TextEngine;

/// 文本管线的一个顶点：位置（NDC）+ 图集 `uv` + 颜色（**不预乘**）。
///
/// **`#[repr(C)]` 是必须的**（与 [`crate::gpu_geom::GpuVertex`] 同一个理由）：顶点要按字节
/// 灌进顶点缓冲，Task 4 的 `VkVertexInputAttributeDescription` 要手写 stride/offset。
/// 默认 `repr(Rust)` 不保证字段顺序。锁成 C 布局后：
/// `pos @ 0` / `uv @ 8` / `color @ 16`，**stride = 32**（= 8 + 8 + 16）。
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextVertex {
    /// NDC 位置 `(x, y)`，`y` 向下为正（与 M3a 同一约定）。
    pub pos: [f32; 2],
    /// 图集纹理坐标（0–1，**纹素边界**语义，见模块文档）。
    pub uv: [f32; 2],
    /// 颜色 `(r, g, b, a)`，0–1，**不预乘**（覆盖率由片元着色器乘进来）。
    pub color: [f32; 4],
}

/// 一帧的文本顶点流 + **被跳过的命令数**（见模块文档「`skipped`」）。
#[derive(Debug, Clone, PartialEq)]
pub struct TextStream {
    pub vertices: Vec<TextVertex>,
    /// 没有产出任何顶点的 `Text` 命令数（空串 / `size <= 0` / 被裁空 / 字形放不进图集）。
    pub skipped: usize,
}

/// 把 `DrawList` 里的文本命令翻译成顶点流；`extent` 用于像素→NDC 换算与裁剪。
///
/// 需要 `&mut TextEngine`：字形是按需光栅化的（见模块文档的偏离说明）。
/// 形状命令（`FillRect`/`StrokeRect`/…）**不产出**任何顶点（它们走 M3a 的管线），
/// `PushClip`/`PopClip` 维护裁剪栈，`NodeHint` 忽略。
pub fn build_text_stream(list: &DrawList, extent: Extent, engine: &mut TextEngine) -> TextStream {
    // 与 M3a 同一约定：0 尺寸按 1 处理（Vulkan 图像不能是 0 宽高；CPU 也做 `max(1)`）
    let w = extent.width.max(1);
    let h = extent.height.max(1);
    let full = RectI::new(0, 0, w as i32, h as i32);
    let mut clip = full;
    let mut stack: Vec<RectI> = Vec::new();
    let mut out = TextStream {
        vertices: Vec::new(),
        skipped: 0,
    };

    for cmd in &list.cmds {
        match cmd {
            DrawCmd::PushClip { rect } => {
                stack.push(clip);
                clip = intersect(&clip, rect);
            }
            DrawCmd::PopClip => clip = stack.pop().unwrap_or(full),
            DrawCmd::Text {
                rect,
                text,
                color,
                size,
                align,
            } => emit_text(&mut out, engine, *rect, text, *color, *size, *align, &clip, w, h),
            // 形状命令走 M3a 的管线；诊断提示忽略
            _ => {}
        }
    }

    out
}

/// 翻译一条 `Text` 命令：每个**有墨迹**的字形一个四边形（6 顶点）。
///
/// 逐行对应 CPU `null.rs::draw_text_real`（见模块文档的对照表）。
#[allow(clippy::too_many_arguments)]
fn emit_text(
    out: &mut TextStream,
    engine: &mut TextEngine,
    rect: RectI,
    text: &str,
    color: Color,
    size: f32,
    align: u8,
    clip: &RectI,
    w: u32,
    h: u32,
) {
    // ① 假阳性修复：这两种情况根本没东西可画 ⇒ 跳过（不报错）
    if text.is_empty() || size <= 0.0 {
        out.skipped += 1;
        return;
    }

    // ② 逐字形取排版信息（首次出现会光栅化入图集；缺字回退 `.notdef`）—— 与 CPU 同序同因
    let placements: Vec<Option<GlyphPlacement>> = text.chars().map(|c| engine.glyph(c, size)).collect();
    let total: f32 = placements.iter().flatten().map(|p| p.advance).sum();

    // ③ 起点（对齐语义与 CPU 同一套）
    let start_x = match align {
        1 => rect.x as f32 + (rect.w as f32 - total) / 2.0,
        2 => rect.right() as f32 - total,
        _ => rect.x as f32,
    };

    // ④ 基线：升部/降部在 `rect` 里垂直居中
    let (ascent, descent) = {
        let m = engine.measure();
        (m.ascent(), m.descent())
    };
    let top_of_text_block = rect.y as f32 + ((rect.h as f32 - (ascent + descent)) / 2.0).round();
    let baseline = (top_of_text_block + ascent.round()).round();

    // ⑤ 图集尺寸：**必须在 ② 之后**读（新字形会让图集增高，见模块文档）
    let (atlas_w, atlas_h) = engine.atlas().size();
    if atlas_w == 0 || atlas_h == 0 {
        // 空图集（宽为 0）：任何字形都放不进去 ⇒ 这条命令什么都画不出来
        out.skipped += 1;
        return;
    }

    let before = out.vertices.len();
    let mut pen = start_x;
    for p in placements.into_iter().flatten() {
        // 没有墨迹的字形（空格 / 无轮廓字形）：只推进笔位置（与 CPU 的 `slot.w > 0 && slot.h > 0` 一致）
        if p.slot.w > 0 && p.slot.h > 0 {
            let gx0 = pen.round() as i32 + p.left;
            let gy0 = baseline as i32 - p.top;
            let quad = RectI::new(gx0, gy0, p.slot.w as i32, p.slot.h as i32);
            // 与 clip 求交：可见部分才光栅化（等价于 CPU 的逐像素裁剪，见模块文档）
            let vis = intersect(&quad, clip);
            if vis.w > 0 && vis.h > 0 {
                emit_quad(out, vis, gx0, gy0, p.slot, atlas_w, atlas_h, color, w, h);
            }
        }
        pen += p.advance;
    }

    // ⑥ 这条命令一个顶点都没产出（被裁空 / 只有空格 / 字形全放不进图集）⇒ 也算 skipped
    if out.vertices.len() == before {
        out.skipped += 1;
    }
}

/// 把一个可见的字形像素矩形展开成 6 个顶点（`TL,TR,BR` + `TL,BR,BL`）。
///
/// `uv` 按**像素位置**线性映射（裁剪后按 `vis` 的边界重算），保证 NEAREST 采样逐像素精确。
#[allow(clippy::too_many_arguments)]
fn emit_quad(
    out: &mut TextStream,
    vis: RectI,
    gx0: i32,
    gy0: i32,
    slot: AtlasSlot,
    atlas_w: u32,
    atlas_h: u32,
    color: Color,
    w: u32,
    h: u32,
) {
    let (x0, y0, x1, y1) = (vis.x, vis.y, vis.right(), vis.bottom());
    // u(px) = (slot.x + (px - gx0)) / atlas_w —— 见模块文档的推导
    let u = |px: i32| (slot.x as f32 + (px - gx0) as f32) / atlas_w as f32;
    let v = |py: i32| (slot.y as f32 + (py - gy0) as f32) / atlas_h as f32;
    let color = color_f32(color);
    let corner = |px: i32, py: i32| TextVertex {
        pos: [ndc_x(px, w), ndc_y(py, h)],
        uv: [u(px), v(py)],
        color,
    };
    for v in [
        corner(x0, y0),
        corner(x1, y0),
        corner(x1, y1),
        corner(x0, y0),
        corner(x1, y1),
        corner(x0, y1),
    ] {
        out.vertices.push(v);
    }
}

/// 像素 → NDC：`x = 2*px/w - 1`、`y = 2*py/h - 1`（**`y` 向下为正**，与 M3a 同一约定）。
fn ndc_x(px: i32, w: u32) -> f32 {
    2.0 * px as f32 / w as f32 - 1.0
}
fn ndc_y(py: i32, h: u32) -> f32 {
    2.0 * py as f32 / h as f32 - 1.0
}

/// 两个矩形的交集（与 `gpu_geom` / `null.rs` 的裁剪算法逐字一致）。
///
/// **为什么在这里再写一遍**：`gpu_geom` 的 `intersect` 是私有的，而 `gpu_geom.rs` 在 M3b-T3
/// 的允许改动清单之外（它已冻结）。复制的是 6 行纯函数，且语义有测试钉住（裁剪用例）；
/// 若将来两者要合并，应提为 `pub(crate)` 共用一个 —— 已记在 Task 3 报告里。
fn intersect(a: &RectI, b: &RectI) -> RectI {
    let x = a.x.max(b.x);
    let y = a.y.max(b.y);
    let r = a.right().min(b.right());
    let bottom = a.bottom().min(b.bottom());
    RectI::new(x, y, (r - x).max(0), (bottom - y).max(0))
}

/// `Color`（`u8` 通道 + 0–1 alpha）→ 顶点颜色（0–1），**alpha 夹到 `[0,1]`**。
///
/// 与 `gpu_geom::color_f32`（以及 CPU `null.rs::blend_cov` 的第一步）**同一约定**：
/// alpha 越界时 CPU 会 `clamp`，GPU 侧不夹就会出现「两边混合权重不同」。
fn color_f32(c: Color) -> [f32; 4] {
    [
        c.r as f32 / 255.0,
        c.g as f32 / 255.0,
        c.b as f32 / 255.0,
        c.a.clamp(0.0, 1.0),
    ]
}
