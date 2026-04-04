// write <name> <hex-bytes>
//
// Decodes a hex string (e.g. "deadbeef") into raw bytes, stores them in the
// static write-buffer pool, and registers the result as a named file.
//
// Example:
//   write test.bin 48656c6c6f
//
// Limitations (this sprint):
//   - Maximum file size: 4 096 bytes (one pool slot).
//   - Pool has 4 slots total; once exhausted, further writes fail.
//   - Files written this way are in-memory only; they do not survive reboot.

pub fn run(argv: &[&str]) {
    if argv.len() < 2 {
        crate::println!("usage: write <name> <hex-bytes>");
        return;
    }
    let name = argv[0];
    let hex  = argv[1];

    // Temporary decode buffer on the stack (max 4 KiB = 8 192 hex chars).
    let mut tmp = [0u8; 4096];
    let len = match hex_decode(hex, &mut tmp) {
        Some(n) => n,
        None => {
            crate::println!("write: invalid hex string");
            return;
        }
    };

    match crate::fs::alloc_write_buf(&tmp[..len]) {
        Some(buf) => {
            crate::fs::register_file(name, buf);
            crate::println!("wrote {} ({} bytes)", name, len);
        }
        None => {
            crate::println!("write: out of write-buffer pool slots or data too large (max 4096 bytes)");
        }
    }
}

fn hex_decode(hex: &str, out: &mut [u8]) -> Option<usize> {
    let src = hex.as_bytes();
    if src.len() % 2 != 0 {
        return None;
    }
    let len = src.len() / 2;
    if len > out.len() {
        return None;
    }
    for i in 0..len {
        let hi = hex_nibble(src[i * 2])?;
        let lo = hex_nibble(src[i * 2 + 1])?;
        out[i] = (hi << 4) | lo;
    }
    Some(len)
}

fn hex_nibble(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}
