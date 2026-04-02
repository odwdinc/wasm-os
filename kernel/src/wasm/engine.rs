//! Execution engine — Sprint 2.3
//!
//! Wires the loader and interpreter together into a single entry point.
//! `run(bytes, func_idx)` is all a caller needs.

use super::loader::{load, LoadError, read_u32_leb128};
use super::interp::{Interpreter, InterpError};

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

// ── Public entry point ────────────────────────────────────────────────────────

/// Parse `bytes` as a WASM binary, call function at absolute index `func_idx`,
/// and return the top of the value stack (if any) after execution.
pub fn run(bytes: &[u8], func_idx: usize) -> Result<Option<i32>, RunError> {
    let module = load(bytes).map_err(RunError::Load)?;
    let import_count = count_func_imports(module.import_section);
    let mut interp = Interpreter::new(&module, import_count)
        .map_err(RunError::Interp)?;
    interp.call(func_idx).map_err(RunError::Interp)?;
    Ok(interp.top_i32())
}

// ── Import counting ───────────────────────────────────────────────────────────

/// Walk the import section and count function-typed imports.
/// Returns 0 if the section is absent or malformed.
fn count_func_imports(import_section: Option<&[u8]>) -> usize {
    let bytes = match import_section { Some(b) => b, None => return 0 };
    let mut cur = 0usize;

    let (count, n) = match read_u32_leb128(bytes) {
        Some(x) => x,
        None    => return 0,
    };
    cur += n;

    let mut func_count = 0usize;
    for _ in 0..count as usize {
        // module name (length-prefixed string)
        let (mod_len, n) = match read_u32_leb128(&bytes[cur..]) {
            Some(x) => x, None => return func_count,
        };
        cur += n + mod_len as usize;
        if cur > bytes.len() { return func_count; }

        // field name
        let (name_len, n) = match read_u32_leb128(&bytes[cur..]) {
            Some(x) => x, None => return func_count,
        };
        cur += n + name_len as usize;
        if cur > bytes.len() { return func_count; }

        // import kind byte
        if cur >= bytes.len() { return func_count; }
        let kind = bytes[cur]; cur += 1;

        match kind {
            0 => { // function — type index (uleb128)
                let (_, n) = match read_u32_leb128(&bytes[cur..]) {
                    Some(x) => x, None => return func_count,
                };
                cur += n;
                func_count += 1;
            }
            1 => { // table — reftype (1 byte) + limits
                cur += 1; // skip reftype
                if cur >= bytes.len() { return func_count; }
                let flag = bytes[cur]; cur += 1;
                let (_, n) = match read_u32_leb128(&bytes[cur..]) {
                    Some(x) => x, None => return func_count,
                };
                cur += n; // min
                if flag != 0 {
                    let (_, n) = match read_u32_leb128(&bytes[cur..]) {
                        Some(x) => x, None => return func_count,
                    };
                    cur += n; // max
                }
            }
            2 => { // memory — limits
                if cur >= bytes.len() { return func_count; }
                let flag = bytes[cur]; cur += 1;
                let (_, n) = match read_u32_leb128(&bytes[cur..]) {
                    Some(x) => x, None => return func_count,
                };
                cur += n; // min
                if flag != 0 {
                    let (_, n) = match read_u32_leb128(&bytes[cur..]) {
                        Some(x) => x, None => return func_count,
                    };
                    cur += n; // max
                }
            }
            3 => { cur += 2; } // global — valtype (1) + mutability (1)
            _  => return func_count,
        }
    }
    func_count
}
