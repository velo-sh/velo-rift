//! RFC-0039: Unified Ingest Queue (Single Consumer Pattern)
//!
//! All ingest events from L1 (Shim IPC), L2 (FS Watch), and L3 (Compensation Scan)
//! flow through this single queue for serialized, conflict-free processing.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
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
        let config = vrift_config::config();
        let dedup_window = Duration::from_millis(config.ingest.dedup_window_ms);

        Self {
            rx,
            state: AtomicU8::new(IngestState::Init as u8),
            recent: HashSet::new(),
            dedup_window,
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

/// Handler that processes ingest events and updates manifest + VDir
pub struct IngestHandler {
    project_root: std::path::PathBuf,
    manifest: std::sync::Arc<vrift_manifest::lmdb::LmdbManifest>,
    cas: vrift_cas::CasStore,
    vdir: Option<Arc<Mutex<crate::vdir::VDir>>>,
    vfs_prefix: String,
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
            vdir: None,
            vfs_prefix: "/vrift".to_string(), // Default, should be updated via with_vfs_prefix
        }
    }

    /// Set VFS virtual prefix (RFC-0050)
    pub fn with_vfs_prefix(mut self, prefix: String) -> Self {
        self.vfs_prefix = prefix;
        self
    }

    /// Set VDir handle for dual-write (manifest + VDir mmap)
    pub fn with_vdir(mut self, vdir: Arc<Mutex<crate::vdir::VDir>>) -> Self {
        self.vdir = Some(vdir);
        self
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
        let tier = self.classify_tier(&rel_path);

        // Zero-copy ingest: reflink → hardlink → copy (RFC-0040)
        use vrift_cas::ingest_solid_tier2;

        match ingest_solid_tier2(path, self.cas.root()) {
            Ok(result) => {
                // Get metadata for manifest entry
                match std::fs::metadata(path) {
                    Ok(meta) => {
                        use std::os::unix::fs::MetadataExt;

                        let vnode = vrift_ipc::VnodeEntry {
                            content_hash: result.hash,
                            size: result.size,
                            mtime: meta.mtime() as u64 * 1_000_000_000 + meta.mtime_nsec() as u64,
                            mode: meta.mode(),
                            flags: 0,
                            _pad: 0,
                        };

                        // Insert into manifest with classified tier
                        self.manifest.insert(&rel_path, vnode, tier);

                        // Also update VDir mmap so inception layer fast-path sees it
                        if let Some(ref vdir_lock) = self.vdir {
                            use crate::vdir::{fnv1a_hash, VDirEntry};
                            let vdir_entry = VDirEntry {
                                path_hash: fnv1a_hash(&rel_path),
                                cas_hash: result.hash,
                                size: result.size,
                                mtime_sec: meta.mtime(),
                                mtime_nsec: meta.mtime_nsec() as u32,
                                mode: meta.mode(),
                                flags: 0,
                                path_offset: 0,
                                path_len: 0,
                                ..Default::default()
                            };
                            if let Ok(mut vdir) = vdir_lock.lock() {
                                if let Err(e) = vdir.upsert_with_path(vdir_entry, &rel_path) {
                                    tracing::warn!(error = %e, path = %rel_path, "VDir upsert failed during ingest");
                                }
                            }
                        }

                        info!(
                            path = %rel_path,
                            size = result.size,
                            tier = ?tier,
                            hash = %vrift_cas::CasStore::hash_to_hex(&result.hash)[..8],
                            was_new = result.was_new,
                            "Ingest: file stored to CAS + VDir (zero-copy)"
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

    /// Classify tier based on config patterns
    fn classify_tier(&self, path: &str) -> vrift_manifest::lmdb::AssetTier {
        let config = vrift_config::config();

        // Check Tier1 patterns first (immutable dependencies)
        for pattern in &config.tiers.tier1_patterns {
            if path.contains(pattern.trim_end_matches('/')) {
                return vrift_manifest::lmdb::AssetTier::Tier1Immutable;
            }
        }

        // Check Tier2 patterns (mutable build outputs)
        for pattern in &config.tiers.tier2_patterns {
            if path.contains(pattern.trim_end_matches('/')) {
                return vrift_manifest::lmdb::AssetTier::Tier2Mutable;
            }
        }

        // Default to Tier2 for unclassified files
        vrift_manifest::lmdb::AssetTier::Tier2Mutable
    }

    fn handle_dir_created(&self, path: &std::path::Path) {
        let rel_path = self.to_manifest_key(path);

        match std::fs::metadata(path) {
            Ok(meta) => {
                use std::os::unix::fs::MetadataExt;

                let vnode = vrift_ipc::VnodeEntry {
                    content_hash: [0u8; 32], // Directories have empty hash
                    size: 0,
                    mtime: meta.mtime() as u64 * 1_000_000_000 + meta.mtime_nsec() as u64,
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
                    mtime: meta.mtime() as u64 * 1_000_000_000 + meta.mtime_nsec() as u64,
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

    /// Convert absolute path to manifest key (RFC-0050: prefix + relative path)
    fn to_manifest_key(&self, path: &std::path::Path) -> String {
        let rel_path = path
            .strip_prefix(&self.project_root)
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|_| path.to_string_lossy().into_owned());

        let prefix = self.vfs_prefix.trim_end_matches('/');
        format!("{}/{}", prefix, rel_path.trim_start_matches('/'))
    }
}

/// Consumer task that processes ingest events with batching and async CAS
pub async fn run_consumer(mut queue: IngestQueue, handler: std::sync::Arc<IngestHandler>) {
    use tokio::time::timeout;

    // Extract config values in a scoped block to drop RwLockReadGuard before await
    let (batch_size, batch_timeout) = {
        let config = vrift_config::config();
        (
            config.ingest.batch_size,
            Duration::from_millis(config.ingest.batch_timeout_ms),
        )
    };

    info!(batch_size, batch_timeout_ms = ?batch_timeout, "Ingest consumer started with batching");

    // Mark consumer ready
    queue.transition(IngestState::ConsumerReady);

    let mut batch: Vec<IngestEvent> = Vec::with_capacity(batch_size);

    loop {
        // Collect events up to batch_size or timeout
        let deadline = Instant::now() + batch_timeout;

        while batch.len() < batch_size {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }

            match timeout(remaining, queue.next()).await {
                Ok(Some(event)) => {
                    debug!(?event, "Queued ingest event for batch");
                    batch.push(event);
                }
                Ok(None) => {
                    // Channel closed, process remaining and exit
                    if !batch.is_empty() {
                        process_batch(&handler, std::mem::take(&mut batch)).await;
                    }
                    info!("Ingest consumer stopped (channel closed)");
                    return;
                }
                Err(_) => {
                    // Timeout, process current batch
                    break;
                }
            }
        }

        // Process batch if not empty
        if !batch.is_empty() {
            let batch_len = batch.len();
            debug!(batch_len, "Processing ingest batch");
            process_batch(&handler, std::mem::take(&mut batch)).await;
        }
    }
}

/// Process a batch of ingest events with async CAS storage
async fn process_batch(handler: &std::sync::Arc<IngestHandler>, events: Vec<IngestEvent>) {
    use tokio::task::JoinSet;

    let mut join_set: JoinSet<()> = JoinSet::new();

    for event in events {
        let handler = handler.clone();
        join_set.spawn(async move {
            // Use spawn_blocking for CPU/IO-bound CAS operations
            let _ = tokio::task::spawn_blocking(move || {
                handler.handle(event);
            })
            .await;
        });
    }

    // Wait for all tasks in batch to complete
    while join_set.join_next().await.is_some() {}
}
