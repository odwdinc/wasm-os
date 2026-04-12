// Point directly at the kernel source files.  #[path] makes each file *be*
// the module, so `//!` inner-doc comments and `use super::…` paths both
// resolve exactly as they do in the kernel — zero changes to kernel code.
#[path = "../../kernel/src/wasm/loader.rs"]
pub mod loader;

#[path = "../../kernel/src/wasm/interp.rs"]
pub mod interp;

#[path = "../../kernel/src/wasm/opcode.rs"]
pub mod opcode;

use std::cell::RefCell;
use loader::{load, find_export, read_memory_min_pages, read_u32_leb128};
pub use interp::{Interpreter, InterpError, HostFn, MAX_FUNCS, read_i32_leb128};

const PAGE_SIZE: usize = 65536;

// ── Error type ────────────────────────────────────────────────────────────────

#[derive(Debug)]
pub enum TestError {
    Load(String),
    Interp(String),
    ExportNotFound(String),
}

impl std::fmt::Display for TestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TestError::Load(s)           => write!(f, "load error: {s}"),
            TestError::Interp(s)         => write!(f, "interp error: {s}"),
            TestError::ExportNotFound(s) => write!(f, "export not found: {s}"),
        }
    }
}

// ── run_wasm — pure-compute helper (no host imports) ─────────────────────────

/// Instantiate `wasm`, call `func` with `args`, return the top-of-stack value.
/// Use this for modules with no host imports (pure arithmetic / logic).
pub fn run_wasm(wasm: &[u8], func: &str, args: &[i32]) -> Result<Option<i64>, TestError> {
    let module = load(wasm).map_err(|e| TestError::Load(e.as_str().to_string()))?;

    let min_pages = module.memory_section.map(read_memory_min_pages).unwrap_or(0);
    let mem_size  = (min_pages as usize).max(1) * PAGE_SIZE;
    let mut mem_buf = vec![0u8; mem_size];

    let host_fns: [Option<HostFn>; MAX_FUNCS] = [None; MAX_FUNCS];

    let mut interp = Interpreter::new(&module, 0, &mut mem_buf, host_fns, min_pages)
        .map_err(|e| TestError::Interp(e.as_str().to_string()))?;

    if let Some(data) = module.data_section {
        init_data(data, &mut interp.mem);
    }

    let func_idx = find_export(&module, func)
        .ok_or_else(|| TestError::ExportNotFound(func.to_string()))? as usize;

    interp.reset_for_call();
    for &arg in args {
        interp.vstack[interp.vsp] = arg as i64;
        interp.vsp += 1;
    }

    interp.call(func_idx).map_err(|e| TestError::Interp(e.as_str().to_string()))?;
    Ok(interp.top())
}

// ── run_app — app helper (wires env host functions, captures output) ──────────

// `HostFn` is a bare fn pointer — it can't close over state.  We use a
// thread-local buffer so the capture fns and `run_app` share state without
// unsafe globals.  Each test thread gets its own buffer, so parallel tests
// don't interfere.
thread_local! {
    static OUTPUT: RefCell<Vec<u8>> = RefCell::new(Vec::new());
}

fn append_output(bytes: &[u8]) {
    OUTPUT.with(|o| o.borrow_mut().extend_from_slice(bytes));
    // Mirror to stderr so output is visible with `cargo test -- --nocapture`
    // (or always, since eprintln! bypasses libtest's stdout capture).
    eprint!("{}", String::from_utf8_lossy(bytes));
}

fn host_print(vstack: &mut [i64], vsp: &mut usize, mem: &mut [u8]) -> Result<(), InterpError> {
    if *vsp < 2 { return Err(InterpError::StackUnderflow); }
    let len = vstack[*vsp - 1] as usize;
    let ptr = vstack[*vsp - 2] as usize;
    *vsp -= 2;
    let end = ptr.saturating_add(len).min(mem.len());
    append_output(&mem[ptr..end]);
    Ok(())
}

fn host_print_int(vstack: &mut [i64], vsp: &mut usize, _mem: &mut [u8]) -> Result<(), InterpError> {
    if *vsp < 1 { return Err(InterpError::StackUnderflow); }
    let n = vstack[*vsp - 1] as i32;
    *vsp -= 1;
    append_output(format!("{n}\n").as_bytes());
    Ok(())
}

fn host_print_i64(vstack: &mut [i64], vsp: &mut usize, _mem: &mut [u8]) -> Result<(), InterpError> {
    if *vsp < 1 { return Err(InterpError::StackUnderflow); }
    let n = vstack[*vsp - 1];
    *vsp -= 1;
    append_output(format!("{n}\n").as_bytes());
    Ok(())
}

fn host_print_char(vstack: &mut [i64], vsp: &mut usize, _mem: &mut [u8]) -> Result<(), InterpError> {
    if *vsp < 1 { return Err(InterpError::StackUnderflow); }
    let c = (vstack[*vsp - 1] & 0xFF) as u8;
    *vsp -= 1;
    append_output(&[c]);
    Ok(())
}

fn host_print_hex(vstack: &mut [i64], vsp: &mut usize, _mem: &mut [u8]) -> Result<(), InterpError> {
    if *vsp < 1 { return Err(InterpError::StackUnderflow); }
    let n = vstack[*vsp - 1] as u32;
    *vsp -= 1;
    append_output(format!("0x{n:08X}\n").as_bytes());
    Ok(())
}

fn host_sleep_ms(_vstack: &mut [i64], vsp: &mut usize, _mem: &mut [u8]) -> Result<(), InterpError> {
    if *vsp < 1 { return Err(InterpError::StackUnderflow); }
    *vsp -= 1;
    Err(InterpError::Yielded) // cooperative yield; run_app resumes automatically
}

fn host_yield_fn(_vstack: &mut [i64], _vsp: &mut usize, _mem: &mut [u8]) -> Result<(), InterpError> {
    Err(InterpError::Yielded)
}

fn host_uptime_ms(vstack: &mut [i64], vsp: &mut usize, _mem: &mut [u8]) -> Result<(), InterpError> {
    if *vsp >= crate::interp::STACK_DEPTH { return Err(InterpError::StackOverflow); }
    vstack[*vsp] = 0;  // stub: always 0 in test context
    *vsp += 1;
    Ok(())
}

#[macro_export]
macro_rules! println {
    ($($arg:tt)*) => {        
        #[cfg(test)]
        {
            // Use standard println or a test-specific implementation
            std::println!($($arg)*);
        }
    };
}

/// Instantiate `wasm`, wire up the kernel's `env` host functions
/// (`print`, `print_int`, `sleep_ms`, `yield`), call `"main"` with `args`,
/// and return everything written via print/print_int as a `String`.
///
/// Cooperative yields from `sleep_ms` / `yield` are handled transparently —
/// the interpreter is resumed until the function returns normally.
pub fn run_app(wasm: &[u8], args: &[i32]) -> Result<String, TestError> {
    OUTPUT.with(|o| o.borrow_mut().clear());

    let module = load(wasm).map_err(|e| TestError::Load(e.as_str().to_string()))?;

    let min_pages = module.memory_section.map(read_memory_min_pages).unwrap_or(0);
    let mem_size  = (min_pages as usize).max(1) * PAGE_SIZE;
    let mut mem_buf = vec![0u8; mem_size];

    // Resolve imports in declaration order (same as engine::spawn).
    let mut host_fns: [Option<HostFn>; MAX_FUNCS] = [None; MAX_FUNCS];
    let mut import_count = 0usize;
    if let Some(sec) = module.import_section {
        loader::for_each_func_import(sec, &mut |mod_name, func_name| {
            host_fns[import_count] = match (mod_name, func_name) {
                ("env", "print")      => Some(host_print      as HostFn),
                ("env", "print_int")  => Some(host_print_int  as HostFn),
                ("env", "print_i64")  => Some(host_print_i64  as HostFn),
                ("env", "print_char") => Some(host_print_char as HostFn),
                ("env", "print_hex")  => Some(host_print_hex  as HostFn),
                ("env", "sleep_ms")   => Some(host_sleep_ms   as HostFn),
                ("env", "yield")      => Some(host_yield_fn   as HostFn),
                ("env", "uptime_ms")  => Some(host_uptime_ms  as HostFn),
                _                     => None,
            };
            import_count += 1;
        });
    }

    let mut interp = Interpreter::new(&module, import_count, &mut mem_buf, host_fns, min_pages)
        .map_err(|e| TestError::Interp(e.as_str().to_string()))?;

    if let Some(data) = module.data_section {
        init_data(data, &mut interp.mem);
    }

    let func_idx = find_export(&module, "main")
        .ok_or_else(|| TestError::ExportNotFound("main".to_string()))? as usize;

    interp.reset_for_call();
    for &arg in args {
        interp.vstack[interp.vsp] = arg as i64;
        interp.vsp += 1;
    }

    // Run to completion, resuming through any cooperative yields.
    let mut result = interp.call(func_idx);
    while let Err(InterpError::Yielded) = result {
        result = interp.resume();
    }
    result.map_err(|e| TestError::Interp(e.as_str().to_string()))?;

    let out = OUTPUT.with(|o| String::from_utf8_lossy(&o.borrow()).into_owned());
    Ok(out)
}

// ── Data-section initialiser ─────────────────────────────────────────────────

fn init_data(data: &[u8], mem: &mut [u8]) {
    let Some((count, n)) = read_u32_leb128(data) else { return };
    let mut cur = n;

    for _ in 0..count as usize {
        let Some((kind, n)) = read_u32_leb128(&data[cur..]) else { return };
        cur += n;

        if kind != 0 { continue; }

        if cur >= data.len() || data[cur] != 0x41 { return; }
        cur += 1;
        let Some((offset, n)) = read_i32_leb128(&data[cur..]) else { return };
        cur += n;
        if cur >= data.len() || data[cur] != 0x0B { return; }
        cur += 1;

        let Some((seg_len, n)) = read_u32_leb128(&data[cur..]) else { return };
        cur += n;
        let seg_len = seg_len as usize;

        let dst   = offset as usize;
        let avail = mem.len().saturating_sub(dst);
        let copy  = seg_len.min(avail);
        if copy > 0 && cur + seg_len <= data.len() {
            mem[dst..dst + copy].copy_from_slice(&data[cur..cur + copy]);
        }
        cur += seg_len;
    }
}
