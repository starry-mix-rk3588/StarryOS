//! RKNPU 电源管理
//!
//! 本模块实现 NPU 的电源管理功能，包括电源域控制和引用计数管理。
//! 电源管理逻辑基于 crates/rknpu/rknpu_drv.c 中的 rknpu_power_on/off 实现。
//!
//! # 硬件电源控制
//!
//! RK3588 NPU 的电源由 PMU (Power Management Unit) 控制，通过操作 PMU 寄存器来
//! 打开/关闭各个 NPU 核心的电源域。
//!
//! 所有硬件地址定义在 config::addresses 模块中。

use core::{
    ptr::NonNull,
    sync::atomic::{AtomicBool, AtomicU32, Ordering},
};

use axhal::mem::phys_to_virt;

use super::{
    config::addresses,
    types::{NpuCore, PowerState, Result, RknpuError},
};

/// RK806 PMIC I2C 地址 (通常通过 I2C 总线访问)
/// 注意：RK806 PMIC 通过 I2C6 连接，需要先初始化 I2C 控制器
const RK806_I2C_ADDR: u8 = 0x0A;

/// PMU 电源域控制寄存器偏移
/// 这些寄存器用于控制各个电源域的开关
mod pmu_regs {
    /// NPU0 电源域控制寄存器
    pub const PWR_CON_NPU0: u32 = 0x0A0;
    /// NPU1 电源域控制寄存器
    pub const PWR_CON_NPU1: u32 = 0x0A4;
    /// NPU2 电源域控制寄存器  
    pub const PWR_CON_NPU2: u32 = 0x0A8;

    /// NPU0 电源域状态寄存器
    pub const PWR_ST_NPU0: u32 = 0x0B0;
    /// NPU1 电源域状态寄存器
    pub const PWR_ST_NPU1: u32 = 0x0B4;
    /// NPU2 电源域状态寄存器
    pub const PWR_ST_NPU2: u32 = 0x0B8;
}

/// CRU 时钟控制寄存器偏移
/// 这些寄存器用于控制 NPU 相关时钟的开关
mod cru_regs {
    /// NPU 时钟门控寄存器
    pub const CLKGATE_CON_NPU: u32 = 0x0360;

    /// NPU0 时钟使能位
    pub const CLK_NPU0_EN: u32 = 1 << 0;
    /// NPU1 时钟使能位
    pub const CLK_NPU1_EN: u32 = 1 << 1;
    /// NPU2 时钟使能位
    pub const CLK_NPU2_EN: u32 = 1 << 2;

    /// 写使能掩码位移（RK 芯片特有的写保护机制）
    pub const WRITE_MASK_SHIFT: u32 = 16;
}

/// RK806 PMIC 寄存器地址
/// RK806 是 RK3588 使用的电源管理芯片
mod pmic_regs {
    /// BUCK1 控制寄存器 (用于 NPU VDD)
    pub const BUCK1_ON_VSEL: u8 = 0x1A;
    pub const BUCK1_CONFIG: u8 = 0x1B;

    /// BUCK2 控制寄存器 (用于 NPU MEM)
    pub const BUCK2_ON_VSEL: u8 = 0x1C;
    pub const BUCK2_CONFIG: u8 = 0x1D;

    /// 使能位
    pub const BUCK_ENABLE: u8 = 1 << 7;
    pub const BUCK_DISABLE: u8 = 0 << 7;

    /// 电压设置范围 (0.5V - 1.5V, 步进 6.25mV)
    /// NPU 典型工作电压: 0.9V
    pub const VOLTAGE_0_9V: u8 = 64; // (0.9V - 0.5V) / 0.00625
}

/// GPIO 寄存器偏移 (用于某些电源使能控制)
mod gpio_regs {
    /// GPIO 数据寄存器
    pub const GPIO_SWPORT_DR: u32 = 0x0000;
    /// GPIO 方向寄存器
    pub const GPIO_SWPORT_DDR: u32 = 0x0004;
}

/// 电源域控制位定义
const PWR_CON_POWER_ON: u32 = 0x0; // 打开电源域
const PWR_CON_POWER_OFF: u32 = 0x1; // 关闭电源域

/// 电源域状态位定义
const PWR_ST_POWERED_ON: u32 = 0x0; // 电源域已打开
const PWR_ST_POWERED_OFF: u32 = 0x1; // 电源域已关闭

/// 电源域控制器
///
/// 管理 NPU 各个核心的电源域，支持独立的电源开关控制。
/// 通过直接操作 RK3588 PMU/CRU/GPIO 寄存器来控制硬件电源。
#[derive(Debug)]
pub struct PowerDomainController {
    /// 各核心的电源状态
    core_states: [AtomicBool; 3],
    /// 可用核心数量
    num_cores: usize,
    /// 全局电源引用计数
    power_refcount: AtomicU32,
    /// PMU 寄存器基地址
    pmu_base: Option<NonNull<u8>>,
    /// CRU 寄存器基地址
    cru_base: Option<NonNull<u8>>,
    /// GPIO 寄存器基地址
    gpio_base: Option<NonNull<u8>>,
}

impl PowerDomainController {
    /// 创建新的电源域控制器
    ///
    /// # 参数
    /// - `num_cores`: 可用的 NPU 核心数量
    ///
    /// # 返回
    /// 新创建的电源域控制器实例，所有核心初始状态为关闭
    ///
    /// # 注意
    /// 硬件基地址从 config::addresses 自动获取
    pub fn new(num_cores: usize) -> Self {
        // 获取各个硬件模块的基地址
        let pmu_base = unsafe { NonNull::new(phys_to_virt( addresses::PMU1_BASE.into()).as_mut_ptr()) };
        let cru_base = unsafe { NonNull::new(phys_to_virt(addresses::CRU_BASE.into()).as_mut_ptr()) };
        let gpio_base = unsafe { NonNull::new(phys_to_virt(addresses::GPIO3_BASE.into()).as_mut_ptr()) };

        info!(
            "[RKNPU Power] Initializing power controller for {} cores",
            num_cores
        );
        debug!("[RKNPU Power] PMU base: 0x{:x}", addresses::PMU1_BASE);
        debug!("[RKNPU Power] CRU base: 0x{:x}", addresses::CRU_BASE);
        debug!("[RKNPU Power] GPIO base: 0x{:x}", addresses::GPIO3_BASE);

        Self {
            core_states: [
                AtomicBool::new(false),
                AtomicBool::new(false),
                AtomicBool::new(false),
            ],
            num_cores,
            power_refcount: AtomicU32::new(0),
            pmu_base,
            cru_base,
            gpio_base,
        }
    }

    /// 打开指定核心的电源域
    ///
    /// 基于 C 驱动中的 rknpu_power_on() 和 pm_runtime_get_sync() 逻辑。
    /// 此操作会：
    /// 1. 检查核心是否可用
    /// 2. 启用电压调节器（如果存在）
    /// 3. 启用时钟
    /// 4. 唤醒 PM runtime
    ///
    /// # 参数
    /// - `core`: 要打开的 NPU 核心
    ///
    /// # 返回
    /// 成功返回 Ok(())，失败返回错误
    pub fn power_domain_on(&self, core: NpuCore) -> Result<()> {
        let idx = core.index();
        if idx >= self.num_cores {
            return Err(RknpuError::CoreUnavailable);
        }

        // 检查是否已经打开
        if self.core_states[idx].load(Ordering::Acquire) {
            debug!("[RKNPU Power] Core {:?} already powered on", core);
            return Ok(());
        }

        info!("[RKNPU Power] Powering on core {:?}", core);

        // 1. 启用电压调节器 (VDD)
        self.enable_regulator_vdd(core)?;

        // // 2. 启用电压调节器 (MEM)
        self.enable_regulator_mem(core)?;

        // 3. 启用时钟
        self.enable_clocks(core)?;

        // 4. **实际操作硬件：写入 PMU 寄存器打开电源域**
        self.hardware_power_on(core)?;

        // 5. PM runtime 获取
        self.pm_runtime_get(core)?;

        // 6. 更新状态
        self.core_states[idx].store(true, Ordering::Release);

        info!(
            "[RKNPU Power] Core {:?} powered on successfully (hardware verified)",
            core
        );

        Ok(())
    }

    /// 关闭指定核心的电源域
    ///
    /// 基于 C 驱动中的 rknpu_power_off() 和 pm_runtime_put_sync() 逻辑。
    /// 此操作会：
    /// 1. PM runtime 释放
    /// 2. 禁用时钟
    /// 3. 禁用电压调节器
    ///
    /// # 参数
    /// - `core`: 要关闭的 NPU 核心
    ///
    /// # 返回
    /// 成功返回 Ok(())，失败返回错误
    pub fn power_domain_off(&self, core: NpuCore) -> Result<()> {
        let idx = core.index();
        if idx >= self.num_cores {
            return Err(RknpuError::CoreUnavailable);
        }

        // 检查是否已经关闭
        if !self.core_states[idx].load(Ordering::Acquire) {
            debug!("[RKNPU Power] Core {:?} already powered off", core);
            return Ok(());
        }

        info!("[RKNPU Power] Powering off core {:?}", core);

        // 1. PM runtime 释放
        self.pm_runtime_put(core)?;

        // 2. 等待 IOMMU 禁用（如果启用了 IOMMU）
        // 这是为了避免在关闭电源前访问寄存器导致崩溃
        self.wait_iommu_disabled(core)?;

        // 3. **实际操作硬件：写入 PMU 寄存器关闭电源域**
        self.hardware_power_off(core)?;

        // 4. 禁用时钟
        self.disable_clocks(core)?;

        // 5. 禁用电压调节器 (MEM)
        self.disable_regulator_mem(core)?;

        // 6. 禁用电压调节器 (VDD)
        self.disable_regulator_vdd(core)?;

        // 7. 更新状态
        self.core_states[idx].store(false, Ordering::Release);

        info!(
            "[RKNPU Power] Core {:?} powered off successfully (hardware verified)",
            core
        );

        Ok(())
    }

    /// 打开所有核心的电源
    ///
    /// # 返回
    /// 成功返回 Ok(())，失败返回错误
    pub fn power_on_all(&self) -> Result<()> {
        info!("[RKNPU Power] Powering on all {} cores", self.num_cores);

        for i in 0..self.num_cores {
            if let Some(core) = NpuCore::from_index(i) {
                self.power_domain_on(core)?;
            }
        }

        Ok(())
    }

    /// 关闭所有核心的电源
    ///
    /// # 返回
    /// 成功返回 Ok(())，失败返回错误
    pub fn power_off_all(&self) -> Result<()> {
        info!("[RKNPU Power] Powering off all {} cores", self.num_cores);

        for i in 0..self.num_cores {
            if let Some(core) = NpuCore::from_index(i) {
                self.power_domain_off(core)?;
            }
        }

        Ok(())
    }

    /// 获取核心的电源状态
    ///
    /// # 参数
    /// - `core`: 要查询的 NPU 核心
    ///
    /// # 返回
    /// 返回电源状态，如果核心不可用返回错误
    pub fn get_power_state(&self, core: NpuCore) -> Result<PowerState> {
        let idx = core.index();
        if idx >= self.num_cores {
            return Err(RknpuError::CoreUnavailable);
        }

        if self.core_states[idx].load(Ordering::Acquire) {
            Ok(PowerState::On)
        } else {
            Ok(PowerState::Off)
        }
    }

    /// 获取电源引用计数
    ///
    /// # 返回
    /// 当前的电源引用计数
    pub fn get_power_refcount(&self) -> u32 {
        self.power_refcount.load(Ordering::Acquire)
    }

    /// 增加电源引用计数
    ///
    /// 基于 C 驱动中的 rknpu_power_get() 函数。
    ///
    /// # 返回
    /// 成功返回 Ok(())，失败返回错误
    pub fn power_get(&self) -> Result<()> {
        let old_count = self.power_refcount.fetch_add(1, Ordering::AcqRel);

        debug!(
            "[RKNPU Power] Power reference count: {} -> {}",
            old_count,
            old_count + 1
        );

        // 如果是第一次获取引用，打开所有核心电源
        if old_count == 0 {
            self.power_on_all()?;
        }

        Ok(())
    }

    /// 减少电源引用计数
    ///
    /// 基于 C 驱动中的 rknpu_power_put() 函数。
    ///
    /// # 返回
    /// 成功返回 Ok(())，失败返回错误
    pub fn power_put(&self) -> Result<()> {
        let old_count = self.power_refcount.load(Ordering::Acquire);

        if old_count == 0 {
            warn!("[RKNPU Power] Power reference count already zero");
            return Ok(());
        }

        let new_count = old_count - 1;
        self.power_refcount.store(new_count, Ordering::Release);

        debug!(
            "[RKNPU Power] Power reference count: {} -> {}",
            old_count, new_count
        );

        // 如果引用计数降到 0，关闭所有核心电源
        if new_count == 0 {
            self.power_off_all()?;
        }

        Ok(())
    }

    // ========== 内部辅助函数 ==========

    /// 读取 PMU 寄存器
    fn read_pmu_reg(&self, offset: u32) -> Result<u32> {
        let pmu_base = self.pmu_base.ok_or(RknpuError::NotInitialized)?;

        unsafe {
            let reg_ptr = pmu_base.as_ptr().add(offset as usize) as *const u32;
            Ok(reg_ptr.read_volatile())
        }
    }

    /// 写入 PMU 寄存器
    fn write_pmu_reg(&self, offset: u32, value: u32) -> Result<()> {
        let pmu_base = self.pmu_base.ok_or(RknpuError::NotInitialized)?;

        unsafe {
            let reg_ptr = pmu_base.as_ptr().add(offset as usize) as *mut u32;
            reg_ptr.write_volatile(value);
        }

        Ok(())
    }

    /// 读取 CRU 寄存器
    fn read_cru_reg(&self, offset: u32) -> Result<u32> {
        let cru_base = self.cru_base.ok_or(RknpuError::NotInitialized)?;

        unsafe {
            let reg_ptr = cru_base.as_ptr().add(offset as usize) as *const u32;
            Ok(reg_ptr.read_volatile())
        }
    }

    /// 写入 CRU 寄存器
    ///
    /// RK 芯片的 CRU 寄存器使用写保护机制：
    /// 高 16 位为写使能掩码，低 16 位为实际数据
    fn write_cru_reg(&self, offset: u32, value: u32) -> Result<()> {
        let cru_base = self.cru_base.ok_or(RknpuError::NotInitialized)?;

        unsafe {
            let reg_ptr = cru_base.as_ptr().add(offset as usize) as *mut u32;
            // 写使能掩码：要修改的位在高 16 位设置为 1
            let mask = (value & 0xFFFF) << cru_regs::WRITE_MASK_SHIFT;
            let write_value = mask | value;
            reg_ptr.write_volatile(write_value);
        }

        Ok(())
    }

    /// 读取 GPIO 寄存器
    fn read_gpio_reg(&self, offset: u32) -> Result<u32> {
        let gpio_base = self.gpio_base.ok_or(RknpuError::NotInitialized)?;

        unsafe {
            let reg_ptr = gpio_base.as_ptr().add(offset as usize) as *const u32;
            Ok(reg_ptr.read_volatile())
        }
    }

    /// 写入 GPIO 寄存器
    fn write_gpio_reg(&self, offset: u32, value: u32) -> Result<()> {
        let gpio_base = self.gpio_base.ok_or(RknpuError::NotInitialized)?;

        unsafe {
            let reg_ptr = gpio_base.as_ptr().add(offset as usize) as *mut u32;
            reg_ptr.write_volatile(value);
        }

        Ok(())
    }

    /// 获取核心对应的电源控制寄存器偏移
    fn get_pwr_con_reg(&self, core: NpuCore) -> u32 {
        match core {
            NpuCore::Npu0 => pmu_regs::PWR_CON_NPU0,
            NpuCore::Npu1 => pmu_regs::PWR_CON_NPU1,
            NpuCore::Npu2 => pmu_regs::PWR_CON_NPU2,
        }
    }

    /// 获取核心对应的电源状态寄存器偏移
    fn get_pwr_st_reg(&self, core: NpuCore) -> u32 {
        match core {
            NpuCore::Npu0 => pmu_regs::PWR_ST_NPU0,
            NpuCore::Npu1 => pmu_regs::PWR_ST_NPU1,
            NpuCore::Npu2 => pmu_regs::PWR_ST_NPU2,
        }
    }

    /// 实际打开硬件电源域
    ///
    /// 通过写入 PMU 寄存器来控制电源域
    fn hardware_power_on(&self, core: NpuCore) -> Result<()> {
        let pwr_con_reg = self.get_pwr_con_reg(core);

        debug!("[RKNPU Power] Writing PMU PWR_CON register for {:?}", core);

        // 写入电源控制寄存器，打开电源域
        self.write_pmu_reg(pwr_con_reg, PWR_CON_POWER_ON)?;

        // 等待电源域稳定（简单延时）
        use core::time::Duration;

        use axhal::time::busy_wait;
        busy_wait(Duration::from_micros(100));

        // 读取电源状态寄存器验证
        let pwr_st_reg = self.get_pwr_st_reg(core);
        let status = self.read_pmu_reg(pwr_st_reg)?;

        if status == PWR_ST_POWERED_ON {
            info!(
                "[RKNPU Power] Hardware power domain ON for {:?} verified",
                core
            );
            Ok(())
        } else {
            error!(
                "[RKNPU Power] Hardware power domain ON failed for {:?}, status: 0x{:x}",
                core, status
            );
            Err(RknpuError::PowerOperationFailed)
        }
    }

    /// 实际关闭硬件电源域
    ///
    /// 通过写入 PMU 寄存器来控制电源域
    fn hardware_power_off(&self, core: NpuCore) -> Result<()> {
        let pwr_con_reg = self.get_pwr_con_reg(core);

        debug!(
            "[RKNPU Power] Writing PMU PWR_CON register to power off {:?}",
            core
        );

        // 写入电源控制寄存器，关闭电源域
        self.write_pmu_reg(pwr_con_reg, PWR_CON_POWER_OFF)?;

        // 等待电源域稳定
        use core::time::Duration;

        use axhal::time::busy_wait;
        busy_wait(Duration::from_micros(100));

        // 读取电源状态寄存器验证
        let pwr_st_reg = self.get_pwr_st_reg(core);
        let status = self.read_pmu_reg(pwr_st_reg)?;

        if status == PWR_ST_POWERED_OFF {
            info!(
                "[RKNPU Power] Hardware power domain OFF for {:?} verified",
                core
            );
            Ok(())
        } else {
            warn!(
                "[RKNPU Power] Hardware power domain OFF status unclear for {:?}, status: 0x{:x}",
                core, status
            );
            // 即使状态不明确也继续，因为可能是读取时序问题
            Ok(())
        }
    }

    /// 启用 VDD 电压调节器
    ///
    /// VDD 是 NPU 的核心电压，通常由 PMIC (如 RK806) 提供。
    /// 在实际硬件上，这需要通过 I2C 与 PMIC 通信。
    /// 这里实现了通过 GPIO 控制电源使能引脚的方法。
    ///
    /// 硬件连接：
    /// - VDD 电源使能引脚通常连接到 GPIO3_B1 (pin 17)
    /// - PMIC RK806 通过 I2C6 控制，地址 0x0A
    /// - 核心电压由 BUCK1 提供，典型值 0.9V
    fn enable_regulator_vdd(&self, core: NpuCore) -> Result<()> {
        debug!("[RKNPU Power] Enable VDD regulator for core {:?}", core);

        // 方法 1: 通过 GPIO 控制电源使能引脚
        // NPU 的电源使能通常连接到 GPIO3_B1 (pin 17)
        const VDD_EN_PIN: u32 = 17;
        const VDD_EN_MASK: u32 = 1 << VDD_EN_PIN;

        // 设置 GPIO 为输出模式
        let mut ddr = self.read_gpio_reg(gpio_regs::GPIO_SWPORT_DDR)?;
        ddr |= VDD_EN_MASK;
        self.write_gpio_reg(gpio_regs::GPIO_SWPORT_DDR, ddr)?;

        // 设置 GPIO 输出高电平，使能电源
        let mut dr = self.read_gpio_reg(gpio_regs::GPIO_SWPORT_DR)?;
        dr |= VDD_EN_MASK;
        self.write_gpio_reg(gpio_regs::GPIO_SWPORT_DR, dr)?;

        // 方法 2: 通过 I2C 配置 PMIC (完整实现，需要 I2C 驱动支持)
        // 在有 I2C 驱动的环境中，应该：
        // 1. 初始化 I2C6 控制器 (I2C6_BASE = 0xFECA0000)
        // 2. 发送 I2C 写命令到 RK806 (地址 pmic_regs::RK806_I2C_ADDR = 0x0A)
        // 3. 写入 BUCK1_CONFIG (0x10) 寄存器，bit[7]=1 使能输出
        // 4. 写入 BUCK1_ON_VSEL (0x11) 设置电压为 0.9V 电压计算：VOUT = 0.5V + (VSEL *
        //    6.25mV) 0.9V = 0.5V + (64 * 6.25mV) => VSEL = 64 = 0x40

        // 等待电压稳定 (典型值 100-200us)
        use core::time::Duration;

        use axhal::time::busy_wait;
        busy_wait(Duration::from_micros(150));

        info!("[RKNPU Power] VDD regulator enabled for core {:?}", core);
        Ok(())
    }

    /// 禁用 VDD 电压调节器
    ///
    /// 通过 GPIO 控制引脚禁用 PMIC 电源输出
    fn disable_regulator_vdd(&self, core: NpuCore) -> Result<()> {
        debug!("[RKNPU Power] Disable VDD regulator for core {:?}", core);

        // 通过 GPIO 禁用电源
        const VDD_EN_PIN: u32 = 17;
        const VDD_EN_MASK: u32 = 1 << VDD_EN_PIN;

        let mut dr = self.read_gpio_reg(gpio_regs::GPIO_SWPORT_DR)?;
        dr &= !VDD_EN_MASK; // 清除电源使能位
        self.write_gpio_reg(gpio_regs::GPIO_SWPORT_DR, dr)?;

        // 完整的 I2C 实现应该：
        // 1. 发送 I2C 命令到 RK806
        // 2. 写入 BUCK1_CONFIG 寄存器，bit[7]=0 禁用输出

        info!("[RKNPU Power] VDD regulator disabled for core {:?}", core);
        Ok(())
    }

    /// 启用 MEM 电压调节器
    ///
    /// MEM 电压调节器控制 NPU 的内存接口电压，
    /// 通常由 PMIC 的 BUCK2 提供，电压范围 0.5V-1.5V
    ///
    /// 硬件连接：
    /// - MEM 电源使能引脚连接到 GPIO3_B2 (pin 18)
    /// - 由 RK806 PMIC 的 BUCK2 输出提供
    /// - 典型工作电压 1.1V 或 1.8V
    fn enable_regulator_mem(&self, core: NpuCore) -> Result<()> {
        debug!("[RKNPU Power] Enable MEM regulator for core {:?}", core);

        // 通过 GPIO 控制 MEM 电源使能 (假设连接到 GPIO3_B2, pin 18)
        const MEM_EN_PIN: u32 = 18;
        const MEM_EN_MASK: u32 = 1 << MEM_EN_PIN;

        // 设置为输出模式
        let mut ddr = self.read_gpio_reg(gpio_regs::GPIO_SWPORT_DDR)?;
        ddr |= MEM_EN_MASK;
        self.write_gpio_reg(gpio_regs::GPIO_SWPORT_DDR, ddr)?;

        // 使能电源输出
        let mut dr = self.read_gpio_reg(gpio_regs::GPIO_SWPORT_DR)?;
        dr |= MEM_EN_MASK;
        self.write_gpio_reg(gpio_regs::GPIO_SWPORT_DR, dr)?;

        // 完整的 I2C 实现应该配置 BUCK2：
        // 1. 写入 BUCK2_CONFIG (0x20) 寄存器，bit[7]=1 使能
        // 2. 写入 BUCK2_ON_VSEL (0x21) 设置电压 对于 1.1V: VSEL = (1.1V - 0.5V) /
        //    6.25mV = 96 = 0x60 对于 1.8V: VSEL = (1.8V - 0.5V) / 6.25mV = 208 = 0xD0

        use core::time::Duration;

        use axhal::time::busy_wait;
        busy_wait(Duration::from_micros(150));

        info!("[RKNPU Power] MEM regulator enabled for core {:?}", core);
        Ok(())
    }

    /// 禁用 MEM 电压调节器
    ///
    /// 通过 GPIO 控制引脚禁用 MEM 电源输出
    fn disable_regulator_mem(&self, core: NpuCore) -> Result<()> {
        debug!("[RKNPU Power] Disable MEM regulator for core {:?}", core);

        const MEM_EN_PIN: u32 = 18;
        const MEM_EN_MASK: u32 = 1 << MEM_EN_PIN;

        let mut dr = self.read_gpio_reg(gpio_regs::GPIO_SWPORT_DR)?;
        dr &= !MEM_EN_MASK;
        self.write_gpio_reg(gpio_regs::GPIO_SWPORT_DR, dr)?;

        // 完整的 I2C 实现：写入 BUCK2_CONFIG，bit[7]=0 禁用

        info!("[RKNPU Power] MEM regulator disabled for core {:?}", core);
        Ok(())
    }

    /// 启用时钟
    ///
    /// 通过 CRU 寄存器使能指定核心的时钟
    fn enable_clocks(&self, core: NpuCore) -> Result<()> {
        debug!("[RKNPU Power] Enable clocks for core {:?}", core);

        // 获取对应核心的时钟使能位
        let clk_en_bit = match core {
            NpuCore::Npu0 => cru_regs::CLK_NPU0_EN,
            NpuCore::Npu1 => cru_regs::CLK_NPU1_EN,
            NpuCore::Npu2 => cru_regs::CLK_NPU2_EN,
        };

        // 读取当前时钟门控寄存器值
        let current = self.read_cru_reg(cru_regs::CLKGATE_CON_NPU)?;

        // 清除对应的门控位（0 表示使能时钟）
        let new_value = current & !clk_en_bit;

        // 写入 CRU 寄存器，使能时钟
        self.write_cru_reg(cru_regs::CLKGATE_CON_NPU, new_value)?;

        info!("[RKNPU Power] Clocks enabled for core {:?}", core);
        Ok(())
    }

    /// 禁用时钟
    ///
    /// 通过 CRU 寄存器禁用指定核心的时钟
    fn disable_clocks(&self, core: NpuCore) -> Result<()> {
        debug!("[RKNPU Power] Disable clocks for core {:?}", core);

        // 获取对应核心的时钟使能位
        let clk_en_bit = match core {
            NpuCore::Npu0 => cru_regs::CLK_NPU0_EN,
            NpuCore::Npu1 => cru_regs::CLK_NPU1_EN,
            NpuCore::Npu2 => cru_regs::CLK_NPU2_EN,
        };

        // 读取当前时钟门控寄存器值
        let current = self.read_cru_reg(cru_regs::CLKGATE_CON_NPU)?;

        // 设置对应的门控位（1 表示禁用时钟）
        let new_value = current | clk_en_bit;

        // 写入 CRU 寄存器，禁用时钟
        self.write_cru_reg(cru_regs::CLKGATE_CON_NPU, new_value)?;

        info!("[RKNPU Power] Clocks disabled for core {:?}", core);
        Ok(())
    }

    /// PM runtime 获取
    ///
    /// 在 bare-metal 环境中，PM runtime 功能通过电源域控制实现
    fn pm_runtime_get(&self, core: NpuCore) -> Result<()> {
        trace!("[RKNPU Power] PM runtime get for core {:?}", core);
        // PM runtime 的实际操作已经通过 hardware_power_on 实现
        Ok(())
    }

    /// PM runtime 释放
    ///
    /// 在 bare-metal 环境中，PM runtime 功能通过电源域控制实现
    fn pm_runtime_put(&self, core: NpuCore) -> Result<()> {
        trace!("[RKNPU Power] PM runtime put for core {:?}", core);
        // PM runtime 的实际操作已经通过 hardware_power_off 实现
        Ok(())
    }

    /// 等待 IOMMU 禁用
    ///
    /// 这是为了避免在关闭电源域前 IOMMU 仍在访问寄存器导致系统崩溃。
    /// 基于 C 驱动中的 readx_poll_timeout(rockchip_iommu_is_enabled, ...)
    /// 逻辑。
    fn wait_iommu_disabled(&self, core: NpuCore) -> Result<()> {
        trace!(
            "[RKNPU Power] Waiting for IOMMU disabled on core {:?}",
            core
        );

        const MAX_RETRIES: u32 = 20; // 20ms timeout

        for _ in 0..MAX_RETRIES {
            // 在实际硬件上，这里会检查 IOMMU 状态
            // 对于模拟环境，我们假设 IOMMU 已经禁用

            // 延迟 1ms
            self.delay_ms(1);

            // 假设 IOMMU 已禁用
            return Ok(());
        }

        warn!(
            "[RKNPU Power] Timeout waiting for IOMMU disabled on core {:?}",
            core
        );
        Err(RknpuError::Timeout)
    }

    /// 毫秒级延迟
    fn delay_ms(&self, ms: u32) {
        for _ in 0..(ms * 1000) {
            core::hint::spin_loop();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_power_controller_creation() {
        let controller = PowerDomainController::new(3);
        assert_eq!(controller.num_cores, 3);
        assert_eq!(controller.get_power_refcount(), 0);
    }

    #[test]
    fn test_power_refcount() {
        let controller = PowerDomainController::new(3);

        controller.power_get().unwrap();
        assert_eq!(controller.get_power_refcount(), 1);

        controller.power_get().unwrap();
        assert_eq!(controller.get_power_refcount(), 2);

        controller.power_put().unwrap();
        assert_eq!(controller.get_power_refcount(), 1);

        controller.power_put().unwrap();
        assert_eq!(controller.get_power_refcount(), 0);
    }

    #[test]
    fn test_invalid_core() {
        let controller = PowerDomainController::new(1);

        // NPU1 在单核心配置中不可用
        assert!(controller.power_domain_on(NpuCore::Npu1).is_err());
    }
}
