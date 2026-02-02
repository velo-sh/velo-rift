//! Syscall interposition table for macOS/Linux shim.
//! Safety: All extern "C" functions here are dangerous FFI and must be used correctly.
#![allow(clippy::missing_safety_doc)]

#[cfg(target_os = "macos")]
use crate::syscalls::dir::{closedir_shim, opendir_shim, readdir_shim};
#[cfg(target_os = "macos")]
use crate::syscalls::io::{dup2_shim, dup_shim, fchdir_shim, ftruncate_shim, lseek_shim};
#[cfg(target_os = "macos")]
use crate::syscalls::misc::{
    chflags_shim, chmod_shim, fchmodat_shim, link_shim, linkat_shim, removexattr_shim, rename_shim,
    renameat_shim, setxattr_shim, truncate_shim, utimensat_shim, utimes_shim,
};
#[cfg(target_os = "macos")]
use crate::syscalls::mmap::{mmap_shim, munmap_shim};
#[cfg(target_os = "macos")]
use crate::syscalls::open::{open_shim, openat_shim};
#[cfg(target_os = "macos")]
use crate::syscalls::path::{readlink_shim, realpath_shim};
#[cfg(target_os = "macos")]
use crate::syscalls::stat::{access_shim, fstat_shim, lstat_shim, stat_shim};
#[cfg(target_os = "macos")]
use libc::{c_char, c_int, c_void, dirent, mode_t, pid_t, size_t, ssize_t, timespec, timeval, DIR};
#[cfg(target_os = "macos")]
use std::ffi::CStr;

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
// RFC-0047: VFS-aware unlink - removes from Manifest for VFS paths
#[cfg(target_os = "macos")]
#[no_mangle]
pub unsafe extern "C" fn unlink_shim(p: *const c_char) -> c_int {
    use crate::state::{ShimGuard, SHIM_STATE};
    use std::sync::atomic::Ordering;

    let _guard = match ShimGuard::enter() {
        Some(g) => g,
        None => return unlink(p),
    };

    if p.is_null() {
        return unlink(p);
    }

    let path_str = match CStr::from_ptr(p).to_str() {
        Ok(s) => s,
        Err(_) => return unlink(p),
    };

    let state_ptr = SHIM_STATE.load(Ordering::Acquire);
    if state_ptr.is_null() {
        return unlink(p);
    }
    let state = &*state_ptr;

    // VFS path: remove from manifest
    if state.psfs_applicable(path_str) {
        if let Ok(()) = state.manifest_remove(path_str) {
            return 0;
        }
        // Fallthrough to real unlink if IPC fails
    }

    unlink(p)
}
// Note: rename_shim, renameat_shim imported from syscalls/misc.rs with EXDEV logic
// RFC-0047: VFS-aware rmdir - removes directory from Manifest for VFS paths
#[cfg(target_os = "macos")]
#[no_mangle]
pub unsafe extern "C" fn rmdir_shim(p: *const c_char) -> c_int {
    use crate::state::{ShimGuard, SHIM_STATE};
    use std::sync::atomic::Ordering;

    let _guard = match ShimGuard::enter() {
        Some(g) => g,
        None => return rmdir(p),
    };

    if p.is_null() {
        return rmdir(p);
    }

    let path_str = match CStr::from_ptr(p).to_str() {
        Ok(s) => s,
        Err(_) => return rmdir(p),
    };

    let state_ptr = SHIM_STATE.load(Ordering::Acquire);
    if state_ptr.is_null() {
        return rmdir(p);
    }
    let state = &*state_ptr;

    // VFS path: remove from manifest
    if state.psfs_applicable(path_str) {
        if let Ok(()) = state.manifest_remove(path_str) {
            return 0;
        }
    }

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
// Note: access_shim imported from syscalls/stat.rs with VFS logic
#[cfg(target_os = "macos")]
#[no_mangle]
pub unsafe extern "C" fn faccessat_shim(d: c_int, p: *const c_char, m: c_int, f: c_int) -> c_int {
    faccessat(d, p, m, f)
}
// Note: openat_shim imported from syscalls/open.rs
// Note: link_shim, linkat_shim imported from syscalls/misc.rs with VFS boundary logic
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
// utimensat_shim moved to syscalls/misc.rs with VFS blocking logic
// RFC-0047: VFS-aware mkdir - adds directory entry to Manifest for VFS paths
#[cfg(target_os = "macos")]
#[no_mangle]
pub unsafe extern "C" fn mkdir_shim(p: *const c_char, m: mode_t) -> c_int {
    use crate::state::{ShimGuard, SHIM_STATE};
    use std::sync::atomic::Ordering;

    let _guard = match ShimGuard::enter() {
        Some(g) => g,
        None => return mkdir(p, m),
    };

    if p.is_null() {
        return mkdir(p, m);
    }

    let path_str = match CStr::from_ptr(p).to_str() {
        Ok(s) => s,
        Err(_) => return mkdir(p, m),
    };

    let state_ptr = SHIM_STATE.load(Ordering::Acquire);
    if state_ptr.is_null() {
        return mkdir(p, m);
    }
    let state = &*state_ptr;

    // VFS path: add directory entry to manifest
    if state.psfs_applicable(path_str) {
        if let Ok(()) = state.manifest_mkdir(path_str, m) {
            return 0;
        }
    }

    mkdir(p, m)
}
// Variadic ABI Fix: fcntl() is fn(fd, cmd, ...) - arg only valid for certain cmds
#[cfg(target_os = "macos")]
#[no_mangle]
pub unsafe extern "C" fn fcntl_shim(fd: c_int, cmd: c_int, arg: c_int) -> c_int {
    // Variadic ABI safety: third arg only valid for certain commands
    // F_GETFD, F_GETFL, F_GETOWN, F_GETPATH, etc. don't use arg
    // F_SETFD, F_SETFL, F_DUPFD, F_SETOWN, etc. do use arg
    let safe_arg = match cmd {
        libc::F_GETFD | libc::F_GETFL => 0, // arg not used
        _ => arg,                           // use provided arg
    };
    fcntl(fd, cmd, safe_arg)
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

// NOTE: open() variadic ABI cannot work on macOS ARM64!
// Variadic args go to STACK but non-variadic shim reads from REGISTERS.
// O_CREAT check cannot fix this - mode is read from wrong location entirely.
// TODO: Use inline assembly or naked function to handle variadic correctly.
// #[cfg(target_os = "macos")]
// #[link_section = "__DATA,__interpose"]
// #[used]
// pub static IT_OPEN: Interpose = Interpose {
//     new_func: open_shim as *const (),
//     old_func: open as *const (),
// };
#[cfg(target_os = "macos")]
#[used]
#[allow(dead_code)]
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
// NOTE: fcntl variadic ABI cannot be fixed with cmd check on macOS ARM64!
// On macOS ARM64, variadic args are passed on STACK not registers, causing
// fundamental ABI mismatch when caller passes 2 args but shim reads 3.
// This breaks Tokio's F_SETFL O_NONBLOCK calls. Keeping fcntl_shim for Solid mode
// file locking where we call it directly, but NOT interposing.
// #[cfg(target_os = "macos")]
// #[link_section = "__DATA,__interpose"]
// #[used]
// pub static IT_FCNTL: Interpose = Interpose {
//     new_func: fcntl_shim as *const (),
//     old_func: libc::fcntl as *const (),
// };
// NOTE: openat() has same variadic ABI issue as open() on macOS ARM64
// #[cfg(target_os = "macos")]
// #[link_section = "__DATA,__interpose"]
// #[used]
// pub static IT_OPENAT: Interpose = Interpose {
//     new_func: openat_shim as *const (),
//     old_func: openat as *const (),
// };
#[cfg(target_os = "macos")]
#[used]
#[allow(dead_code)]
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

// Mutation Perimeter Interpositions
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

// FD Tracking Interpositions
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
