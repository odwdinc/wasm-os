#![no_std]
#![no_main]

extern crate alloc;

mod drivers;
mod fs;
mod interrupts;
mod jit;
mod memory;
mod scheduler;
mod shell;
mod vga;
mod wasm;

use bootloader_api::{entry_point, BootInfo, BootloaderConfig};
use core::panic::PanicInfo;

const BOOTLOADER_CONFIG: BootloaderConfig = {
    let mut c = BootloaderConfig::new_default();
    // Stack budget (worst case — no NRVO through Result/map_err/? chain):
    //   Interpreter::new() frame : ~70 KiB (Self) + ~70 KiB (Result<Self,E> return slot)
    //                              + ~25 KiB (parse locals) = ~165 KiB
    //   spawn() frame            : ~70 KiB (Result<_,E1>) + ~70 KiB (after map_err)
    //                              + ~70 KiB (interp local) + ~4 KiB (host_fns) = ~214 KiB
    //   Rest of kernel call chain: ~20 KiB  →  ~400 KiB peak; 1024 KiB = 2.5× margin.
    c.kernel_stack_size = 1024 * 1024;
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
    println!("vga::init - framebuffer");

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
    println!("memory::init");
    memory::init(phys_mem_off);
    // Mount FAT filesystem — virtio-blk for true persistence, ramdisk fallback.
    let mounted_virtio = if let Some(blk) = drivers::virtio_blk::VirtioBlk::try_init() {
        if fs::fat::mount_virtio(blk) {
            println!("virtio-blk: FAT volume mounted");
            true
        } else {
            println!("virtio-blk: FAT mount failed, falling back to embedded image");
            false
        }
    } else {
        false
    };

    if !mounted_virtio {
        static FS_IMG: &[u8] = include_bytes!("../../fs.img");
        if fs::fat::mount_ramdisk(FS_IMG) {
            println!("ramdisk: FAT volume mounted ({} bytes)", FS_IMG.len());
        } else {
            println!("ramdisk: FAT mount failed (image may be empty)");
        }
    }

    // Populate the in-memory file table from the mounted FAT volume so that
    // the wasm engine can call `fs::find_file` with a `'static` slice.
    fs::load_fat_files_to_table();

    if let Some(hello) = fs::find_file("hello.wasm") {
        if let Err(e) = wasm::engine::run(hello, "main", &[]) {
            println!("wasm boot error: {}", e.as_str());
        }
    }
    println!("Type 'help' for commands.");
    scheduler::run();
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    println!("KERNEL PANIC: {}", info);
    loop {}
}
