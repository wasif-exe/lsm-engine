//! WAL recovery: scan a WAL file from offset 0, verify each record,
//! and yield valid records until we hit EOL, a torn write, or corruption.

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;

use super::record::{decode_record, WalError, WalRecord, HEADER_SIZE};

pub struct RecoveryScanner {
    buf: Vec<u8>,
    pos: usize,
    prev_seq: Option<u64>,
    /// File byte offset of the last successfully validated record end.
    /// After recovery, the WAL should be truncated to this length.
    last_valid_end: u64,
}

impl RecoveryScanner {
    /// Read the entire WAL into memory and prepare for scanning.
    /// For very large WALs, swap for a streaming reader.
    pub fn open<P: AsRef<Path>>(path: P) -> io::Result<Self> {
        // NOTE: recovery uses BUFFERED reads (no O_DIRECT). We just want
        // to inspect every byte the kernel has committed to the file.
        let mut f = File::open(path)?;
        f.seek(SeekFrom::Start(0))?;
        let mut buf = Vec::new();
        f.read_to_end(&mut buf)?;

        Ok(Self { buf, pos: 0, prev_seq: None, last_valid_end: 0 })
    }

    /// Pull the next valid record, or None if we've reached end/torn tail.
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
                // Any of these indicate we've hit the end of valid data.
                // last_valid_end already points to the truncation target.
                None
            }
            Err(_) => None,
        }
    }

    /// File length to truncate to (drops torn/corrupt tail).
    pub fn truncation_offset(&self) -> u64 { self.last_valid_end }

    /// Convenience: collect all valid records.
    pub fn replay_all(mut self) -> (Vec<WalRecord>, u64) {
        let mut out = Vec::new();
        while let Some(r) = self.next_record() { out.push(r); }
        (out, self.last_valid_end)
    }
}