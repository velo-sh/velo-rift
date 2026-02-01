# RFC-0049: VFS Inception Gap Analysis

## Abstract

This RFC documents all potential gaps that could break the "Inception" illusion - 
scenarios where a compiler or runtime system could detect it's running in VFS mode
instead of on a real filesystem.

**Goal**: Compilers should see NO difference between VFS and real FS.

---

## Threat Model

The VFS shim must maintain the illusion that processes are operating on a normal
filesystem. Any detectable inconsistency could cause:

1. **Build failures** - Compiler/linker exits with error
2. **Silent corruption** - Wrong output, undetected
3. **Performance regression** - Fallback to slow paths
4. **Security bypasses** - Escape the virtualization

---

## Gap Categories

### 1. Metadata Inconsistency

| Gap | Description | Impact | Priority |
|-----|-------------|--------|----------|
| **st_ino** | CAS files share inodes (hard links) | `find -inum`, rsync dedup | P1 |
| **st_dev** | VFS path vs temp file have different device ID | Cross-device rename detection | P2 |
| **st_nlink** | CAS dedup = many links to same blob | "Is this hard-linked?" checks | P1 |
| **ctime** | chmod/chown should update ctime, not mtime | Make, git metadata tracking | P2 |

**Impact Analysis:**
- `find -inum 12345` may match multiple logical files
- `rsync -H` (hard-link preservation) gets confused
- `git diff` may show spurious changes

---

### 2. File Descriptor Tracking

| Gap | Description | Impact | Priority |
|-----|-------------|--------|----------|
| **dup/dup2/dup3** | dup(vfs_fd) creates untracked FD | Lost VFS tracking | P1 |
| **fchdir(fd)** | Change CWD via FD from VFS | chdir tracking bypassed | P1 |
| **/proc/self/fd/** | Reading symlinks reveals temp path | Debug tools, strace | P3 |

**Impact Analysis:**
```bash
# Shell redirection pattern
exec 3< vfs_file.txt
cat <&3  # FD 3 may point to temp, not tracked
```

---

### 3. Advanced File Operations

| Gap | Description | Impact | Priority |
|-----|-------------|--------|----------|
| **sendfile** | Kernel zero-copy between FDs | Bypasses shim | 🔴 P0 |
| **copy_file_range** | Kernel reflink/copy | Bypasses shim | 🔴 P0 |
| **splice/vmsplice** | Pipe-based zero-copy | Bypasses shim | P1 |
| **mmap(MAP_SHARED)** | Shared memory writes | Changes not tracked for reingest | 🔴 P0 |

**Impact Analysis:**
```c
// Git pack-objects pattern
void *map = mmap(NULL, size, PROT_READ|PROT_WRITE, MAP_SHARED, fd, 0);
memcpy(map + offset, data, len);  // Write not tracked!
msync(map, size, MS_SYNC);
munmap(map, size);
// VFS doesn't know file changed
```

---

### 4. Directory Semantics

| Gap | Description | Impact | Priority |
|-----|-------------|--------|----------|
| **readdir order** | VFS listing order differs from real FS | Test result order changes | P2 |
| **opendir+unlink+readdir** | Unlink while iterating | Entry visibility semantics | P2 |
| **. and ..** | inode/dev must match parent | Path traversal | P2 |

**Impact Analysis:**
```bash
# Pattern used by find/rm
for f in dir/*; do rm "$f"; done
# If iteration order changes, some tools misbehave
```

---

### 5. File Locking

| Gap | Description | Impact | Priority |
|-----|-------------|--------|----------|
| **flock()** | Lock on temp file, not logical file | Two processes both get "lock" | P0 |
| **fcntl(F_SETLK)** | POSIX record locking | Lock range on wrong file | P1 |
| **O_EXLOCK/O_SHLOCK** | macOS atomic open+lock | Lock not applied | P1 |

**Impact Analysis:**
```c
// ccache pattern
fd = open(".ccache/lock", O_RDWR);
flock(fd, LOCK_EX);  // If fd points to temp, lock is useless
// Two ccache instances both think they have the lock
```

---

### 6. Special File Types

| Gap | Description | Impact | Priority |
|-----|-------------|--------|----------|
| **Symlink chains** | VFS→real→VFS traversal | realpath confusion | P2 |
| **FIFO (mkfifo)** | Named pipes in VFS | Not content-addressable | P2 |
| **Unix sockets** | Socket files in VFS | Not content-addressable | P3 |

---

### 7. Extended Attributes & Security

| Gap | Description | Impact | Priority |
|-----|-------------|--------|----------|
| **xattr** | getxattr/setxattr | macOS code signing, Finder | P2 |
| **ACL** | Access control lists | Enterprise environments | P3 |
| **SELinux** | Security labels | Linux sandboxing | P3 |

---

## Priority Matrix

### 🔴 P0: Will Break Common Compiler Workflows

| Gap | Affected Tools | Symptom |
|-----|----------------|---------|
| mmap(MAP_SHARED)+write | Git, databases | Silent data loss |
| sendfile/copy_file_range | cp, rsync, nginx | Partial copy |
| flock semantics | ccache, distcc, make -j | Race conditions |

### ⚠️ P1: May Cause Intermittent Issues

| Gap | Affected Tools | Symptom |
|-----|----------------|---------|
| st_ino/st_nlink | find, rsync, git | Spurious matches |
| dup/dup2 tracking | Shell scripts | Lost FD state |
| fcntl locking | npm, pip, package managers | Corruption |

### 🟢 P2-P3: Edge Cases

- readdir order: Test flakiness
- xattr: Signing failures
- FIFO/socket: Build system IPC

---

## Mitigation Strategies

### Strategy A: Intercept at Syscall Level

```
sendfile → decompose to read() + write()
copy_file_range → decompose to read() + write()
mmap(MAP_SHARED) → Convert to MAP_PRIVATE + manual sync
```

**Pros**: Complete control
**Cons**: Performance overhead, complexity

### Strategy B: Shadow Locking

Create a parallel lock namespace in daemon:
```
flock(vfs_fd) → daemon tracks logical lock
                not filesystem lock
```

### Strategy C: Virtual Inodes

Assign synthetic inode numbers from Manifest:
```
stat(vfs_path) → st_ino = hash(logical_path) % 2^32
                 st_dev = VRIFT_VIRTUAL_DEV
                 st_nlink = 1 (always)
```

---

## Verification Tests Needed

| Test | Description | File |
|------|-------------|------|
| `test_e2e_mmap_shared` | MAP_SHARED write detection | TODO |
| `test_e2e_sendfile` | Zero-copy bypass detection | TODO |
| `test_e2e_flock_semantic` | Lock isolation | TODO |
| `test_e2e_inode_consistency` | st_ino uniqueness | TODO |
| `test_e2e_dup_tracking` | FD duplication | TODO |

---

## Related RFCs

- RFC-0047: Syscall Compliance Audit
- RFC-0039: VFS Architecture
