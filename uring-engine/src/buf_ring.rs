//! Provided Buffer Ring (`io_uring_buf_ring`) Implementation (Linux 6.0+)
//!
//! Shared memory ring buffer where userspace provides buffers that the kernel
//! can atomically select on incoming network packet arrival.

use crate::sys::*;
use core::sync::atomic::{AtomicU16, Ordering};
use std::alloc::{alloc_zeroed, dealloc, Layout};
use std::io;
use std::os::unix::io::RawFd;

pub struct BufRing {
    bgid: u16,
    entries: u32,
    mask: u16,
    tail: u16,
    ring_ptr: *mut io_uring_buf,
    layout: Layout,
    buf_storage: *mut u8,
    buf_storage_layout: Layout,
    buf_size: usize,
}

impl BufRing {
    /// Allocate and register a new Provided Buffer Ring.
    ///
    /// - `entries`: Must be a power of 2 (e.g. 1024, 2048, 4096).
    /// - `buf_size`: Size of each buffer in bytes (e.g. 2048).
    /// - `bgid`: Buffer Group ID (0..65535).
    pub fn new(ring_fd: RawFd, bgid: u16, entries: u32, buf_size: usize) -> io::Result<Self> {
        assert!(entries.is_power_of_two(), "entries must be a power of two");

        // 1. Allocate page-aligned memory for the io_uring_buf_ring descriptor array
        let ring_bytes = (entries as usize) * core::mem::size_of::<io_uring_buf>();
        let layout = Layout::from_size_align(ring_bytes, 4096)
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))?;

        let ring_ptr = unsafe { alloc_zeroed(layout) as *mut io_uring_buf };
        if ring_ptr.is_null() {
            return Err(io::Error::new(io::ErrorKind::OutOfMemory, "Ring allocation failed"));
        }

        // 2. Allocate the contiguous backing data buffers (page-aligned)
        let total_storage_bytes = (entries as usize) * buf_size;
        let buf_storage_layout = Layout::from_size_align(total_storage_bytes, 4096)
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))?;

        let buf_storage = unsafe { alloc_zeroed(buf_storage_layout) };
        if buf_storage.is_null() {
            unsafe { dealloc(ring_ptr as *mut u8, layout) };
            return Err(io::Error::new(io::ErrorKind::OutOfMemory, "Buffer memory allocation failed"));
        }

        // 3. Register the buffer ring with the Linux kernel
        let reg = io_uring_buf_reg {
            ring_addr: ring_ptr as u64,
            ring_entries: entries,
            bgid,
            flags: 0,
            resv: [0; 3],
        };

        let ret = unsafe {
            io_uring_register(
                ring_fd,
                IORING_REGISTER_PBUF_RING,
                &reg as *const _ as *const libc::c_void,
                1,
            )
        };

        if ret < 0 {
            let err = io::Error::last_os_error();
            unsafe {
                dealloc(buf_storage, buf_storage_layout);
                dealloc(ring_ptr as *mut u8, layout);
            }
            return Err(err);
        }

        let mut br = Self {
            bgid,
            entries,
            mask: (entries - 1) as u16,
            tail: 0,
            ring_ptr,
            layout,
            buf_storage,
            buf_storage_layout,
            buf_size,
        };

        // 4. Populate all initial buffer slots in the ring
        for bid in 0..(entries as u16) {
            let buf_addr = unsafe { br.buf_storage.add((bid as usize) * buf_size) };
            br.add(buf_addr, buf_size as u32, bid);
        }
        br.flush();

        Ok(br)
    }

    /// Add a buffer back into the ring descriptor table.
    #[inline(always)]
    pub fn add(&mut self, addr: *mut u8, len: u32, bid: u16) {
        let idx = (self.tail & self.mask) as usize;
        unsafe {
            let entry = &mut *self.ring_ptr.add(idx);
            entry.addr = addr as u64;
            entry.len = len;
            entry.bid = bid;
            entry.resv = 0;
        }
        self.tail = self.tail.wrapping_add(1);
    }

    /// Recycle a buffer by its buffer ID (`bid`).
    #[inline(always)]
    pub fn recycle(&mut self, bid: u16) {
        let addr = unsafe { self.buf_storage.add((bid as usize) * self.buf_size) };
        self.add(addr, self.buf_size as u32, bid);
    }

    /// Flush the new tail to the kernel with release memory ordering.
    #[inline(always)]
    pub fn flush(&self) {
        // The tail is stored at byte offset 14 of the ring base pointer.
        let tail_ptr = unsafe {
            ((self.ring_ptr as *mut u8).add(14)) as *const AtomicU16
        };
        unsafe {
            (*tail_ptr).store(self.tail, Ordering::Release);
        }
    }

    /// Get raw pointer to a buffer by its `bid`.
    #[inline(always)]
    pub fn get_buffer_ptr(&self, bid: u16) -> *const u8 {
        unsafe { self.buf_storage.add((bid as usize) * self.buf_size) }
    }
}

impl Drop for BufRing {
    fn drop(&mut self) {
        unsafe {
            dealloc(self.buf_storage, self.buf_storage_layout);
            dealloc(self.ring_ptr as *mut u8, self.layout);
        }
    }
}