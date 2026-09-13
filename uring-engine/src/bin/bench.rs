
use std::io::{Read, Write};
use std::net::TcpStream;
use std::thread;
use std::time::{Duration, Instant};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

const CONCURRENT_CONNS: usize = 200;
const DURATION_SECS: u64 = 10;
const PAYLOAD: &[u8] = b"PING_PAYLOAD_32_BYTES_RECORD_OK!";

fn main() {
    println!("Connecting {} workers over TCP for {}s...", CONCURRENT_CONNS, DURATION_SECS);

    let running = Arc::new(AtomicBool::new(true));
    let total_ops = Arc::new(AtomicU64::new(0));

    let mut handles = Vec::new();

    let start = Instant::now();

    for _ in 0..CONCURRENT_CONNS {
        let running_clone = Arc::clone(&running);
        let ops_clone = Arc::clone(&total_ops);

        handles.push(thread::spawn(move || {
            let mut stream = match TcpStream::connect("127.0.0.1:8080") {
                Ok(s) => s,
                Err(_) => return,
            };
            stream.set_nodelay(true).unwrap();

            let mut buf = [0u8; 64];

            while running_clone.load(Ordering::Relaxed) {
                if stream.write_all(PAYLOAD).is_err() { break; }
                if stream.read_exact(&mut buf[..PAYLOAD.len()]).is_err() { break; }
                ops_clone.fetch_add(1, Ordering::Relaxed);
            }
        }));
    }

    thread::sleep(Duration::from_secs(DURATION_SECS));
    running.store(false, Ordering::SeqCst);

    for h in handles {
        let _ = h.join();
    }

    let elapsed = start.elapsed().as_secs_f64();
    let ops = total_ops.load(Ordering::SeqCst);
    println!("========================================");
    println!("Benchmark Results:");
    println!("  Total Requests:  {}", ops);
    println!("  Throughput:      {:.2} req/sec", (ops as f64) / elapsed);
    println!("========================================");
}