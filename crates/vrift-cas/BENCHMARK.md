# Ingest Baseline Benchmark

Benchmark results for zero-copy ingest operations (RFC-0039 aligned).

## Test Environment

- **Machine**: MacBook Pro
- **Date**: 2026-01-31
- **Commit**: `e04f5cf`

## Results: 1000 x 4KB Files

| Mode | Time | Throughput | Operation |
|------|------|------------|-----------|
| **Phantom** | 114ms | 8,772 files/sec | `rename()` |
| Solid Tier-2 | 283ms | 3,534 files/sec | `hard_link()` |

## Key Findings

1. **Phantom mode is 2.5x faster** than Solid mode
2. Both are O(1) metadata operations - **zero data copy**
3. Bottleneck is filesystem metadata, not I/O

## Detailed Results

```
ingest_solid_tier2/10    time: [270.60 µs 287.92 µs 307.53 µs]
ingest_solid_tier2/100   time: [19.251 ms 19.641 ms 20.086 ms]
ingest_solid_tier2/1000  time: [279.70 ms 283.20 ms 287.65 ms]

ingest_phantom/10        time: [199.96 µs 212.74 µs 228.74 µs]
ingest_phantom/100       time: [13.025 ms 13.217 ms 13.457 ms]
ingest_phantom/1000      time: [113.38 ms 114.65 ms 116.43 ms]
```

## Running Benchmarks

```bash
# Run zero-copy benchmarks
cargo bench -p vrift-cas --bench zero_copy_bench

# Run with specific filter
cargo bench -p vrift-cas --bench zero_copy_bench -- phantom
```

## Comparison with Old Pipeline

The old `streaming_pipeline.rs` used `read() + write()` which is:
- O(n) data copy (unnecessary)
- Memory allocation overhead
- Slower than zero-copy approach

The new `zero_copy_ingest.rs` uses:
- `hard_link()` - O(1) inode operation
- `rename()` - O(1) atomic move

## Future Work

- Test with larger files (1MB, 100MB, 1GB)
- Test on network filesystems (NFS)
- Evaluate if streaming_pipeline watch-first pattern is still needed

---

## Real-World Benchmark: Tiered Datasets

**Date**: 2026-01-31  
**Commit**: `c6864a9`  
**Script**: `scripts/benchmark_parallel.sh`

### Test Datasets

All datasets share common dependencies for dedup testing:

| Dataset | Description | Files | Size |
|---------|-------------|-------|------|
| **Small** | Basic Next.js + React | 16,647 | 271MB |
| **Medium** | +i18n, echarts, sentry | 23,948 | 415MB |
| **Large** | +Web3, AWS SDK, Redis | 61,703 | 684MB |
| **XLarge** | Real production project | 138,201 | 1.4GB |

### Performance Results (4 threads, DashSet dedup)

| Dataset | Time | Throughput |
|---------|------|------------|
| Small | 2.57s | **6,464 files/s** |
| Medium | 3.67s | **6,530 files/s** |
| Large | 10.30s | **5,993 files/s** |
| XLarge | 28.0s | **4,936 files/s** |

### Parallel Speedup

| Threads | Time (12K files) | Speedup |
|---------|------------------|---------|
| 1 | 3.93s | 1.0x |
| 4 | 1.65s | **2.4x** |

### Optimizations Applied

1. **Rayon Parallel Ingest**: Multi-threaded file processing
2. **DashSet In-Memory Dedup**: Skip redundant hard_link for same-hash files
3. **TOCTOU Fix**: Handle EEXIST gracefully in race conditions

### Running Benchmarks

```bash
# Run all datasets
./scripts/benchmark_parallel.sh --size all

# Run specific size
./scripts/benchmark_parallel.sh --size large
```

### Notes

- XLarge based on real `velo-rift-node_modules_package.json`
- puppeteer excluded (macOS EPERM on Chromium code signing)
- LMDB manifest committed to `.vrift/manifest.lmdb`

