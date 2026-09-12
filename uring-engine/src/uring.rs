//! io_uring ring management: mmap, submit, reap.
//!
//! This module owns the raw memory-mapped ring buffers shared with the kernel.
//! All pointer arithmetic is derived from the offsets the kernel returned
//! in `io_uring_params` during setup — we never hardcode layout assumptions.

use crate::sys::*;
use core::sync::atomic::{AtomicU32, Ordering};
use std::io;
use std::os::unix::io::RawFd;
use std::ptr;

/// A single io_uring instance bound to one thread.
///
/// In Phase 2, each worker core gets its own `Uring` — no sharing.
pub struct Uring {
    /// Ring file descriptor (from io_uring_setup).
    fd: RawFd,

    // -- SQ ring pointers (into mmap'd memory) --
    sq_head:     *const AtomicU32,  // kernel writes, we read
    sq_tail:     *const AtomicU32,  // we write, kernel reads (via local mirror)
    pub sq_mask:     u32,           // cached: ring_entries - 1 (power-of-2)
    pub sq_entries:  u32,           // cached: total SQ slots
    sq_array:    *mut u32,          // indirection table: sq_array[i] -> SQE index

    // -- SQE buffer (separate mmap) --
    sqes:        *mut io_uring_sqe,

    // -- CQ ring pointers --
    cq_head:     *const AtomicU32,  // we write, kernel reads
    cq_tail:     *const AtomicU32,  // kernel writes, we read
    pub cq_mask:     u32,
    pub cq_entries:  u32,
    cqes:        *const io_uring_cqe,

    // -- Local submission state (avoids atomic reads on hot path) --
    sq_tail_local: u32,  // mirrors *sq_tail; flushed on submit()
    sq_head_cache: u32,  // cached *sq_head; refreshed on submit()

    // -- mmap bookkeeping for cleanup --
    ring_ptr: *mut libc::c_void,
    ring_sz:  usize,
    sqes_ptr: *mut libc::c_void,
    sqes_sz:  usize,
}

// SAFETY: Uring contains raw pointers to mmap'd memory that is valid for the
// lifetime of the struct. The fd is closed in Drop. No cross-thread access
// is permitted (enforced by Phase 2's thread-per-core design).
unsafe impl Send for Uring {}

impl Uring {
    /// Create a new io_uring with `entries` SQ slots.
    /// The kernel will allocate 2× entries for the CQ by default.
    pub fn new(entries: u32) -> io::Result<Self> {
        // 1. Setup: ask the kernel to create the ring.
        let mut params = io_uring_params::default();
        // SINGLE_ISSUER: tells the kernel only one thread will submit.
        // This enables internal optimizations (lock elision in the kernel).
        params.flags = IORING_SETUP_SINGLE_ISSUER | IORING_SETUP_CLAMP;

        let fd = unsafe { io_uring_setup(entries, &mut params) };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }

        // 2. Compute mmap sizes from kernel-provided offsets.
        let sq_ring_sz = (params.sq_off.array as usize)
            + (params.sq_entries as usize) * core::mem::size_of::<u32>();

        let cq_ring_sz = (params.cq_off.cqes as usize)
            + (params.cq_entries as usize) * core::mem::size_of::<io_uring_cqe>();

        let sqes_sz = (params.sq_entries as usize) * core::mem::size_of::<io_uring_sqe>();

        // 3. mmap the ring buffers.
        // With IORING_FEAT_SINGLE_MMAP, SQ and CQ share one mapping.
        let ring_sz = if params.features & IORING_FEAT_SINGLE_MMAP != 0 {
            sq_ring_sz.max(cq_ring_sz)
        } else {
            // Fallback: two separate mmaps (not expected on kernel >= 5.4)
            sq_ring_sz  // we'd need a second mmap for CQ; omitted for brevity
        };

        let ring_ptr = unsafe {
            libc::mmap(
                ptr::null_mut(),
                ring_sz,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED | libc::MAP_POPULATE,
                fd,
                IORING_OFF_SQ_RING,
            )
        };
        if ring_ptr == libc::MAP_FAILED {
            unsafe { libc::close(fd) };
            return Err(io::Error::last_os_error());
        }

        let sqes_ptr = unsafe {
            libc::mmap(
                ptr::null_mut(),
                sqes_sz,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED | libc::MAP_POPULATE,
                fd,
                IORING_OFF_SQES,
            )
        };
        if sqes_ptr == libc::MAP_FAILED {
            unsafe {
                libc::munmap(ring_ptr, ring_sz);
                libc::close(fd);
            }
            return Err(io::Error::last_os_error());
        }

        // 4. Derive typed pointers from the mmap base + kernel offsets.
        // SAFETY: The kernel guarantees these offsets are within the mmap'd region.
        let ring_base = ring_ptr as *const u8;

        let sq_head = unsafe {
            &*(ring_base.add(params.sq_off.head as usize) as *const AtomicU32)
        };
        let sq_tail = unsafe {
            &*(ring_base.add(params.sq_off.tail as usize) as *const AtomicU32)
        };
        let sq_mask = unsafe {
            *(ring_base.add(params.sq_off.ring_mask as usize) as *const u32)
        };
        let sq_entries_kern = unsafe {
            *(ring_base.add(params.sq_off.ring_entries as usize) as *const u32)
        };
        let sq_array = unsafe {
            ring_base.add(params.sq_off.array as usize) as *mut u32
        };

        let cq_head = unsafe {
            &*(ring_base.add(params.cq_off.head as usize) as *const AtomicU32)
        };
        let cq_tail = unsafe {
            &*(ring_base.add(params.cq_off.tail as usize) as *const AtomicU32)
        };
        let cq_mask = unsafe {
            *(ring_base.add(params.cq_off.ring_mask as usize) as *const u32)
        };
        let cq_entries_kern = unsafe {
            *(ring_base.add(params.cq_off.ring_entries as usize) as *const u32)
        };
        let cqes = unsafe {
            ring_base.add(params.cq_off.cqes as usize) as *const io_uring_cqe
        };

        // 5. Initialize the SQ indirection array: identity mapping.
        // sq_array[i] = i means "SQE slot i is at index i in the SQE buffer."
        // This is the simplest mapping; advanced use cases reorder for priority.
        for i in 0..sq_entries_kern {
            unsafe { *sq_array.add(i as usize) = i };
        }

        Ok(Self {
            fd,
            sq_head,
            sq_tail,
            sq_mask,
            sq_entries: sq_entries_kern,
            sq_array,
            sqes: sqes_ptr as *mut io_uring_sqe,
            cq_head,
            cq_tail,
            cq_mask,
            cq_entries: cq_entries_kern,
            cqes,
            sq_tail_local: 0,
            sq_head_cache: 0,
            ring_ptr,
            ring_sz,
            sqes_ptr,
            sqes_sz,
        })
    }

    /// Returns the ring file descriptor (needed for io_uring_enter).
    #[inline(always)]
    pub fn fd(&self) -> RawFd {
        self.fd
    }

    /// Get the next available SQE slot, or `None` if the SQ is full.
    ///
    /// The returned SQE is zeroed and ready to be filled by an op helper.
    /// The SQE is NOT submitted until `submit()` is called.
    #[inline(always)]
    pub fn get_sqe(&mut self) -> Option<&mut io_uring_sqe> {
        // Refresh head cache if we might be full.
        // This is the only atomic read on the submit path — and only when
        // the ring is nearly full, so it's rarely executed under steady state.
        let available = self.sq_tail_local.wrapping_sub(self.sq_head_cache);
        if available >= self.sq_entries {
            // Kernel may have consumed entries; refresh.
            self.sq_head_cache = unsafe {
                (*self.sq_head).load(Ordering::Acquire)
            };
            let available = self.sq_tail_local.wrapping_sub(self.sq_head_cache);
            if available >= self.sq_entries {
                return None; // genuinely full
            }
        }

        let idx = self.sq_tail_local & self.sq_mask;

        // SAFETY: idx < sq_entries (guaranteed by mask), so the pointer is in-bounds.
        let sqe = unsafe { &mut *self.sqes.add(idx as usize) };

        // Zero the SQE. The kernel reads all 64 bytes, so stale data = UB.
        *sqe = io_uring_sqe::default();

        // Update the indirection array (identity mapping).
        unsafe { *self.sq_array.add(idx as usize) = idx };

        self.sq_tail_local = self.sq_tail_local.wrapping_add(1);

        Some(sqe)
    }

    /// Flush pending SQEs to the kernel.
    ///
    /// Returns the number of SQEs the kernel accepted.
    #[inline]
    pub fn submit(&mut self) -> io::Result<u32> {
        let to_submit = self.sq_tail_local.wrapping_sub(self.sq_head_cache);
        if to_submit == 0 {
            return Ok(0);
        }

        // Publish the new tail to the kernel.
        // Release ordering: all SQE writes must be visible before the tail advance.
        unsafe {
            (*self.sq_tail).store(self.sq_tail_local, Ordering::Release);
        }

        let ret = unsafe {
            io_uring_enter(self.fd, to_submit, 0, 0, ptr::null())
        };

        if ret < 0 {
            return Err(io::Error::last_os_error());
        }

        // Refresh head cache after submission.
        self.sq_head_cache = unsafe {
            (*self.sq_head).load(Ordering::Acquire)
        };

        Ok(ret as u32)
    }

    /// Submit pending SQEs AND wait for at least `min_complete` CQEs.
    ///
    /// This is the primary hot-path entry point: one syscall does both
    /// submission and completion harvesting, eliminating a round-trip.
    #[inline]
    pub fn submit_and_wait(&mut self, min_complete: u32) -> io::Result<u32> {
        let to_submit = self.sq_tail_local.wrapping_sub(self.sq_head_cache);

        // Publish tail.
        unsafe {
            (*self.sq_tail).store(self.sq_tail_local, Ordering::Release);
        }

        let ret = unsafe {
            io_uring_enter(
                self.fd,
                to_submit,
                min_complete,
                IORING_ENTER_GETEVENTS,
                ptr::null(),
            )
        };

        if ret < 0 {
            return Err(io::Error::last_os_error());
        }

        self.sq_head_cache = unsafe {
            (*self.sq_head).load(Ordering::Acquire)
        };

        Ok(ret as u32)
    }

    /// Peek at the next CQE without consuming it.
    ///
    /// Returns `None` if the CQ is empty (no completions ready).
    #[inline(always)]
    pub fn peek_cqe(&self) -> Option<&io_uring_cqe> {
        let head = unsafe { (*self.cq_head).load(Ordering::Relaxed) };
        // Acquire: we must see the CQE data before we see the tail advance.
        let tail = unsafe { (*self.cq_tail).load(Ordering::Acquire) };

        if head == tail {
            return None;
        }

        let idx = head & self.cq_mask;
        // SAFETY: idx < cq_entries (guaranteed by mask), pointer is in-bounds.
        Some(unsafe { &*self.cqes.add(idx as usize) })
    }

    /// Advance the CQ head by `n` entries, telling the kernel we've consumed them.
    ///
    /// # Safety contract
    /// Caller must have processed exactly `n` CQEs obtained via `peek_cqe()`.
    #[inline(always)]
    pub fn advance_cq(&self, n: u32) {
        let head = unsafe { (*self.cq_head).load(Ordering::Relaxed) };
        // Release: kernel must see our CQE processing before the head advance.
        unsafe {
            (*self.cq_head).store(head.wrapping_add(n), Ordering::Release);
        }
    }

    /// Drain all available CQEs, calling `f` for each one.
    ///
    /// This is the main completion loop pattern:
    /// ```ignore
    /// ring.for_each_cqe(|cqe| {
    ///     // dispatch based on cqe.user_data
    /// });
    /// ```
    #[inline]
    pub fn for_each_cqe<F: FnMut(&io_uring_cqe)>(&self, mut f: F) {
        let mut head = unsafe { (*self.cq_head).load(Ordering::Relaxed) };
        let tail = unsafe { (*self.cq_tail).load(Ordering::Acquire) };

        while head != tail {
            let idx = head & self.cq_mask;
            let cqe = unsafe { &*self.cqes.add(idx as usize) };
            f(cqe);
            head = head.wrapping_add(1);
        }

        if head != unsafe { (*self.cq_head).load(Ordering::Relaxed) } {
            unsafe { (*self.cq_head).store(head, Ordering::Release) };
        }
    }

    /// Register an array of `iovec` structures representing buffers with the kernel.
    ///
    /// # Safety
    /// The memory pointed to by the `iovec` structures must remain valid and MUST NOT
    /// be deallocated, resized, or moved for the duration of the registration (until
    /// the ring is dropped).
    pub unsafe fn register_buffers(&self, iovecs: &[libc::iovec]) -> io::Result<()> {
        let ret = io_uring_register(
            self.fd,
            IORING_REGISTER_BUFFERS,
            iovecs.as_ptr() as *const _,
            iovecs.len() as u32,
        );
        if ret < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
}

impl Drop for Uring {
    fn drop(&mut self) {
        unsafe {
            libc::munmap(self.sqes_ptr, self.sqes_sz);
            libc::munmap(self.ring_ptr, self.ring_sz);
            libc::close(self.fd);
        }
    }
}