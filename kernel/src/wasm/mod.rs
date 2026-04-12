//! WASM subsystem — binary loader, interpreter, execution engine, and cooperative task layer.
//!
//! # Module layout
//!
//! | Module | Role |
//! |--------|------|
//! | [`loader`] | Zero-copy WASM binary parser; produces [`loader::Module`] |
//! | [`interp`] | Stack-machine interpreter; all state in fixed-size arrays on the kernel stack |
//! | [`engine`] | Instance pool, host-function registry, `spawn`/`call`/`destroy` API |
//! | [`task`]   | Cooperative task wrapper; feeds the round-robin [`crate::scheduler`] |
//!
//! No heap allocation occurs in the loader or interpreter.  The engine allocates
//! static memory (via [`engine::SLOT_MEM`]) at compile time.

pub mod engine;
pub mod interp;
pub mod loader;
pub mod task;
pub mod opcode;
