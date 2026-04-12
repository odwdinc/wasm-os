# Epic: NES WASM Runtime — 1200ms → 16ms/frame


## Sprint 0 — Boot Prerequisites
**Goal:** One-time setup required before any JIT code can execute.

### Deliverables:
* [x] Call `make_jit_executable()` in `kernel/src/main.rs` at boot, after the IDT and PIT are initialised but before the scheduler starts. Without this, any JIT'd function pointer invocation faults immediately.
* [x] Verify `JIT_BUF_SIZE` (currently 512 KiB) is large enough for AOT of all nes.wasm functions. Measure: dump `jit_used()` after compiling all bodies at spawn time. nes.wasm has ~144 functions; x86 output is typically 5–15x bytecode size. If usage exceeds 400 KiB, increase `JIT_BUF_SIZE` to 2 MiB or implement selective (hot-only) JIT before Sprint 3 builds on top of it.

### Success:
`make_jit_executable()` called, no page-fault on a hand-written test stub invoked via `jit_alloc`.


## Sprint 1 — Instrumentation & Baseline
**Goal:** Know exactly how many WASM ops/frame and real elapsed time (not 10ms-granular).

### Deliverables:
* [x] **RDTSC host timer** — replace `host_uptime_ms` with TSC-based impl: `rdtsc` → divide by calibrated MHz constant → milliseconds. Calibrate at boot using one PIT tick (10ms = known interval).
* [x] **Opcode counter** — add `static mut OPCODE_COUNT: u64` to `interp.rs`, increment each `run()` iteration. Expose via new host fn `wasm_opcount() → i64` registered in `engine::init_host_fns`.
* [x] **NES timing report** — extend `nes-wasm/src/lib.rs` print block: add ops/frame and ops/sec alongside ms/frame.
* [x] **Per-function call counter** (stretch) — `[u32; MAX_FUNCS]` call count array in `Interpreter`, printed via host fn or at task exit.

### Success:
Can read `ops/frame=47M ops/sec=39K` style output from serial. Timing accurate to <1ms.


## Sprint 2 — Interpreter Hot-Loop Micro-Optimizations
**Goal:** Extract 2–4x from interpreter without JIT. Low risk, immediate gain.

### Deliverables:
* [x] **Cache frame fields as locals in `run()`** — current code re-fetches `fi`, `body_idx`, `body`, `pc` from `self.frames[fi]` on every opcode. Hoist into local Rust variables; write `pc` back to the frame only on PC-advancing ops and at loop iteration end. Reduces ~6 array indexing operations per opcode.
* [x] **Inline `v_push`/`v_pop` in hot paths** — added `#[inline(always)]` to `v_push`/`v_pop` to ensure inlining.
* [x] **Pre-compute `active_mem_bytes` once** — hoisted to local `active_mem` at start of `run()`, updated only on `memory.grow`.
* [x] **PIT → 1000 Hz** — reprogrammed `PIT_DIVISOR` from `11_931` to `1_193` (÷10). Fixed `uptime_ms` fallback multiplier to `ticks * 1`. Fixed TSC calibration divisor from `/ 10_000` to `/ 1_000`.

### Success:
ms/frame drops measurably (target: <600ms). Opcode counter validates throughput increase.


## Sprint 3 — JIT Foundation: Calling Convention + Arithmetic
**Goal:** First JIT'd WASM functions execute correctly. Prove the pipeline end-to-end.

### Deliverables:
* [x] **Commit to calling convention** — use the x86 stack as the WASM operand stack (each WASM i64 value is pushed/popped via `push`/`pop` on RSP). Callee-saved registers hold pointers that persist across the whole function:
  ```
  R13 = locals base  (*mut i64, frame-allocated on x86 stack at prologue)
  R14 = globals base (*mut i64, passed in as arg)
  R15 = mem base     (*mut u8,  passed in as arg)
  ```
  JIT function signature (System V AMD64):
  ```rust
  fn jit_func(mem: *mut u8, globals: *mut i64) -> i32  // RDI=mem, RSI=globals
  ```
  Locals allocated by `sub rsp, locals_count*8` in prologue; freed in epilogue.

* [x] **Extend `jit/emit.rs`** with helpers needed before the compilation pass can work:
  - `emit_imul_rr(dst, src)` — `imul r64, r64`
  - `emit_sar_rcx(reg)` — `sar reg, cl` (arithmetic right shift for `i32.shr_s`)
  - `emit_shl_rcx(reg)` / `emit_shr_rcx(reg)` — shift by CL
  - `emit_neg(reg)` — two's complement negate
  - `emit_setcc(cond, reg)` — `sete`/`setl`/`setle` etc. + `movzx` for comparison results
  - `emit_movsx_r32_mem8(dst, base, offset)` / `emit_movzx_r32_mem8` — for narrow loads
  - `emit_mem_load_u8/u16/u32/u64(dst, base_reg, offset_reg)` — bounds-checked load patterns
  - `emit_mem_store_u8/u16/u32/u64(src, base_reg, offset_reg)` — bounds-checked store patterns

* [x] **JIT compilation pass** — `fn jit_compile_body(body: &[u8], locals_count: usize, buf: &mut CodeBuf) -> bool` that walks WASM bytecode and emits x86-64 for:
  - `i32.const` / `i64.const` → `push imm64`
  - `local.get N` / `local.set N` / `local.tee N` → load/store `[R13 + N*8]`
  - `i32.add/sub/mul/and/or/xor/shl/shr_u/shr_s/rotl/rotr`
  - `drop` → `add rsp, 8`
  - `return` → restore RSP (unwind locals), emit epilogue + `ret`
  - Returns `false` for any unrecognised opcode → caller falls back to interpreter for that function

* [x] **JIT table in engine** — `[Option<unsafe fn(*mut u8, *mut i64) -> i32>; MAX_FUNCS]` per instance. Populated at spawn time for functions that compile successfully. Miss → interpreter handles that function.

* [x] **Call from interpreter into JIT** — when `OP_CALL` targets a JIT-compiled function index, invoke the native pointer directly instead of pushing an interpreter frame. Pass `self.mem.as_mut_ptr()` and `self.globals.as_mut_ptr()`.

* [x] **Shadow-mode correctness check** — gated by `#[cfg(feature = "jit_shadow")]`: run the function in both JIT and interpreter, compare vstack top after return, panic/print on mismatch. Enable during Sprint 3–4 development; disable for production.

### Success:
`fib.wasm` with N=30 runs 10–20x faster through JIT path vs interpreter (measured with RDTSC timer from Sprint 1). NES still interpreter-only at this stage (control flow not yet JIT'd).


## Sprint 4 — JIT Control Flow: block/loop/if/br
**Goal:** JIT handles all control flow nes.wasm uses. NES CPU/PPU emulation can run JIT'd.

### Deliverables:
* [ ] **Two-pass compiler** — pass 1: reuse `pre_scan_body_jumps` jump-table data already built at instantiation to resolve all `block`/`loop`/`if` → `end`/`else` PC pairs. Pass 2: emit x86, patching forward `rel32` references with `CodeBuf::patch_rel32`.
* [ ] **`block`** → record end label position; `br 0` inside → `jmp end_label`
* [ ] **`loop`** → record start label; `br 0` inside → `jmp start_label`
* [ ] **`if` / `else` / `end`** → pop condition, `test rax, rax` + `jz else_or_end_label`
* [ ] **`br N`** → unwind N control frames, `jmp` to target label
* [ ] **`br_if N`** → `pop rcx; test rcx, rcx; jnz target`
* [ ] **`br_table`** → emit a jump table in the code buffer: `cmp i, count; cmovae i, count; jmp [rip + table + i*8]`
* [ ] **`select`** → `pop cond; pop b; pop a; test cond,cond; cmovz a, b; push a`
* [ ] **`call` between JIT functions** → `call [jit_fn_ptr]` direct
* [ ] **`call` to host fn** → emit `mov rdi, mem_base; mov rsi, globals_base; mov rdx, host_fn_ptr; call rdx` — marshal vstack in/out around host call
* [ ] **Type conversion opcodes** (required by nes.wasm):
  - `i32.wrap_i64` → `and rax, 0xFFFFFFFF`
  - `i64.extend_i32_s` → `movsxd rax, eax`
  - `i64.extend_i32_u` → `mov eax, eax` (zero-extends implicitly)
* [ ] **Sign-extension opcodes** (required by nes.wasm):
  - `i32.extend8_s` → `movsx eax, al`
  - `i32.extend16_s` → `movsx eax, ax`
  - `i64.extend8_s` / `i64.extend16_s` / `i64.extend32_s` → `movsx rax, al/ax/eax`

### Success:
nes.wasm functions compile via JIT. First JIT-driven NES frame completes. Target: <100ms/frame.


## Sprint 5 — JIT Memory Ops + Full Opcode Coverage
**Goal:** All opcodes that nes.wasm emits are handled by JIT. Zero interpreter fallback on hot path.

### Deliverables:
* [ ] **Memory loads** — `i32.load`, `i32.load8_u`, `i32.load8_s`, `i32.load16_u`, `i32.load16_s`, `i64.load`, all narrow i64 loads → emit bounds check (`cmp ea, mem_pages*65536; jae trap`) + `movzx`/`movsx`/`mov` from `[R15 + ea]`
* [ ] **Memory stores** — `i32.store`, `i32.store8`, `i32.store16`, `i64.store`, narrow i64 stores → bounds check + `mov [R15 + ea], ...`
* [ ] **Float load/store** — `f32.load` (0x2A), `f64.load` (0x2B), `f32.store` (0x38), `f64.store` (0x39) — treat as opaque bit patterns (`i32`/`i64`); same emit as integer loads/stores of the same width. nes.wasm does not perform float arithmetic but the opcodes appear in wee_alloc.
* [ ] **`memory.size`** → `push current_pages` (load from instance state)
* [ ] **`memory.grow`** → call a host-side helper that attempts to extend `current_pages`; pushes new page count or -1. Required: `wee_alloc` inside nes.wasm calls this at startup.
* [ ] **Globals** — `global.get N` / `global.set N` → load/store `[R14 + N*8]`
* [ ] **Remaining i32 ops** — `i32.eqz`, `i32.eq`, `i32.ne`, `i32.lt_s/u`, `i32.gt_s/u`, `i32.ge_s/u`, `i32.le_s/u`, `i32.clz` (`lzcnt`), `i32.ctz` (`tzcnt`), `i32.popcnt`, `i32.div_s/u`, `i32.rem_s/u`
* [ ] **i64 arithmetic** (entire group — missing from earlier sprints):
  - `i64.add/sub/mul/and/or/xor/shl/shr_u/shr_s/rotl/rotr`
  - `i64.eqz/eq/ne/lt_s/u/gt_s/u/ge_s/u/le_s/u`
  - `i64.clz/ctz/popcnt`
  - `i64.div_s/u`, `i64.rem_s/u`
* [ ] **`call_indirect`** → emit table bounds check + type-signature check + `call [table + idx*8]`
* [ ] **Fallback audit** — add a `static mut JIT_MISS_COUNT: u64` incremented whenever the interpreter is entered for a function that was expected to be JIT'd. Print at frame report. Target: zero misses during NES run.

### Success:
Zero interpreter fallback during NES frame. Target: <30ms/frame.


## Sprint 6 — Frame Pacing + Reach 16ms Target
**Goal:** Accurate frame timing + final tuning. Ship it.

### Deliverables:
* [ ] **RDTSC frame pacing in nes-wasm** — use `uptime_ms()` (now sub-ms via RDTSC from Sprint 1) for proper elapsed/sleep calculation. The `elapsed < 16` branch in `lib.rs:313` now fires correctly.
* [ ] **`sleep_ms` accuracy** — at 1000Hz PIT (Sprint 2), `sleep_ms(1)` = 1ms. Fix NES emulator to `sleep_ms(0)` (cooperative yield only) inside the frame loop; rely on scheduler for pacing. Remove the `sleep_ms(16 - elapsed)` call that currently sleeps a minimum of 10ms.
* [ ] **`fb_blit` performance audit** — blitting 256×240 = 61,440 pixels through VGA MMIO every frame. Measure time spent in `host_fb_blit` using RDTSC before/after. If >2ms, investigate batching or DMA-style copy.
* [ ] **JIT buffer usage report** — print `jit_used()` at NES spawn. Confirm 512 KiB is sufficient. If not, increase `JIT_BUF_SIZE` or restrict JIT to functions with call-count > threshold.
* [ ] **`wee_alloc` correctness under JIT** — `memory.grow` (Sprint 5) must work correctly; wee_alloc calls it at startup, not in the hot path. Verify NES runs without memory fault after JIT enabled.
* [ ] **Final profiling pass** — use per-function call counters (Sprint 1 stretch goal) to identify any remaining hot interpreter path. Compile and tune.
* [ ] **Regression test** — run `fib.wasm`, `primes.wasm`, `httpd.wasm` under JIT to confirm no breakage from the new dispatch path.

### Success:
nes.wasm prints `ms/frame=14-18 (target 16)` stably across 600+ frames.


## Speedup Projection

| After Sprint | Expected ms/frame | Speedup vs baseline |
|---|---|---|
| 0 (boot prereq) | 1200ms | 1x |
| 1 (measurement) | 1200ms | 1x (better data) |
| 2 (interp opts + 1kHz PIT) | ~400ms | ~3x |
| 3 (JIT foundation, NES still interp) | ~400ms | ~3x |
| 4 (JIT control flow) | ~80ms | ~15x |
| 5 (JIT full coverage) | ~20ms | ~60x |
| 6 (pacing + tuning) | **~16ms** | **~75x** |


## Risk Register

| Risk | Likelihood | Mitigation |
|---|---|---|
| JIT buffer (512 KiB) too small for full AOT | Medium | Measure in Sprint 0; increase or use selective JIT before Sprint 3 |
| JIT calling convention wrong, requires rework | Medium | Validate with `fib.wasm` in Sprint 3 before wiring NES; shadow mode catches bugs |
| Two-pass compiler misses `br_table` edge case | Low–Medium | Fall back to interpreter for functions that fail to compile; log the miss |
| `memory.grow` under JIT breaks `wee_alloc` | Low | Sprint 5 includes explicit wee_alloc correctness check |
| RDTSC frequency varies (no `CPUID` in bare metal) | Low | Calibrate against PIT at boot; accept ±2% timing error |
| `fb_blit` consumes >2ms of frame budget | Low–Medium | Audit in Sprint 6; if hot, optimise or move to async blit |
