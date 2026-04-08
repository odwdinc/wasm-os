//! Framebuffer text renderer and direct pixel access.
//!
//! Writes characters to the linear framebuffer provided by the bootloader
//! using an 8×8 pixel bitmap font for printable ASCII (32–126).
//!
//! The global [`WRITER`] is a [`spin::Mutex`]-protected [`VgaBuffer`] that
//! is initialised once by [`init`].  All output is also mirrored to the
//! serial port via the `print!` / `println!` macros in `main.rs`.
//!
//! [`set_pixel`] bypasses the text cursor and writes a single pixel directly
//! to the framebuffer; it handles BGR/RGB conversion automatically.

use bootloader_api::info::{FrameBufferInfo, PixelFormat};
use spin::Mutex;

use core::fmt::{Write};
use alloc::string::String;

// ANSI SGR — reset
pub const RESET: &str = "\x1B[0m";

// ANSI SGR — normal foreground colors (30–37)
pub const FG_BLACK: &str = "\x1B[30m";
pub const FG_RED: &str = "\x1B[31m";
pub const FG_GREEN: &str = "\x1B[32m";
pub const FG_YELLOW: &str = "\x1B[33m";
pub const FG_BLUE: &str = "\x1B[34m";
pub const FG_MAGENTA: &str = "\x1B[35m";
pub const FG_CYAN: &str = "\x1B[36m";
pub const FG_WHITE: &str = "\x1B[37m";

// ANSI SGR — bright foreground colors (90–97)
pub const FG_BRIGHT_BLACK: &str = "\x1B[90m";
pub const FG_BRIGHT_RED: &str = "\x1B[91m";
pub const FG_BRIGHT_GREEN: &str = "\x1B[92m";
pub const FG_BRIGHT_YELLOW: &str = "\x1B[93m";
pub const FG_BRIGHT_BLUE: &str = "\x1B[94m";
pub const FG_BRIGHT_MAGENTA: &str = "\x1B[95m";
pub const FG_BRIGHT_CYAN: &str = "\x1B[96m";
pub const FG_BRIGHT_WHITE: &str = "\x1B[97m";

// ANSI SGR — normal background colors (40–47)
pub const BG_BLACK: &str = "\x1B[40m";
pub const BG_RED: &str = "\x1B[41m";
pub const BG_GREEN: &str = "\x1B[42m";
pub const BG_YELLOW: &str = "\x1B[43m";
pub const BG_BLUE: &str = "\x1B[44m";
pub const BG_MAGENTA: &str = "\x1B[45m";
pub const BG_CYAN: &str = "\x1B[46m";
pub const BG_WHITE: &str = "\x1B[47m";

// ANSI SGR — bright background colors (100–107)
pub const BG_BRIGHT_BLACK: &str = "\x1B[100m";
pub const BG_BRIGHT_RED: &str = "\x1B[101m";
pub const BG_BRIGHT_GREEN: &str = "\x1B[102m";
pub const BG_BRIGHT_YELLOW: &str = "\x1B[103m";
pub const BG_BRIGHT_BLUE: &str = "\x1B[104m";
pub const BG_BRIGHT_MAGENTA: &str = "\x1B[105m";
pub const BG_BRIGHT_CYAN: &str = "\x1B[106m";
pub const BG_BRIGHT_WHITE: &str = "\x1B[107m";

// ANSI cursor movement
pub const CURSOR_UP: &str = "\x1B[A";
pub const CURSOR_DOWN: &str = "\x1B[B";
pub const CURSOR_RIGHT: &str = "\x1B[C";
pub const CURSOR_LEFT: &str = "\x1B[D";
pub const CURSOR_NEXT_LINE: &str = "\x1B[E";  // down + col 0
pub const CURSOR_PREV_LINE: &str = "\x1B[F";  // up + col 0
pub const CURSOR_POSITION: &str = "\x1B[H";   // home (1,1)
pub const CURSOR_SAVE: &str = "\x1B[s";
pub const CURSOR_RESTORE: &str = "\x1B[u";

// ANSI erase
pub const CLEAR_SCREEN: &str = "\x1B[2J";
pub const CLEAR_SCREEN_END: &str = "\x1B[0J";    // cursor to end of screen
pub const CLEAR_SCREEN_START: &str = "\x1B[1J";  // start of screen to cursor
pub const CLEAR_LINE: &str = "\x1B[2K";
pub const CLEAR_LINE_END: &str = "\x1B[0K";      // cursor to end of line
pub const CLEAR_LINE_START: &str = "\x1B[1K";    // start of line to cursor

// Define colors using RGB values
struct Color(u8, u8, u8);

impl Color {
    // Normal (dim) palette
    const BLACK: Self = Self(0, 0, 0);
    const RED: Self = Self(170, 0, 0);
    const GREEN: Self = Self(0, 170, 0);
    const YELLOW: Self = Self(170, 170, 0);
    const BLUE: Self = Self(0, 0, 170);
    const MAGENTA: Self = Self(170, 0, 170);
    const CYAN: Self = Self(0, 170, 170);
    const WHITE: Self = Self(170, 170, 170);
    // Bright palette
    const BRIGHT_BLACK: Self = Self(85, 85, 85);
    const BRIGHT_RED: Self = Self(255, 85, 85);
    const BRIGHT_GREEN: Self = Self(85, 255, 85);
    const BRIGHT_YELLOW: Self = Self(255, 255, 85);
    const BRIGHT_BLUE: Self = Self(85, 85, 255);
    const BRIGHT_MAGENTA: Self = Self(255, 85, 255);
    const BRIGHT_CYAN: Self = Self(85, 255, 255);
    const BRIGHT_WHITE: Self = Self(255, 255, 255);
}


const CHAR_W: usize = 8;
const CHAR_H: usize = 8;

// 8×8 bitmap font for printable ASCII 32–126.
// Each entry is 8 rows; within each row byte bit 7 (MSB) is the leftmost pixel.
#[rustfmt::skip]
static FONT: [[u8; 8]; 95] = [
    [0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00], // 32  ' '
    [0x18,0x18,0x18,0x18,0x18,0x00,0x18,0x00], // 33  '!'
    [0x66,0x66,0x66,0x00,0x00,0x00,0x00,0x00], // 34  '"'
    [0x66,0x66,0xFF,0x66,0xFF,0x66,0x66,0x00], // 35  '#'
    [0x18,0x7E,0xC0,0x7C,0x06,0x7E,0x18,0x00], // 36  '$'
    [0x62,0x66,0x0C,0x18,0x30,0x66,0x46,0x00], // 37  '%'
    [0x3C,0x66,0x3C,0x38,0x67,0x66,0x3F,0x00], // 38  '&'
    [0x06,0x0C,0x18,0x00,0x00,0x00,0x00,0x00], // 39  '\''
    [0x0C,0x18,0x30,0x30,0x30,0x18,0x0C,0x00], // 40  '('
    [0x30,0x18,0x0C,0x0C,0x0C,0x18,0x30,0x00], // 41  ')'
    [0x00,0x66,0x3C,0xFF,0x3C,0x66,0x00,0x00], // 42  '*'
    [0x00,0x18,0x18,0x7E,0x18,0x18,0x00,0x00], // 43  '+'
    [0x00,0x00,0x00,0x00,0x00,0x18,0x18,0x30], // 44  ','
    [0x00,0x00,0x00,0x7E,0x00,0x00,0x00,0x00], // 45  '-'
    [0x00,0x00,0x00,0x00,0x00,0x18,0x18,0x00], // 46  '.'
    [0x06,0x0C,0x18,0x30,0x60,0xC0,0x80,0x00], // 47  '/'
    [0x3C,0x66,0x6E,0x76,0x66,0x66,0x3C,0x00], // 48  '0'
    [0x18,0x38,0x18,0x18,0x18,0x18,0x7E,0x00], // 49  '1'
    [0x3C,0x66,0x06,0x0C,0x30,0x60,0x7E,0x00], // 50  '2'
    [0x3C,0x66,0x06,0x1C,0x06,0x66,0x3C,0x00], // 51  '3'
    [0x0C,0x1C,0x3C,0x6C,0x7E,0x0C,0x0C,0x00], // 52  '4'
    [0x7E,0x60,0x7C,0x06,0x06,0x66,0x3C,0x00], // 53  '5'
    [0x3C,0x66,0x60,0x7C,0x66,0x66,0x3C,0x00], // 54  '6'
    [0x7E,0x66,0x0C,0x18,0x18,0x18,0x18,0x00], // 55  '7'
    [0x3C,0x66,0x66,0x3C,0x66,0x66,0x3C,0x00], // 56  '8'
    [0x3C,0x66,0x66,0x3E,0x06,0x66,0x3C,0x00], // 57  '9'
    [0x00,0x00,0x18,0x00,0x00,0x18,0x00,0x00], // 58  ':'
    [0x00,0x00,0x18,0x00,0x00,0x18,0x18,0x30], // 59  ';'
    [0x0C,0x18,0x30,0x60,0x30,0x18,0x0C,0x00], // 60  '<'
    [0x00,0x00,0x7E,0x00,0x7E,0x00,0x00,0x00], // 61  '='
    [0x60,0x30,0x18,0x0C,0x18,0x30,0x60,0x00], // 62  '>'
    [0x3C,0x66,0x06,0x0C,0x18,0x00,0x18,0x00], // 63  '?'
    [0x3E,0x63,0x6F,0x6B,0x6F,0x60,0x3E,0x00], // 64  '@'
    [0x18,0x3C,0x66,0x66,0x7E,0x66,0x66,0x00], // 65  'A'
    [0x7C,0x66,0x66,0x7C,0x66,0x66,0x7C,0x00], // 66  'B'
    [0x3C,0x66,0x60,0x60,0x60,0x66,0x3C,0x00], // 67  'C'
    [0x78,0x6C,0x66,0x66,0x66,0x6C,0x78,0x00], // 68  'D'
    [0x7E,0x60,0x60,0x78,0x60,0x60,0x7E,0x00], // 69  'E'
    [0x7E,0x60,0x60,0x78,0x60,0x60,0x60,0x00], // 70  'F'
    [0x3C,0x66,0x60,0x6E,0x66,0x66,0x3C,0x00], // 71  'G'
    [0x66,0x66,0x66,0x7E,0x66,0x66,0x66,0x00], // 72  'H'
    [0x3C,0x18,0x18,0x18,0x18,0x18,0x3C,0x00], // 73  'I'
    [0x1E,0x0C,0x0C,0x0C,0x0C,0x6C,0x38,0x00], // 74  'J'
    [0x66,0x6C,0x78,0x70,0x78,0x6C,0x66,0x00], // 75  'K'
    [0x60,0x60,0x60,0x60,0x60,0x60,0x7E,0x00], // 76  'L'
    [0x63,0x77,0x7F,0x6B,0x63,0x63,0x63,0x00], // 77  'M'
    [0x66,0x76,0x7E,0x7E,0x6E,0x66,0x66,0x00], // 78  'N'
    [0x3C,0x66,0x66,0x66,0x66,0x66,0x3C,0x00], // 79  'O'
    [0x7C,0x66,0x66,0x7C,0x60,0x60,0x60,0x00], // 80  'P'
    [0x3C,0x66,0x66,0x66,0x66,0x6C,0x36,0x00], // 81  'Q'
    [0x7C,0x66,0x66,0x7C,0x6C,0x66,0x66,0x00], // 82  'R'
    [0x3C,0x66,0x60,0x3C,0x06,0x66,0x3C,0x00], // 83  'S'
    [0x7E,0x18,0x18,0x18,0x18,0x18,0x18,0x00], // 84  'T'
    [0x66,0x66,0x66,0x66,0x66,0x66,0x3C,0x00], // 85  'U'
    [0x66,0x66,0x66,0x66,0x66,0x3C,0x18,0x00], // 86  'V'
    [0x63,0x63,0x63,0x6B,0x7F,0x77,0x63,0x00], // 87  'W'
    [0x66,0x66,0x3C,0x18,0x3C,0x66,0x66,0x00], // 88  'X'
    [0x66,0x66,0x66,0x3C,0x18,0x18,0x18,0x00], // 89  'Y'
    [0x7E,0x06,0x0C,0x18,0x30,0x60,0x7E,0x00], // 90  'Z'
    [0x3C,0x30,0x30,0x30,0x30,0x30,0x3C,0x00], // 91  '['
    [0xC0,0x60,0x30,0x18,0x0C,0x06,0x02,0x00], // 92  '\\'
    [0x3C,0x0C,0x0C,0x0C,0x0C,0x0C,0x3C,0x00], // 93  ']'
    [0x18,0x3C,0x66,0x00,0x00,0x00,0x00,0x00], // 94  '^'
    [0x00,0x00,0x00,0x00,0x00,0x00,0x00,0xFF], // 95  '_'
    [0x30,0x18,0x0C,0x00,0x00,0x00,0x00,0x00], // 96  '`'
    [0x00,0x00,0x3C,0x06,0x3E,0x66,0x3E,0x00], // 97  'a'
    [0x60,0x60,0x7C,0x66,0x66,0x66,0x7C,0x00], // 98  'b'
    [0x00,0x00,0x3C,0x60,0x60,0x60,0x3C,0x00], // 99  'c'
    [0x06,0x06,0x3E,0x66,0x66,0x66,0x3E,0x00], // 100 'd'
    [0x00,0x00,0x3C,0x66,0x7E,0x60,0x3C,0x00], // 101 'e'
    [0x1C,0x30,0x30,0x7C,0x30,0x30,0x30,0x00], // 102 'f'
    [0x00,0x00,0x3E,0x66,0x66,0x3E,0x06,0x3C], // 103 'g'
    [0x60,0x60,0x7C,0x66,0x66,0x66,0x66,0x00], // 104 'h'
    [0x18,0x00,0x38,0x18,0x18,0x18,0x3C,0x00], // 105 'i'
    [0x06,0x00,0x06,0x06,0x06,0x06,0x66,0x3C], // 106 'j'
    [0x60,0x60,0x66,0x6C,0x78,0x6C,0x66,0x00], // 107 'k'
    [0x38,0x18,0x18,0x18,0x18,0x18,0x3C,0x00], // 108 'l'
    [0x00,0x00,0x66,0x7F,0x7F,0x6B,0x63,0x00], // 109 'm'
    [0x00,0x00,0x7C,0x66,0x66,0x66,0x66,0x00], // 110 'n'
    [0x00,0x00,0x3C,0x66,0x66,0x66,0x3C,0x00], // 111 'o'
    [0x00,0x00,0x7C,0x66,0x66,0x7C,0x60,0x60], // 112 'p'
    [0x00,0x00,0x3E,0x66,0x66,0x3E,0x06,0x06], // 113 'q'
    [0x00,0x00,0x7C,0x66,0x60,0x60,0x60,0x00], // 114 'r'
    [0x00,0x00,0x3C,0x60,0x3C,0x06,0x3C,0x00], // 115 's'
    [0x30,0x30,0x7C,0x30,0x30,0x30,0x1C,0x00], // 116 't'
    [0x00,0x00,0x66,0x66,0x66,0x66,0x3E,0x00], // 117 'u'
    [0x00,0x00,0x66,0x66,0x66,0x3C,0x18,0x00], // 118 'v'
    [0x00,0x00,0x63,0x6B,0x7F,0x7F,0x36,0x00], // 119 'w'
    [0x00,0x00,0x66,0x3C,0x18,0x3C,0x66,0x00], // 120 'x'
    [0x00,0x00,0x66,0x66,0x66,0x3E,0x06,0x3C], // 121 'y'
    [0x00,0x00,0x7E,0x0C,0x18,0x30,0x7E,0x00], // 122 'z'
    [0x0C,0x18,0x18,0x70,0x18,0x18,0x0C,0x00], // 123 '{'
    [0x18,0x18,0x18,0x18,0x18,0x18,0x18,0x00], // 124 '|'
    [0x30,0x18,0x18,0x0E,0x18,0x18,0x30,0x00], // 125 '}'
    [0x31,0x6B,0x46,0x00,0x00,0x00,0x00,0x00], // 126 '~'
];

// ---------------------------------------------------------------------------
// Writer
// ---------------------------------------------------------------------------

/// Direct-write framebuffer text renderer with cursor tracking and scrolling.
pub struct VgaBuffer {
    buf: *mut u8,
    buf_len: usize,
    info: FrameBufferInfo,
    col: usize,       // current character column
    row: usize,       // current character row
    saved_col: usize, // last ESC[s position
    saved_row: usize,
    fg: Color,
    bg: Color,
}

// SAFETY: the framebuffer pointer is valid for the kernel's lifetime and
// is only accessed while holding the WRITER spinlock.
unsafe impl Send for VgaBuffer {}

impl VgaBuffer {
    /// Create a new `VgaBuffer` from the raw framebuffer slice and its metadata.
    pub fn new(buf: &mut [u8], info: FrameBufferInfo) -> Self {
        VgaBuffer {
            buf: buf.as_mut_ptr(),
            buf_len: buf.len(),
            info,
            col: 0,
            row: 0,
            saved_col: 0,
            saved_row: 0,
            fg: Color::BRIGHT_WHITE,
            bg: Color::BLACK,
        }
    }

    fn cols(&self) -> usize {
        self.info.width / CHAR_W
    }

    fn rows(&self) -> usize {
        self.info.height / CHAR_H
    }

    /// Scroll the framebuffer up by one character row and clear the last row.
    fn scroll_up(&mut self) {
        let bpp = self.info.bytes_per_pixel;
        let stride = self.info.stride;
        let row_bytes = CHAR_H * stride * bpp;
        let total_rows = self.rows();
        if total_rows == 0 {
            return;
        }
        // SAFETY: buf is a valid allocation of buf_len bytes.
        unsafe {
            // Shift every row except the first up by one character row.
            core::ptr::copy(
                self.buf.add(row_bytes),
                self.buf,
                (total_rows - 1) * row_bytes,
            );
            // Blank the last character row.
            core::ptr::write_bytes(
                self.buf.add((total_rows - 1) * row_bytes),
                0,
                row_bytes,
            );
        }
    }

    fn parse_ansi(&mut self, s: &str) {
        let mut chars = s.chars();
        while let Some(c) = chars.next() {
            if c != '\x1B' {
                self.write_char(c);
                continue;
            }
            // Only handle CSI sequences (ESC [).
            if chars.next() != Some('[') {
                continue;
            }
            // Read digits and semicolons until the command character.
            let mut param_str = String::new();
            let cmd = loop {
                match chars.next() {
                    Some(c) if c.is_ascii_digit() || c == ';' => param_str.push(c),
                    Some(c) => break c,
                    None => return,
                }
            };
            // Parse up to two numeric params (0 means "use default").
            let mut p = [0usize; 2];
            for (i, part) in param_str.splitn(3, ';').take(2).enumerate() {
                p[i] = part.parse().unwrap_or(0);
            }
            match cmd {
                'm' => {
                    // SGR — may carry multiple codes separated by ';'.
                    if param_str.is_empty() {
                        self.set_color(0);
                    } else {
                        for part in param_str.split(';') {
                            self.set_color(part.parse::<u8>().unwrap_or(0));
                        }
                    }
                }
                'J' => match p[0] {
                    0 => self.clear_to_end_of_screen(),
                    1 => self.clear_to_start_of_screen(),
                    2 => self.clear_screen(),
                    _ => {}
                },
                'K' => match p[0] {
                    0 => self.clear_to_end_of_line(),
                    1 => self.clear_to_start_of_line(),
                    2 => self.clear_line(),
                    _ => {}
                },
                'H' | 'f' => {
                    // Cursor position: 1-based row;col, default 1;1 → 0-based 0;0.
                    let row = p[0].saturating_sub(1);
                    let col = p[1].saturating_sub(1);
                    self.set_cursor_position(row, col);
                }
                // Cursor up — stay in column
                'A' => { self.row = self.row.saturating_sub(p[0].max(1)); }
                // Cursor previous line — move up and reset to col 0
                'F' => { self.row = self.row.saturating_sub(p[0].max(1)); self.col = 0; }
                // Cursor down — stay in column
                'B' => {
                    self.row = (self.row + p[0].max(1)).min(self.rows().saturating_sub(1));
                }
                // Cursor next line — move down and reset to col 0
                'E' => {
                    self.row = (self.row + p[0].max(1)).min(self.rows().saturating_sub(1));
                    self.col = 0;
                }
                // Cursor right
                'C' => {
                    self.col = (self.col + p[0].max(1)).min(self.cols().saturating_sub(1));
                }
                // Cursor left
                'D' => { self.col = self.col.saturating_sub(p[0].max(1)); }
                // Cursor to column (1-based)
                'G' => {
                    self.col = p[0].saturating_sub(1).min(self.cols().saturating_sub(1));
                }
                // Cursor to row (1-based)
                'd' => {
                    self.row = p[0].saturating_sub(1).min(self.rows().saturating_sub(1));
                }
                // Save / restore cursor position
                's' => { self.saved_row = self.row; self.saved_col = self.col; }
                'u' => { self.row = self.saved_row; self.col = self.saved_col; }
                // Insert / delete lines at cursor row
                'L' => self.insert_lines(p[0].max(1)),
                'M' => self.delete_lines(p[0].max(1)),
                _ => {}
            }
        }
    }

    fn set_cursor_position(&mut self, row: usize, col: usize) {
        self.row = row.min(self.rows() - 1);
        self.col = col.min(self.cols() - 1);
    }

    /// Erase from cursor to end of screen (cursor row partial + all rows below).
    fn clear_to_end_of_screen(&mut self) {
        self.clear_to_end_of_line();
        let next_row = self.row + 1;
        let total_rows = self.rows();
        if next_row < total_rows {
            let row_bytes = CHAR_H * self.info.stride * self.info.bytes_per_pixel;
            unsafe {
                core::ptr::write_bytes(
                    self.buf.add(next_row * row_bytes),
                    0,
                    (total_rows - next_row) * row_bytes,
                );
            }
        }
    }

    /// Erase from start of screen to cursor (all rows above + cursor row partial).
    fn clear_to_start_of_screen(&mut self) {
        let row_bytes = CHAR_H * self.info.stride * self.info.bytes_per_pixel;
        if self.row > 0 {
            unsafe {
                core::ptr::write_bytes(self.buf, 0, self.row * row_bytes);
            }
        }
        self.clear_to_start_of_line();
    }

    /// Erase from cursor to end of the current line.
    fn clear_to_end_of_line(&mut self) {
        let py = self.row * CHAR_H;
        let start_px = self.col * CHAR_W;
        for row_i in 0..CHAR_H {
            for col_i in start_px..self.cols() * CHAR_W {
                self.put_pixel(col_i, py + row_i, self.bg.0, self.bg.1, self.bg.2);
            }
        }
    }

    /// Erase from start of the current line to (and including) the cursor cell.
    fn clear_to_start_of_line(&mut self) {
        let py = self.row * CHAR_H;
        let end_px = (self.col + 1) * CHAR_W;
        for row_i in 0..CHAR_H {
            for col_i in 0..end_px {
                self.put_pixel(col_i, py + row_i, self.bg.0, self.bg.1, self.bg.2);
            }
        }
    }

    /// Insert `n` blank lines at the cursor row, pushing existing lines down.
    /// Lines that scroll past the bottom are discarded.
    fn insert_lines(&mut self, n: usize) {
        let total_rows = self.rows();
        let n = n.min(total_rows - self.row);
        let row_bytes = CHAR_H * self.info.stride * self.info.bytes_per_pixel;
        let rows_to_move = total_rows - self.row - n;
        if rows_to_move > 0 {
            unsafe {
                core::ptr::copy(
                    self.buf.add(self.row * row_bytes),
                    self.buf.add((self.row + n) * row_bytes),
                    rows_to_move * row_bytes,
                );
            }
        }
        unsafe {
            core::ptr::write_bytes(self.buf.add(self.row * row_bytes), 0, n * row_bytes);
        }
    }

    /// Delete `n` lines starting at the cursor row, pulling lines below up.
    /// Blank lines fill in at the bottom.
    fn delete_lines(&mut self, n: usize) {
        let total_rows = self.rows();
        let n = n.min(total_rows - self.row);
        let row_bytes = CHAR_H * self.info.stride * self.info.bytes_per_pixel;
        let rows_to_move = total_rows - self.row - n;
        if rows_to_move > 0 {
            unsafe {
                core::ptr::copy(
                    self.buf.add((self.row + n) * row_bytes),
                    self.buf.add(self.row * row_bytes),
                    rows_to_move * row_bytes,
                );
            }
        }
        unsafe {
            core::ptr::write_bytes(
                self.buf.add((total_rows - n) * row_bytes),
                0,
                n * row_bytes,
            );
        }
    }

    fn set_color(&mut self, code: u8) {
        match code {
            0  => { self.fg = Color::BRIGHT_WHITE; self.bg = Color::BLACK; }
            // Normal foreground (30–37)
            30 => self.fg = Color::BLACK,
            31 => self.fg = Color::RED,
            32 => self.fg = Color::GREEN,
            33 => self.fg = Color::YELLOW,
            34 => self.fg = Color::BLUE,
            35 => self.fg = Color::MAGENTA,
            36 => self.fg = Color::CYAN,
            37 => self.fg = Color::WHITE,
            // Normal background (40–47)
            40 => self.bg = Color::BLACK,
            41 => self.bg = Color::RED,
            42 => self.bg = Color::GREEN,
            43 => self.bg = Color::YELLOW,
            44 => self.bg = Color::BLUE,
            45 => self.bg = Color::MAGENTA,
            46 => self.bg = Color::CYAN,
            47 => self.bg = Color::WHITE,
            // Bright foreground (90–97)
            90  => self.fg = Color::BRIGHT_BLACK,
            91  => self.fg = Color::BRIGHT_RED,
            92  => self.fg = Color::BRIGHT_GREEN,
            93  => self.fg = Color::BRIGHT_YELLOW,
            94  => self.fg = Color::BRIGHT_BLUE,
            95  => self.fg = Color::BRIGHT_MAGENTA,
            96  => self.fg = Color::BRIGHT_CYAN,
            97  => self.fg = Color::BRIGHT_WHITE,
            // Bright background (100–107)
            100 => self.bg = Color::BRIGHT_BLACK,
            101 => self.bg = Color::BRIGHT_RED,
            102 => self.bg = Color::BRIGHT_GREEN,
            103 => self.bg = Color::BRIGHT_YELLOW,
            104 => self.bg = Color::BRIGHT_BLUE,
            105 => self.bg = Color::BRIGHT_MAGENTA,
            106 => self.bg = Color::BRIGHT_CYAN,
            107 => self.bg = Color::BRIGHT_WHITE,
            _ => {}
        }
    }

    fn put_pixel(&mut self, x: usize, y: usize, r: u8, g: u8, b: u8) {
        let bpp = self.info.bytes_per_pixel;
        let off = (y * self.info.stride + x) * bpp;
        if off + bpp > self.buf_len {
            return;
        }
        // SAFETY: bounds checked above.
        unsafe {
            match self.info.pixel_format {
                PixelFormat::Bgr => {
                    self.buf.add(off).write_volatile(b);
                    self.buf.add(off + 1).write_volatile(g);
                    self.buf.add(off + 2).write_volatile(r);
                }
                _ => {
                    self.buf.add(off).write_volatile(r);
                    self.buf.add(off + 1).write_volatile(g);
                    self.buf.add(off + 2).write_volatile(b);
                }
            }
        }
    }

    /// Write a single character, advancing the cursor.
    ///
    /// - `'\n'` moves to the start of the next row.
    /// - `'\x08'` (backspace) erases the previous character and moves the cursor back.
    /// - All other printable ASCII characters are rendered using the 8×8 font.
    ///
    /// Scrolls up by one row when the cursor passes the last row.
    pub fn write_char(&mut self, c: char) {
        if c == '\n' {
            self.col = 0;
            self.row += 1;
        } else if c == '\r' {
            self.col = 0;
        } else if c == '\x08' {
            // Backspace: move cursor left one position and erase the glyph.
            if self.col > 0 {
                self.col -= 1;
            } else if self.row > 0 {
                self.row -= 1;
                self.col = self.cols() - 1;
            }
            let px = self.col * CHAR_W;
            let py = self.row * CHAR_H;
            for row_i in 0..CHAR_H {
                for col_i in 0..CHAR_W {
                    self.put_pixel(px + col_i, py + row_i, 0, 0, 0);
                }
            }
        } else {
            let code = c as usize;
            let glyph = if (32..=126).contains(&code) {
                FONT[code - 32]
            } else {
                FONT[0]
            };

            let px = self.col * CHAR_W;
            let py = self.row * CHAR_H;

            for (row_i, &row_bits) in glyph.iter().enumerate() {
                for col_i in 0..CHAR_W {
                    if (row_bits >> (7 - col_i)) & 1 == 1 {
                        self.put_pixel(px + col_i, py + row_i, self.fg.0, self.fg.1, self.fg.2);
                    } else {
                        self.put_pixel(px + col_i, py + row_i, self.bg.0, self.bg.1, self.bg.2);
                    }
                }
            }

            self.col += 1;
            if self.col >= self.cols() {
                self.col = 0;
                self.row += 1;
            }
        }

        // Scroll when we go past the last row.
        if self.row >= self.rows() {
            self.scroll_up();
            self.row = self.rows() - 1;
        }
    }

    /// Zero the entire framebuffer and reset the cursor to (0, 0).
    pub fn clear_screen(&mut self) {
        // SAFETY: buf and buf_len describe the full framebuffer allocation.
        unsafe {
            core::ptr::write_bytes(self.buf, 0, self.buf_len);
        }
        self.col = 0;
        self.row = 0;
    }

    fn clear_line(&mut self) {
        let py = self.row * CHAR_H;
        for row_i in 0..CHAR_H {
            for col_i in 0..self.cols() * CHAR_W {
                self.put_pixel(col_i, py + row_i, self.bg.0, self.bg.1, self.bg.2);
            }
        }
        self.col = 0;
    }
}

// ---------------------------------------------------------------------------
// fmt::Write — enables write!() / format_args!() on VgaBuffer
// ---------------------------------------------------------------------------

impl Write for VgaBuffer {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        self.parse_ansi(s);
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Global writer — initialised once at boot, then used by print!/println!
// ---------------------------------------------------------------------------

static WRITER: Mutex<Option<VgaBuffer>> = Mutex::new(None);

/// Initialise the global writer from the bootloader framebuffer.
/// Must be called before any print!/println! use.
pub fn init(buf: &mut [u8], info: FrameBufferInfo) {
    let mut w = WRITER.lock();
    *w = Some(VgaBuffer::new(buf, info));
    if let Some(vga) = w.as_mut() {
        vga.clear_screen();
    }
}

/// Clear the screen and reset the cursor to (0, 0).
pub fn clear_screen() {
    if let Some(w) = WRITER.lock().as_mut() {
        w.clear_screen();
    }
}

/// Blit a packed pixel buffer to the top-left of the framebuffer.
///
/// `buf` contains `width * height` u32 values each packed as `0x00RRGGBB`.
/// Acquires the framebuffer lock **once** for the whole operation.
pub fn blit_rgb32(buf: &[u32], width: usize, height: usize) {
    if let Some(w) = WRITER.lock().as_mut() {
        let bpp    = w.info.bytes_per_pixel;
        let stride = w.info.stride;
        let bgr    = matches!(w.info.pixel_format, PixelFormat::Bgr);
        for y in 0..height {
            for x in 0..width {
                let rgb = buf[y * width + x];
                let r = ((rgb >> 16) & 0xFF) as u8;
                let g = ((rgb >>  8) & 0xFF) as u8;
                let b = ( rgb        & 0xFF) as u8;
                let off = (y * stride + x) * bpp;
                if off + bpp > w.buf_len { continue; }
                unsafe {
                    if bgr {
                        w.buf.add(off).write_volatile(b);
                        w.buf.add(off + 1).write_volatile(g);
                        w.buf.add(off + 2).write_volatile(r);
                    } else {
                        w.buf.add(off).write_volatile(r);
                        w.buf.add(off + 1).write_volatile(g);
                        w.buf.add(off + 2).write_volatile(b);
                    }
                }
            }
        }
    }
}

/// Write one pixel directly to the framebuffer, bypassing the text cursor.
///
/// `x` and `y` are pixel coordinates from the top-left corner.  Out-of-bounds
/// coordinates are silently ignored.  `rgb` is packed as `0x00RRGGBB`; the
/// function converts to the actual framebuffer pixel format automatically.
pub fn set_pixel(x: i32, y: i32, rgb: u32) {
    if x < 0 || y < 0 { return; }
    let r = ((rgb >> 16) & 0xFF) as u8;
    let g = ((rgb >>  8) & 0xFF) as u8;
    let b = ( rgb        & 0xFF) as u8;
    if let Some(w) = WRITER.lock().as_mut() {
        w.put_pixel(x as usize, y as usize, r, g, b);
    }
}

/// Called by the print! macro — writes to the VGA framebuffer only.
/// The print! macro also calls serial::_print separately.
pub fn _print(args: core::fmt::Arguments) {
    // Format to a complete string first so the ANSI parser never sees a
    // sequence split across multiple write_str calls (which write_fmt does
    // for each literal piece and argument in a format string separately).
    let s = alloc::format!("{}", args);
    if let Some(w) = WRITER.lock().as_mut() {
        w.parse_ansi(&s);
    }
}