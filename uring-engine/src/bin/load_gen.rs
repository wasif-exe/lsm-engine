use std::io::Write;
use std::net::TcpStream;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

fn main() {
    println!("============================================================");
    println!(" HIGH-THROUGHPUT CONCURRENT LOAD GENERATOR (FLOW-CONTROLLED)");
    println!(" Target: 127.0.0.1:8080 | Pipelined TCP Streams");
    println!("============================================================");

    let num_threads = 4;
    let connections_per_thread = 2;
    let pipeline_depth = 16; 

    let is_running = Arc::new(AtomicBool::new(true));
    let total_ops = Arc::new(AtomicU64::new(0));

    let mut handles = Vec::new();

    for t in 0..num_threads {
        let running = is_running.clone();
        let ops = total_ops.clone();

        let handle = thread::spawn(move || {
            let mut streams: Vec<TcpStream> = Vec::new();

            for _ in 0..connections_per_thread {
                if let Ok(stream) = TcpStream::connect("127.0.0.1:8080") {
                    stream.set_nodelay(true).unwrap();
                    streams.push(stream);
                }
            }

            if streams.is_empty() {
                return;
            }

            let mut batch_buf = Vec::with_capacity(pipeline_depth * 17);
            for i in 0..pipeline_depth {
                let key = (t * 10_000_000 + i) as u64;
                let val = key * 2;
                batch_buf.push(1u8); // Cmd = 1 (Write)
                batch_buf.extend_from_slice(&key.to_be_bytes());
                batch_buf.extend_from_slice(&val.to_be_bytes());
            }

            while running.load(Ordering::Relaxed) {
                let mut active_streams = 0;
                for stream in &mut streams {
                    if stream.write_all(&batch_buf).is_ok() {
                        ops.fetch_add(pipeline_depth as u64, Ordering::Relaxed);
                        active_streams += 1;
                    }
                }
                
                if active_streams == 0 {
                    // All connections dropped by server -> exit thread cleanly
                    break;
                }
                // Yield thread briefly to prevent overwhelming the socket buffers
                thread::sleep(Duration::from_micros(200));
            }
        });

        handles.push(handle);
    }

    let test_duration_secs = 10;
    println!("Running stress test for {} seconds...", test_duration_secs);

    let start_time = Instant::now();
    let mut last_ops = 0u64;

    for sec in 1..=test_duration_secs {
        thread::sleep(Duration::from_secs(1));
        let current_ops = total_ops.load(Ordering::Relaxed);
        let delta = current_ops - last_ops;
        println!("  [{:>2}s] Throughput: {:>9} ops/sec | Total: {}", sec, delta, current_ops);
        last_ops = current_ops;
    }

    is_running.store(false, Ordering::Relaxed);
    for h in handles {
        let _ = h.join();
    }

    let elapsed = start_time.elapsed().as_secs_f64();
    let final_ops = total_ops.load(Ordering::Relaxed);
    println!("============================================================");
    println!(" FINAL RESULT: {:.2} req/sec across {}s", (final_ops as f64) / elapsed, elapsed);
    println!("============================================================\n");
}