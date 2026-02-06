// Symbols imported from reals.rs via crate::reals
#[cfg(target_os = "macos")]
use crate::state::*;
#[cfg(target_os = "linux")]
use libc::c_int;
#[cfg(target_os = "macos")]
use libc::{c_int, c_void};
#[cfg(target_os = "macos")]
use std::ffi::CStr;

#[no_mangle]
#[cfg(target_os = "macos")]
pub unsafe extern "C" fn opendir_inception(path: *const libc::c_char) -> *mut c_void {
    // RFC-0050: Use IT_OPENDIR.old_func from interpose table to avoid recursion.
    // dlsym(RTLD_NEXT) returns the inception layer itself due to __interpose mechanism.
    let real = std::mem::transmute::<
        *const (),
        unsafe extern "C" fn(*const libc::c_char) -> *mut c_void,
    >(crate::interpose::IT_OPENDIR.old_func);

    // Early-boot passthrough
    passthrough_if_init!(real, path);

    if path.is_null() {
        return real(path);
    }

    let path_str = match CStr::from_ptr(path).to_str() {
        Ok(s) => s,
        Err(_) => return real(path),
    };

    // Get inception layer state
    let state = match InceptionLayerState::get() {
        Some(s) => s,
        None => return real(path),
    };

    // Check if path is in VFS domain
    if !state.inception_applicable(path_str) {
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
        let mut dirs = state.open_dirs.lock();
        dirs.insert(
            ptr as usize,
            SyntheticDir {
                vpath: String::new(),
                entries: vec![],
                position: 0,
            },
        );

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
pub unsafe extern "C" fn readdir_inception(dir: *mut c_void) -> *mut libc::dirent {
    // RFC-0050: Use IT_READDIR.old_func to avoid recursion (same fix as opendir_inception)
    let real = std::mem::transmute::<
        *const (),
        unsafe extern "C" fn(*mut c_void) -> *mut libc::dirent,
    >(crate::interpose::IT_READDIR.old_func);

    // Pattern 2648/2649: Passthrough during initialization to avoid TLS hazard
    passthrough_if_init!(real, dir);

    if dir.is_null() {
        return real(dir);
    }

    // Check if this is a synthetic directory
    let syn_dir = dir as *mut SyntheticDir;

    // Try to read from synthetic dir if it's one of ours
    if let Some(state) = InceptionLayerState::get() {
        let dirs = state.open_dirs.lock();
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

    real(dir)
}

#[no_mangle]
#[cfg(target_os = "macos")]
pub unsafe extern "C" fn closedir_inception(dir: *mut c_void) -> c_int {
    // RFC-0050: Use IT_CLOSEDIR.old_func to avoid recursion (same fix as opendir_inception)
    let real = std::mem::transmute::<*const (), unsafe extern "C" fn(*mut c_void) -> c_int>(
        crate::interpose::IT_CLOSEDIR.old_func,
    );

    // Pattern 2648/2649: Passthrough during initialization to avoid TLS hazard
    passthrough_if_init!(real, dir);

    if dir.is_null() {
        return real(dir);
    }

    // Check if this is a synthetic directory
    if let Some(state) = InceptionLayerState::get() {
        let is_synthetic = {
            let mut dirs = state.open_dirs.lock();
            dirs.remove(&(dir as usize)).is_some()
        };

        if is_synthetic {
            // Free the synthetic directory
            let _ = Box::from_raw(dir as *mut SyntheticDir);
            return 0;
        }
    }

    real(dir)
}
#[no_mangle]
pub unsafe extern "C" fn getcwd_inception(
    buf: *mut libc::c_char,
    size: libc::size_t,
) -> *mut libc::c_char {
    // Pattern 2930: Use raw syscall to avoid post-init dlsym hazard
    #[cfg(target_os = "macos")]
    return crate::syscalls::macos_raw::raw_getcwd(buf, size);
    #[cfg(target_os = "linux")]
    return crate::syscalls::linux_raw::raw_getcwd(buf, size);
}

#[no_mangle]
pub unsafe extern "C" fn chdir_inception(path: *const libc::c_char) -> c_int {
    // Pattern 2930: Use raw syscall to avoid post-init dlsym hazard
    #[cfg(target_os = "macos")]
    return crate::syscalls::macos_raw::raw_chdir(path);
    #[cfg(target_os = "linux")]
    return crate::syscalls::linux_raw::raw_chdir(path);
}
