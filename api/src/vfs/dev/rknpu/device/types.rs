//! RKNPU 基础类型定义
//!
//! 本模块定义了 RKNPU 驱动中使用的基础类型、枚举和错误类型。

use core::fmt;

/// RKNPU 错误类型
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RknpuError {
    /// 无效的参数
    InvalidParameter,
    /// 设备未初始化
    NotInitialized,
    /// 电源操作失败
    PowerOperationFailed,
    /// 复位操作失败
    ResetFailed,
    /// 超时
    Timeout,
    /// 核心不可用
    CoreUnavailable,
    /// 内存操作失败
    MemoryError,
    /// 不支持的操作
    Unsupported,
    /// 电源验证失败
    PowerVerificationFailed,
    /// 硬件未就绪
    HardwareNotReady,
    /// 任务提交失败
    TaskSubmitFailed,
    /// 任务执行失败
    TaskExecutionFailed,
    /// 中断状态异常
    InvalidInterruptStatus,
}

impl fmt::Display for RknpuError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidParameter => write!(f, "Invalid parameter"),
            Self::NotInitialized => write!(f, "Device not initialized"),
            Self::PowerOperationFailed => write!(f, "Power operation failed"),
            Self::ResetFailed => write!(f, "Reset operation failed"),
            Self::Timeout => write!(f, "Operation timeout"),
            Self::CoreUnavailable => write!(f, "NPU core unavailable"),
            Self::MemoryError => write!(f, "Memory operation error"),
            Self::Unsupported => write!(f, "Unsupported operation"),
            Self::PowerVerificationFailed => write!(f, "Power verification failed"),
            Self::HardwareNotReady => write!(f, "Hardware not ready or inaccessible"),
            Self::TaskSubmitFailed => write!(f, "Task submit failed"),
            Self::TaskExecutionFailed => write!(f, "Task execution failed"),
            Self::InvalidInterruptStatus => write!(f, "Invalid interrupt status"),
        }
    }
}

/// RKNPU 操作结果
pub type Result<T> = core::result::Result<T, RknpuError>;

/// Rockchip 板型枚举
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RkBoard {
    /// RK3588 - 3个 NPU 核心
    Rk3588,
    /// RK3568 - 1个 NPU 核心
    Rk3568,
    /// RV1106 - 1个 NPU 核心
    Rv1106,
    /// RK3562 - 1个 NPU 核心
    Rk3562,
    /// RK3583 - 2个 NPU 核心
    Rk3583,
}

impl RkBoard {
    /// 获取该板型支持的 NPU 核心数量
    pub const fn num_cores(&self) -> usize {
        match self {
            Self::Rk3588 => 3,
            Self::Rk3583 => 2,
            Self::Rk3568 | Self::Rv1106 | Self::Rk3562 => 1,
        }
    }

    /// 获取核心掩码
    pub const fn core_mask(&self) -> u32 {
        match self {
            Self::Rk3588 => 0x7,                               // 0b111 - 3 cores
            Self::Rk3583 => 0x3,                               // 0b011 - 2 cores
            Self::Rk3568 | Self::Rv1106 | Self::Rk3562 => 0x1, // 0b001 - 1 core
        }
    }
}

/// NPU 核心标识
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NpuCore {
    /// NPU 核心 0
    Npu0 = 0,
    /// NPU 核心 1
    Npu1 = 1,
    /// NPU 核心 2
    Npu2 = 2,
}

impl NpuCore {
    /// 从索引创建核心标识
    pub const fn from_index(index: usize) -> Option<Self> {
        match index {
            0 => Some(Self::Npu0),
            1 => Some(Self::Npu1),
            2 => Some(Self::Npu2),
            _ => None,
        }
    }

    /// 获取核心索引
    pub const fn index(&self) -> usize {
        *self as usize
    }

    /// 获取核心掩码位
    pub const fn mask_bit(&self) -> u32 {
        1 << (*self as u32)
    }
}

/// 电源域标识（兼容名称）
pub use NpuCore as PowerDomain;

/// NPU0 电源域常量
pub const NPU0: NpuCore = NpuCore::Npu0;
/// NPU1 电源域常量
pub const NPU1: NpuCore = NpuCore::Npu1;
/// NPU2 电源域常量
pub const NPU2: NpuCore = NpuCore::Npu2;

/// 电源状态
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PowerState {
    /// 电源关闭
    Off,
    /// 电源打开
    On,
}

/// 复位类型
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResetType {
    /// AXI 总线复位
    Axi,
    /// AHB 总线复位
    Ahb,
    /// 软复位（包括 AXI 和 AHB）
    Soft,
}
