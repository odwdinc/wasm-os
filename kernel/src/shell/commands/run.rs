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
