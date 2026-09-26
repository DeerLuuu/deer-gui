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
//! 里程碑 M1 只到「实例 + 物理设备枚举」：这就能验证
//! ① 符号解析正确、② 结构体布局正确、③ 这台机器的 Vulkan 可用。
//! 逻辑设备 / 交换链 / 管线 / 出图在 M2–M3。

#![deny(clippy::all)]

pub mod device;
pub mod ffi;
pub mod ffi_dev;
pub mod loader;
pub mod spirv;

pub use device::{ShaderModule, VkDevice};

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
    pub fn new() -> GpuResult<VkBackend> {
        let instance = ffi::Instance::create()?;
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
        if adapter >= self.physical_devices.len() {
            return Err(GpuError::NoAdapter);
        }
        // M2 在此创建 VkDevice + 队列。当前**明确报「未实现」**，
        // 而不是返回一个假装能用的对象（假成功比报错难查得多）。
        Err(GpuError::Unsupported(
            "deer-vk: 逻辑设备创建在里程碑 M2 实现（当前只到实例 + 设备枚举）".to_string(),
        ))
    }
}
