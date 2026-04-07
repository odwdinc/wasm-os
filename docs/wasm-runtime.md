# WASM Runtime Internals

The runtime lives in `kernel/src/wasm/` and consists of four files:
`loader.rs`, `engine.rs`, `interp.rs`, and `task.rs`.

---

## Loader (`loader.rs`)

Parses a WASM binary into a `Module<'_>` — zero-copy slices into the
input buffer.  No allocation; all fields are `Option<&'a [u8]>` pointing
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

Custom sections (ID 0) and any other unknown IDs are silently skipped.

**Public functions:**

| Function | Description |
|---|---|
| `load(bytes) -> Result<Module, LoadError>` | Parse header + sections; zero-copy |
| `find_export(module, name) -> Option<u32>` | Scan export section for a function export; returns absolute function index |
| `for_each_func_import(section, f)` | Iterate function imports calling `f(module_name, func_name)` |
| `read_memory_min_pages(section) -> u32` | Parse memory section; return `min` page count (0 if absent) |
| `read_u32_leb128(bytes) -> Option<(u32, usize)>` | Unsigned 32-bit LEB-128 |

---

## Engine (`engine.rs`)

Owns the host function registry, the instance pool, and per-instance
linear memory.  All state lives in `static mut` — no heap allocation.

### Host Function Registry

```
MAX_HOST_FUNCS = 32
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
| `"print_i64"` | `(param i64)` | Print i64 as decimal + newline |
| `"print_char"` | `(param i32)` | Print low byte as a single ASCII character |
| `"print_hex"` | `(param i32)` | Print i32 as `0x` + 8 uppercase hex digits + newline |
| `"yield"` | `()` | Yield to the scheduler (cooperative multitasking) |
| `"sleep_ms"` | `(param i32)` | Yield for at least N milliseconds |
| `"uptime_ms"` | `() → i32` | Milliseconds since boot (PIT ticks × 10) |
| `"exit"` | `(param i32)` | Terminate the module cleanly |
| `"read_char"` | `() → i32` | Block until a key is pressed; returns ASCII code (Enter = 10) |
| `"read_line"` | `(param i32 i32) → i32` | Read a line into memory (ptr, cap); echoes input; returns byte count or -1 |
| `"fs_read"` | `(param i32 i32 i32 i32) → i32` | Read file into memory (name_ptr, name_len, buf_ptr, buf_cap); returns byte count or -1 |
| `"fs_write"` | `(param i32 i32 i32 i32) → i32` | Write bytes to a file (name_ptr, name_len, buf_ptr, buf_len); returns 0 or -1 |
| `"fs_size"` | `(param i32 i32) → i32` | Return file size in bytes (name_ptr, name_len), or -1 if not found |
| `"args_get"` | `(param i32 i32) → i32` | Copy space-joined run args into memory (ptr, cap); returns byte count or -1 |

### Instance Pool

```
MAX_INSTANCES = 4
MAX_MEM_PAGES = 16   (1 page = 64 KiB → 1 MiB per slot)
```

Each pool slot has a dedicated `1 MiB` static memory region
(`SLOT_MEM[slot]`).  Only `min_pages * 65536` bytes are initially active;
`memory.grow` can grow up to `max_pages` within that limit.  Slots are
zeroed on `spawn` and on `destroy`.

**Public API:**

| Function | Description |
|---|---|
| `init_host_fns()` | Register kernel built-ins — call once at boot |
| `register_host(module, name, fn)` | Add an entry to the host registry |
| `set_args(args)` | Store the argument string for the next `args_get` call |
| `take_pending_sleep_ms() -> u32` | Consume the sleep duration set by the last `sleep_ms` call |
| `spawn(name, bytes) -> Result<usize, RunError>` | Instantiate into pool, return handle |
| `destroy(handle)` | Drop instance, zero slot memory |
| `for_each_instance(f)` | Iterate active slots: `f(handle, name, mem_pages)` |
| `start_task(handle, entry, args) -> Result<TaskResult, RunError>` | Begin executing `entry` on a spawned instance |
| `resume_task(handle) -> Result<TaskResult, RunError>` | Continue a suspended (yielded) task |
| `run(bytes, entry, args)` | Convenience: spawn + start + drain yields + destroy |
| `count_func_imports(import_section) -> usize` | Count function imports in a section |

**`spawn` sequence:**

1. Find a free pool slot (error: `PoolFull`)
2. `loader::load(bytes)` — parse module header and sections
3. `read_memory_min_pages` — validate against `MAX_MEM_PAGES` (error: `MemoryTooLarge`)
4. Zero `SLOT_MEM[slot]`, take `&'static mut [u8]` slice
5. Resolve all function imports against the host registry (error: `ImportNotFound`)
6. `Interpreter::new(module, import_count, mem, host_fns, min_pages)`
7. Apply data-section initializers to linear memory
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
the engine pool slot; it does not own them.

### Data Structures

```
vstack:    [i64; 256]              — value stack (i32 sign-extended to i64)
frames:    [Frame; 128]            — call stack
ctrl:      [CtrlFrame; 64]         — control stack (block/loop/if nesting)
mem:       &'a mut [u8]            — linear memory (slice into SLOT_MEM)
host_fns:  [Option<HostFn>; 512]  — pre-resolved host function pointers
globals:   [i64; 64]              — global variables
table:     [u32; 512]             — function reference table (call_indirect)
```

**`Frame`**

```rust
struct Frame {
    body_idx:     usize,          // index into function body table
    pc:           usize,          // program counter within body slice
    locals:       [i64; 32],      // params + declared locals (i32 sign-extended)
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
| `MAX_FUNCS` | 512 | Maximum defined + imported functions |
| `MAX_TYPES` | 128 | Maximum type section entries |
| `MAX_LOCALS` | 32 | Maximum locals per function (params + declared) |
| `MAX_GLOBALS` | 64 | Maximum global variables |
| `MAX_TABLE` | 512 | Maximum function table entries (for `call_indirect`) |

### Value Stack

All values are stored as `i64`.  `i32` operations sign-extend their
results so that `i32` and `i64` values coexist on the same stack.
`i32.wrap_i64` and `i64.extend_i32_s/u` perform explicit conversions.

### Host Dispatch

`host_fns[i]` holds the pre-resolved `HostFn` for import index `i`.
At `call N` where `N < import_count`, the interpreter does:

```rust
let host = self.host_fns[callee].ok_or(ImportNotFound)?;
host(&mut self.vstack, &mut self.vsp, &mut *self.mem)?;
```

There is no runtime name lookup — all imports are resolved at `spawn` time.

### Memory Growth

`memory.grow` grows the active page count up to `max_pages` (the physical
size of the pool slot, i.e. `MAX_MEM_PAGES`).  New pages are pre-zeroed.
Returns the previous page count on success, `-1` if the requested growth
would exceed `max_pages`.

### Control Flow

Block entry (`block`, `loop`, `if`) calls `scan_block_end(body, start)` which
scans forward over all immediates to find the matching `end` and, for `if`,
the optional `else`.  The result is stored in a `CtrlFrame` pushed onto `ctrl`.

`br N` and `br_if N` call `do_br(depth)`:
- `Block` / `If`: jump to `end_pc + 1` (past the `end`)
- `Loop`: jump to `pc_start` (back to the top of the loop body)

`br_table` reads a vector of label depths plus a default, pops a selector,
and dispatches to the matching depth.

`return` pops the current frame; if no frames remain, execution is complete.

### Multi-Value Returns

The type section is parsed to record `param_count` and `result_count` per
type.  `push_frame` reads `param_count` values off the stack into
`frame.locals`.  On `end`/`return`, `result_count` values are preserved on
the stack and `vsp` is restored to `vsp_base + result_count`.

### Signed LEB-128 Helpers (in `interp.rs`)

| Function | Description |
|---|---|
| `read_i32_leb128(bytes) -> Option<(i32, usize)>` | Signed 32-bit (data-segment offsets, `i32.const`) |
| `read_i64_leb128(bytes) -> Option<(i64, usize)>` | Signed 64-bit (`i64.const`, global init exprs) |

### Supported Opcodes

| Category | Opcodes |
|---|---|
| Control | `unreachable` `nop` `block` `loop` `if` `else` `end` `br` `br_if` `br_table` `return` |
| Calls | `call` `call_indirect` |
| Stack | `drop` `select` |
| Locals | `local.get` `local.set` `local.tee` |
| Globals | `global.get` `global.set` |
| Memory loads (i32) | `i32.load` `i32.load8_s` `i32.load8_u` `i32.load16_s` `i32.load16_u` |
| Memory loads (i64) | `i64.load` `i64.load8_s` `i64.load8_u` `i64.load16_s` `i64.load16_u` `i64.load32_s` `i64.load32_u` |
| Memory loads (f32/f64) | `f32.load` `f64.load` |
| Memory stores (i32) | `i32.store` `i32.store8` `i32.store16` |
| Memory stores (i64) | `i64.store` `i64.store8` `i64.store16` `i64.store32` |
| Memory stores (f32/f64) | `f32.store` `f64.store` |
| Memory misc | `memory.size` `memory.grow` |
| i32 const/arith | `i32.const` `i32.add` `i32.sub` `i32.mul` `i32.div_s` `i32.div_u` `i32.rem_s` `i32.rem_u` |
| i32 bitwise | `i32.and` `i32.or` `i32.xor` `i32.shl` `i32.shr_s` `i32.shr_u` `i32.rotl` `i32.rotr` `i32.clz` `i32.ctz` `i32.popcnt` |
| i32 compare | `i32.eqz` `i32.eq` `i32.ne` `i32.lt_s` `i32.lt_u` `i32.gt_s` `i32.gt_u` `i32.le_s` `i32.le_u` `i32.ge_s` `i32.ge_u` |
| i32 sign-extend | `i32.extend8_s` `i32.extend16_s` |
| i64 const/arith | `i64.const` `i64.add` `i64.sub` `i64.mul` `i64.div_s` `i64.div_u` `i64.rem_s` `i64.rem_u` |
| i64 bitwise | `i64.and` `i64.or` `i64.xor` `i64.shl` `i64.shr_s` `i64.shr_u` `i64.rotl` `i64.rotr` `i64.clz` `i64.ctz` `i64.popcnt` |
| i64 compare | `i64.eqz` `i64.eq` `i64.ne` `i64.lt_s` `i64.lt_u` `i64.gt_s` `i64.gt_u` `i64.le_s` `i64.le_u` `i64.ge_s` `i64.ge_u` |
| i64 sign-extend | `i64.extend8_s` `i64.extend16_s` `i64.extend32_s` |
| f32 arith | `f32.abs` `f32.neg` `f32.ceil` `f32.floor` `f32.trunc` `f32.nearest` `f32.sqrt` `f32.add` `f32.sub` `f32.mul` `f32.div` `f32.min` `f32.max` `f32.copysign` |
| f32 compare | `f32.eq` `f32.ne` `f32.lt` `f32.gt` `f32.le` `f32.ge` |
| f64 arith | `f64.abs` `f64.neg` `f64.ceil` `f64.floor` `f64.trunc` `f64.nearest` `f64.sqrt` `f64.add` `f64.sub` `f64.mul` `f64.div` `f64.min` `f64.max` `f64.copysign` |
| f64 compare | `f64.eq` `f64.ne` `f64.lt` `f64.gt` `f64.le` `f64.ge` |
| Conversions (int↔int) | `i32.wrap_i64` `i64.extend_i32_s` `i64.extend_i32_u` |
| Conversions (float→int) | `i32.trunc_f32_s/u` `i32.trunc_f64_s/u` `i64.trunc_f32_s/u` `i64.trunc_f64_s/u` |
| Conversions (int→float) | `f32.convert_i32_s/u` `f32.convert_i64_s/u` `f64.convert_i32_s/u` `f64.convert_i64_s/u` |
| Demote/promote | `f32.demote_f64` `f64.promote_f32` |
| Reinterpret | `i32.reinterpret_f32` `i64.reinterpret_f64` `f32.reinterpret_i32` `f64.reinterpret_i64` |
| Saturating trunc (`0xFC` prefix) | `i32.trunc_sat_f32_s/u` `i32.trunc_sat_f64_s/u` `i64.trunc_sat_f32_s/u` `i64.trunc_sat_f64_s/u` |

### `InterpError` Variants

| Variant | Meaning |
|---|---|
| `NoCodeSection` | Module has no code section |
| `MalformedCode` | Bytecode parse failure |
| `TooManyFuncs` / `TooManyTypes` / `TooManyLocals` / `TooManyGlobals` | Exceeded fixed capacity |
| `CtrlStackOverflow` | Block nesting deeper than `MAX_CTRL_DEPTH` |
| `FuncIndexOutOfRange` | Call to undefined function index |
| `LocalIndexOutOfRange` / `GlobalIndexOutOfRange` | Bad variable index |
| `GlobalImmutable` | Write to a `const` global |
| `IndirectCallNull` | `call_indirect` on a null table entry |
| `IndirectCallTypeMismatch` | `call_indirect` type check failed |
| `ImportNotFound` | Call to an import that was not resolved at instantiation |
| `Unreachable` | `unreachable` opcode executed |
| `StackOverflow` / `StackUnderflow` | Value stack bounds violated |
| `CallStackOverflow` | Call depth exceeded `CALL_DEPTH` |
| `MemOutOfBounds` | Linear memory access outside allocated region |
| `DivisionByZero` | Integer division or remainder by zero |
| `InvalidConversion` | Float-to-int conversion out of range or NaN |
| `UnknownOpcode(u8)` | Opcode not implemented |
| `Yielded` | Pseudo-error: module yielded cooperatively (not a trap) |
| `Exited` | Pseudo-error: module called `env.exit` (treated as clean return) |

---

## Task Layer (`task.rs`)

Wraps an engine pool slot with a [`TaskState`] state machine for use by
the round-robin scheduler.

**`TaskState` variants:**

| Variant | Meaning |
|---|---|
| `Ready` | Spawned; `main` not yet called |
| `Running` | Currently selected by the scheduler |
| `Suspended` | Called `env.yield`; waiting to be resumed |
| `Sleeping(u64)` | Called `env.sleep_ms`; wake when PIT tick count ≥ stored value |
| `Done` | Completed or killed |

**Public API:**

| Function | Description |
|---|---|
| `task_spawn(name, bytes) -> Result<usize, RunError>` | Instantiate + register; returns task ID |
| `task_kill(id)` | Remove task and free engine pool slot |
| `task_state(id) -> Option<TaskState>` | Query current state |
| `task_step(id) -> Option<Result<TaskResult, RunError>>` | Advance one step; start or resume |
| `is_task_runnable(id) -> bool` | True if the task can be stepped right now |
| `for_each_task(f)` | Iterate all slots: `f(id, name, state)` |
