use std::sync::Arc;
use std::thread;
use tier2_concurrency::ebr::ThreadLocalLocal;
use tier2_concurrency::map::ConcurrentHashMap;

#[test]
fn test_map_insert_and_get() {
    let map = ConcurrentHashMap::new(16);
    let mut local = ThreadLocalLocal::new(map.collector());

    assert!(map.insert("A", 100, &mut local));
    assert!(map.insert("B", 200, &mut local));

    assert_eq!(map.get(&"A", &local, |v| *v), Some(100));
    assert_eq!(map.get(&"B", &local, |v| *v), Some(200));
    assert_eq!(map.get(&"C", &local, |v| *v), None);

    local.destroy(map.collector());
}

#[test]
fn test_map_remove() {
    let map = ConcurrentHashMap::new(16);
    let mut local = ThreadLocalLocal::new(map.collector());

    map.insert("A", 100, &mut local);
    assert_eq!(map.remove(&"A", &mut local), Some(100));
    assert_eq!(map.get(&"A", &local, |v| *v), None);

    local.destroy(map.collector());
}

#[test]
fn test_map_concurrent_readers() {
    let map = Arc::new(ConcurrentHashMap::new(128));
    let mut writer_local = ThreadLocalLocal::new(map.collector());

    map.insert(1, 100, &mut writer_local);

    let m_clone = Arc::clone(&map);
    let handle = thread::spawn(move || {
        let reader_local = ThreadLocalLocal::new(m_clone.collector());
        for _ in 0..1000 {
            let res = m_clone.get(&1, &reader_local, |v| *v);
            assert!(res == Some(100) || res == None);
        }
        reader_local.destroy(m_clone.collector());
    });

    for _ in 0..100 {
        map.remove(&1, &mut writer_local);
        map.insert(1, 100, &mut writer_local);
    }

    handle.join().unwrap();
    writer_local.destroy(map.collector());
}
