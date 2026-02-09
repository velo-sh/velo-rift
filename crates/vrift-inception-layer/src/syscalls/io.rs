//! FD Tracking and I/O syscall inception layers
//!
//! Provides file descriptor tracking for VFS files, enabling proper
//! handling of dup/dup2, fchdir, lseek, ftruncate, etc.

use crate::state::InceptionLayerGuard;
use libc::{c_int, c_void, off_t, size_t, ssize_t};
use std::sync::atomic::AtomicUsize;

/// Global counter for open FDs to monitor saturation (RFC-0051)
pub static OPEN_FD_COUNT: AtomicUsize = AtomicUsize::new(0);

// RFC-0051 / Pattern 2648: Lock-Free FD tracking via Tiered Atomic Array.
// The legacy Mutex-protected Map is replaced by REACTOR.fd_table.

#[derive(Clone, Debug)]
pub struct FdEntry {
    pub vpath: crate::state::FixedString<1024>,
    pub manifest_key: crate::state::FixedString<1024>,
    pub manifest_key_hash: u64,
    pub temp_path: crate::state::FixedString<1024>,
    pub is_vfs: bool,
    pub cached_stat: Option<libc::stat>,
    pub mmap_count: usize,
    pub lock_fd: i32, // -1 if no lock FD held
}

// RFC-0051 / Pattern 2648: Using Mutex for FD_TABLE to avoid RwLock hazards during dyld bootstrap.
// Mutation (track_fd) and Read (get_fd_entry) ratio is balanced, but safety is paramount.

/// Track a new FD opened for a VFS path
#[inline(always)]
pub fn track_fd(
    fd: c_int,
    path: &str,
    is_vfs: bool,
    cached_stat: Option<libc::stat>,
    manifest_key_hash: u64,
) {
    if fd < 0 {
        return;
    }

    let mut vpath_fs = crate::state::FixedString::<1024>::new();
    vpath_fs.set(path);

    let entry = Box::into_raw(Box::new(FdEntry {
        vpath: vpath_fs,
        manifest_key: vpath_fs, // For now assume path is the manifest key if not otherwise specified
        manifest_key_hash,
        temp_path: crate::state::FixedString::new(),
        is_vfs,
        cached_stat,
        mmap_count: 0,
        lock_fd: -1,
    }));

    if let Some(state) = crate::state::InceptionLayerState::get() {
        let old = state.open_fds.set(fd as u32, entry);
        if !old.is_null() {
            // Push old entry to RingBuffer for safe reclamation by Worker
            if let Some(reactor) = crate::sync::get_reactor() {
                let _ = reactor
                    .ring_buffer
                    .push(crate::sync::Task::ReclaimFd(fd as u32, old));
            } else {
                unsafe { drop(Box::from_raw(old)) };
            }
        } else {
            // New entry, increment count
            OPEN_FD_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
    } else {
        unsafe { drop(Box::from_raw(entry)) };
    }
}

/// Stop tracking an FD
#[inline(always)]
pub fn untrack_fd(fd: c_int) {
    if fd < 0 {
        return;
    }
    if let Some(state) = crate::state::InceptionLayerState::get() {
        let old = state.open_fds.remove(fd as u32);
        if !old.is_null() {
            // Entry removed, decrement count
            OPEN_FD_COUNT.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);

            if let Some(reactor) = crate::sync::get_reactor() {
                let _ = reactor
                    .ring_buffer
                    .push(crate::sync::Task::ReclaimFd(fd as u32, old));
            } else {
                unsafe { drop(Box::from_raw(old)) };
            }
        }
    }
}

/// Get a copy of the FD entry if it exists
#[inline(always)]
pub fn get_fd_entry(fd: c_int) -> Option<FdEntry> {
    if fd < 0 {
        return None;
    }
    let state = crate::state::InceptionLayerState::get()?;
    let entry_ptr = state.open_fds.get(fd as u32);
    if !entry_ptr.is_null() {
        // Safety: We assume the grace period in the RingBuffer is sufficient
        // to prevent UAF during this clone.
        return unsafe { Some((&*entry_ptr).clone()) };
    }
    None
}

/// Check if FD is a VFS file
pub fn is_vfs_fd(fd: c_int) -> bool {
    get_fd_entry(fd).map(|e| e.is_vfs).unwrap_or(false)
}

// ============================================================================
// dup/dup2 inception layers - copy FD tracking on duplicate
// ============================================================================

#[no_mangle]
pub unsafe extern "C" fn dup_inception(oldfd: c_int) -> c_int {
    // BUG-007: Use raw syscall during early init OR when inception layer not fully ready
    // to avoid dlsym recursion and TLS pthread deadlock
    let init_state = crate::state::INITIALIZING.load(std::sync::atomic::Ordering::Relaxed);
    if init_state != 0
        || crate::state::INCEPTION_LAYER_STATE
            .load(std::sync::atomic::Ordering::Acquire)
            .is_null()
    {
        #[cfg(target_os = "macos")]
        return crate::syscalls::macos_raw::raw_dup(oldfd);
        #[cfg(target_os = "linux")]
        return crate::syscalls::linux_raw::raw_dup(oldfd);
    }

    let _guard = match InceptionLayerGuard::enter() {
        Some(g) => g,
        None => {
            #[cfg(target_os = "macos")]
            return crate::syscalls::macos_raw::raw_dup(oldfd);
            #[cfg(target_os = "linux")]
            return crate::syscalls::linux_raw::raw_dup(oldfd);
        }
    };

    #[cfg(target_os = "macos")]
    let newfd = crate::syscalls::macos_raw::raw_dup(oldfd);
    #[cfg(target_os = "linux")]
    let newfd = crate::syscalls::linux_raw::raw_dup(oldfd);

    if newfd >= 0 {
        // Copy tracking from oldfd to newfd
        if let Some(entry) = get_fd_entry(oldfd) {
            track_fd(
                newfd,
                entry.vpath.as_str(),
                entry.is_vfs,
                entry.cached_stat,
                entry.manifest_key_hash,
            );
        }
    }
    newfd
}

#[no_mangle]
pub unsafe extern "C" fn dup2_inception(oldfd: c_int, newfd: c_int) -> c_int {
    // BUG-007: Use raw syscall during early init OR when inception layer not fully ready
    // to avoid dlsym recursion and TLS pthread deadlock
    let init_state = crate::state::INITIALIZING.load(std::sync::atomic::Ordering::Relaxed);
    if init_state != 0
        || crate::state::INCEPTION_LAYER_STATE
            .load(std::sync::atomic::Ordering::Acquire)
            .is_null()
    {
        #[cfg(target_os = "macos")]
        return crate::syscalls::macos_raw::raw_dup2(oldfd, newfd);
        #[cfg(target_os = "linux")]
        return crate::syscalls::linux_raw::raw_dup2(oldfd, newfd);
    }

    let _guard = match InceptionLayerGuard::enter() {
        Some(g) => g,
        None => {
            #[cfg(target_os = "macos")]
            return crate::syscalls::macos_raw::raw_dup2(oldfd, newfd);
            #[cfg(target_os = "linux")]
            return crate::syscalls::linux_raw::raw_dup2(oldfd, newfd);
        }
    };

    // If newfd was tracked, untrack it (it's being replaced)
    untrack_fd(newfd);

    #[cfg(target_os = "macos")]
    let result = crate::syscalls::macos_raw::raw_dup2(oldfd, newfd);
    #[cfg(target_os = "linux")]
    let result = crate::syscalls::linux_raw::raw_dup2(oldfd, newfd);

    if result >= 0 {
        // Copy tracking from oldfd to newfd
        if let Some(entry) = get_fd_entry(oldfd) {
            track_fd(
                result,
                entry.vpath.as_str(),
                entry.is_vfs,
                entry.cached_stat,
                entry.manifest_key_hash,
            );
        }
    }
    result
}

// ============================================================================
// fchdir inception layer - update virtual CWD from FD
// ============================================================================

#[no_mangle]
pub unsafe extern "C" fn fchdir_inception(fd: c_int) -> c_int {
    let init_state = crate::state::INITIALIZING.load(std::sync::atomic::Ordering::Relaxed);
    if init_state != 0
        || crate::state::INCEPTION_LAYER_STATE
            .load(std::sync::atomic::Ordering::Acquire)
            .is_null()
    {
        #[cfg(target_os = "macos")]
        return crate::syscalls::macos_raw::raw_fchdir(fd);
        #[cfg(target_os = "linux")]
        return crate::syscalls::linux_raw::raw_fchdir(fd);
    }

    // TODO: Update virtual CWD tracking if fd is a VFS directory
    // Pattern 2930: Use raw syscall to avoid post-init dlsym hazard
    #[cfg(target_os = "macos")]
    return crate::syscalls::macos_raw::raw_fchdir(fd);
    #[cfg(target_os = "linux")]
    return crate::syscalls::linux_raw::raw_fchdir(fd);
}

// ============================================================================
// lseek inception layer - passthrough with tracking
// ============================================================================

#[no_mangle]
pub unsafe extern "C" fn lseek_inception(fd: c_int, offset: off_t, whence: c_int) -> off_t {
    // Pattern 2930: Use raw syscall to avoid post-init dlsym hazard
    #[cfg(target_os = "macos")]
    return crate::syscalls::macos_raw::raw_lseek(fd, offset, whence);
    #[cfg(target_os = "linux")]
    return crate::syscalls::linux_raw::raw_lseek(fd, offset, whence);
}

// ============================================================================
// ftruncate inception layer - truncate VFS file's CoW copy
// ============================================================================

#[no_mangle]
pub unsafe extern "C" fn ftruncate_inception(fd: c_int, length: off_t) -> c_int {
    // Pattern 2930: Use raw syscall to avoid post-init dlsym hazard
    #[cfg(target_os = "macos")]
    return crate::syscalls::macos_raw::raw_ftruncate(fd, length);
    #[cfg(target_os = "linux")]
    return crate::syscalls::linux_raw::raw_ftruncate(fd, length);
}

// ============================================================================
// close inception layer - untrack and trigger COW reingest
// ============================================================================

#[no_mangle]
pub unsafe extern "C" fn write_inception(fd: c_int, buf: *const c_void, count: size_t) -> ssize_t {
    profile_timed!(write_calls, write_ns, {
        #[cfg(target_os = "macos")]
        {
            crate::syscalls::macos_raw::raw_write(fd, buf, count)
        }
        #[cfg(target_os = "linux")]
        {
            crate::syscalls::linux_raw::raw_write(fd, buf, count)
        }
    })
}

#[no_mangle]
pub unsafe extern "C" fn read_inception(fd: c_int, buf: *mut c_void, count: size_t) -> ssize_t {
    profile_timed!(read_calls, read_ns, {
        #[cfg(target_os = "macos")]
        {
            crate::syscalls::macos_raw::raw_read(fd, buf, count)
        }
        #[cfg(target_os = "linux")]
        {
            crate::syscalls::linux_raw::raw_read(fd, buf, count)
        }
    })
}

#[no_mangle]
pub unsafe extern "C" fn close_inception(fd: c_int) -> c_int {
    profile_timed!(close_calls, close_ns, {
        use crate::state::{EventType, InceptionLayerGuard, InceptionLayerState};

        let init_state = crate::state::INITIALIZING.load(std::sync::atomic::Ordering::Relaxed);
        if init_state != 0
            || crate::state::CIRCUIT_TRIPPED.load(std::sync::atomic::Ordering::Relaxed)
        {
            #[cfg(target_os = "macos")]
            return crate::syscalls::macos_raw::raw_close(fd);
            #[cfg(target_os = "linux")]
            return crate::syscalls::linux_raw::raw_close(fd);
        }

        let _guard = match InceptionLayerGuard::enter() {
            Some(g) => g,
            None => {
                #[cfg(target_os = "macos")]
                return crate::syscalls::macos_raw::raw_close(fd);
                #[cfg(target_os = "linux")]
                return crate::syscalls::linux_raw::raw_close(fd);
            }
        };

        let state = match InceptionLayerState::get() {
            Some(s) => s,
            None => {
                #[cfg(target_os = "macos")]
                return crate::syscalls::macos_raw::raw_close(fd);
                #[cfg(target_os = "linux")]
                return crate::syscalls::linux_raw::raw_close(fd);
            }
        };

        // RFC-0051: Monitor FD usage on close (to reset warning thresholds)
        let _ = crate::syscalls::io::OPEN_FD_COUNT.fetch_update(
            std::sync::atomic::Ordering::Relaxed,
            std::sync::atomic::Ordering::Relaxed,
            |val| Some(val.saturating_sub(1)),
        );
        state.check_fd_usage();

        // Check if this FD is a COW session
        let cow_info = {
            let entry_ptr = state.open_fds.remove(fd as u32);
            if !entry_ptr.is_null() {
                unsafe { Some(*Box::from_raw(entry_ptr)) }
            } else {
                None
            }
        };

        // Use a hash of the FD or 0 if not tracked for general close event
        let file_id = 0; // Simplified for general close
        inception_record!(EventType::Close, file_id, fd);

        // Final close of the file
        #[cfg(target_os = "macos")]
        let res = crate::syscalls::macos_raw::raw_close(fd);
        #[cfg(target_os = "linux")]
        let res = crate::syscalls::linux_raw::raw_close(fd);

        // Offload IPC task to Worker (asynchronous)
        if let Some(info) = cow_info {
            inception_log!(
                "COW CLOSE: fd={} vpath='{}' temp='{}'",
                fd,
                info.vpath,
                info.temp_path
            );

            // Offload reingest to Worker (non-blocking)
            if let Some(reactor) = crate::sync::get_reactor() {
                let _ = reactor.ring_buffer.push(crate::sync::Task::Reingest {
                    vpath: info.vpath.to_string(),
                    temp_path: info.temp_path.to_string(),
                });
            }

            res
        } else {
            // Not a COW file, but might be a VFS read-only file or non-VFS file
            untrack_fd(fd);
            #[cfg(target_os = "macos")]
            {
                crate::syscalls::macos_raw::raw_close(fd)
            }
            #[cfg(target_os = "linux")]
            {
                crate::syscalls::linux_raw::raw_close(fd)
            }
        }
    }) // profile_timed! close
}

// ============================================================================
// sendfile / copy_file_range inception layers - prevent VFS write bypass
// ============================================================================

#[cfg(target_os = "macos")]
#[no_mangle]
pub unsafe extern "C" fn sendfile_inception(
    fd: c_int,
    s: c_int,
    offset: libc::off_t,
    len: *mut libc::off_t,
    hdtr: *mut libc::c_void,
    flags: c_int,
) -> c_int {
    // macOS: 's' is the drain (out_fd). Block if it points to VFS territory.
    if crate::syscalls::misc::quick_block_vfs_fd_mutation(s).is_some() {
        return -1;
    }
    crate::syscalls::macos_raw::raw_sendfile(fd, s, offset, len, hdtr, flags)
}

#[cfg(target_os = "linux")]
#[no_mangle]
pub unsafe extern "C" fn sendfile_inception(
    out_fd: c_int,
    in_fd: c_int,
    offset: *mut libc::off_t,
    count: libc::size_t,
) -> libc::ssize_t {
    if crate::syscalls::misc::quick_block_vfs_fd_mutation(out_fd).is_some() {
        return -1;
    }
    crate::syscalls::linux_raw::raw_sendfile(out_fd, in_fd, offset, count)
}

#[cfg(target_os = "linux")]
#[no_mangle]
pub unsafe extern "C" fn copy_file_range_inception(
    fd_in: c_int,
    off_in: *mut libc::off_t,
    fd_out: c_int,
    off_out: *mut libc::off_t,
    len: libc::size_t,
    flags: libc::c_uint,
) -> libc::ssize_t {
    if crate::syscalls::misc::quick_block_vfs_fd_mutation(fd_out).is_some() {
        return -1;
    }
    crate::syscalls::linux_raw::raw_copy_file_range(fd_in, off_in, fd_out, off_out, len, flags)
}
