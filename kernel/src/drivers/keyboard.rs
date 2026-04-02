//! Low-level PS/2 keyboard driver — scancode set 1, polled.

use core::sync::atomic::{AtomicBool, Ordering};

const DATA_PORT: u16 = 0x60;
const STATUS_PORT: u16 = 0x64;

/// Read one byte from an x86 I/O port.
#[inline]
unsafe fn inb(port: u16) -> u8 {
    let val: u8;
    core::arch::asm!(
        "in al, dx",
        out("al") val,
        in("dx") port,
        options(nomem, nostack, preserves_flags)
    );
    val
}

/// Tracks whether either Shift key is currently held.
static SHIFT: AtomicBool = AtomicBool::new(false);

pub enum Key {
    Char(char),
    Backspace,
    Enter,
    Unknown,
}

/// Block until a key-press event is available, then decode and return it.
/// Key-release and unhandled scancodes are silently consumed.
pub fn next_key() -> Key {
    loop {
        // Wait: status bit 0 = output buffer full = data ready.
        while unsafe { inb(STATUS_PORT) } & 0x01 == 0 {
            core::hint::spin_loop();
        }
        let sc = unsafe { inb(DATA_PORT) };

        // Update shift state for both make and break codes.
        match sc {
            0x2A | 0x36 => {
                SHIFT.store(true, Ordering::Relaxed);
                continue;
            }
            0xAA | 0xB6 => {
                SHIFT.store(false, Ordering::Relaxed);
                continue;
            }
            _ => {}
        }

        // Skip break codes (key release — bit 7 set).
        if sc & 0x80 != 0 {
            continue;
        }

        let shift = SHIFT.load(Ordering::Relaxed);
        return match sc {
            0x0E => Key::Backspace,
            0x1C => Key::Enter,
            _ => match scancode_to_char(sc, shift) {
                Some(c) => Key::Char(c),
                None => Key::Unknown,
            },
        };
    }
}

/// Map a scancode (set 1, make code only) to a Unicode character.
fn scancode_to_char(sc: u8, shift: bool) -> Option<char> {
    Some(match sc {
        0x02 => if shift { '!' } else { '1' },
        0x03 => if shift { '@' } else { '2' },
        0x04 => if shift { '#' } else { '3' },
        0x05 => if shift { '$' } else { '4' },
        0x06 => if shift { '%' } else { '5' },
        0x07 => if shift { '^' } else { '6' },
        0x08 => if shift { '&' } else { '7' },
        0x09 => if shift { '*' } else { '8' },
        0x0A => if shift { '(' } else { '9' },
        0x0B => if shift { ')' } else { '0' },
        0x0C => if shift { '_' } else { '-' },
        0x0D => if shift { '+' } else { '=' },
        0x10 => if shift { 'Q' } else { 'q' },
        0x11 => if shift { 'W' } else { 'w' },
        0x12 => if shift { 'E' } else { 'e' },
        0x13 => if shift { 'R' } else { 'r' },
        0x14 => if shift { 'T' } else { 't' },
        0x15 => if shift { 'Y' } else { 'y' },
        0x16 => if shift { 'U' } else { 'u' },
        0x17 => if shift { 'I' } else { 'i' },
        0x18 => if shift { 'O' } else { 'o' },
        0x19 => if shift { 'P' } else { 'p' },
        0x1A => if shift { '{' } else { '[' },
        0x1B => if shift { '}' } else { ']' },
        0x1E => if shift { 'A' } else { 'a' },
        0x1F => if shift { 'S' } else { 's' },
        0x20 => if shift { 'D' } else { 'd' },
        0x21 => if shift { 'F' } else { 'f' },
        0x22 => if shift { 'G' } else { 'g' },
        0x23 => if shift { 'H' } else { 'h' },
        0x24 => if shift { 'J' } else { 'j' },
        0x25 => if shift { 'K' } else { 'k' },
        0x26 => if shift { 'L' } else { 'l' },
        0x27 => if shift { ':' } else { ';' },
        0x28 => if shift { '"' } else { '\'' },
        0x29 => if shift { '~' } else { '`' },
        0x2B => if shift { '|' } else { '\\' },
        0x2C => if shift { 'Z' } else { 'z' },
        0x2D => if shift { 'X' } else { 'x' },
        0x2E => if shift { 'C' } else { 'c' },
        0x2F => if shift { 'V' } else { 'v' },
        0x30 => if shift { 'B' } else { 'b' },
        0x31 => if shift { 'N' } else { 'n' },
        0x32 => if shift { 'M' } else { 'm' },
        0x33 => if shift { '<' } else { ',' },
        0x34 => if shift { '>' } else { '.' },
        0x35 => if shift { '?' } else { '/' },
        0x39 => ' ',
        _ => return None,
    })
}
