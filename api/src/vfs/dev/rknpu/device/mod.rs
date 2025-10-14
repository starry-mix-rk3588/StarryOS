//! RKNPU Device 抽象层
//!
//! 本模块提供了 Rockchip NPU (Neural Processing Unit) 的完整抽象接口。
//! 支持多种 Rockchip 芯片，包括 RK3588、RK3568、RV1106 等。
//!
//! # 主要功能
//!
//! - **电源管理**: 支持独立的电源域控制，可以单独开关每个 NPU 核心
//! - **复位控制**: 支持软复位、AXI 复位、AHB 复位
//! - **寄存器访问**: 提供安全的寄存器读写接口
//! - **多核心支持**: 支持最多 3 个 NPU 核心（RK3588）
//!
//! # 示例
//!
//! ```no_run
//! use rknpu_device::{NPU0, NPU1, NPU2, RK3588NPU, RkBoard};
//!
//! // 初始化 NPU (地址从配置自动获取)
//! let mut rknpu = RK3588NPU::new(RkBoard::Rk3588);
//!
//! // 初始化硬件
//! rknpu.init().unwrap();
//!
//! // 单独控制电源域
//! rknpu.power_domain_on(NPU1).unwrap();
//! rknpu.power_domain_off(NPU2).unwrap();
//!
//! // 软复位
//! rknpu.soft_reset().unwrap();
//!
//! // 读写寄存器
//! let version = rknpu.read_reg(NPU0, 0x0).unwrap();
//! rknpu.write_reg(NPU0, 0x20, 0x0).unwrap();
//! ```
//!
//! # 架构
//!
//! 本模块采用模块化设计，按功能划分为以下子模块：
//!
//! - `types`: 基础类型定义（错误类型、枚举等）
//! - `config`: 硬件配置（寄存器定义、芯片参数）
//! - `power`: 电源域管理
//! - `reset`: 复位控制
//! - `rknpu_dev`: 核心设备结构

// 子模块
mod config;
mod power;
mod reset;
mod rknpu_dev;
mod types;

// 示例模块（可选）
#[cfg(feature = "examples")]
pub mod examples;

// 重新导出公共接口
pub use config::{
    addresses, registers, INVALID_REG_VALUE_ALL_ONE, INVALID_REG_VALUE_ALL_ZERO,
    POWER_OFF_VERIFY_TIMEOUT_US, POWER_ON_VERIFY_POLL_INTERVAL_US, POWER_ON_VERIFY_TIMEOUT_US,
    RK3588_NPU_VERSION, RknpuConfig,
};
pub use rknpu_dev::RK3588NPU;
pub use types::{
    NPU0, NPU1, NPU2, NpuCore, PowerDomain, PowerState, ResetType, Result, RkBoard, RknpuError,
};

// 版本信息
/// RKNPU Device 抽象层版本
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// 驱动描述
pub const DRIVER_DESC: &str = "RKNPU Device Abstraction Layer";

/// 驱动日期
pub const DRIVER_DATE: &str = "20241013";
