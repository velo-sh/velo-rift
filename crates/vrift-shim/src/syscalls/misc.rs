// RFC-0047: VFS boundary enforcement for rename operations
#[cfg(target_os = "macos")]
use crate::interpose::*;
use crate::state::*;
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
