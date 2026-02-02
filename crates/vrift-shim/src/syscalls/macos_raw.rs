//! Raw macOS syscall wrappers for bootstrap-safe operations.
//!
//! BUG-007: During dyld bootstrap (__malloc_init), calling libc functions can cause
//! infinite recursion when DYLD_INSERT_LIBRARIES is active because the interposed
//! functions call back into our shim.
//!
//! This module provides bare-metal syscall wrappers using inline assembly that
//! bypass libc entirely, making them safe to call at any stage of initialization.
//!
//! Syscall numbers from: /usr/include/sys/syscall.h

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

// Stub implementations for other platforms
#[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
pub unsafe fn raw_fstat64(_fd: libc::c_int, _buf: *mut libc::stat) -> libc::c_int {
    -1 // Not supported
}

#[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
pub unsafe fn raw_close(_fd: libc::c_int) -> libc::c_int {
    -1
}

#[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
pub unsafe fn raw_mmap(
    _addr: *mut libc::c_void,
    _len: libc::size_t,
    _prot: libc::c_int,
    _flags: libc::c_int,
    _fd: libc::c_int,
    _offset: libc::off_t,
) -> *mut libc::c_void {
    std::ptr::null_mut()
}

#[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
pub unsafe fn raw_munmap(_addr: *mut libc::c_void, _len: libc::size_t) -> libc::c_int {
    -1
}

#[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
pub unsafe fn raw_access(_path: *const libc::c_char, _mode: libc::c_int) -> libc::c_int {
    -1
}
