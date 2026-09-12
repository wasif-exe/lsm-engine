#[cfg(loom)]
mod map_loom_tests {
    use loom::thread;
    use std::sync::Arc;
    use tier2_concurrency::ebr::ThreadLocalLocal;
    use tier2_concurrency::map::ConcurrentHashMap;

    #[test]
    fn loom_map_race() {
        loom::model(|| {
            let map = Arc::new(ConcurrentHashMap::<usize, usize>::new(4));

            let m1 = Arc::clone(&map);
            let h1 = thread::spawn(move || {
                let mut local = ThreadLocalLocal::new(m1.collector());
                m1.insert(1, 100, &mut local);
                local.destroy(m1.collector());
            });

            let m2 = Arc::clone(&map);
            let h2 = thread::spawn(move || {
                let mut local = ThreadLocalLocal::new(m2.collector());
                let _ = m2.get(&1, &local, |v| *v);
                local.destroy(m2.collector());
            });

            h1.join().unwrap();
            h2.join().unwrap();
        });
    }
}
