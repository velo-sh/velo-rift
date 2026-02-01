use crate::interpose::*;
use crate::ipc::sync_ipc_manifest_reingest;
use crate::state::{ShimState, INITIALIZING};
use libc::c_int;
use std::sync::atomic::Ordering;

/// Close implementation with CoW reingest logic.
/// 
/// If this FD was opened for writing in the VFS domain:
/// 1. Close the temp file FD
/// 2. Send reingest request to daemon (temp_path -> vpath)
/// 3. Remove FD from tracking
#[no_mangle]
#[cfg(target_os = "macos")]
pub unsafe extern "C" fn close_shim(fd: c_int) -> c_int {
    let real = std::mem::transmute::<*const (), unsafe extern "C" fn(c_int) -> c_int>(IT_CLOSE.old_func);
    
    // Early-boot passthrough
    if INITIALIZING.load(Ordering::Relaxed) {
        return real(fd);
    }
    
    // Check if this is a tracked CoW FD
    if let Some(state) = ShimState::get() {
        let tracked = {
            if let Ok(mut fds) = state.open_fds.lock() {
                fds.remove(&fd)
            } else {
                None
            }
        };
        
        if let Some(open_file) = tracked {
            // Close the actual FD first
            let result = real(fd);
            
            // Only reingest if close succeeded and there are no active mmaps
            if result == 0 && open_file.mmap_count == 0 {
                // Send reingest request to daemon
                sync_ipc_manifest_reingest(
                    &state.socket_path,
                    &open_file.vpath,
                    &open_file.temp_path,
                );
                
                // Clean up temp file
                let temp_cpath = std::ffi::CString::new(open_file.temp_path.as_str()).ok();
                if let Some(cpath) = temp_cpath {
                    libc::unlink(cpath.as_ptr());
                }
            }
            
            return result;
        }
    }
    
    // Not a tracked FD, passthrough
    real(fd)
}
