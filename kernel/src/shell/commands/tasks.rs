pub fn task_run(argv: &[&str]) {
    let name = match argv.first() {
        Some(n) if !n.is_empty() => *n,
        _ => { crate::println!("usage: task-run <name> [args...]"); return; }
    };
    task_run_with(name, &argv[1..]);
}

/// Spawn `name` as a background task, passing `args` as the space-joined
/// argument string accessible via the `args_get` host function.
pub fn task_run_with(name: &str, args: &[&str]) {
    // Build the raw args string (space-joined) into a stack buffer for set_args.
    let mut raw_buf = [0u8; 128];
    let mut raw_len = 0usize;
    for (i, s) in args.iter().enumerate() {
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

    // Parse string args into i32 values for the WASM main() call.
    let mut wasm_args = [0i32; 8];
    let mut wasm_argc = 0usize;
    for s in args {
        if wasm_argc >= 8 { break; }
        match crate::shell::parse_i32(s) {
            Some(n) => { wasm_args[wasm_argc] = n; wasm_argc += 1; }
            None    => { crate::println!("invalid arg: {}", s); return; }
        }
    }

    let data = match crate::fs::find_file(name) {
        Some(d) => d,
        None    => { crate::println!("not found: {}", name); return; }
    };
    match crate::wasm::task::task_spawn(name, data, &wasm_args[..wasm_argc]) {
        Ok(id)  => crate::println!("task {} spawned: {}", id, name),
        Err(e)  => crate::println!("error: {}", e.as_str()),
    }
}

pub fn task_kill(argv: &[&str]) {
    let id = match argv.first().and_then(|s| crate::shell::parse_usize(s)) {
        Some(n) => n,
        None    => { crate::println!("usage: task-kill <id>"); return; }
    };
    crate::wasm::task::task_kill(id);
    crate::println!("task {} killed", id);
}

pub fn list() {
    let mut any = false;
    crate::wasm::task::for_each_task(|id, name, state| {
        let s = match state {
            crate::wasm::task::TaskState::Ready        => "ready",
            crate::wasm::task::TaskState::Running      => "running",
            crate::wasm::task::TaskState::Suspended    => "suspended",
            crate::wasm::task::TaskState::Sleeping(_)  => "sleeping",
            crate::wasm::task::TaskState::Done         => "done",
        };
        crate::println!("[{}] {}  ({})", id, name, s);
        any = true;
    });
    if !any { crate::println!("(no tasks)"); }
}
