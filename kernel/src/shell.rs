//! Sprint 1.5 — Shell v0: command parsing and built-in commands.

/// Parse and execute one line of input. `line` must be trimmed.
pub fn run_command(line: &str) {
    if line.is_empty() {
        return;
    }

    // Split into command and the rest of the arguments.
    let (cmd, rest) = match line.find(' ') {
        Some(i) => (&line[..i], line[i + 1..].trim()),
        None => (line, ""),
    };

    match cmd {
        "help" => cmd_help(),
        "echo" => cmd_echo(rest),
        _ => { crate::println!("unknown command: {}", cmd); }
    }
}

fn cmd_help() {
    crate::println!("Commands:");
    crate::println!("  help        show this message");
    crate::println!("  echo <msg>  print msg");
}

fn cmd_echo(args: &str) {
    crate::println!("{}", args);
}
