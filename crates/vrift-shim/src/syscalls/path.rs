use crate::interpose::*;
use crate::ipc::*;
use crate::path::*;
use crate::state::*;
use libc::{c_char, c_int, c_void, mode_t, size_t, ssize_t};
use std::ffi::CStr;
use std::ptr;
use std::sync::atomic::{AtomicPtr, Ordering};

// ============================================================================
// Path Implementation
// ============================================================================

type ReadlinkFn = unsafe extern "C" fn(*const c_char, *mut c_char, size_t) -> ssize_t;
type RealpathFn = unsafe extern "C" fn(*const c_char, *mut c_char) -> *mut c_char;
type GetcwdFn = unsafe extern "C" fn(*mut c_char, size_t) -> *mut c_char;
type ChdirFn = unsafe extern "C" fn(*const c_char) -> c_int;
type UnlinkFn = unsafe extern "C" fn(*const c_char) -> c_int;
type RenameFn = unsafe extern "C" fn(*const c_char, *const c_char) -> c_int;
type RmdirFn = unsafe extern "C" fn(*const c_char) -> c_int;
type UtimensatFn =
    unsafe extern "C" fn(c_int, *const c_char, *const libc::timespec, c_int) -> c_int;
type MkdirFn = unsafe extern "C" fn(*const c_char, mode_t) -> c_int;
type SymlinkFn = unsafe extern "C" fn(*const c_char, *const c_char) -> c_int;
type LinkFn = unsafe extern "C" fn(*const c_char, *const c_char) -> c_int;
type LinkatFn = unsafe extern "C" fn(c_int, *const c_char, c_int, *const c_char, c_int) -> c_int;
type RenameatFn = unsafe extern "C" fn(c_int, *const c_char, c_int, *const c_char) -> c_int;

unsafe fn readlink_impl(path: *const c_char, buf: *mut c_char, bufsiz: size_t) -> Option<ssize_t> {
    // Early bailout during ShimState initialization
    if INITIALIZING.load(Ordering::SeqCst) {
        return None;
    }

    let _guard = ShimGuard::enter()?;

    let state = ShimState::get()?;

    let path_str = CStr::from_ptr(path).to_str().ok()?;

    if state.psfs_applicable(path_str) {
        if let Some(entry) = state.query_manifest(path_str) {
            if entry.is_symlink() {
                if let Some(cas_guard) = state.get_cas() {
                    if let Some(cas) = cas_guard.as_ref() {
                        if let Ok(data) = cas.get(&entry.content_hash) {
                            let len = std::cmp::min(data.len(), bufsiz);
                            ptr::copy_nonoverlapping(data.as_ptr(), buf as *mut u8, len);
                            return Some(len as ssize_t);
                        }
                    }
                }
            }
        }
    }

    None
}

// ============================================================================
// Shims (Shared & Linux/macOS)
// ============================================================================

#[cfg(target_os = "linux")]
static REAL_READLINK: AtomicPtr<c_void> = AtomicPtr::new(ptr::null_mut());
static REAL_REALPATH: AtomicPtr<c_void> = AtomicPtr::new(ptr::null_mut());
static REAL_GETCWD: AtomicPtr<c_void> = AtomicPtr::new(ptr::null_mut());
static REAL_CHDIR: AtomicPtr<c_void> = AtomicPtr::new(ptr::null_mut());
static REAL_UNLINK: AtomicPtr<c_void> = AtomicPtr::new(ptr::null_mut());
static REAL_RENAME: AtomicPtr<c_void> = AtomicPtr::new(ptr::null_mut());
static REAL_RMDIR: AtomicPtr<c_void> = AtomicPtr::new(ptr::null_mut());
static REAL_UTIMENSAT: AtomicPtr<c_void> = AtomicPtr::new(ptr::null_mut());
static REAL_MKDIR: AtomicPtr<c_void> = AtomicPtr::new(ptr::null_mut());
static REAL_SYMLINK: AtomicPtr<c_void> = AtomicPtr::new(ptr::null_mut());
static REAL_LINK: AtomicPtr<c_void> = AtomicPtr::new(ptr::null_mut());
static REAL_LINKAT: AtomicPtr<c_void> = AtomicPtr::new(ptr::null_mut());
static REAL_RENAMEAT: AtomicPtr<c_void> = AtomicPtr::new(ptr::null_mut());

#[cfg(target_os = "linux")]
#[no_mangle]
pub unsafe extern "C" fn readlink(p: *const c_char, b: *mut c_char, s: size_t) -> ssize_t {
    let real = get_real!(REAL_READLINK, "readlink", ReadlinkFn);
    readlink_impl(p, b, s).unwrap_or_else(|| real(p, b, s))
}

#[cfg(target_os = "macos")]
#[no_mangle]
pub unsafe extern "C" fn readlink_shim(p: *const c_char, b: *mut c_char, s: size_t) -> ssize_t {
    let real = std::mem::transmute::<*const (), ReadlinkFn>(IT_READLINK.old_func);
    readlink_impl(p, b, s).unwrap_or_else(|| real(p, b, s))
}

#[no_mangle]
pub unsafe extern "C" fn realpath_shim(
    pathname: *const c_char,
    resolved_path: *mut c_char,
) -> *mut c_char {
    let _guard = match ShimGuard::enter() {
        Some(g) => g,
        None => {
            let real = get_real_shim!(REAL_REALPATH, "realpath", IT_REALPATH, RealpathFn);
            return real(pathname, resolved_path);
        }
    };

    if pathname.is_null() {
        return ptr::null_mut();
    }

    let path = CStr::from_ptr(pathname).to_string_lossy();
    if let Some(state) = ShimState::get() {
        if state.psfs_applicable(&path) {
            let mut buf = [0u8; 1024];
            if let Some(len) = resolve_path_with_cwd(&path, &mut buf) {
                let result = if resolved_path.is_null() {
                    libc::malloc(len + 1) as *mut c_char
                } else {
                    resolved_path
                };
                if !result.is_null() {
                    ptr::copy_nonoverlapping(buf.as_ptr(), result as *mut u8, len);
                    *result.add(len) = 0;
                    return result;
                }
            }
        }
    }

    let real = get_real_shim!(REAL_REALPATH, "realpath", IT_REALPATH, RealpathFn);
    real(pathname, resolved_path)
}

#[no_mangle]
pub unsafe extern "C" fn getcwd_shim(buf: *mut c_char, size: size_t) -> *mut c_char {
    let _guard = match ShimGuard::enter() {
        Some(g) => g,
        None => {
            let real = get_real_shim!(REAL_GETCWD, "getcwd", IT_GETCWD, GetcwdFn);
            return real(buf, size);
        }
    };

    if let Some(vpath) = get_virtual_cwd() {
        let vbytes = vpath.as_bytes();
        if size != 0 && size < vbytes.len() + 1 {
            set_errno(libc::ERANGE);
            return ptr::null_mut();
        }
        let result = if buf.is_null() {
            libc::malloc(vbytes.len() + 1) as *mut c_char
        } else {
            buf
        };
        if !result.is_null() {
            ptr::copy_nonoverlapping(vbytes.as_ptr(), result as *mut u8, vbytes.len());
            *result.add(vbytes.len()) = 0;
            return result;
        }
    }

    let real = get_real_shim!(REAL_GETCWD, "getcwd", IT_GETCWD, GetcwdFn);
    real(buf, size)
}

#[no_mangle]
pub unsafe extern "C" fn chdir_shim(path: *const c_char) -> c_int {
    let _guard = match ShimGuard::enter() {
        Some(g) => g,
        None => {
            let real = get_real_shim!(REAL_CHDIR, "chdir", IT_CHDIR, ChdirFn);
            return real(path);
        }
    };

    if path.is_null() {
        set_errno(libc::EFAULT);
        return -1;
    }

    let path_str = CStr::from_ptr(path).to_string_lossy();
    if let Some(state) = ShimState::get() {
        let mut path_buf = [0u8; 1024];
        if let Some(len) = resolve_path_with_cwd(&path_str, &mut path_buf) {
            let resolved_path = unsafe { std::str::from_utf8_unchecked(&path_buf[..len]) };

            // RFC-0043: Robust virtualization support
            if resolved_path.starts_with(&*state.vfs_prefix) {
                // Check if it exists and is a directory in manifest
                if let Some(entry) = state.psfs_lookup(resolved_path) {
                    if (entry.mode as u32 & libc::S_IFMT as u32) == libc::S_IFDIR as u32 {
                        set_virtual_cwd(Some(resolved_path.to_string()));
                        return 0;
                    } else {
                        set_errno(libc::ENOTDIR);
                        return -1;
                    }
                } else {
                    set_errno(libc::ENOENT);
                    return -1;
                }
            } else {
                // Moving out of virtual domain - clear virtual CWD
                set_virtual_cwd(None);
            }
        }
    }

    let real = get_real_shim!(REAL_CHDIR, "chdir", IT_CHDIR, ChdirFn);
    real(path)
}

#[no_mangle]
pub unsafe extern "C" fn unlink_shim(path: *const c_char) -> c_int {
    let _guard = match ShimGuard::enter() {
        Some(g) => g,
        None => {
            let real = get_real_shim!(REAL_UNLINK, "unlink", IT_UNLINK, UnlinkFn);
            return real(path);
        }
    };

    if let Some(state) = ShimState::get() {
        let path_str = CStr::from_ptr(path).to_string_lossy();
        let mut path_buf = [0u8; 1024];
        if let Some(len) = resolve_path_with_cwd(&path_str, &mut path_buf) {
            let resolved_path = unsafe { std::str::from_utf8_unchecked(&path_buf[..len]) };
            if resolved_path.starts_with(&*state.vfs_prefix) {
                // RFC-0047: Remove from Manifest via IPC instead of EROFS
                if sync_ipc_manifest_remove(&state.socket_path, resolved_path) {
                    return 0; // Success - manifest entry removed
                }
                // IPC failed - fallback to real unlink
            }
        }
    }

    let real = get_real_shim!(REAL_UNLINK, "unlink", IT_UNLINK, UnlinkFn);
    real(path)
}

#[no_mangle]
pub unsafe extern "C" fn rename_shim(oldpath: *const c_char, newpath: *const c_char) -> c_int {
    let _guard = match ShimGuard::enter() {
        Some(g) => g,
        None => {
            let real = get_real_shim!(REAL_RENAME, "rename", IT_RENAME, RenameFn);
            return real(oldpath, newpath);
        }
    };

    if let Some(state) = ShimState::get() {
        let old_str = CStr::from_ptr(oldpath).to_string_lossy();
        let new_str = CStr::from_ptr(newpath).to_string_lossy();
        let mut buf_old = [0u8; 1024];
        let mut buf_new = [0u8; 1024];

        let old_res = resolve_path_with_cwd(&old_str, &mut buf_old);
        let new_res = resolve_path_with_cwd(&new_str, &mut buf_new);

        let old_is_vfs = old_res.map_or(false, |len| {
            let p = unsafe { std::str::from_utf8_unchecked(&buf_old[..len]) };
            p.starts_with(&*state.vfs_prefix)
        });
        let new_is_vfs = new_res.map_or(false, |len| {
            let p = unsafe { std::str::from_utf8_unchecked(&buf_new[..len]) };
            p.starts_with(&*state.vfs_prefix)
        });

        if old_is_vfs && new_is_vfs {
            // RFC-0047: Pure VFS rename - Update Manifest via IPC
            let old_resolved = old_res
                .map(|len| unsafe { std::str::from_utf8_unchecked(&buf_old[..len]) })
                .unwrap_or("");
            let new_resolved = new_res
                .map(|len| unsafe { std::str::from_utf8_unchecked(&buf_new[..len]) })
                .unwrap_or("");
            if sync_ipc_manifest_rename(&state.socket_path, old_resolved, new_resolved) {
                return 0; // Success - manifest entry renamed
            }
            // IPC failed - fallback to real rename (risky but consistent with failure)
        } else if old_is_vfs != new_is_vfs {
            // Cross-Domain Rename (VFS <-> Host)
            // MUST return EXDEV to trigger `mv` fallback to copy + unlink
            set_errno(libc::EXDEV);
            return -1;
        }
    }

    let real = get_real_shim!(REAL_RENAME, "rename", IT_RENAME, RenameFn);
    real(oldpath, newpath)
}

#[no_mangle]
pub unsafe extern "C" fn renameat_shim(
    olddirfd: c_int,
    oldpath: *const c_char,
    newdirfd: c_int,
    newpath: *const c_char,
) -> c_int {
    let _guard = match ShimGuard::enter() {
        Some(g) => g,
        None => {
            let real = get_real_shim!(REAL_RENAMEAT, "renameat", IT_RENAMEAT, RenameatFn);
            return real(olddirfd, oldpath, newdirfd, newpath);
        }
    };

    if let Some(state) = ShimState::get() {
        let mut buf_old = [0u8; 1024];
        let mut buf_new = [0u8; 1024];

        let old_res = resolve_path_at(olddirfd, oldpath, &mut buf_old);
        let new_res = resolve_path_at(newdirfd, newpath, &mut buf_new);

        let old_is_vfs = old_res.map_or(false, |len| {
            let p = std::str::from_utf8_unchecked(&buf_old[..len]);
            p.starts_with(&*state.vfs_prefix)
        });
        let new_is_vfs = new_res.map_or(false, |len| {
            let p = std::str::from_utf8_unchecked(&buf_new[..len]);
            p.starts_with(&*state.vfs_prefix)
        });

        if old_is_vfs && new_is_vfs {
            // Pure VFS renameat
            let old_resolved = old_res
                .map(|len| std::str::from_utf8_unchecked(&buf_old[..len]))
                .unwrap_or("");
            let new_resolved = new_res
                .map(|len| std::str::from_utf8_unchecked(&buf_new[..len]))
                .unwrap_or("");
            if sync_ipc_manifest_rename(&state.socket_path, old_resolved, new_resolved) {
                return 0;
            }
        } else if old_is_vfs != new_is_vfs {
            // Cross-Domain
            set_errno(libc::EXDEV);
            return -1;
        }
    }

    let real = get_real_shim!(REAL_RENAMEAT, "renameat", IT_RENAMEAT, RenameatFn);
    real(olddirfd, oldpath, newdirfd, newpath)
}

#[no_mangle]
pub unsafe extern "C" fn link_shim(oldpath: *const c_char, newpath: *const c_char) -> c_int {
    let _guard = match ShimGuard::enter() {
        Some(g) => g,
        None => {
            let real = get_real_shim!(REAL_LINK, "link", IT_LINK, LinkFn);
            return real(oldpath, newpath);
        }
    };

    if let Some(state) = ShimState::get() {
        let mut buf = [0u8; 1024];
        // Check oldpath (target)
        if let Some(len) =
            resolve_path_with_cwd(CStr::from_ptr(oldpath).to_str().unwrap_or(""), &mut buf)
        {
            let p = std::str::from_utf8_unchecked(&buf[..len]);
            if p.starts_with(&*state.vfs_prefix) {
                // VFS Hard Link -> EXDEV (Block)
                set_errno(libc::EXDEV);
                return -1;
            }
        }
        // Check newpath (destination) - if creating link INSIDE vfs, also block?
        // Yes, VFS is read-only for structure changes via hardlink.
        if let Some(len) =
            resolve_path_with_cwd(CStr::from_ptr(newpath).to_str().unwrap_or(""), &mut buf)
        {
            let p = std::str::from_utf8_unchecked(&buf[..len]);
            if p.starts_with(&*state.vfs_prefix) {
                set_errno(libc::EXDEV);
                return -1;
            }
        }
    }

    let real = get_real_shim!(REAL_LINK, "link", IT_LINK, LinkFn);
    real(oldpath, newpath)
}

#[no_mangle]
pub unsafe extern "C" fn linkat_shim(
    olddirfd: c_int,
    oldpath: *const c_char,
    newdirfd: c_int,
    newpath: *const c_char,
    flags: c_int,
) -> c_int {
    let _guard = match ShimGuard::enter() {
        Some(g) => g,
        None => {
            let real = get_real_shim!(REAL_LINKAT, "linkat", IT_LINKAT, LinkatFn);
            return real(olddirfd, oldpath, newdirfd, newpath, flags);
        }
    };

    if let Some(state) = ShimState::get() {
        let mut buf = [0u8; 1024];
        if let Some(len) = resolve_path_at(olddirfd, oldpath, &mut buf) {
            let p = std::str::from_utf8_unchecked(&buf[..len]);
            if p.starts_with(&*state.vfs_prefix) {
                set_errno(libc::EXDEV);
                return -1;
            }
        }
        if let Some(len) = resolve_path_at(newdirfd, newpath, &mut buf) {
            let p = std::str::from_utf8_unchecked(&buf[..len]);
            if p.starts_with(&*state.vfs_prefix) {
                set_errno(libc::EXDEV);
                return -1;
            }
        }
    }

    let real = get_real_shim!(REAL_LINKAT, "linkat", IT_LINKAT, LinkatFn);
    real(olddirfd, oldpath, newdirfd, newpath, flags)
}

#[no_mangle]
pub unsafe extern "C" fn rmdir_shim(path: *const c_char) -> c_int {
    let _guard = match ShimGuard::enter() {
        Some(g) => g,
        None => {
            let real = get_real_shim!(REAL_RMDIR, "rmdir", IT_RMDIR, RmdirFn);
            return real(path);
        }
    };

    if let Some(state) = ShimState::get() {
        let path_str = CStr::from_ptr(path).to_string_lossy();
        let mut path_buf = [0u8; 1024];
        if let Some(len) = resolve_path_with_cwd(&path_str, &mut path_buf) {
            let resolved_path = unsafe { std::str::from_utf8_unchecked(&path_buf[..len]) };
            if resolved_path.starts_with(&*state.vfs_prefix) {
                // RFC-0047: Remove dir from Manifest via IPC instead of EROFS
                if sync_ipc_manifest_remove(&state.socket_path, resolved_path) {
                    return 0; // Success - manifest dir entry removed
                }
                // IPC failed - fallback to real rmdir
            }
        }
    }

    let real = get_real_shim!(REAL_RMDIR, "rmdir", IT_RMDIR, RmdirFn);
    real(path)
}

/// RFC-0047: utimensat shim - update Manifest mtime for VFS paths
/// This is critical for incremental builds (touch, make)
#[no_mangle]
pub unsafe extern "C" fn utimensat_shim(
    dirfd: c_int,
    path: *const c_char,
    times: *const libc::timespec,
    flags: c_int,
) -> c_int {
    let _guard = match ShimGuard::enter() {
        Some(g) => g,
        None => {
            let real = get_real_shim!(REAL_UTIMENSAT, "utimensat", IT_UTIMENSAT, UtimensatFn);
            return real(dirfd, path, times, flags);
        }
    };

    if let Some(state) = ShimState::get() {
        if !path.is_null() {
            let path_str = CStr::from_ptr(path).to_string_lossy();
            let mut path_buf = [0u8; 1024];
            if let Some(len) = resolve_path_with_cwd(&path_str, &mut path_buf) {
                let resolved_path = std::str::from_utf8_unchecked(&path_buf[..len]);
                if resolved_path.starts_with(&*state.vfs_prefix) {
                    // Extract mtime from times array (times[1] is mtime)
                    let mtime_ns = if times.is_null() {
                        // UTIME_NOW: use current time
                        use std::time::{SystemTime, UNIX_EPOCH};
                        SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .map(|d| d.as_nanos() as u64)
                            .unwrap_or(0)
                    } else {
                        let mtime = &*times.add(1); // times[1] = mtime
                        if mtime.tv_nsec == libc::UTIME_NOW as i64 {
                            use std::time::{SystemTime, UNIX_EPOCH};
                            SystemTime::now()
                                .duration_since(UNIX_EPOCH)
                                .map(|d| d.as_nanos() as u64)
                                .unwrap_or(0)
                        } else if mtime.tv_nsec == libc::UTIME_OMIT as i64 {
                            // UTIME_OMIT: don't update mtime, passthrough
                            let real = get_real_shim!(
                                REAL_UTIMENSAT,
                                "utimensat",
                                IT_UTIMENSAT,
                                UtimensatFn
                            );
                            return real(dirfd, path, times, flags);
                        } else {
                            (mtime.tv_sec as u64) * 1_000_000_000 + (mtime.tv_nsec as u64)
                        }
                    };

                    // RFC-0047: Update Manifest mtime via IPC
                    if sync_ipc_manifest_update_mtime(&state.socket_path, resolved_path, mtime_ns) {
                        return 0; // Success - manifest mtime updated
                    }
                    // IPC failed - fallback to real utimensat
                }
            }
        }
    }

    let real = get_real_shim!(REAL_UTIMENSAT, "utimensat", IT_UTIMENSAT, UtimensatFn);
    real(dirfd, path, times, flags)
}

/// RFC-0047 P1: mkdir shim - create directory entry in Manifest for VFS paths
#[no_mangle]
pub unsafe extern "C" fn mkdir_shim(path: *const c_char, mode: mode_t) -> c_int {
    let _guard = match ShimGuard::enter() {
        Some(g) => g,
        None => {
            let real = get_real_shim!(REAL_MKDIR, "mkdir", IT_MKDIR, MkdirFn);
            return real(path, mode);
        }
    };

    if let Some(state) = ShimState::get() {
        if !path.is_null() {
            let path_str = CStr::from_ptr(path).to_string_lossy();
            let mut path_buf = [0u8; 1024];
            if let Some(len) = resolve_path_with_cwd(&path_str, &mut path_buf) {
                let resolved_path = std::str::from_utf8_unchecked(&path_buf[..len]);
                if resolved_path.starts_with(&*state.vfs_prefix) {
                    // RFC-0047 P1: Create directory entry in Manifest via IPC
                    if sync_ipc_manifest_mkdir(&state.socket_path, resolved_path, mode as u32) {
                        return 0; // Success - manifest dir entry created
                    }
                    // IPC failed - fallback to real mkdir
                }
            }
        }
    }

    let real = get_real_shim!(REAL_MKDIR, "mkdir", IT_MKDIR, MkdirFn);
    real(path, mode)
}

/// RFC-0047 P1: symlink shim - create symlink entry in Manifest for VFS paths
#[no_mangle]
pub unsafe extern "C" fn symlink_shim(target: *const c_char, linkpath: *const c_char) -> c_int {
    let _guard = match ShimGuard::enter() {
        Some(g) => g,
        None => {
            let real = get_real_shim!(REAL_SYMLINK, "symlink", IT_SYMLINK, SymlinkFn);
            return real(target, linkpath);
        }
    };

    if let Some(state) = ShimState::get() {
        if !linkpath.is_null() {
            let link_str = CStr::from_ptr(linkpath).to_string_lossy();
            let mut path_buf = [0u8; 1024];
            if let Some(len) = resolve_path_with_cwd(&link_str, &mut path_buf) {
                let resolved_path = std::str::from_utf8_unchecked(&path_buf[..len]);
                if resolved_path.starts_with(&*state.vfs_prefix) {
                    // RFC-0047 P1: Create symlink entry in Manifest via IPC
                    let target_str = if target.is_null() {
                        ""
                    } else {
                        CStr::from_ptr(target).to_str().unwrap_or("")
                    };
                    if sync_ipc_manifest_symlink(&state.socket_path, resolved_path, target_str) {
                        return 0; // Success - manifest symlink entry created
                    }
                    // IPC failed - fallback to real symlink
                }
            }
        }
    }

    let real = get_real_shim!(REAL_SYMLINK, "symlink", IT_SYMLINK, SymlinkFn);
    real(target, linkpath)
}

#[cfg(target_os = "linux")]
unsafe fn set_errno(e: c_int) {
    *libc::__errno_location() = e;
}
#[cfg(target_os = "macos")]
unsafe fn set_errno(e: c_int) {
    *libc::__error() = e;
}
