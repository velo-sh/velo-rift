use crate::interpose::*;
use crate::ipc::*;
use crate::state::*;
use libc::{c_int, c_void, size_t, ssize_t};
use std::sync::atomic::Ordering;

// ============================================================================
// I/O Implementations
// ============================================================================

unsafe fn write_impl(fd: c_int, buf: *const c_void, count: size_t, real_write: WriteFn) -> ssize_t {
    real_write(fd, buf, count)
}

unsafe fn close_impl(fd: c_int, real_close: CloseFn) -> c_int {
    // Skip if ShimState is not yet initialized (avoids malloc during dyld __malloc_init)
    if SHIM_STATE.load(Ordering::Acquire).is_null() {
        return real_close(fd);
    }

    let _guard = match ShimGuard::enter() {
        Some(g) => g,
        None => return real_close(fd),
    };

    if let Some(state) = ShimState::get() {
        let open_file = {
            let mut fds = state.open_fds.lock().unwrap();
            fds.remove(&fd)
        };

        if let Some(file) = open_file {
            // RFC-0047: CoW close path - reingest modified file back to CAS and Manifest
            // Daemon will read temp file, hash it, insert to CAS, update Manifest
            if sync_ipc_manifest_reingest(&state.socket_path, &file.vpath, &file.temp_path) {
                shim_log("[VRift-Shim] File re-ingested successfully: ");
                shim_log(&file.vpath);
                shim_log("\n");
            } else {
                shim_log("[VRift-Shim] File reingest IPC failed: ");
                shim_log(&file.vpath);
                shim_log("\n");
            }
        }
    }

    real_close(fd)
}

// ============================================================================
// Linux Shims
// ============================================================================

#[cfg(target_os = "linux")]
static REAL_WRITE: AtomicPtr<c_void> = AtomicPtr::new(ptr::null_mut());
#[cfg(target_os = "linux")]
static REAL_CLOSE: AtomicPtr<c_void> = AtomicPtr::new(ptr::null_mut());

#[cfg(target_os = "linux")]
#[no_mangle]
pub unsafe extern "C" fn write(fd: c_int, b: *const c_void, c: size_t) -> ssize_t {
    write_impl(fd, b, c, get_real!(REAL_WRITE, "write", WriteFn))
}

#[cfg(target_os = "linux")]
#[no_mangle]
pub unsafe extern "C" fn close(fd: c_int) -> c_int {
    close_impl(fd, get_real!(REAL_CLOSE, "close", CloseFn))
}

// ============================================================================
// macOS Shims
// ============================================================================

#[cfg(target_os = "macos")]
#[no_mangle]
pub unsafe extern "C" fn write_shim(fd: c_int, b: *const c_void, c: size_t) -> ssize_t {
    let real = std::mem::transmute::<*const (), WriteFn>(IT_WRITE.old_func);
    write_impl(fd, b, c, real)
}

#[cfg(target_os = "macos")]
#[no_mangle]
pub unsafe extern "C" fn close_shim(fd: c_int) -> c_int {
    let real = std::mem::transmute::<*const (), CloseFn>(IT_CLOSE.old_func);
    close_impl(fd, real)
}

#[cfg(target_os = "macos")]
#[no_mangle]
pub unsafe extern "C" fn read_shim(fd: c_int, buf: *mut c_void, count: size_t) -> ssize_t {
    let real = std::mem::transmute::<*const (), ReadFn>(IT_READ.old_func);
    // Passthrough to real read - shim tracks fds but read content comes from
    // CAS backing store which is the actual file content
    real(fd, buf, count)
}

type WriteFn = unsafe extern "C" fn(c_int, *const c_void, size_t) -> ssize_t;
type CloseFn = unsafe extern "C" fn(c_int) -> c_int;
#[cfg(target_os = "macos")]
type ReadFn = unsafe extern "C" fn(c_int, *mut c_void, size_t) -> ssize_t;
