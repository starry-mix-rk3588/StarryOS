use alloc::collections::BTreeMap;
use alloc::vec::Vec;
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

/// NPU 内存池大小：256MB
const NPU_MEMORY_POOL_SIZE: usize = 256 * 1024 * 1024;

/// 内存句柄信息
struct MemHandle {
    offset: usize,
    size: usize,
}

/// NPU 内存池管理器
struct MemoryPool {
    dma_info: DMAInfo,
    pool_size: usize,
    free_regions: BTreeMap<usize, usize>, // offset -> size 的空闲区间映射
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
            free_regions: {
                let mut map = BTreeMap::new();
                map.insert(0, NPU_MEMORY_POOL_SIZE); // 整个池子初始都是空闲的
                map
            },
            handles: BTreeMap::new(),
            next_handle: AtomicU32::new(1),
        })
    }

    fn create_handle(&mut self, size: usize) -> RkNpuResult<(u32, u64, u64)> {
        let handle = self.next_handle.fetch_add(1, Ordering::SeqCst);
        let aligned_size = align_up_4k(size);

        // 查找第一个足够大的空闲区间（首次适应算法）
        let allocation = self
            .free_regions
            .iter()
            .find(|(_, region_size)| **region_size >= aligned_size)
            .map(|(offset, region_size)| (*offset, *region_size));

        if let Some((offset, region_size)) = allocation {
            // 从空闲区间映射中移除
            self.free_regions.remove(&offset);

            // 如果有剩余空间，重新插入剩余部分
            let remaining = region_size - aligned_size;
            if remaining > 0 {
                self.free_regions.insert(offset + aligned_size, remaining);
            }

            // 使用总线地址 (bus_addr) 而不是物理地址
            // NPU 硬件通过总线访问内存,需要使用 bus_addr
            let bus_base = self.dma_info.bus_addr.as_u64();
            let dma_addr = bus_base + offset as u64;
            let obj_addr = dma_addr; // obj_addr 和 dma_addr 相同

            debug!(
                "[NPU DMA] Created handle={}, offset=0x{:x}, size={}, bus_addr=0x{:x}, free_regions={}",
                handle, offset, size, dma_addr, self.free_regions.len()
            );

            self.handles.insert(handle, MemHandle { offset, size: aligned_size });

            Ok((handle, obj_addr, dma_addr))
        } else {
            error!(
                "[NPU DMA] Out of memory: requested={}, available fragments: {:?}",
                aligned_size,
                self.free_regions.values().collect::<Vec<_>>()
            );
            Err(RkNpuError::OutOfMemory)
        }
    }

    fn destroy_handle(&mut self, handle: u32) -> bool {
        if let Some(mem) = self.handles.remove(&handle) {
            let offset = mem.offset;
            let size = mem.size;

            debug!(
                "[NPU DMA] Destroying handle={}, offset=0x{:x}, size={}",
                handle, offset, size
            );

            // 尝试与前面和后面的空闲区间合并
            let mut merged_offset = offset;
            let mut merged_size = size;

            // 查找前面相邻的空闲区间
            if let Some((prev_offset, prev_size)) = self
                .free_regions
                .range(..offset)
                .next_back()
                .map(|(k, v)| (*k, *v))
            {
                if prev_offset + prev_size == offset {
                    // 可以与前面的区间合并
                    self.free_regions.remove(&prev_offset);
                    merged_offset = prev_offset;
                    merged_size += prev_size;
                    debug!(
                        "[NPU DMA] Merged with previous region: offset=0x{:x}, size={}",
                        prev_offset, prev_size
                    );
                }
            }

            // 查找后面相邻的空闲区间
            if let Some((next_offset, next_size)) = self
                .free_regions
                .range((offset + size)..)
                .next()
                .map(|(k, v)| (*k, *v))
            {
                if merged_offset + merged_size == next_offset {
                    // 可以与后面的区间合并
                    self.free_regions.remove(&next_offset);
                    merged_size += next_size;
                    debug!(
                        "[NPU DMA] Merged with next region: offset=0x{:x}, size={}",
                        next_offset, next_size
                    );
                }
            }

            // 插入合并后的空闲区间
            self.free_regions.insert(merged_offset, merged_size);

            debug!(
                "[NPU DMA] Final free region: offset=0x{:x}, size={}, total free regions: {}",
                merged_offset,
                merged_size,
                self.free_regions.len()
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
