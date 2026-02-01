//! Syscall interposition table for macOS/Linux shim.
//! Safety: All extern "C" functions here are dangerous FFI and must be used correctly.
#![allow(clippy::missing_safety_doc)]

#[cfg(target_os = "macos")]
use crate::syscalls::dir::{closedir_shim, opendir_shim, readdir_shim};
#[cfg(target_os = "macos")]
use crate::syscalls::misc::{rename_shim, renameat_shim};
#[cfg(target_os = "macos")]
use crate::syscalls::mmap::{mmap_shim, munmap_shim};
#[cfg(target_os = "macos")]
use crate::syscalls::open::{open_shim, openat_shim};
#[cfg(target_os = "macos")]
use crate::syscalls::path::{readlink_shim, realpath_shim};
#[cfg(target_os = "macos")]
use crate::syscalls::stat::{fstat_shim, lstat_shim, stat_shim};
#[cfg(target_os = "macos")]
use libc::{c_char, c_int, c_void, dirent, mode_t, pid_t, size_t, ssize_t, timespec, DIR};

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

// Simple I/O passthroughs (no VFS logic needed)
#[cfg(target_os = "macos")]
#[no_mangle]
pub unsafe extern "C" fn close_shim(fd: c_int) -> c_int {
    close(fd)
}
#[cfg(target_os = "macos")]
#[no_mangle]
pub unsafe extern "C" fn write_shim(fd: c_int, b: *const c_void, c: size_t) -> ssize_t {
    write(fd, b, c)
}
#[cfg(target_os = "macos")]
#[no_mangle]
pub unsafe extern "C" fn read_shim(fd: c_int, b: *mut c_void, c: size_t) -> ssize_t {
    read(fd, b, c)
}
// Note: readlink_shim, realpath_shim imported from syscalls/path.rs
#[cfg(target_os = "macos")]
#[no_mangle]
pub unsafe extern "C" fn getcwd_shim(b: *mut c_char, s: size_t) -> *mut c_char {
    getcwd(b, s)
}
#[cfg(target_os = "macos")]
#[no_mangle]
pub unsafe extern "C" fn chdir_shim(p: *const c_char) -> c_int {
    chdir(p)
}
#[cfg(target_os = "macos")]
#[no_mangle]
pub unsafe extern "C" fn unlink_shim(p: *const c_char) -> c_int {
    unlink(p)
}
// Note: rename_shim, renameat_shim imported from syscalls/misc.rs with EXDEV logic
#[cfg(target_os = "macos")]
#[no_mangle]
pub unsafe extern "C" fn rmdir_shim(p: *const c_char) -> c_int {
    rmdir(p)
}
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
pub unsafe extern "C" fn access_shim(p: *const c_char, m: c_int) -> c_int {
    access(p, m)
}
#[cfg(target_os = "macos")]
#[no_mangle]
pub unsafe extern "C" fn faccessat_shim(d: c_int, p: *const c_char, m: c_int, f: c_int) -> c_int {
    faccessat(d, p, m, f)
}
// Note: openat_shim imported from syscalls/open.rs
#[cfg(target_os = "macos")]
#[no_mangle]
pub unsafe extern "C" fn link_shim(o: *const c_char, n: *const c_char) -> c_int {
    link(o, n)
}
#[cfg(target_os = "macos")]
#[no_mangle]
pub unsafe extern "C" fn linkat_shim(
    f1: c_int,
    p1: *const c_char,
    f2: c_int,
    p2: *const c_char,
    f: c_int,
) -> c_int {
    linkat(f1, p1, f2, p2, f)
}
// Note: renameat_shim imported from syscalls/misc.rs with EXDEV logic
#[cfg(target_os = "macos")]
#[no_mangle]
pub unsafe extern "C" fn symlink_shim(p1: *const c_char, p2: *const c_char) -> c_int {
    symlink(p1, p2)
}
#[cfg(target_os = "macos")]
#[no_mangle]
pub unsafe extern "C" fn flock_shim(fd: c_int, o: c_int) -> c_int {
    flock(fd, o)
}
#[cfg(target_os = "macos")]
#[no_mangle]
pub unsafe extern "C" fn utimensat_shim(
    d: c_int,
    p: *const c_char,
    t: *const timespec,
    f: c_int,
) -> c_int {
    utimensat(d, p, t, f)
}
#[cfg(target_os = "macos")]
#[no_mangle]
pub unsafe extern "C" fn mkdir_shim(p: *const c_char, m: mode_t) -> c_int {
    mkdir(p, m)
}
// Note: mmap_shim, munmap_shim imported from syscalls/mmap.rs
#[cfg(target_os = "macos")]
#[no_mangle]
pub unsafe extern "C" fn fcntl_shim(f: c_int, c: c_int, a: c_int) -> c_int {
    fcntl(f, c, a)
}
#[cfg(target_os = "macos")]
#[no_mangle]
pub unsafe extern "C" fn fstatat_shim(
    d: c_int,
    p: *const c_char,
    b: *mut libc::stat,
    f: c_int,
) -> c_int {
    fstatat(d, p, b, f)
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
    old_func: libc::fcntl as *const (),
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
