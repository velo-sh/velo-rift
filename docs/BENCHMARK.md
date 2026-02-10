# VRift Performance Report

Generated: 2026-02-09 14:40

## Key Metrics (Latest)

| Dataset | Files | Blobs | Dedup | First Ingest | Re-ingest |
|---------|-------|-------|-------|-------------|-----------|
| xsmall | 16,667 | 13,522 | 18.9% | 2,698/s | 100,728/s |
| small | 23,982 | 20,307 | 15.3% | 3,470/s | 203,036/s |
| medium | 61,756 | 51,531 | 16.6% | 2,785/s | 76,336/s |

## Optimization History

| Phase | Change | First Ingest | Re-ingest |
|-------|--------|-------------|-----------|
| Baseline | Single-threaded, no cache | ~1,600/s | ~1,600/s |
| Phase 4 | Mutex-free workers, skip chmod/chflags, warm_directories | ~2,700/s | ~46,000/s |
| Phase 5 | Zero-syscall cache hits, pre-stat metadata, reuse String buffer | **3,841/s** | **138,023/s** |

### Phase 4 Optimizations
- **Pre-create CAS directories** (`warm_directories`): avoids per-file `mkdir` overhead
- **Skip redundant chmod/chflags**: for existing CAS files, skip syscalls
- **Mutex-free result collection**: per-worker local `Vec`, eliminated lock contention

### Phase 5 Optimizations
- **Skip warm_directories on re-ingest**: probe `blake3/ff/ff/` — 1 stat instead of 65K
- **Eliminate redundant stat()**: pass metadata from scanner through channel, zero syscalls on cache hit
- **Reuse String buffer for manifest key**: avoid per-file heap allocation

## Re-ingest Performance (CI Cache Hit)

Performance when CAS already contains content (100% cache hit):

| Dataset | Speed | Speedup vs First Ingest |
|---------|-------|------------------------|
| xsmall | 100,728/s | 37.3x |
| small | 203,036/s | 58.5x |
| medium | 76,336/s | 27.4x |

## Deduplication Efficiency

Space savings from content-addressable storage:

- **xsmall**: 16,667 files → 13,522 blobs (18.9% dedup, ~41.9 MB saved)
- **small**: 23,982 files → 20,307 blobs (15.3% dedup, ~53.0 MB saved)
- **medium**: 61,756 files → 51,531 blobs (16.6% dedup, ~83.9 MB saved)

## Test Environment

- **Hardware**: Apple Silicon (M-series), NVMe SSD
- **OS**: macOS (Darwin)
- **Dataset**: `node_modules` directory (npm install)
- **Method**: daemon pre-started, `vrift ingest` with release build
- **Re-ingest**: same dataset, manifest cache loaded, warm CAS

## Build Cache (VDir Native Architecture)

Target: `velo` project (329 crates, ~51s clean build)

### E2E Flow

```
vrift ingest target/ → cp -a target/ cache/ → rm -rf target/ → cp -a cache/ target/ → cargo build
```

| Scenario | Time | Crates Compiled | Speedup |
|----------|------|-----------------|---------|
| Clean build (no cache) | 51.0s | 329 | — |
| No-op (intact target/) | 0.37s | 0 | 138x |
| `cp -a` restore (mtime preserved) | 0.38s | 0 | 134x |
| `cp -r` restore (mtime reset, no VRift) | 25.7s | 329 | 2x |
| **Cache restore + VRift (VDir native)** | **0.79s** | **0** | **65x** |

### Mechanism

VRift intercepts `stat()` at the syscall layer. VDir entries serve the original
build-time mtime with nanosecond precision. Cargo's fingerprint check sees
exact mtime match → all artifacts fresh → no-op build.

Overhead vs native no-op: +0.23s (VFS stat interception cost).