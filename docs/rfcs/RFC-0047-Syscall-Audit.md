# RFC-0047: VFS Syscall Compatibility Reference

## Purpose

Velo Rift's goal is **compiler/build acceleration** through content-addressable deduplication.

**Design Principle**: Compilers should see NO difference between VFS and real FS (the "Inception" illusion).

**Key Insight**: Not every syscall needs virtualization. The decision depends on:
1. Does it affect **dependency tracking** (mtime)?
2. Does it affect **content integrity** (read/write)?
3. Does it affect **namespace consistency** (path)?

---

## Compiler Workflow Analysis

```
1. Read source files      → stat, open, read, mmap
2. Check dependencies     → stat mtime comparison
3. Compile                → internal
4. Write output           → open(O_WRONLY), write, close
5. Atomic replace         → rename(tmp, final)
6. Update archives        → ar: lseek, write
7. Link                   → dlopen, mmap
```

---

## Syscall Matrix

### ✅ Must Virtualize

| Syscall | Why | Impact if Passthrough |
|---------|-----|----------------------|
| `stat/lstat/fstat` | Mtime for dependency tracking | Wrong rebuild decisions |
| `open(O_RDONLY)` | Read from CAS | Wrong file content |
| `realpath/getcwd/chdir` | Path namespace | Path mismatch |
| `opendir/readdir` | Directory listing | Missing files |
| `unlink` | Remove from Manifest | Real file deleted |
| `rename` | Update Manifest path | Atomic replace fails |
| `utimes` | Update Manifest mtime | Stale incremental builds |

### ⚡ Can Passthrough

| Syscall | Rationale |
|---------|-----------|
| `read/write` | FD already points to correct file |
| `lseek/pread/pwrite` | FD-local operation |
| `ftruncate` | Works on CoW temp |
| `fsync/fdatasync` | CAS already durable |
| `mmap(MAP_PRIVATE)` | FD-based, works on temp |

---

## Current Gaps

### 🔴 P0: Compiler Breaking

| Gap | Current | Required | Test |
|-----|---------|----------|------|
| `open(O_WRONLY)` | break_link | CoW temp + track FD | `test_rfc0047_open_mode_check` |
| `close` | passthrough | hash → CAS → Manifest | `test_rfc0047_cow_write_close` |
| `unlink` | EROFS | Remove Manifest entry | `test_rfc0047_unlink_vfs` |
| `rename` | EROFS | Update Manifest path | `test_rfc0047_rename_vfs` |
| `rmdir` | EROFS | Remove Manifest dir | `test_rfc0047_rmdir_vfs` |
| `utimes` | passthrough | Update Manifest mtime | `test_gap_utimes` |
| `mmap(MAP_SHARED)` | passthrough | Track writes | `test_gap_mmap_shared` |
| `sendfile` | bypass | Decompose to read+write | `test_gap_sendfile` |
| `copy_file_range` | bypass | Decompose to read+write | `test_gap_copy_file_range` |
| `flock` | temp file | Shadow lock in daemon | `test_gap_flock_semantic` |

### ⚠️ P1: May Cause Issues

| Gap | Issue | Test |
|-----|-------|------|
| `st_ino` | CAS files share inodes | `test_gap_inode_uniqueness` |
| `st_nlink` | Shows real CAS link count | `test_gap_st_nlink` |
| `dup/dup2` | Untracked FD | `test_gap_dup_tracking` |
| `fchdir` | Bypasses chdir tracking | `test_gap_fchdir` |
| `fcntl(F_SETLK)` | Lock on temp file | `test_gap_fcntl_lock` |
| `mkdir` | passthrough | `test_rfc0047_mkdir_vfs` |
| `symlink` | passthrough | `test_gap_symlink` |

### 🟢 P2-P3: Edge Cases

| Gap | Issue | Test |
|-----|-------|------|
| `st_dev` | Different device ID | `test_gap_st_dev` |
| `ctime` | Not updated on chmod | `test_gap_ctime` |
| `readdir order` | Order may differ | `test_gap_readdir_order` |
| `xattr` | Not virtualized | `test_gap_xattr` |

---

## Deep Analysis

### Why EROFS Breaks Compilers

```bash
gcc -c foo.c -o foo.o
```

Internally:
```
1. cc1: compile → /tmp/ccXXX.s
2. as: assemble → foo.o (unlink existing first)
3. rename(/tmp/ccXXX.o, foo.o) - atomic replace
```

**Problem**: `unlink(foo.o)` returns EROFS → Compilation fails

### Why utimes Matters for Make

```makefile
foo.o: foo.c foo.h
    $(CC) -c foo.c -o foo.o
```

```bash
touch foo.h  # Mark modified
make         # Should rebuild
```

**Problem**: `touch` → EPERM → mtime unchanged → Make skips rebuild

### Why mmap(MAP_SHARED) Is Dangerous

```c
// Git pack-objects
map = mmap(NULL, size, PROT_READ|PROT_WRITE, MAP_SHARED, fd, 0);
memcpy(map + offset, data, len);  // Write bypasses shim!
msync(map, size, MS_SYNC);
```

**Problem**: VFS doesn't know file changed → Silent data loss

---

## Mitigation Strategies

### A: Syscall Decomposition
```
sendfile → read() + write()
copy_file_range → read() + write()
```

### B: Shadow Locking
```
flock(vfs_fd) → daemon tracks logical lock
```

### C: Virtual Inodes
```
stat(vfs_path) → st_ino = hash(path) % 2^32
                 st_nlink = 1 (always)
```

---

## Test Coverage Summary

| Category | Tests | Status |
|----------|-------|--------|
| RFC-0047 Compliance | 6 | ✅ Complete |
| Gap Detection | 17 | ✅ Complete |
| E2E Integration | 3 | ✅ Complete |
| **Total** | **26** | **100% Covered** |

---

## Verification Status

| Component | Status |
|-----------|--------|
| Read path (stat, open, read) | ✅ Implemented |
| Path resolution (realpath, getcwd, chdir) | ✅ Implemented |
| utimes/utimensat | ✅ Implemented |
| Mutation (unlink, rename, rmdir) | ⚠️ Returns EROFS |
| CoW write path | ⚠️ Incomplete |
| mmap tracking | ❌ Not implemented |
| flock virtualization | ❌ Not implemented |
| Inode virtualization | ❌ Not implemented |
