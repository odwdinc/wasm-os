#![no_std]
extern crate alloc;

use alloc::vec::Vec;
use runes::cartridge::{BankType, Cartridge, MirrorType};
use runes::controller::{stdctl, InputPoller};
use runes::mapper::{Mapper, Mapper1, Mapper2, Mapper4, RefMapper};
use runes::memory::{CPUMemory, PPUMemory};
use runes::mos6502::CPU;
use runes::ppu::{PPU, Screen};
use runes::apu::{APU, Speaker};
use runes::utils;

// ── Global allocator ──────────────────────────────────────────────────────────
#[global_allocator]
static ALLOC: wee_alloc::WeeAlloc = wee_alloc::WeeAlloc::INIT;

// ── Panic handler ─────────────────────────────────────────────────────────────
#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    core::arch::wasm32::unreachable()
}

// ── Host function imports ─────────────────────────────────────────────────────
extern "C" {
    fn fb_blit(ptr: i32, width: i32, height: i32);
    fn sleep_ms(ms: i32);
    fn fs_size(name_ptr: i32, name_len: i32) -> i32;
    fn fs_read(name_ptr: i32, name_len: i32, buf_ptr: i32, buf_cap: i32) -> i32;
    fn print(ptr: i32, len: i32);
    fn print_int(n: i32);
    fn uptime_ms() -> i32;
    fn wasm_opcount() -> i64;
}

macro_rules! dbg_print {
    ($s:expr) => {
        unsafe { print($s.as_ptr() as i32, $s.len() as i32); }
    };
}
macro_rules! dbg_int {
    ($n:expr) => {
        unsafe { print_int($n as i32); }
    };
}

// ── NES palette (copied from runes/src/bin.rs) ────────────────────────────────
const RGB_COLORS: [u32; 64] = [
    0x666666, 0x002a88, 0x1412a7, 0x3b00a4, 0x5c007e, 0x6e0040, 0x6c0600,
    0x561d00, 0x333500, 0x0b4800, 0x005200, 0x004f08, 0x00404d, 0x000000,
    0x000000, 0x000000, 0xadadad, 0x155fd9, 0x4240ff, 0x7527fe, 0xa01acc,
    0xb71e7b, 0xb53120, 0x994e00, 0x6b6d00, 0x388700, 0x0c9300, 0x008f32,
    0x007c8d, 0x000000, 0x000000, 0x000000, 0xfffeff, 0x64b0ff, 0x9290ff,
    0xc676ff, 0xf36aff, 0xfe6ecc, 0xfe8170, 0xea9e22, 0xbcbe00, 0x88d800,
    0x5ce430, 0x45e082, 0x48cdde, 0x4f4f4f, 0x000000, 0x000000, 0xfffeff,
    0xc0dfff, 0xd3d2ff, 0xe8c8ff, 0xfbc2ff, 0xfec4ea, 0xfeccc5, 0xf7d8a5,
    0xe4e594, 0xcfef96, 0xbdf4ab, 0xb3f3cc, 0xb5ebf2, 0xb8b8b8, 0x000000,
    0x000000,
];

static mut FRAME_COUNT: u32 = 0;

// ── WasmScreen ────────────────────────────────────────────────────────────────
// Pixel buffer: 256×240 u32 values packed as little-endian 0x00RRGGBB.
// fb_blit reads it as bytes [B, G, R, 0] per pixel.
struct WasmScreen {
    buf: Vec<u32>,
}

impl WasmScreen {
    fn new() -> Self {
        WasmScreen { buf: alloc::vec![0u32; 256 * 240] }
    }
}

impl Screen for WasmScreen {
    #[inline(always)]
    fn put(&mut self, x: u8, y: u8, color: u8) {
        let rgb = RGB_COLORS[(color & 0x3f) as usize];
        // Store as little-endian: byte0=B, byte1=G, byte2=R, byte3=0
        // u32 0x00RRGGBB in little-endian memory → [BB, GG, RR, 00]
        self.buf[y as usize * 256 + x as usize] = rgb;
    }

    fn render(&mut self) {}

    fn frame(&mut self) {
        unsafe {
            fb_blit(self.buf.as_ptr() as i32, 256, 240);
            FRAME_COUNT += 1;
        }
    }
}

// ── NoAudio ───────────────────────────────────────────────────────────────────
struct NoAudio;

impl Speaker for NoAudio {
    fn queue(&mut self, _sample: i16) {}
}

// ── NoInputPoller ─────────────────────────────────────────────────────────────
struct NoInputPoller;

impl InputPoller for NoInputPoller {
    fn poll(&self) -> u8 { 0 }
}

// ── SimpleCart (copied from runes/src/bin.rs) ─────────────────────────────────
struct SimpleCart {
    chr_rom: Vec<u8>,
    prg_rom: Vec<u8>,
    sram:    Vec<u8>,
    mirror_type: MirrorType,
}

impl SimpleCart {
    fn new(chr_rom: Vec<u8>, prg_rom: Vec<u8>, sram: Vec<u8>, mirror_type: MirrorType) -> Self {
        SimpleCart { chr_rom, prg_rom, sram, mirror_type }
    }
}

impl Cartridge for SimpleCart {
    fn get_size(&self, kind: BankType) -> usize {
        match kind {
            BankType::PrgRom => self.prg_rom.len(),
            BankType::ChrRom => self.chr_rom.len(),
            BankType::Sram   => self.sram.len(),
        }
    }

    fn get_bank<'a>(&self, base: usize, size: usize, kind: BankType) -> &'a [u8] {
        unsafe {
            &*((&(match kind {
                BankType::PrgRom => &self.prg_rom,
                BankType::ChrRom => &self.chr_rom,
                BankType::Sram   => &self.sram,
            })[base..base + size]) as *const [u8])
        }
    }

    fn get_bank_mut<'a>(&mut self, base: usize, size: usize, kind: BankType) -> &'a mut [u8] {
        unsafe {
            &mut *((&mut (match kind {
                BankType::PrgRom => &mut self.prg_rom,
                BankType::ChrRom => &mut self.chr_rom,
                BankType::Sram   => &mut self.sram,
            })[base..base + size]) as *mut [u8])
        }
    }

    fn get_mirror_type(&self) -> MirrorType { self.mirror_type }
    fn set_mirror_type(&mut self, mt: MirrorType) { self.mirror_type = mt; }

    // Save-state stubs — not needed for emulation.
    fn load(&mut self, _reader: &mut dyn utils::Read) -> bool { false }
    fn save(&self, _writer: &mut dyn utils::Write) -> bool { false }
    fn load_sram(&mut self, _reader: &mut dyn utils::Read) -> bool { false }
    fn save_sram(&self, _writer: &mut dyn utils::Write) -> bool { false }
}

// ── ROM loading ───────────────────────────────────────────────────────────────
fn load_rom() -> Option<(Vec<u8>, Vec<u8>, Vec<u8>, MirrorType, u8)> {
    const NAME: &[u8] = b"game.nes";

    let size = unsafe {
        fs_size(NAME.as_ptr() as i32, NAME.len() as i32)
    };
    if size < 16 { return None; }

    let mut rom = Vec::with_capacity(size as usize);
    unsafe { rom.set_len(size as usize); }

    let read = unsafe {
        fs_read(
            NAME.as_ptr() as i32,
            NAME.len() as i32,
            rom.as_mut_ptr() as i32,
            size,
        )
    };
    if read != size { return None; }

    // Validate iNES magic.
    if &rom[0..4] != b"NES\x1a" { return None; }

    let prg_count = rom[4] as usize;   // 16 KiB units
    let chr_count = rom[5] as usize;   //  8 KiB units
    let flags6    = rom[6];
    let flags7    = rom[7];

    let mapper_id: u8 = (flags7 & 0xf0) | (flags6 >> 4);
    let mirror = if flags6 & 0x08 != 0 {
        MirrorType::Four
    } else if flags6 & 0x01 != 0 {
        MirrorType::Vertical
    } else {
        MirrorType::Horizontal
    };

    let trainer_size = if flags6 & 0x04 != 0 { 512 } else { 0 };
    let mut offset   = 16 + trainer_size;

    let prg_size = prg_count * 16384;
    let chr_size = chr_count *  8192;

    if offset + prg_size + chr_size > rom.len() { return None; }

    let prg_rom = rom[offset..offset + prg_size].to_vec();
    offset += prg_size;

    // CHR ROM or CHR RAM (8 KiB of zeroes if chr_count == 0).
    let chr_rom = if chr_size > 0 {
        rom[offset..offset + chr_size].to_vec()
    } else {
        alloc::vec![0u8; 8192]
    };

    let sram = alloc::vec![0u8; 8192];

    Some((prg_rom, chr_rom, sram, mirror, mapper_id))
}

// ── Entry point ───────────────────────────────────────────────────────────────
#[no_mangle]
pub extern "C" fn main() {
    dbg_print!("[nes] starting\n");

    let (prg_rom, chr_rom, sram, mirror, mapper_id) = match load_rom() {
        Some(x) => x,
        None    => {
            dbg_print!("[nes] load_rom failed\n");
            return;
        }
    };

    dbg_print!("[nes] ROM loaded, mapper=");
    dbg_int!(mapper_id as i32);
    dbg_print!("\n");

    let cart = SimpleCart::new(chr_rom, prg_rom, sram, mirror);

    // Declare mapper_box first so it outlives `mapper`.
    let mut mapper_box: alloc::boxed::Box<dyn Mapper> = match mapper_id {
        0 | 2 => alloc::boxed::Box::new(Mapper2::new(cart)),
        1     => alloc::boxed::Box::new(Mapper1::new(cart)),
        4     => alloc::boxed::Box::new(Mapper4::new(cart)),
        _     => {
            dbg_print!("[nes] unsupported mapper\n");
            return;
        }
    };

    dbg_print!("[nes] mapper ok, wiring CPU/PPU/APU\n");

    // Declare poller before p1ctl (p1ctl borrows poller).
    let poller = NoInputPoller;
    let p1ctl  = stdctl::Joystick::new(&poller);

    let mapper = RefMapper::new(&mut *mapper_box as &mut dyn Mapper);

    let mut screen  = WasmScreen::new();
    let mut speaker = NoAudio;

    let mut cpu = CPU::new(CPUMemory::new(&mapper, Some(&p1ctl), None));
    let mut ppu = PPU::new(PPUMemory::new(&mapper), &mut screen);
    let mut apu = APU::new(&mut speaker);

    let cpu_ptr = &mut cpu as *mut CPU;
    cpu.mem.bus.attach(cpu_ptr, &mut ppu, &mut apu);
    cpu.powerup();
    dbg_print!("[nes] powerup done, running\n");


    let mut frame_counter: u32 = 0;
    let mut last_report_frame: u32 = 0;
    let mut last_report_time: i32  = unsafe { uptime_ms() };
    let mut last_report_ops:  i64  = unsafe { wasm_opcount() };

    loop {
        let frame_start = unsafe { uptime_ms() };

        // Run exactly one NES frame worth of emulation.
        while frame_counter == unsafe { FRAME_COUNT } {
            while cpu.cycle > 0 {
                cpu.mem.bus.tick();
            }
            cpu.step();
        }
        frame_counter = unsafe { FRAME_COUNT };

        let now = unsafe { uptime_ms() };

        // Print timing stats once per 60 frames (~1 s at 60 fps).
        if frame_counter - last_report_frame >= 60 {
            let elapsed_total = now - last_report_time;
            let frames_done   = frame_counter - last_report_frame;
            // ms per frame = elapsed / frames (integer approximation)
            let ms_per_frame  = elapsed_total / frames_done as i32;

            let ops_now   = unsafe { wasm_opcount() };
            let ops_delta = ops_now - last_report_ops;
            // ops/frame and ops/sec (elapsed_total is ms, so *1000 → ops/sec)
            let ops_per_frame = if frames_done > 0 { ops_delta / frames_done as i64 } else { 0 };
            let ops_per_sec   = if elapsed_total > 0 { ops_delta * 1000 / elapsed_total as i64 } else { 0 };

            dbg_print!("[nes] frame=");
            dbg_int!(frame_counter as i32);
            dbg_print!(" ms/frame=");
            dbg_int!(ms_per_frame);
            dbg_print!(" ops/frame=");
            // ops/frame and ops/sec may exceed i32 — print as i32 truncated for now
            dbg_int!(ops_per_frame as i32);
            dbg_print!(" ops/sec=");
            dbg_int!(ops_per_sec as i32);
            dbg_print!(" (target 16ms)\n");

            last_report_frame = frame_counter;
            last_report_time  = now;
            last_report_ops   = ops_now;

        }

        // Yield to the cooperative scheduler so the shell and network stack
        // stay responsive between frames.
        unsafe { sleep_ms(0); }

        // If we finished the frame faster than 16 ms, sleep the remainder to
        // cap at ~60 fps and avoid busy-spinning.
        let elapsed = now - frame_start;
        if elapsed < 16 {
            unsafe { sleep_ms(16 - elapsed); }
        }
    }
}

