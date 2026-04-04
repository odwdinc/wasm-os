//! High-level input: reads from PS/2 keyboard and serial console,
//! echoes characters, builds lines, and dispatches shell commands.

use crate::drivers::keyboard::{try_next_key, Key};

const MAX_LINE: usize = 256;

/// Print the shell prompt.
fn prompt() {
    crate::print!("> ");
}

/// Decode a raw serial byte into a Key.
/// Handles the common byte values sent by terminal emulators.
fn serial_byte_to_key(b: u8) -> Key {
    match b {
        0x08 | 0x7F       => Key::Backspace,       // BS or DEL
        0x0D | 0x0A       => Key::Enter,            // CR or LF
        0x20..=0x7E       => Key::Char(b as char),  // printable ASCII
        _                 => Key::Unknown,
    }
}

/// Main input loop — never returns.
/// Polls both the serial UART and PS/2 keyboard; whichever has data first wins.
pub fn run_loop() -> ! {
    prompt();
    let mut buf = [0u8; MAX_LINE];
    let mut len: usize = 0;

    loop {
        // Prefer serial (enables headless/automated testing via -serial mon:stdio).
        let key = if let Some(b) = crate::drivers::serial::read_byte() {
            serial_byte_to_key(b)
        } else if let Some(k) = try_next_key() {
            k
        } else {
            crate::scheduler::tick();
            continue;
        };

        match key {
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
