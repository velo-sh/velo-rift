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

---

# BUG-007b: Multithreaded Stress Deadlock (Stack Overflow + Recursive IPC)

> **Status: ✅ RESOLVED AND VERIFIED** (Feb 7, 2026)
>
> `repro_rwlock_stress.sh` (10 threads × 100 opens): ✅ PASS
> Full qa_v2 regression: ✅ 19/19 PASS
>
> Commit: `392ed02c`

## Summary

When 10+ threads simultaneously call `open()` through the inception layer, the process
hangs indefinitely. Unlike BUG-007 (malloc recursion), this deadlock is **silent** — no
CPU spike, no crash, no error messages. All threads freeze in the prologue of
`InceptionLayerState::get()`.

## Symptom Profile

| Aspect | BUG-007 (malloc) | BUG-007b (stress) |
|--------|-------------------|--------------------|
| Trigger | Single-threaded, dyld bootstrap | Multi-threaded, post-bootstrap |
| CPU | 100% (spin loop) | 0% (all threads blocked) |
| `sample` output | Recursive fstat_shim | Threads stuck at `get() + 52` |
| Visual | Visible infinite recursion | Silent freeze, no stack growth |

## Root Cause: Five Interacting Pitfalls

### Pitfall 1: Stack Overflow from Function Inlining (PRIMARY)

The compiler inlined `init()` + `open_manifest_mmap()` into `get()`, creating a
**605KB combined stack frame**. macOS pthread stacks default to **512KB**.

```
get() prologue (ARM64 disassembly):
    sub x9, sp, #602112      ; 605KB frame allocation
    sub x9, x9, #3360
loop:
    sub sp, sp, #4096         ; touch each page (stack probe)
    cmp sp, x9
    b.gt loop                 ; <-- ALL THREADS STUCK HERE (offset +0x34)
```

Every thread calling `open()` → `open_impl()` → `get()` immediately overflows the
stack in the function prologue. The stack probe touches unmapped guard pages, causing
the thread to permanently block in a kernel page fault.

**Fix**: `#[inline(never)]` + `#[cold]` on `init()` and `open_manifest_mmap()`.

```
Before: sub x9, sp, #602112   ; 605KB stack frame 💀
After:  sub sp, sp, #64       ; 64B stack frame ✅
```

**Rule**: Any function >10KB stack that may be called from interposed syscall paths
MUST be marked `#[inline(never)]`. The compiler cannot know about pthread stack limits.

### Pitfall 2: Interposed libc Calls in IPC Layer

`raw_unix_connect()` and `sync_rpc()` used standard `libc::access()`, `libc::close()`,
and `libc::fcntl()` — all of which are interposed by the shim itself.

```
open() → open_inception → sync_rpc() → libc::access() → access_inception → sync_rpc() → ...
                                                                              ↑ RECURSIVE IPC
```

Under high concurrency, this recursive IPC floods the daemon socket with duplicate
RegisterWorkspace/ManifestGet requests, eventually deadlocking the event loop.

**Fix**: Replace with raw syscall equivalents:
- `libc::access()` → `raw_access()`
- `libc::close()` → `ipc_raw_close()` (wrapper around `raw_close()`)
- `libc::fcntl()` → `raw_fcntl()`

**Rule**: **ALL libc calls inside `ipc.rs` MUST use raw syscalls.** The IPC layer is the
lowest-level communication path — any interposed call here creates infinite recursion.

### Pitfall 3: `std::fs::canonicalize()` in Init Path

`canonicalize()` internally calls `stat()`, `lstat()`, and `readlink()` — all interposed.
During `init()` (INITIALIZING=Busy), `stat_inception` does raw passthrough, so this
*usually* works. But `canonicalize()` also allocates heap memory (`PathBuf`), which can
trigger `mmap` or other interposed calls depending on allocator state.

Three locations were affected:
1. `init()` → VRIFT_VFS_PREFIX canonicalization
2. `init()` → VRIFT_MANIFEST project root canonicalization
3. `open_manifest_mmap()` → project root for VDir path

**Fix**: Replace with `raw_realpath()` (macOS raw syscall wrapper) using stack buffers.

**Rule**: Never use `std::fs::*` functions during initialization. They are convenience
wrappers that internally call multiple interposed syscalls and allocate heap memory.

### Pitfall 4: `canonicalize()` in `sync_rpc()`

Even outside of init, `sync_rpc()` called `std::fs::canonicalize()` on the manifest
path and project root before sending IPC requests. Since `sync_rpc()` is called from
within interposed syscall handlers, this triggered recursive interposition:

```
open → open_impl → sync_rpc → canonicalize → stat → stat_inception → open_impl → ...
```

**Fix**: Removed `canonicalize()` entirely from `sync_rpc()`. Environment variable paths
from `VR_THE_SOURCE`, `VRIFT_MANIFEST`, `VRIFT_SOCKET_PATH` are already absolute.

**Rule**: `sync_rpc()` must be zero-overhead. No filesystem operations, no allocations,
no string transformations beyond what is strictly necessary for the IPC protocol.

### Pitfall 5: TLS Recursion Guard Disabled by BOOTSTRAPPING Flag

`get_recursion_key()` reused the `BOOTSTRAPPING` atomic flag to protect TLS key creation.
But `InceptionLayerGuard::enter()` sets `BOOTSTRAPPING = true` before calling
`get_recursion_key()`, so the recursion guard was **permanently disabled**:

```
enter() → BOOTSTRAPPING = true → get_recursion_key() → sees BOOTSTRAPPING=true → return 0
                                                        ↑ GUARD NEVER ACTIVATES
```

With the guard disabled, recursive calls (from Pitfalls 2-4) were not detected,
allowing unbounded recursion depth.

**Fix**: Dedicated `TLS_KEY_LOCK: AtomicBool` for TLS key initialization, separate from
`BOOTSTRAPPING`.

**Rule**: Each initialization phase must have its own lock. Never reuse atomic flags
across different initialization stages — the ordering assumptions will be violated.

## Golden Rules for Inception Layer Development

These rules are derived from all BUG-007/007b incidents:

1. **Raw syscalls only in IPC**: Every function in `ipc.rs` must use `raw_*` syscalls.
   No `libc::*` calls, no `std::fs::*`, no `std::io::*`.

2. **No inlining of init paths**: `init()`, `open_manifest_mmap()`, and any function
   with >4KB of local variables must be `#[inline(never)]`.

3. **No `std::fs::canonicalize()` in hot paths**: Use `raw_realpath()` with stack
   buffers, or skip canonicalization entirely if paths are already absolute.

4. **Separate locks per init phase**: Never reuse atomic flags across different
   initialization stages. Each phase gets its own synchronization primitive.

5. **Test with threads**: Always run `repro_rwlock_stress.sh` after touching `state.rs`,
   `ipc.rs`, or `interpose.rs`. Single-threaded tests miss stack overflow and race
   conditions.

6. **`sample <PID> 1` is your best friend**: When a process hangs silently, use
   `sample` to capture thread state. Look for:
   - All threads at same offset → stack overflow
   - Threads in `dlsym`/`malloc` → bootstrap recursion (BUG-007)
   - Threads in `raw_unix_connect`/`sync_rpc` → IPC recursion (BUG-007b)

## Diagnostic Cheat Sheet

```bash
# Reproduce the stress test
bash tests/qa_v2/repro_rwlock_stress.sh

# Manual reproduction with sample capture
gcc -O3 tests/qa_v2/repro_rwlock_stress.c -o /tmp/repro -lpthread
DYLD_INSERT_LIBRARIES=target/release/libvrift_inception_layer.dylib \
DYLD_FORCE_FLAT_NAMESPACE=1 \
VRIFT_SOCKET_PATH=/tmp/vrift.sock \
VR_THE_SOURCE=/tmp/cas \
VRIFT_MANIFEST=/tmp/project/.vrift/manifest.lmdb \
VRIFT_VFS_PREFIX=/tmp/project \
/tmp/repro /tmp/project/src/file.txt &
PID=$!; sleep 3; sample $PID 1; kill $PID

# Verify stack frame size after changes
objdump -d target/release/libvrift_inception_layer.dylib | \
  grep -A 5 "InceptionLayerState.*get.*:"
# Look for: sub sp, sp, #<small number> (should be <4096)
```

## Files Modified

| File | Changes |
|------|---------|
| `state.rs` | `#[inline(never)]` on `init()`/`open_manifest_mmap()`, `raw_realpath` ×3, `TLS_KEY_LOCK` |
| `ipc.rs` | `ipc_raw_close()` helper, `raw_access`/`raw_fcntl`, removed `canonicalize()` |
