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
// Path Operations (for P2 Linux shim parity)
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
