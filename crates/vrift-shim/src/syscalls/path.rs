#[cfg(target_os = "macos")]
use libc::{c_char, size_t, ssize_t};

// Symbols imported from reals.rs via crate::reals

#[no_mangle]
#[cfg(target_os = "macos")]
pub unsafe extern "C" fn readlink_shim(
    path: *const c_char,
    buf: *mut c_char,
    bufsiz: size_t,
) -> ssize_t {
    let real = std::mem::transmute::<
        *mut libc::c_void,
        unsafe extern "C" fn(*const c_char, *mut c_char, size_t) -> ssize_t,
    >(crate::reals::REAL_READLINK.get());
    passthrough_if_init!(real, path, buf, bufsiz);
    real(path, buf, bufsiz)
}

#[no_mangle]
#[cfg(target_os = "macos")]
pub unsafe extern "C" fn realpath_shim(
    path: *const c_char,
    resolved_path: *mut c_char,
) -> *mut c_char {
    let real = std::mem::transmute::<
        *mut libc::c_void,
        unsafe extern "C" fn(*const c_char, *mut c_char) -> *mut c_char,
    >(crate::reals::REAL_REALPATH.get());
    passthrough_if_init!(real, path, resolved_path);
    real(path, resolved_path)
}
