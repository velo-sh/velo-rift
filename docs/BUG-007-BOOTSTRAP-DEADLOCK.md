# BUG-007: malloc/fstat Bootstrap Deadlock on macOS ARM64

> **Status: ✅ RESOLVED AND VERIFIED** (Feb 3, 2026)
> 
> All related tests passing:
> - `test_bug007_bootstrap.sh` ✅
> - `test_concurrent_init.sh` ✅
> - `test_init_state.sh` ✅
> - `test_issue1_recursion_deadlock.sh` ✅
> - `test_issue2_tls_bootstrap_hang.sh` ✅
> 
> Latest verification: Commit `6c79872` | 45+ tests PASS | 0 FAIL

## Summary

When using `DYLD_INSERT_LIBRARIES` to inject vrift-shim into a process on macOS ARM64, the process would hang with high CPU usage during the dyld bootstrap phase.


## Symptoms

- Process hangs immediately after launch
- CPU usage spikes to 100% on one core
- `sample` output shows deep recursive call stack in `fstat_shim`
- Occurs with `DYLD_INSERT_LIBRARIES` and optionally `DYLD_FORCE_FLAT_NAMESPACE=1`

## Root Cause Analysis

### Call Chain Leading to Deadlock

```
dyld (dynamic linker)
  └── _dyld_start
       └── libSystem_initializer
            └── __malloc_init
                 └── _os_feature_table_once
                      └── fstat(fd, &sb)
                           └── [INTERPOSED] fstat_shim
                                └── dlsym(RTLD_NEXT, "fstat")
                                     └── malloc (NOT INITIALIZED YET!)
                                          └── DEADLOCK/INFINITE RECURSION
```

### Why This Happens

1. **Timing**: macOS calls `fstat` inside `__malloc_init` BEFORE malloc is ready to service requests.

2. **Interposition**: With `DYLD_INSERT_LIBRARIES` active, the `__DATA,__interpose` section causes all `fstat` calls to redirect to `fstat_shim`.

3. **dlsym Dependency**: The original `fstat_shim` implementation used:
   ```rust
   let real = REAL_FSTAT.get();  // Calls dlsym(RTLD_NEXT, "fstat")
   ```
   But `dlsym` internally allocates memory, requiring malloc.

4. **old_func Trap**: We tried using `IT_FSTAT.old_func` (the interpose table's original function pointer), but with `DYLD_FORCE_FLAT_NAMESPACE=1`, this pointer is resolved to the same symbol that got interposed — creating infinite recursion.

5. **RwLock Hazard**: Even bypassing dlsym, calling helper functions like `get_fd_entry()` uses `RwLock::read()` which may trigger pthread operations that also require initialized memory allocators.

## Solution

### Use Raw Assembly Syscalls

Created `crates/vrift-shim/src/syscalls/macos_raw.rs` with inline assembly syscall wrappers:

```rust
#[inline(never)]
pub unsafe fn raw_fstat64(fd: c_int, buf: *mut stat) -> c_int {
    let ret: i64;
    asm!(
        "mov x16, {syscall}",  // Syscall number in x16
        "svc #0x80",            // Trap to kernel
        syscall = in(reg) 339,  // SYS_fstat64
        in("x0") fd,
        in("x1") buf,
        lateout("x0") ret,
        options(nostack)
    );
    if ret < 0 { -1 } else { ret as c_int }
}
```

This has **ZERO dependencies** on libc, pthread, or malloc.

### Updated Shim Pattern

```rust
pub unsafe extern "C" fn fstat_shim(fd: c_int, buf: *mut stat) -> c_int {
    // Check if still in early bootstrap (INITIALIZING >= 2)
    if INITIALIZING.load(Relaxed) >= 2 {
        return raw_fstat64(fd, buf);  // Use raw syscall
    }

    // Check recursion guard (uses TLS, safe after INITIALIZING < 2)
    let _guard = match ShimGuard::enter() {
        Some(g) => g,
        None => return raw_fstat64(fd, buf),  // In recursion, use raw
    };

    // Normal VFS logic...
    // ...
    
    // Default fallback: raw syscall (safest option)
    raw_fstat64(fd, buf)
}
```

## Affected Shims

| Shim | Raw Syscall | Syscall Number (ARM64) |
|------|-------------|------------------------|
| `fstat_shim` | `raw_fstat64` | 339 |
| `stat_shim` | `raw_stat` | 338 |
| `lstat_shim` | `raw_lstat` | 340 |
| `read_shim` | `raw_read` | 3 |
| `write_shim` | `raw_write` | 4 |
| `close_shim` | `raw_close` | 6 |
| `dup_shim` | `raw_dup` | 41 |
| `dup2_shim` | `raw_dup2` | 90 |
| `chmod_shim` | `raw_chmod` | 15 |
| `mmap_shim` | `raw_mmap` | 197 |
| `munmap_shim` | `raw_munmap` | 73 |
| `access_shim` | `raw_access` | 33 |
| `openat_shim` | `raw_openat` | 463 |
| `fcntl_shim` | `raw_fcntl` | 92 |
| `lseek_shim` | `raw_lseek` | 199 |
| `ftruncate_shim` | `raw_ftruncate` | 201 |

## Pattern 3136: Initialization Helper Recursion

While direct interposition recursion was resolved (above), a secondary deadlock pattern was identified during verification of complex toolchains (e.g., Cargo/Rustc).

### Problem

Initialization helpers like `boost_fd_limit()` or `setup_logging()` are often called early in the shim's `init` phase. If these helpers call standard libc functions that are also shimmed, they trigger the same deadlock loop because the "initialized" state is not yet reached.

Example:
```text
InceptionLayer Init
  └── boost_fd_limit()
       └── getrlimit()
            └── [INTERPOSED] getrlimit_shim
                 └── [WAITING FOR INIT LOCK] -> DEADLOCK
```

### Remediation: Total Zero-Dependency Initialization

The Inception Layer has adopted a **Total Zero-Dependency Initialization** mandate. All internal state setup helpers must use raw assembly escapes or internal-only static logic:

1. **Raw Assembly Wrappers**: Helpers like `getrlimit` must be replaced with `raw_getrlimit`.
2. **Static Buffer Logging**: Initial logs are written to a fixed-size static buffer or via `raw_write`.
3. **No Pthread/Malloc**: Helpers must not use any primitive that might trigger internal interposition.

### Additional Guard: SHIM_STATE Check

For shims that call `block_vfs_mutation()` (chmod, unlink, rmdir, etc.), an extra check 
for `SHIM_STATE.is_null()` is required to avoid TLS pthread deadlocks:

```rust
if init_state >= 2 || SHIM_STATE.load(Ordering::Acquire).is_null() {
    return raw_chmod(path, mode);
}
```

## Testing

```bash
# Create test binary
echo '#include <stdio.h>\nint main() { puts("OK"); }' | cc -x c - -o /tmp/test
codesign -f -s - /tmp/test

# Test with shim injection
DYLD_INSERT_LIBRARIES=target/release/libvrift_shim.dylib \
DYLD_FORCE_FLAT_NAMESPACE=1 \
/tmp/test

# Expected output: "OK" (not a hang)
```

## Debugging Tips

If you encounter similar hangs:

1. Use `sample <PID> 1` to capture the call stack
2. Look for recursive patterns in `*_shim` functions
3. Check if the recursion involves `dlsym`, `malloc`, or `pthread_*`
4. The solution is always to use raw syscalls for bootstrap-phase operations

## References

- `/usr/include/sys/syscall.h` — macOS syscall numbers
- Apple ARM64 Calling Convention documentation
- `crates/vrift-shim/src/syscalls/linux_raw.rs` — similar pattern for Linux
- Pattern 2682: Raw Assembly Syscall Wrappers

### Final Verification Results

*   **`test_boot_safety.sh`**: ✅ **PASS** (Confirmed bootstrap-safe)
*   **`test_e2e_gcc_compile.sh`**: ✅ **PASS** (Verified mutation interposition)
*   **Regression Suite**: ✅ **PASS** (IPC v4 and legacy name issues fixed)
*   **Current Status**: **RESOLVED & VERIFIED**. Fix confirmed via raw syscall allocation proxy.
