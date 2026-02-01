use libc::{c_char, c_int, c_void, mode_t, size_t, ssize_t};

// Import all shims from crate root (lib.rs) for IT_* initialization
#[cfg(target_os = "macos")]
use crate::{
    access_shim, chdir_shim, close_shim, closedir_shim, dlopen_shim, dlsym_shim, execve_shim,
    faccessat_shim, fcntl_shim, flock_shim, fstat_shim, fstatat_shim, getcwd_shim, link_shim,
    linkat_shim, lstat_shim, mkdir_shim, mmap_shim, munmap_shim, open_shim, openat_shim,
    opendir_shim, posix_spawn_shim, posix_spawnp_shim, read_shim, readdir_shim, readlink_shim,
    realpath_shim, rename_shim, renameat_shim, rmdir_shim, stat_shim, symlink_shim, unlink_shim,
    utimensat_shim, write_shim,
};

#[cfg(target_os = "macos")]
#[repr(C)]
pub struct Interpose {
    pub new_func: *const (),
    pub old_func: *const (),
}

#[cfg(target_os = "macos")]
unsafe impl Sync for Interpose {}

#[cfg(target_os = "macos")]
#[allow(improper_ctypes)] // For some libc types that might warn
extern "C" {
    fn open(path: *const c_char, flags: c_int, mode: mode_t) -> c_int;
    fn close(fd: c_int) -> c_int;
    fn write(fd: c_int, buf: *const c_void, count: size_t) -> ssize_t;
    fn stat(path: *const c_char, buf: *mut libc::stat) -> c_int;
    fn lstat(path: *const c_char, buf: *mut libc::stat) -> c_int;
    fn fstat(fd: c_int, buf: *mut libc::stat) -> c_int;
    fn opendir(path: *const c_char) -> *mut libc::DIR;
    fn readdir(dirp: *mut libc::DIR) -> *mut libc::dirent;
    fn closedir(dirp: *mut libc::DIR) -> c_int;
    fn readlink(path: *const c_char, buf: *mut c_char, bufsiz: size_t) -> ssize_t;
    fn execve(path: *const c_char, argv: *const *const c_char, envp: *const *const c_char)
        -> c_int;
    fn posix_spawn(
        pid: *mut libc::pid_t,
        path: *const c_char,
        file_actions: *const c_void,
        attrp: *const c_void,
        argv: *const *const c_char,
        envp: *const *const c_char,
    ) -> c_int;
    fn posix_spawnp(
        pid: *mut libc::pid_t,
        file: *const c_char,
        file_actions: *const c_void,
        attrp: *const c_void,
        argv: *const *const c_char,
        envp: *const *const c_char,
    ) -> c_int;
    fn realpath(pathname: *const c_char, resolved_path: *mut c_char) -> *mut c_char;

    #[link_name = "realpath$DARWIN_EXTSN"]
    fn realpath_darwin(pathname: *const c_char, resolved_path: *mut c_char) -> *mut c_char;
    fn getcwd(buf: *mut c_char, size: size_t) -> *mut c_char;
    fn chdir(path: *const c_char) -> c_int;
    fn unlink(path: *const c_char) -> c_int;
    fn rename(oldpath: *const c_char, newpath: *const c_char) -> c_int;
    fn rmdir(path: *const c_char) -> c_int;

    fn dlopen(filename: *const c_char, flags: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    fn access(path: *const c_char, mode: c_int) -> c_int;

    // Additional external functions usually found in libc but declared here for interpose mapping
    fn faccessat(dirfd: c_int, pathname: *const c_char, mode: c_int, flags: c_int) -> c_int;
    fn openat(dirfd: c_int, pathname: *const c_char, flags: c_int, mode: mode_t) -> c_int;
    fn link(oldpath: *const c_char, newpath: *const c_char) -> c_int;
    fn linkat(
        olddirfd: c_int,
        oldpath: *const c_char,
        newdirfd: c_int,
        newpath: *const c_char,
        flags: c_int,
    ) -> c_int;
    fn renameat(
        olddirfd: c_int,
        oldpath: *const c_char,
        newdirfd: c_int,
        newpath: *const c_char,
    ) -> c_int;
    fn symlink(path1: *const c_char, path2: *const c_char) -> c_int;
    fn flock(fd: c_int, operation: c_int) -> c_int;
    fn utimensat(
        dirfd: c_int,
        pathname: *const c_char,
        times: *const libc::timespec,
        flags: c_int,
    ) -> c_int;
    fn mkdir(path: *const c_char, mode: mode_t) -> c_int;
    fn munmap(addr: *mut c_void, len: size_t) -> c_int;
    fn fcntl(fd: c_int, cmd: c_int, ...) -> c_int;
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
