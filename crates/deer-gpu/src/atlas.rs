//! **字形图集**（M4）：把 [`GlyphImage`] 打包进一张覆盖率纹理（GPU 侧文本渲染的必需品）。
//!
//! ## 为什么是「货架（shelf）打包」而不是紧凑装箱
//!
//! 图集必须**按需增高**（字形是懒加载的），而增高最容易出错的地方是
//! 「增高的同时把已有字形挪了位置」—— 已发给渲染器的 [`AtlasSlot`] 立刻失效，
//! 表现出来就是文字错位/串行。货架打包的增高**只追加新行**：
//!
//! ```text
//!   ┌──────────────────────── 图集宽 ────────────────────────┐
//!   │ [A][B][C]                                              │  ← 货架 0（行高 = 该行最高字形 + 1px 下 padding）
//!   │ [D][E]                                                 │  ← 货架 1（新开一行）
//!   │ [F]                                                    │  ← 货架 2
//!   │                    ← 增高只在这里追加，上面两行**不动** │
//!   └────────────────────────────────────────────────────────┘
//! ```
//!
//! 每个字形**右、下各留 1px padding**：GPU 双线性采样会取到相邻纹素的加权和，
//! 不留边距时旁边字形的墨迹会「渗」进来（经典 atlas bleeding）。
//! padding 用 0 填充，`coverage()` 里槽位之外**必须全 0**（有测试钉住）。
//!
//! ## 不变式（都有测试）
//!
//! - `coverage().len() == 宽 * 高`；
//! - 任意两个非空槽位不重叠（[`AtlasSlot::overlaps`]）；
//! - 槽位之外的像素恒为 0；
//! - 重复 `insert` 同一 key 返回**原槽位**（幂等，不重复占位）；
//! - 同一插入序列 ⇒ 相同槽位布局 + 逐字节相同的 `coverage()`（确定性）；
//! - 失败是**明确失败**：超宽 / 超高返回 `None`，不 panic、不静默截断。
//!
//! ## 为什么每个字形还存一份**逐行紧致**的副本
//!
//! [`GlyphAtlas::get`] 的签名要求返回**一个连续的** `&[u8]`（长度 `w * h`）。
//! 但货架打包里，槽位是图集大图里的一块**带行跨度**的区域
//! （`slot.w < 图集宽` 时相邻行之间有 padding 与别的字形），
//! 连续切片根本表达不了它 —— 而在 `#![forbid(unsafe_code)]` 下也不能
//! 用「跳过 stride」的包装切片。
//!
//! 所以图集维护两份数据：
//! 1. `coverage`：整张图集（**给 GPU 上传用的唯一真值**，含 padding）；
//! 2. 每个 key 的 `pixels`：逐行紧致副本（**只给 `get()` 借用**）。
//!
//! 代价是每像素约 1 字节的重复内存（覆盖率图本来就只有 8 位/像素），
//! 换来的是「`get()` 与图集内容互为交叉验证」—— 两边都对上才算对
//! （测试同时断言了 `get()` == 插入的字节 和 `coverage()` == 独立重建的期望图）。

use crate::glyph::{AtlasSlot, GlyphImage, GlyphKey};

/// 每个字形右/下各留的 padding（像素）。
const PADDING: u32 = 1;

/// 图集高度上限（像素）。超过就 [`GlyphAtlas::insert`] 明确失败。
///
/// 8192 是移动 GPU 上常见的 `maxTextureDimension2D` 下界，取它当保守上限。
pub const MAX_DIMENSION: u32 = 8192;

/// 一条登记记录。
struct Entry {
    key: GlyphKey,
    slot: AtlasSlot,
    /// 逐行紧致副本，长度恒为 `slot.w * slot.h`（空位图是空 `Vec`）。
    pixels: Vec<u8>,
}

/// 一张货架打包的 8 位覆盖率图集。
pub struct GlyphAtlas {
    /// 图集宽（像素）。**构造后不变** —— 这是「增高不移动已有槽位」的前提。
    width: u32,
    /// 图集高（像素）。按需增高。
    height: u32,
    /// 整张覆盖率图，行优先，`len == width * height`。
    coverage: Vec<u8>,
    /// 已登记的字形，按插入顺序（保证确定性）。
    entries: Vec<Entry>,
    /// 当前货架的顶边 y（`shelf_h == 0` 时无意义）。
    shelf_y: u32,
    /// 当前货架已占高度（含下 padding）；`0` = 还没有货架。
    shelf_h: u32,
    /// 当前货架内下一个槽位的 x。
    pen_x: u32,
    /// `Σ slot.w * slot.h`（不含 padding）。
    used: usize,
}

impl GlyphAtlas {
    /// 新图集：宽 `width`，**初始高度 = width**（正方形起步），之后按需增高。
    ///
    /// `width == 0` 也是合法的：任何非空字形都会因为「宽超限」插入失败，
    /// 只有空位图能被登记（见 [`GlyphAtlas::insert`]）。
    pub fn new(width: u32) -> GlyphAtlas {
        GlyphAtlas {
            width,
            height: width,
            coverage: vec![0u8; width as usize * width as usize],
            entries: Vec::new(),
            shelf_y: 0,
            shelf_h: 0,
            pen_x: 0,
            used: 0,
        }
    }

    /// 查询已登记的字形槽位。
    fn slot_of(&self, key: GlyphKey) -> Option<AtlasSlot> {
        self.entries
            .iter()
            .find(|e| e.key == key)
            .map(|e| e.slot)
    }

    /// 插入一个字形。
    ///
    /// 返回 `None` 的**唯一**两种情形（明确失败，不改动任何状态）：
    /// ① `image.width > 图集宽`；
    /// ② 放不下且增高后会超过 [`MAX_DIMENSION`]。
    ///
    /// 空位图（`w == 0 || h == 0`，如空格、无轮廓字形）：仍然登记 key，
    /// 返回 `AtlasSlot { x: 0, y: 0, w: 0, h: 0 }`，**不占空间**；
    /// 之后 `get()` 返回空切片。
    pub fn insert(&mut self, key: GlyphKey, image: &GlyphImage) -> Option<AtlasSlot> {
        // 幂等：同 key 已在图集里就直接复用（也保证「不重复占位」）
        if let Some(slot) = self.slot_of(key) {
            return Some(slot);
        }

        // 空位图：只登记身份，不占空间
        if image.width == 0 || image.height == 0 {
            self.entries.push(Entry {
                key,
                slot: AtlasSlot {
                    x: 0,
                    y: 0,
                    w: 0,
                    h: 0,
                },
                pixels: Vec::new(),
            });
            return Some(AtlasSlot {
                x: 0,
                y: 0,
                w: 0,
                h: 0,
            });
        }

        // 宽度超限：明确失败
        if image.width > self.width {
            return None;
        }

        // 需要占的格子（含右/下 padding）
        let need_w = image.width + PADDING;
        let need_h = image.height + PADDING;

        // 先**试算**位置，全部合法性检查通过后才提交（失败不留半成品状态）
        let shelf_exists = self.shelf_h > 0;
        // 能塞进当前货架吗？——注意「货架右上角还有没有 need_w 这么宽」，
        // 而**不是**「字形本体放不放得下」：字形本体宽度上面已经检查过了。
        // （`shelf_h > 0` 时必有 `pen_x >= 2`，所以不用额外处理「空货架」这种不可能状态。）
        let reuse_shelf = shelf_exists && self.pen_x + need_w <= self.width;
        let (x, y, new_shelf_h) = if reuse_shelf {
            (self.pen_x, self.shelf_y, self.shelf_h.max(need_h))
        } else {
            let y = if shelf_exists {
                self.shelf_y + self.shelf_h
            } else {
                0
            };
            // 新货架从 x = 0 开始；字形本体宽度已在上面的检查里保证放得下
            (0, y, need_h)
        };

        // 增高上限：明确失败而不是截断
        if y + new_shelf_h > MAX_DIMENSION {
            return None;
        }

        // ── 提交 ──
        self.shelf_y = y;
        self.shelf_h = new_shelf_h;
        self.pen_x = x + need_w;
        self.ensure_height(y + new_shelf_h);

        // 把覆盖率拷进槽位（`width` 不变 ⇒ 增高只需在尾部补零，已有行原地不动）
        let slot = AtlasSlot {
            x,
            y,
            w: image.width,
            h: image.height,
        };

        // ① 先做逐行紧致的规范化副本（长度恒为 w*h：短了补 0，长了截断）。
        //    这张副本是 `get()` 的返回物 —— 大图里槽位是**跨行**的，借不出连续切片。
        let w = image.width as usize;
        let total = w * image.height as usize;
        let mut pixels = vec![0u8; total];
        for (row, src) in image.coverage.chunks(w).enumerate() {
            if row >= image.height as usize {
                break;
            }
            let n = src.len().min(w);
            pixels[row * w..row * w + n].copy_from_slice(&src[..n]);
        }

        // ② 再写进大图（GPU 上传用的那份）
        let stride = self.width as usize;
        for (row, src) in pixels.chunks(w).enumerate() {
            let dst = (y as usize + row) * stride + x as usize;
            self.coverage[dst..dst + w].copy_from_slice(src);
        }

        self.used += slot.w as usize * slot.h as usize;
        self.entries.push(Entry { key, slot, pixels });
        Some(slot)
    }

    /// 取回某个字形：`(槽位, 覆盖率切片)`，切片长度**恒为 `w * h`**（空位图是空切片）。
    ///
    /// 返回的是**逐行紧致的副本**（见模块文档「为什么每个字形还存一份副本」），
    /// 即 `get().1` 与「从 [`GlyphAtlas::coverage`] 里按 `slot` 逐行抠出来」逐字节相同；
    /// 调用方不必自己处理行跨度/padding。
    ///
    /// 借用注意：切片借用 `self`，只要它还活着就不能调用 `&mut self` 的
    /// [`GlyphAtlas::insert`]（借用检查器会拦住），所以不存在「切片指向的缓冲被
    /// 重新分配」的悬垂问题；**槽位坐标** `AtlasSlot` 是 `Copy`，可以长期保存 ——
    /// 增高不会移动它。
    pub fn get(&self, key: GlyphKey) -> Option<(AtlasSlot, &[u8])> {
        let e = self.entries.iter().find(|e| e.key == key)?;
        Some((e.slot, e.pixels.as_slice()))
    }

    /// 是否已登记该 key。
    pub fn contains(&self, key: GlyphKey) -> bool {
        self.slot_of(key).is_some()
    }

    /// 图集尺寸 `(宽, 高)`（高会随插入增高）。
    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// 整张覆盖率图（行优先，`len == 宽 * 高`）。
    pub fn coverage(&self) -> &[u8] {
        &self.coverage
    }

    /// 已登记的字形数（含空位图）。
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// 是否一个字形都没登记。
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// `Σ slot.w * slot.h`（**不含 padding**，即真实墨迹面积的上界）。
    pub fn used_pixels(&self) -> usize {
        self.used
    }

    /// 空间利用率：`used_pixels / (宽 * 高)`（面积非 0 时落在 `0.0..=1.0`）。
    pub fn utilization(&self) -> f32 {
        let area = self.width as usize * self.height as usize;
        if area == 0 {
            return 0.0;
        }
        self.used as f32 / area as f32
    }

    /// 把图集高度至少扩到 `h`。
    ///
    /// **关键点**：`width` 构造后不变 ⇒ 行优先缓冲的行跨度不变 ⇒
    /// 「加高」等价于在**尾部补零**，已有像素原地不动，
    /// 因此所有已发出的 [`AtlasSlot`] 坐标仍然有效。
    fn ensure_height(&mut self, h: u32) {
        if h > self.height {
            self.height = h;
            self.coverage.resize(self.width as usize * h as usize, 0);
        }
    }
}
