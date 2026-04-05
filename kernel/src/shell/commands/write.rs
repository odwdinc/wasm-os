// write <name> <hex-bytes>
//
// Decodes a hex string (e.g. "deadbeef") into raw bytes, writes the file to
// the mounted FAT volume, and registers it in the in-memory file table.
//
// Example:
//   write test.bin 48656c6c6f
//
// Files are written immediately to the FAT disk (virtio-blk) or ramdisk FAT,
// so a subsequent `run` or `ls` sees them without needing `save`.

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
    let data = &tmp[..len];

    // Write to FAT volume (persists across reboots on virtio-blk).
    if !crate::fs::fat::fat_write_file(name, data) {
        crate::println!("write: FAT write failed");
        return;
    }

    // Also keep in the in-memory pool so `run` can get a 'static slice.
    match crate::fs::alloc_write_buf(data) {
        Some(buf) => {
            crate::fs::register_file(name, buf);
        }
        None => {
            // Pool exhausted — file is on disk but can't be run this session
            // without a reboot.
            crate::println!("write: in-memory pool full; file saved to disk only");
            return;
        }
    }
    crate::println!("wrote {} ({} bytes)", name, len);
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
