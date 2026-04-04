// fs/block.rs — Block device abstraction (Sprint D.1)
//
// Defines the BlockDevice trait and a Ramdisk implementation backed by a
// static byte array (no heap, no_std).
//
// Block size: 512 bytes (standard sector size).
// Ramdisk:    256 blocks = 128 KiB — enough to hold several small WASM modules.

pub const BLOCK_SIZE: usize = 512;
pub const RAMDISK_BLOCKS: usize = 256; // 128 KiB

// ---------------------------------------------------------------------------
// Trait
// ---------------------------------------------------------------------------

pub trait BlockDevice {
    /// Read one 512-byte block at logical block address `lba` into `buf`.
    /// Returns `Err(())` if `lba` is out of range.
    fn read_block(&mut self, lba: u32, buf: &mut [u8; BLOCK_SIZE]) -> Result<(), ()>;

    /// Write one 512-byte block from `buf` to logical block address `lba`.
    /// Returns `Err(())` if `lba` is out of range.
    fn write_block(&mut self, lba: u32, buf: &[u8; BLOCK_SIZE]) -> Result<(), ()>;

    /// Total number of blocks on this device.
    fn block_count(&self) -> u32;
}

// ---------------------------------------------------------------------------
// Ramdisk
// ---------------------------------------------------------------------------

static mut RAMDISK_DATA: [u8; BLOCK_SIZE * RAMDISK_BLOCKS] = [0u8; BLOCK_SIZE * RAMDISK_BLOCKS];

/// A zero-sized handle to the static ramdisk buffer.
/// Only one instance should be created; use `Ramdisk::get()`.
pub struct Ramdisk;

impl Ramdisk {
    /// Return the singleton ramdisk handle.
    ///
    /// Safety: single-core bare-metal — no concurrent access possible.
    pub fn get() -> Self {
        Ramdisk
    }
}

impl BlockDevice for Ramdisk {
    fn read_block(&mut self, lba: u32, buf: &mut [u8; BLOCK_SIZE]) -> Result<(), ()> {
        if lba as usize >= RAMDISK_BLOCKS {
            return Err(());
        }
        let offset = lba as usize * BLOCK_SIZE;
        unsafe {
            buf.copy_from_slice(&RAMDISK_DATA[offset..offset + BLOCK_SIZE]);
        }
        Ok(())
    }

    fn write_block(&mut self, lba: u32, buf: &[u8; BLOCK_SIZE]) -> Result<(), ()> {
        if lba as usize >= RAMDISK_BLOCKS {
            return Err(());
        }
        let offset = lba as usize * BLOCK_SIZE;
        unsafe {
            RAMDISK_DATA[offset..offset + BLOCK_SIZE].copy_from_slice(buf);
        }
        Ok(())
    }

    fn block_count(&self) -> u32 {
        RAMDISK_BLOCKS as u32
    }
}

// ---------------------------------------------------------------------------
// EmbeddedDisk — read-only BlockDevice over a &'static [u8]
// ---------------------------------------------------------------------------
//
// Wraps a byte slice that was embedded at compile time (e.g. via
// include_bytes!).  Writes are rejected (Err(())).  Used by mount_from_image
// to let WasmFs<EmbeddedDisk> read the boot filesystem image without needing
// a real disk driver.

pub struct EmbeddedDisk {
    data: &'static [u8],
}

impl EmbeddedDisk {
    pub fn new(data: &'static [u8]) -> Self {
        EmbeddedDisk { data }
    }
}

impl BlockDevice for EmbeddedDisk {
    fn read_block(&mut self, lba: u32, buf: &mut [u8; BLOCK_SIZE]) -> Result<(), ()> {
        let offset = lba as usize * BLOCK_SIZE;
        if offset + BLOCK_SIZE > self.data.len() {
            return Err(());
        }
        buf.copy_from_slice(&self.data[offset..offset + BLOCK_SIZE]);
        Ok(())
    }

    fn write_block(&mut self, _lba: u32, _buf: &[u8; BLOCK_SIZE]) -> Result<(), ()> {
        Err(()) // read-only
    }

    fn block_count(&self) -> u32 {
        (self.data.len() / BLOCK_SIZE) as u32
    }
}
