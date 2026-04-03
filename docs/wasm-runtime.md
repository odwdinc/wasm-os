# WASM Runtime Internals

The runtime lives in `kernel/src/wasm/` and consists of three files:
`loader.rs`, `engine.rs`, and `interp.rs`.

---

## Loader (`loader.rs`)

Parses a WASM binary into a `Module<'_>` — zero-copy slices into the
input buffer. No allocation; all fields are `Option<&'a [u8]>` pointing
directly into the source bytes.

**Sections captured:**

| ID | Name | Field |
|---|---|---|
| 1 | Type | `type_section` |
| 2 | Import | `import_section` |
| 3 | Function | `function_section` |
| 7 | Export | `export_section` |
| 10 | Code | `code_section` |
| 11 | Data | `data_section` |

All other sections (table, memory, global, element, start, custom) are silently
skipped.

**`find_export(module, name) -> Option<u32>`**

Scans the export section for an export named `name` with kind = func (0).
Returns the absolute function index (imports counted first).

**LEB-128**

`read_u32_leb128(bytes) -> Option<(u32, usize)>` decodes an unsigned
32-bit LEB-128 integer and returns the value and the number of bytes consumed.
Returns `None` on overflow or truncation.

---

## Engine (`engine.rs`)

`run(bytes: &[u8], entry: &str, args: &[i32]) -> Result<(), RunError>`

1. Load and parse the module (`loader::load`)
2. Count function imports (`count_func_imports`)
3. Look up entry point by export name (`find_export`)
4. Construct `Interpreter`, set host function pointer
5. Initialise linear memory from data segments
6. Push caller args onto the value stack
7. Call the entry function

**Host function dispatch (`kernel_host`)**

Dispatch is by import index, not by name.

| Index | Import | Signature | Behaviour |
|---|---|---|---|
| 0 | `"env"."print"` | `(param i32 i32)` | Print UTF-8 from linear memory (ptr, len) |
| 1 | `"env"."print_int"` | `(param i32)` | Print i32 as decimal + newline |

**Embedded WASM constants**

```rust
pub const HELLO_WASM: &[u8] = include_bytes!("../../../userland/hello/hello.wasm");
pub const GREET_WASM: &[u8] = include_bytes!("../../../userland/greet/greet.wasm");
pub const FIB_WASM:   &[u8] = include_bytes!("../../../userland/fib/fib.wasm");
```

---

## Interpreter (`interp.rs`)

A stack machine with all state in fixed-size arrays — no heap, no `alloc`.

### Data Structures

```
vstack: [i32; 256]        — value stack
frames: [Frame; 128]      — call stack
ctrl:   [CtrlFrame; 64]   — control stack (block/loop/if nesting)
mem:    [u8; 4096]        — linear memory
```

**`Frame`**

```rust
struct Frame {
    body_idx:    usize,       // index into function body table
    pc:          usize,       // program counter within body
    locals:      [i32; 16],  // local variables (params + declared locals)
    local_count: usize,
    ctrl_base:   usize,       // control stack depth at frame entry
}
```

**`CtrlFrame`**

```rust
struct CtrlFrame {
    kind:     BlockKind,   // Block | Loop | If
    pc_start: usize,       // loop restart target (pc of loop opcode)
    end_pc:   usize,       // pc of matching `end` opcode
}
```

### Capacity Constants

| Constant | Value | Meaning |
|---|---|---|
| `STACK_DEPTH` | 256 | Maximum value stack depth |
| `CALL_DEPTH` | 128 | Maximum call stack depth (frames) |
| `MAX_CTRL_DEPTH` | 64 | Maximum nested blocks/loops/ifs |
| `MAX_FUNCS` | 32 | Maximum defined functions |
| `MAX_TYPES` | 16 | Maximum type section entries |
| `MAX_LOCALS` | 16 | Maximum locals per function |
| `MEM_SIZE` | 4096 | Linear memory size in bytes |

The entire `Interpreter` struct (~11 KB) is stack-allocated inside
`engine::run`. The kernel stack is configured to 256 KiB in `main.rs`.

### Control Flow

Block entry (`block`, `loop`, `if`) calls `scan_block_end(body, start)` which
scans forward over all immediates to find the matching `end` and, for `if`,
the optional `else`. The result is stored in a `CtrlFrame` pushed onto `ctrl`.

**`scan_block_end` safety:** LEB-128 bytes with the high bit clear (such as
`0x02`, `0x03`, `0x04`, `0x0B`) can only appear as the *final* byte of a
multi-byte LEB-128 immediate. The scanner skips over immediates correctly and
never mistakes an immediate byte for a nested opcode.

`br N` and `br_if N` call `do_br(interp, depth)`:
- For `Block`: jump to `end_pc + 1` (after the `end`)
- For `Loop`: jump to `pc_start` (back to the `loop` opcode)

`return` pops the current frame; if no frames remain, execution is done.

### Parameter Passing

`push_frame(interp, func_idx, param_count)` pops `param_count` values from
the value stack and stores them in `frame.locals[0..param_count]`. Additional
declared locals are zero-initialised.

### Supported Opcodes

| Category | Opcodes |
|---|---|
| Control | `nop` `unreachable` `block` `loop` `if` `else` `end` `br` `br_if` `return` |
| Calls | `call` |
| Locals | `local.get` `local.set` `local.tee` |
| i32 arithmetic | `i32.add` `i32.sub` `i32.mul` `i32.and` `i32.or` `i32.xor` `i32.shl` `i32.shr_s` |
| i32 comparison | `i32.eq` `i32.ne` `i32.lt_s` `i32.gt_s` `i32.le_s` `i32.ge_s` `i32.eqz` |
| Memory | `i32.load` `i32.store` `i32.load8_u` `i32.store8` |
| Stack | `drop` `select` `i32.const` |

### Error Types

`InterpError`:
- `StackOverflow` / `StackUnderflow`
- `CallStackOverflow`
- `CtrlStackOverflow`
- `OutOfBoundsMemory`
- `UndefinedFunction`
- `Unreachable`
- `UnknownOpcode(u8)`
- `DivisionByZero` (Sprint A)
