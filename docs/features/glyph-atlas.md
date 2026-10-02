# 功能指南：字形图集（glyph-atlas）

> 状态 ✅ · 示例 `cargo run -p deer-gui --example glyph_atlas` ·
> 清单条目见 [`FEATURES.md`](../../FEATURES.md)

## 1. 这是什么 / 什么时候用它

把一堆[已光栅化的字形位图](glyph-raster.md)（`GlyphImage` 的覆盖率缓冲）
**打包进一张大图**，并给每个字形返回一个坐标（`AtlasSlot`）。
这是 GPU 侧文本渲染的前提：一次绑定纹理、多次画四边形，而不是每个字符一张纹理。

**什么时候用它**：你要把字形传给 GPU（上传成一张纹理），或者想把一批字形位图
拼成一张图导出查看。自己写文字渲染管线时，图集就是「字形缓存」那一层。

**什么时候不该用它**：只想让文字出现在图里 —— 用
[`text-rendering.md`](text-rendering.md) 的 `TextEngine`，它内部已经持有图集并管好了
「命中缓存 → 缺失才光栅化 → 入图集」的循环。也不要把它当**通用**的矩形装箱器用：
它是为字形优化过的（见第 6 节边界），没有旋转、没有紧凑重排。

## 2. 最小示例

```rust
use deer_text::atlas::GlyphAtlas;
use deer_text::glyph::{GlyphImage, GlyphKey};

fn main() {
    // 宽度 256，初始高度也是 256（正方形起步，按需往下长）
    let mut atlas = GlyphAtlas::new(256);

    // 手工构造一个 2×2 的覆盖率位图（真实场景里来自 Rasterizer）
    let img = GlyphImage::new(2, 2, 0, 2, 4.0, vec![0, 128, 255, 255]);
    let key = GlyphKey::new(36, 16);        // 字形索引 36，字号 16px

    let slot = atlas.insert(key, &img).expect("图集放得下");
    assert_eq!((slot.w, slot.h), (2, 2));

    // 取回来必须与插入的逐字节一致
    let (slot2, bytes) = atlas.get(key).expect("刚插入就能取到");
    assert_eq!(slot, slot2);
    assert_eq!(bytes, img.coverage.as_slice());

    println!(
        "图集 {}×{}，已登记 {} 个字形，用掉 {} 像素，利用率 {:.3}",
        atlas.size().0, atlas.size().1, atlas.len(),
        atlas.used_pixels(), atlas.utilization()
    );
}
```

跑完整版（光栅化 94 个可见 ASCII，@16 与 @24 两档，写出图集 PNG）：

```sh
cargo run -p deer-gui --example glyph_atlas
```

产物 `render_out/glyph_atlas.png`（覆盖率 0 = 黑、255 = 白，留白处就是图集空位）。

本机实测（`C:\Windows\Fonts\consola.ttf`，256 宽图集，@16/@24 两档，`exit=0`）：

```text
插入字形数：188（2 档字号 × 可见 ASCII）
图集尺寸：256 × 256（65536 字节覆盖率）
利用率：0.3521（used=23077 / area=65536）
最大覆盖率：255
cmap 未覆盖的可见 ASCII：0
```

数字随字体、字号与图集宽度而变 —— **以你自己的输出为准**；示例自己的下界是 0.30。

## 3. 完整 API

### `deer_text::atlas::GlyphAtlas`

| 方法 | 说明 |
|---|---|
| `GlyphAtlas::new(width: u32)` | 图集宽度固定为 `width`；**初始高度 = width**（正方形起步），不够时按需增高。`width == 0` 也合法，但此时任何非空字形都会插入失败（只有空位图能登记） |
| `insert(&mut self, key: GlyphKey, image: &GlyphImage) -> Option<AtlasSlot>` | 放入一个新字形并返回槽位。**同一个 key 重复插入是幂等的**：直接返回已有槽位、不重复占位。放不下（超宽 / 高度会超过 8192）返回 `None`，且**不改动任何状态** |
| `get(&self, key: GlyphKey) -> Option<(AtlasSlot, &[u8])>` | 取槽位 + 该字形的覆盖率切片。**切片长度恒为 `slot.w * slot.h`**；空位图返回空切片 |
| `contains(&self, key: GlyphKey) -> bool` | 这个字形是否已登记 |
| `size(&self) -> (u32, u32)` | `(宽, 高)`。高度会随插入增长，**宽度不变** |
| `coverage(&self) -> &[u8]` | 整张图集的覆盖率缓冲，行优先，`len == 宽 * 高`。槽位之外的像素**恒为 0** |
| `len(&self) -> usize` | 已登记字形数（含空位图；幂等插入不增加） |
| `is_empty(&self) -> bool` | `len() == 0` |
| `used_pixels(&self) -> usize` | `Σ slot.w * slot.h`（**不含** 1px padding；空位图贡献 0） |
| `utilization(&self) -> f32` | `used_pixels / (宽 * 高)`（面积非 0 时落在 `0.0..=1.0`） |

高度上限是公开常量 `deer_text::atlas::MAX_DIMENSION = 8192`。

### `deer_text::glyph::GlyphKey` —— 字形的身份

```rust
GlyphKey { glyph_index: u16, px_size: u16 }
GlyphKey::new(glyph_index, px_size)   // px_size 取 max(1)：0 按 1 处理
```

- `glyph_index` 是**字形索引**，不是字符：多个字符（如 `' '` 与不换行空格）可以映射到同一字形。
- **同一个字形在不同字号下是两条记录** —— 位图按像素大小不同，必须分开缓存。
- `px_size` 是**取整后的字号**：字号连续变化时图集不会无限膨胀（等价于「按字号分桶」，
  这也是位图字体缓存的通行做法）。所以 `16.4` 与 `16.0` 会命中同一条记录，
  而 `16` 与 `17` 是两条。

`GlyphKey` 实现了 `Hash`/`Eq`/`Ord`（可以当 `HashMap`/`BTreeMap` 的键，排序稳定 ⇒ 确定性）。

### `deer_text::glyph::AtlasSlot`

```rust
AtlasSlot { x: u32, y: u32, w: u32, h: u32 }   // 图集左上角为原点，单位像素
```

| 方法 | 说明 |
|---|---|
| `right()` / `bottom()` | `x + w` / `y + h` |
| `overlaps(&other) -> bool` | 是否重叠。**边界相接不算重叠**；空槽位（`w==0` 或 `h==0`）永不重叠 |

不变式：`x + w <= 图集宽`、`y + h <= 图集高`、**任意两个槽位不重叠**。
`overlaps()` 就是给你断言这条用的。

### 打包算法与保证

| 项 | 行为 |
|---|---|
| 算法 | **货架（shelf）打包**：字形按插入顺序放进当前货架（行），该行高度 = 行内最高字形；放不下就新开一行 |
| padding | 每个字形**右、下各留 1px**，防止双线性采样/溢出采样吃到邻居 |
| 增高 | 行放不下且下方没空间时**整张图集往下增高**；**已有槽位坐标不变、不搬动** |
| 幂等 | 同 key 重复 `insert` → 返回已有槽位，`len()` 不变 |
| 空位图 | `w==0 || h==0`（如空格）：**仍然登记 key**，返回 `AtlasSlot{0,0,0,0}`，不占空间，`get()` 返回空切片 |
| 超限 | `image.width > 图集宽`，或需要的高度会超过上限 **8192** ⇒ 返回 `None`（明确失败：不 panic、不静默截断） |
| 确定性 | 同一插入序列 ⇒ 相同槽位布局 + **逐字节相同**的 `coverage()` |

## 4. 自检（怎么确认你真的用对了）

图集错了往往表现为「画面里字形串位/花屏」，很难目视定位，所以必须断言：

```rust
use deer_text::atlas::GlyphAtlas;
use deer_text::glyph::{GlyphImage, GlyphKey};

let mut atlas = GlyphAtlas::new(64);
let mut slots = Vec::new();
let mut keys = Vec::new();

// ① 插一批字形
for i in 0u16..8 {
    let img = GlyphImage::new(6, 6, 0, 6, 7.0, vec![(i as u8) * 30; 36]);
    let key = GlyphKey::new(i, 16);
    slots.push(atlas.insert(key, &img).expect("放得下"));
    keys.push((key, img.coverage.clone()));
}

// ② 取回来的字节必须与插入的完全一致
for (key, want) in &keys {
    let (slot, got) = atlas.get(*key).expect("插过就能取到");
    assert_eq!(got.len(), (slot.w * slot.h) as usize, "切片长度必须是 w*h");
    assert_eq!(got, want.as_slice(), "图集内容被写坏了");
}

// ③ 任意两槽位不重叠
for i in 0..slots.len() {
    for j in (i + 1)..slots.len() {
        assert!(!slots[i].overlaps(&slots[j]), "槽位 {i} 与 {j} 重叠");
    }
}

// ④ 槽位之外的像素必须保持 0，覆盖率长度必须对
assert_eq!(atlas.coverage().len(), (atlas.size().0 * atlas.size().1) as usize);
let mut inside = vec![false; atlas.coverage().len()];
for s in &slots {
    for y in s.y..s.bottom() {
        for x in s.x..s.right() {
            inside[(y * atlas.size().0 + x) as usize] = true;
        }
    }
}
for (i, &c) in atlas.coverage().iter().enumerate() {
    assert!(inside[i] || c == 0, "槽位外的像素 {i} 不是 0");
}

// ⑤ 幂等：重复插入不改变 len，也不改变已有槽位
let before = atlas.len();
let again = atlas.insert(keys[0].0, &GlyphImage::new(6, 6, 0, 6, 7.0, vec![9; 36]));
assert_eq!(again, Some(slots[0]), "重复插入必须返回同一个槽位");
assert_eq!(atlas.len(), before, "重复插入不该增加 len");

// ⑥ 确定性：同样的插入序列，两次的 coverage 逐字节相同
```

## 5. 常见坑

| 现象 | 原因 | 怎么改 |
|---|---|---|
| 字形边缘有**邻居的残影** | 忘了 padding：双线性采样会越过槽位边界取到隔壁字形 | `GlyphAtlas` 自带右/下 1px padding；**自己写图集时这一步不能省** |
| 图集 `insert` 之后先前拿到的槽位失效了 | 只有「紧凑重排」型算法才会这样 | 本项目选货架算法，**增高不搬动已有槽位**；有测试钉住这条，可放心缓存 `AtlasSlot` |
| 取到的字节数和 `w*h` 对不上 | 拿 `image.coverage` 的长度当图集切片长度用 | `get()` 返回的切片长度**恒为 `slot.w * slot.h`**；空位图就是空切片（长度 0） |
| 拿着 `get()` 的切片跨 `insert` 用，遇到借用报错或读到旧内容 | 切片借用图集自身，而**增高会 `Vec::resize`**（可能重新分配） | 槽位坐标（`AtlasSlot`）可以长期缓存 —— 增高不搬动它；**切片不要跨 `insert` 保存**，用完即取 |
| 「同一个字」被重复插了很多条 | 用了 `char` 或 `(char, size)` 当 key：同一字形被不同字符重复登记 | key 用 `GlyphKey{glyph_index, px_size}`；`insert` 本身也幂等 |
| 字号 16.0 与 16.4 想各存一份，结果只有一份 | `px_size` 是 **u16 取整**（按字号分桶），浮点字号会被折叠 | 这是刻意设计；真要按浮点字号分开，得自己换算成不同整数档 |
| 空格插不进去 / `get()` 返回 `None` | 空位图也必须 `insert` 才会登记 | 空格照样 `insert`，拿到 `{0,0,0,0}` 的槽位，`get()` 返回空切片 |
| 大批字符后 `insert` 返回 `None` | 超过了边界（字形比图集宽，或高度会超过 8192） | 加大 `new(width)`；或见第 6 节 —— 多图集/LRU 未实现，要自己分层管理 |

## 6. 相关

- 位图从哪来：[`glyph-raster.md`](glyph-raster.md)
- 把图集用起来（度量 + 光栅化 + 贴图）：[`text-rendering.md`](text-rendering.md)
- 图集转 PNG（调试）：`TextEngine::atlas_png()`，见 [`text-rendering.md`](text-rendering.md)
- **做不到**（本模块的边界）：
  - **没有 LRU / 淘汰**：插入只增不减，字形一旦登记就永久占位；图集满了就 `None`。
    长会话里字号连续变化会让图集只增不减 —— 需要自己按 `px_size` 分层管理。
  - **没有多图集 / 纹理数组**：只有一张二维覆盖率缓冲；超过 8192 高度或宽度的字形放不下。
  - **高度上限 8192**，宽度由 `new(width)` 固定且不可变。
  - **不做通用矩形装箱**：不支持旋转、不支持把已有槽位紧凑重排；
    「不搬动已有槽位」是**故意**的取舍（代价是利用率不如紧凑算法）。
  - 图集是 **8 位覆盖率**，不带颜色（颜色由贴图时给）。

## 7. 检查清单（发布前过一遍）

- [x] 示例能跑：`cargo run -p deer-gui --example glyph_atlas` → `exit=0`
- [x] 示例有自检断言（字节一致 / 不重叠 / 利用率 / 确定性）
- [x] `FEATURES.md` 已登记（状态 / 指南链接 / 示例命令都对）
- [x] 如果属于新手主线，`docs/TUTORIAL.md` 已更新
- [x] 明确写了「做不到什么」
