// RFC-0047: VFS boundary enforcement for rename operations
#[cfg(target_os = "macos")]
use crate::interpose::*;
use crate::state::*;
#[cfg(target_os = "macos")]
use libc::c_void;
use libc::{c_char, c_int};
use std::ffi::CStr;
#[cfg(target_os = "macos")]
use std::sync::atomic::Ordering;

#[cfg(target_os = "macos")]
#[inline]
unsafe fn set_errno(e: c_int) {
    *libc::__error() = e;
}

#[cfg(target_os = "linux")]
#[inline]
unsafe fn set_errno(e: c_int) {
    *libc::__errno_location() = e;
}

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
        set_errno(libc::EXDEV);
        return Some(-1);
    }

    None // Let real syscall handle non-VFS renames
}

#[no_mangle]
#[cfg(target_os = "macos")]
pub unsafe extern "C" fn rename_shim(old: *const c_char, new: *const c_char) -> c_int {
    let real = std::mem::transmute::<
        *const (),
        unsafe extern "C" fn(*const c_char, *const c_char) -> c_int,
    >(IT_RENAME.old_func);
    if INITIALIZING.load(Ordering::Relaxed) {
        return real(old, new);
    }
    rename_impl(old, new).unwrap_or_else(|| real(old, new))
}

#[no_mangle]
#[cfg(target_os = "macos")]
pub unsafe extern "C" fn renameat_shim(
    oldfd: c_int,
    old: *const c_char,
    newfd: c_int,
    new: *const c_char,
) -> c_int {
    let real = std::mem::transmute::<
        *const (),
        unsafe extern "C" fn(c_int, *const c_char, c_int, *const c_char) -> c_int,
    >(IT_RENAMEAT.old_func);
    if INITIALIZING.load(Ordering::Relaxed) {
        return real(oldfd, old, newfd, new);
    }
    // For now, use simple EXDEV check for at-variants too
    rename_impl(old, new).unwrap_or_else(|| real(oldfd, old, newfd, new))
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
        set_errno(libc::EXDEV);
        return Some(-1);
    }

    None // Let real syscall handle non-VFS links
}

#[no_mangle]
#[cfg(target_os = "macos")]
pub unsafe extern "C" fn link_shim(old: *const c_char, new: *const c_char) -> c_int {
    let real = std::mem::transmute::<
        *const (),
        unsafe extern "C" fn(*const c_char, *const c_char) -> c_int,
    >(IT_LINK.old_func);
    if INITIALIZING.load(Ordering::Relaxed) {
        return real(old, new);
    }
    link_impl(old, new).unwrap_or_else(|| real(old, new))
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
    let real = std::mem::transmute::<
        *const (),
        unsafe extern "C" fn(c_int, *const c_char, c_int, *const c_char, c_int) -> c_int,
    >(IT_LINKAT.old_func);
    if INITIALIZING.load(Ordering::Relaxed) {
        return real(oldfd, old, newfd, new, flags);
    }
    link_impl(old, new).unwrap_or_else(|| real(oldfd, old, newfd, new, flags))
}

// ============================================================================
// RFC-0047: Mutation Perimeter - Block modifications to VFS-managed files
// ============================================================================

/// Helper: Check if path is in VFS and return EPERM if so
#[cfg(target_os = "macos")]
unsafe fn block_vfs_mutation(path: *const c_char) -> Option<c_int> {
    if path.is_null() {
        return None;
    }
    let _guard = ShimGuard::enter()?;
    let state = ShimState::get()?;
    let path_str = CStr::from_ptr(path).to_str().ok()?;

    if state.psfs_applicable(path_str) {
        set_errno(libc::EPERM);
        return Some(-1);
    }
    None
}

// --- chmod/fchmod ---

#[no_mangle]
#[cfg(target_os = "macos")]
pub unsafe extern "C" fn chmod_shim(path: *const c_char, mode: libc::mode_t) -> c_int {
    let real = std::mem::transmute::<
        *const (),
        unsafe extern "C" fn(*const c_char, libc::mode_t) -> c_int,
    >(IT_CHMOD.old_func);
    if INITIALIZING.load(Ordering::Relaxed) {
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
    let real = std::mem::transmute::<
        *const (),
        unsafe extern "C" fn(c_int, *const c_char, libc::mode_t, c_int) -> c_int,
    >(IT_FCHMODAT.old_func);
    if INITIALIZING.load(Ordering::Relaxed) {
        return real(dirfd, path, mode, flags);
    }
    block_vfs_mutation(path).unwrap_or_else(|| real(dirfd, path, mode, flags))
}

// --- truncate ---

#[no_mangle]
#[cfg(target_os = "macos")]
pub unsafe extern "C" fn truncate_shim(path: *const c_char, length: libc::off_t) -> c_int {
    let real = std::mem::transmute::<
        *const (),
        unsafe extern "C" fn(*const c_char, libc::off_t) -> c_int,
    >(IT_TRUNCATE.old_func);
    if INITIALIZING.load(Ordering::Relaxed) {
        return real(path, length);
    }
    block_vfs_mutation(path).unwrap_or_else(|| real(path, length))
}

// --- chflags (macOS only) ---

#[no_mangle]
#[cfg(target_os = "macos")]
pub unsafe extern "C" fn chflags_shim(path: *const c_char, flags: libc::c_uint) -> c_int {
    let real = std::mem::transmute::<
        *const (),
        unsafe extern "C" fn(*const c_char, libc::c_uint) -> c_int,
    >(IT_CHFLAGS.old_func);
    if INITIALIZING.load(Ordering::Relaxed) {
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
    if INITIALIZING.load(Ordering::Relaxed) {
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
    let real = std::mem::transmute::<
        *const (),
        unsafe extern "C" fn(*const c_char, *const c_char, c_int) -> c_int,
    >(IT_REMOVEXATTR.old_func);
    if INITIALIZING.load(Ordering::Relaxed) {
        return real(path, name, options);
    }
    block_vfs_mutation(path).unwrap_or_else(|| real(path, name, options))
}
