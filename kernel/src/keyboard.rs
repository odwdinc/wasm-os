//! High-level keyboard input: reads keys, echoes them, and builds lines.

use crate::drivers::keyboard::{next_key, Key};

const MAX_LINE: usize = 256;

/// Print the shell prompt.
fn prompt() {
    crate::print!("> ");
}

/// Main keyboard loop — never returns.
pub fn run_loop() -> ! {
    prompt();
    let mut buf = [0u8; MAX_LINE];
    let mut len: usize = 0;

    loop {
        match next_key() {
            Key::Char(c) => {
                if len < MAX_LINE - 1 {
                    buf[len] = c as u8;
                    len += 1;
                    crate::print!("{}", c);
                }
            }
            Key::Backspace => {
                if len > 0 {
                    len -= 1;
                    crate::print!("\x08");
                }
            }
            Key::Enter => {
                crate::println!();
                let line = core::str::from_utf8(&buf[..len]).unwrap_or("").trim();
                crate::shell::run_command(line);
                len = 0;
                prompt();
            }
            Key::Unknown => {}
        }
    }
}
