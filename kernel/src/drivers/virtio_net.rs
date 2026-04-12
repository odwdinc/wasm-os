// drivers/virtio_net.rs — QEMU virtio-net PCI driver (Sprint E.1)
//
// Implements raw Ethernet frame send/receive for a virtio-net device discovered
// via PCI.  Uses the *legacy* virtio PCI transport (vendor 0x1AF4 / device 0x1000)
// which QEMU presents when you pass `-device virtio-net-pci,netdev=...`.
//
// Virtio-net queue assignment (per spec):
//   Queue 0 — receiveq  (device writes incoming frames into driver-supplied buffers)
//   Queue 1 — transmitq (driver posts outgoing frames here)
//
// Each virtio-net packet is prefixed by a 10-byte virtio_net_hdr (legacy
// transport, no VIRTIO_NET_F_MRG_RXBUF).  For TX the header is posted as a
// separate read-only descriptor ahead of the frame data.  For RX the header
// occupies the first NET_HDR_SIZE bytes of the receive buffer and is stripped
// before handing the frame to the caller.
//
// One RX buffer is pre-posted during initialisation and recycled after every
// completed receive.  TX blocks (spin-polls) until the device signals completion.

use crate::memory::virt_to_phys;
use core::sync::atomic::{fence, Ordering};

// ——— Port I/O —————————————————————————————————————————————————————————————————

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

// ——— PCI config-space access ——————————————────────────────────────────————————

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

// ——— Virtio legacy PCI register offsets ——————————————————————————————————————

const VIRT_DEVICE_FEATURES: u16 = 0x00; // u32  R    host feature bits
const VIRT_DRIVER_FEATURES: u16 = 0x04; // u32  W    guest accepted features
const VIRT_QUEUE_PFN:       u16 = 0x08; // u32  R/W  queue page-frame number
const VIRT_QUEUE_NUM:       u16 = 0x0C; // u16  R    queue size (read-only in legacy QEMU)
const VIRT_QUEUE_SELECT:    u16 = 0x0E; // u16  W    select queue index
const VIRT_QUEUE_NOTIFY:    u16 = 0x10; // u16  W    kick the device
const VIRT_DEVICE_STATUS:   u16 = 0x12; // u8   R/W  device lifecycle status
// Virtio-net device config (legacy, starts at BAR0 + 0x14)
const VIRT_NET_MAC:         u16 = 0x14; // 6 bytes   MAC address
const VIRT_NET_STATUS:      u16 = 0x1A; // u16       link status

// Virtio-net virtqueue indices (per spec)
const RXQ: u16 = 0; // receiveq  — device fills these buffers with incoming frames
const TXQ: u16 = 1; // transmitq — driver posts outgoing frames here

// Device status bits
const STATUS_ACK:       u8 = 0x01;
const STATUS_DRIVER:    u8 = 0x02;
const STATUS_DRIVER_OK: u8 = 0x04;
const STATUS_FAILED:    u8 = 0x80;

// Descriptor flags
const VRING_DESC_F_NEXT:  u16 = 0x0001; // descriptor chains to next
const VRING_DESC_F_WRITE: u16 = 0x0002; // device writes into this buffer

// Virtio-net header: 10 bytes (legacy, without VIRTIO_NET_F_MRG_RXBUF).
// Fields: flags(1) gso_type(1) hdr_len(2) gso_size(2) csum_start(2) csum_offset(2).
// Zeroed = plain Ethernet frame, no GSO or checksum offload.
const NET_HDR_SIZE: usize = 10;

pub const ETH_FRAME_SIZE: usize = 1514; // max Ethernet frame (excl. FCS)
const ETH_HDR_SIZE: usize = 14;

// RX buffer must accommodate the virtio header followed by a full Ethernet frame.
const RX_BUF_SIZE: usize = NET_HDR_SIZE + ETH_FRAME_SIZE;

// ——— Virtqueue layout helpers ————————————————————————————————————————————————
//
// All offsets are relative to the virtqueue's base address.
// Legacy virtio spec, QUEUE_ALIGN = 4096:
//
//   offset 0              descriptor table  (q * 16 bytes)
//   offset q*16 + 0       avail.flags (u16)
//   offset q*16 + 2       avail.idx   (u16)
//   offset q*16 + 4       avail.ring  (q × u16)
//   offset ALIGN(…, 4096) used.flags  (u16)
//   offset …   + 2        used.idx    (u16)
//   offset …   + 4        used.ring   (q × { id:u32, len:u32 })

fn vq_avail_idx_off(q: usize)  -> usize { q * 16 + 2 }
fn vq_avail_ring_off(q: usize) -> usize { q * 16 + 4 }
fn vq_used_base(q: usize)      -> usize {
    let avail_end = q * 16 + 4 + q * 2 + 2; // descriptors + avail (with used_event)
    (avail_end + 4095) & !4095               // round up to QUEUE_ALIGN
}
fn vq_used_idx_off(q: usize)   -> usize { vq_used_base(q) + 2 }
fn vq_total_bytes(q: usize)    -> usize { vq_used_base(q) + 4 + q * 8 + 2 }
fn vq_pages_needed(q: usize)   -> usize { (vq_total_bytes(q) + 4095) / 4096 }

// Raw storage for one virtqueue.  Six pages so we can always find a run of
// up to three physically-contiguous pages (q=256 needs 3) even when the BSS
// frame allocator gives non-contiguous frames.
#[repr(C, align(4096))]
struct VirtqBuf([u8; 4096 * 6]);
static mut VIRTQ_TX_BUF: VirtqBuf = VirtqBuf([0u8; 4096 * 6]);
static mut VIRTQ_RX_BUF: VirtqBuf = VirtqBuf([0u8; 4096 * 6]);

// Zeroed virtio_net_hdr posted as the first descriptor of every TX chain.
static mut TX_HDR: [u8; NET_HDR_SIZE] = [0u8; NET_HDR_SIZE];
// Outgoing frame payload (copied from caller before submission).
static mut TX_BUF: [u8; ETH_FRAME_SIZE] = [0u8; ETH_FRAME_SIZE];
// Receive buffer: virtio header (NET_HDR_SIZE) followed by the frame.
static mut RX_BUF: [u8; RX_BUF_SIZE] = [0u8; RX_BUF_SIZE];

// ——— Volatile virtqueue accessors ————————————————————————————————————————————

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
unsafe fn q_read32(base: *const u8, off: usize) -> u32 {
    u32::from_le(core::ptr::read_volatile(base.add(off) as *const u32))
}

unsafe fn write_desc(base: *mut u8, idx: usize, addr: u64, len: u32, flags: u16, next: u16) {
    let off = idx * 16;
    q_write64(base, off,      addr);
    q_write32(base, off + 8,  len);
    q_write16(base, off + 12, flags);
    q_write16(base, off + 14, next);
}

// ——— VirtioNet ————————————————————————————————————————————————————————————————

pub struct VirtioNet {
    io_base:        u16,
    tx_last_used:   u16,
    rx_last_used:   u16,
    tx_vq_base:     *mut u8,
    rx_vq_base:     *mut u8,
    // Byte offsets within each virtqueue buffer, derived from q_size.
    // Both queues share the same depth so these apply to both.
    avail_idx_off:  usize,
    avail_ring_off: usize,
    used_idx_off:   usize,
    q_size:         usize,
}

unsafe impl Send for VirtioNet {}

impl VirtioNet {
    /// Scan PCI for a virtio-net device, initialise both virtqueues, pre-post
    /// one receive buffer, and return a ready handle.  Returns `None` if no
    /// device is found or initialisation fails.
    pub fn try_init() -> Option<Self> {
        let (bus, dev, io_base) = find_virtio_net()?;

        // Read the queue depth from receiveq (queue 0).  Both queues have the
        // same depth in QEMU.  QUEUE_NUM is read-only in legacy mode.
        unsafe { outw(io_base + VIRT_QUEUE_SELECT, RXQ); }
        let q_size = unsafe { inw(io_base + VIRT_QUEUE_NUM) } as usize;
        if q_size == 0 {
            return None;
        }

        let pages_needed = vq_pages_needed(q_size);
        if pages_needed > 6 {
            // Static buffers are only 6 pages; can't satisfy the request.
            return None;
        }

        let rx_virt = unsafe { find_contiguous_pages(pages_needed, &VIRTQ_RX_BUF)? };
        let tx_virt = unsafe { find_contiguous_pages(pages_needed, &VIRTQ_TX_BUF)? };

        unsafe {
            core::ptr::write_bytes(rx_virt as *mut u8, 0, pages_needed * 4096);
            core::ptr::write_bytes(tx_virt as *mut u8, 0, pages_needed * 4096);

            // Enable I/O space + bus-mastering in PCI command register.
            let cmd = pci_read32(bus, dev, 0, 0x04);
            pci_write32(bus, dev, 0, 0x04, cmd | 0x0005);

            // Reset → ACK → DRIVER (required virtio initialisation sequence).
            outb(io_base + VIRT_DEVICE_STATUS, 0);
            outb(io_base + VIRT_DEVICE_STATUS, STATUS_ACK);
            outb(io_base + VIRT_DEVICE_STATUS, STATUS_ACK | STATUS_DRIVER);

            // Feature negotiation: accept no optional features.
            let _host_feat = inl(io_base + VIRT_DEVICE_FEATURES);
            outl(io_base + VIRT_DRIVER_FEATURES, 0);

            // Register queue 0 (receiveq) — device fills these with incoming frames.
            outw(io_base + VIRT_QUEUE_SELECT, RXQ);
            outl(io_base + VIRT_QUEUE_PFN, (virt_to_phys(rx_virt) >> 12) as u32);

            // Register queue 1 (transmitq) — driver posts outgoing frames here.
            outw(io_base + VIRT_QUEUE_SELECT, TXQ);
            outl(io_base + VIRT_QUEUE_PFN, (virt_to_phys(tx_virt) >> 12) as u32);

            // Signal DRIVER_OK: device may now start processing queues.
            outb(io_base + VIRT_DEVICE_STATUS,
                STATUS_ACK | STATUS_DRIVER | STATUS_DRIVER_OK);

            if inb(io_base + VIRT_DEVICE_STATUS) & STATUS_FAILED != 0 {
                return None;
            }

            // Pre-compute virtqueue field offsets (same for both queues).
            let avail_idx_off  = vq_avail_idx_off(q_size);
            let avail_ring_off = vq_avail_ring_off(q_size);
            let used_idx_off   = vq_used_idx_off(q_size);

            // Pre-populate the receive queue with one buffer so the device has
            // somewhere to deposit the first incoming frame.
            //
            // Descriptor 0: entire RX_BUF (virtio header + frame space), WRITE.
            let rx_phys = virt_to_phys(core::ptr::addr_of!(RX_BUF) as usize);
            let rx_base = rx_virt as *mut u8;
            write_desc(rx_base, 0, rx_phys, RX_BUF_SIZE as u32, VRING_DESC_F_WRITE, 0);

            // avail.ring[0] = descriptor index 0.
            q_write16(rx_base, avail_ring_off, 0);
            fence(Ordering::SeqCst);
            // avail.idx = 1: one buffer is available.
            q_write16(rx_base, avail_idx_off, 1);

            // Kick the receive queue so QEMU picks up the posted buffer.
            outw(io_base + VIRT_QUEUE_NOTIFY, RXQ);

            Some(VirtioNet {
                io_base,
                tx_last_used:   0,
                rx_last_used:   0,
                tx_vq_base:     tx_virt as *mut u8,
                rx_vq_base:     rx_virt as *mut u8,
                avail_idx_off,
                avail_ring_off,
                used_idx_off,
                q_size,
            })
        }
    }

    /// Send a raw Ethernet frame.  `frame` must be ≥ ETH_HDR_SIZE and
    /// ≤ ETH_FRAME_SIZE bytes.  Blocks until the device signals completion.
    pub fn net_send(&mut self, frame: &[u8]) -> Result<usize, ()> {
        if frame.len() < ETH_HDR_SIZE || frame.len() > ETH_FRAME_SIZE {
            return Err(());
        }
        let len = frame.len();
        unsafe {
            TX_BUF[..len].copy_from_slice(frame);
            self.do_tx(len)?;
        }
        Ok(len)
    }

    /// Poll for a received raw Ethernet frame.  Returns the number of bytes
    /// written into `buf` (0 means no frame was available), or `Err(())` if
    /// `buf` is too small.  Non-blocking.
    pub fn net_recv(&mut self, buf: &mut [u8]) -> Result<usize, ()> {
        if buf.len() < ETH_FRAME_SIZE {
            return Err(());
        }
        unsafe { self.do_rx(buf) }
    }

    unsafe fn do_tx(&mut self, len: usize) -> Result<(), ()> {
        let base      = self.tx_vq_base;
        let avail_off = self.avail_idx_off;
        let ring_off  = self.avail_ring_off;
        let used_off  = self.used_idx_off;
        let q_mask    = (self.q_size - 1) as u16;

        let hdr_phys = virt_to_phys(core::ptr::addr_of!(TX_HDR) as usize);
        let buf_phys = virt_to_phys(core::ptr::addr_of!(TX_BUF) as usize);

        // Descriptor 0: zeroed virtio_net_hdr (device reads, chain continues to 1).
        write_desc(base, 0, hdr_phys, NET_HDR_SIZE as u32, VRING_DESC_F_NEXT, 1);
        // Descriptor 1: frame payload (device reads, end of chain).
        write_desc(base, 1, buf_phys, len as u32, 0, 0);

        // Post the chain head (desc 0) into the available ring.
        let avail_idx = q_read16(base, avail_off);
        let slot = (avail_idx & q_mask) as usize;
        q_write16(base, ring_off + slot * 2, 0);

        fence(Ordering::SeqCst);
        q_write16(base, avail_off, avail_idx.wrapping_add(1));

        // Kick the transmit queue (queue 1).
        outw(self.io_base + VIRT_QUEUE_NOTIFY, TXQ);

        // Busy-wait for the device to return the descriptor via the used ring.
        let target = self.tx_last_used.wrapping_add(1);
        wait_for_used(base, used_off, target)?;
        self.tx_last_used = target;

        Ok(())
    }

    unsafe fn do_rx(&mut self, buf: &mut [u8]) -> Result<usize, ()> {
        let base      = self.rx_vq_base;
        let avail_off = self.avail_idx_off;
        let ring_off  = self.avail_ring_off;
        let used_off  = self.used_idx_off;
        let q_mask    = (self.q_size - 1) as u16;

        // Non-blocking check: has the device completed a receive?
        if q_read16(base, used_off) == self.rx_last_used {
            return Ok(0);
        }

        // Consume the completed used-ring entry.
        // used.ring layout: { id: u32, len: u32 } per element, ring starts at +4.
        // used.ring[slot].len is at: vq_used_base + 4 + slot*8 + 4
        let slot = (self.rx_last_used & q_mask) as usize;
        let used_len = q_read32(base,
            vq_used_base(self.q_size) + 4 + slot * 8 + 4) as usize;

        // The device-reported length includes the NET_HDR_SIZE prefix; strip it.
        let frame_len = used_len.saturating_sub(NET_HDR_SIZE).min(ETH_FRAME_SIZE);
        if frame_len > 0 {
            buf[..frame_len].copy_from_slice(&RX_BUF[NET_HDR_SIZE..NET_HDR_SIZE + frame_len]);
        }

        self.rx_last_used = self.rx_last_used.wrapping_add(1);

        // Recycle the buffer: re-post descriptor 0 into the available ring so
        // the device can fill it with the next incoming frame.
        let rx_phys = virt_to_phys(core::ptr::addr_of!(RX_BUF) as usize);
        write_desc(base, 0, rx_phys, RX_BUF_SIZE as u32, VRING_DESC_F_WRITE, 0);

        let avail_idx = q_read16(base, avail_off);
        let slot2 = (avail_idx & q_mask) as usize;
        q_write16(base, ring_off + slot2 * 2, 0);

        fence(Ordering::SeqCst);
        q_write16(base, avail_off, avail_idx.wrapping_add(1));

        // Kick the receive queue (queue 0).
        outw(self.io_base + VIRT_QUEUE_NOTIFY, RXQ);

        Ok(frame_len)
    }

    /// Read the MAC address from the device config region.
    pub fn get_mac(&self) -> [u8; 6] {
        let mut mac = [0u8; 6];
        unsafe {
            for i in 0..6 {
                mac[i] = inb(self.io_base + VIRT_NET_MAC + i as u16);
            }
        }
        mac
    }

    /// Returns `true` if the link-status bit in the device config is set.
    pub fn is_link_up(&self) -> bool {
        unsafe { inw(self.io_base + VIRT_NET_STATUS) & 1 != 0 }
    }
}

// ——— Helpers ——————————————————————————————————————————————————————————————————

/// Busy-wait until `used.idx` reaches `target`.  Times out after ~10^8 spins.
unsafe fn wait_for_used(base: *const u8, used_idx_off: usize, target: u16) -> Result<(), ()> {
    let mut spins: u64 = 0;
    loop {
        fence(Ordering::SeqCst);
        if q_read16(base, used_idx_off) == target {
            return Ok(());
        }
        spins = spins.wrapping_add(1);
        if spins == 100_000_000 {
            return Err(());
        }
        core::hint::spin_loop();
    }
}

/// Search `buf` (6 pages) for a run of `needed` physically-contiguous pages.
/// Returns the virtual address of the first page in the run, or `None`.
unsafe fn find_contiguous_pages(needed: usize, buf: &VirtqBuf) -> Option<usize> {
    if needed == 0 || needed > 6 {
        return None;
    }
    let base = buf as *const _ as usize;
    'outer: for i in 0..=(6 - needed) {
        for j in 0..needed - 1 {
            let p0 = virt_to_phys(base + (i + j)     * 4096);
            let p1 = virt_to_phys(base + (i + j + 1) * 4096);
            if p1 != p0 + 4096 {
                continue 'outer;
            }
        }
        return Some(base + i * 4096);
    }
    None
}

// ——— PCI scan ————————————————————————————————————————————————————————————————

/// Scan all PCI buses/devices for the first virtio-net device
/// (vendor 0x1AF4, device 0x1000) and return its (bus, dev, io_base).
fn find_virtio_net() -> Option<(u8, u8, u16)> {
    for bus in 0u8..=255 {
        for dev in 0u8..32 {
            let id = unsafe { pci_read32(bus, dev, 0, 0x00) };
            if id == 0xFFFF_FFFF {
                continue;
            }
            let vendor = (id & 0x0000_FFFF) as u16;
            let device = (id >> 16)          as u16;

            if vendor == 0x1AF4 && device == 0x1000 {
                let bar0 = unsafe { pci_read32(bus, dev, 0, 0x10) };
                if bar0 & 1 == 1 {
                    // I/O-space BAR: strip the I/O indicator bits.
                    return Some((bus, dev, (bar0 & !3) as u16));
                }
            }
        }
    }
    None
}
