# Architecture

## Overview

WASM-First OS is a bare-metal x86_64 kernel written in `no_std` Rust (with `alloc`).
The entire system lives in a single `kernel` crate. WebAssembly modules are the only
user-visible execution unit — there are no processes, no POSIX, no native binaries.

```
+-----------------------------+
|     WASM Modules (.wasm)    |  ← userland/
+-----------------------------+
|  WASM Interpreter           |  ← kernel/src/wasm/
|  - loader / engine / interp |
|  - host function registry   |
|  - task scheduler           |
+-----------------------------+
|  Shell + FAT Filesystem     |  ← kernel/src/shell/, kernel/src/fs/
|  - command dispatcher       |
|  - CWD tracking             |
|  - virtio-blk + ramdisk     |
+-----------------------------+
|  Drivers + Interrupts       |  ← kernel/src/drivers/, interrupts/
|  - framebuffer, serial      |
|  - PS/2 keyboard, PIT timer |
+-----------------------------+
|  x86_64 bare metal / QEMU   |
+-----------------------------+
```

---

## Boot Sequence

1. BIOS loads the bootloader (built by the `bootloader` crate, BIOS mode)
2. Bootloader sets up long mode, page tables, and calls `kernel_main`
3. `kernel_main` initialises serial, interrupts, heap allocator, framebuffer
4. Physical-memory mapping is resolved for virtio DMA ring setup
5. FAT filesystem is mounted: virtio-blk first, ramdisk fallback (`fs.img`)
6. All files from the mounted FAT volume are loaded into the in-memory file table
7. `hello.wasm` is auto-executed if present
8. The preemptive scheduler starts (`scheduler::run`) — the shell runs as the idle task

Stack size: 1 MiB (configured via `BootloaderConfig`; WASM interpreter frames are large).

---

## Kernel Components

### Framebuffer (`drivers/vga.rs`, `vga.rs`)

- Writes directly to the linear framebuffer provided by the bootloader
- 8×8 pixel bitmap font for printable ASCII
- Tracks character cursor (col, row); scrolls when the last row is exceeded
- `clear_screen()` zeroes the framebuffer and resets the cursor
- Protected by a `spin::Mutex`
- All output is also mirrored to the serial port (`drivers/serial.rs`) for headless use

### Keyboard (`drivers/keyboard.rs`)

- Handles IRQ1 (PS/2 keyboard) via the IDT
- Decodes US QWERTY scancodes to characters
- Puts decoded bytes into a ring buffer consumed by `shell::input::read_line`
- Supports backspace; Enter triggers command dispatch

### PIT Timer (`drivers/pit.rs`)

- Configured for ~10 ms ticks (IRQ0)
- Tick counter drives `sleep_ms` host function and future scheduling preemption

### Shell (`shell/mod.rs`, `shell/commands/`)

- Simple tokenizer — splits on whitespace, up to 8 arguments
- Ring-buffer command history (16 entries, 128 bytes/entry)
- Current working directory (CWD) tracked in a static 128-byte buffer; lazy-initialised to `"/"`
- Commands dispatched by name to individual modules in `shell/commands/`

**Available commands:**

| Command | Module | Description |
|---|---|---|
| `help` | `help.rs` | List all commands |
| `echo` | `echo.rs` | Print arguments |
| `history` | `history.rs` | Show command history |
| `clear` | `clear.rs` | Clear screen |
| `ls` | `ls.rs` | List files in CWD (via FAT) |
| `cat <file>` | `cat.rs` | Print file contents (CWD-relative) |
| `cd <dir>` | `cd.rs` | Change CWD; validates with `fat_is_dir` |
| `mkdir <dir>` | `mkdir.rs` | Create FAT directory |
| `df` | `df.rs` | Show FAT volume space usage |
| `rm <name>` | `rm.rs` | Remove a file from FAT and in-memory table |
| `write <name> <hex>` | `write.rs` | Write hex-encoded bytes as a new file |
| `edit <name>` | `edit.rs` | Line-append editor (`:w` save, `:q` quit) |
| `save` | `save.rs` | Flush in-memory table to FAT volume |
| `info [name]` | `info.rs` | Module section info or tick count |
| `run <name> [args]` | `run.rs` | Execute a `.wasm` module synchronously |
| `ps` | `ps.rs` | List active WASM instance pool slots |
| `task-run <name>` | `tasks.rs` | Spawn a module as a background task |
| `task-kill <id>` | `tasks.rs` | Kill a task by ID |
| `tasks` | `tasks.rs` | List all tasks with state |

### Filesystem (`fs/`)

The filesystem layer has two parts:

**FAT driver (`fs/fat.rs`)** — wraps `rust-fatfs` (FAT12/16/32):
- `mount_virtio(blk)` / `mount_ramdisk(bytes)` — mount the volume
- `fat_list_path(path, cb)` — enumerate entries in a directory
- `fat_read_file(name)` / `fat_read_path(path)` — read file bytes into a `Vec<u8>`
- `fat_write_file(name, data)` — write or overwrite a root-dir file
- `fat_remove_file(name)` — delete a file
- `fat_mkdir(name)` — create a directory
- `fat_is_dir(name)` — check if a path is a directory
- `fat_disk_stats()` — return `(total_bytes, free_bytes)`
- `split_path(path)` — split `"dir/file"` into `("dir", "file")`

**In-memory file table (`fs/mod.rs`)** — static pools for `'static` slices required by the WASM engine:

| Pool | Slots | Slot size | Purpose |
|---|---|---|---|
| `FILE_TABLE` | 64 entries | — | Name → `&'static [u8]` registry |
| `DISK_POOL` | 64 slots | 8 KiB each | Files loaded from FAT at boot |
| `WRITE_POOL` | 16 slots | 4 KiB each | Files written during the session |

`load_fat_files_to_table()` — called at boot; reads every FAT file into `DISK_POOL` and registers it in `FILE_TABLE`.

### Block Devices (`fs/block.rs`, `drivers/virtio_blk.rs`)

- `BlockDevice` trait — `read_block(lba, buf)` / `write_block(lba, buf)`
- `Ramdisk` — in-memory block device backed by a `&'static [u8]`
- `VirtioBlk` — virtio 1.0 block device; uses DMA with physical-address translation from the page table walker

### WASM Subsystem (`wasm/`)

See [wasm-runtime.md](wasm-runtime.md) for full details.

- **`loader.rs`** — zero-copy WASM binary parser
- **`engine.rs`** — instance pool, host function registry, `spawn`/`call`/`destroy` API
- **`interp.rs`** — stack machine interpreter; all state in fixed-size arrays
- **`task.rs`** — cooperative task wrapper; integrates with `scheduler.rs`

### Scheduler (`scheduler.rs`)

- Fixed-size task table (`[Option<Task>; MAX_TASKS]`)
- Tasks are WASM instances stepped cooperatively; timer interrupt drives `sleep_ms` wakeup
- Shell input loop runs as the main (non-task) execution context

---

## Memory Model

- Heap available via a simple bump allocator (`memory/allocator.rs`)
- WASM linear memory: each instance slot has a dedicated static region (`SLOT_MEM[slot]`)
- In-memory FS pools (`DISK_POOL`, `WRITE_POOL`) are static arrays — no heap needed for file data
- The interpreter struct (~70 KiB) lives on the kernel stack inside `engine::spawn`

---

## Host Interface

WASM modules interact with the kernel exclusively through imported functions.
There is no implicit access to memory, I/O, or other modules.

All imports are resolved by name at instantiation time via the host function registry.
`spawn` returns `RunError::ImportNotFound` if any import is unregistered.

| Import | Signature | Description |
|---|---|---|
| `"env"."print"` | `(param i32 i32)` | Print UTF-8 string from linear memory (ptr, len) |
| `"env"."print_int"` | `(param i32)` | Print i32 as decimal + newline |
| `"env"."print_i64"` | `(param i64)` | Print i64 as decimal + newline |
| `"env"."print_char"` | `(param i32)` | Print low byte as a single ASCII character |
| `"env"."print_hex"` | `(param i32)` | Print i32 as `0x` + 8 uppercase hex digits + newline |
| `"env"."yield"` | `()` | Yield to the scheduler (cooperative multitasking) |
| `"env"."sleep_ms"` | `(param i32)` | Yield for at least N milliseconds |
| `"env"."uptime_ms"` | `() → i32` | Milliseconds since boot (PIT ticks × 10) |
| `"env"."exit"` | `(param i32)` | Terminate the module cleanly (exit code consumed, not reported) |
| `"env"."read_char"` | `() → i32` | Block until a key is pressed; returns ASCII code (Enter = 10) |
| `"env"."read_line"` | `(param i32 i32) → i32` | Read a line into memory (ptr, cap); echoes input; returns byte count or -1 |
| `"env"."args_get"` | `(param i32 i32) → i32` | Copy space-joined run args into memory (ptr, cap); returns byte count or -1 |
| `"env"."fs_read"` | `(param i32 i32 i32 i32) → i32` | Read file into memory (name_ptr, name_len, buf_ptr, buf_cap); returns byte count or -1 |
| `"env"."fs_write"` | `(param i32 i32 i32 i32) → i32` | Write bytes from memory to a file (name_ptr, name_len, buf_ptr, buf_len); returns 0 or -1 |
| `"env"."fs_size"` | `(param i32 i32) → i32` | Return file size in bytes (name_ptr, name_len), or -1 if not found |

The registry capacity is `MAX_HOST_FUNCS = 32`. Additional functions can be registered
via `engine::register_host` before any module is spawned.
