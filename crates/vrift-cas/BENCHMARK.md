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

## Real-World Benchmark: node_modules (139K files)

**Date**: 2026-01-31  
**Test Data**: Next.js + React project with ~150 dependencies  
**Script**: `scripts/benchmark_node_modules.sh`

| Metric | Value |
|--------|-------|
| **Files** | 139,587 |
| **Directories** | 14,154 |
| **Total Size** | 995 MB |
| **Ingest Time** | 54 seconds |
| **Throughput** | **~2,585 files/sec** |
| **Mode** | Solid Tier-2 (hard_link) |

### Verification

```
$ stat /tmp/vrift-demo/node_modules/lodash/package.json
  inode: 378565806, links: 2  # hard_link confirmed
```

### Notes

- Excludes `puppeteer/.local-chromium` (macOS code-signed bundle)
- LMDB manifest committed to `.vrift/manifest.lmdb`
- Time is pure ingest (excludes npm download time)
