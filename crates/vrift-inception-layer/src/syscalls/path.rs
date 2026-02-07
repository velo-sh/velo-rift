use crate::state::*;
use libc::{c_char, size_t, ssize_t};
use std::ffi::CStr;

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
    // Pattern 2930: Raw syscall for bootstrap safety
    #[cfg(target_os = "macos")]
    let raw_realpath = crate::syscalls::macos_raw::raw_realpath;
    #[cfg(target_os = "linux")]
    let raw_realpath = libc::realpath; // Fallback or linux_raw

    // Early-boot passthrough
    passthrough_if_init!(raw_realpath, path, resolved_path);

    if path.is_null() {
        return raw_realpath(path, resolved_path);
    }

    let path_str = match CStr::from_ptr(path).to_str() {
        Ok(s) => s,
        Err(_) => return raw_realpath(path, resolved_path),
    };

    // Get inception layer state
    if let Some(state) = InceptionLayerState::get() {
        // Resolve path to see if it's VFS
        if let Some(vfs_path) = state.resolve_path(path_str) {
            // RFC-0049: realpath for a virtual path returns the virtual path itself.
            // This is required to maintain the illusion of the virtual namespace.
            let virt_path = vfs_path.absolute.as_str();

            // Copy to resolved_path if provided, otherwise allocate
            if !resolved_path.is_null() {
                // Buffer must be at least PATH_MAX
                let bytes = virt_path.as_bytes();
                std::ptr::copy_nonoverlapping(
                    bytes.as_ptr(),
                    resolved_path as *mut u8,
                    bytes.len(),
                );
                *(resolved_path.add(bytes.len())) = 0;
                return resolved_path;
            } else {
                // If NULL, we must allocate with malloc (libc::malloc)
                // Note: std::ffi::CString::into_raw is not enough as it uses Rust allocator
                let len = virt_path.len() + 1;
                let ptr = libc::malloc(len) as *mut c_char;
                if ptr.is_null() {
                    return std::ptr::null_mut();
                }
                let bytes = virt_path.as_bytes();
                std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr as *mut u8, bytes.len());
                *(ptr.add(bytes.len())) = 0;
                return ptr;
            }
        }
    }

    raw_realpath(path, resolved_path)
}

#[no_mangle]
pub unsafe extern "C" fn realpath_inception(
    path: *const c_char,
    resolved_path: *mut c_char,
) -> *mut c_char {
    velo_realpath_impl(path, resolved_path)
}
