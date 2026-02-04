#[allow(unused_imports)]
use crate::reals::*;
use crate::state::*;
use libc::{c_char, c_int, stat as libc_stat};
use std::ffi::CStr;
#[cfg(target_os = "linux")]
use std::sync::atomic::Ordering;

/// Linux statx structures (RFC-0044: Metadata virtualization)
#[cfg(target_os = "linux")]
#[repr(C)]
pub struct statx_timestamp {
    pub tv_sec: i64,
    pub tv_nsec: u32,
    pub __reserved: i32,
}

#[cfg(target_os = "linux")]
#[repr(C)]
pub struct statx {
    pub stx_mask: u32,
    pub stx_blksize: u32,
    pub stx_attributes: u64,
    pub stx_nlink: u32,
    pub stx_uid: u32,
    pub stx_gid: u32,
    pub stx_mode: u16,
    pub __spare0: [u16; 1],
    pub stx_ino: u64,
    pub stx_size: u64,
    pub stx_blocks: u64,
    pub stx_attributes_mask: u64,
    pub stx_atime: statx_timestamp,
    pub stx_btime: statx_timestamp,
    pub stx_ctime: statx_timestamp,
    pub stx_mtime: statx_timestamp,
    pub stx_rdev_major: u32,
    pub stx_rdev_minor: u32,
    pub stx_dev_major: u32,
    pub stx_dev_minor: u32,
    pub __spare2: [u64; 14],
}

/// RFC-0044: Virtual stat implementation using Hot Stat Cache
/// Returns None to fallback to OS, Some(0) on success, Some(-1) on error
unsafe fn stat_impl_common(path_str: &str, buf: *mut libc_stat) -> Option<c_int> {
    let state = ShimState::get()?;

    // Check if in VFS domain (O(1) prefix check)
    if !state.psfs_applicable(path_str) {
        return None;
    }

    // Strip VFS prefix to get manifest-relative path
    // e.g., "/vrift/file_1.txt" -> "/file_1.txt"
    // Manifest stores paths WITH leading slash (e.g., "/file_1.txt")
    let manifest_path = if let Some(stripped) = path_str.strip_prefix(state.vfs_prefix.as_ref()) {
        stripped
    } else {
        path_str // Fallback if prefix doesn't match
    };

    // DEBUG: Log first 3 lookups
    static COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let count = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    if count < 3 {
        eprintln!(
            "[DEBUG] stat: path='{}' manifest_path='{}'",
            path_str, manifest_path
        );
    }

    // Try Hot Stat Cache (O(1) mmap lookup)
    if let Some(entry) = mmap_lookup(state.mmap_ptr, state.mmap_size, manifest_path) {
        if count < 3 {
            eprintln!("[DEBUG] mmap HIT for '{}'", manifest_path);
        }
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

    if count < 3 {
        eprintln!("[DEBUG] mmap MISS for '{}', trying IPC", manifest_path);
    }

    // Try IPC query (also use manifest path format)
    if let Some(entry) = state.query_manifest(manifest_path) {
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
pub unsafe extern "C" fn velo_stat_impl(path: *const c_char, buf: *mut libc_stat) -> c_int {
    stat_impl(path, buf, true).unwrap_or_else(|| crate::syscalls::macos_raw::raw_stat(path, buf))
}

#[no_mangle]
#[cfg(target_os = "macos")]
pub unsafe extern "C" fn stat_shim(path: *const c_char, buf: *mut libc_stat) -> c_int {
    // Standard interpose entry point
    let init_state = crate::state::INITIALIZING.load(std::sync::atomic::Ordering::Relaxed);
    if init_state != 0 {
        return crate::syscalls::macos_raw::raw_stat(path, buf);
    }
    velo_stat_impl(path, buf)
}

#[no_mangle]
#[cfg(target_os = "macos")]
pub unsafe extern "C" fn velo_lstat_impl(path: *const c_char, buf: *mut libc_stat) -> c_int {
    stat_impl(path, buf, false).unwrap_or_else(|| crate::syscalls::macos_raw::raw_lstat(path, buf))
}

#[no_mangle]
#[cfg(target_os = "macos")]
pub unsafe extern "C" fn lstat_shim(path: *const c_char, buf: *mut libc_stat) -> c_int {
    let init_state = crate::state::INITIALIZING.load(std::sync::atomic::Ordering::Relaxed);
    if init_state != 0 {
        return crate::syscalls::macos_raw::raw_lstat(path, buf);
    }
    velo_lstat_impl(path, buf)
}

#[no_mangle]
#[cfg(target_os = "macos")]
pub unsafe extern "C" fn velo_fstat_impl(fd: c_int, buf: *mut libc_stat) -> c_int {
    // 🔥 ULTRA-FAST PATH: No ShimGuard for common case
    if let Some(reactor) = crate::sync::get_reactor() {
        let entry_ptr = reactor.fd_table.get(fd as u32);
        if !entry_ptr.is_null() {
            let entry = &*entry_ptr;
            if let Some(ref cached) = entry.cached_stat {
                *buf = *cached;
                return 0;
            }
        }
    }

    let _guard = match ShimGuard::enter() {
        Some(g) => g,
        None => return crate::syscalls::macos_raw::raw_fstat64(fd, buf),
    };

    if let Some(entry) = crate::syscalls::io::get_fd_entry(fd) {
        if let Some(ref cached) = entry.cached_stat {
            *buf = *cached;
            return 0;
        }

        if entry.is_vfs {
            if let Some(state) = ShimState::get() {
                if let Some(vnode) = state.query_manifest(&entry.path) {
                    std::ptr::write_bytes(buf, 0, 1);
                    (*buf).st_size = vnode.size as _;
                    (*buf).st_mode = vnode.mode as u16;
                    (*buf).st_mtime = vnode.mtime as _;
                    (*buf).st_dev = 0x52494654;
                    (*buf).st_nlink = 1;
                    (*buf).st_ino = vrift_ipc::fnv1a_hash(&entry.path) as _;
                    vfs_record!(EventType::StatHit, (*buf).st_ino, 0);
                    return 0;
                }
            }
        }
    }

    crate::syscalls::macos_raw::raw_fstat64(fd, buf)
}

#[no_mangle]
#[cfg(target_os = "macos")]
pub unsafe extern "C" fn fstat_shim(fd: c_int, buf: *mut libc_stat) -> c_int {
    let init_state = crate::state::INITIALIZING.load(std::sync::atomic::Ordering::Relaxed);
    if init_state != 0 {
        return crate::syscalls::macos_raw::raw_fstat64(fd, buf);
    }
    velo_fstat_impl(fd, buf)
}

#[no_mangle]
#[cfg(target_os = "macos")]
pub unsafe extern "C" fn velo_access_impl(path: *const c_char, mode: c_int) -> c_int {
    // Use raw syscall for fallback to avoid dlsym deadlock (Pattern 2682.v2)
    let _guard = match ShimGuard::enter() {
        Some(g) => g,
        None => return crate::syscalls::macos_raw::raw_access(path, mode),
    };

    let path_str = match CStr::from_ptr(path).to_str() {
        Ok(s) => s,
        Err(_) => return crate::syscalls::macos_raw::raw_access(path, mode),
    };

    if ShimState::get()
        .map(|s| s.psfs_applicable(path_str))
        .unwrap_or(false)
    {
        return 0;
    }

    crate::syscalls::macos_raw::raw_access(path, mode)
}

#[no_mangle]
#[cfg(target_os = "macos")]
pub unsafe extern "C" fn access_shim(path: *const c_char, mode: c_int) -> c_int {
    // BUG-007: Use raw syscall during early init to avoid recursion
    let init_state = crate::state::INITIALIZING.load(std::sync::atomic::Ordering::Relaxed);
    if init_state != 0 || crate::state::CIRCUIT_TRIPPED.load(std::sync::atomic::Ordering::Relaxed) {
        return crate::syscalls::macos_raw::raw_access(path, mode);
    }

    velo_access_impl(path, mode)
}

#[no_mangle]
#[cfg(target_os = "macos")]
pub unsafe extern "C" fn velo_fstatat_impl(
    dirfd: c_int,
    path: *const c_char,
    buf: *mut libc_stat,
    flags: c_int,
) -> c_int {
    // Use raw syscall for fallback to avoid dlsym deadlock (Pattern 2682.v2)
    let _guard = match ShimGuard::enter() {
        Some(g) => g,
        None => return crate::syscalls::macos_raw::raw_fstatat64(dirfd, path, buf, flags),
    };

    if dirfd == libc::AT_FDCWD || (!path.is_null() && *path == b'/' as i8) {
        if let Ok(path_str) = CStr::from_ptr(path).to_str() {
            if let Some(res) = stat_impl_common(path_str, buf) {
                return res;
            }
        }
    }

    crate::syscalls::macos_raw::raw_fstatat64(dirfd, path, buf, flags)
}

#[no_mangle]
#[cfg(target_os = "macos")]
pub unsafe extern "C" fn fstatat_shim(
    dirfd: c_int,
    path: *const c_char,
    buf: *mut libc_stat,
    flags: c_int,
) -> c_int {
    // BUG-007 / RFC-0051: Use raw syscall during early init to avoid dlsym recursion.
    // Also check if SHIM_STATE is null to avoid TLS deadlock hazards.
    let init_state = crate::state::INITIALIZING.load(std::sync::atomic::Ordering::Relaxed);
    if init_state != 0
        || crate::state::SHIM_STATE
            .load(std::sync::atomic::Ordering::Acquire)
            .is_null()
    {
        return crate::syscalls::macos_raw::raw_fstatat64(dirfd, path, buf, flags);
    }

    velo_fstatat_impl(dirfd, path, buf, flags)
}

/// Linux-specific fstatat shim called from interpose bridge
#[cfg(target_os = "linux")]
pub unsafe fn fstatat_shim_linux(
    dirfd: c_int,
    path: *const c_char,
    buf: *mut libc_stat,
    _flags: c_int,
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

    stat_impl_common(path_str, buf).unwrap_or(-2)
}

#[no_mangle]
#[cfg(target_os = "linux")]
pub unsafe extern "C" fn statx_shim(
    dirfd: c_int,
    path: *const c_char,
    flags: c_int,
    mask: libc::c_uint,
    buf: *mut statx,
) -> c_int {
    if path.is_null() {
        return crate::syscalls::linux_raw::raw_statx(
            dirfd,
            path,
            flags,
            mask,
            buf as *mut libc::c_void,
        );
    }

    let init_state = INITIALIZING.load(Ordering::Relaxed);
    if init_state != 0 || crate::state::SHIM_STATE.load(Ordering::Acquire).is_null() {
        return crate::syscalls::linux_raw::raw_statx(
            dirfd,
            path,
            flags,
            mask,
            buf as *mut libc::c_void,
        );
    }

    let path_str = match CStr::from_ptr(path).to_str() {
        Ok(s) => s,
        Err(_) => {
            return crate::syscalls::linux_raw::raw_statx(
                dirfd,
                path,
                flags,
                mask,
                buf as *mut libc::c_void,
            )
        }
    };

    // VFS lookup
    if let Some(state) = ShimState::get() {
        if state.psfs_applicable(path_str) {
            if let Some(entry) = state.query_manifest(path_str) {
                std::ptr::write_bytes(buf, 0, 1);
                (*buf).stx_mask = 0x7FF; // basic stats
                (*buf).stx_size = entry.size as _;
                (*buf).stx_mode = entry.mode as _;
                (*buf).stx_ino = vrift_ipc::fnv1a_hash(path_str) as _;
                (*buf).stx_nlink = 1;
                (*buf).stx_mtime.tv_sec = entry.mtime as _;
                (*buf).stx_blksize = 4096;
                (*buf).stx_blocks = entry.size.div_ceil(512);
                vfs_record!(EventType::StatHit, (*buf).stx_ino, 0);
                return 0;
            }
        }
    }

    crate::syscalls::linux_raw::raw_statx(dirfd, path, flags, mask, buf as *mut libc::c_void)
}
