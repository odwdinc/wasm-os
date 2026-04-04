//! Sprint C.3 / C.5 — Round-Robin Cooperative Scheduler
//!
//! `run()` is the kernel's main loop (called instead of `keyboard::run_loop`).
//! Each iteration it:
//!   1. Calls `keyboard::poll_once` — handles one key event if available.
//!   2. Runs one step of the next runnable WASM task (round-robin).
//!   3. If both were idle, executes `hlt` to sleep until the next PIT interrupt
//!      (~10 ms at 100 Hz) rather than busy-spinning.
//!
//! This means the shell and all WASM tasks share the CPU cooperatively:
//! the shell gets a turn on every iteration, and WASM tasks each get a turn
//! before cycling back to the shell.

use crate::wasm::task::{self, MAX_TASKS};
use crate::wasm::engine::TaskResult;

static mut CURSOR: usize = 0;

pub fn run() -> ! {
    let mut shell = crate::keyboard::ShellState::new();

    loop {
        // ── Shell turn ────────────────────────────────────────────────────────
        let had_input = crate::keyboard::poll_once(&mut shell);

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
