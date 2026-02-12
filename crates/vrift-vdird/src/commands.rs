//! Command handlers for vdir_d

use crate::vdir::{fnv1a_hash, VDir, VDirEntry, FLAG_DIR};
use crate::ProjectConfig;
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tracing::{debug, error, info, warn};
use vrift_ipc::{VeloError, VeloRequest, VeloResponse, VnodeEntry, PROTOCOL_VERSION};

/// Command handler for vdir_d
pub struct CommandHandler {
    config: ProjectConfig,
    vdir: Arc<Mutex<VDir>>,
    manifest: std::sync::Arc<vrift_manifest::lmdb::LmdbManifest>,
}

impl CommandHandler {
    pub fn new(
        config: ProjectConfig,
        vdir: Arc<Mutex<VDir>>,
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
                cas_root,
                force_hash: _,
            } => {
                let config = self.config.clone();
                let vdir = self.vdir.clone();
                let manifest = self.manifest.clone();
                let path_clone = path.clone();
                let manifest_path_clone = manifest_path.clone();
                let prefix_clone = prefix.clone();
                let cas_root_clone = cas_root.clone();

                // Offload heavy ingest to blocking thread to keep daemon responsive
                tokio::task::spawn_blocking(move || {
                    Self::handle_ingest_full_scan_sync(
                        &config,
                        &vdir,
                        &manifest,
                        &path_clone,
                        &manifest_path_clone,
                        threads,
                        phantom,
                        tier1,
                        prefix_clone.as_deref(),
                        cas_root_clone.as_deref(),
                    )
                })
                .await
                .unwrap_or_else(|e| {
                    VeloResponse::Error(VeloError::internal(format!("Ingest task panicked: {}", e)))
                })
            }

            // Not yet implemented - forward to future handlers
            _ => {
                warn!(?request, "Unhandled request type");
                VeloResponse::Error(VeloError::internal("Not implemented"))
            }
        }
    }

    /// Handle ManifestGet
    fn handle_manifest_get(&self, path: &str) -> VeloResponse {
        let path_hash = fnv1a_hash(path);
        debug!(path = %path, hash = %path_hash, "ManifestGet request");

        // 1. First check VDir (runtime overlay for COW mutations)
        if let Some(entry) = self.vdir.lock().unwrap().lookup(path_hash) {
            let mtime_ns = entry.mtime_sec as u64 * 1_000_000_000 + entry.mtime_nsec as u64;
            let vnode = VnodeEntry {
                content_hash: entry.cas_hash,
                size: entry.size,
                mtime: mtime_ns,
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

                // Explicitly convert to vrift_ipc's VnodeEntry to ensure correct serialization
                let vnode = vrift_ipc::VnodeEntry {
                    content_hash: entry.vnode.content_hash,
                    size: entry.vnode.size,
                    mtime: entry.vnode.mtime,
                    mode: entry.vnode.mode,
                    flags: entry.vnode.flags,
                    _pad: 0,
                };

                VeloResponse::ManifestAck { entry: Some(vnode) }
            }
            Ok(None) => {
                debug!(path = %path, "ManifestGet: not found");
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
        let mtime_sec = (entry.mtime / 1_000_000_000) as i64;
        let mtime_nsec = (entry.mtime % 1_000_000_000) as u32;
        let vdir_entry = VDirEntry {
            path_hash: fnv1a_hash(path),
            cas_hash: entry.content_hash,
            size: entry.size,
            mtime_sec,
            mtime_nsec,
            mode: entry.mode,
            flags: entry.flags,
            path_offset: 0,
            path_len: 0,
        };

        match self.vdir.lock().unwrap().upsert_with_path(vdir_entry, path) {
            Ok(_) => {
                debug!(path = %path, "Upserted entry in VDir");
                // Also update persistent LMDB
                self.manifest
                    .insert(path, entry.clone(), vrift_manifest::AssetTier::Tier2Mutable);
                if let Err(e) = self.manifest.commit() {
                    error!(error = %e, path = %path, "Failed to commit manifest upsert");
                }
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
        if self.vdir.lock().unwrap().mark_dirty(path_hash, false) {
            // Update persistent LMDB
            self.manifest.remove(path);
            let _ = self.manifest.commit();
            debug!(path = %path, "Marked for removal");
            VeloResponse::ManifestAck { entry: None }
        } else {
            VeloResponse::ManifestAck { entry: None }
        }
    }

    /// Handle ManifestRename
    fn handle_manifest_rename(&mut self, old_path: &str, new_path: &str) -> VeloResponse {
        let old_hash = fnv1a_hash(old_path);
        let new_hash = fnv1a_hash(new_path);

        let old_entry = if let Some(entry) = self.vdir.lock().unwrap().lookup(old_hash) {
            Some(*entry)
        } else if let Ok(Some(lmdb_entry)) = self.manifest.get(old_path) {
            Some(VDirEntry {
                path_hash: old_hash,
                cas_hash: lmdb_entry.vnode.content_hash,
                size: lmdb_entry.vnode.size,
                mtime_sec: (lmdb_entry.vnode.mtime / 1_000_000_000) as i64,
                mtime_nsec: (lmdb_entry.vnode.mtime % 1_000_000_000) as u32,
                mode: lmdb_entry.vnode.mode,
                flags: lmdb_entry.vnode.flags,
                path_offset: 0,
                path_len: 0,
            })
        } else {
            None
        };

        match old_entry {
            Some(entry) => {
                self.vdir.lock().unwrap().mark_dirty(old_hash, false);
                let new_entry = VDirEntry {
                    path_hash: new_hash,
                    ..entry
                };
                match self
                    .vdir
                    .lock()
                    .unwrap()
                    .upsert_with_path(new_entry, new_path)
                {
                    Ok(_) => {
                        debug!(old = %old_path, new = %new_path, "Manifest rename in VDir");
                        // Also update persistent LMDB
                        self.manifest.remove(old_path);
                        self.manifest.insert(
                            new_path,
                            vrift_ipc::VnodeEntry {
                                content_hash: entry.cas_hash,
                                size: entry.size,
                                mtime: (entry.mtime_sec as u64 * 1_000_000_000)
                                    + entry.mtime_nsec as u64,
                                mode: entry.mode,
                                flags: entry.flags,
                                _pad: 0,
                            },
                            vrift_manifest::AssetTier::Tier2Mutable,
                        );
                        let _ = self.manifest.commit();
                        VeloResponse::ManifestAck { entry: None }
                    }
                    Err(e) => {
                        error!(error = %e, "Rename upsert failed");
                        VeloResponse::Error(VeloError::internal(format!("{}", e)))
                    }
                }
            }
            None => {
                debug!(path = %old_path, "Rename: source not found");
                VeloResponse::ManifestAck { entry: None }
            }
        }
    }

    /// Handle ManifestUpdateMtime
    fn handle_manifest_update_mtime(&mut self, path: &str, mtime_ns: u64) -> VeloResponse {
        let path_hash = fnv1a_hash(path);
        let mtime_sec = (mtime_ns / 1_000_000_000) as i64;
        let mtime_nsec = (mtime_ns % 1_000_000_000) as u32;

        let existing = if let Some(entry) = self.vdir.lock().unwrap().lookup(path_hash) {
            Some(*entry)
        } else if let Ok(Some(lmdb_entry)) = self.manifest.get(path) {
            Some(VDirEntry {
                path_hash,
                cas_hash: lmdb_entry.vnode.content_hash,
                size: lmdb_entry.vnode.size,
                mtime_sec: (lmdb_entry.vnode.mtime / 1_000_000_000) as i64,
                mtime_nsec: (lmdb_entry.vnode.mtime % 1_000_000_000) as u32,
                mode: lmdb_entry.vnode.mode,
                flags: lmdb_entry.vnode.flags,
                path_offset: 0,
                path_len: 0,
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
                match self.vdir.lock().unwrap().upsert_with_path(updated, path) {
                    Ok(_) => {
                        debug!(path = %path, mtime_sec, "Updated mtime in VDir");
                        // Also update persistent LMDB
                        self.manifest.mark_stale(path);
                        let _ = self.manifest.commit();
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

    /// Handle ManifestListDir
    fn handle_manifest_list_dir(&self, path: &str) -> VeloResponse {
        let prefix = if path.is_empty() || path == "/" {
            String::new()
        } else if path.ends_with('/') {
            path.to_string()
        } else {
            format!("{}/", path)
        };

        let mut entries = Vec::new();
        let mut seen = std::collections::HashSet::new();

        if let Ok(all_entries) = self.manifest.iter() {
            for (entry_path, manifest_entry) in &all_entries {
                if !entry_path.starts_with(&prefix) {
                    continue;
                }
                let relative = &entry_path[prefix.len()..];
                let child_name = if let Some(slash_pos) = relative.find('/') {
                    let name = &relative[..slash_pos];
                    if !seen.insert(name.to_string()) {
                        continue;
                    }
                    entries.push(vrift_ipc::DirEntry {
                        name: name.to_string(),
                        is_dir: true,
                    });
                    continue;
                } else {
                    relative
                };

                if child_name.is_empty() || !seen.insert(child_name.to_string()) {
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

    /// Handle ManifestReingest
    async fn handle_reingest(&mut self, vpath: &str, temp_path: &str) -> VeloResponse {
        let source = PathBuf::from(temp_path);
        let src_meta = match fs::metadata(&source) {
            Ok(m) => m,
            Err(e) => {
                debug!(error = %e, path = %temp_path, "Reingest source not found");
                return VeloResponse::ManifestAck { entry: None };
            }
        };

        if src_meta.is_dir() {
            return VeloResponse::ManifestAck { entry: None };
        }

        let store = match vrift_cas::CasStore::new(&self.config.cas_path) {
            Ok(s) => s,
            Err(e) => {
                error!(error = %e, "Failed to initialize CAS store");
                return VeloResponse::Error(VeloError::internal(format!("CAS error: {}", e)));
            }
        };

        let is_staging = temp_path.contains("/.vrift/staging/");
        let hash_bytes = if is_staging {
            match store.store_by_move(&source) {
                Ok(h) => h,
                Err(e) => {
                    error!(error = %e, temp = %temp_path, "CAS ingestion failed");
                    return VeloResponse::Error(VeloError::internal(format!(
                        "Ingest error: {}",
                        e
                    )));
                }
            }
        } else {
            match store.store_file(&source) {
                Ok(h) => h,
                Err(e) => {
                    error!(error = %e, path = %temp_path, "CAS ingestion failed");
                    return VeloResponse::Error(VeloError::internal(format!(
                        "Ingest error: {}",
                        e
                    )));
                }
            }
        };

        let entry = VDirEntry {
            path_hash: fnv1a_hash(vpath),
            cas_hash: hash_bytes,
            size: src_meta.len(),
            mtime_sec: src_meta.mtime(),
            mtime_nsec: src_meta.mtime_nsec() as u32,
            mode: src_meta.mode(),
            flags: 0,
            path_offset: 0,
            path_len: 0,
        };

        if let Err(e) = self.vdir.lock().unwrap().upsert_with_path(entry, vpath) {
            return VeloResponse::Error(VeloError::internal(format!("VDir update error: {}", e)));
        }

        VeloResponse::ManifestAck {
            entry: Some(VnodeEntry {
                content_hash: hash_bytes,
                size: src_meta.len(),
                mtime: src_meta.mtime() as u64 * 1_000_000_000 + src_meta.mtime_nsec() as u64,
                mode: src_meta.mode(),
                flags: 0,
                _pad: 0,
            }),
        }
    }

    /// Handle IngestFullScan
    #[allow(clippy::too_many_arguments)]
    /// Handle IngestFullScan (Sync version for spawn_blocking)
    #[allow(clippy::too_many_arguments)]
    fn handle_ingest_full_scan_sync(
        config: &ProjectConfig,
        vdir_arc: &Arc<Mutex<VDir>>,
        manifest: &Arc<vrift_manifest::lmdb::LmdbManifest>,
        path: &str,
        manifest_path: &str,
        threads: Option<usize>,
        phantom: bool,
        tier1: bool,
        prefix: Option<&str>,
        cas_root_override: Option<&str>,
    ) -> VeloResponse {
        use std::time::Instant;
        use vrift_cas::{parallel_ingest_with_progress, IngestMode};
        use walkdir::WalkDir;

        let source_path = PathBuf::from(path);
        let _start = Instant::now();
        info!(
            path = %path,
            manifest = %manifest_path,
            threads = ?threads,
            phantom = phantom,
            tier1 = tier1,
            "Starting full scan ingest"
        );
        let start = Instant::now();

        info!("Collecting files via WalkDir...");
        let file_paths: Vec<PathBuf> = WalkDir::new(&source_path)
            .into_iter()
            .filter_entry(|e| !e.file_name().to_string_lossy().contains(".vrift"))
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
            .map(|e| e.path().to_path_buf())
            .collect();

        let total_files = file_paths.len() as u64;
        info!("DEBUG: Collected {} files", total_files);
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

        let mode = if phantom {
            IngestMode::Phantom
        } else if tier1 {
            IngestMode::SolidTier1
        } else {
            IngestMode::SolidTier2
        };
        // 3. Run parallel ingest — use CLI-provided CAS root if available
        let effective_cas_path = match cas_root_override {
            Some(cli_cas) => {
                let p = PathBuf::from(cli_cas);
                info!(cas_root = %p.display(), "Using CLI-provided CAS root");
                p
            }
            None => config.cas_path.clone(),
        };
        let results = parallel_ingest_with_progress(
            &file_paths,
            &effective_cas_path,
            mode,
            threads,
            |_, _| {},
        );
        info!("DEBUG: parallel_ingest_with_progress complete");

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
        // 5. Build and write manifest (persistently using LMDB)
        {
            let canon_root = source_path.canonicalize().unwrap_or(source_path.clone());
            let prefix_str = prefix.unwrap_or("");
            let _vdir = vdir_arc.lock().unwrap();
            let mut vdir = vdir_arc.lock().unwrap();
            let asset_tier = if tier1 {
                vrift_manifest::AssetTier::Tier1Immutable
            } else {
                vrift_manifest::AssetTier::Tier2Mutable
            };
            for result in results.iter().flatten() {
                let canon_source = result
                    .source_path
                    .canonicalize()
                    .unwrap_or_else(|_| result.source_path.clone());
                let rel = canon_source
                    .strip_prefix(&canon_root)
                    .unwrap_or(&canon_source);
                let key = if prefix_str == "/" || prefix_str.is_empty() {
                    format!("/{}", rel.display())
                } else {
                    format!("{}/{}", prefix_str.trim_end_matches('/'), rel.display())
                };
                let (mtime_sec, mtime_nsec, mode) = match fs::metadata(&result.source_path) {
                    Ok(meta) => (meta.mtime(), meta.mtime_nsec() as u32, meta.mode()),
                    Err(_) => (0, 0, 0o644),
                };
                // Add to persistent LMDB manifest
                let vnode = vrift_ipc::VnodeEntry {
                    content_hash: result.hash,
                    size: result.size,
                    mtime: (mtime_sec as u64 * 1_000_000_000) + mtime_nsec as u64,
                    mode,
                    flags: 0,
                    _pad: 0,
                };
                manifest.insert(&key, vnode, asset_tier);

                // Backfill VDir
                let _path_hash = fnv1a_hash(&key);
                let entry = VDirEntry {
                    path_hash: fnv1a_hash(&key),
                    cas_hash: result.hash,
                    size: result.size,
                    mtime_sec,
                    mtime_nsec,
                    mode,
                    path_offset: 0,
                    flags: 0,
                    path_len: 0,
                };
                let _ = vdir.upsert_with_path(entry, &key);
            }
        }

        // Commit LMDB transactions
        if let Err(e) = manifest.commit() {
            error!(error = %e, "Failed to commit manifest after ingest");
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
            duration_ms: start.elapsed().as_millis() as u64,
            manifest_path: manifest_path.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn create_test_handler() -> (CommandHandler, tempfile::TempDir) {
        let temp = tempdir().unwrap();
        let config = ProjectConfig::from_project_root(temp.path().to_path_buf());
        let vdir_path = temp.path().join("test.vdir");
        let vdir = Arc::new(Mutex::new(VDir::create_or_open(&vdir_path).unwrap()));
        let manifest_path = temp.path().join("manifest.lmdb");
        let manifest =
            std::sync::Arc::new(vrift_manifest::lmdb::LmdbManifest::open(&manifest_path).unwrap());
        (CommandHandler::new(config, vdir, manifest), temp)
    }

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
            VeloResponse::HandshakeAck { .. } => {}
            _ => panic!("Expected HandshakeAck"),
        }
    }

    #[tokio::test]
    async fn test_manifest_upsert_and_get() {
        let (mut handler, _temp) = create_test_handler();
        let entry = VnodeEntry {
            content_hash: [42; 32],
            size: 1000,
            mtime: 1234567890,
            mode: 0o644,
            flags: 0,
            _pad: 0,
        };
        handler
            .handle_request(VeloRequest::ManifestUpsert {
                path: "test.txt".to_string(),
                entry: entry.clone(),
            })
            .await;
        let response = handler
            .handle_request(VeloRequest::ManifestGet {
                path: "test.txt".to_string(),
            })
            .await;
        match response {
            VeloResponse::ManifestAck { entry: Some(e) } => assert_eq!(e.size, 1000),
            _ => panic!("Expected entry"),
        }
    }
}
