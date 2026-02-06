//! # Manifest Registry (RFC-0041)
//!
//! Central registry for tracking manifest files across multiple projects.
//! Enables multi-project garbage collection with safe concurrent access.
//!
//! ## Features
//! - UUID-based manifest identification
//! - File-based locking via `flock`
//! - Atomic JSON writes (write-rename pattern)
//! - Stale manifest detection

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::{BufReader, BufWriter};
use std::path::{Path, PathBuf};
use std::time::Duration;
use uuid::Uuid;
use vrift_cas::Blake3Hash;
use vrift_config::path::normalize_or_original;
use vrift_manifest::{LmdbManifest, Manifest};

/// Default lock timeout in seconds
const DEFAULT_LOCK_TIMEOUT_SECS: u64 = 30;

/// Registry format version
const REGISTRY_VERSION: u32 = 1;

/// Status of a registered manifest
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ManifestStatus {
    /// Manifest source file exists and is valid
    Active,
    /// Manifest source file no longer exists
    Stale,
}

/// Entry for a registered manifest
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestEntry {
    /// Path to the original manifest file
    pub source_path: PathBuf,
    /// BLAKE3 hash of source_path for detecting moves
    pub source_path_hash: String,
    /// Project root directory
    pub project_root: PathBuf,
    /// When this manifest was first registered
    pub registered_at: DateTime<Utc>,
    /// Last verification timestamp
    pub last_verified: DateTime<Utc>,
    /// Current status
    pub status: ManifestStatus,
}

/// The manifest registry tracking all known project manifests
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestRegistry {
    /// Format version for compatibility
    pub version: u32,
    /// UUID -> ManifestEntry mapping
    pub manifests: HashMap<String, ManifestEntry>,
}

impl Default for ManifestRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl ManifestRegistry {
    /// Create a new empty registry
    pub fn new() -> Self {
        Self {
            version: REGISTRY_VERSION,
            manifests: HashMap::new(),
        }
    }

    /// Get the registry directory path (~/.vrift/registry/ or VRIFT_REGISTRY_DIR)
    pub fn registry_dir() -> Result<PathBuf> {
        if let Ok(path) = std::env::var("VRIFT_REGISTRY_DIR") {
            return Ok(PathBuf::from(path));
        }
        let home = dirs::home_dir().context("Failed to get home directory")?;
        let registry_dir = home.join(".vrift").join("registry");
        Ok(registry_dir)
    }

    /// Get the registry file path (~/.vrift/registry/manifests.json)
    pub fn registry_path() -> Result<PathBuf> {
        Ok(Self::registry_dir()?.join("manifests.json"))
    }

    /// Get the lock file path
    fn lock_path() -> Result<PathBuf> {
        Ok(Self::registry_dir()?.join(".lock"))
    }

    /// Ensure the registry directory exists with proper permissions
    fn ensure_registry_dir() -> Result<PathBuf> {
        let registry_dir = Self::registry_dir()?;
        if !registry_dir.exists() {
            fs::create_dir_all(&registry_dir)?;
            // Set 0700 permissions (owner-only) on Unix
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(&registry_dir, fs::Permissions::from_mode(0o700))?;
            }
        }
        Ok(registry_dir)
    }

    /// Acquire an exclusive lock on the registry
    ///
    /// The lock is held until the returned `File` is dropped.
    pub fn acquire_lock() -> Result<File> {
        Self::ensure_registry_dir()?;
        let lock_path = Self::lock_path()?;
        let lock_file = File::create(&lock_path)?;

        // Get timeout from env or use default
        let timeout_secs: u64 = std::env::var("VRIFT_LOCK_TIMEOUT")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(DEFAULT_LOCK_TIMEOUT_SECS);

        // Try to acquire exclusive lock with timeout
        let start = std::time::Instant::now();
        let timeout = Duration::from_secs(timeout_secs);

        loop {
            match lock_file.try_lock_exclusive() {
                Ok(()) => return Ok(lock_file),
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    if start.elapsed() >= timeout {
                        anyhow::bail!(
                            "Timeout waiting for registry lock after {}s. \
                             Another vrift process may be running.",
                            timeout_secs
                        );
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
                Err(e) => return Err(e.into()),
            }
        }
    }

    /// Load registry from disk, or create empty if not exists
    pub fn load_or_create() -> Result<Self> {
        let path = Self::registry_path()?;
        if path.exists() {
            Self::load(&path)
        } else {
            Ok(Self::new())
        }
    }

    /// Load registry from a specific path
    pub fn load(path: &Path) -> Result<Self> {
        let file = File::open(path).context("Failed to open registry file")?;
        let reader = BufReader::new(file);
        let registry: ManifestRegistry =
            serde_json::from_reader(reader).context("Failed to parse registry JSON")?;
        Ok(registry)
    }

    /// Save registry to disk using atomic write-rename pattern
    pub fn save(&self) -> Result<()> {
        Self::ensure_registry_dir()?;
        let path = Self::registry_path()?;
        let tmp_path = path.with_extension("json.tmp");

        // Write to temp file
        let file = File::create(&tmp_path).context("Failed to create temp registry file")?;
        let writer = BufWriter::new(&file);
        serde_json::to_writer_pretty(writer, self).context("Failed to serialize registry")?;

        // Ensure data is on disk
        file.sync_all().context("Failed to sync registry file")?;

        // Atomic rename
        fs::rename(&tmp_path, &path).context("Failed to rename registry file")?;

        Ok(())
    }

    /// Register a new manifest in the registry
    ///
    /// Returns the assigned UUID
    pub fn register_manifest(
        &mut self,
        manifest_path: &Path,
        project_root: &Path,
    ) -> Result<String> {
        let canonical_manifest = normalize_or_original(manifest_path);
        let canonical_root = normalize_or_original(project_root);

        // Check if already registered by source path
        for (uuid, entry) in &self.manifests {
            if entry.source_path == canonical_manifest {
                // Update last_verified and return existing UUID
                let uuid = uuid.clone();
                if let Some(e) = self.manifests.get_mut(&uuid) {
                    e.last_verified = Utc::now();
                    e.status = ManifestStatus::Active;
                }
                return Ok(uuid);
            }
        }

        // Generate new UUID
        let uuid = Uuid::new_v4().to_string();
        let path_hash = format!(
            "blake3:{}",
            vrift_cas::CasStore::hash_to_hex(&vrift_cas::CasStore::compute_hash(
                canonical_manifest.to_string_lossy().as_bytes()
            ))
        );

        let now = Utc::now();
        let entry = ManifestEntry {
            source_path: canonical_manifest,
            source_path_hash: path_hash,
            project_root: canonical_root,
            registered_at: now,
            last_verified: now,
            status: ManifestStatus::Active,
        };

        self.manifests.insert(uuid.clone(), entry);
        Ok(uuid)
    }

    /// Verify all manifests and update their status
    ///
    /// Returns count of (active, stale) manifests
    pub fn verify_all(&mut self) -> (usize, usize) {
        let mut active = 0;
        let mut stale = 0;

        for entry in self.manifests.values_mut() {
            if entry.source_path.exists() {
                entry.status = ManifestStatus::Active;
                entry.last_verified = Utc::now();
                active += 1;
            } else {
                entry.status = ManifestStatus::Stale;
                stale += 1;
            }
        }

        (active, stale)
    }

    /// Remove all stale manifest entries
    ///
    /// Returns the number of entries removed
    pub fn prune_stale(&mut self) -> usize {
        let stale_uuids: Vec<String> = self
            .manifests
            .iter()
            .filter(|(_, e)| e.status == ManifestStatus::Stale)
            .map(|(uuid, _)| uuid.clone())
            .collect();

        let count = stale_uuids.len();
        for uuid in stale_uuids {
            self.manifests.remove(&uuid);
        }
        count
    }

    /// Get all blob hashes referenced by all active manifests
    pub fn get_all_blob_hashes(&self) -> Result<HashSet<Blake3Hash>> {
        let mut hashes = HashSet::new();

        for entry in self.manifests.values() {
            if entry.status != ManifestStatus::Active {
                continue;
            }

            if !entry.source_path.exists() {
                continue;
            }

            if entry.source_path.is_dir() {
                // RFC-0039: Load LMDB manifest
                let lmdb = LmdbManifest::open(&entry.source_path).with_context(|| {
                    format!("Failed to open LMDB manifest at {:?}", entry.source_path)
                })?;
                let entries = lmdb.iter().with_context(|| {
                    format!("Failed to iterate LMDB manifest at {:?}", entry.source_path)
                })?;
                for (_, m_entry) in entries {
                    hashes.insert(m_entry.vnode.content_hash);
                }
            } else {
                // In-memory manifest (rkyv format)
                let manifest = Manifest::load(&entry.source_path)
                    .with_context(|| format!("Failed to load manifest: {:?}", entry.source_path))?;
                for (_, vnode) in manifest.iter() {
                    hashes.insert(vnode.content_hash);
                }
            }
        }

        Ok(hashes)
    }

    /// Get list of active manifests
    pub fn active_manifests(&self) -> Vec<(&String, &ManifestEntry)> {
        self.manifests
            .iter()
            .filter(|(_, e)| e.status == ManifestStatus::Active)
            .collect()
    }

    /// Get list of stale manifests
    pub fn stale_manifests(&self) -> Vec<(&String, &ManifestEntry)> {
        self.manifests
            .iter()
            .filter(|(_, e)| e.status == ManifestStatus::Stale)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_registry_new() {
        let registry = ManifestRegistry::new();
        assert_eq!(registry.version, REGISTRY_VERSION);
        assert!(registry.manifests.is_empty());
    }

    #[test]
    fn test_registry_save_load() {
        let temp = TempDir::new().unwrap();
        let registry_path = temp.path().join("manifests.json");

        let mut registry = ManifestRegistry::new();

        // Create a fake manifest file
        let manifest_path = temp.path().join("test.manifest");
        std::fs::write(&manifest_path, b"dummy").unwrap();

        registry
            .register_manifest(&manifest_path, temp.path())
            .unwrap();

        // Save
        let file = File::create(&registry_path).unwrap();
        let writer = BufWriter::new(&file);
        serde_json::to_writer_pretty(writer, &registry).unwrap();
        file.sync_all().unwrap();

        // Load
        let loaded = ManifestRegistry::load(&registry_path).unwrap();
        assert_eq!(loaded.manifests.len(), 1);
    }

    #[test]
    fn test_registry_stale_detection() {
        let temp = TempDir::new().unwrap();
        let manifest_path = temp.path().join("test.manifest");
        std::fs::write(&manifest_path, b"dummy").unwrap();

        let mut registry = ManifestRegistry::new();
        registry
            .register_manifest(&manifest_path, temp.path())
            .unwrap();

        // Initially active
        let (active, stale) = registry.verify_all();
        assert_eq!(active, 1);
        assert_eq!(stale, 0);

        // Delete the manifest file
        std::fs::remove_file(&manifest_path).unwrap();

        // Now should be stale
        let (active, stale) = registry.verify_all();
        assert_eq!(active, 0);
        assert_eq!(stale, 1);

        // Prune stale
        let pruned = registry.prune_stale();
        assert_eq!(pruned, 1);
        assert!(registry.manifests.is_empty());
    }

    #[test]
    fn test_registry_duplicate_registration() {
        let temp = TempDir::new().unwrap();
        let manifest_path = temp.path().join("test.manifest");
        std::fs::write(&manifest_path, b"dummy").unwrap();

        let mut registry = ManifestRegistry::new();
        let uuid1 = registry
            .register_manifest(&manifest_path, temp.path())
            .unwrap();
        let uuid2 = registry
            .register_manifest(&manifest_path, temp.path())
            .unwrap();

        // Should return same UUID
        assert_eq!(uuid1, uuid2);
        assert_eq!(registry.manifests.len(), 1);
    }
}
