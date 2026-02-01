#![allow(unused_imports)]
use libc::{c_char, c_int, c_void, AT_FDCWD};
use std::ffi::CStr;
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

static VIRTUAL_CWD_KEY_INIT: AtomicBool = AtomicBool::new(false);
static VIRTUAL_CWD_KEY_VALUE: AtomicUsize = AtomicUsize::new(0);

/// Get or create the pthread key for VIRTUAL_CWD storage.
/// Returns 0 if creation fails (will be treated as no virtual CWD).
fn get_virtual_cwd_key() -> libc::pthread_key_t {
    // Fast path: already initialized
    if VIRTUAL_CWD_KEY_INIT.load(Ordering::Acquire) {
        return VIRTUAL_CWD_KEY_VALUE.load(Ordering::Relaxed) as libc::pthread_key_t;
    }

    // Destructor to free the String when thread exits
    extern "C" fn destructor(ptr: *mut c_void) {
        if !ptr.is_null() {
            unsafe {
                let _ = Box::from_raw(ptr as *mut String);
            }
        }
    }

    // Slow path: initialize (only one thread will succeed)
    let mut key: libc::pthread_key_t = 0;
    let ret = unsafe { libc::pthread_key_create(&mut key, Some(destructor)) };
    if ret != 0 {
        return 0;
    }

    // Try to be the one to set the value (CAS)
    let expected = 0usize;
    if VIRTUAL_CWD_KEY_VALUE
        .compare_exchange(expected, key as usize, Ordering::SeqCst, Ordering::SeqCst)
        .is_ok()
    {
        VIRTUAL_CWD_KEY_INIT.store(true, Ordering::Release);
        key
    } else {
        // Another thread beat us, clean up and use their key
        unsafe { libc::pthread_key_delete(key) };
        VIRTUAL_CWD_KEY_VALUE.load(Ordering::Relaxed) as libc::pthread_key_t
    }
}

/// Get the current virtual CWD for this thread (if set).
pub(crate) fn get_virtual_cwd() -> Option<String> {
    let key = get_virtual_cwd_key();
    if key == 0 {
        return None;
    }
    let ptr = unsafe { libc::pthread_getspecific(key) };
    if ptr.is_null() {
        None
    } else {
        unsafe { Some((*(ptr as *const String)).clone()) }
    }
}

/// Set the virtual CWD for this thread.
pub(crate) fn set_virtual_cwd(path: Option<String>) {
    let key = get_virtual_cwd_key();
    if key == 0 {
        return;
    }

    // Free old value if any
    let old_ptr = unsafe { libc::pthread_getspecific(key) };
    if !old_ptr.is_null() {
        unsafe {
            let _ = Box::from_raw(old_ptr as *mut String);
        }
    }

    // Set new value
    let new_ptr = match path {
        Some(s) => Box::into_raw(Box::new(s)) as *mut c_void,
        None => ptr::null_mut(),
    };
    unsafe {
        libc::pthread_setspecific(key, new_ptr);
    }
}

/// Robust path normalization without heap allocation.
/// Handles "..", ".", and duplicate slashes.
/// Returns the length of the normalized path in `out`.
pub(crate) unsafe fn raw_path_normalize(path: &str, out: &mut [u8]) -> Option<usize> {
    if path.is_empty() || out.is_empty() {
        return None;
    }

    let bytes = path.as_bytes();
    let mut out_idx = 0;

    // Always start with / if input is absolute
    if bytes[0] == b'/' {
        out[out_idx] = b'/';
        out_idx += 1;
    }

    let mut i = 0;
    while i < bytes.len() {
        // Skip multiple slashes
        while i < bytes.len() && bytes[i] == b'/' {
            i += 1;
        }
        if i == bytes.len() {
            break;
        }

        // Find component end
        let start = i;
        while i < bytes.len() && bytes[i] != b'/' {
            i += 1;
        }
        let component = &bytes[start..i];

        if component == b"." {
            continue;
        } else if component == b".." {
            if out_idx > 1 {
                // Backtrack to previous slash
                out_idx -= 1;
                while out_idx > 1 && out[out_idx - 1] != b'/' {
                    out_idx -= 1;
                }
            }
        } else {
            // Add slash if not at root and last char isn't slash
            if out_idx > 0 && out[out_idx - 1] != b'/' {
                if out_idx < out.len() {
                    out[out_idx] = b'/';
                    out_idx += 1;
                } else {
                    return None;
                }
            }
            // Add component
            if out_idx + component.len() <= out.len() {
                ptr::copy_nonoverlapping(
                    component.as_ptr(),
                    out.as_mut_ptr().add(out_idx),
                    component.len(),
                );
                out_idx += component.len();
            } else {
                return None;
            }
        }
    }

    if out_idx == 0 {
        if bytes[0] == b'/' {
            out[0] = b'/';
        } else {
            out[0] = b'.';
        }
        out_idx = 1;
    }

    Some(out_idx)
}

/// Resolve a path, potentially relative to VIRTUAL_CWD.
pub(crate) unsafe fn resolve_path_with_cwd(path: &str, out: &mut [u8]) -> Option<usize> {
    if path.starts_with('/') {
        return raw_path_normalize(path, out);
    }

    if let Some(vpath) = get_virtual_cwd() {
        let mut tmp = [b'/'; 1024];
        let vbytes = vpath.as_bytes();
        if vbytes.len() < tmp.len() {
            ptr::copy_nonoverlapping(vbytes.as_ptr(), tmp.as_mut_ptr(), vbytes.len());
            let mut idx = vbytes.len();
            if idx > 0 && tmp[idx - 1] != b'/' && idx < tmp.len() {
                tmp[idx] = b'/';
                idx += 1;
            }
            let pbytes = path.as_bytes();
            if idx + pbytes.len() < tmp.len() {
                ptr::copy_nonoverlapping(pbytes.as_ptr(), tmp.as_mut_ptr().add(idx), pbytes.len());
                let full = std::str::from_utf8_unchecked(&tmp[..idx + pbytes.len()]);
                return raw_path_normalize(full, out);
            }
        }
    }

    raw_path_normalize(path, out)
}

/// RFC-0049: Generate virtual inode from path
/// Prevents st_ino collision when CAS dedup causes multiple logical files to share same blob
/// Uses a simple hash to generate unique inode per logical path
#[inline]
pub(crate) fn path_to_virtual_ino(path: &str) -> libc::ino_t {
    // Simple FNV-1a hash for O(1) inode generation
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in path.bytes() {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash as libc::ino_t
}

pub(crate) unsafe fn resolve_path_at(
    dirfd: c_int,
    path: *const c_char,
    out: &mut [u8],
) -> Option<usize> {
    let path_str = CStr::from_ptr(path).to_str().ok()?;
    if path_str.starts_with('/') {
        return raw_path_normalize(path_str, out);
    }
    if dirfd == AT_FDCWD {
        return resolve_path_with_cwd(path_str, out);
    }
    // Cannot resolve relative path to arbitrary dirfd easily without OS help.
    // Fallback to None (treat as non-VFS)
    None
}
