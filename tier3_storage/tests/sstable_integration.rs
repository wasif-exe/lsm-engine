use tier3_storage::sstable::{SSTableWriter, SSTableReader};
use tempfile::tempdir;

#[test]
fn test_sstable_build_and_read() {
    let dir = tempdir().unwrap();
    let sst_path = dir.path().join("000001.sst");

    let mut writer = SSTableWriter::create(&sst_path).unwrap();
    let count = 5000;

    for i in 0..count {
        let key = format!("key-{:06}", i).into_bytes();
        let val = format!("val-{:06}", i).into_bytes();
        writer.append(&key, Some(&val), i as u64).unwrap();
    }
    writer.finish().unwrap(); // Correct zero-argument call

    let reader = SSTableReader::open(&sst_path).unwrap();

    for i in 0..count {
        let key = format!("key-{:06}", i).into_bytes();
        let res = reader.get(&key);
        assert!(res.is_some(), "Key not found: key-{:06}", i);
        let (val, seq) = res.unwrap();
        assert_eq!(val, Some(format!("val-{:06}", i).into_bytes()));
        assert_eq!(seq, i as u64);
    }

    assert!(reader.get(b"non_existing_key").is_none());
}

#[test]
fn test_sstable_tombstones() {
    let dir = tempdir().unwrap();
    let sst_path = dir.path().join("000002.sst");

    let mut writer = SSTableWriter::create(&sst_path).unwrap();
    writer.append(b"k1", Some(b"v1"), 1).unwrap();
    writer.append(b"k2", None, 2).unwrap();
    writer.append(b"k3", Some(b"v3"), 3).unwrap();
    writer.finish().unwrap(); // Correct zero-argument call

    let reader = SSTableReader::open(&sst_path).unwrap();
    assert_eq!(reader.get(b"k1"), Some((Some(b"v1".to_vec()), 1)));
    assert_eq!(reader.get(b"k2"), Some((None, 2)));
    assert_eq!(reader.get(b"k3"), Some((Some(b"v3".to_vec()), 3)));
}