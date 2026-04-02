#![no_std]
#![no_main]

mod vga;

use bootloader_api::{entry_point, BootInfo};
use core::panic::PanicInfo;

entry_point!(kernel_main);

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
    ($($arg:tt)*) => {
        $crate::print!($($arg)*);
        $crate::print!("\n")
    };
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

    println!("Hello World");
    println!("Sprint 1.3: print! and println! working");

    loop {}
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    // Best-effort: print panic info if the writer is up.
    // The `_` suppresses "unused variable" if the print fails silently.
    let _ = info;
    loop {}
}
