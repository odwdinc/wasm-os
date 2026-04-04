pub fn task_run(argv: &[&str]) {
    let name = match argv.first() {
        Some(n) if !n.is_empty() => *n,
        _ => { crate::println!("usage: task-run <name>"); return; }
    };
    let data = match crate::fs::find_file(name) {
        Some(d) => d,
        None    => { crate::println!("not found: {}", name); return; }
    };
    match crate::wasm::task::task_spawn(name, data) {
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
