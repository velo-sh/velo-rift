#[cfg(target_os = "macos")]
use crate::interpose::*;
use crate::state::*;
use libc::{c_char, c_int, c_void, mode_t};
use std::ffi::CStr;
#[cfg(target_os = "linux")]
use std::ptr;
#[cfg(target_os = "linux")]
use std::sync::atomic::AtomicPtr;
#[cfg(target_os = "macos")]
use std::sync::atomic::Ordering;

/// Open implementation with VFS detection and CoW semantics.
///
/// For paths in the VFS domain:
/// - Read-only opens: Resolve CAS blob path and open directly
/// - Write opens: Copy CAS blob to temp file, track for reingest on close
unsafe fn open_impl(path: *const c_char, flags: c_int, mode: mode_t) -> Option<c_int> {
    if path.is_null() {
        shim_log!("[Shim] open_impl: path is null\n");
        return None;
    }

    let path_cstr = CStr::from_ptr(path);
    let path_str = match path_cstr.to_str() {
        Ok(s) => s,
        Err(_) => {
            shim_log!("[Shim] open_impl: path not valid UTF-8\n");
            return None;
        }
    };

    shim_log!("[Shim] open_impl: path=");
    shim_log!(path_str);
    shim_log!("\n");

    // Get shim state
    let state = match ShimState::get() {
        Some(s) => s,
        None => {
            shim_log!("[Shim] open_impl: ShimState::get() returned None\n");
            return None;
        }
    };

    // Check if path is in VFS domain
    if !state.psfs_applicable(path_str) {
        shim_log!("[Shim] open_impl: not in VFS domain\n");
        return None; // Not our path, passthrough
    }

    shim_log!("[Shim] open_impl: in VFS domain, querying manifest\n");

    // Query manifest for this path
    let entry = match state.query_manifest(path_str) {
        Some(e) => e,
        None => {
            shim_log!("[Shim] open_impl: manifest query returned None\n");
            return None;
        }
    };

    shim_log!("[Shim] open_impl: found entry in manifest\n");

    // Build CAS blob path: {cas_root}/blobs/{hash_hex}
    let hash_hex = hex_encode(&entry.content_hash);
    let blob_path = format!("{}/blobs/{}", state.cas_root, hash_hex);

    let is_write = (flags & (libc::O_WRONLY | libc::O_RDWR | libc::O_APPEND | libc::O_TRUNC)) != 0;

    if is_write {
        // CoW: Copy blob to temp file for writes
        let temp_path = format!("/tmp/vrift_cow_{}.tmp", libc::getpid());

        // Check if blob exists, if so copy it
        let blob_cpath = std::ffi::CString::new(blob_path.as_str()).ok()?;
        let temp_cpath = std::ffi::CString::new(temp_path.as_str()).ok()?;

        // Try to copy from CAS blob if it exists (existing file being modified)
        let src_fd = libc::open(blob_cpath.as_ptr(), libc::O_RDONLY);
        if src_fd >= 0 {
            // Create temp file and copy content
            let dst_fd = libc::open(
                temp_cpath.as_ptr(),
                libc::O_WRONLY | libc::O_CREAT | libc::O_TRUNC,
                0o644,
            );
            if dst_fd >= 0 {
                // Copy file content
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
            // New file - create empty temp
            let dst_fd = libc::open(
                temp_cpath.as_ptr(),
                libc::O_WRONLY | libc::O_CREAT | libc::O_TRUNC,
                0o644,
            );
            if dst_fd >= 0 {
                libc::close(dst_fd);
            }
        }

        // Open temp file with original flags
        let fd = libc::open(temp_cpath.as_ptr(), flags, mode as libc::c_uint);
        if fd >= 0 {
            // Track this FD for reingest on close
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
        // Read-only: Open CAS blob directly
        let blob_cpath = std::ffi::CString::new(blob_path.as_str()).ok()?;
        let fd = libc::open(blob_cpath.as_ptr(), flags, mode as libc::c_uint);
        if fd >= 0 {
            return Some(fd);
        }
        // Blob not found - set ENOENT
        set_errno(libc::ENOENT);
        Some(-1)
    }
}

/// Convert hash bytes to hex string (no allocation via static buffer)
fn hex_encode(hash: &[u8; 32]) -> String {
    const HEX_CHARS: &[u8; 16] = b"0123456789abcdef";
    let mut result = String::with_capacity(64);
    for byte in hash {
        result.push(HEX_CHARS[(byte >> 4) as usize] as char);
        result.push(HEX_CHARS[(byte & 0x0f) as usize] as char);
    }
    result
}

// ============================================================================
// Linux Shims
// ============================================================================

#[cfg(target_os = "linux")]
static REAL_OPEN: AtomicPtr<c_void> = AtomicPtr::new(ptr::null_mut());
#[cfg(target_os = "linux")]
static REAL_OPENAT: AtomicPtr<c_void> = AtomicPtr::new(ptr::null_mut());

#[cfg(target_os = "linux")]
#[no_mangle]
pub unsafe extern "C" fn open(p: *const c_char, f: c_int, m: mode_t) -> c_int {
    shim_log("[Shim] open called\n");
    let real = get_real!(REAL_OPEN, "open", OpenFn);
    open_impl(p, f, m).unwrap_or_else(|| real(p, f, m))
}

#[cfg(target_os = "linux")]
#[no_mangle]
pub unsafe extern "C" fn open64(p: *const c_char, f: c_int, m: mode_t) -> c_int {
    shim_log("[Shim] open64 called\n");
    open(p, f, m)
}

#[cfg(target_os = "linux")]
#[no_mangle]
pub unsafe extern "C" fn __open_2(p: *const c_char, f: c_int) -> c_int {
    shim_log("[Shim] __open_2 called\n");
    open(p, f, 0)
}

#[cfg(target_os = "linux")]
#[no_mangle]
pub unsafe extern "C" fn __open64_2(p: *const c_char, f: c_int) -> c_int {
    shim_log("[Shim] __open64_2 called\n");
    open(p, f, 0)
}

#[cfg(target_os = "linux")]
#[no_mangle]
pub unsafe extern "C" fn openat(dirfd: c_int, p: *const c_char, f: c_int, m: mode_t) -> c_int {
    shim_log("[Shim] openat called\n");
    let real = get_real!(REAL_OPENAT, "openat", OpenatFn);
    open_impl(p, f, m).unwrap_or_else(|| real(dirfd, p, f, m))
}

#[cfg(target_os = "linux")]
#[no_mangle]
pub unsafe extern "C" fn openat64(dirfd: c_int, p: *const c_char, f: c_int, m: mode_t) -> c_int {
    shim_log("[Shim] openat64 called\n");
    openat(dirfd, p, f, m)
}

#[cfg(target_os = "linux")]
#[no_mangle]
pub unsafe extern "C" fn __openat_2(dirfd: c_int, p: *const c_char, f: c_int) -> c_int {
    shim_log("[Shim] __openat_2 called\n");
    openat(dirfd, p, f, 0)
}

#[cfg(target_os = "linux")]
#[no_mangle]
pub unsafe extern "C" fn __openat64_2(dirfd: c_int, p: *const c_char, f: c_int) -> c_int {
    shim_log("[Shim] __openat64_2 called\n");
    openat(dirfd, p, f, 0)
}

// ============================================================================
// macOS Shims
// ============================================================================

#[cfg(target_os = "macos")]
#[no_mangle]
pub unsafe extern "C" fn open_shim(p: *const c_char, f: c_int, m: mode_t) -> c_int {
    let real = std::mem::transmute::<*const (), OpenFn>(IT_OPEN.old_func);
    // Early-boot passthrough to avoid deadlock during dyld initialization
    if INITIALIZING.load(Ordering::Relaxed) {
        return real(p, f, m);
    }
    open_impl(p, f, m).unwrap_or_else(|| real(p, f, m))
}

#[cfg(target_os = "macos")]
#[no_mangle]
pub unsafe extern "C" fn openat_shim(
    dirfd: c_int,
    pathname: *const c_char,
    flags: c_int,
    mode: mode_t,
) -> c_int {
    if INITIALIZING.load(Ordering::Relaxed) {
        let f = libc::dlsym(libc::RTLD_NEXT, c"openat".as_ptr());
        let real: OpenatFn = std::mem::transmute(f);
        return real(dirfd, pathname, flags, mode);
    }

    let _guard = match ShimGuard::enter() {
        Some(g) => g,
        None => {
            let real = std::mem::transmute::<*const (), OpenatFn>(IT_OPENAT.old_func);
            return real(dirfd, pathname, flags, mode);
        }
    };

    let real = std::mem::transmute::<*const (), OpenatFn>(IT_OPENAT.old_func);
    open_impl(pathname, flags, mode).unwrap_or_else(|| real(dirfd, pathname, flags, mode))
}

type OpenFn = unsafe extern "C" fn(*const c_char, c_int, mode_t) -> c_int;
type OpenatFn = unsafe extern "C" fn(c_int, *const c_char, c_int, mode_t) -> c_int;

#[cfg(target_os = "linux")]
unsafe fn set_errno(e: c_int) {
    *libc::__errno_location() = e;
}
#[cfg(target_os = "macos")]
unsafe fn set_errno(e: c_int) {
    *libc::__error() = e;
}
