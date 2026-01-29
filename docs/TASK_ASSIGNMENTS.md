# Velo Rift — Development Task Assignments

> **Architect**: Prioritized tasks based on ARCHITECTURE.md specification

---

## Core Goals

VeloVFS solves two problems:
1. **Read-only file access is too slow** → mmap from CAS
2. **Duplicate files waste storage** → content-addressable deduplication

---

## Phase 1: MVP (P0 — Must Complete First)

### Dev Tasks

| ID | Task | Owner | Est. | Deps |
|----|------|-------|------|------|
| D1 | **The Source (CAS) Basic Implementation** | Dev-1 | 3d | None |
|    | - BLAKE3 hash computation | | | |
|    | - `/var/velo/the_source/` directory layout (fan-out) | | | |
|    | - `store(bytes) → hash`, `get(hash) → bytes` | | | |
| D2 | **Manifest Data Structure** | Dev-2 | 2d | None |
|    | - `VnodeEntry` struct (56 bytes) | | | |
|    | - `PathHash → VnodeEntry` HashMap | | | |
|    | - Serialization/Deserialization (bincode) | | | |
| D3 | **LD_PRELOAD Shim (Basic)** | Dev-1 | 5d | D1, D2 |
|    | - Intercept `open()`, `stat()`, `read()` | | | |
|    | - Path → Manifest lookup | | | |
|    | - CAS file redirection | | | |
| D4 | **CLI Tool (velo)** | Dev-2 | 3d | D1, D2 |
|    | - `velo ingest <dir>` — Import files to CAS | | | |
|    | - `velo run <cmd>` — LD_PRELOAD wrapper | | | |
|    | - `velo status` — Display CAS statistics | | | |

### QA Tasks

| ID | Task | Owner | Est. | Deps |
|----|------|-------|------|------|
| Q1 | **CAS Integrity Tests** | QA-1 | 2d | D1 |
|    | - Store/retrieve 100K files | | | |
|    | - Verify hash consistency | | | |
|    | - Edge cases: empty files, large files (>1GB) | | | |
| Q2 | **LD_PRELOAD Correctness Tests** | QA-1 | 3d | D3 |
|    | - Python `import numpy` via Velo | | | |
|    | - `ls`, `cat`, `stat` output consistency | | | |
|    | - Performance baseline: compare to native I/O | | | |

---

## Phase 2: Deduplication + Performance (P1)

### Dev Tasks

| ID | Task | Owner | Est. | Deps |
|----|------|-------|------|------|
| D5 | **Global Deduplication Verification** | Dev-1 | 2d | D1 |
|    | - Import same file twice → store once | | | |
|    | - Cross-project dedup statistics | | | |
| D6 | **mmap Optimization** | Dev-2 | 3d | D3 |
|    | - Direct mmap from CAS (zero-copy) | | | |
|    | - Verify Page Cache sharing | | | |
| D7 | **Packfile Hotspot Consolidation** | Dev-1 | 5d | D1 |
|    | - Profile-guided packing | | | |
|    | - Index update: hash → packfile:offset | | | |

### QA Tasks

| ID | Task | Owner | Est. | Deps |
|----|------|-------|------|------|
| Q3 | **Deduplication Ratio Verification** | QA-1 | 2d | D5 |
|    | - 10 similar projects → expect 80%+ dedup | | | |
| Q4 | **Cold Start Latency Benchmark** | QA-1 | 2d | D6 |
|    | - Python cold start: target < 100ms | | | |
|    | - npm start: target < 200ms | | | |

---

## Phase 3: Multi-Tenant Isolation (P2)

### Dev Tasks

| ID | Task | Owner | Est. | Deps |
|----|------|-------|------|------|
| D8 | **OverlayFS Integration** | Dev-1 | 5d | D3 |
|    | - Link Farm generation | | | |
|    | - OverlayFS mount sequence | | | |
|    | - CoW UpperDir management | | | |
| D9 | **Namespace Isolation** | Dev-2 | 3d | D8 |
|    | - unshare() + pivot_root() | | | |
|    | - /proc, /sys, /dev mounts | | | |
| D10 | **FUSE Daemon (VeloVFS)** | Dev-1 | 7d | D2 |
|    | - libfuse integration | | | |
|    | - lookup, getattr, read, readdir | | | |
|    | - FD Cache (LRU) | | | |

### QA Tasks

| ID | Task | Owner | Est. | Deps |
|----|------|-------|------|------|
| Q5 | **Isolation Verification** | QA-1 | 3d | D8, D9 |
|    | - Tenant A writes do not affect Tenant B | | | |
|    | - Tenant cannot access host filesystem | | | |
| Q6 | **Concurrent Tenant Stress Test** | QA-1 | 2d | D10 |
|    | - 100 concurrent tenants | | | |
|    | - No deadlock, no performance regression | | | |

---

## Milestone Summary

| Phase | Completion Criteria | Est. Duration |
|-------|---------------------|---------------|
| **P0 MVP** | `velo run python -c "import numpy"` succeeds | 2 weeks |
| **P1 Performance** | Cold start < 100ms, dedup rate > 80% | 1 week |
| **P2 Isolation** | 100 concurrent tenants without issues | 2 weeks |

---

## Tech Stack

| Layer | Technology |
|-------|------------|
| Language | Rust |
| Hash | BLAKE3 |
| KV Store | LMDB |
| Filesystem | FUSE (libfuse3) / OverlayFS |
| Serialization | bincode / serde |
| CLI | clap |

---

*Document Version: 1.0*
*Created: 2026-01-29*
