use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;

use super::record::{decode_record, WalError, WalRecord, HEADER_SIZE};

pub struct RecoveryScanner {
    buf: Vec<u8>,
    pos: usize,
    prev_seq: Option<u64>,
    last_valid_end: u64,
}

impl RecoveryScanner {
    pub fn open<P: AsRef<Path>>(path: P) -> io::Result<Self> {
        let mut f = File::open(path)?;
        f.seek(SeekFrom::Start(0))?;
        let mut buf = Vec::new();
        f.read_to_end(&mut buf)?;

        Ok(Self { buf, pos: 0, prev_seq: None, last_valid_end: 0 })
    }

    pub fn next_record(&mut self) -> Option<WalRecord> {
        if self.pos + HEADER_SIZE > self.buf.len() {
            return None;
        }

        match decode_record(&self.buf[self.pos..], self.prev_seq) {
            Ok((rec, consumed)) => {
                self.pos += consumed;
                self.prev_seq = Some(rec.seq);
                self.last_valid_end = self.pos as u64;
                Some(rec)
            }
            Err(WalError::EndOfLog)
            | Err(WalError::CorruptCrc { .. })
            | Err(WalError::Truncated { .. })
            | Err(WalError::SequenceRegression { .. }) => {
                None
            }
            Err(_) => None,
        }
    }

    pub fn truncation_offset(&self) -> u64 { self.last_valid_end }

    pub fn replay_all(mut self) -> (Vec<WalRecord>, u64) {
        let mut out = Vec::new();
        while let Some(r) = self.next_record() { out.push(r); }
        (out, self.last_valid_end)
    }
}