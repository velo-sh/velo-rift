#[cfg(target_os = "macos")]
use crate::interpose::*;
#[cfg(target_os = "macos")]
use crate::state::*;
#[cfg(target_os = "macos")]
use libc::{c_int, c_void};
#[cfg(target_os = "macos")]
use std::ffi::CStr;
#[cfg(target_os = "macos")]
use std::sync::atomic::Ordering;

#[no_mangle]
#[cfg(target_os = "macos")]
pub unsafe extern "C" fn opendir_shim(path: *const libc::c_char) -> *mut c_void {
    let real = std::mem::transmute::<
        *const (),
        unsafe extern "C" fn(*const libc::c_char) -> *mut c_void,
    >(IT_OPENDIR.old_func);

    // Early-boot passthrough
    if INITIALIZING.load(Ordering::Relaxed) != 0 {
        return real(path);
    }

    if path.is_null() {
        return real(path);
    }

    let path_str = match CStr::from_ptr(path).to_str() {
        Ok(s) => s,
        Err(_) => return real(path),
    };

    // Get shim state
    let state = match ShimState::get() {
        Some(s) => s,
        None => return real(path),
    };

    // Check if path is in VFS domain
    if !state.psfs_applicable(path_str) {
        return real(path);
    }

    // Query directory listing from daemon
    if let Some(entries) = state.query_dir_listing(path_str) {
        // Create synthetic directory
        let syn_dir = Box::new(SyntheticDir {
            vpath: path_str.to_string(),
            entries,
            position: 0,
        });
        let ptr = Box::into_raw(syn_dir) as *mut c_void;

        // Track in open_dirs
        if let Ok(mut dirs) = state.open_dirs.lock() {
            dirs.insert(
                ptr as usize,
                SyntheticDir {
                    vpath: String::new(),
                    entries: vec![],
                    position: 0,
                },
            );
        }

        return ptr;
    }

    // Fallback to real
    real(path)
}

/// Static buffer for readdir dirent (readdir returns pointer to static data)
#[cfg(target_os = "macos")]
static mut DIRENT_BUF: libc::dirent = libc::dirent {
    d_ino: 0,
    d_seekoff: 0,
    d_reclen: 0,
    d_namlen: 0,
    d_type: 0,
    d_name: [0; 1024],
};

#[no_mangle]
#[cfg(target_os = "macos")]
pub unsafe extern "C" fn readdir_shim(dir: *mut c_void) -> *mut libc::dirent {
    let real = std::mem::transmute::<
        *const (),
        unsafe extern "C" fn(*mut c_void) -> *mut libc::dirent,
    >(IT_READDIR.old_func);

    if dir.is_null() {
        return real(dir);
    }

    // Check if this is a synthetic directory
    let syn_dir = dir as *mut SyntheticDir;

    // Try to read from synthetic dir if it's one of ours
    if let Some(state) = ShimState::get() {
        if let Ok(dirs) = state.open_dirs.lock() {
            if dirs.contains_key(&(dir as usize)) {
                drop(dirs); // Release lock before accessing syn_dir

                let sd = &mut *syn_dir;
                if sd.position >= sd.entries.len() {
                    return std::ptr::null_mut();
                }

                let entry = &sd.entries[sd.position];
                sd.position += 1;

                // Fill dirent buffer
                DIRENT_BUF.d_ino = 1; // Synthetic inode
                DIRENT_BUF.d_type = if entry.is_dir {
                    libc::DT_DIR
                } else {
                    libc::DT_REG
                };
                DIRENT_BUF.d_namlen = entry.name.len() as u16;

                // Copy name to buffer
                let name_bytes = entry.name.as_bytes();
                let copy_len = name_bytes.len().min(1023);
                std::ptr::copy_nonoverlapping(
                    name_bytes.as_ptr(),
                    DIRENT_BUF.d_name.as_mut_ptr() as *mut u8,
                    copy_len,
                );
                DIRENT_BUF.d_name[copy_len] = 0;

                return &mut DIRENT_BUF;
            }
        }
    }

    real(dir)
}

#[no_mangle]
#[cfg(target_os = "macos")]
pub unsafe extern "C" fn closedir_shim(dir: *mut c_void) -> c_int {
    let real = std::mem::transmute::<*const (), unsafe extern "C" fn(*mut c_void) -> c_int>(
        IT_CLOSEDIR.old_func,
    );

    if dir.is_null() {
        return real(dir);
    }

    // Check if this is a synthetic directory
    if let Some(state) = ShimState::get() {
        let is_synthetic = {
            if let Ok(mut dirs) = state.open_dirs.lock() {
                dirs.remove(&(dir as usize)).is_some()
            } else {
                false
            }
        };

        if is_synthetic {
            // Free the synthetic directory
            let _ = Box::from_raw(dir as *mut SyntheticDir);
            return 0;
        }
    }

    real(dir)
}
