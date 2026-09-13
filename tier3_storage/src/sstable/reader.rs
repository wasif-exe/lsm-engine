use std::fs::File;
use std::io;
use std::os::fd::AsRawFd;
use std::path::Path;
use std::ptr::NonNull;

use super::block::BlockIter;
use super::footer::{Footer, FOOTER_SIZE};
use crate::bloom::BloomFilter;

pub struct IndexRecord {
    pub last_key: Vec<u8>,
    pub offset: u64,
    pub len: u64,
}

pub struct SSTableReader {
    mmap_ptr: NonNull<u8>,
    mmap_len: usize,
    index: Vec<IndexRecord>,
    footer: Footer,
}


unsafe impl Send for SSTableReader {}
unsafe impl Sync for SSTableReader {}

impl SSTableReader {
    pub fn open<P: AsRef<Path>>(path: P) -> io::Result<Self> {
        let file = File::open(path)?;
        let metadata = file.metadata()?;
        let file_len = metadata.len() as usize;

        if file_len < FOOTER_SIZE {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "file too small for sstable"));
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

        Ok(Self {
            mmap_ptr,
            mmap_len: file_len,
            index,
            footer,
        })
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

            if curr + k_len + 16 > index_slice.len() {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "index record corrupted"));
            }

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

    pub fn get(&self, key: &[u8]) -> Option<(Option<Vec<u8>>, u64)> {
        if let Some(f_slice) = self.filter_slice() {
            if let Some(filter) = BloomFilter::new(f_slice) {
                if !filter.contains(key) {
                    return None;
                }
            }
        }

        if self.index.is_empty() {
            return None;
        }

        let target_idx = match self.index.binary_search_by(|entry| entry.last_key.as_slice().cmp(key)) {
            Ok(idx) => idx,
            Err(idx) => idx,
        };

        if target_idx >= self.index.len() {
            return None;
        }

        let entry = &self.index[target_idx];
        let start = entry.offset as usize;
        let end = start + entry.len as usize;

        if end > self.mmap_len {
            return None;
        }


        let block_slice = unsafe {
            std::slice::from_raw_parts(self.mmap_ptr.as_ptr().add(start), entry.len as usize)
        };

        let mut iter = BlockIter::new(block_slice);
        iter.search(key).map(|(val, seq)| (val.map(|v| v.to_vec()), seq))
    }

    pub fn filter_slice(&self) -> Option<&[u8]> {
        if self.footer.filter_len == 0 {
            return None;
        }
        let start = self.footer.filter_offset as usize;
        let len = self.footer.filter_len as usize;
        if start + len > self.mmap_len {
            return None;
        }

        unsafe {
            Some(std::slice::from_raw_parts(self.mmap_ptr.as_ptr().add(start), len))
        }
    }

    pub fn get_first_key(&self) -> Option<Vec<u8>> {
        if self.index.is_empty() {
            return None;
        }
        let entry = &self.index[0];
        let block_slice = unsafe {
            std::slice::from_raw_parts(self.mmap_ptr.as_ptr().add(entry.offset as usize), entry.len as usize)
        };
        let mut iter = BlockIter::new(block_slice);
        iter.next_entry().map(|(k, _, _)| k.to_vec())
    }

    pub fn get_last_key(&self) -> Option<Vec<u8>> {
        self.index.last().map(|e| e.last_key.clone())
    }
}

impl Drop for SSTableReader {
    fn drop(&mut self) {

        unsafe {
            libc::munmap(self.mmap_ptr.as_ptr() as *mut libc::c_void, self.mmap_len);
        }
    }
}