use std::sync::Arc;
use std::thread;
use tier2_concurrency::queue::{MPMCQueue, QueueError};

#[test]
fn test_mpmc_functional() {
    let queue: MPMCQueue<u32> = MPMCQueue::new(8);
    assert_eq!(queue.pop(), Err(QueueError::Empty));

    queue.push(10).unwrap();
    queue.push(20).unwrap();
    assert_eq!(queue.pop(), Ok(10));
    assert_eq!(queue.pop(), Ok(20));
    assert_eq!(queue.pop(), Err(QueueError::Empty));
}

#[test]
fn test_mpmc_full() {
    let queue: MPMCQueue<u32> = MPMCQueue::new(2);
    queue.push(1).unwrap();
    queue.push(2).unwrap();
    match queue.push(3) {
        Err((QueueError::Full, val)) => assert_eq!(val, 3),
        _ => panic!("Expected Full error"),
    }
}

#[test]
fn test_mpmc_wrap_around() {
    let queue: MPMCQueue<u32> = MPMCQueue::new(4);
    for lap in 0..100 {
        for i in 0..4 {
            queue.push(lap * 4 + i).unwrap();
        }
        for i in 0..4 {
            assert_eq!(queue.pop(), Ok(lap * 4 + i));
        }
    }
}

#[test]
fn test_mpmc_parallel_stress() {
    const NUM_PRODUCERS: usize = 4;
    const NUM_CONSUMERS: usize = 4;
    const ITEMS_PER_PRODUCER: usize = 10_000;

    let queue = Arc::new(MPMCQueue::<usize>::new(256));
    let mut producer_handles = Vec::new();
    let mut consumer_handles = Vec::new();

    for t in 0..NUM_PRODUCERS {
        let q = Arc::clone(&queue);
        producer_handles.push(thread::spawn(move || {
            for i in 0..ITEMS_PER_PRODUCER {
                let val = t * ITEMS_PER_PRODUCER + i;
                loop {
                    match q.push(val) {
                        Ok(()) => break,
                        Err((QueueError::Full, _)) => thread::yield_now(),
                        _ => unreachable!(),
                    }
                }
            }
        }));
    }

    let results = Arc::new(std::sync::Mutex::new(Vec::new()));
    let expected_total = NUM_PRODUCERS * ITEMS_PER_PRODUCER;
    let items_per_consumer = expected_total / NUM_CONSUMERS;

    for _ in 0..NUM_CONSUMERS {
        let q = Arc::clone(&queue);
        let res = Arc::clone(&results);
        consumer_handles.push(thread::spawn(move || {
            let mut local = Vec::with_capacity(items_per_consumer);
            for _ in 0..items_per_consumer {
                loop {
                    match q.pop() {
                        Ok(v) => {
                            local.push(v);
                            break;
                        }
                        Err(QueueError::Empty) => thread::yield_now(),
                        _ => unreachable!(),
                    }
                }
            }
            res.lock().unwrap().extend(local);
        }));
    }

    for h in producer_handles {
        h.join().unwrap();
    }
    for h in consumer_handles {
        h.join().unwrap();
    }

    let mut all_items = Arc::try_unwrap(results).unwrap().into_inner().unwrap();
    all_items.sort_unstable();

    let expected: Vec<usize> = (0..expected_total).collect();
    assert_eq!(all_items.len(), expected.len());
    assert_eq!(all_items, expected);
}
