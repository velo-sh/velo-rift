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

    let _guard = ShimGuard::enter()?;
    let state = ShimState::get()?;

    let old_str = CStr::from_ptr(old).to_str().ok()?;
    let new_str = CStr::from_ptr(new).to_str().ok()?;

    let old_in_vfs = state.psfs_applicable(old_str);
    let new_in_vfs = state.psfs_applicable(new_str);

    // RFC-0047: Cross-boundary rename is forbidden
    if old_in_vfs != new_in_vfs {
        crate::set_errno(libc::EXDEV);
        return Some(-1);
    }

    None // Let real syscall handle non-VFS renames
}

#[no_mangle]
#[cfg(target_os = "macos")]
pub unsafe extern "C" fn rename_shim(old: *const c_char, new: *const c_char) -> c_int {
    let real = std::mem::transmute::<
        *mut libc::c_void,
        unsafe extern "C" fn(*const c_char, *const c_char) -> c_int,
    >(crate::reals::REAL_RENAME.get());
    passthrough_if_init!(real, old, new);
    rename_impl(old, new).unwrap_or_else(|| real(old, new))
}

/// Linux-specific rename shim. Returns -2 if passthrough to real syscall is needed.
#[cfg(target_os = "linux")]
#[no_mangle]
pub unsafe extern "C" fn rename_shim_linux(old: *const c_char, new: *const c_char) -> c_int {
    rename_impl(old, new).unwrap_or(-2) // -2 is a magic value to signal passthrough
}

#[no_mangle]
#[cfg(target_os = "macos")]
pub unsafe extern "C" fn renameat_shim(
    oldfd: c_int,
    old: *const c_char,
    newfd: c_int,
    new: *const c_char,
) -> c_int {
    #[cfg(target_os = "macos")]
    {
        let real = std::mem::transmute::<
            *mut libc::c_void,
            unsafe extern "C" fn(c_int, *const c_char, c_int, *const c_char) -> c_int,
        >(crate::reals::REAL_RENAMEAT.get());
        passthrough_if_init!(real, oldfd, old, newfd, new);

        // Resolve relative paths using getcwd for AT_FDCWD case
        if oldfd == libc::AT_FDCWD && newfd == libc::AT_FDCWD {
            if let Some(result) = renameat_impl(old, new) {
                return result;
            }
        }
        real(oldfd, old, newfd, new)
    }
    #[cfg(target_os = "linux")]
    {
        // For Linux, we don't have renameat shim yet, return passthrough
        // TODO: Implement renameat for Linux
        -1
    }
}

/// renameat path resolution helper - resolves relative paths to absolute
unsafe fn renameat_impl(old: *const c_char, new: *const c_char) -> Option<c_int> {
    if old.is_null() || new.is_null() {
        return None;
    }

    let _guard = ShimGuard::enter()?;
    let state = ShimState::get()?;

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

    let old_in_vfs = state.psfs_applicable(&old_abs);
    let new_in_vfs = state.psfs_applicable(&new_abs);

    // RFC-0047: Cross-boundary rename is forbidden
    if old_in_vfs != new_in_vfs {
        crate::set_errno(libc::EXDEV);
        return Some(-1);
    }

    None // Let real syscall handle
}

/// RFC-0047: Link (hardlink) implementation with VFS boundary enforcement
/// Hardlinks crossing VFS boundary or into CAS are forbidden (returns EXDEV)
unsafe fn link_impl(old: *const c_char, new: *const c_char) -> Option<c_int> {
    if old.is_null() || new.is_null() {
        return None;
    }

    let _guard = ShimGuard::enter()?;
    let state = ShimState::get()?;

    let old_str = CStr::from_ptr(old).to_str().ok()?;
    let new_str = CStr::from_ptr(new).to_str().ok()?;

    let old_in_vfs = state.psfs_applicable(old_str);
    let new_in_vfs = state.psfs_applicable(new_str);

    // RFC-0047: Cross-boundary hardlink is forbidden
    // Also block if source is in VFS (protects CAS blobs)
    if old_in_vfs != new_in_vfs || old_in_vfs {
        crate::set_errno(libc::EXDEV);
        return Some(-1);
    }

    None // Let real syscall handle non-VFS links
}

#[no_mangle]
pub unsafe extern "C" fn link_shim(old: *const c_char, new: *const c_char) -> c_int {
    #[cfg(target_os = "macos")]
    {
        extern "C" {
            fn link(old: *const c_char, new: *const c_char) -> c_int;
        }
        if INITIALIZING.load(Ordering::Relaxed) >= 2 {
            return link(old, new);
        }
        link_impl(old, new).unwrap_or_else(|| link(old, new))
    }
    #[cfg(target_os = "linux")]
    {
        if INITIALIZING.load(Ordering::Relaxed) >= 2 {
            return crate::syscalls::linux_raw::raw_link(old, new);
        }
        link_impl(old, new).unwrap_or_else(|| crate::syscalls::linux_raw::raw_link(old, new))
    }
}

#[no_mangle]
#[cfg(target_os = "macos")]
pub unsafe extern "C" fn linkat_shim(
    oldfd: c_int,
    old: *const c_char,
    newfd: c_int,
    new: *const c_char,
    flags: c_int,
) -> c_int {
    #[cfg(target_os = "macos")]
    {
        let real = std::mem::transmute::<
            *mut libc::c_void,
            unsafe extern "C" fn(c_int, *const c_char, c_int, *const c_char, c_int) -> c_int,
        >(crate::reals::REAL_LINKAT.get());
        passthrough_if_init!(real, oldfd, old, newfd, new, flags);
        block_vfs_mutation(old)
            .or_else(|| block_vfs_mutation(new))
            .unwrap_or_else(|| real(oldfd, old, newfd, new, flags))
    }
    #[cfg(target_os = "linux")]
    {
        // For Linux, we don't have linkat shim yet, return passthrough
        -1
    }
}

// ============================================================================
// RFC-0047: Mutation Perimeter - Block modifications to VFS-managed files
// ============================================================================

#[no_mangle]
pub unsafe extern "C" fn unlink_shim(path: *const c_char) -> c_int {
    #[cfg(target_os = "macos")]
    {
        // BUG-007: Use raw syscall when shim not fully ready to avoid TLS deadlock
        let init_state = INITIALIZING.load(Ordering::Relaxed);
        if init_state >= 2
            || crate::state::SHIM_STATE
                .load(std::sync::atomic::Ordering::Acquire)
                .is_null()
        {
            if let Some(err) = quick_block_vfs_mutation(path) {
                return err;
            }
            return crate::syscalls::macos_raw::raw_unlink(path);
        }
        let real = std::mem::transmute::<
            *mut libc::c_void,
            unsafe extern "C" fn(*const c_char) -> c_int,
        >(crate::reals::REAL_UNLINK.get());
        block_vfs_mutation(path).unwrap_or_else(|| real(path))
    }
    #[cfg(target_os = "linux")]
    {
        let init_state = INITIALIZING.load(Ordering::Relaxed);
        if init_state >= 2
            || crate::state::SHIM_STATE
                .load(std::sync::atomic::Ordering::Acquire)
                .is_null()
        {
            if let Some(err) = quick_block_vfs_mutation(path) {
                return err;
            }
            return crate::syscalls::linux_raw::raw_unlink(path);
        }
        block_vfs_mutation(path).unwrap_or_else(|| crate::syscalls::linux_raw::raw_rmdir(path))
    }
}

#[no_mangle]
pub unsafe extern "C" fn unlinkat_shim(dirfd: c_int, path: *const c_char, flags: c_int) -> c_int {
    #[cfg(target_os = "macos")]
    {
        let init_state = INITIALIZING.load(Ordering::Relaxed);
        if init_state >= 2 || crate::state::SHIM_STATE.load(Ordering::Acquire).is_null() {
            if let Some(err) = quick_block_vfs_mutation(path) {
                return err;
            }
            return crate::syscalls::macos_raw::raw_unlinkat(dirfd, path, flags);
        }
        let real = std::mem::transmute::<
            *mut libc::c_void,
            unsafe extern "C" fn(c_int, *const c_char, c_int) -> c_int,
        >(crate::reals::REAL_UNLINKAT.get());
        block_vfs_mutation(path).unwrap_or_else(|| real(dirfd, path, flags))
    }
    #[cfg(target_os = "linux")]
    {
        if INITIALIZING.load(Ordering::Relaxed) >= 2
            || crate::state::SHIM_STATE.load(Ordering::Acquire).is_null()
        {
            if let Some(err) = quick_block_vfs_mutation(path) {
                return err;
            }
            return crate::syscalls::linux_raw::raw_unlinkat(dirfd, path, flags);
        }
        block_vfs_mutation(path)
            .unwrap_or_else(|| crate::syscalls::linux_raw::raw_unlinkat(dirfd, path, flags))
    }
}

#[no_mangle]
pub unsafe extern "C" fn mkdirat_shim(
    dirfd: c_int,
    path: *const c_char,
    mode: libc::mode_t,
) -> c_int {
    #[cfg(target_os = "macos")]
    {
        let init_state = INITIALIZING.load(Ordering::Relaxed);
        if init_state >= 2 || crate::state::SHIM_STATE.load(Ordering::Acquire).is_null() {
            if let Some(err) = quick_block_vfs_mutation(path) {
                return err;
            }
            return crate::syscalls::macos_raw::raw_mkdirat(dirfd, path, mode);
        }
        let real = std::mem::transmute::<
            *mut libc::c_void,
            unsafe extern "C" fn(c_int, *const c_char, libc::mode_t) -> c_int,
        >(crate::reals::REAL_MKDIRAT.get());
        block_vfs_mutation(path).unwrap_or_else(|| real(dirfd, path, mode))
    }
    #[cfg(target_os = "linux")]
    {
        if INITIALIZING.load(Ordering::Relaxed) >= 2
            || crate::state::SHIM_STATE.load(Ordering::Acquire).is_null()
        {
            if let Some(err) = quick_block_vfs_mutation(path) {
                return err;
            }
            return crate::syscalls::linux_raw::raw_mkdirat(dirfd, path, mode);
        }
        block_vfs_mutation(path)
            .unwrap_or_else(|| crate::syscalls::linux_raw::raw_mkdirat(dirfd, path, mode))
    }
}

#[no_mangle]
pub unsafe extern "C" fn symlinkat_shim(
    p1: *const c_char,
    dirfd: c_int,
    p2: *const c_char,
) -> c_int {
    #[cfg(target_os = "macos")]
    {
        let init_state = INITIALIZING.load(Ordering::Relaxed);
        if init_state >= 2 || crate::state::SHIM_STATE.load(Ordering::Acquire).is_null() {
            if let Some(err) = quick_block_vfs_mutation(p1).or_else(|| quick_block_vfs_mutation(p2))
            {
                return err;
            }
            return crate::syscalls::macos_raw::raw_symlinkat(p1, dirfd, p2);
        }
        let real = std::mem::transmute::<
            *mut libc::c_void,
            unsafe extern "C" fn(*const c_char, c_int, *const c_char) -> c_int,
        >(crate::reals::REAL_SYMLINKAT.get());
        block_vfs_mutation(p1)
            .or_else(|| block_vfs_mutation(p2))
            .unwrap_or_else(|| real(p1, dirfd, p2))
    }
    #[cfg(target_os = "linux")]
    {
        if INITIALIZING.load(Ordering::Relaxed) >= 2
            || crate::state::SHIM_STATE.load(Ordering::Acquire).is_null()
        {
            if let Some(err) = quick_block_vfs_mutation(p1).or_else(|| quick_block_vfs_mutation(p2))
            {
                return err;
            }
            return crate::syscalls::linux_raw::raw_symlinkat(p1, dirfd, p2);
        }
        block_vfs_mutation(p1)
            .or_else(|| block_vfs_mutation(p2))
            .unwrap_or_else(|| crate::syscalls::linux_raw::raw_symlinkat(p1, dirfd, p2))
    }
}

#[no_mangle]
pub unsafe extern "C" fn rmdir_shim(path: *const c_char) -> c_int {
    #[cfg(target_os = "macos")]
    {
        let init_state = INITIALIZING.load(Ordering::Relaxed);
        if init_state >= 2
            || crate::state::SHIM_STATE
                .load(std::sync::atomic::Ordering::Acquire)
                .is_null()
        {
            if let Some(err) = quick_block_vfs_mutation(path) {
                return err;
            }
            return crate::syscalls::macos_raw::raw_rmdir(path);
        }
        let real = std::mem::transmute::<
            *mut libc::c_void,
            unsafe extern "C" fn(*const c_char) -> c_int,
        >(crate::reals::REAL_RMDIR.get());
        block_vfs_mutation(path).unwrap_or_else(|| real(path))
    }
    #[cfg(target_os = "linux")]
    {
        let init_state = INITIALIZING.load(Ordering::Relaxed);
        if init_state >= 2
            || crate::state::SHIM_STATE
                .load(std::sync::atomic::Ordering::Acquire)
                .is_null()
        {
            if let Some(err) = quick_block_vfs_mutation(path) {
                return err;
            }
            return crate::syscalls::linux_raw::raw_rmdir(path);
        }
        block_vfs_mutation(path).unwrap_or_else(|| crate::syscalls::linux_raw::raw_rmdir(path))
    }
}

#[no_mangle]
#[cfg(target_os = "macos")]
pub unsafe extern "C" fn mkdir_shim(path: *const c_char, mode: libc::mode_t) -> c_int {
    let init_state = INITIALIZING.load(Ordering::Relaxed);
    if init_state >= 2
        || crate::state::SHIM_STATE
            .load(std::sync::atomic::Ordering::Acquire)
            .is_null()
    {
        if let Some(err) = quick_block_vfs_mutation(path) {
            return err;
        }
        return crate::syscalls::macos_raw::raw_mkdir(path, mode);
    }
    let real = std::mem::transmute::<
        *mut libc::c_void,
        unsafe extern "C" fn(*const c_char, libc::mode_t) -> c_int,
    >(crate::reals::REAL_MKDIR.get());
    block_vfs_mutation(path).unwrap_or_else(|| real(path, mode))
}

#[no_mangle]
#[cfg(target_os = "linux")]
pub unsafe extern "C" fn mkdir_shim(path: *const c_char, mode: libc::mode_t) -> c_int {
    let init_state = INITIALIZING.load(Ordering::Relaxed);
    if init_state >= 2
        || crate::state::SHIM_STATE
            .load(std::sync::atomic::Ordering::Acquire)
            .is_null()
    {
        if let Some(err) = quick_block_vfs_mutation(path) {
            return err;
        }
        return crate::syscalls::linux_raw::raw_mkdir(path, mode);
    }
    block_vfs_mutation(path).unwrap_or_else(|| crate::syscalls::linux_raw::raw_mkdir(path, mode))
}

/// Helper: Check if path is in VFS and return EPERM if so
/// RFC-0048: Must check is_vfs_ready() FIRST to avoid deadlock during init (Pattern 2543)
/// RFC-0052: Standalone mode - check VRIFT_VFS_PREFIX even without daemon
pub(crate) unsafe fn block_vfs_mutation(path: *const c_char) -> Option<c_int> {
    if path.is_null() {
        return None;
    }

    let path_str = CStr::from_ptr(path).to_str().ok()?;

    // First try: Full shim state check (daemon connected)
    if let Some(_guard) = ShimGuard::enter() {
        if let Some(state) = ShimState::get() {
            if state.psfs_applicable(path_str) {
                crate::set_errno(libc::EPERM);
                return Some(-1);
            }
            return None;
        }
    }

    // Fallback: Standalone VFS prefix check (no daemon needed)
    // This ensures mutation perimeter works even in tests without full daemon
    // CRITICAL: Use libc::getenv() directly to avoid TLS hang (Pattern 2648)
    // std::env::var() triggers Rust runtime TLS which hangs during shim init
    let env_name = b"VRIFT_VFS_PREFIX\0";
    let vfs_prefix_ptr = libc::getenv(env_name.as_ptr() as *const c_char);
    if !vfs_prefix_ptr.is_null() {
        if let Ok(vfs_prefix) = CStr::from_ptr(vfs_prefix_ptr).to_str() {
            if path_str.starts_with(vfs_prefix) {
                crate::set_errno(libc::EPERM);
                return Some(-1);
            }
        }
    }

    None
}

/// Lightweight VFS check for raw syscall path - avoids TLS/ShimGuard
/// Only checks VRIFT_VFS_PREFIX env var, safe to call during early init
#[inline]
pub(crate) unsafe fn quick_block_vfs_mutation(path: *const c_char) -> Option<c_int> {
    if path.is_null() {
        return None;
    }
    let path_str = CStr::from_ptr(path).to_str().ok()?;
    let env_name = b"VRIFT_VFS_PREFIX\0";
    let vfs_prefix_ptr = libc::getenv(env_name.as_ptr() as *const c_char);
    if !vfs_prefix_ptr.is_null() {
        if let Ok(vfs_prefix) = CStr::from_ptr(vfs_prefix_ptr).to_str() {
            if path_str.starts_with(vfs_prefix) {
                crate::set_errno(libc::EPERM);
                return Some(-1);
            }
        }
    }
    None
}

// --- chmod/fchmod ---

#[no_mangle]
#[cfg(target_os = "macos")]
pub unsafe extern "C" fn chmod_shim(path: *const c_char, mode: libc::mode_t) -> c_int {
    // BUG-007: Use raw syscall during early init OR when shim not fully ready
    // to avoid dlsym recursion and TLS pthread deadlock
    let init_state = crate::state::INITIALIZING.load(std::sync::atomic::Ordering::Relaxed);
    if init_state >= 2
        || crate::state::SHIM_STATE
            .load(std::sync::atomic::Ordering::Acquire)
            .is_null()
    {
        // Still check VFS prefix even in raw syscall path
        if let Some(err) = quick_block_vfs_mutation(path) {
            return err;
        }
        return crate::syscalls::macos_raw::raw_chmod(path, mode);
    }

    let real = std::mem::transmute::<
        *mut libc::c_void,
        unsafe extern "C" fn(*const c_char, libc::mode_t) -> c_int,
    >(crate::reals::REAL_CHMOD.get());
    block_vfs_mutation(path).unwrap_or_else(|| real(path, mode))
}

#[no_mangle]
#[cfg(target_os = "macos")]
pub unsafe extern "C" fn fchmodat_shim(
    dirfd: c_int,
    path: *const c_char,
    mode: libc::mode_t,
    flags: c_int,
) -> c_int {
    let init_state = INITIALIZING.load(Ordering::Relaxed);
    if init_state >= 2
        || crate::state::SHIM_STATE
            .load(std::sync::atomic::Ordering::Acquire)
            .is_null()
    {
        if let Some(err) = quick_block_vfs_mutation(path) {
            return err;
        }
        // No raw syscall for fchmodat, use real function
    }
    let real = std::mem::transmute::<
        *mut libc::c_void,
        unsafe extern "C" fn(c_int, *const c_char, libc::mode_t, c_int) -> c_int,
    >(crate::reals::REAL_FCHMODAT.get());
    block_vfs_mutation(path).unwrap_or_else(|| real(dirfd, path, mode, flags))
}

#[no_mangle]
pub unsafe extern "C" fn fchmod_shim(fd: c_int, mode: libc::mode_t) -> c_int {
    #[cfg(target_os = "macos")]
    {
        let init_state = INITIALIZING.load(Ordering::Relaxed);
        if init_state >= 2 || crate::state::SHIM_STATE.load(Ordering::Acquire).is_null() {
            return crate::syscalls::macos_raw::raw_fchmod(fd, mode);
        }

        // RFC-OPT-001: Recursion protection
        let _guard = match ShimGuard::enter() {
            Some(g) => g,
            None => return crate::syscalls::macos_raw::raw_fchmod(fd, mode),
        };

        // VFS logic: if FD points to a VFS file, block mutation
        // Strategy: Try to get path from FD (robust)
        let mut path_buf = [0; 1024];
        if unsafe { libc::fcntl(fd, libc::F_GETPATH, path_buf.as_mut_ptr()) } == 0 {
            let path_cstr = unsafe { CStr::from_ptr(path_buf.as_ptr()) };
            if let Ok(path_str) = path_cstr.to_str() {
                if let Some(state) = ShimState::get() {
                    if state.psfs_applicable(path_str) {
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
            || crate::state::SHIM_STATE.load(Ordering::Acquire).is_null()
        {
            return crate::syscalls::linux_raw::raw_fchmod(fd, mode);
        }

        let _guard = match ShimGuard::enter() {
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
                if let Some(state) = ShimState::get() {
                    if state.psfs_applicable(path_str) {
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

// --- truncate ---

#[no_mangle]
#[cfg(target_os = "macos")]
pub unsafe extern "C" fn truncate_shim(path: *const c_char, length: libc::off_t) -> c_int {
    let init_state = INITIALIZING.load(Ordering::Relaxed);
    if init_state >= 2
        || crate::state::SHIM_STATE
            .load(std::sync::atomic::Ordering::Acquire)
            .is_null()
    {
        if let Some(err) = quick_block_vfs_mutation(path) {
            return err;
        }
        return crate::syscalls::macos_raw::raw_truncate(path, length);
    }
    let real = std::mem::transmute::<
        *mut libc::c_void,
        unsafe extern "C" fn(*const c_char, libc::off_t) -> c_int,
    >(crate::reals::REAL_TRUNCATE.get());
    block_vfs_mutation(path).unwrap_or_else(|| real(path, length))
}

// --- chflags (macOS only) ---

#[no_mangle]
#[cfg(target_os = "macos")]
pub unsafe extern "C" fn chflags_shim(path: *const c_char, flags: libc::c_uint) -> c_int {
    let init_state = INITIALIZING.load(Ordering::Relaxed);
    if init_state >= 2
        || crate::state::SHIM_STATE
            .load(std::sync::atomic::Ordering::Acquire)
            .is_null()
    {
        if let Some(err) = quick_block_vfs_mutation(path) {
            return err;
        }
    }
    let real = std::mem::transmute::<
        *mut libc::c_void,
        unsafe extern "C" fn(*const c_char, libc::c_uint) -> c_int,
    >(crate::reals::REAL_CHFLAGS.get());
    block_vfs_mutation(path).unwrap_or_else(|| real(path, flags))
}

// --- xattr ---

#[no_mangle]
#[cfg(target_os = "macos")]
pub unsafe extern "C" fn setxattr_shim(
    path: *const c_char,
    name: *const c_char,
    value: *const c_void,
    size: libc::size_t,
    position: u32,
    options: c_int,
) -> c_int {
    let init_state = INITIALIZING.load(Ordering::Relaxed);
    if init_state >= 2
        || crate::state::SHIM_STATE
            .load(std::sync::atomic::Ordering::Acquire)
            .is_null()
    {
        if let Some(err) = quick_block_vfs_mutation(path) {
            return err;
        }
    }
    let real = std::mem::transmute::<
        *mut libc::c_void,
        unsafe extern "C" fn(
            *const c_char,
            *const c_char,
            *const c_void,
            libc::size_t,
            u32,
            c_int,
        ) -> c_int,
    >(crate::reals::REAL_SETXATTR.get());
    block_vfs_mutation(path).unwrap_or_else(|| real(path, name, value, size, position, options))
}

#[no_mangle]
#[cfg(target_os = "macos")]
pub unsafe extern "C" fn removexattr_shim(
    path: *const c_char,
    name: *const c_char,
    options: c_int,
) -> c_int {
    let init_state = INITIALIZING.load(Ordering::Relaxed);
    if init_state >= 2
        || crate::state::SHIM_STATE
            .load(std::sync::atomic::Ordering::Acquire)
            .is_null()
    {
        if let Some(err) = quick_block_vfs_mutation(path) {
            return err;
        }
    }
    let real = std::mem::transmute::<
        *mut libc::c_void,
        unsafe extern "C" fn(*const c_char, *const c_char, c_int) -> c_int,
    >(crate::reals::REAL_REMOVEXATTR.get());
    block_vfs_mutation(path).unwrap_or_else(|| real(path, name, options))
}

// ============================================================================
// RFC-0047: Timestamp Modification Protection
// ============================================================================

/// utimes_shim: Block timestamp modifications on VFS files
#[no_mangle]
#[cfg(target_os = "macos")]
pub unsafe extern "C" fn utimes_shim(path: *const c_char, times: *const libc::timeval) -> c_int {
    let init_state = INITIALIZING.load(Ordering::Relaxed);
    if init_state >= 2
        || crate::state::SHIM_STATE
            .load(std::sync::atomic::Ordering::Acquire)
            .is_null()
    {
        if let Some(err) = quick_block_vfs_mutation(path) {
            return err;
        }
    }
    let real = std::mem::transmute::<
        *mut libc::c_void,
        unsafe extern "C" fn(*const c_char, *const libc::timeval) -> c_int,
    >(crate::reals::REAL_UTIMES.get());
    block_vfs_mutation(path).unwrap_or_else(|| real(path, times))
}

/// utimensat_shim: Block timestamp modifications on VFS files (at variant)
#[allow(unused_variables)]
pub unsafe extern "C" fn utimensat_shim(
    dirfd: c_int,
    path: *const c_char,
    times: *const libc::timespec,
    flags: c_int,
) -> c_int {
    #[cfg(target_os = "macos")]
    {
        let real = std::mem::transmute::<
            *mut libc::c_void,
            unsafe extern "C" fn(c_int, *const c_char, *const libc::timespec, c_int) -> c_int,
        >(crate::reals::REAL_UTIMENSAT.get());
        passthrough_if_init!(real, dirfd, path, times, flags);
        block_vfs_mutation(path).unwrap_or_else(|| real(dirfd, path, times, flags))
    }
    #[cfg(target_os = "linux")]
    {
        // For Linux, we don't have utimensat shim yet, return passthrough
        -1
    }
}
