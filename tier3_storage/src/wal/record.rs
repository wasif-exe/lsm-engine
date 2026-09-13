use std::io;

pub const HEADER_SIZE: usize = 4 + 4 + 8;
pub const MAX_PAYLOAD_SIZE: usize = 16 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct WalRecord {
    pub seq: u64,
    pub payload: Vec<u8>,
}

#[derive(Debug)]
pub enum WalError {
    Io(io::Error),
    CorruptCrc { expected: u32, found: u32, seq: u64 },
    Truncated { needed: usize, available: usize },
    PayloadTooLarge(usize),
    SequenceRegression { prev: u64, found: u64 },
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

pub fn encode_record(seq: u64, payload: &[u8], out: &mut [u8]) -> Result<usize, WalError> {
    if payload.len() > MAX_PAYLOAD_SIZE {
        return Err(WalError::PayloadTooLarge(payload.len()));
    }
    let total = HEADER_SIZE + payload.len();
    if out.len() < total {
        return Err(WalError::Truncated { needed: total, available: out.len() });
    }

    let len = payload.len() as u32;

    out[4..8].copy_from_slice(&len.to_le_bytes());
    out[8..16].copy_from_slice(&seq.to_le_bytes());
    out[HEADER_SIZE..total].copy_from_slice(payload);

    let mut hasher = crc32fast::Hasher::new();
    hasher.update(&out[4..total]); 
    let crc = hasher.finalize();
    out[0..4].copy_from_slice(&crc.to_le_bytes());

    Ok(total)
}

pub fn decode_record(buf: &[u8], prev_seq: Option<u64>) -> Result<(WalRecord, usize), WalError> {
    if buf.len() < HEADER_SIZE {
        return Err(WalError::EndOfLog);
    }

    if buf[..HEADER_SIZE].iter().all(|&b| b == 0) {
        return Err(WalError::EndOfLog);
    }

    let crc_stored = u32::from_le_bytes(buf[0..4].try_into().unwrap());
    let len = u32::from_le_bytes(buf[4..8].try_into().unwrap()) as usize;
    let seq = u64::from_le_bytes(buf[8..16].try_into().unwrap());

    if len > MAX_PAYLOAD_SIZE {
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