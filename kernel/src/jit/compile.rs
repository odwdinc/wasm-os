// jit/compile.rs — Sprint 3 JIT compiler: arithmetic + locals, no control flow.
//
// Calling convention (System V AMD64):
//   RDI → R15 = linear memory base
//   RSI → R14 = globals base
//   RDX → R13 = locals base (pre-filled by caller: params first, then zeroed)
//   R12        = saved RSP  (WASM operand stack base; restored on return)
//
// WASM operand stack IS the x86 stack (push/pop via RSP).
// Locals live in caller-supplied [i64; MAX_LOCALS] buffer at [R13 + idx*8].

use super::emit::{CodeBuf, Reg};
use crate::wasm::loader::read_u32_leb128;
use crate::wasm::interp::{read_i32_leb128, read_i64_leb128};
use crate::wasm::opcode;

/// JIT-compile a WASM function body.
///
/// * `body`         — function bytecode (after local-declaration prefix).
/// * `locals_count` — total local slots: `n_params + n_declared_locals`.
/// * `result_count` — 0 or 1 (multi-result functions are rejected).
/// * `buf`          — destination code buffer.
///
/// Returns `true` on success, `false` if any unsupported opcode is encountered
/// (caller should fall back to the interpreter for that function).
pub fn jit_compile_body(
    body: &[u8],
    _locals_count: usize,
    result_count: usize,
    buf: &mut CodeBuf,
) -> bool {
    // Only 0 or 1 results supported in Sprint 3.
    if result_count > 1 { return false; }

    // ── Prologue ─────────────────────────────────────────────────────────────
    // Saves RBP, RBX, R12–R15 per System V ABI; keeps 16-byte stack alignment.
    buf.emit_prologue();
    // Set up JIT register aliases from function arguments.
    buf.emit_mov_rr(Reg::R15, Reg::Rdi); // mem base
    buf.emit_mov_rr(Reg::R14, Reg::Rsi); // globals base
    buf.emit_mov_rr(Reg::R13, Reg::Rdx); // locals base (caller-provided)
    buf.emit_mov_rr(Reg::R12, Reg::Rsp); // save RSP → used to unwind WASM stack on return

    let mut ip = 0usize;

    while ip < body.len() {
        match body[ip] {
            // ── i32.const / i64.const ─────────────────────────────────────
            opcode::OP_I32_CONST => {
                let Some((val, sz)) = read_i32_leb128(&body[ip + 1..]) else { return false; };
                buf.emit_mov_ri64(Reg::Rax, val as i64 as u64);
                buf.emit_push(Reg::Rax);
                ip += 1 + sz;
            }

            opcode::OP_I64_CONST => {
                let Some((val, sz)) = read_i64_leb128(&body[ip + 1..]) else { return false; };
                buf.emit_mov_ri64(Reg::Rax, val as u64);
                buf.emit_push(Reg::Rax);
                ip += 1 + sz;
            }

            // ── local.get / local.set / local.tee ─────────────────────────
            opcode::OP_LOCAL_GET => {
                let Some((idx, sz)) = read_u32_leb128(&body[ip + 1..]) else { return false; };
                let offset = (idx as usize).wrapping_mul(8) as i32;
                buf.emit_load_mem(Reg::Rax, Reg::R13, offset); // mov rax, [R13 + idx*8]
                buf.emit_push(Reg::Rax);
                ip += 1 + sz;
            }

            opcode::OP_LOCAL_SET => {
                let Some((idx, sz)) = read_u32_leb128(&body[ip + 1..]) else { return false; };
                let offset = (idx as usize).wrapping_mul(8) as i32;
                buf.emit_pop(Reg::Rax);
                buf.emit_store_mem(Reg::Rax, Reg::R13, offset); // mov [R13 + idx*8], rax
                ip += 1 + sz;
            }

            opcode::OP_LOCAL_TEE => {
                let Some((idx, sz)) = read_u32_leb128(&body[ip + 1..]) else { return false; };
                let offset = (idx as usize).wrapping_mul(8) as i32;
                // Peek top without consuming: pop + push + store
                buf.emit_pop(Reg::Rax);
                buf.emit_push(Reg::Rax);
                buf.emit_store_mem(Reg::Rax, Reg::R13, offset);
                ip += 1 + sz;
            }

            // ── i32 arithmetic ────────────────────────────────────────────
            opcode::OP_I32_ADD => {
                buf.emit_pop(Reg::Rcx);
                buf.emit_pop(Reg::Rax);
                buf.emit_add_rr(Reg::Rax, Reg::Rcx);
                buf.emit_push(Reg::Rax);
                ip += 1;
            }

            opcode::OP_I32_SUB => {
                buf.emit_pop(Reg::Rcx);
                buf.emit_pop(Reg::Rax);
                buf.emit_sub_rr(Reg::Rax, Reg::Rcx);
                buf.emit_push(Reg::Rax);
                ip += 1;
            }

            opcode::OP_I32_MUL => {
                buf.emit_pop(Reg::Rcx);
                buf.emit_pop(Reg::Rax);
                buf.emit_imul_rr(Reg::Rax, Reg::Rcx);
                buf.emit_push(Reg::Rax);
                ip += 1;
            }

            opcode::OP_I32_AND => {
                buf.emit_pop(Reg::Rcx);
                buf.emit_pop(Reg::Rax);
                buf.emit_and_rr(Reg::Rax, Reg::Rcx);
                buf.emit_push(Reg::Rax);
                ip += 1;
            }

            opcode::OP_I32_OR => {
                buf.emit_pop(Reg::Rcx);
                buf.emit_pop(Reg::Rax);
                buf.emit_or_rr(Reg::Rax, Reg::Rcx);
                buf.emit_push(Reg::Rax);
                ip += 1;
            }

            opcode::OP_I32_XOR => {
                buf.emit_pop(Reg::Rcx);
                buf.emit_pop(Reg::Rax);
                buf.emit_xor_rr(Reg::Rax, Reg::Rcx);
                buf.emit_push(Reg::Rax);
                ip += 1;
            }

            opcode::OP_I32_SHL => {
                buf.emit_pop(Reg::Rcx); // shift count
                buf.emit_pop(Reg::Rax); // value
                buf.emit_shl_rcx(Reg::Rax);
                buf.emit_push(Reg::Rax);
                ip += 1;
            }

            opcode::OP_I32_SHR_U => {
                buf.emit_pop(Reg::Rcx);
                buf.emit_pop(Reg::Rax);
                buf.emit_shr_rcx(Reg::Rax);
                buf.emit_push(Reg::Rax);
                ip += 1;
            }

            opcode::OP_I32_SHR_S => {
                buf.emit_pop(Reg::Rcx);
                buf.emit_pop(Reg::Rax);
                buf.emit_sar_rcx(Reg::Rax);
                buf.emit_push(Reg::Rax);
                ip += 1;
            }

            // ── drop ─────────────────────────────────────────────────────
            opcode::OP_DROP => {
                buf.emit_add_rsp_i8(8);
                ip += 1;
            }

            // ── return (explicit) ────────────────────────────────────────
            opcode::OP_RETURN => {
                emit_return(buf, result_count);
                return true;
            }

            // ── end (function body terminator or block end) ───────────────
            opcode::OP_END => {
                ip += 1;
                if ip == body.len() {
                    // Function's implicit return — the only OP_END we handle.
                    emit_return(buf, result_count);
                    return true;
                }
                // Inner block/loop/if end — requires control-flow support.
                return false;
            }

            // ── NOP ──────────────────────────────────────────────────────
            opcode::OP_NOP => { ip += 1; }

            // Anything else → fall back to interpreter.
            _ => return false,
        }
    }

    // Fell off the end without OP_END (shouldn't happen in valid WASM, but be safe).
    emit_return(buf, result_count);
    true
}

/// Emit the function return sequence.
///
/// If `result_count == 1`, pops the top of the WASM operand stack (x86 RSP)
/// into RAX.  Then restores RSP from R12 (discarding leftover WASM stack
/// values) and emits the ABI epilogue + `ret`.
#[inline]
fn emit_return(buf: &mut CodeBuf, result_count: usize) {
    if result_count > 0 {
        buf.emit_pop(Reg::Rax); // top of WASM operand stack → return value
    }
    buf.emit_mov_rr(Reg::Rsp, Reg::R12); // unwind WASM operand stack
    if result_count == 0 {
        // Clear RAX so callers get a defined zero.
        buf.emit_mov_ri32(Reg::Rax, 0);
    }
    buf.emit_epilogue();
}
