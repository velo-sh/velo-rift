//! Command handlers for vdir_d

use crate::vdir::{fnv1a_hash, VDir, VDirEntry, FLAG_DIR};
use crate::ProjectConfig;
use anyhow::Result;
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use tracing::{debug, error, info, warn};
use vrift_ipc::{
    VeloError, VeloErrorKind, VeloRequest, VeloResponse, VnodeEntry, PROTOCOL_VERSION,
};

/// Command handler for vdir_d
pub struct CommandHandler {
    config: ProjectConfig,
    vdir: VDir,
    manifest: std::sync::Arc<vrift_manifest::lmdb::LmdbManifest>,
}

impl CommandHandler {
    pub fn new(
        config: ProjectConfig,
        vdir: VDir,
        manifest: std::sync::Arc<vrift_manifest::lmdb::LmdbManifest>,
    ) -> Self {
        Self {
            config,
            vdir,
            manifest,
        }
    }

    /// Handle incoming request
    pub async fn handle_request(&mut self, request: VeloRequest) -> VeloResponse {
        match request {
            VeloRequest::Handshake {
                client_version,
                protocol_version,
            } => {
                info!(client_version = %client_version, protocol_version, "Handshake");
                VeloResponse::HandshakeAck {
                    server_version: env!("CARGO_PKG_VERSION").to_string(),
                    protocol_version: PROTOCOL_VERSION,
                    compatible: vrift_ipc::is_version_compatible(protocol_version),
                }
            }

            VeloRequest::Status => VeloResponse::StatusAck {
                status: "ready".to_string(),
            },

            VeloRequest::RegisterWorkspace { project_root } => {
                info!(project_root = %project_root, "Workspace registered");
                VeloResponse::RegisterAck {
                    workspace_id: self.config.project_id.clone(),
                    vdird_socket: self.config.socket_path.to_string_lossy().to_string(),
                    vdir_mmap_path: self.config.vdir_path.to_string_lossy().to_string(),
                }
            }

            VeloRequest::ManifestGet { path } => self.handle_manifest_get(&path),

            VeloRequest::ManifestUpsert { path, entry } => {
                self.handle_manifest_upsert(&path, entry)
            }

            VeloRequest::ManifestRemove { path } => self.handle_manifest_remove(&path),

            VeloRequest::ManifestRename { old_path, new_path } => {
                self.handle_manifest_rename(&old_path, &new_path)
            }

            VeloRequest::ManifestUpdateMtime { path, mtime_ns } => {
                self.handle_manifest_update_mtime(&path, mtime_ns)
            }

            VeloRequest::ManifestListDir { path } => self.handle_manifest_list_dir(&path),

            VeloRequest::ManifestReingest { vpath, temp_path } => {
                self.handle_reingest(&vpath, &temp_path).await
            }

            VeloRequest::IngestFullScan {
                path,
                manifest_path,
                threads,
                phantom,
                tier1,
                prefix,
            } => {
                self.handle_ingest_full_scan(
                    &path,
                    &manifest_path,
                    threads,
                    phantom,
                    tier1,
                    prefix.as_deref(),
                )
                .await
            }

            // Not yet implemented - forward to future handlers
            _ => {
                warn!(?request, "Unhandled request type");
                VeloResponse::Error(VeloError::internal("Not implemented"))
            }
        }
    }

    /// Handle ManifestGet
    /// First checks VDir (runtime overlay for COW), then falls back to LMDB (persistent storage)
    fn handle_manifest_get(&self, path: &str) -> VeloResponse {
        let path_hash = fnv1a_hash(path);

        // 1. First check VDir (runtime overlay for COW mutations)
        if let Some(entry) = self.vdir.lookup(path_hash) {
            let vnode = VnodeEntry {
                content_hash: entry.cas_hash,
                size: entry.size,
                mtime: entry.mtime_sec as u64,
                mode: entry.mode,
                flags: entry.flags,
                _pad: 0,
            };
            return VeloResponse::ManifestAck { entry: Some(vnode) };
        }

        // 2. Fallback to LMDB (persistent storage)
        match self.manifest.get(path) {
            Ok(Some(entry)) => {
                debug!(path = %path, "ManifestGet: found in LMDB");
                VeloResponse::ManifestAck {
                    entry: Some(entry.vnode),
                }
            }
            Ok(None) => {
                debug!(path = %path, "ManifestGet: not found in VDir or LMDB");
                VeloResponse::ManifestAck { entry: None }
            }
            Err(e) => {
                warn!(path = %path, error = %e, "ManifestGet: LMDB lookup failed");
                VeloResponse::ManifestAck { entry: None }
            }
        }
    }

    /// Handle ManifestUpsert
    fn handle_manifest_upsert(&mut self, path: &str, entry: VnodeEntry) -> VeloResponse {
        let vdir_entry = VDirEntry {
            path_hash: fnv1a_hash(path),
            cas_hash: entry.content_hash,
            size: entry.size,
            mtime_sec: entry.mtime as i64,
            mtime_nsec: 0,
            mode: entry.mode,
            flags: entry.flags,
            _pad: [0; 3],
        };

        match self.vdir.upsert(vdir_entry) {
            Ok(_) => {
                debug!(path = %path, "Upserted entry");
                VeloResponse::ManifestAck { entry: Some(entry) }
            }
            Err(e) => {
                error!(error = %e, path = %path, "Upsert failed");
                VeloResponse::Error(VeloError::internal(format!("{}", e)))
            }
        }
    }

    /// Handle ManifestRemove
    fn handle_manifest_remove(&mut self, path: &str) -> VeloResponse {
        let path_hash = fnv1a_hash(path);
        if self.vdir.mark_dirty(path_hash, false) {
            // For now, just clear dirty bit. Full deletion would require tombstone.
            debug!(path = %path, "Marked for removal");
            VeloResponse::ManifestAck { entry: None }
        } else {
            VeloResponse::ManifestAck { entry: None }
        }
    }

    /// Handle ManifestRename: remove old path, upsert under new path
    fn handle_manifest_rename(&mut self, old_path: &str, new_path: &str) -> VeloResponse {
        let old_hash = fnv1a_hash(old_path);
        let new_hash = fnv1a_hash(new_path);

        // Lookup old entry (VDir first, then LMDB)
        let old_entry = if let Some(entry) = self.vdir.lookup(old_hash) {
            Some(*entry)
        } else if let Ok(Some(lmdb_entry)) = self.manifest.get(old_path) {
            Some(VDirEntry {
                path_hash: old_hash,
                cas_hash: lmdb_entry.vnode.content_hash,
                size: lmdb_entry.vnode.size,
                mtime_sec: lmdb_entry.vnode.mtime as i64,
                mtime_nsec: 0,
                mode: lmdb_entry.vnode.mode,
                flags: lmdb_entry.vnode.flags,
                _pad: [0; 3],
            })
        } else {
            None
        };

        match old_entry {
            Some(entry) => {
                // Mark old path as removed
                self.vdir.mark_dirty(old_hash, false);

                // Insert under new path hash
                let new_entry = VDirEntry {
                    path_hash: new_hash,
                    ..entry
                };
                match self.vdir.upsert(new_entry) {
                    Ok(_) => {
                        debug!(old = %old_path, new = %new_path, "Manifest rename");
                        VeloResponse::ManifestAck { entry: None }
                    }
                    Err(e) => {
                        error!(error = %e, "Rename upsert failed");
                        VeloResponse::Error(VeloError::internal(format!("{}", e)))
                    }
                }
            }
            None => {
                debug!(path = %old_path, "Rename: source not found, treating as no-op");
                VeloResponse::ManifestAck { entry: None }
            }
        }
    }

    /// Handle ManifestUpdateMtime: update mtime on existing entry
    fn handle_manifest_update_mtime(&mut self, path: &str, mtime_ns: u64) -> VeloResponse {
        let path_hash = fnv1a_hash(path);
        let mtime_sec = (mtime_ns / 1_000_000_000) as i64;
        let mtime_nsec = (mtime_ns % 1_000_000_000) as u32;

        // Look up existing entry (VDir first, then LMDB)
        let existing = if let Some(entry) = self.vdir.lookup(path_hash) {
            Some(*entry)
        } else if let Ok(Some(lmdb_entry)) = self.manifest.get(path) {
            Some(VDirEntry {
                path_hash,
                cas_hash: lmdb_entry.vnode.content_hash,
                size: lmdb_entry.vnode.size,
                mtime_sec: lmdb_entry.vnode.mtime as i64,
                mtime_nsec: 0,
                mode: lmdb_entry.vnode.mode,
                flags: lmdb_entry.vnode.flags,
                _pad: [0; 3],
            })
        } else {
            None
        };

        match existing {
            Some(entry) => {
                let updated = VDirEntry {
                    mtime_sec,
                    mtime_nsec,
                    ..entry
                };
                match self.vdir.upsert(updated) {
                    Ok(_) => {
                        debug!(path = %path, mtime_sec, "Updated mtime");
                        VeloResponse::ManifestAck { entry: None }
                    }
                    Err(e) => {
                        error!(error = %e, "UpdateMtime upsert failed");
                        VeloResponse::Error(VeloError::internal(format!("{}", e)))
                    }
                }
            }
            None => {
                debug!(path = %path, "UpdateMtime: entry not found");
                VeloResponse::ManifestAck { entry: None }
            }
        }
    }

    /// Handle ManifestListDir: list direct children of a directory path
    fn handle_manifest_list_dir(&self, path: &str) -> VeloResponse {
        // Build prefix for direct children lookup
        let prefix = if path.is_empty() || path == "/" {
            String::new()
        } else if path.ends_with('/') {
            path.to_string()
        } else {
            format!("{}/", path)
        };

        let mut entries = Vec::new();
        let mut seen = std::collections::HashSet::new();

        // Query LMDB for all entries, filter by prefix
        if let Ok(all_entries) = self.manifest.iter() {
            for (entry_path, manifest_entry) in &all_entries {
                if !entry_path.starts_with(&prefix) {
                    continue;
                }
                // Extract direct child name (strip prefix, take first component)
                let relative = &entry_path[prefix.len()..];
                let child_name = if let Some(slash_pos) = relative.find('/') {
                    // This is a deeper path → the direct child is a directory
                    let name = &relative[..slash_pos];
                    if !seen.insert(name.to_string()) {
                        continue; // Already seen this directory
                    }
                    entries.push(vrift_ipc::DirEntry {
                        name: name.to_string(),
                        is_dir: true,
                    });
                    continue;
                } else {
                    relative
                };

                if child_name.is_empty() {
                    continue;
                }
                if !seen.insert(child_name.to_string()) {
                    continue;
                }

                let is_dir = manifest_entry.vnode.flags & FLAG_DIR != 0;
                entries.push(vrift_ipc::DirEntry {
                    name: child_name.to_string(),
                    is_dir,
                });
            }
        }

        debug!(path = %path, count = entries.len(), "ListDir");
        VeloResponse::ManifestListAck { entries }
    }

    /// Handle ManifestReingest (CoW commit)
    async fn handle_reingest(&mut self, vpath: &str, temp_path: &str) -> VeloResponse {
        let temp = PathBuf::from(temp_path);

        // 1. Initialize CAS store
        let store = match vrift_cas::CasStore::new(&self.config.cas_path) {
            Ok(s) => s,
            Err(e) => {
                error!(error = %e, "Failed to initialize CAS store");
                return VeloResponse::Error(VeloError::new(
                    VeloErrorKind::Internal,
                    format!("CAS init error: {}", e),
                ));
            }
        };

        // 2. Ingest to CAS via move (atomic & deduplicated)
        let hash_bytes = match store.store_by_move(&temp) {
            Ok(h) => h,
            Err(e) => {
                error!(error = %e, temp = %temp_path, "CAS ingestion failed");
                return VeloResponse::Error(VeloError::new(
                    VeloErrorKind::IngestFailed,
                    format!("Ingest error: {}", e),
                ));
            }
        };

        // 3. Get metadata for the committed file
        let cas_path = store.blob_path_for_hash(&hash_bytes).unwrap();
        let meta = match fs::metadata(&cas_path) {
            Ok(m) => m,
            Err(e) => {
                return VeloResponse::Error(VeloError::io_error(format!("Metadata error: {}", e)));
            }
        };

        // 4. Update VDir
        let entry = VDirEntry {
            path_hash: fnv1a_hash(vpath),
            cas_hash: hash_bytes,
            size: meta.len(),
            mtime_sec: meta.mtime(),
            mtime_nsec: meta.mtime_nsec() as u32,
            mode: meta.mode(),
            flags: if meta.is_dir() { FLAG_DIR } else { 0 },
            _pad: [0; 3],
        };

        if let Err(e) = self.vdir.upsert(entry) {
            return VeloResponse::Error(VeloError::io_error(format!("VDir update error: {}", e)));
        }

        info!(vpath = %vpath, hash = %hex::encode(hash_bytes), "Reingest complete");

        VeloResponse::ManifestAck {
            entry: Some(VnodeEntry {
                content_hash: hash_bytes,
                size: meta.len(),
                mtime: meta.mtime() as u64,
                mode: meta.mode(),
                flags: 0,
                _pad: 0,
            }),
        }
    }

    /// Handle IngestFullScan - unified ingest through daemon
    /// CLI sends this request instead of doing ingest itself
    async fn handle_ingest_full_scan(
        &self,
        path: &str,
        manifest_path: &str,
        threads: Option<usize>,
        phantom: bool,
        tier1: bool,
        prefix: Option<&str>,
    ) -> VeloResponse {
        use std::time::Instant;
        use vrift_cas::{parallel_ingest_with_progress, IngestMode};
        use walkdir::WalkDir;

        let source_path = PathBuf::from(path);
        let manifest_out = PathBuf::from(manifest_path);

        info!(
            path = %path,
            manifest = %manifest_path,
            threads = ?threads,
            phantom = phantom,
            tier1 = tier1,
            "Starting full scan ingest"
        );

        let start = Instant::now();

        // 1. Collect files
        let file_paths: Vec<PathBuf> = WalkDir::new(&source_path)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
            .map(|e| e.path().to_path_buf())
            .collect();

        let total_files = file_paths.len() as u64;
        if total_files == 0 {
            return VeloResponse::IngestAck {
                files: 0,
                blobs: 0,
                new_bytes: 0,
                total_bytes: 0,
                duration_ms: 0,
                manifest_path: manifest_path.to_string(),
            };
        }

        // 2. Determine mode
        let mode = if phantom {
            IngestMode::Phantom
        } else if tier1 {
            IngestMode::SolidTier1
        } else {
            IngestMode::SolidTier2
        };

        // 3. Run parallel ingest
        let results = parallel_ingest_with_progress(
            &file_paths,
            &self.config.cas_path,
            mode,
            threads,
            |_result, _idx| {
                // Progress callback (could stream to client in future)
            },
        );

        // 4. Collect stats
        let mut total_bytes = 0u64;
        let mut new_bytes = 0u64;
        let mut unique_blobs = 0u64;

        for r in results.iter().flatten() {
            total_bytes += r.size;
            if r.was_new {
                unique_blobs += 1;
                new_bytes += r.size;
            }
        }

        let duration = start.elapsed();

        // 5. Build and write manifest (using vrift_manifest if available)
        // For now, just write a simple binary manifest
        if let Err(e) = self.write_manifest(&manifest_out, &source_path, &results, prefix) {
            return VeloResponse::Error(VeloError::io_error(format!(
                "Failed to write manifest: {}",
                e
            )));
        }

        info!(
            files = total_files,
            blobs = unique_blobs,
            new_bytes = new_bytes,
            duration_ms = duration.as_millis() as u64,
            "Full scan ingest complete"
        );

        VeloResponse::IngestAck {
            files: total_files,
            blobs: unique_blobs,
            new_bytes,
            total_bytes,
            duration_ms: duration.as_millis() as u64,
            manifest_path: manifest_path.to_string(),
        }
    }

    /// Write manifest file from ingest results
    fn write_manifest(
        &self,
        manifest_path: &Path,
        source_root: &Path,
        results: &[Result<vrift_cas::IngestResult, vrift_cas::CasError>],
        prefix: Option<&str>,
    ) -> Result<()> {
        let mut manifest = vrift_manifest::Manifest::new();

        for result in results.iter().flatten() {
            // Try to get metadata for mtime/mode
            let (mtime, mode) = match fs::metadata(&result.source_path) {
                Ok(meta) => (meta.mtime() as u64, meta.mode()),
                Err(_) => (0, 0o644), // Fallback
            };

            let entry = VnodeEntry {
                content_hash: result.hash,
                size: result.size,
                mtime,
                mode,
                flags: 0,
                _pad: 0,
            };

            // RFC-0050: Handle prefix
            let canon_source = result
                .source_path
                .canonicalize()
                .unwrap_or_else(|_| result.source_path.clone());
            let canon_root = source_root
                .canonicalize()
                .unwrap_or_else(|_| source_root.to_path_buf());
            let rel = canon_source
                .strip_prefix(&canon_root)
                .unwrap_or(&canon_source);

            let prefix_str = prefix.unwrap_or("");
            let key = if prefix_str == "/" || prefix_str.is_empty() {
                format!("/{}", rel.display())
            } else {
                format!("{}/{}", prefix_str.trim_end_matches('/'), rel.display())
            };

            manifest.insert(&key, entry);
        }

        manifest
            .save(manifest_path)
            .map_err(|e| anyhow::anyhow!("Failed to save manifest: {}", e))?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::tempdir;

    fn create_test_handler() -> (CommandHandler, tempfile::TempDir) {
        let temp = tempdir().unwrap();
        let config = ProjectConfig::from_project_root(temp.path().to_path_buf());

        // Create VDir
        let vdir_path = temp.path().join("test.vdir");
        let vdir = VDir::create_or_open(&vdir_path).unwrap();

        // Create LMDB manifest
        let manifest_path = temp.path().join("manifest.lmdb");
        let manifest =
            std::sync::Arc::new(vrift_manifest::lmdb::LmdbManifest::open(&manifest_path).unwrap());

        (CommandHandler::new(config, vdir, manifest), temp)
    }

    // ==================== Handshake Tests ====================

    #[tokio::test]
    async fn test_handshake_returns_server_version() {
        let (mut handler, _temp) = create_test_handler();

        let response = handler
            .handle_request(VeloRequest::Handshake {
                client_version: "1.0.0".to_string(),
                protocol_version: PROTOCOL_VERSION,
            })
            .await;

        match response {
            VeloResponse::HandshakeAck {
                server_version,
                compatible,
                ..
            } => {
                assert!(!server_version.is_empty());
                assert!(compatible);
            }
            _ => panic!("Expected HandshakeAck"),
        }
    }

    // ==================== Status Tests ====================

    #[tokio::test]
    async fn test_status_returns_ready() {
        let (mut handler, _temp) = create_test_handler();

        let response = handler.handle_request(VeloRequest::Status).await;

        match response {
            VeloResponse::StatusAck { status } => {
                assert_eq!(status, "ready");
            }
            _ => panic!("Expected StatusAck"),
        }
    }

    // ==================== RegisterWorkspace Tests ====================

    #[tokio::test]
    async fn test_register_workspace_returns_id() {
        let (mut handler, _temp) = create_test_handler();

        let response = handler
            .handle_request(VeloRequest::RegisterWorkspace {
                project_root: "/tmp/myproject".to_string(),
            })
            .await;

        match response {
            VeloResponse::RegisterAck { workspace_id, .. } => {
                assert!(!workspace_id.is_empty());
            }
            _ => panic!("Expected RegisterAck"),
        }
    }

    // ==================== ManifestUpsert Tests ====================

    #[tokio::test]
    async fn test_manifest_upsert_and_get() {
        let (mut handler, _temp) = create_test_handler();

        // Upsert
        let entry = VnodeEntry {
            content_hash: [42; 32],
            size: 1000,
            mtime: 1234567890,
            mode: 0o644,
            flags: 0,
            _pad: 0,
        };

        let response = handler
            .handle_request(VeloRequest::ManifestUpsert {
                path: "src/main.rs".to_string(),
                entry: entry.clone(),
            })
            .await;

        assert!(matches!(response, VeloResponse::ManifestAck { .. }));

        // Get
        let response = handler
            .handle_request(VeloRequest::ManifestGet {
                path: "src/main.rs".to_string(),
            })
            .await;

        match response {
            VeloResponse::ManifestAck { entry: Some(e) } => {
                assert_eq!(e.size, 1000);
                assert_eq!(e.content_hash, [42; 32]);
                assert_eq!(e.mtime, 1234567890);
            }
            _ => panic!("Expected ManifestAck with entry"),
        }
    }

    #[tokio::test]
    async fn test_manifest_upsert_overwrites_existing() {
        let (mut handler, _temp) = create_test_handler();

        // First upsert
        handler
            .handle_request(VeloRequest::ManifestUpsert {
                path: "file.txt".to_string(),
                entry: VnodeEntry {
                    content_hash: [0; 32],
                    size: 100,
                    mtime: 0,
                    mode: 0,
                    flags: 0,
                    _pad: 0,
                },
            })
            .await;

        // Second upsert with different size
        handler
            .handle_request(VeloRequest::ManifestUpsert {
                path: "file.txt".to_string(),
                entry: VnodeEntry {
                    content_hash: [0; 32],
                    size: 200,
                    mtime: 0,
                    mode: 0,
                    flags: 0,
                    _pad: 0,
                },
            })
            .await;

        // Verify new size
        let response = handler
            .handle_request(VeloRequest::ManifestGet {
                path: "file.txt".to_string(),
            })
            .await;

        match response {
            VeloResponse::ManifestAck { entry: Some(e) } => {
                assert_eq!(e.size, 200);
            }
            _ => panic!("Expected 200"),
        }
    }

    // ==================== ManifestGet Tests ====================

    #[tokio::test]
    async fn test_manifest_get_nonexistent_returns_none() {
        let (mut handler, _temp) = create_test_handler();

        let response = handler
            .handle_request(VeloRequest::ManifestGet {
                path: "nonexistent.txt".to_string(),
            })
            .await;

        match response {
            VeloResponse::ManifestAck { entry: None } => {}
            _ => panic!("Expected ManifestAck with None"),
        }
    }

    #[tokio::test]
    async fn test_manifest_get_preserves_all_fields() {
        let (mut handler, _temp) = create_test_handler();

        let original = VnodeEntry {
            content_hash: [
                1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23,
                24, 25, 26, 27, 28, 29, 30, 31, 32,
            ],
            size: 123456789,
            mtime: 9876543210,
            mode: 0o755,
            flags: 0x03,
            _pad: 0,
        };

        handler
            .handle_request(VeloRequest::ManifestUpsert {
                path: "test.bin".to_string(),
                entry: original.clone(),
            })
            .await;

        let response = handler
            .handle_request(VeloRequest::ManifestGet {
                path: "test.bin".to_string(),
            })
            .await;

        match response {
            VeloResponse::ManifestAck { entry: Some(e) } => {
                assert_eq!(e.content_hash, original.content_hash);
                assert_eq!(e.size, original.size);
                assert_eq!(e.mtime, original.mtime);
                assert_eq!(e.mode, original.mode);
                assert_eq!(e.flags, original.flags);
            }
            _ => panic!("Expected all fields preserved"),
        }
    }

    // ==================== ManifestRemove Tests ====================

    #[tokio::test]
    async fn test_manifest_remove_clears_dirty() {
        let (mut handler, _temp) = create_test_handler();

        // Insert with dirty flag
        handler
            .handle_request(VeloRequest::ManifestUpsert {
                path: "dirty.txt".to_string(),
                entry: VnodeEntry {
                    content_hash: [0; 32],
                    size: 0,
                    mtime: 0,
                    mode: 0,
                    flags: 0x01, // FLAG_DIRTY
                    _pad: 0,
                },
            })
            .await;

        // Remove (clears dirty in current implementation)
        let response = handler
            .handle_request(VeloRequest::ManifestRemove {
                path: "dirty.txt".to_string(),
            })
            .await;

        assert!(matches!(
            response,
            VeloResponse::ManifestAck { entry: None }
        ));
    }

    // ==================== ManifestReingest Tests ====================

    #[tokio::test]
    async fn test_reingest_hashes_and_stores_content() {
        let (mut handler, temp) = create_test_handler();

        // Create temp file
        let temp_file = temp.path().join("staging").join("test.tmp");
        std::fs::create_dir_all(temp_file.parent().unwrap()).unwrap();
        let mut f = std::fs::File::create(&temp_file).unwrap();
        f.write_all(b"Hello, World!").unwrap();
        drop(f);

        let response = handler
            .handle_request(VeloRequest::ManifestReingest {
                vpath: "hello.txt".to_string(),
                temp_path: temp_file.to_str().unwrap().to_string(),
            })
            .await;

        match response {
            VeloResponse::ManifestAck { entry: Some(e) } => {
                // Verify expected BLAKE3 hash of "Hello, World!"
                let expected = blake3::hash(b"Hello, World!");
                assert_eq!(e.content_hash, *expected.as_bytes());
                assert_eq!(e.size, 13);
            }
            VeloResponse::Error(e) => panic!("Reingest failed: {}", e),
            _ => panic!("Expected ManifestAck"),
        }

        // Verify entry is in VDir
        let response = handler
            .handle_request(VeloRequest::ManifestGet {
                path: "hello.txt".to_string(),
            })
            .await;

        match response {
            VeloResponse::ManifestAck { entry: Some(e) } => {
                assert_eq!(e.size, 13);
            }
            _ => panic!("Entry not found after reingest"),
        }
    }

    #[tokio::test]
    async fn test_reingest_nonexistent_file_returns_error() {
        let (mut handler, _temp) = create_test_handler();

        let response = handler
            .handle_request(VeloRequest::ManifestReingest {
                vpath: "test.txt".to_string(),
                temp_path: "/nonexistent/path/file.tmp".to_string(),
            })
            .await;

        match response {
            VeloResponse::Error(err) => {
                assert!(err.message.contains("Ingest error"));
            }
            _ => panic!("Expected Error for nonexistent file"),
        }
    }

    // ==================== ManifestRename Tests ====================

    #[tokio::test]
    async fn test_manifest_rename_moves_entry() {
        let (mut handler, _temp) = create_test_handler();

        // Insert a file
        let entry = VnodeEntry {
            content_hash: [42; 32],
            size: 1000,
            mtime: 12345,
            mode: 0o644,
            flags: 0,
            _pad: 0,
        };
        handler
            .handle_request(VeloRequest::ManifestUpsert {
                path: "old/path.txt".to_string(),
                entry: entry.clone(),
            })
            .await;

        // Rename it
        let response = handler
            .handle_request(VeloRequest::ManifestRename {
                old_path: "old/path.txt".to_string(),
                new_path: "new/path.txt".to_string(),
            })
            .await;
        assert!(matches!(response, VeloResponse::ManifestAck { .. }));

        // New path should exist with same data
        let response = handler
            .handle_request(VeloRequest::ManifestGet {
                path: "new/path.txt".to_string(),
            })
            .await;
        match response {
            VeloResponse::ManifestAck { entry: Some(e) } => {
                assert_eq!(e.content_hash, [42; 32]);
                assert_eq!(e.size, 1000);
            }
            _ => panic!("Expected entry at new path"),
        }
    }

    #[tokio::test]
    async fn test_manifest_rename_nonexistent_is_noop() {
        let (mut handler, _temp) = create_test_handler();

        let response = handler
            .handle_request(VeloRequest::ManifestRename {
                old_path: "nonexistent.txt".to_string(),
                new_path: "new.txt".to_string(),
            })
            .await;
        assert!(matches!(
            response,
            VeloResponse::ManifestAck { entry: None }
        ));
    }

    // ==================== ManifestUpdateMtime Tests ====================

    #[tokio::test]
    async fn test_manifest_update_mtime() {
        let (mut handler, _temp) = create_test_handler();

        // Insert a file
        handler
            .handle_request(VeloRequest::ManifestUpsert {
                path: "test.txt".to_string(),
                entry: VnodeEntry {
                    content_hash: [0; 32],
                    size: 100,
                    mtime: 1000,
                    mode: 0o644,
                    flags: 0,
                    _pad: 0,
                },
            })
            .await;

        // Update mtime (nanoseconds)
        let new_mtime_ns: u64 = 5_000_000_000 + 500_000_000; // 5.5 seconds
        let response = handler
            .handle_request(VeloRequest::ManifestUpdateMtime {
                path: "test.txt".to_string(),
                mtime_ns: new_mtime_ns,
            })
            .await;
        assert!(matches!(response, VeloResponse::ManifestAck { .. }));

        // Verify mtime was updated
        let response = handler
            .handle_request(VeloRequest::ManifestGet {
                path: "test.txt".to_string(),
            })
            .await;
        match response {
            VeloResponse::ManifestAck { entry: Some(e) } => {
                assert_eq!(e.mtime, 5); // 5 seconds
                assert_eq!(e.size, 100); // size preserved
            }
            _ => panic!("Expected entry"),
        }
    }

    // ==================== ManifestListDir Tests ====================

    #[tokio::test]
    async fn test_manifest_list_dir_empty() {
        let (mut handler, _temp) = create_test_handler();

        let response = handler
            .handle_request(VeloRequest::ManifestListDir {
                path: "nonexistent".to_string(),
            })
            .await;
        match response {
            VeloResponse::ManifestListAck { entries } => {
                assert!(entries.is_empty());
            }
            _ => panic!("Expected ManifestListAck"),
        }
    }

    // ==================== Unhandled Request Tests ====================

    #[tokio::test]
    async fn test_unhandled_request_returns_not_implemented() {
        let (mut handler, _temp) = create_test_handler();

        // CasGet is not yet implemented
        let response = handler
            .handle_request(VeloRequest::CasGet { hash: [0; 32] })
            .await;

        match response {
            VeloResponse::Error(err) => {
                assert!(err.message.contains("Not implemented"));
            }
            _ => panic!("Expected Not implemented error"),
        }
    }

    // ==================== Multiple Operations Tests ====================

    #[tokio::test]
    async fn test_multiple_files_independent() {
        let (mut handler, _temp) = create_test_handler();

        // Insert multiple files
        for i in 0..10 {
            handler
                .handle_request(VeloRequest::ManifestUpsert {
                    path: format!("file_{}.txt", i),
                    entry: VnodeEntry {
                        content_hash: [0; 32],
                        size: i as u64 * 100,
                        mtime: 0,
                        mode: 0,
                        flags: 0,
                        _pad: 0,
                    },
                })
                .await;
        }

        // Verify each file
        for i in 0..10 {
            let response = handler
                .handle_request(VeloRequest::ManifestGet {
                    path: format!("file_{}.txt", i),
                })
                .await;

            match response {
                VeloResponse::ManifestAck { entry: Some(e) } => {
                    assert_eq!(e.size, i as u64 * 100);
                }
                _ => panic!("File {} not found", i),
            }
        }
    }
}
