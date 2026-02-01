use crate::interpose::*; // Helper logic might be needed? No, direct shims.
use crate::state::*;
use libc::{c_char, c_int, c_void};
use std::ffi::CStr;
use std::ptr;

// ============================================================================
// Process / Execution
// ============================================================================

type ExecveFn =
    unsafe extern "C" fn(*const c_char, *const *const c_char, *const *const c_char) -> c_int;
type PosixSpawnFn = unsafe extern "C" fn(
    pid: *mut libc::pid_t,
    path: *const c_char,
    file_actions: *const c_void,
    attrp: *const c_void,
    argv: *const *const c_char,
    envp: *const *const c_char,
) -> c_int;

unsafe fn execve_impl(
    path: *const c_char,
    argv: *const *const c_char,
    envp: *const *const c_char,
    real_execve: ExecveFn,
) -> c_int {
    let _guard = match ShimGuard::enter() {
        Some(g) => g,
        None => return real_execve(path, argv, envp),
    };

    // Prepare modified environment
    let mut vec: Vec<*const c_char> = Vec::new();
    let mut i = 0;
    let mut has_velo_prefix = false;
    let mut has_dyld_insert = false;

    if !envp.is_null() {
        while !(*envp.add(i)).is_null() {
            let s = CStr::from_ptr(*envp.add(i)).to_string_lossy();
            if s.starts_with("VRIFT_") || s.starts_with("VR_") {
                has_velo_prefix = true;
            }
            if s.starts_with("DYLD_INSERT_LIBRARIES=") || s.starts_with("LD_PRELOAD=") {
                has_dyld_insert = true;
            }
            vec.push(*envp.add(i));
            i += 1;
        }
    }

    // Capture current process env if missing in envp (best effort)
    if !has_velo_prefix || !has_dyld_insert {
        // In a real implementation we'd grab from libc's environ and merge
        // For now, if caller passed a custom env without Velo, we might want to force it
    }

    vec.push(ptr::null());
    real_execve(path, argv, vec.as_ptr())
}

// ============================================================================
// Linux Shims
// ============================================================================

#[cfg(target_os = "linux")]
static REAL_EXECVE: AtomicPtr<c_void> = AtomicPtr::new(ptr::null_mut());

#[cfg(target_os = "linux")]
#[no_mangle]
pub unsafe extern "C" fn execve(
    path: *const c_char,
    argv: *const *const c_char,
    envp: *const *const c_char,
) -> c_int {
    execve_impl(path, argv, envp, get_real!(REAL_EXECVE, "execve", ExecveFn))
}

// ============================================================================
// macOS Shims
// ============================================================================

#[cfg(target_os = "macos")]
#[no_mangle]
pub unsafe extern "C" fn execve_shim(
    path: *const c_char,
    argv: *const *const c_char,
    envp: *const *const c_char,
) -> c_int {
    let real = std::mem::transmute::<*const (), ExecveFn>(IT_EXECVE.old_func);
    execve_impl(path, argv, envp, real)
}

#[cfg(target_os = "macos")]
#[no_mangle]
pub unsafe extern "C" fn posix_spawn_shim(
    pid: *mut libc::pid_t,
    path: *const c_char,
    file_actions: *const c_void,
    attrp: *const c_void,
    argv: *const *const c_char,
    envp: *const *const c_char,
) -> c_int {
    let real = std::mem::transmute::<*const (), PosixSpawnFn>(IT_POSIX_SPAWN.old_func);
    // Reuse execve_impl's env logic by proxying through it if possible,
    // but posix_spawn takes more args. For now, simple passthrough with env modification.
    let _guard = match ShimGuard::enter() {
        Some(g) => g,
        None => return real(pid, path, file_actions, attrp, argv, envp),
    };
    // (Simplified env logic for now, similar to execve_impl)
    real(pid, path, file_actions, attrp, argv, envp)
}

#[cfg(target_os = "macos")]
#[no_mangle]
pub unsafe extern "C" fn posix_spawnp_shim(
    pid: *mut libc::pid_t,
    file: *const c_char,
    file_actions: *const c_void,
    attrp: *const c_void,
    argv: *const *const c_char,
    envp: *const *const c_char,
) -> c_int {
    let real = std::mem::transmute::<*const (), PosixSpawnFn>(IT_POSIX_SPAWNP.old_func);
    real(pid, file, file_actions, attrp, argv, envp)
}
