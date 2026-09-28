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
//! - 纹理（`create_texture` / `upload_texture`）未实现：字形图集上传是 M3；
//! - `read_pixels` 明确报 `Unsupported`：HAL 的这个方法在**提交前**调用，而交换链图像的回读
//!   数据只有**呈现之后**才有效 ⇒ 请用 [`crate::WindowedRenderer::read_back_last_frame()`]
//!   （`render_and_present()` 之后调；交换链图像确实申请了 `TRANSFER_SRC`）。
//!   这里刻意**不做「隐式呈现」**那种惊吓式语义；离屏回读取 [`crate::OffscreenRenderer`]。
//! - 只支持一个交换链（一个窗口）。

use std::cell::RefCell;
use std::rc::Rc;

use deer_gpu::{
    AdapterInfo, Device, DrawCmd, DrawList, Extent, Frame, GpuError, GpuResult, PresentResult,
    RawWindowHandle, Swapchain, TargetFormat, TextureDesc, TextureId, TextureRegion,
};

use crate::windowed::{FrameOutcome, WindowedRenderer};

/// 共享的窗口链句柄：`Swapchain` 与 `Frame` 都要碰它，而 HAL 的两个 trait 对象
/// 无法互相借用 —— 所以用 `Rc<RefCell<..>>`（HAL 刻意不要求 `Send`/`Sync`）。
type Chain = Rc<RefCell<Option<WindowedRenderer>>>;

/// Vulkan 后端在 HAL 里的设备。
pub struct VulkanDevice {
    adapter_index: usize,
    adapter: AdapterInfo,
    chain: Chain,
}

impl VulkanDevice {
    /// 只记下适配器（**不建逻辑设备**）—— 链要等 `create_swapchain` 拿到窗口再建。
    pub fn new(adapter_index: usize, adapter: AdapterInfo) -> VulkanDevice {
        VulkanDevice {
            adapter_index,
            adapter,
            chain: Rc::new(RefCell::new(None)),
        }
    }

    /// 适配器索引（诊断用）。
    pub fn adapter_index(&self) -> usize {
        self.adapter_index
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
            deer_gpu::Color::rgb(0, 0, 0),
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

    fn create_texture(&mut self, _desc: TextureDesc) -> GpuResult<TextureId> {
        Err(GpuError::Unsupported(
            "deer-vk: 纹理创建在里程碑 M3 实现（字形图集上传需要它）".to_string(),
        ))
    }

    fn upload_texture(
        &mut self,
        _id: TextureId,
        _data: &[u8],
        _region: TextureRegion,
    ) -> GpuResult<()> {
        Err(GpuError::Unsupported(
            "deer-vk: 纹理上传在里程碑 M3 实现（字形图集上传需要它）".to_string(),
        ))
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
        }))
    }

    fn wait_idle(&mut self) -> GpuResult<()> {
        match self.chain.borrow_mut().as_mut() {
            Some(r) => r.wait_idle(),
            None => Ok(()),
        }
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
pub struct VulkanFrame {
    chain: Chain,
}

impl Frame for VulkanFrame {
    fn record(&mut self, list: &DrawList) -> GpuResult<()> {
        // `NodeHint` 只是诊断信息，忽略它是诚实的；其它任何绘制命令都意味着
        // 「把 DrawList 送上 GPU」—— 那是 M3，必须明确报错而不是装作画了。
        let has_real_draw = list
            .cmds
            .iter()
            .any(|c| !matches!(c, DrawCmd::NodeHint { .. }));
        if has_real_draw {
            return Err(GpuError::Unsupported(
                "deer-vk: 把 DrawList（矩形/圆角/文本）送上 GPU 是里程碑 M3；\
                 M2b 的窗口路径只呈现清屏色 + 几何（见 docs/features/vulkan-swapchain.md）"
                    .to_string(),
            ));
        }
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
        let mut guard = self.chain.borrow_mut();
        let r = guard.as_mut().ok_or_else(|| GpuError::Driver {
            code: -1,
            message: "deer-vk: 交换链已被释放".to_string(),
        })?;
        Ok(present_result_of(r.render_and_present()?))
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
    use deer_gpu::{Color, RectI};

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
    fn record_rejects_real_draw_commands_until_m3() {
        // 用空链构造一帧：record 不碰链，所以这里能独立测「诚实地报 Unsupported」。
        let frame_chain: Chain = Rc::new(RefCell::new(None));
        let mut f = VulkanFrame { chain: frame_chain };

        // 空列表 / 只有 NodeHint ⇒ 不报错（M2b 的窗口帧本来就只画清屏 + 几何）
        f.record(&DrawList::new()).expect("空列表应当可以记录");
        let mut hints = DrawList::new();
        hints.push(DrawCmd::node_hint(RectI::new(0, 0, 1, 1), "h"));
        f.record(&hints).expect("NodeHint 只是诊断信息，应当可以记录");

        // 真的绘制命令 ⇒ 明确报错（不许静默忽略）
        let mut real = DrawList::new();
        real.push(DrawCmd::FillRect {
            rect: RectI::new(0, 0, 4, 4),
            color: Color::WHITE,
        });
        let err = f.record(&real).expect_err("非空绘制列表必须报错");
        assert!(err.to_string().contains("M3"), "应指明是 M3，实际：{err}");

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
