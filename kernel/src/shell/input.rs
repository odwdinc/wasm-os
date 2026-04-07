//! Non-blocking keyboard/serial input and shell line editor.
//!
//! [`poll_once`] is called by the scheduler on every round so the shell
//! participates in the round-robin loop alongside WASM tasks without blocking.

use crate::drivers::keyboard::{try_next_key, Key};
use crate::shell::command_line_editor::CommandLineEditor;

const MAX_LINE: usize = 256;

// ── Shell state ───────────────────────────────────────────────────────────────

/// Persistent state for the non-blocking shell line editor.
pub struct ShellState {
    editor: CommandLineEditor,
}

impl ShellState {
    pub fn new() -> Self {
        Self { editor: CommandLineEditor::new() }
    }

    pub fn request_render(&mut self) {
        self.editor.request_render();
    }
}


// ── Non-blocking step ─────────────────────────────────────────────────────────

/// Check for one key event and handle it. Returns `true` if a key was
/// available (so the caller knows not to idle).
pub fn poll_once(state: &mut ShellState) -> bool {
    if let Some(command) = state.editor.get_input() {
        crate::print!("\r\x1B[0K\n");  // wipe prompt line, move to next line for output
        crate::shell::run_command(&command);
        state.editor.request_render();  // always redraw prompt once command finishes
    }
    state.editor.render(); // no-op unless dirty
    true
}

/// Blocking line-read for use by WASM host functions (not the scheduler loop).
///
/// Spins polling keyboard and serial until Enter is pressed.  Characters are
/// echoed and backspace is supported.  Returns `Some(&str)` on Enter, or
/// `None` if the buffer content is not valid UTF-8 (should not happen in
/// normal use with ASCII input).
pub fn read_line(buf: &mut [u8]) -> Option<&str> {
    let mut len = 0;

    loop {
        // Poll key
        let key = if let Some(b) = crate::drivers::serial::read_byte() {
            crate::drivers::serial::serial_byte_to_key(b)
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
            Key::ArrowLeft => {},
            Key::ArrowRight => {},
            Key::ArrowUp => {},
            Key::ArrowDown => {},
            Key::Delete => {},
            Key::Home => {},
            Key::End => {},
            Key::Tab => {},
            Key::Unknown => {}
        }
    }
}
