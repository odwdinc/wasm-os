//! WASM interpreter — Sprint 2.2 / 4.1 / 4.2 / 4.3 / 4.4
//!
//! No heap allocation; everything lives in fixed-size arrays.

use super::loader::{Module, read_u32_leb128};

// ── Opcodes ───────────────────────────────────────────────────────────────────
const OP_UNREACHABLE: u8 = 0x00;
const OP_NOP:         u8 = 0x01;
const OP_BLOCK:       u8 = 0x02;
const OP_LOOP:        u8 = 0x03;
const OP_IF:          u8 = 0x04;
const OP_ELSE:        u8 = 0x05;
const OP_END:         u8 = 0x0B;
const OP_BR:          u8 = 0x0C;
const OP_BR_IF:       u8 = 0x0D;
const OP_RETURN:      u8 = 0x0F;
const OP_CALL:        u8 = 0x10;
const OP_DROP:        u8 = 0x1A;
const OP_SELECT:      u8 = 0x1B;
const OP_LOCAL_GET:   u8 = 0x20;
const OP_LOCAL_SET:   u8 = 0x21;
const OP_LOCAL_TEE:   u8 = 0x22;
const OP_I32_LOAD:    u8 = 0x28;
const OP_I32_LOAD8_U: u8 = 0x2D;
const OP_I32_STORE:   u8 = 0x36;
const OP_I32_STORE8:  u8 = 0x3A;
const OP_I32_CONST:   u8 = 0x41;
const OP_I32_EQZ:     u8 = 0x45;
const OP_I32_EQ:      u8 = 0x46;
const OP_I32_NE:      u8 = 0x47;
const OP_I32_LT_S:    u8 = 0x48;
const OP_I32_GT_S:    u8 = 0x4A;
const OP_I32_LE_S:    u8 = 0x4C;
const OP_I32_GE_S:    u8 = 0x4E;
const OP_I32_ADD:     u8 = 0x6A;
const OP_I32_SUB:     u8 = 0x6B;
const OP_I32_MUL:     u8 = 0x6C;
const OP_I32_AND:     u8 = 0x71;
const OP_I32_OR:      u8 = 0x72;
const OP_I32_XOR:     u8 = 0x73;
const OP_I32_SHL:     u8 = 0x74;
const OP_I32_SHR_S:   u8 = 0x75;

// ── Capacity limits ───────────────────────────────────────────────────────────
pub const MAX_FUNCS:      usize = 32;
pub const MAX_TYPES:      usize = 16;
pub const MAX_LOCALS:     usize = 16;
pub const MAX_CTRL_DEPTH: usize = 64; // total across all live call frames
pub const STACK_DEPTH:    usize = 256;
pub const CALL_DEPTH:     usize = 128;
pub const MEM_SIZE:       usize = 4096;

const NO_ELSE: usize = usize::MAX;

/// Host dispatch: called for `call N` where N < import_count.
pub type HostFn = fn(func_idx: usize, vstack: &mut [i32], vsp: &mut usize, mem: &mut [u8])
    -> Result<(), InterpError>;

// ── Error type ────────────────────────────────────────────────────────────────
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum InterpError {
    NoCodeSection,
    MalformedCode,
    TooManyFuncs,
    TooManyTypes,
    TooManyLocals,
    CtrlStackOverflow,
    FuncIndexOutOfRange,
    LocalIndexOutOfRange,
    IsImport,
    Unreachable,
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
            Self::CtrlStackOverflow   => "control stack overflow (increase MAX_CTRL_DEPTH)",
            Self::FuncIndexOutOfRange => "function index out of range",
            Self::LocalIndexOutOfRange => "local variable index out of range",
            Self::IsImport            => "call to import (not yet supported)",
            Self::Unreachable         => "unreachable executed",
            Self::StackOverflow       => "value stack overflow",
            Self::StackUnderflow      => "value stack underflow",
            Self::CallStackOverflow   => "call stack overflow",
            Self::MemOutOfBounds      => "memory access out of bounds",
            Self::UnknownOpcode(_)    => "unknown opcode",
        }
    }
}

// ── Control frame (block / loop / if) ────────────────────────────────────────
#[derive(Clone, Copy, PartialEq)]
enum BlockKind { Block, Loop, If }

#[derive(Clone, Copy)]
struct CtrlFrame {
    kind:     BlockKind,
    /// For Loop: PC to jump back to (top of loop body).
    /// For Block/If: unused.
    pc_start: usize,
    /// Position OF the matching `end` opcode in the body slice.
    end_pc:   usize,
}

const BLANK_CTRL: CtrlFrame = CtrlFrame {
    kind: BlockKind::Block, pc_start: 0, end_pc: 0,
};

// ── Call-stack frame ──────────────────────────────────────────────────────────
#[derive(Clone, Copy)]
struct Frame {
    body_idx:    usize,
    pc:          usize,
    locals:      [i32; MAX_LOCALS],
    local_count: usize,
    /// ctrl_depth value at the moment this function was entered.
    ctrl_base:   usize,
}

const BLANK_FRAME: Frame = Frame {
    body_idx: 0, pc: 0,
    locals: [0i32; MAX_LOCALS], local_count: 0,
    ctrl_base: 0,
};

// ── Default host ──────────────────────────────────────────────────────────────
fn default_host(_: usize, _: &mut [i32], _: &mut usize, _: &mut [u8]) -> Result<(), InterpError> {
    Err(InterpError::IsImport)
}

// ── Interpreter ───────────────────────────────────────────────────────────────
pub struct Interpreter<'a> {
    bodies:            [&'a [u8]; MAX_FUNCS],
    body_count:        usize,
    local_counts:      [usize; MAX_FUNCS],
    type_param_counts: [usize; MAX_TYPES],
    type_count:        usize,
    func_type_indices: [usize; MAX_FUNCS],
    pub import_count:  usize,

    pub vstack: [i32; STACK_DEPTH],
    pub vsp:    usize,

    frames: [Frame; CALL_DEPTH],
    fdepth: usize,

    ctrl:       [CtrlFrame; MAX_CTRL_DEPTH],
    ctrl_depth: usize,

    pub mem:     [u8; MEM_SIZE],
    pub host_fn: HostFn,
}

impl<'a> Interpreter<'a> {
    pub fn new(module: &'a Module<'a>, import_count: usize) -> Result<Self, InterpError> {
        let mut type_param_counts = [0usize; MAX_TYPES];
        let type_count = if let Some(tb) = module.type_section {
            parse_type_section(tb, &mut type_param_counts)?
        } else { 0 };

        let mut func_type_indices = [0usize; MAX_FUNCS];
        if let Some(fb) = module.function_section {
            parse_function_section(fb, &mut func_type_indices)?;
        }

        let code_bytes = module.code_section.ok_or(InterpError::NoCodeSection)?;
        let mut bodies       = [&[][..] as &[u8]; MAX_FUNCS];
        let mut local_counts = [0usize; MAX_FUNCS];
        let body_count = parse_code_section(code_bytes, &mut bodies, &mut local_counts)?;

        Ok(Self {
            bodies, body_count, local_counts,
            type_param_counts, type_count, func_type_indices,
            import_count,
            vstack: [0i32; STACK_DEPTH], vsp: 0,
            frames: [BLANK_FRAME; CALL_DEPTH], fdepth: 0,
            ctrl: [BLANK_CTRL; MAX_CTRL_DEPTH], ctrl_depth: 0,
            mem: [0u8; MEM_SIZE],
            host_fn: default_host,
        })
    }

    pub fn call(&mut self, func_idx: usize) -> Result<(), InterpError> {
        if func_idx < self.import_count { return Err(InterpError::IsImport); }
        let body_idx = func_idx - self.import_count;
        if body_idx >= self.body_count { return Err(InterpError::FuncIndexOutOfRange); }
        self.push_frame(body_idx)?;
        self.run()
    }

    pub fn top_i32(&self) -> Option<i32> {
        if self.vsp > 0 { Some(self.vstack[self.vsp - 1]) } else { None }
    }

    // ── Internal helpers ──────────────────────────────────────────────────────

    fn push_frame(&mut self, body_idx: usize) -> Result<(), InterpError> {
        if self.fdepth >= CALL_DEPTH { return Err(InterpError::CallStackOverflow); }

        let type_idx    = self.func_type_indices[body_idx];
        let param_count = if type_idx < self.type_count { self.type_param_counts[type_idx] } else { 0 };
        let decl_count  = self.local_counts[body_idx];
        let total       = param_count + decl_count;
        if total > MAX_LOCALS { return Err(InterpError::TooManyLocals); }

        let mut frame = Frame {
            body_idx, pc: 0,
            locals: [0i32; MAX_LOCALS], local_count: total,
            ctrl_base: self.ctrl_depth,
        };
        for i in (0..param_count).rev() {
            frame.locals[i] = self.v_pop()?;
        }
        self.frames[self.fdepth] = frame;
        self.fdepth += 1;
        Ok(())
    }

    fn v_push(&mut self, v: i32) -> Result<(), InterpError> {
        if self.vsp >= STACK_DEPTH { return Err(InterpError::StackOverflow); }
        self.vstack[self.vsp] = v;
        self.vsp += 1;
        Ok(())
    }

    pub fn v_pop(&mut self) -> Result<i32, InterpError> {
        if self.vsp == 0 { return Err(InterpError::StackUnderflow); }
        self.vsp -= 1;
        Ok(self.vstack[self.vsp])
    }

    fn ctrl_push(&mut self, cf: CtrlFrame) -> Result<(), InterpError> {
        if self.ctrl_depth >= MAX_CTRL_DEPTH { return Err(InterpError::CtrlStackOverflow); }
        self.ctrl[self.ctrl_depth] = cf;
        self.ctrl_depth += 1;
        Ok(())
    }

    // ── Main dispatch loop ─────────────────────────────────────────────────────
    fn run(&mut self) -> Result<(), InterpError> {
        while self.fdepth > 0 {
            let fi       = self.fdepth - 1;
            let body_idx = self.frames[fi].body_idx;
            let pc       = self.frames[fi].pc;
            let body     = self.bodies[body_idx];

            if pc >= body.len() {
                // Implicit return at end of body.
                self.ctrl_depth = self.frames[fi].ctrl_base;
                self.fdepth -= 1;
                continue;
            }

            let opcode = body[pc];
            self.frames[fi].pc += 1;

            match opcode {

                // ── nop / unreachable ─────────────────────────────────────────
                OP_NOP => {}
                OP_UNREACHABLE => return Err(InterpError::Unreachable),

                // ── block <blocktype> ─────────────────────────────────────────
                OP_BLOCK => {
                    let fi   = self.fdepth - 1;
                    let pc   = self.frames[fi].pc;
                    let body = self.bodies[self.frames[fi].body_idx];
                    // Skip blocktype (single byte in MVP).
                    let pc_body = pc + 1;
                    self.frames[fi].pc = pc_body;
                    let (end_pc, _) = scan_block_end(body, pc_body)
                        .ok_or(InterpError::MalformedCode)?;
                    self.ctrl_push(CtrlFrame {
                        kind: BlockKind::Block,
                        pc_start: pc_body, end_pc,
                    })?;
                }

                // ── loop <blocktype> ─────────────────────────────────────────
                OP_LOOP => {
                    let fi   = self.fdepth - 1;
                    let pc   = self.frames[fi].pc;
                    let body = self.bodies[self.frames[fi].body_idx];
                    let pc_body = pc + 1;
                    self.frames[fi].pc = pc_body;
                    let (end_pc, _) = scan_block_end(body, pc_body)
                        .ok_or(InterpError::MalformedCode)?;
                    self.ctrl_push(CtrlFrame {
                        kind: BlockKind::Loop,
                        pc_start: pc_body, end_pc,
                    })?;
                }

                // ── if <blocktype> ────────────────────────────────────────────
                OP_IF => {
                    let fi        = self.fdepth - 1;
                    let pc        = self.frames[fi].pc;
                    let body      = self.bodies[self.frames[fi].body_idx];
                    let pc_body   = pc + 1; // after blocktype
                    self.frames[fi].pc = pc_body;
                    let (end_pc, else_pc) = scan_block_end(body, pc_body)
                        .ok_or(InterpError::MalformedCode)?;
                    let cond = self.v_pop()?;
                    if cond != 0 {
                        // Enter then-branch.
                        self.ctrl_push(CtrlFrame {
                            kind: BlockKind::If,
                            pc_start: pc_body, end_pc,
                        })?;
                    } else if else_pc != NO_ELSE {
                        // Condition false, has else: jump to else body.
                        self.ctrl_push(CtrlFrame {
                            kind: BlockKind::If,
                            pc_start: pc_body, end_pc,
                        })?;
                        self.frames[fi].pc = else_pc + 1;
                    } else {
                        // Condition false, no else: skip to after end.
                        self.frames[fi].pc = end_pc + 1;
                    }
                }

                // ── else ──────────────────────────────────────────────────────
                OP_ELSE => {
                    // Reached end of then-branch; jump past the else block.
                    let fi   = self.fdepth - 1;
                    let end_pc = self.ctrl[self.ctrl_depth - 1].end_pc;
                    self.ctrl_depth -= 1; // pop the if ctrl frame
                    self.frames[fi].pc = end_pc + 1;
                }

                // ── end ───────────────────────────────────────────────────────
                OP_END => {
                    let fi = self.fdepth - 1;
                    if self.ctrl_depth > self.frames[fi].ctrl_base {
                        // End of a block/loop/if.
                        self.ctrl_depth -= 1;
                    } else {
                        // End of function body.
                        self.fdepth -= 1;
                    }
                }

                // ── br <labelidx> ─────────────────────────────────────────────
                OP_BR => {
                    let fi   = self.fdepth - 1;
                    let pc   = self.frames[fi].pc;
                    let body = self.bodies[self.frames[fi].body_idx];
                    let (n, consumed) = read_u32_leb128(&body[pc..])
                        .ok_or(InterpError::MalformedCode)?;
                    self.frames[fi].pc += consumed;
                    do_br(self, n as usize)?;
                }

                // ── br_if <labelidx> ──────────────────────────────────────────
                OP_BR_IF => {
                    let fi   = self.fdepth - 1;
                    let pc   = self.frames[fi].pc;
                    let body = self.bodies[self.frames[fi].body_idx];
                    let (n, consumed) = read_u32_leb128(&body[pc..])
                        .ok_or(InterpError::MalformedCode)?;
                    self.frames[fi].pc += consumed;
                    let cond = self.v_pop()?;
                    if cond != 0 {
                        do_br(self, n as usize)?;
                    }
                }

                // ── return ────────────────────────────────────────────────────
                OP_RETURN => {
                    let fi = self.fdepth - 1;
                    self.ctrl_depth = self.frames[fi].ctrl_base;
                    self.fdepth -= 1;
                }

                // ── drop / select ─────────────────────────────────────────────
                OP_DROP => { self.v_pop()?; }
                OP_SELECT => {
                    let cond = self.v_pop()?;
                    let b    = self.v_pop()?;
                    let a    = self.v_pop()?;
                    self.v_push(if cond != 0 { a } else { b })?;
                }

                // ── local.get/set/tee ─────────────────────────────────────────
                OP_LOCAL_GET => {
                    let fi   = self.fdepth - 1;
                    let pc   = self.frames[fi].pc;
                    let body = self.bodies[self.frames[fi].body_idx];
                    let (idx, consumed) = read_u32_leb128(&body[pc..])
                        .ok_or(InterpError::MalformedCode)?;
                    self.frames[fi].pc += consumed;
                    let idx = idx as usize;
                    if idx >= self.frames[fi].local_count { return Err(InterpError::LocalIndexOutOfRange); }
                    let val = self.frames[fi].locals[idx];
                    self.v_push(val)?;
                }
                OP_LOCAL_SET => {
                    let fi   = self.fdepth - 1;
                    let pc   = self.frames[fi].pc;
                    let body = self.bodies[self.frames[fi].body_idx];
                    let (idx, consumed) = read_u32_leb128(&body[pc..])
                        .ok_or(InterpError::MalformedCode)?;
                    self.frames[fi].pc += consumed;
                    let idx = idx as usize;
                    if idx >= self.frames[fi].local_count { return Err(InterpError::LocalIndexOutOfRange); }
                    let val = self.v_pop()?;
                    self.frames[fi].locals[idx] = val;
                }
                OP_LOCAL_TEE => {
                    let fi   = self.fdepth - 1;
                    let pc   = self.frames[fi].pc;
                    let body = self.bodies[self.frames[fi].body_idx];
                    let (idx, consumed) = read_u32_leb128(&body[pc..])
                        .ok_or(InterpError::MalformedCode)?;
                    self.frames[fi].pc += consumed;
                    let idx = idx as usize;
                    if idx >= self.frames[fi].local_count { return Err(InterpError::LocalIndexOutOfRange); }
                    if self.vsp == 0 { return Err(InterpError::StackUnderflow); }
                    let val = self.vstack[self.vsp - 1];
                    self.frames[fi].locals[idx] = val;
                }

                // ── memory loads ──────────────────────────────────────────────
                OP_I32_LOAD => {
                    let fi   = self.fdepth - 1;
                    let pc   = self.frames[fi].pc;
                    let body = self.bodies[self.frames[fi].body_idx];
                    let (offset, consumed) = read_memarg(&body[pc..])
                        .ok_or(InterpError::MalformedCode)?;
                    self.frames[fi].pc += consumed;
                    let addr = self.v_pop()? as u32 as usize;
                    let ea   = addr.wrapping_add(offset as usize);
                    if ea + 4 > self.mem.len() { return Err(InterpError::MemOutOfBounds); }
                    let val = i32::from_le_bytes([
                        self.mem[ea], self.mem[ea+1], self.mem[ea+2], self.mem[ea+3],
                    ]);
                    self.v_push(val)?;
                }
                OP_I32_LOAD8_U => {
                    let fi   = self.fdepth - 1;
                    let pc   = self.frames[fi].pc;
                    let body = self.bodies[self.frames[fi].body_idx];
                    let (offset, consumed) = read_memarg(&body[pc..])
                        .ok_or(InterpError::MalformedCode)?;
                    self.frames[fi].pc += consumed;
                    let addr = self.v_pop()? as u32 as usize;
                    let ea   = addr.wrapping_add(offset as usize);
                    if ea >= self.mem.len() { return Err(InterpError::MemOutOfBounds); }
                    self.v_push(self.mem[ea] as i32)?;
                }

                // ── memory stores ─────────────────────────────────────────────
                OP_I32_STORE => {
                    let fi   = self.fdepth - 1;
                    let pc   = self.frames[fi].pc;
                    let body = self.bodies[self.frames[fi].body_idx];
                    let (offset, consumed) = read_memarg(&body[pc..])
                        .ok_or(InterpError::MalformedCode)?;
                    self.frames[fi].pc += consumed;
                    let val  = self.v_pop()?;
                    let addr = self.v_pop()? as u32 as usize;
                    let ea   = addr.wrapping_add(offset as usize);
                    if ea + 4 > self.mem.len() { return Err(InterpError::MemOutOfBounds); }
                    self.mem[ea..ea+4].copy_from_slice(&val.to_le_bytes());
                }
                OP_I32_STORE8 => {
                    let fi   = self.fdepth - 1;
                    let pc   = self.frames[fi].pc;
                    let body = self.bodies[self.frames[fi].body_idx];
                    let (offset, consumed) = read_memarg(&body[pc..])
                        .ok_or(InterpError::MalformedCode)?;
                    self.frames[fi].pc += consumed;
                    let val  = self.v_pop()?;
                    let addr = self.v_pop()? as u32 as usize;
                    let ea   = addr.wrapping_add(offset as usize);
                    if ea >= self.mem.len() { return Err(InterpError::MemOutOfBounds); }
                    self.mem[ea] = val as u8;
                }

                // ── i32.const ────────────────────────────────────────────────
                OP_I32_CONST => {
                    let fi   = self.fdepth - 1;
                    let pc   = self.frames[fi].pc;
                    let body = self.bodies[self.frames[fi].body_idx];
                    let (val, consumed) = read_i32_leb128(&body[pc..])
                        .ok_or(InterpError::MalformedCode)?;
                    self.frames[fi].pc += consumed;
                    self.v_push(val)?;
                }

                // ── i32 comparisons ───────────────────────────────────────────
                OP_I32_EQZ  => { let a = self.v_pop()?; self.v_push(if a == 0 { 1 } else { 0 })?; }
                OP_I32_EQ   => { let b = self.v_pop()?; let a = self.v_pop()?; self.v_push(if a == b { 1 } else { 0 })?; }
                OP_I32_NE   => { let b = self.v_pop()?; let a = self.v_pop()?; self.v_push(if a != b { 1 } else { 0 })?; }
                OP_I32_LT_S => { let b = self.v_pop()?; let a = self.v_pop()?; self.v_push(if a <  b { 1 } else { 0 })?; }
                OP_I32_GT_S => { let b = self.v_pop()?; let a = self.v_pop()?; self.v_push(if a >  b { 1 } else { 0 })?; }
                OP_I32_LE_S => { let b = self.v_pop()?; let a = self.v_pop()?; self.v_push(if a <= b { 1 } else { 0 })?; }
                OP_I32_GE_S => { let b = self.v_pop()?; let a = self.v_pop()?; self.v_push(if a >= b { 1 } else { 0 })?; }

                // ── i32 arithmetic ────────────────────────────────────────────
                OP_I32_ADD   => { let b = self.v_pop()?; let a = self.v_pop()?; self.v_push(a.wrapping_add(b))?; }
                OP_I32_SUB   => { let b = self.v_pop()?; let a = self.v_pop()?; self.v_push(a.wrapping_sub(b))?; }
                OP_I32_MUL   => { let b = self.v_pop()?; let a = self.v_pop()?; self.v_push(a.wrapping_mul(b))?; }
                OP_I32_AND   => { let b = self.v_pop()?; let a = self.v_pop()?; self.v_push(a & b)?; }
                OP_I32_OR    => { let b = self.v_pop()?; let a = self.v_pop()?; self.v_push(a | b)?; }
                OP_I32_XOR   => { let b = self.v_pop()?; let a = self.v_pop()?; self.v_push(a ^ b)?; }
                OP_I32_SHL   => { let b = self.v_pop()?; let a = self.v_pop()?; self.v_push(a.wrapping_shl(b as u32 & 31))?; }
                OP_I32_SHR_S => { let b = self.v_pop()?; let a = self.v_pop()?; self.v_push(a.wrapping_shr(b as u32 & 31))?; }

                // ── call ──────────────────────────────────────────────────────
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
                        if body_idx >= self.body_count { return Err(InterpError::FuncIndexOutOfRange); }
                        self.push_frame(body_idx)?;
                    }
                }

                other => return Err(InterpError::UnknownOpcode(other)),
            }
        }
        Ok(())
    }
}

// ── br helper (shared by OP_BR and OP_BR_IF) ─────────────────────────────────

fn do_br(interp: &mut Interpreter, n: usize) -> Result<(), InterpError> {
    let fi        = interp.fdepth - 1;
    let ctrl_base = interp.frames[fi].ctrl_base;
    let available = interp.ctrl_depth - ctrl_base;

    if n >= available {
        // Branch past function boundary → return.
        interp.ctrl_depth = ctrl_base;
        interp.fdepth    -= 1;
        return Ok(());
    }

    let target_abs = interp.ctrl_depth - 1 - n;
    let cf = interp.ctrl[target_abs];

    match cf.kind {
        BlockKind::Loop => {
            // Keep the loop frame; jump to loop start.
            interp.ctrl_depth = target_abs + 1;
            interp.frames[fi].pc = cf.pc_start;
        }
        BlockKind::Block | BlockKind::If => {
            // Pop the block/if frame; jump past its end.
            interp.ctrl_depth = target_abs;
            interp.frames[fi].pc = cf.end_pc + 1;
        }
    }
    Ok(())
}

// ── Scan ahead for matching end/else ─────────────────────────────────────────

/// Starting at `start` (right after opcode + blocktype), scan forward to find
/// the matching `end` and optional `else`.  Returns `(end_pc, else_pc)` where
/// both are positions OF the opcode byte.  `else_pc` is `NO_ELSE` if absent.
///
/// Correctly skips all LEB-128 and fixed-width immediates so their bytes are
/// never mistaken for structural opcodes.  Safe because 0x02/0x03/0x04/0x0B
/// all have the high bit clear and therefore can only appear as the *final*
/// byte of a LEB-128 (never as a continuation byte).
fn scan_block_end(body: &[u8], start: usize) -> Option<(usize, usize)> {
    let mut depth    = 1usize;
    let mut i        = start;
    let mut else_pc  = NO_ELSE;

    while i < body.len() {
        let op = body[i];
        i += 1;
        match op {
            // Nested structured blocks — consume blocktype (1 byte MVP).
            0x02 | 0x03 | 0x04 => { i += 1; depth += 1; }
            0x05 => { if depth == 1 { else_pc = i - 1; } }
            0x0B => {
                depth -= 1;
                if depth == 0 { return Some((i - 1, else_pc)); }
            }
            // Single LEB-128 immediate.
            0x0C | 0x0D |            // br, br_if
            0x10 |                   // call
            0x20 | 0x21 | 0x22 |    // local.get/set/tee
            0x23 | 0x24 |            // global.get/set
            0x41 | 0x42 => {         // i32.const, i64.const
                i += skip_leb128(body, i)?;
            }
            // call_indirect: two LEB-128.
            0x11 => {
                i += skip_leb128(body, i)?;
                i += skip_leb128(body, i)?;
            }
            // br_table: count + (count+1) labels.
            0x0E => {
                let (count, n) = read_u32_leb128(&body[i..])?;
                i += n;
                for _ in 0..=count { i += skip_leb128(body, i)?; }
            }
            // Memory instructions: align + offset (two LEB-128).
            0x28..=0x3E => {
                i += skip_leb128(body, i)?;
                i += skip_leb128(body, i)?;
            }
            // memory.size / memory.grow: one reserved byte.
            0x3F | 0x40 => { i += 1; }
            // f32.const: 4 bytes literal.
            0x43 => { i += 4; }
            // f64.const: 8 bytes literal.
            0x44 => { i += 8; }
            // All other opcodes have no immediates.
            _ => {}
        }
    }
    None
}

/// Advance past one LEB-128 value starting at `bytes[start]`.
/// Returns the number of bytes consumed, or None on truncation.
fn skip_leb128(bytes: &[u8], start: usize) -> Option<usize> {
    let mut i = start;
    while i < bytes.len() {
        let b = bytes[i]; i += 1;
        if b & 0x80 == 0 { return Some(i - start); }
    }
    None
}

// ── Memory immediate helper ───────────────────────────────────────────────────

fn read_memarg(bytes: &[u8]) -> Option<(u32, usize)> {
    let (_align, n1) = read_u32_leb128(bytes)?;
    let (offset, n2) = read_u32_leb128(&bytes[n1..])?;
    Some((offset, n1 + n2))
}

// ── Section parsers ───────────────────────────────────────────────────────────

fn parse_type_section(bytes: &[u8], out: &mut [usize; MAX_TYPES]) -> Result<usize, InterpError> {
    let mut cur = 0usize;
    let (count, n) = read_u32_leb128(bytes).ok_or(InterpError::MalformedCode)?;
    cur += n;
    let count = count as usize;
    if count > MAX_TYPES { return Err(InterpError::TooManyTypes); }

    for i in 0..count {
        if cur >= bytes.len() { return Err(InterpError::MalformedCode); }
        cur += 1; // skip 0x60
        let (param_count, n) = read_u32_leb128(&bytes[cur..]).ok_or(InterpError::MalformedCode)?;
        cur += n;
        out[i] = param_count as usize;
        cur += param_count as usize;
        let (result_count, n) = read_u32_leb128(&bytes[cur..]).ok_or(InterpError::MalformedCode)?;
        cur += n;
        cur += result_count as usize;
    }
    Ok(count)
}

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

        let (local_groups, n) = read_u32_leb128(entry).ok_or(InterpError::MalformedCode)?;
        let mut lc = n;
        let mut total_declared = 0usize;
        for _ in 0..local_groups {
            let (group_count, n) = read_u32_leb128(&entry[lc..]).ok_or(InterpError::MalformedCode)?;
            lc += n + 1;
            total_declared += group_count as usize;
        }
        if lc > entry.len() { return Err(InterpError::MalformedCode); }

        bodies[i]       = &entry[lc..];
        local_counts[i] = total_declared;
        cur = entry_end;
    }
    Ok(count)
}

// ── Signed LEB-128 ────────────────────────────────────────────────────────────

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
