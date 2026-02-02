//! # velo-shim
//!
//! LD_PRELOAD / DYLD_INSERT_LIBRARIES shim for Velo Rift virtual filesystem.
//! Industrial-grade, zero-allocation, and recursion-safe.

// Allow dead code during incremental restoration - functions will be connected in later phases
#![allow(dead_code)]
// Allow unsafe FFI functions without safety docs - these are inherently unsafe C ABI
#![allow(clippy::missing_safety_doc)]
// Allow static mut refs for FFI buffers - carefully managed in single-threaded context
#![allow(static_mut_refs)]

// Macros must be defined before modules that use them
#[macro_use]
pub mod macros;

pub mod interpose;
pub mod ipc;
pub mod path;
pub mod state;
pub mod syscalls;

extern "C" {
    fn set_vfs_errno(e: libc::c_int);
    fn get_vfs_errno() -> libc::c_int;
}

/// RFC-0051: Platform-agnostic errno access
#[no_mangle]
pub unsafe extern "C" fn set_errno(e: libc::c_int) {
    #[cfg(target_os = "macos")]
    {
        if (unsafe { crate::state::INITIALIZING.load(std::sync::atomic::Ordering::Relaxed) }) == 2 {
            return;
        }
        *libc::__error() = e;
    }
    #[cfg(target_os = "linux")]
    {
        set_vfs_errno(e);
    }
}

#[no_mangle]
pub unsafe extern "C" fn get_errno() -> libc::c_int {
    #[cfg(target_os = "macos")]
    {
        if (unsafe { crate::state::INITIALIZING.load(std::sync::atomic::Ordering::Relaxed) }) == 2 {
            return 0;
        }
        *libc::__error()
    }
    #[cfg(target_os = "linux")]
    {
        get_vfs_errno()
    }
}

// Re-export for linkage - interpose table (macOS) and unified impls (Linux)
pub use interpose::*;
pub use state::LOGGER;
// Note: syscalls module is used internally by interpose, not re-exported
