//! FD Tracking and I/O syscall shims
//!
//! Provides file descriptor tracking for VFS files, enabling proper
//! handling of dup/dup2, fchdir, lseek, ftruncate, etc.

#[cfg(target_os = "macos")]
use crate::state::ShimGuard;
use libc::c_int;
#[cfg(target_os = "macos")]
use libc::c_void;
#[cfg(target_os = "macos")]
use libc::off_t;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Global counter for open FDs to monitor saturation (RFC-0051)
pub static OPEN_FD_COUNT: AtomicUsize = AtomicUsize::new(0);

// RFC-0051 / Pattern 2648: Lock-Free FD tracking via Tiered Atomic Array.
// The legacy Mutex-protected Map is replaced by REACTOR.fd_table.

#[derive(Clone, Debug)]
pub struct FdEntry {
    pub path: String,
    pub is_vfs: bool,
}

// RFC-0051 / Pattern 2648: Using Mutex for FD_TABLE to avoid RwLock hazards during dyld bootstrap.
// Mutation (track_fd) and Read (get_fd_entry) ratio is balanced, but safety is paramount.

/// Track a new FD opened for a VFS path
#[inline(always)]
pub fn track_fd(fd: c_int, path: &str, is_vfs: bool) {
    if fd < 0 {
        return;
    }

    let entry = Box::into_raw(Box::new(FdEntry {
        path: path.to_string(),
        is_vfs,
    }));

    // Safety: Reactor is guaranteed to be initialized after ShimState::init()
    unsafe {
        let reactor = crate::sync::get_reactor_unchecked();
        let old = reactor.fd_table.set(fd as u32, entry);
        if !old.is_null() {
            // Push old entry to RingBuffer for safe reclamation by Worker
            let _ = reactor
                .ring_buffer
                .push(crate::sync::Task::ReclaimFd(fd as u32, old));
        }
    }
}

/// Stop tracking an FD
#[inline(always)]
pub fn untrack_fd(fd: c_int) {
    if fd < 0 {
        return;
    }
    unsafe {
        let reactor = crate::sync::get_reactor_unchecked();
        let old = reactor.fd_table.remove(fd as u32);
        if !old.is_null() {
            let _ = reactor
                .ring_buffer
                .push(crate::sync::Task::ReclaimFd(fd as u32, old));
        }
    }
}

/// Get a copy of the FD entry if it exists
#[inline(always)]
pub fn get_fd_entry(fd: c_int) -> Option<FdEntry> {
    if fd < 0 {
        return None;
    }
    unsafe {
        let reactor = crate::sync::get_reactor_unchecked();
        let entry_ptr = reactor.fd_table.get(fd as u32);
        if !entry_ptr.is_null() {
            // Safety: We assume the grace period in the RingBuffer is sufficient
            // to prevent UAF during this clone.
            return Some((&*entry_ptr).clone());
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
    // BUG-007: Use raw syscall during early init OR when shim not fully ready
    // to avoid dlsym recursion and TLS pthread deadlock
    let init_state = crate::state::INITIALIZING.load(std::sync::atomic::Ordering::Relaxed);
    if init_state >= 2
        || crate::state::SHIM_STATE
            .load(std::sync::atomic::Ordering::Acquire)
            .is_null()
    {
        return crate::syscalls::macos_raw::raw_dup(oldfd);
    }

    let _guard = match ShimGuard::enter() {
        Some(g) => g,
        None => return crate::syscalls::macos_raw::raw_dup(oldfd),
    };

    let newfd = crate::syscalls::macos_raw::raw_dup(oldfd);
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
    // BUG-007: Use raw syscall during early init OR when shim not fully ready
    // to avoid dlsym recursion and TLS pthread deadlock
    let init_state = crate::state::INITIALIZING.load(std::sync::atomic::Ordering::Relaxed);
    if init_state >= 2
        || crate::state::SHIM_STATE
            .load(std::sync::atomic::Ordering::Acquire)
            .is_null()
    {
        return crate::syscalls::macos_raw::raw_dup2(oldfd, newfd);
    }

    let _guard = match ShimGuard::enter() {
        Some(g) => g,
        None => return crate::syscalls::macos_raw::raw_dup2(oldfd, newfd),
    };

    // If newfd was tracked, untrack it (it's being replaced)
    untrack_fd(newfd);

    let result = crate::syscalls::macos_raw::raw_dup2(oldfd, newfd);
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
    // RFC-0051: Always use raw syscall for core I/O shims on macOS to avoid dlsym deadlocks
    crate::syscalls::macos_raw::raw_write(fd, buf, count)
}

#[cfg(target_os = "macos")]
#[no_mangle]
pub unsafe extern "C" fn read_shim(
    fd: c_int,
    buf: *mut c_void,
    count: libc::size_t,
) -> libc::ssize_t {
    // RFC-0051: Always use raw syscall for core I/O shims on macOS to avoid dlsym deadlocks
    crate::syscalls::macos_raw::raw_read(fd, buf, count)
}

#[cfg(target_os = "macos")]
#[no_mangle]
pub unsafe extern "C" fn close_shim(fd: c_int) -> c_int {
    use crate::state::{EventType, ShimGuard, ShimState};

    // BUG-007 / RFC-0051: Use raw syscall to completely bypass libc/dlsym during critical phases.
    let init_state = crate::state::INITIALIZING.load(std::sync::atomic::Ordering::Relaxed);
    if init_state >= 2 || crate::state::CIRCUIT_TRIPPED.load(std::sync::atomic::Ordering::Relaxed) {
        return crate::syscalls::macos_raw::raw_close(fd);
    }

    let _guard = match ShimGuard::enter() {
        Some(g) => g,
        None => return crate::syscalls::macos_raw::raw_close(fd),
    };

    let state = match ShimState::get() {
        Some(s) => s,
        None => return crate::syscalls::macos_raw::raw_close(fd),
    };

    // RFC-0051: Monitor FD usage on close (to reset warning thresholds)
    let _ = crate::syscalls::io::OPEN_FD_COUNT.fetch_update(
        Ordering::Relaxed,
        Ordering::Relaxed,
        |val| Some(val.saturating_sub(1)),
    );
    state.check_fd_usage();

    // Check if this FD is a COW session
    let cow_info = {
        let mut fds = state.open_fds.lock();
        fds.remove(&fd)
    };

    // Use a hash of the FD or 0 if not tracked for general close event
    let file_id = 0; // Simplified for general close
    vfs_record!(EventType::Close, file_id, fd);

    // Final close of the file
    let res = crate::syscalls::macos_raw::raw_close(fd);

    // Offload IPC task to Worker (asynchronous)
    if let Some(info) = cow_info {
        vfs_log!(
            "COW CLOSE: fd={} vpath='{}' temp='{}'",
            fd,
            info.vpath,
            info.temp_path
        );

        // Offload reingest to Worker (non-blocking)
        if let Some(reactor) = crate::sync::get_reactor() {
            let _ = reactor.ring_buffer.push(crate::sync::Task::Reingest {
                vpath: info.vpath.clone(),
                temp_path: info.temp_path.clone(),
            });
        }

        res
    } else {
        // Not a COW file, but might be a VFS read-only file or non-VFS file
        untrack_fd(fd);
        crate::syscalls::macos_raw::raw_close(fd)
    }
}
