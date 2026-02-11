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
    let real_opendir = std::mem::transmute::<
        *const (),
        unsafe extern "C" fn(*const libc::c_char) -> *mut c_void,
    >(crate::interpose::IT_OPENDIR.old_func);

    passthrough_if_init!(real_opendir, path);

    if path.is_null() {
        return real_opendir(path);
    }

    let path_str = match CStr::from_ptr(path).to_str() {
        Ok(s) => s,
        Err(_) => return real_opendir(path),
    };

    let state = match InceptionLayerState::get() {
        Some(s) => s,
        None => return real_opendir(path),
    };

    // RFC-0051++: Lazy Merged Directory Listing
    // We open BOTH physical and virtual sources and stream from both in readdir.
    if !state.inception_applicable(path_str) {
        return real_opendir(path);
    }

    let real_dir = real_opendir(path);
    let vdir_entries = state.query_dir_listing(path_str).unwrap_or_default();

    if vdir_entries.is_empty() && real_dir.is_null() {
        return std::ptr::null_mut();
    }

    let mut fs_vpath = crate::state::FixedString::<1024>::new();
    fs_vpath.set(path_str);

    let syn_dir = Box::new(SyntheticDir {
        vpath: fs_vpath,
        entries: vdir_entries,
        position: 0,
        real_dir,
    });

    crate::inception_log!(
        "OPENDIR path='{}' -> merging {} entries + real_dir={:?}",
        path_str,
        syn_dir.entries.len(),
        real_dir
    );

    let ptr = Box::into_raw(syn_dir) as *mut c_void;

    let mut dirs_guard = state.open_dirs.lock();
    dirs_guard.insert(
        ptr as usize,
        SyntheticDir {
            vpath: fs_vpath,
            entries: Vec::new(),
            position: 0,
            real_dir,
        },
    );

    ptr
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
    profile_count!(readdir_calls);
    let real_readdir = std::mem::transmute::<
        *const (),
        unsafe extern "C" fn(*mut c_void) -> *mut libc::dirent,
    >(crate::interpose::IT_READDIR.old_func);

    passthrough_if_init!(real_readdir, dir);

    if dir.is_null() {
        return real_readdir(dir);
    }

    // Check if this is a synthetic directory
    if let Some(state) = InceptionLayerState::get() {
        let is_synthetic = {
            let dirs = state.open_dirs.lock();
            dirs.contains_key(&(dir as usize))
        };

        if is_synthetic {
            let sd = &mut *(dir as *mut SyntheticDir);

            // Phase 1: Return virtual entries from manifest
            if sd.position < sd.entries.len() {
                let entry = &sd.entries[sd.position];
                sd.position += 1;

                // Fill dirent buffer via raw pointer (Rust 2024 safety)
                let ent_ptr = &raw mut DIRENT_BUF;
                (*ent_ptr).d_ino = 1; // Synthetic inode
                (*ent_ptr).d_type = if entry.is_dir {
                    libc::DT_DIR
                } else {
                    libc::DT_REG
                };
                (*ent_ptr).d_namlen = entry.name.len() as u16;

                // Copy name to buffer
                let name_bytes = entry.name.as_bytes();
                let copy_len = name_bytes.len().min(1023);
                std::ptr::copy_nonoverlapping(
                    name_bytes.as_ptr(),
                    (*ent_ptr).d_name.as_mut_ptr() as *mut u8,
                    copy_len,
                );
                (*ent_ptr).d_name[copy_len] = 0;

                return ent_ptr;
            }

            // Phase 2: Stream from physical directory, skipping duplicates
            if !sd.real_dir.is_null() {
                loop {
                    let ent = real_readdir(sd.real_dir);
                    if ent.is_null() {
                        return std::ptr::null_mut();
                    }

                    // Extract name from physical dirent
                    let name_ptr = (*ent).d_name.as_ptr();
                    let name_bytes = CStr::from_ptr(name_ptr as *const libc::c_char).to_bytes();
                    if let Ok(name) = std::str::from_utf8(name_bytes) {
                        if name == "." || name == ".." {
                            continue;
                        }

                        // Check if we already returned this name from VDir (deduplication)
                        if sd.entries.iter().any(|v| v.name == name) {
                            continue;
                        }

                        return ent;
                    }
                }
            }

            return std::ptr::null_mut();
        }
    }

    real_readdir(dir)
}

#[no_mangle]
#[cfg(target_os = "macos")]
pub unsafe extern "C" fn closedir_inception(dir: *mut c_void) -> c_int {
    let real_closedir = std::mem::transmute::<*const (), unsafe extern "C" fn(*mut c_void) -> c_int>(
        crate::interpose::IT_CLOSEDIR.old_func,
    );

    passthrough_if_init!(real_closedir, dir);

    if dir.is_null() {
        return real_closedir(dir);
    }

    // Check if this is a synthetic directory
    if let Some(state) = InceptionLayerState::get() {
        let mut dirs = state.open_dirs.lock();
        if dirs.remove(&(dir as usize)).is_some() {
            // Free the synthetic directory
            let syn_dir = Box::from_raw(dir as *mut SyntheticDir);
            if !syn_dir.real_dir.is_null() {
                real_closedir(syn_dir.real_dir);
            }
            return 0;
        }
    }

    real_closedir(dir)
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
        let status = raw_chdir(path);
        if status == 0 {
            // Update CWD cache
            let mut buf = [0u8; 1024];
            let res = crate::syscalls::macos_raw::raw_getcwd(
                buf.as_mut_ptr() as *mut libc::c_char,
                buf.len(),
            );
            if !res.is_null() {
                if let Ok(s) = CStr::from_ptr(res).to_str() {
                    let mut fs = FixedString::new();
                    fs.set(s);
                    CACHED_CWD.with(|cache| {
                        *cache.borrow_mut() = Some(fs);
                    });
                }
            }
        }
        status
    }
    #[cfg(target_os = "linux")]
    {
        let status = crate::syscalls::linux_raw::raw_chdir(path);
        if status == 0 {
            // Update CWD cache
            let mut buf = [0u8; 1024];
            let res = crate::syscalls::linux_raw::raw_getcwd(
                buf.as_mut_ptr() as *mut libc::c_char,
                buf.len(),
            );
            if !res.is_null() {
                if let Ok(s) = CStr::from_ptr(res).to_str() {
                    let mut fs = FixedString::new();
                    fs.set(s);
                    CACHED_CWD.with(|cache| {
                        *cache.borrow_mut() = Some(fs);
                    });
                }
            }
        }
        status
    }
}
