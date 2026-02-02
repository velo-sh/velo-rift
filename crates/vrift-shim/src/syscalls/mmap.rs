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
    // BUG-007: mmap is called during __malloc_init before dlsym is safe.
    // Use raw syscall to completely bypass libc.
    let init_state = crate::state::INITIALIZING.load(std::sync::atomic::Ordering::Relaxed);
    if init_state >= 2 || crate::state::CIRCUIT_TRIPPED.load(std::sync::atomic::Ordering::Relaxed) {
        return crate::syscalls::macos_raw::raw_mmap(addr, len, prot, flags, fd, offset);
    }

    let real = std::mem::transmute::<
        *mut libc::c_void,
        unsafe extern "C" fn(*mut c_void, size_t, c_int, c_int, c_int, off_t) -> *mut c_void,
    >(crate::reals::REAL_MMAP.get());
    real(addr, len, prot, flags, fd, offset)
}

#[no_mangle]
#[cfg(target_os = "macos")]
pub unsafe extern "C" fn munmap_shim(addr: *mut c_void, len: size_t) -> c_int {
    // BUG-007: munmap may also be called during early init
    let init_state = crate::state::INITIALIZING.load(std::sync::atomic::Ordering::Relaxed);
    if init_state >= 2 || crate::state::CIRCUIT_TRIPPED.load(std::sync::atomic::Ordering::Relaxed) {
        return crate::syscalls::macos_raw::raw_munmap(addr, len);
    }

    let real = std::mem::transmute::<
        *mut libc::c_void,
        unsafe extern "C" fn(*mut c_void, size_t) -> c_int,
    >(crate::reals::REAL_MUNMAP.get());
    real(addr, len)
}
