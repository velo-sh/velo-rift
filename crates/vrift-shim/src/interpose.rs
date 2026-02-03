//! Syscall interposition table for macOS/Linux shim.
//! Safety: All extern "C" functions here are dangerous FFI and must be used correctly.
#![allow(clippy::missing_safety_doc)]

#[cfg(target_os = "macos")]
use crate::syscalls::dir::{chdir_shim, closedir_shim, getcwd_shim, opendir_shim, readdir_shim};
#[cfg(target_os = "macos")]
use crate::syscalls::io::{
    close_shim, dup2_shim, dup_shim, fchdir_shim, ftruncate_shim, lseek_shim, read_shim, write_shim,
};
#[cfg(target_os = "macos")]
use crate::syscalls::misc::{
    chflags_shim, chmod_shim, fchmodat_shim, link_shim, linkat_shim, mkdir_shim, removexattr_shim,
    rename_shim, renameat_shim, rmdir_shim, setxattr_shim, truncate_shim, unlink_shim,
    utimensat_shim, utimes_shim,
};
#[cfg(target_os = "macos")]
use crate::syscalls::mmap::{mmap_shim, munmap_shim};
// C variadic wrappers call velo_*_impl which is defined in syscalls/open.rs
#[cfg(target_os = "macos")]
use crate::syscalls::open::{open_shim, openat_shim};
#[cfg(target_os = "macos")]
use crate::syscalls::path::{readlink_shim, realpath_shim};
#[cfg(target_os = "macos")]
use crate::syscalls::stat::{access_shim, fstat_shim, fstatat_shim, lstat_shim, stat_shim};

use libc::{c_char, c_int, c_long, mode_t};

#[cfg(target_os = "macos")]
use libc::{c_void, dirent, pid_t, size_t, ssize_t, timespec, timeval, DIR};
#[cfg(target_os = "macos")]
#[repr(C)]
pub struct Interpose {
    pub new_func: *const (),
    pub old_func: *const (),
}

#[cfg(target_os = "macos")]
unsafe impl Sync for Interpose {}

#[cfg(target_os = "macos")]
extern "C" {
    fn open(path: *const c_char, flags: c_int, mode: mode_t) -> c_int;
    fn close(fd: c_int) -> c_int;
    fn write(fd: c_int, buf: *const c_void, count: size_t) -> ssize_t;
    fn read(fd: c_int, buf: *mut c_void, count: size_t) -> ssize_t;
    fn stat(path: *const c_char, buf: *mut libc::stat) -> c_int;
    fn lstat(path: *const c_char, buf: *mut libc::stat) -> c_int;
    fn fstat(fd: c_int, buf: *mut libc::stat) -> c_int;
    fn opendir(path: *const c_char) -> *mut DIR;
    fn readdir(dirp: *mut DIR) -> *mut dirent;
    fn closedir(dirp: *mut DIR) -> c_int;
    fn readlink(path: *const c_char, buf: *mut c_char, bufsiz: size_t) -> ssize_t;
    fn execve(path: *const c_char, argv: *const *const c_char, envp: *const *const c_char)
        -> c_int;
    fn posix_spawn(
        pid: *mut pid_t,
        path: *const c_char,
        fa: *const c_void,
        attr: *const c_void,
        argv: *const *const c_char,
        envp: *const *const c_char,
    ) -> c_int;
    fn posix_spawnp(
        pid: *mut pid_t,
        file: *const c_char,
        fa: *const c_void,
        attr: *const c_void,
        argv: *const *const c_char,
        envp: *const *const c_char,
    ) -> c_int;
    fn realpath(path: *const c_char, resolved: *mut c_char) -> *mut c_char;
    #[link_name = "realpath$DARWIN_EXTSN"]
    fn realpath_darwin(path: *const c_char, resolved: *mut c_char) -> *mut c_char;
    fn getcwd(buf: *mut c_char, size: size_t) -> *mut c_char;
    fn chdir(path: *const c_char) -> c_int;
    fn unlink(path: *const c_char) -> c_int;
    fn rename(old: *const c_char, new: *const c_char) -> c_int;
    fn rmdir(path: *const c_char) -> c_int;
    fn dlopen(path: *const c_char, flags: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    fn access(path: *const c_char, mode: c_int) -> c_int;
    fn faccessat(dirfd: c_int, path: *const c_char, mode: c_int, flags: c_int) -> c_int;
    fn openat(dirfd: c_int, path: *const c_char, flags: c_int, mode: mode_t) -> c_int;
    fn link(old: *const c_char, new: *const c_char) -> c_int;
    fn linkat(fd1: c_int, p1: *const c_char, fd2: c_int, p2: *const c_char, flags: c_int) -> c_int;
    fn renameat(fd1: c_int, p1: *const c_char, fd2: c_int, p2: *const c_char) -> c_int;
    fn symlink(p1: *const c_char, p2: *const c_char) -> c_int;
    fn flock(fd: c_int, op: c_int) -> c_int;
    fn utimensat(dirfd: c_int, path: *const c_char, times: *const timespec, flags: c_int) -> c_int;
    fn mkdir(path: *const c_char, mode: mode_t) -> c_int;
    fn munmap(addr: *mut c_void, len: size_t) -> c_int;
    fn fcntl(fd: c_int, cmd: c_int, ...) -> c_int;
    fn fstatat(dirfd: c_int, path: *const c_char, buf: *mut libc::stat, flags: c_int) -> c_int;
    // Mutation perimeter syscalls
    fn chmod(path: *const c_char, mode: mode_t) -> c_int;
    fn fchmodat(dirfd: c_int, path: *const c_char, mode: mode_t, flags: c_int) -> c_int;
    fn truncate(path: *const c_char, length: libc::off_t) -> c_int;
    fn chflags(path: *const c_char, flags: libc::c_uint) -> c_int;
    fn setxattr(
        path: *const c_char,
        name: *const c_char,
        value: *const c_void,
        size: size_t,
        position: u32,
        options: c_int,
    ) -> c_int;
    fn removexattr(path: *const c_char, name: *const c_char, options: c_int) -> c_int;
    fn utimes(path: *const c_char, times: *const timeval) -> c_int;
    // FD tracking syscalls
    fn dup(oldfd: c_int) -> c_int;
    fn dup2(oldfd: c_int, newfd: c_int) -> c_int;
    fn fchdir(fd: c_int) -> c_int;
    fn lseek(fd: c_int, offset: libc::off_t, whence: c_int) -> libc::off_t;
    fn ftruncate(fd: c_int, length: libc::off_t) -> c_int;
}

extern "C" {
    // Unified bridge implementations
    fn open_shim_c_impl(path: *const c_char, flags: c_int, mode: mode_t) -> c_int;
    fn openat_shim_c_impl(dirfd: c_int, path: *const c_char, flags: c_int, mode: mode_t) -> c_int;
    fn fcntl_shim_c_impl(fd: c_int, cmd: c_int, arg: c_long) -> c_int;
}

// RFC-0051: Linux Native Interception Points
// Rust defined points ensure 100% reliable symbol export on Linux.
#[cfg(target_os = "linux")]
mod linux_shims {
    use super::*;

    #[no_mangle]
    pub unsafe extern "C" fn open(path: *const c_char, flags: c_int, mode: mode_t) -> c_int {
        // RFC-0050: Only bypass when actually busy (state 3), allow state 2 to trigger initialization
        let init_state =
            unsafe { crate::state::INITIALIZING.load(std::sync::atomic::Ordering::Relaxed) };
        if init_state == 2 || init_state == 3 {
            #[cfg(target_arch = "x86_64")]
            {
                let ret: i64;
                std::arch::asm!("syscall", in("rax") 2, in("rdi") path, in("rsi") flags as i64, in("rdx") mode as i64, lateout("rax") ret);
                if ret < 0 {
                    crate::set_errno(-ret as c_int);
                    return -1;
                }
                return ret as c_int;
            }
            #[cfg(target_arch = "aarch64")]
            {
                let ret: i64;
                std::arch::asm!(
                    "svc #0",
                    in("x8") 56i64, // openat
                    in("x0") -100i64, // AT_FDCWD
                    in("x1") path,
                    in("x2") flags as i64,
                    in("x3") mode as i64,
                    lateout("x0") ret,
                );
                if ret < 0 {
                    crate::set_errno(-ret as c_int);
                    return -1;
                }
                return ret as c_int;
            }
        }
        let _guard = match crate::state::ShimGuard::enter() {
            Some(g) => g,
            None => {
                #[cfg(target_arch = "x86_64")]
                {
                    let ret: i64;
                    std::arch::asm!("syscall", in("rax") 2, in("rdi") path, in("rsi") flags as i64, in("rdx") mode as i64, lateout("rax") ret);
                    if ret < 0 {
                        crate::set_errno(-ret as c_int);
                        return -1;
                    }
                    return ret as c_int;
                }
                #[cfg(target_arch = "aarch64")]
                {
                    let ret: i64;
                    std::arch::asm!(
                        "svc #0",
                        in("x8") 56i64, // openat
                        in("x0") -100i64, // AT_FDCWD
                        in("x1") path,
                        in("x2") flags as i64,
                        in("x3") mode as i64,
                        lateout("x0") ret,
                    );
                    if ret < 0 {
                        crate::set_errno(-ret as c_int);
                        return -1;
                    }
                    return ret as c_int;
                }
            }
        };
        crate::syscalls::open::velo_open_impl(path, flags, mode)
    }

    #[no_mangle]
    pub unsafe extern "C" fn open64(path: *const c_char, flags: c_int, mode: mode_t) -> c_int {
        open(path, flags, mode)
    }

    #[no_mangle]
    pub unsafe extern "C" fn newfstatat(
        dirfd: c_int,
        path: *const c_char,
        buf: *mut libc::stat,
        flags: c_int,
    ) -> c_int {
        // Double-guard: check INITIALIZING first for early-init safety
        let init_state =
            unsafe { crate::state::INITIALIZING.load(std::sync::atomic::Ordering::Relaxed) };
        if init_state == 2 || init_state == 3 {
            #[cfg(target_arch = "x86_64")]
            {
                let ret: i64;
                std::arch::asm!("syscall", in("rax") 262, in("rdi") dirfd as i64, in("rsi") path, in("rdx") buf, in("r10") flags as i64, lateout("rax") ret);
                if ret < 0 {
                    crate::set_errno(-ret as c_int);
                    return -1;
                }
                return ret as c_int;
            }
            #[cfg(target_arch = "aarch64")]
            {
                let ret: i64;
                std::arch::asm!(
                    "svc #0",
                    in("x8") 79i64, // fstatat
                    in("x0") dirfd as i64,
                    in("x1") path,
                    in("x2") buf,
                    in("x3") flags as i64,
                    lateout("x0") ret,
                );
                if ret < 0 {
                    crate::set_errno(-ret as c_int);
                    return -1;
                }
                return ret as c_int;
            }
        }
        let _guard = match crate::state::ShimGuard::enter() {
            Some(g) => g,
            None => {
                #[cfg(target_arch = "x86_64")]
                {
                    let ret: i64;
                    std::arch::asm!("syscall", in("rax") 262, in("rdi") dirfd as i64, in("rsi") path, in("rdx") buf, in("r10") flags as i64, lateout("rax") ret);
                    if ret < 0 {
                        crate::set_errno(-ret as c_int);
                        return -1;
                    }
                    return ret as c_int;
                }
                #[cfg(target_arch = "aarch64")]
                {
                    let ret: i64;
                    std::arch::asm!(
                        "svc #0",
                        in("x8") 79i64, // fstatat
                        in("x0") dirfd as i64,
                        in("x1") path,
                        in("x2") buf,
                        in("x3") flags as i64,
                        lateout("x0") ret,
                    );
                    if ret < 0 {
                        crate::set_errno(-ret as c_int);
                        return -1;
                    }
                    return ret as c_int;
                }
            }
        };
        crate::syscalls::stat::fstatat_shim_linux(dirfd, path, buf, flags)
    }

    #[no_mangle]
    pub unsafe extern "C" fn openat(
        dirfd: c_int,
        path: *const c_char,
        flags: c_int,
        mode: mode_t,
    ) -> c_int {
        // Double-guard: check INITIALIZING first for early-init safety
        let init_state =
            unsafe { crate::state::INITIALIZING.load(std::sync::atomic::Ordering::Relaxed) };
        if init_state == 2 || init_state == 3 {
            #[cfg(target_arch = "x86_64")]
            {
                let ret: i64;
                std::arch::asm!("syscall", in("rax") 257, in("rdi") dirfd as i64, in("rsi") path, in("rdx") flags as i64, in("r10") mode as i64, lateout("rax") ret);
                if ret < 0 {
                    crate::set_errno(-ret as c_int);
                    return -1;
                }
                return ret as c_int;
            }
            #[cfg(target_arch = "aarch64")]
            {
                let ret: i64;
                std::arch::asm!(
                    "svc #0",
                    in("x8") 56i64, // openat
                    in("x0") dirfd as i64,
                    in("x1") path,
                    in("x2") flags as i64,
                    in("x3") mode as i64,
                    lateout("x0") ret,
                );
                if ret < 0 {
                    crate::set_errno(-ret as c_int);
                    return -1;
                }
                return ret as c_int;
            }
        }
        let _guard = match crate::state::ShimGuard::enter() {
            Some(g) => g,
            None => {
                #[cfg(target_arch = "x86_64")]
                {
                    let ret: i64;
                    std::arch::asm!("syscall", in("rax") 257, in("rdi") dirfd as i64, in("rsi") path, in("rdx") flags as i64, in("r10") mode as i64, lateout("rax") ret);
                    if ret < 0 {
                        crate::set_errno(-ret as c_int);
                        return -1;
                    }
                    return ret as c_int;
                }
                #[cfg(target_arch = "aarch64")]
                {
                    let ret: i64;
                    std::arch::asm!(
                        "svc #0",
                        in("x8") 56i64, // openat
                        in("x0") dirfd as i64,
                        in("x1") path,
                        in("x2") flags as i64,
                        in("x3") mode as i64,
                        lateout("x0") ret,
                    );
                    if ret < 0 {
                        crate::set_errno(-ret as c_int);
                        return -1;
                    }
                    return ret as c_int;
                }
            }
        };
        crate::syscalls::open::velo_openat_impl(dirfd, path, flags, mode)
    }

    #[no_mangle]
    pub unsafe extern "C" fn openat64(
        dirfd: c_int,
        path: *const c_char,
        flags: c_int,
        mode: mode_t,
    ) -> c_int {
        openat(dirfd, path, flags, mode)
    }

    #[no_mangle]
    pub unsafe extern "C" fn fcntl(fd: c_int, cmd: c_int, arg: c_long) -> c_int {
        // Double-guard: check INITIALIZING first for early-init safety
        let init_state =
            unsafe { crate::state::INITIALIZING.load(std::sync::atomic::Ordering::Relaxed) };
        if init_state == 2 || init_state == 3 {
            #[cfg(target_arch = "x86_64")]
            {
                let ret: i64;
                std::arch::asm!("syscall", in("rax") 72, in("rdi") fd as i64, in("rsi") cmd as i64, in("rdx") arg, lateout("rax") ret);
                if ret < 0 {
                    crate::set_errno(-ret as c_int);
                    return -1;
                }
                return ret as c_int;
            }
            #[cfg(target_arch = "aarch64")]
            {
                let ret: i64;
                std::arch::asm!(
                    "svc #0",
                    in("x8") 25i64, // fcntl
                    in("x0") fd as i64,
                    in("x1") cmd as i64,
                    in("x2") arg,
                    lateout("x0") ret,
                );
                if ret < 0 {
                    crate::set_errno(-ret as c_int);
                    return -1;
                }
                return ret as c_int;
            }
        }
        let _guard = match crate::state::ShimGuard::enter() {
            Some(g) => g,
            None => {
                #[cfg(target_arch = "x86_64")]
                {
                    let ret: i64;
                    std::arch::asm!("syscall", in("rax") 72, in("rdi") fd as i64, in("rsi") cmd as i64, in("rdx") arg, lateout("rax") ret);
                    if ret < 0 {
                        crate::set_errno(-ret as c_int);
                        return -1;
                    }
                    return ret as c_int;
                }
                #[cfg(target_arch = "aarch64")]
                {
                    let ret: i64;
                    std::arch::asm!(
                        "svc #0",
                        in("x8") 25i64, // fcntl
                        in("x0") fd as i64,
                        in("x1") cmd as i64,
                        in("x2") arg,
                        lateout("x0") ret,
                    );
                    if ret < 0 {
                        crate::set_errno(-ret as c_int);
                        return -1;
                    }
                    return ret as c_int;
                }
            }
        };
        velo_fcntl_impl(fd, cmd, arg)
    }

    #[no_mangle]
    pub unsafe extern "C" fn unlink(path: *const c_char) -> c_int {
        crate::syscalls::misc::unlink_shim(path)
    }

    #[no_mangle]
    pub unsafe extern "C" fn rmdir(path: *const c_char) -> c_int {
        crate::syscalls::misc::rmdir_shim(path)
    }

    #[no_mangle]
    pub unsafe extern "C" fn chmod(path: *const c_char, mode: mode_t) -> c_int {
        // Double-guard: check INITIALIZING first
        let init_state =
            unsafe { crate::state::INITIALIZING.load(std::sync::atomic::Ordering::Relaxed) };
        if init_state >= 2 {
            return crate::syscalls::open::raw_chmod(path, mode);
        }

        // Pass through if VFS mutation is NOT blocked
        if let Some(res) = crate::syscalls::misc::block_vfs_mutation(path) {
            return res;
        }
        crate::syscalls::open::raw_chmod(path, mode)
    }

    #[no_mangle]
    pub unsafe extern "C" fn fchmodat(
        dirfd: c_int,
        path: *const c_char,
        mode: mode_t,
        flags: c_int,
    ) -> c_int {
        let init_state =
            unsafe { crate::state::INITIALIZING.load(std::sync::atomic::Ordering::Relaxed) };
        if init_state >= 2 {
            return crate::syscalls::open::raw_fchmodat(dirfd, path, mode, flags);
        }

        if let Some(res) = crate::syscalls::misc::block_vfs_mutation(path) {
            return res;
        }
        crate::syscalls::open::raw_fchmodat(dirfd, path, mode, flags)
    }

    #[no_mangle]
    pub unsafe extern "C" fn mkdir(path: *const c_char, mode: mode_t) -> c_int {
        crate::syscalls::misc::mkdir_shim(path, mode)
    }

    #[no_mangle]
    pub unsafe extern "C" fn rename(old: *const c_char, new: *const c_char) -> c_int {
        let mut ret = crate::syscalls::misc::rename_shim_linux(old, new);
        if ret == -2 {
            // Need to call real rename
            #[cfg(target_arch = "x86_64")]
            {
                let r: i64;
                std::arch::asm!("syscall", in("rax") 82, in("rdi") old, in("rsi") new, lateout("rax") r);
                if r < 0 {
                    crate::set_errno(-r as c_int);
                    ret = -1;
                } else {
                    ret = r as c_int;
                }
            }
            #[cfg(target_arch = "aarch64")]
            {
                let r: i64;
                std::arch::asm!(
                    "svc #0",
                    in("x8") 38i64, // renameat
                    in("x0") -100i64, // AT_FDCWD
                    in("x1") old,
                    in("x2") -100i64, // AT_FDCWD
                    in("x3") new,
                    in("x4") 0,
                    lateout("x0") r,
                );
                if r < 0 {
                    crate::set_errno(-r as c_int);
                    ret = -1;
                } else {
                    ret = r as c_int;
                }
            }
        }
        ret
    }

    // ========================================================================
    // P0: Core I/O shims
    // ========================================================================

    #[no_mangle]
    pub unsafe extern "C" fn close(fd: c_int) -> c_int {
        use crate::syscalls::io::untrack_fd;

        let init_state = crate::state::INITIALIZING.load(std::sync::atomic::Ordering::Relaxed);
        if init_state >= 2 {
            return crate::syscalls::linux_raw::raw_close(fd);
        }

        let _guard = match crate::state::ShimGuard::enter() {
            Some(g) => g,
            None => return crate::syscalls::linux_raw::raw_close(fd),
        };

        untrack_fd(fd);
        crate::syscalls::linux_raw::raw_close(fd)
    }

    #[no_mangle]
    pub unsafe extern "C" fn read(
        fd: c_int,
        buf: *mut c_void,
        count: libc::size_t,
    ) -> libc::ssize_t {
        let init_state = crate::state::INITIALIZING.load(std::sync::atomic::Ordering::Relaxed);
        if init_state >= 2 {
            return crate::syscalls::linux_raw::raw_read(fd, buf, count);
        }
        crate::syscalls::linux_raw::raw_read(fd, buf, count)
    }

    #[no_mangle]
    pub unsafe extern "C" fn write(
        fd: c_int,
        buf: *const c_void,
        count: libc::size_t,
    ) -> libc::ssize_t {
        let init_state = crate::state::INITIALIZING.load(std::sync::atomic::Ordering::Relaxed);
        if init_state >= 2 {
            return crate::syscalls::linux_raw::raw_write(fd, buf, count);
        }
        crate::syscalls::linux_raw::raw_write(fd, buf, count)
    }

    #[no_mangle]
    pub unsafe extern "C" fn stat(path: *const c_char, buf: *mut libc::stat) -> c_int {
        let init_state = crate::state::INITIALIZING.load(std::sync::atomic::Ordering::Relaxed);
        if init_state >= 2 {
            return crate::syscalls::linux_raw::raw_stat(path, buf);
        }
        crate::syscalls::linux_raw::raw_stat(path, buf)
    }

    #[no_mangle]
    pub unsafe extern "C" fn stat64(path: *const c_char, buf: *mut libc::stat) -> c_int {
        stat(path, buf)
    }

    #[no_mangle]
    pub unsafe extern "C" fn lstat(path: *const c_char, buf: *mut libc::stat) -> c_int {
        let init_state = crate::state::INITIALIZING.load(std::sync::atomic::Ordering::Relaxed);
        if init_state >= 2 {
            return crate::syscalls::linux_raw::raw_lstat(path, buf);
        }
        crate::syscalls::linux_raw::raw_lstat(path, buf)
    }

    #[no_mangle]
    pub unsafe extern "C" fn lstat64(path: *const c_char, buf: *mut libc::stat) -> c_int {
        lstat(path, buf)
    }

    #[no_mangle]
    pub unsafe extern "C" fn fstat(fd: c_int, buf: *mut libc::stat) -> c_int {
        let init_state = crate::state::INITIALIZING.load(std::sync::atomic::Ordering::Relaxed);
        if init_state >= 2 {
            return crate::syscalls::linux_raw::raw_fstat(fd, buf);
        }
        crate::syscalls::linux_raw::raw_fstat(fd, buf)
    }

    #[no_mangle]
    pub unsafe extern "C" fn fstat64(fd: c_int, buf: *mut libc::stat) -> c_int {
        fstat(fd, buf)
    }

    // ========================================================================
    // P1: FD tracking shims
    // ========================================================================

    #[no_mangle]
    pub unsafe extern "C" fn dup(oldfd: c_int) -> c_int {
        use crate::syscalls::io::{get_fd_entry, track_fd};

        let init_state = crate::state::INITIALIZING.load(std::sync::atomic::Ordering::Relaxed);
        if init_state >= 2 {
            return crate::syscalls::linux_raw::raw_dup(oldfd);
        }

        let newfd = crate::syscalls::linux_raw::raw_dup(oldfd);
        if newfd >= 0 {
            if let Some(entry) = get_fd_entry(oldfd) {
                track_fd(newfd, &entry.path, entry.is_vfs);
            }
        }
        newfd
    }

    #[no_mangle]
    pub unsafe extern "C" fn dup2(oldfd: c_int, newfd: c_int) -> c_int {
        use crate::syscalls::io::{get_fd_entry, track_fd, untrack_fd};

        let init_state = crate::state::INITIALIZING.load(std::sync::atomic::Ordering::Relaxed);
        if init_state >= 2 {
            return crate::syscalls::linux_raw::raw_dup2(oldfd, newfd);
        }

        untrack_fd(newfd);
        let result = crate::syscalls::linux_raw::raw_dup2(oldfd, newfd);
        if result >= 0 {
            if let Some(entry) = get_fd_entry(oldfd) {
                track_fd(result, &entry.path, entry.is_vfs);
            }
        }
        result
    }

    #[no_mangle]
    pub unsafe extern "C" fn dup3(oldfd: c_int, newfd: c_int, _flags: c_int) -> c_int {
        // dup3 with flags=0 is essentially dup2
        dup2(oldfd, newfd)
    }

    // ========================================================================
    // P2: Path & Memory Operations
    // ========================================================================

    #[no_mangle]
    pub unsafe extern "C" fn access(path: *const c_char, mode: c_int) -> c_int {
        let init_state = crate::state::INITIALIZING.load(std::sync::atomic::Ordering::Relaxed);
        if init_state >= 2 {
            return crate::syscalls::linux_raw::raw_access(path, mode);
        }
        crate::syscalls::linux_raw::raw_access(path, mode)
    }

    #[no_mangle]
    pub unsafe extern "C" fn faccessat(
        dirfd: c_int,
        path: *const c_char,
        mode: c_int,
        _flags: c_int,
    ) -> c_int {
        // Simplified: use raw_access with AT_FDCWD behavior
        let init_state = crate::state::INITIALIZING.load(std::sync::atomic::Ordering::Relaxed);
        if init_state >= 2 || dirfd == libc::AT_FDCWD {
            return crate::syscalls::linux_raw::raw_access(path, mode);
        }
        crate::syscalls::linux_raw::raw_access(path, mode)
    }

    #[no_mangle]
    pub unsafe extern "C" fn readlink(
        path: *const c_char,
        buf: *mut c_char,
        bufsiz: libc::size_t,
    ) -> libc::ssize_t {
        let init_state = crate::state::INITIALIZING.load(std::sync::atomic::Ordering::Relaxed);
        if init_state >= 2 {
            return crate::syscalls::linux_raw::raw_readlink(path, buf, bufsiz);
        }
        crate::syscalls::linux_raw::raw_readlink(path, buf, bufsiz)
    }

    #[no_mangle]
    pub unsafe extern "C" fn getcwd(buf: *mut c_char, size: libc::size_t) -> *mut c_char {
        let init_state = crate::state::INITIALIZING.load(std::sync::atomic::Ordering::Relaxed);
        if init_state >= 2 {
            return crate::syscalls::linux_raw::raw_getcwd(buf, size);
        }
        crate::syscalls::linux_raw::raw_getcwd(buf, size)
    }

    #[no_mangle]
    pub unsafe extern "C" fn chdir(path: *const c_char) -> c_int {
        let init_state = crate::state::INITIALIZING.load(std::sync::atomic::Ordering::Relaxed);
        if init_state >= 2 {
            return crate::syscalls::linux_raw::raw_chdir(path);
        }
        crate::syscalls::linux_raw::raw_chdir(path)
    }

    #[no_mangle]
    pub unsafe extern "C" fn mmap(
        addr: *mut c_void,
        len: libc::size_t,
        prot: c_int,
        flags: c_int,
        fd: c_int,
        offset: libc::off_t,
    ) -> *mut c_void {
        // mmap always uses raw syscall to avoid recursion
        crate::syscalls::linux_raw::raw_mmap(addr, len, prot, flags, fd, offset)
    }

    #[no_mangle]
    pub unsafe extern "C" fn mmap64(
        addr: *mut c_void,
        len: libc::size_t,
        prot: c_int,
        flags: c_int,
        fd: c_int,
        offset: libc::off_t,
    ) -> *mut c_void {
        mmap(addr, len, prot, flags, fd, offset)
    }

    #[no_mangle]
    pub unsafe extern "C" fn munmap(addr: *mut c_void, len: libc::size_t) -> c_int {
        // munmap always uses raw syscall
        crate::syscalls::linux_raw::raw_munmap(addr, len)
    }

    #[no_mangle]
    pub unsafe extern "C" fn lseek(fd: c_int, offset: libc::off_t, whence: c_int) -> libc::off_t {
        let init_state = crate::state::INITIALIZING.load(std::sync::atomic::Ordering::Relaxed);
        if init_state >= 2 {
            return crate::syscalls::linux_raw::raw_lseek(fd, offset, whence);
        }
        crate::syscalls::linux_raw::raw_lseek(fd, offset, whence)
    }

    #[no_mangle]
    pub unsafe extern "C" fn lseek64(fd: c_int, offset: libc::off_t, whence: c_int) -> libc::off_t {
        lseek(fd, offset, whence)
    }

    #[no_mangle]
    pub unsafe extern "C" fn ftruncate(fd: c_int, length: libc::off_t) -> c_int {
        let init_state = crate::state::INITIALIZING.load(std::sync::atomic::Ordering::Relaxed);
        if init_state >= 2 {
            return crate::syscalls::linux_raw::raw_ftruncate(fd, length);
        }
        crate::syscalls::linux_raw::raw_ftruncate(fd, length)
    }

    #[no_mangle]
    pub unsafe extern "C" fn ftruncate64(fd: c_int, length: libc::off_t) -> c_int {
        ftruncate(fd, length)
    }

    #[no_mangle]
    pub unsafe extern "C" fn truncate(path: *const c_char, length: libc::off_t) -> c_int {
        // Block VFS mutation
        if let Some(res) = crate::syscalls::misc::block_vfs_mutation(path) {
            return res;
        }
        // Call via raw syscall pattern (truncate syscall = 76 on x86_64, ftruncateat on aarch64)
        #[cfg(target_arch = "x86_64")]
        {
            let ret: i64;
            std::arch::asm!(
                "syscall",
                in("rax") 76i64,
                in("rdi") path,
                in("rsi") length,
                lateout("rax") ret,
                lateout("rcx") _,
                lateout("r11") _,
            );
            if ret < 0 {
                crate::set_errno(-ret as c_int);
                return -1;
            }
            return ret as c_int;
        }
        #[cfg(target_arch = "aarch64")]
        {
            // aarch64 doesn't have truncate, use openat + ftruncate
            let fd =
                crate::syscalls::linux_raw::raw_openat(libc::AT_FDCWD, path, libc::O_WRONLY, 0);
            if fd < 0 {
                return -1;
            }
            let ret = crate::syscalls::linux_raw::raw_ftruncate(fd, length);
            crate::syscalls::linux_raw::raw_close(fd);
            ret
        }
    }

    #[no_mangle]
    pub unsafe extern "C" fn truncate64(path: *const c_char, length: libc::off_t) -> c_int {
        truncate(path, length)
    }

    #[no_mangle]
    pub unsafe extern "C" fn link(oldpath: *const c_char, newpath: *const c_char) -> c_int {
        // Block VFS mutation
        if let Some(res) = crate::syscalls::misc::block_vfs_mutation(oldpath) {
            return res;
        }
        if let Some(res) = crate::syscalls::misc::block_vfs_mutation(newpath) {
            return res;
        }
        crate::syscalls::linux_raw::raw_link(oldpath, newpath)
    }
}

#[cfg(target_os = "macos")]
#[no_mangle]
pub unsafe extern "C" fn fcntl_shim(fd: c_int, cmd: c_int, arg: c_long) -> c_int {
    fcntl_shim_c_impl(fd, cmd, arg)
}

#[allow(unused_macros)]
macro_rules! shim {
    ($name:ident, $real:ident, ($($arg:ident : $typ:ty),*) -> $ret:ty) => {
        #[no_mangle]
        pub unsafe extern "C" fn $name($($arg : $typ),*) -> $ret {
            let real = std::mem::transmute::<*const (), unsafe extern "C" fn($($typ),*) -> $ret>(IT_OPEN.old_func); // Placeholder, will fix
            real($($arg),*)
        }
    };
}

// Note: VFS logic shims are imported from syscalls/ modules:
// - dir: opendir_shim, readdir_shim, closedir_shim
// - stat: stat_shim, lstat_shim, fstat_shim
// - open: open_shim (with CoW logic)
// - misc: rename_shim (with EXDEV logic)
// RFC-0047: close() with CoW reingest for dirty FDs
// NOTE: Reingest logic temporarily disabled pending investigation
// Note: readlink_shim, realpath_shim imported from syscalls/path.rs
// Note: getcwd_shim, chdir_shim imported from syscalls/dir.rs
// RFC-0047: rename_shim, renameat_shim imported from syscalls/misc.rs with EXDEV logic
// Note: unlink_shim, rmdir_shim, mkdir_shim imported from syscalls/misc.rs
// Note: close_shim imported from syscalls/io.rs
#[cfg(target_os = "macos")]
#[no_mangle]
pub unsafe extern "C" fn dlopen_shim(p: *const c_char, f: c_int) -> *mut c_void {
    dlopen(p, f)
}
#[cfg(target_os = "macos")]
#[no_mangle]
pub unsafe extern "C" fn dlsym_shim(h: *mut c_void, s: *const c_char) -> *mut c_void {
    dlsym(h, s)
}
#[cfg(target_os = "macos")]
#[no_mangle]
pub unsafe extern "C" fn faccessat_shim(d: c_int, p: *const c_char, m: c_int, f: c_int) -> c_int {
    faccessat(d, p, m, f)
}
#[cfg(target_os = "macos")]
#[no_mangle]
pub unsafe extern "C" fn symlink_shim(p1: *const c_char, p2: *const c_char) -> c_int {
    symlink(p1, p2)
}
#[cfg(target_os = "macos")]
#[no_mangle]
pub unsafe extern "C" fn flock_shim(fd: c_int, op: c_int) -> c_int {
    use crate::ipc::sync_ipc_flock;
    use crate::state::{is_vfs_ready, ShimGuard, ShimState};
    use crate::syscalls::io::get_fd_entry;

    if !is_vfs_ready() {
        return flock(fd, op);
    }

    let entry = match get_fd_entry(fd) {
        Some(e) if e.is_vfs => e,
        _ => return flock(fd, op),
    };

    let _guard = match ShimGuard::enter() {
        Some(g) => g,
        None => return flock(fd, op),
    };

    let state = match ShimState::get() {
        Some(s) => s,
        None => return flock(fd, op),
    };

    if sync_ipc_flock(&state.socket_path, &entry.path, op) {
        0
    } else {
        if op & libc::LOCK_NB != 0 {
            crate::set_errno(libc::EWOULDBLOCK);
        } else {
            crate::set_errno(libc::EINTR);
        }
        -1
    }
}
// Note: VFS logic shims are imported from syscalls/ modules:
// - dir: opendir_shim, readdir_shim, closedir_shim
// - stat: stat_shim, lstat_shim, fstat_shim
// - open: open_shim (with CoW logic)
// - misc: rename_shim (with EXDEV logic)
// RFC-0047: close() with CoW reingest for dirty FDs
// NOTE: Reingest logic temporarily disabled pending investigation
// Note: readlink_shim, realpath_shim imported from syscalls/path.rs
// Note: getcwd_shim, chdir_shim imported from syscalls/dir.rs
// RFC-0047: rename_shim, renameat_shim imported from syscalls/misc.rs with EXDEV logic
// Note: unlink_shim, rmdir_shim, mkdir_shim imported from syscalls/misc.rs
// Note: close_shim imported from syscalls/io.rs
#[no_mangle]
pub unsafe extern "C" fn velo_fcntl_impl(fd: c_int, cmd: c_int, arg: c_long) -> c_int {
    #[cfg(target_os = "macos")]
    {
        fcntl(fd, cmd, arg)
    }
    #[cfg(target_os = "linux")]
    {
        #[cfg(target_arch = "x86_64")]
        {
            let ret: i64;
            std::arch::asm!(
                "syscall", in("rax") 72, in("rdi") fd as i64, in("rsi") cmd as i64, in("rdx") arg,
                lateout("rax") ret,
            );
            if ret < 0 {
                crate::set_errno(-ret as c_int);
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
                in("x8") 25i64, // fcntl
                in("x0") fd as i64,
                in("x1") cmd as i64,
                in("x2") arg,
                lateout("x0") ret,
            );
            if ret < 0 {
                crate::set_errno(-ret as c_int);
                -1
            } else {
                ret as c_int
            }
        }
    }
}
#[cfg(target_os = "macos")]
#[no_mangle]
pub unsafe extern "C" fn execve_shim(
    p: *const c_char,
    a: *const *const c_char,
    e: *const *const c_char,
) -> c_int {
    execve(p, a, e)
}
#[cfg(target_os = "macos")]
#[no_mangle]
pub unsafe extern "C" fn posix_spawn_shim(
    p: *mut pid_t,
    pa: *const c_char,
    fa: *const c_void,
    at: *const c_void,
    ar: *const *const c_char,
    e: *const *const c_char,
) -> c_int {
    posix_spawn(p, pa, fa, at, ar, e)
}
#[cfg(target_os = "macos")]
#[no_mangle]
pub unsafe extern "C" fn posix_spawnp_shim(
    p: *mut pid_t,
    f: *const c_char,
    fa: *const c_void,
    at: *const c_void,
    ar: *const *const c_char,
    e: *const *const c_char,
) -> c_int {
    posix_spawnp(p, f, fa, at, ar, e)
}

#[cfg(target_os = "macos")]
#[link_section = "__DATA,__interpose"]
#[used]
pub static IT_OPEN: Interpose = Interpose {
    new_func: open_shim as *const (),
    old_func: open as *const (),
};
#[cfg(target_os = "macos")]
#[link_section = "__DATA,__interpose"]
#[used]
pub static IT_WRITE: Interpose = Interpose {
    new_func: write_shim as *const (),
    old_func: write as *const (),
};
#[cfg(target_os = "macos")]
#[link_section = "__DATA,__interpose"]
#[used]
pub static IT_CLOSE: Interpose = Interpose {
    new_func: close_shim as *const (),
    old_func: close as *const (),
};
#[cfg(target_os = "macos")]
#[link_section = "__DATA,__interpose"]
#[used]
pub static IT_STAT: Interpose = Interpose {
    new_func: stat_shim as *const (),
    old_func: stat as *const (),
};
#[cfg(target_os = "macos")]
#[link_section = "__DATA,__interpose"]
#[used]
pub static IT_LSTAT: Interpose = Interpose {
    new_func: lstat_shim as *const (),
    old_func: lstat as *const (),
};
#[cfg(target_os = "macos")]
#[link_section = "__DATA,__interpose"]
#[used]
pub static IT_FSTAT: Interpose = Interpose {
    new_func: fstat_shim as *const (),
    old_func: fstat as *const (),
};
#[cfg(target_os = "macos")]
#[link_section = "__DATA,__interpose"]
#[used]
pub static IT_OPENDIR: Interpose = Interpose {
    new_func: opendir_shim as *const (),
    old_func: opendir as *const (),
};
#[cfg(target_os = "macos")]
#[link_section = "__DATA,__interpose"]
#[used]
pub static IT_READDIR: Interpose = Interpose {
    new_func: readdir_shim as *const (),
    old_func: readdir as *const (),
};
#[cfg(target_os = "macos")]
#[link_section = "__DATA,__interpose"]
#[used]
pub static IT_CLOSEDIR: Interpose = Interpose {
    new_func: closedir_shim as *const (),
    old_func: closedir as *const (),
};
#[cfg(target_os = "macos")]
#[link_section = "__DATA,__interpose"]
#[used]
pub static IT_REALPATH: Interpose = Interpose {
    new_func: realpath_shim as *const (),
    old_func: realpath as *const (),
};
#[cfg(target_os = "macos")]
#[link_section = "__DATA,__interpose"]
#[used]
pub static IT_REALPATH_DARWIN: Interpose = Interpose {
    new_func: realpath_shim as *const (),
    old_func: realpath_darwin as *const (),
};
#[cfg(target_os = "macos")]
#[link_section = "__DATA,__interpose"]
#[used]
pub static IT_GETCWD: Interpose = Interpose {
    new_func: getcwd_shim as *const (),
    old_func: getcwd as *const (),
};
#[cfg(target_os = "macos")]
#[link_section = "__DATA,__interpose"]
#[used]
pub static IT_CHDIR: Interpose = Interpose {
    new_func: chdir_shim as *const (),
    old_func: chdir as *const (),
};
#[cfg(target_os = "macos")]
#[link_section = "__DATA,__interpose"]
#[used]
pub static IT_UNLINK: Interpose = Interpose {
    new_func: unlink_shim as *const (),
    old_func: unlink as *const (),
};
#[cfg(target_os = "macos")]
#[link_section = "__DATA,__interpose"]
#[used]
pub static IT_RENAME: Interpose = Interpose {
    new_func: rename_shim as *const (),
    old_func: rename as *const (),
};
#[cfg(target_os = "macos")]
#[link_section = "__DATA,__interpose"]
#[used]
pub static IT_RMDIR: Interpose = Interpose {
    new_func: rmdir_shim as *const (),
    old_func: rmdir as *const (),
};
#[cfg(target_os = "macos")]
#[link_section = "__DATA,__interpose"]
#[used]
pub static IT_UTIMENSAT: Interpose = Interpose {
    new_func: utimensat_shim as *const (),
    old_func: utimensat as *const (),
};
#[cfg(target_os = "macos")]
#[link_section = "__DATA,__interpose"]
#[used]
pub static IT_MKDIR: Interpose = Interpose {
    new_func: mkdir_shim as *const (),
    old_func: mkdir as *const (),
};
#[cfg(target_os = "macos")]
#[link_section = "__DATA,__interpose"]
#[used]
pub static IT_SYMLINK: Interpose = Interpose {
    new_func: symlink_shim as *const (),
    old_func: symlink as *const (),
};
#[cfg(target_os = "macos")]
#[link_section = "__DATA,__interpose"]
#[used]
pub static IT_FLOCK: Interpose = Interpose {
    new_func: flock_shim as *const (),
    old_func: flock as *const (),
};
#[cfg(target_os = "macos")]
#[link_section = "__DATA,__interpose"]
#[used]
pub static IT_READLINK: Interpose = Interpose {
    new_func: readlink_shim as *const (),
    old_func: readlink as *const (),
};
#[cfg(target_os = "macos")]
#[link_section = "__DATA,__interpose"]
#[used]
pub static IT_LINK: Interpose = Interpose {
    new_func: link_shim as *const (),
    old_func: link as *const (),
};
#[cfg(target_os = "macos")]
#[link_section = "__DATA,__interpose"]
#[used]
pub static IT_LINKAT: Interpose = Interpose {
    new_func: linkat_shim as *const (),
    old_func: linkat as *const (),
};
#[cfg(target_os = "macos")]
#[link_section = "__DATA,__interpose"]
#[used]
pub static IT_RENAMEAT: Interpose = Interpose {
    new_func: renameat_shim as *const (),
    old_func: renameat as *const (),
};
#[cfg(target_os = "macos")]
#[link_section = "__DATA,__interpose"]
#[used]
pub static IT_EXECVE: Interpose = Interpose {
    new_func: execve_shim as *const (),
    old_func: execve as *const (),
};
#[cfg(target_os = "macos")]
#[link_section = "__DATA,__interpose"]
#[used]
pub static IT_POSIX_SPAWN: Interpose = Interpose {
    new_func: posix_spawn_shim as *const (),
    old_func: posix_spawn as *const (),
};
#[cfg(target_os = "macos")]
#[link_section = "__DATA,__interpose"]
#[used]
pub static IT_POSIX_SPAWNP: Interpose = Interpose {
    new_func: posix_spawnp_shim as *const (),
    old_func: posix_spawnp as *const (),
};
#[cfg(target_os = "macos")]
#[link_section = "__DATA,__interpose"]
#[used]
pub static IT_MMAP: Interpose = Interpose {
    new_func: mmap_shim as *const (),
    old_func: libc::mmap as *const (),
};
#[cfg(target_os = "macos")]
#[link_section = "__DATA,__interpose"]
#[used]
pub static IT_DLOPEN: Interpose = Interpose {
    new_func: dlopen_shim as *const (),
    old_func: dlopen as *const (),
};
#[cfg(target_os = "macos")]
#[link_section = "__DATA,__interpose"]
#[used]
pub static IT_MUNMAP: Interpose = Interpose {
    new_func: munmap_shim as *const (),
    old_func: munmap as *const (),
};
#[cfg(target_os = "macos")]
#[link_section = "__DATA,__interpose"]
#[used]
pub static IT_DLSYM: Interpose = Interpose {
    new_func: dlsym_shim as *const (),
    old_func: dlsym as *const (),
};
#[cfg(target_os = "macos")]
#[link_section = "__DATA,__interpose"]
#[used]
pub static IT_ACCESS: Interpose = Interpose {
    new_func: access_shim as *const (),
    old_func: access as *const (),
};
#[cfg(target_os = "macos")]
#[link_section = "__DATA,__interpose"]
#[used]
pub static IT_READ: Interpose = Interpose {
    new_func: read_shim as *const (),
    old_func: libc::read as *const (),
};
#[cfg(target_os = "macos")]
#[link_section = "__DATA,__interpose"]
#[used]
pub static IT_FCNTL: Interpose = Interpose {
    new_func: fcntl_shim as *const (),
    old_func: fcntl as *const (),
};
#[cfg(target_os = "macos")]
#[link_section = "__DATA,__interpose"]
#[used]
pub static IT_OPENAT: Interpose = Interpose {
    new_func: openat_shim as *const (),
    old_func: openat as *const (),
};
#[cfg(target_os = "macos")]
#[link_section = "__DATA,__interpose"]
#[used]
pub static IT_FACCESSAT: Interpose = Interpose {
    new_func: faccessat_shim as *const (),
    old_func: faccessat as *const (),
};
#[cfg(target_os = "macos")]
#[link_section = "__DATA,__interpose"]
#[used]
pub static IT_FSTATAT: Interpose = Interpose {
    new_func: fstatat_shim as *const (),
    old_func: libc::fstatat as *const (),
};
#[cfg(target_os = "macos")]
#[link_section = "__DATA,__interpose"]
#[used]
pub static IT_CHMOD: Interpose = Interpose {
    new_func: chmod_shim as *const (),
    old_func: chmod as *const (),
};
#[cfg(target_os = "macos")]
#[link_section = "__DATA,__interpose"]
#[used]
pub static IT_FCHMODAT: Interpose = Interpose {
    new_func: fchmodat_shim as *const (),
    old_func: fchmodat as *const (),
};
#[cfg(target_os = "macos")]
#[link_section = "__DATA,__interpose"]
#[used]
pub static IT_TRUNCATE: Interpose = Interpose {
    new_func: truncate_shim as *const (),
    old_func: truncate as *const (),
};
#[cfg(target_os = "macos")]
#[link_section = "__DATA,__interpose"]
#[used]
pub static IT_CHFLAGS: Interpose = Interpose {
    new_func: chflags_shim as *const (),
    old_func: chflags as *const (),
};
#[cfg(target_os = "macos")]
#[link_section = "__DATA,__interpose"]
#[used]
pub static IT_SETXATTR: Interpose = Interpose {
    new_func: setxattr_shim as *const (),
    old_func: setxattr as *const (),
};
#[cfg(target_os = "macos")]
#[link_section = "__DATA,__interpose"]
#[used]
pub static IT_REMOVEXATTR: Interpose = Interpose {
    new_func: removexattr_shim as *const (),
    old_func: removexattr as *const (),
};
#[cfg(target_os = "macos")]
#[link_section = "__DATA,__interpose"]
#[used]
pub static IT_UTIMES: Interpose = Interpose {
    new_func: utimes_shim as *const (),
    old_func: utimes as *const (),
};
#[cfg(target_os = "macos")]
#[link_section = "__DATA,__interpose"]
#[used]
pub static IT_DUP: Interpose = Interpose {
    new_func: dup_shim as *const (),
    old_func: dup as *const (),
};
#[cfg(target_os = "macos")]
#[link_section = "__DATA,__interpose"]
#[used]
pub static IT_DUP2: Interpose = Interpose {
    new_func: dup2_shim as *const (),
    old_func: dup2 as *const (),
};
#[cfg(target_os = "macos")]
#[link_section = "__DATA,__interpose"]
#[used]
pub static IT_FCHDIR: Interpose = Interpose {
    new_func: fchdir_shim as *const (),
    old_func: fchdir as *const (),
};
#[cfg(target_os = "macos")]
#[link_section = "__DATA,__interpose"]
#[used]
pub static IT_LSEEK: Interpose = Interpose {
    new_func: lseek_shim as *const (),
    old_func: lseek as *const (),
};
#[cfg(target_os = "macos")]
#[link_section = "__DATA,__interpose"]
#[used]
pub static IT_FTRUNCATE: Interpose = Interpose {
    new_func: ftruncate_shim as *const (),
    old_func: ftruncate as *const (),
};
