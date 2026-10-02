# 功能指南：BMP 图像解码（image-decode）

> 状态 ✅（24/32 位、底行优先 → RGBA8 → 直喂纹理）·
> 示例 `cargo run -p deer-gui --example bmp_decode` ·
> 清单条目见 [`FEATURES.md`](../../FEATURES.md)

## 1. 这是什么 / 什么时候用它

把一份 **BMP 文件字节**解码成 **RGBA8 像素**，直接交给纹理链（`create_texture` + `upload_texture`）
或自有 PNG 编码器：

```text
  deer_gpu::image::decode_bmp(&bytes)        →  BmpImage { width, height, pixels }
  deer_gpu::image::upload_bmp_to_texture()   →  create_texture + upload_texture（一次到位）
  deer_gpu::png::encode_rgba(w, h, &pixels)  →  重编码成 PNG（回环判据：逐字节可复现）
```

**什么时候用它**：
- 你要在界面/纹理里贴一张**本地产出的图**（Windows 画图、截图工具都能产 BMP）；
- 你要给测试造一张**逐字节可断言**的图（BMP 无压缩，手工构造字节就是画面本身）；
- 你要验证「文件字节 → 纹理字节」这条链没有被中途改坏。

**什么时候不该用它**：
- 图源是 **PNG/JPEG** —— 解码不了（见第 6 节「做不到什么」）；`deer_gpu::png` 只是**编码**器；
- 你要做**色彩管理 / gamma 校正** —— 这里是字节直通，与全仓 `*_UNORM` 口径一致。

## 2. 最小示例

```rust
use deer_gpu::image::{decode_bmp, upload_bmp_to_texture};
use deer_gpu::{Backend, Device};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // ① 读文件（Windows 画图另存的 24 位 BMP 就是最典型的输入）
    let bytes = std::fs::read("render_out/logo.bmp")?;

    // ② 解码：失败会明确报错（不是 BMP / 被截断 / 登记不做的形态都会指名道姓）
    let img = decode_bmp(&bytes)?;
    println!("{}×{}", img.width, img.height);
    // img.pixels：RGBA8、行优先、无 padding、**顶行在前**
    // （pixels[0..4] 是左上角像素 —— 与 png::encode_rgba / upload_texture 的行序一致）

    // ③ 直喂纹理：CPU 参考后端无需 GPU；Vulkan 设备同理（device() 那个）
    let backend = deer_gpu::null::CpuBackend::new();
    let mut device = backend.open(0)?;
    let tex = upload_bmp_to_texture(&img, device.as_mut())?;
    let _ = tex;

    // ④ 想落盘看一眼：用自有 PNG 编码器（字节可复现）
    let png = deer_gpu::png::encode_rgba(img.width, img.height, &img.pixels)?;
    std::fs::write("render_out/decoded.png", png)?;
    Ok(())
}
```

跑完整示例（解码 3 种格式变体 + 回环判据 + 负例 + 可选的 Vulkan 回读保真）：

```sh
cargo run -p deer-gui --example bmp_decode
```

本机实测输出（Windows / NVIDIA GeForce RTX 5070 Ti Laptop GPU；**以你自己的输出为准**）：

```text
① 语料：20×16 一幅渐变+对角线画面 × 3 种格式（24 位 BI_RGB / 32 位 BI_RGB / 32 位 BI_BITFIELDS V4）
   前置断言 ✅（R≠B 有区分度、首末行互不相同）
② 24 位 BI_RGB（底行优先 + 行 padding） ✅ 解码 → 期望像素逐字节相同
② 32 位 BI_RGB（保留位按不透明，GDI 语义） ✅ 解码 → 期望像素逐字节相同
② 32 位 BI_BITFIELDS V4（掩码 + 文件 alpha） ✅ 解码 → 期望像素逐字节相同
③ 编码回环 ✅ 解码 → png.rs 重编码，逐字节可复现
⑥ 直喂 HAL ✅ upload_bmp_to_texture → create_texture + upload_texture（CPU 参考后端）
⑦ GPU 回读 ✅ 解码结果上 Vulkan 纹理，回读逐字节相同
```

## 3. 完整 API

| 条目 | 参数 | 说明 |
|---|---|---|
| `decode_bmp(data: &[u8]) -> Result<BmpImage, BmpError>` | 完整的 BMP 文件字节 | 主入口。24/32 位、底行优先；失败返回 `BmpError`，**不静默给空图** |
| `BmpImage { width, height, pixels }` | — | `pixels` 长度恒为 `w×h×4`：RGBA8、行优先、无 padding、**顶行在前** |
| `upload_bmp_to_texture(&img, device: &mut dyn Device) -> GpuResult<TextureId>` | 任意 HAL 设备 | `create_texture`（`Rgba8Unorm`、`readable: true`）+ `upload_texture` 整幅一次上传；`pixels` 长度不符先报错，不把坏数据交给后端 |
| `BmpError` | — | `BadMagic` / `Truncated { what, need, have }` / `Unsupported(String)` / `Invalid(String)`；`Display` 的信息都指名道姓 |

**支持的格式**（每条都有测试钉住，见 `crates/deer-gpu/tests/image_bmp.rs`）：

| 变体 | 行为 |
|---|---|
| 24 位 `BI_RGB` | 每像素 B、G、R；行按 4 字节对齐的 padding 被跳过 |
| 32 位 `BI_RGB` | 每像素 B、G、R、**X** —— X 按保留位处理，解码**强制不透明**（GDI 语义） |
| 32 位 `BI_BITFIELDS` + V1 头（40） | 读**紧跟头的 3 个掩码 DWORD**（R/G/B）；无 alpha ⇒ 不透明 |
| 32 位 `BI_BITFIELDS` + V2(52)/V3(56)/V4(108)/V5(124) 头 | 读**头内嵌掩码**；V3 起有 alpha，逐像素生效 |
| 位域掩码宽度 ≠ 8 | 位宽 >8 取最高 8 位；<8 做 `round(v×255/max)`（标准 BMP 都是 8 位，这条是顺手正确） |

## 4. 自检（怎么确认你真的用对了）

两句判据都要，缺一半就是空的：

```rust
// ① 解码结果 == 独立推导的期望 RGBA（逐字节）
assert_eq!(img.pixels, expected, "解码结果必须与期望 RGBA 逐字节相同");

// ② 编码回环：解码结果重编码的 PNG == 期望像素重编码的 PNG（逐字节、可复现）
assert_eq!(
    deer_gpu::png::encode_rgba(img.width, img.height, &img.pixels)?,
    deer_gpu::png::encode_rgba(img.width, img.height, &expected)?,
    "回环判据：两份 PNG 必须逐字节相同"
);
```

**前置断言不能省**：语料必须让两类变异可观测 ——
至少一个像素 **R≠B**（否则「通道序 BGR→RGB 改错」抓不到）、**首末行颜色不同**
（否则「底行优先漏翻转」抓不到）。两类变异都实际演练过：改坏 → 红 → 还原 → 绿。

## 5. 常见坑

| 现象 | 原因 | 怎么改 |
|---|---|---|
| 贴出来的图**上下颠倒** | 把 BMP 的「底行优先」又自己翻了一次 | 不要再翻：`decode_bmp` 的输出已经是**顶行在前**，直接喂纹理/PNG |
| 图的颜色**红蓝互换** | 手写 BMP 时按 R、G、B 写了像素字节 | BMP 存储序是 **B、G、R**（,X）；手工构造时按 `BGR` 写 |
| 32 位 BMP 解出来**整张透明** | 文件是 `BI_RGB`，第 4 字节是保留位（常为 0），被当成 alpha 用了 | `decode_bmp` 已按 GDI 语义**强制不透明**；只有 `BI_BITFIELDS` 且掩码里有 alpha 才逐像素生效 |
| 宽不是 4 的倍数时画面**斜着花** | 把行对齐 padding 当像素读了 | padding 由解码器跳过；手工构造文件时每行补齐到 4 字节倍数 |
| 解码报 `Unsupported` 却觉得「应该支持」 | 落在登记「不做」的形态（16 位 / 调色板 / RLE / top-down / OS/2 core 头） | 看错误信息里点名的形态；确有需求先在 `ROADMAP.md` 登记再立项，不要悄悄扩 |

## 6. 相关

- **相关功能**：[`textures.md`](textures.md)（解码结果喂上去的那条纹理链：创建 / 上传 / 回读 / 采样）、
  [`pixels.md`](pixels.md)（像素判据口径）、[`gpu-geometry.md`](gpu-geometry.md)（GPU 侧采样）
- **内部原理**：`crates/deer-gpu/src/image.rs`（解码器本体，头结构注释在模块文档）、
  `crates/deer-gpu/tests/image_bmp.rs`（回环判据 + 变体矩阵 + 负例）、
  `crates/deer-text/src/png.rs`（回环用的零依赖 PNG **编码**器）

### 做不到什么

- **PNG 解码未做**：`deer_gpu::png` 只是**编码**器；解码 PNG 需要自写 inflate
  （数百行），已按「先只做 BMP，PNG 解码单独立项按需决策」登记
  （`ROADMAP.md` Q2）。**不要**拿它解 PNG。
- **16 位 BMP 未做**（需要 5-5-5 / 5-6-5 位域展开）：给 `Unsupported`，不静默。
- **调色板 BMP 未做**（bpp ≤ 8）、**RLE 压缩未做**（`BI_RLE8`/`BI_RLE4`）、
  **内嵌 JPEG/PNG 压缩未做**：同上，全部显式报错。
- **top-down 行序未做**（height 为负的 BMP）：只支持底行优先（height 为正）。
- **OS/2 `BITMAPCOREHEADER` 未做**（DIB 头 < 40 字节的族）。
- **`BI_ALPHABITFIELDS`（压缩 6）未做**。
- **不做色彩管理 / gamma**：字节直通（与全仓颜色附件 `*_UNORM` 的口径一致）。
- **不做增量/流式解码**：输入必须是一份完整的文件字节。

## 7. 检查清单（发布前过一遍）

- [x] 示例能跑：`cargo run -p deer-gui --example bmp_decode` → `exit=0`
- [x] 示例有自检断言（前置断言 + 期望逐字节对照 + 回环可复现 + 负例显式报错 + GPU 回读保真）
- [x] `FEATURES.md` 已登记（状态 / 指南链接 / 示例命令都对）
- [x] `docs/TUTORIAL.md` 已更新（第 8 章「拿原始像素」加了「从 BMP 文件拿像素」小节）
- [x] 明确写了「做不到什么」（第 6 节：PNG 解码未做、16 位 BMP 未做等）
