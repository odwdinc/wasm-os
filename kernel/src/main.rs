#![no_std]
#![no_main]

mod drivers;
mod fs;
mod keyboard;
mod shell;
mod vga;
mod wasm;

use bootloader_api::{entry_point, BootInfo, BootloaderConfig};
use core::panic::PanicInfo;

const BOOTLOADER_CONFIG: BootloaderConfig = {
    let mut c = BootloaderConfig::new_default();
    c.kernel_stack_size = 256 * 1024; // 256 KiB — plenty for nested WASM calls
    c
};

entry_point!(kernel_main, config = &BOOTLOADER_CONFIG);

// ---------------------------------------------------------------------------
// Macros
// ---------------------------------------------------------------------------

#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => {
        $crate::vga::_print(format_args!($($arg)*))
    };
}

#[macro_export]
macro_rules! println {
    () => {
        $crate::print!("\n")
    };
    ($($arg:tt)*) => {{
        $crate::print!($($arg)*);
        $crate::print!("\n");
    }};
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

fn kernel_main(boot_info: &'static mut BootInfo) -> ! {
    if let Some(fb) = boot_info.framebuffer.as_mut() {
        let info = fb.info();
        let buf = fb.buffer_mut();
        vga::init(buf, info);
    }

    // Sprint 3.1: register embedded modules into the in-memory FS.
    fs::register_file("hello.wasm",  wasm::engine::HELLO_WASM);
    fs::register_file("greet.wasm",  wasm::engine::GREET_WASM);
    fs::register_file("fib.wasm",    wasm::engine::FIB_WASM);
    fs::register_file("primes.wasm", wasm::engine::PRIMES_WASM);

    // Sprint 2.5: auto-run the embedded WASM module on boot.
    if let Err(e) = wasm::engine::run(wasm::engine::HELLO_WASM, "main", &[]) {
        println!("wasm boot error: {}", e.as_str());
    }

    println!("Type 'help' for commands.");
    keyboard::run_loop();
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    loop {}
}
