#[cfg(target_os = "macos")]
use crate::interpose::*;
use crate::state::*;
use libc::{c_char, c_int, stat as libc_stat};
use std::ffi::CStr;
#[cfg(target_os = "macos")]
use std::sync::atomic::Ordering;

/// RFC-0044: Virtual stat implementation using Hot Stat Cache
/// Returns None to fallback to OS, Some(0) on success, Some(-1) on error
#[cfg(target_os = "macos")]
unsafe fn stat_impl(
    path: *const c_char,
    buf: *mut libc_stat,
    real_stat: unsafe extern "C" fn(*const c_char, *mut libc_stat) -> c_int,
) -> Option<c_int> {
    if path.is_null() || buf.is_null() {
        return None;
    }

    let _guard = ShimGuard::enter()?;
    let state = ShimState::get()?;

    let path_str = CStr::from_ptr(path).to_str().ok()?;

    // Check if in VFS domain (O(1) prefix check)
    if !state.psfs_applicable(path_str) {
        return None;
    }

    // Try Hot Stat Cache (O(1) mmap lookup)
    if let Some(entry) = mmap_lookup(state.mmap_ptr, state.mmap_size, path_str) {
        std::ptr::write_bytes(buf, 0, 1);
        (*buf).st_size = entry.size as i64;
        (*buf).st_mode = entry.mode as u16;
        (*buf).st_mtime = entry.mtime;
        (*buf).st_dev = 0x52494654; // "RIFT"
        (*buf).st_nlink = 1;
        (*buf).st_ino = vrift_ipc::fnv1a_hash(path_str);
        return Some(0);
    }

    // Try IPC query
    if let Some(entry) = state.query_manifest(path_str) {
        std::ptr::write_bytes(buf, 0, 1);
        (*buf).st_size = entry.size as i64;
        (*buf).st_mode = entry.mode as u16;
        (*buf).st_mtime = entry.mtime as i64;
        (*buf).st_dev = 0x52494654; // "RIFT"
        (*buf).st_nlink = 1;
        (*buf).st_ino = vrift_ipc::fnv1a_hash(path_str);
        return Some(0);
    }

    // RFC-0047: Fallback for VFS files without manifest entry
    // Call real stat, then patch st_dev to mark as VFS file
    let ret = real_stat(path, buf);
    if ret == 0 {
        (*buf).st_dev = 0x52494654; // "RIFT"
                                    // Keep real st_ino from stat
    }
    Some(ret)
}

// Linux stat implementation disabled - needs platform-specific handling
// TODO: Implement Linux stat with same pattern

#[no_mangle]
#[cfg(target_os = "macos")]
pub unsafe extern "C" fn stat_shim(path: *const c_char, buf: *mut libc_stat) -> c_int {
    let real = std::mem::transmute::<
        *const (),
        unsafe extern "C" fn(*const c_char, *mut libc_stat) -> c_int,
    >(IT_STAT.old_func);
    if INITIALIZING.load(Ordering::Relaxed) {
        return real(path, buf);
    }
    stat_impl(path, buf, real).unwrap_or_else(|| real(path, buf))
}

#[no_mangle]
#[cfg(target_os = "macos")]
pub unsafe extern "C" fn lstat_shim(path: *const c_char, buf: *mut libc_stat) -> c_int {
    let real = std::mem::transmute::<
        *const (),
        unsafe extern "C" fn(*const c_char, *mut libc_stat) -> c_int,
    >(IT_LSTAT.old_func);
    if INITIALIZING.load(Ordering::Relaxed) {
        return real(path, buf);
    }
    stat_impl(path, buf, real).unwrap_or_else(|| real(path, buf))
}

#[no_mangle]
#[cfg(target_os = "macos")]
pub unsafe extern "C" fn fstat_shim(fd: c_int, buf: *mut libc_stat) -> c_int {
    let real = std::mem::transmute::<*const (), unsafe extern "C" fn(c_int, *mut libc_stat) -> c_int>(
        IT_FSTAT.old_func,
    );
    if INITIALIZING.load(Ordering::Relaxed) {
        return real(fd, buf);
    }

    // For fstat, we need to resolve fd -> path, which requires tracking open files
    // For now, passthrough to real syscall
    // TODO: Implement fd->path resolution when open() tracking is added
    real(fd, buf)
}

/// RFC-0049: Stub for access syscall - checks file accessibility
/// Returns 0 if accessible, -1 with errno otherwise
#[no_mangle]
#[cfg(target_os = "macos")]
pub unsafe extern "C" fn access_shim(path: *const c_char, mode: c_int) -> c_int {
    let real = std::mem::transmute::<*const (), unsafe extern "C" fn(*const c_char, c_int) -> c_int>(
        IT_ACCESS.old_func,
    );
    if INITIALIZING.load(Ordering::Relaxed) {
        return real(path, mode);
    }

    if path.is_null() {
        return real(path, mode);
    }

    // Get shim state
    let state = match ShimState::get() {
        Some(s) => s,
        None => return real(path, mode),
    };

    let path_str = match CStr::from_ptr(path).to_str() {
        Ok(s) => s,
        Err(_) => return real(path, mode),
    };

    // Check if in VFS domain
    if !state.psfs_applicable(path_str) {
        return real(path, mode);
    }

    // For VFS files, check if entry exists in manifest
    if state.query_manifest(path_str).is_some() {
        return 0; // File exists and is accessible
    }

    real(path, mode)
}
