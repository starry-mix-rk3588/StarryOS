//! RKNPU 复位管理
//!
//! 本模块实现 NPU 的复位功能，包括软复位和单个核心复位。
//! 复位逻辑基于 crates/rknpu/rknpu_reset.c 中的实现。
//!
//! # 硬件复位控制
//!
//! RK3588 NPU 的复位由 CRU (Clock & Reset Unit) 控制，通过操作 CRU 寄存器来
//! 执行 AXI 和 AHB 总线复位。
//!
//! 所有硬件地址定义在 config::addresses 模块中。

use core::ptr::NonNull;

use super::{
    config::addresses,
    types::{NpuCore, ResetType, Result, RknpuError},
};

/// CRU 软复位控制寄存器偏移
mod cru_softrst_regs {
    /// NPU 软复位控制寄存器
    pub const SOFTRST_CON_NPU: u32 = 0x0A00;

    /// NPU0 AXI 复位位
    pub const NPU0_AXI_SRST: u32 = 1 << 0;
    /// NPU0 AHB 复位位
    pub const NPU0_AHB_SRST: u32 = 1 << 1;

    /// NPU1 AXI 复位位
    pub const NPU1_AXI_SRST: u32 = 1 << 2;
    /// NPU1 AHB 复位位
    pub const NPU1_AHB_SRST: u32 = 1 << 3;

    /// NPU2 AXI 复位位
    pub const NPU2_AXI_SRST: u32 = 1 << 4;
    /// NPU2 AHB 复位位
    pub const NPU2_AHB_SRST: u32 = 1 << 5;

    /// 写使能掩码位移（RK 芯片特有的写保护机制）
    pub const WRITE_MASK_SHIFT: u32 = 16;
}

/// 复位控制器
///
/// 管理 NPU 的复位操作，通过直接操作 RK3588 CRU 寄存器来执行硬件复位。
#[derive(Debug)]
pub struct ResetController {
    /// 复位基地址数组（每个核心一个 NPU 寄存器基地址）
    base_addrs: [Option<NonNull<u8>>; 3],
    /// 可用核心数量
    num_cores: usize,
    /// 是否跳过软复位
    bypass_soft_reset: bool,
    /// CRU 寄存器基地址
    cru_base: Option<NonNull<u8>>,
}

impl ResetController {
    /// 创建新的复位控制器
    ///
    /// # 参数
    /// - `num_cores`: 可用的 NPU 核心数量
    ///
    /// # 返回
    /// 新创建的复位控制器实例
    ///
    /// # 注意
    /// CRU 基地址从 config::addresses 自动获取
    pub fn new(num_cores: usize) -> Self {
        // 获取 CRU 基地址
        let cru_base = unsafe { NonNull::new(addresses::CRU_BASE as *mut u8) };

        info!("[RKNPU Reset] Initializing reset controller for {} cores", num_cores);
        debug!("[RKNPU Reset] CRU base: 0x{:x}", addresses::CRU_BASE);

        Self {
            base_addrs: [None; 3],
            num_cores,
            bypass_soft_reset: false,
            cru_base,
        }
    }

    /// 设置核心的复位基地址
    ///
    /// # 参数
    /// - `core`: NPU 核心标识
    /// - `base_addr`: 该核心的寄存器基地址
    ///
    /// # 返回
    /// 成功返回 Ok(())，失败返回错误
    pub fn set_core_base(&mut self, core: NpuCore, base_addr: NonNull<u8>) -> Result<()> {
        let idx = core.index();
        if idx >= self.num_cores {
            return Err(RknpuError::CoreUnavailable);
        }
        self.base_addrs[idx] = Some(base_addr);
        Ok(())
    }

    /// 设置是否跳过软复位
    ///
    /// # 参数
    /// - `bypass`: true 表示跳过软复位操作
    pub fn set_bypass_soft_reset(&mut self, bypass: bool) {
        self.bypass_soft_reset = bypass;
    }

    /// 执行软复位（所有核心）
    ///
    /// 软复位会重置 NPU 的状态，包括清除中断、重置寄存器等。
    /// 基于 C 驱动中的 rknpu_soft_reset() 函数。
    ///
    /// # 返回
    /// 成功返回 Ok(())，失败返回错误
    pub fn soft_reset(&self) -> Result<()> {
        if self.bypass_soft_reset {
            info!("[RKNPU Reset] Bypass soft reset");
            return Ok(());
        }

        info!("[RKNPU Reset] Performing soft reset on all cores");

        for i in 0..self.num_cores {
            if let Some(core) = NpuCore::from_index(i) {
                self.soft_reset_core(core)?;
            }
        }

        Ok(())
    }

    /// 执行指定核心的软复位
    ///
    /// # 参数
    /// - `core`: 要复位的 NPU 核心
    ///
    /// # 返回
    /// 成功返回 Ok(())，失败返回错误
    pub fn soft_reset_core(&self, core: NpuCore) -> Result<()> {
        if self.bypass_soft_reset {
            debug!("[RKNPU Reset] Bypass soft reset for core {:?}", core);
            return Ok(());
        }

        let idx = core.index();
        if idx >= self.num_cores {
            return Err(RknpuError::CoreUnavailable);
        }

        let base_addr = self.base_addrs[idx].ok_or(RknpuError::NotInitialized)?;

        info!("[RKNPU Reset] Soft reset core {:?}", core);

        // 1. 清除中断
        self.clear_interrupts(base_addr)?;

        // 2. 禁用所有使能位
        self.disable_all_enables(base_addr)?;

        // 3. 执行 AXI 复位
        self.reset_axi(core)?;

        // 4. 执行 AHB 复位
        self.reset_ahb(core)?;

        // 5. 等待复位完成
        self.wait_reset_complete(base_addr)?;

        debug!("[RKNPU Reset] Core {:?} reset completed", core);

        Ok(())
    }

    /// 读取 CRU 寄存器
    ///
    /// # 参数
    /// - `offset`: 寄存器偏移量
    ///
    /// # 返回
    /// 返回寄存器的值
    fn read_cru_reg(&self, offset: usize) -> u32 {
        unsafe {
            let cru_base = self.cru_base.expect("CRU base not initialized");
            let reg_ptr = cru_base.as_ptr().add(offset) as *const u32;
            reg_ptr.read_volatile()
        }
    }

    /// 写入 CRU 寄存器
    ///
    /// # 参数
    /// - `offset`: 寄存器偏移量
    /// - `value`: 要写入的值
    ///
    /// # 说明
    /// RK 芯片的寄存器写保护机制：
    /// - 寄存器的高 16 位作为写使能掩码 (write mask)
    /// - 只有当对应位的掩码为 1 时，低 16 位的对应位才会被写入
    /// - 例如：要设置第 3 位，需要写入 (1 << (3 + 16)) | (1 << 3)
    fn write_cru_reg(&self, offset: usize, value: u32) {
        unsafe {
            let cru_base = self.cru_base.expect("CRU base not initialized");
            let reg_ptr = cru_base.as_ptr().add(offset) as *mut u32;
            reg_ptr.write_volatile(value);
        }
    }

    /// 清除中断状态
    fn clear_interrupts(&self, base_addr: NonNull<u8>) -> Result<()> {
        use super::config::registers;

        unsafe {
            let int_clear_ptr = base_addr.as_ptr().add(registers::INT_CLEAR as usize) as *mut u32;
            int_clear_ptr.write_volatile(super::config::INT_CLEAR_VALUE);
        }

        Ok(())
    }

    /// 禁用所有使能位
    fn disable_all_enables(&self, base_addr: NonNull<u8>) -> Result<()> {
        use super::config::registers;

        unsafe {
            // 禁用 PC 操作
            let pc_op_en_ptr = base_addr.as_ptr().add(registers::PC_OP_EN as usize) as *mut u32;
            pc_op_en_ptr.write_volatile(0);

            // 清除使能掩码
            let enable_mask_ptr =
                base_addr.as_ptr().add(registers::ENABLE_MASK as usize) as *mut u32;
            enable_mask_ptr.write_volatile(0);
        }

        Ok(())
    }

    /// 执行 AXI 总线复位
    ///
    /// AXI 复位会重置 NPU 的 AXI 总线接口。
    ///
    /// # 硬件操作流程
    /// 1. 向 CRU SOFTRST_CON 寄存器写入复位位（设置为 1）
    /// 2. 等待一段时间确保复位生效
    /// 3. 清除复位位（设置为 0）恢复正常工作
    fn reset_axi(&self, core: NpuCore) -> Result<()> {
        trace!("[RKNPU Reset] AXI reset for core {:?}", core);

        // 确定对应核心的 AXI 复位位
        let reset_bit = match core {
            NpuCore::Npu0 => cru_softrst_regs::NPU0_AXI_SRST,
            NpuCore::Npu1 => cru_softrst_regs::NPU1_AXI_SRST,
            NpuCore::Npu2 => cru_softrst_regs::NPU2_AXI_SRST,
        };

        // RK 芯片的写保护机制：高 16 位为写使能掩码
        // 要设置某位，需要：(1 << (bit + 16)) | (1 << bit)
        // 要清除某位，需要：(1 << (bit + 16)) | (0 << bit)

        // 步骤 1: 置位 - 触发复位
        let set_value = (1 << (reset_bit + 16)) | (1 << reset_bit);
        self.write_cru_reg(cru_softrst_regs::SOFTRST_CON_NPU as usize, set_value);

        debug!(
            "[RKNPU Reset] AXI reset asserted for core {:?} (bit {})",
            core, reset_bit
        );

        // 步骤 2: 等待复位生效（至少 10us）
        self.delay_us(10);

        // 步骤 3: 清零 - 释放复位
        let clear_value = (1 << (reset_bit + 16)) | (0 << reset_bit);
        self.write_cru_reg(cru_softrst_regs::SOFTRST_CON_NPU as usize, clear_value);

        debug!("[RKNPU Reset] AXI reset deasserted for core {:?}", core);

        // 步骤 4: 等待稳定
        self.delay_us(5);

        Ok(())
    }

    /// 执行 AHB 总线复位
    ///
    /// AHB 复位会重置 NPU 的 AHB 总线接口。
    ///
    /// # 硬件操作流程
    /// 1. 向 CRU SOFTRST_CON 寄存器写入复位位（设置为 1）
    /// 2. 等待一段时间确保复位生效
    /// 3. 清除复位位（设置为 0）恢复正常工作
    fn reset_ahb(&self, core: NpuCore) -> Result<()> {
        trace!("[RKNPU Reset] AHB reset for core {:?}", core);

        // 确定对应核心的 AHB 复位位
        let reset_bit = match core {
            NpuCore::Npu0 => cru_softrst_regs::NPU0_AHB_SRST,
            NpuCore::Npu1 => cru_softrst_regs::NPU1_AHB_SRST,
            NpuCore::Npu2 => cru_softrst_regs::NPU2_AHB_SRST,
        };

        // RK 芯片的写保护机制：高 16 位为写使能掩码

        // 步骤 1: 置位 - 触发复位
        let set_value = (1 << (reset_bit + 16)) | (1 << reset_bit);
        self.write_cru_reg(cru_softrst_regs::SOFTRST_CON_NPU as usize, set_value);

        debug!(
            "[RKNPU Reset] AHB reset asserted for core {:?} (bit {})",
            core, reset_bit
        );

        // 步骤 2: 等待复位生效（至少 10us）
        self.delay_us(10);

        // 步骤 3: 清零 - 释放复位
        let clear_value = (1 << (reset_bit + 16)) | (0 << reset_bit);
        self.write_cru_reg(cru_softrst_regs::SOFTRST_CON_NPU as usize, clear_value);

        debug!("[RKNPU Reset] AHB reset deasserted for core {:?}", core);

        // 步骤 4: 等待稳定
        self.delay_us(5);

        Ok(())
    }

    /// 等待复位完成
    ///
    /// 通过轮询状态寄存器确认复位操作已完成。
    fn wait_reset_complete(&self, base_addr: NonNull<u8>) -> Result<()> {
        use super::config::registers;

        const MAX_RETRIES: u32 = 1000;

        for _ in 0..MAX_RETRIES {
            unsafe {
                let status_ptr =
                    base_addr.as_ptr().add(registers::INT_STATUS as usize) as *const u32;
                let status = status_ptr.read_volatile();

                // 如果状态寄存器为 0，表示复位完成
                if status == 0 {
                    return Ok(());
                }
            }

            self.delay_us(1);
        }

        Err(RknpuError::Timeout)
    }

    /// 微秒级延迟
    ///
    /// # 参数
    /// - `us`: 延迟的微秒数
    fn delay_us(&self, us: u32) {
        // 简单的忙等待实现
        // 在实际系统中应该使用更精确的定时器
        for _ in 0..(us * 100) {
            core::hint::spin_loop();
        }
    }

    /// 执行指定类型的复位
    ///
    /// # 参数
    /// - `core`: 要复位的核心
    /// - `reset_type`: 复位类型
    ///
    /// # 返回
    /// 成功返回 Ok(())，失败返回错误
    pub fn reset(&self, core: NpuCore, reset_type: ResetType) -> Result<()> {
        match reset_type {
            ResetType::Soft => self.soft_reset_core(core),
            ResetType::Axi => self.reset_axi(core),
            ResetType::Ahb => self.reset_ahb(core),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_reset_controller_creation() {
        let controller = ResetController::new(3);
        assert_eq!(controller.num_cores, 3);
        assert!(!controller.bypass_soft_reset);
    }

    #[test]
    fn test_set_bypass() {
        let mut controller = ResetController::new(1);
        controller.set_bypass_soft_reset(true);
        assert!(controller.bypass_soft_reset);
    }
}
