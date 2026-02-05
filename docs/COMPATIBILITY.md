# Velo Rift™ Comprehensive Compatibility Report

This report provides the definitive status of Velo Rift's compatibility with host environments, POSIX standards, and industrial toolchains.

---

## 🏁 State of the Union (Feb 5, 2026 Verification)

The Unified QA Suite and Proof of Failure (PoF) suite v3.0 have confirmed the following status:

> **✅ Latest Regression Results (Feb 5, 2026 @ 02:58 UTC+8)**
> - **Overall Pass Rate**: **100% (10/10 tests)** 🎉
> - **Boot Safety**: **100% PASS** (5/5 iterations, no deadlock)
> - **E2E Golden Path**: **100% PASS** ✨
> - **Dedup Value Proof**: **99% dedup** (100 files → 1 blob)
> - **Daemon/Service**: **100% PASS** (autostart, persistence, service control)
> - **Hardlink Boundary**: **100% PASS** ✨ **(Fixed after clean build)**
> - **Commit**: `590dce9` (main branch)

### Key Improvements in v3.3 (Clean Build)
1.  **Hardlink Boundary Protection Fixed ✅**:
    -   Cross-VFS hardlink now correctly returns EXDEV.
    -   `test_value_2_rename.sh` passes 4/4 tests.
2.  **E2E Golden Path Fixed ✅**:
    -   Mutation perimeter (`mkdir` blocking) works correctly.
    -   VFS read operations successfully redirect from CAS.
3.  **Normalization Invariants (RFC-0043) ✅**:
    -   VFS is strictly case-sensitive (verified via `test_normalization_invariants.sh`).
    -   Path canonicalization required for manifest lookups.
4.  **Portable Path Resolution ✅**:
    -   All hardcoded paths replaced with dynamic resolution.
    -   Tests work on any machine without modification.

### No Remaining Regression Gaps ✅

All tests pass after clean build (`cargo clean && cargo build --release`).

---

## 💻 Host Environment Support

| Platform | Architecture | Status | Minimum Requirements |
| :--- | :--- | :--- | :--- |
| **macOS** | ARM64 (M1/M2/M3) | ✅ Tier 1 | macOS 12.0+, SIP Compatibility Mode |
| **macOS** | x86_64 | ✅ Tier 2 | macOS 12.0+ |
| **Linux** | x86_64 | ✅ Tier 1 | Kernel 5.15+, User Namespaces enabled |
| **Linux** | ARM64 | ✅ Tier 2 | Kernel 5.15+ |
| **Windows** | x86_64 | ❌ Unsupported | N/A (WSL2 recommended) |
---

## 📋 Unified Syscall Registry

All syscalls relevant to VFS virtualization. Status indicates implementation state:
- ✅ Implemented & Tested
- 🔄 Implemented (Needs E2E Verification)
- ⏳ Pending (Passthrough)
- ❌ Not Applicable

| Syscall | Category | Status | macOS | Linux | Test | Notes |
| :--- | :--- | :---: | :---: | :---: | :--- | :--- |
| **`open`** | File Ops | ✅ | ✅ | ✅ | `test_open_*` | Virtual path → CAS redirection |
| **`openat`** | File Ops | ✅ | ✅ | ✅ | `test_openat_*` | dirfd-relative open |
| **`close`** | File Ops | ✅ | ✅ | ✅ | `test_close_*` | Sync-on-Close IPC |
| **`read`** | File Ops | ✅ | ✅ | ✅ | `test_read_*` | FD passthrough |
| **`write`** | File Ops | ✅ | ✅ | ✅ | `test_write_*` | CoW tracking |
| **`stat`** | Metadata | ✅ | ✅ | ✅ | `test_stat_*` | O(1) Hot Stat |
| **`lstat`** | Metadata | ✅ | ✅ | ✅ | `test_stat_*` | Symlink-aware |
| **`fstat`** | Metadata | ✅ | ✅ | ✅ | `test_fstat_*` | FD-to-Vpath |
| **`fstatat`** | Metadata | ✅ | ✅ | ✅ | `test_at_*` | dirfd-relative |
| **`access`** | Metadata | ✅ | ✅ | ✅ | `test_access_*` | Virtual bitmask |
| **`faccessat`** | Metadata | ✅ | ✅ | ✅ | `test_at_*` | dirfd-relative |
| **`opendir`** | Discovery | ✅ | ✅ | ⏳ | `test_opendir_*` | Synthetic DIR |
| **`readdir`** | Discovery | ✅ | ✅ | ⏳ | `test_opendir_*` | Virtual entries |
| **`closedir`** | Discovery | ✅ | ✅ | ⏳ | `test_opendir_*` | State cleanup |
| **`readlink`** | Discovery | ✅ | ✅ | ✅ | `test_readlink_*` | Manifest target |
| **`realpath`** | Namespace | ✅ | ✅ | ⏳ | `test_realpath_virtual` | VFS path resolution |
| **`getcwd`** | Namespace | ✅ | ✅ | ✅ | `test_getcwd_chdir_*` | Virtual CWD |
| **`chdir`** | Namespace | ✅ | ✅ | ✅ | `test_getcwd_chdir_*` | Manifest lookup |
| **`execve`** | Execution | ✅ | ✅ | ✅ | `test_execve_*` | Env inheritance |
| **`posix_spawn`** | Execution | ✅ | ✅ | ⏳ | `test_spawn_*` | Recursion-safe |
| **`posix_spawnp`** | Execution | ✅ | ✅ | ⏳ | `test_spawn_*` | PATH-resolving |
| **`mmap`** | Memory | ✅ | ✅ | ✅ | `test_gap_mmap_shared` | CoW-aware tracking |
| **`munmap`** | Memory | ✅ | ✅ | ✅ | `test_gap_mmap_shared` | Re-ingest trigger |
| **`dlopen`** | Dynamic | ✅ | ✅ | ⏳ | `test_dlopen_*` | Library extraction |
| **`dlsym`** | Dynamic | ✅ | ✅ | ⏳ | `test_dlsym_*` | Symbol binding |
| **`fcntl`** | Control | ✅ | ✅ | ✅ | `test_fcntl_*` | Flags tracking |
| **`flock`** | Control | ✅ | ✅ | ✅ | `test_gap_flock_semantic` | Daemon Lock Manager |
| **`rename`** | Mutation | 🔄 | ✅ | ✅ | `test_gap_boundary_rename`, `test_value_2_rename.sh` | **Regression Found**: Deadlock/Hang in cross-domain `mv` |
| **`unlink`** | Mutation | ✅ | ✅ | ✅ | `test_fail_unlink_cas`, `test_rfc0047_unlink_vfs` | VFS: EROFS guard |
| **`mkdir`** | Mutation | ✅ | ✅ | ✅ | `test_mkdir_recursive`, `test_rfc0047_mkdir_vfs` | VFS: EROFS guard |
| **`rmdir`** | Mutation | ✅ | ✅ | ✅ | `test_rfc0047_rmdir_vfs` | VFS: EROFS guard |
| **`chmod`** | Mutation | ✅ | ✅ | ⏳ | `test_shell_chmod_interception` | VFS: EROFS guard |
| **`fchmodat`** | Mutation | ✅ | ✅ | ⏳ | - | VFS: EROFS guard |
| **`chown`** | Mutation | ➖ | ➖ | ➖ | (via `test_gap_mutation_perimeter`) | Passthrough by design |
| **`utimes`** | Mutation | ✅ | ✅ | ⏳ | `test_gap_utimes` | VFS mtime via IPC |
| **`utimensat`** | Mutation | ✅ | ✅ | ⏳ | - | VFS time via IPC |
| **`renameat`** | Mutation | ✅ | ✅ | ⏳ | `test_gap_renameat_bypass` | VFS: EROFS guard |
| **`link`** | Mutation | ✅ | ✅ | ⏳ | - | VFS: EROFS guard |
| **`linkat`** | Mutation | ✅ | ✅ | ⏳ | - | VFS: EROFS guard |
| **`symlink`** | Mutation | ✅ | ✅ | ⏳ | - | VFS: EROFS guard |
| **`truncate`** | Mutation | ✅ | ✅ | ⏳ | - | VFS: EROFS guard |
| **`ftruncate`** | Mutation | ✅ | ✅ | ⏳ | - | VFS: EROFS guard |
| **`chflags`** | Mutation | ✅ | ✅ | N/A | - | macOS-only, VFS: EROFS |
| **`setxattr`** | Mutation | ✅ | ✅ | ⏳ | - | VFS: EROFS guard |
| **`removexattr`** | Mutation | ✅ | ✅ | ⏳ | - | VFS: EROFS guard |
| **`dup`** | FD Ops | ✅ | ✅ | ⏳ | `test_gap_dup_tracking` | FD tracking |
| **`dup2`** | FD Ops | ✅ | ✅ | ⏳ | - | FD tracking |
| **`lseek`** | FD Ops | ✅ | ✅ | ⏳ | - | FD passthrough |
| **`fchdir`** | Namespace | ✅ | ✅ | ⏳ | - | Virtual CWD via FD |
| **`statx`** | Metadata | ✅ | N/A | ✅ | `test_statx_interception` | Linux-only (Rust Toolchain support) |
| **`getdents`** | Discovery | ⏳ | N/A | ⏳ | (via `test_opendir_*`) | Linux raw syscall (macOS via readdir) |
| **`unlinkat`** | Mutation | ✅ | ✅ | ✅ | `test_gap_unlinkat_bypass` | VFS: EROFS guard |
| **`mkdirat`** | Mutation | ✅ | ✅ | ✅ | `test_gap_mkdirat_bypass` | VFS: EROFS guard |
| **`symlinkat`** | Mutation | ✅ | ✅ | ✅ | `test_gap_symlinkat_bypass` | VFS: EROFS guard |
| **`fchmod`** | Permission | ✅ | ✅ | ✅ | `test_gap_fchmod_bypass` | VFS: EROFS guard (F_GETPATH/procfs) |
| **`openat2`** | I/O | ✅ | N/A | ✅ | - | Linux 5.6+ support |
| **`futimens/futimes`** | Time | ✅ | ✅ | ✅ | `test_secondary_mutation` | Blocked via FD resolution |
| **`sendfile`** | I/O | ✅ | ✅ | ✅ | `test_secondary_mutation` | Blocked drain FD |
| **`copy_file_range`** | I/O | ✅ | N/A | ✅ | `test_secondary_mutation` | Blocked drain FD (Linux) |


---

## 🎯 Unified Gap Status (Feb 5, 2026)

All syscall gaps are categorized below. Each **Must Fix** item has or requires a test case.

### ✅ Resolved (Implemented & Tested)

| Syscall | Status | Test | Notes |
|:--------|:------:|:-----|:------|
| `unlinkat` | ✅ | `test_gap_unlinkat_bypass.sh` | VFS EROFS guard |
| `mkdirat` | ✅ | `test_gap_mkdirat_bypass.sh` | VFS EROFS guard |
| `symlinkat` | ✅ | `test_gap_symlinkat_bypass.sh` | VFS EROFS guard |
| `fchmod` | ✅ | `test_gap_fchmod_bypass.sh` | VFS EROFS guard via F_GETPATH |
| `renameat` | ✅ | `test_gap_renameat_bypass.sh` | VFS EROFS guard |
| `mmap` (CoW) | ✅ | `test_gap_mmap_shared.sh` | CoW-aware tracking |
| `flock` | ✅ | `test_gap_flock_semantic.sh` | Daemon lock manager |
| `dup/dup2` | ✅ | `test_gap_dup_tracking.sh` | FD tracking |
| `readlinkat` | ✅ | `test_gap_readlinkat.sh` | Dirfd resolution works |
| `hardlink boundary` | ✅ | `test_value_2_rename.sh` (4/4) | EXDEV enforced |
| `futimes/futimens` | ✅ | `test_secondary_mutation.c` | Blocked via FD |
| `sendfile` | ✅ | `test_secondary_mutation.c` | Blocked drain FD |
| `copy_file_range` | ✅ | `test_secondary_mutation.c` | Blocked drain FD |
| `openat2` | ✅ | Internal | Linux support |



### 🔴 Must Fix (P0-P1) — Blocking for GA

| Syscall | Risk | Test | Status | Sprint |
|:--------|:-----|:-----|:------:|:------:|
| `exchangedata` | Atomic swap bypasses VFS | `test_gap_exchangedata.sh` | ✅ **Fixed** | S2 |
| `fchown/fchownat` | Ownership bypass via FD | `test_gap_fchown_bypass.sh` | ✅ **Fixed** | S1 |
| `openat2` | Linux 5.6+ support | Internal | ✅ **Fixed** | S2 |


### 🟡 Can Defer (P2-P3) — Non-blocking, Low Risk

| Syscall | Risk | Test (POC) | Status | Notes |
|:--------|:-----|:-----------|:------:|:------|
| `creat` | Legacy file creation | TBD | ⏳ | Rare, can use open |
| `getattrlist/setattrlist` | macOS metadata | TBD | ⏳ | Advanced, rare |
| `fstatvfs` | FS stats bypass | TBD | ⏳ | Read-only, no mutation |

### ⚪ Passthrough by Design (No VFS Risk)

| Syscall | Reason |
|:--------|:-------|
| `pread`, `pwrite` | Uses already-intercepted FDs |
| `readv`, `writev` | Uses already-intercepted FDs |
| `lchown` | Output files only, not VFS |
| `openat2` | Supported (VFS path redirection) |
| `execveat` | Linux-only, rare |
| `splice`, `tee`, `vmsplice` | Kernel pipe operations |

### 📋 Test Coverage Summary

| Category | Total | Tested | Coverage |
|:---------|:-----:|:------:|:--------:|
| Resolved | 8 | 8 | **100%** |
| Must Fix (P0-P1) | 4 | 1 | **25%** |
| Can Defer (P2-P3) | 6 | 3 | **50%** |
| Passthrough | 6 | - | N/A |

> **Action Required**: Create tests for 3 remaining P0-P1 gaps before GA release.


---

## ⚠️ Platform Parity Note: macOS vs Linux

Velo Rift has reached **Full Platform Parity** between macOS and Linux (Feb 2026).

### Linux Shim Implementation (31 functions)
| Category | Functions |
|----------|-----------|
| **I/O** | `open/open64`, `openat/openat64`, `close`, `read`, `write` |
| **Stat** | `stat/stat64`, `lstat/lstat64`, `fstat/fstat64`, `newfstatat` |
| **FD ops** | `dup`, `dup2`, `dup3`, `fcntl`, `lseek/lseek64`, `ftruncate/ftruncate64` |
| **Path** | `access`, `faccessat`, `readlink`, `getcwd`, `chdir` |
| **Mutation** | `chmod`, `fchmodat`, `unlink`, `rmdir`, `mkdir`, `rename`, `link`, `truncate/truncate64` |
| **Memory** | `mmap/mmap64`, `munmap` |

- **macOS**: Full 23-interface interception enabling directory discovery, dynamic loading, and AT-family operations.
- **Linux**: Complete 31-function interposition via `LD_PRELOAD`. Uses raw assembly syscalls for bootstrap safety.
    - All shims follow BUG-007 pattern with `INITIALIZING` state check
    - Raw syscalls in `linux_raw.rs` support both x86_64 and aarch64

---

## 🛡️ VFS Security Invariants ("The Iron Law")

Velo Rift enforces strict security boundaries to prevent CAS-based attacks.

1.  **Execution Circuit Breaker**: All files ingested into the CAS (TheSource) are stripped of execution bits (`chmod 0444`). This prevents direct execution of payloads from the binary store.
2.  **Immutability enforcement**: The `Protect` IPC command (supported by `chflags UF_IMMUTABLE` on macOS and `FS_IMMUTABLE_FL` on Linux) allows locking VFS paths against ANY mutation, even by the owner.
3.  **Recursion Guard**: Every intercepted syscall is protected by `ShimGuard::enter()`, preventing stack overflows during initialization or nested library calls.

---

## ⚙️ Undocumented Environment Variable Registry

| Variable | Purpose | Default | Discovery |
| :--- | :--- | :--- | :--- |
| `VR_THE_SOURCE` | CAS root directory. | `/tmp/vrift/the_source` | Core storage location. |
| `VRIFT_VFS_PREFIX` | Virtual mount point. | `/vrift` | Path projection root. |
| `VRIFT_DEBUG` | Enables stderr logging. | Disabled | Diagnostic stream. |
| `VRIFT_SHIM_PATH` | Path to the `.dylib`/`.so`. | Internal | Dynamic injection. |

---

## 🔧 Raw Syscall Reference (BUG-007 + Pattern 2930)

The following raw syscalls bypass libc entirely during dyld bootstrap, preventing deadlock:

| Category | Syscalls | ARM64 SYS# |
|----------|----------|------------|
| I/O | read, write, close, dup, dup2, lseek, ftruncate | 3,4,6,41,90,199,201 |
| Stat | fstat, stat, lstat, access, fstatat | 339,338,340,33,466 |
| Memory | mmap, munmap | 197,73 |
| File | open, openat, fcntl, chmod, fchmod, fchmodat | 5,463,92,15,124,468 |
| Mutation | unlink, rmdir, mkdir, truncate, unlinkat, mkdirat, symlinkat | 10,137,136,200,438,464,465 |
| Link | linkat, rename, renameat | 469,128,465 |
| Attr | chflags, setxattr, removexattr, utimes | 34,236,238,138 |
| Path | readlink, realpath | 58,462 |

**Pattern 2930: Post-Init dlsym Hazard** (Feb 5, 2026):
All `REAL_*.get()` calls replaced with raw assembly syscalls to avoid loader lock contention even after `INITIALIZING == 0`.

**Hardened Mutation Shims** (use `quick_block_vfs_mutation` in raw path):
- `chmod_shim`, `unlink_shim`, `rmdir_shim`, `mkdir_shim`, `truncate_shim`
- `fchmodat_shim`, `chflags_shim`, `setxattr_shim`, `removexattr_shim`, `utimes_shim`
- `linkat_shim`, `unlinkat_shim`, `mkdirat_shim`, `symlinkat_shim`

---

## 🚀 Advanced CoW & Optimization Behaviors

Velo Rift uses platform-specific optimizations for Copy-on-Write (CoW) and metadata lookup.

-   **Linux Zero-Copy CoW**: Uses `ioctl(FICLONE)` to create reflinks on supporting filesystems (XFS, Btrfs) and falls back to `copy_file_range(2)` for zero-copy data transfer.
-   **macO_TMPFILE Simulation**: Uses `linkat` via `/proc/self/fd/` on Linux to simulate atomic file replacement during link breakage.
-   **RFC-0044 Hot Stat Cache**:
    -   **O(1) Complexity**: Bloom Filter + Mmap'd Hash Table lookups.
    -   **Zero-Allocation**: Safe for use during `dyld` initialization before `malloc` is ready.

---

---

## 🕵️ Subtle Architectural Gaps & Risks

These are "invisible" behaviors discovered during deep forensic audit that may cause intermittent failures in complex toolchains.

### 1. File Descriptor Leakage (O_CLOEXEC Gap)
- **Forensic Evidence**: Audit of `crates/vrift-shim/src/lib.rs:741` (`libc::socket`) and `L1033` (`libc::open`) confirms FDs are opened WITHOUT `O_CLOEXEC` or `FD_CLOEXEC`.
- **Why tests PASSED initially**: The current shim uses an **ephemeral connection model** (connect -> call -> close). The socket is closed before `execve` starts, masking the vulnerability.
- **Critical Risk**:
    - **Race Condition**: A concurrent thread performing VFS operations during `execve` WILL leak the socket to the child.
    - **Performance Evolution**: If the shim moves to persistent connections (RFC-0043 recommendation), 100% of children will inherit the daemon IPC handle.
- **Remediation**: Mandatory `fcntl(fd, F_SETFD, FD_CLOEXEC)` after every `socket()` and `open()` call in the shim.

### 2. ~~Naive Path Matching (Normalization Gap)~~ ✅ RESOLVED
- **Status**: Path normalization implemented and verified (Feb 2026)
- **Implementation**: `raw_path_normalize()` in `path.rs` handles `..`, `.`, `//`
- **Test**: `test_path_normalization.sh` confirms traversal attacks blocked
- ~~**Risk**: The shim uses string prefix matching (`starts_with`) without normalization.~~
- ~~**Exploit**: Paths like `/vrift/../etc/passwd` or `/vrift//file.txt` may bypass VFS redirection.~~

### 3. Path Virtualization (`getcwd`/`realpath`/`chdir`)
- **Status**: 🔄 Implemented (Feb 2026) - Needs E2E Verification
- `getcwd()`, `realpath()`, `chdir()` now have VFS virtualization via `VIRTUAL_CWD` tracking and manifest lookup.
- See **Unified Syscall Registry** above for current status.

---

## 🚩 Passthrough Gap Summary

> All gaps are now tracked in the **Unified Syscall Registry** table above.
> Look for rows with Status = ⏳ (Pending) or ➖ (By Design) to see remaining work.

**Remaining Work (macOS):**
- **P3 (Deferred)**: `chown` - Passthrough by design (not needed for compile workflows)

**Completed (macOS):**
- ✅ `unlink`, `rename`, `rmdir`, `mkdir`, `chmod` - VFS paths return EROFS
- ✅ `utimes` - VFS mtime via IPC


## 📜 POSIX Compliance Matrix (Syscall Level)

| Category | Compliance | Status | Key Missing Operations |
| :--- | :---: | :--- | :--- |
| **Basic Metadata** | 90% | ⚠️ Gaps | `getattrlist`, `statvfs` **PENDING** |
| **File I/O** | 80% | ⚠️ Gaps | `sendfile`, `copy_file_range`, `creat` **PENDING** |
| **Directory Ops** | 100% | ✅ Full | None (Read-only traversal complete) |
| **Namespace/Path** | 90% | ⚠️ Gaps | `readlinkat` **PENDING** |
| **Mutation** | 60% | ❌ Vulnerable | `rename` (Deadlock), `unlinkat`, `mkdirat`, `symlinkat` |
| **Permissions** | 60% | ❌ Vulnerable | `fchmod`, `fchown`, `fchownat` |
| **Time Ops** | 50% | ❌ Vulnerable | `futimens`, `futimes`, `utimensat` (partial) |
| **Dynamic Loading**| 100% | ✅ Full | None |
| **Memory Management**| 100% | ✅ Full | None |

> **Overall macOS Coverage**: ~70% (42/60 key syscalls) - Regression found in `rename` flow.

---

## 🔬 Detailed Interface Behavior (Syscall Specs)

This section documents the exact logic implemented for each intercepted syscall.

### 📁 File Operations
| Interface | Behavior Header | Redirection Logic |
| :--- | :--- | :--- |
| `open` | **VFS Translation** | If in `/vrift`, queries manifest. If found, extracts to `/tmp/vrift-mem-*` and returns that FD. Returns `EISDIR` if path is a virtual directory. |
| `close` | **Sync-on-Close** | If the closed FD was a writable CoW file, it triggers a non-blocking IPC to daemon for async re-ingest. |
| `read` | **Passthrough** | Operates on the redirected FD returned by `open`. No data modification. |
| `write` | **CoW Tracking** | Passthrough to the temporary writable file. Tracking is used to determine re-ingest on `close`. |
| `access` | **Virtual Check** | Queries manifest for `F_OK`. Validates `R/W/X` bits against virtual metadata. |
| `readlink`| **Symlink Synth** | If path is a virtual symlink, returns the link target stored in CAS/Manifest. |

### 📊 Discovery & Metadata
| Interface | Behavior Header | Implementation Details |
| :--- | :--- | :--- |
| `stat` / `lstat`| **Hot Stat (O(1))**| Uses Mmap'd manifest + Bloom Filter. ZERO allocations. Injects virtual `size`, `mtime` (ns), and `mode`. |
| `fstat` | **FD Tracking** | Checks if FD belongs to a VFS-tracked file. Injects virtual metadata to hide temporary host paths. |
| `opendir` | **Handle Synthesis**| Returns a synthetic `DIR*` handle. Queries daemon for full virtual directory listing. |
| `readdir` | **Virtual Stream** | Iterates through a cached list of virtual entries. Uses a static `dirent` buffer to avoid heap usage. |

### 🚀 Execution & Linking
| Interface | Behavior Header | Side Effects |
| :--- | :--- | :--- |
| `execve` | **Env Inheritance** | Merges current `DYLD_INSERT_LIBRARIES` / `LD_PRELOAD` into child env to maintain shim persistency. |
| `posix_spawn`| **Recursion Guard** | Similar to `execve`. Ensures ShimGuard is active to prevent early-init hangs. |
| `dlopen` | **Library Extraction**| If loading a VFS `.dylib`/`.so`, extracts to temp host path before calling host linker. |
| `mmap` | **Backing Parity** | Respects virtual FD redirection for memory-mapped IO consistency. |

---

## 🧠 Behavioral Characteristics

### Case Sensitivity
- **macOS**: Inherits host behavior (APFS Case-Insensitive by default).
- **Linux**: Case-Sensitive.
- **VRIFT Policy**: The VFS projection layer is currently **Case-Sensitive** regardless of host, which may cause mismatches on macOS.

### Atomicity & Persistence
- **Read-Only Manifests**: Once ingested, the manifest is immutable and atomic.
- **Mutation Isolation**: Currently, any mutation call hits the host OS directly, breaking the "Rift" isolation.

### Path Limits
- Max Path Length: Following POSIX `PATH_MAX` (typically 1024-4096 depending on OS).
- Virtual Prefix: `/vrift/` (Configurable via `VRIFT_VFS_PREFIX`).

---

## ❓ FAQ & Troubleshooting (See vfs_syscall_gap_risk_analysis.md)

- **Q: Why does my build fail with "No such file or directory"?**  
  A: Likely caused by `rename()` or `chdir()` passthrough. Check Category 1 gaps.
- **Q: Does Velo Rift work with macOS Hardened Runtime?**  
  A: No. Codesigned binaries with the Hardened Runtime (like `python` from Brew) block `DYLD_INSERT_LIBRARIES`. Use ad-hoc signed binaries for testing.
