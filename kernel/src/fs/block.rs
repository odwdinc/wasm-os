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
    /// Not yet called within the kernel; reserved for virtio-blk and tooling.
    #[allow(dead_code)]
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
