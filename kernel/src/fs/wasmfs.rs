// fs/wasmfs.rs — WasmFS flat filesystem (Sprint D.2)
#![allow(dead_code)] // WasmFS API built for future sprints; not all methods called yet
//
// On-disk layout (all blocks are 512 bytes):
//
//   Block 0        — directory block (8 × 64-byte entries)
//   Blocks 1..N    — file data (contiguous per file, no fragmentation)
//
// Directory entry layout (64 bytes):
//   offset  0..32  name:        null-padded UTF-8 filename
//   offset 32..36  start_block: u32 LE — first data block
//   offset 36..40  size:        u32 LE — file size in bytes
//   offset 40      flags:       u8     — bit 0 = valid
//   offset 41..64  reserved
//
// Write strategy: always append after the last used block.
// Overwriting a file orphans its old blocks (no compaction in this sprint).

use super::block::{BlockDevice, BLOCK_SIZE};

pub const DIR_ENTRY_SIZE: usize = 64;
pub const DIR_ENTRIES_PER_BLOCK: usize = BLOCK_SIZE / DIR_ENTRY_SIZE; // 8
pub const DIR_BLOCK: u32 = 0;
pub const DATA_START_BLOCK: u32 = 1;
pub const MAX_OPEN_FILES: usize = 4;

pub const FLAG_VALID: u8 = 0x01;

// ---------------------------------------------------------------------------
// Directory entry
// ---------------------------------------------------------------------------

pub struct DirEntry {
    pub name: [u8; 32],
    pub start_block: u32,
    pub size: u32,
    pub flags: u8,
}

impl DirEntry {
    fn from_bytes(b: &[u8; DIR_ENTRY_SIZE]) -> Self {
        let mut name = [0u8; 32];
        name.copy_from_slice(&b[0..32]);
        let start_block = u32::from_le_bytes([b[32], b[33], b[34], b[35]]);
        let size        = u32::from_le_bytes([b[36], b[37], b[38], b[39]]);
        let flags       = b[40];
        DirEntry { name, start_block, size, flags }
    }

    fn to_bytes(&self) -> [u8; DIR_ENTRY_SIZE] {
        let mut b = [0u8; DIR_ENTRY_SIZE];
        b[0..32].copy_from_slice(&self.name);
        b[32..36].copy_from_slice(&self.start_block.to_le_bytes());
        b[36..40].copy_from_slice(&self.size.to_le_bytes());
        b[40] = self.flags;
        b
    }

    pub fn is_valid(&self) -> bool {
        self.flags & FLAG_VALID != 0
    }

    /// Returns the filename as a `&str`, trimming the null padding.
    pub fn name_str(&self) -> &str {
        let end = self.name.iter().position(|&b| b == 0).unwrap_or(32);
        core::str::from_utf8(&self.name[..end]).unwrap_or("")
    }
}

// ---------------------------------------------------------------------------
// Open-file table
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
struct OpenFile {
    start_block: u32,
    size:        u32,
    pos:         u32,
    in_use:      bool,
}

impl OpenFile {
    const fn empty() -> Self {
        OpenFile { start_block: 0, size: 0, pos: 0, in_use: false }
    }
}

pub type Fd = usize;

// ---------------------------------------------------------------------------
// WasmFs
// ---------------------------------------------------------------------------

pub struct WasmFs<D: BlockDevice> {
    dev:  D,
    open: [OpenFile; MAX_OPEN_FILES],
}

impl<D: BlockDevice> WasmFs<D> {
    pub fn new(dev: D) -> Self {
        WasmFs { dev, open: [OpenFile::empty(); MAX_OPEN_FILES] }
    }

    // -- internal helpers ---------------------------------------------------

    fn read_dir_block(&mut self) -> [u8; BLOCK_SIZE] {
        let mut buf = [0u8; BLOCK_SIZE];
        let _ = self.dev.read_block(DIR_BLOCK, &mut buf);
        buf
    }

    fn write_dir_block(&mut self, buf: &[u8; BLOCK_SIZE]) {
        let _ = self.dev.write_block(DIR_BLOCK, buf);
    }

    fn read_entry(dir: &[u8; BLOCK_SIZE], idx: usize) -> DirEntry {
        let off = idx * DIR_ENTRY_SIZE;
        let mut b = [0u8; DIR_ENTRY_SIZE];
        b.copy_from_slice(&dir[off..off + DIR_ENTRY_SIZE]);
        DirEntry::from_bytes(&b)
    }

    fn write_entry(dir: &mut [u8; BLOCK_SIZE], idx: usize, entry: &DirEntry) {
        let off = idx * DIR_ENTRY_SIZE;
        dir[off..off + DIR_ENTRY_SIZE].copy_from_slice(&entry.to_bytes());
    }

    /// Returns the first block number past all currently allocated data.
    fn next_free_block(&mut self) -> u32 {
        let dir = self.read_dir_block();
        let mut next = DATA_START_BLOCK;
        for i in 0..DIR_ENTRIES_PER_BLOCK {
            let e = Self::read_entry(&dir, i);
            if e.is_valid() {
                let blocks = ((e.size as usize + BLOCK_SIZE - 1) / BLOCK_SIZE).max(1);
                let end = e.start_block + blocks as u32;
                if end > next {
                    next = end;
                }
            }
        }
        next
    }

    // -- public API ---------------------------------------------------------

    /// Open a file by name.  Returns `None` if not found or no fd slots free.
    pub fn fs_open(&mut self, name: &str) -> Option<Fd> {
        let dir = self.read_dir_block();
        for i in 0..DIR_ENTRIES_PER_BLOCK {
            let e = Self::read_entry(&dir, i);
            if e.is_valid() && e.name_str() == name {
                for fd in 0..MAX_OPEN_FILES {
                    if !self.open[fd].in_use {
                        self.open[fd] = OpenFile {
                            start_block: e.start_block,
                            size:        e.size,
                            pos:         0,
                            in_use:      true,
                        };
                        return Some(fd);
                    }
                }
                return None; // all fd slots taken
            }
        }
        None // file not found
    }

    /// Read up to `len` bytes from `fd` into `buf`.  Returns bytes actually read.
    pub fn fs_read(&mut self, fd: Fd, buf: &mut [u8], len: usize) -> usize {
        if fd >= MAX_OPEN_FILES || !self.open[fd].in_use {
            return 0;
        }
        let remaining = self.open[fd].size.saturating_sub(self.open[fd].pos) as usize;
        let to_read   = len.min(remaining).min(buf.len());
        if to_read == 0 {
            return 0;
        }

        let mut done = 0;
        while done < to_read {
            let pos       = self.open[fd].pos as usize;
            let block_idx = pos / BLOCK_SIZE;
            let block_off = pos % BLOCK_SIZE;
            let lba       = self.open[fd].start_block + block_idx as u32;

            let mut blk = [0u8; BLOCK_SIZE];
            if self.dev.read_block(lba, &mut blk).is_err() {
                break;
            }

            let avail = (BLOCK_SIZE - block_off).min(to_read - done);
            buf[done..done + avail].copy_from_slice(&blk[block_off..block_off + avail]);
            done                  += avail;
            self.open[fd].pos     += avail as u32;
        }
        done
    }

    /// Write (create or overwrite) a file with the given name and data.
    ///
    /// Old blocks are orphaned on overwrite — no compaction in this sprint.
    /// Returns `Err(())` if the directory is full or a block write fails.
    pub fn fs_write(&mut self, name: &str, data: &[u8]) -> Result<(), ()> {
        let mut dir = self.read_dir_block();

        // Find an existing entry for this name, or the first empty slot.
        let mut slot: Option<usize> = None;
        for i in 0..DIR_ENTRIES_PER_BLOCK {
            let e = Self::read_entry(&dir, i);
            if e.is_valid() && e.name_str() == name {
                slot = Some(i);
                break;
            }
            if !e.is_valid() && slot.is_none() {
                slot = Some(i);
            }
        }
        let idx = slot.ok_or(())?; // directory full

        let start_block = self.next_free_block();

        // Write data blocks.
        let mut written = 0;
        let mut lba     = start_block;
        while written < data.len() {
            let mut blk   = [0u8; BLOCK_SIZE];
            let chunk     = (data.len() - written).min(BLOCK_SIZE);
            blk[..chunk].copy_from_slice(&data[written..written + chunk]);
            self.dev.write_block(lba, &blk).map_err(|_| ())?;
            written += chunk;
            lba     += 1;
        }

        // Update directory entry.
        let mut name_bytes = [0u8; 32];
        let src = name.as_bytes();
        name_bytes[..src.len().min(32)].copy_from_slice(&src[..src.len().min(32)]);

        let entry = DirEntry {
            name: name_bytes,
            start_block,
            size: data.len() as u32,
            flags: FLAG_VALID,
        };
        Self::write_entry(&mut dir, idx, &entry);
        self.write_dir_block(&dir);
        Ok(())
    }

    /// Release a file descriptor.
    pub fn fs_close(&mut self, fd: Fd) {
        if fd < MAX_OPEN_FILES {
            self.open[fd].in_use = false;
        }
    }

    /// Delete a file by name.  Returns `Err(())` if not found.
    pub fn fs_unlink(&mut self, name: &str) -> Result<(), ()> {
        let mut dir = self.read_dir_block();
        for i in 0..DIR_ENTRIES_PER_BLOCK {
            let e = Self::read_entry(&dir, i);
            if e.is_valid() && e.name_str() == name {
                let blank = DirEntry { name: [0u8; 32], start_block: 0, size: 0, flags: 0 };
                Self::write_entry(&mut dir, i, &blank);
                self.write_dir_block(&dir);
                return Ok(());
            }
        }
        Err(())
    }

    /// Call `f(name, size)` for every valid directory entry.
    pub fn fs_list<F: FnMut(&str, u32)>(&mut self, mut f: F) {
        let dir = self.read_dir_block();
        for i in 0..DIR_ENTRIES_PER_BLOCK {
            let e = Self::read_entry(&dir, i);
            if e.is_valid() {
                f(e.name_str(), e.size);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Boot mount
// ---------------------------------------------------------------------------

/// Parse a WasmFS image embedded as a `&'static [u8]` and register every
/// valid file into the kernel's in-memory file table (`crate::fs::register_file`).
///
/// Called once at boot from `kernel_main` after the image has been embedded
/// via `include_bytes!`.  The data slices handed to `register_file` are
/// sub-slices of `img` and therefore also `'static`.
pub fn mount_from_image(img: &'static [u8]) {
    use super::block::BLOCK_SIZE;

    if img.len() < BLOCK_SIZE {
        return; // image too small — treat as empty
    }

    for i in 0..DIR_ENTRIES_PER_BLOCK {
        let off = i * DIR_ENTRY_SIZE;
        let mut b = [0u8; DIR_ENTRY_SIZE];
        b.copy_from_slice(&img[off..off + DIR_ENTRY_SIZE]);
        let entry = DirEntry::from_bytes(&b);

        if entry.is_valid() {
            let start = entry.start_block as usize * BLOCK_SIZE;
            let end   = start + entry.size as usize;
            if end <= img.len() {
                super::register_file(entry.name_str(), &img[start..end]);
            }
        }
    }
}
