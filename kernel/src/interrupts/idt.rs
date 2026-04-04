//! Minimal x86_64 IDT — 256 interrupt-gate entries.

// ---------------------------------------------------------------------------
// Entry layout (16 bytes, x86_64 interrupt gate)
// ---------------------------------------------------------------------------

#[repr(C, packed)]
#[derive(Clone, Copy)]
struct IdtEntry {
    offset_lo:  u16, // handler[0:15]
    selector:   u16, // kernel code segment
    ist:        u8,  // IST index (0 = current RSP)
    type_attr:  u8,  // 0x8E = present | DPL=0 | 64-bit interrupt gate
    offset_mid: u16, // handler[16:31]
    offset_hi:  u32, // handler[32:63]
    _reserved:  u32,
}

impl IdtEntry {
    const fn missing() -> Self {
        Self {
            offset_lo:  0,
            selector:   0,
            ist:        0,
            type_attr:  0,
            offset_mid: 0,
            offset_hi:  0,
            _reserved:  0,
        }
    }

    fn new(handler: u64) -> Self {
        Self {
            offset_lo:  (handler & 0xFFFF) as u16,
            selector:   0x08, // kernel CS (GDT index 1)
            ist:        0,
            type_attr:  0x8E, // P=1, DPL=0, type=0xE (64-bit interrupt gate)
            offset_mid: ((handler >> 16) & 0xFFFF) as u16,
            offset_hi:  (handler >> 32) as u32,
            _reserved:  0,
        }
    }
}

// ---------------------------------------------------------------------------
// The IDT itself
// ---------------------------------------------------------------------------

#[repr(C, align(16))]
struct Idt([IdtEntry; 256]);

static mut IDT: Idt = Idt([IdtEntry::missing(); 256]);

pub fn set_gate(vec: usize, handler: u64) {
    unsafe {
        IDT.0[vec] = IdtEntry::new(handler);
    }
}

// ---------------------------------------------------------------------------
// Load via `lidt`
// ---------------------------------------------------------------------------

#[repr(C, packed)]
struct IdtDescriptor {
    limit: u16,
    base:  u64,
}

pub fn load() {
    unsafe {
        let desc = IdtDescriptor {
            limit: (core::mem::size_of::<Idt>() - 1) as u16,
            base:  core::ptr::addr_of!(IDT) as u64,
        };
        core::arch::asm!(
            "lidt [{0}]",
            in(reg) &desc,
            options(readonly, nostack, preserves_flags)
        );
    }
}
