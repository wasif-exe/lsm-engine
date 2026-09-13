use std::alloc::{self, Layout};
use std::ops::{Deref, DerefMut};
use std::ptr::NonNull;
use std::slice;

use crate::SECTOR_SIZE;
pub struct AlignedBuffer {
    ptr: NonNull<u8>,
    len: usize,
    layout: Layout,
}

unsafe impl Send for AlignedBuffer {}
unsafe impl Sync for AlignedBuffer {}

impl AlignedBuffer {
    pub fn new(size: usize) -> Self {
        let aligned_len = size.next_multiple_of(SECTOR_SIZE).max(SECTOR_SIZE);

        let layout = Layout::from_size_align(aligned_len, SECTOR_SIZE)
            .expect("SECTOR_SIZE must be a power of two");

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

    pub fn zero(&mut self) {
        unsafe { std::ptr::write_bytes(self.ptr.as_ptr(), 0, self.len); }
    }
}

impl Deref for AlignedBuffer {
    type Target = [u8];
    #[inline]
    fn deref(&self) -> &[u8] {
        unsafe { slice::from_raw_parts(self.ptr.as_ptr(), self.len) }
    }
}

impl DerefMut for AlignedBuffer {
    #[inline]
    fn deref_mut(&mut self) -> &mut [u8] {
        unsafe { slice::from_raw_parts_mut(self.ptr.as_ptr(), self.len) }
    }
}

impl Drop for AlignedBuffer {
    fn drop(&mut self) {
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