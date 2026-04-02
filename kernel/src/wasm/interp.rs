//! WASM interpreter — Sprint 2.2 / Sprint 4.1
//!
//! Supported opcodes
//!   0x20  local.get  <uleb128>   push value of local variable
//!   0x21  local.set  <uleb128>   pop and store into local variable
//!   0x22  local.tee  <uleb128>   store into local but leave value on stack
//!   0x41  i32.const  <sleb128>   push an i32 literal
//!   0x10  call       <uleb128>   call a function by index
//!   0x0B  end                    return from function
//!
//! No heap allocation; everything lives in fixed-size arrays.

use super::loader::{Module, read_u32_leb128};

// ── Opcodes ──────────────────────────────────────────────────────────────────
const OP_END:       u8 = 0x0B;
const OP_CALL:      u8 = 0x10;
const OP_LOCAL_GET: u8 = 0x20;
const OP_LOCAL_SET: u8 = 0x21;
const OP_LOCAL_TEE: u8 = 0x22;
const OP_I32_CONST: u8 = 0x41;

// ── Capacity limits ───────────────────────────────────────────────────────────
pub const MAX_FUNCS:   usize = 32;  // max WASM-defined functions per module
pub const MAX_TYPES:   usize = 16;  // max type entries
pub const MAX_LOCALS:  usize = 16;  // max locals (params + declared) per frame
pub const STACK_DEPTH: usize = 256; // value stack slots
pub const CALL_DEPTH:  usize = 32;  // max call nesting depth
pub const MEM_SIZE:    usize = 4096;  // linear memory (enough for embedded modules)

/// Host dispatch function: called when WASM executes `call N` where N < import_count.
pub type HostFn = fn(func_idx: usize, vstack: &mut [i32], vsp: &mut usize, mem: &mut [u8])
    -> Result<(), InterpError>;

// ── Error type ───────────────────────────────────────────────────────────────
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum InterpError {
    NoCodeSection,
    MalformedCode,
    TooManyFuncs,
    TooManyTypes,
    TooManyLocals,
    FuncIndexOutOfRange,
    LocalIndexOutOfRange,
    IsImport,
    StackOverflow,
    StackUnderflow,
    CallStackOverflow,
    MemOutOfBounds,
    UnknownOpcode(u8),
}

impl InterpError {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NoCodeSection       => "no code section",
            Self::MalformedCode       => "malformed bytecode",
            Self::TooManyFuncs        => "too many functions (increase MAX_FUNCS)",
            Self::TooManyTypes        => "too many types (increase MAX_TYPES)",
            Self::TooManyLocals       => "too many locals (increase MAX_LOCALS)",
            Self::FuncIndexOutOfRange => "function index out of range",
            Self::LocalIndexOutOfRange => "local variable index out of range",
            Self::IsImport            => "call to import (not yet supported)",
            Self::StackOverflow       => "value stack overflow",
            Self::StackUnderflow      => "value stack underflow",
            Self::CallStackOverflow   => "call stack overflow",
            Self::MemOutOfBounds      => "memory access out of bounds",
            Self::UnknownOpcode(_)    => "unknown opcode",
        }
    }
}

// ── Call-stack frame ──────────────────────────────────────────────────────────
#[derive(Clone, Copy)]
struct Frame {
    body_idx:    usize,             // index into self.bodies[]
    pc:          usize,             // byte offset within that body slice
    locals:      [i32; MAX_LOCALS], // params + declared locals, all zeroed except params
    local_count: usize,             // total locals in this frame
}

// ── Default host (no-op, returns IsImport) ───────────────────────────────────
fn default_host(_: usize, _: &mut [i32], _: &mut usize, _: &mut [u8]) -> Result<(), InterpError> {
    Err(InterpError::IsImport)
}

// ── Interpreter ───────────────────────────────────────────────────────────────
pub struct Interpreter<'a> {
    /// Bytecode slices for each non-import function (locals already skipped).
    /// bodies[i] → absolute function index (import_count + i).
    bodies:     [&'a [u8]; MAX_FUNCS],
    body_count: usize,

    /// Number of declared (non-param) locals for each function body.
    local_counts: [usize; MAX_FUNCS],

    /// Param count for each type entry.
    type_param_counts: [usize; MAX_TYPES],
    type_count: usize,

    /// Type index for each defined function (parallel to bodies[]).
    func_type_indices: [usize; MAX_FUNCS],

    /// Number of imported functions (no body; dispatched through host layer).
    pub import_count: usize,

    // Value stack — i32 only for now.
    pub vstack: [i32; STACK_DEPTH],
    pub vsp:    usize,

    // Call stack.
    frames: [Frame; CALL_DEPTH],
    fdepth: usize,

    /// Linear memory — one flat byte array shared with host functions.
    pub mem: [u8; MEM_SIZE],

    /// Host dispatch: called when a `call N` targets an import (N < import_count).
    pub host_fn: HostFn,
}

impl<'a> Interpreter<'a> {
    /// Parse `module` and build an interpreter ready to call.
    pub fn new(module: &'a Module<'a>, import_count: usize) -> Result<Self, InterpError> {
        // ── type section → param counts ──────────────────────────────────────
        let mut type_param_counts = [0usize; MAX_TYPES];
        let type_count = if let Some(tb) = module.type_section {
            parse_type_section(tb, &mut type_param_counts)?
        } else {
            0
        };

        // ── function section → type indices ──────────────────────────────────
        let mut func_type_indices = [0usize; MAX_FUNCS];
        if let Some(fb) = module.function_section {
            parse_function_section(fb, &mut func_type_indices)?;
        }

        // ── code section → bodies + local counts ─────────────────────────────
        let code_bytes = module.code_section.ok_or(InterpError::NoCodeSection)?;
        let mut bodies       = [&[][..] as &[u8]; MAX_FUNCS];
        let mut local_counts = [0usize; MAX_FUNCS];
        let body_count = parse_code_section(code_bytes, &mut bodies, &mut local_counts)?;

        let blank_frame = Frame {
            body_idx: 0, pc: 0,
            locals: [0i32; MAX_LOCALS], local_count: 0,
        };

        Ok(Self {
            bodies,
            body_count,
            local_counts,
            type_param_counts,
            type_count,
            func_type_indices,
            import_count,
            vstack: [0i32; STACK_DEPTH],
            vsp:    0,
            frames: [blank_frame; CALL_DEPTH],
            fdepth: 0,
            mem:    [0u8; MEM_SIZE],
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

        // Determine param count from type index.
        let type_idx    = self.func_type_indices[body_idx];
        let param_count = if type_idx < self.type_count {
            self.type_param_counts[type_idx]
        } else {
            0
        };
        let decl_count  = self.local_counts[body_idx];
        let total       = param_count + decl_count;
        if total > MAX_LOCALS {
            return Err(InterpError::TooManyLocals);
        }

        let mut frame = Frame {
            body_idx,
            pc: 0,
            locals: [0i32; MAX_LOCALS],
            local_count: total,
        };

        // Pop params off the value stack (last param is on top).
        for i in (0..param_count).rev() {
            frame.locals[i] = self.v_pop()?;
        }

        self.frames[self.fdepth] = frame;
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
            let fi       = self.fdepth - 1;
            let body_idx = self.frames[fi].body_idx;
            let pc       = self.frames[fi].pc;
            let body     = self.bodies[body_idx];

            if pc >= body.len() {
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

                // ── local.get <idx> ───────────────────────────────────────────
                OP_LOCAL_GET => {
                    let fi   = self.fdepth - 1;
                    let pc   = self.frames[fi].pc;
                    let body = self.bodies[self.frames[fi].body_idx];
                    let (idx, consumed) = read_u32_leb128(&body[pc..])
                        .ok_or(InterpError::MalformedCode)?;
                    self.frames[fi].pc += consumed;
                    let idx = idx as usize;
                    if idx >= self.frames[fi].local_count {
                        return Err(InterpError::LocalIndexOutOfRange);
                    }
                    let val = self.frames[fi].locals[idx];
                    self.v_push(val)?;
                }

                // ── local.set <idx> ───────────────────────────────────────────
                OP_LOCAL_SET => {
                    let fi   = self.fdepth - 1;
                    let pc   = self.frames[fi].pc;
                    let body = self.bodies[self.frames[fi].body_idx];
                    let (idx, consumed) = read_u32_leb128(&body[pc..])
                        .ok_or(InterpError::MalformedCode)?;
                    self.frames[fi].pc += consumed;
                    let idx = idx as usize;
                    if idx >= self.frames[fi].local_count {
                        return Err(InterpError::LocalIndexOutOfRange);
                    }
                    let val = self.v_pop()?;
                    self.frames[fi].locals[idx] = val;
                }

                // ── local.tee <idx> ───────────────────────────────────────────
                OP_LOCAL_TEE => {
                    let fi   = self.fdepth - 1;
                    let pc   = self.frames[fi].pc;
                    let body = self.bodies[self.frames[fi].body_idx];
                    let (idx, consumed) = read_u32_leb128(&body[pc..])
                        .ok_or(InterpError::MalformedCode)?;
                    self.frames[fi].pc += consumed;
                    let idx = idx as usize;
                    if idx >= self.frames[fi].local_count {
                        return Err(InterpError::LocalIndexOutOfRange);
                    }
                    // Peek — don't pop.
                    if self.vsp == 0 { return Err(InterpError::StackUnderflow); }
                    let val = self.vstack[self.vsp - 1];
                    self.frames[fi].locals[idx] = val;
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
                        let host = self.host_fn;
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

/// Parse type section → fill `out[i]` with param count for type i.
/// Returns the number of types parsed.
fn parse_type_section(bytes: &[u8], out: &mut [usize; MAX_TYPES]) -> Result<usize, InterpError> {
    let mut cur = 0usize;
    let (count, n) = read_u32_leb128(bytes).ok_or(InterpError::MalformedCode)?;
    cur += n;
    let count = count as usize;
    if count > MAX_TYPES { return Err(InterpError::TooManyTypes); }

    for i in 0..count {
        // Each type entry starts with 0x60 (func type).
        if cur >= bytes.len() { return Err(InterpError::MalformedCode); }
        cur += 1; // skip 0x60

        // Param count + param types.
        let (param_count, n) = read_u32_leb128(&bytes[cur..]).ok_or(InterpError::MalformedCode)?;
        cur += n;
        out[i] = param_count as usize;
        cur += param_count as usize; // skip param valtypes

        // Result count + result types.
        let (result_count, n) = read_u32_leb128(&bytes[cur..]).ok_or(InterpError::MalformedCode)?;
        cur += n;
        cur += result_count as usize; // skip result valtypes
    }
    Ok(count)
}

/// Parse function section → fill `out[i]` with the type index for defined function i.
fn parse_function_section(bytes: &[u8], out: &mut [usize; MAX_FUNCS]) -> Result<(), InterpError> {
    let mut cur = 0usize;
    let (count, n) = read_u32_leb128(bytes).ok_or(InterpError::MalformedCode)?;
    cur += n;
    let count = count as usize;
    if count > MAX_FUNCS { return Err(InterpError::TooManyFuncs); }

    for i in 0..count {
        let (type_idx, n) = read_u32_leb128(&bytes[cur..]).ok_or(InterpError::MalformedCode)?;
        cur += n;
        out[i] = type_idx as usize;
    }
    Ok(())
}

/// Parse code section → fill `bodies[i]` with the bytecode slice (after locals)
/// and `local_counts[i]` with the number of declared locals (not params).
fn parse_code_section<'a>(
    bytes: &'a [u8],
    bodies: &mut [&'a [u8]; MAX_FUNCS],
    local_counts: &mut [usize; MAX_FUNCS],
) -> Result<usize, InterpError> {
    let mut cur = 0usize;

    let (count, n) = read_u32_leb128(&bytes[cur..]).ok_or(InterpError::MalformedCode)?;
    cur += n;
    let count = count as usize;
    if count > MAX_FUNCS { return Err(InterpError::TooManyFuncs); }

    for i in 0..count {
        let (body_size, n) = read_u32_leb128(&bytes[cur..]).ok_or(InterpError::MalformedCode)?;
        cur += n;

        let entry_start = cur;
        let entry_end   = cur + body_size as usize;
        if entry_end > bytes.len() { return Err(InterpError::MalformedCode); }
        let entry = &bytes[entry_start..entry_end];

        // Count declared locals and skip their declarations.
        let (local_groups, n) = read_u32_leb128(entry).ok_or(InterpError::MalformedCode)?;
        let mut lc = n;
        let mut total_declared = 0usize;
        for _ in 0..local_groups {
            let (group_count, n) = read_u32_leb128(&entry[lc..]).ok_or(InterpError::MalformedCode)?;
            lc += n + 1; // +1 for the valtype byte
            total_declared += group_count as usize;
        }
        if lc > entry.len() { return Err(InterpError::MalformedCode); }

        bodies[i]       = &entry[lc..]; // bytecode starts after locals
        local_counts[i] = total_declared;
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
        if shift >= 35 { return None; }
        result |= ((byte & 0x7F) as i32) << shift;
        shift += 7;
        if byte & 0x80 == 0 {
            if shift < 32 && (byte & 0x40) != 0 {
                result |= !0i32 << shift;
            }
            return Some((result, i + 1));
        }
    }
    None
}
