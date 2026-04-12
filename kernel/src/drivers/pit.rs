//! PIT (8253) driver + 8259 PIC remapping.
//!
//! After `init()` the timer fires at ~1000 Hz and `ticks()` returns the
//! monotonically-increasing tick count.
//!
//! Call `calibrate_tsc()` once after `init()` to calibrate the TSC against
//! one PIT tick.  After calibration, `uptime_ms()` returns sub-millisecond
//! accurate elapsed time via RDTSC instead of the 10 ms-granular tick counter.

use core::sync::atomic::{AtomicU64, Ordering};

// ---------------------------------------------------------------------------
// Tick counter
// ---------------------------------------------------------------------------

static TICK_COUNT: AtomicU64 = AtomicU64::new(0);

/// Return the number of timer ticks since `init()` (~100 per second).
pub fn ticks() -> u64 {
    TICK_COUNT.load(Ordering::Relaxed)
}

// ---------------------------------------------------------------------------
// RDTSC-based high-resolution timer
// ---------------------------------------------------------------------------

/// TSC value captured at the start of the calibration tick.
static TSC_BOOT: AtomicU64 = AtomicU64::new(0);
/// Calibrated TSC frequency in MHz (cycles per microsecond).
/// Zero means not yet calibrated — `uptime_ms` falls back to PIT ticks.
static TSC_MHZ: AtomicU64 = AtomicU64::new(0);

#[inline]
fn rdtsc() -> u64 {
    let lo: u32;
    let hi: u32;
    unsafe {
        core::arch::asm!(
            "rdtsc",
            out("eax") lo,
            out("edx") hi,
            options(nomem, nostack, preserves_flags)
        );
    }
    ((hi as u64) << 32) | (lo as u64)
}

/// Return the calibrated TSC frequency in MHz (0 if not yet calibrated).
pub fn tsc_mhz() -> u64 {
    TSC_MHZ.load(Ordering::Relaxed)
}

/// Calibrate the TSC against one PIT tick (≈1 ms at 1000 Hz).
///
/// Spins waiting for two consecutive tick boundaries; the TSC delta over
/// that interval gives the CPU frequency.  Must be called after `init()`.
/// Safe to call multiple times — each call re-calibrates.
pub fn calibrate_tsc() {
    // Align to a tick boundary first, then measure the next full tick.
    let t0 = TICK_COUNT.load(Ordering::Relaxed);
    while TICK_COUNT.load(Ordering::Relaxed) == t0 {}

    let tsc_start = rdtsc();
    let t1 = TICK_COUNT.load(Ordering::Relaxed);
    while TICK_COUNT.load(Ordering::Relaxed) == t1 {}
    let tsc_end = rdtsc();

    // One PIT tick ≈ 1 ms.  MHz = cycles / 1_000 (cycles per µs).
    let delta = tsc_end.saturating_sub(tsc_start);
    let mhz   = (delta / 1_000).max(1); // guard against zero
    TSC_MHZ.store(mhz, Ordering::Relaxed);
    TSC_BOOT.store(tsc_start, Ordering::Relaxed);
}

/// Milliseconds elapsed since `calibrate_tsc()` was called.
///
/// Accurate to < 1 ms.  Falls back to `ticks() * 10` if not yet calibrated.
pub fn uptime_ms() -> i64 {
    let mhz = TSC_MHZ.load(Ordering::Relaxed);
    if mhz == 0 {
        return (TICK_COUNT.load(Ordering::Relaxed).saturating_mul(1))
            .min(i64::MAX as u64) as i64;
    }
    let boot = TSC_BOOT.load(Ordering::Relaxed);
    let elapsed_cycles = rdtsc().saturating_sub(boot);
    // ms = cycles / (MHz * 1_000)
    (elapsed_cycles / (mhz * 1_000)).min(i64::MAX as u64) as i64
}

// ---------------------------------------------------------------------------
// I/O port helpers
// ---------------------------------------------------------------------------

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
unsafe fn io_wait() {
    // Write to POST diagnostic port — tiny delay between PIC init commands.
    outb(0x80, 0);
}

// ---------------------------------------------------------------------------
// 8259 PIC — remap to 0x20-0x2F so IRQs don't collide with CPU exceptions
// ---------------------------------------------------------------------------

const PIC1_CMD:  u16 = 0x20;
const PIC1_DATA: u16 = 0x21;
const PIC2_CMD:  u16 = 0xA0;
const PIC2_DATA: u16 = 0xA1;

unsafe fn pic_remap() {
    // ICW1: start init sequence, ICW4 needed
    outb(PIC1_CMD, 0x11); io_wait();
    outb(PIC2_CMD, 0x11); io_wait();
    // ICW2: vector offsets — master 0x20-0x27, slave 0x28-0x2F
    outb(PIC1_DATA, 0x20); io_wait();
    outb(PIC2_DATA, 0x28); io_wait();
    // ICW3: master has slave on IRQ2; slave cascade identity = 2
    outb(PIC1_DATA, 0x04); io_wait();
    outb(PIC2_DATA, 0x02); io_wait();
    // ICW4: 8086 mode
    outb(PIC1_DATA, 0x01); io_wait();
    outb(PIC2_DATA, 0x01); io_wait();
    // IMR: unmask only IRQ0 (timer); mask everything else
    outb(PIC1_DATA, 0xFE); // 1111_1110
    outb(PIC2_DATA, 0xFF);
}

// ---------------------------------------------------------------------------
// 8253/8254 PIT — channel 0, ~100 Hz
// ---------------------------------------------------------------------------

const PIT_CHANNEL0: u16 = 0x40;
const PIT_CMD:      u16 = 0x43;
// 1 193 182 Hz / 1000 ≈ 1 193  (1 kHz → 1 ms per tick)
const PIT_DIVISOR: u16 = 1_193;

unsafe fn pit_init() {
    // Mode 2 (rate generator), lo/hi byte access, channel 0
    outb(PIT_CMD, 0x34);
    outb(PIT_CHANNEL0, (PIT_DIVISOR & 0xFF) as u8);
    outb(PIT_CHANNEL0, (PIT_DIVISOR >> 8)   as u8);
}

// ---------------------------------------------------------------------------
// ISR callback — called from the asm stub in interrupts::handlers
// ---------------------------------------------------------------------------

/// Increment tick counter and acknowledge the interrupt.
/// `#[no_mangle]` so the `global_asm!` stub can reference it by name.
#[no_mangle]
pub extern "C" fn pit_on_tick() {
    TICK_COUNT.fetch_add(1, Ordering::Relaxed);
    // Send End-Of-Interrupt to master PIC
    unsafe { outb(PIC1_CMD, 0x20); }
}

// ---------------------------------------------------------------------------
// Public init
// ---------------------------------------------------------------------------

/// Remap PIC, program PIT, enable interrupts.
/// Must be called after the IDT is loaded.
pub fn init() {
    unsafe {
        pic_remap();
        pit_init();
        core::arch::asm!("sti", options(nomem, nostack));
    }
}
