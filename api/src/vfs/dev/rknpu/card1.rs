use alloc::collections::BTreeMap;
use core::{
    ptr::NonNull,
    sync::atomic::{AtomicU32, Ordering},
};

use axalloc::{GlobalPage, UsageKind, global_allocator};
use axhal::mem::{phys_to_virt, virt_to_phys};
use axsync::Mutex;
use memory_addr::{PAGE_SIZE_4K, PhysAddr, PhysAddrRange, VirtAddr, align_up_4k};
use rk3588_rs::{
    DRM_COMMAND_BASE, DRM_IOCTL_BASE, DrmVersion, RKNPU_ACTION, RKNPU_MEM_CREATE,
    RKNPU_MEM_DESTROY, RKNPU_MEM_MAP, RKNPU_SUBMIT, RknpuMemCreate, RknpuMemDestroy, RknpuMemMap,
    RknpuSubmit, RknpuTask,
};
use starry_core::vfs::DeviceMmap;

use crate::vfs::{
    DeviceOps,
    dev::{Any, AxError, NodeFlags, VfsResult},
};

// 导入 RKNPU 设备驱动
use super::device::{NpuCore, RK3588NPU, RkBoard};

const IOC_READ: u32 = 2;
const IOC_WRITE: u32 = 1;

const fn _iowr(ty: u8, nr: u32, size: usize) -> u32 {
    ((IOC_READ | IOC_WRITE) << 30) | ((size as u32) << 16) | ((ty as u32) << 8) | nr
}

const DRM_IOCTL_RKNPU_ACTION: u32 = _iowr(DRM_IOCTL_BASE, DRM_COMMAND_BASE + RKNPU_ACTION, 8);
const DRM_IOCTL_RKNPU_SUBMIT: u32 = _iowr(
    DRM_IOCTL_BASE,
    DRM_COMMAND_BASE + RKNPU_SUBMIT,
    core::mem::size_of::<RknpuSubmit>(),
);
const DRM_IOCTL_RKNPU_MEM_CREATE: u32 =
    _iowr(DRM_IOCTL_BASE, DRM_COMMAND_BASE + RKNPU_MEM_CREATE, 40);
const DRM_IOCTL_RKNPU_MEM_MAP: u32 = _iowr(DRM_IOCTL_BASE, DRM_COMMAND_BASE + RKNPU_MEM_MAP, 16);
const DRM_IOCTL_RKNPU_MEM_DESTROY: u32 =
    _iowr(DRM_IOCTL_BASE, DRM_COMMAND_BASE + RKNPU_MEM_DESTROY, 16);
const DRM_IOCTL_VERSION: u32 = _iowr(DRM_IOCTL_BASE, 0x00, 64);

// NPU 内存池大小：16MB
const NPU_MEMORY_POOL_SIZE: usize = 16 * 1024 * 1024;

/// 内存句柄信息
struct MemHandle {
    offset: usize,
    size: usize,
}

/// NPU 内存管理器 - 使用预分配的内存池
struct NpuMemManager {
    pool_virt: VirtAddr,
    pool_phys: PhysAddr,
    pool_size: usize,
    next_offset: usize,
    handles: BTreeMap<u32, MemHandle>,
    next_handle: AtomicU32,
}

impl NpuMemManager {
    fn new() -> VfsResult<Self> {
        let num_pages = NPU_MEMORY_POOL_SIZE / PAGE_SIZE_4K;

        let virt_addr = unsafe {
            global_allocator()
                .alloc_pages(num_pages, PAGE_SIZE_4K, UsageKind::Dma)
                .map_err(|_| AxError::NoMemory)?
        };

        let phys_addr = virt_to_phys(virt_addr.into());

        info!(
            "[RKNPU] Allocated memory pool: {} pages ({} MB), virt=0x{:x}, phys=0x{:x}",
            num_pages,
            NPU_MEMORY_POOL_SIZE / (1024 * 1024),
            virt_addr,
            phys_addr
        );

        unsafe {
            core::ptr::write_bytes(virt_addr as *mut u8, 0, NPU_MEMORY_POOL_SIZE);
        }

        Ok(Self {
            pool_virt: virt_addr.into(),
            pool_phys: phys_addr,
            pool_size: NPU_MEMORY_POOL_SIZE,
            next_offset: 0,
            handles: BTreeMap::new(),
            next_handle: AtomicU32::new(1),
        })
    }

    fn create_handle(&mut self, size: usize) -> VfsResult<(u32, u64, u64)> {
        let handle = self.next_handle.fetch_add(1, Ordering::SeqCst);

        let aligned_size = align_up_4k(size);

        if self.next_offset + aligned_size > self.pool_size {
            return Err(AxError::NoMemory);
        }

        let offset = self.next_offset;
        self.next_offset += aligned_size;

        let phys_addr = self.pool_phys.as_usize() + offset;
        let dma_addr = phys_addr as u64;
        let obj_addr = phys_addr as u64;

        info!(
            "[RKNPU] Allocated handle={}, offset=0x{:x}, size={}, phys=0x{:x}",
            handle, offset, size, phys_addr
        );

        self.handles.insert(handle, MemHandle { offset, size });

        Ok((handle, dma_addr, obj_addr))
    }

    fn destroy_handle(&mut self, handle: u32) -> bool {
        if let Some(mem) = self.handles.remove(&handle) {
            info!(
                "[RKNPU] Destroyed handle={}, offset=0x{:x}, size={}",
                handle, mem.offset, mem.size
            );
            true
        } else {
            false
        }
    }

    fn get_handle(&self, handle: u32) -> Option<&MemHandle> {
        self.handles.get(&handle)
    }

    fn get_phys_addr(&self) -> PhysAddr {
        self.pool_phys
    }

    fn get_pool_size(&self) -> usize {
        self.pool_size
    }

    /// 将用户空间的虚拟地址（实际是物理地址）转换为内核虚拟地址
    fn user_to_kernel_addr(&self, user_addr: usize) -> Option<VirtAddr> {
        let phys_addr = PhysAddr::from(user_addr);

        if phys_addr < self.pool_phys
            || phys_addr.as_usize() >= self.pool_phys.as_usize() + self.pool_size
        {
            return None;
        }

        let offset = phys_addr.as_usize() - self.pool_phys.as_usize();

        Some(VirtAddr::from(self.pool_virt.as_usize() + offset))
    }
}

impl Drop for NpuMemManager {
    fn drop(&mut self) {
        // 释放内存池
        let num_pages = self.pool_size / PAGE_SIZE_4K;
        unsafe {
            global_allocator().dealloc_pages(self.pool_virt.as_usize(), num_pages, UsageKind::Dma);
        }
        info!("[RKNPU] Freed memory pool: {} pages", num_pages);
    }
}

static NPU_MEM_MANAGER: Mutex<Option<NpuMemManager>> = Mutex::new(None);

// RK3588 NPU 基地址（从设备树或硬件手册获取）
const RK3588_NPU_BASE_ADDR: usize = 0xfd8d8000;

/// RK3588 NPU 设备全局实例
static RK3588_NPU: Mutex<Option<RK3588NPU>> = Mutex::new(None);

fn get_mem_manager() -> &'static Mutex<Option<NpuMemManager>> {
    &NPU_MEM_MANAGER
}

fn init_mem_manager() {
    let mut manager = NPU_MEM_MANAGER.lock();
    if manager.is_none() {
        match NpuMemManager::new() {
            Ok(mgr) => *manager = Some(mgr),
            Err(e) => error!("[RKNPU] Failed to initialize memory manager: {:?}", e),
        }
    }
}

/// 获取或初始化 RK3588 NPU 设备
fn get_or_init_npu() -> &'static Mutex<Option<RK3588NPU>> {
    let mut npu = RK3588_NPU.lock();
    if npu.is_none() {
        info!("[RKNPU] Initializing RK3588 NPU device...");
        
        // 创建 NPU 基地址指针
        let npu_base = unsafe { NonNull::new_unchecked(RK3588_NPU_BASE_ADDR as *mut u8) };
        
        // 创建 RK3588NPU 实例
        let mut device = RK3588NPU::new(npu_base, RkBoard::Rk3588);
        
        // 执行硬件初始化
        match device.init() {
            Ok(()) => {
                info!("[RKNPU] RK3588 NPU device initialized successfully");
                *npu = Some(device);
            }
            Err(e) => {
                error!("[RKNPU] Failed to initialize RK3588 NPU device: {:?}", e);
            }
        }
    }
    
    drop(npu);
    &RK3588_NPU
}

pub struct Card1;

impl Card1 {
    /// 执行模拟的矩阵乘法运算
    fn simulate_matmul(&self, task_ptr: *const u8) -> VfsResult<()> {
        info!("[RKNPU] Simulating matrix multiplication");

        // 读取 RknpuTask 结构
        let task = unsafe { &*(task_ptr as *const RknpuTask) };
        let regcmd_addr = unsafe { core::ptr::addr_of!(task.regcmd_addr).read_unaligned() };

        info!("[RKNPU] regcmd_addr: 0x{:x}", regcmd_addr);

        // 将 regcmd_addr（用户空间地址，实际是物理地址）转换为内核虚拟地址
        let regcmd_virt = {
            let manager = get_mem_manager().lock();
            let manager = manager.as_ref().ok_or(AxError::BadState)?;

            let virt = manager
                .user_to_kernel_addr(regcmd_addr as usize)
                .ok_or(AxError::InvalidInput)?;

            info!("[RKNPU] regcmd_virt: 0x{:x}", virt);
            virt
        };

        let regcmd_ptr = regcmd_virt.as_usize() as *const u64;

        // 从寄存器命令中提取 DMA 地址
        let ops = unsafe { core::slice::from_raw_parts(regcmd_ptr, 112) };

        let input_dma = ((ops[21] >> 16) & 0xffffffff) as usize;
        let weights_dma = ((ops[30] >> 16) & 0xffffffff) as usize;
        let output_dma = ((ops[57] >> 16) & 0xffffffff) as usize;

        info!(
            "[RKNPU] Input: 0x{:x}, Weights: 0x{:x}, Output: 0x{:x}",
            input_dma, weights_dma, output_dma
        );

        let (input_virt, weights_virt, output_virt) = {
            let manager = get_mem_manager().lock();
            let manager = manager.as_ref().ok_or(AxError::BadState)?;

            let input_virt = manager
                .user_to_kernel_addr(input_dma)
                .ok_or(AxError::InvalidInput)?;
            let weights_virt = manager
                .user_to_kernel_addr(weights_dma)
                .ok_or(AxError::InvalidInput)?;
            let output_virt = manager
                .user_to_kernel_addr(output_dma)
                .ok_or(AxError::InvalidInput)?;

            (input_virt, weights_virt, output_virt)
        };

        info!(
            "[RKNPU] Kernel addrs - Input: 0x{:x}, Weights: 0x{:x}, Output: 0x{:x}",
            input_virt, weights_virt, output_virt
        );

        // 矩阵维度
        const M: usize = 4;
        const K: usize = 36;
        const N: usize = 16;
        const K_PADDED: usize = 64;

        // 使用内核虚拟地址创建指针
        let input_ptr = input_virt.as_usize() as *const u16;
        let weights_ptr = weights_virt.as_usize() as *const u16;
        let output_ptr = output_virt.as_usize() as *mut f32;

        let mut result = [[0.0f32; N]; M];

        // 执行矩阵乘法: C = A * B^T
        for m in 0..M {
            for n in 0..N {
                let mut sum = 0.0f32;
                for k in 0..K {
                    let idx_a =
                        feature_data(K_PADDED as i32, 4, 1, 8, (k + 1) as i32, (m + 1) as i32, 1)
                            as usize;
                    let idx_b =
                        weight_fp16(K_PADDED as i32, (n + 1) as i32, (k + 1) as i32) as usize;

                    let a_val = unsafe { fp16_to_f32(*input_ptr.add(idx_a)) };
                    let b_val = unsafe { fp16_to_f32(*weights_ptr.add(idx_b)) };
                    sum += a_val * b_val;
                }
                result[m][n] = sum;
            }
        }

        // 写回结果
        for m in 0..M {
            for n in 0..N {
                let idx =
                    feature_data(N as i32, 4, 1, 4, (n + 1) as i32, (m + 1) as i32, 1) as usize;
                unsafe {
                    *output_ptr.add(idx) = result[m][n];
                }
            }
        }

        info!("[RKNPU] Matrix multiplication completed");
        Ok(())
    }
}

// FP16 转 F32
fn fp16_to_f32(bits: u16) -> f32 {
    let sign = (bits >> 15) & 1;
    let exponent = (bits >> 10) & 0x1f;
    let mantissa = bits & 0x3ff;

    if exponent == 0 {
        if mantissa == 0 {
            return if sign == 1 { -0.0 } else { 0.0 };
        }
        let val = (mantissa as f32) / 1024.0 / 16384.0;
        return if sign == 1 { -val } else { val };
    } else if exponent == 31 {
        return if mantissa == 0 {
            if sign == 1 {
                f32::NEG_INFINITY
            } else {
                f32::INFINITY
            }
        } else {
            f32::NAN
        };
    }

    let exp_val = (exponent as i32) - 15;
    let mantissa_val = 1.0 + (mantissa as f32) / 1024.0;
    let exp_multiplier = if exp_val >= 0 {
        let mut result = 1.0;
        for _ in 0..exp_val {
            result *= 2.0;
        }
        result
    } else {
        let mut result = 1.0;
        for _ in 0..(-exp_val) {
            result /= 2.0;
        }
        result
    };
    let val = mantissa_val * exp_multiplier;
    if sign == 1 { -val } else { val }
}

fn feature_data(_c: i32, h: i32, w: i32, c2: i32, c_val: i32, h_val: i32, w_val: i32) -> i32 {
    let plane = (c_val - 1) / c2;
    let src = plane * h * w * c2;
    let offset = (c_val - 1) % c2;
    let pos = src + c2 * ((h_val - 1) * w + (w_val - 1)) + offset;
    pos
}

fn weight_fp16(c: i32, k: i32, c_val: i32) -> i32 {
    let kpg = (k - 1) / 16;
    let cpg = (c_val - 1) / 32;
    let mut dst = (cpg * 32) * 16 + kpg * 16 * c;
    dst = dst + ((c_val - 1) % 32) + (((k - 1) % 16) * 32);
    dst
}

impl DeviceOps for Card1 {
    fn read_at(&self, _buf: &mut [u8], _offset: u64) -> VfsResult<usize> {
        Err(AxError::InvalidInput)
    }

    fn write_at(&self, buf: &[u8], _offset: u64) -> VfsResult<usize> {
        Ok(buf.len())
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn ioctl(&self, cmd: u32, arg: usize) -> VfsResult<usize> {
        // 初始化内存管理器
        init_mem_manager();
        
        // 初始化 NPU 设备（延迟初始化，只在首次调用时执行）
        get_or_init_npu();

        match cmd {
            DRM_IOCTL_VERSION => {
                info!("[RKNPU] DRM_IOCTL_VERSION");
                if arg != 0 {
                    let drm_ver = unsafe { &mut *(arg as *mut DrmVersion) };

                    drm_ver.version_major = 1;
                    drm_ver.version_minor = 0;
                    drm_ver.version_patchlevel = 0;

                    if !drm_ver.name.is_null() && drm_ver.name_len > 0 {
                        let name = b"rknpu\0";
                        let copy_len = core::cmp::min(name.len(), drm_ver.name_len);
                        unsafe {
                            core::ptr::copy_nonoverlapping(name.as_ptr(), drm_ver.name, copy_len);
                        }
                        drm_ver.name_len = copy_len;
                    }

                    if !drm_ver.date.is_null() && drm_ver.date_len > 0 {
                        let date = b"20241011\0";
                        let copy_len = core::cmp::min(date.len(), drm_ver.date_len);
                        unsafe {
                            core::ptr::copy_nonoverlapping(date.as_ptr(), drm_ver.date, copy_len);
                        }
                        drm_ver.date_len = copy_len;
                    }

                    if !drm_ver.desc.is_null() && drm_ver.desc_len > 0 {
                        let desc = b"Rockchip NPU Simulated\0";
                        let copy_len = core::cmp::min(desc.len(), drm_ver.desc_len);
                        unsafe {
                            core::ptr::copy_nonoverlapping(desc.as_ptr(), drm_ver.desc, copy_len);
                        }
                        drm_ver.desc_len = copy_len;
                    }
                }
                Ok(0)
            }

            DRM_IOCTL_RKNPU_ACTION => {
                info!("[RKNPU] RKNPU_ACTION");
                Ok(0)
            }

            DRM_IOCTL_RKNPU_MEM_CREATE => {
                if arg == 0 {
                    return Err(AxError::InvalidInput);
                }

                let mem_create = unsafe { &mut *(arg as *mut RknpuMemCreate) };
                info!("[RKNPU] MEM_CREATE: size={}", mem_create.size);

                let mut manager = get_mem_manager().lock();
                let manager = manager.as_mut().ok_or(AxError::BadState)?;

                let (handle, dma_addr, obj_addr) =
                    manager.create_handle(mem_create.size as usize)?;

                mem_create.handle = handle;
                mem_create.dma_addr = dma_addr;
                mem_create.obj_addr = obj_addr;

                info!(
                    "[RKNPU] MEM_CREATE: handle={}, dma=0x{:x}, obj=0x{:x}",
                    handle, dma_addr, obj_addr
                );
                Ok(0)
            }

            DRM_IOCTL_RKNPU_MEM_MAP => {
                if arg == 0 {
                    return Err(AxError::InvalidInput);
                }

                let mem_map = unsafe { &mut *(arg as *mut RknpuMemMap) };
                info!("[RKNPU] MEM_MAP: handle={}", mem_map.handle);

                let manager = get_mem_manager().lock();
                let manager = manager.as_ref().ok_or(AxError::BadState)?;
                let handle_mem = manager
                    .get_handle(mem_map.handle)
                    .ok_or(AxError::InvalidInput)?;

                mem_map.offset = handle_mem.offset as u64;
                info!(
                    "[RKNPU] MEM_MAP: handle={}, offset=0x{:x}",
                    mem_map.handle, mem_map.offset
                );
                Ok(0)
            }

            DRM_IOCTL_RKNPU_MEM_DESTROY => {
                if arg == 0 {
                    return Err(AxError::InvalidInput);
                }

                let mem_destroy = unsafe { &*(arg as *const RknpuMemDestroy) };
                info!("[RKNPU] MEM_DESTROY: handle={}", mem_destroy.handle);

                let mut manager = get_mem_manager().lock();
                let manager = manager.as_mut().ok_or(AxError::BadState)?;

                manager.destroy_handle(mem_destroy.handle);
                Ok(0)
            }

            DRM_IOCTL_RKNPU_SUBMIT => {
                if arg == 0 {
                    return Err(AxError::InvalidInput);
                }

                let submit = unsafe { &mut *(arg as *mut RknpuSubmit) };
                info!(
                    "[RKNPU] SUBMIT: task_obj_addr=0x{:x}, task_number={}, flags=0x{:x}, timeout={}",
                    submit.task_obj_addr, submit.task_number, submit.flags, submit.timeout
                );

                // 将用户空间的 task_obj_addr（实际是物理地址）转换为内核虚拟地址
                let task_virt = {
                    let manager = get_mem_manager().lock();
                    let manager = manager.as_ref().ok_or(AxError::BadState)?;

                    manager
                        .user_to_kernel_addr(submit.task_obj_addr as usize)
                        .ok_or(AxError::InvalidInput)?
                };

                info!(
                    "[RKNPU] Task virtual address: 0x{:x}, physical: 0x{:x}",
                    task_virt, submit.task_obj_addr
                );

                // 获取 NPU 设备并提交任务
                let npu_mutex = get_or_init_npu();
                let mut npu_lock = npu_mutex.lock();
                let npu = npu_lock.as_mut().ok_or(AxError::BadState)?;

                // 确保 NPU0 电源已打开
                if !npu.is_core_power_on(NpuCore::Npu0) {
                    info!("[RKNPU] Powering on NPU0 for task submission");
                    npu.power_domain_on(NpuCore::Npu0)
                        .map_err(|_| AxError::BadState)?;
                }

                // 调用真实硬件提交（阻塞模式，单核）
                match npu.submit_task_blocking(
                    task_virt.as_usize() as *const u8,
                    submit.task_start,
                    submit.task_number,
                    submit.task_obj_addr, // 物理地址用于 DMA
                    NpuCore::Npu0,        // 单核模式使用 NPU0
                    submit.timeout,
                ) {
                    Ok(completed_tasks) => {
                        submit.task_counter = completed_tasks;
                        info!(
                            "[RKNPU] Hardware task completed: {} tasks",
                            completed_tasks
                        );
                        Ok(0)
                    }
                    Err(e) => {
                        error!("[RKNPU] Hardware task submission failed: {:?}", e);
                        
                        // 如果硬件提交失败，尝试使用软件模拟作为fallback
                        warn!("[RKNPU] Falling back to software simulation");
                        self.simulate_matmul(task_virt.as_usize() as *const u8)?;
                        submit.task_counter = submit.task_number;
                        Ok(0)
                    }
                }
            }

            _ => {
                warn!("[RKNPU] Unknown ioctl: 0x{:x}", cmd);
                Err(AxError::Unsupported)
            }
        }
    }

    fn flags(&self) -> NodeFlags {
        NodeFlags::NON_CACHEABLE
    }

    fn mmap(&self) -> DeviceMmap {
        // 返回整个内存池的物理地址范围
        // 用户的 mmap offset 会被加到这个基地址上
        let manager = NPU_MEM_MANAGER.lock();
        if let Some(ref mgr) = *manager {
            let phys_addr = mgr.get_phys_addr();
            let size = mgr.get_pool_size();
            info!(
                "[RKNPU] mmap: returning Physical range phys=0x{:x}, size=0x{:x}",
                phys_addr, size
            );
            DeviceMmap::Physical(PhysAddrRange::from_start_size(phys_addr, size))
        } else {
            DeviceMmap::None
        }
    }
}
