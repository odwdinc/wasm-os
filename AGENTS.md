# AGENTS.md

<p align="center">
  <strong>WASM-First Bare Metal OS</strong><br/>
  <em>Minimal kernel. WebAssembly as the system ABI.</em>
</p>

<p align="center">
  <img src="https://img.shields.io/badge/status-MVP-blue" />
  <img src="https://img.shields.io/badge/runtime-WASM-green" />
  <img src="https://img.shields.io/badge/language-Rust-orange" />
  <img src="https://img.shields.io/badge/platform-x86__64-lightgrey" />
</p>


---

├── Cargo.toml                # Workspace root
├── rust-toolchain.toml       # Pin nightly toolchain
├── README.md
├── AGENTS.md
├── CONTRIBUTING.md
├── LICENSE (optional)
│
├── /kernel                   # Bare metal kernel (no_std)
│   ├── Cargo.toml
│   └── src/
│       ├── main.rs           # Entry point
│       ├── boot.rs           # Boot/init logic
│       ├── memory/
│       │   ├── mod.rs
│       │   ├── allocator.rs
│       │   └── paging.rs
│       ├── interrupts/
│       │   ├── mod.rs
│       │   ├── idt.rs
│       │   └── handlers.rs
│       ├── drivers/
│       │   ├── mod.rs
│       │   ├── vga.rs        # Text output
│       │   └── keyboard.rs   # Input
│       └── util/
│           └── mod.rs
│
├── /runtime                  # WASM runtime (core of system)
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs
│       ├── module.rs         # WASM module loader
│       ├── interpreter.rs    # Execution loop
│       ├── stack.rs
│       ├── memory.rs         # Linear memory model
│       ├── instructions/
│       │   ├── mod.rs
│       │   ├── control.rs
│       │   ├── numeric.rs
│       │   └── memory.rs
│       └── host/
│           ├── mod.rs
│           ├── api.rs        # Host function definitions
│           └── bindings.rs   # Glue to kernel
│
├── /shell                    # Terminal + command system
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs
│       ├── shell.rs          # REPL loop
│       ├── parser.rs         # Command parsing
│       ├── commands/
│       │   ├── mod.rs
│       │   ├── help.rs
│       │   ├── echo.rs
│       │   ├── ls.rs
│       │   ├── cat.rs
│       │   └── run.rs        # Execute WASM
│       └── input.rs          # Line editing
│
├── /fs                       # Filesystem layer
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs
│       ├── fs.rs             # Core FS logic
│       ├── file.rs
│       ├── directory.rs
│       └── ramfs.rs          # In-memory FS (MVP)
│
├── /shared                   # Shared types/interfaces
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs
│       ├── error.rs
│       ├── types.rs
│       └── constants.rs
│
├── /userland                 # Example WASM programs
│   ├── hello/
│   │   ├── hello.wat
│   │   └── build.sh
│   └── README.md
│
├── /tools                    # Dev + build tooling
│   ├── build-image.sh
│   ├── run-qemu.sh
│   └── wasm-pack.sh
│
├── /scripts                  # Helper scripts
│   ├── setup.sh
│   └── dev-env.sh
│
├── /docs                     # Design docs
│   ├── architecture.md
│   ├── wasm-runtime.md
│   └── roadmap.md
│
└── /tests                    # Host-side tests (std)
    ├── runtime_tests.rs
    └── fs_tests.rs

---

# 🧠 Overview

This project is a **research operating system** that runs **WebAssembly (WASM) as the primary execution environment on bare metal**.

Instead of traditional OS design:

* ❌ No userland binaries
* ❌ No POSIX/syscall model
* ❌ No strict ring3 abstraction boundary

We use:

* ✅ WASM modules as the unit of execution
* ✅ VM-based isolation (memory-safe sandboxing)
* ✅ Host function interfaces instead of syscalls

---

# 🎯 Goals

## Near-Term (MVP+)

* Stable WASM interpreter
* File-based module loading
* Expand host function interface
* Improve terminal UX

## Mid-Term

* Capability-based security model
* Persistent filesystem
* Multi-module execution
* Async I/O model

## Long-Term

* JIT compilation
* Self-hosted toolchain
* Networking stack
* WASM-driven GUI

---

# 🏗️ Architecture

## Layered Design

### 1. Kernel (`no_std`, Rust)

Handles:

* Boot & initialization
* Memory management
* Interrupts
* Basic device I/O

Constraints:

* Minimal surface area
* Deterministic behavior preferred
* Unsafe code must be documented

---

### 2. Terminal / Shell

Responsibilities:

* Text rendering
* Input handling
* Command parsing
* Launching WASM modules

Example:

```
> ls
> run hello.wasm
```

---

### 3. WASM Runtime

Current:

* Minimal interpreter

Responsibilities:

* Load + validate modules
* Execute instructions
* Interface with host functions

Planned:

* Broader spec support
* Optional JIT

---

### 4. Host Interface (Syscall Replacement)

All system interaction happens via imports.

Example:

```wat
(import "os" "print" (func $print (param i32 i32)))
```

Rules:

* No implicit access
* Capabilities must be explicitly provided
* Keep APIs minimal and composable

---

### 5. Filesystem

Current:

* In-memory

Planned:

* Persistent disk-backed FS

Responsibilities:

* Store `.wasm` modules
* Provide file APIs to runtime

---

# 🚀 Getting Started

## Requirements

* Rust (nightly recommended)
* `x86_64-unknown-none` target
* QEMU

---

## Build & Run

```bash
# build
cargo build

# run in qemu
cargo run
```

Expected:

```
> _
```

---

# 🧩 Contribution Guide

## Areas You Can Work On

### 🔹 Kernel

* Memory allocator
* Interrupt handling
* Device drivers

### 🔹 WASM Runtime

* Instruction support
* Execution engine
* Memory correctness

### 🔹 Host Interface

* API design
* Capability model
* Safety boundaries

### 🔹 Filesystem

* Data structures
* Persistence layer
* File APIs

### 🔹 Tooling

* Build pipeline
* Debugging tools
* Dev UX improvements

---

# ⚙️ Development Rules

1. **System must always boot**
2. **Terminal must remain functional**
3. Prefer **small, incremental PRs**
4. Avoid unnecessary dependencies
5. Keep abstractions minimal

---

# 🧪 Testing

## Required

* Boots in QEMU
* Terminal accepts input
* Existing commands work
* WASM execution still functions

## Recommended

* Unit tests (outside kernel)
* Serial logging for debugging

---

# 🧱 Coding Standards

## Rust

* `#![no_std]` in kernel
* Minimize `unsafe`
* Document all unsafe blocks

## Style

* Explicit naming
* Small functions
* Avoid hidden globals

---

# 🤖 AI Agent Guidelines

This repo is designed to be AI-contributor-friendly.

---

## ✅ Allowed

* Implement small, well-scoped features
* Add WASM instructions
* Improve error handling
* Refactor for clarity
* Add tests

---

## ❌ Not Allowed

* Large architectural rewrites without discussion
* Breaking boot or terminal
* Introducing heavy dependencies
* Changing ABI without documentation

---

## 🧭 Task Strategy (For Agents)

1. Identify target layer:

   * kernel / runtime / FS / shell
2. Limit scope strictly
3. Implement minimal working version
4. Validate via QEMU
5. Leave TODOs instead of overbuilding

---

## 📣 Communication Expectations

Agents should:

* Explain tradeoffs briefly
* Highlight unsafe code
* Note performance implications
* Avoid speculative complexity

---

# 🗺️ Roadmap

## Phase 1 (Current)

* Boot → terminal
* WASM execution (basic)

## Phase 2

* Filesystem integration
* Runtime expansion

## Phase 3

* Capability system
* Async execution

## Phase 4

* JIT + performance work
* Self-hosting exploration

---

# 💡 Philosophy

This is an **experimental systems project**.

We are not trying to:

* replicate Linux
* support POSIX
* maximize compatibility

We are trying to:

* rethink OS boundaries
* simplify execution models
* explore WASM as a system interface

---

# ✅ Definition of Done (MVP)

* Boot to terminal
* Accept input
* Load `.wasm`
* Execute module
* Print output

---

# 🤝 Contributing

1. Fork the repo
2. Create a branch
3. Make small, focused changes
4. Test in QEMU
5. Open a PR

---

# 📌 Final Principle

> Working systems > perfect designs

Iterate quickly. Keep it bootable. Build upward.