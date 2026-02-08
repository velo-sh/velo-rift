# VRift Performance Report

Generated: 2026-02-09 04:03

## Key Metrics

| Dataset | Files | Blobs | Dedup | Speed |
|---------|-------|-------|-------|-------|
| xsmall | 16,667 | 13,522 | 18.9% | 2,698/s |
| small | 23,982 | 20,307 | 15.3% | 3,470/s |
| medium | 61,756 | 51,531 | 16.6% | 2,785/s |

## Deduplication Efficiency

Space savings from content-addressable storage:

- **xsmall**: 16,667 files -> 13,522 blobs (18.9% dedup, ~41.9 MB saved)
- **small**: 23,982 files -> 20,307 blobs (15.3% dedup, ~53.0 MB saved)
- **medium**: 61,756 files -> 51,531 blobs (16.6% dedup, ~83.9 MB saved)

## Re-ingest Performance (CI Cache Hit)

Performance when CAS already contains content:

- **xsmall**: 100,728 files/sec (37.3x faster than first ingest)
- **small**: 203,036 files/sec (58.5x faster than first ingest)
- **medium**: 76,336 files/sec (27.4x faster than first ingest)