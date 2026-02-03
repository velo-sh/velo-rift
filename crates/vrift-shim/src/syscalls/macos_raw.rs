//! Raw macOS ARM64 syscall wrappers for bootstrap-safe operations.
//!
//! # BUG-007: malloc/fstat Bootstrap Deadlock
//!
//! ## Problem Discovery
//!
//! When using `DYLD_INSERT_LIBRARIES` to inject vrift-shim into a process,
//! the process would hang during dyld bootstrap on macOS ARM64.
//!
//! Stack trace analysis using `sample` revealed:
//! ```text
//! dyld -> libSystem_initializer -> __malloc_init -> _os_feature_table_once -> fstat
//!                                                                               ↓
//!                                                                         fstat_shim (infinite recursion)
//! ```
//!
//! ## Root Cause Analysis
//!
//! 1. **Timing**: `fstat` is called inside `__malloc_init` BEFORE malloc is ready
//!
//! 2. **Interpose Redirection**: With `DYLD_INSERT_LIBRARIES` active, all calls to
//!    `fstat` get redirected to our `fstat_shim` via the `__DATA,__interpose` section
//!
//! 3. **dlsym Dependency**: `fstat_shim` was using `dlsym(RTLD_NEXT)` to get the
//!    real fstat pointer. But `dlsym` internally uses malloc (not yet initialized)!
//!
//! 4. **IT_FSTAT.old_func Trap**: We tried using the interpose table's `old_func`
//!    pointer, but with `DYLD_FORCE_FLAT_NAMESPACE=1`, even this points back to
//!    our shim, creating infinite recursion.
//!
//! 5. **RwLock Hazard**: Even if we bypass dlsym, calling `get_fd_entry()` uses
//!    `RwLock::read()` which may internally call syscalls that get interposed.
//!
//! ## Solution
//!
//! Use **inline assembly to directly invoke syscalls**, completely bypassing libc.
//!
//! On macOS ARM64:
//! - Syscall number goes in x16
//! - Arguments in x0-x5
//! - `svc #0x80` triggers the syscall
//! - Return value in x0
//!
//! This approach has ZERO dependencies on libc, malloc, or any other library.
//!
//! ## Affected Shims
//!
//! The following shims use raw syscalls during early init (`INITIALIZING >= 2`)
//! or when in recursion (ShimGuard fails):
//!
//! - `fstat_shim` → [`raw_fstat64`] (SYS_fstat64 = 339)
//! - `close_shim` → [`raw_close`] (SYS_close = 6)
//! - `mmap_shim` → [`raw_mmap`] (SYS_mmap = 197)
//! - `munmap_shim` → [`raw_munmap`] (SYS_munmap = 73)
//! - `access_shim` → [`raw_access`] (SYS_access = 33)
//!
//! ## References
//!
//! - Syscall numbers: `/usr/include/sys/syscall.h`
//! - ARM64 ABI: Apple ARM64 Function Calling Conventions
//! - Pattern 2682: Raw Assembly Syscall Wrappers (linux_raw.rs)

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
use std::arch::asm;

/// SYS_fstat64 = 339 on macOS
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
const SYS_FSTAT64: i64 = 339;

/// SYS_close = 6 on macOS
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
const SYS_CLOSE: i64 = 6;

/// SYS_mmap = 197 on macOS (actually uses mmap variant)
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
const SYS_MMAP: i64 = 197;

/// SYS_munmap = 73 on macOS
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
const SYS_MUNMAP: i64 = 73;

/// SYS_access = 33 on macOS
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
const SYS_ACCESS: i64 = 33;

/// SYS_stat64 = 338 on macOS
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
const SYS_STAT64: i64 = 338;

/// SYS_lstat64 = 340 on macOS
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
const SYS_LSTAT64: i64 = 340;

/// Raw stat64 syscall for macOS ARM64.
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[inline(never)]
pub unsafe fn raw_stat(path: *const libc::c_char, buf: *mut libc::stat) -> libc::c_int {
    let ret: i64;
    asm!(
        "mov x16, {syscall}",
        "svc #0x80",
        syscall = in(reg) SYS_STAT64,
        in("x0") path as i64,
        in("x1") buf as i64,
        lateout("x0") ret,
        options(nostack)
    );
    if ret < 0 {
        -1
    } else {
        ret as libc::c_int
    }
}

/// Raw lstat64 syscall for macOS ARM64.
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[inline(never)]
pub unsafe fn raw_lstat(path: *const libc::c_char, buf: *mut libc::stat) -> libc::c_int {
    let ret: i64;
    asm!(
        "mov x16, {syscall}",
        "svc #0x80",
        syscall = in(reg) SYS_LSTAT64,
        in("x0") path as i64,
        in("x1") buf as i64,
        lateout("x0") ret,
        options(nostack)
    );
    if ret < 0 {
        -1
    } else {
        ret as libc::c_int
    }
}

/// Raw fstat64 syscall for macOS ARM64.
/// Returns 0 on success, -1 on error (with errno-style negative return).
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[inline(never)]
pub unsafe fn raw_fstat64(fd: libc::c_int, buf: *mut libc::stat) -> libc::c_int {
    let ret: i64;
    asm!(
        "mov x16, {syscall}",
        "svc #0x80",
        syscall = in(reg) SYS_FSTAT64,
        in("x0") fd as i64,
        in("x1") buf as i64,
        lateout("x0") ret,
        options(nostack)
    );
    if ret < 0 {
        -1
    } else {
        ret as libc::c_int
    }
}

/// Raw close syscall for macOS ARM64.
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[inline(never)]
pub unsafe fn raw_close(fd: libc::c_int) -> libc::c_int {
    let ret: i64;
    asm!(
        "mov x16, {syscall}",
        "svc #0x80",
        syscall = in(reg) SYS_CLOSE,
        in("x0") fd as i64,
        lateout("x0") ret,
        options(nostack)
    );
    if ret < 0 {
        -1
    } else {
        ret as libc::c_int
    }
}

/// Raw mmap syscall for macOS ARM64.
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[inline(never)]
pub unsafe fn raw_mmap(
    addr: *mut libc::c_void,
    len: libc::size_t,
    prot: libc::c_int,
    flags: libc::c_int,
    fd: libc::c_int,
    offset: libc::off_t,
) -> *mut libc::c_void {
    let ret: i64;
    asm!(
        "mov x16, {syscall}",
        "svc #0x80",
        syscall = in(reg) SYS_MMAP,
        in("x0") addr as i64,
        in("x1") len as i64,
        in("x2") prot as i64,
        in("x3") flags as i64,
        in("x4") fd as i64,
        in("x5") offset,
        lateout("x0") ret,
        options(nostack)
    );
    ret as *mut libc::c_void
}

/// Raw munmap syscall for macOS ARM64.
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[inline(never)]
pub unsafe fn raw_munmap(addr: *mut libc::c_void, len: libc::size_t) -> libc::c_int {
    let ret: i64;
    asm!(
        "mov x16, {syscall}",
        "svc #0x80",
        syscall = in(reg) SYS_MUNMAP,
        in("x0") addr as i64,
        in("x1") len as i64,
        lateout("x0") ret,
        options(nostack)
    );
    if ret < 0 {
        -1
    } else {
        ret as libc::c_int
    }
}

/// Raw access syscall for macOS ARM64.
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[inline(never)]
pub unsafe fn raw_access(path: *const libc::c_char, mode: libc::c_int) -> libc::c_int {
    let ret: i64;
    asm!(
        "mov x16, {syscall}",
        "svc #0x80",
        syscall = in(reg) SYS_ACCESS,
        in("x0") path as i64,
        in("x1") mode as i64,
        lateout("x0") ret,
        options(nostack)
    );
    if ret < 0 {
        -1
    } else {
        ret as libc::c_int
    }
}

// =============================================================================
// macOS x86_64 implementations
// =============================================================================

/// SYS_fstat64 on macOS x86_64 = 339
#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
const SYS_FSTAT64_X64: i64 = 339;

/// SYS_close on macOS x86_64 = 6
#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
const SYS_CLOSE_X64: i64 = 6;

/// SYS_mmap on macOS x86_64 = 197
#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
const SYS_MMAP_X64: i64 = 197;

/// SYS_munmap on macOS x86_64 = 73
#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
const SYS_MUNMAP_X64: i64 = 73;

/// SYS_access on macOS x86_64 = 33
#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
const SYS_ACCESS_X64: i64 = 33;

/// SYS_stat64 on macOS x86_64 = 338
#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
const SYS_STAT64_X64: i64 = 338;

/// SYS_lstat64 on macOS x86_64 = 340
#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
const SYS_LSTAT64_X64: i64 = 340;

/// Raw stat64 syscall for macOS x86_64.
#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
#[inline(never)]
pub unsafe fn raw_stat(path: *const libc::c_char, buf: *mut libc::stat) -> libc::c_int {
    let ret: i64;
    std::arch::asm!(
        "syscall",
        in("rax") SYS_STAT64_X64 | 0x2000000,
        in("rdi") path as i64,
        in("rsi") buf as i64,
        lateout("rax") ret,
        lateout("rcx") _,
        lateout("r11") _,
        options(nostack)
    );
    if ret as isize > -4096 && (ret as isize) < 0 {
        -1
    } else {
        ret as libc::c_int
    }
}

/// Raw lstat64 syscall for macOS x86_64.
#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
#[inline(never)]
pub unsafe fn raw_lstat(path: *const libc::c_char, buf: *mut libc::stat) -> libc::c_int {
    let ret: i64;
    std::arch::asm!(
        "syscall",
        in("rax") SYS_LSTAT64_X64 | 0x2000000,
        in("rdi") path as i64,
        in("rsi") buf as i64,
        lateout("rax") ret,
        lateout("rcx") _,
        lateout("r11") _,
        options(nostack)
    );
    if ret as isize > -4096 && (ret as isize) < 0 {
        -1
    } else {
        ret as libc::c_int
    }
}

/// Raw fstat64 syscall for macOS x86_64.
#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
#[inline(never)]
pub unsafe fn raw_fstat64(fd: libc::c_int, buf: *mut libc::stat) -> libc::c_int {
    let ret: i64;
    std::arch::asm!(
        "syscall",
        in("rax") SYS_FSTAT64_X64 | 0x2000000, // macOS syscall class
        in("rdi") fd as i64,
        in("rsi") buf as i64,
        lateout("rax") ret,
        lateout("rcx") _,
        lateout("r11") _,
        options(nostack)
    );
    if ret as isize > -4096 && (ret as isize) < 0 {
        -1
    } else {
        ret as libc::c_int
    }
}

/// Raw close syscall for macOS x86_64.
#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
#[inline(never)]
pub unsafe fn raw_close(fd: libc::c_int) -> libc::c_int {
    let ret: i64;
    std::arch::asm!(
        "syscall",
        in("rax") SYS_CLOSE_X64 | 0x2000000,
        in("rdi") fd as i64,
        lateout("rax") ret,
        lateout("rcx") _,
        lateout("r11") _,
        options(nostack)
    );
    if ret as isize > -4096 && (ret as isize) < 0 {
        -1
    } else {
        ret as libc::c_int
    }
}

/// Raw mmap syscall for macOS x86_64.
#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
#[inline(never)]
pub unsafe fn raw_mmap(
    addr: *mut libc::c_void,
    len: libc::size_t,
    prot: libc::c_int,
    flags: libc::c_int,
    fd: libc::c_int,
    offset: libc::off_t,
) -> *mut libc::c_void {
    let ret: i64;
    std::arch::asm!(
        "syscall",
        in("rax") SYS_MMAP_X64 | 0x2000000,
        in("rdi") addr as i64,
        in("rsi") len as i64,
        in("rdx") prot as i64,
        in("r10") flags as i64,
        in("r8") fd as i64,
        in("r9") offset,
        lateout("rax") ret,
        lateout("rcx") _,
        lateout("r11") _,
        options(nostack)
    );
    ret as *mut libc::c_void
}

/// Raw munmap syscall for macOS x86_64.
#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
#[inline(never)]
pub unsafe fn raw_munmap(addr: *mut libc::c_void, len: libc::size_t) -> libc::c_int {
    let ret: i64;
    std::arch::asm!(
        "syscall",
        in("rax") SYS_MUNMAP_X64 | 0x2000000,
        in("rdi") addr as i64,
        in("rsi") len as i64,
        lateout("rax") ret,
        lateout("rcx") _,
        lateout("r11") _,
        options(nostack)
    );
    if ret as isize > -4096 && (ret as isize) < 0 {
        -1
    } else {
        ret as libc::c_int
    }
}

/// Raw access syscall for macOS x86_64.
#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
#[inline(never)]
pub unsafe fn raw_access(path: *const libc::c_char, mode: libc::c_int) -> libc::c_int {
    let ret: i64;
    std::arch::asm!(
        "syscall",
        in("rax") SYS_ACCESS_X64 | 0x2000000,
        in("rdi") path as i64,
        in("rsi") mode as i64,
        lateout("rax") ret,
        lateout("rcx") _,
        lateout("r11") _,
        options(nostack)
    );
    if ret as isize > -4096 && (ret as isize) < 0 {
        -1
    } else {
        ret as libc::c_int
    }
}

// =============================================================================
// Linux fallback (redirects to linux_raw.rs)
// =============================================================================

#[cfg(target_os = "linux")]
pub unsafe fn raw_fstat64(fd: libc::c_int, buf: *mut libc::stat) -> libc::c_int {
    crate::syscalls::linux_raw::raw_fstat(fd, buf)
}

#[cfg(target_os = "linux")]
pub unsafe fn raw_close(fd: libc::c_int) -> libc::c_int {
    crate::syscalls::linux_raw::raw_close(fd)
}

#[cfg(target_os = "linux")]
pub unsafe fn raw_mmap(
    addr: *mut libc::c_void,
    len: libc::size_t,
    prot: libc::c_int,
    flags: libc::c_int,
    fd: libc::c_int,
    offset: libc::off_t,
) -> *mut libc::c_void {
    crate::syscalls::linux_raw::raw_mmap(addr, len, prot, flags, fd, offset)
}

#[cfg(target_os = "linux")]
pub unsafe fn raw_munmap(addr: *mut libc::c_void, len: libc::size_t) -> libc::c_int {
    crate::syscalls::linux_raw::raw_munmap(addr, len)
}

#[cfg(target_os = "linux")]
pub unsafe fn raw_access(path: *const libc::c_char, mode: libc::c_int) -> libc::c_int {
    crate::syscalls::linux_raw::raw_access(path, mode)
}

#[cfg(target_os = "linux")]
pub unsafe fn raw_stat(path: *const libc::c_char, buf: *mut libc::stat) -> libc::c_int {
    crate::syscalls::linux_raw::raw_stat(path, buf)
}

#[cfg(target_os = "linux")]
pub unsafe fn raw_lstat(path: *const libc::c_char, buf: *mut libc::stat) -> libc::c_int {
    crate::syscalls::linux_raw::raw_lstat(path, buf)
}
