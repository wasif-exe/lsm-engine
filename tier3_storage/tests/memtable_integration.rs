use tier3_storage::memtable::skiplist::ConcurrentSkipList;
use tier3_storage::memtable::manager::MemTableManager;
use std::sync::Arc;
use std::thread;
use tempfile::tempdir;

#[test]
fn test_skiplist_concurrency() {
    let list = Arc::new(ConcurrentSkipList::new());
    let mut threads = vec![];

    for t in 0..8 {
        let list_clone = list.clone();
        threads.push(thread::spawn(move || {
            for i in 0..1000 {
                let key = format!("thread-{}-key-{}", t, i).into_bytes();
                let val = format!("val-{}", i).into_bytes();
                list_clone.insert(key, Some(val), (t * 1000 + i) as u64);
            }
        }));
    }

    for t in threads {
        t.join().unwrap();
    }

    for t in 0..8 {
        for i in 0..1000 {
            let key = format!("thread-{}-key-{}", t, i).into_bytes();
            let res = list.get(&key);
            assert!(res.is_some());
            let (val, seq) = res.unwrap();
            assert_eq!(val, Some(format!("val-{}", i).into_bytes()));
            assert_eq!(seq, (t * 1000 + i) as u64);
        }
    }
}

#[test]
fn test_memtable_freeze_trigger() {
    let dir = tempdir().unwrap();
    let wal_path = dir.path().join("active.wal");

    let (mut manager, rx) = MemTableManager::new(&wal_path);

    let kv_size = 1000;
    let iterations = (tier3_storage::memtable::MEMTABLE_LIMIT / kv_size) + 500;

    for i in 0..iterations {
        let key = format!("key-{:08}", i).into_bytes();
        let val = vec![0u8; kv_size];
        manager.write(&key, Some(&val), i as u64).unwrap();
    }

    let task = rx.recv_timeout(std::time::Duration::from_secs(5));
    assert!(task.is_ok());
    let flush_task = task.unwrap();
    assert!(flush_task.memtable.approximate_size() >= tier3_storage::memtable::MEMTABLE_LIMIT);

    manager.clear_immutable(&flush_task.wal_path);
}