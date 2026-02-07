//! Linux Raw Syscall Wrappers
//!
//! Pattern 2692: Hand-rolled raw assembly syscalls to avoid recursion.
//! These bypass libc entirely and call kernel syscalls directly.
//!
//! Supports both AArch64 and x86_64 architectures.

use libc::{c_char, c_int, c_void, mode_t, off_t, size_t, ssize_t, stat as libc_stat};

/// Set errno from negative syscall return value
#[inline(always)]
unsafe fn set_errno_from_ret(ret: i64) {
    if ret < 0 {
        crate::set_errno(-ret as c_int);
    }
}

// =============================================================================
// File Operations
// =============================================================================

/// Raw open syscall - bypasses libc entirely
#[inline(always)]
pub unsafe fn raw_open(path: *const c_char, flags: c_int, mode: mode_t) -> c_int {
    #[cfg(target_arch = "x86_64")]
    {
        let ret: i64;
        std::arch::asm!(
            "syscall",
            in("rax") 2i64, // SYS_open
            in("rdi") path,
            in("rsi") flags as i64,
            in("rdx") mode as i64,
            lateout("rax") ret,
            lateout("rcx") _,
            lateout("r11") _,
        );
        if ret < 0 {
            set_errno_from_ret(ret);
            -1
        } else {
            ret as c_int
        }
    }
    #[cfg(target_arch = "aarch64")]
    {
        // AArch64 uses openat with AT_FDCWD (-100)
        let ret: i64;
        std::arch::asm!(
            "svc #0",
            in("x8") 56i64, // SYS_openat
            in("x0") -100i64, // AT_FDCWD
            in("x1") path,
            in("x2") flags as i64,
            in("x3") mode as i64,
            lateout("x0") ret,
        );
        if ret < 0 {
            set_errno_from_ret(ret);
            -1
        } else {
            ret as c_int
        }
    }
}

/// Raw openat syscall
#[inline(always)]
pub unsafe fn raw_openat(dirfd: c_int, path: *const c_char, flags: c_int, mode: mode_t) -> c_int {
    #[cfg(target_arch = "x86_64")]
    {
        let ret: i64;
        std::arch::asm!(
            "syscall",
            in("rax") 257i64, // SYS_openat
            in("rdi") dirfd as i64,
            in("rsi") path,
            in("rdx") flags as i64,
            in("r10") mode as i64,
            lateout("rax") ret,
            lateout("rcx") _,
            lateout("r11") _,
        );
        if ret < 0 {
            set_errno_from_ret(ret);
            -1
        } else {
            ret as c_int
        }
    }
    #[cfg(target_arch = "aarch64")]
    {
        let ret: i64;
        std::arch::asm!(
            "svc #0",
            in("x8") 56i64, // SYS_openat
            in("x0") dirfd as i64,
            in("x1") path,
            in("x2") flags as i64,
            in("x3") mode as i64,
            lateout("x0") ret,
        );
        if ret < 0 {
            set_errno_from_ret(ret);
            -1
        } else {
            ret as c_int
        }
    }
}

/// Raw unlinkat syscall
#[inline(always)]
pub unsafe fn raw_unlinkat(dirfd: c_int, path: *const c_char, flags: c_int) -> c_int {
    #[cfg(target_arch = "x86_64")]
    {
        let ret: i64;
        std::arch::asm!(
            "syscall",
            in("rax") 263i64, // SYS_unlinkat
            in("rdi") dirfd as i64,
            in("rsi") path,
            in("rdx") flags as i64,
            lateout("rax") ret,
            lateout("rcx") _,
            lateout("r11") _,
        );
        if ret < 0 {
            set_errno_from_ret(ret);
            -1
        } else {
            ret as c_int
        }
    }
    #[cfg(target_arch = "aarch64")]
    {
        let ret: i64;
        std::arch::asm!(
            "svc #0",
            in("x8") 35i64, // SYS_unlinkat
            in("x0") dirfd as i64,
            in("x1") path,
            in("x2") flags as i64,
            lateout("x0") ret,
        );
        if ret < 0 {
            set_errno_from_ret(ret);
            -1
        } else {
            ret as c_int
        }
    }
}

/// Raw mkdirat syscall
#[inline(always)]
pub unsafe fn raw_mkdirat(dirfd: c_int, path: *const c_char, mode: mode_t) -> c_int {
    #[cfg(target_arch = "x86_64")]
    {
        let ret: i64;
        std::arch::asm!(
            "syscall",
            in("rax") 258i64, // SYS_mkdirat
            in("rdi") dirfd as i64,
            in("rsi") path,
            in("rdx") mode as i64,
            lateout("rax") ret,
            lateout("rcx") _,
            lateout("r11") _,
        );
        if ret < 0 {
            set_errno_from_ret(ret);
            -1
        } else {
            ret as c_int
        }
    }
    #[cfg(target_arch = "aarch64")]
    {
        let ret: i64;
        std::arch::asm!(
            "svc #0",
            in("x8") 34i64, // SYS_mkdirat
            in("x0") dirfd as i64,
            in("x1") path,
            in("x2") mode as i64,
            lateout("x0") ret,
        );
        if ret < 0 {
            set_errno_from_ret(ret);
            -1
        } else {
            ret as c_int
        }
    }
}

/// Raw close syscall
#[inline(always)]
pub unsafe fn raw_close(fd: c_int) -> c_int {
    #[cfg(target_arch = "x86_64")]
    {
        let ret: i64;
        std::arch::asm!(
            "syscall",
            in("rax") 3i64, // SYS_close
            in("rdi") fd as i64,
            lateout("rax") ret,
            lateout("rcx") _,
            lateout("r11") _,
        );
        if ret < 0 {
            set_errno_from_ret(ret);
            -1
        } else {
            ret as c_int
        }
    }
    #[cfg(target_arch = "aarch64")]
    {
        let ret: i64;
        std::arch::asm!(
            "svc #0",
            in("x8") 57i64, // SYS_close
            in("x0") fd as i64,
            lateout("x0") ret,
        );
        if ret < 0 {
            set_errno_from_ret(ret);
            -1
        } else {
            ret as c_int
        }
    }
}

/// Raw read syscall
#[inline(always)]
pub unsafe fn raw_read(fd: c_int, buf: *mut c_void, count: size_t) -> ssize_t {
    #[cfg(target_arch = "x86_64")]
    {
        let ret: i64;
        std::arch::asm!(
            "syscall",
            in("rax") 0i64, // SYS_read
            in("rdi") fd as i64,
            in("rsi") buf,
            in("rdx") count as i64,
            lateout("rax") ret,
            lateout("rcx") _,
            lateout("r11") _,
        );
        if ret < 0 {
            set_errno_from_ret(ret);
            -1
        } else {
            ret as ssize_t
        }
    }
    #[cfg(target_arch = "aarch64")]
    {
        let ret: i64;
        std::arch::asm!(
            "svc #0",
            in("x8") 63i64, // SYS_read
            in("x0") fd as i64,
            in("x1") buf,
            in("x2") count as i64,
            lateout("x0") ret,
        );
        if ret < 0 {
            set_errno_from_ret(ret);
            -1
        } else {
            ret as ssize_t
        }
    }
}

/// Raw write syscall
#[inline(always)]
pub unsafe fn raw_write(fd: c_int, buf: *const c_void, count: size_t) -> ssize_t {
    #[cfg(target_arch = "x86_64")]
    {
        let ret: i64;
        std::arch::asm!(
            "syscall",
            in("rax") 1i64, // SYS_write
            in("rdi") fd as i64,
            in("rsi") buf,
            in("rdx") count as i64,
            lateout("rax") ret,
            lateout("rcx") _,
            lateout("r11") _,
        );
        if ret < 0 {
            set_errno_from_ret(ret);
            -1
        } else {
            ret as ssize_t
        }
    }
    #[cfg(target_arch = "aarch64")]
    {
        let ret: i64;
        std::arch::asm!(
            "svc #0",
            in("x8") 64i64, // SYS_write
            in("x0") fd as i64,
            in("x1") buf,
            in("x2") count as i64,
            lateout("x0") ret,
        );
        if ret < 0 {
            set_errno_from_ret(ret);
            -1
        } else {
            ret as ssize_t
        }
    }
}

// =============================================================================
// Directory Operations
// =============================================================================

/// Raw mkdir syscall
#[inline(always)]
pub unsafe fn raw_mkdir(path: *const c_char, mode: mode_t) -> c_int {
    #[cfg(target_arch = "x86_64")]
    {
        let ret: i64;
        std::arch::asm!(
            "syscall",
            in("rax") 83i64, // SYS_mkdir
            in("rdi") path,
            in("rsi") mode as i64,
            lateout("rax") ret,
            lateout("rcx") _,
            lateout("r11") _,
        );
        if ret < 0 {
            set_errno_from_ret(ret);
            -1
        } else {
            ret as c_int
        }
    }
    #[cfg(target_arch = "aarch64")]
    {
        // AArch64 uses mkdirat
        let ret: i64;
        std::arch::asm!(
            "svc #0",
            in("x8") 34i64, // SYS_mkdirat
            in("x0") -100i64, // AT_FDCWD
            in("x1") path,
            in("x2") mode as i64,
            lateout("x0") ret,
        );
        if ret < 0 {
            set_errno_from_ret(ret);
            -1
        } else {
            ret as c_int
        }
    }
}

/// Raw rmdir syscall
#[inline(always)]
pub unsafe fn raw_rmdir(path: *const c_char) -> c_int {
    #[cfg(target_arch = "x86_64")]
    {
        let ret: i64;
        std::arch::asm!(
            "syscall",
            in("rax") 84i64, // SYS_rmdir
            in("rdi") path,
            lateout("rax") ret,
            lateout("rcx") _,
            lateout("r11") _,
        );
        if ret < 0 {
            set_errno_from_ret(ret);
            -1
        } else {
            ret as c_int
        }
    }
    #[cfg(target_arch = "aarch64")]
    {
        // AArch64 uses unlinkat with AT_REMOVEDIR flag
        let ret: i64;
        std::arch::asm!(
            "svc #0",
            in("x8") 35i64, // SYS_unlinkat
            in("x0") -100i64, // AT_FDCWD
            in("x1") path,
            in("x2") 0x200i64, // AT_REMOVEDIR
            lateout("x0") ret,
        );
        if ret < 0 {
            set_errno_from_ret(ret);
            -1
        } else {
            ret as c_int
        }
    }
}

/// Raw unlink syscall
#[inline(always)]
pub unsafe fn raw_unlink(path: *const c_char) -> c_int {
    #[cfg(target_arch = "x86_64")]
    {
        let ret: i64;
        std::arch::asm!(
            "syscall",
            in("rax") 87i64, // SYS_unlink
            in("rdi") path,
            lateout("rax") ret,
            lateout("rcx") _,
            lateout("r11") _,
        );
        if ret < 0 {
            set_errno_from_ret(ret);
            -1
        } else {
            ret as c_int
        }
    }
    #[cfg(target_arch = "aarch64")]
    {
        let ret: i64;
        std::arch::asm!(
            "svc #0",
            in("x8") 35i64, // SYS_unlinkat
            in("x0") -100i64, // AT_FDCWD
            in("x1") path,
            in("x2") 0i64, // no flags
            lateout("x0") ret,
        );
        if ret < 0 {
            set_errno_from_ret(ret);
            -1
        } else {
            ret as c_int
        }
    }
}

// =============================================================================
// Permission Operations
// =============================================================================

/// Raw chmod syscall
#[inline(always)]
pub unsafe fn raw_chmod(path: *const c_char, mode: mode_t) -> c_int {
    #[cfg(target_arch = "x86_64")]
    {
        let ret: i64;
        std::arch::asm!(
            "syscall",
            in("rax") 90i64, // SYS_chmod
            in("rdi") path,
            in("rsi") mode as i64,
            lateout("rax") ret,
            lateout("rcx") _,
            lateout("r11") _,
        );
        if ret < 0 {
            set_errno_from_ret(ret);
            -1
        } else {
            ret as c_int
        }
    }
    #[cfg(target_arch = "aarch64")]
    {
        // AArch64 uses fchmodat
        let ret: i64;
        std::arch::asm!(
            "svc #0",
            in("x8") 53i64, // SYS_fchmodat
            in("x0") -100i64, // AT_FDCWD
            in("x1") path,
            in("x2") mode as i64,
            in("x3") 0i64, // no flags
            lateout("x0") ret,
        );
        if ret < 0 {
            set_errno_from_ret(ret);
            -1
        } else {
            ret as c_int
        }
    }
}

/// Raw fchmodat syscall
#[inline(always)]
pub unsafe fn raw_fchmodat(dirfd: c_int, path: *const c_char, mode: mode_t, flags: c_int) -> c_int {
    #[cfg(target_arch = "x86_64")]
    {
        let ret: i64;
        std::arch::asm!(
            "syscall",
            in("rax") 268i64, // SYS_fchmodat
            in("rdi") dirfd as i64,
            in("rsi") path,
            in("rdx") mode as i64,
            in("r10") flags as i64,
            lateout("rax") ret,
            lateout("rcx") _,
            lateout("r11") _,
        );
        if ret < 0 {
            set_errno_from_ret(ret);
            -1
        } else {
            ret as c_int
        }
    }
    #[cfg(target_arch = "aarch64")]
    {
        let ret: i64;
        std::arch::asm!(
            "svc #0",
            in("x8") 53i64, // SYS_fchmodat
            in("x0") dirfd as i64,
            in("x1") path,
            in("x2") mode as i64,
            in("x3") flags as i64,
            lateout("x0") ret,
        );
        if ret < 0 {
            set_errno_from_ret(ret);
            -1
        } else {
            ret as c_int
        }
    }
}

// =============================================================================
// Link Operations
// =============================================================================

/// Raw link syscall (hardlink)
#[inline(always)]
pub unsafe fn raw_link(old: *const c_char, new: *const c_char) -> c_int {
    #[cfg(target_arch = "x86_64")]
    {
        let ret: i64;
        std::arch::asm!(
            "syscall",
            in("rax") 86i64, // SYS_link
            in("rdi") old,
            in("rsi") new,
            lateout("rax") ret,
            lateout("rcx") _,
            lateout("r11") _,
        );
        if ret < 0 {
            set_errno_from_ret(ret);
            -1
        } else {
            ret as c_int
        }
    }
    #[cfg(target_arch = "aarch64")]
    {
        // AArch64 uses linkat
        let ret: i64;
        std::arch::asm!(
            "svc #0",
            in("x8") 37i64, // SYS_linkat
            in("x0") -100i64, // AT_FDCWD
            in("x1") old,
            in("x2") -100i64, // AT_FDCWD
            in("x3") new,
            in("x4") 0i64, // no flags
            lateout("x0") ret,
        );
        if ret < 0 {
            set_errno_from_ret(ret);
            -1
        } else {
            ret as c_int
        }
    }
}

/// Raw linkat syscall
#[inline(always)]
pub unsafe fn raw_linkat(
    olddirfd: c_int,
    old: *const c_char,
    newdirfd: c_int,
    new: *const c_char,
    flags: c_int,
) -> c_int {
    #[cfg(target_arch = "x86_64")]
    {
        let ret: i64;
        std::arch::asm!(
            "syscall",
            in("rax") 265i64, // SYS_linkat
            in("rdi") olddirfd as i64,
            in("rsi") old,
            in("rdx") newdirfd as i64,
            in("r10") new,
            in("r8") flags as i64,
            lateout("rax") ret,
            lateout("rcx") _,
            lateout("r11") _,
        );
        if ret < 0 {
            set_errno_from_ret(ret);
            -1
        } else {
            ret as c_int
        }
    }
    #[cfg(target_arch = "aarch64")]
    {
        let ret: i64;
        std::arch::asm!(
            "svc #0",
            in("x8") 37i64, // SYS_linkat
            in("x0") olddirfd as i64,
            in("x1") old,
            in("x2") newdirfd as i64,
            in("x3") new,
            in("x4") flags as i64,
            lateout("x0") ret,
        );
        if ret < 0 {
            set_errno_from_ret(ret);
            -1
        } else {
            ret as c_int
        }
    }
}

/// Raw renameat syscall
#[inline(always)]
pub unsafe fn raw_renameat(
    olddirfd: c_int,
    old: *const c_char,
    newdirfd: c_int,
    new: *const c_char,
) -> c_int {
    #[cfg(target_arch = "x86_64")]
    {
        let ret: i64;
        std::arch::asm!(
            "syscall",
            in("rax") 264i64, // SYS_renameat
            in("rdi") olddirfd as i64,
            in("rsi") old,
            in("rdx") newdirfd as i64,
            in("r10") new,
            lateout("rax") ret,
            lateout("rcx") _,
            lateout("r11") _,
        );
        if ret < 0 {
            set_errno_from_ret(ret);
            -1
        } else {
            ret as c_int
        }
    }
    #[cfg(target_arch = "aarch64")]
    {
        let ret: i64;
        std::arch::asm!(
            "svc #0",
            in("x8") 38i64, // SYS_renameat
            in("x0") olddirfd as i64,
            in("x1") old,
            in("x2") newdirfd as i64,
            in("x3") new,
            in("x4") 0,
            lateout("x0") ret,
        );
        if ret < 0 {
            set_errno_from_ret(ret);
            -1
        } else {
            ret as c_int
        }
    }
}

/// Raw symlink syscall
#[inline(always)]
pub unsafe fn raw_symlink(old: *const c_char, new: *const c_char) -> c_int {
    #[cfg(target_arch = "x86_64")]
    {
        let ret: i64;
        std::arch::asm!(
            "syscall",
            in("rax") 88i64, // SYS_symlink
            in("rdi") old,
            in("rsi") new,
            lateout("rax") ret,
            lateout("rcx") _,
            lateout("r11") _,
        );
        if ret < 0 {
            set_errno_from_ret(ret);
            -1
        } else {
            ret as c_int
        }
    }
    #[cfg(target_arch = "aarch64")]
    {
        // AArch64 uses symlinkat
        let ret: i64;
        std::arch::asm!(
            "svc #0",
            in("x8") 36i64, // SYS_symlinkat
            in("x0") old,
            in("x1") -100i64, // AT_FDCWD
            in("x2") new,
            lateout("x0") ret,
        );
        if ret < 0 {
            set_errno_from_ret(ret);
            -1
        } else {
            ret as c_int
        }
    }
}

/// Raw symlinkat syscall
#[inline(always)]
pub unsafe fn raw_symlinkat(p1: *const c_char, dirfd: c_int, p2: *const c_char) -> c_int {
    #[cfg(target_arch = "x86_64")]
    {
        let ret: i64;
        std::arch::asm!(
            "syscall",
            in("rax") 266i64, // SYS_symlinkat
            in("rdi") p1,
            in("rsi") dirfd as i64,
            in("rdx") p2,
            lateout("rax") ret,
            lateout("rcx") _,
            lateout("r11") _,
        );
        if ret < 0 {
            set_errno_from_ret(ret);
            -1
        } else {
            ret as c_int
        }
    }
    #[cfg(target_arch = "aarch64")]
    {
        let ret: i64;
        std::arch::asm!(
            "svc #0",
            in("x8") 36i64, // SYS_symlinkat
            in("x0") p1,
            in("x1") dirfd as i64,
            in("x2") p2,
            lateout("x0") ret,
        );
        if ret < 0 {
            set_errno_from_ret(ret);
            -1
        } else {
            ret as c_int
        }
    }
}

// =============================================================================
// Stat Operations
// =============================================================================

/// Raw fstatat syscall
#[inline(always)]
pub unsafe fn raw_fstatat(
    dirfd: c_int,
    path: *const c_char,
    buf: *mut libc_stat,
    flags: c_int,
) -> c_int {
    #[cfg(target_arch = "x86_64")]
    {
        let ret: i64;
        std::arch::asm!(
            "syscall",
            in("rax") 262i64, // SYS_newfstatat
            in("rdi") dirfd as i64,
            in("rsi") path,
            in("rdx") buf,
            in("r10") flags as i64,
            lateout("rax") ret,
            lateout("rcx") _,
            lateout("r11") _,
        );
        if ret < 0 {
            set_errno_from_ret(ret);
            -1
        } else {
            ret as c_int
        }
    }
    #[cfg(target_arch = "aarch64")]
    {
        let ret: i64;
        std::arch::asm!(
            "svc #0",
            in("x8") 79i64, // SYS_fstatat
            in("x0") dirfd as i64,
            in("x1") path,
            in("x2") buf,
            in("x3") flags as i64,
            lateout("x0") ret,
        );
        if ret < 0 {
            set_errno_from_ret(ret);
            -1
        } else {
            ret as c_int
        }
    }
}

/// Raw stat - wrapper around fstatat with AT_FDCWD
#[inline(always)]
pub unsafe fn raw_stat(path: *const c_char, buf: *mut libc_stat) -> c_int {
    raw_fstatat(libc::AT_FDCWD, path, buf, 0)
}

/// Raw lstat - wrapper around fstatat with AT_SYMLINK_NOFOLLOW
#[inline(always)]
pub unsafe fn raw_lstat(path: *const c_char, buf: *mut libc_stat) -> c_int {
    raw_fstatat(libc::AT_FDCWD, path, buf, libc::AT_SYMLINK_NOFOLLOW)
}

/// Raw fstat syscall
#[inline(always)]
pub unsafe fn raw_fstat(fd: c_int, buf: *mut libc_stat) -> c_int {
    #[cfg(target_arch = "x86_64")]
    {
        let ret: i64;
        std::arch::asm!(
            "syscall",
            in("rax") 5i64, // SYS_fstat
            in("rdi") fd as i64,
            in("rsi") buf,
            lateout("rax") ret,
            lateout("rcx") _,
            lateout("r11") _,
        );
        if ret < 0 {
            set_errno_from_ret(ret);
            -1
        } else {
            ret as c_int
        }
    }
    #[cfg(target_arch = "aarch64")]
    {
        let ret: i64;
        std::arch::asm!(
            "svc #0",
            in("x8") 80i64, // SYS_fstat
            in("x0") fd as i64,
            in("x1") buf,
            lateout("x0") ret,
        );
        if ret < 0 {
            set_errno_from_ret(ret);
            -1
        } else {
            ret as c_int
        }
    }
}

/// Raw fchmod syscall
#[inline(always)]
pub unsafe fn raw_fchmod(fd: c_int, mode: mode_t) -> c_int {
    #[cfg(target_arch = "x86_64")]
    {
        let ret: i64;
        std::arch::asm!(
            "syscall",
            in("rax") 91i64, // SYS_fchmod
            in("rdi") fd as i64,
            in("rsi") mode as i64,
            lateout("rax") ret,
            lateout("rcx") _,
            lateout("r11") _,
        );
        if ret < 0 {
            set_errno_from_ret(ret);
            -1
        } else {
            ret as c_int
        }
    }
    #[cfg(target_arch = "aarch64")]
    {
        let ret: i64;
        std::arch::asm!(
            "svc #0",
            in("x8") 53i64, // SYS_fchmodat
            in("x0") fd as i64,
            in("x1") 0i64, // NULL path for fchmod simulation via fchmodat
            in("x2") mode as i64,
            in("x3") 0i64,
            lateout("x0") ret,
        );
        if ret < 0 {
            set_errno_from_ret(ret);
            -1
        } else {
            ret as c_int
        }
    }
}

/// Raw utimensat syscall (for touch interception)
#[inline(always)]
pub unsafe fn raw_utimensat(
    dirfd: c_int,
    path: *const c_char,
    times: *const libc::timespec,
    flags: c_int,
) -> c_int {
    #[cfg(target_arch = "x86_64")]
    {
        let ret: i64;
        std::arch::asm!(
            "syscall",
            in("rax") 280i64, // SYS_utimensat
            in("rdi") dirfd as i64,
            in("rsi") path,
            in("rdx") times,
            in("r10") flags as i64,
            lateout("rax") ret,
            lateout("rcx") _,
            lateout("r11") _,
        );
        if ret < 0 {
            set_errno_from_ret(ret);
            -1
        } else {
            ret as c_int
        }
    }
    #[cfg(target_arch = "aarch64")]
    {
        let ret: i64;
        std::arch::asm!(
            "svc #0",
            in("x8") 88i64, // SYS_utimensat
            in("x0") dirfd as i64,
            in("x1") path,
            in("x2") times,
            in("x3") flags as i64,
            lateout("x0") ret,
        );
        if ret < 0 {
            set_errno_from_ret(ret);
            -1
        } else {
            ret as c_int
        }
    }
}

/// Raw utimes syscall (for touch interception - uses utimensat internally)
#[inline(always)]
pub unsafe fn raw_utimes(path: *const c_char, times: *const libc::timeval) -> c_int {
    // utimes is implemented via utimensat on modern Linux
    // Convert timeval to timespec if times is not NULL
    if times.is_null() {
        raw_utimensat(libc::AT_FDCWD, path, std::ptr::null(), 0)
    } else {
        let times_array = std::slice::from_raw_parts(times, 2);
        let ts = [
            libc::timespec {
                tv_sec: times_array[0].tv_sec,
                tv_nsec: times_array[0].tv_usec * 1000,
            },
            libc::timespec {
                tv_sec: times_array[1].tv_sec,
                tv_nsec: times_array[1].tv_usec * 1000,
            },
        ];
        raw_utimensat(libc::AT_FDCWD, path, ts.as_ptr(), 0)
    }
}

/// Raw statx syscall
#[inline(always)]
pub unsafe fn raw_statx(
    dirfd: c_int,
    path: *const c_char,
    flags: c_int,
    mask: libc::c_uint,
    buf: *mut c_void,
) -> c_int {
    #[cfg(target_arch = "x86_64")]
    {
        let ret: i64;
        std::arch::asm!(
            "syscall",
            in("rax") 332i64, // SYS_statx
            in("rdi") dirfd as i64,
            in("rsi") path,
            in("rdx") flags as i64,
            in("r10") mask as i64,
            in("r8") buf,
            lateout("rax") ret,
            lateout("rcx") _,
            lateout("r11") _,
        );
        if ret < 0 {
            set_errno_from_ret(ret);
            -1
        } else {
            ret as c_int
        }
    }
    #[cfg(target_arch = "aarch64")]
    {
        let ret: i64;
        std::arch::asm!(
            "svc #0",
            in("x8") 291i64, // SYS_statx
            in("x0") dirfd as i64,
            in("x1") path,
            in("x2") flags as i64,
            in("x3") mask as i64,
            in("x4") buf,
            lateout("x0") ret,
        );
        if ret < 0 {
            set_errno_from_ret(ret);
            -1
        } else {
            ret as c_int
        }
    }
}

// =============================================================================
// FD Operations
// =============================================================================

/// Raw dup syscall
#[inline(always)]
pub unsafe fn raw_dup(oldfd: c_int) -> c_int {
    #[cfg(target_arch = "x86_64")]
    {
        let ret: i64;
        std::arch::asm!(
            "syscall",
            in("rax") 32i64, // SYS_dup
            in("rdi") oldfd as i64,
            lateout("rax") ret,
            lateout("rcx") _,
            lateout("r11") _,
        );
        if ret < 0 {
            set_errno_from_ret(ret);
            -1
        } else {
            ret as c_int
        }
    }
    #[cfg(target_arch = "aarch64")]
    {
        let ret: i64;
        std::arch::asm!(
            "svc #0",
            in("x8") 23i64, // SYS_dup
            in("x0") oldfd as i64,
            lateout("x0") ret,
        );
        if ret < 0 {
            set_errno_from_ret(ret);
            -1
        } else {
            ret as c_int
        }
    }
}

/// Raw dup2 syscall (via dup3 on AArch64)
#[inline(always)]
pub unsafe fn raw_dup2(oldfd: c_int, newfd: c_int) -> c_int {
    #[cfg(target_arch = "x86_64")]
    {
        let ret: i64;
        std::arch::asm!(
            "syscall",
            in("rax") 33i64, // SYS_dup2
            in("rdi") oldfd as i64,
            in("rsi") newfd as i64,
            lateout("rax") ret,
            lateout("rcx") _,
            lateout("r11") _,
        );
        if ret < 0 {
            set_errno_from_ret(ret);
            -1
        } else {
            ret as c_int
        }
    }
    #[cfg(target_arch = "aarch64")]
    {
        // AArch64 uses dup3 with 0 flags
        let ret: i64;
        std::arch::asm!(
            "svc #0",
            in("x8") 24i64, // SYS_dup3
            in("x0") oldfd as i64,
            in("x1") newfd as i64,
            in("x2") 0i64, // flags
            lateout("x0") ret,
        );
        if ret < 0 {
            set_errno_from_ret(ret);
            -1
        } else {
            ret as c_int
        }
    }
}

/// Raw lseek syscall
#[inline(always)]
pub unsafe fn raw_lseek(fd: c_int, offset: off_t, whence: c_int) -> off_t {
    #[cfg(target_arch = "x86_64")]
    {
        let ret: i64;
        std::arch::asm!(
            "syscall",
            in("rax") 8i64, // SYS_lseek
            in("rdi") fd as i64,
            in("rsi") offset,
            in("rdx") whence as i64,
            lateout("rax") ret,
            lateout("rcx") _,
            lateout("r11") _,
        );
        if ret < 0 {
            set_errno_from_ret(ret);
            -1
        } else {
            ret as off_t
        }
    }
    #[cfg(target_arch = "aarch64")]
    {
        let ret: i64;
        std::arch::asm!(
            "svc #0",
            in("x8") 62i64, // SYS_lseek
            in("x0") fd as i64,
            in("x1") offset,
            in("x2") whence as i64,
            lateout("x0") ret,
        );
        if ret < 0 {
            set_errno_from_ret(ret);
            -1
        } else {
            ret as off_t
        }
    }
}

/// Raw ftruncate syscall
#[inline(always)]
pub unsafe fn raw_ftruncate(fd: c_int, length: off_t) -> c_int {
    #[cfg(target_arch = "x86_64")]
    {
        let ret: i64;
        std::arch::asm!(
            "syscall",
            in("rax") 77i64, // SYS_ftruncate
            in("rdi") fd as i64,
            in("rsi") length,
            lateout("rax") ret,
            lateout("rcx") _,
            lateout("r11") _,
        );
        if ret < 0 {
            set_errno_from_ret(ret);
            -1
        } else {
            ret as c_int
        }
    }
    #[cfg(target_arch = "aarch64")]
    {
        let ret: i64;
        std::arch::asm!(
            "svc #0",
            in("x8") 46i64, // SYS_ftruncate
            in("x0") fd as i64,
            in("x1") length,
            lateout("x0") ret,
        );
        if ret < 0 {
            set_errno_from_ret(ret);
            -1
        } else {
            ret as c_int
        }
    }
}

/// Raw access syscall (via faccessat)
#[inline(always)]
pub unsafe fn raw_access(path: *const c_char, mode: c_int) -> c_int {
    #[cfg(target_arch = "x86_64")]
    {
        let ret: i64;
        std::arch::asm!(
            "syscall",
            in("rax") 21i64, // SYS_access
            in("rdi") path,
            in("rsi") mode as i64,
            lateout("rax") ret,
            lateout("rcx") _,
            lateout("r11") _,
        );
        if ret < 0 {
            set_errno_from_ret(ret);
            -1
        } else {
            ret as c_int
        }
    }
    #[cfg(target_arch = "aarch64")]
    {
        // AArch64 uses faccessat
        let ret: i64;
        std::arch::asm!(
            "svc #0",
            in("x8") 48i64, // SYS_faccessat
            in("x0") -100i64, // AT_FDCWD
            in("x1") path,
            in("x2") mode as i64,
            in("x3") 0i64, // no flags
            lateout("x0") ret,
        );
        if ret < 0 {
            set_errno_from_ret(ret);
            -1
        } else {
            ret as c_int
        }
    }
}

/// Raw faccessat syscall
#[inline(always)]
pub unsafe fn raw_faccessat(dirfd: c_int, path: *const c_char, mode: c_int, flags: c_int) -> c_int {
    #[cfg(target_arch = "x86_64")]
    {
        let ret: i64;
        std::arch::asm!(
            "syscall",
            in("rax") 269i64, // SYS_faccessat
            in("rdi") dirfd as i64,
            in("rsi") path,
            in("rdx") mode as i64,
            in("r10") flags as i64,
            lateout("rax") ret,
            lateout("rcx") _,
            lateout("r11") _,
        );
        if ret < 0 {
            set_errno_from_ret(ret);
            -1
        } else {
            ret as c_int
        }
    }
    #[cfg(target_arch = "aarch64")]
    {
        let ret: i64;
        std::arch::asm!(
            "svc #0",
            in("x8") 48i64, // SYS_faccessat
            in("x0") dirfd as i64,
            in("x1") path,
            in("x2") mode as i64,
            in("x3") flags as i64,
            lateout("x0") ret,
        );
        if ret < 0 {
            set_errno_from_ret(ret);
            -1
        } else {
            ret as c_int
        }
    }
}

/// Raw rename syscall
#[inline(always)]
pub unsafe fn raw_rename(old: *const c_char, new: *const c_char) -> c_int {
    #[cfg(target_arch = "x86_64")]
    {
        let ret: i64;
        std::arch::asm!(
            "syscall",
            in("rax") 82i64, // SYS_rename
            in("rdi") old,
            in("rsi") new,
            lateout("rax") ret,
            lateout("rcx") _,
            lateout("r11") _,
        );
        if ret < 0 {
            set_errno_from_ret(ret);
            -1
        } else {
            ret as c_int
        }
    }
    #[cfg(target_arch = "aarch64")]
    {
        // AArch64 uses renameat
        let ret: i64;
        std::arch::asm!(
            "svc #0",
            in("x8") 38i64, // SYS_renameat
            in("x0") -100i64, // AT_FDCWD
            in("x1") old,
            in("x2") -100i64, // AT_FDCWD
            in("x3") new,
            lateout("x0") ret,
        );
        if ret < 0 {
            set_errno_from_ret(ret);
            -1
        } else {
            ret as c_int
        }
    }
}

// =============================================================================
// Memory Mapping Operations (for BUG-007 cross-platform support)
// =============================================================================

/// Raw mmap syscall
#[inline(always)]
pub unsafe fn raw_mmap(
    addr: *mut c_void,
    len: size_t,
    prot: c_int,
    flags: c_int,
    fd: c_int,
    offset: off_t,
) -> *mut c_void {
    #[cfg(target_arch = "x86_64")]
    {
        let ret: i64;
        std::arch::asm!(
            "syscall",
            in("rax") 9i64, // SYS_mmap
            in("rdi") addr,
            in("rsi") len as i64,
            in("rdx") prot as i64,
            in("r10") flags as i64,
            in("r8") fd as i64,
            in("r9") offset,
            lateout("rax") ret,
            lateout("rcx") _,
            lateout("r11") _,
        );
        ret as *mut c_void
    }
    #[cfg(target_arch = "aarch64")]
    {
        let ret: i64;
        std::arch::asm!(
            "svc #0",
            in("x8") 222i64, // SYS_mmap
            in("x0") addr,
            in("x1") len as i64,
            in("x2") prot as i64,
            in("x3") flags as i64,
            in("x4") fd as i64,
            in("x5") offset,
            lateout("x0") ret,
        );
        ret as *mut c_void
    }
}

/// Raw munmap syscall
#[inline(always)]
pub unsafe fn raw_munmap(addr: *mut c_void, len: size_t) -> c_int {
    #[cfg(target_arch = "x86_64")]
    {
        let ret: i64;
        std::arch::asm!(
            "syscall",
            in("rax") 11i64, // SYS_munmap
            in("rdi") addr,
            in("rsi") len as i64,
            lateout("rax") ret,
            lateout("rcx") _,
            lateout("r11") _,
        );
        if ret < 0 {
            set_errno_from_ret(ret);
            -1
        } else {
            ret as c_int
        }
    }
    #[cfg(target_arch = "aarch64")]
    {
        let ret: i64;
        std::arch::asm!(
            "svc #0",
            in("x8") 215i64, // SYS_munmap
            in("x0") addr,
            in("x1") len as i64,
            lateout("x0") ret,
        );
        if ret < 0 {
            set_errno_from_ret(ret);
            -1
        } else {
            ret as c_int
        }
    }
}

// =============================================================================
// Path Operations (for P2 Linux inception layer parity)
// =============================================================================

/// Raw readlink syscall
#[inline(always)]
pub unsafe fn raw_readlink(path: *const c_char, buf: *mut c_char, bufsiz: size_t) -> ssize_t {
    #[cfg(target_arch = "x86_64")]
    {
        let ret: i64;
        std::arch::asm!(
            "syscall",
            in("rax") 89i64, // SYS_readlink
            in("rdi") path,
            in("rsi") buf,
            in("rdx") bufsiz as i64,
            lateout("rax") ret,
            lateout("rcx") _,
            lateout("r11") _,
        );
        if ret < 0 {
            set_errno_from_ret(ret);
            -1
        } else {
            ret as ssize_t
        }
    }
    #[cfg(target_arch = "aarch64")]
    {
        // AArch64 uses readlinkat
        let ret: i64;
        std::arch::asm!(
            "svc #0",
            in("x8") 78i64, // SYS_readlinkat
            in("x0") -100i64, // AT_FDCWD
            in("x1") path,
            in("x2") buf,
            in("x3") bufsiz as i64,
            lateout("x0") ret,
        );
        if ret < 0 {
            set_errno_from_ret(ret);
            -1
        } else {
            ret as ssize_t
        }
    }
}

/// Raw getcwd syscall
#[inline(always)]
pub unsafe fn raw_getcwd(buf: *mut c_char, size: size_t) -> *mut c_char {
    #[cfg(target_arch = "x86_64")]
    {
        let ret: i64;
        std::arch::asm!(
            "syscall",
            in("rax") 79i64, // SYS_getcwd
            in("rdi") buf,
            in("rsi") size as i64,
            lateout("rax") ret,
            lateout("rcx") _,
            lateout("r11") _,
        );
        if ret < 0 {
            set_errno_from_ret(ret);
            std::ptr::null_mut()
        } else {
            buf
        }
    }
    #[cfg(target_arch = "aarch64")]
    {
        let ret: i64;
        std::arch::asm!(
            "svc #0",
            in("x8") 17i64, // SYS_getcwd
            in("x0") buf,
            in("x1") size as i64,
            lateout("x0") ret,
        );
        if ret < 0 {
            set_errno_from_ret(ret);
            std::ptr::null_mut()
        } else {
            buf
        }
    }
}

/// Raw chdir syscall
#[inline(always)]
pub unsafe fn raw_chdir(path: *const c_char) -> c_int {
    #[cfg(target_arch = "x86_64")]
    {
        let ret: i64;
        std::arch::asm!(
            "syscall",
            in("rax") 80i64, // SYS_chdir
            in("rdi") path,
            lateout("rax") ret,
            lateout("rcx") _,
            lateout("r11") _,
        );
        if ret < 0 {
            set_errno_from_ret(ret);
            -1
        } else {
            ret as c_int
        }
    }
    #[cfg(target_arch = "aarch64")]
    {
        let ret: i64;
        std::arch::asm!(
            "svc #0",
            in("x8") 49i64, // SYS_chdir
            in("x0") path,
            lateout("x0") ret,
        );
        if ret < 0 {
            set_errno_from_ret(ret);
            -1
        } else {
            ret as c_int
        }
    }
}

/// Raw fchdir syscall
#[inline(always)]
pub unsafe fn raw_fchdir(fd: c_int) -> c_int {
    #[cfg(target_arch = "x86_64")]
    {
        let ret: i64;
        std::arch::asm!(
            "syscall",
            in("rax") 81i64, // SYS_fchdir
            in("rdi") fd as i64,
            lateout("rax") ret,
            lateout("rcx") _,
            lateout("r11") _,
        );
        if ret < 0 {
            set_errno_from_ret(ret);
            -1
        } else {
            ret as c_int
        }
    }
    #[cfg(target_arch = "aarch64")]
    {
        let ret: i64;
        std::arch::asm!(
            "svc #0",
            in("x8") 50i64, // SYS_fchdir
            in("x0") fd as i64,
            lateout("x0") ret,
        );
        if ret < 0 {
            set_errno_from_ret(ret);
            -1
        } else {
            ret as c_int
        }
    }
}

// =============================================================================
// Ownership Operations (P0-P1 Gap Fix)
// =============================================================================

/// Raw fchown syscall - change file ownership via FD
#[inline(always)]
pub unsafe fn raw_fchown(fd: c_int, owner: libc::uid_t, group: libc::gid_t) -> c_int {
    #[cfg(target_arch = "x86_64")]
    {
        let ret: i64;
        std::arch::asm!(
            "syscall",
            in("rax") 93i64, // SYS_fchown
            in("rdi") fd as i64,
            in("rsi") owner as i64,
            in("rdx") group as i64,
            lateout("rax") ret,
            lateout("rcx") _,
            lateout("r11") _,
        );
        if ret < 0 {
            set_errno_from_ret(ret);
            -1
        } else {
            ret as c_int
        }
    }
    #[cfg(target_arch = "aarch64")]
    {
        let ret: i64;
        std::arch::asm!(
            "svc #0",
            in("x8") 55i64, // SYS_fchown
            in("x0") fd as i64,
            in("x1") owner as i64,
            in("x2") group as i64,
            lateout("x0") ret,
        );
        if ret < 0 {
            set_errno_from_ret(ret);
            -1
        } else {
            ret as c_int
        }
    }
}

/// Raw fchownat syscall - change file ownership via dirfd + path
#[inline(always)]
pub unsafe fn raw_fchownat(
    dirfd: c_int,
    path: *const c_char,
    owner: libc::uid_t,
    group: libc::gid_t,
    flags: c_int,
) -> c_int {
    #[cfg(target_arch = "x86_64")]
    {
        let ret: i64;
        std::arch::asm!(
            "syscall",
            in("rax") 260i64, // SYS_fchownat
            in("rdi") dirfd as i64,
            in("rsi") path,
            in("rdx") owner as i64,
            in("r10") group as i64,
            in("r8") flags as i64,
            lateout("rax") ret,
            lateout("rcx") _,
            lateout("r11") _,
        );
        if ret < 0 {
            set_errno_from_ret(ret);
            -1
        } else {
            ret as c_int
        }
    }
    #[cfg(target_arch = "aarch64")]
    {
        let ret: i64;
        std::arch::asm!(
            "svc #0",
            in("x8") 54i64, // SYS_fchownat
            in("x0") dirfd as i64,
            in("x1") path,
            in("x2") owner as i64,
            in("x3") group as i64,
            in("x4") flags as i64,
            lateout("x0") ret,
        );
        if ret < 0 {
            set_errno_from_ret(ret);
            -1
        } else {
            ret as c_int
        }
    }
}

/// Raw chown syscall - change file ownership by path (follows symlinks)
/// Uses SYS_chown on x86_64, fchownat(AT_FDCWD, ..., 0) on aarch64
#[inline(always)]
pub unsafe fn raw_chown(path: *const c_char, owner: libc::uid_t, group: libc::gid_t) -> c_int {
    #[cfg(target_arch = "x86_64")]
    {
        let ret: i64;
        std::arch::asm!(
            "syscall",
            in("rax") 92i64, // SYS_chown
            in("rdi") path,
            in("rsi") owner as i64,
            in("rdx") group as i64,
            lateout("rax") ret,
            lateout("rcx") _,
            lateout("r11") _,
        );
        if ret < 0 {
            set_errno_from_ret(ret);
            -1
        } else {
            ret as c_int
        }
    }
    #[cfg(target_arch = "aarch64")]
    {
        // AArch64 has no direct chown — use fchownat(AT_FDCWD, path, ..., 0)
        raw_fchownat(libc::AT_FDCWD, path, owner, group, 0)
    }
}

/// Raw lchown syscall - change symlink ownership (does NOT follow symlinks)
/// Uses SYS_lchown on x86_64, fchownat(AT_FDCWD, ..., AT_SYMLINK_NOFOLLOW) on aarch64
#[inline(always)]
pub unsafe fn raw_lchown(path: *const c_char, owner: libc::uid_t, group: libc::gid_t) -> c_int {
    #[cfg(target_arch = "x86_64")]
    {
        let ret: i64;
        std::arch::asm!(
            "syscall",
            in("rax") 94i64, // SYS_lchown
            in("rdi") path,
            in("rsi") owner as i64,
            in("rdx") group as i64,
            lateout("rax") ret,
            lateout("rcx") _,
            lateout("r11") _,
        );
        if ret < 0 {
            set_errno_from_ret(ret);
            -1
        } else {
            ret as c_int
        }
    }
    #[cfg(target_arch = "aarch64")]
    {
        // AArch64 has no direct lchown — use fchownat with AT_SYMLINK_NOFOLLOW
        raw_fchownat(
            libc::AT_FDCWD,
            path,
            owner,
            group,
            libc::AT_SYMLINK_NOFOLLOW,
        )
    }
}

/// Raw readlinkat syscall - read symlink target via dirfd + path
#[inline(always)]
pub unsafe fn raw_readlinkat(
    dirfd: c_int,
    path: *const c_char,
    buf: *mut c_char,
    bufsiz: size_t,
) -> ssize_t {
    #[cfg(target_arch = "x86_64")]
    {
        let ret: i64;
        std::arch::asm!(
            "syscall",
            in("rax") 267i64, // SYS_readlinkat
            in("rdi") dirfd as i64,
            in("rsi") path,
            in("rdx") buf,
            in("r10") bufsiz as i64,
            lateout("rax") ret,
            lateout("rcx") _,
            lateout("r11") _,
        );
        if ret < 0 {
            set_errno_from_ret(ret);
            -1
        } else {
            ret as ssize_t
        }
    }
    #[cfg(target_arch = "aarch64")]
    {
        let ret: i64;
        std::arch::asm!(
            "svc #0",
            in("x8") 78i64, // SYS_readlinkat
            in("x0") dirfd as i64,
            in("x1") path,
            in("x2") buf,
            in("x3") bufsiz as i64,
            lateout("x0") ret,
        );
        if ret < 0 {
            set_errno_from_ret(ret);
            -1
        } else {
            ret as ssize_t
        }
    }
}

/// Raw futimes syscall - change file timestamps via FD
/// Linux has no dedicated futimes syscall; uses utimensat(fd, NULL, ...) instead
#[inline(always)]
pub unsafe fn raw_futimes(fd: c_int, times: *const libc::timeval) -> c_int {
    if times.is_null() {
        raw_utimensat(fd, std::ptr::null(), std::ptr::null(), 0)
    } else {
        let times_array = std::slice::from_raw_parts(times, 2);
        let ts = [
            libc::timespec {
                tv_sec: times_array[0].tv_sec,
                tv_nsec: times_array[0].tv_usec * 1000,
            },
            libc::timespec {
                tv_sec: times_array[1].tv_sec,
                tv_nsec: times_array[1].tv_usec * 1000,
            },
        ];
        raw_utimensat(fd, std::ptr::null(), ts.as_ptr(), 0)
    }
}

/// Raw truncate syscall
#[inline(always)]
pub unsafe fn raw_truncate(path: *const c_char, length: off_t) -> c_int {
    #[cfg(target_arch = "x86_64")]
    {
        let ret: i64;
        std::arch::asm!(
            "syscall",
            in("rax") 76i64, // SYS_truncate
            in("rdi") path,
            in("rsi") length,
            lateout("rax") ret,
            lateout("rcx") _,
            lateout("r11") _,
        );
        if ret < 0 {
            set_errno_from_ret(ret);
            -1
        } else {
            ret as c_int
        }
    }
    #[cfg(target_arch = "aarch64")]
    {
        let ret: i64;
        std::arch::asm!(
            "svc #0",
            in("x8") 45i64, // SYS_truncate
            in("x0") path,
            in("x1") length,
            lateout("x0") ret,
        );
        if ret < 0 {
            set_errno_from_ret(ret);
            -1
        } else {
            ret as c_int
        }
    }
}

/// Raw sendfile syscall
#[inline(always)]
pub unsafe fn raw_sendfile(
    out_fd: c_int,
    in_fd: c_int,
    offset: *mut off_t,
    count: size_t,
) -> ssize_t {
    #[cfg(target_arch = "x86_64")]
    {
        let ret: i64;
        std::arch::asm!(
            "syscall",
            in("rax") 40i64, // SYS_sendfile
            in("rdi") out_fd as i64,
            in("rsi") in_fd as i64,
            in("rdx") offset,
            in("r10") count as i64,
            lateout("rax") ret,
            lateout("rcx") _,
            lateout("r11") _,
        );
        if ret < 0 {
            set_errno_from_ret(ret);
            -1
        } else {
            ret as ssize_t
        }
    }
    #[cfg(target_arch = "aarch64")]
    {
        let ret: i64;
        std::arch::asm!(
            "svc #0",
            in("x8") 71i64, // SYS_sendfile
            in("x0") out_fd as i64,
            in("x1") in_fd as i64,
            in("x2") offset,
            in("x3") count as i64,
            lateout("x0") ret,
        );
        if ret < 0 {
            set_errno_from_ret(ret);
            -1
        } else {
            ret as ssize_t
        }
    }
}

/// Raw copy_file_range syscall
#[inline(always)]
pub unsafe fn raw_copy_file_range(
    fd_in: c_int,
    off_in: *mut off_t,
    fd_out: c_int,
    off_out: *mut off_t,
    len: size_t,
    flags: c_uint,
) -> ssize_t {
    #[cfg(target_arch = "x86_64")]
    {
        let ret: i64;
        std::arch::asm!(
            "syscall",
            in("rax") 326i64, // SYS_copy_file_range
            in("rdi") fd_in as i64,
            in("rsi") off_in,
            in("rdx") fd_out as i64,
            in("r10") off_out,
            in("r8") len as i64,
            in("r9") flags as i64,
            lateout("rax") ret,
            lateout("rcx") _,
            lateout("r11") _,
        );
        if ret < 0 {
            set_errno_from_ret(ret);
            -1
        } else {
            ret as ssize_t
        }
    }
    #[cfg(target_arch = "aarch64")]
    {
        let ret: i64;
        std::arch::asm!(
            "svc #0",
            in("x8") 285i64, // SYS_copy_file_range
            in("x0") fd_in as i64,
            in("x1") off_in,
            in("x2") fd_out as i64,
            in("x3") off_out,
            in("x4") len as i64,
            in("x5") flags as i64,
            lateout("x0") ret,
        );
        if ret < 0 {
            set_errno_from_ret(ret);
            -1
        } else {
            ret as ssize_t
        }
    }
}

/// Raw openat2 syscall
#[inline(always)]
pub unsafe fn raw_openat2(
    dirfd: c_int,
    pathname: *const c_char,
    how: *const c_void, // struct open_how
    size: size_t,
) -> c_int {
    #[cfg(target_arch = "x86_64")]
    {
        let ret: i64;
        std::arch::asm!(
            "syscall",
            in("rax") 437i64, // SYS_openat2
            in("rdi") dirfd as i64,
            in("rsi") pathname,
            in("rdx") how,
            in("r10") size as i64,
            lateout("rax") ret,
            lateout("rcx") _,
            lateout("r11") _,
        );
        if ret < 0 {
            set_errno_from_ret(ret);
            -1
        } else {
            ret as c_int
        }
    }
    #[cfg(target_arch = "aarch64")]
    {
        let ret: i64;
        std::arch::asm!(
            "svc #0",
            in("x8") 437i64, // SYS_openat2
            in("x0") dirfd as i64,
            in("x1") pathname,
            in("x2") how,
            in("x3") size as i64,
            lateout("x0") ret,
        );
        if ret < 0 {
            set_errno_from_ret(ret);
            -1
        } else {
            ret as c_int
        }
    }
}

/// Raw realpath (via libc::realpath)
#[inline(always)]
pub unsafe fn raw_realpath(path: *const c_char, resolved: *mut c_char) -> *mut c_char {
    libc::realpath(path, resolved)
}

use libc::c_uint;
