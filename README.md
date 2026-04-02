# 🚀 WASM-First OS

> A research operating system that runs **WebAssembly as the primary execution environment on bare metal**

---

## 🧠 What is this?

**WASM-First OS** is an experimental operating system that explores a different model of computing:

* 🧩 **WebAssembly as the system ABI** (not POSIX, not native binaries)
* 🔒 **VM-based isolation** instead of traditional user/kernel separation
* ⚡ **Host functions** instead of syscalls
* 🧱 **Minimal kernel**, with most logic pushed into a runtime layer

In short:

> What if the OS *was* a WebAssembly runtime?

---

## ✨ [Current Status (MVP)](MVP_Agile_plan.md)

---

## 🎯 Project Goals

### Near-Term

* Expand WASM interpreter support
* File-based module loading
* Improve terminal UX
* Stabilize runtime execution

### Mid-Term

* Capability-based security model
* Persistent filesystem
* Multi-module execution
* Async I/O

### Long-Term

* JIT compilation
* Self-hosted toolchain
* Networking stack
* WASM-driven GUI

---

## 🏗️ Architecture (High-Level)

```
+-----------------------------+
|     WASM Applications       |
+-----------------------------+
|     WASM Runtime (VM)       |
|  - Interpreter / (future JIT)
|  - Host function interface  |
+-----------------------------+
|   Minimal Kernel (Rust)     |
|  - Memory                   |
|  - Interrupts               |
|  - Devices                  |
+-----------------------------+
|         Hardware            |
+-----------------------------+
```

---

## 🚀 Getting Started

### Requirements

* Rust (nightly recommended)
* `x86_64-unknown-none` target
* QEMU
```bash
# setup the build env
./scripts/setup.sh
./scripts/dev-env.sh
```
---

### Build & Run

```bash
# build the OS image
./tools/build-image.sh

# run in QEMU
./tools/run-qemu.sh
```

---

### Expected Output

```text
> _
```

---

## 🧪 Running a WASM Module

Once booted:

```text
> ls
hello.wasm

> run hello.wasm
Hello from WASM!
>
```

---

## 🛠️ Example WASM (WAT)

```wat
(module
  (import "os" "print" (func $print (param i32 i32)))
  (memory 1)
  (data (i32.const 0) "Hello from WASM!")

  (func (export "main")
    (call $print (i32.const 0) (i32.const 17))
  )
)
```

---

## 🧩 Why This Exists

Traditional OS design assumes:

* processes
* syscalls
* privilege rings

This project questions those assumptions:

* Can **WASM replace the syscall boundary**?
* Can we simplify isolation using **VM guarantees**?
* What does a system look like if **everything runs in a sandbox by default**?

---

## 🤝 Contributing

We’re actively looking for contributors interested in:

* 🧠 OS design experiments
* ⚙️ Runtime / VM engineering
* 🦀 Rust systems programming
* 🧩 Rethinking system abstractions

Start here:

* Read `AGENTS.md`
* Pick a subsystem (kernel, runtime, FS, shell)
* Open an issue or PR

---

## 🧭 Areas of Work

* Kernel (memory, interrupts, drivers)
* WASM runtime (interpreter, execution engine)
* Host API (capabilities, safety)
* Filesystem (in-memory → persistent)
* Tooling (build, debug, dev UX)

---

## ⚠️ Project Status

This is a **research prototype**, not a production OS.

Expect:

* breaking changes
* incomplete features
* evolving architecture

---

## 💡 Philosophy

> Working systems > perfect designs

We prioritize:

* small iterations
* real execution
* learning through building

---

## 📜 License

TBD

---

## 🔥 Join Us

If you’ve ever wanted to:

* build an OS
* design a runtime
* question how systems *should* work

This is a good place to do it.

Let’s build something weird.
