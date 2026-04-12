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
| `primes/primes.wat` | → `primes.wasm` | Sieve of Eratosthenes up to N |
| `counter/counter.wat` | → `counter.wasm` | Counting loop with cooperative yield |
| `collatz/collatz.wat` | → `collatz.wasm` | Collatz sequence for a given starting value |
| `httpd/httpd.wat` | → `httpd.wasm` | Minimal HTTP/1.0 server on `:8080`; responds with "Hello from WASM-First OS!" |

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

All imports are resolved by `(module, name)` at instantiation time.

### `"env"` — kernel services

| Import | Signature | Description |
|---|---|---|
| `"env"."print"` | `(param i32 i32)` | Print UTF-8 string from linear memory (ptr, len) |
| `"env"."print_int"` | `(param i32)` | Print i32 as decimal + newline |
| `"env"."print_i64"` | `(param i64)` | Print i64 as decimal + newline |
| `"env"."print_char"` | `(param i32)` | Print low byte as a single ASCII character |
| `"env"."print_hex"` | `(param i32)` | Print i32 as `0x` + 8 uppercase hex digits + newline |
| `"env"."yield"` | `()` | Yield to the cooperative scheduler |
| `"env"."sleep_ms"` | `(param i32)` | Yield for at least N milliseconds |
| `"env"."uptime_ms"` | `() → i32` | Milliseconds since boot |
| `"env"."exit"` | `(param i32)` | Terminate the module cleanly |
| `"env"."read_char"` | `() → i32` | Block until a key is pressed; returns ASCII code (Enter = 10) |
| `"env"."read_line"` | `(param i32 i32) → i32` | Read a line into memory (ptr, cap); returns byte count or -1 |
| `"env"."args_get"` | `(param i32 i32) → i32` | Copy run-time args into memory (ptr, cap); returns byte count or -1 |
| `"env"."fs_read"` | `(param i32 i32 i32 i32) → i32` | Read file into memory (name_ptr, name_len, buf_ptr, buf_cap); returns byte count or -1 |
| `"env"."fs_write"` | `(param i32 i32 i32 i32) → i32` | Write bytes to a file (name_ptr, name_len, buf_ptr, buf_len); returns 0 or -1 |
| `"env"."fs_size"` | `(param i32 i32) → i32` | Return file size in bytes (name_ptr, name_len), or -1 if not found |
| `"env"."fb_set_pixel"` | `(param i32 i32 i32)` | Write one pixel to the framebuffer (x, y, rgb as 0x00RRGGBB) |
| `"env"."fb_present"` | `()` | Present the framebuffer (no-op; reserved for double-buffering) |
| `"env"."fb_blit"` | `(param i32 i32 i32)` | Blit a packed 0x00RRGGBB pixel buffer from WASM memory to the framebuffer (ptr, width, height) |

### `"net"` — TCP/IP networking

Non-blocking by design: `accept` and `recv`/`udp_recv` return -1 or 0 immediately when no data is available. Callers should `yield` and retry.

| Import | Signature | Description |
|---|---|---|
| `"net"."listen"` | `(param i32) → i32` | TCP listen on port; returns listen-socket handle or -1 |
| `"net"."connect"` | `(param i32 i32) → i32` | TCP active connect (ip_u32_le, port); returns handle or -1 |
| `"net"."accept"` | `(param i32) → i32` | Accept pending connection; returns conn handle or -1 if none ready |
| `"net"."recv"` | `(param i32 i32 i32) → i32` | Receive into memory (handle, ptr, cap); returns byte count, 0=no data, -1=error |
| `"net"."send"` | `(param i32 i32 i32) → i32` | Send from memory (handle, ptr, len); returns bytes sent or -1 |
| `"net"."close"` | `(param i32) → i32` | Close TCP connection; always returns 0 |
| `"net"."status"` | `(param i32) → i32` | Socket state: 0=closed, 1=listen, 2=handshaking, 3=established, 4=teardown |
| `"net"."get_ip"` | `() → i32` | Kernel IP as u32 little-endian (0 if DHCP not yet bound) |
| `"net"."set_ip"` | `(param i32) → i32` | Manually set the kernel IP (ip_u32_le); always returns 0 |
| `"net"."udp_bind"` | `(param i32) → i32` | Bind UDP socket to port; returns handle or -1 |
| `"net"."udp_connect"` | `(param i32 i32 i32) → i32` | Set UDP remote (handle, ip_u32_le, port); returns 0 or -1 |
| `"net"."udp_send"` | `(param i32 i32 i32) → i32` | Send UDP datagram (handle, ptr, len); returns bytes sent or -1 |
| `"net"."udp_recv"` | `(param i32 i32 i32) → i32` | Receive UDP datagram (handle, ptr, cap); returns byte count or 0 (non-blocking) |
| `"net"."udp_close"` | `(param i32) → i32` | Close UDP socket; always returns 0 |

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

- Linear memory: up to `MAX_MEM_PAGES = 16` pages × 64 KiB = **1 MiB per instance**; initial size comes from the module's memory section
- Maximum 32 locals per function (params + declared locals)
- Maximum call depth: 128 frames
- No heap inside modules — use linear memory directly
