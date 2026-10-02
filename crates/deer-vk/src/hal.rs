//! **把 Vulkan 接进 HAL**（M2b）。
//!
//! [`deer_gpu`] 的 HAL 契约是「一个设备能建交换链、能提交帧」（`Device` / `Swapchain` / `Frame`）。
//! 本模块把 M2b 真正跑通的那条链（[`WindowedRenderer`]：实例 + 设备 + surface + 交换链 +
//! 渲染通道 + 管线 + 三帧同步）包成 HAL 的形状，于是：
//!
//! - `deer-gui` / 上层只需要认 HAL，不需要认 Vulkan 细节；
//! - 「加一个后端 = 实现一个 trait」这条承诺在**窗口化**路径上也成立（DX12/Metal 以后照此实现）。
//!
//! ## 三条刻意的设计（都不是随手写的）
//!
//! 1. **`open()` 只有适配器信息，链在 `create_swapchain` 时才建** —— HAL 的 `Device`
//!    在打开时还不知道窗口，所以「实例/设备」的创建被推迟到拿得到窗口句柄那一刻。
//!    （M2b 复用 [`WindowedRenderer`] 自己的实例/设备；把「HAL 设备」与「窗口链的设备」
//!    合并成一条是 M3 的工程活，见下面的「诚实边界」。）
//! 2. **`Frame::record` 对非空绘制列表明确报 `Unsupported`** —— `DrawList → GPU`
//!    （矩形/圆角/文本）是 M3。**静默忽略**命令会让「窗口里什么都没有」变成一个查不出的 bug，
//!    所以这里宁可吵一声。
//! 3. **`submit_and_present` 把 `FrameOutcome::OutOfDate` 如实上报**（映射成
//!    [`PresentResult::OutOfDate`]），由调用方 resize 后重试 —— 交换链过期是正常路径，
//!    不许当成功。
//!
//! ## 诚实边界（M2b）
//!
//! - 纹理：`create_texture` / `upload_texture` 已落地（T1.2，见下）；**窗口链**上的
//!   纹理绑定（把 HAL 纹理喂进 `WindowedRenderer` 的绘制路径）仍是 M3 的工程活。
//! - `read_pixels` 明确报 `Unsupported`：HAL 的这个方法在**提交前**调用，而交换链图像的回读
//!   数据只有**呈现之后**才有效 ⇒ 请用 [`crate::WindowedRenderer::read_back_last_frame()`]
//!   （`render_and_present()` 之后调；交换链图像确实申请了 `TRANSFER_SRC`）。
//!   这里刻意**不做「隐式呈现」**那种惊吓式语义；离屏回读取 [`crate::OffscreenRenderer`]。
//! - 只支持一个交换链（一个窗口）。
//!
//! ## 纹理（T1.2 之后）
//!
//! HAL 的纹理走 [`VulkanDevice`] **自己开的设备**（[`VkDevice::open`]，**惰性**：
//! 第一次才建），与窗口链的设备是**两条**。这看着像重复，但它是 M2b 既定事实的延续：
//! HAL 的 `Device::open` 在拿到窗口之前就要能建纹理（字形图集/图片不依赖窗口），
//! 而窗口链的实例是 `WindowedRenderer::new` 自己建的（`VkSurfaceKHR` 必须属于创建
//! 它的那个实例）。把两条合并成一条是 M3 的工程活（`hal.rs` 顶部的「三条刻意的设计」①）。
//!
//! 纹理句柄 [`TextureId`] 映射到本模块的槽位表；`read_texture_bytes` 是**固有方法**
//! （不在 HAL `Device` trait 上）⇒ 回读四通道保真的判据可以从 HAL 侧调用，
//! **不动公开 trait**（改 trait 按 `AGENTS.md` §6 需要先登记）。

use std::cell::RefCell;
use std::rc::Rc;

use deer_core::{ DrawList, GpuError, GpuResult, TextureId };
use deer_gpu::{ AdapterInfo, Device, Extent, Frame, PresentResult, RawWindowHandle, Swapchain, TargetFormat, TextureDesc, TextureRegion };

use crate::device::{Texture, TextureFormat, UploadRegion, VkDevice};
use crate::windowed::{FrameOutcome, WindowedRenderer};

/// 共享的窗口链句柄：`Swapchain` 与 `Frame` 都要碰它，而 HAL 的两个 trait 对象
/// 无法互相借用 —— 所以用 `Rc<RefCell<..>>`（HAL 刻意不要求 `Send`/`Sync`）。
type Chain = Rc<RefCell<Option<WindowedRenderer>>>;

/// HAL 纹理的后端资源（槽位表的一项）。
///
/// 记下 `format`：`Texture` 自己也记着，但这里再记一份是为了**在 `upload_texture`
/// 找不到纹理时**也能在错误信息里说清「这个 id 期望什么格式」，而不必先解包
/// `Option<Texture>`（拿不到就没有格式可读）。
#[derive(Debug)]
struct TextureSlot {
    texture: Texture,
    format: TextureFormat,
}

/// 纹理槽位表 + 它所属的设备。
///
/// `VkDevice` 在第一次建纹理时才打开（[`VulkanDevice::with_store`]）：
/// 维持 `VulkanDevice::new` 的「只记适配器、不建逻辑设备」契约 —— 单纯枚举适配器
/// 或只是开个交换链的调用方不该被迫付一次设备创建。
///
/// ## 字段顺序**不是**随意的（这是本结构唯一会静默出错的地方）
///
/// Rust 按**声明顺序**析构 ⇒ `slots` 必须**先于** `device` 声明，
/// 于是纹理（及其 `VkImageView`/`VkImage`/`VkDeviceMemory`）先被销毁，设备最后才销毁。
/// 反过来写会得到：设备先销毁 → 再销毁纹理的视图 ⇒ 向**已失效的设备**句柄调用
/// `vkDestroyImageView`。这个错误**不会**在编译期或普通运行里出现，
/// 它表现为「测试全绿、但进程退出时 STATUS_ACCESS_VIOLATION」——
/// 本文件第一版就这么错过一次（Vulkan Loader 报
/// `vkDestroyImageView: Invalid device [VUID-vkDestroyImageView-device-parameter]`）。
struct TextureStore {
    /// 槽位表。`None` = 该 id 已被释放（本任务不做释放，但留出形状让后续能加，
    /// 且 `Option` 让「id 从未分配过」与「id 已释放」在**语义上可区分**）。
    ///
    /// **必须声明在 `device` 之前**（析构顺序 = 声明顺序，见上）。
    slots: Vec<Option<TextureSlot>>,
    /// 纹理所属的设备。**必须最后声明**（见上）。
    device: VkDevice,
}

impl TextureStore {
    fn new(adapter_index: usize) -> GpuResult<TextureStore> {
        Ok(TextureStore {
            slots: Vec::new(),
            device: VkDevice::open(adapter_index)?,
        })
    }

    /// 分配一个新槽位，返回 HAL 句柄。
    fn push(&mut self, slot: TextureSlot) -> TextureId {
        // 复用空槽（本任务不会产生，但保持「id 单调不复用」与「复用空槽」两种
        // 策略都好换；这里选**复用最低空槽**，于是 id 值域不随历史无限增长）。
        if let Some(i) = self.slots.iter().position(|s| s.is_none()) {
            self.slots[i] = Some(slot);
            return TextureId(i as u32);
        }
        self.slots.push(Some(slot));
        TextureId((self.slots.len() - 1) as u32)
    }

    fn get(&self, id: TextureId) -> Option<&TextureSlot> {
        self.slots.get(id.0 as usize)?.as_ref()
    }
}

/// Vulkan 后端在 HAL 里的设备。
pub struct VulkanDevice {
    adapter_index: usize,
    adapter: AdapterInfo,
    chain: Chain,
    /// 纹理用的设备 + 槽位表（惰性建，见 [`TextureStore`]）。
    textures: RefCell<Option<TextureStore>>,
}

impl VulkanDevice {
    /// 只记下适配器（**不建逻辑设备**）—— 链要等 `create_swapchain` 拿到窗口再建。
    pub fn new(adapter_index: usize, adapter: AdapterInfo) -> VulkanDevice {
        VulkanDevice {
            adapter_index,
            adapter,
            chain: Rc::new(RefCell::new(None)),
            textures: RefCell::new(None),
        }
    }

    /// 适配器索引（诊断用）。
    pub fn adapter_index(&self) -> usize {
        self.adapter_index
    }

    /// 确保纹理设备已打开（第一次调用时打开），并借用它做一件事。
    ///
    /// 把「惰性打开」收在一个地方：`create_texture` 与 `upload_texture` 都不该
    /// 各自写一遍「if None { open }」——那样两处一旦不一致，就会出现
    /// 「建纹理用的设备」与「上传纹理用的设备」是两个设备（Vulkan 对象跨设备非法）。
    fn with_store<T>(
        &self,
        f: impl FnOnce(&mut TextureStore) -> GpuResult<T>,
    ) -> GpuResult<T> {
        let mut guard = self.textures.borrow_mut();
        if guard.is_none() {
            *guard = Some(TextureStore::new(self.adapter_index)?);
        }
        let store = guard
            .as_mut()
            .ok_or_else(|| GpuError::Driver {
                code: -1,
                message: "deer-vk: 纹理设备刚建好却取不到（内部不一致）".to_string(),
            })?;
        f(store)
    }

    /// **回读**纹理像素（四通道保真的直接判据）。
    ///
    /// ## 为什么是固有方法而不是 HAL `Device` trait 上的方法
    ///
    /// HAL `Device` 的公开面刻意小（建链 / 建纹理 / 上传 / 帧 / 等空闲）。
    /// 加一个 `read_texture` 到 trait 上属于**公开 API 变更** —— 按 `AGENTS.md` §6
    /// 与 `DEV-PLAN` T1.4 的精神，这类改动要先在 `ROADMAP.md` 登记再动手。
    /// 而本任务（T1.2）的验收判据「回读四通道保真」**不需要**改 trait 就能拿到，
    /// 所以这里用固有能力，把 trait 变更留给真正需要它的任务。
    ///
    /// ## 语义
    ///
    /// - 返回字节序与上传时一致（`R8` 1 字节/像素；`RGBA8` R,G,B,A 4 字节/像素）；
    /// - 长度 = `宽 × 高 × 每像素字节数`；
    /// - 未知/已释放的 id 明确报错（**不返回空 vec 冒充成功**）。
    pub fn read_texture_bytes(&self, id: TextureId) -> GpuResult<Vec<u8>> {
        self.with_store(|store| {
            let slot = store.get(id).ok_or_else(|| {
                GpuError::Unsupported(format!(
                    "deer-vk: 纹理 id {:?} 不存在（可能从未分配或已被释放）",
                    id
                ))
            })?;
            store.device.read_texture_bytes(&slot.texture)
        })
    }

    /// 某个 HAL 纹理的尺寸（诊断/判据用）。
    pub fn texture_extent(&self, id: TextureId) -> Option<(u32, u32)> {
        let guard = self.textures.borrow();
        let store = guard.as_ref()?;
        store.get(id).map(|s| (s.texture.width(), s.texture.height()))
    }

    /// 某个 HAL 纹理的**后端格式**（诊断/判据用）。
    ///
    /// 存在的理由与 `Texture::width()` 一样：HAL 的 `TextureDesc` 是**请求**，
    /// 后端可能做了映射（例如 `Bgra8Srgb` ⇒ `Rgba8Unorm`，见 [`texture_format_of`]）。
    /// 判据若只看请求值，就无法发现「映射错了」；能读回**实际**格式才闭得上这条环。
    pub fn texture_format(&self, id: TextureId) -> Option<TextureFormat> {
        let guard = self.textures.borrow();
        let store = guard.as_ref()?;
        store.get(id).map(|s| s.format)
    }
}

impl Device for VulkanDevice {
    fn info(&self) -> &AdapterInfo {
        &self.adapter
    }

    fn create_swapchain(
        &mut self,
        window: RawWindowHandle,
        extent: Extent,
        _format: TargetFormat,
    ) -> GpuResult<Box<dyn Swapchain>> {
        // 复用 M2b 已验证的那条链（它自己做：实例扩展 → surface → 设备 → 交换链 → 管线）。
        // 清屏色用黑色；真正的清屏色由示例/上层决定（M3 会把 `DrawCmd` 送上来后一并处理）。
        let renderer = WindowedRenderer::new(
            self.adapter_index,
            window,
            extent,
            deer_core::Color::rgb(0, 0, 0),
        )?;
        let actual = renderer.extent();
        let format = target_format_of(renderer.format());
        *self.chain.borrow_mut() = Some(renderer);
        Ok(Box::new(VulkanSwapchain {
            chain: Rc::clone(&self.chain),
            extent: actual,
            format,
        }))
    }

    fn create_texture(&mut self, desc: TextureDesc) -> GpuResult<TextureId> {
        let format = texture_format_of(desc.format);
        if desc.width == 0 || desc.height == 0 {
            return Err(GpuError::Unsupported(format!(
                "deer-vk: 纹理宽高必须 > 0，实际 {}×{}",
                desc.width, desc.height
            )));
        }
        // 初值全 0（透明/黑）：`create_texture` 的既有契约要求「数据长度 = w*h*bpp」，
        // 所以这里给一块**尺寸正确**的零缓冲，而不是空切片。
        let bpp = format.bytes_per_pixel() as usize;
        let zeros = vec![0u8; desc.width as usize * desc.height as usize * bpp];
        self.with_store(|store| {
            let texture = store
                .device
                .create_texture(desc.width, desc.height, format, &zeros)?;
            Ok(store.push(TextureSlot { texture, format }))
        })
    }

    fn upload_texture(
        &mut self,
        id: TextureId,
        data: &[u8],
        region: TextureRegion,
    ) -> GpuResult<()> {
        self.with_store(|store| {
            let slot = store.get(id).ok_or_else(|| {
                GpuError::Unsupported(format!(
                    "deer-vk: 纹理 id {:?} 不存在（可能从未分配或已被释放）",
                    id
                ))
            })?;
            store.device.upload_texture_region(
                &slot.texture,
                UploadRegion {
                    x: region.x,
                    y: region.y,
                    width: region.width,
                    height: region.height,
                },
                data,
            )
        })
    }

    fn begin_frame(&mut self) -> GpuResult<Box<dyn Frame>> {
        if self.chain.borrow().is_none() {
            return Err(GpuError::Unsupported(
                "deer-vk: 还没有交换链 —— 先调 Device::create_swapchain（HAL 在打开设备时还不知道窗口）"
                    .to_string(),
            ));
        }
        Ok(Box::new(VulkanFrame {
            chain: Rc::clone(&self.chain),
            pending_ui: None,
        }))
    }

    fn wait_idle(&mut self) -> GpuResult<()> {
        match self.chain.borrow_mut().as_mut() {
            Some(r) => r.wait_idle(),
            None => Ok(()),
        }
    }
}

/// HAL 的目标格式 → 纹理格式。
///
/// ## 三种格式里只有两种是纹理格式
///
/// `TargetFormat` 的三种取值描述的是**绘制目标**（交换链图像/离屏目标），
/// 而纹理只有 `R8_UNORM`（覆盖率）与 `R8G8B8A8_UNORM`（通用彩色）两种。
/// 映射规则：
///
/// - `Rgba8Unorm` ⇒ `Rgba8Unorm`（**逐字对应**：这是回读/截图路径用的线性格式，
///   与纹理的 UNORM 语义一致，四通道逐字节保真）；
/// - `Bgra8Srgb` / `Rgba8Srgb` ⇒ `Rgba8Unorm`：纹理采样与 sRGB 无关
///   （本项目统一片元着色器只读 R 通道，且**不做** sRGB 解码），
///   而 `TextureFormat` 没有 sRGB 变体 ⇒ 归到 UNORM。
///   这是**有意的降级**，不是遗漏：`Bgra8Srgb` 的 B/R 换位在这条路径上不成立
///   （纹理数据永远是主机给的 R,G,B,A 顺序）。
///
/// 返回 `Rgba8Unorm` 而不报错，是因为三种取值**都有**一个诚实的落点；
/// 「无法表达」这种情况在当前取值域里不存在。若将来 `TargetFormat` 加了
/// 深度/浮点等纹理无法承载的格式，这里应改为返回 `GpuResult` 并明确报错
/// —— 静默按某个格式建，会让「我传了 X 数据」变成「结果不对」这种归因困难的缺陷。
fn texture_format_of(format: TargetFormat) -> TextureFormat {
    match format {
        TargetFormat::Rgba8Unorm => TextureFormat::Rgba8Unorm,
        // sRGB 目标：纹理侧没有对应格式，按 UNORM 建并**如实**降级（见上）。
        TargetFormat::Bgra8Srgb | TargetFormat::Rgba8Srgb => TextureFormat::Rgba8Unorm,
    }
}

/// 交换链句柄（HAL 形状）。
pub struct VulkanSwapchain {
    chain: Chain,
    extent: Extent,
    format: TargetFormat,
}

impl Swapchain for VulkanSwapchain {
    fn extent(&self) -> Extent {
        self.extent
    }

    fn format(&self) -> TargetFormat {
        self.format
    }

    fn resize(&mut self, extent: Extent) -> GpuResult<()> {
        let mut guard = self.chain.borrow_mut();
        let r = guard.as_mut().ok_or_else(|| GpuError::Driver {
            code: -1,
            message: "deer-vk: 交换链已被释放".to_string(),
        })?;
        r.resize(extent)?;
        self.extent = r.extent();
        Ok(())
    }
}

/// 一帧（HAL 形状）。
///
/// ## `pending_ui`：本帧已经准备好的统一顶点流（T1.1）
///
/// HAL 的 `Frame` 把「录制」与「提交并呈现」拆成两个方法，而窗口路径的
/// `draw_and_present` 是「准备 → 取图 → 录制 → 提交 → 呈现」一把梭。为复用同一段
/// 实现，本类型在 `record` 里调 [`crate::windowed::WindowedRenderer::prepare_ui`]
/// 做好 CPU 侧准备（建资源 / 建顶点流 / 上传缓冲 / 刷图集），把产物**暂存在这里**；
/// `submit_and_present` 再调 `present_prepared` 走完帧舞蹈。
///
/// 暂存在**帧对象**上（而不是 `WindowedRenderer` 里）是刻意的：HAL 允许多次
/// `begin_frame`，若把本帧顶点存进共享的 `chain`，两个帧对象就会互相覆盖。
pub struct VulkanFrame {
    chain: Chain,
    /// `record` 的准备产物；`submit_and_present` 消费它。
    ///
    /// `None` = 还没调过 `record`（或 `begin_frame` 之后直接提交）——
    /// 按「空帧」处理（只清屏 + 呈现）。
    pending_ui: Option<Vec<crate::vertex_unify::UnifiedVertex>>,
}

impl Frame for VulkanFrame {
    fn record(
        &mut self,
        list: &DrawList,
        text: Option<&mut deer_gpu::TextEngine>,
    ) -> GpuResult<()> {
        // T1.1：把 `DrawList` 真的送上 GPU —— 复用窗口路径那条链
        // （[`crate::windowed::WindowedRenderer::prepare_ui`]，与 `draw_and_present`
        // **共用同一段实现**）。
        //
        // ## 这里不再有 `Unsupported(M3)` 分支
        //
        // M2b 时这里对任何非 `NodeHint` 命令报错，是为了**不让「窗口里什么都没有」
        // 变成一个查不出的 bug**（静默忽略更坏）。M3a/M3b/M3c/M3+ 已经落地
        // （形状 + 文本 → 统一管线 → 上屏）⇒ 这个分支的**理由消失了**，删掉它。
        //
        // ## 错误语义（与 `draw_and_present` 逐条一致）
        //
        // - 有文本命令但 `text == None` ⇒ `Unsupported`（**不静默丢弃**）；
        // - 裁剪栈不平衡 ⇒ `Unsupported`；
        // - 形状/文本建流遇到不支持的输入 ⇒ `Unsupported`。
        //
        // 本方法**只做准备**；取图/录制/提交/呈现都在 `submit_and_present` ——
        // 那才是 HAL 契约里「提交」发生的地方。
        let mut guard = self.chain.borrow_mut();
        let s = guard.as_mut().ok_or_else(|| GpuError::Driver {
            code: -1,
            message: "deer-vk: HAL 帧的交换链已被释放（create_swapchain 之后又动了 chain？）"
                .to_string(),
        })?;
        self.pending_ui = Some(s.prepare_ui(list, text)?);
        Ok(())
    }

    fn read_pixels(&mut self) -> GpuResult<Vec<u8>> {
        Err(GpuError::Unsupported(
            "deer-vk: HAL 的 `Frame::read_pixels` 在提交前调用，而交换链图像的回读数据只有\
             呈现之后才有效；请用 `WindowedRenderer::read_back_last_frame()`（在 \
             `render_and_present()` 之后调，取回上一帧的像素）。离屏回读用 \
             `deer_vk::OffscreenRenderer`。"
                .to_string(),
        ))
    }

    fn submit_and_present(self: Box<Self>) -> GpuResult<PresentResult> {
        // 帧舞蹈（取图 → 录制 → 提交 → 呈现）：与窗口路径 `draw_and_present` 完全同一条。
        // `pending_ui` 为 `None`（没调过 `record`）⇒ 用空流 ⇒ 只清屏 + 呈现。
        let mut guard = self.chain.borrow_mut();
        let r = guard.as_mut().ok_or_else(|| GpuError::Driver {
            code: -1,
            message: "deer-vk: 交换链已被释放".to_string(),
        })?;
        let unified = self.pending_ui.as_deref().unwrap_or(&[]);
        Ok(present_result_of(r.present_prepared(unified)?))
    }
}

/// 交换链的呈现结果 → HAL 的呈现结果。
///
/// **抽成独立函数是为了能被单测钉住**：`OutOfDate ⇒ Presented` 这种「把过期当成功」的退化
/// 在端到端路径上很难触发（需要刚好改窗口尺寸），所以用纯函数 + 单测守住这条契约
/// （独立验证者曾把这里的映射改反，而当时全仓 202 条测试无一变红 —— 这就是缺口）。
/// 真窗口下的 HAL 路径端到端断言见 `crates/deer-gui/tests/windowed_hal.rs`。
fn present_result_of(outcome: FrameOutcome) -> PresentResult {
    match outcome {
        FrameOutcome::Presented => PresentResult::Presented,
        // 交换链过期是**正常路径**：如实上报，让调用方 resize 后重试。绝不当成功。
        FrameOutcome::OutOfDate => PresentResult::OutOfDate,
    }
}

/// Vulkan 的交换链格式 → HAL 的目标格式。
fn target_format_of(vk_format: i32) -> TargetFormat {
    match vk_format {
        crate::swapchain::VK_FORMAT_B8G8R8A8_SRGB => TargetFormat::Bgra8Srgb,
        crate::swapchain::VK_FORMAT_R8G8B8A8_SRGB => TargetFormat::Rgba8Srgb,
        _ => TargetFormat::Rgba8Unorm,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn present_outcome_mapping_never_treats_out_of_date_as_success() {
        // 这条断言值钱的原因：独立验证者把 `submit_and_present` 里的映射改反
        // （OutOfDate ⇒ Presented）时，全仓 202 条测试**没有一条变红** —— 因为端到端
        // 路径要刚好触发交换链过期才走到这里。所以用纯函数把它钉死。
        assert_eq!(
            present_result_of(FrameOutcome::Presented),
            PresentResult::Presented
        );
        assert_eq!(
            present_result_of(FrameOutcome::OutOfDate),
            PresentResult::OutOfDate,
            "交换链过期必须如实上报，绝不能当作成功"
        );
        assert_ne!(
            present_result_of(FrameOutcome::OutOfDate),
            PresentResult::Presented
        );
    }

    #[test]
    fn format_mapping_is_explicit() {
        assert_eq!(
            target_format_of(crate::swapchain::VK_FORMAT_B8G8R8A8_SRGB),
            TargetFormat::Bgra8Srgb
        );
        assert_eq!(
            target_format_of(crate::swapchain::VK_FORMAT_R8G8B8A8_SRGB),
            TargetFormat::Rgba8Srgb
        );
        assert_eq!(target_format_of(0), TargetFormat::Rgba8Unorm);
    }

    #[test]
    fn device_reports_adapter_without_opening_the_chain() {
        let d = VulkanDevice::new(3, AdapterInfo {
            name: "fake".to_string(),
            kind: deer_gpu::AdapterKind::DiscreteGpu,
            driver: "test".to_string(),
        });
        assert_eq!(d.info().name, "fake");
        assert_eq!(d.adapter_index(), 3);
    }

    #[test]
    fn begin_frame_without_swapchain_is_a_loud_error() {
        let mut d = VulkanDevice::new(0, AdapterInfo {
            name: "fake".to_string(),
            kind: deer_gpu::AdapterKind::Cpu,
            driver: "test".to_string(),
        });
        let err = d.begin_frame().err().expect("没有交换链时必须报错");
        assert!(
            err.to_string().contains("交换链"),
            "错误信息要说清原因，实际：{err}"
        );
    }

    #[test]
    fn record_reports_a_missing_engine_instead_of_silently_dropping_text() {
        // T1.1 之后这里**不再**是「非空列表必须报 M3」——那个分支已被删掉
        // （M3a/M3b/M3c/M3+ 已落地，理由消失）。本用例钉住的是**这条链上不依赖
        // 真实窗口/驱动**的部分：
        //
        //   ① 没有交换链（空链）时 `record` 明确报错（不是 panic、不是假装成功）；
        //   ② `read_pixels` 在提交前调用必须明确报错，并把调用方指向真正可用的 API。
        //
        // 为什么**不**在这里钉「有文本命令却没给引擎 ⇒ Unsupported」：
        //   那条分支在 `prepare_ui` **之后**才可能命中，而 `prepare_ui` 需要一条真实
        //   交换链（`begin_frame` 的窗口路径），单测里造不出来。该契约由
        //   `deer-gpu/tests/draw_list_and_cpu_backend.rs` 里**同一条语义**的 CPU 用例
        //   `cpu_backend_reports_text_without_engine_instead_of_dropping_it` 钉住
        //   （两端共用同一份契约文档，见 [`Frame::record`]）。
        //
        // 用空链构造一帧：`prepare_ui` 会先取链，链为 `None` ⇒ 报错。
        let frame_chain: Chain = Rc::new(RefCell::new(None));
        let mut f = VulkanFrame {
            chain: frame_chain,
            pending_ui: None,
        };

        // 前置断言：链**确实**是空的（否则下面测的就不是「没交换链」这条路径）。
        assert!(
            f.chain.borrow().is_none(),
            "前置：本用例要求一条**空**链"
        );

        // 空列表：仍然必须**先**发现「没有交换链」——这是本用例真正钉住的第一条。
        let err = f
            .record(&DrawList::new(), None)
            .expect_err("没有交换链时 record 必须明确报错");
        assert!(
            err.to_string().contains("交换链"),
            "错误信息要说清原因（交换链被释放/未创建），实际：{err}"
        );

        // read_pixels 同样必须明确报错，并把调用方指向真正可用的路径
        let err = f
            .read_pixels()
            .expect_err("HAL 的 read_pixels 在提交前调用，必须明确报错");
        assert!(
            err.to_string().contains("read_back_last_frame"),
            "错误信息要指向真正可用的 API，实际：{err}"
        );
    }
}
