//! FD Tracking and I/O syscall shims
//!
//! Provides file descriptor tracking for VFS files, enabling proper
//! handling of dup/dup2, fchdir, lseek, ftruncate, etc.

use libc::c_int;
#[cfg(target_os = "macos")]
use libc::c_void;
#[cfg(target_os = "macos")]
use libc::off_t;
use std::collections::HashMap;
use std::sync::RwLock;

// Symbols imported from reals.rs via crate::reals
/// Global FD tracking table: fd -> (path, is_vfs_file)
static FD_TABLE: RwLock<Option<HashMap<c_int, FdEntry>>> = RwLock::new(None);

#[derive(Clone, Debug)]
pub struct FdEntry {
    pub path: String,
    pub is_vfs: bool,
}

/// Initialize FD table if not already done
fn ensure_fd_table() -> bool {
    let mut table = match FD_TABLE.write() {
        Ok(t) => t,
        Err(_) => return false,
    };
    if table.is_none() {
        *table = Some(HashMap::new());
    }
    true
}

/// Track a new FD opened for a VFS path
pub fn track_fd(fd: c_int, path: &str, is_vfs: bool) {
    if fd < 0 {
        return;
    }
    if !ensure_fd_table() {
        return;
    }
    if let Ok(mut table) = FD_TABLE.write() {
        if let Some(ref mut map) = *table {
            map.insert(
                fd,
                FdEntry {
                    path: path.to_string(),
                    is_vfs,
                },
            );
        }
    }
}

/// Untrack FD on close
pub fn untrack_fd(fd: c_int) {
    if fd < 0 {
        return;
    }
    if let Ok(mut table) = FD_TABLE.write() {
        if let Some(ref mut map) = *table {
            map.remove(&fd);
        }
    }
}

/// Get entry for an FD
pub fn get_fd_entry(fd: c_int) -> Option<FdEntry> {
    if fd < 0 {
        return None;
    }
    if let Ok(table) = FD_TABLE.read() {
        if let Some(ref map) = *table {
            return map.get(&fd).cloned();
        }
    }
    None
}

/// Check if FD is a VFS file
pub fn is_vfs_fd(fd: c_int) -> bool {
    get_fd_entry(fd).map(|e| e.is_vfs).unwrap_or(false)
}

// ============================================================================
// dup/dup2 shims - copy FD tracking on duplicate
// ============================================================================

#[cfg(target_os = "macos")]
#[no_mangle]
pub unsafe extern "C" fn dup_shim(oldfd: c_int) -> c_int {
    let real = std::mem::transmute::<*mut libc::c_void, unsafe extern "C" fn(c_int) -> c_int>(
        crate::reals::REAL_DUP.get(),
    );

    passthrough_if_init!(real, oldfd);

    let newfd = real(oldfd);
    if newfd >= 0 {
        // Copy tracking from oldfd to newfd
        if let Some(entry) = get_fd_entry(oldfd) {
            track_fd(newfd, &entry.path, entry.is_vfs);
        }
    }
    newfd
}

#[cfg(target_os = "macos")]
#[no_mangle]
pub unsafe extern "C" fn dup2_shim(oldfd: c_int, newfd: c_int) -> c_int {
    let real = std::mem::transmute::<*mut libc::c_void, unsafe extern "C" fn(c_int, c_int) -> c_int>(
        crate::reals::REAL_DUP2.get(),
    );

    passthrough_if_init!(real, oldfd, newfd);

    // If newfd was tracked, untrack it (it's being replaced)
    untrack_fd(newfd);

    let result = real(oldfd, newfd);
    if result >= 0 {
        // Copy tracking from oldfd to newfd
        if let Some(entry) = get_fd_entry(oldfd) {
            track_fd(result, &entry.path, entry.is_vfs);
        }
    }
    result
}

// ============================================================================
// fchdir shim - update virtual CWD from FD
// ============================================================================

#[cfg(target_os = "macos")]
#[no_mangle]
pub unsafe extern "C" fn fchdir_shim(fd: c_int) -> c_int {
    let real = std::mem::transmute::<*mut libc::c_void, unsafe extern "C" fn(c_int) -> c_int>(
        crate::reals::REAL_FCHDIR.get(),
    );

    passthrough_if_init!(real, fd);

    // If fd points to a VFS directory, we could update virtual CWD here
    // For now, just passthrough but track
    // TODO: Update virtual CWD tracking if fd is a VFS directory
    // This requires the VFS CWD infrastructure from chdir_shim

    real(fd)
}

// ============================================================================
// lseek shim - passthrough with tracking
// ============================================================================

#[cfg(target_os = "macos")]
#[no_mangle]
pub unsafe extern "C" fn lseek_shim(fd: c_int, offset: off_t, whence: c_int) -> off_t {
    let real = std::mem::transmute::<
        *mut libc::c_void,
        unsafe extern "C" fn(c_int, off_t, c_int) -> off_t,
    >(crate::reals::REAL_LSEEK.get());

    passthrough_if_init!(real, fd, offset, whence);

    // lseek works on the underlying file, which is correct for VFS
    // (VFS files are extracted to temp, so lseek on the temp file is correct)
    real(fd, offset, whence)
}

// ============================================================================
// ftruncate shim - truncate VFS file's CoW copy
// ============================================================================

#[cfg(target_os = "macos")]
#[no_mangle]
pub unsafe extern "C" fn ftruncate_shim(fd: c_int, length: off_t) -> c_int {
    let real = std::mem::transmute::<*mut libc::c_void, unsafe extern "C" fn(c_int, off_t) -> c_int>(
        crate::reals::REAL_FTRUNCATE.get(),
    );

    passthrough_if_init!(real, fd, length);

    // ftruncate works on the underlying file (CoW copy)
    // The Manifest update happens on close
    real(fd, length)
}

// ============================================================================
// close shim - untrack and trigger COW reingest
// ============================================================================

#[cfg(target_os = "macos")]
#[no_mangle]
pub unsafe extern "C" fn write_shim(
    fd: c_int,
    buf: *const c_void,
    count: libc::size_t,
) -> libc::ssize_t {
    // BUG-007: Use raw syscall during early init to avoid dlsym recursion
    let init_state = crate::state::INITIALIZING.load(std::sync::atomic::Ordering::Relaxed);
    if init_state >= 2 {
        return crate::syscalls::macos_raw::raw_write(fd, buf, count);
    }

    let real = std::mem::transmute::<
        *mut libc::c_void,
        unsafe extern "C" fn(c_int, *const c_void, libc::size_t) -> libc::ssize_t,
    >(crate::reals::REAL_WRITE.get());
    real(fd, buf, count)
}

#[cfg(target_os = "macos")]
#[no_mangle]
pub unsafe extern "C" fn read_shim(
    fd: c_int,
    buf: *mut c_void,
    count: libc::size_t,
) -> libc::ssize_t {
    // BUG-007: Use raw syscall during early init to avoid dlsym recursion
    let init_state = crate::state::INITIALIZING.load(std::sync::atomic::Ordering::Relaxed);
    if init_state >= 2 {
        return crate::syscalls::macos_raw::raw_read(fd, buf, count);
    }

    let real = std::mem::transmute::<
        *mut libc::c_void,
        unsafe extern "C" fn(c_int, *mut c_void, libc::size_t) -> libc::ssize_t,
    >(crate::reals::REAL_READ.get());
    real(fd, buf, count)
}

#[cfg(target_os = "macos")]
#[no_mangle]
pub unsafe extern "C" fn close_shim(fd: c_int) -> c_int {
    use crate::ipc::sync_ipc_manifest_reingest;
    use crate::state::{EventType, ShimGuard, ShimState};

    // BUG-007: close is called during __malloc_init before dlsym is safe.
    // Use raw syscall to completely bypass libc.
    let init_state = crate::state::INITIALIZING.load(std::sync::atomic::Ordering::Relaxed);
    if init_state >= 2 || crate::state::CIRCUIT_TRIPPED.load(std::sync::atomic::Ordering::Relaxed) {
        return crate::syscalls::macos_raw::raw_close(fd);
    }

    // After init, we can safely use dlsym-cached version
    let real = std::mem::transmute::<*mut libc::c_void, unsafe extern "C" fn(c_int) -> c_int>(
        crate::reals::REAL_CLOSE.get(),
    );

    let _guard = match ShimGuard::enter() {
        Some(g) => g,
        None => return real(fd),
    };

    let state = match ShimState::get() {
        Some(s) => s,
        None => return real(fd),
    };

    // Check if this FD is a COW session
    let cow_info = if let Ok(mut fds) = state.open_fds.lock() {
        fds.remove(&fd) // Remove from tracking on close
    } else {
        None
    };

    // Use a hash of the FD or 0 if not tracked for general close event
    let file_id = 0; // Simplified for general close
    vfs_record!(EventType::Close, file_id, fd);

    if let Some(info) = cow_info {
        vfs_log!(
            "COW CLOSE: fd={} vpath='{}' temp='{}'",
            fd,
            info.vpath,
            info.temp_path
        );

        // Final close of the temp file before reingest
        let res = real(fd);

        // Trigger reingest IPC
        // RFC-0047: ManifestReingest updates the CAS and Manifest
        if sync_ipc_manifest_reingest(&state.socket_path, &info.vpath, &info.temp_path) {
            vfs_record!(
                EventType::ReingestSuccess,
                vrift_ipc::fnv1a_hash(&info.vpath),
                res
            );
        } else {
            vfs_log!("REINGEST FAILED: IPC error for '{}'", info.vpath);
            vfs_record!(
                EventType::ReingestFail,
                vrift_ipc::fnv1a_hash(&info.vpath),
                -1
            );
        }

        // Note: info.temp_path is cleaned up by the daemon (zero-copy move)
        // or discarded if IPC failed (though that leaves an orphan temp file)

        res
    } else {
        // Not a COW file, but might be a VFS read-only file or non-VFS file
        untrack_fd(fd);
        real(fd)
    }
}
