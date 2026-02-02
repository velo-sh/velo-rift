# VRift Performance Report

Generated: 2026-02-02 19:08

## Key Metrics

| Dataset | Files | Blobs | Dedup | Speed |
|---------|-------|-------|-------|-------|
| xsmall | 16,667 | 13,783 | 17.3% | 4,871/s |
| small | 23,976 | 20,584 | 14.1% | 3,900/s |
| medium | 61,752 | 51,538 | 16.5% | 4,024/s |
| large | 145 | 144 | 0.7% | 1,619/s |

## Deduplication Efficiency

Space savings from content-addressable storage:

- **xsmall**: 16,667 files -> 13,783 blobs (17.3% dedup, ~38.8 MB saved)
- **small**: 23,976 files -> 20,584 blobs (14.1% dedup, ~48.8 MB saved)
- **medium**: 61,752 files -> 51,538 blobs (16.5% dedup, ~83.6 MB saved)
- **large**: 145 files -> 144 blobs (0.7% dedup, ~369.7 KB saved)

## Re-ingest Performance (CI Cache Hit)

Performance when CAS already contains content:

- **xsmall**: 41,911 files/sec (8.6x faster than first ingest)
- **small**: 20,705 files/sec (5.3x faster than first ingest)
- **medium**: 40,623 files/sec (10.1x faster than first ingest)
- **large**: 4,457 files/sec (2.8x faster than first ingest)