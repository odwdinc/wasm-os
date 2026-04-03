//! Execution engine — Sprint B.3: Instance Pool
//!
//! Public API:
//!   `spawn(name, bytes)`            — instantiate into a pool slot, return handle
//!   `call_handle(handle, entry, args)` — execute an exported function
//!   `destroy(handle)`               — free the pool slot, zero its memory
//!   `for_each_instance(f)`          — iterate active slots (for `ps`)
//!   `run(bytes, entry, args)`       — convenience: spawn + call + destroy

use core::mem::MaybeUninit;
use super::loader::{load, find_export, read_memory_min_pages, LoadError, read_u32_leb128};
use super::interp::{Interpreter, InterpError, HostFn, STACK_DEPTH, read_i32_leb128};

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

// ── Error type ────────────────────────────────────────────────────────────────

pub enum RunError {
    Load(LoadError),
    Interp(InterpError),
    EntryNotFound,
    MemoryTooLarge,
    PoolFull,
}

impl RunError {
    pub fn as_str(&self) -> &'static str {
        match self {
            RunError::Load(e)        => e.as_str(),
            RunError::Interp(e)      => e.as_str(),
            RunError::EntryNotFound  => "entry point not found in exports",
            RunError::MemoryTooLarge => "module requests more memory than the kernel allows",
            RunError::PoolFull       => "instance pool is full",
        }
    }
}

// ── Host functions ────────────────────────────────────────────────────────────

/// Kernel host dispatch.
///   index 0 — `print(ptr: i32, len: i32)`  write UTF-8 from linear memory
///   index 1 — `print_int(n: i32)`           print decimal integer + newline
fn kernel_host(
    func_idx: usize,
    vstack:   &mut [i64],
    vsp:      &mut usize,
    mem:      &mut [u8],
) -> Result<(), InterpError> {
    match func_idx {
        0 => {
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
        1 => {
            if *vsp < 1 { return Err(InterpError::StackUnderflow); }
            let n = vstack[*vsp - 1] as i32;
            *vsp -= 1;
            fmt_i32(n);
            Ok(())
        }
        _ => Err(InterpError::IsImport),
    }
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
    let mut interp = Interpreter::new(&module, import_count, mem)
        .map_err(RunError::Interp)?;
    interp.host_fn = kernel_host;

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

// ── Convenience wrapper ───────────────────────────────────────────────────────

/// Spawn an instance, call `entry`, destroy it, return the result.
pub fn run(bytes: &'static [u8], entry: &str, args: &[i32]) -> Result<Option<i64>, RunError> {
    let handle = spawn("", bytes)?;
    let result = call_handle(handle, entry, args);
    destroy(handle);
    result
}

// ── Embedded userland modules ─────────────────────────────────────────────────

pub const HELLO_WASM:  &[u8] = include_bytes!("../../../userland/hello/hello.wasm");
pub const GREET_WASM:  &[u8] = include_bytes!("../../../userland/greet/greet.wasm");
pub const FIB_WASM:    &[u8] = include_bytes!("../../../userland/fib/fib.wasm");
pub const PRIMES_WASM: &[u8] = include_bytes!("../../../userland/primes/primes.wasm");
