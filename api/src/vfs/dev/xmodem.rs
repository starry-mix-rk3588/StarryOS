#![no_std]

// Rust port of xmodem_dw_uart.c (XMODEM-1K receiver, CRC mode)
// Keeps logic and behavior aligned with the original C implementation.

use core::ptr;
use core::arch::asm;
use core::ptr::{read_volatile, write_volatile};
use starry_core::vfs::{DeviceOps};
use axfs_ng_vfs::VfsResult;
use axfs_ng_vfs::VfsError;

// Control codes
const SOH: u8 = 0x01;
const STX: u8 = 0x02;
const EOT: u8 = 0x04;
const ACK: u8 = 0x06;
const NAK: u8 = 0x15;
const CAN: u8 = 0x18;
const CHAR_C: u8 = 0x43; // 'C'

const XMODEM_1K: usize = 1024;
const MAX_INIT_RETRIES: usize = 16;
const GETC_TIMEOUT_MS: u64 = 10_000; // 10 seconds

/* 优化这里 */
// =============================================================
const UART_BASE: usize = 0xffff_0000_feb5_0000;
const RBR: usize = 0x00; // 读
const THR: usize = 0x00; // 写
const IER: usize = 0x04;
const IIR: usize = 0x08; // 读
const FCR: usize = 0x08; // 写
const LCR: usize = 0x0c;
const MCR: usize = 0x10;
const LSR: usize = 0x14;
const MSR: usize = 0x18;
const SCR: usize = 0x1c;
const USR: usize = 0x7c; // DW 定义
const DLL: usize = 0x00; // DLAB=1 时访问
const DLM: usize = 0x04; // DLAB=1 时访问
const LSR_DR: u32 = 1 << 0;   // Data Ready
const LSR_THRE: u32 = 1 << 5; // Transmit Holding Register Empty
const LSR_TEMT: u32 = 1 << 6; // Transmitter Empty (THR & TSR)

#[inline(always)]
fn nop() { unsafe { asm!("nop", options(nomem, nostack, preserves_flags)) } }
#[inline(always)]
fn mmio_read32(addr: usize) -> u32 { unsafe { read_volatile(addr as *const u32) } }
#[inline(always)]
fn mmio_write32(val: u32, addr: usize) { unsafe { write_volatile(addr as *mut u32, val) } }

pub fn dw_uart_putchar(c: i8) {
    // Early：直接写寄存器
    while (mmio_read32(UART_BASE + LSR) & LSR_THRE) == 0 {
        nop();
    }
    mmio_write32(c as u32, UART_BASE + THR);
}

pub fn dw_uart_getchar_nb(c: *mut i8) -> bool {
    let lsr = unsafe { mmio_read32(UART_BASE + LSR) };
    if (lsr & LSR_DR) != 0 {
        // 有数据可读
        let val = unsafe { mmio_read32(UART_BASE + RBR) & 0xFF };
        unsafe {
            *c = val as i8;
        }
        true
    } else {
        false
    }
}

pub fn platform_get_time_ms() -> u64 {
    use core::arch::asm;

    // 静态缓存频率，避免每次读取 CNTFRQ_EL0
    static mut FREQ: u64 = 0;

    let freq: u64;
    unsafe {
        if FREQ == 0 {
            asm!("mrs {0}, cntfrq_el0", out(reg) FREQ);
            if FREQ == 0 {
                // 频率不可用，返回 0 避免除 0
                return 0;
            }
        }
        freq = FREQ;
    }

    let cnt: u64;
    unsafe {
        asm!("mrs {0}, cntpct_el0", out(reg) cnt);
    }

    // (cnt * 1000) / freq
    // 用 64-bit 拆分计算避免 __int128
    let secs_part = cnt / freq;
    let rem = cnt % freq;

    let mut ms = secs_part * 1000;
    ms += (rem * 1000) / freq;
    ms
}
// =============================================================
/* 优化这里 */

pub struct XmodemReceive;

#[inline]
fn uart_putc(c: u8) {
    unsafe { dw_uart_putchar(c as i8) }
}

// Return Some(byte) if received within timeout_ms, otherwise None
fn uart_getc_timeout(timeout_ms: u64) -> Option<u8> {
    let start = platform_get_time_ms();
    let deadline = start.saturating_add(timeout_ms);
    let mut ch: i8 = 0;
    while platform_get_time_ms() <= deadline {
        let ok = unsafe { dw_uart_getchar_nb(&mut ch as *mut i8) };
        if ok {
            return Some(ch as u8);
        }
    }
    None
}

// CRC16-CCITT (poly 0x1021), initial 0
fn crc16_ccitt(buf: &[u8]) -> u16 {
    let mut crc: u16 = 0;
    for &b in buf {
        crc ^= (b as u16) << 8;
        for _ in 0..8 {
            if (crc & 0x8000) != 0 {
                crc = (crc << 1) ^ 0x1021;
            } else {
                crc <<= 1;
            }
        }
    }
    crc
}

#[inline]
fn target_write(dst: *mut u8, src: *const u8, len: usize) {
    // Default: memcpy into RAM
    unsafe { ptr::copy_nonoverlapping(src, dst, len) }
}

pub fn xmodem_receive_1k(dst: *mut u8, maxlen: usize) -> isize {
    let mut expected_blk: u8 = 1;
    let mut write_offset: usize = 0;

    // Send initial 'C' up to MAX_INIT_RETRIES until we get a response
    let mut c: i32 = -1;
    for _try in 0..MAX_INIT_RETRIES {
        uart_putc(CHAR_C);
        if let Some(b) = uart_getc_timeout(GETC_TIMEOUT_MS) {
            c = b as i32;
            break;
        }
    }
    if c < 0 {
        return -1; // no response
    }

    loop {
        match c as u8 {
            EOT => {
                uart_putc(ACK);
                return write_offset as isize;
            }
            CAN => {
                return -1; // cancelled by remote
            }
            SOH | STX => {
                let block_size: usize = if (c as u8) == SOH { 128 } else { 1024 };
                let mut header_blk: u8;
                let mut header_blk_comp: u8;

                let b1 = match uart_getc_timeout(GETC_TIMEOUT_MS) {
                    Some(v) => v,
                    None => {
                        uart_putc(NAK);
                        return -1;
                    }
                };
                let b2 = match uart_getc_timeout(GETC_TIMEOUT_MS) {
                    Some(v) => v,
                    None => {
                        uart_putc(NAK);
                        return -1;
                    }
                };

                header_blk = b1;
                header_blk_comp = b2;
                if (header_blk.wrapping_add(header_blk_comp)) != 0xFF {
                    uart_putc(NAK);
                    c = match uart_getc_timeout(GETC_TIMEOUT_MS) {
                        Some(v) => v as i32,
                        None => -1,
                    };
                    continue;
                }

                // Read data
                let mut block_buf = [0u8; XMODEM_1K];
                for i in 0..block_size {
                    match uart_getc_timeout(GETC_TIMEOUT_MS) {
                        Some(v) => block_buf[i] = v,
                        None => {
                            uart_putc(NAK);
                            return -1;
                        }
                    }
                }

                // Read CRC hi/lo
                let hi = match uart_getc_timeout(GETC_TIMEOUT_MS) {
                    Some(v) => v,
                    None => {
                        uart_putc(NAK);
                        return -1;
                    }
                };
                let lo = match uart_getc_timeout(GETC_TIMEOUT_MS) {
                    Some(v) => v,
                    None => {
                        uart_putc(NAK);
                        return -1;
                    }
                };
                let crc_recv: u16 = ((hi as u16) << 8) | (lo as u16);

                // Verify block number
                if header_blk != expected_blk {
                    if header_blk == expected_blk.wrapping_sub(1) {
                        // duplicate block, ACK and fetch next
                        uart_putc(ACK);
                        c = match uart_getc_timeout(GETC_TIMEOUT_MS) {
                            Some(v) => v as i32,
                            None => -1,
                        };
                        continue;
                    } else {
                        uart_putc(NAK);
                        c = match uart_getc_timeout(GETC_TIMEOUT_MS) {
                            Some(v) => v as i32,
                            None => -1,
                        };
                        continue;
                    }
                }

                // Verify CRC
                let crc_calc = crc16_ccitt(&block_buf[..block_size]);
                if crc_calc != crc_recv {
                    uart_putc(NAK);
                    c = match uart_getc_timeout(GETC_TIMEOUT_MS) {
                        Some(v) => v as i32,
                        None => -1,
                    };
                    continue;
                }

                // Bounds check then write
                if write_offset + block_size <= maxlen {
                    unsafe {
                        target_write(dst.add(write_offset), block_buf.as_ptr(), block_size);
                    }
                    write_offset += block_size;
                } else {
                    // overflow -> cancel
                    uart_putc(CAN);
                    uart_putc(CAN);
                    return -1;
                }

                // success for this block
                uart_putc(ACK);
                expected_blk = expected_blk.wrapping_add(1);
                c = match uart_getc_timeout(GETC_TIMEOUT_MS) {
                    Some(v) => v as i32,
                    None => -1,
                };
                continue;
            }
            _ => {
                // Unexpected char; try to read next
                c = match uart_getc_timeout(GETC_TIMEOUT_MS) {
                    Some(v) => v as i32,
                    None => {
                        uart_putc(CAN);
                        uart_putc(CAN);
                        return -1;
                    }
                };
            }
        }
    }
}

impl DeviceOps for XmodemReceive {
    fn read_at(&self, buf: &mut [u8], _offset: u64) -> VfsResult<usize> {
        // XMODEM 协议接收数据到提供的缓冲区
        // offset 参数在 XMODEM 协议中没有意义，因为它是流式协议
        if buf.is_empty() {
            return Ok(0);
        }
        let result = xmodem_receive_1k(buf.as_mut_ptr(), buf.len());
        if result < 0 {
            Err(VfsError::BrokenPipe) // 或者其他适当的错误类型
        } else {
            Ok(result as usize)
        }
    }

    fn write_at(&self, _buf: &[u8], _offset: u64) -> VfsResult<usize> {
        // XMODEM 接收器不支持写操作
        Err(VfsError::PermissionDenied)
    }

    fn as_any(&self) -> &dyn core::any::Any {
        self
    }
}