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

## 🛠️ Toolchain & Runtime Compatibility

| Category | Typical Tools | Compatibility | Blocking Reason |
| :--- | :--- | :--- | :--- |
| **Compilers** | `gcc`, `clang`, `rustc` | ⚠️ Partial | Failure to `rename` temp objects. |
| **Build Systems** | `make`, `ninja`, `bazel` | ❌ Broken | `chdir` and `unlink` dependency. |
| **Package Mgrs** | `npm`, `pnpm`, `cargo` | ❌ Broken | Heavy use of atomic `rename`. |
| **Runtimes** | `node`, `python`, `jvm` | ✅ Good/High | Basic Execution & Read paths stable. |
| **VCS** | `git`, `hg` | ❌ Broken | Structural mutations on `.git`. |

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
