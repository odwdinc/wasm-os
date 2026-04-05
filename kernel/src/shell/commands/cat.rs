// cat <file> — print file contents to the terminal
//
// Resolves the path relative to the current working directory.
// Non-UTF-8 files are shown as a hex dump summary instead of raw bytes.

use alloc::string::String;

pub fn run(argv: &[&str]) {
    if argv.is_empty() {
        crate::println!("usage: cat <file>");
        return;
    }
    let name = argv[0];
    let fat_path = resolve_fat_path(name);

    match crate::fs::fat::fat_read_path(&fat_path) {
        None => {
            crate::println!("cat: {}: no such file", name);
        }
        Some(data) => {
            match core::str::from_utf8(&data) {
                Ok(s) => {
                    crate::print!("{}", s);
                    if !s.ends_with('\n') {
                        crate::println!();
                    }
                }
                Err(_) => {
                    crate::println!("[binary file: {} bytes]", data.len());
                }
            }
        }
    }
}

/// Build the FAT-relative path for a file name given the current CWD.
/// Result is suitable for `fat_read_path`: "file.txt" or "subdir/file.txt".
fn resolve_fat_path(name: &str) -> String {
    if name.starts_with('/') {
        // Absolute: strip leading '/' — fat_read_path handles the rest.
        String::from(name)
    } else {
        let cwd = crate::shell::get_cwd();
        if cwd == "/" {
            String::from(name)
        } else {
            // CWD is "/subdir"; build "subdir/file"
            let dir = cwd.trim_start_matches('/');
            let mut s = String::from(dir);
            s.push('/');
            s.push_str(name);
            s
        }
    }
}
