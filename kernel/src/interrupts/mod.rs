//! Interrupt subsystem — IDT setup + hardware timer init.

mod handlers;
mod idt;

/// Install the IDT and start the PIT.
/// Call this before enabling interrupts anywhere else.
pub fn init() {
    idt::set_gate(0x20, handlers::timer_isr_stub as *const () as u64);
    idt::load();
    crate::drivers::pit::init(); // programs PIC+PIT, executes `sti`
}
