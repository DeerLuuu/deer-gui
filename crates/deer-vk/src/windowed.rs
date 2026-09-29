//! **窗口出图链**（M2b）：把「实例 + surface + 设备 + 交换链 + 渲染通道 + 管线 + 同步 + 呈现」
//! 装成一条能跑的链。
//!
//! ## 一帧的完整流程
//!
//! ```text
//!  ① 等这一槽的 in-flight 栅栏（上一帧真的做完了才复用它的命令缓冲/信号量）
//!  ② vkAcquireNextImageKHR(image_available[slot])
//!      ├ VK_SUCCESS            → 拿到图像索引
//!      ├ VK_SUBOPTIMAL_KHR     → 索引同样有效：照常画+呈现，然后上报 OutOfDate（要重建）
//!      ├ VK_ERROR_OUT_OF_DATE_KHR → 直接返回 FrameOutcome::OutOfDate（**不 reset 栅栏**）
//!      └ 其它（TIMEOUT/NOT_READY/…）→ 返回 Err（**绝不当成功**）
//!  ③ 只有确定要提交时才 reset 栅栏（否则②的失败路径会留下永不信号的栅栏 ⇒ 下一帧死等）
//!  ④ 录制：beginRenderPass(CLEAR=调用方给的颜色) → bindPipeline → setViewport/Scissor → draw → end
//!     （若开着回读：再用两次屏障把图像复制到主机可见缓冲，**复制必须在 present 之前**）
//!  ⑤ vkQueueSubmit（等 image_available，信号 render_finished，挂 in-flight 栅栏）
//!  ⑥ vkQueuePresentKHR（等 render_finished）
//! ```
//!
//! ## 同步对象为什么这么配
//!
//! - `image_available`：**每帧槽一个**。复用它之前一定等过该槽的栅栏 ⇒ 上一次的等待已经结束。
//! - `render_finished`：**每张交换链图像一个**。呈现引擎等它，只有重新 acquire 到同一张图像
//!   才说明呈现引擎放开了它 —— 这是「不能用一个信号量连续呈现两次」的标准解法。
//! - in-flight 栅栏：**每帧槽一个**，创建时就是 signaled（第一帧的 wait 立即返回）。
//!
//! ## 回读（M2b 补强）
//!
//! [`WindowedRenderer::read_back_last_frame`] 能把**呈现出去的那一帧**读成 RGBA8 像素，
//! 于是「窗口里显示的确实是要求的颜色」有了**像素级**证据（而不只是「呈现了 N 帧」）。
//! 实现在 present **之前**把图像 `vkCmdCopyImageToBuffer` 进一块 `HOST_VISIBLE|HOST_COHERENT`
//! 缓冲（present 之后图像所有权归呈现引擎，不能再碰），布局
//! `PRESENT_SRC_KHR ⇄ TRANSFER_SRC_OPTIMAL` 往返后交还 `PRESENT_SRC_KHR`。
//!
//! ## 本轮的诚实边界（M3 才做）
//!
//! 窗口里显示的是**清屏色 + 一个三角形**（M2a 已验证的几何），**不是**界面：
//! 把 `DrawList`（含真实字形）送上 GPU 属于 M3。同理未做：mailbox 呈现模式、
//! 独占全屏、HDR/色彩空间选择、帧率限制、深度/多重采样。

use std::ffi::c_void;
use std::ptr;

use deer_gpu::{
    AdapterInfo, Color, DrawCmd, DrawList, Extent, GpuError, GpuResult, RawWindowHandle, RectI,
    TextEngine,
};

use crate::device::{
    vk_result_name, DescriptorPool, DescriptorSet, Pipeline, PipelineLayout, RenderPass,
    ShaderModule, Texture, VkDevice,
};
use crate::ffi;
use crate::ffi_dev as vk;
use crate::gpu_geom::{self, GpuVertex};
use crate::gpu_text::{self, TextVertex};
use crate::pipelines;
use crate::spirv;
use crate::surface::{self, Surface};
use crate::swapchain::{
    readback_row_pitch, reorder_to_rgba8, Acquire, Present, Semaphore, Swapchain,
};

/// `VkFenceCreateFlagBits::VK_FENCE_CREATE_SIGNALED_BIT`
/// （`ffi_dev.rs` 里没有它 —— M2a 的栅栏都是「不预设 signaled ⇒ 必须真的等 GPU」）。
/// 这里需要「初始就 signaled」，否则第一帧的 `wait` 会白等一个超时。
const VK_FENCE_CREATE_SIGNALED_BIT: u32 = 0x0000_0001;

/// `VK_PIPELINE_STAGE_HOST_BIT`。`ffi_dev.rs` 里没有这个常量（M2a 只需要 PRESENT/COLOR 那几个），
/// 而窗口路径的界面绘制要发一条 host→vertex 的缓冲区屏障 ⇒ 在这里具名钉住。
///
/// 用**具名常量 + 单测**而不是裸写 `1 << 14`：这四个值（stage 两个、access 两个）里
/// `1 << 2` 与 `1 << 5` 曾经被写混过（后者是细分求值/shader read），而校验层之外
/// 没有任何东西会报错 —— 只有把确切数字钉住才能回归。
const VK_PIPELINE_STAGE_HOST: u32 = 1 << 14;
/// `VK_PIPELINE_STAGE_VERTEX_INPUT_BIT`
const VK_PIPELINE_STAGE_VERTEX_INPUT: u32 = 1 << 2;
/// `VK_ACCESS_HOST_WRITE_BIT`
const VK_ACCESS_HOST_WRITE: u32 = 1 << 14;
/// `VK_ACCESS_VERTEX_ATTRIBUTE_READ_BIT`
const VK_ACCESS_VERTEX_ATTRIBUTE_READ: u32 = 1 << 2;

// ── 间接绘制（M3+ 第 4 项下半）需要的同步位 ────────────────────────────────────
//
// 与上面四个同一处境（`ffi_dev.rs` 不在允许改动清单里 ⇒ 具名常量放在使用者旁边，
// 值取自本机 SDK `1.4.357.0/include/vulkan/vulkan_core.h`，并由单测钉住确切数字）。
// **两组数值刻意不同**：索引在 `VERTEX_INPUT` 阶段被读、间接命令在 `DRAW_INDIRECT`
// 阶段被读 —— 写混不会报错，只会让屏障不覆盖真正读它的那一步。

/// `VK_PIPELINE_STAGE_DRAW_INDIRECT_BIT`（`0x2`）：读**间接命令**的阶段。
const VK_PIPELINE_STAGE_DRAW_INDIRECT: u32 = 1 << 1;
/// `VK_ACCESS_INDIRECT_COMMAND_READ_BIT`（`0x1`）。
const VK_ACCESS_INDIRECT_COMMAND_READ: u32 = 1 << 0;
/// `VK_ACCESS_INDEX_READ_BIT`（`0x2`；与 `DRAW_INDIRECT` 数值相同但属于另一套枚举）。
const VK_ACCESS_INDEX_READ: u32 = 1 << 1;

/// 同时在飞的帧数（也是每帧槽命令缓冲/信号量/栅栏的数量）。///
/// 取 2 的理由：1 会让 CPU 每帧都等 GPU（吞吐掉一半），3 以上对 GUI 没有额外收益
/// 却要多一份命令缓冲与延迟。
pub const FRAMES_IN_FLIGHT: usize = 2;

/// 等 in-flight 栅栏的上限。有限超时的理由同 [`crate::swapchain::DEFAULT_ACQUIRE_TIMEOUT_NS`]：
/// 驱动出问题时**宁可失败也不要永久挂住**（一个挂住的 GUI 进程比一个报错的更难查）。
const FENCE_TIMEOUT_NS: u64 = 5_000_000_000;

/// 三角形的 NDC 顶点（M2a 已验证可画出像素的那组）。
const TRIANGLE_NDC: [[f32; 2]; 3] = [[-0.6, -0.55], [0.6, -0.55], [0.0, 0.6]];
/// 三角形颜色：与默认清屏色明显不同，肉眼一眼能看出「GPU 真的画了」。
const TRIANGLE_COLOR: [f32; 4] = [0.35, 0.62, 1.0, 1.0];

/// 一帧的结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameOutcome {
    /// 这一帧已提交呈现。
    Presented,
    /// 交换链已过期/不再最优 ⇒ 调用方 `resize` 后重试。
    OutOfDate,
}

// ===========================================================================
// M3c-T3：窗口里画**真实界面树**（形状 + 文本）所需的资源
// ===========================================================================

// 绘制段类型与「形状/文本」两条管线的身份都取自**共用的一份**
// （`gpu_render::{DrawCall, PipelineKind}`）：窗口与离屏的「顺序即 z 序」表达必须逐字相同，
// 各写一份就是「只改了一边」的温床（M3c 收敛过一次，这里沿用同一条规矩）。
use crate::device::DrawIndexedIndirectCommand;
use crate::gpu_render::{DrawCall, PipelineKind, RenderStats};

/// 顶点缓冲（host 可见 + coherent；窗口路径每帧重传）。
struct UiVertexBuffer {
    buffer: OwnedBuffer,
    memory: OwnedMemory,
    capacity: u64,
}

/// 界面树所需的全部**渲染器级**资源（M3c 建立；**M3+ B5-2 起是单管线**）。
///
/// 形状与文本**合流成一条统一顶点流 + 一条统一管线**（见
/// [`crate::vertex_unify::unify`] 与 [`crate::spirv::vertex_shader_unified`]）
/// ⇒ 这里只有**一块**顶点缓冲、**一条**界面管线、**一个**必绑的描述符集。
///
/// ## 字段顺序（**是契约，不是风格**）
///
/// 1. `set` 必须声明在 `pool` 之前 —— `DescriptorSet::drop` 会调
///    `vkFreeDescriptorSets(device, pool, ..)` ⇒ **池必须比集活得久**，
///    而 Rust 按声明顺序析构（先声明的先销毁）⇒ 集先销、池后销；
/// 2. `vb` 是 `Option<UiVertexBuffer>`，其中 `buffer` 声明在 `memory` 之前
///    （缓冲先销毁、内存后释放）。
///
/// ⚠️ 第 1 条在 B5-2 期间**实测踩过**：顺序写反 ⇒ 测试逻辑全通过但进程在退出时
/// `STATUS_ACCESS_VIOLATION`（`vkFreeDescriptorSets` 访问已销毁的池）。
/// 校验层能说清（`descriptorPool Invalid VkDescriptorPool Object`），
/// **但关掉校验层时这条消息不存在** ⇒ 症状像「测试框架自己崩了」。
struct UiResources {
    /// 统一管线需要的**共用资源**（管线布局 / `set 0` 布局 / 采样器）。
    ///
    /// ⚠️ **声明在 `unified` 之前**：统一管线建在这份布局上 ⇒ 管线必须先销毁
    /// （B5-3 之前这里装的是「形状 + 文本两条旧管线」，它们已删 —— 见
    /// `pipelines::PipelineResources` 的说明）。
    pipes: pipelines::PipelineResources,
    /// **统一**顶点缓冲（形状 + 文本共用一条流）。
    vb: Option<UiVertexBuffer>,
    /// 统一界面管线（`vertex_shader_unified` / `fragment_shader_unified`，stride 52）。
    unified: Pipeline,
    /// 统一管线的两个着色器模块（**只为所有权**：必须比 `unified` 活得久）。
    #[allow(dead_code)]
    unified_vs: ShaderModule,
    #[allow(dead_code)]
    unified_fs: ShaderModule,
    /// `set 0 / binding 0`：**恒有效**（统一 FS 无条件采样）。
    ///
    /// **必须在 `pool` 之前声明**（见类型文档的字段顺序契约）。
    set: DescriptorSet,
    /// 描述符池：**只为所有权而持有**（`set` 的 `Drop` 会调
    /// `vkFreeDescriptorSets(device, pool, ..)` ⇒ 池必须比集活得久）。
    #[allow(dead_code)]
    pool: DescriptorPool,
    /// **1×1 全覆盖率哑纹理**：没有文本引擎（或那帧没有图集可传）时 `set` 指向它。
    ///
    /// 见 `gpu_render::DUMMY_COVERAGE` 的说明：统一片元着色器**无条件采样**
    /// ⇒ 形状帧也必须有一个有效的 `COMBINED_IMAGE_SAMPLER`。
    ///
    /// ⚠️ 字段**从不被读取** —— 它存在只为**所有权**：`set` 的描述符指向它的
    /// 图像视图，视图/图像/内存必须比那次描述符写入活得久（否则采样读到已释放的图像）。
    /// 删掉它 = 采样悬垂视图。`#[allow(dead_code)]` 是这里的正确表达。
    #[allow(dead_code)]
    dummy_texture: Texture,
    /// 当前已上传的**字形图集**（`None` = 还没传过 ⇒ `set` 指向 `dummy_texture`）。
    texture: Option<Texture>,
    /// **描述符集此刻指着哪张纹理**（宽, 高）—— [`WindowedRenderer::bound_texture_size`]
    /// 的**唯一**来源。
    ///
    /// ## 为什么要单独一个字段（与 `gpu_render.rs` 同一条纪律）
    ///
    /// 从 `texture` / `uploaded` **推**出来的读数会名不副实：变异「跳过
    /// `update_descriptor_texture`（忘了把 `set` 改指到图集）」下 `texture` 照样被更新
    /// ⇒ 读数照旧报图集，断言咬不住（复审在离屏侧实测过这一条）。
    /// 现在它**只能**由 `crate::gpu_render::point_descriptor_at` 的返回值赋值 ——
    /// 那个函数体内就是 `vkUpdateDescriptorSets` 的调用点 ⇒ 读数与副作用同处。
    descriptor_points_at: (u32, u32),
    /// 已上传图集的指纹 `(宽, 高, 已光栅化字形数)`；`None` = 还没传过。
    ///
    /// `Some` 的指纹**不可能**是 `(1,1,0)`：图集宽 = 字号 ≥ 2 ⇒ 不会与哑纹理混淆。
    uploaded: Option<(u32, u32, usize)>,
    /// **上一次实际上传的字节 + 当时那块缓冲的句柄**（M3+ B3）。
    ///
    /// ⚠️ **记句柄是关键（review I-1）**：跳过重传的条件是「**同一块缓冲** + 字节相同」；
    /// 缓冲一旦换新（容量增长、`release_ui_resources()` 之后重建），句柄就不匹配 ⇒
    /// **自动作废**，不依赖任何调用方记得清记录。
    uploaded_vertices: Option<(vk::BufferHandle, Vec<u8>)>,
    /// **索引缓冲 + 间接命令缓冲**（M3+ 第 4 项下半：间接绘制必需，与离屏同一条设计）。
    ///
    /// 惰性创建 + 跨帧复用；**只在顶点数变化时重传**（索引是 `0..N`、命令里只有
    /// `indexCount` 随 N 变）。声明在 `pipes`/`unified` 之前 ⇒ 先销毁缓冲、再销毁管线。
    index: Option<UiVertexBuffer>,
    indirect: Option<UiVertexBuffer>,
    /// 上一次写进索引缓冲的**顶点数 + 缓冲句柄**（句柄一变即作废，与 `uploaded_vertices` 同款）。
    uploaded_indices: Option<(vk::BufferHandle, u32)>,
    /// 上一次写进间接缓冲的**命令 + 缓冲句柄**。
    uploaded_indirect: Option<(vk::BufferHandle, DrawIndexedIndirectCommand)>,
}

/// 存活中的 [`UiResources`] 实例数（进程级）。
///
/// ## 为什么需要这个计数器（控制者的要求：`resize` 失效重建**不能泄漏**）
///
/// `resize()` 会把 `self.ui` 置 `None` 让旧资源析构、下一帧再重建。这条路径不能泄漏
/// 管线/描述符集/纹理 —— 但「代码看起来对」不是证据（本项目已经吃过「实现有、护栏没有」的亏）。
/// 所以：构造时 +1、`Drop` 时 −1（记在**类型自己**身上，不靠调用方上报），
/// 断言写成「反复 resize 之后：存活数**恒为 1**、构建次数**恰好 = 1 + 重建次数**」。
/// 漏了 `Drop`、或旧资源被遗忘（没置 `None`）都会红。
static LIVE_UI_RESOURCES: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

impl Drop for UiResources {
    fn drop(&mut self) {
        LIVE_UI_RESOURCES.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
    }
}

/// 当前存活的界面资源数（进程级；示例用它抓「rebuild 泄漏」）。
pub fn live_ui_resource_count() -> usize {
    LIVE_UI_RESOURCES.load(std::sync::atomic::Ordering::SeqCst)
}

/// viewport/scissor 策略。
///
/// - **动态（默认）**：M2b 起窗口路径一直这么用，实测能上屏；尺寸变化不必重建管线；
/// - **静态**：写进管线（`DEER_VK_WINDOW_VIEWPORT=static` 强制）。
///
/// 两种都能跑出来，是为了**把 M2a 留下的那句「动态在本机 Intel 上画不出像素」查清**。
///
/// **实现事实**（与那句结论不冲突）：管线一旦声明动态 viewport/scissor，**录制时必须**
/// 调 `vkCmdSetViewport`/`vkCmdSetScissor` —— 否则是规范里的未定义行为。
///
/// **本轮的证据（只覆盖窗口路径）**：补上设置调用后，动态与静态**都能上屏**（各 30 帧、
/// 界面像素完全相同）；而**去掉**设置调用会直接 `0xC000041D` 崩溃。
///
/// **存疑**：M2a 当时测的是**离屏 + 三角形管线**，症状是「零像素、不崩」而不是崩溃
/// ⇒ **症状不同不能断定同因**。所以那句结论既不能照旧当硬前提，也还没到能宣布「已证实」
/// 的时候 —— 离屏的复现是一件**文档待办**。
///
/// 环境变量只作为**诊断/实测开关**，默认值才是产品行为。
///
/// **判定先 `trim()` 再比**：`cmd /c "set DEER_VK_WINDOW_VIEWPORT=static && …"` 会把 `&&` 前的
/// 空格也算进变量值（实测 `"static "`）⇒ 严格判等会**静默退回默认 `dynamic`**：以为在测静态、
/// 实际测的是动态（默认值才是产品行为，所以症状完全看不出来）。同一条实测与更完整的理由
/// 见 [`crate::ffi::env_flag`]。断言见本文件 `mod tests` 的 `viewport_strategy_from_value_*`。
pub fn viewport_strategy_from_env() -> pipelines::ViewportStrategy {
    viewport_strategy_from_value(std::env::var("DEER_VK_WINDOW_VIEWPORT").ok().as_deref())
}

/// [`viewport_strategy_from_env`] 的**纯逻辑**部分（不读环境 ⇒ 可单测）。
///
/// 未设、空值或无法识别的值 ⇒ **默认 `dynamic`**（默认值才是产品行为）。
pub fn viewport_strategy_from_value(v: Option<&str>) -> pipelines::ViewportStrategy {
    match v.map(str::trim) {
        Some(v) if v.eq_ignore_ascii_case("static") => pipelines::ViewportStrategy::Static {
            width: 1,
            height: 1,
        },
        _ => pipelines::ViewportStrategy::Dynamic,
    }
}

/// 把「当前生效的裁剪栈 + 这一条命令」组成一个临时 `DrawList`（**保 z 序**的关键）。
///
/// **实现已收敛到共用的一份**（`gpu_render::single_command_in_clip`）：离屏与窗口两条渲染
/// 路径的「逐命令翻译 + 裁剪栈重放」必须逐字相同 —— 复制两份就是「只改了一边」的温床。
/// 这里保留一个薄别名，只为让窗口路径的调用点读起来直白。
fn ui_single_command_in_clip(active_clip: &[RectI], cmd: &DrawCmd) -> DrawList {
    crate::gpu_render::single_command_in_clip(active_clip, cmd)
}

/// 建（或扩容）一个 host 可见的缓冲。**先建新的、成功后再换**（失败不破坏旧状态）。
///
/// `stats.buffer_allocations` 在**真正创建**的那一步自增（与 `vkCreateBuffer` 同处）——
/// 计数放被调用方，删掉创建就必然删掉计数。
///
/// `usage` 由调用方给（顶点 / 索引 / 间接三种）—— M3+ 第 4 项下半起这块结构同时承载三者。
fn ensure_ui_vertex_capacity(
    device: &VkDevice,
    slot: &mut Option<UiVertexBuffer>,
    bytes: u64,
    stats: &mut RenderStats,
    usage: u32,
    what: &'static str,
) -> GpuResult<bool> {
    if slot.as_ref().is_some_and(|v| v.capacity >= bytes) {
        return Ok(false);
    }
    let capacity = bytes.next_power_of_two().max(4096);
    let (buffer, memory) = create_host_vertex_buffer(device, capacity, usage, what)?;
    *slot = Some(UiVertexBuffer {
        buffer,
        memory,
        capacity,
    });
    // 与 `vkCreateBuffer` + `vkBindBufferMemory` 同处
    stats.buffer_allocations += 1;
    // 返回「重建了」⇒ 调用方必须清掉"上次上传的字节"（新缓冲里没有可信数据）
    Ok(true)
}

/// 发一条「主机写缓冲 → GPU 读缓冲」的屏障（`src` 恒为 `HOST`/`HOST_WRITE`，
/// 目标阶段/访问位由调用方给）。
///
/// 抽出来的理由与离屏侧的 `emit_host_buffer_barrier` 相同：三种缓冲各需要一条，
/// 而目标掩码**不同**（顶点/索引在 `VERTEX_INPUT`、间接命令在 `DRAW_INDIRECT`）。
/// 三份字面量屏障正是「写错不报错、屏障照样被接受」的高发形态 ⇒ 集中到一处并让
/// 单测钉住确切数字。
fn emit_ui_buffer_barrier(
    cmd: vk::CommandBufferHandle,
    fns: crate::device::DeviceFns,
    buffer: vk::BufferHandle,
    dst_stage: u32,
    dst_access: u32,
) {
    let barrier = vk::BufferMemoryBarrier {
        s_type: vk::VK_STRUCTURE_TYPE_BUFFER_MEMORY_BARRIER,
        p_next: ptr::null(),
        src_access_mask: VK_ACCESS_HOST_WRITE,
        dst_access_mask: dst_access,
        src_queue_family_index: vk::QUEUE_FAMILY_IGNORED,
        dst_queue_family_index: vk::QUEUE_FAMILY_IGNORED,
        buffer,
        offset: 0,
        size: vk::WHOLE_SIZE,
    };
    // SAFETY: 结构体在栈上存活；命令缓冲处于录制状态；`buffer` 是本结构持有的有效句柄。
    unsafe {
        (fns.cmd_pipeline_barrier)(
            cmd,
            VK_PIPELINE_STAGE_HOST,
            dst_stage,
            0,
            0,
            ptr::null(),
            1,
            &barrier,
            0,
            ptr::null(),
        );
    }
}

/// 建一个 host-visible/coherent 缓冲（用法位由调用方给）。
fn create_host_vertex_buffer(
    device: &VkDevice,
    size: u64,
    usage: u32,
    what: &'static str,
) -> GpuResult<(OwnedBuffer, OwnedMemory)> {
    let fns = device.fns();
    let info = vk::BufferCreateInfo {
        s_type: vk::VK_STRUCTURE_TYPE_BUFFER_CREATE_INFO,
        p_next: ptr::null(),
        flags: 0,
        size,
        usage,
        sharing_mode: vk::VK_SHARING_MODE_EXCLUSIVE,
        queue_family_index_count: 0,
        p_queue_family_indices: ptr::null(),
    };
    let mut handle: vk::BufferHandle = ptr::null_mut();
    // SAFETY: `info` 在栈上存活；`handle` 是可写输出。
    let rc = unsafe { (fns.create_buffer)(device.handle(), &info, ptr::null(), &mut handle) };
    if rc != ffi::VK_SUCCESS {
        return Err(GpuError::Driver {
            code: rc,
            message: format!("vkCreateBuffer（{what} {size} 字节）失败：{}", vk_result_name(rc)),
        });
    }
    let buffer = OwnedBuffer {
        handle,
        device: device.handle(),
        destroy: fns.destroy_buffer,
    };
    let mut req = std::mem::MaybeUninit::<vk::MemoryRequirements>::uninit();
    // SAFETY: 该函数完整写入结构体；缓冲刚创建且有效。
    unsafe { (fns.get_buffer_memory_requirements)(device.handle(), buffer.handle(), req.as_mut_ptr()) };
    let req = unsafe { req.assume_init() };
    let memory_type_index = pick_memory_type(
        device.memory_properties(),
        req.memory_type_bits,
        vk::VK_MEMORY_PROPERTY_HOST_VISIBLE_BIT | vk::VK_MEMORY_PROPERTY_HOST_COHERENT_BIT,
    )?;
    let alloc = vk::MemoryAllocateInfo {
        s_type: vk::VK_STRUCTURE_TYPE_MEMORY_ALLOCATE_INFO,
        p_next: ptr::null(),
        allocation_size: req.size,
        memory_type_index,
    };
    let mut mem: vk::DeviceMemoryHandle = ptr::null_mut();
    // SAFETY: `alloc` 在栈上存活；`mem` 是可写输出。
    let rc = unsafe { (fns.allocate_memory)(device.handle(), &alloc, ptr::null(), &mut mem) };
    if rc != ffi::VK_SUCCESS {
        return Err(GpuError::Driver {
            code: rc,
            message: format!(
                "vkAllocateMemory（顶点缓冲，类型 {memory_type_index}）失败：{}",
                vk_result_name(rc)
            ),
        });
    }
    let memory = OwnedMemory {
        handle: mem,
        device: device.handle(),
        free: fns.free_memory,
    };
    // SAFETY: 缓冲与内存都是本函数刚创建的对象；兼容性由 memory_type_bits 保证；offset 0 合法。
    let rc = unsafe {
        (fns.bind_buffer_memory)(device.handle(), buffer.handle(), memory.handle(), 0)
    };
    if rc != ffi::VK_SUCCESS {
        return Err(GpuError::Driver {
            code: rc,
            message: format!("vkBindBufferMemory（顶点缓冲）失败：{}", vk_result_name(rc)),
        });
    }
    Ok((buffer, memory))
}

/// 把一段 `#[repr(C)]` 纯 `f32` 顶点数据写进缓冲（map → memcpy → unmap）。
///
/// `stats.buffer_uploads` 在 memcpy 之后自增（与那次主机写入同处）——计数放被调用方。
fn upload_ui_vertices(
    device: &VkDevice,
    vb: &UiVertexBuffer,
    src: &[u8],
    what: &str,
    stats: &mut RenderStats,
) -> GpuResult<()> {
    let fns = device.fns();
    let mut mapped: *mut c_void = ptr::null_mut();
    // SAFETY: 内存是本设备对象、HOST_VISIBLE；`mapped` 是可写输出；映射长度由实现取整
    //（`vkMapMemory` 的 `size = WHOLE_SIZE` ⇒ 整块，够 `src.len()`）。
    let rc = unsafe {
        (fns.map_memory)(
            device.handle(),
            vb.memory.handle(),
            0,
            vk::WHOLE_SIZE,
            0,
            &mut mapped,
        )
    };
    if rc != ffi::VK_SUCCESS {
        return Err(GpuError::Driver {
            code: rc,
            message: format!("{what}：vkMapMemory 失败：{}", vk_result_name(rc)),
        });
    }
    // SAFETY: 映射了整块缓冲（≥ `src.len()`，由 `ensure_ui_vertex_capacity` 保证）；
    // 源与目标不重叠；`mapped` 非空（上面检查过返回值）。
    unsafe {
        ptr::copy_nonoverlapping(src.as_ptr(), mapped as *mut u8, src.len());
        // HOST_COHERENT ⇒ 不需要 flush，直接解映射。
        (fns.unmap_memory)(device.handle(), vb.memory.handle());
    }
    // 与那次主机写入（memcpy）同处
    stats.buffer_uploads += 1;
    Ok(())
}

/// HAL 颜色（0–255 + alpha 0.0–1.0）→ Vulkan 的 `VkClearColorValue.float32`。
///
/// **纯函数**且**不做任何"美化"**（不 clamp、不做 sRGB 变换、不替换通道顺序）——
/// 「传进去的颜色就是清屏色」这条约定必须能被单测逐位钉住，否则示例里那句
/// 「窗口里就是这个颜色」的断言就站不住。
pub fn clear_color_value(color: Color) -> [f32; 4] {
    [
        color.r as f32 / 255.0,
        color.g as f32 / 255.0,
        color.b as f32 / 255.0,
        color.a,
    ]
}

/// 线性通道值（0–255）→ **sRGB 附件里实际写入的字节**。
///
/// ## 为什么需要它（实测纠正了一个想当然的假设）
///
/// 交换链格式是 `B8G8R8A8_SRGB`（本机就是），而 Vulkan 规定：写入 sRGB 附件的
/// **颜色值在线性空间**，驱动负责做 sRGB 编码。所以：
///
/// ```text
///   clear = rgb(0x10, 0x14, 0x24)   ← 线性
///   ⇒ 回读到的字节 = sRGB 编码后的 [0x47, 0x4F, 0x69] = [71, 79, 105]
///   （不是「直通」的 [16, 20, 36] —— 第一版断言按直通写，实测假红）
/// ```
///
/// 这条纯函数就是那个编码公式（sRGB 传输函数），用来给回读断言提供**正确参考值**。
/// 它同时是一份证据：本机 Intel 与 NVIDIA 驱动的实测字节都与它逐位一致。
///
/// 想要「逐字节直通」的调用方只能用**线性格式**（`*_UNORM`）的交换链 —— 那是能力选择问题，
/// 不是回读问题。
pub fn srgb_encoded_byte(linear: u8) -> u8 {
    let x = linear as f64 / 255.0;
    // sRGB EOTF 的逆（线性 → 编码），阈值 0.0031308 是规范给的分段点
    let y = if x <= 0.003_130_8 {
        12.92 * x
    } else {
        1.055 * x.powf(1.0 / 2.4) - 0.055
    };
    (y * 255.0).round().clamp(0.0, 255.0) as u8
}

/// 帧缓冲（私有 RAII：`offscreen.rs` 的 `Framebuffer` 字段是私有的，无法在外面构造）。
struct OwnedFramebuffer {
    handle: vk::FramebufferHandle,
    device: vk::DeviceHandle,
    destroy: vk::PfnDestroyFramebuffer,
}

impl OwnedFramebuffer {
    fn handle(&self) -> vk::FramebufferHandle {
        self.handle
    }
}

impl Drop for OwnedFramebuffer {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            // SAFETY: 句柄由本结构创建且未销毁；设备由 `WindowedRenderer` 保证比它活得久
            // （字段声明顺序：framebuffers 在 device 之前）。
            unsafe { (self.destroy)(self.device, self.handle, ptr::null()) };
            self.handle = ptr::null_mut();
        }
    }
}

/// 命令池（私有 RAII）。命令缓冲由池统一回收，不单独 `vkFreeCommandBuffers`。
struct OwnedCommandPool {
    handle: vk::CommandPoolHandle,
    device: vk::DeviceHandle,
    destroy: vk::PfnDestroyCommandPool,
}

impl OwnedCommandPool {
    fn handle(&self) -> vk::CommandPoolHandle {
        self.handle
    }
}

impl Drop for OwnedCommandPool {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            // SAFETY: 同上（command_pool 声明在 device 之前）。
            unsafe { (self.destroy)(self.device, self.handle, ptr::null()) };
            self.handle = ptr::null_mut();
        }
    }
}

/// 栅栏（私有 RAII；`offscreen::Fence` 同样无法从外部构造）。
struct OwnedFence {
    handle: vk::FenceHandle,
    device: vk::DeviceHandle,
    wait: vk::PfnWaitForFences,
    reset: vk::PfnResetFences,
    destroy: vk::PfnDestroyFence,
}

impl OwnedFence {
    /// 创建一个**初始就是 signaled** 的栅栏（第一帧的 `wait` 才能立即返回）。
    fn create(device: &VkDevice) -> GpuResult<OwnedFence> {
        let fns = device.fns();
        let info = vk::FenceCreateInfo {
            s_type: vk::VK_STRUCTURE_TYPE_FENCE_CREATE_INFO,
            p_next: ptr::null(),
            flags: VK_FENCE_CREATE_SIGNALED_BIT,
        };
        let mut handle: vk::FenceHandle = ptr::null_mut();
        // SAFETY: `info` 在栈上存活；`handle` 是可写输出；设备来自 `&VkDevice`（存活）。
        let rc = unsafe { (fns.create_fence)(device.handle(), &info, ptr::null(), &mut handle) };
        if rc != ffi::VK_SUCCESS {
            return Err(GpuError::Driver {
                code: rc,
                message: format!("vkCreateFence 失败：{}", vk_result_name(rc)),
            });
        }
        Ok(OwnedFence {
            handle,
            device: device.handle(),
            wait: fns.wait_for_fences,
            reset: fns.reset_fences,
            destroy: fns.destroy_fence,
        })
    }

    fn handle(&self) -> vk::FenceHandle {
        self.handle
    }

    /// 等栅栏被信号（**有限超时**：超时算失败，不算成功）。
    fn wait(&self, timeout_ns: u64) -> GpuResult<()> {
        // SAFETY: `&self.handle` 指向本结构的句柄且调用期间存活；设备仍有效。
        let rc = unsafe { (self.wait)(self.device, 1, &self.handle, vk::VK_TRUE, timeout_ns) };
        if rc != ffi::VK_SUCCESS {
            return Err(GpuError::Driver {
                code: rc,
                message: format!(
                    "vkWaitForFences 失败：{}（超时 = GPU 没在预期时间内做完，不是「没关系」）",
                    vk_result_name(rc)
                ),
            });
        }
        Ok(())
    }

    fn reset(&self) -> GpuResult<()> {
        // SAFETY: 句柄有效且此刻没有等待者（本实现每帧槽串行使用）。
        let rc = unsafe { (self.reset)(self.device, 1, &self.handle) };
        if rc != ffi::VK_SUCCESS {
            return Err(GpuError::Driver {
                code: rc,
                message: format!("vkResetFences 失败：{}", vk_result_name(rc)),
            });
        }
        Ok(())
    }
}

impl Drop for OwnedFence {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            // SAFETY: 句柄由本结构创建且未销毁；设备比它活得久（声明顺序）。
            unsafe { (self.destroy)(self.device, self.handle, ptr::null()) };
            self.handle = ptr::null_mut();
        }
    }
}

/// 一块缓冲（回读用的暂存目标）。私有 RAII，理由同 `OwnedFramebuffer`。
struct OwnedBuffer {
    handle: vk::BufferHandle,
    device: vk::DeviceHandle,
    destroy: vk::PfnDestroyBuffer,
}

impl OwnedBuffer {
    fn handle(&self) -> vk::BufferHandle {
        self.handle
    }
}

impl Drop for OwnedBuffer {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            // SAFETY: 句柄由本结构创建且未销毁；设备比它活得久（声明顺序）。
            unsafe { (self.destroy)(self.device, self.handle, ptr::null()) };
            self.handle = ptr::null_mut();
        }
    }
}

/// 一块设备内存（回读缓冲绑定的那块，HOST_VISIBLE | HOST_COHERENT）。
struct OwnedMemory {
    handle: vk::DeviceMemoryHandle,
    device: vk::DeviceHandle,
    free: vk::PfnFreeMemory,
}

impl OwnedMemory {
    fn handle(&self) -> vk::DeviceMemoryHandle {
        self.handle
    }
}

impl Drop for OwnedMemory {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            // SAFETY: 句柄由本结构分配且未释放；设备比它活得久（声明顺序）。
            // 缓冲先析构（字段声明顺序：buffer 在 memory 之前）；本渲染器在销毁前已
            // `vkDeviceWaitIdle`，不会有仍在引用这块内存的提交。
            unsafe { (self.free)(self.device, self.handle, ptr::null()) };
            self.handle = ptr::null_mut();
        }
    }
}

/// 呈现帧回读的暂存目标：一块 `HOST_VISIBLE | HOST_COHERENT` 缓冲，尺寸 = 宽×高×4。
///
/// 每帧在 **present 之前**把交换链图像 `vkCmdCopyImageToBuffer` 进来；`read_back_last_frame`
/// 等那一帧的栅栏后映射读回。为什么不做在 present 之后：present 一旦提交，
/// 图像所有权就归呈现引擎（再次碰它是未定义行为），所以复制必须在 present 之前。
struct ReadbackTarget {
    buffer: OwnedBuffer,
    memory: OwnedMemory,
    /// 缓冲字节数 = `readback_row_pitch(w) * h`（用来校验尺寸没算错）
    size: u64,
}

/// 一条完整的窗口出图链。
///
/// ## 字段顺序 = 析构顺序（**不要重排**）
///
/// Rust 按**声明顺序**析构字段，而 Vulkan 要求按依赖倒序销毁。所以：
/// `帧缓冲 → 命令池/管线/渲染通道/着色器 → 信号量/栅栏/回读缓冲 → 交换链 → 设备 → surface → 实例`。
/// 三个关键点：
/// 1. 帧缓冲引用交换链的图像视图 ⇒ 必须比交换链先销毁；
/// 2. `VkDevice` 必须先于 `Surface`/`Instance`（它借用过那个实例）；
/// 3. `Surface` 必须先于 `Instance`（`vkDestroySurfaceKHR` 需要实例还活着）。
pub struct WindowedRenderer {
    framebuffers: Vec<OwnedFramebuffer>,
    /// 命令缓冲从它分配 ⇒ 只为「比命令缓冲/设备活得久」而持有（`Drop` 会回收命令缓冲）。
    #[allow(dead_code)]
    command_pool: OwnedCommandPool,
    command_buffers: Vec<vk::CommandBufferHandle>,
    pipeline: Pipeline,
    pipeline_layout: PipelineLayout,
    render_pass: RenderPass,
    vs: ShaderModule,
    fs: ShaderModule,
    image_available: Vec<Semaphore>,
    render_finished: Vec<Semaphore>,
    in_flight: Vec<OwnedFence>,
    /// 回读暂存目标（`None` = 关闭了回读）。
    readback: Option<ReadbackTarget>,
    /// **界面树**（形状 + 文本）的资源；`None` = 还没建（首次 `draw_and_present` 时惰性建）。
    ///
    /// `resize()` 会把它置回 `None` ⇒ 下次按新尺寸重建（管线里的 viewport 是静态的）。
    /// 声明在 `device` 之前 ⇒ 先于设备析构。
    ui: Option<UiResources>,
    swapchain: Swapchain,
    device: VkDevice,
    surface: Surface,
    instance: ffi::Instance,
    clear: Color,
    extent: Extent,
    frame: usize,
    frames_presented: u64,
    suboptimal_frames: u64,
    /// 是否在每帧 present 前记录「图像 → 回读缓冲」的复制。默认 **开**：
    /// 冻死的契约是「先 `render_and_present()` 再 `read_back_last_frame()`」，
    /// 默认关会让那个 API 必然报错。不想要这份开销的调用方用 `set_readback_enabled(false)`。
    readback_enabled: bool,
    /// 最近一次 `render_and_present()` 是否真的记录了复制（防止读回**上一帧的陈旧数据**）。
    last_frame_readback: bool,
    /// 最近一次呈现用的帧槽（回读要等它的栅栏）。
    last_presented_slot: usize,
    /// 上一帧 `draw_and_present` 里被跳过的文本命令数（诊断，见 [`WindowedRenderer::ui_text_skipped`]）。
    ui_text_skipped: usize,
    /// 界面管线用的是**动态** viewport 吗（录制时据此决定要不要 `vkCmdSetViewport`）。
    ///
    /// **必须与建管线时用的策略一致** —— 不一致就是「对静态状态发动态设置命令」
    /// （校验层报错）或「动态状态从没被设置」（画不出像素）。所以这个标志由
    /// `ensure_ui` 在**同一次**策略解析里写入，不另外读 env。
    ui_viewport_is_dynamic: bool,
    /// 界面资源被**构建**过几次（每次 `ensure_ui` 真正建资源时 +1）。
    ///
    /// 与 [`live_ui_resource_count`] 配对使用：`resize` 反复发生时，
    /// 「构建次数 = 1 + 重建次数」且「存活数恒为 1」才是「失效重建不泄漏」的证据。
    ui_builds: u64,
    /// 渲染统计（M3+ B1；累计值，见 [`RenderStats`]）。三角形路径与界面路径都计入。
    stats: RenderStats,
    /// 本帧是否**真的上传**了统一顶点（M3+ B3）：决定要不要发 host→vertex 屏障。
    ///
    /// B5-2 之前是 `(形状, 文本)` 两个 bool（两块缓冲各一条屏障）；统一之后
    /// **只有一块缓冲** ⇒ 一个 bool、一条屏障。
    ui_barrier: bool,
    /// 索引 / 间接缓冲这一帧是否**真的重传了**（与 `ui_barrier` 同一套语义：
    /// 没重传 = 没有新的主机写入 ⇒ 上一次那条屏障已经给同一块缓冲建立过依赖）。
    ui_index_barrier: bool,
    ui_indirect_barrier: bool,
    /// 累计发出的 host→vertex 屏障条数。
    ///
    /// 为什么必须有（review I-2）：窗口路径的屏障一度**没有计数、没有测试** ——
    /// reviewer 变异「窗口从不发屏障」后 `window_parity` 与 `swapchain_smoke` **全绿**。
    /// 计数写在**发屏障的同一处**（`record_ui` 里的 `cmd_pipeline_barrier` 旁），
    /// 删掉发射就必然删掉计数。
    ///
    /// （B5-2 之前还分「形状 / 文本」两个分项，那是「两块缓冲」的产物；
    ///   现在只有一块缓冲 ⇒ 分项没有意义，已删除。）
    ui_host_to_vertex_barriers: u64,
    /// 累计发出的「主机写索引缓冲 → 索引取数」屏障条数。
    ///
    /// 与 [`Self::ui_host_to_vertex_barriers`] 同一套理由：索引/间接这两条一度
    /// **只有 `ui_index_barrier` / `ui_indirect_barrier` 两个 bool、没有计数** ⇒
    /// 「窗口路径从不发这两条屏障」这类变异在门禁下**全绿**。计数同样写在
    /// **发屏障的同一处**（`emit_ui_buffer_barrier` 调用旁），删发射就必然删掉计数。
    ui_index_barriers: u64,
    /// 累计发出的「主机写间接命令 → 读间接命令」屏障条数（理由同上）。
    ui_indirect_barriers: u64,
    /// **CPU 侧的顶点转换成本口径**（B5-2）：`unify` 的调用次数与累计输出顶点数。
    unify_calls: u64,
    unify_output_vertices: u64,
}

impl WindowedRenderer {
    /// 一条完整链：实例（含 surface 扩展）→ device（图形 + 呈现队列）→ surface → 交换链
    /// → 渲染通道（格式 = 交换链格式）→ 三角形管线（复用 M2a 的 SPIR-V）→ 每帧命令缓冲 + 同步。
    pub fn new(
        adapter_index: usize,
        window: RawWindowHandle,
        want: Extent,
        clear: Color,
    ) -> GpuResult<WindowedRenderer> {
        // ⓪ 回读开关：默认开（见字段文档）。开启时每帧多一次全屏 copy（宽×高×4 字节），
        //    换来「呈现出去的到底是哪张图」可以被逐字节验证。
        let readback_enabled = true;

        // ① 实例：**先用 extension_available 查、缺了就明确报错**（不静默退化成「没有 surface」）
        let validation = ffi::Instance::validation_from_env();
        let platform_ext = Surface::platform_extension(window.platform);
        let instance = ffi::Instance::create_with_extensions(
            validation,
            &[surface::SURFACE_EXTENSION, platform_ext],
        )?;
        if instance.validation_enabled() {
            eprintln!("[deer-vk] 窗口路径已启用 VK_LAYER_KHRONOS_validation（消息打到 stderr）");
        }

        // ② surface（HWND → VkSurfaceKHR）
        let surface = Surface::create(&instance, window)?;

        // ③ 设备：**借用 surface 的实例**拿「图形 + 呈现」队列族（跨实例用 surface 是校验层会抓的错误）
        let device = VkDevice::open_with_present(adapter_index, &surface)?;
        let pd = device.physical_device();

        // ④ 交换链（确定性配置）
        let cfg = Swapchain::choose_config(&surface, pd, want)?;
        let swapchain = Swapchain::create(&device, &surface, cfg, None)?;
        let extent = swapchain.extent();

        // ⑤ 渲染通道：附件最终布局 = PRESENT_SRC_KHR（交换链图像必须处于这个布局才能呈现）；
        //    每帧 CLEAR ⇒ 清屏色就是调用方给的颜色。
        let render_pass = device.create_render_pass(
            swapchain.format(),
            vk::VK_ATTACHMENT_LOAD_OP_CLEAR,
            vk::VK_IMAGE_LAYOUT_PRESENT_SRC_KHR,
        )?;
        let pipeline_layout = device.create_pipeline_layout(None)?;
        // 复用 M2a 真机验证过的两个着色器（自研 SPIR-V 汇编器产出）
        let vs = device.create_shader_module(&spirv::vertex_shader_triangle(TRIANGLE_NDC))?;
        let fs = device.create_shader_module(&spirv::fragment_shader_solid(TRIANGLE_COLOR))?;
        // 动态 viewport/scissor 版管线（`create_graphics_pipeline`）：尺寸变化时不用重建管线
        let pipeline = device.create_graphics_pipeline(&vs, &fs, &pipeline_layout, &render_pass)?;

        // ⑥ 每张交换链图像一个帧缓冲
        let framebuffers =
            create_framebuffers(&device, &render_pass, swapchain.image_views(), extent)?;

        // ⑦ 命令池 + 每帧槽一个命令缓冲
        let command_pool = create_command_pool(&device)?;
        let command_buffers =
            allocate_command_buffers(&device, command_pool.handle(), FRAMES_IN_FLIGHT)?;

        // ⑧ 同步对象
        let in_flight = (0..FRAMES_IN_FLIGHT)
            .map(|_| OwnedFence::create(&device))
            .collect::<GpuResult<Vec<_>>>()?;
        let image_available = (0..FRAMES_IN_FLIGHT)
            .map(|_| Semaphore::create(&device))
            .collect::<GpuResult<Vec<_>>>()?;
        let render_finished = (0..swapchain.image_count())
            .map(|_| Semaphore::create(&device))
            .collect::<GpuResult<Vec<_>>>()?;

        // ⑨ 回读暂存目标（默认开启：契约要求 `render_and_present` 之后能直接回读）
        let readback = if readback_enabled {
            Some(create_readback_target(&device, extent)?)
        } else {
            None
        };

        Ok(WindowedRenderer {
            framebuffers,
            command_pool,
            command_buffers,
            pipeline,
            pipeline_layout,
            render_pass,
            vs,
            fs,
            image_available,
            render_finished,
            in_flight,
            readback,
            ui: None,
            swapchain,
            device,
            surface,
            instance,
            clear,
            extent,
            frame: 0,
            frames_presented: 0,
            suboptimal_frames: 0,
            readback_enabled,
            last_frame_readback: false,
            last_presented_slot: 0,
            ui_text_skipped: 0,
            ui_viewport_is_dynamic: true,
            ui_builds: 0,
            stats: RenderStats::default(),
            ui_barrier: false,
            ui_index_barrier: false,
            ui_indirect_barrier: false,
            ui_host_to_vertex_barriers: 0,
            ui_index_barriers: 0,
            ui_indirect_barriers: 0,
            unify_calls: 0,
            unify_output_vertices: 0,
        })
    }

    pub fn adapter(&self) -> &AdapterInfo {
        self.device.adapter()
    }

    pub fn extent(&self) -> Extent {
        self.extent
    }

    pub fn format(&self) -> i32 {
        self.swapchain.format()
    }

    pub fn present_mode(&self) -> i32 {
        self.swapchain.present_mode()
    }

    pub fn image_count(&self) -> u32 {
        self.swapchain.image_count()
    }

    /// 已成功提交呈现的帧数（**只统计真的调了 `vkQueuePresentKHR` 且返回成功/不最优的帧**）。
    pub fn frames_presented(&self) -> u64 {
        self.frames_presented
    }

    /// 交换链不再最优（`VK_SUBOPTIMAL_KHR`）的帧数 —— 诊断用。
    ///
    /// 这类帧**已经呈现了**，但调用方应当重建交换链，所以它们被上报为
    /// [`FrameOutcome::OutOfDate`]。计数放这里，是为了不把「不最优」伪装成「正常」。
    pub fn suboptimal_frames(&self) -> u64 {
        self.suboptimal_frames
    }

    /// 本渲染器实例实际启用的实例扩展（诊断/验证用：证明 surface 扩展真的启用了）。
    pub fn instance_extensions(&self) -> &[String] {
        self.instance.enabled_extensions()
    }

    /// 开启/关闭**呈现帧回读**（每帧在 present 前多录一次「图像 → 暂存缓冲」的复制）。
    ///
    /// 默认**开**（冻死契约是「`render_and_present()` 之后就能 `read_back_last_frame()`」）。
    /// 关掉可以省掉每帧一次全屏 copy（约 `宽×高×4` 字节的 GPU 带宽 + 一次 barrier 往返），
    /// 代价是 `read_back_last_frame()` 会明确报错（**不会**返回陈旧数据冒充）。
    pub fn set_readback_enabled(&mut self, enabled: bool) -> GpuResult<()> {
        if enabled && self.readback.is_none() {
            self.readback = Some(create_readback_target(&self.device, self.extent)?);
        }
        self.readback_enabled = enabled;
        if !enabled {
            // 关掉之后「最近一帧有没有复制过」必须立即失效，防止读到旧内容
            self.last_frame_readback = false;
        }
        Ok(())
    }

    /// 回读开关当前状态。
    pub fn readback_enabled(&self) -> bool {
        self.readback_enabled
    }

    /// 本渲染器是否**具备**回读能力（暂存缓冲已建好）。
    pub fn readback_available(&self) -> bool {
        self.readback.is_some()
    }

    /// 把**刚刚呈现出去的那一帧**回读为 **RGBA8**（长度 = `宽 × 高 × 4`）。
    ///
    /// ## 语义与契约
    ///
    /// - 返回的是「最近一次 `render_and_present()` 所渲染的那张交换链图像」的像素；
    /// - **通道顺序是 R,G,B,A**：交换链若是 `B8G8R8A8_*`（本机就是），这里会换 R/B
    ///   （见 [`reorder_to_rgba8`]）—— 这正是「清屏色 = 已知值」能当判据的原因；
    /// - 返回值是**图像里的字节**（sRGB 格式下即 sRGB 编码值），不做线性化，
    ///   这样它才和窗口上看到的/截图工具抓到的逐字节一致；
    /// - **会强制一次 GPU→CPU 同步**：等最近那一帧的 in-flight 栅栏（有限超时 5 s），
    ///   所以它适合做验收/截图，**不适合每帧都调**（那会毁掉流水线并行）；
    /// - 必须在 `render_and_present()` **之后**、且中间没有再 `resize` 过；
    ///   回读开关关着（或 resize 过后）会明确报错，**绝不用陈旧数据冒充**。
    pub fn read_back_last_frame(&mut self) -> GpuResult<Vec<u8>> {
        let Some(rb) = self.readback.as_ref() else {
            return Err(GpuError::Unsupported(
                "本渲染器没有回读暂存缓冲（回读被关闭过）⇒ 先 `set_readback_enabled(true)`"
                    .to_string(),
            ));
        };
        if !self.last_frame_readback {
            return Err(GpuError::Unsupported(
                "最近一次 `render_and_present()` 没有记录回读复制（回读开关是关的，或之后 resize 过）\
                 ⇒ 请在 `render_and_present()` 之前调用 `set_readback_enabled(true)`"
                    .to_string(),
            ));
        }
        // 尺寸自检：算错行 stride 会让整幅图斜切，而「四角对、中间错」很难一眼看出
        let want = readback_row_pitch(self.extent.width) as u64 * self.extent.height as u64;
        if rb.size != want {
            return Err(GpuError::Driver {
                code: -1,
                message: format!(
                    "回读缓冲 {} 字节与当前交换链 {}×{} 不符（应为 {}）⇒ 请 resize 重建",
                    rb.size, self.extent.width, self.extent.height, want
                ),
            });
        }

        // 等「最后那一帧」的栅栏：队列内提交是有序的 ⇒ 它 signaled 就意味着
        // 那一帧的 image→buffer 复制已经完成，且它之后的写不可能存在。
        // 用有限超时而不是无限等 / `vkDeviceWaitIdle`（后者没有超时参数，可能永久挂住）。
        self.in_flight[self.last_presented_slot].wait(FENCE_TIMEOUT_NS)?;

        let fns = *self.device.fns();
        let mut mapped: *mut c_void = ptr::null_mut();
        // SAFETY: 内存是 HOST_VISIBLE | HOST_COHERENT（建缓冲时就是这么挑的类型）；
        // `WHOLE_SIZE` 表示映射整块；`mapped` 是可写输出。
        let rc = unsafe {
            (fns.map_memory)(
                self.device.handle(),
                rb.memory.handle(),
                0,
                vk::WHOLE_SIZE,
                0,
                &mut mapped,
            )
        };
        if rc != ffi::VK_SUCCESS {
            return Err(GpuError::Driver {
                code: rc,
                message: format!("vkMapMemory（回读缓冲）失败：{}", vk_result_name(rc)),
            });
        }
        if mapped.is_null() {
            return Err(GpuError::Driver {
                code: rc,
                message: "vkMapMemory 返回空指针".to_string(),
            });
        }
        // SAFETY: 映射了 `rb.size` 字节（WHOLE_SIZE）；拷贝出来即与映射解耦。
        let raw =
            unsafe { std::slice::from_raw_parts(mapped as *const u8, rb.size as usize).to_vec() };
        // SAFETY: 与上面的 map 配对（本函数内唯一一次 map/unmap）。
        unsafe { (fns.unmap_memory)(self.device.handle(), rb.memory.handle()) };

        reorder_to_rgba8(&raw, self.swapchain.format())
    }

    /// 尺寸变化：重建交换链（以及依赖它的帧缓冲/同步对象；格式变了还会重建渲染通道与管线）。
    pub fn resize(&mut self, extent: Extent) -> GpuResult<()> {
        let want = Extent {
            width: extent.width.max(1),
            height: extent.height.max(1),
        };
        // 换交换链前必须等 GPU 空闲：旧图像可能还在被读/被呈现
        self.device.wait_idle()?;

        let old_format = self.swapchain.format();
        let pd = self.device.physical_device();
        let cfg = Swapchain::choose_config(&self.surface, pd, want)?;

        // 先把「新交换链需要的同步对象」备好：尽量让失败发生在旧状态被破坏之前
        let mut new_render_finished = (0..cfg.image_count)
            .map(|_| Semaphore::create(&self.device))
            .collect::<GpuResult<Vec<_>>>()?;
        // 回读缓冲的尺寸跟着交换链走 ⇒ 也提前备好（同样是为了「失败不破坏旧状态」）
        let new_readback = if self.readback_enabled {
            Some(create_readback_target(&self.device, cfg.extent)?)
        } else {
            None
        };
        // `oldSwapchain` 复用：成功之后旧交换链即被 retire，由 `self.swapchain = new` 析构
        let new_swapchain =
            Swapchain::create(&self.device, &self.surface, cfg, Some(&self.swapchain))?;
        // 驱动可以给比 `minImageCount` 更多的图像 ⇒ 补齐信号量
        while (new_render_finished.len() as u32) < new_swapchain.image_count() {
            new_render_finished.push(Semaphore::create(&self.device)?);
        }
        // 交换链实际尺寸可能与请求的不同（`currentExtent` 优先级）；缓冲按实际尺寸再核一次
        if let Some(rb) = &new_readback {
            let want = readback_row_pitch(new_swapchain.extent().width) as u64
                * new_swapchain.extent().height as u64;
            if rb.size != want {
                return Err(GpuError::Driver {
                    code: -1,
                    message: format!(
                        "回读缓冲尺寸 {} 与新交换链 {}×{} 不符（驱动改了尺寸？）",
                        rb.size,
                        new_swapchain.extent().width,
                        new_swapchain.extent().height
                    ),
                });
            }
        }

        // 帧缓冲引用旧交换链的图像视图 ⇒ 必须在旧交换链销毁**之前**销毁
        self.framebuffers.clear();
        self.swapchain = new_swapchain;
        self.render_finished = new_render_finished;
        self.readback = new_readback;
        // 尺寸变了，上次回读的数据不再对应当前交换链 ⇒ 明确失效（不许拿陈旧数据冒充）
        self.last_frame_readback = false;

        // 格式变了就必须重建渲染通道与管线（管线里写着附件格式与渲染通道句柄）。
        // 注意顺序：先换管线（旧管线引用旧渲染通道），再换渲染通道。
        if self.swapchain.format() != old_format {
            let new_pass = self.device.create_render_pass(
                self.swapchain.format(),
                vk::VK_ATTACHMENT_LOAD_OP_CLEAR,
                vk::VK_IMAGE_LAYOUT_PRESENT_SRC_KHR,
            )?;
            let new_pipeline = self.device.create_graphics_pipeline(
                &self.vs,
                &self.fs,
                &self.pipeline_layout,
                &new_pass,
            )?;
            self.pipeline = new_pipeline;
            self.render_pass = new_pass;
        }
        // 动态 viewport/scissor ⇒ 尺寸变化不需要重建三角形管线。
        //
        // 界面管线**只在静态策略下**才需要重建（静态 viewport 把尺寸写死在管线里）。
        // 此刻上面已经 `wait_idle()` 过 ⇒ 旧管线不在使用中，可以安全销毁；
        // 「销毁 + 下次重建」这条路径**不能泄漏**，由 `live_ui_resource_count()`
        // 与 `ui_build_count()` 两个计数器在示例里断言（见 `window_parity.rs`）。
        if !self.ui_viewport_is_dynamic {
            self.ui = None;
        }

        let framebuffers = create_framebuffers(
            &self.device,
            &self.render_pass,
            self.swapchain.image_views(),
            self.swapchain.extent(),
        )?;
        self.framebuffers = framebuffers;
        self.extent = self.swapchain.extent();
        // 尺寸变了，帧槽计数归零（不是必须，但让「第一帧」总是从槽 0 开始，便于复现）
        self.frame = 0;
        Ok(())
    }

    /// 画一帧并呈现（**M2b 的三角形路径**，行为与 M2b 完全一致）。
    ///
    /// 交换链过期/不再最优时返回 [`FrameOutcome::OutOfDate`]（**不 panic、不假装成功**），
    /// 调用方 `resize` 后重试。
    ///
    /// 与 [`Self::draw_and_present`] 共用同一段「取图 → 录制 → 提交 → 呈现」逻辑
    /// （[`Self::present_frame`]）；区别只有「录什么」。
    pub fn render_and_present(&mut self) -> GpuResult<FrameOutcome> {
        // 三角形路径的 `record(&self, ..)` 包一层：`present_frame` 现在交的是 `&mut Self`
        // （界面路径要在发调用的地方自增统计）。
        self.present_frame(|s, slot, image_index| s.record(slot, image_index))
    }

    /// 「取图 → 录制 → 提交 → 呈现」的公共骨架：`record` 决定这一帧录什么。
    ///
    /// 抽出来的理由：两条路径（M2b 的三角形、M3c 的界面树）在**同步与呈现**上
    /// 必须逐字相同 —— 复制一份就等于复制一份「只改了一边」的风险（超时/过期/SUBOPTIMAL
    /// 这些分支极易在复制时漏改）。
    ///
    /// `record` 收 `&mut Self`（M3+ B1）：界面路径要在**发真实绘制调用的地方**自增 `stats`
    /// （draw call / 管线切换），而那需要可变借用。闭包本身不捕获 `self`（只捕获体外的
    /// 局部量如 `calls`），所以 `&mut self` 与它不冲突。
    fn present_frame<R>(&mut self, mut record: R) -> GpuResult<FrameOutcome>
    where
        R: FnMut(&mut Self, usize, u32) -> GpuResult<()>,
    {
        if self.framebuffers.len() != self.swapchain.image_count() as usize {
            return Err(GpuError::Unsupported(
                "帧缓冲与交换链图像数不一致（上一次 resize 失败过？）⇒ 请再调一次 resize"
                    .to_string(),
            ));
        }
        let slot = self.frame % FRAMES_IN_FLIGHT;

        // ① 等这一槽上一帧做完
        self.in_flight[slot].wait(FENCE_TIMEOUT_NS)?;

        // ② 取图像
        let (image_index, acquire_suboptimal) =
            match self.swapchain.acquire(self.image_available[slot].handle())? {
                Acquire::Image(i) => (i, false),
                Acquire::Suboptimal => {
                    // SUBOPTIMAL 时驱动仍然写回了有效索引（规范），拿它去 present 才能释放这张图像
                    let i = self.swapchain.last_acquired_image().ok_or_else(|| {
                        GpuError::Driver {
                            code: ffi::VK_SUBOPTIMAL_KHR,
                            message:
                                "vkAcquireNextImageKHR 返回 SUBOPTIMAL 但没记下图像索引（内部不一致）"
                                    .to_string(),
                        }
                    })?;
                    (i, true)
                }
                Acquire::OutOfDate => {
                    // **不 reset 栅栏**：这一帧不提交，栅栏保持 signaled，下一帧的 wait 立即返回
                    self.frame = (slot + 1) % FRAMES_IN_FLIGHT;
                    return Ok(FrameOutcome::OutOfDate);
                }
            };
        if image_index >= self.swapchain.image_count()
            || image_index as usize >= self.framebuffers.len()
        {
            return Err(GpuError::Driver {
                code: -1,
                message: format!(
                    "驱动给出的图像索引 {image_index} 超出范围（交换链 {} 张 / 帧缓冲 {} 个）",
                    self.swapchain.image_count(),
                    self.framebuffers.len()
                ),
            });
        }

        // ③ 确定要提交了，才 reset 栅栏
        self.in_flight[slot].reset()?;

        // ④ 录制（录什么由调用方给的 `record` 决定：三角形 or 界面树）
        record(self, slot, image_index)?;

        // ⑤ 提交
        let fns = *self.device.fns();
        let image_available = self.image_available[slot].handle();
        let render_finished = self.render_finished[image_index as usize].handle();
        let cmd = self.command_buffers[slot];
        let fence = self.in_flight[slot].handle();
        let wait_semaphores = [image_available];
        let wait_stages = [vk::VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT];
        let command_buffers = [cmd];
        let signal_semaphores = [render_finished];
        let submit = vk::SubmitInfo {
            s_type: vk::VK_STRUCTURE_TYPE_SUBMIT_INFO,
            p_next: ptr::null(),
            wait_semaphore_count: 1,
            p_wait_semaphores: wait_semaphores.as_ptr(),
            p_wait_dst_stage_mask: wait_stages.as_ptr(),
            command_buffer_count: 1,
            p_command_buffers: command_buffers.as_ptr(),
            signal_semaphore_count: 1,
            p_signal_semaphores: signal_semaphores.as_ptr(),
        };
        // SAFETY: 队列是设备自己的；三个数组与 `submit` 都在本栈帧存活；
        // 命令缓冲已结束录制；栅栏已 reset 且本帧独占。
        let rc = unsafe { (fns.queue_submit)(self.device.queue(), 1, &submit, fence) };
        // 计数与真实调用同处（M3+ 第 4 项下半）：`submits` 就是**这一行**的次数 ——
        // 放在 `if rc != VK_SUCCESS` **之前**（提交已经发出去了，成功与否都是「一次提交尝试」），
        // 且删掉这行提交就必然删掉这行计数。
        self.stats.submits += 1;
        if rc != ffi::VK_SUCCESS {
            return Err(GpuError::Driver {
                code: rc,
                message: format!("vkQueueSubmit 失败：{}", vk_result_name(rc)),
            });
        }

        // ⑥ 呈现
        let presented =
            self.swapchain
                .present(self.device.present_queue(), render_finished, image_index)?;
        self.frames_presented += 1;
        self.frame = (slot + 1) % FRAMES_IN_FLIGHT;
        // 回读用：记住这一帧在哪个槽（要等它的栅栏），以及「这一帧确实复制过」。
        self.last_presented_slot = slot;
        self.last_frame_readback = self.readback_enabled && self.readback.is_some();

        match presented {
            Present::Presented if !acquire_suboptimal => Ok(FrameOutcome::Presented),
            // SUBOPTIMAL（acquire 或 present 侧）：**这一帧已呈现**，但交换链已不最优 ⇒
            // 如实上报 OutOfDate 让调用方重建，而不是把它当成功吞掉。
            Present::Presented | Present::Suboptimal => {
                self.suboptimal_frames += 1;
                Ok(FrameOutcome::OutOfDate)
            }
            // 过期：图像已 acquire 但没呈现成功 ⇒ 也必须重建（调用方 resize）
            Present::OutOfDate => Ok(FrameOutcome::OutOfDate),
        }
    }

    /// 等 GPU 空闲（截图、关窗、销毁资源前用）。
    pub fn wait_idle(&mut self) -> GpuResult<()> {
        self.device.wait_idle()
    }

    /// 上一帧**被跳过**的文本命令数（空串 / `size <= 0` / 被裁空；与离屏同一语义）。
    pub fn ui_text_skipped(&self) -> usize {
        self.ui_text_skipped
    }

    /// 界面资源被构建过几次（见 `ui_builds` 字段的文档；示例用它断言 `resize` 不泄漏）。
    pub fn ui_build_count(&self) -> u64 {
        self.ui_builds
    }

    /// **渲染统计**（M3+ B1）：draw call / 管线切换 / 缓冲上传 / 缓冲分配（**累计值**）。
    ///
    /// 四个量都与真实 Vulkan 调用一一对应，计数写在**发调用的同一处**
    /// （三角形路径与界面路径都计入）⇒ 删掉发射必然删掉计数。
    /// 用法：读两次取差值 ⇒ 「这一帧/这一段」的代价。
    pub fn render_stats(&self) -> RenderStats {
        self.stats
    }

    /// 累计发出的 host→vertex 屏障条数（review I-2 的护栏）。
    ///
    /// B5-2 之前分「形状 / 文本」两个计数（两块顶点缓冲各一条）；统一之后只有**一块**
    /// 顶点缓冲 ⇒ 一帧最多一条 ⇒ 分项没有意义，已合并成一个。
    pub fn ui_host_to_vertex_barrier_count(&self) -> u64 {
        self.ui_host_to_vertex_barriers
    }

    /// 累计发出的「主机写索引缓冲 → 索引取数」屏障条数（护栏同 [`Self::ui_host_to_vertex_barrier_count`]）。
    ///
    /// 语义与顶点那条一致：只在**这一帧真的重传了**索引缓冲时才 +1（内容只在顶点数变化
    /// 时才变 ⇒ 稳态零上传、也零屏障）。
    pub fn ui_index_barrier_count(&self) -> u64 {
        self.ui_index_barriers
    }

    /// 累计发出的「主机写间接命令 → 读间接命令」屏障条数（护栏同上）。
    pub fn ui_indirect_barrier_count(&self) -> u64 {
        self.ui_indirect_barriers
    }

    /// **统一顶点转换的 CPU 成本口径**（B5-2）：`unify` 的调用次数（累计）。
    ///
    /// 与 [`Self::unify_output_vertex_count`] 配对 ⇒「画了 N 帧调了 N 次、搬了 Σ 个顶点」。
    /// 本项目口径：**不许只说「可忽略」**。
    pub fn unify_call_count(&self) -> u64 {
        self.unify_calls
    }

    /// `unify` 累计输出的统一顶点数（= 段表声明的顶点数之和）。
    pub fn unify_output_vertex_count(&self) -> u64 {
        self.unify_output_vertices
    }

    /// **本帧绑定并采样的纹理尺寸**（宽, 高）—— 哑纹理 / 图集护栏的读数口
    /// （与 `GpuGeometryRenderer::bound_texture_size` 同一语义）。
    ///
    /// 读的是 **`set` 真实指向**（`descriptor_points_at`，由
    /// `gpu_render::point_descriptor_at` 在发 `vkUpdateDescriptorSets` 的同一处赋值）
    /// —— 不是从 `texture` 字段推出来的代理读数。见那个字段的说明。
    pub fn bound_texture_size(&self) -> (u32, u32) {
        match self.ui.as_ref() {
            // 界面资源已建 ⇒ 它记着描述符集此刻指着谁（哑纹理或字形图集）
            Some(u) => u.descriptor_points_at,
            // 界面资源还没建 ⇒ 下一帧建的时候会绑 1×1 哑纹理
            None => (1, 1),
        }
    }

    /// **主动释放**界面资源（两条管线 + 描述符集 + 图集纹理 + 两块顶点缓冲）；
    /// 下一次 `draw_and_present` 会按当前交换链尺寸/格式重建。
    ///
    /// 用途：窗口最小化、长时间空闲，以及**在两种 viewport 策略下都能验证「析构真的发生」**。
    ///
    /// ## 必须先 `vkDeviceWaitIdle`（实测踩过：不等就 `VK_ERROR_DEVICE_LOST`）
    ///
    /// **任何主动释放都必须等设备空闲，而不是等当前帧栅栏。**
    ///
    /// `FRAMES_IN_FLIGHT = 2` ⇒ 「等当前槽的栅栏」**不等于**「上一帧的提交已经做完」：
    /// 另一个槽的命令缓冲可能还在跑，而它引用着这些管线/缓冲/纹理。
    /// 直接析构就是「正在被 GPU 使用的对象被销毁」—— 本机实测直接
    /// `VK_ERROR_DEVICE_LOST`（`vkQueueSubmit` 失败，随后连 `vkDeviceWaitIdle` 也失败）。
    /// `resize()` 换交换链时同样先等空闲，理由相同。
    ///
    /// ⚠️ 将来若加**第二个 in-flight 槽**或**第二个队列**，这条注释是唯一的护栏：
    /// 那时「等当前帧栅栏」离「设备空闲」更远，而不是更近。
    ///
    /// ## 为什么需要这个入口（review I-2）
    ///
    /// `resize()` 只在**静态**策略下销毁界面资源（动态策略不必重建，见那里的注释）⇒
    /// 默认（动态）配置下「析构」这条路径**根本不存在**：变异「删掉 `Drop` 里的计数递减」
    /// 在默认口径下**不会变红**，断言等于空转（reviewer 实测）。
    /// 有了这个入口，两种策略都能走到析构，「存活资源数」这条护栏才真正咬得住。
    pub fn release_ui_resources(&mut self) -> GpuResult<()> {
        // 契约：调用点都在帧与帧之间；这里再等一次空闲，保证没有在飞的提交引用它们。
        self.device.wait_idle()?;
        self.ui = None;
        Ok(())
    }

    /// 当前界面管线用的是动态 viewport 吗（实测/诊断用）。
    pub fn ui_viewport_is_dynamic(&self) -> bool {        self.ui_viewport_is_dynamic
    }

    /// **画一帧界面树并呈现**（M3c-T3）。
    ///
    /// ## 它做什么（**B5-2 起是单管线、单缓冲、一次 draw**）
    ///
    /// 1. 惰性建界面资源（**一条**统一管线 + `set 0` + 哑纹理）；
    /// 2. **单次遍历** `list`：形状命令走 `gpu_geom::build_stream`、文本命令走
    ///    `gpu_text::build_text_stream`（**不重写顶点流语义**），按原顺序记录段表
    ///    ⇒ z 序与 CPU 一致 → 两路顶点由 `vertex_unify::unify` 合成**一条**统一流；
    /// 3. 每帧把统一顶点流上传进**一块**缓冲（内容逐字节相同则跳过，B3）；
    /// 4. 图集纹理**只在指纹变化时**重传（与离屏同一条契约）；
    /// 5. 录制：清屏 → **一次** bind（管线/顶点缓冲/描述符集）+ **一次** `vkCmdDraw`
    ///    →（可选）回读复制 → 呈现。
    ///
    /// ## viewport：默认**动态**（`DEER_VK_WINDOW_VIEWPORT` 可强制静态）
    ///
    /// 统一管线由 [`crate::gpu_render::build_unified_pipeline`] 按
    /// [`viewport_strategy_from_env`] 建（共用层提供布局/采样器：
    /// `pipelines::build_pipeline_resources`）：
    ///
    /// - **动态（默认）**：M2b 起窗口路径就是这么用的；尺寸变化**不必**重建管线；
    /// - **静态**（诊断/实测用）：viewport 写死在管线里 ⇒ `resize()` 必须让界面资源失效
    ///   （见那里的注释与 `release_ui_resources`）。
    ///
    /// 朝向两条策略**一致**：`(0, 0, w, h)`、`min_depth 0 / max_depth 1` ⇒ NDC `y = -1` 在**上**、
    /// 像素 y 从上往下 —— 正是形状片元着色器里 `gl_FragCoord`（`OriginUpperLeft`）判据要求的方向。
    ///
    /// **本机 Intel 集显的实测**（窗口路径，各 30 帧）：两种策略都能上屏且像素完全相同；
    /// 而「声明动态却从不调 `vkCmdSetViewport`」会直接 `0xC000041D` 崩溃
    /// —— 所以那条「必须调」的规则是硬要求，**离屏若要用动态也一样**。
    /// 复现方式与那句 M2a 旧结论的存疑之处，见 [`viewport_strategy_from_env`] 的文档。
    ///
    /// ## 报错策略（与离屏对齐）
    ///
    /// - 列表裁剪栈不平衡 ⇒ `Unsupported`；
    /// - 有文本命令但调用方没给 `TextEngine` ⇒ `Unsupported`（与离屏 M3a 行为一致，
    ///   **不静默丢弃**）；
    /// - 文本假阳性（空串 / `size <= 0` / 被裁空）⇒ **跳过并计数**（[`Self::ui_text_skipped`]）。
    pub fn draw_and_present(
        &mut self,
        list: &DrawList,
        text: Option<&mut TextEngine>,
    ) -> GpuResult<FrameOutcome> {
        // T1.1：准备段（①..⑤）与帧舞蹈（⑥）各抽成一个方法 —— 与 HAL `Frame` 路径
        // **共用同一段实现**（HAL 的 `record` 调 `prepare_ui`，`submit_and_present`
        // 调 `present_prepared`）。
        let unified = self.prepare_ui(list, text)?;
        self.present_prepared(&unified)
    }

    /// **UI 录制的前半段**（T1.1 从 `draw_and_present` 抽出，供 HAL 路径复用）。
    ///
    /// ① `ensure_ui` → ② 单次遍历建形状/文本顶点 + 段表 → ③ `unify` 合流
    /// → ④ 上传统一顶点缓冲 → ④b 索引 + 间接命令 → ⑤ 刷新图集纹理。
    ///
    /// 返回值（本帧统一顶点流）**不存进 `self`**：它要在「录制」与「呈现」之间
    /// 活着，由调用方管 —— 这样 HAL 的 `VulkanFrame::record` 能把它暂存在帧对象上，
    /// 两次 `begin_frame` 不会互相覆盖。
    pub(crate) fn prepare_ui(
        &mut self,
        list: &DrawList,
        text: Option<&mut TextEngine>,
    ) -> GpuResult<Vec<crate::vertex_unify::UnifiedVertex>> {
        if self.framebuffers.len() != self.swapchain.image_count() as usize {
            return Err(GpuError::Unsupported(
                "帧缓冲与交换链图像数不一致（上一次 resize 失败过？）⇒ 请再调一次 resize"
                    .to_string(),
            ));
        }
        if !list.clip_balanced() {
            return Err(GpuError::Unsupported(
                "绘制列表的裁剪栈不平衡（PushClip/PopClip 未配对）".to_string(),
            ));
        }

        // ① 资源（惰性；resize 之后会被重建）
        self.ensure_ui(text.is_some())?;

        // ② 单次遍历：按原顺序把每条命令送进对应翻译层，并记录绘制段（保 z 序）
        let mut shape_verts: Vec<GpuVertex> = Vec::new();
        let mut text_verts: Vec<TextVertex> = Vec::new();
        let mut calls: Vec<DrawCall> = Vec::new();
        let mut active_clip: Vec<RectI> = Vec::new();
        let mut engine = text;
        let mut skipped = 0usize;
        for cmd in &list.cmds {
            match cmd {
                DrawCmd::PushClip { rect } => active_clip.push(*rect),
                DrawCmd::PopClip => {
                    active_clip.pop();
                }
                DrawCmd::NodeHint { .. } => {}
                DrawCmd::Text { .. } => {
                    let Some(engine) = engine.as_deref_mut() else {
                        return Err(GpuError::Unsupported(
                            "窗口路径收到文本命令，但调用方没有提供 TextEngine\
                             （draw_and_present 的第二个参数为 None）—— 与离屏行为一致：\
                             报告而不是静默丢弃"
                                .to_string(),
                        ));
                    };
                    let one = ui_single_command_in_clip(&active_clip, cmd);
                    let s = gpu_text::build_text_stream(&one, self.extent, engine);
                    skipped += s.skipped;
                    if !s.vertices.is_empty() {
                        let first = u32::try_from(text_verts.len()).map_err(|_| {
                            GpuError::Unsupported("文本顶点数超出 u32".to_string())
                        })?;
                        let count = u32::try_from(s.vertices.len()).map_err(|_| {
                            GpuError::Unsupported("文本顶点数超出 u32".to_string())
                        })?;
                        text_verts.extend_from_slice(&s.vertices);
                        calls.push(DrawCall {
                            kind: PipelineKind::Text,
                            first,
                            count,
                        });
                    }
                }
                other => {
                    let one = ui_single_command_in_clip(&active_clip, other);
                    let s = gpu_geom::build_stream(&one, self.extent);
                    if !s.unsupported.is_empty() {
                        return Err(GpuError::Unsupported(s.unsupported.join("；")));
                    }
                    if !s.vertices.is_empty() {
                        let first = u32::try_from(shape_verts.len()).map_err(|_| {
                            GpuError::Unsupported("形状顶点数超出 u32".to_string())
                        })?;
                        let count = u32::try_from(s.vertices.len()).map_err(|_| {
                            GpuError::Unsupported("形状顶点数超出 u32".to_string())
                        })?;
                        shape_verts.extend_from_slice(&s.vertices);
                        calls.push(DrawCall {
                            kind: PipelineKind::Shape,
                            first,
                            count,
                        });
                    }
                }
            }
        }
        self.ui_text_skipped = skipped;

        // ③ **合流**（B5-2）：两路顶点 + 段表 ⇒ **一条**统一顶点流（顺序即 z 序）。
        self.unify_calls += 1;
        let unified: Vec<crate::vertex_unify::UnifiedVertex> =
            crate::vertex_unify::unify(&shape_verts, &text_verts, &calls);
        self.unify_output_vertices += unified.len() as u64;

        // ④ 上传：**一块**统一顶点缓冲（**内容变化才重传**；容量不足才重建）
        //
        // 计数（`buffer_uploads` / `buffer_allocations`）在**被调用方**里自增，
        // 与真实调用同处 —— 删掉上传就必然删掉计数。
        // B3：内容逐字节相同的帧跳过重传（UI 帧的顶点数据通常与上一帧相同）。
        let mut uploaded_now = false;
        if !unified.is_empty() {
            let bytes = std::mem::size_of_val(unified.as_slice());
            let dev = &self.device;
            let stats = &mut self.stats;
            let ui = self.ui.as_mut().expect("ensure_ui 之后必有资源");
            ensure_ui_vertex_capacity(
                dev,
                &mut ui.vb,
                bytes as u64,
                stats,
                vk::VK_BUFFER_USAGE_VERTEX_BUFFER_BIT,
                "窗口统一顶点缓冲",
            )?;
            // SAFETY: `UnifiedVertex` 是 `#[repr(C)]` 纯 `f32`（无指针、无 Drop）⇒ 字节视图合法。
            let src = unsafe { std::slice::from_raw_parts(unified.as_ptr() as *const u8, bytes) };
            let handle = ui.vb.as_ref().expect("刚 ensure 过").buffer.handle();
            // 跳过条件：**同一块缓冲**（句柄相等）且字节相同 —— 句柄一变就自动作废（I-1）
            let same =
                matches!(&ui.uploaded_vertices, Some((h, b)) if *h == handle && b.as_slice() == src);
            if !same {
                let vb = ui.vb.as_ref().expect("刚 ensure 过");
                upload_ui_vertices(dev, vb, src, "vkMapMemory(窗口统一顶点)", stats)?;
                ui.uploaded_vertices = Some((handle, src.to_vec()));
                uploaded_now = true;
            }
        }
        // 屏障只在**这一帧真的上传了**时发（B3 收紧后的语义，与离屏一致）
        self.ui_barrier = uploaded_now;

        // ④b **索引 + 间接命令**（M3+ 第 4 项下半）：与离屏同一条设计 ——
        //     内容只在顶点数变化时才变 ⇒ 稳态零上传、零分配；三种缓冲各有自己的计数器。
        self.ui_index_barrier = false;
        self.ui_indirect_barrier = false;
        if !unified.is_empty() {
            let vertex_count = unified.len() as u32;
            let dev = &self.device;
            let stats = &mut self.stats;
            let ui = self.ui.as_mut().expect("ensure_ui 之后必有资源");

            let mut index_slot = ui.index.take();
            ensure_ui_vertex_capacity(
                dev,
                &mut index_slot,
                vertex_count as u64 * 4,
                stats,
                crate::device::VK_BUFFER_USAGE_INDEX_BUFFER_BIT,
                "窗口索引缓冲",
            )?;
            ui.index = index_slot;
            let index_handle = ui.index.as_ref().expect("刚 ensure 过").buffer.handle();
            let index_changed = !matches!(
                &ui.uploaded_indices,
                Some((h, n)) if *h == index_handle && *n == vertex_count
            );
            if index_changed {
                let buf = ui.index.as_ref().expect("刚 ensure 过");
                let indices = DrawIndexedIndirectCommand::sequential_indices(vertex_count);
                upload_ui_vertices(dev, buf, &indices, "vkMapMemory(窗口索引)", stats)?;
                // 计数与真实调用同处
                stats.index_uploads += 1;
                ui.uploaded_indices = Some((index_handle, vertex_count));
                self.ui_index_barrier = true;
            }

            let mut indirect_slot = ui.indirect.take();
            ensure_ui_vertex_capacity(
                dev,
                &mut indirect_slot,
                std::mem::size_of::<DrawIndexedIndirectCommand>() as u64,
                stats,
                crate::device::VK_BUFFER_USAGE_INDIRECT_BUFFER_BIT,
                "窗口间接命令缓冲",
            )?;
            ui.indirect = indirect_slot;
            let indirect_handle = ui.indirect.as_ref().expect("刚 ensure 过").buffer.handle();
            let query = DrawIndexedIndirectCommand::for_vertex_count(vertex_count);
            let command_changed = !matches!(
                &ui.uploaded_indirect,
                Some((h, c)) if *h == indirect_handle && *c == query
            );
            if command_changed {
                let buf = ui.indirect.as_ref().expect("刚 ensure 过");
                upload_ui_vertices(dev, buf, &query.to_bytes(), "vkMapMemory(窗口间接命令)", stats)?;
                // 计数与真实调用同处
                stats.indirect_uploads += 1;
                ui.uploaded_indirect = Some((indirect_handle, query));
                self.ui_indirect_barrier = true;
            }
        }

        // ⑤ 图集纹理：指纹变化才重传（与离屏同一条契约）
        if let Some(engine) = engine.as_deref() {
            self.refresh_ui_atlas_texture(engine)?;
        }

        // ⑥ 帧舞蹈（取图 → 录制 → 提交 → 呈现）：与 `render_and_present` 共用同一段逻辑
        //
        // ⚠️ 这里**不需要**任何合段：整帧只发一次 draw，段划分对录制没有影响
        // （与离屏同一条；B5-3 已把那个失去调用点的合段函数删掉）。
        Ok(unified)
    }

    /// **UI 录制的后半段**（T1.1 从 `draw_and_present` 抽出，供 HAL 路径复用）。
    ///
    /// 取图 → 录制（[`Self::record_ui`]）→ 提交 → 呈现。
    /// 与 `render_and_present` 共用同一段 `present_frame` 帧舞蹈。
    ///
    /// **`unified` 可以为空**（「空帧」= 只清屏 + 呈现）：但 `record_ui` 要求界面资源
    /// 已建（它 `expect` `self.ui`），所以这里**先补一次 `ensure_ui(false)`** ——
    /// HAL 的 `render_and_present` 没有「先 record」这一步，空帧路径必须自己兜住。
    pub(crate) fn present_prepared(
        &mut self,
        unified: &[crate::vertex_unify::UnifiedVertex],
    ) -> GpuResult<FrameOutcome> {
        self.ensure_ui(false)?;
        self.present_frame(|s, slot, image_index| s.record_ui(slot, image_index, unified))
    }

    /// 惰性建界面资源（**统一管线**：B5-2 起形状与文本共用一条）。
    ///
    /// ## 顶点着色器与片段着色器的接口由共用层/统一对保证
    ///
    /// M3c 的 Step 1 曾在这里踩过一次：形状管线的 VS 写成 `vertex_shader_from_vertex_buffer`
    /// （M2a 探针用的那个），而形状 FS 声明了 location 0/1/2 的输入 —— **驱动照样建管线成功、
    /// 照样画出像素**，只有校验层报「FS 有 Input 但上一阶段没有对应 Output」。
    /// 现在界面路径的着色器只有**一个**来源：[`crate::gpu_render::build_unified_pipeline`]
    /// （B5-3 起旧的两条管线已删；那 4 支旧着色器函数仍留在 `spirv.rs` 里，
    /// `spirv_val.rs` / 本模块的单测继续钉它们的接口一致性）。窗口路径不自己拼管线。
    ///
    /// ## 颜色格式与 viewport 策略
    ///
    /// - `color_format` = **交换链的实际格式**（运行期事实，不写死）——
    ///   M3c 已把 `pick_config` 改成**线性 `*_UNORM` 优先**（sRGB 附件连混合都在线性空间，
    ///   与 CPU 的字节空间混合对不上，半透明会差几十字节）；
    /// - `viewport` 由 [`viewport_strategy_from_env`] 决定（默认动态）。
    ///
    /// ## 统一管线需要的额外东西
    ///
    /// 统一 FS **无条件采样** ⇒ `set 0` 必须恒有效 ⇒ 这里同时建
    /// **描述符集 + 1×1 哑纹理**并立刻绑上（有引擎时下一帧被图集替换）。
    fn ensure_ui(&mut self, want_text: bool) -> GpuResult<()> {
        if self.ui.is_none() {
            let viewport = match viewport_strategy_from_env() {
                // 静态策略：把**当前交换链尺寸**写进管线（env 里的占位尺寸在这里被替换）
                pipelines::ViewportStrategy::Static { .. } => pipelines::ViewportStrategy::Static {
                    width: self.extent.width,
                    height: self.extent.height,
                },
                other => other,
            };
            self.ui_viewport_is_dynamic = viewport == pipelines::ViewportStrategy::Dynamic;
            // 共用资源（B5-3）：管线布局 / `set 0` 布局 / 采样器 —— **不建管线**。
            // 旧的两条（形状/文本）已删：统一之后它们没有任何绑定点，建了不用就是纯成本。
            let pipes = pipelines::build_pipeline_resources(&self.device)?;
            // ★ 统一管线：界面路径**唯一**建、也唯一使用的那一条（stride 52，5 个 location）
            let (unified, unified_vs, unified_fs) = crate::gpu_render::build_unified_pipeline(
                &self.device,
                &self.render_pass,
                self.swapchain.format(),
                viewport,
                &pipes.text_layout,
            )?;
            let pool = self.device.create_descriptor_pool(1)?;
            let set = self
                .device
                .allocate_descriptor_set(&pool, &pipes.text_set_layout)?;
            let dummy_texture =
                self.device
                    .create_texture_r8(1, 1, &crate::gpu_render::DUMMY_COVERAGE)?;
            // 读数与副作用同处：这块记录**只能**来自那个函数的返回值
            // （删掉调用就没值可赋 ⇒ 跳过改指不可能留下假读数）。
            let descriptor_points_at = crate::gpu_render::point_descriptor_at(
                &self.device,
                &set,
                &pipes.sampler,
                &dummy_texture,
            )?;
            LIVE_UI_RESOURCES.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            self.ui_builds += 1;
            self.ui = Some(UiResources {
                pipes,
                vb: None,
                // 顺序契约（**是硬契约，不是风格**）：`set` 在 `pool` 之前 ⇒ 集先销、池后销
                unified,
                unified_vs,
                unified_fs,
                set,
                pool,
                dummy_texture,
                texture: None,
                descriptor_points_at,
                uploaded: None,
                uploaded_vertices: None,
            index: None,
            indirect: None,
            uploaded_indices: None,
            uploaded_indirect: None,
            });
        }
        // `want_text = false` 时也**照样**建全部资源（含哑纹理）：
        // 统一片元着色器无条件采样 ⇒ 形状帧也必须有一个有效的 `set 0`。
        let _ = want_text;
        Ok(())
    }

    /// 图集**指纹变化**才重传纹理，并把它**改指**到渲染器级的描述符集
    /// （与 `gpu_render.rs` 同一契约：`create_texture_r8` 内部 `vkQueueWaitIdle`，
    /// 只在出现新字形时付这个代价）。
    ///
    /// 描述符集是 `UiResources` 级的（不是文本专属）：统一片元着色器无条件采样 ⇒
    /// 没有引擎时 `set` 指着 1×1 哑纹理，有引擎时指着字形图集 —— 两者**总有一个**。
    /// 指纹 `(宽, 高, 已光栅化字形数)`；真图集的宽 = 字号 ≥ 2 ⇒ 不会与哑纹理的 `(1,1,0)` 混淆。
    fn refresh_ui_atlas_texture(&mut self, engine: &TextEngine) -> GpuResult<()> {
        let (w, h) = engine.atlas().size();
        let key = (w, h, engine.rasterized_glyphs());
        let up_to_date = self
            .ui
            .as_ref()
            .is_some_and(|u| u.uploaded == Some(key));
        if up_to_date {
            return Ok(());
        }
        let data = engine.atlas().coverage().to_vec();
        let texture = self.device.create_texture_r8(w, h, &data)?;
        if let Some(u) = self.ui.as_mut() {
            // 改指 + **同处记录**（读数只能来自这个返回值 ⇒ 跳过改指必然跳过记录）
            u.descriptor_points_at = crate::gpu_render::point_descriptor_at(
                &self.device,
                &u.set,
                &u.pipes.sampler,
                &texture,
            )?;
            u.texture = Some(texture);
            u.uploaded = Some(key);
        }
        Ok(())
    }

    /// 录制第 `slot` 个命令缓冲：清屏 + **一次**绑定/绘制（+ 可选回读复制）。
    ///
    /// `unified` 是这一帧合流后的**全部**顶点（顺序即 z 序，由
    /// [`crate::vertex_unify::unify`] 保证）—— 空 ⇒ 只清屏、不发 draw。
    ///
    /// 需要 `&mut self`（M3+ B1）：要在**发真实调用的地方**自增 `stats`
    /// （draw call / 管线切换）——计数与调用同处，删掉发射就必然删掉计数。
    fn record_ui(
        &mut self,
        slot: usize,
        image_index: u32,
        unified: &[crate::vertex_unify::UnifiedVertex],
    ) -> GpuResult<()> {
        let fns = *self.device.fns();
        let cmd = self.command_buffers[slot];
        let ui = self.ui.as_ref().expect("draw_and_present 已 ensure_ui");

        // SAFETY: 命令缓冲由本结构持有；本帧独占该槽，且上一次提交已被栅栏等到完成。
        let rc = unsafe { (fns.reset_command_buffer)(cmd, 0) };
        if rc != ffi::VK_SUCCESS {
            return Err(GpuError::Driver {
                code: rc,
                message: format!("vkResetCommandBuffer 失败：{}", vk_result_name(rc)),
            });
        }
        let begin = vk::CommandBufferBeginInfo {
            s_type: vk::VK_STRUCTURE_TYPE_COMMAND_BUFFER_BEGIN_INFO,
            p_next: ptr::null(),
            flags: 0,
            p_inheritance_info: ptr::null(),
        };
        // SAFETY: `begin` 在栈上存活；句柄有效。
        let rc = unsafe { (fns.begin_command_buffer)(cmd, &begin) };
        if rc != ffi::VK_SUCCESS {
            return Err(GpuError::Driver {
                code: rc,
                message: format!("vkBeginCommandBuffer 失败：{}", vk_result_name(rc)),
            });
        }

        // ★ 主机刚写进**统一**顶点缓冲 → GPU 的 VERTEX_INPUT 要读它。
        //   **B5-2 起只有一块缓冲 ⇒ 一帧最多一条**（从前是两块各一条）。
        //   参数与离屏同一组语义：`HOST/HOST_WRITE` → `VERTEX_INPUT/VERTEX_ATTRIBUTE_READ`。
        if self.ui_barrier {
            if let Some(vb) = ui.vb.as_ref() {
                let buffer = vb.buffer.handle();
                let barrier = vk::BufferMemoryBarrier {
                    s_type: vk::VK_STRUCTURE_TYPE_BUFFER_MEMORY_BARRIER,
                    p_next: ptr::null(),
                    src_access_mask: VK_ACCESS_HOST_WRITE,
                    dst_access_mask: VK_ACCESS_VERTEX_ATTRIBUTE_READ,
                    src_queue_family_index: vk::QUEUE_FAMILY_IGNORED,
                    dst_queue_family_index: vk::QUEUE_FAMILY_IGNORED,
                    buffer,
                    offset: 0,
                    size: vk::WHOLE_SIZE,
                };
                // SAFETY: 结构体在栈上存活；命令缓冲处于录制状态；句柄有效。
                unsafe {
                    (fns.cmd_pipeline_barrier)(
                        cmd,
                        VK_PIPELINE_STAGE_HOST,
                        VK_PIPELINE_STAGE_VERTEX_INPUT,
                        0,
                        0,
                        ptr::null(),
                        1,
                        &barrier,
                        0,
                        ptr::null(),
                    );
                }
                // 计数与真实调用同处（review I-2）：删掉发射就必然删掉计数
                self.ui_host_to_vertex_barriers += 1;
            }
        }
        // 索引 / 间接缓冲各自只在**这一帧真的重传了**时发一条（与顶点缓冲同一套语义）。
        // 目标阶段/访问位不同：索引在 `VERTEX_INPUT` 被读、间接命令在 `DRAW_INDIRECT` 被读。
        if self.ui_index_barrier {
            if let Some(buf) = ui.index.as_ref() {
                emit_ui_buffer_barrier(
                    cmd,
                    fns,
                    buf.buffer.handle(),
                    VK_PIPELINE_STAGE_VERTEX_INPUT,
                    VK_ACCESS_INDEX_READ,
                );
                // 计数与真实调用同处（与顶点那条同一套理由）：删掉发射就必然删掉计数
                self.ui_index_barriers += 1;
            }
        }
        if self.ui_indirect_barrier {
            if let Some(buf) = ui.indirect.as_ref() {
                emit_ui_buffer_barrier(
                    cmd,
                    fns,
                    buf.buffer.handle(),
                    VK_PIPELINE_STAGE_DRAW_INDIRECT,
                    VK_ACCESS_INDIRECT_COMMAND_READ,
                );
                // 同上
                self.ui_indirect_barriers += 1;
            }
        }

        let clear_value = vk::ClearValue {
            color: vk::ClearColorValue {
                float32: clear_color_value(self.clear),
            },
        };
        let area = vk::Rect2D {
            offset: vk::Offset2D { x: 0, y: 0 },
            extent: vk::Extent2D {
                width: self.extent.width,
                height: self.extent.height,
            },
        };
        let pass_begin = vk::RenderPassBeginInfo {
            s_type: vk::VK_STRUCTURE_TYPE_RENDER_PASS_BEGIN_INFO,
            p_next: ptr::null(),
            render_pass: self.render_pass.handle(),
            framebuffer: self.framebuffers[image_index as usize].handle(),
            render_area: area,
            clear_value_count: 1,
            p_clear_values: &clear_value,
        };
        // SAFETY: 上述结构体都在本栈帧存活；命令缓冲处于录制状态；管线/渲染通道/帧缓冲
        // 都是本结构持有且还没有销毁。
        unsafe {
            (fns.cmd_begin_render_pass)(cmd, &pass_begin, vk::VK_SUBPASS_CONTENTS_INLINE);
            // viewport/scissor：**按策略**给 —— 动态策略必须在录制时设（管线里只有
            // `count = 1` + 空指针）；静态策略下**不能**调，否则校验层报
            // 「对静态状态发动态设置命令」。
            //
            // （「M2a 说动态在 Intel 上画不出像素」这句话的现状：**存疑**——本轮窗口路径的
            //   证据是「不设会崩、设了能上屏」，与 M2a 的「零像素且不崩」症状不同，
            //   **不能断定同因**；离屏复现仍待做。见 `viewport_strategy_from_env` 的文档。）
            if self.ui_viewport_is_dynamic {
                let viewport = vk::Viewport {
                    x: 0.0,
                    y: 0.0,
                    width: self.extent.width as f32,
                    height: self.extent.height as f32,
                    min_depth: 0.0,
                    max_depth: 1.0,
                };
                (fns.cmd_set_viewport)(cmd, 0, 1, &viewport);
                let scissor = vk::Rect2D {
                    offset: vk::Offset2D { x: 0, y: 0 },
                    extent: vk::Extent2D {
                        width: self.extent.width,
                        height: self.extent.height,
                    },
                };
                (fns.cmd_set_scissor)(cmd, 0, 1, &scissor);
            }
            // ★ **B5-2：整帧一次 bind + 一次 draw**（从前按段在两条管线间来回切）。
            //   形状与文本已合成一条顶点流 + 一条管线 ⇒ 绑定一次、一次绘制。
            //   M3+ 第 4 项下半起这次绘制走**间接**（`vkCmdDrawIndexedIndirect`，命令在缓冲里）
            //   —— 与离屏路径同一条设计；`pipeline_switches` / `draw_calls` 对任何非空帧
            //   仍恒为 **1**，且计数都写在**发调用的同一处**（删掉发射必然删掉计数）。
            if !unified.is_empty() {
                (fns.cmd_bind_pipeline)(
                    cmd,
                    vk::VK_PIPELINE_BIND_POINT_GRAPHICS,
                    ui.unified.handle(),
                );
                // 计数与真实调用同处（B1）
                self.stats.pipeline_switches += 1;
                let vb = ui.vb.as_ref().expect("有统一顶点 ⇒ 缓冲已上传");
                let offset: vk::DeviceSize = 0;
                (fns.cmd_bind_vertex_buffers)(cmd, 0, 1, &vb.buffer.handle(), &offset);
                // 索引缓冲：`u32` 索引、偏移 0（`VK_INDEX_TYPE_UINT32` = 1）
                let ib = ui.index.as_ref().expect("有统一顶点 ⇒ 索引缓冲已就绪");
                (fns.cmd_bind_index_buffer)(
                    cmd,
                    ib.buffer.handle(),
                    0,
                    crate::device::VK_INDEX_TYPE_UINT32,
                );
                // `set 0 / binding 0`：**恒有**（字形图集或 1×1 哑纹理）。
                // 统一片元着色器无条件采样 ⇒ 少了这条绑定就是未定义行为。
                (fns.cmd_bind_descriptor_sets)(
                    cmd,
                    vk::VK_PIPELINE_BIND_POINT_GRAPHICS,
                    ui.pipes.text_layout.handle(),
                    0,
                    1,
                    &ui.set.handle(),
                    0,
                    ptr::null(),
                );
                // ★ **间接绘制**：命令来自间接缓冲（`drawCount = 1`、`stride = 20`）。
                //   两个计数都写在**这一行调用旁边** ⇒ 换回 `vkCmdDraw`（或删掉）时
                //   `indirect_draws` 必然掉到 0（`draw_calls` 是派发次数，两种发法都算）。
                let indirect_handle = ui
                    .indirect
                    .as_ref()
                    .expect("有统一顶点 ⇒ 间接命令缓冲已就绪")
                    .buffer
                    .handle();
                (fns.cmd_draw_indexed_indirect)(
                    cmd,
                    indirect_handle,
                    0,
                    1,
                    std::mem::size_of::<DrawIndexedIndirectCommand>() as u32,
                );
                // 计数与真实调用同处（B1）
                self.stats.draw_calls += 1;
                self.stats.indirect_draws += 1;
            }
            (fns.cmd_end_render_pass)(cmd);
        }

        // 回读复制（与 `record` 共用同一段实现）
        self.record_readback_copy(cmd, image_index)?;

        // SAFETY: 命令缓冲处于录制状态。
        let rc = unsafe { (fns.end_command_buffer)(cmd) };
        if rc != ffi::VK_SUCCESS {
            return Err(GpuError::Driver {
                code: rc,
                message: format!("vkEndCommandBuffer 失败：{}", vk_result_name(rc)),
            });
        }
        Ok(())
    }

    /// 录制第 `slot` 个命令缓冲：清屏到调用方给的颜色 + 画三角形（+ 可选的回读复制）。
    fn record(&self, slot: usize, image_index: u32) -> GpuResult<()> {
        let fns = *self.device.fns();
        let cmd = self.command_buffers[slot];

        // SAFETY: 命令缓冲由本结构持有；本帧独占该槽，且上一次提交已被栅栏等到完成。
        let rc = unsafe { (fns.reset_command_buffer)(cmd, 0) };
        if rc != ffi::VK_SUCCESS {
            return Err(GpuError::Driver {
                code: rc,
                message: format!("vkResetCommandBuffer 失败：{}", vk_result_name(rc)),
            });
        }
        let begin = vk::CommandBufferBeginInfo {
            s_type: vk::VK_STRUCTURE_TYPE_COMMAND_BUFFER_BEGIN_INFO,
            p_next: ptr::null(),
            flags: 0,
            p_inheritance_info: ptr::null(),
        };
        // SAFETY: `begin` 在栈上存活；句柄有效。
        let rc = unsafe { (fns.begin_command_buffer)(cmd, &begin) };
        if rc != ffi::VK_SUCCESS {
            return Err(GpuError::Driver {
                code: rc,
                message: format!("vkBeginCommandBuffer 失败：{}", vk_result_name(rc)),
            });
        }

        // 清屏色 = 调用方给的颜色（示例会用它做可断言的颜色）。
        // 转换走纯函数 [`clear_color_value`]，这样「传进去什么」可以被单测钉住。
        let clear_value = vk::ClearValue {
            color: vk::ClearColorValue {
                float32: clear_color_value(self.clear),
            },
        };
        let area = vk::Rect2D {
            offset: vk::Offset2D { x: 0, y: 0 },
            extent: vk::Extent2D {
                width: self.extent.width,
                height: self.extent.height,
            },
        };
        let pass_begin = vk::RenderPassBeginInfo {
            s_type: vk::VK_STRUCTURE_TYPE_RENDER_PASS_BEGIN_INFO,
            p_next: ptr::null(),
            render_pass: self.render_pass.handle(),
            framebuffer: self.framebuffers[image_index as usize].handle(),
            render_area: area,
            clear_value_count: 1,
            p_clear_values: &clear_value,
        };
        // SAFETY: 上述结构体都在本栈帧存活；命令缓冲处于录制状态；
        // 管线/渲染通道/帧缓冲都是本结构持有且都还没有销毁。
        unsafe {
            (fns.cmd_begin_render_pass)(cmd, &pass_begin, vk::VK_SUBPASS_CONTENTS_INLINE);
            (fns.cmd_bind_pipeline)(
                cmd,
                vk::VK_PIPELINE_BIND_POINT_GRAPHICS,
                self.pipeline.handle(),
            );
            // 动态 viewport/scissor：管线里没写死尺寸，这里每帧给
            let viewport = vk::Viewport {
                x: 0.0,
                y: 0.0,
                width: self.extent.width as f32,
                height: self.extent.height as f32,
                min_depth: 0.0,
                max_depth: 1.0,
            };
            (fns.cmd_set_viewport)(cmd, 0, 1, &viewport);
            let scissor = vk::Rect2D {
                offset: vk::Offset2D { x: 0, y: 0 },
                extent: vk::Extent2D {
                    width: self.extent.width,
                    height: self.extent.height,
                },
            };
            (fns.cmd_set_scissor)(cmd, 0, 1, &scissor);
            (fns.cmd_draw)(cmd, 3, 1, 0, 0);
            (fns.cmd_end_render_pass)(cmd);
        }

        // —— 回读复制：**必须在 present 之前**，且布局要交还 `PRESENT_SRC_KHR` ——
        // （实现抽在 [`Self::record_readback_copy`] 里，两条绘制路径共用同一份）
        self.record_readback_copy(cmd, image_index)?;

        // SAFETY: 命令缓冲处于录制状态。
        let rc = unsafe { (fns.end_command_buffer)(cmd) };
        if rc != ffi::VK_SUCCESS {
            return Err(GpuError::Driver {
                code: rc,
                message: format!("vkEndCommandBuffer 失败：{}", vk_result_name(rc)),
            });
        }
        Ok(())
    }

    /// 把「图像 → 回读缓冲」的复制录进 `cmd`（`record` 与 `record_ui` 共用同一份实现）。
    ///
    /// 契约（写在这里免得被复制时漏掉）：
    /// - **必须在 present 之前**：`vkQueuePresentKHR` 一提交，图像所有权就归呈现引擎，
    ///   应用再碰它是未定义行为；
    /// - 渲染通道的 `finalLayout` 是 `PRESENT_SRC_KHR` ⇒ 这里的 `oldLayout` 就是它，
    ///   复制完还要转回 `PRESENT_SRC_KHR`（present 要求图像处于该布局）；
    /// - `readback_enabled` 关掉时**什么都不录**（调用方必须查 `last_frame_readback`，
    ///   不许拿陈旧数据冒充）。
    fn record_readback_copy(&self, cmd: vk::CommandBufferHandle, image_index: u32) -> GpuResult<()> {
        let fns = *self.device.fns();
        if self.readback_enabled {
            if let Some(rb) = &self.readback {
                let image = self.swapchain.images()[image_index as usize];
                let to_transfer = vk::ImageMemoryBarrier {
                    s_type: vk::VK_STRUCTURE_TYPE_IMAGE_MEMORY_BARRIER,
                    p_next: ptr::null(),
                    // 渲染通道刚刚写完（finalLayout 转换也是这次写的一部分）
                    src_access_mask: vk::VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT,
                    dst_access_mask: vk::VK_ACCESS_TRANSFER_READ_BIT,
                    old_layout: vk::VK_IMAGE_LAYOUT_PRESENT_SRC_KHR,
                    new_layout: vk::VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL,
                    src_queue_family_index: vk::QUEUE_FAMILY_IGNORED,
                    dst_queue_family_index: vk::QUEUE_FAMILY_IGNORED,
                    image,
                    subresource_range: color_subresource_range(),
                };
                let copy = vk::BufferImageCopy {
                    buffer_offset: 0,
                    // 0 = 与图像宽度一致 ⇒ 紧排、无行尾 padding（与 [`readback_row_pitch`] 一致）
                    buffer_row_length: 0,
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
                let back_to_present = vk::ImageMemoryBarrier {
                    s_type: vk::VK_STRUCTURE_TYPE_IMAGE_MEMORY_BARRIER,
                    p_next: ptr::null(),
                    src_access_mask: vk::VK_ACCESS_TRANSFER_READ_BIT,
                    // 呈现引擎的读由 `vkQueuePresentKHR` 等的信号量同步 ⇒ 这里到 BOTTOM 即可
                    dst_access_mask: 0,
                    old_layout: vk::VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL,
                    new_layout: vk::VK_IMAGE_LAYOUT_PRESENT_SRC_KHR,
                    src_queue_family_index: vk::QUEUE_FAMILY_IGNORED,
                    dst_queue_family_index: vk::QUEUE_FAMILY_IGNORED,
                    image,
                    subresource_range: color_subresource_range(),
                };
                // SAFETY: 上述结构体都在本栈帧存活；命令缓冲处于录制状态；
                // 图像/缓冲都由本结构持有且都在同一队列族上（EXCLUSIVE 共享模式）。
                unsafe {
                    (fns.cmd_pipeline_barrier)(
                        cmd,
                        vk::VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT,
                        vk::VK_PIPELINE_STAGE_TRANSFER_BIT,
                        0,
                        0,
                        ptr::null(),
                        0,
                        ptr::null(),
                        1,
                        &to_transfer,
                    );
                    (fns.cmd_copy_image_to_buffer)(
                        cmd,
                        image,
                        vk::VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL,
                        rb.buffer.handle(),
                        1,
                        &copy,
                    );
                    (fns.cmd_pipeline_barrier)(
                        cmd,
                        vk::VK_PIPELINE_STAGE_TRANSFER_BIT,
                        vk::VK_PIPELINE_STAGE_BOTTOM_OF_PIPE_BIT,
                        0,
                        0,
                        ptr::null(),
                        0,
                        ptr::null(),
                        1,
                        &back_to_present,
                    );
                }
            }
        }
        Ok(())
    }
}

impl Drop for WindowedRenderer {
    fn drop(&mut self) {
        // 先等 GPU 空闲，然后**字段按声明顺序**（依赖倒序）析构。
        // 这里不手动销毁各对象：`Owned*`/`Swapchain`/`Surface`/`Instance` 各自的 Drop
        // 已经负责，而字段顺序保证了先后关系 —— 手动再写一遍反而容易漏。
        if let Err(e) = self.device.wait_idle() {
            eprintln!("[deer-vk] 关闭窗口渲染器时 vkDeviceWaitIdle 失败：{e}（仍按顺序销毁）");
        }
    }
}

/// 交换链图像的完整子资源范围（1 mip / 1 layer / COLOR）。
fn color_subresource_range() -> vk::ImageSubresourceRange {
    vk::ImageSubresourceRange {
        aspect_mask: vk::VK_IMAGE_ASPECT_COLOR_BIT,
        base_mip_level: 0,
        level_count: 1,
        base_array_layer: 0,
        layer_count: 1,
    }
}

/// 建回读暂存缓冲：`HOST_VISIBLE | HOST_COHERENT`（⇒ 不需要显式 flush/invalidate）。
///
/// 尺寸 = [`readback_row_pitch`] × 高（紧排，无行尾 padding）。
fn create_readback_target(device: &VkDevice, extent: Extent) -> GpuResult<ReadbackTarget> {
    let height = extent.height.max(1);
    let size = readback_row_pitch(extent.width) as u64 * height as u64;
    if size == 0 || extent.width == 0 {
        return Err(GpuError::Unsupported(
            "回读缓冲尺寸为 0（交换链宽或高为 0）".to_string(),
        ));
    }
    let fns = device.fns();

    let info = vk::BufferCreateInfo {
        s_type: vk::VK_STRUCTURE_TYPE_BUFFER_CREATE_INFO,
        p_next: ptr::null(),
        flags: 0,
        size,
        usage: vk::VK_BUFFER_USAGE_TRANSFER_DST_BIT,
        sharing_mode: vk::VK_SHARING_MODE_EXCLUSIVE,
        queue_family_index_count: 0,
        p_queue_family_indices: ptr::null(),
    };
    let mut buffer: vk::BufferHandle = ptr::null_mut();
    // SAFETY: `info` 在栈上存活；`buffer` 是可写输出。
    let rc = unsafe { (fns.create_buffer)(device.handle(), &info, ptr::null(), &mut buffer) };
    if rc != ffi::VK_SUCCESS {
        return Err(GpuError::Driver {
            code: rc,
            message: format!(
                "vkCreateBuffer（回读缓冲 {size} 字节）失败：{}",
                vk_result_name(rc)
            ),
        });
    }
    let buffer = OwnedBuffer {
        handle: buffer,
        device: device.handle(),
        destroy: fns.destroy_buffer,
    };

    let mut req = std::mem::MaybeUninit::<vk::MemoryRequirements>::uninit();
    // SAFETY: 该函数完整写入结构体；缓冲刚创建且有效。
    unsafe {
        (fns.get_buffer_memory_requirements)(device.handle(), buffer.handle(), req.as_mut_ptr())
    };
    let req = unsafe { req.assume_init() };
    let memory_type_index = pick_memory_type(
        device.memory_properties(),
        req.memory_type_bits,
        vk::VK_MEMORY_PROPERTY_HOST_VISIBLE_BIT | vk::VK_MEMORY_PROPERTY_HOST_COHERENT_BIT,
    )?;

    let alloc = vk::MemoryAllocateInfo {
        s_type: vk::VK_STRUCTURE_TYPE_MEMORY_ALLOCATE_INFO,
        p_next: ptr::null(),
        allocation_size: req.size,
        memory_type_index,
    };
    let mut memory: vk::DeviceMemoryHandle = ptr::null_mut();
    // SAFETY: `alloc` 在栈上存活；`memory` 是可写输出。
    let rc = unsafe { (fns.allocate_memory)(device.handle(), &alloc, ptr::null(), &mut memory) };
    if rc != ffi::VK_SUCCESS {
        return Err(GpuError::Driver {
            code: rc,
            message: format!(
                "vkAllocateMemory（回读缓冲，类型 {memory_type_index}）失败：{}",
                vk_result_name(rc)
            ),
        });
    }
    let memory = OwnedMemory {
        handle: memory,
        device: device.handle(),
        free: fns.free_memory,
    };
    // SAFETY: 缓冲与内存都是本函数刚创建的对象，兼容性由 memory_type_bits 保证；offset 0 合法。
    let rc =
        unsafe { (fns.bind_buffer_memory)(device.handle(), buffer.handle(), memory.handle(), 0) };
    if rc != ffi::VK_SUCCESS {
        return Err(GpuError::Driver {
            code: rc,
            message: format!("vkBindBufferMemory（回读缓冲）失败：{}", vk_result_name(rc)),
        });
    }

    Ok(ReadbackTarget {
        buffer,
        memory,
        size,
    })
}

/// 从设备的内存类型里挑一个同时满足 `required` 所有位的。
///
/// 与 `offscreen.rs` 的同名逻辑一致（那边是私有的）；属性来自**物理设备**
/// （逻辑设备上拿不到），由 `VkDevice::memory_properties()` 提供。
fn pick_memory_type(
    props: &vk::PhysicalDeviceMemoryProperties,
    type_bits: u32,
    required: u32,
) -> GpuResult<u32> {
    let count = props.memory_type_count.min(32);
    for i in 0..count {
        let m = &props.memory_types[i as usize];
        if type_bits & (1 << i) == 0 {
            continue;
        }
        if m.property_flags & required == required {
            return Ok(i);
        }
    }
    Err(GpuError::Unsupported(format!(
        "找不到满足属性 {required:#x} 的内存类型（缓冲区允许的 type_bits = {type_bits:#x}）"
    )))
}

/// 每张交换链图像建一个帧缓冲。
fn create_framebuffers(
    device: &VkDevice,
    render_pass: &RenderPass,
    views: &[vk::ImageViewHandle],
    extent: Extent,
) -> GpuResult<Vec<OwnedFramebuffer>> {
    let fns = device.fns();
    let mut out = Vec::with_capacity(views.len());
    for (i, view) in views.iter().enumerate() {
        let info = vk::FramebufferCreateInfo {
            s_type: vk::VK_STRUCTURE_TYPE_FRAMEBUFFER_CREATE_INFO,
            p_next: ptr::null(),
            flags: 0,
            render_pass: render_pass.handle(),
            attachment_count: 1,
            p_attachments: view,
            width: extent.width,
            height: extent.height,
            layers: 1,
        };
        let mut handle: vk::FramebufferHandle = ptr::null_mut();
        // SAFETY: `info` 与 `view` 在调用期间存活；`handle` 是可写输出。
        let rc =
            unsafe { (fns.create_framebuffer)(device.handle(), &info, ptr::null(), &mut handle) };
        if rc != ffi::VK_SUCCESS {
            return Err(GpuError::Driver {
                code: rc,
                message: format!(
                    "vkCreateFramebuffer 失败（第 {i} 张图像，{}×{}）：{}",
                    extent.width,
                    extent.height,
                    vk_result_name(rc)
                ),
            });
        }
        out.push(OwnedFramebuffer {
            handle,
            device: device.handle(),
            destroy: fns.destroy_framebuffer,
        });
    }
    Ok(out)
}

/// 建命令池（允许逐缓冲 `vkResetCommandBuffer`）。
fn create_command_pool(device: &VkDevice) -> GpuResult<OwnedCommandPool> {
    let fns = device.fns();
    let info = vk::CommandPoolCreateInfo {
        s_type: vk::VK_STRUCTURE_TYPE_COMMAND_POOL_CREATE_INFO,
        p_next: ptr::null(),
        flags: vk::VK_COMMAND_POOL_CREATE_RESET_COMMAND_BUFFER_BIT,
        queue_family_index: device.queue_family_index(),
    };
    let mut handle: vk::CommandPoolHandle = ptr::null_mut();
    // SAFETY: `info` 在栈上存活；`handle` 是可写输出。
    let rc = unsafe { (fns.create_command_pool)(device.handle(), &info, ptr::null(), &mut handle) };
    if rc != ffi::VK_SUCCESS {
        return Err(GpuError::Driver {
            code: rc,
            message: format!("vkCreateCommandPool 失败：{}", vk_result_name(rc)),
        });
    }
    Ok(OwnedCommandPool {
        handle,
        device: device.handle(),
        destroy: fns.destroy_command_pool,
    })
}

/// 从池里分配 `count` 个主命令缓冲。
fn allocate_command_buffers(
    device: &VkDevice,
    pool: vk::CommandPoolHandle,
    count: usize,
) -> GpuResult<Vec<vk::CommandBufferHandle>> {
    let fns = device.fns();
    let info = vk::CommandBufferAllocateInfo {
        s_type: vk::VK_STRUCTURE_TYPE_COMMAND_BUFFER_ALLOCATE_INFO,
        p_next: ptr::null(),
        command_pool: pool,
        level: vk::VK_COMMAND_BUFFER_LEVEL_PRIMARY,
        command_buffer_count: count as u32,
    };
    let mut buffers = vec![ptr::null_mut(); count];
    // SAFETY: `info` 在栈上存活；`buffers` 容量与 `command_buffer_count` 一致。
    let rc =
        unsafe { (fns.allocate_command_buffers)(device.handle(), &info, buffers.as_mut_ptr()) };
    if rc != ffi::VK_SUCCESS {
        return Err(GpuError::Driver {
            code: rc,
            message: format!(
                "vkAllocateCommandBuffers 失败（{count} 个）：{}",
                vk_result_name(rc)
            ),
        });
    }
    Ok(buffers)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// SPIR-V `StorageClass`（测试解接口用）：`Input = 1`、`Output = 3`。
    const STORAGE_CLASS_INPUT: u32 = 1;
    const STORAGE_CLASS_OUTPUT: u32 = 3;

    #[test]
    fn frame_outcome_is_compareable() {
        assert_ne!(FrameOutcome::Presented, FrameOutcome::OutOfDate);
        assert_eq!(FrameOutcome::Presented, FrameOutcome::Presented);
    }

    /// viewport 诊断开关**必须容忍两侧空白**：`cmd /c "set DEER_VK_WINDOW_VIEWPORT=static && …"`
    /// 的实际值是 `"static "`（带尾空格），严格判等会静默退回 `dynamic` ⇒
    /// 「以为在测静态、实际测的是动态」。理由见 [`viewport_strategy_from_env`]。
    #[test]
    fn viewport_strategy_from_value_tolerates_surrounding_whitespace() {
        let statik = pipelines::ViewportStrategy::Static {
            width: 1,
            height: 1,
        };
        for v in [
            Some("static"),
            Some("static "),
            Some(" static "),
            Some("STATIC"),
            Some("\tStatic"),
        ] {
            assert_eq!(viewport_strategy_from_value(v), statik, "{v:?} 必须判为静态");
        }
        // 默认（未设 / 空 / 其它）⇒ dynamic —— 默认值才是产品行为
        for v in [
            None,
            Some(""),
            Some(" "),
            Some("dynamic"),
            Some("dynamic "),
            Some("yse"),
        ] {
            assert_eq!(
                viewport_strategy_from_value(v),
                pipelines::ViewportStrategy::Dynamic,
                "{v:?} 必须退回默认 dynamic"
            );
        }
    }

    #[test]
    fn triangle_is_not_degenerate() {
        // 退化三角形（面积为 0 / 三点共线）会被光栅化器直接丢掉 —— 那样「窗口一片清屏色」
        // 就会被误判成「绘制有问题」。这里把顶点质量钉住。
        let [a, b, c] = TRIANGLE_NDC;
        let area = (b[0] - a[0]) * (c[1] - a[1]) - (c[0] - a[0]) * (b[1] - a[1]);
        assert!(area.abs() > 0.05, "三角形面积太小：{area}");
        for p in [a, b, c] {
            assert!(p[0].abs() <= 1.0 && p[1].abs() <= 1.0, "顶点必须在 NDC 内：{p:?}");
        }
        // 颜色要与典型清屏色区分得开（否则「画没画」肉眼看不出来）
        const { assert!(TRIANGLE_COLOR[2] > 0.8, "三角形应当明显偏蓝") };
    }

    #[test]
    fn frames_in_flight_is_at_least_one() {
        const { assert!(FRAMES_IN_FLIGHT >= 1, "至少在飞的帧数要为 1") };
    }

    /// **界面管线用到的四个数值常量必须逐位正确**。
    ///
    /// 理由与 `gpu_render.rs` 的同名测试相同：`1 << 2`（VERTEX_INPUT / VERTEX_ATTRIBUTE_READ）
    /// 与 `1 << 5`（细分求值 / SHADER_READ）曾经被写混过，而**校验层之外没有任何东西会报错**
    /// —— 屏障写错只是「同步不对」，颜色可能照样对（在同一队列、同一提交里几乎必然对），
    /// 于是这种错会一直潜伏。把确切数字钉住才叫可回归。
    #[test]
    fn ui_barrier_constants_pin_the_exact_bits() {
        assert_eq!(VK_PIPELINE_STAGE_HOST, 1 << 14, "srcStageMask = HOST");
        assert_eq!(
            VK_PIPELINE_STAGE_VERTEX_INPUT,
            1 << 2,
            "dstStageMask = VERTEX_INPUT = 0x4（**不是** 1<<5 的细分求值）"
        );
        assert_eq!(VK_ACCESS_HOST_WRITE, 1 << 14, "srcAccessMask = HOST_WRITE");
        assert_eq!(
            VK_ACCESS_VERTEX_ATTRIBUTE_READ,
            1 << 2,
            "dstAccessMask = VERTEX_ATTRIBUTE_READ = 0x4（**不是** 1<<5 的 SHADER_READ）"
        );

        // ── 间接绘制（M3+ 第 4 项下半）的两个目标掩码 ──
        // 变异验证（**已实测**）：把 `VK_PIPELINE_STAGE_DRAW_INDIRECT` 改成
        // `VK_PIPELINE_STAGE_VERTEX_INPUT`（「和顶点一样就行」这个很自然的笔误）⇒ 本测试变红；
        // 把 `VK_ACCESS_INDEX_READ` 写成 `VK_ACCESS_INDIRECT_COMMAND_READ` ⇒ 也变红。
        // 两种写法都**不会**被驱动或校验层拒绝 —— 只会让屏障不覆盖真正读那块缓冲的阶段。
        assert_eq!(
            VK_PIPELINE_STAGE_DRAW_INDIRECT,
            1 << 1,
            "间接命令在 DRAW_INDIRECT = 0x2 阶段被读（**不是** VERTEX_INPUT 的 0x4）"
        );
        assert_eq!(
            VK_ACCESS_INDIRECT_COMMAND_READ,
            1 << 0,
            "dstAccessMask = INDIRECT_COMMAND_READ = 0x1（**不是** INDEX_READ 的 0x2）"
        );
        assert_eq!(
            VK_ACCESS_INDEX_READ,
            1 << 1,
            "dstAccessMask = INDEX_READ = 0x2（与 DRAW_INDIRECT 同值但不同枚举 —— 不是笔误）"
        );
    }

    /// **形状管线的顶点属性表**（现在只有共用的一份：`gpu_render::vertex_attrs`）：
    /// 与 `GpuVertex` 的 `#[repr(C)]` 布局逐字节一致。
    ///
    /// M3c 之前窗口路径自己抄了一份（`ui_shape_attrs`），已按控制者要求**收敛成一处**；
    /// 这条测试现在直接钉**那一处**。
    #[test]
    fn shared_shape_attrs_match_the_frozen_vertex_layout() {
        let a = crate::gpu_render::vertex_attrs();
        assert_eq!(a.len(), 4, "四个 location（少了会报 Input-07904）");
        assert_eq!(
            (a[0].location, a[0].offset),
            (0, std::mem::offset_of!(GpuVertex, pos) as u32)
        );
        assert_eq!(
            (a[1].location, a[1].offset),
            (1, std::mem::offset_of!(GpuVertex, rect) as u32)
        );
        assert_eq!(
            (a[2].location, a[2].offset),
            (2, std::mem::offset_of!(GpuVertex, radius_kind) as u32)
        );
        assert_eq!(
            (a[3].location, a[3].offset),
            (3, std::mem::offset_of!(GpuVertex, color) as u32)
        );
        // 冻结的布局：stride 44、偏移 0/8/24/28（M3a 契约）
        assert_eq!(std::mem::size_of::<GpuVertex>(), 44, "GpuVertex stride 已冻结");
        assert_eq!(a[3].offset, 28, "color 在偏移 28");
        assert_eq!(a[2].format, 100, "radius_kind 是 R32_SFLOAT（规范值 100）");
    }

    /// **文本管线的顶点属性表**（同样只有共用的一份）：`TextVertex`（stride 32 / 偏移 0, 8, 16）。
    #[test]
    fn shared_text_attrs_match_the_frozen_vertex_layout() {
        let a = crate::gpu_render::text_attrs();
        assert_eq!(a.len(), 3);
        assert_eq!((a[0].location, a[0].offset), (0, 0));
        assert_eq!((a[1].location, a[1].offset), (1, 8));
        assert_eq!((a[2].location, a[2].offset), (2, 16));
        assert_eq!(std::mem::size_of::<TextVertex>(), 32, "TextVertex stride 已冻结");
    }

    /// **顶点着色器的输出必须覆盖片段着色器的输入**（同一组 location）。
    ///
    /// ## 为什么需要这条（真实缺陷）
    ///
    /// 本轮最初把形状管线的 VS 写成了 `vertex_shader_from_vertex_buffer`（M2a 探针用的那个），
    /// 而形状 FS 声明了 location 0/1/2 的输入：
    ///
    /// - **驱动照样 `vkCreateGraphicsPipelines` 成功、照样画出像素**（错的是几何：矩形/半径/颜色
    ///   取不到值，界面退化成稀稀拉拉的几笔）；
    /// - 只有校验层报 `[VK ERROR] ... FS has a declared Input at Location 0 ... but the previous
    ///   stage has no Output declared there`（实测 3 条）；
    /// - 那条验收（`DEER_VK_VALIDATION=1` 零消息）抓出来之后，非底色像素从 **6572 跳到 93900**。
    ///
    /// 这条测试把「VS 的输出集合 ⊇ FS 的输入集合」变成**无 GPU 也能回归**的判据：
    /// 直接解 SPIR-V 的 `OpDecorate … Location`（输出/输入各一组）。
    #[test]
    fn shape_pipeline_shader_interface_matches() {
        // 窗口路径用的这一对（与 `ensure_ui` 保持一致）
        let vs = spirv::vertex_shader_rect_attrs();
        let fs = spirv::fragment_shader_rect_shape();
        let vs_out = locations_of(&vs, STORAGE_CLASS_OUTPUT);
        let fs_in = locations_of(&fs, STORAGE_CLASS_INPUT);
        assert!(!fs_in.is_empty(), "形状 FS 必须有输入（否则这条测试没有意义）");
        for loc in &fs_in {
            assert!(
                vs_out.contains(loc),
                "形状 FS 在 location {loc} 有输入，但 VS 没有对应输出（接口不匹配）—— \
                 VS 输出 {vs_out:?}、FS 输入 {fs_in:?}"
            );
        }

        // 文本管线这一对同样检查（T2 的着色器）
        let vs = spirv::vertex_shader_text();
        let fs = spirv::fragment_shader_text();
        let vs_out = locations_of(&vs, STORAGE_CLASS_OUTPUT);
        let fs_in = locations_of(&fs, STORAGE_CLASS_INPUT);
        assert!(!fs_in.is_empty(), "文本 FS 必须有输入");
        for loc in &fs_in {
            assert!(
                vs_out.contains(loc),
                "文本 FS 在 location {loc} 有输入，但 VS 没有对应输出；VS 输出 {vs_out:?}、FS 输入 {fs_in:?}"
            );
        }
    }

    /// 从 SPIR-V 模块里取出「某个存储类的变量被装饰的 location」。
    ///
    /// 只解两条指令：`OpVariable`（opcode 59：`result-type, result-id, storage-class, ...`）
    /// 与 `OpDecorate`（opcode 71：`target, decoration, ...`）。`Location` 装饰 = **30**。
    fn locations_of(bytes: &[u8], storage_class: u32) -> Vec<u32> {
        const OP_VARIABLE: u16 = 59;
        const OP_DECORATE: u16 = 71;
        const DECORATION_LOCATION: u32 = 30;
        let words: Vec<u32> = bytes
            .chunks_exact(4)
            .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect();
        // 变量 id → 存储类
        let mut vars: Vec<(u32, u32)> = Vec::new();
        let mut decorations: Vec<(u32, u32)> = Vec::new(); // (target, location)
        let mut i = 5; // 跳过 header
        while i < words.len() {
            let wc = (words[i] >> 16) as usize;
            let op = (words[i] & 0xffff) as u16;
            if wc == 0 || i + wc > words.len() {
                break;
            }
            match op {
                OP_VARIABLE if wc >= 4 => {
                    vars.push((words[i + 2], words[i + 3])); // (result-id, storage-class)
                }
                OP_DECORATE if wc >= 4 && words[i + 2] == DECORATION_LOCATION => {
                    decorations.push((words[i + 1], words[i + 3]));
                }
                _ => {}
            }
            i += wc;
        }
        let mut out: Vec<u32> = vars
            .iter()
            .filter(|(_, sc)| *sc == storage_class)
            .filter_map(|(id, _)| {
                decorations
                    .iter()
                    .find(|(target, _)| target == id)
                    .map(|(_, loc)| *loc)
            })
            .collect();
        out.sort_unstable();
        out
    }

    /// 「清屏色 = 调用方给的颜色」这条约定的**逐位**判据（示例的 CLEAR 就是 RGB 0x101424）。
    #[test]
    fn clear_color_value_is_exact() {
        let c = Color::rgb(0x10, 0x14, 0x24);
        let v = clear_color_value(c);
        assert_eq!(
            v,
            [16.0 / 255.0, 20.0 / 255.0, 36.0 / 255.0, 1.0],
            "通道不能被交换/缩放（B8G8R8A8 与 R8G8B8A8 的差别由驱动按格式处理）"
        );
        // 边界：0 与 255
        assert_eq!(clear_color_value(Color::rgb(0, 0, 0)), [0.0, 0.0, 0.0, 1.0]);
        assert_eq!(clear_color_value(Color::WHITE), [1.0, 1.0, 1.0, 1.0]);
        // alpha 原样传（半透明清屏是合法用法）
        let t = clear_color_value(Color::rgba(1, 2, 3, 0.25));
        assert_eq!(t[3], 0.25);
        assert_eq!(t[0..3], [1.0 / 255.0, 2.0 / 255.0, 3.0 / 255.0]);
    }

    /// sRGB 编码公式：**实测值**（本机 Intel/NVIDIA 驱动写进交换链的字节）与它逐位一致。
    ///
    /// 这三个数就是真窗口回读测试里四角读到的值 —— 把「驱动实测」与「公式」对齐之后，
    /// 像素断言才有正确参考值。
    #[test]
    fn srgb_encoding_matches_measured_driver_bytes() {
        assert_eq!(srgb_encoded_byte(0), 0);
        assert_eq!(srgb_encoded_byte(255), 255);
        // 清屏色 rgb(0x10,0x14,0x24)（线性）在 sRGB 附件里实测写成 [0x47,0x4F,0x69]
        assert_eq!(srgb_encoded_byte(0x10), 0x47, "16/255 线性 → 71");
        assert_eq!(srgb_encoded_byte(0x14), 0x4F, "20/255 线性 → 79");
        assert_eq!(srgb_encoded_byte(0x24), 0x69, "36/255 线性 → 105");
        // 单调不减（任何分段点写错都会在这里现形）
        let mut prev = 0u8;
        for v in 0..=255u8 {
            let e = srgb_encoded_byte(v);
            assert!(e >= prev, "sRGB 编码必须单调不减：{v} → {e} < {prev}");
            prev = e;
        }
        // 编码后比线性亮（sRGB 的已知性质：暗部抬升）
        assert!(srgb_encoded_byte(16) > 16);
    }
}
