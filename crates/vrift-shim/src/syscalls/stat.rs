use crate::reals::*;
use crate::state::*;
use libc::{c_char, c_int, stat as libc_stat};
use std::ffi::CStr;

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
        *mut libc::c_void,
        unsafe extern "C" fn(*const c_char, *mut libc_stat) -> c_int,
    >(REAL_STAT.get());
    passthrough_if_init!(real, path, buf);
    stat_impl(path, buf, true).unwrap_or_else(|| real(path, buf))
}

#[no_mangle]
#[cfg(target_os = "macos")]
pub unsafe extern "C" fn lstat_shim(path: *const c_char, buf: *mut libc_stat) -> c_int {
    let real = std::mem::transmute::<
        *mut libc::c_void,
        unsafe extern "C" fn(*const c_char, *mut libc_stat) -> c_int,
    >(REAL_LSTAT.get());
    passthrough_if_init!(real, path, buf);
    stat_impl(path, buf, false).unwrap_or_else(|| real(path, buf))
}

#[no_mangle]
#[cfg(target_os = "macos")]
pub unsafe extern "C" fn fstat_shim(fd: c_int, buf: *mut libc_stat) -> c_int {
    let real = std::mem::transmute::<
        *mut libc::c_void,
        unsafe extern "C" fn(c_int, *mut libc_stat) -> c_int,
    >(REAL_FSTAT.get());
    passthrough_if_init!(real, fd, buf);
    real(fd, buf)
}

#[no_mangle]
#[cfg(target_os = "macos")]
pub unsafe extern "C" fn access_shim(path: *const c_char, mode: c_int) -> c_int {
    let real = std::mem::transmute::<
        *mut libc::c_void,
        unsafe extern "C" fn(*const c_char, c_int) -> c_int,
    >(REAL_ACCESS.get());
    passthrough_if_init!(real, path, mode);

    let _guard = match ShimGuard::enter() {
        Some(g) => g,
        None => return real(path, mode),
    };

    let path_str = match CStr::from_ptr(path).to_str() {
        Ok(s) => s,
        Err(_) => return real(path, mode),
    };

    if ShimState::get()
        .map(|s| s.psfs_applicable(path_str))
        .unwrap_or(false)
    {
        return 0;
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
        *mut libc::c_void,
        unsafe extern "C" fn(c_int, *const c_char, *mut libc_stat, c_int) -> c_int,
    >(REAL_FSTATAT.get());
    passthrough_if_init!(real, dirfd, path, buf, flags);

    if dirfd == libc::AT_FDCWD || (!path.is_null() && *path == b'/' as i8) {
        if let Ok(path_str) = CStr::from_ptr(path).to_str() {
            if let Some(res) = stat_impl_common(path_str, buf) {
                return res;
            }
        }
    }

    real(dirfd, path, buf, flags)
}

/// Linux-specific fstatat shim called from interpose bridge
#[cfg(target_os = "linux")]
pub unsafe fn fstatat_shim_linux(
    dirfd: c_int,
    path: *const c_char,
    buf: *mut libc_stat,
    flags: c_int,
) -> c_int {
    if path.is_null() {
        return -libc::EFAULT;
    }

    let path_str = match CStr::from_ptr(path).to_str() {
        Ok(s) => s,
        Err(_) => return -libc::EINVAL,
    };

    if dirfd != libc::AT_FDCWD && !path_str.starts_with('/') {
        return -2;
    }

    match stat_impl_common(path_str, buf) {
        Some(res) => res,
        None => -2,
    }
}
