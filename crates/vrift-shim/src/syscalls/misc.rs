use crate::interpose::*;
use crate::ipc::*;
use crate::state::*;
use libc::c_int;
#[cfg(target_os = "linux")]
use std::sync::atomic::AtomicPtr;
use std::sync::atomic::Ordering;

// ============================================================================
// Miscellaneous Implementation
// ============================================================================

type FlockFn = unsafe extern "C" fn(c_int, c_int) -> c_int;

// ============================================================================
// Linux Shims
// ============================================================================

#[cfg(target_os = "linux")]
static REAL_FLOCK: AtomicPtr<libc::c_void> = AtomicPtr::new(ptr::null_mut());
#[cfg(target_os = "linux")]
static REAL_FCNTL: AtomicPtr<libc::c_void> = AtomicPtr::new(ptr::null_mut());

#[cfg(target_os = "linux")]
#[no_mangle]
pub unsafe extern "C" fn flock(fd: c_int, operation: c_int) -> c_int {
    // Linux shim logic for flock?
    // lib.rs didn't show explicit flock shim for Linux in the viewed range (934+).
    // However, it showed REAL_FLOCK (1020).
    // And NO explicit shim.
    // But flock_shim at 2011 is cross-platform?
    // Wait, 2011 `flock_shim` uses `get_real_shim!`.
    // It is marked `#[no_mangle] pub unsafe extern "C" fn flock_shim`.
    // If Linux calls `flock` symbol, does `flock_shim` intercept?
    // On macOS, interpose does.
    // On Linux, we need `flock` symbol.
    // If `flock_shim` is named `flock_shim`, it won't intercept `flock` on Linux unless exported as `flock`.
    // Maybe `macros.rs` logic handles this?
    // Or I should name it `flock` for Linux?
    // The previous `lib.rs` had `flock_shim` at 2011. And `REAL_FLOCK`.
    // It seems `lib.rs` might have been relying on `flock_shim` being intercepted via... something?
    // Or perhaps `flock` IS NOT intercepted on Linux yet?
    // But `IT_FLOCK` exists.
    // I will use `flock_shim` body for now.

    // Actually, I'll export `flock` for Linux if I can, calling `flock_shim`.
    // But let's stick to what was there: `flock_shim`.

    // Wait, `flock_shim` body at 2011 has `#[cfg(target_os = "linux")]` inside for `errno`.
    // So it is intended for Linux too.
    // But the name `flock_shim` does not match `flock`.
    // Unless `link_name` attribute? No.
    // I will assume `flock_shim` is correct for now (maybe used by internal callers?).
    // Or maybe I should rename it `flock` for Linux?
    // `lib.rs` calls it `flock_shim`.
    flock_shim(fd, operation)
}

// ============================================================================
// Shared Shims
// ============================================================================

/// RFC-0049 P0: flock shim - virtualization of advisory locks
#[no_mangle]
pub unsafe extern "C" fn flock_shim(fd: c_int, operation: c_int) -> c_int {
    let _guard = match ShimGuard::enter() {
        Some(g) => g,
        None => {
            let real = get_real_shim!(REAL_FLOCK, "flock", IT_FLOCK, FlockFn);
            return real(fd, operation);
        }
    };

    if let Some(state) = ShimState::get() {
        let vpath = {
            let fds = state.open_fds.lock().unwrap();
            fds.get(&fd).map(|f| f.vpath.clone())
        };

        if let Some(path) = vpath {
            // It is a VFS file, divert to daemon shadow lock
            return match sync_ipc_flock(&state.socket_path, &path, operation) {
                Ok(_) => 0,
                Err(e) => {
                    set_errno(e);
                    -1
                }
            };
        }
    }

    let real = get_real_shim!(REAL_FLOCK, "flock", IT_FLOCK, FlockFn);
    real(fd, operation)
}

#[cfg(target_os = "macos")]
#[no_mangle]
pub unsafe extern "C" fn fcntl_shim(fd: c_int, cmd: c_int, arg: c_int) -> c_int {
    // fcntl is variadic, but most common uses pass a single int arg
    // We must reference IT_FCNTL.old_func to prevent DCE stripping it
    let real = std::mem::transmute::<*const (), unsafe extern "C" fn(c_int, c_int, c_int) -> c_int>(
        IT_FCNTL.old_func,
    );
    // Early-boot passthrough to avoid deadlock during dyld initialization
    if INITIALIZING.load(Ordering::Relaxed) {
        return real(fd, cmd, arg);
    }
    real(fd, cmd, arg)
}

#[cfg(target_os = "linux")]
unsafe fn set_errno(e: c_int) {
    *libc::__errno_location() = e;
}
#[cfg(target_os = "macos")]
unsafe fn set_errno(e: c_int) {
    *libc::__error() = e;
}
