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

// Re-export for linkage - interpose table provides all shim symbols
#[cfg(target_os = "macos")]
pub use interpose::*;
pub use state::LOGGER;
// Note: syscalls module is used internally by interpose, not re-exported

/// RFC-0049: Static constructor for macOS to signal that the library
/// has been loaded and symbol resolution is complete.
/// This safely clears the INITIALIZING flag to enable shims.
#[cfg(target_os = "macos")]
#[link_section = "__DATA,__mod_init_func"]
pub static SET_READY: unsafe extern "C" fn() = {
    unsafe extern "C" fn ready() {
        crate::state::INITIALIZING.store(0, std::sync::atomic::Ordering::SeqCst);
    }
    ready
};

/// RFC-0049: Static constructor for Linux to signal that the library
/// has been loaded via LD_PRELOAD. Uses .init_array section.
#[cfg(target_os = "linux")]
#[link_section = ".init_array"]
#[used]
pub static SET_READY_LINUX: unsafe extern "C" fn() = {
    unsafe extern "C" fn ready() {
        crate::state::INITIALIZING.store(0, std::sync::atomic::Ordering::SeqCst);
    }
    ready
};
