use crate::path::VfsPath;
use crate::state::*;
use libc::{c_char, c_int, c_void, mode_t};
use std::ffi::CStr;
use std::fmt::Write;
use std::sync::atomic::Ordering;

#[cfg(target_os = "linux")]
use crate::syscalls::linux_raw::raw_open;
#[cfg(target_os = "macos")]
use crate::syscalls::macos_raw::raw_open;

/// Open implementation with VFS detection and CoW semantics.
/// RFC-0044: VDir fast-path — resolve open() via mmap lookup, zero IPC on hit.
pub(crate) unsafe fn open_impl(path: *const c_char, flags: c_int, mode: mode_t) -> Option<c_int> {
    if path.is_null() {
        return None;
    }

    let path_cstr = CStr::from_ptr(path);
    let path_str = match path_cstr.to_str() {
        Ok(s) => s,
        Err(_) => return None,
    };

    let state = InceptionLayerState::get()?;

    let vpath = match state.resolve_path(path_str) {
        Some(p) => {
            inception_log!(
                "open path='{}' -> resolved='{}' (HIT)",
                path_str,
                p.absolute
            );
            profile_count!(vfs_handled);
            inception_record!(EventType::OpenHit, p.manifest_key_hash, 0);
            p
        }
        None => {
            profile_count!(vfs_passthrough);
            return None;
        }
    };

    let is_write = (flags & (libc::O_WRONLY | libc::O_RDWR | libc::O_APPEND | libc::O_TRUNC)) != 0;

    // =========================================================================
    // FAST PATH: VDir mmap lookup — zero syscall, zero IPC, zero allocation
    // Only for non-dirty files where VDir has a valid cas_hash.
    // =========================================================================
    let is_mutation = (flags & (libc::O_CREAT | libc::O_WRONLY | libc::O_RDWR)) != 0;
    if is_mutation {
        unsafe { ensure_parent_dirs(vpath.absolute.as_str()) };
    }

    // =========================================================================
    // FAST PATH: VDir mmap lookup — zero syscall, zero IPC, zero allocation
    // Only for non-dirty files where VDir has a valid cas_hash.
    // =========================================================================
    if !DIRTY_TRACKER.is_dirty(&vpath.manifest_key) {
        if let Some(vdir_entry) = vdir_lookup(state.mmap_ptr, state.mmap_size, &vpath.manifest_key)
        {
            // Skip directories (cas_hash is all zeros for dirs)
            let has_content = !vdir_entry.cas_hash.iter().all(|b| *b == 0);
            if has_content {
                profile_count!(vdir_hits);
                inception_record!(EventType::OpenHit, vpath.manifest_key_hash, 11); // 11 = vdir_open_hit

                let blob_path =
                    format_blob_path_fixed(&state.cas_root, &vdir_entry.cas_hash, vdir_entry.size);

                // BUG-016: Cross-process Dirty Detection Heuristic (Open path).
                // Similar to stat_impl_common, we check if a physical file exists and is newer.
                let mut phys_buf: libc::stat = unsafe { std::mem::zeroed() };
                let phys_rc;

                let path_bytes = vpath.absolute.as_str().as_bytes();
                let mut stack_buf = [0u8; 1024];
                if path_bytes.len() < 1023 {
                    stack_buf[..path_bytes.len()].copy_from_slice(path_bytes);
                    stack_buf[path_bytes.len()] = 0;
                    let path_ptr = stack_buf.as_ptr() as *const libc::c_char;

                    #[cfg(target_os = "macos")]
                    {
                        phys_rc = crate::syscalls::macos_raw::raw_stat(path_ptr, &mut phys_buf);
                    }
                    #[cfg(target_os = "linux")]
                    {
                        phys_rc = crate::syscalls::linux_raw::raw_stat(path_ptr, &mut phys_buf);
                    }

                    let phys_mtime_sec = phys_buf.st_mtime as u64;
                    let phys_mtime_nsec = phys_buf.st_mtime_nsec as u64;

                    // BUG-016: Cross-process Dirty Detection (Nanosecond-aware).
                    let is_phys_newer = (phys_mtime_sec > (vdir_entry.mtime_sec as u64))
                        || (phys_mtime_sec == (vdir_entry.mtime_sec as u64) && phys_mtime_nsec > 0);

                    if phys_rc == 0 && is_phys_newer {
                        profile_count!(vdir_misses);
                        let fd = crate::syscalls::macos_raw::raw_open(path_ptr, flags, mode);
                        if fd >= 0 {
                            crate::syscalls::io::track_fd(
                                fd,
                                &vpath.manifest_key,
                                false,
                                None,
                                vpath.manifest_key_hash,
                            );
                            return Some(fd);
                        }
                        return None;
                    }
                } else {
                    let abs_path_cstr = match std::ffi::CString::new(vpath.absolute.as_str()) {
                        Ok(c) => c,
                        Err(_) => return None,
                    };

                    #[cfg(target_os = "macos")]
                    {
                        phys_rc = crate::syscalls::macos_raw::raw_stat(
                            abs_path_cstr.as_ptr(),
                            &mut phys_buf,
                        );
                    }
                    #[cfg(target_os = "linux")]
                    {
                        phys_rc = crate::syscalls::linux_raw::raw_stat(
                            abs_path_cstr.as_ptr(),
                            &mut phys_buf,
                        );
                    }

                    let phys_mtime_sec = phys_buf.st_mtime as u64;
                    let phys_mtime_nsec = phys_buf.st_mtime_nsec as u64;

                    // BUG-016: Cross-process Dirty Detection (Nanosecond-aware).
                    let is_phys_newer = (phys_mtime_sec > (vdir_entry.mtime_sec as u64))
                        || (phys_mtime_sec == (vdir_entry.mtime_sec as u64) && phys_mtime_nsec > 0);

                    if phys_rc == 0 && is_phys_newer {
                        profile_count!(vdir_misses);
                        let fd = crate::syscalls::macos_raw::raw_open(
                            abs_path_cstr.as_ptr(),
                            flags,
                            mode,
                        );
                        if fd >= 0 {
                            crate::syscalls::io::track_fd(
                                fd,
                                &vpath.manifest_key,
                                false,
                                None,
                                vpath.manifest_key_hash,
                            );
                            return Some(fd);
                        }
                        return None;
                    }
                }

                if is_write {
                    return open_cow_write(
                        state,
                        &vpath,
                        blob_path.as_str(),
                        flags,
                        mode,
                        vdir_entry.mode,
                    );
                } else {
                    return open_cas_read(
                        state,
                        &vpath,
                        blob_path.as_str(),
                        flags,
                        mode,
                        vdir_entry.size,
                        vdir_entry.mode,
                        vdir_entry.mtime_sec as u64,
                    );
                }
            }
            // Directory entry — fall through to passthrough
        }
    }

    // =========================================================================
    // SLOW PATH: IPC fallback — VDir miss, dirty file, or directory
    // =========================================================================
    profile_count!(vdir_misses);
    profile_count!(ipc_calls);
    let entry = match state.query_manifest_ipc(&vpath) {
        Some(e) => {
            inception_log!(
                "manifest lookup '{}': FOUND (mode=0o{:o}, size={})",
                vpath.manifest_key,
                e.mode,
                e.size
            );
            e
        }
        None => {
            // RFC-0039 Solid Mode: Allow new file creation in VFS territory
            inception_log!(
                "manifest lookup '{}': NOT FOUND -> passthrough + track (is_write={})",
                vpath.manifest_key,
                is_write
            );
            // RFC-0051++: Solid Mode - ensure parent directories exist for VFS-territory opens.
            // If the parent directory is supposed to exist according to readdir but is
            // missing physically, we must materialize it to avoid ENOENT on O_CREAT.
            ensure_parent_dirs(vpath.absolute.as_str());

            let fd = unsafe { raw_open(path, flags, mode) };
            if fd >= 0 {
                crate::syscalls::io::track_fd(
                    fd,
                    &vpath.manifest_key,
                    true,
                    None,
                    vpath.manifest_key_hash,
                );
                return Some(fd);
            }

            // Solid-mode CAS materialization: if file doesn't exist on disk
            // but VDir has a cached entry, materialize from CAS blob via clonefile.
            if crate::get_errno() == libc::ENOENT {
                if let Some(fd) = try_materialize_from_cas(state, &vpath, path, flags, mode) {
                    return Some(fd);
                }
            }
            return None;
        }
    };

    let blob_path = format_blob_path_fixed(&state.cas_root, &entry.content_hash, entry.size as u64);

    inception_log!("redirection path (IPC): '{}'", blob_path.as_str());

    if is_write {
        open_cow_write(
            state,
            &vpath,
            blob_path.as_str(),
            flags,
            mode,
            entry.mode as u32,
        )
    } else {
        open_cas_read(
            state,
            &vpath,
            blob_path.as_str(),
            flags,
            mode,
            entry.size as u64,
            entry.mode as u32,
            entry.mtime,
        )
    }
}

// ============================================================================
// Solid-mode CAS materialization — auto-rebuild physical files from CAS
// ============================================================================

/// Materialize a file from CAS blob to a physical path.
/// Uses clonefile (APFS CoW, zero-copy) with hardlink fallback.
/// Restores mtime so cargo fingerprints match.
///
/// Called by:
///   - stat_impl_common(): VDir HIT + physical ENOENT → materialize before returning metadata
///   - try_materialize_from_cas(): open() ENOENT fallback
///
/// Returns true if physical file was successfully created.
#[cfg(target_os = "macos")]
pub(crate) unsafe fn materialize_from_cas_entry(
    state: &InceptionLayerState,
    entry: &crate::state::VDirStatResult,
    physical_path: &str,
) -> bool {
    // Skip directories (cas_hash is all zeros)
    if entry.cas_hash.iter().all(|b| *b == 0) {
        return false;
    }

    // Step 1: Construct CAS blob path
    let blob_path = format_blob_path_fixed(&state.cas_root, &entry.cas_hash, entry.size);
    let blob_cpath = match std::ffi::CString::new(blob_path.as_str()) {
        Ok(c) => c,
        Err(_) => return false,
    };

    // Step 2: Ensure parent directories exist
    ensure_parent_dirs(physical_path);

    // Step 3: clonefile (APFS CoW — zero data copy, instant)
    let dst_cpath = match std::ffi::CString::new(physical_path) {
        Ok(c) => c,
        Err(_) => return false,
    };
    let at_fdcwd: libc::c_int = -2; // AT_FDCWD on macOS
    let rc = crate::syscalls::macos_raw::raw_clonefileat(
        at_fdcwd,
        blob_cpath.as_ptr(),
        at_fdcwd,
        dst_cpath.as_ptr(),
        0, // default flags
    );

    if rc != 0 {
        let clone_errno = crate::get_errno();
        if clone_errno == libc::EEXIST {
            // Already materialized — but still ensure flags/perms are correct
            // (previous materialization may have left uchg from CAS blob)
            crate::syscalls::macos_raw::raw_chflags(dst_cpath.as_ptr(), 0);
            // Restore original mode from VDir (SSOT)
            crate::syscalls::macos_raw::raw_chmod(dst_cpath.as_ptr(), entry.mode as _);
            return true;
        }
        // clonefile failed (e.g. non-APFS, cross-device) — try hardlink fallback
        let rc2 = crate::syscalls::macos_raw::raw_link(blob_cpath.as_ptr(), dst_cpath.as_ptr());
        if rc2 != 0 {
            let link_errno = crate::get_errno();
            if link_errno == libc::EEXIST {
                crate::syscalls::macos_raw::raw_chflags(dst_cpath.as_ptr(), 0);
                crate::syscalls::macos_raw::raw_chmod(dst_cpath.as_ptr(), entry.mode as _);
                return true; // Already materialized
            }
            inception_log!(
                "CAS materialize failed for '{}': clonefile errno={}, link errno={}",
                physical_path,
                clone_errno,
                link_errno
            );
            return false;
        }
    }

    // Step 4: Clear macOS immutable flags (uchg) and restore original VDir mode.
    // CAS blobs have uchg and are 0444, but the working file must honor original perms.
    crate::syscalls::macos_raw::raw_chflags(dst_cpath.as_ptr(), 0);
    crate::syscalls::macos_raw::raw_chmod(dst_cpath.as_ptr(), entry.mode as _);

    // Step 5: Set mtime to NOW — materialization = fake compilation
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let times = [
        libc::timeval {
            tv_sec: now.as_secs() as _,
            tv_usec: 0, // BUG-016: Clear usec to allow detection of real writes (which have usec > 0)
        },
        libc::timeval {
            tv_sec: now.as_secs() as _,
            tv_usec: 0,
        },
    ];
    crate::syscalls::macos_raw::raw_utimes(dst_cpath.as_ptr(), times.as_ptr());

    profile_count!(cas_materializations);
    true
}

#[cfg(not(target_os = "macos"))]
pub(crate) unsafe fn materialize_from_cas_entry(
    _state: &InceptionLayerState,
    _entry: &crate::state::VDirStatResult,
    _physical_path: &str,
) -> bool {
    false
}

/// Try to materialize a file from CAS blob when open() gets ENOENT.
/// Looks up VDir, materializes to physical path, then retries open.
#[cfg(target_os = "macos")]
unsafe fn try_materialize_from_cas(
    state: &InceptionLayerState,
    vpath: &VfsPath,
    path: *const c_char,
    flags: c_int,
    mode: mode_t,
) -> Option<c_int> {
    let vdir_entry = vdir_lookup(state.mmap_ptr, state.mmap_size, &vpath.manifest_key)?;
    let path_str = CStr::from_ptr(path).to_str().ok()?;

    if !materialize_from_cas_entry(state, &vdir_entry, path_str) {
        return None;
    }

    inception_record!(EventType::OpenHit, vpath.manifest_key_hash, 12); // 12 = cas_materialize_hit

    // Retry open — should succeed now
    let fd = raw_open(path, flags, mode);
    if fd >= 0 {
        let mut cached_stat: libc::stat = std::mem::zeroed();
        cached_stat.st_size = vdir_entry.size as _;
        cached_stat.st_mode = vdir_entry.mode as _;
        cached_stat.st_mtime = vdir_entry.mtime_sec as _;
        cached_stat.st_dev = 0x52494654; // "RIFT"
        cached_stat.st_nlink = 1;
        cached_stat.st_ino = vpath.manifest_key_hash as _;

        crate::syscalls::io::track_fd(
            fd,
            &vpath.manifest_key,
            true,
            Some(cached_stat),
            vpath.manifest_key_hash,
        );
        inception_log!(
            "CAS materialized+opened '{}' (size={})",
            vpath.manifest_key,
            vdir_entry.size
        );
        Some(fd)
    } else {
        None
    }
}

/// Recursively create parent directories for a path using raw mkdir syscalls.
// materialize_directory: ensures a physical directory exists (recursive)
pub(crate) unsafe fn materialize_directory(path: &str) {
    if path.is_empty() || path == "/" {
        return;
    }

    let mut stack_buf = [0u8; 1024];
    if path.len() > 1023 {
        return;
    }
    stack_buf[..path.len()].copy_from_slice(path.as_bytes());
    stack_buf[path.len()] = 0;
    let path_ptr = stack_buf.as_ptr() as *const libc::c_char;

    #[cfg(target_os = "macos")]
    let res = crate::syscalls::macos_raw::raw_mkdir(path_ptr, 0o755);
    #[cfg(target_os = "linux")]
    let res = crate::syscalls::linux_raw::raw_mkdirat(libc::AT_FDCWD, path_ptr, 0o755);

    // Diagnostic log (zero-alloc)
    if crate::state::DEBUG_ENABLED.load(std::sync::atomic::Ordering::Relaxed) {
        let mut log_buf = [0u8; 1024];
        let mut log_writer = crate::macros::StackWriter::new(&mut log_buf);
        use std::fmt::Write;
        let _ = writeln!(
            log_writer,
            "[vrift-inception] materialize_directory: {} -> {}",
            path, res
        );
        let log_msg = log_writer.as_str();
        unsafe {
            #[cfg(target_os = "macos")]
            crate::syscalls::macos_raw::raw_write(2, log_msg.as_ptr() as *const _, log_msg.len());
            #[cfg(target_os = "linux")]
            libc::write(2, log_msg.as_ptr() as *const _, log_msg.len());
        }
    }

    if res == -1 {
        let err = crate::get_errno();
        if err == libc::EEXIST {
            return;
        }
        if crate::state::DEBUG_ENABLED.load(std::sync::atomic::Ordering::Relaxed) {
            inception_log!("materialize_directory FAILED: {} errno={}", path, err);
        }
        if err == libc::ENOENT {
            // Parent missing, recurse
            if let Some(last_slash) = path.trim_end_matches('/').rfind('/') {
                materialize_directory(&path[..last_slash]);
                // Try again after parent created
                #[cfg(target_os = "macos")]
                crate::syscalls::macos_raw::raw_mkdir(path_ptr, 0o755);
                #[cfg(target_os = "linux")]
                crate::syscalls::linux_raw::raw_mkdirat(libc::AT_FDCWD, path_ptr, 0o755);
            }
        }
    }
}

pub(crate) unsafe fn ensure_parent_dirs(path: &str) {
    if let Some(last_slash) = path.rfind('/') {
        let parent = &path[..last_slash];
        if parent.is_empty() || parent == "/" {
            return;
        }
        materialize_directory(parent);
    }
}

/// Stub for non-macOS platforms — materialization not supported.
#[cfg(not(target_os = "macos"))]
unsafe fn try_materialize_from_cas(
    _state: &InceptionLayerState,
    _vpath: &VfsPath,
    _path: *const c_char,
    _flags: c_int,
    _mode: mode_t,
) -> Option<c_int> {
    None
}

/// Open a CAS blob for reading — shared by VDir fast-path and IPC fallback.
#[inline]
#[allow(clippy::too_many_arguments)]
unsafe fn open_cas_read(
    _state: &InceptionLayerState,
    vpath: &VfsPath,
    blob_path: &str,
    flags: c_int,
    mode: mode_t,
    size: u64,
    file_mode: u32,
    mtime: u64,
) -> Option<c_int> {
    let blob_cpath = std::ffi::CString::new(blob_path).ok()?;
    let fd = unsafe { libc::open(blob_cpath.as_ptr(), flags, mode as libc::c_uint) };
    if fd >= 0 {
        let mut cached_stat: libc::stat = unsafe { std::mem::zeroed() };
        cached_stat.st_size = size as _;
        cached_stat.st_mode = file_mode as _;
        cached_stat.st_mtime = mtime as _;
        cached_stat.st_dev = 0x52494654; // "RIFT"
        cached_stat.st_nlink = 1;
        cached_stat.st_ino = vpath.manifest_key_hash as _;

        crate::syscalls::io::track_fd(
            fd,
            &vpath.manifest_key,
            true,
            Some(cached_stat),
            vpath.manifest_key_hash,
        );
        Some(fd)
    } else {
        None
    }
}

/// Open with CoW (Copy-on-Write) semantics — shared by VDir fast-path and IPC fallback.
#[inline]
unsafe fn open_cow_write(
    state: &InceptionLayerState,
    vpath: &VfsPath,
    blob_path: &str,
    flags: c_int,
    mode: mode_t,
    vdir_mode: u32,
) -> Option<c_int> {
    inception_log!("open write request for '{}'", vpath.absolute);

    // M4: Mark path as dirty in DirtyTracker (enables stat redirect to staging)
    DIRTY_TRACKER.mark_dirty(&vpath.manifest_key);

    let mut attempts = 0;
    let mut fd = -1;
    let mut temp_path_fs = FixedString::<1024>::new();
    let pid = unsafe { libc::getpid() };
    let tid_addr = &attempts as *const _ as usize;

    while attempts < 100 {
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();

        let mut buf = [0u8; 1024];
        let mut writer = crate::macros::StackWriter::new(&mut buf);
        let _ = write!(
            writer,
            "{}/.vrift/staging/vrift_cow_{}_{}_{}_{}.tmp",
            state.project_root.as_str(),
            pid,
            timestamp,
            tid_addr,
            attempts
        );
        temp_path_fs.set(writer.as_str());

        let c_temp = match std::ffi::CString::new(temp_path_fs.as_str()) {
            Ok(c) => c,
            Err(_) => break,
        };
        fd = unsafe {
            libc::open(
                c_temp.as_ptr(),
                libc::O_RDWR | libc::O_CREAT | libc::O_EXCL | libc::O_CLOEXEC,
                (vdir_mode | 0o600) as libc::c_uint, // Ensure writable for COW while preserving execute bits
            )
        };
        if fd >= 0 {
            break;
        }
        if unsafe { crate::get_errno() } != libc::EEXIST {
            break;
        }
        attempts += 1;
    }

    if fd < 0 {
        return None;
    }
    let temp_fd = fd;
    let temp_path = temp_path_fs;
    unsafe { libc::close(temp_fd) };
    let temp_cpath = std::ffi::CString::new(temp_path.as_str()).ok()?;

    inception_log!("COW TRIGGERED: '{}' -> '{}'", vpath.absolute, temp_path);
    inception_record!(EventType::CowTriggered, vpath.manifest_key_hash, 0);

    let blob_cpath = std::ffi::CString::new(blob_path).ok()?;
    let src_fd = unsafe { libc::open(blob_cpath.as_ptr(), libc::O_RDONLY | libc::O_CLOEXEC) };
    if src_fd >= 0 {
        let dst_fd = unsafe {
            libc::open(
                temp_cpath.as_ptr(),
                libc::O_WRONLY | libc::O_CREAT | libc::O_TRUNC | libc::O_CLOEXEC,
                0o644,
            )
        };
        if dst_fd >= 0 {
            // Already set by first open, but ensure consistency
            let _ = unsafe { libc::fchmod(dst_fd, (vdir_mode | 0o600) as _) };
            let mut buf = [0u8; 8192];
            loop {
                let n = unsafe { libc::read(src_fd, buf.as_mut_ptr() as *mut c_void, buf.len()) };
                if n <= 0 {
                    break;
                }
                unsafe { libc::write(dst_fd, buf.as_ptr() as *const c_void, n as usize) };
            }
            unsafe { libc::close(dst_fd) };
        }
        unsafe { libc::close(src_fd) };
    } else {
        let dst_fd = unsafe {
            libc::open(
                temp_cpath.as_ptr(),
                libc::O_WRONLY | libc::O_CREAT | libc::O_TRUNC | libc::O_CLOEXEC,
                0o644,
            )
        };
        if dst_fd >= 0 {
            unsafe { libc::close(dst_fd) };
        }
    }

    let fd = unsafe { libc::open(temp_cpath.as_ptr(), flags, mode as libc::c_uint) };
    if fd < 0 {
        None
    } else {
        let entry = Box::into_raw(Box::new(crate::syscalls::io::FdEntry {
            vpath: vpath.absolute,
            manifest_key: vpath.manifest_key,
            manifest_key_hash: vpath.manifest_key_hash,
            temp_path,
            is_vfs: true,
            cached_stat: None,
            mmap_count: 0,
            lock_fd: -1,
        }));

        let old = state.open_fds.set(fd as u32, entry);
        if !old.is_null() {
            unsafe { drop(Box::from_raw(old)) };
        } else {
            crate::syscalls::io::OPEN_FD_COUNT.fetch_add(1, Ordering::Relaxed);
        }
        Some(fd)
    }
}

/// Stack-allocated hex encoding of a 32-byte hash → 64-char hex string.
/// Zero heap allocation.
fn hex_encode_fixed(hash: &[u8; 32]) -> FixedString<68> {
    const HEX_CHARS: &[u8; 16] = b"0123456789abcdef";
    let mut buf = [0u8; 64];
    for (i, byte) in hash.iter().enumerate() {
        buf[i * 2] = HEX_CHARS[(byte >> 4) as usize];
        buf[i * 2 + 1] = HEX_CHARS[(byte & 0x0f) as usize];
    }
    let mut result = FixedString::<68>::new();
    // Safety: all bytes are ASCII hex chars
    result.set(unsafe { std::str::from_utf8_unchecked(&buf) });
    result
}

/// Stack-allocated blob path: "{cas_root}/blake3/{xx}/{yy}/{hash}_{size}.bin"
/// Zero heap allocation — uses FixedString<1024>.
fn format_blob_path_fixed(
    cas_root: &FixedString<1024>,
    hash: &[u8; 32],
    size: u64,
) -> FixedString<1024> {
    let hex = hex_encode_fixed(hash);
    let hex_str = hex.as_str();
    let mut path = FixedString::<1024>::new();
    let mut buf = [0u8; 1024];
    let mut writer = crate::macros::StackWriter::new(&mut buf);
    let _ = write!(
        writer,
        "{}/blake3/{}/{}/{}_{}.bin",
        cas_root.as_str(),
        &hex_str[0..2],
        &hex_str[2..4],
        hex_str,
        size
    );
    path.set(writer.as_str());
    path
}

// Called by C bridge (c_open_bridge) after INITIALIZING check passes
#[no_mangle]
pub unsafe extern "C" fn velo_open_impl(path: *const c_char, flags: c_int, mode: mode_t) -> c_int {
    if crate::profile::PROFILE_ENABLED.load(std::sync::atomic::Ordering::Relaxed) {
        let _t0 = crate::profile::now_ns();
        let _result = open_impl(path, flags, mode).unwrap_or_else(|| raw_open(path, flags, mode));
        let _elapsed = crate::profile::now_ns().wrapping_sub(_t0);
        crate::profile::PROFILE
            .open_calls
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        crate::profile::PROFILE
            .open_ns
            .fetch_add(_elapsed, std::sync::atomic::Ordering::Relaxed);
        crate::profile::profile_record_path(path, _elapsed);
        _result
    } else {
        open_impl(path, flags, mode).unwrap_or_else(|| raw_open(path, flags, mode))
    }
}

#[cfg(target_os = "macos")]
#[no_mangle]
pub unsafe extern "C" fn open_inception(p: *const c_char, f: c_int, m: mode_t) -> c_int {
    // Route through C bridge for early-init safety (avoids Rust TLS during dyld bootstrap)
    // C's c_open_bridge checks INITIALIZING state before calling any Rust code
    extern "C" {
        fn c_open_bridge(path: *const c_char, flags: c_int, mode: mode_t) -> c_int;
    }
    c_open_bridge(p, f, m)
}

#[cfg(target_os = "macos")]
#[no_mangle]
pub unsafe extern "C" fn openat_inception(
    dfd: c_int,
    p: *const c_char,
    f: c_int,
    m: mode_t,
) -> c_int {
    // Route through C bridge for early-init safety
    extern "C" {
        fn c_openat_bridge(dirfd: c_int, path: *const c_char, flags: c_int, mode: mode_t) -> c_int;
    }
    c_openat_bridge(dfd, p, f, m)
}

#[no_mangle]
pub unsafe extern "C" fn open_inception_c_impl(p: *const c_char, f: c_int, m: mode_t) -> c_int {
    #[inline(always)]
    unsafe fn raw_open_internal(path: *const c_char, flags: c_int, mode: mode_t) -> c_int {
        #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
        {
            let ret: i64;
            let err: i64;
            std::arch::asm!(
                "mov x16, #5", "svc #0x80", "cset {err}, cs",
                in("x0") path, in("x1") flags as i64, in("x2") mode as i64,
                lateout("x0") ret, err = out(reg) err,
            );
            if err != 0 {
                crate::set_errno(ret as c_int);
                -1
            } else {
                ret as c_int
            }
        }
        #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
        {
            crate::syscalls::macos_raw::raw_open(path, flags, mode)
        }
        #[cfg(target_os = "linux")]
        {
            crate::syscalls::linux_raw::raw_openat(-100, path, flags, mode)
        }
    }

    passthrough_if_init!(raw_open_internal, p, f, m);

    if CIRCUIT_TRIPPED.load(Ordering::Relaxed) {
        return raw_open_internal(p, f, m);
    }
    if !is_vfs_ready() && InceptionLayerState::get().is_none() {
        return raw_open_internal(p, f, m);
    }

    let state = match InceptionLayerState::get() {
        Some(s) => s,
        None => return raw_open_internal(p, f, m),
    };

    // RFC-0051: Monitor FD usage on every attempt (Lock-Free)
    state.check_fd_usage();

    let fd = {
        let path_str = unsafe { CStr::from_ptr(p).to_string_lossy() };
        let vpath = state.resolve_path(&path_str);
        if vpath.is_none() {
            inception_record!(EventType::OpenMiss, 0, 0);
            let fd = raw_open_internal(p, f, m);
            if fd >= 0 {
                crate::syscalls::io::OPEN_FD_COUNT.fetch_add(1, Ordering::Relaxed);
            }
            return fd;
        }

        let _guard = match InceptionLayerGuard::enter() {
            Some(g) => g,
            None => return raw_open_internal(p, f, m),
        };

        velo_open_impl(p, f, m)
    };

    fd
}

#[no_mangle]
pub unsafe extern "C" fn velo_openat_impl(
    dirfd: c_int,
    p: *const c_char,
    f: c_int,
    m: mode_t,
) -> c_int {
    #[inline(always)]
    unsafe fn raw_openat_internal(
        dirfd: c_int,
        path: *const c_char,
        flags: c_int,
        mode: mode_t,
    ) -> c_int {
        #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
        {
            let ret: i64;
            let err: i64;
            std::arch::asm!(
                "mov x16, #463", "svc #0x80", "cset {err}, cs",
                in("x0") dirfd as i64, in("x1") path, in("x2") flags as i64, in("x3") mode as i64,
                lateout("x0") ret, err = out(reg) err,
            );
            if err != 0 {
                crate::set_errno(ret as c_int);
                -1
            } else {
                ret as c_int
            }
        }
        #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
        {
            crate::syscalls::macos_raw::raw_openat(dirfd, path, flags, mode)
        }
        #[cfg(target_os = "linux")]
        {
            crate::syscalls::linux_raw::raw_openat(dirfd, path, flags, mode)
        }
    }

    passthrough_if_init!(raw_openat_internal, dirfd, p, f, m);

    if CIRCUIT_TRIPPED.load(Ordering::Relaxed) {
        return raw_openat_internal(dirfd, p, f, m);
    }
    if !is_vfs_ready() && InceptionLayerState::get().is_none() {
        return raw_openat_internal(dirfd, p, f, m);
    }

    let _guard = match InceptionLayerGuard::enter() {
        Some(g) => g,
        None => return raw_openat_internal(dirfd, p, f, m),
    };
    open_impl(p, f, m).unwrap_or_else(|| raw_openat_internal(dirfd, p, f, m))
}

#[cfg(target_os = "linux")]
#[repr(C)]
pub struct open_how {
    pub flags: u64,
    pub mode: u64,
    pub resolve: u64,
}

#[cfg(target_os = "linux")]
#[no_mangle]
pub unsafe extern "C" fn openat2_inception(
    dirfd: c_int,
    p: *const c_char,
    how: *const open_how,
    size: libc::size_t,
) -> c_int {
    if how.is_null() || size < std::mem::size_of::<open_how>() {
        return crate::syscalls::linux_raw::raw_openat2(dirfd, p, how as _, size);
    }

    passthrough_if_init!(
        crate::syscalls::linux_raw::raw_openat2,
        dirfd,
        p,
        how as _,
        size
    );

    if CIRCUIT_TRIPPED.load(Ordering::Relaxed) {
        return crate::syscalls::linux_raw::raw_openat2(dirfd, p, how as _, size);
    }

    let how_ref = &*how;
    let flags = how_ref.flags as c_int;
    let mode = how_ref.mode as mode_t;

    // Use regular open_impl for VFS redirection
    // Note: open_impl doesn't currenty support 'resolve' flags of openat2,
    // but covering path redirection is the primary goal.
    open_impl(p, flags, mode)
        .unwrap_or_else(|| crate::syscalls::linux_raw::raw_openat2(dirfd, p, how as _, size))
}

#[no_mangle]
pub unsafe extern "C" fn creat_inception(path: *const c_char, mode: mode_t) -> c_int {
    let flags = libc::O_CREAT | libc::O_WRONLY | libc::O_TRUNC;
    // Route through open_inception_c_impl (platform-generic entry point)
    open_inception_c_impl(path, flags, mode)
}
