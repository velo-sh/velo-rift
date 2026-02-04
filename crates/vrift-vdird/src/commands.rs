//! Command handlers for vdir_d

use crate::vdir::{fnv1a_hash, VDir, VDirEntry, FLAG_DIR};
use crate::ProjectConfig;
use anyhow::Result;
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::PathBuf;
use tracing::{debug, error, info, warn};
use vrift_ipc::{VeloRequest, VeloResponse, VnodeEntry};

/// Command handler for vdir_d
pub struct CommandHandler {
    config: ProjectConfig,
    vdir: VDir,
}

impl CommandHandler {
    pub fn new(config: ProjectConfig, vdir: VDir) -> Self {
        Self { config, vdir }
    }

    /// Handle incoming request
    pub async fn handle_request(&mut self, request: VeloRequest) -> VeloResponse {
        match request {
            VeloRequest::Handshake { client_version } => {
                info!(client_version = %client_version, "Handshake");
                VeloResponse::HandshakeAck {
                    server_version: env!("CARGO_PKG_VERSION").to_string(),
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

            // Not yet implemented - forward to future handlers
            _ => {
                warn!(?request, "Unhandled request type");
                VeloResponse::Error("Not implemented".to_string())
            }
        }
    }

    /// Handle ManifestGet
    fn handle_manifest_get(&self, path: &str) -> VeloResponse {
        let path_hash = fnv1a_hash(path);
        match self.vdir.lookup(path_hash) {
            Some(entry) => {
                let vnode = VnodeEntry {
                    content_hash: entry.cas_hash,
                    size: entry.size,
                    mtime: entry.mtime_sec as u64,
                    mode: entry.mode,
                    flags: entry.flags,
                    _pad: 0,
                };
                VeloResponse::ManifestAck { entry: Some(vnode) }
            }
            None => VeloResponse::ManifestAck { entry: None },
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
                VeloResponse::Error(e.to_string())
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
                return VeloResponse::Error(format!("Read error: {}", e));
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
            return VeloResponse::Error(format!("Ingest error: {}", e));
        }

        // 4. Get metadata
        let meta = match fs::metadata(&temp) {
            Ok(m) => m,
            Err(e) => {
                return VeloResponse::Error(format!("Metadata error: {}", e));
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
            return VeloResponse::Error(format!("VDir update error: {}", e));
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

    /// Ingest temp file to CAS
    async fn ingest_to_cas(
        &self,
        _temp: &PathBuf,
        cas_path: &PathBuf,
        content: &[u8],
    ) -> Result<()> {
        // Ensure parent directory exists
        if let Some(parent) = cas_path.parent() {
            fs::create_dir_all(parent)?;
        }

        // Skip if already exists (content-addressed)
        if cas_path.exists() {
            debug!(path = %cas_path.display(), "CAS blob already exists");
            return Ok(());
        }

        // Try reflink first (zero-copy)
        #[cfg(target_os = "linux")]
        {
            use std::os::unix::io::AsRawFd;
            let src = fs::File::open(temp)?;
            let dst = fs::File::create(cas_path)?;

            let result = unsafe {
                libc::ioctl(
                    dst.as_raw_fd(),
                    0x40049409, // FICLONE
                    src.as_raw_fd(),
                )
            };

            if result == 0 {
                debug!("Used reflink for CAS ingestion");
                return Ok(());
            }
        }

        // Fallback: write content
        fs::write(cas_path, content)?;
        debug!(path = %cas_path.display(), "Wrote CAS blob");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_manifest_upsert_and_get() {
        let temp = tempdir().unwrap();
        let config = ProjectConfig::from_project_root(temp.path().to_path_buf());

        // Create VDir
        let vdir_path = temp.path().join("test.vdir");
        let vdir = VDir::create_or_open(&vdir_path).unwrap();

        let mut handler = CommandHandler::new(config, vdir);

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
            }
            _ => panic!("Expected ManifestAck with entry"),
        }
    }
}
