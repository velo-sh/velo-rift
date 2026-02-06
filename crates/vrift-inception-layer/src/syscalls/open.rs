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
            inception_record!(EventType::OpenHit, p.manifest_key_hash, 0);
            p
        }
        None => return None,
    };

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
            // Manifest MISS means the file doesn't exist in VFS yet - this is a NEW file
            // We passthrough to real FS and track for later Live Ingest on close()
            let is_write =
                (flags & (libc::O_WRONLY | libc::O_RDWR | libc::O_APPEND | libc::O_TRUNC)) != 0;

            inception_log!(
                "manifest lookup '{}': NOT FOUND -> passthrough + track (is_write={})",
                vpath.manifest_key,
                is_write
            );
            inception_record!(EventType::OpenMiss, vpath.manifest_key_hash, 0);

            let fd = unsafe { raw_open(path, flags, mode) };
            if fd >= 0 {
                // Track FD for Live Ingest on close() - especially important for writes
                crate::syscalls::io::track_fd(fd, &vpath.manifest_key, true, None);
                return Some(fd);
            }
            return None;
        }
    };

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
                "{}/.vrift/staging/vrift_cow_{}_{}_{}_{}.tmp\0",
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
                    0o600,
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

        let blob_cpath = std::ffi::CString::new(blob_path.as_str()).ok()?;
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
            // Allocate entry manually for lock-free insertion
            let entry = Box::into_raw(Box::new(crate::syscalls::io::FdEntry {
                vpath: vpath.absolute,
                manifest_key: vpath.manifest_key,
                temp_path,
                is_vfs: true,
                cached_stat: None,
                mmap_count: 0,
            }));

            let old = state.open_fds.set(fd as u32, entry);
            if !old.is_null() {
                // If overwritten (unlikely for new FD!), reclaim old
                unsafe { drop(Box::from_raw(old)) };
            } else {
                crate::syscalls::io::OPEN_FD_COUNT.fetch_add(1, Ordering::Relaxed);
            }
            Some(fd)
        }
    } else {
        let blob_cpath = std::ffi::CString::new(blob_path.as_str()).ok()?;
        let fd = unsafe { libc::open(blob_cpath.as_ptr(), flags, mode as libc::c_uint) };
        if fd >= 0 {
            // 🔥 Build and cache stat for VFS file
            let mut cached_stat: libc::stat = unsafe { std::mem::zeroed() };
            cached_stat.st_size = entry.size as _;
            cached_stat.st_mode = entry.mode as _;
            cached_stat.st_mtime = entry.mtime as _;
            cached_stat.st_dev = 0x52494654; // "RIFT"
            cached_stat.st_nlink = 1;
            cached_stat.st_ino = vrift_ipc::fnv1a_hash(&vpath.manifest_key) as _;

            crate::syscalls::io::track_fd(fd, &vpath.manifest_key, true, Some(cached_stat));
            Some(fd)
        } else {
            None
        }
    }
}

// Called by C bridge (c_open_bridge) after INITIALIZING check passes
#[no_mangle]
pub unsafe extern "C" fn velo_open_impl(path: *const c_char, flags: c_int, mode: mode_t) -> c_int {
    open_impl(path, flags, mode).unwrap_or_else(|| raw_open(path, flags, mode))
}

fn hex_encode(hash: &[u8; 32]) -> String {
    const HEX_CHARS: &[u8; 16] = b"0123456789abcdef";
    let mut result = String::with_capacity(64);
    for byte in hash {
        result.push(HEX_CHARS[(byte >> 4) as usize] as char);
        result.push(HEX_CHARS[(byte & 0x0f) as usize] as char);
    }
    result
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

        let fd = velo_open_impl(p, f, m);
        if fd >= 0 {
            crate::syscalls::io::OPEN_FD_COUNT.fetch_add(1, Ordering::Relaxed);
        }
        fd
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
