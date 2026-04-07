//! Shell command dispatcher, tokenizer, history ring buffer, and parse helpers.
//!
//! The entry point for each user command is [`run_command`].  The shell is
//! driven non-blockingly by [`input::poll_once`] from the scheduler; blocking
//! keyboard reads are used only by WASM host functions (`read_char`, `read_line`).

pub mod input;
mod commands;
mod command_line_editor;
use alloc::vec::Vec;
use alloc::vec;
use crate::alloc::string::ToString;
use alloc::string::String;



// ─── Working directory ────────────────────────────────────────────────────────

const CWD_MAX: usize = 128;
static mut CWD_BUF: [u8; CWD_MAX] = [0u8; CWD_MAX];
static mut CWD_LEN: usize = 0; // 0 = uninitialised; lazily set to "/" on first use

pub(crate) fn get_cwd() -> &'static str {
    unsafe {
        if CWD_LEN == 0 {
            CWD_BUF[0] = b'/';
            CWD_LEN = 1;
        }
        core::str::from_utf8(&CWD_BUF[..CWD_LEN]).unwrap_or("/")
    }
}

pub(crate) fn set_cwd(path: &str) {
    unsafe {
        let bytes = path.as_bytes();
        let len = bytes.len().min(CWD_MAX);
        CWD_BUF[..len].copy_from_slice(&bytes[..len]);
        CWD_LEN = len;
    }
}

// ─── Constants ────────────────────────────────────────────────────────────────

const MAX_ARGS:    usize = 8;
const HISTORY_LEN: usize = 16;
const MAX_LINE:    usize = 128;

// ─── Error type ───────────────────────────────────────────────────────────────

#[derive(Debug, PartialEq)]
pub enum ShellError {
    TooManyArgs,
}

// ─── History ──────────────────────────────────────────────────────────────────

struct HistoryBuf {
    lines: [[u8; MAX_LINE]; HISTORY_LEN],
    lens:  [usize; HISTORY_LEN],
    head:  usize,
    pub len:   usize,
}

impl HistoryBuf {
    const fn new() -> Self {
        Self {
            lines: [[0u8; MAX_LINE]; HISTORY_LEN],
            lens:  [0; HISTORY_LEN],
            head:  0,
            len:   0,
        }
    }

    fn push(&mut self, s: &str) {
        let bytes = s.as_bytes();
        let copy_len = bytes.len().min(MAX_LINE);
        self.lines[self.head][..copy_len].copy_from_slice(&bytes[..copy_len]);
        self.lens[self.head] = copy_len;
        self.head = (self.head + 1) % HISTORY_LEN;
        if self.len < HISTORY_LEN { self.len += 1; }
    }

    fn get(&self, i: usize) -> &str {
        let start = if self.len < HISTORY_LEN { 0 } else { self.head };
        let idx = (start + i) % HISTORY_LEN;
        let len = self.lens[idx];
        core::str::from_utf8(&self.lines[idx][..len]).unwrap_or("")
    }
}

static mut HISTORY: HistoryBuf = HistoryBuf::new();

/// Return the number of entries currently in the history ring buffer (0–16).
pub(crate) fn history_len() -> usize {
    unsafe { (*core::ptr::addr_of!(HISTORY)).len }
}

/// Return the `i`-th history entry (0 = oldest visible).
///
/// Returns an empty string if `i` is out of range.
pub(crate) fn history_get(i: usize) -> &'static str {
    unsafe { (*core::ptr::addr_of!(HISTORY)).get(i) }
}

// ─── Entry point ─────────────────────────────────────────────────────────────

/// Tokenize and dispatch one shell command line.
///
/// Lines that are empty or start with `'#'` are silently ignored.
/// Non-empty lines are added to the history ring buffer before dispatch.
/// Unknown command names print `"unknown command: <name>"`.
macro_rules! define_commands {
    ( $argv:ident, $argc:ident; $( $name:literal => $handler:expr ),* $(,)? ) => {
        pub fn command_names() -> Vec<String> {
            let mut names = vec![ $( $name.to_string() ),* ];
            let mut wasm_names = Vec::new();
            crate::fs::for_each_file(|name, _size| {
                if name.ends_with(".wasm") {
                    if let Some(stem) = name.strip_suffix(".wasm") {
                        wasm_names.push(stem.to_string());
                    }
                }
            });
            names.extend(wasm_names);
            names
        }

        pub fn run_command(line: &str) {
            if line.is_empty() || line.starts_with('#') {
                return;
            }
            unsafe { (*core::ptr::addr_of_mut!(HISTORY)).push(line); }
            let mut $argv = [""; MAX_ARGS];
            let $argc = match tokenize(line, &mut $argv) {
                Ok(n)  => n,
                Err(_) => { crate::println!("error: too many arguments"); return; }
            };
            if $argc == 0 { return; }

            match $argv[0] {
                $( $name => $handler, )*
                _ => {
                    let mut wasm_buf = [0u8; MAX_LINE + 5];
                    let nb = $argv[0].as_bytes();
                    wasm_buf[..nb.len()].copy_from_slice(nb);
                    wasm_buf[nb.len()..nb.len() + 5].copy_from_slice(b".wasm");
                    let wasm_name = core::str::from_utf8(&wasm_buf[..nb.len() + 5]).unwrap_or("");
                    if !wasm_name.is_empty() && crate::fs::find_file(wasm_name).is_some() {
                        commands::tasks::task_run_with(wasm_name, &$argv[1..$argc]);
                    } else {
                        crate::println!("unknown command: {}", $argv[0]);
                    }
                }
            }
        }
    };
}

define_commands! {
    argv, argc;
    "help"      => commands::help::run(),
    "echo"      => commands::echo::run(&argv[1..argc]),
    "history"   => commands::history::run(),
    "clear"     => commands::clear::run(),
    "ls"        => commands::ls::run(),
    "rm"        => commands::rm::run(&argv[1..argc]),
    "save"      => commands::save::run(),
    "write"     => commands::write::run(&argv[1..argc]),
    "edit"      => commands::edit::run(&argv[1..argc]),
    "asm"       => commands::asm::run(&argv[1..argc]),
    "cat"       => commands::cat::run(&argv[1..argc]),
    "cd"        => commands::cd::run(&argv[1..argc]),
    "mkdir"     => commands::mkdir::run(&argv[1..argc]),
    "df"        => commands::df::run(),
    "info"      => commands::info::run(argv.get(1).copied().unwrap_or("")),
    "run"       => commands::run::run(&argv[1..argc]),
    "ps"        => commands::ps::run(),
    "task-run"  => commands::tasks::task_run(&argv[1..argc]),
    "task-kill" => commands::tasks::task_kill(&argv[1..argc]),
    "tasks"     => commands::tasks::list(),
}

// pub fn run_command(line: &str) {
//     if line.is_empty() || line.starts_with('#') {
//         return;
//     }

//     unsafe { (*core::ptr::addr_of_mut!(HISTORY)).push(line); }

//     let mut argv = [""; MAX_ARGS];
//     let argc = match tokenize(line, &mut argv) {
//         Ok(n)  => n,
//         Err(_) => { crate::println!("error: too many arguments"); return; }
//     };
//     if argc == 0 { return; }

//     match argv[0] {
//         "help"      => commands::help::run(),
//         "echo"      => commands::echo::run(&argv[1..argc]),
//         "history"   => commands::history::run(),
//         "clear"     => commands::clear::run(),
//         "ls"        => commands::ls::run(),
//         "rm"        => commands::rm::run(&argv[1..argc]),
//         "save"      => commands::save::run(),
//         "write"     => commands::write::run(&argv[1..argc]),
//         "edit"      => commands::edit::run(&argv[1..argc]),
//         "asm"       => commands::asm::run(&argv[1..argc]),
//         "cat"       => commands::cat::run(&argv[1..argc]),
//         "cd"        => commands::cd::run(&argv[1..argc]),
//         "mkdir"     => commands::mkdir::run(&argv[1..argc]),
//         "df"        => commands::df::run(),
//         "info"      => commands::info::run(argv.get(1).copied().unwrap_or("")),
//         "run"       => commands::run::run(&argv[1..argc]),
//         "ps"        => commands::ps::run(),
//         "task-run"  => commands::tasks::task_run(&argv[1..argc]),
//         "task-kill" => commands::tasks::task_kill(&argv[1..argc]),
//         "tasks"     => commands::tasks::list(),
//         _ => {
//             // If <name>.wasm exists on the filesystem, auto-spawn it as a task.
//             let mut wasm_buf = [0u8; MAX_LINE + 5];
//             let nb = argv[0].as_bytes();
//             wasm_buf[..nb.len()].copy_from_slice(nb);
//             wasm_buf[nb.len()..nb.len() + 5].copy_from_slice(b".wasm");
//             let wasm_name = core::str::from_utf8(&wasm_buf[..nb.len() + 5]).unwrap_or("");
//             if !wasm_name.is_empty() && crate::fs::find_file(wasm_name).is_some() {
//                 commands::tasks::task_run_with(wasm_name, &argv[1..argc]);
//             } else {
//                 crate::println!("unknown command: {}", argv[0]);
//             }
//         }
//     }
// }

// ─── Tokenizer ────────────────────────────────────────────────────────────────

fn tokenize<'a>(line: &'a str, out: &mut [&'a str; MAX_ARGS]) -> Result<usize, ShellError> {
    let bytes = line.as_bytes();
    let mut count = 0;
    let mut i = 0;

    while i < bytes.len() {
        while i < bytes.len() && (bytes[i] == b' ' || bytes[i] == b'\t') { i += 1; }
        if i >= bytes.len() { break; }

        let (start, end);
        if bytes[i] == b'"' {
            i += 1;
            start = i;
            while i < bytes.len() && bytes[i] != b'"' { i += 1; }
            end = i;
            if i < bytes.len() { i += 1; }
        } else {
            start = i;
            while i < bytes.len() && bytes[i] != b' ' && bytes[i] != b'\t' { i += 1; }
            end = i;
        }

        if count >= MAX_ARGS { return Err(ShellError::TooManyArgs); }
        out[count] = &line[start..end];
        count += 1;
    }
    Ok(count)
}

// ─── Parse helpers ────────────────────────────────────────────────────────────

/// Parse a decimal integer string (with optional leading `-`) into an `i32`.
///
/// Returns `None` on empty input, non-digit characters, or overflow.
pub(crate) fn parse_i32(s: &str) -> Option<i32> {
    let bytes = s.as_bytes();
    if bytes.is_empty() { return None; }
    let (negative, start) = if bytes[0] == b'-' { (true, 1) } else { (false, 0) };
    if start >= bytes.len() { return None; }
    let mut val = 0i32;
    for &b in &bytes[start..] {
        if b < b'0' || b > b'9' { return None; }
        val = val.checked_mul(10)?.checked_add((b - b'0') as i32)?;
    }
    Some(if negative { -val } else { val })
}

/// Parse a non-negative decimal integer string into a `usize`.
///
/// Returns `None` on empty input, non-digit characters, or overflow.
pub(crate) fn parse_usize(s: &str) -> Option<usize> {
    let bytes = s.as_bytes();
    if bytes.is_empty() { return None; }
    let mut val = 0usize;
    for &b in bytes {
        if b < b'0' || b > b'9' { return None; }
        val = val.checked_mul(10)?.checked_add((b - b'0') as usize)?;
    }
    Some(val)
}
