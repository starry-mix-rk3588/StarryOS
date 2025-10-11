use bstr::BString;

pub const IOC_NRBITS: usize = 8;
pub const IOC_TYPEBITS: usize = 8;
pub const IOC_SIZEBITS: usize = 14;

pub const IOC_NRSHIFT: usize = 0;
pub const IOC_TYPESHIFT: usize = IOC_NRSHIFT + IOC_NRBITS;
pub const IOC_SIZESHIFT: usize = IOC_TYPESHIFT + IOC_TYPEBITS;
pub const IOC_DIRSHIFT: usize = IOC_SIZESHIFT + IOC_SIZEBITS;

pub const IOC_NONE: usize = 0;
pub const IOC_WRITE: usize = 1;
pub const IOC_READ: usize = 2;

pub const fn ioc(dir: usize, ty: usize, nr: usize, size: usize) -> usize {
    ((dir) << IOC_DIRSHIFT)
        | ((ty) << IOC_TYPESHIFT)
        | ((nr) << IOC_NRSHIFT)
        | ((size) << IOC_SIZESHIFT)
}

#[inline]
pub const fn iowr<T>(typ: usize, nr: usize) -> usize {
    ioc(IOC_READ | IOC_WRITE, typ, nr, core::mem::size_of::<T>())
}

// =========== DRM ==============

pub const DRM_IOCTL_BASE: usize = b'd' as usize;

#[inline]
pub const fn drm_iowr<T>(nr: usize) -> usize {
    iowr::<T>(DRM_IOCTL_BASE, nr)
}

#[repr(C)]
pub struct drm_version {
    pub version_major: i32,
    pub version_minor: i32,
    pub version_patchlevel: i32,
    pub name_len: usize,
    pub name: *mut u8,
    pub date_len: usize,
    pub date: *mut u8,
    pub desc_len: usize,
    pub desc: *mut u8,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct DrmVersion {
    pub version_major: i32,
    pub version_minor: i32,
    pub version_patchlevel: i32,
    pub name: BString,
    pub date: BString,
    pub desc: BString,
}

impl DrmVersion {
    pub fn new(
        version_major: u32,
        version_minor: u32,
        version_patch_level: u32,
        name: &'static str,
        date: &'static str,
        desc: &'static str,
    ) -> Self {
        DrmVersion {
            version_major: version_major as i32,
            version_minor: version_minor as i32,
            version_patchlevel: version_patch_level as i32,
            name: BString::from(name),
            date: BString::from(date),
            desc: BString::from(desc),
        }
    }
}

#[repr(C)]
pub struct DrmUnique {
    pub unique_len: usize,
    pub unique: *mut u8,
}

impl DrmUnique {
    pub fn new(unique: &'static str) -> Self {
        DrmUnique {
            unique: unique.as_ptr() as *mut u8,
            unique_len: unique.len(),
        }
    }
}

#[repr(C)]
pub struct DrmQxlAlloc {
    pub size: u32,
    pub handle: u32,
}

pub const DRM_IOCTL_VERSION: usize = drm_iowr::<drm_version>(0x00);

pub const DRM_IOCTL_GET_UNIQUE: usize = drm_iowr::<DrmUnique>(0x1);

// =========== RKNPU ==============

// DRM_COMMAND_BASE is 0x40
pub const DRM_COMMAND_BASE: usize = 0x40;

// RKNPU commands offset from DRM_COMMAND_BASE
pub const RKNPU_ACTION: usize = 0x00;
pub const RKNPU_SUBMIT: usize = 0x01;
pub const RKNPU_MEM_CREATE: usize = 0x02;
pub const RKNPU_MEM_MAP: usize = 0x03;
pub const RKNPU_MEM_DESTROY: usize = 0x04;
pub const RKNPU_MEM_SYNC: usize = 0x05;

// RKNPU actions
pub const RKNPU_GET_HW_VERSION: u32 = 0;
pub const RKNPU_GET_DRV_VERSION: u32 = 1;
pub const RKNPU_GET_FREQ: u32 = 2;
pub const RKNPU_SET_FREQ: u32 = 3;
pub const RKNPU_GET_VOLT: u32 = 4;
pub const RKNPU_SET_VOLT: u32 = 5;
pub const RKNPU_ACT_RESET: u32 = 6;
pub const RKNPU_GET_IOMMU_EN: u32 = 18;
pub const RKNPU_POWER_ON: u32 = 20;
pub const RKNPU_POWER_OFF: u32 = 21;
pub const RKNPU_GET_TOTAL_SRAM_SIZE: u32 = 22;
pub const RKNPU_GET_FREE_SRAM_SIZE: u32 = 23;

// RKNPU driver version: 0.9.3 (>= 0.2.1 required by librknnrt)
pub const RKNPU_DRIVER_VERSION_MAJOR: u32 = 0;
pub const RKNPU_DRIVER_VERSION_MINOR: u32 = 9;
pub const RKNPU_DRIVER_VERSION_PATCHLEVEL: u32 = 3;

#[inline]
pub const fn rknpu_get_drv_version_code(major: u32, minor: u32, patchlevel: u32) -> u32 {
    major * 10000 + minor * 100 + patchlevel
}

#[repr(C)]
pub struct RknpuAction {
    pub flags: u32,
    pub value: u32,
}

#[repr(C)]
pub struct RknpuMemCreate {
    pub handle: u32,
    pub flags: u32,
    pub size: u64,
    pub obj_addr: u64,
    pub dma_addr: u64,
    pub sram_size: u64,
}

pub const DRM_IOCTL_RKNPU_ACTION: usize = drm_iowr::<RknpuAction>(DRM_COMMAND_BASE + RKNPU_ACTION);
pub const DRM_IOCTL_RKNPU_MEM_CREATE: usize =
    drm_iowr::<RknpuMemCreate>(DRM_COMMAND_BASE + RKNPU_MEM_CREATE);
