# vrift Isolation and Fault Tolerance

This document describes the isolation boundaries and fault recovery mechanisms in the vrift architecture.

---

## 1. Design Principles

### Core Philosophy: "Shared-Nothing" (except VDir/CAS)

Each project operates in its own isolated environment:
- Separate `vdir_d` process per project
- Separate VDir mmap file per project
- Shared CAS pool (read-only for clients)

### Isolation Hierarchy

```
┌─────────────────────────────────────────────────────────┐
│                    vriftd (Coordinator)                  │
│  Global Registry, User Limits, Cross-Project Dedup      │
└───────────────┬─────────────────────────┬───────────────┘
                │                         │
     ┌──────────▼──────────┐   ┌──────────▼──────────┐
     │  vdir_d (Project A) │   │  vdir_d (Project B) │
     │  PID: 100           │   │  PID: 101           │
     │  VDir: /dev/shm/A   │   │  VDir: /dev/shm/B   │
     └──────────┬──────────┘   └──────────┬──────────┘
                │                         │
     ┌──────────▼──────────┐   ┌──────────▼──────────┐
     │  InceptionLayer     │   │  InceptionLayer     │
     │  (rustc processes)  │   │  (cargo processes)  │
     │  Project A clients  │   │  Project B clients  │
     └─────────────────────┘   └─────────────────────┘
```

---

## 2. Isolation Dimensions

### 2.1 Process Isolation (OS-Level)

**Mechanism**: Each project has its own `vdir_d` Unix process.

**Properties**:
- Memory corruption in Project A's `vdir_d` cannot affect Project B
- Crash in one `vdir_d` leaves other projects running
- OS provides resource isolation (CPU, memory limits via cgroups)

**Example Failure**:
```
Scenario: Protocol bug causes vdir_d (Project A) to panic.

Result:
  ✗ Project A: All builds stop, clients enter fallback mode
  ✓ Project B: Continues running unaffected
  ✓ vriftd:    Continues running, can restart A's daemon
```

### 2.2 VDir Isolation (Memory Boundary)

**Mechanism**: Each project has a separate shared memory file.

```
/dev/shm/
  ├─ vrift_vdir_a8f9...  (Project A, 60MB)
  ├─ vrift_vdir_fedc...  (Project B, 40MB)
  └─ vrift_cas/          (Shared, read-only for clients)
```

**Properties**:
- Clients of Project A **only** mmap `vrift_vdir_a8f9...`
- They physically **cannot** address Project B's memory
- OS page tables enforce this boundary

**Security Note**:
- Files created with mode `0600` (owner-only access)
- Prevents other users from reading VDir contents

### 2.3 Data Plane Isolation (Per-Project Staging)

**Mechanism**: Each project has its own staging directory.

```
.vrift/staging/
  ├─ project_a8f9.../
  │   ├─ pid_1234/
  │   │   └─ fd_5_123456.tmp
  │   └─ pid_1235/
  │       └─ fd_8_234567.tmp
  └─ project_fedc.../
      └─ ...
```

**Properties**:
- Staging files are isolated by project and process
- Crash in one process leaves sibling files intact
- `vdir_d` only accesses its own project's staging area

### 2.4 CAS Isolation (Content-Level)

**Mechanism**: CAS is shared but content-addressed.

**Properties**:
- Corruption of one blob only affects files with that hash
- Cross-project: deduplication saves space, doesn't share failures
- Blobs are immutable once written

---

## 3. Fault Scenarios and Recovery

### 3.1 Client Crash (InceptionLayer)

**Scenario**: `rustc` process killed during write.

**Detection**:
- `vdir_d` monitors UDS connection
- Socket HUP signals client death

**Recovery**:
```c
void on_client_disconnect(int client_fd) {
    pid_t pid = get_client_pid(client_fd);
    
    // 1. Find dirty files from this client
    files = find_dirty_files_by_pid(vdir, pid);
    
    // 2. Rollback dirty state
    for (file in files) {
        clear_dirty_bit(file);
        // VDir entry reverts to last committed version
    }
    
    // 3. Cleanup staging files
    remove_staging_dir(pid);
    
    log("Client %d disconnected, rolled back %d files", pid, count);
}
```

**Result**: VDir returns to consistent state. No data loss.

### 3.2 vdir_d Crash (Project Daemon)

**Scenario**: Project daemon crashes or is killed.

**Detection**:
- InceptionLayer's UDS connection fails
- `vriftd` coordinator detects missing heartbeat

**Client Behavior** (Fallback Mode):
```c
int inception_stat(const char *path, struct stat *buf) {
    if (!vdir_daemon_available()) {
        // Daemon dead - fall back to real filesystem
        return real_stat(path, buf);
    }
    // Normal path...
}
```

**Recovery**:
```
1. vriftd detects missing daemon
2. vriftd spawns new vdir_d for project
3. New vdir_d loads VDir from LMDB
4. Clients reconnect automatically
5. Normal operation resumes
```

**Result**: Brief fallback to real FS (<100ms), then normal speed.

### 3.3 vriftd Crash (Coordinator)

**Scenario**: Central coordinator crashes.

**Impact**:
- Existing `vdir_d` processes continue running
- Existing builds continue unaffected
- New projects cannot be started

**Recovery**:
```
1. systemd/launchd restarts vriftd
2. vriftd scans for existing vdir_d processes
3. Re-establishes registry
```

**Result**: Minimal disruption to running builds.

### 3.4 Data Corruption

**Scenario**: VDir mmap file corrupted (disk error, cosmic ray).

**Detection**:
```c
// On client stat()
VDirHeader *hdr = vdir_mmap;
if (hdr->magic != 0x56524654) {
    // Corruption detected!
    enter_fallback_mode();
    notify_daemon(CORRUPTION_DETECTED);
}
```

**Recovery**:
```
1. vdir_d receives corruption notification
2. vdir_d invalidates mmap
3. vdir_d rebuilds VDir from LMDB (source of truth)
4. Clients remap new VDir
```

**Result**: ~100ms rebuild time, no data loss.

---

## 4. Resource Limits and Safety Mechanisms

### 4.1 Quota Enforcement

```c
// Server-side watchdog
struct ProjectQuotas {
    size_t max_vdir_size;      // 1GB default
    size_t max_staging_size;   // 10GB default
    uint32_t max_files;        // 10M default
};

void quota_watchdog(VDirHandle *vdir) {
    if (vdir->staging_size > quota.max_staging_size) {
        // Force cleanup or reject new writes
        evict_oldest_staging_files(vdir);
    }
}
```

### 4.2 No Global Locks

**Design**: Each VDir has independent synchronization.

**Properties**:
- Lock contention is strictly intra-project
- Project A's locking never stalls Project B
- Deadlock in one project cannot propagate

### 4.3 Health Checks

```c
// Periodic health monitoring
void health_check_loop() {
    while (running) {
        for (vdir in active_vdirs) {
            if (!is_responsive(vdir->daemon_pid)) {
                restart_daemon(vdir);
            }
            if (vdir->staging_orphans > threshold) {
                cleanup_orphans(vdir);
            }
        }
        sleep(10);  // Every 10 seconds
    }
}
```

---

## 5. Fallback Mode Behavior

When vrift infrastructure is unavailable, InceptionLayer enters **Fallback Mode**:

| Operation | Fallback Behavior | Latency |
|-----------|-------------------|---------|
| `stat()` | Real syscall | ~2µs |
| `read()` | Real syscall | ~3µs/4KB |
| `write()` | Real syscall | Native |

**Properties**:
- Build continues (slower but correct)
- Transparent to application
- Automatic recovery when daemon returns

---

## 6. Summary

| Dimension | Isolation Level | Recovery Time |
|-----------|----------------|---------------|
| Process | OS-level (highest) | Immediate |
| VDir Memory | MMU-enforced | ~100ms (rebuild) |
| Staging Data | Filesystem paths | Immediate (cleanup) |
| CAS Blobs | Content-addressed | N/A (immutable) |

**Key Guarantee**: A failure in Project A **cannot** corrupt or stall Project B.
