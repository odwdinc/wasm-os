pub fn run() {
    crate::println!("Commands:");
    crate::println!("  help               show this message");
    crate::println!("  echo <args>        print arguments");
    crate::println!("  history            show command history");
    crate::println!("  clear              clear the screen");
    crate::println!("  ls                 list registered .wasm files");
    crate::println!("  info [name]        show module info, or tick count if no name");
    crate::println!("  run <name>         execute a .wasm module");
    crate::println!("  ps                 list running wasm instances");
    crate::println!("  task-run <name>    spawn a module as a task");
    crate::println!("  task-kill <id>     kill a task by ID");
    crate::println!("  tasks              list all tasks");
}
