//! # velo-lock
//!
//! Parser for `vrift.lock` files, based on the Velo Technical Architecture §3.1.
//!
//! This crate handles the translation of high-level dependency intent (packages)
//! into physical storage capabilities (CAS trees).

use std::collections::HashMap;
use std::fs::File;
use std::io::BufReader;
use std::path::Path;

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum LockError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON parse error: {0}")]
    Json(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, LockError>;

/// Top-level vrift.lock structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VeloLock {
    pub meta: LockMeta,
    pub roots: HashMap<String, RootEntry>,
    pub packages: HashMap<String, PackageEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LockMeta {
    pub engine: String,
    pub generated_at: u64,
    pub uv_lock_hash: String,
    pub target_platform: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RootEntry {
    pub mount_point: String,
    pub tree_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageEntry {
    pub version: String,
    pub source_tree: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dist_info_tree: Option<String>,
}

impl VeloLock {
    /// Load a lockfile from a JSON file
    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self> {
        let file = File::open(path)?;
        let reader = BufReader::new(file);
        let lock = serde_json::from_reader(reader)?;
        Ok(lock)
    }

    /// Save the lockfile to a JSON file
    pub fn save<P: AsRef<Path>>(&self, path: P) -> Result<()> {
        let file = File::create(path)?;
        serde_json::to_writer_pretty(file, self)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    #[test]
    fn test_lock_roundtrip() {
        let lock = VeloLock {
            meta: LockMeta {
                engine: "vrift-native-v1".to_string(),
                generated_at: 1706448000,
                uv_lock_hash: "sha256:7f8a...".to_string(),
                target_platform: "linux_x86_64_gnu".to_string(),
            },
            roots: HashMap::from([(
                "site_packages".to_string(),
                RootEntry {
                    mount_point: "/app/.venv/lib/python3.11/site-packages".to_string(),
                    tree_hash: "tree:root_site_packages_merged_hash".to_string(),
                },
            )]),
            packages: HashMap::from([
                (
                    "numpy".to_string(),
                    PackageEntry {
                        version: "1.26.0".to_string(),
                        source_tree: "tree:numpy_1.26.0_hash".to_string(),
                        dist_info_tree: Some("tree:numpy_1.26.0_dist_info_hash".to_string()),
                    },
                ),
                (
                    "pandas".to_string(),
                    PackageEntry {
                        version: "2.1.0".to_string(),
                        source_tree: "tree:pandas_2.1.0_hash".to_string(),
                        dist_info_tree: None,
                    },
                ),
            ]),
        };

        let file = NamedTempFile::new().unwrap();
        lock.save(file.path()).unwrap();

        let loaded = VeloLock::load(file.path()).unwrap();

        assert_eq!(loaded.meta.engine, "vrift-native-v1");
        assert_eq!(loaded.packages.len(), 2);
        assert_eq!(loaded.packages["numpy"].version, "1.26.0");
        assert!(loaded.packages["pandas"].dist_info_tree.is_none());
    }
}
