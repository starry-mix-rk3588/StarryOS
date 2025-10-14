# RKNPU 任务提交机制实现总结

## 实现日期
2025年10月13日

## 概述
本次实现为 StarryOS 的 RKNPU 驱动添加了完整的硬件任务提交机制，支持 DRM GEM 接口、阻塞模式执行和单核任务提交。

## 实现的功能

### 1. 任务提交接口 (`submit_task_blocking`)
- **位置**: `api/src/vfs/dev/rknpu/device/rknpu_dev.rs`
- **功能**: 提供阻塞式任务提交接口
- **参数**:
  - `task_base`: 任务数组的内核虚拟地址
  - `task_start`: 起始任务索引
  - `task_number`: 任务数量
  - `task_base_phys`: 任务数组的物理地址（用于 DMA）
  - `core`: 使用的 NPU 核心
  - `timeout_ms`: 超时时间（毫秒）
- **返回**: 完成的任务数量

### 2. PC 模式硬件提交 (`job_commit_pc`)
- **位置**: `api/src/vfs/dev/rknpu/device/rknpu_dev.rs`
- **功能**: 实现 C 驱动中的 `rknpu_job_subcore_commit_pc` 逻辑
- **硬件操作流程**:
  1. 切换到 slave 模式（写 `PC_DATA_ADDR = 0x1`）
  2. 设置第一个任务的寄存器命令地址
  3. 设置数据量（寄存器配置数量）
  4. 设置最后一个任务的中断掩码
  5. 清除中断
  6. 设置任务控制（PC 模式 | 任务数量）
  7. 设置 DMA 基地址
  8. 启动 NPU（写 `PC_OP_EN = 0x1`, 然后 `0x0`）

### 3. 阻塞等待机制 (`wait_job_done`)
- **位置**: `api/src/vfs/dev/rknpu/device/rknpu_dev.rs`
- **功能**: 通过轮询中断状态寄存器等待任务完成
- **实现**:
  - 每 10μs 检查一次中断状态
  - 支持超时机制
  - 任务完成后自动清除中断

### 4. 中断状态检查 (`check_interrupt_status`)
- **位置**: `api/src/vfs/dev/rknpu/device/rknpu_dev.rs`
- **功能**: 读取并判断中断状态
- **返回**: 布尔值表示任务是否完成

### 5. DRM IOCTL 集成
- **位置**: `api/src/vfs/dev/rknpu/card1.rs`
- **功能**: 在 `DRM_IOCTL_RKNPU_SUBMIT` 处理中调用真实硬件提交
- **特性**:
  - 自动管理 NPU 电源状态
  - 失败时 fallback 到软件模拟
  - 更新任务计数器

## 修改的文件

### 1. `api/src/vfs/dev/rknpu/device/types.rs`
- 添加了三个新错误类型:
  - `TaskSubmitFailed`: 任务提交失败
  - `TaskExecutionFailed`: 任务执行失败
  - `InvalidInterruptStatus`: 中断状态异常

### 2. `api/src/vfs/dev/rknpu/device/rknpu_dev.rs`
- 添加公共方法:
  - `submit_task_blocking()`: 阻塞式任务提交接口
  - `clear_job_interrupt()`: 清除任务中断
- 添加私有方法:
  - `job_commit_pc()`: PC 模式硬件提交
  - `wait_job_done()`: 等待任务完成
  - `check_interrupt_status()`: 检查中断状态

### 3. `api/src/vfs/dev/rknpu/card1.rs`
- 导入 `NpuCore` 类型
- 修改 `DRM_IOCTL_RKNPU_SUBMIT` 处理逻辑
- 添加硬件提交流程
- 保留软件模拟作为 fallback

## 技术要点

### 1. 内存地址转换
```rust
// 用户空间物理地址 -> 内核虚拟地址
let task_virt = manager.user_to_kernel_addr(submit.task_obj_addr)?;

// 传递给硬件的物理地址用于 DMA
submit.task_obj_addr  // 物理地址
```

### 2. Packed Struct 非对齐访问
```rust
// RknpuTask 是 packed struct，需要使用 read_unaligned
let regcmd_addr = unsafe {
    let ptr = task_ptr.add(32) as *const u64;
    core::ptr::read_unaligned(ptr)
};
```

### 3. 寄存器写入顺序
严格按照 C 驱动的顺序写入寄存器，确保硬件正确初始化。

### 4. 中断轮询
使用轮询而非中断处理，简化实现并保证可靠性。

## 使用示例

```rust
use rknpu_device::{RK3588NPU, NpuCore, RkBoard};

// 初始化 NPU (地址从配置自动获取)
let mut npu = RK3588NPU::new(RkBoard::Rk3588);
npu.init()?;

// 打开电源
npu.power_domain_on(NpuCore::Npu0)?;

// 提交任务
let completed = npu.submit_task_blocking(
    task_ptr,           // 任务数组虚拟地址
    0,                  // 起始索引
    10,                 // 任务数量
    task_phys_addr,     // 物理地址
    NpuCore::Npu0,      // 使用核心 0
    5000,               // 5 秒超时
)?;

println!("Completed {} tasks", completed);
```

## 测试建议

1. **单任务测试**: 提交单个任务，验证基本流程
2. **多任务测试**: 提交多个任务，验证批处理
3. **超时测试**: 设置短超时，验证超时机制
4. **电源管理测试**: 验证自动开关电源
5. **错误恢复测试**: 验证 fallback 到软件模拟

## 已知限制

1. **单核支持**: 当前只实现了单核（NPU0）任务提交
2. **轮询模式**: 使用轮询而非中断驱动（可能影响性能）
3. **阻塞模式**: 只实现了阻塞模式，未实现异步提交
4. **简化的中断检查**: 未实现详细的中断状态验证

## 未来改进方向

1. **多核支持**: 实现多核任务调度和负载均衡
2. **中断驱动**: 使用真实中断而非轮询
3. **异步提交**: 实现非阻塞任务提交
4. **任务队列**: 实现任务队列管理
5. **性能优化**: 减少寄存器读写次数
6. **错误诊断**: 增强中断状态解析和错误诊断

## 参考文档

- Linux 内核 RKNPU 驱动: `crates/rknpu/rknpu_job.c`
- RK3588 数据手册: NPU 寄存器定义
- DRM GEM 接口规范

## 贡献者

- 实现者: GitHub Copilot (Claude)
- 审核者: [待填写]
- 测试者: [待填写]

---

最后更新: 2025年10月13日
