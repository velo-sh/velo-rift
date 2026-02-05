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
pub mod ingest;
pub mod scan;
pub mod socket;
pub mod vdir;
pub mod watch;

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
    use tokio::sync::mpsc;

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

    // RFC-0039: Initialize LMDB manifest for Live Ingest
    let manifest_path = config.project_root.join(".vrift").join("manifest.lmdb");
    std::fs::create_dir_all(&manifest_path)?;
    let manifest = std::sync::Arc::new(
        vrift_manifest::lmdb::LmdbManifest::open(&manifest_path)
            .map_err(|e| anyhow::anyhow!("Failed to open manifest: {}", e))?,
    );
    info!(path = %manifest_path.display(), "LMDB manifest initialized");

    // RFC-0039: Create ingest channel (fixed-size for backpressure)
    let (ingest_tx, ingest_rx) = mpsc::channel::<watch::IngestEvent>(4096);

    // Phase 1: Start consumer FIRST (consumer-first pattern)
    let ingest_queue = ingest::IngestQueue::new(ingest_rx);
    let handler = ingest::IngestHandler::new(config.project_root.clone(), manifest.clone());
    let consumer_handle = tokio::spawn(async move {
        ingest::run_consumer(ingest_queue, handler).await;
    });
    info!("Ingest consumer started (consumer-first pattern)");

    // Phase 2: Start FS Watch producer
    let watch_handle = watch::spawn_watch_task(config.project_root.clone(), ingest_tx.clone());
    info!("FS Watch producer started");

    // Phase 3: Run compensation scan (Layer 3) for offline changes
    let scan_tx = ingest_tx.clone();
    let scan_root = config.project_root.clone();
    tokio::spawn(async move {
        // Use UNIX_EPOCH to catch all files on first run
        // TODO: Load last_scan from persistent state
        let last_scan = std::time::SystemTime::UNIX_EPOCH;
        scan::run_compensation_scan(scan_root, last_scan, scan_tx).await;
    });
    info!("Compensation scan started");
    let socket_handle = socket::run_listener(config, vdir);

    // Wait for all tasks
    tokio::select! {
        _ = consumer_handle => {
            info!("Consumer exited");
        }
        _ = watch_handle => {
            info!("Watch exited");
        }
        result = socket_handle => {
            result?;
        }
    }

    Ok(())
}
