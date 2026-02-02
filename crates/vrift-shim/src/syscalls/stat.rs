use crate::state::*;
use libc::{c_char, c_int, stat as libc_stat};
use std::ffi::CStr;
use std::sync::atomic::Ordering;

#[cfg(target_os = "macos")]
use crate::interpose::*;

/// RFC-0044: Virtual stat implementation using Hot Stat Cache
/// Returns None to fallback to OS, Some(0) on success, Some(-1) on error
unsafe fn stat_impl_common(path_str: &str, buf: *mut libc_stat) -> Option<c_int> {
    let state = ShimState::get()?;

    // Check if in VFS domain (O(1) prefix check)
    if !state.psfs_applicable(path_str) {
        return None;
    }

    // Try Hot Stat Cache (O(1) mmap lookup)
    if let Some(entry) = mmap_lookup(state.mmap_ptr, state.mmap_size, path_str) {
        std::ptr::write_bytes(buf, 0, 1);
        (*buf).st_size = entry.size as _;
        #[cfg(target_os = "macos")]
        {
            (*buf).st_mode = entry.mode as u16;
            (*buf).st_mtime = entry.mtime as _;
        }
        #[cfg(target_os = "linux")]
        {
            (*buf).st_mode = entry.mode as _;
            (*buf).st_mtime = entry.mtime as _;
        }
        (*buf).st_dev = 0x52494654; // "RIFT"
        (*buf).st_nlink = 1;
        (*buf).st_ino = vrift_ipc::fnv1a_hash(path_str) as _;
        vfs_record!(EventType::StatHit, vrift_ipc::fnv1a_hash(path_str), 0);
        return Some(0);
    }

    // Try IPC query
    if let Some(entry) = state.query_manifest(path_str) {
        std::ptr::write_bytes(buf, 0, 1);
        (*buf).st_size = entry.size as _;
        #[cfg(target_os = "macos")]
        {
            (*buf).st_mode = entry.mode as u16;
            (*buf).st_mtime = entry.mtime as _;
        }
        #[cfg(target_os = "linux")]
        {
            (*buf).st_mode = entry.mode as _;
            (*buf).st_mtime = entry.mtime as _;
        }
        (*buf).st_dev = 0x52494654; // "RIFT"
        (*buf).st_nlink = 1;
        (*buf).st_ino = vrift_ipc::fnv1a_hash(path_str) as _;
        vfs_record!(EventType::StatHit, vrift_ipc::fnv1a_hash(path_str), 0);
        return Some(0);
    }

    vfs_record!(
        EventType::StatMiss,
        vrift_ipc::fnv1a_hash(path_str),
        -libc::ENOENT
    );

    None
}

unsafe fn stat_impl(
    path: *const c_char,
    buf: *mut libc_stat,
    _follow_links: bool,
) -> Option<c_int> {
    if path.is_null() {
        return None;
    }

    let _guard = ShimGuard::enter()?;
    let path_str = CStr::from_ptr(path).to_str().ok()?;

    // RFC-0044: Symlink following logic not yet implemented for VFS
    stat_impl_common(path_str, buf)
}

#[no_mangle]
#[cfg(target_os = "macos")]
pub unsafe extern "C" fn stat_shim(path: *const c_char, buf: *mut libc_stat) -> c_int {
    let real = std::mem::transmute::<
        *const (),
        unsafe extern "C" fn(*const c_char, *mut libc_stat) -> c_int,
    >(IT_STAT.old_func);
    if INITIALIZING.load(Ordering::Relaxed) != 0 || CIRCUIT_TRIPPED.load(Ordering::Relaxed) {
        return real(path, buf);
    }
    stat_impl(path, buf, true).unwrap_or_else(|| real(path, buf))
}

#[no_mangle]
#[cfg(target_os = "macos")]
pub unsafe extern "C" fn lstat_shim(path: *const c_char, buf: *mut libc_stat) -> c_int {
    let real = std::mem::transmute::<
        *const (),
        unsafe extern "C" fn(*const c_char, *mut libc_stat) -> c_int,
    >(IT_LSTAT.old_func);
    if INITIALIZING.load(Ordering::Relaxed) != 0 || CIRCUIT_TRIPPED.load(Ordering::Relaxed) {
        return real(path, buf);
    }
    stat_impl(path, buf, false).unwrap_or_else(|| real(path, buf))
}

#[no_mangle]
#[cfg(target_os = "macos")]
pub unsafe extern "C" fn fstat_shim(fd: c_int, buf: *mut libc_stat) -> c_int {
    let real = std::mem::transmute::<*const (), unsafe extern "C" fn(c_int, *mut libc_stat) -> c_int>(
        IT_FSTAT.old_func,
    );
    if INITIALIZING.load(Ordering::Relaxed) != 0 || CIRCUIT_TRIPPED.load(Ordering::Relaxed) {
        return real(fd, buf);
    }

    // RFC-0044: fstat currently passthrough since shim doesn't track FDs yet
    real(fd, buf)
}

#[no_mangle]
#[cfg(target_os = "macos")]
pub unsafe extern "C" fn access_shim(path: *const c_char, mode: c_int) -> c_int {
    use crate::interpose::IT_ACCESS;
    let real = std::mem::transmute::<*const (), unsafe extern "C" fn(*const c_char, c_int) -> c_int>(
        IT_ACCESS.old_func,
    );
    if INITIALIZING.load(Ordering::Relaxed) != 0 || CIRCUIT_TRIPPED.load(Ordering::Relaxed) {
        return real(path, mode);
    }

    let _guard = match ShimGuard::enter() {
        Some(g) => g,
        None => return real(path, mode),
    };

    let path_str = match CStr::from_ptr(path).to_str() {
        Ok(s) => s,
        Err(_) => return real(path, mode),
    };

    let state = match ShimState::get() {
        Some(s) => s,
        None => return real(path, mode),
    };

    if state.psfs_applicable(path_str) {
        // VFS file is always accessible if in manifest
        if state.query_manifest(path_str).is_some() {
            return 0;
        }
    }

    real(path, mode)
}

#[no_mangle]
#[cfg(target_os = "macos")]
pub unsafe extern "C" fn fstatat_shim(
    dirfd: c_int,
    path: *const c_char,
    buf: *mut libc_stat,
    flags: c_int,
) -> c_int {
    let real = std::mem::transmute::<
        *const (),
        unsafe extern "C" fn(c_int, *const c_char, *mut libc_stat, c_int) -> c_int,
    >(IT_FSTATAT.old_func);
    if INITIALIZING.load(Ordering::Relaxed) != 0 {
        return real(dirfd, path, buf, flags);
    }

    // Attempt VFS stat if path is absolute or AT_FDCWD
    if dirfd == libc::AT_FDCWD {
        if let Some(res) = stat_impl(path, buf, (flags & libc::AT_SYMLINK_NOFOLLOW) == 0) {
            return res;
        }
    }

    real(dirfd, path, buf, flags)
}

#[cfg(target_os = "linux")]
pub unsafe extern "C" fn fstatat_shim_linux(
    dirfd: c_int,
    path: *const c_char,
    buf: *mut libc_stat,
    flags: c_int,
) -> c_int {
    #[inline(always)]
    unsafe fn raw_fstatat(
        dirfd: c_int,
        path: *const c_char,
        buf: *mut libc_stat,
        flags: c_int,
    ) -> c_int {
        #[cfg(target_arch = "x86_64")]
        {
            let ret: i64;
            std::arch::asm!(
                "syscall", in("rax") 262, in("rdi") dirfd as i64, in("rsi") path, in("rdx") buf, in("r10") flags as i64,
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
                in("x8") 79i64, // fstatat
                in("x0") dirfd as i64,
                in("x1") path,
                in("x2") buf,
                in("x3") flags as i64,
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

    if path.is_null() {
        return raw_fstatat(dirfd, path, buf, flags);
    }

    let path_str = match CStr::from_ptr(path).to_str() {
        Ok(s) => s,
        Err(_) => return raw_fstatat(dirfd, path, buf, flags),
    };

    if let Some(res) = stat_impl_common(path_str, buf) {
        return res;
    }

    let ret = raw_fstatat(dirfd, path, buf, flags);
    if ret == 0 {
        let state = match ShimState::get() {
            Some(s) => s,
            None => return ret,
        };
        if state.psfs_applicable(path_str) {
            (*buf).st_dev = 0x52494654; // "RIFT"
        }
    }
    ret
}
