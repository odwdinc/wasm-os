# AGENTS.md — WASM-First OS

> Bare-metal Rust kernel. WebAssembly as the system ABI. Sprints 1–4 + A–G complete.

---

## Project Status

Sprints 1–4 (MVP) and A–E, G (runtime completeness, isolation, scheduling, persistent FS, networking, in-OS WAT assembler) are complete.
Sprint F (JIT compilation) is in progress — detailed plan in [`JIT_Agile_plan.md`](JIT_Agile_plan.md). Target: NES emulator from 1200ms/frame → 16ms/frame (~75x speedup).

---

## Actual Source Layout

```
/
├── Cargo.toml                   # Workspace root (kernel + runner)
├── rust-toolchain.toml          # Pinned nightly toolchain
├── README.md
├── AGENTS.md                    # This file
├── CONTRIBUTING.md
├── MVP_Agile_plan.md            # Sprints 1–4 (complete)
├── Post_MVP_Agile_plan.md       # Sprints A–G
├── JIT_Agile_plan.md            # Sprints 0–6
│
├── kernel/                      # The entire working system lives here
│   ├── build.rs                 # Passes kernel stack size to linker
│   └── src/
│       ├── main.rs              # Entry point, boot sequence, macro definitions
│       ├── vga.rs               # Framebuffer writer, 8×8 font, scrolling
│       ├── scheduler.rs         # Round-robin cooperative scheduler (run loop)
│       ├── drivers/
│       │   ├── keyboard.rs      # PS/2 scancode decoder, try_next_key / next_key
│       │   ├── serial.rs        # 16550 UART, COM1 115200 8N1
│       │   ├── pit.rs           # 8253 PIT + 8259 PIC; ~100 Hz tick counter
│       │   ├── virtio_blk.rs    # Virtio 1.0 block device, DMA + page-table walk
│       │   ├── virtio_net.rs    # Virtio legacy NIC; virtqueue TX/RX, raw Ethernet frames
│       │   └── netstack/        # TCP/IP network stack
│       │       ├── mod.rs       # NetStack: ARP cache, TCP/UDP sockets, DHCP, send_ip
│       │       ├── arp.rs       # ARP table, request/reply encoding
│       │       ├── ethernet.rs  # Ethernet II frame parser/builder
│       │       ├── ip.rs        # IPv4 packet parser/builder, ip_chksum
│       │       ├── tcp.rs       # TCP segment parser/builder, TcpSocket state machine
│       │       ├── udp.rs       # UDP datagram parser/builder, UdpSocket
│       │       └── dhcp.rs      # DHCP DISCOVER/OFFER/REQUEST/ACK client
│       ├── interrupts/
│       │   ├── mod.rs           # IDT init
│       │   ├── idt.rs           # IDT structure and loading
│       │   └── handlers.rs      # IRQ handlers (keyboard, PIT)
│       ├── memory/
│       │   ├── mod.rs           # virt_to_phys (page-table walk), init
│       │   └── allocator.rs     # Bump allocator (global heap)
│       ├── fs/
│       │   ├── mod.rs           # In-memory file table, disk/write pools
│       │   ├── block.rs         # BlockDevice trait + static Ramdisk
│       │   ├── fat.rs           # FAT12/16/32 via rust-fatfs, BlockIo adapter
│       │   └── wasmfs.rs        # Legacy reference (not used at boot)
│       ├── shell/
│       │   ├── mod.rs           # Tokenizer, history, CWD, run_command dispatcher
│       │   ├── input.rs         # Non-blocking poll_once + blocking read_line
│       │   └── commands/        # One file per shell command
│       │       ├── asm.rs       # Assemble tiny WAT → WASM in-kernel
│       │       ├── cat.rs       # Print file contents
│       │       ├── cd.rs        # Change CWD
│       │       ├── clear.rs     # Clear screen
│       │       ├── df.rs        # FAT volume stats
│       │       ├── echo.rs      # Print arguments
│       │       ├── edit.rs      # Line-append editor
│       │       ├── help.rs      # List commands
│       │       ├── history.rs   # Show command history
│       │       ├── info.rs      # Module section info / tick count
│       │       ├── ls.rs        # List directory
│       │       ├── mkdir.rs     # Create directory
│       │       ├── ps.rs        # List WASM instance pool
│       │       ├── rm.rs        # Remove file
│       │       ├── run.rs       # Execute WASM synchronously
│       │       ├── save.rs      # Flush in-memory table to FAT
│       │       ├── tasks.rs     # task-run / task-kill / tasks
│       │       └── write.rs     # Write hex bytes as a file
│       └── wasm/
│           ├── mod.rs           # Module re-exports
│           ├── loader.rs        # Zero-copy WASM binary parser → Module<'_>
│           ├── engine.rs        # Instance pool, host registry, spawn/task API
│           ├── interp.rs        # Stack-machine interpreter, all opcodes
│           └── task.rs          # TaskState, task_spawn/kill/step/for_each
│
├── runner/                      # Host-side tool: wraps kernel ELF → BIOS disk image
│   └── src/main.rs
│
├── userland/                    # WASM source modules (.wat / .wasm)
│   ├── README.md
│   ├── hello/hello.wat          # Prints "Hello from WASM!\n"
│   ├── greet/greet.wat          # Prints a greeting string
│   ├── fib/fib.wat              # Recursive Fibonacci
│   ├── primes/primes.wat        # Sieve of Eratosthenes
│   ├── counter/counter.wat      # Counting demo (cooperative yield)
│   ├── collatz/collatz.wat      # Collatz sequence
│   └── httpd/httpd.wat          # Minimal HTTP/1.0 server on :8080
│
├── wasm-test/                   # Integration tests for the WASM interpreter
│   ├── src/lib.rs
│   └── tests/
│       ├── i32_ops.rs
│       └── userland.rs
│
├── tools/
│   ├── wasm-pack.sh             # Step 1: compile userland/*.wat → *.wasm
│   ├── pack-fs.sh               # Step 2: build fs.img, and disk.img form userland/ *.wasm
│   ├── build-image.sh           # Step 3: wasm-pack + cargo build + disk image
│   └── run-qemu.sh              # Step 4: build-image + launch QEMU
│
└── docs/
    ├── architecture.md          # System design: all components, host interface
    └─── wasm-runtime.md         # Interpreter internals, opcode tables, error types
```

---

## What Is Actually Built

### Kernel subsystems

| Module | Key public API |
|---|---|
| `main.rs` | Boot sequence; `print!` / `println!` macros |
| `vga.rs` | `init(buf, info)`, `clear_screen()`, `_print(args)` |
| `scheduler.rs` | `run() -> !` — the main loop; never returns |
| `memory/mod.rs` | `init(phys_mem_offset)`, `virt_to_phys(virt) -> u64` |
| `drivers/keyboard.rs` | `Key` enum, `try_next_key()`, `next_key()` |
| `drivers/serial.rs` | `init()`, `write_byte/str()`, `read_byte()`, `_print(args)` |
| `drivers/pit.rs` | `init()`, `ticks() -> u64`, `pit_on_tick()` |
| `drivers/virtio_blk.rs` | `VirtioBlk::try_init() -> Option<VirtioBlk>` |
| `drivers/virtio_net.rs` | `VirtioNet::try_init() -> Option<VirtioNet>`, `send_frame(buf)`, `recv_frame(buf) -> usize` |
| `drivers/netstack/mod.rs` | `NetStack::new()`, `poll()`, `tcp_listen(port)`, `tcp_accept(handle)`, `tcp_recv(handle, buf)`, `tcp_send(handle, buf)`, `tcp_close(handle)`, `tcp_status(handle)`, `get_ip()`, `udp_bind(port)`, `udp_send(handle, buf)`, `udp_recv(handle, buf)`, `udp_close(handle)` |

### Filesystem (`kernel/src/fs/`)

| Function | Description |
|---|---|
| `fs::init()` | Zero file table at boot |
| `fs::register_file(name, data)` | Insert a `&'static [u8]` into the table |
| `fs::find_file(name)` | Look up by exact name |
| `fs::remove_file(name)` | Remove entry (leaves a `None` hole) |
| `fs::for_each_file(f)` | Iterate all registered files |
| `fs::alloc_write_buf(data)` | Copy into write-pool slot, return `'static` slice |
| `fs::alloc_disk_slot(len)` | Claim a disk-pool slot for boot-loaded files |
| `fs::load_fat_files_to_table()` | Read FAT volume into disk-pool at boot |
| `fs::save_to_fat()` | Flush in-memory table back to FAT |
| `fat::mount_virtio(blk)` | Mount a virtio-blk FAT volume |
| `fat::mount_ramdisk(img)` | Mount an in-memory FAT image |
| `fat::fat_list(cb)` | Enumerate root-directory files |
| `fat::fat_list_path(path, cb)` | Enumerate a directory path (includes is_dir flag) |
| `fat::fat_read_file(name)` | Read root-directory file → `Vec<u8>` |
| `fat::fat_read_path(path)` | Read file at any path → `Vec<u8>` |
| `fat::fat_write_file(name, data)` | Write/overwrite a root-directory file |
| `fat::fat_remove_file(name)` | Delete a file |
| `fat::fat_mkdir(name)` | Create a directory |
| `fat::fat_is_dir(name)` | Check if a path is a directory |
| `fat::fat_disk_stats()` | Return `(total_bytes, free_bytes)` |

### WASM Subsystem (`kernel/src/wasm/`)

| Function | Description |
|---|---|
| `loader::load(bytes)` | Parse header + sections into zero-copy `Module<'_>` |
| `loader::find_export(module, name)` | Find a function export, return absolute index |
| `loader::for_each_func_import(sec, f)` | Iterate function imports |
| `loader::read_memory_min_pages(sec)` | Return `min` page count from memory section |
| `loader::read_u32_leb128(bytes)` | Unsigned 32-bit LEB-128 decode |
| `engine::init_host_fns()` | Register kernel built-ins; call once at boot |
| `engine::register_host(module, name, fn)` | Add a host function to the registry |
| `engine::set_args(args)` | Store argument string for the next `args_get` call |
| `engine::spawn(name, bytes)` | Instantiate module into pool slot |
| `engine::destroy(handle)` | Free pool slot, zero linear memory |
| `engine::for_each_instance(f)` | Iterate active slots: `f(handle, name, mem_pages)` |
| `engine::start_task(handle, entry, args)` | Begin executing `entry`; may yield |
| `engine::resume_task(handle)` | Continue a suspended task |
| `engine::run(bytes, entry, args)` | Convenience: spawn + run to completion + destroy |
| `task::task_spawn(name, bytes, args)` | Instantiate + register as a cooperative task; `args` are forwarded to `main` |
| `task::task_kill(id)` | Remove task and free pool slot |
| `task::task_step(id)` | Advance task one step (start or resume) |
| `task::is_task_runnable(id)` | True if the task can be stepped now |
| `task::for_each_task(f)` | Iterate all task slots: `f(id, name, state)` |

### Host Functions (registered under `"env"`)

| Name | Signature | Behaviour |
|---|---|---|
| `"print"` | `(param i32 i32)` | Print UTF-8 from linear memory (ptr, len) |
| `"print_int"` | `(param i32)` | Print i32 decimal + newline |
| `"print_i64"` | `(param i64)` | Print i64 decimal + newline |
| `"print_char"` | `(param i32)` | Print low byte as ASCII character |
| `"print_hex"` | `(param i32)` | Print i32 as `0x` + 8 hex digits + newline |
| `"yield"` | `()` | Yield to the scheduler |
| `"sleep_ms"` | `(param i32)` | Yield for at least N milliseconds |
| `"uptime_ms"` | `() → i32` | Milliseconds since boot (PIT ticks × 10) |
| `"exit"` | `(param i32)` | Terminate the module cleanly |
| `"read_char"` | `() → i32` | Blocking key read; returns ASCII (Enter = 10) |
| `"read_line"` | `(param i32 i32) → i32` | Read line into memory (ptr, cap); returns byte count or -1 |
| `"args_get"` | `(param i32 i32) → i32` | Copy run args into memory; returns byte count or -1 |
| `"fs_read"` | `(param i32 i32 i32 i32) → i32` | Read file into memory; returns byte count or -1 |
| `"fs_write"` | `(param i32 i32 i32 i32) → i32` | Write bytes to a file; returns 0 or -1 |
| `"fs_size"` | `(param i32 i32) → i32` | Return file size or -1 |
| `"fb_set_pixel"` | `(param i32 i32 i32)` | Write pixel to framebuffer (x, y, 0x00RRGGBB) |
| `"fb_present"` | `()` | Present framebuffer (no-op; reserved for double-buffering) |
| `"fb_blit"` | `(param i32 i32 i32)` | Blit packed pixel buffer to framebuffer (ptr, width, height) |

**Network host functions (registered under `"net"`):**

| Name | Signature | Behaviour |
|---|---|---|
| `"listen"` | `(param i32) → i32` | TCP listen on port; returns listen-socket handle or -1 |
| `"connect"` | `(param i32 i32) → i32` | TCP active connect (ip_u32_le, port); returns handle or -1 |
| `"accept"` | `(param i32) → i32` | Accept pending connection (non-blocking); returns conn handle or -1 |
| `"recv"` | `(param i32 i32 i32) → i32` | Receive into memory (handle, ptr, cap); returns byte count, 0=no data, -1=error |
| `"send"` | `(param i32 i32 i32) → i32` | Send from memory (handle, ptr, len); returns bytes sent or -1 |
| `"close"` | `(param i32) → i32` | Close TCP connection; always returns 0 |
| `"status"` | `(param i32) → i32` | Socket state: 0=closed, 1=listen, 2=handshaking, 3=established, 4=teardown |
| `"get_ip"` | `() → i32` | Kernel IP as u32 little-endian (0 if DHCP not yet bound) |
| `"set_ip"` | `(param i32) → i32` | Manually set the kernel IP (ip_u32_le); always returns 0 |
| `"udp_bind"` | `(param i32) → i32` | Bind UDP socket to port; returns handle or -1 |
| `"udp_connect"` | `(param i32 i32 i32) → i32` | Set UDP remote (handle, ip_u32_le, port); returns 0 or -1 |
| `"udp_send"` | `(param i32 i32 i32) → i32` | Send UDP datagram (handle, ptr, len); returns bytes sent or -1 |
| `"udp_recv"` | `(param i32 i32 i32) → i32` | Receive UDP datagram (handle, ptr, cap); returns byte count or 0 (non-blocking) |
| `"udp_close"` | `(param i32) → i32` | Close UDP socket; always returns 0 |

Registry capacity: `MAX_HOST_FUNCS = 48`.

### Capacity Limits

| Constant | Value | Where |
|---|---|---|
| `MAX_INSTANCES` | 4 | engine.rs — live WASM instances |
| `MAX_MEM_PAGES` | 16 | engine.rs — 64 KiB pages per instance (1 MiB) |
| `MAX_HOST_FUNCS` | 48 | engine.rs — host function registry |
| `MAX_FUNCS` | 512 | interp.rs — imports + defined functions |
| `MAX_TYPES` | 128 | interp.rs — type section entries |
| `MAX_LOCALS` | 32 | interp.rs — locals per function frame |
| `MAX_GLOBALS` | 64 | interp.rs — global variables |
| `MAX_TABLE` | 512 | interp.rs — function table entries |
| `STACK_DEPTH` | 256 | interp.rs — value stack depth |
| `CALL_DEPTH` | 128 | interp.rs — call stack depth |
| `MAX_CTRL_DEPTH` | 64 | interp.rs — block/loop/if nesting |
| `MAX_TASKS` | 4 | task.rs — concurrent tasks (= MAX_INSTANCES) |

---

## Build Pipeline
The user will run all build commands.

---

## Adding a New WASM Module

1. Create `userland/<name>/<name>.wat`
2. Import host functions from `"env"` and export `main`
3. The file will be loaded at boot via `load_fat_files_to_table()`  
   (or add it to `fs.img` via `tools/pack-fs.sh` for embedded fallback)

---

## Adding a Host Function

1. Write a `fn host_<name>(vstack, vsp, mem) -> Result<(), InterpError>` in `engine.rs`
2. Register it in `init_host_fns()` with `register_host("env", "<name>", host_<name>)`
3. Add a row to the Host Functions table in this file, `README.md`, and `docs/architecture.md`
4. Optionally add a `.wat` test in `wasm-test/tests/`

---

## Development Rules

1. **System must always boot** — never merge if QEMU doesn't boot
2. **Terminal must remain functional** — keyboard/serial input and output always work
3. **No heap in the WASM core** — `loader.rs` and `interp.rs` are allocation-free; the engine uses static pools
4. **Kernel stack budget** — `Interpreter` is ~70 KiB stack-allocated; total stack is 1 MiB (set in `main.rs`)
5. Document all `unsafe` blocks with a `// SAFETY:` comment explaining the invariant
6. Keep rustdoc on all public items.

---

## Agent Task Strategy

When implementing a sprint task:

1. Read the relevant source files before writing anything
2. Identify the minimal change — don't expand scope
3. Keep fixed-size limits conservative (increase only when a real test fails)
4. After changes have the user verify the system still boots (`./tools/run-qemu.sh headless`)
5. Update `AGENTS.md`, `README.md`, `docs/architecture.md`, and `docs/wasm-runtime.md` if the public interface changes
6. Ensure rustdoc is present on all new public items

---

## Completed Sprints Summary

| Sprint | Key Deliverables |
|---|---|
| 1–4 (MVP) | Boot, VGA/serial output, PS/2 keyboard, WASM interpreter (i32), shell, in-memory FS |
| A | i64, f32/f64 (soft-float), globals, `call_indirect`, `br_table`, multi-value returns |
| B | Instance pool, named host registry, `ps` command, memory isolation |
| C | PIT timer, cooperative scheduler, `task-run`/`task-kill`/`tasks`, `yield`/`sleep_ms` |
| D | virtio-blk driver, FAT12/16/32 via rust-fatfs, persistent disk, full shell FS commands |
| E | virtio-net PCI driver, hand-rolled ARP/IP/TCP/UDP/DHCP stack, 12 socket host functions, `httpd.wasm` demo |
| G | In-kernel WAT tokenizer + binary emitter, `asm` shell command, full edit→asm→run round-trip |
| 0 | (JIT) | Boot Prerequisites Call `make_jit_executable()` |
| 1 | (JIT) | Instrumentation & Baseline | 
| 2 | (JIT) | Interpreter Hot-Loop Micro-Optimizations |
| 3 | (JIT) | JIT Foundation: Calling Convention + Arithmetic |