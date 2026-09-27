//! # deer-vk —— Vulkan 后端
//!
//! **不依赖 `ash` / `vulkano` / `wgpu`**：Vulkan 的符号在这里自己声明
//! （见 [`ffi`] 模块），命令缓冲、管线、同步、交换链全部自己写。
//!
//! ## 为什么能不用 SDK
//!
//! Windows 上系统自带 `vulkan-1.dll`（loader）。我们只链接它的**导入库**
//! `vulkan-1.lib`（随 Windows SDK 提供），符号声明与结构体布局全在本 crate 里 ——
//! 因此**不需要安装 Vulkan SDK**。SPIR-V 着色器当前以**预编译字节数组**形式内嵌
//! （无 SDK ⇒ 无 `glslc`），后续换成自写编译器或预生成流程。
//!
//! ## 当前状态
//!
//! | 里程碑 | 内容 | 状态 |
//! |---|---|---|
//! | M1 | 实例 + 物理设备枚举 | ✅ |
//! | M2a | 逻辑设备 + 自研 SPIR-V + 渲染通道/管线 + 离屏图像与回读（逐像素验证） | ✅ |
//! | M2b | **窗口 + `VkSurfaceKHR` + 交换链 + 帧同步 + 呈现**（`surface.rs` / `swapchain.rs` / `windowed.rs`） | ✅ |
//! | M3 | **把 `DrawList`（矩形/圆角/文本）送上 GPU**、纹理与字形图集上传 | ⬜ |
//!
//! HAL 接线见 [`hal`]：`VkBackend::open()` 返回的 `VkDevice` 现在是**真设备**，
//! `create_swapchain` 会建出真正能呈现的交换链。**注意 M2b 的边界**：
//! `Frame::record` 对非空 `DrawList` 会明确报 `Unsupported`（那是 M3），
//! 纹理上传与交换链回读同样未实现 —— 都报错，不静默假装。

#![deny(clippy::all)]

pub mod device;
pub mod ffi;
pub mod ffi_dev;
pub mod gpu_geom;
pub mod gpu_render;
pub mod gpu_text;
pub mod hal;
pub mod loader;
pub mod offscreen;
pub mod pipelines;
pub mod spirv;
pub mod surface;
pub mod swapchain;
pub mod windowed;

pub use device::{Pipeline, PipelineLayout, RenderPass, ShaderModule, VertexAttr, VkDevice};
pub use gpu_geom::{GpuStream, GpuVertex};
pub use gpu_render::GpuGeometryRenderer;
pub use gpu_text::{build_text_stream, TextStream, TextVertex};
pub use hal::{VulkanDevice, VulkanFrame, VulkanSwapchain};
pub use offscreen::{Buffer, CommandPool, Fence, Framebuffer, Image, ImageView, Memory, OffscreenRenderer};
pub use pipelines::{build_pipelines, PipelineState, PipelineSet, ViewportStrategy};
pub use surface::Surface;
pub use swapchain::{Acquire, Present, Semaphore, Swapchain, SwapchainConfig};
pub use windowed::{FrameOutcome, WindowedRenderer};

use deer_gpu::{AdapterInfo, AdapterKind, Backend, Device, GpuError, GpuResult};
use ffi::PhysicalDeviceType;

/// Vulkan 后端。
///
/// 构造时创建 `VkInstance` 并枚举物理设备。若 loader 缺失或没有设备，
/// 返回 [`GpuError`]（**不 panic** —— HAL 的约定）。
pub struct VkBackend {
    instance: ffi::Instance,
    adapters: Vec<AdapterInfo>,
    physical_devices: Vec<ffi::PhysicalDeviceHandle>,
}

impl VkBackend {
    /// 打开 Vulkan 实例并枚举设备。
    ///
    /// 若环境变量 **`DEER_VK_VALIDATION=1`** 被设置，则启用
    /// `VK_LAYER_KHRONOS_validation` 并把消息打到 stderr。
    ///
    /// 为什么用环境变量而不是编译期 feature：本项目暂不引任何第三方依赖，
    /// 也没有 feature 开关体系；环境变量能在**不改代码、不重编译**的情况下
    /// 对同一个二进制开诊断 —— 这正是排查驱动「不报错也不画」这类问题时需要的。
    pub fn new() -> GpuResult<VkBackend> {
        let want_validation = std::env::var("DEER_VK_VALIDATION")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);
        let instance = ffi::Instance::create_with_validation(want_validation)?;
        if instance.validation_enabled() {
            eprintln!("[deer-vk] 已启用 VK_LAYER_KHRONOS_validation（消息将打到 stderr）");
        }
        let physical_devices = instance.enumerate_physical_devices()?;
        if physical_devices.is_empty() {
            return Err(GpuError::NoAdapter);
        }
        let mut adapters = Vec::with_capacity(physical_devices.len());
        for pd in &physical_devices {
            // SAFETY: `pd` 来自本实例的 `enumerate_physical_devices()`，实例仍存活。
            let props = unsafe { instance.physical_device_properties(*pd)? };
            adapters.push(AdapterInfo {
                name: props.device_name,
                kind: match props.device_type {
                    PhysicalDeviceType::DiscreteGpu => AdapterKind::DiscreteGpu,
                    PhysicalDeviceType::IntegratedGpu => AdapterKind::IntegratedGpu,
                    PhysicalDeviceType::VirtualGpu => AdapterKind::VirtualGpu,
                    PhysicalDeviceType::Cpu => AdapterKind::Cpu,
                    PhysicalDeviceType::Other => AdapterKind::Other,
                },
                driver: format!(
                    "Vulkan {}.{}.{} (apiVersion {:#010x}, driver {:#010x})",
                    (props.api_version >> 22) & 0x7f,
                    (props.api_version >> 12) & 0x3ff,
                    props.api_version & 0xfff,
                    props.api_version,
                    props.driver_version
                ),
            });
        }
        Ok(VkBackend {
            instance,
            adapters,
            physical_devices,
        })
    }

    /// 底层实例（诊断/测试用）。
    pub fn instance(&self) -> &ffi::Instance {
        &self.instance
    }

    /// 物理设备原始句柄（诊断/测试用）。
    pub fn physical_devices(&self) -> &[ffi::PhysicalDeviceHandle] {
        &self.physical_devices
    }
}

impl Backend for VkBackend {
    fn name(&self) -> &'static str {
        "vulkan"
    }

    fn adapters(&self) -> Vec<AdapterInfo> {
        self.adapters.clone()
    }

    fn open(&self, adapter: usize) -> GpuResult<Box<dyn Device>> {
        let info = self
            .adapters
            .get(adapter)
            .cloned()
            .ok_or(GpuError::NoAdapter)?;
        // M2b：返回**真设备**（此前这里返回「M2 未实现」）。
        // 逻辑设备与交换链推迟到 `create_swapchain` 拿到窗口时创建 —— 见 [`hal`] 的设计说明。
        Ok(Box::new(hal::VulkanDevice::new(adapter, info)))
    }
}
