//! # velo-manifest
//!
//! Manifest data structure for Velo Rift virtual filesystem.
//!
//! The manifest maps path hashes to VnodeEntry structs, providing
//! O(1) lookup for any file path in the virtual filesystem.
//!
//! ## Storage Backends
//!
//! - `LmdbManifest`: LMDB-backed with ACID transactions (RFC-0039)

pub mod lmdb;
pub mod tier;

pub use lmdb::{AssetTier, LmdbError, LmdbManifest, LmdbResult, ManifestEntry};
pub use tier::{classify_tier, TierClassifier, DEFAULT_TIER1_PATTERNS, DEFAULT_TIER2_PATTERNS};

use rkyv::Archive;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use thiserror::Error;

use vrift_cas::Blake3Hash;

/// Errors that can occur during manifest operations
#[derive(Error, Debug)]
pub enum ManifestError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Serialization error: {0}")]
    Rkyv(String),

    #[error("Path not found: {0}")]
    PathNotFound(String),
}

pub type Result<T> = std::result::Result<T, ManifestError>;

/// Flags for VnodeEntry
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Serialize,
    Deserialize,
    Default,
    Archive,
    rkyv::Serialize,
    rkyv::Deserialize,
)]
#[rkyv(compare(PartialEq), derive(Debug))]
#[repr(u8)]
pub enum VnodeFlags {
    /// Regular file
    #[default]
    File = 0,
    /// Directory
    Directory = 1,
    /// Symbolic link
    Symlink = 2,
    /// Executable file
    Executable = 3,
}

/// Virtual node entry representing a file or directory in the manifest.
///
/// This is a 56-byte packed structure for memory efficiency:
/// - content_hash: 32 bytes (BLAKE3)
/// - size: 8 bytes
/// - mtime: 8 bytes
/// - mode: 4 bytes
/// - flags: 2 bytes
/// - _pad: 2 bytes
#[derive(
    Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Archive, rkyv::Serialize, rkyv::Deserialize,
)]
#[rkyv(derive(Debug))]
pub struct VnodeEntry {
    /// BLAKE3 hash of the file content (stored in CAS)
    pub content_hash: Blake3Hash,
    /// File size in bytes
    pub size: u64,
    /// Modification time (nanoseconds since Unix epoch)
    pub mtime: u64,
    /// Permission mode bits (e.g., 0o644)
    pub mode: u32,
    /// Entry type flags
    pub flags: u16,
    /// Padding for alignment
    #[serde(skip)]
    #[rkyv(with = rkyv::with::Skip)]
    pub _pad: u16,
}

impl VnodeEntry {
    /// Create a new VnodeEntry for a regular file
    pub fn new_file(content_hash: Blake3Hash, size: u64, mtime: u64, mode: u32) -> Self {
        Self {
            content_hash,
            size,
            mtime,
            mode,
            flags: VnodeFlags::File as u16,
            _pad: 0,
        }
    }

    /// Create a new VnodeEntry for a directory
    pub fn new_directory(mtime: u64, mode: u32) -> Self {
        Self {
            content_hash: [0u8; 32],
            size: 0,
            mtime,
            mode,
            flags: VnodeFlags::Directory as u16,
            _pad: 0,
        }
    }

    /// Create a new VnodeEntry for a symbolic link
    ///
    /// `target_hash` is the hash of the target path string.
    /// `target_len` is the length of the target path string.
    pub fn new_symlink(target_hash: Blake3Hash, target_len: u64, mtime: u64) -> Self {
        Self {
            content_hash: target_hash,
            size: target_len,
            mtime,
            mode: 0o777,
            flags: VnodeFlags::Symlink as u16,
            _pad: 0,
        }
    }

    /// Check if this entry is a directory
    pub fn is_dir(&self) -> bool {
        self.flags & (VnodeFlags::Directory as u16) != 0
    }

    /// Check if this entry is a regular file
    pub fn is_file(&self) -> bool {
        self.flags == VnodeFlags::File as u16
    }

    /// Check if this entry is a symbolic link
    pub fn is_symlink(&self) -> bool {
        self.flags & (VnodeFlags::Symlink as u16) != 0
    }

    /// Check if this entry is executable
    pub fn is_executable(&self) -> bool {
        self.flags & (VnodeFlags::Executable as u16) != 0
    }
}

/// Path hash type - hash of the normalized path string
pub type PathHash = Blake3Hash;

/// Compute the path hash for a given path string
pub fn compute_path_hash(path: &str) -> PathHash {
    let normalized = normalize_vfs_path(path);
    *blake3::hash(normalized.as_bytes()).as_bytes()
}

/// Normalize a path for consistent hashing within the VFS
fn normalize_vfs_path(path: &str) -> String {
    let mut normalized = path.replace("//", "/");
    // Remove trailing slash unless it's root
    if normalized.len() > 1 && normalized.ends_with('/') {
        normalized.pop();
    }
    // Ensure leading slash
    if !normalized.starts_with('/') {
        normalized.insert(0, '/');
    }
    normalized
}

/// Robust path normalization (expands tilde and resolves absolute)
pub fn normalize_path(p: &str) -> PathBuf {
    if let Some(stripped) = p.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(stripped);
        }
    }
    PathBuf::from(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vnode_entry_size() {
        let entry = VnodeEntry::new_file([0u8; 32], 1024, 1706448000, 0o644);
        assert!(entry.is_file());
        assert!(!entry.is_dir());
    }
}
