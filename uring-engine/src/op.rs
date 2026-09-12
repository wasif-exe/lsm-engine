//! SQE fill helpers for Multishot and Provided Buffer Operations.

use crate::sys::*;

/// Encode connection state into the 64-bit user_data field.
/// Layout: [fd: 32 bits][op: 8 bits][_reserved: 24 bits]
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

/// Prepare an ACCEPT MULTISHOT SQE.
///
/// Stays armed indefinitely in the kernel. Produces a CQE for every incoming connection.
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

/// Prepare a RECV MULTISHOT SQE with Kernel-Provided Buffers (`PBUF_RING`).
///
/// Stays armed indefinitely. As packets arrive from the NIC, the kernel claims
/// an available buffer from group `bgid` and delivers a CQE.
#[inline(always)]
pub fn prep_recv_multishot(sqe: &mut io_uring_sqe, fd: i32, bgid: u16) {
    sqe.opcode    = IORING_OP_RECV;
    sqe.fd        = fd;
    sqe.addr      = 0;
    sqe.len       = 0;
    sqe.off       = 0;
    sqe.flags     = IOSQE_BUFFER_SELECT;      // Kernel selects buffer from ring
    sqe.buf_index = bgid;                    // Buffer Group ID
    sqe.ioprio    = IORING_RECV_MULTISHOT;   // Stay armed for subsequent packets
    sqe.user_data = encode_user_data(fd, IORING_OP_RECV);
}

/// Prepare a SEND SQE.
#[inline(always)]
pub fn prep_send(sqe: &mut io_uring_sqe, fd: i32, buf: *const u8, len: u32) {
    sqe.opcode    = IORING_OP_SEND;
    sqe.fd        = fd;
    sqe.addr      = buf as u64;
    sqe.len       = len;
    sqe.off       = 0;
    sqe.op_flags  = libc::MSG_NOSIGNAL as u32; // Ignore broken pipes
    sqe.user_data = encode_user_data(fd, IORING_OP_SEND);
}

/// Prepare a CLOSE SQE.
#[inline(always)]
pub fn prep_close(sqe: &mut io_uring_sqe, fd: i32) {
    sqe.opcode    = IORING_OP_CLOSE;
    sqe.fd        = fd;
    sqe.user_data = encode_user_data(fd, IORING_OP_CLOSE);
}