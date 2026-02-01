#![allow(unused_imports)]
use libc::c_int;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::ptr;
use std::time::{SystemTime, UNIX_EPOCH};

/// Raw Unix socket connect using libc syscalls (avoids recursion through shim)
pub(crate) unsafe fn raw_unix_connect(path: &str) -> c_int {
    let fd = libc::socket(libc::AF_UNIX, libc::SOCK_STREAM, 0);
    if fd < 0 {
        return -1;
    }
    // RFC-0043: Prevent FD leakage to child processes
    libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC);

    let mut addr: libc::sockaddr_un = std::mem::zeroed();
    addr.sun_family = libc::AF_UNIX as libc::sa_family_t;

    let path_bytes = path.as_bytes();
    if path_bytes.len() >= addr.sun_path.len() {
        libc::close(fd);
        return -1;
    }
    ptr::copy_nonoverlapping(
        path_bytes.as_ptr(),
        addr.sun_path.as_mut_ptr().cast::<u8>(),
        path_bytes.len(),
    );

    let addr_len = std::mem::size_of::<libc::sockaddr_un>() as libc::socklen_t;
    if libc::connect(fd, &addr as *const _ as *const libc::sockaddr, addr_len) < 0 {
        libc::close(fd);
        return -1;
    }

    fd
}

/// Raw write using libc (avoids recursion through shim)
pub(crate) unsafe fn raw_write_all(fd: c_int, data: &[u8]) -> bool {
    let mut written = 0;
    while written < data.len() {
        let n = libc::write(
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

/// Raw read using libc (avoids recursion through shim)
pub(crate) unsafe fn raw_read_exact(fd: c_int, buf: &mut [u8]) -> bool {
    let mut read = 0;
    while read < buf.len() {
        let n = libc::read(
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

/// Send request and receive response using raw libc I/O
unsafe fn sync_rpc(
    socket_path: &str,
    request: &vrift_ipc::VeloRequest,
) -> Option<vrift_ipc::VeloResponse> {
    let fd = raw_unix_connect(socket_path);
    if fd < 0 {
        return None;
    }

    // Serialize request with bincode
    let req_bytes = match bincode::serialize(request) {
        Ok(b) => b,
        Err(_) => {
            libc::close(fd);
            return None;
        }
    };

    // Send length prefix (4 bytes LE) + payload
    let len_bytes = (req_bytes.len() as u32).to_le_bytes();
    if !raw_write_all(fd, &len_bytes) || !raw_write_all(fd, &req_bytes) {
        libc::close(fd);
        return None;
    }

    // Read response length
    let mut resp_len_buf = [0u8; 4];
    if !raw_read_exact(fd, &mut resp_len_buf) {
        libc::close(fd);
        return None;
    }
    let resp_len = u32::from_le_bytes(resp_len_buf) as usize;

    // Read response payload (use heap allocation for variable size)
    let mut resp_buf = vec![0u8; resp_len];
    if !raw_read_exact(fd, &mut resp_buf) {
        libc::close(fd);
        return None;
    }
    libc::close(fd);

    // Deserialize response
    bincode::deserialize(&resp_buf).ok()
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

/// Query manifest for a single path
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
