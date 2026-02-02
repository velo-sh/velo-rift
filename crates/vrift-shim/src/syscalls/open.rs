use crate::state::*;
use libc::{c_char, c_int, c_void, mode_t};
use std::ffi::CStr;
use std::sync::atomic::Ordering;

/// Open implementation with VFS detection and CoW semantics.
///
/// For paths in the VFS domain:
/// - Read-only opens: Resolve CAS blob path and open directly
/// - Write opens: Copy CAS blob to temp file, track for reingest on close
pub(crate) unsafe fn open_impl(path: *const c_char, flags: c_int, mode: mode_t) -> Option<c_int> {
    if path.is_null() {
        return None;
    }

    let path_cstr = CStr::from_ptr(path);
    let path_str = match path_cstr.to_str() {
        Ok(s) => s,
        Err(_) => return None,
    };

    // Get shim state
    let state = ShimState::get()?;

    // Architecture: Use unified PathResolver to handle relative/absolute paths
    let vpath = match state.resolve_path(path_str) {
        Some(p) => {
            vfs_log!(
                "open path='{}' -> resolved='{}' (VFS HIT)",
                path_str,
                p.absolute
            );
            vfs_record!(EventType::OpenHit, p.manifest_key_hash, 0);
            p
        }
        None => {
            return None;
        }
    };

    // Query manifest via IPC
    let entry = match state.query_manifest_ipc(&vpath) {
        Some(e) => {
            vfs_log!(
                "manifest lookup '{}': FOUND (mode=0o{:o}, size={})",
                vpath.manifest_key,
                e.mode,
                e.size
            );
            e
        }
        None => {
            vfs_log!(
                "manifest lookup '{}': NOT FOUND -> passthrough",
                vpath.manifest_key
            );
            vfs_record!(EventType::OpenMiss, vpath.manifest_key_hash, -libc::ENOENT);
            return None;
        }
    };

    // Build CAS blob path
    let hash_hex = hex_encode(&entry.content_hash);
    let blob_path = format!(
        "{}/blake3/{}/{}/{}_{}.bin",
        state.cas_root,
        &hash_hex[0..2],
        &hash_hex[2..4],
        hash_hex,
        entry.size
    );

    let is_write = (flags & (libc::O_WRONLY | libc::O_RDWR | libc::O_APPEND | libc::O_TRUNC)) != 0;

    if is_write {
        vfs_log!("open write request for '{}'", vpath.absolute);

        // BBW (Break-Before-Write): Always create a CoW temp copy for writes.
        // RFC-0043: Fix non-atomic mkstemp O_CLOEXEC gap by using manual loop with O_CREAT|O_EXCL|O_CLOEXEC

        let mut attempts = 0;
        let mut fd = -1;
        let mut temp_path_string = String::new();

        // Simple Pseudo-Random Context for temp name generation
        // Note: We avoid heavy rand crates to keep shim lightweight
        let pid = unsafe { libc::getpid() };
        // Use address of local var as thread-specific seed component
        let tid_addr = &attempts as *const _ as usize;

        while attempts < 100 {
            let timestamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos();

            // Generate: /tmp/vrift_cow_{pid}_{timestamp}_{tid}_{attempt}.tmp
            temp_path_string = format!(
                "/tmp/vrift_cow_{}_{}_{}_{}.tmp",
                pid, timestamp, tid_addr, attempts
            );
            let c_temp = match std::ffi::CString::new(temp_path_string.as_str()) {
                Ok(c) => c,
                Err(_) => break,
            };

            // Atomic creation with O_CLOEXEC
            // O_EXCL ensures we don't clobber existing files
            fd = unsafe {
                libc::open(
                    c_temp.as_ptr(),
                    libc::O_RDWR | libc::O_CREAT | libc::O_EXCL | libc::O_CLOEXEC,
                    0o600,
                )
            };

            if fd >= 0 {
                break;
            }

            if unsafe { crate::get_errno() } != libc::EEXIST {
                break; // Fatal error
            }

            attempts += 1;
        }

        if fd < 0 {
            vfs_log!(
                "COW FAILED: could not create temp file (errno={})",
                crate::get_errno()
            );
            return None;
        }

        // At this point we have an open FD to a unique empty temp file, and it is O_CLOEXEC
        let temp_fd = fd;
        let temp_path = temp_path_string; // Take ownership

        // Close the temp_fd because the logic below re-opens it.
        // Wait! The logic below (L108, L116) expects to perform the COPY.
        // Current logic: L82 mkstemp (returns fd), L98 close(temp_fd), L109 open(O_TRUNC).
        // Since we created it empty with O_EXCL, we can just close it and let the logic proceed,
        // OR we can keep it open and use it?
        // The original logic closed it at L98 and re-opened at L109.
        // We will stick to the pattern but ensure re-open uses O_CLOEXEC.
        unsafe { libc::close(temp_fd) };
        let temp_cpath = std::ffi::CString::new(temp_path.as_str()).ok()?;

        vfs_log!("COW TRIGGERED: '{}' -> '{}'", vpath.absolute, temp_path);
        vfs_record!(EventType::CowTriggered, vpath.manifest_key_hash, 0);

        let blob_cpath = std::ffi::CString::new(blob_path.as_str()).ok()?;

        // INTERNAL OPEN: Must be O_CLOEXEC
        let src_fd = unsafe { libc::open(blob_cpath.as_ptr(), libc::O_RDONLY | libc::O_CLOEXEC) };
        if src_fd >= 0 {
            // INTERNAL OPEN: Must be O_CLOEXEC
            let dst_fd = unsafe {
                libc::open(
                    temp_cpath.as_ptr(),
                    libc::O_WRONLY | libc::O_CREAT | libc::O_TRUNC | libc::O_CLOEXEC,
                    0o644,
                )
            };
            if dst_fd >= 0 {
                let mut buf = [0u8; 8192];
                loop {
                    let n =
                        unsafe { libc::read(src_fd, buf.as_mut_ptr() as *mut c_void, buf.len()) };
                    if n <= 0 {
                        break;
                    }
                    unsafe { libc::write(dst_fd, buf.as_ptr() as *const c_void, n as usize) };
                }
                unsafe { libc::close(dst_fd) };
                vfs_log!("COW copy successful");
            } else {
                vfs_log!(
                    "COW FAILED: could not create temp file (errno={})",
                    crate::get_errno()
                );
            }
            unsafe { libc::close(src_fd) };
        } else {
            vfs_log!(
                "COW: CAS blob not found (errno={}), creating empty temp",
                crate::get_errno()
            );
            // INTERNAL OPEN: Must be O_CLOEXEC
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
            vfs_log!(
                "COW FAILED: final open of temp file failed (errno={})",
                crate::get_errno()
            );
            None
        } else {
            vfs_log!("COW session started: fd={} vpath='{}'", fd, vpath.absolute);
            if let Ok(mut fds) = state.open_fds.lock() {
                fds.insert(
                    fd,
                    OpenFile {
                        vpath: vpath.absolute.clone(),
                        temp_path,
                        mmap_count: 0,
                    },
                );
            }
            Some(fd)
        }
    } else {
        let blob_cpath = std::ffi::CString::new(blob_path.as_str()).ok()?;
        let fd = libc::open(blob_cpath.as_ptr(), flags, mode as libc::c_uint);
        vfs_log!("READ session: fd={} blob='{}'", fd, blob_path);
        if fd >= 0 {
            Some(fd)
        } else {
            vfs_log!(
                "READ FAILED: could not open CAS blob (errno={})",
                crate::get_errno()
            );
            None
        }
    }
}

// --- Hex Encoding ---

fn hex_encode(hash: &[u8; 32]) -> String {
    const HEX_CHARS: &[u8; 16] = b"0123456789abcdef";
    let mut result = String::with_capacity(64);
    for byte in hash {
        result.push(HEX_CHARS[(byte >> 4) as usize] as char);
        result.push(HEX_CHARS[(byte & 0x0f) as usize] as char);
    }
    result
}

// --- C Bridge Proxies (Force Export) ---

#[cfg(target_os = "macos")]
#[no_mangle]
pub unsafe extern "C" fn open_shim(p: *const c_char, f: c_int, m: mode_t) -> c_int {
    velo_open_impl(p, f, m)
}

#[cfg(target_os = "macos")]
#[no_mangle]
pub unsafe extern "C" fn openat_shim(dfd: c_int, p: *const c_char, f: c_int, m: mode_t) -> c_int {
    velo_openat_impl(dfd, p, f, m)
}

// --- Unified Implementation Entry Points ---

/// Unified open implementation called by C bridge
#[no_mangle]
pub unsafe extern "C" fn velo_open_impl(p: *const c_char, f: c_int, m: mode_t) -> c_int {
    #[inline(always)]
    unsafe fn raw_open(path: *const c_char, flags: c_int, mode: mode_t) -> c_int {
        #[cfg(target_os = "macos")]
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
        #[cfg(target_os = "linux")]
        {
            #[cfg(target_arch = "x86_64")]
            {
                let ret: i64;
                std::arch::asm!(
                    "syscall", in("rax") 2, in("rdi") path, in("rsi") flags as i64, in("rdx") mode as i64,
                    lateout("rax") ret,
                );
                if ret < 0 {
                    crate::set_errno(-ret as c_int);
                    -1
                } else {
                    ret as c_int
                }
            }
            #[cfg(target_arch = "aarch64")]
            {
                let ret: i64;
                std::arch::asm!(
                    "svc #0",
                    in("x8") 56i64, // openat
                    in("x0") -100i64, // AT_FDCWD
                    in("x1") path,
                    in("x2") flags as i64,
                    in("x3") mode as i64,
                    lateout("x0") ret,
                );
                if ret < 0 {
                    crate::set_errno(-ret as c_int);
                    -1
                } else {
                    ret as c_int
                }
            }
        }
    }

    // Force initialization if needed. ShimState::get() sets VFS_READY on success.
    if CIRCUIT_TRIPPED.load(Ordering::Relaxed) {
        return raw_open(p, f, m);
    }
    if !is_vfs_ready() && ShimState::get().is_none() {
        return raw_open(p, f, m);
    }

    let _guard = match ShimGuard::enter() {
        Some(g) => g,
        None => return raw_open(p, f, m),
    };
    open_impl(p, f, m).unwrap_or_else(|| raw_open(p, f, m))
}

/// Unified openat implementation called by C bridge
#[no_mangle]
pub unsafe extern "C" fn velo_openat_impl(
    dirfd: c_int,
    p: *const c_char,
    f: c_int,
    m: mode_t,
) -> c_int {
    #[inline(always)]
    unsafe fn raw_openat(dirfd: c_int, path: *const c_char, flags: c_int, mode: mode_t) -> c_int {
        #[cfg(target_os = "macos")]
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
        #[cfg(target_os = "linux")]
        {
            #[cfg(target_arch = "x86_64")]
            {
                let ret: i64;
                std::arch::asm!(
                    "syscall", in("rax") 257, in("rdi") dirfd as i64, in("rsi") path, in("rdx") flags as i64, in("r10") mode as i64,
                    lateout("rax") ret,
                );
                if ret < 0 {
                    crate::set_errno(-ret as c_int);
                    -1
                } else {
                    ret as c_int
                }
            }
            #[cfg(target_arch = "aarch64")]
            {
                let ret: i64;
                std::arch::asm!(
                    "svc #0",
                    in("x8") 56i64, // openat
                    in("x0") dirfd as i64,
                    in("x1") path,
                    in("x2") flags as i64,
                    in("x3") mode as i64,
                    lateout("x0") ret,
                );
                if ret < 0 {
                    crate::set_errno(-ret as c_int);
                    -1
                } else {
                    ret as c_int
                }
            }
        }
    }

    // Force initialization if needed. ShimState::get() sets VFS_READY on success.
    if CIRCUIT_TRIPPED.load(Ordering::Relaxed) {
        return raw_openat(dirfd, p, f, m);
    }
    if !is_vfs_ready() && ShimState::get().is_none() {
        return raw_openat(dirfd, p, f, m);
    }

    let _guard = match ShimGuard::enter() {
        Some(g) => g,
        None => return raw_openat(dirfd, p, f, m),
    };
    open_impl(p, f, m).unwrap_or_else(|| raw_openat(dirfd, p, f, m))
}

/// Raw hardlink syscall for Linux
#[cfg(target_os = "linux")]
pub(crate) unsafe fn raw_link(old: *const c_char, new: *const c_char) -> c_int {
    #[cfg(target_arch = "x86_64")]
    {
        let ret: i64;
        std::arch::asm!(
            "syscall", in("rax") 86, in("rdi") old, in("rsi") new,
            lateout("rax") ret,
        );
        if ret < 0 {
            crate::set_errno(-ret as c_int);
            -1
        } else {
            ret as c_int
        }
    }
    #[cfg(target_arch = "aarch64")]
    {
        // aarch64 uses linkat
        let ret: i64;
        std::arch::asm!(
            "svc #0",
            in("x8") 37i64, // linkat
            in("x0") -100i64, // AT_FDCWD
            in("x1") old,
            in("x2") -100i64, // AT_FDCWD
            in("x3") new,
            in("x4") 0,
            lateout("x0") ret,
        );
        if ret < 0 {
            crate::set_errno(-ret as c_int);
            -1
        } else {
            ret as c_int
        }
    }
}
