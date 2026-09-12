#[cfg(loom)]
mod ebr_loom_tests {
    use loom::thread;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use tier2_concurrency::ebr::{Collector, ThreadLocalLocal};

    struct LoomTestNode {
        dropped: Arc<AtomicUsize>,
    }

    unsafe fn loom_node_dtor(ptr: *mut LoomTestNode) {
        let node = Box::from_raw(ptr);
        node.dropped.fetch_add(1, Ordering::SeqCst);
    }

    #[test]
    fn loom_ebr_race_test() {
        loom::model(|| {
            let collector: Arc<Collector> = Arc::new(Collector::new());
            let dropped = Arc::new(AtomicUsize::new(0));

            let c1 = Arc::clone(&collector);
            let h1 = thread::spawn(move || {
                let local = ThreadLocalLocal::new(&c1);
                {
                    let _guard = c1.pin(local.thread_index());
                }
                local.destroy(&c1);
            });

            let c2 = Arc::clone(&collector);
            let d2 = Arc::clone(&dropped);
            let h2 = thread::spawn(move || {
                let mut local = ThreadLocalLocal::new(&c2);
                let node = Box::into_raw(Box::new(LoomTestNode { dropped: d2 }));
                unsafe { local.defer(&c2, node, loom_node_dtor); }
                local.flush(&c2);
                local.destroy(&c2);
            });

            h1.join().unwrap();
            h2.join().unwrap();

            let mut parent_local = ThreadLocalLocal::new(&collector);
            parent_local.flush(&collector);
            parent_local.flush(&collector);
            parent_local.destroy(&collector);

            assert_eq!(dropped.load(Ordering::SeqCst), 1, "Memory leaked!");
        });
    }
}
