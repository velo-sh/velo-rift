use crate::state::*;
use libc::{c_char, c_int, c_void, mode_t};
use std::ffi::CStr;

/// Open implementation with VFS detection and CoW semantics.
///
/// For paths in the VFS domain:
/// - Read-only opens: Resolve CAS blob path and open directly
/// - Write opens: Copy CAS blob to temp file, track for reingest on close
unsafe fn open_impl(path: *const c_char, flags: c_int, mode: mode_t) -> Option<c_int> {
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

    // Check if path is in VFS domain
    if !state.psfs_applicable(path_str) {
        return None; // Not our path, passthrough
    }

    // Query manifest via IPC
    let entry = state.query_manifest_ipc(path_str)?;

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
        if (entry.mode & 0o200) == 0 {
            set_errno(libc::EACCES);
            return Some(-1);
        }

        // CoW: Copy blob to temp file for writes
        let temp_path = format!("/tmp/vrift_cow_{}.tmp", libc::getpid());
        let blob_cpath = std::ffi::CString::new(blob_path.as_str()).ok()?;
        let temp_cpath = std::ffi::CString::new(temp_path.as_str()).ok()?;

        let src_fd = libc::open(blob_cpath.as_ptr(), libc::O_RDONLY);
        if src_fd >= 0 {
            let dst_fd = libc::open(
                temp_cpath.as_ptr(),
                libc::O_WRONLY | libc::O_CREAT | libc::O_TRUNC,
                0o644,
            );
            if dst_fd >= 0 {
                let mut buf = [0u8; 8192];
                loop {
                    let n = libc::read(src_fd, buf.as_mut_ptr() as *mut c_void, buf.len());
                    if n <= 0 {
                        break;
                    }
                    libc::write(dst_fd, buf.as_ptr() as *const c_void, n as usize);
                }
                libc::close(dst_fd);
            }
            libc::close(src_fd);
        } else {
            let dst_fd = libc::open(
                temp_cpath.as_ptr(),
                libc::O_WRONLY | libc::O_CREAT | libc::O_TRUNC,
                0o644,
            );
            if dst_fd >= 0 {
                libc::close(dst_fd);
            }
        }

        let fd = libc::open(temp_cpath.as_ptr(), flags, mode as libc::c_uint);
        if fd >= 0 {
            if let Ok(mut fds) = state.open_fds.lock() {
                fds.insert(
                    fd,
                    OpenFile {
                        vpath: path_str.to_string(),
                        temp_path: temp_path.clone(),
                        mmap_count: 0,
                    },
                );
            }
        }
        Some(fd)
    } else {
        let blob_cpath = std::ffi::CString::new(blob_path.as_str()).ok()?;
        let fd = libc::open(blob_cpath.as_ptr(), flags, mode as libc::c_uint);
        if fd >= 0 {
            return Some(fd);
        }
        set_errno(libc::ENOENT);
        Some(-1)
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
    extern "C" {
        fn open_shim_c_impl(p: *const c_char, f: c_int, m: mode_t) -> c_int;
    }
    open_shim_c_impl(p, f, m)
}

#[cfg(target_os = "macos")]
#[no_mangle]
pub unsafe extern "C" fn openat_shim(dfd: c_int, p: *const c_char, f: c_int, m: mode_t) -> c_int {
    extern "C" {
        fn openat_shim_c_impl(dfd: c_int, p: *const c_char, f: c_int, m: mode_t) -> c_int;
    }
    openat_shim_c_impl(dfd, p, f, m)
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
                set_errno(ret as c_int);
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
                    set_errno(-ret as c_int);
                    -1
                } else {
                    ret as c_int
                }
            }
            #[cfg(target_arch = "aarch64")]
            {
                let ret: i64;
                std::arch::asm!(
                    "mov x8, #56", "svc #0", in("x0") -100i64, in("x1") path, in("x2") flags as i64, in("x3") mode as i64,
                    lateout("x0") ret,
                );
                if ret < 0 {
                    set_errno(-ret as c_int);
                    -1
                } else {
                    ret as c_int
                }
            }
        }
    }

    // VFS logic entry
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
                set_errno(ret as c_int);
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
                    set_errno(-ret as c_int);
                    -1
                } else {
                    ret as c_int
                }
            }
            #[cfg(target_arch = "aarch64")]
            {
                let ret: i64;
                std::arch::asm!(
                    "mov x8, #56", "svc #0", in("x0") dirfd as i64, in("x1") path, in("x2") flags as i64, in("x3") mode as i64,
                    lateout("x0") ret,
                );
                if ret < 0 {
                    set_errno(-ret as c_int);
                    -1
                } else {
                    ret as c_int
                }
            }
        }
    }

    // VFS logic entry
    let _guard = match ShimGuard::enter() {
        Some(g) => g,
        None => return raw_openat(dirfd, p, f, m),
    };
    open_impl(p, f, m).unwrap_or_else(|| raw_openat(dirfd, p, f, m))
}

#[cfg(target_os = "macos")]
fn set_errno(e: c_int) {
    unsafe {
        *libc::__error() = e;
    }
}

#[cfg(target_os = "linux")]
fn set_errno(e: c_int) {
    unsafe {
        *libc::__errno_location() = e;
    }
}
