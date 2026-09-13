mod sys;
mod uring;
mod op;
mod buf_ring;

use std::io;
use std::os::unix::io::{AsRawFd, FromRawFd};
use std::net::TcpListener;
use std::thread;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use std::path::PathBuf;


use tier2_concurrency::queue::{MPMCQueue, QueueError};


use tier3_storage::EngineNode;

const BUF_RING_ENTRIES: u32 = 4096;
const BUF_SIZE: usize = 2048;
const BUFFER_GROUP_ID: u16 = 0;

pub enum InterCoreTask {
    ReplicateWrite { key: u64, val: u64 },
}

#[repr(align(64))]
pub struct WorkerStats {
    pub rx_bytes: AtomicU64,
    pub tx_bytes: AtomicU64,
    pub completed_requests: AtomicU64,
}

impl WorkerStats {
    pub const fn new() -> Self {
        Self {
            rx_bytes: AtomicU64::new(0),
            tx_bytes: AtomicU64::new(0),
            completed_requests: AtomicU64::new(0),
        }
    }
}

fn main() -> io::Result<()> {
    let num_cores = num_cpus();
    println!("============================================================");
    println!(" Thread-Per-Core io_uring Network Engine (Linux 6.x+)");
    println!(" Cores: {} | PBUF_RING: {}/core | Alignment: 64B", num_cores, BUF_RING_ENTRIES);
    println!("============================================================");

    let mut stats_vec = Vec::with_capacity(num_cores);
    for _ in 0..num_cores {
        stats_vec.push(WorkerStats::new());
    }
    let stats = Arc::new(stats_vec);

    let mut queues_vec = Vec::with_capacity(num_cores);
    for _ in 0..num_cores {
        queues_vec.push(Arc::new(MPMCQueue::<InterCoreTask>::new(2048)));
    }
    let queues = Arc::new(queues_vec);

    let db_root = PathBuf::from("./db_root");
    if db_root.exists() {
        let _ = std::fs::remove_dir_all(&db_root);
    }
    std::fs::create_dir_all(&db_root)?;

    let monitor_stats = Arc::clone(&stats);
    thread::spawn(move || {
        let mut last_reqs = 0u64;
        let mut last_rx_bytes = 0u64;
        loop {
            thread::sleep(Duration::from_secs(1));
            let mut total_reqs = 0u64;
            let mut total_rx = 0u64;
            for s in monitor_stats.iter() {
                total_reqs += s.completed_requests.load(Ordering::Relaxed);
                total_rx += s.rx_bytes.load(Ordering::Relaxed);
            }
            let delta_reqs = total_reqs.saturating_sub(last_reqs);
            let delta_mb = (total_rx.saturating_sub(last_rx_bytes) as f64) / (1024.0 * 1024.0);
            if delta_reqs > 0 {
                println!(
                    "[TELEMETRY] {:>9} req/sec | {:>7.2} MB/sec RX | Total Reqs: {}",
                    delta_reqs, delta_mb, total_reqs
                );
            }
            last_reqs = total_reqs;
            last_rx_bytes = total_rx;
        }
    });

    let mut handles = Vec::with_capacity(num_cores);

    for core_id in 0..num_cores {
        let stats_ref = Arc::clone(&stats);
        let queues_ref = Arc::clone(&queues);
        let core_db_dir = db_root.join(format!("core_{}", core_id));
        
        let handle = thread::spawn(move || {
            if let Err(e) = run_worker(core_id, stats_ref, queues_ref, core_db_dir) {
                eprintln!("[Worker {}] Exited with error: {:?}", core_id, e);
            }
        });
        handles.push(handle);
    }

    for handle in handles {
        handle.join().unwrap();
    }

    Ok(())
}

fn num_cpus() -> usize {
    unsafe { libc::sysconf(libc::_SC_NPROCESSORS_ONLN) as usize }
}

fn create_reuseport_listener(port: u16) -> io::Result<TcpListener> {
    unsafe {
        let fd = libc::socket(
            libc::AF_INET,
            libc::SOCK_STREAM | libc::SOCK_NONBLOCK | libc::SOCK_CLOEXEC,
            0,
        );
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }

        let optval: libc::c_int = 1;
        libc::setsockopt(fd, libc::SOL_SOCKET, libc::SO_REUSEADDR, &optval as *const _ as *const _, 4);
        libc::setsockopt(fd, libc::SOL_SOCKET, libc::SO_REUSEPORT, &optval as *const _ as *const _, 4);
        libc::setsockopt(fd, libc::IPPROTO_TCP, libc::TCP_NODELAY, &optval as *const _ as *const _, 4);

        let addr = libc::sockaddr_in {
            sin_family: libc::AF_INET as libc::sa_family_t,
            sin_port: port.to_be(),
            sin_addr: libc::in_addr { s_addr: libc::INADDR_ANY.to_be() },
            ..std::mem::zeroed()
        };

        if libc::bind(fd, &addr as *const _ as *const _, std::mem::size_of::<libc::sockaddr_in>() as u32) < 0 {
            let err = io::Error::last_os_error();
            libc::close(fd);
            return Err(err);
        }

        if libc::listen(fd, 4096) < 0 {
            let err = io::Error::last_os_error();
            libc::close(fd);
            return Err(err);
        }

        Ok(TcpListener::from_raw_fd(fd))
    }
}

fn run_worker(
    core_id: usize,
    stats: Arc<Vec<WorkerStats>>,
    queues: Arc<Vec<Arc<MPMCQueue<InterCoreTask>>>>,
    core_db_dir: PathBuf,
) -> io::Result<()> {
    sys::pin_current_thread_to_core(core_id)?;
    println!("[Worker {}] Online & pinned", core_id);

    let mut ring = uring::Uring::new(1024)?;
    let mut buf_ring = buf_ring::BufRing::new(
        ring.fd(),
        BUFFER_GROUP_ID,
        BUF_RING_ENTRIES,
        BUF_SIZE,
    )?;


    let mut db_node = EngineNode::open(&core_db_dir)?;

    let ebr_thread_idx = db_node.collector().register();

    let listener = create_reuseport_listener(8080)?;
    let listen_fd = listener.as_raw_fd();

    let sqe = ring.get_sqe().expect("SQE full");
    op::prep_accept_multishot(sqe, listen_fd);
    ring.submit()?;

    let my_stats = &stats[core_id];
    let inbound_queue = &queues[core_id];

    let mut local_sequence = 1u64;

    loop {

        ring.submit_and_wait(1)?;

        let mut recycled_any = false;

        for _ in 0..16 {
            match inbound_queue.pop() {
                Ok(InterCoreTask::ReplicateWrite { key, val }) => {
                    let k_bytes = key.to_be_bytes();
                    let v_bytes = val.to_be_bytes();
                    db_node.put(&k_bytes, &v_bytes, local_sequence, ebr_thread_idx).ok();
                    local_sequence += 1;
                }
                Err(QueueError::Empty) => break,
                _ => unreachable!(),
            }
        }

        while let Some(cqe_ref) = ring.peek_cqe() {
            let cqe = *cqe_ref;
            ring.advance_cq(1);

            let fd = op::decode_fd(cqe.user_data);
            let op = op::decode_op(cqe.user_data);
            let res = cqe.res;
            let flags = cqe.flags;

            match op {
                sys::IORING_OP_ACCEPT => {
                    if res >= 0 {
                        let client_fd = res;
                        if let Some(sqe) = ring.get_sqe() {
                            op::prep_recv_multishot(sqe, client_fd, BUFFER_GROUP_ID);
                        } else {
                            unsafe { libc::close(client_fd) };
                        }
                    }
                    if flags & sys::IORING_CQE_F_MORE == 0 {
                        if let Some(sqe) = ring.get_sqe() {
                            op::prep_accept_multishot(sqe, listen_fd);
                        }
                    }
                }

                sys::IORING_OP_RECV => {
                    if res <= 0 {
                        if res == -105 {

                            if let Some(sqe) = ring.get_sqe() {
                                op::prep_recv_multishot(sqe, fd, BUFFER_GROUP_ID);
                            }
                        } else {

                            if let Some(sqe) = ring.get_sqe() {
                                op::prep_close(sqe, fd);
                            } else {
                                unsafe { libc::close(fd) };
                            }
                        }
                    } else {
                        let bytes_read = res as u32;

                        if flags & sys::IORING_CQE_F_BUFFER != 0 {
                            let bid = (flags >> sys::IORING_CQE_BUFFER_SHIFT) as u16;
                            let buf_ptr = buf_ring.get_buffer_ptr(bid);


                            let parsed_ok = if bytes_read >= 9 {
                                unsafe {
                                    let cmd = *buf_ptr;
                                    let key_ptr = buf_ptr.add(1) as *const u64;
                                    let key = u64::from_be(key_ptr.read_unaligned());

                                    if cmd == 1 && bytes_read >= 17 {
                                        let val_ptr = buf_ptr.add(9) as *const u64;
                                        let val = u64::from_be(val_ptr.read_unaligned());
                                        
                                        let k_bytes = key.to_be_bytes();
                                        let v_bytes = val.to_be_bytes();
                                        

                                        if let Err(_) = db_node.put(&k_bytes, &v_bytes, local_sequence, ebr_thread_idx) {

                                            db_node.sync().ok();
                                        }
                                        local_sequence += 1;

                                        let target_core = (core_id + 1) % queues.len();
                                        let _ = queues[target_core].push(InterCoreTask::ReplicateWrite { key, val });
                                        
                                        true
                                    } else if cmd == 2 {
                                        let k_bytes = key.to_be_bytes();
                                        
                                        if let Some((Some(v_bytes), _seq)) = db_node.get(&k_bytes, ebr_thread_idx) {
                                            if v_bytes.len() == 8 {
                                                let val = u64::from_be_bytes(v_bytes.try_into().unwrap());
                                                let response_ptr = buf_ptr as *mut u64;
                                                response_ptr.write_unaligned(val.to_be());
                                                if let Some(sqe) = ring.get_sqe() {
                                                    op::prep_send(sqe, fd, buf_ptr, 8);
                                                }
                                            }
                                        }
                                        true
                                    } else {
                                        false
                                    }
                                }
                            } else {
                                false
                            };

                            if !parsed_ok {
                                if let Some(sqe) = ring.get_sqe() {
                                    op::prep_send(sqe, fd, buf_ptr, bytes_read);
                                }
                            }

                            my_stats.rx_bytes.fetch_add(bytes_read as u64, Ordering::Relaxed);
                            my_stats.completed_requests.fetch_add(1, Ordering::Relaxed);

                            buf_ring.recycle(bid);
                            recycled_any = true;
                        }
                    }
                }

                sys::IORING_OP_SEND => {
                    if res < 0 {
                        if let Some(sqe) = ring.get_sqe() {
                            op::prep_close(sqe, fd);
                        } else {
                            unsafe { libc::close(fd) };
                        }
                    }
                }

                sys::IORING_OP_CLOSE => {}

                _ => unreachable!(),
            }
        }

        if recycled_any {
            buf_ring.flush();
        }
    }
}