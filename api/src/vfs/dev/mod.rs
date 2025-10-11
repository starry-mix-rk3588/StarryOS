//! Special devices

#[cfg(feature = "input")]
mod event;
mod fb;
#[cfg(feature = "dev-log")]
mod log;
mod r#loop;
#[cfg(feature = "memtrack")]
mod memtrack;
mod rtc;
pub mod tty;

use alloc::{format, sync::Arc};
use core::any::Any;

use axerrno::AxError;
use axfs_ng_vfs::{DeviceId, Filesystem, NodeFlags, NodeType, VfsResult};
use axsync::Mutex;
#[cfg(feature = "dev-log")]
pub use log::bind_dev_log;
use rand::{RngCore, SeedableRng, rngs::SmallRng};
use starry_core::vfs::{Device, DeviceOps, DirMaker, DirMapping, SimpleDir, SimpleFs};

const RANDOM_SEED: &[u8; 32] = b"0123456789abcdef0123456789abcdef";

pub(crate) fn new_devfs() -> Filesystem {
    SimpleFs::new_with("devfs".into(), 0x01021994, builder)
}

struct Null;

impl DeviceOps for Null {
    fn read_at(&self, _buf: &mut [u8], _offset: u64) -> VfsResult<usize> {
        Ok(0)
    }

    fn write_at(&self, buf: &[u8], _offset: u64) -> VfsResult<usize> {
        Ok(buf.len())
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn flags(&self) -> NodeFlags {
        NodeFlags::NON_CACHEABLE | NodeFlags::STREAM
    }
}

struct Zero;

impl DeviceOps for Zero {
    fn read_at(&self, buf: &mut [u8], _offset: u64) -> VfsResult<usize> {
        buf.fill(0);
        Ok(buf.len())
    }

    fn write_at(&self, buf: &[u8], _offset: u64) -> VfsResult<usize> {
        Ok(buf.len())
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn flags(&self) -> NodeFlags {
        NodeFlags::NON_CACHEABLE | NodeFlags::STREAM
    }
}

struct Random {
    rng: Mutex<SmallRng>,
}

impl Random {
    pub fn new() -> Self {
        Self {
            rng: Mutex::new(SmallRng::from_seed(*RANDOM_SEED)),
        }
    }
}

impl DeviceOps for Random {
    fn read_at(&self, buf: &mut [u8], _offset: u64) -> VfsResult<usize> {
        self.rng.lock().fill_bytes(buf);
        Ok(buf.len())
    }

    fn write_at(&self, buf: &[u8], _offset: u64) -> VfsResult<usize> {
        Ok(buf.len())
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn flags(&self) -> NodeFlags {
        NodeFlags::NON_CACHEABLE | NodeFlags::STREAM
    }
}

struct Full;

impl DeviceOps for Full {
    fn read_at(&self, buf: &mut [u8], _offset: u64) -> VfsResult<usize> {
        buf.fill(0);
        Ok(buf.len())
    }

    fn write_at(&self, _buf: &[u8], _offset: u64) -> VfsResult<usize> {
        Err(AxError::StorageFull)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn flags(&self) -> NodeFlags {
        NodeFlags::NON_CACHEABLE | NodeFlags::STREAM
    }
}

struct CpuDmaLatency;

impl DeviceOps for CpuDmaLatency {
    fn read_at(&self, _buf: &mut [u8], _offset: u64) -> VfsResult<usize> {
        Err(AxError::InvalidInput)
    }

    fn write_at(&self, buf: &[u8], _offset: u64) -> VfsResult<usize> {
        Ok(buf.len())
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn flags(&self) -> NodeFlags {
        NodeFlags::NON_CACHEABLE
    }
}

// mod drm;
mod rknpu;
use core::slice;

use axdma::alloc_coherent;
// use drm::*;
use starry_vm::{VmMutPtr, vm_write_slice};
use rknpu::card1::Card1;
use rknpu::Card;

#[repr(C)]
pub struct RknpuAction {
    pub flags: u32,
    pub value: u32,
}

fn builder(fs: Arc<SimpleFs>) -> DirMaker {
    let mut root = DirMapping::new();
    root.add(
        "null",
        Device::new(
            fs.clone(),
            NodeType::CharacterDevice,
            DeviceId::new(1, 3),
            Arc::new(Null),
        ),
    );
    root.add(
        "zero",
        Device::new(
            fs.clone(),
            NodeType::CharacterDevice,
            DeviceId::new(1, 5),
            Arc::new(Zero),
        ),
    );
    root.add(
        "full",
        Device::new(
            fs.clone(),
            NodeType::CharacterDevice,
            DeviceId::new(1, 7),
            Arc::new(Full),
        ),
    );
    root.add(
        "random",
        Device::new(
            fs.clone(),
            NodeType::CharacterDevice,
            DeviceId::new(1, 8),
            Arc::new(Random::new()),
        ),
    );
    root.add(
        "urandom",
        Device::new(
            fs.clone(),
            NodeType::CharacterDevice,
            DeviceId::new(1, 9),
            Arc::new(Random::new()),
        ),
    );
    root.add(
        "rtc0",
        Device::new(
            fs.clone(),
            NodeType::CharacterDevice,
            rtc::RTC0_DEVICE_ID,
            Arc::new(rtc::Rtc),
        ),
    );
    if axdisplay::has_display() {
        root.add(
            "fb0",
            Device::new(
                fs.clone(),
                NodeType::CharacterDevice,
                DeviceId::new(29, 0),
                Arc::new(fb::FrameBuffer::new()),
            ),
        );
    }

    root.add(
        "tty",
        Device::new(
            fs.clone(),
            NodeType::CharacterDevice,
            DeviceId::new(5, 0),
            Arc::new(tty::CurrentTty),
        ),
    );
    root.add(
        "console",
        Device::new(
            fs.clone(),
            NodeType::CharacterDevice,
            DeviceId::new(5, 1),
            tty::N_TTY.clone(),
        ),
    );

    root.add(
        "ptmx",
        Device::new(
            fs.clone(),
            NodeType::CharacterDevice,
            DeviceId::new(5, 2),
            Arc::new(tty::Ptmx(fs.clone())),
        ),
    );
    root.add(
        "pts",
        SimpleDir::new_maker(fs.clone(), Arc::new(tty::PtsDir)),
    );
    #[cfg(feature = "dev-log")]
    root.add(
        "log",
        starry_core::vfs::SimpleFile::new(fs.clone(), NodeType::Socket, || Ok(b"")),
    );

    #[cfg(feature = "memtrack")]
    root.add(
        "memtrack",
        Device::new(
            fs.clone(),
            NodeType::CharacterDevice,
            DeviceId::new(114, 514),
            Arc::new(memtrack::MemTrack),
        ),
    );

    root.add(
        "cpu_dma_latency",
        Device::new(
            fs.clone(),
            NodeType::CharacterDevice,
            DeviceId::new(10, 1024),
            Arc::new(CpuDmaLatency),
        ),
    );

    // This is mounted to a tmpfs in `new_procfs`
    root.add(
        "shm",
        SimpleDir::new_maker(fs.clone(), Arc::new(DirMapping::new())),
    );

    // Loop devices
    for i in 0..16 {
        let dev_id = DeviceId::new(7, 0);
        root.add(
            format!("loop{i}"),
            Device::new(
                fs.clone(),
                NodeType::BlockDevice,
                dev_id,
                Arc::new(r#loop::LoopDevice::new(i, dev_id)),
            ),
        );
    }

    // Input devices
    #[cfg(feature = "input")]
    root.add(
        "input",
        SimpleDir::new_maker(fs.clone(), Arc::new(event::input_devices(fs.clone()))),
    );

    let mut dri = DirMapping::new();
    dri.add(
        "card0",
        Device::new(
            fs.clone(),
            NodeType::CharacterDevice,
            DeviceId::new(10, 1024),
            Arc::new(Card),
        ),
    );

    dri.add(
        "card1",
        Device::new(
            fs.clone(),
            NodeType::CharacterDevice,
            DeviceId::new(10, 1024),
            Arc::new(Card1),
        ),
    );

    root.add("dri", SimpleDir::new_maker(fs.clone(), Arc::new(dri)));

    SimpleDir::new_maker(fs, Arc::new(root))
}

// 0xc0406400  < DRM_IOCTL_VERSION >
//
//
// cmd的大小为 32位，共分 4 个域：
//
// bit31~bit30   2位为 “区别读写” 区，作用是区分是读取命令还是写入命令。
// bit29~bit15   14位为 "数据大小" 区，表示 ioctl()中的 arg 变量传送的内存大小。
// bit14~bit08   8位为 “魔数"(也称为"幻数")区，这个值用以与其它设备驱动程序的
// ioctl 命令进行区别。 bit07~bit00   8位为 "区别序号"
// 区，是区分命令的命令顺序序号。
//
// 11       00 0000 01000000       0110 0100           0000 0000
//

// 0xc0106401 < DRM_IOCTL_GET_UNIQUE >
// 11       00 0000 00010000      0110 0100            0000 0001
//

// 0xc0086440 < DRM_IOCTL_QXL_ALLOC >
// 11       00 0000 00001000      0110 0100            0100 0000

// 0xc0106407 < unknow >
// 11       00 0000 00010000      0110 0100            0000 0111