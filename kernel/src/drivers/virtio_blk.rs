// drivers/virtio_blk.rs — QEMU virtio-blk PCI driver  (Sprint D.1 stretch)
//
// Implements the `BlockDevice` trait for a virtio-blk device discovered via
// PCI.  Uses the *legacy* virtio PCI transport (vendor 0x1AF4 / device 0x1001)
// which QEMU presents when you pass `-drive …,if=virtio`.
//
// All I/O is synchronous (polled); no interrupt handler is installed.
//
// Usage in kernel_main:
//   if let Some(blk) = VirtioBlk::try_init() {
//       fs::wasmfs::mount_from_blk(blk);
//   }
//
// Virtqueue memory layout (QUEUE_ALIGN = 4096, legacy virtio spec):
//
//   offset 0           q × 16 bytes   descriptor table
//   offset q×16 + 0        2 bytes    avail.flags
//   offset q×16 + 2        2 bytes    avail.idx
//   offset q×16 + 4       q × 2 bytes avail.ring[]
//   offset ALIGN(…,4096)   4 bytes    used.flags + used.idx
//   …                     q × 8 bytes used.ring[]
//
// All ring offsets are computed dynamically from the queue depth q that the
// device reports via QUEUE_NUM (read-only in legacy QEMU — writes are silently
// ignored).  QEMU 8+ uses q=256 by default, requiring 3 physically-contiguous
// pages.  We allocate a 6-page buffer so we can always find the required run.

use crate::fs::block::{BlockDevice, BLOCK_SIZE};
use crate::memory::virt_to_phys;
use core::sync::atomic::{fence, Ordering};

// ── Port I/O ─────────────────────────────────────────────────────────────────

#[inline]
unsafe fn outb(port: u16, val: u8) {
    core::arch::asm!("out dx, al",
        in("dx") port, in("al") val,
        options(nomem, nostack, preserves_flags));
}
#[inline]
unsafe fn inb(port: u16) -> u8 {
    let v: u8;
    core::arch::asm!("in al, dx",
        out("al") v, in("dx") port,
        options(nomem, nostack, preserves_flags));
    v
}
#[inline]
unsafe fn outw(port: u16, val: u16) {
    core::arch::asm!("out dx, ax",
        in("dx") port, in("ax") val,
        options(nomem, nostack, preserves_flags));
}
#[inline]
unsafe fn inw(port: u16) -> u16 {
    let v: u16;
    core::arch::asm!("in ax, dx",
        out("ax") v, in("dx") port,
        options(nomem, nostack, preserves_flags));
    v
}
#[inline]
unsafe fn outl(port: u16, val: u32) {
    core::arch::asm!("out dx, eax",
        in("dx") port, in("eax") val,
        options(nomem, nostack, preserves_flags));
}
#[inline]
unsafe fn inl(port: u16) -> u32 {
    let v: u32;
    core::arch::asm!("in eax, dx",
        out("eax") v, in("dx") port,
        options(nomem, nostack, preserves_flags));
    v
}

// ── PCI config-space access ───────────────────────────────────────────────────

const PCI_ADDR: u16 = 0xCF8;
const PCI_DATA: u16 = 0xCFC;

unsafe fn pci_read32(bus: u8, dev: u8, func: u8, off: u8) -> u32 {
    let addr = 0x8000_0000u32
        | ((bus  as u32) << 16)
        | ((dev  as u32) << 11)
        | ((func as u32) <<  8)
        | ((off & 0xFC)  as u32);
    outl(PCI_ADDR, addr);
    inl(PCI_DATA)
}

unsafe fn pci_write32(bus: u8, dev: u8, func: u8, off: u8, val: u32) {
    let addr = 0x8000_0000u32
        | ((bus  as u32) << 16)
        | ((dev  as u32) << 11)
        | ((func as u32) <<  8)
        | ((off & 0xFC)  as u32);
    outl(PCI_ADDR, addr);
    outl(PCI_DATA, val);
}

// ── Virtio legacy PCI register offsets (from BAR0 I/O base) ──────────────────

const VIRT_DEVICE_FEATURES: u16 = 0x00; // u32  R    host features
const VIRT_DRIVER_FEATURES: u16 = 0x04; // u32  W    guest accepted features
const VIRT_QUEUE_PFN:       u16 = 0x08; // u32  R/W  queue page-frame number
const VIRT_QUEUE_NUM:       u16 = 0x0C; // u16  R    queue size (read-only in legacy QEMU)
const VIRT_QUEUE_SELECT:    u16 = 0x0E; // u16  W    select queue index
const VIRT_QUEUE_NOTIFY:    u16 = 0x10; // u16  W    kick the device
const VIRT_DEVICE_STATUS:   u16 = 0x12; // u8   R/W  device lifecycle status
// Virtio-blk device config (legacy, starts at BAR0 + 0x14)
const VIRT_BLK_CAP_LO:      u16 = 0x14; // u32  R    capacity in 512-byte sectors (low 32)
const VIRT_BLK_CAP_HI:      u16 = 0x18; // u32  R    capacity (high 32)

// Device status bits
const STATUS_ACK:       u8 = 0x01;
const STATUS_DRIVER:    u8 = 0x02;
const STATUS_DRIVER_OK: u8 = 0x04;
const STATUS_FAILED:    u8 = 0x80;

// Descriptor flags
const VRING_DESC_F_NEXT:  u16 = 0x0001;
const VRING_DESC_F_WRITE: u16 = 0x0002; // device writes into this buffer

// Block request types
const BLK_T_IN:  u32 = 0; // read from device
const BLK_T_OUT: u32 = 1; // write to device

// Status byte values written by the device after completing a request
const BLK_S_OK: u8 = 0;

// ── Virtqueue layout helpers ──────────────────────────────────────────────────
//
// All offsets are relative to the start of the virtqueue buffer (vq_base).
// Legacy virtio spec, QUEUE_ALIGN = 4096:
//
//   offset 0              descriptor table  (q * 16 bytes)
//   offset q*16           avail.flags (u16)
//   offset q*16 + 2       avail.idx   (u16)
//   offset q*16 + 4       avail.ring  (q × u16)
//   offset ALIGN(…, 4096) used ring   (used.flags, used.idx, used.ring[])
//
// We compute these at runtime from the queue size the device actually reports,
// because legacy QEMU ignores writes to the QUEUE_NUM register and keeps its
// own default (typically 128).  Using the wrong Q would place our avail.idx
// write at the wrong offset and the device would never see new requests.

fn vq_avail_idx_off(q: usize)  -> usize { q * 16 + 2 }
fn vq_avail_ring_off(q: usize) -> usize { q * 16 + 4 }
fn vq_used_base(q: usize)      -> usize {
    let avail_end = q * 16 + 4 + q * 2 + 2; // desc + avail (with used_event)
    (avail_end + 4095) & !4095               // round up to QUEUE_ALIGN
}
fn vq_used_idx_off(q: usize)   -> usize { vq_used_base(q) + 2 }
/// Total bytes required for a virtqueue of depth `q`.
fn vq_total_bytes(q: usize)    -> usize { vq_used_base(q) + 4 + q * 8 + 2 }
/// Number of 4096-byte pages required for a virtqueue of depth `q`.
fn vq_pages_needed(q: usize)   -> usize { (vq_total_bytes(q) + 4095) / 4096 }

/// Raw storage for one virtqueue.  Six pages so we can always find a run of
/// up to three physically-contiguous pages (needed for q=256) even if the BSS
/// frame allocator gives non-contiguous frames.
#[repr(C, align(4096))]
struct VirtqBuf([u8; 4096 * 6]);

static mut VIRTQ_BUF: VirtqBuf = VirtqBuf([0u8; 4096 * 6]);

// ── Request buffers (one request in flight at a time) ─────────────────────────

/// Virtio-blk request header (16 bytes, read by device).
#[repr(C)]
struct BlkReqHdr {
    req_type: u32, // BLK_T_IN / BLK_T_OUT
    reserved: u32,
    sector:   u64,
}

static mut REQ_HDR:    BlkReqHdr = BlkReqHdr { req_type: 0, reserved: 0, sector: 0 };
static mut REQ_DATA:   [u8; BLOCK_SIZE] = [0u8; BLOCK_SIZE];
static mut REQ_STATUS: u8 = 0xFF;

// ── Virtqueue volatile accessors ─────────────────────────────────────────────
// All take an explicit `base: *mut u8` so they work with whichever contiguous
// page run was selected at init time.

unsafe fn q_write64(base: *mut u8, off: usize, val: u64) {
    core::ptr::write_volatile(base.add(off) as *mut u64, val.to_le());
}
unsafe fn q_write32(base: *mut u8, off: usize, val: u32) {
    core::ptr::write_volatile(base.add(off) as *mut u32, val.to_le());
}
unsafe fn q_write16(base: *mut u8, off: usize, val: u16) {
    core::ptr::write_volatile(base.add(off) as *mut u16, val.to_le());
}
unsafe fn q_read16(base: *const u8, off: usize) -> u16 {
    u16::from_le(core::ptr::read_volatile(base.add(off) as *const u16))
}

/// Write a virtqueue descriptor at slot `idx`.
unsafe fn write_desc(base: *mut u8, idx: usize, addr: u64, len: u32, flags: u16, next: u16) {
    let off = idx * 16; // descriptor table starts at offset 0
    q_write64(base, off,      addr);
    q_write32(base, off + 8,  len);
    q_write16(base, off + 12, flags);
    q_write16(base, off + 14, next);
}

// ── VirtioBlk ─────────────────────────────────────────────────────────────────

/// Handle to an initialized virtio-blk device.
pub struct VirtioBlk {
    io_base:        u16,
    last_used_idx:  u16,
    /// Virtual base address of the physically-contiguous virtqueue pages.
    vq_base:        *mut u8,
    /// Byte offsets within vq_base, computed from the device's actual queue size.
    avail_idx_off:  usize,
    avail_ring_off: usize,
    used_idx_off:   usize,
    q_size:         usize,
}

// Single-core bare-metal: no concurrent access possible.
unsafe impl Send for VirtioBlk {}

impl VirtioBlk {
    /// Scan PCI for a virtio-blk device (vendor 0x1AF4 / device 0x1001),
    /// initialise the legacy virtqueue, and return a ready handle.
    /// Returns `None` if no device is found or initialisation fails.
    pub fn try_init() -> Option<Self> {
        let (bus, dev, io_base) = find_virtio_blk()?;

        // We need to know q_size before scanning pages, so read it now.
        // QUEUE_NUM is read-only in legacy QEMU; we must use whatever it reports.
        unsafe { outw(io_base + VIRT_QUEUE_SELECT, 0); }
        let q_size = unsafe { inw(io_base + VIRT_QUEUE_NUM) } as usize;
        if q_size == 0 {
            return None;
        }
        let pages_needed = vq_pages_needed(q_size);

        // Scan our 6-page buffer for `pages_needed` physically-contiguous pages.
        let buf_virt = { core::ptr::addr_of!(VIRTQ_BUF) as usize };
        let mut q_virt: Option<usize> = None;
        'outer: for i in 0..=(6 - pages_needed) {
            for j in 0..pages_needed - 1 {
                let p0 = virt_to_phys(buf_virt + (i + j) * 4096);
                let p1 = virt_to_phys(buf_virt + (i + j + 1) * 4096);
                if p1 != p0 + 4096 {
                    continue 'outer;
                }
            }
            q_virt = Some(buf_virt + i * 4096);
            break;
        }
        let vq_virt = match q_virt {
            Some(v) => v,
            None => {
                crate::println!("[virtio] FATAL: no {} contiguous physical pages found", pages_needed);
                return None;
            }
        };

        unsafe {
            // Zero the virtqueue pages we're using.
            core::ptr::write_bytes(vq_virt as *mut u8, 0, pages_needed * 4096);

            // 1. Enable I/O space + bus-mastering in PCI command register.
            let cmd = pci_read32(bus, dev, 0, 0x04);
            pci_write32(bus, dev, 0, 0x04, cmd | 0x0005);

            // 2. Reset the device (write 0 to status clears all state).
            outb(io_base + VIRT_DEVICE_STATUS, 0);

            // 3. Acknowledge: we see the device, we have a driver.
            outb(io_base + VIRT_DEVICE_STATUS, STATUS_ACK);
            outb(io_base + VIRT_DEVICE_STATUS, STATUS_ACK | STATUS_DRIVER);

            // 4. Feature negotiation: accept no optional features.
            let _host_feat = inl(io_base + VIRT_DEVICE_FEATURES);
            outl(io_base + VIRT_DRIVER_FEATURES, 0);

            // 5. Re-select queue 0 (reset cleared device state; q_size already
            //    known from pre-init read — QUEUE_NUM is read-only in legacy QEMU).
            outw(io_base + VIRT_QUEUE_SELECT, 0);

            // 6. Hand the physical page frame to the device.
            let q_phys = virt_to_phys(vq_virt);
            outl(io_base + VIRT_QUEUE_PFN, (q_phys >> 12) as u32);

            // 7. Signal that the driver is fully initialised.
            outb(io_base + VIRT_DEVICE_STATUS,
                STATUS_ACK | STATUS_DRIVER | STATUS_DRIVER_OK);

            if inb(io_base + VIRT_DEVICE_STATUS) & STATUS_FAILED != 0 {
                return None;
            }

            Some(VirtioBlk {
                io_base,
                last_used_idx:  0,
                vq_base:        vq_virt as *mut u8,
                avail_idx_off:  vq_avail_idx_off(q_size),
                avail_ring_off: vq_avail_ring_off(q_size),
                used_idx_off:   vq_used_idx_off(q_size),
                q_size,
            })
        }
    }

    /// Submit a 3-descriptor request (header → data → status) and busy-wait
    /// for the device to complete it.
    ///
    /// `data_write` — `true` if the device should write into the data buffer
    ///                (i.e. this is a read-from-disk request).
    unsafe fn do_request(&mut self, hdr_phys: u64, data_phys: u64, data_write: bool)
        -> Result<(), ()>
    {
        let base           = self.vq_base;
        let avail_idx_off  = self.avail_idx_off;
        let avail_ring_off = self.avail_ring_off;
        let used_idx_off   = self.used_idx_off;
        let q_mask         = (self.q_size - 1) as u16;

        let status_phys = virt_to_phys(core::ptr::addr_of!(REQ_STATUS) as usize);
        let avail_idx   = q_read16(base, avail_idx_off);

        // Descriptor 0: request header (device reads this)
        write_desc(base, 0, hdr_phys, 16, VRING_DESC_F_NEXT, 1);

        // Descriptor 1: data block
        //   READ  → device writes → VRING_DESC_F_WRITE
        //   WRITE → device reads → no write flag
        let data_flags = VRING_DESC_F_NEXT
            | if data_write { VRING_DESC_F_WRITE } else { 0 };
        write_desc(base, 1, data_phys, BLOCK_SIZE as u32, data_flags, 2);

        // Descriptor 2: status byte (device always writes this)
        write_desc(base, 2, status_phys, 1, VRING_DESC_F_WRITE, 0);

        // Reset status sentinel before submitting.
        core::ptr::write_volatile(core::ptr::addr_of_mut!(REQ_STATUS), 0xFF);

        // Place descriptor-chain head (index 0) in the available ring.
        let slot = (avail_idx & q_mask) as usize;
        q_write16(base, avail_ring_off + slot * 2, 0);

        // Full memory barrier: descriptors and avail.ring[] must be visible
        // to the device before we advance avail.idx.
        fence(Ordering::SeqCst);

        q_write16(base, avail_idx_off, avail_idx.wrapping_add(1));

        // Kick the device: write queue index 0 to the notify register.
        outw(self.io_base + VIRT_QUEUE_NOTIFY, 0);

        // Busy-wait until used.idx advances.
        let target = self.last_used_idx.wrapping_add(1);
        let mut spins: u64 = 0;
        loop {
            fence(Ordering::SeqCst);
            if q_read16(base, used_idx_off) == target {
                break;
            }
            spins = spins.wrapping_add(1);
            if spins == 100_000_000 {
                let ui  = q_read16(base, used_idx_off);
                let st  = core::ptr::read_volatile(core::ptr::addr_of!(REQ_STATUS));
                let dev = inb(self.io_base + VIRT_DEVICE_STATUS);
                crate::println!("[virtio] TIMEOUT used_idx={} target={} req_status=0x{:x} dev_status=0x{:x}",
                    ui, target, st, dev);
                return Err(());
            }
            core::hint::spin_loop();
        }
        self.last_used_idx = target;

        if core::ptr::read_volatile(core::ptr::addr_of!(REQ_STATUS)) != BLK_S_OK {
            crate::println!("[virtio] bad req_status=0x{:x}",
                core::ptr::read_volatile(core::ptr::addr_of!(REQ_STATUS)));
            return Err(());
        }
        Ok(())
    }
}

impl BlockDevice for VirtioBlk {
    fn read_block(&mut self, lba: u32, buf: &mut [u8; BLOCK_SIZE]) -> Result<(), ()> {
        // SAFETY: single-core bare-metal — no concurrent access possible.
        unsafe {
            REQ_HDR.req_type = BLK_T_IN;
            REQ_HDR.reserved = 0;
            REQ_HDR.sector   = lba as u64;

            let hdr_phys  = virt_to_phys(core::ptr::addr_of!(REQ_HDR)  as usize);
            let data_phys = virt_to_phys(core::ptr::addr_of!(REQ_DATA) as usize);

            self.do_request(hdr_phys, data_phys, true)?;  // device writes data

            buf.copy_from_slice(&REQ_DATA);
        }
        Ok(())
    }

    fn write_block(&mut self, lba: u32, buf: &[u8; BLOCK_SIZE]) -> Result<(), ()> {
        // SAFETY: single-core bare-metal — no concurrent access possible.
        unsafe {
            REQ_HDR.req_type = BLK_T_OUT;
            REQ_HDR.reserved = 0;
            REQ_HDR.sector   = lba as u64;

            REQ_DATA.copy_from_slice(buf);

            let hdr_phys  = virt_to_phys(core::ptr::addr_of!(REQ_HDR)  as usize);
            let data_phys = virt_to_phys(core::ptr::addr_of!(REQ_DATA) as usize);

            self.do_request(hdr_phys, data_phys, false)?; // device reads data
        }
        Ok(())
    }

    fn block_count(&self) -> u32 {
        // Virtio-blk device config: capacity (u64) in 512-byte sectors.
        // Located at BAR0 + 0x14 (legacy transport, device-specific config).
        let lo = unsafe { inl(self.io_base + VIRT_BLK_CAP_LO) };
        let hi = unsafe { inl(self.io_base + VIRT_BLK_CAP_HI) };
        let sectors = ((hi as u64) << 32) | (lo as u64);
        sectors.min(u32::MAX as u64) as u32
    }
}

// ── PCI scan ─────────────────────────────────────────────────────────────────

/// Scan all PCI buses/devices for the first virtio-blk device
/// (vendor 0x1AF4, device 0x1001) and return its (bus, dev, io_base).
fn find_virtio_blk() -> Option<(u8, u8, u16)> {
    for bus in 0u8..=255 {
        for dev in 0u8..32 {
            let id = unsafe { pci_read32(bus, dev, 0, 0x00) };
            if id == 0xFFFF_FFFF {
                continue; // slot empty
            }
            let vendor = (id & 0x0000_FFFF) as u16;
            let device = (id >> 16)          as u16;

            if vendor == 0x1AF4 && device == 0x1001 {
                let bar0 = unsafe { pci_read32(bus, dev, 0, 0x10) };
                if bar0 & 1 == 1 {
                    // I/O-space BAR: base address = bar0[31:2] with bit 0 cleared
                    return Some((bus, dev, (bar0 & !3) as u16));
                }
            }
        }
    }
    None
}
