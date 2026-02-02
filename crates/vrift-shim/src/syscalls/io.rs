//! FD Tracking and I/O syscall shims
//!
//! Provides file descriptor tracking for VFS files, enabling proper
//! handling of dup/dup2, fchdir, lseek, ftruncate, etc.

use libc::c_int;
#[cfg(target_os = "macos")]
use libc::off_t;
use std::collections::HashMap;
use std::sync::RwLock;

#[cfg(target_os = "macos")]
use crate::interpose::{IT_DUP, IT_DUP2, IT_FCHDIR, IT_FTRUNCATE, IT_LSEEK};
#[cfg(target_os = "macos")]
use crate::state::INITIALIZING;
#[cfg(target_os = "macos")]
use std::sync::atomic::Ordering;

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
    let real =
        std::mem::transmute::<*const (), unsafe extern "C" fn(c_int) -> c_int>(IT_DUP.old_func);

    if INITIALIZING.load(Ordering::Relaxed) {
        return real(oldfd);
    }

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
    let real = std::mem::transmute::<*const (), unsafe extern "C" fn(c_int, c_int) -> c_int>(
        IT_DUP2.old_func,
    );

    if INITIALIZING.load(Ordering::Relaxed) {
        return real(oldfd, newfd);
    }

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
    let real =
        std::mem::transmute::<*const (), unsafe extern "C" fn(c_int) -> c_int>(IT_FCHDIR.old_func);

    if INITIALIZING.load(Ordering::Relaxed) {
        return real(fd);
    }

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
    let real = std::mem::transmute::<*const (), unsafe extern "C" fn(c_int, off_t, c_int) -> off_t>(
        IT_LSEEK.old_func,
    );

    if INITIALIZING.load(Ordering::Relaxed) {
        return real(fd, offset, whence);
    }

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
    let real = std::mem::transmute::<*const (), unsafe extern "C" fn(c_int, off_t) -> c_int>(
        IT_FTRUNCATE.old_func,
    );

    if INITIALIZING.load(Ordering::Relaxed) {
        return real(fd, length);
    }

    // ftruncate works on the underlying file (CoW copy)
    // The Manifest update happens on close
    real(fd, length)
}
