//! FAT12/16/32 filesystem driver (via `rust-fatfs`, `no_std` + `alloc`).
//!
//! Provides a [`BlockIo`] adapter that wraps a [`block::BlockDevice`] into
//! the byte-level `Read`/`Write`/`Seek` interface required by `fatfs`.
//!
//! A single global [`FileSystem`] handle is protected by a [`spin::Mutex`]
//! under the [`MountedFs`] enum.  Mount with [`mount_virtio`] or
//! [`mount_ramdisk`]; then use the `fat_*` functions for all I/O.
//!
//! All `fatfs` re-exports (`Read`, `Write`, `Seek`, `SeekFrom`, `IoBase`)
//! are taken from the crate root as per the `fatfs 0.4+` API.

use alloc::vec::Vec;
use alloc::string::String;
use spin::Mutex;

use super::block::{BlockDevice, BLOCK_SIZE};

use fatfs::IoBase;
use fatfs::Read  as FatRead;
use fatfs::Write as FatWrite;
use fatfs::Seek  as FatSeek;
use fatfs::SeekFrom;
use fatfs::{FileSystem, FsOptions};

// ── BlockIo adapter ──────────────────────────────────────────────────────────

/// Byte-level I/O adapter over a [`block::BlockDevice`] for use with `fatfs`.
///
/// Implements a one-block write-back cache: the current block is held in
/// `buf`; a dirty flag tracks whether it needs to be flushed.  The cache is
/// written back before the cursor crosses a block boundary and on an explicit
/// [`fatfs::Write::flush`] call.
pub struct BlockIo<D: BlockDevice> {
    dev:       D,
    pos:       u64,
    buf:       [u8; BLOCK_SIZE],
    buf_lba:   Option<u32>,
    buf_dirty: bool,
}

impl<D: BlockDevice> BlockIo<D> {
    pub fn new(dev: D) -> Self {
        BlockIo { dev, pos: 0, buf: [0u8; BLOCK_SIZE], buf_lba: None, buf_dirty: false }
    }

    fn flush_dirty(&mut self) -> Result<(), ()> {
        if self.buf_dirty {
            if let Some(lba) = self.buf_lba {
                self.dev.write_block(lba, &self.buf)?;
                self.buf_dirty = false;
            }
        }
        Ok(())
    }

    fn ensure_block(&mut self, lba: u32) -> Result<(), ()> {
        if self.buf_lba == Some(lba) {
            return Ok(());
        }
        self.flush_dirty()?;
        self.dev.read_block(lba, &mut self.buf)?;
        self.buf_lba = Some(lba);
        Ok(())
    }
}

// IoBase declares the associated error type for all three IO traits.
impl<D: BlockDevice> IoBase for BlockIo<D> {
    type Error = ();
}

impl<D: BlockDevice> FatRead for BlockIo<D> {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, ()> {
        let mut done = 0;
        while done < buf.len() {
            let lba = (self.pos / BLOCK_SIZE as u64) as u32;
            let off = (self.pos % BLOCK_SIZE as u64) as usize;
            self.ensure_block(lba)?;
            let avail = (BLOCK_SIZE - off).min(buf.len() - done);
            buf[done..done + avail].copy_from_slice(&self.buf[off..off + avail]);
            done     += avail;
            self.pos += avail as u64;
        }
        Ok(done)
    }
}

impl<D: BlockDevice> FatWrite for BlockIo<D> {
    fn write(&mut self, buf: &[u8]) -> Result<usize, ()> {
        let mut done = 0;
        while done < buf.len() {
            let lba = (self.pos / BLOCK_SIZE as u64) as u32;
            let off = (self.pos % BLOCK_SIZE as u64) as usize;
            // Partial-sector write: read existing data first (read-modify-write).
            // Full-sector write: skip the read.
            if off != 0 || (buf.len() - done) < BLOCK_SIZE {
                self.ensure_block(lba)?;
            } else {
                self.flush_dirty()?;
                self.buf_lba   = Some(lba);
                self.buf       = [0u8; BLOCK_SIZE];
            }
            let avail = (BLOCK_SIZE - off).min(buf.len() - done);
            self.buf[off..off + avail].copy_from_slice(&buf[done..done + avail]);
            self.buf_dirty  = true;
            done            += avail;
            self.pos        += avail as u64;
        }
        Ok(done)
    }

    fn flush(&mut self) -> Result<(), ()> {
        self.flush_dirty()
    }
}

impl<D: BlockDevice> FatSeek for BlockIo<D> {
    fn seek(&mut self, from: SeekFrom) -> Result<u64, ()> {
        let new_pos: i64 = match from {
            SeekFrom::Start(n)   => n as i64,
            SeekFrom::Current(n) => self.pos as i64 + n,
            SeekFrom::End(_)     => return Err(()),
        };
        if new_pos < 0 { return Err(()); }
        self.pos = new_pos as u64;
        Ok(self.pos)
    }
}

// ── In-memory ramdisk for the embedded FAT image fallback ────────────────────

pub const RAM_FAT_SIZE: usize = 2 * 1024 * 1024; // 2 MiB
const RAM_BLOCKS: usize = RAM_FAT_SIZE / BLOCK_SIZE;

#[repr(C, align(512))]
struct RamFatBuf([u8; RAM_FAT_SIZE]);
static mut RAM_FAT: RamFatBuf = RamFatBuf([0u8; RAM_FAT_SIZE]);

pub struct RamDisk;
impl RamDisk { fn get() -> Self { RamDisk } }

impl BlockDevice for RamDisk {
    fn read_block(&mut self, lba: u32, buf: &mut [u8; BLOCK_SIZE]) -> Result<(), ()> {
        if lba as usize >= RAM_BLOCKS { return Err(()); }
        let off = lba as usize * BLOCK_SIZE;
        unsafe { buf.copy_from_slice(&RAM_FAT.0[off..off + BLOCK_SIZE]); }
        Ok(())
    }
    fn write_block(&mut self, lba: u32, buf: &[u8; BLOCK_SIZE]) -> Result<(), ()> {
        if lba as usize >= RAM_BLOCKS { return Err(()); }
        let off = lba as usize * BLOCK_SIZE;
        unsafe { RAM_FAT.0[off..off + BLOCK_SIZE].copy_from_slice(buf); }
        Ok(())
    }
    fn block_count(&self) -> u32 { RAM_BLOCKS as u32 }
}

// ── Global filesystem handle ──────────────────────────────────────────────────

use crate::drivers::virtio_blk::VirtioBlk;

pub enum MountedFs {
    Ram(FileSystem<BlockIo<RamDisk>>),
    Virtio(FileSystem<BlockIo<VirtioBlk>>),
}

static FS: Mutex<Option<MountedFs>> = Mutex::new(None);

// ── Mount helpers ─────────────────────────────────────────────────────────────

/// Mount a FAT volume from a virtio-blk device.
///
/// Returns `true` on success; `false` if the device does not contain a
/// recognisable FAT volume.
pub fn mount_virtio(blk: VirtioBlk) -> bool {
    match FileSystem::new(BlockIo::new(blk), FsOptions::new()) {
        Ok(fs) => { *FS.lock() = Some(MountedFs::Virtio(fs)); true }
        Err(_) => false,
    }
}

/// Mount a FAT volume from an in-memory image.
///
/// Copies `img` into the 2 MiB [`RAM_FAT_SIZE`] static buffer, then opens
/// a FAT filesystem over it.  Returns `true` on success.
pub fn mount_ramdisk(img: &[u8]) -> bool {
    let copy_len = img.len().min(RAM_FAT_SIZE);
    unsafe {
        RAM_FAT.0[..copy_len].copy_from_slice(&img[..copy_len]);
        for b in &mut RAM_FAT.0[copy_len..] { *b = 0; }
    }
    match FileSystem::new(BlockIo::new(RamDisk::get()), FsOptions::new()) {
        Ok(fs) => { *FS.lock() = Some(MountedFs::Ram(fs)); true }
        Err(_) => false,
    }
}

// ── FAT operations ───────────────────────────────────────────────────────────

/// List all files in the root directory.
///
/// Calls `cb(name, size_bytes)` for each file entry (directories are skipped).
/// Does nothing if no filesystem is mounted.
pub fn fat_list<F: FnMut(&str, u32)>(mut cb: F) {
    let mut guard = FS.lock();
    match guard.as_mut() {
        None => {}
        Some(MountedFs::Ram(fs))    => do_list(fs, &mut cb),
        Some(MountedFs::Virtio(fs)) => do_list(fs, &mut cb),
    }
}

fn do_list<IO>(fs: &mut FileSystem<IO>, cb: &mut dyn FnMut(&str, u32))
where IO: IoBase<Error=()> + FatRead + FatWrite + FatSeek
{
    for entry in fs.root_dir().iter() {
        if let Ok(e) = entry {
            if e.is_file() {
                let name: String = e.file_name();
                cb(name.as_str(), e.len() as u32);
            }
        }
    }
}

/// Read a file from the root directory into a freshly allocated `Vec<u8>`.
///
/// Returns `None` if the file is not found or the filesystem is not mounted.
pub fn fat_read_file(name: &str) -> Option<Vec<u8>> {
    let mut guard = FS.lock();
    match guard.as_mut() {
        None => None,
        Some(MountedFs::Ram(fs))    => do_read(fs, name),
        Some(MountedFs::Virtio(fs)) => do_read(fs, name),
    }
}

fn do_read<IO>(fs: &mut FileSystem<IO>, name: &str) -> Option<Vec<u8>>
where IO: IoBase<Error=()> + FatRead + FatWrite + FatSeek
{
    let root = fs.root_dir();
    let mut file = root.open_file(name).ok()?;
    // Get size from the directory entry first to pre-allocate.
    let size = {
        let end = file.seek(SeekFrom::End(0)).ok()? as usize;
        file.seek(SeekFrom::Start(0)).ok()?;
        end
    };
    let mut buf = alloc::vec![0u8; size];
    let mut done = 0;
    while done < size {
        match file.read(&mut buf[done..]) {
            Ok(0) => break,
            Ok(n) => done += n,
            Err(_) => return None,
        }
    }
    buf.truncate(done);
    Some(buf)
}

/// Write (or overwrite) a file in the root directory.
///
/// Creates the file if it does not exist; truncates it before writing if it
/// does.  Returns `true` on success, `false` on any I/O error or if no
/// filesystem is mounted.
pub fn fat_write_file(name: &str, data: &[u8]) -> bool {
    let mut guard = FS.lock();
    match guard.as_mut() {
        None => false,
        Some(MountedFs::Ram(fs))    => do_write(fs, name, data),
        Some(MountedFs::Virtio(fs)) => do_write(fs, name, data),
    }
}

fn do_write<IO>(fs: &mut FileSystem<IO>, name: &str, data: &[u8]) -> bool
where IO: IoBase<Error=()> + FatRead + FatWrite + FatSeek
{
    let root = fs.root_dir();
    let mut file = match root.create_file(name) {
        Ok(f) => f,
        Err(_) => return false,
    };
    if file.truncate().is_err() { return false; }
    let mut written = 0;
    while written < data.len() {
        match file.write(&data[written..]) {
            Ok(n) if n > 0 => written += n,
            _ => return false,
        }
    }
    file.flush().is_ok()
}

/// Remove a file from the root directory.
///
/// Returns `true` if the file was found and removed, `false` otherwise.
pub fn fat_remove_file(name: &str) -> bool {
    let mut guard = FS.lock();
    match guard.as_mut() {
        None => false,
        Some(MountedFs::Ram(fs))    => fs.root_dir().remove(name).is_ok(),
        Some(MountedFs::Virtio(fs)) => fs.root_dir().remove(name).is_ok(),
    }
}

// ── Path helper ──────────────────────────────────────────────────────────────

/// Split "dir/file" or "/dir/file" into ("dir", "file"); bare "file" → ("", "file").
fn split_path(path: &str) -> (&str, &str) {
    let p = path.trim_start_matches('/');
    match p.rfind('/') {
        Some(pos) => (&p[..pos], &p[pos + 1..]),
        None      => ("", p),
    }
}

// ── Disk statistics ──────────────────────────────────────────────────────────

/// Return `(total_bytes, free_bytes)` from FAT volume metadata.
///
/// Returns `None` if no filesystem is mounted or the stats call fails.
pub fn fat_disk_stats() -> Option<(u64, u64)> {
    let mut guard = FS.lock();
    match guard.as_mut() {
        None                        => None,
        Some(MountedFs::Ram(fs))    => do_stats(fs),
        Some(MountedFs::Virtio(fs)) => do_stats(fs),
    }
}

fn do_stats<IO>(fs: &FileSystem<IO>) -> Option<(u64, u64)>
where IO: IoBase<Error=()> + FatRead + FatWrite + FatSeek
{
    let s = fs.stats().ok()?;
    let c = s.cluster_size() as u64;
    Some((s.total_clusters() as u64 * c, s.free_clusters() as u64 * c))
}

// ── Directory helpers ────────────────────────────────────────────────────────

/// Return `true` if `path` refers to a directory in the FAT volume.
///
/// Leading `/` is stripped before lookup.  The empty string and `"/"` always
/// return `true` (root directory).
pub fn fat_is_dir(path: &str) -> bool {
    let name = path.trim_start_matches('/');
    if name.is_empty() { return true; }
    let mut guard = FS.lock();
    match guard.as_mut() {
        None                        => false,
        Some(MountedFs::Ram(fs))    => fs.root_dir().open_dir(name).is_ok(),
        Some(MountedFs::Virtio(fs)) => fs.root_dir().open_dir(name).is_ok(),
    }
}

/// Create a directory with `name` inside the root directory.
///
/// Leading `/` is stripped.  Returns `true` on success.
pub fn fat_mkdir(name: &str) -> bool {
    let name = name.trim_start_matches('/');
    if name.is_empty() { return false; }
    let mut guard = FS.lock();
    match guard.as_mut() {
        None                        => false,
        Some(MountedFs::Ram(fs))    => fs.root_dir().create_dir(name).is_ok(),
        Some(MountedFs::Virtio(fs)) => fs.root_dir().create_dir(name).is_ok(),
    }
}

// ── Path-aware listing ───────────────────────────────────────────────────────

/// List entries in the directory at `path`.
///
/// Leading `/` is stripped internally.  Calls `cb(name, size_bytes, is_dir)`
/// for each entry, skipping `.` and `..`.  If `path` is empty or `"/"`, lists
/// the root directory.
pub fn fat_list_path<F: FnMut(&str, u32, bool)>(path: &str, mut cb: F) {
    let mut guard = FS.lock();
    match guard.as_mut() {
        None                        => {}
        Some(MountedFs::Ram(fs))    => do_list_path(fs, path, &mut cb),
        Some(MountedFs::Virtio(fs)) => do_list_path(fs, path, &mut cb),
    }
}

fn do_list_path<IO>(fs: &mut FileSystem<IO>, path: &str, cb: &mut dyn FnMut(&str, u32, bool))
where IO: IoBase<Error=()> + FatRead + FatWrite + FatSeek
{
    let dir_name = path.trim_start_matches('/');
    if dir_name.is_empty() {
        for entry in fs.root_dir().iter() {
            if let Ok(e) = entry {
                let n: String = e.file_name();
                if n == "." || n == ".." { continue; }
                let d = e.is_dir();
                cb(n.as_str(), if d { 0 } else { e.len() as u32 }, d);
            }
        }
    } else {
        let root = fs.root_dir();
        if let Ok(subdir) = root.open_dir(dir_name) {
            for entry in subdir.iter() {
                if let Ok(e) = entry {
                    let n: String = e.file_name();
                    if n == "." || n == ".." { continue; }
                    let d = e.is_dir();
                    cb(n.as_str(), if d { 0 } else { e.len() as u32 }, d);
                }
            }
        }
    }
}

// ── Path-aware file read ─────────────────────────────────────────────────────

/// Read a file at a path (e.g. `"file.txt"` or `"subdir/file.txt"`).
///
/// Leading `/` is stripped.  Returns `None` if not found or the filesystem is
/// not mounted.
pub fn fat_read_path(path: &str) -> Option<Vec<u8>> {
    let mut guard = FS.lock();
    match guard.as_mut() {
        None                        => None,
        Some(MountedFs::Ram(fs))    => do_read_path(fs, path),
        Some(MountedFs::Virtio(fs)) => do_read_path(fs, path),
    }
}

fn do_read_path<IO>(fs: &mut FileSystem<IO>, path: &str) -> Option<Vec<u8>>
where IO: IoBase<Error=()> + FatRead + FatWrite + FatSeek
{
    let (dir_part, file_part) = split_path(path);
    // Inline read to keep File<'_, IO> lifetime within this scope.
    macro_rules! read_file {
        ($file_expr:expr) => {{
            let mut f = $file_expr;
            let size = f.seek(SeekFrom::End(0)).ok()? as usize;
            f.seek(SeekFrom::Start(0)).ok()?;
            let mut buf = alloc::vec![0u8; size];
            let mut done = 0;
            while done < size {
                match f.read(&mut buf[done..]) {
                    Ok(0) => break,
                    Ok(n) => done += n,
                    Err(_) => return None,
                }
            }
            buf.truncate(done);
            Some(buf)
        }};
    }
    if dir_part.is_empty() {
        let root = fs.root_dir();
        read_file!(root.open_file(file_part).ok()?)
    } else {
        let root = fs.root_dir();
        let subdir = root.open_dir(dir_part).ok()?;
        read_file!(subdir.open_file(file_part).ok()?)
    }
}
