# VRift Performance Report

Generated: 2026-02-12 23:17

## Key Metrics

| Dataset | Files | Blobs | Dedup | Speed |
|---------|-------|-------|-------|-------|
| xsmall | 16,667 | 13,778 | 17.3% | 5,025/s |
| small | 23,982 | 20,575 | 14.2% | 6,005/s |
| medium | 61,790 | 51,583 | 16.5% | 5,559/s |
| large | 68,393 | 50,660 | 25.9% | 7,587/s |
| xxlarge | 194,031 | 136,464 | 29.7% | 5,603/s |

## Deduplication Efficiency

Space savings from content-addressable storage:

- **xsmall**: 16,667 files -> 13,778 blobs (17.3% dedup, ~38.5 MB saved)
- **small**: 23,982 files -> 20,575 blobs (14.2% dedup, ~49.1 MB saved)
- **medium**: 61,790 files -> 51,583 blobs (16.5% dedup, ~83.7 MB saved)
- **large**: 68,393 files -> 50,660 blobs (25.9% dedup, ~120.7 MB saved)
- **xxlarge**: 194,031 files -> 136,464 blobs (29.7% dedup, ~599.9 MB saved)

## Re-ingest Performance (CI Cache Hit)

Performance when CAS already contains content:

- **xsmall**: 37,602 files/sec (7.5x faster than first ingest)
- **small**: 30,391 files/sec (5.1x faster than first ingest)
- **medium**: 38,764 files/sec (7.0x faster than first ingest)
- **large**: 39,365 files/sec (5.2x faster than first ingest)
- **xxlarge**: 31,223 files/sec (5.6x faster than first ingest)