// memory/allocator.rs — Bump heap allocator (no_std + alloc)
//
// Provides a simple bump allocator that satisfies `GlobalAlloc`.
// Allocation advances a pointer forward; deallocation is a no-op.
// Total heap: 512 KiB — enough for fatfs internal structures.
//
// Single-core bare-metal: uses a raw static for the next pointer
// (no atomics needed, no preemption).

use core::alloc::{GlobalAlloc, Layout};

const HEAP_SIZE: usize = 512 * 1024; // 512 KiB

#[repr(C, align(16))]
struct HeapBuf([u8; HEAP_SIZE]);

static mut HEAP: HeapBuf = HeapBuf([0u8; HEAP_SIZE]);
static mut HEAP_NEXT: usize = 0;

pub struct BumpAllocator;

unsafe impl GlobalAlloc for BumpAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let current = HEAP_NEXT;
        let align   = layout.align();
        let aligned = (current + align - 1) & !(align - 1);
        let end     = aligned + layout.size();
        if end > HEAP_SIZE {
            return core::ptr::null_mut(); // OOM
        }
        HEAP_NEXT = end;
        HEAP.0.as_mut_ptr().add(aligned)
    }

    unsafe fn dealloc(&self, _ptr: *mut u8, _layout: Layout) {
        // Bump allocator — no deallocation.
    }
}

#[global_allocator]
pub static ALLOCATOR: BumpAllocator = BumpAllocator;
