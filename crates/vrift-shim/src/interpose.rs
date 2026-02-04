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
    chflags_shim, chmod_shim, exchangedata_shim, execve_shim, faccessat_shim, fchmod_shim,
    fchmodat_shim, fchown_shim, fchownat_shim, flock_shim, link_shim, linkat_shim, mkdir_shim,
    mkdirat_shim, posix_spawn_shim, posix_spawnp_shim, removexattr_shim, rmdir_shim,
    setrlimit_shim, setxattr_shim, symlink_shim, symlinkat_shim, truncate_shim, unlink_shim,
    unlinkat_shim, utimensat_shim, utimes_shim,
};

#[cfg(target_os = "macos")]
use crate::syscalls::mmap::{mmap_shim, munmap_shim};
#[cfg(target_os = "macos")]
use crate::syscalls::path::realpath_shim;

use libc::{c_char, c_int, mode_t};

#[cfg(target_os = "macos")]
use libc::{c_long, c_void, dirent, pid_t, size_t, ssize_t, timespec, timeval, DIR};

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
    #[link_name = "open"]
    fn real_open(path: *const c_char, flags: c_int, mode: mode_t) -> c_int;
    #[link_name = "close"]
    fn real_close(fd: c_int) -> c_int;
    #[link_name = "write"]
    fn real_write(fd: c_int, buf: *const c_void, count: size_t) -> ssize_t;
    #[link_name = "read"]
    fn real_read(fd: c_int, buf: *mut c_void, count: size_t) -> ssize_t;
    #[link_name = "stat"]
    fn real_stat(path: *const c_char, buf: *mut libc::stat) -> c_int;
    #[link_name = "lstat"]
    fn real_lstat(path: *const c_char, buf: *mut libc::stat) -> c_int;
    #[link_name = "fstat"]
    fn real_fstat(fd: c_int, buf: *mut libc::stat) -> c_int;
    #[link_name = "opendir"]
    fn real_opendir(path: *const c_char) -> *mut DIR;
    #[link_name = "readdir"]
    fn real_readdir(dirp: *mut DIR) -> *mut dirent;
    #[link_name = "closedir"]
    fn real_closedir(dirp: *mut DIR) -> c_int;
    #[link_name = "readlink"]
    fn real_readlink(path: *const c_char, buf: *mut c_char, bufsiz: size_t) -> ssize_t;
    #[link_name = "execve"]
    fn real_execve(
        path: *const c_char,
        argv: *const *const c_char,
        envp: *const *const c_char,
    ) -> c_int;
    #[link_name = "posix_spawn"]
    fn real_posix_spawn(
        pid: *mut pid_t,
        path: *const c_char,
        fa: *const c_void,
        attr: *const c_void,
        argv: *const *const c_char,
        envp: *const *const c_char,
    ) -> c_int;
    #[link_name = "posix_spawnp"]
    fn real_posix_spawnp(
        pid: *mut pid_t,
        file: *const c_char,
        fa: *const c_void,
        attr: *const c_void,
        argv: *const *const c_char,
        envp: *const *const c_char,
    ) -> c_int;
    #[link_name = "realpath"]
    fn real_realpath(path: *const c_char, resolved: *mut c_char) -> *mut c_char;
    #[link_name = "realpath$DARWIN_EXTSN"]
    fn real_realpath_darwin(path: *const c_char, resolved: *mut c_char) -> *mut c_char;
    #[link_name = "getcwd"]
    fn real_getcwd(buf: *mut c_char, size: size_t) -> *mut c_char;
    #[link_name = "chdir"]
    fn real_chdir(path: *const c_char) -> c_int;
    #[link_name = "unlink"]
    fn real_unlink(path: *const c_char) -> c_int;
    #[link_name = "rename"]
    fn real_rename(old: *const c_char, new: *const c_char) -> c_int;
    #[link_name = "rmdir"]
    fn real_rmdir(path: *const c_char) -> c_int;
    #[link_name = "dlopen"]
    fn real_dlopen(path: *const c_char, flags: c_int) -> *mut c_void;
    #[link_name = "dlsym"]
    fn real_dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    #[link_name = "access"]
    fn real_access(path: *const c_char, mode: c_int) -> c_int;
    #[link_name = "faccessat"]
    fn real_faccessat(dirfd: c_int, path: *const c_char, mode: c_int, flags: c_int) -> c_int;
    #[link_name = "openat"]
    fn real_openat(dirfd: c_int, path: *const c_char, flags: c_int, mode: mode_t) -> c_int;
    #[link_name = "link"]
    fn real_link(old: *const c_char, new: *const c_char) -> c_int;
    #[link_name = "linkat"]
    fn real_linkat(
        fd1: c_int,
        p1: *const c_char,
        fd2: c_int,
        p2: *const c_char,
        flags: c_int,
    ) -> c_int;
    #[link_name = "renameat"]
    fn real_renameat(fd1: c_int, p1: *const c_char, fd2: c_int, p2: *const c_char) -> c_int;
    #[link_name = "symlink"]
    fn real_symlink(p1: *const c_char, p2: *const c_char) -> c_int;
    #[link_name = "flock"]
    fn real_flock(fd: c_int, op: c_int) -> c_int;
    #[link_name = "utimensat"]
    fn real_utimensat(
        dirfd: c_int,
        path: *const c_char,
        times: *const timespec,
        flags: c_int,
    ) -> c_int;
    #[link_name = "mkdir"]
    fn real_mkdir(path: *const c_char, mode: mode_t) -> c_int;
    #[link_name = "mmap"]
    fn real_mmap(
        addr: *mut c_void,
        len: size_t,
        prot: c_int,
        flags: c_int,
        fd: c_int,
        offset: libc::off_t,
    ) -> *mut c_void;
    #[link_name = "munmap"]
    fn real_munmap(addr: *mut c_void, len: size_t) -> c_int;
    #[link_name = "fcntl"]
    fn real_fcntl(fd: c_int, cmd: c_int, ...) -> c_int;
    #[link_name = "fstatat"]
    fn real_fstatat(dirfd: c_int, path: *const c_char, buf: *mut libc::stat, flags: c_int)
        -> c_int;
    #[link_name = "chmod"]
    fn real_chmod(path: *const c_char, mode: mode_t) -> c_int;
    #[link_name = "fchmodat"]
    fn real_fchmodat(dirfd: c_int, path: *const c_char, mode: mode_t, flags: c_int) -> c_int;
    #[link_name = "truncate"]
    fn real_truncate(path: *const c_char, length: libc::off_t) -> c_int;
    #[link_name = "chflags"]
    fn real_chflags(path: *const c_char, flags: libc::c_uint) -> c_int;
    #[link_name = "setxattr"]
    fn real_setxattr(
        path: *const c_char,
        name: *const c_char,
        value: *const c_void,
        size: size_t,
        position: u32,
        options: c_int,
    ) -> c_int;
    #[link_name = "removexattr"]
    fn real_removexattr(path: *const c_char, name: *const c_char, options: c_int) -> c_int;
    #[link_name = "utimes"]
    fn real_utimes(path: *const c_char, times: *const timeval) -> c_int;
    #[link_name = "dup"]
    fn real_dup(oldfd: c_int) -> c_int;
    #[link_name = "dup2"]
    fn real_dup2(oldfd: c_int, newfd: c_int) -> c_int;
    #[link_name = "fchdir"]
    fn real_fchdir(fd: c_int) -> c_int;
    #[link_name = "lseek"]
    fn real_lseek(fd: c_int, offset: libc::off_t, whence: c_int) -> libc::off_t;
    #[link_name = "ftruncate"]
    fn real_ftruncate(fd: c_int, length: libc::off_t) -> c_int;
    #[link_name = "unlinkat"]
    fn real_unlinkat(dirfd: c_int, path: *const c_char, flags: c_int) -> c_int;
    #[link_name = "mkdirat"]
    fn real_mkdirat(dirfd: c_int, path: *const c_char, mode: mode_t) -> c_int;
    #[link_name = "symlinkat"]
    fn real_symlinkat(p1: *const c_char, dirfd: c_int, p2: *const c_char) -> c_int;
    #[link_name = "fchmod"]
    fn real_fchmod(fd: c_int, mode: mode_t) -> c_int;
    #[link_name = "setrlimit"]
    fn real_setrlimit(resource: c_int, rlp: *const libc::rlimit) -> c_int;
    // P0-P1 Gap Fix: fchown/fchownat/exchangedata
    #[link_name = "fchown"]
    fn real_fchown(fd: c_int, owner: libc::uid_t, group: libc::gid_t) -> c_int;
    #[link_name = "fchownat"]
    fn real_fchownat(
        dirfd: c_int,
        path: *const c_char,
        owner: libc::uid_t,
        group: libc::gid_t,
        flags: c_int,
    ) -> c_int;
    #[link_name = "exchangedata"]
    fn real_exchangedata(
        path1: *const c_char,
        path2: *const c_char,
        options: libc::c_uint,
    ) -> c_int;
}

#[cfg(target_os = "macos")]
extern "C" {
    fn c_open_bridge(path: *const c_char, flags: c_int, mode: mode_t) -> c_int;
    fn c_openat_bridge(dirfd: c_int, path: *const c_char, flags: c_int, mode: mode_t) -> c_int;
    fn c_stat_bridge(path: *const c_char, buf: *mut libc::stat) -> c_int;
    fn c_lstat_bridge(path: *const c_char, buf: *mut libc::stat) -> c_int;
    fn c_fstat_bridge(fd: c_int, buf: *mut libc::stat) -> c_int;
    fn c_fstatat_bridge(
        dirfd: c_int,
        path: *const c_char,
        buf: *mut libc::stat,
        flags: c_int,
    ) -> c_int;
    fn c_access_bridge(path: *const c_char, mode: c_int) -> c_int;
    fn c_readlink_bridge(path: *const c_char, buf: *mut c_char, bufsiz: size_t) -> ssize_t;
    fn c_rename_bridge(old: *const c_char, new: *const c_char) -> c_int;
    fn c_renameat_bridge(fd1: c_int, p1: *const c_char, fd2: c_int, p2: *const c_char) -> c_int;
    fn fcntl_shim_c_impl(fd: c_int, cmd: c_int, arg: c_long) -> c_int;
}

// Active Interpositions (Group 1 + Core)
#[cfg(target_os = "macos")]
#[link_section = "__DATA,__interpose"]
#[used]
pub static IT_OPEN: Interpose = Interpose {
    new_func: c_open_bridge as _,
    old_func: real_open as _,
};
#[cfg(target_os = "macos")]
#[link_section = "__DATA,__interpose"]
#[used]
pub static IT_OPENAT: Interpose = Interpose {
    new_func: c_openat_bridge as _,
    old_func: real_openat as _,
};
#[cfg(target_os = "macos")]
#[link_section = "__DATA,__interpose"]
#[used]
pub static IT_STAT: Interpose = Interpose {
    new_func: c_stat_bridge as _,
    old_func: real_stat as _,
};
#[cfg(target_os = "macos")]
#[link_section = "__DATA,__interpose"]
#[used]
pub static IT_LSTAT: Interpose = Interpose {
    new_func: c_lstat_bridge as _,
    old_func: real_lstat as _,
};
#[cfg(target_os = "macos")]
#[link_section = "__DATA,__interpose"]
#[used]
pub static IT_FSTAT: Interpose = Interpose {
    new_func: c_fstat_bridge as _,
    old_func: real_fstat as _,
};
#[cfg(target_os = "macos")]
#[link_section = "__DATA,__interpose"]
#[used]
pub static IT_FSTATAT: Interpose = Interpose {
    new_func: c_fstatat_bridge as _,
    old_func: real_fstatat as _,
};
#[cfg(target_os = "macos")]
#[link_section = "__DATA,__interpose"]
#[used]
pub static IT_ACCESS: Interpose = Interpose {
    new_func: c_access_bridge as _,
    old_func: real_access as _,
};
#[cfg(target_os = "macos")]
#[link_section = "__DATA,__interpose"]
#[used]
pub static IT_READLINK: Interpose = Interpose {
    new_func: c_readlink_bridge as _,
    old_func: real_readlink as _,
};
#[cfg(target_os = "macos")]
#[link_section = "__DATA,__interpose"]
#[used]
pub static IT_RENAME: Interpose = Interpose {
    new_func: c_rename_bridge as _,
    old_func: real_rename as _,
};
#[cfg(target_os = "macos")]
#[link_section = "__DATA,__interpose"]
#[used]
pub static IT_RENAMEAT: Interpose = Interpose {
    new_func: c_renameat_bridge as _,
    old_func: real_renameat as _,
};

#[cfg(target_os = "macos")]
#[link_section = "__DATA,__nointerpose"]
#[used]
pub static IT_FCNTL: Interpose = Interpose {
    new_func: fcntl_shim_c_impl as _,
    old_func: real_fcntl as _,
};

#[cfg(target_os = "macos")]
#[link_section = "__DATA,__nointerpose"]
#[used]
pub static IT_MMAP: Interpose = Interpose {
    new_func: mmap_shim as _,
    old_func: real_mmap as _,
};
#[cfg(target_os = "macos")]
#[link_section = "__DATA,__nointerpose"]
#[used]
pub static IT_MUNMAP: Interpose = Interpose {
    new_func: munmap_shim as _,
    old_func: real_munmap as _,
};

// Passthrough / Inactive Interpositions (Sectioned to __nointerpose to avoid dyld resolution overhead)
#[cfg(target_os = "macos")]
#[link_section = "__DATA,__nointerpose"]
#[used]
pub static IT_WRITE: Interpose = Interpose {
    new_func: write_shim as _,
    old_func: real_write as _,
};
#[cfg(target_os = "macos")]
#[link_section = "__DATA,__nointerpose"]
#[used]
pub static IT_READ: Interpose = Interpose {
    new_func: read_shim as _,
    old_func: real_read as _,
};
#[cfg(target_os = "macos")]
#[link_section = "__DATA,__nointerpose"]
#[used]
pub static IT_CLOSE: Interpose = Interpose {
    new_func: close_shim as _,
    old_func: real_close as _,
};
#[cfg(target_os = "macos")]
#[link_section = "__DATA,__nointerpose"]
#[used]
pub static IT_OPENDIR: Interpose = Interpose {
    new_func: opendir_shim as _,
    old_func: real_opendir as _,
};
#[cfg(target_os = "macos")]
#[link_section = "__DATA,__nointerpose"]
#[used]
pub static IT_READDIR: Interpose = Interpose {
    new_func: readdir_shim as _,
    old_func: real_readdir as _,
};
#[cfg(target_os = "macos")]
#[link_section = "__DATA,__nointerpose"]
#[used]
pub static IT_CLOSEDIR: Interpose = Interpose {
    new_func: closedir_shim as _,
    old_func: real_closedir as _,
};
#[cfg(target_os = "macos")]
#[link_section = "__DATA,__nointerpose"]
#[used]
pub static IT_REALPATH: Interpose = Interpose {
    new_func: realpath_shim as _,
    old_func: real_realpath as _,
};
#[cfg(target_os = "macos")]
#[link_section = "__DATA,__nointerpose"]
#[used]
pub static IT_REALPATH_DARWIN: Interpose = Interpose {
    new_func: realpath_shim as _,
    old_func: real_realpath_darwin as _,
};
#[cfg(target_os = "macos")]
#[link_section = "__DATA,__nointerpose"]
#[used]
pub static IT_GETCWD: Interpose = Interpose {
    new_func: getcwd_shim as _,
    old_func: real_getcwd as _,
};
#[cfg(target_os = "macos")]
#[link_section = "__DATA,__nointerpose"]
#[used]
pub static IT_CHDIR: Interpose = Interpose {
    new_func: chdir_shim as _,
    old_func: real_chdir as _,
};
#[cfg(target_os = "macos")]
#[link_section = "__DATA,__interpose"]
#[used]
pub static IT_UNLINK: Interpose = Interpose {
    new_func: unlink_shim as _,
    old_func: real_unlink as _,
};
#[cfg(target_os = "macos")]
#[link_section = "__DATA,__interpose"]
#[used]
pub static IT_RMDIR: Interpose = Interpose {
    new_func: rmdir_shim as _,
    old_func: real_rmdir as _,
};
#[cfg(target_os = "macos")]
#[link_section = "__DATA,__interpose"]
#[used]
pub static IT_UTIMENSAT: Interpose = Interpose {
    new_func: utimensat_shim as _,
    old_func: real_utimensat as _,
};
#[cfg(target_os = "macos")]
#[link_section = "__DATA,__nointerpose"]
#[used]
pub static IT_MKDIR: Interpose = Interpose {
    new_func: mkdir_shim as _,
    old_func: real_mkdir as _,
};
#[cfg(target_os = "macos")]
#[link_section = "__DATA,__nointerpose"]
#[used]
pub static IT_SYMLINK: Interpose = Interpose {
    new_func: symlink_shim as _,
    old_func: real_symlink as _,
};
#[cfg(target_os = "macos")]
#[link_section = "__DATA,__nointerpose"]
#[used]
pub static IT_FLOCK: Interpose = Interpose {
    new_func: flock_shim as _,
    old_func: real_flock as _,
};
#[cfg(target_os = "macos")]
#[link_section = "__DATA,__interpose"]
#[used]
pub static IT_LINK: Interpose = Interpose {
    new_func: link_shim as _,
    old_func: real_link as _,
};
#[cfg(target_os = "macos")]
#[link_section = "__DATA,__interpose"]
#[used]
pub static IT_LINKAT: Interpose = Interpose {
    new_func: linkat_shim as _,
    old_func: real_linkat as _,
};
#[cfg(target_os = "macos")]
#[link_section = "__DATA,__nointerpose"]
#[used]
pub static IT_EXECVE: Interpose = Interpose {
    new_func: execve_shim as _,
    old_func: real_execve as _,
};
#[cfg(target_os = "macos")]
#[link_section = "__DATA,__nointerpose"]
#[used]
pub static IT_POSIX_SPAWN: Interpose = Interpose {
    new_func: posix_spawn_shim as _,
    old_func: real_posix_spawn as _,
};
#[cfg(target_os = "macos")]
#[link_section = "__DATA,__nointerpose"]
#[used]
pub static IT_POSIX_SPAWNP: Interpose = Interpose {
    new_func: posix_spawnp_shim as _,
    old_func: real_posix_spawnp as _,
};
#[cfg(target_os = "macos")]
#[link_section = "__DATA,__nointerpose"]
#[used]
pub static IT_DLOPEN: Interpose = Interpose {
    new_func: libc::dlopen as _,
    old_func: real_dlopen as _,
};
#[cfg(target_os = "macos")]
#[link_section = "__DATA,__nointerpose"]
#[used]
pub static IT_DLSYM: Interpose = Interpose {
    new_func: libc::dlsym as _,
    old_func: real_dlsym as _,
};
#[cfg(target_os = "macos")]
#[link_section = "__DATA,__nointerpose"]
#[used]
pub static IT_FACCESSAT: Interpose = Interpose {
    new_func: faccessat_shim as _,
    old_func: real_faccessat as _,
};
#[cfg(target_os = "macos")]
#[link_section = "__DATA,__interpose"]
#[used]
pub static IT_CHMOD: Interpose = Interpose {
    new_func: chmod_shim as _,
    old_func: real_chmod as _,
};
#[cfg(target_os = "macos")]
#[link_section = "__DATA,__interpose"]
#[used]
pub static IT_FCHMODAT: Interpose = Interpose {
    new_func: fchmodat_shim as _,
    old_func: real_fchmodat as _,
};
#[cfg(target_os = "macos")]
#[link_section = "__DATA,__interpose"]
#[used]
pub static IT_TRUNCATE: Interpose = Interpose {
    new_func: truncate_shim as _,
    old_func: real_truncate as _,
};
#[cfg(target_os = "macos")]
#[link_section = "__DATA,__interpose"]
#[used]
pub static IT_CHFLAGS: Interpose = Interpose {
    new_func: chflags_shim as _,
    old_func: real_chflags as _,
};
#[cfg(target_os = "macos")]
#[link_section = "__DATA,__nointerpose"]
#[used]
pub static IT_SETXATTR: Interpose = Interpose {
    new_func: setxattr_shim as _,
    old_func: real_setxattr as _,
};
#[cfg(target_os = "macos")]
#[link_section = "__DATA,__nointerpose"]
#[used]
pub static IT_REMOVEXATTR: Interpose = Interpose {
    new_func: removexattr_shim as _,
    old_func: real_removexattr as _,
};
#[cfg(target_os = "macos")]
#[link_section = "__DATA,__nointerpose"]
#[used]
pub static IT_UTIMES: Interpose = Interpose {
    new_func: utimes_shim as _,
    old_func: real_utimes as _,
};
#[cfg(target_os = "macos")]
#[link_section = "__DATA,__nointerpose"]
#[used]
pub static IT_DUP: Interpose = Interpose {
    new_func: dup_shim as _,
    old_func: real_dup as _,
};
#[cfg(target_os = "macos")]
#[link_section = "__DATA,__nointerpose"]
#[used]
pub static IT_DUP2: Interpose = Interpose {
    new_func: dup2_shim as _,
    old_func: real_dup2 as _,
};
#[cfg(target_os = "macos")]
#[link_section = "__DATA,__nointerpose"]
#[used]
pub static IT_FCHDIR: Interpose = Interpose {
    new_func: fchdir_shim as _,
    old_func: real_fchdir as _,
};
#[cfg(target_os = "macos")]
#[link_section = "__DATA,__nointerpose"]
#[used]
pub static IT_LSEEK: Interpose = Interpose {
    new_func: lseek_shim as _,
    old_func: real_lseek as _,
};
#[cfg(target_os = "macos")]
#[link_section = "__DATA,__interpose"]
#[used]
pub static IT_FTRUNCATE: Interpose = Interpose {
    new_func: ftruncate_shim as _,
    old_func: real_ftruncate as _,
};
#[cfg(target_os = "macos")]
#[link_section = "__DATA,__interpose"]
#[used]
pub static IT_UNLINKAT: Interpose = Interpose {
    new_func: unlinkat_shim as _,
    old_func: real_unlinkat as _,
};
#[cfg(target_os = "macos")]
#[link_section = "__DATA,__nointerpose"]
#[used]
pub static IT_MKDIRAT: Interpose = Interpose {
    new_func: mkdirat_shim as _,
    old_func: real_mkdirat as _,
};
#[cfg(target_os = "macos")]
#[link_section = "__DATA,__nointerpose"]
#[used]
pub static IT_SYMLINKAT: Interpose = Interpose {
    new_func: symlinkat_shim as _,
    old_func: real_symlinkat as _,
};
#[cfg(target_os = "macos")]
#[link_section = "__DATA,__interpose"]
#[used]
pub static IT_FCHMOD: Interpose = Interpose {
    new_func: fchmod_shim as _,
    old_func: real_fchmod as _,
};
#[cfg(target_os = "macos")]
#[link_section = "__DATA,__nointerpose"]
#[used]
pub static IT_SETRLIMIT: Interpose = Interpose {
    new_func: setrlimit_shim as _,
    old_func: real_setrlimit as _,
};

// P0-P1 Gap Fix: fchown/fchownat/exchangedata interposition
#[cfg(target_os = "macos")]
#[link_section = "__DATA,__interpose"]
#[used]
pub static IT_FCHOWN: Interpose = Interpose {
    new_func: fchown_shim as _,
    old_func: real_fchown as _,
};
#[cfg(target_os = "macos")]
#[link_section = "__DATA,__interpose"]
#[used]
pub static IT_FCHOWNAT: Interpose = Interpose {
    new_func: fchownat_shim as _,
    old_func: real_fchownat as _,
};
#[cfg(target_os = "macos")]
#[link_section = "__DATA,__nointerpose"]
#[used]
pub static IT_EXCHANGEDATA: Interpose = Interpose {
    new_func: exchangedata_shim as _,
    old_func: real_exchangedata as _,
};

// =============================================================================
// Linux LD_PRELOAD Symbol Exports
// =============================================================================
// On Linux, LD_PRELOAD works by symbol interposition. We export functions
// with the same names as libc functions to intercept them.

#[cfg(target_os = "linux")]
#[no_mangle]
pub unsafe extern "C" fn open(path: *const c_char, flags: c_int, mode: mode_t) -> c_int {
    crate::syscalls::open::open_shim_c_impl(path, flags, mode)
}

#[cfg(target_os = "linux")]
#[no_mangle]
pub unsafe extern "C" fn open64(path: *const c_char, flags: c_int, mode: mode_t) -> c_int {
    crate::syscalls::open::open_shim_c_impl(path, flags, mode)
}

#[cfg(target_os = "linux")]
#[no_mangle]
pub unsafe extern "C" fn openat(
    dirfd: c_int,
    path: *const c_char,
    flags: c_int,
    mode: mode_t,
) -> c_int {
    crate::syscalls::open::velo_openat_impl(dirfd, path, flags, mode)
}

#[cfg(target_os = "linux")]
#[no_mangle]
pub unsafe extern "C" fn openat64(
    dirfd: c_int,
    path: *const c_char,
    flags: c_int,
    mode: mode_t,
) -> c_int {
    crate::syscalls::open::velo_openat_impl(dirfd, path, flags, mode)
}

// Linux chmod interception - blocks VFS mutations
#[cfg(target_os = "linux")]
#[no_mangle]
pub unsafe extern "C" fn chmod(path: *const c_char, mode: mode_t) -> c_int {
    // Check VFS prefix to block mutations on VFS-managed files
    if let Some(err) = crate::syscalls::misc::quick_block_vfs_mutation(path) {
        return err;
    }
    crate::syscalls::linux_raw::raw_chmod(path, mode)
}

#[cfg(target_os = "linux")]
#[no_mangle]
pub unsafe extern "C" fn fchmodat(
    dirfd: c_int,
    path: *const c_char,
    mode: mode_t,
    flags: c_int,
) -> c_int {
    // Check VFS prefix to block mutations on VFS-managed files
    if let Some(err) = crate::syscalls::misc::quick_block_vfs_mutation(path) {
        return err;
    }
    crate::syscalls::linux_raw::raw_fchmodat(dirfd, path, mode, flags)
}

// Linux unlink/rm interception - blocks VFS mutations
#[cfg(target_os = "linux")]
#[no_mangle]
pub unsafe extern "C" fn unlink(path: *const c_char) -> c_int {
    if let Some(err) = crate::syscalls::misc::quick_block_vfs_mutation(path) {
        return err;
    }
    crate::syscalls::linux_raw::raw_unlink(path)
}

#[cfg(target_os = "linux")]
#[no_mangle]
pub unsafe extern "C" fn unlinkat(dirfd: c_int, path: *const c_char, flags: c_int) -> c_int {
    if let Some(err) = crate::syscalls::misc::quick_block_vfs_mutation(path) {
        return err;
    }
    crate::syscalls::linux_raw::raw_unlinkat(dirfd, path, flags)
}

// Linux utimensat/touch interception
#[cfg(target_os = "linux")]
#[no_mangle]
pub unsafe extern "C" fn utimensat(
    dirfd: c_int,
    path: *const c_char,
    times: *const libc::timespec,
    flags: c_int,
) -> c_int {
    if let Some(err) = crate::syscalls::misc::quick_block_vfs_mutation(path) {
        return err;
    }
    crate::syscalls::linux_raw::raw_utimensat(dirfd, path, times, flags)
}

// Linux utimes interception (for touch command)
#[cfg(target_os = "linux")]
#[no_mangle]
pub unsafe extern "C" fn utimes(path: *const c_char, times: *const libc::timeval) -> c_int {
    if let Some(err) = crate::syscalls::misc::quick_block_vfs_mutation(path) {
        return err;
    }
    crate::syscalls::linux_raw::raw_utimes(path, times)
}

// P0-P1 Gap Fix: Linux fchown/fchownat exports
#[cfg(target_os = "linux")]
#[no_mangle]
pub unsafe extern "C" fn fchown(fd: c_int, owner: libc::uid_t, group: libc::gid_t) -> c_int {
    crate::syscalls::misc::fchown_shim(fd, owner, group)
}

#[cfg(target_os = "linux")]
#[no_mangle]
pub unsafe extern "C" fn fchownat(
    dirfd: c_int,
    path: *const c_char,
    owner: libc::uid_t,
    group: libc::gid_t,
    flags: c_int,
) -> c_int {
    crate::syscalls::misc::fchownat_shim(dirfd, path, owner, group, flags)
}
