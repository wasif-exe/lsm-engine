use std::fs::File;
use std::io;
use std::os::fd::AsRawFd;
use std::path::Path;
use std::ptr::NonNull;

use super::block::BlockIter;
use super::footer::{Footer, FOOTER_SIZE};
use super::reader::IndexRecord;

pub struct StreamingSSTableIterator {
    mmap_ptr: NonNull<u8>,
    mmap_len: usize,
    index: Vec<IndexRecord>,
    current_index_idx: usize,
    current_block_iter: Option<BlockIter<'static>>,
}

unsafe impl Send for StreamingSSTableIterator {}
unsafe impl Sync for StreamingSSTableIterator {}

impl StreamingSSTableIterator {
    pub fn open<P: AsRef<Path>>(path: P) -> io::Result<Self> {
        let file = File::open(path)?;
        let metadata = file.metadata()?;
        let file_len = metadata.len() as usize;

        if file_len < FOOTER_SIZE {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "file too small"));
        }


        let raw_ptr = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                file_len,
                libc::PROT_READ,
                libc::MAP_SHARED,
                file.as_raw_fd(),
                0,
            )
        };

        if raw_ptr == libc::MAP_FAILED {
            return Err(io::Error::last_os_error());
        }

        let mmap_ptr = NonNull::new(raw_ptr as *mut u8).unwrap();

        let full_slice = unsafe { std::slice::from_raw_parts(mmap_ptr.as_ptr(), file_len) };

        let footer = Footer::decode(&full_slice[file_len - FOOTER_SIZE..])?;
        let index = Self::parse_index(&full_slice, footer.index_offset as usize, footer.index_len as usize)?;

        let mut iter = Self {
            mmap_ptr,
            mmap_len: file_len,
            index,
            current_index_idx: 0,
            current_block_iter: None,
        };
        iter.load_next_block();
        Ok(iter)
    }

    fn parse_index(slice: &[u8], offset: usize, len: usize) -> io::Result<Vec<IndexRecord>> {
        if offset + len > slice.len() {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "index block out of bounds"));
        }

        let index_slice = &slice[offset..offset + len];
        let mut records = Vec::new();
        let mut curr = 0;

        while curr + 4 <= index_slice.len() {
            let k_len = u32::from_le_bytes(index_slice[curr..curr + 4].try_into().unwrap()) as usize;
            curr += 4;
            let last_key = index_slice[curr..curr + k_len].to_vec();
            curr += k_len;
            let block_offset = u64::from_le_bytes(index_slice[curr..curr + 8].try_into().unwrap());
            curr += 8;
            let block_len = u64::from_le_bytes(index_slice[curr..curr + 8].try_into().unwrap());
            curr += 8;

            records.push(IndexRecord {
                last_key,
                offset: block_offset,
                len: block_len,
            });
        }

        Ok(records)
    }

    fn load_next_block(&mut self) {
        if self.current_index_idx >= self.index.len() {
            self.current_block_iter = None;
            return;
        }

        let entry = &self.index[self.current_index_idx];
        self.current_index_idx += 1;

        let start = entry.offset as usize;
        let len = entry.len as usize;


        let block_slice = unsafe {
            std::slice::from_raw_parts(self.mmap_ptr.as_ptr().add(start), len)
        };

        self.current_block_iter = Some(BlockIter::new(block_slice));
    }

    pub fn next_kv(&mut self) -> Option<(Vec<u8>, Option<Vec<u8>>, u64)> {
        loop {
            if let Some(ref mut b_iter) = self.current_block_iter {
                if let Some((k, v, s)) = b_iter.next_entry() {
                    return Some((k.to_vec(), v.map(|x| x.to_vec()), s));
                }
            }

            if self.current_index_idx < self.index.len() {
                self.load_next_block();
            } else {
                return None;
            }
        }
    }
}

impl Drop for StreamingSSTableIterator {
    fn drop(&mut self) {

        unsafe {
            libc::munmap(self.mmap_ptr.as_ptr() as *mut libc::c_void, self.mmap_len);
        }
    }
}