# VRift Performance Report

Generated: 2026-02-09 02:24

## Key Metrics

| Dataset | Files | Blobs | Dedup | Speed |
|---------|-------|-------|-------|-------|
| xsmall | 16,667 | 13,780 | 17.3% | 3,587/s |
| small | 23,982 | 20,570 | 14.2% | 3,873/s |
| medium | 61,756 | 51,540 | 16.5% | 3,116/s |
| large | 68,383 | 50,650 | 25.9% | 3,639/s |
| xxlarge | 193,851 | 136,344 | 29.7% | 4,090/s |

## Deduplication Efficiency

Space savings from content-addressable storage:

- **xsmall**: 16,667 files -> 13,780 blobs (17.3% dedup, ~38.5 MB saved)
- **small**: 23,982 files -> 20,570 blobs (14.2% dedup, ~49.2 MB saved)
- **medium**: 61,756 files -> 51,540 blobs (16.5% dedup, ~83.9 MB saved)
- **large**: 68,383 files -> 50,650 blobs (25.9% dedup, ~120.5 MB saved)
- **xxlarge**: 193,851 files -> 136,344 blobs (29.7% dedup, ~598.9 MB saved)

## Re-ingest Performance (CI Cache Hit)

Performance when CAS already contains content:

- **xsmall**: 5,719 files/sec (1.6x faster than first ingest)
- **small**: 4,423 files/sec (1.1x faster than first ingest)
- **medium**: 4,892 files/sec (1.6x faster than first ingest)
- **large**: 5,364 files/sec (1.5x faster than first ingest)
- **xxlarge**: 6,238 files/sec (1.5x faster than first ingest)