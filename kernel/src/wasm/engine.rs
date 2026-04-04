//! Execution engine — Sprint B.4: Host Function Registry
//!
//! Public API:
//!   `register_host(module, name, fn)` — register a named host function at boot
//!   `init_host_fns()`                 — register the kernel's built-in host functions
//!   `spawn(name, bytes)`              — instantiate into a pool slot, return handle
//!   `call_handle(handle, entry, args)`— execute an exported function
//!   `destroy(handle)`                 — free the pool slot, zero its memory
//!   `for_each_instance(f)`            — iterate active slots (for `ps`)
//!   `run(bytes, entry, args)`         — convenience: spawn + call + destroy

use core::mem::MaybeUninit;
use core::sync::atomic::{AtomicU32, Ordering};
use super::loader::{load, find_export, read_memory_min_pages, for_each_func_import,
                    LoadError, read_u32_leb128};
use super::interp::{Interpreter, InterpError, HostFn, MAX_FUNCS, STACK_DEPTH, read_i32_leb128};

// ── Cooperative yield channel ─────────────────────────────────────────────────

/// Set by `host_sleep_ms` before returning `Yielded`; consumed by `task_step`.
/// Zero means plain yield (no sleep delay).
static PENDING_SLEEP_MS: AtomicU32 = AtomicU32::new(0);

/// Take the pending sleep duration (ms) set by the last `sleep_ms` call, if any.
/// Resets the global to 0. Called by `task::task_step` after a `Yielded` result.
pub fn take_pending_sleep_ms() -> u32 {
    PENDING_SLEEP_MS.swap(0, Ordering::Relaxed)
}

// ── Capacity limits ───────────────────────────────────────────────────────────

/// Maximum WASM pages (64 KiB each) any module may request.
pub const MAX_MEM_PAGES: u32  = 4;
/// Maximum simultaneously live instances.
pub const MAX_INSTANCES: usize = 4;
const PAGE_SIZE: usize = 65536;

// ── Per-slot memory pool ──────────────────────────────────────────────────────

/// Each pool slot owns a dedicated memory region so instances never share RAM.
/// Only the first `mem_pages * PAGE_SIZE` bytes of each slot are used.
// SAFETY: accessed only from the single kernel thread; no preemption.
static mut SLOT_MEM: [[u8; MAX_MEM_PAGES as usize * PAGE_SIZE]; MAX_INSTANCES] =
    [[0u8; MAX_MEM_PAGES as usize * PAGE_SIZE]; MAX_INSTANCES];

// ── Host function registry ────────────────────────────────────────────────────

/// Maximum number of host functions that can be registered.
pub const MAX_HOST_FUNCS: usize = 16;

#[derive(Clone, Copy)]
struct HostEntry {
    module: &'static str,
    name:   &'static str,
    func:   HostFn,
}

const EMPTY_ENTRY: Option<HostEntry> = None;
static mut HOST_REGISTRY: [Option<HostEntry>; MAX_HOST_FUNCS] = [EMPTY_ENTRY; MAX_HOST_FUNCS];
static mut HOST_COUNT:    usize = 0;

/// Register a named host function.  Called at boot before any module is run.
/// Silently ignores registration if the registry is full.
pub fn register_host(module: &'static str, name: &'static str, func: HostFn) {
    unsafe {
        if HOST_COUNT < MAX_HOST_FUNCS {
            HOST_REGISTRY[HOST_COUNT] = Some(HostEntry { module, name, func });
            HOST_COUNT += 1;
        }
    }
}

fn lookup_host(module: &str, name: &str) -> Option<HostFn> {
    unsafe {
        for i in 0..HOST_COUNT {
            if let Some(e) = HOST_REGISTRY[i] {
                if e.module == module && e.name == name {
                    return Some(e.func);
                }
            }
        }
    }
    None
}

// ── Error type ────────────────────────────────────────────────────────────────

pub enum RunError {
    Load(LoadError),
    Interp(InterpError),
    EntryNotFound,
    MemoryTooLarge,
    PoolFull,
    ImportNotFound,
}

impl RunError {
    pub fn as_str(&self) -> &'static str {
        match self {
            RunError::Load(e)        => e.as_str(),
            RunError::Interp(e)      => e.as_str(),
            RunError::EntryNotFound  => "entry point not found in exports",
            RunError::MemoryTooLarge => "module requests more memory than the kernel allows",
            RunError::PoolFull       => "instance pool is full",
            RunError::ImportNotFound => "module imports an unregistered host function",
        }
    }
}

// ── Built-in host functions ───────────────────────────────────────────────────

/// `print(ptr: i32, len: i32)` — write UTF-8 bytes from linear memory.
fn host_print(vstack: &mut [i64], vsp: &mut usize, mem: &mut [u8]) -> Result<(), InterpError> {
    if *vsp < 2 { return Err(InterpError::StackUnderflow); }
    let len = vstack[*vsp - 1] as usize;
    let ptr = vstack[*vsp - 2] as usize;
    *vsp -= 2;
    let end = ptr.saturating_add(len).min(mem.len());
    if let Ok(s) = core::str::from_utf8(&mem[ptr..end]) {
        crate::print!("{}", s);
    }
    Ok(())
}

/// `print_int(n: i32)` — print a decimal integer followed by a newline.
fn host_print_int(vstack: &mut [i64], vsp: &mut usize, _mem: &mut [u8]) -> Result<(), InterpError> {
    if *vsp < 1 { return Err(InterpError::StackUnderflow); }
    let n = vstack[*vsp - 1] as i32;
    *vsp -= 1;
    fmt_i32(n);
    Ok(())
}

/// `yield()` — cooperatively surrender the CPU to the scheduler.
fn host_yield(_vstack: &mut [i64], _vsp: &mut usize, _mem: &mut [u8]) -> Result<(), InterpError> {
    Err(InterpError::Yielded)
}

/// `sleep_ms(ms: i32)` — yield for at least `ms` milliseconds.
fn host_sleep_ms(vstack: &mut [i64], vsp: &mut usize, _mem: &mut [u8]) -> Result<(), InterpError> {
    if *vsp < 1 { return Err(InterpError::StackUnderflow); }
    let ms = vstack[*vsp - 1] as i32;
    *vsp -= 1;
    PENDING_SLEEP_MS.store(ms.max(0) as u32, Ordering::Relaxed);
    Err(InterpError::Yielded)
}

/// Register the kernel's built-in host functions.  Call once at boot before
/// running any module.
pub fn init_host_fns() {
    register_host("env", "print",     host_print);
    register_host("env", "print_int", host_print_int);
    register_host("env", "yield",     host_yield);
    register_host("env", "sleep_ms",  host_sleep_ms);
}

/// Print an i32 as decimal followed by a newline, without heap allocation.
fn fmt_i32(n: i32) {
    let mut buf = [0u8; 11];
    let mut pos = buf.len();
    let negative = n < 0;
    let mut val: u32 = if n == i32::MIN { 2147483648 }
                       else if negative { (-n) as u32 }
                       else             { n as u32 };
    if val == 0 { crate::println!("0"); return; }
    while val > 0 {
        pos -= 1;
        buf[pos] = b'0' + (val % 10) as u8;
        val /= 10;
    }
    if negative { pos -= 1; buf[pos] = b'-'; }
    if let Ok(s) = core::str::from_utf8(&buf[pos..]) {
        crate::println!("{}", s);
    }
}

// ── Memory initialisation ─────────────────────────────────────────────────────

fn init_memory(data_section: &[u8], mem: &mut [u8]) -> bool {
    let mut cur = 0usize;
    let (count, n) = match read_u32_leb128(data_section) {
        Some(x) => x, None => return false,
    };
    cur += n;

    for _ in 0..count as usize {
        let (kind, n) = match read_u32_leb128(&data_section[cur..]) {
            Some(x) => x, None => return false,
        };
        cur += n;

        if kind == 0 {
            if cur >= data_section.len() || data_section[cur] != 0x41 { return false; }
            cur += 1;
            let (offset, n) = match read_i32_leb128(&data_section[cur..]) {
                Some(x) => x, None => return false,
            };
            cur += n;
            if cur >= data_section.len() || data_section[cur] != 0x0B { return false; }
            cur += 1;

            let (data_len, n) = match read_u32_leb128(&data_section[cur..]) {
                Some(x) => x, None => return false,
            };
            cur += n;
            let data_len = data_len as usize;
            if cur + data_len > data_section.len() { return false; }

            let dst = offset as usize;
            let dst_end = dst.saturating_add(data_len).min(mem.len());
            let copy_len = dst_end - dst;
            mem[dst..dst_end].copy_from_slice(&data_section[cur..cur + copy_len]);
            cur += data_len;
        }
    }
    true
}

// ── Import counting ───────────────────────────────────────────────────────────

pub fn count_func_imports(import_section: Option<&[u8]>) -> usize {
    let bytes = match import_section { Some(b) => b, None => return 0 };
    let mut cur = 0usize;
    let (count, n) = match read_u32_leb128(bytes) { Some(x) => x, None => return 0 };
    cur += n;

    let mut func_count = 0usize;
    for _ in 0..count as usize {
        let (mod_len, n) = match read_u32_leb128(&bytes[cur..]) { Some(x) => x, None => return func_count };
        cur += n + mod_len as usize;
        if cur > bytes.len() { return func_count; }

        let (name_len, n) = match read_u32_leb128(&bytes[cur..]) { Some(x) => x, None => return func_count };
        cur += n + name_len as usize;
        if cur > bytes.len() { return func_count; }

        if cur >= bytes.len() { return func_count; }
        let kind = bytes[cur]; cur += 1;

        match kind {
            0 => {
                let (_, n) = match read_u32_leb128(&bytes[cur..]) { Some(x) => x, None => return func_count };
                cur += n;
                func_count += 1;
            }
            1 => {
                cur += 1;
                if cur >= bytes.len() { return func_count; }
                let flag = bytes[cur]; cur += 1;
                let (_, n) = match read_u32_leb128(&bytes[cur..]) { Some(x) => x, None => return func_count };
                cur += n;
                if flag != 0 {
                    let (_, n) = match read_u32_leb128(&bytes[cur..]) { Some(x) => x, None => return func_count };
                    cur += n;
                }
            }
            2 => {
                if cur >= bytes.len() { return func_count; }
                let flag = bytes[cur]; cur += 1;
                let (_, n) = match read_u32_leb128(&bytes[cur..]) { Some(x) => x, None => return func_count };
                cur += n;
                if flag != 0 {
                    let (_, n) = match read_u32_leb128(&bytes[cur..]) { Some(x) => x, None => return func_count };
                    cur += n;
                }
            }
            3 => { cur += 2; }
            _  => return func_count,
        }
    }
    func_count
}

// ── Instance ──────────────────────────────────────────────────────────────────

/// A live, instantiated WASM module.
/// The lifetime `'a` is tied to the byte slice the module was loaded from.
/// For pool instances this is always `'static` (embedded modules via include_bytes!).
pub struct Instance<'a> {
    bytes:  &'a [u8],
    interp: Interpreter<'a>,
}

fn call_instance<'a>(
    inst:  &mut Instance<'a>,
    entry: &str,
    args:  &[i32],
) -> Result<Option<i64>, RunError> {
    let module   = load(inst.bytes).map_err(RunError::Load)?;
    let func_idx = find_export(&module, entry)
        .ok_or(RunError::EntryNotFound)? as usize;

    inst.interp.reset_for_call();
    for &arg in args {
        if inst.interp.vsp >= STACK_DEPTH {
            return Err(RunError::Interp(InterpError::StackOverflow));
        }
        inst.interp.vstack[inst.interp.vsp] = arg as i64;
        inst.interp.vsp += 1;
    }
    inst.interp.call(func_idx).map_err(RunError::Interp)?;
    Ok(inst.interp.top())
}

// ── Instance pool ─────────────────────────────────────────────────────────────

const MAX_INST_NAME: usize = 32;

struct PoolSlot {
    active:    bool,
    name:      [u8; MAX_INST_NAME],
    name_len:  usize,
    mem_pages: usize,
    inst:      MaybeUninit<Instance<'static>>,
}

impl PoolSlot {
    const fn blank() -> Self {
        Self {
            active:    false,
            name:      [0u8; MAX_INST_NAME],
            name_len:  0,
            mem_pages: 0,
            inst:      MaybeUninit::uninit(),
        }
    }
}

const BLANK_SLOT: PoolSlot = PoolSlot::blank();
static mut POOL: [PoolSlot; MAX_INSTANCES] = [BLANK_SLOT; MAX_INSTANCES];

/// Instantiate `bytes` into the first free pool slot and return its handle.
/// The module is wired up with the kernel host functions.
pub fn spawn(name: &str, bytes: &'static [u8]) -> Result<usize, RunError> {
    // Find a free slot.
    let slot = unsafe {
        POOL.iter().position(|s| !s.active).ok_or(RunError::PoolFull)?
    };

    let module = load(bytes).map_err(RunError::Load)?;

    let min_pages = module.memory_section
        .map(read_memory_min_pages)
        .unwrap_or(0);
    if min_pages > MAX_MEM_PAGES { return Err(RunError::MemoryTooLarge); }
    let mem_bytes = min_pages as usize * PAGE_SIZE;

    // Zero this slot's memory region and take a 'static mutable slice.
    // SAFETY: single-threaded kernel; slot is verified free above.
    let mem: &'static mut [u8] = unsafe {
        SLOT_MEM[slot][..mem_bytes].fill(0);
        &mut SLOT_MEM[slot][..mem_bytes]
    };

    let import_count = count_func_imports(module.import_section);

    // Resolve each function import against the host registry.
    let mut host_fns: [Option<HostFn>; MAX_FUNCS] = [None; MAX_FUNCS];
    let mut idx = 0usize;
    let mut missing = false;
    if let Some(sec) = module.import_section {
        for_each_func_import(sec, &mut |module_name, func_name| {
            if idx < MAX_FUNCS {
                host_fns[idx] = lookup_host(module_name, func_name);
                if host_fns[idx].is_none() { missing = true; }
                idx += 1;
            }
        });
    }
    if missing { return Err(RunError::ImportNotFound); }

    let mut interp = Interpreter::new(&module, import_count, mem, host_fns)
        .map_err(RunError::Interp)?;

    if let Some(data) = module.data_section {
        if !init_memory(data, &mut interp.mem) {
            return Err(RunError::Interp(InterpError::MalformedCode));
        }
    }

    // Write instance into the pool slot.
    unsafe {
        let s = &mut POOL[slot];
        let nb = name.as_bytes();
        let nl = nb.len().min(MAX_INST_NAME);
        s.name[..nl].copy_from_slice(&nb[..nl]);
        s.name_len  = nl;
        s.mem_pages = min_pages as usize;
        s.inst.write(Instance { bytes, interp });
        s.active = true;
    }

    Ok(slot)
}

/// Execute `entry` on the instance identified by `handle`.
pub fn call_handle(handle: usize, entry: &str, args: &[i32]) -> Result<Option<i64>, RunError> {
    if handle >= MAX_INSTANCES { return Err(RunError::EntryNotFound); }
    // SAFETY: active flag guarantees the MaybeUninit is initialised.
    let inst = unsafe {
        if !POOL[handle].active { return Err(RunError::EntryNotFound); }
        POOL[handle].inst.assume_init_mut()
    };
    call_instance(inst, entry, args)
}

/// Free the pool slot: drop the instance and zero its memory.
pub fn destroy(handle: usize) {
    if handle >= MAX_INSTANCES { return; }
    unsafe {
        let s = &mut POOL[handle];
        if !s.active { return; }
        s.inst.assume_init_drop();
        s.active = false;
        SLOT_MEM[handle].fill(0);
    }
}

/// Call `f(handle, name, mem_pages)` for each active pool slot.
pub fn for_each_instance<F: FnMut(usize, &str, usize)>(mut f: F) {
    unsafe {
        for (i, s) in POOL.iter().enumerate() {
            if s.active {
                let name = core::str::from_utf8(&s.name[..s.name_len]).unwrap_or("?");
                f(i, name, s.mem_pages);
            }
        }
    }
}

// ── Task execution helpers ────────────────────────────────────────────────────

/// Result of a task execution step.
pub enum TaskResult {
    /// Task ran to completion; holds the optional return value.
    Completed(Option<i64>),
    /// Task called `host_yield` and suspended; call `resume_task` to continue.
    Yielded,
}

/// Begin executing `entry` on an already-spawned instance.
/// Returns `Yielded` if the task requested a cooperative yield before finishing.
pub fn start_task(handle: usize, entry: &str, args: &[i32]) -> Result<TaskResult, RunError> {
    if handle >= MAX_INSTANCES { return Err(RunError::EntryNotFound); }
    let inst = unsafe {
        if !POOL[handle].active { return Err(RunError::EntryNotFound); }
        POOL[handle].inst.assume_init_mut()
    };
    let module   = load(inst.bytes).map_err(RunError::Load)?;
    let func_idx = find_export(&module, entry)
        .ok_or(RunError::EntryNotFound)? as usize;

    inst.interp.reset_for_call();
    for &arg in args {
        if inst.interp.vsp >= STACK_DEPTH {
            return Err(RunError::Interp(InterpError::StackOverflow));
        }
        inst.interp.vstack[inst.interp.vsp] = arg as i64;
        inst.interp.vsp += 1;
    }
    match inst.interp.call(func_idx) {
        Ok(())                             => Ok(TaskResult::Completed(inst.interp.top())),
        Err(InterpError::Yielded)          => Ok(TaskResult::Yielded),
        Err(e)                             => Err(RunError::Interp(e)),
    }
}

/// Continue a suspended task from where it yielded.
pub fn resume_task(handle: usize) -> Result<TaskResult, RunError> {
    if handle >= MAX_INSTANCES { return Err(RunError::EntryNotFound); }
    let inst = unsafe {
        if !POOL[handle].active { return Err(RunError::EntryNotFound); }
        POOL[handle].inst.assume_init_mut()
    };
    match inst.interp.resume() {
        Ok(())                             => Ok(TaskResult::Completed(inst.interp.top())),
        Err(InterpError::Yielded)          => Ok(TaskResult::Yielded),
        Err(e)                             => Err(RunError::Interp(e)),
    }
}

// ── Convenience wrapper ───────────────────────────────────────────────────────

/// Spawn an instance, call `entry`, destroy it, return the result.
/// Modules that call `yield` or `sleep_ms` are run to completion synchronously
/// (yields are treated as no-ops in this path).
pub fn run(bytes: &'static [u8], entry: &str, args: &[i32]) -> Result<Option<i64>, RunError> {
    let handle = spawn("", bytes)?;
    let mut result = start_task(handle, entry, args);
    // Drain cooperative yields: keep resuming until completion or a real error.
    while let Ok(TaskResult::Yielded) = result {
        result = resume_task(handle);
    }
    let final_result = match result {
        Ok(TaskResult::Completed(v)) => Ok(v),
        Ok(TaskResult::Yielded)      => unreachable!(),
        Err(e)                       => Err(e),
    };
    destroy(handle);
    final_result
}

// ── Embedded userland modules ─────────────────────────────────────────────────

pub const HELLO_WASM:  &[u8] = include_bytes!("../../../userland/hello/hello.wasm");
pub const GREET_WASM:  &[u8] = include_bytes!("../../../userland/greet/greet.wasm");
pub const FIB_WASM:    &[u8] = include_bytes!("../../../userland/fib/fib.wasm");
pub const PRIMES_WASM: &[u8] = include_bytes!("../../../userland/primes/primes.wasm");
pub const COLLATZ_WASM: &[u8] = include_bytes!("../../../userland/collatz/collatz.wasm");
pub const COUNTER_WASM: &[u8] = include_bytes!("../../../userland/counter/counter.wasm");
