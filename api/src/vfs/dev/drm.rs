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

pub const DRM_IOCTL_BASE: usize = 'd' as usize;

#[inline]
pub const fn drm_iowr<T>(nr: usize) -> usize {
    iowr::<T>(DRM_IOCTL_BASE, nr)
}

#[repr(C)]
pub struct DrmVersion {
    pub version_major: u32,
    pub version_minor: u32,
    pub version_patch_level: u32,

    pub name_len: usize,
    pub name: *mut u8, // name of the driver

    pub date_len: usize,
    pub date: *mut u8, // buffer to hold date

    pub desc_len: usize,
    pub desc: *mut u8, // buffer to hold desc
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
        let name_len = name.len();
        let date_len = date.len();
        let desc_len = desc.len();

        DrmVersion {
            version_major,
            version_minor,
            version_patch_level,
            name_len,
            name: name.as_ptr() as *mut u8,
            date_len,
            date: date.as_ptr() as *mut u8,
            desc_len,
            desc: desc.as_ptr() as *mut u8,
        }
    }
}

pub const DRM_IOCTL_VERSION: usize = drm_iowr::<DrmVersion>(0x00);
