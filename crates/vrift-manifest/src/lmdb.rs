//! LMDB-backed Manifest for persistent, crash-safe path→hash mapping.
//!
//! Implements dual-layer architecture:
//! - Base Layer: Immutable entries (LMDB)
//! - Delta Layer: Mutable modifications (DashMap)

use std::path::Path;
use std::sync::Arc;

use dashmap::DashMap;
use heed::types::{Bytes, SerdeBincode, Str};
use heed::{Database, Env, EnvOpenOptions};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::debug;

use crate::{compute_path_hash, Blake3Hash, PathHash, VnodeEntry};

/// LMDB Manifest errors
#[derive(Error, Debug)]
pub enum LmdbError {
    #[error("LMDB error: {0}")]
    Heed(#[from] heed::Error),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Entry not found: {0}")]
    NotFound(String),

    #[error("Manifest corrupted: {0}")]
    Corrupted(String),
}

pub type LmdbResult<T> = std::result::Result<T, LmdbError>;

/// Asset tier for tiered storage model (RFC-0039)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[repr(u8)]
pub enum AssetTier {
    /// Immutable assets: symlink projection, owner transfer, immutable flag
    /// Examples: registry deps, toolchains
    Tier1Immutable = 1,

    /// Mutable assets: hardlink projection, Break-Before-Write
    /// Examples: build outputs, user config
    #[default]
    Tier2Mutable = 2,
}

/// Extended manifest entry with tier information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestEntry {
    /// The VnodeEntry (content hash, size, mtime, mode, flags)
    pub vnode: VnodeEntry,

    /// Asset tier classification
    pub tier: AssetTier,

    /// Whether this entry is "stale" (pending re-ingest)
    #[serde(default)]
    pub stale: bool,
}

/// Delta entry for in-memory modifications
#[derive(Debug, Clone)]
pub enum DeltaEntry {
    /// Modified or new entry
    Modified(ManifestEntry),
    /// Deleted (whiteout)
    Deleted,
}

/// LMDB-backed manifest with dual-layer architecture
///
/// Base Layer (LMDB): Immutable, O(1) mmap reads, ACID transactions
/// Delta Layer (DashMap): Mutable, per-session modifications
pub struct LmdbManifest {
    /// LMDB environment
    env: Env,

    /// Path hash → ManifestEntry database
    entries_db: Database<Bytes, SerdeBincode<ManifestEntry>>,

    /// Path hash → original path string database
    paths_db: Database<Bytes, Str>,

    /// Delta layer for uncommitted modifications
    delta: Arc<DashMap<PathHash, DeltaEntry>>,

    /// Path hash → path string for delta entries
    delta_paths: Arc<DashMap<PathHash, String>>,
}

impl LmdbManifest {
    /// Default LMDB map size: 1GB (expandable)
    const DEFAULT_MAP_SIZE: usize = 1024 * 1024 * 1024;

    /// Maximum readers
    const MAX_READERS: u32 = 128;

    /// Open or create an LMDB manifest at the given path
    ///
    /// Path should point to a directory that will contain the LMDB files.
    pub fn open<P: AsRef<Path>>(path: P) -> LmdbResult<Self> {
        let path = path.as_ref();

        // Create directory if needed
        std::fs::create_dir_all(path)?;

        // Open LMDB environment
        let env = unsafe {
            EnvOpenOptions::new()
                .map_size(Self::DEFAULT_MAP_SIZE)
                .max_readers(Self::MAX_READERS)
                .max_dbs(2)
                .open(path)?
        };

        // Open databases
        let mut wtxn = env.write_txn()?;
        let entries_db = env.create_database(&mut wtxn, Some("entries"))?;
        let paths_db = env.create_database(&mut wtxn, Some("paths"))?;
        wtxn.commit()?;

        debug!("Opened LMDB manifest at {:?}", path);

        Ok(Self {
            env,
            entries_db,
            paths_db,
            delta: Arc::new(DashMap::new()),
            delta_paths: Arc::new(DashMap::new()),
        })
    }

    /// Open with default path: `.vrift/manifest.lmdb`
    pub fn open_default() -> LmdbResult<Self> {
        Self::open(".vrift/manifest.lmdb")
    }

    /// Insert an entry into the delta layer (uncommitted)
    pub fn insert(&self, path: &str, vnode: VnodeEntry, tier: AssetTier) {
        let hash = compute_path_hash(path);
        let entry = ManifestEntry {
            vnode,
            tier,
            stale: false,
        };
        self.delta.insert(hash, DeltaEntry::Modified(entry));
        self.delta_paths.insert(hash, path.to_string());
    }

    /// Get an entry by path (checks delta first, then base)
    pub fn get(&self, path: &str) -> LmdbResult<Option<ManifestEntry>> {
        let hash = compute_path_hash(path);
        self.get_by_hash(&hash)
    }

    /// Get an entry by path hash
    pub fn get_by_hash(&self, hash: &PathHash) -> LmdbResult<Option<ManifestEntry>> {
        // Check delta layer first
        if let Some(delta_ref) = self.delta.get(hash) {
            return match delta_ref.value() {
                DeltaEntry::Modified(entry) => Ok(Some(entry.clone())),
                DeltaEntry::Deleted => Ok(None),
            };
        }

        // Check base layer
        let rtxn = self.env.read_txn()?;
        if let Some(entry) = self.entries_db.get(&rtxn, hash)? {
            return Ok(Some(entry));
        }

        Ok(None)
    }

    /// Mark an entry as stale (pending re-ingest after write)
    pub fn mark_stale(&self, path: &str) {
        let hash = compute_path_hash(path);

        if let Some(mut delta_ref) = self.delta.get_mut(&hash) {
            if let DeltaEntry::Modified(entry) = delta_ref.value_mut() {
                entry.stale = true;
            }
        } else {
            // Copy from base to delta and mark stale
            if let Ok(Some(mut entry)) = self.get_by_hash(&hash) {
                entry.stale = true;
                self.delta.insert(hash, DeltaEntry::Modified(entry));
                // Path should already exist, but ensure it's in delta_paths
                if let Ok(Some(p)) = self.get_path_by_hash(&hash) {
                    self.delta_paths.insert(hash, p);
                }
            }
        }
    }

    /// Remove an entry (creates whiteout in delta)
    pub fn remove(&self, path: &str) {
        let hash = compute_path_hash(path);
        self.delta.insert(hash, DeltaEntry::Deleted);
        self.delta_paths.remove(&hash);
    }

    /// Get the original path string for a hash
    pub fn get_path_by_hash(&self, hash: &PathHash) -> LmdbResult<Option<String>> {
        // Check delta first
        if let Some(path_ref) = self.delta_paths.get(hash) {
            return Ok(Some(path_ref.value().clone()));
        }

        // Check base
        let rtxn = self.env.read_txn()?;
        if let Some(path) = self.paths_db.get(&rtxn, hash)? {
            return Ok(Some(path.to_string()));
        }

        Ok(None)
    }

    /// Commit delta layer to base layer (ACID transaction)
    pub fn commit(&self) -> LmdbResult<()> {
        if self.delta.is_empty() {
            return Ok(());
        }

        let mut wtxn = self.env.write_txn()?;

        // Apply delta to base
        for entry in self.delta.iter() {
            let hash = entry.key();
            match entry.value() {
                DeltaEntry::Modified(manifest_entry) => {
                    self.entries_db.put(&mut wtxn, hash, manifest_entry)?;
                    if let Some(path_ref) = self.delta_paths.get(hash) {
                        self.paths_db.put(&mut wtxn, hash, path_ref.value())?;
                    }
                }
                DeltaEntry::Deleted => {
                    self.entries_db.delete(&mut wtxn, hash)?;
                    self.paths_db.delete(&mut wtxn, hash)?;
                }
            }
        }

        wtxn.commit()?;

        // Clear delta
        self.delta.clear();
        self.delta_paths.clear();

        debug!("Committed delta to LMDB");
        Ok(())
    }

    /// Get the number of entries (base + delta)
    pub fn len(&self) -> LmdbResult<usize> {
        let rtxn = self.env.read_txn()?;
        let base_len = self.entries_db.len(&rtxn)?;

        // Adjust for delta
        let mut delta_added = 0usize;
        let mut delta_removed = 0usize;

        for entry in self.delta.iter() {
            match entry.value() {
                DeltaEntry::Modified(_) => {
                    // Check if it's a new entry or modification
                    if self.entries_db.get(&rtxn, entry.key())?.is_none() {
                        delta_added += 1;
                    }
                }
                DeltaEntry::Deleted => {
                    if self.entries_db.get(&rtxn, entry.key())?.is_some() {
                        delta_removed += 1;
                    }
                }
            }
        }

        Ok(base_len as usize + delta_added - delta_removed)
    }

    /// Check if manifest is empty
    pub fn is_empty(&self) -> LmdbResult<bool> {
        Ok(self.len()? == 0)
    }

    /// Iterate over all entries (base + delta merged)
    ///
    /// Note: This is an expensive operation for large manifests
    pub fn iter(&self) -> LmdbResult<Vec<(String, ManifestEntry)>> {
        let rtxn = self.env.read_txn()?;
        let mut result = Vec::new();
        let mut deleted_hashes = std::collections::HashSet::new();

        // Collect delta deletions
        for entry in self.delta.iter() {
            if matches!(entry.value(), DeltaEntry::Deleted) {
                deleted_hashes.insert(*entry.key());
            }
        }

        // Add delta modifications first
        for entry in self.delta.iter() {
            if let DeltaEntry::Modified(manifest_entry) = entry.value() {
                if let Some(path_ref) = self.delta_paths.get(entry.key()) {
                    result.push((path_ref.value().clone(), manifest_entry.clone()));
                }
            }
        }

        // Add base entries not in delta
        let mut iter = self.entries_db.iter(&rtxn)?;
        while let Some(Ok((hash_bytes, entry))) = iter.next() {
            let hash: PathHash = hash_bytes.try_into().unwrap_or([0u8; 32]);
            if !self.delta.contains_key(&hash) && !deleted_hashes.contains(&hash) {
                if let Some(path) = self.paths_db.get(&rtxn, &hash)? {
                    result.push((path.to_string(), entry));
                }
            }
        }

        Ok(result)
    }

    /// Sync/flush LMDB to disk
    pub fn sync(&self) -> LmdbResult<()> {
        self.env.force_sync()?;
        Ok(())
    }

    /// Get environment statistics
    pub fn stats(&self) -> LmdbResult<ManifestStats> {
        let entries = self.iter()?;

        let mut file_count = 0u64;
        let mut dir_count = 0u64;
        let mut total_size = 0u64;
        let mut tier1_count = 0u64;
        let mut tier2_count = 0u64;

        for (_, entry) in &entries {
            if entry.vnode.is_dir() {
                dir_count += 1;
            } else {
                file_count += 1;
                total_size += entry.vnode.size;
            }

            match entry.tier {
                AssetTier::Tier1Immutable => tier1_count += 1,
                AssetTier::Tier2Mutable => tier2_count += 1,
            }
        }

        Ok(ManifestStats {
            file_count,
            dir_count,
            total_size,
            tier1_count,
            tier2_count,
        })
    }
}

/// Statistics about the LMDB manifest
#[derive(Debug, Clone, Default)]
pub struct ManifestStats {
    pub file_count: u64,
    pub dir_count: u64,
    pub total_size: u64,
    pub tier1_count: u64,
    pub tier2_count: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_lmdb_manifest_insert_get() {
        let temp = TempDir::new().unwrap();
        let manifest = LmdbManifest::open(temp.path().join("manifest")).unwrap();

        let hash = [0xABu8; 32];
        let vnode = VnodeEntry::new_file(hash, 1024, 1706448000, 0o644);

        manifest.insert("/app/main.py", vnode.clone(), AssetTier::Tier2Mutable);

        let retrieved = manifest.get("/app/main.py").unwrap().unwrap();
        assert_eq!(retrieved.vnode.content_hash, hash);
        assert_eq!(retrieved.vnode.size, 1024);
        assert_eq!(retrieved.tier, AssetTier::Tier2Mutable);
    }

    #[test]
    fn test_lmdb_manifest_commit() {
        let temp = TempDir::new().unwrap();
        let manifest = LmdbManifest::open(temp.path().join("manifest")).unwrap();

        let hash = [0xCDu8; 32];
        let vnode = VnodeEntry::new_file(hash, 512, 1706448000, 0o644);

        manifest.insert("/test.txt", vnode, AssetTier::Tier1Immutable);
        assert_eq!(manifest.len().unwrap(), 1);

        // Commit to base
        manifest.commit().unwrap();

        // Re-open and verify persistence
        drop(manifest);
        let manifest2 = LmdbManifest::open(temp.path().join("manifest")).unwrap();
        let retrieved = manifest2.get("/test.txt").unwrap().unwrap();
        assert_eq!(retrieved.vnode.content_hash, hash);
        assert_eq!(retrieved.tier, AssetTier::Tier1Immutable);
    }

    #[test]
    fn test_lmdb_manifest_delta_override() {
        let temp = TempDir::new().unwrap();
        let manifest = LmdbManifest::open(temp.path().join("manifest")).unwrap();

        let hash1 = [0x11u8; 32];
        let hash2 = [0x22u8; 32];

        // Insert and commit
        manifest.insert("/file.txt", VnodeEntry::new_file(hash1, 100, 0, 0o644), AssetTier::Tier2Mutable);
        manifest.commit().unwrap();

        // Override in delta
        manifest.insert("/file.txt", VnodeEntry::new_file(hash2, 200, 0, 0o644), AssetTier::Tier2Mutable);

        // Should see delta version
        let retrieved = manifest.get("/file.txt").unwrap().unwrap();
        assert_eq!(retrieved.vnode.content_hash, hash2);
        assert_eq!(retrieved.vnode.size, 200);
    }

    #[test]
    fn test_lmdb_manifest_remove() {
        let temp = TempDir::new().unwrap();
        let manifest = LmdbManifest::open(temp.path().join("manifest")).unwrap();

        let hash = [0xFFu8; 32];
        manifest.insert("/to_delete.txt", VnodeEntry::new_file(hash, 50, 0, 0o644), AssetTier::Tier2Mutable);
        manifest.commit().unwrap();

        manifest.remove("/to_delete.txt");

        // Should be None due to whiteout
        assert!(manifest.get("/to_delete.txt").unwrap().is_none());
    }

    #[test]
    fn test_tier_classification() {
        assert_eq!(AssetTier::default(), AssetTier::Tier2Mutable);
    }
}
