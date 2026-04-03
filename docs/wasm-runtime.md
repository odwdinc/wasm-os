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
| 4 | Table | `table_section` |
| 5 | Memory | `memory_section` |
| 6 | Global | `global_section` |
| 7 | Export | `export_section` |
| 9 | Element | `element_section` |
| 10 | Code | `code_section` |
| 11 | Data | `data_section` |

Custom sections (ID 0) are silently skipped.

**`find_export(module, name) -> Option<u32>`**

Scans the export section for a function export named `name`.
Returns the absolute function index (imports counted first).

**`for_each_func_import(section, f)`**

Iterates over the import section, calling `f(module_name, func_name)`
for each function import in declaration order. Non-function imports
(table, memory, global) are skipped silently.

**`read_memory_min_pages(section) -> u32`**

Parses the memory section and returns the `min` page count declared by
the module. Returns 0 if absent or malformed.

**LEB-128**

`read_u32_leb128(bytes) -> Option<(u32, usize)>` — unsigned 32-bit.  
`read_i32_leb128(bytes) -> Option<(i32, usize)>` — signed 32-bit (for
data segment offsets and `i32.const` immediates).

---

## Engine (`engine.rs`)

Owns the host function registry, the instance pool, and per-instance
linear memory. All state lives in `static mut` — no heap allocation.

### Host Function Registry

```
MAX_HOST_FUNCS = 16
```

At boot, `init_host_fns()` registers the kernel's built-in host functions.
Additional functions can be registered with `register_host(module, name, fn)`.
Import resolution happens at instantiation: if any import is unregistered,
`spawn` returns `RunError::ImportNotFound` before any code executes.

**Built-in host functions (registered under `"env"`):**

| Name | Signature | Behaviour |
|---|---|---|
| `"print"` | `(param i32 i32)` | Print UTF-8 from linear memory (ptr, len) |
| `"print_int"` | `(param i32)` | Print i32 as decimal + newline |

### Instance Pool

```
MAX_INSTANCES = 4
MAX_MEM_PAGES = 4   (1 page = 64 KiB)
```

Each pool slot has a dedicated `256 KiB` static memory region
(`SLOT_MEM[slot]`). Only `min_pages * 65536` bytes of each slot are
active. Slots are zeroed on `spawn` and on `destroy`.

**Public API:**

| Function | Description |
|---|---|
| `init_host_fns()` | Register kernel built-ins — call once at boot |
| `register_host(module, name, fn)` | Add an entry to the host registry |
| `spawn(name, bytes) -> Result<usize, RunError>` | Instantiate into pool, return handle |
| `call_handle(handle, entry, args) -> Result<Option<i64>, RunError>` | Execute exported function |
| `destroy(handle)` | Drop instance, zero slot memory |
| `for_each_instance(f)` | Iterate active slots: `f(handle, name, mem_pages)` |
| `run(bytes, entry, args)` | Convenience: spawn + call + destroy |

**`spawn` sequence:**

1. Find a free pool slot (error: `PoolFull`)
2. `loader::load(bytes)` — parse module header and sections
3. `read_memory_min_pages` — validate against `MAX_MEM_PAGES` (error: `MemoryTooLarge`)
4. Zero `SLOT_MEM[slot][..mem_bytes]`, take `&'static mut [u8]` slice
5. Resolve all function imports against the host registry (error: `ImportNotFound`)
6. `Interpreter::new(module, import_count, mem, host_fns)`
7. `init_memory` — copy active data segments into linear memory
8. Write `Instance` into `POOL[slot]` via `MaybeUninit::write`

**`RunError` variants:**

| Variant | Meaning |
|---|---|
| `Load(LoadError)` | Binary parse failure |
| `Interp(InterpError)` | Runtime trap |
| `EntryNotFound` | Named export not in module |
| `MemoryTooLarge` | Module requests more than `MAX_MEM_PAGES` pages |
| `PoolFull` | All `MAX_INSTANCES` slots are occupied |
| `ImportNotFound` | Module imports a function not in the host registry |

---

## Interpreter (`interp.rs`)

A stack machine with all state in fixed-size arrays — no heap, no `alloc`.
The `Interpreter<'a>` struct borrows body slices and linear memory from
the caller; it does not own them.

### Data Structures

```
vstack:    [i64; 256]              — value stack (i32 values sign-extended to i64)
frames:    [Frame; 128]            — call stack
ctrl:      [CtrlFrame; 64]         — control stack (block/loop/if nesting)
mem:       &'a mut [u8]            — linear memory (slice into SLOT_MEM)
host_fns:  [Option<HostFn>; 32]   — pre-resolved host function pointers
globals:   [i64; 32]              — global variables
table:     [u32; 256]             — function reference table (call_indirect)
```

**`Frame`**

```rust
struct Frame {
    body_idx:     usize,          // index into function body table
    pc:           usize,          // program counter within body slice
    locals:       [i64; 16],      // params + declared locals (i32 sign-extended)
    local_count:  usize,
    ctrl_base:    usize,          // ctrl_depth at frame entry (restored on return)
    vsp_base:     usize,          // vsp after params moved into locals
    result_count: usize,          // number of return values
}
```

**`CtrlFrame`**

```rust
struct CtrlFrame {
    kind:     BlockKind,   // Block | Loop | If
    pc_start: usize,       // loop restart target (top of loop body)
    end_pc:   usize,       // position of the matching `end` opcode
}
```

### Capacity Constants

| Constant | Value | Meaning |
|---|---|---|
| `STACK_DEPTH` | 256 | Maximum value stack entries |
| `CALL_DEPTH` | 128 | Maximum call stack depth (frames) |
| `MAX_CTRL_DEPTH` | 64 | Maximum nested blocks/loops/ifs across all frames |
| `MAX_FUNCS` | 32 | Maximum defined + imported functions |
| `MAX_TYPES` | 16 | Maximum type section entries |
| `MAX_LOCALS` | 16 | Maximum locals per function (params + declared) |
| `MAX_GLOBALS` | 32 | Maximum global variables |
| `MAX_TABLE` | 256 | Maximum function table entries (for `call_indirect`) |

### Value Stack

All values are stored as `i64`. `i32` operations sign-extend their results
so that `i32` and `i64` values coexist on the same stack without tagging.
`i32.wrap_i64` and `i64.extend_i32_s/u` perform explicit conversions.

### Host Dispatch

`host_fns[i]` holds the pre-resolved `HostFn` for import index `i`.
At `call N` where `N < import_count`, the interpreter does:

```rust
let host = self.host_fns[callee].ok_or(ImportNotFound)?;
host(&mut self.vstack, &mut self.vsp, &mut *self.mem)?;
```

There is no runtime name lookup — all imports are resolved at `spawn` time.

### Control Flow

Block entry (`block`, `loop`, `if`) calls `scan_block_end(body, start)` which
scans forward over all immediates to find the matching `end` and, for `if`,
the optional `else`. The result is stored in a `CtrlFrame` pushed onto `ctrl`.

`br N` and `br_if N` call `do_br(depth)`:
- `Block` / `If`: jump to `end_pc + 1` (past the `end`)
- `Loop`: jump to `pc_start` (back to the top of the loop body)

`br_table` reads a vector of label depths plus a default, pops a selector,
and dispatches to the matching depth.

`return` pops the current frame; if no frames remain, execution is complete.

### Multi-Value Returns

The type section is parsed to record `param_count` and `result_count` per
type. `push_frame` reads `param_count` values off the stack into
`frame.locals`. On `end`/`return`, `result_count` values are preserved on
the stack and `vsp` is restored to `vsp_base + result_count`.

### Supported Opcodes

| Category | Opcodes |
|---|---|
| Control | `unreachable` `nop` `block` `loop` `if` `else` `end` `br` `br_if` `br_table` `return` |
| Calls | `call` `call_indirect` |
| Stack | `drop` `select` |
| Locals | `local.get` `local.set` `local.tee` |
| Globals | `global.get` `global.set` |
| Memory | `i32.load` `i64.load` `i32.load8_u` `i32.store` `i64.store` `i32.store8` `memory.size` `memory.grow` |
| i32 const/arith | `i32.const` `i32.add` `i32.sub` `i32.mul` `i32.div_s` `i32.div_u` `i32.rem_s` `i32.rem_u` |
| i32 bitwise | `i32.and` `i32.or` `i32.xor` `i32.shl` `i32.shr_s` `i32.shr_u` `i32.rotl` `i32.rotr` `i32.clz` `i32.ctz` `i32.popcnt` |
| i32 compare | `i32.eqz` `i32.eq` `i32.ne` `i32.lt_s` `i32.lt_u` `i32.gt_s` `i32.gt_u` `i32.le_s` `i32.le_u` `i32.ge_s` `i32.ge_u` |
| i64 const/arith | `i64.const` `i64.add` `i64.sub` `i64.mul` |
| i64 bitwise | `i64.and` `i64.or` `i64.xor` `i64.shl` `i64.shr_s` `i64.shr_u` `i64.rotl` `i64.rotr` `i64.clz` `i64.ctz` `i64.popcnt` |
| i64 compare | `i64.eqz` `i64.eq` `i64.ne` `i64.lt_s` `i64.lt_u` `i64.gt_s` `i64.gt_u` `i64.le_s` `i64.le_u` `i64.ge_s` `i64.ge_u` |
| Conversions | `i32.wrap_i64` `i64.extend_i32_s` `i64.extend_i32_u` |

`memory.grow` always returns -1 (fixed-size memory, no growth supported).

### `InterpError` Variants

| Variant | Meaning |
|---|---|
| `NoCodeSection` | Module has no code section |
| `MalformedCode` | Bytecode parse failure |
| `TooManyFuncs` / `TooManyTypes` / `TooManyLocals` / `TooManyGlobals` | Exceeded fixed capacity |
| `CtrlStackOverflow` | Block nesting deeper than `MAX_CTRL_DEPTH` |
| `FuncIndexOutOfRange` | Call to undefined function index |
| `LocalIndexOutOfRange` / `GlobalIndexOutOfRange` | Bad variable index |
| `GlobalImmutable` | Write to a const global |
| `IndirectCallNull` | `call_indirect` on a null table entry |
| `IndirectCallTypeMismatch` | `call_indirect` type check failed |
| `ImportNotFound` | Call to an import that was not resolved at instantiation |
| `Unreachable` | `unreachable` opcode executed |
| `StackOverflow` / `StackUnderflow` | Value stack bounds violated |
| `CallStackOverflow` | Call depth exceeded `CALL_DEPTH` |
| `MemOutOfBounds` | Linear memory access outside allocated region |
| `DivisionByZero` | Integer division or remainder by zero |
| `UnknownOpcode(u8)` | Opcode not implemented |
