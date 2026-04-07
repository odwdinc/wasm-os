//! 16550 UART driver — COM1 (I/O base 0x3F8), 115200 8N1, polled.
//!
//! Call `init()` once at boot.  After that:
//!   `write_byte` / `write_str` — output (mirrored from the print! macro)
//!   `read_byte`                 — non-blocking input (None if no data)

const COM1: u16 = 0x3F8;

// Register offsets (DLAB = 0 unless noted)
const OFF_DATA: u16 = 0; // Receive/Transmit data  (DLAB=0)
const OFF_IER:  u16 = 1; // Interrupt Enable Register
const OFF_FCR:  u16 = 2; // FIFO Control Register
const OFF_LCR:  u16 = 3; // Line Control Register
const OFF_MCR:  u16 = 4; // Modem Control Register
const OFF_LSR:  u16 = 5; // Line Status Register
const OFF_DLL:  u16 = 0; // Divisor Latch Low   (DLAB=1)
const OFF_DLH:  u16 = 1; // Divisor Latch High  (DLAB=1)

// LSR bits
const LSR_DATA_READY: u8 = 0x01; // received byte waiting

#[inline]
unsafe fn outb(port: u16, val: u8) {
    core::arch::asm!(
        "out dx, al",
        in("dx") port,
        in("al") val,
        options(nomem, nostack, preserves_flags)
    );
}

#[inline]
unsafe fn inb(port: u16) -> u8 {
    let val: u8;
    core::arch::asm!(
        "in al, dx",
        out("al") val,
        in("dx") port,
        options(nomem, nostack, preserves_flags)
    );
    val
}

/// Initialise COM1 at 115200 baud, 8N1, FIFOs enabled.
/// Safe to call multiple times (idempotent).
pub fn init() {
    unsafe {
        outb(COM1 + OFF_IER, 0x00); // disable all interrupts
        outb(COM1 + OFF_LCR, 0x80); // set DLAB to access divisor latches
        outb(COM1 + OFF_DLL, 0x01); // divisor = 1  →  115200 baud
        outb(COM1 + OFF_DLH, 0x00);
        outb(COM1 + OFF_LCR, 0x03); // 8 data bits, no parity, 1 stop bit; clear DLAB
        outb(COM1 + OFF_FCR, 0xC7); // enable FIFO, clear RX/TX, 14-byte threshold
        outb(COM1 + OFF_MCR, 0x0B); // RTS + DTR + Out2 (needed by some hypervisors)
    }
}

/// Transmit one byte.
/// QEMU's emulated 16550 accepts bytes instantly so we write unconditionally.
/// On real hardware this may occasionally drop a byte under very high output
/// rates; add LSR polling back when running on physical silicon.
pub fn write_byte(b: u8) {
    unsafe { outb(COM1 + OFF_DATA, b); }
}

/// Write a string, converting bare `\n` to `\r\n` for terminals.
pub fn write_str(s: &str) {
    for b in s.bytes() {
        if b == b'\n' {
            write_byte(b'\r');
        }
        write_byte(b);
    }
}

/// Called by the print! macro — formats `args` and writes to serial.
pub fn _print(args: core::fmt::Arguments) {
    use core::fmt::Write;
    struct Writer;
    impl core::fmt::Write for Writer {
        fn write_str(&mut self, s: &str) -> core::fmt::Result {
            write_str(s);
            Ok(())
        }
    }
    Writer.write_fmt(args).ok();
}

/// Non-blocking receive.  Returns `Some(byte)` if the UART has data, `None` otherwise.
pub fn read_byte() -> Option<u8> {
    unsafe {
        if inb(COM1 + OFF_LSR) & LSR_DATA_READY != 0 {
            Some(inb(COM1 + OFF_DATA))
        } else {
            None
        }
    }
}


// ── Key decoding ──────────────────────────────────────────────────────────────
use crate::drivers::keyboard::{ Key};
pub fn serial_byte_to_key(b: u8) -> Key {
    match b {
        0x08 | 0x7F       => Key::Backspace,
        0x0D | 0x0A       => Key::Enter,
        0x20..=0x7E       => Key::Char(b as char),
        _                 => Key::Unknown,
    }
}