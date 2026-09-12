# ⚡ LSM-Engine

**A zero-dependency, high-throughput LSM-Tree storage engine built from raw Linux kernel primitives in Rust.**

No Tokio on the hot path. No crossbeam. No RocksDB bindings. Just `libc`, atomic CAS loops, AVX2 SIMD intrinsics, and `io_uring` submission queues.

---

## Architecture Overview
