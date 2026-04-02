// shell.rs  –  Shell v1 (no_std): static entry point, tokenizer, ring-buffer history

// ─── Constants ────────────────────────────────────────────────────────────────

const MAX_ARGS:    usize = 8;
const HISTORY_LEN: usize = 16;

// ─── Error type ───────────────────────────────────────────────────────────────

#[derive(Debug, PartialEq)]
pub enum ShellError {
    UnknownCommand,
    TooManyArgs,
}

// ─── Shell state (module-level statics) ───────────────────────────────────────

// History stores fixed-length copies so we don't need the original &str to stay alive.
const MAX_LINE: usize = 128;

struct HistoryBuf {
    lines: [[u8; MAX_LINE]; HISTORY_LEN],
    lens:  [usize; HISTORY_LEN],
    head:  usize,
    len:   usize,
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
        // i=0 is oldest
        let start = if self.len < HISTORY_LEN { 0 } else { self.head };
        let idx = (start + i) % HISTORY_LEN;
        let len = self.lens[idx];
        core::str::from_utf8(&self.lines[idx][..len]).unwrap_or("")
    }
}

// Safe on a single-core bare-metal target (no preemption).
static mut HISTORY: HistoryBuf = HistoryBuf::new();

// ─── Entry point (matches your original signature) ────────────────────────────

/// Called by the keyboard driver each time the user presses Enter.
/// `line` must be trimmed.
pub fn run_command(line: &str) {
    if line.is_empty() || line.starts_with('#') {
        return;
    }

    // Safety: single-core, no interrupts mutating HISTORY concurrently.
    unsafe { HISTORY.push(line); }

    let mut argv = [""; MAX_ARGS];
    let argc = match tokenize(line, &mut argv) {
        Ok(n)  => n,
        Err(_) => { crate::println!("error: too many arguments"); return; }
    };
    if argc == 0 { return; }

    match argv[0] {
        "help"    => cmd_help(),
        "echo"    => cmd_echo(&argv[1..argc]),
        "history" => cmd_history(),
        "wasm"    => cmd_wasm(),
        _         => { crate::println!("unknown command: {}", argv[0]); }
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
            if i < bytes.len() { i += 1; } // skip closing quote
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

// ─── Built-ins ────────────────────────────────────────────────────────────────

fn cmd_help() {
    crate::println!("Commands:");
    crate::println!("  help               show this message");
    crate::println!("  echo <args>        print arguments");
    crate::println!("  history            show command history");
    crate::println!("  wasm               run built-in WASM test");
}

fn cmd_echo(args: &[&str]) {
    let mut first = true;
    for word in args {
        if !first { crate::print!(" "); }
        crate::print!("{}", word);
        first = false;
    }
    crate::println!();
}

fn cmd_history() {
    let hist = unsafe { &HISTORY };
    for i in 0..hist.len {
        crate::println!("{:>4}  {}", i + 1, hist.get(i));
    }
}

fn cmd_wasm() {
    // Minimal WASM module (no imports):
    //   (module (func (result i32) (i32.const 42)))
    //
    // Byte layout:
    //   magic + version      00 61 73 6D  01 00 00 00
    //   type section   (id=1, size=5):  01 60 00 01 7F
    //   func section   (id=3, size=2):  01 00
    //   code section   (id=10,size=6):  01 04 00 41 2A 0B
    static TEST_WASM: &[u8] = &[
        // header
        0x00, 0x61, 0x73, 0x6D,
        0x01, 0x00, 0x00, 0x00,
        // type section: () -> i32
        0x01, 0x05, 0x01, 0x60, 0x00, 0x01, 0x7F,
        // function section: func 0 uses type 0
        0x03, 0x02, 0x01, 0x00,
        // code section: i32.const 42; end
        0x0A, 0x06, 0x01, 0x04, 0x00, 0x41, 0x2A, 0x0B,
    ];

    match crate::wasm::engine::run(TEST_WASM, 0) {
        Ok(Some(v)) => { crate::println!("WASM result: {}", v); }
        Ok(None)    => { crate::println!("WASM: ok (no return value)"); }
        Err(e)      => { crate::println!("WASM error: {}", e.as_str()); }
    }
}