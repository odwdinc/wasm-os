//! Sprint F — JIT code-generation infrastructure.
//!
//! # Overview
//!
//! A single static 512 KiB buffer (`JIT_BUF`) holds all generated x86-64
//! machine code.  [`make_jit_executable`] walks the x86-64 page tables for
//! every 4 KiB page in the buffer, clears the NX bit (bit 63), and ensures
//! the writable bit (bit 1) is set, then flushes the TLB with `invlpg`.
//!
//! [`CodeBuf`] is a cursor into `JIT_BUF`.  It exposes low-level byte
//! emitters (`emit_u8`, `emit_u32`, `emit_u64`) plus higher-level helpers
//! for common x86-64 patterns (REX prefixes, MOV, PUSH/POP, RET) and a
//! standard System-V function prologue / epilogue.
//!
//! # Calling convention
//!
//! JIT'd functions use the System V AMD64 ABI:
//! - Arguments: RDI, RSI, RDX, RCX, R8, R9
//! - Callee-saved: RBP, RBX, R12–R15
//! - Returns: RAX
//!
//! The standard prologue pushed by [`CodeBuf::emit_prologue`]:
//! ```text
//!   push rbp
//!   mov  rbp, rsp
//!   push rbx
//!   push r12
//!   push r13
//!   push r14
//!   push r15
//!   sub  rsp, 8       ; keep 16-byte stack alignment (5 pushes + rbp = 6 → +8 pad)
//! ```
//! The matching epilogue:
//! ```text
//!   add  rsp, 8
//!   pop  r15
//!   pop  r14
//!   pop  r13
//!   pop  r12
//!   pop  rbx
//!   pop  rbp
//!   ret
//! ```

pub mod emit;
pub mod compile;
// ── JIT code buffer ───────────────────────────────────────────────────────────

/// Total size of the JIT code buffer in bytes.
pub const JIT_BUF_SIZE: usize = 2048 * 1024;

/// Static backing store for all generated machine code.
///
/// Placed in BSS; [`make_jit_executable`] marks the pages executable at
/// runtime by clearing NX bits in the page tables.
#[repr(C, align(4096))]
pub struct JitBuf([u8; JIT_BUF_SIZE]);

pub static mut JIT_BUF: JitBuf = JitBuf([0u8; JIT_BUF_SIZE]);

/// Current fill offset into [`JIT_BUF`].  Monotonically increasing.
static mut JIT_FILL: usize = 0;

/// Mark every 4 KiB page in [`JIT_BUF`] as present + writable + executable.
///
/// Clears the NX bit (bit 63) and sets the W bit (bit 1) in each 4 KiB PTE,
/// then issues `invlpg` to flush the TLB entry for that page.
///
/// Must be called once before any JIT'd function pointer is invoked.
/// Safe to call multiple times (idempotent).
pub fn make_jit_executable() {
    let base = unsafe { JIT_BUF.0.as_ptr() as usize };
    let pages = JIT_BUF_SIZE.div_ceil(4096);
    for i in 0..pages {
        let virt = base + i * 4096;
        if let Some((pte_phys, pte_val)) = crate::memory::find_pte(virt) {
            // Set present (0) + writable (1), clear NX (63).
            let new_pte = (pte_val | 0x3) & !(1u64 << 63);
            crate::memory::write_phys_u64(pte_phys, new_pte);
            // Flush TLB for this virtual page.
            unsafe {
                core::arch::asm!("invlpg [{}]", in(reg) virt, options(nostack, preserves_flags));
            }
        }
    }
}

/// Allocate `len` bytes from [`JIT_BUF`] and return a mutable pointer to the
/// start of the allocation, or `None` if the buffer is full.
///
/// The returned pointer is 16-byte aligned when `len` is a multiple of 16.
pub fn jit_alloc(len: usize) -> Option<*mut u8> {
    unsafe {
        let aligned = (JIT_FILL + 15) & !15;
        if aligned + len > JIT_BUF_SIZE {
            return None;
        }
        let ptr = JIT_BUF.0.as_mut_ptr().add(aligned);
        JIT_FILL = aligned + len;
        Some(ptr)
    }
}

/// How many bytes have been emitted so far.
pub fn jit_used() -> usize {
    unsafe { JIT_FILL }
}

/// Reset the fill cursor to zero (invalidates all previously emitted code).
pub fn jit_reset() {
    unsafe { JIT_FILL = 0; }
}
