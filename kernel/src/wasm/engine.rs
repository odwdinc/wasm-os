//! Execution engine — Sprint 2.3 / 2.4 / 2.5
//!
//! `run(bytes, func_idx)` — parse, init memory, dispatch host calls, execute.
//! `HELLO_WASM`           — embedded test module that prints "Hello from WASM!\n".

use super::loader::{load, find_export, LoadError, read_u32_leb128};
use super::interp::{Interpreter, InterpError, read_i32_leb128};

// ── Error type ───────────────────────────────────────────────────────────────

pub enum RunError {
    Load(LoadError),
    Interp(InterpError),
    EntryNotFound,
}

impl RunError {
    pub fn as_str(&self) -> &'static str {
        match self {
            RunError::Load(e)   => e.as_str(),
            RunError::Interp(e) => e.as_str(),
            RunError::EntryNotFound => "entry point not found in exports",
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
            // print(ptr: i32, len: i32) — len is on top.
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
            // print_int(n: i32)
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
    let mut buf = [0u8; 11]; // max 11 chars: "-2147483648"
    let mut pos = buf.len();
    let negative = n < 0;
    // Use u32 arithmetic to handle i32::MIN correctly.
    let mut val: u32 = if n == i32::MIN { 2147483648 } else if negative { (-n) as u32 } else { n as u32 };
    if val == 0 {
        crate::println!("0");
        return;
    }
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

/// Parse the data section and copy active segments into `mem`.
/// Returns `false` only on a malformed section (missing bytes / bad offsets).
fn init_memory(data_section: &[u8], mem: &mut [u8]) -> bool {
    let mut cur = 0usize;

    let (count, n) = match read_u32_leb128(data_section) {
        Some(x) => x, None => return false,
    };
    cur += n;

    for _ in 0..count as usize {
        // Segment kind: 0 = active mem-0, 1 = passive, 2 = active explicit
        let (kind, n) = match read_u32_leb128(&data_section[cur..]) {
            Some(x) => x, None => return false,
        };
        cur += n;

        if kind == 0 {
            // Offset init-expression: expect  0x41 <sleb128>  0x0B
            if cur >= data_section.len() || data_section[cur] != 0x41 { return false; }
            cur += 1;
            let (offset, n) = match read_i32_leb128(&data_section[cur..]) {
                Some(x) => x, None => return false,
            };
            cur += n;
            if cur >= data_section.len() || data_section[cur] != 0x0B { return false; }
            cur += 1; // consume `end`

            // Data bytes
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
        // passive / explicit-memory segments: skip for now
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
            1 => { // table: reftype + limits
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
            2 => { // memory: limits
                if cur >= bytes.len() { return func_count; }
                let flag = bytes[cur]; cur += 1;
                let (_, n) = match read_u32_leb128(&bytes[cur..]) { Some(x) => x, None => return func_count };
                cur += n;
                if flag != 0 {
                    let (_, n) = match read_u32_leb128(&bytes[cur..]) { Some(x) => x, None => return func_count };
                    cur += n;
                }
            }
            3 => { cur += 2; } // global: valtype + mutability
            _  => return func_count,
        }
    }
    func_count
}

// ── Public entry point ────────────────────────────────────────────────────────

/// Parse `bytes` as a WASM binary, initialise linear memory from data segments,
/// wire up host functions, look up `entry` in the export section, execute it,
/// and return the top of the value stack (if any) after execution.
pub fn run(bytes: &[u8], entry: &str, args: &[i32]) -> Result<Option<i32>, RunError> {
    let module       = load(bytes).map_err(RunError::Load)?;
    let import_count = count_func_imports(module.import_section);

    let func_idx = find_export(&module, entry)
        .ok_or(RunError::EntryNotFound)? as usize;

    let mut interp = Interpreter::new(&module, import_count)
        .map_err(RunError::Interp)?;

    interp.host_fn = kernel_host;

    if let Some(data) = module.data_section {
        if !init_memory(data, &mut interp.mem) {
            return Err(RunError::Interp(InterpError::MalformedCode));
        }
    }

    // Push caller-supplied arguments onto the value stack before the call.
    for &arg in args {
        if interp.vsp >= interp.vstack.len() {
            return Err(RunError::Interp(InterpError::StackOverflow));
        }
        interp.vstack[interp.vsp] = arg as i64;
        interp.vsp += 1;
    }

    interp.call(func_idx).map_err(RunError::Interp)?;
    Ok(interp.top_i32())
}

// ── Embedded userland modules ─────────────────────────────────────────────────
//
// Source lives under userland/; run tools/wasm-pack.sh to compile .wat → .wasm
// before building the kernel.

pub const HELLO_WASM:  &[u8] = include_bytes!("../../../userland/hello/hello.wasm");
pub const GREET_WASM:  &[u8] = include_bytes!("../../../userland/greet/greet.wasm");
pub const FIB_WASM:    &[u8] = include_bytes!("../../../userland/fib/fib.wasm");
pub const PRIMES_WASM: &[u8] = include_bytes!("../../../userland/primes/primes.wasm");
