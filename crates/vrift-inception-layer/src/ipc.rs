// =============================================================================
// BUG-007b SAFETY: ALL functions in this module MUST use raw syscalls only.
//
// This module is called from within interposed syscall handlers (open, stat, etc.).
// Any libc call here (access, close, fcntl, stat, etc.) will be re-intercepted
// by the shim, creating recursive IPC that floods the daemon socket and deadlocks.
//
// Allowed:     raw_access, raw_close, raw_fcntl, raw_read, raw_write, raw_mmap
// Forbidden:   libc::access, libc::close, libc::fcntl, std::fs::*, std::io::*
// =============================================================================
#![allow(unused_imports)]
use libc::c_int;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::ptr;
use std::time::{SystemTime, UNIX_EPOCH};

/// BUG-007b: Raw close for IPC socket FDs — avoids interposed close_inception
/// which would trigger reingest IPC and recursive socket operations.
#[inline(always)]
unsafe fn ipc_raw_close(fd: c_int) -> c_int {
    #[cfg(target_os = "macos")]
    {
        crate::syscalls::macos_raw::raw_close(fd)
    }
    #[cfg(target_os = "linux")]
    {
        crate::syscalls::linux_raw::raw_close(fd)
    }
}

/// Raw Unix socket connect using libc syscalls (avoids recursion through inception layer)
/// RFC-0053: Adds 5 second timeout to prevent UE process states from blocking IPC
pub(crate) unsafe fn raw_unix_connect(path: &str) -> c_int {
    // Fast-fail: Check if socket file exists before attempting connect
    let path_cstr = match std::ffi::CString::new(path) {
        Ok(p) => p,
        Err(_) => return -1,
    };
    // BUG-007b: Use raw_access, NOT libc::access — access is interposed by the shim
    // and would trigger recursive IPC (open → sync_rpc → access → access_inception → IPC → deadlock)
    #[cfg(target_os = "macos")]
    if crate::syscalls::macos_raw::raw_access(path_cstr.as_ptr(), libc::F_OK) != 0 {
        return -1;
    }
    #[cfg(target_os = "linux")]
    if crate::syscalls::linux_raw::raw_access(path_cstr.as_ptr(), libc::F_OK) != 0 {
        return -1;
    }

    // RFC-0043: Prevent FD leakage to child processes (Atomic CLOEXEC)
    let fd = {
        #[cfg(target_os = "linux")]
        {
            libc::socket(libc::AF_UNIX, libc::SOCK_STREAM | libc::SOCK_CLOEXEC, 0)
        }
        #[cfg(not(target_os = "linux"))]
        {
            let s = libc::socket(libc::AF_UNIX, libc::SOCK_STREAM, 0);
            if s >= 0 {
                // BUG-007b: Use raw_fcntl to avoid interposed fcntl
                #[cfg(target_os = "macos")]
                crate::syscalls::macos_raw::raw_fcntl(s, libc::F_SETFD, libc::FD_CLOEXEC);
                #[cfg(target_os = "linux")]
                libc::fcntl(s, libc::F_SETFD, libc::FD_CLOEXEC); // Linux has no fcntl interpose
            }
            s
        }
    };

    if fd < 0 {
        return -1;
    }

    // RFC-0053: Set socket timeouts BEFORE connect to prevent UE process states
    // 5 second timeout for both send and receive
    let timeout = libc::timeval {
        tv_sec: 5,
        tv_usec: 0,
    };
    libc::setsockopt(
        fd,
        libc::SOL_SOCKET,
        libc::SO_RCVTIMEO,
        &timeout as *const _ as *const libc::c_void,
        std::mem::size_of::<libc::timeval>() as libc::socklen_t,
    );
    libc::setsockopt(
        fd,
        libc::SOL_SOCKET,
        libc::SO_SNDTIMEO,
        &timeout as *const _ as *const libc::c_void,
        std::mem::size_of::<libc::timeval>() as libc::socklen_t,
    );

    let mut addr: libc::sockaddr_un = std::mem::zeroed();
    addr.sun_family = libc::AF_UNIX as libc::sa_family_t;

    let path_bytes = path.as_bytes();
    if path_bytes.len() >= addr.sun_path.len() {
        ipc_raw_close(fd);
        return -1;
    }
    ptr::copy_nonoverlapping(
        path_bytes.as_ptr(),
        addr.sun_path.as_mut_ptr().cast::<u8>(),
        path_bytes.len(),
    );

    let addr_len = std::mem::size_of::<libc::sockaddr_un>() as libc::socklen_t;
    if libc::connect(fd, &addr as *const _ as *const libc::sockaddr, addr_len) < 0 {
        ipc_raw_close(fd);
        return -1;
    }

    fd
}

/// Raw write using libc (avoids recursion through inception layer)
pub(crate) unsafe fn raw_write_all(fd: c_int, data: &[u8]) -> bool {
    let mut written = 0;
    while written < data.len() {
        #[cfg(target_os = "macos")]
        let n = crate::syscalls::macos_raw::raw_write(
            fd,
            data[written..].as_ptr() as *const libc::c_void,
            data.len() - written,
        );
        #[cfg(target_os = "linux")]
        let n = crate::syscalls::linux_raw::raw_write(
            fd,
            data[written..].as_ptr() as *const libc::c_void,
            data.len() - written,
        );
        if n <= 0 {
            return false;
        }
        written += n as usize;
    }
    true
}

/// Raw read using syscall (avoids recursion through inception layer)
pub(crate) unsafe fn raw_read_exact(fd: c_int, buf: &mut [u8]) -> bool {
    let mut read = 0;
    while read < buf.len() {
        #[cfg(target_os = "macos")]
        let n = crate::syscalls::macos_raw::raw_read(
            fd,
            buf[read..].as_mut_ptr() as *mut libc::c_void,
            buf.len() - read,
        );
        #[cfg(target_os = "linux")]
        let n = crate::syscalls::linux_raw::raw_read(
            fd,
            buf[read..].as_mut_ptr() as *mut libc::c_void,
            buf.len() - read,
        );
        if n <= 0 {
            return false;
        }
        read += n as usize;
    }
    true
}

/// Send request and receive response using raw libc I/O.
/// RFC-0043: Ensuring workspace registration for every connection.
/// RFC-0055: Auto-recovery after CIRCUIT_RECOVERY_DELAY seconds.
unsafe fn sync_rpc(
    socket_path: &str,
    request: &vrift_ipc::VeloRequest,
) -> Option<vrift_ipc::VeloResponse> {
    use crate::state::{
        EventType, CIRCUIT_BREAKER_FAILED_COUNT, CIRCUIT_BREAKER_THRESHOLD, CIRCUIT_RECOVERY_DELAY,
        CIRCUIT_TRIPPED, CIRCUIT_TRIP_TIME,
    };
    use std::sync::atomic::Ordering;

    // Check circuit breaker with auto-recovery
    if CIRCUIT_TRIPPED.load(Ordering::Relaxed) {
        let trip_time = CIRCUIT_TRIP_TIME.load(Ordering::Relaxed);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let recovery_delay = CIRCUIT_RECOVERY_DELAY.load(Ordering::Relaxed);

        if now >= trip_time + recovery_delay {
            // Recovery window: try to reset circuit breaker
            inception_info!(
                "Circuit breaker recovery attempt after {}s",
                now - trip_time
            );
            CIRCUIT_TRIPPED.store(false, Ordering::SeqCst);
            CIRCUIT_BREAKER_FAILED_COUNT.store(0, Ordering::Relaxed);
        } else {
            return None;
        }
    }

    let fd = raw_unix_connect(socket_path);
    if fd < 0 {
        let count = CIRCUIT_BREAKER_FAILED_COUNT.fetch_add(1, Ordering::SeqCst) + 1;
        let threshold = CIRCUIT_BREAKER_THRESHOLD.load(Ordering::Relaxed);
        inception_record!(EventType::IpcFail, 0, count as i32);
        if count >= threshold && !CIRCUIT_TRIPPED.swap(true, Ordering::SeqCst) {
            // Record trip time for auto-recovery
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            CIRCUIT_TRIP_TIME.store(now, Ordering::Relaxed);
            inception_error!(
                "DAEMON CONNECTION FAILED {} TIMES. CIRCUIT BREAKER TRIPPED. WILL RETRY AFTER {}s.",
                count,
                CIRCUIT_RECOVERY_DELAY.load(Ordering::Relaxed)
            );
            inception_record!(EventType::CircuitTripped, 0, count as i32);
        }
        return None;
    }

    inception_record!(EventType::IpcSuccess, 0, fd);

    // Success - reset failure count
    CIRCUIT_BREAKER_FAILED_COUNT.store(0, Ordering::Relaxed);

    // RFC-0043: Registration ensures the daemon knows which project manifest to query.
    let project_root = {
        let env_ptr = unsafe { libc::getenv(c"VRIFT_PROJECT_ROOT".as_ptr()) };
        if !env_ptr.is_null() {
            unsafe { std::ffi::CStr::from_ptr(env_ptr) }
                .to_string_lossy()
                .to_string()
        } else {
            let manifest_ptr = unsafe { libc::getenv(c"VRIFT_MANIFEST".as_ptr()) };
            if !manifest_ptr.is_null() {
                let manifest = unsafe { std::ffi::CStr::from_ptr(manifest_ptr).to_string_lossy() };
                let p = std::path::Path::new(manifest.as_ref());
                let parent = p.parent().unwrap_or(p);
                let root = if parent.ends_with(".vrift") {
                    parent.parent().unwrap_or(parent)
                } else {
                    parent
                };
                // BUG-007b: DO NOT call canonicalize() here — it triggers
                // interposed stat/readlink calls, causing recursive IPC deadlock
                // under concurrent workloads. The path from env is already usable.
                root.to_string_lossy().to_string()
            } else {
                String::new()
            }
        }
    };

    // BUG-007b: DO NOT call std::fs::canonicalize() here.
    // canonicalize() calls stat/readlink internally, which are interposed by the shim.
    // When called from within a shim entry point (open -> query_manifest_ipc -> sync_rpc),
    // this creates recursive IPC that floods the daemon socket under concurrent access.
    // project_root from env vars is already an absolute path.

    if !project_root.is_empty() {
        let register_req = vrift_ipc::VeloRequest::RegisterWorkspace { project_root };
        if send_request_on_fd(fd, &register_req) {
            let _ = recv_response_on_fd(fd);
        }
    }

    // Send original request
    if !send_request_on_fd(fd, request) {
        ipc_raw_close(fd);
        return None;
    }

    let response = recv_response_on_fd(fd);
    ipc_raw_close(fd);
    response
}

pub(crate) unsafe fn sync_ipc_manifest_remove(socket_path: &str, path: &str) -> bool {
    let request = vrift_ipc::VeloRequest::ManifestRemove {
        path: path.to_string(),
    };
    matches!(
        sync_rpc(socket_path, &request),
        Some(vrift_ipc::VeloResponse::ManifestAck { .. })
    )
}

pub(crate) unsafe fn sync_ipc_manifest_rename(socket_path: &str, old: &str, new: &str) -> bool {
    let request = vrift_ipc::VeloRequest::ManifestRename {
        old_path: old.to_string(),
        new_path: new.to_string(),
    };
    matches!(
        sync_rpc(socket_path, &request),
        Some(vrift_ipc::VeloResponse::ManifestAck { .. })
    )
}

pub(crate) unsafe fn sync_ipc_manifest_update_mtime(
    socket_path: &str,
    path: &str,
    mtime: u64,
) -> bool {
    let request = vrift_ipc::VeloRequest::ManifestUpdateMtime {
        path: path.to_string(),
        mtime_ns: mtime,
    };
    matches!(
        sync_rpc(socket_path, &request),
        Some(vrift_ipc::VeloResponse::ManifestAck { .. })
    )
}

pub(crate) unsafe fn sync_ipc_manifest_mkdir(socket_path: &str, path: &str, _mode: u32) -> bool {
    // Create a directory entry in the manifest
    let request = vrift_ipc::VeloRequest::ManifestUpsert {
        path: path.to_string(),
        entry: vrift_ipc::VnodeEntry {
            content_hash: [0u8; 32],
            size: 0,
            mtime: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
            mode: 0o755,
            flags: 1, // is_dir flag
            _pad: 0,
        },
    };
    matches!(
        sync_rpc(socket_path, &request),
        Some(vrift_ipc::VeloResponse::ManifestAck { .. })
    )
}

pub(crate) unsafe fn sync_ipc_manifest_symlink(
    socket_path: &str,
    path: &str,
    _target: &str,
) -> bool {
    // Symlinks stored as special manifest entries
    let request = vrift_ipc::VeloRequest::ManifestUpsert {
        path: path.to_string(),
        entry: vrift_ipc::VnodeEntry {
            content_hash: [0u8; 32],
            size: 0,
            mtime: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
            mode: 0o777,
            flags: 2, // is_symlink pseudo-flag
            _pad: 0,
        },
    };
    matches!(
        sync_rpc(socket_path, &request),
        Some(vrift_ipc::VeloResponse::ManifestAck { .. })
    )
}

pub(crate) unsafe fn sync_ipc_manifest_reingest(
    socket_path: &str,
    vpath: &str,
    temp: &str,
) -> bool {
    let request = vrift_ipc::VeloRequest::ManifestReingest {
        vpath: vpath.to_string(),
        temp_path: temp.to_string(),
    };
    matches!(
        sync_rpc(socket_path, &request),
        Some(vrift_ipc::VeloResponse::ManifestAck { .. })
    )
}

pub(crate) unsafe fn sync_ipc_flock(socket_path: &str, path: &str, op: i32) -> bool {
    let request = if op & libc::LOCK_UN != 0 {
        vrift_ipc::VeloRequest::FlockRelease {
            path: path.to_string(),
        }
    } else {
        vrift_ipc::VeloRequest::FlockAcquire {
            path: path.to_string(),
            operation: op,
        }
    };
    matches!(
        sync_rpc(socket_path, &request),
        Some(vrift_ipc::VeloResponse::FlockAck)
    )
}

pub(crate) unsafe fn sync_ipc_fcntl_lock(
    _socket_path: &str,
    _path: &str,
    _cmd: i32,
    _lock: *mut libc::flock,
) -> bool {
    // fcntl F_SETLK/F_GETLK - complex locking, defer to passthrough for now
    false
}

/// Query manifest for a single path (with workspace registration)
/// Daemon requires RegisterWorkspace before ManifestGet on same connection
pub(crate) unsafe fn sync_ipc_manifest_get(
    socket_path: &str,
    path: &str,
) -> Option<vrift_ipc::VnodeEntry> {
    let request = vrift_ipc::VeloRequest::ManifestGet {
        path: path.to_string(),
    };
    match sync_rpc(socket_path, &request) {
        Some(vrift_ipc::VeloResponse::ManifestAck { entry }) => entry,
        _ => None,
    }
}

// Helper: send request on existing FD (v3 frame protocol)
unsafe fn send_request_on_fd(fd: libc::c_int, request: &vrift_ipc::VeloRequest) -> bool {
    use vrift_ipc::{next_seq_id, IpcHeader};

    let payload = match rkyv::to_bytes::<rkyv::rancor::Error>(request) {
        Ok(b) => b,
        Err(_) => return false,
    };

    if payload.len() > vrift_ipc::IpcHeader::MAX_LENGTH {
        return false;
    }

    let seq_id = next_seq_id();
    let header = IpcHeader::new_request(payload.len() as u32, seq_id);

    raw_write_all(fd, &header.to_bytes()) && raw_write_all(fd, &payload)
}

// Helper: receive response on existing FD (v3 frame protocol)
unsafe fn recv_response_on_fd(fd: libc::c_int) -> Option<vrift_ipc::VeloResponse> {
    use vrift_ipc::IpcHeader;

    // Read header
    let mut header_buf = [0u8; IpcHeader::SIZE];
    if !raw_read_exact(fd, &mut header_buf) {
        return None;
    }

    let header = IpcHeader::from_bytes(&header_buf);
    if !header.is_valid() {
        return None;
    }

    // Sanity check
    if header.length as usize > 1024 * 1024 {
        return None;
    }

    // Read payload
    let mut payload = vec![0u8; header.length as usize];
    if !raw_read_exact(fd, &mut payload) {
        return None;
    }

    rkyv::from_bytes::<vrift_ipc::VeloResponse, rkyv::rancor::Error>(&payload).ok()
}

/// Query directory listing from daemon
pub(crate) unsafe fn sync_ipc_manifest_list_dir(
    socket_path: &str,
    path: &str,
) -> Option<Vec<vrift_ipc::DirEntry>> {
    let request = vrift_ipc::VeloRequest::ManifestListDir {
        path: path.to_string(),
    };
    match sync_rpc(socket_path, &request) {
        Some(vrift_ipc::VeloResponse::ManifestListAck { entries }) => Some(entries),
        _ => None,
    }
}
