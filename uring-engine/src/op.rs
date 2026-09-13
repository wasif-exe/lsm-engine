

use crate::sys::*;


#[inline(always)]
pub fn encode_user_data(fd: i32, op: u8) -> u64 {
    (fd as u64 & 0xFFFF_FFFF) | ((op as u64) << 32)
}

#[inline(always)]
pub fn decode_fd(user_data: u64) -> i32 {
    (user_data & 0xFFFF_FFFF) as i32
}

#[inline(always)]
pub fn decode_op(user_data: u64) -> u8 {
    ((user_data >> 32) & 0xFF) as u8
}


#[inline(always)]
pub fn prep_accept_multishot(sqe: &mut io_uring_sqe, listen_fd: i32) {
    sqe.opcode    = IORING_OP_ACCEPT;
    sqe.fd        = listen_fd;
    sqe.addr      = 0;
    sqe.off       = 0;
    sqe.len       = 0;
    sqe.ioprio    = IORING_ACCEPT_MULTISHOT;
    sqe.op_flags  = libc::SOCK_CLOEXEC as u32;
    sqe.user_data = encode_user_data(listen_fd, IORING_OP_ACCEPT);
}

#[inline(always)]
pub fn prep_recv_multishot(sqe: &mut io_uring_sqe, fd: i32, bgid: u16) {
    sqe.opcode    = IORING_OP_RECV;
    sqe.fd        = fd;
    sqe.addr      = 0;
    sqe.len       = 0;
    sqe.off       = 0;
    sqe.flags     = IOSQE_BUFFER_SELECT;     
    sqe.buf_index = bgid;                 
    sqe.ioprio    = IORING_RECV_MULTISHOT;  
    sqe.user_data = encode_user_data(fd, IORING_OP_RECV);
}

#[inline(always)]
pub fn prep_send(sqe: &mut io_uring_sqe, fd: i32, buf: *const u8, len: u32) {
    sqe.opcode    = IORING_OP_SEND;
    sqe.fd        = fd;
    sqe.addr      = buf as u64;
    sqe.len       = len;
    sqe.off       = 0;
    sqe.op_flags  = libc::MSG_NOSIGNAL as u32; 
    sqe.user_data = encode_user_data(fd, IORING_OP_SEND);
}


#[inline(always)]
pub fn prep_close(sqe: &mut io_uring_sqe, fd: i32) {
    sqe.opcode    = IORING_OP_CLOSE;
    sqe.fd        = fd;
    sqe.user_data = encode_user_data(fd, IORING_OP_CLOSE);
}