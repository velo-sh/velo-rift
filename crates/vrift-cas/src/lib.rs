//! # vrift-cas
//!
//! Content-Addressable Storage (CAS) implementation for Velo Rift.
//!
//! The CAS uses BLAKE3 hashing with a 3-level fan-out directory layout
//! for efficient file organization and lookup (RFC-0039 compliant).
//!
//! ## Directory Layout (RFC-0039 §6)
//!
//! ```text
//! ~/.vrift/the_source/
//! └── blake3/
//!     └── ab/
//!         └── cd/
//!             └── abcd1234...efgh_12345.bin  # hash_size.ext
//! ```
//!
//! ## I/O Backend Abstraction
//!
//! The crate provides platform-specific I/O backends for optimal batch ingestion:
//! - Linux: io_uring (feature-gated)
//! - macOS: GCD-style dispatch
//! - Fallback: Rayon thread pool

mod io_backend;

pub use io_backend::{create_backend, rayon_backend, IngestBackend};

use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use tracing::instrument;

use thiserror::Error;

/// BLAKE3 hash type (32 bytes)
pub type Blake3Hash = [u8; 32];

/// Errors that can occur during CAS operations
#[derive(Error, Debug)]
pub enum CasError {
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),

    #[error("Blob not found: {hash}")]
    NotFound { hash: String },

    #[error("Hash mismatch: expected {expected}, got {actual}")]
    HashMismatch { expected: String, actual: String },
}

pub type Result<T> = std::result::Result<T, CasError>;

/// Content-Addressable Storage store
///
/// Stores blobs indexed by their BLAKE3 hash with a 2-char prefix fan-out.
#[derive(Debug, Clone)]
pub struct CasStore {
    root: PathBuf,
}

impl CasStore {
    /// Create a new CAS store at the given root directory.
    ///
    /// The directory will be created if it doesn't exist.
    pub fn new<P: AsRef<Path>>(root: P) -> Result<Self> {
        let root = root.as_ref().to_path_buf();
        fs::create_dir_all(&root)?;
        Ok(Self { root })
    }

    /// Create a CAS store at the default location (`~/.vrift/the_source/`).
    /// 
    /// Per RFC-0039 §3.4, the CAS is stored in the user's home directory.
    pub fn default_location() -> Result<Self> {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
        Self::new(format!("{}/.vrift/the_source", home))
    }

    /// Compute the BLAKE3 hash of the given bytes.
    #[inline]
    pub fn compute_hash(data: &[u8]) -> Blake3Hash {
        *blake3::hash(data).as_bytes()
    }

    /// Convert a hash to its hex string representation.
    #[inline]
    pub fn hash_to_hex(hash: &Blake3Hash) -> String {
        hash.iter().map(|b| format!("{:02x}", b)).collect()
    }

    /// Parse a hex string into a hash.
    pub fn hex_to_hash(hex: &str) -> Option<Blake3Hash> {
        if hex.len() != 64 {
            return None;
        }
        let mut hash = [0u8; 32];
        for (i, chunk) in hex.as_bytes().chunks(2).enumerate() {
            let s = std::str::from_utf8(chunk).ok()?;
            hash[i] = u8::from_str_radix(s, 16).ok()?;
        }
        Some(hash)
    }

    /// Get the path where a blob with the given hash would be stored.
    /// 
    /// Uses RFC-0039 §6 layout: `blake3/ab/cd/hash_size.ext`
    /// The simple version without size/ext (for backwards compat during transition).
    fn blob_path(&self, hash: &Blake3Hash) -> PathBuf {
        let hex = Self::hash_to_hex(hash);
        let l1 = &hex[..2];   // First 2 chars
        let l2 = &hex[2..4];  // Next 2 chars
        self.root.join("blake3").join(l1).join(l2).join(&hex)
    }

    /// Get the path for a self-describing blob (RFC-0039 format).
    /// 
    /// Format: `blake3/ab/cd/hash_size.ext`
    /// - O(1) integrity check via filename size
    /// - Extension enables direct file type inspection
    pub fn blob_path_with_metadata(&self, hash: &Blake3Hash, size: u64, ext: &str) -> PathBuf {
        let hex = Self::hash_to_hex(hash);
        let l1 = &hex[..2];
        let l2 = &hex[2..4];
        let filename = if ext.is_empty() {
            format!("{}_{}", hex, size)
        } else {
            format!("{}_{}.{}", hex, size, ext)
        };
        self.root.join("blake3").join(l1).join(l2).join(filename)
    }

    /// Store bytes in the CAS, returning the content hash.
    ///
    /// If the content already exists, this is a no-op (deduplication).
    /// This method is thread-safe: uses unique temp file names to avoid race conditions.
    #[instrument(skip(self, data), level = "debug")]
    pub fn store(&self, data: &[u8]) -> Result<Blake3Hash> {
        let hash = Self::compute_hash(data);
        let path = self.blob_path(&hash);

        // Deduplication: skip if already exists
        if path.exists() {
            return Ok(hash);
        }

        // Create prefix directory
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        // Write atomically using temp file + rename
        // Use unique temp name to avoid race conditions in parallel mode
        let temp_name = format!(
            "{}.{}.{:?}.tmp",
            path.file_name().unwrap().to_string_lossy(),
            std::process::id(),
            std::thread::current().id()
        );
        let temp_path = path.with_file_name(&temp_name);
        let mut file = File::create(&temp_path)?;
        file.write_all(data)?;
        file.sync_all()?;

        // Atomic rename - if another thread beat us, that's fine (same content)
        if let Err(e) = fs::rename(&temp_path, &path) {
            // Clean up orphaned temp file if rename failed
            let _ = fs::remove_file(&temp_path);
            // If the target exists now (race), that's OK - dedup succeeded
            if path.exists() {
                return Ok(hash);
            }
            return Err(CasError::Io(e));
        }

        Ok(hash)
    }

    /// Store a file in the CAS by reading from the filesystem.
    pub fn store_file<P: AsRef<Path>>(&self, path: P) -> Result<Blake3Hash> {
        let data = fs::read(path)?;
        self.store(&data)
    }

    /// Retrieve bytes from the CAS by hash.
    #[instrument(skip(self), level = "debug")]
    pub fn get(&self, hash: &Blake3Hash) -> Result<Vec<u8>> {
        let path = self.blob_path(hash);
        if !path.exists() {
            return Err(CasError::NotFound {
                hash: Self::hash_to_hex(hash),
            });
        }

        let mut file = File::open(&path)?;
        let mut data = Vec::new();
        file.read_to_end(&mut data)?;

        // Verify hash on read (integrity check)
        let actual_hash = Self::compute_hash(&data);
        if actual_hash != *hash {
            return Err(CasError::HashMismatch {
                expected: Self::hash_to_hex(hash),
                actual: Self::hash_to_hex(&actual_hash),
            });
        }

        Ok(data)
    }

    /// Check if a blob exists in the CAS.
    pub fn exists(&self, hash: &Blake3Hash) -> bool {
        self.blob_path(hash).exists()
    }

    /// Delete a blob from the CAS.
    pub fn delete(&self, hash: &Blake3Hash) -> Result<()> {
        let path = self.blob_path(hash);
        if path.exists() {
            fs::remove_file(path)?;
            Ok(())
        } else {
            Err(CasError::NotFound {
                hash: Self::hash_to_hex(hash),
            })
        }
    }

    /// Get the root path of the CAS.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Get statistics about the CAS.
    /// 
    /// Traverses the 3-level structure: blake3/ab/cd/hash
    pub fn stats(&self) -> Result<CasStats> {
        let mut blob_count = 0u64;
        let mut total_bytes = 0u64;
        let mut size_histogram: std::collections::HashMap<&str, u64> =
            std::collections::HashMap::new();

        // Level 0: blake3/ directory
        let blake3_dir = self.root.join("blake3");
        if !blake3_dir.exists() {
            return Ok(CasStats::default());
        }

        // Level 1: ab/ directories
        for l1_entry in fs::read_dir(&blake3_dir)? {
            let l1_entry = l1_entry?;
            if !l1_entry.file_type()?.is_dir() {
                continue;
            }

            // Level 2: cd/ directories
            for l2_entry in fs::read_dir(l1_entry.path())? {
                let l2_entry = l2_entry?;
                if !l2_entry.file_type()?.is_dir() {
                    continue;
                }

                // Level 3: hash files
                for blob in fs::read_dir(l2_entry.path())? {
                    let blob = blob?;
                    if blob.file_type()?.is_file() {
                        // Skip temp files
                        if blob.path().extension().is_some_and(|ext| ext == "tmp") {
                            continue;
                        }
                        let size = blob.metadata()?.len();
                        blob_count += 1;
                        total_bytes += size;

                        // Categorize by size
                        let category = if size < 1024 {
                            "<1KB"
                        } else if size < 1024 * 1024 {
                            "1KB-1MB"
                        } else if size < 100 * 1024 * 1024 {
                            "1MB-100MB"
                        } else {
                            ">100MB"
                        };
                        *size_histogram.entry(category).or_insert(0) += 1;
                    }
                }
            }
        }

        Ok(CasStats {
            blob_count,
            total_bytes,
            small_blobs: *size_histogram.get("<1KB").unwrap_or(&0),
            medium_blobs: *size_histogram.get("1KB-1MB").unwrap_or(&0),
            large_blobs: *size_histogram.get("1MB-100MB").unwrap_or(&0),
            huge_blobs: *size_histogram.get(">100MB").unwrap_or(&0),
        })
    }

    /// Get a memory-mapped view of a blob (D6: zero-copy optimization).
    ///
    /// This is more efficient than `get()` for large files as it avoids copying
    /// the data into memory. The file is mapped directly from the filesystem,
    /// leveraging the page cache for sharing across processes.
    #[instrument(skip(self), level = "debug")]
    pub fn get_mmap(&self, hash: &Blake3Hash) -> Result<memmap2::Mmap> {
        let path = self.blob_path(hash);
        if !path.exists() {
            return Err(CasError::NotFound {
                hash: Self::hash_to_hex(hash),
            });
        }

        let file = File::open(&path)?;
        // Safety: The file is read-only and we're not modifying it
        let mmap = unsafe { memmap2::Mmap::map(&file) }.map_err(io::Error::other)?;

        Ok(mmap)
    }

    /// Get an iterator over all blob hashes in the CAS.
    /// 
    /// Traverses the 3-level structure: blake3/ab/cd/hash
    pub fn iter(&self) -> Result<CasIterator> {
        let blake3_dir = self.root.join("blake3");
        if !blake3_dir.exists() {
            // Return empty iterator if blake3 dir doesn't exist
            return Ok(CasIterator {
                l1_iter: fs::read_dir(&self.root)?, // Will be empty or invalid
                l2_iter: None,
                l3_iter: None,
                blake3_exists: false,
            });
        }
        Ok(CasIterator {
            l1_iter: fs::read_dir(&blake3_dir)?,
            l2_iter: None,
            l3_iter: None,
            blake3_exists: true,
        })
    }

    /// Get the filesystem path to a blob (for external mmap or direct access).
    pub fn blob_path_for_hash(&self, hash: &Blake3Hash) -> Option<PathBuf> {
        let path = self.blob_path(hash);
        if path.exists() {
            Some(path)
        } else {
            None
        }
    }

    // ========================================================================
    // Tiered Ingest Functions (RFC-0039)
    // ========================================================================

    /// Store data and create a symlink projection (Tier-1 Immutable).
    ///
    /// For immutable assets like registry deps or toolchains:
    /// 1. Store content in CAS
    /// 2. Create symlink from target_path → CAS blob
    /// 3. (Linux only) Set immutable flag on CAS blob
    ///
    /// This provides zero-overhead VFS bypass for reads.
    #[cfg(unix)]
    pub fn store_and_link_immutable<P: AsRef<Path>>(
        &self,
        data: &[u8],
        target_path: P,
    ) -> Result<Blake3Hash> {
        use std::os::unix::fs::symlink;

        let hash = self.store(data)?;
        let cas_path = self.blob_path(&hash);
        let target = target_path.as_ref();

        // Remove existing file/symlink if present
        if target.exists() || target.symlink_metadata().is_ok() {
            fs::remove_file(target).ok();
        }

        // Create parent directories
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }

        // Create symlink: target → CAS blob
        symlink(&cas_path, target)?;

        // (Linux) Try to set immutable flag on CAS blob (requires root)
        #[cfg(target_os = "linux")]
        {
            Self::set_immutable_flag(&cas_path).ok(); // Best effort
        }

        Ok(hash)
    }

    /// Store data and create a hardlink projection (Tier-2 Mutable).
    ///
    /// For mutable assets like build outputs:
    /// 1. Store content in CAS
    /// 2. Create hardlink from target_path → CAS blob
    /// 3. Set read-only permissions (chmod 444)
    ///
    /// Writes trigger Break-Before-Write in the VFS shim.
    #[cfg(unix)]
    pub fn store_and_link_mutable<P: AsRef<Path>>(
        &self,
        data: &[u8],
        target_path: P,
    ) -> Result<Blake3Hash> {
        let hash = self.store(data)?;
        let cas_path = self.blob_path(&hash);
        let target = target_path.as_ref();

        // Remove existing file if present
        if target.exists() {
            fs::remove_file(target)?;
        }

        // Create parent directories
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }

        // Create hardlink: target shares inode with CAS blob
        fs::hard_link(&cas_path, target)?;

        // Set read-only (chmod 444) to catch unintended writes
        Self::set_readonly(target)?;

        Ok(hash)
    }

    /// Create symlink projection without storing (blob already in CAS).
    #[cfg(unix)]
    pub fn link_immutable<P: AsRef<Path>>(&self, hash: &Blake3Hash, target_path: P) -> Result<()> {
        use std::os::unix::fs::symlink;

        let cas_path = self.blob_path(hash);
        if !cas_path.exists() {
            return Err(CasError::NotFound {
                hash: Self::hash_to_hex(hash),
            });
        }

        let target = target_path.as_ref();

        // Remove existing
        if target.exists() || target.symlink_metadata().is_ok() {
            fs::remove_file(target).ok();
        }

        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }

        symlink(&cas_path, target)?;
        Ok(())
    }

    /// Create hardlink projection without storing (blob already in CAS).
    #[cfg(unix)]
    pub fn link_mutable<P: AsRef<Path>>(&self, hash: &Blake3Hash, target_path: P) -> Result<()> {
        let cas_path = self.blob_path(hash);
        if !cas_path.exists() {
            return Err(CasError::NotFound {
                hash: Self::hash_to_hex(hash),
            });
        }

        let target = target_path.as_ref();

        if target.exists() {
            fs::remove_file(target)?;
        }

        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }

        fs::hard_link(&cas_path, target)?;
        Self::set_readonly(target)?;
        Ok(())
    }

    /// Set file to read-only (chmod 444).
    #[cfg(unix)]
    fn set_readonly(path: &Path) -> Result<()> {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(path)?.permissions();
        perms.set_mode(0o444);
        fs::set_permissions(path, perms)?;
        Ok(())
    }

    /// Set immutable flag on Linux (chattr +i).
    #[cfg(target_os = "linux")]
    fn set_immutable_flag(path: &Path) -> Result<()> {
        use std::os::unix::io::AsRawFd;
        use std::process::Command;

        // Try using chattr command (requires root)
        let status = Command::new("chattr")
            .arg("+i")
            .arg(path)
            .status();

        match status {
            Ok(s) if s.success() => Ok(()),
            _ => Ok(()), // Silently fail if not root
        }
    }
}

/// Statistics about the CAS store
#[derive(Debug, Clone, Default)]
pub struct CasStats {
    /// Number of unique blobs stored
    pub blob_count: u64,
    /// Total bytes stored (deduplicated)
    pub total_bytes: u64,
    /// Blobs < 1KB
    pub small_blobs: u64,
    /// Blobs 1KB - 1MB
    pub medium_blobs: u64,
    /// Blobs 1MB - 100MB
    pub large_blobs: u64,
    /// Blobs > 100MB
    pub huge_blobs: u64,
}

impl CasStats {
    /// Calculate average blob size
    pub fn avg_blob_size(&self) -> u64 {
        if self.blob_count == 0 {
            0
        } else {
            self.total_bytes / self.blob_count
        }
    }
}

/// Iterator over CAS hashes (3-level: blake3/ab/cd/hash)
pub struct CasIterator {
    l1_iter: fs::ReadDir,      // Level 1: ab/ directories
    l2_iter: Option<fs::ReadDir>,  // Level 2: cd/ directories
    l3_iter: Option<fs::ReadDir>,  // Level 3: hash files
    blake3_exists: bool,
}

impl Iterator for CasIterator {
    type Item = Result<Blake3Hash>;

    fn next(&mut self) -> Option<Self::Item> {
        if !self.blake3_exists {
            return None;
        }

        loop {
            // Try to get next file from L3 (hash files)
            if let Some(ref mut l3) = self.l3_iter {
                match l3.next() {
                    Some(Ok(entry)) => {
                        let path = entry.path();
                        if path.is_file() {
                            // Skip temp files
                            if path.extension().is_some_and(|ext| ext == "tmp") {
                                continue;
                            }

                            // Parse filename as hash (may include _size suffix)
                            if let Some(filename) = path.file_name().and_then(|n| n.to_str()) {
                                // Handle both "hash" and "hash_size.ext" formats
                                let hash_part = filename.split('_').next().unwrap_or(filename);
                                if let Some(hash) = CasStore::hex_to_hash(hash_part) {
                                    return Some(Ok(hash));
                                }
                            }
                        }
                        continue;
                    }
                    Some(Err(e)) => return Some(Err(CasError::Io(e))),
                    None => self.l3_iter = None,
                }
            }

            // L3 exhausted, try to get next L2 directory
            if let Some(ref mut l2) = self.l2_iter {
                match l2.next() {
                    Some(Ok(entry)) => {
                        if entry.file_type().ok()?.is_dir() {
                            match fs::read_dir(entry.path()) {
                                Ok(iter) => self.l3_iter = Some(iter),
                                Err(e) => return Some(Err(CasError::Io(e))),
                            }
                        }
                        continue;
                    }
                    Some(Err(e)) => return Some(Err(CasError::Io(e))),
                    None => self.l2_iter = None,
                }
            }

            // L2 exhausted, try to get next L1 directory
            match self.l1_iter.next() {
                Some(Ok(entry)) => {
                    if entry.file_type().ok()?.is_dir() {
                        match fs::read_dir(entry.path()) {
                            Ok(iter) => self.l2_iter = Some(iter),
                            Err(e) => return Some(Err(CasError::Io(e))),
                        }
                    }
                }
                Some(Err(e)) => return Some(Err(CasError::Io(e))),
                None => return None, // All levels exhausted
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_store_and_retrieve() {
        let temp = TempDir::new().unwrap();
        let cas = CasStore::new(temp.path()).unwrap();

        let data = b"Hello, Velo!";
        let hash = cas.store(data).unwrap();

        let retrieved = cas.get(&hash).unwrap();
        assert_eq!(retrieved, data);
    }

    #[test]
    fn test_deduplication() {
        let temp = TempDir::new().unwrap();
        let cas = CasStore::new(temp.path()).unwrap();

        let data = b"Duplicate content";
        let hash1 = cas.store(data).unwrap();
        let hash2 = cas.store(data).unwrap();

        assert_eq!(hash1, hash2);

        let stats = cas.stats().unwrap();
        assert_eq!(stats.blob_count, 1);
    }

    #[test]
    fn test_not_found() {
        let temp = TempDir::new().unwrap();
        let cas = CasStore::new(temp.path()).unwrap();

        let fake_hash = [0u8; 32];
        let result = cas.get(&fake_hash);
        assert!(matches!(result, Err(CasError::NotFound { .. })));
    }

    #[test]
    fn test_hash_to_hex_roundtrip() {
        let data = b"test data";
        let hash = CasStore::compute_hash(data);
        let hex = CasStore::hash_to_hex(&hash);
        let parsed = CasStore::hex_to_hash(&hex).unwrap();
        assert_eq!(hash, parsed);
    }

    #[test]
    fn test_empty_file() {
        let temp = TempDir::new().unwrap();
        let cas = CasStore::new(temp.path()).unwrap();

        let data = b"";
        let hash = cas.store(data).unwrap();
        let retrieved = cas.get(&hash).unwrap();
        assert!(retrieved.is_empty());
    }

    #[test]
    fn test_iter() {
        let temp = TempDir::new().unwrap();
        let cas = CasStore::new(temp.path()).unwrap();

        let content1 = b"content1";
        let content2 = b"content2";
        let hash1 = cas.store(content1).unwrap();
        let hash2 = cas.store(content2).unwrap();

        // Use a set to verify unordered results
        let mut hashes = std::collections::HashSet::new();
        for h in cas.iter().unwrap() {
            hashes.insert(h.unwrap());
        }

        assert_eq!(hashes.len(), 2);
        assert!(hashes.contains(&hash1));
        assert!(hashes.contains(&hash2));
    }

    #[cfg(unix)]
    #[test]
    fn test_store_and_link_immutable() {
        let cas_dir = TempDir::new().unwrap();
        let target_dir = TempDir::new().unwrap();
        let cas = CasStore::new(cas_dir.path()).unwrap();

        let data = b"immutable content";
        let target_path = target_dir.path().join("immutable_file.txt");

        let hash = cas.store_and_link_immutable(data, &target_path).unwrap();

        // Verify symlink exists
        assert!(target_path.symlink_metadata().unwrap().file_type().is_symlink());

        // Verify content via symlink
        let read_content = fs::read(&target_path).unwrap();
        assert_eq!(read_content, data);

        // Verify hash matches
        assert_eq!(hash, CasStore::compute_hash(data));
    }

    #[cfg(unix)]
    #[test]
    fn test_store_and_link_mutable() {
        use std::os::unix::fs::MetadataExt;

        let cas_dir = TempDir::new().unwrap();
        let target_dir = TempDir::new().unwrap();
        let cas = CasStore::new(cas_dir.path()).unwrap();

        let data = b"mutable content";
        let target_path = target_dir.path().join("mutable_file.txt");

        let hash = cas.store_and_link_mutable(data, &target_path).unwrap();

        // Verify hardlink exists (not symlink)
        let meta = target_path.metadata().unwrap();
        assert!(meta.file_type().is_file());
        assert!(meta.nlink() >= 2); // At least 2 links (CAS + target)

        // Verify content
        let read_content = fs::read(&target_path).unwrap();
        assert_eq!(read_content, data);

        // Verify read-only permissions (mode 444)
        assert_eq!(meta.mode() & 0o777, 0o444);
    }

    #[cfg(unix)]
    #[test]
    fn test_link_immutable() {
        let cas_dir = TempDir::new().unwrap();
        let target_dir = TempDir::new().unwrap();
        let cas = CasStore::new(cas_dir.path()).unwrap();

        let data = b"pre-stored content";
        let hash = cas.store(data).unwrap();

        let target_path = target_dir.path().join("linked_immutable.txt");
        cas.link_immutable(&hash, &target_path).unwrap();

        assert!(target_path.symlink_metadata().unwrap().file_type().is_symlink());
        assert_eq!(fs::read(&target_path).unwrap(), data);
    }

    // =========================================================================
    // RFC-0039 Specific Tests
    // =========================================================================

    #[test]
    fn test_3level_sharding_path_format() {
        // RFC-0039: CAS path layout should be blake3/ab/cd/hash
        // Note: blob_path() returns hash-only, blob_path_with_metadata() includes size
        let temp = TempDir::new().unwrap();
        let cas = CasStore::new(temp.path()).unwrap();

        let data = b"test content for sharding";
        let hash = cas.store(data).unwrap();
        let hex = CasStore::hash_to_hex(&hash);

        // Verify the file exists in the correct 3-level structure
        let expected_l1 = &hex[..2];
        let expected_l2 = &hex[2..4];
        
        let blob_dir = temp.path().join("blake3").join(expected_l1).join(expected_l2);
        assert!(blob_dir.exists(), "3-level directory should exist: {:?}", blob_dir);
        
        // Find the blob file
        let entries: Vec<_> = fs::read_dir(&blob_dir).unwrap().collect();
        assert_eq!(entries.len(), 1, "Should have exactly one blob");
        
        let filename = entries[0].as_ref().unwrap().file_name();
        let filename_str = filename.to_string_lossy();
        
        // Verify filename starts with hash (basic blob_path format)
        assert!(filename_str.starts_with(&hex), "Filename should start with hash");
    }

    #[test]
    fn test_blob_path_with_metadata() {
        // RFC-0039: Self-describing filename hash_size.ext
        let temp = TempDir::new().unwrap();
        let cas = CasStore::new(temp.path()).unwrap();

        let data = b"metadata test";
        let hash = CasStore::compute_hash(data);
        let hex = CasStore::hash_to_hex(&hash);

        // Test with extension
        let path_with_ext = cas.blob_path_with_metadata(&hash, 1024, "bin");
        assert!(path_with_ext.to_string_lossy().contains("blake3"));
        assert!(path_with_ext.to_string_lossy().contains(&hex[..2]));
        assert!(path_with_ext.to_string_lossy().contains(&hex[2..4]));
        assert!(path_with_ext.to_string_lossy().ends_with("_1024.bin"));

        // Test without extension
        let path_no_ext = cas.blob_path_with_metadata(&hash, 512, "");
        assert!(path_no_ext.to_string_lossy().ends_with("_512"));
        assert!(!path_no_ext.to_string_lossy().ends_with("."));
    }

    #[test]
    fn test_self_describing_filename_with_metadata() {
        // RFC-0039: blob_path_with_metadata() should produce hash_size.ext format
        let temp = TempDir::new().unwrap();
        let cas = CasStore::new(temp.path()).unwrap();

        let data = vec![0u8; 1234]; // Known size
        let hash = CasStore::compute_hash(&data);

        // Test blob_path_with_metadata produces correct format
        let path = cas.blob_path_with_metadata(&hash, 1234, "bin");
        let filename = path.file_name().unwrap().to_string_lossy();

        // Filename should contain decimal size "1234"
        assert!(
            filename.contains("_1234."),
            "Filename '{}' should contain decimal size _1234.",
            filename
        );
        assert!(
            filename.ends_with(".bin"),
            "Filename '{}' should end with .bin",
            filename
        );
    }

    #[test]
    fn test_stats_traverses_3level_structure() {
        // RFC-0039: stats() should correctly traverse blake3/ab/cd/ structure
        let temp = TempDir::new().unwrap();
        let cas = CasStore::new(temp.path()).unwrap();

        // Store multiple blobs
        cas.store(b"blob1").unwrap();
        cas.store(b"blob2").unwrap();
        cas.store(b"blob3").unwrap();

        let stats = cas.stats().unwrap();
        assert_eq!(stats.blob_count, 3, "Should count all 3 blobs");
        assert!(stats.total_bytes > 0, "Total bytes should be non-zero");
    }

    #[test]
    fn test_iter_traverses_3level_structure() {
        // RFC-0039: iter() should correctly traverse blake3/ab/cd/ structure
        let temp = TempDir::new().unwrap();
        let cas = CasStore::new(temp.path()).unwrap();

        let hash1 = cas.store(b"iter1").unwrap();
        let hash2 = cas.store(b"iter2").unwrap();
        let hash3 = cas.store(b"iter3").unwrap();

        let mut found_hashes: Vec<_> = cas.iter().unwrap().filter_map(|r| r.ok()).collect();
        found_hashes.sort();

        let mut expected = vec![hash1, hash2, hash3];
        expected.sort();

        assert_eq!(found_hashes, expected, "Iterator should find all stored hashes");
    }
}
