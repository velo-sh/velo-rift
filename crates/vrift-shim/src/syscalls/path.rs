#[cfg(target_os = "macos")]
use crate::state::ShimGuard;
use libc::{c_char, size_t, ssize_t};

// Symbols imported from reals.rs via crate::reals

#[no_mangle]
#[cfg(target_os = "macos")]
pub unsafe extern "C" fn velo_readlink_impl(
    path: *const c_char,
    buf: *mut c_char,
    bufsiz: size_t,
) -> ssize_t {
    // Use raw syscall for fallback to avoid dlsym deadlock (Pattern 2682.v2)
    let _guard = match ShimGuard::enter() {
        Some(g) => g,
        None => return crate::syscalls::macos_raw::raw_readlink(path, buf, bufsiz),
    };

    // readlink doesn't need VFS resolution, just passthrough for now
    crate::syscalls::macos_raw::raw_readlink(path, buf, bufsiz)
}

#[no_mangle]
#[cfg(target_os = "macos")]
pub unsafe extern "C" fn readlink_shim(
    path: *const c_char,
    buf: *mut c_char,
    bufsiz: size_t,
) -> ssize_t {
    let init_state = crate::state::INITIALIZING.load(std::sync::atomic::Ordering::Relaxed);
    if init_state >= 2 {
        return crate::syscalls::macos_raw::raw_readlink(path, buf, bufsiz);
    }
    velo_readlink_impl(path, buf, bufsiz)
}

#[no_mangle]
#[cfg(target_os = "macos")]
pub unsafe extern "C" fn velo_realpath_impl(
    path: *const c_char,
    resolved_path: *mut c_char,
) -> *mut c_char {
    // Use raw syscall for fallback to avoid dlsym deadlock (Pattern 2682.v2)
    let _guard = match ShimGuard::enter() {
        Some(g) => g,
        None => return crate::syscalls::macos_raw::raw_realpath(path, resolved_path),
    };

    crate::syscalls::macos_raw::raw_realpath(path, resolved_path)
}

#[no_mangle]
#[cfg(target_os = "macos")]
pub unsafe extern "C" fn realpath_shim(
    path: *const c_char,
    resolved_path: *mut c_char,
) -> *mut c_char {
    let init_state = crate::state::INITIALIZING.load(std::sync::atomic::Ordering::Relaxed);
    if init_state >= 2 {
        return crate::syscalls::macos_raw::raw_realpath(path, resolved_path);
    }
    velo_realpath_impl(path, resolved_path)
}
