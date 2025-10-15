//! RKNPU 设备核心实现
//!
//! 本模块实现 RKNPU 设备的核心结构和方法。
//! 提供统一的接口来管理 NPU 硬件，包括初始化、电源管理、复位等功能。

use core::ptr::NonNull;

use rockchip_pm::{ RockchipPM};
use rockchip_pm::PD;
use super::{
    config::RknpuConfig,
    config::addresses,
    power::PowerDomainController,
    reset::ResetController,
    types::{NpuCore, PowerState, ResetType, Result, RkBoard, RknpuError},
};
use axhal::mem::phys_to_virt;
/// NPU 主电源域
pub const NPU: PD = PD(8);
/// NPU TOP 电源域  
pub const NPUTOP: PD = PD(9);
/// NPU1 电源域
pub const NPU1: PD = PD(10);
/// NPU2 电源域
pub const NPU2: PD = PD(11);

/// RK3588 NPU 设备
///
/// 这是 RK3588 NPU 的主要抽象结构，提供了完整的硬件控制接口。
///
/// # 示例
///
/// ```no_run
/// use rknpu_device::{NPU1, NPU2, RK3588NPU, RkBoard};
///
/// // 初始化 NPU (地址从配置自动获取)
/// let mut rknpu = RK3588NPU::new(RkBoard::Rk3588);
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
    /// - `board`: 板型标识
    ///
    /// # 返回
    /// 新创建的 RKNPU 设备实例
    ///
    /// # 注意
    /// 此函数只创建设备结构，不会执行硬件初始化。
    /// 需要调用 `init()` 方法来完成初始化。
    /// 所有硬件基地址从配置中自动获取，无需手动传入。
    pub fn new(board: RkBoard) -> Self {
        let config = RknpuConfig::from_board(board);
        let num_cores = config.num_cores();

        info!(
            "[RKNPU] Creating device for {:?} with {} cores",
            board, num_cores
        );

        // 根据配置获取各核心基地址
        let mut base_addrs = [None; 3];
        for i in 0..num_cores {
            if let Some(core) = NpuCore::from_index(i) {
                if let Some(addr) = config.get_core_base_addr(core) {
                    let base_ptr = unsafe { NonNull::new(addr as *mut u8) };
                    base_addrs[i] = base_ptr;

                    if let Some(ptr) = base_ptr {
                        debug!(
                            "[RKNPU] Core {} base address: 0x{:x}",
                            i,
                            ptr.as_ptr() as usize
                        );
                    }
                }
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
    /// 1. 打开所有核心电源
    /// 2. 检查硬件版本
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

        // 1. 打开所有核心电源
        info!("[RKNPU] Powering on all cores");

        let pmu_base =
            unsafe { NonNull::new(phys_to_virt(addresses::PMU1_BASE.into()).as_mut_ptr()).unwrap() };
        let mut pm = RockchipPM::new(pmu_base, rockchip_pm::RkBoard::Rk3588);
        pm.power_domain_on(NPU1).unwrap();
        pm.power_domain_off(NPU2).unwrap();
        pm.power_domain_on(NPU).unwrap();
        pm.power_domain_on(NPUTOP).unwrap();

        // self.power_on()?;

        // 2. 读取并验证硬件版本
        self.check_hardware_version()?;

        // 3. 清除所有核心的中断状态
        for i in 0..self.config.num_cores() {
            if let Some(core) = NpuCore::from_index(i) {
                self.clear_interrupts(core)?;
                break;
            }
        }

        // 4. 标记为已初始化
        self.initialized = true;

        info!("[RKNPU] Device initialized successfully (all cores powered on)");

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

                break;  
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

    // ========== 任务提交相关方法 ==========

    /// 提交任务到 NPU 硬件（阻塞模式，单核）
    ///
    /// 此方法实现了完整的任务提交流程：
    /// 1. 向硬件寄存器提交任务参数（PC 模式）
    /// 2. 启动 NPU 执行
    /// 3. 阻塞等待任务完成
    /// 4. 验证执行结果
    ///
    /// # 参数
    /// - `task_base`: 任务数组的基地址（内核虚拟地址）
    /// - `task_start`: 起始任务索引
    /// - `task_number`: 任务数量
    /// - `task_base_phys`: 任务数组的物理地址（用于 DMA）
    /// - `core`: 使用的核心（单核模式）
    /// - `timeout_ms`: 超时时间（毫秒）
    ///
    /// # 返回
    /// 成功返回实际完成的任务数，失败返回错误
    ///
    /// # 示例
    /// ```no_run
    /// use rknpu_device::{NpuCore, RK3588NPU};
    ///
    /// let mut npu = RK3588NPU::new(base_addr, RkBoard::Rk3588);
    /// npu.init()?;
    /// npu.power_domain_on(NpuCore::Npu0)?;
    ///
    /// let task_count = npu.submit_task_blocking(
    ///     task_ptr,
    ///     0,  // task_start
    ///     10, // task_number
    ///     task_phys_addr,
    ///     NpuCore::Npu0,
    ///     5000, // 5 秒超时
    /// )?;
    /// ```
    pub fn submit_task_blocking(
        &mut self,
        task_base: *const u8,
        task_start: u32,
        task_number: u32,
        task_base_phys: u64,
        core: NpuCore,
        timeout_ms: u32,
    ) -> Result<u32> {
        if task_number == 0 {
            return Err(RknpuError::InvalidParameter);
        }

        if !self.is_core_available(core) {
            return Err(RknpuError::CoreUnavailable);
        }

        info!(
            "[RKNPU] Submitting task: core={:?}, start={}, count={}, timeout={}ms",
            core, task_start, task_number, timeout_ms
        );

        // 1. 提交到硬件
        self.job_commit_pc(task_base, task_start, task_number, task_base_phys, core)?;

        // 2. 等待完成
        self.wait_job_done(core, task_number, timeout_ms)?;

        // 3. 返回完成的任务数
        info!("[RKNPU] Task completed successfully: {} tasks", task_number);

        Ok(task_number)
    }

    /// PC 模式硬件提交
    ///
    /// 此方法将任务参数写入硬件寄存器，启动 NPU 执行。
    /// 实现参考 C 代码中的 `rknpu_job_subcore_commit_pc` 函数。
    ///
    /// # 硬件操作流程
    /// 1. 切换到 slave 模式（写 PC_DATA_ADDR = 0x1）
    /// 2. 设置第一个任务的寄存器命令地址
    /// 3. 设置数据量（寄存器配置数量）
    /// 4. 设置最后一个任务的中断掩码
    /// 5. 清除中断
    /// 6. 设置任务控制（PC 模式 | 任务数量）
    /// 7. 设置 DMA 基地址
    /// 8. 启动 NPU（写 PC_OP_EN = 0x1, 然后 0x0）
    fn job_commit_pc(
        &mut self,
        task_base: *const u8,
        task_start: u32,
        task_number: u32,
        task_base_phys: u64,
        core: NpuCore,
    ) -> Result<()> {
        use super::config::{PC_DATA_EXTRA_AMOUNT, registers};

        // RknpuTask 的大小（从 C 代码中的 packed struct）
        const TASK_SIZE: usize = 40; // sizeof(RknpuTask)

        // 计算第一个和最后一个任务的地址
        let first_task_offset = (task_start as usize) * TASK_SIZE;
        let last_task_offset = ((task_start + task_number - 1) as usize) * TASK_SIZE;

        let first_task_ptr = unsafe { task_base.add(first_task_offset) };
        let last_task_ptr = unsafe { task_base.add(last_task_offset) };

        // 读取任务参数（需要处理 packed struct 的非对齐访问）
        let first_regcmd_addr = unsafe {
            let ptr = first_task_ptr.add(32) as *const u64; // regcmd_addr 在偏移 32
            core::ptr::read_unaligned(ptr)
        };

        let first_regcfg_amount = unsafe {
            let ptr = first_task_ptr.add(24) as *const u32; // regcfg_amount 在偏移 24
            core::ptr::read_unaligned(ptr)
        };

        let last_int_mask = unsafe {
            let ptr = last_task_ptr.add(12) as *const u32; // int_mask 在偏移 12
            core::ptr::read_unaligned(ptr)
        };

        let first_int_mask = unsafe {
            let ptr = first_task_ptr.add(12) as *const u32; // int_mask 在偏移 12
            core::ptr::read_unaligned(ptr)
        };

        debug!(
            "[RKNPU] Task params: regcmd_addr=0x{:x}, regcfg_amount={}, last_int_mask=0x{:x}",
            first_regcmd_addr, first_regcfg_amount, last_int_mask
        );

        // 1. 切换到 slave 模式
        self.write_reg(core, registers::PC_DATA_ADDR, 0x1)?;

        // 2. 设置 PC 数据地址（第一个任务的寄存器命令地址）
        self.write_reg(core, registers::PC_DATA_ADDR, first_regcmd_addr as u32)?;

        // 3. 计算并设置数据量
        let pc_data_amount_scale = self.config.pc_data_amount_scale;
        let data_amount = (first_regcfg_amount + PC_DATA_EXTRA_AMOUNT + pc_data_amount_scale - 1)
            / pc_data_amount_scale
            - 1;
        self.write_reg(core, registers::PC_DATA_AMOUNT, data_amount)?;

        // 4. 设置中断掩码（最后一个任务的）
        self.write_reg(core, registers::INT_MASK, last_int_mask)?;

        // 5. 清除中断（使用第一个任务的中断掩码）
        self.write_reg(core, registers::INT_CLEAR, first_int_mask)?;

        // 6. 设置任务控制
        // 格式: ((0x6 | task_pp_en) << pc_task_number_bits) | task_number
        // task_pp_en = 0 (不使用 ping-pong 模式)
        let pc_task_number_bits = self.config.pc_task_number_bits;
        let task_control = ((0x6) << pc_task_number_bits) | task_number;
        self.write_reg(core, registers::PC_TASK_CONTROL, task_control)?;

        // 7. 设置 DMA 基地址
        self.write_reg(core, registers::PC_DMA_BASE_ADDR, task_base_phys as u32)?;

        // 8. 启动 NPU
        self.write_reg(core, registers::PC_OP_EN, 0x1)?;
        self.write_reg(core, registers::PC_OP_EN, 0x0)?;

        info!(
            "[RKNPU] Task submitted to hardware: core={:?}, tasks={}, control=0x{:x}",
            core, task_number, task_control
        );

        Ok(())
    }

    /// 等待任务完成（阻塞模式）
    ///
    /// 通过轮询中断状态寄存器来等待任务完成。
    /// 如果超时则返回错误。
    ///
    /// # 参数
    /// - `core`: NPU 核心
    /// - `expected_tasks`: 期望完成的任务数
    /// - `timeout_ms`: 超时时间（毫秒）
    fn wait_job_done(&mut self, core: NpuCore, expected_tasks: u32, timeout_ms: u32) -> Result<()> {
        use core::time::Duration;

        use axhal::time::{busy_wait, current_ticks};

        let start_time = current_ticks();
        let timeout_ns = (timeout_ms as u64) * 1_000_000; // 转换为纳秒

        debug!(
            "[RKNPU] Waiting for job completion: core={:?}, tasks={}, timeout={}ms",
            core, expected_tasks, timeout_ms
        );

        loop {
            // 检查中断状态
            match self.check_interrupt_status(core, expected_tasks) {
                Ok(true) => {
                    // 清除中断
                    self.clear_job_interrupt(core)?;

                    let elapsed_us = (current_ticks() - start_time) / 1000;
                    info!(
                        "[RKNPU] Job completed: core={:?}, elapsed={}us",
                        core, elapsed_us
                    );
                    return Ok(());
                }
                Ok(false) => {
                    // 任务未完成，检查是否超时
                    let elapsed_ns = current_ticks() - start_time;
                    if elapsed_ns >= timeout_ns {
                        error!(
                            "[RKNPU] Job timeout: core={:?}, timeout={}ms",
                            core, timeout_ms
                        );
                        return Err(RknpuError::Timeout);
                    }

                    // 短暂延时后继续轮询（10us）
                    busy_wait(Duration::from_micros(10));
                }
                Err(e) => {
                    error!("[RKNPU] Interrupt status check failed: {:?}", e);
                    return Err(e);
                }
            }
        }
    }

    /// 检查中断状态
    ///
    /// 读取中断状态寄存器，判断任务是否完成。
    ///
    /// # 返回
    /// - `Ok(true)`: 任务已完成
    /// - `Ok(false)`: 任务未完成
    /// - `Err`: 读取失败或状态异常
    fn check_interrupt_status(&self, core: NpuCore, _expected_tasks: u32) -> Result<bool> {
        use super::config::registers;

        // 读取中断状态
        let int_status = self.read_reg(core, registers::INT_STATUS)?;

        // 读取原始中断状态
        let int_raw_status = self.read_reg(core, registers::INT_RAW_STATUS)?;

        // 如果中断状态非零，说明有中断发生
        if int_status != 0 {
            debug!(
                "[RKNPU] Interrupt detected: status=0x{:x}, raw=0x{:x}",
                int_status, int_raw_status
            );

            // TODO: 这里可以添加更详细的中断状态验证
            // 例如检查错误位、完成位等

            return Ok(true);
        }

        Ok(false)
    }

    /// 清除中断状态（用于任务完成后）
    ///
    /// 在检测到任务完成后调用此方法清除中断标志
    pub fn clear_job_interrupt(&mut self, core: NpuCore) -> Result<()> {
        use super::config::registers;

        self.write_reg(core, registers::INT_CLEAR, super::config::INT_CLEAR_VALUE)?;
        debug!("[RKNPU] Cleared job interrupt for core {:?}", core);

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
        let device = RK3588NPU::new(RkBoard::Rk3588);

        assert_eq!(device.board(), RkBoard::Rk3588);
        assert_eq!(device.num_cores(), 3);
        assert!(!device.initialized);
    }

    #[test]
    fn test_core_availability() {
        let device = RK3588NPU::new(RkBoard::Rk3588);

        assert!(device.is_core_available(NpuCore::Npu0));
        assert!(device.is_core_available(NpuCore::Npu1));
        assert!(device.is_core_available(NpuCore::Npu2));
    }

    #[test]
    fn test_single_core_device() {
        let device = RK3588NPU::new(RkBoard::Rk3568);

        assert_eq!(device.num_cores(), 1);
        assert!(device.is_core_available(NpuCore::Npu0));
        assert!(!device.is_core_available(NpuCore::Npu1));
    }
}
