use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::path::Path;
use std::ffi::CString;
use std::io;

use libc::{
    c_int, O_CREAT, O_RDWR, O_DIRECT, O_CLOEXEC,
    pwrite, fsync, ftruncate,
};

use crate::SECTOR_SIZE;
use super::aligned_buffer::AlignedBuffer;
use super::record::{encode_record, HEADER_SIZE, WalError};

const STAGING_SIZE: usize = 1024 * 1024;

pub struct WalWriter {
    fd: OwnedFd,
    sector_offset: u64,
    tail_offset: usize,
    staging: AlignedBuffer,
    next_seq: u64,
}

impl WalWriter {
    pub fn open<P: AsRef<Path>>(path: P) -> io::Result<Self> {
        let cpath = CString::new(path.as_ref().as_os_str().as_encoded_bytes())
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;

        let raw_fd: c_int = unsafe {
            libc::open(
                cpath.as_ptr(),
                O_RDWR | O_CREAT | O_DIRECT | O_CLOEXEC,
                0o644,
            )
        };
        if raw_fd < 0 {
            return Err(io::Error::last_os_error());
        }
        let fd = unsafe { OwnedFd::from_raw_fd(raw_fd) };

        Ok(Self {
            fd,
            sector_offset: 0,
            tail_offset: 0,
            staging: AlignedBuffer::new(STAGING_SIZE),
            next_seq: 1,
        })
    }

    pub fn append(&mut self, payload: &[u8]) -> Result<u64, WalError> {
        let record_size = HEADER_SIZE + payload.len();
        if record_size > STAGING_SIZE {
            return Err(WalError::PayloadTooLarge(payload.len()));
        }

        let seq = self.next_seq;
        self.next_seq += 1;

        let write_start = self.tail_offset;
        if write_start + record_size > STAGING_SIZE {
            return Err(WalError::PayloadTooLarge(payload.len()));
        }

        encode_record(seq, payload, &mut self.staging[write_start..write_start + record_size])?;

        let new_tail = write_start + record_size;
        let write_end_aligned = new_tail.next_multiple_of(SECTOR_SIZE);
        for b in &mut self.staging[new_tail..write_end_aligned] {
            *b = 0;
        }

        let written = unsafe {
            pwrite(
                self.fd.as_raw_fd(),
                self.staging.as_ptr() as *const libc::c_void,
                write_end_aligned,
                self.sector_offset as libc::off_t,
            )
        };
        if written < 0 {
            return Err(WalError::Io(io::Error::last_os_error()));
        }
        if (written as usize) != write_end_aligned {
            return Err(WalError::Io(io::Error::new(
                io::ErrorKind::WriteZero,
                format!("short write: {} of {}", written, write_end_aligned),
            )));
        }

        let full_sectors = write_end_aligned / SECTOR_SIZE;
        let last_sector_start = (full_sectors - 1) * SECTOR_SIZE;

        if last_sector_start > 0 {
            self.staging.copy_within(last_sector_start..last_sector_start + SECTOR_SIZE, 0);
            for b in &mut self.staging[SECTOR_SIZE..write_end_aligned] {
                *b = 0;
            }
        }

        self.tail_offset = new_tail % SECTOR_SIZE;
        self.sector_offset += last_sector_start as u64;

        Ok(seq)
    }

    pub fn sync(&self) -> io::Result<()> {
        let rc = unsafe { fsync(self.fd.as_raw_fd()) };
        if rc < 0 { Err(io::Error::last_os_error()) } else { Ok(()) }
    }

    pub fn truncate_to_logical_end(&self) -> io::Result<()> {
        let logical_end = self.sector_offset + self.tail_offset as u64;
        let rc = unsafe { ftruncate(self.fd.as_raw_fd(), logical_end as libc::off_t) };
        if rc < 0 { Err(io::Error::last_os_error()) } else { Ok(()) }
    }
}