//! Sector-aligned heap buffer for O_DIRECT I/O.
//!
//! Uses posix_memalign under the hood to obtain memory whose starting
//! address is a multiple of SECTOR_SIZE (4096 bytes on modern NVMe).

use std::alloc::{self, Layout};
use std::ops::{Deref, DerefMut};
use std::ptr::NonNull;
use std::slice;

use crate::SECTOR_SIZE;

/// A heap buffer whose base address AND length are aligned to SECTOR_SIZE.
///
/// Invariants:
///  - `ptr` is aligned to SECTOR_SIZE
///  - `len` is a multiple of SECTOR_SIZE
///  - Memory is owned; freed on Drop via matching Layout
pub struct AlignedBuffer {
    ptr: NonNull<u8>,
    len: usize,
    layout: Layout,
}

// SAFETY: AlignedBuffer owns its memory exclusively; the raw pointer
// cannot be aliased across threads without &mut / &.
unsafe impl Send for AlignedBuffer {}
unsafe impl Sync for AlignedBuffer {}

impl AlignedBuffer {
    /// Allocate a new zeroed buffer of `size` bytes.
    ///
    /// `size` will be rounded UP to the next multiple of SECTOR_SIZE
    /// to satisfy O_DIRECT length alignment.
    ///
    /// # Panics
    /// Panics if the allocator returns null (OOM).
    pub fn new(size: usize) -> Self {
        let aligned_len = size.next_multiple_of(SECTOR_SIZE).max(SECTOR_SIZE);

        // SAFETY: SECTOR_SIZE is a power of two (4096); aligned_len > 0.
        // Layout::from_size_align only fails if align is not power-of-two
        // or size overflows when rounded to alignment.
        let layout = Layout::from_size_align(aligned_len, SECTOR_SIZE)
            .expect("SECTOR_SIZE must be a power of two");

        // SAFETY: layout has non-zero size (min SECTOR_SIZE).
        // alloc_zeroed returns aligned memory or null on OOM.
        let raw = unsafe { alloc::alloc_zeroed(layout) };
        let ptr = NonNull::new(raw).unwrap_or_else(|| alloc::handle_alloc_error(layout));

        Self { ptr, len: aligned_len, layout }
    }

    #[inline]
    pub fn as_ptr(&self) -> *const u8 { self.ptr.as_ptr() }

    #[inline]
    pub fn as_mut_ptr(&mut self) -> *mut u8 { self.ptr.as_ptr() }

    #[inline]
    pub fn len(&self) -> usize { self.len }

    #[inline]
    pub fn is_empty(&self) -> bool { self.len == 0 }

    /// Zero the entire buffer.
    pub fn zero(&mut self) {
        // SAFETY: ptr is valid for len bytes, exclusively borrowed.
        unsafe { std::ptr::write_bytes(self.ptr.as_ptr(), 0, self.len); }
    }
}

impl Deref for AlignedBuffer {
    type Target = [u8];
    #[inline]
    fn deref(&self) -> &[u8] {
        // SAFETY: ptr valid for len bytes, exclusively borrowed via &self
        // (no interior mutability). Alignment holds by construction.
        unsafe { slice::from_raw_parts(self.ptr.as_ptr(), self.len) }
    }
}

impl DerefMut for AlignedBuffer {
    #[inline]
    fn deref_mut(&mut self) -> &mut [u8] {
        // SAFETY: exclusive borrow via &mut self; ptr valid for len bytes.
        unsafe { slice::from_raw_parts_mut(self.ptr.as_ptr(), self.len) }
    }
}

impl Drop for AlignedBuffer {
    fn drop(&mut self) {
        // SAFETY: ptr was allocated with exactly this Layout in `new`.
        unsafe { alloc::dealloc(self.ptr.as_ptr(), self.layout); }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allocation_is_sector_aligned() {
        let buf = AlignedBuffer::new(100);
        assert_eq!(buf.as_ptr() as usize % SECTOR_SIZE, 0);
        assert_eq!(buf.len() % SECTOR_SIZE, 0);
        assert!(buf.len() >= SECTOR_SIZE);
    }

    #[test]
    fn zero_initialized() {
        let buf = AlignedBuffer::new(SECTOR_SIZE);
        assert!(buf.iter().all(|&b| b == 0));
    }

    #[test]
    fn round_up_to_sector() {
        let buf = AlignedBuffer::new(SECTOR_SIZE + 1);
        assert_eq!(buf.len(), SECTOR_SIZE * 2);
    }

    #[test]
    fn mutation_persists() {
        let mut buf = AlignedBuffer::new(SECTOR_SIZE);
        buf[0] = 0xAB;
        buf[SECTOR_SIZE - 1] = 0xCD;
        assert_eq!(buf[0], 0xAB);
        assert_eq!(buf[SECTOR_SIZE - 1], 0xCD);
    }
}