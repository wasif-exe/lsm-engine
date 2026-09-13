

use crate::sys::*;
use core::sync::atomic::{AtomicU32, Ordering};
use std::io;
use std::os::unix::io::RawFd;
use std::ptr;


pub struct Uring {

    fd: RawFd,


    sq_head:     *const AtomicU32, 
    sq_tail:     *const AtomicU32, 
    pub sq_mask:     u32,          
    pub sq_entries:  u32,        
    sq_array:    *mut u32,        


    sqes:        *mut io_uring_sqe,


    cq_head:     *const AtomicU32, 
    cq_tail:     *const AtomicU32, 
    pub cq_mask:     u32,
    pub cq_entries:  u32,
    cqes:        *const io_uring_cqe,


    sq_tail_local: u32,  
    sq_head_cache: u32, 


    ring_ptr: *mut libc::c_void,
    ring_sz:  usize,
    sqes_ptr: *mut libc::c_void,
    sqes_sz:  usize,
}


unsafe impl Send for Uring {}

impl Uring {

    pub fn new(entries: u32) -> io::Result<Self> {

        let mut params = io_uring_params::default();

        params.flags = IORING_SETUP_SINGLE_ISSUER | IORING_SETUP_CLAMP;

        let fd = unsafe { io_uring_setup(entries, &mut params) };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }


        let sq_ring_sz = (params.sq_off.array as usize)
            + (params.sq_entries as usize) * core::mem::size_of::<u32>();

        let cq_ring_sz = (params.cq_off.cqes as usize)
            + (params.cq_entries as usize) * core::mem::size_of::<io_uring_cqe>();

        let sqes_sz = (params.sq_entries as usize) * core::mem::size_of::<io_uring_sqe>();

        let ring_sz = if params.features & IORING_FEAT_SINGLE_MMAP != 0 {
            sq_ring_sz.max(cq_ring_sz)
        } else {
          
            sq_ring_sz 
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

    #[inline(always)]
    pub fn fd(&self) -> RawFd {
        self.fd
    }

    #[inline(always)]
    pub fn get_sqe(&mut self) -> Option<&mut io_uring_sqe> {

        let available = self.sq_tail_local.wrapping_sub(self.sq_head_cache);
        if available >= self.sq_entries {

            self.sq_head_cache = unsafe {
                (*self.sq_head).load(Ordering::Acquire)
            };
            let available = self.sq_tail_local.wrapping_sub(self.sq_head_cache);
            if available >= self.sq_entries {
                return None;
            }
        }

        let idx = self.sq_tail_local & self.sq_mask;


        let sqe = unsafe { &mut *self.sqes.add(idx as usize) };

     
        *sqe = io_uring_sqe::default();


        unsafe { *self.sq_array.add(idx as usize) = idx };

        self.sq_tail_local = self.sq_tail_local.wrapping_add(1);

        Some(sqe)
    }


    #[inline]
    pub fn submit(&mut self) -> io::Result<u32> {
        let to_submit = self.sq_tail_local.wrapping_sub(self.sq_head_cache);
        if to_submit == 0 {
            return Ok(0);
        }


        unsafe {
            (*self.sq_tail).store(self.sq_tail_local, Ordering::Release);
        }

        let ret = unsafe {
            io_uring_enter(self.fd, to_submit, 0, 0, ptr::null())
        };

        if ret < 0 {
            return Err(io::Error::last_os_error());
        }


        self.sq_head_cache = unsafe {
            (*self.sq_head).load(Ordering::Acquire)
        };

        Ok(ret as u32)
    }


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


    #[inline(always)]
    pub fn peek_cqe(&self) -> Option<&io_uring_cqe> {
        let head = unsafe { (*self.cq_head).load(Ordering::Relaxed) };

        let tail = unsafe { (*self.cq_tail).load(Ordering::Acquire) };

        if head == tail {
            return None;
        }

        let idx = head & self.cq_mask;

        Some(unsafe { &*self.cqes.add(idx as usize) })
    }


    #[inline(always)]
    pub fn advance_cq(&self, n: u32) {
        let head = unsafe { (*self.cq_head).load(Ordering::Relaxed) };

        unsafe {
            (*self.cq_head).store(head.wrapping_add(n), Ordering::Release);
        }
    }

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