use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender, Receiver};

use crate::wal::WalWriter;
use super::skiplist::ConcurrentSkipList;

pub const MEMTABLE_LIMIT: usize = 64 * 1024 * 1024;

pub struct FlushTask {
    pub memtable: Arc<ConcurrentSkipList>,
    pub wal_path: std::path::PathBuf,
}

pub struct MemTableManager {
    active_mem: Arc<ConcurrentSkipList>,
    imm_mem: Option<Arc<ConcurrentSkipList>>,
    wal: WalWriter,
    wal_path: std::path::PathBuf,
    flush_tx: Sender<FlushTask>,
    is_flushing: Arc<AtomicBool>,
}

impl MemTableManager {
    pub fn new<P: AsRef<std::path::Path>>(wal_path: P) -> (Self, Receiver<FlushTask>) {
        let wal = WalWriter::open(&wal_path).expect("failed to open WAL");
        let (flush_tx, flush_rx) = mpsc::channel();
        (
            Self {
                active_mem: Arc::new(ConcurrentSkipList::new()),
                imm_mem: None,
                wal,
                wal_path: wal_path.as_ref().to_path_buf(),
                flush_tx,
                is_flushing: Arc::new(AtomicBool::new(false)),
            },
            flush_rx,
        )
    }

    pub fn write(&mut self, key: &[u8], value: Option<&[u8]>, seq: u64) -> Result<(), crate::wal::WalError> {
        let payload_len = key.len() + value.map_or(0, |v| v.len()) + 5;
        let mut payload = Vec::with_capacity(payload_len);
        payload.push(if value.is_some() { 1u8 } else { 0u8 });
        payload.extend_from_slice(&(key.len() as u32).to_le_bytes());
        payload.extend_from_slice(key);
        if let Some(v) = value {
            payload.extend_from_slice(v);
        }

        self.wal.append(&payload)?;

        let val_vec = value.map(|v| v.to_vec());
        self.active_mem.insert(key.to_vec(), val_vec, seq);

        if self.active_mem.approximate_size() >= MEMTABLE_LIMIT {
            self.maybe_freeze();
        }

        Ok(())
    }

    pub fn get(&self, key: &[u8]) -> Option<(Option<Vec<u8>>, u64)> {
        if let Some(res) = self.active_mem.get(key) {
            return Some(res);
        }
        if let Some(ref imm) = self.imm_mem {
            if let Some(res) = imm.get(key) {
                return Some(res);
            }
        }
        None
    }

    fn maybe_freeze(&mut self) {
        if self.imm_mem.is_some() {
            return;
        }

        self.wal.sync().ok();

        let old_mem = std::mem::replace(&mut self.active_mem, Arc::new(ConcurrentSkipList::new()));
        self.imm_mem = Some(old_mem.clone());

        let next_wal_path = self.wal_path.with_extension(format!("wal.{}", old_mem.iter().map(|(_, _, s)| s).max().unwrap_or(0)));
        let old_wal_path = std::mem::replace(&mut self.wal_path, next_wal_path);

        let new_wal = WalWriter::open(&self.wal_path).expect("failed to rotate WAL");
        let old_wal = std::mem::replace(&mut self.wal, new_wal);
        old_wal.truncate_to_logical_end().ok();

        let task = FlushTask {
            memtable: old_mem,
            wal_path: old_wal_path,
        };

        self.is_flushing.store(true, Ordering::Release);
        self.flush_tx.send(task).expect("flush channel broken");
    }

    pub fn clear_immutable(&mut self, completed_wal: &std::path::Path) {
        if self.imm_mem.is_some() {
            if completed_wal.exists() {
                std::fs::remove_file(completed_wal).ok();
            }
            self.imm_mem = None;
            self.is_flushing.store(false, Ordering::Release);
        }
    }
}