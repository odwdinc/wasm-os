//! Cooperative task abstraction over the WASM engine instance pool.
//!
//! Each [`Task`] owns one [`engine`](crate::wasm::engine) pool slot and
//! carries a [`TaskState`] that drives the round-robin scheduler in
//! [`crate::scheduler`].
//!
//! The interpreter's own stacks (value, call, control) are the saved
//! execution state — no separate snapshot is needed because the
//! interpreter is never reset between yield/resume cycles.

use super::engine::{self, RunError, TaskResult, MAX_INSTANCES};

// ── Capacity ──────────────────────────────────────────────────────────────────

pub const MAX_TASKS: usize = MAX_INSTANCES; // one task per pool slot at most
const MAX_NAME: usize = 32;

// ── Task state ────────────────────────────────────────────────────────────────

/// Execution state of a WASM task.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum TaskState {
    /// Spawned; the entry function (`main`) has not been called yet.
    Ready,
    /// Selected by the scheduler and currently executing.
    Running,
    /// Called `env.yield`; waiting for the scheduler to call [`task_step`] again.
    Suspended,
    /// Called `env.sleep_ms`; will become [`TaskState::Suspended`] once
    /// [`crate::drivers::pit::ticks`] reaches the stored wake tick.
    Sleeping(u64),
    /// Ran to completion or was killed via [`task_kill`].
    #[allow(dead_code)]
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

/// Instantiate `bytes` as a new task and return its task ID.
///
/// Calls [`engine::spawn`](crate::wasm::engine::spawn) to obtain a pool slot,
/// then registers the slot in the static task queue.
///
/// # Errors
///
/// Propagates [`RunError`](crate::wasm::engine::RunError) from the engine
/// (e.g. `PoolFull`, `ImportNotFound`, parse errors).
pub fn task_spawn(name: &str, bytes: &'static [u8]) -> Result<usize, RunError> {
    let slot = unsafe {
        (*core::ptr::addr_of!(TASK_QUEUE)).iter().position(|t| t.is_none()).ok_or(RunError::PoolFull)?
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

/// Kill a task: remove it from the queue and free its engine pool slot.
///
/// No-op if `id >= MAX_TASKS` or the slot is already empty.
pub fn task_kill(id: usize) {
    if id >= MAX_TASKS { return; }
    unsafe {
        if let Some(t) = TASK_QUEUE[id].take() {
            engine::destroy(t.instance);
        }
    }
}

/// Return the [`TaskState`] of task `id`, or `None` if the slot is empty.
#[allow(dead_code)]
pub fn task_state(id: usize) -> Option<TaskState> {
    if id >= MAX_TASKS { return None; }
    unsafe { TASK_QUEUE[id].as_ref().map(|t| t.state) }
}

/// Advance task `id` by one step.
///
/// - If the task is [`TaskState::Ready`], calls
///   [`engine::start_task`](crate::wasm::engine::start_task) with entry
///   `"main"`.
/// - If the task is [`TaskState::Suspended`] (or a [`TaskState::Sleeping`]
///   task whose wake tick has elapsed), calls
///   [`engine::resume_task`](crate::wasm::engine::resume_task).
///
/// Updates [`TaskState`] based on the outcome:
/// - `Completed` → removes the task and destroys the pool slot.
/// - `Yielded` with pending sleep → transitions to `Sleeping(wake_tick)`.
/// - `Yielded` without sleep → transitions to `Suspended`.
/// - Error → removes the task and destroys the pool slot.
///
/// Returns `None` if the slot is empty or the task is not currently runnable.
pub fn task_step(id: usize) -> Option<Result<TaskResult, RunError>> {
    if id >= MAX_TASKS { return None; }

    // Wake a sleeping task if its timer has elapsed.
    unsafe {
        if let Some(t) = TASK_QUEUE[id].as_mut() {
            if let TaskState::Sleeping(wake) = t.state {
                if crate::drivers::pit::ticks() >= wake {
                    t.state = TaskState::Suspended;
                } else {
                    return None; // still sleeping
                }
            }
        }
    }

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
                    let sleep_ms = engine::take_pending_sleep_ms();
                    if sleep_ms > 0 {
                        // Convert ms → ticks (100 Hz → 1 tick = 10 ms; round up).
                        let wake = crate::drivers::pit::ticks()
                            + (sleep_ms as u64 + 9) / 10;
                        t.state = TaskState::Sleeping(wake);
                    } else {
                        t.state = TaskState::Suspended;
                    }
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

/// Return `true` if task `id` exists and can be stepped right now.
///
/// A `Sleeping` task becomes runnable once its wake tick is reached.
pub fn is_task_runnable(id: usize) -> bool {
    if id >= MAX_TASKS { return false; }
    unsafe {
        match TASK_QUEUE[id].as_ref().map(|t| t.state) {
            Some(TaskState::Ready) | Some(TaskState::Suspended) => true,
            Some(TaskState::Sleeping(wake)) => crate::drivers::pit::ticks() >= wake,
            _ => false,
        }
    }
}

/// Call `f(id, name, state)` for every non-empty task slot.
///
/// Used by the `tasks` shell command.
pub fn for_each_task<F: FnMut(usize, &str, TaskState)>(mut f: F) {
    unsafe {
        for (i, slot) in (*core::ptr::addr_of!(TASK_QUEUE)).iter().enumerate() {
            if let Some(t) = slot {
                let name = core::str::from_utf8(&t.name[..t.name_len]).unwrap_or("?");
                f(i, name, t.state);
            }
        }
    }
}
