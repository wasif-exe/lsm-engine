use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::thread;
use tier2_concurrency::queue::{MPMCQueue, UnpaddedMPMCQueue};

const QUEUE_CAPACITY: usize = 1024;
const NUM_ITEMS: usize = 50_000;

fn run_padded_bench(threads: usize) {
    let queue = Arc::new(MPMCQueue::<usize>::new(QUEUE_CAPACITY));
    let mut handles = Vec::with_capacity(threads * 2);

    for _ in 0..threads {
        let q = Arc::clone(&queue);
        handles.push(thread::spawn(move || {
            for i in 0..NUM_ITEMS {
                let mut item = i;
                loop {
                    match q.push(item) {
                        Ok(()) => break,
                        Err((_, returned)) => {
                            item = returned;
                            thread::yield_now();
                        }
                    }
                }
            }
        }));
    }

    for _ in 0..threads {
        let q = Arc::clone(&queue);
        handles.push(thread::spawn(move || {
            for _ in 0..NUM_ITEMS {
                loop {
                    if let Ok(_) = q.pop() {
                        break;
                    }
                    thread::yield_now();
                }
            }
        }));
    }

    for h in handles {
        h.join().unwrap();
    }
}

fn run_unpadded_bench(threads: usize) {
    let queue = Arc::new(UnpaddedMPMCQueue::<usize>::new(QUEUE_CAPACITY));
    let mut handles = Vec::with_capacity(threads * 2);

    for _ in 0..threads {
        let q = Arc::clone(&queue);
        handles.push(thread::spawn(move || {
            for i in 0..NUM_ITEMS {
                let mut item = i;
                loop {
                    match q.push(item) {
                        Ok(()) => break,
                        Err((_, returned)) => {
                            item = returned;
                            thread::yield_now();
                        }
                    }
                }
            }
        }));
    }

    for _ in 0..threads {
        let q = Arc::clone(&queue);
        handles.push(thread::spawn(move || {
            for _ in 0..NUM_ITEMS {
                loop {
                    if let Ok(_) = q.pop() {
                        break;
                    }
                    thread::yield_now();
                }
            }
        }));
    }

    for h in handles {
        h.join().unwrap();
    }
}

fn run_mutex_bench(threads: usize) {
    let queue = Arc::new(Mutex::new(VecDeque::<usize>::with_capacity(QUEUE_CAPACITY)));
    let mut handles = Vec::with_capacity(threads * 2);

    for _ in 0..threads {
        let q = Arc::clone(&queue);
        handles.push(thread::spawn(move || {
            for i in 0..NUM_ITEMS {
                loop {
                    let mut lock = q.lock().unwrap();
                    if lock.len() < QUEUE_CAPACITY {
                        lock.push_back(i);
                        break;
                    }
                    drop(lock);
                    thread::yield_now();
                }
            }
        }));
    }

    for _ in 0..threads {
        let q = Arc::clone(&queue);
        handles.push(thread::spawn(move || {
            for _ in 0..NUM_ITEMS {
                loop {
                    let mut lock = q.lock().unwrap();
                    if let Some(_) = lock.pop_front() {
                        break;
                    }
                    drop(lock);
                    thread::yield_now();
                }
            }
        }));
    }

    for h in handles {
        h.join().unwrap();
    }
}

fn bench_queues(c: &mut Criterion) {
    let mut group = c.benchmark_group("Contended_Queue_Throughput");
    // Run benchmarks with active concurrent physical thread pairs
    for thread_count in [1, 2, 4].iter() {
        group.bench_with_input(
            BenchmarkId::new("MPMC_Padded", thread_count),
            thread_count,
            |b, &tc| b.iter(|| run_padded_bench(tc)),
        );
        group.bench_with_input(
            BenchmarkId::new("MPMC_Unpadded", thread_count),
            thread_count,
            |b, &tc| b.iter(|| run_unpadded_bench(tc)),
        );
        group.bench_with_input(
            BenchmarkId::new("Mutex_VecDeque", thread_count),
            thread_count,
            |b, &tc| b.iter(|| run_mutex_bench(tc)),
        );
    }
    group.finish();
}

criterion_group!(benches, bench_queues);
criterion_main!(benches);
