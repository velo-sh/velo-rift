# vrift Virtual Directory Real-time Sync

> [!IMPORTANT]
> **Implementation Status**: This document describes the **target v3 architecture**.
> - ✅ **Shared Memory VDir**: Implemented in `vrift-ipc` as `ManifestMmap*` structs
> - ⚠️ **Dirty Bit**: Not yet implemented - Sprint 1 target
> - ⚠️ **Per-project vdir_d**: Not yet implemented - uses single `vriftd` currently

## The Core Question

**When server updates VDir (100µs), how do clients see changes instantly?**

Answer: **Shared memory + atomic generation counter**

---

## Key Concept: Shared Memory is Live

```
/dev/shm/vrift_vdir_project_ABC  (physical memory, one copy)
    ↓ mmap
    ├─ Client A (rustc process 1234) → 0x10a000000
    ├─ Client B (rustc process 1235) → 0x20b000000
    ├─ Client C (cargo process 5678) → 0x30c000000
    └─ Server (vriftd)              → 0x40d000000

All point to SAME physical pages!
```

**When server writes to VDir**:
- Physical memory changes immediately
- All clients' mmap sees the change (OS manages cache coherency)
- **No explicit notification needed!**

---

## Synchronization Protocol

### Server Side: Update and Signal

```c
void server_update_vdir(const char *path, uint8_t cas_hash[32], 
                        uint64_t size, int64_t mtime) {
    VDirHeader *hdr = vdir_mmap;
    VDirEntry *table = vdir_mmap + hdr->table_offset;
    
    // 1. Find insertion slot
    uint64_t path_hash = fnv1a_hash(path);
    uint64_t slot = find_slot(table, path_hash, hdr->table_capacity);
    
    // 2. Write entry (may be seen by clients mid-write!)
    VDirEntry new_entry = {
        .path_hash = path_hash,
        .size = size,
        .mtime_sec = mtime,
        // ... other fields
    };
    memcpy(new_entry.cas_hash, cas_hash, 32);
    
    // 3. CRITICAL: Memory barrier before publishing
    atomic_thread_fence(memory_order_release);
    
    // 4. Write entry to table
    table[slot] = new_entry;
    
    // 5. CRITICAL: Publish update via generation counter
    //    This is the "commit point"
    uint64_t new_gen = atomic_fetch_add_explicit(
        &hdr->generation, 1, memory_order_release
    );
    
    // After this point, clients are guaranteed to see new_entry
}
```

**Time**: ~100µs (includes hash computation, slot search, write)

### Client Side: Check and Refresh

```c
int vrift_stat(const char *path, struct stat *buf) {
    VDirHeader *hdr = client->vdir_mmap;
    
    // 1. Load generation counter (atomic read)
    uint64_t current_gen = atomic_load_acquire(&hdr->generation);
    //    memory_order_acquire ensures we see all writes before generation++
    
    // 2. Check if VDir updated since last access
    if (current_gen != client->last_gen) {
        // VDir changed! 
        // But mmap already reflects changes (OS handles it)
        // We just need to validate consistency
        
        // OPTIONAL: Check if VDir resized
        if (hdr->table_capacity != client->cached_capacity) {
            // Rare: VDir grew, need to remap
            remap_vdir();  // ~1µs, very rare
        }
        
        client->last_gen = current_gen;
        // Time: 10ns (just updating local variable)
    }
    
    // 3. Proceed with lookup (data is fresh!)
    uint64_t hash = fnv1a_hash(path);
    VDirEntry *entry = vdir_lookup(hdr, hash);
    // ...
}
```

**Time**: 10ns overhead (atomic load + compare)

---

## Three Update Scenarios

### Scenario 1: Metadata Update (Same File)

**Example**: Server updates mtime of existing file

```
Server:
  1. Find entry in table (slot 43127)
  2. Update entry.mtime_sec = new_time
  3. Memory barrier (ensure write visible)
  4. generation++ (atomic)
  
  Time: ~50µs

Client (next stat):
  1. Load generation → sees new value
  2. Lookup same slot (43127)
  3. Read entry.mtime_sec → sees new time ✓
  
  Time: 170ns (normal stat)
```

**Key**: Client doesn't need to "reload" anything. OS page cache is coherent!

### Scenario 2: New File Insertion

**Example**: Compiler writes new .o file, server adds to VDir

```
Server:
  1. Hash path → slot 99999 (was empty)
  2. Write new VDirEntry to table[99999]
  3. Memory barrier
  4. generation++
  5. entry_count++
  
  Time: ~100µs

Client A (stat new file):
  1. Load generation → sees increment
  2. Hash same path → slot 99999
  3. Read table[99999] → sees new entry ✓
  4. Return metadata
  
  Time: 170ns

Client B (stat unrelated file):
  1. Load generation → sees increment
  2. Hash different path → slot 12345 (unchanged)
  3. Read table[12345] → unaffected
  
  Time: 180ns (10ns extra for generation check)
```

**Key**: Only clients accessing new file see the data. Others just check generation.

### Scenario 3: VDir Resize (Rare)

**Example**: Project grows from 50K to 100K files, table needs expansion

```
Server:
  1. Allocate new larger mmap file
     └─ /dev/shm/vrift_vdir_project_ABC.new
  2. Rehash all entries into new table
  3. Rename: .new → original (atomic)
  4. generation++
  
  Time: ~10ms (one-time, amortized)

Client (next stat):
  1. Load generation → sees large jump
  2. Check table_capacity changed? → YES
  3. munmap old, mmap new file
  4. Update cached_capacity
  5. Proceed with lookup
  
  Time: First stat after resize ~1µs (remap)
        Subsequent stats: normal 170ns
```

**Frequency**: Maybe once per 100K file additions (very rare)

---

## Memory Ordering Guarantees

### Why `atomic_thread_fence`?

Without memory barriers:
```
Server CPU:
  entry.size = 1234      // Write 1
  generation++           // Write 2

Due to CPU reordering, client might see:
  generation = new       // See Write 2 first!
  entry.size = old       // Still see old Write 1!
  
BROKEN: Client sees generation updated but stale data!
```

With `memory_order_release` / `memory_order_acquire`:
```
Server:
  entry.size = 1234
  atomic_thread_fence(memory_order_release);  // Flush to memory
  generation++ (memory_order_release)         // Publish

Client:
  gen = load(memory_order_acquire)            // Sync point
  → Guaranteed to see all writes before generation++
  entry.size == 1234 ✓
```

**Cost**: ~5ns per barrier (free on x86 TSO, minimal on ARM)

---

## Performance Analysis

### Overhead per stat()

```
Without updates (generation unchanged):
  Load generation:     5ns (atomic read)
  Compare:            5ns
  Total overhead:     10ns
  
  Percentage: 10ns / 170ns = 5.9% overhead ✓ Acceptable

With concurrent update (generation changed):
  Load generation:     5ns
  Compare (mismatch):  5ns
  Update last_gen:     5ns
  Total overhead:     15ns
  
  Percentage: 15ns / 170ns = 8.8% overhead ✓ Still fast
```

### Update frequency impact

**Typical build scenario**:
- Server updates: ~1000 files/second during compile
- Client stats: ~100K stats/second
- Update ratio: 1:100

**Expected overhead**:
- 99% of stats: 10ns overhead (no change)
- 1% of stats: 15ns overhead (generation changed)
- Average: 10.05ns overhead

**Negligible!**

---

## Edge Cases

### Case 1: Client reads during server write

```
Timeline:
  T0: Server starts writing entry (50µs)
  T1: Client stat() happens mid-write
  T2: Server finishes, generation++

Client at T1:
  Load generation → OLD value
  Read entry → Might see partial write (TORN READ!)
  
BROKEN: Client sees corrupted data!
```

**Solution**: Generation as commit point
```c
Client:
  old_gen = load(generation, acquire)
  entry = table[slot]  // May be torn
  new_gen = load(generation, acquire)
  
  if (new_gen != old_gen) {
      // Concurrent update detected, retry
      goto retry;
  }
  // entry is consistent ✓
```

**Cost**: 2 atomic loads instead of 1 → 20ns overhead (rare)

### Case 2: Multiple concurrent server updates

**Problem**: Two server threads update different entries

```
Thread A: Updates entry 1000, generation++
Thread B: Updates entry 2000, generation++

Generation may increment twice, but client sees only +1 or +2
```

**Solution**: Generation only signals "something changed", not "what changed"

Client behavior:
- Generation changed? → Assume VDir is fresher
- Lookup still works (each entry independent)
- No correctness issue ✓

### Case 3: Client mmap out-of-sync with VDir file

**Problem**: Server writes beyond client's mmap range

**Solution**: 
```c
// Server pre-allocates VDir to max size
VDir file size: 1GB (fixed, supports 10M files)
Actual used: 50MB initially

Client mmap: Full 1GB (but only 50MB dirty)
Server writes: Within 1GB range always

No resize needed until 10M files!
```

**Fallback**: If VDir truly needs growth, server creates new file, clients remap on next stat

---

## Dirty Bit Integration

### The Consistency Problem

The generation counter alone is insufficient for write consistency:

```
Without Dirty Bit:
  Writer: open("main.o") → starts writing to staging
  Reader: stat("main.o") → sees OLD metadata from VDir (stale!)
  
  BROKEN: Reader sees old file size/mtime during active write.
```

### Dirty Bit Solution

**Write Path** (InceptionLayer):
```c
// On open(path, O_WRONLY)
void inception_open_for_write(const char *path) {
    // 1. Mark file as DIRTY in shared memory
    set_dirty_bit(vdir, path, true);  // Atomic
    
    // 2. Redirect to staging file
    fd = open_staging_file(path);
    
    // Effect: All readers now forced to check staging
}
```

**Read Path** (InceptionLayer):
```c
int inception_stat(const char *path, struct stat *buf) {
    // 1. Check dirty bit FIRST
    if (is_dirty(vdir, path)) {
        // File being modified - must read real staging file
        return real_stat(staging_path_for(path), buf);
    }
    
    // 2. Clean state - use fast VDir lookup
    return vdir_stat(path, buf);
}
```

**Commit Path** (vdir_d):
```c
void handle_commit(const char *path, const char *staging) {
    // 1. Ingest staging file to CAS
    cas_hash = ingest_via_reflink(staging);
    
    // 2. Update VDir entry
    vdir_update_entry(path, cas_hash, ...);
    
    // 3. Clear dirty bit (LAST!)
    set_dirty_bit(vdir, path, false);  // Atomic
    
    // Now readers see updated VDir entry
}
```

### Dirty Bit Storage

The dirty state is stored in the `VDirEntry.flags` field:

```c
// flags bit layout
#define VDIR_FLAG_DIRTY     (1 << 0)  // File being written
#define VDIR_FLAG_DELETED   (1 << 1)  // File marked for deletion
#define VDIR_FLAG_SYMLINK   (1 << 2)  // Entry is symlink
#define VDIR_FLAG_DIR       (1 << 3)  // Entry is directory
```

**Memory Ordering**:
- Write: `set_dirty_bit` uses `memory_order_release`
- Read: `is_dirty` uses `memory_order_acquire`
- Ensures visibility across all processes via SHM

### Performance Impact

```
stat() with dirty check:
  Load flags (acquire):       5ns
  Check dirty bit:            2ns
  Total overhead:             7ns (4% of 170ns)
  
  Acceptable for strong consistency guarantee.
```

---

## Implementation Checklist

Server side:
- [ ] Use `memory_order_release` on generation write
- [ ] Pre-allocate VDir to avoid frequent resize
- [ ] Batch updates to minimize generation increments

Client side:
- [ ] Use `memory_order_acquire` on generation read
- [ ] Cache last_gen in thread-local storage
- [ ] Handle torn reads with double-check pattern (optional)

---

## Summary

**How clients see updates instantly?**

1. **Shared memory** → All processes mmap same `/dev/shm/vrift_vdir_*`
2. **OS cache coherency** → CPU cache sync handled by kernel
3. **Atomic generation** → Server signals "data ready" via memory_order_release
4. **Client polling** → Every stat() checks generation with memory_order_acquire
5. **10ns overhead** → Atomic load + compare per stat

**Result**: Sub-microsecond propagation from server write to client read!

**No IPC, no locks, no syscalls - pure memory!**
