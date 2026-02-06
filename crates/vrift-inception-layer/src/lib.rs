//! # velo-inception-layer
//!
//! LD_PRELOAD / DYLD_INSERT_LIBRARIES inception-layer for Velo Rift virtual filesystem.
//! Industrial-grade, zero-allocation, and recursion-safe.
//!
//! # ⚠️ TLS SAFETY WARNING (Pattern 2648/2649)
//!
//! This inception layer runs during macOS `dyld` bootstrap phase. **ANY Rust TLS access
//! before `TLS_READY` flag is set will deadlock the process.**
//!
//! ## Forbidden during init phase:
//! - `String`, `Cow<str>`, `Vec` → use `*const libc::c_char` + `libc::malloc`
//! - `HashMap` → use raw pointers, lazy init after `TLS_READY`
//! - `println!`/`eprintln!` → use `libc::write(2, ...)`
//! - `panic!` → use `libc::abort()`
//!
//! ## Required testing after ANY change to init path:
//! ```bash
//! DYLD_INSERT_LIBRARIES=target/debug/libvrift_inception_layer.dylib /tmp/test_minimal
//! ```
//!
//! See `docs/INCEPTION_LAYER_SAFETY_GUIDE.md` for full documentation.

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
pub mod reals;
pub mod state;
pub mod sync;
pub mod syscalls;

extern "C" {
    fn set_inception_errno(e: libc::c_int);
    fn get_inception_errno() -> libc::c_int;
}

/// RFC-0051: Platform-agnostic errno access
#[no_mangle]
pub unsafe extern "C" fn set_errno(e: libc::c_int) {
    #[cfg(target_os = "macos")]
    {
        if (unsafe { crate::state::INITIALIZING.load(std::sync::atomic::Ordering::Relaxed) })
            == crate::state::InceptionState::EarlyInit as u8
        {
            return;
        }
        *libc::__error() = e;
    }
    #[cfg(target_os = "linux")]
    {
        set_inception_errno(e);
    }
}

#[no_mangle]
pub unsafe extern "C" fn get_errno() -> libc::c_int {
    #[cfg(target_os = "macos")]
    {
        if (unsafe { crate::state::INITIALIZING.load(std::sync::atomic::Ordering::Relaxed) })
            == crate::state::InceptionState::EarlyInit as u8
        {
            return 0;
        }
        *libc::__error()
    }
    #[cfg(target_os = "linux")]
    {
        get_inception_errno()
    }
}

// Re-export for linkage - interpose table (macOS) and unified impls (Linux)
#[cfg(target_os = "macos")]
pub use interpose::*;
pub use state::LOGGER;
// Note: syscalls module is used internally by interpose, not re-exported

/// RFC-0050: Telemetry Export for `vrift status --inception`
/// Writes a null-terminated JSON-like string to the provided buffer.
/// Returns actual length written, or -1 if buffer too small.
#[no_mangle]
pub unsafe extern "C" fn vrift_get_telemetry(
    buf: *mut libc::c_char,
    buf_size: usize,
) -> libc::c_int {
    use std::fmt::Write;
    if buf.is_null() || buf_size == 0 {
        return -1;
    }

    let mut scratch = [0u8; 4096];
    let mut writer = crate::macros::StackWriter::new(&mut scratch);

    // Collect stats (Zero-Alloc)
    let state = crate::state::InceptionLayerState::get();
    let is_ready = state.is_some();
    let init_state = crate::state::INITIALIZING.load(std::sync::atomic::Ordering::Relaxed);
    let pid = libc::getpid();

    // Flight Recorder Summary
    let mut counts = [0u32; 16];
    let head = crate::state::FLIGHT_RECORDER
        .head
        .load(std::sync::atomic::Ordering::Relaxed);
    // Scan last N entries (approximate) to avoid heavyweight locking
    let start = head.saturating_sub(1000);
    for i in start..head {
        let idx = i % crate::state::FLIGHT_RECORDER_SIZE;
        let entry = &crate::state::FLIGHT_RECORDER.buffer[idx];
        if entry.event_type > 0 && entry.event_type < 16 {
            counts[entry.event_type as usize] += 1;
        }
    }

    let _ = writeln!(writer, "{{");
    let _ = writeln!(writer, "  \"pid\": {},", pid);
    let _ = writeln!(
        writer,
        "  \"inception_state\": \"{}\",",
        match init_state {
            0 => "Ready",
            1 => "RustInit",
            2 => "EarlyInit",
            3 => "Busy",
            _ => "Unknown",
        }
    );
    let _ = writeln!(writer, "  \"vfs_active\": {},", is_ready);

    if let Some(s) = state {
        let _ = writeln!(writer, "  \"project_root\": \"{}\",", s.project_root);
        let _ = writeln!(
            writer,
            "  \"open_fds\": {},",
            crate::syscalls::io::OPEN_FD_COUNT.load(std::sync::atomic::Ordering::Relaxed)
        );
    }

    let _ = writeln!(writer, "  \"events_last_1k\": {{");
    for (i, name) in crate::state::EVENT_NAMES.iter().enumerate() {
        if i > 0 && i < counts.len() {
            let _ = writeln!(writer, "    \"{}\": {},", name, counts[i]);
        }
    }
    let _ = writeln!(writer, "    \"total_recorded\": {}", head);
    let _ = writeln!(writer, "  }}");
    let _ = write!(writer, "}}"); // End JSON

    let out_str = writer.as_str();
    let len = out_str.len();
    if len >= buf_size {
        return -1;
    }

    // Copy to caller buffer
    std::ptr::copy_nonoverlapping(out_str.as_ptr(), buf as *mut u8, len);
    *buf.add(len) = 0; // Null terminator

    len as libc::c_int
}
