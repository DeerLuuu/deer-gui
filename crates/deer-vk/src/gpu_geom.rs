//! `DrawList` → GPU 顶点流（**纯逻辑：不需要 GPU，也不依赖着色器**）。
//!
//! 这一层是渲染器与 Vulkan 之间的**唯一接口**：`deer-gpu` 的绘制列表在这里被翻译成
//! 一串可 `memcpy` 进顶点缓冲的顶点，`device.rs`（M3a-T3）只负责把数据灌进去、
//! 着色器（M3a-T1）只负责按顶点属性逐像素判定形状。
//!
//! ## 顶点布局（与 `spirv.rs` 的两支新着色器**逐字段对齐**，不允许单边改动）
//!
//! | location | 类型 | 字段 | 含义 |
//! |---|---|---|---|
//! | 0 | `vec2` | [`GpuVertex::pos`] | 位置（**NDC**） |
//! | 1 | `vec4` | [`GpuVertex::rect`] | **原始**矩形 `(x, y, w, h)`，像素单位 |
//! | 2 | `float` | [`GpuVertex::radius_kind`] | `0` = 普通填充；`> 0` = 圆角半径；`< 0` = 描边（`-带宽`） |
//! | 3 | `vec4` | [`GpuVertex::color`] | 颜色（0–1，**不预乘**，直接 src-alpha 混合） |
//!
//! ## 三条已裁定的约定（实现细节都从这里派生）
//!
//! 1. **NDC**：`x = 2*px/w - 1`、`y = 2*py/h - 1` —— `y` **向下为正**，
//!    所以像素原点 (0,0)（左上角）映射到 `(-1,-1)`，与 Vulkan 的 NDC 朝向一致。
//! 2. **裁剪在 CPU 侧做几何裁剪**（不依赖动态 viewport/scissor），且**两个矩形必须分开**：
//!    顶点 `pos` 用**裁剪后**的矩形（它决定光栅化范围），顶点属性 `rect` 保持**原始**矩形
//!    （圆角圆心、描边边带都要按原始矩形算，否则会被裁剪挪位）—— 见 ledger Ruling 5。
//! 3. **描边带宽编码在 `radius_kind` 里**：`width == 1` ⇒ [`RADIUS_STROKE`]（`-1.0`），
//!    更宽 ⇒ `-width`；片元着色器取 `-radius_kind` 当带宽 —— 见 ledger Ruling 6。
//!
//! ## 为什么每个顶点都重复一份 `rect`/`color`
//!
//! 顶点是「三角形角」，形状判据（圆角、描边、`inside`）却要按**整条命令**的矩形算。
//! 把 `rect` 作为**顶点属性**带下去，片元着色器就能在不知道 uniform/push-constant 的情况下
//! 复刻 CPU 的整数像素判据（`floor(gl_FragCoord)` → 与 `null.rs::inside_rounded` 逐字对应）。
//! 代价是冗余：6 个顶点各带一份同样的 `rect`/`radius_kind`/`color` —— 换来的是
//! **一条静态管线画完所有非文本命令**，不需要按命令切换常量。

use deer_gpu::{Color, DrawCmd, DrawList, Extent, RectI};

/// 普通填充：不做圆角。
pub const RADIUS_FILL: f32 = 0.0;

/// 1px 描边（`width == 1`）；更宽的描边按 **`-width`** 编码（见 Ruling 6）。
pub const RADIUS_STROKE: f32 = -1.0;

/// 把 `StrokeRect { width }` 编码成 `radius_kind`。
///
/// `width == 1` ⇒ [`RADIUS_STROKE`]（`-1.0`，最常见的 1px 边框走这个「好认」的值），
/// 否则 ⇒ `-(width as f32)`。片元着色器用 `-radius_kind` 作为带宽。
///
/// **调用方应先做 `width.max(1)`**：`0` 会得到 `-0.0`（等价于无描边），负值会变成**正数**
/// 而被片元着色器误认为圆角半径 —— [`build_stream`] 内部已经先夹过一遍。
pub fn radius_kind_for_stroke(width: i32) -> f32 {
    if width == 1 {
        RADIUS_STROKE
    } else {
        -(width as f32)
    }
}

/// 一个 GPU 顶点：位置在 NDC，其余是**逐顶点重复**的形状属性（见模块文档）。
///
/// **`#[repr(C)]` 是必须的**：这些顶点会被**逐字节**灌进顶点缓冲，T3 的
/// `VkVertexInputBindingDescription` / `VkVertexInputAttributeDescription` 要手写
/// stride 与 offset。默认的 `repr(Rust)` **不保证字段顺序** —— 实测编译器会把 `pos`
/// 排到偏移 32（而不是 0）；那样手写的 offset 会让着色器读到**别的字段的值**，
/// 症状是「画出来是垃圾」而不是编译错误。锁成 C 布局后：
/// `pos @ 0` / `rect @ 8` / `radius_kind @ 24` / `color @ 28`，stride = 44。
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GpuVertex {
    /// NDC 位置 `(x, y)`，`y` 向下为正。
    pub pos: [f32; 2],
    /// **原始**矩形 `(x, y, w, h)`（像素单位）—— 圆角/描边判据的输入。
    pub rect: [f32; 4],
    /// 形状类型：`0` = 填充，`> 0` = 圆角半径，`< 0` = 描边（`-带宽`）。
    pub radius_kind: f32,
    /// 颜色 `(r, g, b, a)`，0–1。
    pub color: [f32; 4],
}

/// 一帧的顶点流 + **没能翻译的命令**。
///
/// `unsupported` 是刻意存在的：`DrawCmd::Text` 目前画不出来，**记下来**而不是静默丢弃
/// —— 静默丢弃会让「渲染器少画了东西」表现成一张「看起来很合理」的图，那是最难查的 bug。
#[derive(Debug, Clone, PartialEq)]
pub struct GpuStream {
    pub vertices: Vec<GpuVertex>,
    /// 每条未能翻译的命令一条说明（人类可读，供上层包成 `GpuError::Unsupported`）。
    pub unsupported: Vec<String>,
}

/// 把绘制列表翻译成顶点流；`extent` 用于像素→NDC 换算与裁剪。
///
/// 语义与 CPU 后端（`deer-gpu/src/null.rs`）**刻意对齐**：
/// - 裁剪栈初始为全画布，`PushClip` 求交、`PopClip` 出栈（越界即退回全画布；
///   不平衡的列表不该走到这里 —— `CpuFrame::record` 已在入口报错）；
/// - 与裁剪区求交后为空的几何**整条跳过**（CPU 那边就是画 0 个像素）；
/// - `NodeHint` 是诊断信息，**安静忽略**；
/// - `Text` 记入 [`GpuStream::unsupported`]。
pub fn build_stream(list: &DrawList, extent: Extent) -> GpuStream {
    // 0 尺寸会让 NDC 换算除零 —— 与 CPU 后端同一约定（`Framebuffer::new(..max(1))`）
    let w = extent.width.max(1);
    let h = extent.height.max(1);
    let ndc = Ndc {
        w: w as f32,
        h: h as f32,
    };
    let full = RectI::new(0, 0, w as i32, h as i32);
    let mut clip = full;
    let mut stack: Vec<RectI> = Vec::new();
    let mut out = GpuStream {
        vertices: Vec::with_capacity(list.len() * 6),
        unsupported: Vec::new(),
    };

    for cmd in &list.cmds {
        match cmd {
            DrawCmd::PushClip { rect } => {
                stack.push(clip);
                clip = intersect(&clip, rect);
            }
            DrawCmd::PopClip => clip = stack.pop().unwrap_or(full),
            DrawCmd::FillRect { rect, color } => {
                emit_quad(&mut out.vertices, *rect, *rect, *color, RADIUS_FILL, &clip, &ndc);
            }
            DrawCmd::FillRoundRect { rect, radius, color } => {
                // 负半径视同无圆角（与 CPU `fill(.., radius.max(0))` 一致）
                let rk = (*radius).max(0) as f32;
                emit_quad(&mut out.vertices, *rect, *rect, *color, rk, &clip, &ndc);
            }
            DrawCmd::StrokeRect { rect, color, width } => {
                // CPU `stroke()` 画的是**厚 width** 的 4 条边（`for k in 0..width` 的并集），
                // 所以这里每带厚度 = width，带宽写进 `radius_kind`（Ruling 6）。
                let band = (*width).max(1);
                let rk = radius_kind_for_stroke(band);
                for side in stroke_bands(*rect, band) {
                    // 顶点 `rect` 属性始终是**原始**矩形：逐条边带各自与 clip 求交（Ruling 5）
                    emit_quad(&mut out.vertices, side, *rect, *color, rk, &clip, &ndc);
                }
            }
            DrawCmd::Text { rect, text, .. } => out.unsupported.push(format!(
                "DrawCmd::Text(rect=({}, {}, {}×{}), {} 字符)：GPU 后端尚未实现文本绘制",
                rect.x,
                rect.y,
                rect.w,
                rect.h,
                text.chars().count()
            )),
            // 诊断用提示，后端可忽略（与 CPU 后端一致）
            DrawCmd::NodeHint { .. } => {}
        }
    }

    out
}

/// NDC 换算（`y` 向下为正）。
struct Ndc {
    w: f32,
    h: f32,
}

impl Ndc {
    fn x(&self, px: i32) -> f32 {
        2.0 * px as f32 / self.w - 1.0
    }

    fn y(&self, py: i32) -> f32 {
        2.0 * py as f32 / self.h - 1.0
    }
}

/// 把一个矩形展开成 6 个顶点（两个三角形：`TL,TR,BR` + `TL,BR,BL`）。
///
/// - `rect`：决定 `pos` 的矩形 —— 会先与 `clip` 求交，为空则**一个顶点都不产出**；
/// - `attr`：写进顶点属性 `rect` 的矩形 —— 对填充类是同一个矩形，对描边是**原始**矩形。
///
/// 参数个数到 7（clippy 的 `too_many_arguments` 上限）：再拆就要引入只为计数服务的
/// 结构体，反而更难读 —— 这里保持「一次调用 = 一个四边形」的直白形状。
fn emit_quad(
    out: &mut Vec<GpuVertex>,
    rect: RectI,
    attr: RectI,
    color: Color,
    radius_kind: f32,
    clip: &RectI,
    ndc: &Ndc,
) {
    let vis = intersect(&rect, clip);
    if vis.w <= 0 || vis.h <= 0 {
        return;
    }
    let x0 = ndc.x(vis.x);
    let y0 = ndc.y(vis.y);
    let x1 = ndc.x(vis.right());
    let y1 = ndc.y(vis.bottom());
    let attr = [attr.x as f32, attr.y as f32, attr.w as f32, attr.h as f32];
    let color = color_f32(color);
    for pos in [[x0, y0], [x1, y0], [x1, y1], [x0, y0], [x1, y1], [x0, y1]] {
        out.push(GpuVertex {
            pos,
            rect: attr,
            radius_kind,
            color,
        });
    }
}

/// CPU `stroke()` 的 4 条边带，**每条厚度 = `band`**。
///
/// 与 `null.rs::stroke()` 的 `for k in 0..width` 展开等价：
/// 那 w 条 1px 带的并集恰好就是「厚 w」的这条带。
/// 顺序与 `null.rs` 一致：上 / 下 / 左 / 右（**带间会重叠** —— 四角像素被画两次，
/// CPU 那边同样重叠，所以两边像素结果一致）。
fn stroke_bands(rect: RectI, band: i32) -> [RectI; 4] {
    let b = band.max(1);
    [
        RectI::new(rect.x, rect.y, rect.w, b),
        RectI::new(rect.x, rect.bottom() - b, rect.w, b),
        RectI::new(rect.x, rect.y, b, rect.h),
        RectI::new(rect.right() - b, rect.y, b, rect.h),
    ]
}

/// 两个矩形的交集（与 `null.rs::soft_rasterize_with` 的裁剪算法逐字一致）。
fn intersect(a: &RectI, b: &RectI) -> RectI {
    let x = a.x.max(b.x);
    let y = a.y.max(b.y);
    let r = a.right().min(b.right());
    let bottom = a.bottom().min(b.bottom());
    RectI::new(x, y, (r - x).max(0), (bottom - y).max(0))
}

/// `Color`（`u8` 通道 + 0–1 alpha）→ 顶点颜色（0–1）。
///
/// `u8 / 255.0` 正是 `Rgba8Unorm` 的通道语义，所以 GPU 的 src-alpha 混合结果
/// 与 CPU 的 `blend()` 在**不透明颜色**上逐字节一致，在半透明颜色上只差舍入。
fn color_f32(c: Color) -> [f32; 4] {
    [c.r as f32 / 255.0, c.g as f32 / 255.0, c.b as f32 / 255.0, c.a]
}
