//! Sprint C.3 — Round-Robin Cooperative Scheduler
//!
//! `tick()` is called from the keyboard input loop on every idle iteration
//! (i.e. when no keypress is waiting).  It runs one step of the next runnable
//! WASM task and returns.  If no task is runnable it executes `hlt` so the CPU
//! sleeps until the next PIT interrupt (~10 ms) rather than busy-spinning.
//!
//! Execution model
//! ───────────────
//! Tasks cooperate by calling the `env.yield` or `env.sleep_ms` host functions,
//! which cause the interpreter to return `Yielded`.  `task_step` catches this,
//! marks the task Suspended (or Sleeping), and control returns here.  The
//! scheduler then moves the cursor to the next ready task.
//!
//! This is a *cooperative* scheduler — preemption is NOT implemented yet (C.5).

use crate::wasm::task::{self, MAX_TASKS};
use crate::wasm::engine::TaskResult;

// Round-robin cursor: index of the task we ran last.
// SAFETY: single-core; no concurrent mutation.
static mut CURSOR: usize = 0;

/// Called from the idle branch of the keyboard input loop.
/// Runs one step of the next runnable task, or idles if there is nothing to do.
pub fn tick() {
    // Scan forward from cursor+1 to find the next runnable task.
    let start = unsafe { (CURSOR + 1) % MAX_TASKS };
    let mut found = None;

    for i in 0..MAX_TASKS {
        let id = (start + i) % MAX_TASKS;
        if task::is_task_runnable(id) {
            found = Some(id);
            break;
        }
    }

    if let Some(id) = found {
        unsafe { CURSOR = id; }
        match task::task_step(id) {
            Some(Ok(TaskResult::Completed(_))) => {
                crate::println!("[task {}] done", id);
            }
            Some(Err(e)) => {
                crate::println!("[task {}] error: {}", id, e.as_str());
            }
            _ => {} // Yielded or Sleeping — normal; no output
        }
    } else {
        // Nothing runnable right now.  Halt until the next timer interrupt so
        // we don't busy-spin.  The PIT fires at ~100 Hz, giving 10 ms latency
        // before we re-check.  Sleeping tasks will be woken on the next tick
        // whose counter has caught up to their wake_tick.
        unsafe { core::arch::asm!("hlt", options(nomem, nostack)); }
    }
}
