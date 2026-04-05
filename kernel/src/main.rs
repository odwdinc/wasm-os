#![no_std]
#![no_main]

mod drivers;
mod fs;
mod interrupts;
mod memory;
mod scheduler;
mod shell;
mod vga;
mod wasm;

use bootloader_api::{entry_point, BootInfo, BootloaderConfig};
use core::panic::PanicInfo;

const BOOTLOADER_CONFIG: BootloaderConfig = {
    let mut c = BootloaderConfig::new_default();
    c.kernel_stack_size = 512 * 1024; // 512 KiB — Instance+Interpreter are large stack types
    // Map all physical memory at a dynamic virtual address so the kernel can
    // walk page tables for accurate virtual→physical translation (needed for
    // virtio DMA ring setup).
    c.mappings.physical_memory = Some(bootloader_api::config::Mapping::Dynamic);
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
    // Explicitly zero critical statics — the bootloader does not zero all BSS
    // pages (physical pages reused from bootloader stages may contain garbage).
    fs::init();
    drivers::serial::init();
    interrupts::init();
    wasm::engine::init_host_fns();

    if let Some(fb) = boot_info.framebuffer.as_mut() {
        let info = fb.info();
        let buf = fb.buffer_mut();
        vga::init(buf, info);
    }

    // Initialise physical-address translation for virtio DMA ring setup.
    // The bootloader maps all physical memory at a dynamic virtual offset;
    // we use that to walk the hardware page tables for accurate virt→phys.
    let phys_mem_off = match boot_info.physical_memory_offset.into_option() {
        Some(off) => {
            println!("[mem] phys_mem_offset=0x{:x}", off);
            off
        }
        None => {
            println!("[mem] FATAL: physical_memory_offset not provided by bootloader");
            loop {}
        }
    };
    memory::init(phys_mem_off);
    // Try to mount the filesystem from the virtio-blk disk (true persistence).
    // Fall back to the compile-time embedded image if no virtio device is found.
    let virtio_files = if let Some(blk) = drivers::virtio_blk::VirtioBlk::try_init() {
        let n = fs::wasmfs::mount_from_blk(blk);
        println!("virtio-blk: mounted {} file(s)", n);
        n
    } else {
        0
    };

    if virtio_files == 0 {
        // No virtio disk (or empty disk) — fall back to the embedded fs.img.
        static FS_IMG: &[u8] = include_bytes!("../../fs.img");
        fs::wasmfs::mount_from_image(FS_IMG);
    }

    if let Some(hello) = fs::find_file("hello.wasm") {
        if let Err(e) = wasm::engine::run(hello, "main", &[]) {
            println!("wasm boot error: {}", e.as_str());
        }
    }

    println!("Type 'help' for commands.");
    scheduler::run();
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    loop {}
}
