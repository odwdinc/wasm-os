# Userland WASM Modules

WASM modules are the only user-visible execution unit in WASM-First OS.
There are no processes, no native binaries, no POSIX.

Each module is written in WebAssembly Text Format (`.wat`), compiled to
binary (`.wasm`) with `wat2wasm`, and embedded into the kernel binary at
compile time via `include_bytes!`.

---

## Included Modules

| File | Source | Description |
|---|---|---|
| `hello/hello.wat` | → `hello.wasm` | Prints "Hello from WASM!\n" |
| `greet/greet.wat` | → `greet.wasm` | Prints "Greetings from the second module!\n" |
| `fib/fib.wat` | → `fib.wasm` | Recursive Fibonacci: `run fib.wasm <n>` |

---

## Writing a New Module

### 1. Create the source file

```
userland/<name>/<name>.wat
```

### 2. Import host functions and export `main`

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

### 3. Compile

```bash
./tools/wasm-pack.sh
```

This runs `wat2wasm` on every `.wat` file under `userland/` and writes
the `.wasm` output beside the source.

### 4. Register in the kernel

In `kernel/src/wasm/engine.rs`, add an `include_bytes!` constant:

```rust
pub const NAME_WASM: &[u8] = include_bytes!("../../../userland/<name>/<name>.wasm");
```

In `kernel/src/main.rs`, register the file:

```rust
fs::register_file("<name>.wasm", wasm::engine::NAME_WASM);
```

### 5. Build and run

```bash
./tools/run-qemu.sh
```

```
> ls
hello.wasm
greet.wasm
fib.wasm
<name>.wasm

> run <name>.wasm
```

---

## Host Functions

| Index | Import | Signature | Description |
|---|---|---|---|
| 0 | `"env"."print"` | `(param i32 i32)` | Print UTF-8 string from linear memory (ptr, len) |
| 1 | `"env"."print_int"` | `(param i32)` | Print i32 as decimal + newline |

Import module names are not validated — dispatch is by import index only.

---

## Passing Arguments

The shell `run` command parses trailing integers and passes them as parameters
to the exported `main` function:

```
> run fib.wasm 20
6765
```

The module must export `main` with the matching parameter count:

```wat
(func (export "main") (param $n i32)
  local.get $n
  call $fib
  call $print_int
)
```

---

## Constraints

- Linear memory is a flat `[u8; 4096]` — 4 KiB per instance (MVP)
- Maximum 16 locals per function
- Maximum call depth: 128 frames
- No heap, no dynamic allocation inside modules
