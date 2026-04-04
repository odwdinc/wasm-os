#![no_std]
#![no_main]

mod drivers;
mod fs;
mod interrupts;
mod keyboard;
mod scheduler;
mod shell;
mod vga;
mod wasm;

use bootloader_api::{entry_point, BootInfo, BootloaderConfig};
use core::panic::PanicInfo;

const BOOTLOADER_CONFIG: BootloaderConfig = {
    let mut c = BootloaderConfig::new_default();
    c.kernel_stack_size = 512 * 1024; // 512 KiB — Instance+Interpreter are large stack types
    c
};

entry_point!(kernel_main, config = &BOOTLOADER_CONFIG);

// ---------------------------------------------------------------------------
// Macros
// ---------------------------------------------------------------------------

#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => {{
        $crate::vga::_print(format_args!($($arg)*));
        $crate::drivers::serial::_print(format_args!($($arg)*));
    }};
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
    drivers::serial::init();
    interrupts::init();
    wasm::engine::init_host_fns();

    if let Some(fb) = boot_info.framebuffer.as_mut() {
        let info = fb.info();
        let buf = fb.buffer_mut();
        vga::init(buf, info);
    }

    fs::register_file("hello.wasm",  wasm::engine::HELLO_WASM);
    fs::register_file("greet.wasm",  wasm::engine::GREET_WASM);
    fs::register_file("fib.wasm",    wasm::engine::FIB_WASM);
    fs::register_file("primes.wasm", wasm::engine::PRIMES_WASM);
    fs::register_file("collatz.wasm", wasm::engine::COLLATZ_WASM);
    fs::register_file("counter.wasm", wasm::engine::COUNTER_WASM);
    

    if let Err(e) = wasm::engine::run(wasm::engine::HELLO_WASM, "main", &[]) {
        println!("wasm boot error: {}", e.as_str());
    }

    println!("Type 'help' for commands.");
    scheduler::run();
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    loop {}
}
