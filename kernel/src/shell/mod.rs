// shell/mod.rs — command dispatcher, tokenizer, history, parse helpers

pub mod input;
mod commands;

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

pub(crate) fn history_len() -> usize {
    unsafe { (*core::ptr::addr_of!(HISTORY)).len }
}

pub(crate) fn history_get(i: usize) -> &'static str {
    unsafe { (*core::ptr::addr_of!(HISTORY)).get(i) }
}

// ─── Entry point ─────────────────────────────────────────────────────────────

pub fn run_command(line: &str) {
    if line.is_empty() || line.starts_with('#') {
        return;
    }

    unsafe { (*core::ptr::addr_of_mut!(HISTORY)).push(line); }

    let mut argv = [""; MAX_ARGS];
    let argc = match tokenize(line, &mut argv) {
        Ok(n)  => n,
        Err(_) => { crate::println!("error: too many arguments"); return; }
    };
    if argc == 0 { return; }

    match argv[0] {
        "help"      => commands::help::run(),
        "echo"      => commands::echo::run(&argv[1..argc]),
        "history"   => commands::history::run(),
        "clear"     => commands::clear::run(),
        "ls"        => commands::ls::run(),
        "rm"        => commands::rm::run(&argv[1..argc]),
        "write"     => commands::write::run(&argv[1..argc]),
        "info"      => commands::info::run(argv.get(1).copied().unwrap_or("")),
        "run"       => commands::run::run(&argv[1..argc]),
        "ps"        => commands::ps::run(),
        "task-run"  => commands::tasks::task_run(&argv[1..argc]),
        "task-kill" => commands::tasks::task_kill(&argv[1..argc]),
        "tasks"     => commands::tasks::list(),
        _           => { crate::println!("unknown command: {}", argv[0]); }
    }
}

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
