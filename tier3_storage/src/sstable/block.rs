pub const TARGET_BLOCK_SIZE: usize = 4096;
pub const TOMBSTONE_VAL_LEN: u32 = u32::MAX;

pub struct BlockBuilder {
    pub buf: Vec<u8>,
    pub last_key: Vec<u8>,
}

impl BlockBuilder {
    pub fn new() -> Self {
        Self {
            buf: Vec::with_capacity(TARGET_BLOCK_SIZE),
            last_key: Vec::new(),
        }
    }

    pub fn add(&mut self, key: &[u8], value: Option<&[u8]>, seq: u64) {
        let k_len = key.len() as u32;
        let v_len = match value {
            Some(v) => v.len() as u32,
            None => TOMBSTONE_VAL_LEN,
        };

        self.buf.extend_from_slice(&k_len.to_le_bytes());
        self.buf.extend_from_slice(&v_len.to_le_bytes());
        self.buf.extend_from_slice(&seq.to_le_bytes());
        self.buf.extend_from_slice(key);
        if let Some(v) = value {
            self.buf.extend_from_slice(v);
        }

        self.last_key.clear();
        self.last_key.extend_from_slice(key);
    }

    pub fn estimated_size(&self) -> usize {
        self.buf.len()
    }

    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }

    pub fn finish(self) -> (Vec<u8>, Vec<u8>) {
        (self.buf, self.last_key)
    }
}

pub struct BlockIter<'a> {
    data: &'a [u8],
    offset: usize,
}

impl<'a> BlockIter<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, offset: 0 }
    }

    pub fn search(&mut self, target_key: &[u8]) -> Option<(Option<&'a [u8]>, u64)> {
        while let Some((key, val, seq)) = self.next_entry() {
            if key == target_key {
                return Some((val, seq));
            }
            if key > target_key {
                break;
            }
        }
        None
    }

    pub fn next_entry(&mut self) -> Option<(&'a [u8], Option<&'a [u8]>, u64)> {
        if self.offset + 16 > self.data.len() {
            return None;
        }

        let k_len = u32::from_le_bytes(self.data[self.offset..self.offset + 4].try_into().unwrap()) as usize;
        let v_len_raw = u32::from_le_bytes(self.data[self.offset + 4..self.offset + 8].try_into().unwrap());
        let seq = u64::from_le_bytes(self.data[self.offset + 8..self.offset + 16].try_into().unwrap());

        let total_val_len = if v_len_raw == TOMBSTONE_VAL_LEN { 0 } else { v_len_raw as usize };
        let total_len = 16 + k_len + total_val_len;

        if self.offset + total_len > self.data.len() {
            return None;
        }

        let key = &self.data[self.offset + 16..self.offset + 16 + k_len];
        let val = if v_len_raw == TOMBSTONE_VAL_LEN {
            None
        } else {
            Some(&self.data[self.offset + 16 + k_len..self.offset + total_len])
        };

        self.offset += total_len;
        Some((key, val, seq))
    }
}