//! WAL record framing.
//!
//! Wire format (little-endian):
//! ┌──────────┬──────────┬──────────┬──────────────┐
//! │ CRC32    │ Length   │ SeqNum   │ Payload      │
//! │ 4 bytes  │ 4 bytes  │ 8 bytes  │ Length bytes │
//! └──────────┴──────────┴──────────┴──────────────┘
//!
//! CRC covers [Length][SeqNum][Payload] — NOT itself.
//! Length is the size of Payload only (excludes header).

use std::io;

/// Size of the fixed record header (CRC + Length + SeqNum).
pub const HEADER_SIZE: usize = 4 + 4 + 8;

/// Maximum payload size for a single WAL record (16 MiB).
/// Larger values should be split at a higher layer.
pub const MAX_PAYLOAD_SIZE: usize = 16 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct WalRecord {
    pub seq: u64,
    pub payload: Vec<u8>,
}

#[derive(Debug)]
pub enum WalError {
    Io(io::Error),
    /// CRC mismatch — record is corrupt or torn.
    CorruptCrc { expected: u32, found: u32, seq: u64 },
    /// Length field claims more bytes than available in the file.
    Truncated { needed: usize, available: usize },
    /// Payload exceeds MAX_PAYLOAD_SIZE.
    PayloadTooLarge(usize),
    /// Sequence numbers must monotonically increase.
    SequenceRegression { prev: u64, found: u64 },
    /// End of a valid stream (not an error, just a marker).
    EndOfLog,
}

impl From<io::Error> for WalError {
    fn from(e: io::Error) -> Self { WalError::Io(e) }
}

impl std::fmt::Display for WalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WalError::Io(e) => write!(f, "I/O error: {}", e),
            WalError::CorruptCrc { expected, found, seq } =>
                write!(f, "CRC mismatch at seq {}: expected {:08x}, found {:08x}", seq, expected, found),
            WalError::Truncated { needed, available } =>
                write!(f, "record truncated: need {} bytes, {} available", needed, available),
            WalError::PayloadTooLarge(n) => write!(f, "payload too large: {} bytes", n),
            WalError::SequenceRegression { prev, found } =>
                write!(f, "sequence regression: prev={}, found={}", prev, found),
            WalError::EndOfLog => write!(f, "end of log"),
        }
    }
}

impl std::error::Error for WalError {}

/// Encode a record into a caller-provided buffer.
/// Returns total bytes written (HEADER_SIZE + payload.len()).
///
/// The buffer must be at least `HEADER_SIZE + payload.len()` bytes.
pub fn encode_record(seq: u64, payload: &[u8], out: &mut [u8]) -> Result<usize, WalError> {
    if payload.len() > MAX_PAYLOAD_SIZE {
        return Err(WalError::PayloadTooLarge(payload.len()));
    }
    let total = HEADER_SIZE + payload.len();
    if out.len() < total {
        return Err(WalError::Truncated { needed: total, available: out.len() });
    }

    let len = payload.len() as u32;

    // Layout: [CRC:4][Len:4][Seq:8][Payload]
    // Write Len + Seq + Payload first, then compute CRC over that region.
    out[4..8].copy_from_slice(&len.to_le_bytes());
    out[8..16].copy_from_slice(&seq.to_le_bytes());
    out[HEADER_SIZE..total].copy_from_slice(payload);

    let mut hasher = crc32fast::Hasher::new();
    hasher.update(&out[4..total]); // CRC covers Len + Seq + Payload
    let crc = hasher.finalize();
    out[0..4].copy_from_slice(&crc.to_le_bytes());

    Ok(total)
}

/// Attempt to decode a single record from the front of `buf`.
/// Returns (record, bytes_consumed) on success.
pub fn decode_record(buf: &[u8], prev_seq: Option<u64>) -> Result<(WalRecord, usize), WalError> {
    if buf.len() < HEADER_SIZE {
        return Err(WalError::EndOfLog);
    }

    // Fast check: an all-zero header means we've hit tail padding.
    // This is our EOL signal, distinct from corruption.
    if buf[..HEADER_SIZE].iter().all(|&b| b == 0) {
        return Err(WalError::EndOfLog);
    }

    let crc_stored = u32::from_le_bytes(buf[0..4].try_into().unwrap());
    let len = u32::from_le_bytes(buf[4..8].try_into().unwrap()) as usize;
    let seq = u64::from_le_bytes(buf[8..16].try_into().unwrap());

    if len > MAX_PAYLOAD_SIZE {
        // Header appears corrupt — treat as end of valid log.
        return Err(WalError::CorruptCrc { expected: 0, found: crc_stored, seq });
    }

    let total = HEADER_SIZE + len;
    if buf.len() < total {
        return Err(WalError::Truncated { needed: total, available: buf.len() });
    }

    let mut hasher = crc32fast::Hasher::new();
    hasher.update(&buf[4..total]);
    let crc_computed = hasher.finalize();

    if crc_computed != crc_stored {
        return Err(WalError::CorruptCrc {
            expected: crc_computed, found: crc_stored, seq,
        });
    }

    if let Some(p) = prev_seq {
        if seq <= p {
            return Err(WalError::SequenceRegression { prev: p, found: seq });
        }
    }

    Ok((WalRecord { seq, payload: buf[HEADER_SIZE..total].to_vec() }, total))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let payload = b"hello world";
        let mut buf = vec![0u8; HEADER_SIZE + payload.len()];
        let n = encode_record(42, payload, &mut buf).unwrap();
        assert_eq!(n, HEADER_SIZE + payload.len());

        let (rec, consumed) = decode_record(&buf, None).unwrap();
        assert_eq!(rec.seq, 42);
        assert_eq!(rec.payload, payload);
        assert_eq!(consumed, n);
    }

    #[test]
    fn crc_detects_corruption() {
        let mut buf = vec![0u8; HEADER_SIZE + 4];
        encode_record(1, b"data", &mut buf).unwrap();
        buf[HEADER_SIZE + 2] ^= 0xFF; // flip a bit in payload
        assert!(matches!(decode_record(&buf, None), Err(WalError::CorruptCrc { .. })));
    }

    #[test]
    fn zero_header_is_end_of_log() {
        let buf = vec![0u8; HEADER_SIZE];
        assert!(matches!(decode_record(&buf, None), Err(WalError::EndOfLog)));
    }

    #[test]
    fn sequence_regression_detected() {
        let mut buf = vec![0u8; HEADER_SIZE + 1];
        encode_record(5, b"x", &mut buf).unwrap();
        assert!(matches!(
            decode_record(&buf, Some(10)),
            Err(WalError::SequenceRegression { prev: 10, found: 5 })
        ));
    }
}