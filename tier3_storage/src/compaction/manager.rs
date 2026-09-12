use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use super::merge::StreamingKWayMerge;
use crate::sstable::reader::SSTableReader;

pub const L0_TRIGGER_COUNT: usize = 4;

#[derive(Clone, Debug)]
pub struct TableMetadata {
    pub id: u64,
    pub path: PathBuf,
    pub smallest_key: Vec<u8>,
    pub largest_key: Vec<u8>,
}

pub struct LeveledCompactor {
    db_dir: PathBuf,
    pub l0: Vec<TableMetadata>,
    pub l1: Vec<TableMetadata>,
    next_sst_id: Arc<AtomicU64>,
}

impl LeveledCompactor {
    pub fn new<P: AsRef<Path>>(db_dir: P, next_sst_id: Arc<AtomicU64>) -> Self {
        Self {
            db_dir: db_dir.as_ref().to_path_buf(),
            l0: Vec::new(),
            l1: Vec::new(),
            next_sst_id,
        }
    }

    pub fn add_l0_table(&mut self, path: PathBuf, smallest_key: Vec<u8>, largest_key: Vec<u8>) {
        let id = self.next_sst_id.fetch_add(1, Ordering::SeqCst);
        self.l0.push(TableMetadata {
            id,
            path,
            smallest_key,
            largest_key,
        });
    }

    pub fn maybe_schedule_compaction(&mut self) -> Option<()> {
        if self.l0.len() < L0_TRIGGER_COUNT {
            return None;
        }

        // Collect all L0 files
        let l0_files: Vec<PathBuf> = self.l0.iter().map(|m| m.path.clone()).collect();

        // Calculate overarching key range across all L0
        let mut min_k: Option<Vec<u8>> = None;
        let mut max_k: Option<Vec<u8>> = None;

        for m in &self.l0 {
            min_k = Some(match min_k {
                Some(k) if k < m.smallest_key => k,
                _ => m.smallest_key.clone(),
            });
            max_k = Some(match max_k {
                Some(k) if k > m.largest_key => k,
                _ => m.largest_key.clone(),
            });
        }

        let min_k = min_k.unwrap();
        let max_k = max_k.unwrap();

        // Find overlapping L1 files
        let mut overlapping_l1_paths = Vec::new();
        let mut remaining_l1 = Vec::new();

        for m in self.l1.drain(..) {
            if m.largest_key < min_k || m.smallest_key > max_k {
                remaining_l1.push(m);
            } else {
                overlapping_l1_paths.push(m.path);
            }
        }

        let mut merge_inputs = l0_files.clone();
        merge_inputs.extend(overlapping_l1_paths.clone());

        let out_id = self.next_sst_id.fetch_add(1, Ordering::SeqCst);
        let out_path = self.db_dir.join(format!("{:06}_L1.sst", out_id));

        StreamingKWayMerge::merge(&merge_inputs, &out_path, true).expect("compaction merge failed");

        // Open newly created L1 SSTable to inspect full key boundaries
        let reader = SSTableReader::open(&out_path).expect("failed to open compacted sst");
        let smallest = reader.get_first_key().unwrap_or_default();
        let largest = reader.get_last_key().unwrap_or_default();

        remaining_l1.push(TableMetadata {
            id: out_id,
            path: out_path,
            smallest_key: smallest,
            largest_key: largest,
        });

        // Sort L1 monotonically by key range
        remaining_l1.sort_by(|a, b| a.smallest_key.cmp(&b.smallest_key));
        self.l1 = remaining_l1;

        // Delete compacted source files from disk
        for p in merge_inputs {
            fs::remove_file(p).ok();
        }

        self.l0.clear();
        Some(())
    }
}