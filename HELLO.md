🚀 **We just built a WASM-first OS (MVP) — looking for contributors**

We’ve got a minimal OS booting on bare metal (via QEMU) that:

* boots to a terminal
* accepts input
* loads and runs WebAssembly modules
* uses a VM-based execution model instead of traditional user/kernel boundaries

Now we’re opening it up.

---

## 🧠 What this project is about

This is a research-driven OS that explores:

* WebAssembly as the **primary application ABI**
* VM isolation instead of ring 0 / ring 3 separation
* Replacing syscalls with **host function interfaces**
* A smaller, more predictable execution model

Think: somewhere between a microkernel, a unikernel, and a WASM runtime — but designed to run on bare metal.

---

## 🎯 Where we’re going

Near-term:

* Stable WASM runtime (interpreter → JIT)
* Capability-based security model
* Real filesystem (persistent, not in-memory)
* Better terminal + developer UX

Mid-term:

* WASI-like interface (or alternative)
* Multi-module execution + isolation guarantees
* Async I/O model
* Tooling (build/run/debug inside the OS)

Long-term:

* Self-hosted toolchain
* Networking stack
* GUI layer (still WASM-driven)
* Explore eliminating traditional process boundaries entirely

---

## 🛠️ Where you can help

We’ve broken work into clean areas so you can jump in where you’re strongest:

### 🔹 Runtime / WASM

* Interpreter improvements
* JIT experimentation
* Memory model + safety guarantees
* Host function interface design

### 🔹 Kernel / Systems

* Memory management
* Interrupts + scheduling
* Device drivers (keyboard, storage, eventually network)

### 🔹 Filesystem

* Simple persistent FS design
* File APIs exposed to WASM modules
* Storage drivers

### 🔹 Tooling

* Dev UX inside the OS (editor, shell improvements)
* Build pipeline for WASM targets
* Debugging + tracing tools

### 🔹 Language / Compiler

* WAT support or lightweight compiler
* Better integration with Rust/C → WASM
* Exploring self-hosted compilation

---

## 🧩 Why this is interesting

* You get to work **close to the metal** *and* at the runtime level
* It challenges long-standing OS assumptions (syscalls, privilege rings)
* It’s small enough to understand, but deep enough to matter
* It’s a clean playground for experimenting with:

  * isolation models
  * execution environments
  * language/runtime boundaries

---

## 🧪 Current state

* Boots reliably in QEMU
* Terminal + command loop working
* WASM modules execute via a minimal runtime

We’re past the “hello world OS” phase and into **real systems design tradeoffs**.

---

## 🤝 Who we’re looking for

* Systems programmers (Rust/C/C++)
* Runtime / VM engineers
* People interested in OS design experiments
* Folks who like building weird but principled systems

You don’t need to be an OS expert — just comfortable reading low-level code and reasoning about performance and safety.

---

## 🧭 How to get involved

* Check out the repo (link coming)
* Pick an area above
* Open an issue / discussion with what you want to tackle
* Or just start hacking and send a PR

---

If you’ve ever wanted to rethink what an OS *should* look like in a WASM-first world, this is a good place to do it.

Let’s build something weird and useful.
