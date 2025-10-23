use crate::vfs::dev::*;
use rk3588_rs::DrmVersion;
use rknpu_driver::{RknpuDev, types::RkBoard, rknpu_ioctl};
use axsync::Mutex;
use axhal::mem::{phys_to_virt, virt_to_phys};

static RKNPU: Mutex<Option<RknpuDev>> = Mutex::new(None);
const RKNPU_CORE_BASE: u64 = 0xFDAB0000;

fn get_or_init_npu() -> VfsResult<&'static RknpuDev> {
    let mut npu_lock = RKNPU.lock();
    if npu_lock.is_none() {
        let npu_base = unsafe {
            NonNull::new(phys_to_virt(RKNPU_CORE_BASE.into()).as_mut_ptr()).unwrap()
        };
        let npu_dev = RknpuDev::new(npu_base, RkBoard::Rk3588);
        npu_dev.initialize()?;
        *npu_lock = Some(npu_dev);
    }
    Ok(npu_lock.as_ref().unwrap())
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
        info!("card0 ioctl => cmd: {:#x}, arg: {:#x}", cmd, arg);

        let rknpu = get_or_init_npu()?;
        if let Err(error) = rknpu_ioctl(rknpu, cmd, arg) {
            error!("card0 ioctl error => {}", error);
            // return Err(AxError::InvalidInput);
        }

        VfsResult::Ok(0)
    }
}
