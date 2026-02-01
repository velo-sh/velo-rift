# RFC-0047 Syscall Compliance Audit

## Audit Criteria

All syscalls must behave as if operating on a **pure virtual filesystem**:
- Reads → from Manifest + CAS
- Writes → CoW to CAS + update Manifest
- Mutations → update Manifest only (not real FS)

---

## Audit Results

### ✅ Correct (Read Path)

| Syscall | Current Behavior | RFC-0047 Compliant? |
|---------|------------------|---------------------|
| `stat` | Query Manifest → return virtual metadata | ✅ Yes |
| `lstat` | Same as stat (symlink-aware) | ✅ Yes |
| `fstat` | FD → virtual metadata from tracked FD | ✅ Yes |
| `access` | Check Manifest entry existence | ✅ Yes |
| `faccessat` | dirfd-relative access | ✅ Yes |
| `fstatat` | dirfd-relative stat | ✅ Yes |
| `readlink` | Return symlink target from Manifest | ✅ Yes |
| `opendir` | Create synthetic DIR from Manifest | ✅ Yes |
| `readdir` | Iterate Manifest entries | ✅ Yes |
| `closedir` | Cleanup synthetic DIR | ✅ Yes |
| `realpath` | Resolve virtual paths | ✅ Yes |
| `getcwd` | Return virtual CWD | ✅ Yes |
| `read` | Read from FD (passthrough) | ✅ Yes |

---

### ⚠️ Partial (Write Path - Needs Review)

| Syscall | Current Behavior | Issue | Fix |
|---------|------------------|-------|-----|
| `open(O_RDONLY)` | Extract CAS → temp file → return FD | ✅ OK | - |
| `open(O_WRONLY)` | break_link + passthrough | ⚠️ Writes to real FS | Should write to temp |
| `write` | Passthrough to FD | ⚠️ OK if FD is temp | - |
| `close` | Passthrough | ❌ No CAS insert | Need: hash → CAS → Manifest |

**Current open(write) problem:**
```rust
if is_write && path_str.starts_with(&*state.vfs_prefix) {
    let _ = break_link(path_str);  // ← This assumes real file exists!
    return None;  // Passthrough
}
```

**Should be:**
```rust
if is_write && path_str.starts_with(&*state.vfs_prefix) {
    // Create temp file for CoW
    let temp_fd = create_temp_file();
    track_dirty_fd(temp_fd, original_path);
    return Some(temp_fd);
}
```

---

### ❌ Wrong (Mutation Path)

| Syscall | Current Behavior | Issue | Fix |
|---------|------------------|-------|-----|
| `unlink` | EROFS if VFS path | ❌ Breaks compiler | Remove Manifest entry |
| `rename` | EROFS if VFS path | ❌ Breaks compiler | Update Manifest path |
| `rmdir` | EROFS if VFS path | ❌ Breaks compiler | Remove Manifest dir |
| `chdir` | Updates VIRTUAL_CWD | ⚠️ Check if dir exists in Manifest | OK |

---

### ✅ Execution (Correct)

| Syscall | Current Behavior | RFC-0047 Compliant? |
|---------|------------------|---------------------|
| `execve` | Env inheritance for shim | ✅ Yes |
| `posix_spawn` | Env inheritance | ✅ Yes |
| `posix_spawnp` | Env inheritance | ✅ Yes |
| `dlopen` | Extract VFS lib → load | ✅ Yes |
| `dlsym` | Passthrough | ✅ Yes |

---

### ✅ Memory (Correct)

| Syscall | Current Behavior | RFC-0047 Compliant? |
|---------|------------------|---------------------|
| `mmap` | FD-based passthrough | ✅ Yes |
| `munmap` | Passthrough | ✅ Yes |

---

### ℹ️ Other (Passthrough OK)

| Syscall | Current Behavior | Notes |
|---------|------------------|-------|
| `fcntl` | Passthrough | OK - FD flags |
| `openat` | dirfd-relative open | Should mirror `open` logic |

---

## Summary

| Category | Count | Status |
|----------|-------|--------|
| Read Path | 13 | ✅ Correct |
| Write Path | 4 | ⚠️ Needs CoW fix |
| Mutation Path | 3 | ❌ Wrong (EROFS) |
| Execution | 5 | ✅ Correct |
| Memory | 2 | ✅ Correct |
| Other | 3 | ✅ OK |

---

## Priority Fixes

### P0: Remove EROFS from mutations
- `unlink_shim` → Remove Manifest entry
- `rename_shim` → Update Manifest path  
- `rmdir_shim` → Remove Manifest dir entry

### P1: Complete CoW write path
- `open(O_WRONLY)` → Create temp, track FD
- `close` → If dirty FD: hash → CAS insert → Manifest update

### P2: mkdir
- `mkdir_shim` → Add Manifest dir entry
