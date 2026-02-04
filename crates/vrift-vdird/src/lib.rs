//! # vrift-vdird
//!
//! Per-project daemon for vrift virtual directory management.
//!
//! ## Architecture
//!
//! Each project has its own `vdir_d` process that:
//! - Manages the VDir mmap file for that project
//! - Handles staging file ingestion (CMD_COMMIT)
//! - Updates VDir entries atomically
//!
//! ## Communication
//!
//! Clients (InceptionLayer) communicate via Unix Domain Socket:
//! - Socket path: `~/.vrift/sockets/<project_id>.sock`
//! - Protocol: bincode-serialized VeloRequest/VeloResponse

pub mod commands;
pub mod socket;
pub mod vdir;

use anyhow::Result;
use std::path::PathBuf;
use tracing::info;

/// Project configuration for a vdir_d instance
#[derive(Debug, Clone)]
pub struct ProjectConfig {
    /// Absolute path to project root
    pub project_root: PathBuf,
    /// Project ID (hash of project_root)
    pub project_id: String,
    /// Path to VDir mmap file
    pub vdir_path: PathBuf,
    /// Path to UDS socket
    pub socket_path: PathBuf,
    /// Path to staging directory
    pub staging_base: PathBuf,
    /// Path to CAS storage
    pub cas_path: PathBuf,
}

impl ProjectConfig {
    /// Create config from project root path
    pub fn from_project_root(project_root: PathBuf) -> Self {
        let project_id = Self::hash_path(&project_root);
        let vrift_home = dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("/tmp"))
            .join(".vrift");

        Self {
            project_root: project_root.clone(),
            project_id: project_id.clone(),
            vdir_path: PathBuf::from(format!("/dev/shm/vrift_vdir_{}", &project_id[..16])),
            socket_path: vrift_home
                .join("sockets")
                .join(format!("{}.sock", &project_id[..16])),
            staging_base: project_root.join(".vrift").join("staging"),
            cas_path: vrift_home.join("cas"),
        }
    }

    /// Generate project ID from path (FNV-1a hash)
    fn hash_path(path: &PathBuf) -> String {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        path.hash(&mut hasher);
        format!("{:016x}", hasher.finish())
    }
}

/// Main daemon entry point
pub async fn run_daemon(config: ProjectConfig) -> Result<()> {
    info!(
        project_root = %config.project_root.display(),
        project_id = %config.project_id,
        "Starting vdir_d"
    );

    // Ensure directories exist
    std::fs::create_dir_all(config.socket_path.parent().unwrap())?;
    std::fs::create_dir_all(&config.staging_base)?;
    std::fs::create_dir_all(&config.cas_path)?;

    // Initialize VDir mmap
    let vdir = vdir::VDir::create_or_open(&config.vdir_path)?;
    info!(path = %config.vdir_path.display(), "VDir mmap initialized");

    // Start socket listener
    socket::run_listener(config, vdir).await
}
