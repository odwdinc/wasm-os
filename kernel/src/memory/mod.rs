// memory/mod.rs — memory subsystem (allocator + virtual→physical translation)

pub mod allocator;

// memory/mod.rs — virtual→physical address translation via page-table walk
//
// The bootloader maps all physical memory starting at `phys_mem_offset`
// (configured via BOOTLOADER_CONFIG.mappings.physical_memory = Dynamic).
//
// To translate a kernel virtual address to its physical address we walk the
// 4-level x86-64 page tables.  This is necessary because BSS pages are
// allocated by the bootloader from its own frame pool and are NOT placed at
// `kernel_phys_base + (virt - kernel_virt_base)` — only file-backed segment
// pages follow that formula.

static mut PHYS_MEM_OFFSET: u64 = 0;

/// Store the physical-memory window base from BootInfo.
/// Must be called once at boot (before any `virt_to_phys` call).
pub fn init(phys_mem_offset: u64) {
    unsafe { PHYS_MEM_OFFSET = phys_mem_offset; }
}

/// Read a `u64` from a physical address through the physical memory window.
#[inline]
fn read_phys_u64(phys: u64) -> u64 {
    let virt = unsafe { PHYS_MEM_OFFSET } + phys;
    unsafe { core::ptr::read_volatile(virt as *const u64) }
}

/// Translate a kernel virtual address to its physical address by walking the
/// hardware page tables.
///
/// Supports 4 KiB, 2 MiB, and 1 GiB pages.
/// Panics (loops forever on bare metal) if the address is not mapped.
pub fn virt_to_phys(virt: usize) -> u64 {
    let v = virt as u64;

    // CR3 holds the physical address of the PML4 table (low 12 bits = flags).
    let cr3: u64;
    unsafe { core::arch::asm!("mov {:r}, cr3", out(reg) cr3) };
    let pml4_phys = cr3 & !0xFFF_u64;

    // PML4 — bits 47:39
    let pml4e = read_phys_u64(pml4_phys + ((v >> 39) & 0x1FF) * 8);
    if pml4e & 1 == 0 { loop {} }
    let pdpt_phys = pml4e & 0x000F_FFFF_FFFF_F000;

    // PDPT — bits 38:30
    let pdpte = read_phys_u64(pdpt_phys + ((v >> 30) & 0x1FF) * 8);
    if pdpte & 1 == 0 { loop {} }
    if pdpte & (1 << 7) != 0 {
        // 1 GiB page
        return (pdpte & 0x000F_FFFF_C000_0000) | (v & 0x3FFF_FFFF);
    }
    let pd_phys = pdpte & 0x000F_FFFF_FFFF_F000;

    // PD — bits 29:21
    let pde = read_phys_u64(pd_phys + ((v >> 21) & 0x1FF) * 8);
    if pde & 1 == 0 { loop {} }
    if pde & (1 << 7) != 0 {
        // 2 MiB page
        return (pde & 0x000F_FFFF_FFE0_0000) | (v & 0x001F_FFFF);
    }
    let pt_phys = pde & 0x000F_FFFF_FFFF_F000;

    // PT — bits 20:12
    let pte = read_phys_u64(pt_phys + ((v >> 12) & 0x1FF) * 8);
    if pte & 1 == 0 { loop {} }

    (pte & 0x000F_FFFF_FFFF_F000) | (v & 0xFFF)
}
