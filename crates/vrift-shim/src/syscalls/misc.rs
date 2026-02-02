use crate::state::*;
use libc::{c_char, c_int, c_void};
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
    use crate::interpose::IT_RENAME;
    let real = std::mem::transmute::<
        *const (),
        unsafe extern "C" fn(*const c_char, *const c_char) -> c_int,
    >(IT_RENAME.old_func);
    if INITIALIZING.load(Ordering::Relaxed) != 0 {
        return real(old, new);
    }
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
        use crate::interpose::IT_RENAMEAT;
        let real = std::mem::transmute::<
            *const (),
            unsafe extern "C" fn(c_int, *const c_char, c_int, *const c_char) -> c_int,
        >(IT_RENAMEAT.old_func);
        if INITIALIZING.load(Ordering::Relaxed) != 0 {
            return real(oldfd, old, newfd, new);
        }

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
        if INITIALIZING.load(Ordering::Relaxed) != 0 {
            return link(old, new);
        }
        link_impl(old, new).unwrap_or_else(|| link(old, new))
    }
    #[cfg(target_os = "linux")]
    {
        if INITIALIZING.load(Ordering::Relaxed) != 0 {
            return crate::syscalls::open::raw_link(old, new);
        }
        link_impl(old, new).unwrap_or_else(|| crate::syscalls::open::raw_link(old, new))
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
        use crate::interpose::IT_LINKAT;
        let real = std::mem::transmute::<
            *const (),
            unsafe extern "C" fn(c_int, *const c_char, c_int, *const c_char, c_int) -> c_int,
        >(IT_LINKAT.old_func);
        if INITIALIZING.load(Ordering::Relaxed) != 0 {
            return real(oldfd, old, newfd, new, flags);
        }
        link_impl(old, new).unwrap_or_else(|| real(oldfd, old, newfd, new, flags))
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
        use crate::interpose::IT_UNLINK;
        let real = std::mem::transmute::<*const (), unsafe extern "C" fn(*const c_char) -> c_int>(
            IT_UNLINK.old_func,
        );
        if INITIALIZING.load(Ordering::Relaxed) != 0 {
            return real(path);
        }
        block_vfs_mutation(path).unwrap_or_else(|| real(path))
    }
    #[cfg(target_os = "linux")]
    {
        if INITIALIZING.load(Ordering::Relaxed) != 0 {
            return libc::unlink(path);
        }
        block_vfs_mutation(path).unwrap_or_else(|| libc::unlink(path))
    }
}

#[no_mangle]
pub unsafe extern "C" fn rmdir_shim(path: *const c_char) -> c_int {
    #[cfg(target_os = "macos")]
    {
        use crate::interpose::IT_RMDIR;
        let real = std::mem::transmute::<*const (), unsafe extern "C" fn(*const c_char) -> c_int>(
            IT_RMDIR.old_func,
        );
        if INITIALIZING.load(Ordering::Relaxed) != 0 {
            return real(path);
        }
        block_vfs_mutation(path).unwrap_or_else(|| real(path))
    }
    #[cfg(target_os = "linux")]
    {
        if INITIALIZING.load(Ordering::Relaxed) != 0 {
            return libc::rmdir(path);
        }
        block_vfs_mutation(path).unwrap_or_else(|| libc::rmdir(path))
    }
}

#[no_mangle]
pub unsafe extern "C" fn mkdir_shim(path: *const c_char, mode: libc::mode_t) -> c_int {
    #[cfg(target_os = "macos")]
    {
        use crate::interpose::IT_MKDIR;
        let real = std::mem::transmute::<
            *const (),
            unsafe extern "C" fn(*const c_char, libc::mode_t) -> c_int,
        >(IT_MKDIR.old_func);
        if INITIALIZING.load(Ordering::Relaxed) != 0 {
            return real(path, mode);
        }
        block_vfs_mutation(path).unwrap_or_else(|| real(path, mode))
    }
    #[cfg(target_os = "linux")]
    {
        if INITIALIZING.load(Ordering::Relaxed) != 0 {
            return libc::mkdir(path, mode);
        }
        block_vfs_mutation(path).unwrap_or_else(|| libc::mkdir(path, mode))
    }
}

/// Helper: Check if path is in VFS and return EPERM if so
/// RFC-0048: Must check is_vfs_ready() FIRST to avoid deadlock during init (Pattern 2543)
unsafe fn block_vfs_mutation(path: *const c_char) -> Option<c_int> {
    if path.is_null() {
        return None;
    }
    let _guard = ShimGuard::enter()?;
    let state = ShimState::get()?;
    let path_str = CStr::from_ptr(path).to_str().ok()?;

    if state.psfs_applicable(path_str) {
        crate::set_errno(libc::EPERM);
        return Some(-1);
    }
    None
}

// --- chmod/fchmod ---

#[no_mangle]
#[cfg(target_os = "macos")]
pub unsafe extern "C" fn chmod_shim(path: *const c_char, mode: libc::mode_t) -> c_int {
    use crate::interpose::IT_CHMOD;
    let real = std::mem::transmute::<
        *const (),
        unsafe extern "C" fn(*const c_char, libc::mode_t) -> c_int,
    >(IT_CHMOD.old_func);
    if INITIALIZING.load(Ordering::Relaxed) != 0 {
        return real(path, mode);
    }
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
    use crate::interpose::IT_FCHMODAT;
    let real = std::mem::transmute::<
        *const (),
        unsafe extern "C" fn(c_int, *const c_char, libc::mode_t, c_int) -> c_int,
    >(IT_FCHMODAT.old_func);
    if INITIALIZING.load(Ordering::Relaxed) != 0 {
        return real(dirfd, path, mode, flags);
    }
    block_vfs_mutation(path).unwrap_or_else(|| real(dirfd, path, mode, flags))
}

// --- truncate ---

#[no_mangle]
#[cfg(target_os = "macos")]
pub unsafe extern "C" fn truncate_shim(path: *const c_char, length: libc::off_t) -> c_int {
    use crate::interpose::IT_TRUNCATE;
    let real = std::mem::transmute::<
        *const (),
        unsafe extern "C" fn(*const c_char, libc::off_t) -> c_int,
    >(IT_TRUNCATE.old_func);
    if INITIALIZING.load(Ordering::Relaxed) != 0 {
        return real(path, length);
    }
    block_vfs_mutation(path).unwrap_or_else(|| real(path, length))
}

// --- chflags (macOS only) ---

#[no_mangle]
#[cfg(target_os = "macos")]
pub unsafe extern "C" fn chflags_shim(path: *const c_char, flags: libc::c_uint) -> c_int {
    use crate::interpose::IT_CHFLAGS;
    let real = std::mem::transmute::<
        *const (),
        unsafe extern "C" fn(*const c_char, libc::c_uint) -> c_int,
    >(IT_CHFLAGS.old_func);
    if INITIALIZING.load(Ordering::Relaxed) != 0 {
        return real(path, flags);
    }
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
    use crate::interpose::IT_SETXATTR;
    let real = std::mem::transmute::<
        *const (),
        unsafe extern "C" fn(
            *const c_char,
            *const c_char,
            *const c_void,
            libc::size_t,
            u32,
            c_int,
        ) -> c_int,
    >(IT_SETXATTR.old_func);
    if INITIALIZING.load(Ordering::Relaxed) != 0 {
        return real(path, name, value, size, position, options);
    }
    block_vfs_mutation(path).unwrap_or_else(|| real(path, name, value, size, position, options))
}

#[no_mangle]
#[cfg(target_os = "macos")]
pub unsafe extern "C" fn removexattr_shim(
    path: *const c_char,
    name: *const c_char,
    options: c_int,
) -> c_int {
    use crate::interpose::IT_REMOVEXATTR;
    let real = std::mem::transmute::<
        *const (),
        unsafe extern "C" fn(*const c_char, *const c_char, c_int) -> c_int,
    >(IT_REMOVEXATTR.old_func);
    if INITIALIZING.load(Ordering::Relaxed) != 0 {
        return real(path, name, options);
    }
    block_vfs_mutation(path).unwrap_or_else(|| real(path, name, options))
}

// ============================================================================
// RFC-0047: Timestamp Modification Protection
// ============================================================================

/// utimes_shim: Block timestamp modifications on VFS files
#[no_mangle]
#[cfg(target_os = "macos")]
pub unsafe extern "C" fn utimes_shim(path: *const c_char, times: *const libc::timeval) -> c_int {
    use crate::interpose::IT_UTIMES;
    let real = std::mem::transmute::<
        *const (),
        unsafe extern "C" fn(*const c_char, *const libc::timeval) -> c_int,
    >(IT_UTIMES.old_func);
    if INITIALIZING.load(Ordering::Relaxed) != 0 {
        return real(path, times);
    }
    block_vfs_mutation(path).unwrap_or_else(|| real(path, times))
}

/// utimensat_shim: Block timestamp modifications on VFS files (at variant)
pub unsafe extern "C" fn utimensat_shim(
    _dirfd: c_int,
    _path: *const c_char,
    _times: *const libc::timespec,
    _flags: c_int,
) -> c_int {
    #[cfg(target_os = "macos")]
    {
        use crate::interpose::IT_UTIMENSAT;
        let real = std::mem::transmute::<
            *const (),
            unsafe extern "C" fn(c_int, *const c_char, *const libc::timespec, c_int) -> c_int,
        >(IT_UTIMENSAT.old_func);
        if INITIALIZING.load(Ordering::Relaxed) != 0 {
            return real(_dirfd, _path, _times, _flags);
        }
        block_vfs_mutation(_path).unwrap_or_else(|| real(_dirfd, _path, _times, _flags))
    }
    #[cfg(target_os = "linux")]
    {
        // For Linux, we don't have utimensat shim yet, return passthrough
        -1
    }
}
