//! WASM stack-machine interpreter.
//!
//! All state lives in fixed-size arrays — no heap allocation, no `alloc`.
//! The [`Interpreter`] struct (~70 KiB) is stack-allocated inside
//! [`engine::spawn`](crate::wasm::engine::spawn) and borrowed for the
//! lifetime of the pool slot.
//!
//! # Execution model
//!
//! - Values are stored as `i64`; `i32` results are sign-extended so that
//!   both types coexist on the same [`STACK_DEPTH`]-deep value stack.
//! - Host imports are resolved at instantiation time and stored as raw
//!   function pointers in `host_fns`, so there is no name-lookup overhead
//!   at call sites.
//! - Cooperative yielding is implemented by returning
//!   [`InterpError::Yielded`] from a host function; the call stack is
//!   left intact so [`Interpreter::resume`] can continue exactly where
//!   execution paused.

use super::loader::{Module, read_u32_leb128};

// ── Opcodes ───────────────────────────────────────────────────────────────────
const OP_UNREACHABLE:   u8 = 0x00;
const OP_NOP:           u8 = 0x01;
const OP_BLOCK:         u8 = 0x02;
const OP_LOOP:          u8 = 0x03;
const OP_IF:            u8 = 0x04;
const OP_ELSE:          u8 = 0x05;
const OP_END:           u8 = 0x0B;
const OP_BR:            u8 = 0x0C;
const OP_BR_IF:         u8 = 0x0D;
const OP_BR_TABLE:      u8 = 0x0E;
const OP_RETURN:        u8 = 0x0F;
const OP_CALL:          u8 = 0x10;
const OP_CALL_INDIRECT: u8 = 0x11;
const OP_DROP:          u8 = 0x1A;
const OP_SELECT:        u8 = 0x1B;
const OP_LOCAL_GET:     u8 = 0x20;
const OP_LOCAL_SET:     u8 = 0x21;
const OP_LOCAL_TEE:     u8 = 0x22;
const OP_GLOBAL_GET:    u8 = 0x23;
const OP_GLOBAL_SET:    u8 = 0x24;
const OP_I32_LOAD:      u8 = 0x28;
const OP_I32_LOAD8_U:   u8 = 0x2D;
const OP_I32_STORE:     u8 = 0x36;
const OP_I32_STORE8:    u8 = 0x3A;
const OP_MEMORY_SIZE:   u8 = 0x3F;
const OP_MEMORY_GROW:   u8 = 0x40;
const OP_I32_CONST:     u8 = 0x41;
const OP_I32_EQZ:       u8 = 0x45;
const OP_I32_EQ:        u8 = 0x46;
const OP_I32_NE:        u8 = 0x47;
const OP_I32_LT_S:      u8 = 0x48;
const OP_I32_LT_U:      u8 = 0x49;
const OP_I32_GT_S:      u8 = 0x4A;
const OP_I32_GT_U:      u8 = 0x4B;
const OP_I32_LE_S:      u8 = 0x4C;
const OP_I32_LE_U:      u8 = 0x4D;
const OP_I32_GE_S:      u8 = 0x4E;
const OP_I32_GE_U:      u8 = 0x4F;
const OP_I32_CLZ:       u8 = 0x67;
const OP_I32_CTZ:       u8 = 0x68;
const OP_I32_POPCNT:    u8 = 0x69;
const OP_I32_ADD:       u8 = 0x6A;
const OP_I32_SUB:       u8 = 0x6B;
const OP_I32_MUL:       u8 = 0x6C;
const OP_I32_DIV_S:     u8 = 0x6D;
const OP_I32_DIV_U:     u8 = 0x6E;
const OP_I32_REM_S:     u8 = 0x6F;
const OP_I32_REM_U:     u8 = 0x70;
const OP_I32_AND:       u8 = 0x71;
const OP_I32_OR:        u8 = 0x72;
const OP_I32_XOR:       u8 = 0x73;
const OP_I32_SHL:       u8 = 0x74;
const OP_I32_SHR_S:     u8 = 0x75;
const OP_I32_SHR_U:     u8 = 0x76;
const OP_I32_ROTL:      u8 = 0x77;
const OP_I32_ROTR:      u8 = 0x78;
const OP_I64_CONST:     u8 = 0x42;
const OP_I64_EQZ:       u8 = 0x50;
const OP_I64_EQ:        u8 = 0x51;
const OP_I64_NE:        u8 = 0x52;
const OP_I64_LT_S:      u8 = 0x53;
const OP_I64_LT_U:      u8 = 0x54;
const OP_I64_GT_S:      u8 = 0x55;
const OP_I64_GT_U:      u8 = 0x56;
const OP_I64_LE_S:      u8 = 0x57;
const OP_I64_LE_U:      u8 = 0x58;
const OP_I64_GE_S:      u8 = 0x59;
const OP_I64_GE_U:      u8 = 0x5A;
const OP_I64_LOAD:      u8 = 0x29;
const OP_I64_STORE:     u8 = 0x37;
const OP_I64_CLZ:       u8 = 0x79;
const OP_I64_CTZ:       u8 = 0x7A;
const OP_I64_POPCNT:    u8 = 0x7B;
const OP_I64_ADD:       u8 = 0x7C;
const OP_I64_SUB:       u8 = 0x7D;
const OP_I64_MUL:       u8 = 0x7E;
const OP_I64_AND:       u8 = 0x83;
const OP_I64_OR:        u8 = 0x84;
const OP_I64_XOR:       u8 = 0x85;
const OP_I64_SHL:       u8 = 0x86;
const OP_I64_SHR_S:     u8 = 0x87;
const OP_I64_SHR_U:     u8 = 0x88;
const OP_I64_ROTL:      u8 = 0x89;
const OP_I64_ROTR:      u8 = 0x8A;
// narrow loads
const OP_I32_LOAD8_S:  u8 = 0x2C;
const OP_I32_LOAD16_S: u8 = 0x2E;
const OP_I32_LOAD16_U: u8 = 0x2F;
const OP_I64_LOAD8_S:  u8 = 0x30;
const OP_I64_LOAD8_U:  u8 = 0x31;
const OP_I64_LOAD16_S: u8 = 0x32;
const OP_I64_LOAD16_U: u8 = 0x33;
const OP_I64_LOAD32_S: u8 = 0x34;
const OP_I64_LOAD32_U: u8 = 0x35;
// narrow stores
const OP_I32_STORE16: u8 = 0x3B;
const OP_I64_STORE8:  u8 = 0x3C;
const OP_I64_STORE16: u8 = 0x3D;
const OP_I64_STORE32: u8 = 0x3E;
// i64 div/rem
const OP_I64_DIV_S: u8 = 0x7F;
const OP_I64_DIV_U: u8 = 0x80;
const OP_I64_REM_S: u8 = 0x81;
const OP_I64_REM_U: u8 = 0x82;
// type conversions
const OP_I32_WRAP_I64:     u8 = 0xA7;
const OP_I64_EXTEND_I32_S: u8 = 0xAC;
const OP_I64_EXTEND_I32_U: u8 = 0xAD;
// sign-extension
const OP_I32_EXTEND8_S:  u8 = 0xC0;
const OP_I32_EXTEND16_S: u8 = 0xC1;
const OP_I64_EXTEND8_S:  u8 = 0xC2;
const OP_I64_EXTEND16_S: u8 = 0xC3;
const OP_I64_EXTEND32_S: u8 = 0xC4;

// ── Capacity limits ───────────────────────────────────────────────────────────

/// Maximum number of functions (imports + defined) in a module.
pub const MAX_FUNCS:      usize = 512;
/// Maximum number of type-section entries (function signatures).
pub const MAX_TYPES:      usize = 128;
/// Maximum locals per function frame (parameters + declared locals).
pub const MAX_LOCALS:     usize = 32;
/// Maximum number of global variables.
pub const MAX_GLOBALS:    usize = 64;
/// Maximum number of function-table entries (for `call_indirect`).
pub const MAX_TABLE:      usize = 512;

const NULL_FUNC: u32 = u32::MAX; // sentinel for an uninitialised table entry

/// Maximum total block/loop/if nesting depth across all live call frames.
pub const MAX_CTRL_DEPTH: usize = 128;
/// Depth of the value stack (`i64` entries).
pub const STACK_DEPTH:    usize = 256;
/// Maximum call-stack depth (number of simultaneously live call frames).
pub const CALL_DEPTH:     usize = 128;
const PAGE_SIZE: usize = 65536;

const NO_ELSE: usize = usize::MAX;

// ── Pre-computed jump table ───────────────────────────────────────────────────
//
// `scan_block_end` is an O(n) linear scan of the function body.  Calling it on
// every `block`/`loop`/`if` opcode during hot execution (e.g. the inner loops
// of a NES CPU emulator) causes a catastrophic slowdown — effectively O(n)
// byte-scan overhead per structural opcode hit, millions of times per second.
//
// The fix: at instantiation time, pre-scan every function body once and store
// all `(body_idx, scan_start) → (end_pc, else_pc)` results in an open-
// addressing hash table.  The dispatch loop then does a O(1) hash lookup
// instead of a repeated linear scan.

/// Size of the jump-table hash map.  Must be a power of two.
/// 4096 slots keep the load factor ≤ 50 % for typical WASM modules
/// (nes.wasm has 144 functions and ~1500 structural opcodes).
const JUMP_SLOTS: usize = 4096;

#[derive(Clone, Copy)]
struct JumpSlot {
    /// Body index into `Interpreter::bodies`.  `u16::MAX` = empty slot.
    key_body: u16,
    _pad:     u16,
    /// Scan-start PC (= opcode_pc + 2, the `start` arg to `scan_block_end`).
    key_pc:   u32,
    end_pc:   u32,
    /// `u32::MAX` = `NO_ELSE`.
    else_pc:  u32,
}

const EMPTY_JUMP_SLOT: JumpSlot = JumpSlot {
    key_body: u16::MAX, _pad: 0, key_pc: 0, end_pc: 0, else_pc: u32::MAX,
};

/// Signature for a kernel host function callable from WASM.
///
/// - `vstack` — the interpreter's value stack (`i64` entries).
/// - `vsp`    — stack pointer (index of the next free slot).
/// - `mem`    — the module's linear memory for the current pool slot.
///
/// A host function pops its arguments from the top of `vstack` (decrementing
/// `vsp`) and, if it returns a value, pushes it back (incrementing `vsp`).
///
/// Return `Err(InterpError::Yielded)` to suspend the task cooperatively.
pub type HostFn = fn(vstack: &mut [i64], vsp: &mut usize, mem: &mut [u8])
    -> Result<(), InterpError>;

// ── Error type ────────────────────────────────────────────────────────────────

/// Errors (and pseudo-errors) that can arise during WASM interpretation.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum InterpError {
    /// The module has no code section.
    NoCodeSection,
    /// Bytecode structure is invalid (unexpected byte, truncated encoding, etc.).
    MalformedCode,
    /// More functions than [`MAX_FUNCS`].
    TooManyFuncs,
    /// More type entries than [`MAX_TYPES`].
    TooManyTypes,
    /// A function requires more locals than [`MAX_LOCALS`].
    TooManyLocals,
    /// More globals than [`MAX_GLOBALS`].
    TooManyGlobals,
    /// Block/loop/if nesting exceeded [`MAX_CTRL_DEPTH`].
    CtrlStackOverflow,
    /// `call` or `call_indirect` target is out of the function index range.
    FuncIndexOutOfRange,
    /// `local.get` / `local.set` / `local.tee` index out of range.
    LocalIndexOutOfRange,
    /// `global.get` / `global.set` index out of range.
    GlobalIndexOutOfRange,
    /// `global.set` on a `const` (immutable) global.
    GlobalImmutable,
    /// `call_indirect` on a null (uninitialised) table entry.
    IndirectCallNull,
    /// `call_indirect` type signature mismatch.
    IndirectCallTypeMismatch,
    /// `call` to an import index that was not resolved at instantiation.
    ImportNotFound,
    /// The `unreachable` opcode was executed.
    Unreachable,
    /// Value stack depth exceeded [`STACK_DEPTH`].
    StackOverflow,
    /// Value stack underflowed (pop on an empty stack).
    StackUnderflow,
    /// Call depth exceeded [`CALL_DEPTH`].
    CallStackOverflow,
    /// Linear memory access outside the allocated page range.
    MemOutOfBounds,
    /// Integer division or remainder by zero.
    DivisionByZero,
    /// A float-to-integer conversion was out of range or produced NaN.
    InvalidConversion,
    /// An opcode with the given byte value is not implemented.
    UnknownOpcode(u8),
    /// Not a real error — the module called `env.yield` or `env.sleep_ms`.
    /// The interpreter's state is intact; call [`Interpreter::resume`] to continue.
    Yielded,
    /// Not a real error — the module called `env.exit`.
    /// Treated as clean completion by the engine.
    Exited,
}

impl InterpError {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NoCodeSection         => "no code section",
            Self::MalformedCode         => "malformed bytecode",
            Self::TooManyFuncs          => "too many functions (increase MAX_FUNCS)",
            Self::TooManyTypes          => "too many types (increase MAX_TYPES)",
            Self::TooManyLocals         => "too many locals (increase MAX_LOCALS)",
            Self::TooManyGlobals        => "too many globals (increase MAX_GLOBALS)",
            Self::CtrlStackOverflow     => "control stack overflow (increase MAX_CTRL_DEPTH)",
            Self::FuncIndexOutOfRange   => "function index out of range",
            Self::LocalIndexOutOfRange  => "local variable index out of range",
            Self::GlobalIndexOutOfRange => "global variable index out of range",
            Self::GlobalImmutable          => "write to immutable global",
            Self::IndirectCallNull         => "call_indirect: null table entry",
            Self::IndirectCallTypeMismatch => "call_indirect: type mismatch",
            Self::ImportNotFound        => "call to unresolved import",
            Self::Unreachable           => "unreachable executed",
            Self::StackOverflow         => "value stack overflow",
            Self::StackUnderflow        => "value stack underflow",
            Self::CallStackOverflow     => "call stack overflow",
            Self::MemOutOfBounds        => "memory access out of bounds",
            Self::DivisionByZero        => "integer divide by zero",
            Self::InvalidConversion     => "invalid float-to-integer conversion",
            Self::UnknownOpcode(_)      => "unknown opcode",
            Self::Yielded               => "task yielded",
            Self::Exited                => "module exited",
        }
    }
}

// ── Control frame (block / loop / if) ────────────────────────────────────────
#[derive(Clone, Copy, PartialEq)]
enum BlockKind { Block, Loop, If }

#[derive(Clone, Copy)]
struct CtrlFrame {
    kind:     BlockKind,
    /// For Loop: PC to jump back to (top of loop body).
    /// For Block/If: unused.
    pc_start: usize,
    /// Position OF the matching `end` opcode in the body slice.
    end_pc:   usize,
}

const BLANK_CTRL: CtrlFrame = CtrlFrame {
    kind: BlockKind::Block, pc_start: 0, end_pc: 0,
};

// ── Call-stack frame ──────────────────────────────────────────────────────────
#[derive(Clone, Copy)]
struct Frame {
    body_idx:     usize,
    pc:           usize,
    locals:       [i64; MAX_LOCALS],
    local_count:  usize,
    /// ctrl_depth at function entry (restored on return).
    ctrl_base:    usize,
    /// vsp after params were moved into locals (restored on return + results).
    vsp_base:     usize,
    /// Number of return values this function produces.
    result_count: usize,
}

const BLANK_FRAME: Frame = Frame {
    body_idx: 0, pc: 0,
    locals: [0i64; MAX_LOCALS], local_count: 0,
    ctrl_base: 0, vsp_base: 0, result_count: 0,
};

// ── Default host ──────────────────────────────────────────────────────────────

// ── Interpreter ───────────────────────────────────────────────────────────────

/// WASM stack-machine interpreter.
///
/// The struct is large (~70 KiB) and is normally stack-allocated inside
/// [`engine::spawn`](crate::wasm::engine::spawn).  It borrows the module's
/// body slices and the pool slot's linear memory for lifetime `'a`.
///
/// # Execution
///
/// - [`Interpreter::call`] — push a call frame for a defined function and run.
/// - [`Interpreter::resume`] — re-enter the dispatch loop after a yield.
/// - [`Interpreter::reset_for_call`] — clear stack/frame state between
///   invocations on the same instance (memory and globals are preserved).
pub struct Interpreter<'a> {
    bodies:             [&'a [u8]; MAX_FUNCS],
    body_count:         usize,
    local_counts:       [usize; MAX_FUNCS],
    type_param_counts:  [usize; MAX_TYPES],
    type_result_counts: [usize; MAX_TYPES],
    type_count:         usize,
    func_type_indices:  [usize; MAX_FUNCS],
    /// Flat type-index table for ALL functions: imports first, then defined.
    /// Indexed by absolute function index.
    all_func_types:     [usize; MAX_FUNCS],
    pub import_count:   usize,

    /// Function reference table (populated from the element section).
    table:      [u32; MAX_TABLE],
    table_size: usize,

    pub vstack: [i64; STACK_DEPTH],
    pub vsp:    usize,

    frames: [Frame; CALL_DEPTH],
    fdepth: usize,

    ctrl:       [CtrlFrame; MAX_CTRL_DEPTH],
    ctrl_depth: usize,

    globals:         [i64; MAX_GLOBALS],
    global_mutable:  [bool; MAX_GLOBALS],
    global_count:    usize,

    pub mem:           &'a mut [u8],
    pub host_fns:      [Option<HostFn>; MAX_FUNCS],

    /// Pre-computed block/loop/if → end_pc hash table (open-addressing).
    /// Populated once during `new()`; read-only during execution.
    jump_table: [JumpSlot; JUMP_SLOTS],

    /// Active WASM linear memory pages (64 KiB each).
    /// Bounds checks use `current_pages * PAGE_SIZE`; memory.grow increases this.
    pub current_pages: u32,
    max_pages:         u32,

    /// Set by host functions; checked at the top of each dispatch iteration.
    pub yield_requested: bool,
}

impl<'a> Interpreter<'a> {
    /// Construct an `Interpreter` from a parsed module.
    ///
    /// Parses the type, function, code, global, and element sections into
    /// the fixed-size internal tables.  `import_count` is the number of
    /// function imports (determines the import/defined split for `call`).
    /// `host_fns[i]` must be `Some` for every import index `i < import_count`.
    /// `initial_pages` sets [`Interpreter::current_pages`].
    ///
    /// # Errors
    ///
    /// Returns [`InterpError`] if any section exceeds the corresponding
    /// capacity limit or is structurally malformed.
    pub fn new(module: &Module<'a>, import_count: usize, mem: &'a mut [u8], host_fns: [Option<HostFn>; MAX_FUNCS], initial_pages: u32) -> Result<Self, InterpError> {
        let mut type_param_counts  = [0usize; MAX_TYPES];
        let mut type_result_counts = [0usize; MAX_TYPES];
        let type_count = if let Some(tb) = module.type_section {
            parse_type_section(tb, &mut type_param_counts, &mut type_result_counts)?
        } else { 0 };

        let mut func_type_indices = [0usize; MAX_FUNCS];
        if let Some(fb) = module.function_section {
            parse_function_section(fb, &mut func_type_indices)?;
        }

        let code_bytes = module.code_section.ok_or(InterpError::NoCodeSection)?;
        let mut bodies       = [&[][..] as &[u8]; MAX_FUNCS];
        let mut local_counts = [0usize; MAX_FUNCS];
        let body_count = parse_code_section(code_bytes, &mut bodies, &mut local_counts)?;

        let mut globals        = [0i64; MAX_GLOBALS];
        let mut global_mutable = [false; MAX_GLOBALS];
        let global_count = if let Some(gb) = module.global_section {
            parse_global_section(gb, &mut globals, &mut global_mutable)?
        } else { 0 };

        // Build all_func_types: type index for every function (imports then defined).
        let mut all_func_types = [0usize; MAX_FUNCS];
        if let Some(ib) = module.import_section {
            parse_import_func_types(ib, &mut all_func_types)?;
        }
        for i in 0..body_count {
            let abs = import_count + i;
            if abs < MAX_FUNCS { all_func_types[abs] = func_type_indices[i]; }
        }

        // Populate function table from element section.
        let mut table      = [NULL_FUNC; MAX_TABLE];
        let mut table_size = 0usize;
        if let Some(eb) = module.element_section {
            parse_element_section(eb, &mut table, &mut table_size)?;
        }

        // Pre-scan all function bodies to build the O(1) jump table.
        // This replaces the per-dispatch O(n) scan_block_end calls with a
        // single O(n_total) pass at instantiation time.
        let mut jump_table = [EMPTY_JUMP_SLOT; JUMP_SLOTS];
        for bi in 0..body_count {
            pre_scan_body_jumps(bodies[bi], bi, &mut jump_table);
        }

        let max_pages = (mem.len() / PAGE_SIZE) as u32;
        Ok(Self {
            bodies, body_count, local_counts,
            type_param_counts, type_result_counts, type_count, func_type_indices,
            all_func_types,
            import_count,
            table, table_size,
            vstack: [0i64; STACK_DEPTH], vsp: 0,
            frames: [BLANK_FRAME; CALL_DEPTH], fdepth: 0,
            ctrl: [BLANK_CTRL; MAX_CTRL_DEPTH], ctrl_depth: 0,
            globals, global_mutable, global_count,

            mem,
            host_fns,
            jump_table,
            current_pages: initial_pages,
            max_pages,
            yield_requested: false,
        })
    }

    #[inline]
    fn active_mem_bytes(&self) -> usize {
        self.current_pages as usize * PAGE_SIZE
    }

    /// Call the defined function at absolute index `func_idx`.
    ///
    /// Pushes a new call frame and runs the dispatch loop until the function
    /// returns, the interpreter yields, or a trap occurs.
    ///
    /// # Errors
    ///
    /// Returns [`InterpError::ImportNotFound`] if `func_idx` is in the import
    /// range, or [`InterpError::FuncIndexOutOfRange`] if it is out of bounds.
    pub fn call(&mut self, func_idx: usize) -> Result<(), InterpError> {
        if func_idx < self.import_count { return Err(InterpError::ImportNotFound); }
        let body_idx = func_idx - self.import_count;
        if body_idx >= self.body_count { return Err(InterpError::FuncIndexOutOfRange); }
        self.push_frame(body_idx)?;
        self.run()
    }

    /// Re-enter the dispatch loop after a [`InterpError::Yielded`] suspension.
    ///
    /// All frame, value-stack, and control-stack state is preserved between
    /// yield and resume.  Do **not** call this after a normal return or trap.
    pub fn resume(&mut self) -> Result<(), InterpError> {
        self.run()
    }

    /// Reset execution state (value stack, call frames, control stack) while
    /// preserving linear memory and globals.
    ///
    /// Use between successive calls on the same instance rather than
    /// constructing a new `Interpreter` each time.
    pub fn reset_for_call(&mut self) {
        self.vsp             = 0;
        self.fdepth          = 0;
        self.ctrl_depth      = 0;
        self.yield_requested = false;
    }

    #[allow(dead_code)]
    pub fn top_i32(&self) -> Option<i32> {
        if self.vsp > 0 { Some(self.vstack[self.vsp - 1] as i32) } else { None }
    }

    /// Return the top-of-stack value (`i64`) without popping, or `None` if
    /// the stack is empty.  Used by the engine to read function return values.
    pub fn top(&self) -> Option<i64> {
        if self.vsp > 0 { Some(self.vstack[self.vsp - 1]) } else { None }
    }

    // ── Internal helpers ──────────────────────────────────────────────────────

    fn push_frame(&mut self, body_idx: usize) -> Result<(), InterpError> {
        if self.fdepth >= CALL_DEPTH { return Err(InterpError::CallStackOverflow); }

        let type_idx     = self.func_type_indices[body_idx];
        let param_count  = if type_idx < self.type_count { self.type_param_counts[type_idx]  } else { 0 };
        let result_count = if type_idx < self.type_count { self.type_result_counts[type_idx] } else { 0 };
        let decl_count   = self.local_counts[body_idx];
        let total        = param_count + decl_count;
        if total > MAX_LOCALS { return Err(InterpError::TooManyLocals); }

        let mut frame = Frame {
            body_idx, pc: 0,
            locals: [0i64; MAX_LOCALS], local_count: total,
            ctrl_base: self.ctrl_depth,
            vsp_base: 0,  // filled in below after params are popped
            result_count,
        };
        for i in (0..param_count).rev() {
            frame.locals[i] = self.v_pop()?;
        }
        frame.vsp_base = self.vsp; // stack base for this frame's results
        self.frames[self.fdepth] = frame;
        self.fdepth += 1;
        Ok(())
    }

    fn v_push(&mut self, v: i64) -> Result<(), InterpError> {
        if self.vsp >= STACK_DEPTH { return Err(InterpError::StackOverflow); }
        self.vstack[self.vsp] = v;
        self.vsp += 1;
        Ok(())
    }

    /// Pop the top value from the value stack.
    ///
    /// Returns [`InterpError::StackUnderflow`] if the stack is empty.
    pub fn v_pop(&mut self) -> Result<i64, InterpError> {
        if self.vsp == 0 { return Err(InterpError::StackUnderflow); }
        self.vsp -= 1;
        Ok(self.vstack[self.vsp])
    }

    /// O(1) hash-table lookup for the pre-computed `(end_pc, else_pc)` of the
    /// block/loop/if whose body starts at `scan_start` in body `body_idx`.
    ///
    /// Returns `None` only if the entry is absent (table overflow at init time,
    /// or malformed code).  Callers fall back to `scan_block_end` in that case.
    #[inline(always)]
    fn jump_lookup(&self, body_idx: usize, scan_start: usize) -> Option<(usize, usize)> {
        let mut i = jump_hash(body_idx, scan_start);
        loop {
            let s = &self.jump_table[i];
            if s.key_body == u16::MAX { return None; }
            if s.key_body as usize == body_idx && s.key_pc as usize == scan_start {
                let else_pc = if s.else_pc == u32::MAX { NO_ELSE } else { s.else_pc as usize };
                return Some((s.end_pc as usize, else_pc));
            }
            i = (i + 1) & (JUMP_SLOTS - 1);
        }
    }

    fn ctrl_push(&mut self, cf: CtrlFrame) -> Result<(), InterpError> {
        if self.ctrl_depth >= MAX_CTRL_DEPTH { return Err(InterpError::CtrlStackOverflow); }
        self.ctrl[self.ctrl_depth] = cf;
        self.ctrl_depth += 1;
        Ok(())
    }

    // ── Main dispatch loop ─────────────────────────────────────────────────────
    fn run(&mut self) -> Result<(), InterpError> {
        while self.fdepth > 0 {
            if self.yield_requested {
                self.yield_requested = false;
                return Err(InterpError::Yielded);
            }

            let fi       = self.fdepth - 1;
            let body_idx = self.frames[fi].body_idx;
            let pc       = self.frames[fi].pc;
            let body     = self.bodies[body_idx];

            if pc >= body.len() {
                // Implicit return at end of body.
                do_return(self);
                continue;
            }

            let opcode = body[pc];
            self.frames[fi].pc += 1;

            match opcode {

                // ── nop / unreachable ─────────────────────────────────────────
                OP_NOP => {}
                OP_UNREACHABLE => return Err(InterpError::Unreachable),

                // ── block <blocktype> ─────────────────────────────────────────
                OP_BLOCK => {
                    let fi   = self.fdepth - 1;
                    let bi   = self.frames[fi].body_idx;
                    let pc   = self.frames[fi].pc;
                    let body = self.bodies[bi];
                    // Skip blocktype (single byte in MVP).
                    let pc_body = pc + 1;
                    self.frames[fi].pc = pc_body;
                    let (end_pc, _) = self.jump_lookup(bi, pc_body)
                        .or_else(|| scan_block_end(body, pc_body))
                        .ok_or(InterpError::MalformedCode)?;
                    self.ctrl_push(CtrlFrame {
                        kind: BlockKind::Block,
                        pc_start: pc_body, end_pc,
                    })?;
                }

                // ── loop <blocktype> ─────────────────────────────────────────
                OP_LOOP => {
                    let fi   = self.fdepth - 1;
                    let bi   = self.frames[fi].body_idx;
                    let pc   = self.frames[fi].pc;
                    let body = self.bodies[bi];
                    let pc_body = pc + 1;
                    self.frames[fi].pc = pc_body;
                    let (end_pc, _) = self.jump_lookup(bi, pc_body)
                        .or_else(|| scan_block_end(body, pc_body))
                        .ok_or(InterpError::MalformedCode)?;
                    self.ctrl_push(CtrlFrame {
                        kind: BlockKind::Loop,
                        pc_start: pc_body, end_pc,
                    })?;
                }

                // ── if <blocktype> ────────────────────────────────────────────
                OP_IF => {
                    let fi        = self.fdepth - 1;
                    let bi        = self.frames[fi].body_idx;
                    let pc        = self.frames[fi].pc;
                    let body      = self.bodies[bi];
                    let pc_body   = pc + 1; // after blocktype
                    self.frames[fi].pc = pc_body;
                    let (end_pc, else_pc) = self.jump_lookup(bi, pc_body)
                        .or_else(|| scan_block_end(body, pc_body))
                        .ok_or(InterpError::MalformedCode)?;
                    let cond = self.v_pop()?;
                    if cond != 0 {
                        // Enter then-branch.
                        self.ctrl_push(CtrlFrame {
                            kind: BlockKind::If,
                            pc_start: pc_body, end_pc,
                        })?;
                    } else if else_pc != NO_ELSE {
                        // Condition false, has else: jump to else body.
                        self.ctrl_push(CtrlFrame {
                            kind: BlockKind::If,
                            pc_start: pc_body, end_pc,
                        })?;
                        self.frames[fi].pc = else_pc + 1;
                    } else {
                        // Condition false, no else: skip to after end.
                        self.frames[fi].pc = end_pc + 1;
                    }
                }

                // ── else ──────────────────────────────────────────────────────
                OP_ELSE => {
                    // Reached end of then-branch; jump past the else block.
                    let fi   = self.fdepth - 1;
                    let end_pc = self.ctrl[self.ctrl_depth - 1].end_pc;
                    self.ctrl_depth -= 1; // pop the if ctrl frame
                    self.frames[fi].pc = end_pc + 1;
                }

                // ── end ───────────────────────────────────────────────────────
                OP_END => {
                    let fi = self.fdepth - 1;
                    if self.ctrl_depth > self.frames[fi].ctrl_base {
                        // End of a block/loop/if.
                        self.ctrl_depth -= 1;
                    } else {
                        // End of function body — treat as implicit return.
                        do_return(self);
                    }
                }

                // ── br <labelidx> ─────────────────────────────────────────────
                OP_BR => {
                    let fi   = self.fdepth - 1;
                    let pc   = self.frames[fi].pc;
                    let body = self.bodies[self.frames[fi].body_idx];
                    let (n, consumed) = read_u32_leb128(&body[pc..])
                        .ok_or(InterpError::MalformedCode)?;
                    self.frames[fi].pc += consumed;
                    do_br(self, n as usize)?;
                }

                // ── br_if <labelidx> ──────────────────────────────────────────
                OP_BR_IF => {
                    let fi   = self.fdepth - 1;
                    let pc   = self.frames[fi].pc;
                    let body = self.bodies[self.frames[fi].body_idx];
                    let (n, consumed) = read_u32_leb128(&body[pc..])
                        .ok_or(InterpError::MalformedCode)?;
                    self.frames[fi].pc += consumed;
                    let cond = self.v_pop()?;
                    if cond != 0 {
                        do_br(self, n as usize)?;
                    }
                }

                // ── br_table <count> <labels…> <default> ──────────────────────
                OP_BR_TABLE => {
                    let fi   = self.fdepth - 1;
                    let pc   = self.frames[fi].pc;
                    let body = self.bodies[self.frames[fi].body_idx];
                    let (count, n) = read_u32_leb128(&body[pc..])
                        .ok_or(InterpError::MalformedCode)?;
                    let i = self.v_pop()? as u32;
                    // Walk labels to find target and compute end position.
                    let mut pos = pc + n;
                    let mut target_label = 0u32;
                    for k in 0..=count {
                        let (label, ln) = read_u32_leb128(&body[pos..])
                            .ok_or(InterpError::MalformedCode)?;
                        if k == (if i < count { i } else { count }) {
                            target_label = label;
                        }
                        pos += ln;
                    }
                    self.frames[fi].pc = pos;
                    do_br(self, target_label as usize)?;
                }

                // ── return ────────────────────────────────────────────────────
                OP_RETURN => { do_return(self); }

                // ── drop / select ─────────────────────────────────────────────
                OP_DROP => { self.v_pop()?; }
                OP_SELECT => {
                    let cond = self.v_pop()?;
                    let b    = self.v_pop()?;
                    let a    = self.v_pop()?;
                    self.v_push(if cond != 0 { a } else { b })?;
                }


                // ── local.get/set/tee ─────────────────────────────────────────
                OP_LOCAL_GET => {
                    let fi   = self.fdepth - 1;
                    let pc   = self.frames[fi].pc;
                    let body = self.bodies[self.frames[fi].body_idx];
                    let (idx, consumed) = read_u32_leb128(&body[pc..])
                        .ok_or(InterpError::MalformedCode)?;
                    self.frames[fi].pc += consumed;
                    let idx = idx as usize;
                    if idx >= self.frames[fi].local_count { return Err(InterpError::LocalIndexOutOfRange); }
                    let val = self.frames[fi].locals[idx];
                    self.v_push(val)?;
                }
                OP_LOCAL_SET => {
                    let fi   = self.fdepth - 1;
                    let pc   = self.frames[fi].pc;
                    let body = self.bodies[self.frames[fi].body_idx];
                    let (idx, consumed) = read_u32_leb128(&body[pc..])
                        .ok_or(InterpError::MalformedCode)?;
                    self.frames[fi].pc += consumed;
                    let idx = idx as usize;
                    if idx >= self.frames[fi].local_count { return Err(InterpError::LocalIndexOutOfRange); }
                    let val = self.v_pop()?;
                    self.frames[fi].locals[idx] = val;
                }
                OP_LOCAL_TEE => {
                    let fi   = self.fdepth - 1;
                    let pc   = self.frames[fi].pc;
                    let body = self.bodies[self.frames[fi].body_idx];
                    let (idx, consumed) = read_u32_leb128(&body[pc..])
                        .ok_or(InterpError::MalformedCode)?;
                    self.frames[fi].pc += consumed;
                    let idx = idx as usize;
                    if idx >= self.frames[fi].local_count { return Err(InterpError::LocalIndexOutOfRange); }
                    if self.vsp == 0 { return Err(InterpError::StackUnderflow); }
                    let val = self.vstack[self.vsp - 1];
                    self.frames[fi].locals[idx] = val;
                }

                // ── global.get / global.set ───────────────────────────────────
                OP_GLOBAL_GET => {
                    let fi   = self.fdepth - 1;
                    let pc   = self.frames[fi].pc;
                    let body = self.bodies[self.frames[fi].body_idx];
                    let (idx, consumed) = read_u32_leb128(&body[pc..])
                        .ok_or(InterpError::MalformedCode)?;
                    self.frames[fi].pc += consumed;
                    let idx = idx as usize;
                    if idx >= self.global_count { return Err(InterpError::GlobalIndexOutOfRange); }
                    let val = self.globals[idx];
                    self.v_push(val)?;
                }
                OP_GLOBAL_SET => {
                    let fi   = self.fdepth - 1;
                    let pc   = self.frames[fi].pc;
                    let body = self.bodies[self.frames[fi].body_idx];
                    let (idx, consumed) = read_u32_leb128(&body[pc..])
                        .ok_or(InterpError::MalformedCode)?;
                    self.frames[fi].pc += consumed;
                    let idx = idx as usize;
                    if idx >= self.global_count { return Err(InterpError::GlobalIndexOutOfRange); }
                    if !self.global_mutable[idx] { return Err(InterpError::GlobalImmutable); }
                    self.globals[idx] = self.v_pop()?;
                }

                // ── memory loads ──────────────────────────────────────────────
                OP_I32_LOAD => {
                    let fi   = self.fdepth - 1;
                    let pc   = self.frames[fi].pc;
                    let body = self.bodies[self.frames[fi].body_idx];
                    let (offset, consumed) = read_memarg(&body[pc..])
                        .ok_or(InterpError::MalformedCode)?;
                    self.frames[fi].pc += consumed;
                    let addr = self.v_pop()? as u32 as usize;
                    let ea   = addr.wrapping_add(offset as usize);
                    if ea + 4 > self.active_mem_bytes() { return Err(InterpError::MemOutOfBounds); }
                    let val = i32::from_le_bytes([
                        self.mem[ea], self.mem[ea+1], self.mem[ea+2], self.mem[ea+3],
                    ]);
                    self.v_push(val as i64)?;
                }
                OP_I32_LOAD8_U => {
                    let fi   = self.fdepth - 1;
                    let pc   = self.frames[fi].pc;
                    let body = self.bodies[self.frames[fi].body_idx];
                    let (offset, consumed) = read_memarg(&body[pc..])
                        .ok_or(InterpError::MalformedCode)?;
                    self.frames[fi].pc += consumed;
                    let addr = self.v_pop()? as u32 as usize;
                    let ea   = addr.wrapping_add(offset as usize);
                    if ea >= self.active_mem_bytes() { return Err(InterpError::MemOutOfBounds); }
                    self.v_push(self.mem[ea] as i64)?;
                }
                OP_I64_LOAD => {
                    let fi   = self.fdepth - 1;
                    let pc   = self.frames[fi].pc;
                    let body = self.bodies[self.frames[fi].body_idx];
                    let (offset, consumed) = read_memarg(&body[pc..])
                        .ok_or(InterpError::MalformedCode)?;
                    self.frames[fi].pc += consumed;
                    let addr = self.v_pop()? as u32 as usize;
                    let ea   = addr.wrapping_add(offset as usize);
                    if ea + 8 > self.active_mem_bytes() { return Err(InterpError::MemOutOfBounds); }
                    let val = i64::from_le_bytes([
                        self.mem[ea], self.mem[ea+1], self.mem[ea+2], self.mem[ea+3],
                        self.mem[ea+4], self.mem[ea+5], self.mem[ea+6], self.mem[ea+7],
                    ]);
                    self.v_push(val)?;
                }

                // f32.load (0x2A) — load 4 bytes as f32 bits
                0x2A => {
                    let fi   = self.fdepth - 1;
                    let pc   = self.frames[fi].pc;
                    let body = self.bodies[self.frames[fi].body_idx];
                    let (offset, consumed) = read_memarg(&body[pc..]).ok_or(InterpError::MalformedCode)?;
                    self.frames[fi].pc += consumed;
                    let addr = self.v_pop()? as u32 as usize;
                    let ea   = addr.wrapping_add(offset as usize);
                    if ea + 4 > self.active_mem_bytes() { return Err(InterpError::MemOutOfBounds); }
                    let bits = u32::from_le_bytes([self.mem[ea], self.mem[ea+1], self.mem[ea+2], self.mem[ea+3]]);
                    self.v_push(bits as i64)?;
                }
                // f64.load (0x2B) — load 8 bytes as f64 bits
                0x2B => {
                    let fi   = self.fdepth - 1;
                    let pc   = self.frames[fi].pc;
                    let body = self.bodies[self.frames[fi].body_idx];
                    let (offset, consumed) = read_memarg(&body[pc..]).ok_or(InterpError::MalformedCode)?;
                    self.frames[fi].pc += consumed;
                    let addr = self.v_pop()? as u32 as usize;
                    let ea   = addr.wrapping_add(offset as usize);
                    if ea + 8 > self.active_mem_bytes() { return Err(InterpError::MemOutOfBounds); }
                    let bits = u64::from_le_bytes([
                        self.mem[ea], self.mem[ea+1], self.mem[ea+2], self.mem[ea+3],
                        self.mem[ea+4], self.mem[ea+5], self.mem[ea+6], self.mem[ea+7],
                    ]);
                    self.v_push(bits as i64)?;
                }
                // narrow loads
                OP_I32_LOAD8_S => {
                    let fi   = self.fdepth - 1;
                    let pc   = self.frames[fi].pc;
                    let body = self.bodies[self.frames[fi].body_idx];
                    let (offset, consumed) = read_memarg(&body[pc..]).ok_or(InterpError::MalformedCode)?;
                    self.frames[fi].pc += consumed;
                    let addr = self.v_pop()? as u32 as usize;
                    let ea   = addr.wrapping_add(offset as usize);
                    if ea >= self.active_mem_bytes() { return Err(InterpError::MemOutOfBounds); }
                    self.v_push(self.mem[ea] as i8 as i32 as i64)?;
                }
                OP_I32_LOAD16_S => {
                    let fi   = self.fdepth - 1;
                    let pc   = self.frames[fi].pc;
                    let body = self.bodies[self.frames[fi].body_idx];
                    let (offset, consumed) = read_memarg(&body[pc..]).ok_or(InterpError::MalformedCode)?;
                    self.frames[fi].pc += consumed;
                    let addr = self.v_pop()? as u32 as usize;
                    let ea   = addr.wrapping_add(offset as usize);
                    if ea + 2 > self.active_mem_bytes() { return Err(InterpError::MemOutOfBounds); }
                    let v = i16::from_le_bytes([self.mem[ea], self.mem[ea+1]]);
                    self.v_push(v as i32 as i64)?;
                }
                OP_I32_LOAD16_U => {
                    let fi   = self.fdepth - 1;
                    let pc   = self.frames[fi].pc;
                    let body = self.bodies[self.frames[fi].body_idx];
                    let (offset, consumed) = read_memarg(&body[pc..]).ok_or(InterpError::MalformedCode)?;
                    self.frames[fi].pc += consumed;
                    let addr = self.v_pop()? as u32 as usize;
                    let ea   = addr.wrapping_add(offset as usize);
                    if ea + 2 > self.active_mem_bytes() { return Err(InterpError::MemOutOfBounds); }
                    let v = u16::from_le_bytes([self.mem[ea], self.mem[ea+1]]);
                    self.v_push(v as i64)?;
                }
                OP_I64_LOAD8_S => {
                    let fi   = self.fdepth - 1;
                    let pc   = self.frames[fi].pc;
                    let body = self.bodies[self.frames[fi].body_idx];
                    let (offset, consumed) = read_memarg(&body[pc..]).ok_or(InterpError::MalformedCode)?;
                    self.frames[fi].pc += consumed;
                    let addr = self.v_pop()? as u32 as usize;
                    let ea   = addr.wrapping_add(offset as usize);
                    if ea >= self.active_mem_bytes() { return Err(InterpError::MemOutOfBounds); }
                    self.v_push(self.mem[ea] as i8 as i64)?;
                }
                OP_I64_LOAD8_U => {
                    let fi   = self.fdepth - 1;
                    let pc   = self.frames[fi].pc;
                    let body = self.bodies[self.frames[fi].body_idx];
                    let (offset, consumed) = read_memarg(&body[pc..]).ok_or(InterpError::MalformedCode)?;
                    self.frames[fi].pc += consumed;
                    let addr = self.v_pop()? as u32 as usize;
                    let ea   = addr.wrapping_add(offset as usize);
                    if ea >= self.active_mem_bytes() { return Err(InterpError::MemOutOfBounds); }
                    self.v_push(self.mem[ea] as i64)?;
                }
                OP_I64_LOAD16_S => {
                    let fi   = self.fdepth - 1;
                    let pc   = self.frames[fi].pc;
                    let body = self.bodies[self.frames[fi].body_idx];
                    let (offset, consumed) = read_memarg(&body[pc..]).ok_or(InterpError::MalformedCode)?;
                    self.frames[fi].pc += consumed;
                    let addr = self.v_pop()? as u32 as usize;
                    let ea   = addr.wrapping_add(offset as usize);
                    if ea + 2 > self.active_mem_bytes() { return Err(InterpError::MemOutOfBounds); }
                    let v = i16::from_le_bytes([self.mem[ea], self.mem[ea+1]]);
                    self.v_push(v as i64)?;
                }
                OP_I64_LOAD16_U => {
                    let fi   = self.fdepth - 1;
                    let pc   = self.frames[fi].pc;
                    let body = self.bodies[self.frames[fi].body_idx];
                    let (offset, consumed) = read_memarg(&body[pc..]).ok_or(InterpError::MalformedCode)?;
                    self.frames[fi].pc += consumed;
                    let addr = self.v_pop()? as u32 as usize;
                    let ea   = addr.wrapping_add(offset as usize);
                    if ea + 2 > self.active_mem_bytes() { return Err(InterpError::MemOutOfBounds); }
                    let v = u16::from_le_bytes([self.mem[ea], self.mem[ea+1]]);
                    self.v_push(v as i64)?;
                }
                OP_I64_LOAD32_S => {
                    let fi   = self.fdepth - 1;
                    let pc   = self.frames[fi].pc;
                    let body = self.bodies[self.frames[fi].body_idx];
                    let (offset, consumed) = read_memarg(&body[pc..]).ok_or(InterpError::MalformedCode)?;
                    self.frames[fi].pc += consumed;
                    let addr = self.v_pop()? as u32 as usize;
                    let ea   = addr.wrapping_add(offset as usize);
                    if ea + 4 > self.active_mem_bytes() { return Err(InterpError::MemOutOfBounds); }
                    let v = i32::from_le_bytes([self.mem[ea], self.mem[ea+1], self.mem[ea+2], self.mem[ea+3]]);
                    self.v_push(v as i64)?;
                }
                OP_I64_LOAD32_U => {
                    let fi   = self.fdepth - 1;
                    let pc   = self.frames[fi].pc;
                    let body = self.bodies[self.frames[fi].body_idx];
                    let (offset, consumed) = read_memarg(&body[pc..]).ok_or(InterpError::MalformedCode)?;
                    self.frames[fi].pc += consumed;
                    let addr = self.v_pop()? as u32 as usize;
                    let ea   = addr.wrapping_add(offset as usize);
                    if ea + 4 > self.active_mem_bytes() { return Err(InterpError::MemOutOfBounds); }
                    let v = u32::from_le_bytes([self.mem[ea], self.mem[ea+1], self.mem[ea+2], self.mem[ea+3]]);
                    self.v_push(v as i64)?;
                }

                // ── memory stores ─────────────────────────────────────────────
                OP_I32_STORE => {
                    let fi   = self.fdepth - 1;
                    let pc   = self.frames[fi].pc;
                    let body = self.bodies[self.frames[fi].body_idx];
                    let (offset, consumed) = read_memarg(&body[pc..])
                        .ok_or(InterpError::MalformedCode)?;
                    self.frames[fi].pc += consumed;
                    let val  = self.v_pop()? as i32;
                    let addr = self.v_pop()? as u32 as usize;
                    let ea   = addr.wrapping_add(offset as usize);
                    if ea + 4 > self.active_mem_bytes() { return Err(InterpError::MemOutOfBounds); }
                    self.mem[ea..ea+4].copy_from_slice(&val.to_le_bytes());
                }
                OP_I32_STORE8 => {
                    let fi   = self.fdepth - 1;
                    let pc   = self.frames[fi].pc;
                    let body = self.bodies[self.frames[fi].body_idx];
                    let (offset, consumed) = read_memarg(&body[pc..])
                        .ok_or(InterpError::MalformedCode)?;
                    self.frames[fi].pc += consumed;
                    let val  = self.v_pop()? as u8;
                    let addr = self.v_pop()? as u32 as usize;
                    let ea   = addr.wrapping_add(offset as usize);
                    if ea >= self.active_mem_bytes() { return Err(InterpError::MemOutOfBounds); }
                    self.mem[ea] = val;
                }
                OP_I64_STORE => {
                    let fi   = self.fdepth - 1;
                    let pc   = self.frames[fi].pc;
                    let body = self.bodies[self.frames[fi].body_idx];
                    let (offset, consumed) = read_memarg(&body[pc..])
                        .ok_or(InterpError::MalformedCode)?;
                    self.frames[fi].pc += consumed;
                    let val  = self.v_pop()?;
                    let addr = self.v_pop()? as u32 as usize;
                    let ea   = addr.wrapping_add(offset as usize);
                    if ea + 8 > self.active_mem_bytes() { return Err(InterpError::MemOutOfBounds); }
                    self.mem[ea..ea+8].copy_from_slice(&val.to_le_bytes());
                }
                // f32.store (0x38) — store 4 bytes of f32 bits
                0x38 => {
                    let fi   = self.fdepth - 1;
                    let pc   = self.frames[fi].pc;
                    let body = self.bodies[self.frames[fi].body_idx];
                    let (offset, consumed) = read_memarg(&body[pc..]).ok_or(InterpError::MalformedCode)?;
                    self.frames[fi].pc += consumed;
                    let val  = self.v_pop()? as u32;
                    let addr = self.v_pop()? as u32 as usize;
                    let ea   = addr.wrapping_add(offset as usize);
                    if ea + 4 > self.active_mem_bytes() { return Err(InterpError::MemOutOfBounds); }
                    self.mem[ea..ea+4].copy_from_slice(&val.to_le_bytes());
                }
                // f64.store (0x39) — store 8 bytes of f64 bits
                0x39 => {
                    let fi   = self.fdepth - 1;
                    let pc   = self.frames[fi].pc;
                    let body = self.bodies[self.frames[fi].body_idx];
                    let (offset, consumed) = read_memarg(&body[pc..]).ok_or(InterpError::MalformedCode)?;
                    self.frames[fi].pc += consumed;
                    let val  = self.v_pop()? as u64;
                    let addr = self.v_pop()? as u32 as usize;
                    let ea   = addr.wrapping_add(offset as usize);
                    if ea + 8 > self.active_mem_bytes() { return Err(InterpError::MemOutOfBounds); }
                    self.mem[ea..ea+8].copy_from_slice(&val.to_le_bytes());
                }
                // narrow stores
                OP_I32_STORE16 => {
                    let fi   = self.fdepth - 1;
                    let pc   = self.frames[fi].pc;
                    let body = self.bodies[self.frames[fi].body_idx];
                    let (offset, consumed) = read_memarg(&body[pc..]).ok_or(InterpError::MalformedCode)?;
                    self.frames[fi].pc += consumed;
                    let val  = self.v_pop()? as u16;
                    let addr = self.v_pop()? as u32 as usize;
                    let ea   = addr.wrapping_add(offset as usize);
                    if ea + 2 > self.active_mem_bytes() { return Err(InterpError::MemOutOfBounds); }
                    self.mem[ea..ea+2].copy_from_slice(&val.to_le_bytes());
                }
                OP_I64_STORE8 => {
                    let fi   = self.fdepth - 1;
                    let pc   = self.frames[fi].pc;
                    let body = self.bodies[self.frames[fi].body_idx];
                    let (offset, consumed) = read_memarg(&body[pc..]).ok_or(InterpError::MalformedCode)?;
                    self.frames[fi].pc += consumed;
                    let val  = self.v_pop()? as u8;
                    let addr = self.v_pop()? as u32 as usize;
                    let ea   = addr.wrapping_add(offset as usize);
                    if ea >= self.active_mem_bytes() { return Err(InterpError::MemOutOfBounds); }
                    self.mem[ea] = val;
                }
                OP_I64_STORE16 => {
                    let fi   = self.fdepth - 1;
                    let pc   = self.frames[fi].pc;
                    let body = self.bodies[self.frames[fi].body_idx];
                    let (offset, consumed) = read_memarg(&body[pc..]).ok_or(InterpError::MalformedCode)?;
                    self.frames[fi].pc += consumed;
                    let val  = self.v_pop()? as u16;
                    let addr = self.v_pop()? as u32 as usize;
                    let ea   = addr.wrapping_add(offset as usize);
                    if ea + 2 > self.active_mem_bytes() { return Err(InterpError::MemOutOfBounds); }
                    self.mem[ea..ea+2].copy_from_slice(&val.to_le_bytes());
                }
                OP_I64_STORE32 => {
                    let fi   = self.fdepth - 1;
                    let pc   = self.frames[fi].pc;
                    let body = self.bodies[self.frames[fi].body_idx];
                    let (offset, consumed) = read_memarg(&body[pc..]).ok_or(InterpError::MalformedCode)?;
                    self.frames[fi].pc += consumed;
                    let val  = self.v_pop()? as u32;
                    let addr = self.v_pop()? as u32 as usize;
                    let ea   = addr.wrapping_add(offset as usize);
                    if ea + 4 > self.active_mem_bytes() { return Err(InterpError::MemOutOfBounds); }
                    self.mem[ea..ea+4].copy_from_slice(&val.to_le_bytes());
                }

                // ── memory.size / memory.grow ─────────────────────────────────
                OP_MEMORY_SIZE => {
                    let fi = self.fdepth - 1;
                    self.frames[fi].pc += 1; // skip reserved 0x00 byte
                    self.v_push(self.current_pages as i64)?;
                }
                OP_MEMORY_GROW => {
                    let fi = self.fdepth - 1;
                    self.frames[fi].pc += 1; // skip reserved 0x00 byte
                    let delta = self.v_pop()? as u32;
                    let old   = self.current_pages;
                    if old.saturating_add(delta) <= self.max_pages {
                        let old_bytes = old as usize * PAGE_SIZE;
                        let new_bytes = (old + delta) as usize * PAGE_SIZE;
                        self.mem[old_bytes..new_bytes].fill(0);
                        self.current_pages = old + delta;
                        self.v_push(old as i64)?;
                    } else {
                        self.v_push(-1i64)?;
                    }
                }

                // ── i32.const / i64.const ────────────────────────────────────
                OP_I32_CONST => {
                    let fi   = self.fdepth - 1;
                    let pc   = self.frames[fi].pc;
                    let body = self.bodies[self.frames[fi].body_idx];
                    let (val, consumed) = read_i32_leb128(&body[pc..])
                        .ok_or(InterpError::MalformedCode)?;
                    self.frames[fi].pc += consumed;
                    self.v_push(val as i64)?;
                }
                OP_I64_CONST => {
                    let fi   = self.fdepth - 1;
                    let pc   = self.frames[fi].pc;
                    let body = self.bodies[self.frames[fi].body_idx];
                    let (val, consumed) = read_i64_leb128(&body[pc..])
                        .ok_or(InterpError::MalformedCode)?;
                    self.frames[fi].pc += consumed;
                    self.v_push(val)?;
                }

                // ── i32 comparisons ───────────────────────────────────────────
                OP_I32_EQZ  => { let a = self.v_pop()? as i32; self.v_push(if a == 0 { 1 } else { 0 })?; }
                OP_I32_EQ   => { let b = self.v_pop()? as i32; let a = self.v_pop()? as i32; self.v_push(if a == b { 1 } else { 0 })?; }
                OP_I32_NE   => { let b = self.v_pop()? as i32; let a = self.v_pop()? as i32; self.v_push(if a != b { 1 } else { 0 })?; }
                OP_I32_LT_S => { let b = self.v_pop()? as i32; let a = self.v_pop()? as i32; self.v_push(if a <  b { 1 } else { 0 })?; }
                OP_I32_LT_U => { let b = self.v_pop()? as u32; let a = self.v_pop()? as u32; self.v_push(if a <  b { 1 } else { 0 })?; }
                OP_I32_GT_S => { let b = self.v_pop()? as i32; let a = self.v_pop()? as i32; self.v_push(if a >  b { 1 } else { 0 })?; }
                OP_I32_GT_U => { let b = self.v_pop()? as u32; let a = self.v_pop()? as u32; self.v_push(if a >  b { 1 } else { 0 })?; }
                OP_I32_LE_S => { let b = self.v_pop()? as i32; let a = self.v_pop()? as i32; self.v_push(if a <= b { 1 } else { 0 })?; }
                OP_I32_LE_U => { let b = self.v_pop()? as u32; let a = self.v_pop()? as u32; self.v_push(if a <= b { 1 } else { 0 })?; }
                OP_I32_GE_S => { let b = self.v_pop()? as i32; let a = self.v_pop()? as i32; self.v_push(if a >= b { 1 } else { 0 })?; }
                OP_I32_GE_U => { let b = self.v_pop()? as u32; let a = self.v_pop()? as u32; self.v_push(if a >= b { 1 } else { 0 })?; }

                // ── i64 comparisons ───────────────────────────────────────────
                OP_I64_EQZ  => { let a = self.v_pop()?; self.v_push(if a == 0 { 1 } else { 0 })?; }
                OP_I64_EQ   => { let b = self.v_pop()?; let a = self.v_pop()?; self.v_push(if a == b { 1 } else { 0 })?; }
                OP_I64_NE   => { let b = self.v_pop()?; let a = self.v_pop()?; self.v_push(if a != b { 1 } else { 0 })?; }
                OP_I64_LT_S => { let b = self.v_pop()?; let a = self.v_pop()?; self.v_push(if a <  b { 1 } else { 0 })?; }
                OP_I64_LT_U => { let b = self.v_pop()? as u64; let a = self.v_pop()? as u64; self.v_push(if a <  b { 1 } else { 0 })?; }
                OP_I64_GT_S => { let b = self.v_pop()?; let a = self.v_pop()?; self.v_push(if a >  b { 1 } else { 0 })?; }
                OP_I64_GT_U => { let b = self.v_pop()? as u64; let a = self.v_pop()? as u64; self.v_push(if a >  b { 1 } else { 0 })?; }
                OP_I64_LE_S => { let b = self.v_pop()?; let a = self.v_pop()?; self.v_push(if a <= b { 1 } else { 0 })?; }
                OP_I64_LE_U => { let b = self.v_pop()? as u64; let a = self.v_pop()? as u64; self.v_push(if a <= b { 1 } else { 0 })?; }
                OP_I64_GE_S => { let b = self.v_pop()?; let a = self.v_pop()?; self.v_push(if a >= b { 1 } else { 0 })?; }
                OP_I64_GE_U => { let b = self.v_pop()? as u64; let a = self.v_pop()? as u64; self.v_push(if a >= b { 1 } else { 0 })?; }

                // ── i32 arithmetic ────────────────────────────────────────────
                OP_I32_CLZ    => { let a = self.v_pop()? as i32; self.v_push((a as u32).leading_zeros() as i64)?; }
                OP_I32_CTZ    => { let a = self.v_pop()? as i32; self.v_push((a as u32).trailing_zeros() as i64)?; }
                OP_I32_POPCNT => { let a = self.v_pop()? as i32; self.v_push((a as u32).count_ones() as i64)?; }
                OP_I32_ADD    => { let b = self.v_pop()? as i32; let a = self.v_pop()? as i32; self.v_push(a.wrapping_add(b) as i64)?; }
                OP_I32_SUB    => { let b = self.v_pop()? as i32; let a = self.v_pop()? as i32; self.v_push(a.wrapping_sub(b) as i64)?; }
                OP_I32_MUL    => { let b = self.v_pop()? as i32; let a = self.v_pop()? as i32; self.v_push(a.wrapping_mul(b) as i64)?; }
                OP_I32_DIV_S  => {
                    let b = self.v_pop()? as i32; let a = self.v_pop()? as i32;
                    if b == 0 { return Err(InterpError::DivisionByZero); }
                    self.v_push(a.wrapping_div(b) as i64)?;
                }
                OP_I32_DIV_U  => {
                    let b = self.v_pop()? as u32; let a = self.v_pop()? as u32;
                    if b == 0 { return Err(InterpError::DivisionByZero); }
                    self.v_push((a / b) as i64)?;
                }
                OP_I32_REM_S  => {
                    let b = self.v_pop()? as i32; let a = self.v_pop()? as i32;
                    if b == 0 { return Err(InterpError::DivisionByZero); }
                    self.v_push(a.wrapping_rem(b) as i64)?;
                }
                OP_I32_REM_U  => {
                    let b = self.v_pop()? as u32; let a = self.v_pop()? as u32;
                    if b == 0 { return Err(InterpError::DivisionByZero); }
                    self.v_push((a % b) as i64)?;
                }
                OP_I32_AND    => { let b = self.v_pop()? as i32; let a = self.v_pop()? as i32; self.v_push((a & b) as i64)?; }
                OP_I32_OR     => { let b = self.v_pop()? as i32; let a = self.v_pop()? as i32; self.v_push((a | b) as i64)?; }
                OP_I32_XOR    => { let b = self.v_pop()? as i32; let a = self.v_pop()? as i32; self.v_push((a ^ b) as i64)?; }
                OP_I32_SHL    => { let b = self.v_pop()? as u32; let a = self.v_pop()? as i32; self.v_push(a.wrapping_shl(b & 31) as i64)?; }
                OP_I32_SHR_S  => { let b = self.v_pop()? as u32; let a = self.v_pop()? as i32; self.v_push(a.wrapping_shr(b & 31) as i64)?; }
                OP_I32_SHR_U  => { let b = self.v_pop()? as u32; let a = self.v_pop()? as u32; self.v_push(a.wrapping_shr(b & 31) as i64)?; }
                OP_I32_ROTL   => { let b = self.v_pop()? as u32; let a = self.v_pop()? as u32; self.v_push(a.rotate_left(b & 31) as i32 as i64)?; }
                OP_I32_ROTR   => { let b = self.v_pop()? as u32; let a = self.v_pop()? as u32; self.v_push(a.rotate_right(b & 31) as i32 as i64)?; }

                // ── i64 arithmetic ────────────────────────────────────────────
                OP_I64_CLZ    => { let a = self.v_pop()?; self.v_push((a as u64).leading_zeros() as i64)?; }
                OP_I64_CTZ    => { let a = self.v_pop()?; self.v_push((a as u64).trailing_zeros() as i64)?; }
                OP_I64_POPCNT => { let a = self.v_pop()?; self.v_push((a as u64).count_ones() as i64)?; }
                OP_I64_ADD    => { let b = self.v_pop()?; let a = self.v_pop()?; self.v_push(a.wrapping_add(b))?; }
                OP_I64_SUB    => { let b = self.v_pop()?; let a = self.v_pop()?; self.v_push(a.wrapping_sub(b))?; }
                OP_I64_MUL    => { let b = self.v_pop()?; let a = self.v_pop()?; self.v_push(a.wrapping_mul(b))?; }
                OP_I64_DIV_S  => {
                    let b = self.v_pop()?; let a = self.v_pop()?;
                    if b == 0 { return Err(InterpError::DivisionByZero); }
                    if a == i64::MIN && b == -1 { return Err(InterpError::DivisionByZero); }
                    self.v_push(a.wrapping_div(b))?;
                }
                OP_I64_DIV_U  => {
                    let b = self.v_pop()? as u64; let a = self.v_pop()? as u64;
                    if b == 0 { return Err(InterpError::DivisionByZero); }
                    self.v_push((a / b) as i64)?;
                }
                OP_I64_REM_S  => {
                    let b = self.v_pop()?; let a = self.v_pop()?;
                    if b == 0 { return Err(InterpError::DivisionByZero); }
                    self.v_push(a.wrapping_rem(b))?;
                }
                OP_I64_REM_U  => {
                    let b = self.v_pop()? as u64; let a = self.v_pop()? as u64;
                    if b == 0 { return Err(InterpError::DivisionByZero); }
                    self.v_push((a % b) as i64)?;
                }
                OP_I64_AND    => { let b = self.v_pop()?; let a = self.v_pop()?; self.v_push(a & b)?; }
                OP_I64_OR     => { let b = self.v_pop()?; let a = self.v_pop()?; self.v_push(a | b)?; }
                OP_I64_XOR    => { let b = self.v_pop()?; let a = self.v_pop()?; self.v_push(a ^ b)?; }
                OP_I64_SHL    => { let b = self.v_pop()? as u64; let a = self.v_pop()?; self.v_push(a.wrapping_shl((b & 63) as u32))?; }
                OP_I64_SHR_S  => { let b = self.v_pop()? as u64; let a = self.v_pop()?; self.v_push(a.wrapping_shr((b & 63) as u32))?; }
                OP_I64_SHR_U  => { let b = self.v_pop()? as u64; let a = self.v_pop()? as u64; self.v_push(a.wrapping_shr((b & 63) as u32) as i64)?; }
                OP_I64_ROTL   => { let b = self.v_pop()? as u64; let a = self.v_pop()? as u64; self.v_push(a.rotate_left((b & 63) as u32) as i64)?; }
                OP_I64_ROTR   => { let b = self.v_pop()? as u64; let a = self.v_pop()? as u64; self.v_push(a.rotate_right((b & 63) as u32) as i64)?; }

                // ── type conversions ──────────────────────────────────────────
                OP_I32_WRAP_I64     => { let a = self.v_pop()?; self.v_push(a as i32 as i64)?; }
                OP_I64_EXTEND_I32_S => { let a = self.v_pop()? as i32; self.v_push(a as i64)?; }
                OP_I64_EXTEND_I32_U => { let a = self.v_pop()? as u32; self.v_push(a as i64)?; }

                // ── sign-extension (0xC0–0xC4) ────────────────────────────────
                OP_I32_EXTEND8_S  => { let v = self.v_pop()? as i32; self.v_push((v as i8)  as i32 as i64)?; }
                OP_I32_EXTEND16_S => { let v = self.v_pop()? as i32; self.v_push((v as i16) as i32 as i64)?; }
                OP_I64_EXTEND8_S  => { let v = self.v_pop()?;        self.v_push((v as i8)  as i64)?; }
                OP_I64_EXTEND16_S => { let v = self.v_pop()?;        self.v_push((v as i16) as i64)?; }
                OP_I64_EXTEND32_S => { let v = self.v_pop()?;        self.v_push((v as i32) as i64)?; }

                // ── f32/f64 constants ─────────────────────────────────────────
                0x43 => { // f32.const — 4-byte LE IEEE 754
                    let fi   = self.fdepth - 1;
                    let pc   = self.frames[fi].pc;
                    let body = self.bodies[self.frames[fi].body_idx];
                    if pc + 4 > body.len() { return Err(InterpError::MalformedCode); }
                    let bits = u32::from_le_bytes([body[pc], body[pc+1], body[pc+2], body[pc+3]]);
                    self.frames[fi].pc += 4;
                    self.v_push(bits as i64)?;
                }
                0x44 => { // f64.const — 8-byte LE IEEE 754
                    let fi   = self.fdepth - 1;
                    let pc   = self.frames[fi].pc;
                    let body = self.bodies[self.frames[fi].body_idx];
                    if pc + 8 > body.len() { return Err(InterpError::MalformedCode); }
                    let bits = u64::from_le_bytes([
                        body[pc], body[pc+1], body[pc+2], body[pc+3],
                        body[pc+4], body[pc+5], body[pc+6], body[pc+7],
                    ]);
                    self.frames[fi].pc += 8;
                    self.v_push(bits as i64)?;
                }

                // ── f32 comparisons (0x5B–0x60) ───────────────────────────────
                0x5B => { let b = f32::from_bits(self.v_pop()? as u32); let a = f32::from_bits(self.v_pop()? as u32); self.v_push(if a == b { 1 } else { 0 })?; }
                0x5C => { let b = f32::from_bits(self.v_pop()? as u32); let a = f32::from_bits(self.v_pop()? as u32); self.v_push(if a != b { 1 } else { 0 })?; }
                0x5D => { let b = f32::from_bits(self.v_pop()? as u32); let a = f32::from_bits(self.v_pop()? as u32); self.v_push(if a <  b { 1 } else { 0 })?; }
                0x5E => { let b = f32::from_bits(self.v_pop()? as u32); let a = f32::from_bits(self.v_pop()? as u32); self.v_push(if a >  b { 1 } else { 0 })?; }
                0x5F => { let b = f32::from_bits(self.v_pop()? as u32); let a = f32::from_bits(self.v_pop()? as u32); self.v_push(if a <= b { 1 } else { 0 })?; }
                0x60 => { let b = f32::from_bits(self.v_pop()? as u32); let a = f32::from_bits(self.v_pop()? as u32); self.v_push(if a >= b { 1 } else { 0 })?; }

                // ── f64 comparisons (0x61–0x66) ───────────────────────────────
                0x61 => { let b = f64::from_bits(self.v_pop()? as u64); let a = f64::from_bits(self.v_pop()? as u64); self.v_push(if a == b { 1 } else { 0 })?; }
                0x62 => { let b = f64::from_bits(self.v_pop()? as u64); let a = f64::from_bits(self.v_pop()? as u64); self.v_push(if a != b { 1 } else { 0 })?; }
                0x63 => { let b = f64::from_bits(self.v_pop()? as u64); let a = f64::from_bits(self.v_pop()? as u64); self.v_push(if a <  b { 1 } else { 0 })?; }
                0x64 => { let b = f64::from_bits(self.v_pop()? as u64); let a = f64::from_bits(self.v_pop()? as u64); self.v_push(if a >  b { 1 } else { 0 })?; }
                0x65 => { let b = f64::from_bits(self.v_pop()? as u64); let a = f64::from_bits(self.v_pop()? as u64); self.v_push(if a <= b { 1 } else { 0 })?; }
                0x66 => { let b = f64::from_bits(self.v_pop()? as u64); let a = f64::from_bits(self.v_pop()? as u64); self.v_push(if a >= b { 1 } else { 0 })?; }

                // ── f32 arithmetic (0x8B–0x98) ────────────────────────────────
                0x8B => { let a = f32::from_bits(self.v_pop()? as u32); self.v_push(a.abs().to_bits() as i64)?; }
                0x8C => { let a = f32::from_bits(self.v_pop()? as u32); self.v_push((-a).to_bits() as i64)?; }
                0x8D => { let a = f32::from_bits(self.v_pop()? as u32); self.v_push(libm::ceilf(a).to_bits() as i64)?; }
                0x8E => { let a = f32::from_bits(self.v_pop()? as u32); self.v_push(libm::floorf(a).to_bits() as i64)?; }
                0x8F => { let a = f32::from_bits(self.v_pop()? as u32); self.v_push(libm::truncf(a).to_bits() as i64)?; }
                0x90 => { let a = f32::from_bits(self.v_pop()? as u32); self.v_push(libm::rintf(a).to_bits() as i64)?; }
                0x91 => { let a = f32::from_bits(self.v_pop()? as u32); self.v_push(libm::sqrtf(a).to_bits() as i64)?; }
                0x92 => { let b = f32::from_bits(self.v_pop()? as u32); let a = f32::from_bits(self.v_pop()? as u32); self.v_push((a + b).to_bits() as i64)?; }
                0x93 => { let b = f32::from_bits(self.v_pop()? as u32); let a = f32::from_bits(self.v_pop()? as u32); self.v_push((a - b).to_bits() as i64)?; }
                0x94 => { let b = f32::from_bits(self.v_pop()? as u32); let a = f32::from_bits(self.v_pop()? as u32); self.v_push((a * b).to_bits() as i64)?; }
                0x95 => { let b = f32::from_bits(self.v_pop()? as u32); let a = f32::from_bits(self.v_pop()? as u32); self.v_push((a / b).to_bits() as i64)?; }
                0x96 => { let b = f32::from_bits(self.v_pop()? as u32); let a = f32::from_bits(self.v_pop()? as u32); self.v_push(libm::fminf(a, b).to_bits() as i64)?; }
                0x97 => { let b = f32::from_bits(self.v_pop()? as u32); let a = f32::from_bits(self.v_pop()? as u32); self.v_push(libm::fmaxf(a, b).to_bits() as i64)?; }
                0x98 => { let b = f32::from_bits(self.v_pop()? as u32); let a = f32::from_bits(self.v_pop()? as u32); self.v_push(libm::copysignf(a, b).to_bits() as i64)?; }

                // ── f64 arithmetic (0x99–0xA6) ────────────────────────────────
                0x99 => { let a = f64::from_bits(self.v_pop()? as u64); self.v_push(a.abs().to_bits() as i64)?; }
                0x9A => { let a = f64::from_bits(self.v_pop()? as u64); self.v_push((-a).to_bits() as i64)?; }
                0x9B => { let a = f64::from_bits(self.v_pop()? as u64); self.v_push(libm::ceil(a).to_bits() as i64)?; }
                0x9C => { let a = f64::from_bits(self.v_pop()? as u64); self.v_push(libm::floor(a).to_bits() as i64)?; }
                0x9D => { let a = f64::from_bits(self.v_pop()? as u64); self.v_push(libm::trunc(a).to_bits() as i64)?; }
                0x9E => { let a = f64::from_bits(self.v_pop()? as u64); self.v_push(libm::rint(a).to_bits() as i64)?; }
                0x9F => { let a = f64::from_bits(self.v_pop()? as u64); self.v_push(libm::sqrt(a).to_bits() as i64)?; }
                0xA0 => { let b = f64::from_bits(self.v_pop()? as u64); let a = f64::from_bits(self.v_pop()? as u64); self.v_push((a + b).to_bits() as i64)?; }
                0xA1 => { let b = f64::from_bits(self.v_pop()? as u64); let a = f64::from_bits(self.v_pop()? as u64); self.v_push((a - b).to_bits() as i64)?; }
                0xA2 => { let b = f64::from_bits(self.v_pop()? as u64); let a = f64::from_bits(self.v_pop()? as u64); self.v_push((a * b).to_bits() as i64)?; }
                0xA3 => { let b = f64::from_bits(self.v_pop()? as u64); let a = f64::from_bits(self.v_pop()? as u64); self.v_push((a / b).to_bits() as i64)?; }
                0xA4 => { let b = f64::from_bits(self.v_pop()? as u64); let a = f64::from_bits(self.v_pop()? as u64); self.v_push(libm::fmin(a, b).to_bits() as i64)?; }
                0xA5 => { let b = f64::from_bits(self.v_pop()? as u64); let a = f64::from_bits(self.v_pop()? as u64); self.v_push(libm::fmax(a, b).to_bits() as i64)?; }
                0xA6 => { let b = f64::from_bits(self.v_pop()? as u64); let a = f64::from_bits(self.v_pop()? as u64); self.v_push(libm::copysign(a, b).to_bits() as i64)?; }

                // ── float → integer conversions ───────────────────────────────
                0xA8 => { // i32.trunc_f32_s
                    let a = f32::from_bits(self.v_pop()? as u32);
                    if a.is_nan() || a >= 2147483648.0_f32 || a < -2147483648.0_f32 { return Err(InterpError::InvalidConversion); }
                    self.v_push(a as i32 as i64)?;
                }
                0xA9 => { // i32.trunc_f32_u
                    let a = f32::from_bits(self.v_pop()? as u32);
                    if a.is_nan() || a >= 4294967296.0_f32 || a < 0.0_f32 { return Err(InterpError::InvalidConversion); }
                    self.v_push(a as u32 as i64)?;
                }
                0xAA => { // i32.trunc_f64_s
                    let a = f64::from_bits(self.v_pop()? as u64);
                    if a.is_nan() || a >= 2147483648.0_f64 || a < -2147483648.0_f64 { return Err(InterpError::InvalidConversion); }
                    self.v_push(a as i32 as i64)?;
                }
                0xAB => { // i32.trunc_f64_u
                    let a = f64::from_bits(self.v_pop()? as u64);
                    if a.is_nan() || a >= 4294967296.0_f64 || a < 0.0_f64 { return Err(InterpError::InvalidConversion); }
                    self.v_push(a as u32 as i64)?;
                }
                0xAE => { // i64.trunc_f32_s
                    let a = f32::from_bits(self.v_pop()? as u32);
                    if a.is_nan() || a >= 9223372036854775808.0_f32 || a < -9223372036854775808.0_f32 { return Err(InterpError::InvalidConversion); }
                    self.v_push(a as i64)?;
                }
                0xAF => { // i64.trunc_f32_u
                    let a = f32::from_bits(self.v_pop()? as u32);
                    if a.is_nan() || a >= 18446744073709551616.0_f32 || a < 0.0_f32 { return Err(InterpError::InvalidConversion); }
                    self.v_push(a as u64 as i64)?;
                }
                0xB0 => { // i64.trunc_f64_s
                    let a = f64::from_bits(self.v_pop()? as u64);
                    if a.is_nan() || a >= 9223372036854775808.0_f64 || a < -9223372036854775808.0_f64 { return Err(InterpError::InvalidConversion); }
                    self.v_push(a as i64)?;
                }
                0xB1 => { // i64.trunc_f64_u
                    let a = f64::from_bits(self.v_pop()? as u64);
                    if a.is_nan() || a >= 18446744073709551616.0_f64 || a < 0.0_f64 { return Err(InterpError::InvalidConversion); }
                    self.v_push(a as u64 as i64)?;
                }
                // ── integer → float conversions ───────────────────────────────
                0xB2 => { let a = self.v_pop()? as i32; self.v_push((a as f32).to_bits() as i64)?; } // f32.convert_i32_s
                0xB3 => { let a = self.v_pop()? as u32; self.v_push((a as f32).to_bits() as i64)?; } // f32.convert_i32_u
                0xB4 => { let a = self.v_pop()?;        self.v_push((a as f32).to_bits() as i64)?; } // f32.convert_i64_s
                0xB5 => { let a = self.v_pop()? as u64; self.v_push((a as f32).to_bits() as i64)?; } // f32.convert_i64_u
                0xB6 => { let a = f64::from_bits(self.v_pop()? as u64); self.v_push((a as f32).to_bits() as i64)?; } // f32.demote_f64
                0xB7 => { let a = self.v_pop()? as i32; self.v_push((a as f64).to_bits() as i64)?; } // f64.convert_i32_s
                0xB8 => { let a = self.v_pop()? as u32; self.v_push((a as f64).to_bits() as i64)?; } // f64.convert_i32_u
                0xB9 => { let a = self.v_pop()?;        self.v_push((a as f64).to_bits() as i64)?; } // f64.convert_i64_s
                0xBA => { let a = self.v_pop()? as u64; self.v_push((a as f64).to_bits() as i64)?; } // f64.convert_i64_u
                0xBB => { let a = f32::from_bits(self.v_pop()? as u32); self.v_push((a as f64).to_bits() as i64)?; } // f64.promote_f32
                // ── reinterpret ───────────────────────────────────────────────
                0xBC => { let a = self.v_pop()? as i32; self.v_push(a.to_le_bytes().iter().fold(0u32, |acc, &b| (acc << 8) | b as u32).swap_bytes() as i64)?; } // i32.reinterpret_f32
                0xBD => {} // i64.reinterpret_f64 — bits already correct, no-op
                0xBE => {} // f32.reinterpret_i32 — bits already correct, no-op
                0xBF => {} // f64.reinterpret_i64 — bits already correct, no-op

                // ── saturating trunc (0xFC prefix) ────────────────────────────
                0xFC => {
                    let fi   = self.fdepth - 1;
                    let pc   = self.frames[fi].pc;
                    let body = self.bodies[self.frames[fi].body_idx];
                    let (sub_op, consumed) = read_u32_leb128(&body[pc..])
                        .ok_or(InterpError::MalformedCode)?;
                    self.frames[fi].pc += consumed;
                    match sub_op {
                        0 => { // i32.trunc_sat_f32_s
                            let a = f32::from_bits(self.v_pop()? as u32);
                            let v = if a.is_nan() { 0i32 } else if a >= 2147483648.0_f32 { i32::MAX } else if a < -2147483648.0_f32 { i32::MIN } else { a as i32 };
                            self.v_push(v as i64)?;
                        }
                        1 => { // i32.trunc_sat_f32_u
                            let a = f32::from_bits(self.v_pop()? as u32);
                            let v = if a.is_nan() || a < 0.0 { 0u32 } else if a >= 4294967296.0_f32 { u32::MAX } else { a as u32 };
                            self.v_push(v as i64)?;
                        }
                        2 => { // i32.trunc_sat_f64_s
                            let a = f64::from_bits(self.v_pop()? as u64);
                            let v = if a.is_nan() { 0i32 } else if a >= 2147483648.0_f64 { i32::MAX } else if a < -2147483648.0_f64 { i32::MIN } else { a as i32 };
                            self.v_push(v as i64)?;
                        }
                        3 => { // i32.trunc_sat_f64_u
                            let a = f64::from_bits(self.v_pop()? as u64);
                            let v = if a.is_nan() || a < 0.0 { 0u32 } else if a >= 4294967296.0_f64 { u32::MAX } else { a as u32 };
                            self.v_push(v as i64)?;
                        }
                        4 => { // i64.trunc_sat_f32_s
                            let a = f32::from_bits(self.v_pop()? as u32);
                            let v = if a.is_nan() { 0i64 } else if a >= 9223372036854775808.0_f32 { i64::MAX } else if a < -9223372036854775808.0_f32 { i64::MIN } else { a as i64 };
                            self.v_push(v)?;
                        }
                        5 => { // i64.trunc_sat_f32_u
                            let a = f32::from_bits(self.v_pop()? as u32);
                            let v = if a.is_nan() || a < 0.0 { 0u64 } else if a >= 18446744073709551616.0_f32 { u64::MAX } else { a as u64 };
                            self.v_push(v as i64)?;
                        }
                        6 => { // i64.trunc_sat_f64_s
                            let a = f64::from_bits(self.v_pop()? as u64);
                            let v = if a.is_nan() { 0i64 } else if a >= 9223372036854775808.0_f64 { i64::MAX } else if a < -9223372036854775808.0_f64 { i64::MIN } else { a as i64 };
                            self.v_push(v)?;
                        }
                        7 => { // i64.trunc_sat_f64_u
                            let a = f64::from_bits(self.v_pop()? as u64);
                            let v = if a.is_nan() || a < 0.0 { 0u64 } else if a >= 18446744073709551616.0_f64 { u64::MAX } else { a as u64 };
                            self.v_push(v as i64)?;
                        }
                        // ── bulk-memory ops ───────────────────────────────────────
                        8 => { // memory.init <seg> 0x00 — copy from passive data segment
                            let fi = self.fdepth - 1;
                            let pc = self.frames[fi].pc;
                            let body = self.bodies[self.frames[fi].body_idx];
                            let (_seg, n) = read_u32_leb128(&body[pc..]).ok_or(InterpError::MalformedCode)?;
                            self.frames[fi].pc += n + 1; // skip segment idx + reserved 0x00
                            let count = self.v_pop()? as usize;
                            let _src  = self.v_pop()? as usize;
                            let _dst  = self.v_pop()? as usize;
                            // Passive segments are already applied at instantiation;
                            // a zero-length init is a no-op, non-zero is unsupported.
                            if count != 0 {
                                crate::println!("[wasm] memory.init with count={} not supported", count);
                                return Err(InterpError::UnknownOpcode(0xFC));
                            }
                        }
                        9 => { // data.drop <seg> — mark segment as dropped (no-op)
                            let fi = self.fdepth - 1;
                            let pc = self.frames[fi].pc;
                            let body = self.bodies[self.frames[fi].body_idx];
                            let (_seg, n) = read_u32_leb128(&body[pc..]).ok_or(InterpError::MalformedCode)?;
                            self.frames[fi].pc += n;
                        }
                        10 => { // memory.copy 0x00 0x00
                            let fi = self.fdepth - 1;
                            self.frames[fi].pc += 2; // skip two reserved 0x00 bytes
                            let n   = self.v_pop()? as usize;
                            let src = self.v_pop()? as usize;
                            let dst = self.v_pop()? as usize;
                            let limit = self.current_pages as usize * PAGE_SIZE;
                            if src.saturating_add(n) > limit || dst.saturating_add(n) > limit {
                                return Err(InterpError::MemOutOfBounds);
                            }
                            self.mem.copy_within(src..src + n, dst);
                        }
                        11 => { // memory.fill 0x00
                            let fi = self.fdepth - 1;
                            self.frames[fi].pc += 1; // skip reserved 0x00 byte
                            let n   = self.v_pop()? as usize;
                            let val = self.v_pop()? as u8;
                            let dst = self.v_pop()? as usize;
                            let limit = self.current_pages as usize * PAGE_SIZE;
                            if dst.saturating_add(n) > limit {
                                return Err(InterpError::MemOutOfBounds);
                            }
                            self.mem[dst..dst + n].fill(val);
                        }
                        sub => {
                            crate::println!("[wasm] unknown 0xFC sub-opcode: 0x{:02X} ({})", sub, sub);
                            let _ = sub;
                            return Err(InterpError::UnknownOpcode(0xFC));
                        }
                    }
                }

                // ── call ──────────────────────────────────────────────────────
                OP_CALL => {
                    let fi   = self.fdepth - 1;
                    let pc   = self.frames[fi].pc;
                    let body = self.bodies[self.frames[fi].body_idx];
                    let (callee, consumed) = read_u32_leb128(&body[pc..])
                        .ok_or(InterpError::MalformedCode)?;
                    self.frames[fi].pc += consumed;
                    let callee = callee as usize;
                    if callee < self.import_count {
                        let host = self.host_fns[callee].ok_or(InterpError::ImportNotFound)?;
                        host(&mut self.vstack, &mut self.vsp, &mut *self.mem)?;
                    } else {
                        let body_idx = callee - self.import_count;
                        if body_idx >= self.body_count { return Err(InterpError::FuncIndexOutOfRange); }
                        self.push_frame(body_idx)?;
                    }
                }

                // ── call_indirect <type_idx> <table_idx> ──────────────────────
                OP_CALL_INDIRECT => {
                    let fi   = self.fdepth - 1;
                    let pc   = self.frames[fi].pc;
                    let body = self.bodies[self.frames[fi].body_idx];
                    let (expected_type, n1) = read_u32_leb128(&body[pc..])
                        .ok_or(InterpError::MalformedCode)?;
                    let (_table_idx, n2) = read_u32_leb128(&body[pc + n1..])
                        .ok_or(InterpError::MalformedCode)?;
                    self.frames[fi].pc += n1 + n2;

                    let i = self.v_pop()? as u32 as usize;
                    if i >= self.table_size || self.table[i] == NULL_FUNC {
                        return Err(InterpError::IndirectCallNull);
                    }
                    let callee = self.table[i] as usize;

                    // Runtime type check.
                    let actual_type = if callee < MAX_FUNCS { self.all_func_types[callee] } else { usize::MAX };
                    if actual_type != expected_type as usize {
                        return Err(InterpError::IndirectCallTypeMismatch);
                    }

                    if callee < self.import_count {
                        let host = self.host_fns[callee].ok_or(InterpError::ImportNotFound)?;
                        host(&mut self.vstack, &mut self.vsp, &mut *self.mem)?;
                    } else {
                        let body_idx = callee - self.import_count;
                        if body_idx >= self.body_count { return Err(InterpError::FuncIndexOutOfRange); }
                        self.push_frame(body_idx)?;
                    }
                }

                other => {
                    crate::println!("[wasm] unknown opcode: 0x{:02X} ({})", other, other);
                    return Err(InterpError::UnknownOpcode(other));
                }
            }
        }
        Ok(())
    }
}

// ── br helper (shared by OP_BR and OP_BR_IF) ─────────────────────────────────

/// Collapse the current call frame's stack to exactly result_count values,
/// moving them to sit just above vsp_base, then pop the frame.
fn do_return(interp: &mut Interpreter) {
    let fi           = interp.fdepth - 1;
    let result_count = interp.frames[fi].result_count;
    let vsp_base     = interp.frames[fi].vsp_base;
    // Copy the top result_count values down to vsp_base.
    let src = interp.vsp.saturating_sub(result_count);
    for i in 0..result_count {
        interp.vstack[vsp_base + i] = interp.vstack[src + i];
    }
    interp.vsp        = vsp_base + result_count;
    interp.ctrl_depth = interp.frames[fi].ctrl_base;
    interp.fdepth    -= 1;
}

fn do_br(interp: &mut Interpreter, n: usize) -> Result<(), InterpError> {
    let fi        = interp.fdepth - 1;
    let ctrl_base = interp.frames[fi].ctrl_base;
    let available = interp.ctrl_depth - ctrl_base;

    if n >= available {
        // Branch past function boundary → implicit return.
        do_return(interp);
        return Ok(());
    }

    let target_abs = interp.ctrl_depth - 1 - n;
    let cf = interp.ctrl[target_abs];

    match cf.kind {
        BlockKind::Loop => {
            // Keep the loop frame; jump to loop start.
            interp.ctrl_depth = target_abs + 1;
            interp.frames[fi].pc = cf.pc_start;
        }
        BlockKind::Block | BlockKind::If => {
            // Pop the block/if frame; jump past its end.
            interp.ctrl_depth = target_abs;
            interp.frames[fi].pc = cf.end_pc + 1;
        }
    }
    Ok(())
}

// ── Jump-table helpers ────────────────────────────────────────────────────────

/// Map `(body_idx, scan_start)` to a slot index (power-of-two mask).
#[inline(always)]
fn jump_hash(body_idx: usize, scan_start: usize) -> usize {
    let h = (body_idx as u64).wrapping_mul(2654435761)
        ^ (scan_start as u64).wrapping_mul(2246822519);
    (h ^ (h >> 16)) as usize & (JUMP_SLOTS - 1)
}

/// Insert one entry into the jump table using linear probing.
/// Silently drops if body_idx exceeds u16 range or the table is full.
fn jump_insert(
    table:      &mut [JumpSlot; JUMP_SLOTS],
    body_idx:   usize,
    scan_start: usize,
    end_pc:     usize,
    else_pc:    usize,
) {
    if body_idx >= u16::MAX as usize { return; }
    let mut i = jump_hash(body_idx, scan_start);
    for _ in 0..JUMP_SLOTS {
        if table[i].key_body == u16::MAX {
            table[i] = JumpSlot {
                key_body: body_idx as u16,
                _pad:     0,
                key_pc:   scan_start as u32,
                end_pc:   end_pc as u32,
                else_pc:  if else_pc == NO_ELSE { u32::MAX } else { else_pc as u32 },
            };
            return;
        }
        i = (i + 1) & (JUMP_SLOTS - 1);
    }
    // Table full — entry dropped; dispatch will fall back to scan_block_end.
}

/// Walk one function body and populate the jump table for every block/loop/if
/// found inside it (including nested ones).  Uses the same immediate-skipping
/// rules as `scan_block_end` so raw immediate bytes cannot be mistaken for
/// structural opcodes.
fn pre_scan_body_jumps(
    body:     &[u8],
    body_idx: usize,
    table:    &mut [JumpSlot; JUMP_SLOTS],
) {
    let mut i = 0usize;
    while i < body.len() {
        let op = body[i];
        i += 1;
        match op {
            // Structural opcodes: record jump entry, then continue INSIDE the
            // body so that nested blocks are also recorded in a single pass.
            0x02 | 0x03 | 0x04 => {
                // i now points to the blocktype byte.
                if i >= body.len() { return; }
                let scan_start = i + 1; // first byte of block body
                i += 1; // advance past blocktype
                if let Some((end_pc, else_pc)) = scan_block_end(body, scan_start) {
                    jump_insert(table, body_idx, scan_start, end_pc, else_pc);
                }
                // Do NOT skip to end_pc: continue from scan_start (i) so that
                // nested blocks inside this body are discovered.
            }
            // Single LEB-128 immediate.
            0x0C | 0x0D |            // br, br_if
            0x10 |                   // call
            0x20 | 0x21 | 0x22 |    // local.get/set/tee
            0x23 | 0x24 |            // global.get/set
            0x41 | 0x42 => {         // i32.const, i64.const
                i += skip_leb128(body, i).unwrap_or(0);
            }
            // call_indirect: two LEB-128.
            0x11 => {
                i += skip_leb128(body, i).unwrap_or(0);
                i += skip_leb128(body, i).unwrap_or(0);
            }
            // br_table: count + (count+1) labels.
            0x0E => {
                if let Some((count, n)) = read_u32_leb128(&body[i..]) {
                    i += n;
                    for _ in 0..=count {
                        i += skip_leb128(body, i).unwrap_or(1);
                    }
                } else { return; }
            }
            // Memory load/store: align + offset (two LEB-128).
            0x28..=0x3E => {
                i += skip_leb128(body, i).unwrap_or(0);
                i += skip_leb128(body, i).unwrap_or(0);
            }
            0x3F | 0x40 => { i += 1; } // memory.size/grow: reserved byte
            0x43 => { i += 4; }         // f32.const: 4-byte IEEE literal
            0x44 => { i += 8; }         // f64.const: 8-byte IEEE literal
            // 0xFC prefix (saturating trunc, bulk-memory): sub-opcode LEB128.
            0xFC => { i += skip_leb128(body, i).unwrap_or(0); }
            // All other opcodes: no immediates.
            _ => {}
        }
    }
}

// ── Scan ahead for matching end/else ─────────────────────────────────────────

/// Starting at `start` (right after opcode + blocktype), scan forward to find
/// the matching `end` and optional `else`.  Returns `(end_pc, else_pc)` where
/// both are positions OF the opcode byte.  `else_pc` is `NO_ELSE` if absent.
///
/// Correctly skips all LEB-128 and fixed-width immediates so their bytes are
/// never mistaken for structural opcodes.  Safe because 0x02/0x03/0x04/0x0B
/// all have the high bit clear and therefore can only appear as the *final*
/// byte of a LEB-128 (never as a continuation byte).
fn scan_block_end(body: &[u8], start: usize) -> Option<(usize, usize)> {
    let mut depth    = 1usize;
    let mut i        = start;
    let mut else_pc  = NO_ELSE;

    while i < body.len() {
        let op = body[i];
        i += 1;
        match op {
            // Nested structured blocks — consume blocktype (1 byte MVP).
            0x02 | 0x03 | 0x04 => { i += 1; depth += 1; }
            0x05 => { if depth == 1 { else_pc = i - 1; } }
            0x0B => {
                depth -= 1;
                if depth == 0 { return Some((i - 1, else_pc)); }
            }
            // Single LEB-128 immediate.
            0x0C | 0x0D |            // br, br_if
            0x10 |                   // call
            0x20 | 0x21 | 0x22 |    // local.get/set/tee
            0x23 | 0x24 |            // global.get/set
            0x41 | 0x42 => {         // i32.const, i64.const
                i += skip_leb128(body, i)?;
            }
            // call_indirect: two LEB-128.
            0x11 => {
                i += skip_leb128(body, i)?;
                i += skip_leb128(body, i)?;
            }
            // br_table: count + (count+1) labels.
            0x0E => {
                let (count, n) = read_u32_leb128(&body[i..])?;
                i += n;
                for _ in 0..=count { i += skip_leb128(body, i)?; }
            }
            // Memory instructions: align + offset (two LEB-128).
            0x28..=0x3E => {
                i += skip_leb128(body, i)?;
                i += skip_leb128(body, i)?;
            }
            // memory.size / memory.grow: one reserved byte.
            0x3F | 0x40 => { i += 1; }
            // f32.const: 4 bytes literal.
            0x43 => { i += 4; }
            // f64.const: 8 bytes literal.
            0x44 => { i += 8; }
            // 0xFC prefix (saturating trunc, bulk-memory, etc.): consume sub-opcode.
            // Without this, sub-opcodes 0x02–0x05 would be mis-read as block/loop/if/else.
            0xFC => { i += skip_leb128(body, i)?; }
            // All other opcodes have no immediates.
            _ => {}
        }
    }
    None
}

/// Advance past one LEB-128 value starting at `bytes[start]`.
/// Returns the number of bytes consumed, or None on truncation.
fn skip_leb128(bytes: &[u8], start: usize) -> Option<usize> {
    let mut i = start;
    while i < bytes.len() {
        let b = bytes[i]; i += 1;
        if b & 0x80 == 0 { return Some(i - start); }
    }
    None
}

// ── Memory immediate helper ───────────────────────────────────────────────────

fn read_memarg(bytes: &[u8]) -> Option<(u32, usize)> {
    let (_align, n1) = read_u32_leb128(bytes)?;
    let (offset, n2) = read_u32_leb128(&bytes[n1..])?;
    Some((offset, n1 + n2))
}

// ── Section parsers ───────────────────────────────────────────────────────────

fn parse_type_section(
    bytes:      &[u8],
    param_out:  &mut [usize; MAX_TYPES],
    result_out: &mut [usize; MAX_TYPES],
) -> Result<usize, InterpError> {
    let mut cur = 0usize;
    let (count, n) = read_u32_leb128(bytes).ok_or(InterpError::MalformedCode)?;
    cur += n;
    let count = count as usize;
    if count > MAX_TYPES { return Err(InterpError::TooManyTypes); }

    for i in 0..count {
        if cur >= bytes.len() { return Err(InterpError::MalformedCode); }
        cur += 1; // skip 0x60 func-type marker
        let (param_count, n) = read_u32_leb128(&bytes[cur..]).ok_or(InterpError::MalformedCode)?;
        cur += n;
        param_out[i] = param_count as usize;
        cur += param_count as usize; // skip param valtypes (1 byte each)
        let (result_count, n) = read_u32_leb128(&bytes[cur..]).ok_or(InterpError::MalformedCode)?;
        cur += n;
        result_out[i] = result_count as usize;
        cur += result_count as usize; // skip result valtypes
    }
    Ok(count)
}

fn parse_function_section(bytes: &[u8], out: &mut [usize; MAX_FUNCS]) -> Result<(), InterpError> {
    let mut cur = 0usize;
    let (count, n) = read_u32_leb128(bytes).ok_or(InterpError::MalformedCode)?;
    cur += n;
    let count = count as usize;
    if count > MAX_FUNCS { return Err(InterpError::TooManyFuncs); }

    for i in 0..count {
        let (type_idx, n) = read_u32_leb128(&bytes[cur..]).ok_or(InterpError::MalformedCode)?;
        cur += n;
        out[i] = type_idx as usize;
    }
    Ok(())
}

fn parse_code_section<'a>(
    bytes: &'a [u8],
    bodies: &mut [&'a [u8]; MAX_FUNCS],
    local_counts: &mut [usize; MAX_FUNCS],
) -> Result<usize, InterpError> {
    let mut cur = 0usize;
    let (count, n) = read_u32_leb128(&bytes[cur..]).ok_or(InterpError::MalformedCode)?;
    cur += n;
    let count = count as usize;
    if count > MAX_FUNCS { return Err(InterpError::TooManyFuncs); }

    for i in 0..count {
        let (body_size, n) = read_u32_leb128(&bytes[cur..]).ok_or(InterpError::MalformedCode)?;
        cur += n;
        let entry_start = cur;
        let entry_end   = cur + body_size as usize;
        if entry_end > bytes.len() { return Err(InterpError::MalformedCode); }
        let entry = &bytes[entry_start..entry_end];

        let (local_groups, n) = read_u32_leb128(entry).ok_or(InterpError::MalformedCode)?;
        let mut lc = n;
        let mut total_declared = 0usize;
        for _ in 0..local_groups {
            let (group_count, n) = read_u32_leb128(&entry[lc..]).ok_or(InterpError::MalformedCode)?;
            lc += n + 1;
            total_declared += group_count as usize;
        }
        if lc > entry.len() { return Err(InterpError::MalformedCode); }

        bodies[i]       = &entry[lc..];
        local_counts[i] = total_declared;
        cur = entry_end;
    }
    Ok(count)
}

/// Scan the import section and record the type index for each function import
/// into `out[0..func_import_count]` (skipping table/memory/global imports).
fn parse_import_func_types(bytes: &[u8], out: &mut [usize; MAX_FUNCS]) -> Result<usize, InterpError> {
    let mut cur = 0usize;
    let (count, n) = read_u32_leb128(bytes).ok_or(InterpError::MalformedCode)?;
    cur += n;
    let mut func_count = 0usize;

    for _ in 0..count as usize {
        // module name
        let (mod_len, n) = read_u32_leb128(&bytes[cur..]).ok_or(InterpError::MalformedCode)?;
        cur += n + mod_len as usize;
        // field name
        let (name_len, n) = read_u32_leb128(&bytes[cur..]).ok_or(InterpError::MalformedCode)?;
        cur += n + name_len as usize;
        // import kind
        if cur >= bytes.len() { return Err(InterpError::MalformedCode); }
        let kind = bytes[cur]; cur += 1;
        match kind {
            0 => { // func: type index
                let (type_idx, n) = read_u32_leb128(&bytes[cur..]).ok_or(InterpError::MalformedCode)?;
                cur += n;
                if func_count < MAX_FUNCS { out[func_count] = type_idx as usize; }
                func_count += 1;
            }
            1 => { // table: reftype (1 byte) + limits
                cur += 1;
                if cur >= bytes.len() { return Err(InterpError::MalformedCode); }
                let flag = bytes[cur]; cur += 1;
                let (_, n) = read_u32_leb128(&bytes[cur..]).ok_or(InterpError::MalformedCode)?;
                cur += n;
                if flag != 0 {
                    let (_, n) = read_u32_leb128(&bytes[cur..]).ok_or(InterpError::MalformedCode)?;
                    cur += n;
                }
            }
            2 => { // memory: limits
                if cur >= bytes.len() { return Err(InterpError::MalformedCode); }
                let flag = bytes[cur]; cur += 1;
                let (_, n) = read_u32_leb128(&bytes[cur..]).ok_or(InterpError::MalformedCode)?;
                cur += n;
                if flag != 0 {
                    let (_, n) = read_u32_leb128(&bytes[cur..]).ok_or(InterpError::MalformedCode)?;
                    cur += n;
                }
            }
            3 => { cur += 2; } // global: valtype + mutability
            _ => return Err(InterpError::MalformedCode),
        }
    }
    Ok(func_count)
}

/// Parse element section (kind=0 active segments only) and populate the table.
fn parse_element_section(
    bytes:      &[u8],
    table:      &mut [u32; MAX_TABLE],
    table_size: &mut usize,
) -> Result<(), InterpError> {
    let mut cur = 0usize;
    let (seg_count, n) = read_u32_leb128(bytes).ok_or(InterpError::MalformedCode)?;
    cur += n;

    for _ in 0..seg_count as usize {
        let (kind, n) = read_u32_leb128(&bytes[cur..]).ok_or(InterpError::MalformedCode)?;
        cur += n;
        match kind {
            0 => {
                // Active segment: table 0, implicit funcref.
                // Offset init expr: i32.const <offset> end
                if cur >= bytes.len() || bytes[cur] != 0x41 { return Err(InterpError::MalformedCode); }
                cur += 1;
                let (offset, n) = read_i32_leb128(&bytes[cur..]).ok_or(InterpError::MalformedCode)?;
                cur += n;
                if cur >= bytes.len() || bytes[cur] != 0x0B { return Err(InterpError::MalformedCode); }
                cur += 1;

                let (elem_count, n) = read_u32_leb128(&bytes[cur..]).ok_or(InterpError::MalformedCode)?;
                cur += n;
                let base = offset as usize;
                for j in 0..elem_count as usize {
                    let (func_idx, n) = read_u32_leb128(&bytes[cur..]).ok_or(InterpError::MalformedCode)?;
                    cur += n;
                    let ti = base + j;
                    if ti < MAX_TABLE {
                        table[ti] = func_idx;
                        if ti + 1 > *table_size { *table_size = ti + 1; }
                    }
                }
            }
            _ => break, // unsupported segment kind — stop, don't error
        }
    }
    Ok(())
}

fn parse_global_section(
    bytes:       &[u8],
    vals:        &mut [i64; MAX_GLOBALS],
    mutability:  &mut [bool; MAX_GLOBALS],
) -> Result<usize, InterpError> {
    let mut cur = 0usize;
    let (count, n) = read_u32_leb128(bytes).ok_or(InterpError::MalformedCode)?;
    cur += n;
    let count = count as usize;
    if count > MAX_GLOBALS { return Err(InterpError::TooManyGlobals); }

    for i in 0..count {
        // valtype (1 byte) + mutability flag (1 byte: 0x00=const, 0x01=var)
        if cur + 2 > bytes.len() { return Err(InterpError::MalformedCode); }
        cur += 1; // skip valtype
        mutability[i] = bytes[cur] == 0x01;
        cur += 1;
        // Init expression: i32.const or i64.const, then end (0x0B)
        if cur >= bytes.len() { return Err(InterpError::MalformedCode); }
        let init_op = bytes[cur]; cur += 1;
        let val = match init_op {
            0x41 => { // i32.const
                let (v, n) = read_i32_leb128(&bytes[cur..]).ok_or(InterpError::MalformedCode)?;
                cur += n;
                v as i64
            }
            0x42 => { // i64.const
                let (v, n) = read_i64_leb128(&bytes[cur..]).ok_or(InterpError::MalformedCode)?;
                cur += n;
                v
            }
            _ => return Err(InterpError::MalformedCode),
        };
        if cur >= bytes.len() || bytes[cur] != 0x0B { return Err(InterpError::MalformedCode); }
        cur += 1;
        vals[i] = val;
    }
    Ok(count)
}

// ── Signed LEB-128 ────────────────────────────────────────────────────────────

/// Decode a signed 64-bit LEB-128 integer from the front of `bytes`.
///
/// Returns `Some((value, bytes_consumed))` on success, or `None` if the
/// encoding is truncated or would overflow an `i64`.
/// Used for `i64.const` immediates and global initializers.
pub fn read_i64_leb128(bytes: &[u8]) -> Option<(i64, usize)> {
    let mut result: i64 = 0;
    let mut shift: u32  = 0;
    for (i, &byte) in bytes.iter().enumerate() {
        if shift >= 70 { return None; }
        result |= ((byte & 0x7F) as i64) << shift;
        shift += 7;
        if byte & 0x80 == 0 {
            if shift < 64 && (byte & 0x40) != 0 {
                result |= !0i64 << shift;
            }
            return Some((result, i + 1));
        }
    }
    None
}

/// Decode a signed 32-bit LEB-128 integer from the front of `bytes`.
///
/// Returns `Some((value, bytes_consumed))` on success, or `None` if the
/// encoding is truncated or would overflow an `i32`.
/// Used for `i32.const` immediates and data-segment offsets.
pub fn read_i32_leb128(bytes: &[u8]) -> Option<(i32, usize)> {
    let mut result: i32 = 0;
    let mut shift: u32  = 0;
    for (i, &byte) in bytes.iter().enumerate() {
        if shift >= 35 { return None; }
        result |= ((byte & 0x7F) as i32) << shift;
        shift += 7;
        if byte & 0x80 == 0 {
            if shift < 32 && (byte & 0x40) != 0 {
                result |= !0i32 << shift;
            }
            return Some((result, i + 1));
        }
    }
    None
}
