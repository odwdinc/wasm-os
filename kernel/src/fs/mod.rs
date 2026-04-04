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

/// Call `f(name, size)` for every registered file (in registration order).
pub fn for_each_file<F: FnMut(&str, usize)>(mut f: F) {
    unsafe {
        for i in 0..FILE_COUNT {
            if let Some(file) = FILE_TABLE[i] {
                f(file.name_str(), file.data.len());
            }
        }
    }
}

/// Remove a file from the in-memory table by name.  Returns `true` if found.
/// Leaves a `None` hole in the table; iterators already skip `None` slots.
pub fn remove_file(name: &str) -> bool {
    unsafe {
        for i in 0..FILE_COUNT {
            if let Some(f) = FILE_TABLE[i] {
                if f.name_str() == name {
                    FILE_TABLE[i] = None;
                    return true;
                }
            }
        }
    }
    false
}

// ── Write-buffer pool ────────────────────────────────────────────────────────
//
// Provides static storage for files created with the `write` shell command.
// No heap: each slot is a fixed 4 KiB array.  Once all slots are claimed they
// cannot be reclaimed (this sprint).

const WRITE_SLOTS:    usize = 4;
const WRITE_SLOT_CAP: usize = 4096;

static mut WRITE_POOL: [[u8; WRITE_SLOT_CAP]; WRITE_SLOTS] =
    [[0u8; WRITE_SLOT_CAP]; WRITE_SLOTS];
static mut WRITE_POOL_NEXT: usize = 0;

/// Claim a slot, copy `data` into it, and return a `'static` slice.
/// Returns `None` if all slots are exhausted or `data` exceeds slot capacity.
pub fn alloc_write_buf(data: &[u8]) -> Option<&'static [u8]> {
    if data.len() > WRITE_SLOT_CAP {
        return None;
    }
    unsafe {
        if WRITE_POOL_NEXT >= WRITE_SLOTS {
            return None;
        }
        let slot = WRITE_POOL_NEXT;
        WRITE_POOL_NEXT += 1;
        WRITE_POOL[slot][..data.len()].copy_from_slice(data);
        Some(&WRITE_POOL[slot][..data.len()])
    }
}

// ── Persist to Ramdisk (Sprint D.5) ─────────────────────────────────────────

/// Serialize the entire in-memory file table into the Ramdisk using WasmFS
/// format.  Returns the number of files successfully written.
///
/// The Ramdisk is volatile (it is a `static mut` byte array reset to zero on
/// cold boot), so persistence across reboots requires a real block device
/// driver (virtio-blk, Sprint D stretch).  Within a session, any file visible
/// via `ls` — including those added by `write` — is preserved in the Ramdisk
/// and can be read back by `WasmFs<Ramdisk>`.
pub fn save_to_ramdisk() -> usize {
    use wasmfs::WasmFs;
    use block::Ramdisk;

    let mut wfs = WasmFs::new(Ramdisk::get());
    let mut saved = 0usize;

    unsafe {
        for i in 0..FILE_COUNT {
            if let Some(f) = FILE_TABLE[i] {
                if wfs.fs_write(f.name_str(), f.data).is_ok() {
                    saved += 1;
                }
            }
        }
    }
    saved
}
