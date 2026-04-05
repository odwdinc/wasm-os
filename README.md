# WASM-First OS

> A bare-metal research OS that runs **WebAssembly as the primary execution environment**

---

## What is this?

**WASM-First OS** is an experimental operating system that replaces the traditional process/syscall model with a WebAssembly runtime:

- **WebAssembly as the system ABI** — not POSIX, not native binaries
- **VM-based isolation** — sandboxing via the WASM memory model
- **Host functions** instead of syscalls
- **Persistent FAT filesystem** — virtio-blk disk or embedded ramdisk fallback
- **Preemptive scheduling** — timer-driven task queue with round-robin

> What if the OS *was* a WebAssembly runtime?

---

## Current Status

The system boots, accepts keyboard input, mounts a FAT filesystem (virtio-blk or embedded fallback), and executes real WASM modules. A preemptive scheduler runs WASM instances as tasks.

```
Type 'help' for commands.
> ls
  hello.wasm               1234 bytes
  fib.wasm                  567 bytes
> run fib.wasm 10
55
> run fib.wasm 20
6765
> info fib.wasm
file:    fib.wasm
funcs:   2 defined, 2 imported
exports: 1
> df
Filesystem    Size (K)  Used (K)  Avail (K)
FAT              32768        12     32756
> task-run hello.wasm
task 0 spawned: hello.wasm
> tasks
[0] hello.wasm  (done)
```

---

## Build & Run

### Requirements

- Rust with `x86_64-unknown-none` target
- `wabt` (for `wat2wasm`)
- QEMU (`qemu-system-x86_64`)
- `dosfstools` + `mtools` (for `mkfs.fat` / `mcopy`)

```bash
# Install Rust target
rustup target add x86_64-unknown-none

# Ubuntu/Debian dependencies
sudo apt install wabt qemu-system-x86 dosfstools mtools

# Full pipeline: compile userland → build kernel → launch QEMU
./tools/run-qemu.sh

# Build profiles
./tools/run-qemu.sh           # debug build, VGA window
./tools/run-qemu.sh release   # release build
./tools/run-qemu.sh headless  # debug, no VGA (serial only — good for CI)

# Step by step
./tools/wasm-pack.sh          # compile userland/*.wat → *.wasm
./tools/build-image.sh        # build kernel + disk images
./tools/run-qemu.sh           # boot in QEMU
```

### Disk Images

Two images are produced by `tools/pack-fs.sh`:

| Image | Size | Purpose |
|---|---|---|
| `fs.img` | Fixed 2 MiB FAT12 | Embedded fallback — baked into the kernel binary |
| `disk.img` | Configurable (default 32 MiB) | Mounted as virtio-blk at boot |

```bash
# Custom disk size
./tools/pack-fs.sh --disk-size 128M userland/*.wasm
```

The kernel always tries to mount `disk.img` via virtio-blk first. If that fails it falls back to the embedded `fs.img`.

---

## Shell Commands

| Command | Description |
|---|---|
| `help` | List commands |
| `echo <args>` | Print arguments |
| `history` | Show command history |
| `clear` | Clear the screen |
| `ls` | List files in the current directory |
| `cat <file>` | Print file contents |
| `cd <dir>` | Change directory |
| `mkdir <dir>` | Create a directory |
| `df` | Show filesystem space usage |
| `rm <name>` | Remove a file |
| `write <name> <hex>` | Write raw bytes (hex-encoded) as a file |
| `edit <name>` | Line-append editor (`:w` = save, `:q` = quit) |
| `save` | Flush in-memory file table to the FAT volume |
| `info [name]` | Show module info, or tick count if no name |
| `run <name> [args...]` | Execute a `.wasm` module |
| `ps` | List running WASM instances |
| `task-run <name>` | Spawn a module as a background task |
| `task-kill <id>` | Kill a task by ID |
| `tasks` | List all tasks |

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
| `"env"."print_i64"` | `(param i64)` | Print i64 as decimal + newline |
| `"env"."print_char"` | `(param i32)` | Print low byte as a single ASCII character |
| `"env"."print_hex"` | `(param i32)` | Print i32 as `0x` + 8 uppercase hex digits + newline |
| `"env"."yield"` | `()` | Yield to the scheduler (cooperative multitasking) |
| `"env"."sleep_ms"` | `(param i32)` | Yield for at least N milliseconds |
| `"env"."uptime_ms"` | `() → i32` | Milliseconds since boot |
| `"env"."exit"` | `(param i32)` | Terminate the module cleanly |
| `"env"."read_char"` | `() → i32` | Block until a key is pressed; returns ASCII code (Enter = 10) |
| `"env"."read_line"` | `(param i32 i32) → i32` | Read a line into memory (ptr, cap); returns byte count or -1 |
| `"env"."args_get"` | `(param i32 i32) → i32` | Copy run-time args string into memory (ptr, cap); returns byte count or -1 |
| `"env"."fs_read"` | `(param i32 i32 i32 i32) → i32` | Read file into memory (name_ptr, name_len, buf_ptr, buf_cap); returns byte count or -1 |
| `"env"."fs_write"` | `(param i32 i32 i32 i32) → i32` | Write bytes from memory to a file (name_ptr, name_len, buf_ptr, buf_len); returns 0 or -1 |
| `"env"."fs_size"` | `(param i32 i32) → i32` | Return file size in bytes (name_ptr, name_len), or -1 if not found |

---

## Architecture

```
+-----------------------------+
|     WASM Modules (.wasm)    |  ← userland/
+-----------------------------+
|  WASM Interpreter           |  ← kernel/src/wasm/
|  - loader, engine, interp   |
|  - host function dispatch   |
|  - task scheduler           |
+-----------------------------+
|  Shell + FAT Filesystem     |  ← kernel/src/shell/, kernel/src/fs/
|  - commands, CWD tracking   |
|  - virtio-blk + ramdisk     |
+-----------------------------+
|  Kernel (no_std Rust)       |  ← kernel/src/
|  - framebuffer, keyboard    |
|  - interrupts, PIT timer    |
+-----------------------------+
|  x86_64 bare metal / QEMU   |
+-----------------------------+
```

---

## Supported WASM Opcodes

- **Control:** `block` `loop` `if/else/end` `br` `br_if` `br_table` `return` `nop` `unreachable`
- **Calls:** `call` `call_indirect`
- **Stack:** `drop` `select`
- **Locals:** `local.get` `local.set` `local.tee`
- **Globals:** `global.get` `global.set`
- **Memory:** `i32.load` `i64.load` `i32.load8_u` `i32.store` `i64.store` `i32.store8` `memory.size` `memory.grow`
- **i32:** full arithmetic, bitwise, and comparison suite
- **i64:** arithmetic, bitwise, and comparison suite
- **Conversions:** `i32.wrap_i64` `i64.extend_i32_s` `i64.extend_i32_u`

See [docs/wasm-runtime.md](docs/wasm-runtime.md) for the complete opcode table.

---

## Roadmap

| Sprint | Focus | Status |
|---|---|---|
| 1–4 | MVP: boot, framebuffer, keyboard, WASM interpreter, shell, in-memory FS | Done |
| A | WASM spec completeness (i64, globals, `call_indirect`, `br_table`) | Done |
| B | Runtime isolation — instance pool, named host registry, `ps` | Done |
| C | Preemptive scheduling — PIT timer, task queue, `task-run`/`task-kill` | Done |
| D | Persistent filesystem — virtio-blk, FAT12/16/32, shell FS commands | Done |
| E | Networking — virtio-net, TCP/IP, socket host functions | Planned |
| F | JIT compilation — x86_64 codegen, tiered execution | Planned |
| G | In-OS WAT assembler — edit, assemble, and run without host tools | Planned |

---

## Philosophy

> **Working systems > perfect designs**

Always keep the system bootable. Every sprint produces something runnable.

---

## License

TBD
