use libc::{c_int, c_void, off_t, size_t};

#[no_mangle]
pub unsafe extern "C" fn mmap_inception(
    addr: *mut c_void,
    len: size_t,
    prot: c_int,
    flags: c_int,
    fd: c_int,
    offset: off_t,
) -> *mut c_void {
    // RFC-0051: Always use raw syscall for mmap to avoid any dlsym dependency.
    // mmap is called during __malloc_init before dlsym is safe.
    #[cfg(target_os = "macos")]
    return crate::syscalls::macos_raw::raw_mmap(addr, len, prot, flags, fd, offset);
    #[cfg(target_os = "linux")]
    return crate::syscalls::linux_raw::raw_mmap(addr, len, prot, flags, fd, offset);
}

#[no_mangle]
pub unsafe extern "C" fn munmap_inception(addr: *mut c_void, len: size_t) -> c_int {
    // RFC-0051: Always use raw syscall for munmap to avoid any dlsym dependency.
    #[cfg(target_os = "macos")]
    return crate::syscalls::macos_raw::raw_munmap(addr, len);
    #[cfg(target_os = "linux")]
    return crate::syscalls::linux_raw::raw_munmap(addr, len);
}
