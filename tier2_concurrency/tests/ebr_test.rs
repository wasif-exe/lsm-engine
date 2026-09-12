use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;
use tier2_concurrency::ebr::{Collector, ThreadLocalLocal};

struct TestNode {
    dropped_counter: Arc<AtomicUsize>,
}

impl Drop for TestNode {
    fn drop(&mut self) {
        self.dropped_counter.fetch_add(1, Ordering::SeqCst);
    }
}

unsafe fn test_node_dtor(ptr: *mut TestNode) {
    let _ = Box::from_raw(ptr);
}

#[test]
fn test_ebr_deferred_destruction() {
    let collector = Box::new(Collector::new());
    let mut local = ThreadLocalLocal::new(&collector);
    let drop_count = Arc::new(AtomicUsize::new(0));

    let node = Box::into_raw(Box::new(TestNode {
        dropped_counter: Arc::clone(&drop_count),
    }));

    let guard = collector.pin(local.thread_index());
    unsafe { local.defer(&collector, node, test_node_dtor); }
    assert_eq!(drop_count.load(Ordering::SeqCst), 0);
    drop(guard);

    local.flush(&collector);
    local.flush(&collector);

    assert_eq!(drop_count.load(Ordering::SeqCst), 1);
    local.destroy(&collector);
}

#[test]
fn test_ebr_concurrent_readers_block_destruction() {
    let collector = Arc::new(Collector::new());
    let drop_count = Arc::new(AtomicUsize::new(0));

    let mut writer_local = ThreadLocalLocal::new(&collector);

    let node = Box::into_raw(Box::new(TestNode {
        dropped_counter: Arc::clone(&drop_count),
    }));

    let c_clone = Arc::clone(&collector);
    let barrier_pinned = Arc::new(std::sync::Barrier::new(2));
    let barrier_release = Arc::new(std::sync::Barrier::new(2));
    let b_pinned = Arc::clone(&barrier_pinned);
    let b_release = Arc::clone(&barrier_release);

    let reader_handle = thread::spawn(move || {
        let reader_local = ThreadLocalLocal::new(&c_clone);
        let guard = c_clone.pin(reader_local.thread_index());
        b_pinned.wait();
        b_release.wait();
        drop(guard);
        reader_local.destroy(&c_clone);
    });

    barrier_pinned.wait();

    unsafe { writer_local.defer(&collector, node, test_node_dtor); }

    writer_local.flush(&collector);
    writer_local.flush(&collector);

    assert_eq!(
        drop_count.load(Ordering::SeqCst),
        0,
        "Node should NOT be reclaimed while reader is pinned to old epoch"
    );

    barrier_release.wait();
    reader_handle.join().unwrap();

    writer_local.flush(&collector);
    writer_local.flush(&collector);

    assert_eq!(
        drop_count.load(Ordering::SeqCst),
        1,
        "Node MUST be reclaimed after reader unpins"
    );

    writer_local.destroy(&collector);
}
