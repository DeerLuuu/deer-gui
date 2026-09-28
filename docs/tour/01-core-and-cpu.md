# 事实地图 01：`deer-layout` + `deer-gpu`（CPU 链路）

> **性质**：只读测绘产出的事实地图，供「代码库导览」第 1~4 讲取材。
> **纪律**：每条结论都带 `path:line` 锚点；签名/常量值/行号照抄真实代码；读不到的写「未确认」。
> **本次未执行**：没有修改任何现有文件、没有提交、没有跑 `cargo fmt`、**没有运行任何 cargo 命令**（因此下文不含「测试是否当前全绿」的判断）。
> **行数口径**：按换行符计数（`\n` 个数），与 read 工具报的 total 一致。

---

## 1. 一句话职责

| crate | 一句话职责 | 锚点 |
|---|---|---|
| `deer-layout` | **语言无关的纯数据核心**：节点树（唯一真相）+ 布局代数（测量/排布/命中测试）+ `.dui` 场景文件解析，完全不知道 GPU／窗口／事件循环的存在。 | `crates/deer-layout/src/lib.rs:1-21` |
| `deer-gpu` | **GPU 硬件抽象层（HAL）+ CPU 参考后端**：定义后端必须实现的契约与平台无关的绘制数据（`DrawList`），并用纯 CPU 软件光栅化把 `DrawList` 变成像素，充当 GPU 侧的黄金基准；自身**不依赖任何图形库**。 | `crates/deer-gpu/src/lib.rs:1-20`、`crates/deer-gpu/src/null.rs:1-8` |

依赖方向（`crates/deer-gpu/Cargo.toml`）：`deer-gpu` 只依赖 `deer-layout`；`crates/deer-layout/Cargo.toml` 的 `[dependencies]` 为空。两者都没有第三方依赖，也没有 `[dev-dependencies]`。

---

## 2. 文件清单表

### 2.1 `crates/deer-layout/src/*.rs`

| 文件 | 行数 | 职责 | 关键类型/函数（带行号） |
|---|---|---|---|
| `crates/deer-layout/src/lib.rs` | 37 | crate 门面：模块声明 + re-export + 版本常量；用模块文档写清「两条构筑路径、一棵树」。 | `pub mod builder/layout/node/scene`（`:26-29`）；re-exports（`:31-34`）；`pub const CORE_VERSION: &str = "0.0.0"`（`:37`） |
| `crates/deer-layout/src/node.rs` | 249 | 节点树数据模型（含确定性 id 生成规则）。 | `enum Kind`（`:21-32`）+ `as_str`（`:35`）/`parse`（`:45`）/`is_container`（`:57`）；`enum Align`（`:64-69`）+`parse`（`:72`）；`enum Size { Px, Pct }`（`:85-88`）；`struct Rect`（`:91-96`）+`new`（`:99`）；`struct LayoutProps`（`:106-115`）；`struct NodeProps`（`:119-122`）；`struct Node`（`:125-131`）；`Node::{new:134, with_layout:145, with_props:150, with_id:156, with_label:161, disabled:166, push:171, is_container:176, walk:181, structurally_eq:189}`；`struct IdGen`（`:209-212`）+`reserve`（`:222`）+`next`（`:238`） |
| `crates/deer-layout/src/layout.rs` | 367 | 布局引擎：文本度量接口 + 固有尺寸测量 + 排布（两趟）+ 命中测试；列出 I-1..I-7 不变量（`:3-12`）。 | `struct TextStyle`（`:20-23`，Default = 13.0/18.0 `:25-32`）；`trait Measure`（`:36-39`）；`struct ApproxMeasure`（`:46`）+ impl（`:48-62`）；`mod metrics`（`:65-71`）；`fn resolve`（`:73-78`）；`type Intrinsics`（`:81`）；`fn measure_tree`（`:83`）；`fn measure_into`（`:89`）；`type Geometry`（`:175`）；`fn layout`（`:178`）；`struct PlaceCtx`（`:203-206`）+ `place`（`:209`）；`fn hit_test`（`:353`） |
| `crates/deer-layout/src/builder.rs` | 254 | 命令式（imgui 手感）构筑 + 保留式节点树。 | `struct Builder`（`:21-26`）；`new`（`:29`）、`auto`（`:42`）、`padding`（`:52`）、`gap`（`:57`）、`size`（`:62`）、`node_at_mut`（`:68`）、`push`（`:76`）、`text`（`:86`）、`button`（`:91`）、`button_opts`（`:96`）、`field`（`:102`）、`push_named`（`:110`）、`container_auto`（`:122`）、`container`（`:128`）、`container_opts`（`:145`）、`container_with`（`:158`）、`build`（`:182`）、`root_id`（`:186`）；`struct L`（`:193-201`）+ 构造器（`:204-235`）；`fn props`（`:249`） |
| `crates/deer-layout/src/scene.rs` | 407 | `.dui`（`.tscn` 式缩进格式）解析与编码；纯解析不执行任何东西（`:17`）。 | `struct SceneError`（`:24-28`）+ `Display`（`:30-34`）+ `Error`（`:36`）；`fn preprocess`（`:53`）；`enum AttrVal`（`:89-92`）；`fn parse_attrs`（`:95`）；`const KNOWN_ATTRS`（`:143-145`）；`as_num`（`:147`）/`as_size`（`:156`）/`as_align`（`:175`）；`struct Open`（`:188-192`）；`fn node_at`（`:194`）；`fn parse_scene`（`:203`）；`fn encode_scene`（`:335`）；`fmt_num`（`:382`）/`align_str`（`:390`）/`quote`（`:399`） |
| `crates/deer-layout/tests/layout_invariants.rs` | 354 | 核心不变式测试套件（T1..T14，含 V0 三个缺陷 B-1/B-2/B-3 的回归守卫，`:1-6`）。 | `geo`（`:18`）、`imperative`（`:23`）、`SCENE`（`:34-41`）；T1 同构（`:48`、`:58`）、T2 确定性/稳定编码（`:83`、`:94`）、T3 纯函数（`:100`）、T4 固有尺寸（`:112`、`:123`）、T5 grow（`:143`）、T6 主轴对齐（`:171`）、T7 交叉轴 stretch（`:195`）、T8 像素取整（`:214`）、T9 命中测试（`:229`、`:239`）、T10 场景往返（`:258`）、T11 错误带行号（`:271`）、T12 百分比（`:297`）、T13（`:319`）、T14（`:330`） |

### 2.2 `crates/deer-gpu/src/*.rs`

| 文件 | 行数 | 职责 | 关键类型/函数（带行号） |
|---|---|---|---|
| `crates/deer-gpu/src/lib.rs` | 223 | HAL 契约（`Backend`/`Device`/`Swapchain`/`Frame`/`Renderer`）+ 句柄与格式类型 + `Theme`。 | `trait Backend`（`:56-63`）；`type AdapterIndex = usize`（`:65`）；`struct AdapterInfo`（`:68-74`）；`enum AdapterKind`（`:77-83`）；`enum TargetFormat`（`:87-93`）；`struct Extent`（`:96-99`）；`trait Device`（`:102-128`）；`struct RawWindowHandle`（`:132-137`）；`enum Platform`（`:140-145`）；`struct TextureDesc`（`:148-154`）；`struct TextureRegion`（`:157-162`）；`trait Swapchain`（`:165-170`）；`trait Frame`（`:173-180`）；`enum PresentResult`（`:183-187`）；`trait Renderer`（`:193-195`）；`struct Theme`（`:199-208`）+ `Default`（`:210-222`） |
| `crates/deer-gpu/src/draw.rs` | 161 | 平台无关绘制数据：颜色 / 整数矩形 / 绘制命令 / 绘制列表 + 裁剪平衡不变式。 | `struct Color`（`:7-12`）+ `rgb`（`:15`）/`rgba`（`:18`）/`TRANSPARENT`（`:21`）/`WHITE`（`:22`）/`packed`（`:25-27`）；`struct RectI`（`:32-37`）+ `new`（`:40`）/`right`（`:43`）/`bottom`（`:46`）/`contains`（`:49-51`）；`struct TextureId(u32)`（`:56`）；`enum DrawCmd`（`:64-85`）；`struct DrawList`（`:89-93`）+ `new`（`:96`）/`from_cmds`（`:104`）/`push`（`:112`）/`len`（`:121`）/`is_empty`（`:125`）/`clip_balanced`（`:130`）/`counts`（`:135`）；`struct DrawCounts`（`:153-161`） |
| `crates/deer-gpu/src/null.rs` | 554 | **CPU 参考后端**：帧缓冲、CPU HAL 实现、软件光栅化（矩形/圆角/描边/文字/裁剪栈）。 | `const CPU_ADAPTER_NAME`（`:19`）；`struct Framebuffer`（`:23-28`）+ `new`（`:31`）/`clear`（`:41`）/`pixel`（`:52`）/`count_color`（`:66`）/`bytes_eq`（`:75`）/`to_rgba`（`:79`）；`struct CpuBackend`（`:86`）+ `new`（`:89`）+ `impl Backend`（`:94-116`）；`struct CpuDevice`（`:119-122`）+ `render_to_framebuffer`（`:129`）+ `impl Device`（`:139-182`）；`struct CpuSwapchain`（`:184`）+ `impl Swapchain`（`:188-199`）；`struct CpuFrame`（`:202-207`）+ `set_extent`（`:211`）/`framebuffer`（`:216`）/`render_into`（`:221`）+ `impl Frame`（`:226-257`）；`struct CpuRenderer`（`:268-270`）+ `new`（`:274`）/`with_text`（`:279`）/`text`（`:284`）/`render`（`:292`）；`fn soft_rasterize_with`（`:308`）；`fn soft_rasterize`（`:359`）；`fn blend`（`:363`）；`fn blend_cov`（`:370`）；`fn fill`（`:387`）；`fn inside_rounded`（`:400`）；`fn stroke`（`:421`）；`struct TextDraw`（`:435-441`）；`fn draw_text`（`:448`）；`fn draw_text_real`（`:493`） |
| `crates/deer-gpu/src/render.rs` | 153 | 把「树 + 几何 + 主题」翻成 `DrawList`（新增控件类型唯一要改的地方，`:3-4`）+ 一个只发 `NodeHint` 的诊断渲染器。 | `struct DefaultRenderer<'a, M: Measure>`（`:17-20`）+ `new`（`:23`）/`build`（`:28`）/`emit`（`:34`）/`text_style`（`:117`）；`struct NullRenderer`（`:126`）+ `build`（`:129`）；`fn build_draw_list`（`:148`）；`pub const TRANSPARENT: Color`（`:153`） |
| `crates/deer-gpu/src/atlas.rs` | 282 | 字形图集：**货架（shelf）打包** + 按需增高 + 双份数据（大图给 GPU / 逐行紧致副本给 `get()`）。 | `const PADDING: u32 = 1`（`:50`）；`pub const MAX_DIMENSION: u32 = 8192`（`:55`）；`struct Entry`（`:58-63`）；`struct GlyphAtlas`（`:66-83`）+ `new`（`:90`）/`slot_of`（`:104`）/`insert`（`:120`）/`get`（`:227`）/`contains`（`:233`）/`size`（`:238`）/`coverage`（`:243`）/`len`（`:248`）/`is_empty`（`:253`）/`used_pixels`（`:258`）/`utilization`（`:263`）/`ensure_height`（`:276`） |
| `crates/deer-gpu/src/glyph.rs` | 207 | 字形流水线的共享数据契约：覆盖率位图 / 图集键 / 图集槽位；模块文档给出坐标约定（`:7-19`）。 | `struct GlyphImage`（`:25-43`）+ `blank`（`:47`）/`new`（`:59`）/`is_blank`（`:83`）/`coverage_at`（`:88`）/`coverage_sum`（`:96`）/`max_coverage`（`:101`）/`ink_pixels`（`:106`）/`mean_coverage`（`:114`）；`struct GlyphKey`（`:127-132`）+ `new`（`:135`）；`struct AtlasSlot`（`:148-153`）+ `right`（`:156`）/`bottom`（`:159`）/`overlaps`（`:163`）；内联单测 `mod tests`（`:171-206`） |
| `crates/deer-gpu/src/raster.rs` | 332 | 字形光栅化：轮廓 → 折线（自适应展平）→ **nonzero winding** 填充 → 超采样覆盖率。 | `const FLATNESS_TOLERANCE: f32 = 0.25`（`:52`）；`MAX_FLATTEN_DEPTH: u32 = 16`（`:58`）；`MAX_SUPERSAMPLE: u32 = 64`（`:64`）；`MAX_BITMAP_DIM: u32 = 4096`（`:70`）；`DEGENERATE_LEN2: f32 = 1e-12`（`:73`）；`struct Rasterizer`（`:79-84`）+ `new`（`:88`）/`with_supersample`（`:96`）/`rasterize`（`:109`）/`rasterize_char`（`:223`）；`to_bitmap`（`:236`）/`dist2`（`:241`）/`mid`（`:248`）/`push_distinct`（`:253`）/`point_line_dist`（`:265`）/`flatten_quad`（`:276`）/`flatten_cubic`（`:291`）/`winding_number`（`:315`）/`is_left`（`:330`） |
| `crates/deer-gpu/src/measure.rs` | 217 | **真实字体度量 + 贪心换行**：`unitsPerEm`/`hmtx`/`hhea` → 像素；实现 `deer_layout::Measure`。 | `struct FontMeasure<'a>`（`:48-53`）+ `new`（`:57`）/`scale`（`:65`）/`ascent`（`:74`）/`descent`（`:83`）/`line_height`（`:88`）/`advance`（`:95`）/`text_width`（`:114`）/`sum_advances`（`:119`）/`wrap`（`:124`）+ `impl Measure`（`:182-196`）；`fn find_system_font`（`:205`） |
| `crates/deer-gpu/src/text.rs` | 241 | 文字引擎：字体 + 图集 + 每字形排版信息；三条链（font/raster/atlas）的汇合点（`:1-31`）。 | `const ATLAS_WIDTH: u32 = 512`（`:70`）；`struct GlyphPlacement`（`:46-55`）；`struct TextEngine`（`:58-65`）+ `from_font`（`:74`）/`from_font_bytes`（`:85`）/`from_font_file`（`:90`）/`from_system_font`（`:101`）/`font`（`:112`）/`font_size`（`:116`）/`set_font_size`（`:121`）/`measure`（`:126`）/`glyph`（`:138`）/`text_width`（`:186`）/`rasterized_glyphs`（`:197`）/`missing_glyphs`（`:203`）/`atlas`（`:207`）/`atlas_png`（`:212`）；手写 `Debug`（`:230-240`） |
| `crates/deer-gpu/src/font.rs` | 885 | **零依赖 TrueType 解析**：sfnt/ttcf 目录、`head`/`maxp`/`hhea`/`hmtx`/`cmap`(0/4/6/12)/`loca`/`glyf`（简单 + 复合）。 | `struct Point`（`:39-43`）；`enum Segment`（`:47-58`）；`struct Contour`（`:62-65`）；`struct Glyph`（`:69-78`）+ `is_blank`（`:82`）/`outline_bbox`（`:90`）；`enum CmapFormat`（`:112-117`）；`struct Font`（`:120-137`）；`tag/u8_at/u16_at/i16_at/u32_at`（`:140-169`）；`Font::parse`（`:176`）/`table_range`（`:352`）/`table_directory`（`:360`）/`face_offset`（`:365`）/`table`（`:377`）/`glyph_index`（`:399`）/`cmap0`（`:431`）/`cmap4`（`:438`）/`cmap6`（`:474`）/`cmap12`（`:483`）/`glyph`（`:506`）/`loca_range`（`:558`）/`h_metrics`（`:576`）/`parse_simple_glyph`（`:600`）/`parse_composite_glyph`（`:697`）/`line_height_units`（`:786`）；`f2dot14`（`:792`）/`transform_contour`（`:797`）/`flatten_contour`（`:832`） |
| `crates/deer-gpu/src/png.rs` | 170 | 零依赖 PNG 编码器（RGBA8）：zlib **stored** deflate + 手写 CRC32/Adler32。 | `fn encode_rgba`（`:14`）/`write_chunk`（`:57`）/`zlib_store`（`:68`）/`crc32`（`:92`）/`adler32`（`:105`）；内联单测（`:115-169`） |
| `crates/deer-gpu/src/error.rs` | 36 | HAL 错误类型（初始化失败不得 panic，`:3-4`）。 | `enum GpuError`（`:9-20`）；`Display`（`:22-32`）；`Error`（`:34`）；`type GpuResult<T>`（`:36`） |

`crates/deer-gpu/src/paint.rs` **不存在**（导览计划里写的「若存在」可以排除）。

---

## 3. 数据流：从「一个 UI 树」到「`DrawList`」到「CPU 像素缓冲」

全部为函数级链路，箭头右侧为返回类型。

### 3.1 构筑 UI 树（两条路径，产出同一结构）

| 步 | 函数 | 输入 → 输出 | 锚点 |
|---|---|---|---|
| A1 | `Builder::new(kind, id)` / `Builder::auto(kind)` | `(Kind, impl Into<String>)` → `Builder`（内部 `Node` + `IdGen`） | `crates/deer-layout/src/builder.rs:29`、`:42` |
| A2 | `Builder::{text,button,field,container*}` | `impl Into<String>` / 闭包 → `String`（叶子 id）/ `()` | `builder.rs:86`、`:91`、`:102`、`:122`、`:128` |
| A3 | `Builder::build()` | `&self` → `Node`（`self.root.clone()`） | `builder.rs:182` |
| B1 | `parse_scene(src, source)` | `(&str, &str)` → `Result<Node, SceneError>` | `crates/deer-layout/src/scene.rs:203` |
| B2 | `encode_scene(root)` | `&Node` → `String`（`parse(encode(t))` 必须结构相等） | `scene.rs:335` |
| — | 两路相等性判据 `Node::structurally_eq` | `(&Node, &Node)` → `bool` | `crates/deer-layout/src/node.rs:189` |

### 3.2 树 → 几何表（布局，纯函数）

| 步 | 函数 | 输入 → 输出 | 锚点 |
|---|---|---|---|
| L1 | `layout(root, box_, style, m)` | `(&Node, Rect, TextStyle, &impl Measure)` → `Geometry`（`HashMap<String, Rect>`） | `crates/deer-layout/src/layout.rs:178` |
| L2 | `layout` 内部第一趟 `measure_tree(root, style, m)` | `(&Node, TextStyle, &impl Measure)` → `Intrinsics`（`HashMap<String,(f32,f32)>`） | `layout.rs:179`、定义 `:83` |
| L3 | `measure_into(n, style, m, out)` 递归（自底向上） | `&Node` → `(f32, f32)` 固有尺寸（`ceil()` 后写入 `out`） | `layout.rs:89`、`ceil` 在 `:169` |
| L4 | `PlaceCtx::place(n, rect, assigned, avail_w, avail_h)` 递归（自顶向下） | 分配盒 + 可用盒 → 写入 `Geometry` | `layout.rs:209`、插入 `:226-229` |
| L5 | 子节点主轴/交叉轴解算 + `grow` 分配 + 对齐 | 内部量 → `child_rect`（`Rect`） | `layout.rs:244-346`（`growth` `:264-277`；对齐 `:279-300`；交叉轴 `:302-333`；递归 `:344`） |
| L6 | `hit_test(root, geo, px, py)` | `(&Node, &Geometry, f32, f32)` → `Option<&Node>`（最深命中者） | `layout.rs:353`、后序覆盖在 `:359` |

### 3.3 树 + 几何 → `DrawList`

| 步 | 函数 | 输入 → 输出 | 锚点 |
|---|---|---|---|
| D1 | `build_draw_list(tree, geo, theme, measure)` | `(&Node, &Geometry, Theme, &M)` → `DrawList` | `crates/deer-gpu/src/render.rs:148` |
| D2 | `DefaultRenderer::build(&self, tree, geo)` | `(&Node, &Geometry)` → `DrawList` | `render.rs:28` |
| D3 | `DefaultRenderer::emit(n, geo, list)` 前序递归 | 每节点 → `list.push(DrawCmd::…)` | `render.rs:34`；`Column/Row` `:43-57`、`Text` `:58-67`、`Button` `:68-90`、`Field` `:91-109`、递归子节点 `:112-114` |
| D4 | `DrawList::push(cmd)` | `DrawCmd` → `()`，同时维护 `clip_balance` | `crates/deer-gpu/src/draw.rs:112-119` |
| D5 | `DrawList::clip_balanced()` | `&self` → `bool`（结构不变式） | `draw.rs:130` |
| D6 | `NullRenderer::build(tree, geo)`（诊断替代路径） | `(&Node, &Geometry)` → `DrawList`（每几何项一个 `NodeHint`） | `render.rs:129`、push 在 `:133` |

### 3.4 `DrawList` → CPU 像素缓冲

| 步 | 函数 | 输入 → 输出 | 锚点 |
|---|---|---|---|
| C1 | `CpuRenderer::render(&mut self, extent, list, clear)` | `(Extent, &DrawList, Color)` → `GpuResult<Framebuffer>` | `crates/deer-gpu/src/null.rs:292` |
| C2 | 裁剪平衡检查（不通过 → `GpuError::Driver{code:-1}`） | `&DrawList` → `GpuResult<()>` | `null.rs:293-298` |
| C3 | `Framebuffer::new(w, h, clear)` | `(u32, u32, Color)` → `Framebuffer`（`extent.max(1)`） | `null.rs:31`、调用处 `:299` |
| C4 | `soft_rasterize_with(fb, list, text)` | `(&mut Framebuffer, &DrawList, Option<&mut TextEngine>)` → `GpuResult<()>` | `null.rs:308`、调用处 `:300` |
| C5 | 逐命令分派 + 裁剪栈（本地 `clip`/`stack`） | `&DrawCmd` → 变换 `clip` 或调用 `fill`/`stroke`/`draw_text*` | `null.rs:317-354`（PushClip `:319-326`、PopClip `:327-329`、Fill `:330`、FillRound `:331`、Stroke `:332`、Text `:333-351`、NodeHint `:352`） |
| C6 | `fill(fb, rect, color, clip, radius)` | 整数矩形 → 逐像素 `blend` | `null.rs:387` |
| C7 | `stroke(fb, rect, color, width, clip)` | 整数矩形 + 线宽 → 4×`width` 条 1px 带，各自 `fill(..., radius=0)` | `null.rs:421-429` |
| C8 | `draw_text(fb, cmd, clip)`（**无字库**路径，等宽占位格） | `&TextDraw` → 每字符一个填充矩形 | `null.rs:448` |
| C9 | `draw_text_real(engine, fb, cmd, clip)`（**有字库**路径） | `&mut TextEngine` → 逐字形采样图集覆盖率 → `blend_cov` | `null.rs:493` |
| C10 | `blend(fb, x, y, c, clip)` → `blend_cov(..., cov = 1.0, ...)` | 整数像素坐标 → 就地改写 4 字节 | `null.rs:363-365`、`:370` |
| C11 | `Framebuffer::pixels` / `to_rgba()` | `&self` → `&[u8]`（RGBA8 行优先无 padding） | `null.rs:27`、`:79` |
| C12 | `encode_rgba(w, h, pixels)`（可选，落盘） | `(u32,u32,&[u8])` → `Result<Vec<u8>, String>` | `crates/deer-gpu/src/png.rs:14` |

### 3.5 门面 crate 的实际调用链（宿主看一眼就懂）

`crates/deer-gui/src/lib.rs:77-100` `render_tree_to_rgba`：`layout(...)`（`:84`）→ `gpu::build_draw_list(...)`（`:93`）→ `CpuRenderer::new().render(...)`（`:94`）。
`crates/deer-gui/src/lib.rs:139-166` `render_tree_to_rgba_with_engine`：额外把 `theme.font_size` 钉成 `font_size`（`:149`）并用 `engine.measure()` 同时喂布局（`:159`）与渲染器（`:161`），最后 `CpuRenderer::with_text(engine)`（`:163`）。

### 3.6 字形的子链路（`deer-gpu` 内部，CPU 出真字用）

`char` → `Font::glyph_index`（`font.rs:399`）→ `Font::glyph`（`font.rs:506`）→ `Rasterizer::rasterize`（`raster.rs:109`）→ `GlyphImage`（`glyph.rs:25`）→ `GlyphAtlas::insert`（`atlas.rs:120`）→ `AtlasSlot`（`glyph.rs:148`）→ `GlyphPlacement`（`text.rs:46`，由 `TextEngine::glyph` 装配 `text.rs:172-179`）→ `draw_text_real` 采样 `atlas.coverage()`（`null.rs:523-525`、`535-548`）。

---

## 4. 公开 API 清单（签名照抄 + 谁在用它）

> 「谁在用它」来自全仓库 `grep`；标注「无调用点」表示除定义/文档/测试外没有消费者。

### 4.1 `deer-layout`

#### `node.rs`

| 签名 | 行 | 谁在用它 |
|---|---|---|
| `pub enum Kind { Column, Row, Text, Button, Field }` | `:21-32` | `builder.rs:87` 等；`scene.rs:227`（`Kind::parse`）；`render.rs:42` 的 match；`deer-gui/examples/*` |
| `pub const fn as_str(self) -> &'static str` | `:35` | `scene.rs:371`（编码）、`node.rs:227`、`:242` |
| `pub fn parse(s: &str) -> Option<Kind>` | `:45` | `scene.rs:227`（唯一调用点） |
| `pub const fn is_container(self) -> bool` | `:57` | `builder.rs:129`、`:152`（断言）；`scene.rs:309`；`layout.rs:231` |
| `pub enum Align { Start, Center, End, Stretch }` | `:64-69` | `builder.rs:223-229`（`L::main/cross`）、`scene.rs:178`/`:265`/`:268`、`layout.rs:286`/`:302` |
| `pub fn parse(s: &str) -> Option<Align>` | `:72` | `scene.rs:178` |
| `pub enum Size { Px(f32), Pct(f32) }` | `:85-88` | `builder.rs:208`/`:212`；`scene.rs:164`/`:169`；`layout.rs:110`/`:112`/`:162`/`:165`/`:250`；`layout_invariants.rs:298`/`:305` |
| `pub struct Rect { pub x: f32, pub y: f32, pub w: f32, pub h: f32 }` | `:91-96` | `layout.rs:175`（`Geometry` 值类型）、`render.rs:36`（`RectI::new(f.x as i32, …)`）、`deer-gui` 门面 `lib.rs:86` |
| `pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Rect` | `:99` | `layout.rs:228`/`:330`/`:332`；`layout_invariants.rs:19`；`deer-gui/src/lib.rs:86`/`:157`/`:186` |
| `pub struct LayoutProps { pub width: Option<Size>, pub height: Option<Size>, pub padding: f32, pub gap: f32, pub main_axis: Option<Align>, pub cross_axis: Option<Align>, pub grow: f32 }` | `:106-115` | `builder.rs:124`/`:149`/`:168`/`:236-244`；`scene.rs:251-272`；`render.rs:45` |
| `pub struct NodeProps { pub label: Option<String>, pub disabled: bool }` | `:119-122` | `builder.rs:87`/`:92`（`with_label`）、`scene.rs:274-278`、`render.rs:40`/`:59`/`:62` |
| `pub struct Node { pub kind: Kind, pub id: String, pub layout: LayoutProps, pub props: NodeProps, pub children: Vec<Node> }` | `:125-131` | 两个 crate 的中枢；`render.rs:34`、`layout.rs:353` |
| `pub fn new(kind: Kind, id: impl Into<String>) -> Node` | `:134` | `builder.rs:35`/`:46`/`:87`/`:130`；`scene.rs:280` |
| `pub fn with_layout(mut self, l: LayoutProps) -> Node` | `:145` | `builder.rs:168`；`scene.rs:280` |
| `pub fn with_props(mut self, p: NodeProps) -> Node` | `:150` | `scene.rs:280`（测试/外部用） |
| `pub fn with_id(mut self, id: impl Into<String>) -> Node` | `:156` | `builder.rs:112`/`:168` |
| `pub fn with_label(mut self, label: impl Into<String>) -> Node` | `:161` | `builder.rs:87`/`:92`/`:97`/`:103` |
| `pub fn disabled(mut self) -> Node` | `:166` | 无调用点（仓库内仅定义；`scene.rs:278` 直接写 `nprops.disabled`） |
| `pub fn push(mut self, child: Node) -> Node` | `:171` | 无调用点（`builder.rs:79`、`scene.rs:318` 直接操作 `children`） |
| `pub fn is_container(&self) -> bool` | `:176` | `layout.rs:231` |
| `pub fn walk(&self, f: &mut impl FnMut(&Node, usize), depth: usize)` | `:181` | `deer-gui/examples/render_to_png.rs:77`、`scene_file.rs:65`（数节点） |
| `pub fn structurally_eq(&self, other: &Node) -> bool` | `:189` | `layout_invariants.rs:52`/`:73`/`:262`；`deer-gui/examples/render_to_png.rs:40`、`scene_file.rs:45`、`tutorial.rs:212` |
| `pub struct IdGen { /* private */ }` | `:209-212` | `builder.rs:25`/`:31`/`:43`；`scene.rs:209` |
| `pub fn new() -> IdGen` | `:215` | `builder.rs:31`/`:43`；`scene.rs:209` |
| `pub fn reserve(&mut self, id: &str)` | `:222` | `builder.rs:33`/`:114`/`:169`；`scene.rs:245` |
| `pub fn next(&mut self, kind: Kind) -> String` | `:238` | `builder.rs:44`/`:112`/`:123`/`:154`；`scene.rs:248` |

#### `layout.rs`

| 签名 | 行 | 谁在用它 |
|---|---|---|
| `pub struct TextStyle { pub font_size: f32, pub line_height: f32 }` | `:20-23` | `deer-gui/src/lib.rs:87`/`:150`/`:187`；`render.rs:117-122`；`measure.rs:185`/`:193`；所有测试 |
| `pub trait Measure { fn width(&self, text: &str, style: TextStyle) -> f32; fn height(&self, text: &str, style: TextStyle, max_width: f32) -> f32; }` | `:36-39` | 实现者只有两个：`ApproxMeasure`（`layout.rs:48`）、`FontMeasure`（`crates/deer-gpu/src/measure.rs:182`） |
| `pub struct ApproxMeasure;` | `:46` | `deer-gui/src/lib.rs:83`/`:191`；`render_pipeline.rs:10`；`layout_invariants.rs:19` 等；`text_measure.rs:144`（作对照） |
| `pub mod metrics { pub const BUTTON_PAD_X: f32 = 10.0; pub const BUTTON_MIN_W: f32 = 28.0; pub const BUTTON_MIN_H: f32 = 22.0; pub const FIELD_MIN_W: f32 = 60.0; pub const FIELD_H: f32 = 22.0; }` | `:65-71` | 只被 `layout.rs:147`/`:148`/`:155`/`:156` 自己用 |
| `pub type Intrinsics = HashMap<String, (f32, f32)>;` | `:81` | `measure_tree` 返回值；`layout.rs:184` 的 `PlaceCtx.intrinsic` |
| `pub fn measure_tree(root: &Node, style: TextStyle, m: &impl Measure) -> Intrinsics` | `:83` | 直接被 `layout.rs:179` 调用；测试 `layout_invariants.rs:117`/`:325`/`:338`/`:350` |
| `pub type Geometry = HashMap<String, Rect>;` | `:175` | `render.rs:11`（`Geometry`）；`deer-gui/src/lib.rs:183` 返回它 |
| `pub fn layout(root: &Node, box_: Rect, style: TextStyle, m: &impl Measure) -> Geometry` | `:178` | `deer-gui/src/lib.rs:84`/`:155`/`:184`；`render_pipeline.rs:38`/`:56`/`:85`/`:99`/`:116`/`:130`/`:164` 等 |
| `pub fn hit_test<'a>(root: &'a Node, geo: &Geometry, px: f32, py: f32) -> Option<&'a Node>` | `:353` | `deer-gui/examples/geometry.rs:37`/`:54`；`layout_invariants.rs:233`/`:235`/`:249` |

#### `scene.rs` / `builder.rs`

| 签名 | 行 | 谁在用它 |
|---|---|---|
| `pub struct SceneError { pub message: String, pub line: usize, pub source: String }` | `:24-28` | `parse_scene` 的 `Err` 载荷；`deer-gui/examples/scene_file.rs:56` |
| `pub fn parse_scene(src: &str, source: &str) -> Result<Node, SceneError>` | `:203` | `layout_invariants.rs:50`/`:67`/`:261`/`:280`；`deer-gui/examples/scene_file.rs:38`/`:44`/`:56`、`render_to_png.rs:38`、`tutorial.rs:194` |
| `pub fn encode_scene(root: &Node) -> String` | `:335` | `layout_invariants.rs:96`/`:260`/`:263`；`deer-gui/examples/render_to_png.rs:37`、`scene_file.rs:43`、`tutorial.rs:197` |
| `pub fn new(kind: Kind, id: impl Into<String>) -> Builder` | `:29` | `deer-gui` 全部示例；`render_pipeline.rs:25`；`layout_invariants.rs:24` 等 |
| `pub fn auto(kind: Kind) -> Builder` | `:42` | `layout_invariants.rs:60` |
| `pub fn padding(mut self, v: f32) -> Builder` | `:52` | 示例与测试（`tutorial.rs:57`、`render_pipeline.rs:25` …） |
| `pub fn gap(mut self, v: f32) -> Builder` | `:57` | 同上 |
| `pub fn size(mut self, w: Option<Size>, h: Option<Size>) -> Builder` | `:62` | `layout_invariants.rs:173`/`:196`/`:298` |
| `pub fn text(&mut self, label: impl Into<String>) -> String` | `:86` | `deer-gui` 示例（`tutorial.rs`、`theme.rs`…）、`layout_invariants.rs:25` |
| `pub fn button(&mut self, label: impl Into<String>) -> String` | `:91` | `builder.rs:9`（文档）、示例与测试 |
| `pub fn button_opts(&mut self, label: impl Into<String>, f: impl FnOnce(&mut Node)) -> String` | `:96` | `layout_invariants.rs:304`/`:323`/`:334`/`:346` |
| `pub fn field(&mut self, label: impl Into<String>) -> String` | `:102` | 示例与测试（表单类树） |
| `pub fn container_auto(&mut self, kind: Kind, body: impl FnOnce(&mut Builder))` | `:122` | `crates/deer-gpu/tests/render_pipeline.rs:81`（唯一调用点） |
| `pub fn container(&mut self, kind: Kind, id: impl Into<String>, body: impl FnOnce(&mut Builder))` | `:128` | `builder.rs:10`（文档示例）、`layout_invariants.rs:63` |
| `pub fn container_opts(&mut self, kind: Kind, id: impl Into<String>, layout: LayoutProps, body: impl FnOnce(&mut Builder))` | `:145` | `layout_invariants.rs:27`/`:299`/`:345`；`deer-gui/src/lib.rs:28`（文档）；示例 |
| `pub fn build(&self) -> Node` | `:182` | 所有示例与测试 |
| `pub fn root_id(&self) -> &str` | `:186` | 无调用点 |
| `pub struct L { pub width: Option<Size>, pub height: Option<Size>, pub padding: Option<f32>, pub gap: Option<f32>, pub main_axis: Option<Align>, pub cross_axis: Option<Align>, pub grow: Option<f32> }` | `:193-201` | `deer-gui/src/lib.rs:28`（文档）、`layout_invariants.rs:27`/`:302`/`:345` |
| `pub fn new()/w/h/pad/gap/main/cross/grow/to_props(…)` | `:204`/`:207`/`:211`/`:215`/`:219`/`:223`/`:227`/`:231`/`:235` | 同上（`L::new().gap(8.0).to_props()`） |
| `pub fn props(label: Option<&str>, disabled: bool) -> NodeProps` | `:249` | 无调用点 |

### 4.2 `deer-gpu` —— HAL 契约（`lib.rs`）

| 签名 | 行 | 谁在用它 |
|---|---|---|
| `pub trait Backend { fn name(&self) -> &'static str; fn adapters(&self) -> Vec<AdapterInfo>; fn open(&self, adapter: AdapterIndex) -> GpuResult<Box<dyn Device>>; }` | `:56-63` | 实现：`crates/deer-gpu/src/null.rs:94`（`CpuBackend`）、`crates/deer-vk/src/lib.rs:126`（`VkBackend`）；调用：`deer-gui/examples/vulkan_devices.rs:15`/`:19`、`gpu_offscreen.rs:21`、`vulkan_pipeline.rs:17`、`deer-gpu/tests/draw_list_and_cpu_backend.rs:8`/`:214` |
| `pub type AdapterIndex = usize;` | `:65` | `Backend::open` 形参（`null.rs:107`、`deer-vk/src/lib.rs`） |
| `pub struct AdapterInfo { pub name: String, pub kind: AdapterKind, pub driver: String }` | `:68-74` | `null.rs:100-104` 构造；`deer-vk/src/device.rs:1346-1350`、`deer-vk/src/lib.rs:92-96` 映射 |
| `pub enum AdapterKind { DiscreteGpu, IntegratedGpu, VirtualGpu, Cpu, Other }` | `:77-83` | 同上 |
| `pub enum TargetFormat { Bgra8Srgb, Rgba8Srgb, Rgba8Unorm }` | `:87-93` | `null.rs:193`（返回 `Rgba8Unorm`）；`deer-vk/src/hal.rs:221-225`/`:258-264`；`deer-vk/tests/vulkan_smoke.rs:80` |
| `pub struct Extent { pub width: u32, pub height: u32 }` | `:96-99` | `null.rs:129`/`:211`/`:243-254`；`deer-vk` 大量使用（`swapchain.rs:164`、`gpu_render.rs:532`、`windowed.rs:467`） |
| `pub trait Device { fn info(&self) -> &AdapterInfo; fn create_swapchain(&mut self, window: RawWindowHandle, extent: Extent, format: TargetFormat) -> GpuResult<Box<dyn Swapchain>>; fn create_texture(&mut self, desc: TextureDesc) -> GpuResult<TextureId>; fn upload_texture(&mut self, id: TextureId, data: &[u8], region: TextureRegion) -> GpuResult<()>; fn begin_frame(&mut self) -> GpuResult<Box<dyn Frame>>; fn wait_idle(&mut self) -> GpuResult<()>; }` | `:102-128` | 实现：`null.rs:139`（`CpuDevice`）、`deer-vk/src/hal.rs:69`（`VulkanDevice`） |
| `pub struct RawWindowHandle { pub platform: Platform, pub handle: usize, pub display: usize }` | `:132-137` | `null.rs:146`（忽略）；`deer-vk` 窗口路径 |
| `pub enum Platform { Windows, X11, Wayland, MacOs }` | `:140-145` | `deer-vk` 窗口路径 |
| `pub struct TextureDesc { pub width: u32, pub height: u32, pub format: TargetFormat, pub readable: bool }` | `:148-154` | `deer-vk/src/hal.rs:98`（忽略参数）；`deer-vk/tests/vulkan_smoke.rs:77-81` |
| `pub struct TextureRegion { pub x: u32, pub y: u32, pub width: u32, pub height: u32 }` | `:157-162` | `null.rs:162`（忽略） |
| `pub trait Swapchain { fn extent(&self) -> Extent; fn format(&self) -> TargetFormat; fn resize(&mut self, extent: Extent) -> GpuResult<()>; }` | `:165-170` | 实现：`null.rs:188`（`CpuSwapchain`）、`deer-vk/src/hal.rs:142` |
| `pub trait Frame { fn record(&mut self, list: &DrawList) -> GpuResult<()>; fn read_pixels(&mut self) -> GpuResult<Vec<u8>>; fn submit_and_present(self: Box<Self>) -> GpuResult<PresentResult>; }` | `:173-180` | 实现：`null.rs:226`（`CpuFrame`）、`deer-vk/src/hal.rs:168`；测试走这条路径：`draw_list_and_cpu_backend.rs:229-244` |
| `pub enum PresentResult { Presented, OutOfDate }` | `:183-187` | `null.rs:255`（恒 `Presented`）；`deer-vk` 呈现路径 |
| `pub trait Renderer { fn build_draw_list(&self, tree: &Node, geo: &Geometry, theme: &Theme) -> DrawList; }` | `:193-195` | **无任何实现**（全仓库 `grep 'impl Renderer for'` 无命中）；实际走的是自由函数 `build_draw_list`（`render.rs:148`）。`DefaultRenderer`（`render.rs:17`）没实现它，只有同名的固有方法 `build` |
| `pub struct Theme { pub text: Color, pub text_dim: Color, pub surface: Color, pub border: Color, pub accent: Color, pub on_accent: Color, pub font_size: f32, pub line_height: f32 }` | `:199-208` | `deer-gui/src/lib.rs:42`（re-export）、`prelude`（`:62`）；示例与测试大量使用 |

### 4.3 `deer-gpu` —— 绘制数据（`draw.rs`）

| 签名 | 行 | 谁在用它 |
|---|---|---|
| `pub struct Color { pub r: u8, pub g: u8, pub b: u8, pub a: f32 }` | `:7-12` | 全链路；`Theme`（`lib.rs:200-207`） |
| `pub const fn rgb(r: u8, g: u8, b: u8) -> Color` | `:15` | `lib.rs:213-218`（默认主题）、测试 |
| `pub const fn rgba(r: u8, g: u8, b: u8, a: f32) -> Color` | `:18` | `lib.rs:215`（`surface`）、`null.rs:253` |
| `pub const TRANSPARENT: Color` / `pub const WHITE: Color` | `:21` / `:22` | `render.rs:153`（再导出）；`deer-vk` 测试大量使用 |
| `pub const fn packed(self) -> u32` | `:25-27` | 无仓库内调用点（断言/uniform 预留） |
| `pub struct RectI { pub x: i32, pub y: i32, pub w: i32, pub h: i32 }` | `:32-37` | `render.rs:36`、`null.rs` 全域、`deer-vk/src/gpu_geom.rs` |
| `pub const fn new(x: i32, y: i32, w: i32, h: i32) -> RectI` | `:40` | `render.rs:36`/`:84`/`:134`；`null.rs:313`/`:325`/`:424-427`/`:467` |
| `pub fn right(&self) -> i32` / `pub fn bottom(&self) -> i32` | `:43` / `:46` | `null.rs`（`fill`/`stroke`/`inside_rounded`/`draw_text`）；`deer-vk/src/gpu_geom.rs` |
| `pub fn contains(&self, px: i32, py: i32) -> bool` | `:49-51` | `null.rs:371`（裁剪判据） |
| `pub struct TextureId(pub u32);` | `:56` | `null.rs:154`（`create_texture` 分配）、`deer-vk` |
| `pub enum DrawCmd { FillRect{rect,color}, StrokeRect{rect,color,width}, FillRoundRect{rect,radius,color}, Text{rect,text,color,size,align}, PushClip{rect}, PopClip, NodeHint{rect,node_id_len,node_id_fp} }` | `:92-131` | 生产：`render.rs:46-108`、`:129`（`DrawCmd::node_hint`）；消费：`null.rs:317-353`（CPU）、`deer-vk/src/gpu_geom.rs:142-194`（GPU）；打印：`deer-gui/examples/draw_list.rs:38-54` |
| `pub struct DrawList { pub cmds: Vec<DrawCmd>, /* private */ clip_balance: i32 }` | `:89-93` | 全链路；`deer-vk/tests/gpu_geom_stream.rs:468-469` 直接写 `cmds`（绕过 `push`） |
| `pub fn new() -> DrawList` | `:96` | `render.rs:29`/`:130`；测试 |
| `pub fn from_cmds(cmds: Vec<DrawCmd>) -> DrawList` | `:104` | `null.rs:245`（`CpuFrame::submit_and_present` 重建列表）；测试 `draw_list_and_cpu_backend.rs:40-49` |
| `pub fn push(&mut self, cmd: DrawCmd)` | `:112` | `render.rs:46-108`/`:133`；测试 |
| `pub fn len(&self) -> usize` / `pub fn is_empty(&self) -> bool` | `:121` / `:125` | `len()`：`deer-gui/examples/draw_list.rs:36`、`text_render.rs:67`；`is_empty()`：**无仓库内调用点** |
| `pub fn clip_balanced(&self) -> bool` | `:130` | `null.rs:228`（`record`）、`:293`（`render`） |
| `pub fn counts(&self) -> DrawCounts` | `:135` | `deer-gui/examples/draw_list.rs:59`、`tutorial.rs:246`、`gpu_geometry.rs:99`；`crates/deer-gpu/tests/render_pipeline.rs:42`/`:119`、`draw_list_and_cpu_backend.rs:52` |
| `pub struct DrawCounts { … }` | `:153-161` | `counts()` 返回值；`draw_list.rs:61-62` 打印 |

### 4.4 `deer-gpu` —— CPU 后端与渲染器

| 签名 | 行 | 谁在用它 |
|---|---|---|
| `pub const CPU_ADAPTER_NAME: &str = "deer-cpu (software rasterizer)"` | `:19` | `null.rs:101` 自己用 |
| `pub struct Framebuffer { pub width: u32, pub height: u32, pub pixels: Vec<u8> }` | `:23-28` | `deer-gui/src/lib.rs:61`（prelude）；`deer-vk` 对照测试 |
| `pub fn new(width: u32, height: u32, clear: Color) -> Framebuffer` | `:31` | `null.rs:299`；`draw_list_and_cpu_backend.rs:248` |
| `pub fn clear(&mut self, c: Color)` | `:41` | `Framebuffer::new` 内部（`:37`） |
| `pub fn pixel(&self, x: i32, y: i32) -> Option<[u8; 4]>` | `:52` | 大量像素断言：`draw_list_and_cpu_backend.rs:69-73`/`:94-95`/`:115-116`/`:130-131`/`:143-144`/`:169`/`:249-251`、`text_pixels.rs:56`/`:172`、`deer-gui/examples/tutorial.rs:265` |
| `pub fn count_color(&self, c: Color) -> usize` | `:66` | `draw_list_and_cpu_backend.rs:75`/`:93`/`:114`/`:129`/`:145`/`:161`/`:252`；`render_pipeline.rs:149` |
| `pub fn bytes_eq(&self, other: &Framebuffer) -> bool` | `:75` | 确定性断言：`draw_list_and_cpu_backend.rs:194`/`:254`、`render_pipeline.rs:160`、`text_pixels.rs:92`/`:121`、`deer-gui/examples/text_render.rs:112`/`:115` |
| `pub fn to_rgba(&self) -> &[u8]` | `:79` | `draw_list_and_cpu_backend.rs:195`、`render_pipeline.rs:151`、`deer-vk/tests/gpu_vs_cpu.rs:123`、`gpu_geom_parity.rs:169`、`deer-gui/examples/text_render.rs:88` |
| `pub struct CpuBackend;` + `pub fn new() -> CpuBackend` | `:86` / `:89` | `deer-gui/examples/vulkan_devices.rs:19`；`draw_list_and_cpu_backend.rs:214` |
| `pub struct CpuDevice` | `:119-122` | 由 `CpuBackend::open` 返回（`:111`） |
| `pub fn render_to_framebuffer(&self, extent: Extent, list: &DrawList, clear: Color) -> GpuResult<Framebuffer>` | `:129-136` | 无仓库内调用点（测试都直接调 `CpuRenderer`） |
| `pub struct CpuFrame` + `pub fn set_extent(&mut self, extent: Extent)` + `pub fn framebuffer(&self) -> Option<&Framebuffer>` + `pub fn render_into(fb: &mut Framebuffer, list: &DrawList) -> GpuResult<()>` | `:202` / `:211` / `:216` / `:221` | `draw_list_and_cpu_backend.rs:230-244`（HAL Frame 路径）、`:247-254`（`render_into`） |
| `pub struct CpuRenderer` | `:268-270` | `deer-gui/src/lib.rs:61`（prelude）、`:94`/`:163`；`render_pipeline.rs:6`；`text_pixels.rs:15` |
| `pub fn new() -> CpuRenderer`（无字库，文字走占位格） | `:274` | `deer-gui/src/lib.rs:94`；`draw_list_and_cpu_backend.rs:67` 等；`render_pipeline.rs:132`/`:166` |
| `pub fn with_text(engine: TextEngine) -> CpuRenderer` | `:279` | `deer-gui/src/lib.rs:163`；`text_pixels.rs:46` |
| `pub fn text(&self) -> Option<&TextEngine>` | `:284` | `deer-gui/examples/text_render.rs:90`/`:99` |
| `pub fn render(&mut self, extent: Extent, list: &DrawList, clear: Color) -> GpuResult<Framebuffer>` | `:292` | 上述全部渲染测试 + `deer-gui` 门面 + `deer-vk` 对照测试 |

### 4.5 `deer-gpu` —— 渲染器 / 字形 / 度量 / 图集 / 光栅 / 字体 / PNG

| 签名 | 行 | 谁在用它 |
|---|---|---|
| `pub struct DefaultRenderer<'a, M: Measure> { pub theme: Theme, pub measure: &'a M }` | `:17-20` | `deer-gui/src/lib.rs:60`（prelude）；`render_pipeline.rs:7`/`:39`/`:57`/`:86`/`:100` |
| `pub fn new(theme: Theme, measure: &'a M) -> Self` | `:23` | 同上 |
| `pub fn build(&self, tree: &Node, geo: &Geometry) -> DrawList` | `:28` | `render.rs:149`（`build_draw_list` 内部）；`render_pipeline.rs` |
| `pub struct NullRenderer;` + `pub fn build(tree: &Node, geo: &Geometry) -> DrawList` | `:126` / `:129` | `render_pipeline.rs:7`/`:117` |
| `pub fn build_draw_list<M: Measure>(tree: &Node, geo: &Geometry, theme: Theme, measure: &M) -> DrawList` | `:148` | `deer-gui/src/lib.rs:93`/`:161`；示例 `draw_list.rs:34`、`tutorial.rs:245`、`gpu_geometry.rs:96` |
| `pub const TRANSPARENT: Color` | `:153` | 无仓库内调用点（再导出便捷常量） |
| `pub struct GlyphImage { pub width: u32, pub height: u32, pub left: i32, pub top: i32, pub advance: f32, pub coverage: Vec<u8> }` | `glyph.rs:25-43` | `raster.rs:216`（构造）、`atlas.rs:120`（消费）、`deer-gui/src/lib.rs:64`（prelude）、`deer-gui/examples/glyph_atlas.rs:35` |
| `pub fn blank(advance: f32) -> GlyphImage` | `glyph.rs:47` | `raster.rs:113/117/121/124/127/130/140/143/189` |
| `pub fn new(width: u32, height: u32, left: i32, top: i32, advance: f32, coverage: Vec<u8>) -> GlyphImage` | `glyph.rs:59` | `raster.rs:216`；测试 |
| `pub fn is_blank(&self) -> bool` | `glyph.rs:83` | `glyph_atlas.rs:287`、`text_raster.rs:305`/`:329`/`:571` |
| `pub fn coverage_at(&self, x: u32, y: u32) -> u8` | `glyph.rs:88` | `text_raster.rs`（大量像素断言，`:183-186`/`:223-228`/`:277-287`/`:358-391`/`:472-538`/`:580`） |
| `pub fn coverage_sum(&self) -> u64` / `pub fn max_coverage(&self) -> u8` / `pub fn ink_pixels(&self, threshold: u8) -> usize` / `pub fn mean_coverage(&self) -> f32` | `glyph.rs:96` / `:101` / `:106` / `:114` | `text_raster.rs:151-157`、`:231`/`:260`、`:358`；`deer-gui/examples/glyph_atlas.rs:99`/`:100`/`:117` |
| `pub struct GlyphKey { pub glyph_index: u16, pub px_size: u16 }` + `pub fn new(glyph_index: u16, px_size: u16) -> GlyphKey` | `glyph.rs:127-132` / `:135` | `text.rs:151`；测试 `glyph_atlas.rs:26`；示例 `glyph_atlas.rs:35` |
| `pub struct AtlasSlot { pub x: u32, pub y: u32, pub w: u32, pub h: u32 }` + `right` / `bottom` / `overlaps` | `glyph.rs:148-153` / `:156` / `:159` / `:163` | `atlas.rs:130-143`/`:185-190`；`text.rs:47`（`GlyphPlacement.slot`）；`null.rs:529-535`；`glyph_atlas.rs:74`（不重叠断言） |
| `pub struct GlyphAtlas` + `pub fn new(width: u32) -> GlyphAtlas` | `atlas.rs:66` / `:90` | `text.rs:78`（`ATLAS_WIDTH = 512`）；示例 `deer-gui/examples/glyph_atlas.rs:33`；`glyph_atlas.rs:113` 等 |
| `pub fn insert(&mut self, key: GlyphKey, image: &GlyphImage) -> Option<AtlasSlot>` | `atlas.rs:120` | `text.rs:165`；`glyph_atlas.rs:113`+ 多处 |
| `pub fn get(&self, key: GlyphKey) -> Option<(AtlasSlot, &[u8])>` | `atlas.rs:227` | `glyph_atlas.rs` 多处（`⑧`/`⑨` 断言）；**`null.rs` 不用它**（CPU 采样走 `coverage()`：`null.rs:525`） |
| `pub fn contains(&self, key: GlyphKey) -> bool` | `atlas.rs:233` | `crates/deer-gpu/tests/glyph_atlas.rs:140`/`:301`/`:333`/`:350` |
| `pub fn size(&self) -> (u32, u32)` / `pub fn coverage(&self) -> &[u8]` / `pub fn len(&self) -> usize` / `pub fn is_empty(&self) -> bool` | `atlas.rs:238` / `:243` / `:248` / `:253` | `text.rs:213-214`（`atlas_png`）、`null.rs:524`（`size`）、`:525`（`coverage`）；`glyph_atlas.rs:494`（`len`） |
| `pub fn used_pixels(&self) -> usize` / `pub fn utilization(&self) -> f32` | `atlas.rs:258` / `:263` | `glyph_atlas.rs:181-187`/`:282-306`/`:336`/`:418-424`/`:487`；`deer-gui/examples/glyph_atlas.rs:131-142` |
| `pub const MAX_DIMENSION: u32 = 8192` | `atlas.rs:55` | `glyph_atlas.rs:23`（测试用它构造超高用例） |
| `pub struct Rasterizer { pub ppem: f32, pub supersample: u32 }` + `new(ppem)` + `with_supersample(ppem, supersample)` | `raster.rs:79-84` / `:88` / `:96` | `text.rs:163`（`Rasterizer::new(px_key as f32)`）；`text_raster.rs` 大量（`:140`/`:175`/`:195`/…）；示例 `glyph_atlas.rs:38`/`:183` |
| `pub fn rasterize(&self, g: &Glyph, units_per_em: u16) -> GlyphImage` | `raster.rs:109` | `text.rs:164`；`text_raster.rs:140` 等 |
| `pub fn rasterize_char(&self, font: &Font, ch: char) -> GpuResult<Option<GlyphImage>>` | `raster.rs:223` | `text_raster.rs:472`/`:526`/`:568`/`:580`；`deer-gui/examples/glyph_atlas.rs:183` |
| `pub struct FontMeasure<'a> { pub font: &'a Font, pub font_size: f32 }` + `new` | `measure.rs:48-53` / `:57` | `text.rs:127`（`TextEngine::measure`）；`text_measure.rs:40`/`:55`/`:113`/`:334`/`:371`；`deer-gui/src/lib.rs:65`（prelude） |
| `pub fn scale(&self)` / `ascent` / `descent` / `line_height` / `advance(&self, ch: char) -> f32` | `measure.rs:65` / `:74` / `:83` / `:88` / `:95` | `null.rs:517-518`（`ascent`/`descent` 定基线）；`text_measure.rs` 各条 |
| `pub fn text_width(&self, text: &str) -> f32` | `measure.rs:114` | `text_measure.rs:157`/`:169`/`:188-205`/`:239`/`:286`/`:307`/`:353`/`:380`；`text_pixels.rs:274`/`:292` |
| `pub fn wrap(&self, text: &str, max_width: f32) -> Vec<String>` | `measure.rs:124` | `measure.rs:194`（`Measure::height`）；`text_measure.rs:211` 起 |
| `pub fn find_system_font() -> Option<PathBuf>` | `measure.rs:205` | `text.rs:102`；`text_measure.rs:386`；示例 `glyph_atlas.rs:36` |
| `pub struct GlyphPlacement { pub slot: AtlasSlot, pub left: i32, pub top: i32, pub advance: f32 }` | `text.rs:46-55` | `text.rs:172`（构造）、`null.rs:503-552`（消费）；prelude（`deer-gui/src/lib.rs:67`） |
| `pub struct TextEngine` + `from_font` / `from_font_bytes` / `from_font_file` / `from_system_font` | `text.rs:58-65` / `:74` / `:85` / `:90` / `:101` | `deer-gui/src/lib.rs:133`（`from_font_file`）；`text_pixels.rs:24`（`from_system_font`）；`null.rs:279`（`with_text` 的入参） |
| `pub fn font(&self) -> &Font` / `font_size` / `set_font_size` / `measure` | `text.rs:112` / `:116` / `:121` / `:126` | `measure`：`deer-gui/src/lib.rs:159`/`:161`；`text_pixels.rs:274`/`:292`。其余三个无仓库内调用点 |
| `pub fn glyph(&mut self, ch: char, px_size: f32) -> Option<GlyphPlacement>` | `text.rs:138` | `null.rs:504`（唯一生产消费点）；`text.rs:189`（`text_width` 内部） |
| `pub fn text_width(&mut self, text: &str, px_size: f32) -> f32` | `text.rs:186` | `text_pixels.rs:254-255`/`:273`/`:291` |
| `pub fn rasterized_glyphs(&self) -> usize` / `missing_glyphs` / `atlas` / `atlas_png` | `text.rs:197` / `:203` / `:207` / `:212` | `deer-gui/examples/text_render.rs:91`（`atlas_png`）、`:100`（`rasterized_glyphs`/`missing_glyphs`/`atlas`）；`text_pixels.rs:279`/`:289`（`missing_glyphs`） |
| `pub struct Point { pub x: f32, pub y: f32, pub on_curve: bool }` | `font.rs:39-43` | `font.rs:832`（`flatten_contour`）、`deer-vk` 无关 |
| `pub enum Segment { Line{to}, Quad{ctrl,to}, Cubic{c1,c2,to} }` | `font.rs:47-58` | `font.rs:828-886`（构造）、`raster.rs:156-175`（消费）；测试 `text_raster.rs:14`、`font_parse.rs:16` |
| `pub struct Contour { pub start: (f32, f32), pub segments: Vec<Segment> }` | `font.rs:62-65` | `font.rs:600`/`:697`（构造）、`raster.rs:151`（消费）；测试构造器 `text_raster.rs:31` |
| `pub struct Glyph { pub contours: Vec<Contour>, pub bbox: (i16,i16,i16,i16), pub advance_width: u16, pub left_side_bearing: i16 }` | `font.rs:69-78` | `font.rs:526`/`:549`（构造）、`measure.rs:99`（`advance_width`）、`raster.rs:109`（消费） |
| `pub fn is_blank(&self) -> bool` / `pub fn outline_bbox(&self) -> Option<(f32,f32,f32,f32)>` | `font.rs:82` / `:90` | `raster.rs:126`；测试 `font_parse.rs:142`/`:145`、`font_synthetic.rs:335`/`:351`/`:353`、`text_raster.rs:302` |
| `pub enum CmapFormat { Format0, Format4, Format6, Format12 }` | `font.rs:112-117` | `Font::cmap_format` 字段（`:132`/`:341`） |
| `pub struct Font { /* private data/tables */ pub units_per_em: u16, pub num_glyphs: u16, pub ascender: i16, pub descender: i16, pub line_gap: i16, pub cmap_format: CmapFormat }` | `font.rs:120-137` | `measure.rs:66`/`:75`/`:84`/`:89`、`text.rs:164`、`raster.rs:115`/`:228`；测试 `font_parse.rs:39`、`font_synthetic.rs:284` |
| `pub fn parse(data: Vec<u8>) -> GpuResult<Font>` | `font.rs:176` | `text.rs:86`；测试 `font_parse.rs:19`、`font_synthetic.rs:284`、`text_measure.rs:36` |
| `pub fn table_range(&self, name: &[u8;4]) -> Option<(u32,u32)>` / `table_directory` / `face_offset` | `font.rs:352` / `:360` / `:365` | 诊断 API；**无仓库内调用点**（`font_synthetic.rs:459` 自己写了 `find_table_offset`） |
| `pub fn glyph_index(&self, ch: char) -> GpuResult<Option<u16>>` | `font.rs:399` | `measure.rs:97`、`text.rs:142`、`raster.rs:224`；测试 `font_parse.rs`、`font_synthetic.rs:299` |
| `pub fn glyph(&self, glyph_index: u16) -> GpuResult<Glyph>` | `font.rs:506` | `measure.rs:98`/`:103`、`text.rs:156`、`raster.rs:227` |
| `pub fn line_height_units(&self) -> f32` | `font.rs:786` | `measure.rs:89`；测试 `font_parse.rs:69-74`、`font_synthetic.rs:294` |
| `pub fn encode_rgba(width: u32, height: u32, pixels: &[u8]) -> Result<Vec<u8>, String>` | `png.rs:14` | `text.rs:226`（`atlas_png`）；`deer-gui/src/lib.rs:111`/`:179`；示例 `pixels.rs:61`、`gpu_offscreen.rs:117`、`tutorial.rs:270`、`glyph_atlas.rs:37` |
| `pub enum GpuError { NoAdapter, Unsupported(String), Driver{code: i32, message: String}, OutOfDate, BadWindowHandle }` + `pub type GpuResult<T> = Result<T, GpuError>` | `error.rs:9-20` / `:36` | `null.rs:109`（`NoAdapter`）、`:229`/`:294`（`Driver`）；`text.rs:92`（`Unsupported`）；`deer-gui/src/lib.rs:42` 再导出；`deer-vk` 全域 |

---

## 5. CPU 后端语义要点（**GPU 逐像素一致性的黄金基准**）

> 全部锚点集中在 `crates/deer-gpu/src/null.rs`。这一节写得最细，因为第 3 讲与 GPU parity 都以它为准。

### 5.1 坐标与遍历口径

| 项 | 事实 | 锚点 |
|---|---|---|
| 遍历单位 | **整数像素坐标**（`i32`），不是像素中心。`fill` 直接 `for y in rect.y..rect.bottom() { for x in rect.x..rect.right() }` | `:389-390` |
| 矩形区间 | **半开区间**：`x ∈ [rect.x, rect.right())`，`y ∈ [rect.y, rect.bottom())`；`right = x + w`、`bottom = y + h` | `:389-390`、`draw.rs:43-48` |
| 裁剪判据 | 也是**半开区间 + 整数比较**：`px >= self.x && py >= self.y && px < self.right() && py < self.bottom()` | `draw.rs:49-51`，被 `:371` 调用 |
| 退化矩形 | `w <= 0` 或 `h <= 0` ⇒ `Range` 为空 ⇒ **一个像素都不画**（不 panic） | `:389-390`（如 `rect.w = 0` 时 `x..x` 为空） |
| 取整方式 | 几何在布局阶段已 `round()` 成整数（`layout.rs:223-224`、`:228`），`render.rs` 再 `as i32` 截断（已是整数） | `layout.rs:223`、`render.rs:36` |

### 5.2 `blend` / `blend_cov`（唯一的写像素入口）

`fn blend(fb, x, y, c, clip)` = `blend_cov(fb, x, y, c, 1.0, clip)`（`:363-365`）。

判据细节（`:370-385`）：

1. **越界/裁剪先行**：`if !clip.contains(x, y) || x < 0 || y < 0 || x >= fb.width as i32 || y >= fb.height as i32 { return; }`（`:371`）。
   - 注意是**双重检查**：`clip` 通常已被初始化为整屏（`:313`），但 `clip` 是**矩形交集**推出来的，所以帧缓冲边界检查是冗余但保底的第二道闸。
   - 越界像素**直接跳过**（不 clamp、不 wrap）—— 因此「越界 alpha」问题不存在：越界根本写不进去。
2. **alpha 合成系数**：`let a = c.a.clamp(0.0, 1.0) * cov.clamp(0.0, 1.0);`（`:374`）。
   - `cov` 是**覆盖率乘子**，字形路径传 `cov as f32 / 255.0`（`:546`），矩形路径传 `1.0`。
   - 上限 1.0 ⇒ `a ∈ [0,1]`；**这是唯一一次对 alpha 的规范化**。
3. **`a <= 0.0` 直接返回**（`:375-377`）：所以 `Color::TRANSPARENT`（`draw.rs:21`）与 `cov == 0` 都是**零成本 no-op**，不写像素（也解释了 `draw_text_real` 里 `if cov == 0 { continue; }` 的额外短路 `:538-540`）。
4. **RGB 用 `round()`**：`fb.pixels[i] = (c.r as f32 * a + fb.pixels[i] as f32 * inv).round() as u8;`，三通道同式，`inv = 1.0 - a`（`:379-382`）。
   - 是**整数源色 × 浮点权重的四舍五入**，不是浮点直通、也不是截断。**这是黄金基准里最容易被 GPU 走样的一行。**
   - 目标色从帧缓冲**原样读**（不做 sRGB 解码 / 线性化）—— 全套混合都在「存储值空间」做。
5. **Alpha 通道也混合，且读回目标值**：`let da = fb.pixels[i + 3] as f32 / 255.0; fb.pixels[i + 3] = ((a + da * inv).clamp(0.0, 1.0) * 255.0).round() as u8;`（`:383-384`）。
   - 即 `dst_a' = a + dst_a*(1-a)`，再 `clamp(0,1)`、再 `round()`、再 `×255`。
   - 注意与 `Framebuffer::clear` 的口径**不同**：`clear` 用的是**截断** `(c.a.clamp(0.0,1.0) * 255.0) as u8`（`:42`）；`count_color` 也用截断（`:67`）。**同一个 alpha 在这两处的取整规则不一致**（`blend` round / `clear` trunc）——写像素断言时要注意。

### 5.3 `fill`（矩形与圆角填充）

```rust
let r = radius.max(0);                     // :388
for y in rect.y..rect.bottom() {           // :389
    for x in rect.x..rect.right() {        // :390
        if r > 0 && !inside_rounded(rect, x, y, r) { continue; }   // :391-393
        blend(fb, x, y, color, clip);      // :394
    }
}
```

- `FillRect` 传 `radius = 0`（`:330`）；`FillRoundRect` 传 `*radius`（`:331`）。
- **负半径视同无圆角**（`max(0)`，`:388`）。
- 遍历**不做 clip 裁剪**（先遍历整矩形，靠 `blend` 过滤）：成本与裁剪区无关。GPU 侧对应实现见 `crates/deer-vk/src/gpu_geom.rs:159`（注释明确「与 CPU `fill(.., radius.max(0))` 一致」）。

### 5.4 `inside_rounded`（圆角判据 —— 第 3 讲的重点）

```rust
let corners = [
    (rect.x + r,             rect.y + r,              -1, -1),   // :402
    (rect.right() - 1 - r,   rect.y + r,               1, -1),   // :403
    (rect.x + r,             rect.bottom() - 1 - r,   -1,  1),   // :404
    (rect.right() - 1 - r,   rect.bottom() - 1 - r,    1,  1),   // :405
];
```

逐条事实：

1. **角心是整数坐标**，不是像素中心（对比 `raster.rs:203-205` 的采样点用 `+0.5` 像素中心）。左上角心 = `(rect.x + r, rect.y + r)`；右下角心 = `(rect.right() - 1 - r, rect.bottom() - 1 - r)` —— **右下/右上用 `right() - 1` 的「末像素」约定**，左上没有 `-1`（`:402-405`）。
2. **是否落在角象限**用**严格不等号**：`in_corner_x = if sx < 0 { x < ccx } else { x > ccx }`，同理 y（`:408-409`）。所以 `x == ccx` 或 `y == ccy` 的像素**不算角内**，直接算实心。
3. **角内判据**：`dx*dx + dy*dy > (r*r) as f32` ⇒ **剔掉**（`return false`）（`:411-414`）。
   - 即 **保留 `dx² + dy² <= r²`**（等号保留，是「<=」，因为只有 `>` 才剔除）。
   - 与中心的整数距离：`dx = (x - ccx) as f32`、`dy = (y - ccy) as f32`（`:411-412`）。
4. **不做抗锯齿**：本函数只返回 `bool`，没有任何覆盖率/alpha 输出；模块注释明确写「不做抗锯齿 —— 抗锯齿属于后端能力，后续加」（`:399`）。
5. `r` 不会被夹到「不超过半宽/半高」：`fill` 只做了 `max(0)`（`:388`）。因此 `r` 很大时角心会互相越界/重叠，判据仍然**确定性**但几何上退化（例：`rect = (1,1,9,7), r = 9` 时右上角心 `right()-1-r = -1`）。仓库里对这类退化的对照只在 `crates/deer-vk/tests/gpu_vs_cpu.rs:180`（`"round-huge", radius: 9`）里出现；**CPU 侧是否单独钉住超大半径：未确认**。
6. 判定顺序：四个角**依次检查，任一剔除即返回 false**（`:407-417`），最后 `true`（`:418`）。

### 5.5 `stroke`（描边）

```rust
let w = width.max(1);                                                  // :422
for k in 0..w {                                                        // :423
    fill(fb, RectI::new(rect.x,             rect.y + k,        rect.w,  1),       color, clip, 0);  // :424
    fill(fb, RectI::new(rect.x,             rect.bottom()-1-k, rect.w,  1),       color, clip, 0);  // :425
    fill(fb, RectI::new(rect.x + k,         rect.y,            1, rect.h),        color, clip, 0);  // :426
    fill(fb, RectI::new(rect.right()-1-k,   rect.y,            1, rect.h),        color, clip, 0);  // :427
}
```

- **线宽 `<= 0` 视同 1**（`:422`）。`deer-vk` 侧对应 `(*width).max(1)`（`crates/deer-vk/src/gpu_geom.rs:166`）。
- 描边**总是 4 条厚度 1 的边带并集**，圆角参数固定传 `0`（`:424-427` 第 5 个实参）。
- **四角像素会被重复绘制**：上/下边带横跨整个 `rect.w`（含左右两端），左/右边带竖跨整个 `rect.h`（含上下两端），所以角上像素被 `blend` 2 次（`w = 1` 时）；`w >= 2` 时更多次。
  - 对**不透明色**（`a = 1.0`）无影响（第二次 `a=1` 写同一值）；对**半透明色**，角像素会被复合多次 ⇒ 比边上像素更实/更偏色。这是由代码直接推出的事实（`:424-427` + `:370-385`）；**是否有测试专门钉住这一点：未确认**。
- 边带本身也可能互相重叠（`width` 大于矩形的一半时），例如 `crates/deer-vk/tests/gpu_vs_cpu.rs:185-191` 的 `stroke-band-exceeds-rect` / `stroke-tiny-rect-thick` 用例。
- 描边与填充**共用同一个 `blend` 与同一个 `clip`**，没有额外的内缩/外扩。

### 5.6 裁剪栈（`PushClip`/`PopClip`）

```rust
let full = RectI::new(0, 0, fb.width as i32, fb.height as i32);   // :313
let mut clip = full; let mut stack: Vec<RectI> = Vec::new();      // :314-315
```

- `PushClip`：`stack.push(clip)` 后做**交集**：`x = max(clip.x, rect.x)`、`y = max(clip.y, rect.y)`、`r = min(clip.right(), rect.right())`、`b = min(clip.bottom(), rect.bottom())`，然后 `RectI::new(x, y, (r-x).max(0), (b-y).max(0))`（`:319-326`）。
  - 宽高用 `max(0)` ⇒ **交集为空时得到一个零面积矩形**，后续 `blend` 的 `contains` 全 false（`:325`）。
- `PopClip`：`clip = stack.pop().unwrap_or(full)`（`:327-329`）。
  - **栈空时静默回退到整屏**，不报错 —— 也就是说 CPU 光栅化本身**不把「多弹一次」当成错误**。
  - 但列表级检查会先拦住它：`DrawList::push` 会把 `clip_balance` 减成负（`draw.rs:112-119`），`clip_balanced()` 返回 false（`draw.rs:130`），`CpuRenderer::render`（`null.rs:293-298`）与 `CpuFrame::record`（`:227-233`）都会返回 `GpuError::Driver { code: -1, .. }`。
  - 因此 `unwrap_or(full)` 这条兜底路径只能通过**直接改 `cmds`**（绕过 `push`）到达 —— 仓库里确实有这种用法：`crates/deer-vk/tests/gpu_geom_stream.rs:468-469`。
- GPU 侧的裁剪**不用 scissor**，而是把交集算在 CPU：`crates/deer-vk/src/gpu_geom.rs:269` 的注释明确写「与 `null.rs::soft_rasterize_with` 的裁剪算法逐字一致」，并且它用自己的 `depth`/`pop_without_push` 记账（`gpu_geom.rs:197`），不读 `DrawList::clip_balanced`。

### 5.7 文字：两条路径

**A. 占位路径 `draw_text`（`CpuRenderer::new()`，无字库）**（`:448-480`）

- 每字符等宽格：`advance = ((size * 0.6).round() as i32).max(1)`（`:457`）；`total = advance * count`（`:458`）。
- 起点按 `align`：`1 => rect.x + (rect.w - total) / 2`、`2 => rect.right() - total`、`_ => rect.x`（`:459-463`）。
  - **整数除法（向零截断）**，不是浮点。
- 字形高：`glyph_h = (size.min(rect.h as f32).round() as i32).max(1)`（`:464`）。
- 每字符的墨迹矩形：`(x + 1, rect.y + 1, (advance-2).max(1), (glyph_h-2).max(1))`（`:467-472`），逐像素 `blend`（`:473-477`），笔位置 `x += advance`（`:478`）。
- **不做换行**：一行到底，超出 `rect` 的部分靠 `clip` 与帧缓冲边界拦（没有 per-glyph 的 rect 裁剪）。

**B. 真字形路径 `draw_text_real`（`CpuRenderer::with_text`）**（`:493-554`）

1. 逐字符 `engine.glyph(c, size)` 收集 `Vec<Option<GlyphPlacement>>`（`:503-504`）；`total` = 所有 `Some` 的 `advance` 之和（`:505`）。
2. 起点按同一套对齐语义，但**用浮点**：`1 => rect.x as f32 + (rect.w as f32 - total)/2.0`、`2 => rect.right() as f32 - total`、`_ => rect.x as f32`（`:508-512`）。
   - 与占位路径的整数除法**取整方式不同**（占位路径截断、这里保留小数，随后 `pen.round()`）；两者在奇数差值上可能差 1px。
3. **基线**：`top_of_text_block = rect.y + ((rect.h - (ascent + descent))/2).round()`，`baseline = (top_of_text_block + ascent.round()).round()`，其中 `ascent/descent` 来自 `engine.measure()`（`FontMeasure::ascent/descent`，`measure.rs:74`/`:83`）（`:515-520`）。
   - 模块注释强调：基线与 `Theme::line_height` 无关，`rect` 已是布局算好的盒子（`:490-492`）。
4. 采样：`atlas = engine.atlas()`、`atlas_w = atlas.size().0`、`coverage = atlas.coverage()`（`:523-525`）。
5. 落位与采样（`:527-553`）：
   - `gx0 = pen.round() as i32 + p.left`、`gy0 = baseline as i32 - p.top`（`:530-531`）。
   - 逐行 `row = (p.slot.y + gy) * atlas_w`，逐列 `coverage.get(row + (p.slot.x + gx))`；**取不到就 `continue`**（`:533-537`）。
   - `cov == 0` 跳过（`:538-540`）。
   - `blend_cov(fb, gx0+gx, gy0+gy, color, cov as f32/255.0, clip)`（`:541-548`）。
   - `pen += p.advance`（`:552`）；`slot.w == 0 || slot.h == 0`（空格）⇒ 不画墨但**仍然推进笔位置**（`:529`）。
6. 采样是**最近邻的点采样**（直接索引 `coverage`，无插值/无双线性）。

### 5.8 CPU 后端的其他确定性细节

| 事实 | 锚点 |
|---|---|
| `CpuRenderer::render` 对 `extent` 取 `max(1)` ⇒ **0×0 等价于 1×1** | `:299`（`CpuFrame::submit_and_present` 同样 `:249-251`）；`deer-vk` 侧同约定 `crates/deer-vk/tests/gpu_geom_parity.rs:245` |
| `CpuFrame::submit_and_present` 的 clear 色**硬编码为 `Color::rgba(0,0,0,1.0)`**，不是主题色 | `:253` |
| `CpuFrame::record` 会 `list.cmds.clone()`，并先做裁剪平衡检查 | `:227-236` |
| `CpuFrame::read_pixels` 在提交前返回**空 Vec** | `:238-240` |
| `CpuDevice::begin_frame` 返回 `extent {0,0}`，必须靠 `set_extent` 指定尺寸（CPU 无窗口可查） | `:168-177`、`:203-213` |
| `CpuDevice::render_to_framebuffer` 走的是**无字库**渲染器（`CpuRenderer::new()`），因此走这条路**画不出真字** | `:135` |
| `DrawCmd::NodeHint` 被完全忽略 | `:352` |
| `CpuSwapchain::format()` 恒返回 `Rgba8Unorm` | `:192-194` |
| 全流程**无随机、无时间、无 HashMap 迭代序依赖**（`soft_rasterize_with` 顺序遍历 `cmds`） | `:317-354` |

### 5.9 `align` 字段的实测覆盖情况

全仓库 `grep 'align: [012]'` 命中 10 处，**全部是 `align: 0`**（`render.rs:65`/`:88`/`:107` 三个生产者 + 7 处测试）。也就是说：
- `align = 1`（居中）与 `align = 2`（右对齐）的分支（`null.rs:460-461`、`:509-510`）**在仓库内没有任何生产者或测试走到**。

---

## 6. 测试清单（`crates/deer-gpu/tests/*.rs`）

行数按换行符计。运行命令：

```sh
cargo test --workspace                # 全仓（README.md:46）
cargo test -p deer-gpu                # 本 crate 的单元 + 集成测试
cargo test -p deer-gpu --test glyph_atlas          # 单个集成测试文件
cargo test -p deer-gpu --test text_raster -- --nocapture   # 看「跳过系统字体」的说明
```

> 依赖系统字体的用例在 `%WINDIR%\Fonts` 找不到 `consola.ttf`/`arial.ttf`/`segoeui.ttf` 时**明确跳过并打印原因**，不伪装通过（`crates/deer-gpu/src/measure.rs:205-216`、`crates/deer-gpu/tests/text_raster.rs:7-8`、`font_parse.rs:9-11`）。

| 文件 | 行数 | 守的是什么 | 关键测试（行号） |
|---|---|---|---|
| `crates/deer-gpu/tests/draw_list_and_cpu_backend.rs` | 255 | **绘制列表结构不变式 + CPU 像素正确性**（无 GPU 时的唯一验证手段，`:1-5`）。 | `clip_stack_must_balance`（`:25`）、`from_cmds_recomputes_balance`（`:37`）、`fill_rect_paints_exactly_its_area`（`:61`）、`clip_actually_clips`（`:82`）、`nested_clips_intersect`（`:99`）、`stroke_draws_only_the_border`（`:120`）、`round_rect_removes_corners`（`:135`）、`text_occupies_a_bounded_region`（`:151`）、`render_is_deterministic`（`:173`）、`unbalanced_clip_is_an_error_not_a_panic`（`:199`）、`cpu_backend_satisfies_the_hal_contract`（`:214`）、`frame_path_renders_and_can_be_read_back`（`:230`）、`cpu_framebuffer_helpers_work`（`:247`） |
| `crates/deer-gpu/tests/render_pipeline.rs` | 176 | **树 + 几何 → DrawList → 像素** 的渲染器语义（`:1-4`），并给出「后续 Vulkan 后端应当相同像素」的基准。 | `every_visible_node_gets_a_draw_command`（`:35`）、`disabled_button_uses_dim_color`（`:53`）、`container_without_padding_draws_no_box`（`:77`）、`button_text_is_centered_by_the_same_measure_as_layout`（`:96`）、`null_renderer_emits_one_hint_per_geometry_entry`（`:113`）、`end_to_end_pixels_are_not_a_flat_fill`（`:126`）、`render_is_deterministic_end_to_end`（`:155`） |
| `crates/deer-gpu/tests/glyph_atlas.rs` | 496 | **货架打包图集**：往返一致 + 两两不重叠 + padding 为 0、幂等、增高不移动旧槽位、空位图、明确失败、确定性；文件头还带**变异测试记录**（`:11-21`）。 | ⑦ `inserted_pixels_round_trip_and_slots_never_overlap`（`:112`）、⑧ `duplicate_insert_is_idempotent`（`:174`）、⑨ `growth_keeps_existing_slots_valid`（`:205`）、⑩ `blank_image_registers_without_using_space`（`:276`）、⑪ `oversized_glyphs_fail_loudly_and_coverage_stays_exact`（`:325`）、⑫ `same_insert_sequence_is_byte_identical`（`:430`）、边界 `zero_width_atlas_only_accepts_blank`（`:483`）；独立读取路径 `gather_from_coverage`（`:94`，专门抓「增高移位」） |
| `crates/deer-gpu/tests/text_raster.rs` | 585 | **光栅化验收**：手工构造轮廓的硬断言（机器无关）+ 系统字体端到端；含手算覆盖率参照（`:10-11`、`:164-171`）。 | `unit_square_is_fully_covered`（`:138`）、`half_pixel_shift_gives_antialiased_edges_and_conserves_area`（`:173`，手算 `coverage_sum = 25519`）、`nonzero_winding_punches_hole_when_orientations_differ`（`:215`）、`same_orientation_contours_do_not_punch_hole`（`:242`，证明不是 even-odd）、`y_axis_is_flipped_between_font_units_and_bitmap_rows`（`:271`）、`glyph_without_contours_is_blank_but_keeps_advance`（`:300`）、`degenerate_scale_returns_blank_without_panic`（`:318`）、`quad_circle_flattens_and_ink_ratio_is_plausible`（`:348`，期望 5/6）、`cubic_circle_flattens_to_a_real_circle`（`:399`，期望 π/4）、`rasterization_is_deterministic`（`:430`）、`system_font_glyphs_are_plausible`（`:458`，无字体则跳过） |
| `crates/deer-gpu/tests/text_measure.rs` | 409 | **真实度量与换行**：合成字体（手选数字）+ 系统字体真值对照；带变异测试记录（`:13-19`）。 | ① `advance_matches_hmtx_through_independent_path`（`:53`）、② `consola_visible_ascii_is_monospace_with_approx_comparison`（`:98`）、③ `text_width_is_ceil_of_advance_sum`（`:186`）、④ `wrap_respects_max_width_without_losing_characters`（`:211`）、⑤ `measure_trait_matches_inherent_api`（`:296`）、⑥ `unmapped_characters_fall_back_to_notdef`（`:332`）、⑥b `broken_hmtx_falls_back_to_half_em_without_panic`（`:363`）、`system_font_probe_returns_existing_file`（`:386`）、`synthetic_font_unitchoice_is_stable`（`:403`） |
| `crates/deer-gpu/tests/text_pixels.rs` | 303 | **真字形的像素级验收**：真字形 ≠ 占位格、渲染确定性、形状性质（窄字形/洞/下伸部/空格推进）；字体缺失则明确跳过（`:1-11`）。 | `real_glyphs_differ_from_placeholder_boxes`（`:81`）、`rendering_with_text_is_deterministic`（`:109`）、`narrow_glyph_ink_is_narrower_than_its_advance`（`:127`）、`glyph_with_a_counter_keeps_its_hole`（`:164`）、`descender_reaches_below_a_glyph_without_one`（`:185`）、`ink_x_position_is_pen_plus_left`（`:222`，注释还写了「把 `+ p.left` 改成 `- p.left` 必须变红」）、`space_advances_the_pen_without_ink`（`:252`）、`buttons_center_labels_using_real_metrics`（`:269`）、`missing_glyph_falls_back_to_notdef_and_keeps_width_consistent`（`:283`） |
| `crates/deer-gpu/tests/font_parse.rs` | 314 | **用什么字体做真值对照**：系统字体的精确参考值（独立 Node 脚本产出，`:1-5`），无字体则跳过。 | `parses_real_font_metrics_exactly`（`:39`）、`consola_metrics_match_independent_reference`（`:84`，参考 `unitsPerEm=2048 numGlyphs=3031 asc/desc/gap=1521/-527/350 …`，`:82`）、`both_cmap_formats_resolve_glyphs`（`:104`，consola format 4 / arial format 12）、`glyph_outline_matches_declared_bbox`（`:133`，两条独立路径交叉验证）、`curved_glyph_has_real_curves`（`:191`）、`unknown_char_returns_none_not_notdef`（`:229`）、`malformed_font_returns_error_not_panic`（`:252`）、`monospace_advances_are_uniform`（`:287`） |
| `crates/deer-gpu/tests/font_synthetic.rs` | 469 | **测试自己造一份最小合法 TTF**（`unitsPerEm=1000`、`.notdef`=500、空格=250、'A'=800 且轮廓为 `(200,200)-(700,700)` 正方形、长格式 `loca`、format 4 子表，`:8-21`），让解析器在任何机器上都有确定性对照组。 | `synthetic_font_parses_with_exact_values`（`:284`）、`synthetic_font_cmap_maps_chars_to_expected_glyphs`（`:299`）、`synthetic_square_glyph_has_exact_geometry`（`:314`）、`synthetic_blank_glyphs_are_empty_not_missing`（`:345`）、`synthetic_font_rejects_cff_variant`（`:360`）、`synthetic_ttc_container_is_handled`（`:376`）、`short_loca_format_is_supported`（`:439`）；构造器 `table_head/hhea/maxp/loca/glyf/hmtx/cmap`（`:60`/`:85`/`:107`/`:146`/`:160`/`:165`/`:178`）与 `build_font`（`:231`） |
| `crates/deer-gpu/tests/support/mod.rs` | 281 | 被 `text_measure.rs` 复用的**同源合成字体构造器**（集成测试各自是独立 crate，无法跨文件 `use`，`:3-10`）。 | `build_font`、`table_*`（`:54`/`:79`/`:101`/`:113`/`:134`/`:144`/`:148`/`:160`）、`dir_entry_offset`/`table_offset`（`:246` 前后、`:257`）、**`build_font_with_unreadable_hmtx`（`:269-276`，把 `hmtx` 目录项指到文件最后一字节、长度 0，用来构造 `advance` 的 `0.5em` 兜底分支）**、`visible_ascii`（`:279-281`） |

`crates/deer-gpu` 内部还有两处**单元测试**（`#[cfg(test)] mod tests`）：`src/glyph.rs:171-206`（越界安全、空字形保留 advance、槽位重叠对称性、`GlyphKey` 不吃 0 字号）与 `src/png.rs:115-169`（PNG 头/块结构、CRC32/Adler32 已知向量、zlib 头校验位、多 stored 块）。
`crates/deer-layout` 的测试在 `crates/deer-layout/tests/layout_invariants.rs`（T1..T14，见 2.1）。

---

## 7. 「想改 X 该动哪里」

> 每条都列「必须改的文件」+「必须同步更新的测试」。所有生产/消费点都用 grep 核过。

### 7.1 加一个 `DrawCmd` 变体（例：`DrawCmd::Line`）

1. **定义**：`crates/deer-gpu/src/draw.rs:64-85`（枚举）。
2. **必然编译失败的三个 exhaustive match（无 `_` 兜底，编译器会逐个指出来）**：
   - CPU 后端：`crates/deer-gpu/src/null.rs:317-353`；
   - GPU 几何翻译：`crates/deer-vk/src/gpu_geom.rs:142-194`（不实现就按现有惯例进 `out.unsupported`，`:91-92`、`:173-191` 有先例与措辞）；
   - 示例里的打印器：`crates/deer-gui/examples/draw_list.rs:38-54`。
3. **统计与平衡**：如果新命令参与裁剪，`draw.rs:112-119`（`push` 的 `clip_balance`）与 `:135-149`（`counts`）+ `:152-161`（`DrawCounts` 字段）要同步。
4. **生产者**（若某控件要发它）：`crates/deer-gpu/src/render.rs:42-110`。
5. **测试必须同步**：
   - `crates/deer-gpu/tests/draw_list_and_cpu_backend.rs`（新增像素级断言，参照 `fill_rect_paints_exactly_its_area:61`、`stroke_draws_only_the_border:120`）；
   - `crates/deer-vk/tests/gpu_geom_stream.rs`（顶点流/`unsupported` 断言）与 `crates/deer-vk/tests/gpu_geom_parity.rs`（CPU/GPU 逐字节对照语料，`:207` 起是命令维度的用例表）。
6. **文档**：`docs/features/draw-list.md:47`（「`DrawCmd` 的全部变体」一节）；若算新功能还要动 `FEATURES.md`（一致性由 `crates/deer-gui/tests/docs_consistency.rs` 强制：链接目标存在、`--example` 存在、✅ 行必须同时有指南与示例）。

### 7.2 改圆角判据（例：改成抗锯齿 / 改成像素中心）

1. **唯一实现**：`crates/deer-gpu/src/null.rs:400-419`（`inside_rounded`），调用点 `:391`、签名 `:387`。
   - 若改成带覆盖率输出，`fill` 的返回/调用形态（`:387-397`）与 `blend`/`blend_cov`（`:363-385`）都要动；`RectI` 的整数语义（`draw.rs:32-37`）会被削弱。
2. **GPU 必须同步**（否则 parity 立刻红）：`crates/deer-vk/src/gpu_geom.rs:158-162` 与 `radius_kind` 相关逻辑（注释在 `:159` 明确对齐 CPU 的 `radius.max(0)`）；着色器侧的圆角判据（`crates/deer-vk/src/spirv.rs` + `gpu_render.rs` 的管线）——具体哪支着色器函数承担圆角**未确认**（本次只读测绘未展开 `deer-vk`）。
3. **测试必须同步**：`crates/deer-gpu/tests/draw_list_and_cpu_backend.rs:135`（`round_rect_removes_corners`）；`crates/deer-vk/tests/gpu_geom_parity.rs:252-282`（radius 1/2/3/5 的对照语料）与 `crates/deer-vk/tests/gpu_vs_cpu.rs:177-180`（含 `round-huge`）；`crates/deer-gpu/tests/render_pipeline.rs:126`（端到端像素）。
4. 注意副作用：`FillRoundRect` 的 `radius` 语义在 `draw.rs:69` 的注释里写的是「半径实际由后端做圆角化；CPU 后端用简单掩码」——改判据要让这句话仍然为真。

### 7.3 改换行规则（`wrap`）

> **⚠️ 本节的行号与结论已按「剩余工作第 1 项」更新过**（多行文本 + 滚动容器落地后，
> `measure.rs` 的 `wrap` 改成委托、`layout.rs` 里加了 `wrap_greedy`/`scroll`，行号整体位移：
> 引用只给**函数名与文件**，不再逐个给行号）。

1. **唯一实现**：`deer_layout::layout::wrap_greedy`（`crates/deer-layout/src/layout.rs`）——
   按空格/制表切词、行首不留空白、单词超宽按字符硬切、`max_width <= 0` 或非有限 ⇒ 不换行、空串算 1 行、不丢字。
   两个实现都**委托**给它、只注入「宽度怎么算」：`ApproxMeasure::wrap`（近似）与
   `FontMeasure::wrap`（真实 advance，`crates/deer-gpu/src/measure.rs`）。
2. **与布局的耦合**（这条**已经变了**）：`ApproxMeasure::height` 现在是
   `wrap(..).len() * line_height`（**不再是** `(w / max_width).ceil()` 的宽度比例近似）——
   理由：度量高度与「画出来的行数」若是两套算法，就会出现「预留 2 行、画出来 3 行」。
   `deer-gpu/tests/text_measure.rs` 与 `deer-layout/tests/scroll_multiline.rs` 各有一条一致性断言。
3. **测试必须同步**：`crates/deer-gpu/tests/text_measure.rs`（`wrap_respects_max_width_without_losing_characters`，
   含「不丢字」「行首无空白」「单字符硬切例外」三条；`measure_trait_matches_inherent_api` 保证 trait 与固有 API 同源）；
   变异记录在 `text_measure.rs` 文件头。
4. **多行绘制**（`text` + `layout.wrap`）：行由**渲染器**展开成 N 条 `DrawCmd::Text`
   （`deer_gpu::render::text_lines`，两个渲染器共用），所以**后端仍然一个 `Text` 命令画一行**
   ——`null.rs::draw_text_real` 不需要懂换行。详见
   [`../features/scroll-and-multiline.md`](../features/scroll-and-multiline.md)。

### 7.4 加一种颜色格式（`TargetFormat`）

1. **定义**：`crates/deer-gpu/src/lib.rs:87-93`。
2. **必然失败/必须补齐的映射点**：
   - Vulkan 格式 ↔ HAL 格式：`crates/deer-vk/src/hal.rs:221-225`（`target_format_of`）与 `:255-265`（反向映射的断言）；
   - 适配器类型映射的同族位置（若同时加 `AdapterKind`）：`crates/deer-vk/src/device.rs:1346-1350`、`crates/deer-vk/src/lib.rs:92-96`；
   - 交换链选取格式：`crates/deer-vk/src/swapchain.rs`（`VK_FORMAT_*` 常量与偏好顺序）。
3. **CPU 后端**：`null.rs:192-194`（`CpuSwapchain::format` 恒 `Rgba8Unorm`）、`null.rs:144-151`（`create_swapchain` 忽略 `_format`）——CPU 路径的混合**永远在 RGBA8 存储值空间**做（`:370-385`），新增 sRGB 格式时要想清楚是否需要线性化。
4. **测试必须同步**：`crates/deer-vk/tests/vulkan_smoke.rs:77-81`（`TextureDesc` 用 `Rgba8Unorm`）、`crates/deer-vk/tests/swapchain_smoke.rs`（交换链格式选择）、以及 `hal.rs:255-265` 附近的内联断言；`crates/deer-gpu` 侧暂无 `TargetFormat` 的对照测试（只有 `null.rs` 的返回值）。

### 7.5 改图集装箱策略（图集打包：货架 → 装箱 / 加 LRU）

1. **实现**：`crates/deer-gpu/src/atlas.rs` 全文件；关键约束全在模块文档 `:3-45`：
   - 增高**只能追加新行**、不能移动已有槽位（`:276-281` 的 `ensure_height` + `:67` 的「`width` 构造后不变」）—— 任何「重排」策略都会让已发给渲染器的 `AtlasSlot` 失效（`:5-8`、`:271-275`）；
   - 每个字形右/下各留 1px padding 防渗色（`:50`、`:18-20`）；
   - 失败必须明确（超宽 / 超 `MAX_DIMENSION=8192` ⇒ `None`）（`:113-115`、`:147-149`、`:174-176`）；
   - 双份数据：`coverage`（GPU 上传的唯一真值）+ 每 key 的逐行紧致 `pixels`（只给 `get()` 借用，`:31-45`、`:192-210`）。
2. **加 LRU/淘汰**要先回答：`GlyphPlacement`（`crates/deer-gpu/src/text.rs:46-55`）与 `TextEngine::placements`（`:63`）里缓存的槽位如何失效 —— 目前 `TextEngine::glyph` 命中缓存即直接返回（`:152-154`），没有失效协议。现在**明确不做淘汰**（`text.rs:31`）。
3. **测试必须同步**：`crates/deer-gpu/tests/glyph_atlas.rs` 全部七条（尤其 ⑨ `:205` 的「增高后大图里槽位内容不变」与 ⑪ `:325` 的「槽位外全 0」）；示例 `crates/deer-gui/examples/glyph_atlas.rs:131-142`（利用率下界断言）。
4. 若改变打包导致「同一插入序列的槽位布局」变化：`glyph_atlas.rs:430`（逐字节确定性）会红，需要更新期望值而不是放松断言。

### 7.6 额外两条（成本低、但容易漏）

- **加一个 `Kind`（新控件类型）**：`crates/deer-layout/src/node.rs:21-32`（枚举）+ `:35`（`as_str`）+ `:45`（`parse`）+ `:57`（`is_container`）+ `:226`（`IdGen::reserve` 的 kind 列表，**漏了就破坏两路径 id 一致**）；`crates/deer-layout/src/scene.rs:143-145`（`KNOWN_ATTRS` 通常也要加）；`crates/deer-gpu/src/render.rs:42-110`（发命令）+ `crates/deer-layout/src/layout.rs:90-159`（固有尺寸）。测试：`crates/deer-layout/tests/layout_invariants.rs:48`（两条路径同构）、`:58`（自动 id 规则）、`crates/deer-gpu/tests/render_pipeline.rs:35`。
- **改布局分配语义（`grow` / 对齐 / 百分比）**：`crates/deer-layout/src/layout.rs:209-347`（尤其 I-7 的「分配尺寸 vs 可用空间」`:334-344` 与 `:279-300` 的「剩余量必须在 grow 之后重算」）。测试：`layout_invariants.rs:143`（T5 grow）、`:171`（T6 对齐）、`:195`（T7 stretch）、`:214`（T8 取整）、`:297`（T12 百分比）；回归守卫 `:319`（T13）、`:330`（T14）。

---

## 8. 已知边界 / 坑（代码注释或文档里**明确写的**）

### 8.1 `deer-layout`

| 边界 | 锚点 |
|---|---|
| 布局的 7 条不变量（纯函数 / 确定性 / 自底向上 / 像素取整 / 不假设拥有窗口 / 不越界 / **分配尺寸 ≠ 可用空间**）。I-7 是 deer-ui V0 的 B-2 缺陷的显式建模。 | `crates/deer-layout/src/layout.rs:3-12` |
| **根不撑满盒子**：宿主给的盒子是上限，根用自己的固有尺寸（除非根自己有显式尺寸）。 | `layout.rs:182-185` |
| 容器固有尺寸：**主轴 = sum(子)，交叉轴 = max(子)**；只把「显式像素」尺寸计入主轴，且**不能在交叉轴也累加**（否则两个 `w=36` 的按钮会让行算成 72）。 | `layout.rs:96-104`（注释）、实现 `:105-133` |
| 百分比**不在测量阶段解析**（需要父的实际宽度），留到排布阶段；`resolve` 只在排布用。 | `layout.rs:104`、`:73-78`、`:250`/`:311` |
| 容器的 `avail_*` 传的是**父的内容盒**（`inner_w`/`inner_h`），不是分配尺寸 —— 传错会让 `50%` 静默变成「已分配空间的一半」。 | `layout.rs:334-344` |
| 显式尺寸也**不许超过可用空间**（I-6）。 | `layout.rs:216-222` |
| 场景文件缩进**必须是 2 的倍数**（否则报错带行号）。 | `crates/deer-layout/src/scene.rs:72-78` |
| 场景**只能有一个根**；叶子节点**不能有子节点**（都报错）。 | `scene.rs:292-295`、`:309-315` |
| 场景属性白名单（未知属性直接报错）：`name/w/h/pad/gap/main/cross/grow/scroll/wrap/label/disabled`（`scroll`/`wrap` 是**裸开关**，带值会被拒绝；剩余工作第 1 项加的）。 | `scene.rs:143-145`、`:231-239` |
| 场景只支持 `k=v`、裸 `k`、`k="带 空格"`；`#` 注释要求前面是行首或空白（引号内不算注释）。 | `scene.rs:52`、`:94`、`:63` |
| 显式命名的节点**必须「占号」**（`IdGen::reserve`），否则自动 id 会撞上显式名（B-1 缺陷）。 | `crates/deer-layout/src/node.rs:203-207`、`:219-235` |
| 树是**纯数据**（不含回调）；事件用 `id` 关联。 | `node.rs:12-15` |
| 命令式 API 的建造顺序坑：**先设 layout/props，最后 `with_id`** —— 反过来会被整块赋值覆盖而丢 label。 | `crates/deer-layout/src/builder.rs:82-85`、`:165-167` |

### 8.2 `deer-gpu`（HAL 与绘制数据）

| 边界 | 锚点 |
|---|---|
| HAL 不依赖 `wgpu`/`ash`/`vulkano`；本层**不含** Vulkan/DX12/Metal 的 API 绑定。 | `crates/deer-gpu/src/lib.rs:3-5`、`:16-20` |
| 后端 `new`/初始化失败**不得 panic**，必须返回 `GpuError` 让上层回退。 | `lib.rs:52-53`、`crates/deer-gpu/src/error.rs:3-4` |
| 资源句柄用不透明 id（`u32`/`u64`），**不暴露后端类型**。 | `lib.rs:54` |
| `RawWindowHandle` 的生命周期约定：`window` 必须仍存活，后端不得比它活得更久。 | `lib.rs:107-109` |
| `PresentResult::OutOfDate` 是**正常路径**：调用方应 `resize` 后重试。 | `lib.rs:178`、`:185-186` |
| 三个 `TargetFormat` 注释里写明语义：`Bgra8Srgb`/`Rgba8Srgb` 是非线性 sRGB，`Rgba8Unorm` 是线性（截图/回读）。 | `lib.rs:88-92` |
| `Renderer` trait 声明为「唯一需要为新控件类型改动的地方」，**但其实没有实现者**（实际入口是 `render.rs:148` 的自由函数）。 | `lib.rs:189-195` vs `render.rs:148` |
| 绘制命令是**「结果」不是「控件」**：新增控件不需要动任何后端。 | `crates/deer-gpu/src/draw.rs:58-62` |
| `PushClip`/`PopClip` 必须配对；`clip_balance` 私有且**只有 `push()`/`from_cmds()` 会维护** —— 直接写 `cmds` 会绕过它。 | `draw.rs:91-92`、`:112-119`、`:129-132` |
| `FillRoundRect` 的圆角「实际由后端做圆角化；CPU 后端用简单掩码」（⇒ CPU 侧无抗锯齿）。 | `draw.rs:69`、`null.rs:399` |
| `Color` 是「线性空间不做转换的直通 RGBA」，alpha 是 `0.0–1.0` 浮点，RGB 是 `u8`。 | `draw.rs:5-12` |
| `packed()` 里 alpha 用**截断** `(self.a * 255.0) as u32`（与 `blend` 的 round 口径不同）。 | `draw.rs:25-27` |
| CPU 后端「没有真窗口可查尺寸」，尺寸必须由调用方显式给（`Extent` 或 `CpuFrame::set_extent`）。 | `null.rs:126-128`、`:203` |
| 无字库与有字库是**刻意的两条路**：没有字库时**不假装**能画字，有字库时**必须**画真字。 | `null.rs:259-266` |
| 圆角不做抗锯齿（同上）。 | `null.rs:399` |
| CPU HAL 的若干能力是**占位实现**：`create_texture` 只发递增 id（`:153-157`）、`upload_texture` 直接 `Ok(())`（`:159-166`）、`wait_idle` 空实现（`:179-181`）、`create_swapchain` 忽略窗口与格式（`:144-151`）。 | `null.rs:144-181` |
| `Frame::record` 只检查裁剪平衡并**克隆命令**，不做翻译。 | `null.rs:227-236` |
| 软件光栅化里没有 alpha 预乘、没有 sRGB 线性化：全部在存储值空间混合。 | `null.rs:370-385` |

### 8.3 文本 / 字形链

| 边界 | 锚点 |
|---|---|
| **`left` 通常 ≥ 0**：多数字体 `lsb` 为正 ⇒ `left = floor(lsb*scale)` 落在笔位置右侧；只有斜体/悬垂字形才为负。实测（consola.ttf @32px）：`'.'`=6、`'i'`=2、`'W'`=0。 | `crates/deer-gpu/src/glyph.rs:30-32` |
| 位图是**紧致**的（宽高正好包住墨迹），所以「位图尺寸」≠「advance」；`top` 可为负（下伸部）。 | `glyph.rs:19`、`:34-38` |
| `coverage` 是 8 位覆盖率（0..=255），**不是颜色**。 | `glyph.rs:17-18`、`:41-42` |
| 位图不变式 `coverage.len() == width * height`（`debug_assert`）；越界用 `coverage_at` 安全取（回 0）。 | `glyph.rs:23`、`:59-80`、`:87-93` |
| `GlyphKey.px_size` 是**取整后的字号** ⇒ 12.4px 与 12.6px 共用同一张位图（按字号分桶缓存）；`0` 视为 `1`。 | `glyph.rs:122-132`、`:135-141` |
| `AtlasSlot` 不变式：`x+w <= 图集宽`、`y+h <= 图集高`、任意两槽位不重叠、`get()` 长度恒为 `w*h`；空槽位永不重叠。 | `glyph.rs:143-146`、`:162-168` |
| 图集**只增不减**（没有 LRU）；长会话里字号种类多会持续增长。 | `crates/deer-gpu/src/text.rs:31`、`atlas.rs`（无淘汰代码） |
| 图集增高**只追加新行**，`width` 构造后不变 ⇒ 已发出的 `AtlasSlot` 仍然有效；`get()` 借用 `self`，所以活着时无法 `insert`（借用检查器拦住悬垂）。 | `atlas.rs:5-16`、`:66-69`、`:220-230`、`:271-281` |
| 字形右/下各 1px padding（`PADDING = 1`），padding 必须为 0，否则 GPU 双线性采样会渗色。 | `atlas.rs:18-20`、`:50` |
| `MAX_DIMENSION = 8192`（移动 GPU 常见 `maxTextureDimension2D` 下界）；超限**明确失败**，不静默截断。 | `atlas.rs:52-55`、`:173-176` |
| `GlyphAtlas::new(0)` 合法：任何非空字形都因「宽超限」失败，只有空位图能登记。 | `atlas.rs:86-89` |
| 光栅化**不做 hinting**（有实测依据：hinting-lite 无净收益）、**亚像素水平定位是 opt-in 路径**（默认 `rasterize` 仍整数落位、逐字节不变；opt-in 走 `rasterize_at`/`rasterize_char_at` + `split_subpixel_x` 的 1/4 相位档）、**不支持 CFF**、**不做精确曲线极值**（bbox 用控制点包围盒 ⇒ 位图可能比真实墨迹宽不到 1px）。 | `crates/deer-gpu/src/raster.rs`（模块文档的「诚实边界」与 opt-in API）、落位公式见 `raster.rs` 的整数落位路径 |
| 采样点正好落在轮廓边上时按环绕数算法的朝向约定判定（结果确定，但不保证与解析面积逐位一致）。 | `raster.rs:34-35`、`:309-314` |
| 退化输入一律给 `blank`（不 panic）：`ppem<=0`/NaN、`units_per_em==0`、非有限 scale、`!advance.is_finite()`、零宽高、单边 > `MAX_BITMAP_DIM=4096`。 | `raster.rs:106-144`、`:70` |
| `supersample` 是公开字段，误设 `u32::MAX` 会爆；内部夹到 `MAX_SUPERSAMPLE = 64`。展平深度上限 `MAX_FLATTEN_DEPTH = 16`、平坦度 `FLATNESS_TOLERANCE = 0.25`。 | `raster.rs:48-64`、`:193` |
| 度量回退链三级：① cmap 命中 → 真实 `hmtx` advance；② 未命中或字形读不出 → **glyph 0（`.notdef`）的 advance**（不用 0.6em 猜）；③ 连 glyph 0 都取不到 → `0.5 * font_size` 兜底，**绝不 panic**（合法字体上不可达，是防御分支）。 | `crates/deer-gpu/src/measure.rs:14-23`、`:95-108` |
| `descent` 取 `-descender`；坏字体把 descender 写成正数时会得到负值 —— **不做绝对值**（「读到的就是事实」）。 | `measure.rs:78-85` |
| `text_width` **向上取整**（`ceil`），与 `ApproxMeasure` 的整数友好约定一致，且 `text_width("") == 0.0`。 | `measure.rs:110-116` |
| `wrap`：按空格/制表切词（其它空白不切）、行首不留空白、单词超宽按字符硬切（单字符超宽时该行允许超宽，否则死循环）、`max_width<=0` 不换行、空串算 1 行、`wrap(text).concat()` 不丢字。 | `measure.rs:25-36`、`:124-179` |
| `Measure::height` 用 `style.line_height`（行距属于**样式**）而不是字体自带行高；`width` 忽略 `style.font_size`（字号由构造时给定）。 | `measure.rs:182-196` |
| `find_system_font` 只找 `consola.ttf` → `arial.ttf` → `segoeui.ttf`（`%WINDIR%\Fonts`，`WINDIR` 缺失退回 `C:\Windows\Fonts`）；**返回 `None` = 明确跳过**，不伪装成功。 | `measure.rs:198-216` |
| 文字引擎「诚实边界」：**本引擎**不做亚像素水平定位（±1px 抖动；光栅化层已有 opt-in 亚像素路径，**尚未接线**）、不做 hinting、不做字距/连字/替换（GSUB/GPOS 未实现）、**不做多字体回退**（缺字画 `.notdef` 豆腐块并计入 `missing_glyphs()`）、图集不淘汰。 | `crates/deer-gpu/src/text.rs` 的「诚实边界」 |
| `TextEngine::from_system_font` **找不到就报错**，绝不静默降级成占位字形。 | `text.rs:97-110` |
| `missing_glyphs()` 是**按调用次数累计**（不是缺字种类数），三种情况各 +1：cmap 未命中（仍回退 `.notdef`）、字形读不出、图集放不下。 | `text.rs:201-205`、`:144-149`、`:156-162`、`:165-171` |
| `TextEngine::text_width` 与 `FontMeasure::text_width` **只差末尾取整**（后者 `ceil()`，差 < 1px）：前者是「笔走过多远」，后者是「盒子要留多宽」。 | `text.rs:182-194` |
| 字形渲染的位图按整数像素落位；`left`/`top` 语义是「位图左边 = `round(pen)+left`」「位图顶边 = `baseline - top`」。 | `text.rs:50-53`、`null.rs:530-531` |
| **`align` 字段目前全仓库只用 `0`**（`render.rs:65`/`:88`/`:107`；grep `align: [012]` 无 `1`/`2`）⇒ 居中/右对齐分支无生产者也无测试。 | `draw.rs:77-78`、`null.rs:459-463`、`:508-512` |
| `Font` 支持范围表：支持 sfnt/`head`/`hhea`/`hmtx`/`maxp`/`cmap` 0/4/6/12/`loca` 短长/`glyf` 简单+复合；**CFF(`OTTO`) 直接报错**；竖排/变体/着色/GSUB/GPOS/字距/ hinting 未实现。 | `crates/deer-gpu/src/font.rs:7-23`、`:194-200` |
| 解析层**不改坐标**（只给 `units_per_em`），缩放到像素是调用方的事。 | `font.rs:25-30` |
| `glyph_index` 找不到返回 `None`（**不返回 0** —— 0 是 `.notdef`，含义不同）；规范允许用 0 表示缺失，所以 `g != 0` 才算命中。 | `font.rs:396-398`、`:421-426` |
| `ttcf` 容器取**第一个**字体（`face_index = 0`）；表目录偏移是「相对字体起始」，代码统一加 `base` 后返回绝对偏移（踩过 `head` 被读成 0 的坑）。 | `font.rs:172-188`、`:232-242`、`:369-376` |
| `hmtx` 紧凑规则：只有前 `num_h_metrics` 个字形有独立 advance，之后的字形**复用最后一个**。 | `font.rs:572-575`、`:585-588` |
| 复合字形嵌套 **> 8 层报错**（字体可能损坏）；点匹配模式（`ARGS_ARE_XY_VALUES` 未置位）**用 0 偏移兜住**（不支持精确点对齐）。 | `font.rs:703-707`、`:725-728`、`:734-737` |
| 坏字体必须返回错误**不许 panic**（字体是外部输入）。 | `font.rs` 全域 `GpuResult`；测试 `font_parse.rs:252` |
| 诊断开关：`DEER_FONT_DEBUG=1` 会往 stderr 打表偏移（临时诊断，不是稳定接口）。 | `font.rs:249-257`、`:516-520`、`:578-584` |
| PNG 用 **zlib stored（未压缩）deflate**：文件比真实压缩大（约等于原始像素），换取零依赖与实现简单。 | `crates/deer-gpu/src/png.rs:3-9`、`:68-90` |
| `encode_rgba` 要求 `pixels.len() == w*h*4` 且宽高 > 0，否则返回 `Err(String)`。 | `png.rs:14-24` |

### 8.4 跨 crate 的已知限制（本次顺带核到的）

| 边界 | 锚点 |
|---|---|
| GPU 侧**文本尚未实现**：`DrawCmd::Text` 被记进 `unsupported`，且**无条件登记**（哪怕空串/零面积/裁剪为空这些 CPU 一个像素都不画的情形也报错 ⇒ 假阳性），本轮已裁定 defer。 | `crates/deer-vk/src/gpu_geom.rs:173-191`、`:91-92` |
| GPU 侧裁剪**不用 scissor**，在 CPU 侧算交集；算法被要求与 `null.rs::soft_rasterize_with` **逐字一致**。 | `crates/deer-vk/src/gpu_geom.rs:269`、`:130-154` |
| CPU/GPU 的 0 尺寸约定一致：`Extent{0,0}` 等价 1×1。 | `crates/deer-vk/tests/gpu_geom_parity.rs:245`、`gpu_vs_cpu.rs:388-393` |
| 「不透明逐字节一致 / 半透明 ≤ 1 LSB」这条验收线的**具体断言与阈值来源**在 `crates/deer-vk/tests/{gpu_geom_parity,gpu_vs_cpu}.rs` 里（导览第 7 讲讲它）—— **本次未展开核对**，见下节。 | `crates/deer-vk/tests/gpu_geom_parity.rs:168-170`、`gpu_vs_cpu.rs:114` |

---

## 9. 未确认（读不到 / 没查的，明确登记）

1. **测试当前是否全绿**：本次是只读测绘，**没有运行任何 cargo 命令**，所以 `cargo test --workspace` 的通过数未验证（README 只说「数量见输出」`README.md:46`）。
2. **「半透明 ≤ 1 LSB」这条 parity 线的确切阈值与断言写法**：只看到 `crates/deer-vk/tests/gpu_vs_cpu.rs:114` 的 `compare(..., max_allowed)` 签名与调用点，未读该文件函数体，也未确认 `gpu_geom_parity.rs` 里「逐字节」的适用范围。
3. **`Framebuffer::clear` 用截断、`blend_cov` 用 round 这一口径差异是否有测试钉住**：未找到对应用例（`count_color` 也用截断，`null.rs:67`）。
4. **`stroke` 四角像素被重复 blend 这件事是否被任何测试专门覆盖**：只找到 CPU↔GPU parity 语料（`crates/deer-vk/tests/gpu_vs_cpu.rs:185-191`）间接覆盖，未找到 CPU 侧的独立断言。
5. **超大 `radius`（> 半宽/半高）在 CPU 侧的行为是否单独钉住**：只看到 parity 语料里的 `round-huge`（`crates/deer-vk/tests/gpu_vs_cpu.rs:180`）。
6. **`deer-vk` 里承担圆角/描边判据的着色器函数具体位置**：本次范围是 `deer-layout`+`deer-gpu`，未展开 `crates/deer-vk/src/spirv.rs` 与 `gpu_render.rs` 的着色器源码（第 7.2 条因此写成「未确认」）。
7. **`docs/features/*.md` 中哪些具体章节会因新增 `DrawCmd` 变体而过期**：只确认了 `docs/features/draw-list.md:47` 有「`DrawCmd` 的全部变体」小节，以及一致性检查的范围（`crates/deer-gui/tests/docs_consistency.rs`：链接目标存在、`--example` 存在、✅ 行必须同时有指南+示例），**未逐篇核对内容是否与代码同步**。
8. **`deer-gpu` 的 HAL trait 在 `deer-window`／`deer-gui` 窗口路径上的完整消费清单**：只核到 `deer-vk` 的实现与示例的 `Backend` 调用；`create_texture`/`upload_texture` 在真实窗口路径上是否被调用未逐一确认。
9. **若干「无仓库内调用点」的公开 API 是否真的无人用**：`Node::disabled`（`node.rs:166`）、`Node::push`（`:171`）、`Builder::root_id`（`builder.rs:186`）、`builder::props`（`:249`）、`Color::packed`（`draw.rs:25`）、`DrawList::is_empty`（`draw.rs:125`）、`CpuDevice::render_to_framebuffer`（`null.rs:129`）、`Font::{table_range,table_directory,face_offset}`（`font.rs:352`/`:360`/`:365`）、`TextEngine::{font,font_size,set_font_size}`（`text.rs:112`/`:116`/`:121`）、`metrics::*` 常量对外、`Frame`/`Swapchain`/`Renderer` trait 的非 `deer-vk`/`null` 消费点 —— 这些经 grep 在**本仓库**内无消费者，但这只说明仓库内没人用，不能排除它们是给外部使用者准备的公开接口。
10. **`crates/deer-layout/src/lib.rs:37` 的 `CORE_VERSION = "0.0.0"` 是否与 workspace 版本号有校验关系**：未找到任何引用点。
