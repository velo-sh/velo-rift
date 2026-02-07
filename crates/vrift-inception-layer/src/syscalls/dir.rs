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
        let mut fs_vpath = crate::state::FixedString::<1024>::new();
        fs_vpath.set(path_str);
        let syn_dir = Box::new(SyntheticDir {
            vpath: fs_vpath,
            entries,
            position: 0,
        });
        let ptr = Box::into_raw(syn_dir) as *mut c_void;

        // Track in open_dirs
        let mut dirs = state.open_dirs.lock();
        dirs.insert(
            ptr as usize,
            SyntheticDir {
                vpath: crate::state::FixedString::new(),
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
    // Pattern 2930: Raw syscall for bootstrap safety
    #[cfg(target_os = "macos")]
    {
        use crate::state::InceptionLayerState;
        let raw_getcwd = crate::syscalls::macos_raw::raw_getcwd;

        // Early-boot passthrough
        passthrough_if_init!(raw_getcwd, buf, size);

        let res = raw_getcwd(buf, size);
        if res.is_null() {
            return res;
        }

        // Check if we need to reverse-map the path
        if let Some(state) = InceptionLayerState::get() {
            let real_cwd = match CStr::from_ptr(buf).to_str() {
                Ok(s) => s,
                Err(_) => return res,
            };

            let prefix = state.path_resolver.vfs_prefix.as_str();
            let project_root = state.path_resolver.project_root.as_str();

            if !project_root.is_empty() && real_cwd.starts_with(project_root) {
                // Map project_root -> vfs_prefix
                let relative = &real_cwd[project_root.len()..];
                let relative = relative.strip_prefix('/').unwrap_or(relative);

                let virt_cwd = if relative.is_empty() {
                    prefix.to_string()
                } else {
                    format!("{}/{}", prefix, relative)
                };

                // Copy back to buffer if it fits
                if virt_cwd.len() < size {
                    let bytes = virt_cwd.as_bytes();
                    std::ptr::copy_nonoverlapping(bytes.as_ptr(), buf as *mut u8, bytes.len());
                    *(buf.add(bytes.len())) = 0;
                    return buf;
                } else {
                    crate::set_errno(libc::ERANGE);
                    return std::ptr::null_mut();
                }
            }
        }

        res
    }
    #[cfg(target_os = "linux")]
    return crate::syscalls::linux_raw::raw_getcwd(buf, size);
}

#[no_mangle]
pub unsafe extern "C" fn chdir_inception(path: *const libc::c_char) -> c_int {
    // Pattern 2930: Raw syscall for bootstrap safety
    #[cfg(target_os = "macos")]
    {
        use crate::state::InceptionLayerState;
        use std::ffi::CStr;

        let raw_chdir = crate::syscalls::macos_raw::raw_chdir;

        // Early-boot passthrough
        passthrough_if_init!(raw_chdir, path);

        if path.is_null() {
            return raw_chdir(path);
        }

        let path_str = match CStr::from_ptr(path).to_str() {
            Ok(s) => s,
            Err(_) => return raw_chdir(path),
        };

        // Get inception layer state
        let state = match InceptionLayerState::get() {
            Some(s) => s,
            None => return raw_chdir(path),
        };

        // Check if path is in VFS domain
        if let Some(_vfs_path) = state.resolve_path(path_str) {
            // Map VFS path to real filesystem path:
            // VFS prefix      -> project_root
            // /vrift/subdir   -> /real/project/subdir
            let prefix = state.path_resolver.vfs_prefix.as_str();
            let project_root = state.path_resolver.project_root.as_str();

            let relative = path_str.strip_prefix(prefix).unwrap_or("");
            let relative = relative.strip_prefix('/').unwrap_or(relative);

            let real_path = if relative.is_empty() {
                project_root.to_string()
            } else {
                format!("{}/{}", project_root, relative)
            };

            let c_real = match std::ffi::CString::new(real_path) {
                Ok(c) => c,
                Err(_) => return raw_chdir(path),
            };
            return raw_chdir(c_real.as_ptr());
        }

        // Not in VFS domain, passthrough
        raw_chdir(path)
    }
    #[cfg(target_os = "linux")]
    return crate::syscalls::linux_raw::raw_chdir(path);
}
