# WASM-First OS

> A bare-metal research OS that runs **WebAssembly as the primary execution environment**

---

## What is this?

**WASM-First OS** is an experimental operating system that replaces the traditional process/syscall model with a WebAssembly runtime:

- **WebAssembly as the system ABI** — not POSIX, not native binaries
- **VM-based isolation** — sandboxing via the WASM memory model
- **Host functions** instead of syscalls
- **Persistent FAT filesystem** — virtio-blk disk or embedded ramdisk fallback
- **Cooperative scheduling** — round-robin task queue with PIT-driven sleep wakeup

> What if the OS *was* a WebAssembly runtime?

---

## Current Status

Sprints 1–4 (MVP) and A–E, G (runtime completeness, isolation, scheduling, persistent FS, networking, in-OS WAT assembler) are complete.

The system boots, acquires an IP address via DHCP, accepts keyboard/serial input, mounts a FAT filesystem, executes real WASM modules, runs multiple modules concurrently as cooperative tasks, and serves HTTP over TCP from a WASM module.

```
Type 'help' for commands.
> ls
  hello.wasm               1234 bytes
  fib.wasm                  567 bytes
  httpd.wasm                890 bytes
> run fib.wasm 10
55
> df
Filesystem    Size (K)  Used (K)  Avail (K)
FAT              32768        16     32752
> task-run httpd.wasm
task 0 spawned: httpd.wasm
[httpd] listening on :8080
```

```bash
# from the host
curl http://localhost:8080/
Hello from WASM-First OS!
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
| `asm <name>` | Assemble a tiny instruction sequence into a WASM module |
| `save` | Flush in-memory file table to the FAT volume |
| `info [name]` | Show module section info, or tick count if no name |
| `run <name> [args...]` | Execute a `.wasm` module synchronously |
| `ps` | List active WASM instance pool slots |
| `task-run <name> [args...]` | Spawn a module as a background task |
| `<name> [args...]` | Auto-spawn: if `<name>.wasm` exists, equivalent to `task-run <name>.wasm [args...]` |
| `task-kill <id>` | Kill a task by ID |
| `tasks` | List all tasks with state |

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
| `"env"."fb_set_pixel"` | `(param i32 i32 i32)` | Write one pixel to the framebuffer (x, y, rgb as 0x00RRGGBB) |
| `"env"."fb_present"` | `()` | Present the framebuffer (no-op; reserved for future double-buffering) |
| `"env"."fb_blit"` | `(param i32 i32 i32)` | Blit a packed 0x00RRGGBB pixel buffer from WASM memory to the framebuffer (ptr, width, height) |

**Network host functions (registered under `"net"`):**

| Import | Signature | Description |
|---|---|---|
| `"net"."listen"` | `(param i32) → i32` | TCP listen on port; returns listen-socket handle or -1 |
| `"net"."connect"` | `(param i32 i32) → i32` | TCP active connect (ip_u32_le, port); returns handle or -1 |
| `"net"."accept"` | `(param i32) → i32` | Accept pending connection (non-blocking); returns conn handle or -1 if none ready |
| `"net"."recv"` | `(param i32 i32 i32) → i32` | Receive into memory (handle, ptr, cap); returns byte count, 0 if no data yet, -1 on error |
| `"net"."send"` | `(param i32 i32 i32) → i32` | Send from memory (handle, ptr, len); returns bytes sent or -1 |
| `"net"."close"` | `(param i32) → i32` | Close a TCP connection; always returns 0 |
| `"net"."status"` | `(param i32) → i32` | Socket state: 0=closed, 1=listen, 2=handshaking, 3=established, 4=teardown |
| `"net"."get_ip"` | `() → i32` | Kernel IP address as u32 little-endian (0 if DHCP not yet bound) |
| `"net"."set_ip"` | `(param i32) → i32` | Manually set the kernel IP (ip_u32_le); always returns 0 |
| `"net"."udp_bind"` | `(param i32) → i32` | Bind UDP socket to port; returns handle or -1 |
| `"net"."udp_connect"` | `(param i32 i32 i32) → i32` | Set UDP remote (handle, ip_u32_le, port); returns 0 or -1 |
| `"net"."udp_send"` | `(param i32 i32 i32) → i32` | Send UDP datagram (handle, ptr, len); returns bytes sent or -1 |
| `"net"."udp_recv"` | `(param i32 i32 i32) → i32` | Receive UDP datagram (handle, ptr, cap); returns byte count or 0 if no data (non-blocking) |
| `"net"."udp_close"` | `(param i32) → i32` | Close UDP socket; always returns 0 |

---

## Architecture

```
+-----------------------------+
|     WASM Modules (.wasm)    |  ← userland/
+-----------------------------+
|  WASM Interpreter           |  ← kernel/src/wasm/
|  - loader, engine, interp   |
|  - host function registry   |
|  - cooperative task layer   |
+-----------------------------+
|  Shell + FAT Filesystem     |  ← kernel/src/shell/, kernel/src/fs/
|  - commands, CWD tracking   |
|  - virtio-blk + ramdisk     |
+-----------------------------+
|  TCP/IP Network Stack       |  ← kernel/src/drivers/netstack/
|  - virtio-net PCI driver    |
|  - ARP, IP, TCP, UDP, DHCP  |
|  - socket host functions    |
+-----------------------------+
|  Kernel (no_std Rust)       |  ← kernel/src/
|  - framebuffer, serial      |
|  - PS/2 keyboard, PIT timer |
|  - interrupts, page tables  |
+-----------------------------+
|  x86_64 bare metal / QEMU   |
+-----------------------------+
```

For full details see [docs/architecture.md](docs/architecture.md) and [docs/wasm-runtime.md](docs/wasm-runtime.md).

---

## Supported WASM Opcodes

- **Control:** `block` `loop` `if/else/end` `br` `br_if` `br_table` `return` `nop` `unreachable`
- **Calls:** `call` `call_indirect`
- **Stack:** `drop` `select`
- **Locals/Globals:** `local.get/set/tee` `global.get/set`
- **Memory:** full load/store suite for i32, i64, f32, f64 (including narrow widths); `memory.size`; `memory.grow`
- **i32:** full arithmetic, bitwise, comparison, and sign-extension suite
- **i64:** full arithmetic, bitwise, comparison, and sign-extension suite
- **f32/f64:** full arithmetic, comparison, and conversion suite (via `libm`)
- **Conversions:** wrap, extend, trunc, convert, demote, promote, reinterpret; saturating trunc (`0xFC` prefix)

See [docs/wasm-runtime.md](docs/wasm-runtime.md) for the complete opcode table.

---

## Roadmap

| Sprint | Focus | Status |
|---|---|---|
| 1–4 | MVP: boot, framebuffer, keyboard, WASM interpreter, shell, in-memory FS | Done |
| A | WASM spec completeness: i64, globals, `call_indirect`, `br_table`, f32/f64 | Done |
| B | Runtime isolation: instance pool, named host registry, `ps` | Done |
| C | Cooperative scheduling: PIT timer, task queue, `task-run`/`task-kill`/`tasks` | Done |
| D | Persistent filesystem: virtio-blk, FAT12/16/32, shell FS commands | Done |
| E | Networking: virtio-net, TCP/IP stack, socket host functions, `httpd.wasm` | Done |
| F | JIT compilation: x86_64 codegen, tiered execution | Planned |
| G | In-OS WAT assembler: edit, assemble, and run without host tools | Done |

---

## Philosophy

> **Working systems > perfect designs**

Always keep the system bootable. Every sprint produces something runnable.

---

## License

TBD
