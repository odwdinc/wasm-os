# Roadmap

## MVP — Complete (Sprints 1–4)

The MVP is done. The system boots, runs a shell, and executes real WASM modules.

| Sprint | Focus | Status |
|---|---|---|
| 1 | Bootable kernel, framebuffer, keyboard | Done |
| 2 | WASM loader, basic interpreter, `hello.wasm` | Done |
| 3 | In-memory FS, export parsing, shell commands (`ls`, `info`, `run`) | Done |
| 4 | Full control flow, locals, memory ops, i32 arithmetic/comparison | Done |

Exit condition met:
```
> run fib.wasm 10
55
> run fib.wasm 20
6765
```

---

## Sprint A — WASM Spec Completeness

Bring the interpreter close enough to the WASM MVP spec that modules produced by
real toolchains (`rustc --target wasm32-unknown-unknown`, `emscripten`) execute
without hitting unsupported-opcode errors.

**Tasks:**
- `i64` type support (tagged value stack)
- `f32`/`f64` (soft-float — no SSE in kernel mode)
- Multi-value returns
- Global variables (section ID 6)
- Table section + `call_indirect`
- Missing opcodes: `memory.size`, `memory.grow`, `i32.div_s/u`, `i32.rem_s/u`, `i32.shr_u`, `i32.rotl/rotr/clz/ctz/popcnt`, `br_table`

**Done when:**
```
> run primes.wasm
Primes up to 50: 2 3 5 7 11 13 17 19 23 29 31 37 41 43 47
```

---

## Sprint B — Runtime Isolation

Run multiple WASM modules without shared memory or state. Each `run` gets a
clean, isolated instance.

**Tasks:**
- `Instance` struct owns its own linear memory
- Instance pool (`[Option<Instance>; MAX_INSTANCES]`)
- Named host function registry (replaces dispatch-by-index)
- `ps` shell command

**Done when:** two sequential `run hello.wasm` calls produce identical output with
no state leak.

---

## Sprint C — Preemptive Scheduling

WASM instances run as tasks. Long-running modules don't freeze the terminal.

**Tasks:**
- PIT/APIC timer interrupt (10ms tick)
- `Task` struct + fixed-size task queue
- Round-robin scheduler
- `host_yield()` / `sleep_ms()` host functions
- Shell as a task

---

## Sprint D — Persistent Filesystem

Files survive reboots. Modules stored, updated, deleted without rebuilding the
kernel.

**Tasks:**
- Block device abstraction (ramdisk + optional virtio-blk)
- Flat "WasmFS" format: fixed-size directory entries + data region
- Host-side `tools/pack-fs.sh` to build a filesystem image
- `ls`, `rm`, `write` against persistent FS

---

## Sprint E — Networking

A WASM module opens TCP connections and sends/receives data.

**Tasks:**
- virtio-net driver
- TCP/IP stack (smoltcp or minimal hand-written)
- Socket host functions (`"net"."connect"`, `"net"."send"`, etc.)
- `httpd.wasm` demo module

---

## Sprint F — JIT Compilation

Compile WASM functions to native x86_64 at instantiation time.

**Tasks:**
- Machine code emitter into a static executable buffer
- Basic codegen for arithmetic, locals, control flow
- Tiered execution: interpret → JIT after N calls

**Done when:** `fib(35)` runs measurably faster under JIT.

---

## Sprint G — In-OS WAT Assembler

Write WAT source inside the OS, assemble it to binary, and run it — no host
toolchain required.

**Tasks:**
- WAT tokenizer + parser
- WASM binary emitter
- `edit <file.wat>` / `asm <file.wat>` shell commands

---

## Dependency Graph

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
