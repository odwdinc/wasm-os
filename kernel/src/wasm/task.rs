//! Sprint C.2 — Task / Fiber Abstraction
//!
//! A Task owns an engine pool slot and tracks execution state for the
//! cooperative scheduler (Sprint C.3).  The interpreter's own fields
//! (frame stack, value stack, PC) serve as the saved execution state;
//! no separate snapshot is needed because the interpreter is not reset
//! between yield/resume cycles.

use super::engine::{self, RunError, TaskResult, MAX_INSTANCES};

// ── Capacity ──────────────────────────────────────────────────────────────────

pub const MAX_TASKS: usize = MAX_INSTANCES; // one task per pool slot at most
const MAX_NAME: usize = 32;

// ── Task state ────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum TaskState {
    /// Spawned but the entry function has not been called yet.
    Ready,
    /// Currently executing (on CPU, or selected by scheduler).
    Running,
    /// Called `host_yield`; waiting for the scheduler to resume it.
    Suspended,
    /// Ran to completion or was killed.
    Done,
}

// ── Task record ───────────────────────────────────────────────────────────────

struct Task {
    state:      TaskState,
    /// Handle into the engine's instance pool.
    instance:   usize,
    name:       [u8; MAX_NAME],
    name_len:   usize,
}

// ── Static queue ──────────────────────────────────────────────────────────────

const BLANK: Option<Task> = None;
// SAFETY: single-core, no preemption during mutations.
static mut TASK_QUEUE: [Option<Task>; MAX_TASKS] = [BLANK; MAX_TASKS];

// ── Public API ────────────────────────────────────────────────────────────────

/// Instantiate `bytes` and register it as a new task.  Returns the task ID.
pub fn task_spawn(name: &str, bytes: &'static [u8]) -> Result<usize, RunError> {
    let slot = unsafe {
        TASK_QUEUE.iter().position(|t| t.is_none()).ok_or(RunError::PoolFull)?
    };

    let instance = engine::spawn(name, bytes)?;

    let nb = name.as_bytes();
    let nl = nb.len().min(MAX_NAME);
    let mut task_name = [0u8; MAX_NAME];
    task_name[..nl].copy_from_slice(&nb[..nl]);

    unsafe {
        TASK_QUEUE[slot] = Some(Task {
            state:    TaskState::Ready,
            instance,
            name:     task_name,
            name_len: nl,
        });
    }
    Ok(slot)
}

/// Mark the task as Done and free its instance pool slot.
pub fn task_kill(id: usize) {
    if id >= MAX_TASKS { return; }
    unsafe {
        if let Some(t) = TASK_QUEUE[id].take() {
            engine::destroy(t.instance);
        }
    }
}

/// Return the current state of a task, or `None` if the slot is empty.
pub fn task_state(id: usize) -> Option<TaskState> {
    if id >= MAX_TASKS { return None; }
    unsafe { TASK_QUEUE[id].as_ref().map(|t| t.state) }
}

/// Step one task: start it (if Ready) or resume it (if Suspended).
/// Updates the task's state and returns what the task did.
/// Returns `None` if the slot is empty or the task is not runnable.
pub fn task_step(id: usize) -> Option<Result<TaskResult, RunError>> {
    if id >= MAX_TASKS { return None; }
    let (state, instance) = unsafe {
        let t = TASK_QUEUE[id].as_mut()?;
        (t.state, t.instance)
    };

    let result = match state {
        TaskState::Ready => {
            unsafe { if let Some(t) = TASK_QUEUE[id].as_mut() { t.state = TaskState::Running; } }
            Some(engine::start_task(instance, "main", &[]))
        }
        TaskState::Suspended => {
            unsafe { if let Some(t) = TASK_QUEUE[id].as_mut() { t.state = TaskState::Running; } }
            Some(engine::resume_task(instance))
        }
        _ => None,
    }?;

    // Update state based on outcome.
    unsafe {
        if let Some(t) = TASK_QUEUE[id].as_mut() {
            match &result {
                Ok(TaskResult::Completed(_)) => {
                    engine::destroy(t.instance);
                    TASK_QUEUE[id] = None;
                }
                Ok(TaskResult::Yielded) => {
                    t.state = TaskState::Suspended;
                }
                Err(_) => {
                    engine::destroy(t.instance);
                    TASK_QUEUE[id] = None;
                }
            }
        }
    }

    Some(result)
}

/// Call `f(id, name, state)` for every non-empty slot.
pub fn for_each_task<F: FnMut(usize, &str, TaskState)>(mut f: F) {
    unsafe {
        for (i, slot) in TASK_QUEUE.iter().enumerate() {
            if let Some(t) = slot {
                let name = core::str::from_utf8(&t.name[..t.name_len]).unwrap_or("?");
                f(i, name, t.state);
            }
        }
    }
}
