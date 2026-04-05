pub fn run(argv: &[&str]) {
    let name = match argv.first() {
        Some(n) if !n.is_empty() => *n,
        _ => { crate::println!("usage: run <name> [args...]"); return; }
    };
    let data = match crate::fs::find_file(name) {
        Some(d) => d,
        None    => { crate::println!("not found: {}", name); return; }
    };
    let mut wasm_args = [0i32; 8];
    let mut arg_count = 0usize;
    for s in &argv[1..] {
        if arg_count >= 8 { break; }
        match crate::shell::parse_i32(s) {
            Some(n) => { wasm_args[arg_count] = n; arg_count += 1; }
            None    => { crate::println!("invalid arg: {}", s); return; }
        }
    }

    // Store the raw args string so `args_get` host function can expose it.
    // Join with spaces into a fixed-size buffer without heap allocation.
    let mut raw_buf = [0u8; 128];
    let mut raw_len = 0usize;
    for (i, s) in argv[1..].iter().enumerate() {
        if i > 0 && raw_len < raw_buf.len() {
            raw_buf[raw_len] = b' ';
            raw_len += 1;
        }
        let bytes = s.as_bytes();
        let copy = bytes.len().min(raw_buf.len() - raw_len);
        raw_buf[raw_len..raw_len + copy].copy_from_slice(&bytes[..copy]);
        raw_len += copy;
    }
    if let Ok(s) = core::str::from_utf8(&raw_buf[..raw_len]) {
        crate::wasm::engine::set_args(s);
    }

    let handle = match crate::wasm::engine::spawn(name, data) {
        Ok(h)  => h,
        Err(e) => { crate::println!("error: {}", e.as_str()); return; }
    };
    // Run to completion, treating yields as no-ops (synchronous path).
    let mut result = crate::wasm::engine::start_task(handle, "main", &wasm_args[..arg_count]);
    while let Ok(crate::wasm::engine::TaskResult::Yielded) = result {
        result = crate::wasm::engine::resume_task(handle);
    }
    crate::wasm::engine::destroy(handle);
    if let Err(e) = result {
        crate::println!("error: {}", e.as_str());
    }
}
