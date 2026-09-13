use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicU64;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use std::thread;
use tier2_concurrency::ebr::Collector;
use tier2_concurrency::queue::{MPMCQueue, QueueError};
use crate::compaction::LeveledCompactor;
use crate::memtable::manager::FlushTask;
use crate::memtable::skiplist::ConcurrentSkipList;
use crate::sstable::reader::SSTableReader;
use crate::wal::{WalError, WalWriter};

pub struct EngineNode {
    active_mem: Arc<ConcurrentSkipList>,
    imm_mem: Option<Arc<ConcurrentSkipList>>,
    wal: WalWriter,
    wal_path: PathBuf,
    _db_dir: PathBuf,
    collector: Arc<Collector>,
    flush_queue: Arc<MPMCQueue<FlushTask>>,
    compactor: Arc<RwLock<LeveledCompactor>>,
    _next_sst_id: Arc<AtomicU64>,
    is_running: Arc<AtomicBool>,
    bg_workers: Vec<thread::JoinHandle<()>>,
}

impl EngineNode {
    pub fn open<P: AsRef<Path>>(db_dir: P) -> io::Result<Self> {
        let db_dir = db_dir.as_ref().to_path_buf();
        fs::create_dir_all(&db_dir)?;

        let wal_path = db_dir.join("active.wal");
        let wal = WalWriter::open(&wal_path)?;

        let collector = Arc::new(Collector::new());
        let flush_queue = Arc::new(MPMCQueue::<FlushTask>::new(1024));

        let next_sst_id = Arc::new(AtomicU64::new(1));
        let compactor = Arc::new(RwLock::new(LeveledCompactor::new(&db_dir, next_sst_id.clone())));

        let is_running = Arc::new(AtomicBool::new(true));
        let mut bg_workers = Vec::new();
        for _ in 0..2 {
            let queue = flush_queue.clone();
            let running = is_running.clone();
            let compactor_clone = compactor.clone();
            let collector_clone = collector.clone();

            let handle = thread::spawn(move || {
                let thread_idx = collector_clone.register();

                while running.load(Ordering::Relaxed) {
                    match queue.pop() {
                        Ok(task) => {
                            let sst_id = task.wal_path.file_name()
                                .and_then(|f| f.to_str())
                                .and_then(|s| s.strip_prefix("wal."))
                                .and_then(|s| s.parse::<u64>().ok())
                                .unwrap_or(0);

                            let sst_path = task.wal_path.with_extension(format!("sst.{}", sst_id));
                            let mut writer = crate::sstable::writer::SSTableWriter::create(&sst_path).unwrap();

                            let mut iter = task.memtable.iter();
                            while let Some((k, v, seq)) = iter.next() {
                                writer.append(&k, v.as_deref(), seq).unwrap();
                            }
                            writer.finish().unwrap();
                            let reader = SSTableReader::open(&sst_path).unwrap();
                            let smallest = reader.get_first_key().unwrap();
                            let largest = reader.get_last_key().unwrap();
                            {
                                let mut guard = compactor_clone.write().unwrap();
                                guard.add_l0_table(sst_path, smallest, largest);
                                guard.maybe_schedule_compaction();
                            }

                            if task.wal_path.exists() {
                                let _ = fs::remove_file(task.wal_path);
                            }
                        }
                        Err(QueueError::Empty) => {
                            thread::park_timeout(std::time::Duration::from_millis(10));
                        }
                        _ => {}
                    }
                }

                collector_clone.unregister(thread_idx);
            });
            bg_workers.push(handle);
        }

        Ok(Self {
            active_mem: Arc::new(ConcurrentSkipList::new()),
            imm_mem: None,
            wal,
            wal_path,
            _db_dir: db_dir,
            collector,
            flush_queue,
            compactor,
            _next_sst_id: next_sst_id,
            is_running,
            bg_workers,
        })
    }

    pub fn put(&mut self, key: &[u8], value: &[u8], seq: u64, thread_idx: usize) -> Result<(), WalError> {
        let collector = self.collector.clone();
        let _guard = collector.pin(thread_idx);
        self.write_internal(key, Some(value), seq)
    }

    pub fn delete(&mut self, key: &[u8], seq: u64, thread_idx: usize) -> Result<(), WalError> {
        let collector = self.collector.clone();
        let _guard = collector.pin(thread_idx);
        self.write_internal(key, None, seq)
    }

    fn write_internal(&mut self, key: &[u8], value: Option<&[u8]>, seq: u64) -> Result<(), WalError> {
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

        if self.active_mem.approximate_size() >= crate::memtable::manager::MEMTABLE_LIMIT {
            self.freeze_active_memtable();
        }

        Ok(())
    }

    pub fn get(&self, key: &[u8], thread_idx: usize) -> Option<(Option<Vec<u8>>, u64)> {
        let collector = self.collector.clone();
        let _guard = collector.pin(thread_idx);

        if let Some(res) = self.active_mem.get(key) {
            return Some(res);
        }

        if let Some(ref imm) = self.imm_mem {
            if let Some(res) = imm.get(key) {
                return Some(res);
            }
        }

        let compactor_guard = self.compactor.read().unwrap();

        for table in compactor_guard.l0.iter().rev() {
            if key >= &table.smallest_key && key <= &table.largest_key {
                if let Ok(reader) = SSTableReader::open(&table.path) {
                    if let Some(res) = reader.get(key) {
                        return Some(res);
                    }
                }
            }
        }


        let target_idx = match compactor_guard.l1.binary_search_by(|m| m.smallest_key.as_slice().cmp(key)) {
            Ok(idx) => Some(idx),
            Err(idx) if idx > 0 => Some(idx - 1),
            _ => None,
        };

        if let Some(idx) = target_idx {
            if idx < compactor_guard.l1.len() {
                let table = &compactor_guard.l1[idx];
                if key <= &table.largest_key {
                    if let Ok(reader) = SSTableReader::open(&table.path) {
                        if let Some(res) = reader.get(key) {
                            return Some(res);
                        }
                    }
                }
            }
        }

        None
    }

    fn freeze_active_memtable(&mut self) {
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


        if let Err((_err, _discarded_task)) = self.flush_queue.push(task) {
            eprintln!("Flush queue full, dropping task!");
        }
    }

    pub fn collector(&self) -> &Collector {
        &self.collector
    }

    pub fn sync(&self) -> io::Result<()> {
        self.wal.sync()
    }
}

impl Drop for EngineNode {
    fn drop(&mut self) {
        self.is_running.store(false, Ordering::Relaxed);
        for bg in self.bg_workers.drain(..) {
            bg.thread().unpark();
            let _ = bg.join();
        }
    }
}