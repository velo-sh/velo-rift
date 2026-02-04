#[cfg(target_os = "macos")]
use libc::{c_int, c_void, off_t, size_t};

// Symbols imported from reals.rs via crate::reals

#[no_mangle]
#[cfg(target_os = "macos")]
pub unsafe extern "C" fn mmap_shim(
    addr: *mut c_void,
    len: size_t,
    prot: c_int,
    flags: c_int,
    fd: c_int,
    offset: off_t,
) -> *mut c_void {
    // RFC-0051: Always use raw syscall for mmap to avoid any dlsym dependency.
    // mmap is called during __malloc_init before dlsym is safe.
    crate::syscalls::macos_raw::raw_mmap(addr, len, prot, flags, fd, offset)
}

#[no_mangle]
#[cfg(target_os = "macos")]
pub unsafe extern "C" fn munmap_shim(addr: *mut c_void, len: size_t) -> c_int {
    // RFC-0051: Always use raw syscall for munmap to avoid any dlsym dependency.
    crate::syscalls::macos_raw::raw_munmap(addr, len)
}
