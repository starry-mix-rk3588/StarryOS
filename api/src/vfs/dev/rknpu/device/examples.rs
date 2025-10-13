//! RKNPU Device 使用示例
//!
//! 本文件展示如何使用 RKNPU Device 抽象层的各种功能。

#![allow(dead_code)]

use core::ptr::NonNull;

// 导入需要的类型
use super::{NPU0, NPU1, NPU2, PowerState, RK3588NPU, ResetType, Result, RkBoard};

/// 示例 1: 基本初始化和电源管理
///
/// 展示如何创建 NPU 设备实例并控制电源。
pub fn example_basic_init() -> Result<()> {
    // 1. 从设备树获取基地址（这里使用硬编码地址作为示例）
    let npu_base_addr = 0xfd8d8000_usize;
    let npu_base = unsafe { NonNull::new_unchecked(npu_base_addr as *mut u8) };

    // 2. 创建 RK3588 NPU 设备实例
    let mut rknpu = RK3588NPU::new(npu_base, RkBoard::Rk3588);

    // 3. 初始化硬件
    rknpu.init()?;

    // 4. 打开所有核心的电源
    rknpu.power_on()?;

    // 5. 执行一些操作...
    log::info!("NPU initialized and powered on");

    // 6. 关闭所有核心的电源
    rknpu.power_off()?;

    Ok(())
}

/// 示例 2: 单独控制核心电源域
///
/// 展示如何独立控制每个 NPU 核心的电源。
pub fn example_power_domain_control() -> Result<()> {
    let npu_base = unsafe { NonNull::new_unchecked(0xfd8d8000 as *mut u8) };
    let mut rknpu = RK3588NPU::new(npu_base, RkBoard::Rk3588);
    rknpu.init()?;

    // 只打开 NPU0 和 NPU1
    log::info!("Powering on NPU0 and NPU1");
    rknpu.power_domain_on(NPU0)?;
    rknpu.power_domain_on(NPU1)?;

    // NPU2 保持关闭状态
    let npu2_state = rknpu.get_power_state(NPU2)?;
    assert_eq!(npu2_state, PowerState::Off);

    // 在 NPU0 和 NPU1 上执行任务...

    // 关闭 NPU1，打开 NPU2
    log::info!("Switching from NPU1 to NPU2");
    rknpu.power_domain_off(NPU1)?;
    rknpu.power_domain_on(NPU2)?;

    // 最后关闭所有核心
    rknpu.power_off()?;

    Ok(())
}

/// 示例 3: 复位操作
///
/// 展示如何执行各种类型的复位操作。
pub fn example_reset_operations() -> Result<()> {
    let npu_base = unsafe { NonNull::new_unchecked(0xfd8d8000 as *mut u8) };
    let mut rknpu = RK3588NPU::new(npu_base, RkBoard::Rk3588);
    rknpu.init()?;
    rknpu.power_on()?;

    // 1. 软复位所有核心
    log::info!("Performing soft reset on all cores");
    rknpu.soft_reset()?;

    // 2. 软复位单个核心
    log::info!("Performing soft reset on NPU1");
    rknpu.soft_reset_core(NPU1)?;

    // 3. 执行 AXI 复位
    log::info!("Performing AXI reset on NPU0");
    rknpu.reset(NPU0, ResetType::Axi)?;

    // 4. 执行 AHB 复位
    log::info!("Performing AHB reset on NPU0");
    rknpu.reset(NPU0, ResetType::Ahb)?;

    rknpu.power_off()?;
    Ok(())
}

/// 示例 4: 寄存器访问
///
/// 展示如何读写 NPU 寄存器。
pub fn example_register_access() -> Result<()> {
    use super::registers;

    let npu_base = unsafe { NonNull::new_unchecked(0xfd8d8000 as *mut u8) };
    let mut rknpu = RK3588NPU::new(npu_base, RkBoard::Rk3588);
    rknpu.init()?;
    rknpu.power_on()?;

    // 1. 读取版本信息
    let version = rknpu.read_reg(NPU0, registers::VERSION)?;
    let version_num = rknpu.read_reg(NPU0, registers::VERSION_NUM)?;
    log::info!(
        "NPU Version: 0x{:x}, Version Num: 0x{:x}",
        version,
        version_num
    );

    // 2. 清除中断状态
    rknpu.write_reg(NPU0, registers::INT_CLEAR, 0x1ffff)?;

    // 3. 读取中断状态
    let int_status = rknpu.read_reg(NPU0, registers::INT_STATUS)?;
    log::info!("Interrupt Status: 0x{:x}", int_status);

    // 4. 禁用 PC 操作
    rknpu.write_reg(NPU0, registers::PC_OP_EN, 0)?;

    rknpu.power_off()?;
    Ok(())
}

/// 示例 5: 查询设备信息
///
/// 展示如何查询设备的配置和状态信息。
pub fn example_device_info() -> Result<()> {
    let npu_base = unsafe { NonNull::new_unchecked(0xfd8d8000 as *mut u8) };
    let rknpu = RK3588NPU::new(npu_base, RkBoard::Rk3588);

    // 1. 查询板型信息
    log::info!("Board: {:?}", rknpu.board());

    // 2. 查询核心数量
    log::info!("Number of cores: {}", rknpu.num_cores());

    // 3. 查询配置信息
    let config = rknpu.config();
    log::info!("DMA mask bits: {}", config.dma_mask_bits);
    log::info!("Max submit number: {}", config.max_submit_number);
    log::info!("Core mask: 0x{:x}", config.core_mask);

    // 4. 检查核心是否可用
    for i in 0..3 {
        if let Some(core) = super::NpuCore::from_index(i) {
            let available = rknpu.is_core_available(core);
            log::info!("Core {:?} available: {}", core, available);
        }
    }

    Ok(())
}

/// 示例 6: 错误处理
///
/// 展示如何处理各种错误情况。
pub fn example_error_handling() -> Result<()> {
    use super::RknpuError;

    let npu_base = unsafe { NonNull::new_unchecked(0xfd8d8000 as *mut u8) };
    let mut rknpu = RK3588NPU::new(npu_base, RkBoard::Rk3568); // 单核心配置

    rknpu.init()?;

    // 尝试访问不可用的核心
    match rknpu.power_domain_on(NPU1) {
        Ok(()) => log::info!("NPU1 powered on"),
        Err(RknpuError::CoreUnavailable) => {
            log::warn!("NPU1 is not available on this board");
        }
        Err(e) => {
            log::error!("Unexpected error: {:?}", e);
        }
    }

    // 尝试在未初始化的情况下读取寄存器
    // （这里已经初始化了，所以不会出错，仅作为示例）
    match rknpu.read_reg(NPU0, 0x0) {
        Ok(val) => log::info!("Register value: 0x{:x}", val),
        Err(RknpuError::NotInitialized) => {
            log::error!("Device not initialized");
        }
        Err(e) => {
            log::error!("Read error: {:?}", e);
        }
    }

    Ok(())
}

/// 示例 7: 多核心配置示例（RK3588）
///
/// 展示如何在 RK3588 的 3 核心配置中使用 NPU。
pub fn example_rk3588_multi_core() -> Result<()> {
    let npu_base = unsafe { NonNull::new_unchecked(0xfd8d8000 as *mut u8) };
    let mut rknpu = RK3588NPU::new(npu_base, RkBoard::Rk3588);
    rknpu.init()?;

    log::info!("RK3588 has {} NPU cores", rknpu.num_cores());

    // 逐个打开核心电源
    for i in 0..rknpu.num_cores() {
        if let Some(core) = super::NpuCore::from_index(i) {
            log::info!("Powering on core {:?}", core);
            rknpu.power_domain_on(core)?;

            // 获取核心基地址
            let base_addr = rknpu.get_core_base_addr(core)?;
            log::info!("Core {:?} base address: {:p}", core, base_addr.as_ptr());

            // 读取核心版本
            let version = rknpu.read_reg(core, super::registers::VERSION)?;
            log::info!("Core {:?} version: 0x{:x}", core, version);
        }
    }

    // 执行任务分配...
    // 例如：NPU0 处理任务 A，NPU1 处理任务 B，NPU2 处理任务 C

    // 全部完成后关闭
    rknpu.power_off()?;

    Ok(())
}

/// 示例 8: 单核心配置示例（RK3568）
///
/// 展示如何在 RK3568 的单核心配置中使用 NPU。
pub fn example_rk3568_single_core() -> Result<()> {
    let npu_base = unsafe { NonNull::new_unchecked(0xfe000000 as *mut u8) };
    let mut rknpu = RK3588NPU::new(npu_base, RkBoard::Rk3568);
    rknpu.init()?;

    log::info!("RK3568 has {} NPU core", rknpu.num_cores());

    // 只有 NPU0 可用
    assert!(rknpu.is_core_available(NPU0));
    assert!(!rknpu.is_core_available(NPU1));
    assert!(!rknpu.is_core_available(NPU2));

    // 打开唯一的核心
    rknpu.power_domain_on(NPU0)?;

    // 使用 NPU0 执行任务...

    rknpu.power_off()?;
    Ok(())
}

/// 示例 9: 跳过软复位
///
/// 在某些情况下可能需要跳过软复位操作。
pub fn example_bypass_soft_reset() -> Result<()> {
    let npu_base = unsafe { NonNull::new_unchecked(0xfd8d8000 as *mut u8) };
    let mut rknpu = RK3588NPU::new(npu_base, RkBoard::Rk3588);

    // 设置跳过软复位
    rknpu.set_bypass_soft_reset(true);

    rknpu.init()?;
    rknpu.power_on()?;

    // 这次软复位将被跳过
    rknpu.soft_reset()?; // 实际上不会执行复位

    // 取消跳过
    rknpu.set_bypass_soft_reset(false);

    // 现在会正常执行复位
    rknpu.soft_reset()?;

    rknpu.power_off()?;
    Ok(())
}

/// 示例 10: 综合使用场景
///
/// 模拟一个完整的 NPU 使用流程。
pub fn example_complete_workflow() -> Result<()> {
    log::info!("=== Starting NPU workflow ===");

    // 1. 初始化设备
    let npu_base = unsafe { NonNull::new_unchecked(0xfd8d8000 as *mut u8) };
    let mut rknpu = RK3588NPU::new(npu_base, RkBoard::Rk3588);
    rknpu.init()?;
    log::info!("✓ Device initialized");

    // 2. 打开需要的核心
    rknpu.power_domain_on(NPU0)?;
    rknpu.power_domain_on(NPU1)?;
    log::info!("✓ Cores powered on");

    // 3. 执行软复位确保状态干净
    rknpu.soft_reset()?;
    log::info!("✓ Soft reset completed");

    // 4. 配置寄存器（示例）
    rknpu.write_reg(NPU0, super::registers::INT_MASK, 0)?;
    log::info!("✓ Registers configured");

    // 5. 提交任务到 NPU...
    // （这里省略实际的任务提交代码）
    log::info!("✓ Tasks submitted");

    // 6. 等待任务完成并检查状态
    let status = rknpu.read_reg(NPU0, super::registers::INT_STATUS)?;
    log::info!("✓ Task status: 0x{:x}", status);

    // 7. 清理并关闭
    rknpu.write_reg(NPU0, super::registers::INT_CLEAR, 0x1ffff)?;
    rknpu.power_off()?;
    log::info!("✓ Cleanup completed");

    log::info!("=== NPU workflow finished ===");

    Ok(())
}

/// 示例 11: 电源状态验证
///
/// 演示如何验证电源是否正确打开，以及如何查询电源状态。
pub fn example_power_verification() -> Result<()> {
    log::info!("=== Example 11: Power Verification ===");

    let npu_base = unsafe { NonNull::new_unchecked(0xfd8d8000 as *mut u8) };
    let mut rknpu = RK3588NPU::new(npu_base, RkBoard::Rk3588);
    rknpu.init()?;

    // 1. 打开电源并自动验证
    log::info!("Opening power for NPU0...");
    match rknpu.power_domain_on(NPU0) {
        Ok(_) => log::info!("✓ NPU0 power ON and verified successfully"),
        Err(e) => {
            log::error!("✗ NPU0 power verification failed: {}", e);
            return Err(e);
        }
    }

    // 2. 查询电源状态
    log::info!("Checking power status...");
    if rknpu.is_core_power_on(NPU0) {
        log::info!("✓ NPU0 is powered ON");
    } else {
        log::warn!("✗ NPU0 is powered OFF or inaccessible");
    }

    // 3. 读取版本寄存器验证硬件
    log::info!("Reading version register...");
    let version = rknpu.read_version_reg(NPU0)?;
    log::info!("NPU0 version: 0x{:08x}", version);

    if version == super::RK3588_NPU_VERSION {
        log::info!("✓ Version matches RK3588 (v6.0.2)");
    } else {
        log::warn!(
            "! Version mismatch: expected 0x{:08x}, got 0x{:08x}",
            super::RK3588_NPU_VERSION,
            version
        );
    }

    // 4. 测试多核心验证
    log::info!("Verifying all cores...");
    for core in [NPU0, NPU1, NPU2] {
        if rknpu.is_core_available(core) {
            match rknpu.power_domain_on(core) {
                Ok(_) => {
                    let version = rknpu.read_version_reg(core)?;
                    log::info!("✓ Core {:?} - Version: 0x{:08x}", core, version);
                }
                Err(e) => {
                    log::error!("✗ Core {:?} verification failed: {}", core, e);
                }
            }
        }
    }

    // 5. 演示验证超时处理
    log::info!("Testing timeout handling...");
    // 注意：在真实硬件上，这个测试可能会成功
    // 只有在硬件故障时才会超时
    match rknpu.verify_power_on_with_timeout(NPU0, 1000) {
        Ok(_) => log::info!("✓ Verification completed within timeout"),
        Err(RknpuError::Timeout) => log::warn!("! Verification timeout (expected in some cases)"),
        Err(e) => log::error!("✗ Verification error: {}", e),
    }

    // 6. 关闭电源
    log::info!("Powering off...");
    rknpu.power_off()?;

    // 7. 验证电源已关闭
    if !rknpu.is_core_power_on(NPU0) {
        log::info!("✓ NPU0 power OFF confirmed");
    } else {
        log::warn!("! NPU0 still shows as powered ON");
    }

    log::info!("=== Power verification example completed ===");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore] // 需要真实硬件才能运行
    fn test_examples() {
        // 这些测试需要真实的 NPU 硬件才能运行
        // 在模拟环境中会失败
    }
}
