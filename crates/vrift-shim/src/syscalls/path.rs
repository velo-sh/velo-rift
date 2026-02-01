#[cfg(target_os = "macos")]
use crate::interpose::*;
#[cfg(target_os = "macos")]
use libc::{c_char, size_t, ssize_t};

#[no_mangle]
#[cfg(target_os = "macos")]
pub unsafe extern "C" fn readlink_shim(
    path: *const c_char,
    buf: *mut c_char,
    bufsiz: size_t,
) -> ssize_t {
    let real = std::mem::transmute::<
        *const (),
        unsafe extern "C" fn(*const c_char, *mut c_char, size_t) -> ssize_t,
    >(IT_READLINK.old_func);
    real(path, buf, bufsiz)
}

#[no_mangle]
#[cfg(target_os = "macos")]
pub unsafe extern "C" fn realpath_shim(
    path: *const c_char,
    resolved_path: *mut c_char,
) -> *mut c_char {
    let real = std::mem::transmute::<
        *const (),
        unsafe extern "C" fn(*const c_char, *mut c_char) -> *mut c_char,
    >(IT_REALPATH.old_func);
    real(path, resolved_path)
}
