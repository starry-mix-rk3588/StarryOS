use core::ptr::NonNull;

use axhal::mem::{PhysAddr, pa, phys_to_virt};
use memory_addr::PhysAddrRange;
use rk3588_rs::{RknpuMemCreate, RknpuMemDestroy, RknpuMemMap};
use rknpu_driver::{
    RknpuDev,
    memory::NpuAllocator,
    rknpu_ioctl,
    types::{NpuCore, RkBoard, RkNpuIoctl},
};
use starry_core::vfs::DeviceMmap;

use crate::vfs::dev::*;

const RKNPU_CORE_BASE: PhysAddr = pa!(0xFDAB0000);
const RKNU_PMU1_BASE: PhysAddr = pa!(0xFD8D8000);

const NPU0_IRQ: usize = 142;
const NPU1_IRQ: usize = 143;
const NPU2_IRQ: usize = 144;

use lazy_static::lazy_static;

use super::memory::NpuDmaAllocator;

lazy_static! {
    static ref RKNPU: RknpuDev = {
        let mut dev = RknpuDev::new(phys_to_virt(RKNPU_CORE_BASE).as_usize(), RkBoard::Rk3588);
        let pmu_base = NonNull::new(phys_to_virt(RKNU_PMU1_BASE).as_mut_ptr()).unwrap();
        dev.initialize(pmu_base).unwrap();

        // 注册中断处理程序
        axplat::irq::register(NPU0_IRQ, |_| {
            // 获取 RKNPU 的引用并处理中断
            if let Ok(status) = RKNPU.handle_irq(NpuCore::Npu0) {
                info!("[RKNPU] IRQ handled, status=0x{:x}", status);
            }
        });
        axplat::irq::register(NPU1_IRQ, |_| {
            // 获取 RKNPU 的引用并处理中断
            if let Ok(status) = RKNPU.handle_irq(NpuCore::Npu1) {
                info!("[RKNPU] IRQ handled, status=0x{:x}", status);
            }
        });
        axplat::irq::register(NPU2_IRQ, |_| {
            // 获取 RKNPU 的引用并处理中断
            if let Ok(status) = RKNPU.handle_irq(NpuCore::Npu2) {
                info!("[RKNPU] IRQ handled, status=0x{:x}", status);
            }
        });

        info!("[RKNPU] Initialized and IRQ {} registered", NPU0_IRQ);

        dev
    };
}

lazy_static! {
    static ref NPU_ALLOCATOR: NpuDmaAllocator = {
        let allocator = NpuDmaAllocator::new();
        allocator
    };
}

pub struct Card0;

impl DeviceOps for Card0 {
    fn read_at(&self, _buf: &mut [u8], _offset: u64) -> VfsResult<usize> {
        info!("card read = >");
        Err(AxError::InvalidInput)
    }

    fn write_at(&self, buf: &[u8], _offset: u64) -> VfsResult<usize> {
        info!("card write = >");
        Ok(buf.len())
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn flags(&self) -> NodeFlags {
        NodeFlags::NON_CACHEABLE
    }

    fn ioctl(&self, cmd: u32, arg: usize) -> VfsResult<usize> {
        let rknpu_cmd = RkNpuIoctl::from_cmd(cmd);
        if rknpu_cmd.is_none() {
            return Err(AxError::InvalidInput);
        }
        info!("card0 ioctl => cmd: {:?}, arg: {:#x}", rknpu_cmd, arg);

        match rknpu_cmd {
            Some(RkNpuIoctl::RknpuMemCreate) => {
                let mem_create = unsafe { &mut *(arg as *mut RknpuMemCreate) };
                info!("[RKNPU] MemCreate ioctl: size={} bytes", mem_create.size);
                if let Ok((handle, dma_addr, obj_addr)) =
                    NPU_ALLOCATOR.create_handle(mem_create.size as usize)
                {
                    mem_create.handle = handle;
                    mem_create.dma_addr = dma_addr;
                    mem_create.obj_addr = obj_addr;

                    info!(
                        "[RKNPU] MemCreate result: handle={}, dma_addr=0x{:x}, obj_addr=0x{:x}",
                        handle, dma_addr, obj_addr
                    );

                    return Ok(0);
                }
                return Err(AxError::InvalidInput);
            }
            Some(RkNpuIoctl::RknpuMemMap) => {
                let mem_map = unsafe { &mut *(arg as *mut RknpuMemMap) };
                info!("[RKNPU] MemMap ioctl: handle={}", mem_map.handle);
                if let Ok((offset, _size)) = NPU_ALLOCATOR.get_handle(mem_map.handle) {
                    mem_map.offset = offset;

                    info!(
                        "[RKNPU] MemMap result: handle={}, offset=0x{:x}",
                        mem_map.handle, mem_map.offset
                    );

                    return Ok(0);
                }
                return Err(AxError::InvalidInput);
            }
            Some(RkNpuIoctl::RknpuMemDestroy) => {
                let mem_destroy = unsafe { &mut *(arg as *mut RknpuMemDestroy) };
                info!("[RKNPU] MemDestroy ioctl: handle={}", mem_destroy.handle);
                if NPU_ALLOCATOR.destroy_handle(mem_destroy.handle) {
                    return Ok(0);
                }
                return Err(AxError::InvalidInput);
            }
            Some(_) => {
                if let Ok(()) = rknpu_ioctl(&RKNPU, rknpu_cmd, arg, |pa| {
                    NPU_ALLOCATOR.user_to_kernel_addr(pa.as_usize()).unwrap()
                }) {
                    return Ok(0);
                } else {
                    error!("[RKNPU] ioctl failed: cmd={:#x}, arg={:#x}", cmd, arg);
                    return Err(AxError::InvalidInput);
                }
            }
            None => return Err(AxError::InvalidInput),
        }
    }

    fn mmap(&self) -> DeviceMmap {
        // 返回整个内存池的物理地址范围
        // 用户的 mmap offset 会被加到这个基地址上
        match (NPU_ALLOCATOR.get_bus_addr(), NPU_ALLOCATOR.get_pool_size()) {
            (Ok(bus_addr), Ok(size)) => {
                info!(
                    "[RKNPU] mmap: returning bus address range bus_addr=0x{:x}, size=0x{:x}",
                    bus_addr, size
                );
                DeviceMmap::Physical(PhysAddrRange::from_start_size(
                    PhysAddr::from(bus_addr as usize),
                    size,
                ))
            }
            _ => {
                warn!("[RKNPU] mmap: memory pool not initialized");
                DeviceMmap::None
            }
        }
    }
}
