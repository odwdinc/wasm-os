//! x86-64 instruction emitter.
//!
//! [`CodeBuf`] wraps a mutable byte slice and tracks the write cursor.
//! All `emit_*` methods append bytes to the slice and advance the cursor.
//! The buffer must be large enough; writes past the end are silently ignored
//! (callers must size the buffer conservatively).
//!
//! # Register encoding
//!
//! Use the [`Reg`] enum for all register arguments.  Registers with indices
//! ≥ 8 (R8–R15) automatically set the appropriate REX.R / REX.B bit.

/// x86-64 general-purpose register encoding (integer registers only).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum Reg {
    Rax = 0,
    Rcx = 1,
    Rdx = 2,
    Rbx = 3,
    Rsp = 4,
    Rbp = 5,
    Rsi = 6,
    Rdi = 7,
    R8  = 8,
    R9  = 9,
    R10 = 10,
    R11 = 11,
    R12 = 12,
    R13 = 13,
    R14 = 14,
    R15 = 15,
}

impl Reg {
    /// Low 3 bits of the ModRM / SIB register field.
    #[inline] pub fn enc(self) -> u8 { (self as u8) & 7 }
    /// True if the register needs a REX extension bit (R8–R15).
    #[inline] pub fn needs_rex(self) -> bool { (self as u8) >= 8 }
}

// ── CodeBuf ───────────────────────────────────────────────────────────────────

/// A write cursor over a mutable byte slice for emitting x86-64 machine code.
pub struct CodeBuf<'a> {
    buf:    &'a mut [u8],
    cursor: usize,
}

impl<'a> CodeBuf<'a> {
    /// Create a new `CodeBuf` starting at the beginning of `buf`.
    pub fn new(buf: &'a mut [u8]) -> Self {
        CodeBuf { buf, cursor: 0 }
    }

    /// Number of bytes emitted so far.
    pub fn len(&self) -> usize { self.cursor }

    /// Current write position (same as `len`).
    pub fn pos(&self) -> usize { self.cursor }

    // ── Primitive emitters ────────────────────────────────────────────────────

    pub fn emit_u8(&mut self, b: u8) {
        if self.cursor < self.buf.len() {
            self.buf[self.cursor] = b;
            self.cursor += 1;
        }
    }

    pub fn emit_u16(&mut self, v: u16) {
        self.emit_u8(v as u8);
        self.emit_u8((v >> 8) as u8);
    }

    pub fn emit_u32(&mut self, v: u32) {
        self.emit_u8( v        as u8);
        self.emit_u8((v >>  8) as u8);
        self.emit_u8((v >> 16) as u8);
        self.emit_u8((v >> 24) as u8);
    }

    pub fn emit_u64(&mut self, v: u64) {
        self.emit_u32(v as u32);
        self.emit_u32((v >> 32) as u32);
    }

    pub fn emit_i32(&mut self, v: i32) { self.emit_u32(v as u32); }

    // ── REX prefix helpers ────────────────────────────────────────────────────

    /// REX.W prefix (operand size = 64-bit).
    pub fn emit_rex_w(&mut self) { self.emit_u8(0x48); }

    /// REX.W + REX.B (dst is R8–R15).
    pub fn emit_rex_wb(&mut self) { self.emit_u8(0x49); }

    /// REX.W + REX.R (reg field is R8–R15).
    pub fn emit_rex_wr(&mut self) { self.emit_u8(0x4C); }

    /// REX.W + REX.R + REX.B.
    pub fn emit_rex_wrb(&mut self) { self.emit_u8(0x4D); }

    /// Emit the correct REX.W prefix for a two-register operation.
    /// Sets REX.R if `reg` ≥ 8, REX.B if `rm` ≥ 8.
    pub fn emit_rex_rr(&mut self, reg: Reg, rm: Reg) {
        let r = if reg.needs_rex() { 0x44 } else { 0x40 };
        let b = if rm.needs_rex()  { 0x41 } else { 0x40 };
        self.emit_u8(0x48 | (r & 4) | (b & 1));
    }

    // ── ModRM ─────────────────────────────────────────────────────────────────

    /// ModRM byte: mod=11 (register direct), reg field, rm field.
    pub fn emit_modrm_rr(&mut self, reg: Reg, rm: Reg) {
        self.emit_u8(0xC0 | (reg.enc() << 3) | rm.enc());
    }

    // ── MOV ──────────────────────────────────────────────────────────────────

    /// `mov dst, src`  (64-bit register←register)
    pub fn emit_mov_rr(&mut self, dst: Reg, src: Reg) {
        self.emit_rex_rr(src, dst); // REX with reg=src, rm=dst
        self.emit_u8(0x89);         // MOV r/m64, r64
        self.emit_modrm_rr(src, dst);
    }

    /// `mov dst, imm64`
    pub fn emit_mov_ri64(&mut self, dst: Reg, imm: u64) {
        if dst.needs_rex() { self.emit_rex_wb(); } else { self.emit_rex_w(); }
        self.emit_u8(0xB8 | dst.enc()); // MOV r64, imm64
        self.emit_u64(imm);
    }

    /// `mov dst, imm32`  (zero-extends to 64 bits)
    pub fn emit_mov_ri32(&mut self, dst: Reg, imm: u32) {
        if dst.needs_rex() { self.emit_u8(0x41); } // REX.B
        self.emit_u8(0xB8 | dst.enc()); // MOV r32, imm32
        self.emit_u32(imm);
    }

    // ── PUSH / POP ────────────────────────────────────────────────────────────

    /// `push reg`  (64-bit)
    pub fn emit_push(&mut self, reg: Reg) {
        if reg.needs_rex() { self.emit_u8(0x41); } // REX.B
        self.emit_u8(0x50 | reg.enc());
    }

    /// `pop reg`  (64-bit)
    pub fn emit_pop(&mut self, reg: Reg) {
        if reg.needs_rex() { self.emit_u8(0x41); } // REX.B
        self.emit_u8(0x58 | reg.enc());
    }

    // ── Arithmetic ────────────────────────────────────────────────────────────

    /// `add rsp, imm8`
    pub fn emit_add_rsp_i8(&mut self, imm: i8) {
        self.emit_rex_w();
        self.emit_u8(0x83);             // ADD r/m64, imm8
        self.emit_u8(0xC4);             // ModRM: mod=11, /0, rsp(4)
        self.emit_u8(imm as u8);
    }

    /// `sub rsp, imm8`
    pub fn emit_sub_rsp_i8(&mut self, imm: i8) {
        self.emit_rex_w();
        self.emit_u8(0x83);             // SUB r/m64, imm8
        self.emit_u8(0xEC);             // ModRM: mod=11, /5, rsp(4)
        self.emit_u8(imm as u8);
    }

    // ── RET ───────────────────────────────────────────────────────────────────

    /// `ret`  (near return)
    pub fn emit_ret(&mut self) { self.emit_u8(0xC3); }

    // ── Branches ─────────────────────────────────────────────────────────────

    /// `jmp rel32`  (unconditional near jump, relative to end of instruction)
    pub fn emit_jmp_rel32(&mut self, rel: i32) {
        self.emit_u8(0xE9);
        self.emit_i32(rel);
    }

    /// `jz rel32`  (jump if zero / equal)
    pub fn emit_jz_rel32(&mut self, rel: i32) {
        self.emit_u8(0x0F);
        self.emit_u8(0x84);
        self.emit_i32(rel);
    }

    /// `jnz rel32`
    pub fn emit_jnz_rel32(&mut self, rel: i32) {
        self.emit_u8(0x0F);
        self.emit_u8(0x85);
        self.emit_i32(rel);
    }

    /// Return the current position as a placeholder for a future `rel32` patch.
    /// Write 4 zero bytes; call [`CodeBuf::patch_rel32`] once the target is known.
    pub fn emit_rel32_placeholder(&mut self) -> usize {
        let pos = self.cursor;
        self.emit_u32(0);
        pos
    }

    /// Patch a previously emitted `rel32` at byte offset `patch_pos`.
    /// `target_pos` is the byte offset of the target instruction.
    /// `instr_end` is the byte offset immediately after the branch instruction
    /// (i.e. the position from which the CPU adds the relative offset).
    pub fn patch_rel32(&mut self, patch_pos: usize, instr_end: usize, target_pos: usize) {
        let rel = (target_pos as i64 - instr_end as i64) as i32;
        let bytes = rel.to_le_bytes();
        if patch_pos + 4 <= self.buf.len() {
            self.buf[patch_pos..patch_pos + 4].copy_from_slice(&bytes);
        }
    }

    // ── CMP ───────────────────────────────────────────────────────────────────

    /// `cmp reg, 0`  (TEST reg, reg)
    pub fn emit_test_rr(&mut self, reg: Reg) {
        self.emit_rex_rr(reg, reg);
        self.emit_u8(0x85);
        self.emit_modrm_rr(reg, reg);
    }

    /// `cmp reg, imm32`
    pub fn emit_cmp_ri32(&mut self, reg: Reg, imm: i32) {
        if reg.needs_rex() { self.emit_u8(0x49); } else { self.emit_rex_w(); }
        self.emit_u8(0x81);
        self.emit_u8(0xF8 | reg.enc()); // ModRM: /7
        self.emit_i32(imm);
    }

    // ── CALL ─────────────────────────────────────────────────────────────────

    /// `call *reg`  (indirect call through register, 64-bit)
    pub fn emit_call_r(&mut self, reg: Reg) {
        if reg.needs_rex() { self.emit_u8(0x41); }
        self.emit_u8(0xFF);
        self.emit_u8(0xD0 | reg.enc()); // ModRM: /2
    }

    // ── Prologue / epilogue ───────────────────────────────────────────────────

    /// Standard System-V prologue.
    ///
    /// ```text
    ///   push rbp
    ///   mov  rbp, rsp
    ///   push rbx
    ///   push r12
    ///   push r13
    ///   push r14
    ///   push r15
    ///   sub  rsp, 8        ; padding → 16-byte stack alignment
    /// ```
    pub fn emit_prologue(&mut self) {
        self.emit_push(Reg::Rbp);
        self.emit_mov_rr(Reg::Rbp, Reg::Rsp);
        self.emit_push(Reg::Rbx);
        self.emit_push(Reg::R12);
        self.emit_push(Reg::R13);
        self.emit_push(Reg::R14);
        self.emit_push(Reg::R15);
        self.emit_sub_rsp_i8(8);
    }

    /// Standard System-V epilogue (mirror of [`emit_prologue`]).
    ///
    /// ```text
    ///   add  rsp, 8
    ///   pop  r15
    ///   pop  r14
    ///   pop  r13
    ///   pop  r12
    ///   pop  rbx
    ///   pop  rbp
    ///   ret
    /// ```
    pub fn emit_epilogue(&mut self) {
        self.emit_add_rsp_i8(8);
        self.emit_pop(Reg::R15);
        self.emit_pop(Reg::R14);
        self.emit_pop(Reg::R13);
        self.emit_pop(Reg::R12);
        self.emit_pop(Reg::Rbx);
        self.emit_pop(Reg::Rbp);
        self.emit_ret();
    }
}

// ── Type alias for a callable JIT'd function ─────────────────────────────────

/// A JIT-compiled function with no arguments and an `i32` return value.
/// Cast the `*mut u8` returned by [`super::jit_alloc`] to this type after
/// calling [`super::make_jit_executable`].
pub type JitFn = unsafe extern "C" fn() -> i32;
