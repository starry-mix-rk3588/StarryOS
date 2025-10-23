// pub mod card0;
// pub mod card1;
pub mod card0;
// pub(crate) mod device;

use crate::vfs::{
    DeviceOps,
    dev::{Any, AxError, NodeFlags, VfsResult},
};

pub struct Card;

impl DeviceOps for Card {
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

    fn ioctl(&self, _cmd: u32, _arg: usize) -> VfsResult<usize> {
        VfsResult::Ok(0)
    }
}
