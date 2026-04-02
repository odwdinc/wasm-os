//! Execution engine — Sprint 2.3 / 2.4 / 2.5
//!
//! `run(bytes, func_idx)` — parse, init memory, dispatch host calls, execute.
//! `HELLO_WASM`           — embedded test module that prints "Hello from WASM!\n".

use super::loader::{load, LoadError, read_u32_leb128};
use super::interp::{Interpreter, InterpError, read_i32_leb128};

// ── Error type ───────────────────────────────────────────────────────────────

pub enum RunError {
    Load(LoadError),
    Interp(InterpError),
}

impl RunError {
    pub fn as_str(&self) -> &'static str {
        match self {
            RunError::Load(e)   => e.as_str(),
            RunError::Interp(e) => e.as_str(),
        }
    }
}

// ── Host functions ────────────────────────────────────────────────────────────

/// Kernel host dispatch.  Currently supports one import:
///   index 0 — `print(ptr: i32, len: i32)`  writes UTF-8 from linear memory.
fn kernel_host(
    func_idx: usize,
    vstack:   &mut [i32],
    vsp:      &mut usize,
    mem:      &mut [u8],
) -> Result<(), InterpError> {
    match func_idx {
        0 => {
            // print(ptr: i32, len: i32)
            // WASM pushes ptr first, len second → len is on top.
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
        _ => Err(InterpError::IsImport),
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

fn count_func_imports(import_section: Option<&[u8]>) -> usize {
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
/// wire up host functions, call `func_idx`, and return the top of the value
/// stack (if any) after execution.
pub fn run(bytes: &[u8], func_idx: usize) -> Result<Option<i32>, RunError> {
    let module       = load(bytes).map_err(RunError::Load)?;
    let import_count = count_func_imports(module.import_section);

    let mut interp = Interpreter::new(&module, import_count)
        .map_err(RunError::Interp)?;

    interp.host_fn = kernel_host;

    if let Some(data) = module.data_section {
        if !init_memory(data, &mut interp.mem) {
            return Err(RunError::Interp(InterpError::MalformedCode));
        }
    }

    interp.call(func_idx).map_err(RunError::Interp)?;
    Ok(interp.top_i32())
}

// ── Embedded test module (Sprint 2.5) ────────────────────────────────────────
//
// WAT equivalent:
//   (module
//     (import "env" "print" (func (param i32 i32)))   ;; func 0 (import)
//     (memory 1)
//     (data (i32.const 0) "Hello from WASM!\n")        ;; 17 bytes at offset 0
//     (func                                             ;; func 1 (defined)
//       i32.const 0   ;; ptr
//       i32.const 17  ;; len
//       call 0)       ;; call print
//   )
//
// To run: engine::run(HELLO_WASM, 1)
pub const HELLO_WASM: &[u8] = &[
    // ── header ──────────────────────────────────────────────────────────────
    0x00, 0x61, 0x73, 0x6D,  // magic "\0asm"
    0x01, 0x00, 0x00, 0x00,  // version 1

    // ── type section (id=1, size=9) — 2 types ───────────────────────────────
    0x01, 0x09,
    0x02,                          // 2 types
    0x60, 0x02, 0x7F, 0x7F, 0x00, // type 0: (i32 i32) -> ()  [print]
    0x60, 0x00, 0x00,              // type 1: ()       -> ()  [main]

    // ── import section (id=2, size=13) — "env"."print" func type 0 ──────────
    0x02, 0x0D,
    0x01,                                        // 1 import
    0x03, 0x65, 0x6E, 0x76,                      // "env"
    0x05, 0x70, 0x72, 0x69, 0x6E, 0x74,          // "print"
    0x00, 0x00,                                  // func, type index 0

    // ── function section (id=3, size=2) — 1 function, type 1 ─────────────────
    0x03, 0x02,
    0x01, 0x01,

    // ── memory section (id=5, size=3) — 1 page minimum ───────────────────────
    0x05, 0x03,
    0x01, 0x00, 0x01,

    // ── code section (id=10, size=10) ────────────────────────────────────────
    // body: 0 locals | i32.const 0 | i32.const 17 | call 0 | end
    0x0A, 0x0A,
    0x01,        // 1 body
    0x08,        // body size = 8
    0x00,        // 0 local groups
    0x41, 0x00,  // i32.const 0   (ptr)
    0x41, 0x11,  // i32.const 17  (len)
    0x10, 0x00,  // call 0        (print)
    0x0B,        // end

    // ── data section (id=11, size=23) ────────────────────────────────────────
    // active segment, mem 0, offset 0, "Hello from WASM!\n"
    0x0B, 0x17,
    0x01,              // 1 segment
    0x00,              // kind = active mem-0
    0x41, 0x00, 0x0B,  // offset: i32.const 0; end
    0x11,              // 17 bytes
    0x48, 0x65, 0x6C, 0x6C, 0x6F, 0x20,  // "Hello "
    0x66, 0x72, 0x6F, 0x6D, 0x20,        // "from "
    0x57, 0x41, 0x53, 0x4D, 0x21, 0x0A,  // "WASM!\n"
];
