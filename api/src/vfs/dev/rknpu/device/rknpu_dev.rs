//! RKNPU 设备核心实现
//!
//! 本模块实现 RKNPU 设备的核心结构和方法。
//! 提供统一的接口来管理 NPU 硬件，包括初始化、电源管理、复位等功能。

use core::ptr::NonNull;

use super::{
    config::RknpuConfig,
    power::PowerDomainController,
    reset::ResetController,
    types::{NpuCore, PowerState, ResetType, Result, RkBoard, RknpuError},
};

/// RK3588 NPU 设备
///
/// 这是 RK3588 NPU 的主要抽象结构，提供了完整的硬件控制接口。
///
/// # 示例
///
/// ```no_run
/// use core::ptr::NonNull;
///
/// use rknpu_device::{NPU1, NPU2, RK3588NPU, RkBoard};
///
/// // 初始化 NPU (基地址需要从设备树获取)
/// let npu_base = unsafe { NonNull::new_unchecked(0xfd8d8000 as *mut u8) };
/// let mut rknpu = RK3588NPU::new(npu_base, RkBoard::Rk3588);
///
/// // 单独控制电源域
/// rknpu.power_domain_on(NPU1).unwrap();
/// rknpu.power_domain_off(NPU2).unwrap();
///
/// // 软复位
/// rknpu.soft_reset().unwrap();
/// ```
#[derive(Debug)]
pub struct RK3588NPU {
    /// 各核心的寄存器基地址
    base_addrs: [Option<NonNull<u8>>; 3],
    /// 硬件配置
    config: RknpuConfig,
    /// 电源域控制器
    power_controller: PowerDomainController,
    /// 复位控制器
    reset_controller: ResetController,
    /// 板型
    board: RkBoard,
    /// 是否已初始化
    initialized: bool,
}

impl RK3588NPU {
    /// 创建新的 RKNPU 设备实例
    ///
    /// # 参数
    /// - `base_addr`: NPU 第一个核心的寄存器基地址（来自设备树）
    /// - `board`: 板型标识
    ///
    /// # 返回
    /// 新创建的 RKNPU 设备实例
    ///
    /// # 注意
    /// 此函数只创建设备结构，不会执行硬件初始化。
    /// 需要调用 `init()` 方法来完成初始化。
    pub fn new(base_addr: NonNull<u8>, board: RkBoard) -> Self {
        let config = RknpuConfig::from_board(board);
        let num_cores = config.num_cores();

        info!(
            "[RKNPU] Creating device for {:?} with {} cores at 0x{:x}",
            board,
            num_cores,
            base_addr.as_ptr() as usize
        );

        let mut base_addrs = [None; 3];
        base_addrs[0] = Some(base_addr);

        // 为多核心配置计算其他核心的基地址
        // 基于 RK3588 的地址布局，每个核心间隔 0x10000
        if num_cores > 1 {
            for i in 1..num_cores {
                let offset = (i * 0x10000) as isize;
                let core_base =
                    unsafe { NonNull::new_unchecked(base_addr.as_ptr().offset(offset)) };
                base_addrs[i] = Some(core_base);

                debug!(
                    "[RKNPU] Core {} base address: 0x{:x}",
                    i,
                    core_base.as_ptr() as usize
                );
            }
        }

        let mut reset_controller = ResetController::new(num_cores);
        for i in 0..num_cores {
            if let Some(addr) = base_addrs[i] {
                if let Some(core) = NpuCore::from_index(i) {
                    let _ = reset_controller.set_core_base(core, addr);
                }
            }
        }

        Self {
            base_addrs,
            config,
            power_controller: PowerDomainController::new(num_cores),
            reset_controller,
            board,
            initialized: false,
        }
    }

    /// 初始化 NPU 设备
    ///
    /// 执行必要的硬件初始化操作，包括：
    /// 1. 检查硬件版本
    /// 2. 配置寄存器
    /// 3. 清除中断状态
    ///
    /// # 返回
    /// 成功返回 Ok(())，失败返回错误
    pub fn init(&mut self) -> Result<()> {
        if self.initialized {
            warn!("[RKNPU] Device already initialized");
            return Ok(());
        }

        info!("[RKNPU] Initializing device");

        // 1. 读取并验证硬件版本
        self.check_hardware_version()?;

        // 2. 清除所有核心的中断状态
        for i in 0..self.config.num_cores() {
            if let Some(core) = NpuCore::from_index(i) {
                self.clear_interrupts(core)?;
            }
        }

        // 3. 标记为已初始化
        self.initialized = true;

        info!("[RKNPU] Device initialized successfully");

        Ok(())
    }

    /// 打开指定核心的电源域
    ///
    /// 该方法会执行电源打开操作，并自动验证电源是否正确打开。
    /// 验证通过读取版本寄存器确保硬件可访问。
    ///
    /// # 参数
    /// - `core`: 要打开的 NPU 核心
    ///
    /// # 返回
    /// 成功返回 Ok(())，失败返回错误
    pub fn power_domain_on(&mut self, core: NpuCore) -> Result<()> {
        use super::config::POWER_ON_VERIFY_TIMEOUT_US;

        // 执行电源打开操作
        self.power_controller.power_domain_on(core)?;

        // 验证电源是否正确打开
        self.verify_power_on_with_timeout(core, POWER_ON_VERIFY_TIMEOUT_US)?;

        info!("[RKNPU] Core {:?} power domain ON and verified", core);
        Ok(())
    }

    /// 关闭指定核心的电源域
    ///
    /// # 参数
    /// - `core`: 要关闭的 NPU 核心
    ///
    /// # 返回
    /// 成功返回 Ok(())，失败返回错误
    #[inline]
    pub fn power_domain_off(&mut self, core: NpuCore) -> Result<()> {
        self.power_controller.power_domain_off(core)
    }

    /// 打开所有核心的电源
    ///
    /// # 返回
    /// 成功返回 Ok(())，失败返回错误
    #[inline]
    pub fn power_on(&mut self) -> Result<()> {
        self.power_controller.power_on_all()
    }

    /// 关闭所有核心的电源
    ///
    /// # 返回
    /// 成功返回 Ok(())，失败返回错误
    #[inline]
    pub fn power_off(&mut self) -> Result<()> {
        self.power_controller.power_off_all()
    }

    /// 获取核心的电源状态
    ///
    /// # 参数
    /// - `core`: 要查询的 NPU 核心
    ///
    /// # 返回
    /// 返回电源状态
    #[inline]
    pub fn get_power_state(&self, core: NpuCore) -> Result<PowerState> {
        self.power_controller.get_power_state(core)
    }

    /// 执行软复位（所有核心）
    ///
    /// # 返回
    /// 成功返回 Ok(())，失败返回错误
    #[inline]
    pub fn soft_reset(&mut self) -> Result<()> {
        self.reset_controller.soft_reset()
    }

    /// 执行指定核心的软复位
    ///
    /// # 参数
    /// - `core`: 要复位的 NPU 核心
    ///
    /// # 返回
    /// 成功返回 Ok(())，失败返回错误
    #[inline]
    pub fn soft_reset_core(&mut self, core: NpuCore) -> Result<()> {
        self.reset_controller.soft_reset_core(core)
    }

    /// 执行指定类型的复位
    ///
    /// # 参数
    /// - `core`: 要复位的核心
    /// - `reset_type`: 复位类型
    ///
    /// # 返回
    /// 成功返回 Ok(())，失败返回错误
    #[inline]
    pub fn reset(&mut self, core: NpuCore, reset_type: ResetType) -> Result<()> {
        self.reset_controller.reset(core, reset_type)
    }

    /// 设置是否跳过软复位
    ///
    /// # 参数
    /// - `bypass`: true 表示跳过软复位操作
    #[inline]
    pub fn set_bypass_soft_reset(&mut self, bypass: bool) {
        self.reset_controller.set_bypass_soft_reset(bypass);
    }

    /// 读取寄存器
    ///
    /// # 参数
    /// - `core`: NPU 核心
    /// - `offset`: 寄存器偏移量
    ///
    /// # 返回
    /// 寄存器值
    pub fn read_reg(&self, core: NpuCore, offset: u32) -> Result<u32> {
        let idx = core.index();
        let base_addr = self.base_addrs[idx].ok_or(RknpuError::NotInitialized)?;

        unsafe {
            let reg_ptr = base_addr.as_ptr().add(offset as usize) as *const u32;
            Ok(reg_ptr.read_volatile())
        }
    }

    /// 写入寄存器
    ///
    /// # 参数
    /// - `core`: NPU 核心
    /// - `offset`: 寄存器偏移量
    /// - `value`: 要写入的值
    pub fn write_reg(&mut self, core: NpuCore, offset: u32, value: u32) -> Result<()> {
        let idx = core.index();
        let base_addr = self.base_addrs[idx].ok_or(RknpuError::NotInitialized)?;

        unsafe {
            let reg_ptr = base_addr.as_ptr().add(offset as usize) as *mut u32;
            reg_ptr.write_volatile(value);
        }

        Ok(())
    }

    /// 读取版本寄存器
    ///
    /// 读取指定核心的版本寄存器，用于验证硬件是否可访问。
    /// 对于 RK3588，预期返回值为 0x60002 (NPU v6.0.2)。
    ///
    /// # 参数
    /// - `core`: NPU 核心
    ///
    /// # 返回
    /// 版本寄存器值
    ///
    /// # 示例
    /// ```no_run
    /// let version = device.read_version_reg(NpuCore::NPU0)?;
    /// if version == config::RK3588_NPU_VERSION {
    ///     println!("RK3588 NPU detected, version: {:#x}", version);
    /// }
    /// ```
    pub fn read_version_reg(&self, core: NpuCore) -> Result<u32> {
        use super::config::registers;
        self.read_reg(core, registers::VERSION)
    }

    /// 验证电源是否正确打开 (带超时轮询)
    ///
    /// 通过轮询方式读取版本寄存器，确保电源稳定后硬件可访问。
    /// 该方法会在指定的超时时间内反复尝试验证硬件可访问性。
    ///
    /// # 参数
    /// - `core`: 要验证的 NPU 核心
    /// - `timeout_us`: 超时时间(微秒)
    ///
    /// # 返回
    /// - `Ok(())`: 电源已正确打开，硬件可访问
    /// - `Err(RknpuError::Timeout)`: 超时未能验证
    /// - `Err(RknpuError::HardwareNotReady)`: 硬件不可访问
    ///
    /// # 示例
    /// ```no_run
    /// use config::POWER_ON_VERIFY_TIMEOUT_US;
    ///
    /// device.verify_power_on_with_timeout(NpuCore::NPU0, POWER_ON_VERIFY_TIMEOUT_US)?;
    /// println!("Power verified successfully");
    /// ```
    pub fn verify_power_on_with_timeout(&self, core: NpuCore, timeout_us: u32) -> Result<()> {
        use core::time::Duration;

        use axhal::time::{busy_wait, current_ticks};

        use super::config::POWER_ON_VERIFY_POLL_INTERVAL_US;

        let start_time = current_ticks();
        let timeout_ns = (timeout_us as u64) * 1000;

        debug!(
            "[RKNPU] Verifying power for core {:?} with timeout {}us",
            core, timeout_us
        );

        loop {
            // 尝试验证硬件可访问性
            match self.verify_hardware_accessible(core) {
                Ok(true) => {
                    let elapsed_us = (current_ticks() - start_time) / 1000;
                    info!(
                        "[RKNPU] Core {:?} power verified successfully after {}us",
                        core, elapsed_us
                    );
                    return Ok(());
                }
                Ok(false) => {
                    // 版本不匹配，但硬件可访问，认为电源已打开
                    warn!(
                        "[RKNPU] Core {:?} hardware accessible but version mismatch",
                        core
                    );
                    return Ok(());
                }
                Err(RknpuError::HardwareNotReady) => {
                    // 硬件未就绪，检查是否超时
                    let elapsed_ns = current_ticks() - start_time;
                    if elapsed_ns >= timeout_ns {
                        error!(
                            "[RKNPU] Core {:?} power verification timeout after {}us",
                            core, timeout_us
                        );
                        return Err(RknpuError::Timeout);
                    }

                    // 延时后继续轮询
                    busy_wait(Duration::from_micros(
                        POWER_ON_VERIFY_POLL_INTERVAL_US as u64,
                    ));
                }
                Err(e) => {
                    // 其他错误直接返回
                    error!("[RKNPU] Core {:?} power verification failed: {}", core, e);
                    return Err(e);
                }
            }
        }
    }

    /// 查询指定核心的电源状态
    ///
    /// 通过尝试读取版本寄存器来判断核心电源是否打开。
    /// 此方法不会阻塞，立即返回当前状态。
    ///
    /// # 参数
    /// - `core`: NPU 核心
    ///
    /// # 返回
    /// - `true`: 核心电源已打开且硬件可访问
    /// - `false`: 核心电源关闭或硬件不可访问
    ///
    /// # 示例
    /// ```no_run
    /// if device.is_core_power_on(NpuCore::NPU0) {
    ///     println!("NPU0 is powered on");
    /// } else {
    ///     println!("NPU0 is powered off or inaccessible");
    /// }
    /// ```
    pub fn is_core_power_on(&self, core: NpuCore) -> bool {
        match self.verify_hardware_accessible(core) {
            Ok(_) => {
                debug!("[RKNPU] Core {:?} power status: ON", core);
                true
            }
            Err(e) => {
                debug!("[RKNPU] Core {:?} power status: OFF ({})", core, e);
                false
            }
        }
    }

    /// 获取硬件配置
    #[inline]
    pub fn config(&self) -> &RknpuConfig {
        &self.config
    }

    /// 获取板型
    #[inline]
    pub fn board(&self) -> RkBoard {
        self.board
    }

    /// 获取核心数量
    #[inline]
    pub fn num_cores(&self) -> usize {
        self.config.num_cores()
    }

    /// 检查核心是否可用
    #[inline]
    pub fn is_core_available(&self, core: NpuCore) -> bool {
        self.config.is_core_available(core.index())
    }

    /// 获取核心基地址
    pub fn get_core_base_addr(&self, core: NpuCore) -> Result<NonNull<u8>> {
        let idx = core.index();
        self.base_addrs[idx].ok_or(RknpuError::NotInitialized)
    }

    // ========== 内部辅助函数 ==========

    /// 验证硬件是否可访问
    ///
    /// 通过读取版本寄存器判断硬件是否正常工作。
    ///
    /// # 参数
    /// - `core`: 要验证的 NPU 核心
    ///
    /// # 返回
    /// - `Ok(true)`: 硬件可访问且版本正确
    /// - `Ok(false)`: 硬件可访问但版本不匹配(非 RK3588)
    /// - `Err`: 硬件不可访问
    fn verify_hardware_accessible(&self, core: NpuCore) -> Result<bool> {
        use super::config::{
            INVALID_REG_VALUE_ALL_ONE, INVALID_REG_VALUE_ALL_ZERO, RK3588_NPU_VERSION,
        };

        // 读取版本寄存器
        let version = self.read_version_reg(core)?;

        // 检查是否为无效值
        if version == INVALID_REG_VALUE_ALL_ZERO || version == INVALID_REG_VALUE_ALL_ONE {
            error!(
                "[RKNPU] Core {:?} version register returned invalid value: 0x{:08x}",
                core, version
            );
            return Err(RknpuError::HardwareNotReady);
        }

        // 对于 RK3588，验证版本号是否匹配
        if self.board == RkBoard::Rk3588 {
            if version != RK3588_NPU_VERSION {
                warn!(
                    "[RKNPU] Core {:?} version mismatch: expected 0x{:08x}, got 0x{:08x}",
                    core, RK3588_NPU_VERSION, version
                );
                return Ok(false);
            }
        }

        debug!(
            "[RKNPU] Core {:?} hardware accessible, version: 0x{:08x}",
            core, version
        );

        Ok(true)
    }

    /// 检查硬件版本
    fn check_hardware_version(&self) -> Result<()> {
        use super::config::registers;

        for i in 0..self.config.num_cores() {
            if let Some(core) = NpuCore::from_index(i) {
                let version = self.read_reg(core, registers::VERSION)?;
                let version_num = self.read_reg(core, registers::VERSION_NUM)?;

                info!(
                    "[RKNPU] Core {:?} - Version: 0x{:x}, Version Num: 0x{:x}",
                    core, version, version_num
                );
            }
        }

        Ok(())
    }

    /// 清除中断状态
    fn clear_interrupts(&mut self, core: NpuCore) -> Result<()> {
        use super::config::registers;

        self.write_reg(core, registers::INT_CLEAR, super::config::INT_CLEAR_VALUE)?;

        debug!("[RKNPU] Cleared interrupts for core {:?}", core);

        Ok(())
    }
}

// 实现 Drop trait 以确保资源正确释放
impl Drop for RK3588NPU {
    fn drop(&mut self) {
        info!("[RKNPU] Dropping device");

        // 关闭所有核心电源
        if let Err(e) = self.power_off() {
            error!("[RKNPU] Failed to power off during drop: {:?}", e);
        }
    }
}

// 由于包含 NonNull，需要手动实现 Send 和 Sync
// 这是安全的，因为我们通过 &mut self 确保了独占访问
unsafe impl Send for RK3588NPU {}
unsafe impl Sync for RK3588NPU {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_device_creation() {
        let base = unsafe { NonNull::new_unchecked(0x1000 as *mut u8) };
        let device = RK3588NPU::new(base, RkBoard::Rk3588);

        assert_eq!(device.board(), RkBoard::Rk3588);
        assert_eq!(device.num_cores(), 3);
        assert!(!device.initialized);
    }

    #[test]
    fn test_core_availability() {
        let base = unsafe { NonNull::new_unchecked(0x1000 as *mut u8) };
        let device = RK3588NPU::new(base, RkBoard::Rk3588);

        assert!(device.is_core_available(NpuCore::Npu0));
        assert!(device.is_core_available(NpuCore::Npu1));
        assert!(device.is_core_available(NpuCore::Npu2));
    }

    #[test]
    fn test_single_core_device() {
        let base = unsafe { NonNull::new_unchecked(0x1000 as *mut u8) };
        let device = RK3588NPU::new(base, RkBoard::Rk3568);

        assert_eq!(device.num_cores(), 1);
        assert!(device.is_core_available(NpuCore::Npu0));
        assert!(!device.is_core_available(NpuCore::Npu1));
    }
}
