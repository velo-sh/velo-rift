use crate::interpose::*;
use crate::ipc::*;
use crate::path::*;
use crate::state::*;
use libc::{c_char, c_int, c_void, size_t};
use std::ffi::{CStr, CString};
use std::sync::atomic::Ordering;

// ============================================================================
// Memory Mapping & Dynamic Loading
// ============================================================================

type MmapFn =
    unsafe extern "C" fn(*mut c_void, size_t, c_int, c_int, c_int, libc::off_t) -> *mut c_void;
type MunmapFn = unsafe extern "C" fn(*mut c_void, size_t) -> c_int;
type DlopenFn = unsafe extern "C" fn(*const c_char, c_int) -> *mut c_void;
type DlsymFn = unsafe extern "C" fn(*mut c_void, *const c_char) -> *mut c_void;

// ============================================================================
// Linux Shims
// ============================================================================

#[cfg(target_os = "linux")]
static REAL_MMAP: AtomicPtr<c_void> = AtomicPtr::new(ptr::null_mut());
#[cfg(target_os = "linux")]
static REAL_MUNMAP: AtomicPtr<c_void> = AtomicPtr::new(ptr::null_mut());
#[cfg(target_os = "linux")]
static REAL_DLOPEN: AtomicPtr<c_void> = AtomicPtr::new(ptr::null_mut());

#[cfg(target_os = "linux")]
#[no_mangle]
pub unsafe extern "C" fn mmap(
    addr: *mut c_void,
    len: size_t,
    prot: c_int,
    flags: c_int,
    fd: c_int,
    offset: libc::off_t,
) -> *mut c_void {
    let real = get_real!(REAL_MMAP, "mmap", MmapFn);
    real(addr, len, prot, flags, fd, offset)
}

#[cfg(target_os = "linux")]
#[no_mangle]
pub unsafe extern "C" fn dlopen(filename: *const c_char, flags: c_int) -> *mut c_void {
    let real = get_real!(REAL_DLOPEN, "dlopen", DlopenFn);
    real(filename, flags)
}

// ============================================================================
// macOS Shims
// ============================================================================

#[cfg(target_os = "macos")]
#[no_mangle]
pub unsafe extern "C" fn dlopen_shim(filename: *const c_char, flags: c_int) -> *mut c_void {
    let real = std::mem::transmute::<*const (), DlopenFn>(IT_DLOPEN.old_func);

    // Early bailout during initialization
    if INITIALIZING.load(Ordering::SeqCst) {
        return real(filename, flags);
    }

    // Guard recursion
    let _guard = match ShimGuard::enter() {
        Some(g) => g,
        None => return real(filename, flags),
    };

    // NULL filename = get main program handle
    if filename.is_null() {
        return real(filename, flags);
    }

    // Check if this is a VFS path
    let path_str = match CStr::from_ptr(filename).to_str() {
        Ok(s) => s,
        Err(_) => return real(filename, flags),
    };

    let Some(state) = ShimState::get() else {
        return real(filename, flags);
    };

    let mut path_buf = [0u8; 1024];
    let resolved_len = match resolve_path_with_cwd(path_str, &mut path_buf) {
        Some(len) => len,
        None => return real(filename, flags),
    };
    let resolved_path = unsafe { std::str::from_utf8_unchecked(&path_buf[..resolved_len]) };

    if state.psfs_applicable(resolved_path) {
        if let Some(entry) = state.psfs_lookup(resolved_path) {
            // Get content from CAS and write to temp file
            if let Ok(cas_guard) = state.cas.lock() {
                if let Some(ref cas) = *cas_guard {
                    if let Ok(content) = cas.get(&entry.content_hash) {
                        // Create temp file for the library
                        let temp_dir = std::env::temp_dir();
                        let lib_name = std::path::Path::new(path_str)
                            .file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or("vrift_lib.dylib");
                        let temp_path = temp_dir.join(format!("vrift_{}", lib_name));

                        if std::fs::write(&temp_path, &content).is_ok() {
                            if let Ok(c_path) = CString::new(temp_path.to_string_lossy().as_bytes())
                            {
                                return real(c_path.as_ptr(), flags);
                            }
                        }
                    }
                }
            }
        }
    }

    // Passthrough for non-VFS paths and fallback
    real(filename, flags)
}

#[cfg(target_os = "macos")]
#[no_mangle]
pub unsafe extern "C" fn dlsym_shim(handle: *mut c_void, symbol: *const c_char) -> *mut c_void {
    let real = std::mem::transmute::<*const (), DlsymFn>(IT_DLSYM.old_func);

    // Early bailout during initialization
    if INITIALIZING.load(Ordering::SeqCst) {
        return real(handle, symbol);
    }

    // Guard recursion
    let _guard = match ShimGuard::enter() {
        Some(g) => g,
        None => return real(handle, symbol),
    };

    real(handle, symbol)
}

/// RFC-0049 P0: mmap shim - tracking of shared mappings
#[no_mangle]
pub unsafe extern "C" fn mmap_shim(
    addr: *mut c_void,
    len: size_t,
    prot: c_int,
    flags: c_int,
    fd: c_int,
    offset: libc::off_t,
) -> *mut c_void {
    let _guard = match ShimGuard::enter() {
        Some(g) => g,
        None => {
            let real = get_real_shim!(REAL_MMAP, "mmap", IT_MMAP, MmapFn);
            return real(addr, len, prot, flags, fd, offset);
        }
    };

    let real = get_real_shim!(REAL_MMAP, "mmap", IT_MMAP, MmapFn);
    let result = real(addr, len, prot, flags, fd, offset);

    if result != libc::MAP_FAILED {
        if let Some(state) = ShimState::get() {
            let maybe_info = {
                let fds = state.open_fds.lock().unwrap();
                fds.get(&fd).map(|f| (f.vpath.clone(), f.temp_path.clone()))
            };

            if let Some((vpath, temp_path)) = maybe_info {
                shim_log("[VRift-Shim] Tracked mmap for: ");
                shim_log(&vpath);
                shim_log("\n");

                let start_addr = result as usize;
                let mut maps = state.active_mmaps.lock().unwrap();
                maps.insert(
                    start_addr,
                    MmapInfo {
                        vpath,
                        temp_path,
                        len: len as usize,
                    },
                );
            }
        }
    }
    result
}

/// RFC-0049 P0: munmap shim - reingest on unmap of tracked VFS region
#[no_mangle]
pub unsafe extern "C" fn munmap_shim(addr: *mut c_void, len: size_t) -> c_int {
    let _guard = match ShimGuard::enter() {
        Some(g) => g,
        None => {
            let real = get_real_shim!(REAL_MUNMAP, "munmap", IT_MUNMAP, MunmapFn);
            return real(addr, len);
        }
    };

    if let Some(state) = ShimState::get() {
        let info_opt = {
            let mut maps = state.active_mmaps.lock().unwrap();
            maps.remove(&(addr as usize))
        };

        if let Some(info) = info_opt {
            if sync_ipc_manifest_reingest(&state.socket_path, &info.vpath, &info.temp_path) {
                shim_log("[VRift-Shim] Re-ingested on munmap: ");
                shim_log(&info.vpath);
                shim_log("\n");
            }
        }
    }

    let real = get_real_shim!(REAL_MUNMAP, "munmap", IT_MUNMAP, MunmapFn);
    real(addr, len)
}
