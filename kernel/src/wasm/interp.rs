//! Minimal WASM interpreter — Sprint 2.2
//!
//! Supported opcodes
//!   0x41  i32.const <sleb128>   push an i32 literal
//!   0x10  call      <uleb128>   call a function by index
//!   0x0B  end                   return from function
//!
//! No heap allocation; everything lives in fixed-size arrays.

use super::loader::{Module, read_u32_leb128};

// ── Opcodes ──────────────────────────────────────────────────────────────────
const OP_END:       u8 = 0x0B;
const OP_CALL:      u8 = 0x10;
const OP_I32_CONST: u8 = 0x41;

// ── Capacity limits ───────────────────────────────────────────────────────────
pub const MAX_FUNCS:   usize = 32;   // max WASM-defined functions per module
pub const STACK_DEPTH: usize = 256;  // value stack slots
pub const CALL_DEPTH:  usize = 32;   // max call nesting depth
pub const MEM_SIZE:    usize = 4096; // linear memory bytes

/// Host dispatch function: called when WASM executes `call N` where N < import_count.
/// The host is responsible for popping its arguments from vstack/vsp and
/// pushing any return values.
pub type HostFn = fn(func_idx: usize, vstack: &mut [i32], vsp: &mut usize, mem: &mut [u8])
    -> Result<(), InterpError>;

// ── Error type ───────────────────────────────────────────────────────────────
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum InterpError {
    NoCodeSection,
    MalformedCode,
    TooManyFuncs,
    FuncIndexOutOfRange,
    IsImport,
    StackOverflow,
    StackUnderflow,
    CallStackOverflow,
    UnknownOpcode(u8),
}

impl InterpError {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NoCodeSection       => "no code section",
            Self::MalformedCode       => "malformed bytecode",
            Self::TooManyFuncs        => "too many functions (increase MAX_FUNCS)",
            Self::FuncIndexOutOfRange => "function index out of range",
            Self::IsImport            => "call to import (not yet supported)",
            Self::StackOverflow       => "value stack overflow",
            Self::StackUnderflow      => "value stack underflow",
            Self::CallStackOverflow   => "call stack overflow",
            Self::UnknownOpcode(_)    => "unknown opcode",
        }
    }
}

// ── Call-stack frame ──────────────────────────────────────────────────────────
#[derive(Clone, Copy)]
struct Frame {
    body_idx: usize, // index into self.bodies[]
    pc: usize,       // byte offset within that body slice
}

// ── Default host (no-op, returns IsImport) ───────────────────────────────────
fn default_host(_: usize, _: &mut [i32], _: &mut usize, _: &mut [u8]) -> Result<(), InterpError> {
    Err(InterpError::IsImport)
}

// ── Interpreter ───────────────────────────────────────────────────────────────
pub struct Interpreter<'a> {
    /// Bytecode slices for each non-import function.
    /// bodies[i] → absolute function index (import_count + i).
    bodies: [&'a [u8]; MAX_FUNCS],
    body_count: usize,
    /// Number of imported functions (have no body; handled by host layer).
    pub import_count: usize,

    // Value stack — i32 only for now.
    pub vstack: [i32; STACK_DEPTH],
    pub vsp: usize,

    // Call stack.
    frames: [Frame; CALL_DEPTH],
    fdepth: usize,

    /// Linear memory — one flat byte array shared with host functions.
    pub mem: [u8; MEM_SIZE],

    /// Host dispatch: called when a `call N` targets an import (N < import_count).
    pub host_fn: HostFn,
}

impl<'a> Interpreter<'a> {
    /// Parse the code section of `module` and build an interpreter.
    ///
    /// `import_count`: number of imported functions declared in the import
    /// section (they occupy the first N function indices but have no body).
    pub fn new(module: &'a Module<'a>, import_count: usize) -> Result<Self, InterpError> {
        let code_bytes = module.code_section.ok_or(InterpError::NoCodeSection)?;
        let mut bodies = [&[][..] as &[u8]; MAX_FUNCS];
        let body_count = parse_code_section(code_bytes, &mut bodies)?;

        Ok(Self {
            bodies,
            body_count,
            import_count,
            vstack: [0i32; STACK_DEPTH],
            vsp: 0,
            frames: [Frame { body_idx: 0, pc: 0 }; CALL_DEPTH],
            fdepth: 0,
            mem: [0u8; MEM_SIZE],
            host_fn: default_host,
        })
    }

    /// Call function at absolute index `func_idx` and run until it returns.
    pub fn call(&mut self, func_idx: usize) -> Result<(), InterpError> {
        if func_idx < self.import_count {
            return Err(InterpError::IsImport);
        }
        let body_idx = func_idx - self.import_count;
        if body_idx >= self.body_count {
            return Err(InterpError::FuncIndexOutOfRange);
        }
        self.push_frame(body_idx)?;
        self.run()
    }

    /// Peek at the top of the value stack after execution.
    pub fn top_i32(&self) -> Option<i32> {
        if self.vsp > 0 { Some(self.vstack[self.vsp - 1]) } else { None }
    }

    // ── Internal helpers ─────────────────────────────────────────────────────

    fn push_frame(&mut self, body_idx: usize) -> Result<(), InterpError> {
        if self.fdepth >= CALL_DEPTH {
            return Err(InterpError::CallStackOverflow);
        }
        self.frames[self.fdepth] = Frame { body_idx, pc: 0 };
        self.fdepth += 1;
        Ok(())
    }

    fn v_push(&mut self, v: i32) -> Result<(), InterpError> {
        if self.vsp >= STACK_DEPTH {
            return Err(InterpError::StackOverflow);
        }
        self.vstack[self.vsp] = v;
        self.vsp += 1;
        Ok(())
    }

    pub fn v_pop(&mut self) -> Result<i32, InterpError> {
        if self.vsp == 0 {
            return Err(InterpError::StackUnderflow);
        }
        self.vsp -= 1;
        Ok(self.vstack[self.vsp])
    }

    // ── Main dispatch loop ────────────────────────────────────────────────────
    fn run(&mut self) -> Result<(), InterpError> {
        while self.fdepth > 0 {
            // Copy out indices so we don't hold a borrow on self while
            // mutating vstack / frames.
            let fi       = self.fdepth - 1;
            let body_idx = self.frames[fi].body_idx;
            let pc       = self.frames[fi].pc;
            let body     = self.bodies[body_idx];

            if pc >= body.len() {
                // Implicit return at end of body.
                self.fdepth -= 1;
                continue;
            }

            let opcode = body[pc];
            self.frames[fi].pc += 1;

            match opcode {
                // ── end ──────────────────────────────────────────────────────
                OP_END => {
                    self.fdepth -= 1;
                }

                // ── i32.const <sleb128> ───────────────────────────────────────
                OP_I32_CONST => {
                    let fi   = self.fdepth - 1;
                    let pc   = self.frames[fi].pc;
                    let body = self.bodies[self.frames[fi].body_idx];
                    let (val, consumed) = read_i32_leb128(&body[pc..])
                        .ok_or(InterpError::MalformedCode)?;
                    self.frames[fi].pc += consumed;
                    self.v_push(val)?;
                }

                // ── call <uleb128> ────────────────────────────────────────────
                OP_CALL => {
                    let fi   = self.fdepth - 1;
                    let pc   = self.frames[fi].pc;
                    let body = self.bodies[self.frames[fi].body_idx];
                    let (callee, consumed) = read_u32_leb128(&body[pc..])
                        .ok_or(InterpError::MalformedCode)?;
                    self.frames[fi].pc += consumed;

                    let callee = callee as usize;
                    if callee < self.import_count {
                        // Host function dispatch.
                        let host = self.host_fn; // copy fn ptr (Copy) before mut borrow
                        host(callee, &mut self.vstack, &mut self.vsp, &mut self.mem)?;
                    } else {
                        let body_idx = callee - self.import_count;
                        if body_idx >= self.body_count {
                            return Err(InterpError::FuncIndexOutOfRange);
                        }
                        self.push_frame(body_idx)?;
                    }
                }

                other => return Err(InterpError::UnknownOpcode(other)),
            }
        }
        Ok(())
    }
}

// ── Section parsers ───────────────────────────────────────────────────────────

/// Extract the bytecode slice for each function body in the code section.
/// Skips over the local-variable declarations at the front of each body.
fn parse_code_section<'a>(
    bytes: &'a [u8],
    bodies: &mut [&'a [u8]; MAX_FUNCS],
) -> Result<usize, InterpError> {
    let mut cur = 0usize;

    let (count, n) = read_u32_leb128(&bytes[cur..]).ok_or(InterpError::MalformedCode)?;
    cur += n;
    let count = count as usize;
    if count > MAX_FUNCS {
        return Err(InterpError::TooManyFuncs);
    }

    for i in 0..count {
        // Total byte size of this function entry (locals + bytecode).
        let (body_size, n) = read_u32_leb128(&bytes[cur..]).ok_or(InterpError::MalformedCode)?;
        cur += n;

        let entry_start = cur;
        let entry_end   = cur + body_size as usize;
        if entry_end > bytes.len() {
            return Err(InterpError::MalformedCode);
        }
        let entry = &bytes[entry_start..entry_end];

        // Skip local declarations: count groups, each group is (n: uleb128, type: u8).
        let (local_groups, n) = read_u32_leb128(entry).ok_or(InterpError::MalformedCode)?;
        let mut lc = n;
        for _ in 0..local_groups {
            let (_, n) = read_u32_leb128(&entry[lc..]).ok_or(InterpError::MalformedCode)?;
            lc += n + 1; // +1 for the valtype byte
        }
        if lc > entry.len() {
            return Err(InterpError::MalformedCode);
        }

        // Everything after the locals is the actual bytecode.
        bodies[i] = &entry[lc..];
        cur = entry_end;
    }

    Ok(count)
}

// ── Signed LEB-128 for i32.const ─────────────────────────────────────────────

/// Decode a signed 32-bit LEB-128 integer.
/// Returns `(value, bytes_consumed)` or `None` on malformed input.
pub fn read_i32_leb128(bytes: &[u8]) -> Option<(i32, usize)> {
    let mut result: i32 = 0;
    let mut shift: u32  = 0;
    for (i, &byte) in bytes.iter().enumerate() {
        if shift >= 35 {
            return None;
        }
        result |= ((byte & 0x7F) as i32) << shift;
        shift += 7;
        if byte & 0x80 == 0 {
            // Sign-extend if the value is negative.
            if shift < 32 && (byte & 0x40) != 0 {
                result |= !0i32 << shift;
            }
            return Some((result, i + 1));
        }
    }
    None
}
