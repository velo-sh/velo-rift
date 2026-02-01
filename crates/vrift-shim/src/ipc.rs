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
        addr.sun_path.as_mut_ptr() as *mut u8,
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

/// Sync IPC to daemon for manifest removal (for unlink/rmdir)
/// Returns true on success, false on failure (caller should fallback)
pub(crate) unsafe fn sync_ipc_manifest_remove(socket_path: &str, path: &str) -> bool {
    let stream = match UnixStream::connect(socket_path) {
        Ok(s) => s,
        Err(_) => return false,
    };

    let request = vrift_ipc::VeloRequest::ManifestRemove {
        path: path.to_string(),
    };

    let req_bytes = match bincode::serialize(&request) {
        Ok(b) => b,
        Err(_) => return false,
    };

    let mut stream = stream;
    let len_bytes = (req_bytes.len() as u32).to_le_bytes();
    if stream.write_all(&len_bytes).is_err() {
        return false;
    }
    if stream.write_all(&req_bytes).is_err() {
        return false;
    }

    // Read response
    let mut len_buf = [0u8; 4];
    if stream.read_exact(&mut len_buf).is_err() {
        return false;
    }
    let resp_len = u32::from_le_bytes(len_buf) as usize;

    let mut resp_buf = vec![0u8; resp_len];
    if stream.read_exact(&mut resp_buf).is_err() {
        return false;
    }

    matches!(
        bincode::deserialize::<vrift_ipc::VeloResponse>(&resp_buf),
        Ok(vrift_ipc::VeloResponse::ManifestAck { .. })
    )
}

/// Sync IPC to daemon for manifest rename
pub(crate) unsafe fn sync_ipc_manifest_rename(
    socket_path: &str,
    old_path: &str,
    new_path: &str,
) -> bool {
    let stream = match UnixStream::connect(socket_path) {
        Ok(s) => s,
        Err(_) => return false,
    };

    let request = vrift_ipc::VeloRequest::ManifestRename {
        old_path: old_path.to_string(),
        new_path: new_path.to_string(),
    };

    let req_bytes = match bincode::serialize(&request) {
        Ok(b) => b,
        Err(_) => return false,
    };

    let mut stream = stream;
    let len_bytes = (req_bytes.len() as u32).to_le_bytes();
    if stream.write_all(&len_bytes).is_err() {
        return false;
    }
    if stream.write_all(&req_bytes).is_err() {
        return false;
    }

    // Read response
    let mut len_buf = [0u8; 4];
    if stream.read_exact(&mut len_buf).is_err() {
        return false;
    }
    let resp_len = u32::from_le_bytes(len_buf) as usize;

    let mut resp_buf = vec![0u8; resp_len];
    if stream.read_exact(&mut resp_buf).is_err() {
        return false;
    }

    matches!(
        bincode::deserialize::<vrift_ipc::VeloResponse>(&resp_buf),
        Ok(vrift_ipc::VeloResponse::ManifestAck { .. })
    )
}

/// Sync IPC to daemon for manifest mtime update (for utimes/utimensat)
pub(crate) unsafe fn sync_ipc_manifest_update_mtime(
    socket_path: &str,
    path: &str,
    mtime_ns: u64,
) -> bool {
    let stream = match UnixStream::connect(socket_path) {
        Ok(s) => s,
        Err(_) => return false,
    };

    let request = vrift_ipc::VeloRequest::ManifestUpdateMtime {
        path: path.to_string(),
        mtime_ns,
    };

    let req_bytes = match bincode::serialize(&request) {
        Ok(b) => b,
        Err(_) => return false,
    };

    let mut stream = stream;
    let len_bytes = (req_bytes.len() as u32).to_le_bytes();
    if stream.write_all(&len_bytes).is_err() {
        return false;
    }
    if stream.write_all(&req_bytes).is_err() {
        return false;
    }

    // Read response
    let mut len_buf = [0u8; 4];
    if stream.read_exact(&mut len_buf).is_err() {
        return false;
    }
    let resp_len = u32::from_le_bytes(len_buf) as usize;

    let mut resp_buf = vec![0u8; resp_len];
    if stream.read_exact(&mut resp_buf).is_err() {
        return false;
    }

    matches!(
        bincode::deserialize::<vrift_ipc::VeloResponse>(&resp_buf),
        Ok(vrift_ipc::VeloResponse::ManifestAck { .. })
    )
}

/// Sync IPC to daemon for mkdir (creates dir entry in Manifest)
pub(crate) unsafe fn sync_ipc_manifest_mkdir(socket_path: &str, path: &str, mode: u32) -> bool {
    let stream = match UnixStream::connect(socket_path) {
        Ok(s) => s,
        Err(_) => return false,
    };

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);

    // Create a directory VnodeEntry using constructor
    let entry = vrift_manifest::VnodeEntry::new_directory(now, mode);

    let request = vrift_ipc::VeloRequest::ManifestUpsert {
        path: path.to_string(),
        entry,
    };

    let req_bytes = match bincode::serialize(&request) {
        Ok(b) => b,
        Err(_) => return false,
    };

    let mut stream = stream;
    let len_bytes = (req_bytes.len() as u32).to_le_bytes();
    if stream.write_all(&len_bytes).is_err() {
        return false;
    }
    if stream.write_all(&req_bytes).is_err() {
        return false;
    }

    let mut len_buf = [0u8; 4];
    if stream.read_exact(&mut len_buf).is_err() {
        return false;
    }
    let resp_len = u32::from_le_bytes(len_buf) as usize;

    let mut resp_buf = vec![0u8; resp_len];
    if stream.read_exact(&mut resp_buf).is_err() {
        return false;
    }

    matches!(
        bincode::deserialize::<vrift_ipc::VeloResponse>(&resp_buf),
        Ok(vrift_ipc::VeloResponse::ManifestAck { .. })
    )
}

/// Sync IPC to daemon for symlink (creates symlink entry in Manifest)
pub(crate) unsafe fn sync_ipc_manifest_symlink(
    socket_path: &str,
    link_path: &str,
    _target: &str,
) -> bool {
    let stream = match UnixStream::connect(socket_path) {
        Ok(s) => s,
        Err(_) => return false,
    };

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);

    // Create a symlink VnodeEntry using constructor
    // Note: target_hash should store hash of target string, using zeros for now
    let entry = vrift_manifest::VnodeEntry::new_symlink([0u8; 32], 0, now);

    let request = vrift_ipc::VeloRequest::ManifestUpsert {
        path: link_path.to_string(),
        entry,
    };

    let req_bytes = match bincode::serialize(&request) {
        Ok(b) => b,
        Err(_) => return false,
    };

    let mut stream = stream;
    let len_bytes = (req_bytes.len() as u32).to_le_bytes();
    if stream.write_all(&len_bytes).is_err() {
        return false;
    }
    if stream.write_all(&req_bytes).is_err() {
        return false;
    }

    let mut len_buf = [0u8; 4];
    if stream.read_exact(&mut len_buf).is_err() {
        return false;
    }
    let resp_len = u32::from_le_bytes(len_buf) as usize;

    let mut resp_buf = vec![0u8; resp_len];
    if stream.read_exact(&mut resp_buf).is_err() {
        return false;
    }

    matches!(
        bincode::deserialize::<vrift_ipc::VeloResponse>(&resp_buf),
        Ok(vrift_ipc::VeloResponse::ManifestAck { .. })
    )
}

/// Sync IPC to daemon for CoW reingest (close of dirty FD)
/// Daemon will read temp_path, hash it, insert to CAS, update Manifest for vpath
pub(crate) unsafe fn sync_ipc_manifest_reingest(
    socket_path: &str,
    vpath: &str,
    temp_path: &str,
) -> bool {
    let stream = match UnixStream::connect(socket_path) {
        Ok(s) => s,
        Err(_) => return false,
    };

    let request = vrift_ipc::VeloRequest::ManifestReingest {
        vpath: vpath.to_string(),
        temp_path: temp_path.to_string(),
    };

    let req_bytes = match bincode::serialize(&request) {
        Ok(b) => b,
        Err(_) => return false,
    };

    let mut stream = stream;
    let len_bytes = (req_bytes.len() as u32).to_le_bytes();
    if stream.write_all(&len_bytes).is_err() {
        return false;
    }
    if stream.write_all(&req_bytes).is_err() {
        return false;
    }

    let mut len_buf = [0u8; 4];
    if stream.read_exact(&mut len_buf).is_err() {
        return false;
    }
    let resp_len = u32::from_le_bytes(len_buf) as usize;

    let mut resp_buf = vec![0u8; resp_len];
    if stream.read_exact(&mut resp_buf).is_err() {
        return false;
    }

    matches!(
        bincode::deserialize::<vrift_ipc::VeloResponse>(&resp_buf),
        Ok(vrift_ipc::VeloResponse::ManifestAck { .. })
    )
}

/// RFC-0049: Sync IPC for advisory locking (flock serialization)
pub(crate) unsafe fn sync_ipc_flock(socket_path: &str, path: &str, op: i32) -> Result<(), i32> {
    let stream = match UnixStream::connect(socket_path) {
        Ok(s) => s,
        Err(_) => return Err(libc::ECONNREFUSED),
    };

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

    let req_bytes = match bincode::serialize(&request) {
        Ok(b) => b,
        Err(_) => return Err(libc::EIO),
    };

    let mut stream = stream;
    let len_bytes = (req_bytes.len() as u32).to_le_bytes();
    if stream.write_all(&len_bytes).is_err() {
        return Err(libc::EPIPE);
    }
    if stream.write_all(&req_bytes).is_err() {
        return Err(libc::EPIPE);
    }

    let mut len_buf = [0u8; 4];
    if stream.read_exact(&mut len_buf).is_err() {
        return Err(libc::EPIPE);
    }
    let resp_len = u32::from_le_bytes(len_buf) as usize;

    let mut resp_buf = vec![0u8; resp_len];
    if stream.read_exact(&mut resp_buf).is_err() {
        return Err(libc::EPIPE);
    }

    match bincode::deserialize::<vrift_ipc::VeloResponse>(&resp_buf) {
        Ok(vrift_ipc::VeloResponse::FlockAck) => Ok(()),
        Ok(vrift_ipc::VeloResponse::Error(msg)) => {
            if msg.contains("WOULDBLOCK") {
                Err(libc::EWOULDBLOCK)
            } else {
                Err(libc::EIO)
            }
        }
        _ => Err(libc::EIO),
    }
}
