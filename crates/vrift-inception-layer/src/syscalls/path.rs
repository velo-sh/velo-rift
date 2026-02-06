use crate::state::*;
use libc::{c_char, size_t, ssize_t};

#[no_mangle]
pub unsafe extern "C" fn velo_readlink_impl(
    path: *const c_char,
    buf: *mut c_char,
    bufsiz: size_t,
) -> ssize_t {
    // Use raw syscall for fallback to avoid dlsym deadlock (Pattern 2682.v2)
    let _guard = match InceptionLayerGuard::enter() {
        Some(g) => g,
        None => {
            #[cfg(target_os = "macos")]
            return crate::syscalls::macos_raw::raw_readlink(path, buf, bufsiz);
            #[cfg(target_os = "linux")]
            return crate::syscalls::linux_raw::raw_readlink(path, buf, bufsiz);
        }
    };

    #[cfg(target_os = "macos")]
    return crate::syscalls::macos_raw::raw_readlink(path, buf, bufsiz);
    #[cfg(target_os = "linux")]
    return crate::syscalls::linux_raw::raw_readlink(path, buf, bufsiz);
}

#[no_mangle]
pub unsafe extern "C" fn readlink_inception(
    path: *const c_char,
    buf: *mut c_char,
    bufsiz: size_t,
) -> ssize_t {
    let init_state = crate::state::INITIALIZING.load(std::sync::atomic::Ordering::Relaxed);
    if init_state >= 2 {
        #[cfg(target_os = "macos")]
        return crate::syscalls::macos_raw::raw_readlink(path, buf, bufsiz);
        #[cfg(target_os = "linux")]
        return crate::syscalls::linux_raw::raw_readlink(path, buf, bufsiz);
    }
    velo_readlink_impl(path, buf, bufsiz)
}

#[no_mangle]
pub unsafe extern "C" fn velo_realpath_impl(
    path: *const c_char,
    resolved_path: *mut c_char,
) -> *mut c_char {
    // Use raw syscall for fallback to avoid dlsym deadlock (Pattern 2682.v2)
    let _guard = match InceptionLayerGuard::enter() {
        Some(g) => g,
        None => {
            #[cfg(target_os = "macos")]
            return crate::syscalls::macos_raw::raw_realpath(path, resolved_path);
            #[cfg(target_os = "linux")]
            return crate::syscalls::linux_raw::raw_realpath(path, resolved_path);
        }
    };

    #[cfg(target_os = "macos")]
    return crate::syscalls::macos_raw::raw_realpath(path, resolved_path);
    #[cfg(target_os = "linux")]
    return crate::syscalls::linux_raw::raw_realpath(path, resolved_path);
}

#[no_mangle]
pub unsafe extern "C" fn realpath_inception(
    path: *const c_char,
    resolved_path: *mut c_char,
) -> *mut c_char {
    let init_state = crate::state::INITIALIZING.load(std::sync::atomic::Ordering::Relaxed);
    if init_state >= 2 {
        #[cfg(target_os = "macos")]
        return crate::syscalls::macos_raw::raw_realpath(path, resolved_path);
        #[cfg(target_os = "linux")]
        return crate::syscalls::linux_raw::raw_realpath(path, resolved_path);
    }
    velo_realpath_impl(path, resolved_path)
}
