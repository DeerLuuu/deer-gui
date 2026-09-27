//! 离屏 **GPU 几何渲染器**（M3a-T3）：`DrawList` → 顶点流 → 顶点缓冲 → GPU 光栅化 → 回读像素。
//!
//! 这是 M3a 的终点：把「非文本绘制命令」真的交给 Vulkan 画出来，并让结果**与 CPU 后端
//! 逐像素可比**（对照测试见 `tests/gpu_vs_cpu.rs`）。三个模块的分工：
//!
//! | 层 | 模块 | 职责 |
//! |---|---|---|
//! | 翻译（纯逻辑） | [`crate::gpu_geom`] | `DrawList` → [`GpuVertex`] 流 + CPU 侧裁剪 |
//! | 着色器 | [`crate::spirv`] | 透传属性 + 按顶点属性上的 `rect`/`radius_kind` 逐像素判定 |
//! | **本模块** | `gpu_render` | 顶点缓冲、静态管线、离屏图像、命令录制、回读 |
//!
//! ## 一帧的流程
//!
//! ```text
//!  ① 入口检查：裁剪栈平衡（与 CPU 同一判据）；`unsupported` 非空 ⇒ Unsupported
//!  ② gpu_geom::build_stream ⇒ 顶点流（CPU 侧已裁剪）
//!  ③ 顶点写进 HOST_VISIBLE 顶点缓冲（每帧重传；容量不足时重建）
//!  ④ 录制：beginRenderPass(CLEAR) → bindPipeline → bindVertexBuffers → draw(vertex_count)
//!          → endRenderPass → 屏障(TRANSFER_SRC) → copyImageToBuffer
//!  ⑤ vkQueueSubmit + 等栅栏（有限超时）
//!  ⑥ map 暂存缓冲读回 RGBA8
//! ```
//!
//! ## 为什么不复用 `offscreen.rs`
//!
//! 它做的是同一件事，但有两点不对：
//! 1. 它在录制时调 `vkCmdSetViewport`/`vkCmdSetScissor`（它的管线是**动态** viewport）；
//!    本模块用**静态** viewport/scissor —— 实测动态版在本机 Intel 驱动上画不出任何像素，
//!    而对着静态状态的管线发动态设置命令会触发校验层报错（`tests/vbo_probe.rs` 有实测记录）；
//! 2. 它没有 `vkCmdBindVertexBuffers`（M2a 的顶点来自着色器常量表）。
//!
//! 所以本模块自己持有资源。**资源全部包成 [`VkObject`]**（析构顺序由字段顺序保证：
//! 对象在前、`VkDevice` 在最后 ⇒ 设备最后销毁）。
//!
//! ## 与 CPU 逐像素可比的三条前提
//!
//! - **颜色格式是 `R8G8B8A8_UNORM`（不是 `_SRGB`）**：CPU 参考实现不做 gamma 转换，
//!   用 SRGB 格式会让 GPU 多一次编码 ⇒ 两边永远对不上；
//! - **alpha 混合按 `src-alpha / one-minus-src-alpha`**（[`crate::device`] 里既有的管线状态），
//!   与 `null.rs::blend_cov`（覆盖率 = 1）同式；
//! - **不预乘**：顶点颜色就是 `Color` 的原值（见 [`crate::gpu_geom`] 的颜色约定）。
//!
//! ## 同步（T3 review I1/I2）
//!
//! ### ① 顶点缓冲的 host 写入 → `vkCmdDraw` 读取：**依赖没有被显式表达**
//!
//! ⚠️ **定性（fix round 2 收紧；依据 Vulkan §7.9 Host Write Ordering Guarantees）**：
//! `vkQueueSubmit` 对 happened-before 的 host 写入**已经隐式建立 HOST → ALL_COMMANDS 依赖**，
//! 因此**像素一直是对的、加屏障前后也不会变**。这不是「本次观察到过错误像素」的缺陷，而是
//! **依赖没有被写出来** —— 一旦将来出现下列任一改动，它会**静默**出错（驱动不报错、校验层也不查）：
//! - 主机写入挪到 `vkQueueSubmit` **之后**（例如「先提交再改缓冲」这类优化）；
//! - 顶点缓冲改用**非相干**的 `HOST_VISIBLE` 内存；
//! - 数据来自**另一个队列**（跨队列没有隐式的执行顺序保证）。
//!
//! 所以这里显式发一条 `VkBufferMemoryBarrier` 把它钉住（参数来自纯函数
//! [`vertex_buffer_barrier_params`]，有单元测试钉确切常量；发射次数也有断言，见
//! [`GpuGeometryRenderer::host_to_vertex_barrier_count`]）。
//!
//! 另一个**确定**的事实：这类内存域依赖**校验层不查**（它做对象/参数/布局类校验，
//! 不做通用同步验证）—— 所以「`DEER_VK_VALIDATION=1` 零消息」**不能**当作同步正确的证据。
//!
//! 关于 flush：**只**选 `HOST_COHERENT` 的内存（`create_host_buffer` 的 `required` 位里带着它，
//! 拿不到就报 `Unsupported`；`pick_memory_type` 的单元测试把「绝不用非相干内存」钉住），
//! 所以不需要 `vkFlushMappedMemoryRanges`。顺便说明为什么**不能**顺手加上它：`ffi_dev` 里没有
//! `vkFlushMappedMemoryRanges` 符号，而 `ffi_dev.rs` 不在本任务的允许改动清单里。
//! **将来若放开「必须相干」这个约束，必须同时补上 flush**，否则主机写入对设备不可见。
//!
//! 读回方向（`copyImageToBuffer` → 主机 `map`）靠**栅栏**保证：栅栏信号使设备写入对主机可见
//! （相干内存下不需要 invalidate），我们等到栅栏才 map。
//!
//! ### ② 栅栏等待失败（超时）**不等于**提交完成 —— 之后不许复用任何东西（**真缺陷**）
//!
//! 这条与 I1 性质不同：`vkWaitForFences` 超时只说明「还没等到」，此时命令缓冲可能仍在执行、
//! 顶点缓冲可能仍被读，于是下一帧的 `vkResetCommandBuffer` / 重写顶点缓冲 / `vkResetFences`
//! 都是**未定义行为**（驱动不一定报错）。所以超时/失败后把渲染器置为 [`SubmitState::Broken`]，
//! 之后**任何** `render` 都直接报错，直到调用方丢弃并重建。
//! **守卫放在会做破坏性操作的地方本身**（不只是 `render` 开头）—— 这样即便调用方漏写 `?`，
//! 也不会真的去碰那些资源；判定逻辑有单元测试，见文件末尾 `tests`。

use std::ffi::c_void;

use deer_gpu::{Color, DrawCmd, DrawList, Extent, GpuError, GpuResult, RectI, TextEngine};

use crate::device::{
    vk_result_name, DescriptorPool, DescriptorSet, DescriptorSetLayout, DeviceFns, Pipeline,
    PipelineLayout, RenderPass, Sampler, ShaderModule, Texture, VertexAttr, VkDevice,
};
use crate::ffi;
use crate::ffi_dev as vk;
use crate::gpu_geom::{self, GpuVertex};
use crate::gpu_text::{self, TextVertex};
use crate::spirv;

/// `VK_FORMAT_R32_SFLOAT`（单个 `float`）——`radius_kind` 用它。
///
/// **为什么在这里定义**：`ffi_dev` 目前只声明了 `R32G32_SFLOAT` / `R32G32B32_SFLOAT` /
/// `R32G32B32A32_SFLOAT`（`vbo_probe.rs` 只需要 vec2）。规范里 `VK_FORMAT_R32_SFLOAT = 100`。
/// 放在本模块而不是随手写 100：名字带来源，且只在这一处出现。
const VK_FORMAT_R32_SFLOAT: i32 = 100;

/// `VK_PIPELINE_STAGE_HOST_BIT`（= `0x0000_4000 = 1 << 14`）。
///
/// 与上面同一个理由：`ffi_dev` 里只有 `TOP_OF_PIPE` / `TRANSFER` / `COLOR_ATTACHMENT_OUTPUT` /
/// `BOTTOM_OF_PIPE` / `ALL_COMMANDS` —— 主机侧同步（HOST / VERTEX_INPUT 与下面两个 access 位）
/// 是 M3a-T3 第一次需要，而 `ffi_dev.rs` 不在允许改动清单里。
const VK_PIPELINE_STAGE_HOST_BIT: u32 = 1 << 14;
/// `VK_PIPELINE_STAGE_VERTEX_INPUT_BIT`（= `0x0000_0004 = 1 << 2`）。
///
/// ⚠️ **不是 `1 << 5`** —— `1 << 5`（`0x20`）是 `VK_PIPELINE_STAGE_TESSELLATION_EVALUATION_SHADER_BIT`。
/// 这一点是校验层替我们抓到的（fix round 1 首次跑校验：`dstStageMask` 含细分求值阶段而设备
/// 没开 `tessellationShader` ⇒ `VUID-vkCmdPipelineBarrier-dstStageMask-04091`）。
/// 症状很隐蔽：屏障**照样被接受**，只是它建立的依赖根本不覆盖顶点取数 ——
/// 「加了屏障但没用」比「没加屏障」更难发现。
const VK_PIPELINE_STAGE_VERTEX_INPUT_BIT: u32 = 1 << 2;
/// `VK_ACCESS_HOST_WRITE_BIT`（= `0x0000_4000 = 1 << 14`）。
const VK_ACCESS_HOST_WRITE_BIT: u32 = 1 << 14;
/// `VK_ACCESS_VERTEX_ATTRIBUTE_READ_BIT`（= `0x0000_0004 = 1 << 2`）。
///
/// 同样**不是** `1 << 5`（那是 `VK_ACCESS_SHADER_READ_BIT`）。注意它与上面那个阶段位
/// **数值相同但属于不同枚举** —— 这不是笔误。
const VK_ACCESS_VERTEX_ATTRIBUTE_READ_BIT: u32 = 1 << 2;

/// 颜色附件的格式：**线性 UNORM**（见模块文档「三条前提」）。
const COLOR_FORMAT: i32 = vk::VK_FORMAT_R8G8B8A8_UNORM;

/// 等栅栏的超时（1 秒）：驱动出问题时**宁可失败，也不要永久挂住**（比失败更难排查）。
const TIMEOUT_NS: u64 = 1_000_000_000;

/// 顶点缓冲的最小分配（即使一帧只有 6 个顶点也按 4 KiB 分配，避免每帧重建）。
const MIN_VERTEX_BYTES: u64 = 4096;

/// 顶点属性表：`location` 与**顶点偏移**都由 [`GpuVertex`] 的锁布局算出（`offset_of!`），
/// 所以它不可能与 `gpu_geom` 的 `#[repr(C)]` 布局漂移。
///
/// `tests/gpu_vs_cpu.rs::vertex_layout_matches_the_hand_written_attribute_offsets`
/// 另外钉住**字面数字**（stride 44 / 0 / 8 / 24 / 28）—— 布局若被改动，两个地方都会红。
fn vertex_attrs() -> [VertexAttr; 4] {
    [
        VertexAttr {
            location: 0,
            format: vk::VK_FORMAT_R32G32_SFLOAT,
            offset: std::mem::offset_of!(GpuVertex, pos) as u32,
        },
        VertexAttr {
            location: 1,
            format: vk::VK_FORMAT_R32G32B32A32_SFLOAT,
            offset: std::mem::offset_of!(GpuVertex, rect) as u32,
        },
        VertexAttr {
            location: 2,
            format: VK_FORMAT_R32_SFLOAT,
            offset: std::mem::offset_of!(GpuVertex, radius_kind) as u32,
        },
        VertexAttr {
            location: 3,
            format: vk::VK_FORMAT_R32G32B32A32_SFLOAT,
            offset: std::mem::offset_of!(GpuVertex, color) as u32,
        },
    ]
}

/// 文本管线的顶点属性表：`TextVertex`（**stride 32**：`pos` 0 / `uv` 8 / `color` 16）。
///
/// 与 `spirv::vertex_shader_text` 的 `location 0/1/2` 逐字段对应；偏移同样用 `offset_of!`
/// 取（结构上不可能与 `gpu_text` 的 `#[repr(C)]` 布局漂移）。
fn text_attrs() -> [VertexAttr; 3] {
    [
        VertexAttr {
            location: 0,
            format: vk::VK_FORMAT_R32G32_SFLOAT,
            offset: std::mem::offset_of!(crate::gpu_text::TextVertex, pos) as u32,
        },
        VertexAttr {
            location: 1,
            format: vk::VK_FORMAT_R32G32_SFLOAT,
            offset: std::mem::offset_of!(crate::gpu_text::TextVertex, uv) as u32,
        },
        VertexAttr {
            location: 2,
            format: vk::VK_FORMAT_R32G32B32A32_SFLOAT,
            offset: std::mem::offset_of!(crate::gpu_text::TextVertex, color) as u32,
        },
    ]
}

/// 一条绘制段属于哪条管线。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PipelineKind {
    Shape,
    Text,
}

/// 一帧里的一段绘制：**顺序即 z 序**。
///
/// `first`/`count` 是**各自顶点缓冲内**的区间（形状与文本各有一块缓冲）——
/// 两条管线不共用顶点布局，所以用「段」而不是「同一条 `vkCmdDraw` 的偏移」来表达顺序。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DrawCall {
    kind: PipelineKind,
    first: u32,
    count: u32,
}

/// 文本管线的全部资源（渲染器以 `Option` 持有：不调 [`GpuGeometryRenderer::with_text`]
/// 就没有它，`Text` 命令保持 M3a 的 `Unsupported` 行为）。
///
/// ## 字段顺序（**不要重排**）
///
/// Rust **按声明顺序析构**（先声明的先 drop）。这里的契约是
/// **`DescriptorSet` 必须先于 `DescriptorPool` 释放**（集是从池里分配的，它的 `Drop` 会调
/// `vkFreeDescriptorSets(device, pool, ..)`）⇒ **`pool` 必须声明在 `set` 之后**。
/// 这与 `device.rs::allocate_descriptor_set` 的类型文档一致（那份文档是对的）。
///
/// ## 为什么允许 `dead_code`
///
/// `vs` / `fs` / `set_layout` / `pool` **从不被读取** —— 它们存在只为**所有权**：
/// 着色器模块与描述符集布局必须在管线存活期间有效，池必须在集释放之后才销毁。
/// 删掉任何一个都会让对象提前销毁。`allow(dead_code)` 是这里的正确表达。
#[allow(dead_code)]
struct TextResources {
    engine: TextEngine,
    pipeline: Pipeline,
    layout: PipelineLayout,
    vs: ShaderModule,
    fs: ShaderModule,
    set_layout: DescriptorSetLayout,
    /// 描述符集（**必须在 `pool` 之前声明** ⇒ 先于池析构，见上面的说明）。
    set: DescriptorSet,
    pool: DescriptorPool,
    sampler: Sampler,
    /// 文本顶点缓冲（**独立于形状的**：两者顶点布局不同，不能共用）。
    vertex: Option<VertexBuffer>,
    /// 当前已上传的图集纹理（`None` = 还没传过）。
    texture: Option<Texture>,
    /// 已上传图集的指纹 `(宽, 高, 已光栅化字形数)`：三者任一变化就重传。
    ///
    /// 依据：`GlyphAtlas` 只**追加/增高**、不淘汰，内容只在「新字形入图集」时改变，
    /// 而那只会让 `rasterized_glyphs()` 增加 ⇒ 这个三元组是充分的。
    uploaded: Option<(u32, u32, usize)>,
    /// 上一帧被跳过的文本命令数（诊断，见 [`GpuGeometryRenderer::text_skipped`]）。
    skipped: usize,
}

/// 把「当前生效的裁剪栈 + 这一条命令」组成一个临时 `DrawList`。
///
/// ## 为什么逐条命令，而不是整份列表一次
///
/// 形状与文本走**两条独立管线**，而 z 序要求它们按 `DrawList` 的原顺序交错绘制
/// ⇒ 必须逐条命令决定「这条进哪条管线」。两条翻译层（`gpu_geom` / `gpu_text`）的公开入口
/// 都是「整个 `DrawList`」，所以我们把**生效的 `PushClip` 序列原样重放**：
/// 翻译层内部算的是 `full ∩ r1 ∩ r2 …`，与「完整列表」时**逐字相同**
/// （同一个初始全画布 + 同一顺序的求交），裁剪语义不会因为拆分而改变。
///
/// 代价是每条命令一次小分配。GUI 一帧的命令数在几十~几百量级，可忽略；
/// 真正的批处理优化（合并段、减少 draw call）属后续任务。
fn single_command_in_clip(active_clip: &[RectI], cmd: &DrawCmd) -> DrawList {
    let mut l = DrawList::new();
    for r in active_clip {
        l.push(DrawCmd::PushClip { rect: *r });
    }
    l.push(cmd.clone());
    l
}

/// 把「新一段顶点」的偏移与数量转成 `u32`（`vkCmdDraw` 的参数类型）。
///
/// 抽成纯函数的理由（review M3）：循环体里**不能**再用 `?` 早退 —— 那时 `self.text` 已被
/// 借出，早退会把它丢成 `None`。改成「返回 `Result`，由调用方记进 `fatal`」，循环后统一
/// 「还原资源 + 返回错误」。顺带也让这段溢出判断可以被单元测试直接喂边界值。
fn vertex_range(base: usize, len: usize) -> GpuResult<(u32, u32)> {
    let first = u32::try_from(base).map_err(|_| {
        GpuError::Unsupported(format!("顶点偏移 {base} 超出 u32（一帧画不了这么多顶点）"))
    })?;
    let count = u32::try_from(len).map_err(|_| {
        GpuError::Unsupported(format!("顶点数 {len} 超出 u32（一帧画不了这么多顶点）"))
    })?;
    Ok((first, count))
}

/// 把翻译层报出的「未支持」清单并进帧级错误状态（**两条管线共用**）。
///
/// ## 为什么抽出这个函数（review M1）
///
/// 形状路径今天**不可达**（`gpu_geom` 什么命令都翻译得了），而「不可达的分支」最容易被
/// 后人删掉或写错却没有任何测试发现。但它在**将来**一定会用到：`gpu_geom` 只要新增一条
/// 尚未支持的命令，落到 `other` 分支的就是「静默少画」——比报错糟得多。
/// 抽成纯函数后可以用合成消息直接钉住行为（见本模块单测），不必等真实分支出现。
fn absorb_unsupported(acc: &mut Vec<String>, from: &[String]) {
    acc.extend_from_slice(from);
}

/// 帧末把收集到的「未支持」清单变成结果：**非空 ⇒ `Unsupported`**（与 M3a 同一策略）。
///
/// 与 [`absorb_unsupported`] 配对：一个负责收集、一个负责在**所有**路径都结束时统一报错
/// （包括循环里 `break` 出来的路径 —— 见 `render` 的 M3 说明）。
fn unsupported_outcome(messages: &[String]) -> GpuResult<()> {
    if messages.is_empty() {
        Ok(())
    } else {
        Err(GpuError::Unsupported(messages.join("；")))
    }
}

/// 对象销毁/内存释放的函数形态。
///
/// 本项目里所有 `vk::*Handle` 都是 `*mut c_void` 的别名、所有 destroy/free 都是
/// `(DeviceHandle, 句柄, 分配器)` 且不返回错误码 —— 所以**一个**包装类型就够
/// （`offscreen.rs` 里为 6 种对象各写一遍 `Drop`，这里等价但少 100 行样板；
/// 代价是类型上区分不了句柄种类，用[`wrap_create`] 的 `what` 参数在报错里补回来）。
type DestroyFn = unsafe extern "system" fn(vk::DeviceHandle, *mut c_void, *const c_void);

/// 顶点缓冲的 host→vertex 依赖所用的四个掩码。
///
/// 抽成**纯函数 + 具名结构**的理由（fix round 2 / R1）：reviewer 的变异证明「代码语义对了」
/// 不等于「**可回归**」—— 把 `dstStage` 写成 `1 << 5`、或把整段屏障删掉，16 个测试靶**全绿**。
/// 参数一旦由纯函数产出，就能用单元测试钉**确切常量**
/// （本项目既有做法：`hal.rs` 的 `present_result_of()` 就是为补同类覆盖漏洞抽出来的）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct BarrierParams {
    src_stage: u32,
    dst_stage: u32,
    src_access: u32,
    dst_access: u32,
}

/// 「主机写顶点缓冲 → GPU 顶点取数」这条依赖的参数：
/// `HOST` / `HOST_WRITE` → `VERTEX_INPUT` / `VERTEX_ATTRIBUTE_READ`。
///
/// ⚠️ **`VERTEX_INPUT` 是 `1 << 2`（`0x4`），不是 `1 << 5`** —— `1 << 5` 是
/// `VK_PIPELINE_STAGE_TESSELLATION_EVALUATION_SHADER_BIT`。写错时校验层会报
/// `VUID-vkCmdPipelineBarrier-dstStageMask-04091`（本设备没开细分），但它**照样接受**这条屏障 ——
/// 只是它建立的依赖不覆盖顶点取数（「加了屏障但没用」，比不加更难发现）。
/// 单元测试 `vertex_barrier_params_pin_the_exact_masks` 钉住这四个值。
fn vertex_buffer_barrier_params() -> BarrierParams {
    BarrierParams {
        src_stage: VK_PIPELINE_STAGE_HOST_BIT,
        dst_stage: VK_PIPELINE_STAGE_VERTEX_INPUT_BIT,
        src_access: VK_ACCESS_HOST_WRITE_BIT,
        dst_access: VK_ACCESS_VERTEX_ATTRIBUTE_READ_BIT,
    }
}

/// 一个「有句柄 + 有销毁函数」的 Vulkan 对象：`Drop` 时销毁。
///
/// 不带 `what` 字段：对象名只在**创建失败**时要紧（那时对象还不存在），所以它是
/// [`wrap_create`] 的参数；留成字段没人读只会招来 `dead_code`。
struct VkObject {
    handle: *mut c_void,
    device: vk::DeviceHandle,
    destroy: DestroyFn,
}

impl VkObject {
    fn handle(&self) -> *mut c_void {
        self.handle
    }
}

impl Drop for VkObject {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            // SAFETY: 句柄由本模块在本设备上创建且尚未销毁；`destroy` 是对应的销毁函数；
            // 设备比本对象活得久（`GpuGeometryRenderer` 里 `device` 声明在最后 ⇒ 最后析构）。
            unsafe { (self.destroy)(self.device, self.handle, std::ptr::null()) };
            self.handle = std::ptr::null_mut();
        }
    }
}

/// 顶点缓冲 + 它绑定的内存。
///
/// **字段顺序 = 析构顺序**：`buffer` 在前 ⇒ 先 `vkDestroyBuffer`、再 `vkFreeMemory`
/// （内存不能先于绑定它的缓冲消失）。
struct VertexBuffer {
    buffer: VkObject,
    memory: VkObject,
    capacity: u64,
}

/// 提交同步状态（**纯逻辑** ⇒ 可以在无 GPU 的单元测试里覆盖，见文件末尾 `tests`）。
///
/// 存在理由（T3 review I2）：`vkWaitForFences` 失败（超时）**不等于**提交完成。
/// 旧实现直接把错误往上抛，但渲染器**状态不变** —— 调用方（或同一次 `render` 的重试）
/// 会接着 `vkResetCommandBuffer` / 重写顶点缓冲 / `vkResetFences`，而这些东西可能**仍在
/// 被 GPU 使用**。那是未定义行为，且驱动往往不报错（症状是偶尔的垃圾像素或挂死）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum SubmitState {
    /// 没有在飞的提交（上次已确认完成，或还没提交过）—— 可以复用命令缓冲与顶点缓冲。
    #[default]
    Idle,
    /// 已提交、正在等栅栏 —— 只允许走「等 → resolve」这一条路。
    InFlight,
    /// 等待**失败**（超时等）：无法证明 GPU 已不再访问这些资源 ⇒ **永久不可复用**。
    Broken,
}

impl SubmitState {
    /// 复用任何资源之前的检查。
    fn ensure_reusable(self) -> GpuResult<()> {
        match self {
            SubmitState::Broken => Err(GpuError::Unsupported(
                "上一次提交没有确认完成（栅栏等待失败/超时）：命令缓冲与顶点缓冲里的内容\
                 可能仍被 GPU 使用，本渲染器已不可复用 —— 请丢弃它并重建"
                    .to_string(),
            )),
            SubmitState::Idle | SubmitState::InFlight => Ok(()),
        }
    }

    /// 提交前的状态迁移。
    fn begin(&mut self) {
        *self = SubmitState::InFlight;
    }

    /// 等栅栏之后的状态迁移：成功 ⇒ 可复用；失败 ⇒ **Broken**（这是本条修复的核心）。
    fn resolve(&mut self, wait_rc: i32) -> GpuResult<()> {
        if wait_rc == ffi::VK_SUCCESS {
            *self = SubmitState::Idle;
            return Ok(());
        }
        *self = SubmitState::Broken;
        Err(GpuError::Driver {
            code: wait_rc,
            message: format!(
                "vkWaitForFences 失败（{} ⇒ 很可能是超时，即 GPU 没在预期时间内做完）。\
                 提交状态未知 ⇒ 渲染器已被标记为不可复用（见 SubmitState）",
                vk_result_name(wait_rc)
            ),
        })
    }

    /// 提交阶段出现别的错误时：**同样**不敢假设资源空闲。
    fn mark_broken(&mut self) {
        *self = SubmitState::Broken;
    }
}

/// 已映射内存的 RAII 守卫：**无论从哪条路提前返回**都会 `unmap`。
///
/// 存在理由（T3 review F9）：`vkMapMemory` 成功之后的任何提前返回（例如空指针检查、拷贝失败）
/// 都会漏掉 `vkUnmapMemory` —— 映射泄漏不会立刻报错，但下一次 `vkMapMemory` 会失败
/// （VUID-vkMapMemory-memory-00678 之类），症状离原因很远。
struct Mapped<'a> {
    fns: &'a DeviceFns,
    device: vk::DeviceHandle,
    memory: vk::DeviceMemoryHandle,
    ptr: *mut c_void,
}

impl Mapped<'_> {
    fn as_mut_ptr(&self) -> *mut c_void {
        self.ptr
    }
}

impl Drop for Mapped<'_> {
    fn drop(&mut self) {
        // SAFETY: 与创建时的 map 配对；句柄有效；本守卫持有期间内存保持映射。
        unsafe { (self.fns.unmap_memory)(self.device, self.memory) };
    }
}

/// 把 `rc` 变成 `GpuResult<()>`（错误信息里带上调用名与驱动返回码的名字）。
fn check(what: &str, rc: i32) -> GpuResult<()> {
    if rc == ffi::VK_SUCCESS {
        Ok(())
    } else {
        Err(GpuError::Driver {
            code: rc,
            message: format!("{what} 失败：{}", vk_result_name(rc)),
        })
    }
}

/// 把一个「创建对象」调用的结果包成 [`VkObject`]。
fn wrap_create(
    what: &'static str,
    rc: i32,
    handle: *mut c_void,
    device: vk::DeviceHandle,
    destroy: DestroyFn,
) -> GpuResult<VkObject> {
    check(what, rc)?;
    if handle.is_null() {
        // 驱动返回成功却没写输出参数 —— 几乎总是「我们给的结构体与驱动理解的不一致」。
        //
        // 归到 `Unsupported` 而不是 `Driver { code: rc }`（T3 review F4）：
        // 此刻 `rc == VK_SUCCESS`，把它当错误码塞进 `Driver` 只会打印出「驱动错误 0」，
        // 是个**假的错误码**。本项目一律让 `Driver.code` 只承载**真实**的 `VkResult`。
        return Err(GpuError::Unsupported(format!(
            "{what} 返回成功但句柄为空（结构体或参数不符）"
        )));
    }
    Ok(VkObject {
        handle,
        device,
        destroy,
    })
}

/// 挑一个同时满足 `required` 所有位的内存类型。
///
/// 与 `offscreen.rs::pick_memory_type` 同逻辑（那个函数不 `pub`，而本模块不允许改它）
/// —— 属性来自**物理设备**，逻辑设备上拿不到，所以由调用方传进来。
fn pick_memory_type(
    props: &vk::PhysicalDeviceMemoryProperties,
    type_bits: u32,
    required: u32,
) -> GpuResult<u32> {
    for i in 0..props.memory_type_count.min(32) {
        let m = &props.memory_types[i as usize];
        if type_bits & (1 << i) == 0 {
            continue;
        }
        if m.property_flags & required == required {
            return Ok(i);
        }
    }
    Err(GpuError::Unsupported(format!(
        "找不到满足属性 {required:#x} 的内存类型（资源的 type_bits = {type_bits:#x}）"
    )))
}

/// 分配一块设备内存。
fn alloc_memory(
    what: &'static str,
    device: vk::DeviceHandle,
    fns: &DeviceFns,
    size: u64,
    memory_type_index: u32,
) -> GpuResult<VkObject> {
    let info = vk::MemoryAllocateInfo {
        s_type: vk::VK_STRUCTURE_TYPE_MEMORY_ALLOCATE_INFO,
        p_next: std::ptr::null(),
        allocation_size: size,
        memory_type_index,
    };
    let mut handle: vk::DeviceMemoryHandle = std::ptr::null_mut();
    // SAFETY: `info` 在栈上存活；`handle` 是可写输出；函数指针来自成功解析的 loader。
    let rc = unsafe { (fns.allocate_memory)(device, &info, std::ptr::null(), &mut handle) };
    if rc != ffi::VK_SUCCESS {
        return Err(GpuError::Driver {
            code: rc,
            message: format!("{what} 失败（size={size}）：{}", vk_result_name(rc)),
        });
    }
    wrap_create(what, rc, handle, device, fns.free_memory)
}

/// map 一块主机可见内存，返回 **RAII 守卫**（释放即 `unmap`，见 [`Mapped`]）。
fn map_memory<'a>(
    what: &str,
    fns: &'a DeviceFns,
    device: vk::DeviceHandle,
    memory: vk::DeviceMemoryHandle,
) -> GpuResult<Mapped<'a>> {
    let mut ptr: *mut c_void = std::ptr::null_mut();
    // SAFETY: 内存是 HOST_VISIBLE；`ptr` 是可写输出；映射整块。
    check(what, unsafe {
        (fns.map_memory)(device, memory, 0, vk::WHOLE_SIZE, 0, &mut ptr)
    })?;
    if ptr.is_null() {
        // 措辞与实现必须一致（fix round 2 / R5）：`vkMapMemory` 返回成功却给出空指针是驱动异常，
        // 但按规范的语义「调用成功 ⇒ 该内存被视为已映射」—— 所以这里**防御性地解除映射**，
        // 不让一个可疑的映射留在进程里（否则下一次 map 会失败，症状离原因很远）。
        // SAFETY: 与上面刚成功的那次 map 配对。
        unsafe { (fns.unmap_memory)(device, memory) };
        return Err(GpuError::Unsupported(format!(
            "{what} 返回成功但给出空指针（已防御性 unmap）"
        )));
    }
    Ok(Mapped {
        fns,
        device,
        memory,
        ptr,
    })
}

/// 建一块缓冲 + 主机可见内存（顶点缓冲与回读暂存都用它）。
fn create_host_buffer(
    what: &'static str,
    device: vk::DeviceHandle,
    fns: &DeviceFns,
    mem_props: &vk::PhysicalDeviceMemoryProperties,
    size: u64,
    usage: u32,
) -> GpuResult<(VkObject, VkObject)> {
    let info = vk::BufferCreateInfo {
        s_type: vk::VK_STRUCTURE_TYPE_BUFFER_CREATE_INFO,
        p_next: std::ptr::null(),
        flags: 0,
        size,
        usage,
        sharing_mode: vk::VK_SHARING_MODE_EXCLUSIVE,
        queue_family_index_count: 0,
        p_queue_family_indices: std::ptr::null(),
    };
    let mut handle: vk::BufferHandle = std::ptr::null_mut();
    // SAFETY: 同上。
    let rc = unsafe { (fns.create_buffer)(device, &info, std::ptr::null(), &mut handle) };
    let buffer = wrap_create(what, rc, handle, device, fns.destroy_buffer)?;

    let mut req = std::mem::MaybeUninit::<vk::MemoryRequirements>::uninit();
    // SAFETY: 该函数完整写入结构体。
    unsafe { (fns.get_buffer_memory_requirements)(device, buffer.handle(), req.as_mut_ptr()) };
    let req = unsafe { req.assume_init() };
    let index = pick_memory_type(
        mem_props,
        req.memory_type_bits,
        vk::VK_MEMORY_PROPERTY_HOST_VISIBLE_BIT | vk::VK_MEMORY_PROPERTY_HOST_COHERENT_BIT,
    )?;
    let memory = alloc_memory(what, device, fns, req.size, index)?;
    // SAFETY: 缓冲与内存都是本设备的新对象，尺寸匹配；offset 0 合法。
    check("vkBindBufferMemory", unsafe {
        (fns.bind_buffer_memory)(device, buffer.handle(), memory.handle(), 0)
    })?;
    Ok((buffer, memory))
}

/// 离屏 GPU 几何渲染器。
///
/// ## 字段顺序（**不要重排**）
///
/// 资源在前、`device` 最后 ⇒ Rust 按声明顺序析构 ⇒ **`VkDevice` 最后销毁**，
/// 所有资源都在设备仍存活时被销毁（否则校验层会报 `leaked objects` 或直接崩）。
///
/// ## 为什么允许 `dead_code`
///
/// 其中几个字段（`layout` / `vs` / `fs` / `image_memory` / `view` / `pool` / `device`）
/// **从不被读取** —— 它们存在只为**所有权**：`Drop` 要在正确的顺序上把对象销毁。
/// `allow(dead_code)` 是这里的正确表达；**删掉任何一个都会漏资源或让销毁顺序变错**。
#[allow(dead_code)]
pub struct GpuGeometryRenderer {
    pass: RenderPass,
    pipeline: Pipeline,
    layout: PipelineLayout,
    vs: ShaderModule,
    fs: ShaderModule,
    image: VkObject,
    image_memory: VkObject,
    view: VkObject,
    framebuffer: VkObject,
    pool: VkObject,
    cmd: vk::CommandBufferHandle,
    /// 顶点缓冲（**惰性创建**：一帧都没画过非空几何时不分配）。
    vertex: Option<VertexBuffer>,
    staging: VkObject,
    staging_memory: VkObject,
    /// 回读暂存的字节数（= 宽 × 高 × 4）。
    staging_size: u64,
    fence: VkObject,
    fns: DeviceFns,
    device_handle: vk::DeviceHandle,
    queue: vk::QueueHandle,
    mem_props: vk::PhysicalDeviceMemoryProperties,
    extent: Extent,
    clear: [f32; 4],
    unsupported: Vec<String>,
    /// 提交同步状态：栅栏等待失败后置为 [`SubmitState::Broken`]，此后**拒绝复用**。
    sync: SubmitState,
    /// 文本管线资源；`None` = 没调 `with_text` ⇒ `Text` 命令报 `Unsupported`（M3a 行为）。
    ///
    /// **必须声明在 `device` 之前**（所有 Vulkan 子对象都在 `device` 之前析构）。
    text: Option<TextResources>,
    /// 累计发出的 host→vertex 屏障条数（诊断 + 回归，见
    /// [`GpuGeometryRenderer::host_to_vertex_barrier_count`]）。
    host_to_vertex_barriers: u64,
    /// 其中属于**形状**顶点缓冲的条数（见 [`GpuGeometryRenderer::shape_host_to_vertex_barrier_count`]）。
    shape_host_to_vertex_barriers: u64,
    /// 其中属于**文本**顶点缓冲的条数（review M2 的护栏，见
    /// [`GpuGeometryRenderer::text_host_to_vertex_barrier_count`]）。
    text_host_to_vertex_barriers: u64,
    /// **必须最后**（最后析构）。
    device: VkDevice,
}

impl GpuGeometryRenderer {
    /// 建一整套离屏 GPU 渲染设施（并打开 Vulkan 设备）。
    ///
    /// `extent` 为 0 时按 **1** 处理（与 CPU `Framebuffer::new(..max(1))` 同一约定）——
    /// Vulkan 图像不能是 0 宽高，而 CPU 能「画」1×1，所以这里也按 1×1 渲染，两边仍可比。
    /// [`GpuGeometryRenderer::extent`] 返回的是**实际**渲染尺寸。
    pub fn new(adapter_index: usize, extent: Extent, clear: Color) -> GpuResult<GpuGeometryRenderer> {
        let extent = Extent {
            width: extent.width.max(1),
            height: extent.height.max(1),
        };
        let device = VkDevice::open(adapter_index)?;
        let fns = *device.fns();
        let device_handle = device.handle();
        let mem_props = *device.memory_properties();

        // ① 着色器：顶点属性透传 + 按 `rect`/`radius_kind` 逐像素判定（M3a-T1）
        let vs = device.create_shader_module(&spirv::vertex_shader_rect_attrs())?;
        let fs = device.create_shader_module(&spirv::fragment_shader_rect_shape())?;

        // ② 渲染通道：清屏 + 离开通道即 `TRANSFER_SRC_OPTIMAL`（好直接回读）
        let pass = device.create_render_pass(
            COLOR_FORMAT,
            vk::VK_ATTACHMENT_LOAD_OP_CLEAR,
            vk::VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL,
        )?;

        // 管线布局：**不含推送常量** —— 顶点属性方案，刻意绕开 M2a 的推送常量地雷
        // （`spirv.rs::vertex_shader_rect_pushconstant` 的说明）。
        let layout = device.create_pipeline_layout(None)?;

        // ③ 管线：静态 viewport/scissor = 整幅 extent；真实顶点输入（binding stride 44）
        let entry = c"main";
        let stages = [
            vk::PipelineShaderStageCreateInfo {
                s_type: vk::VK_STRUCTURE_TYPE_PIPELINE_SHADER_STAGE_CREATE_INFO,
                p_next: std::ptr::null(),
                flags: 0,
                stage: vk::VK_SHADER_STAGE_VERTEX_BIT,
                module: vs.handle(),
                p_name: entry.as_ptr(),
                p_specialization_info: std::ptr::null(),
            },
            vk::PipelineShaderStageCreateInfo {
                s_type: vk::VK_STRUCTURE_TYPE_PIPELINE_SHADER_STAGE_CREATE_INFO,
                p_next: std::ptr::null(),
                flags: 0,
                stage: vk::VK_SHADER_STAGE_FRAGMENT_BIT,
                module: fs.handle(),
                p_name: entry.as_ptr(),
                p_specialization_info: std::ptr::null(),
            },
        ];
        let pipeline = device.create_vertex_pipeline(
            &stages,
            &layout,
            &pass,
            vk::Extent2D {
                width: extent.width,
                height: extent.height,
            },
            std::mem::size_of::<GpuVertex>() as u32,
            &vertex_attrs(),
        )?;

        // ④ 离屏图像（DEVICE_LOCAL：颜色附件 | 传输源）
        let img_info = vk::ImageCreateInfo {
            s_type: vk::VK_STRUCTURE_TYPE_IMAGE_CREATE_INFO,
            p_next: std::ptr::null(),
            flags: 0,
            image_type: vk::VK_IMAGE_TYPE_2D,
            format: COLOR_FORMAT,
            extent: vk::Extent3D {
                width: extent.width,
                height: extent.height,
                depth: 1,
            },
            mip_levels: 1,
            array_layers: 1,
            samples: vk::VK_SAMPLE_COUNT_1_BIT,
            tiling: vk::VK_IMAGE_TILING_OPTIMAL,
            usage: vk::VK_IMAGE_USAGE_COLOR_ATTACHMENT_BIT | vk::VK_IMAGE_USAGE_TRANSFER_SRC_BIT,
            sharing_mode: vk::VK_SHARING_MODE_EXCLUSIVE,
            queue_family_index_count: 0,
            p_queue_family_indices: std::ptr::null(),
            initial_layout: vk::VK_IMAGE_LAYOUT_UNDEFINED,
        };
        let mut image_handle: vk::ImageHandle = std::ptr::null_mut();
        // SAFETY: 结构体在栈上存活；句柄是可写输出。
        let rc = unsafe { (fns.create_image)(device_handle, &img_info, std::ptr::null(), &mut image_handle) };
        let image = wrap_create("vkCreateImage", rc, image_handle, device_handle, fns.destroy_image)?;

        let mut image_req = std::mem::MaybeUninit::<vk::MemoryRequirements>::uninit();
        // SAFETY: 该函数完整写入结构体。
        unsafe { (fns.get_image_memory_requirements)(device_handle, image.handle(), image_req.as_mut_ptr()) };
        let image_req = unsafe { image_req.assume_init() };
        let image_mem_index = pick_memory_type(
            &mem_props,
            image_req.memory_type_bits,
            vk::VK_MEMORY_PROPERTY_DEVICE_LOCAL_BIT,
        )?;
        let image_memory = alloc_memory(
            "vkAllocateMemory(image)",
            device_handle,
            &fns,
            image_req.size,
            image_mem_index,
        )?;
        // SAFETY: 图像与内存都是新对象，尺寸匹配；offset 0 合法。
        check("vkBindImageMemory", unsafe {
            (fns.bind_image_memory)(device_handle, image.handle(), image_memory.handle(), 0)
        })?;

        // ⑤ 视图 + 帧缓冲
        let view_info = vk::ImageViewCreateInfo {
            s_type: vk::VK_STRUCTURE_TYPE_IMAGE_VIEW_CREATE_INFO,
            p_next: std::ptr::null(),
            flags: 0,
            image: image.handle(),
            view_type: vk::VK_IMAGE_VIEW_TYPE_2D,
            format: COLOR_FORMAT,
            components_r: vk::VK_COMPONENT_SWIZZLE_IDENTITY,
            components_g: vk::VK_COMPONENT_SWIZZLE_IDENTITY,
            components_b: vk::VK_COMPONENT_SWIZZLE_IDENTITY,
            components_a: vk::VK_COMPONENT_SWIZZLE_IDENTITY,
            subresource_range: color_range(),
        };
        let mut view_handle: vk::ImageViewHandle = std::ptr::null_mut();
        // SAFETY: 同上。
        let rc = unsafe { (fns.create_image_view)(device_handle, &view_info, std::ptr::null(), &mut view_handle) };
        let view = wrap_create("vkCreateImageView", rc, view_handle, device_handle, fns.destroy_image_view)?;

        let fb_info = vk::FramebufferCreateInfo {
            s_type: vk::VK_STRUCTURE_TYPE_FRAMEBUFFER_CREATE_INFO,
            p_next: std::ptr::null(),
            flags: 0,
            render_pass: pass.handle(),
            attachment_count: 1,
            p_attachments: &view.handle(),
            width: extent.width,
            height: extent.height,
            layers: 1,
        };
        let mut fb_handle: vk::FramebufferHandle = std::ptr::null_mut();
        // SAFETY: 同上。
        let rc = unsafe { (fns.create_framebuffer)(device_handle, &fb_info, std::ptr::null(), &mut fb_handle) };
        let framebuffer = wrap_create("vkCreateFramebuffer", rc, fb_handle, device_handle, fns.destroy_framebuffer)?;

        // ⑥ 命令池 + 一个主命令缓冲
        let pool_info = vk::CommandPoolCreateInfo {
            s_type: vk::VK_STRUCTURE_TYPE_COMMAND_POOL_CREATE_INFO,
            p_next: std::ptr::null(),
            flags: vk::VK_COMMAND_POOL_CREATE_RESET_COMMAND_BUFFER_BIT,
            queue_family_index: device.queue_family_index(),
        };
        let mut pool_handle: vk::CommandPoolHandle = std::ptr::null_mut();
        // SAFETY: 同上。
        let rc = unsafe { (fns.create_command_pool)(device_handle, &pool_info, std::ptr::null(), &mut pool_handle) };
        let pool = wrap_create("vkCreateCommandPool", rc, pool_handle, device_handle, fns.destroy_command_pool)?;

        let alloc_info = vk::CommandBufferAllocateInfo {
            s_type: vk::VK_STRUCTURE_TYPE_COMMAND_BUFFER_ALLOCATE_INFO,
            p_next: std::ptr::null(),
            command_pool: pool.handle(),
            level: vk::VK_COMMAND_BUFFER_LEVEL_PRIMARY,
            command_buffer_count: 1,
        };
        let mut cmd: vk::CommandBufferHandle = std::ptr::null_mut();
        // SAFETY: 池有效；`cmd` 是可写输出。
        check("vkAllocateCommandBuffers", unsafe {
            (fns.allocate_command_buffers)(device_handle, &alloc_info, &mut cmd)
        })?;
        if cmd.is_null() {
            return Err(GpuError::Unsupported(
                "vkAllocateCommandBuffers 返回成功但命令缓冲为空".to_string(),
            ));
        }

        // ⑦ 回读暂存（HOST_VISIBLE | HOST_COHERENT ⇒ 不需要显式 flush）
        let staging_size = (extent.width as u64) * (extent.height as u64) * 4;
        let (staging, staging_memory) = create_host_buffer(
            "vkCreateBuffer(staging)",
            device_handle,
            &fns,
            &mem_props,
            staging_size,
            vk::VK_BUFFER_USAGE_TRANSFER_DST_BIT,
        )?;

        // ⑧ 栅栏（每帧复用：提交前 reset、提交后等它）
        let fence_info = vk::FenceCreateInfo {
            s_type: vk::VK_STRUCTURE_TYPE_FENCE_CREATE_INFO,
            p_next: std::ptr::null(),
            flags: 0, // 不预设 signaled ⇒ 第一次等就是真的等 GPU
        };
        let mut fence_handle: vk::FenceHandle = std::ptr::null_mut();
        // SAFETY: 同上。
        let rc = unsafe { (fns.create_fence)(device_handle, &fence_info, std::ptr::null(), &mut fence_handle) };
        let fence = wrap_create("vkCreateFence", rc, fence_handle, device_handle, fns.destroy_fence)?;

        Ok(GpuGeometryRenderer {
            pass,
            pipeline,
            layout,
            vs,
            fs,
            image,
            image_memory,
            view,
            framebuffer,
            pool,
            cmd,
            vertex: None,
            staging,
            staging_memory,
            staging_size,
            fence,
            fns,
            device_handle,
            queue: device.queue(),
            mem_props,
            extent,
            clear: [
                clear.r as f32 / 255.0,
                clear.g as f32 / 255.0,
                clear.b as f32 / 255.0,
                clear.a,
            ],
            unsupported: Vec::new(),
            sync: SubmitState::Idle,
            text: None,
            host_to_vertex_barriers: 0,
            shape_host_to_vertex_barriers: 0,
            text_host_to_vertex_barriers: 0,
            device,
        })
    }

    /// **实际**渲染尺寸（请求 0 尺寸时是 1×1，见 [`GpuGeometryRenderer::new`]）。
    pub fn extent(&self) -> Extent {
        self.extent
    }

    /// **让本渲染器支持文本**：接管一个 [`TextEngine`]（字体 + 字形图集 + 排版缓存）。
    ///
    /// 不调用它的渲染器保持 **M3a 行为不变**：`DrawCmd::Text` ⇒ `Unsupported`
    /// （不会静默丢弃）。调用之后，文本命令由**第二条管线**绘制（独立顶点缓冲、独立管线、
    /// 一条 `set 0 / binding 0` 的组合图像采样器指向字形图集）。
    ///
    /// ## 资源与生命周期
    ///
    /// - 文本管线用**独立**的顶点布局（`TextVertex`，stride 32）—— M3a 的 `GpuVertex`
    ///   （stride 44）已冻结，两者不共用顶点缓冲；
    /// - 管线布局带一个描述符集布局（`set 0`）—— 文本的片元着色器声明了
    ///   `OpTypeSampledImage`，布局里没有对应 set 的话 `vkCreateGraphicsPipelines` 会拒绝；
    /// - 图集纹理**不在这里上传**：改为在 [`Self::render`] 里按「图集指纹」惰性重传
    ///   （图集只在出现新字形时变化，见 [`TextResources::uploaded`]）。
    pub fn with_text(mut self, engine: TextEngine) -> GpuResult<Self> {
        // 与 M3a 的 shape 管线完全独立：不同顶点布局、不同着色器、多一条描述符集
        let vs = self.device.create_shader_module(&spirv::vertex_shader_text())?;
        let fs = self.device.create_shader_module(&spirv::fragment_shader_text())?;
        let set_layout = self.device.create_descriptor_set_layout_combined_sampler()?;
        let layout = self
            .device
            .create_pipeline_layout_ex(None, Some(&set_layout))?;
        let stages = [
            vk::PipelineShaderStageCreateInfo {
                s_type: vk::VK_STRUCTURE_TYPE_PIPELINE_SHADER_STAGE_CREATE_INFO,
                p_next: std::ptr::null(),
                flags: 0,
                stage: vk::VK_SHADER_STAGE_VERTEX_BIT,
                module: vs.handle(),
                p_name: c"main".as_ptr(),
                p_specialization_info: std::ptr::null(),
            },
            vk::PipelineShaderStageCreateInfo {
                s_type: vk::VK_STRUCTURE_TYPE_PIPELINE_SHADER_STAGE_CREATE_INFO,
                p_next: std::ptr::null(),
                flags: 0,
                stage: vk::VK_SHADER_STAGE_FRAGMENT_BIT,
                module: fs.handle(),
                p_name: c"main".as_ptr(),
                p_specialization_info: std::ptr::null(),
            },
        ];
        let pipeline = self.device.create_vertex_pipeline(
            &stages,
            &layout,
            &self.pass,
            vk::Extent2D {
                width: self.extent.width,
                height: self.extent.height,
            },
            std::mem::size_of::<TextVertex>() as u32,
            &text_attrs(),
        )?;
        let sampler = self.device.create_sampler()?;
        let pool = self.device.create_descriptor_pool(1)?;
        let set = self.device.allocate_descriptor_set(&pool, &set_layout)?;

        self.text = Some(TextResources {
            engine,
            pipeline,
            layout,
            vs,
            fs,
            set_layout,
            // 字段顺序有契约：`set` 必须在 `pool` 之前（见 `TextResources` 的文档）
            set,
            pool,
            sampler,
            vertex: None,
            texture: None,
            uploaded: None,
            skipped: 0,
        });
        Ok(self)
    }

    /// 上一帧被**跳过**的文本命令数（== [`crate::gpu_text::TextStream::skipped`]）。
    ///
    /// 与 M3a 的假阳性相关：空串 / `size <= 0` / 被裁空的文本**不再报错**，但也不该
    /// 无声无息 —— 想知道「这一帧有几条文本什么都没画」就查这里。
    pub fn text_skipped(&self) -> usize {
        self.text.as_ref().map_or(0, |t| t.skipped)
    }

    /// 文本管线是否已接管（即是否调过 [`GpuGeometryRenderer::with_text`]）。
    pub fn text_enabled(&self) -> bool {
        self.text.is_some()
    }

    /// **校验层是否真的启用**（不是「是否请求」）—— 见 [`VkDevice::validation_enabled`]。
    ///
    /// 测试可以据此**断言**「`DEER_VK_VALIDATION=1` ⇒ 校验层确实在跑」，从而把
    /// 「零校验消息」从人工观察升级成可回归结论（T3 review F7）。
    pub fn validation_enabled(&self) -> bool {
        self.device.validation_enabled()
    }

    /// 上一帧里**未能翻译**的命令说明（目前只有 `DrawCmd::Text`）。
    ///
    /// 与 [`GpuGeometryRenderer::render`] 的错误配套：`render` 返回 `Unsupported` 时，
    /// 这里能看到具体是哪几条、什么内容。
    pub fn unsupported(&self) -> &[String] {
        &self.unsupported
    }

    /// 累计发出的 **host→vertex 屏障**条数。
    ///
    /// 为什么需要这个计数器（fix round 2 / R1-1）：屏障的**参数**可以由纯函数 + 单元测试钉住，
    /// 但「屏障有没有真的被发出来」在进程内原本不可观测 —— 于是「把整段屏障删掉」这种变异
    /// （= 原始缺陷复原）在 16 个测试靶上**全绿**。计数器把这件事变成可断言的：
    /// 一个「有顶点」的帧必须让计数 +1，空帧不得增加。
    ///
    /// 诚实说明它的上限：它证明「这段代码被执行了」，不能证明驱动真的按语义用了这条屏障
    /// （那属于真机逐像素对照的范畴）。反过来说，任何「删掉屏障却留着计数自增」的变异是
    /// 刻意构造的，不在防御范围内。
    pub fn host_to_vertex_barrier_count(&self) -> u64 {
        self.host_to_vertex_barriers
    }

    /// 累计发出的 host→VERTEX_INPUT 屏障里，**属于文本顶点缓冲**的那部分（review M2）。
    ///
    /// ## 为什么需要分项计数
    ///
    /// 文本路径引入了**第二块**顶点缓冲，它的屏障一开始只有实现、没有护栏：
    /// reviewer 把那条发射整体删掉，`cargo test -p deer-vk` **17 靶仍然全绿**
    /// （总数计数器只被形状帧的断言驱动，文本帧没人查）。
    ///
    /// 现在测试用差值断言：**文本单管线帧 ⇒ 本计数 +1**、**形状+文本交错帧 ⇒ 总数 +2 且本计数 +1**、
    /// **全被跳过的文本帧 ⇒ 本计数 +0**。删掉发射必然红。
    pub fn text_host_to_vertex_barrier_count(&self) -> u64 {
        self.text_host_to_vertex_barriers
    }

    /// 与 [`Self::text_host_to_vertex_barrier_count`] 对称：属于形状顶点缓冲的那部分。
    pub fn shape_host_to_vertex_barrier_count(&self) -> u64 {
        self.shape_host_to_vertex_barriers
    }

    /// **把渲染器置为「上次提交未确认完成」** —— 之后所有 `render` 都会报错。
    ///
    /// 真实触发路径是栅栏等待失败（超时/设备丢失），那在测试里无法稳定复现；这个入口让
    /// 「Broken ⇒ render 必 Err（且不碰命令缓冲/顶点缓冲/栅栏）」成为**可回归**断言，
    /// 而不是靠读代码确认接线。名字带 `for_test` 是刻意的：它不是给正常调用方的功能。
    #[doc(hidden)]
    pub fn force_unconfirmed_submit_for_test(&mut self) {
        self.sync.mark_broken();
    }

    /// 画一帧并回读 RGBA8（长度 = 宽 × 高 × 4，行优先、无 padding）。
    ///
    /// ## 报错策略（与 CPU 后端对齐）
    ///
    /// - **裁剪栈不平衡**（`!list.clip_balanced()`，帧末净计数）⇒ 报错。
    ///   判据与 `null.rs:227-236`（`CpuFrame::record` / `CpuRenderer::render`）**完全相同**
    ///   （CPU 用的错误形态是 `Driver { code: -1 }` 哨兵；这里用 `Unsupported` ——
    ///   两边**都报错**才是契约，错误码不必逐字节相同，而假的 `-1` 不该当 VkResult 用）。
    ///   刻意**不用** [`gpu_geom::GpuStream::clip_unbalanced`] 当报错条件 —— 它更严
    ///   （额外拒绝「多出的 `PopClip`」这种 CPU 画得出来的列表），拿它报错会让 GPU 拒收
    ///   CPU 能画的输入，两边行为不再可比；那个字段是**诊断**用的；
    /// - **`unsupported` 非空**（文本）⇒ `GpuError::Unsupported` + [`Self::unsupported`] 可查；
    /// - **上一次提交没等到栅栏** ⇒ 本渲染器已不可复用，直接报错（见 [`SubmitState`]）；
    /// - 其余是 `GpuError::Driver`（Vulkan 调用失败），`code` 一定是**真实的** `VkResult`。
    pub fn render(&mut self, list: &DrawList) -> GpuResult<Vec<u8>> {
        self.unsupported.clear();

        // ⓪ 上一次提交没确认完成 ⇒ 什么都不许碰（命令缓冲/顶点缓冲可能仍在被 GPU 用）
        self.sync.ensure_reusable()?;

        // ① 入口检查（与 CPU 同一判据）
        if !list.clip_balanced() {
            return Err(GpuError::Unsupported(
                "绘制列表的裁剪栈不平衡（PushClip/PopClip 未配对）".to_string(),
            ));
        }

        // ② **单次遍历**：按 `DrawList` 的原顺序把每条命令送进对应的翻译层，
        //    同时记录绘制段（`DrawCall`）—— 这样形状与文本的**交错顺序**被完整保留
        //    （z 序），而不是「先画所有形状、再画所有文本」。
        let mut shape_verts: Vec<GpuVertex> = Vec::new();
        let mut text_verts: Vec<TextVertex> = Vec::new();
        let mut calls: Vec<DrawCall> = Vec::new();
        let mut active_clip: Vec<RectI> = Vec::new();
        let mut text_skipped = 0usize;
        // 借出文本资源：避免在循环里同时可变借用 `self.text` 与读 `self.unsupported` 等字段。
        // ⚠️ 因此**循环体内绝不提前 `return`**（review M3）：任何早退都会把 `self.text`
        //    留在 `None`（文本资源丢失）。致命错误记进 `fatal`，循环后统一「还原 + 返回」。
        let mut text_res = self.text.take();
        let mut fatal: Option<GpuError> = None;

        for cmd in &list.cmds {
            match cmd {
                DrawCmd::PushClip { rect } => active_clip.push(*rect),
                DrawCmd::PopClip => {
                    active_clip.pop();
                }
                // 诊断提示：忽略（与两条翻译层一致）
                DrawCmd::NodeHint { .. } => {}
                // 文本：有引擎就翻译；没引擎时走下面那个分支（M3a 行为不变）
                DrawCmd::Text { rect, text, .. } if text_res.is_some() => {
                    let res = text_res.as_mut().expect("刚判过 is_some");
                    let one = single_command_in_clip(&active_clip, cmd);
                    let s = gpu_text::build_text_stream(&one, self.extent, &mut res.engine);
                    text_skipped += s.skipped;
                    if !s.vertices.is_empty() {
                        match vertex_range(text_verts.len(), s.vertices.len()) {
                            Ok((first, count)) => {
                                text_verts.extend_from_slice(&s.vertices);
                                calls.push(DrawCall {
                                    kind: PipelineKind::Text,
                                    first,
                                    count,
                                });
                            }
                            Err(e) => {
                                fatal = Some(e);
                                break;
                            }
                        }
                    }
                    let _ = (rect, text);
                }
                DrawCmd::Text { rect, text, .. } => {
                    // 没有 `TextEngine` ⇒ M3a 行为：**报告**而不是静默丢弃
                    absorb_unsupported(
                        &mut self.unsupported,
                        &[format!(
                            "DrawCmd::Text(rect=({}, {}, {}×{}), {} 字符)：GPU 后端尚未实现文本绘制",
                            rect.x,
                            rect.y,
                            rect.w,
                            rect.h,
                            text.chars().count()
                        )],
                    );
                }
                // 其余（形状命令）：走 M3a 的翻译层
                other => {
                    let one = single_command_in_clip(&active_clip, other);
                    let s = gpu_geom::build_stream(&one, self.extent);
                    // **M1（review）**：形状路径今天不会产出 `unsupported`，但**将来会**
                    // （`gpu_geom` 里任何新增的未支持命令都会落到这里）。丢掉它 =
                    // 「实现有、护栏没有」的反面：**静默少画**。所以按与 `Text` 同一策略处理 ——
                    // 收集起来，帧末统一报错（`absorb_unsupported` + `unsupported_outcome`）。
                    absorb_unsupported(&mut self.unsupported, &s.unsupported);
                    if !s.vertices.is_empty() {
                        match vertex_range(shape_verts.len(), s.vertices.len()) {
                            Ok((first, count)) => {
                                shape_verts.extend_from_slice(&s.vertices);
                                calls.push(DrawCall {
                                    kind: PipelineKind::Shape,
                                    first,
                                    count,
                                });
                            }
                            Err(e) => {
                                fatal = Some(e);
                                break;
                            }
                        }
                    }
                }
            }
        }

        // ★ 无论循环是正常结束还是 `break`，都先把文本资源还回去（M3）
        if let Some(res) = text_res.as_mut() {
            res.skipped = text_skipped;
        }
        self.text = text_res;

        if let Some(e) = fatal {
            return Err(e);
        }
        unsupported_outcome(&self.unsupported)?;

        // ③④⑤⑥ 上传顶点/图集（按需）+ 录制 + 提交 + 回读
        self.record_and_submit(&shape_verts, &text_verts, &calls)?;
        self.read_back()
    }

    /// 确保某个顶点缓冲至少有 `bytes` 字节（不够就按 2 的幂重建），**先建后换**。
    ///
    /// 两个缓冲（形状 stride 44 / 文本 stride 32）共用这段逻辑 —— 它们只在「容量」上不同。
    fn ensure_vertex_capacity(&mut self, slot: &mut Option<VertexBuffer>, bytes: u64) -> GpuResult<()> {
        // 破坏性操作（会销毁旧缓冲、分配新内存）⇒ 守卫放在这里（见 R1-3）。
        self.sync.ensure_reusable()?;
        if slot.as_ref().is_some_and(|v| v.capacity >= bytes) {
            return Ok(());
        }
        let capacity = bytes.next_power_of_two().max(MIN_VERTEX_BYTES);
        // **先建新的、成功后再换**（T3 review F11）：失败时旧缓冲仍然可用、容量信息不丢。
        // 析构顺序仍然正确：`VertexBuffer` 的字段顺序保证「先缓冲、后内存」；
        // 赋值时旧值被 drop，此刻上一帧的提交已经等过栅栏 ⇒ 缓冲不在使用中。
        let (buffer, memory) = create_host_buffer(
            "vkCreateBuffer(vertex)",
            self.device_handle,
            &self.fns,
            &self.mem_props,
            capacity,
            vk::VK_BUFFER_USAGE_VERTEX_BUFFER_BIT,
        )?;
        *slot = Some(VertexBuffer {
            buffer,
            memory,
            capacity,
        });
        Ok(())
    }

    /// 把一段**已经是 `#[repr(C)]` 纯 `f32`** 的顶点数据写进缓冲（map → memcpy → unmap）。
    ///
    /// `what` 只用于报错里指认是哪个缓冲（形状 / 文本）。
    fn upload_vertices(&self, vb: &VertexBuffer, src: &[u8], what: &str) -> GpuResult<()> {
        // map 的守卫：**任何**提前返回都会 unmap（T3 review F9）。
        let mapped = map_memory(what, &self.fns, self.device_handle, vb.memory.handle())?;
        // SAFETY: 映射了整块缓冲（≥ src.len()，由 `ensure_vertex_capacity` 保证）；源与目标不重叠。
        unsafe {
            std::ptr::copy_nonoverlapping(src.as_ptr(), mapped.as_mut_ptr() as *mut u8, src.len());
        }
        Ok(())
    }

    /// 图集**变化时**才重传纹理，并把新纹理写进描述符集。
    ///
    /// 指纹 = `(图集宽, 图集高, 已光栅化字形数)`（见 [`TextResources::uploaded`]）。
    /// 上传走 `create_texture_r8`（一次性路径，内部 `vkQueueWaitIdle`）——
    /// **只在图集变化时发生**（新字形首次出现），所以那次等空闲被摊薄；
    /// 若将来改成每帧重传，就必须改异步上传 + 栅栏（`device.rs` 的文档里写着这条前提）。
    fn refresh_atlas_texture(&mut self) -> GpuResult<()> {
        let (key, data) = {
            let res = self.text.as_ref().expect("调用方保证了文本资源存在");
            let (w, h) = res.engine.atlas().size();
            let key = (w, h, res.engine.rasterized_glyphs());
            if res.uploaded == Some(key) {
                return Ok(());
            }
            (key, res.engine.atlas().coverage().to_vec())
        };
        let (w, h, glyphs) = key;
        let texture = self.device.create_texture_r8(w, h, &data)?;
        {
            let res = self.text.as_mut().expect("同上");
            self.device
                .update_descriptor_texture(&res.set, &texture, &res.sampler)?;
            res.texture = Some(texture);
            res.uploaded = Some((w, h, glyphs));
        }
        Ok(())
    }

    /// 为某块顶点缓冲发一条「主机写 → 顶点取数」屏障（参数见 [`vertex_buffer_barrier_params`]）。
    ///
    /// 收**句柄**而不是 `&VertexBuffer`：调用点通常正持有 `self.vertex` / `self.text` 的借用，
    /// 传引用会和 `&mut self`（要自增计数器）撞借用检查 —— 句柄是 `Copy` 的普通值。
    ///
    /// `kind` 决定除总数之外**再**记到哪个分项计数器上（review M2：文本那条屏障要有单独护栏）。
    /// 计数与 Vulkan 调用**写在同一处** ⇒ 删掉发射就必然删掉计数，护栏挡得住「整体删掉」这类变异。
    fn emit_host_to_vertex_barrier(&mut self, buffer: vk::BufferHandle, kind: PipelineKind) {
        let p = vertex_buffer_barrier_params();
        let host_to_vertex = vk::BufferMemoryBarrier {
            s_type: vk::VK_STRUCTURE_TYPE_BUFFER_MEMORY_BARRIER,
            p_next: std::ptr::null(),
            src_access_mask: p.src_access,
            dst_access_mask: p.dst_access,
            src_queue_family_index: vk::QUEUE_FAMILY_IGNORED,
            dst_queue_family_index: vk::QUEUE_FAMILY_IGNORED,
            buffer,
            offset: 0,
            size: vk::WHOLE_SIZE,
        };
        // SAFETY: 结构体在栈上存活；命令缓冲处于录制状态；`buffer` 是本结构持有的有效句柄。
        unsafe {
            (self.fns.cmd_pipeline_barrier)(
                self.cmd,
                p.src_stage,
                p.dst_stage,
                0,
                0,
                std::ptr::null(),
                1,
                &host_to_vertex,
                0,
                std::ptr::null(),
            );
        }
        self.host_to_vertex_barriers += 1;
        match kind {
            PipelineKind::Shape => self.shape_host_to_vertex_barriers += 1,
            PipelineKind::Text => self.text_host_to_vertex_barriers += 1,
        }
    }

    /// 录制一帧（清屏 + 按 z 序逐段绑定管线/顶点缓冲 + 绘制 + 屏障 + 拷贝），提交并等栅栏。
    ///
    /// 需要 `&mut self`：等待失败时要把 [`SubmitState`] 置为 `Broken`（见模块文档「同步②」）。
    fn record_and_submit(
        &mut self,
        shape_verts: &[GpuVertex],
        text_verts: &[TextVertex],
        calls: &[DrawCall],
    ) -> GpuResult<()> {
        // ★ **守卫就放在破坏性操作本身**（fix round 2 / R1-3）：`render` 开头那句检查可能
        //   因为调用方漏写 `?` 而失效（reviewer 的变异 C 就是这么全绿的）。这里再查一次，
        //   于是「Broken ⇒ 绝不去碰命令缓冲/栅栏」不依赖任何调用方的写法。
        self.sync.ensure_reusable()?;

        // ① 上传：形状与文本各有独立缓冲，各自「按需扩容 + 每帧重传」
        if !shape_verts.is_empty() {
            let bytes = std::mem::size_of_val(shape_verts) as u64;
            let mut slot = self.vertex.take();
            let r = self.ensure_vertex_capacity(&mut slot, bytes);
            self.vertex = slot;
            r?;
            let bytes = std::mem::size_of_val(shape_verts);
            // SAFETY: `GpuVertex` 是 `#[repr(C)]` 纯 `f32`（无指针、无 Drop）⇒ 字节视图合法。
            let src = unsafe {
                std::slice::from_raw_parts(shape_verts.as_ptr() as *const u8, bytes)
            };
            let vb = self.vertex.as_ref().expect("ensure 之后必有缓冲");
            self.upload_vertices(vb, src, "vkMapMemory(shape vertex)")?;
        }
        if !text_verts.is_empty() {
            let bytes = std::mem::size_of_val(text_verts) as u64;
            let mut slot = self
                .text
                .as_mut()
                .expect("有文本顶点 ⇒ 文本资源存在")
                .vertex
                .take();
            let r = self.ensure_vertex_capacity(&mut slot, bytes);
            if let Some(res) = self.text.as_mut() {
                res.vertex = slot;
            }
            r?;
            let bytes = std::mem::size_of_val(text_verts);
            // SAFETY: `TextVertex` 是 `#[repr(C)]` 纯 `f32` ⇒ 字节视图合法。
            let src = unsafe {
                std::slice::from_raw_parts(text_verts.as_ptr() as *const u8, bytes)
            };
            let vb = self
                .text
                .as_ref()
                .and_then(|r| r.vertex.as_ref())
                .expect("ensure 之后必有缓冲");
            self.upload_vertices(vb, src, "vkMapMemory(text vertex)")?;
            // 图集若变了就重传纹理 + 更新描述符集（**必须在提交之前**）
            self.refresh_atlas_texture()?;
        }

        // SAFETY: `self.cmd` 是 `new()` 里从本结构的命令池分配出来的主命令缓冲句柄，仍然有效；
        // 上一次使用它的提交已经在 `record_and_submit` 末尾等到栅栏（或已被判为 Broken ⇒ 上面
        // 那行守卫已经返回 Err），所以此刻它**不在**执行中，可以重置。函数指针来自成功解析的
        // loader（`DeviceFns`），设备比本结构活得久（`device` 字段最后析构）。
        // 上一帧已经等过栅栏 ⇒ 命令缓冲不在执行中，可以重置。
        check("vkResetCommandBuffer", unsafe {
            (self.fns.reset_command_buffer)(self.cmd, 0)
        })?;
        let begin = vk::CommandBufferBeginInfo {
            s_type: vk::VK_STRUCTURE_TYPE_COMMAND_BUFFER_BEGIN_INFO,
            p_next: std::ptr::null(),
            flags: vk::VK_COMMAND_BUFFER_USAGE_ONE_TIME_SUBMIT_BIT,
            p_inheritance_info: std::ptr::null(),
        };
        // SAFETY: 句柄有效；`begin` 在栈上存活。
        check("vkBeginCommandBuffer", unsafe {
            (self.fns.begin_command_buffer)(self.cmd, &begin)
        })?;

        // ★ 主机刚写进顶点缓冲（map/memcpy/unmap）→ GPU 的 VERTEX_INPUT 要读它。
        //   **每个本帧用到的缓冲各一条**（形状与文本是两块独立缓冲）。
        //   参数来自纯函数（可被单元测试钉常量）；「屏障是否真的发出」由计数器断言 ——
        //   `host_to_vertex_barrier_count()`（总数）+ `text_host_to_vertex_barrier_count()`
        //   （**文本路径单独计数**，review M2）：两条合起来才让「文本那条屏障」也可回归。
        //   历史：文本屏障曾经**只有实现、没有护栏** —— reviewer 把它整体删掉、17 靶仍然全绿。
        //   （放在渲染通道**之前**：缓冲区屏障在通道内也合法，但放在外面更简单、更不容易踩
        //     「通道内允许哪些屏障」的规则。）
        if !shape_verts.is_empty() {
            let h = self.vertex.as_ref().expect("形状顶点已上传").buffer.handle();
            self.emit_host_to_vertex_barrier(h, PipelineKind::Shape);
        }
        if !text_verts.is_empty() {
            let h = self
                .text
                .as_ref()
                .and_then(|r| r.vertex.as_ref())
                .expect("文本顶点已上传")
                .buffer
                .handle();
            self.emit_host_to_vertex_barrier(h, PipelineKind::Text);
        }

        let clear_value = vk::ClearValue {
            color: vk::ClearColorValue {
                float32: self.clear,
            },
        };
        let begin_pass = vk::RenderPassBeginInfo {
            s_type: vk::VK_STRUCTURE_TYPE_RENDER_PASS_BEGIN_INFO,
            p_next: std::ptr::null(),
            render_pass: self.pass.handle(),
            framebuffer: self.framebuffer.handle(),
            render_area: vk::Rect2D {
                offset: vk::Offset2D { x: 0, y: 0 },
                extent: vk::Extent2D {
                    width: self.extent.width,
                    height: self.extent.height,
                },
            },
            clear_value_count: 1,
            p_clear_values: &clear_value,
        };
        // SAFETY: 上述结构体都在本栈帧存活；句柄都是本结构持有的有效句柄。
        unsafe {
            (self.fns.cmd_begin_render_pass)(self.cmd, &begin_pass, vk::VK_SUBPASS_CONTENTS_INLINE);
            // ⚠️ **不调** `vkCmdSetViewport`/`vkCmdSetScissor`：本管线的 viewport/scissor 是
            // **静态**的（写死在管线里，见 `device.rs::create_vertex_pipeline`）。对静态状态
            // 发动态设置命令会触发校验层报错 —— `tests/vbo_probe.rs` 记着这条实测。
            //
            // **按 z 序逐段绘制**：只在「管线切换」时重新绑定管线/顶点缓冲/描述符集，
            // 同一管线的连续段只更新 `vkCmdDraw` 的 `firstVertex`。
            let mut bound: Option<PipelineKind> = None;
            for call in calls {
                if bound != Some(call.kind) {
                    match call.kind {
                        PipelineKind::Shape => {
                            (self.fns.cmd_bind_pipeline)(
                                self.cmd,
                                vk::VK_PIPELINE_BIND_POINT_GRAPHICS,
                                self.pipeline.handle(),
                            );
                            let vb = self.vertex.as_ref().expect("形状段 ⇒ 缓冲已上传");
                            let offset: vk::DeviceSize = 0;
                            (self.fns.cmd_bind_vertex_buffers)(
                                self.cmd,
                                0,
                                1,
                                &vb.buffer.handle(),
                                &offset,
                            );
                        }
                        PipelineKind::Text => {
                            let res = self.text.as_ref().expect("文本段 ⇒ 文本资源存在");
                            (self.fns.cmd_bind_pipeline)(
                                self.cmd,
                                vk::VK_PIPELINE_BIND_POINT_GRAPHICS,
                                res.pipeline.handle(),
                            );
                            let vb = res.vertex.as_ref().expect("文本段 ⇒ 缓冲已上传");
                            let offset: vk::DeviceSize = 0;
                            (self.fns.cmd_bind_vertex_buffers)(
                                self.cmd,
                                0,
                                1,
                                &vb.buffer.handle(),
                                &offset,
                            );
                            // 字形图集：set 0 / binding 0（与着色器的装饰逐字对应）
                            (self.fns.cmd_bind_descriptor_sets)(
                                self.cmd,
                                vk::VK_PIPELINE_BIND_POINT_GRAPHICS,
                                res.layout.handle(),
                                0,
                                1,
                                &res.set.handle(),
                                0,
                                std::ptr::null(),
                            );
                        }
                    }
                    bound = Some(call.kind);
                }
                (self.fns.cmd_draw)(self.cmd, call.count, 1, call.first, 0);
            }
            (self.fns.cmd_end_render_pass)(self.cmd);
        }

        // 屏障：`oldLayout` 必须是图像**当前实际**布局 ⇒ 向渲染通道要 `final_layout()`。
        // （曾经在别处写死 `COLOR_ATTACHMENT_OPTIMAL` 而渲染通道给的是
        //   `TRANSFER_SRC_OPTIMAL`，校验层直接报 `cannot transition the layout`。）
        let barrier = vk::ImageMemoryBarrier {
            s_type: vk::VK_STRUCTURE_TYPE_IMAGE_MEMORY_BARRIER,
            p_next: std::ptr::null(),
            src_access_mask: vk::VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT,
            dst_access_mask: vk::VK_ACCESS_TRANSFER_READ_BIT,
            old_layout: self.pass.final_layout(),
            new_layout: vk::VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL,
            src_queue_family_index: vk::QUEUE_FAMILY_IGNORED,
            dst_queue_family_index: vk::QUEUE_FAMILY_IGNORED,
            image: self.image.handle(),
            subresource_range: color_range(),
        };
        let copy = vk::BufferImageCopy {
            buffer_offset: 0,
            buffer_row_length: 0, // 0 = 与图像宽度一致（无 padding）
            buffer_image_height: 0,
            image_subresource: vk::ImageSubresourceLayers {
                aspect_mask: vk::VK_IMAGE_ASPECT_COLOR_BIT,
                mip_level: 0,
                base_array_layer: 0,
                layer_count: 1,
            },
            image_offset: vk::Offset3D { x: 0, y: 0, z: 0 },
            image_extent: vk::Extent3D {
                width: self.extent.width,
                height: self.extent.height,
                depth: 1,
            },
        };
        // SAFETY: 结构体在栈上存活。
        unsafe {
            (self.fns.cmd_pipeline_barrier)(
                self.cmd,
                vk::VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT,
                vk::VK_PIPELINE_STAGE_TRANSFER_BIT,
                0,
                0,
                std::ptr::null(),
                0,
                std::ptr::null(),
                1,
                &barrier,
            );
            (self.fns.cmd_copy_image_to_buffer)(
                self.cmd,
                self.image.handle(),
                vk::VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL,
                self.staging.handle(),
                1,
                &copy,
            );
        }
        // SAFETY: 句柄有效且处于录制状态。
        check("vkEndCommandBuffer", unsafe {
            (self.fns.end_command_buffer)(self.cmd)
        })?;

        // 提交前把栅栏放回未信号态（它上一帧已经信号过了）。
        // SAFETY: 栅栏有效且当前没有等待者（上一帧已等到）。
        check("vkResetFences", unsafe {
            (self.fns.reset_fences)(self.device_handle, 1, &self.fence.handle())
        })?;
        let submit = vk::SubmitInfo {
            s_type: vk::VK_STRUCTURE_TYPE_SUBMIT_INFO,
            p_next: std::ptr::null(),
            wait_semaphore_count: 0,
            p_wait_semaphores: std::ptr::null(),
            p_wait_dst_stage_mask: std::ptr::null(),
            command_buffer_count: 1,
            p_command_buffers: &self.cmd,
            signal_semaphore_count: 0,
            p_signal_semaphores: std::ptr::null(),
        };
        // **提交前**把状态标成「在飞」：从这里到 `resolve` 之间任何失败都意味着
        // 「未知 GPU 状态」，不允许再复用资源（见 `SubmitState`）。
        self.sync.begin();
        // SAFETY: 队列与命令缓冲都有效；`submit` 在栈上存活；栅栏用于同步。
        if let Err(e) = check("vkQueueSubmit", unsafe {
            (self.fns.queue_submit)(self.queue, 1, &submit, self.fence.handle())
        }) {
            // 提交失败时**无法**证明设备没有开始执行这条命令缓冲（例如 DEVICE_LOST）。
            self.sync.mark_broken();
            return Err(e);
        }
        // 有限超时：驱动出问题时宁可失败，不要永久挂住。
        // SAFETY: 栅栏有效。
        let rc = unsafe {
            (self.fns.wait_for_fences)(
                self.device_handle,
                1,
                &self.fence.handle(),
                vk::VK_TRUE,
                TIMEOUT_NS,
            )
        };
        // ★ T3 review I2：**超时 ≠ 完成**。`resolve` 成功才回到可复用态，失败即 Broken
        //   （之后任何 render 都会先被 `ensure_reusable` 拦下）。
        self.sync.resolve(rc)
    }

    /// map 暂存缓冲 → 拷出 → unmap（`Mapped` 守卫保证 unmap 一定会发生，见 F9）。
    fn read_back(&self) -> GpuResult<Vec<u8>> {
        let mapped = map_memory(
            "vkMapMemory(staging)",
            &self.fns,
            self.device_handle,
            self.staging_memory.handle(),
        )?;
        // SAFETY: 映射了整块缓冲；长度取自缓冲大小。
        let out = unsafe {
            std::slice::from_raw_parts(mapped.as_mut_ptr() as *const u8, self.staging_size as usize)
                .to_vec()
        };
        Ok(out)
    }
}

/// 颜色附件的子资源范围（只有一个 mip / 一层）。
fn color_range() -> vk::ImageSubresourceRange {
    vk::ImageSubresourceRange {
        aspect_mask: vk::VK_IMAGE_ASPECT_COLOR_BIT,
        base_mip_level: 0,
        level_count: 1,
        base_array_layer: 0,
        layer_count: 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **I2 的回归判据**：栅栏等待失败（例如超时）之后，渲染器必须**拒绝复用**。
    ///
    /// 为什么能在无 GPU 的单元测试里跑：这段逻辑是纯状态机（`SubmitState`），
    /// 真正的 `vkWaitForFences` 只提供返回码。跑在 `cargo test -p deer-vk --lib` 里，
    /// 不需要任何 Vulkan 环境。
    #[test]
    fn a_failed_fence_wait_makes_the_renderer_unusable() {
        let mut s = SubmitState::default();
        assert_eq!(s, SubmitState::Idle);
        assert!(s.ensure_reusable().is_ok(), "空闲态可复用");

        s.begin();
        assert_eq!(s, SubmitState::InFlight);

        // VK_TIMEOUT = 2 —— 这就是「超时」：**绝不能**当成完成。
        let err = s.resolve(2).expect_err("超时必须报错");
        let msg = format!("{err}");
        assert!(msg.contains("2"), "错误里要带真实返回码：{msg}");
        assert_eq!(s, SubmitState::Broken);
        assert!(
            s.ensure_reusable().is_err(),
            "超时之后**不许**再碰命令缓冲/顶点缓冲 —— 旧实现会在这里继续跑"
        );
    }

    /// **I1 的回归判据（fix round 2 / R1-1）**：屏障的四个掩码必须**确切**是
    /// `HOST` / `VERTEX_INPUT(0x4)` / `HOST_WRITE` / `VERTEX_ATTRIBUTE_READ(0x4)`。
    ///
    /// 变异验证：把 `VK_PIPELINE_STAGE_VERTEX_INPUT_BIT` 改成 `1 << 5`（细分求值）⇒ 本测试**变红**。
    /// 之前没有这条断言时，那个变异在 16 个测试靶上全绿（只有校验层刷 VUID-04091）。
    #[test]
    fn vertex_barrier_params_pin_the_exact_masks() {
        let p = vertex_buffer_barrier_params();
        assert_eq!(p.src_stage, 1 << 14, "srcStageMask 必须是 VK_PIPELINE_STAGE_HOST_BIT");
        assert_eq!(
            p.dst_stage, 0x4,
            "dstStageMask 必须是 VK_PIPELINE_STAGE_VERTEX_INPUT_BIT = 0x4 = 1<<2"
        );
        assert_eq!(p.src_access, 1 << 14, "srcAccessMask 必须是 VK_ACCESS_HOST_WRITE_BIT");
        assert_eq!(
            p.dst_access, 0x4,
            "dstAccessMask 必须是 VK_ACCESS_VERTEX_ATTRIBUTE_READ_BIT = 0x4 = 1<<2"
        );
        // 把「易错的那个值」显式钉出来：1<<5 是**细分求值**阶段，不是顶点输入。
        assert_ne!(p.dst_stage, 1 << 5, "1<<5 = VK_PIPELINE_STAGE_TESSELLATION_EVALUATION_SHADER_BIT");
        assert_ne!(p.dst_access, 1 << 5, "1<<5 = VK_ACCESS_SHADER_READ_BIT");
    }

    /// **M1 护栏**：形状路径报出的「未支持」必须被**收集**（而不是被丢掉）。
    ///
    /// 形状路径今天不会产出 `unsupported`（`gpu_geom` 什么都翻译得了）⇒ 这条策略**不可达**。
    /// 不可达的分支最容易被后人删掉而没有测试发现（review M1 的原话：今天等价、将来会静默少画），
    /// 所以这里直接喂合成消息钉住「收集」这一步的行为。
    #[test]
    fn absorb_unsupported_keeps_every_message_in_order() {
        let mut acc: Vec<String> = Vec::new();
        absorb_unsupported(&mut acc, &[]);
        assert!(acc.is_empty(), "空清单不该产生任何条目");

        absorb_unsupported(&mut acc, &["形状：某条命令未支持".to_string()]);
        absorb_unsupported(&mut acc, &["文本：另一条".to_string()]);
        assert_eq!(
            acc,
            vec![
                "形状：某条命令未支持".to_string(),
                "文本：另一条".to_string()
            ],
            "两个管线的消息都要按出现顺序留下（丢了就是静默少画）"
        );
    }

    /// **M1 护栏**：帧末的判定 —— 空 ⇒ `Ok`；非空 ⇒ `Unsupported` 且消息带在错误里。
    #[test]
    fn unsupported_outcome_errors_iff_there_are_messages() {
        assert!(unsupported_outcome(&[]).is_ok(), "没有消息 ⇒ 这一帧不算失败");

        let e = unsupported_outcome(&["形状：未支持".to_string(), "文本：未支持".to_string()])
            .expect_err("有消息就必须报错（否则等于静默少画）");
        let msg = format!("{e}");
        assert!(
            msg.contains("形状：未支持") && msg.contains("文本：未支持"),
            "两条消息都要出现在错误里（用「；」连接）：{msg}"
        );
    }

    /// **M3 护栏**：`vertex_range` 的边界与错误信息（循环里改用它是为了「不提前 return」）。
    #[test]
    fn vertex_range_converts_and_rejects_overflow() {
        assert_eq!(vertex_range(0, 6).expect("正常值"), (0, 6));
        assert_eq!(vertex_range(1024, 0).expect("零长度也合法"), (1024, 0));
        assert_eq!(
            vertex_range(u32::MAX as usize, 1).expect("刚好放得下"),
            (u32::MAX, 1)
        );
        // 溢出：两侧都要报 `Unsupported`（不是 panic、也不是截断）
        assert!(vertex_range(u32::MAX as usize + 1, 1).is_err(), "偏移溢出必须报错");
        assert!(vertex_range(0, u32::MAX as usize + 1).is_err(), "数量溢出必须报错");
        let e = vertex_range(0, u32::MAX as usize + 1).expect_err("应报错");
        assert!(
            format!("{e}").contains("超出 u32"),
            "错误信息要说清是「超出 u32」：{e}"
        );
    }

    /// 一次成功的帧之后可以继续画（否则第一帧之后就全废了）。
    #[test]
    fn a_successful_fence_wait_restores_reusability() {
        let mut s = SubmitState::default();
        s.begin();
        s.resolve(ffi::VK_SUCCESS).expect("成功不该报错");
        assert_eq!(s, SubmitState::Idle);
        assert!(s.ensure_reusable().is_ok());
    }

    /// 提交阶段别的错误（例如 `vkQueueSubmit` 失败）同样不敢假设资源空闲。
    #[test]
    fn other_submit_errors_also_poison_the_state() {
        let mut s = SubmitState::default();
        s.begin();
        s.mark_broken();
        assert!(s.ensure_reusable().is_err());
    }

    /// **I1 的前提**：主机缓冲区**只**选相干内存（所以不需要 `vkFlushMappedMemoryRanges`；
    /// 那支符号本项目还没有，见模块文档「同步①」）。
    #[test]
    fn host_buffers_never_use_non_coherent_memory() {
        // 类型 0 = DEVICE_LOCAL（不可见）、类型 1 = HOST_VISIBLE 但**不相干**、
        // 类型 2 = HOST_VISIBLE | HOST_COHERENT。
        let mut props = vk::PhysicalDeviceMemoryProperties {
            memory_type_count: 3,
            memory_types: [vk::MemoryType {
                property_flags: 0,
                heap_index: 0,
            }; 32],
            memory_heap_count: 1,
            memory_heaps: [vk::MemoryHeap {
                size: 1 << 30,
                flags: 0,
            }; 16],
        };
        props.memory_types[0].property_flags = vk::VK_MEMORY_PROPERTY_DEVICE_LOCAL_BIT;
        props.memory_types[1].property_flags = vk::VK_MEMORY_PROPERTY_HOST_VISIBLE_BIT;
        props.memory_types[2].property_flags =
            vk::VK_MEMORY_PROPERTY_HOST_VISIBLE_BIT | vk::VK_MEMORY_PROPERTY_HOST_COHERENT_BIT;

        let want =
            vk::VK_MEMORY_PROPERTY_HOST_VISIBLE_BIT | vk::VK_MEMORY_PROPERTY_HOST_COHERENT_BIT;
        // 三种都可选时，必须挑到相干的那个（类型 2），**不能**挑类型 1。
        assert_eq!(pick_memory_type(&props, 0b111, want).unwrap(), 2);
        // 只有不相干的主机可见内存时（类型 1）：宁可报错，也不静默用非相干内存
        // —— 非相干内存必须配 `vkFlushMappedMemoryRanges`，而本项目没有那支符号。
        assert!(
            pick_memory_type(&props, 0b010, want).is_err(),
            "拿不到相干内存时必须明确报错，而不是静默降级"
        );
    }
}
