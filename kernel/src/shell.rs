// shell.rs  –  Shell v1 (no_std): static entry point, tokenizer, ring-buffer history

// ─── Constants ────────────────────────────────────────────────────────────────

const MAX_ARGS:    usize = 8;
const HISTORY_LEN: usize = 16;

// ─── Error type ───────────────────────────────────────────────────────────────

#[derive(Debug, PartialEq)]
pub enum ShellError {
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
    unsafe { (*core::ptr::addr_of_mut!(HISTORY)).push(line); }

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
        "clear"   => cmd_clear(),
        "ls"      => cmd_ls(),
        "info"    => cmd_info(argv.get(1).copied().unwrap_or("")),
        "run"     => cmd_run(&argv[1..argc]),
        "ps"      => cmd_ps(),
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
    crate::println!("  clear              clear the screen");
    crate::println!("  ls                 list registered .wasm files");
    crate::println!("  info [name]        show module info, or tick count if no name");
    crate::println!("  run <name>         execute a .wasm module");
    crate::println!("  ps                 list running wasm instances");
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
    let hist = unsafe { &*core::ptr::addr_of!(HISTORY) };
    for i in 0..hist.len {
        crate::println!("{:>4}  {}", i + 1, hist.get(i));
    }
}

fn cmd_clear() {
    crate::vga::clear_screen();
}

fn cmd_ls() {
    let mut found = false;
    crate::fs::for_each_file(|name| {
        crate::println!("{}", name);
        found = true;
    });
    if !found {
        crate::println!("(no files registered)");
    }
}

fn cmd_info(name: &str) {
    if name.is_empty() {
        let t = crate::drivers::pit::ticks();
        crate::println!("ticks: {}  (~{} s)", t, t / 100);
        return;
    }
    let data = match crate::fs::find_file(name) {
        Some(d) => d,
        None    => { crate::println!("not found: {}", name); return; }
    };
    let module = match crate::wasm::loader::load(data) {
        Ok(m)  => m,
        Err(e) => { crate::println!("load error: {}", e.as_str()); return; }
    };
    let func_count = module.function_section
        .and_then(|s| crate::wasm::loader::read_u32_leb128(s))
        .map(|(n, _)| n)
        .unwrap_or(0);
    let import_count = crate::wasm::engine::count_func_imports(module.import_section);
    let export_count = module.export_section
        .and_then(|s| crate::wasm::loader::read_u32_leb128(s))
        .map(|(n, _)| n)
        .unwrap_or(0);
    crate::println!("file:    {}", name);
    crate::println!("funcs:   {} defined, {} imported", func_count, import_count);
    crate::println!("exports: {}", export_count);
}

fn cmd_run(argv: &[&str]) {
    let name = match argv.first() {
        Some(n) if !n.is_empty() => *n,
        _ => { crate::println!("usage: run <name> [args...]"); return; }
    };
    let data = match crate::fs::find_file(name) {
        Some(d) => d,
        None    => { crate::println!("not found: {}", name); return; }
    };
    // Parse optional integer arguments.
    let mut wasm_args = [0i32; 8];
    let mut arg_count = 0usize;
    for s in &argv[1..] {
        if arg_count >= 8 { break; }
        match parse_i32(s) {
            Some(n) => { wasm_args[arg_count] = n; arg_count += 1; }
            None    => { crate::println!("invalid arg: {}", s); return; }
        }
    }
    let handle = match crate::wasm::engine::spawn(name, data) {
        Ok(h)  => h,
        Err(e) => { crate::println!("error: {}", e.as_str()); return; }
    };
    let result = crate::wasm::engine::call_handle(handle, "main", &wasm_args[..arg_count]);
    crate::wasm::engine::destroy(handle);
    if let Err(e) = result {
        crate::println!("error: {}", e.as_str());
    }
}

fn cmd_ps() {
    let mut any = false;
    crate::wasm::engine::for_each_instance(|handle, name, mem_pages| {
        crate::println!("[{}] {}  ({} page(s), {} KiB)", handle, name, mem_pages, mem_pages * 64);
        any = true;
    });
    if !any {
        crate::println!("(no instances)");
    }
}

/// Parse a decimal integer string into i32. Returns None on invalid input.
fn parse_i32(s: &str) -> Option<i32> {
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