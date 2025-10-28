//! pwm_hw.rs - Hardware PWM driver skeleton (Rust, no_std compatible)
//!
//! Adapted for RK3588 DTS node: pwm@fd8b0000
//!   reg = <0x00 0xfd8b0000 0x00 0x10>  -> base = 0xFD8B0000, size = 0x10
//!   clocks = <...> "pwm" "pclk"         -> driver must enable the block clock(s) before accessing registers
//!   #pwm-cells = <3>
//!
//! This file is a template: register offsets/bitfields are PLACEHOLDERS and MUST be replaced with real
//! offsets and bit definitions from RK3588 TRM or from an existing Linux driver (recommended).
//!
//! What I changed compared to the generic template:
//! - Set PWM0_BASE to 0xFD8B0000 and PWM_REG_SIZE to 0x10 per your DTS
//! - Default input clock set to 24_000_000 Hz (typical value) — update if actual pwm input clock differs.
//! - Left TODOs where hardware specifics (CRU/GRF/IOMUX and register layout) must be filled in.

#![no_std]

use core::ptr::{read_volatile, write_volatile};
use core::fmt::Write;
use alloc::string::String;

/// MMIO base comes from your DTS: pwm@fd8b0000
pub const PWM0_BASE: usize = 0xFD8B0000;
pub const PWM_REG_SIZE: usize = 0x10; // as in DTS: size 0x10

/// Example register offsets (PLACEHOLDERS).
/// Replace with RK3588 TRM/Linux driver register definitions.
const REG_CTRL: usize = 0x00;
const REG_PRESCALE: usize = 0x04;
const REG_PERIOD: usize = 0x08;
const REG_DUTY: usize = 0x0C;
const REG_ENABLE_BASE: usize = 0x10; // if controller has enable bits per channel (placeholder)

/// Assumed maximum channels per PWM block (adjust per hardware)
pub const HW_PWM_MAX_CHANNELS: usize = 4;

/// Default input clock for PWM block (Hz).
/// The DTS node lists two clocks "pwm" and "pclk". You should replace this
/// with the real input clock frequency after resolving clocks from CRU.
const DEFAULT_INPUT_CLK_HZ: u32 = 24_000_000;

#[derive(Clone, Copy)]
pub struct HwChannel {
    pub used: bool,
    pub channel_id: u8,
    pub pin: u32,
    pub period_us: u32,
    pub duty_us: u32,
    pub enabled: bool,
}

impl Default for HwChannel {
    fn default() -> Self {
        Self {
            used: false,
            channel_id: 0,
            pin: 0,
            period_us: 0,
            duty_us: 0,
            enabled: false,
        }
    }
}

/// Hardware PWM controller instance (single block)
pub struct HwPwm {
    base: usize,
    clk_enabled: bool,
    channels: [HwChannel; HW_PWM_MAX_CHANNELS],
    input_clk_hz: u32,
}

impl HwPwm {
    pub fn new(base: usize, input_clk_hz: u32) -> Self {
        Self {
            base,
            clk_enabled: false,
            channels: [HwChannel::default(); HW_PWM_MAX_CHANNELS],
            input_clk_hz: if input_clk_hz == 0 { DEFAULT_INPUT_CLK_HZ } else { input_clk_hz },
        }
    }

    fn reg_ptr(&self, offset: usize) -> *mut u32 {
        (self.base + offset) as *mut u32
    }

    fn read_reg(&self, offset: usize) -> u32 {
        unsafe { read_volatile(self.reg_ptr(offset)) }
    }

    fn write_reg(&self, offset: usize, val: u32) {
        unsafe { write_volatile(self.reg_ptr(offset), val) }
    }

    pub fn enable_clock(&mut self) {
        if self.clk_enabled { return; }
        // TODO: enable clocks via CRU / platform API
        self.clk_enabled = true;
    }

    pub fn configure_iomux_for_pin(&mut self, _pin: u32) {
        // TODO: implement pinmux configuration
    }

    fn compute_prescaler_and_period(&self, period_us: u32) -> Option<(u32, u32)> {
        if period_us == 0 { return None; }
        let ticks = (self.input_clk_hz as u64 * period_us as u64) / 1_000_000u64;
        if ticks == 0 { return None; }
        let mut presc: u32 = 0;
        let mut t = ticks;
        let max_count = 0xFFFF_FFFFu64;
        while t > max_count && presc < 31 {
            presc += 1;
            t = ticks >> presc;
        }
        if t == 0 || t > max_count { return None; }
        Some((presc, t as u32))
    }

    pub fn setup_channel(&mut self, pin: u32, period_us: u32, duty_us: u32) -> Result<usize, &'static str> {
        if period_us == 0 || duty_us > period_us {
            return Err("invalid period/duty");
        }

        let mut slot = None;
        for (i, ch) in self.channels.iter_mut().enumerate() {
            if !ch.used {
                slot = Some(i);
                break;
            }
        }
        let idx = slot.ok_or("no free hw channel")?;

        self.enable_clock();
        self.configure_iomux_for_pin(pin);

        let (presc, period_count) = self.compute_prescaler_and_period(period_us)
            .ok_or("cannot compute prescaler/period")?;
        let duty_count = ((period_count as u64 * duty_us as u64) / period_us as u64) as u32;

        // write registers (placeholder offsets)
        let per_ch_base = REG_PRESCALE + idx * 0x10;
        self.write_reg(per_ch_base + 0x00, presc);
        self.write_reg(per_ch_base + 0x04, period_count);
        self.write_reg(per_ch_base + 0x08, duty_count);

        // store metadata (mutable borrow happens after above writes)
        self.channels[idx] = HwChannel {
            used: true,
            channel_id: idx as u8,
            pin,
            period_us,
            duty_us,
            enabled: false,
        };

        Ok(idx)
    }

    /// Enable channel (start output). Fixed borrow issues: write registers before mutably updating channel state.
    pub fn enable_channel(&mut self, idx: usize) -> Result<(), &'static str> {
        if idx >= HW_PWM_MAX_CHANNELS { return Err("idx oob"); }
        if !self.channels[idx].used { return Err("not used"); }

        // write enable bit first (immutable borrow during call)
        let enable_reg = REG_ENABLE_BASE + (idx as usize * 4);
        self.write_reg(enable_reg, 1);

        // now update channel state mutably (no conflict)
        self.channels[idx].enabled = true;
        Ok(())
    }

    /// Disable channel
    pub fn disable_channel(&mut self, idx: usize) -> Result<(), &'static str> {
        if idx >= HW_PWM_MAX_CHANNELS { return Err("idx oob"); }
        if !self.channels[idx].used { return Err("not used"); }

        let enable_reg = REG_ENABLE_BASE + (idx as usize * 4);
        self.write_reg(enable_reg, 0);

        self.channels[idx].enabled = false;
        Ok(())
    }

    /// Set duty in microseconds
    pub fn set_duty_us(&mut self, idx: usize, duty_us: u32) -> Result<(), &'static str> {
        if idx >= HW_PWM_MAX_CHANNELS { return Err("idx oob"); }

        // Read necessary checks and values immutably first (no active mutable borrow)
        if !self.channels[idx].used { return Err("not used"); }
        let ch_period = self.channels[idx].period_us;
        if duty_us > ch_period { return Err("duty > period"); }

        // read period_count from HW (immutable borrow within method call)
        let per_ch_base = REG_PRESCALE + idx * 0x10;
        let period_count = self.read_reg(per_ch_base + 0x04);
        let duty_count = ((period_count as u64 * duty_us as u64) / ch_period as u64) as u32;

        // write new duty then update structure (no active mutable borrow during read_reg/write_reg)
        self.write_reg(per_ch_base + 0x08, duty_count);
        self.channels[idx].duty_us = duty_us;
        Ok(())
    }

    /// Set period in microseconds (recompute prescaler and period)
    pub fn set_period_us(&mut self, idx: usize, period_us: u32) -> Result<(), &'static str> {
        if idx >= HW_PWM_MAX_CHANNELS { return Err("idx oob"); }
        if !self.channels[idx].used { return Err("not used"); }
        if period_us == 0 { return Err("invalid period"); }

        // compute presc/period (no mutable borrow)
        let (presc, period_count) = self.compute_prescaler_and_period(period_us)
            .ok_or("cannot compute prescaler")?;

        // read existing duty_us (immutable read)
        let old_duty = self.channels[idx].duty_us;
        // calculate new duty_count proportional to new period
        let duty_count = ((period_count as u64 * old_duty as u64) / (self.channels[idx].period_us as u64)) as u32;

        // write new presc/period/duty (immutable borrow while writing)
        let per_ch_base = REG_PRESCALE + idx * 0x10;
        self.write_reg(per_ch_base + 0x00, presc);
        self.write_reg(per_ch_base + 0x04, period_count);
        self.write_reg(per_ch_base + 0x08, duty_count);

        // finally update metadata mutably
        self.channels[idx].period_us = period_us;
        Ok(())
    }

    /// Release channel
    pub fn release_channel(&mut self, idx: usize) -> Result<(), &'static str> {
        if idx >= HW_PWM_MAX_CHANNELS { return Err("idx oob"); }
        if !self.channels[idx].used { return Err("not used"); }

        // disable first
        let _ = self.disable_channel(idx);
        // then clear entry
        self.channels[idx] = HwChannel::default();

        Ok(())
    }

    pub fn status_string(&self) -> String {
        let mut s = String::new();
        let _ = write!(&mut s, "HwPwm base=0x{:x} clk_hz={} channels:\n", self.base, self.input_clk_hz);
        for (i, ch) in self.channels.iter().enumerate() {
            if ch.used {
                let _ = write!(
                    &mut s,
                    "ch{} pin={} used={} en={} per={} duty={}\n",
                    i, ch.pin, ch.used as u8, ch.enabled as u8, ch.period_us, ch.duty_us
                );
            } else {
                let _ = write!(&mut s, "ch{} unused\n", i);
            }
        }
        s
    }
}