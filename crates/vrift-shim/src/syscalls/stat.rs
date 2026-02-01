use crate::interpose::*;
use crate::path::*;
use crate::state::*;
use libc::{c_char, c_int, c_long};
use std::ffi::CStr;
use std::ptr;
#[cfg(target_os = "linux")]
use std::sync::atomic::AtomicPtr;
use std::sync::atomic::Ordering;

// ============================================================================
// Stat Implementation
// ============================================================================

type StatFn = unsafe extern "C" fn(*const c_char, *mut libc::stat) -> c_int;
type FstatFn = unsafe extern "C" fn(c_int, *mut libc::stat) -> c_int;
type AccessFn = unsafe extern "C" fn(*const c_char, c_int) -> c_int;
type FstatatFn = unsafe extern "C" fn(c_int, *const c_char, *mut libc::stat, c_int) -> c_int;
type FaccessatFn = unsafe extern "C" fn(c_int, *const c_char, c_int, c_int) -> c_int;

unsafe fn stat_common(path: *const c_char, buf: *mut libc::stat) -> Option<c_int> {
    // Early bailout during ShimState initialization
    if INITIALIZING.load(Ordering::SeqCst) {
        return None;
    }

    // Skip if ShimState is not yet initialized (avoids malloc during dyld __malloc_init)
    if SHIM_STATE.load(Ordering::Acquire).is_null() {
        return None;
    }

    let _guard = ShimGuard::enter()?;
    let path_str = CStr::from_ptr(path).to_str().ok()?;
    let state = ShimState::get()?;

    let mut path_buf = [0u8; 1024];
    let resolved_len = (unsafe { resolve_path_with_cwd(path_str, &mut path_buf) })?;
    let resolved_path = unsafe { std::str::from_utf8_unchecked(&path_buf[..resolved_len]) };

    // RFC-0044 PSFS: VFS prefix root (special case)
    if resolved_path == state.vfs_prefix {
        ptr::write_bytes(buf, 0, 1);
        (*buf).st_mode = libc::S_IFDIR | 0o755;
        (*buf).st_nlink = 2;
        (*buf).st_dev = 0x4C4F474F5321_u64 as libc::dev_t;
        (*buf).st_uid = libc::getuid();
        (*buf).st_gid = libc::getgid();
        return Some(0);
    }

    // PHYSICAL DOMAIN CHECK (RFC-0044 Fast Path with RFC-0046 exclusions)
    if resolved_path.starts_with(&*state.vfs_prefix)
        && !resolved_path.contains("/.vrift/")
        && !resolved_path.starts_with(&*state.cas_root)
    {
        // ★ RFC-0044 Hot Stat Cache: O(1) mmap lookup (NO IPC, NO ALLOC)
        let manifest_path = if resolved_path.len() > state.vfs_prefix.len() {
            &resolved_path[state.vfs_prefix.len()..].trim_start_matches('/')
        } else {
            ""
        };

        if let Some(mmap_entry) = mmap_lookup(state.mmap_ptr, state.mmap_size, manifest_path) {
            ptr::write_bytes(buf, 0, 1);
            (*buf).st_size = mmap_entry.size as libc::off_t;
            (*buf).st_mtime = mmap_entry.mtime as libc::time_t;
            #[cfg(target_os = "macos")]
            {
                (*buf).st_mtime_nsec = mmap_entry.mtime_nsec;
            }
            #[cfg(target_os = "linux")]
            {
                (*buf).st_mtime = mmap_entry.mtime as libc::time_t;
                (*buf).st_mtime_nsec = mmap_entry.mtime_nsec;
            }
            (*buf).st_mode = mmap_entry.mode as libc::mode_t;
            if mmap_entry.is_dir() {
                (*buf).st_mode |= libc::S_IFDIR;
            } else if mmap_entry.is_symlink() {
                (*buf).st_mode |= libc::S_IFLNK;
            } else {
                (*buf).st_mode |= libc::S_IFREG;
            }
            // RFC-0049: Virtual inode to prevent collision from CAS dedup
            (*buf).st_ino = path_to_virtual_ino(manifest_path);
            (*buf).st_nlink = 1;
            (*buf).st_dev = 0x4C4F474F5321_u64 as libc::dev_t;
            (*buf).st_uid = libc::getuid();
            (*buf).st_gid = libc::getgid();
            return Some(0);
        }

        // Fallback: IPC-based manifest lookup (slower but more complete)
        if let Some(entry) = state.psfs_lookup(path_str) {
            // Fill stat buffer from manifest entry
            ptr::write_bytes(buf, 0, 1);
            let mtime_secs = (entry.mtime / 1_000_000_000) as libc::time_t;
            let mtime_nsecs = (entry.mtime % 1_000_000_000) as c_long;

            (*buf).st_size = entry.size as libc::off_t;
            (*buf).st_mtime = mtime_secs;
            // Platform-specific nanosecond field
            #[cfg(target_os = "macos")]
            {
                (*buf).st_mtime_nsec = mtime_nsecs;
            }
            #[cfg(target_os = "linux")]
            {
                (*buf).st_atime = mtime_secs;
                (*buf).st_atime_nsec = mtime_nsecs;
                (*buf).st_mtime = mtime_secs;
                (*buf).st_mtime_nsec = mtime_nsecs;
                (*buf).st_ctime = mtime_secs;
                (*buf).st_ctime_nsec = mtime_nsecs;
            }

            (*buf).st_mode = entry.mode as libc::mode_t;
            if entry.is_dir() {
                (*buf).st_mode |= libc::S_IFDIR;
            } else if entry.is_symlink() {
                (*buf).st_mode |= libc::S_IFLNK;
            } else {
                (*buf).st_mode |= libc::S_IFREG;
            }
            // RFC-0049: Virtual inode to prevent collision from CAS dedup
            if let Ok(p_str) = CStr::from_ptr(path).to_str() {
                (*buf).st_ino = path_to_virtual_ino(p_str);
            }
            (*buf).st_nlink = 1;
            (*buf).st_dev = 0x4C4F474F5321_u64 as libc::dev_t;
            (*buf).st_uid = libc::getuid();
            (*buf).st_gid = libc::getgid();
            (*buf).st_blksize = 4096;
            (*buf).st_blocks = entry.size.div_ceil(512) as libc::blkcnt_t;
            shim_log("[VRift-Shim] fstat returned virtual metadata for: ");
            shim_log(path_str);
            shim_log("\n");
            return Some(0);
        }
    }

    None
}

unsafe fn fstat_impl(fd: c_int, buf: *mut libc::stat) -> Option<c_int> {
    // Early bailout during ShimState initialization to prevent CasStore::new recursion
    if INITIALIZING.load(Ordering::SeqCst) {
        return None;
    }

    // Skip if ShimState is not yet initialized (avoids malloc during dyld __malloc_init)
    if SHIM_STATE.load(Ordering::Acquire).is_null() {
        return None;
    }

    let _guard = ShimGuard::enter()?;

    let state = ShimState::get()?;

    // Check if this fd belongs to a VFS file we're tracking
    let fds = state.open_fds.lock().unwrap();
    if let Some(open_file) = fds.get(&fd) {
        // Query manifest for this vpath to get virtual metadata
        let vpath = open_file.vpath.clone();
        drop(fds); // Release lock before IPC

        if let Some(entry) = state.query_manifest(&vpath) {
            // Return virtual metadata from manifest
            ptr::write_bytes(buf, 0, 1);
            (*buf).st_size = entry.size as libc::off_t;

            // mtime is stored as nanoseconds - convert to seconds + nanoseconds
            let mtime_secs = (entry.mtime / 1_000_000_000) as libc::time_t;
            let mtime_nsecs = (entry.mtime % 1_000_000_000) as c_long;
            (*buf).st_mtime = mtime_secs;
            // Platform-specific nanosecond field
            #[cfg(target_os = "macos")]
            {
                (*buf).st_mtime_nsec = mtime_nsecs;
            }
            #[cfg(target_os = "linux")]
            {
                (*buf).st_atime = mtime_secs;
                (*buf).st_atime_nsec = mtime_nsecs;
                (*buf).st_mtime = mtime_secs;
                (*buf).st_mtime_nsec = mtime_nsecs;
                (*buf).st_ctime = mtime_secs;
                (*buf).st_ctime_nsec = mtime_nsecs;
            }

            (*buf).st_mode = entry.mode as libc::mode_t;
            if entry.is_dir() {
                (*buf).st_mode |= libc::S_IFDIR;
            } else if entry.is_symlink() {
                (*buf).st_mode |= libc::S_IFLNK;
            } else {
                (*buf).st_mode |= libc::S_IFREG;
            }
            (*buf).st_nlink = 1;
            (*buf).st_dev = 0x4C4F474F5321_u64 as libc::dev_t;
            (*buf).st_uid = libc::getuid();
            (*buf).st_gid = libc::getgid();
            (*buf).st_blksize = 4096;
            (*buf).st_blocks = entry.size.div_ceil(512) as libc::blkcnt_t;
            shim_log("[VRift-Shim] fstat returned virtual metadata for: ");
            shim_log(&vpath);
            shim_log("\n");
            return Some(0);
        }
    } else {
        drop(fds);
    }

    None
}

unsafe fn access_impl(path: *const c_char, mode: c_int) -> Option<c_int> {
    // Early bailout during ShimState initialization
    if INITIALIZING.load(Ordering::SeqCst) {
        return None;
    }

    // Skip if ShimState is not yet initialized
    if SHIM_STATE.load(Ordering::Acquire).is_null() {
        return None;
    }

    let _guard = ShimGuard::enter()?;

    let state = ShimState::get()?;

    let path_str = CStr::from_ptr(path).to_str().ok()?;

    // Check if this is a VFS path
    if state.psfs_applicable(path_str) {
        if let Some(entry) = state.psfs_lookup(path_str) {
            // F_OK (0) = check existence
            if mode == libc::F_OK {
                return Some(0); // File exists in manifest
            }

            // Check permission bits from manifest entry mode
            let file_mode = entry.mode;

            // R_OK (4) = check read permission
            if (mode & libc::R_OK) != 0 {
                // Check user/group/other read bits
                if (file_mode & 0o444) == 0 {
                    set_errno(libc::EACCES);
                    return Some(-1);
                }
            }

            // W_OK (2) = check write permission
            // VFS files are typically read-only (hardlinked from CAS)
            if (mode & libc::W_OK) != 0 {
                // CAS files are immutable, but CoW will handle writes
                // For now, allow write checks to pass as CoW can handle it
            }

            // X_OK (1) = check execute permission
            if (mode & libc::X_OK) != 0 && (file_mode & 0o111) == 0 {
                set_errno(libc::EACCES);
                return Some(-1);
            }

            shim_log("[VRift-Shim] access() returned 0 for VFS path: ");
            shim_log(path_str);
            shim_log("\n");
            return Some(0);
        }
        // Path is in VFS prefix but not in manifest - let real syscall handle ENOENT
    }

    None
}

// ============================================================================
// Linux Shims
// ============================================================================

#[cfg(target_os = "linux")]
static REAL_STAT: AtomicPtr<c_void> = AtomicPtr::new(ptr::null_mut());
#[cfg(target_os = "linux")]
static REAL_LSTAT: AtomicPtr<c_void> = AtomicPtr::new(ptr::null_mut());
#[cfg(target_os = "linux")]
static REAL_FSTAT: AtomicPtr<c_void> = AtomicPtr::new(ptr::null_mut());
#[cfg(target_os = "linux")]
static REAL_FSTATAT: AtomicPtr<c_void> = AtomicPtr::new(ptr::null_mut());
#[cfg(target_os = "linux")]
static REAL_FACCESSAT: AtomicPtr<c_void> = AtomicPtr::new(ptr::null_mut());

#[cfg(target_os = "linux")]
#[no_mangle]
pub unsafe extern "C" fn stat(p: *const c_char, b: *mut libc::stat) -> c_int {
    let real = get_real!(REAL_STAT, "stat", StatFn);
    stat_common(p, b).unwrap_or_else(|| real(p, b))
}

#[no_mangle]
#[cfg(target_os = "linux")]
pub unsafe extern "C" fn __xstat(_ver: c_int, p: *const c_char, b: *mut libc::stat) -> c_int {
    stat(p, b)
}

#[cfg(target_os = "linux")]
#[no_mangle]
pub unsafe extern "C" fn lstat(p: *const c_char, b: *mut libc::stat) -> c_int {
    let real = get_real!(REAL_LSTAT, "lstat", StatFn);
    stat_common(p, b).unwrap_or_else(|| real(p, b))
}

#[no_mangle]
#[cfg(target_os = "linux")]
pub unsafe extern "C" fn __lxstat(_ver: c_int, p: *const c_char, b: *mut libc::stat) -> c_int {
    lstat(p, b)
}

#[cfg(target_os = "linux")]
#[no_mangle]
pub unsafe extern "C" fn fstat(fd: c_int, b: *mut libc::stat) -> c_int {
    let real = get_real!(REAL_FSTAT, "fstat", FstatFn);
    fstat_impl(fd, b).unwrap_or_else(|| real(fd, b))
}

#[no_mangle]
#[cfg(target_os = "linux")]
pub unsafe extern "C" fn __fxstat(_ver: c_int, fd: c_int, b: *mut libc::stat) -> c_int {
    fstat(fd, b)
}

#[cfg(target_os = "linux")]
#[no_mangle]
pub unsafe extern "C" fn fstatat(
    dirfd: c_int,
    p: *const c_char,
    b: *mut libc::stat,
    f: c_int,
) -> c_int {
    let real = get_real!(REAL_FSTATAT, "fstatat", FstatatFn);
    stat_common(p, b).unwrap_or_else(|| real(dirfd, p, b, f))
}

#[cfg(target_os = "linux")]
#[no_mangle]
pub unsafe extern "C" fn fstatat64(
    dirfd: c_int,
    p: *const c_char,
    b: *mut libc::stat,
    f: c_int,
) -> c_int {
    fstatat(dirfd, p, b, f)
}

#[cfg(target_os = "linux")]
#[no_mangle]
pub unsafe extern "C" fn faccessat(dirfd: c_int, p: *const c_char, m: c_int, f: c_int) -> c_int {
    let real = get_real!(REAL_FACCESSAT, "faccessat", FaccessatFn);
    access_impl(p, m).unwrap_or_else(|| real(dirfd, p, m, f))
}

// ============================================================================
// macOS Shims
// ============================================================================

#[cfg(target_os = "macos")]
#[no_mangle]
pub unsafe extern "C" fn stat_shim(p: *const c_char, b: *mut libc::stat) -> c_int {
    // Use IT_STAT.old_func to get the real libc stat, avoiding recursion
    let real = std::mem::transmute::<*const (), StatFn>(IT_STAT.old_func);
    stat_common(p, b).unwrap_or_else(|| real(p, b))
}

#[cfg(target_os = "macos")]
#[no_mangle]
pub unsafe extern "C" fn lstat_shim(p: *const c_char, b: *mut libc::stat) -> c_int {
    let real = std::mem::transmute::<*const (), StatFn>(IT_LSTAT.old_func);
    stat_common(p, b).unwrap_or_else(|| real(p, b))
}

#[cfg(target_os = "macos")]
#[no_mangle]
pub unsafe extern "C" fn fstat_shim(fd: c_int, b: *mut libc::stat) -> c_int {
    let real = std::mem::transmute::<*const (), FstatFn>(IT_FSTAT.old_func);
    fstat_impl(fd, b).unwrap_or_else(|| real(fd, b))
}

#[cfg(target_os = "macos")]
#[no_mangle]
pub unsafe extern "C" fn access_shim(path: *const c_char, mode: c_int) -> c_int {
    let real = std::mem::transmute::<*const (), AccessFn>(IT_ACCESS.old_func);
    access_impl(path, mode).unwrap_or_else(|| real(path, mode))
}

#[cfg(target_os = "macos")]
#[no_mangle]
pub unsafe extern "C" fn faccessat_shim(
    dirfd: c_int,
    pathname: *const c_char,
    mode: c_int,
    flags: c_int,
) -> c_int {
    // Passthrough to real faccessat - permission checks work on underlying files
    let real = std::mem::transmute::<*const (), FaccessatFn>(IT_FACCESSAT.old_func);
    real(dirfd, pathname, mode, flags)
}

#[cfg(target_os = "macos")]
#[no_mangle]
pub unsafe extern "C" fn fstatat_shim(
    dirfd: c_int,
    pathname: *const c_char,
    buf: *mut libc::stat,
    flags: c_int,
) -> c_int {
    // Passthrough to real fstatat - stat operations handled via stat/lstat shims
    let real = std::mem::transmute::<*const (), FstatatFn>(IT_FSTATAT.old_func);
    real(dirfd, pathname, buf, flags)
}

#[cfg(target_os = "linux")]
unsafe fn set_errno(e: c_int) {
    *libc::__errno_location() = e;
}
#[cfg(target_os = "macos")]
unsafe fn set_errno(e: c_int) {
    *libc::__error() = e;
}
