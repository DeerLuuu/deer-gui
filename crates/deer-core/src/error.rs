//! HAL 错误类型。
//!
//! 原则：**后端初始化失败不得 panic** —— 必须返回错误，让上层决定回退
//! （例如「Vulkan 不可用 ⇒ 换 CPU 后端」）。这条在无 GPU 的 CI 上是刚性需求。

use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GpuError {
    /// 没有可用的图形适配器。
    NoAdapter,
    /// 所需扩展 / 特性不支持。
    Unsupported(String),
    /// 驱动或运行时返回了错误码。
    Driver { code: i32, message: String },
    /// 交换链过期（正常路径，调用方应 resize 后重试）。
    OutOfDate,
    /// 窗口句柄无效（平台层问题）。
    BadWindowHandle,
}

impl fmt::Display for GpuError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GpuError::NoAdapter => write!(f, "没有可用的图形适配器"),
            GpuError::Unsupported(what) => write!(f, "不支持：{what}"),
            GpuError::Driver { code, message } => write!(f, "驱动错误 {code}：{message}"),
            GpuError::OutOfDate => write!(f, "交换链过期"),
            GpuError::BadWindowHandle => write!(f, "窗口句柄无效"),
        }
    }
}

impl std::error::Error for GpuError {}

pub type GpuResult<T> = Result<T, GpuError>;
