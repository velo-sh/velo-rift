//! RFC-0039: Unified Ingest Queue (Single Consumer Pattern)
//!
//! All ingest events from L1 (Shim IPC), L2 (FS Watch), and L3 (Compensation Scan)
//! flow through this single queue for serialized, conflict-free processing.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tracing::{debug, info};

use crate::watch::IngestEvent;

/// State machine states for Ingest Queue
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum IngestState {
    /// Initial state, loading manifest
    Init = 0,
    /// Manifest loaded, consumer not yet started
    Loading = 1,
    /// Consumer started, ready for producers
    ConsumerReady = 2,
    /// Producers (Watch/Scan) starting
    ProducersStarting = 3,
    /// All systems active
    Active = 4,
    /// Draining queue before shutdown
    Draining = 5,
    /// Stopped
    Stopped = 6,
}

impl From<u8> for IngestState {
    fn from(v: u8) -> Self {
        match v {
            0 => Self::Init,
            1 => Self::Loading,
            2 => Self::ConsumerReady,
            3 => Self::ProducersStarting,
            4 => Self::Active,
            5 => Self::Draining,
            6 => Self::Stopped,
            _ => Self::Init,
        }
    }
}

/// Ingest Queue with deduplication
pub struct IngestQueue {
    /// Event receiver
    rx: mpsc::Receiver<IngestEvent>,
    /// State
    state: AtomicU8,
    /// Recent paths for deduplication (path -> last_seen)
    recent: HashSet<PathBuf>,
    /// Dedup window
    dedup_window: Duration,
    /// Last cleanup time
    last_cleanup: Instant,
}

impl IngestQueue {
    /// Create a new ingest queue
    pub fn new(rx: mpsc::Receiver<IngestEvent>) -> Self {
        Self {
            rx,
            state: AtomicU8::new(IngestState::Init as u8),
            recent: HashSet::new(),
            dedup_window: Duration::from_millis(200),
            last_cleanup: Instant::now(),
        }
    }

    /// Get current state
    pub fn state(&self) -> IngestState {
        IngestState::from(self.state.load(Ordering::Acquire))
    }

    /// Transition to new state (returns false if transition invalid)
    pub fn transition(&self, new_state: IngestState) -> bool {
        let current = self.state();

        // Valid transitions
        let valid = matches!(
            (current, new_state),
            (IngestState::Init, IngestState::Loading)
                | (IngestState::Loading, IngestState::ConsumerReady)
                | (IngestState::ConsumerReady, IngestState::ProducersStarting)
                | (IngestState::ProducersStarting, IngestState::Active)
                | (IngestState::Active, IngestState::Draining)
                | (IngestState::Draining, IngestState::Stopped)
        );

        if valid {
            self.state.store(new_state as u8, Ordering::Release);
            info!(?current, ?new_state, "Ingest queue state transition");
            true
        } else {
            false
        }
    }

    /// Check if path is in recent set (for deduplication)
    fn is_recent(&self, path: &PathBuf) -> bool {
        self.recent.contains(path)
    }

    /// Mark path as recently processed
    fn mark_recent(&mut self, path: PathBuf) {
        self.recent.insert(path);
    }

    /// Cleanup old entries from recent set
    fn cleanup_recent(&mut self) {
        if self.last_cleanup.elapsed() > self.dedup_window {
            self.recent.clear();
            self.last_cleanup = Instant::now();
        }
    }

    /// Process next event (with deduplication)
    pub async fn next(&mut self) -> Option<IngestEvent> {
        self.cleanup_recent();

        loop {
            match self.rx.recv().await {
                Some(event) => {
                    let path = match &event {
                        IngestEvent::FileChanged { path } => path,
                        IngestEvent::DirCreated { path } => path,
                        IngestEvent::Removed { path } => path,
                        IngestEvent::SymlinkCreated { path, .. } => path,
                    };

                    // Dedup check
                    if self.is_recent(path) {
                        debug!(?path, "Dedup: skipping recent path");
                        continue;
                    }

                    self.mark_recent(path.clone());
                    return Some(event);
                }
                None => return None,
            }
        }
    }
}

/// Handler that processes ingest events and updates manifest
pub struct IngestHandler {
    project_root: std::path::PathBuf,
    manifest: std::sync::Arc<vrift_manifest::lmdb::LmdbManifest>,
    cas: vrift_cas::CasStore,
}

impl IngestHandler {
    pub fn new(
        project_root: std::path::PathBuf,
        manifest: std::sync::Arc<vrift_manifest::lmdb::LmdbManifest>,
        cas: vrift_cas::CasStore,
    ) -> Self {
        Self {
            project_root,
            manifest,
            cas,
        }
    }

    /// Process a single ingest event
    pub fn handle(&self, event: IngestEvent) {
        match event {
            IngestEvent::FileChanged { path } => {
                self.handle_file_changed(&path);
            }
            IngestEvent::DirCreated { path } => {
                self.handle_dir_created(&path);
            }
            IngestEvent::Removed { path } => {
                self.handle_removed(&path);
            }
            IngestEvent::SymlinkCreated { path, target } => {
                self.handle_symlink_created(&path, &target);
            }
        }
    }

    fn handle_file_changed(&self, path: &std::path::Path) {
        let rel_path = self.to_manifest_key(path);

        // Store file content to CAS and get hash
        match self.cas.store_file(path) {
            Ok(content_hash) => {
                // Get metadata for manifest entry
                match std::fs::metadata(path) {
                    Ok(meta) => {
                        use std::os::unix::fs::MetadataExt;

                        let vnode = vrift_ipc::VnodeEntry {
                            content_hash,
                            size: meta.size(),
                            mtime: meta.mtime() as u64,
                            mode: meta.mode(),
                            flags: 0,
                            _pad: 0,
                        };

                        // Insert into manifest
                        self.manifest.insert(
                            &rel_path,
                            vnode,
                            vrift_manifest::lmdb::AssetTier::Tier2Mutable,
                        );

                        info!(
                            path = %rel_path,
                            size = meta.size(),
                            hash = %vrift_cas::CasStore::hash_to_hex(&content_hash)[..8],
                            "Ingest: file stored to CAS and registered in manifest"
                        );
                    }
                    Err(e) => {
                        info!(path = %path.display(), error = %e, "Ingest: file metadata not accessible after CAS store");
                    }
                }
            }
            Err(e) => {
                info!(path = %path.display(), error = %e, "Ingest: failed to store file to CAS");
            }
        }
    }

    fn handle_dir_created(&self, path: &std::path::Path) {
        let rel_path = self.to_manifest_key(path);

        match std::fs::metadata(path) {
            Ok(meta) => {
                use std::os::unix::fs::MetadataExt;

                let vnode = vrift_ipc::VnodeEntry {
                    content_hash: [0u8; 32], // Directories have empty hash
                    size: 0,
                    mtime: meta.mtime() as u64,
                    mode: meta.mode(),
                    flags: 1, // Directory flag
                    _pad: 0,
                };

                self.manifest.insert(
                    &rel_path,
                    vnode,
                    vrift_manifest::lmdb::AssetTier::Tier2Mutable,
                );

                info!(path = %rel_path, "Ingest: directory registered in manifest");
            }
            Err(e) => {
                info!(path = %path.display(), error = %e, "Ingest: directory not accessible");
            }
        }
    }

    fn handle_removed(&self, path: &std::path::Path) {
        let rel_path = self.to_manifest_key(path);
        self.manifest.remove(&rel_path);
        info!(path = %rel_path, "Ingest: removed from manifest");
    }

    fn handle_symlink_created(&self, path: &std::path::Path, target: &std::path::Path) {
        let rel_path = self.to_manifest_key(path);

        // Use symlink metadata (lstat)
        match std::fs::symlink_metadata(path) {
            Ok(meta) => {
                use std::os::unix::fs::MetadataExt;

                // Store target path as blob in CAS (for symlink reconstruction)
                let target_bytes = target.as_os_str().as_encoded_bytes();
                let content_hash = match self.cas.store(target_bytes) {
                    Ok(hash) => hash,
                    Err(e) => {
                        info!(path = %path.display(), error = %e, "Ingest: failed to store symlink target");
                        return;
                    }
                };

                let vnode = vrift_ipc::VnodeEntry {
                    content_hash,
                    size: target_bytes.len() as u64,
                    mtime: meta.mtime() as u64,
                    mode: 0o777,
                    flags: 2, // Symlink flag
                    _pad: 0,
                };

                self.manifest.insert(
                    &rel_path,
                    vnode,
                    vrift_manifest::lmdb::AssetTier::Tier2Mutable,
                );

                info!(
                    path = %rel_path,
                    target = %target.display(),
                    hash = %vrift_cas::CasStore::hash_to_hex(&content_hash)[..8],
                    "Ingest: symlink stored to CAS and registered in manifest"
                );
            }
            Err(e) => {
                info!(path = %path.display(), error = %e, "Ingest: symlink not accessible");
            }
        }
    }

    /// Convert absolute path to manifest key (relative path)
    fn to_manifest_key(&self, path: &std::path::Path) -> String {
        path.strip_prefix(&self.project_root)
            .map(|p| format!("/{}", p.display()))
            .unwrap_or_else(|_| path.display().to_string())
    }
}

/// Consumer task that processes ingest events
pub async fn run_consumer(mut queue: IngestQueue, handler: IngestHandler) {
    info!("Ingest consumer started");

    // Mark consumer ready
    queue.transition(IngestState::ConsumerReady);

    while let Some(event) = queue.next().await {
        debug!(?event, "Processing ingest event");
        handler.handle(event);
    }

    info!("Ingest consumer stopped");
}
