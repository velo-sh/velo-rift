# Velo Rift™ Compatibility Guide

This document tracks the support status of system calls and common toolchains within the Velo Rift VFS environment. Use this to troubleshoot issues or understand the current limitations of the shim.

## 🛠️ Toolchain Support Status

| Toolchain | Status | Notes |
| :--- | :--- | :--- |
| **GCC / Clang** | ⚠️ Partial | Read/Compile works. Link/Rename fails (See Category 1). |
| **Make / Ninja** | ❌ Broken | Parallel builds and directory changes (`chdir`) are currently unsupported. |
| **Git** | ❌ Broken | Relies heavily on `rename` and `unlink` for index management. |
| **Node.js** | ⚠️ Partial | Basic execution works. Complex module loading with `pwrite` may fail. |
| **Python** | ✅ Good | Most standard library operations (Read/Stat) are fully supported. |

---

## 🔍 Syscall Support Matrix

### Category 1: Metadata & Read (Stable)
| Syscall | Status | Verified By |
| :--- | :--- | :--- |
| `stat` / `lstat` / `fstat` | ✅ 100% | `test_fstat_basic.sh` |
| `open` / `close` | ✅ 100% | `test_cow.sh` |
| `read` | ✅ 100% | Internal VFS core |
| `opendir` / `readdir` | ✅ 100% | `test_readdir.sh` |

### Category 2: Dynamic Loading (Stable)
| Syscall | Status | Verified By |
| :--- | :--- | :--- |
| `dlopen` | ✅ 100% | `test_dlopen_vfs.sh` |
| `dlsym` | ✅ 100% | `test_dlsym_interception.sh` |
| `mmap` / `munmap` | ✅ 100% | `test_munmap_interception.sh` |

### Category 3: Context & Mutation (Developing) 🚩
| Syscall | Status | Risk / Failure Proof |
| :--- | :--- | :--- |
| `rename` | ❌ Passthrough | [test_fail_rename_leak.sh](file:///Users/antigravity/rust_source/vrift_qa/tests/poc/test_fail_rename_leak.sh) |
| `unlink` | ❌ Passthrough | [test_fail_unlink_cas.sh](file:///Users/antigravity/rust_source/vrift_qa/tests/poc/test_fail_unlink_cas.sh) |
| `chdir` / `getcwd` | ❌ Passthrough | [test_fail_cwd_leak.sh](file:///Users/antigravity/rust_source/vrift_qa/tests/poc/test_fail_cwd_leak.sh) |
| `mkdir` / `rmdir` | ❌ Passthrough | Hits Host OS directly |

---

## ❓ FAQ & Troubleshooting

### Why does my build fail with "No such file or directory" during a rename?
Velo Rift currently does not intercept `rename()`. If your tool tries to rename a file within a `/vrift` path, the call hits the host OS, which does not recognize the virtual path. This is a **P0 priority** for the next development phase.

### Why does `pwd` show a physical path inside a virtual directory?
We currently do not shim `getcwd()`. While `open()` and `stat()` calls will still work via path-based redirection, the process doesn't "know" it's in a virtual path via its working directory status.

### How do I report a missing syscall?
Run `bash tests/poc/test_compiler_syscall_coverage.sh` and attach the output to a new issue.
