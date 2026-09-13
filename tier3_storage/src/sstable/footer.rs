use std::io;

pub const FOOTER_SIZE: usize = 40;
pub const SSTABLE_MAGIC: u64 = 0x53535441424C4531;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Footer {
    pub index_offset: u64,
    pub index_len: u64,
    pub filter_offset: u64,
    pub filter_len: u64,
    pub magic: u64,
}

impl Footer {
    pub fn new(index_offset: u64, index_len: u64, filter_offset: u64, filter_len: u64) -> Self {
        Self {
            index_offset,
            index_len,
            filter_offset,
            filter_len,
            magic: SSTABLE_MAGIC,
        }
    }

    pub fn encode(&self, out: &mut [u8]) {
        assert!(out.len() >= FOOTER_SIZE);
        out[0..8].copy_from_slice(&self.index_offset.to_le_bytes());
        out[8..16].copy_from_slice(&self.index_len.to_le_bytes());
        out[16..24].copy_from_slice(&self.filter_offset.to_le_bytes());
        out[24..32].copy_from_slice(&self.filter_len.to_le_bytes());
        out[32..40].copy_from_slice(&self.magic.to_le_bytes());
    }

    pub fn decode(buf: &[u8]) -> io::Result<Self> {
        if buf.len() < FOOTER_SIZE {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "footer too small"));
        }
        let magic = u64::from_le_bytes(buf[32..40].try_into().unwrap());
        if magic != SSTABLE_MAGIC {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "invalid sstable magic"));
        }
        Ok(Self {
            index_offset: u64::from_le_bytes(buf[0..8].try_into().unwrap()),
            index_len: u64::from_le_bytes(buf[8..16].try_into().unwrap()),
            filter_offset: u64::from_le_bytes(buf[16..24].try_into().unwrap()),
            filter_len: u64::from_le_bytes(buf[24..32].try_into().unwrap()),
            magic,
        })
    }
}