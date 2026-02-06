use libc::{c_char, c_int, AT_FDCWD};
use std::ffi::CStr;
use std::ptr;

/// RFC-0049: Unified path resolution for VFS domain.
/// Encapsulates absolute path and the corresponding manifest key.
#[derive(Debug, Clone)]
pub(crate) struct VfsPath {
    pub absolute: String,
    pub manifest_key: String,
    pub manifest_key_hash: u64,
}

pub(crate) struct PathResolver {
    pub vfs_prefix: String,
    pub project_root: String,
}

impl PathResolver {
    pub fn new(vfs_prefix: &str, project_root: &str) -> Self {
        Self {
            vfs_prefix: vfs_prefix.to_string(),
            project_root: project_root.to_string(),
        }
    }

    /// Resolve an incoming path (absolute or relative) into a VfsPath.
    /// Returns None if the path is not within the VFS domain.
    pub fn resolve(&self, path: &str) -> Option<VfsPath> {
        // RFC-0050: Early exit if VFS is not configured
        // Empty vfs_prefix means no virtual filesystem is active
        if self.vfs_prefix.is_empty() {
            return None;
        }

        // 1. Resolve relative paths using project_root
        let abs_path = if !path.starts_with('/') {
            if self.project_root.is_empty() {
                return None;
            }
            format!("{}/{}", self.project_root, path)
        } else {
            path.to_string()
        };

        // 2. Normalize (handle .., ., //)
        let mut buf = vec![0u8; abs_path.len() + 1];
        let len = unsafe { raw_path_normalize(&abs_path, &mut buf)? };
        let normalized = std::str::from_utf8(&buf[..len]).ok()?.to_string();

        // 3. Check VFS applicability
        if !normalized.starts_with(&self.vfs_prefix) {
            return None;
        }

        // Ensure we match on component boundaries (e.g., /vrift matches /vrift/file but not /vriftfile)
        if normalized.len() > self.vfs_prefix.len()
            && !self.vfs_prefix.ends_with('/')
            && normalized.as_bytes()[self.vfs_prefix.len()] != b'/'
        {
            return None;
        }

        // 4. Extract manifest key (relative to project_root, must start with /)
        // If project_root is "/a/b" and normalized is "/a/b/c/d", manifest_key is "/c/d"
        // RFC-0039: vrift ingest always prefixes paths with /
        let manifest_key = if !self.project_root.is_empty()
            && normalized.starts_with(&self.project_root)
            && (normalized.len() == self.project_root.len()
                || self.project_root.ends_with('/')
                || normalized.as_bytes()[self.project_root.len()] == b'/')
        {
            let key = normalized.strip_prefix(&self.project_root).unwrap_or("");
            if !key.starts_with('/') {
                format!("/{}", key)
            } else {
                key.to_string()
            }
        } else {
            // Fallback: if not under project_root but under vfs_prefix
            let key = normalized.strip_prefix(&self.vfs_prefix).unwrap_or("");
            if !key.starts_with('/') {
                format!("/{}", key)
            } else {
                key.to_string()
            }
        };

        let manifest_key_hash = vrift_ipc::fnv1a_hash(&manifest_key);
        Some(VfsPath {
            absolute: normalized,
            manifest_key,
            manifest_key_hash,
        })
    }
}

/// Robust path normalization without heap allocation (low-level).
/// Handles "..", ".", and duplicate slashes.
/// Returns the length of the normalized path in `out`.
pub(crate) unsafe fn raw_path_normalize(path: &str, out: &mut [u8]) -> Option<usize> {
    if path.is_empty() || out.is_empty() {
        return None;
    }

    let bytes = path.as_bytes();
    let mut out_idx = 0;

    // Always start with / if input is absolute
    if bytes[0] == b'/' {
        out[out_idx] = b'/';
        out_idx += 1;
    }

    let mut i = 0;
    while i < bytes.len() {
        // Skip multiple slashes
        while i < bytes.len() && bytes[i] == b'/' {
            i += 1;
        }
        if i == bytes.len() {
            break;
        }

        // Find component end
        let start = i;
        while i < bytes.len() && bytes[i] != b'/' {
            i += 1;
        }
        let component = &bytes[start..i];

        if component == b"." {
            continue;
        } else if component == b".." {
            if out_idx > 1 {
                // Backtrack to previous slash
                out_idx -= 1;
                while out_idx > 1 && out[out_idx - 1] != b'/' {
                    out_idx -= 1;
                }
            } else if out_idx == 1 && out[0] == b'/' {
                // At root, stay at root
                continue;
            }
        } else {
            // Add slash if not at root and last char isn't slash
            if out_idx > 0 && out[out_idx - 1] != b'/' {
                if out_idx < out.len() {
                    out[out_idx] = b'/';
                    out_idx += 1;
                } else {
                    return None;
                }
            }
            // Add component
            if out_idx + component.len() <= out.len() {
                ptr::copy_nonoverlapping(
                    component.as_ptr(),
                    out.as_mut_ptr().add(out_idx),
                    component.len(),
                );
                out_idx += component.len();
            } else {
                return None;
            }
        }
    }

    if out_idx == 0 {
        if bytes[0] == b'/' {
            out[0] = b'/';
        } else {
            out[0] = b'.';
        }
        out_idx = 1;
    }

    Some(out_idx)
}

/// RFC-0049: Generate virtual inode from path
/// Prevents st_ino collision when CAS dedup causes multiple logical files to share same blob
/// Uses a simple hash to generate unique inode per logical path
#[inline]
pub(crate) fn path_to_virtual_ino(path: &str) -> libc::ino_t {
    // Simple FNV-1a hash for O(1) inode generation
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in path.bytes() {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash as libc::ino_t
}

pub(crate) unsafe fn resolve_path_at(
    dirfd: c_int,
    path: *const c_char,
    out: &mut [u8],
) -> Option<usize> {
    let path_str = CStr::from_ptr(path).to_str().ok()?;
    if path_str.starts_with('/') {
        return raw_path_normalize(path_str, out);
    }
    if dirfd == AT_FDCWD {
        // Fallback to basic normalization if no complex resolver is available
        return raw_path_normalize(path_str, out);
    }
    // Cannot resolve relative path to arbitrary dirfd easily without OS help.
    None
}
