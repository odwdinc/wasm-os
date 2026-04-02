// fs.rs — In-memory file system (Sprint 3.1)
//
// Fixed-size, no-heap file table.  All names and data are 'static because
// registered files are either embedded byte arrays or other static slices.

const MAX_FILES: usize = 16;

#[derive(Clone, Copy)]
pub struct File {
    pub name: &'static str,
    pub data: &'static [u8],
}

// Const sentinel used to zero-initialise the table without requiring Default.
const NONE_FILE: Option<File> = None;

static mut FILE_TABLE: [Option<File>; MAX_FILES] = [NONE_FILE; MAX_FILES];
static mut FILE_COUNT: usize = 0;

/// Register a named file.  Silently does nothing if the table is full.
pub fn register_file(name: &'static str, data: &'static [u8]) {
    // Safety: single-core bare-metal, no preemption.
    unsafe {
        if FILE_COUNT < MAX_FILES {
            FILE_TABLE[FILE_COUNT] = Some(File { name, data });
            FILE_COUNT += 1;
        }
    }
}

/// Look up a file by exact name.  Returns its raw bytes if found.
pub fn find_file(name: &str) -> Option<&'static [u8]> {
    unsafe {
        for i in 0..FILE_COUNT {
            if let Some(f) = FILE_TABLE[i] {
                if f.name == name {
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
                f(file.name);
            }
        }
    }
}
