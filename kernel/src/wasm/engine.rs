//! WASM execution engine — instance pool, host-function registry, and
//! task execution helpers.
//!
//! # Host-function registry
//!
//! Up to [`MAX_HOST_FUNCS`] host functions can be registered before any
//! module is spawned.  Call [`init_host_fns`] once at boot to register the
//! kernel built-ins, then optionally call [`register_host`] for any
//! application-specific imports.
//!
//! # Instance pool
//!
//! Up to [`MAX_INSTANCES`] modules can be live simultaneously.  Each slot
//! owns a dedicated [`MAX_MEM_PAGES`] × 64 KiB static memory region in
//! `SLOT_MEM`.  Use [`spawn`] / [`destroy`] to manage lifetimes, or the
//! convenience wrapper [`run`] for fire-and-forget execution.
//!
//! # Task execution
//!
//! [`start_task`] and [`resume_task`] integrate with the cooperative
//! scheduler: a module that calls `host_yield` or `sleep_ms` returns
//! [`TaskResult::Yielded`] rather than completing, and execution resumes
//! from the same point on the next call to [`resume_task`].

use core::mem::MaybeUninit;
use core::sync::atomic::{AtomicU32, Ordering};
use super::loader::{load, find_export, read_memory_min_pages, for_each_func_import,
                    LoadError, read_u32_leb128};
use super::interp::{Interpreter, InterpError, HostFn, MAX_FUNCS, STACK_DEPTH, read_i32_leb128};

// ── Args buffer ──────────────────────────────────────────────────────────────
//
// Holds the raw argument string for the currently-running module so that
// `args_get` can copy it into linear memory.  Set by the shell `run` command
// before spawning; cleared to empty at the start of each spawn.

const ARGS_CAP: usize = 128;
static mut ARGS_BUF: [u8; ARGS_CAP] = [0u8; ARGS_CAP];
static mut ARGS_LEN: usize = 0;

/// Store `args` so the next module can read them via `args_get`.
/// Accepts the raw space-joined argument string (everything after the module name).
pub fn set_args(args: &str) {
    unsafe {
        let bytes = args.as_bytes();
        let len = bytes.len().min(ARGS_CAP);
        ARGS_BUF[..len].copy_from_slice(&bytes[..len]);
        ARGS_LEN = len;
    }
}

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
pub const MAX_MEM_PAGES: u32  = 32;
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
pub const MAX_HOST_FUNCS: usize = 48;

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

/// Errors that can be returned by [`spawn`], [`start_task`], [`resume_task`],
/// or the convenience wrapper [`run`].
pub enum RunError {
    /// The WASM binary could not be parsed (see [`LoadError`]).
    Load(LoadError),
    /// A runtime trap occurred during execution (see [`InterpError`]).
    Interp(InterpError),
    /// The named export was not found in the module.
    EntryNotFound,
    /// The module requests more linear memory than [`MAX_MEM_PAGES`] pages.
    MemoryTooLarge,
    /// All [`MAX_INSTANCES`] pool slots are occupied.
    PoolFull,
    /// The module imports a function that is not in the host registry.
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

/// `print_i64(n: i64)` — print a signed 64-bit integer followed by a newline.
fn host_print_i64(vstack: &mut [i64], vsp: &mut usize, _mem: &mut [u8]) -> Result<(), InterpError> {
    if *vsp < 1 { return Err(InterpError::StackUnderflow); }
    let n = vstack[*vsp - 1];
    *vsp -= 1;
    fmt_i64(n);
    Ok(())
}

/// `print_char(c: i32)` — print a single ASCII character (low byte of c).
fn host_print_char(vstack: &mut [i64], vsp: &mut usize, _mem: &mut [u8]) -> Result<(), InterpError> {
    if *vsp < 1 { return Err(InterpError::StackUnderflow); }
    let c = (vstack[*vsp - 1] & 0xFF) as u8;
    *vsp -= 1;
    if let Ok(s) = core::str::from_utf8(core::slice::from_ref(&c)) {
        crate::print!("{}", s);
    }
    Ok(())
}

/// `print_hex(n: i32)` — print i32 as zero-padded 8-digit hex (e.g. `0x0000002A`).
fn host_print_hex(vstack: &mut [i64], vsp: &mut usize, _mem: &mut [u8]) -> Result<(), InterpError> {
    if *vsp < 1 { return Err(InterpError::StackUnderflow); }
    let n = vstack[*vsp - 1] as u32;
    *vsp -= 1;
    fmt_hex(n);
    Ok(())
}

/// `uptime_ms() → i32` — milliseconds since TSC calibration (sub-ms accurate).
fn host_uptime_ms(vstack: &mut [i64], vsp: &mut usize, _mem: &mut [u8]) -> Result<(), InterpError> {
    if *vsp >= STACK_DEPTH { return Err(InterpError::StackOverflow); }
    let ms = crate::drivers::pit::uptime_ms().min(i32::MAX as i64);
    vstack[*vsp] = ms;
    *vsp += 1;
    Ok(())
}

/// `wasm_opcount() → i64` — total WASM opcodes dispatched since boot.
fn host_wasm_opcount(vstack: &mut [i64], vsp: &mut usize, _mem: &mut [u8]) -> Result<(), InterpError> {
    if *vsp >= STACK_DEPTH { return Err(InterpError::StackOverflow); }
    vstack[*vsp] = unsafe { crate::wasm::interp::OPCODE_COUNT as i64 };
    *vsp += 1;
    Ok(())
}

/// `exit(code: i32)` — terminate the module cleanly (treated as normal return).
fn host_exit(_vstack: &mut [i64], vsp: &mut usize, _mem: &mut [u8]) -> Result<(), InterpError> {
    if *vsp < 1 { return Err(InterpError::StackUnderflow); }
    *vsp -= 1; // consume the code; we don't use it
    Err(InterpError::Exited)
}

/// `read_char() → i32` — block until a printable key is pressed; returns its ASCII code.
/// Returns 10 (newline) for Enter, -1 on unrecognised key.
fn host_read_char(vstack: &mut [i64], vsp: &mut usize, _mem: &mut [u8]) -> Result<(), InterpError> {
    if *vsp >= STACK_DEPTH { return Err(InterpError::StackOverflow); }
    use crate::drivers::keyboard::{next_key, Key};
    let code: i32 = loop {
        match next_key() {
            Key::Char(c) => break c as i32,
            Key::Enter   => break 10,
            Key::Backspace | Key::Unknown => continue,
            Key::ArrowLeft => continue,
            Key::ArrowRight => continue,
            Key::ArrowUp => continue,
            Key::ArrowDown => continue,
            Key::Delete => continue,
            Key::Home => continue,
            Key::End => continue,
            Key::Tab => continue,
        }
    };
    vstack[*vsp] = code as i64;
    *vsp += 1;
    Ok(())
}

/// `read_line(ptr: i32, cap: i32) → i32`
/// Read a line of keyboard input (terminated by Enter) into linear memory.
/// Characters are echoed to the screen. Backspace removes the last character.
/// Returns the number of bytes written (not including any terminator).
/// Returns -1 if `ptr+cap` exceeds memory bounds.
fn host_read_line(vstack: &mut [i64], vsp: &mut usize, mem: &mut [u8]) -> Result<(), InterpError> {
    if *vsp < 2 { return Err(InterpError::StackUnderflow); }
    let cap = vstack[*vsp - 1] as usize;
    let ptr = vstack[*vsp - 2] as usize;
    *vsp -= 2;

    let end = ptr.saturating_add(cap);
    if end > mem.len() {
        vstack[*vsp] = -1i64;
        *vsp += 1;
        return Ok(());
    }

    use crate::drivers::keyboard::{next_key, Key};
    let mut len = 0usize;
    loop {
        match next_key() {
            Key::Enter => {
                crate::println!();
                break;
            }
            Key::Backspace => {
                if len > 0 {
                    len -= 1;
                    // Overwrite the last char on screen with a space.
                    crate::print!("\x08 \x08");
                }
            }
            Key::Char(c) if (c as u32) < 128 => {
                if len < cap {
                    mem[ptr + len] = c as u8;
                    len += 1;
                    crate::print!("{}", c);
                }
            }
            Key::ArrowLeft => {},
            Key::ArrowRight => {},
            Key::ArrowUp => {},
            Key::ArrowDown => {},
            Key::Delete => {},
            Key::Home => {},
            Key::End => {},
            _ => {}
        }
    }

    vstack[*vsp] = len as i64;
    *vsp += 1;
    Ok(())
}

/// `fs_read(name_ptr: i32, name_len: i32, buf_ptr: i32, buf_cap: i32) → i32`
/// Copy a file from the in-memory file table into linear memory.
/// Returns the number of bytes written, or -1 if the file is not found or the
/// buffer is too small / out of bounds.
fn host_fs_read(vstack: &mut [i64], vsp: &mut usize, mem: &mut [u8]) -> Result<(), InterpError> {
    if *vsp < 4 { return Err(InterpError::StackUnderflow); }
    let buf_cap  = vstack[*vsp - 1] as usize;
    let buf_ptr  = vstack[*vsp - 2] as usize;
    let name_len = vstack[*vsp - 3] as usize;
    let name_ptr = vstack[*vsp - 4] as usize;
    *vsp -= 4;

    let result: i64 = 'done: {
        // Validate name slice.
        let name_end = name_ptr.saturating_add(name_len);
        if name_end > mem.len() { break 'done -1; }
        let name = match core::str::from_utf8(&mem[name_ptr..name_end]) {
            Ok(s) => s, Err(_) => break 'done -1,
        };
        // Look up the file.
        let data = match crate::fs::find_file(name) {
            Some(d) => d, None => break 'done -1,
        };
        // Validate destination buffer.
        let buf_end = buf_ptr.saturating_add(buf_cap);
        if buf_end > mem.len() || data.len() > buf_cap { break 'done -1; }
        mem[buf_ptr..buf_ptr + data.len()].copy_from_slice(data);
        data.len() as i64
    };

    vstack[*vsp] = result;
    *vsp += 1;
    Ok(())
}

/// `fs_write(name_ptr: i32, name_len: i32, buf_ptr: i32, buf_len: i32) → i32`
/// Write bytes from linear memory to the filesystem (in-memory table + FAT).
/// Returns 0 on success, -1 on failure (bad pointers, write pool exhausted, etc.).
fn host_fs_write(vstack: &mut [i64], vsp: &mut usize, mem: &mut [u8]) -> Result<(), InterpError> {
    if *vsp < 4 { return Err(InterpError::StackUnderflow); }
    let buf_len  = vstack[*vsp - 1] as usize;
    let buf_ptr  = vstack[*vsp - 2] as usize;
    let name_len = vstack[*vsp - 3] as usize;
    let name_ptr = vstack[*vsp - 4] as usize;
    *vsp -= 4;

    let result: i64 = 'done: {
        let name_end = name_ptr.saturating_add(name_len);
        let buf_end  = buf_ptr.saturating_add(buf_len);
        if name_end > mem.len() || buf_end > mem.len() { break 'done -1; }

        // Copy name out of (mutable) linear memory before we need to borrow `mem`.
        let mut name_buf = [0u8; 64];
        let nlen = name_len.min(64);
        name_buf[..nlen].copy_from_slice(&mem[name_ptr..name_ptr + nlen]);
        let name = match core::str::from_utf8(&name_buf[..nlen]) {
            Ok(s) => s, Err(_) => break 'done -1,
        };

        // Allocate a static write-pool slot and copy the data into it.
        let data = &mem[buf_ptr..buf_ptr + buf_len];
        let static_data = match crate::fs::alloc_write_buf(data) {
            Some(s) => s, None => break 'done -1,
        };

        // Remove any existing entry so the new one takes precedence.
        crate::fs::remove_file(name);
        crate::fs::register_file(name, static_data);

        // Best-effort flush to FAT (ignore errors — in-memory table is already updated).
        crate::fs::fat::fat_write_file(name, static_data);

        0i64
    };

    vstack[*vsp] = result;
    *vsp += 1;
    Ok(())
}

/// `args_get(buf_ptr: i32, buf_cap: i32) → i32`
/// Copy the current module's argument string into linear memory.
/// Returns the number of bytes written, or -1 if the buffer is too small / out of bounds.
/// The string is space-separated (e.g. `"10 hello"` for `run mod.wasm 10 hello`).
fn host_args_get(vstack: &mut [i64], vsp: &mut usize, mem: &mut [u8]) -> Result<(), InterpError> {
    if *vsp < 2 { return Err(InterpError::StackUnderflow); }
    let cap = vstack[*vsp - 1] as usize;
    let ptr = vstack[*vsp - 2] as usize;
    *vsp -= 2;

    let result: i64 = unsafe {
        let len = ARGS_LEN;
        let end = ptr.saturating_add(len);
        if end > mem.len() || len > cap {
            -1
        } else {
            mem[ptr..ptr + len].copy_from_slice(&ARGS_BUF[..len]);
            len as i64
        }
    };

    vstack[*vsp] = result;
    *vsp += 1;
    Ok(())
}

/// `fs_size(name_ptr: i32, name_len: i32) → i32`
/// Return the byte length of a file in the in-memory table, or -1 if not found.
fn host_fs_size(vstack: &mut [i64], vsp: &mut usize, mem: &mut [u8]) -> Result<(), InterpError> {
    if *vsp < 2 { return Err(InterpError::StackUnderflow); }
    let name_len = vstack[*vsp - 1] as usize;
    let name_ptr = vstack[*vsp - 2] as usize;
    *vsp -= 2;

    let result: i64 = {
        let name_end = name_ptr.saturating_add(name_len);
        if name_end > mem.len() {
            -1
        } else {
            match core::str::from_utf8(&mem[name_ptr..name_end]) {
                Ok(name) => match crate::fs::find_file(name) {
                    Some(data) => data.len() as i64,
                    None       => -1,
                },
                Err(_) => -1,
            }
        }
    };

    vstack[*vsp] = result;
    *vsp += 1;
    Ok(())
}

/// `fb_set_pixel(x: i32, y: i32, rgb: i32)` — write one pixel to the framebuffer.
///
/// `x` and `y` are pixel coordinates (top-left origin).  `rgb` is packed as
/// `0x00RRGGBB`.  The kernel converts to the actual pixel format (BGR or RGB)
/// automatically.  Out-of-bounds coordinates are silently ignored.
fn host_fb_set_pixel(vstack: &mut [i64], vsp: &mut usize, _mem: &mut [u8]) -> Result<(), InterpError> {
    if *vsp < 3 { return Err(InterpError::StackUnderflow); }
    let rgb = vstack[*vsp - 1] as u32;
    let y   = vstack[*vsp - 2] as i32;
    let x   = vstack[*vsp - 3] as i32;
    *vsp -= 3;
    crate::vga::set_pixel(x, y, rgb);
    Ok(())
}

/// `fb_present()` — present the framebuffer (no-op on this single-buffered display).
fn host_fb_present(_vstack: &mut [i64], _vsp: &mut usize, _mem: &mut [u8]) -> Result<(), InterpError> {
    Ok(())
}

/// `fb_blit(ptr: i32, width: i32, height: i32)` — blit a packed 0x00RRGGBB
/// pixel buffer from WASM linear memory to the framebuffer in one lock cycle.
/// Pixels are stored as little-endian u32 values in WASM memory.
fn host_fb_blit(vstack: &mut [i64], vsp: &mut usize, mem: &mut [u8]) -> Result<(), InterpError> {
    if *vsp < 3 { return Err(InterpError::StackUnderflow); }
    let height = vstack[*vsp - 1] as usize;
    let width  = vstack[*vsp - 2] as usize;
    let ptr    = vstack[*vsp - 3] as usize;
    *vsp -= 3;
    let pixel_count = width.saturating_mul(height);
    let byte_count  = pixel_count.saturating_mul(4);
    if ptr.saturating_add(byte_count) > mem.len() { return Ok(()); }
    // Reinterpret the WASM byte slice as a &[u32] (little-endian, 4-byte aligned).
    // WASM linear memory is always aligned to at least 4 bytes at ptr==0, and
    // our buffer is Vec<u32> so ptr is 4-byte aligned.
    let pixels: &[u32] = unsafe {
        core::slice::from_raw_parts(mem.as_ptr().add(ptr) as *const u32, pixel_count)
    };
    crate::vga::blit_rgb32(pixels, width, height);
    Ok(())
}

fn host_net_listen(vstack: &mut [i64], vsp: &mut usize, _mem: &mut [u8]) -> Result<(), InterpError> {
    if *vsp < 1 { return Err(InterpError::StackUnderflow); }
    let port = vstack[*vsp - 1] as u16;
    *vsp -= 1;

    let result = crate::drivers::netstack::with_network(|stack| {
        stack.tcp_listen(port)
    }).unwrap_or(None);

    vstack[*vsp] = match result {
        Some(idx) => idx as i64,
        None => -1i64,
    };
    *vsp += 1;
    Ok(())
}

fn host_net_connect(vstack: &mut [i64], vsp: &mut usize, _mem: &mut [u8]) -> Result<(), InterpError> {
    if *vsp < 2 { return Err(InterpError::StackUnderflow); }
    let port = vstack[*vsp - 1] as u16;
    let ip_bytes = vstack[*vsp - 2] as u32;
    *vsp -= 2;

    let ip = crate::drivers::netstack::IpAddr::from_u32(ip_bytes);
    let result = crate::drivers::netstack::with_network(|stack| {
        stack.tcp_connect(ip, port)
    }).unwrap_or(None);

    vstack[*vsp] = match result {
        Some(idx) => idx as i64,
        None => -1i64,
    };
    *vsp += 1;
    Ok(())
}

fn host_net_send(vstack: &mut [i64], vsp: &mut usize, mem: &mut [u8]) -> Result<(), InterpError> {
    if *vsp < 3 { return Err(InterpError::StackUnderflow); }
    let len = vstack[*vsp - 1] as usize;
    let ptr = vstack[*vsp - 2] as usize;
    let handle = vstack[*vsp - 3] as usize;
    *vsp -= 3;

    let data = &mem[ptr..ptr.saturating_add(len)];
    let result = crate::drivers::netstack::with_network(|stack| {
        stack.tcp_send(handle, data)
    }).unwrap_or(Err(()));

    vstack[*vsp] = match result {
        Ok(n) => n as i64,
        Err(_) => -1i64,
    };
    *vsp += 1;
    Ok(())
}

fn host_net_recv(vstack: &mut [i64], vsp: &mut usize, mem: &mut [u8]) -> Result<(), InterpError> {
    if *vsp < 3 { return Err(InterpError::StackUnderflow); }
    let cap = vstack[*vsp - 1] as usize;
    let ptr = vstack[*vsp - 2] as usize;
    let handle = vstack[*vsp - 3] as usize;
    *vsp -= 3;

    let buf = &mut mem[ptr..ptr.saturating_add(cap)];
    let result = crate::drivers::netstack::with_network(|stack| {
        stack.tcp_recv(handle, buf)
    }).unwrap_or(Err(()));

    vstack[*vsp] = match result {
        Ok(n) => n as i64,
        Err(_) => -1i64,
    };
    *vsp += 1;
    Ok(())
}

fn host_net_close(vstack: &mut [i64], vsp: &mut usize, _mem: &mut [u8]) -> Result<(), InterpError> {
    if *vsp < 1 { return Err(InterpError::StackUnderflow); }
    let handle = vstack[*vsp - 1] as usize;
    *vsp -= 1;

    let result = crate::drivers::netstack::with_network(|stack| {
        stack.tcp_close(handle)
    }).unwrap_or(Err(()));

    vstack[*vsp] = match result {
        Ok(()) => 0i64,
        Err(_) => -1i64,
    };
    *vsp += 1;
    Ok(())
}

fn host_net_set_ip(vstack: &mut [i64], vsp: &mut usize, _mem: &mut [u8]) -> Result<(), InterpError> {
    if *vsp < 1 { return Err(InterpError::StackUnderflow); }
    let ip_bytes = vstack[*vsp - 1] as u32;
    *vsp -= 1;

    let ip = crate::drivers::netstack::IpAddr::from_u32(ip_bytes);
    crate::drivers::netstack::with_network(|stack| {
        stack.set_ip(ip);
    });

    vstack[*vsp] = 0i64;
    *vsp += 1;
    Ok(())
}

/// `net_accept(listen_handle: i32) → i32`
/// If the listening socket has completed a handshake, move the established
/// connection into a new slot, reset the listener, and return the new handle.
/// Returns -1 if the connection is not yet ready.
fn host_net_accept(vstack: &mut [i64], vsp: &mut usize, _mem: &mut [u8]) -> Result<(), InterpError> {
    if *vsp < 1 { return Err(InterpError::StackUnderflow); }
    let handle = vstack[*vsp - 1] as usize;
    *vsp -= 1;

    let result = crate::drivers::netstack::with_network(|stack| {
        stack.tcp_accept(handle)
    }).unwrap_or(None);

    vstack[*vsp] = match result {
        Some(idx) => idx as i64,
        None      => -1i64,
    };
    *vsp += 1;
    Ok(())
}

/// `net_status(handle: i32) → i32`
/// Returns the current TCP socket state:
///   0 = closed/invalid, 1 = listening, 2 = handshaking, 3 = established,
///   4 = half-closed / teardown
fn host_net_status(vstack: &mut [i64], vsp: &mut usize, _mem: &mut [u8]) -> Result<(), InterpError> {
    if *vsp < 1 { return Err(InterpError::StackUnderflow); }
    let handle = vstack[*vsp - 1] as usize;
    *vsp -= 1;

    let status = crate::drivers::netstack::with_network(|stack| {
        stack.tcp_status(handle)
    }).unwrap_or(0);

    vstack[*vsp] = status as i64;
    *vsp += 1;
    Ok(())
}

/// `net_get_ip() → i32`
/// Return the kernel's current IPv4 address as a u32 (little-endian octets,
/// same encoding as `net_connect`'s ip argument).  Returns 0 if DHCP has not
/// yet bound an address.
fn host_net_get_ip(vstack: &mut [i64], vsp: &mut usize, _mem: &mut [u8]) -> Result<(), InterpError> {
    if *vsp >= STACK_DEPTH { return Err(InterpError::StackOverflow); }
    let ip = crate::drivers::netstack::with_network(|stack| stack.get_ip()).unwrap_or(0);
    vstack[*vsp] = ip as i64;
    *vsp += 1;
    Ok(())
}

/// `net_udp_bind(port: i32) → i32`
/// Bind a UDP socket to `port`.  Returns a handle (≥ 0) or -1 on failure.
fn host_net_udp_bind(vstack: &mut [i64], vsp: &mut usize, _mem: &mut [u8]) -> Result<(), InterpError> {
    if *vsp < 1 { return Err(InterpError::StackUnderflow); }
    let port = vstack[*vsp - 1] as u16;
    *vsp -= 1;

    let result = crate::drivers::netstack::with_network(|stack| {
        stack.udp_bind(port)
    }).unwrap_or(None);

    vstack[*vsp] = match result {
        Some(idx) => idx as i64,
        None      => -1i64,
    };
    *vsp += 1;
    Ok(())
}

/// `net_udp_connect(handle: i32, ip: i32, port: i32) → i32`
/// Set the remote address/port for a bound UDP socket so `net_udp_send` knows
/// where to deliver datagrams.  Returns 0 on success, -1 on failure.
fn host_net_udp_connect(vstack: &mut [i64], vsp: &mut usize, _mem: &mut [u8]) -> Result<(), InterpError> {
    if *vsp < 3 { return Err(InterpError::StackUnderflow); }
    let port     = vstack[*vsp - 1] as u16;
    let ip_bytes = vstack[*vsp - 2] as u32;
    let handle   = vstack[*vsp - 3] as usize;
    *vsp -= 3;

    let ip = crate::drivers::netstack::IpAddr::from_u32(ip_bytes);
    let result = crate::drivers::netstack::with_network(|stack| {
        stack.udp_connect(handle, ip, port)
    }).unwrap_or(Err(()));

    vstack[*vsp] = match result { Ok(()) => 0i64, Err(_) => -1i64 };
    *vsp += 1;
    Ok(())
}

/// `net_udp_send(handle: i32, ptr: i32, len: i32) → i32`
/// Send `len` bytes from linear memory via a UDP socket.
/// Returns bytes sent (≥ 0) or -1 on error.
fn host_net_udp_send(vstack: &mut [i64], vsp: &mut usize, mem: &mut [u8]) -> Result<(), InterpError> {
    if *vsp < 3 { return Err(InterpError::StackUnderflow); }
    let len    = vstack[*vsp - 1] as usize;
    let ptr    = vstack[*vsp - 2] as usize;
    let handle = vstack[*vsp - 3] as usize;
    *vsp -= 3;

    let end  = ptr.saturating_add(len).min(mem.len());
    let data = mem[ptr..end].to_vec();  // copy out before the closure borrows stack
    let result = crate::drivers::netstack::with_network(|stack| {
        stack.udp_send(handle, &data)
    }).unwrap_or(Err(()));

    vstack[*vsp] = match result { Ok(n) => n as i64, Err(_) => -1i64 };
    *vsp += 1;
    Ok(())
}

/// `net_udp_recv(handle: i32, ptr: i32, cap: i32) → i32`
/// Non-blocking receive into linear memory.
/// Returns bytes received (≥ 0), 0 if the buffer is empty, or -1 on error.
fn host_net_udp_recv(vstack: &mut [i64], vsp: &mut usize, mem: &mut [u8]) -> Result<(), InterpError> {
    if *vsp < 3 { return Err(InterpError::StackUnderflow); }
    let cap    = vstack[*vsp - 1] as usize;
    let ptr    = vstack[*vsp - 2] as usize;
    let handle = vstack[*vsp - 3] as usize;
    *vsp -= 3;

    let end = ptr.saturating_add(cap).min(mem.len());
    let buf = &mut mem[ptr..end];
    let result = crate::drivers::netstack::with_network(|stack| {
        stack.udp_recv(handle, buf)
    }).unwrap_or(Err(()));

    vstack[*vsp] = match result { Ok(n) => n as i64, Err(_) => 0i64 };
    *vsp += 1;
    Ok(())
}

/// `net_udp_close(handle: i32) → i32`
/// Release a UDP socket slot.  Always returns 0.
fn host_net_udp_close(vstack: &mut [i64], vsp: &mut usize, _mem: &mut [u8]) -> Result<(), InterpError> {
    if *vsp < 1 { return Err(InterpError::StackUnderflow); }
    let handle = vstack[*vsp - 1] as usize;
    *vsp -= 1;

    crate::drivers::netstack::with_network(|stack| stack.udp_close(handle));

    vstack[*vsp] = 0i64;
    *vsp += 1;
    Ok(())
}

/// Register the kernel's built-in host functions.  Call once at boot before
/// running any module.
pub fn init_host_fns() {
    // BSS may not be zeroed by the bootloader — reset the registry explicitly.
    unsafe {
        HOST_COUNT    = 0;
        HOST_REGISTRY = [EMPTY_ENTRY; MAX_HOST_FUNCS];
    }
    register_host("env", "print",      host_print);
    register_host("env", "print_int",  host_print_int);
    register_host("env", "print_i64",  host_print_i64);
    register_host("env", "print_char", host_print_char);
    register_host("env", "print_hex",  host_print_hex);
    register_host("env", "yield",      host_yield);
    register_host("env", "sleep_ms",   host_sleep_ms);
    register_host("env", "uptime_ms",   host_uptime_ms);
    register_host("env", "wasm_opcount", host_wasm_opcount);
    register_host("env", "exit",       host_exit);
    register_host("env", "read_char",  host_read_char);
    register_host("env", "read_line",  host_read_line);
    register_host("env", "fs_read",    host_fs_read);
    register_host("env", "fs_write",   host_fs_write);
    register_host("env", "fs_size",    host_fs_size);
    register_host("env", "args_get",     host_args_get);
    register_host("env", "fb_set_pixel", host_fb_set_pixel);
    register_host("env", "fb_present",   host_fb_present);
    register_host("env", "fb_blit",      host_fb_blit);

    register_host("net", "listen",      host_net_listen);
    register_host("net", "connect",     host_net_connect);
    register_host("net", "send",        host_net_send);
    register_host("net", "recv",        host_net_recv);
    register_host("net", "close",       host_net_close);
    register_host("net", "set_ip",      host_net_set_ip);
    register_host("net", "accept",      host_net_accept);
    register_host("net", "status",      host_net_status);
    register_host("net", "get_ip",      host_net_get_ip);
    register_host("net", "udp_bind",    host_net_udp_bind);
    register_host("net", "udp_connect", host_net_udp_connect);
    register_host("net", "udp_send",    host_net_udp_send);
    register_host("net", "udp_recv",    host_net_udp_recv);
    register_host("net", "udp_close",   host_net_udp_close);
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

/// Print an i64 as decimal followed by a newline, without heap allocation.
fn fmt_i64(n: i64) {
    let mut buf = [0u8; 20];
    let mut pos = buf.len();
    let negative = n < 0;
    let mut val: u64 = if n == i64::MIN { 9223372036854775808 }
                       else if negative { (-n) as u64 }
                       else             { n as u64 };
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

/// Print a u32 as `0x` + 8 uppercase hex digits, followed by a newline.
fn fmt_hex(n: u32) {
    const HEX: &[u8] = b"0123456789ABCDEF";
    let mut buf = [0u8; 10]; // "0x" + 8 digits
    buf[0] = b'0';
    buf[1] = b'x';
    for i in 0..8 {
        buf[9 - i] = HEX[((n >> (i * 4)) & 0xF) as usize];
    }
    if let Ok(s) = core::str::from_utf8(&buf) {
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

/// Count the number of function imports in `import_section`.
///
/// Returns `0` if the section is `None` or malformed.  Used during
/// [`spawn`] to determine the import/defined split in the function index space.
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

/// Instantiate a WASM module into the first free pool slot.
///
/// Returns the pool slot index (the *handle*) on success.  The handle is
/// used with [`start_task`], [`resume_task`], [`destroy`], and
/// [`for_each_instance`].
///
/// # Steps performed
///
/// 1. Find a free slot; fail with [`RunError::PoolFull`] if none available.
/// 2. Parse `bytes` with [`loader::load`].
/// 3. Validate `min_pages <= MAX_MEM_PAGES`; fail with [`RunError::MemoryTooLarge`] otherwise.
/// 4. Zero the slot's `SLOT_MEM` region.
/// 5. Resolve all function imports against the host registry; fail with
///    [`RunError::ImportNotFound`] if any import is unregistered.
/// 6. Construct an [`Interpreter`](crate::wasm::interp::Interpreter) and
///    apply the data section initializers to linear memory.
/// 7. Mark the slot as active.
pub fn spawn(name: &str, bytes: &'static [u8]) -> Result<usize, RunError> {
    // Find a free slot.
    let slot = unsafe {
        (*core::ptr::addr_of!(POOL)).iter().position(|s| !s.active).ok_or(RunError::PoolFull)?
    };

    let module = load(bytes).map_err(RunError::Load)?;

    let min_pages = module.memory_section
        .map(read_memory_min_pages)
        .unwrap_or(0);
    if min_pages > MAX_MEM_PAGES { return Err(RunError::MemoryTooLarge); }

    // Zero the entire slot so memory.grow pages are already clean.
    // SAFETY: single-threaded kernel; slot is verified free above.
    let mem: &'static mut [u8] = unsafe {
        SLOT_MEM[slot].fill(0);
        &mut SLOT_MEM[slot][..]
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

    let mut interp = Interpreter::new(&module, import_count, mem, host_fns, min_pages)
        .map_err(RunError::Interp)?;

    if let Some(data) = module.data_section {
        if !init_memory(data, &mut interp.mem) {
            return Err(RunError::Interp(InterpError::MalformedCode));
        }
    }

    // JIT-compile all function bodies that the compiler supports.
    // Functions that fall back to the interpreter are silently skipped.
    interp.compile_jit();

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

/// Free a pool slot, drop the instance, and zero its linear memory.
///
/// No-op if `handle >= MAX_INSTANCES` or the slot is already inactive.
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
///
/// Used by the `ps` shell command to display running instances.
pub fn for_each_instance<F: FnMut(usize, &str, usize)>(mut f: F) {
    unsafe {
        for (i, s) in (*core::ptr::addr_of!(POOL)).iter().enumerate() {
            if s.active {
                let name = core::str::from_utf8(&s.name[..s.name_len]).unwrap_or("?");
                f(i, name, s.mem_pages);
            }
        }
    }
}

// ── Task execution helpers ────────────────────────────────────────────────────

/// Outcome of a single execution step for a cooperative task.
pub enum TaskResult {
    /// The entry function returned normally; holds the top-of-stack value
    /// (the function's return value), if any.
    Completed(Option<i64>),
    /// The module called `env.yield` or `env.sleep_ms` and surrendered the CPU.
    /// Call [`resume_task`] to continue execution from the same point.
    Yielded,
}

/// Begin executing the export named `entry` on an already-spawned instance.
///
/// Pushes `args` onto the value stack before calling.  Returns
/// [`TaskResult::Yielded`] if the module calls `env.yield` or `env.sleep_ms`
/// before returning; call [`resume_task`] to continue from where it stopped.
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
        Err(InterpError::Exited)           => Ok(TaskResult::Completed(None)),
        Err(e)                             => Err(RunError::Interp(e)),
    }
}

/// Continue a suspended task from the exact point it yielded.
///
/// The interpreter's frame/value/control stacks are preserved between
/// calls — no re-parsing or re-initialisation is needed.
pub fn resume_task(handle: usize) -> Result<TaskResult, RunError> {
    if handle >= MAX_INSTANCES { return Err(RunError::EntryNotFound); }
    let inst = unsafe {
        if !POOL[handle].active { return Err(RunError::EntryNotFound); }
        POOL[handle].inst.assume_init_mut()
    };
    match inst.interp.resume() {
        Ok(())                             => Ok(TaskResult::Completed(inst.interp.top())),
        Err(InterpError::Yielded)          => Ok(TaskResult::Yielded),
        Err(InterpError::Exited)           => Ok(TaskResult::Completed(None)),
        Err(e)                             => Err(RunError::Interp(e)),
    }
}

// ── Convenience wrapper ───────────────────────────────────────────────────────

/// Convenience wrapper: spawn, execute `entry` to completion, then destroy.
///
/// Cooperative yields (`env.yield`, `env.sleep_ms`) are drained synchronously —
/// the module is immediately resumed after each yield.  This is suitable for
/// short synchronous invocations from the shell (`run` command); long-running
/// or truly concurrent modules should use [`spawn`] + [`start_task`] +
/// [`task`](crate::wasm::task) instead.
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

