# Post-MVP Roadmap

This document scopes out work that begins after the MVP is complete (Sprints 1–4).
Each sprint assumes the previous one is done. Dependencies are noted where they exist.

---

## MVP Exit Condition (recap)

Before starting any sprint here, the following must be true:

```
> run fib.wasm
fib(10) = 55
```

- i32-only interpreter with locals, arithmetic, control flow, memory ops
- In-memory FS with `ls`, `run`, `info`
- Entry point discovered via export section

---

# Sprint A: WASM Spec Completeness

**Depends on:** MVP (Sprints 1–4)

## Goal

Bring the interpreter to near-full WASM MVP spec so modules produced by
real toolchains (`wasm-pack`, `emscripten`, `rustc --target wasm32-unknown-unknown`)
can execute without hitting unsupported opcode errors.

---

## Tasks

### 1. i64 Type Support

* [x] Extend value stack to `i64` (untyped i64 stack — i32 ops sign-extend results)
* [x] `i64.const`, `i64.add`, `i64.sub`, `i64.mul`
* [x] `i64.and`, `i64.or`, `i64.xor`, `i64.shl`, `i64.shr_s`, `i64.shr_u`
* [x] `i64.eq`, `i64.ne`, `i64.lt_s`, `i64.gt_s`, `i64.eqz`
* [x] `i64.load`, `i64.store`
* [x] `i32.wrap_i64`, `i64.extend_i32_s`, `i64.extend_i32_u`

---

### 2. f32 / f64 Type Support

> ⚠️ No FPU in kernel mode by default — requires explicit SSE/x87 state save or soft-float.
> Soft-float (pure Rust) is simpler but slower. SSE needs `CR0.EM` cleared and `FXSAVE` in context switch.

* [ ] Decide: soft-float vs SSE (recommend soft-float for correctness first)
* [ ] `f32.const`, `f64.const`
* [ ] `f32.add/sub/mul/div`, `f64.add/sub/mul/div`
* [ ] `f32.eq/ne/lt/gt/le/ge`, same for f64
* [ ] `f32.load`, `f32.store`, `f64.load`, `f64.store`
* [ ] `i32.trunc_f32_s`, `f32.convert_i32_s`, and related conversions

---

### 3. Multi-Value Returns

* [x] Update type section parser to handle multiple return types
* [x] Update call/return logic to push/pop multiple values
* [x] Update `engine::run()` to return `Vec`-equivalent (fixed-size array) of results

---

### 4. Global Variables

* [x] Parse global section (section ID 6)
* [x] Store globals in `Interpreter` (fixed-size array)
* [x] `global.get`, `global.set`
* [x] Mutable vs immutable globals (validate on set)

---

### 5. Table + `call_indirect`

* [x] Parse table section (section ID 4)
* [x] Parse element section (section ID 9) — populates function table
* [x] `call_indirect <type_idx> <table_idx>` — runtime type check, dispatch
* [x] Trap on null table entry or type mismatch → `InterpError::IndirectCallTypeMismatch`

---

### 6. Missing Opcodes

* [x] `memory.size`, `memory.grow` (grow: stub returning -1 is acceptable for now)
* [x] `i32.rem_s`, `i32.rem_u`, `i32.div_s`, `i32.div_u`
* [x] `i32.rotl`, `i32.rotr`, `i32.clz`, `i32.ctz`, `i32.popcnt`
* [x] `i32.shr_u`
* [x] `br_table`
* [x] `select` (already in MVP plan, confirm complete)

---

## Done When ✅

A non-trivial module compiled with `rustc --target wasm32-unknown-unknown` (no std, no alloc)
runs to completion:

```
> run primes.wasm
Primes up to 50: 2 3 5 7 11 13 17 19 23 29 31 37 41 43 47
```

---

# Sprint B: Runtime Isolation + Multi-Instance

**Depends on:** Sprint A (type completeness avoids surprises mid-isolation work)

## Goal

Run multiple WASM modules without them sharing memory or state.
Each `run` call gets a clean, isolated instance.

---

## Tasks

### 1. Module Instance Type

* [x] Define `Instance` struct: owns `mem: [u8; MEM_SIZE]`, `globals`, `host_fn`
* [x] `engine::instantiate(bytes, entry) -> Result<Instance, RunError>`
* [x] `engine::call(instance, entry) -> Result<..., RunError>`
* [x] Remove global mutable state from `Interpreter` — all state lives in `Instance`

---

### 2. Configurable Memory Size

* [ ] Read memory section `min` pages (1 page = 64 KiB)
* [ ] Allocate `min * 65536` bytes for instance memory (static pool — no heap)
* [ ] Cap at a kernel-configured max (e.g. 4 pages = 256 KiB)
* [ ] Return `RunError::MemoryTooLarge` if request exceeds cap

---

### 3. Instance Pool

* [ ] Fixed-size pool: `[Option<Instance>; MAX_INSTANCES]`
* [ ] `spawn(name)` → allocate slot, instantiate module, return handle
* [ ] `destroy(handle)` → zero memory, free slot
* [ ] `ps` shell command — list running instances with name + memory usage

---

### 4. Host Function Registry

* [ ] Replace single `host_fn` pointer with a small dispatch table
* [ ] `register_host(module, name, fn_ptr)` at boot
* [ ] Instances resolve imports against registry at instantiation time
* [ ] `ImportNotFound` error if module requests an unregistered host function

---

## Done When

```
> run hello.wasm
Hello from WASM!

> run hello.wasm
Hello from WASM!

> ps
(no instances — both completed and freed)
```

Two sequential runs produce identical output with no state leak between them.

---

# Sprint C: Preemptive Scheduling

**Depends on:** Sprint B (isolated instances are the schedulable unit)

## Goal

WASM instances run as cooperative or preemptive tasks.
Long-running modules don't freeze the terminal.

---

## Tasks

### 1. Hardware Timer

* [ ] Configure PIT (8253) or APIC timer for periodic interrupt (e.g. 10ms)
* [ ] Timer ISR increments a tick counter
* [ ] Verify tick counter increments (print in `info` command)

---

### 2. Task / Fiber Abstraction

* [ ] `Task` struct: instance handle + saved execution state (interpreter PC + stack snapshot)
* [ ] Fixed-size task queue: `[Option<Task>; MAX_TASKS]`
* [ ] `task_spawn(name)` → add to queue, return task ID
* [ ] `task_kill(id)` → mark for removal

---

### 3. Round-Robin Scheduler

* [ ] On timer interrupt: save current task state, advance to next ready task
* [ ] Resume task from saved state
* [ ] Idle task when queue is empty (halts with `hlt`)

---

### 4. Yield Host Function

* [ ] `host_yield()` — WASM calls this to voluntarily give up the CPU
* [ ] Register as `"env"."yield"` in host function registry
* [ ] `sleep_ms(ms: i32)` — yield for at least N ticks

---

### 5. Shell as a Task

* [ ] Move `keyboard::run_loop()` into its own task
* [ ] Shell and WASM instances share the CPU via the scheduler

---

## Done When

```
> task-run counter.wasm
> task-run hello.wasm
counter: 1
Hello from WASM!
counter: 2
counter: 3
...
```

Two tasks interleave output without either blocking the other.

---

# Sprint D: Persistent Filesystem

**Depends on:** Sprint B (instance isolation) — can be parallelized with Sprint C

## Goal

Files survive across reboots. Modules can be stored, updated, and deleted without
rebuilding the kernel.

---

## Tasks

### 1. Block Device Abstraction

* [ ] `BlockDevice` trait: `read_block(lba, buf)`, `write_block(lba, buf)`
* [ ] Ramdisk implementation: embedded `static mut` byte array, block-addressed
* [ ] QEMU virtio-blk driver (stretch — enables true disk persistence)

---

### 2. Flat Filesystem (WasmFS)

* [ ] Custom simple format: fixed-size directory entries + data region
* [ ] Directory entry: `name[32]`, `start_block: u32`, `size: u32`, `flags: u8`
* [ ] `fs_open(name)`, `fs_read(fd, buf, len)`, `fs_write(fd, buf, len)`, `fs_close(fd)`
* [ ] `fs_unlink(name)`, `fs_list()` iterator

---

### 3. Boot Image Tool

* [ ] Host-side tool (`tools/pack-fs.sh`) packs files into a filesystem image
* [ ] QEMU loads image as a second drive (`-drive file=fs.img`)
* [ ] Kernel mounts at boot: reads directory, registers files

---

### 4. Shell Commands

* [ ] `ls` — reads from persistent FS (replaces in-memory table)
* [ ] `rm <name>` — delete file
* [ ] `write <name> <hex-bytes>` — write raw bytes (for testing)

---

### 5. Persist In-Memory FS to Disk

* [ ] On `save` or periodic flush: sync in-memory file table back to block device
* [ ] On boot: load directory from block device into in-memory table

---

## Done When

1. Pack `hello.wasm` into `fs.img` with `tools/pack-fs.sh`
2. Boot QEMU with `fs.img`
3. `ls` shows `hello.wasm` without it being embedded in the kernel binary
4. Reboot — file still there

---

# Sprint E: Networking

**Depends on:** Sprint C (scheduling — async I/O needs cooperative yield), Sprint D (sockets as files)

## Goal

A WASM module can open a TCP connection and send/receive data.
The kernel is not a web server — WASM modules are.

---

## Tasks

### 1. virtio-net Driver

* [ ] Detect virtio-net device via PCI enumeration
* [ ] Initialize virtqueue (descriptor ring, available ring, used ring)
* [ ] `net_send(buf, len)`, `net_recv(buf, len)` — raw Ethernet frames

---

### 2. TCP/IP Stack

* [ ] Integrate `smoltcp` (no_std compatible) or implement minimal ARP + IP + TCP
* [ ] DHCP client or static IP config
* [ ] `tcp_connect(ip, port)`, `tcp_listen(port)`, `tcp_send`, `tcp_recv`, `tcp_close`

---

### 3. Socket Host Functions

* [ ] Register `"net"."connect"`, `"net"."listen"`, `"net"."send"`, `"net"."recv"`, `"net"."close"`
* [ ] Non-blocking recv returns 0 immediately if no data (caller should yield + retry)
* [ ] Socket handles as `i32` (index into a fixed socket table)

---

### 4. Demo Module

* [ ] `httpd.wasm` — minimal HTTP/1.0 server in WAT/WASM
* [ ] Responds to `GET /` with `Hello from WASM-First OS!`

---

## Done When

```
> run httpd.wasm
Listening on 0.0.0.0:8080
```

```bash
# from host
curl http://localhost:8080/
Hello from WASM-First OS!
```

---

# Sprint F: JIT Compilation

**Depends on:** Sprint A (spec completeness — JIT must handle same opcodes as interpreter)

## Goal

Compile WASM functions to native x86_64 at instantiation time.
Interpreter remains as fallback for unsupported patterns.

---

## Tasks

### 1. Code Generation Infrastructure

* [ ] Emit x86_64 machine code into a fixed executable buffer (`static mut`)
* [ ] Mark buffer executable (`NX` bit management, or identity-map with exec permissions)
* [ ] Function prologue/epilogue (save callee-saved regs, set up frame)

---

### 2. Basic Code Generation

* [ ] `i32.const N` → `mov eax, N; push rax`
* [ ] `i32.add` → `pop rbx; pop rax; add rax, rbx; push rax`
* [ ] `call <idx>` → `call <compiled_fn_ptr>` or interpreter fallback
* [ ] `local.get/set` → `mov` to/from stack frame slot
* [ ] `if/block/loop/br` → conditional jumps with backpatching

---

### 3. Tiered Execution

* [ ] First call: interpret + count invocations
* [ ] After N calls: JIT compile the function, replace dispatch pointer
* [ ] Hot functions run native; cold/complex functions fall back to interpreter

---

### 4. Benchmark

* [ ] Compare interpreter vs JIT on `fib(35)` — should see 10–50x speedup

---

## Done When

```
> run fib-jit.wasm
fib(35) = 9227465  [interpreter: 4200ms, jit: 80ms]
```

---

# Sprint G: WAT Parser (In-OS Assembler)

**Depends on:** Sprint D (persistent FS to store `.wat` files)

## Goal

Write WebAssembly Text Format inside the OS, assemble it to binary, and run it.
No host toolchain required.

---

## Tasks

### 1. WAT Tokenizer

* [ ] Token types: `(`, `)`, keyword, string literal, integer, float, identifier (`$name`)
* [ ] Fixed-size token buffer (no heap)
* [ ] Handles `;;` line comments

---

### 2. WAT Parser → WASM Binary Emitter

* [ ] Parse `(module ...)` structure
* [ ] Emit type section from `(func (param ...) (result ...))`
* [ ] Emit import section from `(import ...)`
* [ ] Emit function / code sections from `(func ...)`
* [ ] Emit memory section from `(memory ...)`
* [ ] Emit data section from `(data ...)`
* [ ] Emit export section from `(export ...)`
* [ ] Write output to FS as `.wasm`

---

### 3. Shell Integration

* [ ] `edit <file.wat>` — line-append editor with `save`/`quit`
* [ ] `asm <file.wat>` — assemble to `<file.wasm>`
* [ ] Full round-trip: edit → asm → run

---

## Done When

Type this into the OS terminal, with no host tools involved:

```
> edit add.wat
(module
  (import "env" "print" (func (param i32 i32)))
  (memory 1)
  (data (i32.const 0) "2 + 3 = 5\n")
  (func (export "main")
    i32.const 0
    i32.const 10
    call 0)
)
> asm add.wat
> run add.wasm
2 + 3 = 5
```

---

# Dependency Graph

```
MVP (Sprints 1-4)
    └── Sprint A: WASM Spec Completeness
            ├── Sprint B: Runtime Isolation
            │       └── Sprint C: Preemptive Scheduling
            │               └── Sprint E: Networking ──┐
            ├── Sprint D: Persistent FS ───────────────┘
            │       └── Sprint G: WAT Parser
            └── Sprint F: JIT Compilation
```

---

# Principles (unchanged from MVP)

> **Always keep the system bootable.**

Never break:
* boot
* terminal
* the `run` command

Everything else is incremental.
