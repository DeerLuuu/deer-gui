//! M4 **字形图集**测试（t2）：货架打包、幂等、按需增高、空位图、明确失败、确定性。
//!
//! 全部用**手工构造的 `GlyphImage`**（不依赖字体文件、不依赖 rasterizer），
//! 于是每一条断言在任何机器上都确定可复现。覆盖率样本由算术式生成（无随机数）。
//!
//! 覆盖：⑦ 插入/取回逐字节一致 + 两两不重叠 + padding 为 0
//! ⑧ 同 key 幂等 ⑨ 增高不移动已有槽位 ⑩ 空位图不占空间
//! ⑪ 超宽/超高明确失败 + coverage 长度与「槽位外全 0」
//! ⑫ 同一插入序列 ⇒ 逐字节相同
//!
//! ## 变异测试记录（改坏 → 哪条断言变红 → 改回）
//!
//! | # | 变异 | 变红的断言（实测） |
//! |---|---|---|
//! | M1 | `atlas.rs` 里 `PADDING = 1` → `0` | ⑦ `padding_pixels_zero`：「槽位 {x:0,y:0,w:10,h:7} 的右侧 padding 非 0（相邻字形渗色）」；⑪ 超高失败断言（无 padding 后 2×8192 正好卡进 8192 上限，插入反而成功） |
//! | M2 | `insert` 去掉「同 key 去重」 | ⑧「同 key 必须返回已有槽位」（x=0 vs x=9）；⑫ 幂等复插检查（`Some(x:18,y:56)` vs `Some(x:0,y:0)`） |
//! | M3 | `ensure_height` 增高时把旧数据整体下移一行（模拟「增高时重排槽位」） | ⑨「增高后**图集大图**里槽位内容变了」（这才是 GPU 会采样到的那份数据）；⑪「coverage() 与独立重建的期望图不一致」 |
//!
//! M3 是**有价值的失败**：它暴露了初版测试 ⑨ 只看 `get()`（逐行紧致副本）的漏洞 ——
//! 增高移位后 `get()` 仍然正确、但图集大图已经错位。于是 ⑨ 补上了
//! `gather_from_coverage()` 这条独立读取路径的断言（现在 M3 会被 ⑨ 直接抓住）。

use deer_gpu::atlas::{GlyphAtlas, MAX_DIMENSION};
use deer_gpu::glyph::{AtlasSlot, GlyphImage, GlyphKey};

fn key(glyph_index: u16, px_size: u16) -> GlyphKey {
    GlyphKey::new(glyph_index, px_size)
}

/// 确定性的覆盖率样本：纯算术式，不依赖随机数/时间。
fn make_image(w: u32, h: u32, seed: u32) -> GlyphImage {
    let mut coverage = Vec::with_capacity((w * h) as usize);
    for y in 0..h {
        for x in 0..w {
            let v = (x * 37 + y * 91 + seed * 13 + 7) % 256;
            coverage.push(v as u8);
        }
    }
    GlyphImage::new(w, h, -(seed as i32), h as i32, w as f32 * 0.9, coverage)
}

/// 断言某个非空槽位右/下各 1px 的 padding 是 0（防 GPU 采样渗色）。
/// 返回实际检查到的像素数（0 表示该槽位在图集边缘，没有 padding 可查）。
fn padding_pixels_zero(atlas: &GlyphAtlas, slot: &AtlasSlot) -> usize {
    let (aw, ah) = atlas.size();
    let cov = atlas.coverage();
    let idx = |x: u32, y: u32| (y as usize) * (aw as usize) + (x as usize);
    let mut checked = 0usize;

    if slot.right() < aw {
        for y in slot.y..slot.bottom() {
            assert_eq!(
                cov[idx(slot.right(), y)],
                0,
                "槽位 {slot:?} 的右侧 padding 非 0（相邻字形渗色）"
            );
            checked += 1;
        }
    }
    if slot.bottom() < ah {
        for x in slot.x..slot.right() {
            assert_eq!(
                cov[idx(x, slot.bottom())],
                0,
                "槽位 {slot:?} 的下方 padding 非 0（相邻货架渗色）"
            );
            checked += 1;
        }
    }
    checked
}

/// 两两不重叠（用 `AtlasSlot::overlaps`，非空槽位必须互不重叠）。
fn assert_no_overlap(slots: &[AtlasSlot]) {
    for i in 0..slots.len() {
        for j in (i + 1)..slots.len() {
            assert!(
                !slots[i].overlaps(&slots[j]),
                "槽位 {} {:?} 与 {} {:?} 重叠",
                i,
                slots[i],
                j,
                slots[j]
            );
        }
    }
}

/// 从**整张图集** `coverage()` 里按槽位逐行抠出内容（GPU 采样实际会读到的那些字节）。
///
/// 这是与 `get()`（逐行紧致副本）**互相独立**的一条读取路径：
/// 「槽位仍然有效」的真正含义是「大图里这块区域还是那个字形」，
/// 所以断言必须落到 `coverage()` 上，而不是只看 `get()`。
fn gather_from_coverage(atlas: &GlyphAtlas, slot: &AtlasSlot) -> Vec<u8> {
    let (aw, _) = atlas.size();
    let cov = atlas.coverage();
    let mut out = Vec::with_capacity((slot.w * slot.h) as usize);
    for row in 0..slot.h {
        let start = (slot.y as usize + row as usize) * (aw as usize) + (slot.x as usize);
        out.extend_from_slice(&cov[start..start + slot.w as usize]);
    }
    out
}

/// 两个字节序列第一处不同的下标（诊断用：避免把几百 KB 的缓冲全打进失败信息）。
fn first_mismatch(a: &[u8], b: &[u8]) -> Option<usize> {
    a.iter().zip(b.iter()).position(|(x, y)| x != y)
}

/// ⑦ 手工构造若干 `GlyphImage` → `get()` 回来的字节**完全一致**，槽位两两不重叠。
#[test]
fn inserted_pixels_round_trip_and_slots_never_overlap() {
    let mut atlas = GlyphAtlas::new(64);
    assert_eq!(atlas.size(), (64, 64), "初始高度 = 宽度（正方形起步）");
    assert!(atlas.is_empty());

    // (宽, 高, seed)：混入小字形/宽字形/高字形，逼出多次换行
    let specs = [
        (10u32, 7u32, 1u32),
        (23, 9, 2),
        (5, 30, 3),
        (18, 18, 4),
        (31, 4, 5),
        (2, 2, 6),
        (12, 12, 7),
    ];
    let mut inserted = Vec::new();
    for (i, (w, h, seed)) in specs.iter().enumerate() {
        let img = make_image(*w, *h, *seed);
        let k = key(i as u16 + 1, 16);
        let slot = atlas.insert(k, &img).expect("应当放得下");
        assert_eq!((slot.w, slot.h), (*w, *h), "槽位尺寸必须等于位图尺寸");
        inserted.push((k, img, slot));
    }
    assert_eq!(atlas.len(), specs.len());
    assert!(!atlas.is_empty());

    let mut checked_padding = 0usize;
    for (k, img, slot) in &inserted {
        assert!(atlas.contains(*k));
        let (got_slot, bytes) = atlas.get(*k).expect("插入过就必须能取回");
        assert_eq!(got_slot, *slot, "槽位必须稳定");
        assert_eq!(
            bytes.len(),
            (slot.w * slot.h) as usize,
            "切片长度恒为 w*h"
        );
        assert_eq!(bytes, &img.coverage[..], "取回的字节必须与插入时逐字节一致");

        // 两份数据互相印证：get() 的逐行紧致副本 == 从 coverage() 大图里按 slot 抠出来的内容
        let gathered = gather_from_coverage(&atlas, slot);
        assert_eq!(
            bytes,
            &gathered[..],
            "get() 的副本与整张图集 coverage() 里的同一槽位不一致"
        );

        checked_padding += padding_pixels_zero(&atlas, slot);
    }
    let slots: Vec<AtlasSlot> = inserted.iter().map(|(_, _, s)| *s).collect();
    assert_no_overlap(&slots);
    assert!(
        checked_padding > 0,
        "至少要检查到一些 padding 像素（否则这条断言是空转）"
    );
    println!(
        "⑦ {} 个字形全部逐字节一致；槽位两两不重叠；检查了 {checked_padding} 个 padding 像素（全 0）",
        inserted.len()
    );
}

/// ⑧ 幂等：重复 `insert` 同 key 不增加 `len()`、不改变槽位与内容。
#[test]
fn duplicate_insert_is_idempotent() {
    let mut atlas = GlyphAtlas::new(32);
    let img = make_image(8, 8, 1);
    let k = key(1, 16);

    let s1 = atlas.insert(k, &img).expect("第一次插入");
    let len1 = atlas.len();
    let used1 = atlas.used_pixels();
    let coverage1 = atlas.coverage().to_vec();

    let s2 = atlas.insert(k, &img).expect("第二次插入");
    assert_eq!(s1, s2, "同 key 必须返回已有槽位");
    assert_eq!(atlas.len(), len1, "len() 不许增加");
    assert_eq!(atlas.used_pixels(), used1, "used_pixels 不许增加");
    assert_eq!(atlas.coverage(), &coverage1[..], "coverage 不许被改写");

    // 更狠一点：同 key 换一张**内容不同**的图，也必须返回原槽位且不覆盖旧数据
    let other = make_image(8, 8, 99);
    let s3 = atlas.insert(k, &other).expect("同 key 第三次插入");
    assert_eq!(s3, s1, "同 key 换图也必须复用原槽位（幂等优先）");
    assert_eq!(atlas.len(), len1);
    assert_eq!(
        atlas.get(k).expect("取回").1,
        &img.coverage[..],
        "已有数据不许被后来的同 key 覆盖"
    );
    println!("⑧ 幂等：len={} 保持，槽位 {s1:?} 稳定，旧数据未被覆盖", atlas.len());
}

/// ⑨ 按需增高：增高**前**拿到的槽位，增高**后**读到的字节仍然一致。
#[test]
fn growth_keeps_existing_slots_valid() {
    let mut atlas = GlyphAtlas::new(64);
    assert_eq!(atlas.size(), (64, 64));

    // 先插 6 个 20×20（每货架放 3 个，行高 21）
    let mut early = Vec::new();
    for i in 0..6u16 {
        let img = make_image(20, 20, u32::from(i) + 1);
        let k = key(i + 1, 16);
        let slot = atlas.insert(k, &img).expect("放得下");
        early.push((k, img, slot));
    }
    let height_before = atlas.size().1;
    let slots_before: Vec<AtlasSlot> = early.iter().map(|(_, _, s)| *s).collect();

    // 继续插到必然增高（64 宽 × 20+1 需要 14 个货架才到 294 行）
    let mut all = early.clone();
    for i in 6..40u16 {
        let img = make_image(20, 20, u32::from(i) + 1);
        let k = key(i + 1, 16);
        let slot = atlas.insert(k, &img).expect("放得下");
        assert_eq!(slot.w, 20);
        all.push((k, img, slot));
    }
    let (aw, ah) = atlas.size();
    assert!(ah > height_before, "必须真的增高了：之前 {height_before}，现在 {ah}");
    assert_eq!(aw, 64, "增高不许改变宽度");
    assert_eq!(atlas.coverage().len(), (aw * ah) as usize);
    println!("⑨ 增高：{height_before} → {ah} 行（共 {} 个字形）", all.len());

    // 增高后，增高前发出的槽位**坐标不变**，并且三条读取路径都必须一致：
    //   ① `get()`（逐行紧致副本）② **整张图集 `coverage()` 里该槽位那块区域**（GPU 采样的真值）
    for ((k, img, slot), slot_before) in early.iter().zip(slots_before.iter()) {
        let (now, bytes) = atlas.get(*k).expect("增高后仍能取回");
        assert_eq!(now, *slot_before, "增高不得移动已有槽位（这是选货架算法的原因）");
        assert_eq!(now, *slot, "槽位与插入时返回的一致");
        assert_eq!(bytes, &img.coverage[..], "增高后 get() 的副本变了");

        let gathered = gather_from_coverage(&atlas, &now);
        assert_eq!(gathered.len(), img.coverage.len(), "槽位尺寸与位图不匹配");
        if let Some(pos) = first_mismatch(&gathered, &img.coverage) {
            panic!(
                "增高后**图集大图**里槽位 {now:?} 的内容变了（数据被搬移/错位）：\
                 首个不一致下标 {pos}，图集值 {} vs 插入值 {} ——\
                 这才是 GPU 真正会采样到的那份数据",
                gathered[pos], img.coverage[pos]
            );
        }
    }

    // 每一个字形（含增高之后插入的）都要能在**大图**里按槽位正确读出
    for (k, img, slot) in &all {
        let (now, _) = atlas.get(*k).expect("取回");
        assert_eq!(now, *slot);
        let gathered = gather_from_coverage(&atlas, slot);
        assert_eq!(gathered.len(), img.coverage.len(), "槽位尺寸与位图不匹配");
        if let Some(pos) = first_mismatch(&gathered, &img.coverage) {
            panic!(
                "字形 {k:?} 在增高后的图集里错位：槽位 {slot:?}，首个不一致下标 {pos}\
                 （图集值 {} vs 插入值 {}）",
                gathered[pos], img.coverage[pos]
            );
        }
    }
    // 早期槽位与全部槽位都不重叠
    assert_no_overlap(&slots_before);
    assert_no_overlap(&all.iter().map(|(_, _, s)| *s).collect::<Vec<_>>());
}

/// ⑩ 空位图（空格）：`get()` 返回空切片，不占空间，也不拉低利用率。
#[test]
fn blank_image_registers_without_using_space() {
    let mut atlas = GlyphAtlas::new(32);
    let ink = make_image(6, 6, 1);
    let k_ink = key(1, 16);
    let slot_ink = atlas.insert(k_ink, &ink).expect("插入墨迹字形");

    let used_before = atlas.used_pixels();
    let util_before = atlas.utilization();
    let (w_before, h_before) = atlas.size();

    let blank = GlyphImage::blank(4.0);
    assert!(blank.is_blank());
    let k_blank = key(2, 16);
    let slot_blank = atlas.insert(k_blank, &blank).expect("空位图也必须登记成功");

    assert_eq!(
        slot_blank,
        AtlasSlot {
            x: 0,
            y: 0,
            w: 0,
            h: 0
        },
        "空位图返回零尺寸槽位"
    );
    assert!(atlas.contains(k_blank), "空位图仍然登记 key");
    assert_eq!(atlas.len(), 2, "登记数包含空位图");
    assert_eq!(atlas.get(k_blank).expect("取回").1.len(), 0, "空切片");
    assert_eq!(atlas.used_pixels(), used_before, "空位图不占空间");
    assert_eq!(
        atlas.utilization(),
        util_before,
        "空位图不许拉低利用率"
    );
    assert_eq!(atlas.size(), (w_before, h_before), "空位图不许触发增高");
    assert!(!slot_blank.overlaps(&slot_ink), "空槽位视为不重叠");

    // 之后的正常插入不受影响
    let later = make_image(4, 4, 3);
    let slot_later = atlas.insert(key(3, 16), &later).expect("继续插入");
    assert!(!slot_blank.overlaps(&slot_later));
    assert_eq!(slot_later.w, 4);
    println!(
        "⑩ 空位图：槽位 {slot_blank:?}，利用率保持 {util_before}，不占 {used_before} 之外的像素"
    );
}

/// ⑪ 明确失败（超宽 / 超高）+ `coverage()` 长度与「槽位外全 0」。
#[test]
fn oversized_glyphs_fail_loudly_and_coverage_stays_exact() {
    // ── 超宽 ──
    let mut atlas = GlyphAtlas::new(16);
    let too_wide = make_image(17, 4, 1);
    assert!(
        atlas.insert(key(1, 16), &too_wide).is_none(),
        "宽 17 > 图集宽 16 ⇒ 必须返回 None"
    );
    assert!(!atlas.contains(key(1, 16)), "失败的插入不许登记 key");
    assert_eq!(atlas.len(), 0);
    assert_eq!(atlas.size(), (16, 16), "失败不许改动图集尺寸");
    assert_eq!(atlas.used_pixels(), 0);

    // ── 超高（会超过 8192 上限）──
    // 图集宽 12：先放一个 10×10（横向只够一个），于是下面必然要**新开货架**，
    // 覆盖「新货架 y > 0 时也要撞上限」这条路。
    let mut atlas = GlyphAtlas::new(12);
    let _ = atlas.insert(key(1, 16), &make_image(10, 10, 5)).expect("先立一个货架");
    let y_after = atlas.get(key(1, 16)).expect("取回").0.y;
    // h = 8192 ⇒ need_h = 8193 > MAX_DIMENSION，无论 y 是 0 还是 11 都必须明确失败
    let tall = make_image(2, MAX_DIMENSION, 2);
    assert!(
        atlas.insert(key(2, 16), &tall).is_none(),
        "增高会超过 MAX_DIMENSION({MAX_DIMENSION}) ⇒ 必须返回 None"
    );
    assert!(!atlas.contains(key(2, 16)));
    assert_eq!(
        atlas.size(),
        (12, 12),
        "失败的增高不许改尺寸（初始高 = 宽 = 12，10×10 放得下不用增高）"
    );
    println!(
        "⑪ 失败留痕检查：图集仍只有 {} 个字形，尺寸 {:?}，货架 y={y_after}",
        atlas.len(),
        atlas.size()
    );

    // ── coverage 的长度与内容：用独立「期望图」逐像素对比 ──
    let mut atlas = GlyphAtlas::new(48);
    let mut inserted = Vec::new();
    let specs = [
        (13u32, 9u32, 1u32),
        (7, 21, 2),
        (29, 5, 3),
        (4, 4, 4),
        (11, 11, 5),
        (22, 8, 6),
        (3, 17, 7),
        (16, 16, 8),
    ];
    for (i, (w, h, seed)) in specs.iter().enumerate() {
        let img = make_image(*w, *h, *seed);
        let k = key(i as u16 + 1, 24);
        let slot = atlas.insert(k, &img).expect("放得下");
        inserted.push((k, img, slot));
    }
    let (aw, ah) = atlas.size();
    assert_eq!(
        atlas.coverage().len(),
        (aw * ah) as usize,
        "coverage 长度恒为 宽*高"
    );

    // 独立重建一张期望图：槽位内 = 插入的字节，其余（含 padding）= 0
    let mut expect = vec![0u8; (aw * ah) as usize];
    for (_, img, slot) in &inserted {
        for (row, src) in img.coverage.chunks(slot.w as usize).enumerate() {
            if row >= slot.h as usize {
                break;
            }
            let dst = (slot.y as usize + row) * (aw as usize) + slot.x as usize;
            expect[dst..dst + src.len()].copy_from_slice(src);
        }
    }
    if let Some(pos) = first_mismatch(atlas.coverage(), &expect) {
        let (x, y) = (pos % aw as usize, pos / aw as usize);
        panic!(
            "coverage() 与独立重建的期望图不一致：首个不同在下标 {pos}（x={x}, y={y}）\
             ——图集 {} vs 期望 {}。槽位外必须是 0、槽位内必须原样",
            atlas.coverage()[pos],
            expect[pos]
        );
    }

    // 非零像素数 = 各图非零数之和（没有重叠/渗漏）
    let nonzero_expected: usize = expect.iter().filter(|&&v| v != 0).count();
    let nonzero_actual: usize = atlas.coverage().iter().filter(|&&v| v != 0).count();
    assert_eq!(nonzero_actual, nonzero_expected);

    // 统计口径与槽位一致
    let slots: Vec<AtlasSlot> = inserted.iter().map(|(_, _, s)| *s).collect();
    assert_no_overlap(&slots);
    let expected_used: usize = slots.iter().map(|s| (s.w * s.h) as usize).sum();
    assert_eq!(atlas.used_pixels(), expected_used, "Σ slot.w*slot.h");
    let expected_util = expected_used as f32 / (aw as f32 * ah as f32);
    assert_eq!(atlas.utilization(), expected_util);
    println!(
        "⑪ coverage 恒为 {aw}×{ah}={} 字节；非零像素 {nonzero_actual}；利用率 {}（used={expected_used}）",
        atlas.coverage().len(),
        atlas.utilization()
    );
}

/// ⑫ 确定性：两次相同插入序列 ⇒ 相同槽位布局 + 逐字节相同的 `coverage()`。
#[test]
fn same_insert_sequence_is_byte_identical() {
    fn build() -> (GlyphAtlas, Vec<(GlyphKey, AtlasSlot)>) {
        let mut atlas = GlyphAtlas::new(40);
        let specs = [
            (9u32, 13u32, 1u32),
            (17, 6, 2),
            (6, 25, 3),
            (20, 20, 4),
            (2, 2, 5),
            (14, 9, 6),
            (8, 8, 7),
            (25, 3, 8),
            (11, 30, 9),
            (5, 5, 10),
        ];
        let mut slots = Vec::new();
        for (i, (w, h, seed)) in specs.iter().enumerate() {
            let img = make_image(*w, *h, *seed);
            let k = key(i as u16 + 1, 32);
            slots.push((k, atlas.insert(k, &img).expect("放得下")));
        }
        (atlas, slots)
    }

    let (a1, s1) = build();
    let (a2, s2) = build();
    assert_eq!(s1, s2, "同一序列必须给出相同槽位");
    assert_eq!(a1.size(), a2.size());
    assert_eq!(a1.len(), a2.len());
    assert_eq!(a1.used_pixels(), a2.used_pixels());
    assert_eq!(
        a1.coverage(),
        a2.coverage(),
        "coverage 必须逐字节相同（无 HashMap 迭代序/时间/随机依赖）"
    );

    // 同一图集内重复插同一序列也不改变任何东西（幂等 + 确定性）
    let (mut a3, _) = build();
    let before = a3.coverage().to_vec();
    for (k, slot) in &s1 {
        let img = make_image(slot.w, slot.h, 0); // 内容不同也无所谓：幂等优先
        assert_eq!(a3.insert(*k, &img), Some(*slot));
    }
    assert_eq!(a3.coverage(), &before[..]);
    println!(
        "⑫ 两次构建：{} 字节 coverage 逐字节相同，槽位 {:?}",
        a1.coverage().len(),
        a1.size()
    );
}

/// 边界：宽度 0 的图集只能登记空位图（其余一律明确失败）。
#[test]
fn zero_width_atlas_only_accepts_blank() {
    let mut atlas = GlyphAtlas::new(0);
    assert_eq!(atlas.size(), (0, 0));
    assert_eq!(atlas.coverage().len(), 0);
    assert_eq!(atlas.utilization(), 0.0, "面积为 0 时利用率为 0，不许 NaN");

    let blank = GlyphImage::blank(5.0);
    let slot = atlas.insert(key(1, 16), &blank).expect("空位图仍可登记");
    assert_eq!(slot, AtlasSlot { x: 0, y: 0, w: 0, h: 0 });
    assert_eq!(atlas.get(key(1, 16)).expect("取回").1.len(), 0);
    assert!(atlas.insert(key(2, 16), &make_image(1, 1, 1)).is_none());
    assert_eq!(atlas.len(), 1);
    println!("边界：宽 0 图集：空位图 OK、1×1 明确失败、利用率 0.0");
}
