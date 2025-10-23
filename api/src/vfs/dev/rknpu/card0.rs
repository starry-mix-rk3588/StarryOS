use crate::vfs::dev::*;
use rknpu_driver::{rknpu_ioctl, types::RkBoard, RknpuDev};
use axhal::mem::{phys_to_virt,  PhysAddr, pa};
use core::ptr::NonNull;

const RKNPU_CORE_BASE: PhysAddr = pa!(0xFDAB0000);
const RKNU_PMU1_BASE: PhysAddr = pa!(0xFD8D8000);
// static RKNPU: SpinNoIrq<RknpuDev> = SpinNoIrq::new(RknpuDev::new(phys_to_virt(RKNPU_CORE_BASE).as_usize(), RkBoard::Rk3588));
use lazy_static::lazy_static;

lazy_static! {
    static ref RKNPU: RknpuDev = {
        let mut dev = RknpuDev::new(phys_to_virt(RKNPU_CORE_BASE).as_usize(), RkBoard::Rk3588);
        let pmu_base = NonNull::new(phys_to_virt(RKNU_PMU1_BASE).as_mut_ptr()).unwrap();
        dev.initialize(pmu_base).unwrap();
        dev
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
        info!("card0 ioctl => cmd: {:#x}, arg: {:#x}", cmd, arg);

        if let Err(_error) = rknpu_ioctl(&RKNPU, cmd, arg) {
            // error!("card0 ioctl error => {}", error);
            // return Err(AxError::InvalidInput);
            todo!()
        }

        VfsResult::Ok(0)
    }
}
