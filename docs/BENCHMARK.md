# VRift Performance Report

Generated: 2026-02-14 20:23

## Key Metrics

| Dataset | Files | Blobs | Dedup | Speed |
|---------|-------|-------|-------|-------|
| xsmall | 16,667 | 13,778 | 17.3% | 3,744/s |
| small | 23,982 | 20,579 | 14.2% | 5,224/s |
| medium | 61,790 | 51,583 | 16.5% | 6,194/s |
| large | 68,393 | 50,658 | 25.9% | 5,700/s |
| xxlarge | 194,121 | 136,527 | 29.7% | 5,471/s |

## Deduplication Efficiency

Space savings from content-addressable storage:

- **xsmall**: 16,667 files -> 13,778 blobs (17.3% dedup, ~38.5 MB saved)
- **small**: 23,982 files -> 20,579 blobs (14.2% dedup, ~49.1 MB saved)
- **medium**: 61,790 files -> 51,583 blobs (16.5% dedup, ~83.7 MB saved)
- **large**: 68,393 files -> 50,658 blobs (25.9% dedup, ~120.7 MB saved)
- **xxlarge**: 194,121 files -> 136,527 blobs (29.7% dedup, ~600.2 MB saved)

## Re-ingest Performance (CI Cache Hit)

Performance when CAS already contains content:

- **xsmall**: 31,242 files/sec (8.3x faster than first ingest)
- **small**: 34,751 files/sec (6.7x faster than first ingest)
- **medium**: 33,243 files/sec (5.4x faster than first ingest)
- **large**: 31,075 files/sec (5.5x faster than first ingest)
- **xxlarge**: 29,006 files/sec (5.3x faster than first ingest)