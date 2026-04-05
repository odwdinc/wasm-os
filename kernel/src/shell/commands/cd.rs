// cd <dir> — change working directory
//
// Supports absolute paths ("/", "/subdir") and relative paths ("subdir", "..").
// Only one level of subdirectory is supported (flat FAT root structure).

use alloc::string::String;

pub fn run(argv: &[&str]) {
    let target = argv.first().copied().unwrap_or("/");

    // Resolve to an absolute path string.
    let resolved = resolve(target);

    if resolved == "/" {
        crate::shell::set_cwd("/");
        return;
    }

    // Strip leading "/" for the FAT lookup.
    let fat_name = resolved.trim_start_matches('/');
    if !crate::fs::fat::fat_is_dir(fat_name) {
        crate::println!("cd: {}: not a directory", target);
        return;
    }

    crate::shell::set_cwd(&resolved);
}

/// Build a normalised absolute path from `target` and the current CWD.
fn resolve(target: &str) -> String {
    let raw: String = if target.starts_with('/') {
        String::from(target)
    } else if target == ".." {
        // Go up one level (we only have one level, so parent is always root).
        let cwd = crate::shell::get_cwd();
        match cwd.rfind('/') {
            Some(0) | None => String::from("/"),
            Some(pos)      => String::from(&cwd[..pos]),
        }
    } else {
        let cwd = crate::shell::get_cwd();
        let mut s = String::from(cwd);
        if !s.ends_with('/') { s.push('/'); }
        s.push_str(target);
        s
    };

    // Normalise: strip trailing slashes, ensure non-empty root.
    let trimmed = raw.trim_end_matches('/');
    if trimmed.is_empty() {
        String::from("/")
    } else {
        String::from(trimmed)
    }
}
