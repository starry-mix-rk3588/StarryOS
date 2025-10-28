use axfs_ng_vfs::VfsResult;
use axhal::mem::phys_to_virt;
use kspin::SpinNoIrq;
use lazy_static::lazy_static;
use memory_addr::pa;
use rkgpio::Gpio;
use starry_core::vfs::DeviceOps;

const GPIO_PADDR: usize = 0xfec40000;

lazy_static! {
    static ref GPIO: SpinNoIrq<Gpio> = {
        let mut gpio = Gpio::new(phys_to_virt(pa!(GPIO_PADDR)).as_mut_ptr());
        gpio.gpio_init();
        SpinNoIrq::new(gpio)
    };
}

pub struct GpioDevice;

impl GpioDevice {
    pub fn new() -> Self {
        Self
    }
}

impl DeviceOps for GpioDevice {
    fn read_at(&self, buf: &mut [u8], offset: u64) -> VfsResult<usize> {
        if offset > 0 {
            return Ok(0); // 返回 EOF
        }
        debug!(
            "read buf: {:?}, offset: {}, gpio: {}",
            buf,
            offset,
            GPIO.lock().gpio_read(6)
        );
        let gpio_level = GPIO.lock().gpio_read(6);

        let status_str = if gpio_level == 0 {
            b"gpio status: OFF\n"
        } else {
            b"gpio status: ON \n"
        };

        let len = status_str.len().min(buf.len());
        buf[..len].copy_from_slice(&status_str[..len]);
        Ok(len)
    }

    fn write_at(&self, buf: &[u8], offset: u64) -> VfsResult<usize> {
        debug!("write buf: {:?}, offset: {}", buf, offset);

        if buf.len() != 0 && buf[0] == 48 {
            // Turn off the LED
            GPIO.lock().gpio_setdir(6, 1);
            GPIO.lock().gpio_write(6, 0);
        } else if buf.len() != 0 {
            // Turn on the led
            GPIO.lock().gpio_setdir(6, 1);
            GPIO.lock().gpio_write(6, 1);
        }
        Ok(buf.len())
    }

    fn as_any(&self) -> &dyn core::any::Any {
        self
    }
}
