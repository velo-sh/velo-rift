# RFC-0041: CAS Garbage Collection and Multi-Project Management

**Status**: Draft  
**Authors**: VRift Team  
**Created**: 2026-01-31  
**Supersedes**: None

---

## Abstract

This RFC proposes a garbage collection (GC) and project management system for VRift's Content-Addressable Store (CAS). It addresses the challenge of safely managing CAS blobs when multiple projects share a global store, ensuring orphaned blobs are cleaned up without breaking active projects.

---

## Motivation

### Problem Statement

VRift's power comes from cross-project deduplication via a shared CAS. However, this creates a management challenge:

```
~/.vrift/the_source/
├── blob_A  # Referenced by project1, project2
├── blob_B  # Only referenced by project1  
├── blob_C  # Orphan - no manifest references it
└── blob_D  # Hard-linked, permission restrictions
```

**Current Issues**:
1. No built-in way to clean orphaned blobs
2. Manual `rm -rf` fails on hard-linked files due to permissions
3. No visibility into which blobs belong to which projects
4. No safe way to "uninstall" a project without affecting others

### Use Cases

| Scenario | Need |
|----------|------|
| **Disk pressure** | Clean orphaned blobs to reclaim space |
| **Project removal** | Safely remove a project's blobs without breaking others |
| **Fresh start** | Wipe entire CAS for clean testing environment |
| **Audit** | Understand which projects use how much space |

---

## Design

### Core Components

#### 1. Manifest Registry

A central registry tracking all known manifests with UUID-based identification:

```
~/.vrift/registry/
├── manifests.json           # Active manifest list (SSOT)
└── manifests/
    └── <uuid>.manifest      # Cached copy of manifest (for GC when project deleted)
```

**manifests.json**:
```json
{
  "version": 1,
  "manifests": {
    "a1b2c3d4-e5f6-7890-abcd-ef1234567890": {
      "source_path": "/home/user/project1/.vrift.manifest",
      "source_path_hash": "blake3:abc123...",
      "project_root": "/home/user/project1",
      "registered_at": "2026-01-31T12:00:00Z",
      "last_verified": "2026-01-31T18:00:00Z",
      "status": "active"
    },
    "b2c3d4e5-f6a7-8901-bcde-f23456789012": {
      "source_path": "/home/user/project2/.vrift.manifest",
      "source_path_hash": "blake3:def456...",
      "project_root": "/home/user/project2",
      "registered_at": "2026-01-31T14:00:00Z",
      "last_verified": "2026-01-31T18:00:00Z",
      "status": "active"
    }
  }
}
```

**Key Design Decisions**:

| Issue | Solution |
|-------|----------|
| **Filename collision** | Use UUID as primary key, not project name |
| **Project deleted** | Detect via `source_path` existence check; mark `status: "stale"` |
| **Manifest moved** | Use `source_path_hash` to detect content changes |
| **Offline GC** | Cache manifest copy in `manifests/<uuid>.manifest` |

**Stale Project Handling**:

```
gc --verify:
  1. For each registered manifest:
     - Check if source_path exists
     - If not: mark status = "stale"
  2. Stale manifests still protect blobs until explicitly removed
  3. User can run: vrift gc --prune-stale to remove stale entries
```

**Example Flow**:
```bash
# User deletes project directory
rm -rf /home/user/project1

# Next GC detects stale manifest
vrift gc
# Output:
#   ⚠️  Stale manifest: project1 (source path deleted)
#   🗑️  Orphaned blobs: 0 (stale manifest still protecting blobs)
#
#   Run `vrift gc --prune-stale` to remove stale manifests first.

# User confirms stale removal
vrift gc --prune-stale
# Output:
#   Removed stale manifest: a1b2c3d4-... (project1)
#   Orphaned blobs now available for cleanup.

# Then clean orphans
vrift gc --delete
```

#### 2. Command: `vrift gc`

**Behavior**: Scan all registered manifests, identify unreferenced blobs, delete them.

```bash
# Dry run (default) - show what would be deleted
vrift gc

# Actually delete orphaned blobs
vrift gc --delete

# Aggressive mode - only keep blobs from specified manifests
vrift gc --only manifest1.manifest manifest2.manifest --delete
```

**Algorithm**:
```
1. Load all registered manifests
2. Build set of all referenced blob hashes
3. Walk CAS directory, identify blobs not in reference set
4. Delete orphans (if --delete) or report (dry run)
```

**Output**:
```
GC Analysis:
  📁 Registered manifests: 3
  🗄️  Total CAS blobs: 45,231
  ✅ Referenced blobs: 42,108
  🗑️  Orphaned blobs: 3,123 (142 MB)

Run with --delete to clean orphans.
```

#### 3. Command: `vrift clean`

**Behavior**: Project-level cleanup operations.

```bash
# Unregister a project (marks its unique blobs as orphans)
vrift clean --unregister /path/to/project

# Force clean entire CAS (dangerous!)
vrift clean --all --force

# Clean CAS with permission fix (chmod before rm)
vrift clean --all --force --fix-perms
```

#### 4. Command: `vrift status`

Enhanced status with per-project breakdown:

```bash
vrift status
```

**Output**:
```
VRift CAS Status:

  CAS Location: ~/.vrift/the_source
  Total Size:   1.48 GB
  Total Blobs:  115,363

  Registered Projects:
  ┌─────────────────────────────────────────────────────────────────┐
  │ Project        │ Files    │ Unique Blobs │ Shared │ Size       │
  ├─────────────────────────────────────────────────────────────────┤
  │ project1       │ 16,647   │ 13,783       │ 0      │ 222 MB     │
  │ project2       │ 23,948   │ 6,816        │ 6,967  │ +122 MB    │
  │ project3       │ 61,703   │ 30,947       │ 13,829 │ +365 MB    │
  └─────────────────────────────────────────────────────────────────┘

  Orphaned Blobs: 0 (run `vrift gc` to check)
```

---

## Hard Link Permission Handling

Hard-linked files inherit restrictive permissions. Fix strategy:

```rust
fn fix_permissions(cas_root: &Path) -> Result<()> {
    for entry in WalkDir::new(cas_root) {
        let path = entry?.path();
        if path.is_file() {
            // Add write permission
            let mut perms = fs::metadata(&path)?.permissions();
            perms.set_mode(perms.mode() | 0o200);
            fs::set_permissions(&path, perms)?;
        }
    }
    Ok(())
}
```

---

## CLI Interface Summary

| Command | Description | Safety |
|---------|-------------|--------|
| `vrift gc` | Dry-run GC analysis | Safe |
| `vrift gc --delete` | Delete orphaned blobs | Safe (only orphans) |
| `vrift gc --only <manifests> --delete` | Keep only specified | Dangerous |
| `vrift gc --older-than <duration>` | Only delete orphans older than threshold | Safe |
| `vrift clean --unregister <project>` | Remove project registration | Safe |
| `vrift clean --all --force` | Wipe entire CAS | Destructive |
| `vrift clean --all --force --fix-perms` | Wipe with perm fix | Destructive |
| `vrift status` | Show CAS and project status | Safe |
| `vrift registry --rebuild` | Rebuild registry from discovered manifests | Safe |
| `vrift doctor` | Health check (integrity, permissions, disk) | Safe |

### Interactive Confirmations

For destructive operations, require explicit user confirmation:

```
$ vrift clean --all --force

⚠️  WARNING: This will delete the ENTIRE CAS!

   Location: ~/.vrift/the_source
   Size:     1.48 GB
   Blobs:    115,363
   Projects: 4 registered

   Type 'DELETE ALL' to confirm: DELETE ALL

   Deleting... Done.
   Removed 115,363 blobs (1.48 GB)
```

**Bypass**: Use `--yes` flag for scripted usage (e.g., in CI):
```bash
vrift clean --all --force --yes
```

---

## Concurrency & Locking

### Problem
Concurrent operations (e.g., `vrift ingest` and `vrift gc --delete`) could cause data corruption or premature blob deletion.

### Solution: File-Based Locking

```rust
use fs2::FileExt;

fn acquire_registry_lock() -> Result<File> {
    let lock_path = PathBuf::from("~/.vrift/registry/.lock");
    let lock_file = File::create(&lock_path)?;
    
    // Try to acquire exclusive lock (blocks if held by another process)
    lock_file.lock_exclusive()?;
    
    Ok(lock_file)
}

// Usage in GC
fn run_gc(delete: bool) -> Result<()> {
    let _lock = acquire_registry_lock()?;
    
    // ... GC operations ...
    
    Ok(())
    // Lock released when _lock goes out of scope
}
```

### Lock Behavior

| Operation | Lock Type | Behavior if Locked |
|-----------|-----------|-------------------|
| `vrift ingest` | Exclusive | Wait (with timeout) |
| `vrift gc` | Exclusive | Wait (with timeout) |
| `vrift status` | Shared | Proceed (read-only) |

**Timeout**: Default 30s, configurable via `VRIFT_LOCK_TIMEOUT`.

---

## Atomic Operations

### Registry Write Safety

Prevent corruption from interrupted writes using write-rename pattern:

```rust
fn atomic_write_json<T: Serialize>(path: &Path, data: &T) -> Result<()> {
    let tmp_path = path.with_extension("json.tmp");
    
    // 1. Write to temp file
    let file = File::create(&tmp_path)?;
    serde_json::to_writer_pretty(file, data)?;
    file.sync_all()?;
    
    // 2. Atomic rename (POSIX guarantees atomicity)
    fs::rename(&tmp_path, path)?;
    
    Ok(())
}
```

### Registry Directory Permissions

Ensure user-private access at creation:

```rust
fn ensure_registry_dir() -> Result<PathBuf> {
    let registry_dir = PathBuf::from("~/.vrift/registry");
    fs::create_dir_all(&registry_dir)?;
    
    // Set 0700 permissions (owner-only)
    fs::set_permissions(&registry_dir, Permissions::from_mode(0o700))?;
    
    Ok(registry_dir)
}
```

---

## Two-Phase GC Safety

### Problem: TOCTOU Race Condition

Between marking a blob as orphan and deletion, another process could reference it.

### Solution: Mark-and-Sweep with Grace Period

```
Phase 1: Mark (vrift gc)
  1. Scan all manifests
  2. Identify orphan blobs
  3. Record orphans with timestamp in ~/.vrift/registry/orphans.json
  
Phase 2: Sweep (vrift gc --delete)
  1. Load orphans.json
  2. Only delete blobs marked as orphan for > GRACE_PERIOD (default: 1 hour)
  3. Re-verify each blob is still orphan before deletion
```

**orphans.json**:
```json
{
  "version": 1,
  "grace_period_seconds": 3600,
  "orphans": {
    "blake3:abc123...": {"marked_at": "2026-01-31T12:00:00Z"},
    "blake3:def456...": {"marked_at": "2026-01-31T12:00:00Z"}
  }
}
```

**CLI Integration**:
```bash
# Immediate delete (skips grace period - dangerous)
vrift gc --delete --immediate

# Respect grace period (default - safe)
vrift gc --delete

# Custom grace period
vrift gc --delete --older-than 2h
```

---

## Recovery & Diagnostics

### Registry Recovery

If `manifests.json` is corrupted or deleted:

```bash
vrift registry --rebuild
```

**Algorithm**:
1. Scan common locations for `.vrift.manifest` files:
   - All project directories in `manifests/<uuid>.manifest` cache
   - Optionally: search paths provided by user
2. Rebuild `manifests.json` from discovered manifests
3. Report discovered vs. expected manifests

### Health Check

```bash
vrift doctor
```

**Checks**:
- Registry file integrity (valid JSON, schema version)
- Stale manifest detection
- Orphan blob count
- CAS directory permissions
- Disk space availability
- Lock file status

**Output**:
```
VRift Doctor Report:

  ✅ Registry: valid (3 manifests)
  ⚠️  Stale manifests: 1 (run gc --prune-stale)
  ✅ CAS permissions: OK
  ✅ Disk space: 45 GB available
  ✅ Lock: not held

  Recommendations:
  - Run `vrift gc --prune-stale` to clean stale manifests
```

### Audit Logging

All destructive operations logged to `~/.vrift/gc.log`:

```
2026-01-31T12:00:00Z DELETE blob:blake3:abc123 size:1024 reason:orphan
2026-01-31T12:00:01Z PRUNE manifest:a1b2c3d4-... path:/home/user/project1 reason:stale
2026-01-31T12:00:02Z CLEAN_ALL blobs:45231 size:1.48GB
```

**Log Rotation**: Keep last 10 log files or 100MB total.

---

## Implementation Phases

### Phase 1: Basic GC (MVP)
- [x] Implement manifest registry (`~/.vrift/registry/manifests.json`)
- [x] File-based locking (`flock`) for concurrent operations
- [x] Atomic JSON writes (write-rename pattern)
- [x] `vrift ingest` auto-registers manifests
- [x] `vrift gc` scans and reports orphans
- [x] `vrift gc --delete` removes orphans (with grace period)

### Phase 2: Project Management
- [ ] `vrift clean --unregister <project>`
- [ ] `vrift status` with per-project breakdown
- [ ] Permission fix utilities
- [ ] Audit logging to `~/.vrift/gc.log`

### Phase 3: Recovery & Advanced Features
- [ ] `vrift registry --rebuild` recovery command
- [ ] `vrift doctor` health check
- [ ] **Bloom Filter for fast orphan detection** (see below)
- [ ] Reference counting per-blob
- [ ] Incremental GC (track last GC time)
- [ ] CAS compaction (defragmentation)

### Phase 3 Detail: Bloom Filter Optimization

For large CAS (100K+ blobs), full manifest scan is expensive. Bloom Filter accelerates orphan detection:

```
Bloom Filter Properties:
  - "NO"  → Definitely NOT in set (100% accurate, no false negatives)
  - "YES" → Possibly in set (may have false positives)
```

**Algorithm**:
```
1. Build Bloom Filter from all referenced blob hashes
2. Walk CAS directory:
   - Filter says "NO" → Definitely orphan → Fast delete ✅
   - Filter says "YES" → Maybe referenced → Exact check needed
3. Only ~1% of blobs need exact verification
```

**Memory Efficiency**:
| Blobs | HashSet (exact) | Bloom Filter (0.1% FP) |
|-------|-----------------|------------------------|
| 100K  | ~6.4 MB         | ~120 KB                |
| 1M    | ~64 MB          | ~1.2 MB                |

**Implementation**:
```rust
use probabilistic_collections::bloom::BloomFilter;

fn build_reference_filter(manifests: &[Manifest]) -> BloomFilter<Blake3Hash> {
    // 0.1% false positive rate
    let mut filter = BloomFilter::with_rate(0.001, estimated_blob_count);
    for manifest in manifests {
        for entry in &manifest.entries {
            filter.insert(&entry.hash);
        }
    }
    filter
}

fn is_orphan(hash: &Blake3Hash, filter: &BloomFilter<Blake3Hash>) -> OrphanStatus {
    if !filter.contains(hash) {
        OrphanStatus::DefinitelyOrphan  // Fast path: no false negatives
    } else {
        OrphanStatus::MaybeReferenced   // Slow path: need exact check
    }
}
```

---

## Alternatives Considered

### 1. Per-Blob Reference Counting
Store reference count with each blob. **Rejected** because:
- Adds complexity to ingest/delete paths
- Requires atomic updates
- Registry approach is simpler for MVP

### 2. Embedded Manifest in CAS
Store manifests inside CAS itself. **Rejected** because:
- Makes GC self-referential
- Harder to enumerate active projects

### 3. FUSE-Level Tracking
Track references at VFS level. **Rejected** because:
- Requires FUSE to be running
- Overkill for offline GC needs

---

## Open Questions

### Resolved

1. ~~**Manifest discovery**: Should `vrift gc` auto-discover manifests in common locations?~~
   - **Answer**: Yes, via `vrift registry --rebuild` command

2. ~~**Stale manifests**: How to handle manifests that point to deleted projects?~~
   - **Answer**: Auto-detect via `source_path` check, mark as `status: "stale"`, require explicit `--prune-stale`

3. ~~**Concurrent access**: Locking strategy during GC?~~
   - **Answer**: File-based `flock` with configurable timeout (default 30s)

### Remaining

4. **Cross-device CAS**: How to handle CAS on different filesystem/device from project?
5. **Remote CAS**: Future support for network-attached CAS (NFS, S3)?

---

## References

- RFC-0039: Transparent Virtual Projection
- Git's garbage collection: `git gc`
- Docker's image prune: `docker image prune`
