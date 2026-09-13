

#![allow(non_camel_case_types, dead_code)]

use core::ffi::c_void;

pub const SYS_IO_URING_SETUP:    libc::c_long = 425;
pub const SYS_IO_URING_ENTER:    libc::c_long = 426;
pub const SYS_IO_URING_REGISTER: libc::c_long = 427;
pub const IORING_SETUP_SQPOLL:       u32 = 1 << 1;  
pub const IORING_SETUP_CQSIZE:       u32 = 1 << 3;  
pub const IORING_SETUP_CLAMP:        u32 = 1 << 4;  
pub const IORING_SETUP_SINGLE_ISSUER:u32 = 1 << 12; 
pub const IORING_SETUP_DEFER_TASKRUN:u32 = 1 << 13; 
pub const IORING_SETUP_COOP_TASKRUN: u32 = 1 << 8;  
pub const IORING_REGISTER_BUFFERS:   u32 = 0;
pub const IORING_REGISTER_PBUF_RING:   u32 = 22;
pub const IORING_UNREGISTER_PBUF_RING: u32 = 23;
pub const IORING_ENTER_GETEVENTS: u32 = 1 << 0;
pub const IORING_ENTER_SQ_WAKEUP: u32 = 1 << 1;
pub const IORING_OFF_SQ_RING: libc::off_t = 0;
pub const IORING_OFF_CQ_RING: libc::off_t = 0x0800_0000;
pub const IORING_OFF_SQES:    libc::off_t = 0x1000_0000;
pub const IORING_OP_NOP:         u8 = 0;
pub const IORING_OP_READ_FIXED:  u8 = 4;
pub const IORING_OP_WRITE_FIXED: u8 = 5;
pub const IORING_OP_ACCEPT:      u8 = 13;
pub const IORING_OP_CLOSE:       u8 = 19;
pub const IORING_OP_READ:        u8 = 22;
pub const IORING_OP_WRITE:       u8 = 23;
pub const IORING_OP_SEND:        u8 = 26;
pub const IORING_OP_RECV:        u8 = 27;
pub const IOSQE_BUFFER_SELECT:  u8 = 1 << 5;
pub const IORING_ACCEPT_MULTISHOT: u16 = 1 << 0;
pub const IORING_RECV_MULTISHOT:   u16 = 1 << 1;
pub const IORING_CQE_F_BUFFER:    u32 = 1 << 0;
pub const IORING_CQE_F_MORE:      u32 = 1 << 1;
pub const IORING_CQE_BUFFER_SHIFT: u32 = 16;
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


#[repr(C)]
#[derive(Debug, Default, Clone, Copy)]
pub struct io_uring_sqe {
    pub opcode:       u8,  
    pub flags:        u8,   
    pub ioprio:       u16,
    pub fd:           i32,
    pub off:          u64, 
    pub addr:         u64, 
    pub len:          u32,  
    pub op_flags:     u32,  
    pub user_data:    u64,  
    pub buf_index:    u16,  
    pub personality:  u16,
    pub splice_fd_in: i32, 
    pub addr3:        u64, 
    pub __pad2:       [u64; 1],
}

const _: () = assert!(core::mem::size_of::<io_uring_sqe>() == 64);


#[repr(C)]
#[derive(Debug, Default, Clone, Copy)]
pub struct io_uring_cqe {
    pub user_data: u64,  
    pub res:       i32,
    pub flags:     u32, 
}

const _: () = assert!(core::mem::size_of::<io_uring_cqe>() == 16);

#[repr(C)]
#[derive(Debug, Default, Clone, Copy)]
pub struct io_uring_buf {
    pub addr: u64,
    pub len:  u32,
    pub bid:  u16,
    pub resv: u16,
}

const _: () = assert!(core::mem::size_of::<io_uring_buf>() == 16);


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


#[inline]
pub unsafe fn io_uring_setup(entries: u32, params: *mut io_uring_params) -> i32 {
    libc::syscall(SYS_IO_URING_SETUP, entries, params) as i32
}


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


#[inline]
pub unsafe fn io_uring_register(
    fd: i32,
    opcode: u32,
    arg: *const c_void,
    nr_args: u32,
) -> i32 {
    libc::syscall(SYS_IO_URING_REGISTER, fd, opcode, arg, nr_args) as i32
}


#[inline]
pub fn errno() -> i32 {

    unsafe { *libc::__errno_location() }
}


pub const IORING_FEAT_SINGLE_MMAP:   u32 = 1 << 0;
pub const IORING_FEAT_NODROP:        u32 = 1 << 1;
pub const IORING_FEAT_SUBMIT_STABLE: u32 = 1 << 2;
pub const IORING_FEAT_FAST_POLL:     u32 = 1 << 5;
pub const IORING_FEAT_CQE_SKIP:      u32 = 1 << 11;


pub fn pin_current_thread_to_core(core_id: usize) -> std::io::Result<()> {
    unsafe {
        let mut cpuset: libc::cpu_set_t = std::mem::zeroed();
        

        libc::CPU_ZERO(&mut cpuset);
        libc::CPU_SET(core_id, &mut cpuset);


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