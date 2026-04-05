// edit <name> — line-append editor
//
// Opens an existing file from the FAT volume (if present) and lets the user
// append lines interactively.  Two built-in commands are recognised:
//   save   — write the buffer to the FAT volume and exit
//   quit   — discard changes and exit
//
// Buffer limit: 4 KiB.  Files larger than that cannot be edited.

pub fn run(argv: &[&str]) {
    if argv.is_empty() {
        crate::println!("usage: edit <name>");
        return;
    }
    let name = argv[0];

    let mut buf = [0u8; 4096];
    let mut len = 0usize;
    let mut line_buf = [0u8; 256];

    // Load existing file from FAT if present, and display it.
    if let Some(data) = crate::fs::fat::fat_read_file(name) {
        if data.len() > buf.len() {
            crate::println!("edit: file too large ({} bytes, max {})", data.len(), buf.len());
            return;
        }
        buf[..data.len()].copy_from_slice(&data);
        len = data.len();
        crate::println!("-- {} ({} bytes) ---", name, len);
        match core::str::from_utf8(&buf[..len]) {
            Ok(s) => {
                crate::print!("{}", s);
                if !s.ends_with('\n') { crate::println!(); }
            }
            Err(_) => {
                crate::println!("[binary content not shown]");
            }
        }
        crate::println!("---");
    } else {
        crate::println!("-- new file: {} ---", name);
    }
    crate::println!("Append lines.  Commands: :w = save  :q = quit");

    loop {
        crate::print!("> ");

        let line = match crate::shell::input::read_line(&mut line_buf) {
            Some(l) => l,
            None => break,
        };

        // Strip trailing CR/LF.
        let bytes = line.as_bytes();
        let mut end = bytes.len();
        if end > 0 && bytes[end - 1] == b'\n' { end -= 1; }
        if end > 0 && bytes[end - 1] == b'\r' { end -= 1; }
        let trimmed = &bytes[..end];

        if trimmed == b":q" {
            crate::println!("quit (no save)");
            break;
        }

        if trimmed == b":w" {
            // Persist to FAT.
            let fat_ok = crate::fs::fat::fat_write_file(name, &buf[..len]);
            // Also update the in-memory pool so `run` can see it this session.
            let mem_ok = match crate::fs::alloc_write_buf(&buf[..len]) {
                Some(static_buf) => {
                    crate::fs::register_file(name, static_buf);
                    true
                }
                None => false,
            };
            if fat_ok || mem_ok {
                crate::println!("saved {} ({} bytes)", name, len);
            } else {
                crate::println!("edit: save failed");
            }
            break;
        }

        // Append the line + newline to the buffer.
        let needed = trimmed.len() + 1;
        if len + needed > buf.len() {
            crate::println!("edit: buffer full ({} / {} bytes)", len, buf.len());
            continue;
        }
        buf[len..len + trimmed.len()].copy_from_slice(trimmed);
        len += trimmed.len();
        buf[len] = b'\n';
        len += 1;
    }
}
