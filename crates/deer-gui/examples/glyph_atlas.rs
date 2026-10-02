//! # glyph_atlas —— 真实字形的**覆盖率图集**示例（M4）
//!
//! ## 这是什么
//!
//! 用 `deer-text`（L1 TextServer）自己的 TrueType 解析器 + 光栅化器把 **94 个可见 ASCII**（`!`..=`~`）
//! 在 **@16px 与 @24px** 两档字号下光栅化成覆盖率位图，再打进一张**货架打包**的
//! 字形图集（[`deer_text::atlas::GlyphAtlas`]），最后把整张图集写成 PNG。
//!
//! 图集是 GPU 侧文本渲染的必需品：所有字形共用一张纹理，绘制时按槽位采样。
//! 本示例不碰 GPU —— 它产出的是**那张纹理的 CPU 侧内容**，
//! 所以没有显卡也能看结果、能在 CI 里断言。
//!
//! ## 怎么跑
//!
//! ```sh
//! cargo run -p deer-gui --example glyph_atlas
//! ```
//!
//! ## 产物在哪
//!
//! `render_out/glyph_atlas.png`（8 位灰度：覆盖率 0 = 黑，255 = 白；留白处即图集空位）。
//! 找不到系统字体（`consola.ttf` / `arial.ttf` / `segoeui.ttf`）时会 **明确报错并以非 0 退出** ——
//! 示例是给人看的，不伪装成功。
//!
//! ## 自检（跑成功 ≠ 正确）
//!
//! ① 每个插入的 key 都能 `get()` 且字节与插入时逐字节一致；
//! ② 任意两槽位不重叠（`AtlasSlot::overlaps`）；
//! ③ `utilization()` 大于**实测下界**（见下）；
//! ④ 至少一个字形有 `max_coverage() == 255`（说明真的着墨了，不是全灰糊）；
//! ⑤ 两次独立构建的图集 `coverage()` 逐字节相同（确定性）。

// LY2：文本栈（解析 / 光栅化 / 图集 / 度量）来自 L1 crate `deer-text`；`png` 仍在 deer-gpu
// （它由 deer-gpu 以模块别名转发 deer-text 的编码器，路径 `deer_gpu::png::encode_rgba` 不变）。
use deer_gpu::png::encode_rgba;
use deer_text::atlas::GlyphAtlas;
use deer_text::font::Font;
use deer_text::glyph::{GlyphImage, GlyphKey};
use deer_text::measure::find_system_font;
use deer_text::raster::Rasterizer;

/// 图集宽度（像素）。高度初始 = 宽度，必要时按需增高（货架打包不会移动已有槽位）。
const ATLAS_WIDTH: u32 = 256;

/// 光栅化字号（像素）。
const SIZES: [f32; 2] = [16.0, 24.0];

/// 利用率下界。**实测值**（consola.ttf，256 宽图集，@16/@24 两档）：`0.3521`；
/// 这里取 0.30 当下界 —— 留出余量（换字体/字号会让它浮动），但明显高于
/// 「打包退化」时的量级（例如每行只放一个字形会掉到 0.05 上下）。
const MIN_UTILIZATION: f32 = 0.30;

fn main() -> std::process::ExitCode {
    match run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("glyph_atlas 示例失败：{e}");
            std::process::ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    // ── ① 字体来源：系统字体；找不到就明确失败（非 0 退出）──
    let path = find_system_font().ok_or_else(|| {
        r"没有找到系统字体（依次找 %WINDIR%\Fonts\consola.ttf / arial.ttf / segoeui.ttf）。\
本示例需要一份真实 TrueType 字体；请安装其中一个，或把字体放到该目录。"
            .to_string()
    })?;
    let data = std::fs::read(&path).map_err(|e| format!("读字体 {} 失败：{e}", path.display()))?;
    let font = Font::parse(data).map_err(|e| format!("解析字体 {} 失败：{e}", path.display()))?;
    println!("字体：{}", path.display());
    println!(
        "  unitsPerEm={}，字形数={}，cmap 格式={:?}",
        font.units_per_em, font.num_glyphs, font.cmap_format
    );

    // ── ② 构建图集（两次，用来做确定性自检 ⑤）──
    let (atlas, inserted) = build(&font)?;
    let (atlas_again, _) = build(&font)?;

    // ── ③ 自检 ①：每个 key 都能取回，且字节与插入时一致；顺带统计 ──
    let mut ink_pixels = 0usize;
    let mut max_coverage = 0u8;
    let mut slots = Vec::with_capacity(inserted.len());
    for (key, img) in &inserted {
        let (slot, bytes) = atlas
            .get(*key)
            .ok_or_else(|| format!("插入过 {key:?} 却 get() 不到"))?;
        if bytes != &img.coverage[..] {
            return Err(format!(
                "{key:?} 取回的 {} 字节与插入时不一致（前 8 字节：{:?} vs {:?}）",
                bytes.len(),
                &bytes[..bytes.len().min(8)],
                &img.coverage[..img.coverage.len().min(8)]
            ));
        }
        if bytes.len() != (slot.w * slot.h) as usize {
            return Err(format!("{key:?} 的切片长度应恒为 w*h"));
        }
        max_coverage = max_coverage.max(img.max_coverage());
        ink_pixels += img.ink_pixels(128);
        slots.push(slot);
    }

    // ── 自检 ②：任意两槽位不重叠 ──
    for i in 0..slots.len() {
        for j in (i + 1)..slots.len() {
            if slots[i].overlaps(&slots[j]) {
                return Err(format!(
                    "槽位重叠：{} {:?} 与 {} {:?}",
                    i, slots[i], j, slots[j]
                ));
            }
        }
    }

    // ── 自检 ④：至少一个字形是全实心（255）──
    if max_coverage != 255 {
        return Err(format!("没有任何字形达到 max_coverage()==255（实测 {max_coverage}）"));
    }

    // ── 自检 ⑤：两次构建逐字节相同 ──
    if atlas.coverage() != atlas_again.coverage() {
        return Err("两次构建图集的 coverage() 不一致（非确定性！）".to_string());
    }
    if atlas.size() != atlas_again.size() {
        return Err("两次构建图集的尺寸不一致".to_string());
    }

    // ── ④ 打印统计 ──
    let (aw, ah) = atlas.size();
    let utilization = atlas.utilization();
    println!("插入字形数：{}（{} 档字号 × 可见 ASCII）", atlas.len(), SIZES.len());
    println!("图集尺寸：{aw} × {ah}（{} 字节覆盖率）", atlas.coverage().len());
    println!("利用率：{utilization:.4}（used={} / area={}）", atlas.used_pixels(), aw as usize * ah as usize);
    println!("最大覆盖率：{max_coverage}");
    println!("墨迹像素数（覆盖率 ≥ 128 的像素，两档合计）：{ink_pixels}");
    println!("逐字节确定性：两次构建 coverage() 完全相同 ✅");

    // ── 自检 ③：利用率必须高于实测下界 ──
    if utilization <= MIN_UTILIZATION {
        return Err(format!(
            "利用率 {utilization:.4} 不高于下界 {MIN_UTILIZATION}（打包策略退化或统计口径错了）"
        ));
    }

    // ── ⑤ 覆盖率 → RGBA（0 = 黑，255 = 白）→ PNG ──
    let mut rgba = Vec::with_capacity(atlas.coverage().len() * 4);
    for &c in atlas.coverage() {
        rgba.extend_from_slice(&[c, c, c, 255]);
    }
    let png = encode_rgba(aw, ah, &rgba).map_err(|e| format!("PNG 编码失败：{e}"))?;
    std::fs::create_dir_all("render_out").map_err(|e| format!("创建 render_out 失败：{e}"))?;
    let out = "render_out/glyph_atlas.png";
    std::fs::write(out, &png).map_err(|e| format!("写 {out} 失败：{e}"))?;
    println!("产物：{out}（{aw}×{ah}，{} 字节）", png.len());

    // 未参与打包的字符数（透明诊断：cmap 缺失的可见 ASCII）
    let missing: usize = ('!'..='~')
        .filter(|&ch| !matches!(font.glyph_index(ch), Ok(Some(_))))
        .count();
    println!("cmap 未覆盖的可见 ASCII：{missing}");
    Ok(())
}

/// 构建一次图集：光栅化 `SIZES` 各档的全部可见 ASCII → 逐个插入。
///
/// 返回 `(图集, 插入清单)`；清单保留原图，供调用方做「取回 == 插入」的交叉验证。
fn build(font: &Font) -> Result<(GlyphAtlas, Vec<(GlyphKey, GlyphImage)>), String> {
    let mut atlas = GlyphAtlas::new(ATLAS_WIDTH);
    let mut inserted = Vec::new();

    for &px in &SIZES {
        let rasterizer = Rasterizer::new(px);
        for ch in '!'..='~' {
            // cmap 里没有这个字符 ⇒ 跳过（示例只打包真正存在的字形）
            let Some(glyph_index) = font
                .glyph_index(ch)
                .map_err(|e| format!("查 cmap({ch:?}) 失败：{e}"))?
            else {
                continue;
            };
            let Some(image) = rasterizer
                .rasterize_char(font, ch)
                .map_err(|e| format!("光栅化 {ch:?} 失败：{e}"))?
            else {
                continue;
            };
            let key = GlyphKey::new(glyph_index, px as u16);
            let slot = atlas.insert(key, &image).ok_or_else(|| {
                format!(
                    "图集放不下 {ch:?} @{px}px（位图 {}×{}）",
                    image.width, image.height
                )
            })?;
            debug_assert_eq!((slot.w, slot.h), (image.width, image.height));
            inserted.push((key, image));
        }
    }
    Ok((atlas, inserted))
}
