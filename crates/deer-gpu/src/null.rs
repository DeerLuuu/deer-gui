//! CPU 参考后端（`null`）：把 [`DrawList`] 光栅化到内存缓冲，**不需要 GPU**。
//!
//! 两个用途：
//! 1. **让渲染链路在 CI 里可断言** —— 无显卡、无驱动也能验证「绘制列表 → 像素」；
//! 2. **作为 GPU 后端的参考实现** —— 当 `deer-vk` 的画面对不上时，先用它取正确像素，
//!    再判断问题在渲染器还是在后端。
//!
//! 它**故意不引任何图形库**，也是「自研渲染」这条路上第一个能跑出图的实现。

use deer_core::draw::{Color, DrawCmd, DrawList, RectI};
use deer_core::error::{GpuError, GpuResult};
use crate::text::{GlyphPlacement, TextEngine};
use deer_core::{ TextureId };
use crate::{ AdapterInfo, AdapterKind, Backend, Device, Extent, Frame, PresentResult, RawWindowHandle, Swapchain, TargetFormat, TextureDesc, TextureRegion };

/// CPU 后端的适配器名（供 `adapters()` 与诊断）。
pub const CPU_ADAPTER_NAME: &str = "deer-cpu (software rasterizer)";

/// 一个纯内存的帧缓冲。
#[derive(Debug, Clone, PartialEq)]
pub struct Framebuffer {
    pub width: u32,
    pub height: u32,
    /// RGBA8，行优先，**无 padding**。
    pub pixels: Vec<u8>,
}

impl Framebuffer {
    pub fn new(width: u32, height: u32, clear: Color) -> Framebuffer {
        let mut fb = Framebuffer {
            width,
            height,
            pixels: vec![0; (width as usize) * (height as usize) * 4],
        };
        fb.clear(clear);
        fb
    }

    pub fn clear(&mut self, c: Color) {
        let a = (c.a.clamp(0.0, 1.0) * 255.0) as u8;
        for px in self.pixels.chunks_exact_mut(4) {
            px[0] = c.r;
            px[1] = c.g;
            px[2] = c.b;
            px[3] = a;
        }
    }

    /// 读取一个像素（越界返回 `None`）。
    pub fn pixel(&self, x: i32, y: i32) -> Option<[u8; 4]> {
        if x < 0 || y < 0 || x >= self.width as i32 || y >= self.height as i32 {
            return None;
        }
        let i = ((y as usize) * (self.width as usize) + (x as usize)) * 4;
        Some([
            self.pixels[i],
            self.pixels[i + 1],
            self.pixels[i + 2],
            self.pixels[i + 3],
        ])
    }

    /// 统计与给定颜色**逐字节相同**的像素数（断言用）。
    pub fn count_color(&self, c: Color) -> usize {
        let a = (c.a.clamp(0.0, 1.0) * 255.0) as u8;
        self.pixels
            .chunks_exact(4)
            .filter(|p| p[0] == c.r && p[1] == c.g && p[2] == c.b && p[3] == a)
            .count()
    }

    /// 与另一帧缓冲逐字节相同（确定性验证用）。
    pub fn bytes_eq(&self, other: &Framebuffer) -> bool {
        self.width == other.width && self.height == other.height && self.pixels == other.pixels
    }

    pub fn to_rgba(&self) -> &[u8] {
        &self.pixels
    }
}

/// CPU 后端。
#[derive(Debug, Default)]
pub struct CpuBackend;

impl CpuBackend {
    pub fn new() -> CpuBackend {
        CpuBackend
    }
}

impl Backend for CpuBackend {
    fn name(&self) -> &'static str {
        "cpu"
    }

    fn adapters(&self) -> Vec<AdapterInfo> {
        vec![AdapterInfo {
            name: CPU_ADAPTER_NAME.to_string(),
            kind: AdapterKind::Cpu,
            driver: "built-in".to_string(),
        }]
    }

    fn open(&self, adapter: usize) -> GpuResult<Box<dyn Device>> {
        if adapter != 0 {
            return Err(GpuError::NoAdapter);
        }
        Ok(Box::new(CpuDevice {
            info: self.adapters().remove(0),
            next_texture: 1,
        }))
    }
}

/// CPU 设备。
pub struct CpuDevice {
    info: AdapterInfo,
    next_texture: u32,
}

impl CpuDevice {
    /// 直接渲染一帧到帧缓冲。
    ///
    /// 这条路径**不经过 [`Frame`]**，因为 CPU 后端没有真窗口可查尺寸 ——
    /// 尺寸必须由调用方显式给。它也是测试的主入口。
    pub fn render_to_framebuffer(
        &self,
        extent: Extent,
        list: &DrawList,
        clear: Color,
    ) -> GpuResult<Framebuffer> {
        CpuRenderer::new().render(extent, list, clear)
    }
}

impl Device for CpuDevice {
    fn info(&self) -> &AdapterInfo {
        &self.info
    }

    fn create_swapchain(
        &mut self,
        _window: RawWindowHandle,
        extent: Extent,
        _format: TargetFormat,
    ) -> GpuResult<Box<dyn Swapchain>> {
        Ok(Box::new(CpuSwapchain { extent }))
    }

    fn create_texture(&mut self, _desc: TextureDesc) -> GpuResult<TextureId> {
        let id = TextureId(self.next_texture);
        self.next_texture += 1;
        Ok(id)
    }

    fn upload_texture(
        &mut self,
        _id: TextureId,
        _data: &[u8],
        _region: TextureRegion,
    ) -> GpuResult<()> {
        Ok(())
    }

    fn begin_frame(&mut self) -> GpuResult<Box<dyn Frame>> {
        Ok(Box::new(CpuFrame {
            extent: Extent {
                width: 0,
                height: 0,
            },
            pending: Vec::new(),
            outcome: None,
        }))
    }

    fn wait_idle(&mut self) -> GpuResult<()> {
        Ok(())
    }
}

struct CpuSwapchain {
    extent: Extent,
}

impl Swapchain for CpuSwapchain {
    fn extent(&self) -> Extent {
        self.extent
    }
    fn format(&self) -> TargetFormat {
        TargetFormat::Rgba8Unorm
    }
    fn resize(&mut self, extent: Extent) -> GpuResult<()> {
        self.extent = extent;
        Ok(())
    }
}

/// 一帧的 CPU 记录器：先收命令，`submit_and_present` 时一次性光栅化。
pub struct CpuFrame {
    /// CPU 后端没有真窗口 ⇒ 尺寸由调用方通过 [`CpuFrame::set_extent`] 指定。
    extent: Extent,
    pending: Vec<DrawCmd>,
    outcome: Option<Framebuffer>,
}

impl CpuFrame {
    /// 设定本帧目标尺寸（CPU 后端专用：无窗口可查）。
    pub fn set_extent(&mut self, extent: Extent) {
        self.extent = extent;
    }

    /// 取回渲染结果（`submit_and_present` 之后才有值）。
    pub fn framebuffer(&self) -> Option<&Framebuffer> {
        self.outcome.as_ref()
    }

    /// 直接渲染到给定帧缓冲（测试用；绕过 `record`）。
    pub fn render_into(fb: &mut Framebuffer, list: &DrawList) -> GpuResult<()> {
        soft_rasterize(fb, list)
    }
}

impl Frame for CpuFrame {
    fn record(&mut self, list: &DrawList, text: Option<&mut TextEngine>) -> GpuResult<()> {
        if !list.clip_balanced() {
            return Err(GpuError::Driver {
                code: -1,
                message: "绘制列表的裁剪栈不平衡（PushClip/PopClip 未配对）".to_string(),
            });
        }
        // 与 Vulkan 后端同一条契约（见 [`Frame::record`] 的文档）：
        // 有文本命令却没给引擎 ⇒ **明确报错**，不许静默丢弃。
        //
        // 这里刻意**不**就地用掉引擎：CPU 帧的契约是「先收命令、
        // `submit_and_present` 时一次性出图」（见 `CpuFrame` 的文档），
        // 本帧的光栅化走 `CpuRenderer::new()` 的占位行为；真实字形路径由
        // `CpuRenderer::with_text` 单独驱动。引擎在此只做**存在性校验**。
        let has_text = list.cmds.iter().any(|c| matches!(c, DrawCmd::Text { .. }));
        if has_text && text.is_none() {
            return Err(GpuError::Unsupported(
                "CPU 后端收到文本命令，但调用方没有提供 TextEngine（record 的第二个参数为 None）\
                 —— 与 Vulkan 后端一致：报告而不是静默丢弃"
                    .to_string(),
            ));
        }
        let _ = text;
        self.pending = list.cmds.clone();
        Ok(())
    }

    fn read_pixels(&mut self) -> GpuResult<Vec<u8>> {
        Ok(self.outcome.as_ref().map(|fb| fb.pixels.clone()).unwrap_or_default())
    }

    fn submit_and_present(mut self: Box<Self>) -> GpuResult<PresentResult> {
        let extent = self.extent;
        // 用 `from_cmds` 重建（会重新计算裁剪平衡，与逐个 push 等价）
        let list = DrawList::from_cmds(self.pending.clone());
        // 结果留在 `self.outcome`；调用方若持有 `Box<CpuFrame>` 可直接读回。
        self.outcome = Some(CpuRenderer::new().render(
            Extent {
                width: extent.width.max(1),
                height: extent.height.max(1),
            },
            &list,
            Color::rgba(0, 0, 0, 1.0),
        )?);
        Ok(PresentResult::Presented)
    }
}

/// CPU 渲染入口：把绘制列表渲染成帧缓冲。**无 GPU 环境下唯一需要的 API。**
///
/// 两种模式：
/// - [`CpuRenderer::new`]：**没有字库**。文字按「等宽占位格」画（M1 的旧行为，便于验证布局）。
/// - [`CpuRenderer::with_text`]：**有字库**。文字从 [`TextEngine`] 的字形图集里采样真实覆盖率。
///
/// 这条分界是刻意的：没有字库时**不假装**能画字，有字库时**必须**画真字 ——
/// 两者行为都能被像素断言区分（见 `tests/text_pixels.rs`）。
#[derive(Debug, Default)]
pub struct CpuRenderer {
    text: Option<TextEngine>,
}

impl CpuRenderer {
    /// 无字库渲染器（文字走占位格）。
    pub fn new() -> CpuRenderer {
        CpuRenderer { text: None }
    }

    /// 有字库渲染器（文字走真实字形）。
    pub fn with_text(engine: TextEngine) -> CpuRenderer {
        CpuRenderer { text: Some(engine) }
    }

    /// 取字库（诊断用）。
    pub fn text(&self) -> Option<&TextEngine> {
        self.text.as_ref()
    }

    /// 渲染一帧，返回帧缓冲。
    ///
    /// 需要 `&mut self`：字符串里出现新字形时要**就地光栅化并入图集**（这是缓存填充，不是状态污染 ——
    /// 同一份绘制列表重渲染仍是逐字节相同的，见 `tests/text_pixels.rs` 的确定性断言）。
    pub fn render(&mut self, extent: Extent, list: &DrawList, clear: Color) -> GpuResult<Framebuffer> {
        if !list.clip_balanced() {
            return Err(GpuError::Driver {
                code: -1,
                message: "绘制列表的裁剪栈不平衡".to_string(),
            });
        }
        let mut fb = Framebuffer::new(extent.width.max(1), extent.height.max(1), clear);
        soft_rasterize_with(&mut fb, list, self.text.as_mut())?;
        Ok(fb)
    }
}

/// 软件光栅化：矩形 / 描边 / 圆角 / 文字 / 裁剪栈。
///
/// `text` 决定文字走哪条路：`None` ⇒ 等宽占位格；`Some` ⇒ 真实字形（覆盖率采样）。
fn soft_rasterize_with(
    fb: &mut Framebuffer,
    list: &DrawList,
    mut text: Option<&mut TextEngine>,
) -> GpuResult<()> {
    let full = RectI::new(0, 0, fb.width as i32, fb.height as i32);
    let mut clip = full;
    let mut stack: Vec<RectI> = Vec::new();

    for cmd in &list.cmds {
        match cmd {
            DrawCmd::PushClip { rect } => {
                stack.push(clip);
                let x = clip.x.max(rect.x);
                let y = clip.y.max(rect.y);
                let r = clip.right().min(rect.right());
                let b = clip.bottom().min(rect.bottom());
                clip = RectI::new(x, y, (r - x).max(0), (b - y).max(0));
            }
            DrawCmd::PopClip => {
                clip = stack.pop().unwrap_or(full);
            }
            DrawCmd::FillRect { rect, color } => fill(fb, *rect, *color, &clip, 0),
            DrawCmd::FillRoundRect { rect, radius, color } => fill(fb, *rect, *color, &clip, *radius),
            DrawCmd::StrokeRect { rect, color, width } => stroke(fb, *rect, *color, *width, &clip),
            DrawCmd::Text {
                rect,
                text: s,
                color,
                size,
                align,
            } => {
                let cmd = TextDraw {
                    rect: *rect,
                    text: s,
                    color: *color,
                    size: *size,
                    align: *align,
                };
                match text.as_deref_mut() {
                    Some(engine) => draw_text_real(engine, fb, &cmd, &clip),
                    None => draw_text(fb, &cmd, &clip),
                }
            }
            DrawCmd::NodeHint { .. } => {}
        }
    }
    Ok(())
}

/// 不带字库的光栅化（占位字形格）。
fn soft_rasterize(fb: &mut Framebuffer, list: &DrawList) -> GpuResult<()> {
    soft_rasterize_with(fb, list, None)
}

fn blend(fb: &mut Framebuffer, x: i32, y: i32, c: Color, clip: &RectI) {
    blend_cov(fb, x, y, c, 1.0, clip)
}

/// 带**覆盖率乘子**的混合：字形覆盖率 `c/255` 就从这里进来。
///
/// `cov = 1.0` 时与旧 `blend` 逐字节等价（既有像素断言因此不受影响）。
fn blend_cov(fb: &mut Framebuffer, x: i32, y: i32, c: Color, cov: f32, clip: &RectI) {
    if !clip.contains(x, y) || x < 0 || y < 0 || x >= fb.width as i32 || y >= fb.height as i32 {
        return;
    }
    let a = c.a.clamp(0.0, 1.0) * cov.clamp(0.0, 1.0);
    if a <= 0.0 {
        return;
    }
    let i = ((y as usize) * (fb.width as usize) + (x as usize)) * 4;
    let inv = 1.0 - a;
    fb.pixels[i] = (c.r as f32 * a + fb.pixels[i] as f32 * inv).round() as u8;
    fb.pixels[i + 1] = (c.g as f32 * a + fb.pixels[i + 1] as f32 * inv).round() as u8;
    fb.pixels[i + 2] = (c.b as f32 * a + fb.pixels[i + 2] as f32 * inv).round() as u8;
    let da = fb.pixels[i + 3] as f32 / 255.0;
    fb.pixels[i + 3] = ((a + da * inv).clamp(0.0, 1.0) * 255.0).round() as u8;
}

fn fill(fb: &mut Framebuffer, rect: RectI, color: Color, clip: &RectI, radius: i32) {
    let r = radius.max(0);
    for y in rect.y..rect.bottom() {
        for x in rect.x..rect.right() {
            if r > 0 && !inside_rounded(rect, x, y, r) {
                continue;
            }
            blend(fb, x, y, color, clip);
        }
    }
}

/// 圆角判定：四角用圆心距离近似（不做抗锯齿 —— 抗锯齿属于后端能力，后续加）。
fn inside_rounded(rect: RectI, x: i32, y: i32, r: i32) -> bool {
    let corners = [
        (rect.x + r, rect.y + r, -1, -1),
        (rect.right() - 1 - r, rect.y + r, 1, -1),
        (rect.x + r, rect.bottom() - 1 - r, -1, 1),
        (rect.right() - 1 - r, rect.bottom() - 1 - r, 1, 1),
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

fn stroke(fb: &mut Framebuffer, rect: RectI, color: Color, width: i32, clip: &RectI) {
    let w = width.max(1);
    for k in 0..w {
        fill(fb, RectI::new(rect.x, rect.y + k, rect.w, 1), color, clip, 0);
        fill(fb, RectI::new(rect.x, rect.bottom() - 1 - k, rect.w, 1), color, clip, 0);
        fill(fb, RectI::new(rect.x + k, rect.y, 1, rect.h), color, clip, 0);
        fill(fb, RectI::new(rect.right() - 1 - k, rect.y, 1, rect.h), color, clip, 0);
    }
}

/// 一次文本绘制所需的全部参数（`DrawCmd::Text` 的字段打包）。
///
/// 为什么要打包：内部函数本来会到 8 个参数，触发 clippy 的 `too_many_arguments`（上限 7）。
/// 打包后签名更短，也让「占位路径」与「真实字形路径」拿到**完全相同**的一组输入。
struct TextDraw<'a> {
    rect: RectI,
    text: &'a str,
    color: Color,
    size: f32,
    align: u8,
}

/// 文字绘制：**占位实现** —— 每个字符画一个等宽格。
///
/// 这是**没有字库**时的路径（`CpuRenderer::new()`），作用是让「文字命令占据的像素区域」
/// 可被断言，从而验证布局 → 绘制列表 → 像素的对应关系。它有字库时会被
/// [`draw_text_real`] 取代（绘制列表与调用方无需改动）。
fn draw_text(fb: &mut Framebuffer, cmd: &TextDraw<'_>, clip: &RectI) {
    let TextDraw {
        rect,
        text,
        color,
        size,
        align,
    } = *cmd;
    let count = text.chars().count() as i32;
    let advance = ((size * 0.6).round() as i32).max(1);
    let total = advance * count;
    let start_x = match align {
        1 => rect.x + (rect.w - total) / 2,
        2 => rect.right() - total,
        _ => rect.x,
    };
    let glyph_h = (size.min(rect.h as f32).round() as i32).max(1);
    let mut x = start_x;
    for _ch in text.chars() {
        let g = RectI::new(
            x + 1,
            rect.y + 1,
            (advance - 2).max(1),
            (glyph_h - 2).max(1),
        );
        for y in g.y..g.bottom() {
            for px in g.x..g.right() {
                blend(fb, px, y, color, clip);
            }
        }
        x += advance;
    }
}

/// 真实字形绘制（**有字库**时的路径）。
///
/// 数据流：字符 → [`TextEngine::glyph`]（命中缓存或光栅化入图集）→ 按 `GlyphPlacement`
/// 算出目标像素 → 从图集覆盖率缓冲采样 → 按覆盖率混合颜色。
///
/// 三处关键语义：
/// - **advance 是真实字体的**（不是 0.6em）：所以「画出来的文本宽度」与布局阶段
///   `FontMeasure` 算的宽度一致 —— 这是按钮文字居中对齐正确的前提。
/// - **`left`/`top` 是相对笔位置与基线的偏移**：`x = round(pen) + left`、`y = round(baseline) - top`。
/// - **基线**：把「升部 + 降部」这块垂直居中放进 `rect`，与 `Theme::line_height` 无关
///   （`rect` 已经是布局算好的盒子）。
fn draw_text_real(engine: &mut TextEngine, fb: &mut Framebuffer, cmd: &TextDraw<'_>, clip: &RectI) {
    let TextDraw {
        rect,
        text,
        color,
        size,
        align,
    } = *cmd;

    // ① 解析字形（首次出现会光栅化并入图集）
    let placements: Vec<Option<GlyphPlacement>> =
        text.chars().map(|c| engine.glyph(c, size)).collect();
    let total: f32 = placements.iter().flatten().map(|p| p.advance).sum();

    // ② 起点（对齐方式与占位路径同一套语义）
    let start_x = match align {
        1 => rect.x as f32 + (rect.w as f32 - total) / 2.0,
        2 => rect.right() as f32 - total,
        _ => rect.x as f32,
    };

    // ③ 基线：升部/降部居中
    let (ascent, descent) = {
        let m = engine.measure();
        (m.ascent(), m.descent())
    };
    let top_of_text_block = rect.y as f32 + ((rect.h as f32 - (ascent + descent)) / 2.0).round();
    let baseline = (top_of_text_block + ascent.round()).round();

    // ④ 采样图集贴图
    let atlas = engine.atlas();
    let (atlas_w, _atlas_h) = atlas.size();
    let coverage = atlas.coverage();

    let mut pen = start_x;
    for p in placements.into_iter().flatten() {
        if p.slot.w > 0 && p.slot.h > 0 {
            let gx0 = pen.round() as i32 + p.left;
            let gy0 = baseline as i32 - p.top;
            for gy in 0..p.slot.h {
                let row = ((p.slot.y + gy) as usize) * (atlas_w as usize);
                for gx in 0..p.slot.w {
                    let Some(&cov) = coverage.get(row + (p.slot.x + gx) as usize) else {
                        continue;
                    };
                    if cov == 0 {
                        continue;
                    }
                    blend_cov(
                        fb,
                        gx0 + gx as i32,
                        gy0 + gy as i32,
                        color,
                        cov as f32 / 255.0,
                        clip,
                    );
                }
            }
        }
        pen += p.advance;
    }
}
