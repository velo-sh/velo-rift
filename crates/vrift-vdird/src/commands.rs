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
                }
            }

            VeloRequest::ManifestGet { path } => self.handle_manifest_get(&path),

            VeloRequest::ManifestUpsert { path, entry } => {
                self.handle_manifest_upsert(&path, entry)
            }

            VeloRequest::ManifestRemove { path } => self.handle_manifest_remove(&path),

            VeloRequest::ManifestReingest { vpath, temp_path } => {
                self.handle_reingest(&vpath, &temp_path).await
            }

            VeloRequest::IngestFullScan {
                path,
                manifest_path,
                cas_root: _, // vdird uses config.cas_path instead
                threads,
                phantom,
                tier1,
            } => {
                self.handle_ingest_full_scan(&path, &manifest_path, threads, phantom, tier1)
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

    /// Handle ManifestReingest (CoW commit)
    async fn handle_reingest(&mut self, vpath: &str, temp_path: &str) -> VeloResponse {
        let temp = PathBuf::from(temp_path);

        // 1. Read and hash temp file
        let content = match fs::read(&temp) {
            Ok(c) => c,
            Err(e) => {
                error!(error = %e, temp = %temp_path, "Failed to read temp file");
                return VeloResponse::Error(VeloError::io_error(format!("Read error: {}", e)));
            }
        };

        let hash = blake3::hash(&content);
        let hash_bytes: [u8; 32] = *hash.as_bytes();
        let hash_hex = hex::encode(&hash_bytes[..8]);

        // 2. Determine CAS path
        let cas_path = self
            .config
            .cas_path
            .join(&hash_hex[..2])
            .join(&hash_hex[2..]);

        // 3. Ingest to CAS (try reflink, fallback to copy)
        if let Err(e) = self.ingest_to_cas(&temp, &cas_path, &content).await {
            error!(error = %e, "CAS ingestion failed");
            return VeloResponse::Error(VeloError::new(
                VeloErrorKind::IngestFailed,
                format!("Ingest error: {}", e),
            ));
        }

        // 4. Get metadata
        let meta = match fs::metadata(&temp) {
            Ok(m) => m,
            Err(e) => {
                return VeloResponse::Error(VeloError::io_error(format!("Metadata error: {}", e)));
            }
        };

        // 5. Update VDir
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

        // 6. Cleanup temp file
        let _ = fs::remove_file(&temp);

        info!(vpath = %vpath, hash = %hash_hex, "Reingest complete");

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

    /// Ingest temp file to CAS using reflink with automatic fallback
    async fn ingest_to_cas(&self, temp: &Path, cas_path: &Path, _content: &[u8]) -> Result<()> {
        use vrift_cas::reflink::ingest_with_fallback;

        // Skip if already exists (content-addressed)
        if cas_path.exists() {
            debug!(path = %cas_path.display(), "CAS blob already exists");
            return Ok(());
        }

        // Use reflink module with automatic fallback chain:
        // 1. reflink (zero-copy CoW)
        // 2. hardlink (same inode)
        // 3. copy (full data copy)
        match ingest_with_fallback(temp, cas_path) {
            Ok(method) => {
                debug!(
                    src = %temp.display(),
                    dst = %cas_path.display(),
                    method = %method,
                    "CAS ingestion complete"
                );
                Ok(())
            }
            Err(e) => {
                error!(error = %e, "CAS ingestion failed");
                Err(anyhow::anyhow!("Ingestion failed: {}", e))
            }
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
        if let Err(e) = self.write_manifest(&manifest_out, &file_paths, &results) {
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
        _file_paths: &[PathBuf],
        results: &[Result<vrift_cas::IngestResult, vrift_cas::CasError>],
    ) -> Result<()> {
        use std::io::Write;

        // Simple binary manifest: count + entries
        let entries: Vec<_> = results.iter().filter_map(|r| r.as_ref().ok()).collect();

        let mut file = fs::File::create(manifest_path)?;

        // Write count
        let count = entries.len() as u32;
        file.write_all(&count.to_le_bytes())?;

        // Write entries (hash + size)
        for entry in entries {
            file.write_all(&entry.hash)?;
            file.write_all(&entry.size.to_le_bytes())?;
        }

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
            VeloResponse::RegisterAck { workspace_id } => {
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
                assert!(err.message.contains("Read error"));
            }
            _ => panic!("Expected Error for nonexistent file"),
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
