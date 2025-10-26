use alloc::collections::BTreeMap;
use core::{
    alloc::Layout,
    sync::atomic::{AtomicU32, Ordering},
};

use axdma::{DMAInfo, alloc_coherent, dealloc_coherent};
use axhal::mem::virt_to_phys;
use axsync::Mutex;
use memory_addr::{PAGE_SIZE_4K, VirtAddr, align_up_4k};
use rknpu_driver::{
    memory::NpuAllocator,
    types::{RkNpuError, RkNpuResult},
};

/// NPU 内存池大小：32MB
const NPU_MEMORY_POOL_SIZE: usize = 32 * 1024 * 1024;

/// 内存句柄信息
struct MemHandle {
    offset: usize,
    size: usize,
}

/// NPU 内存池管理器
struct MemoryPool {
    dma_info: DMAInfo,
    pool_size: usize,
    next_offset: usize,
    handles: BTreeMap<u32, MemHandle>,
    next_handle: AtomicU32,
}

unsafe impl Send for MemoryPool {}
unsafe impl Sync for MemoryPool {}

impl MemoryPool {
    fn new() -> RkNpuResult<Self> {
        let layout = Layout::from_size_align(NPU_MEMORY_POOL_SIZE, PAGE_SIZE_4K)
            .map_err(|_| RkNpuError::InvalidParameter)?;

        let dma_info = unsafe { alloc_coherent(layout) }.map_err(|_| RkNpuError::OutOfMemory)?;

        let virt_addr = dma_info.cpu_addr.as_ptr() as usize;
        let phys_addr = virt_to_phys(VirtAddr::from(virt_addr));
        let bus_addr = dma_info.bus_addr.as_u64();

        info!(
            "[NPU DMA] Allocated memory pool: {} MB, virt=0x{:x}, phys=0x{:x}, bus=0x{:x}",
            NPU_MEMORY_POOL_SIZE / (1024 * 1024),
            virt_addr,
            phys_addr,
            bus_addr
        );

        // 初始化内存池为零
        unsafe {
            core::ptr::write_bytes(virt_addr as *mut u8, 0, NPU_MEMORY_POOL_SIZE);
        }

        Ok(Self {
            dma_info,
            pool_size: NPU_MEMORY_POOL_SIZE,
            next_offset: 0,
            handles: BTreeMap::new(),
            next_handle: AtomicU32::new(1),
        })
    }

    fn create_handle(&mut self, size: usize) -> RkNpuResult<(u32, u64, u64)> {
        let handle = self.next_handle.fetch_add(1, Ordering::SeqCst);
        let aligned_size = align_up_4k(size);

        if self.next_offset + aligned_size > self.pool_size {
            error!(
                "[NPU DMA] Out of memory: requested={}, available={}",
                aligned_size,
                self.pool_size - self.next_offset
            );
            return Err(RkNpuError::OutOfMemory);
        }

        let offset = self.next_offset;
        self.next_offset += aligned_size;

        // 使用总线地址 (bus_addr) 而不是物理地址
        // NPU 硬件通过总线访问内存,需要使用 bus_addr
        let bus_base = self.dma_info.bus_addr.as_u64();
        let dma_addr = bus_base + offset as u64;
        let obj_addr = dma_addr; // obj_addr 和 dma_addr 相同

        debug!(
            "[NPU DMA] Created handle={}, offset=0x{:x}, size={}, bus_addr=0x{:x}",
            handle, offset, size, dma_addr
        );

        self.handles.insert(handle, MemHandle { offset, size });

        Ok((handle, obj_addr, dma_addr))
    }

    fn destroy_handle(&mut self, handle: u32) -> bool {
        if let Some(mem) = self.handles.remove(&handle) {
            debug!(
                "[NPU DMA] Destroyed handle={}, offset=0x{:x}, size={}",
                handle, mem.offset, mem.size
            );
            true
        } else {
            warn!("[NPU DMA] Attempted to destroy invalid handle={}", handle);
            false
        }
    }

    fn get_handle(&self, handle: u32) -> RkNpuResult<(u64, usize)> {
        let mem = self
            .handles
            .get(&handle)
            .ok_or(RkNpuError::InvalidParameter)?;

        Ok((mem.offset as u64, mem.size))
    }

    fn user_to_kernel_addr(&self, user_addr: usize) -> RkNpuResult<VirtAddr> {
        // user_addr 是总线地址 (bus_addr),需要转换为虚拟地址
        let bus_base = self.dma_info.bus_addr.as_u64() as usize;
        let bus_end = bus_base + self.pool_size;

        if user_addr < bus_base || user_addr >= bus_end {
            error!(
                "[NPU DMA] Invalid address conversion: user_addr=0x{:x}, bus_range=[0x{:x}, \
                 0x{:x})",
                user_addr, bus_base, bus_end
            );
            return Err(RkNpuError::InvalidParameter);
        }

        let offset = user_addr - bus_base;
        let virt_addr = self.dma_info.cpu_addr.as_ptr() as usize + offset;

        Ok(VirtAddr::from(virt_addr))
    }
}

impl Drop for MemoryPool {
    fn drop(&mut self) {
        let layout = Layout::from_size_align(self.pool_size, PAGE_SIZE_4K).expect("Invalid layout");

        unsafe {
            dealloc_coherent(self.dma_info, layout);
        }

        info!(
            "[NPU DMA] Freed memory pool: {} MB",
            self.pool_size / (1024 * 1024)
        );
    }
}

pub struct NpuDmaAllocator {
    pool: Mutex<Option<MemoryPool>>,
}

impl NpuDmaAllocator {
    pub fn new() -> Self {
        Self {
            pool: Mutex::new(None),
        }
    }

    fn ensure_pool_initialized(&self) -> RkNpuResult<()> {
        let mut pool = self.pool.lock();
        if pool.is_none() {
            *pool = Some(MemoryPool::new()?);
            info!("[NPU DMA] Memory pool initialized");
        }
        Ok(())
    }
}

impl NpuAllocator for NpuDmaAllocator {
    fn create_handle(&self, size: usize) -> RkNpuResult<(u32, u64, u64)> {
        self.ensure_pool_initialized()?;

        let mut pool = self.pool.lock();
        let pool = pool.as_mut().ok_or(RkNpuError::NotSupported)?;

        pool.create_handle(size)
    }

    fn destroy_handle(&self, handle: u32) -> bool {
        let mut pool = self.pool.lock();
        if let Some(ref mut pool) = *pool {
            pool.destroy_handle(handle)
        } else {
            false
        }
    }

    fn get_handle(&self, handle: u32) -> RkNpuResult<(u64, usize)> {
        let pool = self.pool.lock();
        let pool = pool.as_ref().ok_or(RkNpuError::NotSupported)?;

        pool.get_handle(handle)
    }

    fn user_to_kernel_addr(&self, user_addr: usize) -> RkNpuResult<VirtAddr> {
        let pool = self.pool.lock();
        let pool = pool.as_ref().ok_or(RkNpuError::NotSupported)?;

        pool.user_to_kernel_addr(user_addr)
    }
}

impl NpuDmaAllocator {
    /// 获取内存池的总线地址(用于 mmap)
    pub fn get_bus_addr(&self) -> RkNpuResult<u64> {
        let pool = self.pool.lock();
        let pool = pool.as_ref().ok_or(RkNpuError::NotSupported)?;
        Ok(pool.dma_info.bus_addr.as_u64())
    }

    /// 获取内存池的大小
    pub fn get_pool_size(&self) -> RkNpuResult<usize> {
        let pool = self.pool.lock();
        let pool = pool.as_ref().ok_or(RkNpuError::NotSupported)?;
        Ok(pool.pool_size)
    }
}
