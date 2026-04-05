//! Non-blocking keyboard/serial input and shell line editor.
//!
//! `poll_once` is called by the scheduler on every round so the shell
//! participates in round-robin alongside WASM tasks.

use crate::drivers::keyboard::{try_next_key, Key};

const MAX_LINE: usize = 256;

// ── Shell state ───────────────────────────────────────────────────────────────

pub struct ShellState {
    buf:            [u8; MAX_LINE],
    len:            usize,
    prompt_pending: bool,
}

impl ShellState {
    pub fn new() -> Self {
        Self { buf: [0u8; MAX_LINE], len: 0, prompt_pending: true }
    }
}

// ── Key decoding ──────────────────────────────────────────────────────────────

fn serial_byte_to_key(b: u8) -> Key {
    match b {
        0x08 | 0x7F       => Key::Backspace,
        0x0D | 0x0A       => Key::Enter,
        0x20..=0x7E       => Key::Char(b as char),
        _                 => Key::Unknown,
    }
}

// ── Non-blocking step ─────────────────────────────────────────────────────────

/// Check for one key event and handle it. Returns `true` if a key was
/// available (so the caller knows not to idle).
pub fn poll_once(state: &mut ShellState) -> bool {
    if state.prompt_pending {
        crate::print!("> ");
        state.prompt_pending = false;
    }

    let key = if let Some(b) = crate::drivers::serial::read_byte() {
        serial_byte_to_key(b)
    } else if let Some(k) = try_next_key() {
        k
    } else {
        return false;
    };

    match key {
        Key::Char(c) => {
            if state.len < MAX_LINE - 1 {
                state.buf[state.len] = c as u8;
                state.len += 1;
                crate::print!("{}", c);
            }
        }
        Key::Backspace => {
            if state.len > 0 {
                state.len -= 1;
                crate::print!("\x08");
            }
        }
        Key::Enter => {
            crate::println!();
            let line = core::str::from_utf8(&state.buf[..state.len])
                .unwrap_or("")
                .trim();
            crate::shell::run_command(line);
            state.len = 0;
            state.prompt_pending = true;
        }
        Key::Unknown => {}
    }
    true
}

/// Reads a line into `buf` (max buf.len()), returns Some(&str) on Enter, None if interrupted.
/// Supports Backspace (ASCII 0x08 or 0x7f)
pub fn read_line(buf: &mut [u8]) -> Option<&str> {
    let mut len = 0;

    loop {
        // Poll key
        let key = if let Some(b) = crate::drivers::serial::read_byte() {
            serial_byte_to_key(b)
        } else if let Some(k) = try_next_key() {
            k
        } else {
            continue; // nothing ready yet
        };

        match key {
            Key::Enter => {
                // Echo newline
                crate::print!("\r\n");
                // Return the current buffer as str
                return core::str::from_utf8(&buf[..len]).ok();
            }

            Key::Backspace => {
                // Backspace
                if len > 0 {
                    len -= 1;
                    // Move cursor back, erase char visually
                    crate::print!("\x08 \x08");
                }
            }

            Key::Char(c) => {
                if len < buf.len() {
                    buf[len] = c as u8;
                    len += 1;
                    crate::print!("{}", c); // echo
                } else {
                    // Optional: beep or ignore
                }
            }
            
            Key::Unknown => {}
        }
    }
}
