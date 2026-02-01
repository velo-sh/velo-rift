use crate::interpose::*;
use crate::path::*;
use crate::state::*;
use crate::syscalls::path_ops::break_link;
use libc::{c_char, c_int, c_void, mode_t};
use std::ffi::CStr;
use std::ptr;
use std::sync::atomic::Ordering;

// ============================================================================
// Open
// ============================================================================

unsafe fn open_impl(path: *const c_char, flags: c_int, _mode: mode_t) -> Option<c_int> {
    // Early bailout during ShimState initialization to prevent CasStore::new recursion
    if INITIALIZING.load(Ordering::SeqCst) {
        return None;
    }

    // Note: Don't check SHIM_STATE.is_null() here - ShimState::get() handles lazy init properly

    let _guard = ShimGuard::enter()?;
    let state = ShimState::get()?;

    let path_str = CStr::from_ptr(path).to_str().ok()?;

    let mut path_buf = [0u8; 1024];
    let resolved_len = (unsafe { resolve_path_with_cwd(path_str, &mut path_buf) })?;
    let resolved_path = unsafe { std::str::from_utf8_unchecked(&path_buf[..resolved_len]) };

    let is_write = (flags & (libc::O_WRONLY | libc::O_RDWR | libc::O_TRUNC | libc::O_APPEND)) != 0;

    // RFC-0046/RFC-0043: Robust check with mandatory exclusions
    if resolved_path.starts_with(&*state.vfs_prefix)
        && !resolved_path.contains("/.vrift/")
        && !resolved_path.starts_with(&*state.cas_root)
    {
        // Query with full path since manifest stores full paths (e.g., /vrift/testfile.txt)
        if let Some(entry) = state.query_manifest(resolved_path) {
            if entry.is_dir() {
                set_errno(libc::EISDIR);
                return Some(-1);
            }
            // RFC-0047: Check mode permission before allowing writes
            // Faithfully reflect original file permissions
            if is_write {
                let write_bits = 0o222; // S_IWUSR | S_IWGRP | S_IWOTH
                if (entry.mode & write_bits) == 0 {
                    set_errno(libc::EACCES);
                    return Some(-1);
                }
            }
            if let Some(cas_guard) = state.get_cas() {
                if let Some(cas) = cas_guard.as_ref() {
                    if let Ok(content) = cas.get(&entry.content_hash) {
                        let mut tmp_path_buf = [0u8; 128];
                        let prefix = b"/tmp/vrift-mem-";
                        unsafe {
                            ptr::copy_nonoverlapping(
                                prefix.as_ptr(),
                                tmp_path_buf.as_mut_ptr(),
                                prefix.len(),
                            )
                        };
                        for i in 0..32 {
                            let hex = b"0123456789abcdef";
                            tmp_path_buf[prefix.len() + i * 2] =
                                hex[(entry.content_hash[i] >> 4) as usize];
                            tmp_path_buf[prefix.len() + i * 2 + 1] =
                                hex[(entry.content_hash[i] & 0x0f) as usize];
                        }
                        tmp_path_buf[prefix.len() + 64] = 0;

                        let tmp_fd = libc::open(
                            tmp_path_buf.as_ptr() as *const c_char,
                            libc::O_CREAT | libc::O_RDWR | libc::O_TRUNC,
                            0o644,
                        );
                        if tmp_fd >= 0 {
                            libc::write(tmp_fd, content.as_ptr() as *const c_void, content.len());
                            libc::lseek(tmp_fd, 0, libc::SEEK_SET);

                            // Helper to extract temp_path string
                            let tmp_len = prefix.len() + 64;
                            if let Ok(tmp_str) = std::str::from_utf8(&tmp_path_buf[..tmp_len]) {
                                state.open_fds.lock().unwrap().insert(
                                    tmp_fd,
                                    OpenFile {
                                        vpath: resolved_path.to_string(),
                                        temp_path: tmp_str.to_string(),
                                        mmap_count: 0,
                                    },
                                );
                            }
                            return Some(tmp_fd);
                        }
                    }
                }
            }
        }
    }

    if is_write && path_str.starts_with(&*state.vfs_prefix) {
        let _ = unsafe { break_link(path_str) };

        // For write operations on VFS paths, we let the real open happen
        // and track the fd for re-ingest. We return None here to indicate
        // that the real open should be called by the shim.
        // The tracking of the fd will happen in the shim's wrapper function
        // after the real_open call.
        return None;
    }

    None
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
    // Passthrough to real openat - VFS path resolution happens at open time
    // AT_FDCWD (-2) means use current working directory
    let real = get_real_shim!(REAL_OPENAT, "openat", IT_OPENAT, OpenatFn);
    real(dirfd, pathname, flags, mode)
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
