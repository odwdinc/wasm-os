// fs/mod.rs — Filesystem subsystem
//
// mod.rs   — in-memory file table (for wasm engine's 'static slice requirement)
// block.rs — BlockDevice trait + Ramdisk/VirtioBlk impls
// fat.rs   — FAT12/16/32 via rust-fatfs; BlockIo adapter; global FS handle

pub mod block;
pub mod fat;
pub mod wasmfs; // kept for reference; no longer used at boot

const MAX_FILES: usize = 64;

#[derive(Clone, Copy)]
pub struct File {
    name:     [u8; 64],
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

/// Zero-initialise the file table.  Call once at boot before any register/find.
/// Needed because the bootloader may not zero BSS for all physical pages.
pub fn init() {
    unsafe {
        FILE_TABLE = [NONE_FILE; MAX_FILES];
        FILE_COUNT = 0;
    }
}

/// Register a named file.  Silently does nothing if the table is full.
/// `name` need not be `'static`; the bytes are copied into the static table.
pub fn register_file(name: &str, data: &'static [u8]) {
    // Safety: single-core bare-metal, no preemption.
    unsafe {
        if FILE_COUNT < MAX_FILES {
            let mut name_bytes = [0u8; 64];
            let len = name.len().min(64);
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

const WRITE_SLOTS:    usize = 16;
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

// ── Static pool for files loaded from a virtio-blk disk at boot ─────────────
//
// Each slot is 8 KiB — large enough for typical WASM demo modules.
// Slots are allocated once and never freed (single-session lifetime).

pub const DISK_SLOT_SIZE: usize = 8192;
const DISK_SLOTS: usize = 64;

static mut DISK_POOL: [[u8; DISK_SLOT_SIZE]; DISK_SLOTS] =
    [[0u8; DISK_SLOT_SIZE]; DISK_SLOTS];
static mut DISK_POOL_NEXT: usize = 0;

/// Claim a slot of exactly `len` bytes from the disk pool.
/// Returns a raw pointer to the slot's start, or `None` if the pool is
/// exhausted or `len` exceeds `DISK_SLOT_SIZE`.
pub fn alloc_disk_slot(len: usize) -> Option<*mut u8> {
    if len > DISK_SLOT_SIZE {
        return None;
    }
    unsafe {
        if DISK_POOL_NEXT >= DISK_SLOTS {
            return None;
        }
        let idx = DISK_POOL_NEXT;
        DISK_POOL_NEXT += 1;
        Some(DISK_POOL[idx].as_mut_ptr())
    }
}

// ── FAT boot loader ──────────────────────────────────────────────────────────

/// Read every file from the mounted FAT volume into the static disk-pool and
/// register it in the in-memory table.  Called once at boot after FAT mount.
pub fn load_fat_files_to_table() {
    // Collect names first to avoid holding the FAT lock while writing to pool.
    let mut names: [Option<([u8; 64], usize)>; DISK_SLOTS] = [None; DISK_SLOTS];
    let mut count = 0usize;
    fat::fat_list(|name, _size| {
        if count < DISK_SLOTS {
            let mut buf = [0u8; 64];
            let len = name.len().min(64);
            buf[..len].copy_from_slice(&name.as_bytes()[..len]);
            names[count] = Some((buf, len));
            count += 1;
        }
    });

    for slot in names[..count].iter().flatten() {
        let (buf, len) = slot;
        let name = core::str::from_utf8(&buf[..*len]).unwrap_or(""); // buf is 64 bytes
        if name.is_empty() { continue; }
        if let Some(data) = fat::fat_read_file(name) {
            if let Some(ptr) = alloc_disk_slot(data.len()) {
                // Safety: ptr is valid, unique, static-lifetime (backed by DISK_POOL).
                unsafe {
                    core::ptr::copy_nonoverlapping(data.as_ptr(), ptr, data.len());
                    let slice = core::slice::from_raw_parts(ptr, data.len());
                    register_file(name, slice);
                }
            }
        }
    }
}

// ── Persist: write current in-memory file to FAT disk ────────────────────────

/// Flush all registered files back to the FAT volume.  Useful if `write`
/// added in-memory-only files that were not yet synced to disk.
pub fn save_to_fat() -> usize {
    let mut saved = 0usize;
    unsafe {
        for i in 0..FILE_COUNT {
            if let Some(f) = FILE_TABLE[i] {
                if fat::fat_write_file(f.name_str(), f.data) {
                    saved += 1;
                }
            }
        }
    }
    saved
}
