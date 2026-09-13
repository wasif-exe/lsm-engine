use tier3_storage::{WalWriter, RecoveryScanner};
use tempfile::tempdir;

#[test]
fn append_and_recover_single_record() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("wal.log");

    {
        let mut w = WalWriter::open(&path).unwrap();
        let seq = w.append(b"hello").unwrap();
        assert_eq!(seq, 1);
        w.sync().unwrap();
    }

    let (records, _) = RecoveryScanner::open(&path).unwrap().replay_all();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].seq, 1);
    assert_eq!(records[0].payload, b"hello");
}

#[test]
fn append_many_and_recover_all() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("wal.log");

    {
        let mut w = WalWriter::open(&path).unwrap();
        for i in 0..1000 {
            let payload = format!("record-{}", i);
            w.append(payload.as_bytes()).unwrap();
        }
        w.sync().unwrap();
    }

    let (records, _) = RecoveryScanner::open(&path).unwrap().replay_all();
    assert_eq!(records.len(), 1000);
    for (i, rec) in records.iter().enumerate() {
        assert_eq!(rec.seq, (i + 1) as u64);
        assert_eq!(rec.payload, format!("record-{}", i).as_bytes());
    }
}

#[test]
fn crash_simulation_torn_tail_recovers_valid_prefix() {
    use std::fs::OpenOptions;

    let dir = tempdir().unwrap();
    let path = dir.path().join("wal.log");
    {
        let mut w = WalWriter::open(&path).unwrap();
        for i in 0..100 {
            w.append(format!("rec-{}", i).as_bytes()).unwrap();
        }
        w.sync().unwrap();
    }

    let f = OpenOptions::new().write(true).open(&path).unwrap();
    let len = f.metadata().unwrap().len();
    f.set_len(len - 37).unwrap();

    let (records, _truncate_to) = RecoveryScanner::open(&path).unwrap().replay_all();
    assert!(records.len() > 0);
    assert!(records.len() <= 100);
    for w in records.windows(2) {
        assert!(w[0].seq < w[1].seq);
    }
}

#[test]
fn crash_simulation_bitflip_stops_replay() {
    use std::fs::OpenOptions;
    use std::io::{Read, Seek, SeekFrom, Write};

    let dir = tempdir().unwrap();
    let path = dir.path().join("wal.log");

    {
        let mut w = WalWriter::open(&path).unwrap();
        for i in 0..50 {
            w.append(format!("rec-{}", i).as_bytes()).unwrap();
        }
        w.sync().unwrap();
    }

    let mut f = OpenOptions::new().read(true).write(true).open(&path).unwrap();
    f.seek(SeekFrom::Start(80)).unwrap();
    let mut b = [0u8; 1];
    f.read_exact(&mut b).unwrap();
    b[0] ^= 0x01;
    f.seek(SeekFrom::Start(80)).unwrap();
    f.write_all(&b).unwrap();
    f.sync_all().unwrap();

    let (records, _) = RecoveryScanner::open(&path).unwrap().replay_all();
    assert!(
        records.len() < 50,
        "corrupt record must stop replay early, got {}",
        records.len()
    );
    assert!(
        records.len() > 0,
        "records before the corruption must survive"
    );
}