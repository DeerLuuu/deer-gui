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
//! ## 一帧的流程（**M3+ B5-2 起是单管线**）
//!
//! ```text
//!  ① 入口检查：裁剪栈平衡（与 CPU 同一判据）；`unsupported` 非空 ⇒ Unsupported
//!  ② 逐命令翻译：gpu_geom::build_stream / gpu_text::build_text_stream
//!     ⇒ 形状顶点流 + 文本顶点流 + **段表**（顺序即 z 序）
//!  ③ vertex_unify::unify(形状, 文本, 段表) ⇒ **一条**统一顶点流（stride 52）
//!  ④ 顶点写进**一块** HOST_VISIBLE 顶点缓冲（每帧重传；容量不足时重建）
//!  ⑤ 录制：beginRenderPass(CLEAR) → bindPipeline(**统一**) → bindVertexBuffers(1)
//!          → bindDescriptorSets(set 0 = 图集 **或 1×1 哑纹理**)
//!          → **draw(0, 全部顶点数) 一次** → endRenderPass
//!          → 屏障(TRANSFER_SRC) → copyImageToBuffer
//!  ⑥ vkQueueSubmit + 等栅栏（有限超时）
//!  ⑦ map 暂存缓冲读回 RGBA8
//! ```
//!
//! ## B5-2：为什么从「8 draw + 8 switch」变成「1 draw + 1 switch」
//!
//! 从前形状与文本是**两条管线**，交错语料的每一段都要重绑管线 + 顶点缓冲 + 描述符集
//! ⇒ 段数 = draw 次数 = 切换次数。现在两条图元合成**一条**顶点流 + **一条**管线
//! ⇒ 整个 `DrawList` 是**一次** `vkCmdDraw(0, N)`。
//!
//! ## ⚠️ 隐式依赖：统一片元着色器**无条件采样**
//!
//! 统一 FS 用 `OpSelect` 消掉分支的代价是「文本那一支永远被算」（见
//! [`crate::spirv::fragment_shader_unified`]）⇒ **形状帧也会采样** ⇒
//! `set 0` 必须恒有一个有效的 `COMBINED_IMAGE_SAMPLER`：
//!
//! - 有 `TextEngine` 且这一帧上了图集 ⇒ 绑**字形图集**；
//! - 否则 ⇒ 绑 [`DUMMY_COVERAGE`]（`R8_UNORM` 1×1、覆盖率 255 = 全覆盖）。
//!
//! 形状段的 `uv = (-1,-1)` 是**越界**采样，靠采样器的 `ClampToEdge` 兜住
//! （`device.rs::create_sampler` 已是 `NEAREST + ClampToEdge`）；采到的值**不被采用**
//! （`is_shape` 为真时选的是形状那一支），但**采样本身必须合法**。
//! 这条依赖是**静默的**（不绑 = 未定义行为，驱动与校验层都不一定报）。
//!
//! ⚠️ **判据要找对目标（B5-3 更正）**：承重的是「**绑**一个已写入的有效描述符」，
//! **不是** [`DUMMY_COVERAGE`] 的**取值** —— 后者改成 `[0]` / `[128]` 形状帧
//! **逐字节不变**（采样结果被 `OpSelect` 丢弃）。所以判据是
//! `tests/gpu_vs_cpu.rs::a_shape_only_frame_binds_the_one_by_one_dummy_texture`
//! （删掉 `cmd_bind_descriptor_sets` / 跳过 `update_descriptor_texture` 时它会红，
//! 表现是**读回整幅 0**，不是「颜色不对」）。详见 [`DUMMY_COVERAGE`] 的说明。
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
    vk_result_name, DescriptorPool, DescriptorSet, DeviceFns, DrawIndexedIndirectCommand, RenderPass,
    Texture, VertexAttr, VkDevice, VK_BUFFER_USAGE_INDIRECT_BUFFER_BIT,
    VK_BUFFER_USAGE_INDEX_BUFFER_BIT, VK_INDEX_TYPE_UINT32,
};
use crate::ffi;
use crate::ffi_dev as vk;
use crate::gpu_geom::{self, GpuVertex};
use crate::gpu_text::{self, TextVertex};
use crate::vertex_unify::{self, UnifiedVertex};

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

// ── 间接绘制（M3+ 第 4 项下半）需要的同步位 ────────────────────────────────────
//
// 与上面 `HOST`/`VERTEX_INPUT` 同一处境：`ffi_dev.rs` 里没有这几个位，而它**不在本任务
// 的允许改动清单**里 ⇒ 按既有先例（M3a-T3 也是这么做的）把具名常量放在**使用者旁边**，
// 让纯函数产出参数、单元测试钉确切数值。
//
// 值取自本机 Vulkan SDK `1.4.357.0/include/vulkan/vulkan_core.h`：
// `VK_PIPELINE_STAGE_DRAW_INDIRECT_BIT = 0x2`、`VK_ACCESS_INDIRECT_COMMAND_READ_BIT = 0x1`、
// `VK_ACCESS_INDEX_READ_BIT = 0x2`。

/// `VK_PIPELINE_STAGE_DRAW_INDIRECT_BIT`（= `1 << 1` = `0x2`）：**读间接命令**的阶段。
const VK_PIPELINE_STAGE_DRAW_INDIRECT_BIT: u32 = 1 << 1;
/// `VK_ACCESS_INDIRECT_COMMAND_READ_BIT`（= `1 << 0` = `0x1`）。
///
/// 注意它与 `VK_ACCESS_INDEX_READ_BIT`（`0x2`）**数值不同但相邻** —— 两个都容易写混。
const VK_ACCESS_INDIRECT_COMMAND_READ_BIT: u32 = 1 << 0;
/// `VK_ACCESS_INDEX_READ_BIT`（= `1 << 1` = `0x2`）。
///
/// ⚠️ 它与 [`VK_PIPELINE_STAGE_DRAW_INDIRECT_BIT`] **数值相同、枚举不同**（前者是
/// `VkAccessFlagBits`、后者是 `VkPipelineStageFlagBits`）—— 看上去像复制粘贴错误，
/// 但规范如此；`index_and_indirect_barrier_params_pin_the_exact_masks` 钉住这两组。
const VK_ACCESS_INDEX_READ_BIT: u32 = 1 << 1;

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
///
/// ## B5-3 之后谁在读它（**消费者只剩测试**，如实登记）
///
/// 旧的两条管线已删 ⇒ 绘制路径不再建「认 `GpuVertex` 的管线」，本表**没有生产消费者**。
/// 保留它的理由与保留 `spirv.rs` 那 4 支旧着色器**是同一条**：它们是冻结产物的
/// **数据侧对照物** —— `windowed.rs::shared_shape_attrs_match_the_frozen_vertex_layout`
/// 拿它钉住「`GpuVertex` 字段偏移 ⇒ 冻结着色器的 location」这条对应，
/// 而 `spirv_val.rs` 在**着色器侧**钉住同一条契约。删掉本表 = 那条对应只剩一半。
///
/// `#[cfg_attr(not(test), allow(dead_code))]` 是**精确**表达（不是全局 `allow`）：
/// 唯一读者在 `#[cfg(test)]` 里 ⇒ 非测试构建下它确实未被使用。
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn vertex_attrs() -> [VertexAttr; 4] {
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
///
/// 消费者与 [`vertex_attrs`] 同一条（B5-3 起只剩单测：
/// `windowed.rs::shared_text_attrs_match_the_frozen_vertex_layout`）。
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn text_attrs() -> [VertexAttr; 3] {
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

/// **统一管线**的顶点属性表：`UnifiedVertex`（**stride 52**：`pos` 0 / `rect` 8 /
/// `radius_kind` 24 / `color` 28 / `uv` 44）。
///
/// 与 [`crate::spirv::vertex_shader_unified`] 的 `location 0..4` **逐字段对应**，
/// 偏移用 `offset_of!` 取 ⇒ 结构上不可能与 `vertex_unify.rs` 的 `#[repr(C)]` 布局漂移
/// （`tests/gpu_vs_cpu.rs::unified_vertex_layout_matches_the_unified_attribute_offsets`
/// 另外钉住字面数字 52 / 0 / 8 / 24 / 28 / 44）。
///
/// `pub(crate)`：窗口路径（`windowed.rs`）用**同一份** —— 两条渲染路径的顶点布局
/// 必须逐字相同，各写一份就是「只改了一边」的温床（与 [`vertex_attrs`] 同一规矩：
/// 全仓库只有一份，两条路径都 `use` 它）。
pub(crate) fn unified_attrs() -> [VertexAttr; 5] {
    [
        VertexAttr {
            location: 0,
            format: vk::VK_FORMAT_R32G32_SFLOAT,
            offset: std::mem::offset_of!(UnifiedVertex, pos) as u32,
        },
        VertexAttr {
            location: 1,
            format: vk::VK_FORMAT_R32G32B32A32_SFLOAT,
            offset: std::mem::offset_of!(UnifiedVertex, rect) as u32,
        },
        VertexAttr {
            location: 2,
            format: VK_FORMAT_R32_SFLOAT,
            offset: std::mem::offset_of!(UnifiedVertex, radius_kind) as u32,
        },
        VertexAttr {
            location: 3,
            format: vk::VK_FORMAT_R32G32B32A32_SFLOAT,
            offset: std::mem::offset_of!(UnifiedVertex, color) as u32,
        },
        VertexAttr {
            location: 4,
            format: vk::VK_FORMAT_R32G32_SFLOAT,
            offset: std::mem::offset_of!(UnifiedVertex, uv) as u32,
        },
    ]
}

/// **哑纹理**的 1×1 覆盖率数据（`R8_UNORM`，值 255 = 覆盖率 1.0）。
///
/// 统一片元着色器**无条件采样**（无分支的代价）⇒ 形状帧也必须绑定一张有效纹理。
/// 这张 1×1 的「全白覆盖率」就是那个绑定物：形状段的 `uv = (-1,-1)` 被
/// `ClampToEdge` 兜到唯一的那个纹素上，采到 255 ⇒ 即使**误用**文本那一支，
/// `color.a * 1.0` 也还是原色（不会把像素变没）。
///
/// ## ⚠️ 分清两件事：**取值不承重；绑定承重**（实测结论，别再把两者混起来）
///
/// - **取值不承重**：把这里改成 `[0]` 甚至 `[128]`，形状帧**逐字节完全相同**
///   （实测：`gpu_vs_cpu` 27 passed / 0 failed，形状帧最大通道差仍是 0）。
///   原因是可断言的：统一片元着色器里 `out = select(is_shape, shape_out, text_out)`，
///   而形状段的 `is_shape` 恒真 ⇒ 采样结果被丢弃，它的**值**影响不了任何像素。
///   所以**不存在**「把这里改成 `[0]` ⇒ 形状帧必红」这种判据 —— 早先代码注释里
///   那句「变异 B 必须让形状帧变红」与实测**相反**，已删。
/// - **绑定承重**：**必须绑一个已写入的有效描述符**，否则整条 `vkCmdDraw` 是**未定义**的
///   —— 实测的表现不是「形状帧颜色不对」，而是**读回整幅全 0**（连清屏色都没了，
///   `GPU=[0,0,0,0]`、形状帧差 255）。变异记录（复审独立复现）：
///   删掉录制里的 `cmd_bind_descriptor_sets` ⇒ **默认档 18 failed / 9 passed**、
///   校验层报 `VUID-vkCmdDraw-None-08600`（"statically uses set n 但没绑"）；
///   描述符集**从不写入**（跳过 `update_descriptor_texture`）⇒ 校验层报
///   `VUID-vkCmdDraw-None-08114`、默认档 9 failed。
///
/// 这条依赖**在默认档就有常驻判据**：
/// `tests/gpu_vs_cpu.rs::a_shape_only_frame_binds_the_one_by_one_dummy_texture`
/// （要求形状帧逐字节 0）在上面两种变异下都会红 ⇒ **不需要**再补一条。
/// 本常量的存在理由因此是**接口契约**（无分支着色器要求恒有一张有效纹理），
/// 而不是「它的值会让像素变」。
pub(crate) const DUMMY_COVERAGE: [u8; 1] = [255];

// 为什么**不需要**一个「当前绑的是哑纹理」的哨兵指纹（推理留在注释里，别造无用常量）：
//
// `TextResources.uploaded` 的 `None` 已经表达了「这个引擎的图集还没传过」
// （那时描述符集里是 `GpuGeometryRenderer::new` 绑好的 1×1 哑纹理），
// 而真实图集的宽 = 字号 ≥ 2 ⇒ 它的指纹**不可能**等于 `(1, 1, 0)`
// ⇒ 「哑纹理被误判成已上传的图集」在结构上不会发生。
// 先前我写过一个 `DUMMY_TEXTURE_FINGERPRINT` 常量来「表达」这件事，但它**不参与任何比较**
// ⇒ 只是个会挂 `dead_code` 警告的摆设，已删。

/// 图集纹理的**指纹** `(宽, 高, 已光栅化字形数)`：三者任一变化就重传。
///
/// 依据：`GlyphAtlas` 只**追加/增高**、不淘汰，内容只在「新字形入图集」时改变，
/// 而那只会让 `rasterized_glyphs()` 增加 ⇒ 这个三元组是充分的（M3b-T4 的契约）。
pub(crate) fn texture_fingerprint(engine: &TextEngine) -> (u32, u32, usize) {
    let (w, h) = engine.atlas().size();
    (w, h, engine.rasterized_glyphs())
}

/// 建**统一管线**（B5-2）：`vertex_shader_unified` + `fragment_shader_unified`，
/// 顶点布局 = [`unified_attrs`]（stride 52），其余管线状态**仍然只有一处来源**
/// （[`crate::pipelines::shape_state`] —— 它产出的就是「除颜色格式/viewport/顶点布局外
/// 与文本管线逐字相同」的那份状态，`pipelines.rs` 的测试钉着这条不变式）。
///
/// ## 为什么它在这里、而不在 `pipelines.rs`
///
/// 顶点布局是 [`UnifiedVertex`]（`stride 52` / 5 个 location）—— 那个类型属于本模块。
/// 共用层（[`crate::pipelines::build_pipeline_resources`]）只提供它需要的三样资源
/// （管线布局 / `set 0` 布局 / 采样器），**不建管线**（B5-3 起不再建旧的两条管线）。
///
/// `pub`：两条绘制路径（离屏 [`GpuGeometryRenderer::new`] 与窗口
/// `windowed::ensure_ui`）都用它，`tests/pipeline_smoke.rs` 也用它断言
/// 「两种颜色格式 / 两种 viewport 策略都能建出统一管线」—— 这是 M3c-T1
/// 「共用层能服务两条路径」那条判据在 B5-3 之后的落点（原来落在已删除的
/// `build_pipelines` 上）。
///
/// **生命周期契约**：返回的管线引用了 `layout`（→ `text_set_layout`）与两个着色器模块
/// ⇒ 调用方必须把它们声明在管线**之前**（Rust 按声明顺序析构 ⇒ 管线先销毁）。
/// 建统一管线（形状 + 文本），FS 由调用方给。
///
/// 抽出来的理由：T1.3 的**纹理 quad** 要的是**同一支顶点着色器 + 同一套顶点布局 +
/// 另一个片元着色器** ⇒ 另一条管线。若把建管线的过程抄第二份，「顶点布局」就会有两处
/// 定义，而 `UnifiedVertex` 的布局是**冻结**的（`unified_vertex_layout_is_frozen` 只钉一处）。
pub fn build_unified_pipeline_with_fs(
    device: &VkDevice,
    render_pass: &RenderPass,
    color_format: i32,
    viewport: crate::pipelines::ViewportStrategy,
    layout: &crate::device::PipelineLayout,
    fs_bytes: &[u8],
) -> GpuResult<(crate::device::Pipeline, crate::device::ShaderModule, crate::device::ShaderModule)>
{
    let vs = device.create_shader_module(&crate::spirv::vertex_shader_unified())?;
    let fs = device.create_shader_module(fs_bytes)?;
    let state = crate::pipelines::shape_state(
        color_format,
        viewport,
        std::mem::size_of::<UnifiedVertex>() as u32,
        unified_attrs().to_vec(),
    );
    let pipeline = device.create_pipeline_from_state(
        &state,
        &[
            (&vs, vk::VK_SHADER_STAGE_VERTEX_BIT),
            (&fs, vk::VK_SHADER_STAGE_FRAGMENT_BIT),
        ],
        layout,
        render_pass,
    )?;
    Ok((pipeline, vs, fs))
}

/// 建**统一**管线（形状 + 文本，覆盖率语义）。等价于
/// [`build_unified_pipeline_with_fs`] 传入 [`crate::spirv::fragment_shader_unified`]。
pub fn build_unified_pipeline(
    device: &VkDevice,
    render_pass: &RenderPass,
    color_format: i32,
    viewport: crate::pipelines::ViewportStrategy,
    layout: &crate::device::PipelineLayout,
) -> GpuResult<(crate::device::Pipeline, crate::device::ShaderModule, crate::device::ShaderModule)>
{
    build_unified_pipeline_with_fs(
        device,
        render_pass,
        color_format,
        viewport,
        layout,
        &crate::spirv::fragment_shader_unified(),
    )
}


/// 把 `set` 改指到 `texture`，并**在同一处**返回「它现在指着谁」。
///
/// ## 为什么是「返回值」而不是「自己记一个字段」
///
/// 本项目既有原则是「**读数与真实调用同处**」（见 [`RenderStats`] 的说明）。
/// 这条更进一步：调用点是 `vkUpdateDescriptorSets` **本身**，而记录**只能是它的返回值**
/// ⇒ 「跳过这次写入」在调用侧**无法**留下一条假记录（连编译都过不去，没有值可赋）。
/// 于是 [`GpuGeometryRenderer::bound_texture_size`] 名副其实 ——
/// 它报的就是描述符集里那张纹理，而不是别的字段的副产物。
pub(crate) fn point_descriptor_at(
    device: &VkDevice,
    set: &DescriptorSet,
    sampler: &crate::device::Sampler,
    texture: &Texture,
) -> GpuResult<(u32, u32)> {
    device.update_descriptor_texture(set, texture, sampler)?;
    Ok((texture.width(), texture.height()))
}

/// 一条绘制段属于哪条**来源**管线（顺序即 z 序）。
///
/// ## B5-2 之后的语义变化（**只当段表的标记，不再决定绑定**）
///
/// 从前它决定「这一段该绑哪条管线」，于是段数 = draw 次数 = 管线切换次数。
/// 统一之后两种段搬进**同一条**统一顶点流（[`vertex_unify::unify`] 按它选源数组、
/// 并给形状段打上判别符 `uv = (-1,-1)`）⇒ 录制时**只有一条管线、一次 draw**。
/// 这个枚举只在**翻译/搬运阶段**用来分派段的来源；段表本身仍是 z 序的唯一真值。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PipelineKind {
    Shape,
    Text,
}

/// 一帧里的一段绘制：**顺序即 z 序**。
///
/// `first`/`count` 是**各自（翻译层）顶点数组内**的区间（形状 44 字节 / 文本 32 字节
/// 两种顶点）—— [`vertex_unify::unify`] 按这张表把两路搬进统一顶点流，
/// 于是「z 序」只在这一处被解释，**不存在第二份顺序真值**。
///
/// ⚠️ **B5-3：段表只用于 [`vertex_unify::unify`] 的「选源 + z 序」** ——
/// 录制侧整帧只发**一次** `vkCmdDraw(0, 全部顶点数)`，段划分对它没有任何影响。
/// 于是 M3+ B2 的合段函数（`merge_adjacent_draw_calls`）**失去了它存在的唯一理由**
/// （它守的是「把相邻同管线且区间连续的段并成一次 draw」），已连同单测一起删掉；
/// 段的划分现在是**搬运阶段的输入**，不是绘制次数的决定因素。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DrawCall {
    pub(crate) kind: PipelineKind,
    pub(crate) first: u32,
    pub(crate) count: u32,
}

/// **渲染统计**（M3+ B1）：全部是**累计值**（自渲染器创建起），测试用两次读取的**差值**断言。
///
/// ## 为什么用这些量，而不是 fps（项目既有原则）
///
/// 本仓库早先吃过「不可复现的 fps 快照」的亏（被 reviewer 判为缺陷）：fps 依赖机器负载、
/// 电源状态、窗口是否被遮挡，**不能作为结论**。这四个量都是**可计数、可复现**的：
///
/// | 字段 | 含义（与真实 Vulkan 调用一一对应） |
/// |---|---|
/// | `draw_calls` | **绘制派发**次数（`vkCmdDraw` **或** `vkCmdDrawIndexedIndirect`，二者只会有一种在跑） |
/// | `pipeline_switches` | `vkCmdBindPipeline` 的调用次数（每次「切管线」一次） |
/// | `buffer_uploads` | 主机→**顶点缓冲**的上传次数（map + memcpy + unmap） |
/// | `buffer_allocations` | **缓冲的创建**次数（顶点 / 索引 / 间接，每个 `vkCreateBuffer` + 绑定内存一次） |
/// | `submits` | `vkQueueSubmit` 的调用次数（「一帧一提交」的可断言口径） |
/// | `indirect_draws` | `vkCmdDrawIndexedIndirect` 的调用次数（**证明走的是间接路径**） |
/// | `index_uploads` | 主机→**索引缓冲**的上传次数 |
/// | `indirect_uploads` | 主机→**间接命令缓冲**的上传次数 |
///
/// ## 计数位置（**必须与真实调用同处**）
///
/// 八个 `+= 1` 都写在**发那条 Vulkan 调用的同一个地方** —— 于是「删掉发射」必然也删掉计数，
/// 护栏不会退化成「实现者自证」。这是本项目反复验证过的唯一能挡住「把护栏一起删掉」的写法。
///
/// ## 为什么 `draw_calls` 之外还要一个 `indirect_draws`
///
/// `draw_calls` 是**派发次数**（两种发法都算），`indirect_draws` 只在**间接**那一支自增。
/// 两个都要：前者守住「一帧一次绘制」这条不变量（不因换实现而漂移），后者让
/// 「**真的走了 indirect**」变成可断言的事实 —— 把 `vkCmdDrawIndexedIndirect` 换回
/// `vkCmdDraw`（或整个删掉）时 `draw_calls` 可能仍对，但 `indirect_draws` 会掉到 0。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RenderStats {
    pub draw_calls: u64,
    pub pipeline_switches: u64,
    pub buffer_uploads: u64,
    pub buffer_allocations: u64,
    pub submits: u64,
    pub indirect_draws: u64,
    pub index_uploads: u64,
    pub indirect_uploads: u64,
}

/// 文本管线**特有的**资源（渲染器以 `Option` 持有：不调
/// [`GpuGeometryRenderer::with_text`] 就没有它，`Draw` 命令里的文本保持 M3a 的
/// `Unsupported` 行为）。
///
/// ## M3+ B5-2：**顶点缓冲与描述符集不再属于这里**
///
/// - **顶点缓冲**：统一之后只有**一块**（`GpuGeometryRenderer::vertex`）⇒ 搬走了；
/// - **描述符集/池**：统一片元着色器**无条件采样** ⇒ 形状帧也要绑一张纹理
///   ⇒ 描述符集必须是**渲染器级**的（`GpuGeometryRenderer::{descriptor_pool, descriptor_set,
///   dummy_texture}`），否则「没有 `TextEngine`」时无集可绑。本结构体只保留
///   **引擎 + 图集纹理 + 指纹**。
///
/// ## 为什么允许 `dead_code`
///
/// `pool` 等字段可能不被读取，但它们存在只为**所有权/析构顺序**。删掉会漏资源。
#[allow(dead_code)]
struct TextResources {
    engine: TextEngine,
    /// 当前已上传的图集纹理（`None` = 还没传过 ⇒ 此刻绑的是 1×1 哑纹理）。
    texture: Option<Texture>,
    /// 当前**已上传并已写进描述符集**的纹理指纹（见 [`texture_fingerprint`]）。
    ///
    /// `Some(DUMMY_TEXTURE_FINGERPRINT)` = 描述符集里现在是 1×1 哑纹理。
    /// 三者任一变化就重传 —— 依据：`GlyphAtlas` 只**追加/增高**、不淘汰，
    /// 内容只在「新字形入图集」时改变，而那只会让 `rasterized_glyphs()` 增加。
    uploaded: Option<(u32, u32, usize)>,
    /// 上一帧被跳过的文本命令数（诊断，见 [`GpuGeometryRenderer::text_skipped`]）。
    skipped: usize,
}

/// 把「当前生效的裁剪栈 + 这一条命令」组成一个临时 `DrawList`。
///
/// ## 为什么逐条命令，而不是整份列表一次
///
/// 形状与文本走**两条翻译层**（`gpu_geom` / `gpu_text`），它们的公开入口都是
/// 「整个 `DrawList`」⇒ 要按 `DrawList` 的原顺序把**每条**命令送给对应的那一层。
/// 所以我们把**生效的 `PushClip` 序列原样重放**：
/// 翻译层内部算的是 `full ∩ r1 ∩ r2 …`，与「完整列表」时**逐字相同**
/// （同一个初始全画布 + 同一顺序的求交），裁剪语义不会因为拆分而改变。
///
/// ⚠️ **B5-2 之后这条「逐条」不再是「为了分派管线」**（统一后只有一条管线），
/// 而是因为两层的入口 API 就是整份列表 —— 顺序仍由段表承载，**z 序不变**。
///
/// 代价是每条命令一次小分配。GUI 一帧的命令数在几十~几百量级，可忽略。
/// `pub(crate)`：窗口路径（`windowed.rs::draw_and_present`）用**同一份**实现 ——
/// 两条渲染路径的「逐命令翻译 + 裁剪栈重放」必须逐字相同，复制两份就是「只改了一边」的温床。
pub(crate) fn single_command_in_clip(active_clip: &[RectI], cmd: &DrawCmd) -> DrawList {
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

/// 一张**纹理 quad** 的 6 个统一顶点（`TL,TR,BR` + `TL,BR,BL`）—— `uv ≥ 0` ⇒ 走**采样支**。
///
/// ## `uv` 是「像素边界」语义（与 `gpu_text` 同源）
///
/// ```text
///   u(px) = (px - rect.x) / rect.w        （v 同理）
/// ```
///
/// 片元在**像素中心** `px + 0.5` 求值 ⇒ `u * tex_w = (px - rect.x + 0.5) * tex_w / rect.w`；
/// **1:1 时**（`rect.w == tex_w`）正好落在 texel 中心 ⇒ `NEAREST` 取到
/// `px - rect.x` 那个 texel，**无平局、逐像素精确** ⇒ 可与 CPU 参考逐字节对照。
///
/// **朝向**：纹理左上角对到 quad 左上角（画布 y 向下 + 着色器 `OriginUpperLeft`）
/// ⇒ **不做 V 翻转**；翻错会让上下颠倒（`textured_quad_uv_orientation_is_top_down` 抓这条）。
///
/// `rect`/`radius_kind` 对采样支**不被读取**（统一 FS 在 `uv.x ≥ 0` 时不看它们），
/// 这里照填 quad 的真实矩形以便诊断。
/// 离屏与**窗口**两条路径共用（T1.3 ② 起窗口侧也要铺纹理 quad）——
/// 顶点语义只有这一份来源，别在 `windowed.rs` 里另写一份。
pub(crate) fn textured_quad_vertices(
    extent: Extent,
    tex_w: u32,
    tex_h: u32,
    rect: RectI,
    tint: Color,
) -> Vec<UnifiedVertex> {
    if rect.w <= 0 || rect.h <= 0 || tex_w == 0 || tex_h == 0 {
        return Vec::new();
    }
    let w = extent.width.max(1) as f32;
    let h = extent.height.max(1) as f32;
    let ndc_x = |px: i32| 2.0 * px as f32 / w - 1.0;
    let ndc_y = |py: i32| 2.0 * py as f32 / h - 1.0;
    let u = |px: i32| (px - rect.x) as f32 / rect.w as f32;
    let v = |py: i32| (py - rect.y) as f32 / rect.h as f32;
    let color = [
        tint.r as f32 / 255.0,
        tint.g as f32 / 255.0,
        tint.b as f32 / 255.0,
        tint.a.clamp(0.0, 1.0),
    ];
    let rect_attr = [rect.x as f32, rect.y as f32, rect.w as f32, rect.h as f32];
    let corner = |px: i32, py: i32| UnifiedVertex {
        pos: [ndc_x(px), ndc_y(py)],
        rect: rect_attr,
        radius_kind: 0.0,
        color,
        uv: [u(px), v(py)],
    };
    let (x0, y0, x1, y1) = (rect.x, rect.y, rect.right(), rect.bottom());
    vec![
        corner(x0, y0),
        corner(x1, y0),
        corner(x1, y1),
        corner(x0, y0),
        corner(x1, y1),
        corner(x0, y1),
    ]
}

/// 把翻译层报出的「未支持」清单并进帧级错误状态（**两条管线共用**）。///
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

/// 「主机写索引缓冲 → GPU 索引取数」这条依赖的参数：
/// `HOST` / `HOST_WRITE` → `VERTEX_INPUT` / `INDEX_READ`。
///
/// 索引在**顶点装配**阶段被消费（`vkCmdBindIndexBuffer`），所以 `dst_stage` 与顶点缓冲
/// 相同（`VERTEX_INPUT`，`1 << 2`），只有 access 位换成 `INDEX_READ`（`1 << 1`）。
fn index_buffer_barrier_params() -> BarrierParams {
    BarrierParams {
        src_stage: VK_PIPELINE_STAGE_HOST_BIT,
        dst_stage: VK_PIPELINE_STAGE_VERTEX_INPUT_BIT,
        src_access: VK_ACCESS_HOST_WRITE_BIT,
        dst_access: VK_ACCESS_INDEX_READ_BIT,
    }
}

/// 「主机写间接命令缓冲 → GPU 读间接命令」这条依赖的参数：
/// `HOST` / `HOST_WRITE` → `DRAW_INDIRECT` / `INDIRECT_COMMAND_READ`。
///
/// ⚠️ 这里的 `dst_stage` 是 `DRAW_INDIRECT`（`1 << 1`）而**不是** `VERTEX_INPUT`：
/// 间接命令在**绘制之前**被读（比顶点取数更早）。写错不会报错、屏障也照样被接受，
/// 只是它不覆盖「读间接命令」这一步 —— 正是本项目反复吃过的那类「加了屏障但没用」。
fn indirect_buffer_barrier_params() -> BarrierParams {
    BarrierParams {
        src_stage: VK_PIPELINE_STAGE_HOST_BIT,
        dst_stage: VK_PIPELINE_STAGE_DRAW_INDIRECT_BIT,
        src_access: VK_ACCESS_HOST_WRITE_BIT,
        dst_access: VK_ACCESS_INDIRECT_COMMAND_READ_BIT,
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

/// 一块「主机可见 + 跨帧复用 + 按需扩容」的缓冲 + 它绑定的内存。
///
/// 名字来自历史（原来只有顶点缓冲），M3+ 第 4 项下半起它**同时承载索引缓冲与间接命令
/// 缓冲** —— 三者是同一种东西（主机写、GPU 读、内容不变就不重传），所以共用一个结构；
/// 行为未变，只是语义放宽了。
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
    /// **两条管线统一由共用层建出**（M3c-T1）。
    ///
    /// 从前这里是 `pipeline: Pipeline` + `TextResources` 里的 `pipeline`/`layout`/
    /// `vs`/`fs`/`set_layout` —— 即**管线状态被手写了两份**（形状一份、文本一份）。
    /// 现在状态集中在 [`crate::pipelines::shape_state`]（B5-3：旧的两条管线已删，
    /// 统一管线的状态取自那里），离屏与（M3c 的）窗口
    /// 两条路径共用同一份来源：复制 N 份字面量 = N 份「将来只改一份」的风险。
    ///
    /// 离屏用**静态** viewport（M2a 实测：动态在本机 Intel 上零像素），
    /// 窗口用**动态**（M2b 实证能上屏）—— 这个差异是 [`ViewportStrategy`] 参数，
    /// 不是两份手写状态。
    pipelines: crate::pipelines::PipelineResources,
    /// **统一管线**（B5-2）：整帧只用这一条（形状 + 文本合流后的 `UnifiedVertex`，stride 52）。
    ///
    /// 它的布局用 [`crate::pipelines::PipelineResources::text_layout`]（= 带 `set 0` 的那个）
    /// ⇒ 描述符集（图集或哑纹理）在这里也是**必绑**的。
    /// **字段顺序契约**：本管线引用 `unified_vs` / `unified_fs` 与 `pipelines` 里的布局
    /// ⇒ 必须声明在它们**之前**（Rust 按声明顺序析构 ⇒ 管线先销毁）。
    unified: crate::device::Pipeline,
    /// 统一管线的两个着色器模块（**只为所有权**：必须比 `unified` 活得久）。
    #[allow(dead_code)]
    unified_vs: crate::device::ShaderModule,
    #[allow(dead_code)]
    unified_fs: crate::device::ShaderModule,
    /// **纹理管线**（T1.3）：与 `unified` **同一支顶点着色器、同一套顶点布局**，
    /// 只换片元着色器（[`crate::spirv::fragment_shader_textured`] ⇒ RGBA 调制，
    /// 而不是统一 FS 的覆盖率语义）。只被
    /// [`GpuGeometryRenderer::draw_textured_quad`] 用 —— 那条路径会把 `set 0`
    /// 临时改指到用户纹理并单独提交，所以它与文本**从不在同一次 draw 里**。
    ///
    /// **字段顺序契约**同 `unified`：本管线引用 `textured_vs` / `textured_fs`
    /// ⇒ 必须声明在它们**之前**。
    textured: crate::device::Pipeline,
    #[allow(dead_code)]
    textured_vs: crate::device::ShaderModule,
    #[allow(dead_code)]
    textured_fs: crate::device::ShaderModule,
    image: VkObject,
    image_memory: VkObject,
    view: VkObject,
    framebuffer: VkObject,
    pool: VkObject,
    cmd: vk::CommandBufferHandle,
    /// **统一顶点缓冲**（B5-2：全帧只有这一块；**惰性创建**：一帧都没画过非空几何时不分配）。
    vertex: Option<VertexBuffer>,
    /// **索引缓冲**（M3+ 第 4 项下半）：`0..顶点数` 的 `u32` 序列，间接绘制必需。
    ///
    /// 惰性创建 + 跨帧复用；**只在顶点数变化时重传**（内容就是 `0..N`，N 不变则逐字节相同）。
    /// 声明在 `device` 之前 ⇒ 设备存活时销毁。
    index: Option<VertexBuffer>,
    /// **间接命令缓冲**（`VkDrawIndexedIndirectCommand`，20 字节），每帧读它来发绘制。
    ///
    /// 同样是惰性 + 复用：`index_count` 不变就不重传（那是唯一会变的字段）。
    indirect: Option<VertexBuffer>,
    /// 上一次写进索引缓冲的**顶点数 + 缓冲句柄**（句柄一变即作废，与 `uploaded_vertices` 同款）。
    uploaded_indices: Option<(vk::BufferHandle, u32)>,
    /// 上一次写进间接缓冲的**命令 + 缓冲句柄**。
    uploaded_indirect: Option<(vk::BufferHandle, DrawIndexedIndirectCommand)>,
    /// `set 0 / binding 0` 的组合图像采样器：**恒有效** ——
    /// 统一片元着色器无条件采样 ⇒ 形状帧也必须绑它（内容 = 图集或 [`DUMMY_COVERAGE`]）。
    ///
    /// ⚠️ **它必须声明在 `descriptor_pool` 之前**（= 先于池析构）：见池字段的说明。
    descriptor_set: DescriptorSet,
    /// 描述符池：**只为所有权**而持有 —— 但它的**声明位置**是硬契约：
    /// [`DescriptorSet`] 的 `Drop` 会调 `vkFreeDescriptorSets(device, pool, ..)`
    /// ⇒ **池必须比集活得久** ⇒ 集声明在池**之前**
    /// （Rust 按声明顺序析构，**先声明的先销毁** ⇒ 集先销、池后销）。
    ///
    /// ⚠️ **实测踩过（B5-2 实现期，两次）**：顺序写反时测试的**逻辑全通过**
    /// （像素逐字节相同、计数 1/1、`render` 无错），但**进程在退出时
    /// `STATUS_ACCESS_VIOLATION`（0xC0000005）**：`vkFreeDescriptorSets` 访问了
    /// 已销毁的池。校验层把它说得很清楚
    /// （`vkFreeDescriptorSets(): descriptorPool Invalid VkDescriptorPool Object`），
    /// **但在 `DEER_VK_VALIDATION` 关掉时这条消息不存在**，症状看起来就像
    /// 「测试框架自己崩了」—— 离原因极远。
    /// 更隐蔽的一点：`DescriptorPool` 的 `Drop` **不报错也不打日志**，
    /// 所以只有「先销毁的那个是集」才能对上。
    #[allow(dead_code)]
    descriptor_pool: DescriptorPool,
    /// **1×1 全覆盖率哑纹理**（见 [`DUMMY_COVERAGE`]）。
    ///
    /// 没有 `TextEngine`、或这一帧没有文本时绑的就是它。**不许省**：不绑任何纹理
    /// 会让无条件采样读到未定义内容（驱动不必报错）。
    #[allow(dead_code)]
    dummy_texture: Texture,
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
    /// 文本特有的资源（引擎 + 图集纹理 + 指纹）；`None` = 没调 `with_text`
    /// ⇒ `Text` 命令报 `Unsupported`（M3a 行为不变）。
    ///
    /// **必须声明在 `device` 之前**（所有 Vulkan 子对象都在 `device` 之前析构）。
    text: Option<TextResources>,
    /// 累计发出的 host→vertex 屏障条数（诊断 + 回归，见
    /// [`GpuGeometryRenderer::host_to_vertex_barrier_count`]）。
    ///
    /// **B5-2 起只有一块统一顶点缓冲** ⇒ 每帧最多一条 —— 从前那两个分项计数器
    /// （形状 / 文本各一）已经**没有意义**（它们存在的唯一理由是「两块缓冲各一条」），
    /// 所以连同它们的 getter 一起删掉，而不是留着恒等的两个数自欺。
    host_to_vertex_barriers: u64,
    /// 渲染统计（M3+ B1；累计值，见 [`RenderStats`]）。
    stats: RenderStats,
    /// **CPU 侧的顶点转换成本口径**（B5-2）：`unify` 被调用的次数。
    ///
    /// 与 [`GpuGeometryRenderer::unify_output_vertex_count`] 配对使用：
    /// 「一帧搬运了 N 个顶点、调了 1 次」是可报出的数字，而不是一句「很便宜」。
    unify_calls: u64,
    /// `unify` 累计输出的顶点数（= 段表声明的顶点数之和；**这是 CPU 转换的 O(N) 口径**）。
    unify_output_vertices: u64,
    /// **上一次实际上传的字节 + 当时那块（统一）缓冲的句柄**（M3+ B3）。
    ///
    /// ⚠️ **记句柄是关键（review I-1）**：跳过重传的条件是「**同一块缓冲** + 字节相同」。
    /// 缓冲一旦被重建（容量增长、释放后重建），句柄就变 ⇒ **自动作废**，
    /// 不需要任何调用方记得去清它。先前那版只比字节、靠调用方在 4 处手动清记录 ——
    /// reviewer 变异「让记录活过重建」后**所有断言仍全绿**（实际 75% 画面陈旧），
    /// 即那段清理是**承重但没有测试**的。现在把它变成**结构上不可能出错**。
    uploaded_vertices: Option<(vk::BufferHandle, Vec<u8>)>,
    /// **描述符集此刻指着哪张纹理**（宽, 高）—— B5-3 起这是
    /// [`GpuGeometryRenderer::bound_texture_size`] 的**唯一**来源。
    ///
    /// ## 为什么要单独一个字段（而不是从 `text.texture` 推）
    ///
    /// 从前 `bound_texture_size()` 读的是 `TextResources.texture` 这个**代理字段**，
    /// 于是变异「**跳过** `update_descriptor_texture`（忘了把 `set` 改指到图集）」
    /// 下它**照样返回图集尺寸** ⇒ 断言名不副实（复审 M12 实测：`gpu_text_reupload`
    /// 仍然绿，只有像素判据接住）。
    ///
    /// 现在读数与**真实副作用同处**：它由 [`point_descriptor_at`] 返回、且**只能**由
    /// 那个函数的返回值赋值（`?` 传播）—— 那个函数体内就是 `vkUpdateDescriptorSets`
    /// 的调用点。所以「跳过改指」既会漏掉写入、也必然漏掉这个记录
    /// （删掉调用连编译都过不去，返回值没有来源）。
    descriptor_points_at: (u32, u32),
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

        // ① 着色器不在这里建：本路径**唯一**的管线（统一管线）由
        //    [`build_unified_pipeline`] 建它的两个模块（B5-3 起旧的两条管线已删；
        //    从前是「这里建形状的、`with_text` 里建文本的」两处各一份）。

        // ② 渲染通道：清屏 + 离开通道即 `TRANSFER_SRC_OPTIMAL`（好直接回读）
        let pass = device.create_render_pass(
            COLOR_FORMAT,
            vk::VK_ATTACHMENT_LOAD_OP_CLEAR,
            vk::VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL,
        )?;

        // ③ 共用资源（B5-3）：**不建管线**，只建统一管线需要的三样东西
        //    （管线布局 / `set 0` 布局 / 采样器）。旧的两条管线已删 ——
        //    B5-2 之后它们没有任何绑定点，建了不用就是纯创建成本（复审 Minor F-4）。
        let pipelines = crate::pipelines::build_pipeline_resources(&device)?;

        // ③' **统一管线**（B5-2）：形状与文本合流后的一条管线（本路径**唯一**建的管线）。
        //     **声明顺序契约**：`unified` 先于 `unified_vs`/`unified_fs`（先销毁管线）；
        //     它引用的 `text_layout` 在 `pipelines` 里（同一条规矩）。
        //     颜色格式 = 本路径的附件格式；viewport 用**静态**
        //     （M2a 实测：动态在本机 Intel 驱动上零像素）。
        let (unified, unified_vs, unified_fs) = build_unified_pipeline(
            &device,
            &pass,
            COLOR_FORMAT,
            crate::pipelines::ViewportStrategy::Static {
                width: extent.width,
                height: extent.height,
            },
            &pipelines.text_layout,
        )?;

        // ③'a **纹理管线**（T1.3）：同一支 VS + 同一套顶点布局，只换 FS
        //     （`fragment_shader_textured`：四通道调制，不是覆盖率）。
        //     共用 `pipelines.text_layout` ⇒ 描述符布局**没有**被扩成两个 binding。
        let (textured, textured_vs, textured_fs) = build_unified_pipeline_with_fs(
            &device,
            &pass,
            COLOR_FORMAT,
            crate::pipelines::ViewportStrategy::Static {
                width: extent.width,
                height: extent.height,
            },
            &pipelines.text_layout,
            &crate::spirv::fragment_shader_textured(),
        )?;

        // ③'' `set 0`（描述符集）：统一片元着色器**无条件采样** ⇒ 必须恒有效。
        //      没有 `TextEngine` 时绑 [`DUMMY_COVERAGE`] 的 1×1 `R8_UNORM` 哑纹理。
        //
        //      ⚠️ **字段声明顺序契约**（见结构体字段的说明）：`descriptor_set` 必须声明在
        //      `descriptor_pool` **之前**。这里造的顺序（先池后集）无所谓 ——
        //      要紧的是**析构**顺序。
        let descriptor_pool = device.create_descriptor_pool(1)?;
        let descriptor_set =
            device.allocate_descriptor_set(&descriptor_pool, &pipelines.text_set_layout)?;
        let dummy_texture = device.create_texture_r8(1, 1, &DUMMY_COVERAGE)?;
        // 读数与副作用同处：`descriptor_points_at` **只能**来自这个返回值
        // ⇒ 「跳过改指」不可能留下一条旧读数为真的记录（见字段与函数的说明）。
        let descriptor_points_at =
            point_descriptor_at(&device, &descriptor_set, &pipelines.sampler, &dummy_texture)?;

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
            pipelines,
            unified,
            unified_vs,
            unified_fs,
            textured,
            textured_vs,
            textured_fs,
            image,
            image_memory,
            view,
            framebuffer,
            pool,
            cmd,
            vertex: None,
            // 字段声明顺序契约：`descriptor_set` 在前、`descriptor_pool` 在后（集先销）
            descriptor_set,
            descriptor_pool,
            dummy_texture,
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
            stats: RenderStats::default(),
            unify_calls: 0,
            unify_output_vertices: 0,
            uploaded_vertices: None,
            index: None,
            indirect: None,
            uploaded_indices: None,
            uploaded_indirect: None,
            // `new()` 刚把哑纹理写进描述符集 ⇒ 这就是它此刻指着的东西
            descriptor_points_at,
            device,
        })
    }

    /// **实际**渲染尺寸（请求 0 尺寸时是 1×1，见 [`GpuGeometryRenderer::new`]）。
    pub fn extent(&self) -> Extent {
        self.extent
    }

    /// 底层设备（诊断 / 在**同一个设备上**创建纹理用）。
    ///
    /// ## 为什么必须暴露它（而不是让调用方自己开一个设备）
    ///
    /// `VkImage`/`VkImageView` 是**设备级对象**：拿 A 设备的纹理去 B 设备的描述符集里采样
    /// 是非法用法（校验层会报，驱动可能只是采到垃圾）。而 [`Self::draw_textured_quad`]
    /// 需要一个与本渲染器**同设备**的纹理 ⇒ 调用方必须能拿到这个设备。
    pub fn device(&self) -> &VkDevice {
        &self.device
    }

    /// **让本渲染器支持文本**：接管一个 [`TextEngine`]（字体 + 字形图集 + 排版缓存）。
    ///
    /// 不调用它的渲染器保持 **M3a 行为不变**：`DrawCmd::Text` ⇒ `Unsupported`
    /// （不会静默丢弃）。调用之后，文本命令与形状命令一起被搬进**同一条**统一顶点流，
    /// 由**同一条**统一管线一次画完（B5-2）。
    ///
    /// ## 资源与生命周期
    ///
    /// - **顶点缓冲不再属于文本**：统一之后只有一块（[`Self::vertex`]）；
    /// - **描述符集也不属于文本**：统一片元着色器无条件采样 ⇒ 集在 [`Self::new`] 里就建好、
    ///   并**恒**绑着哑纹理；这里只是在图集指纹变化时把集**改指**到字形图集
    ///   （见 [`Self::refresh_atlas_texture`]）；
    /// - 图集纹理**不在这里上传**：改为在 [`Self::render`] 里按「图集指纹」惰性重传
    ///   （图集只在出现新字形时变化，见 [`TextResources::uploaded`]）。
    ///
    /// ## 二次接管（`with_text` 被调两次）
    ///
    /// 第二次调用会把 `TextResources` 整体换新（`uploaded: None`）⇒
    /// **图集必然重传**（指纹不可能匹配）⇒ 描述符集被改指到新引擎的图集。
    ///
    /// ## ⚠️ 顶点缓冲不再是「文本专属的」，这里改为**显式作废上传记录**
    ///
    /// B5-2 之前文本有**自己**的顶点缓冲，`with_text` 会让它变成 `None`
    /// ⇒ 下一帧必然重建 ⇒ 句柄变化 ⇒ B3 的上传记录**自动作废**（review I-1 的机制）。
    /// 统一之后缓冲是**共享**的（[`Self::vertex`]），接管新引擎不会动它 ——
    /// 于是「同一份顶点字节 + 同一个句柄」在第二次接管后**仍然成立** ⇒ B3 会**跳过**上传。
    ///
    /// 那本身是**像素正确的**（缓冲里就是那份内容），但它让「接管新引擎」这条路径
    /// 不再有任何**缓冲侧**的护栏。所以这里把上传记录**显式**清掉：
    /// 语义是「**接管新引擎 ⇒ 下一帧重新上传顶点**」，代价是每个引擎一次（不是每帧）。
    /// 好处与老机制相同：**不依赖任何调用方记得去清**，且 `tests/gpu_text_reupload.rs`
    /// 那条咬住断言（「换新 ⇒ 必须重新上传」）继续有效。
    pub fn with_text(mut self, engine: TextEngine) -> GpuResult<Self> {
        // **管线、布局、着色器、采样器、set 0 布局都已经在 `new()` 里建好了** ——
        // 现在连描述符**集**都在那里（因为统一管线无条件采样 ⇒ 形状帧也要绑）。
        // 这里只剩下**文本特有的资源**：引擎 + 图集纹理 + 指纹。
        //
        // 「二次接管 ⇒ 必须重传图集」是**结构性**的：新 `TextResources` 的 `uploaded`
        // 是 `None`（而不是 `Some(DUMMY_TEXTURE_FINGERPRINT)`），所以指纹必然不匹配。
        self.text = Some(TextResources {
            engine,
            texture: None,
            uploaded: None,
            skipped: 0,
        });
        // 顶点缓冲的上传记录显式作废（见上面的说明）：换引擎 ⇒ 下一帧重新上传
        self.uploaded_vertices = None;
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

    /// **渲染统计**（M3+ B1）：draw call / 管线切换 / 缓冲上传 / 缓冲分配（**累计值**）。
    ///
    /// 四个量都与真实 Vulkan 调用一一对应，且计数写在**发调用的同一处**（见 [`RenderStats`]）。
    /// 用法：读两次、取差值 ⇒ 「这一帧/这一段」的代价。
    pub fn render_stats(&self) -> RenderStats {
        self.stats
    }

    /// **统一顶点转换的 CPU 成本口径**（B5-2）：`unify` 被调用的次数（累计）。
    ///
    /// 与 [`Self::unify_output_vertex_count`] 一起构成「**可报出的数字**」：
    /// 「画了 N 帧 ⇒ 调了 N 次、搬运了 Σ 个顶点」。本项目的口径是
    /// **不许只说「可忽略」**（M3+ 计划里明写）—— 代价必须能被计数。
    ///
    /// 上限诚实说明：这只是**顶点数**这一维的 O(N) 口径（一次线性搬运 + 每顶点 52 字节的
    /// 写入），不含分配器的行为。它证明「转换发生了多少次、搬了多少顶点」，
    /// 不构成任何性能结论。
    pub fn unify_call_count(&self) -> u64 {
        self.unify_calls
    }

    /// `unify` 累计输出的**统一顶点数**（= 段表声明的顶点数之和）。
    ///
    /// 这是「一帧搬了多少顶点」的直接读数：`render` 里每帧读一次差值即得本帧的转换量
    /// （也是那段转换的内存写入量口径：`顶点数 × 52` 字节）。
    pub fn unify_output_vertex_count(&self) -> u64 {
        self.unify_output_vertices
    }

    /// **本帧绑定并采样的纹理尺寸**（宽, 高）—— 哑纹理 / 图集护栏的读数口。
    ///
    /// 为什么需要它：统一片元着色器**无条件采样**，所以「形状帧也必须绑一张纹理」
    /// 是一条**静默依赖**（不绑 = 未定义行为，驱动不报错）。有了这个读数，
    /// 测试就能断言「**只有形状、没有 `TextEngine`** 的渲染器，绑的是 **1×1** 哑纹理」
    /// 以及「接管了引擎的渲染器，绑的是**字形图集**」。
    ///
    /// ## 它读的是**描述符集真实指向**（B5-3 修正）
    ///
    /// 返回值来自 [`GpuGeometryRenderer::descriptor_points_at`] ——
    /// 那个字段**只能**由 [`point_descriptor_at`]（= 发 `vkUpdateDescriptorSets` 的
    /// 那个函数）的返回值赋值。从前它读的是 `TextResources.texture` 这个**代理字段**，
    /// 于是「忘了把 `set` 改指到图集」这类变异下它**照样**报图集尺寸
    /// ⇒ 断言名不副实（复审 M12 实测：`gpu_text_reupload` 当时仍然绿）。
    ///
    /// 语义边界**如实体现在返回值里**：它返回的是**当前描述符集里那张纹理**的尺寸
    /// （哑纹理 `(1, 1)` 或字形图集 `(w, h)`），不表示「这一帧采到了什么」。
    pub fn bound_texture_size(&self) -> (u32, u32) {
        self.descriptor_points_at
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

        // ③ **合流**（B5-2 的核心）：两路顶点 + 段表 ⇒ **一条**统一顶点流。
        //    段表**顺序即 z 序**，`unify` 只按它搬运（不排序、不合并）⇒ 像素语义不变。
        //    这里不需要任何合段：整帧只发一次 draw，段划分不影响录制
        //    （B5-3 已把那个失去调用点的合段函数连同单测一起删掉，见 `DrawCall` 的说明）。
        self.unify_calls += 1;
        let unified: Vec<UnifiedVertex> = vertex_unify::unify(&shape_verts, &text_verts, &calls);
        self.unify_output_vertices += unified.len() as u64;

        // ④⑤⑥⑦ 上传顶点/图集（按需）+ 录制 + 提交 + 回读
        let unified_pipe = self.unified.handle();
        self.record_and_submit(&unified, unified_pipe)?;
        self.read_back()
    }

    /// 把一张**通用纹理**铺到一个矩形上并回读整帧（M3+ 第 4 项下半）。
    ///
    /// ## 语义
    ///
    /// - 顶点走**统一顶点流的「采样支」**（`uv ≥ 0`）⇒ 片元着色器做
    ///   `out = vec4(tint.rgb, tint.a * texture(tex, uv).r)`（**R 通道当覆盖率**）；
    /// - `uv` 是**像素边界**语义：`u = (px - rect.x)/rect.w`（与 `gpu_text` 的推导同源），
    ///   于是 1:1 时每个像素中心正好采到对应 texel、NEAREST 无平局 ⇒ 可与 CPU 逐字节对照；
    /// - `tint.a` 与覆盖率相乘 ⇒ `cov ∈ {0,1}` + 不透明 tint 时是**逐字节**判据，
    ///   中间覆盖率则是「≤1 LSB」（与项目既有的不透明/半透明判据口径一致）；
    /// - **纹理必须与渲染器同一个设备**（`VkImage` 是设备级对象）⇒ 用
    ///   [`Self::device`] 上的 `create_texture_rgba8`/`create_texture_r8` 造它；
    /// - 画完把描述符集**改回**「默认纹理」（图集或 1×1 哑纹理），否则后续
    ///   形状/文本帧会绑着一张无关的纹理（虽然形状支不采样，但那是隐性状态，不该留）。
    ///
    /// ## 与 `render(&DrawList)` 的关系
    ///
    /// 两者共用同一条录制/提交/回读路径（[`Self::record_and_submit`]），所以
    /// 间接绘制、屏障、计数、稳态复用这些性质对纹理 quad 同样成立。
    /// 纹理 quad **不是** `DrawCmd`：`DrawList` 属于 `deer-gpu` 的契约（不在本任务 scope），
    /// 而「一张任意纹理铺到矩形上」目前只有测试/诊断需要。
    pub fn draw_textured_quad(
        &mut self,
        texture: &Texture,
        rect: RectI,
        tint: Color,
    ) -> GpuResult<Vec<u8>> {
        self.unsupported.clear();
        // 上一次提交没确认完成 ⇒ 什么都不许碰（与 `render` 同一守卫）
        self.sync.ensure_reusable()?;

        let unified = textured_quad_vertices(self.extent, texture.width(), texture.height(), rect, tint);
        // 改指描述符集（读数与副作用同处：返回值只能来自 `point_descriptor_at`）
        self.descriptor_points_at = point_descriptor_at(
            &self.device,
            &self.descriptor_set,
            &self.pipelines.sampler,
            texture,
        )?;
        let textured_pipe = self.textured.handle();
        let recorded = self.record_and_submit(&unified, textured_pipe);
        // 无论成败都把描述符集改回默认纹理：失败路径也不该留下「指着别人纹理」的状态。
        let restored = self.rebind_default_texture();
        recorded?;
        restored?;
        self.read_back()
    }

    /// 把渲染器级描述符集改回**默认纹理**（有文本引擎时是图集，否则是 1×1 哑纹理）。
    fn rebind_default_texture(&mut self) -> GpuResult<()> {
        let target: &Texture = match self.text.as_ref().and_then(|t| t.texture.as_ref()) {
            Some(t) => t,
            None => &self.dummy_texture,
        };
        self.descriptor_points_at = point_descriptor_at(
            &self.device,
            &self.descriptor_set,
            &self.pipelines.sampler,
            target,
        )?;
        Ok(())
    }

    /// 确保某块**主机可见缓冲**至少有 `bytes` 字节（不够就按 2 的幂重建），**先建后换**。
    ///
    /// 重建 ⇒ 缓冲句柄变化 ⇒ 复用记录**自动作废**（记录里带着句柄，见 `uploaded_vertices`）。
    ///
    /// `usage` 由调用方给（顶点 / 索引 / 间接各一种）—— 从 M3+ 第 4 项下半起这块结构
    /// 同时承载三种缓冲，所以用法位不能再写死成 `VERTEX_BUFFER`。
    fn ensure_vertex_capacity(
        &mut self,
        slot: &mut Option<VertexBuffer>,
        bytes: u64,
        usage: u32,
        what: &'static str,
    ) -> GpuResult<()> {
        // 破坏性操作（会销毁旧缓冲、分配新内存）⇒ 守卫放在这里（见 R1-3）。
        self.sync.ensure_reusable()?;
        if slot.as_ref().is_some_and(|v| v.capacity >= bytes) {
            return Ok(());
        }
        let capacity = bytes.next_power_of_two().max(MIN_VERTEX_BYTES);
        // **先建新的、成功后再换**（T3 review F11）：失败时旧缓冲仍然可用、容量信息不丢。
        // 析构顺序仍然正确：`VertexBuffer` 的字段顺序保证「先缓冲、后内存」；
        // 赋值时旧值被 drop，此刻上一帧的提交已经等过栅栏 ⇒ 缓冲不在使用中。
        let (buffer, memory) = create_host_buffer(what, self.device_handle, &self.fns, &self.mem_props, capacity, usage)?;
        *slot = Some(VertexBuffer {
            buffer,
            memory,
            capacity,
        });
        // 计数与真实调用同处（`vkCreateBuffer` + 绑定内存在上一行刚发生）
        self.stats.buffer_allocations += 1;
        Ok(())
    }

    /// 把一段字节写进一块**主机可见缓冲**（map → memcpy → unmap）。
    ///
    /// ## 为什么**不在这里**自增计数
    ///
    /// 从 M3+ 第 4 项下半起这块结构同时承载三种缓冲（顶点 / 索引 / 间接），而它们各有
    /// **三个不同的计数器**（`buffer_uploads` / `index_uploads` / `indirect_uploads`）。
    /// 在共用函数里自增就会把三种上传混成一个数（实测踩过：`buffer_uploads` 变成 3）。
    /// 所以计数器由调用方在**紧邻这一步的同一处**自增 —— 删掉这次写入仍会同时删掉那一行计数。
    ///
    /// `what` 只用于报错里指认是哪块缓冲。
    /// 收 `memory` 句柄（而不是 `&VertexBuffer`）：调用点通常正持有 `self.vertex`
    /// 的借用，传引用会和 `&mut self`（要自增 `stats`）撞借用检查 —— 句柄是 `Copy` 的普通值。
    fn upload_buffer_bytes(
        &mut self,
        memory: vk::DeviceMemoryHandle,
        src: &[u8],
        what: &str,
    ) -> GpuResult<()> {
        // map 的守卫：**任何**提前返回都会 unmap（T3 review F9）。
        let mapped = map_memory(what, &self.fns, self.device_handle, memory)?;
        // SAFETY: 映射了整块缓冲（≥ src.len()，由 `ensure_vertex_capacity` 保证）；源与目标不重叠。
        unsafe {
            std::ptr::copy_nonoverlapping(src.as_ptr(), mapped.as_mut_ptr() as *mut u8, src.len());
        }
        Ok(())
    }

    /// 图集**变化时**才重传纹理，并把新纹理写进描述符集。
    ///
    /// ## 指纹（见 [`texture_fingerprint`]）
    ///
    /// `(图集宽, 图集高, 已光栅化字形数)` —— 与 [`DUMMY_TEXTURE_FINGERPRINT`] 区分：
    /// 后者标记「描述符集里现在是 1×1 哑纹理」（`GlyphAtlas` 的宽 ≥ 2 ⇒ 不会撞上）。
    ///
    /// ## ⚠️ 描述符集是**渲染器级**的（B5-2）
    ///
    /// 从前这个集属于「文本管线资源」；统一片元着色器无条件采样之后它必须**恒**有效
    /// ⇒ 集在 [`GpuGeometryRenderer::new`] 里就建好并绑着哑纹理，这里只是**改指**到图集。
    /// 描述符集的写入必须发生在「集没有被在飞的提交使用」时：本函数在
    /// `record_and_submit` 的录制**之前**调用，而上一帧的提交已经等到栅栏
    /// （`sync.ensure_reusable()` 保证），所以是安全的。
    ///
    /// 上传走 `create_texture_r8`（一次性路径，内部 `vkQueueWaitIdle`）——
    /// **只在图集变化时发生**（新字形首次出现），所以那次等空闲被摊薄；
    /// 若将来改成每帧重传，就必须改异步上传 + 栅栏（`device.rs` 的文档里写着这条前提）。
    fn refresh_atlas_texture(&mut self) -> GpuResult<()> {
        let (key, data) = {
            let res = self.text.as_ref().expect("调用方保证了文本资源存在");
            let key = texture_fingerprint(&res.engine);
            if res.uploaded == Some(key) {
                return Ok(());
            }
            (key, res.engine.atlas().coverage().to_vec())
        };
        let (w, h, glyphs) = key;
        let texture = self.device.create_texture_r8(w, h, &data)?;
        // 改指描述符集：**同一个集**（渲染器级的那个），不是文本自己的。
        // 读数与副作用同处：记录**只能**来自这个返回值（见 `point_descriptor_at`）——
        // 跳过这次改指，`bound_texture_size()` 就不可能报出图集尺寸。
        self.descriptor_points_at = point_descriptor_at(
            &self.device,
            &self.descriptor_set,
            &self.pipelines.sampler,
            &texture,
        )?;
        {
            let res = self.text.as_mut().expect("同上");
            res.texture = Some(texture);
            res.uploaded = Some((w, h, glyphs));
        }
        Ok(())
    }

    /// 为**统一顶点缓冲**发一条「主机写 → 顶点取数」屏障（参数见 [`vertex_buffer_barrier_params`]）。
    ///
    /// 收**句柄**而不是 `&VertexBuffer`：调用点通常正持有 `self.vertex` 的借用，
    /// 传引用会和 `&mut self`（要自增计数器）撞借用检查 —— 句柄是 `Copy` 的普通值。
    ///
    /// 计数与 Vulkan 调用**写在同一处** ⇒ 删掉发射就必然删掉计数，护栏挡得住「整体删掉」这类变异。
    ///
    /// ⚠️ **B5-2 起只有一个计数器**：全帧只有一块顶点缓冲 ⇒ 每帧最多一条屏障，
    /// 从前那两个分项计数器（形状/文本）**没有意义**了，已随字段一起删除
    /// （留两个恒等的数只会让人以为它们还分辨着什么）。
    fn emit_host_to_vertex_barrier(&mut self, buffer: vk::BufferHandle) {
        self.emit_host_buffer_barrier(buffer, vertex_buffer_barrier_params());
        self.host_to_vertex_barriers += 1;
    }

    /// 为**任意一块主机写的缓冲**发一条「主机写 → GPU 读」屏障（参数由纯函数给）。
    ///
    /// 抽出来的理由：索引缓冲与间接命令缓冲各需要一条，但目标阶段/访问位不同
    /// （`VERTEX_INPUT`/`INDEX_READ` 与 `DRAW_INDIRECT`/`INDIRECT_COMMAND_READ`）——
    /// 三份「字面量屏障」正是本项目反复出错的形态（写错不报错、屏障照样被接受）。
    /// 参数集中到 [`BarrierParams`] 纯函数里，就能用单元测试钉确切掩码。
    fn emit_host_buffer_barrier(&mut self, buffer: vk::BufferHandle, p: BarrierParams) {
        let barrier = vk::BufferMemoryBarrier {
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
                &barrier,
                0,
                std::ptr::null(),
            );
        }
    }

    /// 录制一帧（清屏 + 按 z 序逐段绑定管线/顶点缓冲 + 绘制 + 屏障 + 拷贝），提交并等栅栏。
    ///
    /// 需要 `&mut self`：等待失败时要把 [`SubmitState`] 置为 `Broken`（见模块文档「同步②」）。
    /// `pipeline` 是要绑的那条管线句柄（**先取出来再进本函数**，避免与 `&mut self` 撞借用）。
    ///
    /// 为什么要它作参数：T1.3 起有**两条**共用顶点布局的管线（统一 / 纹理），
    /// 「这一趟用哪条」是**调用点**的事，不该是录制函数里的常量。
    fn record_and_submit(
        &mut self,
        unified: &[UnifiedVertex],
        pipeline: vk::PipelineHandle,
    ) -> GpuResult<()> {
        // ★ **守卫就放在破坏性操作本身**（fix round 2 / R1-3）：`render` 开头那句检查可能
        //   因为调用方漏写 `?` 而失效（reviewer 的变异 C 就是这么全绿的）。这里再查一次，
        //   于是「Broken ⇒ 绝不去碰命令缓冲/栅栏」不依赖任何调用方的写法。
        self.sync.ensure_reusable()?;

        // ① 上传：**一块**统一顶点缓冲，「按需扩容 + **内容变化才重传**」
        //
        // **B3（跨帧复用）**：连续帧语料不变时顶点数据逐字节相同 ⇒ 跳过 map/memcpy/unmap
        // （`buffer_uploads` 不增长）。**作废是结构性的**：记录里带着缓冲句柄，
        // 缓冲一旦换新句柄就不匹配 ⇒ 必然重传（review I-1）。
        //
        // ⚠️ **B5-2 的语义收紧（如实登记）**：从前是「形状 / 文本两块缓冲各有一条
        // host→vertex 屏障」，交错帧发 **2** 条；现在只有一块缓冲 ⇒ **整帧最多 1 条**。
        // 于是 `gpu_vs_cpu.rs` 里 `text_barriers_are_emitted_per_buffer` 那条
        // （要求交错帧 +2）**不可能再成立** —— 它守的是「两块缓冲各有护栏」，
        // 而那两块缓冲已经不存在了。该用例改成「统一缓冲的屏障」语义（见那里的说明）。
        let uploaded_now = if unified.is_empty() {
            false
        } else {
            let bytes = std::mem::size_of_val(unified) as u64;
            let mut slot = self.vertex.take();
            self.ensure_vertex_capacity(
                &mut slot,
                bytes,
                vk::VK_BUFFER_USAGE_VERTEX_BUFFER_BIT,
                "vkCreateBuffer(vertex)",
            )?;
            self.vertex = slot;
            let bytes = std::mem::size_of_val(unified);
            // SAFETY: `UnifiedVertex` 是 `#[repr(C)]` 纯 `f32`（无指针、无 Drop）⇒ 字节视图合法。
            let src = unsafe { std::slice::from_raw_parts(unified.as_ptr() as *const u8, bytes) };
            let handle = self
                .vertex
                .as_ref()
                .expect("ensure 之后必有缓冲")
                .buffer
                .handle();
            // 跳过条件：**同一块缓冲**（句柄相等）且字节相同
            let same =
                matches!(&self.uploaded_vertices, Some((h, b)) if *h == handle && b.as_slice() == src);
            if !same {
                let mem = self
                    .vertex
                    .as_ref()
                    .expect("ensure 之后必有缓冲")
                    .memory
                    .handle();
                self.upload_buffer_bytes(mem, src, "vkMapMemory(unified vertex)")?;
                // 计数与真实调用同处（memcpy 刚发生、unmap 随 `mapped` 析构）
                self.stats.buffer_uploads += 1;
                self.uploaded_vertices = Some((handle, src.to_vec()));
                true
            } else {
                false
            }
        };
        // 图集若变了就重传纹理 + **改指**渲染器级的描述符集（必须在提交之前）。
        // 没有文本引擎时不动：描述符集里是 `new()` 绑好的 1×1 哑纹理。
        if self.text.is_some() {
            self.refresh_atlas_texture()?;
        }

        // ①b **索引缓冲 + 间接命令缓冲**（M3+ 第 4 项下半）：间接绘制必需的输入。
        //
        // 内容只在**顶点数变化**时才变（索引是 `0..N`、命令里只有 `indexCount` 随 N 变）
        // ⇒ 与顶点缓冲同一套「句柄 + 内容相同就跳过」的复用判据；稳态每帧零上传、零分配。
        // 顺序：先建/扩容（可能重建 ⇒ 句柄变化 ⇒ 记录自动作废），再按需重传，最后发屏障。
        let mut indirect_draw = None;
        let mut index_uploaded_now = false;
        let mut indirect_uploaded_now = false;
        if !unified.is_empty() {
            let vertex_count = unified.len() as u32;
            let index_bytes = vertex_count as u64 * 4; // u32 索引
            let mut index_slot = self.index.take();
            self.ensure_vertex_capacity(
                &mut index_slot,
                index_bytes,
                VK_BUFFER_USAGE_INDEX_BUFFER_BIT,
                "vkCreateBuffer(index)",
            )?;
            self.index = index_slot;
            let index_handle = self
                .index
                .as_ref()
                .expect("ensure 之后必有缓冲")
                .buffer
                .handle();
            let query = DrawIndexedIndirectCommand::for_vertex_count(vertex_count);
            let index_changed = !matches!(
                &self.uploaded_indices,
                Some((h, n)) if *h == index_handle && *n == vertex_count
            );
            if index_changed {
                let mem = self
                    .index
                    .as_ref()
                    .expect("ensure 之后必有缓冲")
                    .memory
                    .handle();
                self.upload_buffer_bytes(
                    mem,
                    &DrawIndexedIndirectCommand::sequential_indices(vertex_count),
                    "vkMapMemory(index)",
                )?;
                // 计数与真实调用同处（索引缓冲的写入刚发生）
                self.stats.index_uploads += 1;
                self.uploaded_indices = Some((index_handle, vertex_count));
                index_uploaded_now = true;
            }

            let mut indirect_slot = self.indirect.take();
            self.ensure_vertex_capacity(
                &mut indirect_slot,
                std::mem::size_of::<DrawIndexedIndirectCommand>() as u64,
                VK_BUFFER_USAGE_INDIRECT_BUFFER_BIT,
                "vkCreateBuffer(indirect)",
            )?;
            self.indirect = indirect_slot;
            let indirect_handle = self
                .indirect
                .as_ref()
                .expect("ensure 之后必有缓冲")
                .buffer
                .handle();
            let command_changed = !matches!(
                &self.uploaded_indirect,
                Some((h, c)) if *h == indirect_handle && *c == query
            );
            if command_changed {
                let mem = self
                    .indirect
                    .as_ref()
                    .expect("ensure 之后必有缓冲")
                    .memory
                    .handle();
                self.upload_buffer_bytes(mem, &query.to_bytes(), "vkMapMemory(indirect)")?;
                // 计数与真实调用同处（间接命令的写入刚发生）
                self.stats.indirect_uploads += 1;
                self.uploaded_indirect = Some((indirect_handle, query));
                indirect_uploaded_now = true;
            }
            indirect_draw = Some(indirect_handle);
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
        //   **B5-2 起只有一块缓冲 ⇒ 这条最多发一次。**
        //   参数来自纯函数（可被单元测试钉常量）；「屏障是否真的发出」由
        //   `host_to_vertex_barrier_count()` 断言（删掉发射 ⇒ 计数不增长 ⇒ 用例红）。
        //   （放在渲染通道**之前**：缓冲区屏障在通道内也合法，但放在外面更简单、更不容易踩
        //     「通道内允许哪些屏障」的规则。）
        //
        //   B3 起：**只有这一帧真的上传了**才发（没上传 = 没有新的主机写入，
        //   上一次那条屏障已经给同一块缓冲建立过依赖）。
        if uploaded_now {
            let h = self
                .vertex
                .as_ref()
                .expect("统一顶点已上传")
                .buffer
                .handle();
            self.emit_host_to_vertex_barrier(h);
        }
        //   ★ 同样地，索引/间接缓冲**这一帧真的重传了**才发各自的屏障
        //   （没重传 = 没有新的主机写入；上一次那条屏障已经给同一块缓冲建立过依赖）。
        if index_uploaded_now {
            let h = self.index.as_ref().expect("刚上传过 ⇒ 缓冲在").buffer.handle();
            self.emit_host_buffer_barrier(h, index_buffer_barrier_params());
        }
        if indirect_uploaded_now {
            let h = self
                .indirect
                .as_ref()
                .expect("刚上传过 ⇒ 缓冲在")
                .buffer
                .handle();
            self.emit_host_buffer_barrier(h, indirect_buffer_barrier_params());
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
            // ★ **B5-2：整帧一次 bind + 一次 draw**。
            //   从前这里按段（z 序）在两条管线间来回切：段数 = draw 次数 = 切换次数
            //   （实测语料 8/8）。现在两者已经是**同一条顶点流 + 同一条管线**
            //   ⇒ 绑定一次、`vkCmdDraw(0, 全部顶点数)` 一次。
            //
            //   于是 `pipeline_switches` / `draw_calls` 对任何非空帧都恒为 **1**
            //   （`pipeline_switches` 计的是真实的 `vkCmdBindPipeline` 调用，
            //    与 `draw_calls` 的计数一样写在**发调用的同一处** ⇒ 删掉发射必然删掉计数）。
            if !unified.is_empty() {
                (self.fns.cmd_bind_pipeline)(
                    self.cmd,
                    vk::VK_PIPELINE_BIND_POINT_GRAPHICS,
                    pipeline,
                );
                // 计数与真实调用同处（B1）：删掉这行绑定就必然删掉计数
                self.stats.pipeline_switches += 1;
                let vb = self.vertex.as_ref().expect("有统一顶点 ⇒ 缓冲已上传");
                let offset: vk::DeviceSize = 0;
                (self.fns.cmd_bind_vertex_buffers)(self.cmd, 0, 1, &vb.buffer.handle(), &offset);
                // 索引缓冲：`u32` 索引、偏移 0（`VK_INDEX_TYPE_UINT32` = 1）。
                let ib = self.index.as_ref().expect("有统一顶点 ⇒ 索引缓冲已就绪");
                (self.fns.cmd_bind_index_buffer)(
                    self.cmd,
                    ib.buffer.handle(),
                    0,
                    VK_INDEX_TYPE_UINT32,
                );
                // `set 0 / binding 0`：**恒有**（图集或 1×1 哑纹理，见 `dummy_texture`）。
                // 统一片元着色器无条件采样 ⇒ 少了这条绑定就是未定义行为（驱动不必报错）。
                (self.fns.cmd_bind_descriptor_sets)(
                    self.cmd,
                    vk::VK_PIPELINE_BIND_POINT_GRAPHICS,
                    self.pipelines.text_layout.handle(),
                    0,
                    1,
                    &self.descriptor_set.handle(),
                    0,
                    std::ptr::null(),
                );
                // ★ **间接绘制**（M3+ 第 4 项下半）：命令来自那块间接缓冲，而不是主机给的数字。
                //   `drawCount = 1`、`stride = size_of::<VkDrawIndexedIndirectCommand>()`。
                //   两个计数都写在**这一行调用旁边** ⇒ 换回 `vkCmdDraw`（或删掉）时
                //   `indirect_draws` 必然掉到 0（`draw_calls` 是派发次数，两种发法都算）。
                let indirect_handle = indirect_draw.expect("有统一顶点 ⇒ 间接缓冲已就绪");
                (self.fns.cmd_draw_indexed_indirect)(
                    self.cmd,
                    indirect_handle,
                    0,
                    1,
                    std::mem::size_of::<DrawIndexedIndirectCommand>() as u32,
                );
                // 计数与真实调用同处（B1）
                self.stats.draw_calls += 1;
                self.stats.indirect_draws += 1;
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
        // 计数与真实调用同处：删掉这行提交就必然删掉计数（「一帧一提交」的可断言口径）。
        self.stats.submits += 1;
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

    /// **B1 的统计字段语义**：默认全 0（新渲染器 = 没画过任何东西）。
    #[test]
    fn render_stats_starts_at_zero() {
        let s = RenderStats::default();
        assert_eq!(s.draw_calls, 0);
        assert_eq!(s.pipeline_switches, 0);
        assert_eq!(s.buffer_uploads, 0);
        assert_eq!(s.buffer_allocations, 0);
        assert_eq!(s.submits, 0);
        assert_eq!(s.indirect_draws, 0);
        assert_eq!(s.index_uploads, 0);
        assert_eq!(s.indirect_uploads, 0);
    }

    /// **间接绘制的两条屏障**必须确切是：
    /// - 索引：`HOST`/`HOST_WRITE` → `VERTEX_INPUT(0x4)`/`INDEX_READ(0x2)`
    /// - 间接：`HOST`/`HOST_WRITE` → `DRAW_INDIRECT(0x2)`/`INDIRECT_COMMAND_READ(0x1)`
    ///
    /// 变异验证（**已实测**，见任务报告）：
    /// ① 把间接那条的 `dst_stage` 改成 `VERTEX_INPUT`（一个非常自然的「和顶点一样就行」笔误）
    ///    ⇒ 本测试**变红**；② 把 `INDEX_READ` 写成 `INDIRECT_COMMAND_READ` ⇒ 也变红。
    /// 这两种写法**不会**被驱动或校验层拒绝（屏障照样被接受），只会让它建的依赖不覆盖
    /// 真正读那块缓冲的阶段 —— 属于本项目最怕的「加了屏障但没用」。
    #[test]
    fn index_and_indirect_barrier_params_pin_the_exact_masks() {
        let idx = index_buffer_barrier_params();
        assert_eq!(idx.src_stage, 1 << 14, "srcStageMask 必须是 HOST");
        assert_eq!(idx.dst_stage, 0x4, "索引在顶点装配阶段被读 ⇒ VERTEX_INPUT(0x4)");
        assert_eq!(idx.src_access, 1 << 14, "srcAccessMask 必须是 HOST_WRITE");
        assert_eq!(idx.dst_access, 0x2, "dstAccessMask 必须是 INDEX_READ(0x2)");

        let ind = indirect_buffer_barrier_params();
        assert_eq!(ind.src_stage, 1 << 14, "srcStageMask 必须是 HOST");
        assert_eq!(
            ind.dst_stage, 0x2,
            "间接命令在 DRAW_INDIRECT(0x2) 阶段被读 —— **不是** VERTEX_INPUT(0x4)"
        );
        assert_eq!(ind.src_access, 1 << 14, "srcAccessMask 必须是 HOST_WRITE");
        assert_eq!(
            ind.dst_access, 0x1,
            "dstAccessMask 必须是 INDIRECT_COMMAND_READ(0x1) —— 不是 INDEX_READ(0x2)"
        );

        // 显式钉住「这两个位数值相同但枚举不同」这一条容易看错的事实
        assert_eq!(
            VK_PIPELINE_STAGE_DRAW_INDIRECT_BIT, VK_ACCESS_INDEX_READ_BIT,
            "阶段位 / 访问位是两套枚举：DRAW_INDIRECT(0x2) 与 INDEX_READ(0x2) 数值相同不是笔误"
        );
        assert_ne!(ind.dst_stage, idx.dst_stage, "两者的目标阶段必须不同");
        assert_ne!(ind.dst_access, idx.dst_access, "两者的目标访问必须不同");
    }

    /// **纹理 quad 的 uv/朝向是纯逻辑**（不需要 GPU）：上下方向与左右方向都要对。
    ///
    /// 变异验证（**已实测**）：把 `v` 改成 `1.0 - v`（经典 V 翻转）⇒ 本测试**变红**；
    /// 把 `u` 改成 `1.0 - u` ⇒ 也变红。
    #[test]
    fn textured_quad_uv_is_top_down_and_left_to_right() {
        let extent = Extent { width: 8, height: 8 };
        let quad = RectI::new(2, 2, 4, 4);
        let vs = textured_quad_vertices(extent, 4, 4, quad, Color::rgb(1, 2, 3));
        assert_eq!(vs.len(), 6, "两个三角形 = 6 个顶点");

        // 4×4 纹理铺到 4×4 quad ⇒ 像素 (2,2)（quad 左上）必须采到 uv≈(0,0)
        let tl = vs[0];
        assert_eq!(tl.pos, [2.0 * 2.0 / 8.0 - 1.0, 2.0 * 2.0 / 8.0 - 1.0], "左上角 NDC");
        assert_eq!(tl.uv, [0.0, 0.0], "quad 左上角 ⇒ 纹理左上角（**不做 V 翻转**）");
        // 右上角（x1, y0）⇒ uv = (1, 0)
        assert_eq!(vs[1].uv, [1.0, 0.0]);
        // 右下角（x1, y1）⇒ uv = (1, 1)
        assert_eq!(vs[2].uv, [1.0, 1.0]);
        // 左下角（x0, y1）⇒ uv = (0, 1)
        assert_eq!(vs[5].uv, [0.0, 1.0]);

        // 采样支的判别符：`uv.x >= 0` 才走采样（形状段用 SHAPE_UV_SENTINEL = (-1,-1)）
        assert!(
            vs.iter().all(|v| !v.is_shape()),
            "纹理 quad 必须全部落在**采样支**（否则片元着色器不采样）"
        );

        // 退化输入：空矩形 / 0 尺寸纹理 ⇒ 不产出顶点（与 `emit_quad` 的「裁剪后为空」一致）
        assert!(textured_quad_vertices(extent, 4, 4, RectI::new(0, 0, 0, 4), Color::WHITE).is_empty());
        assert!(textured_quad_vertices(extent, 0, 4, quad, Color::WHITE).is_empty());
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
