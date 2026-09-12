use tier3_storage::bloom::{BloomBuilder, BloomFilter};
use tier3_storage::sstable::{SSTableWriter, SSTableReader};
use tempfile::tempdir;

#[test]
fn test_bloom_zero_false_negatives() {
    let mut builder = BloomBuilder::new();
    let count = 10_000;

    for i in 0..count {
        let key = format!("key-{:08}", i);
        builder.add(key.as_bytes());
    }

    let filter_data = builder.build();
    let filter = BloomFilter::new(&filter_data).unwrap();

    for i in 0..count {
        let key = format!("key-{:08}", i);
        assert!(filter.contains(key.as_bytes()), "False negative detected for {}", key);
    }
}

#[test]
fn test_bloom_false_positive_rate() {
    let mut builder = BloomBuilder::new();
    let count = 10_000;

    for i in 0..count {
        let key = format!("existing-{:08}", i);
        builder.add(key.as_bytes());
    }

    let filter_data = builder.build();
    let filter = BloomFilter::new(&filter_data).unwrap();

    let mut false_positives = 0;
    let test_count = 100_000;

    for i in 0..test_count {
        let key = format!("missing-{:08}", i);
        if filter.contains(key.as_bytes()) {
            false_positives += 1;
        }
    }

    let fpr = (false_positives as f64) / (test_count as f64);
    assert!(fpr < 0.02, "FPR too high: {:.4}%", fpr * 100.0);
}

#[test]
fn test_sstable_bloom_integration() {
    let dir = tempdir().unwrap();
    let sst_path = dir.path().join("bloom_test.sst");

    let mut writer = SSTableWriter::create(&sst_path).unwrap();
    for i in 0..1000 {
        let key = format!("k-{:04}", i).into_bytes();
        let val = format!("v-{:04}", i).into_bytes();
        writer.append(&key, Some(&val), i as u64).unwrap();
    }
    writer.finish().unwrap();

    let reader = SSTableReader::open(&sst_path).unwrap();

    for i in 0..1000 {
        let key = format!("k-{:04}", i).into_bytes();
        assert!(reader.get(&key).is_some());
    }

    for i in 1000..2000 {
        let key = format!("k-{:04}", i).into_bytes();
        assert!(reader.get(&key).is_none());
    }
}