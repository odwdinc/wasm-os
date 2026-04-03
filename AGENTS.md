# AGENTS.md — WASM-First OS

> Bare-metal Rust kernel. WebAssembly as the system ABI. MVP complete.

---

## Project Status

**MVP is done.** The system boots, runs a shell, and executes real WASM modules.
Next work begins at [Sprint A](Post_MVP_Agile_plan.md).

---

## Actual Source Layout

```
/
├── Cargo.toml                   # Workspace root (kernel, runtime, shell, fs, shared)
├── rust-toolchain.toml          # Pinned nightly toolchain
├── README.md
├── AGENTS.md                    # This file
├── CONTRIBUTING.md
├── MVP_Agile_plan.md            # Sprints 1–4 (complete)
├── Post_MVP_Agile_plan.md       # Sprints A–G (next)
│
├── kernel/                      # The entire working system lives here
│   └── src/
│       ├── main.rs              # Entry point, boot sequence, file registration
│       ├── vga.rs               # Framebuffer writer, 8×8 font, scrolling
│       ├── keyboard.rs          # PS/2 interrupt handler, scancode decoder
│       ├── shell.rs             # REPL, command dispatch, history, tokenizer
│       ├── fs.rs                # In-memory file table (register_file, find_file)
│       ├── drivers/             # Hardware driver stubs (future expansion)
│       ├── interrupts/          # IDT setup and handlers
│       ├── memory/              # Memory management stubs
│       └── wasm/
│           ├── mod.rs           # Module re-exports
│           ├── loader.rs        # WASM binary parser (sections → Module struct)
│           ├── engine.rs        # run(), host functions (print, print_int)
│           └── interp.rs        # Interpreter loop, all opcodes, control stack
│
├── runner/                      # Host-side tool: wraps kernel ELF → BIOS disk image
│   └── src/main.rs              # Uses bootloader crate
│
├── userland/                    # WASM source modules
│   ├── hello/hello.wat          # Prints "Hello from WASM!\n"
│   ├── greet/greet.wat          # Prints "Greetings from the second module!\n"
│   └── fib/fib.wat              # Recursive fibonacci: run fib.wasm <n>
│
├── tools/
│   ├── wasm-pack.sh             # Step 1: compile userland/*.wat → *.wasm
│   ├── build-image.sh           # Step 2: wasm-pack + cargo build + disk image
│   └── run-qemu.sh              # Step 3: build-image + launch QEMU
│
├── docs/
│   ├── architecture.md          # System design details
│   ├── wasm-runtime.md          # Interpreter internals
│   └── roadmap.md               # Sprint-by-sprint plan
│
└── shared/, runtime/, shell/, fs/   # Empty workspace crates (reserved for Sprint B+)
```

---

## What Is Actually Built

### Kernel (`kernel/src/`)

| File | What it does |
|---|---|
| `main.rs` | Bootloader entry, framebuffer init, file registration, keyboard loop |
| `vga.rs` | Framebuffer text output, scrolling, 8×8 bitmap font, `clear_screen()` |
| `keyboard.rs` | PS/2 interrupt handler, scancode → char, line buffering |
| `shell.rs` | Tokenizer, command dispatch: `help echo history clear ls info run` |
| `fs.rs` | `[Option<File>; 16]` table, `register_file` / `find_file` / `for_each_file` |

### WASM Subsystem (`kernel/src/wasm/`)

| File | What it does |
|---|---|
| `loader.rs` | Parses WASM binary sections into zero-copy `Module<'_>` slices; `find_export` |
| `engine.rs` | `run(bytes, entry, args)`, data section init, host function dispatch |
| `interp.rs` | Stack machine interpreter — see opcode table below |

### Supported Opcodes

| Category | Opcodes |
|---|---|
| Control | `nop` `unreachable` `block` `loop` `if/else/end` `br` `br_if` `return` |
| Calls | `call` (imports → host dispatch, defined → push frame with params) |
| Locals | `local.get` `local.set` `local.tee` |
| i32 arithmetic | `add` `sub` `mul` `and` `or` `xor` `shl` `shr_s` |
| i32 comparison | `eq` `ne` `lt_s` `gt_s` `le_s` `ge_s` `eqz` |
| Memory | `i32.load` `i32.store` `i32.load8_u` `i32.store8` |
| Stack | `drop` `select` `i32.const` |

### Host Functions

| Index | Import | Signature | Behaviour |
|---|---|---|---|
| 0 | `"env"."print"` | `(param i32 i32)` | Print UTF-8 from linear memory (ptr, len) |
| 1 | `"env"."print_int"` | `(param i32)` | Print i32 as decimal + newline |

---

## Build Pipeline

```bash
./tools/run-qemu.sh          # full pipeline
./tools/build-image.sh       # wasm-pack + kernel build + disk image only
./tools/wasm-pack.sh         # compile userland .wat → .wasm only
```

The `.wasm` files are embedded into the kernel binary via `include_bytes!` at compile time.
**Run `wasm-pack.sh` before building the kernel** if you change any `.wat` files.

---

## Adding a New WASM Module

1. Create `userland/<name>/<name>.wat`
2. Import host functions and export `main`
3. Run `tools/wasm-pack.sh`
4. Register in `kernel/src/main.rs`:
   ```rust
   fs::register_file("<name>.wasm", wasm::engine::<NAME>_WASM);
   ```
5. Add `include_bytes!` constant in `kernel/src/wasm/engine.rs`

---

## Adding a Host Function

1. Add a new `match` arm in `kernel_host()` in `engine.rs` (next available index)
2. Document it in `AGENTS.md` host function table
3. Update any `.wat` modules that use it

---

## Development Rules

1. **System must always boot** — never merge if QEMU doesn't boot
2. **Terminal must remain functional** — input/output always works
3. **No heap** — all data structures are fixed-size arrays; no `alloc`
4. **Kernel stack budget** — `Interpreter` is stack-allocated (~11KB); keep stack use under 200KB
5. Prefer small, incremental changes
6. Document all `unsafe` blocks

---

## Agent Task Strategy

When implementing a sprint task:

1. Read the relevant source files before writing anything
2. Identify the minimal change — don't expand scope
3. Keep all fixed-size limits conservative (increase only if a test fails)
4. Verify the system still boots after changes
5. Update this file and `README.md` if the public interface changes

---

## Next Work (Sprint A)

See [Post_MVP_Agile_plan.md](Post_MVP_Agile_plan.md) for full task breakdown.

Priority tasks:
- `i64` type support (extend value stack to tagged union)
- `memory.size` / `memory.grow`
- `i32.rem_s`, `i32.rem_u`, `i32.div_s`, `i32.div_u`
- `br_table`
- Global variables (section ID 6)
- `call_indirect` + table section

Done condition for Sprint A:
```
> run primes.wasm
Primes up to 50: 2 3 5 7 11 13 17 19 23 29 31 37 41 43 47
```
