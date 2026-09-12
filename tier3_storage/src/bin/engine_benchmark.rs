use std::path::PathBuf;
use std::sync::atomic::AtomicU64;
use std::sync::Arc;
use std::time::Instant;
use tempfile::tempdir;

use tier3_storage::compaction::LeveledCompactor;
use tier3_storage::memtable::ConcurrentSkipList;
use tier3_storage::sstable::{SSTableReader, SSTableWriter};

fn main() {
    println!("============================================================");
    println!(" TIER 3 LSM-TREE PERFORMANCE & AMPLIFICATION BENCHMARK SUITE");
    println!("============================================================");

    let dir = tempdir().unwrap();
    let db_path = dir.path().to_path_buf();
    let sst_id_gen = Arc::new(AtomicU64::new(1));
    let mut compactor = LeveledCompactor::new(&db_path, sst_id_gen.clone());

    let total_operations = 400_000;
    let keys_per_sstable = 10_000;
    let num_tables = total_operations / keys_per_sstable;

    println!("\n[1/3] Benchmarking In-Memory MemTable Concurrent Writes...");
    let mem = Arc::new(ConcurrentSkipList::new());
    let write_start = Instant::now();

    for i in 0..total_operations {
        let key = format!("k_{:08}", i).into_bytes();
        let val = format!("v_{:08}", i).into_bytes();
        mem.insert(key, Some(val), i as u64);
    }

    let write_duration = write_start.elapsed();
    let write_throughput = (total_operations as f64) / write_duration.as_secs_f64();
    println!("  -> MemTable Insert: {:.2} ops/sec ({:.2?} total)", write_throughput, write_duration);

    println!("\n[2/3] Simulating L0 Flushes & Multi-Way Leveled Compaction...");
    let mut total_bytes_written: u64 = 0;
    let compaction_start = Instant::now();

    for t in 0..num_tables {
        let sst_path = db_path.join(format!("{:06}_L0.sst", t));
        let mut writer = SSTableWriter::create(&sst_path).unwrap();

        let start_k = t * keys_per_sstable;
        let end_k = start_k + keys_per_sstable;

        for i in start_k..end_k {
            let key = format!("k_{:08}", i).into_bytes();
            let val = format!("v_{:08}", i).into_bytes();
            writer.append(&key, Some(&val), i as u64).unwrap();
        }

        let written_bytes = writer.finish().unwrap();
        total_bytes_written += written_bytes;

        let reader = SSTableReader::open(&sst_path).unwrap();
        let smallest = reader.get_first_key().unwrap();
        let largest = reader.get_last_key().unwrap();

        compactor.add_l0_table(sst_path, smallest, largest);
        compactor.maybe_schedule_compaction();
    }

    let compaction_duration = compaction_start.elapsed();
    println!("  -> Compaction Completed: {:.2?} total", compaction_duration);
    println!("  -> L1 Tables Active: {}", compactor.l1.len());
    println!("  -> Remaining L0 Tables: {}", compactor.l0.len());

    println!("\n[3/3] Point Lookup Speed & Bloom Filter Rejection Benchmark...");
    let mut readers: Vec<SSTableReader> = Vec::new();
    for meta in &compactor.l1 {
        readers.push(SSTableReader::open(&meta.path).unwrap());
    }

    let read_start = Instant::now();
    let mut hits = 0;

    for i in 0..total_operations {
        let key = format!("k_{:08}", i).into_bytes();
        for r in &readers {
            if let Some((_, _)) = r.get(&key) {
                hits += 1;
                break;
            }
        }
    }

    let read_duration = read_start.elapsed();
    let read_throughput = (total_operations as f64) / read_duration.as_secs_f64();
    println!("  -> Point Reads Hit: {}/{}", hits, total_operations);
    println!("  -> Point Read Throughput: {:.2} ops/sec ({:.2?} total)", read_throughput, read_duration);

    let logical_data_size = total_operations * (10 + 10);
    let waf = (total_bytes_written as f64) / (logical_data_size as f64);
    println!("\n------------------------------------------------------------");
    println!(" AMPLIFICATION METRICS");
    println!("------------------------------------------------------------");
    println!("  -> Write Amplification Factor (WAF): {:.2}x", waf);
    println!("============================================================\n");
}