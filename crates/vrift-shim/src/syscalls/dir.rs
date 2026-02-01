use crate::interpose::*;
use crate::state::*;
use libc::{c_char, c_int};
use std::ffi::CStr;
use std::ptr;
use std::sync::atomic::{AtomicUsize, Ordering};

// ============================================================================
// Directory Implementation
// ============================================================================

type OpendirFn = unsafe extern "C" fn(*const c_char) -> *mut libc::DIR;
type ReaddirFn = unsafe extern "C" fn(*mut libc::DIR) -> *mut libc::dirent;
type ClosedirFn = unsafe extern "C" fn(*mut libc::DIR) -> c_int;

/// Synthetic DIR handle counter (unique per synthetic directory)
static SYNTHETIC_DIR_COUNTER: AtomicUsize = AtomicUsize::new(0x7F000000);

unsafe fn opendir_impl(path: *const c_char, real_opendir: OpendirFn) -> *mut libc::DIR {
    // Early bailout during ShimState initialization to prevent CasStore::new recursion
    if INITIALIZING.load(Ordering::SeqCst) {
        return real_opendir(path);
    }

    // Skip if ShimState is not yet initialized (avoids malloc during dyld __malloc_init)
    if SHIM_STATE.load(Ordering::Acquire).is_null() {
        return real_opendir(path);
    }

    let _guard = match ShimGuard::enter() {
        Some(g) => g,
        None => return real_opendir(path),
    };

    let path_str = match CStr::from_ptr(path).to_str() {
        Ok(s) => s,
        Err(_) => return real_opendir(path),
    };

    let Some(state) = ShimState::get() else {
        return real_opendir(path);
    };

    // Check if this is a VFS path
    if path_str.starts_with(&*state.vfs_prefix) {
        // Normalize path: remove trailing slash except if it's just "/"
        let lookup_path = if path_str.len() > 1 {
            path_str.trim_end_matches('/')
        } else {
            path_str
        };

        let vpath = &path_str[state.vfs_prefix.len()..];

        // 1. Try mmap lookup first (Zero-Copy)
        if let Some((children_ptr, count)) =
            mmap_dir_lookup(state.mmap_ptr, state.mmap_size, lookup_path)
        {
            shim_log("[VRift] opendir mmap: ");
            shim_log(lookup_path);
            shim_log("\n");

            let handle = SYNTHETIC_DIR_COUNTER.fetch_add(1, Ordering::Relaxed);
            let synthetic = SyntheticDir {
                vpath: vpath.to_string(),
                entries: Vec::new(),
                mmap_children: Some((children_ptr, count)),
                position: 0,
            };
            let mut dirs = state.open_dirs.lock().unwrap();
            dirs.insert(handle, synthetic);
            return handle as *mut libc::DIR;
        }

        // 2. Fallback: Query daemon for directory entries (IPC)
        if let Some(entries) = state.query_dir_listing(lookup_path) {
            shim_log("[VRift] opendir IPC fallback: ");
            shim_log(lookup_path);
            shim_log("\n");

            // Create synthetic DIR handle
            let handle = SYNTHETIC_DIR_COUNTER.fetch_add(1, Ordering::Relaxed);

            let synthetic = SyntheticDir {
                vpath: vpath.to_string(),
                entries,
                mmap_children: None,
                position: 0,
            };

            let mut dirs = state.open_dirs.lock().unwrap();
            dirs.insert(handle, synthetic);

            shim_log("[VRift-Shim] opendir VFS (IPC): ");
            shim_log(lookup_path);
            shim_log("\n");

            // Return synthetic DIR* (cast handle as pointer)
            return handle as *mut libc::DIR;
        }
    }

    real_opendir(path)
}

/// Static dirent for returning from readdir (must be static to remain valid after return)
static mut SYNTHETIC_DIRENT: libc::dirent = unsafe { std::mem::zeroed() };

#[allow(static_mut_refs)] // Required for returning static dirent from readdir
unsafe fn readdir_impl(dir: *mut libc::DIR, real_readdir: ReaddirFn) -> *mut libc::dirent {
    let _guard = match ShimGuard::enter() {
        Some(g) => g,
        None => return real_readdir(dir),
    };

    let Some(state) = ShimState::get() else {
        return real_readdir(dir);
    };

    let handle = dir as usize;

    // Check if this is a synthetic directory
    let mut dirs = state.open_dirs.lock().unwrap();
    if let Some(synthetic) = dirs.get_mut(&handle) {
        // Case A: mmap-backed children (Zero-Copy)
        if let Some((children_ptr, count)) = synthetic.mmap_children {
            if synthetic.position >= count {
                return ptr::null_mut();
            }

            let child = unsafe { &*children_ptr.add(synthetic.position) };
            synthetic.position += 1;

            // Fill in the static dirent
            ptr::write_bytes(&mut SYNTHETIC_DIRENT, 0, 1);
            SYNTHETIC_DIRENT.d_ino = (handle + synthetic.position) as libc::ino_t;
            SYNTHETIC_DIRENT.d_type = if child.is_dir != 0 {
                libc::DT_DIR
            } else {
                libc::DT_REG
            };

            // Copy name using name_as_str helper
            let name = child.name_as_str();
            let name_bytes = name.as_bytes();
            let copy_len = std::cmp::min(name_bytes.len(), SYNTHETIC_DIRENT.d_name.len() - 1);
            ptr::copy_nonoverlapping(
                name_bytes.as_ptr(),
                SYNTHETIC_DIRENT.d_name.as_mut_ptr() as *mut u8,
                copy_len,
            );
            SYNTHETIC_DIRENT.d_name[copy_len] = 0;

            return &mut SYNTHETIC_DIRENT;
        }

        // Case B: IPC-backed entries (Fallback)
        if synthetic.position >= synthetic.entries.len() {
            // No more entries
            return ptr::null_mut();
        }

        let entry = &synthetic.entries[synthetic.position];
        synthetic.position += 1;

        // Fill in the static dirent
        ptr::write_bytes(&mut SYNTHETIC_DIRENT, 0, 1);
        SYNTHETIC_DIRENT.d_ino = (handle + synthetic.position) as libc::ino_t;
        SYNTHETIC_DIRENT.d_type = if entry.is_dir {
            libc::DT_DIR
        } else {
            libc::DT_REG
        };

        // Copy name (truncate if too long)
        let name_bytes = entry.name.as_bytes();
        let copy_len = std::cmp::min(name_bytes.len(), SYNTHETIC_DIRENT.d_name.len() - 1);
        ptr::copy_nonoverlapping(
            name_bytes.as_ptr(),
            SYNTHETIC_DIRENT.d_name.as_mut_ptr() as *mut u8,
            copy_len,
        );
        SYNTHETIC_DIRENT.d_name[copy_len] = 0;

        return &mut SYNTHETIC_DIRENT;
    }
    drop(dirs);

    real_readdir(dir)
}

unsafe fn closedir_impl(dir: *mut libc::DIR, real_closedir: ClosedirFn) -> c_int {
    let _guard = match ShimGuard::enter() {
        Some(g) => g,
        None => return real_closedir(dir),
    };

    let Some(state) = ShimState::get() else {
        return real_closedir(dir);
    };

    let handle = dir as usize;

    // Check if this was a synthetic directory
    let mut dirs = state.open_dirs.lock().unwrap();
    if dirs.remove(&handle).is_some() {
        shim_log("[VRift-Shim] closedir synthetic\n");
        return 0;
    }
    drop(dirs);

    real_closedir(dir)
}

// ============================================================================
// macOS Shims
// ============================================================================

#[cfg(target_os = "macos")]
#[no_mangle]
pub unsafe extern "C" fn opendir_shim(p: *const c_char) -> *mut libc::DIR {
    let real = std::mem::transmute::<*const (), OpendirFn>(IT_OPENDIR.old_func);
    opendir_impl(p, real)
}

#[cfg(target_os = "macos")]
#[no_mangle]
pub unsafe extern "C" fn readdir_shim(d: *mut libc::DIR) -> *mut libc::dirent {
    let real = std::mem::transmute::<*const (), ReaddirFn>(IT_READDIR.old_func);
    readdir_impl(d, real)
}

#[cfg(target_os = "macos")]
#[no_mangle]
pub unsafe extern "C" fn closedir_shim(d: *mut libc::DIR) -> c_int {
    let real = std::mem::transmute::<*const (), ClosedirFn>(IT_CLOSEDIR.old_func);
    closedir_impl(d, real)
}
