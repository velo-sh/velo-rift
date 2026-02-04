# vrift VDir Persistence with LMDB

## Architecture Overview

### Two-Tier Storage Model

```
┌─────────────────────────────────────────┐
│  Hot Tier: Shared Memory VDir           │
│  Location: /dev/shm/vrift_vdir_*        │
│  Purpose: Client read-only access       │
│  Latency: 170ns (hash table lookup)     │
│  Volatility: Lost on reboot/crash       │
└─────────────────────────────────────────┘
           ↕ (Server manages both)
┌─────────────────────────────────────────┐
│  Cold Tier: LMDB Database               │
│  Location: ~/.vrift/db/<project_id>/    │
│  Purpose: Persistent storage            │
│  Latency: ~1-5µs (B+ tree lookup)       │
│  Durability: Survives reboot/crash      │
└─────────────────────────────────────────┘
```

**Key principle**: 
- Clients only see hot tier (mmap)
- Server synchronizes hot ↔ cold
- LMDB = source of truth for persistence

---

## LMDB Schema Design

### Database Structure

```
~/.vrift/db/
  ├─ a8f9c1d2e3f4a5b6/           (project_id)
  │   ├─ data.mdb                (LMDB data file)
  │   ├─ lock.mdb                (LMDB lock file)
  │   └─ metadata.json           (project info)
  │
  └─ fedcba0987654321/           (another project)
      └─ ...
```

### LMDB Tables (DBI)

**Table 1: `vdir_entries` (main table)**
```c
Key:   path (string, null-terminated)
Value: VDirEntry (binary, fixed 64 bytes)

struct VDirEntry {
    uint64_t path_hash;      // FNV1a (redundant but speeds up mmap export)
    uint8_t cas_hash[32];    // BLAKE3
    uint64_t size;
    int64_t mtime_sec;
    uint32_t mtime_nsec;
    uint32_t mode;
    uint16_t flags;
    uint16_t _pad;
} __attribute__((packed));  // 64 bytes
```

**Table 2: `metadata` (config)**
```c
Key:   "generation" | "entry_count" | "table_capacity" | ...
Value: uint64_t

Examples:
  "generation" → 12345
  "entry_count" → 50000
  "project_root" → "/home/user/my-app" (string)
```

**LMDB configuration**:
```c
env = mdb_env_create();
mdb_env_set_mapsize(env, 10 * 1024 * 1024 * 1024);  // 10GB max
mdb_env_set_maxdbs(env, 4);  // vdir_entries, metadata, ...
mdb_env_open(env, db_path, MDB_NOSYNC, 0644);
//                          ^^^^^^^^^
//                          Async writes (performance)
```

---

## Startup: LMDB → VDir Mmap

### Server Startup Flow

```c
void server_startup(const char *project_root) {
    project_id = generate_project_id(project_root);
    
    // 1. Open LMDB database
    char db_path[PATH_MAX];
    snprintf(db_path, sizeof(db_path), 
             "%s/.vrift/db/%s", getenv("HOME"), project_id);
    
    MDB_env *env = lmdb_open(db_path);
    
    // 2. Read metadata
    uint64_t generation = lmdb_get_u64(env, "generation");
    uint64_t entry_count = lmdb_get_u64(env, "entry_count");
    
    // 3. Create mmap VDir file
    char mmap_path[256];
    snprintf(mmap_path, sizeof(mmap_path),
             "/dev/shm/vrift_vdir_%.16s", project_id);
    
    size_t mmap_size = calculate_vdir_size(entry_count);
    int fd = open(mmap_path, O_RDWR | O_CREAT, 0644);
    ftruncate(fd, mmap_size);
    
    void *vdir_mmap = mmap(NULL, mmap_size, 
                           PROT_READ | PROT_WRITE,
                           MAP_SHARED, fd, 0);
    
    // 4. Initialize VDir header
    VDirHeader *header = (VDirHeader*)vdir_mmap;
    header->magic = 0x56524654;
    header->version = 1;
    header->generation = generation;
    header->entry_count = entry_count;
    header->table_capacity = next_power_of_2(entry_count / 0.7);
    // ...
    
    // 5. Rebuild hash table from LMDB
    rebuild_vdir_from_lmdb(env, vdir_mmap);
    
    printf("[Server] Loaded %lu entries from LMDB to VDir\n", 
           entry_count);
}
```

**Time**: ~100ms for 50K entries (bottleneck: LMDB scan + hash table rebuild)

### Rebuild Hash Table

```c
void rebuild_vdir_from_lmdb(MDB_env *env, void *vdir_mmap) {
    VDirHeader *header = (VDirHeader*)vdir_mmap;
    VDirEntry *table = vdir_mmap + header->table_offset;
    
    // Iterate LMDB entries
    MDB_txn *txn;
    MDB_cursor *cursor;
    mdb_txn_begin(env, NULL, MDB_RDONLY, &txn);
    mdb_cursor_open(txn, dbi_entries, &cursor);
    
    MDB_val key, value;
    while (mdb_cursor_get(cursor, &key, &value, MDB_NEXT) == 0) {
        const char *path = (const char*)key.mv_data;
        VDirEntry *entry = (VDirEntry*)value.mv_data;
        
        // Insert into hash table
        uint64_t slot = find_slot(table, entry->path_hash, 
                                   header->table_capacity);
        table[slot] = *entry;
    }
    
    mdb_cursor_close(cursor);
    mdb_txn_commit(txn);
}
```

---

## Runtime: Write Ordering for Consistency

### Critical Problem: Write Ordering

**Wrong approach** (data loss risk):
```c
// ❌ INCORRECT: Update volatile first
void server_handle_write_wrong(...) {
    1. update_vdir_mmap(...);      // Volatile (lost on crash)
    2. enqueue_lmdb_update(...);   // Async, may not complete
    
    // If crash here → mmap lost, LMDB behind
    // Restart → load old data from LMDB ❌
}
```

**Key principle**: **LMDB is source of truth, must be durable BEFORE mmap update**

---

### Strategy 1: Synchronous LMDB (Safest)

**Write order**: LMDB first (with fsync) → mmap second

```c
void server_handle_file_write(const char *path,
                               uint8_t cas_hash[32],
                               uint64_t size, int64_t mtime) {
    VDirHandle *vdir = get_vdir(project_root);
    
    // 1. Write to LMDB first (source of truth)
    MDB_txn *txn;
    mdb_txn_begin(vdir->lmdb_env, NULL, 0, &txn);
    
    MDB_val key = {strlen(path), (void*)path};
    VDirEntry entry = {
        .cas_hash = cas_hash,
        .size = size,
        .mtime_sec = mtime,
        // ...
    };
    MDB_val val = {sizeof(entry), &entry};
    mdb_put(txn, vdir->dbi_entries, &key, &val, 0);
    
    // Update generation
    uint64_t new_gen = vdir->generation + 1;
    mdb_put_u64(txn, "generation", new_gen);
    
    mdb_txn_commit(txn);  // ← fsync to disk (~1ms)
    //                       After this, data is DURABLE
    
    // 2. Update mmap VDir (derived from LMDB)
    update_vdir_mmap(vdir, path, &entry);
    atomic_store_release(&vdir->vdir_mmap->generation, new_gen);
    
    // Clients see update immediately via mmap
}
```

**Guarantees**:
- ✅ Perfect consistency: LMDB always complete
- ✅ Zero data loss: crash at any point is safe
- ✅ Simple implementation

**Performance**:
- LMDB commit: **~1ms** (includes fsync)
- mmap update: 100µs
- **Total: ~1.1ms per write**

**Drawback**: **10x slower** than async (100µs → 1ms)

---

### Strategy 2: WAL (Write-Ahead Log) - Recommended

**Write order**: WAL (fast fsync) → mmap → async LMDB

```c
void server_handle_file_write(const char *path, ...) {
    VDirHandle *vdir = get_vdir(project_root);
    
    // 1. Append to WAL (sequential write, fast fsync)
    WalEntry wal_entry = {
        .magic = 0x57414C31,  // "WAL1"
        .generation = vdir->generation + 1,
        .path = path,
        .entry = {cas_hash, size, mtime, ...},
        .checksum = crc32(...)
    };
    append_to_wal(vdir->wal_fd, &wal_entry);
    fsync(vdir->wal_fd);  // ← Sequential fsync (~100µs on SSD)
    //                       WAL is now DURABLE
    
    // 2. Update mmap immediately (clients see it)
    update_vdir_mmap(vdir, path, &wal_entry.entry);
    atomic_store_release(&vdir->vdir_mmap->generation, 
                         wal_entry.generation);
    
    // 3. Signal background thread (non-blocking)
    signal_wal_flush(vdir);
    
    // Total: ~200µs (WAL write + mmap update)
}

// Background WAL flusher thread
void* wal_flusher_thread(void *arg) {
    while (true) {
        wait_for_signal_or_timeout(100ms);
        
        // Batch read WAL entries
        WalEntry entries[1000];
        int count = read_wal_batch(vdir->wal_fd, entries, 1000);
        if (count == 0) continue;
        
        // Single LMDB transaction for all entries
        MDB_txn *txn;
        mdb_txn_begin(vdir->lmdb_env, NULL, 0, &txn);
        
        for (int i = 0; i < count; i++) {
            MDB_val key = {strlen(entries[i].path), entries[i].path};
            MDB_val val = {sizeof(VDirEntry), &entries[i].entry};
            mdb_put(txn, vdir->dbi_entries, &key, &val, 0);
        }
        
        // Update generation to latest
        uint64_t latest_gen = entries[count-1].generation;
        mdb_put_u64(txn, "generation", latest_gen);
        
        mdb_txn_commit(txn);  // 1 fsync for 1000 entries!
        
        // Truncate WAL (free space)
        ftruncate(vdir->wal_fd, 0);
        lseek(vdir->wal_fd, 0, SEEK_SET);
    }
}
```

**WAL File Format**:
```c
// Sequential append-only file
// Location: ~/.vrift/wal/<project_id>.wal

struct WalEntry {
    uint32_t magic;           // 0x57414C31 ("WAL1")
    uint64_t generation;      // Monotonic counter
    uint16_t path_len;
    char path[PATH_MAX];
    VDirEntry entry;          // 64 bytes
    uint32_t checksum;        // CRC32 of entire entry
} __attribute__((packed));
```

**Crash Recovery**:
```c
void server_startup(const char *project_root) {
    project_id = generate_project_id(project_root);
    
    // 1. Load LMDB (last stable state)
    lmdb_env = lmdb_open(project_id);
    uint64_t lmdb_gen = lmdb_get_u64(lmdb_env, "generation");
    
    // 2. Check for WAL
    wal_path = sprintf("~/.vrift/wal/%s.wal", project_id);
    if (access(wal_path, F_OK) == 0) {
        // WAL exists → replay uncommitted updates
        WalEntry entries[MAX_WAL_SIZE];
        int count = read_all_wal_entries(wal_path, entries);
        
        printf("[Recovery] Found %d WAL entries\n", count);
        
        // Replay to LMDB
        MDB_txn *txn;
        mdb_txn_begin(lmdb_env, NULL, 0, &txn);
        for (int i = 0; i < count; i++) {
            if (entries[i].generation <= lmdb_gen) {
                continue;  // Already in LMDB
            }
            mdb_put(txn, ..., &entries[i]);
        }
        mdb_txn_commit(txn);
        
        // Delete WAL (recovery complete)
        unlink(wal_path);
    }
    
    // 3. Rebuild mmap from LMDB (now complete)
    rebuild_vdir_from_lmdb(lmdb_env, vdir_mmap);
    
    printf("[Server] Startup complete, generation=%lu\n", 
           vdir->generation);
}
```

**Guarantees**:
- ✅ Zero data loss: WAL fsync ensures durability
- ✅ Perfect recovery: replay WAL on startup
- ✅ Fast writes: sequential WAL append (~100µs)

**Performance**:
- WAL append + fsync: **~100µs** (sequential write)
- mmap update: 100µs
- **Total: ~200µs per write** (2x async, but acceptable)
- LMDB flush: background, 1000 entries/batch

**Why WAL is fast**:
- Sequential writes to single file (no seeking)
- OS can optimize (write combining, elevator scheduling)
- SSD: sequential ~1GB/s vs random ~100MB/s

---

### Strategy 3: Async LMDB (Fastest, Accept Loss)

**Write order**: LMDB (no fsync) + mmap together → periodic sync

```c
// Configure LMDB to skip fsync
mdb_env_open(env, path, MDB_NOSYNC, 0644);
//                       ^^^^^^^^^^
//                       Writes go to page cache only

void server_handle_file_write(const char *path, ...) {
    // 1. Write to LMDB (no fsync, ~50µs)
    MDB_txn *txn;
    mdb_txn_begin(...);
    mdb_put(txn, ..., key, value);
    mdb_txn_commit(txn);  // ← No disk I/O!
    
    // 2. Update mmap (~100µs)
    update_vdir_mmap(...);
    
    // Total: ~150µs
}

// Background sync thread
void* periodic_sync_thread(void *arg) {
    while (true) {
        sleep(100ms);  // Configurable
        mdb_env_sync(vdir->lmdb_env, 1);  // Force fsync
    }
}
```

**Guarantees**:
- ⚠️ May lose last ~100ms of updates on crash
- ✅ LMDB and mmap lose together → consistent state
- ✅ Build artifacts are recompilable (acceptable loss)

**Performance**:
- LMDB write (no sync): **50µs**
- mmap update: 100µs
- **Total: ~150µs per write** (fastest!)

**Recovery**:
```c
void server_startup() {
    // 1. Open LMDB (may be behind by 100ms)
    lmdb_env = lmdb_open(...);
    
    // 2. Rebuild mmap from LMDB
    rebuild_vdir_from_lmdb(...);
    
    // Both mmap and LMDB at same (possibly old) state
    // Consistency preserved ✓
}
```

---

## Strategy Comparison

| Aspect | Sync LMDB | WAL (Recommended) | Async LMDB |
|--------|-----------|-------------------|------------|
| **Write Latency** | 1.1ms ❌ | 200µs ✅ | 150µs ✅ |
| **Durability** | Immediate ✅ | Immediate ✅ | Delayed 100ms ⚠️ |
| **Data Loss** | Never ✅ | Never ✅ | Last 100ms ⚠️ |
| **Complexity** | Low ✅ | Medium ⚠️ | Low ✅ |
| **Recovery** | Simple ✅ | WAL replay ⚠️ | Simple ✅ |
| **Throughput** | ~900/sec | ~5000/sec ✅ | ~6600/sec ✅ |

**Recommendation**: **Strategy 2 (WAL)** for production

Reasons:
- Zero data loss (critical for correctness)
- Good performance (200µs is acceptable)
- Proven pattern (used by PostgreSQL, SQLite, etc.)

**Use Strategy 3** only for:
- Development/testing environments
- Ephemeral build caches (CI)
- Cases where 100ms loss is truly acceptable

---

## Crash Recovery

### Scenario 1: Clean Shutdown

```c
void server_shutdown(VDirHandle *vdir) {
    // 1. Stop accepting new updates
    vdir->shutdown = true;
    
    // 2. Flush all pending LMDB updates
    flush_lmdb_queue(vdir);  // Blocking
    
    // 3. Close LMDB cleanly
    mdb_env_sync(vdir->lmdb_env, 1);  // Force fsync
    mdb_env_close(vdir->lmdb_env);
    
    // 4. Unmap VDir
    munmap(vdir->vdir_mmap, vdir->vdir_size);
    unlink(vdir->mmap_path);
    
    printf("[Server] Clean shutdown, all data persisted\n");
}
```

### Scenario 2: Server Crash

**What happens**:
```
Timeline:
T0: VDir mmap updated (generation=1000)
T1: LMDB queue has 500 pending updates
T2: CRASH! (server killed)

Recovery:
T3: Server restarts
T4: Load LMDB (generation=800, 200 updates behind)
T5: Rebuild VDir mmap from LMDB
T6: VDir now at generation=800 (lost 200 updates)
```

**Lost updates**: Last ~100ms of writes (default sync interval)

**Mitigation options**:

**Option A: Accept data loss (default)**
- Build artifacts are recompilable
- Loss of 100ms writes is acceptable
- Performance: no overhead

**Option B: Synchronous mode**
```c
mdb_env_open(env, path, 0, 0644);  // No MDB_NOSYNC
// Every commit → fsync
// Latency: +1ms per update
// Guaranteed durability
```

**Option C: WAL (Write-Ahead Log)**
```c
// Before updating VDir mmap:
append_to_wal(path, cas_hash, size, mtime);  // Sequential write, fast

// On crash recovery:
replay_wal_to_lmdb();  // Recover all uncommitted updates
```

---

## Performance Analysis

### Hot Path Impact (Client Read)

**Client stat() operation**:
```
Without LMDB:
  1. Load generation (acquire)     5ns
  2. Hash table lookup            30ns
  3. Fill stat buffer             10ns
  Total: 45ns (excluding intercept)

With LMDB:
  1. Load generation (acquire)     5ns  ← Same
  2. Hash table lookup            30ns  ← Same (mmap!)
  3. Fill stat buffer             10ns  ← Same
  Total: 45ns ← ZERO OVERHEAD!

LMDB is not touched on reads!
```

### Server Update Path

**Server file write handling**:
```
Without LMDB:
  1. Update VDir mmap             100µs
  2. Return
  Total: 100µs

With LMDB (async):
  1. Update VDir mmap             100µs
  2. Enqueue LMDB update           1µs  (lock-free queue)
  Total: 101µs (1% overhead)
  
  Background thread:
    3. LMDB batch write           ~5ms/1000 updates
       → Amortized: 5µs/update
```

### LMDB Write Performance

**Batch write benchmark**:
```c
// Insert 1000 entries in single transaction
mdb_txn_begin(...);
for (int i = 0; i < 1000; i++) {
    mdb_put(...);
}
mdb_txn_commit(...);  // + fsync

Time: ~5ms (1000 updates)
→ 5µs per update (amortized)
→ 200K updates/sec
```

**Sequential vs Random**:
- LMDB uses B+ tree (random-write optimized)
- SSD: ~100K random writes/sec
- Bottleneck: fsync (~1ms per commit)

---

## Startup Performance

### Cold Start (No VDir mmap exists)

```
1. Open LMDB:               ~1ms
2. Read metadata:           ~100µs
3. Create mmap file:        ~500µs
4. Rebuild hash table:      ~100ms (50K entries)
   └─ LMDB scan:            50ms
   └─ Hash table insert:    50ms
5. Ready
Total: ~102ms
```

**Optimization**: Serialize hash table directly to mmap file on shutdown
```c
// Shutdown: dump hash table to disk
write(mmap_snapshot_fd, vdir_mmap, mmap_size);

// Startup: restore from snapshot
mmap_size = filesize(mmap_snapshot_path);
vdir_mmap = mmap(NULL, mmap_size, ..., fd, 0);
// Skip rebuild! Just validate generation vs LMDB

Time: 1ms (mmap existing file)
Speedup: 100x
```

### Warm Start (VDir mmap exists in /dev/shm)

```
1. Check if mmap file exists
2. mmap existing file:      ~500µs
3. Validate generation:     ~100µs
4. Ready
Total: ~1ms

Condition: Server crashed but /dev/shm persists
```

---

## Multi-Project LMDB Management

### Per-Project Databases

```
~/.vrift/db/
  ├─ a8f9c1d2.../  (Project A)
  │   └─ data.mdb (100MB)
  │
  ├─ fedcba09.../  (Project B)
  │   └─ data.mdb (50MB)
  │
  └─ 12345678.../  (Project C)
      └─ data.mdb (80MB)

Total: 230MB on disk (metadata only)
CAS: 2.5GB (separate, shared)
```

**Isolation**: Each project has independent LMDB env

### Global LMDB Pool

```c
struct VriftServer {
    HashMap<project_id, VDirHandle*> vdirs;
    
    struct VDirHandle {
        MDB_env *lmdb_env;      // Per-project LMDB
        void *vdir_mmap;        // Per-project mmap
        // ...
    };
};
```

---

## Configuration

### Server Config

```c
// Sync interval
VRIFT_LMDB_SYNC_INTERVAL_MS=100  // Default: 100ms

// Sync batch size
VRIFT_LMDB_SYNC_BATCH_SIZE=1000  // Default: 1000 updates

// Durability mode
VRIFT_LMDB_DURABILITY=async      // async | sync | wal

// LMDB map size
VRIFT_LMDB_MAPSIZE_GB=10         // Default: 10GB
```

---

## Summary

**Two-tier architecture**:
- **Hot tier** (mmap): Client reads, 170ns
- **Cold tier** (LMDB): Persistent storage, async synced

**Performance**:
- Client read: 0ns overhead (LMDB not touched)
- Server update: +1µs overhead (async queue)
- Background sync: 5µs/update (batched)

**Durability**:
- Default: Last ~100ms writes may be lost on crash
- Acceptable for build artifacts (recompilable)
- Configurable to sync mode if needed

**Startup**:
- Cold start: 102ms (rebuild hash table)
- With snapshot: 1ms (direct mmap)

**LMDB benefits**:
- ✅ Proven durability (ACID)
- ✅ Zero-copy reads (data.mdb can be mmap'd directly)
- ✅ Efficient updates (B+ tree)
- ✅ Cross-platform

This design achieves **persistence without sacrificing performance**!
