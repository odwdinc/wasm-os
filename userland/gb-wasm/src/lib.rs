#![no_std]
extern crate alloc;

use alloc::vec::Vec;
use alloc::vec;
use alloc::boxed::Box;

use rgy::{Config, Key, Stream, VRAM_HEIGHT, VRAM_WIDTH};

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

struct Hardware {
    display: Vec<Vec<u32>>,
}

impl Hardware {
    fn new() -> Self {
        // Create a frame buffer with the size VRAM_WIDTH * VRAM_HEIGHT.
        let display = vec![vec![0u32; VRAM_HEIGHT]; VRAM_WIDTH];

        Self { display }
    }
}

impl rgy::Hardware for Hardware {
    fn vram_update(&mut self, line: usize, buffer: &[u32]) {
        // `line` corresponds to the y coordinate.
        let y = line;

        for (x, col) in buffer.iter().enumerate() {
            self.display[x][y] = *col;
        }
        // unsafe {
        //     fb_blit(self.display.as_ptr() as i32, VRAM_HEIGHT.try_into().unwrap(), VRAM_WIDTH.try_into().unwrap());
        // }
    }

    fn joypad_pressed(&mut self, key: Key) -> bool {
        // Read a keyboard device and check if the `key` is pressed or not.
        // dbg_print!("[gb] Check if {:?} is pressed\n", key);
        false
    }

    fn sound_play(&mut self, _stream: Box<dyn Stream>) {
        // Play the wave provided `Stream`.
    }

    fn clock(&mut self) -> u64 {
    	let mut last_report_time: i32  = unsafe { uptime_ms() };
        last_report_time as u64
    }

    fn send_byte(&mut self, _b: u8) {
        // Send a byte to a serial port.
    }

    fn recv_byte(&mut self) -> Option<u8> {
        // Try to read a byte from a serial port.
        None
    }

    fn sched(&mut self) -> bool {
        // `true` to continue, `false` to stop the emulator.
        dbg_print!("[gb] It's running!\n");
        // Yield to the cooperative scheduler so the shell and network stack
        // stay responsive between frames.
        // unsafe { sleep_ms(0); }
        true
    }

    fn load_ram(&mut self, size: usize) -> Vec<u8> {
        // Return save data.
        vec![0; size]
    }

    fn save_ram(&mut self, _ram: &[u8]) {
        // Store save data.
    }
}

// ── ROM loading ───────────────────────────────────────────────────────────────
fn load_rom( name: &[u8]) -> Option<(Vec<u8>)> {
    let size = unsafe {
        fs_size(name.as_ptr() as i32, name.len() as i32)
    };
    if size < 16 { return None; }

    let mut rom = Vec::with_capacity(size as usize);
    unsafe { rom.set_len(size as usize); }

    let read = unsafe {
        fs_read(
            name.as_ptr() as i32,
            name.len() as i32,
            rom.as_mut_ptr() as i32,
            size,
        )
    };
    if read != size { return None; }
    let read = unsafe {
        fs_read(
            name.as_ptr() as i32,
            name.len() as i32,
            rom.as_mut_ptr() as i32,
            size,
        )
    };
    if read != size { return None; }
    core::prelude::v1::Some(rom)
}

// ── Entry point ───────────────────────────────────────────────────────────────
#[no_mangle]
pub extern "C" fn main(){
	dbg_print!("[gb] starting\n");
    let cfg = Config::new();
    const NAME: &[u8] = b"game.gb";
    let rom = match load_rom(NAME) {
        Some(x) => x,
        None    => {
            dbg_print!("[gb] load_rom failed\n");
            return;
        }
    };
    dbg_print!("[gb] ROM loaded:");
    dbg_print!(NAME);
    dbg_print!("\n");
    // // Create the hardware instance.
    let hw = Hardware::new();
    dbg_print!("[gb] running\n");
    rgy::run(cfg, &rom, hw);
}