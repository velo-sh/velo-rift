#[allow(unused_imports)]
use crate::reals::*;
use crate::state::*;
use libc::{c_char, c_int, stat as libc_stat};
use std::ffi::CStr;
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
    let state = InceptionLayerState::get()?;

    // 1. Resolve path to VFS domain
    let vpath = match state.resolve_path(path_str) {
        Some(p) => {
            profile_count!(vfs_handled);
            p
        }
        None => {
            profile_count!(vfs_passthrough);
            return None;
        }
    };
    let manifest_path = vpath.manifest_key.as_str();

    // BUG-011: VDir only stores file entries. Before VDir virtualization, check if the
    // physical path exists as a directory. If so, skip VDir entirely and let kernel stat
    // handle it. This prevents hash collisions from returning file metadata for directories,
    // which breaks create_dir_all (sees EEXIST + !is_dir → error).
    let mut phys_buf: libc_stat = unsafe { std::mem::zeroed() };
    let mut phys_exists = false;
    {
        // Use the resolved absolute physical path for the physical check
        // Performance: Optimization for common path lengths using stack CString
        let path_bytes = vpath.absolute.as_str().as_bytes();
        let mut stack_buf = [0u8; 1024];
        if path_bytes.len() < 1023 {
            stack_buf[..path_bytes.len()].copy_from_slice(path_bytes);
            stack_buf[path_bytes.len()] = 0;
            let path_ptr = stack_buf.as_ptr() as *const libc::c_char;

            #[cfg(target_os = "macos")]
            let phys_result = crate::syscalls::macos_raw::raw_stat(path_ptr, &mut phys_buf);
            #[cfg(target_os = "linux")]
            let phys_result = crate::syscalls::linux_raw::raw_stat(path_ptr, &mut phys_buf);

            if phys_result == 0 {
                let mode = phys_buf.st_mode as u32;
                if mode & 0o170000 == 0o040000 {
                    // Physical path is a directory — skip VDir, let kernel stat handle it
                    return None;
                }
                phys_exists = true;
            }
        } else {
            // Fallback to heap for very long paths
            let path_cstr = match std::ffi::CString::new(vpath.absolute.as_str()) {
                Ok(c) => c,
                Err(_) => return None,
            };

            #[cfg(target_os = "macos")]
            let phys_result =
                crate::syscalls::macos_raw::raw_stat(path_cstr.as_ptr(), &mut phys_buf);
            #[cfg(target_os = "linux")]
            let phys_result =
                crate::syscalls::linux_raw::raw_stat(path_cstr.as_ptr(), &mut phys_buf);

            if phys_result == 0 {
                let mode = phys_buf.st_mode as u32;
                if mode & 0o170000 == 0o040000 {
                    // Physical path is a directory — skip VDir, let kernel stat handle it
                    return None;
                }
                phys_exists = true;
            }
        }
    }

    // PSFS: hot path — zero alloc, zero lock, zero syscall. Hit/Miss recorded below.

    // M4: Dirty Check - if file is being written to, bypass mmap cache
    if DIRTY_TRACKER.is_dirty(manifest_path) {
        // Try to find live metadata from open FDs
        if let Some(temp_path) = find_live_temp_path(manifest_path) {
            let temp_path_cstr = match std::ffi::CString::new(temp_path.as_str()) {
                Ok(c) => c,
                Err(_) => return None,
            };
            #[cfg(target_os = "macos")]
            let res = unsafe { crate::syscalls::macos_raw::raw_stat(temp_path_cstr.as_ptr(), buf) };
            #[cfg(target_os = "linux")]
            let res = unsafe { crate::syscalls::linux_raw::raw_stat(temp_path_cstr.as_ptr(), buf) };

            if res == 0 {
                // Virtualize the dev/ino to match VFS expectations
                unsafe {
                    (*buf).st_dev = 0x52494654; // "RIFT"
                    (*buf).st_ino = vpath.manifest_key_hash as _;
                }
                inception_record!(EventType::StatHit, vpath.manifest_key_hash, 10); // 10 = dirty_hit (temp file stat)
                return Some(0);
            }
        }
        // If not found in open FDs (e.g. closed but not reingested), fall back to IPC
        // but SKIP mmap cache.
    } else {
        // Try Hot Stat Cache — Phase 1.3: seqlock-protected VDir lookup
        if let Some(entry) = vdir_lookup(state.mmap_ptr, state.mmap_size, manifest_path) {
            // BUG-016: Cross-process Dirty Detection Heuristic.
            // If physical file exists and its mtime is newer than VDir entry, it was
            // likely updated by a sibling process (e.g. rustc) during the current session.
            // Since DIRTY_TRACKER is per-process, we use this mtime check as a fallback.
            let phys_mtime_sec = phys_buf.st_mtime as u64;
            let phys_mtime_nsec = phys_buf.st_mtime_nsec as u64;

            inception_log!(
                "DEBUG mtime '{}': phys={}.{:09}, vdir={}.0",
                manifest_path,
                phys_mtime_sec,
                phys_mtime_nsec,
                entry.mtime_sec
            );

            // BUG-016: Cross-process Dirty Detection (Nanosecond-aware).
            // Materialized files have nanoseconds set to 0 (see materialize_from_cas_entry).
            // Newly written files by rustc have high-precision nanoseconds > 0.
            let is_phys_newer = (phys_mtime_sec > (entry.mtime_sec as u64))
                || (phys_mtime_sec == (entry.mtime_sec as u64) && phys_mtime_nsec > 0);

            if phys_exists && is_phys_newer {
                inception_log!(
                    "physical file newer than VDir entry, bypassing VDir for '{}' (phys={}.{:09}, vdir={}.0)",
                    manifest_path,
                    phys_mtime_sec,
                    phys_mtime_nsec,
                    entry.mtime_sec
                );
                profile_count!(vdir_misses);
                // Return the physical stat results directly by copying into the output buffer
                unsafe {
                    std::ptr::copy_nonoverlapping(&phys_buf, buf, 1);
                }
                return Some(0);
            }

            // BUG-011: VDir only stores file entries. If the entry mode lacks S_IFREG
            // (0o100000), it's likely a hash collision or corrupted entry — fall through
            // to kernel stat so directories are reported correctly.
            let mode_with_type = if entry.mode & 0o170000 == 0 {
                // No S_IFMT bits set — assume regular file, add S_IFREG
                entry.mode | 0o100000
            } else if entry.mode & 0o170000 != 0o100000 {
                // Has S_IFMT but not S_IFREG (e.g. S_IFDIR) — skip VDir, fall through
                profile_count!(vdir_misses);
                inception_record!(EventType::StatMiss, vpath.manifest_key_hash, 20);
                return None;
            } else {
                entry.mode
            };
            // RFC-0039 Architecture: Faithfully report the original mode from VDir.
            // Decouples VFS visibility from the read-only CAS physical storage.

            // Solid Mode: materialize from CAS if physical file doesn't exist.
            // VDir says "file exists" → ensure it ACTUALLY exists on disk.
            // After this, all subsequent syscalls (open/unlink/chmod) hit real files.
            // Also ensures filesystem is intact after exiting inception mode.
            if !phys_exists {
                crate::syscalls::open::materialize_from_cas_entry(state, &entry, path_str);
            }

            profile_count!(vdir_hits);
            inception_record!(EventType::StatHit, vpath.manifest_key_hash, 11); // 11 = vdir_hit (seqlock)
            std::ptr::write_bytes(buf, 0, 1);
            (*buf).st_size = entry.size as _;
            // Materialization = fake compilation → mtime = Now()
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default();
            #[cfg(target_os = "macos")]
            {
                (*buf).st_mode = mode_with_type as u16;
                (*buf).st_mtime = now.as_secs() as _;
                (*buf).st_mtime_nsec = now.subsec_nanos() as _;
            }
            #[cfg(target_os = "linux")]
            {
                (*buf).st_mode = mode_with_type as _;
                (*buf).st_mtime = now.as_secs() as _;
                (*buf).st_mtime_nsec = now.subsec_nanos() as _;
            }
            (*buf).st_dev = 0x52494654; // "RIFT"
            (*buf).st_nlink = 1;
            (*buf).st_ino = vpath.manifest_key_hash as _;
            // duplicate record removed — line 83 already records the vdir_hit
            return Some(0);
        }

        // RFC-0051: Synthetic directory stat for VDir-implied directories.
        // VDir only stores file entries, but directories are implied by file paths.
        // When stat() is called on a non-existent path that has VDir children
        // (e.g. target/debug/.fingerprint/slab-HASH/), return synthetic S_IFDIR
        // metadata so cargo doesn't skip the directory.
        if !phys_exists {
            use crate::state::vdir_list_dir;
            // Use manifest_path which already has the correct VDir-relative format
            if vdir_list_dir(state.mmap_ptr, state.mmap_size, manifest_path).is_some() {
                // BUG-016: If we claim a directory exists virtually, we MUST ensure its physical
                // parent exists to avoid ENOENT in subsequent child creation.
                unsafe { crate::syscalls::open::materialize_directory(path_str) };

                std::ptr::write_bytes(buf, 0, 1);
                (*buf).st_size = 0;
                #[cfg(target_os = "macos")]
                {
                    (*buf).st_mode = 0o040755_u16; // S_IFDIR | rwxr-xr-x
                    (*buf).st_mtime = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs() as _;
                }
                #[cfg(target_os = "linux")]
                {
                    (*buf).st_mode = 0o040755;
                    (*buf).st_mtime = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs() as _;
                }
                (*buf).st_dev = 0x52494654; // "RIFT"
                (*buf).st_nlink = 2;
                (*buf).st_ino = vpath.manifest_key_hash as _;
                profile_count!(vdir_hits);
                inception_record!(EventType::StatHit, vpath.manifest_key_hash, 13); // 13 = synthetic_dir
                return Some(0);
            }
        }
    }

    profile_count!(vdir_misses);
    inception_record!(EventType::StatMiss, vpath.manifest_key_hash, 20); // 20 = vdir_miss, trying IPC

    // Try IPC query (also use manifest path format)
    profile_count!(ipc_calls);
    if let Some(entry) = state.query_manifest(&vpath) {
        std::ptr::write_bytes(buf, 0, 1);
        (*buf).st_size = entry.size as _;
        // Materialization = fake compilation → mtime = Now()
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        #[cfg(target_os = "macos")]
        {
            (*buf).st_mode = entry.mode as u16;
            (*buf).st_mtime = now.as_secs() as _;
            (*buf).st_mtime_nsec = now.subsec_nanos() as _;
        }
        #[cfg(target_os = "linux")]
        {
            (*buf).st_mode = entry.mode as _;
            (*buf).st_mtime = now.as_secs() as _;
            (*buf).st_mtime_nsec = now.subsec_nanos() as _;
        }
        (*buf).st_dev = 0x52494654; // "RIFT"
        (*buf).st_nlink = 1;
        (*buf).st_ino = vpath.manifest_key_hash as _;
        inception_record!(EventType::StatHit, vpath.manifest_key_hash, 12); // 12 = ipc_hit
        return Some(0);
    }

    inception_record!(
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

    let _guard = InceptionLayerGuard::enter()?;
    let path_str = CStr::from_ptr(path).to_str().ok()?;

    // RFC-0044: Try VDir first (symlink following not yet implemented)
    if let Some(result) = stat_impl_common(path_str, buf) {
        return Some(result);
    }

    None
}

#[no_mangle]
pub unsafe extern "C" fn velo_stat_impl(path: *const c_char, buf: *mut libc_stat) -> c_int {
    if crate::profile::PROFILE_ENABLED.load(std::sync::atomic::Ordering::Relaxed) {
        let _t0 = crate::profile::now_ns();
        let _result = stat_impl(path, buf, true).unwrap_or_else(|| {
            #[cfg(target_os = "macos")]
            return crate::syscalls::macos_raw::raw_stat(path, buf);
            #[cfg(target_os = "linux")]
            return crate::syscalls::linux_raw::raw_stat(path, buf);
        });
        let _elapsed = crate::profile::now_ns().wrapping_sub(_t0);
        crate::profile::PROFILE
            .stat_calls
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        crate::profile::PROFILE
            .stat_ns
            .fetch_add(_elapsed, std::sync::atomic::Ordering::Relaxed);
        crate::profile::profile_record_path(path, _elapsed);
        _result
    } else {
        stat_impl(path, buf, true).unwrap_or_else(|| {
            #[cfg(target_os = "macos")]
            return crate::syscalls::macos_raw::raw_stat(path, buf);
            #[cfg(target_os = "linux")]
            return crate::syscalls::linux_raw::raw_stat(path, buf);
        })
    }
}

#[no_mangle]
pub unsafe extern "C" fn stat_inception(path: *const c_char, buf: *mut libc_stat) -> c_int {
    // Standard interpose entry point
    let init_state = crate::state::INITIALIZING.load(std::sync::atomic::Ordering::Relaxed);
    if init_state != 0 {
        #[cfg(target_os = "macos")]
        return crate::syscalls::macos_raw::raw_stat(path, buf);
        #[cfg(target_os = "linux")]
        return crate::syscalls::linux_raw::raw_stat(path, buf);
    }
    velo_stat_impl(path, buf)
}

#[no_mangle]
pub unsafe extern "C" fn velo_lstat_impl(path: *const c_char, buf: *mut libc_stat) -> c_int {
    profile_timed!(lstat_calls, lstat_ns, {
        stat_impl(path, buf, false).unwrap_or_else(|| {
            #[cfg(target_os = "macos")]
            return crate::syscalls::macos_raw::raw_lstat(path, buf);
            #[cfg(target_os = "linux")]
            return crate::syscalls::linux_raw::raw_lstat(path, buf);
        })
    })
}

#[no_mangle]
pub unsafe extern "C" fn lstat_inception(path: *const c_char, buf: *mut libc_stat) -> c_int {
    let init_state = crate::state::INITIALIZING.load(std::sync::atomic::Ordering::Relaxed);
    if init_state != 0 {
        #[cfg(target_os = "macos")]
        return crate::syscalls::macos_raw::raw_lstat(path, buf);
        #[cfg(target_os = "linux")]
        return crate::syscalls::linux_raw::raw_lstat(path, buf);
    }
    velo_lstat_impl(path, buf)
}

#[no_mangle]
pub unsafe extern "C" fn velo_fstat_impl(fd: c_int, buf: *mut libc_stat) -> c_int {
    profile_timed!(fstat_calls, fstat_ns, {
        // 🔥 ULTRA-FAST PATH: Lock-free, Allocation-free, TLS-free logic
        // This supports usage inside malloc() without deadlock.

        // 1. Check FdTable (if initialized)
        // Note: We use InceptionLayerState directly instead of Reactor to ensure consistency
        if let Some(state) = InceptionLayerState::get() {
            let entry_ptr = state.open_fds.get(fd as u32);
            if !entry_ptr.is_null() {
                let entry = &*entry_ptr;

                // M4: If this is a COW file with a temp_path, return live metadata from temp file
                if !entry.temp_path.is_empty() {
                    let temp_path_cstr = match std::ffi::CString::new(entry.temp_path.as_str()) {
                        Ok(c) => c,
                        Err(_) => return -1,
                    };
                    #[cfg(target_os = "macos")]
                    let res = crate::syscalls::macos_raw::raw_stat(temp_path_cstr.as_ptr(), buf);
                    #[cfg(target_os = "linux")]
                    let res = crate::syscalls::linux_raw::raw_stat(temp_path_cstr.as_ptr(), buf);

                    if res == 0 {
                        // Virtualize the dev/ino to match VFS expectations
                        (*buf).st_dev = 0x52494654;
                        (*buf).st_ino = entry.manifest_key_hash as _;
                        return 0;
                    }
                }

                // If we have a cached stat (standard case for VFS files)
                if let Some(ref cached) = entry.cached_stat {
                    *buf = *cached;
                    return 0;
                }

                // Fallback for VFS files without cached stat (rare?)
                if entry.is_vfs {
                    // BUG FIX: Use resolve_path to get a VfsPath for query_manifest
                    if let Some(vpath) = state.resolve_path(entry.vpath.as_str()) {
                        if let Some(vnode) = state.query_manifest(&vpath) {
                            std::ptr::write_bytes(buf, 0, 1);
                            (*buf).st_size = vnode.size as _;
                            #[cfg(target_os = "macos")]
                            {
                                (*buf).st_mode = vnode.mode as u16;
                            }
                            #[cfg(target_os = "linux")]
                            {
                                (*buf).st_mode = vnode.mode as _;
                            }
                            (*buf).st_mtime = vnode.mtime as _;
                            (*buf).st_dev = 0x52494654;
                            (*buf).st_nlink = 1;
                            (*buf).st_ino = vpath.manifest_key_hash as _;
                            inception_record!(EventType::StatHit, vpath.manifest_key_hash, 0);
                            return 0;
                        }
                    }
                }
            }
        }

        // 2. Not tracked or state not ready -> Raw Syscall
        // We do NOT use InceptionLayerGuard here because fstat is used by malloc/TLS init.
        // If it's not in FdTable, it's not a VFS file (Closed World Assumption).
        #[cfg(target_os = "macos")]
        {
            crate::syscalls::macos_raw::raw_fstat64(fd, buf)
        }
        #[cfg(target_os = "linux")]
        {
            crate::syscalls::linux_raw::raw_fstat(fd, buf)
        }
    }) // profile_timed! close
}

#[no_mangle]
pub unsafe extern "C" fn fstat_inception(fd: c_int, buf: *mut libc_stat) -> c_int {
    let init_state = crate::state::INITIALIZING.load(std::sync::atomic::Ordering::Relaxed);
    if init_state != 0 {
        #[cfg(target_os = "macos")]
        return crate::syscalls::macos_raw::raw_fstat64(fd, buf);
        #[cfg(target_os = "linux")]
        return crate::syscalls::linux_raw::raw_fstat(fd, buf);
    }
    velo_fstat_impl(fd, buf)
}

#[no_mangle]
pub unsafe extern "C" fn velo_access_impl(path: *const c_char, mode: c_int) -> c_int {
    profile_timed!(access_calls, access_ns, {
        // Use raw syscall for fallback to avoid dlsym deadlock (Pattern 2682.v2)
        let _guard = match InceptionLayerGuard::enter() {
            Some(g) => g,
            None => {
                #[cfg(target_os = "macos")]
                return crate::syscalls::macos_raw::raw_access(path, mode);
                #[cfg(target_os = "linux")]
                return crate::syscalls::linux_raw::raw_access(path, mode);
            }
        };

        let path_str = match CStr::from_ptr(path).to_str() {
            Ok(s) => s,
            Err(_) => {
                #[cfg(target_os = "macos")]
                return crate::syscalls::macos_raw::raw_access(path, mode);
                #[cfg(target_os = "linux")]
                return crate::syscalls::linux_raw::raw_access(path, mode);
            }
        };

        if InceptionLayerState::get()
            .map(|s| s.inception_applicable(path_str))
            .unwrap_or(false)
        {
            return 0;
        }

        #[cfg(target_os = "macos")]
        {
            crate::syscalls::macos_raw::raw_access(path, mode)
        }
        #[cfg(target_os = "linux")]
        {
            crate::syscalls::linux_raw::raw_access(path, mode)
        }
    }) // profile_timed! close
}

#[no_mangle]
pub unsafe extern "C" fn access_inception(path: *const c_char, mode: c_int) -> c_int {
    // BUG-007: Use raw syscall during early init to avoid recursion
    let init_state = crate::state::INITIALIZING.load(std::sync::atomic::Ordering::Relaxed);
    if init_state != 0 || crate::state::CIRCUIT_TRIPPED.load(std::sync::atomic::Ordering::Relaxed) {
        #[cfg(target_os = "macos")]
        return crate::syscalls::macos_raw::raw_access(path, mode);
        #[cfg(target_os = "linux")]
        return crate::syscalls::linux_raw::raw_access(path, mode);
    }

    velo_access_impl(path, mode)
}

#[no_mangle]
pub unsafe extern "C" fn velo_fstatat_impl(
    dirfd: c_int,
    path: *const c_char,
    buf: *mut libc_stat,
    flags: c_int,
) -> c_int {
    // Use raw syscall for fallback to avoid dlsym deadlock (Pattern 2682.v2)
    let _guard = match InceptionLayerGuard::enter() {
        Some(g) => g,
        None => {
            #[cfg(target_os = "macos")]
            return crate::syscalls::macos_raw::raw_fstatat64(dirfd, path, buf, flags);
            #[cfg(target_os = "linux")]
            return crate::syscalls::linux_raw::raw_fstatat(dirfd, path, buf, flags);
        }
    };

    if dirfd == libc::AT_FDCWD || (!path.is_null() && unsafe { *path == b'/' as libc::c_char }) {
        if let Ok(path_str) = unsafe { CStr::from_ptr(path).to_str() } {
            if let Some(res) = stat_impl_common(path_str, buf) {
                return res;
            }
        }
    }

    #[cfg(target_os = "macos")]
    return crate::syscalls::macos_raw::raw_fstatat64(dirfd, path, buf, flags);
    #[cfg(target_os = "linux")]
    return crate::syscalls::linux_raw::raw_fstatat(dirfd, path, buf, flags);
}

#[no_mangle]
pub unsafe extern "C" fn fstatat_inception(
    dirfd: c_int,
    path: *const c_char,
    buf: *mut libc_stat,
    flags: c_int,
) -> c_int {
    // BUG-007 / RFC-0051: Use raw syscall during early init to avoid dlsym recursion.
    // Also check if INCEPTION_LAYER_STATE is null to avoid TLS deadlock hazards.
    let init_state = crate::state::INITIALIZING.load(Ordering::Relaxed);
    if init_state != 0
        || crate::state::INCEPTION_LAYER_STATE
            .load(Ordering::Acquire)
            .is_null()
    {
        #[cfg(target_os = "macos")]
        return crate::syscalls::macos_raw::raw_fstatat64(dirfd, path, buf, flags);
        #[cfg(target_os = "linux")]
        return crate::syscalls::linux_raw::raw_fstatat(dirfd, path, buf, flags);
    }

    velo_fstatat_impl(dirfd, path, buf, flags)
}

/// Linux-specific fstatat inception layer call
#[cfg(target_os = "linux")]
pub unsafe fn fstatat_inception_linux(
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
pub unsafe extern "C" fn statx_inception(
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
    if init_state != 0
        || crate::state::INCEPTION_LAYER_STATE
            .load(Ordering::Acquire)
            .is_null()
    {
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
    if let Some(state) = InceptionLayerState::get() {
        if let Some(vpath) = state.resolve_path(path_str) {
            if let Some(entry) = state.query_manifest(&vpath) {
                std::ptr::write_bytes(buf, 0, 1);
                (*buf).stx_mask = 0x7FF; // basic stats
                (*buf).stx_size = entry.size as _;
                (*buf).stx_mode = entry.mode as _;
                (*buf).stx_ino = vrift_ipc::fnv1a_hash(path_str) as _;
                (*buf).stx_nlink = 1;
                (*buf).stx_mtime.tv_sec = entry.mtime as _;
                (*buf).stx_blksize = 4096;
                (*buf).stx_blocks = entry.size.div_ceil(512);
                inception_record!(EventType::StatHit, (*buf).stx_ino, 0);
                return 0;
            }
        }
    }

    crate::syscalls::linux_raw::raw_statx(dirfd, path, flags, mask, buf as *mut libc::c_void)
}

/// Helper: Find an open temp_path for a given manifest path.
unsafe fn find_live_temp_path(manifest_path: &str) -> Option<crate::state::FixedString<1024>> {
    let state = InceptionLayerState::get()?;
    let mut result = None;
    state.open_fds.for_each(|entry| {
        if entry.manifest_key.as_str() == manifest_path && !entry.temp_path.is_empty() {
            result = Some(entry.temp_path);
        }
    });
    result
}
