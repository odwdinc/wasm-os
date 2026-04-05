# Roadmap

## Completed Sprints

| Sprint | Focus | Status |
|---|---|---|
| 1 | Bootable kernel, framebuffer, keyboard | Done |
| 2 | WASM loader, basic interpreter, `hello.wasm` | Done |
| 3 | In-memory FS, export parsing, shell commands (`ls`, `info`, `run`) | Done |
| 4 | Full control flow, locals, memory ops, i32 arithmetic/comparison | Done |
| A | WASM spec completeness — i64, globals, `call_indirect`, `br_table`, missing opcodes | Done |
| B | Runtime isolation — instance pool, named host registry, `ps` command | Done |
| C | Preemptive scheduling — PIT timer, task queue, `task-run`/`task-kill`/`tasks` | Done |
| D | Persistent filesystem — virtio-blk driver, FAT12/16/32, shell FS commands | Done |

Exit condition for FAT/filesystem sprint:
```
> ls
  hello.wasm               1234 bytes
> df
Filesystem    Size (K)  Used (K)  Avail (K)
FAT              32768        12     32756
> edit notes.txt
-- new file: notes.txt ---
Append lines.  Commands: :w = save  :q = quit
> hello world
> :w
saved notes.txt (12 bytes)
> cat notes.txt
hello world
```

---

## Sprint E — Networking

A WASM module opens TCP connections and sends/receives data.

**Tasks:**
- virtio-net driver
- TCP/IP stack (smoltcp or minimal hand-written)
- Socket host functions (`"net"."connect"`, `"net"."send"`, `"net"."recv"`, `"net"."close"`)
- `httpd.wasm` demo module

**Done when:** a WASM module fetches a URL and prints the response body.

---

## Sprint F — JIT Compilation

Compile WASM functions to native x86_64 at instantiation time.

**Tasks:**
- Machine code emitter into a static executable buffer
- Basic codegen for arithmetic, locals, control flow
- Tiered execution: interpret → JIT after N calls

**Done when:** `fib(35)` runs measurably faster under JIT.

---

## Sprint G — In-OS WAT Assembler

Write WAT source inside the OS, assemble it to binary, and run it — no host
toolchain required.

**Tasks:**
- WAT tokenizer + parser
- WASM binary emitter
- `asm <file.wat>` shell command (compile to `.wasm` in place)

---

## Dependency Graph

```
Sprints 1-4 (MVP)
    └── Sprint A: WASM Spec Completeness
            ├── Sprint B: Runtime Isolation
            │       └── Sprint C: Preemptive Scheduling
            │               └── Sprint E: Networking ──┐
            ├── Sprint D: Persistent FS ───────────────┘
            │       └── Sprint G: WAT Assembler
            └── Sprint F: JIT Compilation
```
