// fs/mod.rs — Filesystem subsystem (Sprint 3.1 + Sprint D)
//
// mod.rs holds the in-memory file table (static, no-heap).
// block.rs  — BlockDevice trait, Ramdisk, EmbeddedDisk
// wasmfs.rs — WasmFS flat-filesystem format + mount_from_image

pub mod block;
pub mod wasmfs;

const MAX_FILES: usize = 16;

#[derive(Clone, Copy)]
pub struct File {
    name:     [u8; 32],
    name_len: usize,
    pub data: &'static [u8],
}

impl File {
    pub fn name_str(&self) -> &str {
        core::str::from_utf8(&self.name[..self.name_len]).unwrap_or("")
    }
}

// Const sentinel used to zero-initialise the table without requiring Default.
const NONE_FILE: Option<File> = None;

static mut FILE_TABLE: [Option<File>; MAX_FILES] = [NONE_FILE; MAX_FILES];
static mut FILE_COUNT: usize = 0;

/// Register a named file.  Silently does nothing if the table is full.
/// `name` need not be `'static`; the bytes are copied into the static table.
pub fn register_file(name: &str, data: &'static [u8]) {
    // Safety: single-core bare-metal, no preemption.
    unsafe {
        if FILE_COUNT < MAX_FILES {
            let mut name_bytes = [0u8; 32];
            let len = name.len().min(32);
            name_bytes[..len].copy_from_slice(&name.as_bytes()[..len]);
            FILE_TABLE[FILE_COUNT] = Some(File {
                name:     name_bytes,
                name_len: len,
                data,
            });
            FILE_COUNT += 1;
        }
    }
}

/// Look up a file by exact name.  Returns its raw bytes if found.
pub fn find_file(name: &str) -> Option<&'static [u8]> {
    unsafe {
        for i in 0..FILE_COUNT {
            if let Some(f) = FILE_TABLE[i] {
                if f.name_str() == name {
                    return Some(f.data);
                }
            }
        }
    }
    None
}

/// Call `f` with the name of every registered file (in registration order).
pub fn for_each_file<F: FnMut(&str)>(mut f: F) {
    unsafe {
        for i in 0..FILE_COUNT {
            if let Some(file) = FILE_TABLE[i] {
                f(file.name_str());
            }
        }
    }
}
