//! ISR stubs — thin assembly wrappers that save caller-saved registers,
//! call the Rust handler, restore registers, and execute `iretq`.

use core::arch::global_asm;

// ---------------------------------------------------------------------------
// Timer ISR (IRQ0 → vector 0x20)
// ---------------------------------------------------------------------------
//
// Caller-saved registers (System V AMD64 ABI): rax, rcx, rdx, rsi, rdi,
// r8, r9, r10, r11.  We must preserve these across the Rust call.
//
// Stack on entry (no error code, no privilege change):
//   [rsp+16] rflags
//   [rsp+8]  cs
//   [rsp+0]  rip   ← rsp points here
//
// After 9 pushes (72 bytes) the stack is 16-byte-aligned before `call`.

global_asm!(
    ".global timer_isr_stub",
    "timer_isr_stub:",
    "push rax",
    "push rcx",
    "push rdx",
    "push rsi",
    "push rdi",
    "push r8",
    "push r9",
    "push r10",
    "push r11",
    "call pit_on_tick",
    "pop r11",
    "pop r10",
    "pop r9",
    "pop r8",
    "pop rdi",
    "pop rsi",
    "pop rdx",
    "pop rcx",
    "pop rax",
    "iretq",
);

extern "C" {
    pub fn timer_isr_stub();
}
