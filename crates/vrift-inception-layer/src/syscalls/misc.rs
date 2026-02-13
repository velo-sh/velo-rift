use crate::state::*;
#[cfg(target_os = "macos")]
use libc::c_void;
use libc::{c_char, c_int};
use std::ffi::CStr;
use std::sync::atomic::Ordering;

/// RFC-0047: Rename implementation with VFS boundary enforcement
/// Returns EXDEV (18) for cross-domain renames
unsafe fn rename_impl(old: *const c_char, new: *const c_char) -> Option<c_int> {
    if old.is_null() || new.is_null() {
        return None;
    }

    let _guard = InceptionLayerGuard::enter()?;
    let state = InceptionLayerState::get()?;

    let old_str = CStr::from_ptr(old).to_str().ok()?;
    let new_str = CStr::from_ptr(new).to_str().ok()?;

    let old_in_vfs = state.inception_applicable(old_str);
    let new_in_vfs = state.inception_applicable(new_str);

    // RFC-0047: Cross-boundary rename is forbidden
    if old_in_vfs != new_in_vfs {
        crate::set_errno(libc::EXDEV);
        return Some(-1);
    }

    // Both in VFS territory -> Virtual Rename via Daemon IPC
    if old_in_vfs && new_in_vfs {
        if let (Some(v1), Some(v2)) = (state.resolve_path(old_str), state.resolve_path(new_str)) {
            // RFC-0047: Only use Virtual Rename for managed files.
            // For local files in VFS territory, let raw_rename handle it.
            if state.query_manifest_ipc(&v1).is_some() {
                if state
                    .manifest_rename(&v1.manifest_key, &v2.manifest_key)
                    .is_ok()
                {
                    return Some(0);
                }
                crate::set_errno(libc::EPERM);
                return Some(-1);
            }
        }
        return None; // Fallback to raw syscall for local files
    }

    None // Let real syscall handle non-VFS renames
}

#[no_mangle]
#[cfg(target_os = "macos")]
pub unsafe extern "C" fn rename_inception(old: *const c_char, new: *const c_char) -> c_int {
    extern "C" {
        fn c_rename_bridge(old: *const c_char, new: *const c_char) -> c_int;
    }
    c_rename_bridge(old, new)
}

#[no_mangle]
pub unsafe extern "C" fn velo_rename_impl(old: *const c_char, new: *const c_char) -> c_int {
    // RFC-0047 logic + fallback to raw
    if let Some(res) = rename_impl(old, new) {
        return res;
    }
    #[cfg(target_os = "macos")]
    {
        crate::syscalls::macos_raw::raw_rename(old, new)
    }
    #[cfg(target_os = "linux")]
    {
        crate::syscalls::linux_raw::raw_rename(old, new)
    }
}

/// Linux-specific rename inception call
#[cfg(target_os = "linux")]
#[no_mangle]
pub unsafe extern "C" fn rename_inception_linux(old: *const c_char, new: *const c_char) -> c_int {
    if let Some(res) = rename_impl(old, new) {
        return res;
    }
    crate::syscalls::linux_raw::raw_rename(old, new)
}

#[cfg(target_os = "linux")]
#[no_mangle]
pub unsafe extern "C" fn renameat_inception_linux(
    oldfd: c_int,
    old: *const c_char,
    newfd: c_int,
    new: *const c_char,
) -> c_int {
    if let Some(res) = renameat_impl(old, new) {
        return res;
    }
    crate::syscalls::linux_raw::raw_renameat(oldfd, old, newfd, new)
}

#[no_mangle]
#[cfg(target_os = "macos")]
pub unsafe extern "C" fn renameat_inception(
    oldfd: c_int,
    old: *const c_char,
    newfd: c_int,
    new: *const c_char,
) -> c_int {
    extern "C" {
        fn c_renameat_bridge(
            oldfd: c_int,
            old: *const c_char,
            newfd: c_int,
            new: *const c_char,
        ) -> c_int;
    }
    c_renameat_bridge(oldfd, old, newfd, new)
}

#[no_mangle]
pub unsafe extern "C" fn velo_renameat_impl(
    oldfd: c_int,
    old: *const c_char,
    newfd: c_int,
    new: *const c_char,
) -> c_int {
    // Resolve relative paths using getcwd for AT_FDCWD case
    if oldfd == libc::AT_FDCWD && newfd == libc::AT_FDCWD {
        if let Some(result) = renameat_impl(old, new) {
            return result;
        }
    }
    #[cfg(target_os = "macos")]
    {
        crate::syscalls::macos_raw::raw_renameat(oldfd, old, newfd, new)
    }
    #[cfg(target_os = "linux")]
    {
        crate::syscalls::linux_raw::raw_renameat(oldfd, old, newfd, new)
    }
}

/// renameat path resolution helper - resolves relative paths to absolute
unsafe fn renameat_impl(old: *const c_char, new: *const c_char) -> Option<c_int> {
    if old.is_null() || new.is_null() {
        return None;
    }

    let _guard = InceptionLayerGuard::enter()?;
    let state = InceptionLayerState::get()?;

    let old_str = CStr::from_ptr(old).to_str().ok()?;
    let new_str = CStr::from_ptr(new).to_str().ok()?;

    // Resolve relative paths via getcwd
    let resolve_path = |path: &str| -> Option<String> {
        if path.starts_with('/') {
            Some(path.to_string())
        } else {
            let mut buf = [0u8; 1024];
            let cwd = libc::getcwd(buf.as_mut_ptr().cast(), buf.len());
            if cwd.is_null() {
                None
            } else {
                let cwd_str = CStr::from_ptr(cwd).to_str().ok()?;
                Some(format!("{}/{}", cwd_str, path))
            }
        }
    };

    let old_abs = resolve_path(old_str)?;
    let new_abs = resolve_path(new_str)?;

    let old_in_vfs = state.inception_applicable(&old_abs);
    let new_in_vfs = state.inception_applicable(&new_abs);

    // RFC-0047: Cross-boundary rename is forbidden
    if old_in_vfs != new_in_vfs {
        crate::set_errno(libc::EXDEV);
        return Some(-1);
    }

    None // Let real syscall handle
}

/// Helper to block mutation on VFS-managed files via FD
pub(crate) unsafe fn quick_block_vfs_fd_mutation(fd: c_int) -> Option<c_int> {
    let _guard = InceptionLayerGuard::enter()?;
    let state = InceptionLayerState::get()?;

    // 1. Try to resolve FD to path (OS specific)
    #[cfg(target_os = "macos")]
    {
        let mut path_buf = [0i8; 1024];
        if libc::fcntl(fd, libc::F_GETPATH, path_buf.as_mut_ptr()) == 0 {
            let path_cstr = CStr::from_ptr(path_buf.as_ptr());
            if let Ok(path_str) = path_cstr.to_str() {
                if state.inception_applicable(path_str) {
                    crate::set_errno(libc::EPERM);
                    return Some(-1);
                }
            }
        }
    }
    #[cfg(target_os = "linux")]
    {
        let fd_path = format!("/proc/self/fd/{}\0", fd);
        let mut path_buf = [0u8; 1024];
        let n = libc::readlink(
            fd_path.as_ptr() as *const c_char,
            path_buf.as_mut_ptr() as *mut c_char,
            path_buf.len(),
        );
        if n > 0 && (n as usize) < path_buf.len() {
            if let Ok(path_str) = std::str::from_utf8(&path_buf[..n as usize]) {
                if state.inception_applicable(path_str) {
                    crate::set_errno(libc::EPERM);
                    return Some(-1);
                }
            }
        }
    }

    // 2. Fallback: Check FD tracking table
    if crate::syscalls::io::is_vfs_fd(fd) {
        crate::set_errno(libc::EPERM);
        return Some(-1);
    }

    None
}

#[no_mangle]
#[cfg(target_os = "macos")]
pub unsafe extern "C" fn futimes_inception(fd: c_int, times: *const libc::timeval) -> c_int {
    if let Some(err) = quick_block_vfs_fd_mutation(fd) {
        return err;
    }
    crate::syscalls::macos_raw::raw_futimes(fd, times)
}

#[no_mangle]
#[cfg(target_os = "linux")]
pub unsafe extern "C" fn futimes_inception(fd: c_int, times: *const libc::timeval) -> c_int {
    if let Some(err) = quick_block_vfs_fd_mutation(fd) {
        return err;
    }
    // Linux: use raw syscall to avoid LD_PRELOAD recursion
    crate::syscalls::linux_raw::raw_futimes(fd, times)
}

#[no_mangle]
pub unsafe extern "C" fn futimens_inception(fd: c_int, times: *const libc::timespec) -> c_int {
    // Early-init guard: during bootstrap, passthrough to real libc
    let init_state = INITIALIZING.load(Ordering::Relaxed);
    if init_state != 0
        || crate::state::INCEPTION_LAYER_STATE
            .load(std::sync::atomic::Ordering::Acquire)
            .is_null()
    {
        #[cfg(target_os = "macos")]
        return crate::syscalls::macos_raw::raw_futimens(fd, times);
        #[cfg(target_os = "linux")]
        return crate::syscalls::linux_raw::raw_utimensat(fd, std::ptr::null(), times, 0);
    }

    let _guard = match InceptionLayerGuard::enter() {
        Some(g) => g,
        None => {
            #[cfg(target_os = "macos")]
            return crate::syscalls::macos_raw::raw_futimens(fd, times);
            #[cfg(target_os = "linux")]
            return crate::syscalls::linux_raw::raw_utimensat(fd, std::ptr::null(), times, 0);
        }
    };

    if let Some(err) = quick_block_vfs_fd_mutation(fd) {
        return err;
    }

    #[cfg(target_os = "macos")]
    return crate::syscalls::macos_raw::raw_futimens(fd, times);
    #[cfg(target_os = "linux")]
    return crate::syscalls::linux_raw::raw_utimensat(fd, std::ptr::null(), times, 0);
}

#[no_mangle]
pub unsafe extern "C" fn utimensat_inception(
    dirfd: c_int,
    path: *const c_char,
    times: *const libc::timespec,
    flags: c_int,
) -> c_int {
    // Early-init guard: during bootstrap, passthrough to real libc
    let init_state = INITIALIZING.load(Ordering::Relaxed);
    if init_state != 0
        || crate::state::INCEPTION_LAYER_STATE
            .load(std::sync::atomic::Ordering::Acquire)
            .is_null()
    {
        if let Some(err) = quick_block_vfs_mutation(path) {
            return err;
        }
        #[cfg(target_os = "macos")]
        return crate::syscalls::macos_raw::raw_utimensat(dirfd, path, times, flags);
        #[cfg(target_os = "linux")]
        return crate::syscalls::linux_raw::raw_utimensat(dirfd, path, times, flags);
    }

    let _guard = match InceptionLayerGuard::enter() {
        Some(g) => g,
        None => {
            #[cfg(target_os = "macos")]
            return crate::syscalls::macos_raw::raw_utimensat(dirfd, path, times, flags);
            #[cfg(target_os = "linux")]
            return crate::syscalls::linux_raw::raw_utimensat(dirfd, path, times, flags);
        }
    };
    // RFC-0039: Block timestamp mutation on VFS-managed files (returns EPERM)
    block_vfs_mutation(path).unwrap_or_else(|| {
        #[cfg(target_os = "macos")]
        return crate::syscalls::macos_raw::raw_utimensat(dirfd, path, times, flags);
        #[cfg(target_os = "linux")]
        return crate::syscalls::linux_raw::raw_utimensat(dirfd, path, times, flags);
    })
}

#[no_mangle]
#[cfg(target_os = "macos")]
pub unsafe extern "C" fn fchflags_inception(fd: c_int, flags: libc::c_uint) -> c_int {
    if let Some(err) = quick_block_vfs_fd_mutation(fd) {
        return err;
    }
    crate::syscalls::macos_raw::raw_fchflags(fd, flags)
}

/// RFC-0047: Link (hardlink) implementation with VFS boundary enforcement
/// Cross-boundary hardlinks are forbidden (EXDEV). Intra-VFS hard links on
/// manifest entries are also blocked (EXDEV) to preserve CAS integrity.
unsafe fn link_impl(old: *const c_char, new: *const c_char) -> Option<c_int> {
    if old.is_null() || new.is_null() {
        return None;
    }

    let _guard = InceptionLayerGuard::enter()?;
    let state = InceptionLayerState::get()?;

    let old_str = CStr::from_ptr(old).to_str().ok()?;
    let new_str = CStr::from_ptr(new).to_str().ok()?;

    let old_in_vfs = state.inception_applicable(old_str);
    let new_in_vfs = state.inception_applicable(new_str);

    // RFC-0047: Cross-boundary hardlink is forbidden
    if old_in_vfs != new_in_vfs {
        crate::set_errno(libc::EXDEV);
        return Some(-1);
    }

    // Block intra-VFS hardlinks on manifest entries (CAS integrity)
    if old_in_vfs && new_in_vfs {
        if let Some(vpath) = state.resolve_path(old_str) {
            if vdir_lookup(state.mmap_ptr, state.mmap_size, &vpath.manifest_key).is_some() {
                crate::set_errno(libc::EXDEV);
                return Some(-1);
            }
        }
    }

    None // Non-VFS or local files: passthrough
}

#[no_mangle]
#[cfg(target_os = "macos")]
pub unsafe extern "C" fn link_inception(old: *const c_char, new: *const c_char) -> c_int {
    // RFC-0047: Cross-boundary hardlink ALWAYS returns EXDEV
    // This check must happen BEFORE any other logic, regardless of init state
    let old_in_vfs = quick_is_in_vfs(old);
    let new_in_vfs = quick_is_in_vfs(new);
    if old_in_vfs != new_in_vfs {
        // Cross-device link: one path in VFS, other outside
        crate::set_errno(libc::EXDEV);
        return -1;
    }

    let init_state = INITIALIZING.load(Ordering::Relaxed);
    if init_state != 0
        || crate::state::INCEPTION_LAYER_STATE
            .load(Ordering::Acquire)
            .is_null()
    {
        // Early init: passthrough
        return crate::syscalls::macos_raw::raw_link(old, new);
    }
    // Post-init: use link_impl for manifest-aware checks
    if let Some(err) = link_impl(old, new) {
        return err;
    }
    // Block intra-VFS links via block_vfs_mutation fallback
    if let Some(err) = block_vfs_mutation(old).or_else(|| block_vfs_mutation(new)) {
        return err;
    }
    // Non-VFS or intra-VFS local files: passthrough to raw link
    let rc = crate::syscalls::macos_raw::raw_link(old, new);
    if rc == -1 && crate::get_errno() == libc::EPERM {
        // Destination may be a CAS-materialized file with uchg flag.
        // Clear flags, unlink, and retry.
        crate::syscalls::macos_raw::raw_chflags(new, 0);
        crate::syscalls::macos_raw::raw_unlink(new);
        return crate::syscalls::macos_raw::raw_link(old, new);
    }
    rc
}

#[no_mangle]
#[cfg(target_os = "linux")]
pub unsafe extern "C" fn link_inception(old: *const c_char, new: *const c_char) -> c_int {
    // RFC-0047: Cross-boundary hardlink ALWAYS returns EXDEV
    let old_in_vfs = quick_is_in_vfs(old);
    let new_in_vfs = quick_is_in_vfs(new);
    if old_in_vfs != new_in_vfs {
        crate::set_errno(libc::EXDEV);
        return -1;
    }

    let init_state = INITIALIZING.load(Ordering::Relaxed);
    if init_state != 0
        || crate::state::INCEPTION_LAYER_STATE
            .load(Ordering::Acquire)
            .is_null()
    {
        return crate::syscalls::linux_raw::raw_link(old, new);
    }
    // Post-init: use link_impl for manifest-aware checks
    if let Some(err) = link_impl(old, new) {
        return err;
    }
    // Non-VFS or intra-VFS local files: passthrough to raw link
    crate::syscalls::linux_raw::raw_link(old, new)
}

#[no_mangle]
pub unsafe extern "C" fn linkat_inception(
    olddirfd: c_int,
    oldpath: *const c_char,
    newdirfd: c_int,
    path: *const c_char,
    flags: c_int,
) -> c_int {
    let init_state = INITIALIZING.load(Ordering::Relaxed);
    if init_state != 0
        || crate::state::INCEPTION_LAYER_STATE
            .load(Ordering::Acquire)
            .is_null()
    {
        if let Some(err) =
            quick_block_vfs_mutation(oldpath).or_else(|| quick_block_vfs_mutation(path))
        {
            return err;
        }
        #[cfg(target_os = "macos")]
        return crate::syscalls::macos_raw::raw_linkat(olddirfd, oldpath, newdirfd, path, flags);
        #[cfg(target_os = "linux")]
        return crate::syscalls::linux_raw::raw_linkat(olddirfd, oldpath, newdirfd, path, flags);
    }
    if let Some(res) = link_impl(oldpath, path) {
        return res;
    }
    block_vfs_mutation(oldpath)
        .or_else(|| block_vfs_mutation(path))
        .unwrap_or_else(|| {
            #[cfg(target_os = "macos")]
            return crate::syscalls::macos_raw::raw_linkat(
                olddirfd, oldpath, newdirfd, path, flags,
            );
            #[cfg(target_os = "linux")]
            return crate::syscalls::linux_raw::raw_linkat(
                olddirfd, oldpath, newdirfd, path, flags,
            );
        })
}

// RFC-0047: Mutation Perimeter - Block modifications to VFS-managed files

#[no_mangle]
#[cfg(target_os = "macos")]
pub unsafe extern "C" fn unlink_inception(path: *const c_char) -> c_int {
    // Early-init guard: passthrough during bootstrap
    let init_state = INITIALIZING.load(Ordering::Relaxed);
    if init_state != 0
        || crate::state::INCEPTION_LAYER_STATE
            .load(Ordering::Acquire)
            .is_null()
    {
        if let Some(err) = quick_block_vfs_mutation(path) {
            return err;
        }
        return crate::syscalls::macos_raw::raw_unlink(path);
    }

    // Block unlink on manifest-tracked files (Tier-1 immutable)
    block_vfs_mutation(path).unwrap_or_else(|| crate::syscalls::macos_raw::raw_unlink(path))
}

#[no_mangle]
#[cfg(target_os = "macos")]
pub unsafe extern "C" fn remove_inception(path: *const c_char) -> c_int {
    let path_str = CStr::from_ptr(path).to_string_lossy();

    let state = match InceptionLayerState::get() {
        Some(s) => s,
        None => return libc::remove(path),
    };

    let vpath = match state.resolve_path(&path_str) {
        Some(v) => v,
        None => return libc::remove(path),
    };

    let res = libc::remove(path);

    if res == -1
        && crate::get_errno() == libc::ENOENT
        && vdir_list_dir(state.mmap_ptr, state.mmap_size, &vpath.manifest_key)
            .map(|v| !v.is_empty())
            .unwrap_or(false)
    {
        return 0;
    }

    res
}

/// BUG-016: Intercept fclonefileat to fix uchg/perms on CAS-cloned build artifacts.
/// Cargo's fs::hard_link on macOS uses fclonefileat (CoW clone) instead of linkat.
/// The cloned file inherits CAS blob's uchg flag and 0o444 mode, causing "Permission
/// denied" when Cargo tries to execute build scripts.
#[no_mangle]
#[cfg(target_os = "macos")]
pub unsafe extern "C" fn fclonefileat_inception(
    srcfd: libc::c_int,
    dst_dirfd: libc::c_int,
    dst: *const c_char,
    flags: libc::c_int,
) -> c_int {
    // Use raw syscall to avoid DYLD_FORCE_FLAT_NAMESPACE recursion
    let rc = crate::syscalls::macos_raw::raw_fclonefileat(srcfd, dst_dirfd, dst, flags as u32);
    if rc == 0 && !dst.is_null() {
        // Clone succeeded — clear uchg and set executable perms on the clone.
        // This is safe because we only change the NEW clone, not the CAS blob.
        crate::syscalls::macos_raw::raw_chflags(dst, 0);
        crate::syscalls::macos_raw::raw_chmod(dst, 0o755);
    }
    rc
}

/// BUG-016: Intercept clonefileat to fix uchg/perms on CAS-cloned build artifacts.
#[no_mangle]
#[cfg(target_os = "macos")]
pub unsafe extern "C" fn clonefileat_inception(
    src_dirfd: libc::c_int,
    src: *const c_char,
    dst_dirfd: libc::c_int,
    dst: *const c_char,
    flags: libc::c_int,
) -> c_int {
    // Use raw syscall to avoid DYLD_FORCE_FLAT_NAMESPACE recursion
    let rc =
        crate::syscalls::macos_raw::raw_clonefileat(src_dirfd, src, dst_dirfd, dst, flags as u32);
    if rc == 0 && !dst.is_null() {
        crate::syscalls::macos_raw::raw_chflags(dst, 0);
        crate::syscalls::macos_raw::raw_chmod(dst, 0o755);
    }
    rc
}

#[no_mangle]
#[cfg(target_os = "linux")]
pub unsafe extern "C" fn unlink_inception(path: *const c_char) -> c_int {
    let path_str = CStr::from_ptr(path).to_string_lossy();
    let state = match InceptionLayerState::get() {
        Some(s) => s,
        None => return crate::syscalls::linux_raw::raw_unlink(path),
    };

    let vpath = match state.resolve_path(&path_str) {
        Some(v) => v,
        None => return crate::syscalls::linux_raw::raw_unlink(path),
    };

    // Block unlink on manifest-tracked files (Tier-1 immutable)
    if vdir_lookup(state.mmap_ptr, state.mmap_size, &vpath.manifest_key).is_some() {
        crate::set_errno(libc::EPERM);
        return -1;
    }

    // Non-manifest file: passthrough to real unlink
    crate::syscalls::linux_raw::raw_unlink(path)
}

#[no_mangle]
pub unsafe extern "C" fn unlinkat_inception(
    dirfd: c_int,
    path: *const c_char,
    flags: c_int,
) -> c_int {
    let path_str = CStr::from_ptr(path).to_string_lossy();
    let state = match InceptionLayerState::get() {
        Some(s) => s,
        None => {
            #[cfg(target_os = "macos")]
            return crate::syscalls::macos_raw::raw_unlinkat(dirfd, path, flags);
            #[cfg(target_os = "linux")]
            return crate::syscalls::linux_raw::raw_unlinkat(dirfd, path, flags);
        }
    };

    let vpath = match state.resolve_path(&path_str) {
        Some(v) => v,
        None => {
            #[cfg(target_os = "macos")]
            return crate::syscalls::macos_raw::raw_unlinkat(dirfd, path, flags);
            #[cfg(target_os = "linux")]
            return crate::syscalls::linux_raw::raw_unlinkat(dirfd, path, flags);
        }
    };

    #[cfg(target_os = "macos")]
    let res = crate::syscalls::macos_raw::raw_unlinkat(dirfd, path, flags);
    #[cfg(target_os = "linux")]
    let res = crate::syscalls::linux_raw::raw_unlinkat(dirfd, path, flags);

    if res == -1
        && crate::get_errno() == libc::ENOENT
        && vdir_lookup(state.mmap_ptr, state.mmap_size, &vpath.manifest_key).is_some()
    {
        DIRTY_TRACKER.mark_dirty(&vpath.manifest_key);
        return 0;
    }

    if res == 0 {
        DIRTY_TRACKER.mark_dirty(&vpath.manifest_key);
    }

    res
}

#[no_mangle]
pub unsafe extern "C" fn mkdirat_inception(
    dirfd: c_int,
    path: *const c_char,
    mode: libc::mode_t,
) -> c_int {
    // RFC-0039: For mkdirat, we do the simple passthrough since resolving
    // dirfd-relative paths for VDir lookup is complex. The kernel handles EEXIST.
    #[cfg(target_os = "macos")]
    let result = crate::syscalls::macos_raw::raw_mkdirat(dirfd, path, mode);
    #[cfg(target_os = "linux")]
    let result = crate::syscalls::linux_raw::raw_mkdirat(dirfd, path, mode);

    // On success, update VDir with new directory entry
    if result == 0 {
        if let Some(state) = crate::state::InceptionLayerState::get() {
            // Resolve the path (handles dirfd-relative paths)
            let path_str = CStr::from_ptr(path).to_string_lossy();
            if let Some(vpath) = state.resolve_path(&path_str) {
                let _ = state.manifest_mkdir(&vpath.manifest_key, mode);
            }
        }
    }

    result
}

#[no_mangle]
pub unsafe extern "C" fn symlinkat_inception(
    p1: *const c_char,
    dirfd: c_int,
    p2: *const c_char,
) -> c_int {
    #[cfg(target_os = "macos")]
    {
        let init_state = INITIALIZING.load(Ordering::Relaxed);
        if init_state != 0
            || crate::state::INCEPTION_LAYER_STATE
                .load(Ordering::Acquire)
                .is_null()
        {
            if let Some(err) = quick_block_vfs_mutation(p1).or_else(|| quick_block_vfs_mutation(p2))
            {
                return err;
            }
            return crate::syscalls::macos_raw::raw_symlinkat(p1, dirfd, p2);
        }

        // Block mutation on existing VFS entries
        if let Some(err) = block_vfs_mutation(p1).or_else(|| block_vfs_mutation(p2)) {
            return err;
        }

        // Execute the actual symlinkat
        let result = crate::syscalls::macos_raw::raw_symlinkat(p1, dirfd, p2);

        // RFC-0039 Live Ingest: Notify daemon of successful symlink
        if result == 0 {
            if let Some(state) = crate::state::InceptionLayerState::get() {
                let target_str = CStr::from_ptr(p1).to_string_lossy();
                let link_str = CStr::from_ptr(p2).to_string_lossy();
                if let Some(vpath) = state.resolve_path(&link_str) {
                    let _ = state.manifest_symlink(&vpath.manifest_key, &target_str);
                }
            }
        }

        result
    }
    #[cfg(target_os = "linux")]
    {
        if INITIALIZING.load(Ordering::Relaxed) >= 2
            || crate::state::INCEPTION_LAYER_STATE
                .load(Ordering::Acquire)
                .is_null()
        {
            if let Some(err) = quick_block_vfs_mutation(p1).or_else(|| quick_block_vfs_mutation(p2))
            {
                return err;
            }
            return crate::syscalls::linux_raw::raw_symlinkat(p1, dirfd, p2);
        }

        // Block mutation on existing VFS entries
        if let Some(err) = block_vfs_mutation(p1).or_else(|| block_vfs_mutation(p2)) {
            return err;
        }

        // Execute the actual symlinkat
        let result = crate::syscalls::linux_raw::raw_symlinkat(p1, dirfd, p2);

        // RFC-0039 Live Ingest: Notify daemon of successful symlink
        if result == 0 {
            if let Some(state) = crate::state::InceptionLayerState::get() {
                let target_str = CStr::from_ptr(p1).to_string_lossy();
                let link_str = CStr::from_ptr(p2).to_string_lossy();
                if let Some(vpath) = state.resolve_path(&link_str) {
                    let _ = state.manifest_symlink(&vpath.manifest_key, &target_str);
                }
            }
        }

        result
    }
}

#[no_mangle]
#[cfg(target_os = "macos")]
pub unsafe extern "C" fn rmdir_inception(path: *const c_char) -> c_int {
    let path_str = CStr::from_ptr(path).to_string_lossy();
    let state = match InceptionLayerState::get() {
        Some(s) => s,
        None => return crate::syscalls::macos_raw::raw_rmdir(path),
    };

    let vpath = match state.resolve_path(&path_str) {
        Some(v) => v,
        None => return crate::syscalls::macos_raw::raw_rmdir(path),
    };

    let res = crate::syscalls::macos_raw::raw_rmdir(path);

    if res == -1
        && crate::get_errno() == libc::ENOENT
        && vdir_list_dir(state.mmap_ptr, state.mmap_size, &vpath.manifest_key)
            .map(|v| !v.is_empty())
            .unwrap_or(false)
    {
        return 0;
    }

    res
}

#[no_mangle]
#[cfg(target_os = "linux")]
pub unsafe extern "C" fn rmdir_inception(path: *const c_char) -> c_int {
    let path_str = CStr::from_ptr(path).to_string_lossy();
    let state = match InceptionLayerState::get() {
        Some(s) => s,
        None => return crate::syscalls::linux_raw::raw_rmdir(path),
    };

    let vpath = match state.resolve_path(&path_str) {
        Some(v) => v,
        None => return crate::syscalls::linux_raw::raw_rmdir(path),
    };

    let res = crate::syscalls::linux_raw::raw_rmdir(path);

    if res == -1
        && crate::get_errno() == libc::ENOENT
        && vdir_list_dir(state.mmap_ptr, state.mmap_size, &vpath.manifest_key)
            .map(|v| !v.is_empty())
            .unwrap_or(false)
    {
        return 0;
    }

    res
}

#[no_mangle]
#[cfg(target_os = "macos")]
pub unsafe extern "C" fn mkdir_inception(path: *const c_char, mode: libc::mode_t) -> c_int {
    let init_state = INITIALIZING.load(Ordering::Relaxed);
    if init_state != 0
        || crate::state::INCEPTION_LAYER_STATE
            .load(std::sync::atomic::Ordering::Acquire)
            .is_null()
    {
        // RFC-0039: During early init, allow mkdir passthrough (FS handles EEXIST)
        return crate::syscalls::macos_raw::raw_mkdir(path, mode);
    }

    // BUG-010: Always pass mkdir through to kernel.
    // The kernel correctly returns EEXIST for existing directories which
    // std::fs::create_dir_all handles gracefully. The VDir EEXIST pre-check
    // was blocking build tools (Cargo) that use create_dir_all patterns.
    let result = crate::syscalls::macos_raw::raw_mkdir(path, mode);

    // RFC-0039 Live Ingest: Update VDir with new directory entry
    if result == 0 {
        if let Some(state) = crate::state::InceptionLayerState::get() {
            let path_str = CStr::from_ptr(path).to_string_lossy();
            if let Some(vpath) = state.resolve_path(&path_str) {
                let _ = state.manifest_mkdir(&vpath.manifest_key, mode);
            }
        }
    }

    result
}

#[no_mangle]
#[cfg(target_os = "linux")]
pub unsafe extern "C" fn mkdir_inception(path: *const c_char, mode: libc::mode_t) -> c_int {
    let init_state = INITIALIZING.load(Ordering::Relaxed);
    if init_state != 0
        || crate::state::INCEPTION_LAYER_STATE
            .load(std::sync::atomic::Ordering::Acquire)
            .is_null()
    {
        // RFC-0039: During early init, allow mkdir passthrough (FS handles EEXIST)
        return crate::syscalls::linux_raw::raw_mkdir(path, mode);
    }

    // BUG-010: Always pass mkdir through to kernel (see macOS version for rationale)
    let result = crate::syscalls::linux_raw::raw_mkdir(path, mode);

    // RFC-0039 Live Ingest: Update VDir with new directory entry
    if result == 0 {
        if let Some(state) = crate::state::InceptionLayerState::get() {
            let path_str = CStr::from_ptr(path).to_string_lossy();
            if let Some(vpath) = state.resolve_path(&path_str) {
                let _ = state.manifest_mkdir(&vpath.manifest_key, mode);
            }
        }
    }

    result
}

#[no_mangle]
pub unsafe extern "C" fn utimes_inception(
    path: *const c_char,
    times: *const libc::timeval,
) -> c_int {
    let init_state = INITIALIZING.load(Ordering::Relaxed);
    if init_state != 0
        || crate::state::INCEPTION_LAYER_STATE
            .load(std::sync::atomic::Ordering::Acquire)
            .is_null()
    {
        if let Some(err) = quick_block_vfs_mutation(path) {
            return err;
        }
        #[cfg(target_os = "macos")]
        return crate::syscalls::macos_raw::raw_utimes(path, times);
        #[cfg(target_os = "linux")]
        return crate::syscalls::linux_raw::raw_utimes(path, times);
    }
    block_vfs_mutation(path).unwrap_or_else(|| {
        #[cfg(target_os = "macos")]
        return crate::syscalls::macos_raw::raw_utimes(path, times);
        #[cfg(target_os = "linux")]
        return crate::syscalls::linux_raw::raw_utimes(path, times);
    })
}

#[cfg(target_os = "linux")]
pub unsafe extern "C" fn utime_inception(path: *const c_char, times: *const libc::c_void) -> c_int {
    // utime(path, utimbuf) is legacy but used by some tools
    // utimbuf has 2 longs (actime, modtime)
    // We treat it generically and block if in VFS
    if let Some(err) = block_vfs_mutation(path) {
        return err;
    }
    // Fallback to raw utimes with NULL (current time) if times is NULL
    if times.is_null() {
        return crate::syscalls::linux_raw::raw_utimes(path, std::ptr::null());
    }
    // Otherwise use real libc utime via dlsym or just let it pass to libc
    // For simplicity, we just use the raw assembly for utimes(path, NULL)
    // if it's in VFS we already blocked it.
    libc::utime(path, times as _)
}

#[no_mangle]
#[cfg(target_os = "macos")]
pub unsafe extern "C" fn getattrlist_inception(
    path: *const c_char,
    attrlist: *mut libc::c_void,
    attrbuf: *mut libc::c_void,
    attrbufsize: libc::size_t,
    options: libc::c_ulong,
) -> c_int {
    inception_log!(
        "getattrlist_inception called for path: {:?}",
        CStr::from_ptr(path)
    );
    if let Some(res) = block_vfs_mutation(path) {
        return res;
    }
    crate::syscalls::macos_raw::raw_getattrlist(path, attrlist, attrbuf, attrbufsize, options)
}

#[no_mangle]
#[cfg(target_os = "macos")]
pub unsafe extern "C" fn setattrlist_inception(
    path: *const c_char,
    attrlist: *mut libc::c_void,
    attrbuf: *mut libc::c_void,
    attrbufsize: libc::size_t,
    options: libc::c_ulong,
) -> c_int {
    inception_log!(
        "setattrlist_inception called for path: {:?}",
        CStr::from_ptr(path)
    );
    if let Some(err) = quick_block_vfs_mutation(path) {
        return err;
    }
    let init_state = INITIALIZING.load(Ordering::Relaxed);
    if init_state == 0 {
        if let Some(res) = block_vfs_mutation(path) {
            return res;
        }
    }
    crate::syscalls::macos_raw::raw_setattrlist(path, attrlist, attrbuf, attrbufsize, options)
}

/// Helper: Check if path is in VFS and return EPERM if so
///
/// Manifest-aware VFS mutation blocking (replaces BUG-012 no-op).
///
/// Only blocks mutations on files that exist in the VDir manifest (Tier-1 CAS
/// immutable files). Local/COW build artifacts (not in manifest) pass through,
/// preserving Cargo build compatibility.
pub(crate) unsafe fn block_vfs_mutation(path: *const c_char) -> Option<c_int> {
    if path.is_null() {
        return None;
    }
    let _guard = InceptionLayerGuard::enter()?;
    let state = InceptionLayerState::get()?;
    let path_str = CStr::from_ptr(path).to_str().ok()?;
    if !state.inception_applicable(path_str) {
        return None;
    }

    // If mmap is available, use manifest-aware blocking (Tier-1 only)
    if !state.mmap_ptr.is_null() && state.mmap_size > 0 {
        let vpath = state.resolve_path(path_str)?;
        // Only block if file exists in manifest (Tier-1 immutable)
        if vdir_lookup(state.mmap_ptr, state.mmap_size, &vpath.manifest_key).is_some() {
            crate::set_errno(libc::EPERM);
            return Some(-1);
        }
        return None;
    }

    // No mmap available: fall back to VFS prefix check.
    // When the daemon hasn't populated the VDir mmap yet, we conservatively
    // block all mutations on paths under the VFS prefix. This is safe because
    // Cargo builds don't start until the daemon is fully initialized (mmap populated).
    if quick_is_in_vfs(path) {
        crate::set_errno(libc::EPERM);
        return Some(-1);
    }
    None
}

/// Helper for CREATION ops (mkdir, symlink): Only block if path EXISTS in manifest
pub(crate) unsafe fn block_existing_vfs_entry_at(
    _dirfd: c_int,
    path: *const c_char,
) -> Option<c_int> {
    block_vfs_mutation(path)
}

pub(crate) unsafe fn block_existing_vfs_entry(path: *const c_char) -> Option<c_int> {
    block_existing_vfs_entry_at(libc::AT_FDCWD, path)
}

#[inline]
pub(crate) unsafe fn quick_is_in_vfs(path: *const c_char) -> bool {
    if path.is_null() {
        return false;
    }
    let path_str = if let Ok(s) = CStr::from_ptr(path).to_str() {
        s
    } else {
        return false;
    };
    let env_name = b"VRIFT_VFS_PREFIX\0";
    let vfs_prefix_ptr = libc::getenv(env_name.as_ptr() as *const c_char);
    if !vfs_prefix_ptr.is_null() {
        if let Ok(vfs_prefix) = CStr::from_ptr(vfs_prefix_ptr).to_str() {
            if path_str.starts_with(vfs_prefix) {
                return true;
            }
            // macOS: F_GETPATH returns resolved paths (e.g. /private/tmp/...)
            // but VRIFT_VFS_PREFIX may use /tmp/... (symlink)
            #[cfg(target_os = "macos")]
            {
                if let Some(suffix) = path_str.strip_prefix("/private") {
                    if suffix.starts_with(vfs_prefix) {
                        return true;
                    }
                }
                // Also handle reverse: prefix is /private/tmp but path is /tmp
                if let Some(suffix) = vfs_prefix.strip_prefix("/private") {
                    if path_str.starts_with(suffix) {
                        return true;
                    }
                }
            }
        }
    }
    false
}

/// Lightweight VFS prefix check for raw syscall path during early init.
/// Uses VRIFT_VFS_PREFIX env var only (no InceptionLayerState needed).
#[inline]
pub(crate) unsafe fn quick_block_vfs_mutation(path: *const c_char) -> Option<c_int> {
    if path.is_null() {
        return None;
    }
    if quick_is_in_vfs(path) {
        crate::set_errno(libc::EPERM);
        return Some(-1);
    }
    None
}

// --- chmod/fchmod ---

#[no_mangle]
pub unsafe extern "C" fn chmod_inception(path: *const c_char, mode: libc::mode_t) -> c_int {
    // BUG-007: Use raw syscall during early init OR when inception layer not fully ready
    // to avoid dlsym recursion and TLS pthread deadlock
    let init_state = crate::state::INITIALIZING.load(std::sync::atomic::Ordering::Relaxed);
    if init_state != 0
        || crate::state::INCEPTION_LAYER_STATE
            .load(std::sync::atomic::Ordering::Acquire)
            .is_null()
    {
        // Still check VFS prefix even in raw syscall path
        if let Some(err) = quick_block_vfs_mutation(path) {
            return err;
        }
        #[cfg(target_os = "macos")]
        return crate::syscalls::macos_raw::raw_chmod(path, mode);
        #[cfg(target_os = "linux")]
        return crate::syscalls::linux_raw::raw_chmod(path, mode);
    }

    // Pattern 2930: Use raw syscall to avoid post-init dlsym hazard
    block_vfs_mutation(path).unwrap_or_else(|| {
        #[cfg(target_os = "macos")]
        return crate::syscalls::macos_raw::raw_chmod(path, mode);
        #[cfg(target_os = "linux")]
        return crate::syscalls::linux_raw::raw_chmod(path, mode);
    })
}

#[no_mangle]
pub unsafe extern "C" fn fchmodat_inception(
    dirfd: c_int,
    path: *const c_char,
    mode: libc::mode_t,
    flags: c_int,
) -> c_int {
    let init_state = INITIALIZING.load(Ordering::Relaxed);
    if init_state != 0
        || crate::state::INCEPTION_LAYER_STATE
            .load(std::sync::atomic::Ordering::Acquire)
            .is_null()
    {
        if let Some(err) = quick_block_vfs_mutation(path) {
            return err;
        }
        #[cfg(target_os = "macos")]
        return crate::syscalls::macos_raw::raw_fchmodat(dirfd, path, mode, flags);
        #[cfg(target_os = "linux")]
        return crate::syscalls::linux_raw::raw_fchmodat(dirfd, path, mode, flags);
    }
    // Pattern 2930: Use raw syscall to avoid post-init dlsym hazard
    block_vfs_mutation(path).unwrap_or_else(|| {
        #[cfg(target_os = "macos")]
        return crate::syscalls::macos_raw::raw_fchmodat(dirfd, path, mode, flags);
        #[cfg(target_os = "linux")]
        return crate::syscalls::linux_raw::raw_fchmodat(dirfd, path, mode, flags);
    })
}

#[no_mangle]
pub unsafe extern "C" fn fchmod_inception(fd: c_int, mode: libc::mode_t) -> c_int {
    #[cfg(target_os = "macos")]
    {
        let init_state = INITIALIZING.load(Ordering::Relaxed);
        if init_state != 0
            || crate::state::INCEPTION_LAYER_STATE
                .load(Ordering::Acquire)
                .is_null()
        {
            return crate::syscalls::macos_raw::raw_fchmod(fd, mode);
        }

        // RFC-OPT-001: Recursion protection
        let _guard = match InceptionLayerGuard::enter() {
            Some(g) => g,
            None => return crate::syscalls::macos_raw::raw_fchmod(fd, mode),
        };

        // VFS logic: if FD points to a VFS file, block mutation
        // Strategy: Try to get path from FD (robust)
        let mut path_buf = [0; 1024];
        if unsafe { libc::fcntl(fd, libc::F_GETPATH, path_buf.as_mut_ptr()) } == 0 {
            let path_cstr = unsafe { CStr::from_ptr(path_buf.as_ptr()) };
            if let Ok(path_str) = path_cstr.to_str() {
                if let Some(state) = InceptionLayerState::get() {
                    if state.inception_applicable(path_str) {
                        crate::set_errno(libc::EPERM);
                        return -1;
                    }
                }
            }
        }

        // Fallback to FD table if F_GETPATH failed or for extra safety
        use crate::syscalls::io::get_fd_entry;
        if let Some(entry) = get_fd_entry(fd) {
            if entry.is_vfs {
                crate::set_errno(libc::EPERM);
                return -1;
            }
        }

        crate::syscalls::macos_raw::raw_fchmod(fd, mode)
    }
    #[cfg(target_os = "linux")]
    {
        if INITIALIZING.load(Ordering::Relaxed) >= 2
            || crate::state::INCEPTION_LAYER_STATE
                .load(Ordering::Acquire)
                .is_null()
        {
            return crate::syscalls::linux_raw::raw_fchmod(fd, mode);
        }

        let _guard = match InceptionLayerGuard::enter() {
            Some(g) => g,
            None => return crate::syscalls::linux_raw::raw_fchmod(fd, mode),
        };

        // Strategy: Use /proc/self/fd/N to get path
        let fd_path = format!("/proc/self/fd/{}\0", fd);
        let mut path_buf = [0u8; 1024];
        let n = unsafe {
            libc::readlink(
                fd_path.as_ptr() as *const c_char,
                path_buf.as_mut_ptr() as *mut c_char,
                path_buf.len(),
            )
        };
        if n > 0 && (n as usize) < path_buf.len() {
            if let Ok(path_str) = std::str::from_utf8(&path_buf[..n as usize]) {
                if let Some(state) = InceptionLayerState::get() {
                    if state.inception_applicable(path_str) {
                        crate::set_errno(libc::EPERM);
                        return -1;
                    }
                }
            }
        }

        use crate::syscalls::io::get_fd_entry;
        if let Some(entry) = get_fd_entry(fd) {
            if entry.is_vfs {
                crate::set_errno(libc::EPERM);
                return -1;
            }
        }

        crate::syscalls::linux_raw::raw_fchmod(fd, mode)
    }
}

// P0-P1 Gap Fix: fchown/fchownat - Block ownership changes on VFS files via FD
// Pattern: Same as fchmod_inception - resolve FD to path, check VFS

/// fchown_inception: Block ownership changes on VFS files via FD
/// Uses F_GETPATH (macOS) or /proc/self/fd (Linux) to resolve FD to path
#[no_mangle]
pub unsafe extern "C" fn fchown_inception(
    fd: c_int,
    owner: libc::uid_t,
    group: libc::gid_t,
) -> c_int {
    #[cfg(target_os = "macos")]
    {
        // VFS check via fd path resolution — must use raw_fcntl to avoid
        // triggering fcntl_inception (which causes recursive syscall chain crash)
        let mut path_buf = [0i8; 1024];
        if crate::syscalls::macos_raw::raw_fcntl(fd, libc::F_GETPATH, path_buf.as_mut_ptr() as i64)
            == 0
            && quick_is_in_vfs(path_buf.as_ptr())
        {
            crate::set_errno(libc::EPERM);
            return -1;
        }

        let init_state = INITIALIZING.load(Ordering::Relaxed);
        if init_state != 0
            || crate::state::INCEPTION_LAYER_STATE
                .load(Ordering::Acquire)
                .is_null()
        {
            return crate::syscalls::macos_raw::raw_fchown(fd, owner, group);
        }

        // RFC-OPT-001: Recursion protection
        let _guard = match InceptionLayerGuard::enter() {
            Some(g) => g,
            None => return crate::syscalls::macos_raw::raw_fchown(fd, owner, group),
        };

        // Fallback to FD table if F_GETPATH failed
        use crate::syscalls::io::get_fd_entry;
        if let Some(entry) = get_fd_entry(fd) {
            if entry.is_vfs {
                crate::set_errno(libc::EPERM);
                return -1;
            }
        }

        crate::syscalls::macos_raw::raw_fchown(fd, owner, group)
    }
    #[cfg(target_os = "linux")]
    {
        if INITIALIZING.load(Ordering::Relaxed) >= 2
            || crate::state::INCEPTION_LAYER_STATE
                .load(Ordering::Acquire)
                .is_null()
        {
            return crate::syscalls::linux_raw::raw_fchown(fd, owner, group);
        }

        let _guard = match InceptionLayerGuard::enter() {
            Some(g) => g,
            None => return crate::syscalls::linux_raw::raw_fchown(fd, owner, group),
        };

        // Strategy: Use /proc/self/fd/N to get path
        let fd_path = format!("/proc/self/fd/{}\0", fd);
        let mut path_buf = [0u8; 1024];
        let n = libc::readlink(
            fd_path.as_ptr() as *const c_char,
            path_buf.as_mut_ptr() as *mut c_char,
            path_buf.len(),
        );
        if n > 0 && (n as usize) < path_buf.len() {
            if let Ok(path_str) = std::str::from_utf8(&path_buf[..n as usize]) {
                if let Some(state) = InceptionLayerState::get() {
                    if state.inception_applicable(path_str) {
                        crate::set_errno(libc::EPERM);
                        return -1;
                    }
                }
            }
        }

        use crate::syscalls::io::get_fd_entry;
        if let Some(entry) = get_fd_entry(fd) {
            if entry.is_vfs {
                crate::set_errno(libc::EPERM);
                return -1;
            }
        }

        crate::syscalls::linux_raw::raw_fchown(fd, owner, group)
    }
}

/// fchownat_inception: Block ownership changes on VFS files via dirfd + path
#[no_mangle]
pub unsafe extern "C" fn fchownat_inception(
    dirfd: c_int,
    path: *const c_char,
    owner: libc::uid_t,
    group: libc::gid_t,
    flags: c_int,
) -> c_int {
    #[cfg(target_os = "macos")]
    {
        let init_state = INITIALIZING.load(Ordering::Relaxed);
        if init_state != 0
            || crate::state::INCEPTION_LAYER_STATE
                .load(Ordering::Acquire)
                .is_null()
        {
            if let Some(err) = quick_block_vfs_mutation(path) {
                return err;
            }
            return crate::syscalls::macos_raw::raw_fchownat(dirfd, path, owner, group, flags);
        }
        // Pattern 2930: Use raw syscall to avoid post-init dlsym hazard
        block_vfs_mutation(path).unwrap_or_else(|| {
            crate::syscalls::macos_raw::raw_fchownat(dirfd, path, owner, group, flags)
        })
    }
    #[cfg(target_os = "linux")]
    {
        if INITIALIZING.load(Ordering::Relaxed) >= 2
            || crate::state::INCEPTION_LAYER_STATE
                .load(Ordering::Acquire)
                .is_null()
        {
            if let Some(err) = quick_block_vfs_mutation(path) {
                return err;
            }
            return crate::syscalls::linux_raw::raw_fchownat(dirfd, path, owner, group, flags);
        }
        block_vfs_mutation(path).unwrap_or_else(|| {
            crate::syscalls::linux_raw::raw_fchownat(dirfd, path, owner, group, flags)
        })
    }
}

// P0-P1 Gap Fix: exchangedata - Block atomic file swaps involving VFS (macOS only)

// Gap Fix: chown/lchown path-based ownership interposition

/// chown_inception: Block ownership changes on VFS files via path
#[no_mangle]
pub unsafe extern "C" fn chown_inception(
    path: *const c_char,
    owner: libc::uid_t,
    group: libc::gid_t,
) -> c_int {
    let init_state = INITIALIZING.load(Ordering::Relaxed);
    if init_state != 0
        || crate::state::INCEPTION_LAYER_STATE
            .load(Ordering::Acquire)
            .is_null()
    {
        if let Some(err) = quick_block_vfs_mutation(path) {
            return err;
        }
        #[cfg(target_os = "macos")]
        return crate::syscalls::macos_raw::raw_chown(path, owner, group);
        #[cfg(target_os = "linux")]
        return crate::syscalls::linux_raw::raw_chown(path, owner, group);
    }
    block_vfs_mutation(path).unwrap_or_else(|| {
        #[cfg(target_os = "macos")]
        return crate::syscalls::macos_raw::raw_chown(path, owner, group);
        #[cfg(target_os = "linux")]
        return crate::syscalls::linux_raw::raw_chown(path, owner, group);
    })
}

/// lchown_inception: Block ownership changes on VFS symlinks via path (no-follow)
#[no_mangle]
pub unsafe extern "C" fn lchown_inception(
    path: *const c_char,
    owner: libc::uid_t,
    group: libc::gid_t,
) -> c_int {
    let init_state = INITIALIZING.load(Ordering::Relaxed);
    if init_state != 0
        || crate::state::INCEPTION_LAYER_STATE
            .load(Ordering::Acquire)
            .is_null()
    {
        if let Some(err) = quick_block_vfs_mutation(path) {
            return err;
        }
        #[cfg(target_os = "macos")]
        return crate::syscalls::macos_raw::raw_lchown(path, owner, group);
        #[cfg(target_os = "linux")]
        return crate::syscalls::linux_raw::raw_lchown(path, owner, group);
    }
    block_vfs_mutation(path).unwrap_or_else(|| {
        #[cfg(target_os = "macos")]
        return crate::syscalls::macos_raw::raw_lchown(path, owner, group);
        #[cfg(target_os = "linux")]
        return crate::syscalls::linux_raw::raw_lchown(path, owner, group);
    })
}

// Gap Fix: readlinkat interposition

/// readlinkat_inception: Interpose readlinkat to handle VFS symlinks
/// For absolute paths or AT_FDCWD, delegates to readlink VFS logic.
/// For dirfd-relative paths, passes through to real readlinkat.
#[no_mangle]
pub unsafe extern "C" fn readlinkat_inception(
    dirfd: c_int,
    path: *const c_char,
    buf: *mut c_char,
    bufsiz: libc::size_t,
) -> libc::ssize_t {
    // Fast path: during early init, just passthrough
    let init_state = INITIALIZING.load(Ordering::Relaxed);
    if init_state != 0
        || crate::state::INCEPTION_LAYER_STATE
            .load(Ordering::Acquire)
            .is_null()
    {
        #[cfg(target_os = "macos")]
        return crate::syscalls::macos_raw::raw_readlinkat(dirfd, path, buf, bufsiz);
        #[cfg(target_os = "linux")]
        return crate::syscalls::linux_raw::raw_readlinkat(dirfd, path, buf, bufsiz);
    }

    // For AT_FDCWD with absolute paths, use VFS-resolved readlink path
    if !path.is_null() && (dirfd == libc::AT_FDCWD || *path == b'/' as c_char) {
        // Delegate to readlink inception logic which handles VFS paths
        if let Some(_guard) = InceptionLayerGuard::enter() {
            if let Some(state) = InceptionLayerState::get() {
                let path_cstr = CStr::from_ptr(path);
                if let Ok(path_str) = path_cstr.to_str() {
                    if let Some(vpath) = state.resolve_path(path_str) {
                        // VFS path: use raw readlinkat to read the underlying symlink
                        // (the real symlink target is in the materialized workspace)
                        inception_log!("readlinkat on VFS path: '{}'", vpath.absolute);
                    }
                }
            }
        }
    }

    // Passthrough to real readlinkat
    #[cfg(target_os = "macos")]
    return crate::syscalls::macos_raw::raw_readlinkat(dirfd, path, buf, bufsiz);
    #[cfg(target_os = "linux")]
    return crate::syscalls::linux_raw::raw_readlinkat(dirfd, path, buf as *mut _, bufsiz);
}

/// exchangedata_inception: Block atomic file swaps involving VFS (macOS only)
/// Returns EXDEV if any path is in VFS (cross-device semantics)
#[no_mangle]
#[cfg(target_os = "macos")]
pub unsafe extern "C" fn exchangedata_inception(
    path1: *const c_char,
    path2: *const c_char,
    options: libc::c_uint,
) -> c_int {
    // Quick VFS check - always applies, even during init
    let path1_in_vfs = quick_is_in_vfs(path1);
    let path2_in_vfs = quick_is_in_vfs(path2);
    if path1_in_vfs || path2_in_vfs {
        // Either path in VFS: block the swap
        crate::set_errno(libc::EXDEV);
        return -1;
    }

    let init_state = INITIALIZING.load(Ordering::Relaxed);
    if init_state != 0
        || crate::state::INCEPTION_LAYER_STATE
            .load(Ordering::Acquire)
            .is_null()
    {
        return crate::syscalls::macos_raw::raw_exchangedata(path1, path2, options);
    }

    let _guard = match InceptionLayerGuard::enter() {
        Some(g) => g,
        None => return crate::syscalls::macos_raw::raw_exchangedata(path1, path2, options),
    };

    // Full VFS check with state
    if let Some(state) = InceptionLayerState::get() {
        let p1_str = CStr::from_ptr(path1).to_str().ok();
        let p2_str = CStr::from_ptr(path2).to_str().ok();
        if let (Some(p1), Some(p2)) = (p1_str, p2_str) {
            if state.inception_applicable(p1) || state.inception_applicable(p2) {
                crate::set_errno(libc::EXDEV);
                return -1;
            }
        }
    }

    crate::syscalls::macos_raw::raw_exchangedata(path1, path2, options)
}

// --- truncate ---

#[no_mangle]
pub unsafe extern "C" fn truncate_inception(path: *const c_char, length: libc::off_t) -> c_int {
    let init_state = INITIALIZING.load(Ordering::Relaxed);
    if init_state != 0
        || crate::state::INCEPTION_LAYER_STATE
            .load(std::sync::atomic::Ordering::Acquire)
            .is_null()
    {
        if let Some(err) = quick_block_vfs_mutation(path) {
            return err;
        }
        #[cfg(target_os = "macos")]
        return crate::syscalls::macos_raw::raw_truncate(path, length);
        #[cfg(target_os = "linux")]
        return crate::syscalls::linux_raw::raw_truncate(path, length);
    }
    // Pattern 2930: Use raw syscall to avoid post-init dlsym hazard
    #[cfg(target_os = "macos")]
    return block_vfs_mutation(path)
        .unwrap_or_else(|| crate::syscalls::macos_raw::raw_truncate(path, length));
    #[cfg(target_os = "linux")]
    return block_vfs_mutation(path)
        .unwrap_or_else(|| crate::syscalls::linux_raw::raw_truncate(path, length));
}

#[no_mangle]
#[cfg(target_os = "macos")]
pub unsafe extern "C" fn chflags_inception(path: *const c_char, flags: libc::c_uint) -> c_int {
    let init_state = INITIALIZING.load(Ordering::Relaxed);
    if init_state != 0
        || crate::state::INCEPTION_LAYER_STATE
            .load(std::sync::atomic::Ordering::Acquire)
            .is_null()
    {
        if let Some(err) = quick_block_vfs_mutation(path) {
            return err;
        }
        return crate::syscalls::macos_raw::raw_chflags(path, flags);
    }
    // Pattern 2930: Use raw syscall to avoid post-init dlsym hazard
    block_vfs_mutation(path).unwrap_or_else(|| crate::syscalls::macos_raw::raw_chflags(path, flags))
}

// --- xattr ---

#[no_mangle]
#[cfg(target_os = "macos")]
pub unsafe extern "C" fn setxattr_inception(
    path: *const c_char,
    name: *const c_char,
    value: *const c_void,
    size: libc::size_t,
    position: u32,
    options: c_int,
) -> c_int {
    let init_state = INITIALIZING.load(Ordering::Relaxed);
    if init_state != 0
        || crate::state::INCEPTION_LAYER_STATE
            .load(std::sync::atomic::Ordering::Acquire)
            .is_null()
    {
        if let Some(err) = quick_block_vfs_mutation(path) {
            return err;
        }
        return crate::syscalls::macos_raw::raw_setxattr(
            path, name, value, size, position, options,
        );
    }
    // Pattern 2930: Use raw syscall to avoid post-init dlsym hazard
    block_vfs_mutation(path).unwrap_or_else(|| {
        crate::syscalls::macos_raw::raw_setxattr(path, name, value, size, position, options)
    })
}

#[no_mangle]
#[cfg(target_os = "macos")]
pub unsafe extern "C" fn removexattr_inception(
    path: *const c_char,
    name: *const c_char,
    options: c_int,
) -> c_int {
    let init_state = INITIALIZING.load(Ordering::Relaxed);
    if init_state != 0
        || crate::state::INCEPTION_LAYER_STATE
            .load(std::sync::atomic::Ordering::Acquire)
            .is_null()
    {
        if let Some(err) = quick_block_vfs_mutation(path) {
            return err;
        }
        return crate::syscalls::macos_raw::raw_removexattr(path, name, options);
    }
    // Pattern 2930: Use raw syscall to avoid post-init dlsym hazard
    block_vfs_mutation(path)
        .unwrap_or_else(|| crate::syscalls::macos_raw::raw_removexattr(path, name, options))
}

// RFC-0047: Timestamp Modification Protection

/// utimensat_inception: Block timestamp modifications on VFS files (at variant)
/// Note: macOS doesn't have a direct utimensat syscall - it uses getattrlist/setattrlist
#[no_mangle]
pub unsafe extern "C" fn setrlimit_inception(resource: c_int, rlp: *const libc::rlimit) -> c_int {
    #[cfg(target_os = "macos")]
    let ret = crate::syscalls::macos_raw::raw_setrlimit(resource, rlp);
    #[cfg(target_os = "linux")]
    let ret = libc::setrlimit(resource as u32, rlp);

    // Linux uses u32 for RLIMIT_NOFILE constant
    #[cfg(target_os = "linux")]
    let is_nofile = resource as u32 == libc::RLIMIT_NOFILE;
    #[cfg(target_os = "macos")]
    let is_nofile = resource == libc::RLIMIT_NOFILE;

    if ret == 0 && is_nofile && !rlp.is_null() {
        if let Some(state) = InceptionLayerState::get() {
            let new_soft = (*rlp).rlim_cur as usize;
            state.cached_soft_limit.store(new_soft, Ordering::Release);
        }
    }

    ret
}

// Passthrough inception layers for interpose table compatibility

#[no_mangle]
#[cfg(target_os = "macos")]
pub unsafe extern "C" fn flock_inception(fd: c_int, op: c_int) -> c_int {
    let _guard = match InceptionLayerGuard::enter() {
        Some(g) => g,
        None => {
            #[cfg(target_os = "macos")]
            return crate::syscalls::macos_raw::raw_flock(fd, op);
            #[cfg(target_os = "linux")]
            return crate::syscalls::linux_raw::raw_flock(fd, op);
        }
    };

    let state = match InceptionLayerState::get() {
        Some(s) => s,
        None => {
            #[cfg(target_os = "macos")]
            return crate::syscalls::macos_raw::raw_flock(fd, op);
            #[cfg(target_os = "linux")]
            return crate::syscalls::linux_raw::raw_flock(fd, op);
        }
    };

    let entry_ptr = state.open_fds.get(fd as u32);
    if entry_ptr.is_null() {
        #[cfg(target_os = "macos")]
        return crate::syscalls::macos_raw::raw_flock(fd, op);
        #[cfg(target_os = "linux")]
        return crate::syscalls::linux_raw::raw_flock(fd, op);
    }

    let entry = &mut *entry_ptr;
    if !entry.is_vfs || entry.manifest_key.is_empty() {
        #[cfg(target_os = "macos")]
        return crate::syscalls::macos_raw::raw_flock(fd, op);
        #[cfg(target_os = "linux")]
        return crate::syscalls::linux_raw::raw_flock(fd, op);
    }

    // RFC-0049: Logical flock implementation
    // Use a shared lockfile based on the manifest key hash
    let mut lock_path_buf = [0u8; 1024];
    let mut writer = crate::macros::StackWriter::new(&mut lock_path_buf);
    use std::fmt::Write;

    let key_hash = vrift_ipc::fnv1a_hash(entry.manifest_key.as_str());
    let _ = write!(
        writer,
        "{}/.vrift/locks/{:016x}.lock",
        state.project_root.as_str(),
        key_hash
    );
    let lock_path = writer.as_str();

    // Open lock FD if not already held for this FD
    if entry.lock_fd < 0 {
        let lock_path_cstr = std::ffi::CString::new(lock_path).unwrap_or_default();
        #[cfg(target_os = "macos")]
        let lfd = crate::syscalls::macos_raw::raw_openat(
            libc::AT_FDCWD,
            lock_path_cstr.as_ptr(),
            libc::O_RDWR | libc::O_CREAT | libc::O_CLOEXEC,
            0o666,
        );
        #[cfg(target_os = "linux")]
        let lfd = crate::syscalls::linux_raw::raw_open(
            lock_path_cstr.as_ptr(),
            libc::O_RDWR | libc::O_CREAT | libc::O_CLOEXEC,
            0o666,
        );

        if lfd < 0 {
            // Fallback to original FD if we can't open lockfile
            #[cfg(target_os = "macos")]
            return crate::syscalls::macos_raw::raw_flock(fd, op);
            #[cfg(target_os = "linux")]
            return crate::syscalls::linux_raw::raw_flock(fd, op);
        }
        entry.lock_fd = lfd;
    }

    // Call flock on the logical lockfile
    #[cfg(target_os = "macos")]
    return crate::syscalls::macos_raw::raw_flock(entry.lock_fd, op);
    #[cfg(target_os = "linux")]
    return crate::syscalls::linux_raw::raw_flock(entry.lock_fd, op);
}

#[no_mangle]
pub unsafe extern "C" fn symlink_inception(p1: *const c_char, p2: *const c_char) -> c_int {
    // Note: IT_SYMLINK is now in __DATA,__interpose, so libc::symlink would
    // recurse back into this function. Must use raw syscall.
    let init_state = INITIALIZING.load(Ordering::Relaxed);
    if init_state != 0
        || crate::state::INCEPTION_LAYER_STATE
            .load(Ordering::Acquire)
            .is_null()
    {
        // During early init: use raw syscall via symlinkat(target, AT_FDCWD, linkpath)
        #[cfg(target_os = "macos")]
        return crate::syscalls::macos_raw::raw_symlinkat(p1, libc::AT_FDCWD, p2);
        #[cfg(target_os = "linux")]
        return crate::syscalls::linux_raw::raw_symlink(p1, p2);
    }
    // RFC-0039: Only block if path EXISTS in manifest, allow new symlink creation
    if let Some(err) = block_existing_vfs_entry(p2) {
        return err;
    }
    #[cfg(target_os = "macos")]
    return crate::syscalls::macos_raw::raw_symlinkat(p1, libc::AT_FDCWD, p2);
    #[cfg(target_os = "linux")]
    return crate::syscalls::linux_raw::raw_symlink(p1, p2);
}

#[no_mangle]
#[cfg(target_os = "macos")]
pub unsafe extern "C" fn execve_inception(
    path: *const c_char,
    argv: *const *const c_char,
    envp: *const *const c_char,
) -> c_int {
    libc::execve(path, argv, envp)
}

#[no_mangle]
#[cfg(target_os = "macos")]
pub unsafe extern "C" fn posix_spawn_inception(
    pid: *mut libc::pid_t,
    path: *const c_char,
    fa: *const c_void,
    attr: *const c_void,
    argv: *const *const c_char,
    envp: *const *const c_char,
) -> c_int {
    // BUG-016: CAS-cloned build scripts may have uchg flag and 0o444 mode
    // inherited from the CAS blob. Clear flags and ensure executable perms
    // right before spawning — this is the last line of defense.
    if !path.is_null() {
        let path_bytes = std::ffi::CStr::from_ptr(path).to_bytes();
        // Only fix VFS paths (avoid touching system binaries)
        if let Some(state) = crate::state::InceptionLayerState::get() {
            let path_s = std::str::from_utf8_unchecked(path_bytes);
            if state.inception_applicable(path_s) {
                let _cf_rc = crate::syscalls::macos_raw::raw_chflags(path, 0);
                let _cm_rc = crate::syscalls::macos_raw::raw_chmod(path, 0o755);
            }
        }
    }
    libc::posix_spawn(
        pid,
        path,
        fa as *const libc::posix_spawn_file_actions_t,
        attr as *const libc::posix_spawnattr_t,
        argv as *const *mut c_char,
        envp as *const *mut c_char,
    )
}

#[no_mangle]
#[cfg(target_os = "macos")]
pub unsafe extern "C" fn posix_spawnp_inception(
    pid: *mut libc::pid_t,
    file: *const c_char,
    fa: *const c_void,
    attr: *const c_void,
    argv: *const *const c_char,
    envp: *const *const c_char,
) -> c_int {
    // BUG-016: Same as posix_spawn_inception — ensure executable perms
    if !file.is_null() {
        let path_bytes = std::ffi::CStr::from_ptr(file).to_bytes();
        if let Some(state) = crate::state::InceptionLayerState::get() {
            let path_s = std::str::from_utf8_unchecked(path_bytes);
            if state.inception_applicable(path_s) {
                crate::syscalls::macos_raw::raw_chflags(file, 0);
                crate::syscalls::macos_raw::raw_chmod(file, 0o755);
            }
        }
    }
    libc::posix_spawnp(
        pid,
        file,
        fa as *const libc::posix_spawn_file_actions_t,
        attr as *const libc::posix_spawnattr_t,
        argv as *const *mut c_char,
        envp as *const *mut c_char,
    )
}

#[no_mangle]
#[cfg(target_os = "macos")]
pub unsafe extern "C" fn faccessat_inception(
    dirfd: c_int,
    path: *const c_char,
    mode: c_int,
    flags: c_int,
) -> c_int {
    libc::faccessat(dirfd, path, mode, flags)
}

/// fcntl implementation called from C bridge (variadic_inception.c)
#[no_mangle]
#[cfg(target_os = "macos")]
pub unsafe extern "C" fn velo_fcntl_impl(fd: c_int, cmd: c_int, arg: libc::c_long) -> c_int {
    // Simple passthrough for now - fcntl doesn't need VFS virtualization
    libc::fcntl(fd, cmd, arg)
}
