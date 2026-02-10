//! Raw macOS ARM64 syscall wrappers for bootstrap-safe operations.
//!
//! # BUG-007: malloc/fstat Bootstrap Deadlock
//!
//! ## Problem Discovery
//!
//! When using `DYLD_INSERT_LIBRARIES` to inject vrift-inception layer into a process,
//! the process would hang during dyld bootstrap on macOS ARM64.
//!
//! Stack trace analysis using `sample` revealed:
//! ```text
//! dyld -> libSystem_initializer -> __malloc_init -> _os_feature_table_once -> fstat
//!                                                                               ↓
//!                                                                         fstat_inception (infinite recursion)
//! ```
//!
//! ## Root Cause Analysis
//!
//! 1. **Timing**: `fstat` is called inside `__malloc_init` BEFORE malloc is ready
//!
//! 2. **Interpose Redirection**: With `DYLD_INSERT_LIBRARIES` active, all calls to
//!    `fstat` get redirected to our `fstat_inception` via the `__DATA,__interpose` section
//!
//! 3. **dlsym Dependency**: `fstat_inception` was using `dlsym(RTLD_NEXT)` to get the
//!    real fstat pointer. But `dlsym` internally uses malloc (not yet initialized)!
//!
//! 4. **IT_FSTAT.old_func Trap**: We tried using the interpose table's `old_func`
//!    pointer, but with `DYLD_FORCE_FLAT_NAMESPACE=1`, even this points back to
//!    our inception layer, creating infinite recursion.
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
//! ## Affected Inception Layers
//!
//! The following inception layers use raw syscalls during early init (`INITIALIZING >= 2`)
//! or when in recursion (InceptionLayerGuard fails):
//!
//! - `fstat_inception` → [`raw_fstat64`] (SYS_fstat64 = 339)
//! - `close_inception` → [`raw_close`] (SYS_close = 6)
//! - `mmap_inception` → [`raw_mmap`] (SYS_mmap = 197)
//! - `munmap_inception` → [`raw_munmap`] (SYS_munmap = 73)
//! - `access_inception` → [`raw_access`] (SYS_access = 33)
//!
//! ## References
//!
//! - Syscall numbers: `/usr/include/sys/syscall.h`
//! - ARM64 ABI: Apple ARM64 Function Calling Conventions
//! - Pattern 2682: Raw Assembly Syscall Wrappers (linux_raw.rs)

use libc::{c_char, c_int};
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

/// SYS_openat = 463 on macOS
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
const SYS_OPENAT: i64 = 463;

/// SYS_fcntl = 92 on macOS
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
const SYS_FCNTL: i64 = 92;

/// SYS_flock = 131 on macOS
#[cfg(target_os = "macos")]
const SYS_FLOCK: i64 = 131;

/// SYS_chmod = 15 on macOS
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
const SYS_CHMOD: i64 = 15;

/// SYS_unlink = 10 on macOS
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
const SYS_UNLINK: i64 = 10;

/// SYS_rmdir = 137 on macOS
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
const SYS_RMDIR: i64 = 137;

/// SYS_mkdir = 136 on macOS
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
const SYS_MKDIR: i64 = 136;

/// SYS_truncate = 200 on macOS
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
const SYS_TRUNCATE: i64 = 200;

/// SYS_unlinkat = 472 on macOS
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
const SYS_UNLINKAT: i64 = 472;

/// SYS_mkdirat = 475 on macOS
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
const SYS_MKDIRAT: i64 = 475;
// NOTE: macOS has NO kernel syscall for utimensat/futimens.
// SYS 476 = getattrlistat, SYS 477 = proc_trace_log.
// These functions are libc wrappers over setattrlist.
// We use dlsym-based passthrough instead (see raw_utimensat / raw_futimens below).

/// SYS_symlinkat = 474 on macOS
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
const SYS_SYMLINKAT: i64 = 474;

/// SYS_fchmod = 124 on macOS
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
const SYS_FCHMOD: i64 = 124;

/// SYS_fchmodat = 467 on macOS
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
const SYS_FCHMODAT: i64 = 467;

/// SYS_fstatat64 = 470 on macOS
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
const SYS_FSTATAT64: i64 = 470;

/// SYS_rename = 128 on macOS
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
const SYS_RENAME: i64 = 128;

/// SYS_renameat = 465 on macOS
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
const SYS_RENAMEAT: i64 = 465;

/// SYS_readlink = 58 on macOS
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
const SYS_READLINK: i64 = 58;
// No SYS_REALPATH syscall on macOS

/// SYS_clonefileat = 462 on macOS (APFS CoW clone)
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
const SYS_CLONEFILEAT: i64 = 462;

/// Raw stat64 syscall for macOS ARM64.
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[inline(never)]
pub unsafe fn raw_stat(path: *const libc::c_char, buf: *mut libc::stat) -> libc::c_int {
    let ret: i64;
    let err: i64;
    asm!(
        "mov x16, {syscall}",
        "svc #0x80",
        "cset {err}, cs",
        syscall = in(reg) SYS_STAT64,
        in("x0") path as i64,
        in("x1") buf as i64,
        lateout("x0") ret,
        err = out(reg) err,
        options(nostack)
    );
    if err != 0 {
        crate::set_errno(ret as libc::c_int);
        return -1;
    }
    ret as libc::c_int
}

/// Raw lstat64 syscall for macOS ARM64.
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[inline(never)]
pub unsafe fn raw_lstat(path: *const libc::c_char, buf: *mut libc::stat) -> libc::c_int {
    let ret: i64;
    let err: i64;
    asm!(
        "mov x16, {syscall}",
        "svc #0x80",
        "cset {err}, cs",
        syscall = in(reg) SYS_LSTAT64,
        in("x0") path as i64,
        in("x1") buf as i64,
        lateout("x0") ret,
        err = out(reg) err,
        options(nostack)
    );
    if err != 0 {
        crate::set_errno(ret as libc::c_int);
        return -1;
    }
    ret as libc::c_int
}

/// Raw fstat64 syscall for macOS ARM64.
/// Returns 0 on success, -1 on error (with errno-style negative return).
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[inline(never)]
pub unsafe fn raw_fstat64(fd: libc::c_int, buf: *mut libc::stat) -> libc::c_int {
    let ret: i64;
    let err: i64;
    asm!(
        "mov x16, {syscall}",
        "svc #0x80",
        "cset {err}, cs",
        syscall = in(reg) SYS_FSTAT64,
        in("x0") fd as i64,
        in("x1") buf as i64,
        lateout("x0") ret,
        err = out(reg) err,
        options(nostack)
    );
    if err != 0 {
        crate::set_errno(ret as libc::c_int);
        return -1;
    }
    ret as libc::c_int
}

/// Raw close syscall for macOS ARM64.
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[inline(never)]
pub unsafe fn raw_close(fd: libc::c_int) -> libc::c_int {
    let ret: i64;
    let err: i64;
    asm!(
        "mov x16, {syscall}",
        "svc #0x80",
        "cset {err}, cs",
        syscall = in(reg) SYS_CLOSE,
        in("x0") fd as i64,
        lateout("x0") ret,
        err = out(reg) err,
        options(nostack)
    );
    if err != 0 {
        crate::set_errno(ret as libc::c_int);
        return -1;
    }
    ret as libc::c_int
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
    let err: i64;
    asm!(
        "mov x16, {syscall}",
        "svc #0x80",
        "cset {err}, cs",
        syscall = in(reg) SYS_MMAP,
        in("x0") addr as i64,
        in("x1") len as i64,
        in("x2") prot as i64,
        in("x3") flags as i64,
        in("x4") fd as i64,
        in("x5") offset,
        lateout("x0") ret,
        err = out(reg) err,
        options(nostack)
    );
    if err != 0 {
        crate::set_errno(ret as libc::c_int);
        return libc::MAP_FAILED;
    }
    ret as *mut libc::c_void
}

/// Raw munmap syscall for macOS ARM64.
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[inline(never)]
pub unsafe fn raw_munmap(addr: *mut libc::c_void, len: libc::size_t) -> libc::c_int {
    let ret: i64;
    let err: i64;
    asm!(
        "mov x16, {syscall}",
        "svc #0x80",
        "cset {err}, cs",
        syscall = in(reg) SYS_MUNMAP,
        in("x0") addr as i64,
        in("x1") len as i64,
        lateout("x0") ret,
        err = out(reg) err,
        options(nostack)
    );
    if err != 0 {
        crate::set_errno(ret as libc::c_int);
        return -1;
    }
    ret as libc::c_int
}

/// Raw access syscall for macOS ARM64.
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[inline(never)]
pub unsafe fn raw_access(path: *const libc::c_char, mode: libc::c_int) -> libc::c_int {
    let ret: i64;
    let err: i64;
    asm!(
        "mov x16, {syscall}",
        "svc #0x80",
        "cset {err}, cs",
        syscall = in(reg) SYS_ACCESS,
        in("x0") path as i64,
        in("x1") mode as i64,
        lateout("x0") ret,
        err = out(reg) err,
        options(nostack)
    );
    if err != 0 {
        crate::set_errno(ret as libc::c_int);
        return -1;
    }
    ret as libc::c_int
}

/// SYS_read = 3 on macOS
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
const SYS_READ: i64 = 3;

/// SYS_write = 4 on macOS
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
const SYS_WRITE: i64 = 4;

/// Raw read syscall for macOS ARM64.
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[inline(never)]
pub unsafe fn raw_read(
    fd: libc::c_int,
    buf: *mut libc::c_void,
    count: libc::size_t,
) -> libc::ssize_t {
    let ret: i64;
    let err: i64;
    asm!(
        "mov x16, {syscall}",
        "svc #0x80",
        "cset {err}, cs",
        syscall = in(reg) SYS_READ,
        in("x0") fd as i64,
        in("x1") buf as i64,
        in("x2") count as i64,
        lateout("x0") ret,
        err = out(reg) err,
        options(nostack)
    );
    if err != 0 {
        crate::set_errno(ret as libc::c_int);
        return -1;
    }
    ret as libc::ssize_t
}

/// Raw write syscall for macOS ARM64.
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[inline(never)]
pub unsafe fn raw_write(
    fd: libc::c_int,
    buf: *const libc::c_void,
    count: libc::size_t,
) -> libc::ssize_t {
    let ret: i64;
    let err: i64;
    asm!(
        "mov x16, {syscall}",
        "svc #0x80",
        "cset {err}, cs",
        syscall = in(reg) SYS_WRITE,
        in("x0") fd as i64,
        in("x1") buf as i64,
        in("x2") count as i64,
        lateout("x0") ret,
        err = out(reg) err,
        options(nostack)
    );
    if err != 0 {
        crate::set_errno(ret as libc::c_int);
        return -1;
    }
    ret as libc::ssize_t
}

/// SYS_dup = 41 on macOS
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
const SYS_DUP: i64 = 41;

/// SYS_dup2 = 90 on macOS
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
const SYS_DUP2: i64 = 90;

/// SYS_lseek = 199 on macOS
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
const SYS_LSEEK: i64 = 199;

/// SYS_ftruncate = 201 on macOS
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
const SYS_FTRUNCATE: i64 = 201;

/// Raw dup syscall for macOS ARM64.
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[inline(never)]
pub unsafe fn raw_dup(oldfd: libc::c_int) -> libc::c_int {
    let ret: i64;
    let err: i64;
    asm!(
        "mov x16, {syscall}",
        "svc #0x80",
        "cset {err}, cs",
        syscall = in(reg) SYS_DUP,
        in("x0") oldfd as i64,
        lateout("x0") ret,
        err = out(reg) err,
        options(nostack)
    );
    if err != 0 {
        crate::set_errno(ret as libc::c_int);
        return -1;
    }
    ret as libc::c_int
}

/// Raw dup2 syscall for macOS ARM64.
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[inline(never)]
pub unsafe fn raw_dup2(oldfd: libc::c_int, newfd: libc::c_int) -> libc::c_int {
    let ret: i64;
    let err: i64;
    asm!(
        "mov x16, {syscall}",
        "svc #0x80",
        "cset {err}, cs",
        syscall = in(reg) SYS_DUP2,
        in("x0") oldfd as i64,
        in("x1") newfd as i64,
        lateout("x0") ret,
        err = out(reg) err,
        options(nostack)
    );
    if err != 0 {
        crate::set_errno(ret as libc::c_int);
        return -1;
    }
    ret as libc::c_int
}

/// Raw lseek syscall for macOS ARM64.
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[inline(never)]
pub unsafe fn raw_lseek(fd: libc::c_int, offset: libc::off_t, whence: libc::c_int) -> libc::off_t {
    let ret: i64;
    let err: i64;
    asm!(
        "mov x16, {syscall}",
        "svc #0x80",
        "cset {err}, cs",
        syscall = in(reg) SYS_LSEEK,
        in("x0") fd as i64,
        in("x1") offset,
        in("x2") whence as i64,
        lateout("x0") ret,
        err = out(reg) err,
        options(nostack)
    );
    if err != 0 {
        crate::set_errno(ret as libc::c_int);
        return -1;
    }
    ret as libc::off_t
}

/// Raw ftruncate syscall for macOS ARM64.
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[inline(never)]
pub unsafe fn raw_ftruncate(fd: libc::c_int, length: libc::off_t) -> libc::c_int {
    let ret: i64;
    let err: i64;
    asm!(
        "mov x16, {syscall}",
        "svc #0x80",
        "cset {err}, cs",
        syscall = in(reg) SYS_FTRUNCATE,
        in("x0") fd as i64,
        in("x1") length,
        lateout("x0") ret,
        err = out(reg) err,
        options(nostack)
    );
    if err != 0 {
        crate::set_errno(ret as libc::c_int);
        return -1;
    }
    ret as libc::c_int
}

/// Raw openat syscall for macOS ARM64.
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[inline(never)]
pub unsafe fn raw_openat(
    dirfd: libc::c_int,
    path: *const libc::c_char,
    flags: libc::c_int,
    mode: libc::mode_t,
) -> libc::c_int {
    let ret: i64;
    let err: i64;
    asm!(
        "mov x16, #463",
        "svc #0x80",
        "cset {err}, cs",
        in("x0") dirfd as i64,
        in("x1") path,
        in("x2") flags as i64,
        in("x3") mode as i64,
        lateout("x0") ret,
        err = out(reg) err,
        options(nostack)
    );
    if err != 0 {
        crate::set_errno(ret as libc::c_int);
        return -1;
    }
    ret as libc::c_int
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[inline(always)]
pub unsafe fn raw_open(
    path: *const libc::c_char,
    flags: libc::c_int,
    mode: libc::mode_t,
) -> libc::c_int {
    raw_openat(libc::AT_FDCWD, path, flags, mode)
}

/// Raw fcntl syscall for macOS ARM64.
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[inline(never)]
pub unsafe fn raw_fcntl(fd: libc::c_int, cmd: libc::c_int, arg: i64) -> libc::c_int {
    let ret: i64;
    let err: i64;
    asm!(
        "mov x16, {syscall}",
        "svc #0x80",
        "cset {err}, cs",
        syscall = in(reg) SYS_FCNTL,
        in("x0") fd as i64,
        in("x1") cmd as i64,
        in("x2") arg,
        lateout("x0") ret,
        err = out(reg) err,
        options(nostack)
    );
    if err != 0 {
        crate::set_errno(ret as libc::c_int);
        return -1;
    }
    ret as libc::c_int
}

/// Raw chmod syscall for macOS ARM64.
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[inline(never)]
pub unsafe fn raw_chmod(path: *const libc::c_char, mode: libc::mode_t) -> libc::c_int {
    let ret: i64;
    asm!(
        "mov x16, {syscall}",
        "svc #0x80",
        syscall = in(reg) SYS_CHMOD,
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

/// Raw unlink syscall for macOS ARM64.
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[inline(never)]
pub unsafe fn raw_unlink(path: *const libc::c_char) -> libc::c_int {
    let ret: i64;
    asm!(
        "mov x16, {syscall}",
        "svc #0x80",
        syscall = in(reg) SYS_UNLINK,
        in("x0") path as i64,
        lateout("x0") ret,
        options(nostack)
    );
    if ret < 0 {
        -1
    } else {
        ret as libc::c_int
    }
}

/// Raw rmdir syscall for macOS ARM64.
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[inline(never)]
pub unsafe fn raw_rmdir(path: *const libc::c_char) -> libc::c_int {
    let ret: i64;
    let err: i64;
    asm!(
        "mov x16, {syscall}",
        "svc #0x80",
        "cset {err}, cs",
        syscall = in(reg) SYS_RMDIR,
        in("x0") path as i64,
        lateout("x0") ret,
        err = out(reg) err,
        options(nostack)
    );
    if err != 0 {
        crate::set_errno(ret as libc::c_int);
        return -1;
    }
    ret as libc::c_int
}

/// Raw rename syscall for macOS ARM64.
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[inline(never)]
pub unsafe fn raw_rename(old: *const libc::c_char, new: *const libc::c_char) -> libc::c_int {
    let ret: i64;
    let err: i64;
    asm!(
        "mov x16, {syscall}",
        "svc #0x80",
        "cset {err}, cs",
        syscall = in(reg) SYS_RENAME,
        in("x0") old as i64,
        in("x1") new as i64,
        lateout("x0") ret,
        err = out(reg) err,
        options(nostack)
    );
    if err != 0 {
        crate::set_errno(ret as libc::c_int);
        return -1;
    }
    ret as libc::c_int
}

/// Raw renameat syscall for macOS ARM64.
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[inline(never)]
pub unsafe fn raw_renameat(
    olddirfd: libc::c_int,
    old: *const libc::c_char,
    newdirfd: libc::c_int,
    new: *const libc::c_char,
) -> libc::c_int {
    let ret: i64;
    let err: i64;
    asm!(
        "mov x16, {syscall}",
        "svc #0x80",
        "cset {err}, cs",
        syscall = in(reg) SYS_RENAMEAT,
        in("x0") olddirfd as i64,
        in("x1") old as i64,
        in("x2") newdirfd as i64,
        in("x3") new as i64,
        lateout("x0") ret,
        err = out(reg) err,
        options(nostack)
    );
    if err != 0 {
        crate::set_errno(ret as libc::c_int);
        return -1;
    }
    ret as libc::c_int
}

/// Raw readlink syscall for macOS ARM64.
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[inline(never)]
pub unsafe fn raw_readlink(
    path: *const libc::c_char,
    buf: *mut libc::c_char,
    bufsiz: libc::size_t,
) -> libc::ssize_t {
    let ret: i64;
    let err: i64;
    asm!(
        "mov x16, {syscall}",
        "svc #0x80",
        "cset {err}, cs",
        syscall = in(reg) SYS_READLINK,
        in("x0") path as i64,
        in("x1") buf as i64,
        in("x2") bufsiz as i64,
        lateout("x0") ret,
        err = out(reg) err,
        options(nostack)
    );
    if err != 0 {
        crate::set_errno(ret as libc::c_int);
        return -1;
    }
    ret as libc::ssize_t
}

/// Raw realpath proxy for macOS ARM64.
/// NOTE: There is no direct realpath syscall on macOS (SYS 462 = clonefileat).
///
/// Under DYLD_FORCE_FLAT_NAMESPACE, ANY dlsym-based resolution of "realpath"
/// (including RTLD_NEXT, RTLD_DEFAULT, or even specific library handles)
/// resolves to our own `realpath_inception`, causing infinite recursion.
///
/// Solution: Use raw kernel syscalls only — open(path) + fcntl(F_GETPATH) + close.
/// fcntl(F_GETPATH) is a macOS kernel operation that returns the canonical
/// filesystem path for an open file descriptor, equivalent to realpath.
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
pub unsafe fn raw_realpath(
    path: *const libc::c_char,
    resolved: *mut libc::c_char,
) -> *mut libc::c_char {
    use std::sync::atomic::Ordering;

    // Bootstrap safety: during early init, just copy the path.
    let init_state = crate::state::INITIALIZING.load(Ordering::Relaxed);
    if init_state >= 2 {
        if path.is_null() {
            return std::ptr::null_mut();
        }
        let len = libc::strlen(path);
        if !resolved.is_null() {
            libc::memcpy(
                resolved as *mut libc::c_void,
                path as *const libc::c_void,
                len + 1,
            );
            return resolved;
        } else {
            return std::ptr::null_mut();
        }
    }

    if path.is_null() {
        crate::set_errno(libc::EINVAL);
        return std::ptr::null_mut();
    }

    // Open the path read-only (O_RDONLY). For directories, this works.
    // For files, we only need the fd to get the canonical path.
    let fd = raw_open(path, libc::O_RDONLY | libc::O_CLOEXEC, 0);
    if fd < 0 {
        // open failed — errno is already set by raw_open
        return std::ptr::null_mut();
    }

    // Use a stack buffer if caller didn't provide one, or use caller's buffer
    let mut stack_buf = [0u8; libc::PATH_MAX as usize];
    let buf_ptr = if !resolved.is_null() {
        resolved
    } else {
        stack_buf.as_mut_ptr() as *mut libc::c_char
    };

    // fcntl(fd, F_GETPATH, buf) — kernel resolves the canonical path
    let ret = raw_fcntl(fd, libc::F_GETPATH, buf_ptr as i64);
    raw_close(fd);

    if ret < 0 {
        crate::set_errno(libc::ENOENT);
        return std::ptr::null_mut();
    }

    if !resolved.is_null() {
        // Caller provided buffer — just return it
        resolved
    } else {
        // Need to allocate: use libc::malloc + copy from stack buffer
        let len = libc::strlen(buf_ptr);
        let alloc = libc::malloc(len + 1) as *mut libc::c_char;
        if alloc.is_null() {
            crate::set_errno(libc::ENOMEM);
            return std::ptr::null_mut();
        }
        libc::memcpy(
            alloc as *mut libc::c_void,
            buf_ptr as *const libc::c_void,
            len + 1,
        );
        alloc
    }
}

/// Raw mkdir syscall for macOS ARM64.
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[inline(never)]
pub unsafe fn raw_mkdir(path: *const libc::c_char, mode: libc::mode_t) -> libc::c_int {
    let ret: i64;
    let err: i64;
    asm!(
        "mov x16, {syscall}",
        "svc #0x80",
        "cset {err}, cs",
        syscall = in(reg) SYS_MKDIR,
        in("x0") path as i64,
        in("x1") mode as i64,
        lateout("x0") ret,
        err = out(reg) err,
        options(nostack)
    );
    if err != 0 {
        crate::set_errno(ret as libc::c_int);
        return -1;
    }
    ret as libc::c_int
}

/// Raw truncate syscall for macOS ARM64.
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[inline(never)]
pub unsafe fn raw_truncate(path: *const libc::c_char, length: libc::off_t) -> libc::c_int {
    let ret: i64;
    let err: i64;
    asm!(
        "mov x16, {syscall}",
        "svc #0x80",
        "cset {err}, cs",
        syscall = in(reg) SYS_TRUNCATE,
        in("x0") path as i64,
        in("x1") length,
        lateout("x0") ret,
        err = out(reg) err,
        options(nostack)
    );
    if err != 0 {
        crate::set_errno(ret as libc::c_int);
        return -1;
    }
    ret as libc::c_int
}

/// Raw unlinkat syscall for macOS ARM64.
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[inline(never)]
pub unsafe fn raw_unlinkat(
    dirfd: libc::c_int,
    path: *const libc::c_char,
    flags: libc::c_int,
) -> libc::c_int {
    let ret: i64;
    let err: i64;
    asm!(
        "mov x16, {syscall}",
        "svc #0x80",
        "cset {err}, cs",
        syscall = in(reg) SYS_UNLINKAT,
        in("x0") dirfd as i64,
        in("x1") path as i64,
        in("x2") flags as i64,
        lateout("x0") ret,
        err = out(reg) err,
        options(nostack)
    );
    if err != 0 {
        crate::set_errno(ret as libc::c_int);
        return -1;
    }
    ret as libc::c_int
}

/// Raw mkdirat syscall for macOS ARM64.
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[inline(never)]
pub unsafe fn raw_mkdirat(
    dirfd: libc::c_int,
    path: *const libc::c_char,
    mode: libc::mode_t,
) -> libc::c_int {
    let ret: i64;
    let err: i64;
    asm!(
        "mov x16, {syscall}",
        "svc #0x80",
        "cset {err}, cs",
        syscall = in(reg) SYS_MKDIRAT,
        in("x0") dirfd as i64,
        in("x1") path as i64,
        in("x2") mode as i64,
        lateout("x0") ret,
        err = out(reg) err,
        options(nostack)
    );
    if err != 0 {
        crate::set_errno(ret as libc::c_int);
        return -1;
    }
    ret as libc::c_int
}

/// Raw clonefileat syscall for macOS ARM64.
/// Creates an APFS Copy-on-Write clone of a file — zero data copy, instant.
/// `src_dirfd` and `dst_dirfd` can be AT_FDCWD (-2) for cwd-relative paths.
/// `flags`: 0 for default; CLONE_NOFOLLOW (1), CLONE_NOOWNERCOPY (2).
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[inline(never)]
pub unsafe fn raw_clonefileat(
    src_dirfd: libc::c_int,
    src: *const libc::c_char,
    dst_dirfd: libc::c_int,
    dst: *const libc::c_char,
    flags: u32,
) -> libc::c_int {
    let ret: i64;
    let err: i64;
    asm!(
        "mov x16, {syscall}",
        "svc #0x80",
        "cset {err}, cs",
        syscall = in(reg) SYS_CLONEFILEAT,
        in("x0") src_dirfd as i64,
        in("x1") src as i64,
        in("x2") dst_dirfd as i64,
        in("x3") dst as i64,
        in("x4") flags as i64,
        lateout("x0") ret,
        err = out(reg) err,
        options(nostack)
    );
    if err != 0 {
        crate::set_errno(ret as libc::c_int);
        return -1;
    }
    ret as libc::c_int
}

/// Raw symlinkat syscall for macOS ARM64.
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[inline(never)]
pub unsafe fn raw_symlinkat(
    p1: *const libc::c_char,
    dirfd: libc::c_int,
    p2: *const libc::c_char,
) -> libc::c_int {
    let ret: i64;
    let err: i64;
    asm!(
        "mov x16, {syscall}",
        "svc #0x80",
        "cset {err}, cs",
        syscall = in(reg) SYS_SYMLINKAT,
        in("x0") p1 as i64,
        in("x1") dirfd as i64,
        in("x2") p2 as i64,
        lateout("x0") ret,
        err = out(reg) err,
        options(nostack)
    );
    if err != 0 {
        crate::set_errno(ret as libc::c_int);
        return -1;
    }
    ret as libc::c_int
}

/// Raw fchmod syscall for macOS ARM64.
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[inline(never)]
pub unsafe fn raw_fchmod(fd: libc::c_int, mode: libc::mode_t) -> libc::c_int {
    let ret: i64;
    let err: i64;
    asm!(
        "mov x16, {syscall}",
        "svc #0x80",
        "cset {err}, cs",
        syscall = in(reg) SYS_FCHMOD,
        in("x0") fd as i64,
        in("x1") mode as i64,
        lateout("x0") ret,
        err = out(reg) err,
        options(nostack)
    );
    if err != 0 {
        crate::set_errno(ret as libc::c_int);
        return -1;
    }
    ret as libc::c_int
}

/// Raw fchmodat syscall for macOS ARM64.
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[inline(never)]
pub unsafe fn raw_fchmodat(
    dirfd: libc::c_int,
    path: *const libc::c_char,
    mode: libc::mode_t,
    flags: libc::c_int,
) -> libc::c_int {
    let ret: i64;
    let err: i64;
    asm!(
        "mov x16, {syscall}",
        "svc #0x80",
        "cset {err}, cs",
        syscall = in(reg) SYS_FCHMODAT,
        in("x0") dirfd as i64,
        in("x1") path as i64,
        in("x2") mode as i64,
        in("x3") flags as i64,
        lateout("x0") ret,
        err = out(reg) err,
        options(nostack)
    );
    if err != 0 {
        crate::set_errno(ret as libc::c_int);
        return -1;
    }
    ret as libc::c_int
}

/// Raw fstatat64 syscall for macOS ARM64.
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[inline(never)]
pub unsafe fn raw_fstatat64(
    dirfd: libc::c_int,
    path: *const libc::c_char,
    buf: *mut libc::stat,
    flags: libc::c_int,
) -> libc::c_int {
    let ret: i64;
    let err: i64;
    asm!(
        "mov x16, {syscall}",
        "svc #0x80",
        "cset {err}, cs",
        syscall = in(reg) SYS_FSTATAT64,
        in("x0") dirfd as i64,
        in("x1") path as i64,
        in("x2") buf as i64,
        in("x3") flags as i64,
        lateout("x0") ret,
        err = out(reg) err,
        options(nostack)
    );
    if err != 0 {
        crate::set_errno(ret as libc::c_int);
        return -1;
    }
    ret as libc::c_int
}

/// SYS_linkat = 471 on macOS
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
const SYS_LINKAT: i64 = 471;

/// SYS_chflags = 34 on macOS
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
const SYS_CHFLAGS: i64 = 34;

/// SYS_setxattr = 236 on macOS
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
const SYS_SETXATTR: i64 = 236;

/// SYS_removexattr = 238 on macOS
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
const SYS_REMOVEXATTR: i64 = 238;

const SYS_UTIMES: i64 = 138;
const SYS_GETATTRLIST: i64 = 220;
const SYS_SETATTRLIST: i64 = 221;

/// Raw linkat syscall for macOS ARM64.
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[inline(never)]
pub unsafe fn raw_linkat(
    olddirfd: libc::c_int,
    oldpath: *const libc::c_char,
    newdirfd: libc::c_int,
    newpath: *const libc::c_char,
    flags: libc::c_int,
) -> libc::c_int {
    let ret: i64;
    let err: i64;
    asm!(
        "mov x16, {syscall}",
        "svc #0x80",
        "cset {err}, cs",
        syscall = in(reg) SYS_LINKAT,
        in("x0") olddirfd as i64,
        in("x1") oldpath as i64,
        in("x2") newdirfd as i64,
        in("x3") newpath as i64,
        in("x4") flags as i64,
        lateout("x0") ret,
        err = out(reg) err,
        options(nostack)
    );
    if err != 0 {
        crate::set_errno(ret as libc::c_int);
        return -1;
    }
    ret as libc::c_int
}

/// Raw link syscall for macOS ARM64 (uses linkat with AT_FDCWD).
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[inline(always)]
pub unsafe fn raw_link(oldpath: *const libc::c_char, newpath: *const libc::c_char) -> libc::c_int {
    raw_linkat(libc::AT_FDCWD, oldpath, libc::AT_FDCWD, newpath, 0)
}

/// Raw chflags syscall for macOS ARM64.
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[inline(never)]
pub unsafe fn raw_chflags(path: *const libc::c_char, flags: libc::c_uint) -> libc::c_int {
    let ret: i64;
    let err: i64;
    asm!(
        "mov x16, {syscall}",
        "svc #0x80",
        "cset {err}, cs",
        syscall = in(reg) SYS_CHFLAGS,
        in("x0") path as i64,
        in("x1") flags as i64,
        lateout("x0") ret,
        err = out(reg) err,
        options(nostack)
    );
    if err != 0 {
        crate::set_errno(ret as libc::c_int);
        return -1;
    }
    ret as libc::c_int
}

/// Raw setxattr syscall for macOS ARM64.
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[inline(never)]
pub unsafe fn raw_setxattr(
    path: *const libc::c_char,
    name: *const libc::c_char,
    value: *const libc::c_void,
    size: libc::size_t,
    position: u32,
    options: libc::c_int,
) -> libc::c_int {
    let ret: i64;
    let err: i64;
    asm!(
        "mov x16, {syscall}",
        "svc #0x80",
        "cset {err}, cs",
        syscall = in(reg) SYS_SETXATTR,
        in("x0") path as i64,
        in("x1") name as i64,
        in("x2") value as i64,
        in("x3") size as i64,
        in("x4") position as i64,
        in("x5") options as i64,
        lateout("x0") ret,
        err = out(reg) err,
        options(nostack)
    );
    if err != 0 {
        crate::set_errno(ret as libc::c_int);
        return -1;
    }
    ret as libc::c_int
}

/// Raw removexattr syscall for macOS ARM64.
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[inline(never)]
pub unsafe fn raw_removexattr(
    path: *const libc::c_char,
    name: *const libc::c_char,
    options: libc::c_int,
) -> libc::c_int {
    let ret: i64;
    let err: i64;
    asm!(
        "mov x16, {syscall}",
        "svc #0x80",
        "cset {err}, cs",
        syscall = in(reg) SYS_REMOVEXATTR,
        in("x0") path as i64,
        in("x1") name as i64,
        in("x2") options as i64,
        lateout("x0") ret,
        err = out(reg) err,
        options(nostack)
    );
    if err != 0 {
        crate::set_errno(ret as libc::c_int);
        return -1;
    }
    ret as libc::c_int
}

/// Raw utimes syscall for macOS ARM64.
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[inline(never)]
pub unsafe fn raw_utimes(path: *const libc::c_char, times: *const libc::timeval) -> libc::c_int {
    let ret: i64;
    let err: i64;
    asm!(
        "mov x16, {syscall}",
        "svc #0x80",
        "cset {err}, cs",
        syscall = in(reg) SYS_UTIMES,
        in("x0") path as i64,
        in("x1") times as i64,
        lateout("x0") ret,
        err = out(reg) err,
        options(nostack)
    );
    if err != 0 {
        crate::set_errno(ret as libc::c_int);
        return -1;
    }
    ret as libc::c_int
}

/// Raw getattrlist syscall for macOS ARM64.
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[inline(always)]
pub unsafe fn raw_getattrlist(
    path: *const libc::c_char,
    attrlist: *mut libc::c_void,
    attrbuf: *mut libc::c_void,
    attrbufsize: libc::size_t,
    options: libc::c_ulong,
) -> libc::c_int {
    let ret: i64;
    let err: i64;
    asm!(
        "mov x16, {syscall}",
        "svc #0x80",
        "cset {err}, cs",
        syscall = in(reg) SYS_GETATTRLIST,
        in("x0") path as i64,
        in("x1") attrlist as i64,
        in("x2") attrbuf as i64,
        in("x3") attrbufsize as i64,
        in("x4") options as i64,
        lateout("x0") ret,
        err = out(reg) err,
        options(nostack)
    );
    if err != 0 {
        -1
    } else {
        ret as libc::c_int
    }
}

/// Raw setattrlist syscall for macOS ARM64.
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[inline(always)]
pub unsafe fn raw_setattrlist(
    path: *const libc::c_char,
    attrlist: *mut libc::c_void,
    attrbuf: *mut libc::c_void,
    attrbufsize: libc::size_t,
    options: libc::c_ulong,
) -> libc::c_int {
    let ret: i64;
    let err: i64;
    asm!(
        "mov x16, {syscall}",
        "svc #0x80",
        "cset {err}, cs",
        syscall = in(reg) SYS_SETATTRLIST,
        in("x0") path as i64,
        in("x1") attrlist as i64,
        in("x2") attrbuf as i64,
        in("x3") attrbufsize as i64,
        in("x4") options as i64,
        lateout("x0") ret,
        err = out(reg) err,
        options(nostack)
    );
    if err != 0 {
        -1
    } else {
        ret as libc::c_int
    }
}

/// SYS_fchdir = 13 on macOS
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
const SYS_FCHDIR: i64 = 13;

/// NOTE: There is NO direct __getcwd kernel syscall on macOS!
/// SYS 304 is psynch_cvsignal. raw_getcwd uses open(".") + fcntl(F_GETPATH) instead.
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[allow(dead_code)]
const SYS_GETCWD: i64 = 304; // WRONG — keep for reference only

/// SYS_chdir = 12 on macOS
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
const SYS_CHDIR: i64 = 12;

/// SYS_setrlimit = 195 on macOS
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
const SYS_SETRLIMIT: i64 = 195;

/// Raw fchdir syscall for macOS ARM64.
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[inline(never)]
pub unsafe fn raw_fchdir(fd: libc::c_int) -> libc::c_int {
    let ret: i64;
    let err: i64;
    asm!(
        "mov x16, {syscall}",
        "svc #0x80",
        "cset {err}, cs",
        syscall = in(reg) SYS_FCHDIR,
        in("x0") fd as i64,
        lateout("x0") ret,
        err = out(reg) err,
        options(nostack)
    );
    if err != 0 {
        crate::set_errno(ret as libc::c_int);
        return -1;
    }
    ret as libc::c_int
}

/// Raw getcwd for macOS ARM64.
///
/// NOTE: There is no direct __getcwd kernel syscall on macOS
/// (SYS 304 is psynch_cvsignal, NOT __getcwd).
///
/// Under DYLD_FORCE_FLAT_NAMESPACE, ANY dlsym-based resolution of "getcwd"
/// (including RTLD_NEXT, RTLD_DEFAULT) resolves to our own `getcwd_inception`,
/// causing infinite recursion (same pattern as realpath — see 60ac738).
///
/// Solution: Use raw_open(".") + raw_fcntl(F_GETPATH) + raw_close() — the same
/// proven approach used by raw_realpath.
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[inline(never)]
pub unsafe fn raw_getcwd(buf: *mut libc::c_char, size: libc::size_t) -> *mut libc::c_char {
    use std::sync::atomic::Ordering;

    // BUG-007 check: Bootstrap safety
    let init_state = crate::state::INITIALIZING.load(Ordering::Relaxed);
    if init_state >= 2 {
        return std::ptr::null_mut();
    }

    if buf.is_null() || size == 0 {
        crate::set_errno(libc::EINVAL);
        return std::ptr::null_mut();
    }

    // Open current directory "." using raw syscall
    let dot = b".\0";
    let fd = raw_open(
        dot.as_ptr() as *const libc::c_char,
        libc::O_RDONLY | libc::O_CLOEXEC,
        0,
    );
    if fd < 0 {
        return std::ptr::null_mut();
    }

    // fcntl(fd, F_GETPATH, buf) — kernel resolves the canonical path
    let ret = raw_fcntl(fd, libc::F_GETPATH, buf as i64);
    raw_close(fd);

    if ret < 0 {
        crate::set_errno(libc::ENOENT);
        return std::ptr::null_mut();
    }

    buf
}

/// Raw chdir syscall for macOS ARM64.
///
/// NOTE: Under DYLD_FORCE_FLAT_NAMESPACE, dlsym-based resolution of "chdir"
/// may resolve to our own `chdir_inception`, causing infinite recursion.
/// Solution: Use raw kernel syscall SYS_chdir (12) directly.
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
pub unsafe fn raw_chdir(path: *const libc::c_char) -> libc::c_int {
    use std::arch::asm;
    use std::sync::atomic::Ordering;

    // BUG-007 check: Bootstrap safety
    let init_state = crate::state::INITIALIZING.load(Ordering::Relaxed);
    if init_state >= 2 {
        return -1;
    }

    if path.is_null() {
        crate::set_errno(libc::EINVAL);
        return -1;
    }

    let ret: i64;
    let err: i64;
    asm!(
        "mov x16, {syscall}",
        "svc #0x80",
        "cset {err}, cs",
        syscall = in(reg) SYS_CHDIR,
        in("x0") path as i64,
        lateout("x0") ret,
        err = out(reg) err,
        options(nostack)
    );
    if err != 0 {
        crate::set_errno(ret as libc::c_int);
        return -1;
    }
    ret as libc::c_int
}

/// Raw setrlimit syscall for macOS ARM64.
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[inline(never)]
pub unsafe fn raw_setrlimit(resource: libc::c_int, rlp: *const libc::rlimit) -> libc::c_int {
    let ret: i64;
    let err: i64;
    asm!(
        "mov x16, {syscall}",
        "svc #0x80",
        "cset {err}, cs",
        syscall = in(reg) SYS_SETRLIMIT,
        in("x0") resource as i64,
        in("x1") rlp as i64,
        lateout("x0") ret,
        err = out(reg) err,
        options(nostack)
    );
    if err != 0 {
        crate::set_errno(ret as libc::c_int);
        return -1;
    }
    ret as libc::c_int
}

// =============================================================================
// P0-P1 Gap Fix: Ownership and Atomic Swap Operations
// =============================================================================

/// SYS_fchown = 123 on macOS
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
const SYS_FCHOWN: i64 = 123;

/// SYS_fchownat = 468 on macOS (not available on all versions, use carefully)
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
const SYS_FCHOWNAT: i64 = 468;

/// SYS_exchangedata = 223 on macOS
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
const SYS_EXCHANGEDATA: i64 = 223;

/// SYS_futimes = 206 on macOS
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
const SYS_FUTIMES: i64 = 206;

/// SYS_fchflags = 124 on macOS
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
const SYS_FCHFLAGS: i64 = 124;

/// SYS_sendfile = 337 on macOS
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
const SYS_SENDFILE: i64 = 337;

/// Raw fchown syscall for macOS ARM64.
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[inline(never)]
pub unsafe fn raw_fchown(fd: libc::c_int, owner: libc::uid_t, group: libc::gid_t) -> libc::c_int {
    let ret: i64;
    let err: i64;
    asm!(
        "mov x16, {syscall}",
        "svc #0x80",
        "cset {err}, cs",
        syscall = in(reg) SYS_FCHOWN,
        in("x0") fd as i64,
        in("x1") owner as i64,
        in("x2") group as i64,
        lateout("x0") ret,
        err = out(reg) err,
        options(nostack)
    );
    if err != 0 {
        crate::set_errno(ret as libc::c_int);
        return -1;
    }
    ret as libc::c_int
}

/// Raw fchownat syscall for macOS ARM64.
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[inline(never)]
pub unsafe fn raw_fchownat(
    dirfd: libc::c_int,
    path: *const libc::c_char,
    owner: libc::uid_t,
    group: libc::gid_t,
    flags: libc::c_int,
) -> libc::c_int {
    let ret: i64;
    let err: i64;
    asm!(
        "mov x16, {syscall}",
        "svc #0x80",
        "cset {err}, cs",
        syscall = in(reg) SYS_FCHOWNAT,
        in("x0") dirfd as i64,
        in("x1") path as i64,
        in("x2") owner as i64,
        in("x3") group as i64,
        in("x4") flags as i64,
        lateout("x0") ret,
        err = out(reg) err,
        options(nostack)
    );
    if err != 0 {
        crate::set_errno(ret as libc::c_int);
        return -1;
    }
    ret as libc::c_int
}

/// Raw exchangedata syscall for macOS ARM64.
/// Atomically swaps the contents of two files.
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[inline(never)]
pub unsafe fn raw_exchangedata(
    path1: *const libc::c_char,
    path2: *const libc::c_char,
    options: libc::c_uint,
) -> libc::c_int {
    let ret: i64;
    let err: i64;
    asm!(
        "mov x16, {syscall}",
        "svc #0x80",
        "cset {err}, cs",
        syscall = in(reg) SYS_EXCHANGEDATA,
        in("x0") path1 as i64,
        in("x1") path2 as i64,
        in("x2") options as i64,
        lateout("x0") ret,
        err = out(reg) err,
        options(nostack)
    );
    if err != 0 {
        crate::set_errno(ret as libc::c_int);
        return -1;
    }
    ret as libc::c_int
}

/// Raw futimes syscall for macOS ARM64.
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[inline(never)]
pub unsafe fn raw_futimes(fd: libc::c_int, times: *const libc::timeval) -> libc::c_int {
    let ret: i64;
    let err: i64;
    asm!(
        "mov x16, {syscall}",
        "svc #0x80",
        "cset {err}, cs",
        syscall = in(reg) SYS_FUTIMES,
        in("x0") fd as i64,
        in("x1") times as i64,
        lateout("x0") ret,
        err = out(reg) err,
        options(nostack)
    );
    if err != 0 {
        crate::set_errno(ret as libc::c_int);
        return -1;
    }
    ret as libc::c_int
}

/// Raw fchflags syscall for macOS ARM64.
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[inline(never)]
pub unsafe fn raw_fchflags(fd: libc::c_int, flags: libc::c_uint) -> libc::c_int {
    let ret: i64;
    let err: i64;
    asm!(
        "mov x16, {syscall}",
        "svc #0x80",
        "cset {err}, cs",
        syscall = in(reg) SYS_FCHFLAGS,
        in("x0") fd as i64,
        in("x1") flags as i64,
        lateout("x0") ret,
        err = out(reg) err,
        options(nostack)
    );
    if err != 0 {
        crate::set_errno(ret as libc::c_int);
        return -1;
    }
    ret as libc::c_int
}

/// Raw sendfile syscall for macOS ARM64.
/// Note: sendfile on macOS has a complex signature:
/// int sendfile(int fd, int s, off_t offset, off_t *len, struct sf_hdtr *hdtr, int flags);
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[inline(never)]
pub unsafe fn raw_sendfile(
    fd: libc::c_int,
    s: libc::c_int,
    offset: libc::off_t,
    len: *mut libc::off_t,
    hdtr: *mut libc::c_void,
    flags: libc::c_int,
) -> libc::c_int {
    let ret: i64;
    let err: i64;
    asm!(
        "mov x16, {syscall}",
        "svc #0x80",
        "cset {err}, cs",
        syscall = in(reg) SYS_SENDFILE,
        in("x0") fd as i64,
        in("x1") s as i64,
        in("x2") offset,
        in("x3") len as i64,
        in("x4") hdtr as i64,
        in("x5") flags as i64,
        lateout("x0") ret,
        err = out(reg) err,
        options(nostack)
    );
    if err != 0 {
        crate::set_errno(ret as libc::c_int);
        return -1;
    }
    ret as libc::c_int
}

// Gap Fix: chown/lchown/readlinkat syscall numbers
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
const SYS_CHOWN: i64 = 16;
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
const SYS_LCHOWN: i64 = 364;
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
const SYS_READLINKAT: i64 = 473;

/// Raw chown syscall for macOS ARM64.
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[inline(never)]
pub unsafe fn raw_chown(
    path: *const libc::c_char,
    owner: libc::uid_t,
    group: libc::gid_t,
) -> libc::c_int {
    let ret: i64;
    let err: i64;
    asm!(
        "mov x16, {syscall}",
        "svc #0x80",
        "cset {err}, cs",
        syscall = in(reg) SYS_CHOWN,
        in("x0") path as i64,
        in("x1") owner as i64,
        in("x2") group as i64,
        lateout("x0") ret,
        err = out(reg) err,
        options(nostack)
    );
    if err != 0 {
        crate::set_errno(ret as libc::c_int);
        return -1;
    }
    ret as libc::c_int
}

/// Raw lchown syscall for macOS ARM64.
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[inline(never)]
pub unsafe fn raw_lchown(
    path: *const libc::c_char,
    owner: libc::uid_t,
    group: libc::gid_t,
) -> libc::c_int {
    let ret: i64;
    let err: i64;
    asm!(
        "mov x16, {syscall}",
        "svc #0x80",
        "cset {err}, cs",
        syscall = in(reg) SYS_LCHOWN,
        in("x0") path as i64,
        in("x1") owner as i64,
        in("x2") group as i64,
        lateout("x0") ret,
        err = out(reg) err,
        options(nostack)
    );
    if err != 0 {
        crate::set_errno(ret as libc::c_int);
        return -1;
    }
    ret as libc::c_int
}

/// Raw readlinkat syscall for macOS ARM64.
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[inline(never)]
pub unsafe fn raw_readlinkat(
    dirfd: libc::c_int,
    path: *const libc::c_char,
    buf: *mut libc::c_char,
    bufsiz: libc::size_t,
) -> libc::ssize_t {
    let ret: i64;
    let err: i64;
    asm!(
        "mov x16, {syscall}",
        "svc #0x80",
        "cset {err}, cs",
        syscall = in(reg) SYS_READLINKAT,
        in("x0") dirfd as i64,
        in("x1") path as i64,
        in("x2") buf as i64,
        in("x3") bufsiz as i64,
        lateout("x0") ret,
        err = out(reg) err,
        options(nostack)
    );
    if err != 0 {
        crate::set_errno(ret as libc::c_int);
        return -1;
    }
    ret as libc::ssize_t
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
const SYS_LSTAT64_X64: i64 = 340;
const SYS_GETATTRLIST_X64: i64 = 220;
const SYS_SETATTRLIST_X64: i64 = 221;

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

/// SYS_read = 3 on macOS x86_64
#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
const SYS_READ_X64: i64 = 3;

/// SYS_write = 4 on macOS x86_64
#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
const SYS_WRITE_X64: i64 = 4;

/// Raw read syscall for macOS x86_64.
#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
#[inline(never)]
pub unsafe fn raw_read(
    fd: libc::c_int,
    buf: *mut libc::c_void,
    count: libc::size_t,
) -> libc::ssize_t {
    let ret: i64;
    std::arch::asm!(
        "syscall",
        in("rax") SYS_READ_X64 | 0x2000000,
        in("rdi") fd as i64,
        in("rsi") buf as i64,
        in("rdx") count as i64,
        lateout("rax") ret,
        lateout("rcx") _,
        lateout("r11") _,
        options(nostack)
    );
    ret as libc::ssize_t
}

/// Raw write syscall for macOS x86_64.
#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
#[inline(never)]
pub unsafe fn raw_write(
    fd: libc::c_int,
    buf: *const libc::c_void,
    count: libc::size_t,
) -> libc::ssize_t {
    let ret: i64;
    std::arch::asm!(
        "syscall",
        in("rax") SYS_WRITE_X64 | 0x2000000,
        in("rdi") fd as i64,
        in("rsi") buf as i64,
        in("rdx") count as i64,
        lateout("rax") ret,
        lateout("rcx") _,
        lateout("r11") _,
        options(nostack)
    );
    ret as libc::ssize_t
}

/// SYS_dup = 41 on macOS x86_64
#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
const SYS_DUP_X64: i64 = 41;

/// SYS_dup2 = 90 on macOS x86_64
#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
const SYS_DUP2_X64: i64 = 90;

/// SYS_lseek = 199 on macOS x86_64
#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
const SYS_LSEEK_X64: i64 = 199;

/// SYS_ftruncate = 201 on macOS x86_64
#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
const SYS_FTRUNCATE_X64: i64 = 201;

/// Raw dup syscall for macOS x86_64.
#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
#[inline(never)]
pub unsafe fn raw_dup(oldfd: libc::c_int) -> libc::c_int {
    let ret: i64;
    std::arch::asm!(
        "syscall",
        in("rax") SYS_DUP_X64 | 0x2000000,
        in("rdi") oldfd as i64,
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

/// Raw dup2 syscall for macOS x86_64.
#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
#[inline(never)]
pub unsafe fn raw_dup2(oldfd: libc::c_int, newfd: libc::c_int) -> libc::c_int {
    let ret: i64;
    std::arch::asm!(
        "syscall",
        in("rax") SYS_DUP2_X64 | 0x2000000,
        in("rdi") oldfd as i64,
        in("rsi") newfd as i64,
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

/// Raw lseek syscall for macOS x86_64.
#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
#[inline(never)]
pub unsafe fn raw_lseek(fd: libc::c_int, offset: libc::off_t, whence: libc::c_int) -> libc::off_t {
    let ret: i64;
    std::arch::asm!(
        "syscall",
        in("rax") SYS_LSEEK_X64 | 0x2000000,
        in("rdi") fd as i64,
        in("rsi") offset as i64,
        in("rdx") whence as i64,
        lateout("rax") ret,
        lateout("rcx") _,
        lateout("r11") _,
        options(nostack)
    );
    ret as libc::off_t
}

/// Raw ftruncate syscall for macOS x86_64.
#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
#[inline(never)]
pub unsafe fn raw_ftruncate(fd: libc::c_int, length: libc::off_t) -> libc::c_int {
    let ret: i64;
    std::arch::asm!(
        "syscall",
        in("rax") SYS_FTRUNCATE_X64 | 0x2000000,
        in("rdi") fd as i64,
        in("rsi") length as i64,
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

/// SYS_openat = 463 on macOS x86_64
#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
const SYS_OPENAT_X64: i64 = 463;

/// SYS_fcntl = 92 on macOS x86_64
#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
const SYS_FCNTL_X64: i64 = 92;

/// Raw openat syscall for macOS x86_64.
#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
#[inline(never)]
pub unsafe fn raw_openat(
    dirfd: libc::c_int,
    path: *const libc::c_char,
    flags: libc::c_int,
    mode: libc::mode_t,
) -> libc::c_int {
    let ret: i64;
    std::arch::asm!(
        "syscall",
        in("rax") SYS_OPENAT_X64 | 0x2000000,
        in("rdi") dirfd as i64,
        in("rsi") path as i64,
        in("rdx") flags as i64,
        in("r10") mode as i64,
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

/// Raw fcntl syscall for macOS x86_64.
#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
#[inline(never)]
pub unsafe fn raw_fcntl(fd: libc::c_int, cmd: libc::c_int, arg: i64) -> libc::c_int {
    let ret: i64;
    std::arch::asm!(
        "syscall",
        in("rax") SYS_FCNTL_X64 | 0x2000000,
        in("rdi") fd as i64,
        in("rsi") cmd as i64,
        in("rdx") arg as i64,
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

/// SYS_chmod = 15 on macOS x86_64
#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
const SYS_CHMOD_X64: i64 = 15;

/// Raw chmod syscall for macOS x86_64.
#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
#[inline(never)]
pub unsafe fn raw_chmod(path: *const libc::c_char, mode: libc::mode_t) -> libc::c_int {
    let ret: i64;
    std::arch::asm!(
        "syscall",
        in("rax") SYS_CHMOD_X64 | 0x2000000,
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

/// SYS_unlink = 10 on macOS x86_64
#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
const SYS_UNLINK_X64: i64 = 10;
/// SYS_rmdir = 137 on macOS x86_64
#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
const SYS_RMDIR_X64: i64 = 137;
/// SYS_mkdir = 136 on macOS x86_64
#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
const SYS_MKDIR_X64: i64 = 136;
/// SYS_truncate on macOS x86_64 = 200
#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
const SYS_TRUNCATE_X64: i64 = 200;
/// SYS_unlinkat on macOS x86_64 = 438
#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
const SYS_UNLINKAT_X64: i64 = 472;
/// SYS_mkdirat on macOS x86_64 = 464
#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
const SYS_MKDIRAT_X64: i64 = 475;
/// SYS_symlinkat on macOS x86_64 = 465
#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
const SYS_SYMLINKAT_X64: i64 = 474;
/// SYS_fchmod on macOS x86_64 = 124
#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
const SYS_FCHMOD_X64: i64 = 124;
/// SYS_fchmodat on macOS x86_64 = 468
#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
const SYS_FCHMODAT_X64: i64 = 467;

/// SYS_fstatat64 on macOS x86_64 = 466
#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
const SYS_FSTATAT64_X64: i64 = 470;

/// Raw unlink syscall for macOS x86_64.
#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
#[inline(never)]
pub unsafe fn raw_unlink(path: *const libc::c_char) -> libc::c_int {
    let ret: i64;
    std::arch::asm!(
        "syscall",
        in("rax") SYS_UNLINK_X64 | 0x2000000,
        in("rdi") path as i64,
        lateout("rax") ret, lateout("rcx") _, lateout("r11") _,
        options(nostack)
    );
    if ret as isize > -4096 && (ret as isize) < 0 {
        -1
    } else {
        ret as libc::c_int
    }
}

/// Raw rmdir syscall for macOS x86_64.
#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
#[inline(never)]
pub unsafe fn raw_rmdir(path: *const libc::c_char) -> libc::c_int {
    let ret: i64;
    std::arch::asm!(
        "syscall",
        in("rax") SYS_RMDIR_X64 | 0x2000000,
        in("rdi") path as i64,
        lateout("rax") ret, lateout("rcx") _, lateout("r11") _,
        options(nostack)
    );
    if ret as isize > -4096 && (ret as isize) < 0 {
        -1
    } else {
        ret as libc::c_int
    }
}

/// Raw mkdir syscall for macOS x86_64.
#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
#[inline(never)]
pub unsafe fn raw_mkdir(path: *const libc::c_char, mode: libc::mode_t) -> libc::c_int {
    let ret: i64;
    std::arch::asm!(
        "syscall",
        in("rax") SYS_MKDIR_X64 | 0x2000000,
        in("rdi") path as i64,
        in("rsi") mode as i64,
        lateout("rax") ret, lateout("rcx") _, lateout("r11") _,
        options(nostack)
    );
    if ret as isize > -4096 && (ret as isize) < 0 {
        -1
    } else {
        ret as libc::c_int
    }
}

/// Raw truncate syscall for macOS x86_64.
#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
#[inline(never)]
pub unsafe fn raw_truncate(path: *const libc::c_char, length: libc::off_t) -> libc::c_int {
    let ret: i64;
    std::arch::asm!(
        "syscall",
        in("rax") SYS_TRUNCATE_X64 | 0x2000000,
        in("rdi") path as i64,
        in("rsi") length as i64,
        lateout("rax") ret, lateout("rcx") _, lateout("r11") _,
        options(nostack)
    );
    if ret as isize > -4096 && (ret as isize) < 0 {
        -1
    } else {
        ret as libc::c_int
    }
}

/// Raw unlinkat syscall for macOS x86_64.
#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
#[inline(never)]
pub unsafe fn raw_unlinkat(
    dirfd: libc::c_int,
    path: *const libc::c_char,
    flags: libc::c_int,
) -> libc::c_int {
    let ret: i64;
    std::arch::asm!(
        "syscall",
        in("rax") SYS_UNLINKAT_X64 | 0x2000000,
        in("rdi") dirfd as i64,
        in("rsi") path as i64,
        in("rdx") flags as i64,
        lateout("rax") ret, lateout("rcx") _, lateout("r11") _,
        options(nostack)
    );
    if ret as isize > -4096 && (ret as isize) < 0 {
        -1
    } else {
        ret as libc::c_int
    }
}

/// Raw mkdirat syscall for macOS x86_64.
#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
#[inline(never)]
pub unsafe fn raw_mkdirat(
    dirfd: libc::c_int,
    path: *const libc::c_char,
    mode: libc::mode_t,
) -> libc::c_int {
    let ret: i64;
    std::arch::asm!(
        "syscall",
        in("rax") SYS_MKDIRAT_X64 | 0x2000000,
        in("rdi") dirfd as i64,
        in("rsi") path as i64,
        in("rdx") mode as i64,
        lateout("rax") ret, lateout("rcx") _, lateout("r11") _,
        options(nostack)
    );
    if ret as isize > -4096 && (ret as isize) < 0 {
        -1
    } else {
        ret as libc::c_int
    }
}

/// Raw symlinkat syscall for macOS x86_64.
#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
#[inline(never)]
pub unsafe fn raw_symlinkat(
    p1: *const libc::c_char,
    dirfd: libc::c_int,
    p2: *const libc::c_char,
) -> libc::c_int {
    let ret: i64;
    std::arch::asm!(
        "syscall",
        in("rax") SYS_SYMLINKAT_X64 | 0x2000000,
        in("rdi") p1 as i64,
        in("rsi") dirfd as i64,
        in("rdx") p2 as i64,
        lateout("rax") ret, lateout("rcx") _, lateout("r11") _,
        options(nostack)
    );
    if ret as isize > -4096 && (ret as isize) < 0 {
        -1
    } else {
        ret as libc::c_int
    }
}

/// Raw fchmod syscall for macOS x86_64.
#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
#[inline(never)]
pub unsafe fn raw_fchmod(fd: libc::c_int, mode: libc::mode_t) -> libc::c_int {
    let ret: i64;
    std::arch::asm!(
        "syscall",
        in("rax") SYS_FCHMOD_X64 | 0x2000000,
        in("rdi") fd as i64,
        in("rsi") mode as i64,
        lateout("rax") ret, lateout("rcx") _, lateout("r11") _,
        options(nostack)
    );
    if ret as isize > -4096 && (ret as isize) < 0 {
        -1
    } else {
        ret as libc::c_int
    }
}

/// Raw fchmodat syscall for macOS x86_64.
#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
#[inline(never)]
pub unsafe fn raw_fchmodat(
    dirfd: libc::c_int,
    path: *const libc::c_char,
    mode: libc::mode_t,
    flags: libc::c_int,
) -> libc::c_int {
    let ret: i64;
    std::arch::asm!(
        "syscall",
        in("rax") SYS_FCHMODAT_X64 | 0x2000000,
        in("rdi") dirfd as i64,
        in("rsi") path as i64,
        in("rdx") mode as i64,
        in("r10") flags as i64,
        lateout("rax") ret, lateout("rcx") _, lateout("r11") _,
        options(nostack)
    );
    if ret as isize > -4096 && (ret as isize) < 0 {
        -1
    } else {
        ret as libc::c_int
    }
}

/// Raw fstatat64 syscall for macOS x86_64.
#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
#[inline(never)]
pub unsafe fn raw_fstatat64(
    dirfd: libc::c_int,
    path: *const libc::c_char,
    buf: *mut libc::stat,
    flags: libc::c_int,
) -> libc::c_int {
    let ret: i64;
    std::arch::asm!(
        "syscall",
        in("rax") SYS_FSTATAT64_X64 | 0x2000000,
        in("rdi") dirfd as i64,
        in("rsi") path as i64,
        in("rdx") buf as i64,
        in("r10") flags as i64,
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

/// Raw getattrlist syscall for macOS x86_64.
#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
#[inline(never)]
pub unsafe fn raw_getattrlist(
    path: *const libc::c_char,
    attrlist: *mut libc::c_void,
    attrbuf: *mut libc::c_void,
    attrbufsize: libc::size_t,
    options: libc::c_ulong,
) -> libc::c_int {
    let ret: i64;
    std::arch::asm!(
        "syscall",
        in("rax") SYS_GETATTRLIST_X64 | 0x2000000,
        in("rdi") path as i64,
        in("rsi") attrlist as i64,
        in("rdx") attrbuf as i64,
        in("r10") attrbufsize as i64,
        in("r8") options as i64,
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

/// Raw setattrlist syscall for macOS x86_64.
#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
#[inline(never)]
pub unsafe fn raw_setattrlist(
    path: *const libc::c_char,
    attrlist: *mut libc::c_void,
    attrbuf: *mut libc::c_void,
    attrbufsize: libc::size_t,
    options: libc::c_ulong,
) -> libc::c_int {
    let ret: i64;
    std::arch::asm!(
        "syscall",
        in("rax") SYS_SETATTRLIST_X64 | 0x2000000,
        in("rdi") path as i64,
        in("rsi") attrlist as i64,
        in("rdx") attrbuf as i64,
        in("r10") attrbufsize as i64,
        in("r8") options as i64,
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

#[cfg(target_os = "linux")]
pub unsafe fn raw_read(
    fd: libc::c_int,
    buf: *mut libc::c_void,
    count: libc::size_t,
) -> libc::ssize_t {
    crate::syscalls::linux_raw::raw_read(fd, buf, count)
}

#[cfg(target_os = "linux")]
pub unsafe fn raw_write(
    fd: libc::c_int,
    buf: *const libc::c_void,
    count: libc::size_t,
) -> libc::ssize_t {
    crate::syscalls::linux_raw::raw_write(fd, buf, count)
}

#[cfg(target_os = "linux")]
pub unsafe fn raw_dup(oldfd: libc::c_int) -> libc::c_int {
    crate::syscalls::linux_raw::raw_dup(oldfd)
}

#[cfg(target_os = "linux")]
pub unsafe fn raw_dup2(oldfd: libc::c_int, newfd: libc::c_int) -> libc::c_int {
    crate::syscalls::linux_raw::raw_dup2(oldfd, newfd)
}

#[cfg(target_os = "linux")]
pub unsafe fn raw_lseek(fd: libc::c_int, offset: libc::off_t, whence: libc::c_int) -> libc::off_t {
    crate::syscalls::linux_raw::raw_lseek(fd, offset, whence)
}

#[cfg(target_os = "linux")]
pub unsafe fn raw_ftruncate(fd: libc::c_int, length: libc::off_t) -> libc::c_int {
    crate::syscalls::linux_raw::raw_ftruncate(fd, length)
}

#[cfg(target_os = "linux")]
pub unsafe fn raw_openat(
    dirfd: libc::c_int,
    path: *const libc::c_char,
    flags: libc::c_int,
    mode: libc::mode_t,
) -> libc::c_int {
    crate::syscalls::linux_raw::raw_openat(dirfd, path, flags, mode)
}

#[cfg(target_os = "linux")]
pub unsafe fn raw_fcntl(fd: libc::c_int, cmd: libc::c_int, arg: i64) -> libc::c_int {
    crate::syscalls::linux_raw::raw_fcntl(fd, cmd, arg)
}

#[cfg(target_os = "linux")]
pub unsafe fn raw_chmod(path: *const libc::c_char, mode: libc::mode_t) -> libc::c_int {
    crate::syscalls::linux_raw::raw_chmod(path, mode)
}

#[cfg(target_os = "linux")]
pub unsafe fn raw_unlink(path: *const libc::c_char) -> libc::c_int {
    crate::syscalls::linux_raw::raw_unlink(path)
}

#[cfg(target_os = "linux")]
pub unsafe fn raw_rmdir(path: *const libc::c_char) -> libc::c_int {
    crate::syscalls::linux_raw::raw_rmdir(path)
}

#[cfg(target_os = "linux")]
pub unsafe fn raw_mkdir(path: *const libc::c_char, mode: libc::mode_t) -> libc::c_int {
    crate::syscalls::linux_raw::raw_mkdir(path, mode)
}

#[cfg(target_os = "linux")]
pub unsafe fn raw_truncate(path: *const libc::c_char, length: libc::off_t) -> libc::c_int {
    crate::syscalls::linux_raw::raw_truncate(path, length)
}

#[cfg(target_os = "linux")]
pub unsafe fn raw_fstatat64(
    dirfd: libc::c_int,
    path: *const libc::c_char,
    buf: *mut libc::stat,
    flags: libc::c_int,
) -> libc::c_int {
    crate::syscalls::linux_raw::raw_fstatat(dirfd, path, buf, flags)
}

#[cfg(target_os = "linux")]
pub unsafe fn raw_fchown(fd: libc::c_int, owner: libc::uid_t, group: libc::gid_t) -> libc::c_int {
    crate::syscalls::linux_raw::raw_fchown(fd, owner, group)
}

#[cfg(target_os = "linux")]
pub unsafe fn raw_fchownat(
    dirfd: libc::c_int,
    path: *const libc::c_char,
    owner: libc::uid_t,
    group: libc::gid_t,
    flags: libc::c_int,
) -> libc::c_int {
    crate::syscalls::linux_raw::raw_fchownat(dirfd, path, owner, group, flags)
}
/// raw_utimensat — dlsym-based passthrough for macOS.
/// macOS has NO kernel syscall for utimensat; it is a libc wrapper over setattrlist.
/// We resolve the real libc function via dlsym(RTLD_NEXT) to avoid interposition recursion.
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
pub unsafe fn raw_utimensat(
    dirfd: c_int,
    path: *const c_char,
    times: *const libc::timespec,
    flags: c_int,
) -> c_int {
    use std::sync::atomic::Ordering;

    // Bootstrap safety: during early init, skip timestamp operations (non-critical)
    let init_state = crate::state::INITIALIZING.load(Ordering::Relaxed);
    if init_state >= 2 {
        return 0;
    }

    let real_func = crate::reals::REAL_UTIMENSAT.get();
    if real_func.is_null() {
        crate::set_errno(libc::ENOSYS);
        return -1;
    }
    let func: unsafe extern "C" fn(c_int, *const c_char, *const libc::timespec, c_int) -> c_int =
        std::mem::transmute(real_func);
    func(dirfd, path, times, flags)
}

/// raw_futimens — delegates to raw_utimensat with AT_FDCWD and null path.
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
pub unsafe fn raw_futimens(fd: c_int, times: *const libc::timespec) -> c_int {
    use std::sync::atomic::Ordering;

    // Bootstrap safety: during early init, skip timestamp operations (non-critical)
    let init_state = crate::state::INITIALIZING.load(Ordering::Relaxed);
    if init_state >= 2 {
        return 0;
    }

    let real_func = crate::reals::REAL_FUTIMENS.get();
    if real_func.is_null() {
        crate::set_errno(libc::ENOSYS);
        return -1;
    }
    let func: unsafe extern "C" fn(c_int, *const libc::timespec) -> c_int =
        std::mem::transmute(real_func);
    func(fd, times)
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
pub unsafe fn raw_flock(fd: c_int, operation: c_int) -> c_int {
    let ret: i64;
    asm!(
        "svc #0x80",
        in("x16") SYS_FLOCK,
        in("x0") fd as i64,
        in("x1") operation as i64,
        lateout("x0") ret,
    );
    if ret < 0 {
        crate::set_errno(-(ret as i32));
        -1
    } else {
        ret as c_int
    }
}
