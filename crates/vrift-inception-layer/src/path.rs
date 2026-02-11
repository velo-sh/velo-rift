use libc::{c_char, c_int, AT_FDCWD};
use std::ffi::CStr;
use std::ptr;

use crate::state::FixedString;

/// RFC-0049: Unified path resolution for VFS domain.
/// Encapsulates absolute path and the corresponding manifest key.
#[derive(Debug, Clone)]
pub(crate) struct VfsPath {
    pub absolute: FixedString<1024>,
    pub manifest_key: FixedString<1024>,
    pub manifest_key_hash: u64,
}

pub(crate) struct PathResolver {
    pub vfs_prefix: FixedString<256>,
    pub project_root: FixedString<1024>,
}

impl PathResolver {
    pub fn new(vfs_prefix: &str, project_root: &str) -> Self {
        let mut prefix = FixedString::new();
        prefix.set(vfs_prefix);
        let mut root = FixedString::new();
        root.set(project_root);
        Self {
            vfs_prefix: prefix,
            project_root: root,
        }
    }

    /// Resolve an incoming path (absolute or relative) into a VfsPath.
    /// Returns None if the path is not within the VFS domain.
    pub fn resolve(&self, path: &str) -> Option<VfsPath> {
        // RFC-0050: Early exit if VFS is not configured
        if self.vfs_prefix.is_empty() {
            return None;
        }

        let mut abs_buf = [0u8; 1024];
        let mut abs_writer = crate::macros::StackWriter::new(&mut abs_buf);
        use std::fmt::Write;

        // 1. Resolve relative paths using real process cwd (POSIX semantics).
        //    Previous bug: used project_root, which broke build-scripts that chdir
        //    to external directories (e.g. ~/.cargo/registry/.../zerocopy-0.8.31/)
        //    and read relative "Cargo.toml" — that incorrectly resolved to
        //    {project_root}/Cargo.toml instead of the crate's own Cargo.toml.
        if !path.starts_with('/') {
            let mut found = false;
            crate::state::CACHED_CWD.with(|cache| {
                if let Some(ref fs) = *cache.borrow() {
                    let _ = write!(abs_writer, "{}/{}", fs.as_str(), path);
                    found = true;
                }
            });

            if !found {
                let mut cwd_buf = [0u8; 1024];
                #[cfg(target_os = "macos")]
                let cwd_ptr = unsafe {
                    crate::syscalls::macos_raw::raw_getcwd(
                        cwd_buf.as_mut_ptr() as *mut libc::c_char,
                        cwd_buf.len(),
                    )
                };
                #[cfg(target_os = "linux")]
                let cwd_ptr = unsafe {
                    crate::syscalls::linux_raw::raw_getcwd(
                        cwd_buf.as_mut_ptr() as *mut libc::c_char,
                        cwd_buf.len(),
                    )
                };
                if cwd_ptr.is_null() {
                    return None;
                }
                let cwd = unsafe { CStr::from_ptr(cwd_buf.as_ptr() as *const libc::c_char) };
                let cwd_str = cwd.to_str().ok()?;
                let _ = write!(abs_writer, "{}/{}", cwd_str, path);

                // Update cache for next time
                let mut fs = crate::state::FixedString::new();
                fs.set(cwd_str);
                crate::state::CACHED_CWD.with(|cache| {
                    *cache.borrow_mut() = Some(fs);
                });
            }
        } else {
            let _ = write!(abs_writer, "{}", path);
        };
        let abs_path = abs_writer.as_str();

        // 2. Normalize (handle .., ., //)
        let mut norm_buf = [0u8; 1024];
        let len = unsafe { raw_path_normalize(abs_path, &mut norm_buf)? };
        let normalized = std::str::from_utf8(&norm_buf[..len]).ok()?;

        // 3. Check VFS applicability
        let prefix = self.vfs_prefix.as_str();
        #[allow(unused_mut)]
        let mut applicable = normalized.starts_with(prefix);

        // RFC-0050: Handle macOS /tmp symlink invisibility
        #[cfg(target_os = "macos")]
        if !applicable && normalized.starts_with("/tmp/") {
            let mut alt_buf = [0u8; 1024];
            let mut aw = crate::macros::StackWriter::new(&mut alt_buf);
            let _ = write!(aw, "/private{}", normalized);
            applicable = aw.as_str().starts_with(prefix);
        }

        // Drop-in build cache: also match project_root-prefixed paths.
        // Virtual prefix (e.g. /vrift) is for read path; real paths under project root
        // are for write-back capture (cargo/rustc write to real paths like
        // /Users/.../velo/target/debug/..., which must be tracked for close-reingest).
        if !applicable && !self.project_root.is_empty() {
            let proj_root_str = self.project_root.as_str();
            if normalized.starts_with(proj_root_str) {
                let boundary_char = normalized.as_bytes().get(proj_root_str.len()).copied();
                // Match if exact or if followed by component separator
                if boundary_char.is_none() || boundary_char == Some(b'/') {
                    applicable = true;
                }
            }
        }

        if !applicable {
            return None;
        }

        // Ensure we match on component boundaries
        let prefix_len = self.vfs_prefix.len;
        if normalized.len() > prefix_len
            && !self.vfs_prefix.as_str().ends_with('/')
            && normalized.as_bytes()[prefix_len] != b'/'
        {
            return None;
        }

        // 4. Extract manifest key
        let mut key_fs = FixedString::<1024>::new();
        let proj_root_str = self.project_root.as_str();

        #[allow(unused_mut)]
        let mut normalized_for_strip = normalized;
        #[cfg(target_os = "macos")]
        if !normalized.starts_with(proj_root_str) && normalized.starts_with("/tmp/") {
            // Try the /private variant for stripping
            let mut alt_buf = [0u8; 1024];
            let mut aw = crate::macros::StackWriter::new(&mut alt_buf);
            let _ = write!(aw, "/private{}", normalized);
            // We need a way to use aw.as_str() longer than the let binding.
            // Actually, since we only use it for stripping prefix, we can do it here.
            let alt_normalized = aw.as_str();
            if alt_normalized.starts_with(proj_root_str) {
                let key = alt_normalized.strip_prefix(proj_root_str).unwrap_or("");
                if !key.starts_with('/') {
                    let mut key_buf = [0u8; 1024];
                    let mut kw = crate::macros::StackWriter::new(&mut key_buf);
                    let _ = write!(kw, "/{}", key);
                    key_fs.set(kw.as_str());
                } else {
                    key_fs.set(key);
                }
                // Set flag to skip normal stripping
                normalized_for_strip = "";
            }
        }

        if !normalized_for_strip.is_empty()
            && !self.project_root.is_empty()
            && normalized_for_strip.starts_with(proj_root_str)
        {
            let boundary_char = normalized_for_strip
                .as_bytes()
                .get(proj_root_str.len())
                .copied();
            if boundary_char.is_none() || boundary_char == Some(b'/') {
                let key = normalized_for_strip
                    .strip_prefix(proj_root_str)
                    .unwrap_or("");
                if !key.starts_with('/') {
                    let mut key_buf = [0u8; 1024];
                    let mut kw = crate::macros::StackWriter::new(&mut key_buf);
                    let _ = write!(kw, "/{}", key);
                    key_fs.set(kw.as_str());
                } else {
                    key_fs.set(key);
                }
            }
        } else {
            // Check if normalized matches the prefix.
            // If the prefix is a virtual namespace (like /myvirt), and we ARE that path,
            // we should probably use the full path as the key if it matches what's in the manifest.
            // RFC-0050: In most cases (ingest --prefix), the full path IS the key.
            // If the prefix is just a local mount point, we strip it.
            // Strategy: if vfs_prefix starts with / and looks like a virtual path,
            // we use the full normalized path as the key.
            let prefix_str = self.vfs_prefix.as_str();
            if prefix_str.starts_with('/')
                && (self.project_root.is_empty() || !prefix_str.starts_with(proj_root_str))
            {
                // Virtual prefix (e.g. /myvirt) - keep it in the key
                key_fs.set(normalized);
            } else {
                // Physical prefix (e.g. project root) - strip it
                let key = normalized.strip_prefix(prefix_str).unwrap_or("");
                if !key.starts_with('/') {
                    let mut key_buf = [0u8; 1024];
                    let mut kw = crate::macros::StackWriter::new(&mut key_buf);
                    let _ = write!(kw, "/{}", key);
                    key_fs.set(kw.as_str());
                } else {
                    key_fs.set(key);
                }
            }
        };

        let mut norm_fs = FixedString::<1024>::new();
        norm_fs.set(normalized);

        let manifest_key_hash = vrift_ipc::fnv1a_hash(key_fs.as_str());
        Some(VfsPath {
            absolute: norm_fs,
            manifest_key: key_fs,
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
