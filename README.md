# WASM-First OS

> A bare-metal research OS that runs **WebAssembly as the primary execution environment**

---

## What is this?

**WASM-First OS** is an experimental operating system that replaces the traditional process/syscall model with a WebAssembly runtime:

- **WebAssembly as the system ABI** — not POSIX, not native binaries
- **VM-based isolation** — sandboxing via the WASM memory model
- **Host functions** instead of syscalls
- **Minimal kernel** — boot, terminal, and a WASM interpreter

> What if the OS *was* a WebAssembly runtime?

---

## Current Status — MVP Complete

The MVP (Sprints 1–4) is done. The system boots, accepts keyboard input, and executes real WASM modules compiled with `wat2wasm`.

```
Hello from WASM!
Type 'help' for commands.
> ls
hello.wasm
greet.wasm
fib.wasm
> run fib.wasm 10
55
> run fib.wasm 20
6765
> info fib.wasm
file:    fib.wasm
funcs:   2 defined, 2 imported
exports: 1
> run greet.wasm
Greetings from the second module!
```

See [Post-MVP roadmap](Post_MVP_Agile_plan.md) for what's next.

---

## Build & Run

### Requirements

- Rust with `x86_64-unknown-none` target
- `wabt` (for `wat2wasm`)
- QEMU

```bash
# Install Rust target
rustup target add x86_64-unknown-none

# Install wabt (Ubuntu/Debian)
sudo apt install wabt

# Full pipeline: compile userland → build kernel → launch QEMU
./tools/run-qemu.sh

# Or step by step:
./tools/wasm-pack.sh      # compile userland/*.wat → *.wasm
./tools/build-image.sh    # build kernel + disk image
./tools/run-qemu.sh       # boot in QEMU
```

---

## Shell Commands

| Command | Description |
|---|---|
| `help` | List commands |
| `ls` | List registered `.wasm` modules |
| `run <name> [args...]` | Execute a module, passing integer args to `main` |
| `info <name>` | Show module section/function counts |
| `echo <text>` | Print text |
| `history` | Show command history |
| `clear` | Clear the screen |

---

## Writing a WASM Module

Modules import host functions and export `main`:

```wat
(module
  (import "env" "print"     (func $print     (param i32 i32)))
  (import "env" "print_int" (func $print_int (param i32)))
  (memory 1)
  (data (i32.const 0) "Hello!\n")

  (func (export "main")
    i32.const 0   ;; ptr
    i32.const 7   ;; len
    call $print
  )
)
```

Compile with `wat2wasm`, place under `userland/`, run `tools/wasm-pack.sh`.

### Host Functions

| Import | Signature | Description |
|---|---|---|
| `"env"."print"` | `(param i32 i32)` | Print UTF-8 string from linear memory (ptr, len) |
| `"env"."print_int"` | `(param i32)` | Print i32 as decimal + newline |

---

## Architecture

```
+-----------------------------+
|     WASM Modules (.wasm)    |  ← userland/
+-----------------------------+
|  WASM Interpreter           |  ← kernel/src/wasm/
|  - loader, engine, interp   |
|  - host function dispatch   |
+-----------------------------+
|  Shell + FS                 |  ← kernel/src/shell.rs, fs.rs
|  - commands, file registry  |
+-----------------------------+
|  Kernel (no_std Rust)       |  ← kernel/src/
|  - framebuffer, keyboard    |
|  - interrupts, boot         |
+-----------------------------+
|  x86_64 bare metal / QEMU   |
+-----------------------------+
```

---

## Supported WASM Opcodes (MVP)

- **Control:** `block`, `loop`, `if/else/end`, `br`, `br_if`, `return`, `nop`, `unreachable`, `call`
- **Locals:** `local.get`, `local.set`, `local.tee`
- **i32 arithmetic:** `add`, `sub`, `mul`, `and`, `or`, `xor`, `shl`, `shr_s`
- **i32 comparison:** `eq`, `ne`, `lt_s`, `gt_s`, `le_s`, `ge_s`, `eqz`
- **Memory:** `i32.load`, `i32.store`, `i32.load8_u`, `i32.store8`
- **Stack:** `drop`, `select`, `i32.const`

---

## Roadmap

Post-MVP work is planned in sprints — see [Post_MVP_Agile_plan.md](Post_MVP_Agile_plan.md).

| Sprint | Focus |
|---|---|
| A | WASM spec completeness (i64, f32/f64, globals, `call_indirect`) |
| B | Runtime isolation — per-instance memory, instance pool |
| C | Preemptive scheduling — timer, task queue, round-robin |
| D | Persistent filesystem — block device, WasmFS, boot image |
| E | Networking — virtio-net, TCP/IP, socket host functions |
| F | JIT compilation — x86_64 codegen, tiered execution |
| G | In-OS WAT assembler — edit, assemble, and run without host tools |

---

## Philosophy

> **Working systems > perfect designs**

Always keep the system bootable. Every sprint produces something runnable.

---

## License

TBD
