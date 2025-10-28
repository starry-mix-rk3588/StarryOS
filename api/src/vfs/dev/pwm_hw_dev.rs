//! pwm_hw_device.rs - VFS wrapper for HwPwm (read/write/ioctl style)
//!
//! Adapted to RTP/RK3588 DTS node: pwm@fd8b0000
//! - Exposes text commands through DeviceOps write_at
//! - read_at returns status string
//!
//! IMPORTANT:
//! - HW_PWM_INSTANCE here is initialized with PWM0_BASE from pwm_hw.rs and a default input clock.
//! - You should replace `50_000_000` below with the actual input clock frequency resolved from DTS CRU
//!   (clocks = <...> "pwm" "pclk"). If you have a clock framework, call it during board init and pass real freq.

use core::str;
use lazy_static::lazy_static;
use crate::vfs::dev::pwm_hw::*;

use starry_core::vfs::DeviceOps;
use axfs_ng_vfs::VfsResult;
use kspin::SpinNoIrq;

// ioctl codes
pub const IOCTL_HW_PWM_ENABLE: u32 = 1;
pub const IOCTL_HW_PWM_DISABLE: u32 = 2;
pub const IOCTL_HW_PWM_SET_DUTY: u32 = 3;   // arg = (idx<<32) | duty_us
pub const IOCTL_HW_PWM_SET_PERIOD: u32 = 4;
pub const IOCTL_HW_PWM_RELEASE: u32 = 5;

lazy_static! {
    /// Create the hardware PWM instance here with base from DTS.
    /// Replace the second parameter with real input clock frequency (Hz).
    pub static ref HW_PWM_INSTANCE: SpinNoIrq<HwPwm> = {
        // Use base and a conservative default input clock; replace 24_000_000 if you know real value.
        let mut drv = HwPwm::new(PWM0_BASE, 24_000_000);
        // Optionally enable clocks here if you have platform helper available.
        // drv.enable_clock();
        SpinNoIrq::new(drv)
    };
}

pub struct HwPwmDevice;

impl HwPwmDevice {
    pub fn new() -> Self { Self }
}

impl DeviceOps for HwPwmDevice {
    fn read_at(&self, buf: &mut [u8], offset: u64) -> VfsResult<usize> {
        if offset > 0 {
            return Ok(0);
        }
        let s = HW_PWM_INSTANCE.lock().status_string();
        let b = s.as_bytes();
        let len = b.len().min(buf.len());
        buf[..len].copy_from_slice(&b[..len]);
        Ok(len)
    }

    fn write_at(&self, buf: &[u8], _offset: u64) -> VfsResult<usize> {
        let s = match str::from_utf8(buf) {
            Ok(v) => v.trim(),
            Err(_) => {
                debug!("hw_pwm_device: invalid utf8");
                return Ok(buf.len());
            }
        };

        let mut parts = s.split_whitespace();
        let cmd = parts.next().unwrap_or("");

        match cmd {
            "setup" => {
                if let (Some(pin_s), Some(per_s), Some(duty_s)) = (parts.next(), parts.next(), parts.next()) {
                    if let (Ok(pin), Ok(per), Ok(duty)) = (pin_s.parse::<u32>(), per_s.parse::<u32>(), duty_s.parse::<u32>()) {
                        let mut drv = HW_PWM_INSTANCE.lock();
                        match drv.setup_channel(pin, per, duty) {
                            Ok(idx) => debug!("hw_pwm setup ok idx={}", idx),
                            Err(e) => debug!("hw_pwm setup err: {}", e),
                        }
                    }
                }
            }
            "enable" => {
                if let Some(idx_s) = parts.next().and_then(|s| s.parse::<usize>().ok()) {
                    let mut drv = HW_PWM_INSTANCE.lock();
                    let _ = drv.enable_channel(idx_s);
                }
            }
            "disable" => {
                if let Some(idx_s) = parts.next().and_then(|s| s.parse::<usize>().ok()) {
                    let mut drv = HW_PWM_INSTANCE.lock();
                    let _ = drv.disable_channel(idx_s);
                }
            }
            "duty" => {
                if let (Some(idx_s), Some(duty_s)) = (parts.next(), parts.next()) {
                    if let (Ok(idx), Ok(duty)) = (idx_s.parse::<usize>(), duty_s.parse::<u32>()) {
                        let mut drv = HW_PWM_INSTANCE.lock();
                        let _ = drv.set_duty_us(idx, duty);
                    }
                }
            }
            "period" => {
                if let (Some(idx_s), Some(per_s)) = (parts.next(), parts.next()) {
                    if let (Ok(idx), Ok(per)) = (idx_s.parse::<usize>(), per_s.parse::<u32>()) {
                        let mut drv = HW_PWM_INSTANCE.lock();
                        let _ = drv.set_period_us(idx, per);
                    }
                }
            }
            "release" => {
                if let Some(idx_s) = parts.next().and_then(|s| s.parse::<usize>().ok()) {
                    let mut drv = HW_PWM_INSTANCE.lock();
                    let _ = drv.release_channel(idx_s);
                }
            }
            "tick_us" => {
                // not relevant for HW PWM but kept to be compatible with software API
                debug!("hw_pwm_device: tick_us ignored for hardware pwm");
            }
            _ => {
                debug!("hw_pwm_device: unknown cmd {}", cmd);
            }
        }

        Ok(buf.len())
    }

    fn as_any(&self) -> &dyn core::any::Any { self }
}

/// ioctl-like helper (callable from kernel/firmware)
pub fn ioctl_cmd(cmd: u32, arg: usize) -> VfsResult<i32> {
    let mut drv = HW_PWM_INSTANCE.lock();
    match cmd {
        IOCTL_HW_PWM_ENABLE => {
            let idx = arg as usize;
            match drv.enable_channel(idx) { Ok(()) => Ok(0), Err(_) => Ok(-1) }
        }
        IOCTL_HW_PWM_DISABLE => {
            let idx = arg as usize;
            match drv.disable_channel(idx) { Ok(()) => Ok(0), Err(_) => Ok(-1) }
        }
        IOCTL_HW_PWM_RELEASE => {
            let idx = arg as usize;
            match drv.release_channel(idx) { Ok(()) => Ok(0), Err(_) => Ok(-1) }
        }
        IOCTL_HW_PWM_SET_DUTY => {
            let idx = (arg >> 32) as usize;
            let duty = (arg & 0xffff_ffff) as u32;
            match drv.set_duty_us(idx, duty) { Ok(()) => Ok(0), Err(_) => Ok(-1) }
        }
        IOCTL_HW_PWM_SET_PERIOD => {
            let idx = (arg >> 32) as usize;
            let per = (arg & 0xffff_ffff) as u32;
            match drv.set_period_us(idx, per) { Ok(()) => Ok(0), Err(_) => Ok(-1) }
        }
        _ => {
            debug!("hw_pwm_device ioctl unknown {}", cmd);
            Ok(-1)
        }
    }
}