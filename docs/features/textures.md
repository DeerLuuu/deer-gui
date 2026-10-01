# 功能指南：通用纹理（textures）

> 状态 ✅（**仅离屏**；`RGBA8_UNORM` 创建 / 上传 / 回读 / 铺到矩形上采样）·
> 示例 `cargo run -p deer-gui --example textures` ·
> 清单条目见 [`FEATURES.md`](../../FEATURES.md)

## 1. 这是什么 / 什么时候用它

把**一张任意的 RGBA 图**交给 GPU：建纹理 → 上传像素 → （可选）**回读验证** → 铺到一个矩形上采样成像素。

```text
  create_texture_rgba8(w, h, &data)     →  创建 + 上传
  read_texture_bytes(&tex)              →  回读（验证真的传对了）
  draw_textured_quad(&tex, rect, tint)  →  采样成像素（与 CPU 参考逐字节对照）
```

**什么时候用它**：
- 你要在 GPU 上贴一张**非字形**的图（图标、缩略图、颜色表……）；
- 你在做纹理链的改动，需要一个「上传保真 + 采样与 CPU 一致」的回归抓手；
- 你要验证某个图源（解码器、生成器）产出的 RGBA 字节是否**原样**到达 GPU。

**什么时候不该用它**：
- 你要显示**文字** —— 那是**字形图集**（`R8_UNORM` 专用纹理），引擎自己管，不需要你建纹理
  （见 [`gpu-geometry.md`](gpu-geometry.md)）；
- 你要在**窗口里**贴图 —— **可以**（T1.3 ②）：`WindowedRenderer::draw_textured_quad`，见第 6 节；
- 你要按 **RGB 调制**（把图的红绿蓝当颜色用）—— **可以**（T1.3 ①）：走**纹理管线**
  （`fragment_shader_textured` ⇒ `out = 顶点色 × 采样 rgba`），四个通道都进像素。

## 2. 最小示例

```rust
use deer_gpu::{Color, Extent, RectI};
use deer_vk::GpuGeometryRenderer;

fn main() -> Result<(), String> {
    let extent = Extent { width: 32, height: 24 };
    let mut r = GpuGeometryRenderer::new(0, extent, Color::rgb(16, 16, 16))
        .map_err(|e| e.to_string())?;

    // ① 4×4 的 RGBA8 图（这里手搓；真实场景来自解码器/生成器）
    let (tw, th) = (4u32, 4u32);
    let mut data = Vec::new();
    for y in 0..th {
        for x in 0..tw {
            let on = (x + y) % 2 == 0;
            data.extend_from_slice(&[if on { 255 } else { 0 }, 7, 9, 255]); // R 棋盘，G/B/A 随意
        }
    }

    // ② 创建 + 上传（失败会明确报错，不静默给空纹理）
    let tex = r.device().create_texture_rgba8(tw, th, &data).map_err(|e| e.to_string())?;

    // ③ 回读：这是唯一能证明「G/B/A 也传对了」的判据（理由见第 4 节）
    let back = r.device().read_texture_bytes(&tex).map_err(|e| e.to_string())?;
    assert_eq!(back, data, "RGBA8 上传必须逐字节保真");

    // ④ 铺到矩形上采样成像素（像素中心采样 ⇒ NEAREST 无平局、可与 CPU 逐字节对照）
    let px = r
        .draw_textured_quad(&tex, RectI::new(4, 3, 24, 18), Color::rgb(200, 100, 50))
        .map_err(|e| e.to_string())?;
    assert_eq!(px.len(), (extent.width * extent.height * 4) as usize);
    Ok(())
}
```

跑完整示例（写 `render_out/textures.png`，放大 6 倍便于肉眼看朝向）：

```sh
cargo run -p deer-gui --example textures
```

本机实测输出（Windows / Intel RaptorLake-S 集显）：

```text
=== 通用纹理：创建 / 上传 / 回读 / 采样 ===

设备：Intel(R) RaptorLake-S Mobile Graphics Controller

① 纹理语料：4×4，R 通道棋盘 0/255、G/B/A 各不相同
② 创建 + 上传 ✅（4×4，RGBA8_UNORM）
   非法参数（0 宽 / 数据不足）都被拒绝 ✅
③ 回读 ✅ 64 字节、四通道逐字节相同
④ 采样 ✅ 与 CPU 参考最大通道差 0（棋盘 0/255 ⇒ 判据是逐字节 0 差）
⑤ UV 朝向 ✅ 纹理 (0,0) 落在 quad 左上角（V 未翻转、U 未镜像）
```

## 3. 完整 API

### `VkDevice`（用 `GpuGeometryRenderer::device()` 拿到）

| 方法 | 参数 | 说明 |
|---|---|---|
| `create_texture_rgba8(w, h, data)` | `data.len()` **必须** = `w*h*4`；`w`/`h` **不能为 0** | 建 `RGBA8_UNORM` 纹理并上传。参数非法**在碰驱动前**就报错 |
| `create_texture_r8(w, h, data)` | 同上，每像素 1 字节 | 字形图集用的格式；显式纹理一般用上面的 `rgba8` |
| `read_texture_bytes(&tex)` | — | 回读成 `Vec<u8>`，行优先、无 padding。**要 `TRANSFER_SRC`** —— `create_texture_rgba8` 已带上 |
| `texture.width()` / `.height()` / `.format()` | — | 纹理元信息（`format()` 返回 `TextureFormat::Rgba8Unorm`） |

### `GpuGeometryRenderer`

| 方法 | 说明 |
|---|---|
| `device()` | 拿到 `&VkDevice`（纹理必须在**同一设备**上建 —— `VkImage` 是设备级对象） |
| `draw_textured_quad(&tex, rect, tint)` | 把纹理铺到 `rect` 上并**回读整帧**。返回 `Vec<u8>`（整张画布的 RGBA8） |
| `unsupported()` | 翻译层报出的「未支持」清单（纹理 quad 正常时为空） |

**`tint` 的语义**：`out = vec4(tint.rgb, tint.a * texture(tex, uv).r)`。
即 **R 通道当覆盖率**、`tint.a` 是覆盖率乘子。`tint.a == 1` 且 `R ∈ {0,255}` 时结果只有
「纯 tint 色 / 纯透明」两态 ⇒ **不透明判据（逐字节 0 差）**成立；R 取中间值则是半透明 ⇒ **≤1 LSB**。

**`uv` 的语义**：像素**边界**，`u = (px - rect.x) / rect.w`。片元在像素中心 `px + 0.5` 求值
⇒ `rect.w == tex_w`（1:1）时采样点落在 texel 中心、`NEAREST` **无平局** ⇒ 逐像素精确。
非 1:1（如本示例 4×4 纹理铺到 24×18）也能对照，只是每个 texel 覆盖多个像素。

**朝向**：纹理**左上角**对到 quad **左上角**（画布 y 向下 + 着色器 `OriginUpperLeft`）
⇒ **不做 V 翻转**。翻转是贴图最常见的 bug，示例第 ⑤ 步专门钉住它。

## 4. 自检（怎么确认你真的用对了）

**两句都要**，缺一半判据就是空的：

```rust
// ① 上传保真：四通道逐字节（这是唯一能证明 G/B/A 传对了的判据）
let back = r.device().read_texture_bytes(&tex)?;
assert_eq!(back, data, "RGBA8 上传必须逐字节保真（含 alpha）");

// ② 采样正确：与 CPU 参考逐字节对照（R 通道 0/255 时判据是 0 差）
assert_eq!(max_channel_diff(&gpu_px, &cpu_reference), 0);
```

**为什么必须回读**（本项最容易踩的认知坑）：现有片元着色器是
`texture(tex, uv).r` —— **只读 R 通道**当覆盖率。所以哪怕 G/B/A 全传成 0，
**渲染出来的像素也是一样的**。「只看渲染结果」这个判据对通道错位**完全无感**；
回读是唯一直接证明。

**前置断言不能省**：语料必须有区分度（四通道各不止一种取值），
否则「四通道保真」在「四个通道全是同一个常数」时会平凡通过。
示例用 `BTreeSet` 逐通道断言这一点。

## 5. 常见坑

| 现象 | 原因 | 怎么改 |
|---|---|---|
| 「纹理贴上去上下颠倒了」 | 把 Vulkan 的 y 向下约定按 OpenGL 习惯又翻了一次 | **不要翻**。本项目的 `uv` 已是「纹理左上角 ↔ quad 左上角」 |
| 「G/B/A 传错了但画面看不出来」 | 片元着色器**只读 R 通道** | 用 `read_texture_bytes` 回读证明，别只看像素（见第 4 节） |
| 「纹理在别的设备上建的，画不出来」 | `VkImage` 是**设备级**对象 | 用 `GpuGeometryRenderer::device()` 上的 `create_texture_*` 建 |
| 「采样结果与 CPU 差 1」 | R 通道有中间值 ⇒ 有效 alpha < 1 ⇒ 半透明 | 那是预期行为：改判据为 **≤1 LSB**，或把语料改成 R ∈ {0,255} 走逐字节 |
| 「画完纹理后，形状/文本帧变花了」 | 描述符集还指着别人的纹理 | 不用管：`draw_textured_quad` 结束时**自动**改回默认纹理（图集或 1×1 哑纹理） |
| 「非 1:1 时边缘 texel 对不齐」 | `uv` 是连续映射，不是 texel 对齐 | 要逐字节对照就用 1:1；非 1:1 时按「每个 texel 覆盖的像素都取同一个 texel」推 CPU 参考 |

## 6. 相关

- **相关功能**：[`gpu-geometry.md`](gpu-geometry.md)（形状 + 字形图集采样；纹理 quad 与它共用同一条
  录制/提交/回读路径，所以间接绘制、屏障、计数、跨帧复用这些性质同样成立）、
  [`gpu-offscreen.md`](gpu-offscreen.md)（离屏设施）、[`pixels.md`](pixels.md)（像素判据口径）
- **内部原理**：`crates/deer-vk/src/device.rs`（`create_texture` / `read_texture_bytes`）、
  `crates/deer-vk/src/gpu_render.rs`（`draw_textured_quad` / `textured_quad_vertices`）
- **判据的测试版本**：`crates/deer-vk/tests/texture_indirect.rs`（四通道保真、逐字节对照、
  ≤1 LSB、uv 朝向四条；示例是它的使用者视角版本）

### 做不到什么

- **按 RGB 调制**：**已做**（T1.3 ①）—— `fragment_shader_textured` 读 `rgba` 并与顶点色**相乘**。
  注意它只在**纹理管线**上：界面管线（形状 + 文本）那一支**仍是覆盖率语义**
  （`R8` 字形图集 + `color.a * texel.r`），那是文本该有的语义，**刻意保留**，不是没做完。
- **窗口路径贴任意纹理**：**已做**（T1.3 ②）—— `WindowedRenderer::draw_textured_quad`，
  与离屏侧同形同语义（同一支顶点着色器、同一套顶点布局、另一条管线）。
  纹理趟是**单独一趟 present**（画完还原描述符），所以它会**清屏**：
  纹理**没法与界面同帧叠加**（要叠就得在同一趟里两次 draw + 中途换描述符，未做）。
- **纹理作为 `DrawCmd`**：`DrawList` 属于 `deer-gpu` 的契约，「一张任意纹理铺到矩形上」
  目前只有测试/诊断需要，所以**没有**进 `DrawCmd`。界面树里贴图 = 控件的活（M6 `Icon`）。
- **mipmap / 各向异性 / 重复寻址**：采样器是 `NEAREST` + `CLAMP_TO_EDGE`，没有 mip 链
  ⇒ 缩小采样会有锯齿，超出 `[0,1]` 的 uv 会被钳住。
- **纹理淘汰**：引擎只管字形图集的增长，不提供纹理 LRU；你自己建的纹理由你负责释放。
- **压缩格式 / 非 RGBA8**：只有 `RGBA8_UNORM` 与 `R8_UNORM` 两种格式。

## 7. 检查清单（发布前过一遍）

- [x] 示例能跑：`cargo run -p deer-gui --example textures` → `exit=0`
- [x] 示例有自检断言（回读逐字节 + 采样与 CPU 对照 + uv 朝向，且带前置断言）
- [x] `FEATURES.md` 已登记（状态 / 指南链接 / 示例命令都对）
- [x] `docs/TUTORIAL.md` 判断：**不属新手主线** —— 新手主线是「建树 → 出图」，
      通用纹理是**渲染层的低阶能力**，不放进教程（教程里不提纹理，避免主线被支线打断）
- [x] 明确写了「做不到什么」（第 6 节）
