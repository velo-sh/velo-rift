// =============================================================================
// BUG-007b SAFETY: ALL functions in this module MUST use raw syscalls only.
//
// This module is called from within interposed syscall handlers (open, stat, etc.).
// Any libc call here (access, close, fcntl, stat, etc.) will be re-intercepted
// by the shim, creating recursive IPC that floods the daemon socket and deadlocks.
//
// ENFORCEMENT: All I/O goes through `RawContext` which only exposes raw syscalls.
// Allowed:     ctx.access(), ctx.close(), ctx.fcntl(), ctx.read(), ctx.write()
// Forbidden:   libc::access, libc::close, libc::fcntl, std::fs::*, std::io::*
// =============================================================================
use crate::raw_context::RawContext;
use libc::c_int;
use std::ptr;
use std::time::{SystemTime, UNIX_EPOCH};

/// The singleton RawContext for IPC operations.
/// All raw syscall access must go through this instance.
const CTX: &RawContext = &RawContext::INSTANCE;

/// BUG-007b: Raw close for IPC socket FDs — avoids interposed close_inception
/// which would trigger reingest IPC and recursive socket operations.
#[inline(always)]
unsafe fn ipc_raw_close(fd: c_int) -> c_int {
    CTX.close(fd)
}

/// Raw Unix socket connect using raw syscalls (avoids recursion through inception layer)
/// RFC-0053: Adds 5 second timeout to prevent UE process states from blocking IPC
pub(crate) unsafe fn raw_unix_connect(path: &str) -> c_int {
    // Fast-fail: Check if socket file exists before attempting connect
    let path_cstr = match std::ffi::CString::new(path) {
        Ok(p) => p,
        Err(_) => return -1,
    };
    // BUG-007b: Use raw_access via RawContext — access is interposed by the shim
    if CTX.access(path_cstr.as_ptr(), libc::F_OK) != 0 {
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
                // BUG-007b: Use raw_fcntl via RawContext to avoid interposed fcntl
                CTX.fcntl(s, libc::F_SETFD, libc::FD_CLOEXEC);
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

/// Raw write using RawContext (avoids recursion through inception layer)
pub(crate) unsafe fn raw_write_all(fd: c_int, data: &[u8]) -> bool {
    CTX.write_all(fd, data)
}

/// Raw read using RawContext (avoids recursion through inception layer)
pub(crate) unsafe fn raw_read_exact(fd: c_int, buf: &mut [u8]) -> bool {
    CTX.read_exact(fd, buf)
}

/// Send request and receive response using raw I/O via RawContext.
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
    let project_root = get_project_root();

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

/// Phase 3: Fire-and-forget IPC — push a VeloRequest to the ring buffer
/// for background processing by the worker thread. This avoids blocking
/// the hot-path interposed syscall while the daemon processes the request.
///
/// The request is serialized upfront (small cost on caller thread) and
/// the worker handles connection + send. If the ring buffer is full,
/// falls back to synchronous IPC to avoid data loss.
///
/// Returns true if the request was successfully queued or sent synchronously.
pub(crate) unsafe fn fire_and_forget_ipc(
    socket_path: &str,
    request: &vrift_ipc::VeloRequest,
) -> bool {
    // Serialize upfront so the worker only needs to connect + write
    let payload = match rkyv::to_bytes::<rkyv::rancor::Error>(request) {
        Ok(bytes) => bytes.to_vec(),
        Err(_) => return false,
    };

    // Try to push to ring buffer for async processing
    if let Some(reactor) = crate::sync::get_reactor() {
        let task = crate::sync::Task::IpcFireAndForget {
            socket_path: socket_path.to_string(),
            payload,
        };
        match reactor.ring_buffer.push(task) {
            Ok(()) => return true,
            Err(crate::sync::Task::IpcFireAndForget {
                socket_path: sp,
                payload: pl,
            }) => {
                // Ring buffer full — fall back to synchronous send
                inception_warn!("Ring buffer full, falling back to sync IPC");
                return send_fire_and_forget_sync(&sp, &pl);
            }
            Err(_) => return false, // unreachable, but satisfy exhaustive match
        }
    }

    // No reactor available — send synchronously
    send_fire_and_forget_sync(socket_path, &payload)
}

/// Synchronous fallback for fire-and-forget: connect, register, send, close.
/// Does not read the response — the daemon processes the request asynchronously.
pub(crate) unsafe fn send_fire_and_forget_sync(socket_path: &str, payload: &[u8]) -> bool {
    let fd = raw_unix_connect(socket_path);
    if fd < 0 {
        return false;
    }

    // Workspace registration (same as sync_rpc)
    let project_root = get_project_root();
    if !project_root.is_empty() {
        let register_req = vrift_ipc::VeloRequest::RegisterWorkspace { project_root };
        if send_request_on_fd(fd, &register_req) {
            let _ = recv_response_on_fd(fd);
        }
    }

    // Send the pre-serialized request
    let seq_id = vrift_ipc::next_seq_id();
    let header = vrift_ipc::IpcHeader::new_request(payload.len() as u32, seq_id);
    let success = raw_write_all(fd, &header.to_bytes()) && raw_write_all(fd, payload);
    ipc_raw_close(fd);
    success
}

/// Extract project root from env vars (shared between sync_rpc and fire-and-forget).
/// RFC-0044: Use raw_realpath to avoid Project ID Divergence (e.g. /var vs /private/var)
fn get_project_root() -> String {
    let raw_root = {
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
                root.to_string_lossy().to_string()
            } else {
                return String::new();
            }
        }
    };

    if raw_root.is_empty() {
        return String::new();
    }

    // Canonicalize using raw_realpath to avoid recursion (Pattern 2682.v2)
    let root_cstr = std::ffi::CString::new(raw_root.clone()).unwrap_or_default();
    let mut resolved = [0u8; libc::PATH_MAX as usize];
    #[cfg(target_os = "macos")]
    let result = unsafe {
        crate::syscalls::macos_raw::raw_realpath(
            root_cstr.as_ptr(),
            resolved.as_mut_ptr() as *mut libc::c_char,
        )
    };
    #[cfg(target_os = "linux")]
    let result = unsafe {
        libc::realpath(
            root_cstr.as_ptr(),
            resolved.as_mut_ptr() as *mut libc::c_char,
        )
    };

    if !result.is_null() {
        unsafe { std::ffi::CStr::from_ptr(result) }
            .to_string_lossy()
            .to_string()
    } else {
        raw_root
    }
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
