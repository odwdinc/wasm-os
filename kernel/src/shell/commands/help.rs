pub fn run() {
    crate::println!("Commands:");
    crate::println!("  help               show this message");
    crate::println!("  echo <args>        print arguments");
    crate::println!("  history            show command history");
    crate::println!("  clear              clear the screen");
    crate::println!("  ls                 list files and directories");
    crate::println!("  cat <file>         print file contents");
    crate::println!("  cd <dir>           change directory");
    crate::println!("  mkdir <dir>        create a directory");
    crate::println!("  df                 show filesystem space usage");
    crate::println!("  rm <name>          remove a file");
    crate::println!("  write <name> <hex> write raw bytes (hex-encoded) as a file");
    crate::println!("  edit <name>        line-append editor (:w = save, :q = quit)");
    crate::println!("  save               flush file table to FAT volume");
    crate::println!("  info [name]        show module info, or tick count if no name");
    crate::println!("  run <name>         execute a .wasm module");
    crate::println!("  ps                 list running wasm instances");
    crate::println!("  task-run <name>    spawn a module as a task");
    crate::println!("  task-kill <id>     kill a task by ID");
    crate::println!("  tasks              list all tasks");
}
