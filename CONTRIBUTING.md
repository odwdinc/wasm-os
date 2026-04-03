# 🤝 CONTRIBUTING.md

Welcome! This project is an experimental **WASM-first operating system**, and contributions are a core part of pushing the design forward.

This guide will help you get from zero → first PR quickly and safely.

---

# 🧠 Project Mindset

This is a **research system**, not a production OS.

We value:

* Working code over perfect abstractions
* Small, testable changes
* Clear reasoning over cleverness

Expect things to evolve.

---

# 🚀 Getting Started

## 1. Prerequisites

* Rust nightly (see `rust-toolchain.toml` — toolchain is pinned)
* `x86_64-unknown-none` target: `rustup target add x86_64-unknown-none`
* `wabt` (for `wat2wasm`): `sudo apt install wabt`
* QEMU: `sudo apt install qemu-system-x86`

---

## 2. Build & Run

```bash
# Full pipeline: compile userland .wat → build kernel → launch QEMU
./tools/run-qemu.sh

# Or step by step:
./tools/wasm-pack.sh      # compile userland/*.wat → *.wasm  (requires wabt)
./tools/build-image.sh    # build kernel ELF + disk image
./tools/run-qemu.sh       # boot in QEMU
```

You should see:

```text
Hello from WASM!
Type ‘help’ for commands.
> _
```

If it boots and you can type, you’re ready.

> **Note:** Run `wasm-pack.sh` before building the kernel if you change any `.wat` files.
> The `.wasm` binaries are embedded at compile time via `include_bytes!`.

---

## 3. Read First

Before contributing:

* `README.md` → project overview
* `AGENTS.md` → architecture + rules

---

# 🧩 How to Contribute

## Step 1: Pick a Task

Good starting points (Sprint A):

* `i64` type support
* `i32.div_s`, `i32.rem_s`, `i32.shr_u`
* `memory.size` / `memory.grow`
* `br_table`
* Global variables (section ID 6)
* Bug fixes, error message improvements

See [Post_MVP_Agile_plan.md](Post_MVP_Agile_plan.md) for full task breakdown.

---

## Step 2: Create a Branch

```bash
git checkout -b feature/short-description
```

---

## Step 3: Make Small Changes

Keep PRs:

* Focused
* Reviewable
* Testable in isolation

---

## Step 4: Test in QEMU

Before submitting:

* [ ] System boots
* [ ] Terminal works
* [ ] No regressions
* [ ] Feature behaves as expected

---

## Step 5: Open a PR

Include:

* What you changed
* Why you changed it
* Any tradeoffs

---

# 📦 Contribution Areas

## 🔹 Kernel

* Memory management
* Interrupt handling
* Device drivers

## 🔹 WASM Runtime

* Instruction support
* Execution engine
* Validation

## 🔹 Host Interface

* API design
* Capability model
* Safety boundaries

## 🔹 Filesystem

* File structures
* Persistence
* APIs

## 🔹 Tooling

* Build system
* Debugging tools
* Developer UX

---

# ⚙️ Development Rules (Important)

## 1. Keep the system bootable

If it doesn’t boot in QEMU, it’s a blocker.

## 2. Don’t break the terminal

Input/output must always work.

## 3. Prefer small PRs

Large rewrites are hard to review and risky.

## 4. Avoid unnecessary dependencies

Especially in kernel code.

## 5. Document non-obvious decisions

Particularly around:

* memory
* safety
* WASM behavior

---

# 🧪 Testing Guidelines

## Required

* Boots successfully
* Terminal accepts input
* Existing commands still work
* WASM execution not broken

## Recommended

* Unit tests (where possible outside kernel)
* Serial logging for debugging

---

# 🧱 Coding Standards

## Rust

* `#![no_std]` for kernel code
* Minimize `unsafe`
* Document every `unsafe` block

## Style

* Clear naming
* Small functions
* Avoid hidden global state

---

# 🐛 Reporting Issues

When opening an issue, include:

* What happened
* What you expected
* Steps to reproduce
* Logs / screenshots (if relevant)

---

# 💡 Good First Contributions

If you’re new, try:

* Implement a missing i32 opcode (`i32.div_s`, `i32.rem_s`, `i32.shr_u`)
* Add `memory.size` returning the current page count
* Improve WASM error messages (show the bad opcode byte)
* Add a new shell command (e.g. `cat <name>` to hex-dump a registered file)

---

# 🚫 What to Avoid

* Large architectural changes without discussion
* Breaking existing functionality
* Overengineering early systems
* Adding heavy libraries

---

# 🧭 Design Philosophy

We are exploring:

* WASM as a system interface
* VM-based isolation
* Simpler OS abstractions

Not trying to:

* replicate Linux
* support POSIX
* be production-ready

---

# 🤖 AI Contributions

AI-generated contributions are welcome, but must:

* Follow all rules above
* Be understandable and reviewable
* Avoid large, speculative changes

---

# 📌 Final Checklist Before PR

* [ ] Builds successfully
* [ ] Boots in QEMU
* [ ] Terminal works
* [ ] No regressions
* [ ] Code is readable
* [ ] Unsafe code is documented

---

# 🙌 Thanks

Every contribution helps push this experiment forward.

Let’s build something weird—and make it work.
