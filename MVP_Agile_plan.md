# 🚀 MVP Goal (Definition of Done)

By the end:

* Boots in QEMU
* Shows a terminal prompt
* Accepts keyboard input
* `ls` shows registered `.wasm` files
* `run <file>` loads a module, discovers its entry point by export name, and executes it
* Prints output from the WASM program via host `print` function
* Runtime handles real WASM output from `wat2wasm` (locals, arithmetic, control flow)

---

# 🧭 Sprint Structure

* Duration: **3 weeks (can compress to 2)**
* Style: **vertical slices** (each step produces something runnable)

---

# 🟢 Sprint 1 (Days 1–5): Boot + Terminal Foundation

## 🎯 Goal

Boot into a usable terminal with input/output.

---

## 🧩 Tasks (Broken Down)

### 1. Project Setup

* [x] Create Rust workspace
* [x] Add target: `x86_64-unknown-none`
* [x] Configure build script
* [x] Set up boot image creation

---

### 2. Bootloader Integration

Use:

* bootloader crate

Tasks:

* [x] Minimal kernel entry point
* [x] Print “Hello World” on boot
* [x] Verify boot in QEMU

✅ Output:

```
Hello World
```

---

### 3. Basic Output System

* [x] VGA text buffer OR framebuffer writer
* [x] `print!` / `println!` macros
* [x] Clear screen function

---

### 4. Keyboard Input

* [x] Handle PS/2 keyboard interrupts
* [x] Decode scancodes → characters
* [x] Print typed characters

✅ Output:

```
> hello
hello
```

---

### 5. Terminal Loop (Shell v0)

* [x] Input buffer (string)
* [x] Enter key handling
* [x] Basic command parsing

Commands:

* [x] `help`
* [x] `echo`

---

## ✅ Sprint 1 Done When:

You see:

```
> echo hi
hi
>
```

---

# 🟡 Sprint 2 (Days 6–10): WASM Runtime Skeleton

## 🎯 Goal

Load and execute a minimal WebAssembly module.

---

## 🧩 Tasks

### 1. WASM Binary Loader

* [x] Load `.wasm` from memory (hardcoded first)
* [x] Parse header (`\0asm`)
* [x] Validate basic structure

---

### 2. Minimal WASM Interpreter

Start tiny—support only:

* [x] `i32.const`
* [x] `call`
* [x] `end`

---

### 3. Execution Engine

* [x] Stack implementation
* [x] Instruction loop
* [x] Function call handling

---

### 4. Host Functions (Your “Syscalls”)

Design interface:

* [x] `print(ptr, len)`
* [x] Wire into terminal output

---

### 5. Hardcoded WASM Test

Example:

* [x] Precompile a WASM module that prints text
* [x] Embed bytes in kernel

---

## ✅ Sprint 2 Done When:

Boot → auto-runs WASM:

```
Hello from WASM!
>
```

---

# 🔵 Sprint 3 (Days 11–15): File System + Run Command

## 🎯 Goal

Run named `.wasm` files from a terminal command with entry-point discovery.

---

## 🧩 Tasks

### 1. In-Memory File System

* [x] `File` struct (`name: &str`, `data: &[u8]`)
* [x] Fixed-size file table (`[Option<File>; MAX_FILES]`)
* [x] `register_file(name, data)` and `find_file(name)`
* [x] Pre-register `HELLO_WASM` as `”hello.wasm”` and a second module as `”greet.wasm”` at boot

---

### 2. Export Section Parsing

* [x] Parse export section in `loader.rs` (already captured, needs iterator)
* [x] `find_export(module, name) -> Option<u32>` — returns func index by name
* [x] Change `engine::run(bytes, func_idx)` → `engine::run(bytes, entry: &str)` using export lookup
* [x] Fallback: if no export named `entry`, return `RunError::EntryNotFound`

---

### 3. Shell Commands

* [x] `ls` — list all registered files
* [x] `info <name>` — show section counts, func count, import count (replaces `cat`)
* [x] `run <name>` — look up file, parse exports, execute entry `”main”`
* [x] `clear` — clear the terminal screen
* [x] Remove old `wasm` test command

---

### 4. Error Handling

* [ ] File not found
* [ ] Export/entry not found
* [ ] Invalid WASM (propagate `LoadError`)
* [ ] Runtime errors (propagate `InterpError`)

---

## ✅ Sprint 3 Done When:

```
> ls
hello.wasm
greet.wasm

> run hello.wasm
Hello from WASM!

> run greet.wasm
Greetings from the second module!
>
```

---

# 🔴 Sprint 4 (Days 16–20): Core Opcode Coverage

## 🎯 Goal

Expand the interpreter enough to run WASM produced by a real assembler (`wat2wasm`),
not just hand-assembled bytes. This is what makes the runtime generally useful.

---

## 🧩 Tasks

### 1. Local Variables

* [ ] `local.get <idx>`
* [ ] `local.set <idx>`
* [ ] `local.tee <idx>`
* [ ] Allocate locals per frame (extend `Frame` with a locals array)

---

### 2. Arithmetic & Comparison (i32)

* [ ] `i32.add`, `i32.sub`, `i32.mul`
* [ ] `i32.and`, `i32.or`, `i32.xor`, `i32.shl`, `i32.shr_s`
* [ ] `i32.eq`, `i32.ne`, `i32.lt_s`, `i32.gt_s`, `i32.le_s`, `i32.ge_s`
* [ ] `i32.eqz`

---

### 3. Memory Operations

* [ ] `i32.load` (4-byte load from linear memory)
* [ ] `i32.store` (4-byte store)
* [ ] `i32.load8_u`, `i32.store8`
* [ ] Bounds-check all memory accesses → `InterpError::MemOutOfBounds`

---

### 4. Control Flow

* [ ] `if / else / end`
* [ ] `block / end` with `br` (branch-to-end)
* [ ] `loop / end` with `br` (branch-to-top)
* [ ] `br_if`
* [ ] `return`
* [ ] `drop`, `select`
* [ ] `nop`, `unreachable`

---

### 5. Validation

* [ ] Reject unknown section IDs gracefully (skip with logged warning)
* [ ] Enforce stack underflow detection on all pop operations

---

## ✅ Sprint 4 Done When:

A module compiled with `wat2wasm` that uses locals, arithmetic, and a loop runs correctly:

```
> run fib.wasm
fib(10) = 55
```
---

# 🧱 Backlog (Post-MVP)

* i64 / f32 / f64 type support
* Multi-value returns
* Table section + `call_indirect`
* Memory isolation per module (separate `mem` per instance)
* Multiple WASM instances running concurrently
* Preemptive scheduling
* Disk-backed filesystem
* Networking stack
* JIT compilation
* WAT parser (true in-OS assembler)

---

# ⚙️ Daily Workflow

Each day:

1. Code
2. Build image
3. Run in QEMU
4. Verify output

---

# 🧪 Testing Strategy

* Unit test logic outside kernel (Rust std env)
* Kernel tests:

  * Visual output
  * Serial logs

---

# 💡 Key Agile Principle for This Project

> **Always keep the system bootable.**

Never break:

* boot
* terminal

Everything else is incremental.

---
