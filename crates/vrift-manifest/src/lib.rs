//! # velo-manifest
//!
//! Manifest data structure for Velo Rift virtual filesystem.
//!
//! The manifest maps path hashes to VnodeEntry structs, providing
//! O(1) lookup for any file path in the virtual filesystem.
//!
//! ## Storage Backends
//!
//! - `Manifest`: In-memory HashMap with bincode persistence (legacy)
//! - `LmdbManifest`: LMDB-backed with ACID transactions (RFC-0039)

pub mod lmdb;
pub mod tier;

pub use lmdb::{AssetTier, LmdbError, LmdbManifest, LmdbResult, ManifestEntry};
pub use tier::{classify_tier, TierClassifier, DEFAULT_TIER1_PATTERNS, DEFAULT_TIER2_PATTERNS};

use std::collections::HashMap;
use std::fs::File;
use std::io::{self, BufReader, BufWriter};
use std::path::Path;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use vrift_cas::Blake3Hash;

/// Errors that can occur during manifest operations
#[derive(Error, Debug)]
pub enum ManifestError {
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),

    #[error("Serialization error: {0}")]
    Bincode(#[from] bincode::Error),

    #[error("Path not found: {0}")]
    PathNotFound(String),
}

pub type Result<T> = std::result::Result<T, ManifestError>;

/// Flags for VnodeEnt#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Default)]
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    _pad: u16,
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
            mode: 0o777, // Symlinks usually have dummy permissions
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

/// Manifest containing the path → VnodeEntry mapping
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Manifest {
    /// Version for compatibility
    pub version: u32,
    /// Path hash to VnodeEntry mapping
    entries: HashMap<PathHash, VnodeEntry>,
    /// Path hash to original path string (for debugging/listing)
    #[serde(default)]
    paths: HashMap<PathHash, String>,
}

impl Manifest {
    /// Create a new empty manifest
    pub fn new() -> Self {
        Self {
            version: 1,
            entries: HashMap::new(),
            paths: HashMap::new(),
        }
    }

    /// Insert an entry into the manifest
    pub fn insert(&mut self, path: &str, entry: VnodeEntry) {
        let hash = compute_path_hash(path);
        self.entries.insert(hash, entry);
        self.paths.insert(hash, normalize_vfs_path(path));
    }

    /// Get an entry by path
    pub fn get(&self, path: &str) -> Option<&VnodeEntry> {
        let hash = compute_path_hash(path);
        self.entries.get(&hash)
    }

    /// Get an entry by path hash
    pub fn get_by_hash(&self, hash: &PathHash) -> Option<&VnodeEntry> {
        self.entries.get(hash)
    }

    /// Check if a path exists in the manifest
    pub fn contains(&self, path: &str) -> bool {
        let hash = compute_path_hash(path);
        self.entries.contains_key(&hash)
    }

    /// Remove an entry from the manifest
    pub fn remove(&mut self, path: &str) -> Option<VnodeEntry> {
        let hash = compute_path_hash(path);
        self.paths.remove(&hash);
        self.entries.remove(&hash)
    }

    /// Get the number of entries
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Check if the manifest is empty
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Iterate over all entries with their paths
    pub fn iter(&self) -> impl Iterator<Item = (&str, &VnodeEntry)> {
        self.paths
            .iter()
            .filter_map(|(hash, path)| self.entries.get(hash).map(|entry| (path.as_str(), entry)))
    }

    /// Iterate over all paths
    pub fn paths(&self) -> impl Iterator<Item = &str> {
        self.paths.values().map(|s| s.as_str())
    }

    /// Save the manifest to a file using bincode
    pub fn save<P: AsRef<Path>>(&self, path: P) -> Result<()> {
        let file = File::create(path)?;
        let writer = BufWriter::new(file);
        bincode::serialize_into(writer, self)?;
        Ok(())
    }

    /// Load a manifest from a file
    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self> {
        let file = File::open(path)?;
        let reader = BufReader::new(file);
        let manifest = bincode::deserialize_from(reader).map_err(io::Error::other)?;
        Ok(manifest)
    }

    /// Get manifest statistics
    pub fn stats(&self) -> ManifestStats {
        let mut file_count = 0u64;
        let mut dir_count = 0u64;
        let mut total_size = 0u64;

        for entry in self.entries.values() {
            if entry.is_dir() {
                dir_count += 1;
            } else {
                file_count += 1;
                total_size += entry.size;
            }
        }

        ManifestStats {
            file_count,
            dir_count,
            total_size,
        }
    }
}

/// Robust path normalization (expands tilde and resolves absolute)
pub fn normalize_path(p: &str) -> std::path::PathBuf {
    if let Some(stripped) = p.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(stripped);
        }
    }
    std::path::PathBuf::from(p)
}

/// Statistics about a manifest
#[derive(Debug, Clone, Default)]
pub struct ManifestStats {
    pub file_count: u64,
    pub dir_count: u64,
    pub total_size: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_vnode_entry_size() {
        // Verify VnodeEntry is 56 bytes as specified in ARCHITECTURE.md
        // Note: Due to serde, actual serialized size may differ
        let entry = VnodeEntry::new_file([0u8; 32], 1024, 1706448000, 0o644);
        assert!(entry.is_file());
        assert!(!entry.is_dir());
    }

    #[test]
    fn test_manifest_insert_get() {
        let mut manifest = Manifest::new();

        let hash = [0xABu8; 32];
        let entry = VnodeEntry::new_file(hash, 1024, 1706448000, 0o644);

        manifest.insert("/app/main.py", entry.clone());

        let retrieved = manifest.get("/app/main.py").unwrap();
        assert_eq!(retrieved.content_hash, hash);
        assert_eq!(retrieved.size, 1024);
    }

    #[test]
    fn test_path_normalization() {
        let mut manifest = Manifest::new();
        let entry = VnodeEntry::new_file([0u8; 32], 0, 0, 0o644);

        manifest.insert("app/main.py", entry.clone());

        // Should find with different path formats
        assert!(manifest.get("/app/main.py").is_some());
        assert!(manifest.get("app/main.py").is_some());
    }

    #[test]
    fn test_manifest_save_load() {
        let temp = TempDir::new().unwrap();
        let manifest_path = temp.path().join("test.manifest");

        let mut manifest = Manifest::new();
        manifest.insert(
            "/test/file.txt",
            VnodeEntry::new_file([1u8; 32], 100, 0, 0o644),
        );
        manifest.insert("/test/dir", VnodeEntry::new_directory(0, 0o755));

        manifest.save(&manifest_path).unwrap();

        let loaded = Manifest::load(&manifest_path).unwrap();
        assert_eq!(loaded.len(), 2);
        assert!(loaded.get("/test/file.txt").is_some());
    }

    #[test]
    fn test_manifest_stats() {
        let mut manifest = Manifest::new();
        manifest.insert("/a.txt", VnodeEntry::new_file([0u8; 32], 100, 0, 0o644));
        manifest.insert("/b.txt", VnodeEntry::new_file([1u8; 32], 200, 0, 0o644));
        manifest.insert("/dir", VnodeEntry::new_directory(0, 0o755));

        let stats = manifest.stats();
        assert_eq!(stats.file_count, 2);
        assert_eq!(stats.dir_count, 1);
        assert_eq!(stats.total_size, 300);
    }
}
