use std::fs::File;
use std::io::{self, Write};
use std::path::Path;

use super::block::{BlockBuilder, TARGET_BLOCK_SIZE};
use super::footer::{Footer, FOOTER_SIZE};
use crate::bloom::BloomBuilder;

pub struct IndexEntry {
    pub last_key: Vec<u8>,
    pub offset: u64,
    pub len: u64,
}

pub struct SSTableWriter {
    file: File,
    current_block: BlockBuilder,
    index_entries: Vec<IndexEntry>,
    bloom_builder: BloomBuilder,
    current_offset: u64,
}

impl SSTableWriter {
    pub fn create<P: AsRef<Path>>(path: P) -> io::Result<Self> {
        let file = File::create(path)?;
        Ok(Self {
            file,
            current_block: BlockBuilder::new(),
            index_entries: Vec::new(),
            bloom_builder: BloomBuilder::new(),
            current_offset: 0,
        })
    }

    pub fn append(&mut self, key: &[u8], value: Option<&[u8]>, seq: u64) -> io::Result<()> {
        self.bloom_builder.add(key);
        self.current_block.add(key, value, seq);

        if self.current_block.estimated_size() >= TARGET_BLOCK_SIZE {
            self.flush_block()?;
        }
        Ok(())
    }

    fn flush_block(&mut self) -> io::Result<()> {
        if self.current_block.is_empty() {
            return Ok(());
        }

        let old_builder = std::mem::replace(&mut self.current_block, BlockBuilder::new());
        let (buf, last_key) = old_builder.finish();
        let block_len = buf.len() as u64;

        self.file.write_all(&buf)?;
        self.index_entries.push(IndexEntry {
            last_key,
            offset: self.current_offset,
            len: block_len,
        });

        self.current_offset += block_len;
        Ok(())
    }

    pub fn finish(mut self) -> io::Result<u64> {
        self.flush_block()?;

        let index_offset = self.current_offset;
        let mut index_buf = Vec::new();

        for entry in &self.index_entries {
            let k_len = entry.last_key.len() as u32;
            index_buf.extend_from_slice(&k_len.to_le_bytes());
            index_buf.extend_from_slice(&entry.last_key);
            index_buf.extend_from_slice(&entry.offset.to_le_bytes());
            index_buf.extend_from_slice(&entry.len.to_le_bytes());
        }

        self.file.write_all(&index_buf)?;
        let index_len = index_buf.len() as u64;
        self.current_offset += index_len;

        let filter_data = self.bloom_builder.build();
        let filter_offset = self.current_offset;
        let filter_len = filter_data.len() as u64;

        if filter_len > 0 {
            self.file.write_all(&filter_data)?;
            self.current_offset += filter_len;
        }

        let footer = Footer::new(index_offset, index_len, filter_offset, filter_len);
        let mut footer_buf = [0u8; FOOTER_SIZE];
        footer.encode(&mut footer_buf);
        self.file.write_all(&footer_buf)?;
        self.current_offset += FOOTER_SIZE as u64;

        self.file.sync_all()?;
        Ok(self.current_offset)
    }
}