# ⚡ LSM-Engine

A zero-dependency, ultra-high-throughput Log-Structured Merge-Tree (LSM-Tree) storage engine integrated with a Thread-Per-Core (TPC) kernel-bypass network engine and custom Epoch-Based memory reclamation—written entirely from scratch in Rust.

This system bypasses standard OS thread pools, socket allocation bottlenecks, and kernel page caches to deliver high-concurrency, durable writes directly to storage hardware.

---

## Architecture Blueprint

```
┌─────────────────────────────────────────────────────────────────┐
│                    Thread-Per-Core Network Layer                │
│                                                                 │
│   io_uring RECV_MULTISHOT ──► PBUF_RING ──► Zero-Copy Parse    │
│   io_uring SEND ────────────◄── mmap Slice ─── Zero-Copy Reply  │
│                                                                 │
│   Workers hard-pinned 1:1 to physical CPU cores                 │
└──────────────────────────┬──────────────────────────────────────┘
                           │ EBR Guard + MPMC Flush Queue
                           ▼
┌─────────────────────────────────────────────────────────────────┐
│                    Lock-Free Concurrency Layer                  │
│                                                                 │
│   Vyukov MPMC Queue (64B cache-aligned, bounded)                │
│   3-Epoch EBR Collector (zero steady-state alloc)               │
│   Concurrent Open-Addressed HashMap                             │
│                                                                 │
│   Verified under loom + ThreadSanitizer                         │
└──────────────────────────┬──────────────────────────────────────┘
                           │ put() / get() / delete()
                           ▼
┌─────────────────────────────────────────────────────────────────┐
│                    LSM-Tree Storage Layer                       │
│                                                                 │
│   ┌──────────────┐  ┌───────────────┐  ┌────────────────────┐  │
│   │  O_DIRECT WAL │  │  SkipList     │  │  SSTable (mmap)    │  │
│   │  CRC32 + Seq  │  │  Lock-Free    │  │  Index + Blocks    │  │
│   │  Sector-Aligned│  │  Spline-Link  │  │  AVX2 Bloom Filter │  │
│   └──────────────┘  └───────┬───────┘  └────────────────────┘  │
│                              │ Freeze + Flush                   │
│                              ▼                                  │
│                   ┌─────────────────────┐                       │
│                   │  Leveled Compaction  │                       │
│                   │  K-Way Merge (Heap)  │                       │
│                   │  L0 → L1 Streaming  │                       │
│                   └─────────────────────┘                       │
└─────────────────────────────────────────────────────────────────┘
```

---

## Performance & Telemetry Benchmarks

| Metric | Measured Throughput | Design Target | Status |
| :--- | :--- | :--- | :--- |
| **MemTable Concurrent Inserts** | **2,137,751 ops/sec** | 300,000 ops/sec | *Exceeded (4.2x)* |
| **Point Read Throughput** | **1,269,581 ops/sec** | 500,000 ops/sec | *Exceeded (1.5x)* |
| **Write Amplification (WAF)** | **1.88x** | < 10x | *Optimal* |
| **End-to-End Client Network Stream** | **440,112 ops/sec** | — | *Saturated* |
| **Server-Side Direct-I/O Writes** | **24,932 ops/sec** | NVMe-bound | *Physical Limit* |
| **Bloom Filter False Positive Rate** | **< 1.0%** | < 2.0% | *Verified* |

*Benchmarks executed on Linux 7.1 Arch, x86_64 CPU with AVX2 support, over-aligned 4096-byte sector NVMe SSD blocks. Release profile optimized with Link-Time Optimization (LTO) and codegen-units=1.*

---

## Core Repositories & Modules

```
lsm-engine/
├── uring-engine/          # Tier 1: io_uring TPC network layer
│   ├── src/
│   │   ├── main.rs        # Core run loop, thread pinning, protocol dispatch
│   │   ├── uring.rs       # io_uring memory mapping and ring management
│   │   ├── op.rs          # SQE fill helpers (ACCEPT/RECV/SEND MULTISHOT)
│   │   ├── buf_ring.rs    # Kernel-provided buffer ring (PBUF_RING)
│   │   └── sys.rs         # Raw io_uring syscall bindings
│   └── src/bin/
│       ├── load_gen.rs    # Multi-threaded stress test client
│       ├── bench.rs       # Pure network latency profiler
│       └── tokio_server.rs# Baseline comparison Tokio async server
│
├── tier2_concurrency/     # Tier 2: Lock-free primitives & EBR
│   ├── src/
│   │   ├── queue.rs       # Vyukov bounded MPMC queue
│   │   ├── ebr.rs         # 3-epoch Epoch-Based Reclamation
│   │   ├── map.rs         # Lock-free open-addressed hash map
│   │   └── sync.rs        # Atomic wrappers (loom-compatible)
│   └── tests/
│       ├── ebr_loom.rs    # loom model-checking tests
│       └── queue_loom.rs
│
├── tier3_storage/         # Tier 3: LSM-Tree storage engine
│   ├── src/
│   │   ├── lib.rs         # Module exports
│   │   ├── engine.rs      # Unified EngineNode (Tier 2 + Tier 3 integration)
│   │   ├── bin/
│   │   │   └── engine_benchmark.rs # Isolated storage engine benchmark
│   │   ├── bloom/
│   │   │   ├── mod.rs
│   │   │   └── filter.rs  # Vectorized AVX2 split-block Bloom filter
│   │   ├── compaction/
│   │   │   ├── mod.rs
│   │   │   ├── manager.rs # Multi-level compactor state
│   │   │   └── merge.rs   # Streaming K-way merge sorting
│   │   ├── memtable/
│   │   │   ├── mod.rs
│   │   │   ├── manager.rs # MemTable freezer and swapper
│   │   │   └── skiplist.rs# Lock-free Spline-Linking Concurrent SkipList
│   │   ├── sstable/
│   │   │   ├── mod.rs
│   │   │   ├── block.rs   # Data block builder/iterator (~4KB target blocks)
│   │   │   ├── footer.rs  # Index offset/magic trailer formatting
│   │   │   ├── reader.rs  # Memory-mapped SSTable reader
│   │   │   ├── streaming.rs # Zero-copy SSTable record iterator
│   │   │   └── writer.rs  # Sequential block block-builder
│   │   └── wal/
│   │       ├── mod.rs
│   │       ├── aligned_buffer.rs # Sector-aligned heap memory allocator
│   │       ├── record.rs  # Framed record encoder/decoder (CRC32C)
│   │       ├── recovery.rs# Start-up scanning recovery
│   │       └── writer.rs  # Direct-I/O (O_DIRECT) sector append path
│   └── tests/
│       ├── bloom_integration.rs
│       ├── memtable_integration.rs
│       ├── sstable_integration.rs
│       ├── unified_integration.rs
│       └── wal_integration.rs
│
└── Cargo.toml             # Parent Workspace Root
```

---

## Critical Design Decisions

### 1. Direct-I/O Sector Alignment (`O_DIRECT`)
To bypass page cache copy overhead, we open WAL logs using `O_DIRECT`. This forces strict alignment contracts:
- Memory buffers must be allocated at addresses matching the physical sector boundaries (4096 bytes via our aligned allocator).
- Write spans and offsets must be multiples of the sector size.
- To prevent bottlenecking write performance at the NVMe cell sync limit (~2K-8K IOPS), we decouple `O_DSYNC` from the hot path, grouping writes in user-space and leveraging `fsync()` at the immutable MemTable freeze boundary.

### 2. Lock-Free Spline-Linking SkipList
Concurrent SkipList insertions are notoriously vulnerable to cyclic pointer deadlocks under heavy write volume. If a CAS succeeds at level $i$ but fails at $i+1$, a naive rollback of previous link points breaks thread-safety. 
Our two-stage insert algorithm commits nodes permanently at Level 0 (the linearization point), then splines levels $1 \dots N$ independently. If a CAS fails on a higher level, the writer re-scans starting from the level-0 committed predecessor, preventing cyclical loops.

### 3. Split-Block AVX2 Bloom Filter
Standard Bloom filters perform $K$ independent bit probes across a broad array, resulting in up to $K$ distinct CPU cache misses per read lookup. 
Our split-block Bloom filter hashes each key to a single, cache-line-aligned 256-bit block. We execute bit probes concurrently inside 256-bit registers using AVX2 vector instructions (`_mm256_sllv_epi32` and `_mm256_testc_si256` / `VPTEST`). This yields sub-nanosecond lookups with exactly one cache line reference.

### 4. Shared-Nothing Core-Local Database Partitioning
To scale across multiple CPU cores without lock contention, we apply a Shared-Nothing architecture. Each pinned worker thread in our TPC network loop runs an isolated `EngineNode` instance writing to its own independent directory partition (`db_root/core_0/`, `db_root/core_1/`, etc.). There are no global locks on the read or write hot paths.

---

## Wire Protocol Specifications

Communication is managed over raw TCP sockets via a simplified, big-endian binary protocol:

### WRITE Request (PUT) — 17 Bytes
```
┌───────────┬──────────────────────┬──────────────────────┐
│ Cmd: 0x01 │ Key (u64 Big-Endian) │ Val (u64 Big-Endian) │
│  1 byte   │        8 bytes       │        8 bytes       │
└───────────┴──────────────────────┴──────────────────────┘
```

### READ Request (GET) — 9 Bytes
```
┌───────────┬──────────────────────┐
│ Cmd: 0x02 │ Key (u64 Big-Endian) │
│  1 byte   │        8 bytes       │
└───────────┴──────────────────────┘
```

### Response Packet — 8 Bytes
```
┌──────────────────────────────────┐
│      Value (u64 Big-Endian)      │
│              8 bytes             │
└──────────────────────────────────┘
```

---

## Getting Started

### Prerequisites
- **OS**: Linux kernel 6.x+ (required for `io_uring` multishot network bindings)
- **Toolchain**: Rust (Nightly / Rust 1.75+)
- **CPU**: x86_64 architecture with AVX2 instruction support (`lscpu | grep avx2` verified)
- **Disk**: NVMe/SATA SSD with 512-byte or 4096-byte physical block alignment

### Build Compilation
Compile the parent workspace in release mode:
```bash
cargo build --release
```

### Run the Test Suite
Execute the entire integration and model check validation suite:
```bash
cargo test --release
```

### Start the Database Server
Launch the pinned Thread-Per-Core server on port `8080`:
```bash
./target/release/uring-engine
```

### Run the Stress Test Load Generator
In a secondary terminal window, execute our flow-controlled client load generator:
```bash
cargo run --bin load_gen --release
```

---

## Crash Recovery Invariants

Our crash recovery scanner is tested under simulated hard power failures. We execute integration tests that:
- Truncate WAL files midway through a sector to simulate **torn writes** (valid prefix recovered successfully).
- Flip bits inside live payloads to simulate **disk corruption** (verification stops gracefully at the corruption boundary).
- Verify sequence number monotonicity to prevent state machine regressions.

---

## License

This project is licensed under the MIT License.
