//! RKNPU 硬件配置
//!
//! 本模块定义了不同 Rockchip 芯片的 NPU 硬件配置参数。
//! 配置参数基于 Linux 内核驱动 crates/rknpu/rknpu_drv.c 中的定义。

use super::types::{NpuCore, RkBoard};

/// RK3588 NPU 硬件地址映射
///
/// 这些地址来自设备树 (rk3588-orangepi-5-plus.dts) 和硬件手册。
/// 所有地址均为物理地址，需要通过 MMU 映射后才能访问。
pub mod addresses {
    /// NPU 核心寄存器基地址
    /// 
    /// 来自设备树: npu@fdab0000
    /// reg = <0x00 0xfdab0000 0x00 0x10000    # NPU0 核心
    ///        0x00 0xfdac0000 0x00 0x10000    # NPU1 核心  
    ///        0x00 0xfdad0000 0x00 0x10000>;  # NPU2 核心
    pub const NPU0_BASE: usize = 0xFDAB0000;
    pub const NPU1_BASE: usize = 0xFDAC0000;
    pub const NPU2_BASE: usize = 0xFDAD0000;
    
    /// 每个核心的寄存器空间大小 (64KB)
    pub const NPU_CORE_SIZE: usize = 0x10000;
    
    /// PMU1 (电源管理单元) 基地址
    /// 
    /// 来自设备树: power-management@fd8d8000
    /// 用于控制 NPU 各核心的电源域开关
    pub const PMU1_BASE: usize = 0xFD8D8000;
    
    /// CRU (时钟复位单元) 基地址
    /// 
    /// 用于控制 NPU 时钟门控和软复位
    pub const CRU_BASE: usize = 0xFD7C0000;
    
    /// GPIO3 基地址
    /// 
    /// 用于某些电源控制引脚的 GPIO 操作
    pub const GPIO3_BASE: usize = 0xFEC40000;
}

/// RKNPU 寄存器偏移量定义
pub mod registers {
    /// 版本寄存器
    pub const VERSION: u32 = 0x0;
    /// 版本号寄存器
    pub const VERSION_NUM: u32 = 0x4;
    /// PC 操作使能寄存器
    pub const PC_OP_EN: u32 = 0x8;
    /// PC 数据地址寄存器
    pub const PC_DATA_ADDR: u32 = 0x10;
    /// PC 数据数量寄存器
    pub const PC_DATA_AMOUNT: u32 = 0x14;
    /// 中断屏蔽寄存器
    pub const INT_MASK: u32 = 0x20;
    /// 中断清除寄存器
    pub const INT_CLEAR: u32 = 0x24;
    /// 中断状态寄存器
    pub const INT_STATUS: u32 = 0x28;
    /// 原始中断状态寄存器
    pub const INT_RAW_STATUS: u32 = 0x2c;
    /// PC 任务控制寄存器
    pub const PC_TASK_CONTROL: u32 = 0x30;
    /// PC DMA 基地址寄存器
    pub const PC_DMA_BASE_ADDR: u32 = 0x34;
    /// PC 任务状态寄存器
    pub const PC_TASK_STATUS: u32 = 0x3c;
    /// 使能掩码寄存器
    pub const ENABLE_MASK: u32 = 0xf008;
    /// 清除所有读写量寄存器
    pub const CLR_ALL_RW_AMOUNT: u32 = 0x8010;
    /// 数据传输写入量寄存器
    pub const DT_WR_AMOUNT: u32 = 0x8034;
    /// 数据传输读取量寄存器
    pub const DT_RD_AMOUNT: u32 = 0x8038;
    /// 权重传输读取量寄存器
    pub const WT_RD_AMOUNT: u32 = 0x803c;
}

/// 中断清除值
pub const INT_CLEAR_VALUE: u32 = 0x1ffff;

/// PC 数据额外数量
pub const PC_DATA_EXTRA_AMOUNT: u32 = 4;

/// 电源验证配置常量
///
/// 这些常量用于电源状态验证和超时控制

/// 电源打开验证超时时间 (微秒)
pub const POWER_ON_VERIFY_TIMEOUT_US: u32 = 100_000; // 100ms

/// 电源验证轮询间隔 (微秒)
pub const POWER_ON_VERIFY_POLL_INTERVAL_US: u32 = 10; // 10us

/// 电源关闭验证超时时间 (微秒)
pub const POWER_OFF_VERIFY_TIMEOUT_US: u32 = 100_000; // 100ms

/// RK3588 NPU 预期版本号 (v6.0.2)
///
/// 从版本寄存器读取时应返回此值，用于验证硬件是否正确初始化
pub const RK3588_NPU_VERSION: u32 = 0x60002;

/// 无效寄存器值 - 全零
///
/// 当硬件未就绪或电源未打开时，寄存器可能返回此值
pub const INVALID_REG_VALUE_ALL_ZERO: u32 = 0x00000000;

/// 无效寄存器值 - 全一
///
/// 当硬件总线错误或设备不可访问时，寄存器可能返回此值
pub const INVALID_REG_VALUE_ALL_ONE: u32 = 0xFFFFFFFF;

/// RKNPU 硬件配置
#[derive(Debug, Clone, Copy)]
pub struct RknpuConfig {
    /// 带宽优先级寄存器地址
    pub bw_priority_addr: u32,
    /// 带宽优先级寄存器长度
    pub bw_priority_length: u32,
    /// DMA 掩码位数
    pub dma_mask_bits: u32,
    /// PC 数据量缩放比例
    pub pc_data_amount_scale: u32,
    /// PC 任务编号位数
    pub pc_task_number_bits: u32,
    /// PC 任务编号掩码
    pub pc_task_number_mask: u32,
    /// PC 任务状态偏移
    pub pc_task_status_offset: u32,
    /// PC DMA 控制
    pub pc_dma_ctrl: u32,
    /// 带宽使能
    pub bw_enable: bool,
    /// 中断数量
    pub num_irqs: usize,
    /// 复位数量
    pub num_resets: usize,
    /// NBUF 物理地址
    pub nbuf_phyaddr: u64,
    /// NBUF 大小
    pub nbuf_size: u64,
    /// 最大提交数量
    pub max_submit_number: u64,
    /// 核心掩码
    pub core_mask: u32,
}

impl RknpuConfig {
    /// RK3562 配置
    ///
    /// 特性:
    /// - 1 个 NPU 核心
    /// - 40 位 DMA 地址
    /// - NBUF 支持
    pub const RK3562: Self = Self {
        bw_priority_addr: 0x0,
        bw_priority_length: 0x0,
        dma_mask_bits: 40,
        pc_data_amount_scale: 2,
        pc_task_number_bits: 16,
        pc_task_number_mask: 0xffff,
        pc_task_status_offset: 0x48,
        pc_dma_ctrl: 1,
        bw_enable: true,
        num_irqs: 1,
        num_resets: 1,
        nbuf_phyaddr: 0xfe400000,
        nbuf_size: 256 * 1024,
        max_submit_number: (1 << 16) - 1,
        core_mask: 0x1,
    };
    /// RK3568 配置
    ///
    /// 特性:
    /// - 1 个 NPU 核心
    /// - 32 位 DMA 地址
    /// - 支持带宽控制
    pub const RK3568: Self = Self {
        bw_priority_addr: 0xfe180008,
        bw_priority_length: 0x10,
        dma_mask_bits: 32,
        pc_data_amount_scale: 1,
        pc_task_number_bits: 12,
        pc_task_number_mask: 0xfff,
        pc_task_status_offset: 0x3c,
        pc_dma_ctrl: 0,
        bw_enable: true,
        num_irqs: 1,
        num_resets: 1,
        nbuf_phyaddr: 0,
        nbuf_size: 0,
        max_submit_number: (1 << 12) - 1,
        core_mask: 0x1,
    };
    /// RK3583 配置
    ///
    /// 特性:
    /// - 2 个 NPU 核心
    /// - 40 位 DMA 地址
    pub const RK3583: Self = Self {
        bw_priority_addr: 0x0,
        bw_priority_length: 0x0,
        dma_mask_bits: 40,
        pc_data_amount_scale: 2,
        pc_task_number_bits: 12,
        pc_task_number_mask: 0xfff,
        pc_task_status_offset: 0x3c,
        pc_dma_ctrl: 0,
        bw_enable: false,
        num_irqs: 2,
        num_resets: 2,
        nbuf_phyaddr: 0,
        nbuf_size: 0,
        max_submit_number: (1 << 12) - 1,
        core_mask: 0x3,
    };
    /// RK3588 配置
    ///
    /// 特性:
    /// - 3 个 NPU 核心
    /// - 40 位 DMA 地址
    /// - 不支持带宽控制
    pub const RK3588: Self = Self {
        bw_priority_addr: 0x0,
        bw_priority_length: 0x0,
        dma_mask_bits: 40,
        pc_data_amount_scale: 2,
        pc_task_number_bits: 12,
        pc_task_number_mask: 0xfff,
        pc_task_status_offset: 0x3c,
        pc_dma_ctrl: 0,
        bw_enable: false,
        num_irqs: 3,
        num_resets: 3,
        nbuf_phyaddr: 0,
        nbuf_size: 0,
        max_submit_number: (1 << 12) - 1,
        core_mask: 0x7,
    };
    /// RV1106 配置
    ///
    /// 特性:
    /// - 1 个 NPU 核心
    /// - 32 位 DMA 地址
    /// - 16 位任务编号
    pub const RV1106: Self = Self {
        bw_priority_addr: 0x0,
        bw_priority_length: 0x0,
        dma_mask_bits: 32,
        pc_data_amount_scale: 2,
        pc_task_number_bits: 16,
        pc_task_number_mask: 0xffff,
        pc_task_status_offset: 0x3c,
        pc_dma_ctrl: 0,
        bw_enable: true,
        num_irqs: 1,
        num_resets: 1,
        nbuf_phyaddr: 0,
        nbuf_size: 0,
        max_submit_number: (1 << 16) - 1,
        core_mask: 0x1,
    };

    /// 根据板型获取配置
    pub const fn from_board(board: RkBoard) -> Self {
        match board {
            RkBoard::Rk3588 => Self::RK3588,
            RkBoard::Rk3568 => Self::RK3568,
            RkBoard::Rv1106 => Self::RV1106,
            RkBoard::Rk3562 => Self::RK3562,
            RkBoard::Rk3583 => Self::RK3583,
        }
    }

    /// 获取核心数量
    pub const fn num_cores(&self) -> usize {
        match self.core_mask {
            0x7 => 3, // RK3588
            0x3 => 2, // RK3583
            0x1 => 1, // RK3568, RV1106, RK3562
            _ => 0,
        }
    }

    /// 检查核心是否可用
    pub const fn is_core_available(&self, core: usize) -> bool {
        if core >= 3 {
            return false;
        }
        (self.core_mask & (1 << core)) != 0
    }

    /// 获取指定核心的寄存器基地址
    ///
    /// # 参数
    /// - `core`: NPU 核心标识
    ///
    /// # 返回
    /// - `Some(usize)`: 核心的物理基地址
    /// - `None`: 核心不可用或索引越界
    ///
    /// # 示例
    /// ```
    /// let config = RknpuConfig::RK3588;
    /// assert_eq!(config.get_core_base_addr(NpuCore::Npu0), Some(0xFDAB0000));
    /// assert_eq!(config.get_core_base_addr(NpuCore::Npu1), Some(0xFDAC0000));
    /// assert_eq!(config.get_core_base_addr(NpuCore::Npu2), Some(0xFDAD0000));
    /// ```
    pub fn get_core_base_addr(&self, core: NpuCore) -> Option<usize> {
        let index = core.index();
        if !self.is_core_available(index) {
            return None;
        }
        
        match core {
            NpuCore::Npu0 => Some(addresses::NPU0_BASE),
            NpuCore::Npu1 => Some(addresses::NPU1_BASE),
            NpuCore::Npu2 => Some(addresses::NPU2_BASE),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rk3588_config() {
        let config = RknpuConfig::RK3588;
        assert_eq!(config.num_cores(), 3);
        assert_eq!(config.core_mask, 0x7);
        assert!(config.is_core_available(0));
        assert!(config.is_core_available(1));
        assert!(config.is_core_available(2));
    }

    #[test]
    fn test_rk3568_config() {
        let config = RknpuConfig::RK3568;
        assert_eq!(config.num_cores(), 1);
        assert_eq!(config.core_mask, 0x1);
        assert!(config.is_core_available(0));
        assert!(!config.is_core_available(1));
    }
}
