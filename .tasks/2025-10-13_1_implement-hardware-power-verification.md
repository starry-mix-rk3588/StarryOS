# Context
File name: 2025-10-13_1
Created at: 2025-10-13_15:30:00
Created by: debin
Main branch: rk3588
Task Branch: task/implement-hardware-power-verification_2025-10-13_1
Yolo Mode: Off

# Task Description
实现 RKNPU Device 的真实硬件电源管理和验证功能。用户要求实际操作真正的硬件并验证电源是否正确打开。

需要实现:
1. 真实的硬件寄存器读写操作
2. 电源打开后的状态验证机制
3. 通过读取版本寄存器等方式验证硬件可访问性
4. 添加超时和轮询机制确保电源状态稳定
5. 实现 `is_power_on()` 查询接口

参考:
- C 驱动: `crates/rknpu/rknpu_drv.c` 中的 `rknpu_power_on()` 函数
- 寄存器定义: `crates/rknpu2-rslab/rk3588-rs/src/ioctl.rs`
- 现有实现: `api/src/vfs/dev/rknpu/device/power.rs`

# Project Overview
StarryOS RKNPU 驱动项目,为 RK3588 NPU 设备提供 Rust 抽象层。目标是在 no_std 环境中提供硬件级别的 NPU 电源和复位控制。

⚠️ WARNING: NEVER MODIFY THIS SECTION ⚠️
## RIPER-5 Core Protocol Rules
- 始终以 [MODE: MODE_NAME] 开始响应
- 不得在未经明确许可的情况下跨模式转换
- EXECUTE 模式必须 100% 遵循计划
- REVIEW 模式必须标记最小的偏差
- 在 EXECUTE 模式外无权做出独立决策
⚠️ WARNING: NEVER MODIFY THIS SECTION ⚠️

# Analysis
通过研究 C 驱动代码,发现电源验证的关键方法:

1. **返回值检查**: 
   - `regulator_enable()` 返回 0 表示成功
   - `clk_bulk_prepare_enable()` 返回 0 表示成功
   - `pm_runtime_get_sync()` 返回值 ≥ 0 表示成功

2. **寄存器读取验证**:
   - 版本寄存器 `RKNPU_OFFSET_VERSION` (0x0) 应返回 0x60002
   - 版本号寄存器 `RKNPU_OFFSET_VERSION_NUM` (0x4) 应返回有效值
   - 如果电源未开启,读取会返回异常值(全0或全1)

3. **PM Runtime 状态**:
   - `pm_runtime_active()` 可检查设备是否处于活动状态
   - 在 `rknpu_devfreq.c` 中使用此方法验证

4. **超时机制**:
   - C 驱动使用 `readx_poll_timeout()` 轮询 IOMMU 状态
   - 需要实现类似的轮询机制确保状态稳定

当前实现位于 `api/src/vfs/dev/rknpu/device/`:
- `power.rs` (357行): 模拟了电源操作但未真实访问硬件
- `rknpu_dev.rs` (399行): 提供了 `read_reg()`/`write_reg()` 接口
- `config.rs` (259行): 定义了寄存器地址

需要增强的部分:
1. 在 `PowerDomainController` 中添加硬件验证
2. 读取版本寄存器验证电源状态
3. 添加超时轮询机制
4. 提供 `is_power_on()` 公共接口

# Proposed Solution
## 方案 1: 在 PowerDomainController 中直接验证
**优点**: 
- 封装完整,电源控制和验证在同一个模块
- 可以在 power_domain_on/off 时自动验证

**缺点**:
- PowerDomainController 需要持有设备基地址
- 增加耦合度

## 方案 2: 在 RK3588NPU 中添加验证方法
**优点**:
- 保持 PowerDomainController 的独立性
- 利用已有的 read_reg/write_reg 接口
- 更清晰的职责划分

**缺点**:
- 验证逻辑与电源操作分离

**推荐方案 2**,原因:
- 符合单一职责原则
- 利用现有基础设施
- 更容易测试和维护

## 实现计划:
1. 在 `power.rs` 中添加硬件状态验证所需的常量
2. 在 `rknpu_dev.rs` 中实现电源状态验证方法:
   - `verify_power_on()`: 验证电源是否正确打开
   - `is_power_on()`: 查询当前电源状态  
   - `read_version_reg()`: 读取版本寄存器
3. 在 `power_domain_on()` 后自动调用验证
4. 添加超时和重试机制
5. 更新文档说明验证机制

# Current execution step: "1. 创建任务分支"

# Task Progress
[2025-10-13_15:30:00]
- Modified: .tasks/2025-10-13_1_implement-hardware-power-verification.md
- Changes: 创建任务文件,完成需求分析和方案设计
- Reason: 明确实现真实硬件电源验证的需求和计划
- Blockers: 无
- Status: UNCONFIRMED

# Final Review:
待完成
