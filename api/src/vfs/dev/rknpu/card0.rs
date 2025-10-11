use super::{drm, drm::*};
use crate::vfs::dev::*;

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
        info!("card0 ioctl => cmd: {:#x}, arg: {:#x}", cmd, arg);
        let cmd: usize = cmd as usize;

        // 添加已知命令的标识，方便调试
        if cmd == DRM_IOCTL_VERSION {
            info!("  -> DRM_IOCTL_VERSION ({:#x})", DRM_IOCTL_VERSION);
        } else if cmd == DRM_IOCTL_GET_UNIQUE {
            info!("  -> DRM_IOCTL_GET_UNIQUE ({:#x})", DRM_IOCTL_GET_UNIQUE);
        } else if cmd == DRM_IOCTL_RKNPU_ACTION {
            info!(
                "  -> DRM_IOCTL_RKNPU_ACTION ({:#x})",
                DRM_IOCTL_RKNPU_ACTION
            );
        } else if cmd == DRM_IOCTL_RKNPU_MEM_CREATE {
            info!(
                "  -> DRM_IOCTL_RKNPU_MEM_CREATE ({:#x})",
                DRM_IOCTL_RKNPU_MEM_CREATE
            );
        } else {
            info!("  -> UNKNOWN ioctl command!");
            info!("     DRM_IOCTL_VERSION = {:#x}", DRM_IOCTL_VERSION);
            info!("     DRM_IOCTL_GET_UNIQUE = {:#x}", DRM_IOCTL_GET_UNIQUE);
            info!(
                "     DRM_IOCTL_RKNPU_ACTION = {:#x}",
                DRM_IOCTL_RKNPU_ACTION
            );
            info!(
                "     DRM_IOCTL_RKNPU_MEM_CREATE = {:#x}",
                DRM_IOCTL_RKNPU_MEM_CREATE
            );
        }

        if cmd == DRM_IOCTL_VERSION {
            info!("DRM_IOCTL_VERSION...");

            let k_drm_version =
                DrmVersion::new(1, 6, 0, "rknpu", "20140818", "Rockchip NPU DRM Driver");
            let user_drm: &mut drm_version = unsafe { &mut *(arg as *mut drm_version) };

            // 获取前端传入的缓冲区容量和指针
            let name_capacity = user_drm.name_len;
            let date_capacity = user_drm.date_len;
            let desc_capacity = user_drm.desc_len;

            let name_addr = user_drm.name as usize;
            let date_addr = user_drm.date as usize;
            let desc_addr = user_drm.desc as usize;

            info!(
                "DRM_IOCTL_VERSION: capacity[name={}, date={}, desc={}], addr[name={:#x}, \
                 date={:#x}, desc={:#x}]",
                name_capacity, date_capacity, desc_capacity, name_addr, date_addr, desc_addr
            );

            // 设置版本信息
            user_drm.version_major = k_drm_version.version_major;
            user_drm.version_minor = k_drm_version.version_minor;
            user_drm.version_patchlevel = k_drm_version.version_patchlevel;

            // 设置实际需要的长度
            user_drm.name_len = k_drm_version.name.len();
            user_drm.date_len = k_drm_version.date.len();
            user_drm.desc_len = k_drm_version.desc.len();

            // 只有当前端提供了足够的缓冲区容量时，才写入实际数据
            if name_capacity >= k_drm_version.name.len()
                && date_capacity >= k_drm_version.date.len()
                && desc_capacity >= k_drm_version.desc.len()
                && name_addr != 0
                && date_addr != 0
                && desc_addr != 0
            {
                info!("Writing version data to user buffers");
                let name_slice: &[u8] = unsafe {
                    slice::from_raw_parts(k_drm_version.name.as_ptr(), k_drm_version.name.len())
                };
                let date_slice: &[u8] = unsafe {
                    slice::from_raw_parts(k_drm_version.date.as_ptr(), k_drm_version.date.len())
                };
                let desc_slice: &[u8] = unsafe {
                    slice::from_raw_parts(k_drm_version.desc.as_ptr(), k_drm_version.desc.len())
                };
                let _ = vm_write_slice(name_addr as *mut _, name_slice);
                let _ = vm_write_slice(date_addr as *mut _, date_slice);
                let _ = vm_write_slice(desc_addr as *mut _, desc_slice);
                info!("Version data written successfully");
            } else {
                info!("Capacity insufficient or pointers null, only returning lengths");
            }

            return VfsResult::Ok(0);
        } else if cmd == DRM_IOCTL_GET_UNIQUE {
            info!("DRM_IOCTL_GET_UNIQUE...");
            // move relevant information to Card structure.
            let mut k_drm_unique = DrmUnique::new("");

            let user_drm: &mut DrmUnique = unsafe { &mut *(arg as *mut DrmUnique) };
            let unique_addr = user_drm.unique as usize;

            let unique_slice: &[u8] =
                unsafe { slice::from_raw_parts(k_drm_unique.unique, k_drm_unique.unique_len) };
            let _ = vm_write_slice(unique_addr as *mut _, unique_slice);

            k_drm_unique.unique = unique_addr as *mut u8;

            let _ = (arg as *mut DrmUnique).vm_write(k_drm_unique);
        } else if cmd == DRM_IOCTL_RKNPU_MEM_CREATE {
            info!("DRM_IOCTL_RKNPU_MEM_CREATE...");
            let user_mem: &mut RknpuMemCreate = unsafe { &mut *(arg as *mut RknpuMemCreate) };
            info!(
                "request size: {}, flags: {:#x}, handle: {}",
                user_mem.size, user_mem.flags, user_mem.handle
            );

            // TODO: 实际分配 DMA 内存
            let k_mem_create = RknpuMemCreate {
                handle: 0, // TODO: generate a unique handle
                flags: user_mem.flags,
                size: user_mem.size,
                obj_addr: 0,
                dma_addr: 0,
                sram_size: 0,
            };
            info!(
                "allocated size: {}, handle: {}",
                k_mem_create.size, k_mem_create.handle
            );
            let _ = (arg as *mut RknpuMemCreate).vm_write(k_mem_create);

            return VfsResult::Ok(0);
        } else if cmd == DRM_IOCTL_RKNPU_ACTION {
            info!("DRM_IOCTL_RKNPU_ACTION...");
            let user_action: &mut drm::RknpuAction =
                unsafe { &mut *(arg as *mut drm::RknpuAction) };
            info!("RKNPU action flags: {}", user_action.flags);

            match user_action.flags {
                drm::RKNPU_GET_DRV_VERSION => {
                    let version_code = drm::rknpu_get_drv_version_code(
                        drm::RKNPU_DRIVER_VERSION_MAJOR,
                        drm::RKNPU_DRIVER_VERSION_MINOR,
                        drm::RKNPU_DRIVER_VERSION_PATCHLEVEL,
                    );
                    info!(
                        "RKNPU_GET_DRV_VERSION: {}.{}.{} (code: {})",
                        drm::RKNPU_DRIVER_VERSION_MAJOR,
                        drm::RKNPU_DRIVER_VERSION_MINOR,
                        drm::RKNPU_DRIVER_VERSION_PATCHLEVEL,
                        version_code
                    );
                    user_action.value = version_code;
                }
                drm::RKNPU_GET_HW_VERSION => {
                    info!("RKNPU_GET_HW_VERSION");
                    // RK3588 NPU2 version (0x60002 means NPU version 6.0.2)
                    user_action.value = 0x60002;
                }
                drm::RKNPU_GET_IOMMU_EN => {
                    info!("RKNPU_GET_IOMMU_EN");
                    user_action.value = 1; // IOMMU enabled
                }
                drm::RKNPU_GET_TOTAL_SRAM_SIZE => {
                    info!("RKNPU_GET_TOTAL_SRAM_SIZE");
                    user_action.value = 0; // No SRAM for now
                }
                drm::RKNPU_GET_FREE_SRAM_SIZE => {
                    info!("RKNPU_GET_FREE_SRAM_SIZE");
                    user_action.value = 0;
                }
                _ => {
                    info!("Unsupported RKNPU action: {}", user_action.flags);
                }
            }

            return VfsResult::Ok(0);
        }
        VfsResult::Ok(0)
    }
}
