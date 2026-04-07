//! Round-robin cooperative scheduler.
//!
//! [`run`] is the kernel's main loop, entered once at boot and never returning.
//! Each iteration:
//!
//! 1. Calls [`shell::input::poll_once`](crate::shell::input::poll_once) to
//!    handle one keyboard/serial key event (if any).
//! 2. Steps the next runnable WASM task via
//!    [`task::task_step`](crate::wasm::task::task_step) (round-robin).
//! 3. If both the shell and all tasks were idle, executes `hlt` to sleep
//!    until the next PIT interrupt (~10 ms at 100 Hz).
//!
//! The shell and all WASM tasks cooperate: the shell gets a turn every
//! loop iteration; tasks each get one step before the cursor advances to the
//! next slot.

use crate::wasm::task::{self, MAX_TASKS};
use crate::wasm::engine::TaskResult;

static mut CURSOR: usize = 0;

/// Start the cooperative scheduler loop.  Never returns (`-> !`).
///
/// Creates a [`ShellState`](crate::shell::input::ShellState) and then loops
/// forever, alternating between the shell and runnable WASM tasks.
pub fn run() -> ! {
    let mut shell = crate::shell::input::ShellState::new();

    loop {
        // ── Shell turn ────────────────────────────────────────────────────────
        let had_input = crate::shell::input::poll_once(&mut shell);

        // ── WASM task turn (round-robin) ──────────────────────────────────────
        let ran_task = run_next_task();

        // ── Idle: sleep until the next timer interrupt ────────────────────────
        if !had_input && !ran_task {
            unsafe { core::arch::asm!("hlt", options(nomem, nostack)); }
        }
    }
}

/// Find and step the next runnable task.  Returns true if a task was stepped.
fn run_next_task() -> bool {
    let start = unsafe { (CURSOR + 1) % MAX_TASKS };

    for i in 0..MAX_TASKS {
        let id = (start + i) % MAX_TASKS;
        if task::is_task_runnable(id) {
            unsafe { CURSOR = id; }
            match task::task_step(id) {
                Some(Ok(TaskResult::Completed(_))) => {
                    crate::println!("[task {}] done", id);
                }
                Some(Err(e)) => {
                    crate::println!("[task {}] error: {}", id, e.as_str());
                }
                _ => {}
            }
            return true;
        }
    }
    false
}
