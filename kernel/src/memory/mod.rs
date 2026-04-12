//! Memory subsystem: heap allocator and virtual→physical address translation.
//!
//! The bootloader maps all physical memory at a dynamic virtual offset
//! (`BOOTLOADER_CONFIG.mappings.physical_memory = Dynamic`).  [`init`] stores
//! that offset so that [`virt_to_phys`] can walk the 4-level x86-64 page
//! tables to find the physical address of any mapped virtual address.
//!
//! This page-table walk is necessary because BSS pages are allocated from the
//! bootloader's own frame pool — they are **not** located at a fixed offset
//! from the kernel's virtual base address — so a simple formula cannot be used.

pub mod allocator;

static mut PHYS_MEM_OFFSET: u64 = 0;

/// Store the physical-memory window base address from `BootInfo`.
///
/// Must be called exactly once at boot, before any call to [`virt_to_phys`].
pub fn init(phys_mem_offset: u64) {
    unsafe { PHYS_MEM_OFFSET = phys_mem_offset; }
}

/// Return the physical-memory window base set by [`init`].
///
/// Used by the JIT module to write page-table entries (clear NX, set W).
#[inline]
#[allow(dead_code)]
pub fn phys_mem_offset() -> u64 {
    unsafe { PHYS_MEM_OFFSET }
}

/// Read a `u64` from a physical address through the physical memory window.
#[inline]
#[allow(dead_code)]
fn read_phys_u64(phys: u64) -> u64 {
    let virt = unsafe { PHYS_MEM_OFFSET } + phys;
    unsafe { core::ptr::read_volatile(virt as *const u64) }
}

/// Write a `u64` to a physical address through the physical memory window.
#[inline]
#[allow(dead_code)]
pub fn write_phys_u64(phys: u64, val: u64) {
    let virt = unsafe { PHYS_MEM_OFFSET } + phys;
    unsafe { core::ptr::write_volatile(virt as *mut u64, val); }
}

/// Find the physical address of the PTE that maps `virt`, along with the
/// current PTE value.  Returns `None` if any level is not present.
///
/// Only handles 4 KiB pages (not 2 MiB / 1 GiB huge pages).
#[allow(dead_code)]
pub fn find_pte(virt: usize) -> Option<(u64 /*pte_phys*/, u64 /*pte_val*/)> {
    let v = virt as u64;
    let cr3: u64;
    unsafe { core::arch::asm!("mov {:r}, cr3", out(reg) cr3) };
    let pml4_phys = cr3 & !0xFFF_u64;

    let pml4e = read_phys_u64(pml4_phys + ((v >> 39) & 0x1FF) * 8);
    if pml4e & 1 == 0 { return None; }
    let pdpt_phys = pml4e & 0x000F_FFFF_FFFF_F000;

    let pdpte = read_phys_u64(pdpt_phys + ((v >> 30) & 0x1FF) * 8);
    if pdpte & 1 == 0 { return None; }
    if pdpte & (1 << 7) != 0 { return None; } // 1 GiB page — no PTE

    let pd_phys = pdpte & 0x000F_FFFF_FFFF_F000;
    let pde = read_phys_u64(pd_phys + ((v >> 21) & 0x1FF) * 8);
    if pde & 1 == 0 { return None; }
    if pde & (1 << 7) != 0 { return None; } // 2 MiB page — no PTE

    let pt_phys = pde & 0x000F_FFFF_FFFF_F000;
    let pte_phys = pt_phys + ((v >> 12) & 0x1FF) * 8;
    let pte_val  = read_phys_u64(pte_phys);
    Some((pte_phys, pte_val))
}

/// Translate a kernel virtual address to its physical address.
///
/// Walks the 4-level x86-64 page tables (PML4 → PDPT → PD → PT) via the
/// physical-memory window established by [`init`].  Supports 4 KiB, 2 MiB,
/// and 1 GiB pages.
///
/// # Panics / infinite loop
///
/// Loops forever if the address is not mapped (bare-metal equivalent of a
/// page-fault panic).
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
