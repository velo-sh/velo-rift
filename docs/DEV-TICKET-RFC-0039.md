# RFC-0039 Developer Implementation Ticket

## Overview
Implement Transparent Virtual Projection as defined in RFC-0039.

---

## P0: Core Implementation

### 1. CAS Storage Layer
- [ ] Implement `TheSource` with BLAKE3 sharding (`blake3/ab/cd/...`)
- [ ] Self-describing filenames: `[hash]_[size].[ext]`
- [ ] O(1) integrity check via filename size

### 2. Manifest (LMDB)
- [ ] Implement `LmdbManifest` with heed crate
- [ ] Dual-layer structure: Base (shared) + Delta (per-project)
- [ ] Path → Hash lookup with O(1) mmap reads
- [ ] ACID transactions for crash safety

### 3. Tiered Asset Model
- [ ] **Tier-1 (Immutable)**: Owner transfer + immutable flag + symlink
- [ ] **Tier-2 (Mutable)**: Hardlink + chmod 444 + VFS intercept
- [ ] Tier classification logic based on path patterns

### 4. VFS Shim (LD_PRELOAD)
- [ ] Intercept `open()`, `stat()`, `read()`, `write()`, `close()`
- [ ] Manifest lookup on read
- [ ] Break-Before-Write for Tier-2
- [ ] O_TRUNC detection (fast path, skip content copy)

### 5. Startup Recovery
- [ ] Load Manifest from `.vrift/manifest.lmdb`
- [ ] Validate Tier-1 symlinks
- [ ] Validate Tier-2 hardlinks (inode check)
- [ ] Auto-repair broken projections

---

## P1: CLI & UX

### 6. `vrift inception` Command
- [ ] Enter VFS inception mode ("Enter the dream")
- [ ] Persist Session state to `.vrift/session.json`
- [ ] Display mode: `[Solid]` or `[Phantom]`
- [ ] `vrift wake` to exit inception mode

### 7. Mode Selection
- [ ] `--solid` (default): Physical files safe
- [ ] `--phantom`: Pure virtual (files in CAS only)

---

## P2: Performance Optimization

### 8. Read Path
- [ ] Tier-1: Zero VFS overhead (kernel symlink follow)
- [ ] Tier-2: Prefetch with `posix_fadvise(WILLNEED)`

### 9. Write Path
- [ ] Truncate-Write detection (skip content copy)
- [ ] Re-ingest on `close()`

---

## P3: Future Work (Post-MVP)

- [ ] GC: Orphan CAS entry cleanup
- [ ] Multi-project shared Base Layer
- [ ] Packfile for Tier-1 assets
- [ ] Async write buffer (mmap + io_uring)
- [ ] Remote CAS for distributed teams

---

## Acceptance Criteria

| Requirement | Test |
|-------------|------|
| P0-a: Hash-Content Match | `CAS[hash]` content equals `blake3(content)` |
| P0-b: Projection-Link Match | `VFS[path]` returns correct version |
| Crash Recovery | Restart preserves all projections |
| Solid Mode Rollback | Disable Velo → files intact |

---

## References

- [RFC-0039](./docs/RFC-0039-Transparent-Virtual-Projection.md)
- [ARCHITECTURE.md §9](./docs/ARCHITECTURE.md#9-velovfs-runtime-architecture)
