//! Raw syscall gate for io_uring.
//!
//! We deliberately avoid any liburing / io-uring crate abstractions here.
//! Every struct is a byte-for-byte match of `linux/io_uring.h` on Linux 6.x+.
//!
//! ABI stability contract:
//!   The io_uring struct layouts are part of the stable kernel UAPI.
//!   Fields may be *added* (via unions/reserved), but never reordered.
//!   We pin to Linux >= 6.0 semantics.

#![allow(non_camel_case_types, dead_code)]

use core::ffi::c_void;

// ---------------------------------------------------------------------------
// Syscall numbers (x86_64 Linux). Verified via /usr/include/asm/unistd_64.h.
// ---------------------------------------------------------------------------
pub const SYS_IO_URING_SETUP:    libc::c_long = 425;
pub const SYS_IO_URING_ENTER:    libc::c_long = 426;
pub const SYS_IO_URING_REGISTER: libc::c_long = 427;

// ---------------------------------------------------------------------------
// io_uring_setup() flags (subset — we'll add more as needed)
// ---------------------------------------------------------------------------
pub const IORING_SETUP_SQPOLL:       u32 = 1 << 1;  // Kernel polls SQ (no enter syscall)
pub const IORING_SETUP_CQSIZE:       u32 = 1 << 3;  // Custom CQ size
pub const IORING_SETUP_CLAMP:        u32 = 1 << 4;  // Clamp SQ/CQ to max
pub const IORING_SETUP_SINGLE_ISSUER:u32 = 1 << 12; // Only one thread submits (perf hint, 6.0+)
pub const IORING_SETUP_DEFER_TASKRUN:u32 = 1 << 13; // Defer CQE work until ENTER (6.1+)
pub const IORING_SETUP_COOP_TASKRUN: u32 = 1 << 8;  // Skip IPI wakeups (5.19+)

// ---------------------------------------------------------------------------
// io_uring_register opcodes
// ---------------------------------------------------------------------------
pub const IORING_REGISTER_BUFFERS:   u32 = 0;
pub const IORING_REGISTER_PBUF_RING:   u32 = 22;
pub const IORING_UNREGISTER_PBUF_RING: u32 = 23;

// ---------------------------------------------------------------------------
// io_uring_enter() flags
// ---------------------------------------------------------------------------
pub const IORING_ENTER_GETEVENTS: u32 = 1 << 0;
pub const IORING_ENTER_SQ_WAKEUP: u32 = 1 << 1;

// ---------------------------------------------------------------------------
// mmap() offsets into the ring fd — kernel places SQ, CQ, and SQEs
// at these fixed offsets in the file. See io_uring_setup(2).
// ---------------------------------------------------------------------------
pub const IORING_OFF_SQ_RING: libc::off_t = 0;
pub const IORING_OFF_CQ_RING: libc::off_t = 0x0800_0000;
pub const IORING_OFF_SQES:    libc::off_t = 0x1000_0000;

// ---------------------------------------------------------------------------
// SQE opcodes
// ---------------------------------------------------------------------------
pub const IORING_OP_NOP:         u8 = 0;
pub const IORING_OP_READ_FIXED:  u8 = 4;
pub const IORING_OP_WRITE_FIXED: u8 = 5;
pub const IORING_OP_ACCEPT:      u8 = 13;
pub const IORING_OP_CLOSE:       u8 = 19;
pub const IORING_OP_READ:        u8 = 22;
pub const IORING_OP_WRITE:       u8 = 23;
pub const IORING_OP_SEND:        u8 = 26;
pub const IORING_OP_RECV:        u8 = 27;

// ---------------------------------------------------------------------------
// SQE Flags
// ---------------------------------------------------------------------------
pub const IOSQE_BUFFER_SELECT:  u8 = 1 << 5;

// ---------------------------------------------------------------------------
// Multishot flags (passed in sqe.ioprio)
// ---------------------------------------------------------------------------
pub const IORING_ACCEPT_MULTISHOT: u16 = 1 << 0;
pub const IORING_RECV_MULTISHOT:   u16 = 1 << 1;

// ---------------------------------------------------------------------------
// CQE Flags
// ---------------------------------------------------------------------------
pub const IORING_CQE_F_BUFFER:    u32 = 1 << 0;
pub const IORING_CQE_F_MORE:      u32 = 1 << 1;
pub const IORING_CQE_BUFFER_SHIFT: u32 = 16;

// ---------------------------------------------------------------------------
// io_sqring_offsets — tells us where head/tail/flags/etc. live in the mmap'd SQ ring.
// Layout: kernel/include/uapi/linux/io_uring.h
// ---------------------------------------------------------------------------
#[repr(C)]
#[derive(Debug, Default, Clone, Copy)]
pub struct io_sqring_offsets {
    pub head:         u32,
    pub tail:         u32,
    pub ring_mask:    u32,
    pub ring_entries: u32,
    pub flags:        u32,
    pub dropped:      u32,
    pub array:        u32,
    pub resv1:        u32,
    pub user_addr:    u64,
}

#[repr(C)]
#[derive(Debug, Default, Clone, Copy)]
pub struct io_cqring_offsets {
    pub head:         u32,
    pub tail:         u32,
    pub ring_mask:    u32,
    pub ring_entries: u32,
    pub overflow:     u32,
    pub cqes:         u32,
    pub flags:        u32,
    pub resv1:        u32,
    pub user_addr:    u64,
}

// ---------------------------------------------------------------------------
// io_uring_params — passed to io_uring_setup(). Kernel fills in offsets.
// ---------------------------------------------------------------------------
#[repr(C)]
#[derive(Debug, Default, Clone, Copy)]
pub struct io_uring_params {
    pub sq_entries:     u32,
    pub cq_entries:     u32,
    pub flags:          u32,
    pub sq_thread_cpu:  u32,
    pub sq_thread_idle: u32,
    pub features:       u32,
    pub wq_fd:          u32,
    pub resv:           [u32; 3],
    pub sq_off:         io_sqring_offsets,
    pub cq_off:         io_cqring_offsets,
}

// ---------------------------------------------------------------------------
// io_uring_sqe — 64-byte Submission Queue Entry.
// This struct has extensive unions in the kernel header. We flatten them
// using raw u64 fields and helper accessors in op.rs.
//
// Layout (Linux 6.x): 64 bytes exactly. `assert_eq!(size_of::<io_uring_sqe>(), 64)`.
// ---------------------------------------------------------------------------
#[repr(C)]
#[derive(Debug, Default, Clone, Copy)]
pub struct io_uring_sqe {
    pub opcode:       u8,   // IORING_OP_*
    pub flags:        u8,   // IOSQE_* flags
    pub ioprio:       u16,
    pub fd:           i32,
    pub off:          u64,  // offset OR addr2 (union)
    pub addr:         u64,  // buffer/iovec pointer
    pub len:          u32,  // buffer length OR nr iovecs
    pub op_flags:     u32,  // opcode-specific (e.g., accept flags, msg flags)
    pub user_data:    u64,  // opaque — echoed back in CQE (we stash conn state here)
    pub buf_index:    u16,  // OR buf_group (union)
    pub personality:  u16,
    pub splice_fd_in: i32,  // OR file_index / addr_len (union)
    pub addr3:        u64,  // OR cmd (union)
    pub __pad2:       [u64; 1],
}

const _: () = assert!(core::mem::size_of::<io_uring_sqe>() == 64);

// ---------------------------------------------------------------------------
// io_uring_cqe — 16-byte Completion Queue Entry (base form).
// With IORING_SETUP_CQE32 it becomes 32 bytes; we use the base form.
// ---------------------------------------------------------------------------
#[repr(C)]
#[derive(Debug, Default, Clone, Copy)]
pub struct io_uring_cqe {
    pub user_data: u64,  // echoed from SQE
    pub res:       i32,  // syscall return value (or -errno)
    pub flags:     u32,  // IORING_CQE_F_* (buffer selection, more coming, etc.)
}

const _: () = assert!(core::mem::size_of::<io_uring_cqe>() == 16);

// ---------------------------------------------------------------------------
// Linux 6.0+ Provided Buffer Ring & Multishot Structs
// ---------------------------------------------------------------------------

/// Entry in the userspace-kernel shared buffer ring.
#[repr(C)]
#[derive(Debug, Default, Clone, Copy)]
pub struct io_uring_buf {
    pub addr: u64,
    pub len:  u32,
    pub bid:  u16,
    pub resv: u16,
}

const _: () = assert!(core::mem::size_of::<io_uring_buf>() == 16);

/// Registration struct passed to io_uring_register(IORING_REGISTER_PBUF_RING)
#[repr(C)]
#[derive(Debug, Default, Clone, Copy)]
pub struct io_uring_buf_reg {
    pub ring_addr:    u64,
    pub ring_entries: u32,
    pub bgid:         u16,
    pub flags:        u16,
    pub resv:         [u64; 3],
}

const _: () = assert!(core::mem::size_of::<io_uring_buf_reg>() == 40);

// ---------------------------------------------------------------------------
// Syscall wrappers. All are #[inline] to allow the compiler to fold
// the libc::syscall trampoline into a direct `syscall` instruction under LTO.
// ---------------------------------------------------------------------------

/// io_uring_setup(entries, params) -> ring fd
///
/// SAFETY: `params` must point to a valid, writable `io_uring_params`.
/// Kernel writes back sq_off/cq_off/features.
#[inline]
pub unsafe fn io_uring_setup(entries: u32, params: *mut io_uring_params) -> i32 {
    libc::syscall(SYS_IO_URING_SETUP, entries, params) as i32
}

/// io_uring_enter(fd, to_submit, min_complete, flags, sig, sigsz)
///
/// SAFETY: `fd` must be a valid ring fd. `sig` may be null.
#[inline]
pub unsafe fn io_uring_enter(
    fd: i32,
    to_submit: u32,
    min_complete: u32,
    flags: u32,
    sig: *const libc::sigset_t,
) -> i32 {
    libc::syscall(
        SYS_IO_URING_ENTER,
        fd, to_submit, min_complete, flags, sig, 8usize
    ) as i32
}

/// io_uring_register(fd, opcode, arg, nr_args)
#[inline]
pub unsafe fn io_uring_register(
    fd: i32,
    opcode: u32,
    arg: *const c_void,
    nr_args: u32,
) -> i32 {
    libc::syscall(SYS_IO_URING_REGISTER, fd, opcode, arg, nr_args) as i32
}

/// Fetch current errno. Used after any syscall returning -1.
#[inline]
pub fn errno() -> i32 {
    // SAFETY: __errno_location is always valid on Linux glibc/musl.
    unsafe { *libc::__errno_location() }
}

// ---------------------------------------------------------------------------
// io_uring feature flags (returned in params.features after setup)
// ---------------------------------------------------------------------------
pub const IORING_FEAT_SINGLE_MMAP:   u32 = 1 << 0;
pub const IORING_FEAT_NODROP:        u32 = 1 << 1;
pub const IORING_FEAT_SUBMIT_STABLE: u32 = 1 << 2;
pub const IORING_FEAT_FAST_POLL:     u32 = 1 << 5;
pub const IORING_FEAT_CQE_SKIP:      u32 = 1 << 11;

// ---------------------------------------------------------------------------
// Thread Pinning & Affinity (libc bindings)
// ---------------------------------------------------------------------------

/// Pin the calling thread to a specific CPU core.
pub fn pin_current_thread_to_core(core_id: usize) -> std::io::Result<()> {
    unsafe {
        let mut cpuset: libc::cpu_set_t = std::mem::zeroed();
        
        // Use the official libc macros mapped to Rust functions
        libc::CPU_ZERO(&mut cpuset);
        libc::CPU_SET(core_id, &mut cpuset);

        // Set the affinity of the current thread (0 represents the calling thread)
        let ret = libc::pthread_setaffinity_np(
            libc::pthread_self(),
            std::mem::size_of::<libc::cpu_set_t>(),
            &cpuset,
        );

        if ret != 0 {
            return Err(std::io::Error::from_raw_os_error(ret));
        }
    }
    Ok(())
}