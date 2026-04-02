---

# 🚀 MVP Goal (Definition of Done)

By the end:

* Boots in QEMU
* Shows a terminal prompt
* Accepts keyboard input
* Can load and execute a `.wasm` file
* Prints output from the WASM program

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

* [ ] Handle PS/2 keyboard interrupts
* [ ] Decode scancodes → characters
* [ ] Print typed characters

✅ Output:

```
> hello
hello
```

---

### 5. Terminal Loop (Shell v0)

* [ ] Input buffer (string)
* [ ] Enter key handling
* [ ] Basic command parsing

Commands:

* [ ] `help`
* [ ] `echo`

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

* [ ] Load `.wasm` from memory (hardcoded first)
* [ ] Parse header (`\0asm`)
* [ ] Validate basic structure

---

### 2. Minimal WASM Interpreter

Start tiny—support only:

* [ ] `i32.const`
* [ ] `call`
* [ ] `end`

---

### 3. Execution Engine

* [ ] Stack implementation
* [ ] Instruction loop
* [ ] Function call handling

---

### 4. Host Functions (Your “Syscalls”)

Design interface:

* [ ] `print(ptr, len)`
* [ ] Wire into terminal output

---

### 5. Hardcoded WASM Test

Example:

* Precompile a WASM module that prints text
* Embed bytes in kernel

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

Run external `.wasm` files via terminal.

---

## 🧩 Tasks

### 1. In-Memory File System

* [ ] File struct (`name`, `data`)
* [ ] File table (array or hashmap)
* [ ] `read_file(name)`

---

### 2. File Commands

* [ ] `ls`
* [ ] `cat file.wasm`

---

### 3. WASM Loader from FS

* [ ] Replace hardcoded module
* [ ] Load from file system

---

### 4. `run` Command

* [ ] `run hello.wasm`
* [ ] Execute module
* [ ] Capture output

---

### 5. Error Handling

* [ ] File not found
* [ ] Invalid WASM
* [ ] Runtime errors

---

## ✅ Sprint 3 Done When:

```
> ls
hello.wasm

> run hello.wasm
Hello from WASM!
>
```

---

# 🔴 Optional Sprint 4 (Days 16–20): Editing + Compile

*(Stretch goal, not required for MVP)*

---

## 🎯 Goal

Basic code editing + simple compile step.

---

## 🧩 Tasks

### 1. Text Editor (Minimal)

* [ ] `edit file.wat`
* [ ] Append text
* [ ] Save file

---

### 2. WAT Support (Optional)

* [ ] Store `.wat` files
* [ ] Stub “compiler”

---

### 3. Fake Compile Step (Shortcut)

* [ ] `build file.wat`
* just maps → precompiled `.wasm`

👉 Keeps momentum without building a compiler

---

## ✅ Done When:

```
> edit hello.wat
> build hello.wat
> run hello.wasm
```

---

# 🧱 Backlog (Post-MVP)

* Real WASM spec support
* Memory isolation per module
* Preemptive scheduling
* Disk-backed filesystem
* Networking
* JIT compilation

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

# 🧭 Critical Path (Shortest Route to Success)

If you want to *compress to 1–2 weeks*, focus ONLY on:

1. Boot + terminal
2. Hardcoded WASM execution
3. `run` command

Skip:

* editor
* compiler
* real filesystem

---