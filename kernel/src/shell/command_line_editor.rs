use core::fmt::{self};
use alloc::string::String;
use alloc::vec::Vec;
use crate::drivers::keyboard::{try_next_key, Key};
use crate::alloc::string::ToString;

const PROMPT: &str = "> ";

pub struct CommandLineEditor {
    pub buffer: String,
    pub cursor_pos: usize,
    pub history: Vec<String>,
    pub history_index: usize,
    needs_render: bool,
}

impl CommandLineEditor {
    pub fn new() -> Self {
        Self {
            buffer: String::new(),
            cursor_pos: 0,
            history: Vec::new(),
            history_index: 0,
            needs_render: true, // draw prompt on first tick
        }
    }

    fn handle_key(&mut self, key: Key) {
        match key {
            Key::Char(c) => self.insert_char(c),
            Key::ArrowLeft => self.move_left(),
            Key::ArrowRight => self.move_right(),
            Key::Backspace => self.delete_left(),
            Key::Delete => self.delete_right(),
            Key::Home => self.move_home(),
            Key::End => self.move_end(),
            Key::ArrowUp => self.navigate_history_up(),
            Key::ArrowDown => self.navigate_history_down(),
            Key::Tab => self.auto_complete(),
            Key::Enter => {},
            Key::Unknown => {}
        }
    }

    fn insert_char(&mut self, c: char) {
        if self.cursor_pos < self.buffer.len() {
            self.buffer.insert(self.cursor_pos, c);
        } else {
            self.buffer.push(c);
        }
        self.cursor_pos += 1;
        self.needs_render = true;
    }

    fn move_left(&mut self) {
        if self.cursor_pos > 0 {
            self.cursor_pos -= 1;
            self.needs_render = true;
        }
    }

    fn move_right(&mut self) {
        if self.cursor_pos < self.buffer.len() {
            self.cursor_pos += 1;
            self.needs_render = true;
        }
    }

    fn delete_left(&mut self) {
        if self.cursor_pos > 0 {
            self.buffer.remove(self.cursor_pos - 1);
            self.cursor_pos -= 1;
            self.needs_render = true;
        }
    }

    fn delete_right(&mut self) {
        if self.cursor_pos < self.buffer.len() {
            self.buffer.remove(self.cursor_pos);
            self.needs_render = true;
        }
    }

    fn move_home(&mut self) {
        if self.cursor_pos != 0 {
            self.cursor_pos = 0;
            self.needs_render = true;
        }
    }

    fn move_end(&mut self) {
        if self.cursor_pos != self.buffer.len() {
            self.cursor_pos = self.buffer.len();
            self.needs_render = true;
        }
    }

    fn navigate_history_up(&mut self) {
        if !self.history.is_empty() && self.history_index > 0 {
            self.history_index -= 1;
            self.buffer = self.history[self.history_index].clone();
            self.cursor_pos = self.buffer.len();
            self.needs_render = true;
        }
    }

    fn navigate_history_down(&mut self) {
        if self.history_index < self.history.len() {
            self.history_index += 1;
            if self.history_index < self.history.len() {
                self.buffer = self.history[self.history_index].clone();
            } else {
                self.buffer.clear();
            }
            self.cursor_pos = self.buffer.len();
            self.needs_render = true;
        }
    }

    /// Check for a key and process it. Returns the completed command on Enter,
    /// or `None` if no key was available or the line isn't finished yet.
    pub fn get_input(&mut self) -> Option<String> {
        let key = if let Some(b) = crate::drivers::serial::read_byte() {
            crate::drivers::serial::serial_byte_to_key(b)
        } else if let Some(k) = try_next_key() {
            k
        } else {
            return None;
        };

        match key {
            Key::Enter => {
                if !self.buffer.is_empty() {
                    self.history.push(self.buffer.clone());
                    self.history_index = self.history.len();
                    let input = self.buffer.clone();
                    self.buffer.clear();
                    self.cursor_pos = 0;
                    self.needs_render = true; // redraw prompt after command finishes
                    return Some(input);
                }
            }
            _ => self.handle_key(key),
        }
        None
    }

   fn auto_complete(&mut self) {
    let names = crate::shell::command_names();
    let matches: Vec<&str> = names.iter()
        .map(|s| s.as_str())
        .filter(|command| command.starts_with(self.buffer.as_str()))
        .collect();

        if matches.is_empty() {
            return;
        } else if matches.len() == 1 {
            // Complete the command
            let completion = matches[0];
            self.buffer = completion.to_string();
            self.cursor_pos = self.buffer.len();
            self.needs_render = true;
        } else {
            // Show all possible completions
            crate::println!();
            for match_ in &matches {
                crate::print!("{} ", match_);
            }
            crate::println!();
            self.needs_render = true;
        }
        // Re-render the current input line
        self.render();
    }

    /// Mark the prompt as needing a redraw on the next `render()` call.
    pub fn request_render(&mut self) {
        self.needs_render = true;
    }

    /// Redraw the prompt line. No-op if nothing has changed since the last draw.
    pub fn render(&mut self) {
        if !self.needs_render {
            return;
        }
        self.needs_render = false;
        // \r        — go to column 0
        // \x1B[0K   — erase to end of line
        // PROMPT    — display-only prefix, never included in the returned command
        // buffer    — current user input
        // \x1B[nG   — position cursor (1-based column)
        crate::print!("\r\x1B[0K{}{}\x1B[{}G",
            PROMPT,
            self.buffer,
            PROMPT.len() + self.cursor_pos + 1);
    }
}

impl fmt::Write for CommandLineEditor {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for c in s.chars() {
            self.insert_char(c);
        }
        Ok(())
    }
}
