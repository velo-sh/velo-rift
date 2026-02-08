# VRift Performance Report

Generated: 2026-02-09 03:39

## Key Metrics

| Dataset | Files | Blobs | Dedup | Speed |
|---------|-------|-------|-------|-------|
| xsmall | 16,667 | 13,776 | 17.3% | 3,298/s |
| small | 23,982 | 20,567 | 14.2% | 3,005/s |

## Deduplication Efficiency

Space savings from content-addressable storage:

- **xsmall**: 16,667 files -> 13,776 blobs (17.3% dedup, ~38.6 MB saved)
- **small**: 23,982 files -> 20,567 blobs (14.2% dedup, ~49.2 MB saved)

## Re-ingest Performance (CI Cache Hit)

Performance when CAS already contains content:

- **xsmall**: 182,682 files/sec (55.4x faster than first ingest)
- **small**: 157,496 files/sec (52.4x faster than first ingest)