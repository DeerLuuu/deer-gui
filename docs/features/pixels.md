# 功能指南：像素与 PNG（pixels）

> 状态 ✅ · 示例 `cargo run -p deer-gui --example pixels` ·
> 清单条目见 [`FEATURES.md`](../../FEATURES.md)

## 1. 这是什么 / 什么时候用它

**直接拿 RGBA8 像素缓冲**，以及**自己把它编码成 PNG**。

什么时候用它：
- 想把结果喂给别的库（缩放、拼接、比对）；
- 想分析像素（「这个位置到底是什么颜色」）；
- 想验证「某块区域真的被画上了」。

只想出一张图的话用 [`rendering.md`](rendering.md) 里的 `render_tree_to_png` 就够。

## 2. 最小示例

```rust
use deer_gui::prelude::*;

let (w, h, pixels) = deer_gui::render_tree_to_rgba(&tree, 120, 80, Theme::default())?;

// 像素布局：**行优先、无 padding**，每 4 字节一个像素，顺序 R G B A
// 第 (x, y) 个像素从 pixels[(y * w + x) * 4] 开始
let i = ((10 * w + 20) * 4) as usize;
println!("(20,10) = R{} G{} B{} A{}", pixels[i], pixels[i+1], pixels[i+2], pixels[i+3]);

// 编码成 PNG
let png = deer_gpu::png::encode_rgba(w, h, &pixels).map_err(|e| format!("编码失败：{e}"))?;
std::fs::write("out.png", png)?;
```

跑完整版：`cargo run -p deer-gui --example pixels`
（它还会写一个 `.ppm`，是无依赖的中间格式，多数图片工具能打开）

## 3. 完整 API

### 像素

| 项 | 说明 |
|---|---|
| `render_tree_to_rgba(...) -> (u32, u32, Vec<u8>)` | 宽、高、RGBA8 字节 |
| **布局** | 行优先、无 padding、每像素 4 字节、顺序 `R G B A` |
| **下标公式** | `(y * width + x) * 4`（`x`/`y` 从 0 开始） |
| 长度校验 | 必须等于 `width * height * 4` |

### PNG 编码器

| 函数 | 说明 |
|---|---|
| `deer_gpu::png::encode_rgba(w, h, &pixels) -> Result<Vec<u8>, String>` | 编成 PNG 字节流（可直接 `fs::write`） |

**为什么自己写编码器**：本项目**除窗口层 `winit`（已登记例外，见 [`ROADMAP.md`](../../ROADMAP.md) 的 Q-1）外**不引第三方依赖
（没有 `image` / `png` crate；`deer-gpu` 本身仍然零第三方依赖）。
实现用 zlib 的 **stored（未压缩）deflate** 块 —— 合法 zlib 流且绕开压缩算法，
CRC32 与 Adler32 自己实现（约 20 行各）。

**代价**：文件比真实压缩大（≈ 原始像素 + 少量开销）。例如 300×200 的图约 240 KB。
换来的是**这个编码器零依赖**。

## 4. 自检

```rust
let (w, h, px) = deer_gui::render_tree_to_rgba(&tree, 120, 80, Theme::default())?;

assert_eq!(px.len(), (w as usize) * (h as usize) * 4, "长度必须等于 宽×高×4");

// 画面不能只有一种颜色
let mut colors = std::collections::BTreeSet::new();
for p in px.chunks_exact(4) { colors.insert([p[0], p[1], p[2], p[3]]); }
assert!(colors.len() > 1, "只有一种颜色 ⇒ 什么都没画上");
```

编码器自身的正确性有已知向量测试（CRC32 与 Adler32 都对着标准值断言），
所以**你不必怀疑 PNG 是不是编码坏了** —— 如果图片打不开，先用 PPM 交叉验证
（示例里两种格式都写了）。

## 5. 常见坑

| 现象 | 原因 | 怎么改 |
|---|---|---|
| 下标算错、颜色取到了别的位置 | 忘了 `* 4`，或把 `x`/`y` 弄反 | 用 `(y * width + x) * 4` |
| PNG 很大 | stored 块不压缩（设计如此） | 正常；要小就接 deflate |
| `encode_rgba` 返回 `Err` | 像素长度与 `宽×高×4` 不符，或宽高为 0 | 检查长度；`w`/`h` 必须 > 0 |
| 图片打开是乱码 | 把 PNG 字节当文本写过（比如 `write_all` 到文本模式句柄） | 用 `std::fs::write`（二进制） |

## 6. 相关

- 渲染：[`rendering.md`](rendering.md)
- **做不到**：JPEG / WebP（只做 PNG）；抗锯齿；色彩空间转换（现在直接写原始字节）

## 7. 检查清单

- [x] 示例能跑：`cargo run -p deer-gui --example pixels` → `exit=0`
- [x] 示例有自检断言（长度 + 颜色数 + 同时写 PPM 交叉验证）
- [x] `FEATURES.md` 已登记
- [x] `docs/TUTORIAL.md` 已包含
- [x] 明确写了「做不到什么」
