#[cfg(loom)]
mod loom_tests {
    use loom::thread;
    use std::sync::Arc;
    use tier2_concurrency::queue::MPMCQueue;

    #[test]
    fn loom_mpmc_bounded_2_producers_2_consumers() {
        loom::model(|| {
            let queue = Arc::new(MPMCQueue::<usize>::new(2));
            let mut handles = Vec::new();

            for p in 0..2 {
                let q = Arc::clone(&queue);
                handles.push(thread::spawn(move || {
                    let val = (p + 1) * 10;
                    let _ = q.push(val);
                }));
            }

            let popped_results = Arc::new(std::sync::Mutex::new(Vec::new()));
            for _ in 0..2 {
                let q = Arc::clone(&queue);
                let res = Arc::clone(&popped_results);
                handles.push(thread::spawn(move || {
                    if let Ok(val) = q.pop() {
                        res.lock().unwrap().push(val);
                    }
                }));
            }

            for h in handles {
                h.join().unwrap();
            }

            let results = popped_results.lock().unwrap();
            for &val in results.iter() {
                assert!(val == 10 || val == 20, "Corrupted value: {}", val);
            }
        });
    }

    #[test]
    fn loom_mpmc_wrap_around() {
        loom::model(|| {
            let queue = MPMCQueue::<usize>::new(2);

            queue.push(100).unwrap();
            queue.push(200).unwrap();
            assert_eq!(queue.pop(), Ok(100));

            queue.push(300).unwrap();
            assert_eq!(queue.pop(), Ok(200));
            assert_eq!(queue.pop(), Ok(300));
        });
    }
}
