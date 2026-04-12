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


#[repr(u8)]
pub enum Cond {
    E = 0x4,  // equal / zero
    NE = 0x5, // not equal
    L = 0xC,  // less (signed)
    LE = 0xE, // less or equal (signed)
    G = 0xF,  // greater (signed)
    GE = 0xD, // greater or equal (signed)
    B = 0x2,  // below (unsigned)
    BE = 0x6, // below or equal (unsigned)
    A = 0x7,  // above (unsigned)
    AE = 0x3, // above or equal (unsigned)
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

    // ── Arithmetic (reg←reg) ──────────────────────────────────────────────────

    /// `add dst, src`  (64-bit, dst += src)
    pub fn emit_add_rr(&mut self, dst: Reg, src: Reg) {
        self.emit_rex_rr(dst, src);
        self.emit_u8(0x03); // ADD r64, r/m64
        self.emit_modrm_rr(dst, src);
    }

    /// `sub dst, src`  (64-bit, dst -= src)
    pub fn emit_sub_rr(&mut self, dst: Reg, src: Reg) {
        self.emit_rex_rr(dst, src);
        self.emit_u8(0x2B); // SUB r64, r/m64
        self.emit_modrm_rr(dst, src);
    }

    /// `and dst, src`  (64-bit, dst &= src)
    pub fn emit_and_rr(&mut self, dst: Reg, src: Reg) {
        self.emit_rex_rr(dst, src);
        self.emit_u8(0x23); // AND r64, r/m64
        self.emit_modrm_rr(dst, src);
    }

    /// `or dst, src`  (64-bit, dst |= src)
    pub fn emit_or_rr(&mut self, dst: Reg, src: Reg) {
        self.emit_rex_rr(dst, src);
        self.emit_u8(0x0B); // OR r64, r/m64
        self.emit_modrm_rr(dst, src);
    }

    /// `xor dst, src`  (64-bit, dst ^= src)
    pub fn emit_xor_rr(&mut self, dst: Reg, src: Reg) {
        self.emit_rex_rr(dst, src);
        self.emit_u8(0x33); // XOR r64, r/m64
        self.emit_modrm_rr(dst, src);
    }

    // ── Memory load / store (64-bit displacement) ──────────────────────────────

    /// `mov dst, [base + byte_offset]`  (64-bit load)
    pub fn emit_load_mem(&mut self, dst: Reg, base: Reg, byte_offset: i32) {
        let rex = 0x48
            | (if dst.needs_rex()  { 0x04 } else { 0 })
            | (if base.needs_rex() { 0x01 } else { 0 });
        self.emit_u8(rex);
        self.emit_u8(0x8B); // MOV r64, r/m64
        self.emit_mem_modrm(dst, base, byte_offset);
    }

    /// `mov [base + byte_offset], src`  (64-bit store)
    pub fn emit_store_mem(&mut self, src: Reg, base: Reg, byte_offset: i32) {
        let rex = 0x48
            | (if src.needs_rex()  { 0x04 } else { 0 })
            | (if base.needs_rex() { 0x01 } else { 0 });
        self.emit_u8(rex);
        self.emit_u8(0x89); // MOV r/m64, r64
        self.emit_mem_modrm(src, base, byte_offset);
    }

    // ── RSP-relative arithmetic ───────────────────────────────────────────────

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

    // ── Multiplication ────────────────────────────────────────────────────────────

    /// `imul dst, src`  (64-bit signed multiply, dst ← dst * src)
    pub fn emit_imul_rr(&mut self, dst: Reg, src: Reg) {
        self.emit_rex_rr(dst, src);
        self.emit_u8(0x0F);
        self.emit_u8(0xAF);
        self.emit_modrm_rr(dst, src);
    }

    // ── Shift operations with CL ─────────────────────────────────────────────────

    /// `sar reg, cl`  (arithmetic right shift by CL)
    pub fn emit_sar_rcx(&mut self, reg: Reg) {
        self.emit_rex_rr(Reg::Rcx, reg);
        self.emit_u8(0xD3);
        self.emit_u8(0xF8 | reg.enc());  // ModRM: /7 for SAR
    }

    /// `shl reg, cl`  (logical left shift by CL)
    pub fn emit_shl_rcx(&mut self, reg: Reg) {
        self.emit_rex_rr(Reg::Rcx, reg);
        self.emit_u8(0xD3);
        self.emit_u8(0xE0 | reg.enc());  // ModRM: /4 for SHL
    }

    /// `shr reg, cl`  (logical right shift by CL)
    pub fn emit_shr_rcx(&mut self, reg: Reg) {
        self.emit_rex_rr(Reg::Rcx, reg);
        self.emit_u8(0xD3);
        self.emit_u8(0xE8 | reg.enc());  // ModRM: /5 for SHR
    }

    // ── Negation ─────────────────────────────────────────────────────────────────

    /// `neg reg`  (two's complement negation)
    pub fn emit_neg(&mut self, reg: Reg) {
        self.emit_rex_w();
        self.emit_u8(0xF7);
        self.emit_u8(0xD8 | reg.enc());  // ModRM: /3 for NEG
    }

    // ── Conditional set byte ─────────────────────────────────────────────────────

    /// `setcc cond, reg`  (set byte to 0/1 based on condition flags)
    pub fn emit_setcc(&mut self, cond: Cond, reg: Reg) {
        if reg.needs_rex() {
            self.emit_u8(0x40 | (reg.enc() >> 3));
        }
        self.emit_u8(0x0F);
        self.emit_u8(0x90 | (cond as u8));
        self.emit_modrm_rr(reg, Reg::Rax);  // ModRM with reg field
    }

    /// `movzx dst8, src8`  (zero-extend byte to 32-bit)
    pub fn emit_movzx_r32_r8(&mut self, dst: Reg, src: Reg) {
        if dst.needs_rex() || src.needs_rex() {
            self.emit_u8(0x40 | 
                (if dst.needs_rex() { 0x4 } else { 0 }) |
                (if src.needs_rex() { 0x1 } else { 0 }));
        }
        self.emit_u8(0x0F);
        self.emit_u8(0xB6);
        self.emit_modrm_rr(dst, src);
    }

    // ── Sign/zero extend memory loads ────────────────────────────────────────────

    /// `movsx dst, [base + offset]`  (sign-extend byte to 32-bit)
    pub fn emit_movsx_r32_mem8(&mut self, dst: Reg, base: Reg, offset: i32) {
        self.emit_rex_rr(dst, base);
        self.emit_u8(0x0F);
        self.emit_u8(0xBE);
        self.emit_mem_modrm(dst, base, offset);
    }

    /// `movzx dst, [base + offset]`  (zero-extend byte to 32-bit)
    pub fn emit_movzx_r32_mem8(&mut self, dst: Reg, base: Reg, offset: i32) {
        self.emit_rex_rr(dst, base);
        self.emit_u8(0x0F);
        self.emit_u8(0xB6);
        self.emit_mem_modrm(dst, base, offset);
    }

    // Helper for memory addressing with displacement
    fn emit_mem_modrm(&mut self, reg: Reg, base: Reg, offset: i32) {
        if offset == 0 && base.enc() != 5 {
            self.emit_u8(0x00 | (reg.enc() << 3) | base.enc());
        } else if offset >= -128 && offset <= 127 {
            self.emit_u8(0x40 | (reg.enc() << 3) | base.enc());
            self.emit_u8(offset as u8);
        } else {
            self.emit_u8(0x80 | (reg.enc() << 3) | base.enc());
            self.emit_i32(offset);
        }
    }

    // ── Memory load/store with bounds checking ───────────────────────────────────

    /// Load u64 from [base + offset*8] with bounds checking
    /// offset_reg should contain byte offset, not element index
    pub fn emit_mem_load_u64(&mut self, dst: Reg, base: Reg, offset_reg: Reg) {
        // mov dst, [base + offset_reg]
        self.emit_rex_rr(dst, base);
        self.emit_u8(0x8B);
        if offset_reg == Reg::Rax {
            self.emit_u8(0x00 | (dst.enc() << 3) | base.enc());
        } else {
            self.emit_u8(0x04 | (dst.enc() << 3));  // SIB byte
            self.emit_u8((offset_reg.enc() << 3) | base.enc());
        }
    }

    /// Store u64 from src to [base + offset_reg]
    pub fn emit_mem_store_u64(&mut self, src: Reg, base: Reg, offset_reg: Reg) {
        self.emit_rex_rr(src, base);
        self.emit_u8(0x89);
        if offset_reg == Reg::Rax {
            self.emit_u8(0x00 | (src.enc() << 3) | base.enc());
        } else {
            self.emit_u8(0x04 | (src.enc() << 3));
            self.emit_u8((offset_reg.enc() << 3) | base.enc());
        }
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

/// Calling convention for JIT-compiled WASM functions (System V AMD64):
///   RDI = linear memory base (`*mut u8`)
///   RSI = globals base      (`*mut i64`)
///   RDX = locals base       (`*mut i64`, pre-filled by caller with params)
/// Returns the top-of-operand-stack value as i64 (0 if void).
pub type JitFn = unsafe extern "C" fn(*mut u8, *mut i64, *mut i64) -> i64;
