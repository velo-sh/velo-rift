# Velo Rift™ Comprehensive Compatibility Report

This report provides the definitive status of Velo Rift's compatibility with host environments, POSIX standards, and industrial toolchains.

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

## 📜 POSIX Compliance Matrix (Syscall Level)

| Category | Compliance | Status | Key Missing Operations |
| :--- | :---: | :--- | :--- |
| **Basic Metadata** | 95% | ✅ Strong | `statx` (Linux-specific partial) |
| **File I/O** | 90% | ✅ Strong | `preadv`/`pwritev`, `sendfile` |
| **Directory Ops** | 100% | ✅ Strong | None (Read-only traversal complete) |
| **Namespace/Path** | 60% | ⚠️ Partial | `chdir`, `fchdir`, `getcwd` |
| **Mutation (P0)** | 10% | 🛑 **Critical Gap** | `unlink`, `rename`, `mkdir`, `rmdir` |
| **Permissions** | 80% | ✅ Good | `chmod`, `chown` (Passthrough risks) |
| **Dynamic Loading**| 100% | ✅ Full | None |
| **Memory Management**| 100% | ✅ Full | None |

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
