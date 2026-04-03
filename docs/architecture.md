# Architecture

## Overview

WASM-First OS is a bare-metal x86_64 kernel written in `no_std` Rust. The entire
system lives in a single `kernel` crate. WebAssembly modules are the only
user-visible execution unit — there are no processes, no POSIX, no native binaries.

```
+-----------------------------+
|     WASM Modules (.wasm)    |
+-----------------------------+
|  WASM Interpreter           |
|  loader / engine / interp   |
+-----------------------------+
|  Shell + In-Memory FS       |
+-----------------------------+
|  Framebuffer + Keyboard     |
+-----------------------------+
|  x86_64 bare metal / QEMU   |
+-----------------------------+
```

---

## Boot Sequence

1. BIOS loads the bootloader (built by the `bootloader` crate, BIOS mode)
2. Bootloader sets up long mode, page tables, and calls `kernel_main`
3. `kernel_main` initialises the framebuffer writer
4. Embedded `.wasm` modules are registered in the in-memory filesystem
5. The boot module (`hello.wasm`) is auto-executed
6. The PS/2 keyboard loop starts — the shell runs from here

Stack size: 256 KiB (configured via `BootloaderConfig`).

---

## Kernel Components

### Framebuffer (`vga.rs`)

- Writes directly to the linear framebuffer provided by the bootloader
- 8×8 pixel bitmap font for printable ASCII
- Tracks character cursor (col, row); scrolls when the last row is exceeded
- `clear_screen()` zeroes the framebuffer and resets the cursor
- Protected by a `spin::Mutex`

### Keyboard (`keyboard.rs`)

- Handles IRQ1 (PS/2 keyboard) via the IDT
- Decodes US QWERTY scancodes to characters
- Accumulates a line buffer; calls `shell::run_command` on Enter
- Supports backspace

### Shell (`shell.rs`)

- Simple tokenizer — splits on whitespace, handles quoted strings
- Ring-buffer command history (16 entries)
- Commands: `help` `echo` `history` `clear` `ls` `info` `run`
- `run <name> [args...]` parses trailing integers and passes them as WASM params

### In-Memory Filesystem (`fs.rs`)

- Fixed-size table: `[Option<File>; 16]`
- Files are `(&'static str, &'static [u8])` pairs — names and data are static
- `register_file` / `find_file` / `for_each_file`
- All `.wasm` modules are embedded in the kernel binary via `include_bytes!`

---

## WASM Subsystem

### Loader (`wasm/loader.rs`)

Parses a WASM binary into a `Module<'_>` — zero-copy slices into the input buffer.
Captured sections: type, import, function, export, code, data.
Unknown sections are silently skipped (table, memory, global, element, start).

`find_export(module, name)` scans the export section for a named function.

### Engine (`wasm/engine.rs`)

`run(bytes, entry, args)`:
1. Load and parse the module
2. Count function imports
3. Look up entry point by export name
4. Construct `Interpreter`, set host function
5. Initialise linear memory from data segments
6. Push caller args onto the value stack
7. Call entry function

Host function dispatch (by import index):
- 0: `print(ptr, len)` — UTF-8 from linear memory
- 1: `print_int(n)` — decimal integer + newline

### Interpreter (`wasm/interp.rs`)

A stack machine with fixed-size arrays — no heap.

Key data structures:
- `vstack: [i32; 256]` — value stack
- `frames: [Frame; 128]` — call stack; each frame owns its locals
- `ctrl: [CtrlFrame; 64]` — control stack (block/loop/if nesting), shared across frames
- `mem: [u8; 4096]` — linear memory

`Frame` holds: `body_idx`, `pc`, `locals: [i32; 16]`, `local_count`, `ctrl_base`.

`CtrlFrame` holds: `kind` (Block/Loop/If), `pc_start` (loop restart target), `end_pc`.

**Control flow** uses `scan_block_end` to locate matching `end`/`else` positions at block-entry time by scanning forward over all immediates. LEB-128 bytes with the high bit clear (like `0x0B`) can only appear as final bytes of an immediate, so the scan is safe.

**Parameter passing**: `push_frame` pops `param_count` values from the value stack
(derived from the type section) into `frame.locals[0..param_count]`.

---

## Memory Model

- No dynamic allocation — no `alloc` crate, no heap
- All capacities are compile-time constants in `interp.rs`
- Linear memory is a flat `[u8; 4096]` inside the `Interpreter` struct
- The interpreter struct (~11 KB) lives on the kernel stack inside `engine::run`

Sprint B will move linear memory to a pool-allocated buffer per instance.

---

## Host Interface

WASM modules interact with the kernel exclusively through imported functions.
There is no implicit access to memory, I/O, or other modules.

```wat
(import "env" "print"     (func $print     (param i32 i32)))
(import "env" "print_int" (func $print_int (param i32)))
```

Import module names are not validated — dispatch is by function index only.
Sprint B will introduce a named host function registry.
