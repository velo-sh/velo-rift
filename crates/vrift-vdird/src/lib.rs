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
//! - Protocol: rkyv-serialized VeloRequest/VeloResponse

pub mod commands;
pub mod ignore;
pub mod ingest;
pub mod journal;
pub mod scan;
pub mod socket;
pub mod state;
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
    /// Path to LMDB manifest
    pub manifest_path: PathBuf,
}

impl ProjectConfig {
    /// Create config from project root path
    pub fn from_project_root(project_root: PathBuf) -> Self {
        let project_id = Self::hash_path(&project_root);
        let vrift_home = dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("/tmp"))
            .join(".vrift");

        // VDir path: use /dev/shm on Linux (fast), tmpdir on macOS
        #[cfg(target_os = "linux")]
        let vdir_path = PathBuf::from(format!("/dev/shm/vrift_vdir_{}", &project_id[..16]));

        #[cfg(not(target_os = "linux"))]
        let vdir_path = {
            let vdir_dir = vrift_home.join("vdir");
            std::fs::create_dir_all(&vdir_dir).ok();
            vdir_dir.join(format!("{}.vdir", &project_id[..16]))
        };

        Self {
            project_root: project_root.clone(),
            project_id: project_id.clone(),
            vdir_path,
            socket_path: vrift_home
                .join("sockets")
                .join(format!("{}.sock", &project_id[..16])),
            staging_base: project_root.join(".vrift").join("staging"),
            cas_path: std::env::var("VR_THE_SOURCE")
                .map(PathBuf::from)
                .unwrap_or_else(|_| vrift_home.join("the_source")),
            manifest_path: vrift_config::path::get_manifest_db_path(&project_id)
                .unwrap_or_else(|| project_root.join(".vrift").join("manifest.lmdb")),
        }
    }

    /// Generate project ID from path (BLAKE3 hash via vrift-config)
    fn hash_path(path: &PathBuf) -> String {
        vrift_config::path::compute_project_id(path)
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

    // Cleanup orphan staging files (max age: 1 hour)
    match state::cleanup_orphan_staging(&config.staging_base, 3600) {
        Ok(0) => {}
        Ok(count) => info!(count, "Cleaned orphan staging files"),
        Err(e) => tracing::warn!(error = %e, "Failed to cleanup orphan staging files"),
    }

    // Initialize VDir mmap
    let vdir = vdir::VDir::create_or_open(&config.vdir_path)?;
    info!(path = %config.vdir_path.display(), "VDir mmap initialized");

    // Initialize reingest journal for crash recovery
    let journal_path = config
        .project_root
        .join(".vrift")
        .join("reingest_journal.bin");
    let mut reingest_journal = journal::ReingestJournal::open(&journal_path)
        .map_err(|e| anyhow::anyhow!("Failed to open reingest journal: {}", e))?;

    // Cleanup stale journal entries (older than 1 hour)
    if let Err(e) = reingest_journal.cleanup_stale(3600) {
        tracing::warn!(error = %e, "Failed to cleanup stale journal entries");
    }

    // Log recoverable entries (entries with CAS hash but incomplete VDir update)
    let recoverable = reingest_journal.recoverable_entries();
    if !recoverable.is_empty() {
        tracing::warn!(
            count = recoverable.len(),
            "Found recoverable reingest entries from previous crash - manual recovery may be needed"
        );
        for entry in recoverable {
            tracing::warn!(
                vpath = %entry.vpath,
                has_cas_hash = entry.cas_hash.is_some(),
                "Recoverable reingest entry"
            );
        }
    }
    info!(path = %journal_path.display(), pending = reingest_journal.len(), "Reingest journal initialized");

    // RFC-0039: Initialize LMDB manifest for Live Ingest
    let manifest_path = &config.manifest_path;
    std::fs::create_dir_all(manifest_path.parent().unwrap())?;
    let manifest = std::sync::Arc::new(
        vrift_manifest::lmdb::LmdbManifest::open(manifest_path)
            .map_err(|e| anyhow::anyhow!("Failed to open manifest: {}", e))?,
    );
    info!(path = %manifest_path.display(), "LMDB manifest initialized");

    // P0: Load persistent state (last_scan time)
    let state_path = state::state_path(&config.project_root);
    let mut daemon_state = state::DaemonState::load(&state_path);
    let last_scan = daemon_state.last_scan();
    info!(
        last_scan_secs = daemon_state.last_scan_secs,
        "Loaded daemon state"
    );

    // RFC-0039: Create ingest channel (fixed-size for backpressure)
    let (ingest_tx, ingest_rx) = mpsc::channel::<watch::IngestEvent>(4096);

    // Initialize CAS store (TheSource™)
    let cas = vrift_cas::CasStore::default_location()
        .map_err(|e| anyhow::anyhow!("Failed to initialize CAS: {}", e))?;
    info!(root = %cas.root().display(), "CAS store initialized");

    // Phase 1: Start consumer FIRST (consumer-first pattern)
    let ingest_queue = ingest::IngestQueue::new(ingest_rx);
    let handler = std::sync::Arc::new(ingest::IngestHandler::new(
        config.project_root.clone(),
        manifest.clone(),
        cas,
    ));
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
    let state_path_clone = state_path.clone();
    tokio::spawn(async move {
        let count = scan::run_compensation_scan(scan_root, last_scan, scan_tx).await;

        // P0: Update last_scan after successful scan
        if count > 0 || last_scan == std::time::SystemTime::UNIX_EPOCH {
            let mut state = state::DaemonState::load(&state_path_clone);
            state.update_last_scan();
            if let Err(e) = state.save(&state_path_clone) {
                tracing::warn!(error = %e, "Failed to save daemon state after scan");
            }
        }
    });
    info!("Compensation scan started");

    // P1: Periodic manifest commit task (every 30 seconds)
    let commit_manifest = manifest.clone();
    let commit_state_path = state_path.clone();
    let commit_handle = tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));
        loop {
            interval.tick().await;

            // Commit delta layer to base layer
            match commit_manifest.commit() {
                Ok(_) => {
                    let mut state = state::DaemonState::load(&commit_state_path);
                    state.update_last_commit();
                    if let Err(e) = commit_manifest.len() {
                        tracing::debug!(error = %e, "Failed to get manifest len");
                    }
                    if let Err(e) = state.save(&commit_state_path) {
                        tracing::warn!(error = %e, "Failed to save state after commit");
                    }
                    tracing::debug!("Periodic manifest commit completed");
                }
                Err(e) => {
                    tracing::warn!(error = %e, "Periodic manifest commit failed");
                }
            }
        }
    });
    info!("Periodic commit task started (30s interval)");

    let socket_handle = socket::run_listener(config, vdir, manifest.clone());

    // Wait for any task to complete, or signal for graceful shutdown
    tokio::select! {
        _ = consumer_handle => {
            info!("Consumer exited");
        }
        _ = watch_handle => {
            info!("Watch exited");
        }
        _ = commit_handle => {
            info!("Commit task exited");
        }
        result = socket_handle => {
            result?;
        }
        _ = tokio::signal::ctrl_c() => {
            info!("Received SIGINT/SIGTERM, initiating graceful shutdown...");
        }
    }

    // P0: Save state on shutdown
    daemon_state.update_last_scan();
    if let Err(e) = daemon_state.save(&state_path) {
        tracing::warn!(error = %e, "Failed to save daemon state on shutdown");
    }
    info!("Daemon state saved on shutdown");

    // P1: Final commit on shutdown
    if let Err(e) = manifest.commit() {
        tracing::warn!(error = %e, "Failed to commit manifest on shutdown");
    }
    info!("Manifest committed on shutdown");

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_respects_vr_the_source() {
        let root = PathBuf::from("/tmp/test_project");

        // Case 1: Default (unset)
        // Ensure var is unset for this test
        unsafe { std::env::remove_var("VR_THE_SOURCE") };
        let config_default = ProjectConfig::from_project_root(root.clone());
        assert!(config_default.cas_path.ends_with("the_source"));

        // Case 2: Set VR_THE_SOURCE
        let custom_path = "/tmp/custom_cas";
        unsafe { std::env::set_var("VR_THE_SOURCE", custom_path) };
        let config_custom = ProjectConfig::from_project_root(root);
        assert_eq!(config_custom.cas_path, PathBuf::from(custom_path));

        // Cleanup
        unsafe { std::env::remove_var("VR_THE_SOURCE") };
    }
}
